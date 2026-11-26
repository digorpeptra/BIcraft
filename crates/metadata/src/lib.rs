//! L3: the Analysis Services `metadata.sqlitedb` (found inside a PBIX `DataModel` ABF backup) →
//! the format-neutral [`SemanticModel`].
//!
//! The database holds one table per tabular object type (`Model`, `Table`, `Column`, `Measure`,
//! `Partition`, `Relationship`, `Expression`, `Role`, `Annotation`, …), the TMSCHEMA rowsets of
//! [MS-SSAS-T] §2.2.5. Objects point at each other by numeric ID. The mapping is documented in
//! `docs/formats/metadata-sqlitedb.md`; in short:
//!
//! - Names come from `Name`, or for columns from `ExplicitName` falling back to `InferredName`.
//! - Every collection is ordered by object `ID`, which is creation order.
//! - Enumeration codes become their TMSL spellings. Properties at their TOM default are left
//!   out, the way TOM serializes TMSL, so the result looks like what `bicraft-tmsl` reads from
//!   the same model's `model.bim`. Properties without a typed field go into `extra` under their
//!   TMSL name; `rowNumber` and `calculatedTableColumn` columns carry `extra["type"]`.
//! - Only user tables of the model are returned: system tables (`SystemFlags` bit 0, the
//!   `H$`/`R$`/`U$` storage tables) and storage-only tables (`*Storage`, `StorageFile`, …) are
//!   ignored.
//! - The database name and compatibility level are not stored in `metadata.sqlitedb`; the
//!   returned model leaves them empty for the container reader to fill in.
//!
//! Input is hostile: missing tables read as empty, values of an unexpected type read as absent,
//! dangling references are dropped (a relationship whose ends do not resolve is skipped). Only a
//! missing `Model` row is an error.
//!
//! - Objects the model keeps as JSON in `extra` (hierarchies, perspectives, cultures and
//!   translations, calculation groups, KPIs, refresh policies, calendars, data sources, …) are
//!   described once in `spec.rs`; `nested.rs` reads and writes them from that description.
//!
//! [`build_model`] works on [`MetadataTables`], a reader-independent row abstraction;
//! [`read_sqlitedb`] fills it from the raw database bytes with `bicraft-sqlite`.
//!
//! The inverse direction, [`apply_model`], diffs an edited model against
//! `build_model(base)` and applies the metadata-only part of the difference (measures,
//! descriptions, display properties, annotations, roles, shared expression text) to a copy of the
//! base rows, keeping every untouched row unchanged. Edits that would need new VertiPaq data
//! (tables, columns, relationships, partitions, …) are refused with
//! [`Error::UnsupportedChange`]. [`tables_to_rows`] shapes the result for an SQLite writer, and
//! [`write_sqlitedb`] does the whole round trip on the database bytes: read, apply, rewrite only
//! the tables that changed.
//!
//! Structural edits go through [`apply_structural`] / [`write_structural_sqlitedb`]: added and
//! removed tables, columns and relationships, with their storage rows and the ABF files to add
//! and remove ([`StoragePlan`]), given the encoded VertiPaq data of new objects
//! ([`EncodedData`]).
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable,
    clippy::indexing_slicing
)]

mod apply;
mod build;
mod codes;
mod fresh;
mod input;
mod nested;
mod spec;
mod structural;
mod write;

