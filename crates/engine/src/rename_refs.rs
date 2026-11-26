//! Keeping name-keyed model metadata consistent through renames and measure moves (G15).
//!
//! Besides DAX ([`crate::dax_rename`]), relationships, sort-by columns and hierarchy levels, a
//! model refers to its objects by name in:
//!
//! - perspectives: `perspectives[].tables[]` (by table name) and their `columns`, `measures`
//!   and `hierarchies` (by object name);
//! - cultures: object translations (`translations.model.tables[]`, their `columns` and
//!   `measures`) and linguistic metadata (`ConceptualEntity` / `ConceptualProperty` bindings,
//!   and the entity keys generated from the names: `sales.amount`, with the relationship roles
//!   that target them);
//! - calculated table columns: `sourceColumn` (`Sales[Amount]` for a column of `= Sales`);
//! - column properties that point at a column or table (`variations`' default column or
//!   hierarchy, `alternateOf`, `relatedColumnDetails`' grouping columns) and calendars.
//!
//! [`follow`] updates them all for one [`NameChange`]. Names compare case-insensitively, as the
//! engine does.

use bicraft_model::SemanticModel;
use serde_json::{Map, Value, json};

/// A renamed or moved model object, with its names before and after.
#[derive(Debug, Clone, Copy)]
pub(crate) enum NameChange<'a> {
    Table {
        old: &'a str,
        new: &'a str,
    },
    Column {
        table: &'a str,
        old: &'a str,
        new: &'a str,
    },
    Measure {
        table: &'a str,
        old: &'a str,
        new: &'a str,
    },
    MeasureMoved {
        name: &'a str,
        from: &'a str,
        to: &'a str,
    },
}

/// Deepest JSON nesting walked in linguistic metadata and column properties.
const MAX_DEPTH: usize = 64;

fn eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn name_is(v: &Value, name: &str) -> bool {
    v.get("name").and_then(Value::as_str).is_some_and(|n| eq(n, name))
}

fn set_name(v: &mut Value, name: &str) {
    if let Some(o) = v.as_object_mut() {
        o.insert("name".into(), json!(name));
    }
}

/// Renames the entry named `old` of the array `key` of `owner` (a perspective or translation
/// table); whether one was renamed.
fn rename_entry(owner: &mut Value, key: &str, old: &str, new: &str) -> bool {
    let mut changed = false;
    for e in owner
        .get_mut(key)
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        if name_is(e, old) {
            set_name(e, new);
            changed = true;
        }
    }
    changed
}

/// The object entries of `tables` (an array of `{name, …}` table entries).
fn tables_of(holder: Option<&mut Value>) -> impl Iterator<Item = &mut Value> {
    holder.and_then(Value::as_array_mut).into_iter().flatten()
}

/// Applies `change` to the table entries of one perspective or one culture's translations
/// (`tables`: `[{name, columns, measures, …}]`); whether anything changed.
fn follow_in_tables(tables: Option<&mut Value>, change: NameChange<'_>) -> bool {
    let Some(tables) = tables.and_then(Value::as_array_mut) else {
        return false;
    };
    let mut changed = false;
    match change {
        NameChange::Table { old, new } => {
            for t in tables.iter_mut().filter(|t| name_is(t, old)) {
                set_name(t, new);
                changed = true;
            }
        }
        NameChange::Column { table, old, new } => {
            for t in tables.iter_mut().filter(|t| name_is(t, table)) {
                changed |= rename_entry(t, "columns", old, new);
            }
        }
        NameChange::Measure { table, old, new } => {
            for t in tables.iter_mut().filter(|t| name_is(t, table)) {
                changed |= rename_entry(t, "measures", old, new);
            }
        }
        NameChange::MeasureMoved { name, from, to } => {
            let mut moved: Vec<Value> = Vec::new();
            for t in tables.iter_mut().filter(|t| name_is(t, from)) {
                let Some(o) = t.as_object_mut() else { continue };
                let Some(list) = o.get_mut("measures").and_then(Value::as_array_mut) else {
                    continue;
                };
                let (taken, kept): (Vec<Value>, Vec<Value>) =
                    std::mem::take(list).into_iter().partition(|m| name_is(m, name));
                *list = kept;
                if list.is_empty() {
                    o.remove("measures");
                }
                moved.extend(taken);
            }
            if !moved.is_empty() {
                changed = true;
                let index = match tables.iter().position(|t| name_is(t, to)) {
                    Some(i) => i,
                    None => {
                        tables.push(json!({ "name": to }));
                        tables.len().saturating_sub(1)
                    }
                };
                if let Some(o) = tables.get_mut(index).and_then(Value::as_object_mut) {
                    let list = o.entry("measures").or_insert_with(|| Value::Array(Vec::new()));
                    if let Some(list) = list.as_array_mut() {
                        list.extend(moved);
                    }
                }
            }
        }
    }
    changed
}

