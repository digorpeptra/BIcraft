//! Command answers (`answer.rs`), the pickers of every object kind, the forms' defaults, the
//! title row's search button and the browser's keyboard, through the accessibility tree
//! (egui_kittest, no GPU). Sources are engine sessions run in the test.

use std::path::Path;
use std::time::{Duration, Instant};

use bicraft_engine::Session;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::{frame, harness, shell_app, shop_snapshot};
use crate::App;
use crate::app::Tab;
use crate::form;
use crate::registry::{self, Kind, Obj};
use crate::shell::Caps;

/// A model with every kind of object a form picks: tables with a hierarchy and a relationship,
/// a calculation group with an item, a role, a culture, a shared query, a report with a page, a
/// card and a report measure.
fn rich() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Amount"}, {{"Wheels", 100}, {"Toys", 5}})"#;
    let cats = r#"#table({"Category"}, {{"Wheels"}, {"Toys"}})"#;
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
            "table.add",
            json!({"name": "Product", "partition": {"source_type": "m", "expression": cats}}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Category", "data_type": "string"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[Category]", "to": "Product[Category]"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "hierarchy.add",
            json!({"table": "Product", "name": "Products", "levels": ["Category"]}),
        ),
        ("calculation_group.add", json!({"name": "Time"})),
        (
            "calculation_item.add",
            json!({"group": "Time", "name": "Current", "expression": "SELECTEDMEASURE()"}),
        ),
        ("role.add", json!({"name": "Readers"})),
        ("culture.add", json!({"name": "de-DE"})),
        (
            "expression.add",
            json!({"name": "Root", "expression": "\"C:\\\\data\""}),
        ),
        ("report.new", json!({})),
        (
            "report_measure.add",
            json!({"table": "Sales", "name": "Twice", "expression": "[Total] * 2"}),
        ),
    ] {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute(
        "visual.add",
        &json!({"page": page, "visual_type": "card", "x": 10, "y": 10, "width": 200, "height": 100,
                "fields": [{"role": "Values", "field": "[Total]"}]}),
    )
    .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn app(caps: Caps) -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(caps, rich));
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1280.0, 800.0));
    steps(&mut h, 6);
    assert!(h.state().loaded.is_some(), "the snapshot never came");
    h
}

/// `n` records `{name, rows, size}`.
fn records(n: usize) -> Value {
    Value::Array(
        (0..n)
            .map(|i| json!({"name": format!("row {i:06}"), "rows": (i * 7919) % 1000, "size": i}))
            .collect(),
    )
}

/// Shows `v` as the answer of `id` in the ribbon's answer window.
fn show_answer(h: &mut Harness<'_, App>, id: &'static str, v: Value) {
    h.state_mut().cmd.view = None;
    h.state_mut().cmd.result = Some((id, Some(Ok(v))));
    steps(h, 3);
}

fn copied(h: &Harness<'_, App>) -> Option<String> {
    h.output().platform_output.commands.iter().find_map(|c| match c {
        egui::OutputCommand::CopyText(s) => Some(s.clone()),
        _ => None,
    })
}

/// A list of records is a grid: a click on a header sorts (ascending, descending, back),
/// Copy as TSV copies the rows in that order, JSON shows the raw answer.
#[test]
fn a_grid_answer_sorts_and_copies() {
    let mut h = app(Caps::DESKTOP);
    show_answer(&mut h, "model.summary", json!({"tables": records(5), "count": 5}));
    h.get_by_label_contains("tables · 5 rows");
    h.get_by_label("row 000000");
    h.get_by_label("size").click();
    steps(&mut h, 3);
    let grid = |h: &Harness<'_, App>| h.state().cmd.view.as_ref().unwrap().grids[0].clone();
    assert_eq!(grid(&h).sort, Some((2, true)));
    h.get_by_label("size").click();
    steps(&mut h, 3);
    assert_eq!(grid(&h).sort, Some((2, false)));
    assert_eq!(grid(&h).cell(0, 0), Some("row 000004"));
    h.get_by_label("Copy as TSV").click();
    steps(&mut h, 1);
    let tsv = copied(&h).expect("the grid was copied");
    assert!(tsv.starts_with("name\trows\tsize\nrow 000004\t"), "{tsv}");
    assert_eq!(tsv.lines().count(), 6);
    // a third click: the answer's own order
    h.get_by_label("size").click();
    steps(&mut h, 3);
    assert_eq!(grid(&h).sort, None);
    // the JSON toggle and its copy
    h.get_by_label("JSON").click();
    steps(&mut h, 3);
    assert!(h.state().cmd.view.as_ref().unwrap().json);
    assert!(
        h.query_by_label("row 000000").is_none(),
        "the JSON text replaces the grid"
    );
    h.get_by_label("Copy JSON").click();
    steps(&mut h, 1);
    let text = copied(&h).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["count"], 5);
    // Escape closes the window
    h.key_press(Key::Escape);
    steps(&mut h, 2);
    assert!(h.state().cmd.result.is_none() && h.state().cmd.view.is_none());
}

