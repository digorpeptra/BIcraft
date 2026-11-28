//! `bicraft-cli setup`: every test runs against a temporary configuration root (`--config-root` or
//! `BICRAFT_SETUP_HOME`), never the real home directory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;
use tempdir::TempDir;

const ALL: [&str; 5] = ["claude_code", "claude_desktop", "vscode", "cursor", "codex"];

/// A fresh, empty directory under the system temp dir, removed when the guard drops.
fn root(name: &str) -> TempDir {
    TempDir::new(&format!("setup-{name}"))
}

/// The binary, with every variable that could point setup at real files removed.
fn bicraft() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_bicraft-cli"));
    for v in ["BICRAFT_SETUP_HOME", "CLAUDE_CONFIG_DIR", "CODEX_HOME"] {
        c.env_remove(v);
    }
    c.stdin(Stdio::null());
    c
}

struct Out {
    code: i32,
    json: Value,
    stdout: String,
}

/// `bicraft-cli setup --config-root ROOT --platform PLATFORM ARGS... --json`.
fn setup_on(root: &Path, platform: &str, args: &[&str]) -> Out {
    let mut cmd = bicraft();
    cmd.arg("setup");
    if args.first() == Some(&"doctor") || args.first() == Some(&"uninstall") {
        cmd.arg(args[0]);
    }
    let rest = if args.first() == Some(&"doctor") || args.first() == Some(&"uninstall") {
        &args[1..]
    } else {
        args
    };
    cmd.arg("--config-root")
        .arg(root)
        .args(["--platform", platform])
        .args(rest);
    run(cmd)
}

fn run(mut cmd: Command) -> Out {
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    Out {
        code: out.status.code().unwrap(),
        json: serde_json::from_str(&stdout).unwrap_or(Value::Null),
        stdout,
    }
}

fn setup(root: &Path, args: &[&str]) -> Out {
    let mut a = args.to_vec();
    a.push("--json");
    setup_on(root, "windows", &a)
}

fn clients_args(ids: &[&'static str]) -> Vec<&'static str> {
    ids.iter().flat_map(|id| ["--client", id]).collect()
}

/// Every file under `dir`, with its bytes.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.insert(
                    p.strip_prefix(dir).unwrap().to_path_buf(),
                    std::fs::read(&p).unwrap(),
                );
            }
        }
    }
    out
}

/// The configuration path of `client` under `root` for `platform`.
fn path(root: &Path, platform: &str, client: &str) -> PathBuf {
    let app = match platform {
        "windows" => root.join("AppData").join("Roaming"),
        "macos" => root.join("Library").join("Application Support"),
        _ => root.join(".config"),
    };
    match client {
        "claude_code" => root.join(".claude.json"),
        "claude_desktop" => app.join("Claude").join("claude_desktop_config.json"),
        "vscode" => app.join("Code").join("User").join("mcp.json"),
        "cursor" => root.join(".cursor").join("mcp.json"),
        "codex" => root.join(".codex").join("config.toml"),
        _ => unreachable!(),
    }
}

fn write(p: &Path, text: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

fn read_json(p: &Path) -> Value {
    serde_json::from_str(&read(p)).unwrap()
}

fn backup(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_os_string();
    s.push(".bicraft.bak");
    PathBuf::from(s)
}

fn toml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn client<'a>(out: &'a Out, id: &str) -> &'a Value {
    out.json["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["client"] == id)
        .unwrap()
}

fn command(out: &Out) -> String {
    out.json["server"]["command"].as_str().unwrap().to_string()
}

