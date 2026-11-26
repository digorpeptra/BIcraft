//! `model.refresh` on an open PBIX (ROADMAP M11.18, gap G50): bicraft refreshes the file's
//! import tables itself and writes the new rows into its `DataModel`, as Power BI Desktop's
//! Refresh does, so Desktop opens the file with the new data.
//!
//! - **Sources.** The stored model's `m` partitions run in bicraft's Power Query evaluator with
//!   the same rules as a PBIP refresh (`refresh.rs`): files only under `data_root`, connectors only
//!   when `allow_connectors` names them, the refresh's start time as the clock.
//! - **What is refreshed.** Every import and calculated table, or the `tables` named and what
//!   depends on them: calculated tables that read them are evaluated again, and a table whose
//!   calculated columns read them keeps its stored data columns and gets those columns
//!   recomputed (`origin: "stored"`). Other tables keep their storage files byte for byte.
//! - **Writing.** The structural writer replaces each refreshed table (`bicraft_pbix::structural`,
//!   [`DataModelEditor::write_refreshed`]): VertiPaq encoding, `H$`/`R$` system tables,
//!   partitions `State` 1 and `RefreshedTime` = `ModifiedTime` = the refresh time,
//!   `LASTCUBEDATACHANGE` raised (`docs/formats/metadata-sqlitedb.md`, "Refresh state").
//! - **Safety.** Nothing is written unless every table loads. The package is written to a
//!   temporary sibling, reopened with bicraft's reader (model, structure and row counts
//!   checked), then renamed over the file; any error leaves the file untouched.
//! - **Memory.** The rows of the tables being refreshed are reserved against the process'
//!   memory limit (`BICRAFT_MEMORY_LIMIT`) as each table loads: over the limit the refresh is
//!   refused before anything is written. The other tables are never decoded.
//!
//! See `docs/mcp.md`, *Refreshing data with BIcraft*, and `docs/formats/datamodel-fresh.md`,
//! *Refresh of an existing PBIX*.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Instant;

use bicraft_data::connect::Connectors;
use bicraft_dax::deps::{ObjectRef, dependencies};
use bicraft_model::{SemanticModel, Table};
use bicraft_pbix::structural::DataModelEditor;
use bicraft_vertipaq::CellValue;
use serde_json::{Value, json};

use crate::pbix_struct::{Cols, Load};
use crate::{Session, Source};

/// Days from 1601-01-01 (FILETIME) to 1899-12-30 (the refresh clock's epoch).
const FILETIME_TO_OLE_DAYS: i64 = 109_205;

/// What `model.refresh` hands over for a PBIX.
pub(crate) struct Request {
    pub wanted: Option<Vec<String>>,
    pub root: Option<PathBuf>,
    pub connectors: Connectors,
    pub connector_names: Vec<String>,
    pub dbx: Option<std::sync::Arc<bicraft_databricks::DatabricksConnector>>,
    pub clock: i64,
    /// `utc_offset_minutes`: local time minus UTC (`None`: the process's local clock).
    pub offset_minutes: Option<i32>,
    pub refreshed_at: String,
    pub force: bool,
    pub backup: bool,
}

/// How a table takes part in the refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Its `m` partitions run again.
    Import,
    /// A calculated table, evaluated again.
    Calculated,
}

fn is(source: &str, kind: &str) -> bool {
    source.eq_ignore_ascii_case(kind)
}

/// How `t` can be refreshed, or why not.
fn kind_of(t: &Table) -> Result<Kind, String> {
    if t.partitions.is_empty() {
        return Err("no partition".into());
    }
    if t.partitions.iter().all(|p| is(&p.source_type, "calculated")) {
        return Ok(Kind::Calculated);
    }
    if t.partitions
        .iter()
        .any(|p| is(&p.source_type, "calculationGroup"))
    {
        return Err("a calculation group holds no imported rows".into());
    }
    if let Some(p) = t.partitions.iter().find(|p| !is(&p.source_type, "m")) {
        return Err(format!(
            "partition source `{}` is not refreshed by bicraft",
            p.source_type
        ));
    }
    if t.partitions.iter().any(|p| {
        p.mode
            .as_deref()
            .is_some_and(|m| m.eq_ignore_ascii_case("directQuery"))
    }) {
        return Err("DirectQuery partitions hold no imported rows".into());
    }
    Ok(Kind::Import)
}

