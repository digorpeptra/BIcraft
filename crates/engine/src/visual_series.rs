//! Per-series and per-category formatting (M2.5 c): the `series` and `colors` keys of the visual
//! `props` object, the `visual.series` command, and the selector-entry editing that conditional
//! formatting ([`crate::visual_conditional`]) shares.
//!
//! # Targets
//!
//! A *target* says which part of the data an entry formats. It becomes the entry's `selector`:
//!
//! - `"[Total]"`, `"Sales[Amount]"`, `"Sum(Sales[Amount])"`: a field bound to the visual (a
//!   measure series, a table column). Written `{"metadata": "<queryRef>"}`, the projection's
//!   `queryRef` (`Sales.Total`).
//! - `"Product[Name]=Bikes"`: one value of a column (a category or legend value). Written
//!   `{"data": [{"scopeId": {"Comparison": {"ComparisonKind": 0, "Left": <column>, "Right":
//!   <literal>}}}]}`. The value is typed by the column (`'Bikes'`, `3L`, `2.5D`,
//!   `datetime'2024-01-31T00:00:00'`). `{field, value}` says the same with a JSON value (numbers,
//!   `null` for blank, or text containing `=` at the start).
//!
//! # The `series` and `colors` keys
//!
//! - `colors`: `{"<target>": "#hex" | null}`, a shorthand for data colors (`dataPoint.fill`).
//!   On a bar-type chart whose only series is the target ([`sole_series`]), the color is the
//!   chart's default color (the `dataPoint` entry without a selector): Desktop ignores a
//!   per-measure color there. [`extract`] gives that default color back as the series' `color`.
//! - `series`: `[{target, color?, labels?, line?, objects?, clear?}]`:
//!   - `color`: `"#hex"` (`dataPoint.fill`), `null` removes it;
//!   - `labels`: `bool` | `{show?, color?, font_size?, position?, display_units?, precision?}`
//!     (`labels`), `null` removes the series' label formatting;
//!   - `line`: `{show?, width?, style?: solid|dashed|dotted, show_marker?, marker_shape?,
//!     marker_size?}` (`lineStyles`), `null` removes it;
//!   - `objects`: `[{object, property, value}]`, any other property for that target (values as in
//!     `format.objects`; `null` removes);
//!   - `clear: true`: first removes every entry with that target's selector.
//!
//! Both are patches: properties not named stay as they are. Values are compared by meaning, so
//! re-applying what [`extract`] returns changes nothing. Entries whose selector is neither of the
//! two shapes above (`id`, roles, wildcards) are left alone and not extracted.

use bicraft_model::SemanticModel;
use bicraft_report::{FieldRef, Visual};
use bicraft_visuals::PropType;
use serde_json::{Map, Value, json};

use crate::refs::{field_text, parse_field, resolve_field, same_field};
use crate::visual_props::{
    Lit, VISUAL_ORDER, ValueKind, check_ident, check_keys, check_text, color_text, decode_literal,
    field_from_json, field_to_json, insert_ordered, is_color, num_json, num_text, parse_lit, parsed_matches,
    quote, to_lit, value_kind,
};

/// Most `series` items, and most `colors` entries.
const MAX_SERIES: usize = 500;
/// Most `objects` edits of one series item.
const MAX_ITEM_OBJECTS: usize = 50;
/// Deepest JSON compared by [`same_json`].
const MAX_DEPTH: usize = 256;

// ---------------------------------------------------------------------------------------------
// Selector-entry edits (shared with conditional formatting)
// ---------------------------------------------------------------------------------------------

/// What the catalog vets before a selector-entry value is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Check {
    /// Nothing (friendly targets bicraft knows).
    None,
    /// The object takes selectors, and the value fits the property's type.
    Value,
    /// The property allows conditional formatting.
    Conditional,
}

/// One edit of `visual.objects` entries that carry a selector.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum EntryOp {
    /// Sets `prop` in the entry of `object` whose selector is one of `selectors` and already has
    /// the property, else the entry with `selectors[0]` (created when missing).
    Set {
        object: String,
        selectors: Vec<Value>,
        prop: String,
        value: Value,
        /// What the change log says (`[Total]: dataPoint.fill = #118DFF`).
        label: String,
        /// What the catalog checks before the value is written.
        check: Check,
    },
    /// Removes `prop` (or, when `None`, the whole entry) from the entries of `object` whose
    /// selector is one of `selectors`.
    Remove {
        object: String,
        selectors: Vec<Value>,
        prop: Option<String>,
        label: String,
    },
    /// Removes every entry, of any object, whose selector is one of `selectors`.
    Clear { selectors: Vec<Value>, label: String },
}

/// JSON equality by meaning: literals (`"Value"` strings) compare as query literals (`12L` =
/// `12D`, colors case-insensitively), numbers by value, object keys in any order.
pub(crate) fn same_json(a: &Value, b: &Value) -> bool {
    same_json_at(a, b, 0)
}

fn same_json_at(a: &Value, b: &Value, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return a == b;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter().all(|(k, va)| {
                    y.get(k)
                        .is_some_and(|vb| match (k.as_str(), va.as_str(), vb.as_str()) {
                            ("Value", Some(sa), Some(sb)) => same_literal(sa, sb),
                            _ => same_json_at(va, vb, depth + 1),
                        })
                })
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same_json_at(p, q, depth + 1))
        }
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

fn same_literal(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (parse_lit(a), parse_lit(b)) {
        (Lit::Text(x), Lit::Text(y)) if is_color(&x) && is_color(&y) => x.eq_ignore_ascii_case(&y),
        (x, y) => x.same(&y),
    }
}

/// Whether `entry`'s selector is one of `selectors`. `Value::Null` in `selectors` stands for the
/// entry without a selector (the visual's default, e.g. a single-series chart's color).
fn selector_in(entry: &Value, selectors: &[Value]) -> bool {
    match entry.get("selector") {
        Some(s) if !s.is_null() => selectors.iter().any(|x| !x.is_null() && same_json(s, x)),
        _ => selectors.iter().any(Value::is_null),
    }
}

