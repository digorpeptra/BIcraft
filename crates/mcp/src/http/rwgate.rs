//! The lock of a workspace's session (`docs/web-deployment.md`, "Concurrency"): read-only tool
//! calls run side by side under its read side (at most `--max-concurrent-reads` at once), every
//! other call runs alone under its write side.
//!
//! The policy lives in a small gate (a mutex and a condition variable), not in the platform's
//! read-write lock, whose fairness is unspecified: it **prefers writers**. A writer that waits
//! stops new readers from entering, so the running reads drain and the writer goes next; a
//! stream of reads can never starve an edit. The data itself sits in a `std::sync::RwLock` that
//! the gate only ever lets in compatibly (readers together, or one writer), so it never blocks
//! for long.
//!
//! A blocking [`SessionLock::read`] or [`SessionLock::write`] counts itself in a caller's gauge
//! while it waits at the gate (`bicraft_tool_calls_queued`): under the gate's mutex, after a writer
//! has announced itself, so a count seen means the call is in the queue the policy orders.

use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Gate {
    readers: usize,
    writer: bool,
    writers_waiting: usize,
}

pub(super) struct SessionLock<T> {
    gate: Mutex<Gate>,
    changed: Condvar,
    data: RwLock<T>,
    max_readers: usize,
}

/// Shared access; the read slot is released when dropped (fields drop in order: the data
/// guard, then the slot).
pub(super) struct ReadGuard<'a, T> {
    guard: RwLockReadGuard<'a, T>,
    _slot: Slot<'a, T>,
}

/// Exclusive access; released when dropped.
pub(super) struct WriteGuard<'a, T> {
    guard: RwLockWriteGuard<'a, T>,
    _slot: Slot<'a, T>,
}

/// A place taken in the gate: a reader's (`write` false) or the writer's.
struct Slot<'a, T> {
    lock: &'a SessionLock<T>,
    write: bool,
}

impl<T> SessionLock<T> {
    /// A lock letting at most `max_readers` (at least one) readers in at once.
    pub(super) fn new(value: T, max_readers: usize) -> Self {
        Self {
            gate: Mutex::new(Gate::default()),
            changed: Condvar::new(),
            data: RwLock::new(value),
            max_readers: max_readers.max(1),
        }
    }

    fn gate(&self) -> std::sync::MutexGuard<'_, Gate> {
        self.gate.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Waits on the gate until `ready`, at most `wait` (`None`: no limit); `false` on time out.
    /// While it waits it counts itself in `queued` (raised and lowered under the gate's mutex).
    fn wait_for(
        &self,
        mut g: std::sync::MutexGuard<'_, Gate>,
        wait: Option<Duration>,
        queued: Option<&AtomicUsize>,
        ready: impl Fn(&Gate) -> bool,
        enter: impl FnOnce(&mut Gate),
        give_up: impl FnOnce(&mut Gate),
    ) -> bool {
        let deadline = wait.and_then(|w| Instant::now().checked_add(w));
        let queued = queued.filter(|_| !ready(&g));
        if let Some(q) = queued {
            q.fetch_add(1, Ordering::SeqCst);
        }
        while !ready(&g) {
            g = match deadline {
                None => self.changed.wait(g).unwrap_or_else(PoisonError::into_inner),
                Some(d) => {
                    let left = d.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        give_up(&mut g);
                        if let Some(q) = queued {
                            q.fetch_sub(1, Ordering::SeqCst);
                        }
                        drop(g);
                        self.changed.notify_all();
                        return false;
                    }
                    self.changed
                        .wait_timeout(g, left)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
            };
        }
        enter(&mut g);
        if let Some(q) = queued {
            q.fetch_sub(1, Ordering::SeqCst);
        }
        true
    }

    /// Shared access, waiting as long as it takes; counted in `queued` while it waits.
    pub(super) fn read(&self, queued: Option<&AtomicUsize>) -> ReadGuard<'_, T> {
        let max = self.max_readers;
        self.wait_for(
            self.gate(),
            None,
            queued,
            |g| !g.writer && g.writers_waiting == 0 && g.readers < max,
            |g| g.readers = g.readers.saturating_add(1),
            |_| {},
        );
        ReadGuard {
            guard: self.data.read().unwrap_or_else(PoisonError::into_inner),
            _slot: Slot {
                lock: self,
                write: false,
            },
        }
    }

    /// Exclusive access, waiting as long as it takes; counted in `queued` while it waits.
    pub(super) fn write(&self, queued: Option<&AtomicUsize>) -> WriteGuard<'_, T> {
        let mut g = self.gate();
        g.writers_waiting = g.writers_waiting.saturating_add(1);
        self.wait_for(
            g,
            None,
            queued,
            |g| !g.writer && g.readers == 0,
            |g| {
                g.writers_waiting = g.writers_waiting.saturating_sub(1);
                g.writer = true;
            },
            |g| g.writers_waiting = g.writers_waiting.saturating_sub(1),
        );
        WriteGuard {
            guard: self.data.write().unwrap_or_else(PoisonError::into_inner),
            _slot: Slot {
                lock: self,
                write: true,
            },
        }
    }

    /// Shared access: waits while a writer runs or waits, or the reads are at their cap.
    pub(super) fn read_within(&self, wait: Option<Duration>) -> Option<ReadGuard<'_, T>> {
        let max = self.max_readers;
        let ok = self.wait_for(
            self.gate(),
            wait,
            None,
            |g| !g.writer && g.writers_waiting == 0 && g.readers < max,
            |g| g.readers = g.readers.saturating_add(1),
            |_| {},
        );
        ok.then(|| ReadGuard {
            guard: self.data.read().unwrap_or_else(PoisonError::into_inner),
            _slot: Slot {
                lock: self,
                write: false,
            },
        })
    }

    /// Exclusive access: announces itself (new readers wait), then waits for the running ones.
    pub(super) fn write_within(&self, wait: Option<Duration>) -> Option<WriteGuard<'_, T>> {
        let mut g = self.gate();
        g.writers_waiting = g.writers_waiting.saturating_add(1);
        let ok = self.wait_for(
            g,
            wait,
            None,
            |g| !g.writer && g.readers == 0,
            |g| {
                g.writers_waiting = g.writers_waiting.saturating_sub(1);
                g.writer = true;
            },
            |g| g.writers_waiting = g.writers_waiting.saturating_sub(1),
        );
        ok.then(|| WriteGuard {
            guard: self.data.write().unwrap_or_else(PoisonError::into_inner),
            _slot: Slot {
                lock: self,
                write: true,
            },
        })
    }

    /// Exclusive access only if nobody holds or waits for the lock now.
    pub(super) fn try_write(&self) -> Option<WriteGuard<'_, T>> {
        let mut g = self.gate();
        if g.writer || g.readers > 0 || g.writers_waiting > 0 {
            return None;
        }
        g.writer = true;
        drop(g);
        Some(WriteGuard {
            guard: self.data.write().unwrap_or_else(PoisonError::into_inner),
            _slot: Slot {
                lock: self,
                write: true,
            },
        })
    }

    /// Nobody holds or waits for the lock.
    pub(super) fn is_free(&self) -> bool {
        let g = self.gate();
        !g.writer && g.readers == 0 && g.writers_waiting == 0
    }

    /// Readers in now.
    #[cfg(test)]
    pub(super) fn readers(&self) -> usize {
        self.gate().readers
    }
}

