//! `file.open` of an Excel workbook with a Power Pivot data model (`.xlsx`, `.xlsm`; M11.33).
//!
//! `bicraft-powerpivot` reads the model and the values of every table from
//! `xl/model/item.data`; they become a PBIX held in memory: a new `DataModel` built
//! with those values (`bicraft_pbix::fresh`, as a new model saved as a PBIX), a blank one-page
//! report, the parts every PBIX has. The session then works as on an opened PBIX whose path is
//! the workbook's: the model, the data previews, `file.save_as` / `file.convert` to `.pbix`,
//! `.pbip` (with `.pbi/cache.abf`), `.pbit` or `.bim`. The workbook itself is never written:
//! `file.save` asks for `file.save_as`. See `docs/formats/powerpivot.md`.

use std::path::Path;

use bicraft_model::SemanticModel;
use bicraft_opc::Package;
use bicraft_pbix::structural::{ColumnValues, StructuralData, TableValues};
use bicraft_report::Report;
use serde_json::{Value, json};

/// The extensions `file.open` imports as Excel workbooks.
pub(crate) const EXTENSIONS: [&str; 2] = ["xlsx", "xlsm"];

/// Whether `path` names an Excel workbook (by extension).
pub(crate) fn is_workbook(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// The error of `file.save` on an imported workbook.
pub(crate) fn save_refused(path: &Path) -> String {
    format!(
        "{} is an Excel workbook: bicraft imports its Power Pivot model but never writes the workbook; save the \
         model with file.save_as to a .pbix or .pbip path",
        path.display()
    )
}

pub(crate) struct Imported {
    pub package: Package,
    pub model: SemanticModel,
    pub report: Report,
    /// `{from, compatibility_level, tables, rows, warnings}` for `file.open`'s result.
    pub info: Value,
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

pub(crate) fn open(path: &Path) -> Result<Imported, String> {
    // the file's name only: callers (the app, `file.convert`) already name the full path
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let at = |e: &dyn std::fmt::Display| format!("{name}: {e}");
    let size = std::fs::metadata(path).map_err(|e| at(&e))?.len();
    if size > bicraft_opc::MAX_PACKAGE_BYTES {
        return Err(at(&format!("{size} bytes, more than a workbook bicraft imports")));
    }
    let bytes = std::fs::read(path).map_err(|e| at(&e))?;
    let wb = bicraft_powerpivot::read_workbook(&bytes, &bicraft_powerpivot::Limits::default())
        .map_err(|e| at(&e))?;
    drop(bytes);
    let mut model = wb.model;
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        model.name = stem.to_string();
    }
    let rows: u64 = wb.tables.iter().map(|t| t.rows() as u64).sum();
    let data = StructuralData {
        tables: wb
            .tables
            .into_iter()
            .map(|t| TableValues {
                name: t.name,
                columns: t
                    .columns
                    .into_iter()
                    .map(|(name, values)| ColumnValues { name, values })
                    .collect(),
            })
            .collect(),
        columns: Vec::new(),
    };
    let part = bicraft_pbix::fresh::build_data_model_at(&model, &data, filetime_now()?).map_err(|e| {
        at(&format!(
            "the imported model cannot be written as a Power BI model: {e}"
        ))
    })?;
    let report = bicraft_pbip::blank_report(&bicraft_pbip::ProjectIds::random().page_name);
    let docs = bicraft_pbir::write_documents(&report);
    let package = bicraft_pbixwriter::write_pbix(
        part,
        bicraft_pbixwriter::ReportParts::Pbir(&docs),
        &bicraft_pbixwriter::Extras::default(),
    )
    .map_err(|e| at(&e))?;
    // the session holds what the package holds, as after opening a PBIX
    let model = bicraft_pbix::read_model(&package).map_err(|e| at(&e))?;
    let report = bicraft_pbix::read_report(&package).map_err(|e| at(&e))?;
    let info = json!({
        "from": "excel-power-pivot",
        "compatibility_level": wb.compatibility_level,
        "tables": model.tables.len(),
        "rows": rows,
        "warnings": wb.warnings,
        "note": "imported into a new Power BI model held in memory: save it with file.save_as to a .pbix or .pbip \
                 path; each table's query keeps its Power Pivot source as a comment and must be replaced before a \
                 refresh",
    });
    Ok(Imported {
        package,
        model,
        report,
        info,
    })
}

#[cfg(test)]
#[path = "workbook_import_tests.rs"]
mod tests;