/// An object is a key/value tree; a nested list of records is a grid inside it.
#[test]
fn an_object_answer_is_a_key_value_tree() {
    let mut h = app(Caps::DESKTOP);
    show_answer(
        &mut h,
        "model.get",
        json!({"name": "Shop", "culture": "en-US", "valid": true, "nested": {"depth": 2}, "items": records(2)}),
    );
    for k in ["culture", "en-US", "valid", "nested", "depth", "items · 2 rows"] {
        h.get_by_label(k);
    }
    assert!(h.query_all_by_label("Shop").count() >= 1);
    assert!(h.query_all_by_label("true").count() >= 1);
    h.get_by_label("row 000001");
}

/// The form's inline answer uses the same grid: `dax.evaluate`'s rows in its columns' order.
#[test]
fn the_forms_answer_is_a_grid_too() {
    let mut h = app(Caps::DESKTOP);
    form::open(
        h.state_mut(),
        "dax.evaluate",
        json!({"query": "EVALUATE SUMMARIZECOLUMNS(Sales[Category], \"Sum\", SUM(Sales[Amount]))"}),
    );
    steps(&mut h, 2);
    form::run(h.state_mut());
    steps(&mut h, 6);
    let f = h.state().form.as_ref().unwrap();
    assert!(matches!(f.result, Some(Ok(_))), "{:?}", f.result);
    let g = &f.view.as_ref().unwrap().grids[0];
    assert_eq!(g.columns, ["Sales[Category]", "[Sum]"]);
    assert_eq!(g.rows.len(), 2);
    h.get_by_label("Wheels");
    h.get_by_label("[Sum]");
}

