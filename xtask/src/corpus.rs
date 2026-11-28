//! `cargo xtask corpus verify|fetch|stage`: the pinned corpus of files we author ourselves
//! (ROADMAP M11.8, docs/corpus.md).
//!
//! `corpus/manifest.json` lists every file with its size and SHA-256. The files live outside the
//! repository, in the folder named by `BICRAFT_CORPUS` (or `--dir`). `verify` checks each file
//! against the manifest; `fetch` downloads the missing or wrong ones from a base URL
//! (`BICRAFT_CORPUS_URL`, `--url`, or the manifest's `base_url`) and checks them the same way.
//! Without a base URL `fetch` prints a note and does nothing.
//!
//! The maintainers host the files as the assets of a GitHub release (2026-11-24): a base URL
//! `https://github.com/<owner>/<repo>/releases/download/<tag>` is read through the GitHub API,
//! authenticated by `GITHUB_TOKEN`, `GH_TOKEN` or `gh auth token` when one is found (the
//! repository is public, so its release also answers without a token; a private fork's needs
//! one); the token goes in a header only, never in a message. A release's assets have flat names, so
//! each file is stored under [`asset_name`] of its path. `stage` copies a verified corpus under
//! those names into a folder and prints the `gh release create` command that uploads it; it never
//! runs it.

// Without `net`, `fetch` re-runs xtask with the HTTP client; the download code is not compiled.
#![cfg_attr(not(feature = "net"), allow(dead_code))]

use std::path::{Path, PathBuf};

use serde_json::Value;

/// The folder that holds the corpus files.
pub const DIR_VAR: &str = "BICRAFT_CORPUS";
/// The base URL `fetch` downloads from (`<base>/<path>` for each file).
pub const URL_VAR: &str = "BICRAFT_CORPUS_URL";
/// Largest file the manifest may list (a corpus of small synthetic models).
const MAX_FILE: u64 = 512 << 20;
/// Longest release asset name kept (GitHub's limit on a file name is 255 bytes).
const MAX_ASSET_NAME: usize = 255;
/// The token variables read for a private release, in order (then `gh auth token`).
const TOKEN_VARS: [&str; 2] = ["GITHUB_TOKEN", "GH_TOKEN"];

/// One pinned file of the corpus.
#[derive(Debug, Clone, PartialEq)]
pub struct File {
    /// Path relative to the corpus folder, `/`-separated.
    pub path: String,
    pub size: u64,
    /// Lower-case hex.
    pub sha256: String,
}

#[derive(Debug, Default)]
pub struct Manifest {
    pub base_url: Option<String>,
    pub files: Vec<File>,
}

/// The committed manifest.
pub fn manifest_path() -> PathBuf {
    crate::root().join("corpus").join("manifest.json")
}

/// A safe relative path: no root, no drive, no `..`, no backslash, no empty segment.
fn check_path(p: &str) -> Result<(), String> {
    let bad = p.is_empty()
        || p.starts_with('/')
        || p.contains('\\')
        || p.contains(':')
        || p.split('/').any(|s| s.is_empty() || s == "." || s == "..");
    if bad {
        Err(format!("unsafe corpus path `{p}`"))
    } else {
        Ok(())
    }
}

/// The name of the release asset that holds corpus file `path`: its segments joined by `__`
/// (`shop/Shop.Report/.platform` is `shop__Shop.Report__.platform`). Only ASCII letters, digits,
/// `.`, `-` and `_`, with no leading or trailing `.` and no `..`, so GitHub stores the asset under
/// that very name.
pub fn asset_name(path: &str) -> Result<String, String> {
    check_path(path)?;
    let name = path.replace('/', "__");
    let ok = name.len() <= MAX_ASSET_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        && !name.starts_with('.')
        && !name.ends_with('.')
        && !name.contains("..");
    if ok {
        Ok(name)
    } else {
        Err(format!(
            "corpus path `{path}` has no safe release asset name (ASCII letters, digits, `.`, `-`, `_`; \
             no leading or trailing `.`): rename it"
        ))
    }
}

/// `(owner, repo, tag)` when `base` is a GitHub release download URL,
/// `https://github.com/<owner>/<repo>/releases/download/<tag>`.
pub fn github_release(base: &str) -> Option<(String, String, String)> {
    let rest = base.trim_end_matches('/').strip_prefix("https://github.com/")?;
    let parts: Vec<&str> = rest.split('/').collect();
    let safe = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    };
    match parts.as_slice() {
        [owner, repo, "releases", "download", tag] if [owner, repo, tag].iter().all(|s| safe(s)) => {
            Some(((*owner).to_owned(), (*repo).to_owned(), (*tag).to_owned()))
        }
        _ => None,
    }
}

