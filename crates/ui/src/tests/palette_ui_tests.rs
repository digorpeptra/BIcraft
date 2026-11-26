//! The command palette and the parameters form (ROADMAP M11.2), through the accessibility tree
//! (egui_kittest, no GPU). The source is an engine session run in the test; its snapshots carry
//! `commands.disabled`, the session's own answer to what can run.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::sync::Arc;

use bicraft_engine::{Caller, Session};
use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::{frame, harness, shell_app, shop_snapshot};
use crate::App;
use crate::app::Tab;
use crate::form;
use crate::palette;
use crate::registry;
use crate::ribbon;
use crate::shell::{Caps, Shell};
use crate::source::{Driver, Link, Loaded, Request, Running, Update};

struct Engine {
    session: RefCell<Session>,
    link: Link,
    seq: Cell<u64>,
}

impl Engine {
    fn publish(&self) {
        let snap = self.session.borrow().snapshot();
        self.seq.set(self.seq.get() + 1);
        self.link.send(Update::Snapshot(Loaded {
            snap: Arc::new(snap),
            assets: Arc::new(Default::default()),
            seq: self.seq.get(),
        }));
    }
}

impl Driver for Engine {
    fn request(&self, r: Request) {
        if let Request::Command { tag, id, params } = r {
            let result = self.session.borrow_mut().execute(&id, &params);
            if result.is_ok() {
                self.publish();
            }
            self.link.send(Update::Ran { tag, result });
        }
    }
}

fn shell(caps: Caps, caller: Caller, make: impl Fn() -> Session + 'static) -> Shell {
    Shell::new(caps, move |_o, link| {
        if caller == Caller::Reader {
            link.send(Update::Connected {
                access: "reader".into(),
            });
        }
        let d = Engine {
            session: RefCell::new(make()),
            link,
            seq: Cell::new(0),
        };
        d.publish();
        Running::Driven(Box::new(d))
    })
}

/// A model with a table and a measure, and a report of one page with a card.
fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": "#table({\"Amount\"}, {{1}})"}}),
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
    s.execute("page.rename", &json!({"page": page, "display_name": "Overview"}))
        .unwrap();
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

/// A model without a report.
fn model_only() -> Session {
    let mut s = Session::new();
    s.execute("model.new", &json!({"name": "M"})).unwrap();
    s
}

fn app_with(lang: &str, caller: Caller, make: impl Fn() -> Session + 'static) -> Harness<'static, App> {
    let mut app = App::new(lang, shell(Caps::DESKTOP, caller, make));
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1280.0, 800.0));
    steps(&mut h, 6);
    h
}

fn steps(h: &mut Harness<'_, App>, n: usize) {
    for _ in 0..n {
        h.step();
    }
}

fn palette_key(h: &mut Harness<'_, App>) {
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    steps(h, 3);
}

fn type_text(h: &mut Harness<'_, App>, s: &str) {
    h.event(Event::Text(s.into()));
    steps(h, 3);
}

fn first_row(h: &Harness<'_, App>) -> palette::Row {
    let q = h.state().palette.as_ref().unwrap().query.clone();
    palette::rows(h.state(), &q).remove(0)
}

