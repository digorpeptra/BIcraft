//! Matrix subtotals and grand totals (`data.subtotals`): the visual evaluated again at each
//! displayed level of its row and column hierarchies, so a subtotal is the measure in that
//! level's filter context (Desktop evaluates it, it does not add the cells up: a ratio's total
//! is the ratio of the totals). Also the displayed depth of each hierarchy (`data.row_levels`,
//! `data.column_levels`): Desktop 2.158 shows a matrix collapsed to its top level unless its
//! `expansionStates` expand the levels below.

use bicraft_report::Visual;
use serde_json::{Value, json};

use super::{Ctx, Fail, Item, Miss, Plan, Query, finish_data, format, general, grid};

/// Most levels of one hierarchy evaluated (Desktop has no such limit; deeper levels are rare;
/// the renderers draw as many, `MAX_MATRIX_LEVELS` in `crates/render/src/tables.rs`).
const MAX_DEPTH: usize = 8;

/// Indices of `v.fields` that group the matrix by `role` (`Rows` or `Columns`), in order.
pub(super) fn group_fields(v: &Visual, is_group: &[bool], role: &str) -> Vec<usize> {
    v.fields
        .iter()
        .enumerate()
        .filter(|(i, b)| b.role.eq_ignore_ascii_case(role) && is_group.get(*i).copied().unwrap_or(false))
        .map(|(i, _)| i)
        .take(MAX_DEPTH)
        .collect()
}

/// Most toggled members read from one expansion state.
const MAX_TOGGLED: usize = 2000;

/// The `expansionStates` entry of `role`.
fn state_of<'a>(v: &'a Visual, role: &str) -> Option<&'a Value> {
    let states = v.visual_extra.get("expansionStates").and_then(Value::as_array);
    states.into_iter().flatten().find(|e| {
        e.get("roles").and_then(Value::as_array).is_some_and(|r| {
            r.iter()
                .any(|x| x.as_str().is_some_and(|s| s.eq_ignore_ascii_case(role)))
        })
    })
}

/// How many leading projections of `role` are marked `active: true`.
fn active_levels(v: &Visual, role: &str) -> usize {
    v.visual_extra
        .get("query")
        .and_then(|q| q.get("queryState"))
        .and_then(Value::as_object)
        .and_then(|s| s.iter().find(|(k, _)| k.eq_ignore_ascii_case(role)))
        .and_then(|(_, r)| r.get("projections"))
        .and_then(Value::as_array)
        .map_or(0, |ps| {
            ps.iter()
                .take_while(|p| p.get("active").and_then(Value::as_bool) == Some(true))
                .count()
        })
}

/// One hierarchy's expansion (Desktop 2.158's `expansionStates`): a level whose entry has no
/// `isCollapsed: true` is open (each of its members shows its children: "Expand all down one
/// level" writes the expanded levels without the flag and the last one with it); members in
/// `root.children` (nested `children` for deeper levels) with `isToggled` are the exceptions
/// (an expanded member of a collapsed level, "Expand > Selection").
#[derive(Debug, Default, PartialEq)]
pub(super) struct Expansion {
    /// Per level (field order): its members are expanded.
    pub open: Vec<bool>,
    /// Toggled member paths (top-level value first).
    pub toggled: Vec<Vec<Value>>,
}

/// The value of an identity literal (`'South'`, `5L`, `1.5D`, `true`,
/// `datetime'2025-01-01T00:00:00'`) as the data rows hold it.
pub(super) fn literal(x: &Value) -> Value {
    let Some(s) = x
        .get("Literal")
        .and_then(|l| l.get("Value"))
        .and_then(Value::as_str)
    else {
        return Value::Null;
    };
    if let Some(t) = s.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')) {
        return json!(t.replace("''", "'"));
    }
    if let Some(t) = s.strip_prefix("datetime'").and_then(|t| t.strip_suffix('\'')) {
        return json!(t);
    }
    match s {
        "null" => return Value::Null,
        "true" => return json!(true),
        "false" => return json!(false),
        _ => {}
    }
    let num = s
        .strip_suffix(['L', 'D', 'M'])
        .and_then(|t| t.parse::<f64>().ok())
        .and_then(serde_json::Number::from_f64);
    num.map_or_else(|| json!(s), Value::Number)
}

