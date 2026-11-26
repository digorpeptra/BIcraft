//! `dax.apply_define`: the DAX query view's "Update model" (Power BI Desktop's CodeLens above a
//! query's `DEFINE MEASURE`, and its "Update model with changes" button; Microsoft Learn, "DAX
//! query view"). The query is parsed with `bicraft-dax`; each `MEASURE Table[Name] = …` of its
//! DEFINE block (or the ones named) becomes a model measure: added when the model has none of that
//! name, its expression replaced when it has one in that table. Every change is one edit (one undo
//! step) through [`crate::edit::apply`], so the expressions are validated against the model with
//! the other changes made, and nothing changes on an error.

use bicraft_dax::Severity;
use bicraft_dax::ast::Define;
use bicraft_model::Measure;
use serde_json::{Value, json};

use crate::Session;
use crate::edit::{self, Edit};
use crate::params::{allow, str_param};

/// Most measures one call applies.
const MAX_MEASURES: usize = 10_000;

/// One `DEFINE MEASURE` of the query: its table, name and expression text.
struct Defined {
    table: String,
    name: String,
    expression: String,
}

/// The query's `DEFINE MEASURE` entries, in order; an error for a query that does not parse.
fn defined(query: &str) -> Result<Vec<Defined>, String> {
    let (parsed, diags) = bicraft_dax::parse_query(query);
    let errors: Vec<String> = diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .take(5)
        .map(|d| format!("line {}, column {}: {}", d.line, d.column, d.message))
        .collect();
    let parsed = match parsed {
        Some(q) if errors.is_empty() => q,
        _ => {
            return Err(format!(
                "the query does not parse, nothing was changed: {}",
                if errors.is_empty() {
                    "no DEFINE or EVALUATE".to_string()
                } else {
                    errors.join("; ")
                }
            ));
        }
    };
    let mut out = Vec::new();
    for d in &parsed.defines {
        if let Define::Measure {
            table,
            name,
            expr_span,
            ..
        } = d
        {
            let text = query
                .get(expr_span.start..expr_span.end)
                .ok_or_else(|| format!("internal: the expression of measure `{name}` is out of the query"))?;
            out.push(Defined {
                table: table.clone(),
                name: name.clone(),
                expression: text.trim().to_string(),
            });
        }
    }
    Ok(out)
}

/// The names of `measures`, when given: a list of strings.
fn wanted(p: &Value) -> Result<Option<Vec<String>>, String> {
    match p.get("measures") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "`measures` must be a list of measure names".to_string())
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err("`measures` must be a list of measure names".into()),
    }
}

/// `dax.apply_define {query, measures?, force?}`.
pub(crate) fn dax_apply_define(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["query", "measures", "force"])?;
    let query = str_param(p, "query")?;
    let all = defined(query)?;
    if all.is_empty() {
        return Err("the query defines no measure (DEFINE MEASURE Table[Name] = …)".into());
    }
    let picked: Vec<Defined> = match wanted(p)? {
        None => all,
        Some(names) => {
            if names.is_empty() {
                return Err(
                    "`measures` is empty: name the measures to apply, or leave it out for all".into(),
                );
            }
            for n in &names {
                if !all.iter().any(|d| d.name.eq_ignore_ascii_case(n)) {
                    return Err(format!("the query defines no measure named `{n}`"));
                }
            }
            all.into_iter()
                .filter(|d| names.iter().any(|n| d.name.eq_ignore_ascii_case(n)))
                .collect()
        }
    };
    if picked.len() > MAX_MEASURES {
        return Err(format!("at most {MAX_MEASURES} measures at once"));
    }
    for (i, d) in picked.iter().enumerate() {
        if d.table.trim().is_empty() {
            return Err(format!(
                "DEFINE MEASURE [{}] names no table: write Table[{}]",
                d.name, d.name
            ));
        }
        edit::check_name("measure", &d.name)?;
        if picked
            .iter()
            .take(i)
            .any(|e| e.name.eq_ignore_ascii_case(&d.name))
        {
            return Err(format!("the query defines measure `{}` twice", d.name));
        }
    }
    // what each would do, read before the edit (an unchanged one is no edit)
    let current = crate::commands::model(s)?;
    let mut changes = Vec::new();
    for d in &picked {
        let ti = edit::table_index(current, &d.table)
            .map_err(|e| format!("DEFINE MEASURE {}[{}]: {e}", d.table, d.name))?;
        let table = current.tables.get(ti).map(|t| t.name.clone()).unwrap_or_default();
        let action = match edit::measure_position(current, &d.name) {
            Ok((mti, mi)) => {
                let home = current.tables.get(mti);
                if mti != ti {
                    return Err(format!(
                        "measure `{}` is in table `{}`, not `{table}`: define it as {}",
                        d.name,
                        home.map_or("", |t| t.name.as_str()),
                        edit::dax_column(home.map_or("", |t| t.name.as_str()), &d.name)
                    ));
                }
                let same = home
                    .and_then(|t| t.measures.get(mi))
                    .is_some_and(|m| m.expression.trim() == d.expression);
                if same { "unchanged" } else { "updated" }
            }
            Err(_) => "added",
        };
        changes.push(json!({"table": table, "measure": d.name, "action": action}));
    }
    let count = |a: &str| changes.iter().filter(|c| c["action"] == a).count();
    let (added, updated, unchanged) = (count("added"), count("updated"), count("unchanged"));
    if added + updated == 0 {
        return Ok(json!({
            "changes": changes, "added": 0, "updated": 0, "unchanged": unchanged, "changed": false,
        }));
    }
    let mut out = edit::apply(s, p, |m| {
        let mut e = Edit::new(json!({}));
        for d in &picked {
            let ti = edit::table_index(m, &d.table)?;
            match edit::measure_position(m, &d.name) {
                Ok((mti, mi)) => {
                    let t = edit::table_at(m, mti)?;
                    let table = t.name.clone();
                    let x = t
                        .measures
                        .get_mut(mi)
                        .ok_or_else(|| "internal: measure index out of range".to_string())?;
                    if x.expression.trim() == d.expression {
                        continue;
                    }
                    x.expression = d.expression.clone();
                    e.check(
                        format!("measure {}", edit::dax_measure(&d.name)),
                        &d.expression,
                        Some(&table),
                    );
                }
                Err(_) => {
                    edit::check_measure_name_free(m, ti, &d.name, None)?;
                    let t = edit::table_at(m, ti)?;
                    let table = t.name.clone();
                    t.measures.push(Measure {
                        name: d.name.clone(),
                        expression: d.expression.clone(),
                        lineage_tag: Some(edit::new_guid()),
                        ..Default::default()
                    });
                    e.check(
                        format!("measure {}", edit::dax_measure(&d.name)),
                        &d.expression,
                        Some(&table),
                    );
                }
            }
        }
        Ok(e)
    })?;
    if let Some(o) = out.as_object_mut() {
        o.insert("changes".into(), Value::Array(changes));
        o.insert("added".into(), json!(added));
        o.insert("updated".into(), json!(updated));
        o.insert("unchanged".into(), json!(unchanged));
        o.insert("changed".into(), json!(true));
    }
    Ok(out)
}
