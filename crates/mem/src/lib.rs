//! The process-wide memory limit (`docs/beyond-ram.md`, lot 5).
//!
//! With `BICRAFT_MEMORY_LIMIT` set (or [`set_limit`]), BIcraft's big allocations reserve their
//! bytes against one limit before they are made: decoded columns and what a session keeps
//! between queries ([`Class::Kept`]), a query's formula-engine tables, scan states and answer
//! ([`Class::Query`]), background aggregation builds ([`Class::Background`]) and decompression
//! buffers ([`Class::Io`]). A reservation that does not fit first asks for memory to be freed
//! (the caller's own session, then every registered [`Evictor`]), then is refused: the caller
//! fails cleanly (a query error naming what needed the memory and the limit), never allocates.
//!
//! - **Without a limit** (the default) nothing is counted: a reservation is two words and no
//!   atomic operation.
//! - **Fair shares.** A query's reservations are admitted while its total stays within its share:
//!   what the limit leaves after the kept data, the background builds and the decompression
//!   buffers, divided by the queries running at that moment ([`QueryTicket`]). A query alone may
//!   use all of it.
//! - **Background work yields.** Background reservations are admitted only while the total stays
//!   under three quarters of the limit, and are never given memory by eviction.
//! - What is not counted (the program, the model's metadata, the package's small parts, thread
//!   stacks, the allocator's overhead, small tables) is the fixed base a deployment adds to the
//!   limit (`docs/beyond-ram.md`, "Lot 5").
//!
//! A foundation crate: no workspace dependency (`cargo xtask layers`); it reads its one variable
//! with `std::env` (a name introduced after the rename, without a pre-rename fallback).

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

/// What a reservation is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Class {
    /// What a session keeps between queries: decoded columns, segment and storage caches,
    /// aggregations.
    Kept,
    /// One query's working memory.
    Query,
    /// Background work (aggregation builds): admitted under three quarters of the limit only.
    Background,
    /// Decompression and file buffers.
    Io,
}

impl Class {
    pub const ALL: [Class; 4] = [Class::Kept, Class::Query, Class::Background, Class::Io];

    fn index(self) -> usize {
        match self {
            Class::Kept => 0,
            Class::Query => 1,
            Class::Background => 2,
            Class::Io => 3,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Class::Kept => "kept",
            Class::Query => "query",
            Class::Background => "background",
            Class::Io => "io",
        }
    }
}

/// Bytes with an optional binary `K`, `M`, `G` or `T` suffix (`300M` = 300 MiB, `64KiB`, `1MB`);
/// `0`, `unlimited`, `none` or an empty value: `Ok(None)`.
pub fn parse_bytes(text: &str) -> Result<Option<u64>, String> {
    let t = text.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("unlimited") || t.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let lower = t.to_ascii_lowercase();
    let body = lower
        .strip_suffix("ib")
        .or_else(|| lower.strip_suffix('b'))
        .unwrap_or(&lower);
    let (digits, shift) = [('k', 10u32), ('m', 20), ('g', 30), ('t', 40)]
        .iter()
        .find_map(|(c, s)| body.strip_suffix(*c).map(|d| (d, *s)))
        .unwrap_or((body, 0));
    let digits = digits.trim().replace('_', "");
    let n: u64 = digits
        .parse()
        .map_err(|_| format!("`{text}`: expected bytes, optionally with K, M, G or T"))?;
    let bytes = n
        .checked_mul(1u64 << shift)
        .ok_or_else(|| format!("`{text}` is too large"))?;
    Ok((bytes > 0).then_some(bytes))
}

/// A size for messages: MB, or bytes below one MB.
pub fn size_text(bytes: usize) -> String {
    if bytes < (1 << 20) {
        format!("{bytes} bytes")
    } else {
        format!("{:.1} MB", bytes as f64 / (1u64 << 20) as f64)
    }
}

