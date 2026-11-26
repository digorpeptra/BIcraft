//! VPAX export (M9.3): `model.export_vpax` / `bicraft vpax`. A VPAX file is the open
//! VertiPaq Analyzer format (sql-bi/VertiPaq-Analyzer, MIT license): an OPC package (ZIP with
//! `[Content_Types].xml`) holding `DaxModel.json`, the model and its storage statistics as the
//! `Dax.Metadata` classes serialize them, and optionally `Model.bim`, the model as TMSL.
//!
//! `DaxModel.json` follows that project's serializer settings: Json.NET with
//! `PreserveReferencesHandling.All` (every object and array carries `$id`, arrays wrap their
//! items in `$values`, later occurrences of an object are `{"$ref": id}`), names, notes and
//! expressions as plain strings, enums as numbers. Every figure is derived from bicraft's
//! own reading of the file (`storage_stats.rs`); see `docs/formats/vertipaq-stats.md`.

use std::collections::HashMap;
use std::path::PathBuf;

use bicraft_model::{Column, SemanticModel, Table};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::commands::model;
use crate::params::{allow, opt_bool, str_param};
use crate::storage_stats::{self, ColumnStats, Storage, TableStats};

/// `Dax.Metadata` model version written (the project's `CurrentDaxModelVersion`).
const DAX_MODEL_VERSION: &str = "1.9.0";

/// Json.NET `$id` numbering and the objects already written.
struct Refs {
    next: u64,
    seen: HashMap<String, u64>,
}

impl Refs {
    fn id(&mut self) -> Value {
        self.next += 1;
        Value::String(self.next.to_string())
    }

    /// A new object with its `$id`; `key` lets later occurrences refer to it.
    fn object(&mut self, key: Option<&str>) -> Map<String, Value> {
        let id = self.id();
        if let (Some(k), Some(n)) = (key, id.as_str().and_then(|s| s.parse().ok())) {
            self.seen.insert(k.to_string(), n);
        }
        let mut m = Map::new();
        m.insert("$id".into(), id);
        m
    }

    fn reference(&self, key: &str) -> Value {
        match self.seen.get(key) {
            Some(n) => json!({"$ref": n.to_string()}),
            None => Value::Null,
        }
    }

    fn array(&mut self, values: Vec<Value>) -> Value {
        json!({"$id": self.id(), "$values": values})
    }
}

/// Inserts `v` unless it is null (`DefaultValueHandling.Ignore` leaves defaults out).
fn put(m: &mut Map<String, Value>, k: &str, v: Value) {
    if !v.is_null() {
        m.insert(k.into(), v);
    }
}

fn opt(s: Option<&str>) -> Value {
    s.filter(|x| !x.is_empty()).map_or(Value::Null, |x| json!(x))
}

/// A `DaxNote` (description, display folder) or format string: always written, empty when
/// unset. DAX Studio's VPAX import dereferences these notes without a null check (an absent
/// one fails "Import Metrics" with a NullReferenceException), and VertiPaq Analyzer's own
/// extractor writes `""`.
fn note(s: Option<&str>) -> Value {
    json!(s.unwrap_or(""))
}

/// `DataType` of a measure with TOM's names: the stored one, else the result type the DAX type
/// checker infers (as `model.analyze` does), else `Variant`. DAX Studio parses it as a TOM
/// `DataType` and fails the import when it is missing.
fn measure_data_type(
    validator: &mut bicraft_dax::Validator,
    table: &str,
    m: &bicraft_model::Measure,
) -> String {
    if let Some(t) = extra_str(&m.extra, "dataType") {
        return pascal(t);
    }
    use bicraft_dax::types::{ScalarType, ValueType};
    let only_blank = matches!(
        bicraft_dax::parse_expression(&m.expression).0,
        Some(bicraft_dax::ast::Expr::Call { ref name, ref args, .. })
            if args.is_empty() && name.eq_ignore_ascii_case("BLANK")
    );
    let result = if only_blank {
        Some(ValueType::Scalar(ScalarType::Integer))
    } else if m.expression.trim().is_empty() {
        None
    } else {
        validator
            .validate_as(&m.expression, Some(table), bicraft_dax::ExprKind::Measure)
            .result_type
    };
    match result {
        Some(ValueType::Scalar(s)) => match s {
            ScalarType::Integer => "Int64",
            ScalarType::Decimal => "Double",
            ScalarType::Currency => "Decimal",
            ScalarType::Boolean => "Boolean",
            ScalarType::String => "String",
            ScalarType::DateTime => "DateTime",
            ScalarType::Variant => "Variant",
        },
        _ => "Variant",
    }
    .to_string()
}

