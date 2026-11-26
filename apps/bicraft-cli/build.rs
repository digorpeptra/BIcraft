//! Embeds the git commit the binary was built from, shown by `bicraft --version`.
//!
//! The commit is read from the `.git` directory directly (no `git` process: inside WSL a checkout
//! on `/mnt/c` is often refused by git as "dubious ownership"). `BICRAFT_GIT_COMMIT` overrides it,
//! for builds from a source archive without `.git`. Without either, no commit is embedded.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=BICRAFT_GIT_COMMIT");
    let from_env = bicraft_env::var("BICRAFT_GIT_COMMIT")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let commit = from_env.or_else(|| {
        let manifest = std::env::var_os("CARGO_MANIFEST_DIR")?;
        git_commit(Path::new(&manifest))
    });
    if let Some(c) = commit {
        println!("cargo:rustc-env=BICRAFT_GIT_COMMIT={c}");
    }
}

/// Short hash of `HEAD` of the repository containing `start`, if any.
fn git_commit(start: &Path) -> Option<String> {
    let git_dir = find_git_dir(start)?;
    // A linked worktree keeps its refs in the main repository (`commondir`).
    let common = match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(rel) => git_dir.join(rel.trim()),
        Err(_) => git_dir.clone(),
    };
    let head_path = git_dir.join("HEAD");
    watch(&head_path);
    let head = std::fs::read_to_string(&head_path).ok()?;
    let head = head.trim();
    let hash = match head.strip_prefix("ref:") {
        None => head.to_owned(),
        Some(name) => {
            let name = name.trim();
            let packed = common.join("packed-refs");
            watch(&packed);
            let mut found = None;
            for dir in [&git_dir, &common] {
                let loose = dir.join(name);
                if let Ok(s) = std::fs::read_to_string(&loose) {
                    watch(&loose);
                    found = Some(s.trim().to_owned());
                    break;
                }
            }
            match found {
                Some(h) => h,
                None => {
                    // A branch committed to later becomes a loose ref: watch the folder.
                    watch(&common.join("refs").join("heads"));
                    let text = std::fs::read_to_string(&packed).ok()?;
                    text.lines()
                        .filter_map(|l| l.split_once(' '))
                        .find(|(_, r)| r.trim() == name)
                        .map(|(h, _)| h.to_owned())?
                }
            }
        }
    };
    let ok = matches!(hash.len(), 40 | 64) && hash.bytes().all(|b| b.is_ascii_hexdigit());
    ok.then(|| hash.get(..12).unwrap_or(&hash).to_ascii_lowercase())
}

/// The `.git` directory of the nearest enclosing repository (a `.git` file points to it).
fn find_git_dir(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let dot = dir.join(".git");
        if dot.is_dir() {
            return Some(dot);
        }
        if dot.is_file() {
            let text = std::fs::read_to_string(&dot).ok()?;
            let rel = text.trim().strip_prefix("gitdir:")?.trim();
            return Some(dir.join(rel));
        }
    }
    None
}

fn watch(path: &Path) {
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}