/// Something that can free memory it keeps when the limit is reached (a session's kept columns
/// and caches). `evict` frees about `want` bytes if it can (without waiting for a lock another
/// thread holds) and returns the bytes it released from its reservations.
pub trait Evictor: Send + Sync {
    fn evict(&self, want: usize) -> usize;
}

/// A refused reservation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub class: Class,
    /// Bytes asked for.
    pub wanted: usize,
    /// Bytes reserved in the process when it was refused.
    pub used: usize,
    pub limit: usize,
    /// For a query over its fair share: the share and the queries sharing the limit.
    pub share: Option<(usize, usize)>,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.share {
            Some((share, queries)) => write!(
                f,
                "over this query's share of the memory limit ({} of {} with {queries} queries running; \
                 BICRAFT_MEMORY_LIMIT)",
                size_text(share),
                size_text(self.limit)
            ),
            None => write!(
                f,
                "over the memory limit of {} ({} already reserved; BICRAFT_MEMORY_LIMIT)",
                size_text(self.limit),
                size_text(self.used)
            ),
        }
    }
}

/// Counters of a [`Pool`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub limit: Option<usize>,
    pub used: usize,
    pub peak: usize,
    /// Reserved bytes per class, in [`Class::ALL`] order: kept, query, background, io.
    pub by_class: [usize; 4],
    pub refused: u64,
    pub evictions: u64,
    pub active_queries: usize,
}

impl Stats {
    pub fn class(&self, c: Class) -> usize {
        self.by_class.get(c.index()).copied().unwrap_or(0)
    }
}

/// Setting of a pool's limit: follow the environment.
const FOLLOW: usize = 0;
/// Setting of a pool's limit: no limit.
const NONE: usize = usize::MAX;

/// An accounting of reserved bytes against a limit. The process has one ([`global`]); tests
/// make their own.
pub struct Pool {
    setting: AtomicUsize,
    /// The environment's limit, read once (only the global pool reads it).
    env: Option<OnceLock<Option<usize>>>,
    used: AtomicUsize,
    peak: AtomicUsize,
    by_class: [AtomicUsize; 4],
    refused: AtomicU64,
    evictions: AtomicU64,
    active: AtomicUsize,
    evictors: Mutex<Vec<Weak<dyn Evictor>>>,
}

impl Pool {
    /// A pool with no limit that ignores the environment.
    pub const fn new() -> Pool {
        Pool {
            setting: AtomicUsize::new(FOLLOW),
            env: None,
            used: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            by_class: [
                AtomicUsize::new(0),
                AtomicUsize::new(0),
                AtomicUsize::new(0),
                AtomicUsize::new(0),
            ],
            refused: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
            active: AtomicUsize::new(0),
            evictors: Mutex::new(Vec::new()),
        }
    }

    const fn from_environment() -> Pool {
        Pool {
            setting: AtomicUsize::new(FOLLOW),
            env: Some(OnceLock::new()),
            used: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            by_class: [
                AtomicUsize::new(0),
                AtomicUsize::new(0),
                AtomicUsize::new(0),
                AtomicUsize::new(0),
            ],
            refused: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
            active: AtomicUsize::new(0),
            evictors: Mutex::new(Vec::new()),
        }
    }

    /// The limit in bytes; `None`: nothing is counted.
    pub fn limit(&self) -> Option<usize> {
        match self.setting.load(Ordering::Relaxed) {
            FOLLOW => self.env.as_ref().and_then(|e| {
                *e.get_or_init(|| {
                    std::env::var("BICRAFT_MEMORY_LIMIT")
                        .ok()
                        .and_then(|v| parse_bytes(&v).ok().flatten())
                        .map(|b| usize::try_from(b).unwrap_or(usize::MAX - 1))
                })
            }),
            NONE => None,
            b => Some(b),
        }
    }

