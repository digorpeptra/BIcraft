//! Model → TMSL. See the crate docs for the output conventions.

use bicraft_model::{
    Annotation, Column, Measure, NamedExpression, Partition, Properties, Relationship, Role, SemanticModel,
    Table, TablePermission,
};
use serde_json::{Map, Value};

pub(crate) fn database(m: &SemanticModel) -> Value {
    let mut model = Obj::default();
    model.string("culture", m.culture.as_deref());
    model.string("description", m.description.as_deref());
    model.list("tables", &m.tables, table);
    model.list("relationships", &m.relationships, relationship);
    model.list("expressions", &m.expressions, named_expression);
    model.list("roles", &m.roles, role);
    model.list("annotations", &m.annotations, annotation);

    let mut db = Obj::default();
    db.name("name", &m.name);
    if let Some(level) = m.compatibility_level {
        db.put("compatibilityLevel", Value::from(level));
    }
    db.put("model", model.finish(&m.extra));
    db.finish(&m.database_extra)
}

fn table(t: &Table) -> Value {
    let mut o = Obj::default();
    o.name("name", &t.name);
    o.string("description", t.description.as_deref());
    o.string("lineageTag", t.lineage_tag.as_deref());
    o.flag("isHidden", t.is_hidden, false);
    o.list("columns", &t.columns, column);
    o.list("measures", &t.measures, measure);
    o.list("partitions", &t.partitions, partition);
    o.list("annotations", &t.annotations, annotation);
    o.finish(&t.extra)
}

fn column(c: &Column) -> Value {
    let mut o = Obj::default();
    o.name("name", &c.name);
    if let Some(data_type) = c.data_type.as_tmsl() {
        o.put("dataType", Value::from(data_type));
    }
    if let Some(expression) = &c.expression {
        o.put("expression", expression_value(expression));
        // A column with a DAX expression is a calculated column in TOM.
        if !c.extra.contains_key("type") {
            o.put("type", Value::from("calculated"));
        }
    }
    o.string("sourceColumn", c.source_column.as_deref());
    o.string("formatString", c.format_string.as_deref());
    o.string("displayFolder", c.display_folder.as_deref());
    o.string("summarizeBy", c.summarize_by.as_deref());
    o.string("sortByColumn", c.sort_by_column.as_deref());
    o.string("dataCategory", c.data_category.as_deref());
    o.string("description", c.description.as_deref());
    o.string("lineageTag", c.lineage_tag.as_deref());
    o.flag("isHidden", c.is_hidden, false);
    o.list("annotations", &c.annotations, annotation);
    o.finish(&c.extra)
}

fn measure(m: &Measure) -> Value {
    let mut o = Obj::default();
    o.name("name", &m.name);
    o.required_expression("expression", &m.expression);
    o.string("formatString", m.format_string.as_deref());
    o.string("displayFolder", m.display_folder.as_deref());
    o.string("description", m.description.as_deref());
    o.string("lineageTag", m.lineage_tag.as_deref());
    o.flag("isHidden", m.is_hidden, false);
    o.list("annotations", &m.annotations, annotation);
    o.finish(&m.extra)
}

fn partition(p: &Partition) -> Value {
    let mut source = Obj::default();
    source.name("type", &p.source_type);
    if let Some(expression) = &p.expression {
        source.put("expression", expression_value(expression));
    }
    let source = source.finish(&p.source_extra);

    let mut o = Obj::default();
    o.name("name", &p.name);
    o.string("mode", p.mode.as_deref());
    if source.as_object().is_some_and(|s| !s.is_empty()) {
        o.put("source", source);
    }
    o.finish(&p.extra)
}

fn relationship(r: &Relationship) -> Value {
    let mut o = Obj::default();
    o.name("name", &r.name);
    o.name("fromTable", &r.from_table);
    o.name("fromColumn", &r.from_column);
    o.name("toTable", &r.to_table);
    o.name("toColumn", &r.to_column);
    o.flag("isActive", r.is_active, true);
    o.string("crossFilteringBehavior", r.cross_filtering_behavior.as_deref());
    o.string("fromCardinality", r.from_cardinality.as_deref());
    o.string("toCardinality", r.to_cardinality.as_deref());
    o.list("annotations", &r.annotations, annotation);
    o.finish(&r.extra)
}

