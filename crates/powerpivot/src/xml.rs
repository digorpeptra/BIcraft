// The CREATE MEASURE splitter (`parse_create_measures`, `delimited`, `find_ci`), the XML name
// decoder (`decode_xml_name`) and `object_id` are ported from DashCraft (MIT OR Apache-2.0),
// crates/pbi-vertipaq/src/metadata/xml.rs@0958fd82b04656ee1797101a2b4df8d114dfebd1
// Copyright (c) 2026 Jack Sweeney and the DashCraft contributors. Used under the MIT License.
// See LICENSE-MIT-dashcraft in crates/xpress8 and THIRD_PARTY_NOTICES.md at the repository root.
// The rest is BIcraft's own, written from docs/formats/powerpivot.md.

//! The ASSL XML metadata of a Power Pivot backup (compatibility levels 1100/1103) → a
//! [`SemanticModel`]: one `Load` document per object, found by file name
//! (`docs/formats/powerpivot.md`, "Metadata").

use std::collections::HashMap;

use bicraft_model::{Column, DataType, Measure, Partition, Relationship, SemanticModel, Table};
use roxmltree::{Document, Node, ParsingOptions};
use serde_json::{Value, json};

use crate::Error;
use crate::backup::{Backup, ends_with_ci};

/// One column of a table, as its storage names it.
#[derive(Debug, Clone)]
pub(crate) struct ColumnSource {
    pub name: String,
    /// The attribute ID: the name of the column's `XMRawColumn` in the table storage.
    pub attribute_id: String,
    /// The type from the attribute's key column; `Unknown` for a calculated column (`Empty`).
    pub data_type: DataType,
}

/// One table and where its data is.
#[derive(Debug, Clone)]
pub(crate) struct TableSource {
    pub name: String,
    pub dimension_id: String,
    pub columns: Vec<ColumnSource>,
}

pub(crate) struct Loaded {
    pub model: SemanticModel,
    pub tables: Vec<TableSource>,
    pub compatibility_level: Option<u32>,
    pub warnings: Vec<String>,
}

pub(crate) fn parse<'t>(text: &'t str, backup: &Backup<'_>, what: &str) -> Result<Document<'t>, Error> {
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: backup.limits.max_xml_nodes,
        ..ParsingOptions::default()
    };
    Document::parse_with_options(text, options).map_err(|e| Error::Metadata(format!("{what}: {e}")))
}

pub(crate) fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}

pub(crate) fn children<'a, 'i: 'a>(
    node: Node<'a, 'i>,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    node.children()
        .filter(move |n| n.is_element() && n.tag_name().name() == name)
}

/// The trimmed text of child `name`; `None` when absent or empty.
pub(crate) fn text(node: Node<'_, '_>, name: &str) -> Option<String> {
    let element = child(node, name)?;
    let value: String = element
        .children()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect();
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub(crate) fn flag(node: Node<'_, '_>, name: &str) -> Option<bool> {
    text(node, name).map(|v| v.eq_ignore_ascii_case("true") || v == "1")
}

/// `xsi:type` without its prefix (`ddl200_200:ExpressionBinding` → `ExpressionBinding`).
fn xsi_type(node: Node<'_, '_>) -> Option<String> {
    node.attributes()
        .find(|a| a.name() == "type")
        .map(|a| a.value().rsplit(':').next().unwrap_or_default().to_owned())
}

/// The first element named `name` in the document (the object of a `Load` document).
fn object<'a, 'i>(doc: &'a Document<'i>, name: &str) -> Option<Node<'a, 'i>> {
    doc.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}

/// `<id>.<n>.<kind>.xml` → `<id>` (IDs may contain dots).
pub(crate) fn object_id(file_name: &str, kind: &str) -> Option<String> {
    let suffix = format!(".{kind}.xml");
    if !ends_with_ci(file_name, &suffix) {
        return None;
    }
    let stem = file_name.get(..file_name.len() - suffix.len())?;
    let (id, version) = stem.rsplit_once('.')?;
    (!version.is_empty() && version.chars().all(|c| c.is_ascii_digit())).then(|| id.to_owned())
}

/// Decodes XML-encoded names (`Plant_x0020_ID` → `Plant ID`).
pub(crate) fn decode_xml_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(i) = rest.find("_x") {
        out.push_str(rest.get(..i).unwrap_or_default());
        let tail = rest.get(i..).unwrap_or_default();
        let decoded = tail
            .get(2..6)
            .filter(|hex| hex.chars().all(|c| c.is_ascii_hexdigit()))
            .filter(|_| tail.get(6..7) == Some("_"))
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .and_then(char::from_u32);
        match decoded {
            Some(c) => {
                out.push(c);
                rest = tail.get(7..).unwrap_or_default();
            }
            None => {
                out.push_str("_x");
                rest = tail.get(2..).unwrap_or_default();
            }
        }
    }
    out.push_str(rest);
    out
}

