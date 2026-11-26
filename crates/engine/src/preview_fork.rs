//! Concurrent evaluations on one session (`docs/web-deployment.md`, "Concurrency").
//!
//! The preview state ([`State`]) is one mutex, held for a whole evaluation: its caches (decoded
//! columns, storage-engine caches, aggregations, memory accounting) are updated as the query
//! runs. When `bicraft serve` runs read-only commands side by side on one session
//! (`Session::execute_shared`), a second evaluation would wait for the first. Instead it
//! **forks**:
//!
//! - **Main.** An evaluation that finds the state free takes it, as before: nothing else changes
//!   on that path. Whoever holds it publishes a [`Seed`] when what it holds changed (O(1) check;
//!   the seed is rebuilt only after new columns were read or the session changed).
//! - **Fork.** An evaluation that finds the state busy builds a private state from the seed: the
//!   opened backup, bicraft's own data snapshot, the model's registry and measure facts, its
//!   memory settings. The decoded columns are **weak** references, upgraded only when the fork
//!   reads them: the seed never keeps a column alive, so the memory budget can still evict any
//!   column no query holds. A fork's own memory accounting is per query, like the main's (the
//!   query budget `BICRAFT_QUERY_BUDGET` caps each evaluation; every reservation goes to the
//!   same pool, so the process limit holds across all of them). It answers from a
//!   read-only snapshot of the report-derived aggregations the main state built (same plans,
//!   same tables; its counters and build wishes go back); its storage-engine caches start empty
//!   and are dropped at the end.
//! - **Never stale.** A seed is used only for the session revision it was published at, and only
//!   when the data it points to is still alive; otherwise the evaluation waits for the main
//!   state, as before. Under the server's read lock no command can change the session while a
//!   fork runs; the checks are for every other caller.
//! - **Merge-back.** At the end, what the fork read that the main state lacks (decoded columns,
//!   literal tables, viewer results) goes back to it: at once when the main state is free, else
//!   in a short list that the next holder applies after synchronizing. An entry is applied only
//!   to the same session revision and data generation it was computed for, and only where the
//!   main state has nothing for that key; anything else is dropped (a reuse lost, never a stale
//!   value).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, MutexGuard, Weak};

use serde_json::Value;

use super::store::{ColData, LiteralTable, Miss, VertipaqStore};
use super::{PreviewCache, State, general, memory, source_fingerprint};
use crate::{Session, Source};

/// The aggregations' state.
type AggState = general::storage::aggs::AggState;
/// What a snapshot of the aggregations depends on ([`AggState::shape`]).
type AggShape = ((u64, u64, u64), Option<u64>, usize, usize);
/// What a fork hands back of the aggregations ([`AggState::harvest`]).
type AggHarvest = (
    (u64, u64, u64),
    u64,
    u64,
    std::collections::BTreeMap<String, usize>,
);

/// How long an evaluation waits for a busy main state before it forks.
pub(super) const FORK_AFTER: std::time::Duration = std::time::Duration::from_millis(50);
/// Longest wait between two tries for the main state while waiting for it.
const FREED_SLICE: std::time::Duration = std::time::Duration::from_millis(2);

/// Merge-backs waiting for the main state at most; beyond, the oldest is dropped.
const MAX_HARVESTS: usize = 8;
/// Waits for the seed of the session's revision before an evaluation waits for the main state.
const SEED_WAITS: usize = 4;
const SEED_WAIT: std::time::Duration = std::time::Duration::from_millis(10);

/// What a fork starts from, published by the main state's holder ([`PreviewCache::publish`]).
pub(super) struct Seed {
    revision: u64,
    source_fp: u64,
    generation: u64,
    literal_inputs: Option<u64>,
    model_hash: Option<u64>,
    /// The opened backup (`None`: not opened yet).
    store: Option<Result<Weak<VertipaqStore>, String>>,
    own: Option<Option<Weak<crate::refresh::OwnData>>>,
    columns: Arc<HashMap<(String, String), Weak<ColData>>>,
    literal: HashMap<String, (u64, Weak<LiteralTable>)>,
    registry: Option<Arc<general::Registry>>,
    measure_facts: Option<general::rels::MeasureFacts>,
    segs: Option<Weak<bicraft_scan::lazy::SegCache>>,
    aggs_config: general::storage::aggs::AggConfig,
    memory_setting: Option<Option<u64>>,
    query_setting: Option<Option<u64>>,
    pool: &'static bicraft_mem::Pool,
    activity: Arc<general::storage::aggs::Activity>,
    /// What the seed was built from: (columns, literal tables, store opened, own read).
    shape: (usize, usize, bool, bool, Option<AggShape>),
    /// A read-only snapshot of the report-derived aggregations (plans and built tables, shared).
    aggs: Option<Arc<AggState>>,
}

