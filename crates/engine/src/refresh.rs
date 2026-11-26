//! `model.refresh` (gap G21): bicraft refreshes an open PBIP project's import tables itself,
//! with the connectors the caller explicitly allows (Databricks), and keeps the rows in its own
//! data snapshot next to the project, `<Name>.SemanticModel/.pbi/bicraft-data.bin`
//! (`bicraft_data::store`), which the DAX evaluator (`dax.evaluate`, `report.render`, the
//! viewer) then reads.
//!
//! - M partitions run in bicraft's Power Query evaluator with the allowed connectors and the
//!   refresh's start time as the UTC clock; files only under `data_root`.
//! - Calculated tables and calculated columns are evaluated by bicraft's DAX evaluator over the
//!   refreshed tables, in passes (they may depend on each other); one bicraft cannot evaluate is
//!   listed in `not_computed` and stays unavailable.
//! - Nothing is written unless every requested import table loads. A refresh of some tables
//!   keeps the other tables of the previous snapshot that still match the model.
//! - `.pbi/cache.abf` is never read, written or copied here. The snapshot is the data the session
//!   reads for the project while it is newer than `cache.abf` (Desktop's own refresh) and no
//!   explicit `cache_abf` is set ([`open_snapshot`]).
//!
//! Each stored table carries a stable fingerprint of what produced it (partitions, columns,
//! culture, the queries and parameters it references): a table whose definition changed since is
//! stale and ignored. See `docs/formats/data-sources.md`, *Refresh by bicraft*.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use bicraft_data::connect::Connectors;
use bicraft_data::store::{Snapshot, StoredTable, fingerprint, read_snapshot, write_snapshot};
use bicraft_data::{ColumnData, Limits, TableData};
use bicraft_model::{DataType, SemanticModel, Table};
use bicraft_vertipaq::CellValue;
use serde_json::{Value, json};

use crate::params::{allow, opt_bool};
use crate::{Session, Source};

/// File name of the snapshot in the semantic model's `.pbi` folder.
pub const SNAPSHOT_FILE: &str = "bicraft-data.bin";
/// Largest snapshot read (and decompressed body).
const MAX_SNAPSHOT_BYTES: usize = 4 << 30;
/// Most rows of one refreshed table.
const MAX_ROWS: usize = 50_000_000;
/// Passes over calculated tables and columns.
const MAX_PASSES: usize = 16;
/// Connector families `allow_connectors` accepts.
pub const CONNECTORS: &[&str] = &["databricks", "sqlserver", "web", "odata", "sharepoint"];

/// The `allow_connectors` family that runs M function `function`.
pub(crate) fn family_of(function: &str) -> Option<&'static str> {
    Some(match function {
        "DatabricksMultiCloud.Catalogs" | "Databricks.Catalogs" => "databricks",
        "Sql.Database" | "Sql.Databases" => "sqlserver",
        "Web.Contents" => "web",
        "OData.Feed" => "odata",
        "SharePoint.Files" | "SharePoint.Contents" | "SharePoint.Tables" => "sharepoint",
        _ => return None,
    })
}

/// Where the Databricks token comes from (`model.refresh` with `allow_connectors:
/// ["databricks"]`). `None` in the session: the environment variable
/// `BICRAFT_DATABRICKS_TOKEN`. The CLI adds a masked console prompt when it is unset.
#[derive(Clone)]
pub struct TokenSource(pub bicraft_databricks::TokenProvider);

impl std::fmt::Debug for TokenSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenSource(..)")
    }
}

/// The token of the environment variable, else why there is none.
pub fn token_from_env() -> Result<bicraft_databricks::Token, String> {
    bicraft_databricks::Token::from_env(bicraft_databricks::TOKEN_ENV).ok_or_else(|| {
        format!(
            "no Databricks token: set the environment variable {} (a personal access token) in the \
             shell that runs bicraft; it is read at refresh time and never stored",
            bicraft_databricks::TOKEN_ENV
        )
    })
}

/// A test warehouse: requests for `host` go to the loopback `base`, with `settings`.
#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct TestEndpoint {
    pub host: String,
    pub base: String,
    pub settings: bicraft_databricks::Settings,
}

/// The snapshot file of an open project.
pub(crate) fn snapshot_path(project: &bicraft_pbip::Project) -> Option<PathBuf> {
    project
        .semantic_model
        .as_ref()
        .map(|d| d.join(".pbi").join(SNAPSHOT_FILE))
}

