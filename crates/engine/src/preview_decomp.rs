//! Decomposition trees (`decompositionTreeVisual`, `data.decomp`): the analysed measure (role
//! `Analyze`) at the root, then each level the visual's expansion state shows, evaluated by its
//! Explain-by field under the selected path of the levels above it.
//!
//! The state is the PBIR `expansionStates` entry of role `ExplainBy` (public visualConfiguration
//! schema: `levels[].queryRefs / isPinned / AIInformation.method`, `root.children[].
//! identityValues / isToggled`), with the shapes Desktop 2.159 saved after the same actions in
//! its UI (docs/render.md, "Decomposition tree"):
//! - the pinned levels are shown, in order;
//! - the selected path is the chain of toggled members under `root`, one per level;
//! - an AI level (`MaxSplit`: "High value", `MinSplit`: "Low value") is re-chosen whenever it is
//!   shown: among the Explain-by fields no other level uses, the one whose highest (lowest)
//!   category value under the parent path is the highest (lowest) of all; the saved field is
//!   ignored. Its highest (lowest) member is selected when the path does not name one;
//! - a level whose parent has no selected member is not shown.
//!
//! Each level is a grouped query of its field and the measure, under the visual's filters and
//! the path (`field = value` of each level above). Members sort like the visual's other queries
//! (`sortDefinition`, else by category, A to Z) and are capped at [`MAX_NODES`].

use bicraft_report::{FieldBinding, FieldRef, Visual};
use serde_json::{Map, Value, json};

use super::filters::{Cond, Item, Operand, Val};
use super::{Ctx, Fail, Miss, Plan, Query, data_type_name, finish_data, format, grid};

/// Most levels shown (Desktop takes one per Explain-by field).
const MAX_LEVELS: usize = 16;
/// Most members kept per level.
pub(super) const MAX_NODES: usize = 1000;
/// Most fields an AI level compares.
const MAX_CANDIDATES: usize = 32;

/// How an AI level picks its field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Ai {
    /// `MaxSplit` ("High value").
    High,
    /// `MinSplit` ("Low value").
    Low,
}

impl Ai {
    fn name(self) -> &'static str {
        match self {
            Ai::High => "high",
            Ai::Low => "low",
        }
    }
}

/// One shown level of the saved state.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct LevelSpec {
    /// Index in `Visual::fields` of its Explain-by field (an AI level: the saved one, if any).
    pub field: Option<usize>,
    pub ai: Option<Ai>,
}

/// The saved state of a tree.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Spec {
    /// Index in `Visual::fields` of the analysed measure.
    pub analyze: Option<usize>,
    /// Indices of the Explain-by columns, in field order.
    pub explain: Vec<usize>,
    pub levels: Vec<LevelSpec>,
    /// The selected member of each level from the first, as the data rows hold it.
    pub path: Vec<Value>,
    /// The reader's scroll position of each level from the first: the first member shown
    /// (`None`: the window Desktop opens with). Reader state only, never saved.
    pub scroll: Vec<Option<usize>>,
    /// Levels scrolled out on the left of a tree wider than the visual (reader state only).
    pub shift: usize,
}

/// The `queryRef` of each field of `role` (`Table.Column` when the projection has none), with
/// the field's index.
fn refs_of(v: &Visual, role: &str) -> Vec<(usize, String)> {
    let projections: Vec<String> = v
        .visual_extra
        .get("query")
        .and_then(|q| q.get("queryState"))
        .and_then(Value::as_object)
        .and_then(|s| s.iter().find(|(k, _)| k.eq_ignore_ascii_case(role)))
        .and_then(|(_, r)| r.get("projections"))
        .and_then(Value::as_array)
        .map(|ps| {
            ps.iter()
                .map(|p| {
                    p.get("queryRef")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                })
                .collect()
        })
        .unwrap_or_default();
    v.fields
        .iter()
        .enumerate()
        .filter(|(_, b)| b.role.eq_ignore_ascii_case(role))
        .enumerate()
        .map(|(k, (i, b))| {
            let fallback = match &b.field {
                FieldRef::Column { table, column } => format!("{table}.{column}"),
                FieldRef::Measure { table, measure } => format!("{table}.{measure}"),
                _ => String::new(),
            };
            let r = projections
                .get(k)
                .filter(|r| !r.is_empty())
                .cloned()
                .unwrap_or(fallback);
            (i, r)
        })
        .collect()
}

impl Spec {
    /// The JSON of [`DECOMP_STATE`].
    fn to_state(&self) -> Value {
        let levels: Vec<Value> = self
            .levels
            .iter()
            .map(|l| json!({"column": l.field, "ai": l.ai.map(Ai::name)}))
            .collect();
        json!({"levels": levels, "path": self.path, "scroll": self.scroll, "shift": self.shift})
    }

