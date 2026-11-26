//! Get data › SharePoint folder and SharePoint list, web credentials and privacy levels (Data
//! source settings) through `Session::execute`, against the in-process SharePoint test site of
//! `bicraft-webdata` (feature `fake-server`; never the internet): credentials in session memory
//! only, the Navigator's items, preview and load with `allow_connectors: ["sharepoint"]`, refresh,
//! PBIP and PBIX round trips, server mode.

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::{ServerConfig, Session};
use bicraft_webdata::fake;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Team"}));
    s
}

fn token(s: &mut Session, site: &str) -> Value {
    run(
        s,
        "source.credentials",
        json!({"kind": "sharepoint", "url": site, "authentication": "bearer", "token": fake::SP_TOKEN}),
    )
}

#[test]
fn sharepoint_folder_navigator_load_refresh_and_pbip_round_trip() {
    let srv = fake::start_sharepoint().unwrap();
    let site = srv.url("/sites/team");
    let mut s = session();
    // without a credential the site refuses, and the error says where to set one
    let e = s
        .execute(
            "source.navigate",
            &json!({"kind": "sharepoint_folder", "url": site}),
        )
        .unwrap_err();
    assert!(e.contains("401") && e.contains("source.credentials"), "{e}");
    let stored = token(&mut s, &site);
    assert_eq!(stored["authentication"], "bearer");
    assert!(!stored.to_string().contains(fake::SP_TOKEN));
    let nav = run(
        &mut s,
        "source.navigate",
        json!({"kind": "sharepoint_folder", "url": site}),
    );
    assert_eq!(nav["allow_connectors"], json!(["sharepoint"]));
    let items = nav["items"].as_array().unwrap();
    let kinds: Vec<&str> = items.iter().filter_map(|i| i["kind"].as_str()).collect();
    assert_eq!(kinds, ["Folder", "Combined"], "{nav}");
    let m = items[0]["m"].as_str().unwrap().to_string();
    assert!(
        m.contains(&format!("SharePoint.Files(\"{site}\", [ApiVersion = 15])")),
        "{m}"
    );
    let preview = run(
        &mut s,
        "query.preview",
        json!({"expression": m, "allow_connectors": ["sharepoint"]}),
    );
    assert_eq!(preview["rows"].as_array().map(Vec::len), Some(4), "{preview}");
    // the combined CSV files: a.csv and q1.csv share no header, as Desktop's sample file decides
    let combined = items[1]["m"].as_str().unwrap().to_string();
    assert!(combined.contains("Table.Combine("), "{combined}");
    run(
        &mut s,
        "source.load",
        json!({"tables": [{"name": "Files", "m": m}], "allow_connectors": ["sharepoint"]}),
    );
    let dir = TempDir::new("engine-sharepoint-pbip");
    let path = dir.join("team.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    run(
        &mut s,
        "model.refresh",
        json!({"tables": ["Files"], "allow_connectors": ["sharepoint"]}),
    );
    run(&mut s, "file.save", json!({}));
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": path}));
    let rows = run(&mut again, "table.rows", json!({"table": "Files"}));
    assert_eq!(rows["rows"].as_array().map(Vec::len), Some(4), "{rows}");
    let t = run(&mut again, "table.get", json!({"name": "Files"}));
    assert!(t.to_string().contains("SharePoint.Files("), "{t}");
    // the token was never written
    let mut texts = String::new();
    let mut todo = vec![dir.path().to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            if e.path().is_dir() {
                todo.push(e.path());
            } else {
                texts.push_str(&String::from_utf8_lossy(&std::fs::read(e.path()).unwrap()));
            }
        }
    }
    assert!(!texts.contains(fake::SP_TOKEN));
    // a new session holds no credential
    let e = again
        .execute(
            "model.refresh",
            &json!({"tables": ["Files"], "allow_connectors": ["sharepoint"]}),
        )
        .unwrap_err();
    assert!(e.contains("401"), "{e}");
}

