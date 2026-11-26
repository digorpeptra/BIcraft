//! `cargo xtask assets`: every asset the project includes has a row in `ATTRIBUTION.md`
//! (AGENTS.md §2).
//!
//! The files checked: everything under `assets/`, `crates/render/geo/` (the bundled map data) and
//! `docs/images/` (the README's screenshots), the fonts the app and the renderer bundle
//! (`crates/ui/fonts/`, `crates/render/fonts/`), and the UI's icons, which are drawn in code
//! (`crates/ui/src/icons.rs`, and the drawings of BIcraft's visuals, `crates/visuals/src/icons.rs`).
//! A file is covered when a table row of `ATTRIBUTION.md` names it
//! in backticks, in one of these forms:
//!
//! - its path from the repository root (`assets/logo/bicraft-icon.svg`);
//! - a folder holding it (`crates/render/fonts/`), or a glob (`crates/render/tests/golden/*.png`,
//!   `dir/**`);
//! - its bare file name (`bicraft-icon-mono.svg`) when the same row names its folder through
//!   another path (`assets/logo/bicraft-icon.svg`).
//!
//! A folder or glob covers a checked file only when it is the checked folder holding it or lies
//! inside it: an upstream folder a row cites as a source (`docs/`) is not a row for
//! `docs/images/`.
//!
//! It fails listing the files without a row, and the rows naming a checked file that is gone.

use std::path::Path;

/// Folders whose every file is an asset (the renderer's bundled map data and the README's images
/// included).
const ASSET_DIRS: &[&str] = &["assets", "crates/render/geo", "docs/images"];
/// Folders whose font files are bundled.
const FONT_DIRS: &[&str] = &["crates/ui/fonts", "crates/render/fonts"];
const FONT_EXTS: &[&str] = &["ttf", "otf", "ttc", "woff", "woff2"];
/// Single files that bundle drawn assets (the UI's icons, drawn in code).
const FILES: &[&str] = &["crates/ui/src/icons.rs", "crates/visuals/src/icons.rs"];

pub fn cmd() -> Result<(), String> {
    let root = crate::root();
    let attribution = std::fs::read_to_string(root.join("ATTRIBUTION.md"))
        .map_err(|e| format!("assets: cannot read ATTRIBUTION.md: {e}"))?;
    let files = checked_files(&root)?;
    let problems = check(&attribution, &files);
    if problems.is_empty() {
        println!(
            "assets: {} files under assets/ and docs/images/, the map data, the bundled fonts and the UI's icons, each with an ATTRIBUTION.md row",
            files.len()
        );
        return Ok(());
    }
    for p in &problems {
        eprintln!("  - {p}");
    }
    Err(format!(
        "assets: {} problem(s) with ATTRIBUTION.md (add a row naming each file: author, source, licence, use)",
        problems.len()
    ))
}

/// The repository-relative paths (with `/`) of the files to check, sorted.
fn checked_files(root: &Path) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for d in ASSET_DIRS {
        walk(root, &root.join(d), &mut out, &|_| true)?;
    }
    for d in FONT_DIRS {
        walk(root, &root.join(d), &mut out, &|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| FONT_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        })?;
    }
    for f in FILES {
        if root.join(f).is_file() {
            out.push((*f).to_string());
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>, keep: &dyn Fn(&Path) -> bool) -> Result<(), String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(()); // a folder that does not exist has nothing to list
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        let ty = e
            .file_type()
            .map_err(|err| format!("assets: {}: {err}", path.display()))?;
        if ty.is_dir() {
            walk(root, &path, out, keep)?;
        } else if keep(&path) {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let rel: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            out.push(rel.join("/"));
        }
    }
    Ok(())
}

