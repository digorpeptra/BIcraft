//! Structural edits of a PBIX model (gap G1): saving tables, columns and relationships that were
//! added, removed, renamed or given new data, and the `table.import` / `table.set_data`
//! commands that give a table its data.
//!
//! `file.save` on a PBIX goes through [`write_model`]: a model without structural differences
//! takes the metadata-only path; otherwise the values of every new table and column are
//! gathered here and written by `bicraft_pbix::structural` (VertiPaq encoding, system tables,
//! storage rows, ABF entries):
//!
//! - import tables (`m` partitions): the query runs in `bicraft-data`'s Power Query evaluator
//!   (literals, Enter data, the common transforms; CSV/JSON/Excel files and folders only under the session's
//!   opt-in `data_root`), with the model's culture and the queries/parameters it references
//!   (`crate::datasrc`); text is stored as Analysis Services stores it (no trailing blanks, one
//!   spelling per equivalent value);
//! - calculated tables: DAX literal tables (`DATATABLE`, `{…}`, `ROW`, `CALENDAR`) through
//!   `bicraft-data`, anything else through the viewer's DAX evaluator; a calculated table
//!   without declared columns gets them from the result;
//! - calculated columns (of new tables, replaced tables or existing tables): the DAX evaluator,
//!   row by row;
//! - a data column added to an existing import table: the table's `m` partitions run again and
//!   the new column's values are matched to the stored rows through the stored columns (the
//!   query must still return the stored rows); otherwise give the table new data with
//!   `table.set_data`;
//! - tables of several `m` partitions: each partition's query runs on its own; each partition
//!   keeps its rows (`bicraft_pbix::structural::PartitionRows`).
//!
//! Whatever cannot be evaluated exactly is refused with the reason; nothing is written then.

use std::path::Path;

use bicraft_data::csv::{CsvOptions, read_csv};
use bicraft_data::{Culture, DataType, TableData};
use bicraft_model::{Column, Partition, SemanticModel, Table};
use bicraft_opc::Package;
use bicraft_pbix::structural::{
    ColumnValues, DataModelEditor, NewColumnValues, PartitionRows, StructuralData, TableValues,
};
use bicraft_vertipaq::CellValue;
use serde_json::{Map, Value, json};

use crate::Session;
use crate::commands::model;
use crate::edit::{check_name, new_guid};
use crate::params::{allow, opt_bool, opt_str, str_param};
use crate::preview::StructEval;

/// Largest CSV file `table.import {csv}` reads: its data becomes an M `#table` literal in the
/// model. Larger files load through `m` (`Csv.Document(File.Contents(…))` under `data_root`),
/// which streams the file (`docs/formats/data-sources.md`, *Fast CSV loading*).
const MAX_CSV_BYTES: u64 = 256 << 20;
/// Most rows of one table written or evaluated by a structural save.
const MAX_ROWS: usize = bicraft_pbix::structural::MAX_ROWS;
/// Passes over pending calculated columns (they may depend on each other).
const MAX_PASSES: usize = 16;

/// Writes `model` into a copy of `package` (see the module documentation). PBIT and thin
/// packages go through `bicraft_pbix::write_model`. `root`: the folder Power Query sources may
/// read (`None`: no file access).
pub(crate) fn write_model(
    package: &Package,
    model: &SemanticModel,
    root: Option<&Path>,
) -> Result<Package, String> {
    let part = match package.get(bicraft_pbix::DATA_MODEL) {
        Some(p) if package.get(bicraft_pbix::DATA_MODEL_SCHEMA).is_none() => p,
        _ => return bicraft_pbix::write_model(package, model).map_err(|e| e.to_string()),
    };
    if part.len() > bicraft_opc::MAX_PART_BYTES {
        return Err(bicraft_pbix::Error::DataModelTooLargeToWrite(part.len()).to_string());
    }
    // a part left in the file by choice (memory budget) is read for the write
    let part_bytes = part.bytes().map_err(|e| e.to_string())?;
    let editor = DataModelEditor::open(&part_bytes, &bicraft_pbix::DataModelLimits::default())
        .map_err(|e| e.to_string())?;
    let changes = editor.changes(model);
    let bytes = if changes.is_empty() {
        editor.write(model, &StructuralData::default())
    } else {
        let g = gather(&changes, model, &editor, &part_bytes, &Load::files(root))?;
        editor.write_partitioned(&g.model, &g.data, &g.partitions, None)
    }
    .map_err(|e| e.to_string())?;
    let name = part.name.clone();
    let mut out = package.clone();
    out.put(&name, bytes, true);
    Ok(out)
}

fn is_row_number(c: &Column) -> bool {
    c.extra.get("type").and_then(Value::as_str) == Some("rowNumber")
}

fn find_table<'m>(m: &'m SemanticModel, name: &str) -> Option<&'m Table> {
    m.tables
        .iter()
        .find(|t| t.name == name)
        .or_else(|| m.tables.iter().find(|t| t.name.eq_ignore_ascii_case(name)))
}

fn find_table_mut<'m>(m: &'m mut SemanticModel, name: &str) -> Option<&'m mut Table> {
    let i = m
        .tables
        .iter()
        .position(|t| t.name == name)
        .or_else(|| m.tables.iter().position(|t| t.name.eq_ignore_ascii_case(name)))?;
    m.tables.get_mut(i)
}

/// Values of the data columns of a table, keyed by column name.
pub(crate) type Cols = Vec<(String, Vec<CellValue>)>;

/// A check of each table's loaded data (rows, columns) before the next one loads.
pub(crate) type Admit<'a> = &'a dyn Fn(&str, usize, &Cols) -> Result<(), String>;

/// How a structural save (or a PBIX refresh, `refresh_pbix.rs`) runs the tables' sources.
#[derive(Default)]
pub(crate) struct Load<'a> {
    /// The only folder Power Query file sources may read (`None`: no file access).
    pub root: Option<&'a Path>,
    /// Connectors `model.refresh` allows (Databricks); none for a save.
    pub connectors: bicraft_data::connect::Connectors,
    /// The clock of the refresh (ms since 1899-12-30 UTC) the M clock functions read; `None`:
    /// the time of each evaluation.
    pub clock: Option<i64>,
    /// Local time minus UTC, in minutes (`model.refresh`'s `utc_offset_minutes`), for the M
    /// clock functions and calculated tables' and columns' `NOW()`/`UTCNOW()`; `None`: the
    /// process's local clock (`crate::clock`).
    pub offset_minutes: Option<i32>,
    /// Replaced tables whose data columns are read back from the stored `DataModel` instead of
    /// their source (a refresh that recomputes their calculated columns only).
    pub stored: Vec<String>,
    /// Called with each table's loaded data before the next one loads (a refresh reserves its
    /// memory there and can refuse).
    pub admit: Option<Admit<'a>>,
}

