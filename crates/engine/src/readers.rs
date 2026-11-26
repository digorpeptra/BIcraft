//! More files `file.open` reads (ROADMAP M11.36), each into a new, unsaved model (as
//! `model.new` makes one: save it with `file.save_as`), so `objects.import` takes them as a
//! source too (it opens its source with `file.open`):
//!
//! - `.vpax` (VertiPaq Analyzer): the `Model.bim` part when there is one, else the model
//!   `DaxModel.json` describes, and its storage statistics; there is no data;
//! - `.pbids` (Power BI data source file): one table per connection, its query built from the
//!   connection's protocol and address;
//! - `.pq` / `.m` (M text: a section document, or one expression) and `.pqt` (Power Query
//!   template: a ZIP holding `MashupDocument.pq` and its metadata): the queries;
//! - Dataflow Gen2 (`mashup.pq` with its `queryMetadata.json`, either file opened) and the CDM
//!   `model.json` of a dataflow (entities, their attributes and the `pbi:mashup` document).
//!
//! The layouts are in `docs/formats/import-sources.md`, from public descriptions only. Inputs
//! are hostile: sizes are capped, ZIPs read with decompression caps, JSON nesting is capped by
//! the JSON parser (128 levels), M nesting by `bicraft_data::Limits`; nothing here parses XML,
//! so DTDs and entities are never expanded.

use std::collections::HashMap;
use std::path::Path;

use bicraft_model::{Column, DataType, NamedExpression, Partition, Relationship, SemanticModel, Table};
use serde_json::{Map, Value, json};

use crate::Session;

/// Largest text file read (`.pbids`, `.pq`, `.m`, `model.json`, `queryMetadata.json`).
pub const MAX_TEXT_BYTES: u64 = 64 << 20;
/// Largest ZIP read (`.vpax`, `.pqt`).
pub const MAX_ZIP_BYTES: u64 = 512 << 20;
/// Decompression caps of those ZIPs (a ZIP bomb is refused part by part).
const ZIP_LIMITS: bicraft_opc::ReadLimits = bicraft_opc::ReadLimits {
    max_part_bytes: 512 << 20,
    max_total_bytes: 1 << 30,
};
/// Most connections, queries, entities or tables read from one file.
pub const MAX_OBJECTS: usize = 10_000;

/// The kinds of file this module reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Vpax,
    Pbids,
    /// M text (`.pq`, `.m`).
    M,
    Pqt,
    /// A CDM `model.json`.
    Cdm,
    /// A Dataflow Gen2 `queryMetadata.json` (the M document passed apart).
    Dataflow,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Vpax => "vpax",
            Kind::Pbids => "pbids",
            Kind::M => "m",
            Kind::Pqt => "pqt",
            Kind::Cdm => "cdm",
            Kind::Dataflow => "dataflow",
        }
    }
}

/// What a reader made of a file.
#[derive(Debug, Clone)]
pub struct Imported {
    pub kind: Kind,
    pub model: SemanticModel,
    /// `.vpax`: the storage statistics.
    pub statistics: Option<Value>,
    pub notes: Vec<String>,
}

/// The kind of `path` by its extension (`json` files by their content, read by [`read_path`]).
pub fn kind_of_path(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "vpax" => Some("vpax"),
        "pbids" => Some("pbids"),
        "pq" | "m" => Some("m"),
        "pqt" => Some("pqt"),
        "json" => Some("json"),
        _ => None,
    }
}

