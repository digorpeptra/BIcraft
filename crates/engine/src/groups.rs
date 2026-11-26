//! `column.group`: Power BI Desktop's New group on a column (Data pane › New group, Column tools
//! › Data groups), as `docs/formats/groups.md` describes it.
//!
//! A group is a calculated column next to the grouped one, with the `GroupingMetadata` extended
//! property naming the column it groups:
//!
//! - a **list** group maps values to named groups with `SWITCH(TRUE, …)`; a blank value that no
//!   group lists is `(Blank)`, the values left out go to `Other` (or keep their own value);
//! - a **bin** group puts numbers into bins of a size or into a number of equal bins between the
//!   column's minimum and maximum, and dates into bins of years, months, days, hours, minutes or
//!   seconds, with the DAX, unit codes and `GroupingDesignState` Desktop writes for its own bins.
//!
//! [`column_definition`] reads a group column back (the Edit groups dialog, the snapshot).

use bicraft_dax::ast::Expr;
use bicraft_model::{Column, DataType};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::edit::{Edit, apply, check_name, new_guid, table_index};
use crate::params::{allow, opt_bool, opt_str, str_param};
use crate::report::opt_num;

/// The extended property that marks a group column.
pub(crate) const GROUPING_METADATA: &str = "GroupingMetadata";
/// The text of a blank value that no group lists.
const BLANK_GROUP: &str = "(Blank)";
/// The group of the values no group lists, when `other` is on.
const OTHER_GROUP: &str = "Other";
/// Most groups, and most values over all groups.
const MAX_GROUPS: usize = 1_000;
const MAX_VALUES: usize = 10_000;
/// Most bins of a bin count.
const MAX_BINS: f64 = 10_000.0;
/// The kinds and the date units.
pub(crate) const KINDS: &[&str] = &["list", "bin"];
pub(crate) const UNITS: &[&str] = &["number", "year", "month", "day", "hour", "minute", "second"];

/// A table name as DAX, always quoted (`'Sales'`), as Desktop writes group expressions.
fn quoted(table: &str) -> String {
    format!("'{}'", table.replace('\'', "''"))
}

fn qualified(table: &str, name: &str) -> String {
    format!("{}[{}]", quoted(table), name.replace(']', "]]"))
}

fn dax_string(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// A number as DAX writes it (no exponent).
fn dax_number(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{x:.0}")
    } else {
        format!("{x}")
    }
}

/// The JSON number of `x` (whole numbers without a fraction).
fn json_number(x: f64) -> Value {
    if x.fract() == 0.0 && x.abs() < 9e15 {
        json!(x as i64)
    } else {
        json!(x)
    }
}

fn is_number(t: DataType) -> bool {
    matches!(t, DataType::Int64 | DataType::Double | DataType::Decimal)
}

/// A date `YYYY-MM-DD` (a time after it is ignored) as its parts.
fn date_parts(s: &str) -> Option<(i64, i64, i64)> {
    let d = s.get(..10)?;
    let mut it = d.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let day: i64 = it.next()?.parse().ok()?;
    ((1..=9999).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&day)).then_some((y, m, day))
}

/// One listed value as a DAX literal of the column's type; `None` is a blank.
pub(crate) fn literal(v: &Value, t: DataType) -> Result<Option<String>, String> {
    Ok(Some(match (v, t) {
        (Value::Null, _) => return Ok(None),
        (Value::String(s), DataType::String) => dax_string(s),
        (Value::String(s), DataType::DateTime) => {
            let (y, m, d) = date_parts(s).ok_or_else(|| format!("`{s}` is not a date (YYYY-MM-DD)"))?;
            format!("DATE({y}, {m}, {d})")
        }
        (Value::Number(n), _) if is_number(t) => {
            let x = n
                .as_f64()
                .filter(|x| x.is_finite())
                .ok_or("a value is not a finite number")?;
            dax_number(x)
        }
        (Value::Bool(b), DataType::Boolean) => (if *b { "TRUE" } else { "FALSE" }).to_string(),
        (v, t) => {
            return Err(format!(
                "the value {v} does not fit the column's type ({})",
                t.as_tmsl().unwrap_or("unknown")
            ));
        }
    }))
}

