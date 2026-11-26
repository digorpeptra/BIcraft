//! The nightly test path (`cargo xtask heavy`, `.github/workflows/nightly.yml`; docs/ci.md):
//!
//! - [`HEAVY`]: tests left out of `cargo xtask ci` because they are big-volume variants of a
//!   behaviour a gate test already covers at a smaller size;
//! - [`LONG_RUNS`]: gate tests whose loops (fuzz inputs, random cases) take a size from the
//!   environment, run again with many more iterations.
//!
//! docs/ci.md lists both; a unit test below checks that the lists and the doc agree. A test goes
//! into [`HEAVY`] only when the gate keeps failing on any regression it would catch.

/// One heavy test: its package and its full libtest name (module path included).
#[derive(Debug, Clone, Copy)]
pub struct Heavy {
    pub package: &'static str,
    pub test: &'static str,
    /// Why it is heavy.
    pub why: &'static str,
    /// The gate test(s) that cover the same behaviour at a smaller size.
    pub covered_by: &'static str,
}

/// Tests the gate leaves out (run by `cargo xtask heavy`).
pub const HEAVY: &[Heavy] = &[Heavy {
    package: "bicraft-ui",
    test: "tests::speed_ui_tests::a_million_row_report_stays_responsive",
    why: "the UI speed scenarios over a 1,000,000-row report (docs/ui-perf.md)",
    covered_by: "tests::speed_ui_tests::a_large_report_stays_responsive (100,000 rows)",
}];

/// One longer run of gate tests: the tests of `package` matching the nextest `filter`, with
/// `env` (the loop sizes, about ten times the gate's).
#[derive(Debug, Clone, Copy)]
pub struct LongRun {
    pub package: &'static str,
    pub filter: &'static str,
    pub env: &'static [(&'static str, &'static str)],
}

/// Longer runs of the gate's loops. `BICRAFT_FUZZ_SMOKE_ITERS` sets the mutated inputs of every
/// target of a `fuzz_smoke` binary (each target's gate count is in its `smoke::run` call); the
/// value is about ten times the largest gate count of the binary.
pub const LONG_RUNS: &[LongRun] = &[
    LongRun {
        package: "bicraft-abf",
        filter: "binary(fuzz_smoke)",
        env: &[("BICRAFT_FUZZ_SMOKE_ITERS", "150000")],
    },
    LongRun {
        package: "bicraft-dax",
        filter: "binary(fuzz_smoke)",
        env: &[("BICRAFT_FUZZ_SMOKE_ITERS", "400000")],
    },
    LongRun {
        package: "bicraft-layout",
        filter: "binary(fuzz_smoke)",
        env: &[("BICRAFT_FUZZ_SMOKE_ITERS", "6000")],
    },
    LongRun {
        package: "bicraft-mcp",
        filter: "binary(fuzz_smoke)",
        env: &[("BICRAFT_FUZZ_SMOKE_ITERS", "20000")],
    },
    LongRun {
        package: "bicraft-metadata",
        filter: "binary(fuzz_smoke)",
        env: &[("BICRAFT_FUZZ_SMOKE_ITERS", "60000")],
    },
    LongRun {
        package: "bicraft-opc",
        filter: "binary(fuzz_smoke)",
        env: &[("BICRAFT_FUZZ_SMOKE_ITERS", "500000")],
    },
    LongRun {
        package: "bicraft-pbir",
        filter: "binary(fuzz_smoke)",
        env: &[("BICRAFT_FUZZ_SMOKE_ITERS", "6000")],
    },
    LongRun {
        package: "bicraft-pbix",
        filter: "binary(fuzz_smoke)",
        env: &[("BICRAFT_FUZZ_SMOKE_ITERS", "30000")],
    },
    LongRun {
        package: "bicraft-sqlite",
        filter: "binary(fuzz_smoke)",
        env: &[("BICRAFT_FUZZ_SMOKE_ITERS", "150000")],
    },
    LongRun {
        package: "bicraft-tmdl",
        filter: "binary(fuzz_smoke)",
        env: &[("BICRAFT_FUZZ_SMOKE_ITERS", "100000")],
    },
    LongRun {
        package: "bicraft-tmsl",
        filter: "binary(fuzz_smoke)",
        env: &[("BICRAFT_FUZZ_SMOKE_ITERS", "200000")],
    },
    LongRun {
        package: "bicraft-vertipaq",
        filter: "binary(fuzz_smoke)",
        env: &[("BICRAFT_FUZZ_SMOKE_ITERS", "500000")],
    },
    LongRun {
        package: "bicraft-xpress9",
        filter: "binary(fuzz_smoke)",
        env: &[("BICRAFT_FUZZ_SMOKE_ITERS", "100000")],
    },
    // The storage engine and aggregations against the reference evaluator on random models
    // (gate: 240 and 120 cases).
    LongRun {
        package: "bicraft-engine",
        filter: "test(=preview::general::storage_fuzz::storage_fuzz_matches_the_reference_evaluator) \
                 | test(=preview::general::aggs_tests::aggregations_fuzz_matches_the_fact)",
        env: &[("BICRAFT_FUZZ_CASES", "2400")],
    },
    // The scan kernel against a naive loop on random shapes (gate: 64 cases).
    LongRun {
        package: "bicraft-scan",
        filter: "test(=kernel_tests::kernel_matches_a_naive_loop_on_random_shapes)",
        env: &[("BICRAFT_KERNEL_CASES", "640")],
    },
];