/// Checks a value about to be written against the catalog (when it knows the object and
/// property; unknown ones are allowed).
fn check_catalog(
    check: Check,
    visual_type: Option<&str>,
    object: &str,
    prop: &str,
    value: &Value,
) -> Result<(), String> {
    // Custom visuals and unknown types are not checked.
    let Some(vt) = visual_type.filter(|t| bicraft_visuals::visual(t).is_some()) else {
        return Ok(());
    };
    match check {
        Check::None => Ok(()),
        Check::Value => {
            crate::visual_catalog::check_entry_at(Some(vt), Some(false), object, prop, value)?;
            match bicraft_visuals::object_in(vt, object, false) {
                Some(o) if !o.selectors => Err(format!(
                    "`{object}` of a {vt} cannot be formatted per series or per value"
                )),
                _ => Ok(()),
            }
        }
        Check::Conditional => {
            let Some(o) = bicraft_visuals::object_in(vt, object, false) else {
                let hint = bicraft_visuals::suggest(
                    object,
                    bicraft_visuals::visual(vt)
                        .into_iter()
                        .flat_map(|v| v.objects.iter().filter(|o| !o.container).map(|o| o.name.as_str())),
                )
                .map(|h| format!("; did you mean `{h}`?"))
                .unwrap_or_default();
                return Err(format!("`{vt}` has no visual object `{object}`{hint}"));
            };
            match o.properties.iter().find(|p| p.name == prop) {
                None => {
                    let hint = bicraft_visuals::suggest(prop, o.properties.iter().map(|p| p.name.as_str()))
                        .map(|h| format!("; did you mean `{h}`?"))
                        .unwrap_or_default();
                    Err(format!(
                        "`{vt}` visual object `{object}` has no property `{prop}`{hint}"
                    ))
                }
                Some(p) if !p.conditional => Err(format!(
                    "{object}.{prop} of a {vt} does not take conditional formatting"
                )),
                Some(_) => Ok(()),
            }
        }
    }
}

/// Applies selector-entry edits to `v`, recording what changed. `check` asks for the catalog to
/// vet each value that actually changes.
pub(crate) fn apply_ops(
    v: &mut Visual,
    ops: &[EntryOp],
    check: bool,
    changes: &mut Vec<String>,
) -> Result<(), String> {
    apply_ops_in(v, "objects", ops, check, changes)
}

/// [`apply_ops`] on the entries under `root`: `objects`, or `visualContainerObjects` (the
/// container's title, background, border), whose values the catalog does not vet here.
pub(crate) fn apply_ops_in(
    v: &mut Visual,
    root: &str,
    ops: &[EntryOp],
    check: bool,
    changes: &mut Vec<String>,
) -> Result<(), String> {
    let visual_type = v.visual_type.clone();
    let check = check && root == "objects";
    for op in ops {
        let setting = matches!(op, EntryOp::Set { .. });
        if !v.visual_extra.contains_key(root) {
            if !setting {
                continue;
            }
            insert_ordered(&mut v.visual_extra, root, json!({}), VISUAL_ORDER);
        }
        let objects = v
            .visual_extra
            .get_mut(root)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| format!("the visual's `{root}` is not an object; it cannot be edited"))?;
        match op {
            EntryOp::Set {
                object,
                selectors,
                prop,
                value,
                label,
                check: what,
            } => {
                let primary = selectors.first().ok_or("internal: an edit without selector")?;
                if !objects.contains_key(object) {
                    objects.insert(object.clone(), json!([]));
                }
                let arr = objects
                    .get_mut(object)
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| {
                        format!("the visual's `objects.{object}` is not a list; it cannot be edited")
                    })?;
                let holding = arr.iter().position(|e| {
                    selector_in(e, selectors) && e.get("properties").and_then(|p| p.get(prop)).is_some()
                });
                let idx = match holding.or_else(|| {
                    arr.iter()
                        .position(|e| selector_in(e, std::slice::from_ref(primary)))
                }) {
                    Some(i) => i,
                    // The default entry (no selector) goes first, as Desktop writes it.
                    None if primary.is_null() => {
                        arr.insert(0, json!({"properties": {}}));
                        0
                    }
                    None => {
                        arr.push(json!({"properties": {}, "selector": primary}));
                        arr.len().saturating_sub(1)
                    }
                };
                let entry = arr
                    .get_mut(idx)
                    .and_then(Value::as_object_mut)
                    .ok_or_else(|| format!("an entry of `objects.{object}` is not an object"))?;
                if !entry.contains_key("properties") {
                    entry.shift_insert(0, "properties".into(), json!({}));
                }
                let props = entry
                    .get_mut("properties")
                    .and_then(Value::as_object_mut)
                    .ok_or_else(|| format!("the properties of `objects.{object}` are not an object"))?;
                if props.get(prop).is_some_and(|old| same_json(old, value)) {
                    continue;
                }
                if check {
                    check_catalog(*what, visual_type.as_deref(), object, prop, value)?;
                }
                props.insert(prop.clone(), value.clone());
                changes.push(label.clone());
            }
            EntryOp::Remove {
                object,
                selectors,
                prop,
                label,
            } => {
                let Some(arr) = objects.get_mut(object).and_then(Value::as_array_mut) else {
                    continue;
                };
                let mut changed = false;
                match prop {
                    None => {
                        let before = arr.len();
                        arr.retain(|e| !selector_in(e, selectors));
                        changed = arr.len() != before;
                    }
                    Some(p) => {
                        for e in arr.iter_mut() {
                            if !selector_in(e, selectors) {
                                continue;
                            }
                            if let Some(props) = e.get_mut("properties").and_then(Value::as_object_mut)
                                && props.shift_remove(p).is_some()
                            {
                                changed = true;
                            }
                        }
                        // An entry left with nothing but its selector means nothing.
                        arr.retain(|e| {
                            !(selector_in(e, selectors)
                                && e.as_object()
                                    .is_some_and(|o| o.keys().all(|k| k == "properties" || k == "selector"))
                                && e.get("properties")
                                    .and_then(Value::as_object)
                                    .is_some_and(Map::is_empty))
                        });
                    }
                }
                if arr.is_empty() {
                    objects.shift_remove(object);
                }
                if changed {
                    changes.push(label.clone());
                }
            }
            EntryOp::Clear { selectors, label } => {
                let mut changed = false;
                let names: Vec<String> = objects.keys().cloned().collect();
                for name in names {
                    let Some(arr) = objects.get_mut(&name).and_then(Value::as_array_mut) else {
                        continue;
                    };
                    let before = arr.len();
                    arr.retain(|e| !selector_in(e, selectors));
                    changed |= arr.len() != before;
                    if arr.is_empty() {
                        objects.shift_remove(&name);
                    }
                }
                if changed {
                    changes.push(label.clone());
                }
            }
        }
        if objects.is_empty() {
            v.visual_extra.shift_remove(root);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Fields and targets
// ---------------------------------------------------------------------------------------------

/// The visual's projections: `(field, queryRef)`, in role then projection order.
pub(crate) fn projections(v: &Visual) -> Vec<(FieldRef, String)> {
    let mut out = Vec::new();
    let query = bicraft_report::query::effective(v);
    let Some(state) = query
        .as_ref()
        .and_then(|q| q.get("queryState"))
        .and_then(Value::as_object)
    else {
        return out;
    };
    for role in state.values() {
        for p in role
            .get("projections")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(r) = p.get("queryRef").and_then(Value::as_str) {
                out.push((bicraft_report::query::field_ref(p.get("field")), r.to_string()));
            }
        }
    }
    out
}

