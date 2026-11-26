//! The filter context of See records (`visual.records`, [`crate::see_records`]): the report's,
//! the page's and the visual's filters and the page's slicers ([`super::filters::collect`]) as
//! DAX filter arguments of the records query, so the rows are those the visual summarizes.
//!
//! - A column condition over one table becomes `FILTER(ALL(<its columns>), <condition>)`.
//! - A condition over columns of several tables (a hierarchy slicer's tuples, an `Or` across
//!   tables) becomes `FILTER(<the records' table>, <condition>)`, the other tables' columns read
//!   through `RELATED`, when each of those tables is on the one side of a chain of active
//!   relationships from the records' table.
//! - A top N filter becomes `CALCULATETABLE(TOPN(n, VALUES(<column>), <by>, DESC|ASC), <the
//!   column conditions>)`, the top N computed under the other filters as the evaluator does.
//! - A condition on a measure or an aggregation keeps or drops the visual's points: it becomes
//!   `CALCULATETABLE(FILTER(<the visual's grouping columns>, <condition>), <the column
//!   conditions>)` (`VALUES` of one grouping column, else `SUMMARIZE` of the records' table by
//!   them), so the rows are those of the points the filter keeps, the measure computed under the
//!   other filters as the visual computes it.
//!
//! What cannot be applied (a measure filter on a visual without grouping columns, a measure
//! condition that also reads a column the visual does not group by, a table `RELATED` cannot
//! reach) is named in `not_applied`.

use bicraft_model::SemanticModel;
use bicraft_report::{Page, Report, Visual};

use super::filters::{CmpOp, Cond, Item, Operand, TextOp, TopN, Val, collect};

/// Most values written into one condition (a larger `In` list is not applied).
const MAX_VALUES: usize = 10_000;
/// Longest chain of relationships `RELATED` follows from the records' table.
const MAX_HOPS: usize = 8;

/// What the records are: the table whose rows they are and the visual's grouping columns.
pub(crate) struct RecordScope<'a> {
    /// The records' table (the model's name).
    pub fact: &'a str,
    /// The visual's grouping columns, `(table, column)`.
    pub groups: &'a [(String, String)],
    pub model: &'a SemanticModel,
}

/// The DAX filter arguments of a visual's filters.
#[derive(Debug, Default)]
pub(crate) struct RecordFilters {
    /// Filter arguments for `CALCULATETABLE`.
    pub args: Vec<String>,
    /// What was not applied, and why.
    pub not_applied: Vec<String>,
}

fn quote_table(t: &str) -> String {
    format!("'{}'", t.replace('\'', "''"))
}

fn column_ref(t: &str, c: &str) -> String {
    format!("{}[{}]", quote_table(t), c.replace(']', "]]"))
}