/// The snapshot file to read: [`snapshot_path`] when it exists.
pub(crate) fn readable_snapshot_path(project: &bicraft_pbip::Project) -> Option<PathBuf> {
    snapshot_path(project).filter(|p| p.is_file())
}

fn modified(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).ok()?.modified().ok()
}

/// A snapshot the session reads.
#[derive(Debug)]
pub(crate) struct OwnData {
    pub snapshot: Snapshot,
}

impl OwnData {
    /// The stored rows of `t`, when they were produced by its current definition.
    pub(crate) fn fresh(&self, model: &SemanticModel, t: &Table) -> Option<&StoredTable> {
        self.snapshot
            .table(&t.name)
            .filter(|st| st.fingerprint == table_fingerprint(model, t))
    }
}

/// The project's snapshot file, when it is the data to read (see [`open_snapshot`]).
pub(crate) fn active_snapshot(s: &Session) -> Option<PathBuf> {
    let Some(Source::Project { project, .. }) = &s.source else {
        return None;
    };
    if s.cache_abf.is_some() {
        return None;
    }
    let path = readable_snapshot_path(project)?;
    if let Some(abf) = bicraft_pbip::cache_abf_path(project).filter(|p| p.is_file())
        && let (Some(a), Some(b)) = (modified(&abf), modified(&path))
        && a > b
    {
        return None;
    }
    Some(path)
}

/// The project's snapshot, when it is the data to read: it exists, no explicit `cache_abf` is
/// set, and the project's `.pbi/cache.abf` (Desktop's data) is not newer.
pub(crate) fn open_snapshot(s: &Session) -> Result<Option<Arc<OwnData>>, String> {
    let Some(path) = active_snapshot(s) else {
        return Ok(None);
    };
    let bytes = read_capped(&path)?;
    let snapshot =
        read_snapshot(&bytes, &snapshot_limits()).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some(Arc::new(OwnData { snapshot })))
}

fn snapshot_limits() -> Limits {
    Limits {
        max_bytes: MAX_SNAPSHOT_BYTES,
        max_rows: MAX_ROWS,
        ..Limits::default()
    }
}

fn read_capped(path: &Path) -> Result<Vec<u8>, String> {
    let len = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    if len > MAX_SNAPSHOT_BYTES as u64 {
        return Err(format!("{}: {len} bytes is too large", path.display()));
    }
    std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Whether a `.gitignore` from the snapshot's folder up to the repository root (a folder with
/// `.git`, at most 32 levels) has a line that ignores it: the file name, `.pbi/` or `*.bin`
/// patterns. A heuristic for a warning only: git's full matching rules are not reimplemented.
fn git_ignores_snapshot(snapshot: &Path, stop: Option<&Path>) -> bool {
    const PATTERNS: &[&str] = &[
        "**/.pbi/bicraft-data.bin",
        ".pbi/bicraft-data.bin",
        "bicraft-data.bin",
        "**/bicraft-data.bin",
        "**/.pbi/",
        "**/.pbi",
        ".pbi/",
        ".pbi",
        "*.bin",
        "**/*.bin",
    ];
    let mut dir = snapshot.parent();
    for _ in 0..32 {
        let Some(d) = dir else { break };
        // server mode: never read above the workspace root
        if stop.is_some_and(|r| !d.starts_with(r)) {
            break;
        }
        if let Ok(text) = std::fs::read_to_string(d.join(".gitignore"))
            && text.lines().any(|l| PATTERNS.contains(&l.trim()))
        {
            return true;
        }
        if d.join(".git").exists() {
            return false;
        }
        dir = d.parent();
    }
    false
}

/// What a table's stored rows depend on: its partitions, columns, the model's cultures and the
/// shared queries and parameters its M partitions reference.
pub(crate) fn table_fingerprint(model: &SemanticModel, t: &Table) -> u64 {
    let mut parts: Vec<String> = vec!["bicraft-data/1".into(), t.name.clone()];
    parts.push(model.culture.clone().unwrap_or_default());
    parts.push(
        model
            .extra
            .get("sourceQueryCulture")
            .map(Value::to_string)
            .unwrap_or_default(),
    );
    for p in &t.partitions {
        parts.push(p.source_type.clone());
        let expr = p.expression.clone().unwrap_or_default();
        if p.source_type.eq_ignore_ascii_case("m") {
            for (name, text) in crate::datasrc::shared_for(model, &t.name, &expr) {
                parts.push(name);
                parts.push(text);
            }
        }
        parts.push(expr);
    }
    for c in &t.columns {
        parts.push(c.name.clone());
        parts.push(c.source_column.clone().unwrap_or_default());
        parts.push(c.expression.clone().unwrap_or_default());
        parts.push(format!("{:?}", c.data_type));
    }
    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
    fingerprint(&refs)
}

/// Milliseconds since 1899-12-30T00:00 UTC now, and the ISO text of that instant.
pub(crate) fn now() -> (i64, String) {
    let unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0));
    let ms = unix_ms.saturating_add(25_569 * 86_400_000);
    let days = unix_ms.div_euclid(86_400_000);
    let secs = unix_ms.rem_euclid(86_400_000) / 1000;
    let (y, m, d) = civil(days);
    let iso = format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    );
    (ms, iso)
}

