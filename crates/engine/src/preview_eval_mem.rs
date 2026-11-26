//! One query's formula-engine memory (`docs/beyond-ram.md`, lot 4): the tables, groupings, value
//! vectors, scan states, sort keys and answer rows an evaluation builds are counted against the
//! query's cap, by bytes, so that a heavy query is refused cleanly instead of exhausting memory.
//!
//! - Without a cap (no memory budget, no query budget: the default) nothing here does any work.
//! - **Reserved before allocating**: where the size of a big allocation is known before it is
//!   made (a grouping's combinations, the value vectors of a `SUMMARIZECOLUMNS`, a scan's per-thread
//!   states, a row mask, the rows of a materialized table, `ORDER BY` keys, the answer's rows), a
//!   [`Charge`] is taken first; over the cap the query fails with an error naming the part and the
//!   cap, before the allocation.
//! - **Counted when built**: every table value of more than [`MIN_HELD`] bytes an expression
//!   returns is held by the meter (by address, through a weak reference) as soon as it is built,
//!   and released when its last reference goes. Nothing big is allocated between building it and
//!   counting it, so the next reservation sees it.
//! - A refused query leaves nothing behind: charges are released when they drop (the engine
//!   drops every one with the evaluation).

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Weak};

use super::super::Fail;
use super::super::store::Miss;
use super::{Ext, Tab, V};

/// Tables smaller than this are not counted one by one (their number is bounded by the
/// expression's nesting and the evaluation's own structures).
pub(super) const MIN_HELD: usize = 256 << 10;

#[cfg(test)]
thread_local! {
    /// Tests: the smallest table counted on this thread (instead of [`MIN_HELD`]).
    pub(super) static TEST_MIN_HELD: Cell<Option<usize>> = const { Cell::new(None) };
}

fn min_held() -> usize {
    #[cfg(test)]
    if let Some(m) = TEST_MIN_HELD.with(Cell::get) {
        return m;
    }
    MIN_HELD
}

/// Bytes of one row of `width` values in a table of tuples: the row's vector, its values and the
/// allocator's overhead (text counted apart).
pub(super) fn row_bytes(width: usize) -> usize {
    std::mem::size_of::<Vec<V>>()
        .saturating_add(width.saturating_mul(std::mem::size_of::<V>()))
        .saturating_add(16)
}

/// Bytes of the text one value holds.
pub(super) fn text_bytes(v: &V) -> usize {
    match v {
        V::Text(t) => t.capacity(),
        _ => 0,
    }
}

/// Bytes a table value holds (its rows; the columns' descriptions are small).
pub(super) fn tab_bytes(t: &Tab) -> usize {
    match t {
        Tab::Rows { rows, ext, .. } => {
            let mut b = rows.capacity().saturating_mul(4);
            if let Some(e) = ext {
                b = b.saturating_add(tuples_bytes(&e.vals, e.cols.len()));
            }
            b
        }
        Tab::Tuples { cols, rows } => tuples_bytes(rows, cols.len()),
    }
}

/// Bytes of rows of values: their vectors and values (estimated from the width), plus their text.
pub(super) fn tuples_bytes(rows: &[Vec<V>], width: usize) -> usize {
    let fixed = rows.len().saturating_mul(row_bytes(width));
    // text: every row of a small table, else one row in `step`, scaled up
    let text: usize = if rows.len() <= 4096 {
        rows.iter().flatten().map(text_bytes).sum()
    } else {
        let step = (rows.len() / 1024).max(1);
        let (mut sampled, mut n) = (0usize, 0usize);
        for r in rows.iter().step_by(step) {
            sampled = sampled.saturating_add(r.iter().map(text_bytes).sum::<usize>());
            n = n.saturating_add(1);
        }
        sampled
            .checked_div(n.max(1))
            .unwrap_or(0)
            .saturating_add(1)
            .saturating_mul(rows.len())
    };
    fixed.saturating_add(text)
}