#[test]
fn fresh_install_on_every_layout() {
    for platform in ["windows", "linux", "macos"] {
        let r = root(&format!("fresh-{platform}"));
        let mut args = clients_args(&ALL);
        args.extend(["--yes", "--json"]);
        let out = setup_on(&r, platform, &args);
        assert_eq!(out.code, 0, "{}", out.stdout);
        assert_eq!(out.json["status"], "applied");
        assert_eq!(out.json["platform"], platform);
        let cmd = command(&out);
        assert!(Path::new(&cmd).is_file(), "{cmd}");
        for id in ALL {
            let c = client(&out, id);
            assert_eq!(c["action"], "add", "{id}");
            assert_eq!(c["result"], "written", "{id}");
            assert_eq!(c["backup"], Value::Null, "{id}: a new file has no backup");
            assert_eq!(PathBuf::from(c["path"].as_str().unwrap()), path(&r, platform, id));
        }
        let entry = |id: &str, key: &str| read_json(&path(&r, platform, id))[key]["bicraft"].clone();
        assert_eq!(
            entry("claude_code", "mcpServers"),
            json!({"type": "stdio", "command": cmd, "args": ["mcp"]})
        );
        assert_eq!(
            entry("claude_desktop", "mcpServers"),
            json!({"command": cmd, "args": ["mcp"]})
        );
        assert_eq!(
            entry("vscode", "servers"),
            json!({"type": "stdio", "command": cmd, "args": ["mcp"]})
        );
        assert_eq!(
            entry("cursor", "mcpServers"),
            json!({"type": "stdio", "command": cmd, "args": ["mcp"]})
        );
        assert_eq!(
            read(&path(&r, platform, "codex")),
            format!(
                "[mcp_servers.bicraft]\ncommand = {}\nargs = [\"mcp\"]\n",
                toml_str(&cmd)
            )
        );
        let restart = out.json["restart"].as_array().unwrap();
        assert_eq!(restart.len(), 5);
        let _ = std::fs::remove_dir_all(&r);
    }
}

#[test]
fn second_run_changes_nothing() {
    let r = root("idempotent");
    let mut args = clients_args(&ALL);
    args.push("--yes");
    assert_eq!(setup(&r, &args).json["status"], "applied");
    let before = snapshot(&r);
    let out = setup(&r, &args);
    assert_eq!(out.code, 0);
    assert_eq!(out.json["status"], "unchanged");
    for id in ALL {
        assert_eq!(client(&out, id)["action"], "unchanged");
        assert_eq!(client(&out, id)["registration"], "aligned");
    }
    assert_eq!(snapshot(&r), before, "no file written, no backup made");
}