pub fn parse(text: &str) -> Result<Manifest, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("manifest: bad JSON: {e}"))?;
    let base_url = v
        .get("base_url")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let entries = v
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("manifest: no `entries` array")?;
    let mut files = Vec::new();
    for e in entries {
        let name = e.get("name").and_then(Value::as_str).unwrap_or("?");
        let list = e
            .get("files")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("manifest: entry `{name}` has no `files`"))?;
        for f in list {
            let path = f
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("manifest: entry `{name}`: a file has no `path`"))?;
            check_path(path)?;
            let size = f
                .get("size")
                .and_then(Value::as_u64)
                .filter(|s| *s <= MAX_FILE)
                .ok_or_else(|| format!("manifest: {path}: missing or too large `size`"))?;
            let sha256 = f
                .get("sha256")
                .and_then(Value::as_str)
                .filter(|s| s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
                .ok_or_else(|| format!("manifest: {path}: `sha256` is not 64 lower-case hex digits"))?;
            if files.iter().any(|x: &File| x.path == path) {
                return Err(format!("manifest: {path} is listed twice"));
            }
            let asset = asset_name(path).map_err(|e| format!("manifest: {e}"))?;
            if let Some(other) = files
                .iter()
                .find(|x: &&File| asset_name(&x.path).is_ok_and(|a| a == asset))
            {
                return Err(format!(
                    "manifest: {path} and {} share the release asset name {asset}",
                    other.path
                ));
            }
            files.push(File {
                path: path.to_owned(),
                size,
                sha256: sha256.to_owned(),
            });
        }
    }
    Ok(Manifest { base_url, files })
}

pub fn load() -> Result<Manifest, String> {
    let p = manifest_path();
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    parse(&text)
}

/// What `verify` found for one file.
#[derive(Debug, PartialEq)]
pub enum State {
    Ok,
    Missing,
    /// The size on disk.
    WrongSize(u64),
    /// The SHA-256 on disk.
    WrongHash(String),
}

pub fn local(dir: &Path, f: &File) -> PathBuf {
    f.path.split('/').fold(dir.to_path_buf(), |p, s| p.join(s))
}

pub fn check(dir: &Path, f: &File) -> Result<State, String> {
    let p = local(dir, f);
    let meta = match std::fs::metadata(&p) {
        Ok(m) if m.is_file() => m,
        Ok(_) => return Err(format!("{}: not a file", p.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(State::Missing),
        Err(e) => return Err(format!("{}: {e}", p.display())),
    };
    if meta.len() != f.size {
        return Ok(State::WrongSize(meta.len()));
    }
    let data = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let sha = crate::font_subset::sha256_hex(&data);
    Ok(if sha == f.sha256 {
        State::Ok
    } else {
        State::WrongHash(sha)
    })
}

fn describe(f: &File, s: &State) -> String {
    match s {
        State::Ok => format!("ok       {}", f.path),
        State::Missing => format!("missing  {}", f.path),
        State::WrongSize(n) => format!("SIZE     {}: {n} bytes, expected {}", f.path, f.size),
        State::WrongHash(h) => format!("SHA-256  {}: {h}, expected {}", f.path, f.sha256),
    }
}

/// Checks every file; missing files and mismatches are errors.
pub fn verify(dir: &Path, m: &Manifest) -> Result<(), String> {
    let mut bad = 0usize;
    for f in &m.files {
        let s = check(dir, f)?;
        println!("{}", describe(f, &s));
        if s != State::Ok {
            bad += 1;
        }
    }
    if bad == 0 {
        println!("OK: {} corpus file(s) match the manifest.", m.files.len());
        Ok(())
    } else {
        Err(format!(
            "{bad} of {} corpus file(s) missing or not matching the manifest ({}); \
             fetch them with `cargo xtask corpus fetch`",
            m.files.len(),
            dir.display()
        ))
    }
}

struct Opts {
    dir: Option<PathBuf>,
    url: Option<String>,
    out: Option<PathBuf>,
}

fn opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        dir: bicraft_env::var_os(DIR_VAR).map(PathBuf::from),
        url: bicraft_env::var(URL_VAR).ok().filter(|s| !s.is_empty()),
        out: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dir" => o.dir = Some(PathBuf::from(it.next().ok_or("--dir needs a folder")?)),
            "--url" => o.url = Some(it.next().ok_or("--url needs a base URL")?.clone()),
            "--out" => o.out = Some(PathBuf::from(it.next().ok_or("--out needs a folder")?)),
            other => return Err(format!("corpus: unknown argument `{other}`")),
        }
    }
    Ok(o)
}

