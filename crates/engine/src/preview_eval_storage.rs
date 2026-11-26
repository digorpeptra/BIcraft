//! The storage engine side of the evaluator (`docs/storage-engine.md`): what the formula engine
//! ([`super`]) asks of the decoded columns, answered by the kernels of `bicraft-scan` over
//! dictionary codes, in parallel, and kept in a data cache.
//!
//! - **Predicates** ([`Engine::pred`]): the rows of a table visible in a filter context, as a
//!   conjunction of allowed-code bitmaps over its own columns (its filters, and the filters of
//!   the tables that reach it through relationships, propagated over codes) plus row masks,
//!   instead of a materialized mask per filter. The other tables' masks are the evaluator's own.
//! - **Aggregations** ([`Engine::aggregate_storage`]): a filter context splits into a base (most
//!   filters) and single-code filters on columns that map to the aggregated table's rows (its own
//!   columns, or a dimension column reached by one path of many-to-one relationships, through a
//!   code lookup table). One grouped scan under the base computes the aggregation for every
//!   combination of those codes; it is cached (key: aggregation, column, base filters by content,
//!   relationship overrides, grouping columns), so the cells of a `SUMMARIZECOLUMNS`, the rows of
//!   an iterator with context transition or of `RANKX` read one value each from one scan.
//!   The split is exact: the filters are a conjunction, and through a many-to-one path a fact
//!   row is visible under `code = x` exactly when its related row's code is `x` (unmatched rows:
//!   the blank row, code 0).
//! - **Groupings** ([`Engine::combos_storage`]), **visible rows** and **distinct values**: the
//!   same predicates, one parallel scan each.
//!
//! Every result is the one the reference path (each filter context scanned on its own) gives;
//! where an aggregation cannot be split exactly (real-number sums over large groupings use
//! compensated sums, see `docs/storage-engine.md`) or a cache would not fit, the reference path
//! runs.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

use bicraft_scan as scan;
use bicraft_vertipaq::CellValue;

use super::super::store::ColData;
use bicraft_dax::ast::Expr;

#[path = "preview_eval_aggs.rs"]
pub(in crate::preview) mod aggs;

#[path = "preview_eval_iterx.rs"]
pub(super) mod iterx;

use super::{
    Agg, ColId, Combos, Dir, Engine, Env, FCtx, FKind, Fail, Filter, MAX_FILTER_HOPS, MeasureDef, NONE,
    Scope, Tab, TabId, V, agg_function, as_num, currency, internal, overflow,
};

/// Cells of the `SUMMARIZECOLUMNS` tables kept between queries.
const MAX_TABLE_CELLS: usize = 40_000_000;
/// Slots of a (group × code) histogram.
const MAX_HISTOGRAM: usize = 1 << 22;
/// Bytes of one cached aggregation.
const MAX_ENTRY_BYTES: usize = 512 << 20;
/// Largest state of a distinct count kept as one bitmap per group; beyond, sorted lists
/// ([`scan::SparseDistinct`]).
const MAX_DENSE_DISTINCT_BYTES: usize = 64 << 20;

#[cfg(test)]
thread_local! {
    /// Tests force the sorted lists (`set_dense_distinct_limit(0)`).
    static DENSE_DISTINCT_LIMIT: std::cell::Cell<usize> = const { std::cell::Cell::new(MAX_DENSE_DISTINCT_BYTES) };
}

#[cfg(test)]
pub(super) fn set_dense_distinct_limit(bytes: usize) {
    DENSE_DISTINCT_LIMIT.with(|c| c.set(bytes));
}

fn dense_distinct_limit() -> usize {
    #[cfg(test)]
    {
        DENSE_DISTINCT_LIMIT.with(std::cell::Cell::get)
    }
    #[cfg(not(test))]
    {
        MAX_DENSE_DISTINCT_BYTES
    }
}
/// Bytes of all cached aggregations of one evaluation before the cache is emptied.
const MAX_CACHE_BYTES: usize = 2 << 30;
/// Most codes of a column whose filters (sets of codes) become grouping columns.
const MAX_SET_CARD: usize = 1 << 13;
/// Most groups one aggregation folds together.
const MAX_FOLD: usize = 1 << 14;

/// A distinct-values key: (column, filters by content, overrides, with the blank row).
type ValuesKey = (ColId, Vec<u64>, RelKey, bool);

/// An iteration's results key: (rows address, aggregation and column, base filters, overrides).
type EachKey = (usize, usize, Vec<u64>, RelKey);

/// A relationship override as a cache key: (relationship, active, direction).
/// The values `RANKX` ranks among, sorted, and the same without repeats (`DENSE`).
pub(super) type Ranked = (Vec<f64>, Vec<f64>);

type RelKey = Vec<(usize, Option<bool>, Option<u8>)>;

fn rel_key(fc: &FCtx) -> RelKey {
    fc.rels
        .iter()
        .map(|o| {
            let d = o.dir.map(|d| match d {
                Dir::None => 0,
                Dir::Single => 1,
                Dir::Both => 2,
            });
            (o.rel, o.active, d)
        })
        .collect()
}

/// The rows of a table visible in a filter context: every predicate and every row mask.
#[derive(Clone)]
pub(super) struct PredSet {
    rows: usize,
    codes: Vec<(Arc<ColData>, Arc<Vec<bool>>)>,
    masks: Vec<Arc<Vec<bool>>>,
    /// Whether the table's virtual blank row is visible.
    pub blank: bool,
}

impl PredSet {
    fn all(rows: usize) -> PredSet {
        PredSet {
            rows,
            codes: Vec::new(),
            masks: Vec::new(),
            blank: true,
        }
    }

    fn selection(&self) -> scan::Selection<'_> {
        let none = self.codes.iter().any(|(_, a)| !a.iter().any(|x| *x));
        scan::Selection {
            rows: self.rows,
            preds: self
                .codes
                .iter()
                .map(|(d, a)| scan::Pred {
                    codes: d.codes.col(),
                    allowed: a,
                })
                .collect(),
            masks: self
                .masks
                .iter()
                .map(|m| m.get(..self.rows).unwrap_or(m))
                .collect(),
            none,
        }
    }
}

/// The rows of a one-column table of values (kept alive), its column, and the codes of its rows.
type TupleCodes = (Arc<Vec<Vec<V>>>, ColId, Arc<Vec<u32>>);

/// A column an iterator's body reads, its data, and the lookup table its codes are grouped by
/// (`None`: its own codes).
type BodyPart = (ColId, Arc<ColData>, Option<Arc<Lut>>);

/// How the rows of a scanned table map to the codes of a grouping column.
pub(super) struct Lut {
    /// The scanned table's column whose codes are mapped (the column itself when it is the
    /// table's own).
    col: ColId,
    /// Its code → the grouping column's code; `None`: the same codes.
    map: Option<Vec<u32>>,
    /// Codes of the grouping column.
    card: usize,
}

/// The state of a cached aggregation, per group.
enum Acc {
    Rows(Vec<u64>),
    Blanks(scan::BlankCounts),
    Distinct(scan::Distinct),
    /// Distinct codes per group as sorted lists, when a bitmap per group would be too large.
    Sparse(scan::SparseDistinct),
    Ranks {
        min: Vec<u32>,
        max: Vec<u32>,
        seen: Vec<bool>,
    },
    /// Exact sums of Currency values (1/10 000 units) and their counts.
    Cur {
        sums: Vec<i128>,
        counts: Vec<u64>,
    },
    /// Exact sums of whole numbers and their counts.
    Int {
        sums: Vec<i128>,
        counts: Vec<u64>,
    },
    /// Compensated sums of real numbers and their counts.
    Float {
        sums: Vec<f64>,
        counts: Vec<u64>,
    },
    Hist(scan::Histogram),
    /// The aggregation over the whole column (one group), from its rows per code.
    Value(V),
}

struct Entry {
    /// Codes of each grouping column, in key order.
    cards: Vec<usize>,
    acc: Acc,
}

impl Entry {
    fn group(&self, codes: &[u32]) -> Option<usize> {
        let mut g = 0usize;
        for (card, c) in self.cards.iter().zip(codes) {
            let c = *c as usize;
            if c >= (*card).max(1) {
                return None;
            }
            g = g.checked_mul((*card).max(1))?.checked_add(c)?;
        }
        Some(g)
    }
}

/// Most bytes `sum_by_id` holds over `groups` groups on `threads` workers: per worker and group a
/// sum (up to 16 bytes), its direct and blank counts and its rows (8 each), then the result (24).
fn sum_states(threads: usize, groups: usize) -> usize {
    groups
        .saturating_add(64)
        .saturating_mul(40)
        .saturating_mul(threads.max(1))
        .saturating_add(groups.saturating_mul(24))
}

/// The codes per group of a histogram of `groups` groups over a column of `card` codes, `None`
/// when they would be over [`MAX_HISTOGRAM`] slots (`scan::Histogram::new`'s rule, without
/// allocating it).
fn hist_card(groups: usize, card: usize) -> Option<usize> {
    groups
        .checked_mul(card.max(1))
        .filter(|s| *s <= MAX_HISTOGRAM)
        .map(|_| card.max(1))
}

/// What the grouped scan of [`Engine::vector_plan`] needs to be computed a range of groups at a
/// time ([`Engine::range_entry`]; `docs/beyond-ram.md`, "Groupings by ranges"), when its states
/// for every group do not fit in what the query may use.
pub(super) struct RangePlan {
    agg: Agg,
    c: ColId,
    d: Arc<ColData>,
    n: usize,
    p: PredSet,
    keys: Vec<(ColId, Arc<Lut>)>,
    /// Positions of the grouping columns the scan is keyed by.
    pub(super) pos: Vec<usize>,
    /// Codes of each of them.
    pub(super) cards: Vec<usize>,
    /// Bytes of one group's state.
    pub(super) group_bytes: usize,
}

impl RangePlan {
    /// Its groups.
    pub(super) fn groups(&self) -> usize {
        self.cards
            .iter()
            .fold(1usize, |a, c| a.saturating_mul((*c).max(1)))
    }

    /// The group of a combination of the grouping's codes (by position), as the whole scan's
    /// entry numbers it; `None`: a code the columns do not hold (no row, every value blank).
    pub(super) fn group(&self, combo: &[u32]) -> Option<usize> {
        let mut g = 0usize;
        for (card, p) in self.cards.iter().zip(&self.pos) {
            let c = *combo.get(*p)? as usize;
            if c >= (*card).max(1) {
                return None;
            }
            g = g.checked_mul((*card).max(1))?.checked_add(c)?;
        }
        Some(g)
    }
}

/// The states of the groups `lo..lo + len` of a [`RangePlan`], with their reservation.
pub(super) struct RangeEntry {
    entry: Entry,
    lo: usize,
    len: usize,
    _charge: super::mem::Charge,
}

/// A grouped scan held to read values from ([`Engine::vector_plan`]).
pub(super) struct VecPlan {
    agg: Agg,
    c: ColId,
    d: Arc<ColData>,
    entry: Arc<Entry>,
    /// Positions of the grouping columns the scan is keyed by.
    pos: Vec<usize>,
}

#[derive(PartialEq, Eq, Hash)]
struct EntryKey {
    agg: Agg,
    col: ColId,
    groups: Vec<ColId>,
    base: Vec<u64>,
    rels: RelKey,
}

/// What the storage engine keeps: for one evaluation, and (aggregations, lookup tables, `ALL`
/// tables: what only depends on the model and its data) from one query to the next, as Analysis
/// Services' storage engine cache does, until the model or the data change or `dax.clear_cache`.
#[derive(Default)]
pub(in crate::preview) struct Store {
    /// The model and data the kept results are for.
    pub(in crate::preview) key: u64,
    /// This evaluation added columns of its own (query `COLUMN`s): its results are not kept.
    pub(super) private: bool,
    /// Fingerprint of this evaluation's measures, in the keys of results kept by text.
    pub(super) measures: u64,
    entries: HashMap<EntryKey, Arc<Entry>>,
    bytes: usize,
    luts: HashMap<(TabId, ColId, RelKey), Option<Arc<Lut>>>,
    reach: HashMap<(TabId, RelKey), Arc<Vec<bool>>>,
    /// `ALL(columns)` tables, by columns.
    all: HashMap<Vec<ColId>, Tab>,
    /// The codes of the rows of the one-column `ALL` tables of `all`.
    all_codes: HashMap<ColId, Arc<Vec<u32>>>,
    /// [`Engine::each_direct`] results: (rows, body, base filters, overrides) → values; the
    /// rows are kept alive so that their address stays theirs.
    each: HashMap<EachKey, (Tab, Arc<Vec<V>>)>,
    /// Codes of the rows of one-column tables of values built from codes (`VALUES(c)`, `ALL(c)`,
    /// a `FILTER` of them), by the address of their rows (kept alive here): what a lookup of each
    /// value by its key would give, for a column whose values all have their own key. For this
    /// evaluation only.
    tuple_codes: HashMap<usize, TupleCodes>,
    /// Tables of `SUMMARIZECOLUMNS` whose items are all aggregations: (text, base filters,
    /// overrides) → table, with their cells counted.
    tables: HashMap<(String, Vec<u64>, RelKey), Tab>,
    table_cells: usize,
    /// Distinct codes of a column among visible rows ([`Engine::values_codes_storage`]).
    values: HashMap<ValuesKey, Arc<Vec<u32>>>,
    /// Results of iterators over a model table ([`Engine::iterate_table`]): (function and
    /// expressions, base filters, overrides) → value.
    iters: HashMap<(String, Vec<u64>, RelKey), V>,
    /// Sorted values `RANKX` ranks among ([`Engine::rank_values`]), keyed as `iters`.
    ranks: HashMap<(String, Vec<u64>, RelKey), Arc<Ranked>>,
    /// Bytes of cached aggregations before the cache is emptied (0: [`MAX_CACHE_BYTES`]); a
    /// memory budget sets a quarter of it (`docs/beyond-ram.md`).
    pub(super) cap: usize,
    /// Content hashes of the bitmaps of filters, by address (the bitmap is kept alive).
    fps: HashMap<usize, (Arc<Vec<bool>>, u64)>,
    /// Its bytes ([`Store::approx_bytes`]), reserved against the process' memory limit (lot 5):
    /// counted as they are kept, within its cap.
    pub(in crate::preview) res: bicraft_mem::Reservation,
}

impl Store {
    /// See [`Engine::note_tuple_codes`].
    pub(super) fn note_tuple_codes(&mut self, rows: &Arc<Vec<Vec<V>>>, c: ColId, codes: Vec<u32>) {
        if self.tuple_codes.len() > 4096 {
            self.tuple_codes.clear();
        }
        self.tuple_codes
            .insert(Arc::as_ptr(rows) as usize, (Arc::clone(rows), c, Arc::new(codes)));
    }

    /// See [`Engine::tuple_codes`].
    pub(super) fn tuple_codes_of(&self, rows: &Arc<Vec<Vec<V>>>, c: ColId) -> Option<Arc<Vec<u32>>> {
        let (_, col, codes) = self.tuple_codes.get(&(Arc::as_ptr(rows) as usize))?;
        (*col == c).then(|| Arc::clone(codes))
    }
}

impl Store {
    fn cap(&self) -> usize {
        if self.cap == 0 {
            MAX_CACHE_BYTES
        } else {
            self.cap.min(MAX_CACHE_BYTES)
        }
    }

    /// Bytes it holds, roughly: the cached aggregations and the kept tables' cells.
    pub(in crate::preview) fn approx_bytes(&self) -> usize {
        let codes: usize = self.all_codes.values().map(|c| c.len()).sum();
        self.bytes
            .saturating_add(self.table_cells.saturating_mul(std::mem::size_of::<V>()))
            .saturating_add(codes.saturating_mul(4))
    }

    /// The store a new evaluation starts from: the kept one when it is for the same model and
    /// data, without what is keyed by the previous evaluation's values (addresses).
    pub(super) fn reuse(
        kept: Option<Store>,
        key: u64,
        pool: &'static bicraft_mem::Pool,
        class: bicraft_mem::Class,
    ) -> Store {
        match kept {
            Some(mut s) if s.key == key && !s.private => {
                s.each.clear();
                s.tuple_codes.clear();
                s.fps.clear();
                s
            }
            _ => Store {
                key,
                res: pool.empty(class),
                ..Store::default()
            },
        }
    }

    /// The reservation follows the bytes kept; what does not fit under the memory limit is dropped
    /// (the caches are computed again when needed).
    pub(super) fn sync_res(&mut self) {
        let b = self.approx_bytes();
        let have = self.res.bytes();
        if b > have && self.res.try_grow(b.saturating_sub(have)).is_err() {
            self.drop_caches();
            return;
        }
        self.res.set(b);
    }

    /// Drops the cached results (what `dax.clear_cache` drops), for memory: they are computed
    /// again when needed. Returns the bytes released from its reservation.
    pub(in crate::preview) fn drop_caches(&mut self) -> usize {
        let before = self.res.bytes();
        self.entries.clear();
        self.bytes = 0;
        self.all.clear();
        self.all_codes.clear();
        self.tables.clear();
        self.table_cells = 0;
        self.values.clear();
        self.iters.clear();
        self.ranks.clear();
        self.res.set(0);
        before
    }
}

impl Engine<'_, '_> {
    fn store_mut(&mut self) -> Result<&mut Store, Fail> {
        match &mut self.store {
            Some(s) => Ok(s),
            None => internal(),
        }
    }

    /// The visible rows of `t` in `fc` as predicates; `None`: all rows (no filter reaches `t`).
    /// The same rows as [`Engine::mask_from`] at the top level.
    pub(super) fn pred(&mut self, fc: &FCtx, t: TabId) -> Result<Option<PredSet>, Fail> {
        if fc.filters.is_empty() {
            return Ok(None);
        }
        let n = self.rows(t)?;
        let mut p = PredSet::all(n);
        let mut any = false;
        let own: Vec<Filter> = fc.filters.iter().filter(|f| f.table == t).cloned().collect();
        for f in &own {
            any = true;
            match &f.kind {
                FKind::Codes(a) => {
                    let Some(c) = f.cols.first().copied() else {
                        return internal();
                    };
                    let d = self.col(c)?;
                    p.blank &= a.first().copied().unwrap_or(false);
                    p.codes.push((d, Arc::clone(a)));
                }
                FKind::Code(x) => {
                    let Some(c) = f.cols.first().copied() else {
                        return internal();
                    };
                    let d = self.col(c)?;
                    let mut a = vec![false; d.values.len()];
                    if let Some(s) = a.get_mut(*x as usize) {
                        *s = true;
                    }
                    p.blank &= *x == 0;
                    p.codes.push((d, Arc::new(a)));
                }
                FKind::Rows(m) => {
                    let m = if m.len() > n {
                        Arc::clone(m)
                    } else {
                        let mut v = m.as_ref().clone();
                        v.resize(n.saturating_add(1), false);
                        Arc::new(v)
                    };
                    p.blank &= m.get(n).copied().unwrap_or(false);
                    p.masks.push(m);
                }
            }
        }
        let mut visited = vec![false; self.tabs.len()];
        if let Some(v) = visited.get_mut(t) {
            *v = true;
        }
        for r in 0..self.rels.len() {
            let (active, dir) = self.rel_state(fc, r);
            if !active || dir == Dir::None {
                continue;
            }
            let Some(rel) = self.rels.get(r) else {
                continue;
            };
            let (other, t_is_from) = if rel.from_t == t {
                (rel.to_t, true)
            } else if rel.to_t == t {
                (rel.from_t, false)
            } else {
                continue;
            };
            if !t_is_from && dir != Dir::Both {
                continue;
            }
            if visited.get(other).copied().unwrap_or(true) {
                continue;
            }
            let Some(sub) = self.mask_from(fc, other, Some(r), &mut visited, 1)? else {
                continue;
            };
            let (tc, t_ok, t_blank) = self.propagate_codes(r, !t_is_from, &sub)?;
            any = true;
            p.blank &= t_blank;
            let d = self.col(tc)?;
            p.codes.push((d, Arc::new(t_ok)));
        }
        Ok(any.then_some(p))
    }

