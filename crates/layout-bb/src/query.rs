//! A visual's data: legacy `singleVisual.projections` + `prototypeQuery` (+ `columnProperties`,
//! `showAllRoles`, `hasDefaultSort`, `isDrillDisabled`) ↔ PBIR `visual.query`.
//!
//! In the legacy form a projection names a `prototypeQuery.Select` item by its `Name` (the
//! queryRef); the select item's expression refers to a `From` alias. In PBIR each projection
//! carries its field expression, with the entity instead of the alias (black-box LO-visuals,
//! LO-report).

use serde_json::{Map, Value, json};

use crate::codes::{self, SORT_DIRECTION};
use crate::path::{At, MAX_DEPTH, Object, Unmapped};

/// The `singleVisual` keys this module reads.
pub(crate) const KEYS: [&str; 6] = [
    "projections",
    "prototypeQuery",
    "columnProperties",
    "showAllRoles",
    "hasDefaultSort",
    "isDrillDisabled",
];

/// `(entity, schema)` of each plain `From` alias.
type Aliases = Vec<(String, (String, Option<String>))>;

/// Replaces every `SourceRef: {Source: alias}` by `SourceRef: {Schema?, Entity}`.
pub(crate) fn resolve(expr: &Value, aliases: &Aliases) -> Value {
    resolve_at(expr, aliases, 0)
}

fn resolve_at(expr: &Value, aliases: &Aliases, depth: usize) -> Value {
    if depth > MAX_DEPTH {
        return expr.clone();
    }
    match expr {
        Value::Object(m) => {
            let mut out = Map::new();
            for (k, v) in m {
                if k == "SourceRef"
                    && let Some(alias) = v
                        .as_object()
                        .filter(|o| o.len() == 1)
                        .and_then(|o| o.get("Source"))
                        .and_then(Value::as_str)
                    && let Some((_, (entity, schema))) = aliases.iter().find(|(a, _)| a == alias)
                {
                    let mut src = Map::new();
                    if let Some(s) = schema {
                        src.insert("Schema".into(), json!(s));
                    }
                    src.insert("Entity".into(), json!(entity));
                    out.insert(k.clone(), Value::Object(src));
                } else {
                    out.insert(k.clone(), resolve_at(v, aliases, depth + 1));
                }
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(|v| resolve_at(v, aliases, depth + 1)).collect()),
        other => other.clone(),
    }
}

/// Builds the aliases of a query being written (`From`), in first-use order.
#[derive(Default)]
pub(crate) struct FromBuilder {
    entries: Vec<(String, String, Option<String>)>,
}

impl FromBuilder {
    fn alias(&mut self, entity: &str, schema: Option<&str>) -> String {
        if let Some((a, _, _)) = self
            .entries
            .iter()
            .find(|(_, e, s)| e == entity && s.as_deref() == schema)
        {
            return a.clone();
        }
        let base: String = entity
            .chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_lowercase().collect())
            .unwrap_or_else(|| "t".to_string());
        let mut name = base.clone();
        let mut n = 1u32;
        while self.entries.iter().any(|(a, _, _)| *a == name) {
            name = format!("{base}{n}");
            n = n.saturating_add(1);
        }
        self.entries
            .push((name.clone(), entity.to_string(), schema.map(str::to_string)));
        name
    }

    /// The `From` array.
    pub fn from(&self) -> Value {
        Value::Array(
            self.entries
                .iter()
                .map(|(a, e, s)| {
                    let mut m = Map::new();
                    m.insert("Name".into(), json!(a));
                    m.insert("Entity".into(), json!(e));
                    if let Some(s) = s {
                        m.insert("Schema".into(), json!(s));
                    }
                    m.insert("Type".into(), json!(0));
                    Value::Object(m)
                })
                .collect(),
        )
    }

    /// The inverse of [`resolve`]: entity references become aliases.
    pub fn unresolve(&mut self, expr: &Value) -> Value {
        self.unresolve_at(expr, 0)
    }

    fn unresolve_at(&mut self, expr: &Value, depth: usize) -> Value {
        if depth > MAX_DEPTH {
            return expr.clone();
        }
        match expr {
            Value::Object(m) => {
                let mut out = Map::new();
                for (k, v) in m {
                    let entity = v
                        .as_object()
                        .filter(|o| o.keys().all(|k| k == "Entity" || k == "Schema"))
                        .and_then(|o| {
                            Some((
                                o.get("Entity")?.as_str()?.to_string(),
                                o.get("Schema").and_then(Value::as_str).map(str::to_string),
                            ))
                        });
                    match entity {
                        Some((e, s)) if k == "SourceRef" => {
                            let alias = self.alias(&e, s.as_deref());
                            out.insert(k.clone(), json!({"Source": alias}));
                        }
                        _ => {
                            out.insert(k.clone(), self.unresolve_at(v, depth + 1));
                        }
                    }
                }
                Value::Object(out)
            }
            Value::Array(a) => Value::Array(a.iter().map(|v| self.unresolve_at(v, depth + 1)).collect()),
            other => other.clone(),
        }
    }
}

