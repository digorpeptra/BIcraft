//! `Visual.fields` ↔ the PBIR `visual.query` (`queryState.<role>.projections[]`). Shared by the
//! PBIR writer and the legacy layout writer, which both regenerate a query from edited bindings.

use crate::{FieldBinding, FieldRef, ShowAs};
use serde_json::{Map, Value, json};

/// Aggregation codes (`QueryAggregateFunction`, public `semanticQuery` schema) and the names used
/// in `FieldRef::Aggregation::function`. `Count` is the distinct count; `CountNonNull` counts rows.
const AGGREGATIONS: [(u64, &str); 9] = [
    (0, "Sum"),
    (1, "Avg"),
    (2, "Count"),
    (3, "Min"),
    (4, "Max"),
    (5, "CountNonNull"),
    (6, "Median"),
    (7, "StandardDeviation"),
    (8, "Variance"),
];

/// Accepted spellings when writing (case-insensitive), besides the names above.
const ALIASES: [(&str, u64); 5] = [
    ("average", 1),
    ("distinctcount", 2),
    ("stdev", 7),
    ("stddev", 7),
    ("var", 8),
];

fn function_name(code: u64) -> String {
    AGGREGATIONS
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, n)| (*n).to_string())
        .unwrap_or_else(|| code.to_string())
}

fn function_code(name: &str) -> u64 {
    if let Some((c, _)) = AGGREGATIONS.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)) {
        return *c;
    }
    if let Some((_, c)) = ALIASES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
        return *c;
    }
    name.parse().unwrap_or(0)
}

/// One binding's place in a role of a raw `query`: a projection, or the range of projections a
/// field parameter put there (the role's `fieldParameters[]` entry `{parameterExpr, index,
/// length}`, public `visualConfiguration` schema `RoleFieldParameter`), read as one binding of
/// the parameter's column (docs/formats/parameters.md).
enum Slot<'a> {
    Plain(&'a Value),
    Param {
        entry: &'a Value,
        projections: &'a [Value],
    },
}

/// The slots of a role, in projection order. A `fieldParameters` entry that is not a range of
/// the projections, or that starts inside another one, is left out (its projections stay plain).
fn slots(role_state: &Value) -> Vec<Slot<'_>> {
    let Some(projections) = role_state.get("projections").and_then(Value::as_array) else {
        return Vec::new();
    };
    let ranges: Vec<(usize, usize, &Value)> = role_state
        .get("fieldParameters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| {
            e.get("parameterExpr")?;
            let at = usize::try_from(e.get("index")?.as_u64()?).ok()?;
            let n = usize::try_from(e.get("length")?.as_u64()?).ok()?;
            (n > 0 && at.checked_add(n)? <= projections.len()).then_some((at, n, e))
        })
        .collect();
    let mut out = Vec::with_capacity(projections.len());
    let mut i = 0usize;
    while let Some(p) = projections.get(i) {
        match ranges.iter().find(|r| r.0 == i) {
            Some(&(at, n, entry)) => {
                let end = at.saturating_add(n);
                out.push(Slot::Param {
                    entry,
                    projections: projections.get(at..end).unwrap_or_default(),
                });
                i = end;
            }
            None => {
                out.push(Slot::Plain(p));
                i = i.saturating_add(1);
            }
        }
    }
    out
}

/// The bindings a raw `query` yields, in role then projection order. A field parameter's range
/// of projections is one binding of the parameter's column (`parameterExpr`).
pub fn derive(query: Option<&Value>) -> Vec<FieldBinding> {
    let mut out = Vec::new();
    let Some(state) = query.and_then(|q| q.get("queryState")).and_then(Value::as_object) else {
        return out;
    };
    for (role, role_state) in state {
        for slot in slots(role_state) {
            out.push(match slot {
                Slot::Plain(projection) => FieldBinding {
                    role: role.clone(),
                    field: field_ref(projection.get("field")),
                    display_name: projection_display_name(projection),
                    show_as: None,
                },
                Slot::Param { entry, .. } => FieldBinding {
                    role: role.clone(),
                    field: field_ref(entry.get("parameterExpr")),
                    display_name: None,
                    show_as: None,
                },
            });
        }
    }
    // "Show value as": a percentage field's scope is read against the other roles' columns
    let plain = out.clone();
    for b in &mut out {
        if let FieldRef::Other { raw } = &b.field
            && let Some((base, kind)) = show_as_parts(raw, &plain)
        {
            b.field = base;
            b.show_as = Some(kind);
        }
    }
    out
}

/// The `ScopedEval` scope of `kind` over `fields`: the plain column fields of its role (the
/// public `semanticQuery` schema allows only columns there), in order; none for the grand total.
pub fn show_as_scope(kind: ShowAs, fields: &[FieldBinding]) -> Vec<Value> {
    let Some(role) = kind.scope_role() else {
        return Vec::new();
    };
    fields
        .iter()
        .filter(|b| b.role.eq_ignore_ascii_case(role) && b.show_as.is_none())
        .filter(|b| matches!(b.field, FieldRef::Column { .. }))
        .map(|b| field_json(&b.field))
        .collect()
}

/// The PBIR field of `field` shown as `kind` in a visual bound to `fields`:
/// `{"Arithmetic": {"Left": f, "Right": {"ScopedEval": {"Expression": f, "Scope": [...]}},
/// "Operator": 3}}` (3 is Divide in the public `semanticQuery` schema).
pub fn show_as_json(field: &FieldRef, kind: ShowAs, fields: &[FieldBinding]) -> Value {
    let f = field_json(field);
    json!({"Arithmetic": {
        "Left": f,
        "Right": {"ScopedEval": {"Expression": f, "Scope": show_as_scope(kind, fields)}},
        "Operator": 3,
    }})
}

/// `(field, kind)` of a percentage field: `Arithmetic` Divide of a typed field by its
/// `ScopedEval` whose scope is empty (grand total) or exactly the `Columns` (column total) or
/// `Rows` (row total) columns of `fields`. Anything else is no percentage BIcraft models.
fn show_as_parts(raw: &Value, fields: &[FieldBinding]) -> Option<(FieldRef, ShowAs)> {
    let a = single(raw, "Arithmetic")?.as_object()?;
    if a.len() != 3 || a.get("Operator")?.as_u64()? != 3 {
        return None;
    }
    let left = a.get("Left")?;
    let se = single(a.get("Right")?, "ScopedEval")?.as_object()?;
    if se.len() != 2 || se.get("Expression")? != left {
        return None;
    }
    let scope = se.get("Scope")?.as_array()?;
    let base = typed_field(left)?;
    let kind = [ShowAs::GrandTotal, ShowAs::ColumnTotal, ShowAs::RowTotal]
        .into_iter()
        .find(|k| (k.scope_role().is_none() || !scope.is_empty()) && show_as_scope(*k, fields) == *scope)?;
    Some((base, kind))
}

/// Whether role `role` of a raw `query` shows its items with no data (`showAll: true`).
pub fn show_all(query: Option<&Value>, role: &str) -> bool {
    query
        .and_then(|q| q.get("queryState"))
        .and_then(Value::as_object)
        .and_then(|s| s.iter().find(|(k, _)| k.eq_ignore_ascii_case(role)))
        .and_then(|(_, r)| r.get("showAll"))
        == Some(&Value::Bool(true))
}