/// TOM enum spelling (`oneDirection` → `OneDirection`).
fn pascal(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn tom_data_type(c: &Column) -> Option<&'static str> {
    Some(match c.data_type {
        bicraft_model::DataType::String => "String",
        bicraft_model::DataType::Int64 => "Int64",
        bicraft_model::DataType::Double => "Double",
        bicraft_model::DataType::Decimal => "Decimal",
        bicraft_model::DataType::DateTime => "DateTime",
        bicraft_model::DataType::Boolean => "Boolean",
        bicraft_model::DataType::Binary => "Binary",
        bicraft_model::DataType::Variant => "Variant",
        bicraft_model::DataType::Unknown => return None,
    })
}

fn extra_str<'a>(c: &'a bicraft_model::Properties, k: &str) -> Option<&'a str> {
    c.get(k).and_then(Value::as_str)
}

fn extra_bool(c: &bicraft_model::Properties, k: &str) -> Option<bool> {
    c.get(k).and_then(Value::as_bool)
}

/// `Partition.PartitionMode` number.
fn partition_mode(mode: Option<&str>) -> Option<i64> {
    Some(match mode?.to_ascii_lowercase().as_str() {
        "import" => 0,
        "directquery" => 1,
        "default" => 2,
        "push" => 3,
        "dual" => 4,
        "directlake" => 5,
        _ => return None,
    })
}

/// `Partition.PartitionType` number.
fn partition_type(source_type: &str) -> Option<i64> {
    Some(match source_type.to_ascii_lowercase().as_str() {
        "query" => 1,
        "calculated" => 2,
        "none" => 3,
        "m" => 4,
        "entity" => 5,
        "policyrange" => 6,
        "calculationgroup" => 7,
        "inferred" => 8,
        _ => return None,
    })
}

fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    iso_utc(secs)
}

/// `YYYY-MM-DDThh:mm:ssZ` of a Unix time (civil-from-days, proleptic Gregorian).
pub(crate) fn iso_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = bicraft_locale::calendar::civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn column_key(table: &str, column: &str) -> String {
    format!("c|{}|{}", table.to_lowercase(), column.to_lowercase())
}

fn partition_key(table: &str, partition: &str) -> String {
    format!("p|{}|{}", table.to_lowercase(), partition.to_lowercase())
}

fn segment_json(refs: &mut Refs, table: &str, s: &storage_stats::Segment) -> Value {
    let mut o = refs.object(None);
    let key = partition_key(table, &s.partition);
    let partition = if refs.seen.contains_key(&key) {
        refs.reference(&key)
    } else {
        let mut p = refs.object(Some(&key));
        p.insert("PartitionName".into(), json!(s.partition));
        p.insert("PartitionNumber".into(), json!(s.partition_number));
        Value::Object(p)
    };
    o.insert("Partition".into(), partition);
    o.insert("SegmentNumber".into(), json!(s.number));
    o.insert("SegmentRows".into(), json!(s.records));
    o.insert("UsedSize".into(), json!(s.used_size));
    o.insert("CompressionType".into(), json!(s.compression));
    o.insert("BitsCount".into(), json!(s.bits));
    o.insert("BookmarkBitsCount".into(), json!(s.bookmark_bits));
    o.insert(
        "VertipaqState".into(),
        json!(if s.skipped { "SKIPPED" } else { "COMPLETED" }),
    );
    Value::Object(o)
}

