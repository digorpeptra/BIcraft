//! A PBIX built without an existing `DataModel` (M5): `file.save_as` / `file.convert` /
//! `bicraft-cli convert` to `.pbix` from a new model (`model.new`, `project.new`), a `model.bim`, a
//! PBIT, or a PBIP project without `.pbi/cache.abf`.
//!
//! The model-level part of the model becomes the image of a brand-new database
//! (`bicraft_pbix::fresh`); then the structural save of `crate::pbix_struct` adds every table
//! with its data, exactly as when tables are added to an existing PBIX: M partitions run in
//! bicraft's Power Query evaluator (files only under the session's `data_root`), DAX literal
//! and calculated tables and calculated columns in the DAX evaluator. A table whose source
//! bicraft cannot run (an external connector: SQL, web, …) is refused with its name, unless
//! `empty_tables` asks for it to be written without rows (Desktop fills it on the next refresh).
//! See `docs/formats/datamodel-fresh.md`.

use std::path::Path;

use bicraft_model::{Column, DataType, SemanticModel, Table};
use bicraft_opc::Package;

/// The `DataModel` part of `model`, built from scratch. `root`: the folder Power Query sources
/// may read. `empty_tables`: write the tables whose source bicraft cannot run without rows
/// instead of refusing them; their names are returned.
pub(crate) fn data_model(
    model: &SemanticModel,
    root: Option<&Path>,
    empty_tables: bool,
) -> Result<(Vec<u8>, Vec<String>), String> {
    let model = bicraft_pbix::fresh::with_model_name(model);
    // a DirectQuery table holds no imported rows: it is always written without them
    let (write, emptied) = without_rows(&model, root, empty_tables);
    let now = filetime_now()?;
    let empty = bicraft_pbix::fresh::empty_data_model_at(&model, now)
        .map_err(|e| format!("the new DataModel cannot be written: {e}"))?;
    let mut package = Package::default();
    package.put(bicraft_pbix::DATA_MODEL, empty, true);
    let built = crate::pbix_struct::write_model(&package, &write, root)?;
    let mut part = built
        .get(bicraft_pbix::DATA_MODEL)
        .map(|p| p.data.clone())
        .ok_or("internal error: the built package has no DataModel")?;
    if !emptied.is_empty() {
        part = restore_queries(&part, &model, &emptied)?;
    }
    Ok((part, emptied))
}

/// Windows FILETIME ticks now.
fn filetime_now() -> Result<i64, String> {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "the system clock is before 1970".to_string())?;
    i64::try_from(since.as_nanos() / 100)
        .ok()
        .and_then(|t| t.checked_add(116_444_736_000_000_000))
        .ok_or_else(|| "clock overflow".to_string())
}

/// Whether the M partition of `t` cannot be run by bicraft (an external connector, or a source
/// it does not support).
fn is_external(model: &SemanticModel, t: &Table, root: Option<&Path>) -> bool {
    let [p] = t.partitions.as_slice() else {
        return false;
    };
    if !p.source_type.eq_ignore_ascii_case("m") {
        return false;
    }
    let expr = p.expression.as_deref().unwrap_or_default();
    // Only find out whether the source runs: one row is enough.
    let limits = bicraft_data::Limits {
        max_rows: 1,
        ..bicraft_data::Limits::default()
    };
    let opts = crate::datasrc::source_options(Some(model), &t.name, expr, root, limits);
    matches!(
        bicraft_data::m::load_m_with(expr, &opts),
        Err(bicraft_data::Error::UnsupportedSource(_) | bicraft_data::Error::ExternalSource(_))
    )
}

/// The M type name of a column for an empty `#table`.
fn m_type(t: DataType) -> &'static str {
    match t {
        DataType::Int64 => "Int64.Type",
        DataType::Double => "number",
        DataType::Decimal => "Currency.Type",
        DataType::DateTime => "datetime",
        DataType::Boolean => "logical",
        DataType::Binary => "binary",
        _ => "text",
    }
}

fn m_name(name: &str) -> String {
    format!("#\"{}\"", name.replace('"', "\"\""))
}

/// An empty, typed `#table` with the data columns of `t` (by source column).
fn empty_table_query(t: &Table) -> String {
    let cols: Vec<String> = t
        .columns
        .iter()
        .filter(|c: &&Column| c.expression.is_none())
        .map(|c| {
            let name = c.source_column.as_deref().unwrap_or(&c.name);
            format!("{} = {}", m_name(name), m_type(c.data_type))
        })
        .collect();
    format!("#table(type table [{}], {{}})", cols.join(", "))
}

/// `model` with every DirectQuery table, and with `external` every table whose source cannot
/// run, given an empty literal source (so it is written without rows), and those tables' names.
fn without_rows(model: &SemanticModel, root: Option<&Path>, external: bool) -> (SemanticModel, Vec<String>) {
    let mut m = model.clone();
    let mut names = Vec::new();
    for t in &mut m.tables {
        if crate::storage_mode::is_direct_query(model, t) || (external && is_external(model, t, root)) {
            let query = empty_table_query(t);
            if let Some(p) = t.partitions.first_mut() {
                p.expression = Some(query);
            }
            names.push(t.name.clone());
        }
    }
    (m, names)
}

/// Puts the original M queries of `tables` back into the written `DataModel` (they were
/// written with an empty literal source).
fn restore_queries(part: &[u8], model: &SemanticModel, tables: &[String]) -> Result<Vec<u8>, String> {
    let queries: Vec<(String, String)> = tables
        .iter()
        .filter_map(|name| {
            let t = model.tables.iter().find(|t| &t.name == name)?;
            let q = t.partitions.first()?.expression.clone()?;
            Some((name.clone(), q))
        })
        .collect();
    bicraft_pbix::fresh::set_partition_queries(part, &queries)
        .map_err(|e| format!("the source queries of the empty tables cannot be written: {e}"))
}
