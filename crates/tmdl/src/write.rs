//! `SemanticModel` -> TMDL documents, in the layout and style Power BI Desktop writes.
//!
//! Typed objects are first turned into the TMSL-shaped JSON object they stand for (typed fields
//! win over `extra` entries with the same key), then one emitter writes any such object using
//! the schema tables: description as `///` lines, header with the default property after `=`,
//! properties in Desktop's order, then child objects, each after a blank line.

use bicraft_model::{
    Annotation, Column, Measure, NamedExpression, Partition, Relationship, Role, SemanticModel, Table,
};
use serde_json::{Map, Value};

use crate::map::retyped;
use crate::parse::{FENCE, MAX_DEPTH, is_keyword};
use crate::schema::{
    CREDENTIAL_PROPS, DATE_KEYS, DEFAULT_IDENTITY_PROVIDER, DOTTED_KEYS, DefaultMode, EXPRESSION_KEYS,
    JSON_OBJECT_KEYS, Kind, LIST_PROPS, NAME_KEYS, NESTED_PROPS, SOURCE_PROPS, STRING_KEYS, Style, TMSL_ONLY,
    TRANSLATION_PROPS, child_kind, kind_by_keyword, list_item_name, singular,
};

type Obj = Map<String, Value>;

/// Line ending of every written line. Desktop writes CRLF; the writer does so on every platform
/// so output is byte-identical on Windows and Linux.
pub(crate) const NL: &str = "\r\n";

/// Longest file name stem (bytes) before truncation. File systems cap names at 255 bytes, and
/// Desktop (.NET Framework) fails on paths over 260 characters, so stay well below.
const MAX_STEM_BYTES: usize = 120;

/// Model collections written to files of their own rather than into `model.tmdl`.
const FILE_COLLECTIONS: &[&str] = &[
    "tables",
    "relationships",
    "expressions",
    "roles",
    "perspectives",
    "cultures",
    "dataSources",
    "functions",
];

pub(crate) fn documents(model: &SemanticModel) -> Vec<(String, String)> {
    let mut docs = Vec::new();
    let mut names = FileNames::default();

    let mut db = Obj::new();
    if !model.name.is_empty() {
        db.insert("name".into(), Value::String(model.name.clone()));
    }
    if let Some(level) = model.compatibility_level {
        db.insert("compatibilityLevel".into(), Value::from(level));
    }
    let db = with_extra(db, &model.database_extra);
    let mut e = Emitter::default();
    e.top("database", &db);
    docs.push(("database.tmdl".to_string(), e.out));

    docs.push(("model.tmdl".to_string(), model_document(model)));

    if !model.relationships.is_empty() {
        let mut e = Emitter::default();
        for r in &model.relationships {
            e.top("relationship", &relationship(r));
        }
        docs.push(("relationships.tmdl".to_string(), e.out));
    }
    if !model.expressions.is_empty() {
        let mut e = Emitter::default();
        for x in &model.expressions {
            e.top("expression", &expression(x));
        }
        docs.push(("expressions.tmdl".to_string(), e.out));
    }
    for (key, keyword) in [("dataSources", "dataSource"), ("functions", "function")] {
        let items = objects(model.extra.get(key));
        if !items.is_empty() {
            let mut e = Emitter::default();
            for item in items {
                e.top(keyword, item);
            }
            docs.push((format!("{key}.tmdl"), e.out));
        }
    }
    for t in &model.tables {
        let mut e = Emitter::default();
        e.top("table", &table(t));
        docs.push((names.path("tables", &t.name), e.out));
    }
    for r in &model.roles {
        let mut e = Emitter::default();
        e.top("role", &role(r));
        docs.push((names.path("roles", &r.name), e.out));
    }
    for (key, folder, keyword) in [
        ("perspectives", "perspectives", "perspective"),
        ("cultures", "cultures", "cultureInfo"),
    ] {
        for item in objects(model.extra.get(key)) {
            let mut e = Emitter::default();
            e.top(keyword, item);
            docs.push((names.path(folder, &text_of(item.get("name"))), e.out));
        }
    }
    docs.sort_by(|a, b| a.0.cmp(&b.0));
    docs
}

/// One object of a TMDL script, written as a top-level declaration (what [`crate::script`]
/// indents under its command): `keyword` is the TMDL object type.
pub(crate) enum Entry<'a> {
    Table(&'a Table),
    /// `ref table T` with some of its members.
    TableMembers(&'a Table),
    Relationship(&'a Relationship),
    Expression(&'a NamedExpression),
    Role(&'a Role),
    /// A model-level collection item: `function`, `perspective`, `cultureInfo`, `dataSource`.
    Item(&'a str, &'a Obj),
}

pub(crate) fn entry_text(entry: &Entry<'_>) -> String {
    let mut e = Emitter::default();
    match entry {
        Entry::Table(t) => e.top("table", &table(t)),
        Entry::TableMembers(t) => {
            e.top("table", &table(t));
            if let Some(rest) = e.out.strip_prefix("table ") {
                e.out = format!("ref table {rest}");
            }
        }
        Entry::Relationship(r) => e.top("relationship", &relationship(r)),
        Entry::Expression(x) => e.top("expression", &expression(x)),
        Entry::Role(r) => e.top("role", &role(r)),
        Entry::Item(keyword, o) => e.top(keyword, o),
    }
    e.out
}

fn model_document(model: &SemanticModel) -> String {
    let mut o = Obj::new();
    if let Some(d) = &model.description {
        o.insert("description".into(), Value::String(d.clone()));
    }
    if let Some(c) = &model.culture {
        o.insert("culture".into(), Value::String(c.clone()));
    }
    if !model.annotations.is_empty() {
        o.insert("annotations".into(), annotations(&model.annotations));
    }
    for (k, v) in &model.extra {
        if !o.contains_key(k) && !FILE_COLLECTIONS.contains(&k.as_str()) {
            o.insert(k.clone(), v.clone());
        }
    }
    let name = match o.shift_remove("name") {
        Some(Value::String(n)) => n,
        _ => "Model".to_string(),
    };
    // The model's own child objects (query groups, annotations, ...) are written at the top
    // level of model.tmdl, after the model block.
    let kind = kind_by_keyword("model");
    let (props, children) = classify(kind, "model", &o, None);
    let mut e = Emitter::default();
    e.description(0, &o);
    e.line(0, &format!("model {}", quote_name(&name)));
    for (k, v) in props {
        e.prop(1, k, v, 0);
    }
    e.blank();
    for child in children {
        for (i, item) in child.items.iter().enumerate() {
            e.object(0, child.kind, &child.keyword, item, Some(i), 0);
            e.blank();
        }
    }
    let refs: [(&str, Vec<String>); 4] = [
        ("table", model.tables.iter().map(|t| t.name.clone()).collect()),
        ("role", model.roles.iter().map(|r| r.name.clone()).collect()),
        (
            "perspective",
            objects(model.extra.get("perspectives"))
                .iter()
                .map(|p| text_of(p.get("name")))
                .collect(),
        ),
        (
            "cultureInfo",
            objects(model.extra.get("cultures"))
                .iter()
                .map(|c| text_of(c.get("name")))
                .collect(),
        ),
    ];
    for (keyword, names) in refs {
        if names.is_empty() {
            continue;
        }
        for name in names {
            e.line(0, &format!("ref {keyword} {}", quote_name(&name)));
        }
        e.blank();
    }
    e.out
}

/// Array items that are objects.
fn objects(v: Option<&Value>) -> Vec<&Obj> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_object).collect())
        .unwrap_or_default()
}

fn text_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(other) => compact(other),
        None => String::new(),
    }
}