fn find<'m>(m: &'m SemanticModel, name: &str) -> Option<&'m Table> {
    m.tables.iter().find(|t| t.name.eq_ignore_ascii_case(name))
}

fn listed(list: &[String], name: &str) -> bool {
    list.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// What the refresh covers: the tables whose data is replaced (`refresh`: import tables run
/// again, calculated tables evaluated again), those that keep their stored data columns and get
/// their calculated columns recomputed (`stored`), what is skipped and why, and the calculated
/// columns that read refreshed data but cannot be recomputed (`stale`).
#[derive(Debug, Default)]
struct Plan {
    refresh: Vec<String>,
    stored: Vec<String>,
    skipped: Vec<Value>,
    stale: Vec<Value>,
}

fn plan(model: &SemanticModel, wanted: Option<&[String]>) -> Plan {
    let mut out = Plan::default();
    for t in &model.tables {
        let chosen = wanted.is_none_or(|w| listed(w, &t.name));
        if !chosen {
            continue;
        }
        match kind_of(t) {
            Ok(_) => out.refresh.push(t.name.clone()),
            Err(reason) => {
                // a full refresh says nothing of a calculation group (it has no data to load)
                let quiet = wanted.is_none()
                    && t.partitions
                        .iter()
                        .any(|p| is(&p.source_type, "calculationGroup"));
                if !quiet {
                    out.skipped.push(json!({"table": t.name, "reason": reason}));
                }
            }
        }
    }
    if out.refresh.is_empty() {
        return out;
    }
    // What reads the refreshed tables, transitively: calculated tables are evaluated again,
    // calculated columns of other import tables recomputed over their stored rows.
    let g = dependencies(model);
    let mut changed: Vec<ObjectRef> = Vec::new();
    let seed = |t: &Table, changed: &mut Vec<ObjectRef>| {
        changed.push(ObjectRef::Table { name: t.name.clone() });
        for c in t.columns.iter().filter(|c| !c.is_row_number()) {
            changed.push(ObjectRef::Column {
                table: t.name.clone(),
                name: c.name.clone(),
            });
        }
    };
    for name in &out.refresh {
        if let Some(t) = find(model, name) {
            seed(t, &mut changed);
        }
    }
    let mut seen: std::collections::HashSet<usize> = changed.iter().filter_map(|o| g.index_of(o)).collect();
    let mut queue: Vec<ObjectRef> = changed;
    let mut steps = 0usize;
    while let Some(o) = queue.pop() {
        steps = steps.saturating_add(1);
        if steps > 1_000_000 {
            break;
        }
        for d in g.dependents_of(&o) {
            let Some(i) = g.index_of(d) else { continue };
            if !seen.insert(i) {
                continue;
            }
            queue.push(d.clone());
            match d {
                ObjectRef::Table { name } => {
                    if let Some(t) = find(model, name)
                        && kind_of(t) == Ok(Kind::Calculated)
                        && !listed(&out.refresh, &t.name)
                    {
                        out.refresh.push(t.name.clone());
                        let mut more = Vec::new();
                        seed(t, &mut more);
                        for m in more {
                            if let Some(j) = g.index_of(&m)
                                && seen.insert(j)
                            {
                                queue.push(m);
                            }
                        }
                    }
                }
                ObjectRef::Column { table, name } => {
                    let Some(t) = find(model, table) else { continue };
                    let calculated = t
                        .columns
                        .iter()
                        .any(|c| c.name.eq_ignore_ascii_case(name) && c.expression.is_some());
                    if !calculated || listed(&out.refresh, &t.name) || listed(&out.stored, &t.name) {
                        continue;
                    }
                    match kind_of(t) {
                        Ok(Kind::Import) => {
                            out.stored.push(t.name.clone());
                            // what reads the table as a whole (FILTER(Sales, …)) sees the new column
                            let whole = ObjectRef::Table { name: t.name.clone() };
                            if let Some(j) = g.index_of(&whole)
                                && seen.insert(j)
                            {
                                queue.push(whole);
                            }
                        }
                        Ok(Kind::Calculated) => {}
                        Err(reason) => out.stale.push(json!({
                            "table": t.name,
                            "column": name,
                            "reason": format!("its table is not refreshed by bicraft ({reason})"),
                        })),
                    }
                }
                _ => {}
            }
        }
    }
    // a table refreshed later in the walk is no longer only recomputed
    let refresh = out.refresh.clone();
    out.stored.retain(|t| !listed(&refresh, t));
    out
}

/// Bytes the rows of a loaded table take in memory, about (values, text, the encoder's data
/// ids).
fn table_bytes(rows: usize, cols: &Cols) -> usize {
    let cell = std::mem::size_of::<CellValue>().saturating_add(8);
    let mut total = rows.saturating_mul(cols.len()).saturating_mul(cell);
    for (_, values) in cols {
        for v in values {
            if let CellValue::Text(s) = v {
                total = total.saturating_add(s.capacity());
            }
        }
    }
    total
}

/// The refresh time as Windows FILETIME ticks.
fn filetime(clock_ms: i64) -> i64 {
    clock_ms
        .saturating_add(FILETIME_TO_OLE_DAYS.saturating_mul(86_400_000))
        .saturating_mul(10_000)
}

/// What the written file must read back as.
struct Expect<'a> {
    base: &'a SemanticModel,
    rows: Vec<(String, usize)>,
}

