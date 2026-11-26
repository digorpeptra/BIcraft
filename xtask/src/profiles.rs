//! `profiles` step of `cargo xtask ci`: the build profiles the tests run with keep debug
//! assertions and overflow checks on.
//!
//! The never-crash rules (AGENTS.md §3) rely on the tests panicking on an arithmetic overflow
//! and on `debug_assert!`. The dev and test profiles are tuned for speed (docs/ci.md, "Build
//! profiles"): optimisation levels and debug info may change, those two checks may not. This
//! step reads the root `Cargo.toml` and fails when a profile the tests use (`dev`, `test` and
//! their package overrides) turns either off.

/// Profiles whose code the tests run: `cargo test`, `cargo nextest` and `cargo xtask heavy`.
const TEST_PROFILES: &[&str] = &["dev", "test"];

/// Settings that must stay on.
const KEPT_ON: &[&str] = &["overflow-checks", "debug-assertions"];

pub fn check() -> Result<(), String> {
    let path = crate::root().join("Cargo.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let problems = problems(&text);
    if problems.is_empty() {
        eprintln!("profiles: dev and test keep overflow checks and debug assertions");
        return Ok(());
    }
    for p in &problems {
        eprintln!("Cargo.toml:{p}");
    }
    Err(format!(
        "{} profile setting(s) turn off overflow checks or debug assertions for the tests \
         (AGENTS.md section 3, docs/ci.md)",
        problems.len()
    ))
}

/// `line: why` for each offending setting.
fn problems(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut profile: Option<String> = None;
    for (i, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            profile = line
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim()
                .strip_prefix("profile.")
                .map(|rest| rest.split('.').next().unwrap_or("").trim_matches('"').to_string());
            continue;
        }
        let Some(p) = profile.as_deref() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if KEPT_ON.contains(&key) && value != "true" && TEST_PROFILES.contains(&p) {
            out.push(format!("{}: `{key} = {value}` in profile `{p}`", i + 1));
        }
        // A profile the tests use must not inherit from one that may turn the checks off.
        if key == "inherits" && TEST_PROFILES.contains(&p) {
            out.push(format!(
                "{}: profile `{p}` must not inherit another profile",
                i + 1
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_workspace_profiles_pass() {
        let text = std::fs::read_to_string(crate::root().join("Cargo.toml")).unwrap();
        assert_eq!(problems(&text), Vec::<String>::new());
    }

    #[test]
    fn turning_a_check_off_fails() {
        let text = "[profile.dev]\nopt-level = 1\n\n[profile.dev.package.\"bicraft-dax\"]\n\
                    overflow-checks = false # fast\n[profile.release]\noverflow-checks = false\n\
                    [profile.test]\ndebug-assertions=false\n";
        let p = problems(text);
        assert_eq!(p.len(), 2, "{p:?}");
        assert!(p[0].starts_with("5:") && p[0].contains("`dev`"), "{p:?}");
        assert!(p[1].starts_with("9:") && p[1].contains("`test`"), "{p:?}");
    }

    #[test]
    fn other_sections_are_ignored() {
        assert!(problems("[package]\noverflow-checks = false\n").is_empty());
        assert!(problems("[profile.dev]\n# overflow-checks = false\n").is_empty());
    }

    /// The test binaries themselves: an overflow panics and debug assertions are compiled in.
    #[test]
    fn this_test_build_checks_overflow() {
        assert!(std::hint::black_box(cfg!(debug_assertions)));
        let max = std::hint::black_box(u8::MAX);
        let r = std::panic::catch_unwind(|| max + std::hint::black_box(1));
        assert!(r.is_err(), "u8::MAX + 1 did not panic: overflow checks are off");
    }
}
