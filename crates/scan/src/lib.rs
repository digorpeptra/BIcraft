//! L0: the kernels of bicraft's storage engine (`docs/storage-engine.md`).
//!
//! The DAX evaluator (`bicraft-engine`, the formula engine) turns the "scan, filter, group,
//! aggregate" parts of a query into storage requests, and these kernels run them over the
//! dictionary codes of the decoded VertiPaq columns, without going back to the values:
//!
//! - a [`Selection`] is a conjunction of code predicates (`allowed[code[row]]`, one bitmap per
//!   column over its dictionary) and row masks;
//! - a [`Key`] maps each selected row to a dense group number from the codes of the grouping
//!   columns, through a code lookup table when the column is on a dimension reached by
//!   many-to-one relationships (fact foreign-key code → dimension column code);
//! - a [`Sink`] accumulates one group's state per selected row (counts, exact integer sums,
//!   compensated double sums, min / max ranks, distinct-code bitmaps, code histograms).
//!
//! Execution is morsel-driven (Leis et al., SIGMOD 2014): the rows are cut into morsels that the
//! worker threads take from a shared counter, each worker filters a block of rows at a time into a
//! selection vector (vectorized, Kersten et al., VLDB 2018), then feeds its own sink; the sinks are
//! merged at the end. The workers belong to a persistent pool ([`pool`]); small scans stay on the
//! calling thread.
//!
//! Columns are read either as one code per row in memory or compressed ([`Packed`]: the
//! VertiPaq segments, unpacked a block at a time into the worker's buffers, RLE runs counted per
//! run where an operation allows it).
//!
//! Everything is safe Rust with bounded allocations; a worker that panics makes the scan return
//! `None` instead of propagating the panic.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable,
    clippy::indexing_slicing
)]

use std::sync::atomic::{AtomicUsize, Ordering};

pub mod lazy;
mod packed;
pub mod pool;

pub use packed::{Col, Ids, Packed, Run, SegData, SegStats, Segment};

/// Rows filtered at a time into a selection vector.
const BLOCK: usize = 2048;
/// Rows a worker takes at a time.
const MORSEL: usize = 1 << 16;
/// Fewest rows that make waking one more worker of the pool worth it: one morsel. A machine
/// whose wake-ups are slow gives each worker more ([`rows_per_thread`]). It was 512 Ki rows when
/// each scan started its own threads (~1.5 ms for 20 on Windows).
const ROWS_PER_THREAD: usize = 1 << 16;
/// Most rows per worker however slow the wake-ups.
const ROWS_PER_THREAD_MAX: usize = 1 << 19;
/// Below this many rows a scan stays on one thread (`BICRAFT_SCAN_PARALLEL_MIN_ROWS`): two
/// morsels, the least that two workers can share.
const PARALLEL_MIN_ROWS: usize = 1 << 17;

/// A positive number from the environment variable `key`.
fn env_rows(key: &str) -> Option<usize> {
    bicraft_env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n >= 1)
}

/// Rows per extra worker (`BICRAFT_SCAN_ROWS_PER_THREAD` overrides it, for tuning): at least as
/// many rows as the pool's wake-up round trip lasts in nanoseconds (the cheapest scans read about
/// a row per nanosecond on one thread), as a power of two between one morsel and 512 Ki rows.
/// Measured once, the first time a scan is big enough to ask: a few µs on Windows (one morsel),
/// ~120 µs in a WSL 2 VM (128 Ki rows).
fn rows_per_thread() -> usize {
    static T: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *T.get_or_init(|| {
        env_rows("BICRAFT_SCAN_ROWS_PER_THREAD").unwrap_or_else(|| {
            let rt = usize::try_from(pool::wake_round_trip_ns()).unwrap_or(usize::MAX);
            rt.checked_next_power_of_two()
                .unwrap_or(ROWS_PER_THREAD_MAX)
                .clamp(ROWS_PER_THREAD, ROWS_PER_THREAD_MAX)
        })
    })
}

/// Whether a scan of `rows` rows is too small to run in parallel (no pool needed to tell).
fn too_small(rows: usize) -> bool {
    static T: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    rows < *T.get_or_init(|| env_rows("BICRAFT_SCAN_PARALLEL_MIN_ROWS").unwrap_or(PARALLEL_MIN_ROWS))
}

/// Rows per worker of the cheapest loops (`mask`, `rows`: one predicate lookup per row, ~2 G rows
/// per second on one thread), where waking a worker is worth fewer rows' work.
const CHEAP_ROWS_PER_THREAD: usize = 1 << 18;

/// Workers for one of the cheapest loops over `rows` rows, at most `threads`.
fn cheap_threads(rows: usize, threads: usize) -> usize {
    if too_small(rows) {
        return 1;
    }
    let per = CHEAP_ROWS_PER_THREAD.max(rows_per_thread());
    threads.min(rows / per).max(1)
}

/// Most bytes of sink state all the workers of one scan may hold together.
const SINK_BUDGET: usize = 1 << 30;
/// Most groups of a [`Key`].
pub const MAX_GROUPS: usize = 1 << 26;

/// Rows of a column whose code is allowed (`allowed[code]`; codes past the end are not).
#[derive(Debug, Clone, Copy)]
pub struct Pred<'a> {
    pub codes: Col<'a>,
    pub allowed: &'a [bool],
}

/// The rows a scan visits: `0..rows`, kept by every predicate and every mask.
#[derive(Debug, Clone, Default)]
pub struct Selection<'a> {
    pub rows: usize,
    pub preds: Vec<Pred<'a>>,
    pub masks: Vec<&'a [bool]>,
    /// Nothing is selected (a predicate that allows no code at all).
    pub none: bool,
}

/// How a scan reads one column: its codes, or the data ids of a compressed column, with the
/// operation's per-code table composed with the column's id → code table once per scan.
#[derive(Clone, Copy)]
enum Read<'a> {
    Codes(&'a [u32]),
    Ids(&'a Packed),
}

impl Read<'_> {
    #[inline]
    fn block<'b>(&'b self, start: usize, len: usize, buf: &'b mut Vec<u32>) -> &'b [u32] {
        match self {
            Read::Codes(c) => match c.get(start..start.saturating_add(len)) {
                Some(s) => s,
                None => {
                    buf.clear();
                    buf.extend((start..start.saturating_add(len)).map(|r| c.get(r).copied().unwrap_or(0)));
                    buf
                }
            },
            Read::Ids(p) => {
                buf.resize(len, 0);
                p.fill_ids(start, buf);
                buf
            }
        }
    }

    #[inline]
    fn get(&self, row: usize) -> u32 {
        match self {
            Read::Codes(c) => c.get(row).copied().unwrap_or(0),
            Read::Ids(p) => p.get_id(row),
        }
    }
}

/// One grouping column of a [`Plan`]: (column, code or id → group code, codes, stride).
/// (column, its code or id → the part's group number already multiplied by its stride, the
/// number for codes past the table).
type PlanKey<'a> = (Read<'a>, Vec<u32>, u32);

/// One predicate of a [`Plan`].
struct PlanPred<'a> {
    col: Read<'a>,
    /// Allowed codes, or ids of a compressed column.
    table: std::borrow::Cow<'a, [bool]>,
    /// Segments of a compressed column where no row can be allowed: their statistics' id range
    /// holds no allowed id.
    dead: Vec<bool>,
    /// Per segment that is not dead, its zones ([`Packed::zones`]) where no row can be allowed.
    dead_zones: Vec<Vec<bool>>,
}

/// Whether a table of allowed ids holds one in a range of ids, in constant time whatever the
/// range: the table as bits, and the allowed ids before each word (a prefix sum).
struct AllowedIds {
    words: Vec<u64>,
    before: Vec<u64>,
}

impl AllowedIds {
    fn new(t: &[bool]) -> AllowedIds {
        let words: Vec<u64> = t
            .chunks(64)
            .map(|ch| {
                ch.iter()
                    .enumerate()
                    .fold(0u64, |w, (i, x)| w | (u64::from(*x) << i))
            })
            .collect();
        let mut before = Vec::with_capacity(words.len().saturating_add(1));
        let mut n = 0u64;
        before.push(0);
        for w in &words {
            n = n.saturating_add(u64::from(w.count_ones()));
            before.push(n);
        }
        AllowedIds { words, before }
    }

    /// Allowed ids below `x`.
    fn below(&self, x: usize) -> u64 {
        let (w, b) = (x / 64, x % 64);
        match self.words.get(w) {
            Some(word) => {
                let low = word & ((1u64 << b) - 1);
                self.before.get(w).copied().unwrap_or(0) + u64::from(low.count_ones())
            }
            None => self.before.last().copied().unwrap_or(0),
        }
    }

    /// Whether an id of `lo..=hi` is allowed.
    fn any(&self, lo: u32, hi: u32) -> bool {
        lo <= hi && self.below((hi as usize).saturating_add(1)) > self.below(lo as usize)
    }
}

/// A [`Selection`] and a [`Key`] with their tables composed for the columns they read.
struct Plan<'a> {
    none: bool,
    preds: Vec<PlanPred<'a>>,
    masks: Vec<&'a [bool]>,
    keys: Vec<PlanKey<'a>>,
}

impl<'a> Plan<'a> {
    fn new(sel: &Selection<'a>, key: &Key<'a>) -> Plan<'a> {
        use std::borrow::Cow;
        let preds = sel
            .preds
            .iter()
            .map(|p| match p.codes {
                Col::Plain(c) => PlanPred {
                    col: Read::Codes(c),
                    table: Cow::Borrowed(p.allowed),
                    dead: Vec::new(),
                    dead_zones: Vec::new(),
                },
                Col::Packed(pk) => {
                    let t: Vec<bool> = pk
                        .lut()
                        .iter()
                        .map(|code| p.allowed.get(*code as usize).copied().unwrap_or(false))
                        .collect();
                    // segment statistics and zone maps: skip what cannot match, whatever the
                    // width of the id range (a min-max skip index)
                    let any = AllowedIds::new(&t);
                    let dead: Vec<bool> = pk
                        .segments()
                        .iter()
                        .map(|s| !any.any(s.min_id, s.max_id))
                        .collect();
                    let dead_zones = dead
                        .iter()
                        .enumerate()
                        .map(|(si, d)| match d {
                            true => Vec::new(),
                            false => pk.zones(si).iter().map(|(lo, hi)| !any.any(*lo, *hi)).collect(),
                        })
                        .collect();
                    PlanPred {
                        col: Read::Ids(pk),
                        table: Cow::Owned(t),
                        dead,
                        dead_zones,
                    }
                }
            })
            .collect::<Vec<PlanPred<'a>>>();
        // the most selective first (PREWHERE-style): the fraction of its codes or ids allowed,
        // then the fewest of them (a conjunction gives the same rows in any order)
        let mut preds: Vec<(f64, usize, PlanPred<'a>)> = preds
            .into_iter()
            .map(|p| {
                let n = p.table.len();
                let allowed = p.table.iter().filter(|x| **x).count();
                (allowed as f64 / n.max(1) as f64, n, p)
            })
            .collect();
        preds.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let preds: Vec<PlanPred<'a>> = preds.into_iter().map(|(_, _, p)| p).collect();
        let keys = key
            .parts
            .iter()
            .map(|(p, stride)| {
                let card = p.card.max(1);
                let stride = u32::try_from(*stride).unwrap_or(u32::MAX);
                let last = u32::try_from(card - 1).unwrap_or(u32::MAX).wrapping_mul(stride);
                // the part's group number of a code, clamped to its codes, times its stride
                let of_code = |c: u32| -> u32 {
                    let g = match p.lut {
                        Some(l) => l.get(c as usize).copied().unwrap_or(0),
                        None => c,
                    };
                    g.min(u32::try_from(card - 1).unwrap_or(u32::MAX))
                        .wrapping_mul(stride)
                };
                match p.codes {
                    Col::Plain(c) => {
                        let n = p.lut.map_or(card, <[u32]>::len);
                        let t: Vec<u32> = (0..n).map(|k| of_code(u32::try_from(k).unwrap_or(0))).collect();
                        // codes past a lookup table had group 0 (they do not occur)
                        (Read::Codes(c), t, if p.lut.is_some() { 0 } else { last })
                    }
                    Col::Packed(pk) => {
                        let t: Vec<u32> = pk.lut().iter().map(|code| of_code(*code)).collect();
                        (Read::Ids(pk), t, 0)
                    }
                }
            })
            .collect();
        Plan {
            none: sel.none,
            preds,
            masks: sel.masks.clone(),
            keys,
        }
    }

    fn filtered(&self) -> bool {
        !self.preds.is_empty() || !self.masks.is_empty()
    }

