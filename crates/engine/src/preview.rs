//! Data preview for the live viewer (M8.2): a small evaluated result per visual
//! (`visual.data` in `docs/view-snapshot.md`).
//!
//! For each visual: group by the bound columns, aggregate the bound aggregations, and evaluate
//! the bound measures when their expression is a simple aggregation of a column (`SUM`,
//! `AVERAGE`, `MIN`, `MAX`, `COUNT`, `COUNTROWS`, `DISTINCTCOUNT`, …), `DIVIDE`, `BLANK()`,
//! `VAR`/`RETURN`, references to other such measures, and arithmetic between them and constants.
//! Visuals this fast path cannot express go to the general evaluator ([`general`]: filter
//! context, `CALCULATE`, time intelligence, iterators); what neither evaluates is `unsupported`,
//! with the reason.
//!
//! Grouping by a column of another table follows active many-to-one (or one-to-one)
//! relationships from the aggregated table, up to [`MAX_HOPS`] hops; any other shape is
//! `unsupported`. Report, page and visual filters and the page's slicer selections are applied
//! ([`filters`]): column conditions and top N filters through the general evaluator's filter
//! context (the fast path reads the rows they leave), measure conditions on the visual's rows.
//! Filters that cannot be applied are named in `filters_ignored_reason`. Categories sort by
//! their sort-by column; conditional formatting is evaluated against the rows ([`format`]).
//!
//! Table data comes from (in order) the PBIX `DataModel` or the PBIP `.pbi/cache.abf`
//! (VertiPaq columns decoded lazily, see [`store`]), then literal partitions
//! (`bicraft_data::load_partition`); otherwise `unavailable`.
//!
//! Everything is cached in the session ([`PreviewCache`]): the decompressed backup and decoded
//! columns until the source data changes, and results keyed by (data generation, model hash,
//! visual type + fields + query). A snapshot computes missing results within a time budget,
//! active page first, then pages that changed since the last snapshot, then the rest; visuals
//! left over get `data: null` and are computed by a later snapshot.

#[path = "preview_store.rs"]
mod store;

#[path = "preview_memory.rs"]
mod memory;

#[path = "preview_eval.rs"]
mod general;

#[path = "preview_currency.rs"]
mod currency;

#[path = "preview_filters.rs"]
pub(crate) mod filters;

#[path = "preview_records.rs"]
pub(crate) mod records;

#[path = "preview_format.rs"]
mod format;

#[path = "preview_field_params.rs"]
mod field_params;

#[path = "preview_matrix.rs"]
mod matrix;

#[path = "preview_show.rs"]
pub(crate) mod show;

#[path = "preview_viscalc.rs"]
mod viscalc;

#[path = "preview_decomp.rs"]
mod decomp;

#[path = "preview_sparklines.rs"]
mod sparks;
pub(crate) use decomp::{DECOMP_STATE, decomp_action, decomp_state};

#[path = "preview_fork.rs"]
mod fork;

#[path = "preview_rls.rs"]
mod rls;

#[path = "preview_key_influencers.rs"]
pub(crate) mod key_influencers;

#[path = "preview_perf.rs"]
pub(crate) mod perf;

#[path = "preview_rows.rs"]
pub(crate) mod rows;

pub(crate) use general::query::{DATA_UNAVAILABLE, dax_evaluate};
pub(crate) use general::structural::StructEval;

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use bicraft_dax::ast::Expr;
use bicraft_model::{Relationship, SemanticModel};
use bicraft_report::{FieldRef, Report, Visual};
use bicraft_vertipaq::CellValue;
use serde_json::{Map, Value, json};

use crate::refs::same_field;
use crate::{Session, Source};
use currency::Kind;
use filters::{Applied, Cond, Item, Operand};
use store::{ColData, Key, LiteralTable, Miss, VertipaqStore, literal_key, load_literal, model_table};

/// Most relationship hops between an aggregated table and a grouping column's table.
pub(crate) const MAX_HOPS: usize = 4;
/// Deepest measure-reference / expression nesting evaluated.
const MAX_DEPTH: usize = 48;
/// Largest decompressed backup kept in memory for the preview and DAX queries (the storage
/// engine scans the decoded codes, not the backup; 8 GiB holds a fact of several hundred million
/// rows).
const MAX_BACKUP_BYTES: usize = 8 << 30;
/// Results kept before the cache is cleared.
const MAX_CACHED_RESULTS: usize = 20_000;
/// Largest table (rows) the storage engine reads for the viewer's visuals, as for `dax.evaluate`
/// (codes and row numbers are 32-bit); memory is bounded by the session's memory limit
/// (`docs/beyond-ram.md`), not by a row count.
pub(crate) const STORAGE_TABLE_ROWS: usize = 2_000_000_000;
/// Most rows a calculated column the data does not hold is computed over ([`Ctx::computed`]),
/// and the longest time it may take.
const MAX_COMPUTED_ROWS: usize = 2_000_000;
const COMPUTED_BUDGET: std::time::Duration = std::time::Duration::from_secs(60);

/// Caps for the preview (from `SnapshotLimits`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct PreviewLimits {
    pub rows_per_visual: usize,
    /// Largest table (rows) read: the storage engine's limit for the viewer's visuals
    /// ([`STORAGE_TABLE_ROWS`]), the caller's own cap elsewhere.
    pub table_rows: usize,
    /// Largest table (rows) the row-by-row evaluator reads ([`grid`]): a visual the storage
    /// engine cannot evaluate falls back to it only under this cap
    /// (`SnapshotLimits::data_table_rows` in the viewer).
    pub fallback_rows: usize,
    pub budget: Duration,
    /// What `TODAY()`/`NOW()` return: days since 1899-12-30 ([`filters::clock`]), the local
    /// time of Desktop's machine.
    pub now: f64,
    /// Local time minus UTC, in days: `UTCNOW()` is `now - utc_offset` (the caller's
    /// `utc_offset_minutes`, else the process's local clock, `crate::clock`).
    pub utc_offset: f64,
}

/// A session's state, which other sessions may ask to free memory (`bicraft_mem::Evictor`).
#[derive(Default)]
struct StateCell(Mutex<State>);

impl bicraft_mem::Evictor for StateCell {
    /// Frees what the session keeps (storage caches, then its least recently used columns not in
    /// use, then its segment cache) when its state is not busy; never waits.
    fn evict(&self, want: usize) -> usize {
        match self.0.try_lock() {
            Ok(mut st) => st.evict_kept(want),
            Err(_) => 0,
        }
    }
}

/// Per-session preview cache. Interior mutability: `Session::snapshot` takes `&self`.
#[derive(Default)]
pub struct PreviewCache {
    /// Shared with the memory pool's evictors (a weak reference: lot 5).
    state: Arc<StateCell>,
    /// The state is registered as an evictor with its pool.
    registered: AtomicBool,
    /// [`background_work`] has something to do (a query wanted aggregations, `dax.aggregations`
    /// ran, or the session changed while it has aggregations): without it, background work
    /// neither locks nor synchronizes.
    pending: AtomicBool,
    /// The session has aggregations (planned, built, wanted or being built): a new revision
    /// makes background work pending (the report is derived again off the query path).
    has_aggs: AtomicBool,
    /// What forks start from, and their merge-backs waiting for the main state (`preview_fork.rs`).
    seed: Mutex<Option<Arc<fork::Seed>>>,
    seeded_cv: std::sync::Condvar,
    /// Notified when an evaluation leaves the main state (`preview_fork.rs`, [`fork::FORK_AFTER`]).
    freed: (Mutex<()>, std::sync::Condvar),
    harvests: Mutex<Vec<fork::Harvest>>,
    /// Stored columns being read now, by (data generation, table, column): a concurrent
    /// evaluation waits for the read instead of making a second copy.
    loading: Mutex<HashMap<(u64, String, String), Arc<fork::Loading>>>,
    /// Forks made, harvests merged, harvests dropped as stale.
    forks: std::sync::atomic::AtomicU64,
    merged: std::sync::atomic::AtomicU64,
    stale: std::sync::atomic::AtomicU64,
    /// Column reads taken from a concurrent evaluation instead of read again.
    dedups: std::sync::atomic::AtomicU64,
    /// The state of View as evaluations (`preview_rls.rs`): its own caches, over the rows the
    /// roles keep.
    secured: Mutex<rls::SecuredSlot>,
    /// Refresh visuals (`report.refresh_visuals`, `optimize.rs`): the next snapshot evaluates the
    /// visuals of a paused report once.
    refresh: AtomicBool,
    /// The data each visual last showed, by (page, visual): what a paused report keeps showing.
    shown: Mutex<HashMap<(String, String), Value>>,
    /// Tests: times background work took the lock.
    #[cfg(test)]
    background_locks: std::sync::atomic::AtomicU64,
}

impl std::fmt::Debug for PreviewCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewCache").finish_non_exhaustive()
    }
}

impl PreviewCache {
    /// The memory pool this session reserves against (the process' unless a test gave it one).
    pub(crate) fn memory_pool(&self) -> &'static bicraft_mem::Pool {
        self.lock().memory.pool()
    }

    /// Tests: reserve against `pool` instead of the process' pool.
    #[cfg(test)]
    pub(crate) fn set_memory_pool(&self, pool: &'static bicraft_mem::Pool) {
        self.lock().memory.pool = Some(pool);
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        match self.state.0.lock() {
            Ok(g) => self.adopt(g),
            Err(poisoned) => self.adopt_poisoned(poisoned.into_inner()),
        }
    }

    /// The main state after a panic while computing: start over from an empty cache.
    fn adopt_poisoned<'a>(&'a self, mut g: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        *g = State::default();
        // a new state counts its data generations from zero again: reads listed under the old
        // numbers must not match
        self.loading
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.state.0.clear_poison();
        self.adopt(g)
    }

    /// The main state just locked: registered with its memory pool when there is a limit.
    fn adopt<'a>(&'a self, g: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        // under a memory limit, other sessions may ask this one to free what it keeps
        if g.memory.limit().is_some() && !self.registered.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let weak: std::sync::Weak<dyn bicraft_mem::Evictor> =
                Arc::downgrade(&(Arc::clone(&self.state) as Arc<dyn bicraft_mem::Evictor>));
            g.memory.pool().register(weak);
        }
        g
    }

    /// Hands over the backup `file.open` just decompressed from the open package's `DataModel`
    /// part (to read its model), so the first query reads the stored data from it instead of
    /// decompressing the part again. Kept with the identity of the part it came from
    /// ([`part_identity`]): used once, only while the session's `DataModel` is still that part.
    pub(crate) fn seed_backup(&self, part: &bicraft_opc::Part, abf: Vec<u8>) {
        self.lock().seeded = Some((part_identity(part), Seed::Backup(abf)));
    }

    /// Hands over the store `file.open` opened under a memory budget: its backup already in a
    /// temporary file, decompressed there as a stream (`docs/beyond-ram.md`, lot 2).
    pub(crate) fn seed_store(&self, part: &bicraft_opc::Part, store: VertipaqStore) {
        self.lock().seeded = Some((part_identity(part), Seed::Store(store)));
    }

    /// The memory budget a query would run under now (the session's setting, else
    /// `BICRAFT_MEMORY_BUDGET`).
    pub(crate) fn budget(&self) -> Option<usize> {
        self.lock().memory.budget()
    }

    /// Refresh visuals: the next snapshot evaluates a paused report's visuals once.
    pub(crate) fn request_refresh(&self) {
        self.refresh.store(true, Ordering::Release);
    }

    /// Whether a refresh was asked for since the last call (and takes it).
    pub(crate) fn take_refresh(&self) -> bool {
        self.refresh.swap(false, Ordering::AcqRel)
    }

    fn shown(&self) -> MutexGuard<'_, HashMap<(String, String), Value>> {
        self.shown
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Background work has something to do.
    pub(crate) fn set_pending(&self) {
        self.pending.store(true, Ordering::Release);
    }

    /// The session has aggregations: they follow its revisions (see [`PreviewCache::revised`]).
    fn set_has_aggs(&self) {
        self.has_aggs.store(true, Ordering::Release);
    }

    /// The session changed (a new revision): background work is pending only when it has
    /// aggregations to follow.
    pub(crate) fn revised(&self) {
        if self.has_aggs.load(Ordering::Acquire) {
            self.set_pending();
        }
    }
}

/// A computed calculated column: the (model hash, data generation) it was computed at, and its
/// values.
type Computed = ((u64, u64), Result<Arc<ColData>, Miss>);

#[derive(Default)]
struct State {
    /// Session revision the cache was last synchronized with.
    revision: Option<u64>,
    /// Fingerprint of the source's stored data (DataModel / cache.abf).
    source_fp: Option<u64>,
    /// Bumped whenever the stored data changes.
    generation: u64,
    /// The opened backup: `None` until first needed.
    store: Option<Result<Arc<VertipaqStore>, String>>,
    /// The backup `file.open` decompressed, with the identity of the `DataModel` part it
    /// came from ([`PreviewCache::seed_backup`]): the store opens from it once.
    seeded: Option<((u64, usize, usize), Seed)>,
    /// bicraft's own data snapshot of a PBIP project (`crate::refresh`): `None` until first
    /// needed, then the snapshot when it is the data to read.
    own: Option<Option<Arc<crate::refresh::OwnData>>>,
    /// Decoded backup columns, by lower-cased (table, column).
    columns: HashMap<(String, String), Result<Arc<ColData>, Miss>>,
    /// Row counts of stored tables given new data by a structural save or a PBIX refresh
    /// (`StructEval::provide`), by lower-cased name: they override the stored counts.
    provided_rows: HashMap<String, usize>,
    /// Literal tables by lower-cased name: (fingerprint, data).
    literal: HashMap<String, (u64, Result<Arc<LiteralTable>, Miss>)>,
    /// Calculated columns being computed now ([`Ctx::computed`]), by lower-cased (table,
    /// column): a column that reads itself again fails instead of recursing.
    computing: std::collections::HashSet<(String, String)>,
    /// Calculated columns computed by [`Ctx::computed`], by lower-cased (table, column): (the
    /// model hash and data generation they were computed at, their values).
    computed: HashMap<(String, String), Computed>,
    /// Fingerprint of what every loaded partition depends on besides its own definition.
    literal_inputs: Option<u64>,
    /// Hash of the model, computed on first use after each synchronization ([`State::model_hash`]).
    model_hash: Option<u64>,
    results: HashMap<u64, Value>,
    /// The storage engine's caches kept between evaluations ([`general::storage::Store`]).
    storage: Option<general::storage::Store>,
    /// Relationship code maps and blank-key flags (`preview_eval_rels.rs`), kept by
    /// `dax.clear_cache`, dropped with the model or data they were built from.
    rel_indexes: general::rels::RelIndexes,
    /// What evaluations read from the text of the session's measures, for one model hash.
    measure_facts: Option<general::rels::MeasureFacts>,
    /// The shape of the session's model as evaluations read it, for one model hash
    /// ([`general::Registry`]).
    registry: Option<Arc<general::Registry>>,
    /// The report-derived aggregations (`preview_eval_aggs.rs`), kept by `dax.clear_cache`.
    aggs: Option<general::storage::aggs::AggState>,
    /// The session's aggregation settings (`dax.aggregations {enabled}`; tests set the others):
    /// each `None` falls back to the process' environment, read once.
    aggs_config: general::storage::aggs::AggConfig,
    /// Tests: synchronizations that did work, and model hashes computed.
    #[cfg(test)]
    syncs: u64,
    #[cfg(test)]
    model_hashes: u64,
    /// Evaluations in flight and the end of the last one: background aggregation jobs wait for
    /// the session to be idle.
    activity: Arc<general::storage::aggs::Activity>,
    /// Page name → hash of its visuals at the last snapshot.
    page_hashes: HashMap<String, u64>,
    /// The memory budget's accounting (`docs/beyond-ram.md`).
    memory: memory::Memory,
    /// Segments of the columns read segment by segment under the budget (lot 3), kept within a
    /// quarter of it.
    segs: Option<Arc<bicraft_scan::lazy::SegCache>>,
    /// Its decoded columns, reserved against the process' memory limit (lot 5; the segment cache,
    /// the storage caches and the aggregations reserve their own).
    kept_res: bicraft_mem::Reservation,
    /// The aggregations' tables, reserved against the limit.
    aggs_res: bicraft_mem::Reservation,
    /// An evaluation runs on this state (its reservations count what it adds to the columns).
    in_query: bool,
    /// A fork's columns inherited from the main state (`preview_fork.rs`); `None` on the main state.
    inherited: Option<fork::Inherited>,
    /// The Data view's row orders and computed columns (`table.rows`, `preview_rows.rs`).
    rows_kept: rows::Kept,
    /// View as: the rows each secured table keeps (`preview_rls.rs`); `None` on the main state
    /// and forks.
    security: Option<Arc<rls::Security>>,
    /// View as: the secured tables' columns, by lower-cased (table, column).
    secured_columns: HashMap<(String, String), Result<Arc<ColData>, Miss>>,
    /// What `USERPRINCIPALNAME()` and `USERNAME()` return (`None`: they are not evaluated): set
    /// under View as.
    user: Option<String>,
}

fn hash_of(parts: &[&[u8]]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in parts {
        p.hash(&mut h);
    }
    h.finish()
}

/// What `file.open` hands over for the first query ([`PreviewCache::seed_backup`],
/// [`PreviewCache::seed_store`]).
enum Seed {
    Backup(Vec<u8>),
    Store(VertipaqStore),
}

/// Identity of a `DataModel` part for [`PreviewCache::seed_backup`]: its fingerprint
/// ([`part_fingerprint`]), its length and where its bytes are (the address of the part still in
/// the package, not a copy; the offset in the file of a part left there).
fn part_identity(part: &bicraft_opc::Part) -> (u64, usize, usize) {
    let at = match &part.deferred {
        Some(d) => usize::try_from(d.data_offset).unwrap_or(usize::MAX),
        None => part.data.as_ptr() as usize,
    };
    (
        part_fingerprint(part),
        usize::try_from(part.len()).unwrap_or(usize::MAX),
        at,
    )
}

/// Fingerprint of a part's bytes: sampled from memory, or the size and CRC-32 of a part left
/// in the file (its bytes are not read).
fn part_fingerprint(part: &bicraft_opc::Part) -> u64 {
    match &part.deferred {
        Some(d) => hash_of(&[b"deferred", &d.size.to_le_bytes(), &d.crc32.to_le_bytes()]),
        None => fingerprint_bytes(&part.data),
    }
}

/// Hash of a byte buffer; big buffers are sampled (length, both ends and 4 KiB every MiB).
fn fingerprint_bytes(b: &[u8]) -> u64 {
    const FULL: usize = 8 << 20;
    if b.len() <= FULL {
        return hash_of(&[b]);
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    b.len().hash(&mut h);
    b.get(..65_536).hash(&mut h);
    b.get(b.len().saturating_sub(65_536)..).hash(&mut h);
    let mut at = 0usize;
    while at < b.len() {
        b.get(at..at.saturating_add(4096).min(b.len())).hash(&mut h);
        at = at.saturating_add(1 << 20);
    }
    h.finish()
}

/// The `cache.abf` of a PBIP project (the session's explicit `cache_abf` when set; `None` when it
/// cannot be read: [`crate::cache_abf::path`] has the error).
fn cache_abf(s: &Session) -> Option<PathBuf> {
    crate::cache_abf::path(s).ok().flatten()
}

/// A store whose backup is decompressed as a stream from what `open` reads to a temporary file
/// in the spill folder, and the backup's `metadata.sqlitedb` (`docs/beyond-ram.md`, lot 2).
pub(crate) fn open_streamed<'a>(
    open: &mut dyn FnMut() -> Result<Box<dyn std::io::Read + Send + 'a>, String>,
) -> Result<(VertipaqStore, Vec<u8>), String> {
    VertipaqStore::open_streamed(open, &memory::spill_dir(), MAX_BACKUP_BYTES)
}