/// Memory reserved by a query for something it builds; released when dropped.
#[derive(Default)]
pub(super) struct Charge {
    used: Option<Rc<Cell<usize>>>,
    bytes: usize,
    /// The same bytes in the process' pool, under a memory limit (lot 5).
    _res: bicraft_mem::Reservation,
}

impl std::fmt::Debug for Charge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Charge").field("bytes", &self.bytes).finish()
    }
}

impl Charge {
    #[cfg(test)]
    pub(super) fn bytes(&self) -> usize {
        self.bytes
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        if let Some(u) = &self.used {
            u.set(u.get().saturating_sub(self.bytes));
        }
    }
}

/// What a held table is: kept alive by others, counted here while it lives.
enum Alive {
    Rows(Weak<Vec<u32>>),
    Tuples(Weak<Vec<Vec<V>>>),
    Mask(Weak<Vec<bool>>),
    Ext(Weak<Ext>),
}

impl Alive {
    fn alive(&self) -> bool {
        match self {
            Alive::Rows(w) => w.strong_count() > 0,
            Alive::Tuples(w) => w.strong_count() > 0,
            Alive::Mask(w) => w.strong_count() > 0,
            Alive::Ext(w) => w.strong_count() > 0,
        }
    }
}

/// The meter of one evaluation.
#[derive(Default)]
pub(super) struct QueryMem {
    /// The cap in bytes; `None`: no cap of its own.
    cap: Option<usize>,
    /// The pool of the process' memory limit, when there is one (lot 5): every charge is also
    /// reserved there, within the query's fair share.
    pool: Option<&'static bicraft_mem::Pool>,
    /// What the charges are in the pool (a background job's: background work).
    class: Option<bicraft_mem::Class>,
    /// The query counts among the running ones while it lives (fair shares).
    _ticket: Option<bicraft_mem::QueryTicket>,
    used: Rc<Cell<usize>>,
    peak: usize,
    /// Tables counted by address, each with its charge.
    held: HashMap<usize, (Alive, Charge)>,
    /// Charges kept until the end of the evaluation (the answer's rows).
    kept: Vec<Charge>,
    /// Refusals of this evaluation.
    pub(super) refused: u64,
}

/// A size for a message.
fn mb(bytes: usize) -> String {
    super::super::memory::size_text(bytes)
}

impl QueryMem {
    /// A meter under `cap`, its charges also reserved in `pool` when it has a limit (`background`:
    /// as background work, which does not count among the running queries).
    pub(super) fn new(cap: Option<usize>, pool: &'static bicraft_mem::Pool, background: bool) -> QueryMem {
        let limited = pool.limit().is_some();
        QueryMem {
            cap,
            pool: limited.then_some(pool),
            class: Some(if background {
                bicraft_mem::Class::Background
            } else {
                bicraft_mem::Class::Query
            }),
            _ticket: (limited && !background).then(|| pool.enter_query()),
            ..QueryMem::default()
        }
    }

    /// Whether anything is counted.
    pub(super) fn on(&self) -> bool {
        self.cap.is_some() || self.pool.is_some()
    }

    #[cfg(test)]
    pub(super) fn used(&self) -> usize {
        self.used.get()
    }

    pub(super) fn peak(&self) -> usize {
        self.peak
    }

    /// Bytes still free under the cap and the pool's limit (`usize::MAX` without either).
    pub(super) fn room(&mut self) -> usize {
        if !self.on() {
            return usize::MAX;
        }
        self.prune();
        let mut room = self.cap.map_or(usize::MAX, |c| c.saturating_sub(self.used.get()));
        if let Some(p) = self.pool
            && let Some(limit) = p.limit()
        {
            let s = p.stats();
            let mut free = limit.saturating_sub(s.used);
            if s.active_queries > 1 {
                // within its fair share (what the kept data and the others leave, divided)
                let others = s
                    .class(bicraft_mem::Class::Kept)
                    .saturating_add(s.class(bicraft_mem::Class::Background))
                    .saturating_add(s.class(bicraft_mem::Class::Io));
                let share = limit.saturating_sub(others) / s.active_queries;
                free = free.min(share.saturating_sub(self.used.get()));
            }
            room = room.min(free);
        }
        room
    }

