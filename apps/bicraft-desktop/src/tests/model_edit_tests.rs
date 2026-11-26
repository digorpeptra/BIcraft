//! The Model view's editing gestures on the desktop's engine session (egui_kittest, no GPU;
//! ROADMAP lot 18, docs/gaps.md G61): drag a card (`diagram.move`, saved in the project's
//! `diagramLayout.json`), drag a column onto another table's (`relationship.add`), double-click
//! a relationship (`relationship.set`'s form), Delete (`relationship.remove`), and the
//! Properties pane's fields (`column.set`, `measure.set`, `table.set`, `relationship.set`).

use egui::{Event, Modifiers, PointerButton, Pos2, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::Value;

use super::model_tests::{open_model, star};
use super::{Temp, click, wait};
use crate::App;
use bicraft_ui::diagram;
use bicraft_ui::model_view::Sel;

fn model(app: &App) -> Value {
    app.loaded.as_ref().unwrap().snap["model"].clone()
}

fn revision(app: &App) -> u64 {
    app.loaded
        .as_ref()
        .map_or(0, |l| l.snap["revision"].as_u64().unwrap_or(0))
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
        let p = from + (to - from) * (k as f32 / 6.0);
        h.hover_at(p);
        h.step();
    }
    h.event(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
}

fn card(app: &App, table: &str) -> diagram::Card {
    let d = app.model_view.diagram().unwrap();
    d.cards[d.card_named(table).unwrap()].clone()
}

/// The screen centre of column `column`'s row in `table`'s card.
fn column_row(app: &App, table: &str, column: &str) -> Pos2 {
    let m = model(app);
    let c = card(app, table);
    let idx = m["tables"][c.table]["columns"]
        .as_array()
        .unwrap()
        .iter()
        .position(|x| x["name"] == column)
        .unwrap();
    let row = c.columns.iter().find(|r| r.index == idx).unwrap();
    app.model_view.screen_pos(row.rect.center())
}

fn nodes(app: &App) -> Vec<(String, f64, f64)> {
    model(app)["diagram"]["nodes"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|n| {
            (
                n["table"].as_str().unwrap().to_string(),
                n["x"].as_f64().unwrap(),
                n["y"].as_f64().unwrap(),
            )
        })
        .collect()
}

#[test]
fn dragging_a_card_moves_it_and_saves_the_layout() {
    let dir = Temp::new("model-move");
    let path = star(dir.path());
    let mut h = open_model(&path, "en-US", (1400.0, 900.0));
    let before = card(h.state(), "Notes").rect;
    let others: Vec<(String, Pos2)> = ["Sales", "Product", "Category"]
        .iter()
        .map(|t| {
            (
                t.to_string(),
                h.state().model_view.screen_pos(card(h.state(), t).rect.min),
            )
        })
        .collect();
    let rev = revision(h.state());
    let from = h
        .state()
        .model_view
        .screen_pos(card(h.state(), "Notes").head.center());
    let zoom = h.state().model_view.zoom;
    drag(&mut h, from, from + Vec2::new(-60.0, 200.0));
    assert!(!h.state().model_view.dragging());
    wait(&mut h, "the move", |a| revision(a) > rev && !nodes(a).is_empty());
    let app = h.state();
    // one diagram.move with every card: the moved one where it was dropped, the others kept
    let n = nodes(app);
    assert_eq!(n.len(), 6, "{n:?}");
    let shift = app.model_view.diagram().unwrap().shift;
    let after = card(app, "Notes").rect;
    let moved = (after.min - before.min) * zoom;
    assert!((moved - Vec2::new(-60.0, 200.0)).length() < 3.0, "{moved:?}");
    let notes = n.iter().find(|x| x.0 == "Notes").unwrap();
    assert!((notes.1 as f32 + shift.x - after.min.x).abs() < 0.2);
    for (t, screen) in others {
        let now = card(app, &t).rect;
        let saved = n.iter().find(|x| x.0 == t).unwrap();
        assert!((saved.1 as f32 + shift.x - now.min.x).abs() < 0.2, "{t}");
        // the cards that did not move stay where they were on the screen
        let moved = app.model_view.screen_pos(now.min) - screen;
        assert!(moved.length() < 1.5, "{t} moved by {moved:?}");
    }
    // the title shows the unsaved edit; Save writes diagramLayout.json
    wait(&mut h, "the unsaved dot", |a| a.window_title().starts_with("● "));
    h.state_mut().save();
    wait(&mut h, "the save", |a| !a.window_title().starts_with("● "));
    let text = std::fs::read_to_string(
        path.with_file_name("star.SemanticModel")
            .join("diagramLayout.json"),
    )
    .unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["diagrams"][0]["nodes"].as_array().unwrap().len(), 6);
    // Ctrl+Z puts the card back
    let rev = revision(h.state());
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Z);
    wait(&mut h, "the undo", |a| revision(a) > rev);
    assert!(nodes(h.state()).is_empty());
}

