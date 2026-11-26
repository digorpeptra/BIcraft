//! `page.format_filter_pane`: the Filters pane and its cards formatted per page
//! (`objects.outspacePane`, `objects.filterCard` with `selector.id` `Available` / `Applied`),
//! the theme's `outspacePane` / `filterCard` as defaults, kept by a PBIP, a PBIX and a legacy
//! layout (`docs/formats/pbir.md`, "Filters pane formatting and card order").

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

const SYNTHETIC_METADATA: &[u8] =
    include_bytes!("../../../metadata/tests/fixtures/synthetic_metadata.sqlitedb");

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn shop() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "m": "let Source = #table(type table [Region = text, Amount = Int64.Type], {{\"East\", 5}, {\"West\", 7}}) in Source"}),
    );
    run(&mut s, "report.new", json!({}));
    s
}

fn first_page(s: &Session) -> String {
    s.report.as_ref().unwrap().pages[0].name.clone()
}

fn objects(s: &Session) -> Value {
    s.report.as_ref().unwrap().pages[0]
        .extra
        .get("objects")
        .cloned()
        .unwrap_or(Value::Null)
}

fn lit(v: &str) -> Value {
    json!({"expr": {"Literal": {"Value": v}}})
}

fn color(c: &str) -> Value {
    json!({"solid": {"color": lit(&format!("'{c}'"))}})
}

/// Formats the pane and both card states as a report author would.
fn format_all(s: &mut Session) {
    let page = first_page(s);
    run(
        s,
        "page.format_filter_pane",
        json!({"page": page, "background_color": "#102030", "transparency": 20,
               "foreground_color": "#f0f0f0", "title_size": 18, "header_size": 11,
               "border": true, "border_color": "#FF0000", "font_family": "Georgia",
               "input_box_color": "#203040", "search_text_size": 10, "checkbox_color": "#00FF00"}),
    );
    run(
        s,
        "page.format_filter_pane",
        json!({"page": page, "card": "available", "background_color": "#EEEEEE", "text_size": 10}),
    );
    run(
        s,
        "page.format_filter_pane",
        json!({"page": page, "card": "applied", "background_color": "#FFE0B0", "border": true,
               "border_color": "#C04000", "foreground_color": "#202020"}),
    );
}

fn check_written(s: &Session, label: &str) {
    let o = objects(s);
    let pane = &o["outspacePane"];
    assert_eq!(pane.as_array().map(Vec::len), Some(1), "{label}: {o}");
    let props = &pane[0]["properties"];
    assert_eq!(props["backgroundColor"], color("#102030"), "{label}");
    assert_eq!(props["foregroundColor"], color("#F0F0F0"), "{label}");
    assert_eq!(props["transparency"], lit("20D"), "{label}");
    assert_eq!(props["titleSize"], lit("18D"), "{label}");
    assert_eq!(props["border"], lit("true"), "{label}");
    assert_eq!(props["fontFamily"], lit("'Georgia'"), "{label}");
    assert_eq!(props["checkboxAndApplyColor"], color("#00FF00"), "{label}");
    let cards = o["filterCard"].as_array().cloned().unwrap_or_default();
    assert_eq!(cards.len(), 2, "{label}: {o}");
    let applied = cards
        .iter()
        .find(|e| e["selector"]["id"] == "Applied")
        .unwrap_or_else(|| panic!("{label}: no Applied entry in {o}"));
    assert_eq!(
        applied["properties"]["backgroundColor"],
        color("#FFE0B0"),
        "{label}"
    );
    assert_eq!(applied["properties"]["border"], lit("true"), "{label}");
    let available = cards
        .iter()
        .find(|e| e["selector"]["id"] == "Available")
        .unwrap_or_else(|| panic!("{label}: no Available entry in {o}"));
    assert_eq!(available["properties"]["textSize"], lit("10D"), "{label}");
    // the snapshot's resolved format
    let snap = s.snapshot();
    let f = &snap["report"]["pages"][0]["filter_pane_format"];
    assert_eq!(f["pane"]["background_color"], "#102030", "{label}: {f}");
    assert_eq!(f["pane"]["title_size"], 18.0, "{label}: {f}");
    assert_eq!(f["pane"]["font_family"], "Georgia", "{label}: {f}");
    assert_eq!(f["pane"]["border"], true, "{label}: {f}");
    assert_eq!(f["applied"]["background_color"], "#FFE0B0", "{label}: {f}");
    assert_eq!(f["applied"]["foreground_color"], "#202020", "{label}: {f}");
    assert_eq!(f["available"]["text_size"], 10.0, "{label}: {f}");
    assert!(f["available"].get("border").is_none(), "{label}: {f}");
}

#[test]
fn the_pane_and_both_card_states_are_written_read_back_and_undone() {
    let mut s = shop();
    assert!(
        s.snapshot()["report"]["pages"][0]
            .get("filter_pane_format")
            .is_none(),
        "a new report formats nothing"
    );
    format_all(&mut s);
    check_written(&s, "session");
    let page = first_page(&s);
    // the same value again changes nothing
    let again = run(
        &mut s,
        "page.format_filter_pane",
        json!({"page": page, "card": "available", "text_size": 10}),
    );
    assert_eq!(again["changed"], false);
    // null removes one property; reset removes the card's properties, the entry goes
    let out = run(
        &mut s,
        "page.format_filter_pane",
        json!({"page": page, "title_size": null}),
    );
    assert_eq!(out["changed"], true);
    assert!(
        objects(&s)["outspacePane"][0]["properties"]
            .get("titleSize")
            .is_none()
    );
    run(
        &mut s,
        "page.format_filter_pane",
        json!({"page": page, "card": "available", "reset": true}),
    );
    let cards = objects(&s)["filterCard"].as_array().cloned().unwrap_or_default();
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert_eq!(cards[0]["selector"]["id"], "Applied");
    // one undo step per command
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    check_written(&s, "undo");
}

