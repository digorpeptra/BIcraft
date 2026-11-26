//! L2: typed table data.
//!
//! Produces row-aligned, typed column values ([`TableData`]) from sources that
//! bicraft can read without Power BI Desktop, so a PBIX can be compiled with
//! data:
//!
//! - CSV text ([`csv::read_csv`]), with culture-aware type inference;
//! - Power Query (M) expressions, evaluated by a small sandboxed interpreter
//!   ([`m::load_m`], [`m::load_m_with`]): `#table` literals, Desktop's "Enter
//!   data" tables, the common `Table.*`/`List.*`/`Text.*` transforms and, under
//!   an explicit allow-list of folders only, local CSV/JSON/Excel files and
//!   folder listings;
//! - DAX literal tables: `DATATABLE`, table constructors, `ROW`, `CALENDAR`
//!   over literal dates ([`dax::load_dax`]).
//!
//! [`load_partition`] dispatches on a model partition's source type and maps
//! the result onto the table's declared columns ([`conform`]). An external
//! connector (a database, the web, a script) is [`Error::ExternalSource`]: it is
//! never run. Anything else bicraft cannot evaluate here (unsupported functions,
//! DAX beyond literals, which the engine evaluates) is
//! [`Error::UnsupportedSource`]. The rules are documented in
//! `docs/formats/data-sources.md`.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod connect;
pub mod csv;
mod csvscan;
pub mod dax;
pub mod m;
pub mod store;
mod value;

pub use bicraft_model::DataType;
pub use bicraft_vertipaq::CellValue;
pub use value::{Culture, DateOrder};

use bicraft_model::{Column, Partition};
use value::Coercion;

/// Errors from the loaders. Row numbers are 1-based data rows (the header is not counted).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The source is not something bicraft can load without an engine.
    #[error("unsupported source: {0}")]
    UnsupportedSource(String),
    /// The query reads an external source (a database, a web service, a script runtime):
    /// bicraft never connects to one, so the query is not evaluated. The query text itself
    /// is not changed.
    #[error("external source, not evaluated: {0}")]
    ExternalSource(String),
    /// Malformed source text (`format` is `csv`, `M` or `DAX`).
    #[error("{format} syntax error at line {line}, column {column}: {message}")]
    Syntax {
        format: &'static str,
        line: usize,
        column: usize,
        message: String,
    },
    /// A value that does not fit its column.
    #[error("row {row}, column `{column}`: {message}")]
    Value {
        row: usize,
        column: String,
        message: String,
    },
    /// A row with the wrong number of values.
    #[error("row {row} has {found} values, expected {expected}")]
    RowLength {
        row: usize,
        expected: usize,
        found: usize,
    },
    /// A model column whose source column is not in the data.
    #[error("column `{column}` (source `{source_column}`) not found in the source data")]
    MissingColumn { column: String, source_column: String },
    /// A [`TableData`] invariant is violated.
    #[error("invalid table: {0}")]
    InvalidTable(String),
    /// A size cap from [`Limits`] was hit.
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
    /// A connector the caller allowed failed in a way `try` cannot catch: missing or rejected
    /// credentials, the network, the service ([`connect::ConnectErrorKind`]). The message never
    /// carries a credential.
    #[error("connector error: {0}")]
    Connector(String),
    /// The M expression raised an error (`reason` is the M error reason, e.g.
    /// `Expression.Error` or `DataFormat.Error`).
    #[error("M evaluation error: {reason}: {message}")]
    Evaluation { reason: String, message: String },
}

