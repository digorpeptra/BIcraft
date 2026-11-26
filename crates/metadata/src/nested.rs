//! Rows ↔ JSON for the objects the model keeps in `extra`, driven by [`crate::spec`].
//!
//! [`Lookups`] indexes the base rows once; [`Lookups::children`] / [`Lookups::item`] read
//! (rows → TMSL-shaped JSON), [`Writer`] writes (JSON difference → row edits through the
//! [`Editor`] of `apply`). Annotations, extended properties and changed properties of nested
//! objects (the "tail") and culture translations are handled here too.

use std::collections::{BTreeMap, HashMap, HashSet};

use bicraft_locale::calendar::civil_from_days;
use bicraft_model::Annotation;
use serde_json::{Map, Value, json};

use crate::apply::{Cell, Editor};
use crate::codes;
use crate::input::{MetaValue, MetadataTables, Row};
use crate::spec::{self, Back, Child, Kind, Link, Order, Prop, Shape, Times, Ty};
use crate::{Error, UnsupportedChange};

type Obj = Map<String, Value>;

/// Rows (with their positions) by owner ID, in ID order.
type ByOwner<'a> = HashMap<i64, Vec<(usize, Row<'a>)>>;

/// Depth limit for nested JSON (the spec is at most four levels deep).
const MAX_DEPTH: usize = 16;

/// The owner of a child collection, as the reader sees it.
#[derive(Clone, Copy)]
pub(crate) struct Owner<'a> {
    pub row: Option<Row<'a>>,
    pub id: Option<i64>,
    /// The table that column, measure and hierarchy references of the children resolve in.
    pub scope: Option<i64>,
}

/// Indexes over the base rows.
pub(crate) struct Lookups<'a> {
    pub model_id: Option<i64>,
    model_name: String,
    /// User tables of the model, in ID order.
    tables: Vec<(i64, String)>,
    table_names: HashMap<i64, String>,
    /// Every column: ID → (table ID, name); per table in ID order.
    pub columns: HashMap<i64, (i64, String)>,
    table_columns: HashMap<i64, Vec<(i64, String)>>,
    pub measures: HashMap<i64, (i64, String)>,
    table_measures: HashMap<i64, Vec<(i64, String)>>,
    pub hierarchies: HashMap<i64, (i64, String)>,
    table_hierarchies: HashMap<i64, Vec<(i64, String)>>,
    /// Every set: ID → (table ID, name); per table in ID order.
    sets: HashMap<i64, (i64, String)>,
    table_sets: HashMap<i64, Vec<(i64, String)>>,
    /// Hierarchy ID → its levels (ID, name) by ordinal.
    levels: HashMap<i64, Vec<(i64, String)>>,
    pub relationships: HashMap<i64, String>,
    perspectives: Vec<(i64, String)>,
    roles: Vec<(i64, String)>,
    expressions: Vec<(i64, String)>,
    annotations: HashMap<(i64, i64), Vec<(usize, Row<'a>)>>,
    extended: HashMap<(i64, i64), Vec<(usize, Row<'a>)>>,
    changed: HashMap<(i64, i64), Vec<(usize, Row<'a>)>>,
    /// (table, foreign key) → owner ID → rows in ID order.
    fk: HashMap<(&'static str, &'static str), ByOwner<'a>>,
    /// table → ID → row.
    by_id: HashMap<&'static str, HashMap<i64, (usize, Row<'a>)>>,
}

/// Rows of `table` with their positions, sorted by `ID` (rows without one last).
fn sorted<'a>(t: &'a MetadataTables, table: &str) -> Vec<(usize, Row<'a>)> {
    let mut rows: Vec<(usize, Row<'a>)> = t.rows(table).iter().map(Row).enumerate().collect();
    rows.sort_by_key(|(_, r)| r.id().unwrap_or(i64::MAX));
    rows
}

/// `ExplicitName`, falling back to `InferredName` (calculated-table columns) and `Name`.
pub(crate) fn column_name(r: Row<'_>) -> String {
    r.text("ExplicitName")
        .or_else(|| r.text("InferredName"))
        .or_else(|| r.text("Name"))
        .unwrap_or_default()
        .to_string()
}

fn eq_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn find_id(list: &[(i64, String)], name: &str) -> Option<i64> {
    list.iter().find(|(_, n)| eq_name(n, name)).map(|(id, _)| *id)
}

/// Every (table, foreign key) and pointer table the spec uses.
fn spec_links(
    children: &[Child],
    fks: &mut HashSet<(&'static str, &'static str)>,
    ids: &mut HashSet<&'static str>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    for c in children {
        match c.link {
            Link::Fk(col) => {
                fks.insert((c.kind.table, col));
            }
            Link::Pointer { .. } => {
                ids.insert(c.kind.table);
            }
        }
        spec_links(c.kind.children, fks, ids, depth.saturating_add(1));
    }
}

/// Every child collection hanging off a typed object.
pub(crate) static ROOTS: [&[Child]; 8] = [
    spec::MODEL_CHILDREN,
    spec::TABLE_CHILDREN,
    spec::EXPRESSION_CHILDREN,
    spec::COLUMN_CHILDREN,
    spec::MEASURE_CHILDREN,
    spec::PARTITION_CHILDREN,
    spec::ROLE_CHILDREN,
    spec::TABLE_PERMISSION_CHILDREN,
];

impl<'a> Lookups<'a> {
    pub(crate) fn new(t: &'a MetadataTables) -> Self {
        let model = sorted(t, "Model").into_iter().next().map(|(_, r)| r);
        let model_id = model.and_then(|m| m.id());
        let in_model = |r: &Row<'_>| match (model_id, r.int("ModelID")) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        };
        let mut l = Lookups {
            model_id,
            model_name: model
                .and_then(|m| m.string("Name"))
                .unwrap_or_else(|| "Model".to_string()),
            tables: Vec::new(),
            table_names: HashMap::new(),
            columns: HashMap::new(),
            table_columns: HashMap::new(),
            measures: HashMap::new(),
            table_measures: HashMap::new(),
            hierarchies: HashMap::new(),
            table_hierarchies: HashMap::new(),
            sets: HashMap::new(),
            table_sets: HashMap::new(),
            levels: HashMap::new(),
            relationships: HashMap::new(),
            perspectives: Vec::new(),
            roles: Vec::new(),
            expressions: Vec::new(),
            annotations: HashMap::new(),
            extended: HashMap::new(),
            changed: HashMap::new(),
            fk: HashMap::new(),
            by_id: HashMap::new(),
        };
        for (_, r) in sorted(t, "Table") {
            if in_model(&r)
                && r.int("SystemFlags").unwrap_or(0) & 1 == 0
                && let Some(id) = r.id()
            {
                let name = r.string("Name").unwrap_or_default();
                l.table_names.insert(id, name.clone());
                l.tables.push((id, name));
            }
        }
        for (_, r) in sorted(t, "Column") {
            if let (Some(id), Some(table)) = (r.id(), r.int("TableID")) {
                let name = column_name(r);
                l.columns.insert(id, (table, name.clone()));
                l.table_columns.entry(table).or_default().push((id, name));
            }
        }
        for (_, r) in sorted(t, "Measure") {
            if let (Some(id), Some(table)) = (r.id(), r.int("TableID")) {
                let name = r.string("Name").unwrap_or_default();
                l.measures.insert(id, (table, name.clone()));
                l.table_measures.entry(table).or_default().push((id, name));
            }
        }
        for (_, r) in sorted(t, "Hierarchy") {
            if let (Some(id), Some(table)) = (r.id(), r.int("TableID")) {
                let name = r.string("Name").unwrap_or_default();
                l.hierarchies.insert(id, (table, name.clone()));
                l.table_hierarchies.entry(table).or_default().push((id, name));
            }
        }
        for (_, r) in sorted(t, "Set") {
            if let (Some(id), Some(table)) = (r.id(), r.int("TableID")) {
                let name = r.string("Name").unwrap_or_default();
                l.sets.insert(id, (table, name.clone()));
                l.table_sets.entry(table).or_default().push((id, name));
            }
        }
        let mut levels: Vec<Row<'a>> = sorted(t, "Level").into_iter().map(|(_, r)| r).collect();
        levels.sort_by_key(|r| (r.int("Ordinal").unwrap_or(i64::MAX), r.id().unwrap_or(i64::MAX)));
        for r in levels {
            if let (Some(id), Some(h)) = (r.id(), r.int("HierarchyID")) {
                l.levels
                    .entry(h)
                    .or_default()
                    .push((id, r.string("Name").unwrap_or_default()));
            }
        }
        for (_, r) in sorted(t, "Relationship") {
            if let (Some(id), Some(name)) = (r.id(), r.text("Name")) {
                l.relationships.insert(id, name.to_string());
            }
        }
        for (table, list) in [
            ("Perspective", &mut l.perspectives),
            ("Role", &mut l.roles),
            ("Expression", &mut l.expressions),
        ] {
            for (_, r) in sorted(t, table) {
                if in_model(&r)
                    && let (Some(id), Some(name)) = (r.id(), r.text("Name"))
                {
                    list.push((id, name.to_string()));
                }
            }
        }
        for (table, map) in [
            ("Annotation", &mut l.annotations),
            ("ExtendedProperty", &mut l.extended),
            ("ChangedProperty", &mut l.changed),
        ] {
            for (pos, r) in sorted(t, table) {
                if let (Some(k), Some(o)) = (r.int("ObjectType"), r.int("ObjectID")) {
                    map.entry((k, o)).or_default().push((pos, r));
                }
            }
        }
        let mut fks = HashSet::new();
        let mut ids = HashSet::new();
        for root in ROOTS {
            spec_links(root, &mut fks, &mut ids, 0);
        }
        for (table, col) in fks {
            let mut map: ByOwner<'a> = HashMap::new();
            for (pos, r) in sorted(t, table) {
                if let Some(k) = r.int(col) {
                    map.entry(k).or_default().push((pos, r));
                }
            }
            l.fk.insert((table, col), map);
        }
        for table in ids {
            let mut map = HashMap::new();
            for (pos, r) in sorted(t, table) {
                if let Some(id) = r.id() {
                    map.entry(id).or_insert((pos, r));
                }
            }
            l.by_id.insert(table, map);
        }
        l
    }