/// Adds `extra` entries whose key the typed fields did not set.
fn with_extra(mut o: Obj, extra: &Obj) -> Obj {
    for (k, v) in extra {
        if !o.contains_key(k) {
            o.insert(k.clone(), v.clone());
        }
    }
    o
}

fn put(o: &mut Obj, key: &str, v: Option<&String>) {
    if let Some(v) = v {
        o.insert(key.into(), Value::String(v.clone()));
    }
}

fn annotations(list: &[Annotation]) -> Value {
    Value::Array(
        list.iter()
            .map(|a| {
                let mut o = Obj::new();
                o.insert("name".into(), Value::String(a.name.clone()));
                o.insert("value".into(), Value::String(a.value.clone()));
                Value::Object(o)
            })
            .collect(),
    )
}

fn table(t: &Table) -> Obj {
    let mut o = Obj::new();
    o.insert("name".into(), Value::String(t.name.clone()));
    put(&mut o, "description", t.description.as_ref());
    if t.is_hidden {
        o.insert("isHidden".into(), Value::Bool(true));
    }
    put(&mut o, "lineageTag", t.lineage_tag.as_ref());
    // `rowNumber` columns are engine-internal; TMDL never declares them. The columns of a
    // calculated table are calculated table columns; Desktop leaves that type implied.
    let calculated = is_calculated_table(t);
    let columns: Vec<Value> = t
        .columns
        .iter()
        .filter(|c| c.extra.get("type").and_then(Value::as_str) != Some("rowNumber"))
        .map(|c| {
            let mut o = column(c);
            if calculated && o.get("type").and_then(Value::as_str) == Some(CALCULATED_TABLE_COLUMN) {
                o.shift_remove("type");
            }
            Value::Object(o)
        })
        .collect();
    if !columns.is_empty() {
        o.insert("columns".into(), Value::Array(columns));
    }
    if !t.measures.is_empty() {
        let m = t.measures.iter().map(|m| Value::Object(measure(m)));
        o.insert("measures".into(), Value::Array(m.collect()));
    }
    // A calculation group's own partition is implied (Desktop writes none and recreates it).
    let partitions: Vec<Value> = t
        .partitions
        .iter()
        .filter(|p| !(t.extra.contains_key("calculationGroup") && is_calculation_group_partition(p)))
        .map(|p| Value::Object(partition(p)))
        .collect();
    if !partitions.is_empty() {
        o.insert("partitions".into(), Value::Array(partitions));
    }
    if !t.annotations.is_empty() {
        o.insert("annotations".into(), annotations(&t.annotations));
    }
    with_extra(o, &t.extra)
}

/// The partition TMDL leaves implied on a calculation group table: `Partition`, import mode,
/// nothing else (TMSL: `{"name": "Partition", "mode": "import", "source": {"type":
/// "calculationGroup"}}`).
pub(crate) const CALCULATION_GROUP_PARTITION: &str = "Partition";

fn is_calculation_group_partition(p: &Partition) -> bool {
    p.name == CALCULATION_GROUP_PARTITION
        && p.mode.as_deref() == Some("import")
        && p.source_type == "calculationGroup"
        && p.expression.is_none()
        && p.extra.is_empty()
        && p.source_extra.is_empty()
}

/// TMSL column type of the columns of a calculated table, implied in TMDL.
pub(crate) const CALCULATED_TABLE_COLUMN: &str = "calculatedTableColumn";

fn is_calculated_table(t: &Table) -> bool {
    t.partitions.iter().any(|p| p.source_type == "calculated")
}

fn column(c: &Column) -> Obj {
    let mut o = Obj::new();
    o.insert("name".into(), Value::String(c.name.clone()));
    put(&mut o, "description", c.description.as_ref());
    put(&mut o, "expression", c.expression.as_ref());
    if let Some(t) = c.data_type.as_tmsl() {
        o.insert("dataType".into(), Value::String(t.into()));
    }
    if c.is_hidden {
        o.insert("isHidden".into(), Value::Bool(true));
    }
    put(&mut o, "formatString", c.format_string.as_ref());
    put(&mut o, "displayFolder", c.display_folder.as_ref());
    put(&mut o, "lineageTag", c.lineage_tag.as_ref());
    put(&mut o, "dataCategory", c.data_category.as_ref());
    put(&mut o, "summarizeBy", c.summarize_by.as_ref());
    put(&mut o, "sourceColumn", c.source_column.as_ref());
    put(&mut o, "sortByColumn", c.sort_by_column.as_ref());
    if !c.annotations.is_empty() {
        o.insert("annotations".into(), annotations(&c.annotations));
    }
    with_extra(o, &c.extra)
}

