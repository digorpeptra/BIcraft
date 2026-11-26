//! The fix commands of the report rules (`report_lint.rs`), for the places no existing command
//! repairs on its own:
//!
//! - `report.rebind {from, to}`: every reference the report makes to a column or measure the
//!   model lacks is pointed at another one (visual roles, sorts, filters, drillthrough and tooltip
//!   bindings, formatting, bookmarks, properties), with the queryRefs and the formatting selectors
//!   that name it, as `column.rename` with `update_report` rewrites the report.
//! - `report.remove_reference {field, place, page?, visual?, bookmark?}`: what refers to a field
//!   at one place goes: a drillthrough or tooltip page's field (its filter card and binding
//!   parameter), conditional formatting (the formatted properties, or the whole entry when its
//!   selector names the field), a bookmark's state (each entry of its lists that names it).
//! - `visual.fix_query_refs {page, visual}`: projections whose `queryRef` names another field
//!   take the field's own, and the selectors, bookmarks and phone layout naming them follow.
//! - `parameter.fix_field {name, field, to?}`: a field parameter's `NAMEOF` of a field is pointed
//!   at `to`, or its row goes.
//!
//! Each is one command, so one undo step; each refuses (changing nothing) when it would not
//! remove the reference it is asked about.

use bicraft_dax::ast::Expr;
use bicraft_report::{FieldRef, Visual, query};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::edit::{Edit, apply};
use crate::params::{allow, opt_str, str_param};
use crate::refs::{ModelChange, ParsedField, SiteKind, apply_change, field_text, find_refs, parse_field};
use crate::report::{page_index, parts, visual_index};

/// Deepest JSON nesting the strip walks descend into.
const MAX_DEPTH: usize = 256;

/// A column or measure as a fix names it (it may be missing from the model): `table` is `None`
/// for `[Measure]`, which matches the measure on any table.
#[derive(Debug, Clone)]
pub(crate) struct FieldName {
    pub table: Option<String>,
    pub name: String,
}

impl FieldName {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        Ok(match parse_field(text)? {
            ParsedField::Qualified { table, name } => Self {
                table: Some(table),
                name,
            },
            ParsedField::Measure(name) => Self { table: None, name },
            ParsedField::Aggregation { table, column, .. } => Self {
                table: Some(table),
                name: column,
            },
        })
    }

    fn matches(&self, table: &str, name: &str) -> bool {
        name.eq_ignore_ascii_case(&self.name)
            && self
                .table
                .as_deref()
                .is_none_or(|t| t.eq_ignore_ascii_case(table))
    }
}

/// Whether `v` holds a column or measure reference to `f`.
fn refers(v: &Value, f: &FieldName) -> bool {
    let mut hit = false;
    find_refs(v, &mut |s| {
        if s.kind != SiteKind::Table && !s.extension && f.matches(s.entity, s.property) {
            hit = true;
        }
    });
    hit
}

/// What a strip did: references removed, references it could not remove.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Strip {
    pub removed: usize,
    pub remaining: usize,
}

impl Strip {
    /// Removed something and left nothing behind.
    pub(crate) fn clean(&self) -> bool {
        self.removed > 0 && self.remaining == 0
    }
}

// ---------------------------------------------------------------------------------------------
// Drillthrough and tooltip page fields
// ---------------------------------------------------------------------------------------------

fn is_drill_filter(f: &Value) -> bool {
    f.get("howCreated")
        .and_then(Value::as_str)
        .is_some_and(|h| h.eq_ignore_ascii_case("drillthrough"))
}

