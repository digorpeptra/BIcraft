//! Segments read on demand (`docs/beyond-ram.md`, lot 3): a column too large to keep in memory
//! keeps only its segments' statistics and the id table; each segment's data is read from its
//! source (the backup's temporary file) when a scan reaches it, and kept in a [`SegCache`]
//! shared by the columns of a session, bounded in bytes.
//!
//! A segment a scan holds stays alive through its `Arc` (the cache only drops its own
//! reference), so eviction never pulls data from under a running scan. Each column read on demand
//! has an identity of its own ([`Lazy`]): a column read again is another identity, so a cache can
//! never serve a segment of an earlier read; a dropped column removes its segments from the cache.
//!
//! A segment that cannot be read (the file failed) is counted in [`load_failures`] and reads as
//! blank ids; the caller checks the count around an evaluation and fails it (never an answer
//! computed from a failed read).
//!
//! **Scan resistance** (`docs/beyond-ram.md`, "Re-reads below the working set"). Under plain LRU a
//! scan repeated over more segments than the cache holds finds none of them (each pass evicts the
//! segments the next pass needs first). The cache is split in two parts, as in 2Q / segmented LRU:
//! - a *protected* part, at most three quarters of the bound: a segment enters it when it is used
//!   again while kept, or read again soon after it was evicted (a short history of evicted
//!   segments); it leaves only when it was not used for two passes over every column reading
//!   through the cache (stale), or when the bound shrinks;
//! - a *probation* part, least recently used out first, where every other segment goes.
//!
//! A new segment never displaces a protected segment still in use: a repeated scan over more
//! segments than the cache holds finds the protected share every time, and the rest streams
//! through probation. Besides: a segment being read is read once (the other threads that need it
//! wait for that read), and a segment evicted while a scan still holds it is handed out again
//! rather than read again.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use crate::packed::{Ids, SegData};

/// Where the data of a column's segments comes from.
pub trait SegSource: Send + Sync + std::fmt::Debug {
    /// The data of segment `seg`; `None` when it cannot be read.
    fn load(&self, seg: usize) -> Option<SegData>;
}

/// Segments that could not be read since the process started.
static FAILURES: AtomicU64 = AtomicU64::new(0);

/// Segments that could not be read since the process started (see the module's notes).
pub fn load_failures() -> u64 {
    FAILURES.load(Ordering::Relaxed)
}

/// Identities of on-demand columns (never reused).
static IDS: AtomicU64 = AtomicU64::new(1);

/// Bytes of a segment's data in memory.
pub fn seg_data_bytes(d: &SegData) -> usize {
    let ids = |i: &Ids| match i {
        Ids::Bits { words, .. } => words.len().saturating_mul(8),
        Ids::Seq { .. } => 0,
    };
    match d {
        SegData::Plain(i) => ids(i),
        SegData::Rle { runs, sub } => runs.len().saturating_mul(16).saturating_add(ids(sub)),
    }
    .saturating_add(64)
}

/// (column identity, segment).
type Key = (u64, usize);

/// A segment being read: the threads that need it meanwhile wait for the same read.
type Loading = Arc<OnceLock<Option<Arc<SegData>>>>;

struct Entry {
    data: Arc<SegData>,
    bytes: usize,
    /// Its last use on the access clock (least recently used out first).
    tick: u64,
    /// The load clock at its last use: a protected segment unused for [`Inner::stale`] loads is
    /// stale.
    used: u64,
    protected: bool,
}

#[derive(Default)]
struct Inner {
    map: HashMap<Key, Entry>,
    bytes: usize,
    /// Bytes of the protected part.
    protected: usize,
    /// Access clock (one step per use).
    tick: u64,
    /// Load clock (one step per segment read).
    clock: u64,
    /// Segments evicted lately, with the load clock of their eviction: one read again soon is
    /// protected.
    ghost: HashMap<Key, u64>,
    /// Segments read, possibly still held by a scan after their eviction.
    live: HashMap<Key, Weak<SegData>>,
    /// Segments being read.
    loading: HashMap<Key, Loading>,
    /// Segments of the columns reading through the cache.
    segments: usize,
    /// Its bytes, reserved against the process' memory limit (`bicraft_mem`, lot 5): a segment
    /// that does not fit is not kept (the scan that read it still uses it).
    res: bicraft_mem::Reservation,
}

