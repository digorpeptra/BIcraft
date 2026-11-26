//! The Performance analyzer's hooks in the data preview (ROADMAP review lot 14, `perf.rs`): the
//! visuals of a page evaluated afresh and timed ([`evaluate_page`]), the visuals the viewer
//! computes while a recording runs ([`viewed`]), and the DAX query text BIcraft writes for a
//! visual ([`query_text`]).
//!
//! The times are BIcraft's own, read from a monotonic clock around its evaluator; they are never
//! compared with another product's. The query text says what BIcraft evaluates as a DAX query
//! (`SUMMARIZECOLUMNS` over the visual's grouping columns, its filters as filter tables, its
//! measures and aggregations as named expressions); the preview itself does not run that text,
//! and what the text cannot say is listed in `notes`.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

use bicraft_report::{FieldRef, Page, Report, Visual};
use serde_json::Value;

use super::filters::{self, Applied, CmpOp, Cond, Item, Operand, TextOp, Val};
use super::{PreviewLimits, agg_label, at_drill_level, compute, status};
use crate::Session;

/// Longest query text written; a longer one is left out with a note.
const MAX_QUERY_BYTES: usize = 1 << 20;

/// The query text of a visual, or why there is none, with what it leaves out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct QueryText {
    pub text: Option<String>,
    pub notes: Vec<String>,
}

/// One visual evaluated and timed.
#[derive(Debug, Clone)]
pub(crate) struct Timed {
    pub visual: String,
    pub visual_type: Option<String>,
    pub elapsed: Duration,
    pub data: Value,
    pub query: QueryText,
}

/// Evaluates the visuals of `page` of `report` (the one named `only`, when given) afresh, without
/// the result cache, each timed on its own; visuals without fields are left out, as the viewer
/// leaves them. The decoded columns stay cached: the times are those of a warm store.
pub(crate) fn evaluate_page(
    s: &Session,
    report: &Report,
    page: &Page,
    only: Option<&str>,
    limits: &PreviewLimits,
) -> Vec<Timed> {
    let mut st = s.preview.acquire(s);
    let now = limits.now.floor();
    let mut out = Vec::new();
    for v in &page.visuals {
        if v.fields.is_empty() || only.is_some_and(|o| o != v.name) {
            continue;
        }
        let applied = filters::collect(report, page, v, now);
        let drilled = at_drill_level(v);
        let v = drilled.as_ref().unwrap_or(v);
        let start = Instant::now();
        let data = match catch_unwind(AssertUnwindSafe(|| compute(s, &mut st, v, &applied, limits))) {
            Ok(d) => d,
            Err(_) => status("error", "internal error while computing the preview"),
        };
        let elapsed = start.elapsed();
        out.push(Timed {
            visual: v.name.clone(),
            visual_type: v.visual_type.clone(),
            elapsed,
            data,
            query: query_text(v, &applied),
        });
    }
    out
}

/// A visual the viewer just computed (not answered from the cache) for `page`: recorded when a
/// recording runs. Cheap when none does.
pub(super) fn viewed(
    s: &Session,
    page: &Page,
    v: &Visual,
    applied: &Applied,
    data: &Value,
    elapsed: Duration,
) {
    if !s.perf.recording() {
        return;
    }
    let t = Timed {
        visual: v.name.clone(),
        visual_type: v.visual_type.clone(),
        elapsed,
        data: data.clone(),
        query: query_text(v, applied),
    };
    s.perf.record(&page.name, crate::perf::Cause::View, t);
}