    /// Keeps a charge until the evaluation ends.
    pub(super) fn keep(&mut self, c: Charge) {
        self.kept.push(c);
    }

    /// Forgets the tables nobody holds any more.
    fn prune(&mut self) {
        self.held.retain(|_, (a, _)| a.alive());
    }

    /// Reserves `bytes` for `what`; over the cap or the process' limit (after `local` freed what
    /// it could): the error naming both.
    pub(super) fn charge(
        &mut self,
        what: &dyn Fn() -> String,
        bytes: usize,
        local: &mut dyn FnMut(usize) -> usize,
    ) -> Result<Charge, Fail> {
        if !self.on() || bytes == 0 {
            return Ok(Charge::default());
        }
        if let Some(cap) = self.cap
            && self.used.get().saturating_add(bytes) > cap
        {
            self.prune();
        }
        let used = self.used.get();
        let total = used.saturating_add(bytes);
        if let Some(cap) = self.cap
            && total > cap
        {
            self.refused = self.refused.saturating_add(1);
            return Err(Fail::Miss(Miss::Error(format!(
                "the query needs more memory than its cap: {} needs {}{}, over the query's cap of {} \
                 (dax.memory query_budget, BICRAFT_QUERY_BUDGET, or the memory budget)",
                what(),
                mb(bytes),
                held_text(used),
                mb(cap)
            ))));
        }
        let mut res = bicraft_mem::Reservation::default();
        if let (Some(pool), Some(class)) = (self.pool, self.class) {
            res = pool.empty(class);
            let owner = (class == bicraft_mem::Class::Query).then_some(total);
            if let Err(e) = res.grow(bytes, owner, local) {
                self.prune();
                self.refused = self.refused.saturating_add(1);
                return Err(Fail::Miss(Miss::Error(format!(
                    "the query needs more memory than the process allows: {} needs {}{}, {e}",
                    what(),
                    mb(bytes),
                    held_text(used),
                ))));
            }
        }
        self.used.set(total);
        self.peak = self.peak.max(total);
        Ok(Charge {
            used: Some(Rc::clone(&self.used)),
            bytes,
            _res: res,
        })
    }

    /// Counts a table value an expression built (`what` names it), while it lives.
    pub(super) fn hold(
        &mut self,
        t: &Tab,
        what: &dyn Fn() -> String,
        local: &mut dyn FnMut(usize) -> usize,
    ) -> Result<(), Fail> {
        if !self.on() {
            return Ok(());
        }
        let (addr, alive) = match t {
            Tab::Rows { rows, ext: None, .. } => {
                (Arc::as_ptr(rows) as usize, Alive::Rows(Arc::downgrade(rows)))
            }
            // the added columns are the bulk: counted by the extension, with the rows it extends
            Tab::Rows { ext: Some(e), .. } => (Arc::as_ptr(e) as usize, Alive::Ext(Arc::downgrade(e))),
            Tab::Tuples { rows, .. } => (Arc::as_ptr(rows) as usize, Alive::Tuples(Arc::downgrade(rows))),
        };
        self.hold_alive(addr, alive, tab_bytes(t), what, local)
    }

    /// Counts rows of values (a materialized table) while they live.
    pub(super) fn hold_tuples(
        &mut self,
        rows: &Arc<Vec<Vec<V>>>,
        width: usize,
        what: &dyn Fn() -> String,
        local: &mut dyn FnMut(usize) -> usize,
    ) -> Result<(), Fail> {
        if !self.on() {
            return Ok(());
        }
        let bytes = tuples_bytes(rows, width);
        self.hold_alive(
            Arc::as_ptr(rows) as usize,
            Alive::Tuples(Arc::downgrade(rows)),
            bytes,
            what,
            local,
        )
    }

