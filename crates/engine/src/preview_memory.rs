//! The session's memory budget (`docs/beyond-ram.md`, lot 1): what the session keeps between
//! queries (decoded columns, storage-engine caches) is held under `BICRAFT_MEMORY_BUDGET` or the
//! session's `dax.memory {budget}`. Without a budget (the default) nothing here does any work.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use bicraft_vertipaq::CellValue;
use serde_json::{Value, json};

use super::store::{Codes, ColData, Miss};

/// Evaluations between two halvings of the columns' use counts ([`Memory::begin_evaluation`]).
const AGE_EVALUATIONS: u64 = 64;

/// A storage cache of at most this share of the budget stays while columns can give way
/// ([`enforce`]).
const SMALL_CACHE_SHARE: usize = 8;

/// A budget: bytes with an optional binary `K`, `M`, `G` or `T` suffix (`300M` = 300 MiB);
/// `0`, `unlimited`, `none` or an empty value: no budget (`Ok(None)`).
pub(crate) fn parse_budget(text: &str) -> Result<Option<u64>, String> {
    bicraft_mem::parse_bytes(text).map_err(|e| format!("memory budget {e}"))
}

/// `BICRAFT_MEMORY_BUDGET`, read once per process (an unreadable value: no budget).
fn env_budget() -> Option<u64> {
    static BUDGET: OnceLock<Option<u64>> = OnceLock::new();
    *BUDGET.get_or_init(|| {
        bicraft_env::var("BICRAFT_MEMORY_BUDGET")
            .ok()
            .and_then(|v| parse_budget(&v).ok().flatten())
    })
}

/// `BICRAFT_QUERY_BUDGET`, read once per process: `None` unset (or unreadable), `Some(None)` set
/// to no cap (`0`, `unlimited`).
fn env_query_budget() -> Option<Option<u64>> {
    static BUDGET: OnceLock<Option<Option<u64>>> = OnceLock::new();
    *BUDGET.get_or_init(|| {
        bicraft_env::var("BICRAFT_QUERY_BUDGET")
            .ok()
            .and_then(|v| parse_budget(&v).ok())
    })
}

#[cfg(test)]
thread_local! {
    /// Tests: this thread's spill folder (instead of the environment's).
    pub(crate) static TEST_SPILL_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Where a backup's temporary file goes: `BICRAFT_SPILL_DIR`, else the system's temporary folder.
pub(crate) fn spill_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(d) = TEST_SPILL_DIR.with(|d| d.borrow().clone()) {
        return d;
    }
    bicraft_env::var_os("BICRAFT_SPILL_DIR")
        .filter(|v| !v.is_empty())
        .map_or_else(std::env::temp_dir, PathBuf::from)
}

/// The budget's accounting for one session state.
#[derive(Default)]
pub(crate) struct Memory {
    /// The session's setting: `None` follows the process (`BICRAFT_MEMORY_BUDGET`), `Some(None)`
    /// is no budget.
    pub setting: Option<Option<u64>>,
    tick: u64,
    /// Per kept column: (bytes that do not change once read, last use).
    meta: HashMap<(String, String), (Option<usize>, u64)>,
    /// Evaluations started (see [`Memory::begin_evaluation`]).
    epoch: u64,
    /// Per column, kept or not: (evaluations that used it, aged; the last one). It survives the
    /// column's eviction, so a column read again keeps its history.
    uses: HashMap<(String, String), (u32, u64)>,
    /// Columns used by the running (or last finished) evaluation.
    recent: HashSet<(String, String)>,
    recent_epoch: u64,
    /// Tests: what was evicted ("column", "plain:column", "cache"), in order.
    #[cfg(test)]
    pub evicted_log: Vec<String>,
    pub evictions: u64,
    pub plain_dropped: u64,
    pub refused: u64,
    /// Columns read segment by segment (lot 3).
    pub lazy_reads: u64,
    /// Stored columns read under the budget (whole or segment by segment; a column evicted and
    /// read again counts again).
    pub column_reads: u64,
    pub cache_drops: u64,
    pub peak_counted: usize,
    /// The last failure to move the backup to a temporary file (shared with the thread that moves
    /// it after an answer).
    pub spill_error: Arc<std::sync::Mutex<Option<String>>>,
    /// The store was opened under the budget and its backup is still to be moved to a file.
    pub spill_pending: bool,
    /// The session's query budget (lot 4): `None` follows the process (`BICRAFT_QUERY_BUDGET`, else
    /// the memory budget), `Some(None)` is no cap.
    pub query_setting: Option<Option<u64>>,
    /// Formula-engine bytes: the last evaluation's peak, the highest peak, and refused evaluations.
    pub query_last_peak: usize,
    pub query_peak: usize,
    pub query_refused: u64,
    /// The pool its reservations go to (lot 5; `None`: the process', `bicraft_mem::global`). Tests
    /// give sessions a pool of their own.
    pub pool: Option<&'static bicraft_mem::Pool>,
    /// A background job's private state: its reservations are background work (admitted under
    /// three quarters of the limit, never given memory by eviction).
    pub background: bool,
    /// The last evaluation could not unpack a column's codes under the memory limit: it fails.
    pub unpack_refused: Option<String>,
}

