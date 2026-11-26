//! Tests of the slicer selections a bookmark captures (`singleVisual.objects` of each slicer's
//! container): captured as the session shows them (a reader's click included), restored by
//! `bookmark.apply` (into the report) and by `view.interact {action: "bookmark"}` (into the
//! reader's view only), within the Data option, and kept through PBIR, PBIX and a legacy
//! layout.

use serde_json::{Value, json};

use crate::Session;
use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// One page with a slicer on `Sales[Category]` and a card of `[Total]` (Wheels 100, Toys 5).
fn session() -> (Session, String, String) {
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Amount"}, {{"Wheels", 100}, {"Toys", 5}})"#;
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Category", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
    ] {
        run(&mut s, id, p);
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let slicer = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "slicer", "x": 20, "y": 20, "width": 220,
               "height": 120, "fields": [{"role": "Values", "field": "Sales[Category]"}]}),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "card", "x": 260, "y": 20, "width": 200,
               "height": 120, "fields": [{"role": "Values", "field": "[Total]"}]}),
    );
    (s, page, slicer)
}

fn click(s: &mut Session, page: &str, slicer: &str, value: &str) {
    run(
        s,
        "view.interact",
        json!({"action": "slicer_toggle", "page": page, "visual": slicer, "value": value}),
    );
}

/// The snapshot's slicer selection and card value.
fn shown(s: &Session) -> (Value, Value) {
    let snap = s.snapshot();
    let visuals = snap["report"]["pages"][0]["visuals"].as_array().unwrap().clone();
    let of = |ty: &str| visuals.iter().find(|v| v["type"] == json!(ty)).cloned().unwrap();
    let sel = of("slicer")["slicer"]["selected"].clone();
    let sel = if sel.is_null() { json!([]) } else { sel };
    (sel, of("card")["data"]["rows"].clone())
}

/// The slicer's container in bookmark `i`.
fn container(s: &Session, i: usize, page: &str, slicer: &str) -> Value {
    s.report.as_ref().unwrap().bookmarks[i].document["explorationState"]["sections"][page]["visualContainers"]
        [slicer]
        .clone()
}

/// The filter the report stores for the slicer.
fn stored_filter(s: &Session, slicer: &str) -> Option<Value> {
    let r = s.report.as_ref().unwrap();
    let v = r.pages[0].visuals.iter().find(|v| v.name == slicer).unwrap();
    crate::preview::filters::slicer_filter(v).cloned()
}

