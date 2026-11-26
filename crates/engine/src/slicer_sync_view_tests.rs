//! Sync groups end to end (G55): a group set with `slicer.set_sync` is followed by the reader's
//! clicks (`view.interact`) on every page of the snapshot, joining a group reconciles nothing,
//! group names match without case, and Desktop's Fields option (`fieldChanges`) gives a field
//! change to the group in one undo step.

use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Two pages, each a slicer on `Sales[Category]` and a card of `[Total]` (105; Toys 5).
/// Returns the session and the (page, slicer, card) of each page.
fn session() -> (Session, Vec<(String, String, String)>) {
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
        ("page.add", json!({"display_name": "Detail"})),
    ] {
        run(&mut s, id, p);
    }
    let pages: Vec<String> = s
        .report
        .as_ref()
        .unwrap()
        .pages
        .iter()
        .map(|p| p.name.clone())
        .collect();
    let mut out = Vec::new();
    for page in pages {
        let slicer = run(
            &mut s,
            "visual.add",
            json!({"page": page, "visual_type": "slicer", "x": 20, "y": 20, "width": 220,
                   "height": 120, "fields": [{"role": "Values", "field": "Sales[Category]"}]}),
        )["visual"]
            .as_str()
            .unwrap()
            .to_string();
        let card = run(
            &mut s,
            "visual.add",
            json!({"page": page, "visual_type": "card", "x": 260, "y": 20, "width": 200,
                   "height": 120, "fields": [{"role": "Values", "field": "[Total]"}]}),
        )["visual"]
            .as_str()
            .unwrap()
            .to_string();
        out.push((page, slicer, card));
    }
    (s, out)
}

fn sync(s: &mut Session, page: &str, slicer: &str, group: &str, fields: bool) {
    run(
        s,
        "slicer.set_sync",
        json!({"page": page, "visual": slicer, "group": group, "field_changes": fields}),
    );
}

fn visual<'a>(snap: &'a Value, name: &str) -> &'a Value {
    snap["report"]["pages"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|p| p["visuals"].as_array().into_iter().flatten())
        .find(|v| v["name"] == json!(name))
        .unwrap_or_else(|| panic!("no visual {name}"))
}

fn total(snap: &Value, card: &str) -> Value {
    let d = &visual(snap, card)["data"];
    assert_eq!(d["status"], "ok", "{d}");
    d["rows"][0][0].clone()
}

fn selected(snap: &Value, slicer: &str) -> Value {
    visual(snap, slicer)["slicer"]["selected"].clone()
}

fn toggle(s: &mut Session, page: &str, slicer: &str, value: &str) {
    run(
        s,
        "view.interact",
        json!({"action": "slicer_toggle", "page": page, "visual": slicer, "value": value}),
    );
}

fn fields(s: &Session, page: &str, slicer: &str) -> Vec<String> {
    s.report
        .as_ref()
        .and_then(|r| r.pages.iter().find(|p| p.name == page))
        .and_then(|p| p.visuals.iter().find(|v| v.name == slicer))
        .map(|v| v.fields.iter().map(|b| format!("{:?}", b.field)).collect())
        .unwrap_or_default()
}