impl<'a> Load<'a> {
    /// Files under `root`, nothing else.
    pub(crate) fn files(root: Option<&'a Path>) -> Self {
        Self {
            root,
            ..Self::default()
        }
    }

    fn options(
        &self,
        m: &SemanticModel,
        table: &str,
        expr: &str,
        limits: bicraft_data::Limits,
    ) -> bicraft_data::SourceOptions {
        let mut o = crate::datasrc::source_options(Some(m), table, expr, self.root, limits);
        o.connectors = self.connectors.clone();
        if let Some(clock) = self.clock {
            o.clock_utc_ms = Some(clock);
            o.clock_local_offset_minutes = Some(
                self.offset_minutes
                    .unwrap_or_else(|| crate::clock::offset_at_serial_ms(clock)),
            );
        }
        o
    }

    fn is_stored(&self, table: &str) -> bool {
        self.stored.iter().any(|t| t.eq_ignore_ascii_case(table))
    }

    fn admit(&self, table: &str, rows: usize, cols: &Cols) -> Result<(), String> {
        match self.admit {
            Some(f) => f(table, rows, cols),
            None => Ok(()),
        }
    }
}

/// One table [`gather`] gave data: where its rows came from and how long that took.
#[derive(Debug, Clone)]
pub(crate) struct TableStat {
    pub name: String,
    /// `m` (its Power Query partitions), `dax` (a calculated table), `stored` (data columns
    /// read back, calculated columns recomputed), `literal` (a calculation group's items).
    pub origin: &'static str,
    pub rows: usize,
    pub columns: usize,
    pub partitions: usize,
    pub millis: u64,
}

/// What [`gather`] returns: the full model (calculated tables get their columns), the values of
/// every new or replaced table and new column, the partition row counts, and per-table stats.
pub(crate) struct Gathered {
    pub model: SemanticModel,
    pub data: StructuralData,
    pub partitions: Vec<PartitionRows>,
    pub stats: Vec<TableStat>,
}