fn sent_ids(h: &Harness<'_, App>) -> Vec<&'static str> {
    h.state().sent.iter().map(|c| c.id).collect()
}

/// Ctrl+Shift+P opens the palette with every command; typing filters (label, id, place, in
/// English or French), the arrows move the selection, Enter runs it through its form, Escape
/// closes; the command run comes first next time.
#[test]
fn the_palette_searches_and_runs_with_the_keyboard() {
    let mut h = app_with("en-US", Caller::Editor, session);
    palette_key(&mut h);
    assert!(h.state().palette.is_some());
    assert_eq!(palette::rows(h.state(), "").len(), registry::COMMANDS.len());
    type_text(&mut h, "new page");
    assert_eq!(first_row(&h).id, "page.add");
    h.get_by_label_contains("Report › Pages · page.add");
    assert_eq!(first_row(&h).shortcut, Some("Ctrl+M"));
    // the arrows move the selection
    h.key_press(Key::ArrowDown);
    steps(&mut h, 2);
    assert_eq!(h.state().palette.as_ref().unwrap().sel, 1);
    h.key_press(Key::ArrowUp);
    steps(&mut h, 2);
    assert_eq!(h.state().palette.as_ref().unwrap().sel, 0);
    // Enter runs it as its button does: page.add gets the next page's name, no form
    h.key_press(Key::Enter);
    steps(&mut h, 5);
    assert!(h.state().palette.is_none());
    assert_eq!(sent_ids(&h).last(), Some(&"page.add"));
    assert!(h.state().form.is_none());
    assert_eq!(h.state().page_names().len(), 2);
    // a read command: its answer in the ribbon's answer window
    palette_key(&mut h);
    type_text(&mut h, "model.validate");
    h.key_press(Key::Enter);
    steps(&mut h, 5);
    let (id, answer) = h.state().cmd.result.clone().expect("the answer is shown");
    assert_eq!(id, "model.validate");
    assert!(matches!(answer, Some(Ok(_))), "{answer:?}");
    h.key_press(Key::Escape);
    steps(&mut h, 3);
    // Shift+Enter: the form, even for a command that needs no value
    palette_key(&mut h);
    type_text(&mut h, "model.validate");
    h.key_press_modifiers(Modifiers::SHIFT, Key::Enter);
    steps(&mut h, 3);
    assert_eq!(h.state().form.as_ref().map(|f| f.id), Some("model.validate"));
    h.key_press(Key::Escape);
    steps(&mut h, 3);
    assert!(h.state().form.is_none());
    // recent first
    palette_key(&mut h);
    let rows = palette::rows(h.state(), "");
    assert_eq!((rows[0].id, rows[0].recent), ("model.validate", true));
    assert_eq!((rows[1].id, rows[1].recent), ("page.add", true));
    h.key_press(Key::Escape);
    steps(&mut h, 2);
    assert!(h.state().palette.is_none());
    // New measure runs as its button does: the formula bar, `Measure = `
    palette_key(&mut h);
    type_text(&mut h, "measure.add");
    assert_eq!(first_row(&h).id, "measure.add");
    h.key_press(Key::Enter);
    steps(&mut h, 4);
    assert!(h.state().formula.target.as_ref().is_some_and(|t| t.new));
    assert!(h.state().form.is_none());
    h.state_mut().formula = Default::default();
    // Shift+Enter: a command with required values opens its form
    palette_key(&mut h);
    type_text(&mut h, "measure.add");
    h.key_press_modifiers(Modifiers::SHIFT, Key::Enter);
    steps(&mut h, 4);
    let f = h.state().form.as_ref().unwrap();
    assert_eq!((f.id, f.running), ("measure.add", None));
    assert!(!sent_ids(&h).contains(&"measure.add"), "nothing ran yet");
}

/// The French window finds commands by their French and English words alike.
#[test]
fn the_palette_speaks_french() {
    let mut h = app_with("fr-FR", Caller::Editor, session);
    palette_key(&mut h);
    type_text(&mut h, "nouvelle mesure");
    assert_eq!(first_row(&h).id, "measure.add");
    assert_eq!(first_row(&h).label, "Nouvelle mesure");
    h.get_by_label_contains("Accueil › Calculs · measure.add");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    type_text(&mut h, "relationship");
    assert!(first_row(&h).id.starts_with("relationship."));
}

/// Commands the session cannot run are greyed with the reason; Enter on one runs nothing and
/// keeps the palette open.
#[test]
fn disabled_commands_show_their_reason() {
    let mut h = app_with("en-US", Caller::Editor, model_only);
    palette_key(&mut h);
    type_text(&mut h, "page.add");
    let r = first_row(&h);
    assert_eq!(
        (r.id, r.state.clone()),
        ("page.add", Err("needs a report".into()))
    );
    assert!(h.get_all_by_label("Unavailable: needs a report").count() >= 1);
    let before = h.state().sent.len();
    h.key_press(Key::Enter);
    steps(&mut h, 3);
    assert!(h.state().palette.is_some());
    assert_eq!(h.state().sent.len(), before);
    // the engine's own answer: nothing to undo yet
    assert_eq!(
        ribbon::state(h.state(), "edit.undo"),
        Err("nothing to undo".into())
    );
    // in French
    h.state_mut().lang = crate::i18n::Lang::Fr;
    assert_eq!(
        ribbon::state(h.state(), "page.add"),
        Err("il faut un rapport".into())
    );
}

/// A reader sees the commands that change the session greyed, and their forms show the reason
/// instead of fields.
#[test]
fn a_reader_gets_no_form_for_what_changes_the_session() {
    let mut h = app_with("en-US", Caller::Reader, session);
    assert_eq!(h.state().access.as_deref(), Some("reader"));
    let why = ribbon::state(h.state(), "measure.add").unwrap_err();
    assert!(why.contains("read-only"), "{why}");
    assert_eq!(ribbon::state(h.state(), "model.summary"), Ok(()));
    form::open(h.state_mut(), "measure.add", json!({}));
    steps(&mut h, 3);
    h.get_by_label_contains("Unavailable: read-only");
    assert!(h.query_by_label("Run").is_none());
    assert!(h.query_by_label("name *").is_none());
    h.get_by_label("Close").click();
    steps(&mut h, 2);
    assert!(h.state().form.is_none());
}

/// Every command's form opens and draws without a panic, with every field labelled (required
/// ones marked), empty and filled with hostile values.
#[test]
fn every_commands_form_opens_and_draws() {
    let mut h = app_with("en-US", Caller::Editor, session);
    // as if the session let every command run, so each form shows its fields
    {
        let l = h.state_mut().loaded.as_mut().unwrap();
        let mut snap = (*l.snap).clone();
        snap["commands"] = json!({"disabled": {}});
        l.snap = Arc::new(snap);
    }
    for c in registry::COMMANDS {
        for prefill in [
            json!({}),
            json!(null),
            json!([1, 2]),
            Value::Object(
                c.form
                    .iter()
                    .map(|f| (f.name.to_string(), json!({"deep": [null, "\u{0}", 1e308]})))
                    .chain([("unknown_key".to_string(), json!("x".repeat(5000)))])
                    .collect(),
            ),
        ] {
            form::open(h.state_mut(), c.id, prefill);
            steps(&mut h, 2);
            let f = h.state().form.as_ref().unwrap();
            assert_eq!(f.id, c.id);
            for field in c.form {
                if field.kind == registry::Kind::Bool {
                    h.get_by_label(&form::field_label(field.name));
                    continue;
                }
                let star = if field.required { " *" } else { "" };
                h.get_by_label(&format!("{}{star}", form::field_label(field.name)));
            }
            if form::FREE_KEYS.contains(&c.id) {
                h.get_by_label("more (JSON)");
            }
        }
    }
    // an unknown id is an error, never a panic
    form::open(h.state_mut(), "no.such", json!({}));
    assert!(h.state().error.as_deref().unwrap_or("").contains("no.such"));
}

/// Runs through the engine: a new measure from its name, table and DAX; a page renamed with its
/// picker; a DAX check whose answer shows inline; the engine's error inline too.
#[test]
fn forms_run_commands_through_the_engine() {
    let mut h = app_with("en-US", Caller::Editor, session);
    form::open(h.state_mut(), "measure.add", json!({"table": "Sales"}));
    steps(&mut h, 3);
    h.get_by_label("name *").focus();
    steps(&mut h, 1);
    type_text(&mut h, "Double");
    {
        let f = h.state_mut().form.as_mut().unwrap();
        f.input_mut("expression").unwrap().text = "[Total] * 2".into();
    }
    h.get_by_label("Run").click();
    steps(&mut h, 5);
    let f = h.state().form.as_ref().unwrap();
    assert!(matches!(f.result, Some(Ok(_))), "{:?}", f.result);
    let c = h.state().sent.last().unwrap();
    assert_eq!(
        (c.id, c.params.clone()),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Double", "expression": "[Total] * 2"})
        )
    );
    let snap = h.state().loaded.as_ref().unwrap().snap.clone();
    let measures = snap["model"]["tables"][0]["measures"].as_array().unwrap().clone();
    assert!(measures.iter().any(|m| m["name"] == "Double"));
    // the engine's error, inline
    {
        let f = h.state_mut().form.as_mut().unwrap();
        f.input_mut("name").unwrap().text = "Broken".into();
        f.input_mut("expression").unwrap().text = "SUM(Sales[Nope])".into();
    }
    h.get_by_label("Run").click();
    steps(&mut h, 5);
    let f = h.state().form.as_ref().unwrap();
    assert!(
        matches!(&f.result, Some(Err(e)) if e.contains("Nope")),
        "{:?}",
        f.result
    );
    h.get_by_label_contains("Error:");
    // a page renamed: its picker lists the pages by their names
    let page = h.state().page_names()[0].0.clone();
    form::open(h.state_mut(), "page.rename", json!({}));
    steps(&mut h, 3);
    assert_eq!(
        h.state().form.as_ref().unwrap().input("page").unwrap().text,
        "",
        "an explicit form opens empty (the palette fills the active page)"
    );
    h.get_by_label("page *");
    {
        let app = h.state();
        let f = app.form.as_ref().unwrap();
        let field = f.input("page").unwrap().field;
        assert_eq!(
            form::options(app, f, field),
            vec![(page.clone(), "Overview".to_string())]
        );
    }
    {
        let f = h.state_mut().form.as_mut().unwrap();
        f.input_mut("page").unwrap().text = page.clone();
        f.input_mut("display_name").unwrap().text = "Summary".into();
    }
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    steps(&mut h, 5);
    assert_eq!(h.state().page_names()[0].1, "Summary");
    // a read command's answer shows inline
    form::open(
        h.state_mut(),
        "dax.validate",
        json!({"expression": "SUM(Sales[Amount])"}),
    );
    steps(&mut h, 2);
    h.get_by_label("Run").click();
    steps(&mut h, 5);
    let f = h.state().form.as_ref().unwrap();
    assert!(matches!(f.result, Some(Ok(Value::Object(_)))), "{:?}", f.result);
    h.get_by_label_contains("Done");
}

