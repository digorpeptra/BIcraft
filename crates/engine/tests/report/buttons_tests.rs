//! Buttons, shapes and navigators beyond their insertion (`docs/formats/buttons.md`): conditional
//! (fx) destinations and tooltips, a drillthrough button's enabled and disabled states and
//! tooltips, the button, shape and navigator states the readers draw (hover, press, disabled,
//! selected), a shape's text, a bookmark navigator's Allow deselection; the snapshot, the reader's
//! clicks and the PBIP, PBIX and legacy-layout round trips. Synthetic model only.

use std::path::{Path, PathBuf};

use bicraft_report::{Report, Visual};
use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

const SYNTHETIC_METADATA: &[u8] =
    include_bytes!("../../../metadata/tests/fixtures/synthetic_metadata.sqlitedb");

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Runs a command that must fail and checks it left the report alone.
fn fails(s: &mut Session, id: &str, params: Value) -> String {
    let before = s.report.clone();
    let err = s
        .execute(id, &params)
        .expect_err(&format!("{id} {params} should fail"));
    assert_eq!(s.report, before, "{id} changed the report although it failed");
    err
}

/// A model with a region column, a measure and a text measure naming a page; a report with two
/// pages (Main, Detail).
fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "m": "let Source = #table(type table [Region = text, Amount = Int64.Type], {{\"East\", 5}, {\"West\", 7}}) in Source"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Where", "expression": "\"Detail\""}),
    );
    run(&mut s, "report.new", json!({}));
    let first = page(&s, 0);
    run(
        &mut s,
        "page.rename",
        json!({"page": first, "display_name": "Main"}),
    );
    run(&mut s, "page.add", json!({"display_name": "Detail"}));
    s
}

fn page(s: &Session, i: usize) -> String {
    s.report.as_ref().unwrap().pages[i].name.clone()
}

fn report(s: &Session) -> &Report {
    s.report.as_ref().unwrap()
}

fn find<'a>(r: &'a Report, name: &str) -> &'a Visual {
    r.pages
        .iter()
        .flat_map(|p| &p.visuals)
        .find(|v| v.name == name)
        .unwrap_or_else(|| panic!("no visual {name}"))
}

fn snap_visual(s: &Session, name: &str) -> Value {
    let snap = s.snapshot();
    for p in snap["report"]["pages"].as_array().cloned().unwrap_or_default() {
        for v in p["visuals"].as_array().cloned().unwrap_or_default() {
            if v["name"] == name {
                return v;
            }
        }
    }
    panic!("visual {name} not in the snapshot")
}

fn add_button(s: &mut Session, page: &str, kind: &str) -> String {
    run(s, "button.add", json!({"page": page, "kind": kind}))["visual"]
        .as_str()
        .unwrap()
        .to_string()
}

fn reopened(s: &mut Session, dir: &TempDir, file: &str) -> Session {
    run(s, "file.save_as", json!({"path": dir.join(file)}));
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": dir.join(file)}));
    back
}