/// A reader's click is captured; `bookmark.apply` writes it into the slicer and drops a later
/// click; a bookmark of no selection clears it; without Data the slicer is left alone.
#[test]
fn bookmarks_capture_and_restore_slicer_selections() {
    let (mut s, page, slicer) = session();
    click(&mut s, &page, &slicer, "Toys");
    assert_eq!(shown(&s), (json!(["Toys"]), json!([[5]])));
    run(&mut s, "bookmark.capture", json!({"display_name": "Toys"}));
    let c = container(&s, 0, &page, &slicer);
    assert_eq!(c["singleVisual"]["visualType"], json!("slicer"), "{c}");
    let f = c["singleVisual"]["objects"]["merge"]["general"][0]["properties"]["filter"]["filter"].clone();
    assert!(f.is_object(), "the selection is captured: {c}");
    assert!(f.to_string().contains("Toys"), "{f}");
    // the capture did not write the selection into the report
    assert_eq!(stored_filter(&s, &slicer), None);

    // nothing selected: the bookmark removes the filter
    click(&mut s, &page, &slicer, "Toys");
    assert_eq!(shown(&s).0, json!([]));
    run(&mut s, "bookmark.capture", json!({"display_name": "All"}));
    let c = container(&s, 1, &page, &slicer);
    assert_eq!(
        c["singleVisual"]["objects"]["remove"],
        json!([{"object": "general", "property": "filter"}]),
        "{c}"
    );
    assert!(c["singleVisual"]["objects"].get("merge").is_none(), "{c}");

    // another click, then View: the bookmark's selection comes back, written into the slicer
    click(&mut s, &page, &slicer, "Wheels");
    assert_eq!(shown(&s), (json!(["Wheels"]), json!([[100]])));
    let out = run(&mut s, "bookmark.apply", json!({"bookmark": "Toys"}));
    assert_eq!(out["changed"], json!(true), "{out}");
    assert_eq!(out["view_changed"], json!(true), "{out}");
    assert_eq!(out["slicers"], json!([{"page": page, "visual": slicer}]), "{out}");
    assert_eq!(stored_filter(&s, &slicer), Some(f.clone()));
    assert_eq!(shown(&s), (json!(["Toys"]), json!([[5]])));
    // again: nothing changes
    let out = run(&mut s, "bookmark.apply", json!({"bookmark": "Toys"}));
    assert_eq!(out["changed"], json!(false), "{out}");

    // the bookmark of no selection clears it
    run(&mut s, "bookmark.apply", json!({"bookmark": "All"}));
    assert_eq!(stored_filter(&s, &slicer), None);
    assert_eq!(shown(&s), (json!([]), json!([[105]])));

    // without Data, the selection is not applied
    run(&mut s, "bookmark.set", json!({"bookmark": "Toys", "data": false}));
    let out = run(&mut s, "bookmark.apply", json!({"bookmark": "Toys"}));
    assert_eq!(out["slicers"], json!([]), "{out}");
    assert_eq!(stored_filter(&s, &slicer), None);

    // Update captures the selection shown now
    run(&mut s, "bookmark.set", json!({"bookmark": "Toys", "data": true}));
    click(&mut s, &page, &slicer, "Wheels");
    let out = run(&mut s, "bookmark.update", json!({"bookmark": "All"}));
    assert_eq!(out["changed"], json!(true), "{out}");
    let c = container(&s, 1, &page, &slicer);
    assert!(
        c.to_string().contains("Wheels"),
        "the update holds the new selection: {c}"
    );
}

/// The reader's bookmark sets the slicer's selection in the view only: the report, its
/// revision and dirty flag stay; a pending selection is dropped.
#[test]
fn a_reader_bookmark_restores_the_slicer_in_the_view() {
    let (mut s, page, slicer) = session();
    click(&mut s, &page, &slicer, "Toys");
    run(&mut s, "bookmark.capture", json!({"display_name": "Toys"}));
    click(&mut s, &page, &slicer, "Wheels");
    s.report_dirty = false;
    let before = s.report.clone();
    let out = run(
        &mut s,
        "view.interact",
        json!({"action": "bookmark", "bookmark": "Toys"}),
    );
    assert_eq!(out["slicers"], json!([{"page": page, "visual": slicer}]), "{out}");
    assert_eq!(shown(&s), (json!(["Toys"]), json!([[5]])));
    assert_eq!(s.report, before, "the report is not changed");
    assert!(!s.report_dirty);
    // a click goes on from the bookmark's selection (Ctrl adds Wheels)
    run(
        &mut s,
        "view.interact",
        json!({"action": "slicer_toggle", "page": page, "visual": slicer, "value": "Wheels",
               "ctrl": true}),
    );
    assert_eq!(shown(&s), (json!(["Toys", "Wheels"]), json!([[105]])));
    // applied again, the bookmark brings Toys back; a stored selection equal to it is no
    // override
    run(
        &mut s,
        "view.interact",
        json!({"action": "bookmark", "bookmark": "Toys"}),
    );
    assert_eq!(shown(&s).0, json!(["Toys"]));
    run(&mut s, "bookmark.apply", json!({"bookmark": "Toys"}));
    let state = run(&mut s, "view.interactions", json!({}));
    assert_eq!(
        state["slicers"].as_array().map_or(usize::MAX, Vec::len),
        0,
        "no override left: {state}"
    );
}

