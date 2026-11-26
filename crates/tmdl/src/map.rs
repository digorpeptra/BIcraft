//! Generic TMDL tree -> TMSL-shaped JSON -> `bicraft_model::SemanticModel`.
//!
//! Every object is first converted to the JSON object TMSL would hold for it (object keyword ->
//! TMSL collection, `key: value` -> typed JSON property, `= value` -> the default property such as
//! `expression`), then the properties the model types are taken out of that object and the rest
//! is kept, in order, in the matching `extra` map. The rules mirror the TMSL reader's: a property
//! moves into a typed field only when the writer emits it again the same way, so explicit
//! defaults (`isHidden: false`, `isActive`) stay in `extra`.

use std::collections::{HashMap, HashSet};

use bicraft_model::{
    Annotation, Column, DataType, Measure, NamedExpression, Partition, Properties, Relationship, Role,
    SemanticModel, Table, TablePermission,
};
use serde_json::{Map, Value};

use crate::Error;
use crate::parse::{Document, MAX_DEPTH, Node, ValueKind, object_name, reference};
use crate::schema::{
    CREDENTIAL_PROPS, DATE_KEYS, DEFAULT_IDENTITY_PROVIDER, DOTTED_KEYS, DefaultMode, JSON_OBJECT_KEYS, Kind,
    LIST_PROPS, NAME_KEYS, STRING_KEYS, TRANSLATION_PROPS, canonical_key, kind_by_keyword, plural,
};
use crate::write::{CALCULATED_TABLE_COLUMN, CALCULATION_GROUP_PARTITION};

type Obj = Map<String, Value>;

#[derive(Default)]
struct Builder {
    /// Whether a `database` object was read; Desktop writes it without a name when the database
    /// has none.
    saw_database: bool,
    database_name: Option<String>,
    model_name: Option<String>,
    model_description: Option<String>,
    /// Database properties (TMSL root, minus `name` and `model`).
    database: Obj,
    /// Model properties and model-level collections other than the typed ones.
    model: Obj,
    tables: Vec<Obj>,
    /// Lowercased table name -> index in `tables` (names compare case-insensitively).
    table_index: HashMap<String, usize>,
    /// (table index, "column" | "measure", lowercased name) already declared.
    members: HashSet<(usize, &'static str, String)>,
    relationships: Vec<Obj>,
    expressions: Vec<Obj>,
    roles: Vec<Obj>,
    /// `ref <kind> <name>` entries, by lowercased kind, in reading order.
    refs: HashMap<String, Vec<String>>,
}

pub(crate) fn build(docs: &[(String, Document)]) -> Result<SemanticModel, Error> {
    let mut b = Builder::default();
    for (file, doc) in docs {
        b.visit(file, doc, &doc.roots, 0)?;
    }
    Ok(b.finish())
}

impl Builder {
    fn visit(&mut self, file: &str, doc: &Document, ids: &[usize], depth: usize) -> Result<(), Error> {
        if depth > MAX_DEPTH {
            return Ok(());
        }
        let conv = Conv { doc };
        for &id in ids {
            let Some(node) = doc.node(id) else { continue };
            if node.is_ref {
                if let Some(name) = &node.name {
                    self.refs.entry(node.kind.clone()).or_default().push(name.clone());
                }
                continue;
            }
            match node.kind.as_str() {
                "database" => {
                    self.saw_database = true;
                    if node.name.is_some() {
                        self.database_name.clone_from(&node.name);
                    }
                    if let Some(d) = &node.description {
                        self.database
                            .insert("description".into(), Value::String(d.clone()));
                    }
                    let (objects, props) = split_children(doc, node);
                    conv.fill(&mut self.database, &props, None, depth);
                    self.visit(file, doc, &objects, depth.saturating_add(1))?;
                }
                "model" => {
                    if node.name.is_some() {
                        self.model_name.clone_from(&node.name);
                    }
                    if node.description.is_some() {
                        self.model_description.clone_from(&node.description);
                    }
                    let (objects, props) = split_children(doc, node);
                    conv.fill(&mut self.model, &props, kind_by_keyword("model"), depth);
                    self.visit(file, doc, &objects, depth.saturating_add(1))?;
                }
                // TMDL scripts wrap definitions in `createOrReplace`.
                "createorreplace" => {
                    self.visit(file, doc, &node.children, depth.saturating_add(1))?;
                }
                "table" => self.table(file, doc, node, depth)?,
                "relationship" if node.name.is_some() => {
                    self.relationships
                        .push(conv.object(node, kind_by_keyword("relationship"), depth));
                }
                "expression" if node.name.is_some() => {
                    self.expressions
                        .push(conv.object(node, kind_by_keyword("expression"), depth));
                }
                "role" if node.name.is_some() => {
                    self.roles.push(conv.object(node, kind_by_keyword("role"), depth));
                }
                // Everything else at the top level belongs to the model: query groups,
                // annotations, perspectives, cultures, data sources, functions, properties.
                _ => conv.fill(
                    &mut self.model,
                    std::slice::from_ref(&id),
                    kind_by_keyword("model"),
                    depth,
                ),
            }
        }
        Ok(())
    }