/// `(alias, entity, schema)` of a `From` entry that names an entity (`Type` 0), nothing else.
fn plain_from(f: &Value) -> Option<(String, String, Option<String>)> {
    let o = f.as_object()?;
    if !o
        .keys()
        .all(|k| matches!(k.as_str(), "Name" | "Entity" | "Schema" | "Type"))
    {
        return None;
    }
    if o.get("Type").is_some_and(|t| t.as_i64() != Some(0)) {
        return None;
    }
    let schema = match o.get("Schema") {
        None => None,
        Some(s) => Some(s.as_str()?.to_string()),
    };
    Some((
        o.get("Name")?.as_str()?.to_string(),
        o.get("Entity")?.as_str()?.to_string(),
        schema,
    ))
}

/// The single key of a select item that is its expression (`Column`, `Measure`, …).
fn expression_of(item: &Object) -> Option<(&String, &Value)> {
    let mut it = item
        .iter()
        .filter(|(k, _)| k.as_str() != "Name" && k.as_str() != "NativeReferenceName");
    let first = it.next()?;
    if it.next().is_some() {
        return None;
    }
    Some(first)
}

/// Reads the legacy query keys of `sv` (a `singleVisual`, at `at`) into a PBIR `query`.
pub(crate) fn from_legacy(sv: &Object, at: &At, un: &mut Unmapped) -> Option<Value> {
    let empty = Map::new();
    let proto = sv.get("prototypeQuery");
    let proto_obj = proto.and_then(Value::as_object).unwrap_or(&empty);
    let pat = at.key("prototypeQuery");
    if let Some(p) = proto.filter(|p| !p.is_object()) {
        un.put(&pat, p.clone());
    }

    // Aliases.
    let mut aliases: Aliases = Vec::new();
    if let Some(from) = proto_obj.get("From") {
        match from.as_array() {
            Some(list) => {
                for (i, f) in list.iter().enumerate() {
                    match plain_from(f) {
                        Some((name, entity, schema)) => aliases.push((name, (entity, schema))),
                        None => un.put(&pat.key("From").index(i), f.clone()),
                    }
                }
            }
            None => un.put(&pat.key("From"), from.clone()),
        }
    }
    // Select items by queryRef: (expression, native name, index).
    let mut selects: Vec<(String, Value, Option<Value>, usize, bool)> = Vec::new();
    if let Some(sel) = proto_obj.get("Select") {
        match sel.as_array() {
            Some(list) => {
                for (i, s) in list.iter().enumerate() {
                    let parsed = s.as_object().and_then(|o| {
                        let name = o.get("Name")?.as_str()?.to_string();
                        let (k, v) = expression_of(o)?;
                        let mut e = Map::new();
                        e.insert(k.clone(), resolve(v, &aliases));
                        Some((name, Value::Object(e), o.get("NativeReferenceName").cloned()))
                    });
                    match parsed {
                        Some((name, e, native)) if !selects.iter().any(|x| x.0 == name) => {
                            selects.push((name, e, native, i, false));
                        }
                        _ => un.put(&pat.key("Select").index(i), s.clone()),
                    }
                }
            }
            None => un.put(&pat.key("Select"), sel.clone()),
        }
    }
    for (k, v) in proto_obj {
        match k.as_str() {
            "From" | "Select" | "OrderBy" => {}
            "Version" if v.as_i64() == Some(2) => {}
            _ => un.put(&pat.key(k), v.clone()),
        }
    }

    let column_props = sv.get("columnProperties").and_then(Value::as_object);
    if let Some(cp) = sv.get("columnProperties")
        && !cp.is_object()
    {
        un.put(&at.key("columnProperties"), cp.clone());
    }
    let mut used_props: Vec<String> = Vec::new();
    let show_all: Vec<String> = match sv.get("showAllRoles") {
        None => Vec::new(),
        Some(Value::Array(a)) if a.iter().all(Value::is_string) => {
            a.iter().filter_map(Value::as_str).map(str::to_string).collect()
        }
        Some(other) => {
            un.put(&at.key("showAllRoles"), other.clone());
            Vec::new()
        }
    };

    let mut state = Map::new();
    let proj_at = at.key("projections");
    match sv.get("projections") {
        None => {}
        Some(Value::Object(roles)) => {
            for (role, list) in roles {
                let Some(list) = list.as_array() else {
                    un.put(&proj_at.key(role), list.clone());
                    continue;
                };
                let mut out = Vec::new();
                for (i, p) in list.iter().enumerate() {
                    let qr = p.get("queryRef").and_then(Value::as_str);
                    let found = qr.and_then(|q| selects.iter_mut().find(|s| s.0 == q));
                    let (Some(Value::Object(pm)), Some(sel)) = (Some(p), found) else {
                        un.put(&proj_at.key(role).index(i), p.clone());
                        continue;
                    };
                    sel.4 = true;
                    let mut po = Map::new();
                    po.insert("field".into(), sel.1.clone());
                    po.insert("queryRef".into(), json!(sel.0));
                    if let Some(n) = &sel.2 {
                        po.insert("nativeQueryRef".into(), n.clone());
                    }
                    if let Some(props) = column_props.and_then(|c| c.get(&sel.0)) {
                        let pat = at.key("columnProperties").key(&sel.0);
                        if !used_props.contains(&sel.0) {
                            used_props.push(sel.0.clone());
                            match props.as_object() {
                                Some(pr) => {
                                    for (k, v) in pr {
                                        match k.as_str() {
                                            "displayName" | "formatString" if v.is_string() => {}
                                            _ => un.put(&pat.key(k), v.clone()),
                                        }
                                    }
                                }
                                None => un.put(&pat, props.clone()),
                            }
                        }
                        if let Some(d) = props.get("displayName").filter(|v| v.is_string()) {
                            po.insert("displayName".into(), d.clone());
                        }
                        if let Some(f) = props.get("formatString").filter(|v| v.is_string()) {
                            po.insert("format".into(), f.clone());
                        }
                    }
                    for (k, v) in pm {
                        if k != "queryRef" {
                            po.insert(k.clone(), v.clone());
                        }
                    }
                    out.push(Value::Object(po));
                }
                let mut rs = Map::new();
                rs.insert("projections".into(), Value::Array(out));
                if show_all.contains(role) {
                    rs.insert("showAll".into(), json!(true));
                }
                state.insert(role.clone(), Value::Object(rs));
            }
        }
        Some(other) => un.put(&proj_at, other.clone()),
    }
    for (i, r) in show_all.iter().enumerate() {
        if !state.contains_key(r) {
            un.put(&at.key("showAllRoles").index(i), json!(r));
        }
    }
    if let Some(cp) = column_props {
        for (k, v) in cp {
            if !used_props.contains(k) {
                un.put(&at.key("columnProperties").key(k), v.clone());
            }
        }
    }
    for (name, e, native, i, used) in &selects {
        if !used {
            let mut m = Map::new();
            if let Value::Object(em) = e {
                m.extend(em.clone());
            }
            m.insert("Name".into(), json!(name));
            if let Some(n) = native {
                m.insert("NativeReferenceName".into(), n.clone());
            }
            // Kept with its entity resolved: the writer turns entities back into aliases.
            un.put(&pat.key("Select").index(*i), Value::Object(m));
        }
    }

    // Sort.
    let mut sort = Vec::new();
    if let Some(ob) = proto_obj.get("OrderBy") {
        match ob.as_array() {
            Some(list) => {
                for (i, o) in list.iter().enumerate() {
                    let dir = o.get("Direction").and_then(|d| codes::name(SORT_DIRECTION, d));
                    let expr = o.get("Expression");
                    match (dir, expr, o.as_object().map(Map::len)) {
                        (Some(d), Some(e), Some(2)) => {
                            sort.push(json!({"field": resolve(e, &aliases), "direction": d}));
                        }
                        _ => un.put(&pat.key("OrderBy").index(i), o.clone()),
                    }
                }
            }
            None => un.put(&pat.key("OrderBy"), ob.clone()),
        }
    }
    let mut sort_def = Map::new();
    if !sort.is_empty() {
        sort_def.insert("sort".into(), Value::Array(sort));
    }
    match sv.get("hasDefaultSort") {
        None => {}
        Some(Value::Bool(true)) => {
            sort_def.insert("isDefaultSort".into(), json!(true));
        }
        Some(other) => un.put(&at.key("hasDefaultSort"), other.clone()),
    }

    if state.is_empty() && sort_def.is_empty() {
        if let Some(d) = sv.get("isDrillDisabled") {
            un.put(&at.key("isDrillDisabled"), d.clone());
        }
        return None;
    }
    let mut q = Map::new();
    q.insert("queryState".into(), Value::Object(state));
    if !sort_def.is_empty() {
        q.insert("sortDefinition".into(), Value::Object(sort_def));
    }
    match sv.get("isDrillDisabled") {
        None => {}
        Some(d @ Value::Bool(_)) => {
            q.insert("isDrillDisabled".into(), d.clone());
        }
        Some(other) => un.put(&at.key("isDrillDisabled"), other.clone()),
    }
    Some(Value::Object(q))
}

