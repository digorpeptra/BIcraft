//! `relationship.autodetect` (Manage relationships › Autodetect) on synthetic models: the
//! candidates from matching names, types and the data (many-to-one, one-to-one, none without a
//! unique side or without data), no ambiguous path, the preview that changes nothing, the apply
//! of all or some in one undo step, and PBIP (TMDL) and PBIX saves of what it added.

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn import(s: &mut Session, name: &str, m: &str) {
    run(s, "table.import", json!({"name": name, "m": m}));
}

/// Sales(ProductKey, Region, Amount), Product(ProductKey, Name), Region(Region, Manager),
/// Budget(Project, Amount: text, so no match with Sales[Amount]) and Priority(Project).
fn shop() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    import(
        &mut s,
        "Sales",
        "#table(type table [ProductKey = number, Region = text, Amount = number], {{1, \"E\", 10}, {1, \"W\", 5}, {2, \"E\", 7}})",
    );
    import(
        &mut s,
        "Product",
        "#table(type table [ProductKey = number, Name = text], {{1, \"Bike\"}, {2, \"Car\"}, {3, \"Kite\"}})",
    );
    import(
        &mut s,
        "Region",
        "#table(type table [Region = text, Manager = text], {{\"E\", \"a\"}, {\"W\", \"b\"}})",
    );
    import(
        &mut s,
        "Budget",
        "#table(type table [Project = text, Amount = text], {{\"Blue\", \"x\"}, {\"Red\", \"y\"}})",
    );
    import(
        &mut s,
        "Priority",
        "#table(type table [Project = text, Level = text], {{\"Blue\", \"A\"}, {\"Red\", \"B\"}, {\"Green\", \"C\"}})",
    );
    s
}

