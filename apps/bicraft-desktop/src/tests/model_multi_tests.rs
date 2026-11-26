//! The Properties pane on several objects (egui_kittest, no GPU; docs/gaps.md G61): Ctrl+click in
//! the diagram and Ctrl or Shift+click in the Data pane select several tables, columns or
//! measures; the pane shows the properties they share, a value they differ on as mixed, and one
//! change sets it on all of them as one undo step (`model.set_properties`); Synonyms of one object.

use egui::accesskit::Toggled;
use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::Value;

use super::model_explorer_tests::pane_rect;
use super::model_tests::{open_model, star};
use super::{Temp, click, wait};
use crate::App;
use bicraft_ui::diagram;
use bicraft_ui::model_view::Sel;

fn model(app: &App) -> Value {
    app.loaded.as_ref().unwrap().snap["model"].clone()
}

fn object(app: &App, table: &str, kind: &str, name: &str) -> Value {
    let m = model(app);
    let t = m["tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == table)
        .unwrap()
        .clone();
    if kind == "table" {
        return t;
    }
    t[kind]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == name)
        .unwrap()
        .clone()
}

fn card(app: &App, table: &str) -> diagram::Card {
    let d = app.model_view.diagram().unwrap();
    d.cards[d.card_named(table).unwrap()].clone()
}

/// The screen centre of row `name` (`kind`: columns or measures) of `table`'s card.
pub(super) fn row(app: &App, table: &str, kind: &str, name: &str) -> Pos2 {
    let m = model(app);
    let c = card(app, table);
    let idx = m["tables"][c.table][kind]
        .as_array()
        .unwrap()
        .iter()
        .position(|x| x["name"] == name)
        .unwrap();
    let rows = if kind == "columns" {
        &c.columns
    } else {
        &c.measures
    };
    let r = rows.iter().find(|r| r.index == idx).unwrap();
    app.model_view.screen_pos(r.rect.center())
}

/// A click with `modifiers` held.
fn click_with(h: &mut Harness<'_, App>, pos: Pos2, modifiers: Modifiers) {
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event_modifiers(
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers,
            },
            modifiers,
        );
        h.step();
    }
}

fn col(table: &str, name: &str) -> Sel {
    Sel::Column {
        table: table.into(),
        name: name.into(),
    }
}

fn revision(app: &App) -> u64 {
    app.loaded
        .as_ref()
        .map_or(0, |l| l.snap["revision"].as_u64().unwrap_or(0))
}

#[test]
fn ctrl_click_in_the_diagram_selects_several_and_one_change_sets_all() {
    let dir = Temp::new("model-multi-diagram");
    let path = star(dir.path());
    let mut h = open_model(&path, "en-US", (1400.0, 900.0));
    let p = row(h.state(), "Sales", "columns", "Amount");
    click(&mut h, p, false);
    let p = row(h.state(), "Sales", "columns", "Cost");
    click(&mut h, p, true);
    h.step();
    assert_eq!(
        h.state().model_view.selection(),
        vec![col("Sales", "Amount"), col("Sales", "Cost")]
    );
    h.get_by_label("2 objects selected");
    // Amount is shown, Cost hidden: the box is half ticked
    assert_eq!(
        h.get_by_label("Is hidden").accesskit_node().toggled(),
        Some(Toggled::Mixed)
    );
    // one click hides both, in one undo step
    let rev = revision(h.state());
    h.get_by_label("Is hidden").click();
    wait(&mut h, "both hidden", |a| {
        object(a, "Sales", "columns", "Amount")["hidden"] == true
            && object(a, "Sales", "columns", "Cost")["hidden"] == true
    });
    assert!(revision(h.state()) > rev);
    assert_eq!(
        h.get_by_label("Is hidden").accesskit_node().toggled(),
        Some(Toggled::True)
    );
    let rev = revision(h.state());
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Z);
    wait(&mut h, "the undo", |a| revision(a) > rev);
    assert_eq!(object(h.state(), "Sales", "columns", "Amount")["hidden"], false);
    assert_eq!(object(h.state(), "Sales", "columns", "Cost")["hidden"], true);
    // a Ctrl+click on a selected row takes it out: one object, its own pane
    let p = row(h.state(), "Sales", "columns", "Cost");
    click(&mut h, p, true);
    h.step();
    assert_eq!(h.state().model_view.selection(), vec![col("Sales", "Amount")]);
    assert!(h.query_by_label("2 objects selected").is_none());
    h.get_by_label("Synonyms");
    // a plain click selects one object again
    let p = row(h.state(), "Sales", "columns", "Cost");
    click(&mut h, p, true);
    let p = row(h.state(), "Sales", "measures", "Total");
    click(&mut h, p, false);
    h.step();
    assert_eq!(
        h.state().model_view.selection(),
        vec![Sel::Measure {
            table: "Sales".into(),
            name: "Total".into()
        }]
    );
}

