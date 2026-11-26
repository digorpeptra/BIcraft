//! Process-wide gauges of the engine's background work, read by `bicraft serve`'s `/metrics`
//! (docs/kubernetes.md, "Observability"): the spill files of backups moved out of memory under a
//! budget (`docs/beyond-ram.md`), and the background aggregation builds. Plain atomics, updated
//! where the work starts and ends; reading them takes no lock and never touches a session.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Bytes held by this process' spill files now.
pub(crate) static SPILL_BYTES: AtomicU64 = AtomicU64::new(0);
/// Aggregation builds running now.
static AGG_RUNNING: AtomicUsize = AtomicUsize::new(0);
/// Aggregation builds ended with tables, and ended without (cancelled, refused, failed).
static AGG_BUILT: AtomicU64 = AtomicU64::new(0);
static AGG_STOPPED: AtomicU64 = AtomicU64::new(0);

/// A snapshot of the gauges.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EngineGauges {
    /// Spill files alive now, and their bytes.
    pub spill_files: usize,
    pub spill_bytes: u64,
    /// Background aggregation builds running now.
    pub aggregation_builds_running: usize,
    /// Builds ended with aggregation tables since the process started.
    pub aggregation_builds_done: u64,
    /// Builds ended without (cancelled by a newer report, refused memory, failed).
    pub aggregation_builds_stopped: u64,
}

/// The engine's gauges now.
pub fn engine_gauges() -> EngineGauges {
    EngineGauges {
        spill_files: crate::preview::spill_files_alive(),
        spill_bytes: SPILL_BYTES.load(Ordering::Relaxed),
        aggregation_builds_running: AGG_RUNNING.load(Ordering::Relaxed),
        aggregation_builds_done: AGG_BUILT.load(Ordering::Relaxed),
        aggregation_builds_stopped: AGG_STOPPED.load(Ordering::Relaxed),
    }
}

/// A spill file's length went from `old` to `new` bytes.
pub(crate) fn spill_bytes_changed(old: usize, new: usize) {
    let (old, new) = (old as u64, new as u64);
    if new > old {
        SPILL_BYTES.fetch_add(new - old, Ordering::Relaxed);
    } else if old > new {
        // every byte removed was added before (the file only reports its own length)
        SPILL_BYTES.fetch_sub(old - new, Ordering::Relaxed);
    }
}

/// One background aggregation build in progress; ends as built or stopped when dropped.
pub(crate) struct AggBuild {
    built: bool,
}

impl AggBuild {
    pub(crate) fn start() -> Self {
        AGG_RUNNING.fetch_add(1, Ordering::Relaxed);
        Self { built: false }
    }

    pub(crate) fn built(&mut self) {
        self.built = true;
    }
}

impl Drop for AggBuild {
    fn drop(&mut self) {
        AGG_RUNNING.fetch_sub(1, Ordering::Relaxed);
        if self.built {
            AGG_BUILT.fetch_add(1, Ordering::Relaxed);
        } else {
            AGG_STOPPED.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gauges_move_with_the_work() {
        let before = engine_gauges();
        {
            let mut b = AggBuild::start();
            assert!(engine_gauges().aggregation_builds_running >= 1);
            b.built();
        }
        drop(AggBuild::start());
        let after = engine_gauges();
        assert!(after.aggregation_builds_done > before.aggregation_builds_done);
        assert!(after.aggregation_builds_stopped > before.aggregation_builds_stopped);
        // a length change moves the spill bytes by the difference (other tests may spill too)
        spill_bytes_changed(0, 1 << 40);
        assert!(SPILL_BYTES.load(Ordering::Relaxed) >= 1 << 40);
        spill_bytes_changed(1 << 40, 0);
    }
}