#[test]
fn stale_path_is_updated_keeping_everything_else() {
    let r = root("stale");
    let old = if cfg!(windows) {
        r"C:\old\bicraft.exe"
    } else {
        "/old/bicraft"
    };
    let old_json = serde_json::to_string(old).unwrap();
    // Claude Code: 4-space indent, unknown keys around, the entry's env kept.
    let cc = format!(
        "{{\n    \"zeta\": 1,\n    \"mcpServers\": {{\n        \"other\": {{\"command\": \"other\", \"args\": []}},\n        \
         \"bicraft\": {{\"command\": {old_json}, \"args\": [\"mcp\"], \"env\": {{\"X\": \"1\"}}}},\n        \
         \"later\": {{\"command\": \"z\"}}\n    }},\n    \"alpha\": [1, 2.5, \"x\"]\n}}\n"
    );
    write(&path(&r, "windows", "claude_code"), &cc);
    // VS Code: CRLF line endings, tab indent, `inputs` kept.
    let vs = format!(
        "{{\r\n\t\"inputs\": [],\r\n\t\"servers\": {{\r\n\t\t\"bicraft\": {{\r\n\t\t\t\"type\": \"stdio\",\r\n\t\t\t\
         \"command\": {old_json},\r\n\t\t\t\"args\": [\"mcp\"]\r\n\t\t}}\r\n\t}}\r\n}}"
    );
    write(&path(&r, "windows", "vscode"), &vs);
    // Codex: comments, other servers, the entry's env sub-table and extra keys.
    let codex = format!(
        "# Codex settings\nmodel = \"gpt-5\"\n\n[mcp_servers.bicraft]\ncommand = {} # the binary\n\
         startup_timeout_sec = 30\nargs = [\n  \"old\",\n]\n\n[mcp_servers.bicraft.env]\nX = \"1\"\n\n\
         # another server\n[mcp_servers.other]\ncommand = \"other\"\n",
        toml_str(old)
    );
    write(&path(&r, "windows", "codex"), &codex);

    let mut args = clients_args(&["claude_code", "vscode", "codex"]);
    args.push("--yes");
    let out = setup(&r, &args);
    assert_eq!(out.code, 0, "{}", out.stdout);
    assert_eq!(out.json["status"], "applied");
    let cmd = command(&out);
    for id in ["claude_code", "vscode", "codex"] {
        let c = client(&out, id);
        assert_eq!(c["action"], "update", "{id}");
        assert_eq!(c["registration"], "stale", "{id}");
        assert_eq!(
            PathBuf::from(c["backup"].as_str().unwrap()),
            backup(&path(&r, "windows", id))
        );
    }
    assert_eq!(client(&out, "claude_code")["registered_command"], old);
    assert_eq!(read(&backup(&path(&r, "windows", "claude_code"))), cc);
    assert_eq!(read(&backup(&path(&r, "windows", "vscode"))), vs);
    assert_eq!(read(&backup(&path(&r, "windows", "codex"))), codex);

    let cc_new = read(&path(&r, "windows", "claude_code"));
    let cmd_json = serde_json::to_string(&cmd).unwrap();
    assert_eq!(
        cc_new,
        format!(
            "{{\n    \"zeta\": 1,\n    \"mcpServers\": {{\n        \"other\": {{\n            \"command\": \"other\",\n            \
             \"args\": []\n        }},\n        \"bicraft\": {{\n            \"command\": {cmd_json},\n            \
             \"args\": [\n                \"mcp\"\n            ],\n            \"env\": {{\n                \"X\": \"1\"\n            \
             }}\n        }},\n        \"later\": {{\n            \"command\": \"z\"\n        }}\n    }},\n    \"alpha\": [\n        1,\n        \
             2.5,\n        \"x\"\n    ]\n}}\n"
        )
    );
    let vs_new = read(&path(&r, "windows", "vscode"));
    assert!(vs_new.contains("\r\n\t\t\t\"command\": "), "{vs_new:?}");
    assert!(!vs_new.replace("\r\n", "").contains('\n'), "CRLF kept");
    assert!(!vs_new.ends_with('\n'), "no final newline added");
    let v: Value = serde_json::from_str(&vs_new).unwrap();
    assert_eq!(v["inputs"], json!([]));
    assert_eq!(
        v["servers"]["bicraft"],
        json!({"type": "stdio", "command": cmd, "args": ["mcp"]})
    );

    assert_eq!(
        read(&path(&r, "windows", "codex")),
        format!(
            "# Codex settings\nmodel = \"gpt-5\"\n\n[mcp_servers.bicraft]\ncommand = {} # the binary\n\
             startup_timeout_sec = 30\nargs = [\"mcp\"]\n\n[mcp_servers.bicraft.env]\nX = \"1\"\n\n\
             # another server\n[mcp_servers.other]\ncommand = \"other\"\n",
            toml_str(&cmd)
        )
    );
    // And the update is idempotent.
    assert_eq!(setup(&r, &args).json["status"], "unchanged");
}

#[test]
fn malformed_files_block_the_run_and_stay_untouched() {
    let cases: [(&str, &str); 9] = [
        ("cursor", "{\"mcpServers\": {"),
        ("cursor", "[1, 2]"),
        ("cursor", "{\"mcpServers\": []}"),
        ("cursor", "{\"mcpServers\": {\"bicraft\": \"x\"}}"),
        ("vscode", "{\n  // my servers\n  \"servers\": {}\n}\n"),
        ("claude_desktop", "\u{0}\u{1}not json"),
        ("codex", "[mcp_servers.bicraft\ncommand = \"x\"\n"),
        ("codex", "model = \"unterminated\n"),
        (
            "codex",
            "[mcp_servers]\nbicraft = { command = \"/old\", args = [\"mcp\"] }\n",
        ),
    ];
    for (i, (id, text)) in cases.iter().enumerate() {
        let r = root(&format!("malformed-{i}"));
        let p = path(&r, "windows", id);
        write(&p, text);
        // A valid client alongside: the whole run is blocked, so it is not written either.
        let other = if *id == "claude_code" {
            "cursor"
        } else {
            "claude_code"
        };
        let mut args = clients_args(&[id, other]);
        args.push("--yes");
        let out = setup(&r, &args);
        assert_eq!(out.code, 1, "case {i}: {}", out.stdout);
        assert_eq!(out.json["status"], "blocked", "case {i}");
        assert_eq!(client(&out, id)["action"], "invalid", "case {i}");
        assert!(client(&out, id)["error"].as_str().unwrap().len() > 5, "case {i}");
        assert_eq!(read(&p), *text, "case {i}: the malformed file is untouched");
        assert!(!backup(&p).exists(), "case {i}");
        assert!(!path(&r, "windows", other).exists(), "case {i}: nothing written");
        let _ = std::fs::remove_dir_all(&r);
    }
}