    fn table(&mut self, file: &str, doc: &Document, node: &Node, depth: usize) -> Result<(), Error> {
        let name = required_name(file, node)?;
        // Tables may be declared partially across several documents.
        let next = self.tables.len();
        let index = *self.table_index.entry(name.to_ascii_lowercase()).or_insert(next);
        for child in node.children.iter().filter_map(|&id| doc.node(id)) {
            let what = match child.kind.as_str() {
                "column" if !child.is_ref => "column",
                "measure" if !child.is_ref => "measure",
                _ => continue,
            };
            let member = required_name(file, child)?;
            if !self.members.insert((index, what, member.to_ascii_lowercase())) {
                return Err(Error::Syntax {
                    file: file.to_string(),
                    line: child.line,
                    message: format!("{what} `{member}` is declared twice"),
                });
            }
        }
        let obj = Conv { doc }.object(node, kind_by_keyword("table"), depth);
        if index == next {
            self.tables.push(obj);
        } else if let Some(existing) = self.tables.get_mut(index) {
            merge(existing, obj);
        }
        Ok(())
    }

    fn finish(mut self) -> SemanticModel {
        let mut tables: Vec<Table> = self.tables.into_iter().map(table).collect();
        reorder(&mut tables, |t: &Table| t.name.as_str(), self.refs.get("table"));
        let mut roles: Vec<Role> = self.roles.into_iter().map(role).collect();
        reorder(&mut roles, |r: &Role| r.name.as_str(), self.refs.get("role"));
        for (key, kind) in [("perspectives", "perspective"), ("cultures", "cultureinfo")] {
            if let Some(Value::Array(items)) = self.model.get_mut(key) {
                reorder(items, value_name, self.refs.get(kind));
            }
        }

        let mut db = Take(self.database);
        let compatibility_level = db.take("compatibilityLevel", |v| {
            v.as_u64().and_then(|n| u32::try_from(n).ok())
        });
        let mut model = Take(self.model);
        let culture = model.string("culture");
        let annotations = model.annotations();
        let mut extra = model.extra();
        if let Some(name) = &self.model_name
            && name != "Model"
        {
            extra.insert("name".into(), Value::String(name.clone()));
        }
        SemanticModel {
            // Without a `database` object, the model's name stands in.
            name: if self.saw_database {
                self.database_name.unwrap_or_default()
            } else {
                self.model_name.unwrap_or_default()
            },
            compatibility_level,
            culture,
            description: self.model_description,
            tables,
            relationships: self.relationships.into_iter().map(relationship).collect(),
            expressions: self.expressions.into_iter().map(expression).collect(),
            roles,
            annotations,
            database_extra: db.extra(),
            extra,
        }
    }
}

/// Splits a `database`/`model` node's children into nested objects (visited like top-level
/// entries) and its own properties.
fn split_children(doc: &Document, node: &Node) -> (Vec<usize>, Vec<usize>) {
    node.children.iter().partition(|&&id| {
        doc.node(id).is_some_and(|c| {
            c.is_ref || c.name.is_some() || matches!(c.kind.as_str(), "model" | "createorreplace")
        })
    })
}

/// Merges a partial declaration into an earlier one: collections append, properties replace.
fn merge(into: &mut Obj, from: Obj) {
    for (key, value) in from {
        match (into.get_mut(&key), value) {
            (Some(Value::Array(a)), Value::Array(b)) => a.extend(b),
            (_, value) => {
                into.insert(key, value);
            }
        }
    }
}

fn value_name(v: &Value) -> &str {
    v.get("name").and_then(Value::as_str).unwrap_or_default()
}

/// Moves the items named in `order` (case-insensitively) to the front, in that order.
fn reorder<T>(items: &mut Vec<T>, name: fn(&T) -> &str, order: Option<&Vec<String>>) {
    let Some(order) = order else { return };
    // First item per lowercased name; linear in items + refs, whatever the input.
    let mut first: HashMap<String, usize> = HashMap::new();
    for (i, t) in items.iter().enumerate() {
        first.entry(name(t).to_ascii_lowercase()).or_insert(i);
    }
    let mut slots: Vec<Option<T>> = std::mem::take(items).into_iter().map(Some).collect();
    for wanted in order {
        if let Some(i) = first.remove(&wanted.to_ascii_lowercase())
            && let Some(t) = slots.get_mut(i).and_then(Option::take)
        {
            items.push(t);
        }
    }
    items.extend(slots.into_iter().flatten());
}

/// Converts generic document nodes into TMSL-shaped JSON.
struct Conv<'a> {
    doc: &'a Document,
}