/// The stored data of the open file (a PBIX's `DataModel`, a PBIP's `cache.abf`) for the DAX
/// evaluator outside the viewer (`table.import {dax}`); `None` when there is none or it is too
/// large to read.
pub(crate) fn stored_backup(s: &Session) -> Option<std::borrow::Cow<'_, [u8]>> {
    match &s.source {
        Some(Source::Package { package, .. }) => package
            .get(bicraft_pbix::DATA_MODEL)
            .filter(|p| {
                p.len() <= bicraft_opc::MAX_PART_BYTES
                    && package.get(bicraft_pbix::DATA_MODEL_SCHEMA).is_none()
            })
            .and_then(|p| p.bytes().ok()),
        Some(Source::Project { .. }) => {
            let p = cache_abf(s)?;
            let len = std::fs::metadata(&p).ok()?.len();
            if len > MAX_BACKUP_BYTES as u64 {
                return None;
            }
            std::fs::read(&p).ok().map(std::borrow::Cow::Owned)
        }
        _ => None,
    }
}

/// The values of one column of the open model, one per row, for edit-time data checks
/// (`relationship.set` cardinalities, date tables): the stored data (PBIX `DataModel`, PBIP
/// `cache.abf`) or the table's literal partitions. `Ok(None)` when the session has no data for
/// it; tables over `max_rows` rows are an error.
pub(crate) fn column_values(
    s: &Session,
    table: &str,
    column: &str,
    max_rows: usize,
) -> Result<Option<Vec<CellValue>>, String> {
    let Some(model) = s.model.as_ref() else {
        return Ok(None);
    };
    // edit-time checks read the data as stored, never through View as
    let mut st = s.preview.acquire_raw(s);
    let limits = PreviewLimits {
        rows_per_visual: 0,
        table_rows: max_rows,
        fallback_rows: max_rows,
        budget: Duration::from_secs(60),
        now: filters::clock(s),
        utc_offset: filters::utc_offset(s),
    };
    let mut ctx = Ctx {
        s,
        st: &mut st,
        model,
        limits: &limits,
    };
    match ctx.column(table, column) {
        Ok(c) => Ok(Some(
            c.codes
                .iter()
                .map(|&k| {
                    usize::try_from(k)
                        .ok()
                        .and_then(|k| c.values.get(k))
                        .cloned()
                        .unwrap_or(CellValue::Null)
                })
                .collect(),
        )),
        Err(Miss::Unavailable(_)) => Ok(None),
        Err(Miss::Error(e)) => Err(e),
    }
}

/// The distinct values of one column of the open model (only those some row uses), as JSON,
/// sorted: blank first, then numbers, dates and text (text case-insensitively). `Ok(None)` when
/// the session has no data for it; tables over `max_rows` rows are an error.
pub(crate) fn distinct_values(
    s: &Session,
    table: &str,
    column: &str,
    max_rows: usize,
) -> Result<Option<Vec<Value>>, String> {
    let Some(model) = s.model.as_ref() else {
        return Ok(None);
    };
    let mut st = s.preview.acquire(s);
    let limits = PreviewLimits {
        rows_per_visual: 0,
        table_rows: max_rows,
        fallback_rows: max_rows,
        budget: Duration::from_secs(60),
        now: filters::clock(s),
        utc_offset: filters::utc_offset(s),
    };
    let mut ctx = Ctx {
        s,
        st: &mut st,
        model,
        limits: &limits,
    };
    let c = match ctx.column(table, column) {
        Ok(c) => c,
        Err(Miss::Unavailable(_)) => return Ok(None),
        Err(Miss::Error(e)) => return Err(e),
    };
    let mut used = vec![false; c.values.len()];
    let mut blank = false;
    for &k in c.codes.iter() {
        match usize::try_from(k).ok().and_then(|k| used.get_mut(k)) {
            Some(u) => *u = true,
            None => blank = true,
        }
    }
    let mut vals: Vec<&CellValue> = c
        .values
        .iter()
        .zip(&used)
        .filter(|(_, u)| **u)
        .map(|(v, _)| v)
        .collect();
    let rank = |v: &CellValue| match v {
        CellValue::Null => 0u8,
        CellValue::Bool(_) => 1,
        CellValue::Int(_) | CellValue::Real(_) | CellValue::Decimal(_) => 2,
        CellValue::DateTime(_) => 3,
        CellValue::Text(_) => 4,
    };
    let num = |v: &CellValue| match v {
        CellValue::Int(n) => *n as f64,
        CellValue::Real(x) | CellValue::DateTime(x) => *x,
        CellValue::Decimal(n) => *n as f64 / 10_000.0,
        CellValue::Bool(b) => f64::from(u8::from(*b)),
        _ => 0.0,
    };
    vals.sort_by(|a, b| {
        rank(a).cmp(&rank(b)).then_with(|| match (a, b) {
            (CellValue::Text(x), CellValue::Text(y)) => {
                x.to_lowercase().cmp(&y.to_lowercase()).then_with(|| x.cmp(y))
            }
            _ => num(a).total_cmp(&num(b)),
        })
    });
    let mut out: Vec<Value> = Vec::with_capacity(vals.len().saturating_add(1));
    if blank && !vals.iter().any(|v| matches!(v, CellValue::Null)) {
        out.push(Value::Null);
    }
    for v in vals {
        let j = cell_json(v);
        if out.last() != Some(&j) {
            out.push(j);
        }
    }
    Ok(Some(out))
}

/// Most rows read for a range slicer's exact domain.
const MAX_DOMAIN_ROWS: usize = 50_000_000;

/// `{min, max}` of a numeric or date column over every row of the stored data (or the table's
/// literal partitions); `None` when there is no data, no numeric/date value, or on any error.
pub(crate) fn column_domain(s: &Session, table: &str, column: &str) -> Option<Value> {
    let model = s.model.as_ref()?;
    let mut st = s.preview.acquire(s);
    let limits = PreviewLimits {
        rows_per_visual: 0,
        table_rows: MAX_DOMAIN_ROWS,
        fallback_rows: MAX_DOMAIN_ROWS,
        budget: Duration::from_secs(30),
        now: filters::clock(s),
        utc_offset: filters::utc_offset(s),
    };
    let mut ctx = Ctx {
        s,
        st: &mut st,
        model,
        limits: &limits,
    };
    let c = ctx.column(table, column).ok()?;
    // only the dictionary values some row uses
    let mut used = vec![false; c.values.len()];
    for &k in c.codes.iter() {
        if let Some(u) = usize::try_from(k).ok().and_then(|k| used.get_mut(k)) {
            *u = true;
        }
    }
    let (mut lo, mut hi): (Option<f64>, Option<f64>) = (None, None);
    let mut date = false;
    for (v, _) in c.values.iter().zip(&used).filter(|(_, u)| **u) {
        let x = match v {
            CellValue::Int(n) => *n as f64,
            CellValue::Real(x) => *x,
            CellValue::Decimal(n) => *n as f64 / 10_000.0,
            CellValue::DateTime(x) => {
                date = true;
                *x
            }
            _ => continue,
        };
        if !x.is_finite() {
            continue;
        }
        lo = Some(lo.map_or(x, |l| l.min(x)));
        hi = Some(hi.map_or(x, |h| h.max(x)));
    }
    let (lo, hi) = (lo?, hi?);
    let j = |x: f64| {
        if date {
            ole_to_iso(x).map_or(Value::Null, Value::String)
        } else {
            num_json(x)
        }
    };
    Some(json!({"min": j(lo), "max": j(hi)}))
}

/// Puts decoded columns in the preview cache as the data of `table` (as if loaded from its
/// partitions), for tests and micro-benchmarks on synthetic data built in memory. The session
/// must not change afterwards (a model edit reloads the partitions).
#[cfg(test)]
pub(crate) fn inject_table(s: &Session, table: &str, rows: usize, columns: Vec<(String, ColData)>) {
    let Some(t) = s.model.as_ref().and_then(|m| model_table(m, table)) else {
        return;
    };
    let mut st = s.preview.lock();
    st.sync(s);
    let columns = columns
        .into_iter()
        .map(|(n, c)| (n.to_lowercase(), Arc::new(c)))
        .collect();
    st.literal.insert(
        t.name.to_lowercase(),
        (literal_key(t), Ok(Arc::new(LiteralTable { rows, columns }))),
    );
    // new data: what was computed from the old one is not reused
    st.generation = st.generation.wrapping_add(1);
    st.results.clear();
}

/// `dax.clear_cache`: drops what queries keep between them (the per-code tables derived from
/// the decoded columns, the viewer's results), keeping the opened data and the decoded columns:
/// the next query runs as on a cold Analysis Services engine whose data is in memory
/// (`ClearCache`).
pub(crate) fn clear_cache(s: &mut Session, p: &Value) -> Result<Value, String> {
    crate::params::allow(p, &[])?;
    let mut st = s.preview.lock();
    let mut n = 0usize;
    for c in st.columns.values().flatten() {
        c.derived.clear();
        n = n.saturating_add(1);
    }
    for (_, t) in st.literal.values() {
        if let Ok(t) = t {
            for c in t.columns.values() {
                c.derived.clear();
                n = n.saturating_add(1);
            }
        }
    }
    let results = st.results.len();
    st.results.clear();
    st.storage = None;
    Ok(json!({"columns": n, "results": results}))
}

/// Spill files of this process alive now (`crate::obs`).
pub(crate) fn spill_files_alive() -> usize {
    store::SPILLS_ALIVE.load(Ordering::Relaxed)
}

/// Work that waits for a command's answer to be sent (`Session::background_work`): background
/// aggregation jobs the last queries asked for.
/// Nothing is done (no lock, no synchronization, no model hash) unless something made it
/// pending ([`PreviewCache::set_pending`]).
pub(crate) fn background_work(s: &Session) {
    if !s.preview.pending.swap(false, Ordering::AcqRel) {
        return;
    }
    #[cfg(test)]
    s.preview
        .background_locks
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // an evaluation holds the state (reads run side by side on a server): it does the work once
    // its own answer is sent; this call does not wait for it
    let mut st = match s.preview.state.0.try_lock() {
        Ok(g) => s.preview.adopt(g),
        Err(std::sync::TryLockError::Poisoned(p)) => s.preview.adopt_poisoned(p.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => {
            s.preview.set_pending();
            return;
        }
    };
    // the backup of a store opened under a memory budget goes to its file off the query path
    if st.memory.spill_pending {
        st.memory.spill_pending = false;
        if let Some(Ok(store)) = &st.store {
            let store = Arc::clone(store);
            let err = Arc::clone(&st.memory.spill_error);
            let spawned = std::thread::Builder::new()
                .name("bicraft-spill".into())
                .spawn(move || {
                    let e = store.spill(&memory::spill_dir()).err();
                    if let Ok(mut g) = err.lock() {
                        *g = e;
                    }
                });
            if spawned.is_err() {
                st.spill_now();
            }
        }
    }
    if !general::storage::aggs::jobs_due(s, &st) {
        return;
    }
    st.sync(s);
    general::storage::aggs::start_jobs(s, &mut st);
    st.sync_aggs_res();
}

/// Tests: (background locks, synchronizations that did work, model hashes computed).
#[cfg(test)]
pub(crate) fn work_counters(s: &Session) -> (u64, u64, u64) {
    let locks = s
        .preview
        .background_locks
        .load(std::sync::atomic::Ordering::Relaxed);
    let st = s.preview.lock();
    (locks, st.syncs, st.model_hashes)
}

/// Tests: the session's aggregation settings (`None`: the process' environment).
#[cfg(test)]
pub(crate) fn set_aggregation_config(s: &Session, config: general::storage::aggs::AggConfig) {
    s.preview.lock().aggs_config = config;
}

/// `dax.aggregations`: the report-derived aggregations (`docs/storage-engine.md`, "Aggregations"):
/// turns them on or off for the session, builds them now, and reports what was derived and built.
pub(crate) fn dax_aggregations(s: &Session, p: &Value) -> Result<Value, String> {
    crate::params::allow(p, &["enabled", "build"])?;
    let enabled = crate::params::opt_bool(p, "enabled")?;
    let build = crate::params::opt_bool(p, "build")?.unwrap_or(false);
    let s: &Session = s;
    let mut st = s.preview.lock();
    st.sync(s);
    if let Some(on) = enabled {
        st.aggs_config.enabled = Some(on);
    }
    let on = general::storage::aggs::enabled(&st.aggs_config);
    // the aggregations follow the session from now on; background work reads the report again
    s.preview.set_has_aggs();
    s.preview.set_pending();
    if build && on {
        let Some(model) = s.model.as_ref() else {
            return Err("no model is open (file.open first)".into());
        };
        let limits = PreviewLimits {
            rows_per_visual: 0,
            table_rows: usize::MAX,
            fallback_rows: usize::MAX,
            budget: Duration::from_secs(3600),
            now: filters::clock(s),
            utc_offset: filters::utc_offset(s),
        };
        let mut ctx = Ctx {
            s,
            st: &mut st,
            model,
            limits: &limits,
        };
        general::storage::aggs::build(&mut ctx)?;
    }
    let mut out = st
        .aggs
        .as_ref()
        .map_or_else(|| json!({"facts": [], "visuals": []}), |a| a.report());
    if let Some(o) = out.as_object_mut() {
        o.insert("enabled".into(), json!(on));
        // what the report needs now (the aggregations built follow it at the next query)
        if let (Some(m), Some(r)) = (s.model.as_ref(), s.report.as_ref()) {
            let d = crate::aggs_derive::derive(m, r);
            o.insert(
                "visuals".into(),
                json!(d.visuals.iter().map(|v| v.to_json()).collect::<Vec<_>>()),
            );
        }
    }
    Ok(out)
}

/// Fingerprint of the source's stored data (0: none).
fn source_fingerprint(s: &Session) -> u64 {
    match &s.source {
        Some(Source::Package { package, .. }) => {
            package.get(bicraft_pbix::DATA_MODEL).map_or(0, part_fingerprint)
        }
        Some(Source::Project { project, .. }) => cache_abf(s)
            .into_iter()
            .chain(crate::refresh::readable_snapshot_path(project))
            .map(|p| {
                let m = std::fs::metadata(&p).ok()?;
                let modified = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                let mut h = std::collections::hash_map::DefaultHasher::new();
                p.hash(&mut h);
                m.len().hash(&mut h);
                modified.hash(&mut h);
                Some(h.finish())
            })
            .fold(0u64, |acc, h| acc.rotate_left(17) ^ h.unwrap_or(0)),
        _ => 0,
    }
}

/// Fingerprint of the data root, the model's culture and expressions and every table's
/// partitions (an M query may reference another table's query).
fn literal_inputs(s: &Session) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.data_root.hash(&mut h);
    if let Some(m) = &s.model {
        m.culture.hash(&mut h);
        m.extra
            .get("sourceQueryCulture")
            .map(|v| v.to_string())
            .hash(&mut h);
        for e in &m.expressions {
            e.name.hash(&mut h);
            e.expression.hash(&mut h);
        }
        for t in &m.tables {
            t.name.hash(&mut h);
            for p in &t.partitions {
                p.source_type.hash(&mut h);
                p.expression.hash(&mut h);
            }
        }
    }
    h.finish()
}

impl State {
    fn sync(&mut self, s: &Session) {
        if self.revision == Some(s.revision) {
            return;
        }
        let fp = source_fingerprint(s);
        if self.source_fp != Some(fp) {
            self.source_fp = Some(fp);
            if self
                .seeded
                .as_ref()
                .is_some_and(|((seed_fp, _, _), _)| *seed_fp != fp)
            {
                self.seeded = None;
            }
            self.generation = self.generation.wrapping_add(1);
            self.rel_indexes.clear();
            self.store = None;
            self.own = None;
            self.literal.clear();
            self.columns.clear();
            self.results.clear();
            self.rows_kept.clear();
        }
        #[cfg(test)]
        {
            self.syncs = self.syncs.saturating_add(1);
        }
        // computed when first read: a report edit does not hash the model until it is needed
        self.model_hash = None;
        // Loaded partitions also depend on the data root, the culture and the queries they
        // reference: reload them (and recompute results) when any of these change.
        let inputs = literal_inputs(s);
        if self.literal_inputs != Some(inputs) {
            self.literal_inputs = Some(inputs);
            self.literal.clear();
            self.rows_kept.clear();
            self.generation = self.generation.wrapping_add(1);
            self.rel_indexes.clear();
            self.results.clear();
        }
        self.revision = Some(s.revision);
        if self.results.len() > MAX_CACHED_RESULTS {
            self.results.clear();
        }
    }

    /// Hash of the session's model (part of the keys of every result kept), computed on first
    /// use after a synchronization: the model streamed into the hasher, never copied.
    fn model_hash(&mut self, s: &Session) -> u64 {
        if let Some(h) = self.model_hash {
            return h;
        }
        #[cfg(test)]
        {
            self.model_hashes = self.model_hashes.saturating_add(1);
        }
        let h = s.model.as_ref().map_or(0, |m| {
            let mut w = HashWriter(std::collections::hash_map::DefaultHasher::new());
            match serde_json::to_writer(&mut w, m) {
                Ok(()) => w.0.finish(),
                Err(_) => 0,
            }
        });
        self.model_hash = Some(h);
        h
    }

    /// The segment cache of the columns read segment by segment, bounded for `budget`.
    fn seg_cache(&mut self, budget: usize) -> Arc<bicraft_scan::lazy::SegCache> {
        let cap = memory::segment_room(budget);
        match &self.segs {
            Some(c) => {
                if c.cap() != cap {
                    c.set_cap(cap);
                }
                Arc::clone(c)
            }
            None => {
                let c = bicraft_scan::lazy::SegCache::new_in(cap, self.memory.pool());
                self.segs = Some(Arc::clone(&c));
                c
            }
        }
    }

    /// Moves the open store's backup to its temporary file now (under a memory budget).
    fn spill_now(&mut self) {
        self.memory.spill_pending = false;
        if let Some(Ok(store)) = &self.store {
            let e = store.spill(&memory::spill_dir()).err();
            if let Ok(mut g) = self.memory.spill_error.lock() {
                *g = e;
            }
        }
    }

    /// Holds what is kept under the memory budget, if there is one ([`memory::enforce`]).
    fn enforce_budget(&mut self) {
        let Some(budget) = self.memory.budget() else {
            return;
        };
        self.enforce_to(budget, None);
    }

    /// [`memory::enforce`] under `budget`, the columns held to `target` when given (eviction under
    /// the memory limit); then the reservation of the columns follows them.
    fn enforce_to(&mut self, budget: usize, target: Option<usize>) {
        self.enforce_core(budget, target);
        self.sync_kept();
    }

    /// [`State::enforce_to`] without the reservation's update.
    fn enforce_core(&mut self, budget: usize, target: Option<usize>) {
        let cache_bytes = self
            .storage
            .as_ref()
            .map_or(0, general::storage::Store::approx_bytes);
        // the segments read on demand keep up to half the budget, in what the columns leave: a
        // column evicted is read again whole, a segment alone; they always keep an eighth of it
        let reserved = self
            .segs
            .as_ref()
            .map_or(0, |c| c.bytes())
            .min(memory::segment_floor(budget));
        let mut drop_cache = false;
        let mut forgotten: Vec<usize> = Vec::new();
        let rels = &self.rel_indexes;
        let kept = memory::enforce(
            &mut self.memory,
            memory::Kept {
                columns: &mut self.columns,
                cache_bytes,
                drop_cache: &mut || drop_cache = true,
                reserved,
                soft_refs: &|addr| rels.refs(addr),
                forget: &mut |addr| forgotten.push(addr),
            },
            target.map(|t| t.saturating_add(reserved)),
        );
        if drop_cache {
            self.storage = None;
        }
        if let Some(c) = &self.segs {
            c.shrink_to(budget.saturating_sub(kept));
        }
        for addr in forgotten {
            self.rel_indexes.forget(addr);
        }
    }