    /// Sets the limit (`None`: none), overriding the environment. Reservations made before keep
    /// counting as they were made: set it before the work it should bound.
    pub fn set_limit(&self, limit: Option<usize>) {
        let v = match limit {
            Some(0) | None => NONE,
            Some(b) => b.min(usize::MAX - 1),
        };
        self.setting.store(v, Ordering::Relaxed);
    }

    /// Back to the environment's limit.
    pub fn follow_environment(&self) {
        self.setting.store(FOLLOW, Ordering::Relaxed);
    }

    pub fn stats(&self) -> Stats {
        let mut by_class = [0usize; 4];
        for (o, a) in by_class.iter_mut().zip(&self.by_class) {
            *o = a.load(Ordering::Relaxed);
        }
        Stats {
            limit: self.limit(),
            used: self.used.load(Ordering::Relaxed),
            peak: self.peak.load(Ordering::Relaxed),
            by_class,
            refused: self.refused.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            active_queries: self.active.load(Ordering::Relaxed),
        }
    }

    /// Starts the peak over from what is reserved now.
    pub fn reset_peak(&self) {
        self.peak
            .store(self.used.load(Ordering::Relaxed), Ordering::Relaxed);
    }

    /// Registers something that frees memory under pressure (kept as a weak reference).
    pub fn register(&self, e: Weak<dyn Evictor>) {
        if let Ok(mut g) = self.evictors.lock() {
            g.retain(|w| w.strong_count() > 0);
            g.push(e);
        }
    }

    /// A query starts: it shares the limit with the others running until the ticket drops.
    pub fn enter_query(&'static self) -> QueryTicket {
        let counted = self.limit().is_some();
        if counted {
            self.active.fetch_add(1, Ordering::Relaxed);
        }
        QueryTicket { pool: self, counted }
    }

    /// An empty reservation of `class` (counts nothing until it grows).
    pub fn empty(&'static self, class: Class) -> Reservation {
        Reservation {
            pool: Some(self),
            class,
            counted: 0,
        }
    }

    /// Reserves `bytes`, without eviction.
    pub fn reserve(&'static self, class: Class, bytes: usize) -> Result<Reservation, Refused> {
        let mut r = self.empty(class);
        r.grow(bytes, None, &mut |_| 0)?;
        Ok(r)
    }

    /// The share of the limit one query may hold now.
    fn share(&self, limit: usize) -> (usize, usize) {
        let others = [Class::Kept, Class::Background, Class::Io]
            .iter()
            .map(|c| {
                self.by_class
                    .get(c.index())
                    .map_or(0, |a| a.load(Ordering::Relaxed))
            })
            .fold(0usize, usize::saturating_add);
        let queries = self.active.load(Ordering::Relaxed).max(1);
        (limit.saturating_sub(others) / queries, queries)
    }

    /// Adds `bytes` of `class` when they fit (`owner_total`: what the query holds with them).
    fn admit(&self, class: Class, bytes: usize, owner_total: Option<usize>) -> Result<(), Refused> {
        let Some(limit) = self.limit() else {
            return Ok(());
        };
        let refuse = |used: usize, share: Option<(usize, usize)>| Refused {
            class,
            wanted: bytes,
            used,
            limit,
            share,
        };
        if class == Class::Query
            && let Some(total) = owner_total
        {
            let (share, queries) = self.share(limit);
            if total > share && queries > 1 {
                return Err(refuse(self.used.load(Ordering::Relaxed), Some((share, queries))));
            }
        }
        let cap = match class {
            Class::Background => limit.saturating_sub(limit / 4),
            _ => limit,
        };
        let mut cur = self.used.load(Ordering::Relaxed);
        loop {
            let new = cur.saturating_add(bytes);
            if new > cap {
                return Err(refuse(cur, None));
            }
            match self
                .used
                .compare_exchange_weak(cur, new, Ordering::AcqRel, Ordering::Relaxed)
            {
                Ok(_) => {
                    if let Some(a) = self.by_class.get(class.index()) {
                        a.fetch_add(bytes, Ordering::Relaxed);
                    }
                    self.peak.fetch_max(new, Ordering::Relaxed);
                    return Ok(());
                }
                Err(now) => cur = now,
            }
        }
    }