/// A `json` extended property `GroupingMetadata` of `table[column]`, with `binning` when given.
fn metadata(table: &str, column: &str, binning: Option<Value>) -> Value {
    let mut v = json!({
        "version": 0,
        "groupedColumns": [{"Column": {"Expression": {"SourceRef": {"Entity": table}}, "Property": column}}],
    });
    if let Some(b) = binning {
        v["binningMetadata"] = b;
    }
    json!({"type": "json", "name": GROUPING_METADATA, "value": v})
}

/// The annotation in which Power BI Desktop keeps its Groups dialog's state; its Edit groups
/// lists the groups of a list group from it (Desktop 2.159.1231.0, docs/formats/groups.md).
pub(crate) const GROUPING_DESIGN_STATE: &str = "GroupingDesignState";

/// One listed value as a query literal (`'text'`, `1L`, `1.5D`, `true`,
/// `datetime'2025-01-31T00:00:00'`), as the public `semanticQuery` schema writes literals.
fn query_literal(v: &Value, t: DataType) -> Option<String> {
    Some(match (v, t) {
        (Value::String(s), DataType::String) => format!("'{}'", s.replace('\'', "''")),
        (Value::String(s), DataType::DateTime) => {
            let (y, m, d) = date_parts(s)?;
            format!("datetime'{y:04}-{m:02}-{d:02}T00:00:00'")
        }
        (Value::Number(n), DataType::Int64) => format!("{}L", dax_number(n.as_f64()?)),
        (Value::Number(n), DataType::Double) => format!("{}D", dax_number(n.as_f64()?)),
        (Value::Number(n), DataType::Decimal) => format!("{}M", dax_number(n.as_f64()?)),
        (Value::Bool(b), DataType::Boolean) => b.to_string(),
        _ => return None,
    })
}

/// The `GroupingDesignState` of a list group, as Desktop writes it for its own: the grouped
/// column through a source alias, a `(Blank)` placeholder item, one item per group with an `In`
/// of its values, and an `Other` item when `other` is on. `None` when a group lists a blank
/// (how Desktop writes that is not known).
fn design_state(table: &str, column: &str, dt: DataType, groups: &[Group], other: bool) -> Option<String> {
    let (mut state, col) = design_header(table, column);
    let mut items = vec![json!({"DisplayName": BLANK_GROUP, "BlankDefaultPlaceholder": true})];
    for g in groups {
        let values = g
            .values
            .iter()
            .map(|v| query_literal(v, dt).map(|l| json!([{"Literal": {"Value": l}}])))
            .collect::<Option<Vec<_>>>()?;
        items.push(json!({"DisplayName": g.name,
                          "Expression": {"In": {"Expressions": [col], "Values": values}}}));
    }
    if other {
        items.push(json!({"DisplayName": OTHER_GROUP}));
    }
    state["GroupItems"] = Value::Array(items);
    Some(state.to_string())
}

/// The start of a design state of `table[column]`: `{Version, Sources, GroupedColumns}` (the
/// source alias is the table name's first letter in lower case, as Desktop's `s` for `Sales`),
/// and the grouped column through that alias.
fn design_header(table: &str, column: &str) -> (Value, Value) {
    let alias = table
        .chars()
        .next()
        .filter(char::is_ascii_alphabetic)
        .map_or_else(|| "t".to_string(), |c| c.to_ascii_lowercase().to_string());
    let col = json!({"Column": {"Expression": {"SourceRef": {"Source": alias}}, "Property": column}});
    let state = json!({"Version": 0, "Sources": [{"Name": alias, "Entity": table}], "GroupedColumns": [col]});
    (state, col)
}