#[test]
fn the_data_pane_selects_with_ctrl_and_shift_and_a_format_sets_all() {
    let dir = Temp::new("model-multi-pane");
    let path = star(dir.path());
    let mut h = open_model(&path, "en-US", (1400.0, 900.0));
    // open Sales in the Data pane, then Amount, Shift+Margin, Ctrl+Total
    let r = pane_rect(&h, "Sales");
    click(&mut h, r.center(), false);
    h.step();
    let r = pane_rect(&h, "Amount");
    click(&mut h, r.center(), false);
    h.step();
    let r = pane_rect(&h, "Margin");
    click_with(&mut h, r.center(), Modifiers::SHIFT);
    h.step();
    assert_eq!(
        h.state().model_view.selection(),
        vec![
            col("Sales", "Amount"),
            col("Sales", "Cost"),
            col("Sales", "Margin")
        ]
    );
    h.get_by_label("3 objects selected");
    let r = pane_rect(&h, "Total");
    click_with(&mut h, r.center(), Modifiers::COMMAND);
    h.step();
    assert_eq!(h.state().model_view.selection().len(), 4);
    h.get_by_label("4 objects selected");
    // columns and a measure share no Summarize by
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "Summarize by")
            .is_none()
    );
    // the measure has a format, the columns none: the field is blank, "Multiple values"
    let field = h.get_by_label("Format");
    assert_eq!(field.accesskit_node().placeholder(), Some("Multiple values"));
    assert_eq!(field.accesskit_node().value().as_deref(), Some(""));
    h.get_by_label("Format").click();
    h.step();
    h.get_by_label("Format").type_text("0.0");
    h.step();
    h.key_press(egui::Key::Enter);
    wait(&mut h, "the formats", |a| {
        ["Amount", "Cost", "Margin"]
            .iter()
            .all(|c| object(a, "Sales", "columns", c)["format_string"] == "0.0")
            && object(a, "Sales", "measures", "Total")["format_string"] == "0.0"
    });
    assert_eq!(
        h.get_by_label("Format").accesskit_node().value().as_deref(),
        Some("0.0")
    );
    // one Ctrl+Z puts the four back
    let rev = revision(h.state());
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Z);
    wait(&mut h, "the undo", |a| revision(a) > rev);
    let app = h.state();
    assert!(
        ["Amount", "Cost", "Margin"]
            .iter()
            .all(|c| object(app, "Sales", "columns", c)["format_string"] != "0.0")
    );
    assert_eq!(object(app, "Sales", "measures", "Total")["format_string"], "#,0");
}

#[test]
fn tables_in_french_and_the_synonyms_of_one() {
    let dir = Temp::new("model-multi-fr");
    let path = star(dir.path());
    let mut h = open_model(&path, "fr-FR", (1400.0, 900.0));
    for (t, ctrl) in [("Notes", false), ("Region", true)] {
        let p = h.state().model_view.screen_pos(card(h.state(), t).head.center());
        click(&mut h, p, ctrl);
    }
    h.step();
    h.get_by_label("2 objets sélectionnés");
    assert_eq!(
        h.get_by_label("Masquée").accesskit_node().toggled(),
        Some(Toggled::False)
    );
    // no Format nor Display folder for tables
    assert!(h.query_by_label("Format").is_none());
    h.get_by_label("Masquée").click();
    wait(&mut h, "both tables hidden", |a| {
        object(a, "Notes", "table", "")["hidden"] == true
            && object(a, "Region", "table", "")["hidden"] == true
    });
    // one table: its synonyms, comma separated
    let p = h
        .state()
        .model_view
        .screen_pos(card(h.state(), "Notes").head.center());
    click(&mut h, p, false);
    h.step();
    h.get_by_label("Synonymes").click();
    h.step();
    h.get_by_label("Synonymes").type_text("memo, remarks");
    h.step();
    h.key_press(egui::Key::Enter);
    wait(&mut h, "the synonyms", |a| {
        object(a, "Notes", "table", "")["synonyms"] == serde_json::json!(["memo", "remarks"])
    });
}
