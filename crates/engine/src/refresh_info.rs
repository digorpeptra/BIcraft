//! Refresh state of the stored data (M9.3): each partition's `State` and `RefreshedTime`, the
//! tables' and the model's `ModifiedTime` / `StructureModifiedTime`, and the database's last
//! data and schema change, read from `metadata.sqlitedb` of the open file's backup (a PBIX
//! `DataModel` or a PBIP `.pbi/cache.abf`). Read-only.
//!
//! `model.storage_stats` reports them with its partitions; `model.get {detail:"full"}` and
//! `table.get` add `refresh` blocks to the model, its tables and their partitions when the open
//! file has a backup. Times are Windows FILETIME ticks (100 ns since 1601-01-01 UTC) in the
//! database and ISO-8601 UTC strings here; the "never" value the engine writes (1699-12-31, see
//! `docs/formats/metadata-sqlitedb.md`, "Refresh state") reads as `null`.

use std::sync::Arc;

use bicraft_sqlite::Value as Sql;
use serde_json::{Value, json};

use crate::{Session, Source};

type SqlRow = std::collections::BTreeMap<String, Sql>;

/// `RefreshedTime` of objects the engine never refreshed (1699-12-31T00:00Z as FILETIME
/// ticks); every value at or before it reads as "never".
const NEVER: i64 = 31_240_512_000_000_000;
/// FILETIME ticks per second.
const TICKS_PER_SECOND: i64 = 10_000_000;
/// Seconds between 1601-01-01 and 1970-01-01.
const FILETIME_UNIX_OFFSET_SECS: i64 = 11_644_473_600;
/// 9999-12-31T23:59:59Z as FILETIME ticks: later values are not dates.
const LATEST: i64 = 2_650_467_743_990_000_000;

/// The TOM `ObjectState` name of a `State` code.
pub(crate) fn state_name(code: i64) -> &'static str {
    match code {
        1 => "Ready",
        3 => "NoData",
        4 => "CalculationNeeded",
        5 => "SemanticError",
        6 => "EvaluationError",
        7 => "DependencyError",
        8 => "Incomplete",
        10 => "ForceCalculationNeeded",
        _ => "Unknown",
    }
}

/// A FILETIME value as an ISO-8601 UTC string (`2026-10-14T09:30:00Z`, milliseconds when not
/// zero); `None` for "never", zero, negative and out-of-range values.
pub(crate) fn filetime_iso(ticks: i64) -> Option<String> {
    if ticks <= NEVER || ticks > LATEST {
        return None;
    }
    let secs = ticks.div_euclid(TICKS_PER_SECOND);
    let millis = ticks.rem_euclid(TICKS_PER_SECOND) / 10_000;
    let unix = secs.checked_sub(FILETIME_UNIX_OFFSET_SECS)?;
    let days = unix.div_euclid(86_400);
    let rem = unix.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let time = format!("{:02}:{:02}:{:02}", rem / 3600, rem % 3600 / 60, rem % 60);
    Some(if millis == 0 {
        format!("{y:04}-{m:02}-{d:02}T{time}Z")
    } else {
        format!("{y:04}-{m:02}-{d:02}T{time}.{millis:03}Z")
    })
}

/// Days from 1601-01-01 (the FILETIME epoch) to 1899-12-30 (the DAX date serial 0).
const FILETIME_SERIAL_OFFSET_DAYS: f64 = 109_205.0;
/// FILETIME ticks per day.
const TICKS_PER_DAY: f64 = 864_000_000_000.0;

/// A FILETIME value as a DAX date/time serial (days since 1899-12-30, UTC, fraction of the day
/// for the time); `None` for the values [`filetime_iso`] reads as "never" or out of range.
pub(crate) fn filetime_serial(ticks: i64) -> Option<f64> {
    if ticks <= NEVER || ticks > LATEST {
        return None;
    }
    Some(ticks as f64 / TICKS_PER_DAY - FILETIME_SERIAL_OFFSET_DAYS)
}

/// Year, month, day of a day count since 1970-01-01 (proleptic Gregorian).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    bicraft_locale::calendar::civil_from_days(days)
}

fn int(row: &SqlRow, col: &str) -> Option<i64> {
    match row.get(col)? {
        Sql::Integer(n) => Some(*n),
        _ => None,
    }
}

fn text(row: &SqlRow, col: &str) -> Option<String> {
    match row.get(col) {
        Some(Sql::Text(s)) => Some(s.clone()),
        _ => None,
    }
}