/// How a bin group bins, for its design state.
enum BinBy {
    /// Bins of `size` (of a date `unit`, or numbers).
    Size { size: f64, unit: &'static str },
    /// `count` equal bins between the column's minimum and maximum.
    Count(f64),
}

/// The `GroupingDesignState` of a bin group, as Desktop writes it for its own: a `BinItem` whose
/// expression is the public `semanticQuery` `Floor` (a size, with a `TimeUnit` for dates) or the
/// `Min` member of a `Discretize` (a number of bins).
fn bin_design_state(table: &str, column: &str, by: &BinBy) -> String {
    let (mut state, col) = design_header(table, column);
    let item = match by {
        BinBy::Size { size, unit } => {
            let mut floor = json!({"Expression": col, "Size": size});
            if let Some(t) = time_unit(unit) {
                floor["TimeUnit"] = json!(t);
            }
            json!({"Floor": floor})
        }
        BinBy::Count(n) => json!({"Member": {
            "Expression": {"Discretize": {"Expression": col, "Count": json_number(*n)}},
            "Member": "Min",
        }}),
    };
    state["BinItem"] = json!({"Expression": item});
    state.to_string()
}

/// Puts the design state on the column in place of any, or removes it with `None`.
fn set_design_state(c: &mut Column, state: Option<&str>) {
    c.annotations.retain(|a| a.name != GROUPING_DESIGN_STATE);
    if let Some(v) = state {
        c.annotations.push(bicraft_model::Annotation {
            name: GROUPING_DESIGN_STATE.into(),
            value: v.to_string(),
        });
    }
}

/// The column's `GroupingMetadata` value, when it is a group column.
fn grouping(c: &Column) -> Option<&Value> {
    c.extra
        .get("extendedProperties")?
        .as_array()?
        .iter()
        .find(|e| e.get("name").and_then(Value::as_str) == Some(GROUPING_METADATA))?
        .get("value")
}

/// One group of a list: its name and its values (`None` a blank).
struct Group {
    name: String,
    values: Vec<Value>,
}

fn parse_groups(p: &Value) -> Result<Vec<Group>, String> {
    let list = match p.get("groups") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(a)) => a,
        Some(_) => return Err("`groups` must be a list of {name, values}".into()),
    };
    if list.len() > MAX_GROUPS {
        return Err(format!("at most {MAX_GROUPS} groups"));
    }
    let mut out: Vec<Group> = Vec::with_capacity(list.len());
    let mut total = 0usize;
    for (i, g) in list.iter().enumerate() {
        allow(g, &["name", "values"]).map_err(|e| format!("groups[{i}]: {e}"))?;
        let name = str_param(g, "name").map_err(|e| format!("groups[{i}]: {e}"))?;
        check_name("group", name).map_err(|e| format!("groups[{i}]: {e}"))?;
        if out.iter().any(|o| o.name.eq_ignore_ascii_case(name)) {
            return Err(format!("groups[{i}]: the group `{name}` is listed twice"));
        }
        let values = match g.get("values") {
            Some(Value::Array(a)) if !a.is_empty() => a.clone(),
            _ => return Err(format!("groups[{i}]: `values` must list at least one value")),
        };
        total = total.saturating_add(values.len());
        if total > MAX_VALUES {
            return Err(format!("at most {MAX_VALUES} values over all groups"));
        }
        out.push(Group {
            name: name.to_string(),
            values,
        });
    }
    Ok(out)
}

/// The key that tells two listed values apart (texts without case, as the engine compares).
fn value_key(v: &Value) -> String {
    match v {
        Value::String(s) => format!("s:{}", s.to_lowercase()),
        other => other.to_string(),
    }
}

