//! The Model view's Data / Model pane and layout tabs on the desktop's engine session
//! (egui_kittest, no GPU; ROADMAP lot 18, docs/gaps.md G61): the Model explorer's tree selects
//! in the Properties pane and its menus run the engine's commands, a calculation item dragged
//! onto another moves there (`calculation_item.set`, one undo step), and the layout tabs under the
//! diagram add, show, fill, rename and delete layouts (`diagram.add`, `diagram.move`,
//! `diagram.rename`, `diagram.remove`).

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use egui::{Event, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::model_tests::{build_star, open_model};
use super::{Temp, click, wait};
use crate::App;
use bicraft_ui::model_view::Sel;

fn model(app: &App) -> Value {
    app.loaded.as_ref().unwrap().snap["model"].clone()
}

/// The star schema with a calculation group (Time: Current, YTD, PY), a role, a culture and a
/// shared expression, saved as a PBIP project.
pub fn rich_star(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    build_star(&mut s);
    let run = |s: &mut Session, id: &str, p: Value| {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e:?}"));
    };
    run(&mut s, "calculation_group.add", json!({"name": "Time"}));
    for n in ["Current", "YTD", "PY"] {
        run(
            &mut s,
            "calculation_item.add",
            json!({"group": "Time", "name": n, "expression": "SELECTEDMEASURE()"}),
        );
    }
    run(&mut s, "role.add", json!({"name": "Readers"}));
    run(&mut s, "culture.add", json!({"name": "fr-FR"}));
    run(
        &mut s,
        "expression.add",
        json!({"name": "Folder", "expression": "\"data\""}),
    );
    run(&mut s, "report.new", json!({}));
    let path = dir.join("star.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

/// The rectangle of the right-most widget labelled `label`, the highest of those (the Data / Model pane is the
/// right-most panel; the Properties pane and the diagram may show the same names).
pub fn pane_rect(h: &Harness<'_, App>, label: &str) -> Rect {
    h.query_all_by_label(label)
        .map(|n| n.rect())
        .max_by(|a, b| a.min.x.total_cmp(&b.min.x).then(b.min.y.total_cmp(&a.min.y)))
        .unwrap_or_else(|| panic!("no `{label}` in the window"))
}

/// The lowest widget labelled `label` (the layout tabs are under the diagram).
fn lowest_rect(h: &Harness<'_, App>, label: &str) -> Rect {
    h.query_all_by_label(label)
        .map(|n| n.rect())
        .max_by(|a, b| a.min.y.total_cmp(&b.min.y))
        .unwrap_or_else(|| panic!("no `{label}` in the window"))
}

/// The item `label` of the open context menu (a ribbon button may have the same text: the menu
/// is below the ribbon).
fn menu_item(h: &Harness<'_, App>, label: &str) -> Rect {
    lowest_rect(h, label)
}

pub fn right_click(h: &mut Harness<'_, App>, pos: Pos2) {
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.step();
}

pub fn click_pane(h: &mut Harness<'_, App>, label: &str) {
    let r = pane_rect(h, label);
    click(h, r.center(), false);
    h.step();
}

/// A press at `from`, moves to `to` in steps, a release there.
fn drag(h: &mut Harness<'_, App>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.step();
    h.event(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    for k in 1..=6 {
        h.hover_at(from + (to - from) * (k as f32 / 6.0));
        h.step();
    }
    h.event(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.step();
}

fn items(app: &App) -> Vec<String> {
    model(app)["tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "Time")
        .unwrap()["calculation_items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

fn revision(app: &App) -> u64 {
    app.loaded
        .as_ref()
        .map_or(0, |l| l.snap["revision"].as_u64().unwrap_or(0))
}

#[test]
fn the_model_explorer_lists_the_model_and_selects_in_the_properties_pane() {
    let dir = Temp::new("model-explorer");
    let path = rich_star(dir.path());
    let mut h = open_model(&path, "en-US", (1500.0, 900.0));
    // the Data tab first, as in Desktop: the tables
    pane_rect(&h, "Sales");
    click_pane(&mut h, "Model");
    // the semantic model's sections, with their counts
    for label in [
        "Calculation groups (1)",
        "Cultures (1)",
        "Measures (3)",
        "Perspectives (0)",
        "Relationships (4)",
        "Roles (1)",
        "Shared expressions (1)",
        "Tables (7)",
    ] {
        pane_rect(&h, label);
    }
    // a relationship: selected, the Properties pane shows it
    click_pane(&mut h, "Relationships (4)");
    let rels = model(h.state())["relationships"].clone();
    let i = rels
        .as_array()
        .unwrap()
        .iter()
        .position(|r| r["to_table"] == "Region")
        .unwrap();
    click_pane(&mut h, "Sales[RegionKey] → Region[RegionKey]");
    assert_eq!(h.state().model_view.sel, Some(Sel::Rel(i)));
    h.get_by_label("Behaviour");
    // a table, then one of its measures
    click_pane(&mut h, "Tables (7)");
    click_pane(&mut h, "Product");
    assert_eq!(h.state().model_view.sel, Some(Sel::Table("Product".into())));
    click_pane(&mut h, "Measures (3)");
    click_pane(&mut h, "Average");
    assert_eq!(
        h.state().model_view.sel,
        Some(Sel::Measure {
            table: "Sales".into(),
            name: "Average".into()
        })
    );
    // a calculation group is its table
    click_pane(&mut h, "Calculation groups (1)");
    click_pane(&mut h, "Time");
    assert_eq!(h.state().model_view.sel, Some(Sel::Table("Time".into())));

    // a role's menu: Delete role runs role.remove
    click_pane(&mut h, "Roles (1)");
    let r = pane_rect(&h, "Readers");
    right_click(&mut h, r.center());
    let item = menu_item(&h, "Delete role");
    click(&mut h, item.center(), false);
    wait(&mut h, "the role removed", |a| {
        model(a)["roles"].as_array().is_some_and(Vec::is_empty)
    });
    // a calculation item's menu: Edit calculation item opens its form with its names
    let r = pane_rect(&h, "YTD");
    right_click(&mut h, r.center());
    let item = menu_item(&h, "Edit calculation item");
    click(&mut h, item.center(), false);
    let form = h.state().form.as_ref().expect("the item's form");
    assert_eq!(form.id, "calculation_item.set");
    let text = |k: &str| {
        form.inputs
            .iter()
            .find(|i| i.field.name == k)
            .map(|i| i.text.clone())
            .unwrap()
    };
    assert_eq!(text("group"), "Time");
    assert_eq!(text("name"), "YTD");
}

#[test]
fn a_calculation_item_dragged_onto_another_takes_its_place() {
    let dir = Temp::new("model-explorer-drag");
    let path = rich_star(dir.path());
    let mut h = open_model(&path, "fr-FR", (1500.0, 900.0));
    click_pane(&mut h, "Modèle");
    click_pane(&mut h, "Groupes de calcul (1)");
    click_pane(&mut h, "Time");
    assert_eq!(items(h.state()), ["Current", "YTD", "PY"]);
    let rev = revision(h.state());
    let from = pane_rect(&h, "PY").center();
    let to = pane_rect(&h, "Current").center();
    drag(&mut h, from, to);
    wait(&mut h, "the moved item", |a| revision(a) > rev);
    assert_eq!(items(h.state()), ["PY", "Current", "YTD"]);
    // one step: Ctrl+Z puts it back
    let rev = revision(h.state());
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Z);
    wait(&mut h, "the undo", |a| revision(a) > rev);
    assert_eq!(items(h.state()), ["Current", "YTD", "PY"]);
}

fn layouts(app: &App) -> Vec<(String, Vec<String>)> {
    model(app)["diagram"]["layouts"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|d| {
            (
                d["name"].as_str().unwrap().to_string(),
                d["nodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n["table"].as_str().unwrap().to_string())
                    .collect(),
            )
        })
        .collect()
}

fn cards(app: &App) -> usize {
    app.model_view.diagram().map_or(0, |d| d.cards.len())
}

#[test]
fn layout_tabs_are_added_filled_shown_renamed_and_deleted() {
    let dir = Temp::new("model-layouts");
    let path = super::model_tests::star(dir.path());
    let mut h = open_model(&path, "en-US", (1500.0, 900.0));
    assert_eq!(cards(h.state()), 6);
    lowest_rect(&h, "All tables");
    // + adds a layout, shown at once, empty
    let plus = lowest_rect(&h, "New layout");
    click(&mut h, plus.center(), false);
    wait(&mut h, "the new layout", |a| layouts(a).len() == 2);
    assert_eq!(layouts(h.state())[1].0, "Layout 1");
    wait(&mut h, "the layout shown", |a| {
        a.model_view.shown_layout.as_deref() == Some("Layout 1")
    });
    assert_eq!(cards(h.state()), 0);
    // a table added to it from the Data pane's menu
    let r = pane_rect(&h, "Notes");
    right_click(&mut h, r.center());
    let item = menu_item(&h, "Add to layout");
    click(&mut h, item.center(), false);
    wait(&mut h, "the table in the layout", |a| {
        layouts(a).get(1).is_some_and(|l| l.1 == ["Notes"])
    });
    wait(&mut h, "its card", |a| cards(a) == 1);
    // the layout of all tables again
    let all = lowest_rect(&h, "All tables");
    click(&mut h, all.center(), false);
    wait(&mut h, "all the cards", |a| cards(a) == 6);
    // the tab's menu: Rename layout opens its form with the name, Delete layout deletes it
    let tab = lowest_rect(&h, "Layout 1");
    right_click(&mut h, tab.center());
    let item = menu_item(&h, "Rename layout");
    click(&mut h, item.center(), false);
    let form = h.state().form.as_ref().expect("the rename form");
    assert_eq!(form.id, "diagram.rename");
    assert_eq!(
        form.inputs.iter().find(|i| i.field.name == "name").unwrap().text,
        "Layout 1"
    );
    h.state_mut().form = None;
    h.step();
    let tab = lowest_rect(&h, "Layout 1");
    right_click(&mut h, tab.center());
    let item = menu_item(&h, "Delete layout");
    click(&mut h, item.center(), false);
    wait(&mut h, "the layout deleted", |a| layouts(a).len() == 1);
    assert_eq!(cards(h.state()), 6);
}