fn measure(m: &Measure) -> Obj {
    let mut o = Obj::new();
    o.insert("name".into(), Value::String(m.name.clone()));
    put(&mut o, "description", m.description.as_ref());
    o.insert("expression".into(), Value::String(m.expression.clone()));
    put(&mut o, "formatString", m.format_string.as_ref());
    if m.is_hidden {
        o.insert("isHidden".into(), Value::Bool(true));
    }
    put(&mut o, "displayFolder", m.display_folder.as_ref());
    put(&mut o, "lineageTag", m.lineage_tag.as_ref());
    if !m.annotations.is_empty() {
        o.insert("annotations".into(), annotations(&m.annotations));
    }
    with_extra(o, &m.extra)
}

fn partition(p: &Partition) -> Obj {
    let mut o = Obj::new();
    o.insert("name".into(), Value::String(p.name.clone()));
    put(&mut o, "mode", p.mode.as_ref());
    let mut source = Obj::new();
    if !p.source_type.is_empty() {
        source.insert("type".into(), Value::String(p.source_type.clone()));
    }
    put(&mut source, "expression", p.expression.as_ref());
    let source = with_extra(source, &p.source_extra);
    let mut o = with_extra(o, &p.extra);
    o.insert("source".into(), Value::Object(source));
    o
}

fn relationship(r: &Relationship) -> Obj {
    let mut o = Obj::new();
    o.insert("name".into(), Value::String(r.name.clone()));
    if !r.is_active {
        o.insert("isActive".into(), Value::Bool(false));
    }
    put(
        &mut o,
        "crossFilteringBehavior",
        r.cross_filtering_behavior.as_ref(),
    );
    put(&mut o, "fromCardinality", r.from_cardinality.as_ref());
    put(&mut o, "toCardinality", r.to_cardinality.as_ref());
    // Endpoints are `Table.Column` references; incomplete ones keep their `extra` text.
    for (key, table, column) in [
        ("fromColumn", &r.from_table, &r.from_column),
        ("toColumn", &r.to_table, &r.to_column),
    ] {
        if !table.is_empty() || !column.is_empty() {
            let text = format!("{}.{}", quote_name(table), quote_name(column));
            o.insert(key.into(), Value::String(text));
        }
    }
    if !r.annotations.is_empty() {
        o.insert("annotations".into(), annotations(&r.annotations));
    }
    with_extra(o, &r.extra)
}

fn expression(x: &NamedExpression) -> Obj {
    let mut o = Obj::new();
    o.insert("name".into(), Value::String(x.name.clone()));
    put(&mut o, "description", x.description.as_ref());
    if !x.kind.is_empty() && x.kind != "m" {
        o.insert("kind".into(), Value::String(x.kind.clone()));
    }
    o.insert("expression".into(), Value::String(x.expression.clone()));
    put(&mut o, "lineageTag", x.lineage_tag.as_ref());
    if !x.annotations.is_empty() {
        o.insert("annotations".into(), annotations(&x.annotations));
    }
    with_extra(o, &x.extra)
}

fn role(r: &Role) -> Obj {
    let mut o = Obj::new();
    o.insert("name".into(), Value::String(r.name.clone()));
    put(&mut o, "modelPermission", r.model_permission.as_ref());
    if !r.table_permissions.is_empty() {
        let items = r.table_permissions.iter().map(|t| {
            let mut p = Obj::new();
            p.insert("name".into(), Value::String(t.table.clone()));
            put(&mut p, "filterExpression", t.filter_expression.as_ref());
            Value::Object(with_extra(p, &t.extra))
        });
        o.insert("tablePermissions".into(), Value::Array(items.collect()));
    }
    if !r.annotations.is_empty() {
        o.insert("annotations".into(), annotations(&r.annotations));
    }
    with_extra(o, &r.extra)
}

/// A collection of child objects to write.
struct Children<'a> {
    keyword: String,
    kind: Option<&'static Kind>,
    items: Vec<&'a Obj>,
}

/// Splits an object's entries into properties and child collections, both in Desktop's order.
fn classify<'a>(
    kind: Option<&'static Kind>,
    keyword: &str,
    o: &'a Obj,
    index: Option<usize>,
) -> (Vec<(&'a str, &'a Value)>, Vec<Children<'a>>) {
    let name_key = kind.map_or(Some("name"), |k| k.name_key);
    let default_key = if keyword == "partition" {
        None
    } else {
        kind.map_or(Some("expression"), |k| k.default_key)
    };
    let mut props = Vec::new();
    let mut children = Vec::new();
    for (key, value) in o {
        let k = key.as_str();
        if Some(k) == name_key || Some(k) == default_key {
            continue;
        }
        if k == "description" && value.is_string() {
            continue;
        }
        if TMSL_ONLY.contains(&(keyword, k)) {
            continue;
        }
        // Levels and calculation items are written in ordinal order and their ordinal is their
        // position: TMDL has no ordinal (Desktop's reader numbers them by position).
        if k == "ordinal" && matches!(keyword, "level" | "calculationItem") && index.is_some() {
            continue;
        }
        // Implied by the value of an extended property.
        if k == "type" && keyword == "extendedProperty" {
            let implied = match o.get("value") {
                Some(Value::String(_)) => value == "string",
                Some(_) => value == "json",
                None => false,
            };
            if implied {
                continue;
            }
        }
        let listed = LIST_PROPS
            .iter()
            .any(|(kw, _, key, _)| *kw == keyword && *key == k);
        let ck = kind.and_then(|p| child_kind(p, k));
        let keyword = ck.map_or_else(|| singular(k), |c| c.keyword.to_string());
        // An unknown collection is written as objects only when its keyword and its items' keys
        // can be written as TMDL lines; otherwise it stays a property (written as JSON text).
        let writable = |items: &Vec<&Obj>| {
            ck.is_some()
                || (is_keyword(&keyword) && items.iter().all(|item| property_keys(item, Some("name"))))
        };
        if let Some(items) = child_objects(value).filter(|items| !listed && writable(items)) {
            children.push(Children {
                keyword,
                kind: ck,
                items,
            });
            continue;
        }
        if let Value::Object(single) = value
            && let Some(ck) = kind
                .and_then(|p| child_kind(p, k))
                .or_else(|| kind_by_keyword(k).filter(|c| c.key == k))
                .filter(|c| c.single && c.name_key.is_none())
        {
            children.push(Children {
                keyword: ck.keyword.to_string(),
                kind: Some(ck),
                items: vec![single],
            });
            continue;
        }
        props.push((k, value));
    }
    if let Some(kind) = kind {
        props.sort_by_key(|(k, _)| rank(kind.props, k));
        children.sort_by_key(|c| rank(kind.children, &c.keyword));
    }
    (props, children)
}

