//! `bicraft-cli diff` (`docs/diff.md`): the semantic diff (`file.diff`) and its git integrations.
//!
//! ```text
//! bicraft-cli diff [OPTIONS] A B                   # two files of any format
//! bicraft-cli diff [OPTIONS] --git REV1 [REV2] PATH    # a project (or file) between two revisions;
//!                                                  # REV2 omitted: the working tree
//! bicraft-cli diff --textconv FILE                 # git textconv: canonical text of one file
//! bicraft-cli diff PATH OLD HEX MODE NEW HEX MODE  # GIT_EXTERNAL_DIFF / diff.<driver>.command
//! OPTIONS: --json | --markdown | --text (default), --model-only | --report-only,
//!          --ignore-formatting, --ignore-positions, --ignore-whitespace, --exit-code
//! ```
//!
//! `--git` only reads the repository: `git rev-parse`, `git cat-file` and `git ls-tree`, whose
//! output is written to a temporary folder that is removed afterwards. Nothing in the
//! repository (index, worktree, refs, config) is touched; `GIT_OPTIONAL_LOCKS=0` keeps git from
//! taking even optional locks.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bicraft_engine::Session;
use serde_json::{Map, Value, json};

use super::{Failure, OK, USAGE_ERROR, print_json, usage};

/// Exit code of `--exit-code` when the files differ (as `git diff --exit-code`).
const DIFFERENT: u8 = 1;

/// Most files copied out of one revision.
const MAX_FILES: usize = 100_000;
/// Most bytes copied out of one revision.
const MAX_TOTAL_BYTES: u64 = 8 << 30;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Output {
    Text,
    Json,
    Markdown,
}

struct Opts {
    output: Output,
    params: Map<String, Value>,
    exit_code: bool,
}

pub(crate) fn diff(args: &[String]) -> Result<u8, Failure> {
    let mut opts = Opts {
        output: Output::Text,
        params: Map::new(),
        exit_code: false,
    };
    let mut positional: Vec<&str> = Vec::new();
    let mut git = false;
    let mut textconv = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => opts.output = Output::Json,
            "--markdown" => opts.output = Output::Markdown,
            "--text" => opts.output = Output::Text,
            "--model-only" => {
                opts.params.insert("scope".into(), json!("model"));
            }
            "--report-only" => {
                opts.params.insert("scope".into(), json!("report"));
            }
            "--ignore-formatting" | "--ignore-positions" | "--ignore-whitespace" => {
                let key = a.trim_start_matches("--").replace('-', "_");
                opts.params.insert(key, json!(true));
            }
            "--exit-code" => opts.exit_code = true,
            "--git" => git = true,
            "--textconv" => textconv = true,
            "--" => positional.extend(it.by_ref().map(String::as_str)),
            other if other.starts_with("--") => return Err(usage(format!("unknown option `{other}`"))),
            other => positional.push(other),
        }
    }
    if textconv {
        let [file] = positional.as_slice() else {
            return Err(usage("diff --textconv needs one FILE"));
        };
        let v = Session::new()
            .execute("file.textconv", &json!({"file": file}))
            .map_err(|e| (USAGE_ERROR, e))?;
        let text = v.get("text").and_then(Value::as_str).unwrap_or("");
        outln!("{}", text.trim_end_matches('\n'));
        return Ok(OK);
    }
    if git {
        return match positional.as_slice() {
            [rev1, path] => git_diff(rev1, None, path, &opts),
            [rev1, rev2, path] => git_diff(rev1, Some(rev2), path, &opts),
            _ => Err(usage("diff --git needs REV1 [REV2] PATH")),
        };
    }
    match positional.as_slice() {
        [a, b] => run(Some(a), Some(b), None, &opts),
        // git's external diff driver: path old-file old-hex old-mode new-file new-hex new-mode
        [path, old, _, _, new, _, _] => {
            let side = |f: &str| (!is_null_file(f)).then(|| f.to_string());
            let (a, b) = (side(old), side(new));
            run(
                a.as_deref(),
                b.as_deref(),
                Some((format!("a/{path}"), format!("b/{path}"))),
                &opts,
            )
            // git stops the whole `git diff` on a non-zero exit: differences are not an error
            .map(|code| if code == DIFFERENT { OK } else { code })
        }
        _ => Err(usage(
            "diff needs A B, --git REV1 [REV2] PATH, --textconv FILE, or git's 7 external-diff arguments",
        )),
    }
}

/// git passes `/dev/null` (also on Windows) for the missing side of an added or deleted file.
fn is_null_file(f: &str) -> bool {
    f == "/dev/null" || f.eq_ignore_ascii_case("nul")
}

