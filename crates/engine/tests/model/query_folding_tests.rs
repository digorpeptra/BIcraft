//! Folding indicators and View native query (`query.folding`, `query.native`) through
//! `Session::execute`, against the in-process OData test server of `bicraft-webdata` (feature
//! `fake-server`; never the internet). The SQL Server cases are in `sqlserver_tests.rs`, next to
//! its scripted fake TDS server.

use serde_json::{Value, json};

use bicraft_engine::Session;
use bicraft_webdata::fake;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Products of the OData shop: navigation, two folded steps, a rename that runs locally.
fn products(url: &str) -> String {
    format!(
        "let\n    Source = OData.Feed(\"{url}\", null, [Implementation=\"2.0\"]),\n    \
         Products = Source{{[Name=\"Products\",Signature=\"table\"]}}[Data],\n    \
         Kept = Table.SelectColumns(Products, {{\"ID\", \"Name\", \"Price\"}}),\n    \
         Big = Table.SelectRows(Kept, each [ID] > 2),\n    \
         Renamed = Table.RenameColumns(Big, {{{{\"Name\", \"Product\"}}}})\nin\n    Renamed"
    )
}

fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    s
}

fn indicators(v: &Value) -> Vec<(String, String)> {
    v["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x["name"].as_str().unwrap().to_string(),
                x["folding"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn odata_steps_fold_until_a_rename_and_read_no_rows() {
    let srv = fake::start_shop().unwrap();
    let mut s = session();
    let m = products(&srv.url("/odata/"));
    let v = run(
        &mut s,
        "query.folding",
        json!({"expression": m, "allow_connectors": ["odata"]}),
    );
    assert_eq!(
        indicators(&v),
        [
            ("Source".to_string(), "folded".to_string()),
            ("Products".into(), "folded".into()),
            ("Kept".into(), "folded".into()),
            ("Big".into(), "folded".into()),
            ("Renamed".into(), "partial".into()),
        ],
        "{v}"
    );
    assert_eq!(v["connectors"], json!(["odata"]));
    let steps = v["steps"].as_array().unwrap();
    assert_eq!(steps[0]["native_available"], false, "a navigation table: {v}");
    assert_eq!(steps[3]["native_available"], true);
    assert_eq!(steps[4]["native_available"], false);
    assert_eq!(steps[3]["connector"], "odata");
    // no entity was read: only the service document and $metadata
    assert!(
        srv.seen().iter().all(|r| r.path() != "/odata/Products"),
        "{:?}",
        srv.seen().iter().map(|r| r.decoded()).collect::<Vec<_>>()
    );

    let n = run(
        &mut s,
        "query.native",
        json!({"expression": m, "step": "Big", "allow_connectors": ["odata"]}),
    );
    assert_eq!(n["kind"], "url");
    assert_eq!(n["folding"], "folded");
    let url = n["native"].as_str().unwrap();
    assert!(url.starts_with(&srv.url("/odata/Products?")), "{n}");
    assert!(url.contains("$select=ID,Name,Price"), "{url}");
    assert!(
        url.contains("$filter=ID%20gt%202") || url.contains("$filter=ID gt 2"),
        "{url}"
    );
    // the step that does not fold: no text, a reason
    let n = run(
        &mut s,
        "query.native",
        json!({"expression": m, "step": "Renamed", "allow_connectors": ["odata"]}),
    );
    assert_eq!(n["native"], Value::Null);
    assert_eq!(n["kind"], Value::Null);
    assert!(n["reason"].as_str().unwrap().contains("does not fold"), "{n}");
    // the default step is the result
    let n = run(
        &mut s,
        "query.native",
        json!({"expression": m, "allow_connectors": ["odata"]}),
    );
    assert_eq!(n["step"], "Renamed");
    assert!(srv.seen().iter().all(|r| r.path() != "/odata/Products"));
}

#[test]
fn without_the_connector_steps_are_unknown_and_local_queries_do_not_fold() {
    let srv = fake::start_shop().unwrap();
    let mut s = session();
    let m = products(&srv.url("/odata/"));
    let v = run(&mut s, "query.folding", json!({"expression": m}));
    let steps = v["steps"].as_array().unwrap();
    assert!(steps.iter().all(|x| x["folding"] == "unknown"), "{v}");
    assert!(
        steps[0]["reason"].as_str().unwrap().contains("allow_connectors"),
        "{v}"
    );
    assert_eq!(v["connectors"], json!(["odata"]));
    assert!(srv.seen().is_empty(), "nothing was sent");

    let local = "let\n    Source = #table({\"a\"}, {{1}, {2}}),\n    Kept = Table.SelectRows(Source, each [a] > 1)\nin\n    Kept";
    let v = run(&mut s, "query.folding", json!({"expression": local}));
    assert_eq!(
        indicators(&v),
        [
            ("Source".to_string(), "not_folded".to_string()),
            ("Kept".into(), "not_folded".into())
        ]
    );
    assert_eq!(v["connectors"], json!([]));
    // a model query that reads a connector query: its family is found through it
    run(
        &mut s,
        "expression.add",
        json!({"name": "Feed", "expression": format!("OData.Feed(\"{}\")", srv.url("/odata/"))}),
    );
    let v = run(
        &mut s,
        "query.folding",
        json!({"expression": "let\n    Source = Feed\nin\n    Source"}),
    );
    assert_eq!(v["connectors"], json!(["odata"]), "{v}");
}

#[test]
fn bad_parameters_are_errors() {
    let mut s = session();
    for (id, p) in [
        ("query.folding", json!({})),
        ("query.folding", json!({"expression": "1", "bogus": 1})),
        ("query.folding", json!({"expression": "1", "timeout_ms": 0})),
        (
            "query.folding",
            json!({"expression": "1", "allow_connectors": ["nope"]}),
        ),
        (
            "query.native",
            json!({"expression": "let A = 1 in A", "step": "B"}),
        ),
        ("query.native", json!({"expression": "let A = in A"})),
        ("query.native", json!({"name": "Missing"})),
    ] {
        assert!(s.execute(id, &p).is_err(), "{id} {p}");
    }
}

#[test]
fn diagnose_times_the_steps_and_lists_the_source_requests() {
    let srv = fake::start_shop().unwrap();
    let mut s = session();
    let m = products(&srv.url("/odata/"));
    let v = run(
        &mut s,
        "query.diagnose",
        json!({"expression": m, "step": "Big", "allow_connectors": ["odata"]}),
    );
    let st = &v["steps"][0];
    assert_eq!(st["name"], "Big");
    assert_eq!(st["rows"], 3, "{v}");
    assert_eq!(st["columns"], 3);
    assert_eq!(st["error"], Value::Null);
    assert!(st["elapsed_ms"].is_u64());
    let requests = st["requests"].as_array().unwrap();
    let queries: Vec<&Value> = requests.iter().filter(|r| r["kind"] == "query").collect();
    assert_eq!(queries.len(), 1, "{v}");
    assert_eq!(queries[0]["connector"], "odata");
    assert_eq!(queries[0]["rows"], 3);
    let text = queries[0]["text"].as_str().unwrap();
    assert!(text.contains("$select=ID,Name,Price"), "{text}");
    // every step in turn; the default is the result
    let v = run(
        &mut s,
        "query.diagnose",
        json!({"expression": m, "all_steps": true, "allow_connectors": ["odata"]}),
    );
    assert_eq!(v["steps"].as_array().unwrap().len(), 5);
    let v = run(
        &mut s,
        "query.diagnose",
        json!({"expression": m, "allow_connectors": ["odata"]}),
    );
    assert_eq!(v["steps"][0]["name"], "Renamed");
    // a local query sends nothing; an error is the step's
    let v = run(
        &mut s,
        "query.diagnose",
        json!({"expression": "let A = #table({\"a\"}, {{1}}), B = error \"boom\" in B", "all_steps": true}),
    );
    assert_eq!(v["steps"][0]["requests_total"], 0);
    assert_eq!(v["steps"][0]["rows"], 1);
    assert!(v["steps"][1]["error"].as_str().unwrap().contains("boom"), "{v}");
    for p in [
        json!({"expression": m, "step": "Big", "all_steps": true}),
        json!({"expression": m, "step": "Nope"}),
        json!({"expression": m, "all_steps": "yes"}),
    ] {
        assert!(s.execute("query.diagnose", &p).is_err(), "{p}");
    }
}