    /// The reservation of the aggregations follows their tables (lot 5).
    fn sync_aggs_res(&mut self) {
        if self.memory.limit().is_none() && !self.aggs_res.is_bound() {
            return;
        }
        if !self.aggs_res.is_bound() {
            self.aggs_res = self.memory.pool().empty(self.memory.kept_class());
        }
        let bytes = self
            .aggs
            .as_ref()
            .map_or(0, general::storage::aggs::AggState::bytes);
        self.aggs_res.set(bytes);
    }

    /// The reservation of the decoded columns follows what is kept (lot 5).
    fn sync_kept(&mut self) {
        if self.memory.limit().is_none() && !self.kept_res.is_bound() {
            return;
        }
        if !self.kept_res.is_bound() {
            self.kept_res = self.memory.pool().empty(self.memory.kept_class());
        }
        // while a query runs, what it adds to the columns is counted by its own reservations
        let bytes = self.memory.column_bytes(&self.columns, !self.in_query);
        let have = self.kept_res.bytes();
        if bytes > have
            && self.memory.limit().is_some()
            && self.kept_res.try_grow(bytes.saturating_sub(have)).is_err()
        {
            // more than the limit leaves (what queries added to the columns, counted now): the least
            // recently used columns not in use go first
            let pool = self.memory.pool();
            let free = pool.limit().unwrap_or(0).saturating_sub(pool.stats().used);
            let budget = self.memory.budget().unwrap_or(usize::MAX);
            self.enforce_core(budget, Some(have.saturating_add(free)));
            let bytes = self.memory.column_bytes(&self.columns, !self.in_query);
            self.kept_res.set(bytes);
            return;
        }
        self.kept_res.set(bytes);
    }

    /// Frees about `want` bytes of what the session keeps, for a reservation that did not fit
    /// under the memory limit: the storage caches, then the least recently used columns no query
    /// holds, then the segment cache. Returns the bytes released from its reservations.
    fn evict_kept(&mut self, want: usize) -> usize {
        let before = self.reserved_bytes();
        // the Data view's row orders and computed columns go first (computed again on demand)
        let rows = self.rows_kept.bytes();
        self.rows_kept.clear();
        if rows >= want {
            return rows;
        }
        if let Some(s) = &mut self.storage {
            s.drop_caches();
        }
        if self.reserved_bytes().saturating_add(want) > before {
            let target = self.kept_res.bytes().saturating_sub(want);
            let budget = self.memory.budget().unwrap_or(usize::MAX);
            self.enforce_to(budget, Some(target));
        }
        let after = self.reserved_bytes();
        if before.saturating_sub(after) < want
            && let Some(c) = &self.segs
        {
            c.shrink_to(
                c.bytes()
                    .saturating_sub(want.saturating_sub(before.saturating_sub(after))),
            );
        }
        before.saturating_sub(self.reserved_bytes()).saturating_add(rows)
    }

    /// Bytes this state holds in its pool: columns, storage caches, segments, aggregations.
    fn reserved_bytes(&self) -> usize {
        self.kept_res
            .bytes()
            .saturating_add(self.storage.as_ref().map_or(0, |s| s.res.bytes()))
            .saturating_add(self.segs.as_ref().map_or(0, |c| c.reserved()))
            .saturating_add(self.aggs_res.bytes())
    }

    /// Reserves about `bytes` for a column about to be read (lot 5), freeing what the session (and
    /// then other sessions) keep when it does not fit. The reservation is held while the column is
    /// read; [`State::sync_kept`] then counts the column itself.
    fn admit_column(
        &mut self,
        table: &str,
        column: &str,
        bytes: usize,
    ) -> Result<bicraft_mem::Reservation, Miss> {
        let pool = self.memory.pool();
        let mut r = pool.empty(self.memory.kept_class());
        if pool.limit().is_none() {
            return Ok(r);
        }
        match r.grow(bytes, None, &mut |want| self.evict_kept(want)) {
            Ok(()) => Ok(r),
            Err(e) => {
                self.memory.refused = self.memory.refused.saturating_add(1);
                Err(Miss::Error(format!(
                    "column '{table}'[{column}] needs about {} in memory: {e}",
                    memory::size_text(bytes)
                )))
            }
        }
    }
}

/// `dax.memory`: sets the session's memory budget (`docs/beyond-ram.md`) and reports what the
/// session keeps.
pub(crate) fn dax_memory(s: &Session, p: &Value) -> Result<Value, String> {
    crate::params::allow(p, &["budget", "query_budget"])?;
    let bytes_param = |key: &str| -> Result<Option<Option<Option<u64>>>, String> {
        Ok(match p.get(key) {
            None => None,
            Some(Value::Null) => Some(None),
            Some(Value::Number(n)) => {
                let b = n.as_u64().ok_or_else(|| {
                    format!("{key}: expected a number of bytes (a whole number, 0 for none)")
                })?;
                Some(Some((b > 0).then_some(b)))
            }
            Some(Value::String(t)) => Some(Some(memory::parse_budget(t)?)),
            Some(_) => {
                return Err(format!(
                    "{key}: expected bytes (number or text such as \"300M\") or null"
                ));
            }
        })
    };
    let setting = bytes_param("budget")?;
    let query = bytes_param("query_budget")?;
    let s: &Session = s;
    let mut st = s.preview.lock();
    st.sync(s);
    if let Some(q) = query {
        // null: back to the process' default (BICRAFT_QUERY_BUDGET, else the memory budget)
        st.memory.query_setting = q;
    }
    if let Some(setting) = setting {
        // null: back to the process' default (BICRAFT_MEMORY_BUDGET)
        st.memory.setting = setting;
        if let Some(b) = st.memory.budget() {
            st.spill_now();
            if let Some(c) = &st.segs {
                c.set_cap(memory::segment_room(b));
            }
        }
        st.enforce_budget();
    }
    let cache_bytes = st
        .storage
        .as_ref()
        .map_or(0, general::storage::Store::approx_bytes)
        .saturating_add(st.segs.as_ref().map_or(0, |c| c.bytes()));
    let backup = match &st.store {
        Some(Ok(store)) => Some(store.backing()),
        _ => None,
    };
    st.sync_kept();
    st.sync_aggs_res();
    let mut out = memory::report(&st.memory, &st.columns, cache_bytes, backup, st.segs.as_deref());
    // the process' memory limit (lot 5): what is reserved, by whom, and the session's part
    let p = st.memory.pool().stats();
    let limit = json!({
        "limit": p.limit,
        "reserved_bytes": p.used,
        "peak_reserved_bytes": p.peak,
        "by_class": {
            "kept": p.class(bicraft_mem::Class::Kept),
            "query": p.class(bicraft_mem::Class::Query),
            "background": p.class(bicraft_mem::Class::Background),
            "io": p.class(bicraft_mem::Class::Io),
        },
        "refused": p.refused,
        "evictions": p.evictions,
        "running_queries": p.active_queries,
        "session_reserved_bytes": st.reserved_bytes(),
        "process_peak_bytes": bicraft_mem::process_peak_bytes(),
    });
    if let Some(o) = out.as_object_mut() {
        o.insert("limit".into(), limit);
    }
    Ok(out)
}

/// A hasher as a byte sink (the model is hashed through its JSON without building it).
struct HashWriter(std::collections::hash_map::DefaultHasher);

impl std::io::Write for HashWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// orchestration
// ---------------------------------------------------------------------------------------------

/// Hash of what a visual's data depends on in the visual itself.
fn visual_hash(v: &Visual) -> u64 {
    let fields = serde_json::to_vec(&v.fields).unwrap_or_default();
    let text = |k: &str| v.visual_extra.get(k).map(|q| q.to_string()).unwrap_or_default();
    // `objects`: conditional formatting rules are evaluated with the data; `expansionStates`:
    // a matrix's displayed levels and subtotals
    // a matrix's displayed levels and subtotals, a decomposition tree's levels; the reader's tree
    // state (never saved) too
    let (query, objects, expansion) = (text("query"), text("objects"), text("expansionStates"));
    let tree = text(DECOMP_STATE);
    hash_of(&[
        v.visual_type.as_deref().unwrap_or("").as_bytes(),
        &fields,
        query.as_bytes(),
        objects.as_bytes(),
        expansion.as_bytes(),
        tree.as_bytes(),
    ])
}

/// Pause visuals (`optimize.rs`) as one `report_data` call sees it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pause {
    /// The report is paused: a visual not in the result cache is not evaluated.
    pub on: bool,
    /// The call computes the data the snapshot shows (not the highlight report's): it keeps
    /// each visual's last data, and a paused visual shows it again, marked `pending`.
    pub main: bool,
}

/// `visual.data` for every visual that gets one in this snapshot, keyed by (page name, visual
/// name). Visuals without fields get none; visuals not reached within the budget get none. While
/// the report is paused (`pause.on`), a visual whose data is not in the result cache is not
/// evaluated: it keeps the data it last showed, with `pending: true`, or gets the `pending`
/// status.
pub(crate) fn report_data(
    s: &Session,
    report: &Report,
    pages: usize,
    visuals_per_page: usize,
    limits: &PreviewLimits,
    pause: Pause,
) -> HashMap<(String, String), Value> {
    let start = Instant::now();
    let mut out = HashMap::new();
    let mut st = s.preview.acquire(s);

    let pages: Vec<_> = report.pages.iter().take(pages).collect();
    let hashes: Vec<Vec<u64>> = pages
        .iter()
        .map(|p| p.visuals.iter().take(visuals_per_page).map(visual_hash).collect())
        .collect();
    let page_hash = |i: usize| {
        hashes.get(i).map_or(0, |h| {
            hash_of(&[&h.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()])
        })
    };
    let active = report
        .active_page
        .as_deref()
        .and_then(|a| pages.iter().position(|p| p.name == a))
        .unwrap_or(0);
    let mut order: Vec<usize> = vec![active];
    order.extend((0..pages.len()).filter(|i| {
        *i != active
            && st
                .page_hashes
                .get(&pages.get(*i).map_or(String::new(), |p| p.name.clone()))
                != Some(&page_hash(*i))
    }));
    for i in 0..pages.len() {
        if !order.contains(&i) {
            order.push(i);
        }
    }
    let rows = limits.rows_per_visual;
    let (generation, model_hash) = (st.generation, st.model_hash(s));
    let now = limits.now;
    let clock = clock_key(s, limits);
    // what each visual showed last time, and what it shows now (the main call only)
    let mut last = if pause.main {
        std::mem::take(&mut *s.preview.shown())
    } else {
        HashMap::new()
    };
    let mut kept: HashMap<(String, String), Value> = HashMap::new();
    for i in order {
        let Some(page) = pages.get(i) else {
            continue;
        };
        for (j, v) in page.visuals.iter().take(visuals_per_page).enumerate() {
            if v.fields.is_empty() {
                continue;
            }
            let applied = filters::collect(report, page, v, now);
            let vh = hashes.get(i).and_then(|h| h.get(j)).copied().unwrap_or(0);
            let key = hash_of(&[
                &generation.to_le_bytes(),
                &model_hash.to_le_bytes(),
                &vh.to_le_bytes(),
                &rows.to_le_bytes(),
                &applied.hash.to_le_bytes(),
                &clock.to_le_bytes(),
            ]);
            let data = match st.results.get(&key) {
                Some(d) => d.clone(),
                // paused: the last state, never an evaluation
                None if pause.on => {
                    if !pause.main {
                        continue;
                    }
                    let id = (page.name.clone(), v.name.clone());
                    let Some(mut d) = last.remove(&id) else {
                        out.insert(
                            id,
                            status(
                                "pending",
                                "visual queries are paused: Refresh visuals evaluates the visuals once",
                            ),
                        );
                        continue;
                    };
                    if let Some(o) = d.as_object_mut() {
                        o.insert("pending".into(), json!(true));
                    }
                    kept.insert(id.clone(), d.clone());
                    out.insert(id, d);
                    continue;
                }
                None if start.elapsed() < limits.budget => {
                    // a hierarchical axis is queried at its displayed level
                    let drilled = at_drill_level(v);
                    let v = drilled.as_ref().unwrap_or(v);
                    let t0 = Instant::now();
                    let d = match catch_unwind(AssertUnwindSafe(|| compute(s, &mut st, v, &applied, limits)))
                    {
                        Ok(d) => d,
                        Err(_) => status("error", "internal error while computing the preview"),
                    };
                    // the Performance analyzer records the page the viewer shows
                    if i == active {
                        perf::viewed(s, page, v, &applied, &d, t0.elapsed());
                    }
                    st.results.insert(key, d.clone());
                    d
                }
                None => continue,
            };
            if pause.main {
                kept.insert((page.name.clone(), v.name.clone()), data.clone());
            }
            out.insert((page.name.clone(), v.name.clone()), data);
        }
    }
    if pause.main {
        *s.preview.shown() = kept;
    }
    st.page_hashes = pages
        .iter()
        .enumerate()
        .map(|(i, p)| (p.name.clone(), page_hash(i)))
        .collect();
    out
}

/// The rows `visual.export_data` writes (`export_data.rs`): visual `v` of page `page` of
/// `report` as a flat table of its fields in their order, under its filters, at most `rows`
/// rows (`truncated` and `total_rows` say what was left out). Summarized: the visual's own
/// aggregations, every level of a hierarchy (not only the one drilled to). Underlying: each
/// aggregated column grouped instead of aggregated, so a row per distinct combination of the
/// columns, with the visual's measures evaluated on it.
pub(crate) fn export_rows(
    s: &Session,
    report: &Report,
    page: &bicraft_report::Page,
    v: &Visual,
    underlying: bool,
    rows: usize,
) -> Value {
    let mut flat = v.clone();
    flat.visual_type = Some("tableEx".into());
    let mut fields: Vec<bicraft_report::FieldBinding> = Vec::new();
    for f in &v.fields {
        // a sparkline is a small chart, not a value to export
        if bicraft_report::query::sparkline(&f.field).is_some() {
            continue;
        }
        let mut f = f.clone();
        f.role = "Values".into();
        if underlying && let FieldRef::Aggregation { table, column, .. } = &f.field {
            f.field = FieldRef::Column {
                table: table.clone(),
                column: column.clone(),
            };
        }
        if !fields.iter().any(|x| x.field == f.field) {
            fields.push(f);
        }
    }
    flat.fields = fields;
    let now = filters::clock(s);
    let limits = PreviewLimits {
        rows_per_visual: rows,
        table_rows: STORAGE_TABLE_ROWS,
        fallback_rows: crate::SnapshotLimits::default().data_table_rows,
        budget: Duration::from_secs(120),
        now,
        utc_offset: filters::utc_offset(s),
    };
    let applied = filters::collect(report, page, v, now);
    let mut st = s.preview.acquire(s);
    match catch_unwind(AssertUnwindSafe(|| compute(s, &mut st, &flat, &applied, &limits))) {
        Ok(d) => d,
        Err(_) => status("error", "internal error while computing the export"),
    }
}

/// What cached results depend on in the clock: the day, or the exact `as_of` when one is set.
fn clock_key(s: &Session, limits: &PreviewLimits) -> u64 {
    if s.as_of_now().is_some() {
        limits.now.to_bits()
    } else {
        limits.now.floor().to_bits()
    }
}

/// The measure-bound properties' values (`bound_values.rs`) of every visual that has some, as
/// the `visual.data` of a field-less copy binding just those measures, keyed by (page name,
/// visual name). Visuals not reached within the budget get none.
pub(crate) fn report_bound(
    s: &Session,
    report: &Report,
    pages: usize,
    visuals_per_page: usize,
    limits: &PreviewLimits,
) -> HashMap<(String, String), Value> {
    let start = Instant::now();
    let mut out = HashMap::new();
    let mut st = s.preview.acquire(s);
    let (generation, model_hash) = (st.generation, st.model_hash(s));
    let now = limits.now;
    let clock = clock_key(s, limits);
    for page in report.pages.iter().take(pages) {
        for v in page.visuals.iter().take(visuals_per_page) {
            let bound = crate::bound_values::fields(v);
            if bound.is_empty() {
                continue;
            }
            let sv = crate::bound_values::synthetic(v, &bound);
            let applied = filters::collect(report, page, &sv, now);
            let key = hash_of(&[
                b"bound",
                &generation.to_le_bytes(),
                &model_hash.to_le_bytes(),
                &visual_hash(&sv).to_le_bytes(),
                &applied.hash.to_le_bytes(),
                &clock.to_le_bytes(),
            ]);
            let data = match st.results.get(&key) {
                Some(d) => d.clone(),
                None if start.elapsed() < limits.budget => {
                    let d =
                        match catch_unwind(AssertUnwindSafe(|| compute(s, &mut st, &sv, &applied, limits))) {
                            Ok(d) => d,
                            Err(_) => status("error", "internal error while computing the preview"),
                        };
                    st.results.insert(key, d.clone());
                    d
                }
                None => continue,
            };
            out.insert((page.name.clone(), v.name.clone()), data);
        }
    }
    out
}

fn status(kind: &str, reason: impl Into<String>) -> Value {
    json!({"status": kind, "reason": reason.into()})
}

fn miss_json(m: Miss) -> Value {
    match m {
        Miss::Unavailable(r) => status("unavailable", r),
        Miss::Error(r) => status("error", r),
    }
}

/// Why a visual cannot be evaluated.
#[derive(Debug)]
enum Fail {
    Unsupported(String),
    Miss(Miss),
}

impl From<Miss> for Fail {
    fn from(m: Miss) -> Self {
        Fail::Miss(m)
    }
}

fn unsupported<T>(reason: impl Into<String>) -> Result<T, Fail> {
    Err(Fail::Unsupported(reason.into()))
}

/// Visual types whose fields are not a plain grouped query.
const UNSUPPORTED_TYPES: &[&str] = &[
    "keyDriversVisual",
    "qnaVisual",
    "scriptVisual",
    "pythonVisual",
    "aiNarratives",
];

/// What one evaluation reads besides the visual's fields.
struct Query<'q> {
    /// Column conditions and top N filters (measure conditions are applied to the rows after).
    filters: &'q [Item],
    /// Hidden fields evaluated after the visual's own (read by measure conditions and
    /// conditional formatting rules).
    extra: &'q [FieldRef],
}

/// An evaluated visual before ordering: one cell per field, the visual's fields then the hidden
/// ones.
struct Grid {
    /// `data.columns` (visible fields only).
    columns: Vec<Value>,
    is_group: Vec<bool>,
    /// The model (table, column) of each grouping field.
    groups: Vec<Option<(String, String)>>,
    rows: Vec<Vec<Out>>,
    /// Filters the evaluation could not apply, and why.
    ignored: Vec<String>,
}

/// The visual's filters, split by how they apply.
struct Plan {
    /// Column conditions and top N filters.
    items: Vec<Item>,
    /// Conditions on measures and aggregations (applied to the visual's rows).
    values: Vec<Cond>,
    ignored: Vec<String>,
}

fn operand_field(o: &Operand) -> FieldRef {
    match o {
        Operand::Col { table, column } => FieldRef::Column {
            table: table.clone(),
            column: column.clone(),
        },
        Operand::Measure { table, name } => FieldRef::Measure {
            table: table.clone(),
            measure: name.clone(),
        },
        Operand::Agg {
            table,
            column,
            function,
        } => FieldRef::Aggregation {
            table: table.clone(),
            column: column.clone(),
            function: function.clone(),
        },
    }
}