/// A fork's state: the inherited (weak) columns of its seed, and what it holds of them.
#[derive(Default)]
pub(super) struct Inherited {
    columns: Arc<HashMap<(String, String), Weak<ColData>>>,
    /// Inherited columns this fork read (strong while it runs; not counted as its own).
    held: HashMap<(String, String), Arc<ColData>>,
    /// Literal tables taken from the seed (not merged back).
    literal_keys: HashSet<String>,
}

impl Inherited {
    /// The main state's decoded columns as weak references, for the View as state
    /// (`preview_rls.rs`): read from there, never copied.
    pub(super) fn of(columns: &HashMap<(String, String), Result<Arc<ColData>, Miss>>) -> Inherited {
        Inherited {
            columns: Arc::new(
                columns
                    .iter()
                    .filter_map(|(k, c)| c.as_ref().ok().map(|c| (k.clone(), Arc::downgrade(c))))
                    .collect(),
            ),
            ..Inherited::default()
        }
    }
}

/// A literal table entry of the state: (fingerprint, data).
type LiteralEntry = (u64, Result<Arc<LiteralTable>, Miss>);

/// What a fork read that the main state may lack, for its revision and data generation.
pub(super) struct Harvest {
    revision: Option<u64>,
    generation: u64,
    source_fp: Option<u64>,
    columns: Vec<((String, String), Arc<ColData>)>,
    literal: Vec<(String, LiteralEntry)>,
    results: Vec<(u64, Value)>,
    /// What it counted on the aggregations' snapshot, and the facts it wished aggregations for.
    aggs: Option<AggHarvest>,
}

/// The state an evaluation runs on: the main one, a fork that merges back when dropped, or the
/// View as state (`preview_rls.rs`), which never merges back.
pub(in crate::preview) enum StateRef<'a> {
    Main(&'a PreviewCache, MutexGuard<'a, State>),
    Fork(&'a PreviewCache, &'a Session, Box<State>),
    Secured(MutexGuard<'a, super::rls::SecuredSlot>),
}

impl std::ops::Deref for StateRef<'_> {
    type Target = State;
    fn deref(&self) -> &State {
        match self {
            StateRef::Main(_, g) => g,
            StateRef::Fork(_, _, st) => st,
            StateRef::Secured(g) => &g.state,
        }
    }
}

impl std::ops::DerefMut for StateRef<'_> {
    fn deref_mut(&mut self) -> &mut State {
        match self {
            StateRef::Main(_, g) => g,
            StateRef::Fork(_, _, st) => st,
            StateRef::Secured(g) => &mut g.state,
        }
    }
}

impl Drop for StateRef<'_> {
    fn drop(&mut self) {
        match self {
            // what this evaluation read is offered to the forks to come
            StateRef::Main(cache, g) => {
                cache.publish(g);
                // evaluations waiting a little for the main state try again
                cache.freed.1.notify_all();
            }
            StateRef::Fork(cache, s, st) => {
                let h = harvest(st);
                // the fork's reservations go before the main state counts what it takes over
                drop(std::mem::take(&mut **st));
                cache.give_back(s, h);
            }
            // what a View as evaluation read stays in its own state
            StateRef::Secured(_) => {}
        }
    }
}

impl PreviewCache {
    /// The state an evaluation of `s` runs on, synchronized with it: the main state when it is
    /// free, else a fork when the seed fits `s`, else the main state once it is free. The data
    /// as stored: [`PreviewCache::acquire`] applies View as.
    pub(in crate::preview) fn acquire_raw<'a>(&'a self, s: &'a Session) -> StateRef<'a> {
        // a short evaluation on the main state ends soon: waiting for it a little is faster than a
        // fork, whose result caches start empty (a warm query answered from them takes a few
        // ms); a long one is not waited for
        let until = std::time::Instant::now().checked_add(FORK_AFTER);
        loop {
            match self.state.0.try_lock() {
                Ok(g) => return self.main(s, self.adopt(g)),
                Err(std::sync::TryLockError::Poisoned(p)) => {
                    return self.main(s, self.adopt_poisoned(p.into_inner()));
                }
                Err(std::sync::TryLockError::WouldBlock) => {}
            }
            let left = until.map_or(std::time::Duration::ZERO, |u| {
                u.saturating_duration_since(std::time::Instant::now())
            });
            if left.is_zero() {
                break;
            }
            // woken when an evaluation leaves the main state (it may not have unlocked yet: a
            // short slice, then try again)
            let g = self
                .freed
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            drop(self.freed.1.wait_timeout(g, left.min(FREED_SLICE)));
        }
        // the holder may not have published the seed of this revision yet (it synchronizes
        // first): a few short waits for it, then the main state as before
        for _ in 0..SEED_WAITS {
            if let Some(st) = self.fork(s) {
                self.forks.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return StateRef::Fork(self, s, Box::new(st));
            }
            let seed = self
                .seed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            drop(self.seeded_cv.wait_timeout(seed, SEED_WAIT));
            match self.state.0.try_lock() {
                Ok(g) => return self.main(s, self.adopt(g)),
                Err(std::sync::TryLockError::Poisoned(p)) => {
                    return self.main(s, self.adopt_poisoned(p.into_inner()));
                }
                Err(std::sync::TryLockError::WouldBlock) => {}
            }
        }
        self.main(s, self.lock())
    }

