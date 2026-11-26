//! `query.dependencies` (`query_deps.rs`): the graph of queries, sources and model tables, from
//! the model or the editor's drafts.

use serde_json::{Value, json};

use super::*;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Folder (a parameter), Raw (a CSV in it, not loaded), Sales (reads Raw, loaded), Cities (a
/// literal table, loaded).
fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Cities", "m": "#table({\"City\"}, {{\"Paris\"}})"}),
    );
    let m = s.model.as_mut().unwrap();
    for (name, e) in [
        (
            "Folder",
            "\"C:\\data\" meta [IsParameterQuery=true, Type=\"Text\", IsParameterQueryRequired=true]",
        ),
        (
            "Raw",
            "let\n    Source = Csv.Document(File.Contents(Folder & \"\\sales.csv\"))\nin\n    Source",
        ),
    ] {
        m.expressions.push(bicraft_model::NamedExpression {
            name: name.into(),
            kind: "m".into(),
            expression: e.into(),
            ..Default::default()
        });
    }
    if let Some(t) = m.tables.iter_mut().find(|t| t.name == "Cities")
        && let Some(p) = t.partitions.first_mut()
    {
        p.expression = Some("let\n    Source = #table({\"City\"}, {{\"Paris\"}}),\n    J = Table.NestedJoin(Source, {\"City\"}, Raw, {\"City\"}, \"Raw\")\nin\n    J".into());
    }
    s.dirty = false;
    s
}

fn node<'a>(v: &'a Value, id: &str) -> &'a Value {
    v["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == id)
        .unwrap_or_else(|| panic!("no node {id}: {v}"))
}

fn has_edge(v: &Value, from: &str, to: &str, kind: &str) -> bool {
    v["edges"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["from"] == from && e["to"] == to && e["kind"] == kind)
}

#[test]
fn the_graph_links_sources_queries_and_the_tables_they_load_into() {
    let mut s = session();
    let v = run(&mut s, "query.dependencies", json!({}));
    assert_eq!(node(&v, "query:Folder")["kind"], "parameter");
    assert_eq!(node(&v, "query:Raw")["kind"], "query");
    assert_eq!(node(&v, "query:Cities")["kind"], "table");
    assert_eq!(node(&v, "table:Cities")["kind"], "model_table");
    let src = v["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"] == "source")
        .expect("a source node");
    assert_eq!(src["function"], "File.Contents");
    assert!(src["name"].as_str().unwrap().contains("[Folder]"), "{src}");
    let sid = src["id"].as_str().unwrap();
    assert!(has_edge(&v, sid, "query:Raw", "source"));
    assert!(has_edge(&v, "query:Folder", "query:Raw", "reference"));
    assert!(has_edge(&v, "query:Raw", "query:Cities", "reference"));
    assert!(has_edge(&v, "query:Cities", "table:Cities", "load"));
    // left to right: the source, then each query after what it reads, the tables last
    let layer = |id: &str| node(&v, id)["layer"].as_u64().unwrap();
    assert_eq!(src["layer"], 0);
    assert!(layer("query:Raw") > layer("query:Folder"));
    assert!(layer("query:Cities") > layer("query:Raw"));
    assert!(layer("table:Cities") > layer("query:Cities"));
    assert_eq!(v["layers"].as_u64().unwrap(), layer("table:Cities") + 1);
    assert!(v["unparsed"].as_array().unwrap().is_empty());
    assert!(!s.dirty);
}

#[test]
fn the_editors_drafts_replace_the_models_queries() {
    let mut s = session();
    let v = run(
        &mut s,
        "query.dependencies",
        json!({"queries": [
            {"name": "Cities", "expression": "#table({\"City\"}, {})"},
            {"name": "Raw", "expression": "let Source = Web.Contents(\"https://example.test/a.json\") in Source"},
            {"name": "Query1", "expression": "let Source = Raw in Source", "load": false},
            {"name": "Broken", "expression": "let A = in A"},
        ]}),
    );
    // Cities reads nothing now; Raw is a shared query of the model: not loaded
    assert!(!has_edge(&v, "query:Raw", "query:Cities", "reference"));
    assert_eq!(node(&v, "query:Raw")["kind"], "query");
    assert_eq!(node(&v, "query:Query1")["kind"], "query");
    assert!(has_edge(&v, "query:Raw", "query:Query1", "reference"));
    assert!(has_edge(
        &v,
        "source:Web.Contents: https://example.test/a.json",
        "query:Raw",
        "source"
    ));
    assert_eq!(v["unparsed"][0]["name"], "Broken");
    // a new draft that is loaded gets its model table
    assert_eq!(node(&v, "table:Broken")["kind"], "model_table");
}

#[test]
fn a_cycle_does_not_loop() {
    let mut s = session();
    let v = run(
        &mut s,
        "query.dependencies",
        json!({"queries": [
            {"name": "A", "expression": "B"},
            {"name": "B", "expression": "A"},
        ]}),
    );
    assert!(has_edge(&v, "query:A", "query:B", "reference"));
    assert!(has_edge(&v, "query:B", "query:A", "reference"));
}

#[test]
fn hostile_dependencies_params_are_errors() {
    let mut s = session();
    for bad in [
        json!({"queries": 3}),
        json!({"queries": [{"name": "A"}]}),
        json!({"queries": [{"name": "A", "expression": "1", "x": 1}]}),
        json!({"queries": [{"name": "A", "expression": "1"}, {"name": "A", "expression": "2"}]}),
        json!({"queries": [{"name": "A", "expression": "1", "load": "yes"}]}),
        json!({"other": 1}),
    ] {
        assert!(s.execute("query.dependencies", &bad).is_err(), "{bad}");
    }
    assert!(Session::new().execute("query.dependencies", &json!({})).is_err());
}
