//! The engine's command-table tests that need only its public API: the table itself, the
//! command placement in the UI (`command_ui`) and the server-mode roles. An integration test
//! links the library already built instead of compiling it again under `cfg(test)` (docs/ci.md,
//! "Where engine tests go"). Each file keeps the module path it had inside the crate, so the
//! test names do not change.
//! `#[path = "."]` on an inline module keeps its files next to `main.rs` (a path through the
//! module's missing directory, `module/../file.rs`, does not resolve on Linux).

#[path = "../../../../testsupport/tempdir.rs"]
mod tempdir;

mod roles_tests;
mod tests;

#[path = "."]
mod command_ui {
    #[path = "command_ui_tests.rs"]
    mod tests;
}