    fn main<'a>(&'a self, s: &Session, mut g: MutexGuard<'a, State>) -> StateRef<'a> {
        g.sync(s);
        self.drain(&mut g);
        // forks of this revision may start while this evaluation runs
        self.publish(&g);
        StateRef::Main(self, g)
    }

    /// Publishes the seed when what `st` holds changed since the last one.
    fn publish(&self, st: &State) {
        let Some(revision) = st.revision else { return };
        let shape = (
            st.columns.len(),
            st.literal.len(),
            st.store.is_some(),
            st.own.is_some(),
            st.aggs.as_ref().map(AggState::shape),
        );
        let mut seed = self
            .seed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if seed.as_ref().is_some_and(|x| {
            x.revision == revision
                && x.generation == st.generation
                && x.shape == shape
                && x.model_hash == st.model_hash
                && x.registry.as_ref().map(Arc::as_ptr) == st.registry.as_ref().map(Arc::as_ptr)
        }) {
            return;
        }
        *seed = Some(Arc::new(Seed {
            revision,
            source_fp: st.source_fp.unwrap_or(0),
            generation: st.generation,
            literal_inputs: st.literal_inputs,
            model_hash: st.model_hash,
            store: st
                .store
                .as_ref()
                .map(|r| r.as_ref().map(Arc::downgrade).map_err(Clone::clone)),
            own: st.own.as_ref().map(|o| o.as_ref().map(Arc::downgrade)),
            columns: Arc::new(
                st.columns
                    .iter()
                    .filter_map(|(k, c)| c.as_ref().ok().map(|c| (k.clone(), Arc::downgrade(c))))
                    .collect(),
            ),
            literal: st
                .literal
                .iter()
                .filter_map(|(k, (fp, t))| t.as_ref().ok().map(|t| (k.clone(), (*fp, Arc::downgrade(t)))))
                .collect(),
            registry: st.registry.clone(),
            measure_facts: st.measure_facts,
            segs: st.segs.as_ref().map(Arc::downgrade),
            aggs_config: st.aggs_config,
            memory_setting: st.memory.setting,
            query_setting: st.memory.query_setting,
            pool: st.memory.pool(),
            activity: Arc::clone(&st.activity),
            aggs: st.aggs.as_ref().map(|a| Arc::new(a.snapshot())),
            shape,
        }));
        self.seeded_cv.notify_all();
    }

    /// A fork for an evaluation of `s`, when the seed fits it (`None`: wait for the main state).
    fn fork(&self, s: &Session) -> Option<State> {
        let seed = self
            .seed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()?;
        if seed.revision != s.revision || seed.source_fp != source_fingerprint(s) {
            return None;
        }
        // the stored data must be open in the main state: a fork never opens a backup or reads
        // bicraft's own snapshot (that would duplicate them)
        let store = match &seed.store {
            Some(Ok(w)) => Some(Ok(w.upgrade()?)),
            Some(Err(e)) => Some(Err(e.clone())),
            None if seed.source_fp != 0 => return None,
            None => None,
        };
        let own = match &seed.own {
            Some(Some(w)) => Some(Some(w.upgrade()?)),
            Some(None) => Some(None),
            None if matches!(s.source, Some(Source::Project { .. })) => return None,
            None => None,
        };
        let segs = match &seed.segs {
            Some(w) => Some(w.upgrade()?),
            None => None,
        };
        let literal: HashMap<String, LiteralEntry> = seed
            .literal
            .iter()
            .filter_map(|(k, (fp, w))| w.upgrade().map(|t| (k.clone(), (*fp, Ok(t)))))
            .collect();
        let mut memory = memory::Memory::background(seed.memory_setting, seed.pool);
        memory.background = false;
        memory.query_setting = seed.query_setting;
        Some(State {
            revision: Some(seed.revision),
            source_fp: Some(seed.source_fp),
            generation: seed.generation,
            store,
            own,
            inherited: Some(Inherited {
                columns: Arc::clone(&seed.columns),
                literal_keys: literal.keys().cloned().collect(),
                ..Inherited::default()
            }),
            literal,
            literal_inputs: seed.literal_inputs,
            model_hash: seed.model_hash,
            registry: seed.registry.clone(),
            measure_facts: seed.measure_facts,
            segs,
            // the aggregations the main state built answer here too: the same plans and
            // tables, for the same revision and data (`AggState` follows both); a fork never
            // starts a background build (its wishes go back to the main state)
            aggs: seed.aggs.as_ref().map(|a| a.snapshot()),
            aggs_config: seed.aggs_config,
            activity: Arc::clone(&seed.activity),
            memory,
            ..State::default()
        })
    }