/// A projection's `displayName` (a non-empty string; anything else is no rename).
pub fn projection_display_name(projection: &Value) -> Option<String> {
    projection
        .get("displayName")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// `{"Entity": t}` exactly (no `Schema`).
fn entity(expr: &Value) -> Option<String> {
    let source = single(expr, "SourceRef")?.as_object()?;
    if source.len() != 1 {
        return None;
    }
    Some(source.get("Entity")?.as_str()?.to_string())
}

/// The value of `key` when `v` is an object with exactly that one key.
fn single<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    let obj = v.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    obj.get(key)
}

/// `{"Expression": {"SourceRef": {"Entity": t}}, "Property": p}` exactly.
fn property(v: &Value) -> Option<(String, String)> {
    let obj = v.as_object()?;
    if obj.len() != 2 {
        return None;
    }
    let table = entity(obj.get("Expression")?)?;
    let name = obj.get("Property")?.as_str()?.to_string();
    Some((table, name))
}

fn typed_field(field: &Value) -> Option<FieldRef> {
    let obj = field.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    let (kind, inner) = obj.iter().next()?;
    match kind.as_str() {
        "Column" => property(inner).map(|(table, column)| FieldRef::Column { table, column }),
        "Measure" => property(inner).map(|(table, measure)| FieldRef::Measure { table, measure }),
        "Aggregation" => {
            let agg = inner.as_object()?;
            if agg.len() != 2 {
                return None;
            }
            let (table, column) = property(single(agg.get("Expression")?, "Column")?)?;
            let code = agg.get("Function")?.as_u64()?;
            Some(FieldRef::Aggregation {
                table,
                column,
                function: function_name(code),
            })
        }
        _ => None,
    }
}

/// The [`FieldRef`] of a PBIR field expression.
pub fn field_ref(field: Option<&Value>) -> FieldRef {
    match field {
        Some(f) => typed_field(f).unwrap_or_else(|| FieldRef::Other { raw: f.clone() }),
        None => FieldRef::Other { raw: Value::Null },
    }
}

fn property_json(table: &str, name: &str) -> Value {
    json!({"Expression": {"SourceRef": {"Entity": table}}, "Property": name})
}

/// The PBIR field JSON of a binding.
pub fn field_json(field: &FieldRef) -> Value {
    match field {
        FieldRef::Column { table, column } => json!({"Column": property_json(table, column)}),
        FieldRef::Measure { table, measure } => json!({"Measure": property_json(table, measure)}),
        FieldRef::Aggregation {
            table,
            column,
            function,
        } => json!({"Aggregation": {
            "Expression": {"Column": property_json(table, column)},
            "Function": function_code(function),
        }}),
        FieldRef::Other { raw } => raw.clone(),
    }
}

/// `projection` with `displayName` set to `name`, or without one for `None` (a projection whose
/// `displayName` is not a non-empty string keeps it: it is no rename).
fn with_display_name(mut projection: Value, name: Option<&str>) -> Value {
    let Some(o) = projection.as_object_mut() else {
        return projection;
    };
    match name {
        Some(n) => {
            o.insert("displayName".into(), Value::String(n.to_string()));
        }
        None => {
            if o.get("displayName")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
            {
                o.shift_remove("displayName");
            }
        }
    }
    projection
}

/// A visual calculation: a projection whose field is `{"NativeVisualCalculation": {"Language":
/// "dax", "Expression": e, "Name": n, "DataType"?}}` (public `semanticQuery` schema,
/// `QueryNativeVisualCalc`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualCalc {
    pub name: String,
    pub expression: String,
    /// `DataType` when the projection states one.
    pub data_type: Option<String>,
}

/// The visual calculation a field is, if it is one.
pub fn visual_calc(field: &FieldRef) -> Option<VisualCalc> {
    let FieldRef::Other { raw } = field else {
        return None;
    };
    let c = single(raw, "NativeVisualCalculation")?;
    Some(VisualCalc {
        name: c.get("Name")?.as_str()?.to_string(),
        expression: c.get("Expression")?.as_str()?.to_string(),
        data_type: c.get("DataType").and_then(Value::as_str).map(str::to_string),
    })
}

/// The field of a visual calculation named `name` computing `expression` (DAX).
pub fn visual_calc_field(name: &str, expression: &str) -> FieldRef {
    FieldRef::Other {
        raw: json!({"NativeVisualCalculation": {"Language": "dax", "Expression": expression, "Name": name}}),
    }
}

/// Points Desktop computes per sparkline (public `semanticQuery` schema,
/// `QuerySparklineDataExpression.PointsPerSparkline`, a constant 52; Microsoft Learn, "Create
/// sparklines in a table or matrix": up to 52 points per sparkline).
pub const SPARKLINE_POINTS: u64 = 52;

/// A sparkline: a projection whose field is `{"SparklineData": {"Measure": m, "Groupings": [x]}}`
/// (public `semanticQuery` schema, `QuerySparklineDataExpression`; docs/formats/sparklines.md).
/// `PointsPerSparkline` (52) is read when present; Desktop 2.159.1231.0 does not write it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sparkline {
    /// The Y axis: a measure or a summarized column.
    pub measure: FieldRef,
    /// The X axis: the columns the measure is evaluated per (Desktop's dialog sets one).
    pub groupings: Vec<FieldRef>,
    /// `ApplyCalculationGroupTo` when written (`Sparkline` or `Point`).
    pub apply_calc_group_to: Option<String>,
}