/// Run checks the values first: a required one left empty and a number typed wrong are shown,
/// and nothing is sent until they are fixed.
#[test]
fn forms_check_their_values_before_run() {
    let mut h = app_with("en-US", Caller::Editor, session);
    form::open(h.state_mut(), "measure.add", json!({"table": "Sales"}));
    steps(&mut h, 3);
    assert!(h.query_by_label("✕ required").is_none(), "not before Run");
    let before = h.state().sent.len();
    h.get_by_label("Run").click();
    steps(&mut h, 3);
    assert_eq!(h.state().sent.len(), before, "nothing sent");
    assert!(h.state().form.as_ref().unwrap().tried);
    assert!(
        h.get_all_by_label("✕ required").count() >= 2,
        "name and expression"
    );
    h.get_by_label("✕ Some values need fixing");
    // a wrong number shows at once
    form::open(h.state_mut(), "edit.undo", json!({"steps": "two"}));
    steps(&mut h, 3);
    h.get_by_label("✕ a whole number");
    h.get_by_label("Run").click();
    steps(&mut h, 3);
    assert_eq!(h.state().sent.len(), before);
    // a JSON field that is not an object
    form::open(h.state_mut(), "visual.format", json!({}));
    {
        let f = h.state_mut().form.as_mut().unwrap();
        f.extra = Some("[1".into());
    }
    steps(&mut h, 3);
    h.get_by_label("✕ a JSON object");
}