    // --- names --------------------------------------------------------------------------

    /// Records that measure `id` is now named `name` (renamed by the edit being applied), so
    /// nested objects that name it (perspectives, translations) resolve the new name.
    pub(crate) fn rename_relationship(&mut self, old: &str, new: &str) {
        for n in self.relationships.values_mut().filter(|n| n.as_str() == old) {
            *n = new.to_string();
        }
    }

    pub(crate) fn rename_measure(&mut self, id: i64, name: &str) {
        let Some((table, n)) = self.measures.get_mut(&id) else {
            return;
        };
        *n = name.to_string();
        let table = *table;
        for (mid, n) in self.table_measures.get_mut(&table).into_iter().flatten() {
            if *mid == id {
                *n = name.to_string();
            }
        }
    }

    /// Records a measure the edit being applied adds, so nested objects (perspectives,
    /// translations) can name it.
    pub(crate) fn add_measure(&mut self, id: i64, table: i64, name: &str) {
        self.measures.insert(id, (table, name.to_string()));
        self.table_measures
            .entry(table)
            .or_default()
            .push((id, name.to_string()));
    }

    pub(crate) fn table_name(&self, id: i64) -> Option<&str> {
        self.table_names.get(&id).map(String::as_str)
    }

    pub(crate) fn table_id(&self, name: &str) -> Option<i64> {
        find_id(&self.tables, name)
    }

    fn column_id(&self, table: Option<i64>, name: &str) -> Option<i64> {
        find_id(self.table_columns.get(&table?)?, name)
    }

    fn measure_id(&self, table: Option<i64>, name: &str) -> Option<i64> {
        if let Some(id) = table
            .and_then(|t| self.table_measures.get(&t))
            .and_then(|l| find_id(l, name))
        {
            return Some(id);
        }
        // Measure names are unique in a model.
        let mut it = self.measures.iter().filter(|(_, (_, n))| eq_name(n, name));
        let first = it.next().map(|(id, _)| *id);
        if it.next().is_some() { None } else { first }
    }

    fn hierarchy_id(&self, table: Option<i64>, name: &str) -> Option<i64> {
        find_id(self.table_hierarchies.get(&table?)?, name)
    }

    fn set_id(&self, table: Option<i64>, name: &str) -> Option<i64> {
        find_id(self.table_sets.get(&table?)?, name)
    }

    fn relationship_id(&self, name: &str) -> Option<i64> {
        self.relationships
            .iter()
            .find(|(_, n)| n.as_str() == name)
            .map(|(id, _)| *id)
    }

    // --- rows ---------------------------------------------------------------------------

    fn fk_rows(&self, table: &'static str, col: &'static str, owner: Option<i64>) -> &[(usize, Row<'a>)] {
        owner
            .and_then(|o| self.fk.get(&(table, col))?.get(&o))
            .map_or(&[], Vec::as_slice)
    }

    fn row_by_id(&self, table: &'static str, id: Option<i64>) -> Option<(usize, Row<'a>)> {
        self.by_id.get(table)?.get(&id?).copied()
    }

    /// The rows of a child collection of `owner`, in collection order.
    pub(crate) fn collection(&self, child: &Child, owner: &Owner<'_>) -> Vec<(usize, Row<'a>)> {
        let mut rows: Vec<(usize, Row<'a>)> = match child.link {
            Link::Fk(col) => self.fk_rows(child.kind.table, col, owner.id).to_vec(),
            Link::Pointer { owner_col, .. } => owner
                .row
                .and_then(|r| self.row_by_id(child.kind.table, r.reference(owner_col)))
                .into_iter()
                .collect(),
        };
        if let Some((col, v)) = child.filter {
            rows.retain(|(_, r)| r.int(col) == Some(v));
        }
        if child.kind.order == Order::Ordinal {
            rows.sort_by_key(|(_, r)| (r.int("Ordinal").unwrap_or(i64::MAX), r.id().unwrap_or(i64::MAX)));
        }
        rows
    }

    // --- reading ------------------------------------------------------------------------

    /// Typed annotations of an object, in ID order.
    pub(crate) fn annotations(&self, kind: i64, id: Option<i64>) -> Vec<Annotation> {
        id.and_then(|id| self.annotations.get(&(kind, id)))
            .into_iter()
            .flatten()
            .filter_map(|(_, r)| {
                Some(Annotation {
                    name: r.text("Name")?.to_string(),
                    value: r.raw_text("Value").unwrap_or_default().to_string(),
                })
            })
            .collect()
    }

    fn annotations_json(&self, kind: i64, id: i64) -> Vec<Value> {
        self.annotations(kind, Some(id))
            .into_iter()
            .map(|a| json!({ "name": a.name, "value": a.value }))
            .collect()
    }

    fn extended_json(&self, kind: i64, id: i64) -> Vec<Value> {
        self.extended
            .get(&(kind, id))
            .into_iter()
            .flatten()
            .filter_map(|(_, r)| extended_property(*r))
            .collect()
    }

    fn changed_json(&self, kind: i64, id: i64) -> Vec<Value> {
        self.changed
            .get(&(kind, id))
            .into_iter()
            .flatten()
            .filter_map(|(_, r)| Some(json!({ "property": r.text("Property")? })))
            .collect()
    }

    /// `annotations` (when `with_annotations`), `changedProperties` and `extendedProperties` of
    /// an object, added to `out` when not empty.
    pub(crate) fn tail(&self, out: &mut Obj, kind: i64, id: Option<i64>, with_annotations: bool) {
        let Some(id) = id else { return };
        if with_annotations {
            put_array(out, "annotations", self.annotations_json(kind, id));
        }
        put_array(out, "changedProperties", self.changed_json(kind, id));
        put_array(out, "extendedProperties", self.extended_json(kind, id));
    }

    /// The child collections of `owner` into `out`.
    pub(crate) fn children(&self, out: &mut Obj, children: &[Child], owner: &Owner<'_>) {
        self.children_at(out, children, owner, 0);
    }

    fn children_at(&self, out: &mut Obj, children: &[Child], owner: &Owner<'_>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for child in children {
            let key = child_key(child, owner.row);
            if child.shape == Shape::Translations {
                if let Some(v) = self.translations(owner.id) {
                    out.insert(key.into(), v);
                }
                continue;
            }
            let rows = self.collection(child, owner);
            match child.shape {
                Shape::Array => {
                    let items = rows
                        .iter()
                        .map(|(_, r)| self.item_at(child.kind, *r, owner.scope, depth))
                        .collect();
                    put_array(out, key, items);
                }
                Shape::Object => {
                    if let Some((_, r)) = rows.first() {
                        out.insert(key.into(), self.item_at(child.kind, *r, owner.scope, depth));
                    }
                }
                Shape::ScalarArray => {
                    let items = rows
                        .iter()
                        .filter_map(|(_, r)| self.scalar(child.kind, *r, owner.scope))
                        .collect();
                    put_array(out, key, items);
                }
                Shape::Scalar => {
                    if let Some(v) = rows
                        .iter()
                        .find_map(|(_, r)| self.scalar(child.kind, *r, owner.scope))
                    {
                        out.insert(key.into(), v);
                    }
                }
                Shape::Translations => {}
            }
        }
    }

