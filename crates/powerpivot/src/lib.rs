//! L3: Excel workbooks with a Power Pivot data model (`.xlsx`, `.xlsm`).
//!
//! The model is an Analysis Services backup stored in the part `xl/model/item.data`
//! ([MS-XLDM]): an ABF whose files are each compressed with the Plain LZ77 of [MS-XCA]
//! (`ApplyCompression`, [`bicraft_xpress8`]) and followed by CRC markers, the model described
//! by ASSL XML documents (compatibility levels 1100/1103), and the data stored as VertiPaq
//! columns described by `XMObject` XML trees. [`read_workbook`] turns it into a
//! [`SemanticModel`] and the values of every table, decoded by `bicraft-vertipaq`; the engine
//! then writes them as a PBIX or PBIP. The sheets and everything else in the workbook are not
//! read. The format facts are in `docs/formats/powerpivot.md`.
//!
//! Every input is untrusted: sizes are capped by [`Limits`], XML is parsed without DTDs and
//! with a node limit, and every malformed input yields an [`Error`] or a warning, never a panic.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod backup;
mod storage;
#[doc(hidden)]
pub mod synthetic;
mod xml;

use bicraft_model::SemanticModel;
use bicraft_vertipaq::CellValue;

/// The part of a workbook that holds the Power Pivot backup ([MS-XLDM]).
pub const ITEM_DATA: &str = "xl/model/item.data";

/// Why a workbook cannot be imported.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("not an Excel workbook: {0}")]
    NotWorkbook(String),
    #[error(
        "this Excel workbook has no Power Pivot data model (no xl/model/item.data): only a workbook with a data \
         model can be opened; to load its sheets, use Get data › Excel in a model"
    )]
    NoDataModel,
    #[error("the workbook's data model cannot be read: {0}")]
    Backup(String),
    #[error(
        "the workbook's data model uses the tabular metadata of newer Excel versions (metadata.sqlitedb), which \
         bicraft does not import yet"
    )]
    TabularMetadata,
    #[error("the workbook's data model has invalid metadata: {0}")]
    Metadata(String),
    #[error("the workbook's data model exceeds a limit: {0}")]
    Limit(String),
}

/// Caps applied while reading. `Default` suits real workbooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Largest `item.data` part, and largest backup once decompressed.
    pub max_backup_bytes: u64,
    /// Largest file of the backup once decompressed.
    pub max_file_bytes: u64,
    /// Largest XML document of the backup.
    pub max_xml_bytes: u64,
    /// Most XML nodes of one document.
    pub max_xml_nodes: u32,
    /// Most cells decoded over the whole model.
    pub max_cells: u64,
    /// Caps of each column decode.
    pub vertipaq: bicraft_vertipaq::Limits,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_backup_bytes: 4 << 30,
            max_file_bytes: 2 << 30,
            max_xml_bytes: 64 << 20,
            max_xml_nodes: 4 << 20,
            max_cells: 1 << 26,
            vertipaq: bicraft_vertipaq::Limits::default(),
        }
    }
}

/// The values of one table, one vector per column in model order (row-number column excluded).
#[derive(Debug, Clone, PartialEq)]
pub struct TableRows {
    pub name: String,
    pub columns: Vec<(String, Vec<CellValue>)>,
}

impl TableRows {
    /// Rows of the table (all columns have the same length).
    pub fn rows(&self) -> usize {
        self.columns.first().map_or(0, |(_, v)| v.len())
    }
}

/// An imported data model.
#[derive(Debug, Clone, PartialEq)]
pub struct Workbook {
    /// Tables, columns (data and calculated), measures, relationships and hierarchies. Each table
    /// has one Power Query partition that keeps the original source query as a comment and fails
    /// on refresh (the Power Pivot connections are not translated).
    pub model: SemanticModel,
    /// The data of every table of `model`, in the same order.
    pub tables: Vec<TableRows>,
    /// The database's `CompatibilityLevel` (1100 or 1103 for Power Pivot).
    pub compatibility_level: Option<u32>,
    /// What could not be imported (a table's data, a measure of an unknown table, …).
    pub warnings: Vec<String>,
}