impl Conv<'_> {
    /// One object: name, description, default property, then its children.
    fn object(&self, node: &Node, kind: Option<&Kind>, depth: usize) -> Obj {
        let mut o = Obj::new();
        let keyword = kind.map_or(node.kind.as_str(), |k| k.keyword);
        if let Some(name) = &node.name {
            let key = kind.map_or(Some("name"), |k| k.name_key).unwrap_or("name");
            o.insert(key.into(), Value::String(name.clone()));
        }
        if let Some(d) = &node.description {
            o.insert("description".into(), Value::String(d.clone()));
        }
        if keyword == "partition" {
            let mut source = Obj::new();
            if let Some(v) = &node.value {
                source.insert("type".into(), Value::String(v.clone()));
            }
            o.insert("source".into(), Value::Object(source));
        } else if let Some(v) = &node.value {
            let key = kind.and_then(|k| k.default_key).unwrap_or("expression");
            let mode = kind.map_or(DefaultMode::Text, |k| k.default_mode);
            match mode {
                // TMSL leaves a provider data source's type implied.
                DefaultMode::Text if keyword == "dataSource" && v == "provider" => {}
                DefaultMode::Text => {
                    o.insert(key.into(), Value::String(v.clone()));
                }
                DefaultMode::Json => {
                    let value = json_text(v);
                    // TOM's TMSL leaves a string extended property's type implied.
                    if keyword == "extendedProperty" && !value.is_string() {
                        o.insert("type".into(), Value::String("json".into()));
                    }
                    o.insert(key.into(), value);
                }
            }
        } else if keyword == "annotation" {
            o.insert("value".into(), Value::String(String::new()));
        } else if keyword == "dataSource" {
            // Desktop leaves the default `structured` type implied.
            o.insert("type".into(), Value::String("structured".into()));
        }
        self.fill(&mut o, &node.children, kind, depth.saturating_add(1));
        if keyword == "member" {
            o = tmsl_member(o);
        }
        o
    }

    /// Adds the entries `ids` (children of an object of `kind`) to `o`.
    fn fill(&self, o: &mut Obj, ids: &[usize], kind: Option<&Kind>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let next = depth.saturating_add(1);
        let in_partition = kind.is_some_and(|k| k.keyword == "partition");
        let in_data_source = kind.is_some_and(|k| k.keyword == "dataSource");
        for child in ids.iter().filter_map(|&id| self.doc.node(id)) {
            if child.is_ref {
                continue;
            }
            let child_kind = kind_by_keyword(&child.kind);
            if child.name.is_some() {
                let key =
                    child_kind.map_or_else(|| plural(&canonical_key(&child.keyword)), |k| k.key.to_string());
                let item = Value::Object(self.object(child, child_kind, next));
                if child_kind.is_some_and(|k| k.single) {
                    o.insert(key, item);
                } else {
                    push_item(o, key, item);
                }
                continue;
            }
            let key = canonical_key(&child.keyword);
            if in_partition && key == "source" {
                let mut source = match o.shift_remove("source") {
                    Some(Value::Object(s)) => s,
                    _ => Obj::new(),
                };
                if let Some(v) = &child.value {
                    source.insert("expression".into(), Value::String(v.clone()));
                }
                self.fill(&mut source, &child.children, None, next);
                // TMDL dates have no time of day; TMSL writes midnight.
                for key in DATE_KEYS {
                    if let Some(Value::String(d)) = source.get_mut(*key)
                        && d.len() == 10
                        && d.bytes().filter(|b| *b == b'-').count() == 2
                    {
                        d.push_str("T00:00:00");
                    }
                }
                o.insert("source".into(), Value::Object(source));
                continue;
            }
            if let Some((_, _, list, field)) = LIST_PROPS.iter().find(|(kw, line_key, _, _)| {
                kind.is_some_and(|k| k.keyword == *kw) && line_key.eq_ignore_ascii_case(&key)
            }) && child.children.is_empty()
                && child.value_kind != ValueKind::Expression
            {
                let name = Value::String(name_value(child.raw.as_deref().unwrap_or_default()));
                let item = match field {
                    Some(f) => {
                        let mut m = Obj::new();
                        m.insert((*f).to_string(), name);
                        Value::Object(m)
                    }
                    None => name,
                };
                push_item(o, (*list).to_string(), item);
                continue;
            }
            if in_data_source && JSON_OBJECT_KEYS.contains(&key.as_str()) {
                o.insert(key.clone(), self.json_object(child, key == "credential", next));
                continue;
            }
            if key == "translations" && !child.children.is_empty() {
                o.insert(key, Value::Object(self.translations(child, next)));
                continue;
            }
            if let Some(ck) = child_kind.filter(|k| k.name_key.is_none())
                && (child.value_kind == ValueKind::Expression || !child.children.is_empty())
            {
                let item = Value::Object(self.object(child, Some(ck), next));
                if ck.single {
                    o.insert(ck.key.into(), item);
                } else {
                    push_item(o, ck.key.into(), item);
                }
                continue;
            }
            let value = match (&child.value, child.value_kind) {
                (Some(v), ValueKind::Expression) => Value::String(v.clone()),
                // Names and references are read from the text as written.
                (Some(_), _) if is_reference_key(&key) => {
                    scalar(&key, child.raw.as_deref().unwrap_or_default())
                }
                (Some(v), ValueKind::QuotedText) => Value::String(v.clone()),
                (Some(v), _) => scalar(&key, v),
                (None, _) if child.children.is_empty() => Value::Bool(true),
                (None, _) => {
                    let mut nested = Obj::new();
                    self.fill(&mut nested, &child.children, child_kind, next);
                    Value::Object(nested)
                }
            };
            o.insert(key, value);
        }
        // Level and calculation item order is their ordinal; TMDL leaves it implied.
        for key in ["levels", "calculationItems"] {
            if let Some(Value::Array(items)) = o.get_mut(key) {
                for (i, item) in items.iter_mut().enumerate() {
                    if let Value::Object(m) = item
                        && !m.contains_key("ordinal")
                    {
                        let mut with = Obj::new();
                        if let Some(name) = m.shift_remove("name") {
                            with.insert("name".into(), name);
                        }
                        with.insert("ordinal".into(), Value::from(i));
                        with.extend(std::mem::take(m));
                        *m = with;
                    }
                }
            }
        }
    }

    /// A data source's `options = <JSON>` or `credential = <JSON>` (with a credential's
    /// well-known fields as properties below it). Text that is not a JSON object stays text.
    fn json_object(&self, node: &Node, credential: bool, depth: usize) -> Value {
        let mut o = match node.value.as_deref().map(serde_json::from_str::<Value>) {
            Some(Ok(Value::Object(m))) => m,
            None => Obj::new(),
            Some(_) => return Value::String(node.value.clone().unwrap_or_default()),
        };
        let mut fields = Obj::new();
        self.fill(&mut fields, &node.children, None, depth);
        for (k, v) in fields {
            let key = CREDENTIAL_PROPS
                .iter()
                .find(|(camel, _)| credential && camel.eq_ignore_ascii_case(&k))
                .map_or(k, |(_, pascal)| (*pascal).to_string());
            o.insert(key, v);
        }
        Value::Object(o)
    }

    /// A `translations` block: object keywords become TMSL collections and `caption`,
    /// `description`, `displayFolder` become `translatedCaption`, ... (all values are text).
    fn translations(&self, node: &Node, depth: usize) -> Obj {
        let mut o = Obj::new();
        if depth > MAX_DEPTH {
            return o;
        }
        for child in node.children.iter().filter_map(|&id| self.doc.node(id)) {
            if let Some(name) = &child.name {
                let mut item = Obj::new();
                item.insert("name".into(), Value::String(name.clone()));
                item.extend(self.translations(child, depth.saturating_add(1)));
                if child.kind == "model" {
                    o.insert("model".into(), Value::Object(item));
                } else {
                    push_item(
                        &mut o,
                        plural(&canonical_key(&child.keyword)),
                        Value::Object(item),
                    );
                }
                continue;
            }
            let key = canonical_key(&child.keyword);
            let key = TRANSLATION_PROPS
                .iter()
                .find(|(short, _)| *short == key)
                .map_or(key.clone(), |(_, long)| (*long).to_string());
            let value = match &child.value {
                Some(v) => Value::String(v.clone()),
                None if child.children.is_empty() => Value::Bool(true),
                None => Value::Object(self.translations(child, depth.saturating_add(1))),
            };
            o.insert(key, value);
        }
        o
    }
}