fn text_lit(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// A DAX literal; `None` for a blank.
fn literal(v: &Val) -> Option<String> {
    Some(match v {
        Val::Null => return None,
        Val::Num(x) if x.is_finite() => format!("{x}"),
        Val::Num(_) => return None,
        Val::Text(t) => text_lit(t),
        Val::Bool(b) => if *b { "TRUE()" } else { "FALSE()" }.to_string(),
        Val::Date(d) => {
            if !d.is_finite() || d.abs() > 3_000_000.0 {
                return None;
            }
            let days = d.floor();
            let secs = ((d - days) * 86_400.0).round() as i64;
            let (y, m, dd) = bicraft_locale::calendar::civil_from_serial(days as i64);
            if secs == 0 {
                format!("DATE({y}, {m}, {dd})")
            } else {
                format!(
                    "(DATE({y}, {m}, {dd}) + TIME({}, {}, {}))",
                    secs / 3600,
                    secs / 60 % 60,
                    secs % 60
                )
            }
        }
    })
}

/// The column of an operand (`None` for a measure or an aggregation).
fn col(o: &Operand) -> Option<(&str, &str)> {
    match o {
        Operand::Col { table, column } => Some((table, column)),
        _ => None,
    }
}

/// `left = value`, a blank as `ISBLANK(left)`.
fn equals(left: &str, v: &Val) -> String {
    match literal(v) {
        Some(l) => format!("{left} = {l}"),
        None => format!("ISBLANK({left})"),
    }
}

/// How a condition's operands are written in DAX.
type OperandDax<'a> = &'a dyn Fn(&Operand) -> Result<String, String>;

/// A column operand as its column reference; a measure or an aggregation is refused.
fn plain(o: &Operand) -> Result<String, String> {
    col(o)
        .map(|(t, c)| column_ref(t, c))
        .ok_or_else(|| "a condition on a measure or an aggregation".to_string())
}

/// A condition as a DAX boolean expression over its columns.
#[cfg(test)]
fn expr(c: &Cond, depth: usize) -> Result<String, String> {
    expr_with(c, depth, &plain)
}

/// A condition as a DAX boolean expression, its operands written by `left`.
fn expr_with(c: &Cond, depth: usize, left: OperandDax<'_>) -> Result<String, String> {
    if depth > 32 {
        return Err("a condition nests too deeply".into());
    }
    Ok(match c {
        Cond::In { exprs, rows } => {
            if rows.len() > MAX_VALUES {
                return Err(format!("a list of more than {MAX_VALUES} values"));
            }
            let cols: Vec<String> = exprs.iter().map(left).collect::<Result<_, _>>()?;
            if rows.is_empty() {
                return Ok("FALSE()".into());
            }
            if let [one] = cols.as_slice() {
                let mut vals = Vec::new();
                let mut blank = false;
                for r in rows {
                    match r.first().and_then(literal) {
                        Some(l) => vals.push(l),
                        None => blank = true,
                    }
                }
                let mut parts = Vec::new();
                if !vals.is_empty() {
                    parts.push(format!("{one} IN {{{}}}", vals.join(", ")));
                }
                if blank {
                    parts.push(format!("ISBLANK({one})"));
                }
                parts.join(" || ")
            } else {
                rows.iter()
                    .map(|r| {
                        let ands: Vec<String> = cols.iter().zip(r).map(|(c, v)| equals(c, v)).collect();
                        format!("({})", ands.join(" && "))
                    })
                    .collect::<Vec<_>>()
                    .join(" || ")
            }
        }
        Cond::Cmp { op, left: l, right } => {
            let l = left(l)?;
            match (op, literal(right)) {
                (CmpOp::Eq, _) => equals(&l, right),
                (_, None) => return Err("a comparison with a blank".into()),
                (op, Some(r)) => {
                    let sym = match op {
                        CmpOp::Gt => ">",
                        CmpOp::Ge => ">=",
                        CmpOp::Lt => "<",
                        CmpOp::Le => "<=",
                        CmpOp::Eq => "=",
                    };
                    format!("{l} {sym} {r}")
                }
            }
        }
        Cond::Text { op, left: l, right } => {
            let l = left(l)?;
            let n = right.chars().count();
            match op {
                TextOp::Contains => format!("CONTAINSSTRING({l}, {})", text_lit(right)),
                TextOp::StartsWith => format!("LEFT({l}, {n}) = {}", text_lit(right)),
                TextOp::EndsWith => format!("RIGHT({l}, {n}) = {}", text_lit(right)),
            }
        }
        Cond::Not(x) => format!("NOT({})", expr_with(x, depth + 1, left)?),
        Cond::And(a, b) => format!(
            "({}) && ({})",
            expr_with(a, depth + 1, left)?,
            expr_with(b, depth + 1, left)?
        ),
        Cond::Or(a, b) => format!(
            "({}) || ({})",
            expr_with(a, depth + 1, left)?,
            expr_with(b, depth + 1, left)?
        ),
    })
}

/// Whether `to` is on the one side of a chain of active relationships from `from` (so
/// `RELATED` reads its columns from a row of `from`).
fn reaches(model: &SemanticModel, from: &str, to: &str) -> bool {
    let is = |c: &Option<String>, k: &str| c.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(k));
    let mut seen: Vec<&str> = vec![from];
    let mut frontier: Vec<&str> = vec![from];
    for _ in 0..MAX_HOPS {
        let mut next = Vec::new();
        for t in &frontier {
            for r in model.relationships.iter().filter(|r| r.is_active) {
                // the many (from) side reads the one (to) side; a one-to-one both ways
                let step = if r.from_table.eq_ignore_ascii_case(t) && !is(&r.to_cardinality, "many") {
                    Some(r.to_table.as_str())
                } else if r.to_table.eq_ignore_ascii_case(t)
                    && is(&r.from_cardinality, "one")
                    && !is(&r.to_cardinality, "many")
                {
                    Some(r.from_table.as_str())
                } else {
                    None
                };
                if let Some(n) = step
                    && !seen.iter().any(|x| x.eq_ignore_ascii_case(n))
                {
                    if n.eq_ignore_ascii_case(to) {
                        return true;
                    }
                    seen.push(n);
                    next.push(n);
                }
            }
        }
        if next.is_empty() {
            return false;
        }
        frontier = next;
    }
    false
}