    /// Adds `bytes` whether they fit or not (memory already allocated, counted after the fact).
    fn force(&self, class: Class, bytes: usize) {
        let new = self.used.fetch_add(bytes, Ordering::AcqRel).saturating_add(bytes);
        if let Some(a) = self.by_class.get(class.index()) {
            a.fetch_add(bytes, Ordering::Relaxed);
        }
        self.peak.fetch_max(new, Ordering::Relaxed);
    }

    fn release(&self, class: Class, bytes: usize) {
        if bytes == 0 {
            return;
        }
        // never below zero, whatever the order of concurrent releases
        sub_floor(&self.used, bytes);
        if let Some(a) = self.by_class.get(class.index()) {
            sub_floor(a, bytes);
        }
    }

    /// Asks the registered evictors (other sessions) to free about `want` bytes.
    fn evict_others(&self, want: usize) -> usize {
        let live: Vec<Arc<dyn Evictor>> = match self.evictors.lock() {
            Ok(mut g) => {
                g.retain(|w| w.strong_count() > 0);
                g.iter().filter_map(Weak::upgrade).collect()
            }
            Err(_) => Vec::new(),
        };
        let mut freed = 0usize;
        for e in live {
            if freed >= want {
                break;
            }
            freed = freed.saturating_add(e.evict(want.saturating_sub(freed)));
        }
        freed
    }
}

impl Default for Pool {
    fn default() -> Self {
        Pool::new()
    }
}

/// Bytes reserved against a pool, released when dropped.
#[must_use]
pub struct Reservation {
    pool: Option<&'static Pool>,
    class: Class,
    /// Bytes added to the pool (0 while there is no limit).
    counted: usize,
}

impl std::fmt::Debug for Reservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reservation")
            .field("class", &self.class)
            .field("bytes", &self.counted)
            .finish()
    }
}

impl Default for Reservation {
    fn default() -> Self {
        Reservation {
            pool: None,
            class: Class::Query,
            counted: 0,
        }
    }
}

impl Reservation {
    /// Bytes it holds in its pool (0 without a limit).
    pub fn bytes(&self) -> usize {
        self.counted
    }

    pub fn class(&self) -> Class {
        self.class
    }

    /// Whether it belongs to a pool (a default reservation counts nothing, ever).
    pub fn is_bound(&self) -> bool {
        self.pool.is_some()
    }

    /// Grows by `more` bytes. When they do not fit: `local` frees what it can (the caller's own
    /// kept memory, called with the bytes missing), then the registered evictors (not for
    /// background work), then the reservation is refused (and stays as it was).
    /// `owner_total`: for a query, what it holds with these bytes (its fair share is checked).
    pub fn grow(
        &mut self,
        more: usize,
        owner_total: Option<usize>,
        local: &mut dyn FnMut(usize) -> usize,
    ) -> Result<(), Refused> {
        let Some(pool) = self.pool else {
            return Ok(());
        };
        if more == 0 || pool.limit().is_none() {
            return Ok(());
        }
        let mut last = match pool.admit(self.class, more, owner_total) {
            Ok(()) => {
                self.counted = self.counted.saturating_add(more);
                return Ok(());
            }
            Err(r) => r,
        };
        if self.class != Class::Background {
            let missing = |r: &Refused| {
                match r.share {
                    Some((share, _)) => owner_total.unwrap_or(r.wanted).saturating_sub(share),
                    None => r.used.saturating_add(r.wanted).saturating_sub(r.limit),
                }
                .max(1)
            };
            let steps: [&mut dyn FnMut(usize) -> usize; 2] = [local, &mut |w| pool.evict_others(w)];
            for step in steps {
                let freed = step(missing(&last));
                if freed > 0 {
                    pool.evictions.fetch_add(1, Ordering::Relaxed);
                }
                match pool.admit(self.class, more, owner_total) {
                    Ok(()) => {
                        self.counted = self.counted.saturating_add(more);
                        return Ok(());
                    }
                    Err(r) => last = r,
                }
            }
        }
        pool.refused.fetch_add(1, Ordering::Relaxed);
        Err(last)
    }