/// The backticked names of each table row of `ATTRIBUTION.md`.
fn rows(attribution: &str) -> Vec<Vec<String>> {
    attribution
        .lines()
        .filter(|l| l.trim_start().starts_with('|'))
        .map(|l| {
            l.split('`')
                .skip(1)
                .step_by(2)
                .map(|t| t.trim().trim_start_matches("./").to_string())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .collect()
}

/// What is wrong: checked files no row covers, and rows naming a checked file that is gone.
pub(crate) fn check(attribution: &str, files: &[String]) -> Vec<String> {
    let rows = rows(attribution);
    let mut problems = Vec::new();
    for f in files {
        if !rows.iter().any(|r| covers(r, f)) {
            problems.push(format!("{f}: no row in ATTRIBUTION.md"));
        }
    }
    let scanned = |p: &str| {
        ASSET_DIRS
            .iter()
            .chain(FONT_DIRS)
            .any(|d| p.starts_with(&format!("{d}/")))
            || FILES.contains(&p)
    };
    for r in &rows {
        for t in r {
            let named = !t.contains('*') && !t.ends_with('/') && t.contains('/');
            if named && scanned(t) && !files.iter().any(|f| f == t) {
                problems.push(format!(
                    "{t}: named in ATTRIBUTION.md but not in the tree (or not an asset)"
                ));
            }
        }
    }
    problems
}

/// The checked folder holding `file` (`docs/images` for a README image), if any.
fn checked_root(file: &str) -> Option<&'static str> {
    ASSET_DIRS
        .iter()
        .chain(FONT_DIRS)
        .copied()
        .filter(|d| file.starts_with(&format!("{d}/")))
        .max_by_key(|d| d.len())
}

/// Whether folder `d` is the checked folder `root` or lies inside it: a wider folder (`docs/`
/// for `docs/images/`, such as an upstream source folder cited in a row) covers nothing there.
fn within(d: &str, root: Option<&str>) -> bool {
    root.is_none_or(|r| d == r || d.starts_with(&format!("{r}/")))
}

/// The folder before a glob's first wildcard segment (`assets/img` of `assets/img/*.png`).
fn glob_folder(pattern: &str) -> String {
    pattern
        .split('/')
        .take_while(|s| !s.contains('*'))
        .collect::<Vec<_>>()
        .join("/")
}

/// Whether one row's names cover `file`.
fn covers(row: &[String], file: &str) -> bool {
    let root = checked_root(file);
    let (dir, name) = file.rsplit_once('/').unwrap_or(("", file));
    let row_dirs: Vec<&str> = row
        .iter()
        .filter(|t| t.contains('/') && !t.contains('*'))
        .map(|t| {
            if let Some(d) = t.strip_suffix('/') {
                d
            } else {
                t.rsplit_once('/').map_or("", |(d, _)| d)
            }
        })
        .collect();
    row.iter().any(|t| {
        if t.contains('*') {
            glob(t, file) && within(&glob_folder(t), root)
        } else if let Some(d) = t.strip_suffix('/') {
            file.starts_with(&format!("{d}/")) && within(d, root)
        } else if t.contains('/') {
            t == file
        } else {
            t == name && row_dirs.contains(&dir)
        }
    })
}

/// `*` matches within one path segment, `**` any number of segments.
fn glob(pattern: &str, path: &str) -> bool {
    let p: Vec<&str> = pattern.split('/').collect();
    let s: Vec<&str> = path.split('/').collect();
    glob_segments(&p, &s)
}

fn glob_segments(p: &[&str], s: &[&str]) -> bool {
    match p.split_first() {
        None => s.is_empty(),
        Some((&"**", rest)) => (0..=s.len()).any(|i| glob_segments(rest, s.get(i..).unwrap_or(&[]))),
        Some((first, rest)) => match s.split_first() {
            Some((seg, srest)) => segment(first, seg) && glob_segments(rest, srest),
            None => false,
        },
    }
}

/// One segment against a pattern with `*` wildcards.
fn segment(p: &str, s: &str) -> bool {
    let parts: Vec<&str> = p.split('*').collect();
    if parts.len() == 1 {
        return p == s;
    }
    let (first, last) = (
        parts.first().copied().unwrap_or(""),
        parts.last().copied().unwrap_or(""),
    );
    if !s.starts_with(first) || s.len() < first.len() + last.len() || !s.ends_with(last) {
        return false;
    }
    let mut rest = s.get(first.len()..s.len() - last.len()).unwrap_or("");
    for mid in parts.iter().skip(1).take(parts.len().saturating_sub(2)) {
        match rest.find(mid) {
            Some(i) => rest = rest.get(i + mid.len()..).unwrap_or(""),
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    const DOC: &str = "\
# Attribution

| Asset | Author |
|---|---|
| `assets/logo/a.svg`, `b.svg` | us |
| `crates/render/fonts/` (Selawik) | Microsoft |
| `assets/img/*.png` | us |
| `assets/deep/**` | us |
| `crates/ui/fonts/Sym-subset.ttf` | Noto |

Prose mentioning `assets/prose.svg` is not a row.
";

    #[test]
    fn every_form_of_name_covers_its_files() {
        let f = files(&[
            "assets/logo/a.svg",
            "assets/logo/b.svg",
            "assets/img/x.png",
            "assets/deep/one/two.bin",
            "crates/render/fonts/selawk.ttf",
            "crates/ui/fonts/Sym-subset.ttf",
        ]);
        assert_eq!(check(DOC, &f), Vec::<String>::new());
    }

    #[test]
    fn a_file_without_a_row_is_listed() {
        let f = files(&[
            "assets/logo/a.svg",
            "assets/logo/c.svg",    // bare names only cover their row's folder
            "assets/img/x.jpg",     // the glob is *.png
            "assets/img/sub/y.png", // `*` stays in one segment
            "assets/prose.svg",     // named outside a table row
            "assets/other/b.svg",   // `b.svg` belongs to assets/logo
            "crates/ui/fonts/New.ttf",
        ]);
        let p = check(DOC, &f);
        for missing in [
            "assets/logo/c.svg",
            "assets/img/x.jpg",
            "assets/img/sub/y.png",
            "assets/prose.svg",
            "assets/other/b.svg",
            "crates/ui/fonts/New.ttf",
        ] {
            assert!(
                p.contains(&format!("{missing}: no row in ATTRIBUTION.md")),
                "{missing} not reported: {p:?}"
            );
        }
        assert!(
            !p.iter().any(|m| m.starts_with("assets/logo/a.svg: no row")),
            "{p:?}"
        );
    }

    #[test]
    fn a_row_naming_a_missing_asset_is_listed() {
        let p = check(DOC, &files(&["crates/ui/fonts/Sym-subset.ttf"]));
        assert!(
            p.iter()
                .any(|m| m.starts_with("assets/logo/a.svg: named in ATTRIBUTION.md")),
            "{p:?}"
        );
        assert!(
            !p.iter().any(|m| m.starts_with("crates/ui/fonts/Sym-subset.ttf")),
            "{p:?}"
        );
    }

    #[test]
    fn globs() {
        assert!(glob("a/*.png", "a/x.png"));
        assert!(!glob("a/*.png", "a/b/x.png"));
        assert!(glob("a/**", "a/b/c/d"));
        assert!(glob("a/**/z.txt", "a/z.txt"));
        assert!(glob("a/**/z.txt", "a/b/c/z.txt"));
        assert!(glob("a/x*y*z", "a/x1y2z"));
        assert!(!glob("a/x*y*z", "a/xz"));
    }

    #[test]
    fn a_folder_wider_than_a_checked_folder_covers_nothing_in_it() {
        // a source cell citing an upstream `docs/` folder is not a row for the README's images
        let doc = "| `x.ksy` | them | [repo](https://example.org) `docs/` | CC0 |\n| `assets/**` | us |\n";
        let p = check(doc, &files(&["docs/images/shot.png", "assets/logo/a.svg"]));
        assert_eq!(
            p,
            vec!["docs/images/shot.png: no row in ATTRIBUTION.md".to_string()]
        );
        // the checked folder itself, or one inside it, still covers
        for row in ["`docs/images/`", "`docs/images/*.png`", "`docs/images/**`"] {
            let doc = format!("| {row} | us |\n");
            assert_eq!(
                check(&doc, &files(&["docs/images/shot.png"])),
                Vec::<String>::new(),
                "{row}"
            );
        }
    }

    /// A README image added without its row fails the check on the repository's own file.
    #[test]
    fn a_stray_readme_image_without_a_row_fails() {
        let root = crate::root();
        let doc = std::fs::read_to_string(root.join("ATTRIBUTION.md")).unwrap();
        let mut files = checked_files(&root).unwrap();
        assert!(files.iter().any(|f| f.starts_with("docs/images/")), "{files:?}");
        files.push("docs/images/stray.png".into());
        let p = check(&doc, &files);
        assert_eq!(
            p,
            vec!["docs/images/stray.png: no row in ATTRIBUTION.md".to_string()]
        );
    }

    /// The repository itself passes (what `cargo xtask ci` checks).
    #[test]
    fn the_repository_has_a_row_for_every_asset() {
        let root = crate::root();
        let doc = std::fs::read_to_string(root.join("ATTRIBUTION.md")).unwrap();
        let files = checked_files(&root).unwrap();
        assert!(files.iter().any(|f| f.starts_with("assets/logo/")), "{files:?}");
        assert!(files.iter().any(|f| f.ends_with(".ttf")), "{files:?}");
        assert_eq!(check(&doc, &files), Vec::<String>::new());
    }
}