/// What fields are resolved against.
pub(crate) struct Ctx<'a> {
    pub model: Option<&'a SemanticModel>,
    pub projections: Vec<(FieldRef, String)>,
    pub visual_type: Option<String>,
}

impl<'a> Ctx<'a> {
    pub(crate) fn new(v: &Visual, model: Option<&'a SemanticModel>) -> Self {
        Self {
            model,
            projections: projections(v),
            visual_type: v.visual_type.clone(),
        }
    }

    /// A field string: a bound field first (whatever the model says), else the model's.
    pub(crate) fn field(&self, text: &str) -> Result<FieldRef, String> {
        let parsed = parse_field(text)?;
        if let Some((f, _)) = self.projections.iter().find(|(f, _)| parsed_matches(&parsed, f)) {
            if let Some(m) = self.model
                && let Ok(r) = resolve_field(Some(m), text)
                && same_field(&r.field, f)
            {
                return Ok(r.field);
            }
            return Ok(f.clone());
        }
        resolve_field(self.model, text).map(|r| r.field)
    }

    /// A field bound to the visual and its `queryRef`.
    pub(crate) fn bound(&self, text: &str) -> Result<(FieldRef, String), String> {
        let parsed = parse_field(text)?;
        if let Some((f, r)) = self.projections.iter().find(|(f, _)| parsed_matches(&parsed, f)) {
            return Ok((f.clone(), r.clone()));
        }
        // Say why: unknown field, or known but not bound.
        resolve_field(self.model, text)?;
        let bound: Vec<String> = self
            .projections
            .iter()
            .filter(|(f, _)| !matches!(f, FieldRef::Other { .. }))
            .map(|(f, _)| field_text(f))
            .collect();
        Err(format!(
            "{text} is not bound to the visual (bound: {})",
            if bound.is_empty() {
                "nothing".to_string()
            } else {
                bound.join(", ")
            }
        ))
    }

    /// The `queryRef` of the first projection of `f`, when `f` is bound.
    pub(crate) fn query_ref(&self, f: &FieldRef) -> Option<&str> {
        self.projections
            .iter()
            .find(|(p, _)| same_field(p, f))
            .map(|(_, r)| r.as_str())
    }

    /// The bound field whose `queryRef` is `r`, when `r` is the first projection of that field (so
    /// that resolving the field's text gives `r` back).
    pub(crate) fn field_of_ref(&self, r: &str) -> Option<&FieldRef> {
        let (f, _) = self.projections.iter().find(|(_, q)| q == r)?;
        if matches!(f, FieldRef::Other { .. }) || self.query_ref(f) != Some(r) {
            return None;
        }
        Some(f)
    }
}

/// Charts that color a single series of bars (columns, funnel steps, ribbons) with their default
/// color (the `dataPoint` entry without a selector) and ignore a per-measure (`metadata`) color
/// there (Power BI Desktop 2.158). Line and area charts do the opposite: they use the
/// per-measure color, so they are not listed.
const SINGLE_SERIES_CHARTS: &[&str] = &[
    "barChart",
    "clusteredBarChart",
    "clusteredColumnChart",
    "columnChart",
    "funnel",
    "hundredPercentStackedBarChart",
    "hundredPercentStackedColumnChart",
    "lineClusteredColumnComboChart",
    "lineStackedColumnComboChart",
    "ribbonChart",
];

/// The `queryRef` of the only series of a chart that shows one: a chart of
/// [`SINGLE_SERIES_CHARTS`] with exactly one field in `Y` (the bars; nothing in a combo chart's
/// line values `Y2`) and nothing in its legend (`Series`).
pub(crate) fn sole_series(v: &Visual) -> Option<String> {
    let vt = v.visual_type.as_deref()?;
    if !SINGLE_SERIES_CHARTS.contains(&vt) {
        return None;
    }
    let query = bicraft_report::query::effective(v)?;
    let state = query.get("queryState")?.as_object()?;
    let projections = |role: &str| {
        state
            .get(role)
            .and_then(|r| r.get("projections"))
            .and_then(Value::as_array)
    };
    let count = |role: &str| projections(role).map_or(0, Vec::len);
    if count("Series") > 0 || count("Y2") > 0 || count("Y") != 1 {
        return None;
    }
    projections("Y")?
        .first()?
        .get("queryRef")?
        .as_str()
        .map(str::to_string)
}