pub(crate) fn millis_since(t: std::time::Instant) -> u64 {
    u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// A table being given data.
struct Pending {
    name: String,
    rows: usize,
    cols: Cols,
    replaced: bool,
    /// Rows of each partition of a table of several partitions (empty: one partition).
    partitions: Vec<u64>,
    /// Where its rows came from (`m`, `dax`, `stored`, `literal`).
    origin: &'static str,
    /// Milliseconds its source took.
    millis: u64,
}

/// What is left to evaluate.
enum Work {
    /// A calculated table the literal loaders cannot read.
    Table {
        name: String,
        expression: String,
        replaced: bool,
    },
    /// A calculated column.
    Column {
        table: String,
        column: String,
        expression: String,
    },
}

fn column_work(t: &Table) -> Vec<Work> {
    t.columns
        .iter()
        .filter_map(|c| {
            c.expression.as_ref().map(|e| Work::Column {
                table: t.name.clone(),
                column: c.name.clone(),
                expression: e.clone(),
            })
        })
        .collect()
}

/// The full model (calculated tables get their columns) and the values of every new table and
/// column.
pub(crate) fn gather(
    changes: &bicraft_pbix::structural::Changes,
    model: &SemanticModel,
    editor: &DataModelEditor,
    data_model: &[u8],
    ld: &Load<'_>,
) -> Result<Gathered, String> {
    let root = ld.root;
    let mut m = model.clone();
    let mut eval: Option<StructEval> = None;
    let mut tables: Vec<Pending> = Vec::new();
    let mut work: Vec<Work> = Vec::new();
    let mut problems = Vec::new();

    // 1. Literal data of new and replaced tables; other calculated tables are evaluated below.
    for name in changes.tables_needing_data() {
        let replaced = changes.replaced_tables.contains(name);
        let Some(t) = find_table(&m, name).cloned() else {
            continue;
        };
        let t0 = std::time::Instant::now();
        match load_table(&t, &mut m, editor, ld) {
            Ok(Some((rows, cols, partitions, origin))) => {
                ld.admit(&t.name, rows, &cols)?;
                if let Some(t) = find_table(&m, name) {
                    work.extend(column_work(t));
                }
                tables.push(Pending {
                    name: t.name.clone(),
                    rows,
                    cols,
                    replaced,
                    partitions,
                    origin,
                    millis: millis_since(t0),
                });
            }
            Ok(None) => work.push(Work::Table {
                name: t.name.clone(),
                expression: t
                    .partitions
                    .first()
                    .and_then(|p| p.expression.clone())
                    .unwrap_or_default(),
                replaced,
            }),
            Err(e) => problems.push(format!("table '{}': {e}", t.name)),
        }
    }
    // 2. Columns added to existing tables: calculated ones are evaluated, data columns read
    //    from the table's source queries run again.
    let mut added: Vec<NewColumnValues> = Vec::new();
    let mut data_columns: Vec<(String, Vec<String>)> = Vec::new();
    for (table, column) in &changes.added_columns {
        let Some(c) = find_table(&m, table)
            .and_then(|t| t.columns.iter().find(|c| c.name.eq_ignore_ascii_case(column)))
        else {
            continue;
        };
        match &c.expression {
            Some(e) => work.push(Work::Column {
                table: table.clone(),
                column: c.name.clone(),
                expression: e.clone(),
            }),
            None => match data_columns
                .iter_mut()
                .find(|(t, _)| t.eq_ignore_ascii_case(table))
            {
                Some((_, cols)) => cols.push(c.name.clone()),
                None => data_columns.push((table.clone(), vec![c.name.clone()])),
            },
        }
    }
    for (table, columns) in &data_columns {
        match added_data_columns(&m, changes, editor, table, columns, ld) {
            Ok(values) => added.extend(values),
            Err(e) => problems.push(e),
        }
    }
    if !problems.is_empty() {
        return Err(refused(&problems));
    }

    // 3. Evaluate, in passes (expressions may depend on each other).
    if !work.is_empty() {
        let ev = evaluator(&mut eval, &m, data_model, root);
        ev.set_clock(ld.clock, ld.offset_minutes);
        ev.rename_stored(&changes.renamed_tables, &changes.renamed_columns);
        ev.set_model(m.clone());
        for t in &tables {
            ev.provide(&t.name, t.rows, &t.cols, t.replaced)?;
        }
        for w in &work {
            // A replaced calculated table must not read its stored values.
            if let Work::Table {
                name, replaced: true, ..
            } = w
            {
                ev.provide(name, 0, &[], true)?;
            }
        }
        let mut errors: Vec<String> = Vec::new();
        for _ in 0..MAX_PASSES {
            if work.is_empty() {
                break;
            }
            errors.clear();
            let mut progressed = false;
            let mut next = Vec::new();
            for w in std::mem::take(&mut work) {
                match w {
                    Work::Table {
                        name,
                        expression,
                        replaced,
                    } => match (std::time::Instant::now(), ev.table(&expression)) {
                        (t0, Ok(result)) => {
                            progressed = true;
                            let (rows, cols) =
                                calculated_table(&mut m, &name, result.columns, &result.sources)
                                    .map_err(|e| refused(&[format!("calculated table '{name}': {e}")]))?;
                            let cols = stored_cols(&m, false, cols);
                            ld.admit(&name, rows, &cols)?;
                            ev.set_model(m.clone());
                            ev.provide(&name, rows, &cols, replaced)?;
                            if let Some(t) = find_table(&m, &name) {
                                next.extend(column_work(t));
                            }
                            tables.push(Pending {
                                name,
                                rows,
                                cols,
                                replaced,
                                partitions: Vec::new(),
                                origin: "dax",
                                millis: millis_since(t0),
                            });
                        }
                        (_, Err(e)) => {
                            errors.push(format!("calculated table '{name}' cannot be evaluated: {e}"));
                            next.push(Work::Table {
                                name,
                                expression,
                                replaced,
                            });
                        }
                    },
                    Work::Column {
                        table,
                        column,
                        expression,
                    } => {
                        let data_type = find_table(&m, &table)
                            .and_then(|t| t.columns.iter().find(|c| c.name == column))
                            .map_or(DataType::Unknown, |c| c.data_type);
                        let result = ev.column(&table, &expression).and_then(|v| {
                            coerce(v, data_type)
                                .map(|(values, _)| crate::datasrc::stored_values(&m, false, &values))
                        });
                        match result {
                            Ok(values) => {
                                progressed = true;
                                ev.provide_column(&table, &column, &values)?;
                                match tables.iter_mut().find(|t| t.name.eq_ignore_ascii_case(&table)) {
                                    Some(t) => t.cols.push((column, values)),
                                    None => added.push(NewColumnValues {
                                        table,
                                        column,
                                        values,
                                    }),
                                }
                            }
                            Err(e) => {
                                errors.push(format!("calculated column '{table}'[{column}]: {e}"));
                                next.push(Work::Column {
                                    table,
                                    column,
                                    expression,
                                });
                            }
                        }
                    }
                }
            }
            work = next;
            if !progressed {
                break;
            }
        }
        if !work.is_empty() {
            return Err(refused(&errors));
        }
    }
    let partitions: Vec<PartitionRows> = tables
        .iter()
        .filter(|t| !t.partitions.is_empty())
        .map(|t| PartitionRows {
            table: t.name.clone(),
            rows: t.partitions.clone(),
        })
        .collect();
    let stats: Vec<TableStat> = tables
        .iter()
        .map(|t| TableStat {
            name: t.name.clone(),
            origin: t.origin,
            rows: t.rows,
            columns: t.cols.len(),
            partitions: t.partitions.len().max(1),
            millis: t.millis,
        })
        .collect();
    let data = StructuralData {
        tables: tables
            .into_iter()
            .map(|t| TableValues {
                name: t.name,
                columns: t
                    .cols
                    .into_iter()
                    .map(|(name, values)| ColumnValues { name, values })
                    .collect(),
            })
            .collect(),
        columns: added,
    };
    Ok(Gathered {
        model: m,
        data,
        partitions,
        stats,
    })
}

/// A row of stored or re-evaluated values, compared as the engine identifies values (numbers by
/// value, text per the model culture's stored identity).
#[derive(Hash, PartialEq, Eq)]
enum CellKey {
    Blank,
    Num(u64),
    Bool(bool),
    Text(bicraft_vertipaq::system::EquivalenceKey),
}

fn cell_key(v: &CellValue, keys: &bicraft_vertipaq::system::TextStorage) -> CellKey {
    let num = |x: f64| CellKey::Num(if x == 0.0 { 0 } else { x.to_bits() });
    match v {
        CellValue::Null => CellKey::Blank,
        CellValue::Int(i) => num(*i as f64),
        CellValue::Real(x) | CellValue::DateTime(x) => num(*x),
        CellValue::Decimal(d) => num(*d as f64 / 10_000.0),
        CellValue::Bool(b) => CellKey::Bool(*b),
        CellValue::Text(s) => CellKey::Text(keys.equivalence_key(s)),
    }
}

/// Values of data columns added to an existing import table: its `m` partitions run again
/// (bicraft's Power Query evaluator), then every row the query returns is matched to a stored
/// row through the columns the table already stores, so the new values land on the right rows
/// whatever order the engine stored them in. Refused when the query no longer returns exactly
/// the stored rows.
fn added_data_columns(
    m: &SemanticModel,
    changes: &bicraft_pbix::structural::Changes,
    editor: &DataModelEditor,
    table: &str,
    columns: &[String],
    ld: &Load<'_>,
) -> Result<Vec<NewColumnValues>, String> {
    let object = |c: &str| format!("column '{table}'[{c}]");
    let all = columns.iter().map(|c| object(c)).collect::<Vec<_>>().join(", ");
    let advice = "give the table new data with table.set_data (or table.import with replace), or make it a calculated column";
    let t = find_table(m, table).ok_or_else(|| format!("{all}: table not found"))?;
    // The query result, mapped onto the declared data columns (the new ones included).
    let mut scratch = m.clone();
    let (rows, cols, _) = literal_data(t, &mut scratch, ld)
        .map_err(|e| format!("{all}: the table's source query cannot be run again ({e}); {advice}"))?
        .ok_or_else(|| format!("{all}: the table is not an import table; {advice}"))?;
    // The stored columns: every data column the table already had (by its stored name).
    let stored_name = |name: &str| -> String {
        changes
            .renamed_columns
            .iter()
            .find(|(tn, _, new)| tn.eq_ignore_ascii_case(table) && new.eq_ignore_ascii_case(name))
            .map_or_else(|| name.to_string(), |(_, old, _)| old.clone())
    };
    let stored_table = changes
        .renamed_tables
        .iter()
        .find(|(_, new)| new.eq_ignore_ascii_case(table))
        .map_or_else(|| table.to_string(), |(old, _)| old.clone());
    let keys = crate::datasrc::relationship_keys(m);
    let mut evaluated_keys: Vec<Vec<CellKey>> = (0..rows).map(|_| Vec::new()).collect();
    let mut stored_keys: Vec<Vec<CellKey>> = Vec::new();
    let mut compared = 0usize;
    for c in t.columns.iter().filter(|c| {
        c.expression.is_none()
            && !is_row_number(c)
            && !columns.iter().any(|n| n.eq_ignore_ascii_case(&c.name))
    }) {
        let Some((_, values)) = cols.iter().find(|(n, _)| n.eq_ignore_ascii_case(&c.name)) else {
            continue;
        };
        let stored = editor
            .column_values(&stored_table, &stored_name(&c.name))
            .map_err(|e| format!("{all}: {e}"))?;
        if stored.len() != rows {
            return Err(format!(
                "{all}: the table's source query returns {rows} rows, the file stores {}; {advice}",
                stored.len()
            ));
        }
        if stored_keys.is_empty() {
            stored_keys = (0..stored.len()).map(|_| Vec::new()).collect();
        }
        for (k, v) in stored_keys.iter_mut().zip(&stored) {
            k.push(cell_key(v, &keys));
        }
        for (k, v) in evaluated_keys.iter_mut().zip(values) {
            k.push(cell_key(v, &keys));
        }
        compared = compared.saturating_add(1);
    }
    if compared == 0 {
        return Err(format!(
            "{all}: the table stores no data column to match the new rows with; {advice}"
        ));
    }
    // Stored row → query row: in order when they agree, else by matching values.
    let order: Vec<usize> = if stored_keys == evaluated_keys {
        (0..rows).collect()
    } else {
        let mut by_key: std::collections::HashMap<&[CellKey], Vec<usize>> = std::collections::HashMap::new();
        for (i, k) in evaluated_keys.iter().enumerate().rev() {
            by_key.entry(k.as_slice()).or_default().push(i);
        }
        let mut order = Vec::with_capacity(rows);
        for (r, k) in stored_keys.iter().enumerate() {
            let i = by_key.get_mut(k.as_slice()).and_then(Vec::pop).ok_or_else(|| {
                format!(
                    "{all}: the table's source query no longer returns the stored rows (stored row {} is \
                     missing from its result); {advice}",
                    r.saturating_add(1)
                )
            })?;
            order.push(i);
        }
        order
    };
    let mut out = Vec::with_capacity(columns.len());
    for name in columns {
        let (_, values) = cols
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                format!(
                    "{}: the table's source query has no such column; {advice}",
                    object(name)
                )
            })?;
        let values: Vec<CellValue> = order
            .iter()
            .map(|&i| values.get(i).cloned().unwrap_or(CellValue::Null))
            .collect();
        out.push(NewColumnValues {
            table: table.to_string(),
            column: name.clone(),
            values: crate::datasrc::stored_values(m, true, &values),
        });
    }
    Ok(out)
}