/// The DAX query BIcraft evaluates for visual `v` under `applied`.
pub(super) fn query_text(v: &Visual, applied: &Applied) -> QueryText {
    let mut notes = Vec::new();
    let mut groups: Vec<String> = Vec::new();
    let mut values: Vec<(String, String)> = Vec::new();
    for b in &v.fields {
        match &b.field {
            FieldRef::Column { table, column } => {
                let c = col(table, column);
                if !groups.contains(&c) {
                    groups.push(c);
                }
            }
            FieldRef::Measure { table: _, measure } => {
                values.push((
                    b.display_name.clone().unwrap_or_else(|| measure.clone()),
                    bracket(measure),
                ));
            }
            FieldRef::Aggregation {
                table,
                column,
                function,
            } => match agg_function(function) {
                Some(f) => {
                    let name = b
                        .display_name
                        .clone()
                        .unwrap_or_else(|| format!("{} of {column}", agg_label(function)));
                    values.push((name, format!("CALCULATE({f}({}))", col(table, column))));
                }
                None => notes.push(format!(
                    "the aggregation `{function}` of {table}[{column}] is not written"
                )),
            },
            FieldRef::Other { .. } => notes.push(format!(
                "a field of role {} (an expression or a hierarchy level) is not written",
                b.role
            )),
        }
    }
    if groups.is_empty() && values.is_empty() {
        notes.push("the visual has no field the query reads".into());
        return QueryText { text: None, notes };
    }
    let mut defines: Vec<String> = Vec::new();
    for item in &applied.items {
        match item {
            Item::Cond(c) => match filter_table(c) {
                Some(t) => defines.push(t),
                None => notes.push(
                    "a filter on a measure or an aggregation is applied to the visual's rows after the query: not written"
                        .into(),
                ),
            },
            Item::TopN(t) => notes.push(format!(
                "the top {} filter on {}[{}] is not written",
                t.n, t.table, t.column
            )),
        }
    }
    for why in &applied.ignored {
        notes.push(format!("not applied by BIcraft: {why}"));
    }
    let mut text = String::new();
    if !defines.is_empty() {
        text.push_str("DEFINE\n");
        for (i, d) in defines.iter().enumerate() {
            text.push_str(&format!("\tVAR __Filter{i} = {d}\n"));
        }
    }
    text.push_str("EVALUATE\n\tSUMMARIZECOLUMNS(");
    let mut args: Vec<String> = groups;
    args.extend((0..defines.len()).map(|i| format!("__Filter{i}")));
    args.extend(values.iter().map(|(n, e)| format!("{}, {e}", string(n))));
    for (i, a) in args.iter().enumerate() {
        text.push_str(if i == 0 { "\n\t\t" } else { ",\n\t\t" });
        text.push_str(a);
    }
    text.push_str("\n\t)\n");
    if text.len() > MAX_QUERY_BYTES {
        notes.push("the query text would be longer than 1 MiB: left out".into());
        return QueryText { text: None, notes };
    }
    QueryText {
        text: Some(text),
        notes,
    }
}

/// The DAX function of a PBIR aggregation (`Count` is a distinct count, `CountNonNull` counts the
/// non-blank values; the deviation and variance are the population forms the preview computes).
fn agg_function(function: &str) -> Option<&'static str> {
    Some(match function {
        "Sum" => "SUM",
        "Avg" => "AVERAGE",
        "Count" => "DISTINCTCOUNT",
        "Min" => "MIN",
        "Max" => "MAX",
        "CountNonNull" => "COUNTA",
        "Median" => "MEDIAN",
        "StandardDeviation" => "STDEV.P",
        "Variance" => "VAR.P",
        _ => return None,
    })
}

