//! The worker pool every parallel scan runs on (`docs/storage-engine.md`, "Worker pool").
//!
//! Starting and joining threads per scan costs about 1.8 ms for 20 threads on Windows (0.2 ms on
//! Linux), more than a whole scan of a million rows takes. The workers are therefore started once
//! per process, on first use, and stay parked between scans; waking them costs tens of
//! microseconds. The pool is `rayon-core`'s (its `unsafe` stays inside that crate; this crate
//! keeps `forbid(unsafe_code)`), and nothing else in the workspace names it: every parallel loop
//! goes through [`run`], [`map`] or [`each`], so the pool can be swapped without touching a
//! kernel.
//!
//! - **Foreground pool**: one worker less than the scan threads (`BICRAFT_SCAN_THREADS`, else the
//!   available parallelism), since the thread that starts a scan runs one of its tasks itself.
//! - **Background pool**: a thread marked by [`set_background`] (the aggregation build's) runs
//!   its tasks on a small pool of its own, so a background job never queues work in front of a
//!   query's.
//! - A task that panics is caught: the loop reports a failure (`false` / `None`) and the panic
//!   goes no further. The pool's threads never die of it.
//! - When a pool cannot be started (the system refuses threads), the tasks run on scoped threads
//!   started for the call, as before the pool.

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

thread_local! {
    /// Threads of the background pool when this thread's loops go there ([`set_background`]).
    static BACKGROUND: Cell<Option<usize>> = const { Cell::new(None) };
}

static FOREGROUND_POOL: OnceLock<Option<rayon_core::ThreadPool>> = OnceLock::new();
static BACKGROUND_POOL: OnceLock<Option<rayon_core::ThreadPool>> = OnceLock::new();

/// Sends the parallel loops started from this thread to the background pool, sized for
/// `threads` threads in all (the calling thread included) when this call creates it; `None`
/// sends them back to the foreground pool.
pub fn set_background(threads: Option<usize>) {
    BACKGROUND.with(|b| b.set(threads.map(|n| n.clamp(1, 256))));
}

/// Whether this thread's loops go to the background pool.
pub fn is_background() -> bool {
    BACKGROUND.with(Cell::get).is_some()
}

/// The machine's scan threads for the whole process (`BICRAFT_SCAN_THREADS`, else the available
/// parallelism), at most 256.
pub(crate) fn process_threads() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| {
        bicraft_env::var("BICRAFT_SCAN_THREADS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|n| *n >= 1)
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
            .min(256)
    })
}

fn build(name: &'static str, workers: usize, background: Option<usize>) -> Option<rayon_core::ThreadPool> {
    rayon_core::ThreadPoolBuilder::new()
        .num_threads(workers.max(1))
        .thread_name(move |i| format!("bicraft-{name}-{i}"))
        // a task that starts a loop of its own (nested) stays in the same pool
        .start_handler(move |_| BACKGROUND.with(|b| b.set(background)))
        .build()
        .ok()
}

/// The pool this thread's loops run on; `None` when it could not be started.
fn pool() -> Option<&'static rayon_core::ThreadPool> {
    match BACKGROUND.with(Cell::get) {
        Some(n) => BACKGROUND_POOL
            .get_or_init(|| build("bg", n.saturating_sub(1), Some(n)))
            .as_ref(),
        None => FOREGROUND_POOL
            .get_or_init(|| build("scan", process_threads().saturating_sub(1), None))
            .as_ref(),
    }
}

/// Runs `task(i)` for every `i` in `0..n`, in parallel: the calling thread runs `task(0)` and
/// the pool the others. Returns when all are done; `false` when one of them panicked.
pub fn run(n: usize, task: impl Fn(usize) + Sync) -> bool {
    let failed = AtomicBool::new(false);
    let one = |i: usize| {
        if catch_unwind(AssertUnwindSafe(|| task(i))).is_err() {
            failed.store(true, Ordering::Relaxed);
        }
    };
    let one = &one;
    if n <= 1 {
        (0..n).for_each(one);
    } else if let Some(p) = pool() {
        p.in_place_scope(|sc| fan(sc, 0, n, one));
    } else {
        std::thread::scope(|sc| {
            for i in 1..n {
                if std::thread::Builder::new()
                    .spawn_scoped(sc, move || one(i))
                    .is_err()
                {
                    one(i);
                }
            }
            one(0);
        });
    }
    !failed.load(Ordering::Relaxed)
}

/// Runs `one(i)` for `i` in `lo..hi`: hands the upper half to another worker, again and again,
/// and runs `one(lo)` itself. Each woken worker wakes the next ones, so starting `n` tasks
/// takes about log2(n) wake-ups in a row instead of `n` (a wake-up costs ~70 µs in a WSL 2 VM).
fn fan<'s, F: Fn(usize) + Sync>(sc: &rayon_core::Scope<'s>, lo: usize, mut hi: usize, one: &'s F) {
    while hi.saturating_sub(lo) > 1 {
        let mid = lo + (hi - lo) / 2;
        sc.spawn(move |sc| fan(sc, mid, hi, one));
        hi = mid;
    }
    one(lo);
}

/// The shortest of a few loops of two empty tasks, in nanoseconds, measured once per process: what
/// handing work to one parked worker and waiting for it costs on this machine (a few µs on
/// Windows, ~120 µs in a WSL 2 VM). The first loop, which may start the pool, is not counted.
pub fn wake_round_trip_ns() -> u64 {
    static NS: OnceLock<u64> = OnceLock::new();
    *NS.get_or_init(|| {
        run(2, |_| {});
        (0..8)
            .map(|_| {
                let t = std::time::Instant::now();
                run(2, |_| {});
                u64::try_from(t.elapsed().as_nanos()).unwrap_or(u64::MAX)
            })
            .min()
            .unwrap_or(0)
    })
}

/// `task(i)` for every `i` in `0..n`, in parallel ([`run`]), the results in order of `i`;
/// `None` when a task panicked.
pub fn map<T: Send>(n: usize, task: impl Fn(usize) -> T + Sync) -> Option<Vec<T>> {
    let slots: Vec<Mutex<Option<T>>> = (0..n).map(|_| Mutex::new(None)).collect();
    let ok = run(n, |i| {
        let v = task(i);
        if let Some(Ok(mut s)) = slots.get(i).map(Mutex::lock) {
            *s = Some(v);
        }
    });
    if !ok {
        return None;
    }
    slots.into_iter().map(|s| s.into_inner().ok().flatten()).collect()
}

/// `task(i, item)` for every item of `items`, each moved to its task, in parallel ([`run`]);
/// `false` when a task panicked.
pub fn each<T: Send>(items: Vec<T>, task: impl Fn(usize, T) + Sync) -> bool {
    let slots: Vec<Mutex<Option<T>>> = items.into_iter().map(|t| Mutex::new(Some(t))).collect();
    run(slots.len(), |i| {
        let item = slots
            .get(i)
            .and_then(|s| s.lock().ok().and_then(|mut g| g.take()));
        if let Some(item) = item {
            task(i, item);
        }
    })
}

#[cfg(test)]
#[path = "pool_tests.rs"]
mod tests;
