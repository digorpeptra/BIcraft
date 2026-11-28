//! The reader's drill, matrix expansion, drillthrough and report-page tooltips
//! (`docs/view-snapshot.md`, "Drill, drillthrough and page tooltips"). Like the rest of the
//! interaction state, nothing here reaches the files on its own: the state is applied to the
//! derived reports of [`Interactions::effective`] with the fields Power BI Desktop writes when it
//! saves a drilled visual (the projections' `active` flags, `filterConfig` containers with
//! `howCreated: "Drill"`, a matrix's `expansionStates`, the drillthrough page's
//! `howCreated: "Drillthrough"` filters given a value), so the evaluator reads them as it reads a
//! saved file. `view.keep_drill` writes the drill state into the report on request.
//!
//! Desktop's behaviour as Microsoft Learn documents it (read 2026-11-06; not recorded in Desktop,
//! "documented, not measured" in the parity rows):
//! - *Drill up* goes one level up; *drill mode* (the ↓ toggle) makes a click on a data point drill
//!   into it; *go to the next level* replaces the level by the next one for every point;
//!   *expand all down one level* keeps the level and adds the next one under it. Drilling does not
//!   filter the other visuals unless the designer turns "Drilling filters other visuals" on (read
//!   from the drilled visual's `drillFilterOtherVisuals`). Without drill mode a click selects.
//! - A matrix's members expand and collapse one by one (+/−, "Expand / Collapse > Selection"), a
//!   whole level ("Entire level") or everything ("All").
//! - *Drillthrough*: a right click on a data point of a visual that contains a drillthrough field
//!   of a page opens that page filtered on the point's value; "Keep all filters" (on unless
//!   `pageBinding.acceptsFilterContext` is `None`) also passes the source visual's filters; the
//!   page's Back button returns. Cross-report drillthrough (`referenceScope: CrossReport`) is a
//!   service feature and is refused.
//! - *Report-page tooltips*: a visual's tooltip page (`visualTooltip` `type: Canvas`, or "Auto": a
//!   tooltip page whose tooltip fields the visual uses) is shown on hover, filtered by the hovered
//!   point (a line chart's legend value excluded) and the source visual's filters.

use std::collections::HashMap;

use bicraft_model::{DataType, SemanticModel};
use bicraft_report::{FieldRef, Page, Report, Visual};
use serde_json::{Map, Value, json};

use super::{Interactions, Pv, data_type, in_filter, interaction_type, literal_text, unquote, value_literal};

/// Name prefix of the drill filters written into the drilled visual (data report only).
const DRILL: &str = "bicraft-drill";
/// Name prefix of the drill filters added to the other visuals.
const DRILL_OTHER: &str = "bicraft-drill-from";
/// Name prefix of the filters a drillthrough or a tooltip carries to its page.
const CARRIED: &str = "bicraft-carried";
/// Most levels of one drill hierarchy.
const MAX_LEVELS: usize = 16;
/// Most member paths a matrix's expansion holds.
const MAX_TOGGLED: usize = 2000;
/// Most filters carried to a drillthrough or tooltip page.
const MAX_CARRIED: usize = 256;

/// One level of a drill hierarchy: the model column behind it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Level {
    pub table: String,
    pub column: String,
    pub name: String,
    pub ty: DataType,
    /// The projection's `queryRef` (a matrix's `expansionStates` names its levels by it).
    pub query_ref: String,
}

/// A visual's drill hierarchy: the first drill role (`drilldown.roles`) with two or more column
/// fields, or the role asked for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Hier {
    /// The role as the query names it (`Category`, `Rows`…).
    pub role: String,
    pub levels: Vec<Level>,
    pub matrix: bool,
}

/// A matrix's expansion over its displayed levels: per level whether its members are expanded,
/// and the member paths (top displayed level first) that are the exceptions.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct MExp {
    pub open: Vec<bool>,
    pub toggled: Vec<Vec<Value>>,
}

/// The drill state of one visual.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Drill {
    pub hier: Hier,
    /// First and last displayed levels (`bottom > top`: expanded).
    pub top: usize,
    pub bottom: usize,
    /// The values drilled into, by level (levels above `top`).
    pub path: Vec<(usize, Value)>,
    /// Drill mode: a click on a point drills into it.
    pub mode: bool,
    /// A matrix's expansion as the reader set it (`None`: the saved one).
    pub exp: Option<MExp>,
}

/// A drillthrough page's filters, set by the last drillthrough to it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Through {
    pub source: (String, String),
    /// The page's drillthrough containers (by name) and their filter.
    pub values: Vec<(String, Value)>,
    /// The source visual's filters ("Keep all filters").
    pub context: Vec<Value>,
}

/// The report-page tooltip being shown.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Tip {
    pub page: String,
    pub visual: String,
    pub point: Value,
    pub target: String,
    pub filters: Vec<Value>,
}

// ---------------------------------------------------------------------------------------------
// Hierarchies

/// The projections of `role` in the visual's query.
fn projections<'a>(v: &'a Visual, role: &str) -> Option<&'a Vec<Value>> {
    v.visual_extra
        .get("query")?
        .get("queryState")?
        .as_object()?
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(role))?
        .1
        .get("projections")?
        .as_array()
}