fn time(t: Option<i64>) -> Value {
    t.and_then(filetime_iso).map_or(Value::Null, Value::String)
}

/// One partition's refresh state.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct PartitionRefresh {
    pub name: String,
    pub state: Option<i64>,
    pub refreshed: Option<i64>,
    pub modified: Option<i64>,
    pub error: Option<String>,
}

impl PartitionRefresh {
    fn of(row: &SqlRow) -> Self {
        PartitionRefresh {
            name: text(row, "Name").unwrap_or_default(),
            state: int(row, "State"),
            refreshed: int(row, "RefreshedTime").filter(|t| *t > NEVER),
            modified: int(row, "ModifiedTime"),
            error: text(row, "ErrorMessage").filter(|e| !e.is_empty()),
        }
    }

    /// `state`, `state_code`, `refreshed_time`, `modified_time`, `error_message`.
    pub fn insert_into(&self, o: &mut serde_json::Map<String, Value>) {
        o.insert("state".into(), json!(self.state.map(state_name)));
        o.insert("state_code".into(), json!(self.state));
        o.insert("refreshed_time".into(), time(self.refreshed));
        o.insert("modified_time".into(), time(self.modified));
        o.insert("error_message".into(), json!(self.error));
    }

    fn json(&self) -> Value {
        let mut o = serde_json::Map::new();
        self.insert_into(&mut o);
        Value::Object(o)
    }
}

/// One user table's times and its partitions.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct TableRefresh {
    pub name: String,
    pub modified: Option<i64>,
    pub structure_modified: Option<i64>,
    pub partitions: Vec<PartitionRefresh>,
}

impl TableRefresh {
    fn last_refresh(&self) -> Option<i64> {
        self.partitions.iter().filter_map(|p| p.refreshed).max()
    }

    pub fn partition(&self, name: &str) -> Option<&PartitionRefresh> {
        self.partitions.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    fn json(&self) -> Value {
        json!({
            "last_refresh": time(self.last_refresh()),
            "modified_time": time(self.modified),
            "structure_modified_time": time(self.structure_modified),
        })
    }
}

/// The refresh state of a whole backup.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct RefreshInfo {
    /// `DataModel` or `.pbi/cache.abf`.
    pub source: &'static str,
    pub modified: Option<i64>,
    pub structure_modified: Option<i64>,
    /// `DBPROPERTIES` `LASTCUBEDATACHANGE` / `LASTCUBESCHEMACHANGE`.
    pub data_change: Option<i64>,
    pub schema_change: Option<i64>,
    pub tables: Vec<TableRefresh>,
}

impl RefreshInfo {
    pub fn table(&self, name: &str) -> Option<&TableRefresh> {
        self.tables.iter().find(|t| t.name.eq_ignore_ascii_case(name))
    }

    /// The model-level block: last refresh (latest partition), partitions not ready, times.
    pub fn json(&self) -> Value {
        let parts = || self.tables.iter().flat_map(|t| t.partitions.iter());
        let not_ready: Vec<Value> =
            self.tables
                .iter()
                .flat_map(|t| {
                    t.partitions.iter().filter(|p| p.state != Some(1)).map(
                        |p| json!({"table": t.name, "partition": p.name, "state": p.state.map(state_name)}),
                    )
                })
                .collect();
        json!({
            "source": self.source,
            "last_refresh": time(self.tables.iter().filter_map(TableRefresh::last_refresh).max()),
            "last_data_change": time(self.data_change),
            "last_schema_change": time(self.schema_change),
            "modified_time": time(self.modified),
            "structure_modified_time": time(self.structure_modified),
            "partition_count": parts().count(),
            "partitions_not_ready": not_ready,
        })
    }
}