    /// The mask of [`Engine::mask`] (one more slot: the virtual blank row) from predicates.
    pub(super) fn materialize(&mut self, p: &PredSet) -> Result<Vec<bool>, Fail> {
        self.tick(p.rows)?;
        let rows = p.rows;
        let _mask = self.charge_bytes(
            &move || format!("a row mask of {rows} rows"),
            rows.saturating_add(1),
        )?;
        let sel = p.selection();
        let mut m = scan::mask(&sel, scan::threads_for(p.rows, 0));
        m.push(p.blank);
        Ok(m)
    }

    /// [`Engine::visible_rows`] by one parallel scan.
    pub(super) fn visible_rows_storage(&mut self, fc: &FCtx, t: TabId) -> Result<Vec<u32>, Fail> {
        let n = self.rows(t)?;
        self.tick(n)?;
        let p = self.pred(fc, t)?.unwrap_or_else(|| PredSet::all(n));
        let _rows = self.charge_bytes(
            &move || format!("the visible rows of a table of {n} rows"),
            n.saturating_mul(4),
        )?;
        Ok(scan::rows(&p.selection(), scan::threads_for(n, 0)))
    }

    /// [`Engine::values_codes`] by one parallel scan; `None`: a small table, or a bitmap too
    /// large (the reference path runs).
    pub(super) fn values_codes_storage(
        &mut self,
        fc: &FCtx,
        c: ColId,
        blank_row: bool,
    ) -> Result<Option<Vec<u32>>, Fail> {
        let t = self.col_table(c)?;
        let n = self.rows(t)?;
        // (whatever the size: a scan per cell of a grouping by `c` adds up)
        if let Some(v) = self.values_of_one_code(fc, c, t, blank_row)? {
            return Ok(Some(v));
        }
        if n < 1 << 16 {
            return Ok(None);
        }
        // kept between queries: (column, filters by content, overrides, blank row)
        let mut fps = Vec::with_capacity(fc.filters.len());
        for f in &fc.filters {
            fps.push(self.filter_fp(f)?);
        }
        fps.sort_unstable();
        let key = (c, fps, rel_key(fc), blank_row);
        if let Some(v) = self.store_mut()?.values.get(&key) {
            let v = v.as_ref().clone();
            self.tick(v.len())?;
            return Ok(Some(v));
        }
        let out = self.values_codes_scan(fc, c, blank_row, t, n)?;
        if let Some(v) = &out {
            let store = self.store_mut()?;
            if store.values.len() > 4096 {
                store.values.clear();
            }
            store.values.insert(key, Arc::new(v.clone()));
        }
        Ok(out)
    }

    /// [`Engine::values_codes_storage`] when every filter of `fc` is one code of `c` itself (a
    /// cell of a grouping by `c`, a context transition over its values): that code when some row
    /// holds it, without a scan of the table (`HASONEVALUE` or `VALUES` in each of a big
    /// dimension's cells would otherwise scan it once per cell). `None`: another filter.
    fn values_of_one_code(
        &mut self,
        fc: &FCtx,
        c: ColId,
        t: TabId,
        blank_row: bool,
    ) -> Result<Option<Vec<u32>>, Fail> {
        let mut code = None;
        for f in &fc.filters {
            let FKind::Code(x) = f.kind else {
                return Ok(None);
            };
            if f.table != t || f.cols.as_slice() != [c] {
                return Ok(None);
            }
            match code {
                None => code = Some(x),
                // two different codes of one column: nothing is visible
                Some(y) if y != x => return Ok(Some(Vec::new())),
                Some(_) => {}
            }
        }
        let Some(x) = code else {
            return Ok(None);
        };
        let d = self.col(c)?;
        // the codes some row holds: every code of a value, and code 0 when a row is blank
        let held = if x == 0 {
            match d.codes.known_blank() {
                Some(b) => b,
                None => return Ok(None),
            }
        } else {
            d.values
                .get(x as usize)
                .is_some_and(|v| !matches!(v, CellValue::Null))
        };
        self.tick(1)?;
        let mut out = if held { vec![x] } else { Vec::new() };
        // the virtual blank row is visible only under the blank code
        if blank_row && x == 0 && out.is_empty() && self.has_blank_row(t)? {
            out.push(0);
        }
        Ok(Some(out))
    }

    fn values_codes_scan(
        &mut self,
        fc: &FCtx,
        c: ColId,
        blank_row: bool,
        t: TabId,
        n: usize,
    ) -> Result<Option<Vec<u32>>, Fail> {
        let d = self.col(c)?;
        let card = d.values.len();
        // no filter reaches the table: the codes some row holds, from the dictionary (every code
        // of a value is held by a row of a compressed column, code 0 when a row is blank)
        let pred = self.pred(fc, t)?;
        if pred.is_none()
            && let Some(blank) = d.codes.known_blank()
        {
            self.tick(card)?;
            let mut out: Vec<u32> = Vec::with_capacity(card);
            if blank {
                out.push(0);
            }
            for (k, v) in d.values.iter().enumerate().skip(1) {
                if !matches!(v, CellValue::Null)
                    && let Ok(code) = u32::try_from(k)
                {
                    out.push(code);
                }
            }
            if blank_row && out.first() != Some(&0) && self.has_blank_row(t)? {
                out.insert(0, 0);
            }
            return Ok(Some(out));
        }
        let Some(empty) = scan::Distinct::new(1, card, MAX_ENTRY_BYTES) else {
            return Ok(None);
        };
        self.tick(n)?;
        let p = pred.unwrap_or_else(|| PredSet::all(n));
        let sel = p.selection();
        let (threads, _sinks) = self.scan_threads(n, card / 8, &move || {
            format!("a scan of the {card} values of a column")
        })?;
        let Some(seen) = scan::scan(&sel, &scan::Key::single(), Some(d.codes.col()), threads, || {
            empty.clone()
        }) else {
            return internal();
        };
        let mut out: Vec<u32> = Vec::new();
        for (w, bits) in seen.bits.iter().enumerate() {
            let mut b = *bits;
            while b != 0 {
                let i = b.trailing_zeros() as usize;
                if let Ok(code) = u32::try_from(w.saturating_mul(64).saturating_add(i)) {
                    out.push(code);
                }
                b &= b - 1;
            }
        }
        if blank_row && p.blank && out.first() != Some(&0) && self.has_blank_row(t)? {
            out.insert(0, 0);
        }
        Ok(Some(out))
    }

    /// The existing combinations of `cols` (columns of `t`) visible in `fc`, in the order of
    /// their first row, then the blank row's: [`Engine::combos_table`] by one parallel scan.
    /// `None`: too many combinations for a dense grouping.
    pub(super) fn combos_storage(
        &mut self,
        fc: &FCtx,
        t: TabId,
        cols: &[ColId],
    ) -> Result<Option<Combos>, Fail> {
        let n = self.rows(t)?;
        let datas = cols.iter().map(|c| self.col(*c)).collect::<Result<Vec<_>, _>>()?;
        let p = self.pred(fc, t)?;
        // one column, every row: its codes in order are its values in the order of their first
        // row (codes are numbered so), when no value merged into the blank code 0 hides where the
        // blank row's comes. A compressed column without a blank row holds every code 1.. (they
        // are numbered from the distinct values its rows meet): no count needed.
        if let (None, [d]) = (&p, datas.as_slice())
            && d.codes.known_blank() == Some(false)
        {
            let card = u32::try_from(d.values.len()).unwrap_or(u32::MAX);
            let c = self.charge_bytes(
                &move || format!("the {card} values of a grouping column"),
                Combos::bytes_for(1, card as usize),
            )?;
            let mut tuples = Combos::with_capacity(1, card as usize).charged(c);
            for c in 1..card {
                tuples.push_one(c);
            }
            if self.has_blank_row(t)? {
                tuples.push_one(0);
            }
            return Ok(Some(tuples));
        }
        if let (None, [d]) = (&p, datas.as_slice())
            && let Some(counts) = self.full_counts(d)?
            && counts.first().copied() == Some(0)
        {
            let n = counts.len();
            let c = self.charge_bytes(
                &move || format!("the {n} values of a grouping column"),
                Combos::bytes_for(1, n),
            )?;
            let mut tuples = Combos::new(1).charged(c);
            for (c, k) in counts.iter().enumerate() {
                if *k > 0
                    && let Ok(c) = u32::try_from(c)
                {
                    tuples.push_one(c);
                }
            }
            if self.has_blank_row(t)? {
                tuples.push_one(0);
            }
            return Ok(Some(tuples));
        }
        let parts: Vec<scan::KeyPart<'_>> = datas
            .iter()
            .map(|d| scan::KeyPart {
                codes: d.codes.col(),
                lut: None,
                card: d.values.len(),
            })
            .collect();
        let Some(key) = scan::Key::new(parts) else {
            return Ok(None);
        };
        let groups = key.groups();
        if groups.saturating_mul(4) > MAX_ENTRY_BYTES {
            return Ok(None);
        }
        let (states, routed) = self.grouped_scan_charge("a grouping's combinations", n, groups, 4)?;
        self.tick(n)?;
        let p = p.unwrap_or_else(|| PredSet::all(n));
        let sel = p.selection();
        let Some(first) = scan::scan_groups_within(&sel, &key, None, 4, routed, scan::FirstRow::new) else {
            return internal();
        };
        drop(states);
        let first_charge = self.charge_bytes(
            &move || format!("the scan of a grouping's {groups} combinations"),
            groups.saturating_mul(4),
        )?;
        let zero_seen = key
            .id(&vec![0; cols.len()])
            .is_some_and(|z| first.0.get(z).is_some_and(|f| *f != u32::MAX));
        // (first row, group) of the groups met, then their codes in that order: 8 bytes per
        // group, then 4 per code, the scan's state dropped in between
        let np = first.0.iter().filter(|f| **f != u32::MAX).count();
        let present_charge = self.charge_bytes(
            &move || format!("the first rows of a grouping's {np} combinations"),
            np.saturating_mul(8),
        )?;
        let mut present: Vec<(u32, u32)> = Vec::with_capacity(np);
        for (g, f) in first.0.iter().enumerate() {
            if *f != u32::MAX
                && let Ok(g) = u32::try_from(g)
            {
                present.push((*f, g));
            }
        }
        drop(first);
        drop(first_charge);
        present.sort_unstable();
        let w = cols.len();
        let c = self.charge_bytes(
            &move || format!("the {np} combinations of {w} grouping columns"),
            Combos::bytes_for(w, np),
        )?;
        let mut tuples = Combos::with_capacity(cols.len(), present.len()).charged(c);
        for (_, g) in &present {
            tuples.push(&key.codes(*g as usize));
        }
        drop(present);
        drop(present_charge);
        if p.blank && !zero_seen && self.has_blank_row(t)? {
            tuples.push(&[]);
        }
        Ok(Some(tuples))
    }

    /// The tables whose filters reach `t` in the relationship state of `fc` (the traversal of
    /// [`Engine::mask_from`]).
    fn reach(&mut self, fc: &FCtx, t: TabId) -> Result<Arc<Vec<bool>>, Fail> {
        let key = (t, rel_key(fc));
        if let Some(r) = self.store_mut()?.reach.get(&key) {
            return Ok(Arc::clone(r));
        }
        let mut seen = vec![false; self.tabs.len()];
        let mut visited = vec![false; self.tabs.len()];
        let mut paths = Vec::new();
        self.walk(
            fc,
            t,
            None,
            &mut visited,
            0,
            None,
            &mut Vec::new(),
            &mut paths,
            &mut seen,
        );
        let r = Arc::new(seen);
        self.store_mut()?.reach.insert(key, Arc::clone(&r));
        Ok(r)
    }

    /// The traversal of [`Engine::mask_from`] from `x`: marks the tables it visits in `seen` and
    /// records in `paths` each way it reaches `target` (relationship, and whether the hop goes
    /// from a many side to a one side), stopping after two.
    #[allow(clippy::too_many_arguments)]
    fn walk(
        &self,
        fc: &FCtx,
        x: TabId,
        via: Option<usize>,
        visited: &mut [bool],
        depth: usize,
        target: Option<TabId>,
        cur: &mut Vec<(usize, bool)>,
        paths: &mut Vec<Vec<(usize, bool)>>,
        seen: &mut [bool],
    ) {
        if let Some(s) = seen.get_mut(x) {
            *s = true;
        }
        if depth > 0 && target == Some(x) {
            paths.push(cur.clone());
        }
        if paths.len() > 1 || depth >= MAX_FILTER_HOPS {
            return;
        }
        if let Some(v) = visited.get_mut(x) {
            *v = true;
        }
        for r in 0..self.rels.len() {
            if via == Some(r) {
                continue;
            }
            let (active, dir) = self.rel_state(fc, r);
            if !active || dir == Dir::None {
                continue;
            }
            let Some(rel) = self.rels.get(r) else {
                continue;
            };
            let (other, x_is_from) = if rel.from_t == x {
                (rel.to_t, true)
            } else if rel.to_t == x {
                (rel.from_t, false)
            } else {
                continue;
            };
            if !x_is_from && dir != Dir::Both {
                continue;
            }
            if visited.get(other).copied().unwrap_or(true) {
                continue;
            }
            cur.push((r, x_is_from && rel.to_one));
            self.walk(
                fc,
                other,
                Some(r),
                visited,
                depth.saturating_add(1),
                target,
                cur,
                paths,
                seen,
            );
            cur.pop();
        }
        if let Some(v) = visited.get_mut(x) {
            *v = false;
        }
    }

    /// How the rows of `t` map to the codes of column `g` in the relationship state of `fc`;
    /// `None` when a filter on `g` does not reach `t` through exactly one many-to-one path.
    /// The columns an iterator's body reads ([`Engine::row_deps`]) as the parts of its grouping
    /// key: each column's own codes, except a foreign key read only through `RELATED`, which is
    /// keyed by the codes of the related columns it reads (through the many-to-one lookup table
    /// of [`Engine::group_lut`]). `SUMX(Ventes, IF(RELATED(Client[Fidelite]) = "Oui", …))` then
    /// has as many combinations as loyalty values, not customers. The body evaluated in a row
    /// of each combination reads the same values, since `RELATED` reads them on the row its key
    /// finds and the lookup table maps every key to that row's codes. Only without relationship
    /// overrides (`RELATED` follows the model's active relationships).
    fn body_parts(
        &mut self,
        fc: &FCtx,
        t: TabId,
        body: &Expr,
        env: &Env,
        deps: &[ColId],
    ) -> Result<Vec<BodyPart>, Fail> {
        let mut direct = Vec::new();
        let mut related = Vec::new();
        if fc.rels.is_empty() {
            self.body_reads(body, t, env, &mut direct, &mut related, 0)?;
        }
        let mut out = Vec::with_capacity(deps.len());
        for c in deps {
            let targets: Vec<ColId> = related
                .iter()
                .filter(|(fk, _)| fk == c)
                .map(|(_, g)| *g)
                .collect();
            let mut luts = Vec::new();
            if !fc.rels.is_empty() || direct.contains(c) || targets.is_empty() {
                luts.clear();
            } else {
                for g in targets {
                    match self.group_lut(fc, t, g)? {
                        Some(l) if l.col == *c && l.map.is_some() => luts.push(l),
                        _ => {
                            luts.clear();
                            break;
                        }
                    }
                }
            }
            let d = self.col(*c)?;
            if luts.is_empty() {
                out.push((*c, d, None));
            } else {
                for l in luts {
                    out.push((*c, Arc::clone(&d), Some(l)));
                }
            }
        }
        Ok(out)
    }

