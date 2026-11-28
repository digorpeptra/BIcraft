//! The UI control channel in a real browser (docs/webui.md, "Control channel in the browser"):
//! headless Firefox loads the browser app from `bicraft-cli view --control` and from
//! `bicraft-cli serve --viewer-control`, and `bicraft-cli ui` drives the page through the server's relay:
//! open a file, run a command, switch the page and the ribbon tab, the palette and a form, and a
//! screenshot of the canvas as the GPU drew it.
//!
//! Ignored by default: it needs a `bicraft-cli` built with the browser app embedded
//! (`cargo xtask webui`, then `BICRAFT_WEBUI_DIR` at build time) and Firefox. Run it with
//!
//! ```text
//! set BICRAFT_FIREFOX=C:\Program Files\Mozilla Firefox\firefox.exe   # or /usr/bin/firefox
//! set BICRAFT_BROWSER_SHOTS=<DIR>                                     # optional: keeps the PNGs
//! cargo test -p bicraft-cli --test browser_control -- --ignored --test-threads 1
//! ```
//!
//! Each test prints a note and passes when Firefox or the embedded app is missing. Firefox runs
//! headless with a throwaway profile; nothing drives it but the page itself: every action is a
//! control request, never synthetic input.

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;
use tempdir::TempDir;

/// Kills the child when the test ends, even on failure.
struct Kill(Child);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/engine/tests/fixtures/layout-oracle/visuals/legacy")
}

fn firefox() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("BICRAFT_FIREFOX")?);
    p.is_file().then_some(p)
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Starts `bicraft ARGS` and waits for a stderr line containing `ready`; `None` (with a note)
/// when the binary has no browser app.
fn spawn(args: &[&str], ready: &str) -> Option<Kill> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_bicraft-cli"))
        .args(args)
        .env_remove("BICRAFT_VIEWER_CONTROL_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let child = Kill(child);
    let (tx, rx) = mpsc::channel();
    let ready = ready.to_string();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            eprintln!("[bicraft] {line}");
            if line.contains(&ready) {
                let _ = tx.send(true);
            } else if line.contains("without the browser app") {
                let _ = tx.send(false);
            }
        }
    });
    match rx.recv_timeout(Duration::from_secs(60)) {
        Ok(true) => Some(child),
        Ok(false) => {
            eprintln!("note: this bicraft has no browser app embedded (BICRAFT_WEBUI_DIR); skipped");
            None
        }
        Err(_) => panic!("bicraft did not start"),
    }
}

/// Headless Firefox on `url`, with a throwaway profile.
fn browser(exe: &Path, profile: &Path, url: &str) -> Kill {
    std::fs::create_dir_all(profile).unwrap();
    std::fs::write(
        profile.join("user.js"),
        [
            r#"user_pref("browser.shell.checkDefaultBrowser", false);"#,
            r#"user_pref("browser.aboutwelcome.enabled", false);"#,
            r#"user_pref("datareporting.policy.dataSubmissionEnabled", false);"#,
            r#"user_pref("toolkit.telemetry.reportingpolicy.firstRun", false);"#,
            r#"user_pref("app.update.disabledForTesting", true);"#,
            r#"user_pref("browser.startup.homepage_override.mstone", "ignore");"#,
            "",
        ]
        .join("\n"),
    )
    .unwrap();
    let child = Command::new(exe)
        .args([
            "-headless",
            "-no-remote",
            "-new-instance",
            "--window-size=1280,800",
            "-profile",
        ])
        .arg(profile)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    Kill(child)
}