fn refused(problems: &[String]) -> String {
    format!(
        "these model changes cannot be saved into the PBIX (nothing was written): {}",
        problems.join("; ")
    )
}

fn evaluator<'e>(
    slot: &'e mut Option<StructEval>,
    m: &SemanticModel,
    data_model: &[u8],
    root: Option<&Path>,
) -> &'e mut StructEval {
    slot.get_or_insert_with(|| {
        let mut ev = StructEval::new(Some(data_model), m.clone(), MAX_ROWS);
        ev.set_data_root(root.map(Path::to_path_buf));
        ev
    })
}

/// Text values as the engine stores them (see `crate::datasrc::stored_values`).
fn stored_cols(m: &SemanticModel, imported: bool, cols: Cols) -> Cols {
    cols.into_iter()
        .map(|(name, values)| {
            let values = crate::datasrc::stored_values(m, imported, &values);
            (name, values)
        })
        .collect()
}

/// A table's loaded data: rows, data columns, rows per partition, origin.
type Loaded = (usize, Cols, Vec<u64>, &'static str);

/// [`literal_data`] with where the rows came from; a table [`Load::stored`] names reads its
/// data columns back from the stored `DataModel` (each partition keeps its rows).
fn load_table(
    t: &Table,
    m: &mut SemanticModel,
    editor: &DataModelEditor,
    ld: &Load<'_>,
) -> Result<Option<Loaded>, String> {
    if ld.is_stored(&t.name) {
        let mut cols: Cols = Vec::new();
        let mut rows = None;
        for c in t
            .columns
            .iter()
            .filter(|c| c.expression.is_none() && !is_row_number(c))
        {
            let values = editor
                .column_values(&t.name, &c.name)
                .map_err(|e| format!("its stored column '{}' cannot be read: {e}", c.name))?;
            if *rows.get_or_insert(values.len()) != values.len() {
                return Err(format!("its stored column '{}' has another row count", c.name));
            }
            cols.push((c.name.clone(), values));
        }
        let rows = match rows {
            Some(n) => n,
            None => editor
                .row_count(&t.name)
                .map_err(|e| e.to_string())?
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(0),
        };
        let partitions = match editor.partition_rows(&t.name).map_err(|e| e.to_string())? {
            Some(p) if p.len() > 1 => p,
            _ => Vec::new(),
        };
        return Ok(Some((rows, cols, partitions, "stored")));
    }
    let origin = match t.partitions.first().map(|p| p.source_type.to_ascii_lowercase()) {
        Some(s) if s == "m" => "m",
        Some(s) if s == "calculationgroup" => "literal",
        _ => "dax",
    };
    Ok(literal_data(t, m, ld)?.map(|(rows, cols, parts)| (rows, cols, parts, origin)))
}

/// Rows and data-column values of a new or replaced table from its source (an M query run by
/// bicraft's Power Query evaluator, files only under `root`, or a DAX literal table), with the
/// rows of each partition of a table of several `m` partitions; `None` for a calculated table
/// the DAX evaluator must compute. A calculated table without declared columns gets them (in
/// `m`) from its data.
fn literal_data(
    t: &Table,
    m: &mut SemanticModel,
    ld: &Load<'_>,
) -> Result<Option<(usize, Cols, Vec<u64>)>, String> {
    let root = ld.root;
    if t.partitions.len() > 1 {
        return partitioned_data(t, m, ld).map(Some);
    }
    let [p] = t.partitions.as_slice() else {
        return Err("a table needs a partition".into());
    };
    let limits = bicraft_data::Limits {
        max_rows: MAX_ROWS,
        ..bicraft_data::Limits::default()
    };
    let source = p.source_type.to_ascii_lowercase();
    if source == "calculationgroup" {
        return crate::calc_group::stored_rows(t)
            .map(|(rows, cols)| Some((rows, stored_cols(m, true, cols), Vec::new())));
    }
    if source != "m" && source != "calculated" {
        return Err(format!(
            "partition source type `{}` cannot be given data",
            p.source_type
        ));
    }
    let expr = p.expression.as_deref().unwrap_or_default();
    let literal = match source.as_str() {
        "m" => {
            let opts = ld.options(m, &t.name, expr, limits);
            bicraft_data::m::load_m_with(expr, &opts)
        }
        _ => bicraft_data::dax::load_dax(expr, &limits),
    };
    let result: Vec<(String, Option<DataType>, Vec<CellValue>)> = match literal {
        Ok(data) => data
            .into_columns()
            .into_iter()
            .map(|c| (c.name, Some(c.data_type), c.values))
            .collect(),
        Err(e @ (bicraft_data::Error::UnsupportedSource(_) | bicraft_data::Error::ExternalSource(_)))
            if source == "m" =>
        {
            return Err(format!(
                "its Power Query source cannot be run by bicraft ({}); give it data with table.set_data \
                 or table.import",
                crate::datasrc::describe(&e, root)
            ));
        }
        Err(bicraft_data::Error::UnsupportedSource(_)) => return Ok(None),
        Err(e) => return Err(crate::datasrc::describe(&e, root)),
    };
    if source == "m"
        && !t
            .columns
            .iter()
            .any(|c| c.expression.is_none() && !is_row_number(c))
    {
        return Err("the table declares no columns".into());
    }
    let (rows, cols) = calculated_table(m, &t.name, result, &[])?;
    Ok(Some((rows, stored_cols(m, source == "m", cols), Vec::new())))
}

/// [`literal_data`] of a table of several partitions: each `m` partition's query runs on its
/// own; the rows are concatenated in partition order.
fn partitioned_data(
    t: &Table,
    m: &mut SemanticModel,
    ld: &Load<'_>,
) -> Result<(usize, Cols, Vec<u64>), String> {
    let root = ld.root;
    let limits = bicraft_data::Limits {
        max_rows: MAX_ROWS,
        ..bicraft_data::Limits::default()
    };
    if !t
        .columns
        .iter()
        .any(|c| c.expression.is_none() && !is_row_number(c))
    {
        return Err("the table declares no columns".into());
    }
    let mut counts = Vec::with_capacity(t.partitions.len());
    let mut all: Cols = Vec::new();
    let mut total = 0usize;
    for p in &t.partitions {
        if !p.source_type.eq_ignore_ascii_case("m") {
            return Err(format!(
                "partition `{}`: source type `{}` cannot be given data (a table of several partitions is \
                 written from its `m` partitions; incremental-refresh policy ranges are refreshed in \
                 Power BI)",
                p.name, p.source_type
            ));
        }
        let expr = p.expression.as_deref().unwrap_or_default();
        let opts = ld.options(m, &t.name, expr, limits);
        let data = bicraft_data::m::load_m_with(expr, &opts).map_err(|e| match e {
            bicraft_data::Error::UnsupportedSource(_) | bicraft_data::Error::ExternalSource(_) => format!(
                "partition `{}`: its Power Query source cannot be run by bicraft ({}); give the table data \
                 with table.set_data or table.import",
                p.name,
                crate::datasrc::describe(&e, root)
            ),
            other => format!(
                "partition `{}`: {}",
                p.name,
                crate::datasrc::describe(&other, root)
            ),
        })?;
        let result = data
            .into_columns()
            .into_iter()
            .map(|c| (c.name, Some(c.data_type), c.values))
            .collect();
        let (rows, cols) =
            calculated_table(m, &t.name, result, &[]).map_err(|e| format!("partition `{}`: {e}", p.name))?;
        total = total.saturating_add(rows);
        if total > MAX_ROWS {
            return Err(format!("more than {MAX_ROWS} rows"));
        }
        counts.push(rows as u64);
        if all.is_empty() {
            all = cols;
        } else {
            for (name, values) in cols {
                match all.iter_mut().find(|(n, _)| *n == name) {
                    Some((_, v)) => v.extend(values),
                    None => {
                        return Err(format!(
                            "partition `{}`: column `{name}` not in the first partition",
                            p.name
                        ));
                    }
                }
            }
        }
    }
    Ok((total, stored_cols(m, true, all), counts))
}

/// Maps a table's computed data onto its declared data columns (by source column, then name),
/// or, for a calculated table without declared columns, declares them from the data.
/// `sources`: per result column, the model column it comes from (`Table[Column]`, see
/// the evaluator's `EvalTable`), when the evaluator knows it.
fn calculated_table(
    m: &mut SemanticModel,
    name: &str,
    result: Vec<(String, Option<DataType>, Vec<CellValue>)>,
    sources: &[Option<String>],
) -> Result<(usize, Cols), String> {
    let rows = result.first().map_or(0, |c| c.2.len());
    if rows > MAX_ROWS {
        return Err(format!("more than {MAX_ROWS} rows"));
    }
    let t = find_table(m, name).ok_or_else(|| format!("table '{name}' is not in the model"))?;
    let mut data_cols: Vec<Column> = t
        .columns
        .iter()
        .filter(|c| c.expression.is_none() && !is_row_number(c))
        .cloned()
        .collect();
    if data_cols.is_empty() {
        // Columns from the result, as Desktop infers them.
        let mut columns = Vec::new();
        let mut cols = Vec::new();
        for (i, (cname, dt, values)) in result.into_iter().enumerate() {
            let (values, data_type) = coerce(values, dt.unwrap_or(DataType::Unknown))?;
            // As the engine writes it: the lineage column, else the column's own name.
            let source = sources
                .get(i)
                .cloned()
                .flatten()
                .unwrap_or_else(|| format!("[{cname}]"));
            let mut extra = Map::new();
            extra.insert("type".into(), json!("calculatedTableColumn"));
            extra.insert("isNameInferred".into(), json!(true));
            extra.insert("isDataTypeInferred".into(), json!(true));
            columns.push(Column {
                name: cname.clone(),
                data_type,
                source_column: Some(source),
                lineage_tag: Some(new_guid()),
                summarize_by: Some(default_summarize(data_type).to_string()),
                extra,
                ..Default::default()
            });
            cols.push((cname, values));
        }
        if let Some(target) = find_table_mut(m, name) {
            let calculated: Vec<Column> = target
                .columns
                .drain(..)
                .filter(|c| c.expression.is_some())
                .collect();
            target.columns = columns;
            target.columns.extend(calculated);
        }
        return Ok((rows, cols));
    }
    // A declared column whose `sourceColumn` is a result column's lineage (`Table[Column]`)
    // reads that column, whatever its name (`Table_Column` for an unnamed `SELECTCOLUMNS`).
    for c in &mut data_cols {
        let found = c.source_column.as_deref().and_then(|src| {
            sources
                .iter()
                .position(|s| s.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(src)))
        });
        if let Some((cname, _, _)) = found.and_then(|i| result.get(i)) {
            c.source_column = Some(format!("[{cname}]"));
        }
    }
    let table_data = TableData::new(
        result
            .into_iter()
            .map(|(cname, dt, values)| {
                let data_type = dt.unwrap_or_else(|| infer_type(&values));
                bicraft_data::ColumnData {
                    name: cname,
                    data_type,
                    values: values.into_iter().map(|v| normalize(v, data_type)).collect(),
                }
            })
            .collect(),
    )
    .map_err(|e| e.to_string())?;
    let conformed = bicraft_data::conform(table_data, &data_cols).map_err(|e| e.to_string())?;
    let rows = conformed.row_count();
    let cols = conformed
        .into_columns()
        .into_iter()
        .map(|c| (c.name, c.values))
        .collect();
    Ok((rows, cols))
}