/// The selectors a color of `target` is written with: the default entry (no selector) when the
/// target is the only series of a chart ([`sole_series`]), else the target's own selector.
fn color_selectors(target: &SeriesTarget, sole: Option<&str>) -> Result<Vec<Value>, String> {
    match target {
        SeriesTarget::Field { query_ref, .. } if Some(query_ref.as_str()) == sole => Ok(vec![Value::Null]),
        _ => Ok(vec![target.selector()?]),
    }
}

/// A series target.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SeriesTarget {
    /// A bound field (`{"metadata": queryRef}`).
    Field { field: FieldRef, query_ref: String },
    /// One value of a column (`{"data": [{"scopeId": …}]}`).
    Value { field: FieldRef, lit: Lit },
}

impl SeriesTarget {
    pub(crate) fn selector(&self) -> Result<Value, String> {
        match self {
            Self::Field { query_ref, .. } => Ok(json!({"metadata": query_ref})),
            Self::Value { field, lit } => {
                let left = field_to_json(field, None).ok_or("internal: target field cannot be written")?;
                Ok(json!({"data": [{"scopeId": {"Comparison": {
                    "ComparisonKind": 0,
                    "Left": left,
                    "Right": lit.expr(),
                }}}]}))
            }
        }
    }

    /// The target as `props` writes it.
    pub(crate) fn to_json(&self) -> Value {
        match self {
            Self::Field { field, .. } => json!(field_text(field)),
            Self::Value {
                field,
                lit: Lit::Text(t),
            } => json!(format!("{}={t}", field_text(field))),
            Self::Value { field, lit } => json!({"field": field_text(field), "value": lit.to_json()}),
        }
    }

    pub(crate) fn label(&self) -> String {
        match self {
            Self::Field { field, .. } => field_text(field),
            Self::Value { field, lit } => format!("{}={}", field_text(field), lit.render()),
        }
    }
}

/// `Table[Column]=value`: the column and the value text, when `text` reads so.
fn split_value_target(text: &str) -> Option<(&str, &str)> {
    for (i, _) in text.match_indices('=') {
        let (head, tail) = (text.get(..i)?, text.get(i + 1..)?);
        if head.trim_end().ends_with(']') && parse_field(head.trim()).is_ok() {
            return Some((head.trim(), tail));
        }
    }
    None
}

/// A column value from its text, typed by the column.
fn value_from_text(text: &str, kind: ValueKind, what: &str) -> Result<Lit, String> {
    let json = match kind {
        ValueKind::Int | ValueKind::Double | ValueKind::Decimal => match text.trim().parse::<f64>() {
            Ok(n) if n.is_finite() => json!(n),
            _ => {
                return Err(format!(
                    "{what}: `{text}` is not a number (the column is numeric)"
                ));
            }
        },
        ValueKind::Bool => match text.trim().to_ascii_lowercase().as_str() {
            "true" => json!(true),
            "false" => json!(false),
            _ => return Err(format!("{what}: `{text}` is not true or false")),
        },
        _ => json!(text),
    };
    to_lit(&json, kind, what)
}

fn column_value_target(
    ctx: &Ctx<'_>,
    field_text_in: &str,
    value: &Value,
    from_text: bool,
    what: &str,
) -> Result<SeriesTarget, String> {
    let field = ctx.field(field_text_in).map_err(|e| format!("{what}: {e}"))?;
    if !matches!(field, FieldRef::Column { .. }) {
        return Err(format!(
            "{what}: a value target names a column (Product[Name]=Bikes), not {}",
            field_text(&field)
        ));
    }
    let kind = value_kind(ctx.model, &field);
    let lit = match (from_text, value) {
        (true, Value::String(s)) => value_from_text(s, kind, what)?,
        _ => to_lit(value, kind, what)?,
    };
    Ok(SeriesTarget::Value { field, lit })
}

/// A target: `"[Total]"`, `"Product[Name]=Bikes"`, or `{field, value}`.
pub(crate) fn parse_target(t: &Value, ctx: &Ctx<'_>, what: &str) -> Result<SeriesTarget, String> {
    match t {
        Value::String(s) => {
            check_text(s, what)?;
            if let Some((head, tail)) = split_value_target(s) {
                return column_value_target(ctx, head, &json!(tail), true, what);
            }
            let (field, query_ref) = ctx.bound(s).map_err(|e| format!("{what}: {e}"))?;
            Ok(SeriesTarget::Field { field, query_ref })
        }
        Value::Object(o) => {
            check_keys(o, &["field", "value"], what)?;
            let f = o
                .get("field")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{what}.field must be a column like Product[Name]"))?;
            let value = o
                .get("value")
                .ok_or_else(|| format!("{what}.value is missing (null means blank)"))?;
            column_value_target(ctx, f, value, false, what)
        }
        _ => Err(format!(
            "{what} must be a field (\"[Total]\"), a column value (\"Product[Name]=Bikes\") or {{field, value}}"
        )),
    }
}

/// The target a selector names, when it is one of the two shapes [`SeriesTarget`] writes.
pub(crate) fn decode_selector(sel: &Value, ctx: &Ctx<'_>) -> Option<SeriesTarget> {
    let o = sel.as_object()?;
    if o.len() != 1 {
        return None;
    }
    if let Some(r) = o.get("metadata") {
        let r = r.as_str()?;
        let field = ctx.field_of_ref(r)?.clone();
        return Some(SeriesTarget::Field {
            field,
            query_ref: r.to_string(),
        });
    }
    let data = o.get("data")?.as_array()?;
    if data.len() != 1 {
        return None;
    }
    let d = data.first()?.as_object()?;
    if d.len() != 1 {
        return None;
    }
    let cmp = d.get("scopeId")?.as_object()?;
    if cmp.len() != 1 {
        return None;
    }
    let c = cmp.get("Comparison")?.as_object()?;
    if c.len() != 3 || c.get("ComparisonKind")?.as_u64()? != 0 {
        return None;
    }
    let field = field_from_json(c.get("Left")?, &Vec::new(), None)?;
    if !matches!(field, FieldRef::Column { .. }) {
        return None;
    }
    let right = c.get("Right")?.as_object()?;
    if right.len() != 1 {
        return None;
    }
    let l = right.get("Literal")?.as_object()?;
    if l.len() != 1 {
        return None;
    }
    let lit = parse_lit(l.get("Value")?.as_str()?);
    if matches!(lit, Lit::Other(_)) {
        return None;
    }
    Some(SeriesTarget::Value { field, lit })
}