#[test]
fn a_click_reaches_the_slicer_a_set_sync_grouped_and_filters_its_page() {
    let (mut s, v) = session();
    let ((p1, s1, _), (p2, s2, c2)) = (v[0].clone(), v[1].clone());
    sync(&mut s, &p1, &s1, "Category", true);
    sync(&mut s, &p2, &s2, "Category", true);
    toggle(&mut s, &p1, &s1, "Toys");
    let snap = s.snapshot();
    assert_eq!(selected(&snap, &s2), json!(["Toys"]));
    assert_eq!(total(&snap, &c2), json!(5));
    // page 2 shown: the same
    run(&mut s, "view.interact", json!({"action": "page", "page": p2}));
    let snap = s.snapshot();
    assert_eq!(snap["interaction"]["page"], json!(p2));
    assert_eq!(selected(&snap, &s2), json!(["Toys"]));
    assert_eq!(total(&snap, &c2), json!(5));
    // a click from page 2 comes back to page 1
    toggle(&mut s, &p2, &s2, "Wheels");
    let snap = s.snapshot();
    assert_eq!(selected(&snap, &s1), json!(["Wheels"]));
    // Filters off on page 2's slicer: it neither sends nor receives
    run(
        &mut s,
        "slicer.set_sync",
        json!({"page": p2, "visual": s2, "group": "Category", "filter_changes": false}),
    );
    toggle(&mut s, &p1, &s1, "Toys");
    let snap = s.snapshot();
    assert_eq!(selected(&snap, &s1), json!(["Toys"]));
    assert_eq!(selected(&snap, &s2), json!(["Wheels"]));
}

#[test]
fn a_browser_viewer_state_follows_the_group_too() {
    // the browser viewer keeps one state per viewer over a shared session (`interact_view`,
    // `snapshot_view`, crates/view): the same propagation, the session's own state untouched
    let (mut s, v) = session();
    let ((p1, s1, _), (p2, s2, c2)) = (v[0].clone(), v[1].clone());
    sync(&mut s, &p1, &s1, "Category", true);
    sync(&mut s, &p2, &s2, "Category", true);
    let mut viewer = crate::Interactions::default();
    s.interact_view(
        &mut viewer,
        &json!({"action": "slicer_toggle", "page": p1, "visual": s1, "value": "Toys"}),
    )
    .unwrap();
    s.interact_view(&mut viewer, &json!({"action": "page", "page": p2}))
        .unwrap();
    let snap = s.snapshot_view(&viewer);
    assert_eq!(selected(&snap, &s2), json!(["Toys"]));
    assert_eq!(total(&snap, &c2), json!(5));
    assert_eq!(total(&s.snapshot(), &c2), json!(105));
}

#[test]
fn joining_a_group_reconciles_nothing_until_the_next_click() {
    // Desktop leaves each copy's selection when the report opens (oracle rule 6); joining a
    // group in the pane does not copy one either: the next click in the group does
    let (mut s, v) = session();
    let ((p1, s1, _), (p2, s2, c2)) = (v[0].clone(), v[1].clone());
    toggle(&mut s, &p1, &s1, "Toys");
    sync(&mut s, &p1, &s1, "Category", true);
    sync(&mut s, &p2, &s2, "Category", true);
    let snap = s.snapshot();
    assert_eq!(selected(&snap, &s1), json!(["Toys"]));
    assert_eq!(total(&snap, &c2), json!(105), "page 2 keeps its own selection");
    toggle(&mut s, &p1, &s1, "Wheels");
    let snap = s.snapshot();
    assert_eq!(selected(&snap, &s2), json!(["Wheels"]));
    assert_eq!(total(&snap, &c2), json!(100));
}

#[test]
fn a_group_typed_in_another_case_is_the_same_group() {
    let (mut s, v) = session();
    let ((p1, s1, _), (p2, s2, c2)) = (v[0].clone(), v[1].clone());
    sync(&mut s, &p1, &s1, "Category", true);
    let out = run(
        &mut s,
        "slicer.set_sync",
        json!({"page": p2, "visual": s2, "group": "CATEGORY"}),
    );
    assert_eq!(out["group"], json!("Category"), "the group keeps its spelling");
    let list = run(&mut s, "slicer.sync_list", json!({}));
    assert_eq!(list["groups"].as_array().map(Vec::len), Some(1), "{list}");
    toggle(&mut s, &p1, &s1, "Toys");
    let snap = s.snapshot();
    assert_eq!(selected(&snap, &s2), json!(["Toys"]));
    assert_eq!(total(&snap, &c2), json!(5));
}