/// `SWITCH(TRUE, …)` of a list group.
fn list_expression(c: &str, dt: DataType, groups: &[Group], other: bool) -> Result<String, String> {
    let mut seen = std::collections::HashSet::new();
    let mut branches: Vec<(String, String)> = Vec::new();
    let mut blank_grouped = false;
    for g in groups {
        let mut lits = Vec::new();
        let mut blank = false;
        for v in &g.values {
            if !seen.insert(value_key(v)) {
                return Err(format!("the value {v} is in more than one group"));
            }
            match literal(v, dt)? {
                Some(l) => lits.push(l),
                None => blank = true,
            }
        }
        let mut cond = Vec::new();
        if blank {
            blank_grouped = true;
            cond.push(format!("ISBLANK({c})"));
        }
        if !lits.is_empty() {
            cond.push(format!("{c} IN {{{}}}", lits.join(", ")));
        }
        branches.push((cond.join(" || "), dax_string(&g.name)));
    }
    let mut lines = vec!["TRUE".to_string()];
    if !blank_grouped {
        lines.push(format!("ISBLANK({c}), {}", dax_string(BLANK_GROUP)));
    }
    for (cond, name) in branches {
        lines.push(format!("{cond}, {name}"));
    }
    lines.push(if other {
        dax_string(OTHER_GROUP)
    } else if dt == DataType::String {
        c.to_string()
    } else {
        format!("{c} & \"\"")
    });
    Ok(format!("SWITCH(\n    {}\n)", lines.join(",\n    ")))
}

/// `body` for the non-blank values of `c`, a blank for a blank.
fn unless_blank(c: &str, body: &str) -> String {
    format!("IF(\n    ISBLANK({c}),\n    BLANK(),\n    {body}\n)")
}

/// The bin expression of a date column by `unit` of `n` units, in Desktop's form: years and
/// months restart each year; days count from day 1 of the date serial numbers; hours, minutes and
/// seconds floor the fraction of the day.
fn date_bins(c: &str, unit: &str, n: i64) -> String {
    let body = match (unit, n) {
        ("year", 1) => format!("DATE(YEAR({c}), 1, 1)"),
        ("year", n) => format!("DATE(INT(YEAR({c}) / {n}) * {n}, 1, 1)"),
        ("month", 1) => format!("DATE(YEAR({c}), 1 + (MONTH({c}) - 1), 1)"),
        ("month", n) => format!("DATE(YEAR({c}), 1 + (INT((MONTH({c}) - 1) / {n}) * {n}), 1)"),
        ("hour", n) => format!("(INT(({c} * 24) / {n}) * {n}) / 24"),
        ("minute", n) => format!("(INT(({c} * 1440) / {n}) * {n}) / 1440"),
        ("second", n) => format!("(INT(({c} * 86400) / {n}) * {n}) / 86400"),
        (_, n) => format!("1 + (INT((INT({c}) - 1) / {n}) * {n})"),
    };
    unless_blank(c, &body)
}

/// Bins of numbers of size `n` (DAX), toward zero on both sides of it, as Desktop writes them.
fn number_bins(c: &str, n: &str) -> String {
    unless_blank(
        c,
        &format!(
            "IF(\n        {c} >= 0,\n        ROUNDDOWN({c} / {n}, 0) * {n},\n        ROUNDUP({c} / {n}, 0) * {n}\n    )"
        ),
    )
}

/// `count` equal bins between the column's minimum and maximum, computed by the expression (they
/// follow the data on refresh), as Desktop writes them.
fn count_bins(c: &str, count: &str) -> String {
    format!(
        "VAR __Count = {count}\nVAR __Min = MIN({c})\nVAR __Max = MAX({c})\nVAR __Difference = (__Max - __Min)\n\
         VAR __Size = IF(__Difference > 0, __Difference / __Count, 1)\n\
         VAR __BinNumber = INT(({c} - __Min) / __Size)\nRETURN\n    {}",
        unless_blank(c, "__Min + (MIN(__BinNumber, __Count - 1) * __Size)").replace('\n', "\n    ")
    )
}