fn default_summarize(t: DataType) -> &'static str {
    match t {
        DataType::Int64 | DataType::Double | DataType::Decimal => "sum",
        _ => "none",
    }
}

/// The type of evaluated values without lineage: integers, numbers, text, …; blank-only → whole
/// number (as the engine types `ADDCOLUMNS(T, "X", BLANK())`, Desktop 2.158).
fn infer_type(values: &[CellValue]) -> DataType {
    let mut t: Option<DataType> = None;
    for v in values {
        let this = match v {
            CellValue::Null => continue,
            CellValue::Int(_) => DataType::Int64,
            CellValue::Real(x) if x.fract() == 0.0 && x.abs() < 9.0e15 => DataType::Int64,
            CellValue::Real(_) => DataType::Double,
            CellValue::Decimal(_) => DataType::Decimal,
            CellValue::DateTime(_) => DataType::DateTime,
            CellValue::Bool(_) => DataType::Boolean,
            CellValue::Text(_) => DataType::String,
        };
        t = Some(match (t, this) {
            (None, x) => x,
            (Some(a), b) if a == b => a,
            (Some(DataType::Int64 | DataType::Double), DataType::Int64 | DataType::Double) => {
                DataType::Double
            }
            _ => DataType::Variant,
        });
    }
    t.unwrap_or(DataType::Int64)
}