fn find_ci(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    let hay = haystack.as_bytes();
    let needle = needle.as_bytes();
    (from..=hay.len().checked_sub(needle.len())?).find(|&i| {
        hay.get(i..i + needle.len())
            .is_some_and(|w| w.eq_ignore_ascii_case(needle))
    })
}

/// Reads a delimited name (`'…'` with `''`, or `[…]` with `]]`) at the start of `s`; returns
/// the unescaped name and the rest.
fn delimited(s: &str, open: char, close: char) -> Option<(String, &str)> {
    let mut chars = s.strip_prefix(open)?.char_indices().peekable();
    let body = s.get(open.len_utf8()..)?;
    let mut name = String::new();
    while let Some((i, c)) = chars.next() {
        if c == close {
            if chars.peek().map(|&(_, n)| n) == Some(close) {
                chars.next();
                name.push(close);
                continue;
            }
            return Some((name, body.get(i + close.len_utf8()..)?));
        }
        name.push(c);
    }
    None
}

/// Splits `CREATE MEASURE 'Table'[Name] = <DAX>;` statements into (table, name, expression).
pub(crate) fn parse_create_measures(text: &str) -> Vec<(String, String, String)> {
    const KEYWORD: &str = "CREATE MEASURE";
    let mut starts = Vec::new();
    let mut from = 0;
    while let Some(i) = find_ci(text, KEYWORD, from) {
        starts.push(i);
        from = i + KEYWORD.len();
    }
    let mut out = Vec::new();
    for (k, &start) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(text.len());
        let Some(statement) = text.get(start + KEYWORD.len()..end) else {
            continue;
        };
        let rest = statement.trim_start();
        let (table, rest) = if rest.starts_with('\'') {
            match delimited(rest, '\'', '\'') {
                Some(t) => t,
                None => continue,
            }
        } else {
            let cut = rest.find('[').unwrap_or(rest.len());
            (
                rest.get(..cut).unwrap_or_default().trim().to_owned(),
                rest.get(cut..).unwrap_or_default(),
            )
        };
        let Some((name, rest)) = delimited(rest.trim_start(), '[', ']') else {
            continue;
        };
        let Some(expression) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let expression = expression.trim();
        let expression = expression.strip_suffix(';').unwrap_or(expression).trim_end();
        out.push((table, name, expression.to_owned()));
    }
    out
}

/// The model type of an ASSL key column `DataType`.
pub(crate) fn data_type(assl: Option<&str>) -> DataType {
    match assl.unwrap_or("Empty") {
        "WChar" | "Char" | "String" => DataType::String,
        "BigInt" | "Integer" | "SmallInt" | "TinyInt" | "UnsignedBigInt" | "UnsignedInt"
        | "UnsignedSmallInt" | "UnsignedTinyInt" => DataType::Int64,
        "Double" | "Single" => DataType::Double,
        "Currency" => DataType::Decimal,
        "Date" => DataType::DateTime,
        "Boolean" => DataType::Boolean,
        "Binary" => DataType::Binary,
        _ => DataType::Unknown,
    }
}

#[derive(Debug, Default)]
struct Attribute {
    id: String,
    name: String,
    description: Option<String>,
    row_number: bool,
    data_type: Option<String>,
    source_column: Option<String>,
    expression: Option<String>,
    visible: bool,
    display_folder: Option<String>,
    format_string: Option<String>,
    order_by: Option<String>,
}

#[derive(Debug, Default)]
struct End {
    dimension: String,
    attribute: String,
    multiplicity: Option<String>,
}