/// 100 000 rows: only the visible ones are laid out, so a frame costs about what a frame of 100
/// rows costs, and scrolling keeps it so.
#[test]
fn a_hundred_thousand_rows_scroll_without_slowing_down() {
    let mut h = app(Caps::DESKTOP);
    let time = |h: &mut Harness<'_, App>| {
        steps(h, 3);
        let t = Instant::now();
        steps(h, 10);
        t.elapsed()
    };
    show_answer(&mut h, "model.storage_stats", records(100));
    let small = time(&mut h);
    let t = Instant::now();
    show_answer(&mut h, "model.storage_stats", records(100_000));
    let build = t.elapsed();
    let big = time(&mut h);
    let a = h.state().cmd.view.as_ref().unwrap();
    assert_eq!(a.grids[0].rows.len(), 100_000);
    let drawn = h.query_all_by_label_contains("row 0").count();
    assert!(drawn > 5 && drawn < 80, "rows laid out: {drawn}");
    // scroll to the end
    h.get_by_label("row 000002").hover();
    h.step();
    for _ in 0..30 {
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Page,
            delta: egui::vec2(0.0, -2000.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    let end = time(&mut h);
    let last = h.query_all_by_label_contains("row 0").count();
    assert!(last < 80, "rows laid out at the end: {last}");
    let slack = Duration::from_millis(40) * 10;
    assert!(
        big < small * 4 + slack && end < small * 4 + slack,
        "10 frames: 100 rows {small:?}, 100 000 rows {big:?}, at the end {end:?} (built in {build:?})"
    );
    // a sort of 100 000 rows is one click
    let t = Instant::now();
    h.get_by_label("name").click();
    steps(&mut h, 2);
    assert_eq!(
        h.state().cmd.view.as_ref().unwrap().grids[0].sort,
        Some((0, true))
    );
    assert!(
        t.elapsed() < Duration::from_secs(10),
        "sorted in {:?}",
        t.elapsed()
    );
}

/// The desktop and the browser draw the same frames for a grid answer.
#[test]
fn a_grid_answer_draws_the_same_frames_in_both_shells() {
    let snap = shop_snapshot();
    let mut desktop = shell_app(Caps::DESKTOP, &snap, "en-US");
    let mut browser = shell_app(Caps::BROWSER, &snap, "en-US");
    for h in [&mut desktop, &mut browser] {
        h.state_mut().cmd.result = Some((
            "model.summary",
            Some(Ok(json!({"tables": records(50), "count": 50}))),
        ));
        steps(h, 4);
    }
    let (a, b) = (frame(&desktop), frame(&browser));
    assert!(a.0 == b.0, "the meshes differ");
    assert!(a.1 == b.1, "the font atlases differ");
    // and the form's
    for h in [&mut desktop, &mut browser] {
        h.state_mut().cmd.result = None;
        form::open(h.state_mut(), "model.summary", json!({}));
        let f = h.state_mut().form.as_mut().unwrap();
        f.result = Some(Ok(json!({"tables": records(20)})));
        steps(h, 4);
    }
    let (a, b) = (frame(&desktop), frame(&browser));
    assert!(a.0 == b.0 && a.1 == b.1, "the forms' answers differ");
}

/// Every name parameter of every command offers the sample's objects: tables, columns,
/// measures, hierarchies, partitions, roles, cultures, calculation groups and items, shared
/// queries, report measures, relationships, pages, visuals, data roles, visual types, and the
/// environment profiles `env.list` returns through the document's source.
#[test]
fn every_picker_is_filled_from_a_sample_session() {
    let mut h = app(Caps::DESKTOP);
    let env = json!({"profiles": {"dev": {}, "prod": {"description": "production"}}});
    let mut seen: Vec<Obj> = Vec::new();
    for c in registry::COMMANDS {
        let objs: Vec<&registry::Param> = c
            .form
            .iter()
            .filter(|p| matches!(p.kind, Kind::Name(_)))
            .collect();
        if objs.is_empty() {
            continue;
        }
        let prefill = if objs.iter().any(|p| p.kind == Kind::Name(Obj::Profile)) {
            json!({"env_file": env})
        } else {
            json!({})
        };
        form::open(h.state_mut(), c.id, prefill);
        steps(&mut h, 4);
        let app = h.state();
        let f = app.form.as_ref().unwrap();
        for p in objs {
            let Kind::Name(o) = p.kind else { continue };
            let opts = form::options(app, f, p);
            assert!(!opts.is_empty(), "{}.{}: no {o:?} to pick", c.id, p.name);
            let want = match o {
                Obj::Hierarchy => Some("Products"),
                Obj::Partition => Some("Sales"),
                Obj::CalculationGroup => Some("Time"),
                Obj::CalculationItem => Some("Current"),
                Obj::Role => Some("Readers"),
                Obj::Culture => Some("de-DE"),
                Obj::Expression => Some("Root"),
                Obj::ReportMeasure => Some("Twice"),
                Obj::Profile => Some("prod"),
                Obj::Measure => Some("Total"),
                _ => None,
            };
            if let Some(w) = want {
                assert!(opts.iter().any(|(v, _)| v == w), "{}.{}: {opts:?}", c.id, p.name);
            }
            if !seen.contains(&o) {
                seen.push(o);
            }
        }
        h.state_mut().form = None;
        steps(&mut h, 1);
    }
    for o in [
        Obj::Table,
        Obj::Column,
        Obj::Measure,
        Obj::Hierarchy,
        Obj::Partition,
        Obj::Expression,
        Obj::Relationship,
        Obj::Role,
        Obj::Culture,
        Obj::CalculationGroup,
        Obj::CalculationItem,
        Obj::Page,
        Obj::Visual,
        Obj::ReportMeasure,
        Obj::DataRole,
        Obj::VisualType,
        Obj::Profile,
    ] {
        assert!(seen.contains(&o), "no command picks {o:?}");
    }
    // a relationship is labelled by its ends
    form::open(h.state_mut(), "relationship.remove", json!({}));
    steps(&mut h, 2);
    let app = h.state();
    let f = app.form.as_ref().unwrap();
    let opts = form::options(app, f, f.inputs[0].field);
    assert_eq!(opts[0].1, "Sales[Category] → Product[Category]");
    // the picker under the field, then a pick fills it
    h.query_all_by_role(egui::accesskit::Role::ComboBox)
        .next()
        .expect("a picker")
        .click();
    steps(&mut h, 2);
    h.get_by_label("Sales[Category] → Product[Category]").click();
    steps(&mut h, 2);
    let name = &h.state().form.as_ref().unwrap().inputs[0].text;
    assert!(!name.is_empty(), "the relationship's name was picked");
}

/// An optional boolean shows the command's default until it is touched (and is left out of the
/// parameters); Reset leaves it out again. One without a default (left out means unchanged)
/// says so.
#[test]
fn optional_booleans_show_their_default() {
    let mut h = app(Caps::DESKTOP);
    form::open(h.state_mut(), "model.unused", json!({}));
    steps(&mut h, 3);
    let toggled = |h: &Harness<'_, App>, name: &str| h.get_by_label(name).accesskit_node().toggled();
    assert_eq!(
        toggled(&h, "kept"),
        Some(egui::accesskit::Toggled::True),
        "default true"
    );
    assert_eq!(toggled(&h, "keep metadata"), Some(egui::accesskit::Toggled::True));
    assert_eq!(h.query_all_by_label("(default)").count(), 2);
    let f = h.state().form.as_ref().unwrap();
    assert_eq!(
        form::params(f, crate::i18n::Lang::En),
        Ok(json!({})),
        "defaults are left out"
    );
    // touched: sent; Reset: left out again
    h.get_by_label("kept").click();
    steps(&mut h, 2);
    let f = h.state().form.as_ref().unwrap();
    assert_eq!(form::params(f, crate::i18n::Lang::En), Ok(json!({"kept": false})));
    h.get_by_label("Reset").click();
    steps(&mut h, 2);
    let f = h.state().form.as_ref().unwrap();
    assert_eq!(form::params(f, crate::i18n::Lang::En), Ok(json!({})));
    assert_eq!(toggled(&h, "kept"), Some(egui::accesskit::Toggled::True));
    // a default false
    h.state_mut().form = None;
    form::open(h.state_mut(), "table.add", json!({}));
    steps(&mut h, 3);
    assert_eq!(toggled(&h, "hidden"), Some(egui::accesskit::Toggled::False));
    assert_eq!(h.query_all_by_label("(default)").count(), 2);
    // no fixed default: drawn mixed, "(not set)"
    h.state_mut().form = None;
    form::open(h.state_mut(), "table.set", json!({"name": "Sales"}));
    steps(&mut h, 3);
    assert_eq!(toggled(&h, "hidden"), Some(egui::accesskit::Toggled::Mixed));
    h.get_by_label("(not set)");
    // every optional boolean of the registry declares its default or none on purpose
    let declared = registry::COMMANDS
        .iter()
        .flat_map(|c| c.form.iter())
        .filter(|p| p.kind == Kind::Bool && !p.required && p.default.is_some())
        .count();
    assert!(declared > 90, "{declared}");
}

/// The title row's search button opens the command palette, and closes it.
#[test]
fn the_search_button_opens_the_palette() {
    let mut h = app(Caps::DESKTOP);
    h.state_mut().tab = Tab::Report;
    h.get_by_label("Search commands (Ctrl+Shift+P)").click();
    steps(&mut h, 3);
    assert!(h.state().palette.is_some());
    h.key_press(Key::Escape);
    steps(&mut h, 2);
    assert!(h.state().palette.is_none());
}

/// The model diagram's menus list the objects their commands act on: a table's hierarchies,
/// the background's queries, roles and cultures; a pick runs the command on that object.
#[test]
fn the_model_menus_pick_their_objects() {
    let mut h = app(Caps::DESKTOP);
    let snap = h.state().loaded.as_ref().unwrap().snap.clone();
    let model = &snap["model"];
    let product = crate::model_view::Sel::Table("Product".into());
    assert_eq!(
        crate::menus::menu_objects("hierarchy.remove", Some(&product), model),
        Some(vec!["Products".to_string()])
    );
    let time = crate::model_view::Sel::Table("Time".into());
    assert_eq!(
        crate::menus::menu_objects("calculation_item.remove", Some(&time), model),
        Some(vec!["Current".to_string()])
    );
    for (id, want) in [
        ("expression.rename", "Root"),
        ("expression.remove", "Root"),
        ("role.remove", "Readers"),
        ("culture.remove", "de-DE"),
    ] {
        let got = crate::menus::menu_objects(id, None, model).unwrap();
        assert!(got.iter().any(|n| n == want), "{id}: {got:?}");
    }
    assert_eq!(
        crate::menus::menu_objects("table.rename", Some(&product), model),
        None
    );
    // a pick from the background's submenu: role.remove {name} runs at once
    h.state_mut().tab = Tab::Model;
    steps(&mut h, 3);
    h.state_mut().model_view.picked = Some(("role.remove", json!({"name": "Readers"})));
    steps(&mut h, 4);
    let sent: Vec<&str> = h.state().sent.iter().map(|c| c.id).collect();
    assert_eq!(sent.last(), Some(&"role.remove"));
    let roles = h.state().loaded.as_ref().unwrap().snap["model"]["roles"].clone();
    assert_eq!(roles, json!([]));
    // a rename opens its form with the query picked
    h.state_mut().model_view.picked = Some(("expression.rename", json!({"name": "Root"})));
    steps(&mut h, 3);
    let f = h.state().form.as_ref().unwrap();
    assert_eq!(
        (f.id, f.input("name").unwrap().text.as_str()),
        ("expression.rename", "Root")
    );
}

/// The browser shows and takes its own chord for what it keeps (Ctrl+N is New on the desktop,
/// Alt+N in a browser); everything else is the same.
#[test]
fn the_browser_shows_its_own_chord_for_new() {
    let c = registry::find("model.new").unwrap();
    assert_eq!(c.shortcut_for(false), Some("Ctrl+N"));
    assert_eq!(c.shortcut_for(true), Some("Alt+N"));
    assert!(
        c.tooltip(crate::i18n::Lang::En, &Ok(()), true)
            .contains("(Alt+N)")
    );
    assert!(
        c.tooltip(crate::i18n::Lang::En, &Ok(()), false)
            .contains("(Ctrl+N)")
    );
    let f5 = registry::find("model.refresh").unwrap();
    assert_eq!(
        f5.shortcut_for(true),
        Some("F5"),
        "the browser shell keeps F5 for the app"
    );
    for (desktop, browser) in registry::BROWSER_CHORDS {
        assert!(crate::ribbon::chord(browser).is_some(), "{browser}");
        assert!(
            !registry::COMMANDS.iter().any(|c| c.shortcut == Some(*browser)),
            "{browser} (for {desktop}) is a desktop chord too"
        );
    }
    // the palette's row in each shell
    let mut b = app(Caps::BROWSER);
    let rows = crate::palette::rows(b.state(), "model.new");
    assert_eq!(rows[0].shortcut, Some("Alt+N"));
    let d = app(Caps::DESKTOP);
    assert_eq!(
        crate::palette::rows(d.state(), "model.new")[0].shortcut,
        Some("Ctrl+N")
    );
    // Alt+N runs New in the browser (a model is open: the form of model.new)
    b.key_press_modifiers(Modifiers::ALT, Key::N);
    steps(&mut b, 3);
    let opened = b.state().form.as_ref().map(|f| f.id);
    let sent: Vec<&str> = b.state().sent.iter().map(|c| c.id).collect();
    assert!(
        opened == Some("model.new") || sent.contains(&"model.new"),
        "{opened:?} {sent:?}"
    );
}