fn run(
    a: Option<&str>,
    b: Option<&str>,
    labels: Option<(String, String)>,
    opts: &Opts,
) -> Result<u8, Failure> {
    let mut params = opts.params.clone();
    params.insert("a".into(), json!(a));
    params.insert("b".into(), json!(b));
    if let Some((la, lb)) = labels {
        params.insert("a_label".into(), json!(la));
        params.insert("b_label".into(), json!(lb));
    }
    let format = match opts.output {
        Output::Text => "text",
        Output::Markdown => "markdown",
        Output::Json => "json",
    };
    params.insert("format".into(), json!(format));
    let v = Session::new()
        .execute("file.diff", &Value::Object(params))
        .map_err(|e| (USAGE_ERROR, e))?;
    match opts.output {
        Output::Json => print_json(&v),
        Output::Text | Output::Markdown => {
            let text = v.get(format).and_then(Value::as_str).unwrap_or("");
            outln!("{}", text.trim_end_matches('\n'));
        }
    }
    let identical = v.get("identical").and_then(Value::as_bool).unwrap_or(true);
    Ok(if opts.exit_code && !identical {
        DIFFERENT
    } else {
        OK
    })
}

// ---- --git ----------------------------------------------------------------------------------

/// A temporary folder removed on drop.
struct TempTree(PathBuf);

impl TempTree {
    fn new() -> Result<TempTree, Failure> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("bicraft-diff-{}-{nanos}", std::process::id()));
        std::fs::create_dir(&dir).map_err(|e| (USAGE_ERROR, format!("{}: {e}", dir.display())))?;
        Ok(TempTree(dir))
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A read-only git command (in `dir`), its stdout.
fn git(dir: Option<&Path>, args: &[&str]) -> Result<Vec<u8>, Failure> {
    let mut cmd = Command::new("git");
    if let Some(d) = dir {
        cmd.arg("-C").arg(d);
    }
    let out = cmd
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| (USAGE_ERROR, format!("cannot run git: {e}")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err((USAGE_ERROR, format!("git {}: {}", args.join(" "), err.trim())));
    }
    Ok(out.stdout)
}

fn git_line(dir: Option<&Path>, args: &[&str]) -> Result<String, Failure> {
    let out = git(dir, args)?;
    Ok(String::from_utf8_lossy(&out)
        .trim_end_matches(['\n', '\r'])
        .to_string())
}

/// `PATH` (relative to the current folder, or absolute inside the repository) as a path from
/// the repository root, `/`-separated, without `.` or `..`.
fn repo_path(top: &str, prefix: &str, path: &str) -> Result<String, Failure> {
    let p = path.replace('\\', "/");
    let top_slash = format!("{}/", top.trim_end_matches('/'));
    let joined = if Path::new(path).is_absolute() || p.starts_with('/') {
        let lower = p.to_lowercase();
        if !lower.starts_with(&top_slash.to_lowercase()) {
            return Err(usage(format!("{path} is not inside the repository {top}")));
        }
        p.get(top_slash.len()..).unwrap_or("").to_string()
    } else {
        format!("{prefix}{p}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for c in joined.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(usage(format!("{path} leaves the repository")));
                }
            }
            c => parts.push(c),
        }
    }
    Ok(parts.join("/"))
}

/// A path component git gave that is safe to create here.
fn safe_component(c: &str) -> bool {
    !c.is_empty()
        && c != "."
        && c != ".."
        && !c.chars().any(|ch| ch.is_control() || "<>:\"|?*\\".contains(ch))
        && !c.ends_with(['.', ' '])
}

fn git_diff(rev1: &str, rev2: Option<&str>, path: &str, opts: &Opts) -> Result<u8, Failure> {
    for r in std::iter::once(rev1).chain(rev2) {
        if r.starts_with('-') {
            return Err(usage(format!("`{r}` is not a revision")));
        }
    }
    let top = git_line(None, &["rev-parse", "--show-toplevel"])?;
    let prefix = git_line(None, &["rev-parse", "--show-prefix"])?;
    let full = repo_path(&top, &prefix, path)?;
    if full.is_empty() {
        return Err(usage("diff --git needs a PATH below the repository root"));
    }
    let top = PathBuf::from(&top);
    let tmp = TempTree::new()?;
    let a = materialize(&top, rev1, &full, &tmp.0.join("a"))?;
    let a_label = format!("{rev1}:{full}");
    let (b, b_label) = match rev2 {
        Some(r) => (
            materialize(&top, r, &full, &tmp.0.join("b"))?,
            format!("{r}:{full}"),
        ),
        None => {
            let p = PathBuf::from(path);
            (p.exists().then_some(p), format!("{full} (working tree)"))
        }
    };
    let s = |p: &Option<PathBuf>| p.as_ref().map(|p| p.to_string_lossy().into_owned());
    let (sa, sb) = (s(&a), s(&b));
    run(sa.as_deref(), sb.as_deref(), Some((a_label, b_label)), opts)
}

