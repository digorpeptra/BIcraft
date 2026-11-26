//! `measure.quick`: Power BI Desktop's Quick measures (Home › Quick measure, the Data pane's New
//! quick measure). Each calculation writes an ordinary measure from BIcraft's own DAX template,
//! validated by the DAX binder like any `measure.add`; `docs/formats/quick-measures.md` lists the
//! calculations, their parameters and templates.

use bicraft_model::{DataType, Measure, SemanticModel};
use bicraft_report::FieldRef;
use serde_json::{Value, json};

use crate::Session;
use crate::edit::{Edit, apply, check_name, new_guid, table_index};
use crate::params::{allow, opt_bool, opt_str, str_param};
use crate::report::opt_num;

/// The calculations, in the dialog's order: (id, its parameters).
pub(crate) const KINDS: &[&str] = &[
    "average_per_category",
    "variance_per_category",
    "max_per_category",
    "min_per_category",
    "weighted_average_per_category",
    "filtered_value",
    "difference_from_filtered_value",
    "percentage_difference_from_filtered_value",
    "sales_from_new_customers",
    "year_to_date",
    "quarter_to_date",
    "month_to_date",
    "year_over_year_change",
    "quarter_over_quarter_change",
    "month_over_month_change",
    "rolling_average",
    "running_total",
    "total_for_category_filtered",
    "total_for_category_unfiltered",
    "addition",
    "subtraction",
    "multiplication",
    "division",
    "percentage_difference",
    "correlation_coefficient",
    "star_rating",
    "concatenated_list",
];

/// The parameters each calculation takes (`params` keys); the required ones first, then `?`.
pub(crate) fn inputs(kind: &str) -> &'static [&'static str] {
    match kind {
        "average_per_category" | "variance_per_category" | "max_per_category" | "min_per_category" => {
            &["base", "category"]
        }
        "weighted_average_per_category" => &["base", "weight", "category"],
        "filtered_value" | "difference_from_filtered_value" | "percentage_difference_from_filtered_value" => {
            &["base", "filter", "values"]
        }
        "sales_from_new_customers" => &["base", "customer", "date"],
        "year_to_date" => &["base", "date", "year_end_month?"],
        "quarter_to_date"
        | "month_to_date"
        | "year_over_year_change"
        | "quarter_over_quarter_change"
        | "month_over_month_change" => &["base", "date"],
        "rolling_average" => &["base", "date", "period?", "before?", "after?"],
        "running_total" => &["base", "field", "descending?"],
        "total_for_category_filtered" | "total_for_category_unfiltered" => &["base", "category"],
        "addition" | "subtraction" | "multiplication" | "division" | "percentage_difference" => {
            &["base", "second"]
        }
        "correlation_coefficient" => &["category", "base", "second"],
        "star_rating" => &["base", "max_value", "min_value?", "stars?"],
        "concatenated_list" => &["field", "max_values?", "separator?"],
        _ => &[],
    }
}

/// Rolling average periods.
pub(crate) const PERIODS: &[&str] = &["day", "month", "quarter", "year"];
/// Most values of a filter, stars, listed values, periods.
const MAX_FILTER_VALUES: usize = 1_000;
const MAX_STARS: f64 = 100.0;
const MAX_LISTED: f64 = 1_000.0;
const MAX_PERIODS: f64 = 10_000.0;

fn quoted(table: &str) -> String {
    format!("'{}'", table.replace('\'', "''"))
}

fn qualified(table: &str, name: &str) -> String {
    format!("{}[{}]", quoted(table), name.replace(']', "]]"))
}

fn bracket(name: &str) -> String {
    format!("[{}]", name.replace(']', "]]"))
}

fn dax_string(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn dax_number(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{x:.0}")
    } else {
        format!("{x}")
    }
}

/// A value as the DAX expression a quick measure uses, with the name it gives the measure.
struct Operand {
    dax: String,
    label: String,
    table: String,
}

/// A column of the model: its table, name and type.
struct Col {
    table: String,
    name: String,
    data_type: DataType,
}

impl Col {
    fn dax(&self) -> String {
        qualified(&self.table, &self.name)
    }
}

/// The DAX aggregation of a PBIR aggregation code (`Sum`, `Avg`…).
fn aggregation(function: &str) -> Option<(&'static str, &'static str)> {
    Some(match function {
        "Sum" => ("SUM", "Sum of"),
        "Avg" => ("AVERAGE", "Average of"),
        "Min" => ("MIN", "Min of"),
        "Max" => ("MAX", "Max of"),
        "Count" => ("DISTINCTCOUNT", "Count of"),
        "CountNonNull" => ("COUNT", "Count of"),
        "Median" => ("MEDIAN", "Median of"),
        _ => return None,
    })
}

