//! `visual.records`: "See records" of a data point, as Power BI Desktop's right-click on a point
//! shows the rows behind it (docs/mcp.md "See records"). The point is what the renderer's hit
//! map gives a click (`[{c, v}]`: a data column of the visual and its value, as `view.interact`
//! `select` takes it); the rows are those of the table the visual summarizes (the home table of
//! its first measure, else the table of its first aggregated column, else of its first column)
//! under the point's values, computed by BIcraft's DAX evaluator as
//! `EVALUATE CALCULATETABLE('Table', <column> = <value>, …)`, so a value of a related table
//! filters through the model's relationships. A mark that stands for several points (a
//! histogram bin, a Venn set of BIcraft's visuals) sends them all as `points`: the rows of
//! every point, the `UNION` of one `CALCULATETABLE` per point.
//!
//! The report's, the page's and the visual's filters and the page's slicer selections (the
//! reader's, from the view state, as the visual shows them) are added to the query as filter
//! arguments ([`crate::preview::records`]): a filter on a measure keeps the visual's points it
//! keeps (over its grouping columns), a condition over several tables reads the other tables
//! through `RELATED`; what cannot be applied is listed in `not_applied`. When another visual's
//! selection cross-highlights the visual, its filters come from the highlight report (the
//! selection added): the rows of the highlighted part, `cross_highlight` naming the source.

use bicraft_model::{DataType, SemanticModel};
use bicraft_report::FieldRef;
use serde_json::{Value, json};

use crate::Session;
use crate::params::allow;

/// Rows returned when `limit` is not given.
const DEFAULT_LIMIT: u64 = 1_000;
/// Most rows returned.
const MAX_LIMIT: u64 = 10_000;
/// Most points of one mark (the cap of a `view.interact` selection).
const MAX_POINTS: usize = 1_000;

fn quote_table(t: &str) -> String {
    format!("'{}'", t.replace('\'', "''"))
}

fn column_ref(t: &str, c: &str) -> String {
    format!("{}[{}]", quote_table(t), c.replace(']', "]]"))
}

/// A DAX literal of `v` for a column of type `ty` (`None`: a blank).
fn literal(v: &Value, ty: DataType) -> Result<Option<String>, String> {
    Ok(Some(match v {
        Value::Null => return Ok(None),
        Value::Bool(b) => if *b { "TRUE()" } else { "FALSE()" }.to_string(),
        Value::Number(n) if ty == DataType::String => format!("\"{n}\""),
        Value::Number(n) => {
            let x = n
                .as_f64()
                .filter(|x| x.is_finite())
                .ok_or("a point value is not a number")?;
            format!("{x}")
        }
        Value::String(s) if ty == DataType::DateTime => {
            let serial =
                crate::preview::filters::parse_as_of(s).map_err(|_| format!("`{s}` is not a date"))?;
            // the serial date: DAX reads a number as a date where a date is compared
            let days = serial.floor();
            let secs = ((serial - days) * 86_400.0).round();
            let (y, m, d) = bicraft_locale::calendar::civil_from_serial(days as i64);
            if secs == 0.0 {
                format!("DATE({y}, {m}, {d})")
            } else {
                let s = secs as i64;
                format!(
                    "DATE({y}, {m}, {d}) + TIME({}, {}, {})",
                    s / 3600,
                    s / 60 % 60,
                    s % 60
                )
            }
        }
        Value::String(s) => format!("\"{}\"", s.replace('"', "\"\"")),
        _ => return Err("a point value must be text, a number, true/false or null".into()),
    }))
}

/// The table whose rows the visual summarizes.
fn fact_table(v: &bicraft_report::Visual, model: &SemanticModel) -> Option<String> {
    let home = |measure: &str, table: &str| {
        model
            .tables
            .iter()
            .find(|t| t.measures.iter().any(|m| m.name.eq_ignore_ascii_case(measure)))
            .map(|t| t.name.clone())
            .or_else(|| (!table.is_empty()).then(|| table.to_string()))
    };
    let fields = || v.fields.iter().map(|b| &b.field);
    fields()
        .find_map(|f| match f {
            FieldRef::Measure { table, measure } => home(measure, table),
            _ => None,
        })
        .or_else(|| {
            fields().find_map(|f| match f {
                FieldRef::Aggregation { table, .. } => Some(table.clone()),
                _ => None,
            })
        })
        .or_else(|| {
            fields().find_map(|f| match f {
                FieldRef::Column { table, .. } => Some(table.clone()),
                _ => None,
            })
        })
        .and_then(|t| {
            model
                .tables
                .iter()
                .find(|x| x.name.eq_ignore_ascii_case(&t))
                .map(|x| x.name.clone())
        })
}