/// Caps applied to hostile input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Maximum source size in bytes (CSV text, M or DAX expression).
    pub max_bytes: usize,
    /// Maximum data rows.
    pub max_rows: usize,
    /// Maximum columns.
    pub max_columns: usize,
    /// Maximum size of one text cell, in bytes.
    pub max_cell_bytes: usize,
    /// Maximum nesting depth of M/DAX expressions (parsing).
    pub max_depth: usize,
    /// Maximum nesting of M evaluation (function calls, step references, recursion).
    pub max_eval_depth: usize,
    /// Maximum units of M evaluation work (expression steps, rows and items touched).
    pub max_steps: u64,
    /// Maximum wall-clock time of one M evaluation, in milliseconds (0 = no limit).
    pub max_millis: u64,
    /// Maximum size of a CSV file the fast CSV path streams (`Csv.Document(File.Contents(…))`
    /// in the shapes of `docs/formats/data-sources.md`, *Fast CSV loading*). Other file reads
    /// keep [`Limits::max_bytes`].
    pub max_file_bytes: u64,
    /// Memory budget of one CSV load by the fast paths, in bytes: the table's cells and texts
    /// and the read buffers, estimated before loading and counted while loading. A load over
    /// it is refused ([`Error::LimitExceeded`]). The bytes are also reserved against the
    /// process-wide memory limit (`BICRAFT_MEMORY_LIMIT`, `bicraft-mem`) while loading.
    pub max_load_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 256 << 20,
            max_rows: 10_000_000,
            max_columns: 4096,
            max_cell_bytes: 1 << 20,
            max_depth: 64,
            max_eval_depth: 2000,
            max_steps: 500_000_000,
            max_millis: 120_000,
            max_file_bytes: 64 << 30,
            max_load_bytes: usize::try_from(16u64 << 30).unwrap_or(usize::MAX),
        }
    }
}

impl Limits {
    pub(crate) fn check_bytes(&self, n: usize, what: &str) -> Result<(), Error> {
        if n > self.max_bytes {
            return Err(Error::LimitExceeded(format!(
                "{what} is {n} bytes (max {})",
                self.max_bytes
            )));
        }
        Ok(())
    }

    pub(crate) fn check_rows(&self, n: usize) -> Result<(), Error> {
        if n > self.max_rows {
            return Err(Error::LimitExceeded(format!("more than {} rows", self.max_rows)));
        }
        Ok(())
    }

    pub(crate) fn check_columns(&self, n: usize) -> Result<(), Error> {
        if n > self.max_columns {
            return Err(Error::LimitExceeded(format!(
                "{n} columns (max {})",
                self.max_columns
            )));
        }
        Ok(())
    }
}

/// Options for evaluating M sources ([`load_partition_opts`], [`m::load_m_with`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SourceOptions {
    pub limits: Limits,
    /// The allow-list of directories `File.Contents`, `Folder.Files` and
    /// `Folder.Contents` may read (recursively). Empty (the default) disables
    /// file access: any of them is [`Error::UnsupportedSource`]. Paths are
    /// resolved (`..`, symbolic links) before the check and must lie under one
    /// of the roots; a relative path is joined to the first root. Folder
    /// listings never follow a link out of the roots.
    pub file_roots: Vec<std::path::PathBuf>,
    /// Culture for text ↔ number/date conversions without an explicit culture
    /// (the model's `culture`, e.g. `fr-FR`). `None` is `en-US`.
    pub culture: Option<String>,
    /// Named M expressions the partition may reference: other queries and
    /// parameters (`name`, M text), as in a model's shared expressions.
    pub shared: Vec<(String, String)>,
    /// Fail on cells that hold an M error (`Error::Value`) instead of loading
    /// them as blanks. Desktop loads them as blanks (and reports the count),
    /// whether or not the model sets `returnErrorValuesAsNull`.
    pub strict_errors: bool,
    /// The connectors the evaluation may run ([`connect`]). Empty (the default): every
    /// connector function is [`Error::ExternalSource`] and nothing reaches the network.
    pub connectors: connect::Connectors,
    /// What `DateTimeZone.UtcNow()` and `DateTimeZone.FixedUtcNow()` return, in milliseconds
    /// since 1899-12-30T00:00 UTC (a refresh sets it to its start time). `None` (the default):
    /// clock functions are [`Error::UnsupportedSource`]. Local-time clock functions stay
    /// unsupported (they need the machine's time zone).
    pub clock_utc_ms: Option<i64>,
    /// Local time minus UTC, in minutes, for `DateTime.LocalNow()`, `DateTimeZone.LocalNow()` and
    /// their `Fixed` forms, with `clock_utc_ms`. `None` (the default): they are
    /// [`Error::UnsupportedSource`]. bicraft passes 0: its local clock is UTC, as in the Power BI
    /// service.
    pub clock_local_offset_minutes: Option<i32>,
}