/// The captured selection is kept by PBIP (PBIR) and PBIX, and applied after reopening.
#[test]
fn slicer_selections_round_trip_through_pbir_and_pbix() {
    let dir = TempDir::new("bookmarks-slicer-round-trip");
    let (mut s, page, slicer) = session();
    click(&mut s, &page, &slicer, "Toys");
    run(&mut s, "bookmark.capture", json!({"display_name": "Toys"}));
    run(&mut s, "view.interact", json!({"action": "reset"}));
    let expected = container(&s, 0, &page, &slicer);
    for path in [dir.join("pbip").join("Shop.pbip"), dir.join("Shop.pbix")] {
        run(&mut s, "file.save_as", json!({"path": path}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        assert_eq!(container(&back, 0, &page, &slicer), expected, "{path:?}");
        assert_eq!(shown(&back).0, json!([]), "{path:?}");
        run(
            &mut back,
            "view.interact",
            json!({"action": "bookmark", "bookmark": "Toys"}),
        );
        assert_eq!(shown(&back), (json!(["Toys"]), json!([[5]])), "{path:?}");
    }
}

/// A legacy layout's bookmark (`config.bookmarks`) with a slicer's selection: read verbatim,
/// applied into the slicer's `objects`, written back into the legacy layout.
#[test]
fn a_legacy_layout_bookmark_restores_its_slicer() {
    const METADATA: &[u8] = include_bytes!("../../metadata/tests/fixtures/synthetic_metadata.sqlitedb");
    let filter = json!({"Version": 2, "From": [{"Name": "s", "Entity": "Sales", "Type": 0}],
        "Where": [{"Condition": {"In": {"Expressions": [{"Column": {"Expression":
        {"SourceRef": {"Source": "s"}}, "Property": "Category"}}],
        "Values": [[{"Literal": {"Value": "'Toys'"}}]]}}}]});
    let bookmark = json!({"name": "Bookmark1", "displayName": "Toys", "options": {},
        "explorationState": {"version": "1.3", "activeSection": "ReportSection",
        "sections": {"ReportSection": {"visualContainers": {"s1": {"singleVisual": {
            "visualType": "slicer",
            "objects": {"merge": {"general": [{"properties": {"filter": {"filter": filter}}}]}}}}}}}}});
    let visual = json!({"name": "s1", "layouts": [{"id": 0, "position": {"x": 10, "y": 10, "z": 0,
        "width": 200, "height": 100}}], "singleVisual": {"visualType": "slicer", "objects": {}}});
    let layout = json!({"config": json!({"bookmarks": [bookmark]}).to_string(),
        "layoutOptimization": 0,
        "sections": [{"name": "ReportSection", "displayName": "Page 1", "width": 1280,
            "height": 720, "displayOption": 1, "config": "{}", "filters": "[]",
            "visualContainers": [{"x": 10, "y": 10, "z": 0, "width": 200, "height": 100,
                "filters": "[]", "config": visual.to_string()}]}]});
    let dir = TempDir::new("bookmarks-slicer-legacy");
    let mut pkg = bicraft_pbix::synthetic::pbix(METADATA).unwrap();
    pkg.put(
        "Report/Layout",
        bicraft_opc::encode_text(&layout.to_string(), bicraft_opc::Encoding::Utf16Le),
        false,
    );
    let path = dir.join("Legacy.pbix");
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    assert_eq!(stored_filter(&s, "s1"), None);
    let out = run(&mut s, "bookmark.apply", json!({"bookmark": "Toys"}));
    assert_eq!(out["changed"], json!(true), "{out}");
    assert_eq!(stored_filter(&s, "s1"), Some(filter.clone()));
    let saved = dir.join("Saved.pbix");
    run(&mut s, "file.save_as", json!({"path": saved}));
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": saved}));
    assert_eq!(stored_filter(&back, "s1"), Some(filter));
    let c = &back.report.as_ref().unwrap().bookmarks[0].document["explorationState"]["sections"]["ReportSection"]
        ["visualContainers"]["s1"];
    assert_eq!(c["singleVisual"]["visualType"], json!("slicer"), "{c}");
}