/// The format string Desktop gives a date bin column of `unit`.
fn date_format(unit: &str) -> &'static str {
    match unit {
        "year" => "yyyy",
        "month" => "mmmm yyyy",
        "day" => "Short Date",
        _ => "General Date",
    }
}

/// `column.group {table, column, name?, kind?, groups?, other?, bin_size?, bin_count?, unit?, replace?}`.
pub(crate) fn group(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "table",
            "column",
            "name",
            "kind",
            "groups",
            "other",
            "bin_size",
            "bin_count",
            "unit",
            "replace",
        ],
    )?;
    let table = str_param(p, "table")?;
    let column = str_param(p, "column")?;
    let (table, column, dt) = {
        let m = crate::commands::model(s)?;
        let t = m
            .table(table)
            .ok_or_else(|| format!("no table named `{table}`"))?;
        let c = t
            .columns
            .iter()
            .find(|c| !c.is_row_number() && c.name.eq_ignore_ascii_case(column))
            .ok_or_else(|| format!("table `{}` has no column `{column}`", t.name))?;
        (t.name.clone(), c.name.clone(), c.data_type)
    };
    let bins_fit = is_number(dt) || dt == DataType::DateTime;
    let kind = match opt_str(p, "kind")? {
        Some(k) => KINDS
            .iter()
            .copied()
            .find(|x| x.eq_ignore_ascii_case(k))
            .ok_or_else(|| format!("unknown kind `{k}` (expected list or bin)"))?,
        None if bins_fit && p.get("groups").is_none() => "bin",
        None => "list",
    };
    if kind == "bin" && !bins_fit {
        return Err(format!(
            "'{table}'[{column}] is not a number or a date: bins need one (use kind: list)"
        ));
    }
    let suffix = if kind == "bin" { "(bins)" } else { "(groups)" };
    let name = match opt_str(p, "name")? {
        Some(n) => n.to_string(),
        None => format!("{column} {suffix}"),
    };
    check_name("column", &name)?;
    let replace = opt_bool(p, "replace")?.unwrap_or(false);
    let c = qualified(&table, &column);
    let mut result = Map::new();
    let design: Option<String>;
    let mut format: Option<&str> = None;
    let (expression, out_type, binning) = if kind == "list" {
        for k in ["bin_size", "bin_count", "unit"] {
            if p.get(k).is_some_and(|v| !v.is_null()) {
                return Err(format!("`{k}` is for bins (kind: bin)"));
            }
        }
        let groups = parse_groups(p)?;
        if groups.is_empty() {
            return Err("a list group needs `groups`: [{name, values}]".into());
        }
        let other = opt_bool(p, "other")?.unwrap_or(true);
        result.insert("other".into(), json!(other));
        let e = list_expression(&c, dt, &groups, other)?;
        design = design_state(&table, &column, dt, &groups, other);
        (e, DataType::String, None)
    } else {
        if p.get("groups").is_some_and(|v| !v.is_null()) || p.get("other").is_some_and(|v| !v.is_null()) {
            return Err("`groups` and `other` are for list groups (kind: list)".into());
        }
        let size = opt_num(p, "bin_size")?;
        let count = opt_num(p, "bin_count")?;
        let default_unit = if dt == DataType::DateTime {
            "year"
        } else {
            "number"
        };
        let unit = match opt_str(p, "unit")? {
            Some(u) => UNITS
                .iter()
                .copied()
                .find(|x| x.eq_ignore_ascii_case(u))
                .ok_or_else(|| {
                    format!("unknown unit `{u}` (expected number, year, month, day, hour, minute or second)")
                })?,
            None => default_unit,
        };
        if (unit == "number") != is_number(dt) {
            return Err(if is_number(dt) {
                format!("'{table}'[{column}] is a number: its unit is `number`")
            } else {
                format!("'{table}'[{column}] is a date: its unit is year, month, day, hour, minute or second")
            });
        }
        match (size, count) {
            (Some(_), Some(_)) => return Err("pass either `bin_size` or `bin_count`, not both".into()),
            (None, None) if unit == "number" => {
                return Err("a bin group needs `bin_size` (> 0) or `bin_count`".into());
            }
            _ => {}
        }
        if unit != "number" {
            if count.is_some() {
                return Err("`bin_count` bins numbers; dates take `bin_size` units".into());
            }
            let n = size.unwrap_or(1.0);
            if n < 1.0 || n.fract() != 0.0 || n > 10_000.0 {
                return Err(
                    "`bin_size` of a date bin must be a whole number of units from 1 to 10000".into(),
                );
            }
            result.insert("unit".into(), json!(unit));
            result.insert("bin_size".into(), json_number(n));
            design = Some(bin_design_state(&table, &column, &BinBy::Size { size: n, unit }));
            format = Some(date_format(unit));
            (
                date_bins(&c, unit, n as i64),
                DataType::DateTime,
                Some(json!({"binSize": {"value": n, "unit": unit_code(unit)}})),
            )
        } else if let Some(size) = size {
            if size <= 0.0 {
                return Err("`bin_size` must be greater than 0".into());
            }
            let ty = if dt == DataType::Int64 && size.fract() == 0.0 {
                DataType::Int64
            } else {
                DataType::Double
            };
            let n = dax_number(size);
            result.insert("unit".into(), json!(unit));
            result.insert("bin_size".into(), json_number(size));
            design = Some(bin_design_state(&table, &column, &BinBy::Size { size, unit }));
            (
                number_bins(&c, &n),
                ty,
                Some(json!({"binSize": {"value": size, "unit": unit_code(unit)}})),
            )
        } else {
            let count = count.unwrap_or(1.0);
            if count < 1.0 || count.fract() != 0.0 || count > MAX_BINS {
                return Err(format!("`bin_count` must be a whole number from 1 to {MAX_BINS}"));
            }
            result.insert("unit".into(), json!(unit));
            result.insert("bin_count".into(), json_number(count));
            design = Some(bin_design_state(&table, &column, &BinBy::Count(count)));
            // Desktop writes no binningMetadata for a number of bins
            (count_bins(&c, &dax_number(count)), DataType::Double, None)
        }
    };
    let ext = metadata(&table, &column, binning);
    let out = apply(s, &json!({}), |m| {
        let ti = table_index(m, &table)?;
        let t = m.tables.get_mut(ti).ok_or("internal: table index out of range")?;
        let existing = t
            .columns
            .iter()
            .position(|x| !x.is_row_number() && x.name.eq_ignore_ascii_case(&name));
        let mut edit = Edit::new(json!({}));
        edit.check(
            format!("calculated column {}", qualified(&t.name, &name)),
            &expression,
            Some(&t.name),
        );
        match existing {
            Some(i) => {
                let x = t
                    .columns
                    .get_mut(i)
                    .ok_or("internal: column index out of range")?;
                if !replace {
                    return Err(format!(
                        "table `{table}` already has a column `{}` (pass replace: true to edit the group)",
                        x.name
                    ));
                }
                if grouping(x).is_none() {
                    return Err(format!("`{}` is not a group column: it is not replaced", x.name));
                }
                x.expression = Some(expression.clone());
                x.data_type = out_type;
                if let Some(f) = format {
                    x.format_string = Some(f.to_string());
                }
                set_metadata(x, ext.clone());
                set_design_state(x, design.as_deref());
            }
            None => {
                let mut col = Column {
                    name: name.clone(),
                    data_type: out_type,
                    expression: Some(expression.clone()),
                    lineage_tag: Some(new_guid()),
                    summarize_by: Some("none".into()),
                    format_string: format.map(str::to_string),
                    ..Default::default()
                };
                set_metadata(&mut col, ext.clone());
                set_design_state(&mut col, design.as_deref());
                crate::desktop_defaults::new_column(&mut col, true, None);
                t.columns.push(col);
            }
        }
        Ok(edit)
    })?;
    result.insert("table".into(), json!(table));
    result.insert("column".into(), json!(name));
    result.insert("grouped_column".into(), json!(column));
    result.insert("kind".into(), json!(kind));
    result.insert("expression".into(), json!(expression));
    result.insert("data_type".into(), json!(out_type.as_tmsl()));
    if let Some(o) = out.as_object() {
        for k in ["warnings", "diagnostics"] {
            if let Some(v) = o.get(k) {
                result.insert(k.into(), v.clone());
            }
        }
    }
    Ok(Value::Object(result))
}

