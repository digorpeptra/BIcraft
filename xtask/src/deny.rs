//! `cargo xtask deny [ARGS]`: `cargo deny check` over `deny.toml` (licence allowlist,
//! crates.io-only sources, RustSec advisories, duplicate majors as warnings) when cargo-deny is
//! installed; without it, a note and success, so a fresh checkout's commands still work. The
//! `deny` job of `.github/workflows/ci.yml` runs the same check on every push.
//!
//! ARGS go to `cargo deny check` (`licenses`, `bans`, `sources`, `advisories`, `--offline`…).

use crate::{cargo, run};

/// The cargo-deny version CI installs (`.github/workflows/ci.yml`).
pub const CARGO_DENY_VERSION: &str = "0.20.2";

pub fn cmd(args: &[String]) -> Result<(), String> {
    if !installed() {
        eprintln!(
            "note: cargo-deny is not installed (cargo install cargo-deny --version {CARGO_DENY_VERSION} --locked); \
             skipping the licence, source and advisory checks (CI runs them)"
        );
        return Ok(());
    }
    let mut c = cargo();
    c.args(["deny", "--all-features", "check"]);
    c.args(args);
    run(c, &format!("cargo deny --all-features check {}", args.join(" ")))
}

fn installed() -> bool {
    cargo()
        .args(["deny", "--version"])
        .output()
        .is_ok_and(|o| o.status.success())
}