#[test]
fn a_column_dropped_on_another_tables_column_relates_them() {
    let dir = Temp::new("model-relate");
    let path = star(dir.path());
    let mut h = open_model(&path, "en-US", (1400.0, 900.0));
    let count = |a: &App| model(a)["relationships"].as_array().unwrap().len();
    let n = count(h.state());
    let from = column_row(h.state(), "Notes", "Text");
    let to = column_row(h.state(), "Category", "Label");
    drag(&mut h, from, to);
    wait(&mut h, "the relationship", |a| count(a) == n + 1);
    let r = model(h.state())["relationships"][n].clone();
    assert_eq!(r["from_table"], "Notes");
    assert_eq!(r["from_column"], "Text");
    assert_eq!(r["to_table"], "Category");
    assert_eq!(r["to_column"], "Label");
    // a card did not move, and nothing was saved in the diagram
    assert!(nodes(h.state()).is_empty());
}

#[test]
fn double_click_edits_a_relationship_and_delete_removes_it() {
    let dir = Temp::new("model-rel-edit");
    let path = star(dir.path());
    let mut h = open_model(&path, "en-US", (1400.0, 900.0));
    let (mid, rel) = {
        let d = h.state().model_view.diagram().unwrap();
        let l = d
            .links
            .iter()
            .find(|l| model(h.state())["relationships"][l.rel]["to_table"] == "Region")
            .unwrap();
        (diagram::cubic(&l.curve, 0.5), l.rel)
    };
    let p = h.state().model_view.screen_pos(mid);
    // a harness step is 0.25 s: a person's quick double click
    h.ctx
        .options_mut(|o| o.input_options.max_double_click_delay = 2.0);
    h.hover_at(p);
    h.step();
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(Event::PointerButton {
                pos: p,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        h.step();
    }
    h.step();
    // the Edit relationship window opens on the relationship, with its settings
    let ed = h
        .state()
        .cmd
        .rel_edit
        .clone()
        .expect("the Edit relationship window");
    let old = ed.old.as_ref().expect("an existing relationship");
    assert_eq!(old.to_table, "Region");
    assert_eq!(ed.sides[1].table, "Region");
    assert!(ed.both, "cross filter in both directions");
    assert!(ed.active);
    assert_eq!(ed.card, bicraft_ui::relationships_dialog::Card::ManyToOne);
    assert!(h.state().form.is_none(), "no generic form");
    h.state_mut().cmd.rel_edit = None;
    h.step();

    // Delete removes the selected relationship
    assert_eq!(h.state().model_view.sel, Some(Sel::Rel(rel)));
    let n = model(h.state())["relationships"].as_array().unwrap().len();
    h.key_press(egui::Key::Delete);
    wait(&mut h, "the removal", |a| {
        model(a)["relationships"].as_array().unwrap().len() == n - 1
    });
    assert!(
        model(h.state())["relationships"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["to_table"] != "Region")
    );
}

#[test]
fn the_properties_pane_sets_a_columns_and_a_measures_properties() {
    let dir = Temp::new("model-props");
    let path = star(dir.path());
    let mut h = open_model(&path, "en-US", (1400.0, 900.0));
    // a column: its description, typed then Enter
    let p = column_row(h.state(), "Sales", "Amount");
    click(&mut h, p, false);
    h.step();
    h.get_by_label("Properties");
    let col = |a: &App, k: &str| {
        let m = model(a);
        let t = m["tables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "Sales")
            .unwrap()
            .clone();
        t["columns"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "Amount")
            .unwrap()[k]
            .clone()
    };
    h.get_by_label("Description").click();
    h.step();
    h.get_by_label("Description").type_text("Net amount");
    h.step();
    h.key_press(egui::Key::Enter);
    wait(&mut h, "the description", |a| {
        col(a, "description") == "Net amount"
    });
    // Is hidden: a box
    h.get_by_label("Is hidden").click();
    wait(&mut h, "hidden", |a| col(a, "hidden") == true);
    // Summarize by: a list
    let r = h
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Summarize by")
        .rect();
    click(&mut h, r.center(), false);
    let r = h.get_by_label("average").rect();
    click(&mut h, r.center(), false);
    wait(&mut h, "summarize by", |a| col(a, "summarize_by") == "average");

    // a measure: its format
    let (k, row) = {
        let d = h.state().model_view.diagram().unwrap();
        let k = d.card_named("Sales").unwrap();
        let m = model(h.state());
        let idx = m["tables"][d.cards[k].table]["measures"]
            .as_array()
            .unwrap()
            .iter()
            .position(|x| x["name"] == "Average")
            .unwrap();
        (
            k,
            d.cards[k].measures.iter().find(|r| r.index == idx).unwrap().rect,
        )
    };
    let _ = k;
    let p = h.state().model_view.screen_pos(row.center());
    click(&mut h, p, false);
    h.step();
    let fmt = |a: &App| {
        let m = model(a);
        let t = m["tables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "Sales")
            .unwrap()
            .clone();
        t["measures"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["name"] == "Average")
            .unwrap()["format_string"]
            .clone()
    };
    h.get_by_label("Format").click();
    h.step();
    h.get_by_label("Format").type_text("0.00");
    h.step();
    h.key_press(egui::Key::Enter);
    wait(&mut h, "the format", |a| fmt(a) == "0.00");
}

#[test]
fn the_properties_pane_sets_a_tables_and_a_relationships_properties() {
    let dir = Temp::new("model-props-rel");
    let path = star(dir.path());
    let mut h = open_model(&path, "fr-FR", (1400.0, 900.0));
    let head = h
        .state()
        .model_view
        .screen_pos(card(h.state(), "Notes").head.center());
    click(&mut h, head, false);
    h.step();
    h.get_by_label("Propriétés");
    let hidden = |a: &App| {
        model(a)["tables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "Notes")
            .unwrap()["hidden"]
            .clone()
    };
    h.get_by_label("Masquée").click();
    wait(&mut h, "the hidden table", |a| hidden(a) == true);

    // the inactive Sales → Date relationship: make it active
    let (mid, rel) = {
        let d = h.state().model_view.diagram().unwrap();
        let l = d.links.iter().find(|l| l.inactive).unwrap();
        (diagram::cubic(&l.curve, 0.5), l.rel)
    };
    let p = h.state().model_view.screen_pos(mid);
    click(&mut h, p, false);
    h.step();
    assert_eq!(h.state().model_view.sel, Some(Sel::Rel(rel)));
    h.get_by_label("Relation active").click();
    wait(&mut h, "the active relationship", |a| {
        model(a)["relationships"][rel]["active"] == true
    });
}
