//! `cargo xtask deploy smoke`: `bicraft-cli serve` started the way the container starts it, without a
//! container. The arguments and variables come from `deploy/manifests/deployment.yaml` (container
//! paths mapped into a scratch folder, `--listen` moved to 127.0.0.1 on a free port), the API token
//! is made by `bicraft-cli serve token create` and mounted as the Secret would be, a generated PBIP
//! project (never client data) is uploaded zipped, and the live checks of `client::checks` run over
//! MCP Streamable HTTP, then the live viewer's checks (`client::viewer_checks`, with a reader token
//! and a reader grant the smoke adds), and `/metrics` is scraped on its own port (`--metrics-listen` moved to
//! 127.0.0.1 on a free port). Then the shutdown: an upload is left half-sent, the server is told to stop
//! (SIGTERM on Unix, as the kubelet does; the end of standard input on Windows, which has no
//! SIGTERM), new requests and `/readyz` must answer 503, the upload must still complete, and the
//! process must exit 0 having removed its temporary folders. With the manifest's JSON log, every
//! line must parse and none may carry the query, the fixture's data, a file name or the token.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use yaml_rust2::YamlLoader;

use super::check;
use super::client::{self, Endpoint};

const USAGE: &str = "\
usage: cargo xtask deploy smoke [--binary PATH] [--target TRIPLE] [--keep]

  --binary PATH    test this bicraft-cli binary (default: build it with
                   `cargo build --release --locked -p bicraft-cli --bin bicraft-cli`)
  --target TRIPLE  build for this target (e.g. x86_64-unknown-linux-musl, the image's binary)
  --keep           keep the scratch folder
";

struct Opts {
    binary: Option<PathBuf>,
    target: Option<String>,
    keep: bool,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        binary: None,
        target: None,
        keep: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--binary" => o.binary = Some(PathBuf::from(it.next().ok_or("--binary needs a path")?)),
            "--target" => o.target = Some(it.next().ok_or("--target needs a triple")?.clone()),
            "--keep" => o.keep = true,
            "-h" | "--help" => return Err(USAGE.to_owned()),
            other => return Err(format!("deploy smoke: unknown argument `{other}`\n\n{USAGE}")),
        }
    }
    Ok(o)
}

/// Builds the release binary (or takes `--binary`); its path.
pub fn binary(given: Option<PathBuf>, target: Option<&str>) -> Result<PathBuf, String> {
    if let Some(b) = given {
        return if b.is_file() {
            Ok(b)
        } else {
            Err(format!("{}: no such file", b.display()))
        };
    }
    let mut args = vec![
        "build",
        "--release",
        "--locked",
        "-p",
        "bicraft-cli",
        "--bin",
        "bicraft-cli",
    ];
    if let Some(t) = target {
        args.extend(["--target", t]);
    }
    crate::run_cargo(&args)?;
    let meta = crate::metadata()?;
    let dir = meta
        .get("target_directory")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or("cargo metadata: no target_directory")?;
    let dir = match target {
        Some(t) => dir.join(t),
        None => dir,
    };
    Ok(dir.join("release").join(if cfg!(windows) {
        "bicraft-cli.exe"
    } else {
        "bicraft-cli"
    }))
}