/// Reopens the written package with bicraft's reader: the model reads, has the same structure
/// as before (tables, columns, relationships, partitions), and every refreshed table has its
/// new row count. Returns the model read back.
fn verify(tmp: &Path, expect: &Expect<'_>) -> Result<SemanticModel, String> {
    let pkg = bicraft_opc::Package::open_deferring(
        tmp,
        &bicraft_opc::ReadLimits::default(),
        &[bicraft_pbix::DATA_MODEL],
    )
    .map_err(|e| format!("the written file does not open: {e}"))?;
    let part = pkg
        .get(bicraft_pbix::DATA_MODEL)
        .ok_or("the written file has no DataModel")?;
    let bytes = part.bytes().map_err(|e| e.to_string())?;
    let editor = DataModelEditor::open(&bytes, &bicraft_pbix::DataModelLimits::default())
        .map_err(|e| format!("the written DataModel does not read back: {e}"))?;
    drop(bytes);
    let back = editor.base().clone();
    let ch = bicraft_pbix::structural::diff(expect.base, &back);
    if !ch.is_empty() {
        return Err(format!(
            "the written model differs from the refreshed one: {}",
            serde_json::to_string(&ch).unwrap_or_default()
        ));
    }
    for (table, rows) in &expect.rows {
        let got = editor.row_count(table).map_err(|e| e.to_string())?;
        if got != Some(*rows as u64) {
            return Err(format!(
                "table '{table}' reads back {} rows, {rows} were written",
                got.map_or_else(|| "no".to_string(), |n| n.to_string())
            ));
        }
    }
    drop(editor);
    Ok(back)
}

