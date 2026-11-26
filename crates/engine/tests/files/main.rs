//! The engine's file tests that need only its public API: saving, converting, diffing, new
//! packages, the PBIP data cache, opening in Desktop and auto-recovery. An integration test links
//! the library already built instead of compiling it again under `cfg(test)` (docs/ci.md, "Where
//! engine tests go"). Each file keeps the module path it had inside the crate, so the test names
//! do not change.
//! `#[path = "."]` on an inline module keeps its files next to `main.rs` (a path through the
//! module's missing directory, `module/../file.rs`, does not resolve on Linux).

#[path = "../../../../testsupport/tempdir.rs"]
mod tempdir;

mod diagram_tests;
mod save_tests;

#[path = "."]
mod cache_abf {
    #[path = "cache_abf_tests.rs"]
    mod cache_abf_tests;
}
#[path = "."]
mod convert {
    #[path = "convert_tests.rs"]
    mod convert_tests;
}
#[path = "."]
mod desktop_open {
    #[path = "desktop_open_tests.rs"]
    mod tests;
}
#[path = "."]
mod diff {
    #[path = "diff_tests.rs"]
    mod diff_tests;
}
#[path = "."]
mod pbix_new {
    #[path = "pbix_new_tests.rs"]
    mod tests;
}
#[path = "."]
mod recovery {
    #[path = "recovery_tests.rs"]
    mod tests;
}
#[path = "."]
mod readers {
    #[path = "readers_tests.rs"]
    mod tests;
}
#[path = "."]
mod redact {
    #[path = "redact_tests.rs"]
    mod tests;
}