/// Non-empty arrays of objects are child collections.
fn child_objects(v: &Value) -> Option<Vec<&Obj>> {
    let items = v.as_array()?;
    if items.is_empty() {
        return None;
    }
    items.iter().map(Value::as_object).collect()
}

/// Position of `key` in an order list; unknown keys take the `*` slot (or go last).
fn rank(order: &[&str], key: &str) -> usize {
    order
        .iter()
        .position(|k| k.eq_ignore_ascii_case(key))
        .or_else(|| order.iter().position(|k| *k == "*"))
        .unwrap_or(order.len())
}

#[derive(Default)]
struct Emitter {
    out: String,
}

impl Emitter {
    fn line(&mut self, level: usize, text: &str) {
        for _ in 0..level {
            self.out.push('\t');
        }
        self.out.push_str(text);
        self.out.push_str(NL);
    }

    fn blank(&mut self) {
        self.out.push_str(NL);
    }

    /// A top-level object of a document, followed by a blank line.
    fn top(&mut self, keyword: &str, o: &Obj) {
        let _ = self.object(0, kind_by_keyword(keyword), keyword, o, None, 0);
        self.blank();
    }

    fn description(&mut self, level: usize, o: &Obj) {
        if let Some(Value::String(d)) = o.get("description") {
            for line in d.split('\n') {
                self.line(level, &format!("/// {}", line.trim_end_matches('\r')));
            }
        }
    }

    fn object(
        &mut self,
        level: usize,
        kind: Option<&'static Kind>,
        keyword: &str,
        o: &Obj,
        index: Option<usize>,
        depth: usize,
    ) -> bool {
        if depth > MAX_DEPTH {
            return false;
        }
        let view;
        let o = if keyword == "member" {
            view = member_view(o);
            &view
        } else {
            o
        };
        self.description(level, o);
        let mut head = keyword.to_string();
        if let Some(name_key) = kind.map_or(Some("name"), |k| k.name_key)
            && let Some(name) = o.get(name_key)
        {
            head.push(' ');
            head.push_str(&quote_name(&text_of(Some(name))));
        }
        // TMSL leaves a provider data source's type implied, TMDL a structured one's.
        let provider = Value::String("provider".into());
        let default = if keyword == "partition" {
            o.get("source").and_then(|s| s.get("type"))
        } else if keyword == "dataSource" {
            match o.get("type") {
                None => Some(&provider),
                Some(v) if v == "structured" => None,
                v => v,
            }
        } else {
            kind.map_or(Some("expression"), |k| k.default_key)
                .and_then(|k| o.get(k))
        };
        match default {
            // Desktop omits empty measure expressions and annotation values; elsewhere an empty
            // default property is kept as an empty fenced block so it is not lost.
            Some(Value::String(s)) if s.is_empty() && !matches!(keyword, "measure" | "annotation") => {
                self.line(level, &format!("{head} = {FENCE}"));
                self.line(level.saturating_add(2), FENCE);
            }
            Some(v) => {
                let mode = kind.map_or(DefaultMode::Text, |k| k.default_mode);
                let text = default_text(v, mode);
                self.expression(level, &head, &text, level.saturating_add(2));
            }
            None => self.line(level, &head),
        }
        let (props, children) = classify(kind, keyword, o, index);
        let inner = level.saturating_add(1);
        for (k, v) in props {
            if keyword == "partition" && k == "source" {
                self.source(inner, v, depth);
            } else if let (true, Value::Object(m)) =
                (keyword == "dataSource" && JSON_OBJECT_KEYS.contains(&k), v)
            {
                self.json_object(inner, k, m, depth);
            } else if let Some((line_key, names)) = LIST_PROPS
                .iter()
                .find(|(kw, _, key, _)| *kw == keyword && *key == k)
                .and_then(|(_, line_key, _, field)| {
                    let names: Option<Vec<String>> =
                        v.as_array()?.iter().map(|i| list_item_name(i, *field)).collect();
                    Some((line_key, names?))
                })
            {
                for name in names {
                    self.line(inner, &format!("{line_key}: {}", quote_name(&name)));
                }
            } else {
                self.prop(inner, k, v, depth);
            }
        }
        let mut wrote_children = false;
        let mut wrote_any = false;
        // Desktop writes a grouped object's (`member`) first child object right below it.
        let tight = kind.is_some_and(|k| k.style == Style::Grouped);
        for mut child in children {
            wrote_children = true;
            let grouped = child.kind.is_some_and(|k| k.style == Style::Grouped);
            if matches!(child.keyword.as_str(), "level" | "calculationItem") {
                // Stable: items without an ordinal keep their order, after the others.
                child
                    .items
                    .sort_by_key(|item| item.get("ordinal").and_then(Value::as_u64).unwrap_or(u64::MAX));
            }
            // Grouped siblings (`member`) run without blank lines, except after one that has
            // child objects of its own.
            let mut after_children = false;
            for (i, item) in child.items.iter().enumerate() {
                if (i == 0 || !grouped || after_children) && !(tight && !wrote_any) {
                    self.blank();
                }
                wrote_any = true;
                after_children = self.object(
                    inner,
                    child.kind,
                    &child.keyword,
                    item,
                    Some(i),
                    depth.saturating_add(1),
                );
            }
        }
        wrote_children
    }