fn plan(applied: &Applied) -> Plan {
    let mut p = Plan {
        items: Vec::new(),
        values: Vec::new(),
        ignored: applied.ignored.clone(),
    };
    for it in &applied.items {
        match it {
            Item::TopN(_) => p.items.push(it.clone()),
            Item::Cond(c) => {
                let ops = c.operands();
                let cols = ops.iter().filter(|o| matches!(o, Operand::Col { .. })).count();
                if cols == ops.len() {
                    p.items.push(it.clone());
                } else if cols == 0 {
                    p.values.push(c.clone());
                } else {
                    p.ignored.push("a condition mixing columns and measures".into());
                }
            }
        }
    }
    p
}

/// Adds `f` to `extra` unless the visual binds it or it is there already.
fn add_extra(v: &Visual, extra: &mut Vec<FieldRef>, f: FieldRef) {
    if !v.fields.iter().any(|b| same_field(&b.field, &f)) && !extra.iter().any(|x| same_field(x, &f)) {
        extra.push(f);
    }
}

/// [`compute_inner`], failed when a column segment could not be read meanwhile (lot 3: an answer
/// is never computed from a failed read).
fn compute(s: &Session, st: &mut State, v: &Visual, applied: &Applied, limits: &PreviewLimits) -> Value {
    // sparklines: the visual without them, then each one's points per row
    if let Some(sp) = sparks::split(v) {
        let mut d = compute(s, st, &sp.base, applied, limits);
        sparks::apply(&sp, &mut d, &mut |sv| compute(s, st, sv, applied, limits));
        sparks::apply_formats(v, &mut d);
        return d;
    }
    let mut d = compute_checked(s, st, v, applied, limits);
    // Format pane › General › Data format: the visual's own format strings
    sparks::apply_formats(v, &mut d);
    d
}

/// [`compute`] of a visual without sparklines.
fn compute_checked(
    s: &Session,
    st: &mut State,
    v: &Visual,
    applied: &Applied,
    limits: &PreviewLimits,
) -> Value {
    let failures = bicraft_scan::lazy::load_failures();
    // visual calculations and hidden fields: the visual without its calculations, then them
    let split = viscalc::split(v);
    let d = match &split {
        Some(sp) => compute_inner(s, st, &sp.base, applied, limits, Some(sp)),
        None => compute_inner(s, st, v, applied, limits, None),
    };
    if let Some(e) = st.memory.unpack_refused.take() {
        return status("error", &e);
    }
    if bicraft_scan::lazy::load_failures() != failures {
        st.storage = None;
        return status("error", SEGMENT_READ_FAILED);
    }
    d
}

/// The error of an answer during which a column segment could not be read.
pub(crate) const SEGMENT_READ_FAILED: &str =
    "a column segment could not be read from the backup's temporary file: no answer is given from it";

fn compute_inner(
    s: &Session,
    st: &mut State,
    v: &Visual,
    applied: &Applied,
    limits: &PreviewLimits,
    calcs: Option<&viscalc::Split>,
) -> Value {
    let Some(model) = s.model.as_ref() else {
        return status("unavailable", "the session has no model");
    };
    if let Some(t) = v.visual_type.as_deref()
        && UNSUPPORTED_TYPES.contains(&t)
    {
        return status("unsupported", format!("visual type `{t}` is not previewed"));
    }
    // a field parameter's binding becomes the fields its slicer chooses
    let expanded = field_params::expand(model, v, applied);
    let (v, applied) = match &expanded {
        Some((v, a)) => (v, a),
        None => (v, applied),
    };
    let normal = normalized(model, v);
    let v = normal.as_ref().unwrap_or(v);
    let mut ctx = Ctx { s, st, model, limits };
    let plan = plan(applied);
    if v.visual_type.as_deref() == Some("decompositionTreeVisual") {
        let notes = plan.ignored.clone();
        return decomp::compute(&mut ctx, v, &plan, notes);
    }
    let rules = format::Rules::of(v, Some(model));
    let mut extra: Vec<FieldRef> = Vec::new();
    for c in &plan.values {
        for o in c.operands() {
            add_extra(v, &mut extra, operand_field(o));
        }
    }
    for f in rules.fields() {
        if !matches!(f, FieldRef::Column { .. }) {
            add_extra(v, &mut extra, f.clone());
        }
    }
    // error bars: their bounds, evaluated per data point beside the series
    let bars = crate::visual_extras::error_bars(v);
    for b in &bars {
        for f in b.upper.iter().chain(&b.lower) {
            add_extra(v, &mut extra, f.clone());
        }
    }
    // the storage engine (general evaluator), else the row-by-row fast path ([`grid`])
    let run = |ctx: &mut Ctx<'_>, q: &Query<'_>| grid(ctx, v, q);
    let mut notes = plan.ignored.clone();
    let full = Query {
        filters: &plan.items,
        extra: &extra,
    };
    let mut used_extra: &[FieldRef] = &extra;
    let mut used_filters: &[Item] = &plan.items;
    // why the conditional formatting rules that read measures were left out
    let mut unformatted: Option<String> = None;
    let mut result = run(&mut ctx, &full);
    if let Err(Fail::Unsupported(r)) = &result
        && !extra.is_empty()
    {
        // without the hidden fields (their conditions and rules are then not applied)
        let r = r.clone();
        let q = Query {
            filters: &plan.items,
            extra: &[],
        };
        result = run(&mut ctx, &q);
        if result.is_ok() {
            used_extra = &[];
            if !plan.values.is_empty() {
                notes.push(format!("measure filters: {r}"));
            }
            if rules.fields().any(|f| !matches!(f, FieldRef::Column { .. })) {
                unformatted = Some(r);
            }
        }
    }
    if let Err(Fail::Unsupported(r)) = &result
        && !plan.items.is_empty()
    {
        // without the filters: the unfiltered data, flagged
        let r = r.clone();
        let q = Query {
            filters: &[],
            extra: &[],
        };
        result = run(&mut ctx, &q);
        if result.is_ok() {
            used_extra = &[];
            used_filters = &[];
            notes.push(format!("the filters could not be applied: {r}"));
        }
    }
    match result {
        Ok(mut grid) => {
            let grouped = grid.is_group.iter().any(|g| *g);
            let is_group = grid.is_group.clone();
            // "Show items with no data": the show-all roles' categories without values
            if let Err(r) = show::add_items_with_no_data(&mut ctx, v, &mut grid, used_filters) {
                notes.push(format!("the items with no data could not be listed: {r}"));
            }
            let mut d = finish_data(&mut ctx, v, grid, used_extra, &plan, &rules, notes, &bars);
            if let Some(o) = d.as_object_mut() {
                if let Some(r) = unformatted {
                    o.insert("formatting_ignored_reason".into(), json!(r));
                }
                // the whole-visual conditional formats (title, background, border): their
                // fields in the visual's filter context, without its grouping
                let vf = rules.visual_fields();
                if !vf.is_empty() {
                    match general::values_in(&mut ctx, &vf, used_filters) {
                        Ok(vals) => {
                            let at = |f: &FieldRef| {
                                vf.iter().position(|x| x == f).and_then(|i| vals.get(i).cloned())
                            };
                            let styles = rules.visual_styles(&at);
                            if !styles.is_empty() {
                                o.insert("visual_styles".into(), Value::Object(styles));
                            }
                        }
                        Err(Fail::Unsupported(r) | Fail::Miss(Miss::Unavailable(r) | Miss::Error(r))) => {
                            o.insert("visual_styles_reason".into(), json!(r));
                        }
                    }
                }
                // a table's total row: its values in the grand-total filter context (Desktop
                // evaluates them, it does not add the rows up); measure filters keep the old sum
                if grouped && plan.values.is_empty() && v.visual_type.as_deref() == Some("tableEx") {
                    match general::totals(&mut ctx, v, used_filters) {
                        Ok(t) => {
                            o.insert("totals".into(), Value::Array(t.iter().map(out_json).collect()));
                        }
                        Err(Fail::Unsupported(r)) => {
                            o.insert("totals_reason".into(), json!(r));
                        }
                        Err(Fail::Miss(Miss::Unavailable(r) | Miss::Error(r))) => {
                            o.insert("totals_reason".into(), json!(r));
                        }
                    }
                }
                // a matrix's subtotals and grand totals, evaluated per level like Desktop's
                if grouped && plan.values.is_empty() && v.visual_type.as_deref() == Some("pivotTable") {
                    o.extend(matrix::hierarchy_keys(v, &is_group));
                    match matrix::levels(&mut ctx, v, &is_group, used_filters) {
                        Ok(l) => {
                            o.insert("subtotals".into(), l);
                        }
                        Err(r) => {
                            o.insert("subtotals_reason".into(), json!(r));
                        }
                    }
                }
            }
            // "Show value as": percentages of the totals, in the rows, total and subtotals
            if v.fields.iter().any(|b| b.show_as.is_some()) {
                let values = d.clone();
                if let Err(r) = show::apply_show_value_as(&mut ctx, v, &is_group, used_filters, &mut d) {
                    // the values as they are, never half of them divided
                    d = values;
                    if let Some(o) = d.as_object_mut() {
                        o.insert("show_as_reason".into(), json!(r));
                    }
                }
            }
            // visual calculations over the visual matrix; hidden fields flagged
            if let Some(sp) = calcs {
                viscalc::apply(&mut ctx, v, sp, &is_group, used_filters, &mut d);
            }
            d
        }
        Err(Fail::Unsupported(r)) => status("unsupported", r),
        Err(Fail::Miss(m)) => miss_json(m),
    }
}

// ---------------------------------------------------------------------------------------------
// table access
// ---------------------------------------------------------------------------------------------

struct Ctx<'a> {
    s: &'a Session,
    st: &'a mut State,
    model: &'a SemanticModel,
    limits: &'a PreviewLimits,
}

