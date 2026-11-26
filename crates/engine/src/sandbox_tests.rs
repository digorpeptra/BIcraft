//! Server mode: path sandbox and command policy (`sandbox.rs`, `docs/server-mode.md`).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::*;
use crate::tempdir::TempDir;
use crate::{COMMANDS, Session};

/// A workspace root and a server-mode session rooted there.
fn workspace(name: &str) -> (TempDir, Session) {
    let dir = TempDir::new(name);
    let s = Session::server(ServerConfig::new(&dir).unwrap());
    (dir, s)
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn root(s: &Session) -> PathBuf {
    s.server_config().unwrap().root().to_path_buf()
}

/// A path outside any workspace root, absolute, as this OS writes it.
fn outside_abs() -> String {
    std::env::temp_dir()
        .join("bicraft-outside.bim")
        .display()
        .to_string()
}

/// Every refusal class with sample values (the OS-specific ones only where they apply).
fn bad_paths() -> Vec<(&'static str, String)> {
    let mut v = vec![
        ("absolute", outside_abs()),
        ("absolute", "/etc/passwd".to_string()),
        ("absolute", "\\Windows\\win.ini".to_string()),
        ("drive", "C:/Windows/win.ini".to_string()),
        ("drive", "c:relative.bim".to_string()),
        ("parent", "../x.bim".to_string()),
        ("parent", "a/../../x.bim".to_string()),
        ("parent", "a\\..\\..\\x.bim".to_string()),
        ("parent", "..".to_string()),
        ("unc", "\\\\server\\share\\x.bim".to_string()),
        ("unc", "//server/share/x.bim".to_string()),
        ("device", "\\\\?\\C:\\x.bim".to_string()),
        ("device", "\\\\.\\pipe\\x".to_string()),
        ("device", "//?/C:/x.bim".to_string()),
        ("nul", "a\0b.bim".to_string()),
    ];
    if cfg!(windows) {
        v.extend([
            ("ads", "x.bim:stream".to_string()),
            ("ads", "x.bim::$DATA".to_string()),
            ("reserved", "NUL".to_string()),
            ("reserved", "con.txt".to_string()),
            ("reserved", "dir/COM1.bim".to_string()),
            ("reserved", "lpt9".to_string()),
            ("reserved", "x.bim.".to_string()),
            ("reserved", "x.bim ".to_string()),
        ]);
    }
    v
}

fn assert_refused(r: Result<Value, String>, what: &str) {
    match r {
        Ok(v) => panic!("{what}: accepted: {v}"),
        Err(e) => assert!(
            e.contains("server mode") && (e.contains("refused") || e.contains("leaves the workspace root")),
            "{what}: unexpected error: {e}"
        ),
    }
}

fn assert_not_sandbox_error(r: Result<Value, String>, what: &str) {
    if let Err(e) = r {
        assert!(!e.contains("server mode"), "{what}: refused by the sandbox: {e}");
    }
}

// ---- the path-parameter audit -------------------------------------------------------------

/// Every path parameter of every command (the audit in `docs/server-mode.md`). A new path
/// parameter must be declared on its command and listed here.
const EXPECTED_PATHS: &[(&str, &str, PathKind)] = &[
    ("file.open", "path", PathKind::One),
    ("file.open", "data_root", PathKind::Clearable),
    ("file.open", "cache_abf", PathKind::Clearable),
    ("file.set_cache_abf", "cache_abf", PathKind::Clearable),
    ("file.save", "data_root", PathKind::Clearable),
    ("file.save_as", "path", PathKind::One),
    ("file.save_as", "data_root", PathKind::Clearable),
    ("file.convert", "from", PathKind::Delegated),
    ("file.convert", "to", PathKind::Delegated),
    ("file.convert", "data_root", PathKind::Delegated),
    ("file.redact", "path", PathKind::Delegated),
    ("file.redact", "out", PathKind::Delegated),
    ("project.new", "dir", PathKind::One),
    ("perf.export", "out", PathKind::One),
    ("project.test", "path", PathKind::Delegated),
    ("project.test", "tests", PathKind::Many),
    ("project.test", "base_dir", PathKind::Clearable),
    ("model.storage_stats", "cache_abf", PathKind::Clearable),
    ("model.export_vpax", "path", PathKind::One),
    ("model.export_vpax", "cache_abf", PathKind::Clearable),
    ("model.analyze", "rules", PathKind::Many),
    ("model.usage", "reports", PathKind::Many),
    ("model.unused", "reports", PathKind::Many),
    ("model.remove_unused", "reports", PathKind::Many),
    ("table.import", "csv", PathKind::PathOrText),
    ("table.import", "data_root", PathKind::Clearable),
    ("table.set_data", "csv", PathKind::PathOrText),
    ("table.set_data", "data_root", PathKind::Clearable),
    ("report.render", "out", PathKind::One),
    ("report.render", "font_dirs", PathKind::Many),
    ("report.render", "cache_abf", PathKind::Clearable),
    ("report.export_pdf", "out", PathKind::One),
    ("report.export_pdf", "font_dirs", PathKind::Many),
    ("report.print", "out", PathKind::One),
    ("visual.export_data", "out", PathKind::One),
    ("theme.import", "path", PathKind::One),
    ("theme.export", "path", PathKind::One),
    ("file.diff", "a", PathKind::Clearable),
    ("file.diff", "b", PathKind::Clearable),
    ("file.textconv", "file", PathKind::One),
    ("objects.import", "source", PathKind::Delegated),
    ("dax.evaluate", "cache_abf", PathKind::Clearable),
    ("model.refresh", "data_root", PathKind::Clearable),
    ("query.preview", "data_root", PathKind::Clearable),
    ("query.profile", "data_root", PathKind::Clearable),
    ("query.export", "out", PathKind::One),
    ("query.export", "data_root", PathKind::Clearable),
    ("query.folding", "data_root", PathKind::Clearable),
    ("query.native", "data_root", PathKind::Clearable),
    ("query.diagnose", "data_root", PathKind::Clearable),
    ("source.navigate", "path", PathKind::One),
    ("source.navigate", "data_root", PathKind::Clearable),
    ("source.load", "data_root", PathKind::Clearable),
    ("query.apply", "data_root", PathKind::Clearable),
    ("env.apply", "env_file", PathKind::Many),
    ("env.list", "env_file", PathKind::Many),
    ("env.check", "env_file", PathKind::Many),
    ("image.add", "path", PathKind::One),
    ("image.set", "path", PathKind::One),
    ("shape_map.set_map", "path", PathKind::One),
    ("visual.import_custom", "path", PathKind::One),
    ("visual.scaffold", "folder", PathKind::One),
    ("visual.build", "folder", PathKind::One),
    ("visual.build", "out", PathKind::Clearable),
    ("visual.pack", "folder", PathKind::One),
    ("visual.pack", "out", PathKind::Clearable),
    ("visual.export_bicraft_visual", "out", PathKind::One),
    ("page.format", "background_image_path", PathKind::Clearable),
    ("page.format", "wallpaper_image_path", PathKind::Clearable),
];

#[test]
fn every_path_parameter_is_declared_and_audited() {
    let mut declared: Vec<(&str, &str, PathKind)> = COMMANDS
        .iter()
        .flat_map(|c| c.server.paths.iter().map(move |p| (c.id, p.name, p.kind)))
        .collect();
    let mut expected = EXPECTED_PATHS.to_vec();
    declared.sort_by_key(|(c, p, _)| (*c, *p));
    expected.sort_by_key(|(c, p, _)| (*c, *p));
    assert_eq!(declared, expected);
    // every declared parameter is one the command accepts (its params doc names it)
    for c in COMMANDS {
        for p in c.server.paths {
            assert!(
                c.params.contains(p.name),
                "{}: `{}` is not in its params doc",
                c.id,
                p.name
            );
        }
    }
}

#[test]
fn only_the_desktop_launch_and_sql_credentials_are_denied_and_only_refresh_uses_the_network() {
    let denied: Vec<&str> = COMMANDS
        .iter()
        .filter(|c| matches!(c.server.access, Access::Denied(_)))
        .map(|c| c.id)
        .collect();
    // SQL Server credentials stay with a desktop session: a server session is shared
    assert_eq!(denied, ["file.open_in_desktop", "source.credentials"]);
    let network: Vec<&str> = COMMANDS
        .iter()
        .filter(|c| c.server.access == Access::Network)
        .map(|c| c.id)
        .collect();
    assert_eq!(network, ["model.refresh"]);
    assert_eq!(
        server_policy("file.open").map(|p| p.access),
        Some(Access::Allowed)
    );
    assert_eq!(server_policy("nope"), None);
}

#[test]
fn denied_commands_are_not_available_in_server_mode_only() {
    let (_dir, server) = workspace("sandbox-available");
    let local = Session::new();
    assert!(local.command_available("file.open_in_desktop"));
    assert!(!server.command_available("file.open_in_desktop"));
    assert!(server.command_available("file.open") && server.command_available("model.refresh"));
    assert!(!server.command_available("nope") && !local.command_available("nope"));
    let ids = |s: &Session, f: Option<&str>| -> Vec<&'static str> {
        s.available_commands(f).iter().map(|c| c.id).collect()
    };
    assert_eq!(ids(&local, None).len(), COMMANDS.len());
    assert_eq!(ids(&server, None).len(), COMMANDS.len() - 2);
    assert!(!server.command_available("source.credentials"));
    assert!(!ids(&server, None).contains(&"file.open_in_desktop"));
    assert!(ids(&local, Some("desktop")).contains(&"file.open_in_desktop"));
    assert!(!ids(&server, Some("desktop")).contains(&"file.open_in_desktop"));
}