/// Writes what `full` is at `rev` under `out` (with the project's sibling folders for a .pbip
/// file); `None` when it does not exist at that revision.
fn materialize(top: &Path, rev: &str, full: &str, out: &Path) -> Result<Option<PathBuf>, Failure> {
    let commit = git_line(
        Some(top),
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &format!("{rev}^{{commit}}"),
        ],
    )
    .map_err(|_| usage(format!("`{rev}` is not a commit")))?;
    let object = format!("{commit}:{full}");
    let kind = match git(Some(top), &["cat-file", "-t", &object]) {
        Ok(k) => String::from_utf8_lossy(&k).trim().to_string(),
        Err(_) => return Ok(None),
    };
    let mut specs: Vec<String> = vec![full.to_string()];
    if kind == "blob" && full.to_ascii_lowercase().ends_with(".pbip") {
        // a project file: its items are the sibling folders named after it
        let stem = full.get(..full.len() - 5).unwrap_or(full);
        specs.push(format!("{stem}.SemanticModel"));
        specs.push(format!("{stem}.Report"));
    }
    let mut args: Vec<&str> = vec!["ls-tree", "-r", "-z", "--full-name", &commit, "--"];
    args.extend(specs.iter().map(String::as_str));
    let listing = git(Some(top), &args)?;
    let mut blobs: Vec<(String, String)> = Vec::new();
    for entry in listing.split(|&b| b == 0).filter(|e| !e.is_empty()) {
        let entry = String::from_utf8_lossy(entry);
        let Some((meta, name)) = entry.split_once('\t') else {
            continue;
        };
        let mut f = meta.split(' ');
        let (mode, ty, oid) = (f.next(), f.next(), f.next());
        // regular files only: no links, no submodules; never the local .pbi cache
        if ty != Some("blob") || !matches!(mode, Some("100644" | "100755")) {
            continue;
        }
        if name.split('/').any(|c| c == ".pbi") {
            continue;
        }
        if !name.split('/').all(safe_component) {
            return Err((USAGE_ERROR, format!("{rev}: unsupported file name {name:?}")));
        }
        if let Some(oid) = oid {
            blobs.push((oid.to_string(), name.to_string()));
        }
        if blobs.len() > MAX_FILES {
            return Err((USAGE_ERROR, format!("{rev}:{full}: more than {MAX_FILES} files")));
        }
    }
    copy_blobs(top, &blobs, out)?;
    Ok(Some(out.join(full)))
}

/// Streams the blobs out of `git cat-file --batch` into files under `out`.
fn copy_blobs(top: &Path, blobs: &[(String, String)], out: &Path) -> Result<(), Failure> {
    let io = |e: std::io::Error| (USAGE_ERROR, format!("git cat-file: {e}"));
    let mut child = Command::new("git")
        .arg("-C")
        .arg(top)
        .args(["cat-file", "--batch"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| (USAGE_ERROR, format!("cannot run git: {e}")))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| (USAGE_ERROR, "git cat-file: no stdin".to_string()))?;
    let ids: String = blobs.iter().map(|(oid, _)| format!("{oid}\n")).collect();
    // written from another thread: git answers while it reads, and both pipes are bounded
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(ids.as_bytes());
    });
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| (USAGE_ERROR, "git cat-file: no stdout".to_string()))?;
    let mut reader = BufReader::new(stdout);
    let mut total = 0u64;
    let result = (|| -> Result<(), Failure> {
        for (oid, name) in blobs {
            let mut header = String::new();
            reader.read_line(&mut header).map_err(io)?;
            let mut f = header.trim_end().split(' ');
            let (got, ty, size) = (f.next(), f.next(), f.next().and_then(|s| s.parse::<u64>().ok()));
            let (Some(got), Some("blob"), Some(size)) = (got, ty, size) else {
                return Err((
                    USAGE_ERROR,
                    format!("git cat-file: unexpected answer for {oid}: {}", header.trim()),
                ));
            };
            if got != oid {
                return Err((USAGE_ERROR, format!("git cat-file: {got} instead of {oid}")));
            }
            total = total.saturating_add(size);
            if total > MAX_TOTAL_BYTES {
                return Err((USAGE_ERROR, format!("more than {MAX_TOTAL_BYTES} bytes to copy")));
            }
            let dest = out.join(name);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(io)?;
            }
            let mut file = std::fs::File::create(&dest).map_err(io)?;
            let copied = std::io::copy(&mut (&mut reader).take(size), &mut file).map_err(io)?;
            if copied != size {
                return Err((USAGE_ERROR, format!("git cat-file: {name} cut short")));
            }
            // the newline after the content
            let mut nl = [0u8; 1];
            reader.read_exact(&mut nl).map_err(io)?;
        }
        Ok(())
    })();
    drop(reader);
    let _ = writer.join();
    let _ = child.kill();
    let _ = child.wait();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_paths() {
        assert_eq!(
            repo_path("C:/r", "sub/", "x/../Sales.pbip").unwrap(),
            "sub/Sales.pbip"
        );
        assert_eq!(repo_path("C:/r", "", "./a\\b").unwrap(), "a/b");
        assert!(repo_path("C:/r", "", "../out").is_err());
        assert_eq!(
            repo_path("/home/r", "", "/home/r/p/Sales.pbix").unwrap(),
            "p/Sales.pbix"
        );
        assert!(repo_path("/home/r", "", "/elsewhere/x").is_err());
    }

    #[test]
    fn unsafe_names_are_refused() {
        for bad in ["", ".", "..", "a:b", "x\u{1}", "trail.", "a\\b"] {
            assert!(!safe_component(bad), "{bad:?}");
        }
        assert!(safe_component("Sales.SemanticModel"));
    }
}