/// nextest filter expression of the heavy tests.
pub fn include_filter() -> String {
    if HEAVY.is_empty() {
        return "none()".to_string();
    }
    HEAVY
        .iter()
        .map(|t| format!("(package({}) & test(={}))", t.package, t.test))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// nextest filter expression of every other test (the gate's).
pub fn exclude_filter() -> String {
    format!("not ({})", include_filter())
}

/// libtest arguments that skip the heavy tests (`cargo test -- ARGS`). `--skip` matches
/// substrings, so each name is the full path of a test, unique in the workspace.
pub fn skip_args() -> Vec<String> {
    HEAVY
        .iter()
        .flat_map(|t| ["--skip".to_string(), t.test.to_string()])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_are_complements() {
        let inc = include_filter();
        assert_eq!(exclude_filter(), format!("not ({inc})"));
        for t in HEAVY {
            assert!(inc.contains(&format!("package({})", t.package)), "{inc}");
            assert!(inc.contains(&format!("test(={})", t.test)), "{inc}");
        }
        assert_eq!(skip_args().len(), 2 * HEAVY.len());
    }

    #[test]
    fn names_are_unique_full_paths() {
        let mut seen = std::collections::BTreeSet::new();
        for t in HEAVY {
            assert!(seen.insert(t.test), "{} listed twice", t.test);
            assert!(!t.test.contains(' ') && !t.package.is_empty(), "{t:?}");
            assert!(!t.why.is_empty() && !t.covered_by.is_empty(), "{t:?}");
        }
    }

    #[test]
    fn long_runs_name_workspace_packages() {
        let root = crate::root();
        for r in LONG_RUNS {
            assert!(r.package.starts_with("bicraft-"), "{r:?}");
            assert!(!r.env.is_empty() && !r.filter.is_empty(), "{r:?}");
            let dir = r.package.trim_start_matches("bicraft-");
            let dir = if dir == "layout" { "layout-bb" } else { dir };
            assert!(root.join("crates").join(dir).is_dir(), "{r:?}");
        }
    }

    /// docs/ci.md names every heavy test and every long run's package and variable.
    #[test]
    fn the_doc_lists_the_nightly_path() {
        let doc = std::fs::read_to_string(crate::root().join("docs/ci.md")).unwrap();
        for t in HEAVY {
            assert!(
                doc.contains(&format!("`{}`", t.test)),
                "docs/ci.md does not list `{}`",
                t.test
            );
        }
        for r in LONG_RUNS {
            for (k, v) in r.env {
                assert!(
                    doc.contains(&format!("`{}`", r.package)) && doc.contains(&format!("{k}={v}")),
                    "docs/ci.md does not list the long run of `{}` ({k}={v})",
                    r.package
                );
            }
        }
    }
}