impl Inner {
    /// The reservation follows the bytes kept (it only shrinks here; growing is [`Inner::keep`]).
    fn sync(&mut self) {
        self.res.shrink_to(self.bytes);
    }

    /// Loads after which an unused protected segment is stale: two passes over every column.
    fn stale(&self) -> u64 {
        u64::try_from(self.segments.max(16).saturating_mul(2)).unwrap_or(u64::MAX)
    }

    fn is_stale(&self, e: &Entry) -> bool {
        self.clock.saturating_sub(e.used) > self.stale()
    }

    /// The next segment to evict: probation before protected, segments no scan holds before held
    /// ones, stale protected segments before the others, least recently used first.
    fn victim(&self) -> Option<Key> {
        self.map
            .iter()
            .map(|(k, e)| {
                let held = Arc::strong_count(&e.data) > 1;
                let rank = match (e.protected, self.is_stale(e), held) {
                    (false, _, false) => 0u8,
                    (true, true, false) => 1,
                    (false, _, true) => 2,
                    (true, true, true) => 3,
                    (true, false, false) => 4,
                    (true, false, true) => 5,
                };
                ((rank, e.tick), *k)
            })
            .min()
            .map(|(_, k)| k)
    }

    /// Removes `key`, remembered as evicted.
    fn remove(&mut self, key: Key) -> bool {
        let Some(e) = self.map.remove(&key) else {
            return false;
        };
        self.bytes = self.bytes.saturating_sub(e.bytes);
        if e.protected {
            self.protected = self.protected.saturating_sub(e.bytes);
        }
        self.ghost.insert(key, self.clock);
        let bound = self.segments.max(64).saturating_mul(2);
        if self.ghost.len() > bound.saturating_mul(2) {
            // the oldest half goes
            let mut ages: Vec<u64> = self.ghost.values().copied().collect();
            ages.sort_unstable();
            let cut = ages.get(ages.len().saturating_sub(bound)).copied().unwrap_or(0);
            self.ghost.retain(|_, c| *c >= cut);
        }
        true
    }

    /// Evicts until at most `cap` bytes are kept; returns the segments evicted.
    fn make_room(&mut self, cap: usize) -> u64 {
        let mut n = 0u64;
        while self.bytes > cap {
            let Some(k) = self.victim() else {
                break;
            };
            if self.remove(k) {
                n = n.saturating_add(1);
            }
        }
        self.sync();
        n
    }

    /// Room in the protected part (three quarters of `cap`) for `bytes` more, stale protected
    /// segments moved to probation to make it; `false` when the protected part is in use.
    fn protect_room(&mut self, bytes: usize, cap: usize) -> bool {
        let pcap = cap / 4 * 3;
        if bytes > pcap {
            return false;
        }
        while self.protected.saturating_add(bytes) > pcap {
            let stale = self
                .map
                .iter()
                .filter(|(_, e)| e.protected && self.is_stale(e))
                .min_by_key(|(_, e)| e.tick)
                .map(|(k, _)| *k);
            let Some(k) = stale else {
                return false;
            };
            if let Some(e) = self.map.get_mut(&k) {
                e.protected = false;
                self.protected = self.protected.saturating_sub(e.bytes);
            }
        }
        true
    }

    /// A kept segment, now the most recently used (protected when there is room: it is used
    /// again).
    fn hit(&mut self, key: Key, cap: usize) -> Option<Arc<SegData>> {
        self.tick = self.tick.wrapping_add(1);
        let (tick, clock) = (self.tick, self.clock);
        let e = self.map.get_mut(&key)?;
        e.tick = tick;
        e.used = clock;
        let data = Arc::clone(&e.data);
        if !e.protected {
            let b = e.bytes;
            if self.protect_room(b, cap)
                && let Some(e) = self.map.get_mut(&key)
            {
                e.protected = true;
                self.protected = self.protected.saturating_add(b);
            }
        }
        Some(data)
    }