/// A role member in TMSL's shape: `= activeDirectory` is a Windows member (no identity provider,
/// no member type); otherwise the member type defaults to `user` (`auto` is TMSL's implied
/// one) and the identity provider to the default one.
fn tmsl_member(mut o: Obj) -> Obj {
    let member_type = o.shift_remove("memberType");
    if member_type.as_ref().is_some_and(|t| t == "activeDirectory") {
        return o;
    }
    let provider = o
        .shift_remove("identityProvider")
        .unwrap_or_else(|| Value::String(DEFAULT_IDENTITY_PROVIDER.into()));
    let mut out = Obj::new();
    for key in ["memberName", "memberId"] {
        if let Some(v) = o.shift_remove(key) {
            out.insert(key.into(), v);
        }
    }
    out.insert("identityProvider".into(), provider);
    match member_type {
        Some(t) if t == "auto" => {}
        Some(t) => {
            out.insert("memberType".into(), t);
        }
        None => {
            out.insert("memberType".into(), Value::String("user".into()));
        }
    }
    out.extend(o);
    out
}

fn push_item(o: &mut Obj, key: String, item: Value) {
    match o.get_mut(&key) {
        Some(Value::Array(items)) => items.push(item),
        _ => {
            o.insert(key, Value::Array(vec![item]));
        }
    }
}