    /// Levels and path from a reader state (`None`: not one).
    fn read_state(&mut self, t: &Value) -> Option<()> {
        let levels = t.get("levels")?.as_array()?;
        let path = t.get("path")?.as_array()?;
        self.levels = levels
            .iter()
            .take(MAX_LEVELS)
            .map(|l| LevelSpec {
                field: l
                    .get("column")
                    .and_then(Value::as_u64)
                    .and_then(|c| usize::try_from(c).ok())
                    .filter(|c| self.explain.contains(c)),
                ai: match l.get("ai").and_then(Value::as_str) {
                    Some("high") => Some(Ai::High),
                    Some("low") => Some(Ai::Low),
                    _ => None,
                },
            })
            .take_while(|l| l.field.is_some() || l.ai.is_some())
            .collect();
        self.path = path.iter().take(MAX_LEVELS).cloned().collect();
        self.scroll = t
            .get("scroll")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(MAX_LEVELS)
            .map(|x| {
                x.as_u64()
                    .and_then(|s| usize::try_from(s).ok())
                    .map(|s| s.min(MAX_NODES))
            })
            .collect();
        self.shift = t
            .get("shift")
            .and_then(Value::as_u64)
            .and_then(|s| usize::try_from(s).ok())
            .map_or(0, |s| s.min(MAX_LEVELS));
        Some(())
    }

    pub(super) fn of(v: &Visual) -> Spec {
        let analyze = v.fields.iter().position(|b| {
            b.role.eq_ignore_ascii_case("Analyze") && !matches!(b.field, FieldRef::Column { .. })
        });
        let explain_refs: Vec<(usize, String)> = refs_of(v, "ExplainBy")
            .into_iter()
            .filter(|(i, _)| {
                v.fields
                    .get(*i)
                    .is_some_and(|b| matches!(b.field, FieldRef::Column { .. }))
            })
            .collect();
        let explain: Vec<usize> = explain_refs.iter().map(|(i, _)| *i).collect();
        let state = v
            .visual_extra
            .get("expansionStates")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|e| {
                e.get("roles").and_then(Value::as_array).is_some_and(|r| {
                    r.iter()
                        .any(|x| x.as_str().is_some_and(|s| s.eq_ignore_ascii_case("ExplainBy")))
                })
            });
        let mut levels = Vec::new();
        let mut path = Vec::new();
        if let Some(e) = state {
            for l in e.get("levels").and_then(Value::as_array).into_iter().flatten() {
                if levels.len() >= MAX_LEVELS {
                    break;
                }
                if l.get("isPinned").and_then(Value::as_bool) != Some(true) {
                    continue;
                }
                let ai = match l
                    .get("AIInformation")
                    .filter(|a| a.get("disabled").and_then(Value::as_bool) != Some(true))
                    .and_then(|a| a.get("method"))
                    .and_then(Value::as_str)
                {
                    Some("MaxSplit") => Some(Ai::High),
                    Some("MinSplit") => Some(Ai::Low),
                    _ => None,
                };
                let r = l
                    .get("queryRefs")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let field = explain_refs.iter().find(|(_, q)| q == r).map(|(i, _)| *i);
                if field.is_none() && ai.is_none() {
                    // a level naming no field of the visual: Desktop cannot show it nor what follows
                    break;
                }
                levels.push(LevelSpec { field, ai });
            }
            // the selected path: the first toggled member at each depth
            let mut node = e.get("root");
            while path.len() < MAX_LEVELS {
                let Some(next) = node
                    .and_then(|n| n.get("children"))
                    .and_then(Value::as_array)
                    .and_then(|c| {
                        c.iter()
                            .find(|x| x.get("isToggled").and_then(Value::as_bool) == Some(true))
                    })
                else {
                    break;
                };
                let id = next
                    .get("identityValues")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .map(super::matrix::literal)
                    .unwrap_or(Value::Null);
                path.push(id);
                node = Some(next);
            }
        }
        let mut spec = Spec {
            analyze,
            explain,
            levels,
            path,
            scroll: Vec::new(),
            shift: 0,
        };
        // the reader changed the tree (`view.interact`, never saved)
        if let Some(t) = v.visual_extra.get(DECOMP_STATE) {
            let _ = spec.read_state(t);
        }
        spec
    }
}

/// One evaluated level.
#[derive(Debug, Clone, PartialEq)]
struct Level {
    field: usize,
    ai: Option<Ai>,
    /// `(member, measure)`, sorted, capped.
    nodes: Vec<(Value, Value)>,
    total: usize,
    selected: Option<usize>,
    /// The selection was made by the AI split (not saved in the path).
    auto: bool,
    /// The reader's scroll position (the first member shown).
    start: Option<usize>,
}

fn reason(f: Fail) -> String {
    match f {
        Fail::Unsupported(r) | Fail::Miss(Miss::Unavailable(r) | Miss::Error(r)) => r,
    }
}

/// `v` reduced to `fields` (no query state, no expansion): its sort definition is kept.
fn sub_visual(v: &Visual, fields: Vec<FieldBinding>) -> Visual {
    let mut d = v.clone();
    d.fields = fields;
    d.visual_extra.remove("expansionStates");
    let sort = d
        .visual_extra
        .get("query")
        .and_then(|q| q.get("sortDefinition"))
        .cloned();
    match sort {
        Some(s) => {
            d.visual_extra
                .insert("query".into(), json!({"sortDefinition": s}));
        }
        None => {
            d.visual_extra.remove("query");
        }
    }
    d
}