    /// Marks in `ok` the selected rows of `start..start + ok.len()`; `true`: every row is (no
    /// predicate or mask removed one: the block lies in runs of allowed ids).
    fn block(&self, start: usize, ok: &mut [bool], bufs: &mut Vec<Vec<u32>>) -> bool {
        ok.fill(!self.none);
        if self.none {
            return false;
        }
        let mut all = true;
        let len = ok.len();
        let end = start.saturating_add(len);
        if bufs.len() < self.preds.len() {
            bufs.resize_with(self.preds.len(), Vec::new);
        }
        // first what costs no unpacking, for every predicate: a dead segment or zone, a run of
        // one id (`runs`: the predicates whose block is one run of an allowed id)
        let mut runs = 0u64;
        for (i, p) in self.preds.iter().enumerate() {
            let Read::Ids(pk) = &p.col else {
                continue;
            };
            if let Some((si, first, rows)) = pk.segment_at(start)
                && first.saturating_add(rows) >= end
            {
                // the whole block is in a segment, or a zone of one, that no allowed id reaches
                let z = start.saturating_sub(first) / packed::ZONE_ROWS;
                let zone_end = first.saturating_add(z.saturating_add(1).saturating_mul(packed::ZONE_ROWS));
                let dead_zone = end <= zone_end
                    && p.dead_zones
                        .get(si)
                        .and_then(|d| d.get(z))
                        .copied()
                        .unwrap_or(false);
                if dead_zone || p.dead.get(si).copied().unwrap_or(false) {
                    ok.fill(false);
                    return false;
                }
            }
            if let Some(id) = pk.run_through(start, end) {
                if !p.table.get(id as usize).copied().unwrap_or(false) {
                    ok.fill(false);
                    return false;
                }
                if i < 64 {
                    runs |= 1 << i;
                }
            }
        }
        let last = self.preds.len().saturating_sub(1);
        for (i, (p, buf)) in self.preds.iter().zip(bufs.iter_mut()).enumerate() {
            let (col, table) = (&p.col, &p.table);
            // the whole block in one run of an allowed id (seen above)
            if i < 64 {
                if runs & (1 << i) != 0 {
                    continue;
                }
            } else if let Read::Ids(pk) = col
                && pk.run_through(start, end).is_some()
            {
                continue;
            }
            all = false;
            let cs = col.block(start, len, buf);
            for (o, c) in ok.iter_mut().zip(cs) {
                *o &= table.get(*c as usize).copied().unwrap_or(false);
            }
            // no row left: the other predicates and masks are not read
            if (i < last || !self.masks.is_empty()) && !ok.iter().any(|x| *x) {
                return false;
            }
        }
        for m in &self.masks {
            all = false;
            match m.get(start..end) {
                Some(ms) => {
                    for (o, x) in ok.iter_mut().zip(ms) {
                        *o &= *x;
                    }
                }
                None => ok.fill(false),
            }
        }
        all
    }

    /// The groups of rows `start..start + len` into `out`; `true`: they are all the same (the
    /// block lies in runs of the keys' columns).
    fn groups(&self, start: usize, len: usize, bufs: &mut Vec<Vec<u32>>, out: &mut Vec<u32>) -> bool {
        if bufs.len() < self.keys.len() {
            bufs.resize_with(self.keys.len(), Vec::new);
        }
        out.clear();
        // keys whose block is one run: their part of the group, the same on every row
        let end = start.saturating_add(len);
        let mut same = 0u32;
        let mut varying = false;
        for (i, ((col, table, last), buf)) in self.keys.iter().zip(bufs.iter_mut()).enumerate() {
            if let Read::Ids(pk) = col
                && let Some(id) = pk.run_through(start, end)
            {
                same = same.wrapping_add(table.get(id as usize).copied().unwrap_or(*last));
                if i == self.keys.len() - 1 && !varying {
                    out.clear();
                    out.resize(len, same);
                    return true;
                }
                continue;
            }
            let first = !varying;
            varying = true;
            let cs = col.block(start, len, buf);
            if first {
                out.extend(
                    cs.iter()
                        .map(|c| table.get(*c as usize).copied().unwrap_or(*last)),
                );
            } else {
                for (g, c) in out.iter_mut().zip(cs) {
                    *g = g.wrapping_add(table.get(*c as usize).copied().unwrap_or(*last));
                }
            }
        }
        out.resize(len, 0);
        if same != 0 {
            for g in out.iter_mut() {
                *g = g.wrapping_add(same);
            }
        }
        !varying
    }

    /// For a grouping by one compressed column whose block lies in runs of one id: the runs and
    /// their groups in `out` ((offset, rows, group)); `false` otherwise.
    fn group_runs(&self, start: usize, len: usize, out: &mut Vec<(u32, u32, u32)>) -> bool {
        let [(Read::Ids(pk), table, last)] = self.keys.as_slice() else {
            return false;
        };
        if !pk.runs_in(start, start.saturating_add(len), out) {
            return false;
        }
        for r in out.iter_mut() {
            r.2 = table.get(r.2 as usize).copied().unwrap_or(*last);
        }
        true
    }

    /// The group of row `row`.
    #[inline]
    fn group_of(&self, row: usize) -> usize {
        let mut g = 0u32;
        for (col, table, last) in &self.keys {
            g = g.wrapping_add(table.get(col.get(row) as usize).copied().unwrap_or(*last));
        }
        g as usize
    }
}

/// A worker's buffers: the codes of one block per column, the selection, the groups.
#[derive(Default)]
struct Scratch {
    ok: Vec<bool>,
    preds: Vec<Vec<u32>>,
    keys: Vec<Vec<u32>>,
    values: Vec<u32>,
    groups: Vec<u32>,
    /// Runs of one group: (offset in the block, rows, group).
    runs: Vec<(u32, u32, u32)>,
    /// Per column of [`sum_by_id_multi`]: its id when the block lies in one run of it, and
    /// whether its ids there hold no blank.
    cols: Vec<(Option<u32>, bool)>,
}

impl Selection<'_> {
    pub fn all(rows: usize) -> Selection<'static> {
        Selection {
            rows,
            ..Selection::default()
        }
    }

    fn filtered(&self) -> bool {
        !self.preds.is_empty() || !self.masks.is_empty()
    }
}

/// One grouping column: its code per row, mapped through `lut` when given (a code of the
/// scanned column → the group column's code), with `card` group codes.
#[derive(Debug, Clone, Copy)]
pub struct KeyPart<'a> {
    pub codes: Col<'a>,
    pub lut: Option<&'a [u32]>,
    pub card: usize,
}

/// Dense group numbers: `Σ part code × stride`, the first part varying slowest.
#[derive(Debug, Clone, Default)]
pub struct Key<'a> {
    parts: Vec<(KeyPart<'a>, usize)>,
    groups: usize,
}

impl<'a> Key<'a> {
    /// `None` when the groups would be more than [`MAX_GROUPS`].
    pub fn new(parts: Vec<KeyPart<'a>>) -> Option<Key<'a>> {
        let mut groups = 1usize;
        for p in &parts {
            groups = groups.checked_mul(p.card.max(1))?;
        }
        if groups > MAX_GROUPS {
            return None;
        }
        let mut stride = groups;
        let mut out = Vec::with_capacity(parts.len());
        for p in parts {
            stride /= p.card.max(1);
            out.push((p, stride));
        }
        Some(Key { parts: out, groups })
    }

    /// One group (no grouping column).
    pub fn single() -> Key<'static> {
        Key {
            parts: Vec::new(),
            groups: 1,
        }
    }

    pub fn groups(&self) -> usize {
        self.groups
    }

    /// The group of a combination of group codes (one per part, in order).
    pub fn id(&self, codes: &[u32]) -> Option<usize> {
        let mut g = 0usize;
        for ((p, stride), c) in self.parts.iter().zip(codes) {
            let c = *c as usize;
            if c >= p.card.max(1) {
                return None;
            }
            g = g.checked_add(c.checked_mul(*stride)?)?;
        }
        Some(g)
    }

    /// The group codes (one per part, in order) of group `g`.
    pub fn codes(&self, g: usize) -> Vec<u32> {
        self.parts
            .iter()
            .map(|(p, stride)| {
                let c = g.checked_div(*stride).unwrap_or(0) % p.card.max(1);
                u32::try_from(c).unwrap_or(u32::MAX)
            })
            .collect()
    }

    #[inline]
    fn part_code(p: &KeyPart<'_>, c: u32) -> usize {
        let c = match p.lut {
            Some(l) => l.get(c as usize).copied().unwrap_or(0),
            None => c,
        } as usize;
        c.min(p.card.saturating_sub(1))
    }

    /// The group of row `row`.
    #[inline]
    pub fn of_row(&self, row: usize) -> usize {
        let mut g = 0usize;
        for (p, stride) in &self.parts {
            g = g.wrapping_add(Self::part_code(p, p.codes.get(row)).wrapping_mul(*stride));
        }
        g
    }
}

/// Calls `f(row, group, value)` for every selected row of `start..start + len` (at most
/// [`BLOCK`] rows).
#[inline]
fn each_selected(
    sel: &Plan<'_>,
    values: Option<Col<'_>>,
    start: usize,
    len: usize,
    scr: &mut Scratch,
    mut f: impl FnMut(usize, usize, u32),
) {
    if sel.none {
        return;
    }
    let Scratch {
        ok,
        preds,
        keys,
        values: vbuf,
        groups,
        ..
    } = scr;
    let filtered = sel.filtered();
    if filtered {
        ok.resize(len, false);
        sel.block(start, ok, preds);
        if !ok.iter().any(|x| *x) {
            return;
        }
    }
    let vs: &[u32] = match &values {
        Some(v) => v.block(start, len, vbuf),
        None => &[],
    };
    if sel.keys.is_empty() {
        // one group: no group numbers to compute
        match (filtered, values.is_some()) {
            (false, true) => {
                for (i, v) in vs.iter().enumerate() {
                    f(start.wrapping_add(i), 0, *v);
                }
            }
            (false, false) => (0..len).for_each(|i| f(start.wrapping_add(i), 0, 0)),
            (true, _) => {
                for (i, keep) in ok.iter().enumerate() {
                    if *keep {
                        f(start.wrapping_add(i), 0, vs.get(i).copied().unwrap_or(0));
                    }
                }
            }
        }
        return;
    }
    sel.groups(start, len, keys, groups);
    for (i, g) in groups.iter().enumerate() {
        if filtered && !ok.get(i).copied().unwrap_or(false) {
            continue;
        }
        f(
            start.wrapping_add(i),
            *g as usize,
            vs.get(i).copied().unwrap_or(0),
        );
    }
}

/// Per-group state of one scan.
pub trait Sink: Send + Sized {
    /// The selected row `row` of group `group`, whose aggregated code is `value` (0 without one).
    fn add(&mut self, group: usize, value: u32, row: usize);
    /// Folds the state of another worker in.
    fn merge(&mut self, other: Self);
    /// Whether [`Sink::add`] reads the row rather than the value (a radix-partitioned scan then
    /// routes the row).
    const NEEDS_ROW: bool = false;
}

/// A sink whose groups can be split in ranges: one worker per range of groups, the ranges joined
/// in order ([`scan_groups`]).
pub trait Ranged: Sink {
    /// Appends the groups of `other` after this sink's.
    fn append(&mut self, other: Self);
}

/// Sinks of more groups than this (per worker) are radix-partitioned by group.
const RANGED_MIN_BYTES: usize = 8 << 20;
/// One worker's rows of a radix-partitioned scan: per partition, (group in the partition,
/// value or row).
type Routed = Vec<Vec<(u32, u32)>>;
/// State of one partition of a radix-partitioned scan.
const PARTITION_BYTES: usize = 256 << 10;

/// Runs `work(start, end)` over morsels of `0..rows` on `threads` workers, each with its own
/// state made by `make`; returns the states (`None`: a worker failed).
fn morsels<S: Send>(
    rows: usize,
    threads: usize,
    make: impl Fn() -> S + Sync,
    work: impl Fn(&mut S, usize, usize) + Sync,
) -> Option<Vec<S>> {
    if threads <= 1 || too_small(rows) {
        let mut s = make();
        let mut at = 0usize;
        while at < rows {
            let end = at.saturating_add(MORSEL).min(rows);
            work(&mut s, at, end);
            at = end;
        }
        return Some(vec![s]);
    }
    // no more workers than morsels
    let threads = threads.min(rows.div_ceil(MORSEL));
    let next = AtomicUsize::new(0);
    pool::map(threads, |_| {
        let mut s = make();
        loop {
            let m = next.fetch_add(1, Ordering::Relaxed);
            let start = m.saturating_mul(MORSEL);
            if start >= rows {
                break;
            }
            work(&mut s, start, start.saturating_add(MORSEL).min(rows));
        }
        s
    })
}