impl Memory {
    /// A background job's accounting: its session's budget setting and pool, its reservations
    /// background work.
    pub fn background(setting: Option<Option<u64>>, pool: &'static bicraft_mem::Pool) -> Memory {
        Memory {
            setting,
            pool: Some(pool),
            background: true,
            ..Memory::default()
        }
    }

    /// The pool of this state's reservations.
    pub fn pool(&self) -> &'static bicraft_mem::Pool {
        self.pool.unwrap_or_else(bicraft_mem::global)
    }

    /// The process' memory limit (lot 5), `None` without one.
    pub fn limit(&self) -> Option<usize> {
        self.pool().limit()
    }

    /// The class of what this state keeps.
    pub fn kept_class(&self) -> bicraft_mem::Class {
        if self.background {
            bicraft_mem::Class::Background
        } else {
            bicraft_mem::Class::Kept
        }
    }

    /// The budget in bytes (`None`: no budget).
    pub fn budget(&self) -> Option<usize> {
        let b = match self.setting {
            Some(s) => s,
            None => env_budget(),
        }
        .map(|b| usize::try_from(b).unwrap_or(usize::MAX));
        // under a memory limit there is always a budget: half of it by default, never more than it
        match self.limit() {
            None => b,
            Some(l) => Some(b.unwrap_or(l / 2).min(l).max(1)),
        }
    }

    /// The cap of one evaluation's formula-engine memory (lot 4): the session's query budget,
    /// else `BICRAFT_QUERY_BUDGET`, else the memory budget; `None`: no cap.
    pub fn query_cap(&self) -> Option<usize> {
        let b = match self.query_setting {
            Some(s) => s,
            None => match env_query_budget() {
                Some(b) => b,
                None => return self.budget(),
            },
        }?;
        Some(usize::try_from(b).unwrap_or(usize::MAX))
    }

    fn query_source(&self) -> &'static str {
        match self.query_setting {
            Some(Some(_)) => "session",
            Some(None) => "none",
            None => match env_query_budget() {
                Some(Some(_)) => "environment",
                Some(None) => "none",
                None if self.budget().is_some() => "memory budget",
                None => "none",
            },
        }
    }

    fn source(&self) -> &'static str {
        match self.setting {
            Some(Some(_)) => "session",
            None if env_budget().is_some() => "environment",
            _ if self.limit().is_some() => "memory limit",
            _ => "none",
        }
    }

    /// A column was read or used: it becomes the most recently used, and counts one more
    /// evaluation using it (once per evaluation).
    pub fn touch(&mut self, key: &(String, String)) {
        if self.budget().is_none() {
            return;
        }
        self.tick = self.tick.wrapping_add(1);
        let tick = self.tick;
        self.meta.entry(key.clone()).or_insert((None, 0)).1 = tick;
        let epoch = self.epoch;
        let u = self.uses.entry(key.clone()).or_insert((0, u64::MAX));
        if u.1 != epoch {
            u.0 = u.0.saturating_add(1);
            u.1 = epoch;
        }
        // the first column an evaluation uses starts its recent set
        if self.recent_epoch != epoch {
            self.recent.clear();
            self.recent_epoch = epoch;
        }
        if !self.recent.contains(key) {
            self.recent.insert(key.clone());
        }
    }

    /// An evaluation starts (the first column it uses replaces the recent ones); every
    /// [`AGE_EVALUATIONS`] evaluations the use counts are halved (an old habit fades), and those
    /// of columns no longer used are forgotten.
    pub fn begin_evaluation(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch.is_multiple_of(AGE_EVALUATIONS) {
            self.uses.retain(|_, u| {
                u.0 /= 2;
                u.0 > 0
            });
        }
    }

    /// The order in which columns give way (first first): the columns the running (or last)
    /// evaluation did not use before those it used; then fewest uses first; among equal uses the
    /// most recently used first.
    ///
    /// A query run again finds what it read last time, and when its columns do not all fit it
    /// reads again only those it reads last. Least recently used first would evict, mid-query,
    /// the very columns the query reads next: each run would read every one of them again. Uses
    /// keep the usual working set ahead of a column a one-off query read.
    fn eviction_order(&self) -> Vec<(String, String)> {
        let mut order: Vec<(u8, u32, u64, &(String, String))> = self
            .meta
            .iter()
            .map(|(key, e)| {
                let recent = u8::from(self.recent.contains(key));
                (recent, self.uses.get(key).map_or(0, |u| u.0), u64::MAX - e.1, key)
            })
            .collect();
        order.sort();
        order.into_iter().map(|(_, _, _, k)| k.clone()).collect()
    }

    /// Follows the kept columns: forgets the dropped ones, sizes the new ones (a column put in the
    /// cache by another path, such as an aggregation job, counts as least recently used).
    fn sync_meta(&mut self, columns: &HashMap<(String, String), Result<Arc<ColData>, Miss>>) {
        self.meta.retain(|k, _| matches!(columns.get(k), Some(Ok(_))));
        for (k, c) in columns {
            if let Ok(c) = c {
                let e = self.meta.entry(k.clone()).or_insert((None, 0));
                if e.0.is_none() {
                    e.0 = Some(base_bytes(c));
                }
            }
        }
    }

    /// Bytes of the kept columns; without what queries added to them (`grown`: false) while a query
    /// runs, whose own reservations count those (lot 5).
    pub fn column_bytes(
        &mut self,
        columns: &HashMap<(String, String), Result<Arc<ColData>, Miss>>,
        grown: bool,
    ) -> usize {
        self.sync_meta(columns);
        columns
            .iter()
            .filter_map(|(key, c)| {
                c.as_ref().ok().map(|c| {
                    if grown {
                        self.bytes_of(key, c)
                    } else {
                        self.meta
                            .get(key)
                            .and_then(|m| m.0)
                            .unwrap_or_else(|| base_bytes(c))
                    }
                })
            })
            .fold(0usize, usize::saturating_add)
    }

    fn bytes_of(&self, key: &(String, String), c: &ColData) -> usize {
        let base = self
            .meta
            .get(key)
            .and_then(|m| m.0)
            .unwrap_or_else(|| base_bytes(c));
        base.saturating_add(changing_bytes(c))
    }
}

