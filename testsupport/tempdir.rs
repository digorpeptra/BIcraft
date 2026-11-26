//! Test-only temporary directory guard, shared by every crate's tests.
//!
//! Not a crate: each test module includes this file with a `#[path]` attribute, so it adds no
//! dependency edge and `cargo xtask layers` is unaffected. From a crate's `src/lib.rs`:
//!
//! ```ignore
//! #[cfg(test)]
//! #[path = "../../../testsupport/tempdir.rs"]
//! mod tempdir;
//! ```
//!
//! and from an integration test `tests/foo.rs`:
//!
//! ```ignore
//! #[path = "../../../testsupport/tempdir.rs"]
//! mod tempdir;
//! ```
//!
//! [`TempDir::new`] creates a fresh, unique directory under [`std::env::temp_dir`]; dropping the
//! guard removes it with everything inside, so a test cleans up on success and on panic alike
//! (only a test killed without unwinding leaves its directory behind). Removal errors are ignored.
//!
//! Every test that needs scratch space uses this guard. Deliberate exceptions, which keep files
//! because a later step reads them:
//! - the `fuzz_smoke` tests save the input of a failing case as
//!   `<temp>/bicraft-fuzz-smoke-<target>-<kind>.bin` (fixed names, only on failure) so it can be
//!   replayed;
//! - the env-gated real-data tests in `crates/pbix/tests/real_*.rs` and
//!   `crates/metadata/tests/real_structural.rs` write fixed-name outputs to
//!   `$BICRAFT_TESTDATA/m5/` for a manual check in Power BI Desktop;
//! - `BICRAFT_FUZZ_SEED_DIR`, `BICRAFT_LARGE_REWRITE` and `BICRAFT_ORACLE_REPORT` write where
//!   the caller points them.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A directory under the system temp folder, removed (recursively) when dropped.
///
/// Derefs to [`Path`], so `dir.join("x")`, `dir.display()` and `&dir` (as `&Path`) all work.
/// Keep the guard alive for as long as the directory is needed: `TempDir::new("x").join("y")`
/// drops it, and removes the directory, at the end of the statement.
#[derive(Debug)]
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Creates `bicraft-<name>-<pid>-<counter>-<nanos>` under the temp folder. `name` is
    /// sanitized to `[A-Za-z0-9_-]`. Panics if the directory cannot be created.
    pub fn new(name: &str) -> TempDir {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let safe: String = name
            .chars()
            .take(48)
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let path = std::env::temp_dir().join(format!("bicraft-{safe}-{}-{n}-{nanos}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        if let Err(e) = std::fs::create_dir_all(&path) {
            panic!("create temp dir {}: {e}", path.display());
        }
        TempDir { path }
    }

    /// The directory.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl std::ops::Deref for TempDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<std::ffi::OsStr> for TempDir {
    fn as_ref(&self) -> &std::ffi::OsStr {
        self.path.as_os_str()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // A few retries: on Windows a file a child process just closed can stay locked briefly.
        for attempt in 0..4u64 {
            if std::fs::remove_dir_all(&self.path).is_ok() || !self.path.exists() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(25 * (attempt + 1)));
        }
    }
}