#[test]
fn a_field_change_reaches_the_group_in_one_undo_step() {
    let (mut s, v) = session();
    let ((p1, s1, _), (p2, s2, _)) = (v[0].clone(), v[1].clone());
    sync(&mut s, &p1, &s1, "Category", true);
    sync(&mut s, &p2, &s2, "Category", true);
    let before = fields(&s, &p2, &s2);
    let out = run(
        &mut s,
        "visual.bind",
        json!({"page": p1, "visual": s1, "role": "Values", "field": "Sales[Amount]"}),
    );
    assert_eq!(out["synced"], json!([{"page": p2, "visual": s2}]), "{out}");
    assert_eq!(fields(&s, &p2, &s2), fields(&s, &p1, &s1));
    assert_eq!(fields(&s, &p2, &s2).len(), 2);
    // one undo step takes both back
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(fields(&s, &p1, &s1), before);
    assert_eq!(fields(&s, &p2, &s2), before);
    // removed, then moved: the group follows each time
    run(
        &mut s,
        "visual.unbind",
        json!({"page": p2, "visual": s2, "role": "Values", "field": "Sales[Category]"}),
    );
    assert!(fields(&s, &p1, &s1).is_empty());
    run(
        &mut s,
        "visual.bind",
        json!({"page": p2, "visual": s2, "role": "Values", "field": "Sales[Amount]"}),
    );
    run(
        &mut s,
        "visual.bind",
        json!({"page": p2, "visual": s2, "role": "Values", "field": "Sales[Category]"}),
    );
    run(
        &mut s,
        "visual.move_field",
        json!({"page": p1, "visual": s1, "role": "Values", "index": 1, "to_index": 0}),
    );
    assert_eq!(fields(&s, &p2, &s2), fields(&s, &p1, &s1));
    assert!(
        fields(&s, &p2, &s2)[0].contains("Category"),
        "{:?}",
        fields(&s, &p2, &s2)
    );
    // Fields off on page 2's slicer: it keeps its fields
    sync(&mut s, &p2, &s2, "Category", false);
    let kept = fields(&s, &p2, &s2);
    let out = run(
        &mut s,
        "visual.unbind",
        json!({"page": p1, "visual": s1, "role": "Values", "field": "Sales[Amount]"}),
    );
    assert_eq!(out["synced"], json!([]));
    assert_eq!(fields(&s, &p2, &s2), kept);
    // and a change on it is not sent either
    let out = run(
        &mut s,
        "visual.unbind",
        json!({"page": p2, "visual": s2, "role": "Values", "field": "Sales[Amount]"}),
    );
    assert!(out.get("synced").is_none(), "{out}");
    assert_eq!(fields(&s, &p1, &s1).len(), 1);
}

#[test]
fn a_field_change_outside_a_group_changes_one_slicer() {
    let (mut s, v) = session();
    let ((p1, s1, c1), (p2, s2, _)) = (v[0].clone(), v[1].clone());
    let before = fields(&s, &p2, &s2);
    let out = run(
        &mut s,
        "visual.bind",
        json!({"page": p1, "visual": s1, "role": "Values", "field": "Sales[Amount]"}),
    );
    assert!(out.get("synced").is_none(), "{out}");
    assert_eq!(fields(&s, &p2, &s2), before);
    // a card is no slicer, even with a syncGroup written by hand
    if let Some(v) = s
        .report
        .as_mut()
        .and_then(|r| r.pages.iter_mut().find(|p| p.name == p1))
        .and_then(|p| p.visuals.iter_mut().find(|v| v.name == c1))
    {
        v.visual_extra
            .insert("syncGroup".into(), json!({"groupName": "Category"}));
    }
    sync(&mut s, &p2, &s2, "Category", true);
    let out = run(
        &mut s,
        "visual.bind",
        json!({"page": p1, "visual": c1, "role": "Values", "field": "Sales[Amount]", "force": true}),
    );
    let _ = out;
    assert_eq!(fields(&s, &p2, &s2), before);
}