/// Applies `change` to linguistic metadata bindings (`ConceptualEntity` = table,
/// `ConceptualProperty` = column or measure); whether anything changed.
fn follow_in_linguistic(v: &mut Value, change: NameChange<'_>, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    let mut changed = false;
    match v {
        Value::Object(o) => {
            changed |= follow_binding(o, change);
            for child in o.values_mut() {
                changed |= follow_in_linguistic(child, change, depth.saturating_add(1));
            }
        }
        Value::Array(a) => {
            for child in a {
                changed |= follow_in_linguistic(child, change, depth.saturating_add(1));
            }
        }
        _ => {}
    }
    changed
}

fn follow_binding(o: &mut Map<String, Value>, change: NameChange<'_>) -> bool {
    let entity = o
        .get("ConceptualEntity")
        .and_then(Value::as_str)
        .map(str::to_string);
    let property = o
        .get("ConceptualProperty")
        .and_then(Value::as_str)
        .map(str::to_string);
    let Some(entity) = entity else { return false };
    match change {
        NameChange::Table { old, new } if eq(&entity, old) => {
            o.insert("ConceptualEntity".into(), json!(new));
            true
        }
        NameChange::Column { table, old, new } | NameChange::Measure { table, old, new }
            if eq(&entity, table) && property.as_deref().is_some_and(|p| eq(p, old)) =>
        {
            o.insert("ConceptualProperty".into(), json!(new));
            true
        }
        NameChange::MeasureMoved { name, from, to }
            if eq(&entity, from) && property.as_deref().is_some_and(|p| eq(p, name)) =>
        {
            o.insert("ConceptualEntity".into(), json!(to));
            true
        }
        _ => false,
    }
}

/// The linguistic schema key of an object name, as Desktop generates them (`sales`,
/// `sales.total_amount`): lower case, each run of white space one `_`. Assumed from the
/// linguistic schema convention (Desktop's own rename could not be observed: the engine leaves
/// linguistic metadata untouched).
fn ling_key(name: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in name.trim().chars() {
        if c.is_whitespace() {
            space = true;
            continue;
        }
        if space {
            out.push('_');
            space = false;
        }
        out.extend(c.to_lowercase());
    }
    out
}

pub(crate) fn ling_pair(entity: &str, property: Option<&str>) -> String {
    match property {
        Some(p) => format!("{}.{}", ling_key(entity), ling_key(p)),
        None => ling_key(entity),
    }
}

/// The binding of a linguistic entity (`Definition.Binding`, or `Binding`): entity, property.
pub(crate) fn entity_binding(v: &Value) -> Option<(String, Option<String>)> {
    let b = v
        .get("Definition")
        .and_then(|d| d.get("Binding"))
        .or_else(|| v.get("Binding"))?;
    let entity = b.get("ConceptualEntity")?.as_str()?.to_string();
    let property = b
        .get("ConceptualProperty")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some((entity, property))
}