    /// Keeps `data` (segment `key`) within `cap` and within what the process' memory limit
    /// leaves: protected when it was evicted lately and there is room, else in probation; a
    /// segment that does not fit without displacing protected segments in use is not kept.
    /// Returns the segments evicted.
    fn keep(&mut self, key: Key, data: &Arc<SegData>, cap: usize) -> u64 {
        self.tick = self.tick.wrapping_add(1);
        if let Some(e) = self.map.get_mut(&key) {
            // another thread kept it meanwhile
            e.tick = self.tick;
            e.used = self.clock;
            return 0;
        }
        let bytes = seg_data_bytes(data);
        let again = self.ghost.remove(&key).is_some();
        let protected = again && self.protect_room(bytes, cap);
        self.map.insert(
            key,
            Entry {
                data: Arc::clone(data),
                bytes,
                tick: self.tick,
                used: self.clock,
                protected,
            },
        );
        self.bytes = self.bytes.saturating_add(bytes);
        if protected {
            self.protected = self.protected.saturating_add(bytes);
        }
        let mut n = self.make_room(cap);
        let more = self.bytes.saturating_sub(self.res.bytes());
        if more > 0 && self.res.try_grow(more).is_err() {
            // over the limit: the cache keeps only what it has reserved
            let room = self.res.bytes();
            n = n.saturating_add(self.make_room(room));
        }
        if self.live.len() > self.map.len().saturating_mul(2).saturating_add(64) {
            self.live.retain(|_, w| w.strong_count() > 0);
        }
        n
    }
}

/// The segments read on demand that a session keeps, at most `cap` bytes (plus the segments
/// scans hold at the moment).
pub struct SegCache {
    cap: AtomicU64,
    inner: Mutex<Inner>,
    loads: AtomicU64,
    hits: AtomicU64,
    shared: AtomicU64,
    evictions: AtomicU64,
}

impl std::fmt::Debug for SegCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegCache")
            .field("cap", &self.cap())
            .field("bytes", &self.bytes())
            .finish_non_exhaustive()
    }
}

impl SegCache {
    pub fn new(cap: usize) -> Arc<SegCache> {
        SegCache::new_in(cap, bicraft_mem::global())
    }

    /// A cache whose segments are reserved in `pool` (lot 5).
    pub fn new_in(cap: usize, pool: &'static bicraft_mem::Pool) -> Arc<SegCache> {
        Arc::new(SegCache {
            cap: AtomicU64::new(u64::try_from(cap).unwrap_or(u64::MAX)),
            inner: Mutex::new(Inner {
                res: pool.empty(bicraft_mem::Class::Kept),
                ..Inner::default()
            }),
            loads: AtomicU64::new(0),
            hits: AtomicU64::new(0),
            shared: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        })
    }

    pub fn cap(&self) -> usize {
        usize::try_from(self.cap.load(Ordering::Relaxed)).unwrap_or(usize::MAX)
    }

    /// Sets the bound (and keeps to it now).
    pub fn set_cap(&self, cap: usize) {
        self.cap
            .store(u64::try_from(cap).unwrap_or(u64::MAX), Ordering::Relaxed);
        self.shrink_to(cap);
    }

    /// Bytes of the segments it keeps.
    pub fn bytes(&self) -> usize {
        self.inner.lock().map_or(0, |g| g.bytes)
    }

    /// Bytes of the protected part (see the module's notes).
    pub fn protected_bytes(&self) -> usize {
        self.inner.lock().map_or(0, |g| g.protected)
    }

    /// Bytes it holds in the process' memory pool (0 without a limit).
    pub fn reserved(&self) -> usize {
        self.inner.lock().map_or(0, |g| g.res.bytes())
    }

