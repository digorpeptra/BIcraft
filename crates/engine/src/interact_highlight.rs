//! A data-point selection in a bookmark: the container's `highlight` (public PBIR schema
//! `bookmark` 2.1.0, `VisualContainerState.highlight`, "Cross-highlights captured in the
//! bookmark"): `{selection: [SelectorsByColumn]}`, one entry per selected point whose `dataMap`
//! gives, for each grouping column (keyed by its `Table.Column` query name), a
//! `DataRepetitionSelector` `{scopeId: <Comparison expression>}` equal to the point's value.
//!
//! `bookmark.capture` writes the selection the session shows (the source visual's container);
//! applying a bookmark (`bookmark.apply`, a reader's bookmark) selects those points again in
//! that visual. The shape follows the public schema; how Desktop fills it is not observed
//! (docs/gaps.md G29). A selection BIcraft cannot read back (another selector form, a column the
//! visual no longer has) is left out.

use bicraft_model::DataType;
use bicraft_report::{FieldRef, Page, Visual};
use serde_json::{Map, Value, json};

use super::{Interactions, MAX_POINT_COLUMNS, MAX_POINTS, Pv, Selection, value_literal};

/// The `highlight` of visual `visual` on page `page`: the session's data-point selection there,
/// `None` when the visual is not its source.
pub(crate) fn capture(view: &Interactions, page: &str, visual: &str) -> Option<Value> {
    let sel = view.selections.get(page).filter(|s| s.visual == visual)?;
    let selection: Vec<Value> = sel
        .points
        .iter()
        .take(MAX_POINTS)
        .map(|p| {
            let mut map = Map::new();
            for x in p {
                let scope = json!({"Comparison": {
                    "ComparisonKind": 0,
                    "Left": {"Column": {"Expression": {"SourceRef": {"Entity": x.table}}, "Property": x.column}},
                    "Right": {"Literal": {"Value": value_literal(&x.value, x.ty)}},
                }});
                map.insert(format!("{}.{}", x.table, x.column), json!([{"scopeId": scope}]));
            }
            json!({"dataMap": map})
        })
        .collect();
    (!selection.is_empty()).then(|| json!({"selection": selection}))
}

/// A literal's value and the type it is written in (`'text'`, `12L`, `1.5D`, `2M`,
/// `datetime'2024-01-31T00:00:00'`, `true`, `null`).
fn read_literal(s: &str) -> Option<(Value, DataType)> {
    let s = s.trim();
    if s == "null" {
        return Some((Value::Null, DataType::Unknown));
    }
    if s == "true" || s == "false" {
        return Some((Value::Bool(s == "true"), DataType::Boolean));
    }
    if let Some(d) = s.strip_prefix("datetime'").and_then(|x| x.strip_suffix('\'')) {
        return Some((json!(d), DataType::DateTime));
    }
    if let Some(t) = s.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')) {
        return Some((json!(t.replace("''", "'")), DataType::String));
    }
    let (num, ty) = match s.chars().last()? {
        'L' => (s.get(..s.len() - 1)?, DataType::Int64),
        'D' => (s.get(..s.len() - 1)?, DataType::Double),
        'M' => (s.get(..s.len() - 1)?, DataType::Decimal),
        _ => (s, DataType::Double),
    };
    if ty == DataType::Int64
        && let Ok(i) = num.parse::<i64>()
    {
        return Some((json!(i), ty));
    }
    let x: f64 = num.parse().ok().filter(|x: &f64| x.is_finite())?;
    // a whole number keeps its integer form, as the data rows hold it
    let v = if x.fract() == 0.0 && x.abs() < 9.0e15 {
        json!(x as i64)
    } else {
        json!(x)
    };
    Some((v, ty))
}

/// The points a container's `highlight` selects in visual `v` (by its fields' columns).
fn points_of(v: &Visual, highlight: &Value) -> Vec<Vec<Pv>> {
    let mut out: Vec<Vec<Pv>> = Vec::new();
    let Some(list) = highlight.get("selection").and_then(Value::as_array) else {
        return out;
    };
    for entry in list.iter().take(MAX_POINTS) {
        let Some(map) = entry.get("dataMap").and_then(Value::as_object) else {
            continue;
        };
        let mut point: Vec<Pv> = Vec::new();
        for sel in map.values().take(MAX_POINT_COLUMNS) {
            let Some(cmp) = sel
                .as_array()
                .and_then(|l| l.first())
                .and_then(|x| x.pointer("/scopeId/Comparison"))
            else {
                continue;
            };
            let col = cmp.pointer("/Left/Column");
            let table = col
                .and_then(|c| c.pointer("/Expression/SourceRef/Entity"))
                .and_then(Value::as_str);
            let column = col.and_then(|c| c.get("Property")).and_then(Value::as_str);
            let lit = cmp.pointer("/Right/Literal/Value").and_then(Value::as_str);
            let (Some(table), Some(column), Some((value, ty))) = (table, column, lit.and_then(read_literal))
            else {
                continue;
            };
            let Some(c) = v.fields.iter().position(|b| {
                matches!(&b.field, FieldRef::Column { table: t, column: k }
                    if t.eq_ignore_ascii_case(table) && k.eq_ignore_ascii_case(column))
            }) else {
                continue;
            };
            if point.iter().any(|x| x.c == c) {
                continue;
            }
            point.push(Pv {
                c,
                table: table.to_string(),
                column: column.to_string(),
                ty,
                value,
            });
        }
        if !point.is_empty() {
            out.push(point);
        }
    }
    out
}

/// Restores the data-point selection a bookmark's section holds for `page` (the first container
/// of `visuals` with a readable `highlight`); without one the page has no selection. True when
/// a selection was restored.
pub(crate) fn restore<'a>(
    view: &mut Interactions,
    page: &Page,
    containers: Option<&Map<String, Value>>,
    visuals: impl Iterator<Item = &'a Visual>,
) -> bool {
    view.selections.remove(&page.name);
    let Some(containers) = containers else {
        return false;
    };
    for v in visuals {
        let Some(h) = containers.get(&v.name).and_then(|c| c.get("highlight")) else {
            continue;
        };
        let points = points_of(v, h);
        if !points.is_empty() {
            view.selections.insert(
                page.name.clone(),
                Selection {
                    visual: v.name.clone(),
                    points,
                },
            );
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_read_back() {
        assert_eq!(
            read_literal("'O''Brien'"),
            Some((json!("O'Brien"), DataType::String))
        );
        assert_eq!(read_literal("12L"), Some((json!(12), DataType::Int64)));
        assert_eq!(read_literal("1.5D"), Some((json!(1.5), DataType::Double)));
        assert_eq!(read_literal("3D"), Some((json!(3), DataType::Double)));
        assert_eq!(
            read_literal("datetime'2024-01-31T00:00:00'"),
            Some((json!("2024-01-31T00:00:00"), DataType::DateTime))
        );
        assert_eq!(read_literal("true"), Some((json!(true), DataType::Boolean)));
        assert_eq!(read_literal("x"), None);
        assert_eq!(read_literal(""), None);
    }
}