/// `bicraft-cli ui ARGS`: the exit code and the reply.
fn ui(args: &[&str]) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_bicraft-cli"))
        .arg("ui")
        .args(args)
        .env_remove("BICRAFT_CONTROL_TOKEN")
        .env_remove("BICRAFT_CONTROL_FILE")
        .env_remove("BICRAFT_SERVER_TOKEN")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let v = serde_json::from_str(&text).unwrap_or(Value::Null);
    if v.is_null() {
        eprintln!(
            "bicraft-cli ui {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    (out.status.code().unwrap_or(-1), v)
}

/// A driver of one page through `bicraft-cli ui` with the target options `base`.
struct Agent {
    base: Vec<String>,
    shots: PathBuf,
}

impl Agent {
    fn call(&self, method: &str, params: Value) -> Value {
        let p = params.to_string();
        let mut args: Vec<&str> = self.base.iter().map(String::as_str).collect();
        args.extend([method, p.as_str()]);
        let (code, v) = ui(&args);
        eprintln!("{method} {p} -> {}", truncate(&v));
        assert_eq!(code, 0, "{method} {p}: {v}");
        assert_eq!(v["ok"], true, "{method} {p}: {v}");
        v["result"].clone()
    }

    fn fails(&self, method: &str, params: Value) -> String {
        let p = params.to_string();
        let mut args: Vec<&str> = self.base.iter().map(String::as_str).collect();
        args.extend([method, p.as_str()]);
        let (code, v) = ui(&args);
        assert_eq!((code, &v["ok"]), (1, &json!(false)), "{method} {p}: {v}");
        v["error"].as_str().unwrap_or_default().to_string()
    }

    /// Waits until the page is connected and its window idle (the wasm loads first).
    fn wait_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut args: Vec<&str> = self.base.iter().map(String::as_str).collect();
        args.extend(["ui.wait", r#"{"timeout": 50}"#]);
        loop {
            let (code, v) = ui(&args);
            if code == 0 && v["ok"] == true {
                return;
            }
            assert!(Instant::now() < deadline, "the page never connected: {v}");
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    /// `ui.screenshot` saved as `name`: a PNG of the canvas, at least `min` pixels wide.
    fn shot(&self, name: &str) -> PathBuf {
        let path = self.shots.join(name);
        let out = path.to_string_lossy().into_owned();
        let mut args: Vec<&str> = self.base.iter().map(String::as_str).collect();
        args.extend(["ui.screenshot", "{}", "--out", out.as_str()]);
        let (code, v) = ui(&args);
        assert_eq!(code, 0, "ui.screenshot: {v}");
        let png = std::fs::read(&path).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "a PNG");
        assert!(v["result"]["width"].as_u64().unwrap() >= 640, "{v}");
        assert!(v["result"]["height"].as_u64().unwrap() >= 400, "{v}");
        eprintln!("screenshot: {} ({} bytes)", path.display(), png.len());
        path
    }
}

fn truncate(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 300 {
        format!("{}…", &s[..300])
    } else {
        s
    }
}

fn shots_dir(tmp: &TempDir, sub: &str) -> PathBuf {
    let dir = match std::env::var_os("BICRAFT_BROWSER_SHOTS") {
        Some(d) => PathBuf::from(d).join(sub),
        None => tmp.path().join("shots"),
    };
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn page_names(inspect: &Value) -> Vec<String> {
    inspect["pages"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| p.as_str().or_else(|| p["name"].as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// `bicraft-cli view --control`: the page connects to the relay, takes the requests and answers them.
#[test]
#[ignore = "needs Firefox and the embedded browser app: BICRAFT_FIREFOX (module docs)"]
fn bicraft_view_in_firefox() {
    let Some(exe) = firefox() else {
        eprintln!("note: BICRAFT_FIREFOX is not set; skipped");
        return;
    };
    let tmp = TempDir::new("browser-view");
    let file = tmp.path().join("control-view.json");
    let port = free_port().to_string();
    let pbip = fixture().join("visuals.pbip");
    let Some(_server) = spawn(
        &[
            "view",
            pbip.to_str().unwrap(),
            "--ui",
            "egui",
            "--port",
            &port,
            "--control",
            "--control-file",
            file.to_str().unwrap(),
        ],
        "UI control channel at",
    ) else {
        return;
    };
    let _ff = browser(
        &exe,
        &tmp.path().join("profile"),
        &format!("http://127.0.0.1:{port}/app/"),
    );
    let agent = Agent {
        base: vec!["--control".into(), file.to_string_lossy().into_owned()],
        shots: shots_dir(&tmp, "view"),
    };
    agent.wait_ready();
    let s = agent.call("ui.inspect", json!({}));
    let pages = page_names(&s);
    assert!(pages.len() >= 2, "the fixture's pages: {s}");
    agent.shot("view-1-report.png");
    // a page tab, as its click does (view.interact)
    let second = pages[1].clone();
    let s = agent.call("ui.set", json!({"page": second}));
    assert!(s.to_string().contains(&second), "{s}");
    agent.call("ui.wait", json!({}));
    agent.shot("view-2-page.png");
    // a command: bicraft-cli view runs no engine command, and the page says so
    let why = agent.fails(
        "command",
        json!({"command": "page.add", "params": {"display_name": "X"}}),
    );
    assert!(!why.is_empty());
    // the palette and a form
    let p = agent.call("ui.palette", json!({"open": true, "query": "measure"}));
    assert!(p.to_string().contains("measure"), "{p}");
    agent.shot("view-3-palette.png");
    agent.call("ui.close", json!({"what": "palette"}));
    agent.call("ui.set", json!({"view": "model"}));
    agent.call("ui.wait", json!({}));
    agent.shot("view-4-model.png");
}

/// `bicraft-cli serve --viewer-control`: the workspace's app page opens a file in a document tab,
/// runs a command on it, switches page and tab, opens the palette and a form; the agent lists its
/// views and addresses the document view.
#[test]
#[ignore = "needs Firefox and the embedded browser app: BICRAFT_FIREFOX (module docs)"]
fn bicraft_serve_in_firefox() {
    let Some(exe) = firefox() else {
        eprintln!("note: BICRAFT_FIREFOX is not set; skipped");
        return;
    };
    let tmp = TempDir::new("browser-serve");
    let root = tmp.path().join("root");
    copy_dir(&fixture(), &root.join("ws1"));
    let file = tmp.path().join("control-serve.json");
    let port = free_port();
    let listen = format!("127.0.0.1:{port}");
    let origin = format!("http://127.0.0.1:{port}");
    let Some(_server) = spawn(
        &[
            "serve",
            "--root",
            root.to_str().unwrap(),
            "--listen",
            &listen,
            "--allow-origin",
            &origin,
            "--viewer-control",
            "--viewer-control-file",
            file.to_str().unwrap(),
            "--drain-on-stdin-close",
        ],
        "UI control channel relayed",
    ) else {
        return;
    };
    let token: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(token["kind"], "serve");
    let _ff = browser(
        &exe,
        &tmp.path().join("profile"),
        &format!("{origin}/w/ws1/view/app/"),
    );
    let agent = Agent {
        base: vec![
            "--control".into(),
            file.to_string_lossy().into_owned(),
            "--workspace".into(),
            "ws1".into(),
        ],
        shots: shots_dir(&tmp, "serve"),
    };
    agent.wait_ready();
    agent.shot("serve-1-start.png");
    // open a workspace file: a document tab of its own (a document view on the server)
    agent.call("app.open", json!({"path": "visuals.pbip"}));
    agent.call("ui.wait", json!({}));
    let s = agent.call("ui.inspect", json!({}));
    let pages = page_names(&s);
    assert!(pages.len() >= 2, "{s}");
    agent.shot("serve-2-open.png");
    // a command on that document's own session, as its button runs it
    agent.call(
        "command",
        json!({"command": "page.add", "params": {"display_name": "From the agent"}}),
    );
    agent.call("ui.wait", json!({}));
    let s = agent.call("ui.set", json!({"page": "From the agent"}));
    assert!(
        page_names(&s).iter().any(|p| p.contains("From the agent"))
            || s.to_string().contains("From the agent"),
        "{s}"
    );
    agent.shot("serve-3-command.png");
    // the agent's views: the page and the document view it opened
    let (code, views) = ui(&[
        "--control",
        file.to_str().unwrap(),
        "--workspace",
        "ws1",
        "server.views",
    ]);
    assert_eq!(code, 0, "{views}");
    let page = views["result"]["pages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["connected"] == true)
        .unwrap_or_else(|| panic!("a connected page: {views}"))
        .clone();
    let doc = page["docs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["file"] == "visuals.pbip")
        .unwrap_or_else(|| panic!("the document view: {views}"))["view"]
        .as_str()
        .unwrap()
        .to_string();
    // switch to the page's own tab, then address the document view: its tab comes back
    agent.call("ui.set", json!({"doc": 0}));
    let by_view = Agent {
        base: [agent.base.clone(), vec!["--view-id".into(), doc]].concat(),
        shots: agent.shots.clone(),
    };
    let s = by_view.call("ui.inspect", json!({}));
    assert!(s.to_string().contains("visuals.pbip"), "{s}");
    // the palette, a form, the ribbon's Model tab
    let p = agent.call("ui.palette", json!({"open": true, "query": "measure"}));
    assert!(p.to_string().contains("measure"), "{p}");
    agent.shot("serve-4-palette.png");
    agent.call("ui.close", json!({"what": "palette"}));
    agent.call("ui.form.open", json!({"command": "measure.add"}));
    agent.shot("serve-5-form.png");
    agent.call("ui.close", json!({"what": "form"}));
    agent.call("ui.set", json!({"view": "model"}));
    agent.call("ui.wait", json!({}));
    agent.shot("serve-6-model.png");
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), dst).unwrap();
        }
    }
}