    /// Counts a row mask while it lives.
    pub(super) fn hold_mask(
        &mut self,
        m: &Arc<Vec<bool>>,
        what: &dyn Fn() -> String,
        local: &mut dyn FnMut(usize) -> usize,
    ) -> Result<(), Fail> {
        if !self.on() {
            return Ok(());
        }
        self.hold_alive(
            Arc::as_ptr(m) as usize,
            Alive::Mask(Arc::downgrade(m)),
            m.capacity(),
            what,
            local,
        )
    }

    fn hold_alive(
        &mut self,
        addr: usize,
        alive: Alive,
        bytes: usize,
        what: &dyn Fn() -> String,
        local: &mut dyn FnMut(usize) -> usize,
    ) -> Result<(), Fail> {
        if bytes < min_held() {
            return Ok(());
        }
        if let Some((a, _)) = self.held.get(&addr)
            && a.alive()
        {
            return Ok(());
        }
        self.held.remove(&addr);
        self.prune();
        let c = self.charge(what, bytes, local)?;
        self.held.insert(addr, (alive, c));
        Ok(())
    }
}

/// " with N already held by its other tables", or nothing.
fn held_text(used: usize) -> String {
    if used > 0 {
        format!(" with {} already held by its other tables", mb(used))
    } else {
        String::new()
    }
}

/// Frees what the evaluation's session keeps, for a reservation that did not fit under the
/// memory limit: the storage caches the evaluation holds, then what the session keeps (its least
/// recently used columns no query holds, its segment cache). Returns the bytes released.
fn evict_own(store: &mut Option<super::storage::Store>, st: &mut super::super::State, want: usize) -> usize {
    let mut freed = 0usize;
    if let Some(s) = store {
        freed = s.drop_caches();
    }
    if freed < want {
        freed = freed.saturating_add(st.evict_kept(want.saturating_sub(freed)));
    }
    freed
}

impl super::Engine<'_, '_> {
    /// Reserves `bytes` of this evaluation's memory for `what` (see the module's notes).
    pub(super) fn charge_bytes(&mut self, what: &dyn Fn() -> String, bytes: usize) -> Result<Charge, Fail> {
        if !self.mem.on() {
            return Ok(Charge::default());
        }
        let store = &mut self.store;
        let st = &mut *self.ctx.st;
        self.mem
            .charge(what, bytes, &mut |want| evict_own(store, st, want))
    }

    /// Reserves `bytes` for `what` until the evaluation ends (results kept in the storage cache:
    /// counted with the query, then with what the session keeps).
    pub(super) fn keep_charge(&mut self, what: &dyn Fn() -> String, bytes: usize) -> Result<(), Fail> {
        if !self.mem.on() {
            return Ok(());
        }
        let c = self.charge_bytes(what, bytes)?;
        self.mem.keep(c);
        Ok(())
    }

    /// [`Engine::keep_charge`] that does not fail the query: `false` when the bytes do not fit (the
    /// caller then drops what they are for).
    pub(super) fn keep_soft(&mut self, what: &dyn Fn() -> String, bytes: usize) -> bool {
        if !self.mem.on() {
            return true;
        }
        let refused = self.mem.refused;
        match self.charge_bytes(what, bytes) {
            Ok(c) => {
                self.mem.keep(c);
                true
            }
            Err(_) => {
                self.mem.refused = refused;
                false
            }
        }
    }

    /// Counts a table an expression built while it lives ([`QueryMem::hold`]).
    pub(super) fn hold_tab(&mut self, t: &Tab, what: &dyn Fn() -> String) -> Result<(), Fail> {
        if !self.mem.on() {
            return Ok(());
        }
        let store = &mut self.store;
        let st = &mut *self.ctx.st;
        self.mem.hold(t, what, &mut |want| evict_own(store, st, want))
    }