    /// A partition's `source`: `source = <expression>` for M and DAX partitions, a nested
    /// block for the others. Its `type` is the partition's default property.
    fn source(&mut self, level: usize, v: &Value, depth: usize) {
        let Value::Object(s) = v else {
            self.prop(level, "source", v, depth);
            return;
        };
        let mut rest: Vec<(&String, &Value)> = s.iter().filter(|(k, _)| *k != "type").collect();
        rest.sort_by_key(|(k, _)| rank(SOURCE_PROPS, k));
        let dates: Vec<(&String, Value)> = rest
            .iter()
            .filter(|(k, _)| DATE_KEYS.contains(&k.as_str()))
            .filter_map(|(k, v)| Some((*k, Value::String(tmdl_date(v.as_str()?)?.to_string()))))
            .collect();
        let rest: Vec<(&String, &Value)> = rest
            .into_iter()
            .map(|(k, v)| (k, dates.iter().find(|(d, _)| *d == k).map_or(v, |(_, d)| d)))
            .collect();
        match rest.as_slice() {
            [] => {}
            [(k, Value::String(e))] if k.as_str() == "expression" => {
                self.expression(level, "source", e, level.saturating_add(2));
            }
            _ => {
                self.line(level, "source");
                for (k, v) in rest {
                    self.prop(level.saturating_add(1), k, v, depth.saturating_add(1));
                }
            }
        }
    }