/// Keys whose value is an object name or a `Table.Object` reference.
fn is_reference_key(key: &str) -> bool {
    NAME_KEYS.contains(&key)
        || DOTTED_KEYS.iter().any(|(k, _)| *k == key)
        || matches!(key, "fromColumn" | "toColumn")
}

/// Types an unquoted `key: value` text: names are unquoted, `Table.Object` references become
/// objects, known text properties stay text, and other values become booleans, `null` or
/// numbers when they read as such.
fn scalar(key: &str, v: &str) -> Value {
    if let Some((_, second)) = DOTTED_KEYS.iter().find(|(k, _)| *k == key) {
        return match reference(v).as_deref() {
            Some([table, other]) => {
                let mut o = Obj::new();
                o.insert("table".into(), Value::String(table.clone()));
                o.insert((*second).into(), Value::String(other.clone()));
                Value::Object(o)
            }
            _ => Value::String(v.to_string()),
        };
    }
    if NAME_KEYS.contains(&key) {
        return Value::String(name_value(v));
    }
    if STRING_KEYS.contains(&key) {
        return Value::String(v.to_string());
    }
    retyped(v).unwrap_or_else(|| Value::String(v.to_string()))
}

/// The JSON value an unquoted text stands for, if it is not plain text.
pub(crate) fn retyped(v: &str) -> Option<Value> {
    match v {
        "true" => return Some(Value::Bool(true)),
        "false" => return Some(Value::Bool(false)),
        "null" => return Some(Value::Null),
        _ => {}
    }
    if !v.starts_with(|c: char| c == '-' || c.is_ascii_digit()) {
        return None;
    }
    // Only numbers whose JSON spelling is exactly `v`, so writing them back is lossless.
    let n: serde_json::Number = serde_json::from_str(v).ok()?;
    (n.to_string() == v).then_some(Value::Number(n))
}

