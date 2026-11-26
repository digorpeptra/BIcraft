//! Get data › Web and OData feed (lot 11) through `Session::execute`, against the in-process
//! test HTTP server of `bicraft-webdata` (feature `fake-server`; never the internet): the
//! Navigator's items, preview and load with `allow_connectors: ["web"]` / `["odata"]`, refresh,
//! PBIP and PBIX round trips, and the refusals (readers, server mode without network, web pages).

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::{ServerConfig, Session};
use bicraft_webdata::fake::{self, FakeHttp};

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    s
}

fn shop() -> FakeHttp {
    fake::start_shop().unwrap()
}

fn files_text(dir: &std::path::Path) -> String {
    let mut texts = String::new();
    let mut todo = vec![dir.to_path_buf()];
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
    texts
}

#[test]
fn web_json_navigator_preview_load_refresh_and_pbip_round_trip() {
    let srv = shop();
    let mut s = session();
    let url = srv.url("/data.json");
    let nav = run(&mut s, "source.navigate", json!({"kind": "web", "url": url}));
    assert_eq!(nav["kind"], "web");
    assert_eq!(nav["allow_connectors"], json!(["web"]));
    let items = nav["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{nav}");
    assert_eq!(items[0]["name"], "data");
    assert_eq!(items[0]["kind"], "List");
    let m = items[0]["m"].as_str().unwrap().to_string();
    assert!(
        m.contains(&format!("Json.Document(Web.Contents(\"{url}\"))")),
        "{m}"
    );
    assert!(m.contains("Table.FromRecords(Source)"), "{m}");
    assert!(m.contains("Table.TransformColumnTypes"), "{m}");

    let preview = run(
        &mut s,
        "query.preview",
        json!({"expression": m, "allow_connectors": ["web"]}),
    );
    assert_eq!(
        preview["rows"],
        json!([[1, "Ann", 9.5], [2, "Bob", 7]]),
        "{preview}"
    );
    // without the connector nothing is read
    assert!(s.execute("query.preview", &json!({"expression": m})).is_err());

    run(
        &mut s,
        "source.load",
        json!({"tables": [{"name": "People", "m": m}], "allow_connectors": ["web"]}),
    );
    let dir = TempDir::new("engine-web-save");
    let path = dir.join("shop.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    run(
        &mut s,
        "model.refresh",
        json!({"tables": ["People"], "allow_connectors": ["web"]}),
    );
    let rows = run(&mut s, "table.rows", json!({"table": "People"}));
    assert_eq!(rows["rows"].as_array().map(Vec::len), Some(2), "{rows}");
    run(&mut s, "file.save", json!({}));
    assert!(files_text(dir.path()).contains("Web.Contents("));
    // reopened: the query and the refreshed rows are there
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": path}));
    let rows = run(&mut again, "table.rows", json!({"table": "People"}));
    assert_eq!(rows["rows"].as_array().map(Vec::len), Some(2), "{rows}");
}

#[test]
fn web_csv_lists_as_a_text_file() {
    let srv = shop();
    let mut s = session();
    let nav = run(
        &mut s,
        "source.navigate",
        json!({"kind": "web", "url": srv.url("/data.csv")}),
    );
    let m = nav["items"][0]["m"].as_str().unwrap().to_string();
    assert!(m.contains("Csv.Document(Web.Contents("), "{m}");
    assert!(m.contains("Table.PromoteHeaders"), "{m}");
    let preview = run(
        &mut s,
        "query.preview",
        json!({"expression": m, "allow_connectors": ["web"]}),
    );
    assert_eq!(preview["rows"], json!([[1, "Ann"], [2, "Bob"]]), "{preview}");
}