    /// Hands a fork's harvest to the main state: now when it is free, else to the next holder.
    fn give_back(&self, s: &Session, h: Harvest) {
        let aggs_empty = h
            .aggs
            .as_ref()
            .is_none_or(|(_, hits, misses, wanted)| *hits == 0 && *misses == 0 && wanted.is_empty());
        if h.columns.is_empty() && h.literal.is_empty() && h.results.is_empty() && aggs_empty {
            return;
        }
        let g = match self.state.0.try_lock() {
            Ok(g) => Some(self.adopt(g)),
            Err(std::sync::TryLockError::Poisoned(p)) => Some(self.adopt_poisoned(p.into_inner())),
            Err(std::sync::TryLockError::WouldBlock) => None,
        };
        match g {
            Some(mut g) => {
                g.sync(s);
                self.drain(&mut g);
                apply(self, &mut g, h);
                self.publish(&g);
            }
            None => {
                let mut list = self
                    .harvests
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if list.len() >= MAX_HARVESTS {
                    list.remove(0);
                }
                list.push(h);
            }
        }
    }

    /// Applies the waiting harvests to the main state (synchronized by the caller).
    fn drain(&self, st: &mut State) {
        let list = std::mem::take(
            &mut *self
                .harvests
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for h in list {
            apply(self, st, h);
        }
    }

    /// Tests: forks made, and harvests (applied, dropped as stale).
    #[cfg(test)]
    pub(crate) fn fork_counters(&self) -> (u64, u64, u64) {
        let o = std::sync::atomic::Ordering::Relaxed;
        (self.forks.load(o), self.merged.load(o), self.stale.load(o))
    }
}

/// What fork `st` read that its seed did not have.
fn harvest(st: &mut State) -> Harvest {
    let inherited = st.inherited.take().unwrap_or_default();
    Harvest {
        revision: st.revision,
        generation: st.generation,
        source_fp: st.source_fp,
        columns: st
            .columns
            .drain()
            .filter_map(|(k, c)| c.ok().map(|c| (k, c)))
            .collect(),
        literal: st
            .literal
            .drain()
            .filter(|(k, (_, t))| t.is_ok() && !inherited.literal_keys.contains(k))
            .collect(),
        // a fork starts with no viewer results: all it has it computed
        results: st.results.drain().collect(),
        aggs: st.aggs.as_mut().map(AggState::harvest),
    }
}

/// Applies `h` to the main state `st` when it is for the same revision and data; only keys the
/// main state has no entry for are filled (two forks reading one column give the same data).
fn apply(cache: &PreviewCache, st: &mut State, h: Harvest) {
    let o = std::sync::atomic::Ordering::Relaxed;
    if st.revision != h.revision || st.generation != h.generation || st.source_fp != h.source_fp {
        cache.stale.fetch_add(1, o);
        return;
    }
    let mut columns = false;
    for (k, c) in h.columns {
        if !st.columns.contains_key(&k) {
            st.memory.touch(&k);
            st.columns.insert(k, Ok(c));
            columns = true;
        }
    }
    for (k, t) in h.literal {
        st.literal.entry(k).or_insert(t);
    }
    for (k, v) in h.results {
        st.results.entry(k).or_insert(v);
    }
    if let (Some((key, hits, misses, wanted)), Some(a)) = (h.aggs, st.aggs.as_mut())
        && a.absorb(key, hits, misses, wanted)
    {
        // the main state's caller starts the builds the fork wished for
        cache.set_pending();
    }
    if columns {
        // what the main state keeps follows the memory budget and the process limit again
        st.enforce_budget();
        st.sync_kept();
    }
    cache.merged.fetch_add(1, o);
}

impl State {
    /// A fork's inherited column `key` (upgraded and held on first read); `None` elsewhere.
    pub(super) fn inherited_column(&mut self, key: &(String, String)) -> Option<Arc<ColData>> {
        let inh = self.inherited.as_mut()?;
        if let Some(c) = inh.held.get(key) {
            return Some(Arc::clone(c));
        }
        let c = inh.columns.get(key)?.upgrade()?;
        inh.held.insert(key.clone(), Arc::clone(&c));
        Some(c)
    }
}

// ---------------------------------------------------------------------------------------------
// One read per stored column: concurrent evaluations that miss the same column wait for the one
// reading it instead of each decoding a copy of it.

/// A column read: in progress (`done` is `None`), done (its column, weakly: listed while it is
/// alive somewhere, for the evaluations that miss it later), or failed.
#[derive(Default)]
pub(super) struct Loading {
    done: std::sync::Mutex<Option<Option<Weak<ColData>>>>,
    cv: std::sync::Condvar,
}

/// Column reads listed at most before the dead ones are pruned.
const MAX_LOADING: usize = 4096;

impl Loading {
    /// In progress, or done with its column still alive somewhere.
    fn live(&self) -> bool {
        match &*self
            .done
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            None => true,
            Some(Some(w)) => w.strong_count() > 0,
            Some(None) => false,
        }
    }
}