/// An object name written as a property value: `'quoted '' name'` or bare text.
fn name_value(v: &str) -> String {
    if v.starts_with('\'')
        && let Ok((name, rest)) = object_name(v)
        && rest.trim().is_empty()
    {
        return name;
    }
    v.to_string()
}

/// Pretty-printed JSON (as the writer prints it) becomes a JSON value; anything else is text.
fn json_text(v: &str) -> Value {
    if v.starts_with(['{', '['])
        && let Ok(parsed) = serde_json::from_str::<Value>(v)
        && serde_json::to_string_pretty(&parsed).is_ok_and(|s| s == v)
    {
        return parsed;
    }
    Value::String(v.to_string())
}

fn required_name<'a>(file: &str, node: &'a Node) -> Result<&'a str, Error> {
    node.name.as_deref().ok_or_else(|| Error::Syntax {
        file: file.to_string(),
        line: node.line,
        message: format!("`{}` needs a name", node.keyword),
    })
}

/// Takes typed properties out of a TMSL-shaped object; what is left becomes `extra`.
struct Take(Obj);

impl Take {
    fn take<T>(&mut self, key: &str, f: impl FnOnce(&Value) -> Option<T>) -> Option<T> {
        let value = f(self.0.get(key)?)?;
        self.0.shift_remove(key);
        Some(value)
    }

    fn string(&mut self, key: &str) -> Option<String> {
        self.take(key, |v| v.as_str().map(str::to_string))
    }

    /// `true` is typed; an explicit `false` stays in `extra`.
    fn flag(&mut self, key: &str) -> bool {
        self.take(key, |v| (v.as_bool() == Some(true)).then_some(true))
            .unwrap_or(false)
    }