use bicraft_model::SemanticModel;
pub use fresh::fresh_sqlitedb_at;
pub use input::{MetaRow, MetaValue, MetadataTables};
pub use structural::check::{dangling_references, storage_file_paths};
pub use structural::data::{
    AttributeHierarchyData, ColumnStatistics, ColumnStorageData, DictionaryData, EncodedData,
    EncodedHierarchy, EncodedNewColumn, EncodedPartition, EncodedRelationship, EncodedTable, FileToWrite,
    PartitionSegments, SegmentMap, StoragePlan, SystemColumnData, SystemTableData,
};
pub use structural::rename::Renames;
pub use structural::{ROW_NUMBER_NAME, StructuralEdit, UserHierarchyDef, user_hierarchies};
pub use write::{
    COMMIT_MARKERS, DATA_CHANGE_MARKER, SqliteRows, StructuralWrite, tables_to_rows, write_sqlitedb,
    write_sqlitedb_at, write_structural_sqlitedb, write_structural_sqlitedb_at,
    write_structural_sqlitedb_renamed_at,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("metadata has no `Model` row")]
    NoModel,
    #[error("metadata.sqlitedb: {0}")]
    Sqlite(#[from] bicraft_sqlite::Error),
    /// The edited model differs from the base in ways that cannot be written without
    /// regenerating VertiPaq data (or that the writer does not support yet). Lists every such
    /// difference.
    #[error("unsupported model change: {}", list(.0))]
    UnsupportedChange(Vec<UnsupportedChange>),
    /// The base metadata is inconsistent (duplicate IDs, ID overflow, a row the edit needs is
    /// missing, a value that does not fit the table schema, …).
    #[error("cannot write metadata: {0}")]
    Invalid(String),
}

/// One difference between the base and the edited model that [`apply_model`] cannot write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedChange {
    /// What changed, e.g. "column added" or "dataType changed".
    pub what: String,
    /// The object, e.g. `column 'Sales'[Amount]`.
    pub object: String,
}

impl std::fmt::Display for UnsupportedChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.object, self.what)
    }
}

fn list(changes: &[UnsupportedChange]) -> String {
    changes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// The metadata tables the mapping reads, plus the few [`apply_model`] needs to keep references
/// and the ID counter consistent (`ObjectTranslation`, `ExcludedArtifact`, `DBPROPERTIES`).
/// Everything else (storage tables) is ignored.
pub const TABLES: &[&str] = &[
    "Model",
    "Table",
    "Column",
    "Measure",
    "Partition",
    "Relationship",
    "Expression",
    "Annotation",
    "ExtendedProperty",
    "ChangedProperty",
    "Hierarchy",
    "Level",
    "Variation",
    "KPI",
    "DetailRowsDefinition",
    "FormatStringDefinition",
    "CalculationGroup",
    "CalculationItem",
    "DataSource",
    "QueryGroup",
    "Culture",
    "LinguisticMetadata",
    "Perspective",
    "PerspectiveTable",
    "PerspectiveColumn",
    "PerspectiveHierarchy",
    "PerspectiveMeasure",
    "Role",
    "RoleMembership",
    "TablePermission",
    "ColumnPermission",
    "Function",
    "CalculationExpression",
    "RefreshPolicy",
    "RelatedColumnDetails",
    "GroupByColumn",
    "AlternateOf",
    "Calendar",
    "CalendarColumnGroup",
    "CalendarColumnReference",
    "DataCoverageDefinition",
    "ObjectTranslation",
    "ExcludedArtifact",
    "Set",
    "PerspectiveSet",
    "AnalyticsAIMetadata",
    "BindingInfo",
    "DBPROPERTIES",
];

/// TOM's JSON name of a column's `IsAvailableInMDX` (TMSL and TMDL): the spelling the reader
/// produces. Desktop refuses the engine's spelling in TMSL/TMDL.
pub const AVAILABLE_IN_MDX: &str = "isAvailableInMdx";

/// The engine's spelling, written by bicraft before 2026-10-14: still accepted on input.
const AVAILABLE_IN_MDX_LEGACY: &str = "isAvailableInMDX";

/// A column's `isAvailableInMdx` (default true), under either spelling.
pub fn column_available_in_mdx(extra: &bicraft_model::Properties) -> bool {
    extra
        .get(AVAILABLE_IN_MDX)
        .or_else(|| extra.get(AVAILABLE_IN_MDX_LEGACY))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true)
}