impl Ctx<'_> {
    /// The opened backup, if the source has one (opened on first use).
    fn store(&mut self) -> Result<Option<Arc<VertipaqStore>>, Miss> {
        if self.st.store.is_none() {
            let budget = self.st.memory.budget();
            let pool = self.st.memory.pool();
            let opened = match &self.s.source {
                Some(Source::Package { package, .. }) => package.get(bicraft_pbix::DATA_MODEL).map(|p| {
                    if p.len() > bicraft_opc::MAX_PART_BYTES {
                        return Err(format!(
                            "the {}-byte DataModel is too large for the preview",
                            p.len()
                        ));
                    }
                    match self.st.seeded.take() {
                        Some((id, Seed::Backup(abf))) if id == part_identity(p) => {
                            VertipaqStore::from_backup(abf, MAX_BACKUP_BYTES)
                        }
                        Some((id, Seed::Store(store))) if id == part_identity(p) => Ok(store),
                        // under a memory budget: decompressed as a stream to a temporary file,
                        // the part read from the package file when it was left there
                        _ if budget.is_some() => {
                            let mut open = || {
                                p.reader()
                                    .map(|r| Box::new(r) as Box<dyn std::io::Read + Send + '_>)
                                    .map_err(|e| e.to_string())
                            };
                            VertipaqStore::open_streamed_in(
                                &mut open,
                                &memory::spill_dir(),
                                MAX_BACKUP_BYTES,
                                pool,
                            )
                            .map(|(store, _)| store)
                        }
                        _ => p
                            .bytes()
                            .map_err(|e| e.to_string())
                            .and_then(|b| VertipaqStore::open(&b, MAX_BACKUP_BYTES)),
                    }
                }),
                Some(Source::Project { .. }) => crate::cache_abf::path(self.s).transpose().map(|p| {
                    let p = p?;
                    let len = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(u64::MAX);
                    if len > MAX_BACKUP_BYTES as u64 {
                        return Err("cache.abf is too large for the preview".to_string());
                    }
                    if budget.is_some() {
                        let mut open = || {
                            std::fs::File::open(&p)
                                .map(|f| {
                                    Box::new(std::io::BufReader::with_capacity(1 << 20, f))
                                        as Box<dyn std::io::Read + Send + '_>
                                })
                                .map_err(|e| format!("cache.abf: {e}"))
                        };
                        return VertipaqStore::open_streamed_in(
                            &mut open,
                            &memory::spill_dir(),
                            MAX_BACKUP_BYTES,
                            pool,
                        )
                        .map(|(store, _)| store);
                    }
                    let bytes = std::fs::read(&p).map_err(|e| format!("cache.abf: {e}"))?;
                    VertipaqStore::open(&bytes, MAX_BACKUP_BYTES)
                }),
                _ => None,
            };
            let Some(opened) = opened else {
                return Ok(None);
            };
            // under a memory budget a backup still in memory goes to a temporary file once this
            // query is answered (background work, else the next evaluation)
            if budget.is_some() && opened.as_ref().is_ok_and(|s| s.backing().0 == "memory") {
                self.st.memory.spill_pending = true;
                self.s.preview.set_pending();
            }
            self.st.store = Some(opened.map(Arc::new));
        }
        match &self.st.store {
            Some(Ok(store)) => Ok(Some(Arc::clone(store))),
            Some(Err(e)) => Err(Miss::Error(format!("cannot read the stored data: {e}"))),
            None => Ok(None),
        }
    }

    /// bicraft's own data snapshot of the project, when it is the data to read (then the
    /// project's `cache.abf` is not: its tables would be older).
    fn own(&mut self) -> Option<Arc<crate::refresh::OwnData>> {
        if self.st.own.is_none() {
            // An unreadable snapshot is ignored (the data falls back to cache.abf / partitions).
            self.st.own = Some(crate::refresh::open_snapshot(self.s).ok().flatten());
        }
        self.st.own.clone().flatten()
    }

    /// The table's data from the backup (`None`: the backup does not hold it, or bicraft's own
    /// snapshot is the data to read).
    fn stored(&mut self, table: &str) -> Result<Option<Arc<VertipaqStore>>, Miss> {
        // a DirectQuery table holds no imported rows: BIcraft does not query sources live
        if let Some(t) = model_table(self.model, table)
            && crate::storage_mode::is_direct_query(self.model, t)
        {
            return Err(Miss::Unavailable(crate::storage_mode::direct_query_reason(
                &t.name,
            )));
        }
        if self.own().is_some() {
            return Ok(None);
        }
        let store = match self.store() {
            Ok(s) => s,
            // a broken backup: fall back to literal partitions, else report the error
            Err(e) => {
                return if self.literal_possible(table) {
                    Ok(None)
                } else {
                    Err(e)
                };
            }
        };
        Ok(store.filter(|s| s.has_table(table)))
    }

    fn literal_possible(&self, table: &str) -> bool {
        model_table(self.model, table).is_some_and(|t| !t.partitions.is_empty())
    }

    fn literal(&mut self, table: &str) -> Result<Arc<LiteralTable>, Miss> {
        let Some(t) = model_table(self.model, table) else {
            return Err(Miss::Unavailable(format!("table '{table}' is not in the model")));
        };
        let key = literal_key(t);
        let lower = t.name.to_lowercase();
        if let Some((k, data)) = self.st.literal.get(&lower)
            && *k == key
        {
            return data.clone();
        }
        // an error under a cap below the storage engine's is not kept: it may be that cap's, and
        // the storage engine reads the table whole
        let keep_errors = self.limits.table_rows >= STORAGE_TABLE_ROWS;
        if let Some(own) = self.own()
            && let Some(stored) = own.fresh(self.model, t)
        {
            let rows = stored.data.row_count();
            let data = if rows > self.limits.table_rows {
                Err(Miss::Error(format!(
                    "table '{}' has more than {} rows (preview limit)",
                    t.name, self.limits.table_rows
                )))
            } else {
                let mut columns = HashMap::new();
                let mut failed = None;
                for c in stored.data.columns() {
                    match ColData::from_stored(c.values.clone()) {
                        Ok(d) => {
                            columns.insert(c.name.to_lowercase(), Arc::new(d));
                        }
                        Err(m) => {
                            failed = Some(m);
                            break;
                        }
                    }
                }
                match failed {
                    Some(m) => Err(m),
                    None => Ok(Arc::new(LiteralTable { rows, columns })),
                }
            };
            if data.is_ok() || keep_errors {
                self.st.literal.insert(lower, (key, data.clone()));
            }
            return data;
        }
        let data =
            load_literal(t, self.model, self.s.data_root.as_deref(), self.limits.table_rows).map(Arc::new);
        if data.is_ok() || keep_errors {
            self.st.literal.insert(lower, (key, data.clone()));
        }
        data
    }

    /// A column's data as evaluations see it: under View as, only the rows the roles keep
    /// (`preview_rls.rs`).
    fn column(&mut self, table: &str, column: &str) -> Result<Arc<ColData>, Miss> {
        let Some(sec) = self.st.security.clone() else {
            return self.column_raw(table, column);
        };
        sec.check_column(table, column)?;
        let Some(keep) = sec.rows_of(table)? else {
            return self.column_raw(table, column);
        };
        let key = (table.to_lowercase(), column.to_lowercase());
        if let Some(c) = self.st.secured_columns.get(&key) {
            return c.clone();
        }
        let c = self
            .column_raw(table, column)
            .and_then(|raw| rls::keep_rows(&raw, &keep).map(Arc::new));
        self.st.secured_columns.insert(key, c.clone());
        c
    }

    /// A column's data as stored (every row).
    fn column_raw(&mut self, table: &str, column: &str) -> Result<Arc<ColData>, Miss> {
        if let Some(store) = self.stored(table)? {
            let key = (table.to_lowercase(), column.to_lowercase());
            if let Some(c) = self.st.columns.get(&key) {
                let c = c.clone();
                self.st.memory.touch(&key);
                return c;
            }
            if let Some(c) = self.st.inherited_column(&key) {
                return Ok(c);
            }
            // a table over this caller's cap is refused before its column is read, and the
            // refusal is not kept: the storage engine reads the table whole
            if let Some(n) = store.row_count(table).and_then(|n| usize::try_from(n).ok())
                && n > self.limits.table_rows
            {
                return Err(Miss::Error(format!(
                    "table '{table}' has more than {} rows (preview limit)",
                    self.limits.table_rows
                )));
            }
            // a calculated column added since the data was stored (a group, `column.group`): its
            // expression over every row, kept with the stored columns
            if store.column_file_bytes(table, column).is_none()
                && model_table(self.model, table)
                    .and_then(|t| t.columns.iter().find(|c| c.name.eq_ignore_ascii_case(column)))
                    .is_some_and(|c| c.expression.is_some())
            {
                return self.computed(table, column);
            }
            // another evaluation of this session may be reading it right now (`preview_fork.rs`):
            // its result is taken instead of a second copy
            let claim = match self.s.preview.claim(self.st.generation, &key) {
                fork::Claim::Loaded(c) => {
                    self.st.adopt_column(key, Arc::clone(&c));
                    return Ok(c);
                }
                fork::Claim::Failed => None,
                fork::Claim::Mine(g) => Some(g),
            };
            let budget = self.st.memory.budget();
            let file = store.column_file_bytes(table, column);
            // a column over half the budget is read segment by segment (lot 3)
            let mut lazy = matches!((budget, file), (Some(b), Some(n)) if n > b / 2);
            // under a memory limit, reserved before it is read (lot 5): what the session keeps is
            // freed first; refused whole, the column is read segment by segment, which keeps only
            // its values and id table (its dictionary file is the least of them): it is refused
            // only when that does not fit either
            let whole = file.unwrap_or(0).min(budget.unwrap_or(usize::MAX));
            let mut admitted = match self.st.admit_column(table, column, whole) {
                Ok(r) => r,
                Err(e) => {
                    let Some(b) = budget else {
                        return Err(e);
                    };
                    let dict = store.dictionary_file_bytes(table, column).unwrap_or(0).min(b);
                    let r = self.st.admit_column(table, column, dict)?;
                    // the whole read was not refused in the end
                    self.st.memory.refused = self.st.memory.refused.saturating_sub(1);
                    lazy = true;
                    r
                }
            };
            let c = match budget {
                Some(b) if lazy => Ok(Arc::new(self.column_lazy(&store, table, column, b)?)),
                _ => store.column(table, column, self.limits.table_rows).map(Arc::new),
            };
            let mut c = match (budget, c) {
                // decoded whole it is over the budget (its values): segment by segment, it keeps
                // only its values and id table
                (Some(b), Ok(d))
                    if memory::base_bytes(&d) > b
                        && !d.codes.as_packed().is_some_and(bicraft_scan::Packed::is_lazy) =>
                {
                    drop(d);
                    Ok(Arc::new(self.column_lazy(&store, table, column, b)?))
                }
                (_, c) => c,
            };
            if let Some(b) = budget {
                while let Ok(d) = &c {
                    let n = memory::base_bytes(d);
                    if n > b {
                        self.st.memory.refused = self.st.memory.refused.saturating_add(1);
                        return Err(memory::too_large(table, column, n, b));
                    }
                    // decoded, it may take more than its files: the rest reserved too, or it goes
                    if n <= admitted.bytes() || self.st.memory.limit().is_none() {
                        break;
                    }
                    let is_lazy = d.codes.as_packed().is_some_and(bicraft_scan::Packed::is_lazy);
                    let more = n.saturating_sub(admitted.bytes());
                    let st = &mut *self.st;
                    match admitted.grow(more, None, &mut |w| st.evict_kept(w)) {
                        Ok(()) => break,
                        // decoded whole it does not fit beside what is reserved: segment by
                        // segment instead
                        Err(_) if !is_lazy => {
                            // the whole column goes first
                            drop(std::mem::replace(&mut c, Err(Miss::Unavailable(String::new()))));
                            c = Ok(Arc::new(self.column_lazy(&store, table, column, b)?));
                        }
                        Err(e) => {
                            self.st.memory.refused = self.st.memory.refused.saturating_add(1);
                            return Err(Miss::Error(format!(
                                "column '{table}'[{column}] needs {} in memory: {e}",
                                memory::size_text(n)
                            )));
                        }
                    }
                }
            }
            if budget.is_some() {
                self.st.memory.column_reads = self.st.memory.column_reads.saturating_add(1);
            }
            if let (Some(g), Ok(col)) = (claim, &c) {
                g.finish(col);
            }
            self.st.columns.insert(key.clone(), c.clone());
            // from here the column is counted with the kept ones
            drop(admitted);
            if budget.is_some() {
                self.st.memory.touch(&key);
                // `c` is held here: the new column is not evicted by its own read
                self.st.enforce_budget();
            }
            return c;
        }
        let calculated = model_table(self.model, table)
            .and_then(|t| t.columns.iter().find(|c| c.name.eq_ignore_ascii_case(column)))
            .is_some_and(|c| c.expression.is_some());
        let t = self.literal(table).map_err(|m| self.stored_error(m))?;
        match t.columns.get(&column.to_lowercase()) {
            Some(c) => Ok(Arc::clone(c)),
            // a calculated column its partitions do not give (a group, `column.group`, or any
            // other): its expression over every row
            None if calculated => self.computed(table, column),
            None => Err(Miss::Unavailable(format!(
                "no data for column '{table}'[{column}]"
            ))),
        }
    }

    /// Calculated column `table[column]` that the data does not hold, computed: its expression
    /// once per row, with that row as the row context and no filter (`preview_eval_rows.rs`).
    /// Kept until the model or the data changes; a column that needs itself fails.
    fn computed(&mut self, table: &str, column: &str) -> Result<Arc<ColData>, Miss> {
        let key = (table.to_lowercase(), column.to_lowercase());
        let stamp = (self.st.model_hash(self.s), self.st.generation);
        if let Some((at, c)) = self.st.computed.get(&key)
            && *at == stamp
        {
            return c.clone();
        }
        let Some((expression, ty)) = model_table(self.model, table)
            .and_then(|t| t.columns.iter().find(|c| c.name.eq_ignore_ascii_case(column)))
            .and_then(|c| c.expression.clone().map(|e| (e, c.data_type)))
        else {
            return Err(Miss::Unavailable(format!(
                "no data for column '{table}'[{column}]"
            )));
        };
        if !self.st.computing.insert(key.clone()) {
            return Err(Miss::Error(format!(
                "calculated column '{table}'[{column}] depends on itself"
            )));
        }
        let r = self.compute_rows(table, column, &expression, ty);
        self.st.computing.remove(&key);
        let r = r.map(Arc::new);
        self.st.computed.insert(key, (stamp, r.clone()));
        r
    }

    fn compute_rows(
        &mut self,
        table: &str,
        column: &str,
        expression: &str,
        ty: bicraft_model::DataType,
    ) -> Result<ColData, Miss> {
        let n = self.rows(table)?;
        if n > MAX_COMPUTED_ROWS {
            return Err(Miss::Unavailable(format!(
                "calculated column '{table}'[{column}] is computed over at most {MAX_COMPUTED_ROWS} rows; \
                 its table has {n}"
            )));
        }
        let rows: Vec<u32> = (0..n).filter_map(|r| u32::try_from(r).ok()).collect();
        let values = general::rows_calc::calc_rows(self, table, expression, &rows, COMPUTED_BUDGET)
            .map_err(|e| Miss::Unavailable(format!("calculated column '{table}'[{column}]: {e}")))?;
        ColData::from_stored(general::rows_calc::as_declared(values, ty))
    }

    /// A stored column read segment by segment under the budget `b` (lot 3): it keeps its values,
    /// id table and segment statistics. A dictionary that cannot fit is refused before any of it
    /// is read, so is a segment over what the budget leaves for one; the error is not kept (another
    /// budget may read the column).
    fn column_lazy(
        &mut self,
        store: &VertipaqStore,
        table: &str,
        column: &str,
        b: usize,
    ) -> Result<ColData, Miss> {
        let dict = store.dictionary_file_bytes(table, column).unwrap_or(0);
        if dict > b {
            self.st.memory.refused = self.st.memory.refused.saturating_add(1);
            return Err(memory::too_large(table, column, dict, b));
        }
        let cache = self.st.seg_cache(b);
        // the first pass reads its batches in what the memory limit leaves (one segment at least)
        let mut batch = memory::segment_room(b);
        let pool = self.st.memory.pool();
        if let Some(l) = pool.limit() {
            batch = batch.min(l.saturating_sub(pool.stats().used)).max(1);
        }
        let limits = store::LazyLimits {
            max_rows: self.limits.table_rows,
            max_segment: memory::segment_room(b),
            batch,
        };
        match store.column_lazy(table, column, &cache, limits) {
            Ok(c) => {
                self.st.memory.lazy_reads = self.st.memory.lazy_reads.saturating_add(1);
                Ok(c)
            }
            Err(e) => {
                self.st.memory.refused = self.st.memory.refused.saturating_add(1);
                Err(e)
            }
        }
    }

    /// A table's row count as evaluations see it (under View as, the rows the roles keep).
    fn rows(&mut self, table: &str) -> Result<usize, Miss> {
        if let Some(sec) = self.st.security.clone()
            && let Some(keep) = sec.rows_of(table)?
        {
            return Ok(keep.len());
        }
        self.rows_raw(table)
    }

    fn rows_raw(&mut self, table: &str) -> Result<usize, Miss> {
        if let Some(n) = self.st.provided_rows.get(&table.to_lowercase()) {
            return Ok(*n);
        }
        if let Some(store) = self.stored(table)? {
            let n = store.row_count(table).unwrap_or(0);
            return usize::try_from(n)
                .ok()
                .filter(|n| *n <= self.limits.table_rows)
                .ok_or_else(|| {
                    Miss::Error(format!(
                        "table '{table}' has more than {} rows (preview limit)",
                        self.limits.table_rows
                    ))
                });
        }
        let n = self.literal(table).map_err(|m| self.stored_error(m))?.rows;
        // loaded whole by the storage engine, the table is still refused over a lower cap
        if n > self.limits.table_rows {
            return Err(Miss::Error(format!(
                "table '{table}' has more than {} rows (preview limit)",
                self.limits.table_rows
            )));
        }
        Ok(n)
    }

    /// A table's partitions could not give its data because the stored data could not be read:
    /// the stored data's error is the one to report.
    fn stored_error(&self, m: Miss) -> Miss {
        match (&m, &self.st.store) {
            (Miss::Unavailable(_), Some(Err(e))) => Miss::Error(format!("cannot read the stored data: {e}")),
            _ => m,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// measures
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Agg {
    Sum,
    Avg,
    Min,
    Max,
    Median,
    /// Sample standard deviation (`STDEV.S`).
    StdDev,
    /// Sample variance (`VAR.S`).
    Var,
    /// Population standard deviation (`STDEV.P`, PBIR `StandardDeviation`: Desktop 2.158 divides
    /// by n for a visual's implicit "Standard deviation of").
    StdDevP,
    /// Population variance (`VAR.P`, PBIR `Variance`).
    VarP,
    /// Non-blank values (`COUNT`, `COUNTA`, PBIR `CountNonNull`).
    Count,
    /// Blank values (`COUNTBLANK`).
    CountBlank,
    /// Distinct values, blank included (`DISTINCTCOUNT`, PBIR `Count`).
    Distinct,
    DistinctNoBlank,
    CountRows,
}

/// One aggregation over a table (the unit the evaluator groups).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Leaf {
    agg: Agg,
    /// Model table name as written in the model.
    table: String,
    column: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

/// A measure expression bicraft can evaluate.
#[derive(Debug, Clone, PartialEq)]
enum Ir {
    Const(f64),
    Blank,
    Leaf(usize),
    Neg(Box<Ir>),
    Bin(Op, Box<Ir>, Box<Ir>),
    Divide(Box<Ir>, Box<Ir>, Option<Box<Ir>>),
    Min2(Box<Ir>, Box<Ir>),
    Max2(Box<Ir>, Box<Ir>),
}

struct Compiler<'m> {
    model: &'m SemanticModel,
    leaves: Vec<Leaf>,
}

type Env = HashMap<String, Ir>;

impl Compiler<'_> {
    fn leaf(&mut self, leaf: Leaf) -> Ir {
        let i = match self.leaves.iter().position(|l| *l == leaf) {
            Some(i) => i,
            None => {
                self.leaves.push(leaf);
                self.leaves.len().saturating_sub(1)
            }
        };
        Ir::Leaf(i)
    }

    /// Canonical (table, column) names of a model column.
    fn column(&self, table: &str, column: &str) -> Result<(String, String), Fail> {
        let t = model_table(self.model, table)
            .ok_or_else(|| Fail::Unsupported(format!("unknown table '{table}'")))?;
        let c = t
            .columns
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(column))
            .ok_or_else(|| Fail::Unsupported(format!("unknown column '{table}'[{column}]")))?;
        Ok((t.name.clone(), c.name.clone()))
    }

    /// A measure by (optional) table and name.
    fn find_measure(&self, table: Option<&str>, name: &str) -> Option<(String, String)> {
        let in_table = |t: &bicraft_model::Table| {
            t.measures
                .iter()
                .find(|m| m.name.eq_ignore_ascii_case(name))
                .map(|m| (t.name.clone(), m.expression.clone()))
        };
        if let Some(found) = table.and_then(|t| model_table(self.model, t)).and_then(in_table) {
            return Some(found);
        }
        self.model.tables.iter().find_map(in_table)
    }

    fn measure(&mut self, table: Option<&str>, name: &str, depth: usize) -> Result<Ir, Fail> {
        if depth > MAX_DEPTH {
            return unsupported("measure references nest too deeply");
        }
        let Some((home, text)) = self.find_measure(table, name) else {
            return unsupported(format!("unknown measure [{name}]"));
        };
        let text = crate::udf::expand_calls(self.model, &text);
        let (expr, diags) = bicraft_dax::parse_expression(&text);
        let errors = diags.iter().any(|d| d.severity == bicraft_dax::Severity::Error);
        let Some(expr) = expr.filter(|_| !errors) else {
            return unsupported(format!("measure [{name}] does not parse"));
        };
        self.expr(&expr, &home, &Env::new(), depth.saturating_add(1))
            .map_err(|e| match e {
                Fail::Unsupported(r) => Fail::Unsupported(format!("measure [{name}]: {r}")),
                other => other,
            })
    }

    fn arg(args: &[Option<Expr>], i: usize) -> Option<&Expr> {
        args.get(i).and_then(Option::as_ref)
    }

    fn expr(&mut self, e: &Expr, home: &str, env: &Env, depth: usize) -> Result<Ir, Fail> {
        if depth > MAX_DEPTH {
            return unsupported("expression nests too deeply");
        }
        let d = depth.saturating_add(1);
        match e {
            Expr::Number { text, .. } => text
                .parse::<f64>()
                .map(Ir::Const)
                .or_else(|_| unsupported(format!("number `{text}`"))),
            Expr::Name { name, .. } => match env.get(&name.to_lowercase()) {
                Some(ir) => Ok(ir.clone()),
                None => unsupported(format!("`{name}` is not a scalar bicraft evaluates")),
            },
            Expr::Column {
                table,
                name,
                variation: None,
                ..
            } => {
                if let Some((t, _)) = self.find_measure(table.as_deref(), name)
                    && table.as_deref().is_none_or(|x| x.eq_ignore_ascii_case(&t))
                {
                    return self.measure(Some(&t), name, d);
                }
                unsupported(format!("a bare column reference [{name}] needs row context"))
            }
            Expr::Column { .. } => unsupported("column variations"),
            Expr::Unary { op, operand, .. } => {
                let inner = self.expr(operand, home, env, d)?;
                match op.as_str() {
                    "-" => Ok(Ir::Neg(Box::new(inner))),
                    "+" => Ok(inner),
                    other => unsupported(format!("operator `{other}`")),
                }
            }
            Expr::Binary { op, lhs, rhs, .. } => {
                let op = match op.as_str() {
                    "+" => Op::Add,
                    "-" => Op::Sub,
                    "*" => Op::Mul,
                    "/" => Op::Div,
                    other => return unsupported(format!("operator `{other}`")),
                };
                let l = self.expr(lhs, home, env, d)?;
                let r = self.expr(rhs, home, env, d)?;
                Ok(Ir::Bin(op, Box::new(l), Box::new(r)))
            }
            Expr::Var { bindings, body, .. } => {
                let mut env = env.clone();
                for b in bindings {
                    let ir = self.expr(&b.value, home, &env, d)?;
                    env.insert(b.name.to_lowercase(), ir);
                }
                self.expr(body, home, &env, d)
            }
            Expr::Call { name, args, .. } => self.call(name, args, home, env, d),
            Expr::Str { .. } => unsupported("text values"),
            Expr::Bool { .. } => unsupported("logical values"),
            Expr::TableCtor { .. } | Expr::Tuple { .. } => unsupported("table expressions"),
        }
    }

    fn call(
        &mut self,
        name: &str,
        args: &[Option<Expr>],
        home: &str,
        env: &Env,
        d: usize,
    ) -> Result<Ir, Fail> {
        let upper = name.to_ascii_uppercase();
        let n = args.len();
        let agg = match upper.as_str() {
            "SUM" => Some(Agg::Sum),
            "AVERAGE" => Some(Agg::Avg),
            "MIN" if n == 1 => Some(Agg::Min),
            "MAX" if n == 1 => Some(Agg::Max),
            "MEDIAN" => Some(Agg::Median),
            "STDEV.S" => Some(Agg::StdDev),
            "VAR.S" => Some(Agg::Var),
            "STDEV.P" => Some(Agg::StdDevP),
            "VAR.P" => Some(Agg::VarP),
            "COUNT" | "COUNTA" => Some(Agg::Count),
            "COUNTBLANK" => Some(Agg::CountBlank),
            "DISTINCTCOUNT" => Some(Agg::Distinct),
            "DISTINCTCOUNTNOBLANK" => Some(Agg::DistinctNoBlank),
            _ => None,
        };
        if let Some(agg) = agg {
            // A column variation (`T[Date].[Year]`) is left to the general evaluator.
            let Some(Expr::Column {
                table,
                name: col,
                variation: None,
                ..
            }) = Self::arg(args, 0).filter(|_| n == 1)
            else {
                return unsupported(format!("{upper} of something other than a column"));
            };
            let (t, c) = self.column(table.as_deref().unwrap_or(home), col)?;
            return Ok(self.leaf(Leaf {
                agg,
                table: t,
                column: Some(c),
            }));
        }
        match upper.as_str() {
            "COUNTROWS" => {
                let Some(Expr::Name { name: t, .. }) = Self::arg(args, 0).filter(|_| n == 1) else {
                    return unsupported("COUNTROWS of a table expression");
                };
                let Some(table) = model_table(self.model, t) else {
                    return unsupported(format!("unknown table '{t}'"));
                };
                let table = table.name.clone();
                Ok(self.leaf(Leaf {
                    agg: Agg::CountRows,
                    table,
                    column: None,
                }))
            }
            "MIN" | "MAX" if n == 2 => {
                let (Some(a), Some(b)) = (Self::arg(args, 0), Self::arg(args, 1)) else {
                    return unsupported(format!("{upper} with a missing argument"));
                };
                let a = Box::new(self.expr(a, home, env, d)?);
                let b = Box::new(self.expr(b, home, env, d)?);
                Ok(if upper == "MIN" {
                    Ir::Min2(a, b)
                } else {
                    Ir::Max2(a, b)
                })
            }
            "DIVIDE" if n == 2 || n == 3 => {
                let (Some(a), Some(b)) = (Self::arg(args, 0), Self::arg(args, 1)) else {
                    return unsupported("DIVIDE with a missing argument");
                };
                let a = Box::new(self.expr(a, home, env, d)?);
                let b = Box::new(self.expr(b, home, env, d)?);
                let alt = match Self::arg(args, 2) {
                    Some(x) => Some(Box::new(self.expr(x, home, env, d)?)),
                    None => None,
                };
                Ok(Ir::Divide(a, b, alt))
            }
            "BLANK" if n == 0 => Ok(Ir::Blank),
            "CALCULATE" if n == 1 => match Self::arg(args, 0) {
                Some(x) => self.expr(x, home, env, d),
                None => unsupported("CALCULATE without an expression"),
            },
            _ => unsupported(format!("{upper}() is not evaluated by the preview")),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// values
// ---------------------------------------------------------------------------------------------

/// An evaluated cell.
#[derive(Debug, Clone, PartialEq)]
enum Out {
    Blank,
    Num(f64),
    /// A column value (text, date… from MIN/MAX).
    Cell(CellValue),
}

fn as_num(v: &CellValue) -> Option<f64> {
    match v {
        CellValue::Int(n) => Some(*n as f64),
        CellValue::Real(x) | CellValue::DateTime(x) => Some(*x),
        CellValue::Decimal(n) => Some(*n as f64 / 10_000.0),
        CellValue::Bool(_) | CellValue::Text(_) | CellValue::Null => None,
    }
}

fn out_num(o: &Out) -> Option<f64> {
    match o {
        Out::Blank => None,
        Out::Num(x) => Some(*x),
        Out::Cell(v) => as_num(v),
    }
}

fn num_or_blank(x: Option<f64>) -> Out {
    x.map_or(Out::Blank, Out::Num)
}

/// Evaluates a measure tree for one row: `leaves` are the row's aggregation values and `kinds`
/// their types (Currency results are rounded to 4 decimals as the engine does).
fn eval(ir: &Ir, leaves: &[Out], kinds: &[Kind]) -> Out {
    eval_k(ir, leaves, kinds).0
}

/// A Currency result of `a op b` when the operation gives one (`None`: use the double result).
fn currency_op(op: char, a: Option<f64>, ka: Kind, b: Option<f64>, kb: Kind) -> Option<Out> {
    if currency::binary_kind(op, ka, kb) != Kind::Currency {
        return None;
    }
    let (x, y) = match op {
        // BLANK + x = x (BLANK counts as the whole number 0)
        '+' | '-' => {
            if a.is_none() && b.is_none() {
                return None;
            }
            (a.unwrap_or(0.0), b.unwrap_or(0.0))
        }
        // x / 0 and x / BLANK keep the double path (the engine reports an error)
        '/' if b.is_none_or(|y| y == 0.0) => return None,
        _ => (a?, b?),
    };
    let ka = if a.is_none() { Kind::Int } else { ka };
    let kb = if b.is_none() { Kind::Int } else { kb };
    // An overflow keeps the double result.
    currency::binary(op, x, ka, y, kb)?.map(|n| Out::Num(currency::to_f64(n)))
}

fn eval_k(ir: &Ir, leaves: &[Out], kinds: &[Kind]) -> (Out, Kind) {
    let ev = |x: &Ir| eval_k(x, leaves, kinds);
    match ir {
        Ir::Const(x) => (Out::Num(*x), Kind::of_value(*x)),
        Ir::Blank => (Out::Blank, Kind::Int),
        Ir::Leaf(i) => (
            leaves.get(*i).cloned().unwrap_or(Out::Blank),
            kinds.get(*i).copied().unwrap_or(Kind::Real),
        ),
        Ir::Neg(x) => {
            let (o, k) = ev(x);
            (num_or_blank(out_num(&o).map(|v| -v)), k)
        }
        Ir::Bin(op, a, b) => {
            let ((oa, ka), (ob, kb)) = (ev(a), ev(b));
            let (a, b) = (out_num(&oa), out_num(&ob));
            let c = match op {
                Op::Add => '+',
                Op::Sub => '-',
                Op::Mul => '*',
                Op::Div => '/',
            };
            let kind = currency::binary_kind(c, ka, kb);
            if let Some(o) = currency_op(c, a, ka, b, kb) {
                return (o, kind);
            }
            let out = num_or_blank(match op {
                // BLANK + x = x; BLANK + BLANK = BLANK
                Op::Add | Op::Sub if a.is_none() && b.is_none() => None,
                Op::Add => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
                Op::Sub => Some(a.unwrap_or(0.0) - b.unwrap_or(0.0)),
                // BLANK * x = BLANK
                Op::Mul => a.zip(b).map(|(a, b)| a * b),
                // BLANK / x = BLANK; x / BLANK = x / 0
                Op::Div => a.map(|a| a / b.unwrap_or(0.0)),
            });
            (out, kind)
        }
        Ir::Divide(a, b, alt) => {
            let ((oa, ka), (ob, kb)) = (ev(a), ev(b));
            let (a, b) = (out_num(&oa), out_num(&ob));
            let kind = currency::binary_kind('/', ka, kb);
            match (a, b) {
                (_, None) | (_, Some(0.0)) => alt.as_ref().map_or((Out::Blank, kind), |x| ev(x)),
                (None, Some(_)) => (Out::Blank, kind),
                (Some(x), Some(y)) => (currency_op('/', a, ka, b, kb).unwrap_or(Out::Num(x / y)), kind),
            }
        }
        Ir::Min2(a, b) | Ir::Max2(a, b) => {
            let ((oa, ka), (ob, kb)) = (ev(a), ev(b));
            let (a, b) = (out_num(&oa), out_num(&ob));
            let kind = if ka == kb { ka } else { Kind::Real };
            // MIN/MAX treat BLANK as 0 when the other side is not blank
            let out = num_or_blank(match (a, b) {
                (None, None) => None,
                (a, b) => {
                    let (a, b) = (a.unwrap_or(0.0), b.unwrap_or(0.0));
                    Some(if matches!(ir, Ir::Min2(..)) {
                        a.min(b)
                    } else {
                        a.max(b)
                    })
                }
            });
            (out, kind)
        }
    }
}

/// Days since 1899-12-30 → ISO 8601 (date only at midnight).
fn ole_to_iso(x: f64) -> Option<String> {
    if !x.is_finite() || x.abs() > 3_000_000.0 {
        return None;
    }
    let days = x.floor();
    let mut ms = ((x - days) * 86_400_000.0).round() as i64;
    let mut days = days as i64;
    if ms >= 86_400_000 {
        days += 1;
        ms -= 86_400_000;
    }
    let (y, m, d) = bicraft_locale::calendar::civil_from_serial(days);
    if ms == 0 {
        return Some(format!("{y:04}-{m:02}-{d:02}"));
    }
    let s = ms / 1000;
    Some(format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        s / 3600,
        (s / 60) % 60,
        s % 60
    ))
}

fn num_json(x: f64) -> Value {
    if !x.is_finite() {
        Value::Null
    } else if x.fract() == 0.0 && x.abs() < 9.0e15 {
        json!(x as i64)
    } else {
        json!(x)
    }
}

pub(crate) fn cell_json(v: &CellValue) -> Value {
    match v {
        CellValue::Null => Value::Null,
        CellValue::Int(n) => json!(n),
        CellValue::Real(x) => num_json(*x),
        CellValue::Decimal(n) => num_json(*n as f64 / 10_000.0),
        CellValue::Bool(b) => json!(b),
        CellValue::Text(s) => json!(s),
        CellValue::DateTime(x) => ole_to_iso(*x).map_or(Value::Null, Value::String),
    }
}

fn out_json(o: &Out) -> Value {
    match o {
        Out::Blank => Value::Null,
        Out::Num(x) => num_json(*x),
        Out::Cell(v) => cell_json(v),
    }
}

/// Total order for sorting: blank first, then numbers, then text (case-insensitive), then bools.
fn compare(a: &Out, b: &Out) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    fn rank(o: &Out) -> (u8, f64, String) {
        match o {
            Out::Blank | Out::Cell(CellValue::Null) => (0, 0.0, String::new()),
            Out::Num(x) => (1, *x, String::new()),
            Out::Cell(CellValue::Text(s)) => (2, 0.0, s.to_lowercase()),
            Out::Cell(CellValue::Bool(b)) => (3, f64::from(u8::from(*b)), String::new()),
            Out::Cell(v) => (1, as_num(v).unwrap_or(0.0), String::new()),
        }
    }
    let (ra, rb) = (rank(a), rank(b));
    ra.0.cmp(&rb.0)
        .then_with(|| ra.1.partial_cmp(&rb.1).unwrap_or(Ordering::Equal))
        .then_with(|| ra.2.cmp(&rb.2))
}

// ---------------------------------------------------------------------------------------------
// aggregation
// ---------------------------------------------------------------------------------------------

enum Acc {
    /// `exact`: the sum in 1/10 000 units, for Currency columns.
    Sum {
        sum: f64,
        n: u64,
        exact: i128,
    },
    Count(u64),
    Distinct(HashSet<u32>),
    Extreme(Option<u32>),
    List(Vec<f64>),
}

impl Acc {
    fn new(agg: Agg) -> Acc {
        match agg {
            Agg::Sum | Agg::Avg => Acc::Sum {
                sum: 0.0,
                n: 0,
                exact: 0,
            },
            Agg::Count | Agg::CountBlank | Agg::CountRows => Acc::Count(0),
            Agg::Distinct | Agg::DistinctNoBlank => Acc::Distinct(HashSet::new()),
            Agg::Min | Agg::Max => Acc::Extreme(None),
            Agg::Median | Agg::StdDev | Agg::Var | Agg::StdDevP | Agg::VarP => Acc::List(Vec::new()),
        }
    }
}

/// Per-code facts of the aggregated column.
struct ColView<'c> {
    col: &'c ColData,
    nums: Vec<Option<f64>>,
    /// Currency column: each code's value in 1/10 000 units.
    scaled: Option<Vec<i64>>,
    /// Sort rank of each code (for MIN/MAX).
    rank: Vec<u32>,
}

