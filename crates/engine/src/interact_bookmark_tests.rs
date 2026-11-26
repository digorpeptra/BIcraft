//! Tests of `interact_bookmark.rs`: `view.interact {action: "bookmark"}` and bookmark buttons
//! change the reader's view only (page, visibility, filters, within the bookmark's Data /
//! Display / Current page options), the snapshot shows it, and hostile parameters are errors that
//! leave the state alone.

use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Two pages: a card and a column chart on the first, a table on the second.
fn session() -> Session {
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
    let first = page(&s, 0);
    for (ty, x, fields) in [
        ("card", 20, json!([{"role": "Values", "field": "[Total]"}])),
        (
            "clusteredColumnChart",
            260,
            json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]),
        ),
    ] {
        run(
            &mut s,
            "visual.add",
            json!({"page": first, "visual_type": ty, "x": x, "y": 20, "width": 200,
                   "height": 160, "fields": fields}),
        );
    }
    run(
        &mut s,
        "page.add",
        json!({"display_name": "Detail", "width": 1280, "height": 720}),
    );
    let second = page(&s, 1);
    run(
        &mut s,
        "visual.add",
        json!({"page": second, "visual_type": "tableEx", "x": 20, "y": 20, "width": 400,
               "height": 200, "fields": [{"role": "Values", "field": "Sales[Category]"}]}),
    );
    run(&mut s, "page.set_active", json!({"page": first}));
    s
}

fn page(s: &Session, i: usize) -> String {
    s.report
        .as_ref()
        .and_then(|r| r.pages.get(i))
        .map(|p| p.name.clone())
        .unwrap_or_default()
}

fn card(s: &Session) -> String {
    s.report.as_ref().unwrap().pages[0].visuals[0].name.clone()
}

/// A bookmark of page 1 with the card hidden and filtered, then the report moved away from it
/// (page 2 active, the card shown, its filter cleared) and saved as clean.
fn moved_away(s: &mut Session, display: &str) -> String {
    let first = page(s, 0);
    let second = page(s, 1);
    let card = card(s);
    run(
        s,
        "visual.filter",
        json!({"page": first, "visual": card, "field": "Sales[Category]", "in": ["Wheels"]}),
    );
    run(
        s,
        "visual.set_hidden",
        json!({"page": first, "visual": card, "hidden": true}),
    );
    let name = run(
        s,
        "bookmark.capture",
        json!({"display_name": display, "current_page": true}),
    )["bookmark"]
        .as_str()
        .unwrap()
        .to_string();
    run(s, "page.set_active", json!({"page": second}));
    run(
        s,
        "visual.set_hidden",
        json!({"page": first, "visual": card, "hidden": false}),
    );
    run(s, "visual.clear_filters", json!({"page": first, "visual": card}));
    s.report_dirty = false;
    name
}