/// `model` with legacy property spellings (`isAvailableInMDX`) renamed to TOM's and
/// `isNameInferred: false` (the default, which TMSL files may spell out; the metadata reads it
/// back as absent) dropped, and relationship properties spelled with their default value
/// (`fromCardinality: many`, `toCardinality: one`, `crossFilteringBehavior: oneDirection`,
/// which the metadata reads back as absent) cleared; borrowed when there is none.
pub fn normalize_spellings(model: &SemanticModel) -> std::borrow::Cow<'_, SemanticModel> {
    const NAME_INFERRED: &str = "isNameInferred";
    let legacy = |c: &bicraft_model::Column| {
        c.extra.contains_key(AVAILABLE_IN_MDX_LEGACY)
            || c.extra.get(NAME_INFERRED) == Some(&serde_json::Value::Bool(false))
    };
    let is =
        |v: &Option<String>, default: &str| v.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(default));
    let default_spelled = |r: &bicraft_model::Relationship| {
        is(&r.from_cardinality, "many")
            || is(&r.to_cardinality, "one")
            || is(&r.cross_filtering_behavior, "oneDirection")
    };
    if !model.tables.iter().any(|t| t.columns.iter().any(legacy))
        && !model.relationships.iter().any(default_spelled)
    {
        return std::borrow::Cow::Borrowed(model);
    }
    let mut out = model.clone();
    for r in &mut out.relationships {
        if is(&r.from_cardinality, "many") {
            r.from_cardinality = None;
        }
        if is(&r.to_cardinality, "one") {
            r.to_cardinality = None;
        }
        if is(&r.cross_filtering_behavior, "oneDirection") {
            r.cross_filtering_behavior = None;
        }
    }
    for column in out.tables.iter_mut().flat_map(|t| t.columns.iter_mut()) {
        if let Some(v) = column.extra.shift_remove(AVAILABLE_IN_MDX_LEGACY) {
            column.extra.entry(AVAILABLE_IN_MDX.to_string()).or_insert(v);
        }
        if column.extra.get(NAME_INFERRED) == Some(&serde_json::Value::Bool(false)) {
            column.extra.shift_remove(NAME_INFERRED);
        }
    }
    std::borrow::Cow::Owned(out)
}

/// Maps metadata rows to a [`SemanticModel`].
pub fn build_model(tables: &MetadataTables) -> Result<SemanticModel, Error> {
    build::build(tables)
}

/// Loads the [`TABLES`] of a `metadata.sqlitedb` image. Tables absent from the file are skipped;
/// a table that exists but cannot be read is an error.
pub fn tables_from_sqlitedb(bytes: &[u8]) -> Result<MetadataTables, Error> {
    tables_from_database(&bicraft_sqlite::Database::from_bytes(bytes)?)
}

/// Loads every table of a `metadata.sqlitedb` image, storage tables included: the base a
/// structural edit ([`apply_structural`]) needs.
pub fn tables_from_sqlitedb_all(bytes: &[u8]) -> Result<MetadataTables, Error> {
    tables_from_database_all(&bicraft_sqlite::Database::from_bytes(bytes)?)
}

fn tables_from_database_all(db: &bicraft_sqlite::Database) -> Result<MetadataTables, Error> {
    let names: Vec<String> = db.tables().iter().map(|t| t.name.clone()).collect();
    load_tables(db, names.iter().map(String::as_str))
}

fn tables_from_database(db: &bicraft_sqlite::Database) -> Result<MetadataTables, Error> {
    load_tables(db, TABLES.iter().copied())
}

fn load_tables<'n>(
    db: &bicraft_sqlite::Database,
    names: impl Iterator<Item = &'n str>,
) -> Result<MetadataTables, Error> {
    let mut out = MetadataTables::new();
    for name in names {
        if db.table(name).is_none() {
            continue;
        }
        let rows = db
            .query_all(name)?
            .into_iter()
            .map(|row| row.into_iter().map(|(k, v)| (k, convert(v))).collect())
            .collect();
        out.insert_table(name, rows);
    }
    Ok(out)
}

/// One encrypted secret of a data source, kept opaque.
///
/// The engine stores `ConnectionString`, `Password` and `Credential` encrypted with its instance
/// key (base64 text of an envelope bicraft does not open; `metadata-sqlitedb.md`, "Encrypted
/// data-source secrets"). The model leaves them out; every write keeps the stored text of a
/// kept data source byte for byte, and this lists them for inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedSecret {
    /// The data source's `Name`.
    pub data_source: String,
    /// The TMSL property: `connectionString`, `password` or `credential`.
    pub property: &'static str,
    /// The stored value, exactly as in the file.
    pub stored: String,
}