/// `visual.records`; see the module docs.
pub(crate) fn visual_records(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "point", "points", "limit"])?;
    let limit = match p.get("limit") {
        None | Some(Value::Null) => DEFAULT_LIMIT,
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=MAX_LIMIT).contains(n))
            .ok_or_else(|| format!("`limit` must be a whole number from 1 to {MAX_LIMIT}"))?,
    };
    let base = s.report.as_ref().ok_or("no report is open")?;
    // the reader's state (slicer selections, cross-filters), as the visual shows it
    let effective = s.interactions.effective(base);
    let report = effective.as_ref().map_or(base, |e| &e.data);
    let model = s.model.as_ref().ok_or("no model is open (file.open first)")?;
    let (page, v) = crate::interact::find_visual(report, p)?;
    // a mark standing for several points (a histogram bin, a Venn set, a Sankey node of
    // BIcraft's visuals) sends them all as `points`, `point` the first: the rows of every one
    let mut points: Vec<Vec<crate::interact::Pv>> = Vec::new();
    match p.get("points") {
        None | Some(Value::Null) => {}
        Some(list) => {
            let list = list
                .as_array()
                .ok_or("`points` must be an array of points ([{c, v}] each)")?;
            if list.len() > MAX_POINTS {
                return Err(format!("`points` holds at most {MAX_POINTS} points"));
            }
            for x in list {
                let pv = crate::interact::resolve_point(v, x, Some(model))?;
                if !points.contains(&pv) {
                    points.push(pv);
                }
            }
        }
    }
    if let Some(pt) = p.get("point").filter(|x| !x.is_null()) {
        let pv = crate::interact::resolve_point(v, pt, Some(model))?;
        if !points.contains(&pv) {
            points.insert(0, pv);
        }
    }
    let point = points.first().cloned().unwrap_or_default();
    let table = fact_table(v, model).ok_or_else(|| {
        format!(
            "visual `{}` summarizes no table of the model: it has no records to show",
            v.name
        )
    })?;
    // each point's conditions, ANDed in its own CALCULATETABLE; several points: their union
    // (their rows are distinct, since two points differ on a column)
    let mut by_point: Vec<Vec<String>> = Vec::new();
    for pt in &points {
        let mut conds = Vec::new();
        for pv in pt {
            let col = column_ref(&pv.table, &pv.column);
            conds.push(match literal(&pv.value, pv.ty)? {
                Some(l) => format!("{col} = {l}"),
                None => format!("ISBLANK({col})"),
            });
        }
        by_point.push(conds);
    }
    if by_point.is_empty() {
        by_point.push(Vec::new());
    }
    // a visual another visual's selection cross-highlights: the rows of the highlighted part,
    // its filters taken from the highlight report (the selection added to them)
    let highlighted = effective.as_ref().and_then(|e| {
        let on = e
            .highlight_targets
            .iter()
            .any(|(pg, vis)| *pg == page.name && *vis == v.name);
        let hp = e.highlight.as_ref()?.pages.iter().find(|x| x.name == page.name)?;
        let hv = hp.visuals.iter().find(|x| x.name == v.name)?;
        on.then_some((e.highlight.as_ref()?, hp, hv))
    });
    let source = highlighted
        .as_ref()
        .and_then(|_| s.interactions.selection_source(&page.name).map(str::to_string));
    let (freport, fpage, fvisual) = highlighted.unwrap_or((report, page, v));
    let groups = grouping_columns(v, model);
    let scope = crate::preview::records::RecordScope {
        fact: &table,
        groups: &groups,
        model,
    };
    let ctx = crate::preview::records::record_filters(
        freport,
        fpage,
        fvisual,
        crate::preview::filters::clock(s),
        &scope,
    );
    let wrap = |rows: String| {
        let parts: Vec<String> = by_point
            .iter()
            .map(|conds| {
                let mut filters = conds.clone();
                filters.extend(ctx.args.iter().cloned());
                if filters.is_empty() {
                    rows.clone()
                } else {
                    format!("CALCULATETABLE({rows}, {})", filters.join(", "))
                }
            })
            .collect();
        match parts.as_slice() {
            [one] => format!("EVALUATE {one}"),
            _ => format!("EVALUATE UNION({})", parts.join(", ")),
        }
    };
    let mut query = wrap(quote_table(&table));
    let mut r = crate::preview::dax_evaluate(s, &json!({"query": query, "max_rows": limit}));
    // calculated columns the data does not hold (added in BIcraft, a literal table's): their
    // expressions computed row by row instead
    let has_calc = model_table_columns(model, &table)
        .iter()
        .any(|c| c.expression.is_some());
    if r.is_err() && has_calc {
        let cols: Vec<String> = model_table_columns(model, &table)
            .iter()
            .map(|c| {
                let value = match &c.expression {
                    Some(e) => format!("({e})"),
                    None => column_ref(&table, &c.name),
                };
                format!("\"{}\", {value}", c.name.replace('"', "\"\""))
            })
            .collect();
        let q = wrap(format!(
            "SELECTCOLUMNS({}, {})",
            quote_table(&table),
            cols.join(", ")
        ));
        if let Ok(v) = crate::preview::dax_evaluate(s, &json!({"query": q, "max_rows": limit})) {
            query = q;
            r = Ok(v);
        }
    }
    let r = r?;
    let res = r.pointer("/results/0").ok_or("the evaluator returned no table")?;
    let cols: Vec<String> = res
        .get("columns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| c.get("name").and_then(Value::as_str).map(str::to_string))
        .collect();
    // `Table[Column]` as the Data view names a column: `Column`
    let short = |n: &str| -> String {
        n.strip_suffix(']')
            .and_then(|x| x.rsplit_once('[').map(|(_, c)| c.replace("]]", "]")))
            .unwrap_or_else(|| n.to_string())
    };
    let types: Vec<Value> = res
        .get("columns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| json!({"name": short(c.get("name").and_then(Value::as_str).unwrap_or("")), "data_type": c.get("type")}))
        .collect();
    let rows: Vec<Value> = res
        .get("rows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|row| {
            Value::Array(
                cols.iter()
                    .map(|c| row.get(c).cloned().unwrap_or(Value::Null))
                    .collect(),
            )
        })
        .collect();
    Ok(json!({
        "page": page.name,
        "visual": v.name,
        "table": table,
        "point": point.iter().map(|pv| json!({"table": pv.table, "column": pv.column, "value": pv.value})).collect::<Vec<_>>(),
        "points": points.len(),
        "query": query,
        "columns": types,
        "rows": rows,
        "row_count": res.get("row_count").cloned().unwrap_or(Value::Null),
        "truncated": res.get("truncated").cloned().unwrap_or(Value::Bool(false)),
        "filters_applied": if ctx.args.is_empty() { "point" } else { "point and filters" },
        "filters": ctx.args,
        "not_applied": ctx.not_applied,
        "cross_highlight": source,
    }))
}

/// The visual's grouping columns (its columns outside the tooltips), as the model names them.
fn grouping_columns(v: &bicraft_report::Visual, model: &SemanticModel) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for b in &v.fields {
        let FieldRef::Column { table, column } = &b.field else {
            continue;
        };
        if b.role.eq_ignore_ascii_case("Tooltips") {
            continue;
        }
        let Some(t) = model.tables.iter().find(|t| t.name.eq_ignore_ascii_case(table)) else {
            continue;
        };
        let Some(c) = t.columns.iter().find(|c| c.name.eq_ignore_ascii_case(column)) else {
            continue;
        };
        let key = (t.name.clone(), c.name.clone());
        if !out.contains(&key) {
            out.push(key);
        }
    }
    out
}

/// The columns of table `name` (without the hidden row number).
fn model_table_columns<'m>(model: &'m SemanticModel, name: &str) -> Vec<&'m bicraft_model::Column> {
    model
        .tables
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case(name))
        .map(|t| t.columns.iter().filter(|c| !c.is_row_number()).collect())
        .unwrap_or_default()
}