#[test]
fn the_network_setting_alone_from_the_environment() {
    fn get(v: Option<&'static str>) -> impl Fn(&str) -> Option<std::ffi::OsString> {
        move |k| {
            if k == SERVER_NETWORK_ENV {
                v.map(Into::into)
            } else {
                None
            }
        }
    }
    assert_eq!(ServerConfig::network_from_lookup(&get(None)), Ok(false));
    assert_eq!(ServerConfig::network_from_lookup(&get(Some("on"))), Ok(true));
    assert_eq!(ServerConfig::network_from_lookup(&get(Some("0"))), Ok(false));
    assert!(ServerConfig::network_from_lookup(&get(Some("maybe"))).is_err());
}

// ---- rejection classes, per command --------------------------------------------------------

/// Every `One`/`Clearable`/`Many` path parameter of every command refuses every class, before
/// the command runs (no file open needed).
#[test]
fn every_declared_path_parameter_refuses_every_class() {
    let (_dir, mut s) = workspace("sandbox-classes");
    let mut checked = 0;
    for c in COMMANDS {
        for p in c.server.paths {
            if !matches!(p.kind, PathKind::One | PathKind::Clearable | PathKind::Many) {
                continue;
            }
            for (class, bad) in bad_paths() {
                let what = format!("{} {{{}: {class} {bad:?}}}", c.id, p.name);
                assert_refused(s.execute(c.id, &json!({ p.name: bad })), &what);
                if p.kind == PathKind::Many {
                    assert_refused(s.execute(c.id, &json!({ p.name: ["ok.json", bad] })), &what);
                }
                checked += 1;
            }
        }
    }
    assert!(checked > 100, "{checked}");
    // nothing was written anywhere under the root
    assert_eq!(std::fs::read_dir(root(&s)).unwrap().count(), 0);
}