    fn scalar(&self, kind: &Kind, r: Row<'_>, scope: Option<i64>) -> Option<Value> {
        let prop = kind.props.first()?;
        self.prop(prop, r, scope)
    }

    /// The JSON object of one row of `kind`.
    pub(crate) fn item(&self, kind: &Kind, r: Row<'_>, scope: Option<i64>) -> Value {
        self.item_at(kind, r, scope, 0)
    }

    fn item_at(&self, kind: &Kind, r: Row<'_>, scope: Option<i64>, depth: usize) -> Value {
        let mut o = Obj::new();
        for prop in kind.props {
            if let Some(v) = self.prop(prop, r, scope) {
                o.insert(prop.key.into(), v);
            }
        }
        let owner = Owner {
            row: Some(r),
            id: r.id(),
            scope: row_scope(kind, r).or(scope),
        };
        self.children_at(&mut o, kind.children, &owner, depth.saturating_add(1));
        if let Some(ot) = kind.object_type {
            self.tail(&mut o, ot, r.id(), kind.annotations);
        }
        Value::Object(o)
    }

    fn prop(&self, prop: &Prop, r: Row<'_>, _scope: Option<i64>) -> Option<Value> {
        let col = prop.col;
        match prop.ty {
            Ty::Text => r.text(col).map(|s| json!(s)),
            Ty::Expr => Some(json!(r.raw_text(col).unwrap_or_default())),
            Ty::Bool(d) => {
                let v = r.flag(col, d);
                (v != d).then(|| json!(v))
            }
            Ty::Int(d) => r.int(col).filter(|v| d != Some(*v)).map(|v| json!(v)),
            Ty::Code(f, d) => r.int(col).filter(|v| d != Some(*v)).and_then(f).map(|s| json!(s)),
            Ty::Json => r.text(col).map(parse_or_text),
            Ty::Content => {
                let raw = r.raw_text(col).unwrap_or_default();
                if r.int("ContentType") == Some(0) {
                    Some(json!(raw))
                } else {
                    Some(parse_or_text(raw))
                }
            }
            Ty::Table => r
                .reference(col)
                .and_then(|id| self.table_name(id))
                .map(|n| json!(n)),
            Ty::Column => self.columns.get(&r.reference(col)?).map(|(_, n)| json!(n)),
            Ty::ColumnPair => {
                let (t, c) = self.columns.get(&r.reference(col)?)?;
                Some(json!({ "table": self.table_name(*t)?, "column": c }))
            }
            Ty::Measure => self.measures.get(&r.reference(col)?).map(|(_, n)| json!(n)),
            Ty::Hierarchy => self.hierarchies.get(&r.reference(col)?).map(|(_, n)| json!(n)),
            Ty::HierarchyPair => {
                let (t, h) = self.hierarchies.get(&r.reference(col)?)?;
                Some(json!({ "table": self.table_name(*t)?, "hierarchy": h }))
            }
            Ty::Relationship => self.relationships.get(&r.reference(col)?).map(|n| json!(n)),
            Ty::Set => self.sets.get(&r.reference(col)?).map(|(_, n)| json!(n)),
        }
    }

    /// The `translations` object of a culture: `{model: {name, translated…, tables: [...],
    /// perspectives: [...], roles: [...], expressions: [...]}}`, with only the objects that
    /// have a translation (or a child that has one).
    fn translations(&self, culture: Option<i64>) -> Option<Value> {
        let rows = self.fk_rows("ObjectTranslation", "CultureID", culture);
        if rows.is_empty() {
            return None;
        }
        // (ObjectType, ObjectID) → (property code, value), in property order.
        let mut by_object: HashMap<(i64, i64), BTreeMap<i64, String>> = HashMap::new();
        for (_, r) in rows {
            if let (Some(t), Some(o), Some(p)) = (r.int("ObjectType"), r.int("ObjectID"), r.int("Property"))
                && codes::translated_property(p).is_some()
            {
                by_object
                    .entry((t, o))
                    .or_default()
                    .insert(p, r.raw_text("Value").unwrap_or_default().to_string());
            }
        }
        let props = |t: i64, id: i64, o: &mut Obj| -> bool {
            let Some(m) = by_object.get(&(t, id)) else {
                return false;
            };
            for (p, v) in m {
                if let Some(k) = codes::translated_property(*p) {
                    o.insert(k.into(), json!(v));
                }
            }
            true
        };
        let named = |name: &str| {
            let mut o = Obj::new();
            o.insert("name".into(), json!(name));
            o
        };
        let leaves = |t: i64, list: Option<&Vec<(i64, String)>>| -> Vec<Value> {
            list.into_iter()
                .flatten()
                .filter_map(|(id, name)| {
                    let mut o = named(name);
                    props(t, *id, &mut o).then_some(Value::Object(o))
                })
                .collect()
        };
        let mut model = named(&self.model_name);
        if let Some(id) = self.model_id {
            props(1, id, &mut model);
        }
        let mut tables = Vec::new();
        for (tid, tname) in &self.tables {
            let mut o = named(tname);
            let mut any = props(3, *tid, &mut o);
            for (key, kind, list) in [
                ("columns", 4, self.table_columns.get(tid)),
                ("measures", 8, self.table_measures.get(tid)),
            ] {
                let items = leaves(kind, list);
                any |= !items.is_empty();
                put_array(&mut o, key, items);
            }
            let hierarchies: Vec<Value> = self
                .table_hierarchies
                .get(tid)
                .into_iter()
                .flatten()
                .filter_map(|(hid, hname)| {
                    let mut h = named(hname);
                    let own = props(9, *hid, &mut h);
                    let levels = leaves(10, self.levels.get(hid));
                    let has_levels = !levels.is_empty();
                    put_array(&mut h, "levels", levels);
                    (own || has_levels).then_some(Value::Object(h))
                })
                .collect();
            any |= !hierarchies.is_empty();
            put_array(&mut o, "hierarchies", hierarchies);
            if any {
                tables.push(Value::Object(o));
            }
        }
        put_array(&mut model, "tables", tables);
        put_array(&mut model, "perspectives", leaves(29, Some(&self.perspectives)));
        put_array(&mut model, "roles", leaves(34, Some(&self.roles)));
        put_array(&mut model, "expressions", leaves(41, Some(&self.expressions)));
        Some(json!({ "model": model }))
    }
}