/// The rows (`data.rows` JSON) of `v` under `filters`.
fn run(
    ctx: &mut Ctx<'_>,
    v: &Visual,
    filters: &[Item],
    plan: &Plan,
) -> Result<(Vec<Value>, usize, Vec<String>), String> {
    let q = Query { filters, extra: &[] };
    let grid = grid(ctx, v, &q).map_err(reason)?;
    let level_plan = Plan {
        items: filters.to_vec(),
        values: plan.values.clone(),
        ignored: Vec::new(),
    };
    let rules = format::Rules::of(v, None);
    let mut notes = Vec::new();
    let d = finish_data(ctx, v, grid, &[], &level_plan, &rules, Vec::new(), &[]);
    if let Some(r) = d.get("filters_ignored_reason").and_then(Value::as_str) {
        notes.push(r.to_string());
    }
    let total = d
        .get("total_rows")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0);
    let rows = d
        .get("rows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok((rows, total, notes))
}

/// A member value as a filter literal (dates are ISO text in the rows).
fn val_of(x: &Value, data_type: &str) -> Val {
    match x {
        Value::Null => Val::Null,
        Value::Bool(b) => Val::Bool(*b),
        Value::Number(n) => Val::Num(n.as_f64().unwrap_or(0.0)),
        Value::String(s) if data_type.eq_ignore_ascii_case("dateTime") => {
            crate::interact::iso_serial(s).map_or_else(|| Val::Text(s.clone()), Val::Date)
        }
        Value::String(s) => Val::Text(s.clone()),
        _ => Val::Null,
    }
}

/// Whether two member values are the same member (text without case, as the engine groups).
pub(super) fn same_member(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(x), Value::String(y)) => x.to_lowercase() == y.to_lowercase(),
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        // a blank member: null in the rows, `null` or an empty text in the files
        (Value::Null, Value::String(s)) | (Value::String(s), Value::Null) => s.is_empty(),
        _ => a == b,
    }
}

/// A level's `(member, measure)` pairs.
type Members = Vec<(Value, Value)>;
/// A level's members, its member count before the cap, and the filters it could not apply.
type LevelRows = (Members, usize, Vec<String>);

/// The members of `field` and the measure under `filters`.
fn level_nodes(
    ctx: &mut Ctx<'_>,
    v: &Visual,
    field: usize,
    analyze: usize,
    filters: &[Item],
    plan: &Plan,
) -> Result<LevelRows, String> {
    let (Some(f), Some(m)) = (v.fields.get(field), v.fields.get(analyze)) else {
        return Err("a level names no field of the visual".into());
    };
    let pv = sub_visual(v, vec![f.clone(), m.clone()]);
    let (rows, total, notes) = run(ctx, &pv, filters, plan)?;
    let nodes = rows
        .into_iter()
        .map(|r| {
            let a = r.as_array();
            let cell = |i: usize| a.and_then(|a| a.get(i)).cloned().unwrap_or(Value::Null);
            (cell(0), cell(1))
        })
        .collect();
    Ok((nodes, total, notes))
}

/// An AI split's score of `nodes`: the extreme measure (absolute mode) or the extreme ratio of a
/// member's measure to the level's mean (relative mode), with the member holding it.
fn score(nodes: &[(Value, Value)], ai: Ai, relative: bool) -> Option<(usize, f64)> {
    if !relative {
        return extreme(nodes, ai);
    }
    let xs: Vec<f64> = nodes
        .iter()
        .filter_map(|(_, m)| m.as_f64().filter(|x| x.is_finite()))
        .collect();
    let mean = xs.iter().sum::<f64>() / xs.len().max(1) as f64;
    if xs.is_empty() || mean == 0.0 {
        return extreme(nodes, ai);
    }
    let ratios: Vec<(Value, Value)> = nodes
        .iter()
        .map(|(k, m)| (k.clone(), m.as_f64().map_or(Value::Null, |x| json!(x / mean))))
        .collect();
    extreme(&ratios, ai)
}

/// The highest (High) or lowest (Low) measure among `nodes`, and the first member holding it.
fn extreme(nodes: &[(Value, Value)], ai: Ai) -> Option<(usize, f64)> {
    let mut best: Option<(usize, f64)> = None;
    for (i, (_, m)) in nodes.iter().enumerate() {
        let Some(x) = m.as_f64().filter(|x| x.is_finite()) else {
            continue;
        };
        let better = match (best, ai) {
            (None, _) => true,
            (Some((_, b)), Ai::High) => x > b,
            (Some((_, b)), Ai::Low) => x < b,
        };
        if better {
            best = Some((i, x));
        }
    }
    best
}