fn column_json(
    refs: &mut Refs,
    table: &Table,
    rows: u64,
    c: Option<&Column>,
    st: Option<&ColumnStats>,
) -> Value {
    let name = c
        .map(|c| c.name.as_str())
        .or(st.map(|s| s.name.as_str()))
        .unwrap_or("");
    let mut o = refs.object(Some(&column_key(&table.name, name)));
    o.insert("ColumnName".into(), json!(name));
    let row_number = st.is_some_and(|s| s.is_row_number);
    // VertiPaq Analyzer's extractor leaves the row-number column's cardinality at 0 (it has no
    // attribute hierarchy to count).
    let cardinality = if row_number {
        0
    } else {
        st.map_or(0, |s| s.cardinality)
    };
    if cardinality > 0 {
        o.insert("ColumnCardinality".into(), json!(cardinality));
    }
    put(
        &mut o,
        "DataType",
        opt(c.and_then(tom_data_type).or(st.and_then(|s| s.data_type))),
    );
    let column_type = match c.and_then(|c| extra_str(&c.extra, "type")) {
        _ if row_number => "RowNumber",
        Some("calculatedTableColumn") => "CalculatedTableColumn",
        Some("calculated") => "Calculated",
        _ if c.is_some_and(|c| c.expression.is_some()) => "Calculated",
        _ => "Data",
    };
    o.insert("ColumnType".into(), json!(column_type));
    o.insert(
        "IsHidden".into(),
        json!(row_number || c.is_some_and(|c| c.is_hidden)),
    );
    o.insert("Encoding".into(), json!(st.map_or("UNKNOWN", |s| s.encoding)));
    o.insert(
        "DisplayFolder".into(),
        note(c.and_then(|c| c.display_folder.as_deref())),
    );
    o.insert(
        "FormatString".into(),
        note(c.and_then(|c| c.format_string.as_deref())),
    );
    o.insert(
        "Description".into(),
        note(c.and_then(|c| c.description.as_deref())),
    );
    o.insert(
        "EncodingHint".into(),
        json!(
            c.and_then(|c| extra_str(&c.extra, "encodingHint"))
                .map_or_else(|| "Default".to_string(), pascal)
        ),
    );
    if row_number {
        // The row-number column is the table's key unless a column of the model is.
        let user_key = table
            .columns
            .iter()
            .any(|c| !c.is_row_number() && extra_bool(&c.extra, "isKey") == Some(true));
        o.insert("IsAvailableInMDX".into(), json!(true));
        o.insert("IsKey".into(), json!(!user_key));
        o.insert("IsUnique".into(), json!(true));
    }
    if let Some(c) = c {
        put(&mut o, "ColumnExpression", opt(c.expression.as_deref()));
        let mdx = extra_bool(&c.extra, "isAvailableInMdx")
            .or(extra_bool(&c.extra, "isAvailableInMDX"))
            .unwrap_or(true);
        o.insert("IsAvailableInMDX".into(), json!(mdx));
        o.insert(
            "IsKey".into(),
            json!(extra_bool(&c.extra, "isKey").unwrap_or(false)),
        );
        o.insert(
            "IsNullable".into(),
            json!(extra_bool(&c.extra, "isNullable").unwrap_or(true)),
        );
        o.insert(
            "IsUnique".into(),
            json!(extra_bool(&c.extra, "isUnique").unwrap_or(false)),
        );
        put(&mut o, "SourceColumn", opt(c.source_column.as_deref()));
        put(&mut o, "SortByColumnName", opt(c.sort_by_column.as_deref()));
    }
    o.insert("State".into(), json!("Ready"));
    o.insert("IsRowNumber".into(), json!(row_number));
    let dictionary = st.map_or(0, |s| s.dictionary_size);
    o.insert("DictionarySize".into(), json!(dictionary));
    // VertiPaq Analyzer reads segment 0 of each H$ structure as the column's hierarchy.
    let hierarchies: Vec<Value> = st
        .map(|s| {
            s.hierarchy
                .iter()
                .filter(|(_, seg)| seg.number == 0)
                .map(|(structure, seg)| {
                    let mut h = refs.object(None);
                    h.insert("StructureName".into(), json!(structure));
                    h.insert("SegmentNumber".into(), json!(seg.number));
                    h.insert("TablePartitionNumber".into(), json!(seg.partition_number));
                    h.insert("UsedSize".into(), json!(seg.used_size));
                    Value::Object(h)
                })
                .collect()
        })
        .unwrap_or_default();
    let hierarchy_size: u64 = hierarchies
        .iter()
        .filter_map(|h| h.get("UsedSize").and_then(Value::as_u64))
        .fold(0, u64::saturating_add);
    let segments: Vec<Value> = st
        .map(|s| {
            s.segments
                .iter()
                .map(|seg| segment_json(refs, &table.name, seg))
                .collect()
        })
        .unwrap_or_default();
    let data_size = st.map_or(0, ColumnStats::data_size);
    o.insert("ColumnHierarchies".into(), refs.array(hierarchies));
    o.insert("ColumnSegments".into(), refs.array(segments));
    o.insert("DataSize".into(), json!(data_size));
    o.insert("HierarchiesSize".into(), json!(hierarchy_size));
    o.insert(
        "TotalSize".into(),
        json!(
            data_size
                .saturating_add(dictionary)
                .saturating_add(hierarchy_size)
        ),
    );
    if rows > 0 && cardinality > 0 {
        o.insert("Selectivity".into(), json!(cardinality as f64 / rows as f64));
    }
    Value::Object(o)
}