/// The command that publishes a staged corpus folder `out` as the assets of release `tag` of
/// `owner/repo` (printed by `stage`, run by a maintainer).
pub fn release_command(owner: &str, repo: &str, tag: &str, out: &Path, m: &Manifest) -> String {
    let mut cmd = format!(
        "gh release create {tag} --repo {owner}/{repo} --title \"Pinned corpus ({tag})\" \
         --notes \"Synthetic Power BI files authored by the BIcraft project, pinned by SHA-256 in \
         corpus/manifest.json (docs/corpus.md). No client material.\""
    );
    for f in &m.files {
        if let Ok(a) = asset_name(&f.path) {
            let p = out.join(a);
            cmd.push_str(&format!(" \"{}\"", p.display()));
        }
    }
    cmd
}

/// Copies every corpus file of `dir` (verified first) into the new or empty folder `out` under
/// its release asset name, then lists them and the upload command. Nothing is uploaded.
pub fn stage(dir: &Path, out: &Path, m: &Manifest) -> Result<(), String> {
    verify(dir, m)?;
    if out.exists() {
        let mut entries = std::fs::read_dir(out).map_err(|e| format!("{}: {e}", out.display()))?;
        if entries.next().is_some() {
            return Err(format!(
                "{}: not empty; stage into a new or empty folder",
                out.display()
            ));
        }
    }
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("staged under {}:", out.display());
    for f in &m.files {
        let a = asset_name(&f.path)?;
        let to = out.join(&a);
        std::fs::copy(local(dir, f), &to).map_err(|e| format!("{}: {e}", to.display()))?;
        println!("  {a}  {} bytes  {}", f.size, f.sha256);
    }
    match m.base_url.as_deref().and_then(github_release) {
        Some((owner, repo, tag)) => {
            println!("upload (a maintainer runs it; nothing was uploaded):");
            println!("  {}", release_command(&owner, &repo, &tag, out, m));
        }
        None => println!(
            "note: the manifest's base_url is not a GitHub release URL; upload the folder where the \
             corpus is hosted (docs/corpus.md)"
        ),
    }
    Ok(())
}

pub fn cmd(args: &[String]) -> Result<(), String> {
    let sub = args.first().map(String::as_str);
    let rest = args.get(1..).unwrap_or(&[]);
    match sub {
        Some("verify") => {
            let o = opts(rest)?;
            let dir = o
                .dir
                .ok_or_else(|| format!("corpus verify: set {DIR_VAR} or pass --dir DIR"))?;
            verify(&dir, &load()?)
        }
        Some("fetch") => {
            let o = opts(rest)?;
            let m = load()?;
            let Some(url) = o.url.or_else(|| m.base_url.clone()) else {
                println!(
                    "note: no corpus URL configured (set {URL_VAR}, pass --url, or fill the \
                     manifest's base_url); nothing fetched. The corpus is hosted as the \
                     maintainers decide (docs/corpus.md)."
                );
                return Ok(());
            };
            let dir = o
                .dir
                .ok_or_else(|| format!("corpus fetch: set {DIR_VAR} or pass --dir DIR"))?;
            fetch(&dir, &url, rest)
        }
        Some("stage") => {
            let o = opts(rest)?;
            let dir = o
                .dir
                .ok_or_else(|| format!("corpus stage: set {DIR_VAR} or pass --dir DIR"))?;
            let out = o
                .out
                .ok_or("corpus stage: pass --out DIR (a new or empty folder)")?;
            stage(&dir, &out, &load()?)
        }
        _ => Err(
            "usage: cargo xtask corpus verify|fetch [--dir DIR] [--url BASE] | stage --out DIR [--dir DIR]"
                .into(),
        ),
    }
}

