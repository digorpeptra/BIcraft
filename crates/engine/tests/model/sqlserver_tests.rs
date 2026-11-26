//! Get data › SQL Server database (lot 11) through `Session::execute`, against the scripted fake
//! TDS server of `bicraft-sqlserver` (feature `fake-server`): credentials kept in memory only,
//! the Navigator's tables and views, preview and load with `allow_connectors: ["sqlserver"]`,
//! refresh, and the refusals (no credentials, readers, server mode without network).

use std::sync::Arc;

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::{ServerConfig, Session, SqlSettings};
use bicraft_sqlserver::fake::{self, FakeServer, Script, tok};
use bicraft_sqlserver::{sql, tls};

const USER: &str = "reader";
const PASSWORD: &str = "s3cret!";

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn orders(rows: &[(i32, i32)]) -> Vec<u8> {
    let mut t = tok::columns(&[("id", tok::INT), ("qty", tok::INT)]);
    for (id, qty) in rows {
        t.extend_from_slice(&tok::row(&[&tok::int(Some(*id)), &tok::int(Some(*qty))]));
    }
    t
}

fn answer(q: &str) -> Option<Vec<u8>> {
    if q == sql::objects(None) {
        let mut t = tok::columns(&[
            ("name", tok::NVARCHAR),
            ("name", tok::NVARCHAR),
            ("type", tok::NVARCHAR),
        ]);
        for (s, o, k) in [("dbo", "Orders", "U "), ("dbo", "Recent", "V ")] {
            t.extend_from_slice(&tok::row(&[
                &tok::nvarchar(Some(s)),
                &tok::nvarchar(Some(o)),
                &tok::nvarchar(Some(k)),
            ]));
        }
        return Some(t);
    }
    Some(match q {
        "SELECT TOP (0) * FROM [dbo].[Orders]" | "SELECT TOP (0) [id], [qty] FROM [dbo].[Orders]" => {
            orders(&[])
        }
        "SELECT [id], [qty] FROM [dbo].[Orders]" => orders(&[(1, 3), (2, 5)]),
        "SELECT TOP (1000) [id], [qty] FROM [dbo].[Orders]" => orders(&[(1, 3), (2, 5)]),
        _ => return None,
    })
}

fn fake_server() -> FakeServer {
    fake::start(Script::new(USER, PASSWORD, Arc::new(answer))).unwrap()
}

/// A session that trusts the fake server's test CA.
fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    s.sql_settings = Some(SqlSettings {
        extra_roots: tls::pem_certificates(fake::CA_PEM).unwrap(),
        ..SqlSettings::default()
    });
    s
}

fn credentials(s: &mut Session, srv: &FakeServer) -> Value {
    run(
        s,
        "source.credentials",
        json!({"server": srv.server(), "user": USER, "password": PASSWORD}),
    )
}

#[test]
fn credentials_then_navigator_preview_load_and_refresh() {
    let srv = fake_server();
    let mut s = session();
    let c = credentials(&mut s, &srv);
    assert_eq!(c["stored"], true);
    // the answer and the journal never carry the password
    assert!(!c.to_string().contains(PASSWORD));
    assert!(!format!("{:?}", s.sql_credentials).contains(PASSWORD));

    let nav = run(
        &mut s,
        "source.navigate",
        json!({"kind": "sqlserver", "server": srv.server(), "database": "Sales"}),
    );
    assert_eq!(nav["kind"], "sqlserver");
    assert_eq!(nav["allow_connectors"], json!(["sqlserver"]));
    let items = nav["items"].as_array().unwrap();
    let names: Vec<&str> = items.iter().filter_map(|i| i["name"].as_str()).collect();
    assert_eq!(names, ["dbo.Orders", "dbo.Recent"]);
    assert_eq!(items[1]["kind"], "View");
    let m = items[0]["m"].as_str().unwrap().to_string();
    assert!(m.contains("Sql.Database("), "{m}");
    assert!(m.contains(r#"[Schema="dbo",Item="Orders"]"#), "{m}");

    let preview = run(
        &mut s,
        "query.preview",
        json!({"expression": m, "allow_connectors": ["sqlserver"]}),
    );
    assert_eq!(preview["rows"], json!([[1, 3], [2, 5]]), "{preview}");

    run(
        &mut s,
        "source.load",
        json!({"tables": [{"name": "Orders", "m": m}], "allow_connectors": ["sqlserver"]}),
    );
    let summary = run(&mut s, "model.summary", json!({}));
    assert!(summary.to_string().contains("Orders"), "{summary}");
    // saved as a project, refreshed: the files hold the query, never the login
    let dir = TempDir::new("engine-sqlserver-save");
    run(&mut s, "file.save_as", json!({"path": dir.join("shop.pbip")}));
    run(
        &mut s,
        "model.refresh",
        json!({"tables": ["Orders"], "allow_connectors": ["sqlserver"]}),
    );
    let rows = run(&mut s, "table.rows", json!({"table": "Orders"}));
    assert_eq!(rows["rows"].as_array().map(Vec::len), Some(2), "{rows}");
    run(&mut s, "file.save", json!({}));
    let mut texts = String::new();
    let mut todo = vec![dir.path().to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                todo.push(p);
            } else {
                texts.push_str(&String::from_utf8_lossy(&std::fs::read(&p).unwrap()));
            }
        }
    }
    assert!(texts.contains("Sql.Database("));
    assert!(!texts.contains(PASSWORD));

    // the logins all used the stored credentials
    assert!(srv.seen().logins.iter().all(|(u, d)| u == USER && d == "Sales"));
}