#[test]
fn table_import_csv_refuses_paths_outside_and_reads_files_inside() {
    let (dir, mut s) = workspace("sandbox-csv");
    run(&mut s, "model.new", json!({"name": "M"}));
    for (class, bad) in bad_paths() {
        if matches!(class, "ads" | "reserved") {
            continue; // single-line CSV text such as `time,12:30`: read as text, never as a file
        }
        for id in ["table.import", "table.set_data"] {
            let r = s.execute(id, &json!({"name": "T", "csv": bad}));
            assert_refused(r, &format!("{id} csv {class} {bad:?}"));
        }
    }
    std::fs::write(dir.join("d.csv"), "Region,Amount\nNorth,1\nSouth,2\n").unwrap();
    std::fs::create_dir(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub").join("e.csv"), "K\nx\n").unwrap();
    run(&mut s, "table.import", json!({"name": "T", "csv": "d.csv"}));
    run(&mut s, "table.import", json!({"name": "U", "csv": "sub\\e.csv"}));
    run(&mut s, "table.set_data", json!({"name": "U", "csv": "sub/e.csv"}));
    // a single-line value that is no file under the root is CSV text, as before
    run(&mut s, "table.import", json!({"name": "V", "csv": "a,b"}));
    let t = run(&mut s, "table.get", json!({"name": "T"}));
    let cols: Vec<&str> = t["columns"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert!(cols.contains(&"Region") && cols.contains(&"Amount"), "{t}");
    let v = run(&mut s, "table.get", json!({"name": "V"}));
    let cols: Vec<&str> = v["columns"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert!(cols.contains(&"a") && cols.contains(&"b"), "{v}");
    if cfg!(windows) {
        // `:` is no stream here: CSV text
        run(&mut s, "table.import", json!({"name": "W", "csv": "time,12:30"}));
    }
}

#[test]
fn file_convert_resolves_from_to_and_data_root_in_its_private_session() {
    let (dir, mut s) = workspace("sandbox-convert");
    run(&mut s, "model.new", json!({"name": "M"}));
    run(&mut s, "file.save_as", json!({"path": "in/m.bim"}));
    for (class, bad) in bad_paths() {
        let what = format!("file.convert {class} {bad:?}");
        assert_refused(
            s.execute("file.convert", &json!({"from": bad, "to": "out/c.bim"})),
            &format!("{what} from"),
        );
        assert_refused(
            s.execute("file.convert", &json!({"from": "in/m.bim", "to": bad})),
            &format!("{what} to"),
        );
        assert_refused(
            s.execute(
                "file.convert",
                &json!({"from": "in/m.bim", "to": "out/c.bim", "data_root": bad}),
            ),
            &format!("{what} data_root"),
        );
    }
    assert!(!dir.join("out").exists());
    run(
        &mut s,
        "file.convert",
        json!({"from": "in/m.bim", "to": "out/c.pbip"}),
    );
    assert!(dir.join("out").join("c.pbip").is_file());
}

// ---- accepted relative paths ---------------------------------------------------------------

#[test]
fn relative_paths_resolve_under_the_root() {
    let (dir, mut s) = workspace("sandbox-accept");
    let r = root(&s);
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(&mut s, "report.new", json!({}));
    // project.new {dir}
    let out = run(&mut s, "project.new", json!({"dir": "proj", "name": "Sales"}));
    assert!(dir.join("proj").join("Sales.pbip").is_file(), "{out}");
    assert!(Path::new(out["path"].as_str().unwrap()).starts_with(&r), "{out}");
    // file.save_as {path} (a model.bim), `\` as a separator, `.` components
    run(&mut s, "file.save_as", json!({"path": "./out\\m.bim"}));
    assert!(dir.join("out").join("m.bim").is_file());
    // file.open {path, data_root, cache_abf}
    std::fs::create_dir(dir.join("data")).unwrap();
    std::fs::write(dir.join("c.abf"), b"not a real backup").unwrap();
    let out = run(
        &mut s,
        "file.open",
        json!({"path": "proj/Sales.pbip", "data_root": "data", "cache_abf": "c.abf"}),
    );
    assert_eq!(
        Path::new(out["data_root"].as_str().unwrap()),
        r.join("data"),
        "{out}"
    );
    assert_eq!(
        Path::new(out["cache_abf"].as_str().unwrap()),
        r.join("c.abf"),
        "{out}"
    );
    // file.set_cache_abf {cache_abf}; null and "" still clear it
    run(&mut s, "file.set_cache_abf", json!({"cache_abf": null}));
    run(&mut s, "file.set_cache_abf", json!({"cache_abf": "c.abf"}));
    run(&mut s, "file.set_cache_abf", json!({"cache_abf": ""}));
    // file.save {data_root}
    run(&mut s, "file.save", json!({"data_root": "data"}));
    // theme.export / theme.import {path}
    run(
        &mut s,
        "theme.set",
        json!({"theme": {"name": "T", "dataColors": ["#112233", "#445566"]}}),
    );
    run(&mut s, "theme.export", json!({"path": "t.json"}));
    assert!(dir.join("t.json").is_file());
    run(&mut s, "theme.import", json!({"path": "t.json"}));
    // report.render {out, font_dirs}
    std::fs::create_dir(dir.join("fonts")).unwrap();
    let out = run(
        &mut s,
        "report.render",
        json!({"out": "png", "font_dirs": ["fonts"], "system_fonts": false}),
    );
    let png = PathBuf::from(out["pages"][0]["path"].as_str().unwrap());
    assert!(png.starts_with(&r) && png.is_file(), "{out}");
    // model.analyze {rules}
    std::fs::write(
        dir.join("rules.json"),
        r#"[{"ID":"R1","Name":"r","Severity":1,"Scope":"Model","Expression":"Tables.Count = 0"}]"#,
    )
    .unwrap();
    run(
        &mut s,
        "model.analyze",
        json!({"rules": "rules.json", "default_rules": false}),
    );
    run(
        &mut s,
        "model.analyze",
        json!({"rules": ["rules.json"], "default_rules": false}),
    );
    // commands whose state needs stored data: the path passes the sandbox, then the command
    // answers as in local mode
    assert_not_sandbox_error(
        s.execute("model.export_vpax", &json!({"path": "v.vpax"})),
        "export_vpax",
    );
    assert_not_sandbox_error(
        s.execute("model.storage_stats", &json!({"cache_abf": "c.abf"})),
        "storage_stats",
    );
    assert_not_sandbox_error(
        s.execute(
            "dax.evaluate",
            &json!({"query": "EVALUATE {1}", "cache_abf": "c.abf"}),
        ),
        "dax.evaluate",
    );
    assert_not_sandbox_error(
        s.execute("model.refresh", &json!({"data_root": "data"})),
        "model.refresh",
    );
    // the root itself
    assert_eq!(ServerConfig::new(&dir).unwrap().resolve(".").unwrap(), r);
    assert_eq!(
        ServerConfig::new(&dir).unwrap().resolve("a/./b/").unwrap(),
        r.join("a").join("b")
    );
}

#[test]
fn local_mode_is_unchanged() {
    let mut s = Session::new();
    assert!(s.server_config().is_none());
    for (_, bad) in bad_paths() {
        if let Err(e) = s.execute("file.open", &json!({ "path": bad })) {
            assert!(!e.contains("server mode"), "{e}");
        }
    }
    // the desktop launch is a local command (dry run: nothing launched)
    let e = Session::new()
        .execute("file.open_in_desktop", &json!({"dry_run": true}))
        .unwrap_err();
    assert!(!e.contains("server mode"), "{e}");
    // an absolute path works as before
    let dir = TempDir::new("sandbox-local");
    run(&mut s, "model.new", json!({"name": "M"}));
    let path = dir.join("m.bim");
    run(
        &mut s,
        "file.save_as",
        json!({"path": path.display().to_string()}),
    );
    assert!(path.is_file());
}

// ---- policy --------------------------------------------------------------------------------

#[test]
fn denied_commands_return_a_clear_error_in_server_mode() {
    let (_dir, mut s) = workspace("sandbox-deny");
    let launched = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = std::sync::Arc::clone(&launched);
    s.external_opener = Some(crate::ExternalOpener(std::sync::Arc::new(move |_: &Path| {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    })));
    run(&mut s, "model.new", json!({"name": "M"}));
    run(&mut s, "report.new", json!({}));
    run(&mut s, "project.new", json!({"dir": ".", "name": "P"}));
    for p in [json!({}), json!({"dry_run": true})] {
        let e = s.execute("file.open_in_desktop", &p).unwrap_err();
        assert!(e.contains("not available in server mode"), "{e}");
    }
    assert!(!launched.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(s.last_command.as_ref().map(|c| c.ok), Some(false));
    // the viewer's open route reads desktop_target: nothing to open on a server
    let t = s.desktop_target();
    assert!(t.path.is_none() && t.reason.as_deref().is_some_and(|r| r.contains("server mode")));
}

#[test]
fn the_databricks_connector_needs_the_network_setting() {
    for allow in [false, true] {
        let dir = TempDir::new("sandbox-network");
        let mut s = Session::server(ServerConfig::new(&dir).unwrap().allow_network(allow));
        s.token_source = Some(crate::TokenSource(std::sync::Arc::new(|| {
            Err("no token in tests".to_string())
        })));
        run(&mut s, "model.new", json!({"name": "M"}));
        run(&mut s, "project.new", json!({"dir": "p", "name": "P"}));
        let r = s.execute("model.refresh", &json!({"allow_connectors": ["databricks"]}));
        match (allow, r) {
            (false, Err(e)) => assert!(e.contains("needs network access"), "{e}"),
            (false, Ok(v)) => panic!("refresh ran without the network setting: {v}"),
            (true, r) => assert_not_sandbox_error(r, "allowed network"),
        }
        // without a connector the refresh needs no network and runs either way
        assert_not_sandbox_error(
            s.execute("model.refresh", &json!({})),
            "refresh without connectors",
        );
    }
}

// ---- links ---------------------------------------------------------------------------------

/// Makes `link` point to the folder `target`; false when this OS or account cannot.
fn link_dir(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        if std::os::windows::fs::symlink_dir(target, link).is_ok() {
            return true;
        }
        // a junction needs no privilege
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|o| o.status.success())
            && link.exists()
    }
}

/// Makes `link` point to the (possibly missing) file `target`; false when not permitted.
fn link_file(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }
}

#[test]
fn links_leaving_the_root_are_refused() {
    let (dir, mut s) = workspace("sandbox-links");
    let outside = TempDir::new("sandbox-links-outside");
    let mut local = Session::new();
    run(&mut local, "model.new", json!({"name": "M"}));
    run(
        &mut local,
        "file.save_as",
        json!({"path": outside.join("x.bim").display().to_string()}),
    );
    if !link_dir(&outside, &dir.join("out")) {
        eprintln!("note: cannot create a directory link here; link escape tests skipped");
        return;
    }
    std::fs::create_dir(dir.join("inner")).unwrap();
    let inner_ok = link_dir(&dir.join("inner"), &dir.join("alias"));
    run(&mut s, "model.new", json!({"name": "M"}));
    // an existing file through the link, a new file through the link, the link itself
    assert_refused(
        s.execute("file.open", &json!({"path": "out/x.bim"})),
        "open through link",
    );
    assert_refused(
        s.execute("file.save_as", &json!({"path": "out/new.bim"})),
        "save through link",
    );
    assert_refused(
        s.execute("project.new", &json!({"dir": "out", "name": "P"})),
        "project.new into link",
    );
    assert_refused(
        s.execute("file.open", &json!({"path": "out", "data_root": "out"})),
        "data_root link",
    );
    assert!(!outside.join("new.bim").exists());
    // a link that stays inside is fine
    if inner_ok {
        run(&mut s, "file.save_as", json!({"path": "alias/m.bim"}));
        assert!(dir.join("inner").join("m.bim").is_file());
    }
    // a dangling link (its target may be created later, outside)
    if link_file(&outside.join("missing.bim"), &dir.join("dangling.bim")) {
        assert_refused(
            s.execute("file.save_as", &json!({"path": "dangling.bim"})),
            "dangling link",
        );
        assert_refused(
            s.execute("file.save_as", &json!({"path": "dangling.bim/x.bim"})),
            "below dangling",
        );
        assert!(!outside.join("missing.bim").exists());
    } else {
        eprintln!("note: cannot create a file link here; dangling-link test skipped");
    }
}

#[test]
fn project_folders_linked_out_of_the_root_are_refused() {
    let (dir, mut s) = workspace("sandbox-project-links");
    let outside = TempDir::new("sandbox-project-outside");
    let mut local = Session::new();
    run(&mut local, "model.new", json!({"name": "M"}));
    run(&mut local, "report.new", json!({}));
    run(
        &mut local,
        "project.new",
        json!({"dir": outside.display().to_string(), "name": "Sales"}),
    );
    std::fs::create_dir(dir.join("p")).unwrap();
    std::fs::copy(outside.join("Sales.pbip"), dir.join("p").join("Sales.pbip")).unwrap();
    if !link_dir(
        &outside.join("Sales.SemanticModel"),
        &dir.join("p").join("Sales.SemanticModel"),
    ) {
        eprintln!("note: cannot create a directory link here; project link tests skipped");
        return;
    }
    assert_refused(
        s.execute("file.open", &json!({"path": "p/Sales.pbip"})),
        "open linked project",
    );
    assert_refused(
        s.execute("file.open", &json!({"path": "p"})),
        "open linked project folder",
    );
    // writing a project over the linked item folder
    run(&mut s, "model.new", json!({"name": "M"}));
    assert_refused(
        s.execute("project.new", &json!({"dir": "p", "name": "Sales"})),
        "project.new over link",
    );
    assert_refused(
        s.execute("file.save_as", &json!({"path": "p/Sales.pbip"})),
        "save_as over link",
    );
    assert_refused(
        s.execute(
            "file.convert",
            &json!({"from": "p/Sales.pbip", "to": "q/Sales.pbip"}),
        ),
        "convert from linked project",
    );
    // a link deeper inside a project folder
    std::fs::create_dir(dir.join("r")).unwrap();
    run(&mut s, "project.new", json!({"dir": "r", "name": "Deep"}));
    let defn = dir.join("r").join("Deep.SemanticModel").join("definition");
    assert!(link_dir(&outside, &defn.join("escape")));
    assert_refused(
        s.execute("file.open", &json!({"path": "r/Deep.pbip"})),
        "deep link",
    );
}

// ---- configuration -------------------------------------------------------------------------

#[test]
fn configuration_from_the_environment() {
    let dir = TempDir::new("sandbox-env");
    let root = dir.display().to_string();
    let env = |pairs: &'static [(&'static str, &'static str)], root: &str| {
        let root = root.to_string();
        move |name: &str| -> Option<std::ffi::OsString> {
            if name == SERVER_ROOT_ENV {
                return Some(root.clone().into());
            }
            pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| (*v).into())
        }
    };
    assert_eq!(ServerConfig::from_lookup(|_| None).unwrap(), None);
    assert_eq!(ServerConfig::from_lookup(|_| Some("".into())).unwrap(), None);
    let c = ServerConfig::from_lookup(env(&[], &root)).unwrap().unwrap();
    assert!(!c.network_allowed());
    assert_eq!(c.root(), ServerConfig::new(&dir).unwrap().root());
    for yes in ["1", "true", "YES", " on "] {
        let pairs: &'static [(&'static str, &'static str)] = match yes {
            "1" => &[(SERVER_NETWORK_ENV, "1")],
            "true" => &[(SERVER_NETWORK_ENV, "true")],
            "YES" => &[(SERVER_NETWORK_ENV, "YES")],
            _ => &[(SERVER_NETWORK_ENV, " on ")],
        };
        assert!(
            ServerConfig::from_lookup(env(pairs, &root))
                .unwrap()
                .unwrap()
                .network_allowed()
        );
    }
    assert!(
        !ServerConfig::from_lookup(env(&[(SERVER_NETWORK_ENV, "0")], &root))
            .unwrap()
            .unwrap()
            .network_allowed()
    );
    assert!(ServerConfig::from_lookup(env(&[(SERVER_NETWORK_ENV, "maybe")], &root)).is_err());
    // a missing root, or a file, is an error (never local mode by accident)
    let missing = dir.join("missing").display().to_string();
    assert!(ServerConfig::from_lookup(env(&[], &missing)).is_err());
    std::fs::write(dir.join("f"), b"").unwrap();
    assert!(ServerConfig::new(dir.join("f")).is_err());
    assert!(ServerConfig::new("").is_err());
}