/// Removes the page's drillthrough filter cards on `f` and the binding parameters that name `f`
/// or are bound to a removed card.
pub(crate) fn strip_drill(
    filter_config: &mut Option<Value>,
    extra: &mut Map<String, Value>,
    f: &FieldName,
) -> Strip {
    let mut out = Strip::default();
    let mut removed_names: Vec<String> = Vec::new();
    let mut emptied = false;
    if let Some(Value::Array(filters)) = filter_config.as_mut().and_then(|c| c.get_mut("filters")) {
        let before = filters.len();
        filters.retain(|x| {
            let hit = is_drill_filter(x) && refers(x, f);
            if hit && let Some(n) = x.get("name").and_then(Value::as_str) {
                removed_names.push(n.to_string());
            }
            !hit
        });
        out.removed = out.removed.saturating_add(before.saturating_sub(filters.len()));
        emptied = before > 0 && filters.is_empty();
        out.remaining = out.remaining.saturating_add(
            filters
                .iter()
                .filter(|x| is_drill_filter(x) && refers(x, f))
                .count(),
        );
    }
    if emptied && let Some(Value::Object(c)) = filter_config.as_mut() {
        c.shift_remove("filters");
        if c.is_empty() {
            *filter_config = None;
        }
    }
    if let Some(binding) = extra.get_mut("pageBinding") {
        if let Some(Value::Array(params)) = binding.get_mut("parameters") {
            let before = params.len();
            params.retain(|x| {
                let bound = x
                    .get("boundFilter")
                    .and_then(Value::as_str)
                    .is_some_and(|b| removed_names.iter().any(|n| n == b));
                !(bound || refers(x, f))
            });
            out.removed = out.removed.saturating_add(before.saturating_sub(params.len()));
        }
        if refers(binding, f) {
            out.remaining = out.remaining.saturating_add(1);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Conditional formatting
// ---------------------------------------------------------------------------------------------

/// In one `objects` map: entries whose selector names `f` go; else their properties naming `f`
/// go, and the entry with them when none is left. Objects left without entries go.
fn strip_objects(objects: &mut Map<String, Value>, f: &FieldName, out: &mut Strip) {
    let mut emptied: Vec<String> = Vec::new();
    for (key, entries) in objects.iter_mut() {
        let Some(list) = entries.as_array_mut() else {
            if refers(entries, f) {
                out.remaining = out.remaining.saturating_add(1);
            }
            continue;
        };
        let before = list.len();
        let mut removed = 0usize;
        list.retain_mut(|e| {
            if e.get("selector").is_some_and(|s| refers(s, f)) {
                removed = removed.saturating_add(1);
                return false;
            }
            let Some(Value::Object(props)) = e.get_mut("properties") else {
                return true;
            };
            let n = props.len();
            props.retain(|_, v| !refers(v, f));
            let gone = n.saturating_sub(props.len());
            removed = removed.saturating_add(gone);
            !(gone > 0 && props.is_empty())
        });
        out.removed = out.removed.saturating_add(removed);
        out.remaining = out
            .remaining
            .saturating_add(list.iter().filter(|e| refers(e, f)).count());
        if before > 0 && list.is_empty() {
            emptied.push(key.clone());
        }
    }
    for k in emptied {
        objects.shift_remove(&k);
    }
}

/// Every `objects` / `visualContainerObjects` map under `map` (at any depth), stripped of `f`.
/// A formatting key left empty goes.
pub(crate) fn strip_formatting(map: &mut Map<String, Value>, f: &FieldName) -> Strip {
    let mut out = Strip::default();
    strip_formatting_in(map, f, 0, &mut out);
    out
}

fn strip_formatting_in(map: &mut Map<String, Value>, f: &FieldName, depth: usize, out: &mut Strip) {
    if depth > MAX_DEPTH {
        return;
    }
    let mut emptied: Vec<String> = Vec::new();
    for (k, v) in map.iter_mut() {
        if k == "objects" || k == "visualContainerObjects" {
            if let Value::Object(objects) = v {
                let had = !objects.is_empty();
                strip_objects(objects, f, out);
                if had && objects.is_empty() {
                    emptied.push(k.clone());
                }
            }
            continue;
        }
        match v {
            Value::Object(m) => strip_formatting_in(m, f, depth.saturating_add(1), out),
            Value::Array(items) => {
                for item in items {
                    if let Value::Object(m) = item {
                        strip_formatting_in(m, f, depth.saturating_add(1), out);
                    }
                }
            }
            _ => {}
        }
    }
    for k in emptied {
        map.shift_remove(&k);
    }
}

// ---------------------------------------------------------------------------------------------
// Bookmarks
// ---------------------------------------------------------------------------------------------

/// Removes each element of the outermost lists of `doc` that names `f` (a filter, a sort, a
/// formatting entry of the bookmark's state), and each filter card of a `byName` map naming it.
pub(crate) fn strip_bookmark(doc: &mut Value, f: &FieldName) -> Strip {
    let mut out = Strip::default();
    strip_lists(doc, f, 0, &mut out);
    if refers(doc, f) {
        out.remaining = out.remaining.saturating_add(1);
    }
    out
}

fn strip_lists(v: &mut Value, f: &FieldName, depth: usize, out: &mut Strip) {
    if depth > MAX_DEPTH {
        return;
    }
    match v {
        Value::Array(items) => {
            let before = items.len();
            items.retain(|x| !refers(x, f));
            out.removed = out.removed.saturating_add(before.saturating_sub(items.len()));
        }
        Value::Object(m) => {
            // a PBIR bookmark keeps its filter cards keyed by name (`filters.byName`); a card's
            // query names its field through the `From` alias, so the card goes whole
            if let Some(Value::Object(cards)) = m.get_mut("byName") {
                let before = cards.len();
                cards.retain(|_, x| !refers(x, f));
                out.removed = out.removed.saturating_add(before.saturating_sub(cards.len()));
            }
            for x in m.values_mut() {
                strip_lists(x, f, depth.saturating_add(1), out);
            }
        }
        _ => {}
    }
}

/// `report.remove_reference {field, place, page?, visual?, bookmark?}`.
pub(crate) fn report_remove_reference(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["field", "place", "page", "visual", "bookmark"])?;
    let text = str_param(p, "field")?;
    let f = FieldName::parse(text)?;
    let place = str_param(p, "place")?.to_ascii_lowercase();
    let page_key = opt_str(p, "page")?;
    let visual_key = opt_str(p, "visual")?;
    let bookmark_key = opt_str(p, "bookmark")?;
    let (_, r) = parts(s)?;
    let mut at = Map::new();
    let strip = match place.as_str() {
        "drillthrough" => {
            if visual_key.is_some() || bookmark_key.is_some() {
                return Err("place drillthrough takes a page, not a visual or a bookmark".into());
            }
            let key = page_key.ok_or("place drillthrough needs `page`")?;
            let pi = page_index(r, key)?;
            let page = r.pages.get_mut(pi).ok_or("internal: page index out of range")?;
            at.insert("page".into(), json!(page.display_name));
            let (mut config, mut extra) = (page.filter_config.clone(), page.extra.clone());
            let st = strip_drill(&mut config, &mut extra, &f);
            if st.clean() {
                (page.filter_config, page.extra) = (config, extra);
            }
            st
        }
        "formatting" => {
            if bookmark_key.is_some() {
                return Err(
                    "place formatting takes a page and a visual, a page, or neither (the report)".into(),
                );
            }
            match (page_key, visual_key) {
                (None, Some(_)) => return Err("`visual` needs `page`".into()),
                (None, None) => {
                    let mut extra = r.extra.clone();
                    let st = strip_formatting(&mut extra, &f);
                    if st.clean() {
                        r.extra = extra;
                    }
                    st
                }
                (Some(pk), None) => {
                    let pi = page_index(r, pk)?;
                    let page = r.pages.get_mut(pi).ok_or("internal: page index out of range")?;
                    at.insert("page".into(), json!(page.display_name));
                    let mut extra = page.extra.clone();
                    let st = strip_formatting(&mut extra, &f);
                    if st.clean() {
                        page.extra = extra;
                    }
                    st
                }
                (Some(pk), Some(vk)) => {
                    let pi = page_index(r, pk)?;
                    let page = r.pages.get_mut(pi).ok_or("internal: page index out of range")?;
                    at.insert("page".into(), json!(page.display_name));
                    let vi = visual_index(page, vk)?;
                    let v = page
                        .visuals
                        .get_mut(vi)
                        .ok_or("internal: visual index out of range")?;
                    at.insert("visual".into(), json!(v.name));
                    let (mut a, mut b) = (v.visual_extra.clone(), v.extra.clone());
                    let mut st = strip_formatting(&mut a, &f);
                    let st2 = strip_formatting(&mut b, &f);
                    st.removed = st.removed.saturating_add(st2.removed);
                    st.remaining = st.remaining.saturating_add(st2.remaining);
                    if st.clean() {
                        (v.visual_extra, v.extra) = (a, b);
                    }
                    st
                }
            }
        }
        "bookmark" => {
            if page_key.is_some() || visual_key.is_some() {
                return Err("place bookmark takes a bookmark, not a page or a visual".into());
            }
            let key = bookmark_key.ok_or("place bookmark needs `bookmark`")?;
            let i = crate::bookmarks::bookmark_index(r, key)?;
            let b = r
                .bookmarks
                .get_mut(i)
                .ok_or("internal: bookmark index out of range")?;
            at.insert("bookmark".into(), json!(b.name));
            let mut doc = b.document.clone();
            let st = strip_bookmark(&mut doc, &f);
            if st.clean() {
                b.document = doc;
            }
            st
        }
        other => {
            return Err(format!(
                "place `{other}` must be drillthrough (a page's drillthrough or tooltip fields), \
                 formatting or bookmark"
            ));
        }
    };
    if strip.removed == 0 {
        return Err(format!("nothing at that place refers to {text}"));
    }
    if strip.remaining > 0 {
        return Err(format!(
            "{} reference(s) to {text} at that place are not in a part that can be removed alone; \
             nothing was changed (re-bind it with report.rebind, or edit it)",
            strip.remaining
        ));
    }
    s.report_dirty = true;
    Ok(json!({"field": text, "place": place, "at": at, "removed": strip.removed}))
}

// ---------------------------------------------------------------------------------------------
// report.rebind
// ---------------------------------------------------------------------------------------------

/// `report.rebind {from, to}`.
pub(crate) fn report_rebind(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["from", "to"])?;
    let from_text = str_param(p, "from")?;
    let to_text = str_param(p, "to")?;
    let (table, name) = match parse_field(from_text)? {
        ParsedField::Qualified { table, name } => (table, name),
        _ => {
            return Err(format!(
                "`from` must name the field as Table[Name], not `{from_text}`"
            ));
        }
    };
    let (model, r) = parts(s)?;
    let model = model.ok_or("report.rebind needs the report's model (none is open)")?;
    let to = crate::refs::resolve_field(Some(model), to_text)?.field;
    let (measure, to_table, to_name) = match &to {
        FieldRef::Column { table, column } => (false, table.clone(), column.clone()),
        FieldRef::Measure { table, measure } => (true, table.clone(), measure.clone()),
        _ => return Err(format!("`to` must be a column or a measure, not `{to_text}`")),
    };
    let present = if measure {
        model.measure(&name).is_some()
    } else {
        model.table(&table).is_some_and(|t| t.column(&name).is_some())
    };
    if present {
        return Err(format!(
            "{from_text} is in the model: report.rebind re-points references the model cannot \
             resolve; use visual.bind, or rename the field with update_report"
        ));
    }
    let change = ModelChange::Rebound {
        measure,
        table,
        name,
        to_table,
        to_name,
    };
    let mut next = r.clone();
    let (effects, _) = apply_change(&mut next, &change, true);
    let count = |k: &str| effects.get(k).and_then(Value::as_u64).unwrap_or(0);
    let found = count("affected_binding_count").saturating_add(count("other_reference_count"));
    let kind = if measure { "measure" } else { "column" };
    if found == 0 {
        return Err(format!(
            "the report has no {kind} reference to {from_text} (`to` is a {kind}, so only {kind} \
             references are re-bound)"
        ));
    }
    if let Some(n) = effects.get("not_updated") {
        return Err(format!("nothing was changed: {}", n.as_str().unwrap_or_default()));
    }
    *r = next;
    s.report_dirty = true;
    let mut out = json!({"from": from_text, "to": field_text(&to), "kind": kind});
    if let (Some(o), Value::Object(e)) = (out.as_object_mut(), effects) {
        for (k, v) in e {
            if k != "hint" && k != "updated" {
                o.insert(k, v);
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Stale queryRefs
// ---------------------------------------------------------------------------------------------

/// A projection whose `queryRef` names another field than the one it projects.
pub(crate) struct StaleRef {
    pub role: String,
    pub index: usize,
    pub query_ref: String,
    /// The field's own queryRef.
    pub expected: String,
    pub field: FieldRef,
    /// Whether `visual.fix_query_refs` renames it (the expected name is free in the visual).
    pub fixable: bool,
}

/// `T.C` inside an aggregation (`Sum(T.C)` → `T.C`), lower-cased.
fn ref_core(r: &str) -> String {
    let r = r.trim();
    let inner = match (r.find('('), r.ends_with(')')) {
        (Some(open), true) if open > 0 => r.get(open + 1..r.len() - 1).unwrap_or(r),
        _ => r,
    };
    inner.to_lowercase()
}

/// The stale projections of `v`'s stored query (case, an aggregation switch and a numeric
/// duplicate suffix are not stale).
pub(crate) fn stale_refs(v: &Visual) -> Vec<StaleRef> {
    let mut out: Vec<StaleRef> = Vec::new();
    let mut all: Vec<String> = Vec::new();
    let states = v
        .visual_extra
        .get("query")
        .and_then(|q| q.get("queryState"))
        .and_then(Value::as_object)
        .into_iter()
        .flatten();
    for (role, state) in states {
        for (index, p) in state
            .get("projections")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let Some(qr) = p.get("queryRef").and_then(Value::as_str) else {
                continue;
            };
            all.push(qr.to_string());
            let field = query::field_ref(p.get("field"));
            if matches!(field, FieldRef::Other { .. }) {
                continue;
            }
            let Some(expected) = query::default_query_ref(&field) else {
                continue;
            };
            let (got, want) = (ref_core(qr), ref_core(&expected));
            let suffixed = got
                .strip_prefix(&want)
                .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()));
            if got == want || suffixed {
                continue;
            }
            out.push(StaleRef {
                role: role.clone(),
                index,
                query_ref: qr.to_string(),
                expected,
                field,
                fixable: false,
            });
        }
    }
    let wanted: Vec<String> = out.iter().map(|x| x.expected.clone()).collect();
    for x in &mut out {
        let taken = all.contains(&x.expected);
        let shared = wanted.iter().filter(|w| **w == x.expected).count() > 1;
        x.fixable = !taken && !shared;
    }
    out
}

/// `visual.fix_query_refs {page, visual}`.
pub(crate) fn visual_fix_query_refs(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual"])?;
    let page_key = str_param(p, "page")?;
    let visual_key = str_param(p, "visual")?;
    let (_, r) = parts(s)?;
    let pi = page_index(r, page_key)?;
    let vi = {
        let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
        visual_index(page, visual_key)?
    };
    let before = crate::refs::snapshot_query_refs(r);
    let v = r
        .pages
        .get_mut(pi)
        .and_then(|pg| pg.visuals.get_mut(vi))
        .ok_or("internal: visual index out of range")?;
    let stale = stale_refs(v);
    if stale.is_empty() {
        return Err(format!("visual `{}` has no stale queryRef", v.name));
    }
    let mut renamed = Vec::new();
    let mut kept = Vec::new();
    for x in &stale {
        let slot = v
            .visual_extra
            .get_mut("query")
            .and_then(|q| q.get_mut("queryState"))
            .and_then(|q| q.get_mut(&x.role))
            .and_then(|q| q.get_mut("projections"))
            .and_then(|q| q.get_mut(x.index))
            .and_then(Value::as_object_mut);
        match slot {
            Some(proj) if x.fixable => {
                proj.insert("queryRef".into(), json!(x.expected));
                renamed.push(json!({"role": x.role, "field": field_text(&x.field),
                                    "from": x.query_ref, "to": x.expected}));
            }
            _ => kept.push(json!({"role": x.role, "query_ref": x.query_ref,
                                  "reason": format!("`{}` is already used in the visual", x.expected)})),
        }
    }
    if renamed.is_empty() {
        return Err(format!(
            "no stale queryRef of visual `{visual_key}` can be renamed: each one's own name is already \
             used by another projection"
        ));
    }
    let followed = crate::refs::follow_query_refs(r, &before);
    s.report_dirty = true;
    Ok(
        json!({"renamed": renamed, "kept": kept, "references_updated": followed.saturating_sub(renamed.len())}),
    )
}

// ---------------------------------------------------------------------------------------------
// Field parameters
// ---------------------------------------------------------------------------------------------

/// The table constructor rows of field parameter `table` and the span of the `NAMEOF` argument
/// naming `f` in each row: `(expression, rows' spans, row index, argument span)`.
fn param_row(
    m: &bicraft_model::SemanticModel,
    table: &str,
    f: &FieldName,
) -> Result<(String, Vec<bicraft_dax::Span>, usize, bicraft_dax::Span), String> {
    let t = m
        .table(table)
        .ok_or_else(|| format!("the model has no table `{table}`"))?;
    if !t.columns.iter().any(crate::parameters::is_fields_column) {
        return Err(format!("table `{}` is not a field parameter", t.name));
    }
    let expr = t
        .partitions
        .iter()
        .find(|p| p.source_type.eq_ignore_ascii_case("calculated"))
        .and_then(|p| p.expression.as_deref())
        .ok_or_else(|| format!("field parameter `{}` has no calculated expression", t.name))?;
    let (Some(Expr::TableCtor { rows, .. }), _) = bicraft_dax::parse_expression(expr) else {
        return Err(format!(
            "field parameter `{}` is not a table of tuples {{(\"Label\", NAMEOF(field), order), …}}",
            t.name
        ));
    };
    let spans: Vec<bicraft_dax::Span> = rows.iter().map(Expr::span).collect();
    for (i, row) in rows.iter().enumerate() {
        let Expr::Tuple { items, .. } = row else {
            continue;
        };
        for item in items {
            if let Expr::Call { name, args, .. } = item
                && name.eq_ignore_ascii_case("NAMEOF")
                && let Some(Some(Expr::Column {
                    table: tb,
                    name: n,
                    span,
                    ..
                })) = args.first()
                && n.eq_ignore_ascii_case(&f.name)
                && match (&f.table, tb) {
                    (None, _) => true,
                    (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                    (Some(_), None) => false,
                }
            {
                return Ok((expr.to_string(), spans, i, *span));
            }
        }
    }
    Err(format!("field parameter `{}` does not list that field", t.name))
}

fn dax_field(table: &str, name: &str) -> String {
    format!("'{}'[{}]", table.replace('\'', "''"), name.replace(']', "]]"))
}

/// `parameter.fix_field {name, field, to?}`.
pub(crate) fn parameter_fix_field(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "field", "to"])?;
    let table = str_param(p, "name")?.to_string();
    let field = str_param(p, "field")?;
    let f = FieldName::parse(field)?;
    let to = opt_str(p, "to")?;
    let model = crate::commands::model(s)?;
    let (expr, spans, row, arg) = param_row(model, &table, &f)?;
    let cut = |sp: bicraft_dax::Span| expr.get(sp.start..sp.end).ok_or("internal: span out of range");
    let (new_expr, action) = match to {
        Some(to) => {
            let target = crate::refs::resolve_field(Some(model), to)?.field;
            let text = match &target {
                FieldRef::Column { table, column } => dax_field(table, column),
                FieldRef::Measure { table, measure } => dax_field(table, measure),
                _ => return Err(format!("`to` must be a column or a measure, not `{to}`")),
            };
            let head = expr.get(..arg.start).ok_or("internal: span out of range")?;
            let tail = expr.get(arg.end..).ok_or("internal: span out of range")?;
            (format!("{head}{text}{tail}"), json!({"rebound": text}))
        }
        None => {
            if spans.len() <= 1 {
                return Err(format!(
                    "{field} is the only field of `{table}`: give `to`, or remove the table (table.remove)"
                ));
            }
            let kept: Vec<&str> = spans
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != row)
                .map(|(_, sp)| cut(*sp))
                .collect::<Result<_, _>>()?;
            (
                format!("{{\n    {}\n}}", kept.join(",\n    ")),
                json!({"removed": row}),
            )
        }
    };
    let expression = new_expr.clone();
    let name = table.clone();
    apply(s, &json!({}), move |m| {
        let t = m
            .table_mut(&name)
            .ok_or_else(|| format!("the model has no table `{name}`"))?;
        let part = t
            .partitions
            .iter_mut()
            .find(|p| p.source_type.eq_ignore_ascii_case("calculated"))
            .ok_or("internal: the calculated partition went")?;
        part.expression = Some(expression.clone());
        let mut edit =
            Edit::new(json!({"table": t.name, "field": field, "expression": expression, "change": action}));
        edit.check(format!("calculated table '{}'", t.name), &expression, None);
        Ok(edit)
    })
}