/// A filter condition over columns as a filter table: `TREATAS` for a list of values, else
/// `FILTER` over the columns it reads. `None` when it reads a measure or an aggregation.
fn filter_table(c: &Cond) -> Option<String> {
    let cols: Vec<String> = c
        .operands()
        .into_iter()
        .map(|o| match o {
            Operand::Col { table, column } => Some(col(table, column)),
            _ => None,
        })
        .collect::<Option<_>>()?;
    if let Cond::In { exprs, rows } = c
        && !rows.iter().flatten().any(|v| matches!(v, Val::Null))
    {
        let list = rows.iter().map(|r| tuple(r)).collect::<Vec<_>>().join(", ");
        let targets = exprs.iter().map(operand).collect::<Option<Vec<_>>>()?.join(", ");
        return Some(format!("TREATAS({{{list}}}, {targets})"));
    }
    let table = match cols.as_slice() {
        [one] => format!("ALL({one})"),
        many => format!(
            "CROSSJOIN({})",
            many.iter()
                .map(|c| format!("ALL({c})"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    Some(format!("FILTER({table}, {})", predicate(c)?))
}

/// A condition as a DAX boolean expression.
fn predicate(c: &Cond) -> Option<String> {
    Some(match c {
        Cond::In { exprs, rows } => {
            let left = match exprs.as_slice() {
                [one] => operand(one)?,
                many => format!(
                    "({})",
                    many.iter().map(operand).collect::<Option<Vec<_>>>()?.join(", ")
                ),
            };
            let list = rows.iter().map(|r| tuple(r)).collect::<Vec<_>>().join(", ");
            format!("{left} IN {{{list}}}")
        }
        Cond::Cmp { op, left, right } => {
            let l = operand(left)?;
            match (op, right) {
                (CmpOp::Eq, Val::Null) => format!("ISBLANK({l})"),
                _ => {
                    let o = match op {
                        CmpOp::Eq => "=",
                        CmpOp::Gt => ">",
                        CmpOp::Ge => ">=",
                        CmpOp::Lt => "<",
                        CmpOp::Le => "<=",
                    };
                    format!("{l} {o} {}", literal(right))
                }
            }
        }
        Cond::Text { op, left, right } => {
            let l = operand(left)?;
            let r = string(right);
            match op {
                TextOp::Contains => format!("CONTAINSSTRING({l}, {r})"),
                TextOp::StartsWith => format!("LEFT({l}, {}) = {r}", right.chars().count()),
                TextOp::EndsWith => format!("RIGHT({l}, {}) = {r}", right.chars().count()),
            }
        }
        Cond::Not(c) => format!("NOT({})", predicate(c)?),
        Cond::And(a, b) => format!("({} && {})", predicate(a)?, predicate(b)?),
        Cond::Or(a, b) => format!("({} || {})", predicate(a)?, predicate(b)?),
    })
}

fn operand(o: &Operand) -> Option<String> {
    match o {
        Operand::Col { table, column } => Some(col(table, column)),
        _ => None,
    }
}

/// One row of an `In` list: a value, or a tuple of values.
fn tuple(r: &[Val]) -> String {
    match r {
        [one] => literal(one),
        many => format!("({})", many.iter().map(literal).collect::<Vec<_>>().join(", ")),
    }
}

/// A value as a DAX literal.
fn literal(v: &Val) -> String {
    match v {
        Val::Null => "BLANK()".into(),
        Val::Num(x) if x.is_finite() => format!("{x}"),
        Val::Num(_) => "BLANK()".into(),
        Val::Text(t) => string(t),
        Val::Bool(true) => "TRUE".into(),
        Val::Bool(false) => "FALSE".into(),
        Val::Date(d) => date(*d),
    }
}

/// Days since 1899-12-30 as `DATE(y, m, d)`, plus `TIME(h, m, s)` when there is a time of day.
fn date(x: f64) -> String {
    if !x.is_finite() || x.abs() > 3_000_000.0 {
        return "BLANK()".into();
    }
    let days = x.floor();
    let secs = ((x - days) * 86_400.0).round() as i64;
    let (days, secs) = if secs >= 86_400 {
        (days as i64 + 1, secs - 86_400)
    } else {
        (days as i64, secs)
    };
    let (y, m, d) = bicraft_locale::calendar::civil_from_serial(days);
    if secs == 0 {
        format!("DATE({y}, {m}, {d})")
    } else {
        format!(
            "(DATE({y}, {m}, {d}) + TIME({}, {}, {}))",
            secs / 3600,
            (secs / 60) % 60,
            secs % 60
        )
    }
}

/// `'Table'[Column]`.
fn col(table: &str, column: &str) -> String {
    format!("'{}'{}", table.replace('\'', "''"), bracket(column))
}

/// `[Name]`.
fn bracket(name: &str) -> String {
    format!("[{}]", name.replace(']', "]]"))
}

/// A DAX string literal.
fn string(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn query_text_escapes_names_and_values() {
        let v: Visual = serde_json::from_value(json!({
            "name": "v",
            "visual_type": "tableEx",
            "fields": [
                {"role": "Values", "field": {"kind": "column", "table": "O'Brien", "column": "a]b"}},
                {"role": "Values", "field": {"kind": "measure", "table": "O'Brien", "measure": "Say \"hi\""}, "display_name": "Q\"x"}
            ]
        }))
        .unwrap();
        let q = query_text(&v, &Applied::default());
        let t = q.text.unwrap();
        assert!(t.contains("'O''Brien'[a]]b]"), "{t}");
        assert!(t.contains("\"Q\"\"x\", [Say \"hi\"]"), "{t}");
        let none: Visual = serde_json::from_value(json!({"name": "e"})).unwrap();
        let q = query_text(&none, &Applied::default());
        assert!(q.text.is_none() && !q.notes.is_empty());
    }

    #[test]
    fn literals_and_predicates() {
        assert_eq!(literal(&Val::Date(45_322.0)), "DATE(2024, 1, 31)");
        assert_eq!(
            literal(&Val::Date(45_322.5)),
            "(DATE(2024, 1, 31) + TIME(12, 0, 0))"
        );
        assert_eq!(literal(&Val::Num(f64::NAN)), "BLANK()");
        assert_eq!(literal(&Val::Bool(true)), "TRUE");
        let c = Operand::Col {
            table: "T".into(),
            column: "C".into(),
        };
        let cond = Cond::Or(
            Box::new(Cond::Text {
                op: TextOp::StartsWith,
                left: c.clone(),
                right: "Bé".into(),
            }),
            Box::new(Cond::Not(Box::new(Cond::Cmp {
                op: CmpOp::Eq,
                left: c.clone(),
                right: Val::Null,
            }))),
        );
        assert_eq!(
            filter_table(&cond).unwrap(),
            "FILTER(ALL('T'[C]), (LEFT('T'[C], 2) = \"Bé\" || NOT(ISBLANK('T'[C]))))"
        );
        let m = Cond::Cmp {
            op: CmpOp::Gt,
            left: Operand::Measure {
                table: "T".into(),
                name: "M".into(),
            },
            right: Val::Num(1.0),
        };
        assert!(filter_table(&m).is_none());
    }
}