    /// Counts materialized rows while they live ([`QueryMem::hold_tuples`]).
    pub(super) fn hold_rows(
        &mut self,
        rows: &Arc<Vec<Vec<V>>>,
        width: usize,
        what: &dyn Fn() -> String,
    ) -> Result<(), Fail> {
        if !self.mem.on() {
            return Ok(());
        }
        let store = &mut self.store;
        let st = &mut *self.ctx.st;
        self.mem
            .hold_tuples(rows, width, what, &mut |want| evict_own(store, st, want))
    }

    /// Counts a row mask while it lives ([`QueryMem::hold_mask`]).
    pub(super) fn hold_mask(&mut self, m: &Arc<Vec<bool>>, what: &dyn Fn() -> String) -> Result<(), Fail> {
        if !self.mem.on() {
            return Ok(());
        }
        let store = &mut self.store;
        let st = &mut *self.ctx.st;
        self.mem
            .hold_mask(m, what, &mut |want| evict_own(store, st, want))
    }

    /// Reserves the states of a grouped scan (`what` over `rows` rows into `groups` groups of
    /// `group_bytes`) and returns the most bytes its routed rows may hold: three quarters of what
    /// the query's cap leaves beside the states (no cap: no bound), so that a high-cardinality scan routes its rows by
    /// windows instead of all at once (`bicraft_scan::scan_groups_within`).
    pub(super) fn grouped_scan_charge(
        &mut self,
        what: &str,
        rows: usize,
        groups: usize,
        group_bytes: usize,
    ) -> Result<(Charge, usize), Fail> {
        self.grouped_scan_charge_in(what, rows, groups, group_bytes, usize::MAX)
    }

    /// [`Engine::grouped_scan_charge`] for a scan whose workers' states are held to `max_states`
    /// bytes together (`bicraft_scan::scan_groups_range`).
    pub(super) fn grouped_scan_charge_in(
        &mut self,
        what: &str,
        rows: usize,
        groups: usize,
        group_bytes: usize,
        max_states: usize,
    ) -> Result<(Charge, usize), Fail> {
        if !self.mem.on() {
            return Ok((Charge::default(), usize::MAX));
        }
        // the routed rows take what the states (and their join) leave, with some margin
        let states = groups.saturating_mul(group_bytes).saturating_mul(2);
        let routed = (self.mem.room().saturating_sub(states) / 4)
            .saturating_mul(3)
            .max(1 << 20);
        let c = self.charge_bytes(
            &|| format!("the grouped scan of {what} ({groups} groups)"),
            bicraft_scan::scan_groups_range_bytes(rows, groups, group_bytes, routed, max_states),
        )?;
        Ok((c, routed))
    }