// ---------------------------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------------------------

/// How a friendly series property is read.
#[derive(Debug, Clone, Copy)]
enum SKind {
    Bool,
    Num(f64, f64),
    Color,
    Text,
    Enum(&'static [&'static str]),
}

struct SGroup {
    key: &'static str,
    object: &'static str,
    /// (friendly name, PBIR property, kind)
    props: &'static [(&'static str, &'static str, SKind)],
}

const LINE_STYLES: &[&str] = &["solid", "dashed", "dotted", "custom"];
const MARKER_SHAPES: &[&str] = &[
    "circle",
    "square",
    "diamond",
    "triangle",
    "x",
    "shortDash",
    "longDash",
    "plus",
];

const LABELS: SGroup = SGroup {
    key: "labels",
    object: "labels",
    props: &[
        ("show", "show", SKind::Bool),
        ("color", "color", SKind::Color),
        ("font_size", "fontSize", SKind::Num(1.0, 400.0)),
        ("position", "labelPosition", SKind::Text),
        ("display_units", "labelDisplayUnits", SKind::Num(0.0, 1.0e12)),
        ("precision", "labelPrecision", SKind::Num(0.0, 15.0)),
    ],
};

const LINE: SGroup = SGroup {
    key: "line",
    object: "lineStyles",
    props: &[
        ("show", "strokeShow", SKind::Bool),
        ("width", "strokeWidth", SKind::Num(0.0, 100.0)),
        ("style", "lineStyle", SKind::Enum(LINE_STYLES)),
        ("show_marker", "showMarker", SKind::Bool),
        ("marker_shape", "markerShape", SKind::Enum(MARKER_SHAPES)),
        ("marker_size", "markerSize", SKind::Num(0.0, 100.0)),
    ],
};

const GROUPS: &[&SGroup] = &[&LABELS, &LINE];

pub(crate) fn literal(text: String) -> Value {
    json!({"expr": {"Literal": {"Value": text}}})
}

pub(crate) fn color_value(c: &str) -> Value {
    json!({"solid": {"color": literal(quote(c))}})
}

/// Whether the catalog says the property is an integer (written `3L`).
fn integer_prop(visual_type: Option<&str>, object: &str, prop: &str) -> bool {
    visual_type
        .and_then(|vt| bicraft_visuals::property(vt, object, prop))
        .is_some_and(|p| matches!(p.ty, PropType::Number { integer: true, .. }))
}

pub(crate) fn number_value(n: f64, integer: bool) -> Value {
    if integer && n.fract() == 0.0 {
        literal(format!("{}L", num_text(n)))
    } else {
        literal(format!("{}D", num_text(n)))
    }
}

/// A rendered value and how the change log shows it.
fn kind_value(v: &Value, kind: SKind, what: &str, integer: bool) -> Result<(Value, String), String> {
    match kind {
        SKind::Bool => v
            .as_bool()
            .map(|b| (literal(b.to_string()), b.to_string()))
            .ok_or_else(|| format!("{what} must be true, false or null")),
        SKind::Num(lo, hi) => match v.as_f64() {
            Some(n) if n.is_finite() && (lo..=hi).contains(&n) => Ok((number_value(n, integer), num_text(n))),
            _ => Err(format!("{what} must be a number from {lo} to {hi}, or null")),
        },
        SKind::Color => match v.as_str() {
            Some(s) if is_color(s) => Ok((color_value(s), s.to_string())),
            _ => Err(format!(
                "{what} must be a color like #1F77B4 (#RGB, #RRGGBB or #RRGGBBAA), or null"
            )),
        },
        SKind::Text => match v.as_str() {
            Some(s) => {
                check_text(s, what)?;
                Ok((literal(quote(s)), quote(s)))
            }
            None => Err(format!("{what} must be a string or null")),
        },
        SKind::Enum(allowed) => {
            let s = v.as_str().unwrap_or_default();
            allowed
                .iter()
                .find(|a| a.eq_ignore_ascii_case(s))
                .map(|a| (literal(quote(a)), (*a).to_string()))
                .ok_or_else(|| format!("{what} must be one of {}, or null", allowed.join(", ")))
        }
    }
}

/// The friendly JSON of a PBIR value of `kind`.
fn decode_kind(v: &Value, kind: SKind) -> Option<Value> {
    match kind {
        SKind::Bool => match decode_literal(v)? {
            Lit::Bool(b) => Some(json!(b)),
            _ => None,
        },
        SKind::Num(lo, hi) => match decode_literal(v)? {
            Lit::Num(n, _) if (lo..=hi).contains(&n) => Some(num_json(n)),
            _ => None,
        },
        SKind::Color => color_text(v).filter(|c| is_color(c)).map(Value::String),
        SKind::Text => match decode_literal(v)? {
            Lit::Text(s) if s.chars().count() <= 4000 => Some(Value::String(s)),
            _ => None,
        },
        SKind::Enum(allowed) => match decode_literal(v)? {
            Lit::Text(s) if allowed.iter().any(|a| a.eq_ignore_ascii_case(&s)) => Some(Value::String(s)),
            _ => None,
        },
    }
}