#[test]
fn without_credentials_or_with_windows_auth_it_says_what_to_do() {
    let srv = fake_server();
    let mut s = session();
    let e = s
        .execute(
            "source.navigate",
            &json!({"kind": "sqlserver", "server": srv.server(), "database": "Sales"}),
        )
        .unwrap_err();
    assert!(e.contains("source.credentials"), "{e}");
    assert!(srv.seen().logins.is_empty());
    let e = s
        .execute(
            "source.credentials",
            &json!({"server": srv.server(), "user": "x", "authentication": "windows"}),
        )
        .unwrap_err();
    assert!(e.contains("Windows"), "{e}");
    // stored then cleared
    credentials(&mut s, &srv);
    let c = run(
        &mut s,
        "source.credentials",
        json!({"server": srv.server(), "user": USER, "clear": true}),
    );
    assert_eq!(c["cleared"], true);
    assert!(s.sql_credentials.get(&srv.server()).is_none());
}

#[test]
fn readers_and_servers_without_network_are_refused() {
    let srv = fake_server();
    let s = session();
    let p = json!({"kind": "sqlserver", "server": srv.server(), "database": "Sales"});
    assert!(s.reader_check("source.navigate", &p).is_err());
    // a connection runs exclusively, never beside other reads of a shared session
    assert!(!s.runs_shared("source.navigate", &p));
    assert!(
        s.reader_check("source.credentials", &json!({"server": "x", "user": "u"}))
            .is_err()
    );
    let dir = TempDir::new("engine-sqlserver-server");
    let mut server = Session::server(ServerConfig::new(dir.path()).unwrap());
    run(&mut server, "model.new", json!({"name": "M"}));
    let e = server
        .execute(
            "source.credentials",
            &json!({"server": srv.server(), "user": USER}),
        )
        .unwrap_err();
    assert!(!e.is_empty());
    let e = server.execute("source.navigate", &p).unwrap_err();
    assert!(e.contains("network"), "{e}");
    assert!(srv.seen().logins.is_empty());
}

#[test]
fn folding_indicators_and_native_queries_of_sql_server_steps() {
    let srv = fake_server();
    let mut s = session();
    credentials(&mut s, &srv);
    let server = srv.server();
    let m = format!(
        "let\n    Source = Sql.Database(\"{server}\", \"Sales\"),\n    \
         dbo_Orders = Source{{[Schema=\"dbo\",Item=\"Orders\"]}}[Data],\n    \
         Kept = Table.FirstN(dbo_Orders, 5),\n    \
         Typed = Table.TransformColumnTypes(Kept, {{{{\"qty\", type text}}}})\nin\n    Typed"
    );
    let v = run(
        &mut s,
        "query.folding",
        json!({"expression": m, "allow_connectors": ["sqlserver"]}),
    );
    let got: Vec<&str> = v["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["folding"].as_str().unwrap())
        .collect();
    assert_eq!(got, ["folded", "folded", "folded", "partial"], "{v}");
    assert_eq!(v["connectors"], json!(["sqlserver"]));
    let n = run(
        &mut s,
        "query.native",
        json!({"expression": m, "step": "Kept", "allow_connectors": ["sqlserver"]}),
    );
    assert_eq!(n["kind"], "sql");
    let sql = n["native"].as_str().unwrap();
    assert!(sql.contains("TOP (5)") && sql.contains("[dbo].[Orders]"), "{n}");
    // a native query is shown as written
    let q = format!("Sql.Database(\"{server}\", \"Sales\", [Query=\"SELECT 1 AS x\"])");
    let n = run(
        &mut s,
        "query.native",
        json!({"expression": q, "allow_connectors": ["sqlserver"]}),
    );
    assert_eq!(n["native"], "SELECT 1 AS x", "{n}");
}