fn pairs(v: &Value, key: &str) -> Vec<(String, String)> {
    v[key]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            (
                c["from"].as_str().unwrap_or_default().to_string(),
                c["to"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

fn p(a: &str, b: &str) -> (String, String) {
    (a.to_string(), b.to_string())
}

#[test]
fn the_preview_lists_candidates_and_changes_nothing() {
    let mut s = shop();
    let before = s.model.clone();
    let history = run(&mut s, "edit.history", json!({}));
    let v = run(&mut s, "relationship.autodetect", json!({}));
    assert_eq!(v["applied"], false);
    assert_eq!(
        pairs(&v, "candidates"),
        [
            p("Sales[ProductKey]", "Product[ProductKey]"),
            p("Sales[Region]", "Region[Region]"),
            p("Budget[Project]", "Priority[Project]"),
        ]
    );
    let c = &v["candidates"];
    assert_eq!(
        (
            c[0]["from_cardinality"].clone(),
            c[0]["to_cardinality"].clone(),
            c[0]["cross_filter"].clone()
        ),
        (json!("many"), json!("one"), json!("single"))
    );
    // both sides unique: one-to-one, both directions
    assert_eq!(
        (c[2]["from_cardinality"].clone(), c[2]["cross_filter"].clone()),
        (json!("one"), json!("both"))
    );
    // the model and the undo history are as they were
    assert_eq!(s.model, before);
    assert_eq!(run(&mut s, "edit.history", json!({})), history);
}

#[test]
fn apply_adds_them_in_one_undo_step() {
    let mut s = shop();
    let v = run(&mut s, "relationship.autodetect", json!({"apply": true}));
    assert_eq!(v["applied"], true);
    assert_eq!(v["added"].as_array().map(Vec::len), Some(3));
    let m = s.model.as_ref().unwrap();
    assert_eq!(m.relationships.len(), 3);
    let r = &m.relationships[2];
    assert_eq!(r.cross_filtering_behavior.as_deref(), Some("bothDirections"));
    assert_eq!(r.from_cardinality.as_deref(), Some("one"));
    assert_eq!(r.name.len(), 36, "a GUID name, as Desktop names them");
    // nothing left to find
    let again = run(&mut s, "relationship.autodetect", json!({}));
    assert_eq!(pairs(&again, "candidates"), []);
    assert!(
        s.execute("relationship.autodetect", &json!({"apply": true}))
            .is_err()
    );
    run(&mut s, "edit.undo", json!({}));
    assert!(s.model.as_ref().unwrap().relationships.is_empty());
}

#[test]
fn only_adds_the_ones_named() {
    let mut s = shop();
    let v = run(
        &mut s,
        "relationship.autodetect",
        json!({"apply": true, "only": [{"from": "sales[region]", "to": "Region[Region]"}]}),
    );
    assert_eq!(pairs(&v, "added"), [p("Sales[Region]", "Region[Region]")]);
    assert_eq!(s.model.as_ref().unwrap().relationships.len(), 1);
    let e = s
        .execute(
            "relationship.autodetect",
            &json!({"apply": true, "only": [{"from": "Sales[Amount]", "to": "Budget[Amount]"}]}),
        )
        .unwrap_err();
    assert!(e.contains("not an autodetected relationship"), "{e}");
    assert!(
        s.execute("relationship.autodetect", &json!({"only": "x"}))
            .is_err()
    );
}

#[test]
fn no_unique_side_no_data_or_another_type_gives_no_candidate() {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "M"}));
    import(&mut s, "A", "#table(type table [K = number], {{1}, {1}})");
    import(&mut s, "B", "#table(type table [K = number], {{2}, {2}})");
    // C[K] is text: no match with A[K] or B[K]
    import(&mut s, "C", "#table(type table [K = text], {{\"1\"}})");
    // tables without data
    run(&mut s, "table.add", json!({"name": "D"}));
    run(&mut s, "table.add", json!({"name": "E"}));
    run(
        &mut s,
        "column.add",
        json!({"table": "D", "name": "Id", "data_type": "int64"}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "E", "name": "Id", "data_type": "int64"}),
    );
    let v = run(&mut s, "relationship.autodetect", json!({}));
    assert_eq!(pairs(&v, "candidates"), []);
    let reasons: Vec<(String, String, String)> = v["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x["from"].as_str().unwrap().to_string(),
                x["to"].as_str().unwrap().to_string(),
                x["reason"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(reasons.len(), 2, "{reasons:?}");
    assert_eq!((reasons[0].0.as_str(), reasons[0].1.as_str()), ("A[K]", "B[K]"));
    assert!(reasons[0].2.contains("unique"), "{reasons:?}");
    assert_eq!((reasons[1].0.as_str(), reasons[1].1.as_str()), ("D[Id]", "E[Id]"));
    assert!(reasons[1].2.contains("no data"), "{reasons:?}");
}

#[test]
fn a_candidate_that_makes_paths_ambiguous_is_left_out() {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "M"}));
    import(
        &mut s,
        "Sales",
        "#table(type table [K = number, R = text], {{1, \"E\"}, {1, \"W\"}})",
    );
    import(
        &mut s,
        "Product",
        "#table(type table [K = number, R = text], {{1, \"E\"}, {2, \"E\"}})",
    );
    import(
        &mut s,
        "Region",
        "#table(type table [R = text], {{\"E\"}, {\"W\"}})",
    );
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[K]", "to": "Product[K]"}),
    );
    let v = run(&mut s, "relationship.autodetect", json!({}));
    // Region → Sales is found first; Region → Product would give Region two paths to Sales
    assert_eq!(pairs(&v, "candidates"), [p("Sales[R]", "Region[R]")]);
    assert_eq!(pairs(&v, "skipped"), [p("Product[R]", "Region[R]")]);
    assert!(
        v["skipped"][0]["reason"].as_str().unwrap().contains("ambiguous"),
        "{v}"
    );
}

#[test]
fn what_it_adds_is_saved_to_pbip_and_pbix() {
    let tmp = TempDir::new("engine-autodetect-save");
    for (format, file) in [("pbip-tmdl", "Shop.pbip"), ("pbix", "shop.pbix")] {
        let mut s = shop();
        run(&mut s, "relationship.autodetect", json!({"apply": true}));
        let path = tmp.path().join(file);
        run(&mut s, "file.save_as", json!({"path": path, "format": format}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        assert_eq!(back.model_error, None, "{format}");
        let list = run(&mut back, "relationship.list", json!({}));
        let rels = list
            .get("relationships")
            .and_then(Value::as_array)
            .cloned()
            .or_else(|| list.as_array().cloned())
            .unwrap_or_default();
        let mut got: Vec<(String, String, String, String)> = rels
            .iter()
            .map(|r| {
                let t = |k: &str| r[k].as_str().unwrap_or_default().to_string();
                (t("from"), t("to"), t("from_cardinality"), t("cross_filter"))
            })
            .collect();
        got.sort();
        assert_eq!(got.len(), 3, "{format}: {list}");
        assert!(
            got.contains(&(
                "Budget[Project]".into(),
                "Priority[Project]".into(),
                "one".into(),
                "both".into()
            )),
            "{format}: {got:?}"
        );
        // reopened, autodetect finds nothing new
        let v = run(&mut back, "relationship.autodetect", json!({}));
        assert_eq!(pairs(&v, "candidates"), [], "{format}");
    }
}
