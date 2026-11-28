//! Checks run from an extracted release archive: `--version` of both programs, a model generated
//! and validated, the desktop app's offscreen render of it, an MCP `initialize` + `tools/list`
//! over stdio, the install script into a scratch folder, and the `.pbiviz` visuals.
//!
//! The archive is extracted with the operating system's own `tar` (Windows' bsdtar reads zip),
//! as a user would, not with the library that wrote it.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use super::Target;

/// How long the MCP server may take to answer one request.
const MCP_TIMEOUT: Duration = Duration::from_secs(60);

pub fn run(t: Target, archive: &Path, out: &Path, top: &str, version: &str) -> Result<(), String> {
    let scratch = out.join(format!(".smoke-{}", t.label()));
    if scratch.exists() {
        std::fs::remove_dir_all(&scratch).map_err(|e| format!("smoke: {}: {e}", scratch.display()))?;
    }
    std::fs::create_dir_all(&scratch).map_err(|e| format!("smoke: {}: {e}", scratch.display()))?;
    let result = checks(t, archive, &scratch, top, version);
    match &result {
        Ok(()) => {
            let _ = std::fs::remove_dir_all(&scratch);
            eprintln!("smoke ({}): all checks passed", t.label());
        }
        Err(_) => eprintln!("smoke ({}): scratch kept in {}", t.label(), scratch.display()),
    }
    result
}

fn checks(t: Target, archive: &Path, scratch: &Path, top: &str, version: &str) -> Result<(), String> {
    let mut tar = Command::new(tar_program());
    tar.arg(if t == Target::Linux { "-xzf" } else { "-xf" })
        .arg(archive)
        .arg("-C")
        .arg(scratch);
    expect_ok(tar, "extract")?;
    let dir = scratch.join(top);
    let bin = dir.join(t.bin_dir());
    let cli = bin.join(t.cli());
    let app = bin.join(t.app());

    // 1. --version of both programs
    let line = stdout_of(Command::new(&cli).arg("--version"), "bicraft-cli --version")?;
    if !line.starts_with(&format!("bicraft-cli {version}")) {
        return Err(format!("smoke: bicraft-cli --version printed `{line}`"));
    }
    eprintln!("smoke: {line}");
    let app_line = stdout_of(Command::new(&app).arg("--version"), "bicraft --version")?;
    if app_line != format!("bicraft {version}") {
        return Err(format!("smoke: bicraft --version printed `{app_line}`"));
    }
    eprintln!("smoke: {app_line}");

    // 2. a generated project, validated.
    let proj = scratch.join("project");
    let mut c = Command::new(&cli);
    c.arg("new-project").arg(&proj).arg("Smoke");
    stdout_of(&mut c, "new-project")?;
    let model = proj.join("Smoke.SemanticModel");
    let report = stdout_of(
        Command::new(&cli).arg("validate-model").arg(&model),
        "validate-model",
    )?;
    let v: serde_json::Value =
        serde_json::from_str(&report).map_err(|e| format!("smoke: validate-model output: {e}"))?;
    if v.get("valid") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("smoke: validate-model: {report}"));
    }
    eprintln!("smoke: validate-model ok");

    // 3. the desktop app opens that project and draws its window offscreen (no display, no GPU)
    let shot = scratch.join("app.png");
    stdout_of(
        Command::new(&app)
            .arg(proj.join("Smoke.pbip"))
            .arg("--render")
            .arg(&shot)
            .args(["--size", "800x500"]),
        "bicraft --render",
    )?;
    let png = std::fs::read(&shot).map_err(|e| format!("smoke: {}: {e}", shot.display()))?;
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(format!("smoke: {} is not a PNG", shot.display()));
    }
    eprintln!("smoke: bicraft --render ok ({} bytes)", png.len());

    // 4. MCP handshake over stdio.
    mcp(&cli, version)?;

    // 5. The install script, into a scratch folder (a prefix on Linux).
    let dest = scratch.join("installed");
    let mut c = match t {
        Target::Windows => {
            let mut c = Command::new("powershell.exe");
            c.args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(dir.join("install.ps1"))
            .arg("-Destination")
            .arg(&dest);
            c
        }
        Target::Linux => {
            let mut c = Command::new("sh");
            c.arg(dir.join("install.sh")).arg(&dest);
            c
        }
    };
    let said = stdout_of(&mut c, "install script")?;
    let installed_bin = dest.join(t.bin_dir());
    let installed = stdout_of(
        Command::new(installed_bin.join(t.cli())).arg("--version"),
        "installed bicraft-cli --version",
    )?;
    let installed_app = stdout_of(
        Command::new(installed_bin.join(t.app())).arg("--version"),
        "installed bicraft --version",
    )?;
    if installed != line || installed_app != app_line || !said.contains(&line) {
        return Err(format!("smoke: install script output:\n{said}"));
    }
    if t == Target::Linux {
        // the menu entry runs the installed app by its absolute path; the icon and types are there
        let entry = dest.join(format!("share/applications/{}.desktop", super::APP_ID));
        let text = std::fs::read_to_string(&entry).map_err(|e| format!("smoke: {}: {e}", entry.display()))?;
        let exec = format!("Exec=\"{}\" %f", installed_bin.join(t.app()).display());
        if !text.lines().any(|l| l == exec) {
            return Err(format!("smoke: {} has no `{exec}`:\n{text}", entry.display()));
        }
        for f in [
            format!("share/icons/hicolor/256x256/apps/{}.png", super::APP_ID),
            format!("share/mime/packages/{}.xml", super::APP_ID),
            format!("share/metainfo/{}.metainfo.xml", super::APP_ID),
            "share/doc/bicraft/README.md".to_owned(),
            "share/bicraft/visuals/README.md".to_owned(),
        ] {
            if !dest.join(&f).is_file() {
                return Err(format!("smoke: the install script did not install {f}"));
            }
        }
    }
    eprintln!("smoke: install script ok");

    // 6. BIcraft's visuals (M12.5): `.pbiviz` packages next to their README.
    let visuals = dir.join(t.visuals_dir());
    let packages: Vec<PathBuf> = std::fs::read_dir(&visuals)
        .map_err(|e| format!("smoke: {}: {e}", visuals.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "pbiviz"))
        .collect();
    if packages.is_empty() || !visuals.join("README.md").is_file() {
        return Err(format!(
            "smoke: {} has no .pbiviz or no README.md",
            visuals.display()
        ));
    }
    for p in &packages {
        let head = std::fs::read(p).map_err(|e| format!("smoke: {}: {e}", p.display()))?;
        if !head.starts_with(b"PK") {
            return Err(format!("smoke: {} is not a .pbiviz package", p.display()));
        }
    }
    eprintln!("smoke: {} visuals ok", packages.len());
    Ok(())
}