/// The commands' shortcuts: Ctrl+M a new page, Ctrl+Z / Ctrl+Y undo and redo it, Ctrl+N the new
/// model form, Ctrl+Shift+S Save As, F5 a reload without a project (model.refresh needs one);
/// none with a form or the palette open.
#[test]
fn shortcuts_run_their_commands() {
    let mut h = app_with("en-US", Caller::Editor, session);
    h.state_mut().show_tab(Tab::Report);
    steps(&mut h, 2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::M);
    steps(&mut h, 5);
    assert_eq!(sent_ids(&h).last(), Some(&"page.add"));
    assert_eq!(h.state().page_names().len(), 2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    steps(&mut h, 5);
    assert_eq!(sent_ids(&h).last(), Some(&"edit.undo"));
    assert_eq!(h.state().page_names().len(), 1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    steps(&mut h, 5);
    assert_eq!(sent_ids(&h).last(), Some(&"edit.redo"));
    assert_eq!(h.state().page_names().len(), 2);
    // nothing to redo now: Ctrl+Y sends nothing
    let n = h.state().sent.len();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    steps(&mut h, 3);
    assert_eq!(h.state().sent.len(), n);
    // Ctrl+N: the new model's form; while it is open, Ctrl+M does nothing
    h.key_press_modifiers(Modifiers::COMMAND, Key::N);
    steps(&mut h, 3);
    assert_eq!(h.state().form.as_ref().map(|f| f.id), Some("model.new"));
    h.key_press_modifiers(Modifiers::COMMAND, Key::M);
    steps(&mut h, 3);
    assert_eq!(h.state().page_names().len(), 2);
    h.key_press(Key::Escape);
    steps(&mut h, 2);
    // and none with the palette open
    palette_key(&mut h);
    h.key_press_modifiers(Modifiers::COMMAND, Key::M);
    steps(&mut h, 3);
    assert_eq!(h.state().page_names().len(), 2);
    h.key_press(Key::Escape);
    steps(&mut h, 2);
    h.key_press(Key::F5);
    steps(&mut h, 3);
    assert!(!sent_ids(&h).contains(&"model.refresh"), "no project: a reload");
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::S);
    steps(&mut h, 3);
    assert_eq!(
        h.state().backstage.as_ref().map(|b| b.task),
        Some(crate::backstage::Task::SaveAs)
    );
}

/// The desktop's and the browser's frames are the same with the palette open and with a form
/// open, light and dark, in English and French.
#[test]
fn the_palette_and_a_form_draw_the_same_frames_in_both_shells() {
    let snap = shop_snapshot();
    for (lang, dark) in [("en-US", false), ("fr-FR", false), ("en-US", true)] {
        let mut desktop = shell_app(Caps::DESKTOP, &snap, lang);
        let mut browser = shell_app(Caps::BROWSER, &snap, lang);
        for step in 0..3 {
            for h in [&mut desktop, &mut browser] {
                if dark {
                    h.state_mut().set_theme(crate::theme::Brightness::Dark);
                }
                match step {
                    0 => {
                        palette::toggle(h.state_mut());
                        h.state_mut().palette.as_mut().unwrap().query = "measure".into();
                    }
                    1 => form::open(
                        h.state_mut(),
                        "measure.add",
                        json!({"table": "Sales", "expression": "SUM(Sales[Amount])"}),
                    ),
                    _ => form::open(h.state_mut(), "file.open", json!({"path": "a.pbix"})),
                }
                for _ in 0..4 {
                    h.step();
                }
            }
            let (a, b) = (frame(&desktop), frame(&browser));
            assert!(a.0 == b.0, "{lang} step {step}: the meshes differ");
            assert!(a.1 == b.1, "{lang} step {step}: the font atlases differ");
        }
    }
}

/// The Refresh info table (`table.add_refresh_info`): its form runs with the declared defaults,
/// then its measure and columns are fields like any other: the bind form's field picker offers
/// them, and `measure.add` takes DAX over them.
#[test]
fn the_refresh_info_table_becomes_fields_to_bind() {
    let mut h = app_with("en-US", Caller::Editor, session);
    form::open(h.state_mut(), "table.add_refresh_info", json!({}));
    steps(&mut h, 3);
    h.get_by_label("Run").click();
    steps(&mut h, 5);
    let f = h.state().form.as_ref().unwrap();
    assert!(matches!(f.result, Some(Ok(_))), "{:?}", f.result);
    assert_eq!(h.state().sent.last().unwrap().id, "table.add_refresh_info");
    h.state_mut().form = None;
    steps(&mut h, 1);
    form::open(h.state_mut(), "visual.bind", json!({}));
    steps(&mut h, 3);
    {
        let app = h.state();
        let f = app.form.as_ref().unwrap();
        let field = f.input("field").unwrap().field;
        let opts: Vec<String> = form::options(app, f, field).into_iter().map(|(v, _)| v).collect();
        for want in [
            "[Last refresh]",
            "Refresh info[Last refresh (UTC)]",
            "Refresh info[Last refresh (local)]",
        ] {
            assert!(opts.iter().any(|o| o == want), "{want}: {opts:?}");
        }
    }
    h.state_mut().form = None;
    steps(&mut h, 1);
    form::open(h.state_mut(), "measure.add", json!({"table": "Sales"}));
    steps(&mut h, 3);
    {
        let f = h.state_mut().form.as_mut().unwrap();
        f.input_mut("name").unwrap().text = "Refreshed".into();
        f.input_mut("expression").unwrap().text =
            "FORMAT([Last refresh], \"yyyy-mm-dd hh:nn\") & \" UTC\"".into();
    }
    h.get_by_label("Run").click();
    steps(&mut h, 5);
    let f = h.state().form.as_ref().unwrap();
    assert!(matches!(f.result, Some(Ok(_))), "{:?}", f.result);
}