/// The token for a private GitHub release: `GITHUB_TOKEN`, `GH_TOKEN`, else what `gh auth token`
/// prints (when `gh` is installed and signed in). Never printed.
pub fn github_token() -> Option<String> {
    for v in TOKEN_VARS {
        if let Some(t) = bicraft_env::var(v)
            .ok()
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
        {
            return Some(t);
        }
    }
    let out = std::process::Command::new("gh")
        .args(["auth", "token"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let t = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (!t.is_empty() && t.len() <= 4096 && t.bytes().all(|b| b.is_ascii_graphic())).then_some(t)
}

/// The release's assets, `(name, API URL, size)`, from the release's JSON.
pub fn release_assets(release: &Value) -> Result<Vec<(String, String, u64)>, String> {
    let list = release
        .get("assets")
        .and_then(Value::as_array)
        .ok_or("the release has no `assets` list")?;
    Ok(list
        .iter()
        .filter_map(|a| {
            Some((
                a.get("name")?.as_str()?.to_owned(),
                a.get("url")?.as_str()?.to_owned(),
                a.get("size")?.as_u64()?,
            ))
        })
        .collect())
}

/// `<base>/<path>`.
pub fn file_url(base: &str, path: &str) -> String {
    format!("{}/{path}", base.trim_end_matches('/'))
}

#[cfg(not(feature = "net"))]
fn fetch(_: &Path, _: &str, args: &[String]) -> Result<(), String> {
    // Re-run with the HTTP client compiled in, in its own target dir (as schema-watch does).
    let base = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::root().join("target"));
    let mut c = crate::cargo();
    c.args([
        "run",
        "--quiet",
        "-p",
        "xtask",
        "--features",
        "net",
        "--target-dir",
    ]);
    c.arg(base.join("xtask-net"));
    c.args(["--", "corpus", "fetch"]);
    c.args(args);
    crate::run(c, "xtask corpus fetch (with the HTTP client)")
}

/// Where `fetch` reads a file: a plain URL, or a GitHub release asset through the API.
#[cfg(feature = "net")]
enum Origin {
    Plain(String),
    Asset { url: String, token: String },
}

/// Reads at most `limit` bytes of `resp`'s body.
#[cfg(feature = "net")]
fn body(resp: ureq::http::Response<ureq::Body>, limit: u64, what: &str) -> Result<Vec<u8>, String> {
    use std::io::Read as _;
    let mut data = Vec::new();
    resp.into_body()
        .into_reader()
        .take(limit)
        .read_to_end(&mut data)
        .map_err(|e| format!("{what}: {e}"))?;
    Ok(data)
}

/// The API's answer for a release asset or listing: the status explained, the token never shown.
#[cfg(feature = "net")]
fn api_error(what: &str, e: ureq::Error) -> String {
    match e {
        ureq::Error::StatusCode(401 | 403) => format!(
            "{what}: refused by GitHub (HTTP 401/403): the token cannot read this repository \
             (set GITHUB_TOKEN to a token with read access, or `gh auth login`)"
        ),
        ureq::Error::StatusCode(404) => format!(
            "{what}: not found (HTTP 404): no such release yet, or a private fork's release read \
             without a token (set GITHUB_TOKEN or GH_TOKEN, or sign in with `gh auth login`)"
        ),
        e => format!("{what}: {e}"),
    }
}

#[cfg(feature = "net")]
fn fetch(dir: &Path, base: &str, _: &[String]) -> Result<(), String> {
    let m = load()?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(300)))
        // a release asset redirects to a signed storage URL: the token stays with api.github.com
        .redirect_auth_headers(ureq::config::RedirectAuthHeaders::Never)
        .build()
        .into();
    let release = github_release(base);
    let token = release.as_ref().and_then(|_| github_token());
    // a GitHub release read through the API (a private repository needs it), its assets by name
    let assets = match (&release, &token) {
        (Some((owner, repo, tag)), Some(token)) => {
            let what = format!("release {tag} of {owner}/{repo}");
            let url = format!("https://api.github.com/repos/{owner}/{repo}/releases/tags/{tag}");
            let resp = agent
                .get(&url)
                .header("User-Agent", "bicraft-xtask-corpus")
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .header("Authorization", &format!("Bearer {token}"))
                .call()
                .map_err(|e| api_error(&what, e))?;
            let text = body(resp, 16 << 20, &what)?;
            let v: Value = serde_json::from_slice(&text).map_err(|e| format!("{what}: bad JSON: {e}"))?;
            println!("source   {what} (GitHub API, authenticated)");
            Some(release_assets(&v).map_err(|e| format!("{what}: {e}"))?)
        }
        (Some((owner, repo, tag)), None) => {
            println!(
                "note: no GitHub token (GITHUB_TOKEN, GH_TOKEN or `gh auth token`): reading release \
                 {tag} of {owner}/{repo} anonymously, which works only for a public repository"
            );
            None
        }
        _ => None,
    };
    let mut got = 0usize;
    for f in &m.files {
        if check(dir, f)? == State::Ok {
            continue;
        }
        let from = match (&release, &assets, &token) {
            (Some(_), Some(list), Some(token)) => {
                let name = asset_name(&f.path)?;
                let (_, url, size) = list
                    .iter()
                    .find(|(n, _, _)| *n == name)
                    .ok_or_else(|| format!("{}: the release has no asset `{name}`", f.path))?;
                if *size != f.size {
                    return Err(format!(
                        "{}: the release's asset `{name}` is {size} bytes, expected {}",
                        f.path, f.size
                    ));
                }
                Origin::Asset {
                    url: url.clone(),
                    token: token.clone(),
                }
            }
            (Some(_), _, _) => Origin::Plain(file_url(base, &asset_name(&f.path)?)),
            _ => Origin::Plain(file_url(base, &f.path)),
        };
        let (url, resp) = match &from {
            Origin::Plain(url) => {
                println!("fetch    {url}");
                let r = agent
                    .get(url)
                    .header("User-Agent", "bicraft-xtask-corpus")
                    .call()
                    .map_err(|e| api_error(url, e))?;
                (url.clone(), r)
            }
            Origin::Asset { url, token } => {
                println!("fetch    {} (release asset)", f.path);
                let r = agent
                    .get(url)
                    .header("User-Agent", "bicraft-xtask-corpus")
                    .header("Accept", "application/octet-stream")
                    .header("X-GitHub-Api-Version", "2022-11-28")
                    .header("Authorization", &format!("Bearer {token}"))
                    .call()
                    .map_err(|e| api_error(&f.path, e))?;
                (f.path.clone(), r)
            }
        };
        let data = body(resp, f.size.saturating_add(1), &url)?;
        if data.len() as u64 != f.size {
            return Err(format!("{url}: {} bytes, expected {}", data.len(), f.size));
        }
        let sha = crate::font_subset::sha256_hex(&data);
        if sha != f.sha256 {
            return Err(format!("{url}: SHA-256 {sha}, expected {}", f.sha256));
        }
        let p = local(dir, f);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        // Written beside, then renamed: an interrupted fetch never leaves a half file.
        let part = p.with_extension("part");
        std::fs::write(&part, &data).map_err(|e| format!("{}: {e}", part.display()))?;
        std::fs::rename(&part, &p).map_err(|e| format!("{}: {e}", p.display()))?;
        got += 1;
    }
    println!("fetched {got} file(s)");
    verify(dir, &m)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn manifest(path: &str, size: u64, sha: &str) -> String {
        serde_json::json!({
            "entries": [{"name": "t", "files": [{"path": path, "size": size, "sha256": sha}]}]
        })
        .to_string()
    }

    #[test]
    fn the_committed_manifest_parses() {
        let m = load().unwrap();
        assert!(!m.files.is_empty());
    }

    #[test]
    fn unsafe_paths_and_bad_hashes_are_refused() {
        for p in ["", "/a", "a/../b", "a\\b", "C:x", "a//b", "./a"] {
            assert!(parse(&manifest(p, 1, SHA_ABC)).is_err(), "{p}");
        }
        assert!(parse(&manifest("a", 1, "ABC")).is_err());
        assert!(parse(&manifest("a", 1, &SHA_ABC.to_uppercase())).is_err());
        assert!(parse(&manifest("a", u64::MAX, SHA_ABC)).is_err());
        assert!(parse(&manifest("dir/a.pbix", 3, SHA_ABC)).is_ok());
    }

    #[test]
    fn verify_reports_missing_wrong_size_and_wrong_hash() {
        let dir = std::env::temp_dir().join(format!("bicraft-corpus-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let m = parse(&manifest("sub/abc.txt", 3, SHA_ABC)).unwrap();
        let f = &m.files[0];
        assert_eq!(check(&dir, f).unwrap(), State::Missing);
        assert!(verify(&dir, &m).is_err());
        std::fs::write(dir.join("sub").join("abc.txt"), b"abcd").unwrap();
        assert_eq!(check(&dir, f).unwrap(), State::WrongSize(4));
        std::fs::write(dir.join("sub").join("abc.txt"), b"abd").unwrap();
        assert!(matches!(check(&dir, f).unwrap(), State::WrongHash(_)));
        assert!(verify(&dir, &m).is_err());
        std::fs::write(dir.join("sub").join("abc.txt"), b"abc").unwrap();
        assert_eq!(check(&dir, f).unwrap(), State::Ok);
        assert!(verify(&dir, &m).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn release_asset_names_are_flat_and_safe() {
        assert_eq!(asset_name("basic.pbix").unwrap(), "basic.pbix");
        assert_eq!(
            asset_name("shop/Shop.Report/.platform").unwrap(),
            "shop__Shop.Report__.platform"
        );
        for p in [".gitignore", "a b.pbix", "dir/naïve.pbix", "a/..b", "x."] {
            assert!(asset_name(p).is_err(), "{p}");
        }
        // two paths with one asset name are refused
        let two = serde_json::json!({"entries": [{"name": "t", "files": [
            {"path": "a/b.pbix", "size": 3, "sha256": SHA_ABC},
            {"path": "a__b.pbix", "size": 3, "sha256": SHA_ABC}]}]});
        assert!(parse(&two.to_string()).is_err());
    }

    #[test]
    fn the_committed_manifest_points_at_the_github_release() {
        let m = load().unwrap();
        let base = m.base_url.as_deref().expect("a base_url");
        assert_eq!(
            github_release(base),
            Some(("digorpeptra".into(), "BIcraft".into(), "corpus-v1".into()))
        );
        // every file has its own asset name
        let mut names: Vec<String> = m.files.iter().map(|f| asset_name(&f.path).unwrap()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), m.files.len());
    }

    #[test]
    fn github_release_urls_are_recognised() {
        assert_eq!(
            github_release("https://github.com/o/r/releases/download/t-1/"),
            Some(("o".into(), "r".into(), "t-1".into()))
        );
        for u in [
            "https://example.com/o/r/releases/download/t",
            "https://github.com/o/r/releases/tag/t",
            "https://github.com/o/r/releases/download/t/extra",
            "http://github.com/o/r/releases/download/t",
            "https://github.com/o/r/releases/download/t?x=1",
        ] {
            assert_eq!(github_release(u), None, "{u}");
        }
    }

    #[test]
    fn release_assets_are_read_by_name_and_the_command_lists_every_file() {
        let v = serde_json::json!({"assets": [
            {"name": "basic.pbix", "url": "https://api.github.com/repos/o/r/releases/assets/1", "size": 3},
            {"name": "broken"}]});
        assert_eq!(
            release_assets(&v).unwrap(),
            vec![(
                "basic.pbix".to_string(),
                "https://api.github.com/repos/o/r/releases/assets/1".to_string(),
                3
            )]
        );
        assert!(release_assets(&serde_json::json!({})).is_err());
        let m = parse(&manifest("shop/a.pbix", 3, SHA_ABC)).unwrap();
        let cmd = release_command("o", "r", "corpus-v1", Path::new("out"), &m);
        assert!(
            cmd.starts_with("gh release create corpus-v1 --repo o/r "),
            "{cmd}"
        );
        assert!(cmd.contains("shop__a.pbix\""), "{cmd}");
    }

    #[test]
    fn stage_copies_a_verified_corpus_under_asset_names() {
        let root = std::env::temp_dir().join(format!("bicraft-corpus-stage-{}", std::process::id()));
        let (dir, out) = (root.join("c"), root.join("out"));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let m = parse(&manifest("sub/abc.txt", 3, SHA_ABC)).unwrap();
        assert!(stage(&dir, &out, &m).is_err(), "a missing file is refused");
        std::fs::write(dir.join("sub").join("abc.txt"), b"abc").unwrap();
        stage(&dir, &out, &m).unwrap();
        assert_eq!(std::fs::read(out.join("sub__abc.txt")).unwrap(), b"abc");
        assert!(
            stage(&dir, &out, &m).is_err(),
            "a folder that is not empty is refused"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn file_urls_join_with_one_slash() {
        assert_eq!(file_url("https://h/c/", "a/b.pbix"), "https://h/c/a/b.pbix");
        assert_eq!(file_url("https://h/c", "a.pbix"), "https://h/c/a.pbix");
    }
}