/// The table scope a row gives its children: its `Ty::Table` property, if it has one.
fn row_scope(kind: &Kind, r: Row<'_>) -> Option<i64> {
    kind.props
        .iter()
        .find(|p| matches!(p.ty, Ty::Table))
        .and_then(|p| r.reference(p.col))
}

/// The key of `child` for an owner row (`alt_key` when the owner holds 0 in its column).
fn child_key(child: &Child, owner: Option<Row<'_>>) -> &'static str {
    match child.alt_key {
        Some((col, alt)) if owner.and_then(|r| r.int(col)).unwrap_or(0) == 0 => alt,
        _ => child.key,
    }
}

fn extended_property(r: Row<'_>) -> Option<Value> {
    let name = r.text("Name")?;
    let raw = r.raw_text("Value").unwrap_or_default();
    let mut o = Obj::new();
    if r.int("Type") == Some(codes::EXTENDED_PROPERTY_JSON) {
        o.insert("type".into(), json!("json"));
        o.insert("name".into(), json!(name));
        o.insert("value".into(), parse_or_text(raw));
    } else {
        o.insert("name".into(), json!(name));
        o.insert("value".into(), json!(raw));
    }
    Some(Value::Object(o))
}

/// JSON text parsed, or kept as a string when it does not parse.
pub(crate) fn parse_or_text(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| json!(raw))
}

pub(crate) fn put_array(o: &mut Obj, key: &str, v: Vec<Value>) {
    if !v.is_empty() {
        o.insert(key.into(), Value::Array(v));
    }
}

/// OLE automation date (days since 1899-12-30, fraction = time of day) as
/// `yyyy-MM-ddTHH:mm:ss`; `None` for values outside years 1..=9999.
pub(crate) fn ole_date_value(v: Option<&MetaValue>) -> Option<Value> {
    let days = match v? {
        MetaValue::Real(f) => *f,
        MetaValue::Integer(i) => *i as f64,
        _ => return None,
    };
    ole_date(days).map(Value::String)
}

pub(crate) fn ole_date(days: f64) -> Option<String> {
    if !days.is_finite() || !(-693_593.0..=2_958_465.0).contains(&days) {
        return None;
    }
    let whole = days.floor();
    let secs = ((days - whole) * 86_400.0).round() as i64;
    // Days from 1899-12-30 to 1970-01-01: 25569.
    let unix_days = (whole as i64).checked_sub(25_569)?;
    let (y, m, d) = civil_from_days(unix_days);
    let (h, mi, s) = (secs / 3600 % 24, secs / 60 % 60, secs % 60);
    Some(format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}"))
}

// ---------------------------------------------------------------------------------------------
// Writing.

/// The owner of a child collection, as the writer sees it.
#[derive(Clone)]
pub(crate) struct WOwner<'a> {
    /// The owner's TMSCHEMA table (to update its pointer columns).
    pub table: &'static str,
    /// Its row in the base; `None` for a row created by this edit.
    pub row: Option<(usize, Row<'a>)>,
    pub id: i64,
    pub object_type: Option<i64>,
    pub scope: Option<i64>,
    /// Description for messages, e.g. `measure 'Sales'[Total]`.
    pub object: String,
}

impl<'a> WOwner<'a> {
    fn read(&self) -> Owner<'a> {
        Owner {
            row: self.row.map(|(_, r)| r),
            id: Some(self.id),
            scope: self.scope,
        }
    }
}

/// Applies differences of nested JSON to the rows.
pub(crate) struct Writer<'w, 'a> {
    pub ed: &'w mut Editor<'a>,
    pub look: &'w Lookups<'a>,
    pub unsupported: &'w mut Vec<UnsupportedChange>,
    /// The owner's stored data is written in this edit (a table added by a structural save):
    /// objects whose rows are stored data (calculation groups and items) may be added.
    pub stored_ok: bool,
}

/// The `extra` keys a typed owner's writer handles: its child collections and the tail.
pub(crate) fn handles(children: &[Child], key: &str) -> bool {
    key == "extendedProperties"
        || key == "changedProperties"
        || children
            .iter()
            .any(|c| c.key == key || c.alt_key.is_some_and(|(_, k)| k == key))
}

/// Values of `key` (or its alternate key) in an object.
fn get_child<'v>(o: &'v Obj, child: &Child) -> Option<&'v Value> {
    o.get(child.key)
        .or_else(|| child.alt_key.and_then(|(_, k)| o.get(k)))
}

fn same_key(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (Some(Value::String(x)), Some(Value::String(y))) => eq_name(x, y),
        (x, y) => x == y,
    }
}

impl<'w, 'a> Writer<'w, 'a> {
    fn refuse(&mut self, object: &str, what: impl Into<String>) {
        self.unsupported.push(UnsupportedChange {
            what: what.into(),
            object: object.to_string(),
        });
    }

    /// Applies the differences between `old` and `new` (the owner's JSON) for every child
    /// collection in `children` and the owner's tail (extended and changed properties, plus
    /// annotations when `annotations`).
    pub(crate) fn update(
        &mut self,
        children: &[Child],
        owner: &WOwner<'a>,
        old: &Obj,
        new: &Obj,
        annotations: bool,
    ) -> Result<(), Error> {
        self.update_at(children, owner, old, new, annotations, 0)
    }

    fn update_at(
        &mut self,
        children: &[Child],
        owner: &WOwner<'a>,
        old: &Obj,
        new: &Obj,
        annotations: bool,
        depth: usize,
    ) -> Result<(), Error> {
        if depth > MAX_DEPTH {
            self.refuse(&owner.object, "nested too deeply");
            return Ok(());
        }
        for child in children {
            let (o, n) = (get_child(old, child), get_child(new, child));
            if o != n {
                self.collection(child, owner, n, depth)?;
            }
        }
        if let Some(ot) = owner.object_type {
            self.tail(ot, owner.id, old, new, annotations, &owner.object)?;
        }
        Ok(())
    }