    fn objects(&mut self, key: &str) -> Vec<Obj> {
        self.take(key, |v| {
            let items = v.as_array()?;
            items
                .iter()
                .map(|i| i.as_object().cloned())
                .collect::<Option<Vec<_>>>()
        })
        .unwrap_or_default()
    }

    /// Annotations are typed only when every entry is exactly `{name, value}` text.
    fn annotations(&mut self) -> Vec<Annotation> {
        self.take("annotations", |v| {
            v.as_array()?
                .iter()
                .map(|a| {
                    let a = a.as_object()?;
                    if a.len() != 2 {
                        return None;
                    }
                    Some(Annotation {
                        name: a.get("name")?.as_str()?.to_string(),
                        value: a.get("value")?.as_str()?.to_string(),
                    })
                })
                .collect::<Option<Vec<_>>>()
        })
        .unwrap_or_default()
    }

    fn extra(self) -> Properties {
        self.0
    }
}

/// Restores the type TMDL leaves implied on the columns of a calculated table: those without an
/// expression are calculated table columns.
fn calculated_table_columns(o: &mut Obj) {
    let calculated = o.get("partitions").and_then(Value::as_array).is_some_and(|ps| {
        ps.iter()
            .any(|p| p.pointer("/source/type").is_some_and(|t| t == "calculated"))
    });
    if !calculated {
        return;
    }
    if let Some(Value::Array(columns)) = o.get_mut("columns") {
        for c in columns.iter_mut().filter_map(Value::as_object_mut) {
            if !c.contains_key("type") && !c.contains_key("expression") {
                c.insert("type".into(), Value::String(CALCULATED_TABLE_COLUMN.into()));
            }
        }
    }
}

/// Restores the partition TMDL leaves implied on a calculation group table.
fn calculation_group_partition(o: &mut Obj) {
    if !o.contains_key("calculationGroup") || o.contains_key("partitions") {
        return;
    }
    let mut source = Obj::new();
    source.insert("type".into(), Value::String("calculationGroup".into()));
    let mut p = Obj::new();
    p.insert("name".into(), Value::String(CALCULATION_GROUP_PARTITION.into()));
    p.insert("mode".into(), Value::String("import".into()));
    p.insert("source".into(), Value::Object(source));
    o.insert("partitions".into(), Value::Array(vec![Value::Object(p)]));
}

fn table(mut o: Obj) -> Table {
    calculated_table_columns(&mut o);
    calculation_group_partition(&mut o);
    let mut p = Take(o);
    Table {
        name: p.string("name").unwrap_or_default(),
        description: p.string("description"),
        lineage_tag: p.string("lineageTag"),
        is_hidden: p.flag("isHidden"),
        columns: p.objects("columns").into_iter().map(column).collect(),
        measures: p.objects("measures").into_iter().map(measure).collect(),
        partitions: p.objects("partitions").into_iter().map(partition).collect(),
        annotations: p.annotations(),
        extra: p.extra(),
    }
}

fn column(o: Obj) -> Column {
    let mut p = Take(o);
    Column {
        name: p.string("name").unwrap_or_default(),
        expression: p.string("expression"),
        data_type: p
            .take("dataType", |v| {
                let t = DataType::from_tmsl(v.as_str()?);
                (t != DataType::Unknown).then_some(t)
            })
            .unwrap_or_default(),
        source_column: p.string("sourceColumn"),
        format_string: p.string("formatString"),
        display_folder: p.string("displayFolder"),
        summarize_by: p.string("summarizeBy"),
        sort_by_column: p.string("sortByColumn"),
        data_category: p.string("dataCategory"),
        description: p.string("description"),
        lineage_tag: p.string("lineageTag"),
        is_hidden: p.flag("isHidden"),
        annotations: p.annotations(),
        extra: p.extra(),
    }
}