    /// Segments kept.
    pub fn len(&self) -> usize {
        self.inner.lock().map_or(0, |g| g.map.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// (segments read, segments found in the cache, segments evicted) since it was made.
    pub fn counters(&self) -> (u64, u64, u64) {
        (
            self.loads.load(Ordering::Relaxed),
            self.hits.load(Ordering::Relaxed),
            self.evictions.load(Ordering::Relaxed),
        )
    }

    /// Segments handed out without a read although not kept: being read by another thread, or
    /// evicted while a scan still held them.
    pub fn shared(&self) -> u64 {
        self.shared.load(Ordering::Relaxed)
    }

    /// Keeps at most `bytes` now (the bound stays): probation first, then stale protected
    /// segments, least recently used first.
    pub fn shrink_to(&self, bytes: usize) {
        if let Ok(mut g) = self.inner.lock() {
            let n = g.make_room(bytes);
            self.evictions.fetch_add(n, Ordering::Relaxed);
        }
    }

    /// Drops every kept segment (scans holding one keep theirs).
    pub fn clear(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.map.clear();
            g.bytes = 0;
            g.protected = 0;
            g.ghost.clear();
            g.sync();
        }
    }

    /// A column of `n` segments reads through the cache.
    fn register(&self, n: usize) {
        if let Ok(mut g) = self.inner.lock() {
            g.segments = g.segments.saturating_add(n);
        }
    }

    /// Forgets the segments of column `id` (of `n` segments, dropped).
    fn forget(&self, id: u64, n: usize) {
        if let Ok(mut g) = self.inner.lock() {
            let (mut freed, mut prot) = (0usize, 0usize);
            g.map.retain(|k, v| {
                if k.0 == id {
                    freed = freed.saturating_add(v.bytes);
                    if v.protected {
                        prot = prot.saturating_add(v.bytes);
                    }
                    false
                } else {
                    true
                }
            });
            g.bytes = g.bytes.saturating_sub(freed);
            g.protected = g.protected.saturating_sub(prot);
            g.ghost.retain(|k, _| k.0 != id);
            g.live.retain(|k, _| k.0 != id);
            g.loading.retain(|k, _| k.0 != id);
            g.segments = g.segments.saturating_sub(n);
            g.sync();
        }
    }
}

/// A column's segments read on demand: its identity, its source and the cache it reads through.
pub struct Lazy {
    id: u64,
    segments: usize,
    source: Arc<dyn SegSource>,
    cache: Weak<SegCache>,
}

impl std::fmt::Debug for Lazy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lazy")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Drop for Lazy {
    fn drop(&mut self) {
        if let Some(c) = self.cache.upgrade() {
            c.forget(self.id, self.segments);
        }
    }
}

/// (column, segment) and its data, not kept alive.
type LastSeg = (Key, Weak<SegData>);

/// Segments a thread remembers (a scan reads a segment's rows in a row, often of several columns
/// at once).
const LAST_SLOTS: usize = 4;

thread_local! {
    /// The segments this thread read last, most recent first, without keeping them alive.
    static LAST: std::cell::RefCell<[Option<LastSeg>; LAST_SLOTS]> =
        const { std::cell::RefCell::new([None, None, None, None]) };
}

/// Every row id 0 (code 0): what a failed read gives (the caller fails the answer).
fn blank() -> Arc<SegData> {
    Arc::new(SegData::Rle {
        runs: vec![crate::packed::Run {
            start: 0,
            len: u32::MAX,
            id: 0,
            packed: false,
        }],
        sub: Ids::Seq { first: 0 },
    })
}

impl Lazy {
    /// The `segments` segments of `source`, read through `cache`.
    pub fn new(source: Arc<dyn SegSource>, segments: usize, cache: &Arc<SegCache>) -> Arc<Lazy> {
        cache.register(segments);
        Arc::new(Lazy {
            id: IDS.fetch_add(1, Ordering::Relaxed),
            segments,
            source,
            cache: Arc::downgrade(cache),
        })
    }

    /// The column's identity.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Segment `seg`'s data, read by the column's first pass (`data` must be what the source
    /// gives): kept in probation when it fits without evicting anything (the first segments of
    /// the pass stay, the scan that follows starts with them), else dropped.
    pub fn offer(&self, seg: usize, data: SegData) {
        let Some(c) = self.cache.upgrade() else {
            return;
        };
        let cap = c.cap();
        let data = Arc::new(data);
        let bytes = seg_data_bytes(&data);
        if let Ok(mut g) = c.inner.lock() {
            let key = (self.id, seg);
            if g.map.contains_key(&key) || g.bytes.saturating_add(bytes) > cap {
                return;
            }
            let more = g.bytes.saturating_add(bytes).saturating_sub(g.res.bytes());
            if more > 0 && g.res.try_grow(more).is_err() {
                return;
            }
            g.tick = g.tick.wrapping_add(1);
            let (tick, clock) = (g.tick, g.clock);
            g.map.insert(
                key,
                Entry {
                    data,
                    bytes,
                    tick,
                    used: clock,
                    protected: false,
                },
            );
            g.bytes = g.bytes.saturating_add(bytes);
        }
    }