    fn collection(
        &mut self,
        child: &Child,
        owner: &WOwner<'a>,
        new: Option<&Value>,
        depth: usize,
    ) -> Result<(), Error> {
        let kind = child.kind;
        let object = format!("{} {}", owner.object, child.key);
        if child.shape == Shape::Translations {
            return self.translations(owner, new, &object);
        }
        let rows = self.look.collection(child, &owner.read());
        match child.shape {
            Shape::Array | Shape::ScalarArray => {
                let items: Vec<Value> = match new {
                    None => Vec::new(),
                    Some(Value::Array(a)) => a.clone(),
                    Some(_) => {
                        self.refuse(&object, "not an array");
                        return Ok(());
                    }
                };
                let scalar = child.shape == Shape::ScalarArray;
                let old: Vec<(usize, Row<'a>, Value)> = rows
                    .iter()
                    .map(|(p, r)| {
                        let v = if scalar {
                            self.look.scalar(kind, *r, owner.scope).unwrap_or(Value::Null)
                        } else {
                            self.look.item(kind, *r, owner.scope)
                        };
                        (*p, *r, v)
                    })
                    .collect();
                let mut used = vec![false; old.len()];
                let mut added = Vec::new();
                for (i, item) in items.iter().enumerate() {
                    let found = old.iter().enumerate().position(|(j, (_, _, v))| {
                        !used.get(j).copied().unwrap_or(true)
                            && match (scalar, kind.key) {
                                (true, _) => same_key(Some(v), Some(item)),
                                (false, Some(k)) => same_key(v.get(k), item.get(k)),
                                (false, None) => j == i,
                            }
                    });
                    match found {
                        Some(j) => {
                            if let Some(u) = used.get_mut(j) {
                                *u = true;
                            }
                            if let Some((p, r, v)) = old.get(j)
                                && v != item
                                && !scalar
                            {
                                self.update_item(kind, *p, *r, v, item, owner.scope, &object, depth)?;
                            }
                        }
                        None => added.push(item),
                    }
                }
                for (j, (p, r, _)) in old.iter().enumerate() {
                    if !used.get(j).copied().unwrap_or(true) {
                        self.delete_item(kind, *p, *r, &object, depth)?;
                    }
                }
                for item in added {
                    let item = if scalar {
                        scalar_item(kind, item)
                    } else {
                        item.clone()
                    };
                    self.insert_item(child, owner, &item, &object, depth)?;
                }
            }
            Shape::Object | Shape::Scalar => {
                let scalar = child.shape == Shape::Scalar;
                let old = rows.first().copied();
                match (old, new) {
                    (Some((p, r)), Some(n)) => {
                        if scalar {
                            let v = self.look.scalar(kind, r, owner.scope);
                            if v.as_ref() != Some(n) {
                                self.delete_item(kind, p, r, &object, depth)?;
                                self.insert_item(child, owner, &scalar_item(kind, n), &object, depth)?;
                            }
                        } else {
                            let v = self.look.item(kind, r, owner.scope);
                            self.update_item(kind, p, r, &v, n, owner.scope, &object, depth)?;
                        }
                    }
                    (Some((p, r)), None) => {
                        if self.delete_item(kind, p, r, &object, depth)?
                            && let Link::Pointer { owner_col, .. } = child.link
                            && let Some((pos, _)) = owner.row
                        {
                            self.ed.set(owner.table, pos, owner_col, MetaValue::Integer(0))?;
                        }
                    }
                    (None, Some(n)) => {
                        let item = if scalar { scalar_item(kind, n) } else { n.clone() };
                        if let Some(id) = self.insert_item(child, owner, &item, &object, depth)?
                            && let Link::Pointer { owner_col, .. } = child.link
                        {
                            match owner.row {
                                Some((pos, _)) => {
                                    self.ed.set(owner.table, pos, owner_col, MetaValue::Integer(id))?
                                }
                                None => self.refuse(&object, "added to an object created by the same edit"),
                            }
                        }
                    }
                    (None, None) => {}
                }
            }
            Shape::Translations => {}
        }
        Ok(())
    }

    /// Keys of `new` that `kind` does not map.
    fn unknown_keys(&mut self, kind: &Kind, new: &Obj, object: &str) -> bool {
        let mut ok = true;
        for k in new.keys() {
            let known = kind.props.iter().any(|p| p.key == k)
                || kind
                    .children
                    .iter()
                    .any(|c| c.key == k || c.alt_key.is_some_and(|(_, a)| a == k))
                || (kind.object_type.is_some()
                    && (k == "extendedProperties"
                        || k == "changedProperties"
                        || (kind.annotations && k == "annotations")));
            if !known {
                self.refuse(
                    object,
                    format!("property `{k}` of a {} is not supported", kind.what),
                );
                ok = false;
            }
        }
        ok
    }

    #[allow(clippy::too_many_arguments)]
    fn update_item(
        &mut self,
        kind: &Kind,
        pos: usize,
        row: Row<'a>,
        old: &Value,
        new: &Value,
        scope: Option<i64>,
        object: &str,
        depth: usize,
    ) -> Result<(), Error> {
        if old == new {
            return Ok(());
        }
        let (Some(old), Some(new)) = (old.as_object(), new.as_object()) else {
            self.refuse(object, format!("a {} that is not an object", kind.what));
            return Ok(());
        };
        if !self.unknown_keys(kind, new, object) {
            return Ok(());
        }
        let item_scope = kind
            .props
            .iter()
            .find(|p| matches!(p.ty, Ty::Table))
            .and_then(|p| new.get(p.key))
            .and_then(Value::as_str)
            .and_then(|n| self.look.table_id(n))
            .or_else(|| row_scope(kind, row))
            .or(scope);
        let mut expression_changed = false;
        for prop in kind.props {
            if old.get(prop.key) == new.get(prop.key) {
                continue;
            }
            if !prop.write {
                self.refuse(object, format!("`{}` of a {} changed", prop.key, kind.what));
                continue;
            }
            match self.value(prop, new.get(prop.key), new, scope) {
                Ok(v) => {
                    self.ed.set(kind.table, pos, prop.col, v)?;
                    expression_changed |= matches!(prop.ty, Ty::Expr);
                }
                Err(e) => self.refuse(object, format!("`{}`: {e}", prop.key)),
            }
        }
        if expression_changed {
            self.ed.set_if_present(kind.table, pos, "State", 1.into())?;
            self.ed
                .set_if_present(kind.table, pos, "ErrorMessage", MetaValue::Null)?;
        }
        let Some(id) = row.id() else {
            self.refuse(object, format!("a {} without an ID", kind.what));
            return Ok(());
        };
        let owner = WOwner {
            table: kind.table,
            row: Some((pos, row)),
            id,
            object_type: kind.object_type,
            scope: item_scope,
            object: object.to_string(),
        };
        self.update_at(
            kind.children,
            &owner,
            old,
            new,
            kind.annotations,
            depth.saturating_add(1),
        )
    }

    /// Converts a JSON value of `prop` (absent: `None`) into a cell value.
    fn value(
        &self,
        prop: &Prop,
        v: Option<&Value>,
        item: &Obj,
        scope: Option<i64>,
    ) -> Result<MetaValue, String> {
        let text = |v: Option<&Value>| -> Result<Option<String>, String> {
            match v {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(s)) => Ok(Some(s.clone())),
                Some(_) => Err("not a string".into()),
            }
        };
        let name = |v: Option<&Value>| text(v);
        let not_found = |what: &str, n: &str| format!("no {what} named '{n}'");
        Ok(match prop.ty {
            Ty::Text => text(v)?
                .filter(|s| !s.is_empty())
                .map_or(MetaValue::Null, MetaValue::Text),
            Ty::Expr => MetaValue::Text(text(v)?.unwrap_or_default()),
            Ty::Bool(d) => match v {
                None => MetaValue::from(d),
                Some(Value::Bool(b)) => MetaValue::from(*b),
                Some(_) => return Err("not a boolean".into()),
            },
            Ty::Int(d) => match v {
                None => MetaValue::Integer(d.ok_or("required")?),
                Some(x) => MetaValue::Integer(x.as_i64().ok_or("not an integer")?),
            },
            Ty::Code(f, d) => match text(v)? {
                None => MetaValue::Integer(d.ok_or("required")?),
                Some(s) => {
                    MetaValue::Integer(codes::code_of(f, &s).ok_or_else(|| format!("unknown value '{s}'"))?)
                }
            },
            Ty::Json => match v {
                None | Some(Value::Null) => MetaValue::Null,
                Some(Value::String(s)) => MetaValue::Text(s.clone()),
                Some(x) => MetaValue::Text(pretty(x)),
            },
            Ty::Content => {
                let xml = item.get("contentType").and_then(Value::as_str) == Some("xml");
                match v {
                    None | Some(Value::Null) => MetaValue::Text(String::new()),
                    Some(Value::String(s)) => MetaValue::Text(s.clone()),
                    Some(_) if xml => return Err("XML content must be a string".into()),
                    Some(x) => MetaValue::Text(pretty(x)),
                }
            }
            Ty::Table => match name(v)? {
                None => MetaValue::Integer(0),
                Some(n) => MetaValue::Integer(self.look.table_id(&n).ok_or_else(|| not_found("table", &n))?),
            },
            Ty::Column => match name(v)? {
                None => MetaValue::Integer(0),
                Some(n) => MetaValue::Integer(
                    self.look
                        .column_id(scope, &n)
                        .ok_or_else(|| not_found("column", &n))?,
                ),
            },
            Ty::Measure => match name(v)? {
                None => MetaValue::Integer(0),
                Some(n) => MetaValue::Integer(
                    self.look
                        .measure_id(scope, &n)
                        .ok_or_else(|| not_found("measure", &n))?,
                ),
            },
            Ty::Hierarchy => match name(v)? {
                None => MetaValue::Integer(0),
                Some(n) => MetaValue::Integer(
                    self.look
                        .hierarchy_id(scope, &n)
                        .ok_or_else(|| not_found("hierarchy", &n))?,
                ),
            },
            Ty::Set => match name(v)? {
                None => MetaValue::Integer(0),
                Some(n) => {
                    MetaValue::Integer(self.look.set_id(scope, &n).ok_or_else(|| not_found("set", &n))?)
                }
            },
            Ty::Relationship => match name(v)? {
                None => MetaValue::Integer(0),
                Some(n) => MetaValue::Integer(
                    self.look
                        .relationship_id(&n)
                        .ok_or_else(|| not_found("relationship", &n))?,
                ),
            },
            Ty::ColumnPair | Ty::HierarchyPair => match v {
                None | Some(Value::Null) => MetaValue::Integer(0),
                Some(Value::Object(o)) => {
                    let t = o.get("table").and_then(Value::as_str).ok_or("missing `table`")?;
                    let tid = self.look.table_id(t).ok_or_else(|| not_found("table", t))?;
                    let (key, what) = if matches!(prop.ty, Ty::ColumnPair) {
                        ("column", "column")
                    } else {
                        ("hierarchy", "hierarchy")
                    };
                    let n = o
                        .get(key)
                        .and_then(Value::as_str)
                        .ok_or_else(|| format!("missing `{key}`"))?;
                    let id = if matches!(prop.ty, Ty::ColumnPair) {
                        self.look.column_id(Some(tid), n)
                    } else {
                        self.look.hierarchy_id(Some(tid), n)
                    };
                    MetaValue::Integer(id.ok_or_else(|| not_found(what, n))?)
                }
                Some(_) => return Err("not an object".into()),
            },
        })
    }