/// The snapshot's card: (hidden, number of filter cards).
fn shown_card(s: &Session) -> (bool, usize) {
    let snap = s.snapshot();
    let first = page(s, 0);
    let card = card(s);
    let v = snap["report"]["pages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == json!(first))
        .unwrap()["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == json!(card))
        .cloned()
        .unwrap();
    let filters = v["filters"].as_array().map_or(0, Vec::len);
    (v["hidden"] == json!(true), filters)
}

fn active(s: &Session) -> Value {
    s.snapshot()["report"]["active_page"].clone()
}

#[test]
fn a_bookmark_applies_to_the_view_and_never_to_the_report() {
    let mut s = session();
    let first = page(&s, 0);
    let second = page(&s, 1);
    let name = moved_away(&mut s, "Wheels");
    let report = s.report.clone();
    let revision = s.revision;
    assert_eq!(active(&s), json!(second));
    assert_eq!(shown_card(&s), (false, 0));

    let out = run(
        &mut s,
        "view.interact",
        json!({"action": "bookmark", "bookmark": "Wheels"}),
    );
    assert_eq!(out["changed"], json!(true), "{out}");
    assert_eq!(out["bookmark"], json!(name));
    assert_eq!(out["page"], json!(first));
    assert_eq!(out["filters_changed"], json!(true));
    assert_eq!(out["hidden"].as_array().map(Vec::len), Some(1), "{out}");
    // the view: the bookmark's page, the card hidden and filtered
    assert_eq!(active(&s), json!(first));
    assert_eq!(shown_card(&s), (true, 1));
    assert_eq!(s.snapshot()["interaction"]["bookmark"], json!(name));
    // the report, its revision and the dirty flags did not move
    assert_eq!(s.report, report);
    assert_eq!(s.revision, revision);
    assert!(!s.report_dirty);
    // the same bookmark again changes nothing; Back returns to the page left
    let out = run(
        &mut s,
        "view.interact",
        json!({"action": "bookmark", "bookmark": name}),
    );
    assert_eq!(out["changed"], json!(false), "{out}");
    run(&mut s, "view.interact", json!({"action": "back"}));
    assert_eq!(active(&s), json!(second));
    // reset drops it all
    run(&mut s, "view.interact", json!({"action": "reset"}));
    assert_eq!(shown_card(&s), (false, 0));
    assert_eq!(s.snapshot()["interaction"]["bookmark"], Value::Null);
}

#[test]
fn the_bookmark_options_say_what_the_view_takes() {
    for (option, page_moves, hides, filters) in [
        ("current_page", false, true, 1),
        ("display", true, false, 1),
        ("data", true, true, 0),
    ] {
        let mut s = session();
        let first = page(&s, 0);
        let second = page(&s, 1);
        let name = moved_away(&mut s, "State");
        run(&mut s, "bookmark.set", json!({"bookmark": name, option: false}));
        s.report_dirty = false;
        let report = s.report.clone();
        run(
            &mut s,
            "view.interact",
            json!({"action": "bookmark", "bookmark": name}),
        );
        let want_page = if page_moves { &first } else { &second };
        assert_eq!(active(&s), json!(want_page), "{option} off");
        assert_eq!(shown_card(&s), (hides, filters), "{option} off");
        assert_eq!(s.report, report, "{option} off: the report is unchanged");
    }
}

#[test]
fn a_bookmark_button_applies_its_bookmark() {
    let mut s = session();
    let first = page(&s, 0);
    let second = page(&s, 1);
    let name = moved_away(&mut s, "Wheels");
    let bt = run(&mut s, "button.add", json!({"page": second, "kind": "blank"}))["visual"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut s,
        "visual.set_action",
        json!({"page": second, "visual": bt, "type": "bookmark", "bookmark": name}),
    );
    let snap = s.snapshot();
    let v = snap["report"]["pages"][1]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == json!(bt))
        .cloned()
        .unwrap();
    assert_eq!(
        v["interaction"]["button"],
        json!({"kind": "Bookmark", "page": null, "bookmark": name, "enabled": true})
    );
    let out = run(
        &mut s,
        "view.interact",
        json!({"action": "button", "page": second, "visual": bt}),
    );
    assert_eq!(out["button"], json!("Bookmark"));
    assert_eq!(out["bookmark"], json!(name));
    assert_eq!(active(&s), json!(first));
    assert_eq!(shown_card(&s), (true, 1));
}

#[test]
fn hostile_bookmark_actions_are_errors_and_leave_the_view_alone() {
    let mut s = session();
    let second = page(&s, 1);
    let name = moved_away(&mut s, "Wheels");
    // a bookmark without state, written by hand
    if let Some(r) = s.report.as_mut() {
        r.bookmarks.push(bicraft_report::NamedDocument {
            name: "bare".into(),
            document: json!({"name": "bare", "displayName": "Bare"}),
        });
    }
    let bt = run(&mut s, "button.add", json!({"page": second, "kind": "blank"}))["visual"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut s,
        "visual.set_action",
        json!({"page": second, "visual": bt, "type": "bookmark"}),
    );
    run(
        &mut s,
        "view.interact",
        json!({"action": "bookmark", "bookmark": name}),
    );
    let before = s.interactions.state_json();
    for p in [
        json!({"action": "bookmark"}),
        json!({"action": "bookmark", "bookmark": null}),
        json!({"action": "bookmark", "bookmark": 3}),
        json!({"action": "bookmark", "bookmark": ["Wheels"]}),
        json!({"action": "bookmark", "bookmark": "nope"}),
        json!({"action": "bookmark", "bookmark": "Bare"}),
        json!({"action": "bookmark", "bookmark": "x".repeat(100_000)}),
        json!({"action": "button", "page": second, "visual": bt}),
    ] {
        assert!(s.execute("view.interact", &p).is_err(), "{p} should fail");
        assert_eq!(s.interactions.state_json(), before, "{p} changed the view");
    }
}

#[test]
fn navigators_reach_the_snapshot_with_the_bookmarks() {
    let mut s = session();
    let first = page(&s, 0);
    run(&mut s, "bookmark.capture", json!({"display_name": "One"}));
    let two = run(&mut s, "bookmark.capture", json!({"display_name": "Two"}))["bookmark"]
        .as_str()
        .unwrap()
        .to_string();
    let g = run(
        &mut s,
        "bookmark.group",
        json!({"bookmarks": [two], "display_name": "Group 1"}),
    );
    let nav = run(
        &mut s,
        "button.add",
        json!({"page": first, "kind": "bookmark_navigator"}),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string();
    let snap = s.snapshot();
    let list = snap["bookmarks"]["bookmarks"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    let grouped = list.iter().find(|b| b["name"] == json!(two)).unwrap();
    assert_eq!(grouped["group_name"], g["group"]);
    let v = snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == json!(nav))
        .cloned()
        .unwrap();
    assert_eq!(v["type"], json!("bookmarkNavigator"));
    assert_eq!(v["navigator"]["orientation"], Value::Null);
    assert_eq!(v["navigator"]["bookmark_group"], Value::Null);
    // its layout and group as Desktop writes them
    if let Some(r) = s.report.as_mut()
        && let Some(v) = r.pages[0].visuals.iter_mut().find(|v| v.name == nav)
    {
        let group = g["group"].as_str().unwrap();
        v.visual_extra.insert(
            "objects".into(),
            json!({
                "layout": [{"properties": {"orientation": {"expr": {"Literal": {"Value": "1L"}}}}}],
                "bookmarks": [{"properties": {"bookmarkGroup": {"expr": {"Literal": {"Value": format!("'{group}'")}}}}}]
            }),
        );
    }
    let snap = s.snapshot();
    let v = snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == json!(nav))
        .cloned()
        .unwrap();
    assert_eq!(v["navigator"]["orientation"], json!("vertical"));
    assert_eq!(v["navigator"]["bookmark_group"], g["group"]);
    assert_eq!(v["navigator"]["styles"], json!({}));
    // per-state styles through `visual.format` (fill, text and accent bar), and the padding
    let page = page(&s, 0);
    let obj = |object: &str, property: &str, value: Value, state: &str| json!({"object": object, "property": property, "value": value, "state": state});
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": nav, "objects": [
            obj("fill", "fillColor", json!({"color": "#102030"}), "default"),
            obj("text", "fontColor", json!({"color": "#FFFFFF"}), "default"),
            obj("fill", "fillColor", json!({"color": "#C04000"}), "selected"),
            obj("text", "bold", json!(false), "selected"),
            obj("accentBar", "position", json!("Top"), "selected"),
            obj("accentBar", "width", json!(6), "selected"),
            json!({"object": "layout", "property": "cellPadding", "value": 12}),
        ]}),
    );
    let snap = s.snapshot();
    let v = snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == json!(nav))
        .cloned()
        .unwrap();
    let n = &v["navigator"];
    assert_eq!(n["styles"]["default"]["fill"], json!("#102030"), "{n}");
    assert_eq!(n["styles"]["default"]["font_color"], json!("#FFFFFF"), "{n}");
    assert_eq!(n["styles"]["selected"]["fill"], json!("#C04000"), "{n}");
    assert_eq!(n["styles"]["selected"]["bold"], json!(false), "{n}");
    assert_eq!(n["styles"]["selected"]["accent_position"], json!("top"), "{n}");
    assert_eq!(n["styles"]["selected"]["accent_width"], json!(6), "{n}");
    assert_eq!(n["padding"], json!(12), "{n}");
    assert_eq!(
        n["orientation"],
        json!("vertical"),
        "the layout keeps its orientation: {n}"
    );
}