/// The code of a bin unit in `binningMetadata.binSize.unit`, as Power BI Desktop 2.159.1231.0
/// writes it (docs/formats/groups.md).
fn unit_code(unit: &str) -> i64 {
    match unit {
        "year" => 4,
        "month" => 6,
        "day" => 8,
        "hour" => 9,
        "minute" => 10,
        "second" => 11,
        _ => 0,
    }
}

/// The unit of a `binningMetadata` code; 1, 2 and 3 are the codes BIcraft wrote for years, months
/// and days before it had Desktop's.
fn unit_name(code: i64) -> &'static str {
    match code {
        4 | 1 => "year",
        6 | 2 => "month",
        8 | 3 => "day",
        9 => "hour",
        10 => "minute",
        11 => "second",
        _ => "number",
    }
}

/// The public `semanticQuery` `TimeUnit` of a date bin unit (the design state's `Floor`).
fn time_unit(unit: &str) -> Option<i64> {
    match unit {
        "day" => Some(0),
        "month" => Some(2),
        "year" => Some(3),
        "second" => Some(5),
        "minute" => Some(6),
        "hour" => Some(7),
        _ => None,
    }
}

/// Puts `ext` in the column's extended properties in place of any `GroupingMetadata`.
fn set_metadata(c: &mut Column, ext: Value) {
    let list = c
        .extra
        .entry("extendedProperties".to_string())
        .or_insert_with(|| json!([]));
    if !list.is_array() {
        *list = json!([]);
    }
    if let Some(a) = list.as_array_mut() {
        a.retain(|e| e.get("name").and_then(Value::as_str) != Some(GROUPING_METADATA));
        a.push(ext);
    }
}