    /// A data source's `options` or `credential`: `key = <JSON>`, two levels deeper like an
    /// object's default property; a credential's well-known fields follow as properties.
    fn json_object(&mut self, level: usize, key: &str, m: &Obj, depth: usize) {
        let credential = key == "credential";
        let known = |k: &str| credential && CREDENTIAL_PROPS.iter().any(|(_, pascal)| *pascal == k);
        let rest: Obj = m
            .iter()
            .filter(|(k, _)| !known(k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let has_known = m.keys().any(|k| known(k));
        if rest.is_empty() && has_known {
            self.line(level, key);
        } else {
            let text = default_text(&Value::Object(rest), DefaultMode::Json);
            self.expression(level, key, &text, level.saturating_add(2));
        }
        for (camel, pascal) in CREDENTIAL_PROPS {
            if let Some(v) = m.get(*pascal).filter(|_| credential) {
                self.prop(level.saturating_add(1), camel, v, depth.saturating_add(1));
            }
        }
    }

    fn prop(&mut self, level: usize, key: &str, value: &Value, depth: usize) {
        // A key that is not a TMDL keyword has no line form. Nested objects with such keys are
        // written as JSON text below (see `property_keys`); only an object's own unknown key
        // (from TMSL) can get here, and it is dropped rather than written unreadable.
        if !is_keyword(key) {
            return;
        }
        let as_json = match value {
            Value::Object(m) if key == "translations" => !translation_keys(m, 0),
            Value::Object(m) => dotted(key, m).is_none() && !property_keys(m, None),
            Value::Array(_) => child_objects(value).is_some_and(|items| {
                !is_keyword(&singular(key)) || items.iter().any(|item| !property_keys(item, Some("name")))
            }),
            _ => false,
        };
        if as_json {
            // Read back as that text, which writes the same line again.
            self.string_prop(level, key, &compact(value));
            return;
        }
        match value {
            Value::Bool(true) => self.line(level, key),
            Value::Bool(false) | Value::Null | Value::Number(_) => {
                self.line(level, &format!("{key}: {value}"));
            }
            Value::String(s) => self.string_prop(level, key, s),
            Value::Array(items) => {
                if items.is_empty() {
                    return;
                }
                let lines: Option<Vec<&str>> = items.iter().map(Value::as_str).collect();
                match lines {
                    // TMSL's array-of-lines form of an expression.
                    Some(lines) => self.string_prop(level, key, &lines.join("\n")),
                    None => {
                        if let Some(nested) = child_objects(value) {
                            if depth > MAX_DEPTH {
                                return;
                            }
                            let keyword = singular(key);
                            for item in nested {
                                self.compact_object(level, &keyword, item, depth);
                            }
                        } else {
                            let text = compact(value);
                            self.expression(level, key, &text, level.saturating_add(1));
                        }
                    }
                }
            }
            Value::Object(m) => {
                if let Some(text) = dotted(key, m) {
                    self.line(level, &format!("{key}: {text}"));
                } else if key == "translations" {
                    self.line(level, key);
                    self.translations(level.saturating_add(1), m, depth.saturating_add(1));
                } else {
                    self.line(level, key);
                    match NESTED_PROPS.iter().find(|(k, _)| *k == key) {
                        Some((_, order)) => {
                            let mut sorted: Vec<(&String, &Value)> = m.iter().collect();
                            sorted.sort_by_key(|(k, _)| rank(order, k));
                            let sorted: Obj =
                                sorted.into_iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                            self.compact_props(level.saturating_add(1), &sorted, depth.saturating_add(1));
                        }
                        None => self.compact_props(level.saturating_add(1), m, depth.saturating_add(1)),
                    }
                }
            }
        }
    }

    /// The entries of a nested property object, without blank lines.
    fn compact_props(&mut self, level: usize, m: &Obj, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for (k, v) in m {
            self.prop(level, k, v, depth);
        }
    }

    /// A named object inside a nested property object.
    fn compact_object(&mut self, level: usize, keyword: &str, o: &Obj, depth: usize) {
        let mut head = keyword.to_string();
        if let Some(name) = o.get("name") {
            head.push(' ');
            head.push_str(&quote_name(&text_of(Some(name))));
        }
        self.line(level, &head);
        let rest: Obj = o
            .iter()
            .filter(|(k, _)| *k != "name")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        self.compact_props(level.saturating_add(1), &rest, depth.saturating_add(1));
    }

    fn string_prop(&mut self, level: usize, key: &str, s: &str) {
        // References are written as read: `Table.Column`, quoted per name.
        if DOTTED_KEYS.iter().any(|(k, _)| *k == key) || matches!(key, "fromColumn" | "toColumn") {
            self.line(level, &format!("{key}: {s}"));
        } else if NAME_KEYS.contains(&key) && !s.contains(['\n', '\r']) {
            self.line(level, &format!("{key}: {}", quote_name(s)));
        } else if !s.is_empty() && (s.contains(['\n', '\r']) || EXPRESSION_KEYS.contains(&key)) {
            self.expression(level, key, s, level.saturating_add(1));
        } else {
            self.line(level, &format!("{key}: {}", quote_text(key, s)));
        }
    }

    /// `translations`: object keywords for collections, short property names, no blank lines.
    fn translations(&mut self, level: usize, m: &Obj, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        // Translated properties first (caption, description, display folder), then the rest.
        let mut entries: Vec<(&String, &Value)> = m.iter().filter(|(k, _)| *k != "name").collect();
        entries.sort_by_key(|(k, v)| {
            TRANSLATION_PROPS
                .iter()
                .position(|(_, long)| long == k)
                .unwrap_or(if v.is_array() || v.is_object() { 4 } else { 3 })
        });
        for (k, v) in entries {
            let short = TRANSLATION_PROPS
                .iter()
                .find(|(_, long)| *long == k)
                .map_or(k.as_str(), |(short, _)| *short);
            match v {
                Value::String(s) => {
                    if s.contains(['\n', '\r']) {
                        self.expression(level, short, s, level.saturating_add(1));
                    } else {
                        self.line(level, &format!("{short}: {}", quote_text(k, s)));
                    }
                }
                Value::Object(item) if k == "model" => {
                    self.translation_item(level, "model", item, depth);
                }
                Value::Array(_) => {
                    for item in child_objects(v).unwrap_or_default() {
                        self.translation_item(level, &singular(k), item, depth);
                    }
                }
                other => self.prop(level, short, other, depth),
            }
        }
    }

    fn translation_item(&mut self, level: usize, keyword: &str, item: &Obj, depth: usize) {
        let name = text_of(item.get("name"));
        self.line(level, &format!("{keyword} {}", quote_name(&name)));
        self.translations(level.saturating_add(1), item, depth.saturating_add(1));
    }

    /// `head = expression`: on one line, indented on the following lines at `base`, or fenced
    /// with ``` when indentation alone would lose whitespace. An empty expression writes `head`.
    ///
    /// TMDL has no escape inside a fence, so a body with a line that is just ``` (it would end
    /// the fence early) is written indented instead. Our reader keeps leading and inner
    /// whitespace there but drops whitespace and empty lines at the very end: the one loss
    /// accepted so the output always reads back (docs/fuzzing.md, F1).
    fn expression(&mut self, level: usize, head: &str, text: &str, base: usize) {
        if text.is_empty() {
            self.line(level, head);
            return;
        }
        let lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
        let needs = needs_fence(&lines);
        let fence = needs && !closes_fence(&lines);
        match lines.as_slice() {
            [one] if !needs && !one.starts_with(FENCE) => {
                self.line(level, &format!("{head} = {one}"));
            }
            _ if !fence => {
                self.line(level, &format!("{head} ="));
                for l in &lines {
                    self.line(base, l);
                }
            }
            _ => {
                self.line(level, &format!("{head} = {FENCE}"));
                for l in &lines {
                    self.line(base, l);
                }
                self.line(base, FENCE);
            }
        }
    }
}

/// Whether every key of a nested property object (but `skip`, written in the object's header)
/// can be written as a TMDL property line. JSON values (an extended property's value, a
/// `key = {...}` text read as JSON) can carry any key, such as `a b`, `""` or `-x`.
fn property_keys(m: &Obj, skip: Option<&str>) -> bool {
    m.keys().all(|k| Some(k.as_str()) == skip || is_keyword(k))
}

/// [`property_keys`] for a `translations` block, whose collections nest: every key written as a
/// line (short names for translated properties, singular keywords for collections).
fn translation_keys(m: &Obj, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    m.iter().filter(|(k, _)| *k != "name").all(|(k, v)| {
        let short = TRANSLATION_PROPS
            .iter()
            .find(|(_, long)| long == k)
            .map_or(k.as_str(), |(short, _)| *short);
        if !is_keyword(short) {
            return false;
        }
        match v {
            Value::Object(item) if k == "model" => translation_keys(item, depth.saturating_add(1)),
            Value::Array(_) => {
                let items = child_objects(v).unwrap_or_default();
                items.is_empty()
                    || (is_keyword(&singular(k))
                        && items
                            .iter()
                            .all(|item| translation_keys(item, depth.saturating_add(1))))
            }
            _ => true,
        }
    })
}

/// Whether an expression needs a ``` fence to survive the reader's trimming: its first
/// non-empty line starts with whitespace, a line ends with whitespace, or it ends with a line
/// break.
fn needs_fence(lines: &[&str]) -> bool {
    let leading = lines
        .iter()
        .find(|l| !l.is_empty())
        .is_some_and(|l| l.starts_with(char::is_whitespace));
    let trailing = lines.iter().any(|l| l.ends_with(char::is_whitespace));
    let last_empty = lines.len() > 1 && lines.last().is_some_and(|l| l.is_empty());
    leading || trailing || last_empty
}

/// Whether a body line would end a ``` fence: the reader closes on any line that is ``` apart
/// from surrounding whitespace.
fn closes_fence(lines: &[&str]) -> bool {
    lines.iter().any(|l| l.trim() == FENCE)
}

/// A TMSL date-time as TMDL writes it: TMDL dates are `yyyy-MM-dd` only (Desktop's reader refuses
/// a time of day and its writer drops it), so `2024-01-01T00:00:00` becomes `2024-01-01`.
fn tmdl_date(s: &str) -> Option<&str> {
    let date = s.get(..10)?;
    let shape = date.bytes().enumerate().all(|(i, b)| {
        if i == 4 || i == 7 {
            b == b'-'
        } else {
            b.is_ascii_digit()
        }
    });
    (shape && s.get(10..11).is_none_or(|t| t == "T")).then_some(date)
}

/// The text after `=` for a default property value.
fn default_text(v: &Value, mode: DefaultMode) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Object(_) | Value::Array(_) if mode == DefaultMode::Json && json_depth(v) <= 100 => {
            serde_json::to_string_pretty(v).unwrap_or_default()
        }
        other => compact(other),
    }
}