fn civil(z: i64) -> (i64, i64, i64) {
    bicraft_locale::calendar::civil_from_days(z)
}

/// The connectors `allow` names, and the Databricks one (for its statement log).
pub(crate) fn connectors(
    s: &Session,
    allow: &[String],
    external_links: bool,
) -> Result<(Connectors, Option<Arc<bicraft_databricks::DatabricksConnector>>), String> {
    let mut out = Connectors::new();
    let mut dbx = None;
    for name in allow {
        match name.to_ascii_lowercase().as_str() {
            "databricks" => {
                crate::sandbox::require_network(s, "the Databricks connector")?;
                let provider: bicraft_databricks::TokenProvider = match &s.token_source {
                    Some(TokenSource(p)) => Arc::clone(p),
                    None => Arc::new(token_from_env),
                };
                #[allow(unused_mut)]
                let mut settings = bicraft_databricks::Settings {
                    external_links,
                    ..bicraft_databricks::Settings::default()
                };
                #[cfg(test)]
                if let Some(t) = &s.test_endpoint {
                    settings = bicraft_databricks::Settings {
                        external_links,
                        ..t.settings.clone()
                    };
                }
                #[allow(unused_mut)]
                let mut c = bicraft_databricks::DatabricksConnector::new(provider, settings);
                #[cfg(test)]
                if let Some(t) = &s.test_endpoint {
                    c = c.with_test_endpoint(&t.host, &t.base)?;
                }
                let c = Arc::new(c);
                out.push(c.clone());
                dbx = Some(c);
            }
            "sqlserver" => out.push(crate::sqlserver::connector(s)?),
            "web" => out.push(crate::webdata::web(s)?),
            "odata" => out.push(crate::webdata::odata(s)?),
            "sharepoint" => out.push(crate::webdata::sharepoint(s)?),
            other => {
                return Err(format!(
                    "unknown connector `{other}` in allow_connectors (bicraft runs: {})",
                    CONNECTORS.join(", ")
                ));
            }
        }
    }
    Ok((out, dbx))
}

pub(crate) fn string_list(p: &Value, key: &str) -> Result<Option<Vec<String>>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(vec![s.clone()])),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("`{key}` must be a list of names"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err(format!("`{key}` must be a list of names")),
    }
}

/// A table loaded or computed by this refresh.
struct Loaded {
    name: String,
    origin: &'static str,
    data: TableData,
    millis: u64,
    statements: Vec<Value>,
}

/// Values converted to the column type, as Analysis Services stores them.
fn stored_column(
    model: &SemanticModel,
    name: &str,
    t: DataType,
    values: Vec<CellValue>,
    imported: bool,
) -> Result<ColumnData, String> {
    let (values, data_type) =
        crate::pbix_struct::coerce(values, t).map_err(|e| format!("column `{name}`: {e}"))?;
    Ok(ColumnData {
        name: name.to_string(),
        data_type,
        values: crate::datasrc::stored_values(model, imported, &values),
    })
}