// ---------------------------------------------------------------------------------------------
// Reading a group back
// ---------------------------------------------------------------------------------------------

/// The definition of group column `c` as `column.group` takes it (`{column, kind,
/// groups, other}` or `{column, kind, bin_size, unit}`), when it is one: what the Edit groups
/// dialog starts from. A list group whose expression is not the `SWITCH(TRUE, …)` form gives
/// its column and kind only.
pub(crate) fn column_definition(c: &Column) -> Option<Value> {
    let meta = grouping(c)?;
    let source = meta
        .pointer("/groupedColumns/0/Column/Property")
        .and_then(Value::as_str)?;
    let mut out = json!({"column": source});
    // a number of bins: Desktop keeps the count in the design state only
    let count = c
        .annotations
        .iter()
        .find(|a| a.name == GROUPING_DESIGN_STATE)
        .and_then(|a| serde_json::from_str::<Value>(&a.value).ok())
        .and_then(|d| {
            d.pointer("/BinItem/Expression/Member/Expression/Discretize/Count")
                .and_then(Value::as_f64)
        });
    if let Some(n) = count {
        out["kind"] = json!("bin");
        out["bin_count"] = json_number(n);
        out["unit"] = json!("number");
        return Some(out);
    }
    if let Some(b) = meta.get("binningMetadata") {
        out["kind"] = json!("bin");
        if let Some(n) = b.get("binCount").and_then(Value::as_f64) {
            out["bin_count"] = json_number(n);
        } else if let Some(x) = b.pointer("/binSize/value").and_then(Value::as_f64) {
            out["bin_size"] = json_number(x);
        }
        let unit = b.pointer("/binSize/unit").and_then(Value::as_i64).unwrap_or(0);
        out["unit"] = json!(unit_name(unit));
        return Some(out);
    }
    out["kind"] = json!("list");
    if let Some((groups, other)) = c.expression.as_deref().and_then(read_switch) {
        out["groups"] = Value::Array(groups);
        out["other"] = json!(other);
    }
    Some(out)
}