/// `params[key]` as a value a measure computes: `[Measure]`, `Sum(T[C])`, or `T[C]` (summed when
/// a number, counted otherwise).
fn operand(m: &SemanticModel, p: &Value, key: &str) -> Result<Operand, String> {
    let text = str_param(p, key).map_err(|_| format!("`params.{key}`: a measure or a column is needed"))?;
    let field = crate::refs::resolve_field(Some(m), text)
        .map_err(|e| format!("`params.{key}`: {e}"))?
        .field;
    Ok(match field {
        FieldRef::Measure { table, measure } => Operand {
            dax: bracket(&measure),
            label: measure,
            table,
        },
        FieldRef::Aggregation {
            table,
            column,
            function,
        } => {
            let (f, words) = aggregation(&function).ok_or_else(|| {
                format!("`params.{key}`: the aggregation `{function}` is not one a quick measure takes")
            })?;
            Operand {
                dax: format!("{f}({})", qualified(&table, &column)),
                label: format!("{words} {column}"),
                table,
            }
        }
        FieldRef::Column { table, column } => {
            let number = m.table(&table).and_then(|t| t.column(&column)).is_some_and(|c| {
                matches!(
                    c.data_type,
                    DataType::Int64 | DataType::Double | DataType::Decimal
                )
            });
            let (f, words) = if number {
                ("SUM", "Sum of")
            } else {
                ("COUNTA", "Count of")
            };
            Operand {
                dax: format!("{f}({})", qualified(&table, &column)),
                label: format!("{words} {column}"),
                table,
            }
        }
        FieldRef::Other { .. } => {
            return Err(format!("`params.{key}`: `{text}` is not a measure or a column"));
        }
    })
}

/// `params[key]` as a column of the model.
fn column(m: &SemanticModel, p: &Value, key: &str) -> Result<Col, String> {
    let text = str_param(p, key).map_err(|_| format!("`params.{key}`: a column is needed"))?;
    match crate::refs::resolve_field(Some(m), text)
        .map_err(|e| format!("`params.{key}`: {e}"))?
        .field
    {
        FieldRef::Column { table, column } => {
            let data_type = m
                .table(&table)
                .and_then(|t| t.column(&column))
                .map_or(DataType::Unknown, |c| c.data_type);
            Ok(Col {
                table,
                name: column,
                data_type,
            })
        }
        _ => Err(format!("`params.{key}`: `{text}` is not a column")),
    }
}

/// `params[key]` as a date column.
fn date_column(m: &SemanticModel, p: &Value, key: &str) -> Result<Col, String> {
    let c = column(m, p, key)?;
    if c.data_type != DataType::DateTime {
        return Err(format!("`params.{key}`: {} is not a date column", c.dax()));
    }
    Ok(c)
}

/// The dates a time-intelligence calculation shifts and walks for date column `d`: `d` itself
/// when its table is marked as a date table; the `Date` of its auto date/time table
/// (`T[C].[Date]`, every date of the years the column spans) when `d` has a date variation;
/// else `d`, with a note: a column with gaps only offers the dates it holds.
fn calendar(m: &SemanticModel, d: &Col) -> (String, Option<String>) {
    let marked = m
        .table(&d.table)
        .is_some_and(|t| crate::date_table::data_category(t) == Some(crate::date_table::TIME));
    if marked {
        return (d.dax(), None);
    }
    if m.variation_column(&d.table, &d.name, "Date").is_some() {
        return (format!("{}.[Date]", d.dax()), None);
    }
    (
        d.dax(),
        Some(format!(
            "{} is neither the date column of a table marked as a date table nor a column with \
             auto date/time: the calculation only sees the dates the column holds (a period without \
             rows is skipped); mark a date table that holds every date (table.set {{data_category: \
             \"Time\", key_column}}) and use its date column",
            d.dax()
        )),
    )
}