/// Reads the Power Pivot data model of a workbook (the bytes of an `.xlsx` or `.xlsm`).
/// [`Error::NoDataModel`] for a workbook without one.
pub fn read_workbook(bytes: &[u8], limits: &Limits) -> Result<Workbook, Error> {
    let read_limits = bicraft_opc::ReadLimits {
        max_part_bytes: limits.max_backup_bytes,
        max_total_bytes: limits.max_backup_bytes.saturating_mul(2),
    };
    let package = bicraft_opc::Package::read_bytes_with(bytes, &read_limits)
        .map_err(|e| Error::NotWorkbook(e.to_string()))?;
    let Some(item) = package.get(ITEM_DATA) else {
        let is_workbook = package.get("xl/workbook.xml").is_some()
            || package.get("xl/workbook.bin").is_some()
            || package
                .get("[Content_Types].xml")
                .is_some_and(|p| String::from_utf8_lossy(&p.data).contains("spreadsheetml"));
        return Err(if is_workbook {
            Error::NoDataModel
        } else {
            Error::NotWorkbook("the package has no xl/workbook.xml".into())
        });
    };
    let data = item.bytes().map_err(|e| Error::Backup(e.to_string()))?;
    read_backup(&data, limits)
}

/// Reads the backup of `xl/model/item.data` (an ABF, possibly XPress9-framed like a PBIX
/// `DataModel`).
pub fn read_backup(item: &[u8], limits: &Limits) -> Result<Workbook, Error> {
    if item.len() as u64 > limits.max_backup_bytes {
        return Err(Error::Limit(format!(
            "{} bytes of item.data, more than {}",
            item.len(),
            limits.max_backup_bytes
        )));
    }
    let decompressed;
    let abf_bytes = match bicraft_abf::Abf::parse(item) {
        Err(bicraft_abf::Error::Xpress9Compressed) => {
            let limit = usize::try_from(limits.max_backup_bytes).unwrap_or(usize::MAX);
            decompressed = bicraft_xpress9::decompress_datamodel(item, limit)
                .map_err(|e| Error::Backup(e.to_string()))?;
            &decompressed[..]
        }
        _ => item,
    };
    let abf = bicraft_abf::Abf::parse(abf_bytes).map_err(|e| Error::Backup(e.to_string()))?;
    if abf.metadata_entry().is_some() {
        return Err(Error::TabularMetadata);
    }
    let backup = backup::Backup::new(abf, limits);
    let loaded = xml::load(&backup)?;
    let mut model = loaded.model;
    let mut warnings = loaded.warnings;
    let mut tables = Vec::with_capacity(loaded.tables.len());
    let mut cells_left = limits.max_cells;
    for source in &loaded.tables {
        match storage::read_table(&backup, source, &mut cells_left) {
            Ok((rows, inferred)) => {
                if let Some(t) = model.table_mut(&source.name) {
                    for (name, data_type) in inferred {
                        if let Some(c) = t.column_mut(&name) {
                            c.data_type = data_type;
                        }
                    }
                }
                tables.push(rows);
            }
            Err(e @ Error::Limit(_)) => return Err(e),
            Err(e) => {
                warnings.push(format!(
                    "table '{}': its data cannot be read ({e}); it is imported without rows",
                    source.name
                ));
                tables.push(TableRows {
                    name: source.name.clone(),
                    columns: source
                        .columns
                        .iter()
                        .map(|c| (c.name.clone(), Vec::new()))
                        .collect(),
                });
            }
        }
    }
    Ok(Workbook {
        model,
        tables,
        compatibility_level: loaded.compatibility_level,
        warnings,
    })
}

/// Fuzzing entry point: reads `data` as a workbook and as a bare backup with small caps. Never
/// panics.
pub fn fuzz(data: &[u8]) {
    let limits = Limits {
        max_backup_bytes: 1 << 22,
        max_file_bytes: 1 << 22,
        max_xml_bytes: 1 << 20,
        max_xml_nodes: 1 << 16,
        max_cells: 1 << 16,
        vertipaq: bicraft_vertipaq::Limits {
            max_rows: 1 << 16,
            max_bytes: 1 << 22,
        },
    };
    let _ = read_workbook(data, &limits);
    let _ = read_backup(data, &limits);
}