#[test]
fn sharepoint_list_navigator_and_pbix_round_trip() {
    let srv = fake::start_sharepoint().unwrap();
    let site = srv.url("/sites/team");
    let mut s = session();
    token(&mut s, &site);
    let nav = run(
        &mut s,
        "source.navigate",
        json!({"kind": "sharepoint_list", "url": site}),
    );
    let items = nav["items"].as_array().unwrap();
    let names: Vec<&str> = items.iter().filter_map(|i| i["name"].as_str()).collect();
    assert_eq!(names, ["Documents", "Tasks"]);
    let m = items[1]["m"].as_str().unwrap().to_string();
    assert!(
        m.contains(&format!(
            "SharePoint.Tables(\"{site}\", [Implementation = \"2.0\", ViewMode = \"All\"])"
        )),
        "{m}"
    );
    assert!(
        m.contains(&format!(
            "#\"{id}\" = Source{{[Id=\"{id}\"]}}[Items]",
            id = fake::SP_TASKS
        )),
        "{m}"
    );
    let preview = run(
        &mut s,
        "query.preview",
        json!({"expression": m, "allow_connectors": ["sharepoint"]}),
    );
    assert_eq!(preview["rows"].as_array().map(Vec::len), Some(3), "{preview}");
    // the lookup column is a record: loading keeps the scalar columns
    let load = format!("let Items = {m} in Table.SelectColumns(Items, {{\"Id\", \"Title\", \"Done\"}})");
    run(
        &mut s,
        "source.load",
        json!({"tables": [{"name": "Tasks", "m": load}], "allow_connectors": ["sharepoint"]}),
    );
    let dir = TempDir::new("engine-sharepoint-pbix");
    let path = dir.join("team.pbix");
    // saved without rows (Desktop loads them on refresh), refreshed (both pages of items), saved
    // again: the PBIX carries the rows
    run(
        &mut s,
        "file.save_as",
        json!({"path": path, "empty_tables": true}),
    );
    run(
        &mut s,
        "model.refresh",
        json!({"tables": ["Tasks"], "allow_connectors": ["sharepoint"]}),
    );
    run(&mut s, "file.save", json!({}));
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": path}));
    let rows = run(&mut again, "table.rows", json!({"table": "Tasks"}));
    assert_eq!(rows["rows"].as_array().map(Vec::len), Some(3), "{rows}");
    let t = run(&mut again, "table.get", json!({"name": "Tasks"}));
    assert!(t.to_string().contains("SharePoint.Tables("), "{t}");
}

#[test]
fn data_source_settings_list_credentials_and_privacy_levels() {
    let srv = fake::start_sharepoint().unwrap();
    let site = srv.url("/sites/team");
    let mut s = session();
    let v = run(&mut s, "source.settings", json!({}));
    assert_eq!(v["sources"], json!([]), "{v}");
    assert_eq!(v["firewall"], false);
    // a credential for the parent path covers the site
    token(&mut s, &srv.url("/sites"));
    run(
        &mut s,
        "source.load",
        json!({"tables": [{"name": "Files", "m": format!("Table.SelectColumns(SharePoint.Files(\"{site}\"), {{\"Name\"}})")}],
               "allow_connectors": ["sharepoint"]}),
    );
    let v = run(&mut s, "source.settings", json!({}));
    let sources = v["sources"].as_array().unwrap();
    let used = sources
        .iter()
        .find(|x| x["functions"] == json!(["SharePoint.Files"]))
        .unwrap_or_else(|| panic!("{v}"));
    assert_eq!(used["kind"], "web");
    assert_eq!(used["credential"], "bearer");
    assert_eq!(used["privacy"], Value::Null);
    run(
        &mut s,
        "source.privacy",
        json!({"source": site, "level": "organizational"}),
    );
    let v = run(&mut s, "source.settings", json!({}));
    let mine = v["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["source"] == site.as_str())
        .unwrap()
        .clone();
    assert_eq!(mine["credential"], "bearer");
    assert_eq!(mine["privacy"], "organizational");
    assert!(!v.to_string().contains(fake::SP_TOKEN));
    // forgetting the credential
    let c = run(
        &mut s,
        "source.credentials",
        json!({"kind": "web", "url": srv.url("/sites"), "clear": true}),
    );
    assert_eq!(c["cleared"], true);
    // server mode: credentials are refused, privacy levels and the listing are allowed
    let dir = TempDir::new("engine-sharepoint-server");
    let mut server = Session::server(ServerConfig::new(dir.path()).unwrap());
    run(&mut server, "model.new", json!({"name": "M"}));
    let e = server
        .execute(
            "source.credentials",
            &json!({"kind": "web", "url": site, "authentication": "bearer", "token": "t"}),
        )
        .unwrap_err();
    assert!(e.contains("server"), "{e}");
    run(
        &mut server,
        "source.privacy",
        json!({"source": site, "level": "public"}),
    );
    run(&mut server, "source.settings", json!({}));
    let e = server
        .execute(
            "source.navigate",
            &json!({"kind": "sharepoint_list", "url": site}),
        )
        .unwrap_err();
    assert!(e.contains("network"), "{e}");
}