impl<T> Drop for Slot<'_, T> {
    fn drop(&mut self) {
        let mut g = self.lock.gate();
        if self.write {
            g.writer = false;
        } else {
            g.readers = g.readers.saturating_sub(1);
        }
        drop(g);
        self.lock.changed.notify_all();
    }
}

impl<T> Deref for ReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> Deref for WriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> DerefMut for WriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn readers_share_up_to_the_cap_and_a_writer_waits_for_them() {
        let l = SessionLock::new(0u32, 2);
        let a = l.read_within(None).unwrap();
        let b = l.read_within(None).unwrap();
        assert_eq!(l.readers(), 2);
        assert!(
            l.read_within(Some(Duration::from_millis(20))).is_none(),
            "at the cap"
        );
        assert!(
            l.write_within(Some(Duration::from_millis(20))).is_none(),
            "readers hold it"
        );
        drop((a, b));
        let mut w = l.write_within(Some(Duration::from_millis(20))).unwrap();
        *w = 7;
        assert!(
            l.read_within(Some(Duration::from_millis(20))).is_none(),
            "the writer holds it"
        );
        drop(w);
        assert_eq!(*l.read_within(None).unwrap(), 7);
        assert!(l.is_free());
    }

    /// A waiting writer stops new readers: under a stream of reads it still gets in.
    #[test]
    fn a_waiting_writer_is_not_starved() {
        let l = Arc::new(SessionLock::new(0u64, 4));
        let first = l.read_within(None).unwrap();
        let done = Arc::new(AtomicUsize::new(0));
        std::thread::scope(|sc| {
            let (l2, d2) = (Arc::clone(&l), Arc::clone(&done));
            sc.spawn(move || {
                let mut w = l2.write_within(None).unwrap();
                *w += 1;
                d2.store(1, Ordering::SeqCst);
            });
            // the writer announces itself, then no new reader gets in
            while l.gate().writers_waiting == 0 {
                std::thread::yield_now();
            }
            assert!(l.read_within(Some(Duration::from_millis(30))).is_none());
            drop(first);
        });
        assert_eq!(done.load(Ordering::SeqCst), 1);
        assert_eq!(*l.read_within(None).unwrap(), 1);
    }

    /// The queued gauge counts a call only while it waits at the gate, and a writer it counts has
    /// announced itself (a read that comes after waits behind it).
    #[test]
    fn the_queued_gauge_counts_calls_waiting_at_the_gate() {
        let l = SessionLock::new((), 4);
        let queued = AtomicUsize::new(0);
        let first = l.read(Some(&queued));
        assert_eq!(queued.load(Ordering::SeqCst), 0, "a read that got in at once");
        std::thread::scope(|sc| {
            let writer = sc.spawn(|| drop(l.write(Some(&queued))));
            while queued.load(Ordering::SeqCst) == 0 {
                std::thread::yield_now();
            }
            assert!(
                l.gate().writers_waiting > 0,
                "a counted writer has announced itself"
            );
            let reader = sc.spawn(|| drop(l.read(Some(&queued))));
            while queued.load(Ordering::SeqCst) < 2 {
                std::thread::yield_now();
            }
            assert!(!writer.is_finished() && !reader.is_finished());
            drop(first);
        });
        assert_eq!(queued.load(Ordering::SeqCst), 0, "nobody waits any more");
    }

    #[test]
    fn a_timed_out_writer_lets_readers_in_again() {
        let l = SessionLock::new((), 1);
        let r = l.read_within(None).unwrap();
        assert!(l.write_within(Some(Duration::from_millis(10))).is_none());
        drop(r);
        assert!(l.read_within(Some(Duration::from_millis(10))).is_some());
        assert!(l.try_write().is_some());
    }
}