#[derive(Debug, Default)]
struct XmlRelationship {
    id: String,
    visible: bool,
    from: End,
    to: End,
}

#[derive(Debug, Default)]
struct Dimension {
    id: String,
    name: String,
    description: Option<String>,
    attributes: Vec<Attribute>,
    hierarchies: Vec<Value>,
    relationships: Vec<XmlRelationship>,
}

fn parse_attribute(node: Node<'_, '_>) -> Attribute {
    let key = child(node, "KeyColumns").and_then(|k| child(k, "KeyColumn"));
    let source = key.and_then(|k| child(k, "Source"));
    let binding = source.and_then(xsi_type).unwrap_or_default();
    let id = text(node, "ID").unwrap_or_default();
    let mut a = Attribute {
        name: text(node, "Name").unwrap_or_else(|| id.clone()),
        id,
        description: text(node, "Description"),
        row_number: text(node, "Type").as_deref() == Some("RowNumber") || binding == "RowNumberBinding",
        data_type: key.and_then(|k| text(k, "DataType")),
        visible: flag(node, "AttributeHierarchyVisible").unwrap_or(true),
        display_folder: text(node, "AttributeHierarchyDisplayFolder"),
        format_string: text(node, "FormatString"),
        ..Attribute::default()
    };
    if text(node, "OrderBy").as_deref() == Some("AttributeKey") {
        a.order_by = text(node, "OrderByAttributeID");
    }
    match (binding.as_str(), source) {
        ("ExpressionBinding", Some(s)) => a.expression = text(s, "Expression"),
        ("ColumnBinding", Some(s)) => a.source_column = text(s, "ColumnID").map(|c| decode_xml_name(&c)),
        _ => {}
    }
    a
}

fn parse_end(node: Node<'_, '_>) -> End {
    End {
        dimension: text(node, "DimensionID").unwrap_or_default(),
        attribute: child(node, "Attributes")
            .and_then(|a| child(a, "Attribute"))
            .and_then(|a| text(a, "AttributeID"))
            .unwrap_or_default(),
        multiplicity: text(node, "Multiplicity"),
    }
}