/// One typed column.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnData {
    pub name: String,
    pub data_type: DataType,
    /// One value per row; `CellValue::Null` is blank.
    pub values: Vec<CellValue>,
}

/// Typed, row-aligned column data for one table.
///
/// Invariants (checked by [`TableData::new`]): every column has the same
/// number of values, names are non-empty and unique (case-insensitively, as
/// in Power BI), and each value fits its column type (`Int64` → `Int`,
/// `Double` → `Real`, `Decimal` → `Decimal`, `DateTime` → `DateTime`,
/// `Boolean` → `Bool`, `String` → `Text`, `Binary` → blanks only,
/// `Unknown`/`Variant` → anything; `Null` fits everywhere).
#[derive(Debug, Clone, Default)]
pub struct TableData {
    columns: Vec<ColumnData>,
    rows: usize,
    /// The M type each `DateTime` column was declared with (M loads only; empty otherwise).
    /// Not part of equality: it describes the source, not the data.
    date_kinds: Vec<Option<DateKind>>,
}

impl PartialEq for TableData {
    fn eq(&self, other: &Self) -> bool {
        self.columns == other.columns && self.rows == other.rows
    }
}

/// The M type a `DateTime` column was declared with (`type date`, `time`, `datetime`,
/// `datetimezone`): Power BI Desktop keeps it as the column's `UnderlyingDateTimeDataType`
/// annotation and picks the default format string from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateKind {
    Date,
    Time,
    DateTime,
    DateTimeZone,
}

impl TableData {
    /// Validates and builds a table.
    pub fn new(columns: Vec<ColumnData>) -> Result<Self, Error> {
        let rows = columns.first().map_or(0, |c| c.values.len());
        let mut seen: Vec<String> = Vec::with_capacity(columns.len());
        for c in &columns {
            if c.name.is_empty() {
                return Err(Error::InvalidTable("a column has an empty name".into()));
            }
            let lower = c.name.to_lowercase();
            if seen.contains(&lower) {
                return Err(Error::InvalidTable(format!("duplicate column name `{}`", c.name)));
            }
            seen.push(lower);
            if c.values.len() != rows {
                return Err(Error::InvalidTable(format!(
                    "column `{}` has {} values, expected {rows}",
                    c.name,
                    c.values.len()
                )));
            }
            if let Some((i, v)) = c
                .values
                .iter()
                .enumerate()
                .find(|(_, v)| !value::fits(v, c.data_type))
            {
                return Err(Error::Value {
                    row: i + 1,
                    column: c.name.clone(),
                    message: format!("{v:?} does not fit column type {:?}", c.data_type),
                });
            }
        }
        Ok(Self {
            columns,
            rows,
            date_kinds: Vec::new(),
        })
    }

    /// The M type column `i` was declared with, for `DateTime` columns of an M load.
    pub fn date_kind(&self, i: usize) -> Option<DateKind> {
        self.date_kinds.get(i).copied().flatten()
    }

    pub(crate) fn set_date_kinds(&mut self, kinds: Vec<Option<DateKind>>) {
        self.date_kinds = kinds;
    }

    /// The columns, in order.
    pub fn columns(&self) -> &[ColumnData] {
        &self.columns
    }

    /// A column by name (exact match first, then case-insensitive).
    pub fn column(&self, name: &str) -> Option<&ColumnData> {
        self.columns.iter().find(|c| c.name == name).or_else(|| {
            self.columns
                .iter()
                .find(|c| c.name.to_lowercase() == name.to_lowercase())
        })
    }

    /// Number of rows.
    pub fn row_count(&self) -> usize {
        self.rows
    }

    /// Takes the columns out.
    pub fn into_columns(self) -> Vec<ColumnData> {
        self.columns
    }