#[test]
fn jsonc_comments_are_reported_with_a_hint() {
    let r = root("jsonc");
    write(
        &path(&r, "windows", "vscode"),
        "{\n  // comment\n  \"servers\": {}\n}\n",
    );
    let out = setup(&r, &["--client", "vscode", "--dry-run"]);
    assert_eq!(out.code, 1);
    let err = client(&out, "vscode")["error"].as_str().unwrap();
    assert!(err.contains("comments"), "{err}");
}

#[test]
fn dry_run_writes_nothing() {
    let r = root("dry");
    write(
        &path(&r, "windows", "cursor"),
        "{\"mcpServers\": {\"bicraft\": {\"command\": \"/old\", \"args\": [\"mcp\"]}}}",
    );
    write(&path(&r, "windows", "codex"), "model = \"x\"\n");
    let before = snapshot(&r);
    let mut args = clients_args(&ALL);
    args.extend(["--dry-run", "--yes"]);
    let out = setup(&r, &args);
    assert_eq!(out.code, 0);
    assert_eq!(out.json["status"], "planned");
    assert_eq!(out.json["dry_run"], true);
    assert_eq!(client(&out, "cursor")["action"], "update");
    assert_eq!(client(&out, "codex")["action"], "add");
    assert_eq!(client(&out, "vscode")["action"], "add");
    assert_eq!(client(&out, "vscode")["result"], Value::Null);
    assert_eq!(snapshot(&r), before);

    // The human plan too.
    let out = setup_on(&r, "windows", &["--client", "cursor", "--dry-run"]);
    assert_eq!(out.code, 0);
    assert!(out.stdout.contains("cursor          update"), "{}", out.stdout);
    assert!(out.stdout.contains("was: /old"), "{}", out.stdout);
    assert!(
        out.stdout.contains("Dry run: nothing was written."),
        "{}",
        out.stdout
    );
    assert_eq!(snapshot(&r), before);
}