#[test]
fn resolve_bounds_and_shapes() {
    let dir = TempDir::new("sandbox-resolve");
    let c = ServerConfig::new(&dir).unwrap();
    assert_eq!(c.resolve(""), Err(Refusal::Empty));
    assert_eq!(c.resolve("  "), Err(Refusal::Empty));
    assert_eq!(c.resolve(&"a".repeat(5000)), Err(Refusal::TooLong));
    assert_eq!(c.resolve(&"a/".repeat(300)), Err(Refusal::TooLong));
    assert_eq!(c.resolve("../x"), Err(Refusal::Parent));
    assert_eq!(c.resolve("C:x"), Err(Refusal::DriveLetter));
    assert_eq!(c.resolve("//s/x"), Err(Refusal::UncOrDevice));
    assert_eq!(c.resolve("/x"), Err(Refusal::Absolute));
    assert_eq!(c.resolve("a\0"), Err(Refusal::Nul));
    // a file used as a folder cannot be resolved
    std::fs::write(dir.join("f"), b"").unwrap();
    assert!(matches!(c.resolve("f/x"), Err(Refusal::Unresolvable(_))));
    // non-existing parts are kept as given
    assert_eq!(
        c.resolve("new/deeper/x.pbix").unwrap(),
        c.root().join("new").join("deeper").join("x.pbix")
    );
    for (_, bad) in bad_paths() {
        assert!(c.resolve(&bad).is_err(), "{bad:?}");
    }
}