/// A value in the representation of `t` when it converts exactly; unchanged otherwise.
fn normalize(v: CellValue, t: DataType) -> CellValue {
    match (t, v) {
        (DataType::Int64, CellValue::Real(x)) if x.fract() == 0.0 && x.abs() < 9.0e15 => {
            CellValue::Int(x as i64)
        }
        (DataType::Double, CellValue::Int(n)) => CellValue::Real(n as f64),
        (DataType::Double, CellValue::Decimal(n)) => CellValue::Real(n as f64 / 10_000.0),
        (DataType::Decimal, CellValue::Int(n)) => n
            .checked_mul(10_000)
            .map_or(CellValue::Int(n), CellValue::Decimal),
        (DataType::Decimal, CellValue::Real(x)) if (x * 10_000.0).abs() < 9.0e18 => {
            CellValue::Decimal((x * 10_000.0).round() as i64)
        }
        (DataType::DateTime, CellValue::Real(x)) => CellValue::DateTime(x),
        (DataType::Boolean, CellValue::Int(n @ (0 | 1))) => CellValue::Bool(n == 1),
        (_, v) => v,
    }
}

/// Evaluated values converted to the column type (`Unknown`: inferred). Values that do not
/// convert exactly are an error.
pub(crate) fn coerce(values: Vec<CellValue>, t: DataType) -> Result<(Vec<CellValue>, DataType), String> {
    let t = if t == DataType::Unknown {
        infer_type(&values)
    } else {
        t
    };
    if matches!(t, DataType::Variant | DataType::Binary | DataType::Unknown) {
        return Err(format!(
            "values of mixed or unsupported types ({}) cannot be stored",
            t.as_tmsl().unwrap_or("unknown")
        ));
    }
    let mut out = Vec::with_capacity(values.len());
    for (i, v) in values.into_iter().enumerate() {
        let v = normalize(v, t);
        let ok = matches!(
            (t, &v),
            (_, CellValue::Null)
                | (DataType::Int64, CellValue::Int(_))
                | (DataType::Double, CellValue::Real(_))
                | (DataType::Decimal, CellValue::Decimal(_))
                | (DataType::DateTime, CellValue::DateTime(_))
                | (DataType::Boolean, CellValue::Bool(_))
                | (DataType::String, CellValue::Text(_))
        );
        if !ok {
            return Err(format!(
                "row {}: the value {v:?} does not fit the column type {}",
                i.saturating_add(1),
                t.as_tmsl().unwrap_or("unknown")
            ));
        }
        out.push(v);
    }
    Ok((out, t))
}

// ---------------------------------------------------------------------------------------------
// table.import / table.set_data
// ---------------------------------------------------------------------------------------------

fn parse_type(name: &str) -> Result<DataType, String> {
    let t = match name.to_ascii_lowercase().as_str() {
        "text" | "string" => DataType::String,
        "int" | "integer" | "whole" => DataType::Int64,
        "number" => DataType::Double,
        "currency" => DataType::Decimal,
        "date" | "datetime" => DataType::DateTime,
        "bool" | "logical" => DataType::Boolean,
        other => DataType::from_tmsl(other),
    };
    if matches!(t, DataType::Unknown | DataType::Binary | DataType::Variant) {
        return Err(format!(
            "unknown column type `{name}` (expected string, int64, double, decimal, dateTime or boolean)"
        ));
    }
    Ok(t)
}

/// The data a `table.import` / `table.set_data` call describes, and the partition that holds it.
/// An `m` query runs in bicraft's Power Query evaluator with the model's culture and shared
/// queries, reading files only under `root`.
fn import_source(
    p: &Value,
    table: &str,
    model: &SemanticModel,
    root: Option<&Path>,
    stored: Option<&[u8]>,
    csv_file: Option<&Path>,
    connectors: bicraft_data::connect::Connectors,
) -> Result<(TableData, Partition), String> {
    let given: Vec<&str> = ["csv", "m", "dax"]
        .into_iter()
        .filter(|k| p.get(*k).is_some_and(|v| !v.is_null()))
        .collect();
    let [kind] = given.as_slice() else {
        return Err("pass exactly one of `csv` (a file path or CSV text), `m` (a Power Query expression) or `dax` (a DAX table expression)".into());
    };
    let limits = bicraft_data::Limits {
        max_rows: MAX_ROWS,
        ..bicraft_data::Limits::default()
    };
    let partition = |source_type: &str, expression: String| Partition {
        name: table.to_string(),
        mode: Some("import".into()),
        source_type: source_type.into(),
        expression: Some(expression),
        ..Default::default()
    };
    match *kind {
        "csv" => {
            let text = str_param(p, "csv")?;
            let bytes = if let Some(file) = csv_file {
                let len = std::fs::metadata(file)
                    .map(|m| m.len())
                    .map_err(|e| format!("{text}: {e}"))?;
                if len > MAX_CSV_BYTES {
                    return Err(format!(
                        "{text}: {len} bytes, over the {MAX_CSV_BYTES}-byte limit of a CSV stored as a #table literal; load it with `m` (Csv.Document(File.Contents(...)) under `data_root`), which streams it"
                    ));
                }
                std::fs::read(file).map_err(|e| format!("{text}: {e}"))?
            } else {
                text.as_bytes().to_vec()
            };
            let mut opts = CsvOptions::with_culture(Culture::from_name(opt_str(p, "culture")?.unwrap_or("")));
            opts.limits = limits;
            if let Some(d) = opt_str(p, "delimiter")? {
                let mut chars = d.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => opts.delimiter = Some(c),
                    _ => return Err("`delimiter` must be one character".into()),
                }
            }
            match p.get("types") {
                None | Some(Value::Null) => {}
                Some(Value::Object(o)) => {
                    for (k, v) in o {
                        let t = v.as_str().ok_or_else(|| format!("types.{k} must be a string"))?;
                        opts.column_types.push((k.clone(), parse_type(t)?));
                    }
                }
                Some(_) => return Err("`types` must be an object {column: type}".into()),
            }
            let data = read_csv(&bytes, &opts).map_err(|e| format!("csv: {e}"))?;
            let m = m_literal(&data)?;
            // The literal must load back as the same data.
            let back =
                bicraft_data::m::load_m(&m, &limits).map_err(|e| format!("internal check failed: {e}"))?;
            if back != data {
                return Err(
                    "internal check failed: the generated #table literal does not load back as the CSV"
                        .into(),
                );
            }
            // The literal's column types (`date` for date-only columns) come with it.
            Ok((back, partition("m", m)))
        }
        "m" => {
            let m = str_param(p, "m")?.to_string();
            let mut opts = crate::datasrc::source_options(Some(model), table, &m, root, limits);
            let data = if connectors.is_empty() {
                bicraft_data::m::load_m_with(&m, &opts)
            } else {
                // an allowed connector: the columns only (a row limit of 0, which the connector
                // folds into its query); the rows come with model.refresh
                opts.connectors = connectors;
                let head =
                    format!("let\n    __bicraft_query =\n{m}\nin\n    Table.FirstN(__bicraft_query, 0)");
                bicraft_data::m::load_m_with(&head, &opts)
            }
            .map_err(|e| format!("m: {}", crate::datasrc::describe(&e, root)))?;
            Ok((data, partition("m", m)))
        }
        _ => {
            let dax = str_param(p, "dax")?.to_string();
            let data = match bicraft_data::dax::load_dax(&dax, &limits) {
                Ok(data) => data,
                // Beyond literal tables: the DAX evaluator over the model and its data.
                Err(bicraft_data::Error::UnsupportedSource(_)) => {
                    eval_dax_table(stored, model, &dax, root).map_err(|e| format!("dax: {e}"))?
                }
                Err(e) => return Err(format!("dax: {e}")),
            };
            Ok((data, partition("calculated", dax)))
        }
    }
}