/// The visual's drill hierarchy (`role`: that role only).
pub(crate) fn hierarchy(model: Option<&SemanticModel>, v: &Visual, role: Option<&str>) -> Option<Hier> {
    let ty = v.visual_type.as_deref()?;
    if matches!(ty, "tableEx" | "table" | "decompositionTreeVisual") || crate::preview::filters::is_slicer(v)
    {
        return None;
    }
    let vt = bicraft_visuals::visual(ty)?;
    let normal = model.and_then(|m| crate::preview::normalized(m, v));
    let fv = normal.as_ref().unwrap_or(v);
    for r in vt.data_roles.iter().filter(|r| r.drill) {
        if role.is_some_and(|w| !w.eq_ignore_ascii_case(&r.name)) {
            continue;
        }
        let bound: Vec<&bicraft_report::FieldBinding> = fv
            .fields
            .iter()
            .filter(|b| b.role.eq_ignore_ascii_case(&r.name))
            .take(MAX_LEVELS + 1)
            .collect();
        if bound.len() < 2 || bound.len() > MAX_LEVELS {
            continue;
        }
        let Some(ps) = projections(fv, &r.name) else {
            continue;
        };
        if ps.len() != bound.len() {
            continue;
        }
        let mut levels = Vec::new();
        for (b, p) in bound.iter().zip(ps) {
            let FieldRef::Column { table, column } = &b.field else {
                break;
            };
            levels.push(Level {
                table: table.clone(),
                column: column.clone(),
                name: b.display_name.clone().unwrap_or_else(|| column.clone()),
                ty: data_type(model, table, column),
                query_ref: p
                    .get("queryRef")
                    .and_then(Value::as_str)
                    .map_or_else(|| format!("{table}.{column}"), str::to_string),
            });
        }
        if levels.len() != bound.len() {
            continue;
        }
        let role_name = bound.first().map_or_else(|| r.name.clone(), |b| b.role.clone());
        return Some(Hier {
            role: role_name,
            levels,
            matrix: ty == "pivotTable",
        });
    }
    None
}

/// The value of a semantic-query literal (`'Bikes'`, `12L`, `datetime'…'`, `true`).
fn literal_value(s: &str) -> Value {
    if let Some(t) = s.strip_prefix("datetime'").and_then(|t| t.strip_suffix('\'')) {
        return json!(t.strip_suffix("T00:00:00").unwrap_or(t));
    }
    if let Some(t) = s.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')) {
        return json!(t.replace("''", "'"));
    }
    match s {
        "null" => return Value::Null,
        "true" => return json!(true),
        "false" => return json!(false),
        _ => {}
    }
    s.strip_suffix(['L', 'D', 'M'])
        .and_then(|t| t.parse::<f64>().ok())
        .and_then(serde_json::Number::from_f64)
        .map_or_else(|| json!(s), Value::Number)
}