/// A generic `objects` value: bool, number, string, `{color}`, `{literal}`, or any other JSON
/// object kept verbatim (a PBIR value). `None` is `null` (remove).
fn generic_value(v: &Value, what: &str, integer: bool) -> Result<Option<(Value, String)>, String> {
    Ok(Some(match v {
        Value::Null => return Ok(None),
        Value::Bool(b) => (literal(b.to_string()), b.to_string()),
        Value::Number(n) => match n.as_f64() {
            Some(x) if x.is_finite() => (number_value(x, integer), num_text(x)),
            _ => return Err(format!("{what} must be a finite number")),
        },
        Value::String(s) => {
            check_text(s, what)?;
            (literal(quote(s)), quote(s))
        }
        Value::Object(o) if o.len() == 1 && o.contains_key("color") => {
            let c = o.get("color").and_then(Value::as_str).unwrap_or_default();
            if !is_color(c) {
                return Err(format!("{what}.color must be a color like #1F77B4"));
            }
            (color_value(c), c.to_string())
        }
        Value::Object(o) if o.len() == 1 && o.contains_key("literal") => {
            let s = o
                .get("literal")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("{what}.literal must be a literal like 12L or 'text'"))?;
            check_text(s, what)?;
            (literal(s.to_string()), s.to_string())
        }
        Value::Object(_) => (v.clone(), "(PBIR value)".to_string()),
        _ => {
            return Err(format!(
                "{what} must be a boolean, number, string, {{color}}, {{literal}}, a PBIR value or null"
            ));
        }
    }))
}

/// The friendly form of a generic PBIR value (inverse of [`generic_value`]).
fn decode_generic(v: &Value) -> Value {
    if let Some(c) = color_text(v).filter(|c| is_color(c)) {
        return json!({ "color": c });
    }
    match decode_literal(v) {
        Some(Lit::Bool(b)) => json!(b),
        Some(Lit::Num(n, _)) => num_json(n),
        Some(Lit::Text(s)) if s.chars().count() <= 4000 => json!(s),
        _ => v.clone(),
    }
}

// ---------------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------------

fn group_ops(
    g: &SGroup,
    v: &Value,
    selectors: &[Value],
    target: &str,
    ctx: &Ctx<'_>,
    what: &str,
    ops: &mut Vec<EntryOp>,
) -> Result<(), String> {
    let set = |prop: &str, value: Value, desc: String| EntryOp::Set {
        object: g.object.to_string(),
        selectors: selectors.to_vec(),
        prop: prop.to_string(),
        value,
        label: format!("{target}: {}.{prop} = {desc}", g.object),
        check: Check::Value,
    };
    match v {
        Value::Null => ops.push(EntryOp::Remove {
            object: g.object.to_string(),
            selectors: selectors.to_vec(),
            prop: None,
            label: format!("{target}: {} removed", g.object),
        }),
        Value::Bool(b) if g.props.iter().any(|(f, _, _)| *f == "show") => {
            let (_, prop, _) = g
                .props
                .iter()
                .find(|(f, _, _)| *f == "show")
                .ok_or("internal: no show property")?;
            ops.push(set(prop, literal(b.to_string()), b.to_string()));
        }
        Value::Object(o) => {
            let names: Vec<&str> = g.props.iter().map(|(f, _, _)| *f).collect();
            check_keys(o, &names, what)?;
            for (k, x) in o {
                let (_, prop, kind) = g
                    .props
                    .iter()
                    .find(|(f, _, _)| f == k)
                    .ok_or("internal: unknown series key")?;
                if x.is_null() {
                    ops.push(EntryOp::Remove {
                        object: g.object.to_string(),
                        selectors: selectors.to_vec(),
                        prop: Some((*prop).to_string()),
                        label: format!("{target}: {}.{prop} removed", g.object),
                    });
                } else {
                    let integer = integer_prop(ctx.visual_type.as_deref(), g.object, prop);
                    let (value, desc) = kind_value(x, *kind, &format!("{what}.{k}"), integer)?;
                    ops.push(set(prop, value, desc));
                }
            }
        }
        _ => return Err(format!("{what} must be an object or null")),
    }
    Ok(())
}

fn object_ops(
    items: &Value,
    selectors: &[Value],
    target: &str,
    ctx: &Ctx<'_>,
    what: &str,
    ops: &mut Vec<EntryOp>,
) -> Result<(), String> {
    let list = match items {
        Value::Null => return Ok(()),
        Value::Array(a) => a,
        _ => return Err(format!("{what} must be a list of {{object, property, value}}")),
    };
    if list.len() > MAX_ITEM_OBJECTS {
        return Err(format!("{what} has more than {MAX_ITEM_OBJECTS} items"));
    }
    for (i, item) in list.iter().enumerate() {
        let w = format!("{what}[{i}]");
        let o = item.as_object().ok_or_else(|| format!("{w} must be an object"))?;
        check_keys(o, &["object", "property", "value"], &w)?;
        let object = o
            .get("object")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{w}.object must be a string"))?;
        let prop = o
            .get("property")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{w}.property must be a string"))?;
        check_ident(object, "object")?;
        check_ident(prop, "property")?;
        let value = o
            .get("value")
            .ok_or_else(|| format!("{w}.value is missing (null removes the property)"))?;
        let integer = integer_prop(ctx.visual_type.as_deref(), object, prop);
        match generic_value(value, &format!("{w}.value"), integer)? {
            None => ops.push(EntryOp::Remove {
                object: object.to_string(),
                selectors: selectors.to_vec(),
                prop: Some(prop.to_string()),
                label: format!("{target}: {object}.{prop} removed"),
            }),
            Some((v, desc)) => ops.push(EntryOp::Set {
                object: object.to_string(),
                selectors: selectors.to_vec(),
                prop: prop.to_string(),
                value: v,
                label: format!("{target}: {object}.{prop} = {desc}"),
                check: Check::Value,
            }),
        }
    }
    Ok(())
}