/// Most bytes [`scan_groups_within`] holds at once over `rows` rows into `groups` groups of
/// `group_bytes` each, its routed rows held to `max_routed` bytes: the workers' states;
/// radix-partitioned, the routed (group, payload) pairs of one window of rows (8 bytes per
/// selected row, twice that while a partition's vector grows) and the partitions' states, joined.
pub fn scan_groups_bytes(rows: usize, groups: usize, group_bytes: usize, max_routed: usize) -> usize {
    scan_groups_range_bytes(rows, groups, group_bytes, max_routed, usize::MAX)
}

/// [`scan_groups_bytes`] of a [`scan_groups_range`] keeping `len` groups whose workers' states
/// hold at most about `max_states` bytes together (fewer workers otherwise, at least one).
pub fn scan_groups_range_bytes(
    rows: usize,
    len: usize,
    group_bytes: usize,
    max_routed: usize,
    max_states: usize,
) -> usize {
    let bytes = len.saturating_mul(group_bytes);
    let threads = configured_threads();
    if per_worker(rows, bytes, max_states) || threads <= 1 || too_small(rows) || len < threads {
        return bytes.saturating_mul(range_threads(rows, bytes, max_states));
    }
    let window = routed_window(rows, max_routed);
    window
        .saturating_mul(ROUTED_ROW_BYTES)
        .saturating_add(bytes.saturating_mul(2))
}

/// Bytes a routed row may take: a (group, payload) pair, twice while its vector grows.
const ROUTED_ROW_BYTES: usize = 16;

/// Most bytes of routed rows a radix-partitioned scan holds at once whatever its cap: the rows go
/// by windows of about 8 Mi rows, whose buffers are reused from one window to the next (an
/// arena: allocated and faulted in once per scan instead of growing for all the rows).
const ROUTED_DEFAULT_BYTES: usize = 128 << 20;

/// Rows routed at once so that they hold at most `max_routed` bytes (and at most
/// [`ROUTED_DEFAULT_BYTES`]): whole morsels, at least one.
fn routed_window(rows: usize, max_routed: usize) -> usize {
    let w = (max_routed.min(ROUTED_DEFAULT_BYTES) / ROUTED_ROW_BYTES).max(MORSEL);
    let w = w.div_ceil(MORSEL).saturating_mul(MORSEL);
    w.min(rows.max(1))
}

/// [`scan`] for a sink of `groups` groups of `group_bytes` each, made by `make(groups)`. With few
/// groups the workers split the rows. With many (a high-cardinality grouping, whose state per
/// worker would not fit in cache or in memory), the scan is radix-partitioned by group: the
/// workers first route each selected row's (group, value or row) to one of a few hundred
/// partitions of consecutive groups, then each partition is aggregated by one worker in a state
/// small enough to stay in cache, and the partitions' states are joined in order.
pub fn scan_groups<S: Ranged>(
    sel: &Selection<'_>,
    key: &Key<'_>,
    values: Option<Col<'_>>,
    group_bytes: usize,
    make: impl Fn(usize) -> S + Sync,
) -> Option<S> {
    scan_groups_within(sel, key, values, group_bytes, usize::MAX, make)
}

/// [`scan_groups`] whose radix-partitioned path holds at most about `max_routed` bytes of routed
/// rows (`docs/beyond-ram.md`, lot 4): the rows go by windows of whole morsels, each routed then
/// added to the partitions' states, which carry from one window to the next. The sinks are
/// order-insensitive (the workers already route their morsels in any order), so the result is
/// the same whatever the window.
pub fn scan_groups_within<S: Ranged>(
    sel: &Selection<'_>,
    key: &Key<'_>,
    values: Option<Col<'_>>,
    group_bytes: usize,
    max_routed: usize,
    make: impl Fn(usize) -> S + Sync,
) -> Option<S> {
    let groups = key.groups();
    scan_groups_in::<S, false>(
        sel,
        key,
        values,
        group_bytes,
        max_routed,
        0,
        groups,
        usize::MAX,
        make,
    )
}

/// [`scan_groups_within`] keeping only the groups `lo..lo + len` of `key` (clamped to its
/// groups), numbered from 0 in the sink made by `make(len)`; the rows of other groups are skipped
/// (`docs/beyond-ram.md`, "Groupings by ranges"). The workers' states hold at most about
/// `max_states` bytes together (fewer workers, at least one). Each group's state is the one
/// [`scan_groups_within`] gives it: a group's state only depends on its own rows, and every sink
/// is order-insensitive.
#[allow(clippy::too_many_arguments)]
pub fn scan_groups_range<S: Ranged>(
    sel: &Selection<'_>,
    key: &Key<'_>,
    values: Option<Col<'_>>,
    group_bytes: usize,
    max_routed: usize,
    lo: usize,
    len: usize,
    max_states: usize,
    make: impl Fn(usize) -> S + Sync,
) -> Option<S> {
    let len = len.min(key.groups().saturating_sub(lo));
    scan_groups_in::<S, true>(
        sel,
        key,
        values,
        group_bytes,
        max_routed,
        lo,
        len,
        max_states,
        make,
    )
}

/// The group of a scan over the groups `lo..lo + len` (`RANGE`), numbered from 0; `None` when
/// outside. Without a range, the group itself (no work).
#[inline(always)]
fn in_range<const RANGE: bool>(g: usize, lo: usize, len: usize) -> Option<usize> {
    if RANGE {
        let x = g.wrapping_sub(lo);
        (x < len).then_some(x)
    } else {
        Some(g)
    }
}

/// Whether a grouped scan whose sink holds `bytes` gives each worker a sink of every group (else
/// it is radix-partitioned): small sinks, unless their copies on the workers would not fit in
/// `max_states` (a range of groups under a memory cap: partitioned states shared by every
/// thread rather than one worker's sink).
fn per_worker(rows: usize, bytes: usize, max_states: usize) -> bool {
    bytes < RANGED_MIN_BYTES && bytes.saturating_mul(threads_for(rows, bytes)) <= max_states
}

/// Threads of a scan whose workers each hold `bytes`, held to `max_states` bytes together.
fn range_threads(rows: usize, bytes: usize, max_states: usize) -> usize {
    threads_for(rows, bytes).min(max_states.checked_div(bytes).unwrap_or(usize::MAX).max(1))
}

#[allow(clippy::too_many_arguments)]
fn scan_groups_in<S: Ranged, const RANGE: bool>(
    sel: &Selection<'_>,
    key: &Key<'_>,
    values: Option<Col<'_>>,
    group_bytes: usize,
    max_routed: usize,
    lo: usize,
    groups: usize,
    max_states: usize,
    make: impl Fn(usize) -> S + Sync,
) -> Option<S> {
    let bytes = groups.saturating_mul(group_bytes);
    let threads = configured_threads();
    if per_worker(sel.rows, bytes, max_states) || threads <= 1 || too_small(sel.rows) || groups < threads {
        let t = range_threads(sel.rows, bytes, max_states);
        return scan_in::<S, RANGE>(sel, key, values, t, lo, groups, || make(groups));
    }
    // partitions of about 256 KiB of state each, a power of two of groups
    let per = (PARTITION_BYTES / group_bytes.max(1)).max(64).next_power_of_two();
    let shift = per.trailing_zeros();
    let parts = groups.div_ceil(per);
    let payload_is_row = S::NEEDS_ROW;
    let plan = Plan::new(sel, key);
    let window = routed_window(sel.rows, max_routed);
    // the partitions' states, made when a window first reaches them
    let sinks: Vec<std::sync::Mutex<Option<S>>> = (0..parts).map(|_| std::sync::Mutex::new(None)).collect();
    // the routed buffers, reused from one window to the next; each partition's vector is first
    // sized for an even share of a worker's rows (a quarter more), so it seldom grows
    let reserve = window
        .div_ceil(threads.min(window.div_ceil(MORSEL)).max(1))
        .div_ceil(parts.max(1))
        .saturating_mul(5)
        / 4;
    let spare: std::sync::Mutex<Vec<Routed>> = std::sync::Mutex::new(Vec::new());
    let routed_buffers = || -> Routed {
        let reused = spare.lock().ok().and_then(|mut s| s.pop());
        reused.unwrap_or_else(|| (0..parts).map(|_| Vec::with_capacity(reserve)).collect())
    };
    let mut lo_row = 0usize;
    while lo_row < sel.rows {
        let hi_row = lo_row.saturating_add(window).min(sel.rows);
        // phase 1: each worker routes its morsels' rows to partitions
        let routed: Vec<(Routed, Scratch)> = morsels(
            hi_row.saturating_sub(lo_row),
            threads,
            || (routed_buffers(), Scratch::default()),
            |(out, scr): &mut (Routed, Scratch), start, end| {
                let mut at = lo_row.saturating_add(start);
                let end = lo_row.saturating_add(end);
                while at < end {
                    let len = (end - at).min(BLOCK);
                    each_selected(&plan, values, at, len, scr, |row, g, v| {
                        let Some(g) = in_range::<RANGE>(g, lo, groups) else {
                            return;
                        };
                        let payload = if payload_is_row {
                            u32::try_from(row).unwrap_or(u32::MAX)
                        } else {
                            v
                        };
                        if let Some(p) = out.get_mut(g >> shift) {
                            p.push((u32::try_from(g & (per - 1)).unwrap_or(0), payload));
                        }
                    });
                    at = at.saturating_add(len);
                }
            },
        )?;
        let routed: Vec<Routed> = routed.into_iter().map(|(r, _)| r).collect();
        // phase 2: one partition at a time per worker
        let next = AtomicUsize::new(0);
        let ok: Vec<bool> = pool::map(threads.min(parts), |_| {
            loop {
                let p = next.fetch_add(1, Ordering::Relaxed);
                if p >= parts {
                    return true;
                }
                let Some(Ok(mut slot)) = sinks.get(p).map(|m| m.lock()) else {
                    return false;
                };
                let sink = slot.get_or_insert_with(|| {
                    let lo = p.saturating_mul(per);
                    make(per.min(groups.saturating_sub(lo)))
                });
                for w in &routed {
                    for (g, payload) in w.get(p).map_or(&[][..], Vec::as_slice) {
                        if payload_is_row {
                            sink.add(*g as usize, 0, *payload as usize);
                        } else {
                            sink.add(*g as usize, *payload, 0);
                        }
                    }
                }
            }
        })?;
        if ok.iter().any(|o| !o) {
            return None;
        }
        lo_row = hi_row;
        if lo_row < sel.rows
            && let Ok(mut s) = spare.lock()
        {
            for mut r in routed {
                r.iter_mut().for_each(Vec::clear);
                s.push(r);
            }
        }
    }
    let mut out: Option<S> = None;
    for (p, m) in sinks.into_iter().enumerate() {
        let s = match m.into_inner() {
            Ok(Some(s)) => s,
            Ok(None) => {
                let lo = p.saturating_mul(per);
                make(per.min(groups.saturating_sub(lo)))
            }
            Err(_) => return None,
        };
        match &mut out {
            None => out = Some(s),
            Some(o) => o.append(s),
        }
    }
    out.or_else(|| Some(make(0)))
}

/// Threads a scan of `rows` rows may use when each worker's sink holds `sink_bytes`:
/// `BICRAFT_SCAN_THREADS` when set, else the machine's parallelism, fewer when the sinks would
/// not fit in the budget, one for small scans.
pub fn threads_for(rows: usize, sink_bytes: usize) -> usize {
    if too_small(rows) {
        return 1;
    }
    let max = configured_threads();
    let by_memory = SINK_BUDGET.checked_div(sink_bytes.max(1)).unwrap_or(1).max(1);
    // a worker is worth waking for `rows_per_thread()` rows, and when its rows outweigh the cost of
    // clearing and merging its sink (~2 rows per byte of sink)
    let by_rows = (rows / rows_per_thread()).max(1);
    let by_merge = rows.checked_div(sink_bytes / 2).unwrap_or(usize::MAX).max(1);
    max.min(by_memory).min(by_rows).min(by_merge).max(1)
}

