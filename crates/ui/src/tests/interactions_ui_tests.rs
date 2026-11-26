//! The Edit interactions mode (`interactions.rs`, ROADMAP M11.15) on an engine session: the
//! ribbon toggle and Escape, the icon sets on the targets only, the marked mode against the
//! engine's own (a data point clicked in the source), a click that sets one rule and Ctrl+Z, the
//! modes a pair cannot take left out, the report's cross-filter default, a reader kept out, the
//! same frames in both shells.

use std::path::Path;

use bicraft_engine::Session;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::panes_ui_tests::{select, visual_named};
use super::{frame, harness};
use crate::App;
use crate::interactions::{self, Choice, Mode};
use crate::registry::Tab as RTab;
use crate::shell::Caps;

/// Sales(Category, Amount) with [Total]; one page with a column chart, a pie, a card, a table
/// and a slicer.
fn shop_session() -> Session {
    let mut s = Session::new();
    let sales =
        r#"#table({"Category", "Amount"}, {{"Wheels", 100}, {"Wheels", 50}, {"Toys", 5}, {"Games", 20}})"#;
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
        s.execute(id, &p).unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let cat_total =
        json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]);
    for (ty, rect, fields) in [
        ("clusteredColumnChart", [20, 20, 600, 300], cat_total.clone()),
        ("pieChart", [640, 20, 400, 300], cat_total),
        (
            "card",
            [20, 340, 300, 150],
            json!([{"role": "Values", "field": "[Total]"}]),
        ),
        (
            "tableEx",
            [340, 340, 400, 300],
            json!([{"role": "Values", "field": "Sales[Category]"}, {"role": "Values", "field": "[Total]"}]),
        ),
        (
            "slicer",
            [760, 340, 280, 300],
            json!([{"role": "Values", "field": "Sales[Category]"}]),
        ),
    ] {
        let out = s
            .execute(
                "visual.add",
                &json!({"page": page, "visual_type": ty, "x": rect[0], "y": rect[1], "width": rect[2],
                        "height": rect[3], "fields": fields}),
            )
            .unwrap();
        // distinct titles: the icons are labelled by them
        let v = out["visual"].as_str().unwrap().to_string();
        s.execute(
            "visual.set_title",
            &json!({"page": page, "visual": v, "title": format!("The {ty}")}),
        )
        .unwrap();
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn open_in(caps: Caps, reader: bool) -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(caps, shop_session)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1600.0, 900.0));
    steps(&mut h, 6);
    if reader {
        h.state_mut().access = Some("reader".into());
    }
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().placed.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    h
}

fn open() -> Harness<'static, App> {
    open_in(Caps::DESKTOP, false)
}

fn page(h: &Harness<'_, App>) -> Value {
    h.state().loaded.as_ref().unwrap().snap["report"]["pages"][0].clone()
}