/// Replaces `"Entity": old` references (relationship roles' targets) by the new keys.
fn rekey_references(v: &mut Value, keys: &[(String, String)], depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    match v {
        Value::Object(o) => {
            for (k, child) in o.iter_mut() {
                if k == "Entity"
                    && let Value::String(s) = child
                    && let Some((_, new)) = keys.iter().find(|(old, _)| old == s)
                {
                    *s = new.clone();
                } else {
                    rekey_references(child, keys, depth.saturating_add(1));
                }
            }
        }
        Value::Array(a) => {
            for child in a {
                rekey_references(child, keys, depth.saturating_add(1));
            }
        }
        _ => {}
    }
}

/// Renames the linguistic entity keys generated from the old names (bindings already
/// followed): `sales.amount` becomes `sales.net` when `Sales[Amount]` becomes `Net`. Keys that
/// are not the generated form of the old names, or whose new key is taken, stay.
fn rekey_entities(content: &mut Value, change: NameChange<'_>) -> bool {
    let Some(entities) = content.get_mut("Entities").and_then(Value::as_object_mut) else {
        return false;
    };
    let mut keys: Vec<(String, String)> = Vec::new();
    for (key, v) in entities.iter() {
        let Some((entity, property)) = entity_binding(v) else {
            continue;
        };
        let p = property.as_deref();
        let (old_key, new_key) = match change {
            NameChange::Table { old, new } if eq(&entity, new) => (ling_pair(old, p), ling_pair(new, p)),
            NameChange::Column { table, old, new } | NameChange::Measure { table, old, new }
                if eq(&entity, table) && p.is_some_and(|p| eq(p, new)) =>
            {
                (ling_pair(table, Some(old)), ling_pair(table, Some(new)))
            }
            NameChange::MeasureMoved { name, from, to }
                if eq(&entity, to) && p.is_some_and(|p| eq(p, name)) =>
            {
                (ling_pair(from, Some(name)), ling_pair(to, Some(name)))
            }
            _ => continue,
        };
        if *key == old_key && new_key != old_key && !entities.contains_key(&new_key) {
            keys.push((old_key, new_key));
        }
    }
    if keys.is_empty() {
        return false;
    }
    let old = std::mem::take(entities);
    for (k, v) in old {
        let k = keys.iter().find(|(o, _)| *o == k).map_or(k, |(_, n)| n.clone());
        entities.insert(k, v);
    }
    rekey_references(content, &keys, 0);
    true
}

/// Linguistic metadata `content`: a JSON object, or JSON text (rewritten only when it changes).
fn follow_in_content(content: &mut Value, change: NameChange<'_>) -> bool {
    match content {
        Value::String(text) => {
            let Ok(mut parsed) = serde_json::from_str::<Value>(text) else {
                return false;
            };
            let bound = follow_in_linguistic(&mut parsed, change, 0);
            if !(rekey_entities(&mut parsed, change) || bound) {
                return false;
            }
            match serde_json::to_string(&parsed) {
                Ok(t) => {
                    *text = t;
                    true
                }
                Err(_) => false,
            }
        }
        other => {
            let bound = follow_in_linguistic(other, change, 0);
            rekey_entities(other, change) || bound
        }
    }
}

/// A calculated table column's `sourceColumn` (`T[C]` or `'T'[C]`) as `(table, quoted, rest)`,
/// `rest` being the `[C]` part as written.
fn split_source(source: &str) -> Option<(String, bool, &str)> {
    if let Some(body) = source.strip_prefix('\'') {
        let mut table = String::new();
        let mut chars = body.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '\'' {
                if chars.peek().is_some_and(|(_, n)| *n == '\'') {
                    chars.next();
                    table.push('\'');
                } else {
                    let rest = body.get(i.checked_add(1)?..)?;
                    return rest.starts_with('[').then_some((table, true, rest));
                }
            } else {
                table.push(c);
            }
        }
        None
    } else {
        let open = source.find('[')?;
        let table = source.get(..open)?;
        (!table.is_empty()).then(|| (table.to_string(), false, source.get(open..).unwrap_or_default()))
    }
}