    /// The threads of a scan of `rows` rows whose workers each hold `sink_bytes` of state, fewer
    /// when their states would not fit in what the query's cap leaves, with the reservation of
    /// those states (the first worker's becomes the result).
    pub(super) fn scan_threads(
        &mut self,
        rows: usize,
        sink_bytes: usize,
        what: &dyn Fn() -> String,
    ) -> Result<(usize, Charge), Fail> {
        let threads = bicraft_scan::threads_for(rows, sink_bytes);
        if !self.mem.on() || sink_bytes == 0 {
            return Ok((threads, Charge::default()));
        }
        let fit = (self.mem.room() / sink_bytes).max(1);
        let threads = threads.min(fit).max(1);
        let c = self.charge_bytes(what, sink_bytes.saturating_mul(threads))?;
        Ok((threads, c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(s: &'static str) -> impl Fn() -> String {
        move || s.to_string()
    }

    /// A pool of its own, without a limit (the process' may have one in other tests).
    fn free_pool() -> &'static bicraft_mem::Pool {
        Box::leak(Box::new(bicraft_mem::Pool::new()))
    }

    #[test]
    fn charges_go_to_the_pool_under_a_limit_and_refusals_say_so() {
        let pool = free_pool();
        pool.set_limit(Some(10_000));
        let mut m = QueryMem::new(None, pool, false);
        assert!(m.on());
        assert_eq!(pool.stats().active_queries, 1);
        let a = m.charge(&named("a"), 6_000, &mut |_| 0).unwrap();
        assert_eq!(pool.stats().class(bicraft_mem::Class::Query), 6_000);
        assert_eq!(m.room(), 4_000);
        let mut asked = 0;
        let e = m.charge(&named("the scan"), 5_000, &mut |w| {
            asked = w;
            0
        });
        let Err(Fail::Miss(Miss::Error(t))) = e else {
            panic!("refused")
        };
        assert!(
            t.contains("more memory than the process allows: the scan needs 5000 bytes"),
            "{t}"
        );
        assert!(t.contains("memory limit of 10000 bytes"), "{t}");
        assert_eq!(asked, 1_000, "the session was asked for what was missing");
        drop(a);
        drop(m);
        let s = pool.stats();
        assert_eq!((s.used, s.active_queries), (0, 0));
        // background work: no ticket, its own class
        let mut b = QueryMem::new(None, pool, true);
        let c = b.charge(&named("b"), 100, &mut |_| 0).unwrap();
        assert_eq!(pool.stats().class(bicraft_mem::Class::Background), 100);
        assert_eq!(pool.stats().active_queries, 0);
        drop(c);
        assert_eq!(pool.stats().used, 0);
    }

    #[test]
    fn charges_are_released_when_dropped_and_refusals_name_the_part() {
        let mut m = QueryMem::new(Some(1000), free_pool(), false);
        let a = m.charge(&named("a"), 600, &mut |_| 0).unwrap();
        assert_eq!(m.used(), 600);
        let e = m.charge(&named("the grouping"), 500, &mut |_| 0).unwrap_err();
        let Fail::Miss(Miss::Error(t)) = e else {
            panic!("an error")
        };
        assert!(t.contains("the grouping needs 500 bytes"), "{t}");
        assert!(t.contains("600 bytes already held"), "{t}");
        assert!(t.contains("cap of 1000 bytes"), "{t}");
        assert_eq!(m.refused, 1);
        drop(a);
        assert_eq!(m.used(), 0);
        let b = m.charge(&named("b"), 1000, &mut |_| 0).unwrap();
        assert_eq!(m.peak(), 1000);
        drop(b);
        assert_eq!(m.used(), 0);
    }

    #[test]
    fn without_a_cap_nothing_is_counted() {
        let mut m = QueryMem::new(None, free_pool(), false);
        let c = m.charge(&named("x"), usize::MAX, &mut |_| 0).unwrap();
        assert_eq!(c.bytes(), 0);
        assert_eq!(m.used(), 0);
        assert_eq!(m.room(), usize::MAX);
    }

    #[test]
    fn held_tables_count_while_they_live() {
        let mut m = QueryMem::new(Some(64 << 20), free_pool(), false);
        let rows: Vec<u32> = (0..1_000_000).collect();
        let t = Tab::rows_of(0, rows);
        m.hold(&t, &named("t"), &mut |_| 0).unwrap();
        let held = m.used();
        assert!(held >= 4_000_000, "{held}");
        // the same table again: counted once
        let t2 = t.clone();
        m.hold(&t2, &named("t"), &mut |_| 0).unwrap();
        assert_eq!(m.used(), held);
        drop(t);
        drop(t2);
        assert_eq!(m.room(), 64 << 20, "released with its last reference");
        assert_eq!(m.used(), 0);
        // a small table is not counted
        let small = Tab::rows_of(0, vec![1, 2, 3]);
        m.hold(&small, &named("s"), &mut |_| 0).unwrap();
        assert_eq!(m.used(), 0);
    }

    #[test]
    fn a_table_over_the_cap_is_refused() {
        let mut m = QueryMem::new(Some(1 << 20), free_pool(), false);
        let rows: Vec<Vec<V>> = (0..100_000)
            .map(|i| vec![V::Num(f64::from(i)), V::Blank])
            .collect();
        let t = Tab::tuples(Vec::new(), rows);
        assert!(m.hold(&t, &named("FILTER"), &mut |_| 0).is_err());
        assert_eq!(m.used(), 0);
    }
}