impl<'c> ColView<'c> {
    fn new(col: &'c ColData, agg: Agg) -> Self {
        let nums: Vec<Option<f64>> = col.values.iter().map(as_num).collect();
        let scaled = col
            .values
            .iter()
            .any(|v| matches!(v, CellValue::Decimal(_)))
            .then(|| {
                col.values
                    .iter()
                    .map(|v| match v {
                        CellValue::Decimal(n) => *n,
                        other => as_num(other).and_then(currency::from_f64).unwrap_or(0),
                    })
                    .collect()
            });
        let mut rank = Vec::new();
        if matches!(agg, Agg::Min | Agg::Max) {
            let mut order: Vec<u32> = (0..u32::try_from(col.values.len()).unwrap_or(u32::MAX)).collect();
            order.sort_by(|a, b| {
                let va = col.values.get(*a as usize).cloned().map_or(Out::Blank, Out::Cell);
                let vb = col.values.get(*b as usize).cloned().map_or(Out::Blank, Out::Cell);
                compare(&va, &vb)
            });
            rank = vec![0u32; col.values.len()];
            for (r, code) in order.iter().enumerate() {
                if let Some(slot) = rank.get_mut(*code as usize) {
                    *slot = u32::try_from(r).unwrap_or(u32::MAX);
                }
            }
        }
        ColView {
            col,
            nums,
            scaled,
            rank,
        }
    }
}

fn add(acc: &mut Acc, agg: Agg, view: Option<&ColView<'_>>, code: u32) {
    let num = || view.and_then(|v| v.nums.get(code as usize).copied().flatten());
    match acc {
        Acc::Sum { sum, n, exact } => {
            if let Some(x) = num() {
                *sum += x;
                *n = n.saturating_add(1);
                if let Some(units) = view
                    .and_then(|v| v.scaled.as_ref())
                    .and_then(|s| s.get(code as usize))
                {
                    *exact = exact.saturating_add(i128::from(*units));
                }
            }
        }
        Acc::Count(n) => {
            let counted = match agg {
                Agg::CountRows => true,
                Agg::CountBlank => code == 0,
                _ => code != 0,
            };
            if counted {
                *n = n.saturating_add(1);
            }
        }
        Acc::Distinct(set) => {
            if code != 0 || agg == Agg::Distinct {
                set.insert(code);
            }
        }
        Acc::Extreme(best) => {
            if code == 0 {
                return;
            }
            let Some(v) = view else {
                return;
            };
            let r = v.rank.get(code as usize).copied().unwrap_or(0);
            let better = match best {
                None => true,
                Some(b) => {
                    let rb = v.rank.get(*b as usize).copied().unwrap_or(0);
                    if agg == Agg::Min { r < rb } else { r > rb }
                }
            };
            if better {
                *best = Some(code);
            }
        }
        Acc::List(xs) => {
            if let Some(x) = num() {
                xs.push(x);
            }
        }
    }
}

/// The type of an aggregation's result (Currency rules, see [`currency`]).
fn leaf_kind(agg: Agg, view: Option<&ColView<'_>>) -> Kind {
    let currency = view.is_some_and(|v| v.scaled.is_some());
    let whole = view.is_some_and(|v| {
        v.col
            .values
            .iter()
            .all(|x| matches!(x, CellValue::Int(_) | CellValue::Null))
    });
    match agg {
        Agg::Count | Agg::CountBlank | Agg::Distinct | Agg::DistinctNoBlank | Agg::CountRows => Kind::Int,
        Agg::Sum | Agg::Min | Agg::Max if currency => Kind::Currency,
        Agg::Sum | Agg::Min | Agg::Max if whole => Kind::Int,
        Agg::Avg | Agg::Median | Agg::StdDev | Agg::StdDevP if currency => Kind::Currency,
        _ => Kind::Real,
    }
}