thread_local! {
    /// Threads the scans started from this thread use, overriding the configuration (tests).
    static THREADS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Sets (or clears) the threads the scans started from this thread may use.
pub fn set_threads(n: Option<usize>) {
    THREADS.with(|t| t.set(n.map(|n| n.clamp(1, 256))));
}

/// [`set_threads`] for this thread, else `BICRAFT_SCAN_THREADS`, else the available
/// parallelism.
pub fn configured_threads() -> usize {
    if let Some(n) = THREADS.with(std::cell::Cell::get) {
        return n;
    }
    pool::process_threads()
}

/// Runs `sel` grouped by `key`, feeding each selected row's group and `values` code to a sink
/// made by `make` per worker. `None`: a worker failed.
pub fn scan<S: Sink>(
    sel: &Selection<'_>,
    key: &Key<'_>,
    values: Option<Col<'_>>,
    threads: usize,
    make: impl Fn() -> S + Sync,
) -> Option<S> {
    scan_in::<S, false>(sel, key, values, threads, 0, usize::MAX, make)
}

/// [`scan`] feeding only the groups `lo..lo + groups` (numbered from 0) when `RANGE`.
fn scan_in<S: Sink, const RANGE: bool>(
    sel: &Selection<'_>,
    key: &Key<'_>,
    values: Option<Col<'_>>,
    threads: usize,
    lo: usize,
    groups: usize,
    make: impl Fn() -> S + Sync,
) -> Option<S> {
    let plan = Plan::new(sel, key);
    let states = morsels(
        sel.rows,
        threads,
        || (make(), Scratch::default()),
        |(sink, scr): &mut (S, Scratch), start, end| {
            let mut at = start;
            while at < end {
                let len = (end - at).min(BLOCK);
                each_selected(&plan, values, at, len, scr, |row, g, v| {
                    if let Some(g) = in_range::<RANGE>(g, lo, groups) {
                        sink.add(g, v, row);
                    }
                });
                at = at.saturating_add(len);
            }
        },
    )?;
    let mut out: Option<S> = None;
    for (s, _) in states {
        match &mut out {
            None => out = Some(s),
            Some(o) => o.merge(s),
        }
    }
    out
}

/// The values a [`sum_by_id`] adds, by data id of a compressed column.
#[derive(Debug, Clone, Copy)]
pub enum IdValues<'a> {
    /// A value-encoded column: the ids from `first` on (their sum; the value is an affine
    /// function of the id), the ids below are blanks.
    Affine { first: u32 },
    /// The value of each id, and whether it has one (a blank has none).
    Table { values: &'a [i64], valid: &'a [bool] },
}

/// Per group, the sum of a compressed column's values and how many there are, read by data id
/// (no id → code lookup per row): one tight loop per block over the unpacked ids, the selection
/// and the group numbers. `None`: a worker failed.
pub fn sum_by_id(
    sel: &Selection<'_>,
    key: &Key<'_>,
    col: &Packed,
    vals: IdValues<'_>,
    threads: usize,
) -> Option<(Vec<i128>, Vec<u64>)> {
    sum_by_id_multi(sel, key, &[(col, vals)], threads)?
        .into_iter()
        .next()
}

/// Per group, a sum and a count (what [`sum_by_id_multi`] gives per column).
pub type GroupSums = (Vec<i128>, Vec<u64>);

/// A worker's sums of one column: of ids (value-encoded: exact in a u64 below 2^32 rows, ids
/// being 32-bit), of values that cannot overflow an i64 over the table's rows, or of any values
/// (i128).
enum Part {
    Ids(ColAcc<u64>),
    Small(ColAcc<i64>),
    Wide(ColAcc<i128>),
}

/// The integer types a column's sums are kept in. Sums of integers do not depend on their
/// order, so the kernel's copies, blocks and workers may add in any order: the result is exact
/// and the same on any number of threads.
trait Num: Copy + Default + std::ops::AddAssign {
    /// `n` times the value (`n` is at most a block's rows: exact within the callers' bounds).
    fn times(self, n: u64) -> Self;
    fn wide(self) -> i128;
}

impl Num for u64 {
    fn times(self, n: u64) -> u64 {
        self.saturating_mul(n)
    }
    fn wide(self) -> i128 {
        i128::from(self)
    }
}

impl Num for i64 {
    fn times(self, n: u64) -> i64 {
        self.saturating_mul(i64::try_from(n).unwrap_or(i64::MAX))
    }
    fn wide(self) -> i128 {
        i128::from(self)
    }
}

impl Num for i128 {
    fn times(self, n: u64) -> i128 {
        self.saturating_mul(i128::from(n))
    }
    fn wide(self) -> i128 {
        self
    }
}

/// Copies of the accumulators of the interleaved kernel: row `i` of a block adds to copy
/// `i % 4`, so rows of one group in a row add to different copies and their adds do not wait on
/// each other through memory (the store-to-load chain of `sums[g] += v` on the same `g`).
const COPIES: usize = 4;
/// Most group slots for which the copies are kept: with more groups, consecutive rows seldom
/// share one (1 in the groups), and four copies would only crowd the cache (measured: ~10 %
/// slower at 511 groups, with four copies).
const COPIES_MAX_SLOTS: usize = 128;
/// Elements left unused before and after a worker's accumulators (at least two cache lines of
/// u64): small vectors allocated by different workers would otherwise share cache lines, and
/// their adds would bounce the lines between cores (false sharing).
const PAD: usize = 16;

/// Where the interleaved kernel adds: `slots` per copy (the groups, then one slot for the rows a
/// filter dropped, never read), and the offset of each lane's copy (after [`PAD`] elements).
#[derive(Clone, Copy)]
struct Lanes {
    slots: usize,
    copies: usize,
    off: [usize; 4],
}

impl Lanes {
    fn new(groups: usize) -> Option<Lanes> {
        let slots = groups.checked_add(1)?;
        Some(if slots <= COPIES_MAX_SLOTS {
            Lanes {
                slots,
                copies: COPIES,
                off: [
                    PAD,
                    PAD + slots,
                    PAD + slots.saturating_mul(2),
                    PAD + slots.saturating_mul(3),
                ],
            }
        } else {
            Lanes {
                slots,
                copies: 1,
                off: [PAD; 4],
            }
        })
    }

    fn len(&self) -> usize {
        self.slots.saturating_mul(self.copies).saturating_add(2 * PAD)
    }
}

/// A worker's sums of one column.
struct ColAcc<T> {
    /// Per copy and slot.
    sums: Vec<T>,
    /// Per slot: counts added outside the interleaved kernel (blocks of one group without a
    /// filter, blocks in runs of groups).
    direct: Vec<u64>,
    /// Per copy and slot: rows of the interleaved kernel without a value (blank). The column's
    /// count there is the worker's rows of the group (counted once for all the columns) minus
    /// these.
    blanks: Vec<u64>,
}

impl<T: Num> ColAcc<T> {
    fn new(l: &Lanes) -> ColAcc<T> {
        ColAcc {
            sums: vec![T::default(); l.len()],
            direct: vec![0; l.slots.saturating_add(2 * PAD)],
            blanks: vec![0; l.len()],
        }
    }

    /// Adds this worker's sums and counts per group to `sums` / `counts` (`rows`: the worker's
    /// rows of the interleaved kernel per copy and slot).
    fn fold(&self, l: &Lanes, rows: &[u64], sums: &mut [i128], counts: &mut [u64]) {
        for (g, (s, n)) in sums.iter_mut().zip(counts.iter_mut()).enumerate() {
            let mut sum = 0i128;
            let mut cnt = self.direct.get(PAD.saturating_add(g)).copied().unwrap_or(0);
            let mut blank = 0u64;
            for j in 0..l.copies {
                let at = j.saturating_mul(l.slots).saturating_add(PAD).saturating_add(g);
                sum = sum.saturating_add(self.sums.get(at).map_or(0, |x| x.wide()));
                cnt = cnt.saturating_add(rows.get(at).copied().unwrap_or(0));
                blank = blank.saturating_add(self.blanks.get(at).copied().unwrap_or(0));
            }
            *s = s.saturating_add(sum);
            *n = n.saturating_add(cnt.saturating_sub(blank));
        }
    }
}

#[inline(always)]
fn bump<T: Num>(s: &mut [T], at: usize, v: T) {
    if let Some(x) = s.get_mut(at) {
        *x += v;
    }
}

/// The interleaved kernel: `sums[group of row i] += value(ids[i])`, row `i` on lane `i % 4`; with
/// `ROWS`, also `rows[group] += 1` in the same loop (a block's rows are counted once, shared by
/// every column).
#[inline]
fn lanes_sum<T: Num, const ROWS: bool>(
    gs: &[u32],
    ids: &[u32],
    l: &Lanes,
    sums: &mut [T],
    rows: &mut [u64],
    value: impl Fn(u32) -> T,
    grp: impl Fn(u32) -> usize + Copy,
) {
    let n = gs.len().min(ids.len());
    // the same length for both, so that one bounds check covers the two adds
    let m = if ROWS {
        sums.len().min(rows.len())
    } else {
        sums.len()
    };
    let (Some(sums), Some(rows)) = (sums.get_mut(..m), rows.get_mut(..if ROWS { m } else { 0 })) else {
        return;
    };
    let (Some(gs), Some(ids)) = (gs.get(..n), ids.get(..n)) else {
        return;
    };
    let (gc, gr) = gs.as_chunks::<4>();
    let (ic, ir) = ids.as_chunks::<4>();
    let [o0, o1, o2, o3] = l.off;
    for (g, i) in gc.iter().zip(ic) {
        let [g0, g1, g2, g3] = g.map(grp);
        let [i0, i1, i2, i3] = *i;
        let (a0, a1, a2, a3) = (
            g0.wrapping_add(o0),
            g1.wrapping_add(o1),
            g2.wrapping_add(o2),
            g3.wrapping_add(o3),
        );
        for (a, i) in [(a0, i0), (a1, i1), (a2, i2), (a3, i3)] {
            if ROWS {
                if let (Some(s), Some(c)) = (sums.get_mut(a), rows.get_mut(a)) {
                    *s += value(i);
                    *c += 1;
                }
            } else {
                bump(sums, a, value(i));
            }
        }
    }
    for (g, i) in gr.iter().zip(ir) {
        let a = grp(*g).wrapping_add(o0);
        bump(sums, a, value(*i));
        if ROWS {
            bump(rows, a, 1);
        }
    }
}

/// The interleaved kernel for ids that may be blanks: `value` gives the value (0 for a blank)
/// and whether there is one; a blank row adds one to `blanks`.
#[inline]
fn lanes_sum_blanks<T: Num>(
    gs: &[u32],
    ids: &[u32],
    l: &Lanes,
    sums: &mut [T],
    blanks: &mut [u64],
    value: impl Fn(u32) -> (T, bool),
    grp: impl Fn(u32) -> usize + Copy,
) {
    let n = gs.len().min(ids.len());
    let (Some(gs), Some(ids)) = (gs.get(..n), ids.get(..n)) else {
        return;
    };
    let (gc, gr) = gs.as_chunks::<4>();
    let (ic, ir) = ids.as_chunks::<4>();
    let [o0, o1, o2, o3] = l.off;
    for (g, i) in gc.iter().zip(ic) {
        let [g0, g1, g2, g3] = g.map(grp);
        for (gj, oj, ij) in [(g0, o0, i[0]), (g1, o1, i[1]), (g2, o2, i[2]), (g3, o3, i[3])] {
            let a = gj.wrapping_add(oj);
            let (v, k) = value(ij);
            bump(sums, a, v);
            bump(blanks, a, u64::from(!k));
        }
    }
    for (g, i) in gr.iter().zip(ir) {
        let a = grp(*g).wrapping_add(o0);
        let (v, k) = value(*i);
        bump(sums, a, v);
        bump(blanks, a, u64::from(!k));
    }
}

/// The interleaved kernel adding the same value on every row.
#[inline]
fn lanes_const<T: Num>(gs: &[u32], l: &Lanes, out: &mut [T], v: T, grp: impl Fn(u32) -> usize + Copy) {
    let (gc, gr) = gs.as_chunks::<4>();
    let [o0, o1, o2, o3] = l.off;
    for g in gc {
        let [g0, g1, g2, g3] = g.map(grp);
        bump(out, g0.wrapping_add(o0), v);
        bump(out, g1.wrapping_add(o1), v);
        bump(out, g2.wrapping_add(o2), v);
        bump(out, g3.wrapping_add(o3), v);
    }
    for g in gr {
        bump(out, grp(*g).wrapping_add(o0), v);
    }
}

/// [`lanes_const`] that also counts the rows per slot in the same loop (a column in one run that
/// counts the block's rows for every column: one pass instead of two).
#[inline]
fn lanes_const_rows<T: Num>(
    gs: &[u32],
    l: &Lanes,
    sums: &mut [T],
    rows: &mut [u64],
    v: T,
    grp: impl Fn(u32) -> usize + Copy,
) {
    // the same length for both, so that one bounds check covers the two adds
    let m = sums.len().min(rows.len());
    let (Some(sums), Some(rows)) = (sums.get_mut(..m), rows.get_mut(..m)) else {
        return;
    };
    let (gc, gr) = gs.as_chunks::<4>();
    let [o0, o1, o2, o3] = l.off;
    for g in gc {
        let [g0, g1, g2, g3] = g.map(grp);
        for a in [
            g0.wrapping_add(o0),
            g1.wrapping_add(o1),
            g2.wrapping_add(o2),
            g3.wrapping_add(o3),
        ] {
            if let (Some(s), Some(c)) = (sums.get_mut(a), rows.get_mut(a)) {
                *s += v;
                *c += 1;
            }
        }
    }
    for g in gr {
        let a = grp(*g).wrapping_add(o0);
        if let (Some(s), Some(c)) = (sums.get_mut(a), rows.get_mut(a)) {
            *s += v;
            *c += 1;
        }
    }
}