/// `model.refresh` on the open PBIX; see the module docs.
pub(crate) fn refresh(s: &mut Session, req: Request) -> Result<Value, String> {
    let started = Instant::now();
    let Some(Source::Package { path, package }) = &s.source else {
        return Err("no PBIX is open".into());
    };
    if package.get(bicraft_pbix::DATA_MODEL_SCHEMA).is_some() {
        return Err(
            "a PBIT holds no data: model.refresh refreshes a PBIX or a PBIP project (save the template as a \
             .pbix in Power BI Desktop, or convert it with file.convert)"
                .into(),
        );
    }
    let part = package.get(bicraft_pbix::DATA_MODEL).ok_or(
        "this package has no DataModel (a report connected to a published semantic model): its data is \
         refreshed where the model lives",
    )?;
    if s.dirty && !req.force {
        return Err(
            "the model has unsaved edits: save them first (file.save), or pass force: true to refresh the \
             file's saved model and keep the edits unsaved"
                .into(),
        );
    }
    if part.len() > bicraft_opc::MAX_PART_BYTES {
        return Err(bicraft_pbix::Error::DataModelTooLargeToWrite(part.len()).to_string());
    }
    let path = path.clone();
    let name = part.name.clone();
    let previous_bytes = std::fs::metadata(&path).map(|m| m.len()).ok();

    // The part and its backup are held in memory while the refresh runs.
    let pool = s.preview.memory_pool();
    let mut held = pool
        .reserve(
            bicraft_mem::Class::Io,
            usize::try_from(part.len()).unwrap_or(usize::MAX),
        )
        .map_err(|e| format!("nothing was refreshed: the DataModel needs memory {e}"))?;
    let part_bytes = part.bytes().map_err(|e| e.to_string())?;
    let editor = DataModelEditor::open(&part_bytes, &bicraft_pbix::DataModelLimits::default())
        .map_err(|e| e.to_string())?;
    held.try_grow(editor.backup_len())
        .map_err(|e| format!("nothing was refreshed: the decompressed DataModel needs memory {e}"))?;
    let base = editor.base().clone();
    if let Some(w) = &req.wanted {
        for n in w {
            if find(&base, n).is_none() {
                return Err(format!("table '{n}' is not in the model"));
            }
        }
    }
    let plan = plan(&base, req.wanted.as_deref());
    if plan.refresh.is_empty() {
        let why: Vec<String> = plan
            .skipped
            .iter()
            .filter_map(|v| {
                Some(format!(
                    "{}: {}",
                    v.get("table")?.as_str()?,
                    v.get("reason")?.as_str()?
                ))
            })
            .collect();
        return Err(format!(
            "nothing to refresh: no import or calculated table{}",
            if why.is_empty() {
                String::new()
            } else {
                format!(" ({})", why.join("; "))
            }
        ));
    }
    let all: Vec<String> = plan.refresh.iter().chain(&plan.stored).cloned().collect();

    // 1. Load and evaluate (nothing is written unless every table loads).
    let held = RefCell::new(held);
    let admit = |table: &str, rows: usize, cols: &Cols| -> Result<(), String> {
        let bytes = table_bytes(rows, cols);
        held.borrow_mut().try_grow(bytes).map_err(|e| {
            format!(
                "nothing was refreshed, the file is unchanged: table '{table}' ({rows} rows) needs about {} \
                 of memory, {e}",
                bicraft_mem::size_text(bytes)
            )
        })
    };
    let ld = Load {
        root: req.root.as_deref(),
        connectors: req.connectors.clone(),
        clock: Some(req.clock),
        offset_minutes: req.offset_minutes,
        stored: plan.stored.clone(),
        admit: Some(&admit),
    };
    let changes = editor.changes_refreshing(&base, &all);
    let t_load = Instant::now();
    let gathered = crate::pbix_struct::gather(&changes, &base, &editor, &part_bytes, &ld).map_err(|e| {
        let hint = if e.contains("external source, not evaluated") && req.connector_names.is_empty() {
            " (pass allow_connectors, e.g. [\"databricks\"], to let bicraft run it)"
        } else {
            ""
        };
        if e.starts_with("nothing was refreshed") {
            format!("{e}{hint}")
        } else {
            format!("nothing was refreshed, the file is unchanged: {e}{hint}")
        }
    })?;
    let load_ms = crate::pbix_struct::millis_since(t_load);
    drop(part_bytes);

    // 2. Encode and write the new DataModel.
    let t_write = Instant::now();
    let bytes = editor
        .write_refreshed(
            &gathered.model,
            &gathered.data,
            &gathered.partitions,
            &all,
            Some(filetime(req.clock)),
        )
        .map_err(|e| format!("nothing was refreshed, the file is unchanged: {e}"))?;
    drop(editor);
    let stats = gathered.stats;
    let full = gathered.model;
    drop(gathered.data);
    let Some(Source::Package { package, .. }) = &s.source else {
        return Err("no PBIX is open".into());
    };
    let mut out_pkg = package.clone();
    out_pkg.put(&name, bytes, true);
    let encode_ms = crate::pbix_struct::millis_since(t_write);

    // 3. Write a temporary sibling, reopen it, then replace the file.
    let expect = Expect {
        base: &full,
        rows: stats.iter().map(|t| (t.name.clone(), t.rows)).collect(),
    };
    let t_verify = Instant::now();
    let read_back: RefCell<Option<SemanticModel>> = RefCell::new(None);
    let check = |tmp: &Path| -> Result<(), String> {
        let m =
            verify(tmp, &expect).map_err(|e| format!("nothing was refreshed, the file is unchanged: {e}"))?;
        *read_back.borrow_mut() = Some(m);
        Ok(())
    };
    let bak = if req.backup {
        Some(crate::save::backup_path(&path)?)
    } else {
        None
    };
    crate::save::replace_file_checked(&path, &mut out_pkg, bak.as_deref(), Some(&check))?;
    let write_ms = crate::pbix_struct::millis_since(t_verify);
    let read_back = read_back.into_inner();

    // 4. The session reads the new file.
    let statements: Vec<Value> = req
        .dbx
        .as_ref()
        .map(|c| {
            c.take_log()
                .into_iter()
                .map(|e| json!({"host": e.host, "sql": e.sql, "rows": e.rows, "ms": e.millis}))
                .collect()
        })
        .unwrap_or_default();
    s.source = Some(Source::Package {
        path: path.clone(),
        package: out_pkg,
    });
    let mut warnings: Vec<String> = Vec::new();
    if s.dirty {
        warnings.push(
            "the model has unsaved edits: the file's saved model was refreshed, the edits are still unsaved"
                .into(),
        );
    } else if let Some(m) = read_back {
        s.model = Some(m);
        s.model_error = None;
    }
    s.refresh = Default::default();
    let bytes_now = std::fs::metadata(&path).map(|m| m.len()).ok();
    let tables: Vec<Value> = stats
        .iter()
        .map(|t| {
            json!({
                "table": t.name,
                "origin": t.origin,
                "rows": t.rows,
                "columns": t.columns,
                "partitions": t.partitions,
                "ms": t.millis,
            })
        })
        .collect();
    let kept = base.tables.iter().filter(|t| !listed(&all, &t.name)).count();
    Ok(json!({
        "path": path.display().to_string(),
        "format": "pbix",
        "refreshed_at": req.refreshed_at,
        "tables": tables,
        "kept": kept,
        "skipped": plan.skipped,
        "stale": plan.stale,
        "not_computed": [],
        "connectors": req.connector_names,
        "statements": statements,
        "bytes": bytes_now,
        "previous_bytes": previous_bytes,
        "backup": bak.map(|b| b.display().to_string()),
        "verified": true,
        "phases_ms": {"load": load_ms, "encode": encode_ms, "write_verify": write_ms},
        "elapsed_ms": crate::pbix_struct::millis_since(started),
        "warnings": warnings,
    }))
}

#[cfg(test)]
#[path = "refresh_pbix_tests.rs"]
mod tests;