/// Writes a PBIR `query` into the legacy keys of `sv`. Unrepresentable parts are added to
/// `errors`, prefixed with `at`.
pub(crate) fn to_legacy(query: &Value, sv: &mut Object, at: &str, errors: &mut Vec<String>) {
    let Some(q) = query.as_object() else {
        errors.push(format!("{at}: query is not an object"));
        return;
    };
    let mut from = FromBuilder::default();
    let mut selects: Vec<Value> = Vec::new();
    let mut select_names: Vec<String> = Vec::new();
    let mut projections = Map::new();
    let mut column_props = Map::new();
    let mut show_all = Vec::new();
    let empty = Map::new();
    for (role, rs) in q.get("queryState").and_then(Value::as_object).unwrap_or(&empty) {
        let Some(rs) = rs.as_object() else {
            errors.push(format!("{at}: query.queryState.{role} is not an object"));
            continue;
        };
        for k in rs.keys() {
            match k.as_str() {
                // a field parameter's range: its fields are the role's projections (labels as
                // `columnProperties.displayName`), which is what the legacy form keeps; the
                // range itself has no legacy key we have seen in a Desktop file, so it is not
                // written (docs/formats/parameters.md, "In a legacy layout")
                "projections" | "fieldParameters" => {}
                "showAll" => {
                    if rs.get("showAll") == Some(&Value::Bool(true)) {
                        show_all.push(json!(role));
                    }
                }
                other => errors.push(format!("{at}: query.queryState.{role}.{other}")),
            }
        }
        let mut list = Vec::new();
        for p in rs
            .get("projections")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(pm) = p.as_object() else { continue };
            let (Some(field), Some(qr)) = (pm.get("field"), pm.get("queryRef").and_then(Value::as_str))
            else {
                errors.push(format!("{at}: a projection of {role} without field or queryRef"));
                continue;
            };
            if !select_names.iter().any(|n| n == qr) {
                select_names.push(qr.to_string());
                let mut item = from.unresolve(field).as_object().cloned().unwrap_or_default();
                item.insert("Name".into(), json!(qr));
                if let Some(n) = pm.get("nativeQueryRef") {
                    item.insert("NativeReferenceName".into(), n.clone());
                }
                selects.push(Value::Object(item));
            }
            let mut lp = Map::new();
            lp.insert("queryRef".into(), json!(qr));
            for (k, v) in pm {
                match k.as_str() {
                    "field" | "queryRef" | "nativeQueryRef" => {}
                    "displayName" | "format" => {
                        let entry = column_props
                            .entry(qr.to_string())
                            .or_insert_with(|| Value::Object(Map::new()));
                        let key = if k == "format" {
                            "formatString"
                        } else {
                            "displayName"
                        };
                        if let Value::Object(e) = entry {
                            e.entry(key.to_string()).or_insert_with(|| v.clone());
                        }
                    }
                    _ => {
                        lp.insert(k.clone(), v.clone());
                    }
                }
            }
            list.push(Value::Object(lp));
        }
        projections.insert(role.clone(), Value::Array(list));
    }
    let mut order_by = Vec::new();
    let mut has_default_sort = None;
    if let Some(sd) = q.get("sortDefinition") {
        let Some(sd) = sd.as_object() else {
            errors.push(format!("{at}: query.sortDefinition is not an object"));
            return;
        };
        for (k, v) in sd {
            match k.as_str() {
                "sort" => {
                    for s in v.as_array().into_iter().flatten() {
                        let dir = s
                            .get("direction")
                            .and_then(Value::as_str)
                            .and_then(|d| codes::code(SORT_DIRECTION, d));
                        match (dir, s.get("field")) {
                            (Some(d), Some(f)) => {
                                order_by.push(json!({"Direction": d, "Expression": from.unresolve(f)}));
                            }
                            _ => errors.push(format!("{at}: a sort entry without field or direction")),
                        }
                    }
                }
                "isDefaultSort" => has_default_sort = v.as_bool().filter(|b| *b),
                other => errors.push(format!("{at}: query.sortDefinition.{other}")),
            }
        }
    }
    for k in q.keys() {
        match k.as_str() {
            "queryState" | "sortDefinition" => {}
            "isDrillDisabled" => {
                if let Some(v) = q.get(k) {
                    sv.insert("isDrillDisabled".into(), v.clone());
                }
            }
            other => errors.push(format!("{at}: query.{other}")),
        }
    }
    sv.insert("projections".into(), Value::Object(projections));
    if !selects.is_empty() || !order_by.is_empty() {
        let mut proto = Map::new();
        proto.insert("Version".into(), json!(2));
        proto.insert("From".into(), from.from());
        proto.insert("Select".into(), Value::Array(selects));
        if !order_by.is_empty() {
            proto.insert("OrderBy".into(), Value::Array(order_by));
        }
        // `From` is complete only once every expression is unresolved.
        proto.insert("From".into(), from.from());
        sv.insert("prototypeQuery".into(), Value::Object(proto));
    }
    if !column_props.is_empty() {
        sv.insert("columnProperties".into(), Value::Object(column_props));
    }
    if !show_all.is_empty() {
        sv.insert("showAllRoles".into(), Value::Array(show_all));
    }
    if has_default_sort == Some(true) {
        sv.insert("hasDefaultSort".into(), json!(true));
    }
}

/// Unmapped `Select` items were kept with entities: turn them back into aliases of `sv`'s query.
pub(crate) fn unresolve_kept_selects(sv: &mut Object) {
    let Some(Value::Object(proto)) = sv.get_mut("prototypeQuery") else {
        return;
    };
    let mut from = FromBuilder::default();
    let original: Vec<Value> = proto
        .get("From")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // Re-learn the aliases already written.
    for f in &original {
        if let Some((a, e, s)) = plain_from(f) {
            from.entries.push((a, e, s));
        }
    }
    let known = from.entries.len();
    if let Some(Value::Array(sel)) = proto.get_mut("Select") {
        for s in sel.iter_mut() {
            *s = from.unresolve(s);
        }
    }
    if from.entries.len() > known {
        // New aliases go after the original entries (kept as they were, plain or not).
        let mut list = original;
        let tail = FromBuilder {
            entries: from.entries.get(known..).map(<[_]>::to_vec).unwrap_or_default(),
        };
        if let Value::Array(added) = tail.from() {
            list.extend(added);
        }
        proto.insert("From".into(), Value::Array(list));
    }
}