#[test]
fn conditional_destinations_are_written_evaluated_and_followed() {
    let mut s = session();
    let (main, detail) = (page(&s, 0), page(&s, 1));
    let go = add_button(&mut s, &main, "blank");
    let out = run(
        &mut s,
        "visual.set_action",
        json!({"page": main, "visual": go, "type": "page_navigation", "destination_field": "[Where]",
               "tooltip_field": "[Where]"}),
    );
    assert_eq!(out["conditional"], true, "{out}");
    let link = &find(report(&s), &go).visual_extra["visualContainerObjects"]["visualLink"][0]["properties"];
    // the destination is the measure's expression, not a literal
    assert_eq!(
        link["navigationSection"]["expr"]["Measure"]["Property"], "Where",
        "{link}"
    );
    assert_eq!(link["tooltip"]["expr"]["Measure"]["Property"], "Where", "{link}");
    let sv = snap_visual(&s, &go);
    assert!(
        sv["action"]["destination_field"]
            .as_str()
            .unwrap()
            .contains("Where"),
        "{sv}"
    );
    assert_eq!(sv["action"]["dynamic_destination"], "Detail", "{sv}");
    let b = &sv["interaction"]["button"];
    assert_eq!(b["conditional"], true, "{b}");
    assert_eq!(b["destination"], detail, "{b}");
    assert_eq!(b["enabled"], true, "{b}");
    assert_eq!(b["tooltip"], "Detail", "{b}");
    // the reader sends the evaluated destination
    let r = run(
        &mut s,
        "view.interact",
        json!({"action": "button", "page": main, "visual": go, "destination": detail}),
    );
    assert_eq!(r["state"]["page"], detail, "{r}");
    run(&mut s, "view.interact", json!({"action": "reset"}));
    // a column is read through Min (Desktop's First)
    run(
        &mut s,
        "visual.set_action",
        json!({"page": main, "visual": go, "type": "bookmark", "destination_field": "Sales[Region]"}),
    );
    let link = &find(report(&s), &go).visual_extra["visualContainerObjects"]["visualLink"][0]["properties"];
    assert!(link["bookmark"]["expr"]["Aggregation"].is_object(), "{link}");
    assert!(link.get("navigationSection").is_none(), "{link}");
    // a value that names no bookmark disables the button
    let sv = snap_visual(&s, &go);
    assert_eq!(sv["interaction"]["button"]["enabled"], false, "{sv}");
    // a fixed destination is never replaced by the reader
    let fixed = add_button(&mut s, &main, "blank");
    run(
        &mut s,
        "visual.set_action",
        json!({"page": main, "visual": fixed, "type": "page_navigation", "destination": "Detail"}),
    );
    let e = s
        .execute(
            "view.interact",
            &json!({"action": "button", "page": main, "visual": fixed, "destination": main}),
        )
        .unwrap_err();
    assert!(e.contains("conditional"), "{e}");
    // refusals
    for (p, why) in [
        (
            json!({"page": main, "visual": go, "type": "back", "destination_field": "[Where]"}),
            "applies to",
        ),
        (
            json!({"page": main, "visual": go, "type": "page_navigation", "destination": "Detail", "destination_field": "[Where]"}),
            "not both",
        ),
        (
            json!({"page": main, "visual": go, "type": "back", "tooltip": "x", "tooltip_field": "[Where]"}),
            "not both",
        ),
        (
            json!({"page": main, "visual": go, "type": "page_navigation", "destination_field": "[Nope]"}),
            "destination_field",
        ),
        (
            json!({"page": main, "visual": go, "type": "none", "disabled_tooltip": "x"}),
            "none",
        ),
        (
            json!({"page": main, "visual": go, "type": "drillthrough", "enabled_tooltip": 3}),
            "enabled_tooltip",
        ),
    ] {
        let e = fails(&mut s, "visual.set_action", p.clone());
        assert!(e.contains(why), "{p}: {e}");
    }
    // PBIP and PBIX keep the conditional action
    let dir = TempDir::new("engine-buttons-fx");
    run(
        &mut s,
        "visual.set_action",
        json!({"page": main, "visual": go, "type": "page_navigation", "destination_field": "[Where]"}),
    );
    for file in ["Shop.pbip", "Shop.pbix"] {
        let back = reopened(&mut s, &dir, file);
        assert_eq!(
            find(report(&back), &go).visual_extra["visualContainerObjects"],
            find(report(&s), &go).visual_extra["visualContainerObjects"],
            "{file}"
        );
        assert_eq!(
            snap_visual(&back, &go)["interaction"]["button"]["destination"],
            page(&back, 1),
            "{file}"
        );
    }
}