/// The drill state the file stores: the displayed levels from the `active` flags, the values
/// drilled into from the `Drill` filters.
pub(crate) fn stored(v: &Visual, hier: &Hier) -> Drill {
    let n = hier.levels.len();
    let flags: Vec<Option<bool>> = projections(v, &hier.role)
        .map(|ps| {
            ps.iter()
                .map(|p| p.get("active").and_then(Value::as_bool))
                .collect()
        })
        .unwrap_or_default();
    let active: Vec<usize> = flags
        .iter()
        .enumerate()
        .filter(|(_, f)| **f == Some(true))
        .map(|(i, _)| i)
        .collect();
    let (top, bottom) = match (active.first(), active.last()) {
        (Some(a), Some(b)) => (*a, *b),
        // no flag: a chart shows every level (bicraft's reading of such files), a matrix its top
        _ if hier.matrix || flags.iter().any(Option::is_some) => (0, 0),
        _ => (0, n.saturating_sub(1)),
    };
    let mut path = Vec::new();
    for f in v
        .filter_config
        .as_ref()
        .and_then(|c| c.get("filters"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|f| f.get("howCreated").and_then(Value::as_str) == Some("Drill"))
    {
        let field = bicraft_report::query::field_ref(f.get("field"));
        let FieldRef::Column { table, column } = field else {
            continue;
        };
        let Some(level) = hier
            .levels
            .iter()
            .position(|l| l.table.eq_ignore_ascii_case(&table) && l.column.eq_ignore_ascii_case(&column))
        else {
            continue;
        };
        let value = f
            .pointer("/filter/Where/0/Condition/In/Values/0/0/Literal/Value")
            .and_then(Value::as_str)
            .map(literal_value);
        if let Some(value) = value
            && level < top
            && !path.iter().any(|(l, _)| *l == level)
        {
            path.push((level, value));
        }
    }
    path.sort_by_key(|(l, _)| *l);
    Drill {
        hier: hier.clone(),
        top: top.min(n.saturating_sub(1)),
        bottom: bottom.clamp(top, n.saturating_sub(1)),
        path,
        mode: false,
        exp: None,
    }
}

// ---------------------------------------------------------------------------------------------
// Applying the state to a visual

fn drill_filter(l: &Level, value: &Value) -> Option<Value> {
    in_filter(
        &l.table,
        std::slice::from_ref(&l.column),
        &[vec![value.clone()]],
        &[l.ty],
        false,
    )
    .ok()
}

/// The `filterConfig.filters` of `v`, created when missing.
fn filters_mut(v: &mut Visual) -> Option<&mut Vec<Value>> {
    let config = v.filter_config.get_or_insert_with(|| json!({"filters": []}));
    if !config.is_object() {
        *config = json!({"filters": []});
    }
    let o = config.as_object_mut()?;
    if !o.get("filters").is_some_and(Value::is_array) {
        o.insert("filters".into(), json!([]));
    }
    o.get_mut("filters")?.as_array_mut()
}

/// The drill containers of `d` (Desktop's `howCreated: "Drill"` shape).
fn drill_containers(d: &Drill, prefix: &str) -> Vec<Value> {
    d.path
        .iter()
        .filter_map(|(lv, value)| {
            let l = d.hier.levels.get(*lv)?;
            let filter = drill_filter(l, value)?;
            Some(json!({
                "name": format!("{prefix}-{lv}"),
                "field": bicraft_report::query::field_json(&FieldRef::Column {
                    table: l.table.clone(),
                    column: l.column.clone(),
                }),
                "type": "Categorical",
                "filter": filter,
                "howCreated": "Drill",
            }))
        })
        .collect()
}

/// A literal of level `l`'s column.
fn level_literal(l: Option<&Level>, x: &Value) -> Value {
    json!({"Literal": {"Value": value_literal(x, l.map_or(DataType::Unknown, |l| l.ty))}})
}

/// The `expansionStates` entry of a matrix's expansion over its displayed levels.
fn expansion_json(d: &Drill, e: &MExp) -> Value {
    let shown: Vec<&Level> = d
        .hier
        .levels
        .iter()
        .skip(d.top)
        .take(d.bottom.saturating_sub(d.top).saturating_add(1))
        .collect();
    let levels: Vec<Value> = shown
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let mut o = Map::new();
            o.insert("queryRefs".into(), json!([l.query_ref]));
            if !e.open.get(i).copied().unwrap_or(false) {
                o.insert("isCollapsed".into(), json!(true));
            }
            o.insert("isPinned".into(), json!(true));
            Value::Object(o)
        })
        .collect();
    // the toggled members as a tree, parents before children
    let mut root: Vec<Value> = Vec::new();
    let mut paths = e.toggled.clone();
    paths.sort_by_key(Vec::len);
    for p in paths.iter().take(MAX_TOGGLED) {
        let mut list = &mut root;
        for (depth, x) in p.iter().enumerate() {
            let lit = level_literal(shown.get(depth).copied(), x);
            let at = list
                .iter()
                .position(|n| n.pointer("/identityValues/0") == Some(&lit));
            let i = match at {
                Some(i) => i,
                None => {
                    list.push(json!({"identityValues": [lit], "children": []}));
                    list.len().saturating_sub(1)
                }
            };
            let Some(node) = list.get_mut(i).and_then(Value::as_object_mut) else {
                break;
            };
            if depth + 1 == p.len() {
                node.insert("isToggled".into(), json!(true));
            }
            if !node.get("children").is_some_and(Value::is_array) {
                node.insert("children".into(), json!([]));
            }
            let Some(kids) = node.get_mut("children").and_then(Value::as_array_mut) else {
                break;
            };
            list = kids;
        }
    }
    fn prune(list: &mut [Value]) {
        for n in list.iter_mut() {
            if let Some(o) = n.as_object_mut() {
                if let Some(k) = o.get_mut("children").and_then(Value::as_array_mut) {
                    prune(k);
                }
                if o.get("children")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
                {
                    o.remove("children");
                }
            }
        }
    }
    prune(&mut root);
    json!({"roles": [d.hier.role], "levels": levels, "root": {"children": root}})
}

/// `v` with drill state `d`: the role's `active` flags, its `Drill` filters, a matrix's
/// expansion.
pub(crate) fn apply(v: &mut Visual, d: &Drill) {
    // a visual whose query an edit left behind (or that has none yet) gets it rebuilt first, as
    // the evaluator would read it
    let raw = v.visual_extra.get("query");
    if bicraft_report::query::derive(raw) != v.fields
        && let Some(q) = bicraft_report::query::regenerate(raw, &v.fields)
    {
        v.visual_extra.insert("query".into(), q);
    }
    if let Some(ps) = v
        .visual_extra
        .get_mut("query")
        .and_then(|q| q.get_mut("queryState"))
        .and_then(Value::as_object_mut)
        .and_then(|s| {
            s.iter_mut()
                .find(|(k, _)| k.eq_ignore_ascii_case(&d.hier.role))
                .map(|(_, r)| r)
        })
        .and_then(|r| r.get_mut("projections"))
        .and_then(Value::as_array_mut)
    {
        for (k, p) in ps.iter_mut().enumerate() {
            if let Some(o) = p.as_object_mut() {
                o.insert("active".into(), json!(k >= d.top && k <= d.bottom));
            }
        }
    }
    if let Some(list) = filters_mut(v) {
        list.retain(|f| f.get("howCreated").and_then(Value::as_str) != Some("Drill"));
        list.extend(drill_containers(d, DRILL));
    }
    if d.hier.matrix {
        let entry = d.exp.as_ref().map(|e| expansion_json(d, e));
        let states = v
            .visual_extra
            .entry("expansionStates".to_string())
            .or_insert_with(|| json!([]));
        if !states.is_array() {
            *states = json!([]);
        }
        if let Some(list) = states.as_array_mut() {
            let role = d.hier.role.clone();
            let mine = |e: &Value| {
                e.get("roles").and_then(Value::as_array).is_some_and(|r| {
                    r.iter()
                        .any(|x| x.as_str().is_some_and(|s| s.eq_ignore_ascii_case(&role)))
                })
            };
            match entry {
                Some(e) => {
                    list.retain(|x| !mine(x));
                    list.push(e);
                }
                // levels moved (drill down, next level): the saved expansion no longer applies
                None if d.bottom == d.top => list.retain(|x| !mine(x)),
                None => {}
            }
        }
    }
}