fn read_capped(path: &Path, cap: u64) -> Result<Vec<u8>, String> {
    let len = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    if len > cap {
        return Err(format!(
            "{}: {len} bytes, more than the {cap} bytes bicraft reads from this kind of file",
            path.display()
        ));
    }
    std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn text_of(bytes: &[u8], what: &str) -> Result<String, String> {
    bicraft_opc::decode_text(what, bytes)
        .map(|(t, _)| t)
        .map_err(|e| format!("{what}: {e}"))
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Model")
        .to_string()
}

/// Reads one file of this module's kinds.
pub fn read_path(path: &Path) -> Result<Imported, String> {
    let name = stem(path);
    match kind_of_path(path) {
        Some("vpax") => read_bytes(Kind::Vpax, &name, &read_capped(path, MAX_ZIP_BYTES)?, None),
        Some("pqt") => read_bytes(Kind::Pqt, &name, &read_capped(path, MAX_ZIP_BYTES)?, None),
        Some("pbids") => read_bytes(Kind::Pbids, &name, &read_capped(path, MAX_TEXT_BYTES)?, None),
        Some("m") => {
            // a Dataflow's mashup.pq reads its queryMetadata.json beside it
            let meta = path.with_file_name("queryMetadata.json");
            let is_mashup = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case("mashup.pq"));
            let bytes = read_capped(path, MAX_TEXT_BYTES)?;
            if is_mashup && meta.is_file() {
                let mbytes = read_capped(&meta, MAX_TEXT_BYTES)?;
                return read_bytes(
                    Kind::Dataflow,
                    &dataflow_name(&mbytes, &name),
                    &mbytes,
                    Some(&bytes),
                );
            }
            read_bytes(Kind::M, &name, &bytes, None)
        }
        Some("json") => {
            let bytes = read_capped(path, MAX_TEXT_BYTES)?;
            let v = parse_json(&bytes, "the JSON file")?;
            if v.get("entities").is_some() {
                return read_bytes(Kind::Cdm, &name, &bytes, None);
            }
            if v.get("queriesMetadata").is_some() {
                let mashup = path.with_file_name("mashup.pq");
                let doc = read_capped(&mashup, MAX_TEXT_BYTES).map_err(|e| {
                    format!("a Dataflow's queryMetadata.json needs its mashup.pq beside it: {e}")
                })?;
                return read_bytes(Kind::Dataflow, &dataflow_name(&bytes, &name), &bytes, Some(&doc));
            }
            Err(format!(
                "{}: not a CDM model.json (no `entities`) nor a Dataflow queryMetadata.json (no `queriesMetadata`)",
                path.display()
            ))
        }
        _ => Err(format!("{}: not a file this reader knows", path.display())),
    }
}

fn dataflow_name(meta: &[u8], fallback: &str) -> String {
    parse_json(meta, "queryMetadata.json")
        .ok()
        .and_then(|v| v.get("name").and_then(Value::as_str).map(str::to_string))
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

/// Reads `bytes` as a file of `kind`; `name` names the model (the file's stem). `doc`: the M
/// document of a [`Kind::Dataflow`] (`bytes` is then its `queryMetadata.json`).
pub fn read_bytes(kind: Kind, name: &str, bytes: &[u8], doc: Option<&[u8]>) -> Result<Imported, String> {
    match kind {
        Kind::Vpax => read_vpax(name, bytes),
        Kind::Pbids => read_pbids(name, bytes),
        Kind::M => {
            let text = text_of(bytes, "the M file")?;
            let mut out = Imported {
                kind,
                model: new_model(name, None),
                statistics: None,
                notes: Vec::new(),
            };
            add_mashup(&mut out, &text, &Meta::default(), &HashMap::new())?;
            Ok(out)
        }
        Kind::Pqt => read_pqt(name, bytes),
        Kind::Cdm => read_cdm(name, bytes),
        Kind::Dataflow => {
            let meta_v = parse_json(bytes, "queryMetadata.json")?;
            let text = text_of(doc.unwrap_or_default(), "mashup.pq")?;
            let culture = meta_v.get("documentLocale").and_then(Value::as_str);
            let mut out = Imported {
                kind,
                model: new_model(name, culture),
                statistics: None,
                notes: Vec::new(),
            };
            add_mashup(&mut out, &text, &Meta::from_doc(&meta_v), &HashMap::new())?;
            Ok(out)
        }
    }
}

fn parse_json(bytes: &[u8], what: &str) -> Result<Value, String> {
    let text = text_of(bytes, what)?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("{what}: {e}"))
}

fn new_model(name: &str, culture: Option<&str>) -> SemanticModel {
    let culture = culture
        .filter(|c| {
            !c.is_empty() && c.len() <= 20 && c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        })
        .unwrap_or("en-US");
    crate::edit::empty_model(name, culture)
}

/// An M string literal.
fn lit(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// A name no other table or expression of `m` has (`base`, then `base (2)`…).
fn free_name(m: &SemanticModel, base: &str) -> String {
    let base = if base.trim().is_empty() {
        "Query"
    } else {
        base.trim()
    };
    let taken = |n: &str| {
        m.tables.iter().any(|t| t.name.eq_ignore_ascii_case(n))
            || m.expressions.iter().any(|e| e.name.eq_ignore_ascii_case(n))
    };
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base} ({i})"))
        .find(|n| !taken(n))
        .unwrap_or_else(|| base.to_string())
}