/// The last day of `month` (1–12) as TOTALYTD's year-end text: the day is above 12, so the text
/// reads the same whatever the locale's day and month order.
fn year_end_text(month: u32) -> String {
    let day = match month {
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    format!("{month}/{day}")
}

/// A whole number parameter in `lo..=hi`, `default` when absent.
fn whole(p: &Value, key: &str, default: f64, lo: f64, hi: f64) -> Result<f64, String> {
    let x = opt_num(p, key)
        .map_err(|e| format!("`params`: {e}"))?
        .unwrap_or(default);
    if x.fract() != 0.0 || x < lo || x > hi {
        return Err(format!("`params.{key}` must be a whole number from {lo} to {hi}"));
    }
    Ok(x)
}

/// The measure a calculation gives: its default name, its DAX and format string.
struct Built {
    name: String,
    expression: String,
    format_string: Option<&'static str>,
    table: String,
    /// What the result's `warnings` say about the inputs.
    note: Option<String>,
}

/// Builds calculation `kind` from `p`.
fn build(m: &SemanticModel, kind: &str, p: &Value) -> Result<Built, String> {
    let allowed: Vec<&str> = inputs(kind).iter().map(|k| k.trim_end_matches('?')).collect();
    allow(p, &allowed).map_err(|e| format!("`params`: {e}"))?;
    let percent = Some("0.00%");
    let built =
        |name: String, expression: String, format_string: Option<&'static str>, table: String| Built {
            name,
            expression,
            format_string,
            table,
            note: None,
        };
    let mut note = None;
    let mut out = match kind {
        "average_per_category" | "variance_per_category" | "max_per_category" | "min_per_category" => {
            let b = operand(m, p, "base")?;
            let c = column(m, p, "category")?;
            let (f, word) = match kind {
                "average_per_category" => ("AVERAGEX", "average"),
                "variance_per_category" => ("VARX.P", "variance"),
                "max_per_category" => ("MAXX", "max"),
                _ => ("MINX", "min"),
            };
            built(
                format!("{} {word} per {}", b.label, c.name),
                format!(
                    "{f}(\n    KEEPFILTERS(VALUES({})),\n    CALCULATE({})\n)",
                    c.dax(),
                    b.dax
                ),
                None,
                b.table,
            )
        }
        "weighted_average_per_category" => {
            let b = operand(m, p, "base")?;
            let w = operand(m, p, "weight")?;
            let c = column(m, p, "category")?;
            built(
                format!("{} weighted by {} per {}", b.label, w.label, c.name),
                format!(
                    "DIVIDE(\n    SUMX(KEEPFILTERS(VALUES({c})), CALCULATE({b} * {w})),\n    \
                     SUMX(KEEPFILTERS(VALUES({c})), CALCULATE({w}))\n)",
                    c = c.dax(),
                    b = b.dax,
                    w = w.dax
                ),
                None,
                b.table,
            )
        }
        "filtered_value" | "difference_from_filtered_value" | "percentage_difference_from_filtered_value" => {
            let b = operand(m, p, "base")?;
            let c = column(m, p, "filter")?;
            let values = match p.get("values") {
                Some(Value::Array(a)) if !a.is_empty() && a.len() <= MAX_FILTER_VALUES => a,
                _ => {
                    return Err(format!(
                        "`params.values` must list from 1 to {MAX_FILTER_VALUES} values of {}",
                        c.dax()
                    ));
                }
            };
            let mut lits = Vec::with_capacity(values.len());
            for v in values {
                lits.push(crate::groups::literal(v, c.data_type)?.unwrap_or_else(|| "BLANK()".to_string()));
            }
            let shown: Vec<String> = values
                .iter()
                .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string))
                .collect();
            let which = shown.join(", ");
            let filtered = format!("CALCULATE({}, {} IN {{ {} }})", b.dax, c.dax(), lits.join(", "));
            match kind {
                "filtered_value" => built(format!("{} for {which}", b.label), filtered, None, b.table),
                "difference_from_filtered_value" => built(
                    format!("{} difference from {which}", b.label),
                    format!(
                        "VAR __Filtered = {filtered}\nRETURN\n    IF(NOT ISBLANK(__Filtered), {} - __Filtered)",
                        b.dax
                    ),
                    None,
                    b.table,
                ),
                _ => built(
                    format!("{} % difference from {which}", b.label),
                    format!(
                        "VAR __Filtered = {filtered}\nRETURN\n    IF(NOT ISBLANK(__Filtered), DIVIDE({} - __Filtered, __Filtered))",
                        b.dax
                    ),
                    percent,
                    b.table,
                ),
            }
        }
        "sales_from_new_customers" => {
            let b = operand(m, p, "base")?;
            let c = column(m, p, "customer")?;
            let d = date_column(m, p, "date")?;
            // a customer is new when it bought nothing before the first date of the context,
            // whatever the other filters (every product, every store); the first date the column
            // holds in the context does as well as the period's first day, gaps or not
            built(
                format!("{} from new customers", b.label),
                format!(
                    "VAR __Start = MIN({d})\nVAR __NewCustomers =\n    FILTER(\n        VALUES({c}),\n        \
                     VAR __Customer = {c}\n        RETURN\n            \
                     ISBLANK(CALCULATE({b}, REMOVEFILTERS(), {c} = __Customer, {d} < __Start))\n    )\n\
                     RETURN\n    CALCULATE({b}, __NewCustomers)",
                    d = d.dax(),
                    c = c.dax(),
                    b = b.dax
                ),
                None,
                b.table,
            )
        }
        "year_to_date" | "quarter_to_date" | "month_to_date" => {
            let b = operand(m, p, "base")?;
            let d = date_column(m, p, "date")?;
            let (dates, n) = calendar(m, &d);
            note = n;
            let (f, tag) = match kind {
                "year_to_date" => ("TOTALYTD", "YTD"),
                "quarter_to_date" => ("TOTALQTD", "QTD"),
                _ => ("TOTALMTD", "MTD"),
            };
            let end = whole(p, "year_end_month", 12.0, 1.0, 12.0)?;
            let year_end = if end == 12.0 {
                String::new()
            } else {
                format!(", {}", dax_string(&year_end_text(end as u32)))
            };
            built(
                format!("{} {tag}", b.label),
                format!("{f}({}, {dates}{year_end})", b.dax),
                None,
                b.table,
            )
        }
        "year_over_year_change" | "quarter_over_quarter_change" | "month_over_month_change" => {
            let b = operand(m, p, "base")?;
            let d = date_column(m, p, "date")?;
            let (dates, n) = calendar(m, &d);
            note = n;
            let (shift, tag) = match kind {
                "year_over_year_change" => ("-1, YEAR", "YoY%"),
                "quarter_over_quarter_change" => ("-1, QUARTER", "QoQ%"),
                _ => ("-1, MONTH", "MoM%"),
            };
            built(
                format!("{} {tag}", b.label),
                format!(
                    "VAR __Previous = CALCULATE({b}, DATEADD({dates}, {shift}))\nRETURN\n    DIVIDE({b} - __Previous, __Previous)",
                    b = b.dax,
                ),
                percent,
                b.table,
            )
        }
        "rolling_average" => {
            let b = operand(m, p, "base")?;
            let d = date_column(m, p, "date")?;
            let (dates, n) = calendar(m, &d);
            note = n;
            let period = match opt_str(p, "period").map_err(|e| format!("`params`: {e}"))? {
                None => "month",
                Some(x) => PERIODS
                    .iter()
                    .copied()
                    .find(|k| k.eq_ignore_ascii_case(x))
                    .ok_or_else(|| {
                        format!("`params.period`: unknown period `{x}` (day, month, quarter or year)")
                    })?,
            };
            let before = whole(p, "before", 3.0, 0.0, MAX_PERIODS)?;
            let after = whole(p, "after", 0.0, 0.0, MAX_PERIODS)?;
            // the window: from `before` periods before the last date of the context (excluded)
            // to `after` periods after it
            let shift = |n: f64| -> String {
                let n = dax_number(n);
                match period {
                    "day" => format!("__Last + {n}"),
                    "month" => format!("EDATE(__Last, {n})"),
                    "quarter" => format!("EDATE(__Last, 3 * {n})"),
                    _ => format!("EDATE(__Last, 12 * {n})"),
                }
            };
            built(
                format!("{} rolling average", b.label),
                format!(
                    "VAR __Last = MAX({d})\nVAR __First = {first} + 1\nVAR __End = {end}\nRETURN\n    \
                     AVERAGEX(\n        DATESBETWEEN({d}, __First, __End),\n        CALCULATE({b})\n    )",
                    d = dates,
                    first = shift(-before),
                    end = shift(after),
                    b = b.dax
                ),
                None,
                b.table,
            )
        }
        "running_total" => {
            let b = operand(m, p, "base")?;
            let f = column(m, p, "field")?;
            let descending = opt_bool(p, "descending")
                .map_err(|e| format!("`params`: {e}"))?
                .unwrap_or(false);
            let op = if descending { ">=" } else { "<=" };
            built(
                format!("{} running total in {}", b.label, f.name),
                format!(
                    "CALCULATE(\n    {b},\n    FILTER(\n        ALLSELECTED({f}),\n        {f} {op} MAX({f})\n    )\n)",
                    b = b.dax,
                    f = f.dax()
                ),
                None,
                b.table,
            )
        }
        "total_for_category_filtered" | "total_for_category_unfiltered" => {
            let b = operand(m, p, "base")?;
            let c = column(m, p, "category")?;
            let (f, tag) = if kind == "total_for_category_filtered" {
                ("ALLSELECTED", "")
            } else {
                ("ALL", " (all)")
            };
            built(
                format!("{} total for {}{tag}", b.label, c.name),
                format!("CALCULATE({}, {f}({}))", b.dax, c.dax()),
                None,
                b.table,
            )
        }
        "addition" | "subtraction" | "multiplication" | "division" | "percentage_difference" => {
            let a = operand(m, p, "base")?;
            let b = operand(m, p, "second")?;
            let (words, expression, format) = match kind {
                "addition" => ("plus", format!("{} + {}", a.dax, b.dax), None),
                "subtraction" => ("minus", format!("{} - {}", a.dax, b.dax), None),
                "multiplication" => ("times", format!("{} * {}", a.dax, b.dax), None),
                "division" => ("divided by", format!("DIVIDE({}, {})", a.dax, b.dax), None),
                _ => (
                    "% difference from",
                    format!(
                        "VAR __Second = {}\nRETURN\n    IF(NOT ISBLANK(__Second), DIVIDE({} - __Second, __Second))",
                        b.dax, a.dax
                    ),
                    percent,
                ),
            };
            built(
                format!("{} {words} {}", a.label, b.label),
                expression,
                format,
                a.table,
            )
        }
        "correlation_coefficient" => {
            let c = column(m, p, "category")?;
            let x = operand(m, p, "base")?;
            let y = operand(m, p, "second")?;
            // Pearson's coefficient over the categories where both values are not blank
            built(
                format!("{} and {} correlation per {}", x.label, y.label, c.name),
                format!(
                    "VAR __Pairs =\n    FILTER(\n        ADDCOLUMNS(\n            KEEPFILTERS(VALUES({c})),\n            \
                     \"__X\", CALCULATE({x}),\n            \"__Y\", CALCULATE({y})\n        ),\n        \
                     NOT ISBLANK([__X]) && NOT ISBLANK([__Y])\n    )\n\
                     VAR __N = COUNTROWS(__Pairs)\n\
                     VAR __SumX = SUMX(__Pairs, [__X])\n\
                     VAR __SumY = SUMX(__Pairs, [__Y])\n\
                     VAR __SumXY = SUMX(__Pairs, [__X] * [__Y])\n\
                     VAR __SumX2 = SUMX(__Pairs, [__X] * [__X])\n\
                     VAR __SumY2 = SUMX(__Pairs, [__Y] * [__Y])\n\
                     RETURN\n    DIVIDE(\n        __N * __SumXY - __SumX * __SumY,\n        \
                     SQRT(MAX(0, __N * __SumX2 - __SumX * __SumX) * MAX(0, __N * __SumY2 - __SumY * __SumY))\n    )",
                    c = c.dax(),
                    x = x.dax,
                    y = y.dax
                ),
                None,
                x.table,
            )
        }
        "star_rating" => {
            let b = operand(m, p, "base")?;
            let max = opt_num(p, "max_value")
                .map_err(|e| format!("`params`: {e}"))?
                .ok_or("`params.max_value`: the value that earns every star is needed")?;
            let min = opt_num(p, "min_value")
                .map_err(|e| format!("`params`: {e}"))?
                .unwrap_or(0.0);
            if max <= min {
                return Err("`params.max_value` must be greater than `params.min_value`".into());
            }
            let stars = whole(p, "stars", 5.0, 1.0, MAX_STARS)?;
            let (min, max, stars) = (dax_number(min), dax_number(max), dax_number(stars));
            built(
                format!("{} star rating", b.label),
                format!(
                    "VAR __Value = {b}\nVAR __Clamped = MIN(MAX(__Value, {min}), {max})\n\
                     VAR __Stars = ROUND(DIVIDE(__Clamped - {min}, {max} - {min}) * {stars}, 0)\nRETURN\n    \
                     IF(\n        NOT ISBLANK(__Value),\n        REPT(UNICHAR(9733), __Stars) & REPT(UNICHAR(9734), {stars} - __Stars)\n    )",
                    b = b.dax
                ),
                None,
                b.table,
            )
        }
        "concatenated_list" => {
            let f = column(m, p, "field")?;
            let n = whole(p, "max_values", 3.0, 1.0, MAX_LISTED)?;
            let sep = opt_str(p, "separator")
                .map_err(|e| format!("`params`: {e}"))?
                .unwrap_or(", ");
            if sep.chars().count() > 20 || sep.chars().any(char::is_control) {
                return Err(
                    "`params.separator` must be at most 20 characters, without control characters".into(),
                );
            }
            let n = dax_number(n);
            built(
                format!("List of {} values", f.name),
                format!(
                    "VAR __Count = DISTINCTCOUNT({c})\nVAR __List =\n    CONCATENATEX(\n        \
                     TOPN({n}, VALUES({c}), {c}, ASC),\n        {c},\n        {sep},\n        {c},\n        ASC\n    )\n\
                     RETURN\n    IF(__Count > {n}, __List & {more}, __List)",
                    c = f.dax(),
                    sep = dax_string(sep),
                    more = dax_string(&format!("{sep}etc.")),
                ),
                None,
                f.table,
            )
        }
        other => {
            return Err(format!(
                "unknown quick measure `{other}` (expected one of {})",
                KINDS.join(", ")
            ));
        }
    };
    out.note = note;
    Ok(out)
}