/// Applies every drill, drillthrough and tooltip state onto the data report `r`.
pub(crate) fn apply_all(st: &Interactions, r: &mut Report) {
    // the report's "Drilling filters other visuals", unless the visual says otherwise
    let report_default = super::drill_filters_default(r);
    for page in &mut r.pages {
        let mut others: Vec<(String, Vec<Value>)> = Vec::new();
        for v in &mut page.visuals {
            let Some(d) = st.drills.get(&(page.name.clone(), v.name.clone())) else {
                continue;
            };
            apply(v, d);
            let filters_others = v
                .visual_extra
                .get("drillFilterOtherVisuals")
                .and_then(Value::as_bool)
                .unwrap_or(report_default);
            if filters_others && !d.path.is_empty() {
                others.push((
                    v.name.clone(),
                    drill_containers(d, &format!("{DRILL_OTHER}-{}", v.name)),
                ));
            }
        }
        for (source, containers) in others {
            let names: Vec<String> = page
                .visuals
                .iter()
                .filter(|t| {
                    t.name != source
                        && !t.fields.is_empty()
                        && interaction_type(page, &source, &t.name) != Some("NoFilter")
                })
                .map(|t| t.name.clone())
                .collect();
            for t in page.visuals.iter_mut().filter(|t| names.contains(&t.name)) {
                if let Some(list) = filters_mut(t) {
                    list.extend(containers.iter().cloned());
                }
            }
        }
    }
    for page in &mut r.pages {
        if let Some(t) = st.through.get(&page.name) {
            set_page_values(page, &t.values);
            add_page_filters(page, &t.context, "through");
        }
        if let Some(tip) = st.tip.as_ref().filter(|t| t.target == page.name) {
            add_page_filters(page, &tip.filters, "tooltip");
        }
    }
}

fn page_filters_mut(page: &mut Page) -> Option<&mut Vec<Value>> {
    let config = page.filter_config.get_or_insert_with(|| json!({"filters": []}));
    if !config.is_object() {
        *config = json!({"filters": []});
    }
    let o = config.as_object_mut()?;
    if !o.get("filters").is_some_and(Value::is_array) {
        o.insert("filters".into(), json!([]));
    }
    o.get_mut("filters")?.as_array_mut()
}

fn set_page_values(page: &mut Page, values: &[(String, Value)]) {
    let Some(list) = page_filters_mut(page) else {
        return;
    };
    for f in list.iter_mut() {
        let name = f
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if let Some((_, filter)) = values.iter().find(|(n, _)| *n == name)
            && let Some(o) = f.as_object_mut()
        {
            o.insert("filter".into(), filter.clone());
        }
    }
}

fn add_page_filters(page: &mut Page, filters: &[Value], kind: &str) {
    let Some(list) = page_filters_mut(page) else {
        return;
    };
    for (i, f) in filters.iter().enumerate() {
        list.push(json!({"name": format!("{CARRIED}-{kind}-{i}"), "filter": f}));
    }
}

// ---------------------------------------------------------------------------------------------
// Pages: drillthrough and tooltips

pub(crate) fn page_kind(p: &Page) -> Option<&str> {
    p.extra.get("type").and_then(Value::as_str)
}

/// A drillthrough or tooltip page's fields: (container name, column, summarized).
pub(crate) fn page_fields(p: &Page) -> Vec<(String, String, String, bool)> {
    let params: Vec<&Value> = p
        .extra
        .get("pageBinding")
        .and_then(|b| b.get("parameters"))
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    p.filter_config
        .as_ref()
        .and_then(|c| c.get("filters"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|f| f.get("howCreated").and_then(Value::as_str) == Some("Drillthrough"))
        .filter_map(|f| {
            let name = f.get("name").and_then(Value::as_str)?.to_string();
            let FieldRef::Column { table, column } = bicraft_report::query::field_ref(f.get("field")) else {
                return None;
            };
            let agg = params.iter().any(|x| {
                x.get("boundFilter").and_then(Value::as_str) == Some(name.as_str())
                    && x.get("asAggregation").and_then(Value::as_bool) == Some(true)
            });
            Some((name, table, column, agg))
        })
        .take(64)
        .collect()
}

pub(crate) fn cross_report(p: &Page) -> bool {
    p.extra
        .get("pageBinding")
        .and_then(|b| b.get("referenceScope"))
        .and_then(Value::as_str)
        == Some("CrossReport")
}

pub(crate) fn keep_all(p: &Page) -> bool {
    p.extra
        .get("pageBinding")
        .and_then(|b| b.get("acceptsFilterContext"))
        .and_then(Value::as_str)
        != Some("None")
}

/// The displayed fields of `shown` (the visual at its drill level) that match a column.
fn column_at(shown: &Visual, table: &str, column: &str) -> Vec<usize> {
    shown
        .fields
        .iter()
        .enumerate()
        .filter(|(_, b)| {
            matches!(&b.field, FieldRef::Column { table: t, column: c }
                if t.eq_ignore_ascii_case(table) && c.eq_ignore_ascii_case(column))
        })
        .map(|(i, _)| i)
        .collect()
}

/// The drillthrough pages a visual can reach: `(page, columns of the point that feed it)`.
pub(crate) fn drillthrough_targets<'a>(r: &'a Report, shown: &Visual) -> Vec<(&'a Page, Vec<usize>)> {
    r.pages
        .iter()
        .filter(|p| page_kind(p) == Some("Drillthrough"))
        .filter_map(|p| {
            let mut cols: Vec<usize> = Vec::new();
            for (_, t, c, agg) in page_fields(p) {
                if !agg {
                    cols.extend(column_at(shown, &t, &c));
                }
            }
            cols.sort_unstable();
            cols.dedup();
            (!cols.is_empty()).then_some((p, cols))
        })
        .collect()
}