    /// The columns of `t` an expression reads directly, and the (foreign key, column) of each
    /// `RELATED` it reads (for [`Engine::body_parts`]).
    fn body_reads(
        &mut self,
        e: &Expr,
        t: TabId,
        env: &Env,
        direct: &mut Vec<ColId>,
        related: &mut Vec<(ColId, ColId)>,
        depth: usize,
    ) -> Result<(), Fail> {
        if depth > 64 {
            return Ok(());
        }
        let d = depth.saturating_add(1);
        match e {
            Expr::Call { name, args, .. } if name.eq_ignore_ascii_case("RELATED") => {
                let mut fk = Vec::new();
                if self.row_deps(e, t, env, &mut fk, 0)?
                    && let (
                        [fk],
                        [
                            Some(Expr::Column {
                                table: Some(tn),
                                name,
                                ..
                            }),
                        ],
                    ) = (fk.as_slice(), args.as_slice())
                    && let Some(g) = self.col_id(tn, name)
                {
                    related.push((*fk, g));
                }
            }
            Expr::Column { .. } => {
                let mut cols = Vec::new();
                if self.row_deps(e, t, env, &mut cols, 0)? {
                    direct.extend(cols);
                }
            }
            Expr::Unary { operand, .. } => self.body_reads(operand, t, env, direct, related, d)?,
            Expr::Binary { lhs, rhs, .. } => {
                self.body_reads(lhs, t, env, direct, related, d)?;
                self.body_reads(rhs, t, env, direct, related, d)?;
            }
            Expr::Call { args, .. } => {
                for a in args.iter().flatten() {
                    self.body_reads(a, t, env, direct, related, d)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn group_lut(&mut self, fc: &FCtx, t: TabId, g: ColId) -> Result<Option<Arc<Lut>>, Fail> {
        let key = (t, g, rel_key(fc));
        if let Some(l) = self.store_mut()?.luts.get(&key) {
            return Ok(l.clone());
        }
        let l = self.build_lut(fc, t, g)?.map(Arc::new);
        self.store_mut()?.luts.insert(key, l.clone());
        Ok(l)
    }

    fn build_lut(&mut self, fc: &FCtx, t: TabId, g: ColId) -> Result<Option<Lut>, Fail> {
        let gt = self.col_table(g)?;
        let gd = self.col(g)?;
        let card = gd.values.len();
        if gt == t {
            return Ok(Some(Lut {
                col: g,
                map: None,
                card,
            }));
        }
        let mut seen = vec![false; self.tabs.len()];
        let mut visited = vec![false; self.tabs.len()];
        let mut paths = Vec::new();
        self.walk(
            fc,
            t,
            None,
            &mut visited,
            0,
            Some(gt),
            &mut Vec::new(),
            &mut paths,
            &mut seen,
        );
        let [path] = paths.as_slice() else {
            return Ok(None);
        };
        if path.is_empty() || !path.iter().all(|(_, n1)| *n1) {
            return Ok(None);
        }
        // From the grouping column's table back to `t`: the group code of each row of the table
        // at the many end of each hop (0: no related row, the blank row).
        let mut per_row: Vec<u32> = gd.codes.plain().to_vec();
        for (i, (r, _)) in path.iter().enumerate().rev() {
            let Some(rel) = self.rels.get(*r) else {
                return internal();
            };
            let (from, to) = (rel.from, rel.to);
            let key = self.col(to)?;
            let mut row_of = vec![NONE; key.values.len()];
            for (row, k) in key.codes.iter().enumerate() {
                if *k == 0 {
                    continue;
                }
                let Some(slot) = row_of.get_mut(*k as usize) else {
                    continue;
                };
                if *slot != NONE {
                    // duplicate keys on the one side: not a function of the key
                    return Ok(None);
                }
                *slot = u32::try_from(row).unwrap_or(NONE);
            }
            let map = self.rel_map(*r, true)?;
            let by_code: Vec<u32> = map
                .iter()
                .map(|k| {
                    if *k == NONE {
                        return 0;
                    }
                    row_of
                        .get(*k as usize)
                        .filter(|row| **row != NONE)
                        .and_then(|row| per_row.get(*row as usize))
                        .copied()
                        .unwrap_or(0)
                })
                .collect();
            if i == 0 {
                return Ok(Some(Lut {
                    col: from,
                    map: Some(by_code),
                    card,
                }));
            }
            let fd = self.col(from)?;
            per_row = fd
                .codes
                .iter()
                .map(|c| by_code.get(*c as usize).copied().unwrap_or(0))
                .collect();
        }
        Ok(None)
    }

    /// Content hash of a filter (bitmaps hashed once per allocation).
    fn filter_fp(&mut self, f: &Filter) -> Result<u64, Fail> {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        f.table.hash(&mut h);
        f.cols.hash(&mut h);
        let bits = match &f.kind {
            FKind::Code(x) => {
                0u8.hash(&mut h);
                x.hash(&mut h);
                None
            }
            FKind::Codes(a) => {
                1u8.hash(&mut h);
                Some(a)
            }
            FKind::Rows(m) => {
                2u8.hash(&mut h);
                Some(m)
            }
        };
        if let Some(b) = bits {
            let addr = Arc::as_ptr(b) as usize;
            let store = self.store_mut()?;
            let fp = match store.fps.get(&addr) {
                Some((_, fp)) => *fp,
                None => {
                    let mut bh = std::collections::hash_map::DefaultHasher::new();
                    b.hash(&mut bh);
                    let fp = bh.finish();
                    store.fps.insert(addr, (Arc::clone(b), fp));
                    fp
                }
            };
            fp.hash(&mut h);
        }
        Ok(h.finish())
    }

    /// Currency values per code of a Currency column (as `currency_aggregate` reads them);
    /// `None` when a value does not convert.
    fn cur_codes(&mut self, d: &ColData) -> Option<Arc<Vec<Option<i64>>>> {
        d.derived
            .get()
            .currency
            .get_or_init(|| {
                let mut out = Vec::with_capacity(d.values.len());
                for v in &d.values {
                    out.push(match v {
                        CellValue::Decimal(x) => Some(*x),
                        CellValue::Null => None,
                        other => Some(as_num(other).and_then(currency::from_f64)?),
                    });
                }
                Some(Arc::new(out))
            })
            .clone()
    }

    /// The rows per code of the whole column, computed once per decoded column (one parallel
    /// scan); `None` when the column has too many codes for it.
    pub(super) fn full_counts(&mut self, d: &ColData) -> Result<Option<Arc<Vec<u64>>>, Fail> {
        let dv = d.derived.get();
        if let Some(v) = dv.counts.get() {
            return Ok(v.clone());
        }
        let card = d.values.len();
        self.keep_charge(&|| "a column's rows per value".into(), card.saturating_mul(8))?;
        let n = d.codes.len();
        self.tick(n)?;
        // a compressed column counts its RLE runs one step per run
        if let Some(p) = d.codes.as_packed()
            && card.saturating_mul(8) <= MAX_ENTRY_BYTES
        {
            let v = Some(Arc::new(p.counts_parallel(card, scan::configured_threads())));
            return Ok(dv.counts.get_or_init(|| v).clone());
        }
        let v = match scan::Histogram::new(1, card, MAX_ENTRY_BYTES / 8) {
            Some(h) => {
                let (threads, _sinks) = self.scan_threads(n, card.saturating_mul(8), &move || {
                    format!("a histogram of the {card} values of a column")
                })?;
                let r = scan::scan(
                    &scan::Selection::all(n),
                    &scan::Key::single(),
                    Some(d.codes.col()),
                    threads,
                    || h.clone(),
                );
                match r {
                    Some(h) => Some(Arc::new(h.counts)),
                    None => return internal(),
                }
            }
            None => None,
        };
        Ok(dv.counts.get_or_init(|| v).clone())
    }

    /// Whether some row of column `d` has the blank code 0, from its rows per code.
    pub(super) fn has_blank_code(&mut self, d: &ColData) -> Result<bool, Fail> {
        if let Some(b) = d.codes.known_blank() {
            return Ok(b);
        }
        Ok(match self.full_counts(d)? {
            Some(c) => c.first().copied().unwrap_or(0) > 0,
            None => d.codes.contains(&0),
        })
    }

    /// Whole-number values per code of `c` when every number of the column is whole and a sum
    /// over all its rows stays exact in a double; `None` otherwise.
    fn int_codes(&mut self, c: ColId, d: &ColData) -> Result<Option<Arc<Vec<Option<i64>>>>, Fail> {
        let dv = d.derived.get();
        if let Some(v) = dv.ints.get() {
            return Ok(v.clone());
        }
        let nums = self.nums(c)?;
        self.keep_charge(
            &|| "a column's whole numbers".into(),
            nums.len().saturating_mul(16),
        )?;
        let limit = 9_007_199_254_740_992.0 / (d.codes.len() as f64 + 1.0);
        let v = dv
            .ints
            .get_or_init(|| {
                let mut out = Vec::with_capacity(nums.len());
                for x in nums.iter() {
                    match x {
                        None => out.push(None),
                        Some(x) if x.fract() == 0.0 && x.abs() <= limit => out.push(Some(*x as i64)),
                        Some(_) => return None,
                    }
                }
                Some(Arc::new(out))
            })
            .clone();
        Ok(v)
    }

    /// [`Engine::aggregate`] through the storage engine; `None`: the reference path runs.
    pub(super) fn aggregate_storage(&mut self, agg: Agg, c: ColId, fc: &FCtx) -> Result<Option<V>, Fail> {
        let t = self.col_table(c)?;
        let n = self.rows(t)?;
        let d = self.col(c)?;
        let reach = self.reach(fc, t)?;
        // the filters that become grouping columns (one code, or a set of codes of a column with
        // few values: time intelligence, slicers), each with its filter; the rest is the base
        let mut keys: Vec<(ColId, Vec<u32>, Arc<Lut>, Filter)> = Vec::new();
        let mut base: Vec<Filter> = Vec::new();
        for f in &fc.filters {
            if !reach.get(f.table).copied().unwrap_or(true) {
                // a filter that does not reach the table changes nothing
                continue;
            }
            if let [g] = f.cols.as_slice()
                && !keys.iter().any(|(k, _, _, _)| k == g)
            {
                let set: Option<Vec<u32>> = match &f.kind {
                    FKind::Code(x) => Some(vec![*x]),
                    FKind::Codes(a) if a.len() <= MAX_SET_CARD => Some(
                        a.iter()
                            .enumerate()
                            .filter(|(_, ok)| **ok)
                            .filter_map(|(i, _)| u32::try_from(i).ok())
                            .collect(),
                    ),
                    _ => None,
                };
                if let Some(set) = set
                    && let Some(l) = self.group_lut(fc, t, *g)?
                {
                    keys.push((*g, set, l, f.clone()));
                    continue;
                }
            }
            base.push(f.clone());
        }
        // a grouping too large for one scan, or too many groups to fold: its largest columns
        // go back to the base
        let per_group = Self::group_bytes(agg, d.values.len());
        loop {
            let groups = keys
                .iter()
                .try_fold(1usize, |a, (_, _, l, _)| a.checked_mul(l.card.max(1)));
            let folds = keys
                .iter()
                .try_fold(1usize, |a, (_, set, _, _)| a.checked_mul(set.len().max(1)));
            match (groups, folds) {
                (Some(g), Some(f))
                    if g <= scan::MAX_GROUPS
                        && g.saturating_mul(per_group) <= MAX_ENTRY_BYTES
                        && f <= MAX_FOLD =>
                {
                    break;
                }
                _ => {}
            }
            let Some(i) = keys
                .iter()
                .enumerate()
                .max_by_key(|(_, (_, set, l, _))| (set.len() > 1, l.card))
                .map(|(i, _)| i)
            else {
                return Ok(None);
            };
            let (_, _, _, f) = keys.remove(i);
            base.push(f);
        }
        let sets: Vec<Vec<u32>> = keys.iter().map(|(_, s, _, _)| s.clone()).collect();
        let luts: Vec<(ColId, Arc<Lut>)> = keys.into_iter().map(|(g, _, l, _)| (g, l)).collect();
        let Some(entry) = self.entry(agg, c, &d, t, n, fc, base, &luts)? else {
            return Ok(None);
        };
        // every combination of the allowed codes
        let mut groups: Vec<usize> = Vec::new();
        let mut combo = vec![0u32; sets.len()];
        let mut at = vec![0usize; sets.len()];
        if sets.iter().all(|s| !s.is_empty()) {
            'combos: loop {
                for (i, s) in sets.iter().enumerate() {
                    if let (Some(slot), Some(code)) = (combo.get_mut(i), at.get(i).and_then(|k| s.get(*k))) {
                        *slot = *code;
                    }
                }
                if let Some(g) = entry.group(&combo) {
                    groups.push(g);
                }
                // next combination (the last key varies fastest)
                let mut i = sets.len();
                loop {
                    if i == 0 {
                        break 'combos;
                    }
                    i -= 1;
                    let (Some(k), Some(len)) = (at.get_mut(i), sets.get(i).map(Vec::len)) else {
                        break 'combos;
                    };
                    *k += 1;
                    if *k < len {
                        break;
                    }
                    *k = 0;
                }
            }
        }
        self.tick(groups.len().max(1))?;
        self.fold(agg, c, &d, &entry, &groups).map(Some)
    }

    /// The aggregation over several groups of a cached scan (their rows together).
    fn fold(&mut self, agg: Agg, c: ColId, d: &ColData, e: &Entry, groups: &[usize]) -> Result<V, Fail> {
        match groups {
            [] => return Ok(V::Blank),
            [g] => return self.value_of(agg, c, d, e, *g),
            _ => {}
        }
        let blank_or = |n: u64| if n == 0 { V::Blank } else { V::Num(n as f64) };
        let sum = |v: &[u64]| {
            groups
                .iter()
                .map(|g| v.get(*g).copied().unwrap_or(0))
                .fold(0u64, u64::saturating_add)
        };
        let sum_i = |v: &[i128]| {
            groups
                .iter()
                .map(|g| v.get(*g).copied().unwrap_or(0))
                .fold(0i128, i128::saturating_add)
        };
        Ok(match &e.acc {
            Acc::Rows(v) => blank_or(sum(v)),
            Acc::Blanks(b) => match agg {
                Agg::CountBlank => blank_or(sum(&b.blank)),
                _ => blank_or(sum(&b.nonblank)),
            },
            Acc::Sparse(s) => {
                let (n, blank) = s.count_union(groups);
                match agg {
                    Agg::DistinctNoBlank => blank_or(n.saturating_sub(u64::from(blank))),
                    _ => blank_or(n),
                }
            }
            Acc::Distinct(s) => {
                let (n, blank) = s.count_union(groups);
                match agg {
                    Agg::DistinctNoBlank => blank_or(n.saturating_sub(u64::from(blank))),
                    _ => blank_or(n),
                }
            }
            Acc::Ranks { min, max, seen } => {
                let live: Vec<usize> = groups
                    .iter()
                    .copied()
                    .filter(|g| seen.get(*g).copied().unwrap_or(false))
                    .collect();
                let r = if agg == Agg::Min {
                    live.iter().filter_map(|g| min.get(*g)).min()
                } else {
                    live.iter().filter_map(|g| max.get(*g)).max()
                };
                let Some(r) = r else {
                    return Ok(V::Blank);
                };
                let ranks = self.rank_table(d);
                let code = ranks.1.get(*r as usize).copied().unwrap_or(0);
                d.values.get(code as usize).map_or(V::Blank, V::of_cell)
            }
            Acc::Cur { sums, counts } => {
                let count = sum(counts);
                if count == 0 {
                    return Ok(V::Blank);
                }
                let total = sum_i(sums);
                let r = match agg {
                    Agg::Avg => currency::div_round(total, i128::from(count)),
                    _ => i64::try_from(total).ok(),
                };
                return r.map_or_else(overflow, |x| Ok(V::Cur(x)));
            }
            Acc::Int { sums, counts } => {
                let count = sum(counts);
                if count == 0 {
                    return Ok(V::Blank);
                }
                let total = sum_i(sums) as f64;
                match agg {
                    Agg::Avg => V::Num(total / count as f64),
                    _ => V::Num(total),
                }
            }
            Acc::Float { sums, counts } => {
                let count = sum(counts);
                if count == 0 {
                    return Ok(V::Blank);
                }
                let mut total = 0.0;
                let mut comp = 0.0;
                for g in groups {
                    scan::neumaier(&mut total, &mut comp, sums.get(*g).copied().unwrap_or(0.0));
                }
                let total = total + comp;
                match agg {
                    Agg::Avg => V::Num(total / count as f64),
                    _ => V::Num(total),
                }
            }
            Acc::Value(v) => v.clone(),
            Acc::Hist(h) => {
                let mut counts = vec![0u64; h.card];
                for g in groups {
                    for (a, b) in counts.iter_mut().zip(h.group(*g)) {
                        *a = a.saturating_add(*b);
                    }
                }
                return self.finish(agg, c, d, &counts);
            }
        })
    }

    /// The cached scan of `agg(c)` under `base` (filters of `fc`'s relationship state) grouped by
    /// `keys`, computed on first use; `None`: the reference path runs.
    #[allow(clippy::too_many_arguments)]
    fn entry(
        &mut self,
        agg: Agg,
        c: ColId,
        d: &ColData,
        t: TabId,
        n: usize,
        fc: &FCtx,
        base: Vec<Filter>,
        keys: &[(ColId, Arc<Lut>)],
    ) -> Result<Option<Arc<Entry>>, Fail> {
        let mut fps = Vec::with_capacity(base.len());
        for f in &base {
            fps.push(self.filter_fp(f)?);
        }
        fps.sort_unstable();
        let key = EntryKey {
            agg,
            col: c,
            groups: keys.iter().map(|(g, _)| *g).collect(),
            base: fps,
            rels: rel_key(fc),
        };
        if let Some(e) = self.store_mut()?.entries.get(&key) {
            return Ok(Some(Arc::clone(e)));
        }
        // a cached scan of the same aggregation under the same filters grouped by more columns:
        // its groups folded together (subtotals, totals beside a grouping)
        if let Some(e) = self.fold_from_finer(&key) {
            let e = Arc::new(e);
            let bytes = entry_bytes(&e);
            self.keep_charge(&|| "a grouped scan's result".into(), bytes)?;
            let store = self.store_mut()?;
            store.bytes = store.bytes.saturating_add(bytes);
            store.entries.insert(key, Arc::clone(&e));
            return Ok(Some(e));
        }
        let base_fc = FCtx {
            filters: base,
            rels: fc.rels.clone(),
            applied: fc.applied,
            memo: Default::default(),
        };
        let Some(e) = self.compute(agg, c, d, t, n, &base_fc, keys)? else {
            return Ok(None);
        };
        let e = Arc::new(e);
        let bytes = entry_bytes(&e);
        // counted with the query until it ends (then with what the session keeps)
        self.keep_charge(&|| "a grouped scan's result".into(), bytes)?;
        let store = self.store_mut()?;
        if store.bytes.saturating_add(bytes) > store.cap() {
            store.entries.clear();
            store.bytes = 0;
        }
        store.bytes = store.bytes.saturating_add(bytes);
        store.entries.insert(key, Arc::clone(&e));
        Ok(Some(e))
    }

    /// The entry of `key` from a cached entry of the same aggregation, column, base and overrides
    /// grouped by a superset of its columns, its groups folded (exact aggregations only: not the
    /// compensated sums of doubles, whose result would depend on the order).
    fn fold_from_finer(&mut self, key: &EntryKey) -> Option<Entry> {
        let store = self.store.as_ref()?;
        let (fine_key, fine) = store
            .entries
            .iter()
            .filter(|(k, _)| {
                k.agg == key.agg
                    && k.col == key.col
                    && k.base == key.base
                    && k.rels == key.rels
                    && k.groups.len() > key.groups.len()
                    && key.groups.iter().all(|g| k.groups.contains(g))
            })
            .min_by_key(|(_, e)| e.cards.iter().product::<usize>())?;
        let pos: Vec<usize> = key
            .groups
            .iter()
            .filter_map(|g| fine_key.groups.iter().position(|x| x == g))
            .collect();
        let cards: Vec<usize> = pos.iter().filter_map(|p| fine.cards.get(*p).copied()).collect();
        let fine_n: usize = fine.cards.iter().map(|c| (*c).max(1)).product();
        let coarse_n: usize = cards.iter().map(|c| (*c).max(1)).product();
        // the coarse group of each fine group
        let mut map = Vec::with_capacity(fine_n);
        let mut codes = vec![0usize; fine.cards.len()];
        for _ in 0..fine_n {
            let mut g = 0usize;
            for p in &pos {
                let c = codes.get(*p).copied().unwrap_or(0);
                let card = fine.cards.get(*p).copied().unwrap_or(1).max(1);
                g = g * card + c;
            }
            map.push(g);
            // next fine combination (the last column varies fastest)
            for i in (0..codes.len()).rev() {
                let card = fine.cards.get(i).copied().unwrap_or(1).max(1);
                if let Some(c) = codes.get_mut(i) {
                    *c += 1;
                    if *c < card {
                        break;
                    }
                    *c = 0;
                }
            }
        }
        let sum_u = |v: &[u64]| {
            let mut out = vec![0u64; coarse_n];
            for (f, x) in v.iter().enumerate() {
                if let Some(o) = map.get(f).and_then(|g| out.get_mut(*g)) {
                    *o = o.wrapping_add(*x);
                }
            }
            out
        };
        let sum_i = |v: &[i128]| {
            let mut out = vec![0i128; coarse_n];
            for (f, x) in v.iter().enumerate() {
                if let Some(o) = map.get(f).and_then(|g| out.get_mut(*g)) {
                    *o = o.saturating_add(*x);
                }
            }
            out
        };
        let acc = match &fine.acc {
            Acc::Rows(v) => Acc::Rows(sum_u(v)),
            Acc::Blanks(b) => Acc::Blanks(scan::BlankCounts {
                nonblank: sum_u(&b.nonblank),
                blank: sum_u(&b.blank),
            }),
            Acc::Cur { sums, counts } => Acc::Cur {
                sums: sum_i(sums),
                counts: sum_u(counts),
            },
            Acc::Int { sums, counts } => Acc::Int {
                sums: sum_i(sums),
                counts: sum_u(counts),
            },
            Acc::Ranks { min, max, seen } => {
                let mut lo = vec![u32::MAX; coarse_n];
                let mut hi = vec![0u32; coarse_n];
                let mut sn = vec![false; coarse_n];
                for f in 0..fine_n {
                    let (Some(g), Some(true)) = (map.get(f).copied(), seen.get(f).copied()) else {
                        continue;
                    };
                    if let (Some(a), Some(b), Some(c), Some(x), Some(y)) = (
                        lo.get_mut(g),
                        hi.get_mut(g),
                        sn.get_mut(g),
                        min.get(f),
                        max.get(f),
                    ) {
                        *a = (*a).min(*x);
                        *b = (*b).max(*y);
                        *c = true;
                    }
                }
                Acc::Ranks {
                    min: lo,
                    max: hi,
                    seen: sn,
                }
            }
            Acc::Distinct(d) => {
                let mut out = scan::Distinct::new(coarse_n, d.card, MAX_ENTRY_BYTES)?;
                let w = d.words();
                for f in 0..fine_n {
                    let Some(g) = map.get(f).copied() else { continue };
                    let src = d.bits.get(f * w..f * w + w)?;
                    let dst = out.bits.get_mut(g * w..g * w + w)?;
                    for (a, b) in dst.iter_mut().zip(src) {
                        *a |= *b;
                    }
                }
                Acc::Distinct(out)
            }
            Acc::Sparse(s) => Acc::Sparse(s.coarsen(&map, coarse_n)?),
            Acc::Hist(h) => {
                let mut out = scan::Histogram::new(coarse_n, h.card, MAX_HISTOGRAM.max(h.counts.len()))?;
                for f in 0..fine_n {
                    let Some(g) = map.get(f).copied() else { continue };
                    let src = h.group(f).to_vec();
                    let start = g * h.card;
                    let dst = out.counts.get_mut(start..start + h.card)?;
                    for (a, b) in dst.iter_mut().zip(src) {
                        *a = a.wrapping_add(b);
                    }
                }
                Acc::Hist(out)
            }
            Acc::Float { .. } | Acc::Value(_) => return None,
        };
        Some(Entry { cards, acc })
    }

    /// Bytes per group of the state of `agg` over a column of `card` codes.
    fn group_bytes(agg: Agg, card: usize) -> usize {
        match agg {
            // a bitmap per group up to `MAX_DENSE_DISTINCT_BYTES`, else sorted lists whose size
            // follows the rows, not the groups
            Agg::Distinct | Agg::DistinctNoBlank => card.div_ceil(8).clamp(1, 16),
            Agg::Median | Agg::StdDev | Agg::Var | Agg::StdDevP | Agg::VarP => card.saturating_mul(8),
            _ => 24,
        }
    }

    /// `e` as one aggregation that only its filter context decides: `AGG(T[c])`,
    /// `COUNTROWS(T)`, or a measure whose expression is one of those (the model has no
    /// calculation group, which could change a measure's expression).
    pub(super) fn direct_agg(&mut self, e: &Expr, env: &Env, depth: usize) -> Option<(Agg, ColId)> {
        if self.store.is_none() || !self.groups.is_empty() || depth > 16 {
            return None;
        }
        match e {
            Expr::Call { name, args, .. } => {
                let up = name.to_ascii_uppercase();
                if up == "COUNTROWS" {
                    let Some(Expr::Name { name, .. }) = args.first().and_then(Option::as_ref) else {
                        return None;
                    };
                    if args.len() != 1 || env.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
                        return None;
                    }
                    let t = self.table_id(name)?;
                    let c = self.value_cols(t).first().copied()?;
                    return self.col(c).is_ok().then_some((Agg::CountRows, c));
                }
                let agg = agg_function(&up, args.len())?;
                let [
                    Some(
                        col @ Expr::Column {
                            table: Some(t),
                            name,
                            variation: None,
                            ..
                        },
                    ),
                ] = args.as_slice()
                else {
                    return None;
                };
                let c = self.col_id(t, name)?;
                let _ = col;
                Some((agg, c))
            }
            Expr::Column {
                table,
                name,
                variation: None,
                ..
            } => {
                if self.refs.contains_key(&(e as *const Expr as usize)) {
                    return None;
                }
                let def = self.measure_def(table.as_deref(), name)?;
                let body = def.expr.as_ref()?;
                self.direct_agg(body, &Env::new(), depth.saturating_add(1))
            }
            _ => None,
        }
    }

    /// `agg(c)` for each combination of codes of `groups` (columns of `combos`), each in `fc`
    /// plus one filter per grouping column: one grouped scan. `None`: the cells are evaluated one
    /// by one.
    pub(super) fn vector_agg(
        &mut self,
        agg: Agg,
        c: ColId,
        fc: &FCtx,
        groups: &[ColId],
        combos: &Combos,
    ) -> Result<Option<Vec<V>>, Fail> {
        self.vector_rows(agg, c, fc, groups, combos.len(), |i, pos, out| {
            let Some(combo) = combos.get(i) else {
                return false;
            };
            for (o, p) in out.iter_mut().zip(pos) {
                match combo.get(*p) {
                    Some(x) => *o = *x,
                    None => return false,
                }
            }
            true
        })
    }

    /// Fills the cache with the sums (and averages) of several compressed columns of one table
    /// over the same filters and grouping, computed in one pass, before the items read them one
    /// by one ([`Engine::vector_agg`]).
    pub(super) fn prefetch_sums(
        &mut self,
        aggs: &[(Agg, ColId)],
        fc: &FCtx,
        groups: &[ColId],
    ) -> Result<(), Fail> {
        let wanted: Vec<(Agg, ColId)> = aggs
            .iter()
            .copied()
            .filter(|(a, _)| matches!(a, Agg::Sum | Agg::Avg))
            .collect();
        let Some((_, first)) = wanted.first().copied() else {
            return Ok(());
        };
        let t = self.col_table(first)?;
        if wanted.len() < 2 || wanted.iter().any(|(_, c)| self.col_table(*c).ok() != Some(t)) {
            return Ok(());
        }
        let n = self.rows(t)?;
        let reach = self.reach(fc, t)?;
        let mut keys: Vec<(ColId, Arc<Lut>)> = Vec::new();
        for g in groups {
            if !reach.get(self.col_table(*g)?).copied().unwrap_or(true) {
                continue;
            }
            let Some(l) = self.group_lut(fc, t, *g)? else {
                return Ok(());
            };
            keys.push((*g, l));
        }
        let groups_n = keys
            .iter()
            .try_fold(1usize, |a, (_, l)| a.checked_mul(l.card.max(1)));
        if groups_n.is_none_or(|g| g > (1 << 16)) {
            return Ok(());
        }
        let base: Vec<Filter> = fc
            .filters
            .iter()
            .filter(|f| reach.get(f.table).copied().unwrap_or(true))
            .cloned()
            .collect();
        let mut fps = Vec::with_capacity(base.len());
        for f in &base {
            fps.push(self.filter_fp(f)?);
        }
        fps.sort_unstable();
        // the columns to sum: compressed, Currency or whole numbers, read by id
        struct Col {
            agg: Agg,
            c: ColId,
            d: Arc<ColData>,
            cur: bool,
            affine: Option<(i64, i64)>,
            table: Option<(Vec<i64>, Vec<bool>)>,
        }
        let mut cols: Vec<Col> = Vec::new();
        for (agg, c) in wanted {
            let key = EntryKey {
                agg,
                col: c,
                groups: keys.iter().map(|(g, _)| *g).collect(),
                base: fps.clone(),
                rels: rel_key(fc),
            };
            if self.store_mut()?.entries.contains_key(&key) {
                continue;
            }
            let d = self.col(c)?;
            let cur = d.kinds().0;
            let per_code = if cur {
                self.cur_codes(&d)
            } else {
                self.int_codes(c, &d)?
            };
            let Some(per_code) = per_code else {
                continue;
            };
            let Some(packed) = d.codes.col_packed() else {
                continue;
            };
            let affine = d.codes.affine().map(|(_, a, b)| (a, b));
            let table = if affine.is_some() {
                None
            } else if packed.lut().len() <= (1 << 20) {
                let mut values = Vec::with_capacity(packed.lut().len());
                let mut valid = Vec::with_capacity(packed.lut().len());
                for code in packed.lut() {
                    let v = per_code.get(*code as usize).copied().flatten();
                    values.push(v.unwrap_or(0));
                    valid.push(v.is_some());
                }
                Some((values, valid))
            } else {
                continue;
            };
            cols.push(Col {
                agg,
                c,
                d: Arc::clone(&d),
                cur,
                affine,
                table,
            });
        }
        if cols.len() < 2 {
            return Ok(());
        }
        let base_fc = FCtx {
            filters: base,
            rels: fc.rels.clone(),
            applied: fc.applied,
            memo: Default::default(),
        };
        let p = self.pred(&base_fc, t)?.unwrap_or_else(|| PredSet::all(n));
        // an aggregation covers these sums: each is read from it (`Engine::compute_with`)
        if let Some(first) = cols.first()
            && self
                .agg_entry(first.agg, first.c, &first.d, n, Some(&p), &keys)?
                .is_some()
        {
            return Ok(());
        }
        let mut datas = Vec::with_capacity(keys.len());
        for (_, l) in &keys {
            datas.push(self.col(l.col)?);
        }
        self.tick(n)?;
        let results = {
            let parts: Vec<scan::KeyPart<'_>> = keys
                .iter()
                .zip(&datas)
                .map(|((_, l), dd)| scan::KeyPart {
                    codes: dd.codes.col(),
                    lut: l.map.as_deref(),
                    card: l.card,
                })
                .collect();
            let Some(key) = scan::Key::new(parts) else {
                return Ok(());
            };
            let sel = p.selection();
            let mut spec: Vec<(&scan::Packed, scan::IdValues<'_>)> = Vec::new();
            for col in &cols {
                let Some(packed) = col.d.codes.col_packed() else {
                    return Ok(());
                };
                let vals = match (&col.affine, &col.table) {
                    (Some(_), _) => scan::IdValues::Affine {
                        first: bicraft_vertipaq::FIRST_DATA_ID,
                    },
                    (None, Some((values, valid))) => scan::IdValues::Table { values, valid },
                    (None, None) => return Ok(()),
                };
                spec.push((packed, vals));
            }
            let (g, ns) = (key.groups(), spec.len());
            let (threads, _sinks) = self.scan_threads(n, g.saturating_mul(24 * ns), &move || {
                format!("the grouped sums of {ns} columns ({g} groups)")
            })?;
            match scan::sum_by_id_multi(&sel, &key, &spec, threads) {
                Some(r) => r,
                None => return internal(),
            }
        };
        let cards: Vec<usize> = keys.iter().map(|(_, l)| l.card).collect();
        for (col, (sums, counts)) in cols.iter().zip(results) {
            let sums: Vec<i128> = match col.affine {
                Some((a, b)) => sums
                    .iter()
                    .zip(&counts)
                    .map(|(s, n)| {
                        i128::from(a)
                            .saturating_mul(i128::from(*n))
                            .saturating_add(i128::from(b).saturating_mul(*s))
                    })
                    .collect(),
                None => sums,
            };
            let acc = if col.cur {
                Acc::Cur { sums, counts }
            } else {
                Acc::Int { sums, counts }
            };
            let e = Arc::new(Entry {
                cards: cards.clone(),
                acc,
            });
            let key = EntryKey {
                agg: col.agg,
                col: col.c,
                groups: keys.iter().map(|(g, _)| *g).collect(),
                base: fps.clone(),
                rels: rel_key(fc),
            };
            let bytes = entry_bytes(&e);
            self.keep_charge(&|| "a grouped scan's result".into(), bytes)?;
            let store = self.store_mut()?;
            if store.bytes.saturating_add(bytes) > store.cap() {
                store.entries.clear();
                store.bytes = 0;
            }
            store.bytes = store.bytes.saturating_add(bytes);
            store.entries.insert(key, e);
        }
        Ok(())
    }

    /// `agg(c)` for `count` rows whose codes of `groups` (by position in `groups`) `codes_at`
    /// writes (`false`: a value a column does not hold, nothing visible).
    fn vector_rows(
        &mut self,
        agg: Agg,
        c: ColId,
        fc: &FCtx,
        groups: &[ColId],
        count: usize,
        codes_at: impl Fn(usize, &[usize], &mut [u32]) -> bool,
    ) -> Result<Option<Vec<V>>, Fail> {
        let Some(plan) = self.vector_plan(agg, c, fc, groups)? else {
            return Ok(None);
        };
        self.vector_fill(&plan, count, codes_at).map(Some)
    }

    /// The grouped scan of `agg(c)` by `groups` in `fc` that [`Engine::vector_rows`] reads its
    /// values from (`None`: the cells are evaluated one by one). Held, it is read again without
    /// looking it up (the storage cache may drop it meanwhile).
    pub(super) fn vector_plan(
        &mut self,
        agg: Agg,
        c: ColId,
        fc: &FCtx,
        groups: &[ColId],
    ) -> Result<Option<VecPlan>, Fail> {
        let t = self.col_table(c)?;
        let n = self.rows(t)?;
        let d = self.col(c)?;
        let reach = self.reach(fc, t)?;
        let mut keys: Vec<(ColId, Arc<Lut>)> = Vec::new();
        let mut pos = Vec::new();
        for (gi, g) in groups.iter().enumerate() {
            if !reach.get(self.col_table(*g)?).copied().unwrap_or(true) {
                continue;
            }
            let Some(l) = self.group_lut(fc, t, *g)? else {
                return Ok(None);
            };
            keys.push((*g, l));
            pos.push(gi);
        }
        let groups_n = keys
            .iter()
            .try_fold(1usize, |a, (_, l)| a.checked_mul(l.card.max(1)));
        match groups_n {
            Some(g)
                if g <= scan::MAX_GROUPS
                    && g.saturating_mul(Self::group_bytes(agg, d.values.len())) <= MAX_ENTRY_BYTES => {}
            _ => return Ok(None),
        }
        let base: Vec<Filter> = fc
            .filters
            .iter()
            .filter(|f| reach.get(f.table).copied().unwrap_or(true))
            .cloned()
            .collect();
        let Some(entry) = self.entry(agg, c, &d, t, n, fc, base, &keys)? else {
            return Ok(None);
        };
        Ok(Some(VecPlan {
            agg,
            c,
            d,
            entry,
            pos,
        }))
    }

    /// The values of `plan` for `count` rows (see [`Engine::vector_rows`]).
    pub(super) fn vector_fill(
        &mut self,
        plan: &VecPlan,
        count: usize,
        codes_at: impl Fn(usize, &[usize], &mut [u32]) -> bool,
    ) -> Result<Vec<V>, Fail> {
        self.tick(count)?;
        let mut out = Vec::with_capacity(count);
        let mut codes = vec![0u32; plan.pos.len()];
        for i in 0..count {
            let g = if codes_at(i, &plan.pos, &mut codes) {
                plan.entry.group(&codes)
            } else {
                None
            };
            out.push(match g {
                None => V::Blank,
                Some(g) => self.value_of(plan.agg, plan.c, &plan.d, &plan.entry, g)?,
            });
        }
        Ok(out)
    }

    /// [`Engine::vector_plan`] without its scan: what [`Engine::range_entry`] needs to compute it
    /// a range of groups at a time. `None` where `vector_plan` gives `None` (the shape is not one
    /// grouped scan).
    pub(super) fn range_plan(
        &mut self,
        agg: Agg,
        c: ColId,
        fc: &FCtx,
        groups: &[ColId],
    ) -> Result<Option<RangePlan>, Fail> {
        let t = self.col_table(c)?;
        let n = self.rows(t)?;
        let d = self.col(c)?;
        let reach = self.reach(fc, t)?;
        let mut keys: Vec<(ColId, Arc<Lut>)> = Vec::new();
        let mut pos = Vec::new();
        for (gi, g) in groups.iter().enumerate() {
            if !reach.get(self.col_table(*g)?).copied().unwrap_or(true) {
                continue;
            }
            let Some(l) = self.group_lut(fc, t, *g)? else {
                return Ok(None);
            };
            keys.push((*g, l));
            pos.push(gi);
        }
        let group_bytes = Self::group_bytes(agg, d.values.len());
        let groups_n = keys
            .iter()
            .try_fold(1usize, |a, (_, l)| a.checked_mul(l.card.max(1)));
        match groups_n {
            Some(g) if g <= scan::MAX_GROUPS && g.saturating_mul(group_bytes) <= MAX_ENTRY_BYTES => {}
            _ => return Ok(None),
        }
        let base: Vec<Filter> = fc
            .filters
            .iter()
            .filter(|f| reach.get(f.table).copied().unwrap_or(true))
            .cloned()
            .collect();
        let base_fc = FCtx {
            filters: base,
            rels: fc.rels.clone(),
            applied: fc.applied,
            memo: Default::default(),
        };
        let p = self.pred(&base_fc, t)?.unwrap_or_else(|| PredSet::all(n));
        let cards = keys.iter().map(|(_, l)| l.card).collect();
        Ok(Some(RangePlan {
            agg,
            c,
            d,
            n,
            p,
            keys,
            pos,
            cards,
            group_bytes,
        }))
    }

    /// The states of the groups `lo..lo + len` of `plan` (one grouped scan restricted to them),
    /// reserved while they are held. `None`: the reference path would run.
    pub(super) fn range_entry(
        &mut self,
        plan: &RangePlan,
        lo: usize,
        len: usize,
    ) -> Result<Option<RangeEntry>, Fail> {
        let Some(entry) = self.scan_entry_in(
            plan.agg,
            plan.c,
            &plan.d,
            plan.n,
            &plan.p,
            &plan.keys,
            Some((lo, len)),
        )?
        else {
            return Ok(None);
        };
        let held = len.min(plan.groups().saturating_sub(lo));
        let bytes = entry_bytes(&entry);
        let charge = self.charge_bytes(
            &move || {
                format!(
                    "the states of the groups {lo} to {} of a grouping",
                    lo.saturating_add(held)
                )
            },
            bytes,
        )?;
        Ok(Some(RangeEntry {
            entry,
            lo,
            len: held,
            _charge: charge,
        }))
    }

    /// The value of group `g` (numbered as the whole scan numbers it) from the states of its
    /// range; blank outside the range.
    pub(super) fn range_value(&mut self, plan: &RangePlan, e: &RangeEntry, g: usize) -> Result<V, Fail> {
        match g.checked_sub(e.lo).filter(|x| *x < e.len) {
            Some(x) => self.value_of(plan.agg, plan.c, &plan.d, &e.entry, x),
            None => Ok(V::Blank),
        }
    }

    /// The measure `body` for each row of `t` (context transition), when it is one aggregation
    /// ([`Engine::direct_agg`]) and `t` a table of values whose columns all come from the model:
    /// the rows' filters replace those of `s` on the same columns, then one grouped scan.
    pub(super) fn each_direct(&mut self, t: &Tab, body: &Expr, s: &Scope) -> Result<Option<Vec<V>>, Fail> {
        let Some(cols) = self.direct_rows(t, s)? else {
            return Ok(None);
        };
        let Expr::Column { table, name, .. } = body else {
            return Ok(None);
        };
        if self.measure_def(table.as_deref(), name).is_none() {
            return Ok(None);
        }
        let Some((agg, c)) = self.direct_agg(body, s.env, 0) else {
            return Ok(None);
        };
        self.each_agg(t, &cols, agg, c, s)
    }

    /// [`Engine::each_direct`] for the items of `ADDCOLUMNS`: rows of values, `None` unless every
    /// item is a direct measure.
    pub(super) fn each_direct_items(
        &mut self,
        t: &Tab,
        items: &[(String, &Expr)],
        s: &Scope,
    ) -> Result<Option<Vec<Vec<V>>>, Fail> {
        if items.is_empty() || self.store.is_none() {
            return Ok(None);
        }
        let mut cols_out: Vec<Vec<V>> = Vec::with_capacity(items.len());
        for (_, e) in items {
            match self.each_direct(t, e, s)? {
                Some(v) => cols_out.push(v),
                None => return Ok(None),
            }
        }
        let mut rows: Vec<Vec<V>> = (0..t.len()).map(|_| Vec::with_capacity(items.len())).collect();
        for col in cols_out {
            for (r, v) in rows.iter_mut().zip(col) {
                r.push(v);
            }
        }
        Ok(Some(rows))
    }

    /// The lineage columns of a table of values iterated with context transition from `s`, when
    /// the storage engine can take it: no outer row context, no calculation group, every column
    /// from a distinct model column.
    fn direct_rows(&mut self, t: &Tab, s: &Scope) -> Result<Option<Vec<ColId>>, Fail> {
        if self.store.is_none() || !self.groups.is_empty() || !s.rc.is_empty() {
            return Ok(None);
        }
        let Tab::Tuples { cols, .. } = t else {
            return Ok(None);
        };
        let mut lin = Vec::with_capacity(cols.len());
        for c in cols.iter() {
            match c.lin {
                Some(l) if !lin.contains(&l) => lin.push(l),
                _ => return Ok(None),
            }
        }
        Ok(Some(lin))
    }

    /// For the context transition over the values of `k`, a column with one row per value (a
    /// key), drops the filters of `fc` on `k`'s table when every iterated value's row passes them:
    /// each row's context then selects that one row of the table with or without them (and so
    /// filters what it relates to the same way), so the values are the same, and one grouped scan
    /// serves every cell of a grouping by another column of that table
    /// (`SUMMARIZECOLUMNS(Client[Segment], "m", AVERAGEX(VALUES(Client[ClientKey]), [CA]))`).
    fn drop_implied_filters(&mut self, t: &Tab, k: ColId, fc: &mut FCtx) -> Result<(), Fail> {
        let dt = self.col_table(k)?;
        if !fc.filters.iter().any(|f| f.table == dt) {
            return Ok(());
        }
        let Some(codes) = self.tuple_codes(t, k) else {
            return Ok(());
        };
        if codes.contains(&0) {
            return Ok(());
        }
        // one row per value: as many codes as rows and no blank
        let d = self.col(k)?;
        let n = self.rows(dt)?;
        if d.values.len() != n.saturating_add(1) || self.blank_code_kept(&d)? {
            return Ok(());
        }
        let own = FCtx {
            filters: fc.filters.iter().filter(|f| f.table == dt).cloned().collect(),
            ..FCtx::default()
        };
        let visible = self.values_codes(&own, k, false)?;
        let mut seen = vec![false; d.values.len()];
        for v in &visible {
            if let Some(x) = seen.get_mut(*v as usize) {
                *x = true;
            }
        }
        self.tick(codes.len())?;
        if codes
            .iter()
            .all(|c| seen.get(*c as usize).copied().unwrap_or(false))
        {
            fc.filters.retain(|f| f.table != dt);
        }
        Ok(())
    }

    fn each_agg(
        &mut self,
        t: &Tab,
        cols: &[ColId],
        agg: Agg,
        c: ColId,
        s: &Scope,
    ) -> Result<Option<Vec<V>>, Fail> {
        let Tab::Tuples { rows, .. } = t else {
            return Ok(None);
        };
        // the context of every row: the filters of `s` without those on the iterated columns
        // (as the context transition removes them), then one filter per column
        let set: std::collections::HashSet<ColId> = cols.iter().copied().collect();
        let mut fc = s.fc.child();
        self.remove_cols(&mut fc.filters, &set)?;
        if let [k] = cols {
            self.drop_implied_filters(t, *k, &mut fc)?;
        }
        let mut fps = Vec::with_capacity(fc.filters.len());
        for f in &fc.filters.clone() {
            fps.push(self.filter_fp(f)?);
        }
        fps.sort_unstable();
        let memo = (
            Arc::as_ptr(rows) as usize,
            (agg as usize) << 32 ^ c,
            fps,
            rel_key(&fc),
        );
        if let Some((_, v)) = self.store_mut()?.each.get(&memo) {
            let v = Arc::clone(v);
            self.tick(v.len())?;
            return Ok(Some(v.as_ref().clone()));
        }
        let mut codes: Vec<Vec<Option<u32>>> = Vec::with_capacity(rows.len());
        // a table of one column's values built from its codes: the codes the lookups give
        let known = match cols {
            [c1] => self.tuple_codes(t, *c1),
            _ => None,
        };
        if let Some(known) = known {
            codes.extend(known.iter().map(|x| vec![Some(*x)]));
        } else {
            for r in rows.iter() {
                let mut k = Vec::with_capacity(cols.len());
                for (col, v) in cols.iter().zip(r) {
                    k.push(self.code_of(*col, v)?);
                }
                codes.push(k);
            }
        }
        let out = self.vector_rows(agg, c, &fc, cols, rows.len(), |i, pos, out| {
            let Some(k) = codes.get(i) else {
                return false;
            };
            for (o, p) in out.iter_mut().zip(pos) {
                match k.get(*p).copied().flatten() {
                    Some(x) => *o = x,
                    None => return false,
                }
            }
            true
        })?;
        if let Some(v) = &out {
            let store = self.store_mut()?;
            if store.each.len() > 4096 {
                store.each.clear();
            }
            store.each.insert(memo, (t.clone(), Arc::new(v.clone())));
        }
        Ok(out)
    }

    /// `ALL(cols)` (columns of one table) by one parallel scan, kept for the evaluation (the same
    /// table value each time, which the caches above key on). `None`: the reference path.
    pub(super) fn all_tuples_storage(&mut self, cols: &[ColId]) -> Result<Option<Tab>, Fail> {
        if self.store.is_none() {
            return Ok(None);
        }
        if let Some(t) = self.store_mut()?.all.get(cols) {
            let t = t.clone();
            // its codes, for this evaluation's lookups of its values
            if let [c] = cols
                && let Some(codes) = self.store_mut()?.all_codes.get(c).cloned()
                && let Tab::Tuples { rows, .. } = &t
            {
                self.store_mut()?
                    .note_tuple_codes(rows, *c, codes.as_ref().clone());
            }
            return Ok(Some(t));
        }
        let Some(first) = cols.first().copied() else {
            return Ok(None);
        };
        let t = self.col_table(first)?;
        for c in cols {
            if self.col_table(*c)? != t {
                return Ok(None);
            }
        }
        let Some(keys) = self.combos_storage(&FCtx::default(), t, cols)? else {
            return Ok(None);
        };
        let datas = cols.iter().map(|c| self.col(*c)).collect::<Result<Vec<_>, _>>()?;
        let rows: Vec<Vec<V>> = keys
            .iter()
            .map(|k: &[u32]| {
                datas
                    .iter()
                    .zip(k)
                    .map(|(d, code)| d.values.get(*code as usize).map_or(V::Blank, V::of_cell))
                    .collect()
            })
            .collect();
        let tab = Tab::tuples(cols.iter().map(|c| super::TCol::of(*c)).collect(), rows);
        self.store_mut()?.all.insert(cols.to_vec(), tab.clone());
        if let [c] = cols
            && self.own_keys(*c)?
        {
            let codes: Vec<u32> = keys
                .iter()
                .map(|k: &[u32]| k.first().copied().unwrap_or(0))
                .collect();
            self.store_mut()?.all_codes.insert(*c, Arc::new(codes.clone()));
            self.note_tuple_codes(&tab, *c, codes)?;
        }
        Ok(Some(tab))
    }

    /// The columns of `t` an expression reads in a row context of `t` (for `RELATED`, the column
    /// of `t` the relationship starts from), when it reads nothing else: literals, scalar
    /// variables, columns of `t`, `RELATED`, operators and pure scalar functions. `false`:
    /// something else (a measure, an aggregation, a table…).
    pub(super) fn row_deps(
        &mut self,
        e: &Expr,
        t: TabId,
        env: &Env,
        out: &mut Vec<ColId>,
        depth: usize,
    ) -> Result<bool, Fail> {
        if depth > 64 {
            return Ok(false);
        }
        let d = depth.saturating_add(1);
        Ok(match e {
            Expr::Number { .. } | Expr::Str { .. } | Expr::Bool { .. } => true,
            Expr::Name { name, .. } => {
                let lower = name.to_lowercase();
                matches!(
                    env.iter().rev().find(|(n, _)| *n == lower),
                    Some((_, super::Bind::S(_)))
                )
            }
            Expr::Column {
                table,
                name,
                variation: None,
                ..
            } => {
                if !self.refs.contains_key(&(e as *const Expr as usize))
                    && self.measure_def(table.as_deref(), name).is_some()
                {
                    return Ok(false);
                }
                let c = match table {
                    Some(tn) => self.col_id(tn, name),
                    None => self.tabs.get(t).and_then(|info| {
                        info.cols.iter().copied().find(|c| {
                            self.cols
                                .get(*c)
                                .is_some_and(|i| i.name.eq_ignore_ascii_case(name))
                        })
                    }),
                };
                match c {
                    Some(c) if self.col_table(c)? == t => {
                        if !out.contains(&c) {
                            out.push(c);
                        }
                        true
                    }
                    _ => false,
                }
            }
            Expr::Unary { operand, .. } => self.row_deps(operand, t, env, out, d)?,
            Expr::Binary { op, lhs, rhs, .. } => {
                !op.eq_ignore_ascii_case("IN")
                    && self.row_deps(lhs, t, env, out, d)?
                    && self.row_deps(rhs, t, env, out, d)?
            }
            Expr::Call { name, args, .. } => {
                let up = name.to_ascii_uppercase();
                if up == "RELATED" {
                    let [
                        Some(Expr::Column {
                            table: Some(tn),
                            name,
                            variation: None,
                            ..
                        }),
                    ] = args.as_slice()
                    else {
                        return Ok(false);
                    };
                    let Some(target) = self.col_id(tn, name).map(|c| self.col_table(c)).transpose()? else {
                        return Ok(false);
                    };
                    // the first hop of the many-to-one path from `t` to the column's table
                    let path = self.expanded(t);
                    let Some(mut at) = path.iter().position(|(x, _)| *x == target) else {
                        return Ok(false);
                    };
                    let first = loop {
                        let Some((_, Some((r, from)))) = path.get(at) else {
                            return Ok(false);
                        };
                        if *from == t {
                            break *r;
                        }
                        match path.iter().position(|(x, _)| x == from) {
                            Some(p) => at = p,
                            None => return Ok(false),
                        }
                    };
                    let Some(rel) = self.rels.get(first) else {
                        return Ok(false);
                    };
                    if !out.contains(&rel.from) {
                        out.push(rel.from);
                    }
                    return Ok(true);
                }
                let pure = matches!(
                    up.as_str(),
                    "IF" | "DIVIDE"
                        | "ABS"
                        | "ROUND"
                        | "INT"
                        | "SIGN"
                        | "AND"
                        | "OR"
                        | "NOT"
                        | "ISBLANK"
                        | "COALESCE"
                        | "BLANK"
                        | "TRUE"
                        | "FALSE"
                        | "SWITCH"
                        | "YEAR"
                        | "MONTH"
                        | "DAY"
                        | "QUARTER"
                        | "DATE"
                        | "CURRENCY"
                ) || (up == "MIN" || up == "MAX") && args.len() == 2
                    || super::math::handles(&up)
                    || super::text::handles(&up);
                if !pure {
                    return Ok(false);
                }
                for a in args.iter().flatten() {
                    if !self.row_deps(a, t, env, out, d)? {
                        return Ok(false);
                    }
                }
                true
            }
            _ => false,
        })
    }

    /// The rows of a model table grouped by the codes of the columns `body` reads
    /// ([`Engine::row_deps`]): (row count, one row) per combination met, `body` evaluated in that
    /// row, plus the virtual blank row's value when listed. `None`: the reference path.
    #[allow(clippy::type_complexity)]
    fn tally_by_codes(
        &mut self,
        t: &Tab,
        body: &Expr,
        s: &Scope,
    ) -> Result<Option<(Vec<(V, u64)>, Option<V>)>, Fail> {
        let Tab::Rows {
            table,
            rows,
            ext: None,
        } = t
        else {
            return Ok(None);
        };
        if self.store.is_none() || rows.len() < 64 {
            return Ok(None);
        }
        let mut deps = Vec::new();
        if !self.row_deps(body, *table, s.env, &mut deps, 0)? {
            return Ok(None);
        }
        let n = self.rows(*table)?;
        let datas = self.body_parts(s.fc, *table, body, s.env, &deps)?;
        let parts: Vec<scan::KeyPart<'_>> = datas
            .iter()
            .map(|(_, d, l)| scan::KeyPart {
                codes: d.codes.col(),
                lut: l.as_ref().and_then(|l| l.map.as_deref()),
                card: l.as_ref().map_or(d.values.len(), |l| l.card),
            })
            .collect();
        let Some(key) = scan::Key::new(parts) else {
            return Ok(None);
        };
        let groups = key.groups();
        if groups > (1 << 22) {
            return Ok(None);
        }
        self.tick(rows.len())?;
        let real: Vec<u32>;
        let (list, blank): (&[u32], bool) = match rows.last() {
            Some(r) if *r as usize >= n => {
                real = rows.iter().copied().filter(|r| (*r as usize) < n).collect();
                (&real, true)
            }
            _ => (rows.as_slice(), false),
        };
        let (threads, _sinks) = self.scan_threads(list.len(), groups.saturating_mul(12), &move || {
            format!("a count of {groups} groups")
        })?;
        let Some(tally) = scan::scan_list(list, &key, None, threads, || scan::Tally::new(groups)) else {
            return internal();
        };
        drop(key);
        let mut reps: Vec<(u32, u64)> = tally
            .first
            .iter()
            .zip(&tally.counts)
            .filter(|(f, _)| **f != u32::MAX)
            .map(|(f, c)| (*f, *c))
            .collect();
        // in the order of their first row, as the reference meets them
        reps.sort_unstable();
        if blank {
            reps.push((u32::try_from(n).unwrap_or(u32::MAX), 1));
        }
        let rep_tab = Tab::rows_of(*table, reps.iter().map(|(r, _)| *r).collect());
        let vals = self.each_with(&rep_tab, s, |eng, inner| eng.scalar(body, inner))?;
        let mut out: Vec<(V, u64)> = vals.into_iter().zip(reps.iter().map(|(_, c)| *c)).collect();
        let blank_val = if blank { out.pop().map(|(v, _)| v) } else { None };
        Ok(Some((out, blank_val)))
    }

    /// `SUMX` / `AVERAGEX` / `MINX` / `MAXX` / `COUNTX` / `COUNTAX` over the rows of a model table
    /// whose expression reads only some of its columns: once per combination of their codes,
    /// weighted by its rows (as `iterate_agg` over every row). `None`: the reference path.
    pub(super) fn iterate_by_codes(
        &mut self,
        f: &str,
        t: &Tab,
        body: &Expr,
        s: &Scope,
    ) -> Result<Option<V>, Fail> {
        let Some((mut vals, blank)) = self.tally_by_codes(t, body, s)? else {
            return Ok(None);
        };
        if let Some(b) = blank {
            vals.push((b, 1));
        }
        self.weighted(f, vals).map(Some)
    }

    /// Whether expressions depend only on the filter context they are evaluated in, the model
    /// and its data (what a result kept by text needs): no clock, no random value, no user, no
    /// `ALLSELECTED` (which reads the context of an outer iteration), in them or in the
    /// measures they use.
    pub(super) fn pure_args(&mut self, args: &[Option<Expr>]) -> bool {
        let mut seen = std::collections::HashSet::new();
        args.iter().flatten().all(|e| self.pure_expr(e, 0, &mut seen))
    }

    /// [`Engine::pure_args`] of expressions given by reference.
    pub(super) fn pure_exprs(&mut self, es: &[&Expr]) -> bool {
        let mut seen = std::collections::HashSet::new();
        es.iter().all(|e| self.pure_expr(e, 0, &mut seen))
    }

    fn pure_expr(&mut self, e: &Expr, depth: usize, seen: &mut std::collections::HashSet<String>) -> bool {
        if depth > 200 {
            return false;
        }
        let d = depth + 1;
        match e {
            Expr::Number { .. } | Expr::Str { .. } | Expr::Bool { .. } | Expr::Name { .. } => true,
            Expr::Column { table, name, .. } => {
                let key = format!("{}\u{0}{}", table.as_deref().unwrap_or(""), name.to_lowercase());
                if seen.contains(&key) {
                    return true;
                }
                match self.measure_def(table.as_deref(), name) {
                    Some(def) => {
                        seen.insert(key);
                        match &def.expr {
                            Some(body) => self.pure_expr(body, d, seen),
                            None => false,
                        }
                    }
                    None => true,
                }
            }
            Expr::Unary { operand, .. } => self.pure_expr(operand, d, seen),
            Expr::Binary { lhs, rhs, .. } => self.pure_expr(lhs, d, seen) && self.pure_expr(rhs, d, seen),
            Expr::Var { bindings, body, .. } => {
                bindings.iter().all(|b| self.pure_expr(&b.value, d, seen)) && self.pure_expr(body, d, seen)
            }
            Expr::TableCtor { rows, .. } => rows.iter().all(|r| self.pure_expr(r, d, seen)),
            Expr::Tuple { items, .. } => items.iter().all(|r| self.pure_expr(r, d, seen)),
            Expr::Call { name, args, .. } => {
                let up = name.to_ascii_uppercase();
                let impure = matches!(
                    up.as_str(),
                    "TODAY"
                        | "NOW"
                        | "UTCNOW"
                        | "UTCTODAY"
                        | "ALLSELECTED"
                        | "RAND"
                        | "RANDBETWEEN"
                        | "USERNAME"
                        | "USERPRINCIPALNAME"
                        | "USEROBJECTID"
                        | "USERCULTURE"
                        | "CUSTOMDATA"
                        | "SELECTEDMEASURE"
                        | "SELECTEDMEASURENAME"
                        | "ISSELECTEDMEASURE"
                        | "SELECTEDMEASUREFORMATSTRING"
                );
                !impure && args.iter().flatten().all(|a| self.pure_expr(a, d, seen))
            }
        }
    }

    /// The cache key of a table function's result in the filter context of `s`: its text, the
    /// filters by content and the relationship overrides; `None` when variables are in scope.
    pub(super) fn table_cache_key(
        &mut self,
        f: &str,
        args: &[Option<Expr>],
        s: &Scope,
    ) -> Result<Option<(String, Vec<u64>, RelKey)>, Fail> {
        if self.store.is_none() || !s.env.is_empty() || !s.rc.is_empty() || !self.groups.is_empty() {
            return Ok(None);
        }
        let mut fps = Vec::with_capacity(s.fc.filters.len());
        for flt in &s.fc.filters {
            fps.push(self.filter_fp(flt)?);
        }
        fps.sort_unstable();
        let m = self.store_mut()?.measures;
        Ok(Some((format!("{m}\u{0}{f}\u{0}{args:?}"), fps, rel_key(s.fc))))
    }

    pub(super) fn cached_table(&mut self, k: &(String, Vec<u64>, RelKey)) -> Result<Option<Tab>, Fail> {
        let t = self.store_mut()?.tables.get(k).cloned();
        if let Some(t) = &t {
            self.tick(t.len())?;
        }
        Ok(t)
    }

    pub(super) fn keep_table(&mut self, k: (String, Vec<u64>, RelKey), t: &Tab) -> Result<(), Fail> {
        let cells = match t {
            Tab::Tuples { cols, rows } => rows.len().saturating_mul(cols.len().max(1)),
            Tab::Rows { rows, .. } => rows.len(),
        };
        let store = self.store_mut()?;
        if store.table_cells.saturating_add(cells) > MAX_TABLE_CELLS {
            store.tables.clear();
            store.table_cells = 0;
        }
        if cells <= MAX_TABLE_CELLS {
            store.table_cells = store.table_cells.saturating_add(cells);
            store.tables.insert(k, t.clone());
        }
        Ok(())
    }

    /// `SUMX(T, body)` or `SUMX(FILTER(T, pred), body)` (and the other iterators) over a model
    /// table `T` whose `body` and `pred` read only some of its columns, `pred` at most one: the
    /// visible rows as a selection, `pred` once per code of its column (an allowed-code bitmap
    /// joined to the selection), `body` once per combination of its columns' codes weighted by
    /// its rows. Two scans, no list of rows. `None`: the general path.
    /// `COUNTROWS` of a grouping of one fact column by one aggregation of that table
    /// (`SUMMARIZECOLUMNS(t[g], "x", SUM(t[c]))`, or `COUNTROWS(t)`, possibly under
    /// `CALCULATETABLE`): the groups whose aggregation is not blank are the distinct values of
    /// `t[g]` over the visible rows (whose `t[c]` is not blank), counted without building them.
    pub(super) fn count_grouping(&mut self, e: &Expr, s: &Scope) -> Result<Option<V>, Fail> {
        if self.store.is_none() || !self.groups.is_empty() || !s.rc.is_empty() {
            return Ok(None);
        }
        // kept between queries as the iterators' results (text and filter context)
        let m = self.store_mut()?.measures;
        let key = if !s.env.is_empty() {
            None
        } else {
            let mut fps = Vec::with_capacity(s.fc.filters.len());
            for flt in &s.fc.filters {
                fps.push(self.filter_fp(flt)?);
            }
            fps.sort_unstable();
            Some((format!("{m}\u{0}COUNTROWS\u{0}{e:?}"), fps, rel_key(s.fc)))
        };
        if let Some(k) = &key
            && let Some(v) = self.store_mut()?.iters.get(k)
        {
            let v = v.clone();
            self.tick(1)?;
            return Ok(Some(v));
        }
        let v = self.count_grouping_scan(e, s)?;
        if let (Some(k), Some(v)) = (key, &v)
            && self.pure_exprs(&[e])
        {
            let store = self.store_mut()?;
            if store.iters.len() > 4096 {
                store.iters.clear();
            }
            store.iters.insert(k, v.clone());
        }
        Ok(v)
    }

    fn count_grouping_scan(&mut self, e: &Expr, s: &Scope) -> Result<Option<V>, Fail> {
        let Expr::Call { name, args, .. } = e else {
            return Ok(None);
        };
        if name.eq_ignore_ascii_case("CALCULATETABLE") {
            let Some(Some(inner)) = args.first() else {
                return Ok(None);
            };
            let fc = self.calc_ctx(args, s)?;
            let scope = Scope {
                fc: &fc,
                rc: &[],
                ..*s
            };
            return self.count_grouping_scan(inner, &scope);
        }
        if !name.eq_ignore_ascii_case("SUMMARIZECOLUMNS") {
            return Ok(None);
        }
        let [Some(g @ Expr::Column { .. }), Some(Expr::Str { .. }), Some(item)] = args.as_slice() else {
            return Ok(None);
        };
        let Ok(g) = self.col_arg(g, &[], "SUMMARIZECOLUMNS") else {
            return Ok(None);
        };
        let t = self.col_table(g)?;
        // the item, through measure references (expressions held by their definitions: column
        // references resolve by the expression's address)
        let mut def: Option<Rc<MeasureDef>> = None;
        for _ in 0..8 {
            let next = {
                let cur = match &def {
                    Some(d) => match d.expr.as_ref() {
                        Some(x) => x,
                        None => return Ok(None),
                    },
                    None => item,
                };
                match cur {
                    Expr::Column {
                        table,
                        name,
                        variation: None,
                        ..
                    } if table.as_deref().is_none_or(|tn| self.col_id(tn, name).is_none()) => {
                        Some((table.clone(), name.clone()))
                    }
                    _ => None,
                }
            };
            let Some((tn, mn)) = next else {
                break;
            };
            let Some(d) = self.measure_def(tn.as_deref(), &mn) else {
                break;
            };
            def = Some(d);
        }
        let item: &Expr = match &def {
            Some(d) => match d.expr.as_ref() {
                Some(x) => x,
                None => return Ok(None),
            },
            None => item,
        };
        let Expr::Call {
            name: f, args: fa, ..
        } = item
        else {
            return Ok(None);
        };
        let summed = match (f.to_ascii_uppercase().as_str(), fa.as_slice()) {
            ("SUM", [Some(c @ Expr::Column { .. })]) => {
                let Ok(c) = self.col_arg(c, &[], "SUM") else {
                    return Ok(None);
                };
                if self.col_table(c)? != t {
                    return Ok(None);
                }
                Some(c)
            }
            ("COUNTROWS", [Some(Expr::Name { name, .. })]) if self.table_id(name) == Some(t) => None,
            _ => return Ok(None),
        };
        let n = self.rows(t)?;
        let mut p = self.pred(s.fc, t)?.unwrap_or_else(|| PredSet::all(n));
        if let Some(c) = summed {
            let d = self.col(c)?;
            // numbers only (a sum of anything else fails as the reference does)
            if !d.values.iter().all(|v| {
                matches!(
                    v,
                    CellValue::Null | CellValue::Int(_) | CellValue::Decimal(_) | CellValue::Real(_)
                )
            }) {
                return Ok(None);
            }
            if self.has_blank_code(&d)? || d.values.iter().skip(1).any(|v| matches!(v, CellValue::Null)) {
                let keep: Vec<bool> = d.values.iter().map(|v| !matches!(v, CellValue::Null)).collect();
                p.codes.push((Arc::clone(&d), Arc::new(keep)));
            }
        }
        let dg = self.col(g)?;
        let Some(entry) = self.compute_with(Agg::Distinct, g, &dg, n, Some(p), &[])? else {
            return Ok(None);
        };
        self.value_of(Agg::Distinct, g, &dg, &entry, 0).map(Some)
    }

    /// The values `RANKX(ALL(...), body)` ranks among, sorted (and without repeats, for `DENSE`),
    /// kept between rows and queries: for a table of `ALL` over columns of one table, the
    /// context transition replaces the filters on those columns, so the filters on them are left
    /// out of the key. `None`: not that form, or a value that does not sort (NaN).
    pub(super) fn rank_values(
        &mut self,
        args: &[Option<Expr>],
        s: &Scope,
    ) -> Result<Option<Arc<Ranked>>, Fail> {
        if self.store.is_none() || !s.rc.is_empty() || !s.env.is_empty() {
            return Ok(None);
        }
        let (Some(Some(src)), Some(Some(body))) = (args.first(), args.get(1)) else {
            return Ok(None);
        };
        let Expr::Call { name, args: all, .. } = src else {
            return Ok(None);
        };
        if !name.eq_ignore_ascii_case("ALL") || all.is_empty() {
            return Ok(None);
        }
        // a body that transitions the row context into filters (a measure, CALCULATE): without
        // it the filters on the ranked columns would still apply to each row
        let transitions = match body {
            Expr::Call { name, .. } => name.eq_ignore_ascii_case("CALCULATE"),
            Expr::Column {
                table,
                name,
                variation: None,
                ..
            } => {
                table.as_deref().is_none_or(|tn| self.col_id(tn, name).is_none())
                    && self.measure_def(table.as_deref(), name).is_some()
            }
            _ => false,
        };
        if !transitions {
            return Ok(None);
        }
        let mut cols: Vec<ColId> = Vec::new();
        for a in all {
            match a {
                Some(c @ Expr::Column { .. }) => match self.col_arg(c, &[], "ALL") {
                    Ok(c) => cols.push(c),
                    Err(_) => return Ok(None),
                },
                Some(Expr::Name { name, .. }) if all.len() == 1 => match self.table_id(name) {
                    Some(t) => cols.extend(self.tabs.get(t).map(|i| i.cols.clone()).unwrap_or_default()),
                    None => return Ok(None),
                },
                _ => return Ok(None),
            }
        }
        let m = self.store_mut()?.measures;
        let mut fps = Vec::with_capacity(s.fc.filters.len());
        for f in &s.fc.filters {
            if !f.cols.is_empty() && f.cols.iter().all(|c| cols.contains(c)) {
                continue;
            }
            fps.push(self.filter_fp(f)?);
        }
        fps.sort_unstable();
        let key = (
            format!("{m}\u{0}RANKX\u{0}{src:?}\u{0}{body:?}"),
            fps,
            rel_key(s.fc),
        );
        if let Some(r) = self.store_mut()?.ranks.get(&key) {
            return Ok(Some(Arc::clone(r)));
        }
        let t = self.table(src, s)?;
        let vals = self.each(&t, body, s)?;
        let mut ys = Vec::with_capacity(vals.len());
        for v in &vals {
            let y = super::num(v)?.unwrap_or(0.0);
            if y.is_nan() {
                return Ok(None);
            }
            ys.push(y);
        }
        ys.sort_by(f64::total_cmp);
        let mut dense = ys.clone();
        dense.dedup();
        let r = Arc::new((ys, dense));
        // kept only when the clock, the user or ALLSELECTED cannot change it
        if !self.pure_exprs(&[src, body]) {
            return Ok(Some(r));
        }
        let store = self.store_mut()?;
        if store.ranks.len() > 4096 {
            store.ranks.clear();
        }
        store.ranks.insert(key, Arc::clone(&r));
        Ok(Some(r))
    }

    pub(super) fn iterate_table(
        &mut self,
        f: &str,
        src: &Expr,
        body: &Expr,
        s: &Scope,
    ) -> Result<Option<V>, Fail> {
        if self.store.is_none() {
            return Ok(None);
        }
        // kept between queries when no variable is in scope (the expressions' text and the
        // filter context decide the result)
        let m = self.store_mut()?.measures;
        let text = format!("{m}\u{0}{f}\u{0}{src:?}\u{0}{body:?}");
        let key = if !s.env.is_empty() {
            None
        } else {
            let mut fps = Vec::with_capacity(s.fc.filters.len());
            for flt in &s.fc.filters {
                fps.push(self.filter_fp(flt)?);
            }
            fps.sort_unstable();
            Some((text, fps, rel_key(s.fc)))
        };
        if let Some(k) = &key
            && let Some(v) = self.store_mut()?.iters.get(k)
        {
            let v = v.clone();
            self.tick(1)?;
            return Ok(Some(v));
        }
        let v = self.iterate_table_scan(f, src, body, s)?;
        if let (Some(k), Some(v)) = (key, &v)
            && self.pure_exprs(&[src, body])
        {
            let store = self.store_mut()?;
            if store.iters.len() > 4096 {
                store.iters.clear();
            }
            store.iters.insert(k, v.clone());
        }
        Ok(v)
    }

    fn iterate_table_scan(&mut self, f: &str, src: &Expr, body: &Expr, s: &Scope) -> Result<Option<V>, Fail> {
        if self.store.is_none() {
            return Ok(None);
        }
        let (name, pred) = match src {
            Expr::Name { name, .. } => (name, None),
            Expr::Call {
                name: fname, args, ..
            } if fname.eq_ignore_ascii_case("FILTER") => match args.as_slice() {
                [Some(Expr::Name { name, .. }), Some(p)] => (name, Some(p)),
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        if s.env.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
            return Ok(None);
        }
        let Some(t) = self.table_id(name) else {
            return Ok(None);
        };
        let mut body_deps = Vec::new();
        if !self.row_deps(body, t, s.env, &mut body_deps, 0)? {
            return Ok(None);
        }
        let mut pred_deps = Vec::new();
        if let Some(p) = pred
            && (!self.row_deps(p, t, s.env, &mut pred_deps, 0)? || pred_deps.len() > 1)
        {
            return Ok(None);
        }
        let n = self.rows(t)?;
        let mut p = self.pred(s.fc, t)?.unwrap_or_else(|| PredSet::all(n));
        self.tick(n)?;
        if let Some(pe) = pred {
            // the predicate once per value of its column (or once, without one), in the order
            // of their first visible row
            let dcol = match pred_deps.first() {
                Some(c) => Some(self.col(*c)?),
                None => None,
            };
            // the predicate depends on its column's value alone: once per code of the column,
            // read on the code's first row of the table (visible or not: only the visible rows
            // are then kept); an error falls back to the visible values alone
            let mut by_code = None;
            if let Some(d) = &dcol
                && let Some(first) = d.first_rows()
            {
                let mut reps: Vec<(u32, u32)> = first
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| **r != u32::MAX)
                    .filter_map(|(g, r)| u32::try_from(g).ok().map(|g| (*r, g)))
                    .collect();
                reps.sort_unstable();
                let rep_tab = Tab::rows_of(t, reps.iter().map(|(r, _)| *r).collect());
                if let Ok(vals) = self.each_with(&rep_tab, s, |eng, inner| eng.scalar(pe, inner)) {
                    let mut keep = vec![false; d.values.len()];
                    let mut ok = true;
                    for ((_, code), v) in reps.iter().zip(&vals) {
                        match (keep.get_mut(*code as usize), super::truth(v)) {
                            (Some(k), Ok(b)) => *k = b,
                            _ => ok = false,
                        }
                    }
                    if ok {
                        by_code = Some(keep);
                    }
                }
            }
            if let (Some(d), Some(keep)) = (&dcol, by_code) {
                p.codes.push((Arc::clone(d), Arc::new(keep)));
            } else {
                let reps: Vec<(u32, u32)> = {
                    let sel = p.selection();
                    let (key, groups) = match &dcol {
                        Some(d) => {
                            let Some(k) = scan::Key::new(vec![scan::KeyPart {
                                codes: d.codes.col(),
                                lut: None,
                                card: d.values.len(),
                            }]) else {
                                return Ok(None);
                            };
                            let g = k.groups();
                            (k, g)
                        }
                        None => (scan::Key::single(), 1),
                    };
                    let n = sel.rows;
                    let (_states, routed) =
                        self.grouped_scan_charge("the first rows of values", n, groups, 4)?;
                    let Some(first) =
                        scan::scan_groups_within(&sel, &key, None, 4, routed, scan::FirstRow::new)
                    else {
                        return internal();
                    };
                    let mut reps: Vec<(u32, u32)> = first
                        .0
                        .iter()
                        .enumerate()
                        .filter(|(_, r)| **r != u32::MAX)
                        .filter_map(|(g, r)| u32::try_from(g).ok().map(|g| (*r, g)))
                        .collect();
                    reps.sort_unstable();
                    let _ = groups;
                    reps
                };
                let rep_tab = Tab::rows_of(t, reps.iter().map(|(r, _)| *r).collect());
                let vals = self.each_with(&rep_tab, s, |eng, inner| eng.scalar(pe, inner))?;
                match &dcol {
                    Some(d) => {
                        let mut keep = vec![false; d.values.len()];
                        for ((_, code), v) in reps.iter().zip(&vals) {
                            if let Some(k) = keep.get_mut(*code as usize) {
                                *k = super::truth(v)?;
                            }
                        }
                        p.codes.push((Arc::clone(d), Arc::new(keep)));
                    }
                    None => {
                        let mut keep_all = true;
                        if let Some(v) = vals.first() {
                            keep_all = super::truth(v)?;
                        }
                        if !keep_all {
                            return self.weighted(f, Vec::new()).map(Some);
                        }
                    }
                }
            }
        }
        // `SUMX(T, T[a] * RELATED(D[b]))`: the sums of `a` grouped by `b`, times `b`
        if f == "SUMX"
            && let Some(v) = self.related_product(body, t, n, &p, s.fc)?
        {
            return Ok(Some(v));
        }
        // `SUMX(T, IF(cond, T[a]))`, `cond` reading few values: the sums of `a` by those values
        if f == "SUMX"
            && let Some(v) = self.conditional_sum(body, t, n, &p, s)?
        {
            return Ok(Some(v));
        }
        // a body that is one column of the table: the aggregation of that column over the kept
        // rows (as the iterator adds its values: exact for Currency and whole numbers)
        if let Some(agg) = self.column_body(f, body, t)? {
            let d = match body_deps.first() {
                Some(c) => self.col(*c)?,
                None => return internal(),
            };
            let c = body_deps.first().copied().unwrap_or(0);
            let Some(e) = self.compute_with(agg, c, &d, n, Some(p), &[])? else {
                return Ok(None);
            };
            return self.value_of(agg, c, &d, &e, 0).map(Some);
        }
        // the body once per combination of the values it reads among the kept rows
        let datas = self.body_parts(s.fc, t, body, s.env, &body_deps)?;
        let reps: Vec<(u32, u64)> = {
            let parts: Vec<scan::KeyPart<'_>> = datas
                .iter()
                .map(|(_, d, l)| scan::KeyPart {
                    codes: d.codes.col(),
                    lut: l.as_ref().and_then(|l| l.map.as_deref()),
                    card: l.as_ref().map_or(d.values.len(), |l| l.card),
                })
                .collect();
            let Some(key) = scan::Key::new(parts) else {
                return Ok(None);
            };
            if key.groups() > (1 << 22) {
                return Ok(None);
            }
            let sel = p.selection();
            let (n, g) = (sel.rows, key.groups());
            let (_states, routed) = self.grouped_scan_charge("a tally of combinations", n, g, 12)?;
            let Some(tally) = scan::scan_groups_within(&sel, &key, None, 12, routed, scan::Tally::new) else {
                return internal();
            };
            let mut reps: Vec<(u32, u64)> = tally
                .first
                .iter()
                .zip(&tally.counts)
                .filter(|(f, _)| **f != u32::MAX)
                .map(|(f, c)| (*f, *c))
                .collect();
            reps.sort_unstable();
            reps
        };
        let rep_tab = Tab::rows_of(t, reps.iter().map(|(r, _)| *r).collect());
        let vals = self.each_with(&rep_tab, s, |eng, inner| eng.scalar(body, inner))?;
        let vals: Vec<(V, u64)> = vals.into_iter().zip(reps.iter().map(|(_, c)| *c)).collect();
        self.weighted(f, vals).map(Some)
    }

    /// The aggregation an iterator over `t` computes when its body is one column of `t` holding
    /// Currency or whole numbers: the same result as adding the rows' values one by one.
    fn column_body(&mut self, f: &str, body: &Expr, t: TabId) -> Result<Option<Agg>, Fail> {
        let Expr::Column {
            table,
            name,
            variation: None,
            ..
        } = body
        else {
            return Ok(None);
        };
        if self.measure_def(table.as_deref(), name).is_some() {
            return Ok(None);
        }
        let c = match table {
            Some(tn) => self.col_id(tn, name),
            None => self.tabs.get(t).and_then(|info| {
                info.cols.iter().copied().find(|c| {
                    self.cols
                        .get(*c)
                        .is_some_and(|i| i.name.eq_ignore_ascii_case(name))
                })
            }),
        };
        let Some(c) = c.filter(|c| self.col_table(*c).is_ok_and(|ct| ct == t)) else {
            return Ok(None);
        };
        let d = self.col(c)?;
        let (_, all_cur, all_int) = d.kinds();
        let exact = all_cur || all_int && self.int_codes(c, &d)?.is_some();
        Ok(match f {
            "SUMX" if exact => Some(Agg::Sum),
            "AVERAGEX" if exact => Some(Agg::Avg),
            "MINX" if exact => Some(Agg::Min),
            "MAXX" if exact => Some(Agg::Max),
            "COUNTX" | "COUNTAX" => Some(Agg::Count),
            _ => None,
        })
    }

    /// A column of `t` named by `e` (not a measure), or `None`.
    fn own_column(&mut self, e: &Expr, t: TabId) -> Option<ColId> {
        let Expr::Column {
            table,
            name,
            variation: None,
            ..
        } = e
        else {
            return None;
        };
        if self.measure_def(table.as_deref(), name).is_some() {
            return None;
        }
        let c = match table {
            Some(tn) => self.col_id(tn, name),
            None => self.tabs.get(t).and_then(|info| {
                info.cols.iter().copied().find(|c| {
                    self.cols
                        .get(*c)
                        .is_some_and(|i| i.name.eq_ignore_ascii_case(name))
                })
            }),
        };
        c.filter(|c| self.col_table(*c).is_ok_and(|ct| ct == t))
    }

    /// `SUMX(T, T[a] * RELATED(D[b]))` over the rows `p` keeps (no relationship override): the
    /// sums of `a` grouped by `b` through the many-to-one path, each times its `b`, exactly as
    /// the iterator adds its products: whole numbers and Currency only, and every product exact
    /// in a double (Currency × whole number is Currency, whole × whole a whole number).
    fn related_product(
        &mut self,
        body: &Expr,
        t: TabId,
        n: usize,
        p: &PredSet,
        fc: &FCtx,
    ) -> Result<Option<V>, Fail> {
        let Expr::Binary { op, lhs, rhs, .. } = body else {
            return Ok(None);
        };
        if op != "*" || !rel_key(fc).is_empty() {
            return Ok(None);
        }
        let related = |e: &Expr| match e {
            Expr::Call { name, args, .. } if name.eq_ignore_ascii_case("RELATED") => match args.as_slice() {
                [Some(c @ Expr::Column { .. })] => Some(c.clone()),
                _ => None,
            },
            _ => None,
        };
        let (a, r) = match (
            self.own_column(lhs, t),
            related(rhs),
            self.own_column(rhs, t),
            related(lhs),
        ) {
            (Some(a), Some(r), _, _) => (a, r),
            (_, _, Some(a), Some(r)) => (a, r),
            _ => return Ok(None),
        };
        let Expr::Column {
            table: Some(rt),
            name: rn,
            ..
        } = &r
        else {
            return Ok(None);
        };
        let Some(b) = self.col_id(rt, rn) else {
            return Ok(None);
        };
        if self.col_table(b)? == t {
            return Ok(None);
        }
        let da = self.col(a)?;
        let db = self.col(b)?;
        let (_, a_cur, a_int) = da.kinds();
        let (_, b_cur, b_int) = db.kinds();
        let a_vals = if a_cur && !a_int {
            self.cur_codes(&da)
        } else if a_int {
            self.int_codes(a, &da)?
        } else {
            None
        };
        let b_vals = if b_cur && !b_int {
            self.cur_codes(&db)
        } else if b_int {
            self.int_codes(b, &db)?
        } else {
            None
        };
        let (Some(a_vals), Some(b_vals)) = (a_vals, b_vals) else {
            return Ok(None);
        };
        let (a_is_cur, b_is_cur) = (a_cur && !a_int, b_cur && !b_int);
        if a_is_cur && b_is_cur {
            // Currency × Currency is a double
            return Ok(None);
        }
        // every product, and every partial sum, exact in a double
        let max = |v: &[Option<i64>]| v.iter().flatten().map(|x| x.unsigned_abs()).max().unwrap_or(0);
        let bound = u128::from(max(&a_vals))
            .saturating_mul(u128::from(max(&b_vals)))
            .saturating_mul(n as u128);
        if bound >= 1u128 << 53 {
            return Ok(None);
        }
        let Some(lut) = self.group_lut(fc, t, b)? else {
            return Ok(None);
        };
        let Some(e) = self.compute_with(Agg::Sum, a, &da, n, Some(p.clone()), &[(b, lut)])? else {
            return Ok(None);
        };
        let (sums, counts) = match &e.acc {
            Acc::Cur { sums, counts } | Acc::Int { sums, counts } => (sums, counts),
            _ => return Ok(None),
        };
        let mut total = 0i128;
        let mut any = false;
        for (code, (sa, k)) in sums.iter().zip(counts).enumerate() {
            if *k == 0 {
                continue;
            }
            let Some(Some(bv)) = b_vals.get(code).copied() else {
                // a blank `b` (or the blank row of unmatched keys): blank products
                continue;
            };
            any = true;
            total = total.saturating_add(sa.saturating_mul(i128::from(bv)));
        }
        if !any {
            return Ok(Some(V::Blank));
        }
        if a_is_cur || b_is_cur {
            return i64::try_from(total).map_or_else(|_| overflow(), |x| Ok(Some(V::Cur(x))));
        }
        Ok(Some(V::Num(total as f64)))
    }

    /// `SUMX(T, IF(cond, T[a]))` (or `IF(cond, T[a], BLANK())`) where `a` holds Currency or whole
    /// numbers and `cond` reads only columns of `T` and `RELATED` columns of few values: the sums
    /// of `a` grouped by the values `cond` reads (one grouped scan), `cond` evaluated once per
    /// combination met (on a row that has it), and the sums of the combinations where it holds
    /// added. The reference adds `a` on the rows where `cond` holds: the same rows, the same
    /// exact sums (Currency, or whole numbers whose partial sums stay exact in a double).
    fn conditional_sum(
        &mut self,
        body: &Expr,
        t: TabId,
        n: usize,
        p: &PredSet,
        s: &Scope,
    ) -> Result<Option<V>, Fail> {
        let Expr::Call { name, args, .. } = body else {
            return Ok(None);
        };
        if !name.eq_ignore_ascii_case("IF") || !rel_key(s.fc).is_empty() {
            return Ok(None);
        }
        let (cond, value) = match args.as_slice() {
            [Some(c), Some(v)] => (c, v),
            [Some(c), Some(v), Some(Expr::Call { name, args, .. })]
                if name.eq_ignore_ascii_case("BLANK") && args.is_empty() =>
            {
                (c, v)
            }
            _ => return Ok(None),
        };
        let Some(a) = self.own_column(value, t) else {
            return Ok(None);
        };
        let mut deps = Vec::new();
        if !self.row_deps(cond, t, s.env, &mut deps, 0)? || deps.is_empty() {
            return Ok(None);
        }
        let da = self.col(a)?;
        let (_, a_cur, a_int) = da.kinds();
        let vals = if a_cur && !a_int {
            self.cur_codes(&da)
        } else if a_int {
            self.int_codes(a, &da)?
        } else {
            None
        };
        let Some(vals) = vals else {
            return Ok(None);
        };
        let is_cur = a_cur && !a_int;
        // whole numbers: every partial sum exact in a double, as the reference adds them
        let max = vals.iter().flatten().map(|x| x.unsigned_abs()).max().unwrap_or(0);
        if !is_cur && u128::from(max).saturating_mul(n as u128) >= 1u128 << 53 {
            return Ok(None);
        }
        // the grouping: the values `cond` reads (a foreign key read through `RELATED` by the
        // related values), each as a lookup table from a column of `T`
        let parts = self.body_parts(s.fc, t, cond, s.env, &deps)?;
        let mut keys: Vec<(ColId, Arc<Lut>)> = Vec::with_capacity(parts.len());
        for (c, d, l) in &parts {
            let l = match l {
                Some(l) => Arc::clone(l),
                None => Arc::new(Lut {
                    col: *c,
                    map: None,
                    card: d.values.len(),
                }),
            };
            keys.push((l.col, l));
        }
        let groups = keys
            .iter()
            .try_fold(1usize, |g, (_, l)| g.checked_mul(l.card.max(1)));
        if groups.is_none_or(|g| g > 1 << 16) {
            return Ok(None);
        }
        // one row of each combination met, in the reference's order
        let reps: Vec<(u32, usize)> = {
            let key_parts: Vec<scan::KeyPart<'_>> = parts
                .iter()
                .map(|(_, d, l)| scan::KeyPart {
                    codes: d.codes.col(),
                    lut: l.as_ref().and_then(|l| l.map.as_deref()),
                    card: l.as_ref().map_or(d.values.len(), |l| l.card),
                })
                .collect();
            let Some(key) = scan::Key::new(key_parts) else {
                return Ok(None);
            };
            let sel = p.selection();
            let g = key.groups();
            let (_states, routed) =
                self.grouped_scan_charge("the first rows of combinations", sel.rows, g, 4)?;
            let Some(first) = scan::scan_groups_within(&sel, &key, None, 4, routed, scan::FirstRow::new)
            else {
                return internal();
            };
            let mut reps: Vec<(u32, usize)> = first
                .0
                .iter()
                .enumerate()
                .filter(|(_, r)| **r != u32::MAX)
                .map(|(g, r)| (*r, g))
                .collect();
            reps.sort_unstable();
            reps
        };
        let Some(e) = self.compute_with(Agg::Sum, a, &da, n, Some(p.clone()), &keys)? else {
            return Ok(None);
        };
        let rep_tab = Tab::rows_of(t, reps.iter().map(|(r, _)| *r).collect());
        let holds = self.each_with(&rep_tab, s, |eng, inner| eng.scalar(cond, inner))?;
        let (sums, counts) = match &e.acc {
            Acc::Cur { sums, counts } | Acc::Int { sums, counts } => (sums, counts),
            _ => return Ok(None),
        };
        let mut total = 0i128;
        let mut any = false;
        for ((_, g), h) in reps.iter().zip(&holds) {
            if !super::truth(h)? {
                continue;
            }
            if counts.get(*g).copied().unwrap_or(0) > 0 {
                any = true;
                total = total.saturating_add(sums.get(*g).copied().unwrap_or(0));
            }
        }
        if !any {
            return Ok(Some(V::Blank));
        }
        if is_cur {
            return i64::try_from(total).map_or_else(|_| overflow(), |x| Ok(Some(V::Cur(x))));
        }
        Ok(Some(V::Num(total as f64)))
    }

    /// An iterator's aggregation (`SUMX`…, as `iterate_agg`) of values each counted for some rows.
    fn weighted(&mut self, f: &str, vals: Vec<(V, u64)>) -> Result<V, Fail> {
        let present: Vec<(V, u64)> = vals.into_iter().filter(|(v, _)| !v.is_blank()).collect();
        let count: u64 = present.iter().map(|(_, n)| *n).fold(0, u64::saturating_add);
        Ok(match f {
            "COUNTX" | "COUNTAX" => {
                if count == 0 {
                    V::Blank
                } else {
                    V::Num(count as f64)
                }
            }
            "MINX" | "MAXX" => {
                let mut best: Option<V> = None;
                for (v, _) in present {
                    let better = match &best {
                        None => true,
                        Some(b) => {
                            let o = super::compare_vals(&v, b)?;
                            if f == "MINX" {
                                o == std::cmp::Ordering::Less
                            } else {
                                o == std::cmp::Ordering::Greater
                            }
                        }
                    };
                    if better {
                        best = Some(v);
                    }
                }
                best.unwrap_or(V::Blank)
            }
            _ if !present.is_empty() && present.iter().all(|(v, _)| matches!(v, V::Cur(_))) => {
                let total: i128 = present
                    .iter()
                    .map(|(v, n)| match v {
                        V::Cur(x) => i128::from(*x).saturating_mul(i128::from(*n)),
                        _ => 0,
                    })
                    .fold(0, i128::saturating_add);
                let r = if f == "AVERAGEX" {
                    currency::div_round(total, i128::from(count))
                } else {
                    i64::try_from(total).ok()
                };
                return r.map_or_else(overflow, |x| Ok(V::Cur(x)));
            }
            _ => {
                let (mut sum, mut comp) = (0.0f64, 0.0f64);
                let mut k = 0u64;
                for (v, n) in &present {
                    if let Some(x) = super::num(v)? {
                        scan::neumaier(&mut sum, &mut comp, x * *n as f64);
                        k = k.saturating_add(*n);
                    }
                }
                let sum = sum + comp;
                match (k, f) {
                    (0, _) => V::Blank,
                    (_, "AVERAGEX") => V::Num(sum / k as f64),
                    _ => V::Num(sum),
                }
            }
        })
    }

    /// `FILTER` over the rows of a model table whose predicate reads only some of its columns:
    /// the predicate once per combination of their codes. `None`: the reference path.
    pub(super) fn filter_by_codes(&mut self, t: &Tab, pred: &Expr, s: &Scope) -> Result<Option<Tab>, Fail> {
        let Tab::Rows {
            table,
            rows,
            ext: None,
        } = t
        else {
            return Ok(None);
        };
        if self.store.is_none() || rows.len() < 64 {
            return Ok(None);
        }
        let mut deps = Vec::new();
        if !self.row_deps(pred, *table, s.env, &mut deps, 0)? {
            return Ok(None);
        }
        let n = self.rows(*table)?;
        let datas = deps.iter().map(|c| self.col(*c)).collect::<Result<Vec<_>, _>>()?;
        let parts: Vec<scan::KeyPart<'_>> = datas
            .iter()
            .map(|d| scan::KeyPart {
                codes: d.codes.col(),
                lut: None,
                card: d.values.len(),
            })
            .collect();
        let Some(key) = scan::Key::new(parts) else {
            return Ok(None);
        };
        let groups = key.groups();
        if groups > (1 << 22) {
            return Ok(None);
        }
        self.tick(rows.len())?;
        let (threads, _sinks) = self.scan_threads(rows.len(), groups.saturating_mul(4), &move || {
            format!("the first rows of {groups} groups")
        })?;
        let real: Vec<u32> = rows.iter().copied().filter(|r| (*r as usize) < n).collect();
        let Some(first) = scan::scan_list(&real, &key, None, threads, || scan::FirstRow::new(groups)) else {
            return internal();
        };
        let mut reps: Vec<(u32, usize)> = first
            .0
            .iter()
            .enumerate()
            .filter(|(_, f)| **f != u32::MAX)
            .map(|(g, f)| (*f, g))
            .collect();
        reps.sort_unstable();
        let blank_listed = real.len() < rows.len();
        let mut rep_rows: Vec<u32> = reps.iter().map(|(r, _)| *r).collect();
        if blank_listed {
            rep_rows.push(u32::try_from(n).unwrap_or(u32::MAX));
        }
        let rep_tab = Tab::rows_of(*table, rep_rows);
        let vals = self.each_with(&rep_tab, s, |eng, inner| eng.scalar(pred, inner))?;
        let mut keep_group = vec![false; groups];
        for ((_, g), v) in reps.iter().zip(&vals) {
            if let Some(k) = keep_group.get_mut(*g) {
                *k = super::truth(v)?;
            }
        }
        let keep_blank = match (blank_listed, vals.get(reps.len())) {
            (true, Some(v)) => super::truth(v)?,
            _ => false,
        };
        // the blank row, when listed, is listed last (as ALL lists it): the others in parallel
        let blank_last = rows
            .iter()
            .position(|r| (*r as usize) >= n)
            .is_none_or(|i| i + 1 == rows.len());
        let kept: Vec<u32> = if blank_last {
            let threads = scan::threads_for(rows.len(), 0);
            let mut kept = scan::filter_list(&real, &key, &keep_group, threads);
            if blank_listed && keep_blank {
                kept.push(u32::try_from(n).unwrap_or(u32::MAX));
            }
            kept
        } else {
            rows.iter()
                .copied()
                .filter(|r| {
                    if (*r as usize) >= n {
                        keep_blank
                    } else {
                        keep_group.get(key.of_row(*r as usize)).copied().unwrap_or(false)
                    }
                })
                .collect()
        };
        Ok(Some(Tab::rows_of(*table, kept)))
    }

    /// An expression over the rows of a model table that reads only some of its columns
    /// ([`Engine::row_deps`]), evaluated once per distinct combination of their codes (in a row
    /// that has it) instead of once per row. `None`: the reference path.
    pub(super) fn each_by_codes(&mut self, t: &Tab, body: &Expr, s: &Scope) -> Result<Option<Vec<V>>, Fail> {
        let Tab::Rows {
            table,
            rows,
            ext: None,
        } = t
        else {
            return Ok(None);
        };
        if self.store.is_none() || rows.len() < 64 {
            return Ok(None);
        }
        let mut deps = Vec::new();
        if !self.row_deps(body, *table, s.env, &mut deps, 0)? {
            return Ok(None);
        }
        let n = self.rows(*table)?;
        let datas = deps.iter().map(|c| self.col(*c)).collect::<Result<Vec<_>, _>>()?;
        let parts: Vec<scan::KeyPart<'_>> = datas
            .iter()
            .map(|d| scan::KeyPart {
                codes: d.codes.col(),
                lut: None,
                card: d.values.len(),
            })
            .collect();
        let Some(key) = scan::Key::new(parts) else {
            return Ok(None);
        };
        if key.groups() > (1 << 22) {
            return Ok(None);
        }
        self.tick(rows.len())?;
        // slot of each group (its first row's result), NONE until met
        let mut slot = vec![NONE; key.groups()];
        let mut reps: Vec<u32> = Vec::new();
        let mut row_slot: Vec<u32> = Vec::with_capacity(rows.len());
        for r in rows.iter() {
            if *r as usize >= n {
                // the virtual blank row: its own slot
                row_slot.push(u32::try_from(reps.len()).unwrap_or(NONE));
                reps.push(*r);
                continue;
            }
            let g = key.of_row(*r as usize);
            let Some(s) = slot.get_mut(g) else {
                return internal();
            };
            if *s == NONE {
                *s = u32::try_from(reps.len()).unwrap_or(NONE);
                reps.push(*r);
            }
            row_slot.push(*s);
        }
        drop(key);
        let rep_tab = Tab::rows_of(*table, reps);
        let vals = self.each_with(&rep_tab, s, |eng, inner| eng.scalar(body, inner))?;
        let out: Option<Vec<V>> = row_slot.iter().map(|i| vals.get(*i as usize).cloned()).collect();
        match out {
            Some(v) => Ok(Some(v)),
            None => internal(),
        }
    }

    /// One grouped scan of `t` under `base` for the grouping columns of `keys`.
    #[allow(clippy::too_many_arguments)]
    fn compute(
        &mut self,
        agg: Agg,
        c: ColId,
        d: &ColData,
        t: TabId,
        n: usize,
        base: &FCtx,
        keys: &[(ColId, Arc<Lut>)],
    ) -> Result<Option<Entry>, Fail> {
        let p = self.pred(base, t)?;
        self.compute_with(agg, c, d, n, p, keys)
    }

    /// [`Engine::compute`] over the rows `p` selects (`None`: every row).
    fn compute_with(
        &mut self,
        agg: Agg,
        c: ColId,
        d: &ColData,
        n: usize,
        p: Option<PredSet>,
        keys: &[(ColId, Arc<Lut>)],
    ) -> Result<Option<Entry>, Fail> {
        // no filter, no grouping: what the column's values alone answer (every code of a column
        // is met by some row, but codes whose value is blank: their rows have code 0)
        if p.is_none() && keys.is_empty() {
            let blank_or = |k: usize| if k == 0 { V::Blank } else { V::Num(k as f64) };
            let v = match agg {
                Agg::CountRows => Some(blank_or(n)),
                Agg::Min | Agg::Max | Agg::Distinct | Agg::DistinctNoBlank
                    if d.codes.known_blank().is_some() =>
                {
                    let blank = self.has_blank_code(d)?;
                    let present = |v: &CellValue| !matches!(v, CellValue::Null);
                    Some(match agg {
                        Agg::Min | Agg::Max => {
                            // the first (last) value in sort order that some row holds
                            let ranks = self.rank_table(d);
                            let held = |code: &&u32| {
                                **code != 0 && d.values.get(**code as usize).is_some_and(present)
                            };
                            let code = if agg == Agg::Min {
                                ranks.1.iter().find(held)
                            } else {
                                ranks.1.iter().rev().find(held)
                            };
                            code.and_then(|k| d.values.get(*k as usize))
                                .map_or(V::Blank, V::of_cell)
                        }
                        _ => {
                            let dv = d.derived.get();
                            let held = *dv
                                .nonblank
                                .get_or_init(|| d.values.iter().skip(1).filter(|v| present(v)).count());
                            let n = held + usize::from(blank && agg == Agg::Distinct);
                            blank_or(n)
                        }
                    })
                }
                _ => None,
            };
            if let Some(v) = v {
                return Ok(Some(Entry {
                    cards: Vec::new(),
                    acc: Acc::Value(v),
                }));
            }
        }
        // no filter, no grouping: the rows per code of the whole column, kept with the column,
        // when they are already counted or the aggregation needs them (a scan is faster
        // otherwise)
        let needs_counts = match agg {
            Agg::Median | Agg::StdDev | Agg::Var | Agg::StdDevP | Agg::VarP => true,
            Agg::Sum | Agg::Avg => !d.kinds().0 && self.int_codes(c, d)?.is_none(),
            _ => false,
        };
        // (or the column is mostly runs of one value: counting is one step per run)
        let runs = || {
            d.codes
                .col_packed()
                .is_some_and(|pk| pk.run_rows().saturating_mul(4) >= pk.rows().saturating_mul(3))
        };
        if p.is_none()
            && keys.is_empty()
            && (needs_counts || d.derived.get().counts.get().is_some() || runs())
            && let Some(full) = self.full_counts(d)?
        {
            let v = self.finish(agg, c, d, &full)?;
            return Ok(Some(Entry {
                cards: Vec::new(),
                acc: Acc::Value(v),
            }));
        }
        // a report-derived aggregation that covers the scan exactly (`preview_eval_aggs.rs`)
        if let Some(e) = self.agg_entry(agg, c, d, n, p.as_ref(), keys)? {
            return Ok(Some(e));
        }
        let p = p.unwrap_or_else(|| PredSet::all(n));
        self.scan_entry(agg, c, d, n, p, keys)
    }

    /// The grouped scan of [`Engine::compute_with`] over the fact.
    fn scan_entry(
        &mut self,
        agg: Agg,
        c: ColId,
        d: &ColData,
        n: usize,
        p: PredSet,
        keys: &[(ColId, Arc<Lut>)],
    ) -> Result<Option<Entry>, Fail> {
        self.scan_entry_in(agg, c, d, n, &p, keys, None)
    }

    /// [`Engine::scan_entry`], over the groups `lo..lo + len` only when `range` is
    /// `Some((lo, len))` (`docs/beyond-ram.md`, "Groupings by ranges"): the entry then holds those
    /// groups, numbered from 0, each with the state the whole scan gives it. The kernel is chosen
    /// from the whole grouping, as without a range, so the values are the same: integer sums,
    /// counts, ranks, bitmaps and histograms do not depend on the order of the rows, and double
    /// sums go through the same morsels merged in the same order.
    #[allow(clippy::too_many_arguments)]
    fn scan_entry_in(
        &mut self,
        agg: Agg,
        c: ColId,
        d: &ColData,
        n: usize,
        p: &PredSet,
        keys: &[(ColId, Arc<Lut>)],
        range: Option<(usize, usize)>,
    ) -> Result<Option<Entry>, Fail> {
        let mut datas = Vec::with_capacity(keys.len());
        for (_, l) in keys {
            datas.push(self.col(l.col)?);
        }
        let parts: Vec<scan::KeyPart<'_>> = keys
            .iter()
            .zip(&datas)
            .map(|((_, l), dd)| scan::KeyPart {
                codes: dd.codes.col(),
                lut: l.map.as_deref(),
                card: l.card,
            })
            .collect();
        let Some(key) = scan::Key::new(parts) else {
            return Ok(None);
        };
        let cards: Vec<usize> = keys.iter().map(|(_, l)| l.card).collect();
        // the kernels are chosen from the whole grouping; the states hold `groups` groups
        let whole = key.groups();
        let (lo, groups) = match range {
            Some((lo, len)) => (lo, len.min(whole.saturating_sub(lo))),
            None => (0, whole),
        };
        let card = d.values.len();
        // per-code value tables first (they borrow `self`)
        let ranks = match agg {
            Agg::Min | Agg::Max => Some(self.ranks(c)?),
            _ => None,
        };
        let sums = match agg {
            Agg::Sum | Agg::Avg => {
                if d.kinds().0 {
                    match self.cur_codes(d) {
                        Some(v) => Some((true, v)),
                        None => return Ok(None),
                    }
                } else {
                    self.int_codes(c, d)?.map(|v| (false, v))
                }
            }
            _ => None,
        };
        let floats = match (agg, &sums) {
            (Agg::Sum | Agg::Avg, None) => Some(self.nums(c)?),
            _ => None,
        };
        // each branch reserves its scan's states (and routed rows) against the query's cap
        let label = if self.mem.on() {
            format!("a {agg:?}")
        } else {
            String::new()
        };
        self.tick(n)?;
        let sel = p.selection();
        let values = Some(d.codes.col());
        let failed = || Fail::Unsupported("a storage scan failed".into());
        // by ranges: the workers' states held to a quarter of what the query may still use
        let max_states = match range {
            Some(_) => (self.mem.room() / 4).max(1),
            None => usize::MAX,
        };
        // one grouped scan: every group, or those of the range
        macro_rules! grouped {
            ($values:expr, $gb:expr, $routed:expr, $make:expr) => {
                match range {
                    None => scan::scan_groups_within(&sel, &key, $values, $gb, $routed, $make),
                    Some(_) => scan::scan_groups_range(
                        &sel, &key, $values, $gb, $routed, lo, groups, max_states, $make,
                    ),
                }
            };
        }
        let acc = match agg {
            Agg::CountRows => {
                let (_states, routed) = self.grouped_scan_charge_in(&label, n, groups, 8, max_states)?;
                Acc::Rows(grouped!(None, 8, routed, scan::Counts::new).ok_or_else(failed)?.0)
            }
            Agg::Count | Agg::CountBlank => {
                let (_states, routed) = self.grouped_scan_charge_in(&label, n, groups, 16, max_states)?;
                Acc::Blanks(grouped!(values, 16, routed, scan::BlankCounts::new).ok_or_else(failed)?)
            }
            Agg::Distinct | Agg::DistinctNoBlank
                if groups
                    .checked_mul(card.max(1).div_ceil(64))
                    .and_then(|w| w.checked_mul(8))
                    .is_none_or(|b| b > dense_distinct_limit()) =>
            {
                // too many groups for a bitmap each: the (group, code) pairs met, sorted
                let _pairs = self.charge_bytes(
                    &move || format!("the distinct values of {groups} groups"),
                    n.saturating_mul(16),
                )?;
                let (_states, routed) = self.grouped_scan_charge(&label, n, groups, 8)?;
                let pairs = scan::scan_groups_within(&sel, &key, values, 8, routed, |len| {
                    scan::DistinctPairs::new(len, card)
                })
                .ok_or_else(failed)?;
                Acc::Sparse(pairs.finish())
            }
            Agg::Distinct | Agg::DistinctNoBlank => {
                // (the bitmaps of every group: within the cap of one cached aggregation)
                let words = card.max(1).div_ceil(64);
                if whole
                    .checked_mul(words)
                    .and_then(|w| w.checked_mul(8))
                    .is_none_or(|b| b > MAX_ENTRY_BYTES)
                {
                    return Ok(None);
                }
                let (_states, routed) =
                    self.grouped_scan_charge_in(&label, n, groups, words.saturating_mul(8), max_states)?;
                Acc::Distinct(
                    grouped!(values, words.saturating_mul(8), routed, |len| {
                        scan::Distinct::new(len, card, usize::MAX)
                            .unwrap_or_else(|| scan::Distinct::empty(card))
                    })
                    .ok_or_else(failed)?,
                )
            }
            Agg::Min | Agg::Max => {
                let Some(ranks) = ranks else {
                    return internal();
                };
                let (_states, routed) = self.grouped_scan_charge_in(&label, n, groups, 9, max_states)?;
                let r =
                    grouped!(values, 9, routed, |len| scan::Ranks::new(len, &ranks)).ok_or_else(failed)?;
                Acc::Ranks {
                    min: r.min,
                    max: r.max,
                    seen: r.seen,
                }
            }
            Agg::Sum | Agg::Avg => {
                // the id-keyed paths below need whole rows and at most 65,536 groups
                let by_id = range.is_none() && groups <= (1 << 16);
                let affine = if by_id { d.codes.affine() } else { None };
                let small_lut = if by_id {
                    d.codes.col_packed().filter(|p| p.lut().len() <= (1 << 20))
                } else {
                    None
                };
                match (&sums, &floats, affine, small_lut) {
                    // a value-encoded column: sums of its data ids, no lookup per row
                    (Some((is_cur, _)), _, Some((packed, a, b)), _) => {
                        let threads = scan::threads_for(n, groups.saturating_mul(24));
                        let _states = self.charge_bytes(
                            &move || format!("the grouped sums of {groups} groups"),
                            sum_states(threads, groups),
                        )?;
                        let first = bicraft_vertipaq::FIRST_DATA_ID;
                        let (ids, counts) =
                            scan::sum_by_id(&sel, &key, packed, scan::IdValues::Affine { first }, threads)
                                .ok_or_else(failed)?;
                        let sums: Vec<i128> = ids
                            .iter()
                            .zip(&counts)
                            .map(|(s, n)| {
                                i128::from(a)
                                    .saturating_mul(i128::from(*n))
                                    .saturating_add(i128::from(b).saturating_mul(*s))
                            })
                            .collect();
                        if *is_cur {
                            Acc::Cur { sums, counts }
                        } else {
                            Acc::Int { sums, counts }
                        }
                    }
                    // a compressed column with a small dictionary: its values by data id
                    (Some((is_cur, per_code)), _, _, Some(packed)) => {
                        let mut values = Vec::with_capacity(packed.lut().len());
                        let mut valid = Vec::with_capacity(packed.lut().len());
                        for code in packed.lut() {
                            let v = per_code.get(*code as usize).copied().flatten();
                            values.push(v.unwrap_or(0));
                            valid.push(v.is_some());
                        }
                        let threads = scan::threads_for(n, groups.saturating_mul(24));
                        let _states = self.charge_bytes(
                            &move || format!("the grouped sums of {groups} groups"),
                            sum_states(threads, groups),
                        )?;
                        let (sums, counts) = scan::sum_by_id(
                            &sel,
                            &key,
                            packed,
                            scan::IdValues::Table {
                                values: &values,
                                valid: &valid,
                            },
                            threads,
                        )
                        .ok_or_else(failed)?;
                        if *is_cur {
                            Acc::Cur { sums, counts }
                        } else {
                            Acc::Int { sums, counts }
                        }
                    }
                    (Some((is_cur, per_code)), _, _, _) => {
                        let (_states, routed) =
                            self.grouped_scan_charge_in(&label, n, groups, 24, max_states)?;
                        let r = grouped!(values, 24, routed, |len| scan::IntSums::new(len, per_code))
                            .ok_or_else(failed)?;
                        if *is_cur {
                            Acc::Cur {
                                sums: r.sums,
                                counts: r.counts,
                            }
                        } else {
                            Acc::Int {
                                sums: r.sums,
                                counts: r.counts,
                            }
                        }
                    }
                    // (a histogram of every group by code when it fits, else ordered double sums)
                    (None, Some(nums), _, _) => match hist_card(whole, card) {
                        Some(hc) => {
                            let (_states, routed) = self.grouped_scan_charge_in(
                                &label,
                                n,
                                groups,
                                hc.saturating_mul(8),
                                max_states,
                            )?;
                            Acc::Hist(
                                grouped!(values, hc.saturating_mul(8), routed, |len| {
                                    scan::Histogram::new(len, hc, usize::MAX)
                                        .unwrap_or_else(|| scan::Histogram::empty(hc))
                                })
                                .ok_or_else(failed)?,
                            )
                        }
                        None => {
                            // sums of doubles depend on their order: one sink per morsel, merged in
                            // row order, the same bits on any number of threads (and by ranges)
                            if whole.saturating_mul(24).saturating_mul(scan::morsel_count(n))
                                > MAX_ENTRY_BYTES
                            {
                                return Ok(None);
                            }
                            let _ordered = self.charge_bytes(
                                &move || format!("the ordered sums of {groups} groups"),
                                groups.saturating_mul(24).saturating_mul(scan::morsel_count(n)),
                            )?;
                            let threads = scan::threads_for(n, groups.saturating_mul(24));
                            let make = || scan::FloatSums::new(groups, nums);
                            let r = match range {
                                None => scan::scan_ordered(&sel, &key, values, threads, make),
                                Some(_) => {
                                    scan::scan_ordered_range(&sel, &key, values, threads, lo, groups, make)
                                }
                            }
                            .ok_or_else(failed)?;
                            let sums = (0..groups).map(|g| r.total(g)).collect();
                            Acc::Float {
                                sums,
                                counts: r.counts,
                            }
                        }
                    },
                    _ => return internal(),
                }
            }
            Agg::Median | Agg::StdDev | Agg::Var | Agg::StdDevP | Agg::VarP => {
                let Some(hc) = hist_card(whole, card) else {
                    return Ok(None);
                };
                let (_states, routed) =
                    self.grouped_scan_charge_in(&label, n, groups, hc.saturating_mul(8), max_states)?;
                Acc::Hist(
                    grouped!(values, hc.saturating_mul(8), routed, |len| {
                        scan::Histogram::new(len, hc, usize::MAX)
                            .unwrap_or_else(|| scan::Histogram::empty(hc))
                    })
                    .ok_or_else(failed)?,
                )
            }
        };
        Ok(Some(Entry { cards, acc }))
    }

    /// The aggregation of group `g` of a cached scan, as [`Engine::finish`] computes it.
    fn value_of(&mut self, agg: Agg, c: ColId, d: &ColData, e: &Entry, g: usize) -> Result<V, Fail> {
        let blank_or = |n: u64| if n == 0 { V::Blank } else { V::Num(n as f64) };
        let at = |v: &[u64]| v.get(g).copied().unwrap_or(0);
        Ok(match &e.acc {
            Acc::Rows(v) => blank_or(at(v)),
            Acc::Blanks(b) => match agg {
                Agg::CountBlank => blank_or(at(&b.blank)),
                _ => blank_or(at(&b.nonblank)),
            },
            Acc::Sparse(s) => {
                let (n, blank) = s.count(g);
                match agg {
                    Agg::DistinctNoBlank => blank_or(n.saturating_sub(u64::from(blank))),
                    _ => blank_or(n),
                }
            }
            Acc::Distinct(s) => {
                let (n, blank) = s.count(g);
                match agg {
                    Agg::DistinctNoBlank => blank_or(n.saturating_sub(u64::from(blank))),
                    _ => blank_or(n),
                }
            }
            Acc::Ranks { min, max, seen } => {
                if !seen.get(g).copied().unwrap_or(false) {
                    return Ok(V::Blank);
                }
                let r = if agg == Agg::Min { min.get(g) } else { max.get(g) };
                let ranks = self.rank_table(d);
                let code = r.and_then(|r| ranks.1.get(*r as usize)).copied().unwrap_or(0);
                d.values.get(code as usize).map_or(V::Blank, V::of_cell)
            }
            Acc::Cur { sums, counts } => {
                let count = at(counts);
                if count == 0 {
                    return Ok(V::Blank);
                }
                let total = sums.get(g).copied().unwrap_or(0);
                let r = match agg {
                    Agg::Avg => currency::div_round(total, i128::from(count)),
                    _ => i64::try_from(total).ok(),
                };
                return r.map_or_else(overflow, |x| Ok(V::Cur(x)));
            }
            Acc::Int { sums, counts } => {
                let count = at(counts);
                if count == 0 {
                    return Ok(V::Blank);
                }
                let total = sums.get(g).copied().unwrap_or(0) as f64;
                match agg {
                    Agg::Avg => V::Num(total / count as f64),
                    _ => V::Num(total),
                }
            }
            Acc::Float { sums, counts } => {
                let count = at(counts);
                if count == 0 {
                    return Ok(V::Blank);
                }
                let total = sums.get(g).copied().unwrap_or(0.0);
                match agg {
                    Agg::Avg => V::Num(total / count as f64),
                    _ => V::Num(total),
                }
            }
            Acc::Hist(h) => {
                let counts = h.group(g).to_vec();
                return self.finish(agg, c, d, &counts);
            }
            Acc::Value(v) => v.clone(),
        })
    }
}

fn entry_bytes(e: &Entry) -> usize {
    match &e.acc {
        Acc::Rows(v) => v.len() * 8,
        Acc::Blanks(b) => b.blank.len() * 16,
        Acc::Distinct(s) => s.bits.len() * 8,
        Acc::Sparse(s) => s.bytes(),
        Acc::Ranks { min, .. } => min.len() * 9,
        Acc::Cur { sums, .. } | Acc::Int { sums, .. } => sums.len() * 24,
        Acc::Float { sums, .. } => sums.len() * 16,
        Acc::Hist(h) => h.counts.len() * 8,
        Acc::Value(_) => 0,
    }
}