/// A column condition as a filter argument (`scope`: the records, for a condition over several
/// tables).
fn cond_arg(c: &Cond, scope: Option<&RecordScope<'_>>) -> Result<String, String> {
    let ops = c.operands();
    let mut cols: Vec<(&str, &str)> = Vec::new();
    for o in &ops {
        let Some(tc) = col(o) else {
            return Err("a condition on a measure or an aggregation".into());
        };
        if !cols.contains(&tc) {
            cols.push(tc);
        }
    }
    let Some((table, _)) = cols.first() else {
        return Err("a condition without a column".into());
    };
    if cols.iter().any(|(t, _)| !t.eq_ignore_ascii_case(table)) {
        // over several tables: the records' rows that meet it, the others through RELATED
        let Some(sc) = scope else {
            return Err("a condition over columns of several tables".into());
        };
        for (t, _) in &cols {
            if !t.eq_ignore_ascii_case(sc.fact) && !reaches(sc.model, sc.fact, t) {
                return Err(format!(
                    "a condition over several tables: `{t}` is not on the one side of a relationship from `{}`",
                    sc.fact
                ));
            }
        }
        let map = |o: &Operand| -> Result<String, String> {
            let (t, c) = col(o).ok_or("a condition on a measure or an aggregation")?;
            Ok(if t.eq_ignore_ascii_case(sc.fact) {
                column_ref(t, c)
            } else {
                format!("RELATED({})", column_ref(t, c))
            })
        };
        return Ok(format!(
            "FILTER({}, {})",
            quote_table(sc.fact),
            expr_with(c, 0, &map)?
        ));
    }
    let all: Vec<String> = cols.iter().map(|(t, c)| column_ref(t, c)).collect();
    Ok(format!(
        "FILTER(ALL({}), {})",
        all.join(", "),
        expr_with(c, 0, &plain)?
    ))
}

/// Whether a condition reads a measure or an aggregation.
fn on_measure(c: &Cond) -> bool {
    c.operands().iter().any(|o| col(o).is_none())
}