#[test]
fn without_yes_and_without_a_terminal_nothing_is_applied() {
    let r = root("confirm");
    let out = setup(&r, &["--client", "cursor"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.json["status"], "needs_confirmation");
    assert!(snapshot(&r).is_empty());
}

#[test]
fn uninstall_removes_only_the_bicraft_entry() {
    let r = root("uninstall");
    write(
        &path(&r, "windows", "cursor"),
        "{\n  \"mcpServers\": {\n    \"a\": {\"command\": \"a\"},\n    \"z\": {\"command\": \"z\"}\n  },\n  \"k\": true\n}\n",
    );
    write(
        &path(&r, "windows", "codex"),
        "model = \"x\"\n\n[mcp_servers.a]\ncommand = \"a\"\n\n# keep me\n[profiles.p]\nmodel = \"y\"\n",
    );
    let cursor_before = read_json(&path(&r, "windows", "cursor"));
    let codex_before = read(&path(&r, "windows", "codex"));
    let mut args = clients_args(&ALL);
    args.push("--yes");
    assert_eq!(setup(&r, &args).json["status"], "applied");
    // Give bicraft's Codex table an env sub-table, as a user might.
    let codex = path(&r, "windows", "codex");
    let with_env = format!("{}\n[mcp_servers.bicraft.env]\nTOKEN = \"t\"\n", read(&codex));
    write(&codex, &with_env);

    // No --client: every client where bicraft is registered.
    let out = setup(&r, &["--uninstall", "--yes"]);
    assert_eq!(out.code, 0, "{}", out.stdout);
    assert_eq!(out.json["status"], "applied");
    assert_eq!(out.json["operation"], "uninstall");
    for id in ALL {
        assert_eq!(client(&out, id)["action"], "remove", "{id}");
        assert_eq!(client(&out, id)["result"], "written", "{id}");
    }
    assert_eq!(read_json(&path(&r, "windows", "cursor")), cursor_before);
    assert_eq!(read(&codex), codex_before);
    assert_eq!(read(&backup(&codex)), with_env);
    assert_eq!(
        read_json(&path(&r, "windows", "claude_code")),
        json!({"mcpServers": {}})
    );
    assert_eq!(read_json(&path(&r, "windows", "vscode")), json!({"servers": {}}));

    // Nothing left to remove.
    let before = snapshot(&r);
    let out = setup_on(&r, "windows", &["uninstall", "--yes", "--json"]);
    assert_eq!(out.code, 0);
    assert_eq!(out.json["status"], "nothing_selected");
    let mut args = clients_args(&ALL);
    args.extend(["--uninstall", "--yes"]);
    let out = setup(&r, &args);
    assert_eq!(out.json["status"], "unchanged");
    assert_eq!(client(&out, "cursor")["action"], "none");
    assert_eq!(snapshot(&r), before);
}

#[test]
fn without_client_flags_only_registered_clients_are_refreshed() {
    let r = root("default");
    write(
        &path(&r, "windows", "cursor"),
        "{\"mcpServers\": {\"bicraft\": {\"command\": \"/old\", \"args\": [\"mcp\"]}}}",
    );
    std::fs::create_dir_all(path(&r, "windows", "vscode").parent().unwrap()).unwrap();
    let out = setup(&r, &["--yes"]);
    assert_eq!(out.code, 0, "{}", out.stdout);
    assert_eq!(out.json["status"], "applied");
    assert_eq!(client(&out, "cursor")["action"], "update");
    assert_eq!(client(&out, "vscode")["detected"], true);
    assert_eq!(client(&out, "vscode")["selected"], false);
    assert_eq!(client(&out, "vscode")["action"], "skip");
    assert!(!path(&r, "windows", "vscode").exists());

    // --all-detected adds the detected ones.
    let out = setup(&r, &["--all-detected", "--yes"]);
    assert_eq!(out.json["status"], "applied");
    assert_eq!(client(&out, "vscode")["action"], "add");
    assert_eq!(client(&out, "cursor")["action"], "unchanged");
    assert_eq!(client(&out, "codex")["selected"], false, "not detected");

    // Nothing registered and nothing chosen.
    let r = root("default-none");
    let out = setup(&r, &["--yes"]);
    assert_eq!(out.code, 0);
    assert_eq!(out.json["status"], "nothing_selected");
    assert!(snapshot(&r).is_empty());
}

#[test]
fn project_scope_writes_into_the_project() {
    let r = root("project");
    let proj = r.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    let mut args = clients_args(&["claude_code", "vscode", "cursor", "codex"]);
    let proj_s = proj.to_str().unwrap();
    args.extend(["--scope", "project", "--project-dir", proj_s, "--yes"]);
    let out = setup(&r, &args);
    assert_eq!(out.code, 0, "{}", out.stdout);
    assert_eq!(out.json["scope"], "project");
    let cmd = command(&out);
    assert_eq!(
        read_json(&proj.join(".mcp.json"))["mcpServers"]["bicraft"]["command"],
        cmd
    );
    assert_eq!(
        read_json(&proj.join(".vscode").join("mcp.json"))["servers"]["bicraft"]["command"],
        cmd
    );
    assert_eq!(
        read_json(&proj.join(".cursor").join("mcp.json"))["mcpServers"]["bicraft"]["command"],
        cmd
    );
    assert!(read(&proj.join(".codex").join("config.toml")).contains("[mcp_servers.bicraft]"));
    assert!(!r.join(".claude.json").exists());
    assert!(
        out.json["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("committed"))
    );

    let out = setup(
        &r,
        &[
            "--client",
            "claude_desktop",
            "--scope",
            "project",
            "--project-dir",
            proj_s,
        ],
    );
    assert_eq!(out.code, 2);
}

#[test]
fn usage_errors() {
    let r = root("usage");
    for args in [
        vec!["--client", "nope"],
        vec!["--scope", "galaxy"],
        vec!["--command", "relative/bicraft"],
        vec!["--bogus"],
        vec!["doctor", "--yes"],
    ] {
        let out = setup_on(&r, "windows", &args);
        assert_eq!(out.code, 2, "{args:?}");
    }
    // --platform needs a sandbox root.
    let out = run({
        let mut c = bicraft();
        c.args(["setup", "--platform", "linux", "--dry-run", "--json"]);
        c
    });
    assert_eq!(out.code, 2);
    assert!(snapshot(&r).is_empty());
    // Help.
    let out = run({
        let mut c = bicraft();
        c.args(["setup", "--help"]);
        c
    });
    assert_eq!(out.code, 0);
    assert!(out.stdout.contains("--config-root"));
}

#[test]
fn env_root_and_explicit_command() {
    let r = root("env");
    let elsewhere = root("env-elsewhere");
    let fake = if cfg!(windows) {
        r"C:\tools\bicraft.exe"
    } else {
        "/opt/tools/bicraft"
    };
    let mut cmd = bicraft();
    cmd.env("BICRAFT_SETUP_HOME", &r)
        // Ignored in a sandbox: they must not redirect writes outside it.
        .env("CLAUDE_CONFIG_DIR", &elsewhere)
        .env("CODEX_HOME", &elsewhere)
        .args([
            "setup",
            "--client",
            "claude_code",
            "--client",
            "codex",
            "--command",
            fake,
            "--yes",
            "--json",
        ]);
    let out = run(cmd);
    assert_eq!(out.code, 0, "{}", out.stdout);
    assert_eq!(out.json["server"]["command"], fake);
    assert!(
        out.json["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("does not exist"))
    );
    let platform = out.json["platform"].as_str().unwrap().to_string();
    assert_eq!(
        read_json(&path(&r, &platform, "claude_code"))["mcpServers"]["bicraft"]["command"],
        fake
    );
    assert!(read(&path(&r, &platform, "codex")).contains(&toml_str(fake)));
    assert!(snapshot(&elsewhere).is_empty());
}

#[test]
fn claude_desktop_msix_private_copy() {
    let r = root("msix");
    let package = r
        .join("AppData")
        .join("Local")
        .join("Packages")
        .join("Claude_pzs8sxrjxfjjc");
    let private = package
        .join("LocalCache")
        .join("Roaming")
        .join("Claude")
        .join("claude_desktop_config.json");
    std::fs::create_dir_all(&package).unwrap();
    // Package installed, no file yet: the private copy is created.
    let out = setup(&r, &["--client", "claude_desktop", "--dry-run"]);
    assert_eq!(
        PathBuf::from(client(&out, "claude_desktop")["path"].as_str().unwrap()),
        private
    );
    assert_eq!(client(&out, "claude_desktop")["detected"], true);
    // An existing %APPDATA% file is still read until the package saves its own copy.
    let plain = path(&r, "windows", "claude_desktop");
    write(&plain, "{}");
    let out = setup(&r, &["--client", "claude_desktop", "--dry-run"]);
    assert_eq!(
        PathBuf::from(client(&out, "claude_desktop")["path"].as_str().unwrap()),
        plain
    );
    write(&private, "{\"preferences\": {}}");
    let out = setup(&r, &["--client", "claude_desktop", "--yes"]);
    assert_eq!(out.json["status"], "applied");
    assert!(
        out.json["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("Claude Desktop"))
    );
    assert_eq!(read_json(&private)["preferences"], json!({}));
    assert!(read_json(&private)["mcpServers"]["bicraft"].is_object());
    assert_eq!(read(&plain), "{}");
}

#[test]
fn codex_edge_cases() {
    // A file without a final newline, with CRLF line endings.
    let r = root("codex-crlf");
    let p = path(&r, "windows", "codex");
    write(&p, "model = \"x\"\r\n[tui]\r\nnotifications = true");
    let out = setup(&r, &["--client", "codex", "--yes"]);
    assert_eq!(out.code, 0, "{}", out.stdout);
    let cmd = command(&out);
    assert_eq!(
        read(&p),
        format!(
            "model = \"x\"\r\n[tui]\r\nnotifications = true\r\n\r\n[mcp_servers.bicraft]\r\ncommand = {}\r\nargs = [\"mcp\"]\r\n",
            toml_str(&cmd)
        )
    );
    let out = setup(&r, &["--client", "codex", "--uninstall", "--yes"]);
    assert_eq!(out.json["status"], "applied");
    assert_eq!(read(&p), "model = \"x\"\r\n[tui]\r\nnotifications = true\r\n");

    // Only a sub-table exists: the main table is put before it.
    let r = root("codex-sub");
    let p = path(&r, "windows", "codex");
    write(&p, "[mcp_servers.bicraft.env]\nA = 'b'\n");
    let out = setup(&r, &["--client", "codex", "--yes"]);
    assert_eq!(client(&out, "codex")["action"], "update");
    assert_eq!(
        read(&p),
        format!(
            "[mcp_servers.bicraft]\ncommand = {}\nargs = [\"mcp\"]\n\n[mcp_servers.bicraft.env]\nA = 'b'\n",
            toml_str(&command(&out))
        )
    );
    assert_eq!(
        setup(&r, &["--client", "codex", "--yes"]).json["status"],
        "unchanged"
    );

    // A table without command or args gets them after its header.
    let r = root("codex-empty");
    let p = path(&r, "windows", "codex");
    write(&p, "[mcp_servers.bicraft]\nenabled = true\n");
    let out = setup(&r, &["--client", "codex", "--yes"]);
    assert_eq!(
        read(&p),
        format!(
            "[mcp_servers.bicraft]\ncommand = {}\nargs = [\"mcp\"]\nenabled = true\n",
            toml_str(&command(&out))
        )
    );

    // Strings of every kind, multi-line values and inline tables are scanned, not misread.
    let r = root("codex-strings");
    let p = path(&r, "windows", "codex");
    let text = "a = \"\"\"\n[mcp_servers.bicraft]\n\"\"\"\nb = '''\nx''' # c\nc = 'lit'\nd = \"\\u00e9\\\"\"\n\
                e = [ 1, [2, 3], { x = 1 } , ]\nf = { g.h = \"i\" }\nt = 1979-05-27 07:32:00Z\n\n['quoted key'.\"x\"]\nv = -1.5e3\n";
    write(&p, text);
    let out = setup(&r, &["--client", "codex", "--dry-run"]);
    assert_eq!(out.code, 0, "{}", out.stdout);
    assert_eq!(client(&out, "codex")["registration"], "absent");
    assert_eq!(client(&out, "codex")["action"], "add");

    // bicraft defined with dotted keys: reported, not edited.
    let r = root("codex-dotted");
    let p = path(&r, "windows", "codex");
    let text = "mcp_servers.bicraft.command = \"/old\"\nmcp_servers.bicraft.args = [\"mcp\"]\n";
    write(&p, text);
    let out = setup(&r, &["--client", "codex", "--yes"]);
    assert_eq!(out.code, 1);
    assert_eq!(client(&out, "codex")["registration"], "stale");
    assert_eq!(client(&out, "codex")["registered_command"], "/old");
    assert!(
        client(&out, "codex")["error"]
            .as_str()
            .unwrap()
            .contains("by hand")
    );
    assert_eq!(read(&p), text);
}

#[test]
fn empty_and_bom_json_files() {
    let r = root("empty");
    write(&path(&r, "windows", "cursor"), "");
    write(&path(&r, "windows", "vscode"), "\u{feff}{\"servers\": {}}");
    let out = setup(&r, &["--client", "cursor", "--client", "vscode", "--yes"]);
    assert_eq!(out.code, 0, "{}", out.stdout);
    assert!(read_json(&path(&r, "windows", "cursor"))["mcpServers"]["bicraft"].is_object());
    let vs = read(&path(&r, "windows", "vscode"));
    assert!(vs.starts_with('\u{feff}'));
    assert!(!vs.contains('\n'), "a one-line file stays on one line: {vs}");
}

#[test]
fn doctor_reports_handshake_and_registrations() {
    let r = root("doctor");
    write(
        &path(&r, "windows", "cursor"),
        "{\"mcpServers\": {\"bicraft\": {\"command\": \"/gone/bicraft\", \"args\": [\"mcp\"]}}}",
    );
    write(&path(&r, "windows", "vscode"), "{ nope");
    let before = snapshot(&r);

    let out = setup(&r, &["doctor"]);
    assert_eq!(out.json["handshake"]["ok"], true, "{}", out.stdout);
    assert_eq!(out.json["handshake"]["server_name"], "bicraft");
    assert!(out.json["handshake"]["tools"].as_u64().unwrap() > 5);
    assert_eq!(client(&out, "cursor")["registration"], "stale");
    assert_eq!(client(&out, "cursor")["registered_command_exists"], false);
    assert_eq!(client(&out, "vscode")["registration"], "invalid");
    assert_eq!(client(&out, "codex")["registration"], "absent");
    // The stale registration makes it unhealthy; the malformed file is only a warning.
    assert_eq!(out.json["healthy"], false);
    assert_eq!(out.code, 1);
    assert_eq!(out.json["warnings"].as_array().unwrap().len(), 1);

    assert_eq!(
        setup(&r, &["--client", "cursor", "--yes"]).json["status"],
        "applied"
    );
    let out = setup(&r, &["doctor"]);
    assert_eq!(out.json["healthy"], true, "{}", out.stdout);
    assert_eq!(out.code, 0);
    assert_eq!(client(&out, "cursor")["registration"], "aligned");

    // Asking about one client: it must be registered.
    let out = setup(&r, &["doctor", "--client", "codex"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.json["healthy"], false);

    // A binary that cannot start fails the handshake.
    let missing = r.join("missing").join("bicraft-nowhere");
    let out = setup(&r, &["doctor", "--command", missing.to_str().unwrap()]);
    assert_eq!(out.code, 1);
    assert_eq!(out.json["handshake"]["ok"], false);

    // Human output.
    let out = setup_on(&r, "windows", &["doctor", "--client", "cursor"]);
    assert_eq!(out.code, 0, "{}", out.stdout);
    assert!(out.stdout.contains("mcp      ok: bicraft"), "{}", out.stdout);
    assert!(out.stdout.contains("Healthy."));

    // Doctor never writes.
    let mut after = snapshot(&r);
    after.retain(|p, _| before.contains_key(p));
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        before.keys().collect::<Vec<_>>()
    );
    assert_eq!(read(&path(&r, "windows", "vscode")), "{ nope");
}

#[test]
fn inside_claude_desktops_package_appdata_writes_are_refused() {
    // The packaged app's private copy and the %APPDATA% file are the same file in that view.
    let r = root("redirected");
    let plain = path(&r, "windows", "claude_desktop");
    let private = r
        .join("AppData")
        .join("Local")
        .join("Packages")
        .join("Claude_pzs8sxrjxfjjc")
        .join("LocalCache")
        .join("Roaming")
        .join("Claude")
        .join("claude_desktop_config.json");
    write(&plain, "{\"preferences\": {}}");
    write(&private, "{\"preferences\": {}}");
    let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    for p in [&plain, &private] {
        std::fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }
    let before = snapshot(&r);
    let out = setup(&r, &["--client", "vscode", "--client", "claude_desktop", "--yes"]);
    assert_eq!(out.code, 1, "{}", out.stdout);
    assert_eq!(out.json["status"], "blocked");
    assert!(
        client(&out, "vscode")["error"]
            .as_str()
            .unwrap()
            .contains("ordinary terminal")
    );
    assert_eq!(snapshot(&r), before);

    // Files outside %APPDATA% are still written, with a warning.
    let out = setup(&r, &["--client", "cursor", "--yes"]);
    assert_eq!(out.json["status"], "applied", "{}", out.stdout);
    assert!(
        out.json["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("MSIX"))
    );
}
