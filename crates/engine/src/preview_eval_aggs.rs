//! Report-deried aggregations, engine half (`docs/storage-engine.md`, "Aggregations"): the
//! requirements read from the report ([`crate::aggs_derive`]) mapped onto each fact table, the
//! aggregations chosen under the size rule and built from the data, and the planner that answers
//! a grouped scan of the storage engine ([`Engine::compute_with`]) from the smallest aggregation
//! that covers it exactly, the fact otherwise.
//!
//! An aggregation's grain is a set of the fact's own columns, each with a class map (a code of
//! the column → its class: the combination of the codes of the grain columns reached through it).
//! A scan is covered when every predicate bitmap and every grouping code table over the fact is
//! constant on each class: the rows it selects per group are then unions of aggregation rows.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{Value, json};

use super::super::super::store::ColData;
use super::super::{Agg, ColId, Engine, FCtx, Fail, TabId};
use super::{Acc, Entry, Lut, MAX_ENTRY_BYTES, PredSet, scan};
use crate::aggs_derive::{Derivation, derive};

/// Facts with fewer rows get no aggregation.
const MIN_FACT_ROWS: usize = 100_000;
/// An aggregation has at most the fact's rows divided by this.
const MAX_FRACTION: usize = 10;
/// All aggregations of a fact have at most the fact's rows divided by this, together.
const MAX_TOTAL_FRACTION: usize = 4;
/// Most aggregations per fact.
const MAX_AGGS: usize = 8;
/// Most combinations of classes of one aggregation (it is built by dense grouped scans).
const MAX_DENSE: usize = 1 << 24;
/// Most distinct visual grains considered per fact (the most frequent first).
const MAX_GRAINS: usize = 64;