/// The encrypted data-source secrets of `tables`, in data source `ID` order, then
/// `connectionString`, `password`, `credential`. Empty values are skipped.
pub fn encrypted_secrets(tables: &MetadataTables) -> Vec<EncryptedSecret> {
    const COLUMNS: [(&str, &str); 3] = [
        ("ConnectionString", "connectionString"),
        ("Password", "password"),
        ("Credential", "credential"),
    ];
    let mut rows: Vec<&MetaRow> = tables.rows("DataSource").iter().collect();
    rows.sort_by_key(|r| match r.get("ID") {
        Some(MetaValue::Integer(i)) => *i,
        _ => i64::MAX,
    });
    let mut out = Vec::new();
    for r in rows {
        let name = match r.get("Name") {
            Some(MetaValue::Text(s)) => s.clone(),
            _ => String::new(),
        };
        for (col, property) in COLUMNS {
            if let Some(MetaValue::Text(v)) = r.get(col)
                && !v.is_empty()
            {
                out.push(EncryptedSecret {
                    data_source: name.clone(),
                    property,
                    stored: v.clone(),
                });
            }
        }
    }
    out
}

/// Reads a `metadata.sqlitedb` image into a [`SemanticModel`].
pub fn read_sqlitedb(bytes: &[u8]) -> Result<SemanticModel, Error> {
    build_model(&tables_from_sqlitedb(bytes)?)
}

/// Applies an edited `model` to the `base` metadata rows: the inverse of [`build_model`] for
/// metadata-only edits. Objects are matched by name (case-insensitively; measures model-wide,
/// columns within their table). Untouched rows are returned unchanged; changed rows get a new
/// `ModifiedTime` (now, as Windows FILETIME ticks); new rows get IDs above every ID in use. See
/// `docs/formats/metadata-sqlitedb.md` ("Writing").
///
/// `apply_model(base, &build_model(base)?)` returns `base` exactly.
pub fn apply_model(base: &MetadataTables, model: &SemanticModel) -> Result<MetadataTables, Error> {
    apply_model_at(base, model, apply::filetime_now()?)
}

/// [`apply_model`] with an explicit clock: `modified_time` is written to `ModifiedTime` /
/// `StructureModifiedTime` of every changed or new row (Windows FILETIME ticks: 100 ns since
/// 1601-01-01 UTC). Also seeds the lineage tags generated for new measures, so the output is
/// deterministic.
pub fn apply_model_at(
    base: &MetadataTables,
    model: &SemanticModel,
    modified_time: i64,
) -> Result<MetadataTables, Error> {
    apply::apply(base, model, modified_time)
}

/// Applies an edited `model` to the `base` metadata rows, structural changes included: tables,
/// columns (of kept tables) and relationships added or removed, with every storage row, the
/// `H$`/`R$` system tables and the list of ABF files to add and remove ([`StoragePlan`]). Every
/// other difference is applied as by [`apply_model`]. `base` must hold every table of the
/// database ([`tables_from_sqlitedb_all`]); `data` carries the encoded VertiPaq content of the
/// added objects. See `docs/formats/metadata-sqlitedb.md` ("Structural writing").
///
/// Changes that cannot be written are refused together with [`Error::UnsupportedChange`].
pub fn apply_structural(
    base: &MetadataTables,
    model: &SemanticModel,
    data: &EncodedData,
) -> Result<StructuralEdit, Error> {
    apply_structural_at(base, model, data, apply::filetime_now()?)
}

/// [`apply_structural`] with an explicit clock (Windows FILETIME ticks), for reproducible
/// output: it also seeds generated lineage tags.
pub fn apply_structural_at(
    base: &MetadataTables,
    model: &SemanticModel,
    data: &EncodedData,
    modified_time: i64,
) -> Result<StructuralEdit, Error> {
    structural::apply(base, model, data, modified_time)
}

fn convert(v: bicraft_sqlite::Value) -> MetaValue {
    use bicraft_sqlite::Value as V;
    match v {
        V::Null => MetaValue::Null,
        V::Integer(i) => MetaValue::Integer(i),
        V::Real(f) => MetaValue::Real(f),
        V::Text(s) => MetaValue::Text(s),
        V::Blob(b) => MetaValue::Blob(b),
    }
}

#[cfg(test)]
mod tests;