    /// Adds one item (and its children) under `owner`; the new row's ID, or `None` when it was
    /// refused.
    fn insert_item(
        &mut self,
        child: &Child,
        owner: &WOwner<'a>,
        item: &Value,
        object: &str,
        depth: usize,
    ) -> Result<Option<i64>, Error> {
        let kind = child.kind;
        let stored = matches!(kind.table, "CalculationGroup" | "CalculationItem");
        if !kind.insert && !(self.stored_ok && stored) {
            self.refuse(object, format!("{} added", kind.what));
            return Ok(None);
        }
        let mut links: Vec<(&'static str, MetaValue)> = match child.link {
            Link::Fk(col) => vec![(col, MetaValue::Integer(owner.id))],
            Link::Pointer { back, .. } => match back {
                Back::Column(col) => vec![(col, MetaValue::Integer(owner.id))],
                Back::Object => match owner.object_type {
                    Some(ot) => vec![
                        ("ObjectID", MetaValue::Integer(owner.id)),
                        ("ObjectType", MetaValue::Integer(ot)),
                    ],
                    None => {
                        self.refuse(object, "owner without an object type");
                        return Ok(None);
                    }
                },
            },
        };
        if let Some((col, v)) = child.filter {
            links.push((col, MetaValue::Integer(v)));
        }
        self.insert_row(kind, links, item, owner.scope, object, depth)
    }