/// Compact JSON text; `null` for values nested too deeply to serialize safely.
fn compact(v: &Value) -> String {
    if json_depth(v) <= 100 {
        v.to_string()
    } else {
        "null".to_string()
    }
}

/// Nesting depth of a JSON value, counted up to a limit (so the walk itself is bounded).
fn json_depth(v: &Value) -> usize {
    fn walk(v: &Value, depth: usize) -> usize {
        if depth > 101 {
            return depth;
        }
        let inner = match v {
            Value::Array(a) => a.iter().map(|x| walk(x, depth + 1)).max(),
            Value::Object(o) => o.values().map(|x| walk(x, depth + 1)).max(),
            _ => None,
        };
        inner.unwrap_or(depth)
    }
    walk(v, 0)
}

/// `{"table": T, "<second>": X}` written as the reference `T.X`.
fn dotted(key: &str, m: &Obj) -> Option<String> {
    let (_, second) = DOTTED_KEYS.iter().find(|(k, _)| *k == key)?;
    if m.len() != 2 {
        return None;
    }
    let table = m.get("table")?.as_str()?;
    let other = m.get(*second)?.as_str()?;
    Some(format!("{}.{}", quote_name(table), quote_name(other)))
}

/// An object name as Desktop writes it: in single quotes (with `''` for `'`) when it is empty,
/// contains whitespace, `.`, `=` or `'`. bicraft also quotes a leading `:`, which Desktop
/// writes bare but then fails to read back.
pub(crate) fn quote_name(name: &str) -> String {
    let needs = name.is_empty()
        || name.starts_with(':')
        || name
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '.' | '=' | '\''));
    if needs {
        format!("'{}'", name.replace('\'', "''"))
    } else {
        name.to_string()
    }
}