/// The sparkline a field is, if it is one (its measure and groupings typed fields).
pub fn sparkline(field: &FieldRef) -> Option<Sparkline> {
    let FieldRef::Other { raw } = field else {
        return None;
    };
    let s = single(raw, "SparklineData")?;
    let measure = typed_field(s.get("Measure")?)?;
    if !matches!(measure, FieldRef::Measure { .. } | FieldRef::Aggregation { .. }) {
        return None;
    }
    let groupings = s
        .get("Groupings")?
        .as_array()?
        .iter()
        .map(typed_field)
        .collect::<Option<Vec<_>>>()?;
    if groupings.is_empty() || !groupings.iter().all(|g| matches!(g, FieldRef::Column { .. })) {
        return None;
    }
    Some(Sparkline {
        measure,
        groupings,
        apply_calc_group_to: s
            .get("ApplyCalculationGroupTo")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// The field of a sparkline of `measure` over `groupings`, as Desktop 2.159.1231.0 writes it:
/// without `PointsPerSparkline`.
pub fn sparkline_field(s: &Sparkline) -> FieldRef {
    let mut body = Map::new();
    body.insert("Measure".into(), field_json(&s.measure));
    body.insert(
        "Groupings".into(),
        Value::Array(s.groupings.iter().map(field_json).collect()),
    );
    if let Some(a) = &s.apply_calc_group_to {
        body.insert("ApplyCalculationGroupTo".into(), json!(a));
    }
    FieldRef::Other {
        raw: json!({"SparklineData": Value::Object(body)}),
    }
}

/// Per binding of a raw `query` (role then projection order, as [`derive`] lists them): its
/// visual-scoped format string (`format`, public `visualConfiguration` schema `RoleProjection`:
/// "format string scoped to the visual"; Desktop's Format pane › General › Data format).
pub fn projection_formats(query: Option<&Value>) -> Vec<Option<String>> {
    let roles = query
        .and_then(|q| q.get("queryState"))
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|state| state.values());
    roles
        .flat_map(slots)
        .map(|slot| match slot {
            Slot::Plain(p) => p
                .get("format")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            Slot::Param { .. } => None,
        })
        .collect()
}

/// Per binding of a raw `query` (role then projection order, as [`derive`] lists them): its
/// `nativeQueryRef` (the name visual calculations refer to it by) and whether it is `hidden`
/// (kept in the visual matrix, not shown).
pub fn projection_meta(query: Option<&Value>) -> Vec<(Option<String>, bool)> {
    let roles = query
        .and_then(|q| q.get("queryState"))
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|state| state.values());
    roles
        .flat_map(slots)
        .map(|slot| match slot {
            Slot::Plain(p) => (
                p.get("nativeQueryRef")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                p.get("hidden") == Some(&Value::Bool(true)),
            ),
            Slot::Param { .. } => (None, false),
        })
        .collect()
}

/// Where binding `index` (role then binding order, as [`derive`] lists them) of a raw `query`
/// lies: its role, the position of its first projection in the role, how many projections it
/// spans and its `fieldParameters` entry when it is a field parameter's range.
fn locate(query: &Value, index: usize) -> Option<(String, usize, usize, Option<Value>)> {
    let state = query.get("queryState")?.as_object()?;
    let mut n = 0usize;
    for (role, role_state) in state {
        let mut at = 0usize;
        for slot in slots(role_state) {
            let (len, entry) = match slot {
                Slot::Plain(_) => (1, None),
                Slot::Param { entry, projections } => (projections.len(), Some(entry.clone())),
            };
            if n == index {
                return Some((role.clone(), at, len, entry));
            }
            n = n.saturating_add(1);
            at = at.saturating_add(len);
        }
    }
    None
}

/// The projection of binding `index` (role then binding order, as [`derive`] lists them) of a
/// raw `query`; `None` for a field parameter's range (several projections).
pub fn projection_mut(query: &mut Value, index: usize) -> Option<&mut Map<String, Value>> {
    let (role, at, _, entry) = locate(query, index)?;
    if entry.is_some() {
        return None;
    }
    query
        .get_mut("queryState")?
        .get_mut(&role)?
        .get_mut("projections")?
        .get_mut(at)?
        .as_object_mut()
}

/// Writes binding `index` of a raw `query` (role then binding order, as [`derive`] lists them),
/// a field parameter's label column `param`, the way Power BI Desktop saves a visual bound to a
/// field parameter (docs/formats/parameters.md): one projection per field the parameter lists,
/// in its order, each with `displayName` its label, and a `fieldParameters` entry
/// `{parameterExpr, index, length}` on the role naming the range. The role's other entries move
/// with their projections. `false` (nothing changed) when `index` is no binding or `fields` is
/// empty.
pub fn set_field_parameter(
    query: &mut Value,
    index: usize,
    param: &FieldRef,
    fields: &[(FieldRef, String)],
) -> bool {
    if fields.is_empty() {
        return false;
    }
    let Some((role, at, len, old)) = locate(query, index) else {
        return false;
    };
    let Some(role_state) = query
        .get_mut("queryState")
        .and_then(|s| s.get_mut(&role))
        .and_then(Value::as_object_mut)
    else {
        return false;
    };
    let new: Vec<Value> = fields
        .iter()
        .enumerate()
        .map(|(i, (f, label))| {
            let (query_ref, native) = refs(f, i);
            json!({"field": field_json(f), "queryRef": query_ref, "nativeQueryRef": native, "displayName": label})
        })
        .collect();
    let added = new.len();
    let Some(list) = role_state.get_mut("projections").and_then(Value::as_array_mut) else {
        return false;
    };
    let end = at.saturating_add(len).min(list.len());
    if at > end {
        return false;
    }
    list.splice(at..end, new);
    let mut entries: Vec<Value> = role_state
        .get("fieldParameters")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(old) = &old {
        entries.retain(|e| e != old);
    }
    for e in &mut entries {
        if let Some(i) = e.get("index").and_then(Value::as_u64)
            && let Ok(i) = usize::try_from(i)
            && i >= end
            && let Some(o) = e.as_object_mut()
        {
            let moved = i.saturating_sub(end).saturating_add(at).saturating_add(added);
            o.insert("index".into(), json!(moved));
        }
    }
    entries.push(json!({"parameterExpr": field_json(param), "index": at, "length": added}));
    entries.sort_by_key(|e| e.get("index").and_then(Value::as_u64).unwrap_or(u64::MAX));
    role_state.insert("fieldParameters".into(), Value::Array(entries));
    true
}

/// `(Entity, Hierarchy, Level)` of a `HierarchyLevel` field over an entity.
pub fn hierarchy_level(raw: &Value) -> Option<(String, String, String)> {
    let hl = raw.get("HierarchyLevel")?;
    let h = hl.get("Expression")?.get("Hierarchy")?;
    let entity = h.get("Expression")?.get("SourceRef")?.get("Entity")?.as_str()?;
    let hierarchy = h.get("Hierarchy")?.as_str()?;
    let level = hl.get("Level")?.as_str()?;
    Some((entity.to_string(), hierarchy.to_string(), level.to_string()))
}

/// The PBIR field of level `level` of hierarchy `hierarchy` of table `table`.
pub fn hierarchy_level_json(table: &str, hierarchy: &str, level: &str) -> Value {
    json!({"HierarchyLevel": {
        "Expression": {"Hierarchy": {"Expression": {"SourceRef": {"Entity": table}}, "Hierarchy": hierarchy}},
        "Level": level,
    }})
}

/// `queryRef` / `nativeQueryRef` as Desktop forms them.
fn refs(field: &FieldRef, fallback: usize) -> (String, String) {
    match field {
        FieldRef::Column { table, column } => (format!("{table}.{column}"), column.clone()),
        FieldRef::Measure { table, measure } => (format!("{table}.{measure}"), measure.clone()),
        FieldRef::Aggregation {
            table,
            column,
            function,
        } => {
            let name = function_name(function_code(function));
            (format!("{name}({table}.{column})"), format!("{name} of {column}"))
        }
        FieldRef::Other { raw } if hierarchy_level(raw).is_some() => {
            // Desktop: `Sales.Geo.Region` and `Geo Region` (layout-oracle fixtures)
            let (e, h, l) = hierarchy_level(raw).unwrap_or_default();
            (format!("{e}.{h}.{l}"), format!("{h} {l}"))
        }
        // a sparkline: BIcraft's names (Desktop's are not publicly documented; G106)
        FieldRef::Other { .. } if sparkline(field).is_some() => {
            let s = sparkline(field).unwrap_or_default();
            let (m_ref, m_name) = refs(&s.measure, fallback);
            let x: Vec<(String, String)> = s.groupings.iter().map(|g| refs(g, fallback)).collect();
            let x_refs: Vec<&str> = x.iter().map(|r| r.0.as_str()).collect();
            let x_names: Vec<&str> = x.iter().map(|r| r.1.as_str()).collect();
            (
                format!("SparklineData({m_ref}_[{}])", x_refs.join(", ")),
                format!("{m_name} by {}", x_names.join(", ")),
            )
        }
        // a visual calculation is selected and referred to by its name
        FieldRef::Other { .. } if visual_calc(field).is_some() => {
            let name = visual_calc(field).map(|c| c.name).unwrap_or_default();
            (name.clone(), name)
        }
        FieldRef::Other { raw } => match other_name(raw) {
            // e.g. a report-level measure (`SourceRef.Schema: "extension"`): Desktop names it
            // like a model measure.
            Some((entity, property)) => (format!("{entity}.{property}"), property),
            None => {
                let name = format!("Field{fallback}");
                (name.clone(), name)
            }
        },
    }
}

/// `queryRef` / `nativeQueryRef` of a binding: a percentage field is named after the expression
/// (`Divide(Sum(Sales.Amount), ScopedEval(Sum(Sales.Amount), []))`, `%GT Sum of Amount`).
fn binding_refs(binding: &FieldBinding, fields: &[FieldBinding], fallback: usize) -> (String, String) {
    let (query_ref, native) = refs(&binding.field, fallback);
    let Some(kind) = binding.show_as else {
        return (query_ref, native);
    };
    let scope: Vec<String> = fields
        .iter()
        .filter(|b| kind.scope_role().is_some_and(|r| b.role.eq_ignore_ascii_case(r)))
        .filter(|b| b.show_as.is_none() && matches!(b.field, FieldRef::Column { .. }))
        .map(|b| refs(&b.field, 0).0)
        .collect();
    (
        format!(
            "Divide({query_ref}, ScopedEval({query_ref}, [{}]))",
            scope.join(", ")
        ),
        format!("{} {native}", kind.prefix()),
    )
}

/// `(Entity, Property)` of a raw `Measure` / `Column` expression whose source is an entity
/// (report-level measures carry `"Schema": "extension"` next to `Entity`).
fn other_name(raw: &Value) -> Option<(String, String)> {
    let body = raw.get("Measure").or_else(|| raw.get("Column"))?;
    let entity = body
        .get("Expression")?
        .get("SourceRef")?
        .get("Entity")?
        .as_str()?;
    let property = body.get("Property")?.as_str()?;
    Some((entity.to_string(), property.to_string()))
}

/// The `nativeQueryRef` Desktop gives a new projection of `field` (`Total`, `Sum of Amount`,
/// a visual calculation's name).
pub fn default_native_name(field: &FieldRef) -> String {
    refs(field, 0).1
}

/// The `queryRef` Desktop gives a new projection of `field` (`Sales.Total`,
/// `Sum(Sales.Amount)`); `None` for a field it cannot name (`FieldRef::Other`).
pub fn default_query_ref(field: &FieldRef) -> Option<String> {
    match field {
        FieldRef::Other { raw } if hierarchy_level(raw).is_some() => Some(refs(field, 0).0),
        FieldRef::Other { .. } if visual_calc(field).is_some() || sparkline(field).is_some() => {
            Some(refs(field, 0).0)
        }
        FieldRef::Other { raw } => other_name(raw).map(|(e, p)| format!("{e}.{p}")),
        f => Some(refs(f, 0).0),
    }
}

/// The `query` a writer emits for `visual`: its raw `visual_extra["query"]`, or one regenerated
/// from [`Visual::fields`](crate::Visual::fields) when the bindings were edited. `None` when the
/// visual has no query.
pub fn effective(visual: &crate::Visual) -> Option<Value> {
    let raw = visual.visual_extra.get("query");
    if derive(raw) == visual.fields {
        raw.cloned()
    } else {
        regenerate(raw, &visual.fields)
    }
}

/// The projections of a `query`, in role then projection order.
fn projections(query: Option<&Value>) -> impl Iterator<Item = &Value> {
    query
        .and_then(|q| q.get("queryState"))
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|state| state.values())
        .filter_map(|role| role.get("projections").and_then(Value::as_array))
        .flatten()
}