    /// The values of one row, in column order.
    pub fn row(&self, i: usize) -> Option<Vec<&CellValue>> {
        if i >= self.rows {
            return None;
        }
        self.columns.iter().map(|c| c.values.get(i)).collect()
    }
}

/// The source column name a model column reads: `sourceColumn` (with the
/// `[...]` brackets of calculated-table columns removed), else the column name.
fn source_name(c: &Column) -> &str {
    let s = c.source_column.as_deref().unwrap_or(&c.name).trim();
    match (s.find('['), s.ends_with(']')) {
        (Some(open), true) => s.get(open + 1..s.len() - 1).unwrap_or(s),
        _ => s,
    }
}

/// Maps loaded data onto a table's declared columns.
///
/// Every model column without a DAX `expression` (data columns and
/// calculated-table columns) is looked up by its source column name — exact
/// match first, then case-insensitive — and its values are converted to the
/// declared data type (`Unknown` keeps the source type). Calculated columns
/// are skipped (the engine computes them) and unreferenced source columns are
/// dropped. Conversion rules are in `docs/formats/data-sources.md`; a value
/// that cannot be converted is an [`Error::Value`] naming the row and column.
pub fn conform(data: TableData, columns: &[Column]) -> Result<TableData, Error> {
    let mut source = data.columns;
    let mode = Coercion {
        parse_text: true,
        format_text: true,
    };
    let mut out = Vec::new();
    for col in columns.iter().filter(|c| c.expression.is_none()) {
        let wanted = source_name(col);
        let idx = source.iter().position(|c| c.name == wanted).or_else(|| {
            source
                .iter()
                .position(|c| c.name.to_lowercase() == wanted.to_lowercase())
        });
        let Some(idx) = idx else {
            return Err(Error::MissingColumn {
                column: col.name.clone(),
                source_column: wanted.to_string(),
            });
        };
        // Two model columns may read the same source column: clone in that case.
        let shared = columns
            .iter()
            .filter(|c| c.expression.is_none() && source_name(c).to_lowercase() == wanted.to_lowercase())
            .count()
            > 1;
        let src = match source.get_mut(idx) {
            Some(s) if shared => s.clone(),
            Some(s) => std::mem::replace(
                s,
                ColumnData {
                    name: String::new(),
                    data_type: s.data_type,
                    values: Vec::new(),
                },
            ),
            None => continue,
        };
        let target = if col.data_type == DataType::Unknown {
            src.data_type
        } else {
            col.data_type
        };
        // In place: a large column is never held twice.
        let mut values = src.values;
        for (i, slot) in values.iter_mut().enumerate() {
            if value::fits(slot, target) {
                continue;
            }
            let v = std::mem::replace(slot, CellValue::Null);
            *slot = value::coerce(v, target, mode).map_err(|message| Error::Value {
                row: i + 1,
                column: col.name.clone(),
                message,
            })?;
        }
        out.push(ColumnData {
            name: col.name.clone(),
            data_type: target,
            values,
        });
    }
    // Keep the row count when no data column is declared.
    if out.is_empty() {
        return Ok(TableData::default());
    }
    TableData::new(out)
}

/// Loads a partition's data and maps it onto `columns` (the table's columns).
///
/// `source_type` `m` → [`m::load_m`] (no file access); `calculated` →
/// [`dax::load_dax`]; anything else (or a missing expression) →
/// [`Error::UnsupportedSource`].
pub fn load_partition(partition: &Partition, columns: &[Column]) -> Result<TableData, Error> {
    load_partition_with(partition, columns, &Limits::default())
}

/// [`load_partition`] with explicit [`Limits`].
pub fn load_partition_with(
    partition: &Partition,
    columns: &[Column],
    limits: &Limits,
) -> Result<TableData, Error> {
    load_partition_opts(
        partition,
        columns,
        &SourceOptions {
            limits: *limits,
            ..SourceOptions::default()
        },
    )
}