thread_local! {
    /// Tests: aggregations on or off, and the fact size threshold, for this thread.
    static TEST_ON: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    static TEST_MIN_ROWS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Tests on small facts: the size caps are lifted (an aggregation may have as many rows as
    /// the fact), so that the planner meets many grains.
    static TEST_LOOSE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Tests: turns the aggregations on or off for this thread (`None`: the session's setting).
#[cfg(test)]
pub(crate) fn set_test_aggregations(on: Option<bool>, min_rows: Option<usize>, loose: bool) {
    TEST_ON.with(|c| c.set(on));
    TEST_MIN_ROWS.with(|c| c.set(min_rows));
    TEST_LOOSE.with(|c| c.set(loose));
}

/// (most rows of one aggregation, of all of them) for a fact of `n` rows.
fn caps(n: usize) -> (usize, usize) {
    if TEST_LOOSE.with(std::cell::Cell::get) {
        return (n, n.saturating_mul(MAX_AGGS));
    }
    (n / MAX_FRACTION, n / MAX_TOTAL_FRACTION)
}

/// The session's aggregation settings; each `None` falls back to the process' environment
/// ([`env_defaults`]). `dax.aggregations {enabled}` sets `enabled`; tests set the others, so that
/// they never depend on the environment.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AggConfig {
    /// On or off (`BICRAFT_AGGREGATIONS`).
    pub enabled: Option<bool>,
    /// Facts with fewer rows get no aggregation (`BICRAFT_AGG_MIN_ROWS`).
    pub min_rows: Option<usize>,
    /// Planned and built in the foreground on first use (`BICRAFT_AGG_SYNC`).
    pub sync: Option<bool>,
}

/// The environment's aggregation settings, read once per process.
struct EnvDefaults {
    enabled: bool,
    min_rows: usize,
    sync: bool,
}

fn env_defaults() -> &'static EnvDefaults {
    static DEFAULTS: std::sync::OnceLock<EnvDefaults> = std::sync::OnceLock::new();
    DEFAULTS.get_or_init(|| EnvDefaults {
        // on unless `0` / `off` / `false`
        enabled: !bicraft_env::var("BICRAFT_AGGREGATIONS")
            .ok()
            .is_some_and(|v| matches!(v.trim(), "0" | "off" | "false" | "OFF" | "FALSE")),
        min_rows: bicraft_env::var("BICRAFT_AGG_MIN_ROWS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(MIN_FACT_ROWS),
        sync: bicraft_env::var("BICRAFT_AGG_SYNC").is_ok_and(|v| v.trim() == "1"),
    })
}

/// The smallest fact that gets aggregations: the test override, the session's setting, else
/// `BICRAFT_AGG_MIN_ROWS`.
fn min_fact_rows(config: &AggConfig) -> usize {
    if let Some(n) = TEST_MIN_ROWS.with(std::cell::Cell::get) {
        return n;
    }
    config.min_rows.unwrap_or_else(|| env_defaults().min_rows)
}

/// Whether aggregations are on: the test override, the session's `dax.aggregations
/// {enabled}`, else `BICRAFT_AGGREGATIONS` (on unless `0` / `off` / `false`).
pub(in crate::preview) fn enabled(config: &AggConfig) -> bool {
    if let Some(on) = TEST_ON.with(std::cell::Cell::get) {
        return on;
    }
    config.enabled.unwrap_or_else(|| env_defaults().enabled)
}

/// The smallest fact that gets aggregations in an evaluation under `config`; `usize::MAX` when
/// they are off (every fact is then below it: the planner returns at once).
pub(in crate::preview) fn agg_floor(config: &AggConfig) -> usize {
    if enabled(config) {
        min_fact_rows(config)
    } else {
        usize::MAX
    }
}

/// Aggregations planned and built in the foreground on first use (`BICRAFT_AGG_SYNC=1`: to run
/// the oracles and differential checks through them; never the default).
fn sync_builds(config: &AggConfig) -> bool {
    config.sync.unwrap_or_else(|| env_defaults().sync)
}

/// One grain column of an aggregation: a column of the fact and its class map.
#[derive(Clone)]
pub(in crate::preview) struct Grain {
    col: ColId,
    label: String,
    by: Vec<String>,
    data: Arc<ColData>,
    /// Code of the fact column → class.
    class: Arc<Vec<u32>>,
    classes: usize,
    /// Class → code, when each class is one code.
    code_of: Option<Vec<u32>>,
}

/// One aggregated column of an aggregation, per aggregation row.
pub(in crate::preview) struct ValAgg {
    col: ColId,
    label: String,
    data: Arc<ColData>,
    nonblank: Vec<u64>,
    /// Exact sums: (Currency, sums, values counted).
    sums: Option<(bool, Vec<i128>, Vec<u64>)>,
    /// Codes of the lowest and highest non-blank values (0: none).
    min: Vec<u32>,
    max: Vec<u32>,
}

/// One aggregation, built.
pub(in crate::preview) struct AggTable {
    grain: Vec<Grain>,
    /// Per grain column, the class of each row.
    keys: Vec<Vec<u32>>,
    count: Vec<u64>,
    values: Vec<ValAgg>,
    from: String,
    build_ms: f64,
    /// Unique within the process: the same id is the same table (kept across report changes).
    id: u64,
}

/// The next [`AggTable::id`].
static NEXT_TABLE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl AggTable {
    fn rows(&self) -> usize {
        self.count.len()
    }

    fn bytes(&self) -> usize {
        let per_row = self.grain.len() * 4
            + 8
            + self
                .values
                .iter()
                .map(|v| 16 + v.sums.as_ref().map_or(0, |_| 24))
                .sum::<usize>();
        self.rows().saturating_mul(per_row)
    }
}

/// One aggregation chosen for a fact: its grain, built on first use.
#[derive(Clone)]
pub(in crate::preview) struct Slot {
    name: String,
    set: GrainSet,
    grain: Vec<Grain>,
    values: BTreeSet<ColId>,
    value_labels: Vec<String>,
    estimate: f64,
    covers: usize,
    table: Option<Arc<AggTable>>,
    /// Why it was not built (it is then never tried again).
    failed: Option<String>,
    /// `table` was built for an earlier report and kept (same fact, grain and values).
    reused: bool,
}

/// The aggregations of one fact table, smallest estimate first.
#[derive(Default, Clone)]
pub(in crate::preview) struct FactAggs {
    table: String,
    rows: usize,
    slots: Vec<Slot>,
    visuals: usize,
    uncovered: Vec<String>,
    notes: Vec<String>,
    plan_ms: f64,
    /// Aggregations built for this plan, and kept from an earlier one.
    built: usize,
    reused: usize,
}

impl FactAggs {
    /// Bytes of its built tables.
    fn bytes(&self) -> usize {
        self.slots
            .iter()
            .filter_map(|s| s.table.as_ref())
            .map(|t| t.bytes())
            .fold(0usize, usize::saturating_add)
    }
}

/// What makes a built aggregation reusable for a new plan of the same model and data (the
/// [`AggState`] holding it is dropped when either changes): (fact, grain, aggregated columns).
type ReuseKey = (String, GrainSet, BTreeSet<ColId>);

/// The session's aggregations: for one model, data generation and report derivation.
#[derive(Default)]
pub(in crate::preview) struct AggState {
    /// (model hash, data generation, derivation hash).
    key: (u64, u64, u64),
    /// The session revision the derivation was read at.
    revision: Option<u64>,
    derivation: Arc<Derivation>,
    facts: HashMap<String, FactAggs>,
    /// Background jobs planning and building the aggregations of a fact, by fact.
    jobs: HashMap<String, Job>,
    /// Facts a query asked for that have no aggregations yet: a job starts when the query ends.
    wanted: BTreeMap<String, usize>,
    /// Aggregations built for an earlier derivation of the report, by what they hold: a new plan
    /// of the same fact takes those it selects again ([`adopt`]) and drops the others.
    reuse: BTreeMap<ReuseKey, Arc<AggTable>>,
    /// Scans answered by an aggregation, and scans of a fact with aggregations that none covered.
    pub hits: u64,
    pub misses: u64,
}

impl AggState {
    /// A read-only copy for an evaluation running beside the session's (`preview_fork.rs`): the
    /// same plans and built tables (shared, not copied), no background job, no wish.
    pub(in crate::preview) fn snapshot(&self) -> AggState {
        AggState {
            key: self.key,
            revision: self.revision,
            derivation: Arc::clone(&self.derivation),
            facts: self.facts.clone(),
            jobs: HashMap::new(),
            wanted: BTreeMap::new(),
            reuse: self.reuse.clone(),
            hits: 0,
            misses: 0,
        }
    }

    /// What a snapshot depends on: (key, revision, plans, built tables).
    pub(in crate::preview) fn shape(&self) -> ((u64, u64, u64), Option<u64>, usize, usize) {
        let built = self
            .facts
            .values()
            .flat_map(|f| f.slots.iter())
            .filter(|s| s.table.is_some() || s.failed.is_some())
            .count();
        (self.key, self.revision, self.facts.len(), built)
    }

    /// Takes over what an evaluation on a snapshot of the same key counted and wished for.
    pub(in crate::preview) fn absorb(
        &mut self,
        key: (u64, u64, u64),
        hits: u64,
        misses: u64,
        wanted: BTreeMap<String, usize>,
    ) -> bool {
        if key != self.key {
            return false;
        }
        self.hits = self.hits.saturating_add(hits);
        self.misses = self.misses.saturating_add(misses);
        let wished = !wanted.is_empty();
        for (k, v) in wanted {
            self.wanted.entry(k).or_insert(v);
        }
        wished
    }

    /// Its key, counters and wishes, for [`AggState::absorb`].
    pub(in crate::preview) fn harvest(&mut self) -> ((u64, u64, u64), u64, u64, BTreeMap<String, usize>) {
        (self.key, self.hits, self.misses, std::mem::take(&mut self.wanted))
    }

    /// Bytes of its built tables (those of the current plans and those kept for reuse).
    pub(in crate::preview) fn bytes(&self) -> usize {
        let planned = self
            .facts
            .values()
            .flat_map(|f| f.slots.iter())
            .filter_map(|s| s.table.as_ref())
            .map(|t| t.bytes());
        let kept = self.reuse.values().map(|t| t.bytes());
        planned.chain(kept).fold(0usize, usize::saturating_add)
    }

    /// What `dax.aggregations` reports.
    pub(in crate::preview) fn report(&self) -> Value {
        let mut facts: Vec<&FactAggs> = self.facts.values().collect();
        facts.sort_by(|a, b| a.table.cmp(&b.table));
        let built = || {
            facts
                .iter()
                .flat_map(|f| f.slots.iter())
                .filter_map(|s| s.table.as_ref())
        };
        json!({
            "key": format!("{:016x}{:016x}{:016x}", self.key.0, self.key.1, self.key.2),
            "visuals": self.derivation.visuals.iter().map(|v| v.to_json()).collect::<Vec<_>>(),
            "facts": facts.iter().map(|f| json!({
                "table": f.table,
                "rows": f.rows,
                "visuals": f.visuals,
                "uncovered": f.uncovered,
                "notes": f.notes,
                "plan_ms": f.plan_ms,
                "built": f.built,
                "reused": f.reused,
                "aggregations": f.slots.iter().map(|s| json!({
                    "name": s.name,
                    "grain": s.grain.iter().map(|g| json!({
                        "column": g.label,
                        "by": g.by,
                        "classes": g.classes,
                    })).collect::<Vec<_>>(),
                    "values": s.value_labels,
                    "estimate": s.estimate.round(),
                    "covers": s.covers,
                    "built": s.table.is_some(),
                    "rows": s.table.as_ref().map(|t| t.rows()),
                    "bytes": s.table.as_ref().map(|t| t.bytes()),
                    "from": s.table.as_ref().map(|t| t.from.clone()),
                    "build_ms": s.table.as_ref().map(|t| t.build_ms),
                    "id": s.table.as_ref().map(|t| t.id),
                    "reused": s.reused,
                    "failed": s.failed,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "total_rows": built().map(|t| t.rows()).sum::<usize>(),
            "total_bytes": built().map(|t| t.bytes()).sum::<usize>(),
            "build_ms": facts.iter().map(|f| f.plan_ms).sum::<f64>()
                + built().map(|t| t.build_ms).sum::<f64>(),
            // the work of the last plans: kept aggregations cost nothing
            "rebuild_ms": facts.iter().map(|f| f.plan_ms).sum::<f64>()
                + facts
                    .iter()
                    .flat_map(|f| f.slots.iter())
                    .filter(|s| !s.reused)
                    .filter_map(|s| s.table.as_ref())
                    .map(|t| t.build_ms)
                    .sum::<f64>(),
            "built_tables": facts.iter().map(|f| f.built).sum::<usize>(),
            "reused_tables": facts.iter().map(|f| f.reused).sum::<usize>(),
            "kept_for_reuse": self.reuse.len(),
            "jobs": self.jobs.len(),
            "hits": self.hits,
            "misses": self.misses,
        })
    }
}

/// A grain: fact column → the grain columns reached through it (`{column}`: the code itself).
type GrainSet = BTreeMap<ColId, BTreeSet<ColId>>;

/// What one visual needs of the fact.
struct Need {
    grain: GrainSet,
    values: BTreeSet<ColId>,
}

/// A class map: (code → class, classes).
type ClassMap = (Arc<Vec<u32>>, usize);

fn covers(a: &GrainSet, v: &GrainSet) -> bool {
    v.iter().all(|(f, xs)| {
        a.get(f)
            .is_some_and(|ax| (ax.len() == 1 && ax.contains(f)) || xs.is_subset(ax))
    })
}

fn union(a: &GrainSet, b: &GrainSet) -> GrainSet {
    let mut out = a.clone();
    for (f, xs) in b {
        let e = out.entry(*f).or_default();
        e.extend(xs.iter().copied());
    }
    out
}

/// The expected number of distinct combinations of `rows` draws over `p` combinations.
fn estimate(p: f64, rows: f64) -> f64 {
    if p <= 0.0 {
        return 1.0;
    }
    p * (1.0 - (-rows / p).exp())
}

fn fail_text(e: Fail) -> String {
    match e {
        Fail::Unsupported(m) => m,
        Fail::Miss(_) => "data unavailable".to_string(),
    }
}

impl Engine<'_, '_> {
    /// Runs `f` outside the query's budget (building aggregations is not the query's work); the
    /// time it took is given back to the query.
    fn own_budget<R>(&mut self, f: impl FnOnce(&mut Self) -> Result<R, Fail>) -> (Result<R, Fail>, f64) {
        let started = Instant::now();
        let (work, start, max_work, max_time) = (self.work, self.start, self.max_work, self.max_time);
        self.max_work = u64::MAX;
        self.max_time = std::time::Duration::from_secs(3600);
        let r = f(self);
        let spent = started.elapsed();
        self.work = work;
        self.start = start.checked_add(spent).unwrap_or(start);
        self.max_work = max_work;
        self.max_time = max_time;
        (r, spent.as_secs_f64() * 1000.0)
    }

    /// The aggregations planned for fact `t` (`n` rows), derived on first use: the key of its
    /// entry in the session's [`AggState`]; `None` when they are off or the fact is too small.
    fn fact_plan(&mut self, t: TabId, n: usize) -> Result<Option<String>, Fail> {
        if !self.aggs_on(n) {
            return Ok(None);
        }
        refresh_derivation(self.ctx.s, &mut *self.ctx.st);
        let tname = self.tabs.get(t).map(|x| x.name.clone()).unwrap_or_default();
        let lower = tname.to_lowercase();
        let Some(state) = self.ctx.st.aggs.as_mut() else {
            return Ok(None);
        };
        if state.facts.contains_key(&lower) {
            return Ok(Some(lower));
        }
        // planned here, now: a background job for the same fact is not needed any more
        state.jobs.remove(&lower);
        let derivation = Arc::clone(&state.derivation);
        // a failure only leaves the fact without aggregations
        let (result, ms) = self.own_budget(|eng| eng.plan_fact(t, n, &tname, &derivation));
        let mut planned = result.unwrap_or_else(|e| FactAggs {
            table: tname.clone(),
            rows: n,
            notes: vec![format!("not planned: {}", fail_text(e))],
            ..FactAggs::default()
        });
        planned.plan_ms = ms;
        if let Some(state) = self.ctx.st.aggs.as_mut() {
            let pool = take_reuse(&mut state.reuse, &lower);
            adopt(&mut planned, &lower, pool);
            state.facts.insert(lower.clone(), planned);
        }
        Ok(Some(lower))
    }

    /// Aggregations are on for this evaluation and a fact of `n` rows.
    /// Two comparisons: the smallest fact with aggregations is resolved once per evaluation
    /// ([`agg_floor`]), `usize::MAX` when they are off.
    #[inline]
    fn aggs_on(&self, n: usize) -> bool {
        n >= self.agg_floor && matches!(&self.store, Some(s) if !s.private)
    }

    /// The session's aggregations follow its revision, model and data: a new key drops them
    /// (and cancels their background jobs).
    fn sync_aggs(&mut self) {
        let s = self.ctx.s;
        let st = &mut *self.ctx.st;
        let model_hash = st.model_hash(s);
        let same = st
            .aggs
            .as_ref()
            .is_some_and(|a| (a.key.0, a.key.1) == (model_hash, st.generation));
        if !same {
            // new data or a new model: what was built is dropped (its jobs cancelled); the report
            // is derived again off the query's path
            st.aggs = Some(AggState {
                key: (model_hash, st.generation, 0),
                ..AggState::default()
            });
        }
    }

    /// Aggregation `i` of the fact at `key`, built on first use (from the smallest built
    /// aggregation that covers it, else from the fact); `None` when it cannot be built.
    fn slot_table(
        &mut self,
        key: &str,
        i: usize,
        t: TabId,
        n: usize,
        build: bool,
    ) -> Result<Option<Arc<AggTable>>, Fail> {
        let (grain, values, set, source) = {
            let Some(fact) = self.ctx.st.aggs.as_ref().and_then(|a| a.facts.get(key)) else {
                return Ok(None);
            };
            let Some(slot) = fact.slots.get(i) else {
                return Ok(None);
            };
            if let Some(tab) = &slot.table {
                return Ok(Some(Arc::clone(tab)));
            }
            if slot.failed.is_some() || !build {
                return Ok(None);
            }
            let source = fact
                .slots
                .iter()
                .enumerate()
                .filter_map(|(j, s)| s.table.as_ref().map(|tab| (j, s, tab)))
                .filter(|(_, s, tab)| {
                    covers(&s.set, &slot.set)
                        && slot.values.iter().all(|v| tab.values.iter().any(|x| x.col == *v))
                })
                .min_by_key(|(_, _, tab)| tab.rows())
                .map(|(j, _, tab)| (j, Arc::clone(tab)));
            (slot.grain.clone(), slot.values.clone(), slot.set.clone(), source)
        };
        let _ = set;
        let (built, ms) = self.own_budget(|eng| match &source {
            Some((_, src)) => eng.fold_agg(src, grain, &values),
            None => eng.scan_agg(t, n, grain, &values),
        });
        let (cap_one, _) = caps(n);
        let outcome = match built {
            Ok(Some(mut a)) if a.rows() <= cap_one => {
                a.build_ms = ms;
                a.id = NEXT_TABLE_ID.fetch_add(1, Ordering::Relaxed);
                a.from = match &source {
                    Some((j, _)) => format!("aggregation {}", j + 1),
                    None => "fact".into(),
                };
                Ok(Arc::new(a))
            }
            Ok(Some(a)) => Err(format!("{} rows, over the size rule", a.rows())),
            Ok(None) => Err("not buildable".to_string()),
            Err(e) => Err(fail_text(e)),
        };
        let Some(fact) = self.ctx.st.aggs.as_mut().and_then(|a| a.facts.get_mut(key)) else {
            return Ok(None);
        };
        if outcome.is_ok() {
            fact.built = fact.built.saturating_add(1);
        }
        let Some(slot) = fact.slots.get_mut(i) else {
            return Ok(None);
        };
        match outcome {
            Ok(tab) => {
                slot.table = Some(Arc::clone(&tab));
                Ok(Some(tab))
            }
            Err(why) => {
                slot.failed = Some(why);
                Ok(None)
            }
        }
    }

    /// Plans and builds the aggregations of every fact the report needs (`dax.aggregations
    /// {build}`).
    pub(in crate::preview) fn build_all_aggs(&mut self) -> Result<(), Fail> {
        let facts: BTreeSet<String> = match (self.ctx.s.model.as_ref(), self.ctx.s.report.as_ref()) {
            (Some(m), Some(r)) => derive(m, r)
                .visuals
                .iter()
                .flat_map(|v| v.facts.iter().cloned())
                .collect(),
            _ => BTreeSet::new(),
        };
        for f in facts {
            let Some(t) = self.table_id(&f) else { continue };
            let n = self.rows(t)?;
            let Some(key) = self.fact_plan(t, n)? else {
                continue;
            };
            let count = self
                .ctx
                .st
                .aggs
                .as_ref()
                .and_then(|a| a.facts.get(&key))
                .map_or(0, |f| f.slots.len());
            // the finest first, so that the coarser ones fold its rows
            for i in (0..count).rev() {
                self.slot_table(&key, i, t, n, true)?;
            }
        }
        Ok(())
    }

    /// The class map of fact column `f` by grain columns `xs` (reached through `f`).
    fn class_map(&mut self, t: TabId, f: ColId, xs: &BTreeSet<ColId>) -> Result<Option<ClassMap>, Fail> {
        let fd = self.col(f)?;
        let card = fd.values.len();
        let fc = FCtx::default();
        let mut maps: Vec<Option<Vec<u32>>> = Vec::new();
        for x in xs {
            if *x == f {
                maps.push(None);
                continue;
            }
            let Some(l) = self.group_lut(&fc, t, *x)? else {
                return Ok(None);
            };
            if l.col != f {
                return Ok(None);
            }
            maps.push(l.map.clone());
        }
        let mut ids: HashMap<Vec<u32>, u32> = HashMap::new();
        let mut class = Vec::with_capacity(card);
        let mut tuple = vec![0u32; maps.len()];
        for code in 0..card {
            let c = u32::try_from(code).unwrap_or(u32::MAX);
            for (slot, m) in tuple.iter_mut().zip(&maps) {
                *slot = match m {
                    None => c,
                    Some(m) => m.get(code).copied().unwrap_or(0),
                };
            }
            let next = u32::try_from(ids.len()).unwrap_or(u32::MAX);
            let id = *ids.entry(tuple.clone()).or_insert(next);
            class.push(id);
        }
        Ok(Some((Arc::new(class), ids.len())))
    }

    /// Chooses the aggregations of fact `t` (see the module docs and the size rule).
    fn plan_fact(&mut self, t: TabId, n: usize, tname: &str, d: &Derivation) -> Result<FactAggs, Fail> {
        let mut out = FactAggs {
            table: tname.to_string(),
            rows: n,
            ..FactAggs::default()
        };
        let fc = FCtx::default();
        let reach = self.reach(&fc, t)?;
        // 1. each visual's needs, on the fact's columns
        let mut needs: Vec<Need> = Vec::new();
        for v in &d.visuals {
            if !v.facts.iter().any(|f| f.eq_ignore_ascii_case(tname)) {
                continue;
            }
            out.visuals += 1;
            let mut need = Need {
                grain: GrainSet::new(),
                values: BTreeSet::new(),
            };
            let mut ok = true;
            for (tab, col) in &v.grain {
                let Some(x) = self.col_id(tab, col) else { continue };
                if !reach.get(self.col_table(x)?).copied().unwrap_or(false) {
                    continue;
                }
                match self.group_lut(&fc, t, x)? {
                    Some(l) => {
                        need.grain.entry(l.col).or_default().insert(x);
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            for (tab, col) in &v.values {
                if tab.eq_ignore_ascii_case(tname)
                    && let Some(c) = self.col_id(tab, col)
                {
                    need.values.insert(c);
                }
            }
            if ok {
                needs.push(need);
            } else {
                out.uncovered.push(format!("{}/{}", v.page, v.visual));
            }
        }
        // 2. normalized grains (a class that is the code itself becomes the column itself)
        let mut classes: HashMap<(ColId, Vec<ColId>), Option<ClassMap>> = HashMap::new();
        let mut normalize = |eng: &mut Self, g: &GrainSet| -> Result<Option<GrainSet>, Fail> {
            let mut out = GrainSet::new();
            for (f, xs) in g {
                let key = (*f, xs.iter().copied().collect::<Vec<_>>());
                if !classes.contains_key(&key) {
                    let m = eng.class_map(t, *f, xs)?;
                    classes.insert(key.clone(), m);
                }
                let Some(Some((_, k))) = classes.get(&key) else {
                    return Ok(None);
                };
                let card = eng.col(*f)?.values.len();
                if *k >= card || xs.contains(f) {
                    out.insert(*f, BTreeSet::from([*f]));
                } else {
                    out.insert(*f, xs.clone());
                }
            }
            Ok(Some(out))
        };
        let mut kept: Vec<Need> = Vec::new();
        for need in needs {
            if let Some(g) = normalize(self, &need.grain)? {
                kept.push(Need {
                    grain: g,
                    values: need.values,
                });
            }
        }
        // 3. candidates: the distinct grains (most frequent first), their pairwise unions, all
        let mut freq: BTreeMap<GrainSet, usize> = BTreeMap::new();
        for k in &kept {
            *freq.entry(k.grain.clone()).or_default() += 1;
        }
        let mut grains: Vec<(GrainSet, usize)> = freq.into_iter().collect();
        grains.sort_by_key(|a| std::cmp::Reverse(a.1));
        grains.truncate(MAX_GRAINS);
        let mut cands: BTreeSet<GrainSet> = grains.iter().map(|g| g.0.clone()).collect();
        for (i, a) in grains.iter().enumerate() {
            for b in grains.iter().skip(i + 1) {
                if let Some(u) = normalize(self, &union(&a.0, &b.0))? {
                    cands.insert(u);
                }
            }
        }
        if let Some(all) = grains.iter().map(|g| g.0.clone()).reduce(|a, b| union(&a, &b))
            && let Some(u) = normalize(self, &all)?
        {
            cands.insert(u);
        }
        // sizes
        let rows = n as f64;
        let (cap_one, cap_all) = caps(n);
        let cap = cap_one as f64;
        let mut sized: Vec<(GrainSet, f64, String)> = Vec::new();
        for g in cands {
            let mut p = 1usize;
            let mut fits = true;
            for (f, xs) in &g {
                let key = (*f, xs.iter().copied().collect::<Vec<_>>());
                if !classes.contains_key(&key) {
                    let m = self.class_map(t, *f, xs)?;
                    classes.insert(key.clone(), m);
                }
                match classes.get(&key) {
                    Some(Some((_, k))) => match p.checked_mul((*k).max(1)) {
                        Some(x) if x <= MAX_DENSE => p = x,
                        _ => fits = false,
                    },
                    _ => fits = false,
                }
            }
            let est = estimate(p as f64, rows);
            if fits && est <= cap {
                let name = self.grain_name(&g);
                sized.push((g, est, name));
            }
        }
        // 4. greedy choice by benefit (HRU)
        let mut chosen: Vec<usize> = Vec::new();
        let mut total = 0.0;
        let budget = cap_all as f64;
        while chosen.len() < MAX_AGGS {
            let mut best: Option<(f64, usize)> = None;
            for (i, (g, est, name)) in sized.iter().enumerate() {
                if chosen.contains(&i) || total + est > budget {
                    continue;
                }
                let mut benefit = 0.0;
                for k in &kept {
                    if !covers(g, &k.grain) {
                        continue;
                    }
                    let cost = chosen
                        .iter()
                        .filter_map(|c| sized.get(*c))
                        .filter(|(cg, _, _)| covers(cg, &k.grain))
                        .map(|(_, e, _)| *e)
                        .fold(rows, f64::min);
                    benefit += (cost - est).max(0.0);
                }
                if benefit <= 0.0 {
                    continue;
                }
                let better = match best {
                    None => true,
                    Some((b, j)) => match sized.get(j) {
                        Some((_, ej, nj)) => {
                            benefit > b || (benefit == b && (est < ej || (est == ej && name < nj)))
                        }
                        None => true,
                    },
                };
                if better {
                    best = Some((benefit, i));
                }
            }
            let Some((_, i)) = best else { break };
            total += sized.get(i).map_or(0.0, |s| s.1);
            chosen.push(i);
        }
        // 5. the plan, smallest first (the planner tries them in that order); each is built on
        // first use
        // the values of visuals without grain (cards: their scans have no filter or grouping)
        // only in the smallest aggregation
        let smallest = chosen.iter().copied().min_by(|a, b| {
            let e = |i: &usize| sized.get(*i).map_or(0.0, |s| s.1);
            e(a).total_cmp(&e(b))
        });
        let mut slots = Vec::new();
        for i in chosen {
            let Some((g, est, name)) = sized.get(i).cloned() else {
                continue;
            };
            let covered: Vec<&Need> = kept.iter().filter(|k| covers(&g, &k.grain)).collect();
            let values: BTreeSet<ColId> = covered
                .iter()
                .filter(|k| !k.grain.is_empty() || Some(i) == smallest)
                .flat_map(|k| k.values.iter().copied())
                .collect();
            let mut grain = Vec::with_capacity(g.len());
            for (f, xs) in &g {
                let key = (*f, xs.iter().copied().collect::<Vec<_>>());
                let Some(Some((class, k))) = classes.get(&key).cloned() else {
                    continue;
                };
                let data = self.col(*f)?;
                let code_of = (k == data.values.len()).then(|| {
                    let mut v = vec![0u32; k];
                    for (code, c) in class.iter().enumerate() {
                        if let Some(s) = v.get_mut(*c as usize) {
                            *s = u32::try_from(code).unwrap_or(0);
                        }
                    }
                    v
                });
                grain.push(Grain {
                    label: self.col_name(*f),
                    by: xs.iter().map(|x| self.col_name(*x)).collect(),
                    col: *f,
                    data,
                    class,
                    classes: k,
                    code_of,
                });
            }
            slots.push(Slot {
                name,
                value_labels: values.iter().map(|c| self.col_name(*c)).collect(),
                values,
                set: g,
                grain,
                estimate: est,
                covers: covered.len(),
                table: None,
                failed: None,
                reused: false,
            });
        }
        slots.sort_by(|a, b| a.estimate.total_cmp(&b.estimate));
        out.slots = slots;
        Ok(out)
    }

    fn grain_name(&self, g: &GrainSet) -> String {
        let parts: Vec<String> = g
            .iter()
            .map(|(f, xs)| {
                if xs.len() == 1 && xs.contains(f) {
                    self.col_name(*f)
                } else {
                    let by: Vec<String> = xs.iter().map(|x| self.col_name(*x)).collect();
                    by.join(" × ")
                }
            })
            .collect();
        if parts.is_empty() {
            "(total)".into()
        } else {
            parts.join(" × ")
        }
    }

    /// An aggregation built from the fact: one dense grouped scan per statistic.
    fn scan_agg(
        &mut self,
        t: TabId,
        n: usize,
        grain: Vec<Grain>,
        values: &BTreeSet<ColId>,
    ) -> Result<Option<AggTable>, Fail> {
        let keys: Vec<(ColId, Arc<Lut>)> = grain
            .iter()
            .map(|g| {
                (
                    g.col,
                    Arc::new(Lut {
                        col: g.col,
                        map: Some(g.class.as_ref().clone()),
                        card: g.classes,
                    }),
                )
            })
            .collect();
        let any = match grain.first() {
            Some(g) => g.col,
            None => match self.value_cols(t).first() {
                Some(c) => *c,
                None => return Ok(None),
            },
        };
        let dany = self.col(any)?;
        gate()?;
        let Some(e) = self.scan_entry(Agg::CountRows, any, &dany, n, PredSet::all(n), &keys)? else {
            return Ok(None);
        };
        let Acc::Rows(counts) = e.acc else {
            return Ok(None);
        };
        let cards: Vec<usize> = grain.iter().map(|g| g.classes.max(1)).collect();
        let groups: Vec<usize> = (0..counts.len())
            .filter(|g| counts.get(*g).copied().unwrap_or(0) > 0)
            .collect();
        let mut keys_out: Vec<Vec<u32>> = vec![Vec::with_capacity(groups.len()); grain.len()];
        for g in &groups {
            let mut rest = *g;
            for (i, card) in cards.iter().enumerate().rev() {
                if let Some(k) = keys_out.get_mut(i) {
                    k.push(u32::try_from(rest % card).unwrap_or(0));
                }
                rest /= card;
            }
        }
        let count: Vec<u64> = groups
            .iter()
            .map(|g| counts.get(*g).copied().unwrap_or(0))
            .collect();
        let mut vals = Vec::new();
        for c in values {
            let d = self.col(*c)?;
            let pick =
                |v: &[u64]| -> Vec<u64> { groups.iter().map(|g| v.get(*g).copied().unwrap_or(0)).collect() };
            gate()?;
            let nonblank = match self.scan_entry(Agg::Count, *c, &d, n, PredSet::all(n), &keys)? {
                Some(Entry {
                    acc: Acc::Blanks(b), ..
                }) => pick(&b.nonblank),
                _ => continue,
            };
            gate()?;
            let sums = match self.scan_entry(Agg::Sum, *c, &d, n, PredSet::all(n), &keys)? {
                Some(Entry {
                    acc: Acc::Cur { sums, counts },
                    ..
                }) => Some((true, sums, counts)),
                Some(Entry {
                    acc: Acc::Int { sums, counts },
                    ..
                }) => Some((false, sums, counts)),
                _ => None,
            }
            .map(|(cur, s, k)| {
                let s2: Vec<i128> = groups.iter().map(|g| s.get(*g).copied().unwrap_or(0)).collect();
                (cur, s2, pick(&k))
            });
            gate()?;
            let (min, max) = match self.scan_entry(Agg::Min, *c, &d, n, PredSet::all(n), &keys)? {
                Some(Entry {
                    acc: Acc::Ranks { min, max, seen },
                    ..
                }) => {
                    let by_rank = self.rank_table(&d);
                    let code = |r: u32| by_rank.1.get(r as usize).copied().unwrap_or(0);
                    let mut lo = Vec::with_capacity(groups.len());
                    let mut hi = Vec::with_capacity(groups.len());
                    for g in &groups {
                        if seen.get(*g).copied().unwrap_or(false) {
                            lo.push(code(min.get(*g).copied().unwrap_or(0)));
                            hi.push(code(max.get(*g).copied().unwrap_or(0)));
                        } else {
                            lo.push(0);
                            hi.push(0);
                        }
                    }
                    (lo, hi)
                }
                _ => continue,
            };
            vals.push(ValAgg {
                label: self.col_name(*c),
                col: *c,
                data: d,
                nonblank,
                sums,
                min,
                max,
            });
        }
        Ok(Some(AggTable {
            grain,
            keys: keys_out,
            count,
            values: vals,
            from: String::new(),
            build_ms: 0.0,
            id: 0,
        }))
    }

    /// An aggregation built from a finer one: its rows folded by the coarser classes.
    fn fold_agg(
        &mut self,
        src: &AggTable,
        grain: Vec<Grain>,
        values: &BTreeSet<ColId>,
    ) -> Result<Option<AggTable>, Fail> {
        gate()?;
        // for each target grain column: the source grain column and its class → target class
        let mut maps: Vec<(usize, Vec<u32>)> = Vec::new();
        for g in &grain {
            let Some(si) = src.grain.iter().position(|s| s.col == g.col) else {
                return Ok(None);
            };
            let Some(s) = src.grain.get(si) else {
                return Ok(None);
            };
            let mut m = vec![u32::MAX; s.classes];
            for (sc, tc) in s.class.iter().zip(g.class.iter()) {
                let Some(slot) = m.get_mut(*sc as usize) else {
                    return Ok(None);
                };
                if *slot == u32::MAX {
                    *slot = *tc;
                } else if *slot != *tc {
                    return Ok(None);
                }
            }
            maps.push((si, m));
        }
        let mut ids: HashMap<Vec<u32>, usize> = HashMap::new();
        let mut row_of = Vec::with_capacity(src.rows());
        let mut keys: Vec<Vec<u32>> = vec![Vec::new(); grain.len()];
        let mut tuple = vec![0u32; grain.len()];
        for r in 0..src.rows() {
            for (slot, (si, m)) in tuple.iter_mut().zip(&maps) {
                let sc = src.keys.get(*si).and_then(|k| k.get(r)).copied().unwrap_or(0);
                *slot = m.get(sc as usize).copied().unwrap_or(0);
            }
            let next = ids.len();
            let id = *ids.entry(tuple.clone()).or_insert_with(|| {
                for (k, v) in keys.iter_mut().zip(&tuple) {
                    k.push(*v);
                }
                next
            });
            row_of.push(id);
        }
        let rows = ids.len();
        let mut count = vec![0u64; rows];
        for (r, id) in row_of.iter().enumerate() {
            if let (Some(c), Some(x)) = (count.get_mut(*id), src.count.get(r)) {
                *c = c.saturating_add(*x);
            }
        }
        let mut vals = Vec::new();
        for c in values {
            let Some(sv) = src.values.iter().find(|v| v.col == *c) else {
                return Ok(None);
            };
            let ranks = Arc::clone(&self.rank_table(&sv.data));
            let ranks = &ranks.0;
            let rank = |code: u32| ranks.get(code as usize).copied().unwrap_or(0);
            let mut nonblank = vec![0u64; rows];
            let mut sums = sv
                .sums
                .as_ref()
                .map(|(cur, _, _)| (*cur, vec![0i128; rows], vec![0u64; rows]));
            let mut min = vec![0u32; rows];
            let mut max = vec![0u32; rows];
            for (r, id) in row_of.iter().enumerate() {
                if let (Some(a), Some(b)) = (nonblank.get_mut(*id), sv.nonblank.get(r)) {
                    *a = a.saturating_add(*b);
                }
                if let (Some((_, ts, tk)), Some((_, ss, sk))) = (sums.as_mut(), sv.sums.as_ref()) {
                    if let (Some(a), Some(b)) = (ts.get_mut(*id), ss.get(r)) {
                        *a = a.saturating_add(*b);
                    }
                    if let (Some(a), Some(b)) = (tk.get_mut(*id), sk.get(r)) {
                        *a = a.saturating_add(*b);
                    }
                }
                let (lo, hi) = (
                    sv.min.get(r).copied().unwrap_or(0),
                    sv.max.get(r).copied().unwrap_or(0),
                );
                if lo != 0
                    && let Some(m) = min.get_mut(*id)
                    && (*m == 0 || rank(lo) < rank(*m))
                {
                    *m = lo;
                }
                if hi != 0
                    && let Some(m) = max.get_mut(*id)
                    && (*m == 0 || rank(hi) > rank(*m))
                {
                    *m = hi;
                }
            }
            vals.push(ValAgg {
                label: sv.label.clone(),
                col: *c,
                data: Arc::clone(&sv.data),
                nonblank,
                sums,
                min,
                max,
            });
        }
        Ok(Some(AggTable {
            grain,
            keys,
            count,
            values: vals,
            from: String::new(),
            build_ms: 0.0,
            id: 0,
        }))
    }

    /// The entry of a grouped scan (`agg` of `c` over the rows `p` selects, grouped by `keys`)
    /// from the smallest aggregation that covers it; `None`: the fact is scanned.
    pub(super) fn agg_entry(
        &mut self,
        agg: Agg,
        c: ColId,
        d: &ColData,
        n: usize,
        p: Option<&PredSet>,
        keys: &[(ColId, Arc<Lut>)],
    ) -> Result<Option<Entry>, Fail> {
        // off, or a fact below the size floor: nothing else is looked at
        if !self.aggs_on(n) {
            return Ok(None);
        }
        if matches!(
            agg,
            Agg::Median | Agg::StdDev | Agg::Var | Agg::StdDevP | Agg::VarP
        ) {
            return Ok(None);
        }
        // without filter or grouping, a value-encoded column's sum reads its segments' statistics
        if p.is_none() && keys.is_empty() && d.codes.affine().is_some() {
            return Ok(None);
        }
        if p.is_some_and(|p| !p.masks.is_empty()) {
            return Ok(None);
        }
        let t = self.col_table(c)?;
        // never planned nor built here: a background job does it, the fact answers meanwhile
        let Some(fact) = self.fact_ready(t, n) else {
            return Ok(None);
        };
        // the key columns' data
        let mut key_data = Vec::with_capacity(keys.len());
        for (_, l) in keys {
            key_data.push(self.col(l.col)?);
        }
        // the model columns the scan reads (the data are the same decoded columns: same model, same
        // data generation, a deterministic decoding)
        let mut pred_cols = Vec::new();
        for (cd, _) in p.map(|p| p.codes.as_slice()).unwrap_or_default() {
            match self.col_of(cd) {
                Some(x) => pred_cols.push(x),
                None => return Ok(None),
            }
        }
        let key_cols: Vec<ColId> = keys.iter().map(|(_, l)| l.col).collect();
        // the aggregations whose grain has every column the scan reads
        let candidates: Vec<usize> = {
            let Some(f) = self.ctx.st.aggs.as_ref().and_then(|a| a.facts.get(&fact)) else {
                return Ok(None);
            };
            let has = |s: &Slot, x: ColId| s.grain.iter().any(|g| g.col == x);
            f.slots
                .iter()
                .enumerate()
                .filter(|(_, s)| s.failed.is_none())
                .filter(|(_, s)| pred_cols.iter().all(|x| has(s, *x)))
                .filter(|(_, s)| key_cols.iter().all(|x| has(s, *x)))
                .filter(|(_, s)| match agg {
                    Agg::CountRows => true,
                    Agg::Distinct | Agg::DistinctNoBlank => has(s, c),
                    _ => s.values.contains(&c),
                })
                .map(|(i, _)| i)
                .collect()
        };
        for i in candidates {
            let Some(a) = self.slot_table(&fact, i, t, n, false)? else {
                continue;
            };
            if let Some(e) = self.answer(&a, agg, c, d, p, &pred_cols, keys, &key_data)? {
                self.tick(a.rows())?;
                if let Some(s) = self.ctx.st.aggs.as_mut() {
                    s.hits = s.hits.saturating_add(1);
                }
                return Ok(Some(e));
            }
        }
        if let Some(s) = self.ctx.st.aggs.as_mut() {
            s.misses = s.misses.saturating_add(1);
        }
        Ok(None)
    }

    /// The entry from aggregation `a` when it covers the scan (see the module docs).
    #[allow(clippy::too_many_arguments)]
    fn answer(
        &mut self,
        a: &AggTable,
        agg: Agg,
        c: ColId,
        d: &ColData,
        p: Option<&PredSet>,
        pred_cols: &[ColId],
        keys: &[(ColId, Arc<Lut>)],
        key_data: &[Arc<ColData>],
    ) -> Result<Option<Entry>, Fail> {
        // a grain column, when its dictionary is the one the request reads
        let grain_of = |x: ColId, data: &ColData| {
            a.grain
                .iter()
                .position(|g| g.col == x && g.data.values.len() == data.values.len())
        };
        // the aggregated column
        let val = a
            .values
            .iter()
            .find(|v| v.col == c && v.data.values.len() == d.values.len());
        let distinct_grain = match agg {
            Agg::CountRows => None,
            Agg::Sum | Agg::Avg => {
                if val.and_then(|v| v.sums.as_ref()).is_none() {
                    return Ok(None);
                }
                None
            }
            Agg::Count | Agg::CountBlank | Agg::Min | Agg::Max => {
                if val.is_none() {
                    return Ok(None);
                }
                None
            }
            Agg::Distinct | Agg::DistinctNoBlank => match grain_of(c, d) {
                Some(i) if a.grain.get(i).is_some_and(|g| g.code_of.is_some()) => Some(i),
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        // predicates: per grain column, the classes allowed
        let mut allowed: Vec<Option<Vec<bool>>> = vec![None; a.grain.len()];
        if let Some(p) = p {
            for ((cd, bits), x) in p.codes.iter().zip(pred_cols) {
                let Some(i) = grain_of(*x, cd) else {
                    return Ok(None);
                };
                let Some(g) = a.grain.get(i) else { return Ok(None) };
                if bits.len() < g.class.len() {
                    return Ok(None);
                }
                let mut per: Vec<Option<bool>> = vec![None; g.classes];
                for (code, k) in g.class.iter().enumerate() {
                    let b = bits.get(code).copied().unwrap_or(false);
                    let Some(slot) = per.get_mut(*k as usize) else {
                        return Ok(None);
                    };
                    match slot {
                        None => *slot = Some(b),
                        Some(x) if *x != b => return Ok(None),
                        Some(_) => {}
                    }
                }
                let per: Vec<bool> = per.into_iter().map(|x| x.unwrap_or(false)).collect();
                let slot = allowed
                    .get_mut(i)
                    .ok_or_else(|| Fail::Unsupported("aggregation".into()))?;
                *slot = Some(match slot.take() {
                    None => per,
                    Some(prev) => prev.iter().zip(&per).map(|(x, y)| *x && *y).collect(),
                });
            }
        }
        // grouping columns: per grain column, the group code of each class
        let mut groups_of: Vec<(usize, Vec<u32>)> = Vec::with_capacity(keys.len());
        for ((_, l), kd) in keys.iter().zip(key_data) {
            let Some(i) = grain_of(l.col, kd) else {
                return Ok(None);
            };
            let Some(g) = a.grain.get(i) else { return Ok(None) };
            let mut per = vec![u32::MAX; g.classes];
            for (code, k) in g.class.iter().enumerate() {
                let gc = match &l.map {
                    None => u32::try_from(code).unwrap_or(u32::MAX),
                    Some(m) => match m.get(code) {
                        Some(x) => *x,
                        None => return Ok(None),
                    },
                };
                let Some(slot) = per.get_mut(*k as usize) else {
                    return Ok(None);
                };
                if *slot == u32::MAX {
                    *slot = gc;
                } else if *slot != gc {
                    return Ok(None);
                }
            }
            groups_of.push((i, per));
        }
        let cards: Vec<usize> = keys.iter().map(|(_, l)| l.card).collect();
        let Some(groups) = cards.iter().try_fold(1usize, |x, c| x.checked_mul((*c).max(1))) else {
            return Ok(None);
        };
        if groups > scan::MAX_GROUPS {
            return Ok(None);
        }
        // each aggregation row: visible?, its group
        let group_of_row = |r: usize| -> Option<usize> {
            for (i, al) in allowed.iter().enumerate() {
                if let Some(al) = al {
                    let k = a.keys.get(i)?.get(r)?;
                    if !al.get(*k as usize).copied().unwrap_or(false) {
                        return None;
                    }
                }
            }
            let mut g = 0usize;
            for ((i, per), card) in groups_of.iter().zip(&cards) {
                let k = a.keys.get(*i)?.get(r)?;
                let code = *per.get(*k as usize)? as usize;
                if code >= (*card).max(1) {
                    return None;
                }
                g = g.checked_mul((*card).max(1))?.checked_add(code)?;
            }
            Some(g)
        };
        let rows = a.rows();
        let acc = match agg {
            Agg::CountRows => {
                let mut v = vec![0u64; groups];
                for r in 0..rows {
                    if let Some(g) = group_of_row(r)
                        && let (Some(s), Some(x)) = (v.get_mut(g), a.count.get(r))
                    {
                        *s = s.wrapping_add(*x);
                    }
                }
                Acc::Rows(v)
            }
            Agg::Count | Agg::CountBlank => {
                let Some(val) = val else { return Ok(None) };
                let mut b = scan::BlankCounts::new(groups);
                for r in 0..rows {
                    if let Some(g) = group_of_row(r) {
                        let nb = val.nonblank.get(r).copied().unwrap_or(0);
                        let all = a.count.get(r).copied().unwrap_or(0);
                        if let Some(s) = b.nonblank.get_mut(g) {
                            *s = s.wrapping_add(nb);
                        }
                        if let Some(s) = b.blank.get_mut(g) {
                            *s = s.wrapping_add(all.saturating_sub(nb));
                        }
                    }
                }
                Acc::Blanks(b)
            }
            Agg::Sum | Agg::Avg => {
                let Some((cur, sums, counts)) = val.and_then(|v| v.sums.as_ref()) else {
                    return Ok(None);
                };
                let mut s = vec![0i128; groups];
                let mut k = vec![0u64; groups];
                for r in 0..rows {
                    if let Some(g) = group_of_row(r) {
                        if let (Some(a), Some(b)) = (s.get_mut(g), sums.get(r)) {
                            *a = a.saturating_add(*b);
                        }
                        if let (Some(a), Some(b)) = (k.get_mut(g), counts.get(r)) {
                            *a = a.wrapping_add(*b);
                        }
                    }
                }
                if *cur {
                    Acc::Cur { sums: s, counts: k }
                } else {
                    Acc::Int { sums: s, counts: k }
                }
            }
            Agg::Min | Agg::Max => {
                let Some(val) = val else { return Ok(None) };
                let ranks = Arc::clone(&self.rank_table(&val.data));
                let ranks = &ranks.0;
                let rank = |code: u32| ranks.get(code as usize).copied().unwrap_or(0);
                let mut min = vec![u32::MAX; groups];
                let mut max = vec![0u32; groups];
                let mut seen = vec![false; groups];
                for r in 0..rows {
                    let lo = val.min.get(r).copied().unwrap_or(0);
                    if lo == 0 {
                        continue;
                    }
                    let hi = val.max.get(r).copied().unwrap_or(0);
                    if let Some(g) = group_of_row(r)
                        && let (Some(a), Some(b), Some(s)) = (min.get_mut(g), max.get_mut(g), seen.get_mut(g))
                    {
                        *a = (*a).min(rank(lo));
                        *b = (*b).max(rank(hi));
                        *s = true;
                    }
                }
                Acc::Ranks { min, max, seen }
            }
            Agg::Distinct | Agg::DistinctNoBlank => {
                let Some(i) = distinct_grain else { return Ok(None) };
                let Some(code_of) = a.grain.get(i).and_then(|g| g.code_of.as_ref()) else {
                    return Ok(None);
                };
                let Some(mut s) = scan::Distinct::new(groups, d.values.len(), MAX_ENTRY_BYTES) else {
                    return Ok(None);
                };
                let w = s.words();
                for r in 0..rows {
                    if let Some(g) = group_of_row(r) {
                        let k = a.keys.get(i).and_then(|k| k.get(r)).copied().unwrap_or(0);
                        let code = code_of.get(k as usize).copied().unwrap_or(0) as usize;
                        let code = code.min(s.card.saturating_sub(1));
                        let at = g.saturating_mul(w).saturating_add(code / 64);
                        if let Some(word) = s.bits.get_mut(at) {
                            *word |= 1u64 << (code % 64);
                        }
                    }
                }
                Acc::Distinct(s)
            }
            _ => return Ok(None),
        };
        Ok(Some(Entry { cards, acc }))
    }
}

/// `dax.aggregations {build}`: derives and builds the aggregations of every fact the report
/// needs, over the session's data.
pub(in crate::preview) fn build(ctx: &mut super::super::super::Ctx<'_>) -> Result<(), String> {
    let mut eng = Engine::new(ctx);
    eng.build_all_aggs().map_err(|e| match e {
        Fail::Unsupported(m) => m,
        Fail::Miss(_) => "the data is unavailable".to_string(),
    })
}

/// Most bytes of the aggregations one background job builds for a fact.
const MAX_JOB_BYTES: usize = 1 << 30;

/// What a background job hands back: the fact's aggregations, planned and built, and the columns
/// it decoded (given to the session, so that queries do not decode them again).
type JobOut = (FactAggs, Vec<((String, String), Arc<ColData>)>);

/// A background job planning and building the aggregations of one fact. Dropping it cancels it.
pub(in crate::preview) struct Job {
    cancel: Arc<AtomicBool>,
    result: Arc<Mutex<Option<Option<JobOut>>>>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Threads of a background job's scans: a quarter of the machine (`BICRAFT_AGG_THREADS`).
fn job_threads() -> usize {
    bicraft_env::var("BICRAFT_AGG_THREADS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n >= 1)
        .unwrap_or_else(|| (scan::configured_threads() / 4).max(1))
}

type Store = super::super::super::store::VertipaqStore;
type Miss = super::super::super::store::Miss;
type Literal = super::super::super::store::LiteralTable;

/// What a background job starts from: copies of the session's model and handles to its data.
struct JobIn {
    model: bicraft_model::SemanticModel,
    derivation: Arc<Derivation>,
    store: Option<Result<Arc<Store>, String>>,
    own: Option<Option<Arc<crate::refresh::OwnData>>>,
    columns: HashMap<(String, String), Result<Arc<ColData>, Miss>>,
    literal: HashMap<String, (u64, Result<Arc<Literal>, Miss>)>,
    literal_inputs: Option<u64>,
    model_hash: u64,
    generation: u64,
    revision: u64,
    key: (u64, u64, u64),
    table: String,
    rows: usize,
    loose: bool,
    min_rows: Option<usize>,
    config: AggConfig,
    /// The aggregations of this fact built for an earlier report ([`adopt`]).
    reuse: BTreeMap<ReuseKey, Arc<AggTable>>,
    cancel: Arc<AtomicBool>,
    activity: Arc<Activity>,
    /// The session's memory budget setting (`docs/beyond-ram.md`): the job's own columns and its
    /// aggregations stay under it.
    memory: Option<Option<u64>>,
    /// The session's memory pool (lot 5): the job's reservations are background work there, and it
    /// waits while a query runs.
    pool: &'static bicraft_mem::Pool,
}

/// The job's work, on its own thread: a private session state over the same data, the plan,
/// then every aggregation, the finest first; `None` when cancelled or failed.
fn run_job(i: JobIn) -> Option<JobOut> {
    TEST_LOOSE.with(|c| c.set(i.loose));
    TEST_MIN_ROWS.with(|c| c.set(i.min_rows));
    scan::set_threads(Some(job_threads()));
    // its scans run on the background pool: a query never waits behind them for a worker
    scan::pool::set_background(Some(job_threads()));
    GATE.with(|g| *g.borrow_mut() = Some((Arc::clone(&i.activity), Arc::clone(&i.cancel), i.pool)));
    if !i.activity.wait_idle(&i.cancel, i.pool) {
        return None;
    }
    let mut s = crate::Session::new();
    s.model = Some(i.model);
    s.revision = i.revision;
    let mut st = super::super::super::State {
        revision: Some(i.revision),
        source_fp: Some(0),
        generation: i.generation,
        store: i.store,
        own: i.own,
        columns: i.columns,
        literal: i.literal,
        literal_inputs: i.literal_inputs,
        model_hash: Some(i.model_hash),
        aggs_config: i.config,
        memory: super::super::super::memory::Memory::background(i.memory, i.pool),
        ..super::super::super::State::default()
    };
    let limits = super::super::super::PreviewLimits {
        rows_per_visual: 0,
        table_rows: usize::MAX,
        fallback_rows: usize::MAX,
        budget: std::time::Duration::from_secs(3600),
        now: 0.0,
        utc_offset: 0.0,
    };
    let model = s.model.as_ref()?;
    let lower = i.table.to_lowercase();
    {
        let mut ctx = super::super::super::Ctx {
            s: &s,
            st: &mut st,
            model,
            limits: &limits,
        };
        let mut eng = Engine::new(&mut ctx);
        let t = eng.table_id(&i.table)?;
        let (planned, ms) = eng.own_budget(|e| e.plan_fact(t, i.rows, &i.table, &i.derivation));
        let mut fact = planned.ok()?;
        fact.plan_ms = ms;
        adopt(&mut fact, &lower, i.reuse);
        let count = fact.slots.len();
        let mut state = AggState {
            key: i.key,
            revision: Some(i.revision),
            derivation: Arc::clone(&i.derivation),
            ..AggState::default()
        };
        state.facts.insert(lower.clone(), fact);
        eng.ctx.st.aggs = Some(state);
        let mut bytes = 0usize;
        // under a memory budget, at most a quarter of it
        let max_bytes = eng
            .ctx
            .st
            .memory
            .budget()
            .map_or(MAX_JOB_BYTES, |b| (b / 4).min(MAX_JOB_BYTES));
        for k in (0..count).rev() {
            if i.cancel.load(Ordering::Relaxed) {
                return None;
            }
            if bytes > max_bytes {
                if let Some(slot) = eng
                    .ctx
                    .st
                    .aggs
                    .as_mut()
                    .and_then(|a| a.facts.get_mut(&lower))
                    .and_then(|f| f.slots.get_mut(k))
                {
                    slot.failed = Some("over the memory budget".into());
                }
                continue;
            }
            if let Ok(Some(tab)) = eng.slot_table(&lower, k, t, i.rows, true) {
                bytes = bytes.saturating_add(tab.bytes());
            }
        }
    }
    if i.cancel.load(Ordering::Relaxed) {
        return None;
    }
    let fact = st.aggs.take()?.facts.remove(&lower)?;
    let columns = st
        .columns
        .into_iter()
        .filter_map(|(k, v)| v.ok().map(|c| (k, c)))
        .collect();
    Some((fact, columns))
}

impl Engine<'_, '_> {
    /// The model column whose decoded data is `d` in this evaluation.
    fn col_of(&self, d: &ColData) -> Option<ColId> {
        self.cols
            .iter()
            .position(|i| i.data.as_ref().is_some_and(|x| std::ptr::eq(&**x, d)))
    }

    /// The fact's aggregations when they are planned (and, by a background job, built); never
    /// plans nor builds: when they are not there, the fact is noted as wanted (a job starts when
    /// the query ends) and the query scans the fact.
    fn fact_ready(&mut self, t: TabId, n: usize) -> Option<String> {
        if !self.aggs_on(n) {
            return None;
        }
        if sync_builds(&self.ctx.st.aggs_config) {
            // oracles and checks: planned and built in the foreground, on first use
            let key = self.fact_plan(t, n).ok()??;
            let count = self
                .ctx
                .st
                .aggs
                .as_ref()
                .and_then(|a| a.facts.get(&key))
                .map_or(0, |f| f.slots.len());
            for i in (0..count).rev() {
                self.slot_table(&key, i, t, n, true).ok()?;
            }
            return Some(key);
        }
        self.sync_aggs();
        let name = self.tabs.get(t).map(|x| x.lower.as_str())?;
        let state = self.ctx.st.aggs.as_mut()?;
        if state.facts.contains_key(name) {
            return Some(name.to_string());
        }
        let done = state
            .jobs
            .get(name)
            .and_then(|j| j.result.try_lock().ok().and_then(|mut g| g.take()));
        match done {
            Some(out) => {
                let lower = name.to_string();
                state.jobs.remove(&lower);
                // the job's plan took what it reuses: the rest of the pool for this fact goes
                drop(take_reuse(&mut state.reuse, &lower));
                let fact = match out {
                    Some((fact, columns)) => {
                        // under a memory limit what the job hands over is reserved first (lot 5):
                        // counted with this query, then with what the session keeps; what does not
                        // fit is dropped (the queries read the fact, and decode the columns again)
                        let fresh: Vec<_> = columns
                            .into_iter()
                            .filter(|(k, _)| !self.ctx.st.columns.contains_key(k))
                            .collect();
                        let bytes = fresh
                            .iter()
                            .map(|(_, c)| super::super::super::memory::base_bytes(c))
                            .fold(fact.bytes(), usize::saturating_add);
                        if self.keep_soft(&|| "the aggregations a background job built".into(), bytes) {
                            for (k, c) in fresh {
                                self.ctx.st.columns.entry(k).or_insert(Ok(c));
                            }
                            fact
                        } else {
                            FactAggs {
                                table: lower.clone(),
                                rows: n,
                                notes: vec!["over the memory limit".into()],
                                ..FactAggs::default()
                            }
                        }
                    }
                    None => FactAggs {
                        table: lower.clone(),
                        rows: n,
                        notes: vec!["the background job failed".into()],
                        ..FactAggs::default()
                    },
                };
                let state = self.ctx.st.aggs.as_mut()?;
                state.facts.insert(lower.clone(), fact);
                Some(lower)
            }
            None => {
                if !state.jobs.contains_key(name) {
                    state.wanted.insert(name.to_string(), n);
                    // a job starts when the query ends ([`start_jobs`])
                    self.ctx.s.preview.set_has_aggs();
                    self.ctx.s.preview.set_pending();
                }
                None
            }
        }
    }
}

/// Quiet time after the session's last evaluation before a background job works.
const IDLE_MS: u64 = 200;

/// The session's evaluations in flight and when the last one ended, shared with its background
/// aggregation jobs, which only work while the session is idle.
pub(in crate::preview) struct Activity {
    busy: std::sync::atomic::AtomicUsize,
    /// Milliseconds since `epoch` at the end of the last evaluation.
    last_end: std::sync::atomic::AtomicU64,
    epoch: Instant,
}

impl Default for Activity {
    fn default() -> Self {
        Activity {
            busy: std::sync::atomic::AtomicUsize::new(0),
            last_end: std::sync::atomic::AtomicU64::new(0),
            epoch: Instant::now(),
        }
    }
}

impl Activity {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    pub(in crate::preview) fn begin(&self) {
        self.busy.fetch_add(1, Ordering::SeqCst);
    }

    pub(in crate::preview) fn end(&self) {
        // never below zero
        let mut b = self.busy.load(Ordering::SeqCst);
        while b > 0 {
            match self
                .busy
                .compare_exchange(b, b - 1, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => break,
                Err(now) => b = now,
            }
        }
        self.last_end.store(self.now_ms(), Ordering::SeqCst);
    }

    /// Waits until no evaluation runs and none ended in the last [`IDLE_MS`] (and, under a memory
    /// limit, no query of any session runs in its pool: background work yields, lot 5); `false`
    /// when the job is cancelled meanwhile.
    fn wait_idle(&self, cancel: &AtomicBool, pool: &bicraft_mem::Pool) -> bool {
        loop {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            let quiet = self.now_ms().saturating_sub(self.last_end.load(Ordering::SeqCst));
            let others = pool.limit().is_some() && pool.stats().active_queries > 0;
            if self.busy.load(Ordering::SeqCst) == 0 && quiet >= IDLE_MS && !others {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

/// A background job's gate: its session's activity, its cancel flag and its memory pool.
type GateOf = (Arc<Activity>, Arc<AtomicBool>, &'static bicraft_mem::Pool);

/// Tests: [`Activity::wait_idle`].
#[cfg(test)]
pub(in crate::preview) fn wait_idle_for_tests(
    a: &Activity,
    cancel: &AtomicBool,
    pool: &bicraft_mem::Pool,
) -> bool {
    a.wait_idle(cancel, pool)
}

thread_local! {
    /// A background job's gate (its session's activity and its cancel flag), for this thread.
    static GATE: std::cell::RefCell<Option<GateOf>> =
        const { std::cell::RefCell::new(None) };
}

/// On a background job's thread: waits for the session to be idle before the next scan;
/// `Err` when the job is cancelled. Elsewhere: nothing.
fn gate() -> Result<(), Fail> {
    let open = GATE.with(|g| match g.borrow().as_ref() {
        Some((act, cancel, pool)) => act.wait_idle(cancel, pool),
        None => true,
    });
    if open {
        Ok(())
    } else {
        Err(Fail::Unsupported("the aggregation job was cancelled".into()))
    }
}

type State = super::super::super::State;

/// Reads what the report needs again when the session changed since it was last read; a new
/// derivation drops the aggregations planned for the old one (they stay exact, but no longer fit
/// the report) and cancels their jobs. Never called on a query's path.
pub(in crate::preview) fn refresh_derivation(s: &crate::Session, st: &mut State) {
    let (model_hash, generation) = (st.model_hash(s), st.generation);
    let fresh = || AggState {
        key: (model_hash, generation, 0),
        ..AggState::default()
    };
    let a = st.aggs.get_or_insert_with(fresh);
    if (a.key.0, a.key.1) != (model_hash, generation) {
        *a = fresh();
    }
    if a.revision == Some(s.revision) {
        return;
    }
    let d = match (s.model.as_ref(), s.report.as_ref()) {
        (Some(m), Some(r)) => derive(m, r),
        _ => Derivation::default(),
    };
    if a.revision.is_none() || d.hash != a.key.2 {
        // the report changed, the model and data did not: what was built stays exact; a new
        // plan takes the aggregations it selects again, the others are dropped
        harvest(a, &d);
        a.jobs.clear();
        a.key.2 = d.hash;
        a.derivation = Arc::new(d);
    }
    a.revision = Some(s.revision);
}

/// After a command's answer is sent (`Session::background_work`): reads the report again if it
/// changed, then starts a background job for each fact a query wanted aggregations for.
/// Whether [`start_jobs`] has anything to do (aggregations on, and a query wanted some or the
/// session changed since the report was last read): read before the session is synchronized.
pub(in crate::preview) fn jobs_due(s: &crate::Session, st: &State) -> bool {
    enabled(&st.aggs_config)
        && st
            .aggs
            .as_ref()
            .is_some_and(|a| !a.wanted.is_empty() || a.revision != Some(s.revision))
}

pub(in crate::preview) fn start_jobs(s: &crate::Session, st: &mut State) {
    if !jobs_due(s, st) {
        return;
    }
    refresh_derivation(s, st);
    let Some(state) = st.aggs.as_mut() else {
        return;
    };
    let wanted = std::mem::take(&mut state.wanted);
    if wanted.is_empty() {
        return;
    }
    let Some(model) = s.model.as_ref() else {
        return;
    };
    let model_hash = st.model_hash(s);
    for (lower, rows) in wanted {
        let Some(state) = st.aggs.as_ref() else { return };
        if state.facts.contains_key(&lower) || state.jobs.contains_key(&lower) {
            continue;
        }
        let Some(table) = model
            .tables
            .iter()
            .find(|x| x.name.to_lowercase() == lower)
            .map(|x| x.name.clone())
        else {
            continue;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let result: Arc<Mutex<Option<Option<JobOut>>>> = Arc::new(Mutex::new(None));
        // shared, not taken: a job cancelled by the next report change leaves them in the pool
        let reuse: BTreeMap<ReuseKey, Arc<AggTable>> = state
            .reuse
            .iter()
            .filter(|((f, _, _), _)| *f == lower)
            .map(|(k, t)| (k.clone(), Arc::clone(t)))
            .collect();
        let input = JobIn {
            model: model.clone(),
            derivation: Arc::clone(&state.derivation),
            store: st.store.clone(),
            own: st.own.clone(),
            columns: st.columns.clone(),
            literal: st.literal.clone(),
            literal_inputs: st.literal_inputs,
            model_hash,
            generation: st.generation,
            revision: state.revision.unwrap_or(0),
            key: state.key,
            table,
            rows,
            loose: TEST_LOOSE.with(std::cell::Cell::get),
            min_rows: TEST_MIN_ROWS.with(std::cell::Cell::get),
            // the job runs under the decision taken here
            config: AggConfig {
                enabled: Some(true),
                ..st.aggs_config
            },
            reuse,
            cancel: Arc::clone(&cancel),
            activity: Arc::clone(&st.activity),
            memory: st.memory.setting,
            pool: st.memory.pool(),
        };
        let out = Arc::clone(&result);
        let spawned = std::thread::Builder::new()
            .name("bicraft-aggregations".into())
            .spawn(move || {
                let mut gauge = crate::obs::AggBuild::start();
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_job(input)))
                    .ok()
                    .flatten();
                if r.is_some() {
                    gauge.built();
                }
                drop(gauge);
                if let Ok(mut g) = out.lock() {
                    *g = Some(r);
                }
            });
        if spawned.is_ok()
            && let Some(state) = st.aggs.as_mut()
        {
            state.jobs.insert(lower, Job { cancel, result });
        }
    }
}

/// A new derivation of the report (same model, same data): the built aggregations of every fact
/// move to the reuse pool (the plans are dropped, the jobs cancelled by the caller); those of
/// facts the report no longer needs are dropped now.
fn harvest(a: &mut AggState, d: &Derivation) {
    let needed: BTreeSet<String> = d
        .visuals
        .iter()
        .flat_map(|v| v.facts.iter().map(|f| f.to_lowercase()))
        .collect();
    for (lower, fact) in a.facts.drain() {
        if !needed.contains(&lower) {
            continue;
        }
        for slot in fact.slots {
            if let Some(t) = slot.table {
                a.reuse.insert((lower.clone(), slot.set, slot.values), t);
            }
        }
    }
    a.reuse.retain(|(f, _, _), _| needed.contains(f));
}

/// The pool entries of fact `lower`, taken out of `reuse` (for a new plan of it).
fn take_reuse(
    reuse: &mut BTreeMap<ReuseKey, Arc<AggTable>>,
    lower: &str,
) -> BTreeMap<ReuseKey, Arc<AggTable>> {
    let (mine, rest): (BTreeMap<_, _>, BTreeMap<_, _>) = std::mem::take(reuse)
        .into_iter()
        .partition(|((f, _, _), _)| f == lower);
    *reuse = rest;
    mine
}

/// A new plan of fact `lower` takes the pooled aggregations that hold exactly what one of its
/// slots selects (same grain, same aggregated columns: the same model and data, as the pool
/// lives in the [`AggState`] of one model hash and data generation); the rest of `pool` is
/// dropped with it.
fn adopt(fact: &mut FactAggs, lower: &str, mut pool: BTreeMap<ReuseKey, Arc<AggTable>>) {
    if pool.is_empty() {
        return;
    }
    let mut key: ReuseKey = (lower.to_string(), GrainSet::new(), BTreeSet::new());
    for slot in &mut fact.slots {
        if slot.table.is_some() || slot.failed.is_some() {
            continue;
        }
        key.1 = slot.set.clone();
        key.2 = slot.values.clone();
        if let Some(t) = pool.remove(&key) {
            slot.table = Some(t);
            slot.reused = true;
            fact.reused = fact.reused.saturating_add(1);
        }
    }
}
