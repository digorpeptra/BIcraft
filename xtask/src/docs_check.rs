//! `cargo xtask docs`: checks on the committed Markdown, part of `cargo xtask ci`.
//!
//! **No Power BI Desktop performance figure.** The Desktop licence forbids disclosing its
//! benchmark results (docs/interoperability.md), so a committed doc must not put a timing, a
//! speed-up or a "faster/slower than" next to the word "Desktop" or a name of Desktop's engine
//! (`VENDOR`: "Analysis Services", a table row headed `Engine:`, the engine's `ImageSave` and
//! `ImageLoad`). A line fails when it, or the line before or after it, names one of them and
//! the line itself holds a duration
//! (`12 s`, `16 ms`, `3 min`), a ratio (`×19`, `19×`, `19x faster`) or a speed comparison
//! (`faster`, `slower`, `speed-up`). Agreement with Desktop (same values, Desktop opens the
//! file) never matches. A false positive is allowed by putting `<!-- perf-guard: ok -->` on
//! the line, with a reason next to it.
//!
//! **Parity summaries.** `docs/parity-checklist.md` and the headline of
//! `docs/target-app-parity.md` match their rows (`cargo xtask parity --check`).
//!
//! **Command placement.** `docs/commands-ui.md` matches the `COMMANDS` table
//! (`cargo xtask commands-ui --check`).
//!
//! **Licence copies.** Each file under `licenses/` (shipped in the container image) equals the
//! licence file next to the vendored material it covers (`dist::BUNDLED_LICENSES`).

use std::path::{Path, PathBuf};

/// The marker that allows one line.
const ALLOW: &str = "perf-guard: ok";

/// Words naming Power BI Desktop or its engine: a performance figure near one of them counts
/// as one of Desktop's figures.
const VENDOR: &[&str] = &[
    "Desktop",
    "Analysis Services",
    "| Engine: ",
    "ImageSave",
    "ImageLoad",
];

/// Directories never scanned (build output, vendored or generated material, and `.claude`,
/// where local agent worktrees, other checkouts, live).
const SKIP_DIRS: &[&str] = &["target", ".git", ".claude", "node_modules", "fuzz", ".pbi"];

pub fn cmd() -> Result<(), String> {
    let root = crate::root();
    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();
    let mut problems = licence_copies(&root);
    problems.extend(crate::parity::check(&root));
    problems.extend(crate::commands_ui::check(&root));
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else {
            continue;
        };
        let rel = f
            .strip_prefix(&root)
            .unwrap_or(f)
            .to_string_lossy()
            .replace('\\', "/");
        for (line, why) in desktop_perf_lines(&text) {
            problems.push(format!("{rel}:{line}: {why}"));
        }
    }
    if problems.is_empty() {
        eprintln!(
            "docs: {} Markdown files, no Desktop performance figure; licenses/ and parity summaries up to date",
            files.len()
        );
        return Ok(());
    }
    for p in &problems {
        eprintln!("{p}");
    }
    Err(format!(
        "{} problem(s): a timing, ratio or speed comparison next to Power BI Desktop \
         (docs/interoperability.md; remove it, or mark a false positive with `<!-- {ALLOW} -->`), \
         or a stale copy under licenses/, parity summary (`cargo xtask parity`) or command \
         placement table (`cargo xtask commands-ui`)",
        problems.len()
    ))
}

/// Differences between `licenses/` and the licence files it copies.
fn licence_copies(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for (src, dest) in crate::dist::BUNDLED_LICENSES {
        let a = std::fs::read(root.join(src));
        let b = std::fs::read(root.join(dest));
        match (a, b) {
            (Ok(a), Ok(b)) if a == b => {}
            (Ok(_), Ok(_)) => out.push(format!("{dest}: differs from {src} (copy it again)")),
            (Err(e), _) => out.push(format!("{src}: {e}")),
            (_, Err(e)) => out.push(format!("{dest}: {e} (copy {src} there)")),
        }
    }
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with("target") {
                collect(&p, out);
            }
        } else if name.ends_with(".md") {
            out.push(p);
        }
    }
}

/// 1-based numbers of the offending lines of `text`, with what matched.
pub fn desktop_perf_lines(text: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.contains(ALLOW) {
            continue;
        }
        let prev = if i > 0 {
            lines.get(i - 1).copied().unwrap_or("")
        } else {
            ""
        };
        let next = lines.get(i + 1).copied().unwrap_or("");
        let names_vendor = |l: &str| VENDOR.iter().any(|w| l.contains(w));
        if !(names_vendor(line) || names_vendor(prev) || names_vendor(next)) {
            continue;
        }
        // Positions in characters within `prev \n line \n next`.
        let context: Vec<char> = format!("{prev}\n{line}\n{next}").chars().collect();
        let offset = prev.chars().count() + 1;
        let desktop: Vec<usize> = VENDOR.iter().flat_map(|w| find_all(&context, w)).collect();
        let chars: Vec<char> = line.chars().collect();
        let hit = perf_figures(&chars).into_iter().find(|(pos, _)| {
            let at = offset + pos;
            desktop.iter().any(|d| d.abs_diff(at) <= NEAR)
        });
        if let Some((_, why)) = hit {
            out.push((i + 1, why));
        }
    }
    out
}