/// The tooltip page of a visual: the one its `visualTooltip` names, else ("Auto") the first
/// tooltip page one of whose tooltip fields the visual uses (a column as a column, a summarized
/// field as a summarized one); `None` with tooltips off or the default tooltip.
pub(crate) fn tooltip_page<'a>(r: &'a Report, v: &Visual) -> Option<&'a Page> {
    let props = v
        .visual_extra
        .get("visualContainerObjects")
        .and_then(|o| o.get("visualTooltip"))
        .and_then(Value::as_array)
        .and_then(|l| l.iter().find(|e| e.get("selector").is_none_or(Value::is_null)))
        .and_then(|e| e.get("properties"));
    let lit = |k: &str| props.and_then(|p| literal_text(p.get(k))).map(|s| unquote(&s));
    if lit("show").as_deref() == Some("false") {
        return None;
    }
    if lit("type").as_deref() == Some("Canvas")
        && let Some(section) = lit("section")
    {
        return r
            .pages
            .iter()
            .find(|p| p.name == section)
            .or_else(|| r.pages.iter().find(|p| p.display_name == section))
            .filter(|p| page_kind(p) == Some("Tooltip"));
    }
    if lit("type").is_some() {
        return None;
    }
    r.pages
        .iter()
        .filter(|p| page_kind(p) == Some("Tooltip"))
        .find(|p| {
            page_fields(p).iter().any(|(_, t, c, agg)| {
                v.fields.iter().any(|b| match &b.field {
                    FieldRef::Column { table, column } => {
                        !agg && table.eq_ignore_ascii_case(t) && column.eq_ignore_ascii_case(c)
                    }
                    FieldRef::Aggregation { table, column, .. } => {
                        *agg && table.eq_ignore_ascii_case(t) && column.eq_ignore_ascii_case(c)
                    }
                    _ => false,
                })
            })
        })
}