fn visual(h: &Harness<'_, App>, name: &str) -> Value {
    page(h)["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == name)
        .cloned()
        .unwrap()
}

/// The icon labels drawn now ("Filter: Total"…), from the accessibility tree.
fn icon_labels(h: &Harness<'_, App>) -> Vec<String> {
    let modes = ["Filter: ", "Highlight: ", "None: "];
    h.get_all_by_role(egui::accesskit::Role::Button)
        .filter_map(|n| n.accesskit_node().label())
        .filter(|l| modes.iter().any(|m| l.starts_with(m)))
        .collect()
}

fn label(h: &Harness<'_, App>, mode: Mode, target: &str) -> String {
    interactions::icon_label(
        mode,
        &interactions::title(&visual(h, target)),
        crate::i18n::Lang::En,
    )
}

/// Whether the node labelled `l` shows itself pressed (selected or toggled).
fn pressed(h: &Harness<'_, App>, l: &str) -> bool {
    let n = h.get_by_label(l);
    let a = n.accesskit_node();
    a.is_selected() == Some(true) || a.toggled() == Some(egui::accesskit::Toggled::True)
}

/// The Edit interactions button on the Visual tab.
fn toggle_button(h: &mut Harness<'_, App>) {
    h.state_mut().select_ribbon_tab(RTab::Visual);
    steps(h, 2);
    h.get_by_label("Edit interactions").click();
    steps(h, 3);
}

/// The choice the UI shows for `target` with `source` selected.
fn choice_of(h: &Harness<'_, App>, source: &str, target: &str) -> Option<Choice> {
    let snap = h.state().loaded.as_ref().unwrap().snap.clone();
    let cf = snap["report"]["settings"]["default_interaction"] == "filter";
    interactions::choice(&page(h), &visual(h, source), &visual(h, target), cf)
}

fn names(h: &Harness<'_, App>) -> Vec<String> {
    page(h)["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap().to_string())
        .collect()
}

/// After a click on a data point of the source, the engine's mode of every target is the one
/// the icons mark (`interaction.mode`; no `interaction` with None).
fn assert_marks_match_the_engine(h: &mut Harness<'_, App>, source: &str) {
    let pg = page(h)["name"].as_str().unwrap().to_string();
    h.state_mut().interact(
        json!({"action": "select", "page": pg, "visual": source, "point": [{"c": 0, "v": "Wheels"}]}),
    );
    steps(h, 4);
    for t in names(h) {
        let Some(c) = choice_of(h, source, &t) else {
            continue;
        };
        let got = visual(h, &t)["interaction"]["mode"].as_str().map(str::to_string);
        let want = match c.current {
            Mode::None => None,
            m => Some(m.word().to_string()),
        };
        assert_eq!(got, want, "{t}: the icons mark {:?}", c.current);
    }
}

#[test]
fn the_ribbon_button_toggles_the_mode_and_icons_sit_on_the_targets() {
    let mut h = open();
    let col = visual_named(&h, "clusteredColumnChart");
    let (pie, card, table, slicer) = (
        visual_named(&h, "pieChart").visual,
        visual_named(&h, "card").visual,
        visual_named(&h, "tableEx").visual,
        visual_named(&h, "slicer").visual,
    );
    select(&mut h, &col);
    assert!(icon_labels(&h).is_empty(), "no icons before the mode");
    toggle_button(&mut h);
    assert!(h.state().cmd.edit_interactions);
    assert!(pressed(&h, "Edit interactions"));
    // the source has none; the pie three; the card, the table and the slicer cannot highlight
    let got = icon_labels(&h);
    assert_eq!(got.len(), 3 + 2 + 2 + 2, "{got:?}");
    assert!(
        !got.iter()
            .any(|l| l.ends_with(&interactions::title(&visual(&h, &col.visual))))
    );
    for m in [Mode::Filter, Mode::Highlight, Mode::None] {
        h.get_by_label(&label(&h, m, &pie));
    }
    for t in [&card, &table, &slicer] {
        h.get_by_label(&label(&h, Mode::Filter, t));
        h.get_by_label(&label(&h, Mode::None, t));
        assert!(h.query_by_label(&label(&h, Mode::Highlight, t)).is_none(), "{t}");
    }
    // the defaults are marked: Highlight on the pie, Filter on the others
    let marked = |h: &Harness<'_, App>, m: Mode, t: &str| pressed(h, &label(h, m, t));
    assert!(marked(&h, Mode::Highlight, &pie) && !marked(&h, Mode::Filter, &pie));
    assert!(marked(&h, Mode::Filter, &card));
    let c = choice_of(&h, &col.visual, &pie).unwrap();
    assert!(!c.explicit && c.default == Mode::Highlight);
    assert_marks_match_the_engine(&mut h, &col.visual);
    // a slicer as the source only filters: two icons on the pie too
    select(
        &mut h,
        &crate::ribbon::VisualSel {
            page: col.page.clone(),
            visual: slicer.clone(),
        },
    );
    assert!(h.query_by_label(&label(&h, Mode::Highlight, &pie)).is_none());
    h.get_by_label(&label(&h, Mode::Filter, &pie));
    // the button again: off, the selection made while editing cleared, no icons
    select(&mut h, &col);
    let pg = col.page.clone();
    h.state_mut().interact(
        json!({"action": "select", "page": pg, "visual": col.visual, "point": [{"c": 0, "v": "Toys"}]}),
    );
    steps(&mut h, 4);
    assert!(
        visual(&h, &col.visual)["interaction"]["selected"].is_array()
            || visual(&h, &col.visual)["interaction"]["selected"].is_object()
    );
    toggle_button(&mut h);
    assert!(!h.state().cmd.edit_interactions);
    steps(&mut h, 4);
    assert!(icon_labels(&h).is_empty());
    assert!(
        visual(&h, &col.visual)["interaction"].get("selected").is_none(),
        "the live preview is reset"
    );
    // Escape leaves the mode, keeping the source selected
    toggle_button(&mut h);
    assert!(h.state().cmd.edit_interactions);
    h.key_press(Key::Escape);
    steps(&mut h, 3);
    assert!(!h.state().cmd.edit_interactions);
    assert_eq!(h.state().cmd.visual.as_ref(), Some(&col));
    // the palette still opens the generic form
    assert!(crate::registry::find("page.interactions").is_some());
}

#[test]
fn a_click_on_an_icon_sets_one_rule_and_undo_restores() {
    let mut h = open();
    let col = visual_named(&h, "clusteredColumnChart");
    let (pie, card) = (
        visual_named(&h, "pieChart").visual,
        visual_named(&h, "card").visual,
    );
    select(&mut h, &col);
    toggle_button(&mut h);
    let n = h.state().sent.len();
    h.get_by_label(&label(&h, Mode::Filter, &pie)).click();
    steps(&mut h, 4);
    let sent: Vec<_> = h.state().sent[n..].iter().collect();
    assert_eq!(sent.len(), 1, "one command");
    assert_eq!(sent[0].id, "page.interactions");
    assert_eq!(
        sent[0].params["rules"],
        json!([{"source": col.visual, "target": pie, "type": "filter"}])
    );
    assert_eq!(
        page(&h)["interactions"],
        json!([{"source": col.visual, "target": pie, "type": "filter"}])
    );
    let c = choice_of(&h, &col.visual, &pie).unwrap();
    assert!(c.explicit && c.current == Mode::Filter);
    assert!(pressed(&h, &label(&h, Mode::Filter, &pie)));
    // None on the card: a second rule
    h.get_by_label(&label(&h, Mode::None, &card)).click();
    steps(&mut h, 4);
    assert_eq!(page(&h)["interactions"].as_array().unwrap().len(), 2);
    assert_marks_match_the_engine(&mut h, &col.visual);
    // the marked icon again: nothing is sent
    let n = h.state().sent.len();
    h.get_by_label(&label(&h, Mode::None, &card)).click();
    steps(&mut h, 3);
    assert_eq!(h.state().sent.len(), n);
    // Ctrl+Z: one step back, the card's rule goes
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    steps(&mut h, 4);
    assert_eq!(
        page(&h)["interactions"],
        json!([{"source": col.visual, "target": pie, "type": "filter"}])
    );
    // the default (Highlight) removes the pie's rule
    h.get_by_label(&label(&h, Mode::Highlight, &pie)).click();
    steps(&mut h, 4);
    assert_eq!(
        h.state().sent.last().unwrap().params["rules"][0]["type"],
        "default"
    );
    assert_eq!(page(&h)["interactions"], json!([]));
}

#[test]
fn the_report_can_cross_filter_by_default() {
    let mut h = open();
    let col = visual_named(&h, "clusteredColumnChart");
    let pie = visual_named(&h, "pieChart").visual;
    h.state_mut().run(
        crate::actions::Call {
            id: "report.settings",
            params: json!({"default_interaction": "filter"}),
        },
        crate::app::Then::Nothing,
    );
    steps(&mut h, 4);
    select(&mut h, &col);
    toggle_button(&mut h);
    let c = choice_of(&h, &col.visual, &pie).unwrap();
    assert_eq!(
        (c.default, c.current, c.explicit),
        (Mode::Filter, Mode::Filter, false)
    );
    assert!(pressed(&h, &label(&h, Mode::Filter, &pie)));
    // Highlight is still offered and becomes a rule
    h.get_by_label(&label(&h, Mode::Highlight, &pie)).click();
    steps(&mut h, 4);
    assert_eq!(
        h.state().sent.last().unwrap().params["rules"][0]["type"],
        "highlight"
    );
    assert_marks_match_the_engine(&mut h, &col.visual);
}

#[test]
fn a_reader_cannot_enter_the_mode() {
    let mut h = open_in(Caps::DESKTOP, true);
    let col = visual_named(&h, "clusteredColumnChart");
    select(&mut h, &col);
    h.state_mut().select_ribbon_tab(RTab::Visual);
    steps(&mut h, 2);
    // the button is greyed (or not drawn): a click does nothing
    if let Some(b) = h.query_by_label("Edit interactions") {
        b.click();
    }
    steps(&mut h, 3);
    assert!(!h.state().cmd.edit_interactions);
    crate::interactions::toggle(h.state_mut());
    steps(&mut h, 3);
    assert!(!h.state().cmd.edit_interactions, "the role refuses it");
    assert!(icon_labels(&h).is_empty());
    // a session that turns read-only leaves the mode
    let mut h = open();
    let col = visual_named(&h, "clusteredColumnChart");
    select(&mut h, &col);
    toggle_button(&mut h);
    assert!(h.state().cmd.edit_interactions);
    h.state_mut().access = Some("reader".into());
    steps(&mut h, 3);
    assert!(!h.state().cmd.edit_interactions);
}

#[test]
fn the_choice_follows_the_capabilities() {
    let src = json!({"name": "s", "interactivity": {"target": true, "highlight": true, "slicer": false}});
    let slicer = json!({"name": "s", "interactivity": {"target": true, "highlight": false, "slicer": true}});
    let bar = json!({"name": "t", "interactivity": {"target": true, "highlight": true, "slicer": false}});
    let card = json!({"name": "t", "interactivity": {"target": true, "highlight": false, "slicer": false}});
    let text = json!({"name": "t", "interactivity": {"target": false, "highlight": false, "slicer": false}});
    let none = json!({});
    let c = interactions::choice(&none, &src, &bar, false).unwrap();
    assert_eq!(c.modes, vec![Mode::Filter, Mode::Highlight, Mode::None]);
    assert_eq!(
        (c.default, c.current, c.explicit),
        (Mode::Highlight, Mode::Highlight, false)
    );
    assert_eq!(
        interactions::choice(&none, &src, &bar, true).unwrap().default,
        Mode::Filter
    );
    assert_eq!(
        interactions::choice(&none, &src, &card, false).unwrap().modes,
        vec![Mode::Filter, Mode::None]
    );
    assert_eq!(
        interactions::choice(&none, &slicer, &bar, false).unwrap().default,
        Mode::Filter
    );
    assert!(interactions::choice(&none, &src, &text, false).is_none());
    assert!(
        interactions::choice(&none, &src, &src, false).is_none(),
        "not itself"
    );
    let hidden = json!({"name": "t", "hidden": true, "interactivity": {"target": true}});
    assert!(interactions::choice(&none, &src, &hidden, false).is_none());
    // a Highlight rule on a card acts as Filter (the viewer's rule), set explicitly
    let page = json!({"interactions": [{"source": "s", "target": "t", "type": "highlight"}]});
    let c = interactions::choice(&page, &src, &card, false).unwrap();
    assert_eq!((c.current, c.explicit), (Mode::Filter, true));
    assert_eq!(interactions::rule_for(Mode::Filter, &c), Some("default"));
    let page = json!({"interactions": [{"source": "s", "target": "t", "type": "none"}]});
    let c = interactions::choice(&page, &src, &bar, false).unwrap();
    assert_eq!(c.current, Mode::None);
    assert_eq!(interactions::rule_for(Mode::None, &c), None);
    assert_eq!(interactions::rule_for(Mode::Filter, &c), Some("filter"));
    assert_eq!(interactions::rule_for(Mode::Highlight, &c), Some("default"));
    // hostile snapshots: nothing breaks
    for bad in [
        json!(null),
        json!({"interactions": 3}),
        json!({"interactions": [1, {"type": 2}]}),
    ] {
        let _ = interactions::choice(&bad, &src, &bar, false);
        let _ = interactions::choice(&src, &bad, &bad, false);
    }
}

#[test]
fn the_mode_draws_the_same_frames_in_both_shells() {
    let mut desktop = open_in(Caps::DESKTOP, false);
    let mut browser = open_in(Caps::BROWSER, false);
    for h in [&mut desktop, &mut browser] {
        let col = visual_named(h, "clusteredColumnChart");
        select(h, &col);
        toggle_button(h);
        steps(h, 3);
    }
    let (a, b) = (frame(&desktop), frame(&browser));
    assert!(!a.0.is_empty());
    assert!(a.0 == b.0, "the meshes differ");
    assert!(a.1 == b.1, "the font atlases differ");
    assert!(a.2 == b.2, "the pages differ");
}

/// Runs command `id` through the app (as a button would) and lets the snapshot follow.
fn run(h: &mut Harness<'_, App>, id: &'static str, params: Value) {
    h.state_mut()
        .run(crate::actions::Call { id, params }, crate::app::Then::Nothing);
    steps(h, 4);
}

#[test]
fn the_icons_are_reached_and_run_by_keyboard() {
    let mut h = open();
    let col = visual_named(&h, "clusteredColumnChart");
    let pie = visual_named(&h, "pieChart").visual;
    select(&mut h, &col);
    toggle_button(&mut h);
    h.get_by_label(&label(&h, Mode::Filter, &pie)).focus();
    steps(&mut h, 2);
    assert!(h.get_by_label(&label(&h, Mode::Filter, &pie)).is_focused());
    // Tab walks the strip: Highlight, then None
    h.key_press(Key::Tab);
    steps(&mut h, 2);
    assert!(h.get_by_label(&label(&h, Mode::Highlight, &pie)).is_focused());
    h.key_press(Key::Tab);
    steps(&mut h, 2);
    assert!(h.get_by_label(&label(&h, Mode::None, &pie)).is_focused());
    // Enter runs the focused icon: one rule
    h.key_press(Key::Enter);
    steps(&mut h, 4);
    assert_eq!(
        page(&h)["interactions"],
        json!([{"source": col.visual, "target": pie, "type": "none"}])
    );
    assert!(h.state().cmd.edit_interactions, "the mode stays on");
}

#[test]
fn a_group_is_a_source_for_each_of_its_visuals() {
    let mut h = open();
    let col = visual_named(&h, "clusteredColumnChart");
    let pie = visual_named(&h, "pieChart").visual;
    let (card, table) = (
        visual_named(&h, "card").visual,
        visual_named(&h, "tableEx").visual,
    );
    let pg = col.page.clone();
    run(
        &mut h,
        "visual.group",
        json!({"page": pg, "visuals": [col.visual, pie]}),
    );
    let group = page(&h)["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| interactions::is_group(v))
        .map(|v| v["name"].as_str().unwrap().to_string())
        .expect("the group");
    select(
        &mut h,
        &crate::ribbon::VisualSel {
            page: pg.clone(),
            visual: group.clone(),
        },
    );
    toggle_button(&mut h);
    // the group's own visuals take no icons; the card, the table and the slicer do
    let got = icon_labels(&h);
    assert_eq!(got.len(), 2 + 2 + 2, "{got:?}");
    assert!(h.query_by_label(&label(&h, Mode::Filter, &pie)).is_none());
    // a click writes the rule for every visual of the group
    h.get_by_label(&label(&h, Mode::None, &card)).click();
    steps(&mut h, 4);
    let mut rules: Vec<Value> = page(&h)["interactions"].as_array().unwrap().clone();
    rules.sort_by_key(|r| r["source"].as_str().unwrap().to_string());
    let mut want = vec![
        json!({"source": col.visual, "target": card, "type": "none"}),
        json!({"source": pie, "target": card, "type": "none"}),
    ];
    want.sort_by_key(|r| r["source"].as_str().unwrap().to_string());
    assert_eq!(rules, want);
    assert!(pressed(&h, &label(&h, Mode::None, &card)));
    // one visual of the group set apart: the table's icons mark nothing, a click unifies
    run(
        &mut h,
        "page.interactions",
        json!({"page": pg, "rules": [{"source": col.visual, "target": table, "type": "none"}]}),
    );
    assert!(!pressed(&h, &label(&h, Mode::Filter, &table)));
    assert!(!pressed(&h, &label(&h, Mode::None, &table)));
    h.get_by_label(&label(&h, Mode::Filter, &table)).click();
    steps(&mut h, 4);
    assert_eq!(
        h.state().sent.last().unwrap().params["rules"][0]["type"],
        "default"
    );
    assert!(pressed(&h, &label(&h, Mode::Filter, &table)));
    assert_eq!(page(&h)["interactions"].as_array().unwrap().len(), 2);
}

#[test]
fn targets_with_the_same_title_get_distinct_names() {
    let mut h = open();
    let col = visual_named(&h, "clusteredColumnChart");
    let (card, table) = (
        visual_named(&h, "card").visual,
        visual_named(&h, "tableEx").visual,
    );
    for v in [&card, &table] {
        run(
            &mut h,
            "visual.set_title",
            json!({"page": col.page, "visual": v, "title": "Same"}),
        );
    }
    select(&mut h, &col);
    toggle_button(&mut h);
    let got = icon_labels(&h);
    let filters: Vec<&String> = got.iter().filter(|l| l.starts_with("Filter: Same")).collect();
    assert_eq!(filters.len(), 2, "{got:?}");
    assert_ne!(filters[0], filters[1]);
    assert!(filters.iter().all(|l| l.starts_with("Filter: Same")));
}