fn tar_program() -> PathBuf {
    if cfg!(windows) {
        // System32's bsdtar extracts zip; a Git-for-Windows GNU tar earlier on PATH would not.
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        PathBuf::from(root).join("System32").join("tar.exe")
    } else {
        PathBuf::from("tar")
    }
}

fn expect_ok(mut c: Command, what: &str) -> Result<(), String> {
    let o = c.output().map_err(|e| format!("smoke: {what}: {e}"))?;
    if o.status.success() {
        Ok(())
    } else {
        Err(format!(
            "smoke: {what} failed ({}): {}",
            o.status,
            String::from_utf8_lossy(&o.stderr)
        ))
    }
}

fn stdout_of(c: &mut Command, what: &str) -> Result<String, String> {
    let o = c.output().map_err(|e| format!("smoke: {what}: {e}"))?;
    if !o.status.success() {
        return Err(format!(
            "smoke: {what} failed ({}): {}{}",
            o.status,
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

/// `initialize` then `tools/list`; the server must exit when stdin closes.
fn mcp(bin: &Path, version: &str) -> Result<(), String> {
    let mut child = Command::new(bin)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("smoke: mcp: {e}"))?;
    let result = mcp_session(&mut child, version);
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|e| format!("smoke: mcp: {e}"))?;
    result?;
    if !status.success() {
        return Err(format!("smoke: mcp exited with {status}"));
    }
    eprintln!("smoke: mcp handshake ok");
    Ok(())
}

fn mcp_session(child: &mut std::process::Child, version: &str) -> Result<(), String> {
    let mut stdin = child.stdin.take().ok_or("smoke: mcp: no stdin")?;
    let stdout = child.stdout.take().ok_or("smoke: mcp: no stdout")?;
    // Lines are read on a thread so a hung server fails the check instead of blocking it.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let send = |stdin: &mut std::process::ChildStdin, msg: serde_json::Value| {
        writeln!(stdin, "{msg}")
            .and_then(|()| stdin.flush())
            .map_err(|e| format!("smoke: mcp write: {e}"))
    };
    let ask =
        |stdin: &mut std::process::ChildStdin, req: serde_json::Value| -> Result<serde_json::Value, String> {
            send(stdin, req)?;
            let line = rx
                .recv_timeout(MCP_TIMEOUT)
                .map_err(|_| "smoke: mcp: no answer".to_owned())?
                .map_err(|e| format!("smoke: mcp read: {e}"))?;
            serde_json::from_str(&line).map_err(|e| format!("smoke: mcp: bad JSON `{line}`: {e}"))
        };
    let init = ask(
        &mut stdin,
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "xtask-dist", "version": "1"}}}),
    )?;
    let info = init.pointer("/result/serverInfo");
    if info
        .and_then(|i| i.get("name"))
        .and_then(serde_json::Value::as_str)
        != Some("bicraft")
        || info
            .and_then(|i| i.get("version"))
            .and_then(serde_json::Value::as_str)
            != Some(version)
    {
        return Err(format!("smoke: mcp initialize answered {init}"));
    }
    send(
        &mut stdin,
        serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )?;
    let tools = ask(
        &mut stdin,
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )?;
    let n = tools
        .pointer("/result/tools")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    if n == 0 {
        return Err(format!("smoke: mcp tools/list answered {tools}"));
    }
    eprintln!("smoke: mcp lists {n} tools");
    // Closing stdin ends the server.
    drop(stdin);
    Ok(())
}