    /// Adds a row of `kind` with the given link cells, its children and its tail.
    fn insert_row(
        &mut self,
        kind: &Kind,
        links: Vec<(&'static str, MetaValue)>,
        item: &Value,
        scope: Option<i64>,
        object: &str,
        depth: usize,
    ) -> Result<Option<i64>, Error> {
        if depth > MAX_DEPTH {
            self.refuse(object, "nested too deeply");
            return Ok(None);
        }
        let Some(obj) = item.as_object() else {
            self.refuse(object, format!("a {} that is not an object", kind.what));
            return Ok(None);
        };
        if !self.unknown_keys(kind, obj, object) {
            return Ok(None);
        }
        let item_scope = kind
            .props
            .iter()
            .find(|p| matches!(p.ty, Ty::Table))
            .and_then(|p| obj.get(p.key))
            .and_then(Value::as_str)
            .and_then(|n| self.look.table_id(n))
            .or(scope);
        let mut cells: Vec<(&str, Cell)> = Vec::new();
        let mut ok = true;
        for prop in kind.props {
            match self.value(prop, obj.get(prop.key), obj, scope) {
                Ok(v) => cells.push((prop.col, Cell::User(v))),
                Err(e) => {
                    self.refuse(object, format!("`{}`: {e}", prop.key));
                    ok = false;
                }
            }
        }
        if !ok {
            return Ok(None);
        }
        let id = self.ed.alloc_id()?;
        let now = self.ed.now;
        cells.insert(0, ("ID", Cell::User(id.into())));
        for (col, v) in links {
            cells.push((col, Cell::User(v)));
        }
        for (col, v) in kind.fixed {
            cells.push((col, Cell::Engine((*v).into())));
        }
        match kind.times {
            Times::None => {}
            Times::Modified => cells.push(("ModifiedTime", Cell::Engine(now.into()))),
            Times::Structure => {
                cells.push(("ModifiedTime", Cell::Engine(now.into())));
                cells.push(("StructureModifiedTime", Cell::Engine(now.into())));
            }
        }
        let me = WOwner {
            table: kind.table,
            row: None,
            id,
            object_type: kind.object_type,
            scope: item_scope,
            object: object.to_string(),
        };
        for child in kind.children {
            let value = get_child(obj, child);
            let pointer = match child.link {
                Link::Pointer { owner_col, .. } => Some(owner_col),
                Link::Fk(_) => None,
            };
            let mut child_id = None;
            match (child.shape, value) {
                (Shape::Translations, Some(v)) => self.translations(&me, Some(v), object)?,
                (Shape::Array | Shape::ScalarArray, Some(Value::Array(items))) => {
                    for it in items {
                        let it = if child.shape == Shape::ScalarArray {
                            scalar_item(child.kind, it)
                        } else {
                            it.clone()
                        };
                        self.insert_item(child, &me, &it, object, depth.saturating_add(1))?;
                    }
                }
                (Shape::Object | Shape::Scalar, Some(v)) => {
                    let it = if child.shape == Shape::Scalar {
                        scalar_item(child.kind, v)
                    } else {
                        v.clone()
                    };
                    child_id = self.insert_item(child, &me, &it, object, depth.saturating_add(1))?;
                }
                (_, None) => {}
                (_, Some(_)) => self.refuse(object, format!("`{}` has the wrong shape", child.key)),
            }
            if let Some(col) = pointer {
                cells.push((col, Cell::User(child_id.unwrap_or(0).into())));
            }
        }
        self.ed.add(kind.table, cells);
        if let Some(ot) = kind.object_type {
            let empty = Obj::new();
            self.tail(ot, id, &empty, obj, kind.annotations, object)?;
        }
        Ok(Some(id))
    }

    /// Removes a row of `kind` with its children and dependents; `false` when refused.
    fn delete_item(
        &mut self,
        kind: &Kind,
        pos: usize,
        row: Row<'a>,
        object: &str,
        depth: usize,
    ) -> Result<bool, Error> {
        if !kind.delete {
            self.refuse(object, format!("{} removed", kind.what));
            return Ok(false);
        }
        let id = row.id();
        if let Some(id) = id {
            for (table, col) in kind.referenced_by {
                // Rows this edit removes too (a perspective's entry for a removed set) no
                // longer count.
                let users = self.ed.find(table, |r| r.int(col) == Some(id));
                if users.iter().any(|&pos| !self.ed.is_deleted(table, pos)) {
                    self.refuse(
                        object,
                        format!("{} removed while `{table}` rows use it", kind.what),
                    );
                    return Ok(false);
                }
            }
        }
        self.delete_rows(kind, pos, row, depth);
        Ok(true)
    }

    fn delete_rows(&mut self, kind: &Kind, pos: usize, row: Row<'a>, depth: usize) {
        self.ed.delete(kind.table, pos);
        let id = row.id();
        if let (Some(ot), Some(id)) = (kind.object_type, id) {
            self.ed.delete_dependents(ot, id);
        }
        if depth > MAX_DEPTH {
            return;
        }
        let owner = Owner {
            row: Some(row),
            id,
            scope: None,
        };
        for child in kind.children {
            for (p, r) in self.look.collection(child, &owner) {
                self.delete_rows(child.kind, p, r, depth.saturating_add(1));
            }
        }
    }

    /// Removes the child collections of a typed object that is being removed (e.g. the KPI of
    /// a removed measure).
    pub(crate) fn delete_children(&mut self, children: &[Child], owner: &WOwner<'a>) {
        let read = owner.read();
        for child in children {
            for (p, r) in self.look.collection(child, &read) {
                self.delete_rows(child.kind, p, r, 0);
            }
        }
    }

    /// Adds the child collections and tail of a typed object created by this edit; returns the
    /// pointer cells (`KPIID`, …) its row needs.
    pub(crate) fn insert_children(
        &mut self,
        children: &[Child],
        owner: &WOwner<'a>,
        new: &Obj,
        annotations: bool,
    ) -> Result<Vec<(&'static str, i64)>, Error> {
        let mut pointers = Vec::new();
        for child in children {
            let Some(v) = get_child(new, child) else {
                if let Link::Pointer { owner_col, .. } = child.link {
                    pointers.push((owner_col, 0));
                }
                continue;
            };
            match (child.shape, v) {
                (Shape::Array | Shape::ScalarArray, Value::Array(items)) => {
                    for it in items {
                        self.insert_item(child, owner, it, &owner.object, 1)?;
                    }
                }
                (Shape::Object, v) => {
                    let id = self.insert_item(child, owner, v, &owner.object, 1)?;
                    if let Link::Pointer { owner_col, .. } = child.link {
                        pointers.push((owner_col, id.unwrap_or(0)));
                    }
                }
                _ => self.refuse(&owner.object, format!("`{}` has the wrong shape", child.key)),
            }
        }
        if let Some(ot) = owner.object_type {
            self.tail(ot, owner.id, &Obj::new(), new, annotations, &owner.object)?;
        }
        Ok(pointers)
    }

    // --- tail -------------------------------------------------------------------------------

    fn tail(
        &mut self,
        ot: i64,
        id: i64,
        old: &Obj,
        new: &Obj,
        annotations: bool,
        object: &str,
    ) -> Result<(), Error> {
        if annotations && old.get("annotations") != new.get("annotations") {
            self.tail_list(TailKind::Annotation, ot, id, new.get("annotations"), object)?;
        }
        if old.get("extendedProperties") != new.get("extendedProperties") {
            self.tail_list(TailKind::Extended, ot, id, new.get("extendedProperties"), object)?;
        }
        if old.get("changedProperties") != new.get("changedProperties") {
            self.tail_list(TailKind::Changed, ot, id, new.get("changedProperties"), object)?;
        }
        Ok(())
    }

    fn tail_list(
        &mut self,
        kind: TailKind,
        ot: i64,
        id: i64,
        new: Option<&Value>,
        object: &str,
    ) -> Result<(), Error> {
        let (table, key) = kind.table_key();
        let items: Vec<Obj> = match new {
            None => Vec::new(),
            Some(Value::Array(a)) => {
                let mut out = Vec::new();
                for v in a {
                    match kind.parse(v) {
                        Some(o) => out.push(o),
                        None => {
                            self.refuse(object, format!("malformed `{}` entry", kind.json_key()));
                            return Ok(());
                        }
                    }
                }
                out
            }
            Some(_) => {
                self.refuse(object, format!("`{}` is not an array", kind.json_key()));
                return Ok(());
            }
        };
        let rows: Vec<(usize, Row<'a>)> = match kind {
            TailKind::Annotation => self.look.annotations.get(&(ot, id)),
            TailKind::Extended => self.look.extended.get(&(ot, id)),
            TailKind::Changed => self.look.changed.get(&(ot, id)),
        }
        .cloned()
        .unwrap_or_default();
        let mut used = vec![false; rows.len()];
        for item in &items {
            let name = item.get(key).and_then(Value::as_str).unwrap_or_default();
            let found = rows.iter().enumerate().position(|(j, (_, r))| {
                !used.get(j).copied().unwrap_or(true) && r.raw_text(key_col(kind)) == Some(name)
            });
            match found {
                Some(j) => {
                    if let Some(u) = used.get_mut(j) {
                        *u = true;
                    }
                    if let Some((pos, r)) = rows.get(j) {
                        for (col, v) in kind.cells(item) {
                            if r.get(col) != Some(&v) {
                                self.ed.set(table, *pos, col, v)?;
                            }
                        }
                    }
                }
                None => {
                    let new_id = self.ed.alloc_id()?;
                    let now = self.ed.now;
                    let mut cells: Vec<(&str, Cell)> = vec![
                        ("ID", Cell::User(new_id.into())),
                        ("ObjectID", Cell::User(id.into())),
                        ("ObjectType", Cell::User(ot.into())),
                    ];
                    for (col, v) in kind.cells(item) {
                        cells.push((col, Cell::User(v)));
                    }
                    if kind != TailKind::Changed {
                        cells.push(("ModifiedTime", Cell::Engine(now.into())));
                    }
                    self.ed.add(table, cells);
                }
            }
        }
        for (j, (pos, _)) in rows.iter().enumerate() {
            if !used.get(j).copied().unwrap_or(true) {
                self.ed.delete(table, *pos);
            }
        }
        Ok(())
    }

    // --- translations -----------------------------------------------------------------------

    fn translations(&mut self, culture: &WOwner<'a>, new: Option<&Value>, object: &str) -> Result<(), Error> {
        let mut desired: BTreeMap<(i64, i64, i64), String> = BTreeMap::new();
        if let Some(v) = new
            && let Err(e) = self.flatten_translations(v, &mut desired)
        {
            self.refuse(object, e);
            return Ok(());
        }
        let rows: Vec<(usize, Row<'a>)> = if culture.row.is_some() {
            self.look
                .fk_rows("ObjectTranslation", "CultureID", Some(culture.id))
                .to_vec()
        } else {
            Vec::new()
        };
        let mut seen = HashSet::new();
        for (pos, r) in rows {
            let key = (
                r.int("ObjectType").unwrap_or(0),
                r.int("ObjectID").unwrap_or(0),
                r.int("Property").unwrap_or(0),
            );
            match desired.get(&key) {
                Some(v) if seen.insert(key) => {
                    if r.raw_text("Value") != Some(v.as_str()) {
                        self.ed
                            .set("ObjectTranslation", pos, "Value", v.as_str().into())?;
                    }
                }
                // Not wanted any more, or a duplicate row for the same property.
                _ => self.ed.delete("ObjectTranslation", pos),
            }
        }
        for ((t, o, p), v) in desired {
            if seen.contains(&(t, o, p)) {
                continue;
            }
            let id = self.ed.alloc_id()?;
            let now = self.ed.now;
            self.ed.add(
                "ObjectTranslation",
                vec![
                    ("ID", Cell::User(id.into())),
                    ("CultureID", Cell::User(culture.id.into())),
                    ("ObjectID", Cell::User(o.into())),
                    ("ObjectType", Cell::User(t.into())),
                    ("Property", Cell::User(p.into())),
                    ("Value", Cell::User(v.into())),
                    ("ModifiedTime", Cell::Engine(now.into())),
                    ("Altered", Cell::Engine(0.into())),
                ],
            );
        }
        Ok(())
    }

    /// The `translations` tree as (ObjectType, ObjectID, Property) → value.
    fn flatten_translations(
        &self,
        v: &Value,
        out: &mut BTreeMap<(i64, i64, i64), String>,
    ) -> Result<(), String> {
        let root = v.as_object().ok_or("translations: not an object")?;
        for k in root.keys() {
            if k != "model" {
                return Err(format!("translations: unknown key `{k}`"));
            }
        }
        let Some(model) = root.get("model") else {
            return Ok(());
        };
        let model = model.as_object().ok_or("translations.model: not an object")?;
        let model_id = self.look.model_id.ok_or("the model has no ID")?;
        let props = |o: &Obj,
                     t: i64,
                     id: i64,
                     out: &mut BTreeMap<(i64, i64, i64), String>,
                     children: &[&str]|
         -> Result<(), String> {
            for (k, v) in o {
                if k == "name" || children.contains(&k.as_str()) {
                    continue;
                }
                let p = codes::code_of(codes::translated_property, k)
                    .ok_or_else(|| format!("translations: unknown property `{k}`"))?;
                let s = v
                    .as_str()
                    .ok_or_else(|| format!("translations: `{k}` is not a string"))?;
                out.insert((t, id, p), s.to_string());
            }
            Ok(())
        };
        let items = |o: &Obj, key: &str| -> Result<Vec<Obj>, String> {
            match o.get(key) {
                None => Ok(Vec::new()),
                Some(Value::Array(a)) => a
                    .iter()
                    .map(|x| {
                        x.as_object()
                            .cloned()
                            .ok_or_else(|| format!("translations: `{key}` entry is not an object"))
                    })
                    .collect(),
                Some(_) => Err(format!("translations: `{key}` is not an array")),
            }
        };
        let name_of = |o: &Obj| -> Result<String, String> {
            o.get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| "translations: an entry without a name".to_string())
        };
        props(
            model,
            1,
            model_id,
            out,
            &["tables", "perspectives", "roles", "expressions"],
        )?;
        for t in items(model, "tables")? {
            let name = name_of(&t)?;
            let tid = self
                .look
                .table_id(&name)
                .ok_or_else(|| format!("translations: no table '{name}'"))?;
            props(&t, 3, tid, out, &["columns", "measures", "hierarchies"])?;
            for c in items(&t, "columns")? {
                let n = name_of(&c)?;
                let id = self
                    .look
                    .column_id(Some(tid), &n)
                    .ok_or_else(|| format!("translations: no column '{name}'[{n}]"))?;
                props(&c, 4, id, out, &[])?;
            }
            for m in items(&t, "measures")? {
                let n = name_of(&m)?;
                let id = self
                    .look
                    .table_measures
                    .get(&tid)
                    .and_then(|l| find_id(l, &n))
                    .ok_or_else(|| format!("translations: no measure '{name}'[{n}]"))?;
                props(&m, 8, id, out, &[])?;
            }
            for h in items(&t, "hierarchies")? {
                let n = name_of(&h)?;
                let hid = self
                    .look
                    .hierarchy_id(Some(tid), &n)
                    .ok_or_else(|| format!("translations: no hierarchy '{name}'[{n}]"))?;
                props(&h, 9, hid, out, &["levels"])?;
                for l in items(&h, "levels")? {
                    let ln = name_of(&l)?;
                    let lid = self
                        .look
                        .levels
                        .get(&hid)
                        .and_then(|list| find_id(list, &ln))
                        .ok_or_else(|| format!("translations: no level '{ln}' in hierarchy '{n}'"))?;
                    props(&l, 10, lid, out, &[])?;
                }
            }
        }
        for (key, t, list) in [
            ("perspectives", 29, &self.look.perspectives),
            ("roles", 34, &self.look.roles),
            ("expressions", 41, &self.look.expressions),
        ] {
            for o in items(model, key)? {
                let n = name_of(&o)?;
                let id = find_id(list, &n).ok_or_else(|| format!("translations: no {key} entry '{n}'"))?;
                props(&o, t, id, out, &[])?;
            }
        }
        Ok(())
    }
}