    /// The data of segment `seg`: from this thread's last segments, the cache, or the source.
    pub fn get(&self, seg: usize) -> Arc<SegData> {
        let key = (self.id, seg);
        let last = LAST.with(|l| {
            l.borrow()
                .iter()
                .flatten()
                .find(|(k, _)| *k == key)
                .and_then(|(_, w)| w.upgrade())
        });
        if let Some(d) = last {
            return d;
        }
        let d = self.fetch(seg);
        LAST.with(|l| {
            let mut l = l.borrow_mut();
            // replaces this column's slot (a scan moves on through a column), else the oldest
            let at = l
                .iter()
                .position(|s| s.as_ref().is_some_and(|(k, _)| k.0 == self.id))
                .unwrap_or(LAST_SLOTS - 1);
            if let Some(s) = l.get_mut(..=at) {
                s.rotate_right(1);
                if let Some(first) = s.first_mut() {
                    *first = Some((key, Arc::downgrade(&d)));
                }
            }
        });
        d
    }

    fn fetch(&self, seg: usize) -> Arc<SegData> {
        let key = (self.id, seg);
        let Some(c) = self.cache.upgrade() else {
            return self.source.load(seg).map_or_else(
                || {
                    FAILURES.fetch_add(1, Ordering::Relaxed);
                    blank()
                },
                Arc::new,
            );
        };
        let cap = c.cap();
        let slot = match c.inner.lock() {
            Ok(mut g) => {
                if let Some(d) = g.hit(key, cap) {
                    c.hits.fetch_add(1, Ordering::Relaxed);
                    return d;
                }
                if let Some(d) = g.live.get(&key).and_then(Weak::upgrade) {
                    // evicted while a scan holds it: the same data, kept again
                    c.shared.fetch_add(1, Ordering::Relaxed);
                    let n = g.keep(key, &d, cap);
                    c.evictions.fetch_add(n, Ordering::Relaxed);
                    return d;
                }
                Arc::clone(g.loading.entry(key).or_default())
            }
            Err(_) => Loading::default(),
        };
        // read outside the lock (other threads go on with their segments); the threads that need
        // the same segment meanwhile wait for this read
        let mut here = false;
        let got = slot
            .get_or_init(|| {
                here = true;
                self.source.load(seg).map(Arc::new)
            })
            .clone();
        #[cfg(test)]
        if here {
            tests::pause_after_read();
        }
        if let Ok(mut g) = c.inner.lock() {
            // only the reader ends the read, once it keeps the segment: a thread that waited for
            // it and ended it first would let another one read the segment again
            if here && g.loading.get(&key).is_some_and(|s| Arc::ptr_eq(s, &slot)) {
                g.loading.remove(&key);
            }
            match (&got, here) {
                (Some(d), true) => {
                    c.loads.fetch_add(1, Ordering::Relaxed);
                    g.clock = g.clock.wrapping_add(1);
                    g.live.insert(key, Arc::downgrade(d));
                    let n = g.keep(key, d, cap);
                    c.evictions.fetch_add(n, Ordering::Relaxed);
                }
                (Some(_), false) => {
                    c.shared.fetch_add(1, Ordering::Relaxed);
                }
                (None, _) => {}
            }
        }
        match got {
            Some(d) => d,
            None => {
                FAILURES.fetch_add(1, Ordering::Relaxed);
                blank()
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::{Col, Counts, Key, KeyPart, Packed, Pred, Segment, Selection, scan};

    /// The segments of a column in memory, counted as they are read.
    #[derive(Debug)]
    struct Mem {
        segs: Vec<SegData>,
        reads: AtomicU64,
        fail: bool,
    }

    impl SegSource for Mem {
        fn load(&self, seg: usize) -> Option<SegData> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            if self.fail {
                return None;
            }
            self.segs.get(seg).cloned()
        }
    }

    thread_local! {
        /// How long this thread waits between reading a segment and keeping it (widens the
        /// window the threads that waited for the same read finish in).
        static PAUSE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    }

    pub(super) fn pause_after_read() {
        let ms = PAUSE.with(std::cell::Cell::get);
        if ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
    }

    /// One segment, read slowly and counted.
    #[derive(Debug, Default)]
    struct Slow {
        reads: AtomicU64,
    }

    impl SegSource for Slow {
        fn load(&self, _: usize) -> Option<SegData> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            std::thread::sleep(std::time::Duration::from_millis(100));
            Some(SegData::Plain(Ids::Seq { first: 0 }))
        }
    }

    #[test]
    fn a_thread_that_waited_for_a_read_does_not_let_a_third_read_it_again() {
        // the reader is slow to keep the segment; the thread that waited for its read finishes
        // first, and a third thread arriving then must still find the read, not read again
        let cache = SegCache::new(1 << 30);
        let src = Arc::new(Slow::default());
        let lazy = Lazy::new(Arc::clone(&src) as Arc<dyn SegSource>, 1, &cache);
        let ms = std::time::Duration::from_millis;
        std::thread::scope(|s| {
            s.spawn(|| {
                PAUSE.with(|p| p.set(300));
                lazy.fetch(0);
            });
            std::thread::sleep(ms(30));
            s.spawn(|| lazy.fetch(0));
            std::thread::sleep(ms(150));
            s.spawn(|| lazy.fetch(0));
        });
        assert_eq!(src.reads.load(Ordering::Relaxed), 1);
    }

    /// `p` with its segments read on demand from a copy of them.
    fn lazy_copy(p: &Packed, cache: &Arc<SegCache>, fail: bool) -> (Packed, Arc<Mem>) {
        let mem = Arc::new(Mem {
            segs: p.segments().iter().map(|s| s.data.clone()).collect(),
            reads: AtomicU64::new(0),
            fail,
        });
        let placeholders = p
            .segments()
            .iter()
            .map(|s| Segment {
                rows: s.rows,
                data: SegData::Plain(Ids::Seq { first: 0 }),
                min_id: s.min_id,
                max_id: s.max_id,
            })
            .collect();
        let stats = p.segments().iter().map(Packed::segment_stats).collect();
        let source: Arc<dyn SegSource> = Arc::clone(&mem) as Arc<dyn SegSource>;
        let lazy = Packed::new_lazy(
            placeholders,
            p.lut().to_vec(),
            stats,
            Lazy::new(source, p.segments().len(), cache),
        )
        .unwrap();
        (lazy, mem)
    }

    fn codes(n: usize) -> Vec<u32> {
        (0..n as u32)
            .map(|i| {
                if (i / 40) % 3 == 0 {
                    (i / 40) % 9
                } else {
                    (i.wrapping_mul(2_654_435_761) >> 7) % 50
                }
            })
            .collect()
    }

    #[test]
    fn segments_read_on_demand_give_the_same_codes_and_scans() {
        let c = codes(50_000);
        for seg in [1000usize, 4096, 20_000] {
            let p = crate::tests::pack(&c, seg);
            let cache = SegCache::new(1 << 30);
            let (l, mem) = lazy_copy(&p, &cache, false);
            assert!(l.is_lazy() && !p.is_lazy());
            assert!(l.bytes() < p.bytes(), "the data is not held by the column");
            for r in (0..c.len()).step_by(37) {
                assert_eq!(l.get(r), p.get(r));
                assert_eq!(l.get_id(r), p.get_id(r));
            }
            let (mut a, mut b) = (vec![0u32; c.len()], vec![0u32; c.len()]);
            l.fill(0, &mut a);
            p.fill(0, &mut b);
            assert_eq!(a, c);
            assert_eq!(a, b);
            assert_eq!(l.counts(51), p.counts(51));
            assert_eq!(l.counts_parallel(51, 4), p.counts(51));
            assert_eq!(l.run_rows(), p.run_rows());
            for start in (0..c.len()).step_by(997) {
                let end = (start + 300).min(c.len());
                assert_eq!(l.run_through(start, end), p.run_through(start, end));
                let (mut x, mut y) = (Vec::new(), Vec::new());
                assert_eq!(l.runs_in(start, end, &mut x), p.runs_in(start, end, &mut y));
                assert_eq!(x, y);
                assert_eq!(l.id_sum(start, end, 3), p.id_sum(start, end, 3));
                assert_eq!(l.ids_within(start, end, 3, 60), p.ids_within(start, end, 3, 60));
            }
            // a grouped, filtered scan on several threads
            let allowed: Vec<bool> = (0..51).map(|k| k % 4 != 1).collect();
            let run = |col: Col<'_>| {
                let sel = Selection {
                    rows: c.len(),
                    preds: vec![Pred {
                        codes: col,
                        allowed: &allowed,
                    }],
                    masks: vec![],
                    none: false,
                };
                let key = Key::new(vec![KeyPart {
                    codes: col,
                    lut: None,
                    card: 51,
                }])
                .unwrap();
                scan(&sel, &key, None, 4, || Counts::new(51)).unwrap()
            };
            assert_eq!(run(Col::Packed(&l)).0, run(Col::Packed(&p)).0);
            // read through the cache: each segment read once
            assert!(mem.reads.load(Ordering::Relaxed) as usize <= p.segments().len() + 4 * 8);
        }
    }