impl Expansion {
    /// The expansion of `role` (`n` levels); `None` without an entry for the role.
    pub(super) fn of(v: &Visual, role: &str, n: usize) -> Option<Expansion> {
        // Desktop 2.158 expands only the levels its projections mark `active` (it marks every
        // level when it expands); without them it shows the top level, whatever the states say
        let n = active_levels(v, role).min(n);
        if n < 2 {
            return None;
        }
        // active levels without a saved expansion: Desktop shows every one expanded
        let Some(e) = state_of(v, role) else {
            return Some(Expansion {
                open: (0..n).map(|i| i + 1 < n).collect(),
                toggled: Vec::new(),
            });
        };
        let levels = e.get("levels").and_then(Value::as_array);
        let open = (0..n)
            .map(|i| {
                i + 1 < n
                    && levels
                        .and_then(|l| l.get(i))
                        .is_some_and(|l| l.get("isCollapsed").and_then(Value::as_bool) != Some(true))
            })
            .collect();
        let mut toggled = Vec::new();
        // depth-first over the member tree, bounded in depth and count
        let mut stack: Vec<(Vec<Value>, &Value)> = e
            .get("root")
            .and_then(|r| r.get("children"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .rev()
            .map(|c| (Vec::new(), c))
            .collect();
        while let Some((parent, node)) = stack.pop() {
            if toggled.len() >= MAX_TOGGLED || parent.len() + 1 >= n.min(MAX_DEPTH) {
                continue;
            }
            let Some(id) = node
                .get("identityValues")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
            else {
                continue;
            };
            let mut path = parent;
            path.push(literal(id));
            if node.get("isToggled").and_then(Value::as_bool) == Some(true) {
                toggled.push(path.clone());
            }
            for c in node
                .get("children")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .rev()
            {
                stack.push((path.clone(), c));
            }
        }
        Some(Expansion { open, toggled })
    }

    /// How many levels can show: the top one, then each next level under an open level or a
    /// toggled member of a shown one (an upper bound: a toggled member of an open level is a
    /// collapsed exception).
    pub(super) fn reach(&self, n: usize) -> usize {
        let mut d = usize::from(n > 0);
        for i in 0..n.saturating_sub(1) {
            let any =
                self.open.get(i).copied().unwrap_or(false) || self.toggled.iter().any(|p| p.len() == i + 1);
            if i < d && any {
                d = i + 2;
            }
        }
        d.min(n)
    }

    pub(super) fn to_json(&self) -> Value {
        json!({"open": self.open, "toggled": self.toggled})
    }
}

/// Displayed levels of the `role` hierarchy (`n` levels) when it is expanded level by level:
/// the top one, plus each following level under an open one (`data.row_levels`).
pub(super) fn shown_levels(v: &Visual, role: &str, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    let open = Expansion::of(v, role, n).map_or(0, |e| e.open.iter().take_while(|o| **o).count());
    open.saturating_add(1).min(n)
}

/// Levels the engine evaluates for `role`: as deep as any member can be expanded.
fn reach(v: &Visual, role: &str, n: usize) -> usize {
    Expansion::of(v, role, n).map_or(usize::from(n > 0), |e| e.reach(n))
}

/// The per-level subtotal switches of `role` (`subTotals` entries with `perRowLevel` /
/// `perColumnLevel` on, and a `selector.metadata` naming a level's field): `[{show, label}]`
/// per level of the hierarchy (`n` levels), `None` when per-level settings are off. Desktop
/// 2.158 names a subtotal by the field it totals over: the top field's entry is the grand
/// total, the second field's the subtotals of the top members, and so on.
pub(super) fn level_totals(v: &Visual, role: &str, refs: &[String]) -> Option<Value> {
    let flag = if role.eq_ignore_ascii_case("Rows") {
        "perRowLevel"
    } else {
        "perColumnLevel"
    };
    let entries = v
        .visual_extra
        .get("objects")
        .and_then(|o| o.get("subTotals"))
        .and_then(Value::as_array)?;
    let lit = |e: &Value, p: &str| {
        e.get("properties")
            .and_then(|x| x.get(p))
            .and_then(|x| x.get("expr"))
            .and_then(|x| x.get("Literal"))
            .and_then(|x| x.get("Value"))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let on = entries
        .iter()
        .filter(|e| e.get("selector").is_none())
        .filter_map(|e| lit(e, flag))
        .next_back()
        .is_some_and(|s| s.eq_ignore_ascii_case("true"));
    if !on {
        return None;
    }
    let levels: Vec<Value> = refs
        .iter()
        .map(|r| {
            let mut o = serde_json::Map::new();
            for e in entries.iter().filter(|e| {
                e.get("selector")
                    .and_then(|s| s.get("metadata"))
                    .and_then(Value::as_str)
                    .is_some_and(|m| m == r)
            }) {
                if let Some(s) = lit(e, "levelSubtotalEnabled") {
                    o.insert("show".into(), json!(!s.eq_ignore_ascii_case("false")));
                }
                if let Some(s) = lit(e, "levelSubtotalLabel") {
                    let s = s
                        .strip_prefix('\'')
                        .and_then(|t| t.strip_suffix('\''))
                        .unwrap_or(&s);
                    o.insert("label".into(), json!(s.replace("''", "'")));
                }
            }
            Value::Object(o)
        })
        .collect();
    Some(Value::Array(levels))
}

/// The query references of the `role` grouping fields (the projections' `queryRef`, else
/// `Table.Column`), in field order.
pub(super) fn group_refs(v: &Visual, is_group: &[bool], role: &str) -> Vec<String> {
    let projections: Vec<&str> = v
        .visual_extra
        .get("query")
        .and_then(|q| q.get("queryState"))
        .and_then(Value::as_object)
        .and_then(|s| s.iter().find(|(k, _)| k.eq_ignore_ascii_case(role)))
        .and_then(|(_, r)| r.get("projections"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| p.get("queryRef").and_then(Value::as_str))
        .collect();
    let mut k = 0usize;
    v.fields
        .iter()
        .enumerate()
        .filter(|(_, b)| b.role.eq_ignore_ascii_case(role))
        .filter_map(|(i, b)| {
            let at = k;
            k = k.saturating_add(1);
            if !is_group.get(i).copied().unwrap_or(false) {
                return None;
            }
            Some(projections.get(at).map_or_else(
                || match &b.field {
                    bicraft_report::FieldRef::Column { table, column } => format!("{table}.{column}"),
                    _ => String::new(),
                },
                |s| (*s).to_string(),
            ))
        })
        .take(MAX_DEPTH)
        .collect()
}

/// The `data` keys describing the matrix's hierarchies: `row_levels` / `column_levels`
/// (uniform depth), `row_expansion` / `column_expansion` (open levels and toggled members),
/// `row_level_totals` / `column_level_totals` (per-level subtotal switches and labels),
/// `row_level_refs` / `column_level_refs` (each shown level's field as the per-level entries
/// name it).
pub(super) fn hierarchy_keys(v: &Visual, is_group: &[bool]) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for (prefix, role) in [("row", "Rows"), ("column", "Columns")] {
        let refs = group_refs(v, is_group, role);
        let n = refs.len();
        if n > 1 {
            out.push((format!("{prefix}_levels"), json!(shown_levels(v, role, n))));
            if let Some(e) = Expansion::of(v, role, n) {
                out.push((format!("{prefix}_expansion"), e.to_json()));
            }
        }
        if n > 0
            && let Some(t) = level_totals(v, role, &refs)
        {
            out.push((format!("{prefix}_level_totals"), t));
        }
        // the fields the per-level subtotal entries name (`selector.metadata`), for the Format
        // pane's Row subtotals and Column subtotals cards
        if n > 0 {
            out.push((format!("{prefix}_level_refs"), json!(refs)));
        }
    }
    out
}

/// `v` with only the fields where `keep` is true (its query projections pruned alike).
pub(super) fn with_fields(v: &Visual, keep: &[bool]) -> Visual {
    let mut d = v.clone();
    let mut k = 0usize;
    d.fields.retain(|_| {
        let kept = keep.get(k).copied().unwrap_or(true);
        k = k.saturating_add(1);
        kept
    });
    if let Some(state) = d
        .visual_extra
        .get_mut("query")
        .and_then(|q| q.get_mut("queryState"))
        .and_then(Value::as_object_mut)
    {
        let mut k = 0usize;
        for role in state.values_mut() {
            if let Some(ps) = role.get_mut("projections").and_then(Value::as_array_mut) {
                ps.retain(|_| {
                    let kept = keep.get(k).copied().unwrap_or(true);
                    k = k.saturating_add(1);
                    kept
                });
            }
        }
    }
    d
}

fn reason(f: Fail) -> String {
    match f {
        Fail::Unsupported(r) | Fail::Miss(Miss::Unavailable(r) | Miss::Error(r)) => r,
    }
}

/// The subtotal levels of matrix `v` (whose grouping fields are `is_group`), evaluated under
/// the visual's outer `filters`: `[{rows, columns, data}]` where `rows` / `columns` count the
/// leading row / column fields kept and `data` has one row per member combination, with a cell
/// per visual field (null under the dropped grouping fields). The leaf level (every field) is
/// the visual's own `data.rows` and is not repeated. `Err` carries why a level failed.
pub(super) fn levels(
    ctx: &mut Ctx<'_>,
    v: &Visual,
    is_group: &[bool],
    filters: &[Item],
) -> Result<Value, String> {
    let rows = group_fields(v, is_group, "Rows");
    let cols = group_fields(v, is_group, "Columns");
    let (dr, dc) = (reach(v, "Rows", rows.len()), reach(v, "Columns", cols.len()));
    let width = v.fields.len();
    let mut out = Vec::new();
    for r in 0..=dr {
        for c in 0..=dc {
            if r == rows.len() && c == cols.len() {
                continue;
            }
            let kept: Vec<usize> = rows.iter().take(r).chain(cols.iter().take(c)).copied().collect();
            let data: Vec<Value> = if kept.is_empty() {
                // the grand total: Desktop's total row context (as a table's)
                let t = general::totals(ctx, v, filters).map_err(reason)?;
                vec![Value::Array(t.iter().map(super::out_json).collect())]
            } else {
                let keep: Vec<bool> = (0..width)
                    .map(|i| !is_group.get(i).copied().unwrap_or(false) || kept.contains(&i))
                    .collect();
                let pv = with_fields(v, &keep);
                let q = Query { filters, extra: &[] };
                let mut grid = grid(ctx, &pv, &q).map_err(reason)?;
                // "Show items with no data": a level lists its members without values too (why
                // they could not be listed is the visual's own note, from its leaf rows)
                let _ = super::show::add_items_with_no_data(ctx, &pv, &mut grid, filters);
                let plan = Plan {
                    items: Vec::new(),
                    values: Vec::new(),
                    ignored: Vec::new(),
                };
                let rules = format::Rules::of(&pv, None);
                let d = finish_data(ctx, &pv, grid, &[], &plan, &rules, Vec::new(), &[]);
                // back to the visual's columns
                let at: Vec<usize> = (0..width)
                    .filter(|i| keep.get(*i).copied().unwrap_or(false))
                    .collect();
                d.get("rows")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|row| {
                        let mut full = vec![Value::Null; width];
                        for (j, x) in row.as_array().into_iter().flatten().enumerate() {
                            if let Some(slot) = at.get(j).and_then(|i| full.get_mut(*i)) {
                                *slot = x.clone();
                            }
                        }
                        Value::Array(full)
                    })
                    .collect()
            };
            out.push(json!({"rows": r, "columns": c, "data": data}));
        }
    }
    Ok(Value::Array(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_states(states: Value) -> Visual {
        let mut v: Visual = serde_json::from_value(json!({"name": "m", "visual_type": "pivotTable"}))
            .unwrap_or_else(|e| panic!("{e}"));
        v.visual_extra.insert("expansionStates".into(), states);
        // four levels per hierarchy, all marked active (as Desktop marks an expanded matrix)
        let ps: Vec<Value> = (0..4)
            .map(|i| json!({"queryRef": format!("T.c{i}"), "active": true}))
            .collect();
        v.visual_extra.insert(
            "query".into(),
            json!({"queryState": {"Rows": {"projections": ps}, "Columns": {"projections": ps}}}),
        );
        v
    }

    #[test]
    fn expansion_needs_active_levels() {
        // Desktop 2.158 shows the top level when the projections carry no `active` flags,
        // and expands only within the active ones
        let open = json!([{"roles": ["Rows"], "levels": [{"queryRefs": ["T.a"]}, {"queryRefs": ["T.b"]}]}]);
        let mut v = with_states(open);
        v.visual_extra.remove("query");
        assert_eq!(Expansion::of(&v, "Rows", 3), None);
        assert_eq!(shown_levels(&v, "Rows", 3), 1);
        v.visual_extra.insert(
            "query".into(),
            json!({"queryState": {"Rows": {"projections": [{"active": true}, {"active": true}, {"active": false}]}}}),
        );
        assert_eq!(shown_levels(&v, "Rows", 3), 2);
        assert_eq!(
            Expansion::of(&v, "Rows", 3).map(|e| e.open),
            Some(vec![true, false])
        );
    }

    #[test]
    fn expansion_states_set_the_displayed_levels() {
        // active levels: no state shows them all; levels without isCollapsed are expanded; a
        // collapsed level stops the uniform depth
        assert_eq!(shown_levels(&with_states(Value::Null), "Rows", 3), 3);
        let open = json!([{"roles": ["Rows"], "levels": [{"queryRefs": ["T.a"]}, {"queryRefs": ["T.b"]}]}]);
        assert_eq!(shown_levels(&with_states(open), "Rows", 3), 3);
        let shut = json!([{"roles": ["Rows"], "levels": [{"queryRefs": ["T.a"], "isCollapsed": true}]}]);
        assert_eq!(shown_levels(&with_states(shut.clone()), "Rows", 3), 1);
        // the Columns hierarchy has no state: all of it expanded
        assert_eq!(shown_levels(&with_states(shut), "Columns", 2), 2);
        assert_eq!(shown_levels(&with_states(Value::Null), "Rows", 0), 0);
    }

    #[test]
    fn desktop_expansion_shapes_are_read() {
        // "Expand all down one level" (Desktop 2.158): Region open, Category collapsed
        let all = json!([{"roles": ["Rows"], "levels": [
            {"queryRefs": ["Sales.Region"], "isPinned": true},
            {"queryRefs": ["Sales.Category"], "isCollapsed": true, "isPinned": true}], "root": {}}]);
        let e = Expansion::of(&with_states(all), "Rows", 2).unwrap_or_default();
        assert_eq!(e.open, vec![true, false]);
        assert!(e.toggled.is_empty());
        assert_eq!(e.reach(2), 2);
        // "Expand > Selection" on South: the level collapsed, the member toggled
        let one = json!([{"roles": ["Rows"], "levels": [
            {"queryRefs": ["Sales.Region"], "isCollapsed": true, "identityKeys": [{}], "isPinned": true},
            {"queryRefs": ["Sales.Category"], "isCollapsed": true}],
            "root": {"children": [{"identityValues": [{"Literal": {"Value": "'South'"}}], "isToggled": true,
                "children": [{"identityValues": [{"Literal": {"Value": "12L"}}], "isToggled": true}]}]}}]);
        let e = Expansion::of(&with_states(one), "Rows", 3).unwrap_or_default();
        assert_eq!(e.open, vec![false, false, false]);
        assert_eq!(
            e.toggled,
            vec![vec![json!("South")], vec![json!("South"), json!(12.0)]]
        );
        assert_eq!(e.reach(3), 3);
        assert_eq!(e.reach(2), 2);
        // every level active, no saved state: all expanded (Desktop 2.158)
        let all_open = Expansion::of(&with_states(Value::Null), "Rows", 3).map(|e| e.open);
        assert_eq!(all_open, Some(vec![true, true, false]));
        // hostile: deep nesting is bounded
        let mut node = json!({"identityValues": [{"Literal": {"Value": "'x'"}}], "isToggled": true});
        for _ in 0..200 {
            node = json!({"identityValues": [{"Literal": {"Value": "'x'"}}], "isToggled": true, "children": [node]});
        }
        let deep = json!([{"roles": ["Rows"], "root": {"children": [node]}}]);
        let e = Expansion::of(&with_states(deep), "Rows", 3).unwrap_or_default();
        assert_eq!(e.toggled.len(), 2);
    }

    #[test]
    fn identity_literals_become_data_values() {
        let l = |s: &str| literal(&json!({"Literal": {"Value": s}}));
        assert_eq!(l("'O''Neil'"), json!("O'Neil"));
        assert_eq!(l("1.5D"), json!(1.5));
        assert_eq!(l("true"), json!(true));
        assert_eq!(l("null"), Value::Null);
        assert_eq!(l("datetime'2025-01-01T00:00:00'"), json!("2025-01-01T00:00:00"));
        assert_eq!(literal(&json!(3)), Value::Null);
    }

    #[test]
    fn per_level_subtotals_follow_the_selectors() {
        let mut v = with_states(Value::Null);
        let lit = |s: &str| json!({"expr": {"Literal": {"Value": s}}});
        v.visual_extra.insert(
            "objects".into(),
            json!({"subTotals": [
                {"properties": {"perRowLevel": lit("true")}},
                {"properties": {"levelSubtotalEnabled": lit("false")}, "selector": {"metadata": "Sales.Region"}},
                {"properties": {"levelSubtotalLabel": lit("'Sous-cat'")}, "selector": {"metadata": "Sales.Category"}}]}),
        );
        let refs = vec!["Sales.Region".to_string(), "Sales.Category".to_string()];
        assert_eq!(
            level_totals(&v, "Rows", &refs),
            Some(json!([{"show": false}, {"label": "Sous-cat"}]))
        );
        // per-level settings off: ignored
        assert_eq!(level_totals(&v, "Columns", &refs), None);
    }

    #[test]
    fn with_fields_prunes_fields_and_projections() {
        let mut v = with_states(Value::Null);
        v.fields = serde_json::from_value(json!([
            {"role": "Rows", "field": {"kind": "column", "table": "T", "column": "a"}},
            {"role": "Rows", "field": {"kind": "column", "table": "T", "column": "b"}}]))
        .unwrap_or_else(|e| panic!("{e}"));
        v.visual_extra.insert(
            "query".into(),
            json!({"queryState": {"Rows": {"projections": [{"queryRef": "T.a"}, {"queryRef": "T.b"}]}}}),
        );
        let p = with_fields(&v, &[true, false]);
        assert_eq!(p.fields.len(), 1);
        assert_eq!(
            p.visual_extra["query"]["queryState"]["Rows"]["projections"],
            json!([{"queryRef": "T.a"}])
        );
    }
}