/// Bytes of a column that do not change once it is read: its codes (compressed segments and id
/// table, or one code per row) and its dictionary.
pub(crate) fn base_bytes(c: &ColData) -> usize {
    let codes = match &c.codes {
        Codes::Plain(v) => v.len().saturating_mul(4),
        Codes::Packed { packed, .. } => packed.bytes(),
    };
    let values = c
        .values
        .len()
        .saturating_mul(std::mem::size_of::<CellValue>())
        .saturating_add(
            c.values
                .iter()
                .map(|v| match v {
                    CellValue::Text(s) => s.capacity(),
                    _ => 0,
                })
                .sum::<usize>(),
        );
    codes.saturating_add(values)
}

/// Bytes of what queries add to a column: its unpacked codes and its derived tables.
fn changing_bytes(c: &ColData) -> usize {
    let plain = match &c.codes {
        Codes::Packed { plain, .. } => plain.get().map_or(0, |v| v.len().saturating_mul(4)),
        Codes::Plain(_) => 0,
    };
    let d = c.derived.get();
    let n = c.values.len();
    let per = |set: bool, b: usize| if set { n.saturating_mul(b) } else { 0 };
    let mut derived = per(d.index.get().is_some(), 48)
        .saturating_add(per(d.nums.get().is_some(), 16))
        .saturating_add(per(d.ranks.get().is_some(), 8))
        .saturating_add(per(d.currency.get().is_some_and(Option::is_some), 16))
        .saturating_add(per(d.ints.get().is_some_and(Option::is_some), 16));
    if let Some(Some(v)) = d.counts.get() {
        derived = derived.saturating_add(v.len().saturating_mul(8));
    }
    if let Some(Some(v)) = d.firsts.get() {
        derived = derived.saturating_add(v.len().saturating_mul(4));
    }
    if let Some(Some(v)) = d.rows_of.get() {
        derived = derived.saturating_add(v.len().saturating_mul(4));
    }
    plain.saturating_add(derived)
}