fn parse_dimension(doc: &Document<'_>) -> Option<Dimension> {
    let node = object(doc, "Dimension")?;
    let mut dim = Dimension {
        id: text(node, "ID").unwrap_or_default(),
        name: text(node, "Name").unwrap_or_default(),
        description: text(node, "Description"),
        ..Dimension::default()
    };
    if let Some(attrs) = child(node, "Attributes") {
        dim.attributes = children(attrs, "Attribute").map(parse_attribute).collect();
    }
    let names: HashMap<String, String> = dim
        .attributes
        .iter()
        .map(|a| (a.id.clone(), a.name.clone()))
        .collect();
    if let Some(list) = child(node, "Hierarchies") {
        for h in children(list, "Hierarchy") {
            let levels: Vec<Value> = child(h, "Levels")
                .map(|l| {
                    children(l, "Level")
                        .enumerate()
                        .map(|(i, level)| {
                            let attr = text(level, "SourceAttributeID").unwrap_or_default();
                            json!({
                                "name": text(level, "Name").unwrap_or_else(|| attr.clone()),
                                "ordinal": i,
                                "column": names.get(&attr).cloned().unwrap_or(attr),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let mut h_json = serde_json::Map::new();
            h_json.insert("name".into(), json!(text(h, "Name").unwrap_or_default()));
            if let Some(d) = text(h, "Description") {
                h_json.insert("description".into(), json!(d));
            }
            if flag(h, "Visible") == Some(false) {
                h_json.insert("isHidden".into(), json!(true));
            }
            if let Some(f) = text(h, "DisplayFolder") {
                h_json.insert("displayFolder".into(), json!(f));
            }
            h_json.insert("levels".into(), Value::Array(levels));
            dim.hierarchies.push(Value::Object(h_json));
        }
    }
    if let Some(list) = child(node, "Relationships") {
        for r in children(list, "Relationship") {
            let (Some(from), Some(to)) = (child(r, "FromRelationshipEnd"), child(r, "ToRelationshipEnd"))
            else {
                continue;
            };
            dim.relationships.push(XmlRelationship {
                id: text(r, "ID").unwrap_or_default(),
                visible: flag(r, "Visible").unwrap_or(true),
                from: parse_end(from),
                to: parse_end(to),
            });
        }
    }
    Some(dim)
}

/// (ID, name, visible) of each table of the cube, in order.
fn parse_cube(doc: &Document<'_>) -> Option<Vec<(String, String, bool)>> {
    let cube = object(doc, "Cube")?;
    Some(
        child(cube, "Dimensions")
            .map(|d| {
                children(d, "Dimension")
                    .map(|n| {
                        let id = text(n, "DimensionID")
                            .or_else(|| text(n, "ID"))
                            .unwrap_or_default();
                        (
                            id.clone(),
                            text(n, "Name").unwrap_or(id),
                            flag(n, "Visible").unwrap_or(true),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
    )
}

/// What the MDX script's `CalculationProperties` say of a measure.
#[derive(Default)]
struct MeasureProperties {
    format_string: Option<String>,
    description: Option<String>,
    display_folder: Option<String>,
    hidden: bool,
}

/// The last bracketed name of an MDX reference (`[Revenue]`, `Measures.[A]]b]` → `A]b`).
fn last_bracketed(reference: &str) -> Option<String> {
    let mut last = None;
    let mut chars = reference.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '[' {
            continue;
        }
        let mut name = String::new();
        loop {
            match chars.next() {
                Some(']') if chars.peek() == Some(&']') => {
                    chars.next();
                    name.push(']');
                }
                Some(']') => break,
                Some(c) => name.push(c),
                None => return None,
            }
        }
        last = Some(name);
    }
    last
}

/// An MDX string literal (`'#,0'`, `"0%"`, quotes doubled inside) → its text; `None` when
/// empty. Unquoted text is taken as is.
fn mdx_string(literal: &str) -> Option<String> {
    let value = match literal.chars().next() {
        Some(q @ ('\'' | '"')) if literal.len() >= 2 && literal.ends_with(q) => {
            let inner = literal.get(1..literal.len() - 1).unwrap_or_default();
            let doubled: String = [q, q].iter().collect();
            inner.replace(&doubled, &q.to_string())
        }
        _ => literal.to_owned(),
    };
    (!value.is_empty()).then_some(value)
}

/// The M of an imported table's partition: the original source query as a comment, and an
/// error when Power Query runs it (the Power Pivot connection is not translated).
fn partition_m(queries: &[String]) -> String {
    let mut m = String::from("// Imported from an Excel Power Pivot workbook.\n");
    if !queries.is_empty() {
        m.push_str("// Its original source query:\n");
        for q in queries {
            for line in q.lines() {
                m.push_str("//   ");
                m.push_str(line);
                m.push('\n');
            }
        }
    }
    m.push_str(
        "error \"This table was imported from an Excel Power Pivot workbook: its source is not translated. \
         Replace this query to refresh the table.\"",
    );
    m
}

pub(crate) fn load(backup: &Backup<'_>) -> Result<Loaded, Error> {
    let mut warnings = Vec::new();
    let cubes = backup.xml_files("cub");
    let cube_file = cubes
        .iter()
        .find(|f| f.name().starts_with("Model."))
        .or_else(|| cubes.first())
        .ok_or_else(|| Error::Metadata("no cube definition (Model.<n>.cub.xml)".into()))?;
    let cube_text = backup.read_text(cube_file)?;
    let cube_doc = parse(&cube_text, backup, "cube definition")?;
    let cube =
        parse_cube(&cube_doc).ok_or_else(|| Error::Metadata("the cube definition has no Cube".into()))?;

    let mut dimensions: HashMap<String, Dimension> = HashMap::new();
    for file in backup.xml_files("dim") {
        let body = backup.read_text(file)?;
        let doc = parse(&body, backup, "dimension definition")?;
        if let Some(mut dim) = parse_dimension(&doc) {
            if dim.id.is_empty() {
                dim.id = object_id(file.name(), "dim").unwrap_or_default();
            }
            dimensions.insert(dim.id.clone(), dim);
        }
    }

    // partitions: the source queries of each table, by measure group (= dimension) ID
    let mut queries: HashMap<String, Vec<String>> = HashMap::new();
    for file in backup.xml_files("prt") {
        let body = backup.read_text(file)?;
        let doc = parse(&body, backup, "partition definition")?;
        let group = object(&doc, "ParentObject").and_then(|p| text(p, "MeasureGroupID"));
        let query = object(&doc, "Partition")
            .and_then(|p| child(p, "Source"))
            .and_then(|s| text(s, "QueryDefinition"));
        if let (Some(group), Some(query)) = (group, query) {
            queries.entry(group).or_default().push(query);
        }
    }

    let mut measures = Vec::new();
    let mut properties: HashMap<String, MeasureProperties> = HashMap::new();
    for file in backup.xml_files("scr") {
        let body = backup.read_text(file)?;
        let doc = parse(&body, backup, "MDX script")?;
        for command in doc
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "Command")
        {
            if let Some(body) = text(command, "Text") {
                measures.extend(parse_create_measures(&body));
            }
        }
        for p in doc
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "CalculationProperty")
        {
            if let Some(name) = text(p, "CalculationReference").and_then(|r| last_bracketed(&r)) {
                properties.entry(name).or_insert_with(|| MeasureProperties {
                    format_string: text(p, "FormatString").and_then(|f| mdx_string(&f)),
                    description: text(p, "Description"),
                    display_folder: text(p, "DisplayFolder"),
                    hidden: flag(p, "Visible") == Some(false),
                });
            }
        }
    }

    let mut compatibility_level = None;
    let mut database_name = None;
    for file in backup.xml_files("db") {
        let body = backup.read_text(file)?;
        let doc = parse(&body, backup, "database definition")?;
        if let Some(db) = object(&doc, "Database") {
            compatibility_level =
                compatibility_level.or_else(|| text(db, "CompatibilityLevel").and_then(|v| v.parse().ok()));
            database_name = database_name.or_else(|| text(db, "Name"));
        }
    }

    let mut model = SemanticModel {
        name: database_name.unwrap_or_default(),
        ..SemanticModel::default()
    };
    let mut tables = Vec::new();
    for (id, name, visible) in &cube {
        let Some(dim) = dimensions.get(id) else {
            warnings.push(format!("table '{name}': no dimension definition; skipped"));
            continue;
        };
        let name = if name == id && !dim.name.is_empty() {
            dim.name.clone()
        } else {
            name.clone()
        };
        let names: HashMap<&str, &str> = dim
            .attributes
            .iter()
            .map(|a| (a.id.as_str(), a.name.as_str()))
            .collect();
        let mut table = Table {
            name: name.clone(),
            description: dim.description.clone(),
            is_hidden: !visible,
            ..Table::default()
        };
        let mut source = TableSource {
            name: name.clone(),
            dimension_id: id.clone(),
            columns: Vec::new(),
        };
        for a in dim.attributes.iter().filter(|a| !a.row_number) {
            let data_type = data_type(a.data_type.as_deref());
            table.columns.push(Column {
                name: a.name.clone(),
                data_type,
                expression: a.expression.clone(),
                source_column: match a.expression {
                    Some(_) => None,
                    None => Some(a.source_column.clone().unwrap_or_else(|| a.name.clone())),
                },
                format_string: a.format_string.clone(),
                display_folder: a.display_folder.clone(),
                sort_by_column: a
                    .order_by
                    .as_deref()
                    .and_then(|o| names.get(o))
                    .map(|n| (*n).to_owned()),
                description: a.description.clone(),
                is_hidden: !a.visible,
                ..Column::default()
            });
            source.columns.push(ColumnSource {
                name: a.name.clone(),
                attribute_id: a.id.clone(),
                data_type,
            });
        }
        if !dim.hierarchies.is_empty() {
            table
                .extra
                .insert("hierarchies".into(), Value::Array(dim.hierarchies.clone()));
        }
        table.partitions.push(Partition {
            name: name.clone(),
            mode: Some("import".into()),
            source_type: "m".into(),
            expression: Some(partition_m(
                queries.get(id).map(Vec::as_slice).unwrap_or_default(),
            )),
            ..Partition::default()
        });
        model.tables.push(table);
        tables.push(source);
    }

    for (table, name, expression) in measures {
        let props = properties.remove(&name).unwrap_or_default();
        match model.table_mut(&table) {
            Some(t) => t.measures.push(Measure {
                name,
                expression,
                format_string: props.format_string,
                description: props.description,
                display_folder: props.display_folder,
                is_hidden: props.hidden,
                ..Measure::default()
            }),
            None => warnings.push(format!(
                "measure [{name}] belongs to an unknown table '{table}'; skipped"
            )),
        }
    }

    let table_of: HashMap<&str, &str> = tables
        .iter()
        .map(|t| (t.dimension_id.as_str(), t.name.as_str()))
        .collect();
    let column_of = |dim: &str, attr: &str| -> String {
        dimensions
            .get(dim)
            .and_then(|d| d.attributes.iter().find(|a| a.id == attr))
            .map_or_else(|| attr.to_owned(), |a| a.name.clone())
    };
    for (id, _, _) in &cube {
        let Some(dim) = dimensions.get(id) else { continue };
        for r in &dim.relationships {
            let (Some(from), Some(to)) = (
                table_of.get(r.from.dimension.as_str()),
                table_of.get(r.to.dimension.as_str()),
            ) else {
                continue;
            };
            let one = |m: &Option<String>| m.as_deref().is_some_and(|m| m.eq_ignore_ascii_case("one"));
            let many = |m: &Option<String>| m.as_deref().is_some_and(|m| m.eq_ignore_ascii_case("many"));
            model.relationships.push(Relationship {
                name: r.id.clone(),
                from_table: (*from).to_owned(),
                from_column: column_of(&r.from.dimension, &r.from.attribute),
                to_table: (*to).to_owned(),
                to_column: column_of(&r.to.dimension, &r.to.attribute),
                is_active: r.visible,
                from_cardinality: one(&r.from.multiplicity).then(|| "one".to_owned()),
                to_cardinality: many(&r.to.multiplicity).then(|| "many".to_owned()),
                ..Relationship::default()
            });
        }
    }
    Ok(Loaded {
        model,
        tables,
        compatibility_level,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_ids_and_measures_parse() {
        assert_eq!(decode_xml_name("Sub_x0020_Category_x0020_ID"), "Sub Category ID");
        assert_eq!(decode_xml_name("a_xZZZZ_b_x"), "a_xZZZZ_b_x");
        assert_eq!(
            object_id("Date_29d5.fe95.43.dim.xml", "dim").as_deref(),
            Some("Date_29d5.fe95")
        );
        assert_eq!(object_id("x.dim.xml", "dim"), None);
        let m = parse_create_measures(
            "CALCULATE;\nCREATE MEASURE 'Sales ''EU'''[Total [x]]]=SUM(Sales[Amount]);\ncreate measure Sales[Avg] = AVERAGE(Sales[Amount]) ;",
        );
        assert_eq!(
            m,
            vec![
                (
                    "Sales 'EU'".into(),
                    "Total [x]".into(),
                    "SUM(Sales[Amount])".into()
                ),
                ("Sales".into(), "Avg".into(), "AVERAGE(Sales[Amount])".into()),
            ]
        );
        assert!(parse_create_measures("CREATE MEASURE 'unterminated").is_empty());
        assert_eq!(data_type(Some("Currency")), DataType::Decimal);
        assert_eq!(data_type(None), DataType::Unknown);
    }

    #[test]
    fn calculation_properties_parse() {
        assert_eq!(last_bracketed("[Revenue]").as_deref(), Some("Revenue"));
        assert_eq!(last_bracketed("Measures.[A]]b]").as_deref(), Some("A]b"));
        assert_eq!(last_bracketed("[Sales].[Units"), None);
        assert_eq!(last_bracketed("Measures"), None);
        assert_eq!(mdx_string("'#,0'").as_deref(), Some("#,0"));
        assert_eq!(mdx_string("'0'' units'''").as_deref(), Some("0' units'"));
        assert_eq!(mdx_string("\"0%\"").as_deref(), Some("0%"));
        assert_eq!(mdx_string("''"), None);
        assert_eq!(mdx_string("'").as_deref(), Some("'"));
        assert_eq!(mdx_string("Percent").as_deref(), Some("Percent"));
    }
}