    #[test]
    fn the_cache_keeps_to_its_bound_and_forgets_a_dropped_column() {
        let c = codes(40_000);
        let p = crate::tests::pack(&c, 2000);
        let one = seg_data_bytes(&p.segments()[0].data);
        let cache = SegCache::new(one * 3);
        let (l, mem) = lazy_copy(&p, &cache, false);
        let mut buf = vec![0u32; c.len()];
        l.fill(0, &mut buf);
        assert_eq!(buf, c);
        assert!(cache.bytes() <= one * 3 + one, "bounded: {}", cache.bytes());
        let (loads, _, evictions) = cache.counters();
        assert!(loads >= 20 && evictions > 0);
        // read again: evicted segments come back the same
        l.fill(0, &mut buf);
        assert_eq!(buf, c);
        assert!(mem.reads.load(Ordering::Relaxed) >= 40);
        // a smaller bound now; then the column dropped: its segments go
        cache.set_cap(one);
        assert!(cache.bytes() <= one);
        assert!(!cache.is_empty());
        drop(l);
        assert!(cache.is_empty());
        assert_eq!(cache.bytes(), 0);
        // a column read again is another identity: nothing of the first is served
        let (l2, mem2) = lazy_copy(&p, &cache, false);
        l2.fill(0, &mut buf);
        assert_eq!(buf, c);
        assert_eq!(mem2.reads.load(Ordering::Relaxed), 20);
    }