/// A block of rows with a group per row (the dropped rows in the dummy slot), for the
/// interleaved kernel.
struct KernelBlock<'b> {
    at: usize,
    /// Per row, its group, or (with `map`) the data id of the only grouping column.
    gs: &'b [u32],
    /// The only grouping column's id → group table and the group of ids past it: the groups
    /// are looked up in the kernel's loop instead of being written out first.
    map: Option<(&'b [u32], u32)>,
    lanes: &'b Lanes,
    /// The column's id when the block lies in one run of it.
    run: Option<u32>,
    /// The column's ids in the block hold no blank (one run of an id with a value, or exact
    /// segment statistics).
    free: bool,
    /// A column in one run of one id with a value is left to [`run_delta`].
    defer: bool,
}

/// Runs `$body` with `$grp` bound to the block's row → group function (the identity, or the
/// lookup of the only grouping column's id), each its own instance of the kernel's loops.
macro_rules! with_groups {
    ($map:expr, $grp:ident, $body:expr) => {
        match $map {
            None => {
                let $grp = |g: u32| g as usize;
                $body
            }
            Some((table, last)) => {
                let $grp = move |id: u32| table.get(id as usize).copied().unwrap_or(last) as usize;
                $body
            }
        }
    };
}

/// One column over a [`KernelBlock`]: `value` reads a non-blank id, `checked` any id (its value,
/// 0 for a blank, and whether it has one). `rows`: count the block's rows in the same loop when
/// the column can (no blank); returns whether it did.
#[inline]
fn kernel_block<T: Num>(
    a: &mut ColAcc<T>,
    col: &Packed,
    b: &KernelBlock<'_>,
    rows: Option<&mut [u64]>,
    ids: &mut Vec<u32>,
    value: impl Fn(u32) -> T,
    checked: impl Fn(u32) -> (T, bool),
) -> bool {
    let len = b.gs.len();
    // the block in one run of one id: its value on every row (and the rows counted in the same
    // loop when asked)
    if let Some(id) = b.run {
        match (checked(id), rows) {
            ((_, true), _) if b.defer => {}
            ((v, true), Some(r)) => {
                with_groups!(
                    b.map,
                    grp,
                    lanes_const_rows(b.gs, b.lanes, &mut a.sums, r, v, grp)
                );
                return true;
            }
            ((v, true), None) => with_groups!(b.map, grp, lanes_const(b.gs, b.lanes, &mut a.sums, v, grp)),
            ((_, false), _) => with_groups!(b.map, grp, lanes_const(b.gs, b.lanes, &mut a.blanks, 1u64, grp)),
        }
        return false;
    }
    ids.resize(len, 0);
    col.fill_ids(b.at, ids);
    if !b.free {
        with_groups!(
            b.map,
            grp,
            lanes_sum_blanks(b.gs, ids, b.lanes, &mut a.sums, &mut a.blanks, checked, grp)
        );
        return false;
    }
    match rows {
        Some(r) => {
            with_groups!(
                b.map,
                grp,
                lanes_sum::<T, true>(b.gs, ids, b.lanes, &mut a.sums, r, &value, grp)
            );
            true
        }
        None => {
            with_groups!(
                b.map,
                grp,
                lanes_sum::<T, false>(b.gs, ids, b.lanes, &mut a.sums, &mut [], &value, grp)
            );
            false
        }
    }
}

/// Whether the columns in one run over a block of `len` rows (`runs` of them) are left to
/// [`run_delta`] (`lanes`: accumulator elements; `counter`: another column counts the block's
/// rows in its own loop). Without it, each such column adds once per row, except one that can
/// count the rows in the same loop when no other column does ([`lanes_const_rows`]); with it,
/// the counts are copied before the block and each column then walks every element. A cost in
/// element steps, so that one run column by 1,001 groups (Q16: ~1,000 elements, the rows counted
/// by no other column) keeps one fused pass, while two sums by 36 groups (Q04) take deltas.
fn defer_runs(lanes: usize, len: usize, runs: usize, counter: bool) -> bool {
    let per_row = runs.saturating_sub(usize::from(!counter));
    runs > 0 && lanes.saturating_mul(runs.saturating_add(1)) < len.saturating_mul(per_row)
}

/// A column whose block (`d`: first row, end, the worker's rows per copy and slot after and
/// before the block) lies in one run of one id with a value: that value times the block's rows
/// of each slot.
fn run_delta<T: Num>(
    a: &mut ColAcc<T>,
    col: &Packed,
    d: (usize, usize, &[u64], &[u64]),
    checked: impl Fn(u32) -> (T, bool),
) {
    let (at, end, rows, before) = d;
    if let Some(id) = col.run_through(at, end)
        && let (v, true) = checked(id)
    {
        for (s, (r, p)) in a.sums.iter_mut().zip(rows.iter().zip(before)) {
            if *r != *p {
                *s += v.times(r.saturating_sub(*p));
            }
        }
    }
}

/// Whether a column's data id has a value (not a blank).
fn has_value(vals: &IdValues<'_>, id: u32) -> bool {
    match vals {
        IdValues::Affine { first } => id >= *first,
        IdValues::Table { valid, .. } => valid.get(id as usize).copied().unwrap_or(false),
    }
}

/// A value-encoded column's id: its value (the id from `first` on) and whether it has one.
fn affine_u64(first: u32) -> impl Fn(u32) -> (u64, bool) {
    move |id| {
        let k = id >= first;
        (u64::from(id) * u64::from(k), k)
    }
}

fn affine_i128(first: u32) -> impl Fn(u32) -> (i128, bool) {
    move |id| {
        let k = id >= first;
        (i128::from(id) * i128::from(k), k)
    }
}

/// A column's id read through a table: its value and whether it has one.
fn table_i64<'a>(values: &'a [i64], valid: &'a [bool]) -> impl Fn(u32) -> (i64, bool) + 'a {
    move |id| {
        let k = valid.get(id as usize).copied().unwrap_or(false);
        (values.get(id as usize).copied().unwrap_or(0) * i64::from(k), k)
    }
}

fn table_i128<'a>(values: &'a [i64], valid: &'a [bool]) -> impl Fn(u32) -> (i128, bool) + 'a {
    move |id| {
        let k = valid.get(id as usize).copied().unwrap_or(false);
        (
            i128::from(values.get(id as usize).copied().unwrap_or(0)) * i128::from(k),
            k,
        )
    }
}

/// A block of one group without a filter, or in runs of groups: (first row, rows, grouping,
/// selected rows when not all, slots of a copy).
struct OtherBlock<'b> {
    at: usize,
    len: usize,
    grouping: Grouping<'b>,
    ok: Option<&'b [bool]>,
    slots: usize,
}

/// One column over an [`OtherBlock`], into the first copy and the direct counts.
#[inline]
fn other_block<T: Num>(
    a: &mut ColAcc<T>,
    col: &Packed,
    b: &OtherBlock<'_>,
    ids: &mut Vec<u32>,
    checked: impl Fn(u32) -> (T, bool),
) {
    let ColAcc { sums, direct, .. } = a;
    let s0: &mut [T] = sums.get_mut(PAD..PAD.saturating_add(b.slots)).unwrap_or(&mut []);
    let direct: &mut [u64] = direct.get_mut(PAD..).unwrap_or(&mut []);
    // the block in one run of one id: its value times the selected rows
    if let Some(id) = col.run_through(b.at, b.at.saturating_add(b.len)) {
        if let (v, true) = checked(id) {
            add_run(b.len, b.grouping, b.ok, s0, direct, |n| v.times(n));
        }
        return;
    }
    ids.resize(b.len, 0);
    col.fill_ids(b.at, ids);
    let val = |id: u32| {
        let (v, k) = checked(id);
        (v, u64::from(k))
    };
    match b.grouping {
        Grouping::Runs(rs) => add_ids_runs(ids, rs, b.ok, s0, direct, val),
        Grouping::One(g0) => add_ids(ids, g0, b.ok, s0, direct, val),
    }
}

/// Adds `val` of each selected row's id to group `g0`.
#[inline]
fn add_ids<T: Copy + std::ops::AddAssign>(
    ids: &[u32],
    g0: usize,
    ok: Option<&[bool]>,
    sums: &mut [T],
    counts: &mut [u64],
    val: impl Fn(u32) -> (T, u64),
) {
    let (Some(s0), Some(n0)) = (sums.get_mut(g0), counts.get_mut(g0)) else {
        return;
    };
    match ok {
        None => {
            for id in ids {
                let (v, k) = val(*id);
                *s0 += v;
                *n0 += k;
            }
        }
        Some(ok) => {
            for (id, o) in ids.iter().zip(ok) {
                if *o {
                    let (v, k) = val(*id);
                    *s0 += v;
                    *n0 += k;
                }
            }
        }
    }
}

/// Adds `val` of each selected row's id to the group of its run (`runs`: (offset, rows, group)),
/// one tight loop per run.
#[inline]
fn add_ids_runs<T: Copy + std::ops::AddAssign + Default>(
    ids: &[u32],
    runs: &[(u32, u32, u32)],
    ok: Option<&[bool]>,
    sums: &mut [T],
    counts: &mut [u64],
    val: impl Fn(u32) -> (T, u64),
) {
    for (off, len, g) in runs {
        let (off, end) = (*off as usize, *off as usize + *len as usize);
        let Some(part) = ids.get(off..end) else {
            continue;
        };
        let (mut s, mut n) = (T::default(), 0u64);
        match ok.and_then(|o| o.get(off..end)) {
            None => {
                for id in part {
                    let (v, k) = val(*id);
                    s += v;
                    n += k;
                }
            }
            Some(o) => {
                for (id, x) in part.iter().zip(o) {
                    if *x {
                        let (v, k) = val(*id);
                        s += v;
                        n += k;
                    }
                }
            }
        }
        if let (Some(a), Some(b)) = (sums.get_mut(*g as usize), counts.get_mut(*g as usize)) {
            *a += s;
            *b += n;
        }
    }
}

/// The groups of a block's rows outside the interleaved kernel: one for all, or one per run of
/// rows.
#[derive(Clone, Copy)]
enum Grouping<'a> {
    One(usize),
    Runs(&'a [(u32, u32, u32)]),
}

/// Adds the value of every selected row of a block that lies in one run of one id (`times(n)`:
/// `n` times that value).
#[inline]
fn add_run<T: Copy + std::ops::AddAssign>(
    len: usize,
    grouping: Grouping<'_>,
    ok: Option<&[bool]>,
    sums: &mut [T],
    counts: &mut [u64],
    times: impl Fn(u64) -> T,
) {
    match grouping {
        Grouping::Runs(runs) => {
            for (off, rl, g) in runs {
                let (off, end) = (*off as usize, *off as usize + *rl as usize);
                let n = ok.map_or(end - off, |o| {
                    o.get(off..end).map_or(0, |x| x.iter().filter(|b| **b).count())
                }) as u64;
                if let (Some(a), Some(b)) = (sums.get_mut(*g as usize), counts.get_mut(*g as usize)) {
                    *a += times(n);
                    *b += n;
                }
            }
        }
        Grouping::One(g0) => {
            let n = ok.map_or(len, |o| o.iter().filter(|x| **x).count()) as u64;
            if let (Some(a), Some(b)) = (sums.get_mut(g0), counts.get_mut(g0)) {
                *a += times(n);
                *b += n;
            }
        }
    }
}