fn named_expression(e: &NamedExpression) -> Value {
    let mut o = Obj::default();
    o.name("name", &e.name);
    o.name("kind", &e.kind);
    o.required_expression("expression", &e.expression);
    o.string("description", e.description.as_deref());
    o.string("lineageTag", e.lineage_tag.as_deref());
    o.list("annotations", &e.annotations, annotation);
    o.finish(&e.extra)
}

fn role(r: &Role) -> Value {
    let mut o = Obj::default();
    o.name("name", &r.name);
    o.string("modelPermission", r.model_permission.as_deref());
    o.list("tablePermissions", &r.table_permissions, table_permission);
    o.list("annotations", &r.annotations, annotation);
    o.finish(&r.extra)
}

fn table_permission(t: &TablePermission) -> Value {
    let mut o = Obj::default();
    o.name("name", &t.table);
    if let Some(filter) = &t.filter_expression {
        o.put("filterExpression", expression_value(filter));
    }
    o.finish(&t.extra)
}

fn annotation(a: &Annotation) -> Value {
    let mut o = Obj::default();
    o.put("name", Value::from(a.name.as_str()));
    o.put("value", Value::from(a.value.as_str()));
    o.finish(&Properties::new())
}

/// Desktop writes a multi-line expression as an array of its lines, and a one-line one as a string.
fn expression_value(text: &str) -> Value {
    if text.contains('\n') {
        Value::Array(text.split('\n').map(Value::from).collect())
    } else {
        Value::from(text)
    }
}

/// An object under construction: typed properties first, then `finish` merges `extra` and orders
/// the keys the way Desktop does.
#[derive(Default)]
struct Obj(Vec<(String, Value)>);

impl Obj {
    fn put(&mut self, key: &str, value: Value) {
        self.0.push((key.to_string(), value));
    }

    /// A required string: written unless empty (an empty one may come back from `extra`).
    fn name(&mut self, key: &str, value: &str) {
        if !value.is_empty() {
            self.put(key, Value::from(value));
        }
    }

    fn string(&mut self, key: &str, value: Option<&str>) {
        if let Some(value) = value {
            self.put(key, Value::from(value));
        }
    }

    fn required_expression(&mut self, key: &str, value: &str) {
        if !value.is_empty() {
            self.put(key, expression_value(value));
        }
    }

    fn flag(&mut self, key: &str, value: bool, default: bool) {
        if value != default {
            self.put(key, Value::Bool(value));
        }
    }

    fn list<T>(&mut self, key: &str, items: &[T], f: fn(&T) -> Value) {
        if !items.is_empty() {
            self.put(key, Value::Array(items.iter().map(f).collect()));
        }
    }

    /// Adds the `extra` properties a typed field did not already write (typed values win), then
    /// orders keys `name` first and the rest ordinally, as Desktop's `model.bim` does, at every
    /// depth: objects kept from `extra` (hierarchies, KPIs, `dataAccessOptions`, ...) are sorted
    /// too, array order is kept.
    fn finish(mut self, extra: &Properties) -> Value {
        for (key, value) in extra {
            if !self.0.iter().any(|(k, _)| k == key) {
                let mut value = value.clone();
                sort_keys(&mut value, 0);
                self.0.push((key.clone(), value));
            }
        }
        sort_entries(&mut self.0);
        Value::Object(self.0.into_iter().collect::<Map<String, Value>>())
    }
}

/// Deeper objects than this keep their key order (bounded recursion on hostile input).
const MAX_SORT_DEPTH: usize = 64;

/// `name` first, then ordinal (UTF-16 code unit) order. Keys are compared as UTF-8 bytes,
/// which only differs from UTF-16 between supplementary characters and U+E000..U+FFFF.
fn sort_entries(entries: &mut [(String, Value)]) {
    entries.sort_by(|(a, _), (b, _)| (a != "name").cmp(&(b != "name")).then_with(|| a.cmp(b)));
}

/// Sorts the keys of every object in `v` like [`sort_entries`].
fn sort_keys(v: &mut Value, depth: usize) {
    if depth > MAX_SORT_DEPTH {
        return;
    }
    match v {
        Value::Object(m) => {
            let mut entries: Vec<(String, Value)> = std::mem::take(m).into_iter().collect();
            for (_, child) in &mut entries {
                sort_keys(child, depth.saturating_add(1));
            }
            sort_entries(&mut entries);
            m.extend(entries);
        }
        Value::Array(items) => {
            for item in items {
                sort_keys(item, depth.saturating_add(1));
            }
        }
        _ => {}
    }
}