/// The edit of a `color` (`dataPoint.fill`). `selectors` come from [`color_selectors`]; removing
/// the color of a chart's only series removes both its default color and any per-measure color.
fn color_op(
    c: &Value,
    selectors: &[Value],
    st: &SeriesTarget,
    target: &str,
    what: &str,
) -> Result<EntryOp, String> {
    Ok(match c {
        Value::Null => {
            let mut all = selectors.to_vec();
            if all.iter().any(Value::is_null) {
                all.push(st.selector()?);
            }
            EntryOp::Remove {
                object: "dataPoint".into(),
                selectors: all,
                prop: Some("fill".into()),
                label: format!("{target}: dataPoint.fill removed"),
            }
        }
        Value::String(s) if is_color(s) && selectors.iter().any(Value::is_null) => EntryOp::Set {
            object: "dataPoint".into(),
            selectors: selectors.to_vec(),
            prop: "fill".into(),
            value: color_value(s),
            label: format!("{target}: dataPoint.fill = {s} (the chart's only series: its default color)"),
            check: Check::Value,
        },
        Value::String(s) if is_color(s) => EntryOp::Set {
            object: "dataPoint".into(),
            selectors: selectors.to_vec(),
            prop: "fill".into(),
            value: color_value(s),
            label: format!("{target}: dataPoint.fill = {s}"),
            check: Check::Value,
        },
        _ => {
            return Err(format!(
                "{what} must be a color like #118DFF (#RGB, #RRGGBB or #RRGGBBAA), or null"
            ));
        }
    })
}

fn series_item(
    item: &Value,
    i: usize,
    ctx: &Ctx<'_>,
    sole: Option<&str>,
    ops: &mut Vec<EntryOp>,
) -> Result<SeriesTarget, String> {
    let what = format!("series[{i}]");
    let o = item
        .as_object()
        .ok_or_else(|| format!("{what} must be an object"))?;
    check_keys(
        o,
        &["target", "color", "labels", "line", "objects", "clear"],
        &what,
    )?;
    let t = o
        .get("target")
        .ok_or_else(|| format!("{what}.target is missing (\"[Total]\" or \"Product[Name]=Bikes\")"))?;
    let target = parse_target(t, ctx, &format!("{what}.target"))?;
    let selectors = vec![target.selector()?];
    let label = target.label();
    match o.get("clear") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => {}
        Some(Value::Bool(true)) => ops.push(EntryOp::Clear {
            selectors: selectors.clone(),
            label: format!("{label}: formatting cleared"),
        }),
        Some(_) => return Err(format!("{what}.clear must be true or false")),
    }
    if let Some(c) = o.get("color") {
        let color_sel = color_selectors(&target, sole)?;
        ops.push(color_op(
            c,
            &color_sel,
            &target,
            &label,
            &format!("{what}.color"),
        )?);
    }
    for g in GROUPS {
        if let Some(x) = o.get(g.key) {
            group_ops(g, x, &selectors, &label, ctx, &format!("{what}.{}", g.key), ops)?;
        }
    }
    if let Some(x) = o.get("objects") {
        object_ops(x, &selectors, &label, ctx, &format!("{what}.objects"), ops)?;
    }
    Ok(target)
}

/// The edits the `series` and `colors` keys of `props` ask for (empty when neither is there).
/// Validates everything; nothing is written.
pub(crate) fn parse(
    props: &Map<String, Value>,
    visual: &Visual,
    model: Option<&SemanticModel>,
) -> Result<Vec<EntryOp>, String> {
    let mut ops = Vec::new();
    if !props.contains_key("series") && !props.contains_key("colors") {
        return Ok(ops);
    }
    let ctx = Ctx::new(visual, model);
    let sole = sole_series(visual);
    let (no_colors, no_items) = (Map::new(), Vec::new());
    if let Some(colors) = props.get("colors") {
        let map = match colors {
            Value::Null => &no_colors,
            Value::Object(m) => m,
            _ => return Err("`colors` must be an object {\"<target>\": \"#hex\" | null}".into()),
        };
        if map.len() > MAX_SERIES {
            return Err(format!("`colors` has more than {MAX_SERIES} entries"));
        }
        for (k, c) in map {
            let what = format!("colors[{k}]");
            let target = parse_target(&json!(k), &ctx, &what)?;
            let selectors = color_selectors(&target, sole.as_deref())?;
            ops.push(color_op(c, &selectors, &target, &target.label(), &what)?);
        }
    }
    if let Some(series) = props.get("series") {
        let items = match series {
            Value::Null => &no_items,
            Value::Array(a) => a,
            _ => {
                return Err(
                    "`series` must be a list of {target, color?, labels?, line?, objects?, clear?}".into(),
                );
            }
        };
        if items.len() > MAX_SERIES {
            return Err(format!("`series` has more than {MAX_SERIES} items"));
        }
        let mut seen: Vec<SeriesTarget> = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let t = series_item(item, i, &ctx, sole.as_deref(), &mut ops)?;
            if seen.iter().any(|s| same_target(s, &t)) {
                return Err(format!("series[{i}] targets {} again", t.label()));
            }
            seen.push(t);
        }
    }
    Ok(ops)
}