#[test]
fn a_drillthrough_button_is_disabled_until_a_point_is_selected() {
    let mut s = session();
    let main = page(&s, 0);
    run(
        &mut s,
        "page.set_type",
        json!({"page": "Detail", "type": "drillthrough", "fields": ["Sales[Region]"]}),
    );
    let chart = run(
        &mut s,
        "visual.add",
        json!({"page": main, "visual_type": "clusteredColumnChart", "x": 0, "y": 100, "width": 400, "height": 300,
               "fields": [{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]}),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string();
    let bt = add_button(&mut s, &main, "blank");
    run(
        &mut s,
        "visual.set_action",
        json!({"page": main, "visual": bt, "type": "drillthrough", "destination": "Detail",
               "enabled_tooltip": "See the region", "disabled_tooltip": "Pick a region first"}),
    );
    let link = &find(report(&s), &bt).visual_extra["visualContainerObjects"]["visualLink"][0]["properties"];
    assert_eq!(
        link["enabledTooltip"]["expr"]["Literal"]["Value"],
        "'See the region'"
    );
    assert_eq!(
        link["disabledTooltip"]["expr"]["Literal"]["Value"],
        "'Pick a region first'"
    );
    let b = snap_visual(&s, &bt)["interaction"]["button"].clone();
    assert_eq!(b["enabled"], false, "{b}");
    assert_eq!(b["tooltip"], "Pick a region first", "{b}");
    assert_eq!(
        snap_visual(&s, &bt)["action"]["disabled_tooltip"],
        "Pick a region first"
    );
    // a selected data point enables it
    run(
        &mut s,
        "view.interact",
        json!({"action": "select", "page": main, "visual": chart, "point": [{"c": 0, "v": "East"}]}),
    );
    let b = snap_visual(&s, &bt)["interaction"]["button"].clone();
    assert_eq!(b["enabled"], true, "{b}");
    assert_eq!(b["tooltip"], "See the region", "{b}");
    let r = run(
        &mut s,
        "view.interact",
        json!({"action": "button", "page": main, "visual": bt}),
    );
    assert_eq!(r["page"], page(&s, 1), "{r}");
}

#[test]
fn button_shape_and_navigator_states_reach_the_snapshot_and_the_files() {
    let mut s = session();
    let main = page(&s, 0);
    let bt = add_button(&mut s, &main, "blank");
    run(
        &mut s,
        "visual.format",
        json!({"page": main, "visual": bt, "objects": [
            {"object": "text", "property": "text", "value": "Go"},
            {"object": "fill", "property": "fillColor", "value": {"color": "#112233"}},
            {"object": "fill", "property": "fillColor", "value": {"color": "#445566"}, "state": "hover"},
            {"object": "outline", "property": "show", "value": true, "state": "press"},
            {"object": "outline", "property": "lineColor", "value": {"color": "#778899"}, "state": "press"},
            {"object": "text", "property": "fontColor", "value": {"color": "#AAAAAA"}, "state": "disabled"},
        ]}),
    );
    let b = snap_visual(&s, &bt)["button"].clone();
    assert_eq!(b["text"], "Go", "{b}");
    assert_eq!(b["fill"], "#112233", "{b}");
    assert_eq!(b["states"]["hover"], json!({"fill": "#445566"}), "{b}");
    assert_eq!(
        b["states"]["press"],
        json!({"show_outline": true, "outline": "#778899"}),
        "{b}"
    );
    assert_eq!(b["states"]["disabled"], json!({"font_color": "#AAAAAA"}), "{b}");
    // a shape's text and its press state
    let sh = run(&mut s, "shape.add", json!({"page": main, "shape": "rectangle"}))["visual"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut s,
        "visual.format",
        json!({"page": main, "visual": sh, "objects": [
            {"object": "text", "property": "show", "value": true},
            {"object": "text", "property": "text", "value": "Hello"},
            {"object": "text", "property": "horizontalAlignment", "value": "left"},
            {"object": "fill", "property": "fillColor", "value": {"color": "#010203"}, "state": "press"},
            {"object": "text", "property": "fontColor", "value": {"color": "#FFFFFF"}, "state": "press"},
        ]}),
    );
    let shape = snap_visual(&s, &sh)["shape"].clone();
    assert_eq!(shape["text"]["text"], "Hello", "{shape}");
    assert_eq!(shape["text"]["align"], "left", "{shape}");
    assert_eq!(shape["states"]["press"]["fill"], "#010203", "{shape}");
    assert_eq!(
        shape["states"]["press"]["text"]["font_color"], "#FFFFFF",
        "{shape}"
    );
    // a navigator's hover state
    let nav = add_button(&mut s, &main, "page_navigator");
    run(
        &mut s,
        "visual.format",
        json!({"page": main, "visual": nav, "objects": [
            {"object": "fill", "property": "fillColor", "value": {"color": "#0000FF"}, "state": "hover"},
            {"object": "text", "property": "bold", "value": true, "state": "press"},
        ]}),
    );
    let styles = snap_visual(&s, &nav)["navigator"]["styles"].clone();
    assert_eq!(styles["hover"]["fill"], "#0000FF", "{styles}");
    assert_eq!(styles["press"]["bold"], true, "{styles}");
    // PBIP and PBIX keep every state entry
    let dir = TempDir::new("engine-buttons-states");
    for file in ["Shop.pbip", "Shop.pbix"] {
        let back = reopened(&mut s, &dir, file);
        for name in [&bt, &sh, &nav] {
            assert_eq!(
                find(report(&back), name).visual_extra["objects"],
                find(report(&s), name).visual_extra["objects"],
                "{file} {name}"
            );
        }
        assert_eq!(snap_visual(&back, &bt)["button"], b, "{file}");
        assert_eq!(snap_visual(&back, &sh)["shape"], shape, "{file}");
    }
}

/// A navigator's buttons' tile shape (`shape`, without selector) and their glow and shadow by
/// state (the report theme schema's `glow` and `shadow` objects) reach the snapshot (G55).
#[test]
fn navigator_shape_glow_and_shadow_reach_the_snapshot() {
    let mut s = session();
    let main = page(&s, 0);
    let nav = add_button(&mut s, &main, "page_navigator");
    run(
        &mut s,
        "visual.format",
        json!({"page": main, "visual": nav, "objects": [
            {"object": "shape", "property": "tileShape", "value": "pill"},
            {"object": "glow", "property": "show", "value": true, "state": "hover"},
            {"object": "glow", "property": "color", "value": {"color": "#00FF00"}, "state": "hover"},
            {"object": "glow", "property": "shadowBlur", "value": 8, "state": "hover"},
            {"object": "shadow", "property": "show", "value": true, "state": "default"},
            {"object": "shadow", "property": "shadowDistance", "value": 6, "state": "default"},
            {"object": "shadow", "property": "angle", "value": 90, "state": "default"},
            {"object": "shadow", "property": "transparency", "value": 20, "state": "default"},
        ]}),
    );
    let n = snap_visual(&s, &nav)["navigator"].clone();
    assert_eq!(n["shape"], json!({"tile": "pill"}), "{n}");
    assert_eq!(
        n["styles"]["hover"],
        json!({"show_glow": true, "glow": "#00FF00", "glow_blur": 8}),
        "{n}"
    );
    let d = &n["styles"]["default"];
    assert_eq!(d["show_shadow"], true, "{n}");
    assert_eq!(
        (
            &d["shadow_distance"],
            &d["shadow_angle"],
            &d["shadow_transparency"]
        ),
        (&json!(6), &json!(90), &json!(20)),
        "{n}"
    );
}

#[test]
fn a_bookmark_navigator_allows_deselection() {
    let mut s = session();
    let main = page(&s, 0);
    run(&mut s, "page.set_active", json!({"page": main}));
    let first = run(&mut s, "bookmark.capture", json!({"display_name": "All"}))["bookmark"]
        .as_str()
        .unwrap()
        .to_string();
    let none = run(&mut s, "bookmark.capture", json!({"display_name": "None"}))["bookmark"]
        .as_str()
        .unwrap()
        .to_string();
    let nav = add_button(&mut s, &main, "bookmark_navigator");
    run(
        &mut s,
        "visual.format",
        json!({"page": main, "visual": nav, "objects": [
            {"object": "bookmarks", "property": "allowDeselectionBookmark", "value": true},
            {"object": "bookmarks", "property": "deselectionBookmark", "value": none},
            {"object": "bookmarks", "property": "hideDeselectedBookmark", "value": true},
        ]}),
    );
    let n = snap_visual(&s, &nav)["navigator"].clone();
    assert_eq!(n["allow_deselection"], true, "{n}");
    assert_eq!(n["deselection_bookmark"], none, "{n}");
    assert_eq!(n["hide_deselection_bookmark"], true, "{n}");
    // the reader applies a bookmark, then deselects it
    let r = run(
        &mut s,
        "view.interact",
        json!({"action": "bookmark", "bookmark": first}),
    );
    assert_eq!(r["state"]["bookmark"], first, "{r}");
    let r = run(
        &mut s,
        "view.interact",
        json!({"action": "bookmark_deselect", "page": main, "visual": nav}),
    );
    assert!(r["state"]["bookmark"].is_null(), "{r}");
    assert_eq!(r["changed"], true, "{r}");
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
fn a_legacy_layout_and_pbir_parts_keep_states_and_conditional_actions() {
    let dir = TempDir::new("engine-buttons-legacy");
    for pbir in [false, true] {
        let label = if pbir { "pbir parts" } else { "legacy layout" };
        let path = synthetic(&dir, pbir);
        let mut s = open(&path);
        let main = page(&s, 0);
        let measure = s.snapshot()["model"]["tables"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|t| t["measures"].as_array().cloned().unwrap_or_default())
            .find_map(|m| m["name"].as_str().map(str::to_string));
        let bt = add_button(&mut s, &main, "blank");
        let mut action = json!({"page": main, "visual": bt, "type": "web_url",
            "enabled_tooltip": "On", "disabled_tooltip": "Off"});
        match &measure {
            Some(m) => action["destination_field"] = json!(format!("[{m}]")),
            None => action["url"] = json!("https://example.com/"),
        }
        run(&mut s, "visual.set_action", action);
        run(
            &mut s,
            "visual.format",
            json!({"page": main, "visual": bt, "objects": [
                {"object": "fill", "property": "fillColor", "value": {"color": "#445566"}, "state": "hover"},
                {"object": "text", "property": "text", "value": "Go", "state": "press"},
            ]}),
        );
        let want = find(report(&s), &bt).visual_extra.clone();
        run(&mut s, "file.save", json!({}));
        let back = open(&path);
        let got = &find(report(&back), &bt).visual_extra;
        assert_eq!(got["objects"], want["objects"], "{label}");
        assert_eq!(
            got["visualContainerObjects"], want["visualContainerObjects"],
            "{label}"
        );
        let b = snap_visual(&back, &bt)["button"].clone();
        assert_eq!(b["states"]["hover"]["fill"], "#445566", "{label}: {b}");
        assert_eq!(b["states"]["press"]["text"], "Go", "{label}: {b}");
    }
}