/// `name`, else `name 2`, `name 3`… the first that no measure of the model has.
fn free_name(m: &SemanticModel, name: &str) -> String {
    if m.measure(name).is_none() {
        return name.to_string();
    }
    (2..10_000)
        .map(|i| format!("{name} {i}"))
        .find(|n| m.measure(n).is_none())
        .unwrap_or_else(|| name.to_string())
}

/// `measure.quick {kind, params, table?, name?}`.
pub(crate) fn quick(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["kind", "params", "table", "name"])?;
    let kind = str_param(p, "kind")?;
    let kind = KINDS
        .iter()
        .copied()
        .find(|k| k.eq_ignore_ascii_case(kind))
        .ok_or_else(|| {
            format!(
                "unknown quick measure `{kind}` (expected one of {})",
                KINDS.join(", ")
            )
        })?;
    let params = match p.get("params") {
        Some(v @ Value::Object(_)) => v.clone(),
        None | Some(Value::Null) => json!({}),
        Some(_) => return Err("`params` must be an object".into()),
    };
    let m = crate::commands::model(s)?;
    let b = build(m, kind, &params)?;
    let table = match opt_str(p, "table")? {
        Some(t) => m
            .table(t)
            .map(|x| x.name.clone())
            .ok_or_else(|| format!("no table named `{t}`"))?,
        None => b.table.clone(),
    };
    let name = match opt_str(p, "name")? {
        Some(n) => n.to_string(),
        None => free_name(m, &b.name),
    };
    check_name("measure", &name)?;
    let out = apply(s, &json!({}), |m| {
        if let Some((t, x)) = m.measure(&name) {
            return Err(format!(
                "a measure named `{}` already exists (in table `{}`); measure names are unique in the model",
                x.name, t.name
            ));
        }
        let ti = table_index(m, &table)?;
        let t = m.tables.get_mut(ti).ok_or("internal: table index out of range")?;
        let mut edit = Edit::new(json!({}));
        edit.check(format!("measure [{name}]"), &b.expression, Some(&t.name));
        t.measures.push(Measure {
            name: name.clone(),
            expression: b.expression.clone(),
            format_string: b.format_string.map(str::to_string),
            lineage_tag: Some(new_guid()),
            ..Default::default()
        });
        Ok(edit)
    })?;
    let mut result = json!({
        "table": table,
        "measure": name,
        "kind": kind,
        "expression": b.expression,
        "format_string": b.format_string,
    });
    if let (Some(o), Some(r)) = (result.as_object_mut(), out.as_object()) {
        for k in ["warnings", "diagnostics"] {
            if let Some(v) = r.get(k) {
                o.insert(k.into(), v.clone());
            }
        }
    }
    if let (Some(note), Some(o)) = (b.note, result.as_object_mut()) {
        let w = o.entry("warnings").or_insert_with(|| json!([]));
        match w.as_array_mut() {
            Some(a) => a.push(json!(note)),
            None => *w = json!([note]),
        }
    }
    Ok(result)
}