/// The `(table, column)` a calculated table column's `sourceColumn` names. The engine writes
/// `Table[Column]` with both names as they are (`Order's Lines[Unit]Price]`, observed on Desktop
/// 2.158); the DAX form `'Table'[Column]` (quotes and `]` doubled) is read too.
pub(crate) fn source_parts(source: &str) -> Option<(String, bool, String)> {
    let (table, quoted, rest) = split_source(source)?;
    let column = rest.strip_prefix('[')?.strip_suffix(']')?;
    let column = if quoted {
        column.replace("]]", "]")
    } else {
        column.to_string()
    };
    Some((table, quoted, column))
}

/// The new `sourceColumn` of a calculated table column for `change`, when it changes; written
/// in the form it was read.
fn follow_source(source: &str, change: NameChange<'_>) -> Option<String> {
    let (table, quoted, column) = source_parts(source)?;
    let (new_table, new_column) = match change {
        NameChange::Table { old, new } if eq(&table, old) => (new.to_string(), column),
        NameChange::Column { table: t, old, new } if eq(&table, t) && eq(&column, old) => {
            (table.clone(), new.to_string())
        }
        _ => return None,
    };
    Some(if quoted {
        format!(
            "'{}'[{}]",
            new_table.replace('\'', "''"),
            new_column.replace(']', "]]")
        )
    } else {
        format!("{new_table}[{new_column}]")
    })
}

/// `{table, column}` / `{table, hierarchy}` pairs and `baseTable` names inside a column
/// property (`variations`, `alternateOf`, …); whether anything changed.
fn follow_in_pairs(v: &mut Value, change: NameChange<'_>, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    let mut changed = false;
    match v {
        Value::Object(o) => {
            let table = o.get("table").and_then(Value::as_str).map(str::to_string);
            match (change, table.as_deref()) {
                (NameChange::Table { old, new }, Some(t)) if eq(t, old) => {
                    o.insert("table".into(), json!(new));
                    changed = true;
                }
                (NameChange::Column { table, old, new }, Some(t))
                    if eq(t, table)
                        && o.get("column")
                            .and_then(Value::as_str)
                            .is_some_and(|c| eq(c, old)) =>
                {
                    o.insert("column".into(), json!(new));
                    changed = true;
                }
                _ => {}
            }
            if let NameChange::Table { old, new } = change
                && o.get("baseTable")
                    .and_then(Value::as_str)
                    .is_some_and(|t| eq(t, old))
            {
                o.insert("baseTable".into(), json!(new));
                changed = true;
            }
            for child in o.values_mut() {
                changed |= follow_in_pairs(child, change, depth.saturating_add(1));
            }
        }
        Value::Array(a) => {
            for child in a {
                changed |= follow_in_pairs(child, change, depth.saturating_add(1));
            }
        }
        _ => {}
    }
    changed
}

/// Renames a column named `old` in the plain column-name properties of a table's own objects:
/// grouping columns of `relatedColumnDetails`, calendar column groups.
fn rename_string(v: &mut Value, old: &str, new: &str) -> bool {
    match v {
        Value::String(s) if eq(s, old) => {
            *s = new.to_string();
            true
        }
        Value::Array(a) => {
            let mut changed = false;
            for x in a {
                changed |= rename_string(x, old, new);
            }
            changed
        }
        _ => false,
    }
}