/// What a budget leaves to the segments of the columns read segment by segment: the cache of
/// their segments, the largest segment accepted, and the segments read together while such a
/// column is read: half of it (a column is read segment by segment from half the budget on, so a
/// column of one segment still fits).
pub(crate) fn segment_room(budget: usize) -> usize {
    (budget / 2).max(1)
}

/// What the segment cache keeps however many columns are kept (an eighth of the budget): the
/// segments scans are reading. Above it, up to [`segment_room`], it keeps what the columns leave.
pub(crate) fn segment_floor(budget: usize) -> usize {
    (budget / 8).max(1)
}

/// A size for an error message: MB, or bytes below one MB.
pub(crate) fn size_text(bytes: usize) -> String {
    if bytes < (1 << 20) {
        format!("{bytes} bytes")
    } else {
        format!("{:.1} MB", bytes as f64 / (1u64 << 20) as f64)
    }
}

/// The error of a column that cannot fit in the budget.
pub(crate) fn too_large(table: &str, column: &str, bytes: usize, budget: usize) -> Miss {
    let mb = |b: usize| b as f64 / (1u64 << 20) as f64;
    Miss::Error(format!(
        "column '{table}'[{column}] needs at least {:.1} MB in memory, over the memory budget of {:.1} MB \
         (BICRAFT_MEMORY_BUDGET or dax.memory)",
        mb(bytes),
        mb(budget)
    ))
}

/// What one pass of [`enforce`] may drop, besides the columns.
pub(crate) struct Kept<'a> {
    pub columns: &'a mut HashMap<(String, String), Result<Arc<ColData>, Miss>>,
    /// Bytes of the storage engine's caches, and a way to drop them.
    pub cache_bytes: usize,
    pub drop_cache: &'a mut dyn FnMut(),
    /// Bytes set apart for the segments of columns read segment by segment: the columns keep to
    /// the budget minus these.
    pub reserved: usize,
    /// References the relationship indexes hold to a column (by address), and a way to forget
    /// them.
    pub soft_refs: &'a dyn Fn(usize) -> usize,
    pub forget: &'a mut dyn FnMut(usize),
}

/// Holds what the session keeps under the budget: storage caches first (when over an eighth of
/// the budget; a smaller one goes last), then the unpacked codes of columns not in use, then the
/// columns not in use in [`Memory::eviction_order`] (a column a query holds is pinned by that
/// reference). Returns the bytes counted afterwards.
pub(crate) fn enforce(m: &mut Memory, k: Kept<'_>, target: Option<usize>) -> usize {
    let Some(budget) = target.or_else(|| m.budget()) else {
        return 0;
    };
    m.sync_meta(k.columns);
    let peak_extra = k.reserved;
    let budget = budget.saturating_sub(k.reserved);
    let mut cols: usize = k
        .columns
        .iter()
        .filter_map(|(key, c)| c.as_ref().ok().map(|c| m.bytes_of(key, c)))
        .sum();
    let total = cols.saturating_add(k.cache_bytes);
    m.peak_counted = m.peak_counted.max(total.saturating_add(peak_extra));
    if total <= budget {
        return total;
    }
    // A small storage cache stays while columns can give way: it answers a repeated query
    // without reading any column again, which costs far less than it keeps. A large one goes
    // first, as before.
    let mut cache = k.cache_bytes;
    let small_cache = cache <= budget / SMALL_CACHE_SHARE;
    if cache > 0 && !small_cache {
        (k.drop_cache)();
        #[cfg(test)]
        m.evicted_log.push(format!("cache {cache}"));
        m.cache_drops = m.cache_drops.saturating_add(1);
        cache = 0;
    }
    let room = budget.saturating_sub(cache);
    if cols <= room {
        return cols.saturating_add(cache);
    }
    let order = m.eviction_order();
    // unpacked codes of columns nobody else holds
    for key in &order {
        if cols <= room {
            return cols.saturating_add(cache);
        }
        if let Some(Ok(arc)) = k.columns.get_mut(key)
            && let Some(c) = Arc::get_mut(arc)
            && let Codes::Packed { plain, .. } = &mut c.codes
            && let Some(v) = plain.take()
        {
            cols = cols.saturating_sub(v.len().saturating_mul(4));
            #[cfg(test)]
            m.evicted_log.push(format!("plain {}.{}", key.0, key.1));
            m.plain_dropped = m.plain_dropped.saturating_add(1);
        }
    }
    // whole columns, in eviction order (fewest uses, then most recently used, first)
    for key in &order {
        if cols <= room {
            break;
        }
        let Some(Ok(arc)) = k.columns.get(key) else {
            continue;
        };
        let addr = Arc::as_ptr(arc) as usize;
        if Arc::strong_count(arc) > 1usize.saturating_add((k.soft_refs)(addr)) {
            continue; // in use
        }
        let bytes = m.bytes_of(key, arc);
        (k.forget)(addr);
        k.columns.remove(key);
        m.meta.remove(key);
        cols = cols.saturating_sub(bytes);
        #[cfg(test)]
        m.evicted_log.push(format!("column {}.{} {bytes}", key.0, key.1));
        m.evictions = m.evictions.saturating_add(1);
    }
    if cache > 0 && cols.saturating_add(cache) > budget {
        // the columns in use do not leave room for it
        (k.drop_cache)();
        #[cfg(test)]
        m.evicted_log
            .push(format!("cache-late {cache} cols {cols} budget {budget}"));
        m.cache_drops = m.cache_drops.saturating_add(1);
        cache = 0;
    }
    cols.saturating_add(cache)
}