/// What [`PreviewCache::claim`] says about a column to read.
pub(super) enum Claim<'a> {
    /// This evaluation reads it, and hands it over with [`LoadGuard::finish`].
    Mine(LoadGuard<'a>),
    /// Another evaluation read it.
    Loaded(Arc<ColData>),
    /// Another evaluation could not read it: read it here.
    Failed,
}

/// The read of one column by this evaluation: dropped without [`LoadGuard::finish`], the
/// waiting evaluations read it themselves.
pub(super) struct LoadGuard<'a> {
    cache: &'a PreviewCache,
    key: (u64, String, String),
    slot: Arc<Loading>,
}

impl LoadGuard<'_> {
    pub(super) fn finish(self, c: &Arc<ColData>) {
        self.end(Some(Arc::downgrade(c)));
    }

    fn end(&self, result: Option<Weak<ColData>>) {
        let mut d = self
            .slot
            .done
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if d.is_some() {
            return; // finished already (`finish`, then drop)
        }
        let failed = result.is_none();
        *d = Some(result);
        drop(d);
        self.slot.cv.notify_all();
        if failed {
            let mut map = self
                .cache
                .loading
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if map.get(&self.key).is_some_and(|s| Arc::ptr_eq(s, &self.slot)) {
                map.remove(&self.key);
            }
        }
    }
}

impl Drop for LoadGuard<'_> {
    fn drop(&mut self) {
        self.end(None);
    }
}

impl PreviewCache {
    /// Claims the read of column `key` of data `generation`, or takes it from the evaluation
    /// that read it (waiting while it reads). A claim is held only while its column is read,
    /// never while waiting for another: no evaluation waits on one that waits on it.
    pub(super) fn claim(&self, generation: u64, key: &(String, String)) -> Claim<'_> {
        let k = (generation, key.0.clone(), key.1.clone());
        let slot = {
            let mut map = self
                .loading
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if map.len() >= MAX_LOADING {
                // reads of other data generations, and those whose column is gone
                map.retain(|key, s| key.0 == generation && s.live());
            }
            match map.get(&k).filter(|s| s.live()) {
                Some(s) => Arc::clone(s),
                None => {
                    let s = Arc::new(Loading::default());
                    map.insert(k.clone(), Arc::clone(&s));
                    return Claim::Mine(LoadGuard {
                        cache: self,
                        key: k,
                        slot: s,
                    });
                }
            }
        };
        let mut d = slot
            .done
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while d.is_none() {
            d = slot.cv.wait(d).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        match d.as_ref().and_then(|r| r.as_ref()).and_then(Weak::upgrade) {
            Some(c) => {
                self.dedups.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Claim::Loaded(c)
            }
            None => Claim::Failed,
        }
    }

    /// Tests: column reads taken from another evaluation instead of read again.
    #[cfg(test)]
    pub(crate) fn dedup_count(&self) -> u64 {
        self.dedups.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl State {
    /// Takes column `key` another evaluation just read: a fork holds it (it is the main state's
    /// or another fork's), the main state keeps it.
    pub(super) fn adopt_column(&mut self, key: (String, String), c: Arc<ColData>) {
        match self.inherited.as_mut() {
            Some(inh) => {
                inh.held.insert(key, c);
            }
            None => {
                self.memory.touch(&key);
                self.columns.insert(key, Ok(c));
                self.enforce_budget();
            }
        }
    }
}