fn table_json(
    refs: &mut Refs,
    validator: &mut bicraft_dax::Validator,
    t: &Table,
    st: Option<&TableStats>,
) -> Value {
    let mut o = refs.object(Some(&format!("t|{}", t.name.to_lowercase())));
    o.insert("Model".into(), json!({"$ref": "1"}));
    o.insert("TableName".into(), json!(t.name));
    let calculated = t
        .partitions
        .iter()
        .find(|p| p.source_type.eq_ignore_ascii_case("calculated"));
    let table_type = if t.extra.contains_key("calculationGroup") {
        Some("CalculationGroup")
    } else if calculated.is_some() {
        Some("Calculated")
    } else {
        None
    };
    put(&mut o, "TableType", opt(table_type));
    o.insert("IsHidden".into(), json!(t.is_hidden));
    o.insert(
        "IsPrivate".into(),
        json!(extra_bool(&t.extra, "isPrivate").unwrap_or(false)),
    );
    o.insert(
        "IsLocalDateTable".into(),
        json!(t.name.starts_with("LocalDateTable_")),
    );
    o.insert(
        "IsTemplateDateTable".into(),
        json!(t.name.starts_with("DateTableTemplate_")),
    );
    put(
        &mut o,
        "TableExpression",
        opt(calculated.and_then(|p| p.expression.as_deref())),
    );
    let rows = st.map_or(0, |s| s.rows);
    o.insert("RowsCount".into(), json!(rows));
    o.insert(
        "ReferentialIntegrityViolationCount".into(),
        json!(st.map_or(0, |s| s.ri_violations)),
    );
    o.insert("Description".into(), note(t.description.as_deref()));
    o.insert(
        "IsDateTable".into(),
        json!(extra_str(&t.extra, "dataCategory") == Some("Time")),
    );

    // Columns: the row-number column first (as the engine stores it), then the model's.
    let mut columns = Vec::new();
    if let Some(row_number) = st.and_then(|s| s.columns.iter().find(|c| c.is_row_number)) {
        columns.push(column_json(refs, t, rows, None, Some(row_number)));
    }
    for c in t.columns.iter().filter(|c| !c.is_row_number()) {
        let cs = st.and_then(|s| {
            s.columns
                .iter()
                .find(|x| !x.is_row_number && x.name.eq_ignore_ascii_case(&c.name))
        });
        columns.push(column_json(refs, t, rows, Some(c), cs));
    }
    o.insert("Columns".into(), refs.array(columns));

    let measures: Vec<Value> = t
        .measures
        .iter()
        .map(|m| {
            let mut x = refs.object(None);
            x.insert("MeasureName".into(), json!(m.name));
            x.insert("MeasureExpression".into(), json!(m.expression));
            let fsd = m
                .extra
                .get("formatStringDefinition")
                .and_then(|v| v.get("expression"))
                .and_then(Value::as_str);
            put(&mut x, "FormatStringExpression", opt(fsd));
            x.insert("DisplayFolder".into(), note(m.display_folder.as_deref()));
            x.insert("Description".into(), note(m.description.as_deref()));
            x.insert("IsHidden".into(), json!(m.is_hidden));
            x.insert("DataType".into(), json!(measure_data_type(validator, &t.name, m)));
            x.insert("FormatString".into(), note(m.format_string.as_deref()));
            Value::Object(x)
        })
        .collect();
    o.insert("Measures".into(), refs.array(measures));

    let hierarchies: Vec<Value> = t
        .extra
        .get("hierarchies")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|h| {
                    let name = h.get("name")?.as_str()?;
                    let mut x = refs.object(None);
                    x.insert("HierarchyName".into(), json!(name));
                    x.insert(
                        "IsHidden".into(),
                        json!(h.get("isHidden").and_then(Value::as_bool).unwrap_or(false)),
                    );
                    let size = st
                        .and_then(|s| {
                            s.user_hierarchies
                                .iter()
                                .find(|u| u.name.eq_ignore_ascii_case(name))
                        })
                        .map_or(0, |u| u.size);
                    x.insert("UsedSize".into(), json!(size));
                    let levels: Vec<Value> = h
                        .get("levels")
                        .and_then(Value::as_array)
                        .map(|ls| {
                            ls.iter()
                                .filter_map(|l| l.get("column").and_then(Value::as_str))
                                .map(|c| refs.reference(&column_key(&t.name, c)))
                                .filter(|v| !v.is_null())
                                .collect()
                        })
                        .unwrap_or_default();
                    x.insert("Levels".into(), refs.array(levels));
                    Some(Value::Object(x))
                })
                .collect()
        })
        .unwrap_or_default();
    o.insert("UserHierarchies".into(), refs.array(hierarchies));

    let partitions: Vec<Value> = t
        .partitions
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let key = partition_key(&t.name, &p.name);
            let number = st
                .and_then(|s| s.partitions.iter().find(|x| x.name.eq_ignore_ascii_case(&p.name)))
                .map_or(i as i64, |x| x.number);
            if refs.seen.contains_key(&key) {
                return refs.reference(&key);
            }
            let mut x = refs.object(Some(&key));
            x.insert("PartitionName".into(), json!(p.name));
            x.insert("PartitionNumber".into(), json!(number));
            x.insert("State".into(), json!(1));
            put(
                &mut x,
                "Type",
                partition_type(&p.source_type).map_or(Value::Null, |n| json!(n)),
            );
            put(
                &mut x,
                "Mode",
                partition_mode(p.mode.as_deref()).map_or(Value::Null, |n| json!(n)),
            );
            Value::Object(x)
        })
        .collect();
    o.insert("Partitions".into(), refs.array(partitions));
    Value::Object(o)
}