/// A `key: value` text as Desktop writes it: double-quoted (with `""` for `"`) when it is
/// empty, starts with `"`, or starts or ends with whitespace; also when the reader would
/// otherwise take it for a boolean, `null` or a number (only for keys not known to be text).
fn quote_text(key: &str, s: &str) -> String {
    let needs = s.is_empty()
        || (s.starts_with('"') && s.ends_with('"'))
        || s.starts_with(char::is_whitespace)
        || s.ends_with(char::is_whitespace)
        || (!STRING_KEYS.contains(&key) && retyped(s).is_some());
    if needs {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// See [`crate::unwritable_name`].
pub(crate) fn unwritable_name(model: &SemanticModel) -> Option<String> {
    fn bad(name: &str) -> bool {
        name.chars().any(char::is_control)
    }
    /// Names in TMSL-shaped JSON: `name`, `memberName` and `folder` keys, at any depth.
    fn walk(v: &Value, depth: usize) -> Option<String> {
        if depth > MAX_DEPTH {
            return None;
        }
        match v {
            Value::Object(m) => m.iter().find_map(|(k, v)| match v {
                Value::String(s) if matches!(k.as_str(), "name" | "memberName" | "folder") && bad(s) => {
                    Some(s.clone())
                }
                _ => walk(v, depth.saturating_add(1)),
            }),
            Value::Array(items) => items.iter().find_map(|x| walk(x, depth.saturating_add(1))),
            _ => None,
        }
    }
    let objects = model
        .tables
        .iter()
        .map(table)
        .chain(model.relationships.iter().map(relationship))
        .chain(model.expressions.iter().map(expression))
        .chain(model.roles.iter().map(role))
        .map(Value::Object);
    std::iter::once(&model.name)
        .chain(model.annotations.iter().map(|a| &a.name))
        .find(|n| bad(n))
        .cloned()
        .or_else(|| objects.into_iter().find_map(|o| walk(&o, 0)))
        .or_else(|| walk(&Value::Object(model.extra.clone()), 0))
        .or_else(|| walk(&Value::Object(model.database_extra.clone()), 0))
}

/// A role member as TMDL holds it: `member X = activeDirectory` for a Windows member (TMSL:
/// neither an identity provider nor a member type), otherwise the member type with `user`
/// implied and `auto` (TMSL: no member type) explicit, and the identity provider only when it is
/// not the default. A member type without an identity provider is an external member in TMSL's
/// schema: it keeps its type and is written without an `identityProvider` line, the form Desktop
/// 2.159 opens and saves back (it refuses `identityProvider: ""`), so it reads back with the
/// default provider. TMDL cannot hold a member's `memberId` (TMSL only).
fn member_view(o: &Obj) -> Obj {
    let external = o.get("identityProvider");
    let member_type = match (external, o.get("memberType")) {
        (None, None) => Some(Value::String("activeDirectory".into())),
        (Some(_), None) => Some(Value::String("auto".into())),
        (_, Some(t)) if t == "user" => None,
        (_, Some(t)) => Some(t.clone()),
    };
    let mut out = Obj::new();
    for (k, v) in o {
        match k.as_str() {
            // TMDL has no `memberId` (Desktop's serializer drops it and refuses to read it).
            "memberType" | "memberId" => {}
            "identityProvider" if v == DEFAULT_IDENTITY_PROVIDER => {}
            _ => {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    if let Some(t) = member_type {
        out.insert("memberType".into(), t);
    }
    out
}

/// File names per folder, made safe and unique.
#[derive(Default)]
struct FileNames {
    used: std::collections::HashSet<String>,
}

impl FileNames {
    /// `folder/<stem>.tmdl` for an object name; see [`file_stem`]. A stem already used in the
    /// folder (compared case-insensitively) gets a `~2`, `~3`, ... suffix.
    fn path(&mut self, folder: &str, name: &str) -> String {
        let stem = file_stem(name);
        let mut candidate = format!("{folder}/{stem}.tmdl");
        let mut n = 2u32;
        while !self.used.insert(candidate.to_lowercase()) {
            candidate = format!("{folder}/{stem}~{n}.tmdl");
            n = n.saturating_add(1);
        }
        candidate
    }
}

/// The file name stem for an object, following Desktop's rule: the characters Windows forbids
/// in file names (`" * / : < > ? \ |`) and a leading space are percent-encoded (`a/b` ->
/// `a%2Fb`). bicraft also encodes control characters and the first character of a Windows
/// device name (`CON` -> `%43ON`), and truncates stems longer than 120 bytes. The result is
/// never `.`/`..` and never contains a path separator.
pub(crate) fn file_stem(name: &str) -> String {
    let mut stem = String::new();
    for (i, c) in name.chars().enumerate() {
        let encode = matches!(c, '"' | '*' | '/' | ':' | '<' | '>' | '?' | '\\' | '|')
            || c.is_control()
            || (i == 0 && c == ' ');
        if encode {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                stem.push_str(&format!("%{b:02X}"));
            }
        } else {
            stem.push(c);
        }
    }
    let device = stem.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let reserved = matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((device.starts_with("COM") || device.starts_with("LPT"))
            && device.len() == 4
            && device.ends_with(|c: char| c.is_ascii_digit()));
    if reserved && let Some(first) = stem.chars().next() {
        stem = format!("%{:02X}{}", u32::from(first), stem.get(1..).unwrap_or_default());
    }
    if stem.len() > MAX_STEM_BYTES {
        let mut cut = MAX_STEM_BYTES;
        while !stem.is_char_boundary(cut) {
            cut = cut.saturating_sub(1);
        }
        stem.truncate(cut);
    }
    stem
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_quoted_like_desktop() {
        for (name, quoted) in [
            ("Sales", "Sales"),
            ("Order Lines", "'Order Lines'"),
            ("Sales.Archive", "'Sales.Archive'"),
            ("Owner's Cut", "'Owner''s Cut'"),
            ("a=b", "'a=b'"),
            ("2024", "2024"),
            ("a-b", "a-b"),
            ("a:b", "a:b"),
            (":a", "':a'"),
            ("a\u{a0}b", "'a\u{a0}b'"),
            ("a\tb", "'a\tb'"),
            ("", "''"),
            ("\"x\"", "\"x\""),
        ] {
            assert_eq!(quote_name(name), quoted, "{name:?}");
        }
    }

    #[test]
    fn text_values_are_quoted_like_desktop() {
        for (value, quoted) in [
            ("#,0.00", "#,0.00"),
            (" lead", "\" lead\""),
            ("trail ", "\"trail \""),
            ("\"q\"", "\"\"\"q\"\"\""),
            ("a\"b", "a\"b"),
            ("", "\"\""),
            ("0", "0"),
        ] {
            assert_eq!(quote_text("formatString", value), quoted, "{value:?}");
        }
        assert_eq!(quote_text("somethingNew", "12"), "\"12\"");
        assert_eq!(quote_text("somethingNew", "true"), "\"true\"");
    }

    #[test]
    fn fences_follow_desktop() {
        let f = |s: &str| needs_fence(&s.split('\n').collect::<Vec<_>>());
        assert!(f(" lead"));
        assert!(f("trail "));
        assert!(f("x\n"));
        assert!(f("\n    x\n    y"));
        assert!(f("a\n   \nb"));
        assert!(!f("\na"));
        assert!(!f("a\n\n    b"));
        assert!(!f("x```y"));

        let c = |s: &str| closes_fence(&s.split('\n').collect::<Vec<_>>());
        assert!(c("a\n```\nb"));
        assert!(c("a\n\t ``` \nb"));
        assert!(!c("a\n```x\nb"));
        assert!(!c("x```y"));
    }

    #[test]
    fn file_stems_cannot_escape() {
        for (name, stem) in [
            ("Sales", "Sales"),
            ("a/b", "a%2Fb"),
            ("../x", "..%2Fx"),
            ("..", ".."),
            ("a\\b:c*d?e\"f<g>h|", "a%5Cb%3Ac%2Ad%3Fe%22f%3Cg%3Eh%7C"),
            (" lead", "%20lead"),
            ("trail ", "trail "),
            ("tab\tname", "tab%09name"),
            ("CON", "%43ON"),
            ("nul.txt", "%6Eul.txt"),
            ("COM1", "%43OM1"),
            ("Console", "Console"),
            ("%41", "%41"),
        ] {
            assert_eq!(file_stem(name), stem, "{name:?}");
        }
        let long = "é".repeat(300);
        let stem = file_stem(&long);
        assert!(stem.len() <= MAX_STEM_BYTES && stem.chars().all(|c| c == 'é'));
        let mut names = FileNames::default();
        assert_eq!(names.path("tables", "a/b"), "tables/a%2Fb.tmdl");
        assert_eq!(names.path("tables", "a%2Fb"), "tables/a%2Fb~2.tmdl");
        assert_eq!(names.path("tables", "A%2FB"), "tables/A%2FB~3.tmdl");
        assert_eq!(names.path("roles", "a/b"), "roles/a%2Fb.tmdl");
    }
}