/// A condition on a measure or an aggregation as a filter of the visual's points (see the
/// module docs), under the column conditions `conds`.
fn measure_arg(c: &Cond, sc: &RecordScope<'_>, conds: &[String]) -> Result<String, String> {
    let points = match sc.groups {
        [] => {
            return Err("a filter on a measure of a visual without grouping columns".into());
        }
        [(t, c)] => format!("VALUES({})", column_ref(t, c)),
        many => format!(
            "SUMMARIZE({}, {})",
            quote_table(sc.fact),
            many.iter()
                .map(|(t, c)| column_ref(t, c))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let map = |o: &Operand| -> Result<String, String> {
        match o {
            Operand::Col { table, column } => {
                if sc
                    .groups
                    .iter()
                    .any(|(t, c)| t.eq_ignore_ascii_case(table) && c.eq_ignore_ascii_case(column))
                {
                    Ok(column_ref(table, column))
                } else {
                    Err(format!(
                        "a measure condition that also reads {}, which the visual does not group by",
                        column_ref(table, column)
                    ))
                }
            }
            other => by_expr(other),
        }
    };
    let keep = format!("FILTER({points}, {})", expr_with(c, 0, &map)?);
    Ok(if conds.is_empty() {
        keep
    } else {
        format!("CALCULATETABLE({keep}, {})", conds.join(", "))
    })
}

/// What a top N filter ranks by.
fn by_expr(o: &Operand) -> Result<String, String> {
    Ok(match o {
        Operand::Measure { name, .. } => format!("[{}]", name.replace(']', "]]")),
        Operand::Agg {
            table,
            column,
            function,
        } => {
            let f = match function.as_str() {
                "Sum" => "SUM",
                "Avg" => "AVERAGE",
                "Min" => "MIN",
                "Max" => "MAX",
                "Count" | "CountNonNull" => "COUNTA",
                "Median" => "MEDIAN",
                other => return Err(format!("a top N ranked by `{other}`")),
            };
            format!("CALCULATE({f}({}))", column_ref(table, column))
        }
        Operand::Col { .. } => return Err("a top N ranked by a column".into()),
    })
}

fn top_arg(t: &TopN, conds: &[String]) -> Result<String, String> {
    let top = format!(
        "TOPN({}, VALUES({}), {}, {})",
        t.n,
        column_ref(&t.table, &t.column),
        by_expr(&t.by)?,
        if t.desc { "DESC" } else { "ASC" }
    );
    Ok(if conds.is_empty() {
        top
    } else {
        format!("CALCULATETABLE({top}, {})", conds.join(", "))
    })
}

/// The filter arguments of every filter that applies to `v` on `page` (see the module docs).
pub(crate) fn record_filters(
    report: &Report,
    page: &Page,
    v: &Visual,
    now: f64,
    scope: &RecordScope<'_>,
) -> RecordFilters {
    let applied = collect(report, page, v, now);
    let mut out = RecordFilters {
        not_applied: applied.ignored.clone(),
        ..RecordFilters::default()
    };
    for it in &applied.items {
        if let Item::Cond(c) = it
            && !on_measure(c)
        {
            match cond_arg(c, Some(scope)) {
                Ok(a) => out.args.push(a),
                Err(e) => out.not_applied.push(e),
            }
        }
    }
    // the filters of the visual's points (measures, top N), under the column conditions
    let conds = out.args.clone();
    for it in &applied.items {
        let r = match it {
            Item::Cond(c) if on_measure(c) => measure_arg(c, scope, &conds),
            Item::Cond(_) => continue,
            Item::TopN(t) => top_arg(t, &conds),
        };
        match r {
            Ok(a) => out.args.push(a),
            Err(e) => out.not_applied.push(e),
        }
    }
    out.not_applied.sort();
    out.not_applied.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(t: &str, n: &str) -> Operand {
        Operand::Col {
            table: t.into(),
            column: n.into(),
        }
    }

    #[test]
    fn conditions_become_dax() {
        let cat = c("Product", "Category");
        let f = Cond::In {
            exprs: vec![cat.clone()],
            rows: vec![vec![Val::Text("Toys".into())], vec![Val::Null]],
        };
        assert_eq!(
            cond_arg(&f, None).unwrap(),
            "FILTER(ALL('Product'[Category]), 'Product'[Category] IN {\"Toys\"} || ISBLANK('Product'[Category]))"
        );
        let d = Cond::And(
            Box::new(Cond::Cmp {
                op: CmpOp::Ge,
                left: c("Cal", "Day"),
                right: Val::Date(45_322.0),
            }),
            Box::new(Cond::Cmp {
                op: CmpOp::Lt,
                left: c("Cal", "Day"),
                right: Val::Date(45_322.5),
            }),
        );
        assert_eq!(
            cond_arg(&d, None).unwrap(),
            "FILTER(ALL('Cal'[Day]), ('Cal'[Day] >= DATE(2024, 1, 31)) && ('Cal'[Day] < (DATE(2024, 1, 31) + TIME(12, 0, 0))))"
        );
        let t = Cond::Not(Box::new(Cond::Text {
            op: TextOp::StartsWith,
            left: c("O'Brien", "N]x"),
            right: "a\"b".into(),
        }));
        assert_eq!(
            cond_arg(&t, None).unwrap(),
            "FILTER(ALL('O''Brien'[N]]x]), NOT(LEFT('O''Brien'[N]]x], 3) = \"a\"\"b\"))"
        );
        // measures and several tables are not applied
        let m = Cond::Cmp {
            op: CmpOp::Gt,
            left: Operand::Measure {
                table: "S".into(),
                name: "Total".into(),
            },
            right: Val::Num(1.0),
        };
        assert!(cond_arg(&m, None).is_err());
        let two = Cond::In {
            exprs: vec![c("A", "x"), c("B", "y")],
            rows: vec![vec![Val::Num(1.0), Val::Num(2.0)]],
        };
        assert!(cond_arg(&two, None).is_err());
        let tuple = Cond::In {
            exprs: vec![c("A", "x"), c("A", "y")],
            rows: vec![vec![Val::Num(1.0), Val::Null]],
        };
        assert_eq!(
            cond_arg(&tuple, None).unwrap(),
            "FILTER(ALL('A'[x], 'A'[y]), ('A'[x] = 1 && ISBLANK('A'[y])))"
        );
        let mut deep = m.clone();
        for _ in 0..40 {
            deep = Cond::Not(Box::new(deep));
        }
        assert!(expr(&deep, 0).is_err());
        let top = TopN {
            table: "Product".into(),
            column: "Category".into(),
            n: 2,
            by: Operand::Agg {
                table: "Sales".into(),
                column: "Amount".into(),
                function: "Sum".into(),
            },
            desc: true,
        };
        assert_eq!(
            top_arg(&top, &[]).unwrap(),
            "TOPN(2, VALUES('Product'[Category]), CALCULATE(SUM('Sales'[Amount])), DESC)"
        );
        assert!(
            top_arg(
                &TopN {
                    by: c("S", "x"),
                    ..top
                },
                &[]
            )
            .is_err()
        );
    }
}