/// [`load_partition`] with [`SourceOptions`]: limits, the file root for
/// `File.Contents`, the culture and the shared queries/parameters.
pub fn load_partition_opts(
    partition: &Partition,
    columns: &[Column],
    opts: &SourceOptions,
) -> Result<TableData, Error> {
    let limits = &opts.limits;
    let Some(expr) = partition.expression.as_deref() else {
        return Err(Error::UnsupportedSource(format!(
            "partition `{}` has no source expression",
            partition.name
        )));
    };
    let data = match partition.source_type.to_ascii_lowercase().as_str() {
        "m" => m::load_m_with(expr, opts)?,
        "calculated" => dax::load_dax(expr, limits)?,
        other => {
            return Err(Error::UnsupportedSource(format!(
                "partition `{}` has source type `{other}`; only `m` and `calculated` sources load",
                partition.name
            )));
        }
    };
    conform(data, columns)
}

#[cfg(test)]
#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, t: DataType, values: Vec<CellValue>) -> ColumnData {
        ColumnData {
            name: name.into(),
            data_type: t,
            values,
        }
    }

    fn model_col(name: &str, t: DataType, source: Option<&str>, expr: Option<&str>) -> Column {
        Column {
            name: name.into(),
            data_type: t,
            source_column: source.map(Into::into),
            expression: expr.map(Into::into),
            ..Default::default()
        }
    }

    #[test]
    fn table_validation() {
        assert!(TableData::new(vec![col("A", DataType::Int64, vec![CellValue::Int(1)])]).is_ok());
        assert!(matches!(
            TableData::new(vec![col("A", DataType::Int64, vec![CellValue::Real(1.0)])]),
            Err(Error::Value { row: 1, .. })
        ));
        assert!(matches!(
            TableData::new(vec![
                col("A", DataType::Int64, vec![]),
                col("a", DataType::Int64, vec![])
            ]),
            Err(Error::InvalidTable(_))
        ));
        assert!(matches!(
            TableData::new(vec![
                col("A", DataType::Int64, vec![CellValue::Null]),
                col("B", DataType::Int64, vec![])
            ]),
            Err(Error::InvalidTable(_))
        ));
        assert!(TableData::new(vec![col("", DataType::Int64, vec![])]).is_err());
        let t = TableData::new(vec![col(
            "A",
            DataType::Variant,
            vec![CellValue::Bool(true), CellValue::Null],
        )])
        .unwrap();
        assert_eq!(t.row_count(), 2);
        assert_eq!(t.row(0), Some(vec![&CellValue::Bool(true)]));
        assert_eq!(t.row(2), None);
        assert!(t.column("a").is_some());
    }

    #[test]
    fn conform_maps_names_types_and_skips_calculated() {
        let data = TableData::new(vec![
            col("id", DataType::Int64, vec![CellValue::Int(1), CellValue::Int(2)]),
            col(
                "Amount",
                DataType::Int64,
                vec![CellValue::Int(5), CellValue::Null],
            ),
            col("Unused", DataType::String, vec![CellValue::Null, CellValue::Null]),
        ])
        .unwrap();
        let cols = [
            model_col("ID", DataType::String, Some("id"), None),
            model_col("Montant", DataType::Decimal, Some("Amount"), None),
            model_col("Double", DataType::Double, None, Some("[Montant] * 2")),
        ];
        let t = conform(data, &cols).unwrap();
        assert_eq!(t.columns().len(), 2);
        assert_eq!(t.columns()[0].name, "ID");
        assert_eq!(
            t.columns()[0].values,
            vec![CellValue::Text("1".into()), CellValue::Text("2".into())]
        );
        assert_eq!(
            t.columns()[1].values,
            vec![CellValue::Decimal(50_000), CellValue::Null]
        );
    }

    #[test]
    fn conform_errors_name_row_and_column() {
        let data = TableData::new(vec![col(
            "A",
            DataType::String,
            vec![CellValue::Text("1".into()), CellValue::Text("x".into())],
        )])
        .unwrap();
        let err = conform(data.clone(), &[model_col("A", DataType::Int64, None, None)]).unwrap_err();
        assert!(
            matches!(&err, Error::Value { row: 2, column, .. } if column == "A"),
            "{err}"
        );
        let err = conform(data, &[model_col("B", DataType::Int64, None, None)]).unwrap_err();
        assert!(matches!(err, Error::MissingColumn { .. }));
    }

    #[test]
    fn conform_shared_source_and_brackets() {
        let data = TableData::new(vec![col(
            "Date",
            DataType::DateTime,
            vec![CellValue::DateTime(2.0)],
        )])
        .unwrap();
        let cols = [
            model_col("Date", DataType::DateTime, Some("[Date]"), None),
            model_col("Copy", DataType::String, Some("Table[Date]"), None),
        ];
        let t = conform(data, &cols).unwrap();
        assert_eq!(t.columns()[1].values, vec![CellValue::Text("1900-01-01".into())]);
    }

    #[test]
    fn load_partition_dispatch() {
        let p = Partition {
            name: "P".into(),
            source_type: "m".into(),
            expression: Some("let\n    Source = #table({\"A\"}, {{1}})\nin\n    Source".into()),
            ..Default::default()
        };
        let t = load_partition(&p, &[model_col("A", DataType::Int64, Some("A"), None)]).unwrap();
        assert_eq!(t.columns()[0].values, vec![CellValue::Int(1)]);

        let p = Partition {
            source_type: "calculated".into(),
            expression: Some("{1, 2}".into()),
            ..p
        };
        let t = load_partition(&p, &[model_col("Value", DataType::Double, Some("[Value]"), None)]).unwrap();
        assert_eq!(
            t.columns()[0].values,
            vec![CellValue::Real(1.0), CellValue::Real(2.0)]
        );

        let q = Partition {
            source_type: "query".into(),
            ..p.clone()
        };
        assert!(matches!(
            load_partition(&q, &[]),
            Err(Error::UnsupportedSource(_))
        ));
        let q = Partition {
            expression: None,
            ..p
        };
        assert!(matches!(
            load_partition(&q, &[]),
            Err(Error::UnsupportedSource(_))
        ));
    }

    /// The demo project's tables (`bicraft-demo/Demo.SemanticModel/model.bim`), as the TMSL
    /// reader hands them over: M partitions plus an auto date table.
    #[test]
    fn demo_project_tables() {
        let ventes = Partition {
            name: "Ventes".into(),
            mode: Some("import".into()),
            source_type: "m".into(),
            expression: Some(
                "let
    Source = #table(type table [Date = date, ProduitID = Int64.Type, Montant = number],                  {{#date(2026,1,5), 1, 450}, {#date(2026,1,7), 2, 60}, {#date(2026,2,3), 1, 520},                  {#date(2026,2,9), 3, 15}})
in
    Source"
                    .into(),
            ),
            ..Default::default()
        };
        let cols = [
            model_col("Date", DataType::DateTime, Some("Date"), None),
            model_col("ProduitID", DataType::Int64, Some("ProduitID"), None),
            model_col("Montant", DataType::Double, Some("Montant"), None),
        ];
        let t = load_partition(&ventes, &cols).unwrap();
        assert_eq!(t.row_count(), 4);
        assert_eq!(t.columns()[2].values[3], CellValue::Real(15.0));

        let local_dates = Partition {
            name: "LocalDateTable".into(),
            source_type: "calculated".into(),
            expression: Some("Calendar(Date(2015,1,1), Date(2015,1,1))".into()),
            ..Default::default()
        };
        let cols = [
            model_col("Date", DataType::DateTime, Some("[Date]"), None),
            model_col("Année", DataType::Int64, None, Some("YEAR([Date])")),
        ];
        let t = load_partition(&local_dates, &cols).unwrap();
        assert_eq!(t.columns().len(), 1);
        assert_eq!(t.columns()[0].values, vec![CellValue::DateTime(42_005.0)]);

        let auto = Partition {
            expression: Some(
                "Calendar(Date(Year(MIN('Ventes'[Date])), 1, 1), Date(Year(MAX('Ventes'[Date])), 12, 31))"
                    .into(),
            ),
            ..local_dates
        };
        assert!(matches!(
            load_partition(&auto, &cols),
            Err(Error::UnsupportedSource(_))
        ));
    }
}