/// [`sum_by_id`] of several columns of one table over the same selection and grouping, in one
/// pass (the selection and the groups computed once per block; a block of a column that lies in
/// one run of one id is added without reading its rows). `None`: a worker failed.
///
/// A block whose rows have their own groups (or one group with a filter) goes through the
/// **interleaved kernel**: the rows a filter dropped are sent to a dummy group slot instead of
/// being branched around (no mispredictions at ~50 % selectivity); each row adds to one of 4
/// copies of the accumulators (row `i` to copy `i % 4`, with up to 4,096 groups), so rows of one
/// group in a row do not chain through memory; the rows per group are counted once per block,
/// in the loop of the first column whose ids there hold no blank (exact segment statistics),
/// and shared by every column, which then only counts its blanks. Integer sums: the result does
/// not depend on the order of the adds.
pub fn sum_by_id_multi(
    sel: &Selection<'_>,
    key: &Key<'_>,
    cols: &[(&Packed, IdValues<'_>)],
    threads: usize,
) -> Option<Vec<GroupSums>> {
    let plan = Plan::new(sel, key);
    let groups = key.groups();
    let grouped = !plan.keys.is_empty();
    let filtered = plan.filtered();
    let empty = || -> Vec<GroupSums> {
        cols.iter()
            .map(|_| (vec![0i128; groups], vec![0u64; groups]))
            .collect()
    };
    if plan.none {
        return Some(empty());
    }
    // no filter, no grouping, value-encoded columns: their segments' statistics
    if !filtered && !grouped {
        let whole: Option<Vec<GroupSums>> = cols
            .iter()
            .map(|(col, v)| match v {
                IdValues::Affine { first } => col
                    .id_sum(0, sel.rows, *first)
                    .map(|(s, n)| (vec![i128::from(s)], vec![n])),
                IdValues::Table { .. } => None,
            })
            .collect();
        if let Some(w) = whole
            && groups == 1
        {
            return Some(w);
        }
    }
    let rows = u128::try_from(sel.rows).unwrap_or(u128::MAX);
    // sums of ids in a u64 below 2^32 rows (ids are 32-bit)
    let ids_fit = rows <= u128::from(u32::MAX);
    // values whose sum over every row stays in an i64
    let small: Vec<bool> = cols
        .iter()
        .map(|(_, v)| match v {
            IdValues::Affine { .. } => false,
            IdValues::Table { values, .. } => {
                let max = values.iter().map(|x| x.unsigned_abs()).max().unwrap_or(0);
                u128::from(max).saturating_mul(rows) < (1u128 << 62)
            }
        })
        .collect();
    let lanes = Lanes::new(groups)?;
    let dummy = u32::try_from(groups).ok()?;
    // per column, how a block is known to hold no blank: its ids from `first` on and below a
    // limit (a table whose only ids without a value are the blank ids 0, 1 and 2)
    let clean: Vec<Option<(u32, usize)>> = cols
        .iter()
        .map(|(_, v)| match v {
            IdValues::Affine { first } => Some((*first, usize::MAX)),
            IdValues::Table { values, valid } => valid
                .iter()
                .skip(3)
                .all(|x| *x)
                .then_some((3, valid.len().min(values.len()))),
        })
        .collect();
    let parts = || -> Vec<Part> {
        cols.iter()
            .zip(&small)
            .map(|((_, v), sm)| match v {
                IdValues::Affine { .. } if ids_fit => Part::Ids(ColAcc::new(&lanes)),
                IdValues::Table { .. } if *sm => Part::Small(ColAcc::new(&lanes)),
                _ => Part::Wide(ColAcc::new(&lanes)),
            })
            .collect()
    };
    type State = (Vec<Part>, Scratch, Vec<u32>, Vec<u64>, Vec<u64>);
    let states = morsels(
        sel.rows,
        threads,
        || {
            (
                parts(),
                Scratch::default(),
                Vec::new(),
                vec![0u64; lanes.len()],
                Vec::new(),
            )
        },
        |(acc, scr, ids, rows, snap): &mut State, start, end| {
            let mut at = start;
            while at < end {
                let len = (end - at).min(BLOCK);
                let block_at = at;
                at = at.saturating_add(len);
                // a block in runs of allowed ids is not filtered, a block in runs of the keys'
                // columns has one group
                let mut part = filtered;
                if filtered {
                    scr.ok.resize(len, false);
                    if plan.block(block_at, &mut scr.ok, &mut scr.preds) {
                        part = false;
                    } else if !scr.ok.iter().any(|x| *x) {
                        continue;
                    }
                }
                let mut g0 = 0usize;
                let mut many = grouped;
                let mut in_runs = false;
                let mut map = None;
                let block_end = block_at.saturating_add(len);
                if grouped && plan.group_runs(block_at, len, &mut scr.runs) {
                    many = false;
                    in_runs = true;
                    if let [(_, _, g)] = scr.runs.as_slice() {
                        in_runs = false;
                        g0 = *g as usize;
                    }
                } else if !part
                    && let [(Read::Ids(pk), table, last)] = plan.keys.as_slice()
                    && pk.run_through(block_at, block_end).is_none()
                {
                    // one grouping column: its ids, looked up in the kernel's loop
                    scr.groups.resize(len, 0);
                    pk.fill_ids(block_at, &mut scr.groups);
                    map = Some((table.as_slice(), *last));
                } else if grouped && plan.groups(block_at, len, &mut scr.keys, &mut scr.groups) {
                    many = false;
                    g0 = scr.groups.first().copied().unwrap_or(0) as usize;
                }
                let ok = part.then_some(scr.ok.as_slice());
                if many || (ok.is_some() && !in_runs) {
                    // the interleaved kernel: a group per row, the dropped rows to the dummy slot
                    if !many {
                        scr.groups.clear();
                        scr.groups.resize(len, u32::try_from(g0).unwrap_or(dummy));
                    }
                    if let Some(ok) = ok {
                        for (g, o) in scr.groups.iter_mut().zip(ok) {
                            *g = if *o { *g } else { dummy };
                        }
                    }
                    let gs = scr.groups.as_slice();
                    // per column: its id when the block lies in one run of it, and whether its
                    // ids there hold no blank
                    scr.cols.clear();
                    scr.cols.extend(cols.iter().zip(&clean).map(|((col, vals), cl)| {
                        let run = col.run_through(block_at, block_end);
                        let free = match run {
                            Some(id) => has_value(vals, id),
                            None => cl.is_some_and(|(first, limit)| {
                                col.ids_within(block_at, block_end, first, limit)
                            }),
                        };
                        (run, free)
                    }));
                    // columns in one run of one id with a value: their value times the block's
                    // rows per group, from the counts, when that costs fewer steps
                    let runs = scr.cols.iter().filter(|(r, f)| r.is_some() && *f).count();
                    let counter = scr.cols.iter().any(|(r, f)| r.is_none() && *f);
                    let defer = defer_runs(lanes.len(), len, runs, counter);
                    if defer {
                        snap.clear();
                        snap.extend_from_slice(rows);
                    }
                    let mut counted = false;
                    for (((col, vals), state), (run, free)) in
                        cols.iter().zip(acc.iter_mut()).zip(scr.cols.iter().copied())
                    {
                        let b = KernelBlock {
                            at: block_at,
                            gs,
                            map,
                            lanes: &lanes,
                            run,
                            free,
                            defer,
                        };
                        let count = (b.free && !counted).then_some(rows.as_mut_slice());
                        counted |= match (vals, state) {
                            (IdValues::Affine { first }, Part::Ids(a)) => {
                                kernel_block(a, col, &b, count, ids, u64::from, affine_u64(*first))
                            }
                            (IdValues::Affine { first }, Part::Wide(a)) => {
                                kernel_block(a, col, &b, count, ids, i128::from, affine_i128(*first))
                            }
                            (IdValues::Table { values, valid }, Part::Small(a)) => kernel_block(
                                a,
                                col,
                                &b,
                                count,
                                ids,
                                |id| values.get(id as usize).copied().unwrap_or(0),
                                table_i64(values, valid),
                            ),
                            (IdValues::Table { values, valid }, Part::Wide(a)) => kernel_block(
                                a,
                                col,
                                &b,
                                count,
                                ids,
                                |id| i128::from(values.get(id as usize).copied().unwrap_or(0)),
                                table_i128(values, valid),
                            ),
                            _ => false,
                        };
                    }
                    if !counted {
                        with_groups!(map, grp, lanes_const(gs, &lanes, rows, 1u64, grp));
                    }
                    if defer {
                        let d = (block_at, block_end, rows.as_slice(), snap.as_slice());
                        for ((col, vals), state) in cols.iter().zip(acc.iter_mut()) {
                            match (vals, state) {
                                (IdValues::Affine { first }, Part::Ids(a)) => {
                                    run_delta(a, col, d, affine_u64(*first))
                                }
                                (IdValues::Affine { first }, Part::Wide(a)) => {
                                    run_delta(a, col, d, affine_i128(*first))
                                }
                                (IdValues::Table { values, valid }, Part::Small(a)) => {
                                    run_delta(a, col, d, table_i64(values, valid));
                                }
                                (IdValues::Table { values, valid }, Part::Wide(a)) => {
                                    run_delta(a, col, d, table_i128(values, valid));
                                }
                                _ => {}
                            }
                        }
                    }
                    continue;
                }
                let b = OtherBlock {
                    at: block_at,
                    len,
                    grouping: if in_runs {
                        Grouping::Runs(scr.runs.as_slice())
                    } else {
                        Grouping::One(g0)
                    },
                    ok,
                    slots: lanes.slots,
                };
                for ((col, vals), state) in cols.iter().zip(acc.iter_mut()) {
                    match (vals, state) {
                        (IdValues::Affine { first }, Part::Ids(a)) => {
                            other_block(a, col, &b, ids, affine_u64(*first))
                        }
                        (IdValues::Affine { first }, Part::Wide(a)) => {
                            other_block(a, col, &b, ids, affine_i128(*first))
                        }
                        (IdValues::Table { values, valid }, Part::Small(a)) => {
                            other_block(a, col, &b, ids, table_i64(values, valid));
                        }
                        (IdValues::Table { values, valid }, Part::Wide(a)) => {
                            other_block(a, col, &b, ids, table_i128(values, valid));
                        }
                        _ => {}
                    }
                }
            }
        },
    )?;
    let mut out = empty();
    for (acc, _, _, rows, _) in &states {
        for ((s, n), part) in out.iter_mut().zip(acc) {
            match part {
                Part::Ids(a) => a.fold(&lanes, rows, s, n),
                Part::Small(a) => a.fold(&lanes, rows, s, n),
                Part::Wide(a) => a.fold(&lanes, rows, s, n),
            }
        }
    }
    Some(out)
}

/// [`scan`] whose result does not depend on how the rows are split among threads: each morsel
/// of [`MORSEL`] rows (always the same, whatever the threads) fills its own sink, and the sinks
/// are merged in row order. For sinks whose merge is not associative (sums of doubles): the
/// result is the same bits on 1 or 20 threads. `None`: a worker failed.
pub fn scan_ordered<S: Sink>(
    sel: &Selection<'_>,
    key: &Key<'_>,
    values: Option<Col<'_>>,
    threads: usize,
    make: impl Fn() -> S + Sync,
) -> Option<S> {
    scan_ordered_in::<S, false>(sel, key, values, threads, 0, usize::MAX, make)
}

/// [`scan_ordered`] keeping only the groups `lo..lo + len` of `key`, numbered from 0 in the
/// sinks `make` gives (`docs/beyond-ram.md`, "Groupings by ranges"). The morsels are the same
/// and merged in the same order, and a group's state in a morsel only depends on its own rows,
/// so each group's result is the same bits as [`scan_ordered`]'s.
pub fn scan_ordered_range<S: Sink>(
    sel: &Selection<'_>,
    key: &Key<'_>,
    values: Option<Col<'_>>,
    threads: usize,
    lo: usize,
    len: usize,
    make: impl Fn() -> S + Sync,
) -> Option<S> {
    let len = len.min(key.groups().saturating_sub(lo));
    scan_ordered_in::<S, true>(sel, key, values, threads, lo, len, make)
}

fn scan_ordered_in<S: Sink, const RANGE: bool>(
    sel: &Selection<'_>,
    key: &Key<'_>,
    values: Option<Col<'_>>,
    threads: usize,
    lo: usize,
    groups: usize,
    make: impl Fn() -> S + Sync,
) -> Option<S> {
    let plan = Plan::new(sel, key);
    let states = morsels(
        sel.rows,
        threads,
        || (Vec::new(), Scratch::default()),
        |(done, scr): &mut (Vec<(usize, S)>, Scratch), start, end| {
            let mut sink = make();
            let mut at = start;
            while at < end {
                let len = (end - at).min(BLOCK);
                each_selected(&plan, values, at, len, scr, |row, g, v| {
                    if let Some(g) = in_range::<RANGE>(g, lo, groups) {
                        sink.add(g, v, row);
                    }
                });
                at = at.saturating_add(len);
            }
            done.push((start, sink));
        },
    )?;
    let mut all: Vec<(usize, S)> = states.into_iter().flat_map(|(d, _)| d).collect();
    all.sort_by_key(|(start, _)| *start);
    let mut out: Option<S> = None;
    for (_, s) in all {
        match &mut out {
            None => out = Some(s),
            Some(o) => o.merge(s),
        }
    }
    out.or_else(|| Some(make()))
}

/// Morsels of a scan of `rows` rows (what [`scan_ordered`] keeps one sink for).
pub fn morsel_count(rows: usize) -> usize {
    rows.div_ceil(MORSEL).max(1)
}

/// [`scan`] over a list of rows (`rows`, in order) instead of a selection: each listed row is fed
/// to the sink with its group and `values` code. Close rows are read a block at a time.
/// `None`: a worker failed.
pub fn scan_list<S: Sink>(
    rows: &[u32],
    key: &Key<'_>,
    values: Option<Col<'_>>,
    threads: usize,
    make: impl Fn() -> S + Sync,
) -> Option<S> {
    let all = Selection::all(usize::MAX);
    let plan = Plan::new(&all, key);
    let states = morsels(
        rows.len(),
        threads,
        || (make(), Scratch::default()),
        |(sink, scr): &mut (S, Scratch), start, end| {
            let Some(part) = rows.get(start..end) else {
                return;
            };
            for chunk in part.chunks(BLOCK) {
                let (Some(first), Some(last)) = (chunk.iter().min(), chunk.iter().max()) else {
                    continue;
                };
                let (first, last) = (*first as usize, *last as usize);
                let window = last.saturating_sub(first).saturating_add(1);
                if last >= first && window <= BLOCK.saturating_mul(4) {
                    // ascending rows close together: decode their window once
                    let mut by_row: Vec<(usize, u32)> = Vec::with_capacity(window);
                    let mut at = first;
                    while at <= last {
                        let len = (last + 1 - at).min(BLOCK);
                        each_selected(&plan, values, at, len, scr, |_, g, v| by_row.push((g, v)));
                        at = at.saturating_add(len);
                    }
                    for r in chunk {
                        let i = (*r as usize).wrapping_sub(first);
                        let (g, v) = by_row.get(i).copied().unwrap_or((0, 0));
                        sink.add(g, v, *r as usize);
                    }
                } else {
                    for r in chunk {
                        let row = *r as usize;
                        let v = values.map_or(0, |v| v.get(row));
                        sink.add(plan.group_of(row), v, row);
                    }
                }
            }
        },
    )?;
    let mut out: Option<S> = None;
    for (s, _) in states {
        match &mut out {
            None => out = Some(s),
            Some(o) => o.merge(s),
        }
    }
    out
}

/// The rows of `rows` (in order) whose group `keep` marks: [`scan_list`]'s reading, in parallel
/// by chunks.
pub fn filter_list(rows: &[u32], key: &Key<'_>, keep: &[bool], threads: usize) -> Vec<u32> {
    let all_rows = Selection::all(usize::MAX);
    let plan = Plan::new(&all_rows, key);
    let plan = &plan;
    let threads = if too_small(rows.len()) { 1 } else { threads.max(1) };
    let chunk = rows.len().div_ceil(threads).max(1);
    let one = |part: &[u32]| {
        let mut out = Vec::new();
        let mut scr = Scratch::default();
        for chunk in part.chunks(BLOCK) {
            let (Some(first), Some(last)) = (chunk.iter().min(), chunk.iter().max()) else {
                continue;
            };
            let (first, last) = (*first as usize, *last as usize);
            let window = last.saturating_sub(first).saturating_add(1);
            if window <= BLOCK.saturating_mul(4) {
                let mut by_row: Vec<usize> = Vec::with_capacity(window);
                let mut at = first;
                while at <= last {
                    let len = (last + 1 - at).min(BLOCK);
                    each_selected(plan, None, at, len, &mut scr, |_, g, _| by_row.push(g));
                    at = at.saturating_add(len);
                }
                for r in chunk {
                    let g = by_row
                        .get((*r as usize).wrapping_sub(first))
                        .copied()
                        .unwrap_or(usize::MAX);
                    if keep.get(g).copied().unwrap_or(false) {
                        out.push(*r);
                    }
                }
            } else {
                for r in chunk {
                    if keep.get(plan.group_of(*r as usize)).copied().unwrap_or(false) {
                        out.push(*r);
                    }
                }
            }
        }
        out
    };
    if threads == 1 {
        return one(rows);
    }
    let chunks: Vec<&[u32]> = rows.chunks(chunk).collect();
    // a failed worker: the whole list again on this thread (never a partial answer)
    pool::map(chunks.len(), |i| chunks.get(i).map_or_else(Vec::new, |p| one(p)))
        .map_or_else(|| one(rows), |parts| parts.concat())
}

/// Rows per group and the first row of each (`u32::MAX`: none).
#[derive(Debug, Clone)]
pub struct Tally {
    pub counts: Vec<u64>,
    pub first: Vec<u32>,
}

impl Tally {
    pub fn new(groups: usize) -> Self {
        Tally {
            counts: vec![0; groups],
            first: vec![u32::MAX; groups],
        }
    }
}

impl Sink for Tally {
    const NEEDS_ROW: bool = true;

    #[inline]
    fn add(&mut self, group: usize, _value: u32, row: usize) {
        if let (Some(n), Some(f)) = (self.counts.get_mut(group), self.first.get_mut(group)) {
            *n = n.wrapping_add(1);
            *f = (*f).min(u32::try_from(row).unwrap_or(u32::MAX));
        }
    }

    fn merge(&mut self, other: Self) {
        for (a, b) in self.counts.iter_mut().zip(other.counts) {
            *a = a.wrapping_add(b);
        }
        for (a, b) in self.first.iter_mut().zip(other.first) {
            *a = (*a).min(b);
        }
    }
}

/// The selected rows as a mask (`rows` slots), filled in parallel.
pub fn mask(sel: &Selection<'_>, threads: usize) -> Vec<bool> {
    let n = sel.rows;
    let mut out = vec![false; n];
    if sel.none {
        return out;
    }
    if !sel.filtered() {
        out.fill(true);
        return out;
    }
    let threads = cheap_threads(n, threads);
    let chunk = n.div_ceil(threads).max(1).next_multiple_of(BLOCK);
    let plan = Plan::new(sel, &Key::single());
    let sel = &plan;
    let work = |i: usize, part: &mut [bool]| {
        let start = i.saturating_mul(chunk);
        let mut bufs = Vec::new();
        for (j, b) in part.chunks_mut(BLOCK).enumerate() {
            sel.block(start.saturating_add(j.saturating_mul(BLOCK)), b, &mut bufs);
        }
    };
    if threads == 1 {
        work(0, &mut out);
    } else if !pool::each(out.chunks_mut(chunk).collect(), work) {
        // a failed worker: the whole mask again on this thread (never a partial answer)
        work(0, &mut out);
    }
    out
}

/// The selected rows, in order, filled in parallel.
pub fn rows(sel: &Selection<'_>, threads: usize) -> Vec<u32> {
    let n = sel.rows;
    if sel.none {
        return Vec::new();
    }
    if !sel.filtered() {
        return (0..n).filter_map(|r| u32::try_from(r).ok()).collect();
    }
    let threads = cheap_threads(n, threads);
    let chunk = n.div_ceil(threads).max(1).next_multiple_of(BLOCK);
    let starts: Vec<usize> = (0..n).step_by(chunk).collect();
    let plan = Plan::new(sel, &Key::single());
    let sel = &plan;
    let one = |start: usize| {
        let end = start.saturating_add(chunk).min(n);
        let mut out = Vec::new();
        let mut ok = vec![false; BLOCK];
        let mut bufs = Vec::new();
        let mut at = start;
        while at < end {
            let len = (end - at).min(BLOCK);
            ok.resize(len, false);
            sel.block(at, &mut ok, &mut bufs);
            for (i, keep) in ok.iter().enumerate() {
                if *keep && let Ok(r) = u32::try_from(at.wrapping_add(i)) {
                    out.push(r);
                }
            }
            at = at.saturating_add(len);
        }
        out
    };
    let serial = || starts.iter().flat_map(|s| one(*s)).collect::<Vec<u32>>();
    if threads == 1 {
        return serial();
    }
    // a failed worker: every chunk again on this thread (never a partial answer)
    pool::map(starts.len(), |i| starts.get(i).map_or_else(Vec::new, |s| one(*s)))
        .map_or_else(serial, |parts| parts.concat())
}

/// The first selected row of each group (`u32::MAX`: none), to list the groups in the order
/// their rows come (first seen first).
#[derive(Debug, Clone)]
pub struct FirstRow(pub Vec<u32>);

impl FirstRow {
    pub fn new(groups: usize) -> Self {
        FirstRow(vec![u32::MAX; groups])
    }
}

impl Sink for FirstRow {
    const NEEDS_ROW: bool = true;

    #[inline]
    fn add(&mut self, group: usize, _value: u32, row: usize) {
        if let Some(f) = self.0.get_mut(group) {
            *f = (*f).min(u32::try_from(row).unwrap_or(u32::MAX));
        }
    }

    fn merge(&mut self, other: Self) {
        for (a, b) in self.0.iter_mut().zip(other.0) {
            *a = (*a).min(b);
        }
    }
}

impl Ranged for FirstRow {
    fn append(&mut self, other: Self) {
        self.0.extend(other.0);
    }
}

impl Ranged for Tally {
    fn append(&mut self, other: Self) {
        self.counts.extend(other.counts);
        self.first.extend(other.first);
    }
}

impl Ranged for Counts {
    fn append(&mut self, other: Self) {
        self.0.extend(other.0);
    }
}

impl Ranged for BlankCounts {
    fn append(&mut self, other: Self) {
        self.nonblank.extend(other.nonblank);
        self.blank.extend(other.blank);
    }
}

impl Ranged for Histogram {
    fn append(&mut self, other: Self) {
        self.counts.extend(other.counts);
    }
}

impl Ranged for IntSums<'_> {
    fn append(&mut self, other: Self) {
        self.sums.extend(other.sums);
        self.counts.extend(other.counts);
    }
}

impl Ranged for FloatSums<'_> {
    fn append(&mut self, other: Self) {
        self.sums.extend(other.sums);
        self.comp.extend(other.comp);
        self.counts.extend(other.counts);
    }
}