/// The refresh state recorded in a `metadata.sqlitedb`. Missing tables read as empty; system
/// tables (`SystemFlags` bit 0) are left out.
pub(crate) fn from_db(db: &bicraft_sqlite::Database) -> Result<RefreshInfo, String> {
    let load = |name: &str| -> Result<Vec<SqlRow>, String> {
        if db.table(name).is_none() {
            return Ok(Vec::new());
        }
        db.query_all(name).map_err(|e| format!("{name}: {e}"))
    };
    let mut out = RefreshInfo::default();
    if let Some(m) = load("Model")?.first() {
        out.modified = int(m, "ModifiedTime");
        out.structure_modified = int(m, "StructureModifiedTime");
    }
    for p in load("DBPROPERTIES")? {
        let field = |want: &str| {
            p.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(want))
                .map(|(_, v)| v)
        };
        let (Some(Sql::Text(name)), Some(Sql::Integer(v))) = (field("NAME"), field("VALUE")) else {
            continue;
        };
        if name.eq_ignore_ascii_case("LASTCUBEDATACHANGE") {
            out.data_change = Some(*v);
        } else if name.eq_ignore_ascii_case("LASTCUBESCHEMACHANGE") {
            out.schema_change = Some(*v);
        }
    }
    let mut tables = load("Table")?;
    tables.sort_by_key(|t| int(t, "ID").unwrap_or(0));
    let mut partitions = load("Partition")?;
    partitions.sort_by_key(|p| int(p, "ID").unwrap_or(0));
    for t in &tables {
        if int(t, "SystemFlags").unwrap_or(0) & 1 != 0 {
            continue;
        }
        let Some(id) = int(t, "ID") else {
            continue;
        };
        out.tables.push(TableRefresh {
            name: text(t, "Name").unwrap_or_default(),
            modified: int(t, "ModifiedTime"),
            structure_modified: int(t, "StructureModifiedTime"),
            partitions: partitions
                .iter()
                .filter(|p| int(p, "TableID") == Some(id))
                .map(PartitionRefresh::of)
                .collect(),
        });
    }
    Ok(out)
}

/// Per-session cache of [`load`], valid for one session revision. Behind a mutex: `model.get`
/// runs on a shared session (`Session::execute_shared`); two reads of one revision compute the
/// same value, so either may fill it.
#[derive(Debug, Default)]
pub(crate) struct Cache(std::sync::Mutex<(Option<u64>, Option<Arc<RefreshInfo>>)>);

/// The refresh state of the open file's backup; `None` when it has none (a PBIT, a PBIP
/// without `.pbi/cache.abf`, a `model.bim`, a new model) or it cannot be read.
pub(crate) fn load(s: &Session) -> Option<Arc<RefreshInfo>> {
    {
        let c = s
            .refresh
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if c.0 == Some(s.revision) {
            return c.1.clone();
        }
    }
    // read outside the lock (it may read a large backup)
    let info = read(s).ok().flatten().map(Arc::new);
    *s.refresh
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = (Some(s.revision), info.clone());
    info
}

fn read(s: &Session) -> Result<Option<RefreshInfo>, String> {
    let none = |_: &bicraft_abf::FileEntry| false;
    let limits = bicraft_pbix::DataModelLimits::default();
    let (source, files) = match &s.source {
        Some(Source::Package { package, .. }) => {
            let Some(part) = package.get(bicraft_pbix::DATA_MODEL) else {
                return Ok(None);
            };
            (
                "DataModel",
                bicraft_pbix::storage::read_part(part, &none, &limits).map_err(|e| e.to_string())?,
            )
        }
        Some(Source::Project { .. }) => {
            let Some(path) = crate::cache_abf::path(s)? else {
                return Ok(None);
            };
            (
                crate::cache_abf::label(s),
                bicraft_pbix::storage::read_file(&path, &none, &limits).map_err(|e| e.to_string())?,
            )
        }
        _ => return Ok(None),
    };
    let db = bicraft_sqlite::Database::from_bytes(&files.sqlitedb).map_err(|e| e.to_string())?;
    let mut info = from_db(&db)?;
    info.source = source;
    Ok(Some(info))
}

/// Adds `refresh` blocks to a `table.get` / `model.get` table object and its partitions.
pub(crate) fn annotate_table(info: &RefreshInfo, v: &mut Value) {
    let Some(name) = v.get("name").and_then(Value::as_str) else {
        return;
    };
    let Some(t) = info.table(name) else {
        return;
    };
    for p in v
        .get_mut("partitions")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        let found = p.get("name").and_then(Value::as_str).and_then(|n| t.partition(n));
        if let (Some(found), Some(o)) = (found, p.as_object_mut()) {
            o.insert("refresh".into(), found.json());
        }
    }
    if let Some(o) = v.as_object_mut() {
        o.insert("refresh".into(), t.json());
    }
}

/// Adds the model-level `refresh` block and the table and partition blocks to a
/// `model.get {detail:"full"}` result.
pub(crate) fn annotate_model(info: &RefreshInfo, v: &mut Value) {
    for t in v
        .get_mut("tables")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        annotate_table(info, t);
    }
    if let Some(o) = v.as_object_mut() {
        o.insert("refresh".into(), info.json());
    }
}

#[cfg(test)]
#[path = "refresh_info_tests.rs"]
mod refresh_info_tests;