fn same_target(a: &SeriesTarget, b: &SeriesTarget) -> bool {
    match (a, b) {
        (SeriesTarget::Field { query_ref: x, .. }, SeriesTarget::Field { query_ref: y, .. }) => x == y,
        (SeriesTarget::Value { field: f, lit: l }, SeriesTarget::Value { field: g, lit: m }) => {
            same_field(f, g) && l.same(m)
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------------------------

/// An extracted series item: its selector, its friendly form, the `(object, property)` pairs
/// already taken.
type SeriesItem = (Value, Map<String, Value>, Vec<(String, String)>);

/// Adds `series` to `out`: every selector entry whose selector is a [`SeriesTarget`], except the
/// conditional-formatting values [`crate::visual_conditional`] extracts.
pub(crate) fn extract(v: &Visual, out: &mut Map<String, Value>) {
    let Some(objects) = v.visual_extra.get("objects").and_then(Value::as_object) else {
        return;
    };
    let ctx = Ctx::new(v, None);
    // A single-series chart: its default color is its series' color (what apply writes).
    let sole = sole_series(v);
    let default_color = sole.as_ref().and_then(|_| default_fill(objects));
    // (selector, target, item, (object, property) pairs already taken)
    let mut items: Vec<SeriesItem> = Vec::new();
    for (object, arr) in objects {
        for entry in arr.as_array().into_iter().flatten() {
            let Some(sel) = entry.get("selector") else {
                continue;
            };
            let Some(props) = entry.get("properties").and_then(Value::as_object) else {
                continue;
            };
            let Some(target) = decode_selector(sel, &ctx) else {
                continue;
            };
            for (prop, val) in props {
                if crate::visual_conditional::is_conditional(object, prop, val) {
                    continue;
                }
                let idx = match items.iter().position(|(s, _, _)| same_json(s, sel)) {
                    Some(i) => i,
                    None => {
                        let mut m = Map::new();
                        m.insert("target".into(), target.to_json());
                        items.push((sel.clone(), m, Vec::new()));
                        items.len().saturating_sub(1)
                    }
                };
                let Some((_, item, taken)) = items.get_mut(idx) else {
                    continue;
                };
                // Duplicate entries: the first value is the one apply finds.
                if taken.iter().any(|(o, p)| o == object && p == prop) {
                    continue;
                }
                taken.push((object.clone(), prop.clone()));
                // The series' own color is shadowed by the default one: kept as an object edit.
                let shadowed = default_color.is_some()
                    && object == "dataPoint"
                    && prop == "fill"
                    && matches!(&target, SeriesTarget::Field { query_ref, .. } if Some(query_ref) == sole.as_ref());
                if shadowed {
                    place_object(item, object, prop, val);
                } else {
                    place(item, object, prop, val);
                }
            }
        }
    }
    if let (Some(r), Some(color)) = (&sole, default_color)
        && let Some(field) = ctx.field_of_ref(r)
    {
        let sel = json!({ "metadata": r });
        let idx = match items.iter().position(|(s, _, _)| same_json(s, &sel)) {
            Some(i) => i,
            None => {
                let mut m = Map::new();
                let target = SeriesTarget::Field {
                    field: field.clone(),
                    query_ref: r.clone(),
                };
                m.insert("target".into(), target.to_json());
                items.insert(0, (sel, m, Vec::new()));
                0
            }
        };
        if let Some((_, item, _)) = items.get_mut(idx) {
            item.insert("color".into(), json!(color));
        }
    }
    let list: Vec<Value> = items
        .into_iter()
        .filter(|(_, m, _)| m.len() > 1)
        .map(|(_, m, _)| Value::Object(m))
        .collect();
    if !list.is_empty() {
        out.insert("series".into(), Value::Array(list));
    }
}

/// Puts one property of a series entry in its friendly place.
fn place(item: &mut Map<String, Value>, object: &str, prop: &str, val: &Value) {
    if object == "dataPoint"
        && prop == "fill"
        && let Some(c) = color_text(val).filter(|c| is_color(c))
    {
        item.insert("color".into(), json!(c));
        return;
    }
    for g in GROUPS {
        if g.object != object {
            continue;
        }
        if let Some((friendly, _, kind)) = g.props.iter().find(|(_, p, _)| *p == prop)
            && let Some(x) = decode_kind(val, *kind)
        {
            let slot = item
                .entry(g.key.to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if let Some(m) = slot.as_object_mut() {
                m.insert((*friendly).to_string(), x);
                return;
            }
        }
    }
    place_object(item, object, prop, val);
}

/// Puts one property of a series entry in the item's `objects` list.
fn place_object(item: &mut Map<String, Value>, object: &str, prop: &str, val: &Value) {
    let slot = item
        .entry("objects".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Some(a) = slot.as_array_mut() {
        a.push(json!({"object": object, "property": prop, "value": decode_generic(val)}));
    }
}

/// The plain color of the first `dataPoint` entry without a selector that sets `fill` (the
/// entry a sole-series color edit finds), when it is one.
fn default_fill(objects: &Map<String, Value>) -> Option<String> {
    let entry = objects.get("dataPoint")?.as_array()?.iter().find(|e| {
        e.get("selector").is_none_or(Value::is_null)
            && e.get("properties").and_then(|p| p.get("fill")).is_some()
    })?;
    let fill = entry.get("properties")?.get("fill")?;
    if crate::visual_conditional::is_conditional("dataPoint", "fill", fill) {
        return None;
    }
    color_text(fill).filter(|c| is_color(c))
}

// ---------------------------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------------------------

/// `visual.series {page, visual, target, color?, labels?, line?, objects?, clear?}` or
/// `{page, visual, series?: [...], colors?: {...}}`.
pub(crate) fn visual_series(s: &mut crate::Session, p: &Value) -> Result<Value, String> {
    const ITEM_KEYS: &[&str] = &["target", "color", "labels", "line", "objects", "clear"];
    crate::params::allow(
        p,
        &[
            "page", "visual", "target", "color", "labels", "line", "objects", "clear", "series", "colors",
        ],
    )?;
    let mut series: Vec<Value> = match p.get("series") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.clone(),
        Some(_) => return Err("parameter `series` must be a list".into()),
    };
    let mut item = Map::new();
    for k in ITEM_KEYS {
        if let Some(x) = p.get(*k) {
            item.insert((*k).to_string(), x.clone());
        }
    }
    if !item.is_empty() {
        if !item.contains_key("target") {
            return Err("missing parameter `target` (\"[Total]\" or \"Product[Name]=Bikes\")".into());
        }
        series.push(Value::Object(item));
    }
    let mut props = Map::new();
    if !series.is_empty() {
        props.insert("series".into(), Value::Array(series));
    }
    if let Some(c) = p.get("colors") {
        props.insert("colors".into(), c.clone());
    }
    if props.is_empty() {
        return Err("nothing to do: pass a target with color, labels, line, objects or clear; or series: [...]; or colors: {...}".into());
    }
    crate::visual_props::edit_visual(s, p, |_, _| Ok(Value::Object(props)))
}