fn m_table(m: &mut SemanticModel, name: &str, expression: &str, mode: &str, columns: Vec<Column>) -> String {
    let name = free_name(m, name);
    let mut t = Table {
        name: name.clone(),
        lineage_tag: Some(crate::edit::new_guid()),
        columns,
        partitions: vec![Partition {
            name: name.clone(),
            mode: Some(mode.into()),
            source_type: "m".into(),
            expression: Some(expression.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    crate::desktop_defaults::new_table(&mut t);
    m.tables.push(t);
    name
}

// ---------------------------------------------------------------------------------------------
// M documents (.pq, .m, .pqt, Dataflow Gen2, CDM)
// ---------------------------------------------------------------------------------------------

/// The queries' metadata of a Power Query document: which are loaded.
#[derive(Debug, Default)]
struct Meta {
    /// query name → loadEnabled; `None`: no metadata (guess from the expression).
    load: Option<HashMap<String, bool>>,
}

impl Meta {
    /// From a document holding `queriesMetadata` at any depth up to 4.
    fn from_doc(v: &Value) -> Meta {
        fn find(v: &Value, depth: usize) -> Option<&Map<String, Value>> {
            let o = v.as_object()?;
            for (k, x) in o {
                if k.eq_ignore_ascii_case("queriesMetadata") {
                    return x.as_object();
                }
            }
            if depth == 0 {
                return None;
            }
            o.values().find_map(|x| find(x, depth - 1))
        }
        let Some(q) = find(v, 4) else {
            return Meta::default();
        };
        let mut load = HashMap::new();
        for (name, m) in q.iter().take(MAX_OBJECTS) {
            let on = m
                .as_object()
                .and_then(|o| o.iter().find(|(k, _)| k.eq_ignore_ascii_case("loadEnabled")))
                .and_then(|(_, v)| v.as_bool())
                .unwrap_or(false);
            let name = m
                .get("queryName")
                .and_then(Value::as_str)
                .unwrap_or(name)
                .to_string();
            load.insert(name.to_lowercase(), on);
        }
        Meta { load: Some(load) }
    }

    fn loads(&self, name: &str, expression: &str) -> bool {
        match &self.load {
            Some(l) => l.get(&name.to_lowercase()).copied().unwrap_or(false),
            None => !looks_like_function_or_parameter(expression),
        }
    }
}

/// A function (`(x) => …`, `each …`) or a parameter (`… meta [IsParameterQuery = true …]`):
/// Desktop keeps those as queries that load nothing.
fn looks_like_function_or_parameter(e: &str) -> bool {
    let t = e.trim_start();
    let compact: String = e
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    t.starts_with("each ")
        || (t.starts_with('(')
            && compact
                .find(")=>")
                .is_some_and(|f| compact.find("let").is_none_or(|l| f < l)))
        || compact.contains("isparameterquery=true")
}

/// Adds the queries of the M text `doc` (a section document, or one expression named after the
/// model) to `out`: loaded queries become import tables (with `columns` when known, by query
/// name), the others shared expressions.
fn add_mashup(
    out: &mut Imported,
    doc: &str,
    meta: &Meta,
    columns: &HashMap<String, Vec<Column>>,
) -> Result<(), String> {
    let limits = bicraft_data::Limits::default();
    let body = doc.trim_start_matches('\u{feff}');
    let starts_section = body
        .trim_start()
        .get(..7)
        .is_some_and(|s| s.eq_ignore_ascii_case("section"));
    let members: Vec<(String, String)> = if starts_section {
        let (_, members) = bicraft_data::m::scan::section_members(body, &limits)
            .map_err(|e| format!("the M document: {e}"))?;
        members.into_iter().map(|m| (m.name, m.expression)).collect()
    } else {
        if body.trim().is_empty() {
            return Err("the M file is empty".into());
        }
        if body.len() > limits.max_bytes {
            return Err(format!("the M file is larger than {} bytes", limits.max_bytes));
        }
        vec![(out.model.name.clone(), body.trim().to_string())]
    };
    if members.len() > MAX_OBJECTS {
        return Err(format!("more than {MAX_OBJECTS} queries"));
    }
    if members.is_empty() {
        out.notes.push("the document has no query".into());
    }
    for (name, expression) in members {
        if meta.loads(&name, &expression) {
            let cols = columns.get(&name.to_lowercase()).cloned().unwrap_or_default();
            if cols.is_empty() {
                out.notes.push(format!(
                    "table `{name}` has no columns yet: they come from its query (load them with table.import and replace: true, or table.set_data, or refresh in Power BI Desktop)"
                ));
            }
            m_table(&mut out.model, &name, &expression, "import", cols);
        } else {
            let name = free_name(&out.model, &name);
            out.model.expressions.push(NamedExpression {
                name,
                kind: "m".into(),
                expression,
                lineage_tag: Some(crate::edit::new_guid()),
                ..Default::default()
            });
        }
    }
    Ok(())
}

fn read_pqt(name: &str, bytes: &[u8]) -> Result<Imported, String> {
    let pkg = bicraft_opc::Package::read_bytes_with(bytes, &ZIP_LIMITS)
        .map_err(|e| format!("the Power Query template is not a ZIP bicraft can read: {e}"))?;
    let pq = pkg
        .parts
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case("MashupDocument.pq"))
        .or_else(|| {
            pkg.parts
                .iter()
                .find(|p| p.name.to_ascii_lowercase().ends_with(".pq"))
        })
        .or_else(|| {
            pkg.parts
                .iter()
                .find(|p| p.name.to_ascii_lowercase().ends_with(".m"))
        })
        .ok_or("the Power Query template holds no M document (MashupDocument.pq)")?;
    let doc = text_of(&pq.data, &pq.name)?;
    let mut meta = Meta::default();
    let mut culture = None;
    for p in pkg
        .parts
        .iter()
        .filter(|p| p.name.to_ascii_lowercase().ends_with(".json"))
    {
        if let Ok(v) = parse_json(&p.data, &p.name) {
            let m = Meta::from_doc(&v);
            if m.load.is_some() {
                culture = v
                    .get("documentLocale")
                    .or_else(|| v.get("culture"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                meta = m;
                break;
            }
        }
    }
    let mut out = Imported {
        kind: Kind::Pqt,
        model: new_model(name, culture.as_deref()),
        statistics: None,
        notes: Vec::new(),
    };
    add_mashup(&mut out, &doc, &meta, &HashMap::new())?;
    Ok(out)
}

/// The model type of a CDM attribute's `dataType` (Common Data Model, *model.json* metadata
/// file, "Data types").
fn cdm_type(t: &str) -> DataType {
    match t.to_ascii_lowercase().as_str() {
        "string" | "guid" => DataType::String,
        "int64" | "integer" | "int32" => DataType::Int64,
        "double" => DataType::Double,
        "decimal" => DataType::Decimal,
        "datetime" | "datetimeoffset" | "date" | "time" => DataType::DateTime,
        "boolean" => DataType::Boolean,
        _ => DataType::String,
    }
}

fn read_cdm(name: &str, bytes: &[u8]) -> Result<Imported, String> {
    let v = parse_json(bytes, "model.json")?;
    let model_name = v
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or(name);
    let mut out = Imported {
        kind: Kind::Cdm,
        model: new_model(model_name, v.get("culture").and_then(Value::as_str)),
        statistics: None,
        notes: Vec::new(),
    };
    let entities = v
        .get("entities")
        .and_then(Value::as_array)
        .ok_or("model.json: `entities` is not a list")?;
    if entities.len() > MAX_OBJECTS {
        return Err(format!("model.json: more than {MAX_OBJECTS} entities"));
    }
    // each entity's columns, by entity name
    let mut columns: HashMap<String, Vec<Column>> = HashMap::new();
    let mut locations: Vec<(String, Option<String>)> = Vec::new();
    for e in entities {
        let Some(ename) = e.get("name").and_then(Value::as_str) else {
            continue;
        };
        let attrs = e
            .get("attributes")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let mut cols = Vec::new();
        for a in attrs.iter().take(MAX_OBJECTS) {
            let Some(cname) = a.get("name").and_then(Value::as_str) else {
                continue;
            };
            let mut c = Column {
                name: cname.to_string(),
                data_type: cdm_type(a.get("dataType").and_then(Value::as_str).unwrap_or("string")),
                source_column: Some(cname.to_string()),
                lineage_tag: Some(crate::edit::new_guid()),
                description: a.get("description").and_then(Value::as_str).map(str::to_string),
                ..Default::default()
            };
            crate::desktop_defaults::new_column(&mut c, false, None);
            cols.push(c);
        }
        columns.insert(ename.to_lowercase(), cols);
        let location = e
            .get("partitions")
            .and_then(Value::as_array)
            .and_then(|p| p.first())
            .and_then(|p| p.get("location"))
            .and_then(Value::as_str)
            .map(str::to_string);
        locations.push((ename.to_string(), location));
    }
    let mashup = v.get("pbi:mashup");
    let doc = mashup.and_then(|m| m.get("document")).and_then(Value::as_str);
    match doc {
        Some(doc) => {
            let mut meta = mashup.map(Meta::from_doc).unwrap_or_default();
            // every entity loads; the document's other queries follow their metadata
            let load = meta.load.get_or_insert_with(HashMap::new);
            for (e, _) in &locations {
                load.insert(e.to_lowercase(), true);
            }
            add_mashup(&mut out, doc, &meta, &columns)?;
            // entities the document does not define
            for (e, loc) in &locations {
                if out.model.table(e).is_none() {
                    add_entity_without_query(&mut out, e, loc.as_deref(), &mut columns);
                }
            }
        }
        None => {
            for (e, loc) in &locations {
                add_entity_without_query(&mut out, e, loc.as_deref(), &mut columns);
            }
        }
    }
    // relationships (SingleKeyRelationship)
    for r in v
        .get("relationships")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .take(MAX_OBJECTS)
    {
        let end = |k: &str| -> Option<(String, String)> {
            let a = r.get(k)?;
            Some((
                a.get("entityName")?.as_str()?.to_string(),
                a.get("attributeName")?.as_str()?.to_string(),
            ))
        };
        let (Some((ft, fc)), Some((tt, tc))) = (end("fromAttribute"), end("toAttribute")) else {
            continue;
        };
        let has = |t: &str, c: &str| out.model.table(t).is_some_and(|t| t.column(c).is_some());
        if !has(&ft, &fc) || !has(&tt, &tc) {
            out.notes.push(format!(
                "relationship {ft}[{fc}] → {tt}[{tc}] left out: a column is missing"
            ));
            continue;
        }
        out.model.relationships.push(Relationship {
            name: crate::edit::new_guid(),
            from_table: ft,
            from_column: fc,
            to_table: tt,
            to_column: tc,
            is_active: true,
            ..Default::default()
        });
    }
    Ok(out)
}

/// An entity with no query in the document: its CSV partition read from its location (CDM
/// folders hold CSV files without a header row).
fn add_entity_without_query(
    out: &mut Imported,
    entity: &str,
    location: Option<&str>,
    columns: &mut HashMap<String, Vec<Column>>,
) {
    let cols = columns.remove(&entity.to_lowercase()).unwrap_or_default();
    let expr = match location {
        Some(loc) => format!(
            "let\n    Source = Csv.Document(Web.Contents({}), [Columns = {}, Encoding = 65001, QuoteStyle = QuoteStyle.Csv])\nin\n    Source",
            lit(loc),
            cols.len()
        ),
        None => {
            out.notes.push(format!(
                "entity `{entity}` has no query and no partition location"
            ));
            "#table({}, {})".to_string()
        }
    };
    m_table(&mut out.model, entity, &expr, "import", cols);
}

// ---------------------------------------------------------------------------------------------
// .pbids
// ---------------------------------------------------------------------------------------------

fn read_pbids(name: &str, bytes: &[u8]) -> Result<Imported, String> {
    let v = parse_json(bytes, "the .pbids file")?;
    let conns = v
        .get("connections")
        .and_then(Value::as_array)
        .ok_or("the .pbids file has no `connections` list")?;
    if conns.is_empty() {
        return Err("the .pbids file has no connection".into());
    }
    if conns.len() > MAX_OBJECTS {
        return Err(format!("the .pbids file has more than {MAX_OBJECTS} connections"));
    }
    let mut out = Imported {
        kind: Kind::Pbids,
        model: new_model(name, None),
        statistics: None,
        notes: Vec::new(),
    };
    for (i, c) in conns.iter().enumerate() {
        let d = c
            .get("details")
            .ok_or(format!("connection {}: no `details`", i + 1))?;
        let protocol = d
            .get("protocol")
            .and_then(Value::as_str)
            .ok_or(format!("connection {}: no `details.protocol`", i + 1))?;
        let addr = d.get("address").cloned().unwrap_or(Value::Null);
        let a = |k: &str| addr.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
        let query = d
            .get("query")
            .and_then(Value::as_str)
            .filter(|q| !q.trim().is_empty());
        let need = |k: &str| a(k).ok_or(format!("connection {} ({protocol}): no `address.{k}`", i + 1));
        let expr = match protocol.to_ascii_lowercase().as_str() {
            "tds" => {
                let server = need("server")?;
                match (a("database"), query) {
                    (Some(db), Some(q)) => {
                        format!("Sql.Database({}, {}, [Query = {}])", lit(server), lit(db), lit(q))
                    }
                    (Some(db), None) => format!("Sql.Database({}, {})", lit(server), lit(db)),
                    (None, _) => format!("Sql.Databases({})", lit(server)),
                }
            }
            "analysis-services" => {
                let server = need("server")?;
                match a("database") {
                    Some(db) => format!("AnalysisServices.Database({}, {})", lit(server), lit(db)),
                    None => format!("AnalysisServices.Databases({})", lit(server)),
                }
            }
            "folder" => format!("Folder.Files({})", lit(need("path")?)),
            "file" => file_query(need("path")?),
            "odata" => format!("OData.Feed({})", lit(need("url")?)),
            "http" => format!("Web.Contents({})", lit(need("url")?)),
            "sharepoint-list" => format!("SharePoint.Tables({}, [ApiVersion = 15])", lit(need("url")?)),
            other => {
                return Err(format!(
                    "connection {}: protocol `{other}` is not read yet (tds, analysis-services, folder, file, odata, http, sharepoint-list)",
                    i + 1
                ));
            }
        };
        let mode = match c.get("mode").and_then(Value::as_str) {
            Some(m) if m.eq_ignore_ascii_case("directquery") => "directQuery",
            _ => "import",
        };
        let table = m_table(&mut out.model, name, &expr, mode, Vec::new());
        out.notes.push(format!(
            "table `{table}` reads the {protocol} source; its columns come from the query (load them with table.import and replace: true, or refresh in Power BI Desktop)"
        ));
    }
    Ok(out)
}

/// The query of a local file, by its extension (as Desktop's Get data would start it).
fn file_query(path: &str) -> String {
    let ext = path
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    let contents = format!("File.Contents({})", lit(path));
    match ext.as_str() {
        "csv" | "txt" | "tsv" => format!(
            "let\n    Source = Csv.Document({contents}, [Delimiter = \"{}\", Encoding = 65001, QuoteStyle = QuoteStyle.Csv]),\n    Promoted = Table.PromoteHeaders(Source, [PromoteAllScalars = true])\nin\n    Promoted",
            if ext == "tsv" { "#(tab)" } else { "," }
        ),
        "xlsx" | "xlsm" | "xls" | "xlsb" => format!("Excel.Workbook({contents}, null, true)"),
        "json" => format!("Json.Document({contents})"),
        "xml" => format!("Xml.Tables({contents})"),
        "parquet" => format!("Parquet.Document({contents})"),
        _ => contents,
    }
}

// ---------------------------------------------------------------------------------------------
// .vpax
// ---------------------------------------------------------------------------------------------

/// The items of a Json.NET array (`{"$id", "$values": […]}`) or a plain one.
fn items(v: Option<&Value>) -> &[Value] {
    match v {
        Some(Value::Array(a)) => a,
        Some(Value::Object(o)) => o
            .get("$values")
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice),
        _ => &[],
    }
}

/// A name, note or expression: a plain string, or an object holding it.
fn text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => ["Name", "Note", "Expression"]
            .iter()
            .find_map(|k| o.get(*k).and_then(Value::as_str))
            .map(str::to_string),
        _ => None,
    }
    .filter(|s| !s.is_empty())
}

fn num(v: Option<&Value>) -> u64 {
    v.and_then(|x| x.as_u64().or_else(|| x.as_f64().map(|f| f.max(0.0) as u64)))
        .unwrap_or(0)
}

fn read_vpax(name: &str, bytes: &[u8]) -> Result<Imported, String> {
    let pkg = bicraft_opc::Package::read_bytes_with(bytes, &ZIP_LIMITS)
        .map_err(|e| format!("the VPAX file is not a ZIP bicraft can read: {e}"))?;
    let part = |n: &str| pkg.parts.iter().find(|p| p.name.eq_ignore_ascii_case(n));
    let dax = part("DaxModel.json").ok_or("the VPAX file has no DaxModel.json")?;
    let doc = parse_json(&dax.data, "DaxModel.json")?;
    let bim = match part("Model.bim") {
        Some(p) => {
            let t = text_of(&p.data, "Model.bim")?;
            Some(
                bicraft_tmsl::read(t.trim_start_matches('\u{feff}'))
                    .map_err(|e| format!("Model.bim: {e}"))?,
            )
        }
        None => None,
    };
    let mut notes = vec!["a VPAX holds no data: the model opens without rows".to_string()];
    let (model, from) = match bim {
        Some(m) => (m, "Model.bim"),
        None => (dax_model(name, &doc, &mut notes)?, "DaxModel.json"),
    };
    let statistics = vpax_statistics(&doc);
    Ok(Imported {
        kind: Kind::Vpax,
        model,
        statistics: Some(
            json!({"model_from": from, "tables": statistics.0, "relationships": statistics.1,
            "extractor": doc.get("ExtractorApp"), "extraction_date": doc.get("ExtractionDate")}),
        ),
        notes,
    })
}

/// The model `DaxModel.json` describes (when the VPAX has no `Model.bim`).
fn dax_model(name: &str, doc: &Value, notes: &mut Vec<String>) -> Result<SemanticModel, String> {
    let model_name = text(doc.get("ModelName")).unwrap_or_else(|| name.to_string());
    let mut m = new_model(&model_name, doc.get("Culture").and_then(Value::as_str));
    if let Some(level) = doc.get("CompatibilityLevel").and_then(Value::as_u64) {
        m.compatibility_level = u32::try_from(level).ok();
    }
    // column objects by `$id`, for the relationships' references
    let mut by_id: HashMap<String, (String, String)> = HashMap::new();
    let tables = items(doc.get("Tables"));
    if tables.len() > MAX_OBJECTS {
        return Err(format!("DaxModel.json: more than {MAX_OBJECTS} tables"));
    }
    for t in tables {
        let Some(tname) = text(t.get("TableName")) else {
            continue;
        };
        let mut table = Table {
            name: tname.clone(),
            is_hidden: t.get("IsHidden").and_then(Value::as_bool).unwrap_or(false),
            description: text(t.get("Description")),
            lineage_tag: Some(crate::edit::new_guid()),
            ..Default::default()
        };
        for c in items(t.get("Columns")).iter().take(MAX_OBJECTS) {
            let Some(cname) = text(c.get("ColumnName")) else {
                continue;
            };
            if let Some(id) = c.get("$id").and_then(Value::as_str) {
                by_id.insert(id.to_string(), (tname.clone(), cname.clone()));
            }
            let kind = c.get("ColumnType").and_then(Value::as_str).unwrap_or("Data");
            if kind.eq_ignore_ascii_case("RowNumber")
                || c.get("IsRowNumber").and_then(Value::as_bool) == Some(true)
            {
                continue;
            }
            let expression = text(c.get("ColumnExpression"));
            table.columns.push(Column {
                name: cname.clone(),
                data_type: DataType::from_tmsl(c.get("DataType").and_then(Value::as_str).unwrap_or("")),
                source_column: if expression.is_none() {
                    text(c.get("SourceColumn")).or(Some(cname.clone()))
                } else {
                    None
                },
                expression,
                is_hidden: c.get("IsHidden").and_then(Value::as_bool).unwrap_or(false),
                format_string: text(c.get("FormatString")),
                display_folder: text(c.get("DisplayFolder")),
                description: text(c.get("Description")),
                sort_by_column: text(c.get("SortByColumnName")),
                lineage_tag: Some(crate::edit::new_guid()),
                ..Default::default()
            });
        }
        for x in items(t.get("Measures")).iter().take(MAX_OBJECTS) {
            let Some(mname) = text(x.get("MeasureName")) else {
                continue;
            };
            table.measures.push(bicraft_model::Measure {
                name: mname,
                expression: text(x.get("MeasureExpression")).unwrap_or_default(),
                format_string: text(x.get("FormatString")),
                display_folder: text(x.get("DisplayFolder")),
                description: text(x.get("Description")),
                is_hidden: x.get("IsHidden").and_then(Value::as_bool).unwrap_or(false),
                lineage_tag: Some(crate::edit::new_guid()),
                ..Default::default()
            });
        }
        if let Some(e) = text(t.get("TableExpression")) {
            table.partitions.push(Partition {
                name: tname.clone(),
                mode: Some("import".into()),
                source_type: "calculated".into(),
                expression: Some(e),
                ..Default::default()
            });
        }
        m.tables.push(table);
    }
    let col = |v: Option<&Value>| -> Option<(String, String)> {
        let o = v?.as_object()?;
        let id = o.get("$ref").or_else(|| o.get("$id"))?.as_str()?;
        by_id.get(id).cloned()
    };
    for r in items(doc.get("Relationships")).iter().take(MAX_OBJECTS) {
        let (Some((ft, fc)), Some((tt, tc))) = (col(r.get("FromColumn")), col(r.get("ToColumn"))) else {
            notes.push("a relationship whose columns are not found was left out".into());
            continue;
        };
        let card = |k: &str| {
            r.get(k)
                .and_then(Value::as_str)
                .map(|s| s.to_ascii_lowercase())
                .filter(|s| s == "one" || s == "many")
        };
        m.relationships.push(Relationship {
            name: text(r.get("Name")).unwrap_or_else(crate::edit::new_guid),
            from_table: ft,
            from_column: fc,
            to_table: tt,
            to_column: tc,
            is_active: r.get("IsActive").and_then(Value::as_bool).unwrap_or(true),
            cross_filtering_behavior: r
                .get("CrossFilteringBehavior")
                .and_then(Value::as_str)
                .filter(|s| s.eq_ignore_ascii_case("BothDirections"))
                .map(|_| "bothDirections".to_string()),
            from_cardinality: card("FromCardinalityType"),
            to_cardinality: card("ToCardinalityType"),
            ..Default::default()
        });
    }
    notes.push("the VPAX has no Model.bim: tables have no partition (no query)".into());
    Ok(m)
}

/// The storage statistics of `DaxModel.json`: per table and column, and per relationship.
fn vpax_statistics(doc: &Value) -> (Vec<Value>, Vec<Value>) {
    let mut tables = Vec::new();
    for t in items(doc.get("Tables")).iter().take(MAX_OBJECTS) {
        let Some(name) = text(t.get("TableName")) else {
            continue;
        };
        let mut cols = Vec::new();
        let mut total = 0u64;
        for c in items(t.get("Columns")).iter().take(MAX_OBJECTS) {
            let size = num(c.get("TotalSize"));
            total = total.saturating_add(size);
            cols.push(json!({
                "name": text(c.get("ColumnName")),
                "cardinality": num(c.get("ColumnCardinality")),
                "data_size": num(c.get("DataSize")),
                "dictionary_size": num(c.get("DictionarySize")),
                "hierarchies_size": num(c.get("HierarchiesSize")),
                "total_size": size,
                "encoding": c.get("Encoding").and_then(Value::as_str),
                "row_number": c.get("IsRowNumber").and_then(Value::as_bool).unwrap_or(false),
            }));
        }
        let hier: u64 = items(t.get("UserHierarchies"))
            .iter()
            .map(|h| num(h.get("UsedSize")))
            .fold(0, u64::saturating_add);
        tables.push(json!({
            "name": name,
            "rows": num(t.get("RowsCount")),
            "columns_size": total,
            "user_hierarchies_size": hier,
            "total_size": total.saturating_add(hier),
            "referential_integrity_violations": num(t.get("ReferentialIntegrityViolationCount")),
            "columns": cols,
        }));
    }
    let rels = items(doc.get("Relationships"))
        .iter()
        .take(MAX_OBJECTS)
        .map(|r| {
            json!({
                "name": text(r.get("Name")),
                "used_size_from": num(r.get("UsedSizeFrom")),
                "used_size_to": num(r.get("UsedSizeTo")),
                "missing_keys": num(r.get("MissingKeys")),
                "invalid_rows": num(r.get("InvalidRows")),
            })
        })
        .collect();
    (tables, rels)
}

// ---------------------------------------------------------------------------------------------
// file.open
// ---------------------------------------------------------------------------------------------

/// `file.open` of a file of this module: the session gets a new, unsaved model.
pub(crate) fn open(
    s: &mut Session,
    path: &Path,
    data_root: Option<std::path::PathBuf>,
) -> Result<Value, String> {
    let imp = read_path(path)?;
    let tables = imp.model.tables.len();
    let expressions = imp.model.expressions.len();
    let name = imp.model.name.clone();
    s.source = None;
    s.cache_abf = None;
    s.model = Some(imp.model);
    s.model_error = None;
    s.dirty = true;
    s.report = None;
    s.report_error = None;
    s.report_dirty = false;
    s.diagram_layout = None;
    s.data_root = data_root;
    let mut out = json!({
        "path": path.display().to_string(),
        "format": imp.kind.name(),
        "details": {"type": "new", "note": "read into a new model, not saved yet: save it with file.save_as"},
        "model": {"name": name, "tables": tables, "expressions": expressions},
        "notes": imp.notes,
        "dirty": true,
    });
    if let (Some(st), Some(o)) = (imp.statistics, out.as_object_mut()) {
        o.insert("statistics".into(), st);
    }
    Ok(out)
}