/// How close (in characters) a figure must be to "Desktop" to count as one of its figures.
const NEAR: usize = 80;

/// Start positions of `needle` in `hay`.
fn find_all(hay: &[char], needle: &str) -> Vec<usize> {
    let n: Vec<char> = needle.chars().collect();
    (0..hay.len())
        .filter(|&i| hay.get(i..i + n.len()).is_some_and(|w| w == n.as_slice()))
        .collect()
}

/// Every performance figure of a line, with its position: durations, ratios, speed words.
fn perf_figures(chars: &[char]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let lower: Vec<char> = chars.iter().flat_map(|c| c.to_lowercase()).collect();
    for w in ["faster", "slower", "speed-up", "speedup", "times as fast"] {
        for p in find_all(&lower, w) {
            out.push((p, format!("speed comparison `{w}`")));
        }
    }
    for (i, c) in chars.iter().enumerate() {
        if *c == '×' {
            let digit_after = chars.get(i + 1).is_some_and(char::is_ascii_digit);
            let digit_before = i > 0 && chars.get(i - 1).is_some_and(char::is_ascii_digit);
            // `20×20` (a size) has digits on both sides; a ratio has one.
            if digit_after != digit_before {
                out.push((i, "ratio `×`".into()));
            }
        }
    }
    out.extend(
        durations(chars)
            .into_iter()
            .map(|(p, d)| (p, format!("duration `{d}`"))),
    );
    out
}

/// Every `<number> <unit>` duration in `chars` (`12 s`, `16ms`, `1.5 min`), with its position.
fn durations(chars: &[char]) -> Vec<(usize, String)> {
    const UNITS: &[&str] = &["ms", "µs", "s", "sec", "seconds", "min", "minutes", "h"];
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let starts_number = chars.get(i).is_some_and(char::is_ascii_digit)
            && (i == 0 || chars.get(i - 1).is_some_and(|c| !c.is_alphanumeric()));
        if !starts_number {
            i += 1;
            continue;
        }
        let mut j = i;
        while chars
            .get(j)
            .is_some_and(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | ' ' | '\u{202f}'))
        {
            // a space belongs to the number only between digit groups (`1 000`)
            if chars.get(j) == Some(&' ')
                && !(chars.get(j + 1).is_some_and(char::is_ascii_digit)
                    && chars.get(j + 2).is_some_and(char::is_ascii_digit)
                    && chars.get(j + 3).is_some_and(char::is_ascii_digit))
            {
                break;
            }
            j += 1;
        }
        let mut k = j;
        if chars.get(k) == Some(&' ') {
            k += 1;
        }
        let mut end = k;
        while chars.get(end).is_some_and(|c| c.is_alphabetic()) {
            end += 1;
        }
        let unit: String = chars.get(k..end).unwrap_or_default().iter().collect();
        if UNITS.contains(&unit.as_str()) {
            let shown: String = chars.get(i..end).unwrap_or_default().iter().collect();
            out.push((i, shown));
        }
        i = j.max(i + 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bad(text: &str) -> Vec<usize> {
        desktop_perf_lines(text).into_iter().map(|(l, _)| l).collect()
    }

    #[test]
    fn timings_and_ratios_next_to_desktop_fail() {
        assert_eq!(bad("BIcraft opens it in 16 ms where Desktop needs 12 s."), [1]);
        assert_eq!(bad("Q05 is ×19 against Desktop"), [1]);
        assert_eq!(bad("Desktop:\n| Q05 | 19x faster |"), [2]);
        assert_eq!(bad("| Desktop | 1 000 ms |"), [1]);
        assert_eq!(bad("much slower than Desktop"), [1]);
        assert_eq!(bad("| Engine: process (refresh) | 17 s |"), [1]);
        assert_eq!(bad("| `ImageSave` / `ImageLoad` | 5 s / 3 s |"), [1]);
        assert_eq!(bad("Analysis Services processed it in 211 s"), [1]);
    }

    #[test]
    fn agreement_sizes_and_other_lines_pass() {
        assert!(bad("Desktop opens the file; values agree on 1141 of 1142 queries.").is_empty());
        assert!(bad("A 20×20 icon written by Desktop.").is_empty());
        assert!(bad("BIcraft answers Q05 in 16 ms.").is_empty());
        assert!(bad("Desktop 2.158 writes version 1.28").is_empty());
        assert!(bad("Desktop writes 5 sessions").is_empty());
        let far = format!(
            "Desktop opens the file. {} BIcraft's open went 60 → 27 ms.",
            "x".repeat(100)
        );
        assert!(bad(&far).is_empty());
        assert!(bad("Desktop takes 12 s <!-- perf-guard: ok: a timeout setting -->").is_empty());
    }
}