impl Ranged for Ranks<'_> {
    fn append(&mut self, other: Self) {
        self.min.extend(other.min);
        self.max.extend(other.max);
        self.seen.extend(other.seen);
    }
}

impl Ranged for Distinct {
    fn append(&mut self, other: Self) {
        self.bits.extend(other.bits);
    }
}

/// Row counts per group.
#[derive(Debug, Clone)]
pub struct Counts(pub Vec<u64>);

impl Counts {
    pub fn new(groups: usize) -> Self {
        Counts(vec![0; groups])
    }
}

impl Sink for Counts {
    #[inline]
    fn add(&mut self, group: usize, _value: u32, _row: usize) {
        if let Some(n) = self.0.get_mut(group) {
            *n = n.wrapping_add(1);
        }
    }

    fn merge(&mut self, other: Self) {
        for (a, b) in self.0.iter_mut().zip(other.0) {
            *a = a.wrapping_add(b);
        }
    }
}

/// Rows per group with a non-blank code (`nonblank`) and with the blank code 0 (`blank`).
#[derive(Debug, Clone)]
pub struct BlankCounts {
    pub nonblank: Vec<u64>,
    pub blank: Vec<u64>,
}

impl BlankCounts {
    pub fn new(groups: usize) -> Self {
        BlankCounts {
            nonblank: vec![0; groups],
            blank: vec![0; groups],
        }
    }
}

impl Sink for BlankCounts {
    #[inline]
    fn add(&mut self, group: usize, value: u32, _row: usize) {
        let v = if value == 0 {
            &mut self.blank
        } else {
            &mut self.nonblank
        };
        if let Some(n) = v.get_mut(group) {
            *n = n.wrapping_add(1);
        }
    }

    fn merge(&mut self, other: Self) {
        for (a, b) in self.nonblank.iter_mut().zip(other.nonblank) {
            *a = a.wrapping_add(b);
        }
        for (a, b) in self.blank.iter_mut().zip(other.blank) {
            *a = a.wrapping_add(b);
        }
    }
}

/// Rows per (group, code): `counts[group × card + code]`.
#[derive(Debug, Clone)]
pub struct Histogram {
    pub card: usize,
    pub counts: Vec<u64>,
}

impl Histogram {
    /// `None` when `groups × card` is over `max` slots.
    pub fn new(groups: usize, card: usize, max: usize) -> Option<Self> {
        let slots = groups.checked_mul(card.max(1)).filter(|s| *s <= max)?;
        Some(Histogram {
            card: card.max(1),
            counts: vec![0; slots],
        })
    }

    /// No group (the start of a range-split scan's result).
    pub fn empty(card: usize) -> Self {
        Histogram {
            card: card.max(1),
            counts: Vec::new(),
        }
    }

    /// The counts of one group, by code.
    pub fn group(&self, g: usize) -> &[u64] {
        let start = g.saturating_mul(self.card);
        self.counts
            .get(start..start.saturating_add(self.card))
            .unwrap_or(&[])
    }
}