    /// Grows by `more` bytes when they fit, without asking anything to free memory (for callers
    /// holding locks an evictor may need).
    pub fn try_grow(&mut self, more: usize) -> Result<(), Refused> {
        let Some(pool) = self.pool else {
            return Ok(());
        };
        if more == 0 || pool.limit().is_none() {
            return Ok(());
        }
        match pool.admit(self.class, more, None) {
            Ok(()) => {
                self.counted = self.counted.saturating_add(more);
                Ok(())
            }
            // a soft refusal: the caller keeps less, nothing fails
            Err(r) => Err(r),
        }
    }

    /// Sets it to `bytes` whether they fit or not: memory already allocated, counted after the
    /// fact (or released).
    pub fn set(&mut self, bytes: usize) {
        let Some(pool) = self.pool else {
            return;
        };
        if bytes < self.counted {
            pool.release(self.class, self.counted.saturating_sub(bytes));
            self.counted = bytes;
        } else if bytes > self.counted && pool.limit().is_some() {
            pool.force(self.class, bytes.saturating_sub(self.counted));
            self.counted = bytes;
        }
    }

    /// Shrinks to at most `bytes`.
    pub fn shrink_to(&mut self, bytes: usize) {
        if bytes < self.counted {
            self.set(bytes);
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(p) = self.pool {
            p.release(self.class, self.counted);
        }
    }
}

/// A running query, sharing the limit with the others (see [`Pool::enter_query`]).
pub struct QueryTicket {
    pool: &'static Pool,
    counted: bool,
}

impl Drop for QueryTicket {
    fn drop(&mut self) {
        if self.counted {
            sub_floor(&self.pool.active, 1);
        }
    }
}

/// `a -= n`, never below zero.
fn sub_floor(a: &AtomicUsize, n: usize) {
    let mut cur = a.load(Ordering::Relaxed);
    loop {
        match a.compare_exchange_weak(cur, cur.saturating_sub(n), Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => return,
            Err(now) => cur = now,
        }
    }
}

static GLOBAL: Pool = Pool::from_environment();

/// The process' pool (its limit: `BICRAFT_MEMORY_LIMIT`, or [`set_limit`]).
pub fn global() -> &'static Pool {
    &GLOBAL
}

/// The process' limit (`None`: nothing is counted).
pub fn limit() -> Option<usize> {
    GLOBAL.limit()
}

/// Sets the process' limit (`None`: none), overriding `BICRAFT_MEMORY_LIMIT` (hosts embedding
/// BIcraft, tests).
pub fn set_limit(limit: Option<usize>) {
    GLOBAL.set_limit(limit);
}

/// The process' counters.
pub fn stats() -> Stats {
    GLOBAL.stats()
}

/// The process' peak resident memory in bytes, where the system reports it without `unsafe`
/// code (Linux: `VmHWM` in `/proc/self/status`); `None` elsewhere.
pub fn process_peak_bytes() -> Option<u64> {
    status_kib("VmHWM:").map(|k| k.saturating_mul(1024))
}

/// The process' resident memory in bytes now (Linux: `VmRSS`); `None` elsewhere.
pub fn process_resident_bytes() -> Option<u64> {
    status_kib("VmRSS:").map(|k| k.saturating_mul(1024))
}