/// The JSON item of a scalar-shaped collection entry: `{<its single property>: value}`.
fn scalar_item(kind: &Kind, v: &Value) -> Value {
    let mut o = Obj::new();
    if let Some(p) = kind.props.first() {
        o.insert(p.key.into(), v.clone());
    }
    Value::Object(o)
}

/// JSON as the engine stores it: indented by two spaces.
fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TailKind {
    Annotation,
    Extended,
    Changed,
}

/// The column that names a tail row.
fn key_col(kind: TailKind) -> &'static str {
    match kind {
        TailKind::Changed => "Property",
        _ => "Name",
    }
}

impl TailKind {
    fn table_key(self) -> (&'static str, &'static str) {
        match self {
            Self::Annotation => ("Annotation", "name"),
            Self::Extended => ("ExtendedProperty", "name"),
            Self::Changed => ("ChangedProperty", "property"),
        }
    }

    fn json_key(self) -> &'static str {
        match self {
            Self::Annotation => "annotations",
            Self::Extended => "extendedProperties",
            Self::Changed => "changedProperties",
        }
    }

    /// Validates one JSON entry.
    fn parse(self, v: &Value) -> Option<Obj> {
        let o = v.as_object()?;
        match self {
            Self::Annotation => {
                o.get("name")?.as_str().filter(|s| !s.is_empty())?;
                o.get("value")?.as_str()?;
                (o.len() == 2).then(|| o.clone())
            }
            Self::Extended => {
                o.get("name")?.as_str().filter(|s| !s.is_empty())?;
                let json = match o.get("type") {
                    None => false,
                    Some(t) => t.as_str()? == "json",
                };
                if json {
                    // TOM accepts only a JSON object here (checked on the engine).
                    o.get("value")?.as_object()?;
                } else {
                    o.get("value")?.as_str()?;
                }
                o.keys()
                    .all(|k| ["type", "name", "value"].contains(&k.as_str()))
                    .then(|| o.clone())
            }
            Self::Changed => {
                o.get("property")?.as_str().filter(|s| !s.is_empty())?;
                (o.len() == 1).then(|| o.clone())
            }
        }
    }

    /// The cells of a (validated) entry.
    fn cells(self, o: &Obj) -> Vec<(&'static str, MetaValue)> {
        let s = |k: &str| o.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        match self {
            Self::Annotation => vec![("Name", s("name").into()), ("Value", s("value").into())],
            Self::Extended => {
                let json = o.get("type").and_then(Value::as_str) == Some("json");
                let value = match o.get("value") {
                    Some(Value::String(v)) if !json => v.clone(),
                    Some(v) => pretty(v),
                    None => String::new(),
                };
                vec![
                    ("Name", s("name").into()),
                    ("Type", MetaValue::Integer(i64::from(json))),
                    ("Value", value.into()),
                ]
            }
            Self::Changed => vec![("Property", s("property").into())],
        }
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn ole_dates() {
        assert_eq!(ole_date(45292.0).as_deref(), Some("2024-01-01T00:00:00"));
        assert_eq!(ole_date(45658.5).as_deref(), Some("2025-01-01T12:00:00"));
        assert_eq!(ole_date(0.0).as_deref(), Some("1899-12-30T00:00:00"));
        assert_eq!(ole_date(f64::NAN), None);
        assert_eq!(ole_date(1e300), None);
    }

    #[test]
    fn every_kind_is_consistent() {
        for k in spec::ALL {
            assert!(!k.table.is_empty(), "{}", k.what);
            if let Some(key) = k.key {
                assert!(k.props.iter().any(|p| p.key == key), "{}: key {key}", k.what);
            }
            for c in k.children {
                assert!(!handles(&[], c.key));
                if matches!(c.shape, Shape::Scalar | Shape::ScalarArray) {
                    assert_eq!(c.kind.props.len(), 1, "{}", c.key);
                }
            }
        }
    }
}