/// A literal of a group's list back as JSON (`DATE(y, m, d)` as `YYYY-MM-DD`).
fn literal_value(e: &Expr) -> Option<Value> {
    match e {
        Expr::Str { value, .. } => Some(json!(value)),
        Expr::Number { text, .. } => text.parse::<f64>().ok().map(json_number),
        Expr::Bool { value, .. } => Some(json!(value)),
        Expr::Unary { op, operand, .. } if op == "-" => match literal_value(operand)? {
            Value::Number(n) => n.as_f64().map(|x| json_number(-x)),
            _ => None,
        },
        Expr::Call { name, args, .. } if name.eq_ignore_ascii_case("DATE") => {
            let part = |i: usize| match args.get(i) {
                Some(Some(Expr::Number { text, .. })) => text.parse::<i64>().ok(),
                _ => None,
            };
            Some(json!(format!("{:04}-{:02}-{:02}", part(0)?, part(1)?, part(2)?)))
        }
        _ => None,
    }
}

/// The values one condition of the `SWITCH` lists: `ISBLANK(c)`, `c IN {…}`, or both joined by
/// `||`.
fn condition_values(e: &Expr, out: &mut Vec<Value>) -> Option<()> {
    match e {
        Expr::Binary { op, lhs, rhs, .. } if op == "||" => {
            condition_values(lhs, out)?;
            condition_values(rhs, out)
        }
        Expr::Call { name, .. } if name.eq_ignore_ascii_case("ISBLANK") => {
            out.push(Value::Null);
            Some(())
        }
        Expr::Binary { op, rhs, .. } if op.eq_ignore_ascii_case("IN") => {
            let Expr::TableCtor { rows, .. } = rhs.as_ref() else {
                return None;
            };
            for r in rows {
                out.push(literal_value(r)?);
            }
            Some(())
        }
        _ => None,
    }
}

/// The groups and `other` of a `SWITCH(TRUE, cond, "name", …, else)` list group.
fn read_switch(expression: &str) -> Option<(Vec<Value>, bool)> {
    let (Some(Expr::Call { name, args, .. }), _) = bicraft_dax::parse_expression(expression) else {
        return None;
    };
    if !name.eq_ignore_ascii_case("SWITCH") || args.len() < 2 {
        return None;
    }
    let args: Vec<&Expr> = args.iter().map(Option::as_ref).collect::<Option<_>>()?;
    let (first, rest) = args.split_first()?;
    let is_true = match first {
        Expr::Bool { value, .. } => *value,
        Expr::Name { name, .. } => name.eq_ignore_ascii_case("TRUE"),
        Expr::Call { name, args, .. } => name.eq_ignore_ascii_case("TRUE") && args.is_empty(),
        _ => false,
    };
    if !is_true {
        return None;
    }
    let (last, pairs) = rest.split_last()?;
    if pairs.len() % 2 != 0 {
        return None;
    }
    let other = matches!(last, Expr::Str { value, .. } if value == OTHER_GROUP);
    let mut groups = Vec::new();
    for pair in pairs.chunks(2) {
        let (cond, label) = (pair.first()?, pair.get(1)?);
        let Expr::Str { value: label, .. } = label else {
            return None;
        };
        let mut values = Vec::new();
        condition_values(cond, &mut values)?;
        // the branch of the blanks no group lists
        if label == BLANK_GROUP && values == [Value::Null] {
            continue;
        }
        groups.push(json!({"name": label, "values": values}));
    }
    Some((groups, other))
}