    #[test]
    fn a_failed_read_is_counted_and_reads_as_blank() {
        let c = codes(5000);
        let p = crate::tests::pack(&c, 1000);
        let cache = SegCache::new(1 << 20);
        let (l, _) = lazy_copy(&p, &cache, true);
        let before = load_failures();
        let mut buf = vec![7u32; c.len()];
        l.fill(0, &mut buf);
        assert!(load_failures() > before);
        assert!(buf.iter().all(|x| *x == 0));
        assert!(cache.is_empty(), "a failed read is not kept");
    }

    /// The largest segment's bytes in the cache.
    fn largest(p: &Packed) -> usize {
        p.segments()
            .iter()
            .map(|s| seg_data_bytes(&s.data))
            .max()
            .unwrap()
    }

    /// Reads every row of `l` (one pass) and returns the segments read from the source meanwhile.
    fn pass(l: &Packed, mem: &Mem, want: &[u32]) -> u64 {
        let before = mem.reads.load(Ordering::Relaxed);
        let mut buf = vec![0u32; want.len()];
        l.fill(0, &mut buf);
        assert_eq!(buf, want);
        mem.reads.load(Ordering::Relaxed) - before
    }

    #[test]
    fn a_repeated_scan_larger_than_the_cache_finds_its_protected_share() {
        let c = codes(40_000);
        let p = crate::tests::pack(&c, 2000); // 20 segments
        let one = largest(&p);
        let cache = SegCache::new(one * 8);
        let (l, mem) = lazy_copy(&p, &cache, false);
        let mut loads = Vec::new();
        for _ in 0..6 {
            loads.push(pass(&l, &mem, &c));
            assert!(cache.bytes() <= one * 8, "bounded: {}", cache.bytes());
            assert!(cache.protected_bytes() <= cache.bytes());
        }
        // plain LRU reads all 20 on every pass; here the protected part (6 segments) stays
        assert_eq!(loads[0], 20);
        for n in &loads[2..] {
            assert!(*n <= 15, "loads per pass: {loads:?}");
        }
        assert!(cache.protected_bytes() >= one * 5, "{}", cache.protected_bytes());
        let (_, hits, _) = cache.counters();
        assert!(hits >= 20, "hits {hits}");
    }