fn finish(acc: Acc, agg: Agg, view: Option<&ColView<'_>>) -> Out {
    let currency = view.is_some_and(|v| v.scaled.is_some());
    match acc {
        Acc::Sum { n: 0, .. } => Out::Blank,
        // Currency: exact sum; the average rounded half away from zero to 4 decimals.
        Acc::Sum { exact, n, .. } if currency => {
            let units = match agg {
                Agg::Avg => currency::div_round(exact, i128::from(n)),
                _ => i64::try_from(exact).ok(),
            };
            units.map_or(Out::Blank, |u| Out::Num(currency::to_f64(u)))
        }
        Acc::Sum { sum, n, .. } => match agg {
            Agg::Avg => Out::Num(sum / n as f64),
            _ => Out::Num(sum),
        },
        Acc::Count(0) => Out::Blank,
        Acc::Count(n) => Out::Num(n as f64),
        Acc::Distinct(set) if set.is_empty() => Out::Blank,
        Acc::Distinct(set) => Out::Num(set.len() as f64),
        Acc::Extreme(None) => Out::Blank,
        Acc::Extreme(Some(code)) => match view.and_then(|v| v.col.values.get(code as usize)) {
            Some(v) if as_num(v).is_some() && !matches!(v, CellValue::DateTime(_)) => num_or_blank(as_num(v)),
            Some(v) => Out::Cell(v.clone()),
            None => Out::Blank,
        },
        Acc::List(mut xs) => {
            if xs.is_empty() {
                return Out::Blank;
            }
            let n = xs.len() as f64;
            match agg {
                Agg::Median => {
                    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    let mid = xs.len() / 2;
                    let hi = xs.get(mid).copied().unwrap_or(0.0);
                    if xs.len() % 2 == 1 {
                        Out::Num(hi)
                    } else {
                        let lo = xs.get(mid.saturating_sub(1)).copied().unwrap_or(hi);
                        let mid = (lo + hi) / 2.0;
                        Out::Num(if currency { currency::round4(mid) } else { mid })
                    }
                }
                _ => {
                    // the sample forms divide by n - 1 (blank under two values), the population
                    // forms by n
                    let population = matches!(agg, Agg::StdDevP | Agg::VarP);
                    if xs.len() < if population { 1 } else { 2 } {
                        return Out::Blank;
                    }
                    let mean = xs.iter().sum::<f64>() / n;
                    let div = if population { n } else { n - 1.0 };
                    let var = xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / div;
                    Out::Num(match agg {
                        // STDEV.S of Currency is Currency; VAR.S is a double
                        Agg::StdDev | Agg::StdDevP if currency => currency::round4(var.sqrt()),
                        Agg::StdDev | Agg::StdDevP => var.sqrt(),
                        _ => var,
                    })
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// grouping
// ---------------------------------------------------------------------------------------------

/// A grouping column.
struct Group {
    table: String,
    column: String,
    data: Arc<ColData>,
}

/// Rows of one table mapped to groups.
struct TableGroups {
    /// Group index of each row.
    row_group: Vec<u32>,
    /// Mixed-radix key of each group.
    keys: Vec<u128>,
}

/// Directed relationship edges usable for grouping: (from table, from column) → (to table, to
/// column), many-to-one (or one-to-one, both ways), active.
fn edges(model: &SemanticModel) -> Vec<(String, String, String, String)> {
    let one = |c: Option<&str>| c.is_none_or(|c| c.eq_ignore_ascii_case("one"));
    let mut out = Vec::new();
    for r in &model.relationships {
        let Relationship {
            from_table,
            from_column,
            to_table,
            to_column,
            ..
        } = r;
        if !r.is_active {
            continue;
        }
        let from_one = r
            .from_cardinality
            .as_deref()
            .is_some_and(|c| c.eq_ignore_ascii_case("one"));
        if one(r.to_cardinality.as_deref()) {
            out.push((
                from_table.clone(),
                from_column.clone(),
                to_table.clone(),
                to_column.clone(),
            ));
            if from_one {
                out.push((
                    to_table.clone(),
                    to_column.clone(),
                    from_table.clone(),
                    from_column.clone(),
                ));
            }
        }
    }
    out
}

/// The shortest chain of edges from `from` to `to` (table names, case-insensitive).
fn path(edges: &[(String, String, String, String)], from: &str, to: &str) -> Option<Vec<usize>> {
    let mut frontier: Vec<(String, Vec<usize>)> = vec![(from.to_lowercase(), Vec::new())];
    let mut seen: HashSet<String> = HashSet::from([from.to_lowercase()]);
    let goal = to.to_lowercase();
    for _ in 0..MAX_HOPS {
        let mut next = Vec::new();
        for (table, hops) in &frontier {
            for (i, e) in edges.iter().enumerate() {
                if e.0.to_lowercase() != *table {
                    continue;
                }
                let target = e.2.to_lowercase();
                let mut h = hops.clone();
                h.push(i);
                if target == goal {
                    return Some(h);
                }
                if seen.insert(target.clone()) {
                    next.push((target, h));
                }
            }
        }
        frontier = next;
    }
    None
}

struct Eval<'c, 'a> {
    ctx: &'c mut Ctx<'a>,
    groups: Vec<Group>,
    edges: Vec<(String, String, String, String)>,
    /// Per table (lower-cased): its groups.
    table_groups: HashMap<String, Arc<TableGroups>>,
    /// Rows the visual's outer filters leave, per table (lower-cased); a table absent or
    /// `None` is not filtered.
    masks: general::Masks,
}

/// `row_group` of a row the outer filters hide.
const HIDDEN: u32 = u32::MAX;
/// Most possible group keys [`Eval::table_groups`] numbers through a table of slots (4 bytes
/// each) instead of a hash map: one map entry per row was most of a cold visual's time over a
/// million rows.
const MAX_DENSE_KEYS: usize = 1 << 20;
/// A slot of no group yet (a group number is at most `u32::MAX - 1`).
const NO_GROUP: u32 = u32::MAX;

impl Eval<'_, '_> {
    /// Group-column codes of each row of `table`.
    fn codes_for(&mut self, table: &str, g: usize) -> Result<Vec<u32>, Fail> {
        let Some(group) = self.groups.get(g) else {
            return Ok(Vec::new());
        };
        let (gtable, gcol, gdata) = (group.table.clone(), group.column.clone(), Arc::clone(&group.data));
        if gtable.eq_ignore_ascii_case(table) {
            return Ok(gdata.codes.plain().to_vec());
        }
        let Some(hops) = path(&self.edges, table, &gtable) else {
            return unsupported(format!(
                "no many-to-one relationship path from '{table}' to '{gtable}' (grouping by '{gtable}'[{gcol}])"
            ));
        };
        let rows = self.ctx.rows(table)?;
        // row of `table` → row of the current hop's table (u32::MAX: no match)
        let mut map: Vec<u32> = (0..u32::try_from(rows).unwrap_or(u32::MAX)).collect();
        for h in hops {
            let Some((ft, fc, tt, tc)) = self.edges.get(h).cloned() else {
                continue;
            };
            let from = self.ctx.column(&ft, &fc)?;
            let to = self.ctx.column(&tt, &tc)?;
            // Keys match as Analysis Services stores them, not case-insensitively.
            let keys = crate::datasrc::relationship_keys(self.ctx.model);
            let mut row_of_key: HashMap<Key, u32> = HashMap::new();
            for (r, code) in to.codes.iter().enumerate() {
                if *code == 0 {
                    continue;
                }
                if let Some(k) = to.values.get(*code as usize).and_then(|v| Key::related(v, &keys)) {
                    row_of_key
                        .entry(k)
                        .or_insert(u32::try_from(r).unwrap_or(u32::MAX));
                }
            }
            let target_of_code: Vec<u32> = from
                .values
                .iter()
                .map(|v| {
                    Key::related(v, &keys)
                        .and_then(|k| row_of_key.get(&k).copied())
                        .unwrap_or(u32::MAX)
                })
                .collect();
            for m in &mut map {
                *m = from
                    .codes
                    .get(*m as usize)
                    .and_then(|c| target_of_code.get(c as usize))
                    .copied()
                    .unwrap_or(u32::MAX);
            }
        }
        Ok(map
            .iter()
            .map(|r| gdata.codes.get(*r as usize).unwrap_or(0))
            .collect())
    }

    fn table_groups(&mut self, table: &str) -> Result<Arc<TableGroups>, Fail> {
        let lower = table.to_lowercase();
        if let Some(t) = self.table_groups.get(&lower) {
            return Ok(Arc::clone(t));
        }
        let rows = self.ctx.rows(table)?;
        let mut keys_by_row: Vec<u128> = vec![0; rows];
        let mut radix: u128 = 1;
        for g in 0..self.groups.len() {
            let codes = self.codes_for(table, g)?;
            if codes.len() != rows {
                return Err(Fail::Miss(Miss::Error(format!(
                    "columns of '{table}' have different lengths"
                ))));
            }
            let card = self.groups.get(g).map_or(1, |gr| gr.data.values.len().max(1)) as u128;
            for (k, c) in keys_by_row.iter_mut().zip(&codes) {
                *k = k.saturating_add(u128::from(*c).saturating_mul(radix));
            }
            radix = radix
                .checked_mul(card)
                .ok_or_else(|| Fail::Unsupported("too many grouping columns".into()))?;
        }
        let mask = self.masks.get(&lower).cloned().flatten();
        // the group of each key: a slot per possible key when they are few (the product of the
        // grouping columns' cardinalities), else a hash map; groups are numbered in first-row
        // order either way
        let mut dense: Option<Vec<u32>> = usize::try_from(radix)
            .ok()
            .filter(|n| *n <= MAX_DENSE_KEYS)
            .map(|n| vec![NO_GROUP; n]);
        let mut index: HashMap<u128, u32> = HashMap::new();
        let mut keys = Vec::new();
        let mut row_group = Vec::with_capacity(rows);
        for (row, k) in keys_by_row.into_iter().enumerate() {
            if mask
                .as_ref()
                .is_some_and(|m| !m.get(row).copied().unwrap_or(false))
            {
                row_group.push(HIDDEN);
                continue;
            }
            let next = u32::try_from(keys.len()).unwrap_or(u32::MAX - 1);
            let slot = dense
                .as_mut()
                .and_then(|d| usize::try_from(k).ok().and_then(|i| d.get_mut(i)));
            let gi = match slot {
                Some(s) => {
                    if *s == NO_GROUP {
                        *s = next;
                    }
                    *s
                }
                None => *index.entry(k).or_insert(next),
            };
            if gi == next {
                keys.push(k);
            }
            row_group.push(gi);
        }
        let t = Arc::new(TableGroups { row_group, keys });
        self.table_groups.insert(lower, Arc::clone(&t));
        Ok(t)
    }

    /// The value of `leaf` per group key, in first-row order (a `Vec`, not a hash map: the
    /// order rows are first seen in must not depend on hash iteration, or tied sorts and
    /// rendered PNGs would vary from run to run).
    fn leaf_values(&mut self, leaf: &Leaf) -> Result<(Vec<(u128, Out)>, Kind), Fail> {
        let tg = self.table_groups(&leaf.table)?;
        let col = match &leaf.column {
            Some(c) => Some(self.ctx.column(&leaf.table, c)?),
            None => None,
        };
        if let Some(c) = &col
            && c.rows() != tg.row_group.len()
        {
            return Err(Fail::Miss(Miss::Error(format!(
                "columns of '{}' have different lengths",
                leaf.table
            ))));
        }
        let view = col.as_deref().map(|c| ColView::new(c, leaf.agg));
        let mut accs: Vec<Acc> = (0..tg.keys.len()).map(|_| Acc::new(leaf.agg)).collect();
        for (row, gi) in tg.row_group.iter().enumerate() {
            let code = col.as_ref().and_then(|c| c.codes.get(row)).unwrap_or(1);
            if let Some(acc) = accs.get_mut(*gi as usize) {
                add(acc, leaf.agg, view.as_ref(), code);
            }
        }
        let kind = leaf_kind(leaf.agg, view.as_ref());
        let values = accs
            .into_iter()
            .zip(&tg.keys)
            .map(|(acc, k)| (*k, finish(acc, leaf.agg, view.as_ref())))
            .collect();
        Ok((values, kind))
    }

    /// A table every grouping column is reachable from (for column-only visuals).
    fn base_table(&self) -> Option<String> {
        self.groups
            .iter()
            .map(|g| g.table.clone())
            .find(|t| {
                self.groups
                    .iter()
                    .all(|g| g.table.eq_ignore_ascii_case(t) || path(&self.edges, t, &g.table).is_some())
            })
            .or_else(|| {
                // a table outside the visual (a fact table) that reaches them all
                self.ctx.model.tables.iter().map(|t| t.name.clone()).find(|t| {
                    self.groups
                        .iter()
                        .all(|g| path(&self.edges, t, &g.table).is_some())
                })
            })
    }

    /// Group-column values of a key.
    fn decode(&self, key: u128) -> Vec<Out> {
        let mut rest = key;
        self.groups
            .iter()
            .map(|g| {
                let card = g.data.values.len().max(1) as u128;
                let code = (rest % card) as usize;
                rest /= card;
                g.data.values.get(code).cloned().map_or(Out::Blank, |v| match v {
                    CellValue::Null => Out::Blank,
                    v => Out::Cell(v),
                })
            })
            .collect()
    }
}

/// One output column of a visual.
enum Field {
    Group(usize),
    Value(Ir),
}

fn agg_of_pbir(function: &str) -> Option<Agg> {
    Some(match function {
        "Sum" => Agg::Sum,
        "Avg" => Agg::Avg,
        "Count" => Agg::Distinct,
        "Min" => Agg::Min,
        "Max" => Agg::Max,
        "CountNonNull" => Agg::Count,
        "Median" => Agg::Median,
        // Desktop 2.158's implicit "Standard deviation" / "Variance" are the population forms
        // (checked against a visual's values, docs/view-snapshot.md)
        "StandardDeviation" => Agg::StdDevP,
        "Variance" => Agg::VarP,
        _ => return None,
    })
}

fn agg_label(function: &str) -> &str {
    match function {
        "Avg" => "Average",
        "Count" => "Count (Distinct)",
        "CountNonNull" => "Count",
        "StandardDeviation" => "Standard deviation",
        other => other,
    }
}

/// Most tasks a Gantt shows (Microsoft Learn: at most 1,000 rows).
const GANTT_MAX_ROWS: usize = 1000;

fn is_gantt(v: &Visual) -> bool {
    v.visual_type
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case("ganttChart"))
}

/// Visual types whose default order is by the first value, descending: none. Without a
/// `sortDefinition`, Desktop 2.158 queries bar, column, pie, funnel, waterfall charts and treemaps
/// in category order (a treemap lays its tiles out by size but colours them in that order;
/// render oracle, docs/render.md).
fn sorts_by_value(_visual_type: &str) -> bool {
    false
}

/// Raw projections of the query, in the order of `Visual::fields`.
fn projections(v: &Visual) -> Vec<&Value> {
    let mut out = Vec::new();
    if let Some(state) = v
        .visual_extra
        .get("query")
        .and_then(|q| q.get("queryState"))
        .and_then(Value::as_object)
    {
        for role in state.values() {
            if let Some(ps) = role.get("projections").and_then(Value::as_array) {
                out.extend(ps.iter());
            }
        }
    }
    out
}

/// `v` as the evaluator reads it: its raw query rebuilt from its fields when an edit left it
/// behind (drill flags, sorts and names then line up with the fields), and its hierarchy levels
/// (`HierarchyLevel` fields) bound as their columns, named by their level; `None` when `v` needs
/// neither.
pub(crate) fn normalized(model: &SemanticModel, v: &Visual) -> Option<Visual> {
    let level = |f: &FieldRef| match f {
        FieldRef::Other { raw } => bicraft_report::query::hierarchy_level(raw),
        _ => None,
    };
    let raw = v.visual_extra.get("query");
    let stale = bicraft_report::query::derive(raw) != v.fields;
    let levels = v.fields.iter().any(|b| level(&b.field).is_some());
    if !stale && !levels {
        return None;
    }
    let mut d = v.clone();
    if stale {
        match bicraft_report::query::regenerate(raw, &v.fields) {
            Some(q) => {
                d.visual_extra.insert("query".into(), q);
            }
            None => {
                d.visual_extra.shift_remove("query");
            }
        }
    }
    for b in &mut d.fields {
        let Some((table, hierarchy, name)) = level(&b.field) else {
            continue;
        };
        let Some(column) = crate::hierarchy::level_column(model, &table, &hierarchy, &name) else {
            continue;
        };
        if b.display_name.is_none() && !name.eq_ignore_ascii_case(&column) {
            b.display_name = Some(name);
        }
        let table = model.table(&table).map_or(table, |t| t.name.clone());
        b.field = FieldRef::Column { table, column };
    }
    Some(d)
}

/// Visual types that show every level of a multi-field role at once (tables, slicers): their
/// projections' `active` flags are no drill level. A matrix's are (Desktop 2.158): it shows
/// the active levels of a hierarchy (`[false, true]`: drilled down to the second), all of them
/// expanded unless its `expansionStates` collapse some; without flags, the top level.
const ALL_LEVELS: &[&str] = &[
    "tableEx",
    "decompositionTreeVisual",
    "table",
    "slicer",
    "advancedSlicerVisual",
    "listSlicer",
    "textSlicer",
];

/// Per `Visual::fields` entry: whether it is a level of a hierarchical axis other than the one
/// displayed. A chart role with several projections carrying `active` flags is a drill
/// hierarchy; Desktop shows the levels marked `active: true` (the top one when no drill state
/// is saved, all of them when expanded), the first when none is marked.
pub(crate) fn drilled_out(v: &Visual) -> Vec<bool> {
    let mut out = Vec::with_capacity(v.fields.len());
    if v.visual_type.as_deref().is_some_and(|t| ALL_LEVELS.contains(&t)) {
        return vec![false; v.fields.len()];
    }
    let Some(state) = v
        .visual_extra
        .get("query")
        .and_then(|q| q.get("queryState"))
        .and_then(Value::as_object)
    else {
        return vec![false; v.fields.len()];
    };
    for role in state.values() {
        let Some(ps) = role.get("projections").and_then(Value::as_array) else {
            continue;
        };
        let flagged = ps.len() > 1 && ps.iter().any(|p| p.get("active").is_some());
        let active = |p: &Value| p.get("active").and_then(Value::as_bool) == Some(true);
        let any_active = ps.iter().any(active);
        for (i, p) in ps.iter().enumerate() {
            out.push(flagged && if any_active { !active(p) } else { i > 0 });
        }
    }
    if out.len() != v.fields.len() {
        return vec![false; v.fields.len()];
    }
    out
}

/// `v` at its displayed drill level (the hidden levels of a hierarchical axis removed from its
/// fields and its query), `None` when it shows every field.
/// A matrix's expansion of `role` (`n` displayed levels) as `v` shows it at its drill level:
/// (per level open, toggled member paths); `None` without expanded levels.
pub(crate) fn matrix_expansion(v: &Visual, role: &str, n: usize) -> Option<(Vec<bool>, Vec<Vec<Value>>)> {
    let d = at_drill_level(v);
    let v = d.as_ref().unwrap_or(v);
    matrix::Expansion::of(v, role, n).map(|e| (e.open, e.toggled))
}

pub(crate) fn at_drill_level(v: &Visual) -> Option<Visual> {
    // a query an edit left behind (a field bound after it was written) is read rebuilt, so the
    // drill flags line up with the fields
    let raw = v.visual_extra.get("query");
    if raw.is_some()
        && bicraft_report::query::derive(raw) != v.fields
        && let Some(q) = bicraft_report::query::regenerate(raw, &v.fields)
    {
        let mut c = v.clone();
        c.visual_extra.insert("query".into(), q);
        if bicraft_report::query::derive(c.visual_extra.get("query")) == c.fields {
            return Some(at_drill_level(&c).unwrap_or(c));
        }
    }
    let hidden = drilled_out(v);
    if !hidden.iter().any(|h| *h) {
        return None;
    }
    let mut d = v.clone();
    let mut k = 0usize;
    d.fields.retain(|_| {
        let keep = !hidden.get(k).copied().unwrap_or(false);
        k = k.saturating_add(1);
        keep
    });
    if let Some(state) = d
        .visual_extra
        .get_mut("query")
        .and_then(|q| q.get_mut("queryState"))
        .and_then(Value::as_object_mut)
    {
        let mut k = 0usize;
        for role in state.values_mut() {
            if let Some(ps) = role.get_mut("projections").and_then(Value::as_array_mut) {
                ps.retain(|_| {
                    let keep = !hidden.get(k).copied().unwrap_or(false);
                    k = k.saturating_add(1);
                    keep
                });
            }
        }
    }
    Some(d)
}

/// `(field index, descending)` from `query.sortDefinition` (a sort field is found among the
/// projections, else among the bound fields).
fn sort_spec(v: &Visual, projections: &[&Value]) -> Vec<(usize, bool)> {
    let Some(sorts) = v
        .visual_extra
        .get("query")
        .and_then(|q| q.get("sortDefinition"))
        .and_then(|s| s.get("sort"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    sorts
        .iter()
        .filter_map(|s| {
            let field = s.get("field")?;
            let i = projections
                .iter()
                .position(|p| p.get("field") == Some(field))
                .or_else(|| {
                    let f = bicraft_report::query::field_ref(Some(field));
                    v.fields.iter().position(|b| same_field(&b.field, &f))
                })?;
            let desc = s.get("direction").and_then(Value::as_str) == Some("Descending");
            Some((i, desc))
        })
        .collect()
}

fn data_type_name(model: &SemanticModel, table: &str, column: &str) -> String {
    model_table(model, table)
        .and_then(|t| t.columns.iter().find(|c| c.name.eq_ignore_ascii_case(column)))
        .and_then(|c| c.data_type.as_tmsl())
        .unwrap_or("unknown")
        .to_string()
}

#[cfg(test)]
thread_local! {
    /// The equivalence tests compute visuals the old way too: the row-by-row evaluator first,
    /// the general evaluator when it cannot, under the caller's row cap.
    static ROW_BY_ROW_FIRST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Computes visuals on this thread the old way (row-by-row evaluator first) or the new one.
#[cfg(test)]
pub(crate) fn set_row_by_row_first(on: bool) {
    ROW_BY_ROW_FIRST.with(|c| c.set(on));
}

/// A visual's values grouped by its columns: through the storage engine first (the general
/// evaluator, `SUMMARIZECOLUMNS` over the visual's fields and filters, whose aggregations are
/// grouped scans over the columns' codes, as `dax.evaluate` answers), else the row-by-row
/// evaluator ([`evaluate`]) under [`PreviewLimits::fallback_rows`] when the storage engine cannot
/// express the visual (too many groups, the work it allows). A visual neither evaluates gets the
/// reason, the row cap named when that is what stopped the fallback.
///
/// A visual of columns only (a slicer, a list of categories) keeps the row-by-row evaluator
/// first: it lists the combinations its rows hold, without a dimension's blank row the general
/// evaluator adds and without crossing tables; over the cap the general evaluator lists the
/// columns of one table, and a list over several tables is refused with the cap's reason.
fn grid(ctx: &mut Ctx<'_>, v: &Visual, q: &Query<'_>) -> Result<Grid, Fail> {
    #[cfg(test)]
    if ROW_BY_ROW_FIRST.with(std::cell::Cell::get) {
        return match row_by_row(ctx, v, q) {
            Err(Fail::Unsupported(_)) => {
                let narrow = PreviewLimits {
                    table_rows: ctx.limits.fallback_rows,
                    ..*ctx.limits
                };
                let mut c = Ctx {
                    s: ctx.s,
                    st: &mut *ctx.st,
                    model: ctx.model,
                    limits: &narrow,
                };
                general::evaluate(&mut c, v, q)
            }
            other => other,
        };
    }
    let fields = || v.fields.iter().map(|b| &b.field).chain(q.extra.iter());
    if fields().all(|f| matches!(f, FieldRef::Column { .. })) {
        return match row_by_row(ctx, v, q) {
            Err(Fail::Unsupported(_)) => general::evaluate(ctx, v, q),
            Err(Fail::Miss(Miss::Error(m))) => {
                let mut tables: Vec<String> = fields()
                    .filter_map(|f| match f {
                        FieldRef::Column { table, .. } => Some(table.to_lowercase()),
                        _ => None,
                    })
                    .collect();
                tables.sort();
                tables.dedup();
                if tables.len() > 1 {
                    return Err(Fail::Miss(Miss::Error(format!(
                        "{m}; the combinations of columns of several tables are listed by the \
                         row-by-row evaluator only"
                    ))));
                }
                general::evaluate(ctx, v, q)
            }
            other => other,
        };
    }
    let storage = match general::evaluate(ctx, v, q) {
        Ok(g) => return Ok(g),
        Err(Fail::Unsupported(r)) if r == crate::CANCELLED => return unsupported(r),
        Err(e) => e,
    };
    match (storage, row_by_row(ctx, v, q)) {
        (_, Ok(g)) => Ok(g),
        // what the row-by-row evaluator says of the data, as before it came second
        (Fail::Miss(_), Err(Fail::Miss(m))) => Err(Fail::Miss(m)),
        (Fail::Unsupported(r), Err(Fail::Miss(Miss::Error(m)))) if m.ends_with(ROW_CAP) => {
            unsupported(format!("{r}; the row-by-row evaluator could not either: {m}"))
        }
        (Fail::Unsupported(_), Err(Fail::Miss(m))) => Err(Fail::Miss(m)),
        (storage, Err(Fail::Unsupported(_))) => Err(storage),
    }
}

/// [`evaluate`] under the row-by-row evaluator's cap ([`PreviewLimits::fallback_rows`]).
fn row_by_row(ctx: &mut Ctx<'_>, v: &Visual, q: &Query<'_>) -> Result<Grid, Fail> {
    let narrow = PreviewLimits {
        table_rows: ctx.limits.fallback_rows.min(ctx.limits.table_rows),
        ..*ctx.limits
    };
    let mut c = Ctx {
        s: ctx.s,
        st: &mut *ctx.st,
        model: ctx.model,
        limits: &narrow,
    };
    evaluate(&mut c, v, q)
}

/// How the error of a table over a row cap ends ([`Ctx::rows`], [`store`]).
const ROW_CAP: &str = "rows (preview limit)";

fn evaluate(ctx: &mut Ctx<'_>, v: &Visual, q: &Query<'_>) -> Result<Grid, Fail> {
    let model = ctx.model;
    // calculation items transform measure references: only the general evaluator applies them
    if model
        .tables
        .iter()
        .any(|t| t.extra.contains_key("calculationGroup"))
    {
        return unsupported("calculation groups (general evaluator)");
    }
    let mut compiler = Compiler {
        model,
        leaves: Vec::new(),
    };
    let mut fields: Vec<Field> = Vec::new();
    let mut columns_json: Vec<Value> = Vec::new();
    let mut group_refs: Vec<(String, String)> = Vec::new();
    let mut group_names: Vec<Option<(String, String)>> = Vec::new();
    let shown = v.fields.len();
    let all = v
        .fields
        .iter()
        .map(|b| (b.role.as_str(), &b.field))
        .chain(q.extra.iter().map(|f| ("", f)));
    for (i, (role, field)) in all.enumerate() {
        let display = v.fields.get(i).and_then(|b| b.display_name.clone());
        let mut group_name = None;
        let (field, kind, name, data_type) = match field {
            FieldRef::Column { .. } if i >= shown => {
                return unsupported("a hidden grouping column");
            }
            FieldRef::Column { table, column } => {
                let (t, c) = compiler.column(table, column)?;
                let dt = data_type_name(model, &t, &c);
                group_name = Some((t.clone(), c.clone()));
                let gi = match group_refs.iter().position(|(gt, gc)| *gt == t && *gc == c) {
                    Some(gi) => gi,
                    None => {
                        group_refs.push((t, c.clone()));
                        group_refs.len().saturating_sub(1)
                    }
                };
                (Field::Group(gi), "column", c, dt)
            }
            FieldRef::Measure { table, measure } => {
                let ir = compiler.measure(Some(table.as_str()), measure, 0)?;
                (Field::Value(ir), "measure", measure.clone(), "double".to_string())
            }
            FieldRef::Aggregation {
                table,
                column,
                function,
            } => {
                let Some(agg) = agg_of_pbir(function) else {
                    return unsupported(format!("aggregation `{function}`"));
                };
                let (t, c) = compiler.column(table, column)?;
                let dt = match agg {
                    Agg::Min | Agg::Max => data_type_name(model, &t, &c),
                    Agg::Count | Agg::Distinct => "int64".to_string(),
                    _ => "double".to_string(),
                };
                let ir = compiler.leaf(Leaf {
                    agg,
                    table: t,
                    column: Some(c.clone()),
                });
                let label = format!("{} of {c}", agg_label(function));
                (Field::Value(ir), "aggregation", label, dt)
            }
            FieldRef::Other { .. } => {
                return unsupported(format!(
                    "field {} of role {} (hierarchy or expression)",
                    i.saturating_add(1),
                    role
                ));
            }
        };
        if i < shown {
            columns_json.push(json!({
                "name": display.unwrap_or(name),
                "role": role,
                "kind": kind,
                "data_type": data_type,
            }));
        }
        group_names.push(group_name);
        fields.push(field);
    }

    let mut groups = Vec::new();
    for (t, c) in &group_refs {
        groups.push(Group {
            table: t.clone(),
            column: c.clone(),
            data: ctx.column(t, c)?,
        });
    }
    let leaves = compiler.leaves;
    let edges = edges(model);
    let mut ev = Eval {
        ctx,
        groups,
        edges,
        table_groups: HashMap::new(),
        masks: general::Masks::new(),
    };
    let base = if !ev.groups.is_empty() && leaves.is_empty() {
        match ev.base_table() {
            Some(b) => Some(b),
            None => return unsupported("the grouping columns are not related by many-to-one relationships"),
        }
    } else {
        None
    };
    let mut ignored = Vec::new();
    if !q.filters.is_empty() {
        let mut tables: Vec<String> = leaves.iter().map(|l| l.table.clone()).collect();
        tables.extend(base.iter().cloned());
        let (masks, why) = general::masks(&mut *ev.ctx, q.filters, &tables)?;
        ev.masks = masks;
        ignored = why;
    }
    let has_values = fields.iter().take(shown).any(|f| matches!(f, Field::Value(_)));

    // group key → leaf values
    let mut rows: Vec<(u128, Vec<Out>)> = Vec::new();
    let mut row_of: HashMap<u128, usize> = HashMap::new();
    let mut push_key = |rows: &mut Vec<(u128, Vec<Out>)>, k: u128| -> usize {
        *row_of.entry(k).or_insert_with(|| {
            rows.push((k, vec![Out::Blank; leaves.len()]));
            rows.len().saturating_sub(1)
        })
    };
    if ev.groups.is_empty() {
        push_key(&mut rows, 0);
    } else if let Some(base) = &base {
        let tg = ev.table_groups(base)?;
        for k in &tg.keys {
            push_key(&mut rows, *k);
        }
    }
    let mut kinds = vec![Kind::Real; leaves.len()];
    for (li, leaf) in leaves.iter().enumerate() {
        let (values, kind) = ev.leaf_values(leaf)?;
        if let Some(slot) = kinds.get_mut(li) {
            *slot = kind;
        }
        for (k, out) in values {
            if out == Out::Blank {
                continue;
            }
            let r = push_key(&mut rows, k);
            if let Some(slot) = rows.get_mut(r).and_then(|(_, ls)| ls.get_mut(li)) {
                *slot = out;
            }
        }
    }

    // evaluate rows
    let mut table: Vec<Vec<Out>> = Vec::with_capacity(rows.len());
    for (k, leaf_values) in &rows {
        let group_values = ev.decode(*k);
        let mut row = Vec::with_capacity(fields.len());
        let mut any_value = false;
        for (i, f) in fields.iter().enumerate() {
            let cell = match f {
                Field::Group(g) => group_values.get(*g).cloned().unwrap_or(Out::Blank),
                Field::Value(ir) => {
                    let o = eval(ir, leaf_values, &kinds);
                    any_value |= i < shown && o != Out::Blank;
                    o
                }
            };
            row.push(cell);
        }
        // rows where every value is blank are hidden (unless nothing groups)
        if has_values && !any_value && !ev.groups.is_empty() {
            continue;
        }
        table.push(row);
    }

    let is_group: Vec<bool> = fields.iter().map(|f| matches!(f, Field::Group(_))).collect();
    Ok(Grid {
        columns: columns_json,
        is_group,
        groups: group_names,
        rows: table,
        ignored,
    })
}

/// A sortable key of a cell (blank: `None`).
fn out_key(o: &Out) -> Option<Key> {
    match o {
        Out::Blank => None,
        Out::Num(x) => Key::of(&CellValue::Real(*x)),
        Out::Cell(c) => Key::of(c),
    }
}

/// Per field: the value of its model sort-by column for each of its values (`sortByColumn`),
/// when it is a grouping column that has one.
fn sort_by_keys(ctx: &mut Ctx<'_>, grid: &Grid) -> Vec<Option<HashMap<Key, Out>>> {
    let mut out = Vec::with_capacity(grid.groups.len());
    for g in &grid.groups {
        let Some((t, c)) = g else {
            out.push(None);
            continue;
        };
        let by = model_table(ctx.model, t)
            .and_then(|tb| tb.columns.iter().find(|x| x.name.eq_ignore_ascii_case(c)))
            .and_then(|x| x.sort_by_column.clone())
            .filter(|s| !s.eq_ignore_ascii_case(c));
        let Some(by) = by else {
            out.push(None);
            continue;
        };
        let (Ok(cd), Ok(sd)) = (ctx.column(t, c), ctx.column(t, &by)) else {
            out.push(None);
            continue;
        };
        let mut map: HashMap<Key, Out> = HashMap::new();
        for (code, scode) in cd.codes.iter().zip(sd.codes.iter()) {
            let Some(k) = cd.values.get(*code as usize).and_then(Key::of) else {
                continue;
            };
            map.entry(k)
                .or_insert_with(|| match sd.values.get(*scode as usize) {
                    Some(CellValue::Null) | None => Out::Blank,
                    Some(v) => Out::Cell(v.clone()),
                });
        }
        out.push(Some(map));
    }
    out
}

/// The values of `table[column]`, each with the value of the column's model sort-by column (one
/// pair per value, as the data's JSON cells); `None` when it has no sort-by column or a column
/// cannot be read. Visual calculations order their axes with it.
fn sort_by_cells(ctx: &mut Ctx<'_>, table: &str, column: &str) -> Option<Vec<(Value, Value)>> {
    let by = model_table(ctx.model, table)
        .and_then(|tb| tb.columns.iter().find(|x| x.name.eq_ignore_ascii_case(column)))
        .and_then(|x| x.sort_by_column.clone())
        .filter(|s| !s.eq_ignore_ascii_case(column))?;
    let (Ok(cd), Ok(sd)) = (ctx.column(table, column), ctx.column(table, &by)) else {
        return None;
    };
    let mut seen: HashSet<u32> = HashSet::new();
    let mut out = Vec::new();
    for (code, scode) in cd.codes.iter().zip(sd.codes.iter()) {
        if !seen.insert(*code) {
            continue;
        }
        let Some(v) = cd.values.get(*code as usize) else {
            continue;
        };
        let s = sd.values.get(*scode as usize).map_or(Value::Null, cell_json);
        out.push((cell_json(v), s));
    }
    Some(out)
}

/// Orders the rows, applies the measure filters and conditional formatting, and builds
/// `visual.data`. `extra`: the hidden fields the grid holds after the visual's own; `bars`: the
/// error bars whose bounds are among them (`data.error_bars`).
#[allow(clippy::too_many_arguments)]
fn finish_data(
    ctx: &mut Ctx<'_>,
    v: &Visual,
    mut grid: Grid,
    extra: &[FieldRef],
    plan: &Plan,
    rules: &format::Rules,
    mut notes: Vec<String>,
    bars: &[crate::visual_extras::ErrorBars],
) -> Value {
    let shown = v.fields.len();
    let all: Vec<&FieldRef> = v.fields.iter().map(|b| &b.field).chain(extra).collect();
    let pos = |f: &FieldRef| all.iter().position(|x| same_field(x, f));
    // measure and aggregation filters, on the visual's rows
    let mut applied = plan.items.len().saturating_sub(grid.ignored.len());
    for c in &plan.values {
        let at = |o: &Operand| pos(&operand_field(o));
        match general::CCond::compile(c, &at) {
            Some(cc) => {
                grid.rows
                    .retain(|r| cc.holds(&r.iter().map(general::out_v).collect::<Vec<_>>()));
                applied = applied.saturating_add(1);
            }
            None => notes.push("a measure filter could not be evaluated".into()),
        }
    }
    notes.append(&mut grid.ignored);

    // order
    let projections = projections(v);
    let is_group = &grid.is_group;
    let grouped = is_group.iter().take(shown).any(|g| *g);
    let mut spec = sort_spec(v, &projections);
    let gantt = is_gantt(v);
    if spec.is_empty() && gantt {
        // a Gantt without a saved sort: tasks by their start, earliest first (Desktop saves
        // that sort when the start is bound, docs/formats/gantt.md)
        spec.extend(
            v.fields
                .iter()
                .position(|b| b.role.eq_ignore_ascii_case("StartDate"))
                .map(|i| (i, false)),
        );
    }
    if spec.is_empty() {
        let first_value = is_group.iter().take(shown).position(|g| !*g);
        let by_value = v.visual_type.as_deref().is_some_and(sorts_by_value);
        match first_value {
            Some(i) if by_value && grouped => spec.push((i, true)),
            _ => spec.extend(
                is_group
                    .iter()
                    .take(shown)
                    .enumerate()
                    .filter(|(_, g)| **g)
                    .map(|(i, _)| (i, false)),
            ),
        }
    }
    // ties: the other grouping columns, ascending in field order (Desktop 2.158's query orders
    // a table sorted by Date by its next columns too: rows of one date by Libellé, A to Z)
    let tie: Vec<(usize, bool)> = is_group
        .iter()
        .take(shown)
        .enumerate()
        .filter(|(i, g)| **g && !spec.iter().any(|(j, _)| j == i))
        .map(|(i, _)| (i, false))
        .collect();
    spec.extend(tie);
    let keys = if spec
        .iter()
        .any(|(i, _)| grid.groups.get(*i).is_some_and(Option::is_some))
    {
        sort_by_keys(ctx, &grid)
    } else {
        Vec::new()
    };
    let sort_value = |i: usize, o: &Out| -> Out {
        match keys.get(i).and_then(Option::as_ref) {
            Some(map) => out_key(o)
                .and_then(|k| map.get(&k).cloned())
                .unwrap_or_else(|| o.clone()),
            None => o.clone(),
        }
    };
    grid.rows.sort_by(|a, b| {
        for (i, desc) in &spec {
            let (x, y) = (a.get(*i).unwrap_or(&Out::Blank), b.get(*i).unwrap_or(&Out::Blank));
            let o = compare(&sort_value(*i, x), &sort_value(*i, y));
            let o = if *desc { o.reverse() } else { o };
            if o != std::cmp::Ordering::Equal {
                return o;
            }
        }
        std::cmp::Ordering::Equal
    });

    let cap = if gantt {
        ctx.limits.rows_per_visual.min(GANTT_MAX_ROWS)
    } else {
        ctx.limits.rows_per_visual
    };
    let total = grid.rows.len();
    let nth = |f: &FieldRef, k: usize| {
        all.iter()
            .enumerate()
            .filter(|(_, x)| same_field(x, f))
            .nth(k)
            .map(|(i, _)| i)
    };
    let styles = rules.styles(&grid.rows, cap, &nth, shown);
    let rows_json: Vec<Value> = grid
        .rows
        .iter()
        .take(cap)
        .map(|r| Value::Array(r.iter().take(shown).map(out_json).collect()))
        .collect();
    let mut o = Map::new();
    o.insert("status".into(), json!("ok"));
    o.insert("columns".into(), Value::Array(grid.columns));
    o.insert("rows".into(), Value::Array(rows_json));
    o.insert("truncated".into(), json!(total > cap));
    o.insert("total_rows".into(), json!(total));
    if !styles.is_empty() {
        o.insert("cell_styles".into(), Value::Array(styles));
    }
    let error_bars = error_bar_values(&grid.rows, cap, &all, shown, bars);
    if !error_bars.is_empty() {
        o.insert("error_bars".into(), Value::Array(error_bars));
    }
    if let Some(a) = anomaly_values(&grid.rows, cap, v) {
        o.insert("anomalies".into(), a);
    }
    if applied > 0 {
        o.insert("filters_applied".into(), json!(applied));
    }
    notes.dedup();
    if !notes.is_empty() {
        o.insert("filters_ignored".into(), Value::Bool(true));
        let mut text = notes.join("; ");
        if text.len() > 1000 {
            let cut = text.char_indices().nth(1000).map_or(text.len(), |(i, _)| i);
            text.truncate(cut);
        }
        o.insert("filters_ignored_reason".into(), json!(text));
    }
    Value::Object(o)
}

/// `data.anomalies`: `{column, sensitivity, estimate: "bicraft", expected, low, high, anomaly}`,
/// one value per row shown (null where the series has none), for a line chart's anomaly
/// detection. The values are BIcraft's own estimate (`bicraft_render::anomaly`), the ones the
/// renderer draws, not Power BI's (its algorithm is not public). No entry without the setting, a
/// legend, or four values.
fn anomaly_values(rows: &[Vec<Out>], cap: usize, v: &Visual) -> Option<Value> {
    let spec = crate::visual_extras::anomaly_detection(v)?;
    if v.fields.iter().any(|b| b.role.eq_ignore_ascii_case("Series")) {
        return None;
    }
    let vals: Vec<Option<f64>> = rows
        .iter()
        .take(cap)
        .map(|r| r.get(spec.series).and_then(out_num))
        .collect();
    let est = bicraft_render::anomaly::detect(&vals, spec.sensitivity);
    if est.is_empty() {
        return None;
    }
    let col = |f: &dyn Fn(&bicraft_render::anomaly::Expected) -> Value| -> Value {
        Value::Array(est.iter().map(|e| e.as_ref().map_or(Value::Null, f)).collect())
    };
    Some(json!({
        "column": spec.series,
        "sensitivity": num_json(spec.sensitivity),
        "estimate": "bicraft",
        "expected": col(&|e| num_json(e.expected)),
        "low": col(&|e| num_json(e.low)),
        "high": col(&|e| num_json(e.high)),
        "anomaly": col(&|e| json!(e.anomaly)),
    }))
}

/// `data.error_bars`: per error bar `{column, upper, lower}`, the bounds of each row shown (a
/// number or null). Relative bounds are the series' value plus the bound's (as Power BI users
/// report Desktop draws them; not measured). Bounds evaluated nowhere (left out of the grid when
/// the evaluation could not take them) give no entry.
fn error_bar_values(
    rows: &[Vec<Out>],
    cap: usize,
    all: &[&FieldRef],
    shown: usize,
    bars: &[crate::visual_extras::ErrorBars],
) -> Vec<Value> {
    // a hidden field's column: after the visual's own
    let hidden = |f: &FieldRef| {
        all.iter()
            .enumerate()
            .skip(shown)
            .find(|(_, x)| same_field(x, f))
            .map(|(i, _)| i)
            .or_else(|| all.iter().take(shown).position(|x| same_field(x, f)))
    };
    let mut out = Vec::new();
    for b in bars {
        let up = b.upper.as_ref().map(hidden);
        let lo = b.lower.as_ref().map(hidden);
        // a bound that was asked for and is not in the grid: nothing to draw
        if matches!(up, Some(None)) || matches!(lo, Some(None)) {
            continue;
        }
        let bound = |at: Option<Option<usize>>, r: &Vec<Out>| -> Value {
            let Some(Some(i)) = at else { return Value::Null };
            let Some(x) = r.get(i).and_then(out_num) else {
                return Value::Null;
            };
            if !b.relative {
                return num_json(x);
            }
            match r.get(b.series).and_then(out_num) {
                Some(s) => num_json(s + x),
                None => Value::Null,
            }
        };
        let (mut us, mut ls) = (Vec::new(), Vec::new());
        for r in rows.iter().take(cap) {
            us.push(bound(up, r));
            ls.push(bound(lo, r));
        }
        out.push(json!({"column": b.series, "upper": us, "lower": ls}));
    }
    out
}

#[cfg(test)]
#[path = "preview_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "preview_filters_tests.rs"]
mod filters_tests;

#[cfg(test)]
#[path = "preview_grid_tests.rs"]
mod grid_tests;

#[cfg(test)]
#[path = "preview_show_tests.rs"]
mod show_tests;

/// Tests: whether the session holds a backup handed over by `file.open`, and whether its store
/// is open.
#[cfg(test)]
pub(crate) fn seed_state(s: &Session) -> (bool, bool) {
    let st = s.preview.lock();
    (st.seeded.is_some(), matches!(st.store, Some(Ok(_))))
}

#[cfg(test)]
#[path = "memory_budget_tests.rs"]
mod memory_budget_tests;
#[cfg(test)]
#[path = "memory_limit_tests.rs"]
mod memory_limit_tests;
#[cfg(test)]
#[path = "open_seed_tests.rs"]
mod open_seed_tests;

/// Tests: gates a DAX query stops at once it holds its preview state (the main one or a fork),
/// selected by a token in its text (a `// gate:<name>` comment). A gate opens when `need` queries
/// stand at it, or when the test opens it; a query never waits more than 20 s (a gate that
/// times out records it, the test fails instead of hanging).
#[cfg(test)]
pub(crate) mod test_gate {
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    pub(crate) struct Gate {
        token: String,
        need: usize,
        state: Mutex<(usize, bool, bool)>,
        cv: Condvar,
    }

    static GATES: Mutex<Vec<Arc<Gate>>> = Mutex::new(Vec::new());

    impl Gate {
        /// A gate for queries whose text holds `// gate:<name>`.
        pub(crate) fn new(name: &str, need: usize) -> Arc<Gate> {
            let g = Arc::new(Gate {
                token: format!("// gate:{name}"),
                need,
                state: Mutex::new((0, false, false)),
                cv: Condvar::new(),
            });
            GATES.lock().unwrap().push(Arc::clone(&g));
            g
        }

        pub(crate) fn open(&self) {
            self.state.lock().unwrap().1 = true;
            self.cv.notify_all();
        }

        /// Queries that reached it.
        pub(crate) fn arrived(&self) -> usize {
            self.state.lock().unwrap().0
        }

        /// Some query gave up waiting at it.
        pub(crate) fn timed_out(&self) -> bool {
            self.state.lock().unwrap().2
        }

        fn pass(&self) {
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut st = self.state.lock().unwrap();
            st.0 += 1;
            self.cv.notify_all();
            while st.0 < self.need && !st.1 {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    st.2 = true;
                    return;
                }
                st = self.cv.wait_timeout(st, left).unwrap().0;
            }
        }
    }

    /// Removes the gate (its test is over).
    pub(crate) fn remove(g: &Arc<Gate>) {
        GATES.lock().unwrap().retain(|x| !Arc::ptr_eq(x, g));
    }

    /// Called by `dax.evaluate` once it holds its state.
    pub(crate) fn pass(text: &str) {
        let gates: Vec<Arc<Gate>> = GATES
            .lock()
            .unwrap()
            .iter()
            .filter(|g| text.contains(&g.token))
            .cloned()
            .collect();
        for g in gates {
            g.pass();
        }
    }
}
#[cfg(test)]
#[path = "concurrency_tests.rs"]
mod concurrency_tests;