#[test]
fn the_theme_gives_the_defaults_and_the_page_wins() {
    let mut s = shop();
    let theme = json!({
        "name": "Filters",
        "dataColors": ["#118DFF"],
        "visualStyles": {"page": {"*": {
            "outspacePane": [{"backgroundColor": {"solid": {"color": "#0000FF"}}, "titleSize": 20, "border": true}],
            "filterCard": [
                {"$id": "Applied", "backgroundColor": {"solid": {"color": "#FF0000"}}, "textSize": 14},
                {"$id": "Available", "backgroundColor": {"solid": {"color": "#00FF00"}}}
            ]
        }}}
    });
    run(&mut s, "theme.set", json!({"theme": theme}));
    let f = s.snapshot()["report"]["pages"][0]["filter_pane_format"].clone();
    assert_eq!(f["pane"]["background_color"], "#0000FF", "{f}");
    assert_eq!(f["pane"]["title_size"], 20.0, "{f}");
    assert_eq!(f["applied"]["background_color"], "#FF0000", "{f}");
    assert_eq!(f["applied"]["text_size"], 14.0, "{f}");
    assert_eq!(f["available"]["background_color"], "#00FF00", "{f}");
    assert!(f["available"].get("text_size").is_none(), "{f}");
    let page = first_page(&s);
    run(
        &mut s,
        "page.format_filter_pane",
        json!({"page": page, "card": "applied", "background_color": "#ABCDEF"}),
    );
    let f = s.snapshot()["report"]["pages"][0]["filter_pane_format"].clone();
    assert_eq!(f["applied"]["background_color"], "#ABCDEF", "{f}");
    // the theme's other properties stay
    assert_eq!(f["applied"]["text_size"], 14.0, "{f}");
    assert_eq!(f["pane"]["background_color"], "#0000FF", "{f}");
}

/// The session after a save as `file` and a fresh open.
fn reopened(s: &mut Session, dir: &TempDir, file: &str) -> Session {
    run(s, "file.save_as", json!({"path": dir.join(file)}));
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": dir.join(file)}));
    back
}

#[test]
fn pbip_and_pbix_keep_the_filters_pane_format() {
    let dir = TempDir::new("filter-pane-format-files");
    for file in ["Shop.pbip", "Shop.pbix"] {
        let mut s = shop();
        format_all(&mut s);
        let back = reopened(&mut s, &dir, file);
        check_written(&back, file);
    }
}

fn synthetic(dir: &TempDir, pbir: bool) -> PathBuf {
    let path = dir.join(if pbir { "Pbir.pbix" } else { "Legacy.pbix" });
    let pkg = if pbir {
        bicraft_pbix::synthetic::pbir_pbix(SYNTHETIC_METADATA)
    } else {
        bicraft_pbix::synthetic::pbix(SYNTHETIC_METADATA)
    }
    .unwrap();
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    path
}

fn open(path: &Path) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    s
}

#[test]
fn a_legacy_layout_and_pbir_parts_keep_the_filters_pane_format() {
    let dir = TempDir::new("filter-pane-format-legacy");
    for pbir in [false, true] {
        let path = synthetic(&dir, pbir);
        let mut s = open(&path);
        format_all(&mut s);
        run(&mut s, "file.save", json!({}));
        let back = open(&path);
        check_written(&back, if pbir { "pbir parts" } else { "legacy layout" });
    }
}

#[test]
fn bad_formats_are_refused() {
    let mut s = shop();
    let page = first_page(&s);
    for bad in [
        json!({"page": page}),
        json!({"page": page, "card": "hover", "border": true}),
        json!({"page": page, "background_color": "blue"}),
        json!({"page": page, "transparency": 140}),
        json!({"page": page, "title_size": "big"}),
        json!({"page": page, "border": "yes"}),
        json!({"page": page, "font_family": ""}),
        json!({"page": page, "text_size": 12}),
        json!({"page": page, "card": "applied", "title_size": 12}),
        json!({"page": page, "color": "#FFFFFF"}),
        json!({"page": "Nope", "border": true}),
        json!({"page": page, "reset": 1}),
    ] {
        assert!(s.execute("page.format_filter_pane", &bad).is_err(), "{bad}");
    }
    // hostile objects are refused, not crashed on
    s.report.as_mut().unwrap().pages[0]
        .extra
        .insert("objects".into(), json!({"filterCard": 3}));
    assert!(
        s.execute(
            "page.format_filter_pane",
            &json!({"page": page, "card": "applied", "border": true})
        )
        .is_err()
    );
    s.report.as_mut().unwrap().pages[0]
        .extra
        .insert("objects".into(), json!([1]));
    assert!(
        s.execute("page.format_filter_pane", &json!({"page": page, "border": true}))
            .is_err()
    );
    let _ = s.snapshot();
    let mut empty = Session::new();
    assert!(
        empty
            .execute("page.format_filter_pane", &json!({"page": "x", "border": true}))
            .is_err()
    );
}