    #[test]
    fn a_single_scan_of_another_column_does_not_flush_the_protected_segments() {
        let c = codes(40_000);
        let p = crate::tests::pack(&c, 2000);
        let one = largest(&p);
        let cache = SegCache::new(one * 8);
        let (a, mem_a) = lazy_copy(&p, &cache, false);
        let (b, mem_b) = lazy_copy(&p, &cache, false);
        for _ in 0..4 {
            pass(&a, &mem_a, &c);
        }
        let steady = pass(&a, &mem_a, &c);
        assert!(steady <= 15, "{steady}");
        // one pass over another column (a one-time scan): every segment read, none displaces
        // the protected segments of the first
        assert_eq!(pass(&b, &mem_b, &c), 20);
        assert_eq!(pass(&a, &mem_a, &c), steady);
        assert!(cache.bytes() <= one * 8);
    }

    #[test]
    fn stale_protected_segments_give_way_to_a_new_working_set() {
        let c = codes(40_000);
        let p = crate::tests::pack(&c, 2000);
        let one = largest(&p);
        let cache = SegCache::new(one * 8);
        let (a, mem_a) = lazy_copy(&p, &cache, false);
        let (b, mem_b) = lazy_copy(&p, &cache, false);
        for _ in 0..4 {
            pass(&a, &mem_a, &c);
        }
        // the scans move to the other column: once the first one's segments are stale (unused
        // for two passes over both columns), the second one's take the protected part
        let mut last = 0;
        for _ in 0..12 {
            last = pass(&b, &mem_b, &c);
        }
        assert!(last <= 15, "{last}");
        assert!(pass(&a, &mem_a, &c) >= 18);
    }

    #[test]
    fn threads_reading_the_same_segments_read_each_once() {
        let c = codes(200_000);
        let p = crate::tests::pack(&c, 10_000); // 20 segments
        let cache = SegCache::new(1 << 30);
        let (l, mem) = lazy_copy(&p, &cache, false);
        let start = std::sync::Barrier::new(8);
        std::thread::scope(|s| {
            for t in 0..8usize {
                let (l, c, start, p) = (&l, &c, &start, &p);
                s.spawn(move || {
                    start.wait();
                    // each thread from its own segment on, around the column
                    for k in 0..c.len() {
                        let r = (k + t * 25_000) % c.len();
                        assert_eq!(l.get(r), p.get(r));
                    }
                });
            }
        });
        assert_eq!(mem.reads.load(Ordering::Relaxed), 20, "each segment read once");
        let (loads, _, _) = cache.counters();
        assert_eq!(loads, 20);
    }

    #[test]
    fn a_shrunk_cache_keeps_to_its_bound_and_a_dropped_column_leaves_nothing() {
        let c = codes(40_000);
        let p = crate::tests::pack(&c, 2000);
        let one = largest(&p);
        let cache = SegCache::new(one * 8);
        let (l, mem) = lazy_copy(&p, &cache, false);
        for _ in 0..4 {
            pass(&l, &mem, &c);
        }
        assert!(cache.protected_bytes() > 0);
        for cap in [one * 5, one * 2, one, 0] {
            cache.set_cap(cap);
            assert!(cache.bytes() <= cap, "{} over {cap}", cache.bytes());
            assert!(cache.protected_bytes() <= cache.bytes());
            pass(&l, &mem, &c);
            assert!(cache.bytes() <= cap.max(1) || cap == 0 && cache.bytes() == 0);
        }
        cache.set_cap(one * 8);
        pass(&l, &mem, &c);
        drop(l);
        assert!(cache.is_empty());
        assert_eq!((cache.bytes(), cache.protected_bytes()), (0, 0));
    }

    #[test]
    fn segments_offered_by_the_first_pass_are_kept_while_they_fit() {
        let c = codes(40_000);
        let p = crate::tests::pack(&c, 2000);
        let one = largest(&p);
        let cache = SegCache::new(one * 5);
        let (l, mem) = lazy_copy(&p, &cache, false);
        let lazy = l.lazy_source().unwrap();
        for (i, s) in p.segments().iter().enumerate() {
            lazy.offer(i, s.data.clone());
            assert!(cache.bytes() <= one * 5);
        }
        assert!(cache.len() >= 4, "the first segments stay: {}", cache.len());
        assert_eq!(cache.counters(), (0, 0, 0), "offering evicts nothing");
        // the pass that follows finds them
        assert!(pass(&l, &mem, &c) <= 16);
        assert!(cache.counters().1 >= 4);
    }
}