#[test]
fn odata_navigator_preview_load_and_pbix_round_trip() {
    let srv = shop();
    let mut s = session();
    let url = srv.url("/odata/");
    let nav = run(&mut s, "source.navigate", json!({"kind": "odata", "url": url}));
    assert_eq!(nav["allow_connectors"], json!(["odata"]));
    let items = nav["items"].as_array().unwrap();
    let names: Vec<&str> = items.iter().filter_map(|i| i["name"].as_str()).collect();
    assert_eq!(names, ["Products", "Categories"]);
    let m = items[0]["m"].as_str().unwrap().to_string();
    assert!(
        m.contains(&format!("OData.Feed(\"{url}\", null, [Implementation=\"2.0\"])")),
        "{m}"
    );
    assert!(
        m.contains(r#"Products_table = Source{[Name="Products",Signature="table"]}[Data]"#),
        "{m}"
    );

    let preview = run(
        &mut s,
        "query.preview",
        json!({"expression": m, "limit": 3, "allow_connectors": ["odata"]}),
    );
    let cols: Vec<&str> = preview["columns"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert_eq!(cols, ["ID", "Name", "Price", "InStock", "Released"]);
    assert_eq!(preview["rows"].as_array().map(Vec::len), Some(3), "{preview}");
    // the preview's first rows fold into $top
    let tops: Vec<String> = srv
        .seen()
        .iter()
        .filter(|r| r.path() == "/odata/Products")
        .map(|r| r.decoded())
        .collect();
    assert!(tops.iter().any(|t| t.contains("$top=4")), "{tops:?}");

    run(
        &mut s,
        "source.load",
        json!({"tables": [{"name": "Products", "m": m}], "allow_connectors": ["odata"]}),
    );
    // saved as a PBIX without rows (Desktop loads them on refresh), refreshed (every page
    // read through the nextLinks), saved again: the PBIX carries the rows
    let dir = TempDir::new("engine-odata-pbix");
    let path = dir.join("shop.pbix");
    run(
        &mut s,
        "file.save_as",
        json!({"path": path, "empty_tables": true}),
    );
    run(
        &mut s,
        "model.refresh",
        json!({"tables": ["Products"], "allow_connectors": ["odata"]}),
    );
    run(&mut s, "file.save", json!({}));
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": path}));
    let rows = run(&mut again, "table.rows", json!({"table": "Products"}));
    assert_eq!(rows["rows"].as_array().map(Vec::len), Some(5), "{rows}");
    let parts = run(&mut again, "table.get", json!({"name": "Products"}));
    assert!(parts.to_string().contains("OData.Feed("), "{parts}");
}

#[test]
fn refusals() {
    let srv = shop();
    let mut s = session();
    // a web page is not read
    let e = s
        .execute(
            "source.navigate",
            &json!({"kind": "web", "url": srv.url("/page.html")}),
        )
        .unwrap_err();
    assert!(e.contains("HTML"), "{e}");
    for p in [
        json!({"kind": "web"}),
        json!({"kind": "web", "url": "ftp://example.com/x"}),
        json!({"kind": "odata", "url": srv.url("/odata/"), "path": "x"}),
        json!({"url": srv.url("/data.json")}),
    ] {
        assert!(s.execute("source.navigate", &p).is_err(), "{p}");
    }
    // a 404 says so
    let e = s
        .execute(
            "source.navigate",
            &json!({"kind": "web", "url": srv.url("/missing")}),
        )
        .unwrap_err();
    assert!(e.contains("404"), "{e}");

    let p = json!({"kind": "odata", "url": srv.url("/odata/")});
    assert!(s.reader_check("source.navigate", &p).is_err());
    let before = srv.seen().len();
    let dir = TempDir::new("engine-web-server");
    let mut server = Session::server(ServerConfig::new(dir.path()).unwrap());
    run(&mut server, "model.new", json!({"name": "M"}));
    let e = server.execute("source.navigate", &p).unwrap_err();
    assert!(e.contains("network"), "{e}");
    let e = server
        .execute(
            "query.preview",
            &json!({"expression": "Web.Contents(\"http://127.0.0.1:9/\")", "allow_connectors": ["web"]}),
        )
        .unwrap_err();
    assert!(e.contains("network"), "{e}");
    assert_eq!(srv.seen().len(), before);
}