/// A calculated table expression evaluated by the DAX evaluator in an empty filter context,
/// over `stored` (the open file's data) and the tables bicraft can load (M queries, literal
/// tables); column types come from lineage, else from the values.
fn eval_dax_table(
    stored: Option<&[u8]>,
    model: &SemanticModel,
    dax: &str,
    root: Option<&Path>,
) -> Result<TableData, String> {
    let mut ev = StructEval::new(stored, model.clone(), MAX_ROWS);
    ev.set_data_root(root.map(Path::to_path_buf));
    let result = ev.table(dax)?;
    let mut columns = Vec::with_capacity(result.columns.len());
    for (name, data_type, values) in result.columns {
        let (values, data_type) = coerce(values, data_type.unwrap_or(DataType::Unknown))
            .map_err(|e| format!("column `{name}`: {e}"))?;
        columns.push(bicraft_data::ColumnData {
            name,
            data_type,
            values: crate::datasrc::stored_values(model, false, &values),
        });
    }
    TableData::new(columns).map_err(|e| e.to_string())
}

/// `table.import`: a new import table with data, or (with `replace`) new data for an existing one.
pub(crate) fn table_import(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "name",
            "csv",
            "m",
            "dax",
            "culture",
            "delimiter",
            "types",
            "replace",
            "hidden",
            "description",
            "data_root",
            "allow_connectors",
        ],
    )?;
    let replace = opt_bool(p, "replace")?.unwrap_or(false);
    import(s, p, replace, false)
}

/// `table.set_data`: new data for an existing table.
pub(crate) fn table_set_data(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "name",
            "csv",
            "m",
            "dax",
            "culture",
            "delimiter",
            "types",
            "data_root",
        ],
    )?;
    import(s, p, true, true)
}

pub(crate) fn import(s: &mut Session, p: &Value, replace: bool, must_exist: bool) -> Result<Value, String> {
    let name = str_param(p, "name")?.to_string();
    check_name("table", &name)?;
    let given_root = crate::datasrc::data_root_param(p)?;
    let root = crate::datasrc::effective_root(&given_root, s.data_root.as_deref());
    let allow_list = crate::refresh::string_list(p, "allow_connectors")?.unwrap_or_default();
    let (connectors, _) = crate::refresh::connectors(s, &allow_list, false)?;
    let (data, partition) = {
        // The open file's data, for a DAX expression over the model.
        let backup = if p.get("dax").is_some_and(|v| !v.is_null()) {
            crate::preview::stored_backup(s)
        } else {
            None
        };
        // a `csv` naming a file: resolved under the workspace root in server mode
        let csv_file = match p.get("csv").and_then(Value::as_str) {
            Some(text) => crate::sandbox::csv_file(s, text)?,
            None => None,
        };
        import_source(
            p,
            &name,
            model(s)?,
            root.as_deref(),
            backup.as_deref(),
            csv_file.as_deref(),
            connectors,
        )?
    };
    let calculated = partition.source_type == "calculated";
    let mut m = model(s)?.clone();
    let existing = m.tables.iter().position(|t| t.name.eq_ignore_ascii_case(&name));
    let mut warnings: Vec<String> = Vec::new();
    let replaced = match existing {
        Some(_) if !replace => {
            return Err(format!(
                "a table named `{name}` already exists; pass \"replace\": true (or use table.set_data) to give it new data"
            ));
        }
        None if must_exist => return Err(format!("no table named `{name}`")),
        Some(i) => {
            let t = m.tables.get_mut(i).ok_or("internal error")?;
            let mut columns = Vec::new();
            for (ci, c) in data.columns().iter().enumerate() {
                let old = t.columns.iter().find(|x| {
                    x.expression.is_none() && !is_row_number(x) && x.name.eq_ignore_ascii_case(&c.name)
                });
                let mut col = match old {
                    Some(o) => o.clone(),
                    None => new_column(&c.name, c.data_type, calculated, data.date_kind(ci)),
                };
                if col.data_type != c.data_type {
                    col.data_type = c.data_type;
                    col.summarize_by = Some(default_summarize(c.data_type).to_string());
                }
                col.name.clone_from(&c.name);
                if calculated {
                    col.source_column = Some(format!("[{}]", c.name));
                    col.extra.insert("type".into(), json!("calculatedTableColumn"));
                } else {
                    col.source_column = Some(c.name.clone());
                    col.extra.remove("type");
                    col.extra.remove("isNameInferred");
                    col.extra.remove("isDataTypeInferred");
                }
                columns.push(col);
            }
            let dropped: Vec<String> = t
                .columns
                .iter()
                .filter(|x| x.expression.is_none() && !is_row_number(x))
                .filter(|x| !columns.iter().any(|c| c.name.eq_ignore_ascii_case(&x.name)))
                .map(|x| x.name.clone())
                .collect();
            if let Some(r) = m.relationships.iter().find(|r| {
                (r.from_table.eq_ignore_ascii_case(&name)
                    && dropped.iter().any(|d| d.eq_ignore_ascii_case(&r.from_column)))
                    || (r.to_table.eq_ignore_ascii_case(&name)
                        && dropped.iter().any(|d| d.eq_ignore_ascii_case(&r.to_column)))
            }) {
                return Err(format!(
                    "the new data has no column used by relationship `{}`; remove the relationship first",
                    r.name
                ));
            }
            if !dropped.is_empty() {
                warnings.push(format!(
                    "columns without new data were removed: {}",
                    dropped.join(", ")
                ));
            }
            let t = m.tables.get_mut(i).ok_or("internal error")?;
            let keep: Vec<Column> = t.columns.drain(..).filter(|c| c.expression.is_some()).collect();
            t.columns = columns;
            t.columns.extend(keep);
            t.partitions = vec![partition];
            true
        }
        None => {
            let mut t = Table {
                name: name.clone(),
                lineage_tag: Some(new_guid()),
                is_hidden: opt_bool(p, "hidden")?.unwrap_or(false),
                description: opt_str(p, "description")?.map(str::to_string),
                partitions: vec![partition],
                ..Default::default()
            };
            for (ci, c) in data.columns().iter().enumerate() {
                t.columns
                    .push(new_column(&c.name, c.data_type, calculated, data.date_kind(ci)));
            }
            crate::desktop_defaults::new_table(&mut t);
            m.tables.push(t);
            false
        }
    };
    let columns: Vec<Value> = data
        .columns()
        .iter()
        .map(|c| json!({"name": c.name, "data_type": c.data_type.as_tmsl()}))
        .collect();
    s.model = Some(m);
    s.model_error = None;
    s.dirty = true;
    if given_root.is_some() {
        s.data_root.clone_from(&root);
    }
    Ok(json!({
        "table": name,
        "rows": data.row_count(),
        "columns": columns,
        "source_type": if calculated { "calculated" } else { "m" },
        "replaced": replaced,
        "warnings": warnings,
        "data_root": root.map(|r| r.display().to_string()),
    }))
}