/// Whether `r` is `kept` plus a numeric suffix (`Sum(Sales.Qty)1` for `Sum(Sales.Qty)`).
fn suffixed(kept: &str, r: &str) -> bool {
    r.strip_prefix(kept)
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

/// `(kept, duplicate)` queryRef pairs of a `query` that projects one field expression under its
/// first queryRef and again under that queryRef plus a numeric suffix (`Sum(Sales.Qty)` and
/// `Sum(Sales.Qty)1`), as bicraft wrote before it reused the first one. The query then selects
/// the field twice, and Power BI Desktop 2.158 fails it. `kept` is the field's first queryRef in
/// role then projection order (the select item the legacy writer keeps). Desktop itself projects
/// one field under other distinct queryRefs in some visuals (e.g. `T.M` and `T.M (…)`): those are
/// left alone, and projections of different fields never pair up, whatever their names.
pub fn duplicate_refs(query: Option<&Value>) -> Vec<(String, String)> {
    let mut first: Vec<(&Value, &str)> = Vec::new();
    let mut out: Vec<(String, String)> = Vec::new();
    for p in projections(query) {
        let (Some(field), Some(r)) = (p.get("field"), p.get("queryRef").and_then(Value::as_str)) else {
            continue;
        };
        match first.iter().find(|(f, _)| *f == field) {
            Some((_, kept)) => {
                if suffixed(kept, r) && !out.iter().any(|(k, d)| k == kept && d == r) {
                    out.push(((*kept).to_string(), r.to_string()));
                }
            }
            None => first.push((field, r)),
        }
    }
    out
}

/// Rewrites every projection of a field that carries the field's first queryRef plus a numeric
/// suffix to that first queryRef (and its `nativeQueryRef`), so the query selects the field once. Returns the
/// `(kept, duplicate)` pairs it replaced (see [`duplicate_refs`]).
pub fn dedupe_refs(query: &mut Value) -> Vec<(String, String)> {
    let pairs = duplicate_refs(Some(query));
    if pairs.is_empty() {
        return pairs;
    }
    // field -> (queryRef, nativeQueryRef) of its first projection.
    let mut first: Vec<(Value, String, Option<Value>)> = Vec::new();
    let Some(state) = query.get_mut("queryState").and_then(Value::as_object_mut) else {
        return Vec::new();
    };
    for role in state.values_mut() {
        let Some(list) = role.get_mut("projections").and_then(Value::as_array_mut) else {
            continue;
        };
        for p in list.iter_mut() {
            let Some(field) = p.get("field").cloned() else {
                continue;
            };
            let Some(current) = p.get("queryRef").and_then(Value::as_str).map(str::to_string) else {
                continue;
            };
            let Some(obj) = p.as_object_mut() else {
                continue;
            };
            match first.iter().find(|(f, _, _)| *f == field) {
                Some((_, kept, _)) if !suffixed(kept, &current) => {}
                Some((_, kept, native)) => {
                    obj.insert("queryRef".into(), Value::String(kept.clone()));
                    match native {
                        Some(n) => {
                            obj.insert("nativeQueryRef".into(), n.clone());
                        }
                        None => {
                            obj.shift_remove("nativeQueryRef");
                        }
                    }
                }
                None => {
                    let kept = obj
                        .get("queryRef")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    first.push((field, kept, obj.get("nativeQueryRef").cloned()));
                }
            }
        }
    }
    pairs
}

/// A new `query` for `fields`, reusing projections (and role objects) of `raw` where the binding
/// is unchanged. `None` when the visual no longer needs a query. A field projected under two
/// queryRefs (a query saved by an older bicraft) is repaired: see [`dedupe_refs`].
pub fn regenerate(raw: Option<&Value>, fields: &[FieldBinding]) -> Option<Value> {
    let mut query = regenerate_projections(raw, fields)?;
    dedupe_refs(&mut query);
    Some(query)
}

/// A binding as the query selects it: its field and what it is shown as.
type Key = (FieldRef, Option<ShowAs>);

fn regenerate_projections(raw: Option<&Value>, fields: &[FieldBinding]) -> Option<Value> {
    let mut query = raw.and_then(Value::as_object).cloned().unwrap_or_default();
    let old_state = query
        .get("queryState")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    // Pool of existing projections: (role, projection, binding, used). A field parameter's range
    // is one entry (`Value::Array` of its projections) whose `fieldParameters` entry is in
    // `ranges` at the same pool position.
    let old_bindings = derive(raw);
    let mut pool: Vec<(String, Value, Key, bool)> = Vec::new();
    let mut ranges: Vec<Option<Value>> = Vec::new();
    let mut k = 0usize;
    for (role, role_state) in &old_state {
        for slot in slots(role_state) {
            let (p, entry, plain_field) = match slot {
                Slot::Plain(p) => (p.clone(), None, p.get("field")),
                Slot::Param { entry, projections } => (
                    Value::Array(projections.to_vec()),
                    Some(entry.clone()),
                    entry.get("parameterExpr"),
                ),
            };
            let key = old_bindings.get(k).map_or_else(
                || (field_ref(plain_field), None),
                |b| (b.field.clone(), b.show_as),
            );
            k = k.saturating_add(1);
            pool.push((role.clone(), p, key, false));
            ranges.push(entry);
        }
    }
    // fieldParameters entries of the new query, per role
    let mut role_ranges: Vec<(String, Value)> = Vec::new();
    let mut state = Map::new();
    let mut used_refs: Vec<String> = Vec::new();
    // (field, queryRef) of the projections placed so far: a field bound again (in another role,
    // e.g. a scatter chart's Y and Size) reuses its queryRef, as Power BI Desktop does. The query
    // then selects the field once; a suffixed copy (`Sum(T.C)1`) makes Desktop fail the query.
    // A field and its percentage ("Show value as") are different select items.
    let mut placed: Vec<(Key, String)> = Vec::new();
    for (i, binding) in fields.iter().enumerate() {
        let key = (binding.field.clone(), binding.show_as);
        let reuse = pool
            .iter()
            .position(|(r, _, f, used)| !used && r == &binding.role && *f == key)
            .or_else(|| pool.iter().position(|(_, _, f, used)| !used && *f == key));
        // a field parameter's range is kept whole, with its entry
        if let Some(j) = reuse
            && let Some(Some(range)) = ranges.get(j)
            && let Some(entry) = pool.get_mut(j)
        {
            entry.3 = true;
            let Some(list) = role_projections(&mut state, &old_state, &binding.role) else {
                continue;
            };
            let mut range = range.clone();
            if let Some(o) = range.as_object_mut() {
                o.insert("index".into(), json!(list.len()));
            }
            for p in entry.1.as_array().into_iter().flatten() {
                if let Some(r) = p.get("queryRef").and_then(Value::as_str) {
                    used_refs.push(r.to_string());
                }
                list.push(p.clone());
            }
            role_ranges.push((binding.role.clone(), range));
            continue;
        }
        let field_of = |b: &FieldBinding| match b.show_as {
            Some(kind) => show_as_json(&b.field, kind, fields),
            None => field_json(&b.field),
        };
        let projection = match reuse.and_then(|i| pool.get_mut(i)) {
            Some(entry) => {
                entry.3 = true;
                let mut p = entry.1.clone();
                // a percentage's scope follows the fields of its role
                if binding.show_as.is_some()
                    && let Some(o) = p.as_object_mut()
                {
                    o.insert("field".into(), field_of(binding));
                }
                p
            }
            None => {
                let (mut query_ref, native) = binding_refs(binding, fields, i);
                if let Some((_, same)) = placed.iter().find(|(f, _)| *f == key) {
                    query_ref = same.clone();
                } else {
                    let base = query_ref.clone();
                    let mut n = 1u32;
                    while used_refs.contains(&query_ref) {
                        query_ref = format!("{base}{n}");
                        n = n.saturating_add(1);
                    }
                }
                json!({"field": field_of(binding), "queryRef": query_ref, "nativeQueryRef": native})
            }
        };
        let projection = with_display_name(projection, binding.display_name.as_deref());
        if let Some(r) = projection.get("queryRef").and_then(Value::as_str) {
            used_refs.push(r.to_string());
            if !placed.iter().any(|(f, _)| *f == key) {
                placed.push((key, r.to_string()));
            }
        }
        if let Some(list) = role_projections(&mut state, &old_state, &binding.role) {
            list.push(projection);
        }
    }
    for (role, role_state) in &mut state {
        let Some(obj) = role_state.as_object_mut() else {
            continue;
        };
        let kept: Vec<Value> = role_ranges
            .iter()
            .filter(|(r, _)| r == role)
            .map(|(_, e)| e.clone())
            .collect();
        if kept.is_empty() {
            obj.shift_remove("fieldParameters");
        } else {
            obj.insert("fieldParameters".into(), Value::Array(kept));
        }
    }
    if state.is_empty() {
        query.shift_remove("queryState");
        if query.is_empty() {
            return None;
        }
        query.insert("queryState".into(), Value::Object(Map::new()));
        return Some(Value::Object(query));
    }
    if query.contains_key("queryState") {
        query.insert("queryState".into(), Value::Object(state));
        Some(Value::Object(query))
    } else {
        let mut out = Map::new();
        out.insert("queryState".into(), Value::Object(state));
        out.extend(query);
        Some(Value::Object(out))
    }
}

/// The projections of role `role` of the new query `state`: the first time, a copy of the old
/// role object's other keys (`showAll`, …) with no projection yet.
fn role_projections<'a>(
    state: &'a mut Map<String, Value>,
    old_state: &Map<String, Value>,
    role: &str,
) -> Option<&'a mut Vec<Value>> {
    let role_state = state.entry(role.to_string()).or_insert_with(|| {
        let mut obj = old_state
            .get(role)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        obj.insert("projections".into(), Value::Array(Vec::new()));
        Value::Object(obj)
    });
    role_state.get_mut("projections").and_then(Value::as_array_mut)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(role: &str, field: FieldRef) -> FieldBinding {
        FieldBinding {
            display_name: None,
            role: role.into(),
            field,
            show_as: None,
        }
    }

    /// "Show value as": the projection's field is `field / ScopedEval(field, scope)`, the scope
    /// the `Columns` (column total) or `Rows` (row total) columns; read back to the same binding,
    /// and the scope follows the role's fields when they change.
    #[test]
    fn show_value_as_is_a_scoped_division() {
        let amount = FieldRef::Aggregation {
            table: "Sales".into(),
            column: "Amount".into(),
            function: "Sum".into(),
        };
        let col = |c: &str| FieldRef::Column {
            table: "Sales".into(),
            column: c.into(),
        };
        let mut pct = b("Values", amount.clone());
        pct.show_as = Some(ShowAs::GrandTotal);
        let fields = vec![b("Rows", col("Region")), b("Columns", col("Year")), pct.clone()];
        let q = regenerate(None, &fields).unwrap();
        let p = &q["queryState"]["Values"]["projections"][0];
        assert_eq!(p["field"]["Arithmetic"]["Operator"], 3);
        assert_eq!(p["field"]["Arithmetic"]["Left"], field_json(&amount));
        assert_eq!(
            p["field"]["Arithmetic"]["Right"]["ScopedEval"],
            json!({"Expression": field_json(&amount), "Scope": []})
        );
        assert_eq!(
            p["queryRef"],
            "Divide(Sum(Sales.Amount), ScopedEval(Sum(Sales.Amount), []))"
        );
        assert_eq!(p["nativeQueryRef"], "%GT Sum of Amount");
        assert_eq!(derive(Some(&q)), fields);
        // column and row totals keep the Columns / Rows columns
        for (kind, role, col_name) in [
            (ShowAs::ColumnTotal, "Columns", "Year"),
            (ShowAs::RowTotal, "Rows", "Region"),
        ] {
            let mut f = fields.clone();
            if let Some(x) = f.get_mut(2) {
                x.show_as = Some(kind);
            }
            let q = regenerate(Some(&q), &f).unwrap();
            let p = &q["queryState"]["Values"]["projections"][0];
            assert_eq!(
                p["field"]["Arithmetic"]["Right"]["ScopedEval"]["Scope"],
                json!([field_json(&col(col_name))]),
                "{role}"
            );
            assert_eq!(derive(Some(&q)), f, "{role}");
            // another Columns / Rows field: the reused projection's scope follows
            // (after the role's own field: the query keeps the fields role by role)
            let mut more = f.clone();
            let at = more.iter().rposition(|x| x.role == role).map_or(0, |i| i + 1);
            more.insert(at, b(role, col("City")));
            let q2 = regenerate(Some(&q), &more).unwrap();
            assert_eq!(
                q2["queryState"]["Values"]["projections"][0]["field"]["Arithmetic"]["Right"]["ScopedEval"]
                    ["Scope"]
                    .as_array()
                    .map(Vec::len),
                Some(2)
            );
            assert_eq!(derive(Some(&q2)), more);
        }
        // a field and its percentage are two select items
        let both = vec![b("Values", amount.clone()), pct.clone()];
        let q = regenerate(None, &both).unwrap();
        let ps = &q["queryState"]["Values"]["projections"];
        assert_ne!(ps[0]["queryRef"], ps[1]["queryRef"]);
        assert_eq!(derive(Some(&q)), both);
        // shapes BIcraft does not model stay raw: another operator, another scope, a mismatch
        let raw = |op: u64, scope: Value, right: &FieldRef| {
            json!({"queryState": {"Values": {"projections": [{"field": {"Arithmetic": {
                "Left": field_json(&amount),
                "Right": {"ScopedEval": {"Expression": field_json(right), "Scope": scope}},
                "Operator": op}}, "queryRef": "x"}]}}})
        };
        for q in [
            raw(0, json!([]), &amount),
            raw(3, json!([field_json(&col("Other"))]), &amount),
            raw(3, json!([]), &col("Region")),
        ] {
            let d = derive(Some(&q));
            assert!(matches!(d[0].field, FieldRef::Other { .. }));
            assert_eq!(d[0].show_as, None);
            assert_eq!(regenerate(Some(&q), &d).unwrap(), q);
        }
        assert!(!show_all(Some(&q), "Values"));
        let sa = json!({"queryState": {"Rows": {"showAll": true, "projections": []}}});
        assert!(show_all(Some(&sa), "rows"));
        assert!(!show_all(None, "Rows"));
    }

    #[test]
    fn derive_and_regenerate_are_inverse() {
        let fields = vec![
            b(
                "Category",
                FieldRef::Column {
                    table: "Date".into(),
                    column: "Year".into(),
                },
            ),
            b(
                "Y",
                FieldRef::Aggregation {
                    table: "Sales".into(),
                    column: "Amount".into(),
                    function: "Sum".into(),
                },
            ),
            b(
                "Y",
                FieldRef::Measure {
                    table: "Sales".into(),
                    measure: "Margin".into(),
                },
            ),
        ];
        let q = regenerate(None, &fields).unwrap();
        assert_eq!(derive(Some(&q)), fields);
        assert_eq!(
            q["queryState"]["Y"]["projections"][0]["queryRef"],
            "Sum(Sales.Amount)"
        );
        // Unknown codes survive as their number.
        let other = json!({"queryState": {"Values": {"projections": [{"field": {"Aggregation": {
            "Expression": {"Column": property_json("T", "C")}, "Function": 42}}, "queryRef": "x"}]}}});
        let derived = derive(Some(&other));
        assert_eq!(
            derived[0].field,
            FieldRef::Aggregation {
                table: "T".into(),
                column: "C".into(),
                function: "42".into()
            }
        );
        assert_eq!(regenerate(Some(&other), &derived).unwrap(), other);
    }

    #[test]
    fn regenerate_keeps_existing_projection_details_and_other_keys() {
        let raw = json!({"queryState": {"Values": {"showAll": true, "projections": [
            {"field": {"Measure": property_json("T", "M")}, "queryRef": "T.M", "nativeQueryRef": "Renamed", "active": true}
        ]}}, "sortDefinition": {"sort": []}});
        let mut fields = derive(Some(&raw));
        fields.push(b(
            "Values",
            FieldRef::Column {
                table: "T".into(),
                column: "C".into(),
            },
        ));
        let q = regenerate(Some(&raw), &fields).unwrap();
        assert_eq!(q["queryState"]["Values"]["showAll"], true);
        assert_eq!(
            q["queryState"]["Values"]["projections"][0]["nativeQueryRef"],
            "Renamed"
        );
        assert_eq!(q["queryState"]["Values"]["projections"][1]["queryRef"], "T.C");
        assert_eq!(q["sortDefinition"], json!({"sort": []}));
        assert_eq!(regenerate(Some(&json!({"queryState": {}})), &[]), None);
    }

    /// Regression: the same aggregation in two roles (scatter Y and Size) got `Sum(Sales.Qty)1`
    /// for its second projection, and Power BI Desktop 2.158 failed the visual's query.
    #[test]
    fn a_field_in_two_roles_shares_its_query_ref() {
        let qty = FieldRef::Aggregation {
            table: "Sales".into(),
            column: "Qty".into(),
            function: "Sum".into(),
        };
        let fields = vec![
            b(
                "Category",
                FieldRef::Column {
                    table: "Sales".into(),
                    column: "Region".into(),
                },
            ),
            b("Y", qty.clone()),
            b("Size", qty.clone()),
        ];
        let q = regenerate(None, &fields).unwrap();
        for role in ["Y", "Size"] {
            assert_eq!(
                q["queryState"][role]["projections"][0]["queryRef"], "Sum(Sales.Qty)",
                "{role}"
            );
        }
        assert_eq!(derive(Some(&q)), fields);
        // Adding the second role to an existing query reuses the first projection's queryRef,
        // even a custom one.
        let mut first = regenerate(None, &fields[..2]).unwrap();
        first["queryState"]["Y"]["projections"][0]["queryRef"] = json!("Custom");
        let q = regenerate(Some(&first), &fields).unwrap();
        assert_eq!(q["queryState"]["Size"]["projections"][0]["queryRef"], "Custom");
        // Different fields with the same default name still get distinct queryRefs.
        let named = |raw: Value| b("Values", FieldRef::Other { raw });
        let q = regenerate(
            None,
            &[named(json!({"Literal": 1})), named(json!({"Literal": 2}))],
        )
        .unwrap();
        assert_ne!(
            q["queryState"]["Values"]["projections"][0]["queryRef"],
            q["queryState"]["Values"]["projections"][1]["queryRef"]
        );
    }

    /// A scatter query as older bicraft saved it: the field of Y again in Size, suffixed.
    fn suffixed_query() -> Value {
        let qty =
            json!({"Aggregation": {"Expression": {"Column": property_json("Sales", "Qty")}, "Function": 0}});
        json!({"queryState": {
            "Category": {"projections": [{"field": {"Column": property_json("Sales", "Region")},
                                          "queryRef": "Sales.Region", "nativeQueryRef": "Region"}]},
            "Y": {"projections": [{"field": qty.clone(), "queryRef": "Sum(Sales.Qty)", "nativeQueryRef": "Sum of Qty"}]},
            "Size": {"projections": [{"field": qty, "queryRef": "Sum(Sales.Qty)1",
                                      "nativeQueryRef": "Sum of Qty1", "displayName": "Volume"}]}
        }})
    }

    #[test]
    fn duplicate_refs_flags_one_field_under_two_query_refs_and_dedupe_repairs_it() {
        let mut q = suffixed_query();
        let pair = ("Sum(Sales.Qty)".to_string(), "Sum(Sales.Qty)1".to_string());
        assert_eq!(duplicate_refs(Some(&q)), vec![pair.clone()]);
        assert_eq!(dedupe_refs(&mut q), vec![pair]);
        assert_eq!(
            q["queryState"]["Size"]["projections"][0]["queryRef"],
            "Sum(Sales.Qty)"
        );
        assert_eq!(
            q["queryState"]["Size"]["projections"][0]["nativeQueryRef"],
            "Sum of Qty"
        );
        // Other projection details stay.
        assert_eq!(q["queryState"]["Size"]["projections"][0]["displayName"], "Volume");
        assert!(duplicate_refs(Some(&q)).is_empty());
        assert!(dedupe_refs(&mut q).is_empty());
        // Re-binding repairs too: regenerate keeps the projections but dedupes them.
        let raw = suffixed_query();
        let mut fields = derive(Some(&raw));
        fields.push(b(
            "Category",
            FieldRef::Column {
                table: "Sales".into(),
                column: "City".into(),
            },
        ));
        let q = regenerate(Some(&raw), &fields).unwrap();
        assert!(duplicate_refs(Some(&q)).is_empty());
        assert_eq!(
            q["queryState"]["Size"]["projections"][0]["queryRef"],
            "Sum(Sales.Qty)"
        );
    }

    #[test]
    fn duplicate_refs_ignores_same_query_ref_and_different_fields() {
        // The same field under the same queryRef (Desktop's own form) is fine.
        let mut q = suffixed_query();
        q["queryState"]["Size"]["projections"][0]["queryRef"] = json!("Sum(Sales.Qty)");
        assert!(duplicate_refs(Some(&q)).is_empty());
        // Desktop's own second queryRef for one field (not a numeric suffix) stays, also when
        // the visual is re-bound.
        let mut q = suffixed_query();
        q["queryState"]["Size"]["projections"][0]["queryRef"] = json!("Sum(Sales.Qty) (Size)");
        assert!(duplicate_refs(Some(&q)).is_empty());
        let before = q.clone();
        assert!(dedupe_refs(&mut q).is_empty());
        assert_eq!(q, before);
        let again = regenerate(Some(&q), &derive(Some(&q))).unwrap();
        assert_eq!(
            again["queryState"]["Size"]["projections"][0]["queryRef"],
            "Sum(Sales.Qty) (Size)"
        );
        // A suffix that is not all digits is not ours either.
        let mut q = suffixed_query();
        q["queryState"]["Size"]["projections"][0]["queryRef"] = json!("Sum(Sales.Qty)1a");
        assert!(duplicate_refs(Some(&q)).is_empty());
        // Different fields that share a name or differ only by suffix are fine.
        let other =
            json!({"Aggregation": {"Expression": {"Column": property_json("Sales", "Qty")}, "Function": 1}});
        let mut q = suffixed_query();
        q["queryState"]["Size"]["projections"][0]["field"] = other;
        assert!(duplicate_refs(Some(&q)).is_empty());
        let before = q.clone();
        assert!(dedupe_refs(&mut q).is_empty());
        assert_eq!(q, before);
        assert!(duplicate_refs(None).is_empty());
        assert!(duplicate_refs(Some(&json!({"queryState": {"Y": {"projections": 3}}}))).is_empty());
    }

    #[test]
    fn report_measures_get_a_named_query_ref() {
        let raw = json!({"Measure": {"Expression": {"SourceRef": {"Schema": "extension", "Entity": "Sales"}},
                                     "Property": "Share"}});
        let field = FieldRef::Other { raw };
        assert_eq!(default_query_ref(&field).as_deref(), Some("Sales.Share"));
        let q = regenerate(None, &[b("Values", field)]).unwrap();
        assert_eq!(
            q["queryState"]["Values"]["projections"][0]["queryRef"],
            "Sales.Share"
        );
        assert_eq!(
            q["queryState"]["Values"]["projections"][0]["nativeQueryRef"],
            "Share"
        );
        assert_eq!(
            default_query_ref(&FieldRef::Other {
                raw: json!({"Literal": 1})
            }),
            None
        );
    }

    /// A per-visual rename is the projection's `displayName`: read, written on a new projection,
    /// kept or removed on a reused one; a whole hierarchy is one `HierarchyLevel` projection per
    /// level, named as Desktop names them.
    #[test]
    fn display_names_and_hierarchy_levels() {
        let total = FieldRef::Measure {
            table: "Sales".into(),
            measure: "Total".into(),
        };
        let mut renamed = b("Y", total.clone());
        renamed.display_name = Some("Revenue".into());
        let q = regenerate(None, &[renamed.clone()]).unwrap();
        assert_eq!(q["queryState"]["Y"]["projections"][0]["displayName"], "Revenue");
        assert_eq!(derive(Some(&q)), vec![renamed.clone()]);
        // renamed again, then back: the projection is reused (its queryRef kept)
        let mut q2 = q.clone();
        q2["queryState"]["Y"]["projections"][0]["queryRef"] = json!("kept");
        let mut again = renamed.clone();
        again.display_name = Some("Sales total".into());
        let r = regenerate(Some(&q2), &[again]).unwrap();
        assert_eq!(
            r["queryState"]["Y"]["projections"][0]["displayName"],
            "Sales total"
        );
        assert_eq!(r["queryState"]["Y"]["projections"][0]["queryRef"], "kept");
        let back = regenerate(Some(&q2), &[b("Y", total)]).unwrap();
        assert!(
            back["queryState"]["Y"]["projections"][0]
                .get("displayName")
                .is_none()
        );
        assert_eq!(back["queryState"]["Y"]["projections"][0]["queryRef"], "kept");
        // an empty displayName is no rename, and stays as it was
        let empty = json!({"queryState": {"Y": {"projections": [{"field": field_json(&renamed.field),
            "queryRef": "Sales.Total", "displayName": ""}]}}});
        let d = derive(Some(&empty));
        assert_eq!(d[0].display_name, None);
        assert_eq!(regenerate(Some(&empty), &d).unwrap(), empty);
        // hierarchy levels
        let level = |l: &str| FieldRef::Other {
            raw: hierarchy_level_json("Sales", "Geo", l),
        };
        assert_eq!(
            hierarchy_level(&hierarchy_level_json("Sales", "Geo", "Region")),
            Some(("Sales".into(), "Geo".into(), "Region".into()))
        );
        let q = regenerate(None, &[b("Rows", level("Region")), b("Rows", level("Product"))]).unwrap();
        let p = &q["queryState"]["Rows"]["projections"];
        assert_eq!(p[0]["queryRef"], "Sales.Geo.Region");
        assert_eq!(p[0]["nativeQueryRef"], "Geo Region");
        assert_eq!(p[1]["queryRef"], "Sales.Geo.Product");
        assert_eq!(
            default_query_ref(&level("Region")).as_deref(),
            Some("Sales.Geo.Region")
        );
    }

    /// G64: a role bound to a field parameter, as Power BI Desktop 2.159.1231.0 saved it (the
    /// parameter's fields as projections, `fieldParameters` naming the range), is one binding of
    /// the parameter's label column; `set_field_parameter` writes that shape, and regenerating
    /// after another edit keeps the range whole.
    #[test]
    fn a_field_parameter_range_is_one_binding() {
        let col = |t: &str, c: &str| FieldRef::Column {
            table: t.into(),
            column: c.into(),
        };
        let total = FieldRef::Measure {
            table: "Sales".into(),
            measure: "Total".into(),
        };
        let pick = col("Pick", "Pick");
        let listed = vec![
            (col("Sales", "Region"), "Region".to_string()),
            (total.clone(), "Amount".to_string()),
        ];
        let fields = vec![b("Values", col("Sales", "City")), b("Values", pick.clone())];
        let mut q = regenerate(None, &fields).unwrap();
        assert!(set_field_parameter(&mut q, 1, &pick, &listed));
        let values = &q["queryState"]["Values"];
        assert_eq!(values["projections"].as_array().map(Vec::len), Some(3));
        assert_eq!(
            values["projections"][1],
            json!({"field": field_json(&col("Sales", "Region")), "queryRef": "Sales.Region",
                   "nativeQueryRef": "Region", "displayName": "Region"})
        );
        assert_eq!(values["projections"][2]["displayName"], "Amount");
        assert_eq!(values["projections"][2]["queryRef"], "Sales.Total");
        assert_eq!(
            values["fieldParameters"],
            json!([{"parameterExpr": field_json(&pick), "index": 1, "length": 2}])
        );
        assert_eq!(derive(Some(&q)), fields);
        // writing it again changes nothing
        let mut again = q.clone();
        assert!(set_field_parameter(&mut again, 1, &pick, &listed));
        assert_eq!(again, q);
        // an edit before the range moves it; removing the parameter drops the entry
        let mut more = vec![b("Values", col("Sales", "Year"))];
        more.extend(fields.clone());
        let q2 = regenerate(Some(&q), &more).unwrap();
        assert_eq!(q2["queryState"]["Values"]["fieldParameters"][0]["index"], 2);
        assert_eq!(derive(Some(&q2)), more);
        let q3 = regenerate(Some(&q), &fields[..1]).unwrap();
        assert!(q3["queryState"]["Values"].get("fieldParameters").is_none());
        assert_eq!(derive(Some(&q3)), fields[..1].to_vec());
        // binding metadata lines up with the bindings: the range has none
        assert_eq!(projection_meta(Some(&q)).len(), 2);
        assert!(projection_mut(&mut q.clone(), 1).is_none());
        assert!(projection_mut(&mut q.clone(), 0).is_some());
        // a range that is not one of the projections is no range
        let mut bad = q.clone();
        bad["queryState"]["Values"]["fieldParameters"][0]["length"] = json!(9);
        assert_eq!(derive(Some(&bad)).len(), 3);
        assert!(!set_field_parameter(&mut q.clone(), 5, &pick, &listed));
        assert!(!set_field_parameter(&mut q.clone(), 1, &pick, &[]));
    }

    /// A sparkline is a `SparklineData` field read back to the same measure and X axis, named
    /// after both; a projection's visual-scoped `format` lines up with its binding and survives a
    /// regeneration of the query.
    #[test]
    fn sparklines_and_visual_formats() {
        let amount = FieldRef::Aggregation {
            table: "Sales".into(),
            column: "Amount".into(),
            function: "Sum".into(),
        };
        let date = FieldRef::Column {
            table: "Sales".into(),
            column: "Date".into(),
        };
        let s = Sparkline {
            measure: amount.clone(),
            groupings: vec![date.clone()],
            apply_calc_group_to: None,
        };
        let f = sparkline_field(&s);
        assert_eq!(sparkline(&f), Some(s.clone()));
        let FieldRef::Other { raw } = &f else {
            panic!("a sparkline is a raw field")
        };
        // Desktop 2.159.1231.0 writes no PointsPerSparkline; one that has it reads the same
        assert!(raw["SparklineData"].get("PointsPerSparkline").is_none());
        let mut with_points = raw.clone();
        with_points["SparklineData"]["PointsPerSparkline"] = json!(52);
        assert_eq!(sparkline(&FieldRef::Other { raw: with_points }), Some(s.clone()));
        assert_eq!(raw["SparklineData"]["Measure"], field_json(&amount));
        assert_eq!(raw["SparklineData"]["Groupings"][0], field_json(&date));
        assert_eq!(
            default_query_ref(&f).as_deref(),
            Some("SparklineData(Sum(Sales.Amount)_[Sales.Date])")
        );
        assert_eq!(default_native_name(&f), "Sum of Amount by Date");
        // not a sparkline: a column as Y, no X, a non-column X
        let bad = |m: Value, g: Value| FieldRef::Other {
            raw: json!({"SparklineData": {"Measure": m, "Groupings": g}}),
        };
        assert!(sparkline(&bad(field_json(&date), json!([field_json(&date)]))).is_none());
        assert!(sparkline(&bad(field_json(&amount), json!([]))).is_none());
        assert!(sparkline(&bad(field_json(&amount), json!([field_json(&amount)]))).is_none());
        assert!(sparkline(&bad(json!(3), json!("x"))).is_none());
        // formats
        let fields = vec![
            b("Values", date.clone()),
            b("Values", amount.clone()),
            b("Values", f),
        ];
        let mut q = regenerate(None, &fields).unwrap();
        assert_eq!(derive(Some(&q)), fields);
        assert_eq!(projection_formats(Some(&q)), vec![None, None, None]);
        projection_mut(&mut q, 1)
            .unwrap()
            .insert("format".into(), json!("#,0.00"));
        assert_eq!(
            projection_formats(Some(&q)),
            vec![None, Some("#,0.00".to_string()), None]
        );
        // a field added in front: the format stays with its projection
        let region = FieldRef::Column {
            table: "Sales".into(),
            column: "Region".into(),
        };
        let mut more = vec![b("Values", region)];
        more.extend(fields);
        let q2 = regenerate(Some(&q), &more).unwrap();
        assert_eq!(
            projection_formats(Some(&q2)),
            vec![None, None, Some("#,0.00".to_string()), None]
        );
    }
}
