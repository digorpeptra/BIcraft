//! A decomposition tree's controls on the canvas (G34, egui_kittest over an engine session):
//! the "+" opens the "Expand by" popup from the mouse or the keyboard and adds the level
//! chosen, the scroll arrows move a long level's window and a wide tree's levels, the header's
//! cross removes a level, all labelled in English and French.

use std::path::Path;

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::engine_shell;
use super::harness;
use crate::App;
use crate::shell::Caps;

/// Sales(Region, Category, Amount): twelve regions, two categories; on the page a tree `tree`
/// of [Total] by Region and Category, 450 px wide (two levels do not fit).
fn tree_session() -> Session {
    let mut s = Session::new();
    let rows: Vec<String> = (0..12)
        .flat_map(|i| {
            [("Bikes", 10 + i), ("Toys", 3 + i)].map(|(c, a)| format!(r#"{{"R{i:02}", "{c}", {a}}}"#))
        })
        .collect();
    let sales = format!(
        r#"#table({{"Region", "Category", "Amount"}}, {{{}}})"#,
        rows.join(", ")
    );
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Region", "data_type": "string"}),
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
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
        ),
        ("report.new", json!({})),
    ] {
        run(&mut s, id, p);
    }
    let page = s
        .report
        .as_ref()
        .map(|r| r.pages[0].name.clone())
        .unwrap_or_default();
    let r = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "decompositionTreeVisual", "x": 20, "y": 20, "width": 450, "height": 420, "fields": []}),
    );
    let made = r["visual"].as_str().unwrap_or_default().to_string();
    if let Some(v) = s
        .report
        .as_mut()
        .and_then(|r| r.pages.first_mut())
        .and_then(|p| p.visuals.iter_mut().find(|v| v.name == made))
    {
        v.name = "tree".into();
    }
    for (role, field) in [
        ("Analyze", "[Total]"),
        ("ExplainBy", "Sales[Region]"),
        ("ExplainBy", "Sales[Category]"),
    ] {
        run(
            &mut s,
            "visual.bind",
            json!({"page": page, "visual": "tree", "role": role, "field": field}),
        );
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn app(locale: &str) -> Harness<'static, App> {
    let mut app = App::new(locale, engine_shell(Caps::DESKTOP, tree_session)).with_sync_render();
    app.prefs.panes.data = false;
    app.prefs.panes.build = false;
    app.prefs.panes.filters = false;
    app.open_path(Path::new("tree.pbip"));
    let mut h = harness(app, (1400.0, 900.0));
    for _ in 0..40 {
        h.step();
        if h.state().shown.is_some() {
            break;
        }
    }
    assert!(h.state().shown.is_some(), "the page was never drawn");
    h.step();
    h
}

fn settle(h: &mut Harness<'_, App>) {
    for _ in 0..60 {
        h.step();
        if h.state().calls_pending() == 0 {
            break;
        }
    }
    h.step();
}

/// The tree's `data.decomp`.
fn decomp(h: &Harness<'_, App>) -> Value {
    let snap = h
        .state()
        .loaded
        .as_ref()
        .map(|l| (*l.snap).clone())
        .unwrap_or(Value::Null);
    snap["report"]["pages"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|p| p["visuals"].as_array().cloned().unwrap_or_default())
        .find(|v| v["name"] == "tree")
        .map(|v| v["data"]["decomp"].clone())
        .unwrap_or(Value::Null)
}

fn levels(h: &Harness<'_, App>) -> Vec<Value> {
    decomp(h)["levels"].as_array().cloned().unwrap_or_default()
}

#[test]
fn the_plus_opens_expand_by_from_the_keyboard_and_adds_the_level_chosen() {
    let mut h = app("en-US");
    assert!(levels(&h).is_empty(), "{}", decomp(&h));
    // the root's "+": a labelled button, reached by focus and pressed with Enter
    let plus = h.get_by_label("Add a level");
    assert_eq!(plus.accesskit_node().role(), egui::accesskit::Role::Button);
    plus.focus();
    h.step();
    h.key_press(egui::Key::Enter);
    settle(&mut h);
    // the popup: the two AI splits, then the fields; the first item has the focus
    assert!(h.query_by_label("Expand by").is_some());
    for item in ["High value", "Low value", "Region", "Category"] {
        assert!(h.query_by_label(item).is_some(), "{item}");
    }
    assert!(h.get_by_label("High value").is_focused());
    // Escape closes it without a change
    h.key_press(egui::Key::Escape);
    settle(&mut h);
    assert!(h.query_by_label("Expand by").is_none());
    assert!(levels(&h).is_empty());
    // again, and Region is chosen: the level is added through view.interact
    h.get_by_label("Add a level").click();
    settle(&mut h);
    h.get_by_label("Region").click();
    settle(&mut h);
    assert_eq!(
        h.state().last_action.as_ref().map(|a| a["action"].clone()),
        Some(json!("decomp_expand"))
    );
    let l = levels(&h);
    assert_eq!(l.len(), 1, "{}", decomp(&h));
    assert_eq!(l[0]["total"], 12);
    assert!(h.query_by_label("Expand by").is_none());
    // its header's cross removes it
    h.get_by_label("Remove the level Region").click();
    settle(&mut h);
    assert!(levels(&h).is_empty());
}

#[test]
fn scroll_arrows_move_a_long_level_and_a_wide_tree() {
    let mut h = app("en-US");
    h.get_by_label("Add a level").click();
    settle(&mut h);
    h.get_by_label("Region").click();
    settle(&mut h);
    // twelve regions do not fit: Desktop's first window, with an arrow on each side
    assert!(h.query_by_label("Scroll Region up").is_some());
    h.get_by_label("Scroll Region down").click();
    settle(&mut h);
    let first = levels(&h)[0]["start"]
        .as_u64()
        .unwrap_or_else(|| panic!("{}", decomp(&h)));
    h.get_by_label("Scroll Region up").click();
    settle(&mut h);
    let back = levels(&h)[0]["start"].as_u64().unwrap_or(u64::MAX);
    assert!(back < first, "{back} {first}");
    // a member of the window, then Category under it: two levels are wider than the visual
    let member = h
        .query_all_by_label_contains("Add a level under")
        .next()
        .map(|n| n.accesskit_node().label().unwrap_or_default().to_string())
        .unwrap_or_else(|| panic!("no + under a member"));
    h.get_by_label(&member).click();
    settle(&mut h);
    h.get_by_label("Category").click();
    settle(&mut h);
    assert_eq!(levels(&h).len(), 2, "{}", decomp(&h));
    assert!(h.query_by_label("Scroll the tree left").is_none());
    h.get_by_label("Scroll the tree right").click();
    settle(&mut h);
    assert_eq!(decomp(&h)["shift"], 1);
    h.get_by_label("Scroll the tree left").click();
    settle(&mut h);
    assert_eq!(decomp(&h)["shift"], 0);
    // nothing was written into the report
    assert!(
        !h.state()
            .sent
            .iter()
            .any(|c| c.id != "view.interact" && c.id.starts_with("visual."))
    );
}

#[test]
fn the_controls_are_labelled_in_french() {
    let mut h = app("fr-FR");
    h.get_by_label("Ajouter un niveau").click();
    settle(&mut h);
    assert!(h.query_by_label("Développer par").is_some());
    assert!(h.query_by_label("Valeur haute").is_some());
    h.get_by_label("Region").click();
    settle(&mut h);
    assert!(h.query_by_label("Faire défiler Region vers le bas").is_some());
    assert!(h.query_by_label("Retirer le niveau Region").is_some());
}