/// Runs the binary; its stdout (fails on a non-zero exit).
fn output(bin: &Path, args: &[&str]) -> Result<String, String> {
    let o = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{}: {e}", bin.display()))?;
    if !o.status.success() {
        return Err(format!(
            "bicraft {}: {}\n{}",
            args.join(" "),
            o.status,
            String::from_utf8_lossy(&o.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// `bicraft-cli serve token create`: (token, tokens-file content).
pub fn make_token(bin: &Path, name: &str) -> Result<(String, String), String> {
    let out = output(
        bin,
        &["serve", "token", "create", "--name", name, "--role", "agent"],
    )?;
    let token = out
        .lines()
        .find_map(|l| l.strip_prefix("token: "))
        .ok_or("serve token create printed no token")?
        .to_owned();
    let entry: Value = out
        .lines()
        .find_map(|l| l.strip_prefix("entry: "))
        .and_then(|e| serde_json::from_str(e).ok())
        .ok_or("serve token create printed no entry")?;
    Ok((token, serde_json::json!({"tokens": [entry]}).to_string()))
}

/// Two tokens files' entries in one file.
pub fn merge_tokens(a: &str, b: &str) -> Result<String, String> {
    let entries = |t: &str| -> Result<Vec<Value>, String> {
        let v: Value = serde_json::from_str(t).map_err(|e| format!("tokens file: {e}"))?;
        Ok(v.get("tokens")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    };
    let mut all = entries(a)?;
    all.extend(entries(b)?);
    Ok(serde_json::json!({"tokens": all}).to_string())
}

/// The fixture: a generated PBIP project with one imported table (Sales: 10 + 32), zipped. Returns
/// the archive and the project file's name.
pub fn fixture(bin: &Path, scratch: &Path) -> Result<(Vec<u8>, String), String> {
    let dir = scratch.join("fixture");
    let dir_s = dir.to_string_lossy().into_owned();
    output(bin, &["new-project", &dir_s, "Smoke"])?;
    let pbip = dir.join("Smoke.pbip");
    let import = r#"table.import={"name":"Sales","csv":"Region,Amount\nNorth,10\nSouth,32"}"#;
    output(
        bin,
        &[
            "run",
            "--open",
            &pbip.to_string_lossy(),
            "--cmd",
            import,
            "--save",
        ],
    )?;
    Ok((zip_dir(&dir)?, "Smoke.pbip".to_owned()))
}

pub fn zip_dir(dir: &Path) -> Result<Vec<u8>, String> {
    use zip::write::SimpleFileOptions;
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let mut stack = vec![dir.to_path_buf()];
    let mut files = Vec::new();
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)
            .map_err(|e| format!("{}: {e}", d.display()))?
            .flatten()
        {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                files.push(p);
            }
        }
    }
    files.sort();
    for f in files {
        let name = f
            .strip_prefix(dir)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let data = std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
        w.start_file(name, SimpleFileOptions::default())
            .and_then(|()| w.write_all(&data).map_err(Into::into))
            .map_err(|e| format!("zip: {e}"))?;
    }
    Ok(w.finish().map_err(|e| format!("zip: {e}"))?.into_inner())
}

/// The container as the plain manifest declares it.
fn manifest(root: &Path) -> Result<check::Container, String> {
    let path = root.join("deploy").join("manifests").join("deployment.yaml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let docs = YamlLoader::load_from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    docs.iter()
        .find_map(check::container)
        .ok_or_else(|| format!("{}: no container", path.display()))
}

/// The server's stderr, forwarded and kept.
struct Server {
    child: Child,
    lines: Arc<Mutex<Vec<String>>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn cmd(args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    let root = crate::root();
    let bin = binary(o.binary, o.target.as_deref())?;
    let scratch = std::env::temp_dir().join(format!("bicraft-deploy-smoke-{}", std::process::id()));
    if scratch.exists() {
        std::fs::remove_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    }
    std::fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let t0 = Instant::now();
    let result = smoke(&root, &bin, &scratch);
    match &result {
        Ok(()) if !o.keep => {
            let _ = std::fs::remove_dir_all(&scratch);
        }
        _ => eprintln!("deploy smoke: scratch kept in {}", scratch.display()),
    }
    if result.is_ok() {
        eprintln!(
            "deploy smoke: PASSED in {:.1} s ({})",
            t0.elapsed().as_secs_f64(),
            bin.display()
        );
    }
    result
}

fn smoke(root: &Path, bin: &Path, scratch: &Path) -> Result<(), String> {
    let version = output(bin, &["--version"])?;
    eprintln!("deploy smoke: {}", version.trim());
    let c = manifest(root)?;

    // Container paths -> scratch folders: every mount point, created like the kubelet would.
    let map = |p: &str| -> String {
        for (m, _) in &c.mounts {
            if p == m || p.starts_with(&format!("{m}/")) {
                let rest = p.trim_start_matches('/');
                return rest
                    .split('/')
                    .fold(scratch.join("fs"), |acc, c| acc.join(c))
                    .to_string_lossy()
                    .into_owned();
            }
        }
        if let Some(r) = check::arg_value(&c.args, "--root")
            && (p == r || p.starts_with(&format!("{r}/")))
        {
            return p
                .trim_start_matches('/')
                .split('/')
                .fold(scratch.join("fs"), |acc, c| acc.join(c))
                .to_string_lossy()
                .into_owned();
        }
        p.to_owned()
    };
    for (m, _) in &c.mounts {
        let d = PathBuf::from(map(m));
        std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    }

    // The API token, written where the Secret is mounted.
    let grants =
        check::env_value(&c, "BICRAFT_AUTH_GRANTS").ok_or("the manifest sets no BICRAFT_AUTH_GRANTS")?;
    let (token_name, workspace) = grants
        .split([';', ' '])
        .find_map(|g| {
            let (subject, ws) = g.rsplit_once('=')?;
            Some((
                subject.strip_prefix("token:")?.to_owned(),
                ws.split(',').next()?.to_owned(),
            ))
        })
        .ok_or("the manifest grants no API token")?;
    let tokens_path =
        map(check::env_value(&c, "BICRAFT_API_TOKENS_FILE").ok_or("no BICRAFT_API_TOKENS_FILE")?);
    let (token, tokens_json) = make_token(bin, &token_name)?;
    // a reader too, for the viewer checks (the manifest grants none; the smoke adds the grant)
    let (reader_token, reader_json) = make_token(bin, "smoke-reader")?;
    let tokens_json = merge_tokens(&tokens_json, &reader_json)?;
    std::fs::write(&tokens_path, tokens_json).map_err(|e| format!("{tokens_path}: {e}"))?;
    let host = check::env_value(&c, "BICRAFT_ALLOWED_HOSTS")
        .and_then(|h| {
            h.split(',')
                .find(|n| n.contains(".svc:"))
                .or_else(|| h.split(',').next())
        })
        .ok_or("the manifest sets no BICRAFT_ALLOWED_HOSTS")?
        .to_owned();

    let (zip, project) = fixture(bin, scratch)?;
    eprintln!(
        "deploy smoke: fixture {project} generated ({} bytes zipped)",
        zip.len()
    );

    // Arguments and variables of the manifest, mapped.
    let mut args: Vec<String> = Vec::new();
    let mut it = c.args.iter();
    while let Some(a) = it.next() {
        args.push(a.clone());
        if let Some(v) = it
            .clone()
            .next()
            .filter(|v| a.starts_with("--") && !v.starts_with("--"))
        {
            it.next();
            args.push(if a == "--listen" || a == "--metrics-listen" {
                "127.0.0.1:0".to_owned()
            } else {
                map(v)
            });
        }
    }
    if cfg!(windows) {
        args.push("--drain-on-stdin-close".into());
    }
    let mut command = Command::new(bin);
    command.arg("serve").args(&args);
    // Nothing from the developer's environment: only the manifest's variables.
    for (k, _) in std::env::vars_os() {
        let k = k.to_string_lossy();
        if k.starts_with("BICRAFT_") {
            command.env_remove(k.as_ref());
        }
    }
    for (k, v) in &c.env {
        command.env(k, map(v));
    }
    command.env(
        "BICRAFT_AUTH_READER_GRANTS",
        format!("token:smoke-reader={workspace}"),
    );
    eprintln!("deploy smoke: bicraft-cli serve {}", args.join(" "));
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", bin.display()))?;
    let stdin = child.stdin.take();
    let stderr = child.stderr.take().ok_or("no stderr")?;
    let lines = Arc::new(Mutex::new(Vec::new()));
    let mut server = Server {
        child,
        lines: Arc::clone(&lines),
    };
    let (tx, rx) = mpsc::channel();
    let (mtx, mrx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            eprintln!("  [bicraft] {line}");
            // kept before the address is announced: the startup check reads the kept lines
            if let Ok(mut l) = lines.lock() {
                l.push(line.clone());
            }
            if let Some(i) = line.find("metrics at http://") {
                let addr = line
                    .get(i + 18..)
                    .unwrap_or_default()
                    .split('/')
                    .next()
                    .unwrap_or_default();
                let _ = mtx.send(addr.to_owned());
            } else if let Some(i) = line.find("http://") {
                let addr = line
                    .get(i + 7..)
                    .unwrap_or_default()
                    .split('/')
                    .next()
                    .unwrap_or_default();
                let _ = tx.send(addr.to_owned());
            }
        }
    });
    let addr = rx
        .recv_timeout(Duration::from_secs(30))
        .map_err(|_| "bicraft-cli serve printed no endpoint within 30 s".to_owned())?;
    let ep = Endpoint {
        addr: addr.parse().map_err(|e| format!("endpoint {addr}: {e}"))?,
        host,
        token,
        workspace,
    };
    let startup = server.lines.lock().map(|l| l.join("\n")).unwrap_or_default();
    if !startup.contains("authentication: 2 API token(s)") {
        return Err(format!(
            "the server did not start with token authentication:\n{startup}"
        ));
    }

    let metrics = if check::arg_value(&c.args, "--metrics-listen").is_some() {
        let m = mrx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "bicraft-cli serve printed no metrics address".to_owned())?;
        Some(
            m.parse::<std::net::SocketAddr>()
                .map_err(|e| format!("metrics {m}: {e}"))?,
        )
    } else {
        None
    };

    client::checks(&ep, &zip, &project)?;
    client::viewer_checks(&ep, &reader_token, true)?;
    client::concurrency_checks(&ep)?;
    if let Some(m) = metrics {
        scrape_checks(m, &ep, &c)?;
    }
    drain(
        &mut server,
        stdin,
        metrics,
        &ep,
        &PathBuf::from(map(check::arg_value(&c.args, "--root").unwrap_or("/"))),
    )?;
    let tmp = PathBuf::from(map(check::env_value(&c, "TMPDIR").unwrap_or("/tmp")));
    let leftovers: Vec<_> = std::fs::read_dir(&tmp)
        .map(|r| r.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    if !leftovers.is_empty() {
        return Err(format!(
            "temporary files left in TMPDIR after the drain: {leftovers:?}"
        ));
    }
    eprintln!("deploy smoke: ok  TMPDIR empty after shutdown");
    if check::env_value(&c, "BICRAFT_LOG_FORMAT") == Some("json") {
        let log = server.lines.lock().map(|l| l.clone()).unwrap_or_default();
        log_checks(&log, &ep.token, &project)?;
    }
    Ok(())
}

/// GET `/metrics` on the metrics listener: the text exposition.
fn scrape(addr: std::net::SocketAddr) -> Result<String, String> {
    let mut s = client::connect(addr)?;
    s.write_all(b"GET /metrics HTTP/1.1\r\nHost: metrics\r\nConnection: close\r\n\r\n")
        .map_err(|e| format!("GET /metrics: {e}"))?;
    let r = client::read_response(s)?;
    if r.status != 200 {
        return Err(format!("GET /metrics: HTTP {} {}", r.status, r.text()));
    }
    Ok(r.text())
}

/// The value of a series written exactly as `name{labels}`.
fn series(text: &str, name: &str) -> Option<f64> {
    text.lines()
        .find_map(|l| l.strip_prefix(name)?.strip_prefix(' ')?.parse().ok())
}

/// `/metrics` after the live checks: a few series of each family, the memory limit of the
/// manifest, no model data, and nothing on the MCP port.
fn scrape_checks(addr: std::net::SocketAddr, ep: &Endpoint, c: &check::Container) -> Result<(), String> {
    let text = scrape(addr)?;
    let need = |name: &str, ok: &dyn Fn(f64) -> bool| -> Result<f64, String> {
        match series(&text, name) {
            Some(v) if ok(v) => Ok(v),
            v => Err(format!("/metrics: {name} = {v:?}\n{text}")),
        }
    };
    need("bicraft_draining", &|v| v == 0.0)?;
    need("bicraft_workspaces_open", &|v| v == 1.0)?;
    let queries = need(
        "bicraft_commands_total{command=\"dax.evaluate\",outcome=\"ok\"}",
        &|v| v >= 1.0,
    )?;
    need(
        "bicraft_command_duration_seconds_count{command=\"file.open\"}",
        &|v| v >= 1.0,
    )?;
    let mcp_ok = need("bicraft_http_requests_total{route=\"mcp\",code=\"200\"}", &|v| {
        v >= 1.0
    })?;
    need("bicraft_http_requests_total{route=\"mcp\",code=\"401\"}", &|v| {
        v >= 1.0
    })?;
    need("bicraft_http_requests_total{route=\"view\",code=\"200\"}", &|v| {
        v >= 1.0
    })?;
    need("bicraft_viewer_interactions_total", &|v| v >= 1.0)?;
    need("bicraft_tool_calls_running", &|v| v == 0.0)?;
    let limit = check::env_value(c, "BICRAFT_MEMORY_LIMIT")
        .and_then(check::bicraft_size_mib)
        .map(|mib| (mib * 1024 * 1024) as f64);
    need("bicraft_memory_limit_bytes", &|v| Some(v) == limit)?;
    need("bicraft_memory_reserved_bytes", &|v| v >= 0.0)?;
    need("bicraft_memory_evictions_total", &|v| v >= 0.0)?;
    need("bicraft_spill_bytes", &|v| v >= 0.0)?;
    if text.contains("SUM(Sales") || text.contains(&ep.token) {
        return Err(format!("/metrics carries a query or the token:\n{text}"));
    }
    let r = client::request(ep, "GET", "/metrics", &[], b"", true)?;
    if r.status != 404 {
        return Err(format!(
            "/metrics on the MCP port: HTTP {} (must be 404)",
            r.status
        ));
    }
    eprintln!(
        "deploy smoke: ok  GET /metrics on its own port: {} dax.evaluate ok, {} MCP 200, memory limit {} bytes, \
         {} series; 404 on the MCP port",
        queries,
        mcp_ok,
        limit.unwrap_or(0.0),
        text.lines().filter(|l| !l.starts_with('#')).count()
    );
    Ok(())
}

/// The JSON log: every line parses, the requests are there, and nothing in it is model data, a
/// file name or a credential.
fn log_checks(lines: &[String], token: &str, project: &str) -> Result<(), String> {
    let forbidden = [
        client::QUERY,
        "Sales[Amount]",
        "North",
        "South",
        project,
        "late-upload.txt",
        token,
    ];
    let mut requests = 0usize;
    let mut queries = 0usize;
    for line in lines {
        let v: Value =
            serde_json::from_str(line).map_err(|e| format!("not a JSON log line ({e}): {line}"))?;
        if v.get("ts").and_then(Value::as_str).is_none() {
            return Err(format!("a log line without ts: {line}"));
        }
        if let Some(bad) = forbidden.iter().find(|f| !f.is_empty() && line.contains(*f)) {
            return Err(format!("the log carries `{bad}`: {line}"));
        }
        if v.get("event").and_then(Value::as_str) == Some("request") {
            requests += 1;
            if v.get("command").and_then(Value::as_str) == Some("dax.evaluate") {
                queries += 1;
            }
        }
    }
    if requests == 0 || queries == 0 {
        return Err(format!(
            "the JSON log has {requests} request line(s), {queries} for dax.evaluate"
        ));
    }
    eprintln!(
        "deploy smoke: ok  JSON log: {} lines, {requests} requests ({queries} dax.evaluate), no query text, \
         data, file name or token",
        lines.len()
    );
    Ok(())
}

/// The shutdown: a running upload finishes, new requests get 503, exit 0.
fn drain(
    server: &mut Server,
    stdin: Option<std::process::ChildStdin>,
    metrics: Option<std::net::SocketAddr>,
    ep: &Endpoint,
    root: &Path,
) -> Result<(), String> {
    let path = format!("/w/{}/files/late-upload.txt", ep.workspace);
    let body = vec![b'x'; 256 * 1024];
    let half = body.len() / 2;
    let mut up = client::connect(ep.addr)?;
    let head = client::head(ep, "PUT", &path, &[], body.len(), true);
    up.write_all(format!("{head}\r\n").as_bytes())
        .and_then(|()| up.write_all(body.get(..half).unwrap_or_default()))
        .map_err(|e| format!("slow upload: {e}"))?;
    std::thread::sleep(Duration::from_millis(300));

    // Stop: SIGTERM where there are signals, the end of standard input on Windows.
    if cfg!(unix) {
        let pid = server.child.id().to_string();
        let st = Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .map_err(|e| format!("kill: {e}"))?;
        if !st.success() {
            return Err(format!("kill -TERM {pid}: {st}"));
        }
        eprintln!("deploy smoke: sent SIGTERM to {pid} with an upload half-sent");
        drop(stdin);
    } else {
        drop(stdin);
        eprintln!("deploy smoke: closed the server's standard input with an upload half-sent");
    }
    let start = Instant::now();
    loop {
        let r = client::request(ep, "GET", "/readyz", &[], b"", false)?;
        if r.status == 503 {
            eprintln!(
                "deploy smoke: ok  GET /readyz while draining -> 503 {}",
                r.text().trim()
            );
            break;
        }
        if start.elapsed() > Duration::from_secs(10) {
            return Err(format!(
                "/readyz still {} 10 s after the shutdown signal",
                r.status
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let r = client::mcp(ep, "server/discover", serde_json::json!({}), false, true)?;
    if r.status != 503 || r.header("retry-after").is_none() {
        return Err(format!(
            "a new request while draining: HTTP {} {}",
            r.status,
            r.text()
        ));
    }
    eprintln!("deploy smoke: ok  a new MCP request while draining -> 503 with Retry-After");
    if let Some(m) = metrics {
        let text = scrape(m)?;
        if series(&text, "bicraft_draining") != Some(1.0) {
            return Err(format!(
                "/metrics while draining: bicraft_draining is not 1:\n{text}"
            ));
        }
        eprintln!(
            "deploy smoke: ok  /metrics while draining: bicraft_draining 1 (the metrics port stays up)"
        );
    }

    up.write_all(body.get(half..).unwrap_or_default())
        .map_err(|e| format!("slow upload, second half: {e}"))?;
    let r = client::read_response(up)?;
    if r.status != 201 {
        return Err(format!(
            "the upload running at shutdown: HTTP {} {}",
            r.status,
            r.text()
        ));
    }
    let stored = std::fs::metadata(root.join(&ep.workspace).join("late-upload.txt")).map(|m| m.len());
    if stored.as_ref().ok() != Some(&(body.len() as u64)) {
        return Err(format!(
            "the upload running at shutdown was not stored whole: {stored:?}"
        ));
    }
    eprintln!(
        "deploy smoke: ok  the upload running at shutdown completed (201, {} bytes stored)",
        body.len()
    );

    let start = Instant::now();
    let status = loop {
        if let Some(s) = server.child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if start.elapsed() > Duration::from_secs(60) {
            return Err("bicraft-cli serve did not exit within 60 s of the shutdown".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    std::thread::sleep(Duration::from_millis(200));
    let log = server.lines.lock().map(|l| l.join("\n")).unwrap_or_default();
    if status.code() != Some(0) || !log.contains("every request finished") || !log.contains("exit 0") {
        return Err(format!("unclean drain: {status}\n{log}"));
    }
    eprintln!(
        "deploy smoke: ok  drained in {} ms, exit code 0 ({})",
        start.elapsed().as_millis(),
        log.lines()
            .rfind(|l| l.contains("exit 0"))
            .unwrap_or_default()
            .trim()
    );
    Ok(())
}