/// Follows `change` in every name-keyed place of `m` listed in the module documentation.
/// Returns one entry per updated object: `{kind, name | culture | table, …}`.
pub(crate) fn follow(m: &mut SemanticModel, change: NameChange<'_>) -> Vec<Value> {
    let mut out = Vec::new();
    for p in tables_of(m.extra.get_mut("perspectives")) {
        let name = p
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if follow_in_tables(p.get_mut("tables"), change) {
            out.push(json!({"kind": "perspective", "name": name}));
        }
    }
    for c in tables_of(m.extra.get_mut("cultures")) {
        let culture = c
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let tables = c
            .get_mut("translations")
            .and_then(|t| t.get_mut("model"))
            .and_then(|t| t.get_mut("tables"));
        if follow_in_tables(tables, change) {
            out.push(json!({"kind": "translations", "culture": culture}));
        }
        if let Some(content) = c.get_mut("linguisticMetadata").and_then(|l| l.get_mut("content"))
            && follow_in_content(content, change)
        {
            out.push(json!({"kind": "linguistic metadata", "culture": culture}));
        }
    }
    for t in &mut m.tables {
        let table_name = t.name.clone();
        let own_column = match change {
            NameChange::Column { table, old, new } if eq(&table_name, table) => Some((old, new)),
            _ => None,
        };
        for c in &mut t.columns {
            let calculated_table_column =
                c.extra.get("type").and_then(Value::as_str) == Some("calculatedTableColumn");
            if calculated_table_column
                && let Some(new) = c.source_column.as_deref().and_then(|s| follow_source(s, change))
            {
                c.source_column = Some(new);
                out.push(json!({"kind": "calculated table column", "table": table_name, "name": c.name}));
            }
            let mut changed = false;
            for (key, v) in c.extra.iter_mut() {
                if matches!(
                    key.as_str(),
                    "variations" | "alternateOf" | "relatedColumnDetails"
                ) {
                    changed |= follow_in_pairs(v, change, 0);
                }
            }
            if let Some((old, new)) = own_column
                && let Some(groups) = c
                    .extra
                    .get_mut("relatedColumnDetails")
                    .and_then(|d| d.get_mut("groupByColumns"))
                    .and_then(Value::as_array_mut)
            {
                for g in groups {
                    if let Some(gc) = g.get_mut("groupingColumn") {
                        changed |= rename_string(gc, old, new);
                    }
                }
            }
            if changed {
                out.push(json!({"kind": "column properties", "table": table_name, "name": c.name}));
            }
        }
        if let Some((old, new)) = own_column {
            for cal in t
                .extra
                .get_mut("calendars")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                let name = cal
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let mut changed = false;
                for g in cal
                    .get_mut("calendarColumnGroups")
                    .and_then(Value::as_array_mut)
                    .into_iter()
                    .flatten()
                {
                    for key in ["primaryColumn", "associatedColumns", "columns"] {
                        if let Some(v) = g.get_mut(key) {
                            changed |= rename_string(v, old, new);
                        }
                    }
                }
                if changed {
                    out.push(json!({"kind": "calendar", "table": table_name, "name": name}));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_columns() {
        let t = NameChange::Table {
            old: "Sales",
            new: "Order's",
        };
        assert_eq!(
            follow_source("Sales[Amount]", t).as_deref(),
            Some("Order's[Amount]")
        );
        assert_eq!(
            follow_source("'sales'[Amount]", t).as_deref(),
            Some("'Order''s'[Amount]")
        );
        assert_eq!(follow_source("[Amount]", t), None);
        assert_eq!(follow_source("Other[Amount]", t), None);
        let c = NameChange::Column {
            table: "Order Lines",
            old: "Qty",
            new: "Units]",
        };
        assert_eq!(
            follow_source("'Order Lines'[qty]", c).as_deref(),
            Some("'Order Lines'[Units]]]")
        );
        // The engine's form keeps names as they are.
        assert_eq!(
            follow_source("Order Lines[Qty]", c).as_deref(),
            Some("Order Lines[Units]]")
        );
        assert_eq!(
            follow_source(
                "Order's Lines[Unit]Price]",
                NameChange::Column {
                    table: "Order's Lines",
                    old: "Unit]Price",
                    new: "Price"
                }
            )
            .as_deref(),
            Some("Order's Lines[Price]")
        );
        assert_eq!(follow_source("'unterminated[Qty]", c), None);
    }
}