impl Sink for Histogram {
    #[inline]
    fn add(&mut self, group: usize, value: u32, _row: usize) {
        let at = group
            .wrapping_mul(self.card)
            .wrapping_add((value as usize).min(self.card.saturating_sub(1)));
        if let Some(n) = self.counts.get_mut(at) {
            *n = n.wrapping_add(1);
        }
    }

    fn merge(&mut self, other: Self) {
        for (a, b) in self.counts.iter_mut().zip(other.counts) {
            *a = a.wrapping_add(b);
        }
    }
}

/// Exact sums of integer values (`per_code[code]`, `None`: blank) and their counts per group.
#[derive(Debug, Clone)]
pub struct IntSums<'a> {
    pub per_code: &'a [Option<i64>],
    pub sums: Vec<i128>,
    pub counts: Vec<u64>,
}

impl<'a> IntSums<'a> {
    pub fn new(groups: usize, per_code: &'a [Option<i64>]) -> Self {
        IntSums {
            per_code,
            sums: vec![0; groups],
            counts: vec![0; groups],
        }
    }
}

impl Sink for IntSums<'_> {
    #[inline]
    fn add(&mut self, group: usize, value: u32, _row: usize) {
        if let Some(Some(x)) = self.per_code.get(value as usize)
            && let (Some(s), Some(n)) = (self.sums.get_mut(group), self.counts.get_mut(group))
        {
            *s = s.saturating_add(i128::from(*x));
            *n = n.wrapping_add(1);
        }
    }

    fn merge(&mut self, other: Self) {
        for (a, b) in self.sums.iter_mut().zip(other.sums) {
            *a = a.saturating_add(b);
        }
        for (a, b) in self.counts.iter_mut().zip(other.counts) {
            *a = a.wrapping_add(b);
        }
    }
}

/// Compensated (Neumaier) sums of double values and their counts per group.
#[derive(Debug, Clone)]
pub struct FloatSums<'a> {
    pub per_code: &'a [Option<f64>],
    pub sums: Vec<f64>,
    pub comp: Vec<f64>,
    pub counts: Vec<u64>,
}

impl<'a> FloatSums<'a> {
    pub fn new(groups: usize, per_code: &'a [Option<f64>]) -> Self {
        FloatSums {
            per_code,
            sums: vec![0.0; groups],
            comp: vec![0.0; groups],
            counts: vec![0; groups],
        }
    }

    /// The sum of one group.
    pub fn total(&self, g: usize) -> f64 {
        self.sums.get(g).copied().unwrap_or(0.0) + self.comp.get(g).copied().unwrap_or(0.0)
    }
}

/// One step of Neumaier's compensated summation: `sum + comp` is the running total.
pub fn neumaier(sum: &mut f64, comp: &mut f64, x: f64) {
    let t = *sum + x;
    if sum.abs() >= x.abs() {
        *comp += (*sum - t) + x;
    } else {
        *comp += (x - t) + *sum;
    }
    *sum = t;
}

impl Sink for FloatSums<'_> {
    #[inline]
    fn add(&mut self, group: usize, value: u32, _row: usize) {
        if let Some(Some(x)) = self.per_code.get(value as usize)
            && let (Some(s), Some(c), Some(n)) = (
                self.sums.get_mut(group),
                self.comp.get_mut(group),
                self.counts.get_mut(group),
            )
        {
            neumaier(s, c, *x);
            *n = n.wrapping_add(1);
        }
    }

    fn merge(&mut self, other: Self) {
        for ((s, c), (os, oc)) in self
            .sums
            .iter_mut()
            .zip(self.comp.iter_mut())
            .zip(other.sums.into_iter().zip(other.comp))
        {
            neumaier(s, c, os);
            *c += oc;
        }
        for (a, b) in self.counts.iter_mut().zip(other.counts) {
            *a = a.wrapping_add(b);
        }
    }
}

/// Lowest and highest rank (`rank[code]`) of the non-blank codes per group; `u32::MAX` / 0
/// with `seen` false when the group has none.
#[derive(Debug, Clone)]
pub struct Ranks<'a> {
    pub rank: &'a [u32],
    pub min: Vec<u32>,
    pub max: Vec<u32>,
    pub seen: Vec<bool>,
}

impl<'a> Ranks<'a> {
    pub fn new(groups: usize, rank: &'a [u32]) -> Self {
        Ranks {
            rank,
            min: vec![u32::MAX; groups],
            max: vec![0; groups],
            seen: vec![false; groups],
        }
    }
}

impl Sink for Ranks<'_> {
    #[inline]
    fn add(&mut self, group: usize, value: u32, _row: usize) {
        if value == 0 {
            return;
        }
        let r = self.rank.get(value as usize).copied().unwrap_or(0);
        if let (Some(lo), Some(hi), Some(s)) = (
            self.min.get_mut(group),
            self.max.get_mut(group),
            self.seen.get_mut(group),
        ) {
            *lo = (*lo).min(r);
            *hi = (*hi).max(r);
            *s = true;
        }
    }

    fn merge(&mut self, other: Self) {
        for (a, b) in self.min.iter_mut().zip(other.min) {
            *a = (*a).min(b);
        }
        for (a, b) in self.max.iter_mut().zip(other.max) {
            *a = (*a).max(b);
        }
        for (a, b) in self.seen.iter_mut().zip(other.seen) {
            *a |= b;
        }
    }
}

/// Distinct codes per group as sorted `(group, code)` pairs: the state of a distinct count whose
/// groups are too many for a bitmap each ([`Distinct`]): its size follows the pairs met (at most
/// one per selected row), not groups × codes. [`DistinctPairs::finish`] gives [`SparseDistinct`].
#[derive(Debug, Clone)]
pub struct DistinctPairs {
    groups: usize,
    card: usize,
    pairs: Vec<u64>,
    /// Length at which the pairs are next sorted and deduplicated.
    next_dedup: usize,
}

/// Pairs held before the first deduplication.
const PAIRS_DEDUP_MIN: usize = 1 << 20;

impl DistinctPairs {
    pub fn new(groups: usize, card: usize) -> Self {
        DistinctPairs {
            groups,
            card: card.max(1),
            pairs: Vec::new(),
            next_dedup: PAIRS_DEDUP_MIN,
        }
    }

    fn dedup(&mut self) {
        self.pairs.sort_unstable();
        self.pairs.dedup();
        self.next_dedup = self.pairs.len().saturating_mul(2).max(PAIRS_DEDUP_MIN);
    }

    /// The groups' distinct codes, sorted.
    pub fn finish(mut self) -> SparseDistinct {
        self.dedup();
        let mut offsets = vec![0usize; self.groups.saturating_add(1)];
        for p in &self.pairs {
            let g = (p >> 32) as usize;
            if let Some(o) = offsets.get_mut(g.saturating_add(1)) {
                *o = o.saturating_add(1);
            }
        }
        for i in 1..offsets.len() {
            let prev = offsets.get(i - 1).copied().unwrap_or(0);
            if let Some(o) = offsets.get_mut(i) {
                *o = o.saturating_add(prev);
            }
        }
        let codes: Vec<u32> = self.pairs.iter().map(|p| *p as u32).collect();
        SparseDistinct {
            card: self.card,
            offsets,
            codes,
        }
    }
}

impl Sink for DistinctPairs {
    #[inline]
    fn add(&mut self, group: usize, value: u32, _row: usize) {
        // the same clamp as `Distinct`
        let v = (value as usize).min(self.card.saturating_sub(1)) as u64;
        self.pairs.push(((group as u64) << 32) | v);
        if self.pairs.len() >= self.next_dedup {
            self.dedup();
        }
    }

    fn merge(&mut self, other: Self) {
        self.pairs.extend(other.pairs);
        if self.pairs.len() >= self.next_dedup {
            self.dedup();
        }
    }
}

impl Ranged for DistinctPairs {
    fn append(&mut self, other: Self) {
        let off = self.groups as u64;
        self.pairs
            .extend(other.pairs.iter().map(|p| p.wrapping_add(off << 32)));
        self.groups = self.groups.saturating_add(other.groups);
    }
}

/// Distinct codes per group, sorted: group `g`'s are `codes[offsets[g]..offsets[g + 1]]`.
#[derive(Debug, Clone)]
pub struct SparseDistinct {
    pub card: usize,
    pub offsets: Vec<usize>,
    pub codes: Vec<u32>,
}

impl SparseDistinct {
    pub fn groups(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    /// Group `g`'s codes (sorted).
    pub fn group(&self, g: usize) -> &[u32] {
        let (Some(a), Some(b)) = (self.offsets.get(g), self.offsets.get(g.saturating_add(1))) else {
            return &[];
        };
        self.codes.get(*a..*b).unwrap_or(&[])
    }

    /// (distinct codes, whether the blank code 0 is among them) of one group, as
    /// [`Distinct::count`].
    pub fn count(&self, g: usize) -> (u64, bool) {
        let c = self.group(g);
        (c.len() as u64, c.first() == Some(&0))
    }

    /// [`SparseDistinct::count`] of the union of several groups.
    pub fn count_union(&self, groups: &[usize]) -> (u64, bool) {
        let mut all: Vec<u32> = groups
            .iter()
            .flat_map(|g| self.group(*g).iter().copied())
            .collect();
        all.sort_unstable();
        all.dedup();
        (all.len() as u64, all.first() == Some(&0))
    }

    /// The groups merged by `map` (fine group → coarse group) into `coarse` groups.
    pub fn coarsen(&self, map: &[usize], coarse: usize) -> Option<SparseDistinct> {
        let mut pairs = DistinctPairs::new(coarse, self.card);
        for f in 0..self.groups() {
            let g = *map.get(f)?;
            if g >= coarse {
                return None;
            }
            for c in self.group(f) {
                pairs.pairs.push(((g as u64) << 32) | u64::from(*c));
            }
        }
        Some(pairs.finish())
    }

    pub fn bytes(&self) -> usize {
        self.codes
            .len()
            .saturating_mul(4)
            .saturating_add(self.offsets.len().saturating_mul(8))
    }
}

/// The distinct codes of each group, as a bitmap of `card` bits per group.
#[derive(Debug, Clone)]
pub struct Distinct {
    pub card: usize,
    words: usize,
    pub bits: Vec<u64>,
}

impl Distinct {
    /// `None` when the bitmaps would be over `max_bytes`.
    pub fn new(groups: usize, card: usize, max_bytes: usize) -> Option<Self> {
        let words = card.max(1).div_ceil(64);
        let total = groups.checked_mul(words)?;
        if total.checked_mul(8)? > max_bytes {
            return None;
        }
        Some(Distinct {
            card: card.max(1),
            words,
            bits: vec![0; total],
        })
    }

    /// No group (the start of a range-split scan's result).
    pub fn empty(card: usize) -> Self {
        Distinct {
            card: card.max(1),
            words: card.max(1).div_ceil(64),
            bits: Vec::new(),
        }
    }

    /// Words of one group's bitmap.
    pub fn words(&self) -> usize {
        self.words
    }

    /// (distinct codes, whether the blank code 0 is among them) of one group.
    pub fn count(&self, g: usize) -> (u64, bool) {
        let start = g.saturating_mul(self.words);
        let w = self
            .bits
            .get(start..start.saturating_add(self.words))
            .unwrap_or(&[]);
        let n: u64 = w.iter().map(|x| u64::from(x.count_ones())).sum();
        (n, w.first().is_some_and(|x| x & 1 == 1))
    }

    /// [`Distinct::count`] of the union of several groups.
    pub fn count_union(&self, groups: &[usize]) -> (u64, bool) {
        let mut acc = vec![0u64; self.words];
        for g in groups {
            let start = g.saturating_mul(self.words);
            let w = self
                .bits
                .get(start..start.saturating_add(self.words))
                .unwrap_or(&[]);
            for (a, b) in acc.iter_mut().zip(w) {
                *a |= *b;
            }
        }
        let n: u64 = acc.iter().map(|x| u64::from(x.count_ones())).sum();
        (n, acc.first().is_some_and(|x| x & 1 == 1))
    }
}

impl Sink for Distinct {
    #[inline]
    fn add(&mut self, group: usize, value: u32, _row: usize) {
        let v = (value as usize).min(self.card.saturating_sub(1));
        let at = group.wrapping_mul(self.words).wrapping_add(v / 64);
        if let Some(w) = self.bits.get_mut(at) {
            *w |= 1u64 << (v % 64);
        }
    }

    fn merge(&mut self, other: Self) {
        for (a, b) in self.bits.iter_mut().zip(other.bits) {
            *a |= b;
        }
    }
}

#[cfg(test)]
mod kernel_tests;
#[cfg(test)]
mod tests;