fn measure(o: Obj) -> Measure {
    let mut p = Take(o);
    Measure {
        name: p.string("name").unwrap_or_default(),
        expression: p.string("expression").unwrap_or_default(),
        format_string: p.string("formatString"),
        display_folder: p.string("displayFolder"),
        description: p.string("description"),
        lineage_tag: p.string("lineageTag"),
        is_hidden: p.flag("isHidden"),
        annotations: p.annotations(),
        extra: p.extra(),
    }
}

fn partition(o: Obj) -> Partition {
    let mut p = Take(o);
    let name = p.string("name").unwrap_or_default();
    let mode = p.string("mode");
    let mut source = Take(p.take("source", |v| v.as_object().cloned()).unwrap_or_default());
    Partition {
        name,
        mode,
        source_type: source.string("type").unwrap_or_default(),
        expression: source.string("expression"),
        extra: p.extra(),
        source_extra: source.extra(),
    }
}

fn relationship(o: Obj) -> Relationship {
    let mut p = Take(o);
    let mut end = |key: &str| {
        p.take(key, |v| match reference(v.as_str()?).as_deref() {
            Some([table, column]) => Some((table.clone(), column.clone())),
            _ => None,
        })
        .unwrap_or_default()
    };
    let (from_table, from_column) = end("fromColumn");
    let (to_table, to_column) = end("toColumn");
    Relationship {
        name: p.string("name").unwrap_or_default(),
        from_table,
        from_column,
        to_table,
        to_column,
        is_active: p
            .take("isActive", |v| (v.as_bool() == Some(false)).then_some(false))
            .unwrap_or(true),
        cross_filtering_behavior: p.string("crossFilteringBehavior"),
        from_cardinality: p.string("fromCardinality"),
        to_cardinality: p.string("toCardinality"),
        annotations: p.annotations(),
        extra: p.extra(),
    }
}

fn expression(o: Obj) -> NamedExpression {
    let mut p = Take(o);
    // An explicit `kind: m` (the default the writer omits) stays in `extra` as well.
    let kind = match p.0.get("kind").and_then(Value::as_str) {
        Some("m") => "m".to_string(),
        Some(_) => p.string("kind").unwrap_or_default(),
        None => "m".to_string(),
    };
    NamedExpression {
        name: p.string("name").unwrap_or_default(),
        kind,
        expression: p.string("expression").unwrap_or_default(),
        description: p.string("description"),
        lineage_tag: p.string("lineageTag"),
        annotations: p.annotations(),
        extra: p.extra(),
    }
}

fn role(o: Obj) -> Role {
    let mut p = Take(o);
    Role {
        name: p.string("name").unwrap_or_default(),
        model_permission: p.string("modelPermission"),
        table_permissions: p
            .objects("tablePermissions")
            .into_iter()
            .map(|t| {
                let mut t = Take(t);
                TablePermission {
                    table: t.string("name").unwrap_or_default(),
                    filter_expression: t.string("filterExpression"),
                    extra: t.extra(),
                }
            })
            .collect(),
        annotations: p.annotations(),
        extra: p.extra(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retyping_is_exact() {
        assert_eq!(retyped("12"), Some(Value::from(12)));
        assert_eq!(retyped("-1"), Some(Value::from(-1)));
        assert_eq!(retyped("1.5"), Some(serde_json::json!(1.5)));
        assert_eq!(retyped("007"), None);
        assert_eq!(retyped("1e3"), None);
        assert_eq!(retyped("-0"), None);
        assert_eq!(retyped("1.50"), None);
        assert_eq!(retyped("True"), None);
        assert_eq!(retyped("null"), Some(Value::Null));
        assert_eq!(retyped("99999999999999999999999"), None);
    }

    #[test]
    fn json_text_needs_the_writer_spelling() {
        assert!(json_text("{\n  \"a\": 1\n}").is_object());
        assert!(json_text("{\"a\":1}").is_string());
        assert!(json_text("[x").is_string());
    }
}