/// `dax.memory`'s report.
pub(crate) fn report(
    m: &Memory,
    columns: &HashMap<(String, String), Result<Arc<ColData>, Miss>>,
    cache_bytes: usize,
    backup: Option<(&'static str, usize)>,
    segs: Option<&bicraft_scan::lazy::SegCache>,
) -> Value {
    let (mut n, mut bytes, mut pinned, mut largest, mut lazy) = (0usize, 0usize, 0usize, 0usize, 0usize);
    for (key, c) in columns {
        if let Ok(c) = c {
            n += 1;
            if c.codes.as_packed().is_some_and(bicraft_scan::Packed::is_lazy) {
                lazy += 1;
            }
            let b = m.bytes_of(key, c);
            largest = largest.max(b);
            bytes = bytes.saturating_add(b);
            if Arc::strong_count(c) > 1 {
                pinned += 1;
            }
        }
    }
    json!({
        "budget": m.budget(),
        "budget_source": m.source(),
        "counted_bytes": bytes.saturating_add(cache_bytes),
        "peak_counted_bytes": m.peak_counted,
        "columns": n,
        "column_bytes": bytes,
        "largest_column_bytes": largest,
        "columns_held_elsewhere": pinned,
        "storage_cache_bytes": cache_bytes,
        "backup": backup.map(|(w, b)| json!({"where": w, "bytes": b})),
        "spill_dir": m.budget().map(|_| spill_dir().display().to_string()),
        "spill_error": m.spill_error.lock().ok().and_then(|g| g.clone()),
        "evictions": m.evictions,
        "plain_dropped": m.plain_dropped,
        "cache_drops": m.cache_drops,
        "refused": m.refused,
        "lazy_columns": lazy,
        "lazy_reads": m.lazy_reads,
        "column_reads": m.column_reads,
        "segment_cache": segs.map(|c| {
            let (loads, hits, evictions) = c.counters();
            json!({
                "bytes": c.bytes(),
                "protected_bytes": c.protected_bytes(),
                "cap": c.cap(),
                "segments": c.len(),
                "loads": loads,
                "hits": hits,
                "shared": c.shared(),
                "evictions": evictions
            })
        }),
        "segment_load_failures": bicraft_scan::lazy::load_failures(),
        "query": {
            "cap": m.query_cap(),
            "cap_source": m.query_source(),
            "last_peak_bytes": m.query_last_peak,
            "peak_bytes": m.query_peak,
            "refused": m.query_refused,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::parse_budget;

    #[test]
    fn budgets_parse_with_binary_suffixes() {
        assert_eq!(parse_budget("300M"), Ok(Some(300 << 20)));
        assert_eq!(parse_budget(" 2g "), Ok(Some(2 << 30)));
        assert_eq!(parse_budget("1_000"), Ok(Some(1000)));
        assert_eq!(parse_budget("64KiB"), Ok(Some(64 << 10)));
        assert_eq!(parse_budget("1MB"), Ok(Some(1 << 20)));
        assert_eq!(parse_budget("0"), Ok(None));
        assert_eq!(parse_budget("unlimited"), Ok(None));
        assert_eq!(parse_budget(""), Ok(None));
        assert!(parse_budget("lots").is_err());
        assert!(parse_budget("-5M").is_err());
        assert!(parse_budget("99999999999T").is_err());
        assert!(parse_budget("M").is_err());
    }
}