/// `model.refresh`; see the module docs.
pub(crate) fn model_refresh(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "tables",
            "allow_connectors",
            "external_links",
            "data_root",
            "force",
            "backup",
            "utc_offset_minutes",
        ],
    )?;
    // `utc_offset_minutes`: local time minus UTC for the M clock functions and the calculated
    // tables' and columns' NOW()/UTCNOW(), as dax.evaluate's (default the process's local clock)
    let offset_minutes = match p.get("utc_offset_minutes") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_i64() {
            Some(m) if (-1080..=1080).contains(&m) => i32::try_from(m).ok(),
            _ => return Err("`utc_offset_minutes` must be an integer from -1080 to 1080".into()),
        },
    };
    let force = opt_bool(p, "force")?.unwrap_or(false);
    let backup = opt_bool(p, "backup")?.unwrap_or(false);
    if matches!(s.source, Some(Source::Package { .. })) {
        // a PBIX: the new rows go into its DataModel (refresh_pbix.rs)
        let wanted = string_list(p, "tables")?;
        let allow_list = string_list(p, "allow_connectors")?.unwrap_or_default();
        let external_links = opt_bool(p, "external_links")?.unwrap_or(false);
        let given_root = crate::datasrc::data_root_param(p)?;
        let root = crate::datasrc::effective_root(&given_root, s.data_root.as_deref());
        let (conns, dbx) = connectors(s, &allow_list, external_links)?;
        let (clock, refreshed_at) = now();
        let connector_names = conns.names().into_iter().map(str::to_string).collect();
        return crate::refresh_pbix::refresh(
            s,
            crate::refresh_pbix::Request {
                wanted,
                root,
                connectors: conns,
                connector_names,
                dbx,
                clock,
                offset_minutes,
                refreshed_at,
                force,
                backup,
            },
        );
    }
    let Some(Source::Project { project, .. }) = &s.source else {
        return Err(
            "model.refresh refreshes a PBIP project or a PBIX file: open one first (a PBIT holds no data, a \
             model.bim no partitions' rows)"
                .into(),
        );
    };
    let path = snapshot_path(project).ok_or("the project has no semantic model folder")?;
    let model = s.model.as_ref().ok_or("no model is open (file.open first)")?;
    let wanted = string_list(p, "tables")?;
    let allow_list = string_list(p, "allow_connectors")?.unwrap_or_default();
    let external_links = opt_bool(p, "external_links")?.unwrap_or(false);
    let given_root = crate::datasrc::data_root_param(p)?;
    let root = crate::datasrc::effective_root(&given_root, s.data_root.as_deref());
    if let Some(w) = &wanted {
        for name in w {
            if !model.tables.iter().any(|t| t.name.eq_ignore_ascii_case(name)) {
                return Err(format!("table '{name}' is not in the model"));
            }
        }
    }
    let in_scope = |t: &Table| {
        wanted
            .as_ref()
            .is_none_or(|w| w.iter().any(|n| n.eq_ignore_ascii_case(&t.name)))
    };
    let (conns, dbx) = connectors(s, &allow_list, external_links)?;
    let (clock, refreshed_at) = now();
    let started = Instant::now();

    // 1. Import tables (M partitions).
    let mut loaded: Vec<Loaded> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    let mut calculated: Vec<&Table> = Vec::new();
    for t in model.tables.iter().filter(|t| in_scope(t)) {
        if t.partitions.is_empty() {
            skipped.push(json!({"table": t.name, "reason": "no partition"}));
            continue;
        }
        if t.partitions
            .iter()
            .all(|p| p.source_type.eq_ignore_ascii_case("calculated"))
        {
            calculated.push(t);
            continue;
        }
        if let Some(p) = t
            .partitions
            .iter()
            .find(|p| !p.source_type.eq_ignore_ascii_case("m"))
        {
            skipped.push(json!({"table": t.name, "reason": format!("partition source `{}` is not refreshed by bicraft", p.source_type)}));
            continue;
        }
        if t.partitions.iter().any(|p| {
            p.mode
                .as_deref()
                .is_some_and(|m| m.eq_ignore_ascii_case("directQuery"))
        }) {
            skipped.push(json!({"table": t.name, "reason": "DirectQuery partitions hold no imported rows"}));
            continue;
        }
        let t0 = Instant::now();
        let mut merged: Vec<ColumnData> = Vec::new();
        let mut error = None;
        for part in &t.partitions {
            let expr = part.expression.as_deref().unwrap_or_default();
            let limits = Limits {
                max_rows: MAX_ROWS,
                max_millis: 0,
                ..Limits::default()
            };
            let mut opts =
                crate::datasrc::source_options(Some(model), &t.name, expr, root.as_deref(), limits);
            opts.connectors = conns.clone();
            opts.clock_utc_ms = Some(clock);
            opts.clock_local_offset_minutes =
                Some(offset_minutes.unwrap_or_else(|| crate::clock::offset_at_serial_ms(clock)));
            match bicraft_data::load_partition_opts(part, &t.columns, &opts) {
                Ok(data) => {
                    for c in data.into_columns() {
                        match merged.iter_mut().find(|m| m.name == c.name) {
                            Some(m) => m.values.extend(c.values),
                            None => merged.push(c),
                        }
                    }
                }
                Err(e) => {
                    error = Some(crate::datasrc::describe(&e, root.as_deref()));
                    break;
                }
            }
        }
        let statements: Vec<Value> = dbx
            .as_ref()
            .map(|c| {
                c.take_log()
                    .into_iter()
                    .map(|e| json!({"host": e.host, "sql": e.sql, "rows": e.rows, "ms": e.millis}))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(e) = error {
            let hint = if e.contains("external source, not evaluated") && allow_list.is_empty() {
                " (pass allow_connectors, e.g. [\"databricks\"], to let bicraft run it)"
            } else {
                ""
            };
            failures.push(format!("table '{}': {e}{hint}", t.name));
            continue;
        }
        let mut cols = Vec::with_capacity(merged.len());
        for c in merged {
            let values = crate::datasrc::stored_values(model, true, &c.values);
            cols.push(ColumnData { values, ..c });
        }
        let data = TableData::new(cols).map_err(|e| format!("table '{}': {e}", t.name))?;
        loaded.push(Loaded {
            name: t.name.clone(),
            origin: "m",
            data,
            millis: u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX),
            statements,
        });
    }
    if !failures.is_empty() {
        return Err(format!(
            "nothing was refreshed (every import table must load): {}",
            failures.join("; ")
        ));
    }

    // 2. Calculated tables and columns, over the refreshed tables (and the previous snapshot's
    //    other tables when only some were refreshed).
    let previous = open_snapshot(s).ok().flatten();
    let mut ev = crate::preview::StructEval::new(None, model.clone(), MAX_ROWS);
    ev.set_clock(Some(clock), offset_minutes);
    ev.set_data_root(root.clone());
    let mut kept: Vec<StoredTable> = Vec::new();
    if wanted.is_some()
        && let Some(prev) = &previous
    {
        for t in &model.tables {
            if in_scope(t) {
                continue;
            }
            if let Some(st) = prev.fresh(model, t) {
                kept.push(st.clone());
            }
        }
    }
    let provide = |ev: &mut crate::preview::StructEval, name: &str, data: &TableData| -> Result<(), String> {
        let cols: Vec<(String, Vec<CellValue>)> = data
            .columns()
            .iter()
            .map(|c| (c.name.clone(), c.values.clone()))
            .collect();
        ev.provide(name, data.row_count(), &cols, false)
    };
    for k in &kept {
        provide(&mut ev, &k.name, &k.data)?;
    }
    for l in &loaded {
        provide(&mut ev, &l.name, &l.data)?;
    }
    enum Work<'m> {
        Table(&'m Table),
        Column(&'m Table, &'m bicraft_model::Column),
    }
    let mut work: Vec<Work<'_>> = calculated.iter().map(|t| Work::Table(t)).collect();
    for t in model.tables.iter().filter(|t| in_scope(t)) {
        for c in t.columns.iter().filter(|c| c.expression.is_some()) {
            work.push(Work::Column(t, c));
        }
    }
    let mut not_computed: Vec<Value> = Vec::new();
    let mut last_errors: Vec<(String, Option<String>, String)> = Vec::new();
    for _ in 0..MAX_PASSES {
        if work.is_empty() {
            break;
        }
        last_errors.clear();
        let mut progressed = false;
        let mut next = Vec::new();
        for w in std::mem::take(&mut work) {
            match w {
                Work::Table(t) => {
                    let t0 = Instant::now();
                    let expr = t
                        .partitions
                        .first()
                        .and_then(|p| p.expression.clone())
                        .unwrap_or_default();
                    let result = ev.table(&expr).and_then(|r| {
                        let mut cols = Vec::with_capacity(r.columns.len());
                        for (name, ty, values) in r.columns {
                            cols.push(stored_column(
                                model,
                                &name,
                                ty.unwrap_or(DataType::Unknown),
                                values,
                                false,
                            )?);
                        }
                        let data = TableData::new(cols).map_err(|e| e.to_string())?;
                        bicraft_data::conform(data, &t.columns).map_err(|e| e.to_string())
                    });
                    match result {
                        Ok(data) => {
                            progressed = true;
                            provide(&mut ev, &t.name, &data)?;
                            loaded.push(Loaded {
                                name: t.name.clone(),
                                origin: "dax",
                                data,
                                millis: u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX),
                                statements: Vec::new(),
                            });
                            for c in t.columns.iter().filter(|c| c.expression.is_some()) {
                                next.push(Work::Column(t, c));
                            }
                        }
                        Err(e) => {
                            last_errors.push((t.name.clone(), None, e));
                            next.push(Work::Table(t));
                        }
                    }
                }
                Work::Column(t, c) => {
                    let Some(owner) = loaded.iter_mut().find(|l| l.name == t.name) else {
                        // its table is not loaded (yet)
                        last_errors.push((
                            t.name.clone(),
                            Some(c.name.clone()),
                            "its table has no data".into(),
                        ));
                        next.push(Work::Column(t, c));
                        continue;
                    };
                    let expr = c.expression.clone().unwrap_or_default();
                    let result = ev
                        .column(&t.name, &expr)
                        .and_then(|v| stored_column(model, &c.name, c.data_type, v, false));
                    match result {
                        Ok(col) => {
                            progressed = true;
                            ev.provide_column(&t.name, &c.name, &col.values)?;
                            let mut cols = std::mem::take(&mut owner.data).into_columns();
                            cols.retain(|x| !x.name.eq_ignore_ascii_case(&c.name));
                            cols.push(col);
                            owner.data =
                                TableData::new(cols).map_err(|e| format!("table '{}': {e}", t.name))?;
                        }
                        Err(e) => {
                            last_errors.push((t.name.clone(), Some(c.name.clone()), e));
                            next.push(Work::Column(t, c));
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
    for (table, column, reason) in last_errors {
        not_computed.push(json!({"table": table, "column": column, "reason": reason}));
    }

    // 3. Write the snapshot (atomically).
    let mut tables: Vec<StoredTable> = kept;
    for l in &loaded {
        let Some(t) = model.tables.iter().find(|t| t.name == l.name) else {
            continue;
        };
        tables.push(StoredTable {
            name: l.name.clone(),
            fingerprint: table_fingerprint(model, t),
            origin: l.origin.into(),
            data: l.data.clone(),
        });
    }
    let mut notes = vec![("refresh".to_string(), "model.refresh".to_string())];
    for c in conns.names() {
        notes.push(("connector".into(), c.into()));
    }
    let snap = Snapshot {
        refreshed_at: refreshed_at.clone(),
        producer: format!("bicraft {}", env!("CARGO_PKG_VERSION")),
        notes,
        tables,
    };
    let bytes = write_snapshot(&snap).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("bin.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })?;

    let mut warnings: Vec<String> = Vec::new();
    if !git_ignores_snapshot(&path, s.server_config().map(crate::ServerConfig::root)) {
        warnings.push(format!(
            "no .gitignore between the project and its git repository ignores the snapshot, which              holds the source's rows: add `**/.pbi/{SNAPSHOT_FILE}` (as `**/.pbi/cache.abf` is)"
        ));
    }
    if s.dirty {
        warnings.push(
            "the model has unsaved edits: the refreshed data matches them, save the project so it matches \
             the files too"
                .into(),
        );
    }
    let report: Vec<Value> = loaded
        .iter()
        .map(|l| {
            json!({
                "table": l.name,
                "origin": l.origin,
                "rows": l.data.row_count(),
                "columns": l.data.columns().len(),
                "ms": l.millis,
                "statements": l.statements,
            })
        })
        .collect();
    let out = json!({
        "path": path.display().to_string(),
        "refreshed_at": refreshed_at,
        "bytes": bytes.len(),
        "tables": report,
        "kept": snap.tables.len().saturating_sub(loaded.len()),
        "skipped": skipped,
        "not_computed": not_computed,
        "connectors": conns.names(),
        "elapsed_ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "warnings": warnings,
    });
    // The DAX evaluator reads the new snapshot from the next command on.
    s.refresh = Default::default();
    Ok(out)
}

#[cfg(test)]
#[path = "refresh_tests.rs"]
mod tests;