fn path_item(v: &Visual, field: usize, value: &Value, model: &bicraft_model::SemanticModel) -> Option<Item> {
    let FieldRef::Column { table, column } = &v.fields.get(field)?.field else {
        return None;
    };
    let dt = data_type_name(model, table, column);
    Some(Item::Cond(Cond::In {
        exprs: vec![Operand::Col {
            table: table.clone(),
            column: column.clone(),
        }],
        rows: vec![vec![val_of(value, &dt)]],
    }))
}

/// The tree's `data` (`status`, `columns`, `rows`, `decomp`), or why it cannot be evaluated.
pub(super) fn compute(ctx: &mut Ctx<'_>, v: &Visual, plan: &Plan, notes: Vec<String>) -> Value {
    let spec = Spec::of(v);
    let Some(analyze) = spec.analyze else {
        // Desktop shows its "no field to analyze" error (docs/formats/visual-roles-bb.md)
        return super::status("unsupported", "the decomposition tree has no field to analyze");
    };
    let model = ctx.model;
    let mut notes = notes;
    // the root: the measure alone
    let root = match v.fields.get(analyze) {
        Some(m) => {
            let pv = sub_visual(v, vec![m.clone()]);
            match run(ctx, &pv, &plan.items, plan) {
                Ok((rows, _, n)) => {
                    notes.extend(n);
                    rows.first()
                        .and_then(Value::as_array)
                        .and_then(|r| r.first())
                        .cloned()
                        .unwrap_or(Value::Null)
                }
                Err(r) => return super::status("unsupported", r),
            }
        }
        None => Value::Null,
    };
    let relative = v
        .visual_extra
        .get("objects")
        .and_then(|o| o.get("analysis"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| {
            e.pointer("/properties/aiMode/expr/Literal/Value")
                .and_then(Value::as_str)
        })
        .next_back()
        .is_some_and(|m| m == "'relative'");
    let mut filters: Vec<Item> = plan.items.clone();
    let mut levels: Vec<Level> = Vec::new();
    let mut used: Vec<usize> = Vec::new();
    for (i, ls) in spec.levels.iter().enumerate() {
        // the fields of the fixed levels below are not candidates of an AI split
        let later: Vec<usize> = spec
            .levels
            .iter()
            .skip(i.saturating_add(1))
            .filter(|l| l.ai.is_none())
            .filter_map(|l| l.field)
            .collect();
        let chosen = match ls.ai {
            None => {
                let Some(f) = ls.field.filter(|f| !used.contains(f)) else {
                    break;
                };
                match level_nodes(ctx, v, f, analyze, &filters, plan) {
                    Ok((nodes, total, n)) => {
                        notes.extend(n);
                        Some((f, nodes, total))
                    }
                    Err(r) => return super::status("unsupported", r),
                }
            }
            Some(ai) => {
                let mut best: Option<(f64, usize, Members, usize)> = None;
                for f in spec
                    .explain
                    .iter()
                    .copied()
                    .filter(|f| !used.contains(f) && !later.contains(f))
                    .take(MAX_CANDIDATES)
                {
                    let (nodes, total, n) = match level_nodes(ctx, v, f, analyze, &filters, plan) {
                        Ok(x) => x,
                        Err(r) => return super::status("unsupported", r),
                    };
                    notes.extend(n);
                    let Some((_, x)) = score(&nodes, ai, relative) else {
                        continue;
                    };
                    let better = match &best {
                        None => true,
                        Some((b, ..)) => match ai {
                            Ai::High => x > *b,
                            Ai::Low => x < *b,
                        },
                    };
                    if better {
                        best = Some((x, f, nodes, total));
                    }
                }
                best.map(|(_, f, nodes, total)| (f, nodes, total))
            }
        };
        let Some((field, mut nodes, total)) = chosen else {
            break;
        };
        nodes.truncate(MAX_NODES);
        let saved = spec.path.get(i);
        let (selected, auto) = match saved {
            Some(p) => (nodes.iter().position(|(m, _)| same_member(m, p)), false),
            None => match ls.ai {
                Some(ai) => (extreme(&nodes, ai).map(|(k, _)| k), true),
                None => (None, false),
            },
        };
        used.push(field);
        let next = selected.and_then(|k| nodes.get(k)).map(|(m, _)| m.clone());
        levels.push(Level {
            field,
            ai: ls.ai,
            nodes,
            total,
            selected,
            auto,
            start: spec.scroll.get(i).copied().flatten(),
        });
        let Some(member) = next else {
            break;
        };
        match path_item(v, field, &member, model) {
            Some(item) => filters.push(item),
            None => break,
        }
    }
    to_json(v, model, analyze, &root, &levels, spec.shift, notes)
}

fn to_json(
    v: &Visual,
    model: &bicraft_model::SemanticModel,
    analyze: usize,
    root: &Value,
    levels: &[Level],
    shift: usize,
    mut notes: Vec<String>,
) -> Value {
    let width = v.fields.len();
    let columns: Vec<Value> = v
        .fields
        .iter()
        .map(|b| match &b.field {
            FieldRef::Column { table, column } => json!({"name": column, "role": b.role, "kind": "column",
                "data_type": data_type_name(model, table, column)}),
            FieldRef::Measure { measure, .. } => {
                json!({"name": measure, "role": b.role, "kind": "measure", "data_type": "double"})
            }
            FieldRef::Aggregation { column, function, .. } => {
                json!({"name": format!("{function} of {column}"),
                "role": b.role, "kind": "aggregation", "data_type": "double"})
            }
            FieldRef::Other { .. } => json!({"name": "", "role": b.role, "kind": "other"}),
        })
        .collect();
    // the rows: the root, then each level's members under the path (one cell per field)
    let mut rows = Vec::new();
    let mut base = vec![Value::Null; width];
    let mut root_row = base.clone();
    if let Some(slot) = root_row.get_mut(analyze) {
        *slot = root.clone();
    }
    rows.push(Value::Array(root_row));
    for l in levels {
        for (m, x) in &l.nodes {
            let mut r = base.clone();
            if let Some(slot) = r.get_mut(l.field) {
                *slot = m.clone();
            }
            if let Some(slot) = r.get_mut(analyze) {
                *slot = x.clone();
            }
            rows.push(Value::Array(r));
        }
        if let Some((m, _)) = l.selected.and_then(|k| l.nodes.get(k))
            && let Some(slot) = base.get_mut(l.field)
        {
            *slot = m.clone();
        }
    }
    let levels_json: Vec<Value> = levels
        .iter()
        .map(|l| {
            json!({
                "column": l.field,
                "ai": l.ai.map(Ai::name),
                "nodes": l.nodes.iter().map(|(m, x)| json!([m, x])).collect::<Vec<_>>(),
                "total": l.total,
                "selected": l.selected,
                "auto": l.auto,
                "start": l.start,
            })
        })
        .collect();
    let mut o = Map::new();
    o.insert("status".into(), json!("ok"));
    o.insert("columns".into(), Value::Array(columns));
    let n = rows.len();
    o.insert("rows".into(), Value::Array(rows));
    o.insert("truncated".into(), json!(false));
    o.insert("total_rows".into(), json!(n));
    o.insert(
        "decomp".into(),
        json!({"analyze": analyze, "root": root, "levels": levels_json, "shift": shift, "options": options(v)}),
    );
    notes.dedup();
    if !notes.is_empty() {
        o.insert("filters_ignored".into(), Value::Bool(true));
        let mut text = notes.join("; ");
        if text.len() > 1000 {
            let cut = text.char_indices().nth(1000).map_or(text.len(), |(i, _)| i);
            text.truncate(cut);
        }
        o.insert("filters_ignored_reason".into(), json!(text));
    }
    Value::Object(o)
}

/// The formatting options the renderer reads (visual-options.md names), as plain JSON:
/// `{object: {property: value}}` with literals decoded (`'dense'` → "dense", `6L` → 6,
/// colours → "#rrggbb").
pub(super) fn options(v: &Visual) -> Value {
    const OBJECTS: [&str; 7] = [
        "tree",
        "dataBars",
        "dataLabels",
        "categoryLabels",
        "levelHeader",
        "analysis",
        "insights",
    ];
    let mut out = Map::new();
    let Some(objects) = v.visual_extra.get("objects").and_then(Value::as_object) else {
        return Value::Object(out);
    };
    for name in OBJECTS {
        let Some(entries) = objects.get(name).and_then(Value::as_array) else {
            continue;
        };
        let mut props = Map::new();
        // the entry without a selector applies to the whole visual
        for e in entries.iter().filter(|e| e.get("selector").is_none()).take(16) {
            for (k, x) in e
                .get("properties")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .take(64)
            {
                if let Some(val) = decode(x) {
                    props.insert(k.clone(), val);
                }
            }
        }
        if !props.is_empty() {
            out.insert(name.to_string(), Value::Object(props));
        }
    }
    Value::Object(out)
}

/// A property value: a literal, or a solid colour.
fn decode(x: &Value) -> Option<Value> {
    let expr = x.get("expr");
    if let Some(s) = expr
        .and_then(|e| e.get("Literal"))
        .and_then(|l| l.get("Value"))
        .and_then(Value::as_str)
    {
        return Some(match crate::visual_props::parse_lit(s) {
            crate::visual_props::Lit::Null => Value::Null,
            crate::visual_props::Lit::Bool(b) => json!(b),
            crate::visual_props::Lit::Num(n, _) => json!(n),
            crate::visual_props::Lit::Text(t) | crate::visual_props::Lit::Date(t) => json!(t),
            crate::visual_props::Lit::Other(t) => json!(t),
        });
    }
    let color = x
        .get("solid")
        .and_then(|s| s.get("color"))
        .and_then(|c| c.get("expr"))
        .and_then(|e| e.get("Literal"))
        .and_then(|l| l.get("Value"))
        .and_then(Value::as_str)?;
    Some(json!(color.trim_matches('\'')))
}

/// The `visual_extra` key under which the viewer's reader state replaces a tree's stored
/// expansion (`{levels: [{column, ai}], path: [member]}`); only in the reports derived for a
/// snapshot, never saved.
pub(crate) const DECOMP_STATE: &str = "bicraft.decomp";

/// The tree's state as stored (its expansion state), as a reader state.
pub(crate) fn decomp_state(v: &Visual) -> Value {
    Spec::of(v).to_state()
}

/// A reader action on a tree (`view.interact`): `decomp_path {level, value}` selects member
/// `value` of level `level` (1 = the first under the root; the levels below stay, AI levels are
/// chosen again), `decomp_expand {level, value, column | ai, columns?}` opens a level under
/// member `value` of level `level` (0: under the root) by a field (`column`, a data column of the
/// visual) or an AI split (`ai`: `high` | `low`); `columns` names the fields the levels above
/// show, which fixes AI levels above as Desktop does, and `path` (both actions) their selected
/// members as shown; `decomp_collapse {level}` removes that
/// level and the ones below (the header's cross); `decomp_scroll {level, start}` shows level
/// `level`'s members from `start` on (its scroll arrows), `decomp_scroll {shift}` scrolls a tree
/// wider than the visual by `shift` levels to the left. A new path or level drops the scroll
/// positions of the levels below it.
pub(crate) fn decomp_action(v: &Visual, state: Value, action: &str, p: &Value) -> Result<Value, String> {
    let mut spec = Spec::of(v);
    let _ = spec.read_state(&state);
    let number = |k: &str| -> Result<Option<usize>, String> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(x) => x
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .map(Some)
                .ok_or_else(|| format!("`{k}` must be a whole number")),
        }
    };
    if action == "decomp_scroll" {
        match (number("level")?, number("start")?, number("shift")?) {
            (Some(level), Some(start), None) => {
                if level == 0 || level > spec.levels.len() {
                    return Err(format!("the tree shows no level {level}"));
                }
                if spec.scroll.len() < level {
                    spec.scroll.resize(level, None);
                }
                if let Some(slot) = spec.scroll.get_mut(level - 1) {
                    *slot = Some(start.min(MAX_NODES));
                }
            }
            (None, None, Some(shift)) => {
                if shift > spec.levels.len() {
                    return Err(format!(
                        "the tree has {} levels to scroll past",
                        spec.levels.len()
                    ));
                }
                spec.shift = shift;
            }
            _ => {
                return Err(
                    "`level` and `start` (a level's members), or `shift` (the levels), is required".into(),
                );
            }
        }
        return Ok(spec.to_state());
    }
    let level = p
        .get("level")
        .and_then(Value::as_u64)
        .and_then(|l| usize::try_from(l).ok())
        .ok_or("`level` (a number) is required")?;
    let value = p.get("value").cloned().unwrap_or(Value::Null);
    if matches!(value, Value::Array(_) | Value::Object(_)) {
        return Err("`value` must be a number, text, boolean or null".into());
    }
    // the members above as the viewer showed them (AI levels select theirs without a saved path)
    if let Some(above) = p.get("path").and_then(Value::as_array) {
        spec.path = above
            .iter()
            .take(level.saturating_sub(1).min(MAX_LEVELS))
            .filter(|x| !matches!(x, Value::Array(_) | Value::Object(_)))
            .cloned()
            .collect();
    }
    match action {
        "decomp_path" => {
            if level == 0 || level > spec.levels.len() {
                return Err(format!("the tree shows no level {level}"));
            }
            spec.path.truncate(level - 1);
            if spec.path.len() + 1 < level {
                return Err("the levels above have no selected member".into());
            }
            spec.path.push(value);
            spec.scroll.truncate(level);
        }
        "decomp_expand" => {
            if level > spec.levels.len() || level >= MAX_LEVELS {
                return Err(format!("the tree shows no level {level}"));
            }
            let ai = match p.get("ai").and_then(Value::as_str) {
                None => None,
                Some("high") => Some(Ai::High),
                Some("low") => Some(Ai::Low),
                Some(o) => return Err(format!("`ai` is high or low, not `{o}`")),
            };
            let column = p
                .get("column")
                .and_then(Value::as_u64)
                .and_then(|c| usize::try_from(c).ok());
            if ai.is_none() && column.is_none_or(|c| !spec.explain.contains(&c)) {
                return Err("`column` (an Explain by field of the visual) or `ai` is required".into());
            }
            // the AI levels above are fixed to the fields they showed
            let shown: Vec<Option<usize>> = p
                .get("columns")
                .and_then(Value::as_array)
                .map(|cs| {
                    cs.iter()
                        .map(|c| c.as_u64().and_then(|c| usize::try_from(c).ok()))
                        .collect()
                })
                .unwrap_or_default();
            for (k, l) in spec.levels.iter_mut().enumerate().take(level) {
                if let Some(Some(c)) = shown.get(k)
                    && spec.explain.contains(c)
                {
                    l.field = Some(*c);
                    l.ai = None;
                }
            }
            spec.levels.truncate(level);
            spec.path.truncate(level.saturating_sub(1));
            if level > 0 {
                if spec.path.len() + 1 < level {
                    return Err("the levels above have no selected member".into());
                }
                spec.path.push(value);
            }
            let used: Vec<usize> = spec.levels.iter().filter_map(|l| l.field).collect();
            if column.is_some_and(|c| used.contains(&c)) {
                return Err("that field is already a level of the tree".into());
            }
            spec.levels.push(LevelSpec {
                field: if ai.is_some() { None } else { column },
                ai,
            });
            spec.scroll.truncate(level);
        }
        "decomp_collapse" => {
            if level == 0 || level > spec.levels.len() {
                return Err(format!("the tree shows no level {level}"));
            }
            spec.levels.truncate(level - 1);
            spec.path.truncate(level - 1);
            spec.scroll.truncate(level - 1);
            spec.shift = spec.shift.min(spec.levels.len());
        }
        other => return Err(format!("unknown tree action `{other}`")),
    }
    Ok(spec.to_state())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(extra: Value) -> Visual {
        let mut v: Visual = serde_json::from_value(json!({
        "name": "t", "visual_type": "decompositionTreeVisual",
        "fields": [
            {"role": "Analyze", "field": {"kind": "measure", "table": "S", "measure": "Total"}},
            {"role": "ExplainBy", "field": {"kind": "column", "table": "S", "column": "Region"}},
            {"role": "ExplainBy", "field": {"kind": "column", "table": "S", "column": "Category"}},
            {"role": "ExplainBy", "field": {"kind": "column", "table": "S", "column": "Channel"}}
        ]}))
        .unwrap_or_else(|e| panic!("{e}"));
        if let Value::Object(m) = extra {
            v.visual_extra.extend(m);
        }
        v
    }

    fn lvl(r: &str, pinned: bool, ai: Option<&str>) -> Value {
        let mut l = json!({"queryRefs": [format!("S.{r}")], "isCollapsed": true});
        if pinned {
            l["isPinned"] = json!(true);
        }
        if let Some(m) = ai {
            l["AIInformation"] = json!({"method": m, "disabled": false});
        }
        l
    }

    #[test]
    fn desktop_states_are_read() {
        // no state: the root only
        let s = Spec::of(&tree(json!({})));
        assert_eq!(s.analyze, Some(0));
        assert_eq!(s.explain, vec![1, 2, 3]);
        assert!(s.levels.is_empty());
        // Region expanded, South toggled, then "High value" (Desktop 2.159's shapes)
        let st = json!({"expansionStates": [{"roles": ["ExplainBy"], "levels": [
            lvl("Region", true, None), lvl("Channel", true, Some("MaxSplit")), lvl("Category", false, None)],
            "root": {"children": [{"identityValues": [{"Literal": {"Value": "'South'"}}], "isToggled": true}]}}]});
        let s = Spec::of(&tree(st));
        assert_eq!(
            s.levels,
            vec![
                LevelSpec {
                    field: Some(1),
                    ai: None
                },
                LevelSpec {
                    field: Some(3),
                    ai: Some(Ai::High)
                }
            ]
        );
        assert_eq!(s.path, vec![json!("South")]);
        // a blank member and a number on the path; a level naming an unknown field stops it
        let st = json!({"expansionStates": [{"roles": ["ExplainBy"], "levels": [
            lvl("Region", true, None), lvl("Nope", true, None), lvl("Category", true, None)],
            "root": {"children": [{"identityValues": [{"Literal": {"Value": "null"}}], "isToggled": true,
                "children": [{"identityValues": [{"Literal": {"Value": "3L"}}], "isToggled": true}]}]}}]});
        let s = Spec::of(&tree(st));
        assert_eq!(s.levels.len(), 1);
        assert_eq!(s.path, vec![Value::Null, json!(3.0)]);
    }

    #[test]
    fn hostile_states_are_bounded() {
        let mut node = json!({"identityValues": [{"Literal": {"Value": "'x'"}}], "isToggled": true});
        for _ in 0..500 {
            node = json!({"identityValues": [{"Literal": {"Value": "'x'"}}], "isToggled": true, "children": [node]});
        }
        let levels: Vec<Value> = (0..100).map(|_| lvl("Region", true, Some("MinSplit"))).collect();
        let st = json!({"expansionStates": [{"roles": ["ExplainBy"], "levels": levels, "root": {"children": [node]}}]});
        let s = Spec::of(&tree(st));
        assert_eq!(s.levels.len(), MAX_LEVELS);
        assert_eq!(s.path.len(), MAX_LEVELS);
        // garbage shapes
        for bad in [
            json!(3),
            json!([1, 2]),
            json!([{"roles": "ExplainBy"}]),
            json!([{"roles": ["ExplainBy"], "levels": 5, "root": []}]),
        ] {
            let s = Spec::of(&tree(json!({"expansionStates": bad})));
            assert!(s.levels.is_empty());
        }
    }

    #[test]
    fn scroll_positions_are_reader_state_reset_below_a_new_path() {
        let v = tree(json!({}));
        let act =
            |st: Value, a: &str, p: Value| decomp_action(&v, st, a, &p).unwrap_or_else(|e| panic!("{e}"));
        // Region, then Category under South
        let st = act(Value::Null, "decomp_expand", json!({"level": 0, "column": 1}));
        let st = act(
            st,
            "decomp_expand",
            json!({"level": 1, "value": "South", "column": 2}),
        );
        // scroll both levels, and the tree one level to the left
        let st = act(st, "decomp_scroll", json!({"level": 1, "start": 3}));
        let st = act(st, "decomp_scroll", json!({"level": 2, "start": 5}));
        let st = act(st, "decomp_scroll", json!({"shift": 1}));
        assert_eq!(st["scroll"], json!([3, 5]));
        assert_eq!(st["shift"], 1);
        // a new member of level 1: level 1 keeps its window, level 2 opens afresh
        let moved = act(st.clone(), "decomp_path", json!({"level": 1, "value": "North"}));
        assert_eq!(moved["scroll"], json!([3]));
        assert_eq!(moved["path"], json!(["North"]));
        // removing level 2 drops its scroll and keeps the shift inside the levels left
        let cut = act(st.clone(), "decomp_collapse", json!({"level": 1}));
        assert_eq!(
            (cut["scroll"].clone(), cut["shift"].clone()),
            (json!([]), json!(0))
        );
        // the state read back holds them (the snapshot evaluates from it)
        let mut spec = Spec::of(&v);
        let _ = spec.read_state(&st);
        assert_eq!((spec.scroll.clone(), spec.shift), (vec![Some(3), Some(5)], 1));
        // refused: a level not shown, no position, both forms at once, a shift past the levels
        for bad in [
            json!({"level": 3, "start": 0}),
            json!({"level": 1}),
            json!({"level": 1, "start": -1}),
            json!({"level": 1, "start": 1, "shift": 1}),
            json!({"shift": 3}),
        ] {
            assert!(
                decomp_action(&v, st.clone(), "decomp_scroll", &bad).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn extremes_skip_blanks_and_keep_the_first() {
        let nodes = vec![
            (json!("a"), json!(5.0)),
            (json!("b"), Value::Null),
            (json!("c"), json!(9.0)),
            (json!("d"), json!(9.0)),
            (Value::Null, json!(1.0)),
        ];
        assert_eq!(extreme(&nodes, Ai::High), Some((2, 9.0)));
        assert_eq!(extreme(&nodes, Ai::Low), Some((4, 1.0)));
        assert_eq!(extreme(&[], Ai::Low), None);
    }

    #[test]
    fn relative_splits_compare_to_the_level_mean() {
        // D12 of the oracle: a member at 1.9 times its level's mean beats one at 1.8
        let cat = vec![
            (json!("a"), json!(116.0)),
            (json!("b"), json!(29.0)),
            (json!("c"), json!(45.6)),
        ];
        let mut prod: Vec<(Value, Value)> = (0..13).map(|i| (json!(i), json!(12.65))).collect();
        prod.push((json!("p"), json!(26.2)));
        let (_, c) = score(&cat, Ai::High, true).unwrap_or_default();
        let (k, p) = score(&prod, Ai::High, true).unwrap_or_default();
        assert!(p > c, "{p} {c}");
        assert_eq!(k, 13);
        // absolute: the category wins
        assert!(
            score(&cat, Ai::High, false).unwrap_or_default().1
                > score(&prod, Ai::High, false).unwrap_or_default().1
        );
        // a level of blanks or zeros has no relative score beyond its absolute one
        assert_eq!(score(&[(json!("z"), json!(0.0))], Ai::Low, true), Some((0, 0.0)));
    }

    #[test]
    fn members_compare_like_the_engine() {
        assert!(same_member(&json!("South"), &json!("south")));
        assert!(same_member(&Value::Null, &json!("")));
        assert!(same_member(&json!(3), &json!(3.0)));
        assert!(!same_member(&json!("a"), &json!("b")));
    }

    #[test]
    fn options_decode_literals_and_colours() {
        let v = tree(json!({"objects": {
            "tree": [{"properties": {"density": {"expr": {"Literal": {"Value": "'dense'"}}},
                                     "barsPerLevel": {"expr": {"Literal": {"Value": "6L"}}}}}],
            "dataBars": [{"properties": {"dataBarColor": {"solid": {"color": {"expr": {"Literal": {"Value": "'#FF0000'"}}}}}}},
                         {"properties": {"dataBarColor": {"solid": {"color": {"expr": {"Literal": {"Value": "'#00FF00'"}}}}}},
                          "selector": {"data": []}}],
            "other": [{"properties": {"x": {"expr": {"Literal": {"Value": "1L"}}}}}]}}));
        assert_eq!(
            options(&v),
            json!({"tree": {"density": "dense", "barsPerLevel": 6.0}, "dataBars": {"dataBarColor": "#FF0000"}})
        );
    }
}