pub(crate) fn new_column(
    name: &str,
    data_type: DataType,
    calculated: bool,
    date_kind: Option<bicraft_data::DateKind>,
) -> Column {
    let mut extra = Map::new();
    if calculated {
        extra.insert("type".into(), json!("calculatedTableColumn"));
        extra.insert("isNameInferred".into(), json!(true));
        extra.insert("isDataTypeInferred".into(), json!(true));
    }
    let mut c = Column {
        name: name.to_string(),
        data_type,
        source_column: Some(if calculated {
            format!("[{name}]")
        } else {
            name.to_string()
        }),
        lineage_tag: Some(new_guid()),
        summarize_by: Some(default_summarize(data_type).to_string()),
        extra,
        ..Default::default()
    };
    crate::desktop_defaults::new_column(&mut c, false, date_kind);
    c
}

// ---------------------------------------------------------------------------------------------
// M #table literal writer
// ---------------------------------------------------------------------------------------------

fn m_name(name: &str) -> String {
    format!("#\"{}\"", m_text_body(name))
}

fn m_text_body(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\"\""),
            '#' => out.push_str("#(#)"),
            '\r' => out.push_str("#(cr)"),
            '\n' => out.push_str("#(lf)"),
            '\t' => out.push_str("#(tab)"),
            c if c.is_control() => out.push_str(&format!("#({:04X})", u32::from(c))),
            c => out.push(c),
        }
    }
    out
}

/// OLE Automation day → (year, month, day).
fn civil(days: i64) -> (i64, i64, i64) {
    bicraft_locale::calendar::civil_from_serial(days)
}

fn m_value(v: &CellValue, date_only: bool) -> Result<String, String> {
    Ok(match v {
        CellValue::Null => "null".into(),
        CellValue::Int(n) => n.to_string(),
        CellValue::Real(x) if x.is_nan() => "#nan".into(),
        CellValue::Real(x) if x.is_infinite() => if *x > 0.0 { "#infinity" } else { "-#infinity" }.into(),
        CellValue::Real(x) => format!("{x:?}"),
        CellValue::Decimal(n) => {
            let a = n.unsigned_abs();
            let frac = a % 10_000;
            let body = if frac == 0 {
                (a / 10_000).to_string()
            } else {
                format!("{}.{}", a / 10_000, format!("{frac:04}").trim_end_matches('0'))
            };
            if *n < 0 { format!("-{body}") } else { body }
        }
        CellValue::Bool(b) => b.to_string(),
        CellValue::Text(s) => format!("\"{}\"", m_text_body(s)),
        CellValue::DateTime(x) => {
            if !x.is_finite() || x.abs() > 3.0e6 {
                return Err(format!("date value {x} is out of range"));
            }
            let day = x.floor();
            let (y, mo, d) = civil(day as i64);
            if date_only {
                format!("#date({y}, {mo}, {d})")
            } else {
                let ms = ((x - day) * 86_400_000.0).round() as i64;
                let (h, mi, sec, milli) = (ms / 3_600_000, (ms / 60_000) % 60, (ms / 1000) % 60, ms % 1000);
                if milli == 0 {
                    format!("#datetime({y}, {mo}, {d}, {h}, {mi}, {sec})")
                } else {
                    format!("#datetime({y}, {mo}, {d}, {h}, {mi}, {sec}.{milli:03})")
                }
            }
        }
    })
}

/// A Power Query `#table` literal holding `data` (what `bicraft_data::m::load_m` reads back).
pub(crate) fn m_literal(data: &TableData) -> Result<String, String> {
    let mut header = Vec::new();
    let mut date_only = Vec::new();
    for c in data.columns() {
        let only = c.values.iter().all(|v| match v {
            CellValue::DateTime(x) => x.fract() == 0.0,
            _ => true,
        });
        let t = match c.data_type {
            DataType::String => "text",
            DataType::Int64 => "Int64.Type",
            DataType::Double => "number",
            DataType::Decimal => "Currency.Type",
            DataType::DateTime if only => "date",
            DataType::DateTime => "datetime",
            DataType::Boolean => "logical",
            other => return Err(format!("column `{}`: type {other:?} cannot be imported", c.name)),
        };
        header.push(format!("{} = {t}", m_name(&c.name)));
        date_only.push(c.data_type == DataType::DateTime && only);
    }
    let mut rows = Vec::with_capacity(data.row_count());
    for i in 0..data.row_count() {
        let mut cells = Vec::with_capacity(header.len());
        for (j, c) in data.columns().iter().enumerate() {
            let v = c.values.get(i).unwrap_or(&CellValue::Null);
            cells.push(m_value(v, date_only.get(j).copied().unwrap_or(false))?);
        }
        rows.push(format!("{{{}}}", cells.join(", ")));
    }
    Ok(format!(
        "let\n    Source = #table(type table [{}], {{\n        {}\n    }})\nin\n    Source",
        header.join(", "),
        rows.join(",\n        ")
    ))
}

#[cfg(test)]
#[path = "pbix_struct_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "pbix_struct_leftover_tests.rs"]
mod leftover_tests;