/// `DaxModel.json` of a model and its storage statistics.
pub(crate) fn dax_model(m: &SemanticModel, st: &Storage, name: &str) -> Value {
    let mut refs = Refs {
        next: 0,
        seen: HashMap::new(),
    };
    let mut root = refs.object(None);
    root.insert("DaxModelVersion".into(), json!(DAX_MODEL_VERSION));
    root.insert("ExtractorApp".into(), json!("bicraft"));
    root.insert("ExtractorAppVersion".into(), json!(env!("CARGO_PKG_VERSION")));
    root.insert("ExtractorLib".into(), json!("bicraft-engine"));
    root.insert("ExtractorLibVersion".into(), json!(env!("CARGO_PKG_VERSION")));
    root.insert("DaxModelLib".into(), json!("bicraft-engine"));
    root.insert("DaxModelLibVersion".into(), json!(env!("CARGO_PKG_VERSION")));
    root.insert("ModelName".into(), json!(name));
    if let Some(level) = m.compatibility_level {
        root.insert("CompatibilityLevel".into(), json!(level));
    }
    root.insert("CompatibilityMode".into(), json!("PowerBI"));
    root.insert("ExtractionDate".into(), json!(now_iso()));
    root.insert("ServerMode".into(), json!("SharePoint"));
    let mut validator = bicraft_dax::Validator::new(m);
    let tables: Vec<Value> = m
        .tables
        .iter()
        .map(|t| {
            let s = st.tables.iter().find(|x| x.name.eq_ignore_ascii_case(&t.name));
            table_json(&mut refs, &mut validator, t, s)
        })
        .collect();
    root.insert("Tables".into(), refs.array(tables));
    let relationships: Vec<Value> = m
        .relationships
        .iter()
        .map(|r| {
            let mut x = refs.object(None);
            x.insert(
                "FromColumn".into(),
                refs.reference(&column_key(&r.from_table, &r.from_column)),
            );
            x.insert(
                "FromCardinalityType".into(),
                json!(pascal(r.from_cardinality.as_deref().unwrap_or("many"))),
            );
            x.insert(
                "ToColumn".into(),
                refs.reference(&column_key(&r.to_table, &r.to_column)),
            );
            x.insert(
                "ToCardinalityType".into(),
                json!(pascal(r.to_cardinality.as_deref().unwrap_or("one"))),
            );
            x.insert(
                "RelyOnReferentialIntegrity".into(),
                json!(extra_bool(&r.extra, "relyOnReferentialIntegrity").unwrap_or(false)),
            );
            x.insert(
                "JoinOnDateBehavior".into(),
                json!(pascal(
                    extra_str(&r.extra, "joinOnDateBehavior").unwrap_or("dateAndTime")
                )),
            );
            x.insert(
                "CrossFilteringBehavior".into(),
                json!(pascal(
                    r.cross_filtering_behavior.as_deref().unwrap_or("oneDirection")
                )),
            );
            x.insert("Type".into(), json!("SingleColumn"));
            x.insert("IsActive".into(), json!(r.is_active));
            x.insert("Name".into(), json!(r.name));
            x.insert(
                "SecurityFilteringBehavior".into(),
                json!(pascal(
                    extra_str(&r.extra, "securityFilteringBehavior").unwrap_or("oneDirection")
                )),
            );
            let s = st
                .relationships
                .iter()
                .find(|x| x.name.eq_ignore_ascii_case(&r.name));
            x.insert("UsedSizeFrom".into(), json!(s.map_or(0, |s| s.size_from)));
            x.insert("UsedSizeTo".into(), json!(s.map_or(0, |s| s.size_to)));
            x.insert("MissingKeys".into(), json!(0));
            x.insert("InvalidRows".into(), json!(0));
            Value::Object(x)
        })
        .collect();
    root.insert("Relationships".into(), refs.array(relationships));
    root.insert("Roles".into(), refs.array(Vec::new()));
    if let Some(c) = &m.culture {
        root.insert("Culture".into(), json!(c));
    }
    Value::Object(root)
}