fn status_kib(field: &str) -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/status").ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix(field))
        .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn pool(limit: Option<usize>) -> &'static Pool {
        let p: &'static Pool = Box::leak(Box::new(Pool::new()));
        p.set_limit(limit);
        p
    }

    #[test]
    fn bytes_parse_with_binary_suffixes() {
        assert_eq!(parse_bytes("300M"), Ok(Some(300 << 20)));
        assert_eq!(parse_bytes(" 2g "), Ok(Some(2 << 30)));
        assert_eq!(parse_bytes("1_000"), Ok(Some(1000)));
        assert_eq!(parse_bytes("64KiB"), Ok(Some(64 << 10)));
        assert_eq!(parse_bytes("1MB"), Ok(Some(1 << 20)));
        assert_eq!(parse_bytes("0"), Ok(None));
        assert_eq!(parse_bytes("unlimited"), Ok(None));
        assert_eq!(parse_bytes(""), Ok(None));
        assert!(parse_bytes("lots").is_err());
        assert!(parse_bytes("-5M").is_err());
        assert!(parse_bytes("99999999999T").is_err());
        assert!(parse_bytes("M").is_err());
    }

    #[test]
    fn without_a_limit_nothing_is_counted() {
        let p = pool(None);
        let r = p.reserve(Class::Query, usize::MAX).unwrap();
        assert_eq!(r.bytes(), 0);
        assert_eq!(p.stats().used, 0);
        let mut k = p.empty(Class::Kept);
        k.set(1 << 30);
        assert_eq!(k.bytes(), 0);
        let _t = p.enter_query();
        assert_eq!(p.stats().active_queries, 0);
    }

    #[test]
    fn reservations_count_until_dropped_and_refusals_change_nothing() {
        let p = pool(Some(1000));
        let a = p.reserve(Class::Kept, 600).unwrap();
        let e = p.reserve(Class::Query, 500).unwrap_err();
        assert_eq!(e.used, 600);
        assert_eq!(e.limit, 1000);
        assert!(e.to_string().contains("memory limit of 1000 bytes"), "{e}");
        assert_eq!(p.stats().used, 600);
        assert_eq!(p.stats().refused, 1);
        let mut b = p.reserve(Class::Query, 400).unwrap();
        assert_eq!(p.stats().used, 1000);
        assert_eq!(p.stats().peak, 1000);
        assert!(b.grow(1, None, &mut |_| 0).is_err());
        b.shrink_to(100);
        assert_eq!(p.stats().used, 700);
        assert_eq!(p.stats().class(Class::Query), 100);
        drop(a);
        drop(b);
        let s = p.stats();
        assert_eq!((s.used, s.by_class), (0, [0; 4]));
    }

    #[test]
    fn forced_bytes_count_beyond_the_limit_and_block_until_released() {
        let p = pool(Some(100));
        let mut k = p.empty(Class::Kept);
        k.set(150);
        assert_eq!(p.stats().used, 150);
        assert!(p.reserve(Class::Io, 1).is_err());
        k.set(50);
        assert!(p.reserve(Class::Io, 50).is_ok());
    }

    struct Freer {
        held: Mutex<Option<Reservation>>,
        calls: AtomicUsize,
    }

    impl Evictor for Freer {
        fn evict(&self, _want: usize) -> usize {
            self.calls.fetch_add(1, Ordering::Relaxed);
            match self.held.lock().unwrap().take() {
                Some(r) => r.bytes(),
                None => 0,
            }
        }
    }

    #[test]
    fn eviction_comes_first_local_then_others_and_background_gets_none() {
        let p = pool(Some(1000));
        let other = Arc::new(Freer {
            held: Mutex::new(Some(p.reserve(Class::Kept, 500).unwrap())),
            calls: AtomicUsize::new(0),
        });
        let weak: Weak<dyn Evictor> = Arc::downgrade(&(other.clone() as Arc<dyn Evictor>));
        p.register(weak);
        let mut mine = Some(p.reserve(Class::Kept, 300).unwrap());
        // background: refused without eviction (500 + 300 + 100 > 750)
        assert!(p.reserve(Class::Background, 100).is_err());
        assert_eq!(other.calls.load(Ordering::Relaxed), 0);
        // a query of 400: the local step frees 300, enough
        let mut q = p.empty(Class::Query);
        let local_called = AtomicBool::new(false);
        q.grow(400, Some(400), &mut |_| {
            local_called.store(true, Ordering::Relaxed);
            mine.take().map_or(0, |r| r.bytes())
        })
        .unwrap();
        assert!(local_called.load(Ordering::Relaxed));
        assert_eq!(other.calls.load(Ordering::Relaxed), 0);
        // 300 more: nothing local, the other session frees its 500
        q.grow(300, Some(700), &mut |_| 0).unwrap();
        assert_eq!(other.calls.load(Ordering::Relaxed), 1);
        assert_eq!(p.stats().used, 700);
        // nothing left to free: refused, nothing changed
        assert!(q.grow(400, Some(1100), &mut |_| 0).is_err());
        assert_eq!(p.stats().used, 700);
        assert!(p.stats().evictions >= 2);
        drop(q);
        assert_eq!(p.stats().used, 0);
    }

    #[test]
    fn queries_share_what_the_kept_data_leaves() {
        let p = pool(Some(1000));
        let _kept = p.reserve(Class::Kept, 200).unwrap();
        let t1 = p.enter_query();
        let mut a = p.empty(Class::Query);
        // alone: all of what is left
        a.grow(700, Some(700), &mut |_| 0).unwrap();
        a.shrink_to(300);
        let t2 = p.enter_query();
        assert_eq!(p.stats().active_queries, 2);
        // two queries: 400 each
        let mut b = p.empty(Class::Query);
        let e = b.grow(450, Some(450), &mut |_| 0).unwrap_err();
        assert_eq!(e.share, Some((400, 2)));
        assert!(e.to_string().contains("share"), "{e}");
        b.grow(400, Some(400), &mut |_| 0).unwrap();
        assert!(a.grow(150, Some(450), &mut |_| 0).is_err());
        a.grow(100, Some(400), &mut |_| 0).unwrap();
        drop(t2);
        drop(b);
        assert_eq!(p.stats().active_queries, 1);
        a.grow(400, Some(800), &mut |_| 0).unwrap();
        drop(t1);
        drop(a);
        assert_eq!(p.stats().class(Class::Query), 0);
        assert_eq!(p.stats().active_queries, 0);
    }

    #[test]
    fn concurrent_reservations_never_pass_the_limit() {
        let p = pool(Some(10_000));
        let over = AtomicBool::new(false);
        std::thread::scope(|s| {
            for t in 0..8usize {
                let over = &over;
                s.spawn(move || {
                    let mut held = Vec::new();
                    for i in 0..2000usize {
                        if let Ok(r) = p.reserve(Class::Query, 1 + (i * 7 + t) % 300) {
                            held.push(r);
                        }
                        if p.stats().used > 10_000 {
                            over.store(true, Ordering::Relaxed);
                        }
                        if held.len() > 5 {
                            drop(held.remove(0));
                        }
                    }
                });
            }
        });
        assert!(!over.load(Ordering::Relaxed));
        assert!(p.stats().peak <= 10_000);
        assert_eq!(p.stats().used, 0);
    }

    #[test]
    fn the_limit_can_be_lifted_and_set_again() {
        let p = pool(Some(10));
        assert_eq!(p.limit(), Some(10));
        p.set_limit(None);
        assert_eq!(p.limit(), None);
        p.set_limit(Some(0));
        assert_eq!(p.limit(), None);
        p.follow_environment();
        assert_eq!(p.limit(), None, "a test pool ignores the environment");
    }

    #[test]
    fn the_process_memory_reads_where_the_system_has_it() {
        let peak = process_peak_bytes();
        if cfg!(target_os = "linux") {
            assert!(peak.unwrap() > 0);
            assert!(process_resident_bytes().unwrap() > 0);
        } else {
            assert!(peak.is_none());
        }
    }
}