/// The filters acting on visual `visual` of `page` in the data report `r`: the page's, the
/// visual's own (drill and cross-filter included) and the selections of the slicers that reach
/// it (report filters apply to every page and are not carried).
pub(crate) fn context(r: &Report, page: &str, visual: &str) -> Vec<Value> {
    let Some(p) = r.pages.iter().find(|p| p.name == page) else {
        return Vec::new();
    };
    let of = |c: Option<&Value>| -> Vec<Value> {
        c.and_then(|c| c.get("filters"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|f| f.get("filter").cloned())
            .collect()
    };
    let mut out = of(p.filter_config.as_ref());
    if let Some(v) = p.visuals.iter().find(|v| v.name == visual) {
        out.extend(of(v.filter_config.as_ref()));
    }
    for s in p
        .visuals
        .iter()
        .filter(|s| s.name != visual && crate::preview::filters::is_slicer(s))
    {
        if interaction_type(p, &s.name, visual) == Some("NoFilter") {
            continue;
        }
        if let Some(f) = crate::preview::filters::slicer_filter(s) {
            out.push(f.clone());
        }
    }
    out.truncate(MAX_CARRIED);
    out
}

/// One filter per grouping value of a point (`skip`: data columns left out).
pub(crate) fn point_filters(point: &[Pv], skip: &[usize]) -> Vec<Value> {
    point
        .iter()
        .filter(|x| !skip.contains(&x.c))
        .filter_map(|x| {
            in_filter(
                &x.table,
                std::slice::from_ref(&x.column),
                &[vec![x.value.clone()]],
                &[x.ty],
                false,
            )
            .ok()
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Actions

/// The displayed field index of hierarchy level `level` in `shown` (the visual at its drill
/// level), from the level's column.
fn shown_index(shown: &Visual, d: &Drill, level: usize) -> Option<usize> {
    let l = d.hier.levels.get(level)?;
    shown.fields.iter().position(|b| {
        b.role.eq_ignore_ascii_case(&d.hier.role)
            && matches!(&b.field, FieldRef::Column { table, column }
                if table.eq_ignore_ascii_case(&l.table) && column.eq_ignore_ascii_case(&l.column))
    })
}

/// "Drill down" on a point: the displayed levels' values become drill filters and the next
/// level is shown.
pub(crate) fn down(d: &mut Drill, shown: &Visual, point: &[Pv]) -> Result<(), String> {
    let n = d.hier.levels.len();
    if d.bottom.saturating_add(1) >= n {
        return Err("the visual is at the lowest level of its hierarchy".into());
    }
    let mut got = Vec::new();
    for lv in d.top..=d.bottom {
        let Some(i) = shown_index(shown, d, lv) else {
            continue;
        };
        if let Some(x) = point.iter().find(|x| x.c == i) {
            got.push((lv, x.value.clone()));
        }
    }
    if got.is_empty() {
        return Err("the point holds no value of the hierarchy's displayed level".into());
    }
    d.path.retain(|(l, _)| *l < d.top);
    d.path.extend(got);
    d.top = d.bottom.saturating_add(1);
    d.bottom = d.top;
    d.exp = None;
    Ok(())
}

/// "Drill up": the last expansion, else the last level drilled into, is undone.
pub(crate) fn up(d: &mut Drill) -> Result<(), String> {
    if d.bottom > d.top {
        d.bottom -= 1;
        if let Some(e) = d.exp.as_mut() {
            let shown = d.bottom - d.top + 1;
            e.open.truncate(shown);
            if let Some(last) = e.open.last_mut() {
                *last = false;
            }
            e.toggled.retain(|p| p.len() < shown);
        }
        return Ok(());
    }
    if d.top == 0 {
        return Err("the visual is at the top level of its hierarchy".into());
    }
    d.top -= 1;
    d.bottom = d.top;
    d.path.retain(|(l, _)| *l < d.top);
    d.exp = None;
    Ok(())
}

/// "Go to the next level": every point moves down one level.
pub(crate) fn next(d: &mut Drill) -> Result<(), String> {
    if d.bottom.saturating_add(1) >= d.hier.levels.len() {
        return Err("the visual is at the lowest level of its hierarchy".into());
    }
    d.top = d.bottom.saturating_add(1);
    d.bottom = d.top;
    d.exp = None;
    Ok(())
}

/// "Expand all down one level": the next level is added under the displayed ones.
pub(crate) fn expand_all(d: &mut Drill) -> Result<(), String> {
    if d.bottom.saturating_add(1) >= d.hier.levels.len() {
        return Err("the visual is at the lowest level of its hierarchy".into());
    }
    d.bottom += 1;
    if d.hier.matrix {
        let shown = d.bottom - d.top + 1;
        d.exp = Some(MExp {
            open: (0..shown).map(|i| i + 1 < shown).collect(),
            toggled: Vec::new(),
        });
    }
    Ok(())
}

/// The matrix expansion currently shown, from the state or the saved `expansionStates`.
pub(crate) fn current_exp(d: &Drill, shown: &Visual) -> MExp {
    if let Some(e) = &d.exp {
        return e.clone();
    }
    let n = d.bottom - d.top + 1;
    match crate::preview::matrix_expansion(shown, &d.hier.role, n) {
        Some((open, toggled)) => MExp { open, toggled },
        None => MExp {
            open: vec![false; n],
            toggled: Vec::new(),
        },
    }
}

/// Whether the member at `path` (relative depth `path.len()`) is expanded under `e`.
fn is_open(e: &MExp, path: &[Value]) -> bool {
    let d = path.len();
    let base = d >= 1 && e.open.get(d - 1).copied().unwrap_or(false);
    base ^ e.toggled.iter().any(|p| same_path(p, path))
}

fn same_path(a: &[Value], b: &[Value]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| super::same_value(x, y))
}

/// A matrix's expand / collapse: a member (`path`, top displayed level first), an entire level
/// (`level`, 0 = the top displayed one) or everything (`all`).
pub(crate) fn matrix(d: &mut Drill, shown: &Visual, expand: bool, p: &Value) -> Result<(), String> {
    if !d.hier.matrix {
        return Err("expand and collapse apply to matrices; use drill_expand on other visuals".into());
    }
    let n = d.hier.levels.len();
    let mut e = current_exp(d, shown);
    let shown_n = |d: &Drill| d.bottom - d.top + 1;
    // one more level becomes visible (collapsed) when the deepest shown one is expanded
    let grow = |d: &mut Drill, e: &mut MExp| -> bool {
        if d.bottom.saturating_add(1) >= n {
            return false;
        }
        d.bottom += 1;
        e.open.resize(d.bottom - d.top + 1, false);
        true
    };
    e.open.resize(shown_n(d), false);
    if let Some(path) = p.get("path").filter(|x| !x.is_null()) {
        let path: Vec<Value> = path
            .as_array()
            .ok_or("`path` must be the member's values, top level first")?
            .iter()
            .take(MAX_LEVELS)
            .cloned()
            .collect();
        if path.is_empty() || path.iter().any(|x| x.is_array() || x.is_object()) {
            return Err("`path` must hold 1 or more values (numbers, text, booleans, null)".into());
        }
        if path.len() > shown_n(d) {
            return Err("`path` is deeper than the levels shown".into());
        }
        if expand && path.len() == shown_n(d) && !grow(d, &mut e) {
            return Err("the member is at the lowest level of the hierarchy".into());
        }
        if is_open(&e, &path) != expand {
            match e.toggled.iter().position(|q| same_path(q, &path)) {
                Some(i) => {
                    e.toggled.remove(i);
                }
                None if e.toggled.len() < MAX_TOGGLED => e.toggled.push(path),
                None => return Err(format!("at most {MAX_TOGGLED} members are expanded one by one")),
            }
        }
    } else if let Some(level) = p.get("level").filter(|x| !x.is_null()) {
        let level = level
            .as_u64()
            .and_then(|x| usize::try_from(x).ok())
            .ok_or("`level` must be a level index (0: the top level shown)")?;
        if level >= shown_n(d) {
            return Err("`level` is beyond the levels shown".into());
        }
        if expand && level + 1 == shown_n(d) && !grow(d, &mut e) {
            return Err("the level is the lowest of the hierarchy".into());
        }
        if let Some(o) = e.open.get_mut(level) {
            *o = expand;
        }
        e.toggled.retain(|q| q.len() != level + 1);
    } else if p.get("all").and_then(Value::as_bool) == Some(true) {
        if expand {
            while grow(d, &mut e) {}
        }
        let k = shown_n(d);
        e.open = (0..k).map(|i| expand && i + 1 < k).collect();
        e.toggled.clear();
    } else {
        return Err("give `path` (a member), `level` (an entire level) or `all: true`".into());
    }
    // the deepest level shown never expands
    if let Some(last) = e.open.last_mut() {
        *last = false;
    }
    d.exp = Some(e);
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Snapshot

/// The `drill` object of a visual in the snapshot.
pub(crate) fn drill_json(d: &Drill, stored_mode: bool) -> Value {
    let n = d.hier.levels.len();
    let exp = d
        .exp
        .as_ref()
        .map(|e| json!({"open": e.open, "toggled": e.toggled}));
    json!({
        "role": d.hier.role,
        "levels": d.hier.levels.iter().map(|l| l.name.clone()).collect::<Vec<_>>(),
        "top": d.top,
        "bottom": d.bottom,
        "mode": d.mode || stored_mode,
        "can_up": d.bottom > d.top || d.top > 0,
        "can_down": d.bottom + 1 < n,
        "path": d.path.iter().map(|(l, v)| json!({"level": l, "value": v})).collect::<Vec<_>>(),
        "matrix": d.hier.matrix,
        "expansion": exp,
    })
}

/// The tooltip state as JSON.
pub(crate) fn tip_json(t: &Tip) -> Value {
    json!({"page": t.page, "visual": t.visual, "point": t.point, "target": t.target})
}

/// Values compared for state equality (`HashMap`s of plain data).
pub(crate) fn same(a: &Interactions, b: &Interactions) -> bool {
    a.drills == b.drills && a.through == b.through && a.tip == b.tip
}

/// Every drill state as JSON (`view.interactions`).
pub(crate) fn drills_json(m: &HashMap<(String, String), Drill>) -> Vec<Value> {
    let mut out: Vec<Value> = m
        .iter()
        .map(|((p, v), d)| json!({"page": p, "visual": v, "drill": drill_json(d, false)}))
        .collect();
    out.sort_by_key(|x| x.to_string());
    out
}

/// The drillthrough states as JSON.
pub(crate) fn through_json(m: &HashMap<String, Through>) -> Vec<Value> {
    let mut out: Vec<Value> = m
        .iter()
        .map(|(p, t)| {
            json!({"page": p, "source": {"page": t.source.0, "visual": t.source.1},
                   "values": t.values.iter().map(|(n, f)| json!({"name": n, "filter": f})).collect::<Vec<_>>(),
                   "kept_filters": t.context.len()})
        })
        .collect();
    out.sort_by_key(|x| x.to_string());
    out
}

// ---------------------------------------------------------------------------------------------
// Bookmarks (G55)
//
// A bookmark captures a drilled visual's state in its container (public PBIR schema `bookmark`
// 2.1.0): `singleVisual.activeProjections` (the displayed levels' fields, by role: the
// `ProjectionState` "updated projections that are used by the visual"), a matrix's
// `singleVisual.expansionStates` when the reader expanded or collapsed members, and the values
// drilled into as `filters.byExpr` entries `{name, type: "Categorical", expression: <column>,
// filter, howCreated: 2}` (2 being `Drill` in the filter schema's `howCreated` order; the
// bookmark schema numbers it without naming it). The shape follows the schemas; how Desktop
// fills it is not observed.
//
// The drill mode (a click drills into the point) has no property in the bookmark schema
// (`SingleVisualConfigState` is closed; its `isDrillDisabled` turns drilling off, a different
// thing). BIcraft writes it as a formatting-object update of its own, which the schema allows:
// `singleVisual.objects.merge.bicraftDrill[0].properties.drillMode` = `true`
// ([`DRILL_MODE_OBJECT`]). Not seen in a Desktop file; Desktop is expected to ignore an object no
// visual declares.

/// The `howCreated` number of a drill filter in a bookmark (`Auto`, `User`, `Drill`… in order).
const BOOKMARK_DRILL: u64 = 2;
/// The formatting object of BIcraft's own that carries the drill mode in a bookmark
/// (`singleVisual.objects.merge.<this>[0].properties.drillMode`).
pub(crate) const DRILL_MODE_OBJECT: &str = "bicraftDrill";

/// What a bookmark container holds of a visual's drill state.
pub(crate) struct Captured {
    /// `singleVisual.activeProjections`.
    pub active: Value,
    /// `singleVisual.expansionStates` (a matrix the reader expanded or collapsed).
    pub expansion: Option<Value>,
    /// `filters.byExpr` entries: the values drilled into.
    pub filters: Vec<Value>,
    /// The drill mode is on (`singleVisual.objects.merge.bicraftDrill`).
    pub mode: bool,
}

/// The `objects.merge` entry that carries the drill mode in a bookmark container.
pub(crate) fn mode_object() -> Value {
    json!([{"properties": {"drillMode": {"expr": {"Literal": {"Value": "true"}}}}}])
}

/// Whether bookmark container `c` turns the drill mode on.
fn mode_of(c: &Value) -> bool {
    c.pointer("/singleVisual/objects/merge")
        .and_then(|m| m.get(DRILL_MODE_OBJECT))
        .and_then(Value::as_array)
        .and_then(|l| l.first())
        .and_then(|e| e.pointer("/properties/drillMode/expr/Literal/Value"))
        .and_then(Value::as_str)
        == Some("true")
}

/// The drill state of visual `v` of `page` the session shows, as a bookmark holds it (`None`:
/// not drilled by the reader).
pub(crate) fn capture(view: &Interactions, page: &str, v: &Visual) -> Option<Captured> {
    let d = view.drills.get(&(page.to_string(), v.name.clone()))?;
    let mut shown = v.clone();
    apply(&mut shown, d);
    let fields: Vec<Value> = projections(&shown, &d.hier.role)?
        .iter()
        .filter(|p| p.get("active").and_then(Value::as_bool) == Some(true))
        .filter_map(|p| p.get("field").cloned())
        .collect();
    let mut active = Map::new();
    active.insert(d.hier.role.clone(), Value::Array(fields));
    let expansion = if d.hier.matrix && d.exp.is_some() {
        shown.visual_extra.get("expansionStates").cloned()
    } else {
        None
    };
    let filters = drill_containers(d, DRILL)
        .into_iter()
        .map(|f| {
            json!({"name": f.get("name"), "type": "Categorical", "expression": f.get("field"),
                   "filter": f.get("filter"), "howCreated": BOOKMARK_DRILL})
        })
        .collect();
    Some(Captured {
        active: Value::Object(active),
        expansion,
        filters,
        mode: d.mode,
    })
}

/// The drill state bookmark container `c` holds for `v` (hierarchy `hier`), `None` when it holds
/// none or the stored state.
fn from_container(v: &Visual, hier: &Hier, c: &Value) -> Option<Drill> {
    let active = c
        .pointer("/singleVisual/activeProjections")
        .and_then(Value::as_object);
    let expansion = c
        .pointer("/singleVisual/expansionStates")
        .filter(|x| x.is_array());
    let drilled: Vec<&Value> = c
        .pointer("/filters/byExpr")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_LEVELS)
        .filter(|f| f.get("howCreated").and_then(Value::as_u64) == Some(BOOKMARK_DRILL))
        .collect();
    let mode = mode_of(c);
    if active.is_none() && expansion.is_none() && drilled.is_empty() && !mode {
        return None;
    }
    let mut shown = v.clone();
    let raw = shown.visual_extra.get("query");
    if bicraft_report::query::derive(raw) != shown.fields
        && let Some(q) = bicraft_report::query::regenerate(raw, &shown.fields)
    {
        shown.visual_extra.insert("query".into(), q);
    }
    // the displayed levels
    let list = active
        .and_then(|a| a.iter().find(|(k, _)| k.eq_ignore_ascii_case(&hier.role)))
        .and_then(|(_, l)| l.as_array());
    if let Some(list) = list {
        let want: Vec<FieldRef> = list
            .iter()
            .take(MAX_LEVELS)
            .map(|f| bicraft_report::query::field_ref(Some(f)))
            .collect();
        if let Some(ps) = shown
            .visual_extra
            .get_mut("query")
            .and_then(|q| q.get_mut("queryState"))
            .and_then(Value::as_object_mut)
            .and_then(|s| {
                s.iter_mut()
                    .find(|(k, _)| k.eq_ignore_ascii_case(&hier.role))
                    .map(|(_, r)| r)
            })
            .and_then(|r| r.get_mut("projections"))
            .and_then(Value::as_array_mut)
        {
            for p in ps.iter_mut() {
                let f = bicraft_report::query::field_ref(p.get("field"));
                let on = want.iter().any(|w| crate::refs::same_field(w, &f));
                if let Some(o) = p.as_object_mut() {
                    o.insert("active".into(), json!(on));
                }
            }
        }
    }
    // the values drilled into
    if let Some(fl) = filters_mut(&mut shown) {
        fl.retain(|f| f.get("howCreated").and_then(Value::as_str) != Some("Drill"));
        for (i, f) in drilled.iter().enumerate() {
            fl.push(json!({
                "name": f.get("name").and_then(Value::as_str).map_or_else(|| format!("{DRILL}-{i}"), str::to_string),
                "field": f.get("expression"),
                "type": "Categorical",
                "filter": f.get("filter"),
                "howCreated": "Drill",
            }));
        }
    }
    let mut d = stored(&shown, hier);
    d.mode = mode;
    if hier.matrix
        && let Some(e) = expansion
    {
        shown.visual_extra.insert("expansionStates".into(), e.clone());
        let n = d.bottom.saturating_sub(d.top).saturating_add(1);
        let mine = crate::preview::matrix_expansion(&shown, &hier.role, n);
        if mine.is_some() && mine != crate::preview::matrix_expansion(v, &hier.role, n) {
            d.exp = mine.map(|(open, toggled)| MExp { open, toggled });
        }
    }
    (d != stored(v, hier)).then_some(d)
}

/// Restores the drill states a bookmark's containers hold on `page` into `view`, for
/// `visuals`; a visual whose container holds none goes back to its stored state. True when the
/// view changed.
pub(crate) fn restore<'a>(
    view: &mut Interactions,
    model: Option<&SemanticModel>,
    page: &Page,
    containers: Option<&Map<String, Value>>,
    visuals: impl Iterator<Item = &'a Visual>,
) -> bool {
    let mut changed = false;
    for v in visuals {
        let key = (page.name.clone(), v.name.clone());
        let c = containers.and_then(|m| m.get(&v.name));
        let role = c
            .and_then(|c| c.pointer("/singleVisual/activeProjections"))
            .and_then(Value::as_object)
            .and_then(|a| a.keys().next())
            .map(String::as_str);
        let want = hierarchy(model, v, role)
            .or_else(|| hierarchy(model, v, None))
            .and_then(|h| c.and_then(|c| from_container(v, &h, c)));
        let before = view.drills.get(&key).cloned();
        match want {
            Some(d) => {
                view.drills.insert(key.clone(), d);
            }
            None => {
                view.drills.remove(&key);
            }
        }
        changed |= view.drills.get(&key) != before.as_ref();
    }
    changed
}