const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"json\" ContentType=\"application/json\" /><Default Extension=\"bim\" ContentType=\"application/json\" /></Types>";

/// The VPAX package bytes.
pub(crate) fn vpax_bytes(
    m: &SemanticModel,
    st: &Storage,
    name: &str,
    with_bim: bool,
) -> Result<Vec<u8>, String> {
    let mut pkg = bicraft_opc::Package::default();
    pkg.put("[Content_Types].xml", CONTENT_TYPES.as_bytes().to_vec(), false);
    let dax = serde_json::to_vec(&dax_model(m, st, name)).map_err(|e| e.to_string())?;
    pkg.put("DaxModel.json", dax, false);
    if with_bim {
        pkg.put("Model.bim", bicraft_tmsl::write(m).into_bytes(), false);
    }
    pkg.to_bytes().map_err(|e| e.to_string())
}

pub(crate) fn export_vpax(s: &mut Session, params: &Value) -> Result<Value, String> {
    allow(params, &["path", "overwrite", "model_bim"])?;
    let path = PathBuf::from(str_param(params, "path")?);
    let overwrite = opt_bool(params, "overwrite")?.unwrap_or(false);
    let with_bim = opt_bool(params, "model_bim")?.unwrap_or(true);
    if path.exists() && !overwrite {
        return Err(format!(
            "{} exists (pass overwrite: true to replace it)",
            path.display()
        ));
    }
    let m = model(s)?;
    let st = storage_stats::load(s)?;
    // A PBIX keeps no database name: the file name stands in for it, as in Desktop.
    let name = if m.name.is_empty() {
        s.source
            .as_ref()
            .and_then(|src| crate::commands::source_path(src).file_stem())
            .and_then(|n| n.to_str())
            .unwrap_or("Model")
            .to_string()
    } else {
        m.name.clone()
    };
    let bytes = vpax_bytes(m, &st, &name, with_bim)?;
    let tmp = path.with_extension("vpax.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })?;
    let total: u64 = st
        .tables
        .iter()
        .map(|t| st.table_size(t))
        .fold(0, u64::saturating_add);
    Ok(json!({
        "path": path.display().to_string(),
        "bytes": bytes.len(),
        "tables": st.tables.len(),
        "total_size": total,
        "model_bim": with_bim,
        "parts": if with_bim { json!(["DaxModel.json", "Model.bim"]) } else { json!(["DaxModel.json"]) },
    }))
}
