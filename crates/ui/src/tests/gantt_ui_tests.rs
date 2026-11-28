//! A Gantt's reader controls on the canvas (G99, egui_kittest over an engine session): the
//! rows' scroll bar shows the tasks below the visual from the keyboard, the time line's zoom
//! narrows the axis and Home shows all of it again, a category's expand/collapse button hides
//! and shows its tasks through `view.interact`, all labelled in English and French. Scrolling
//! and zoom are the app's view state: nothing is sent to the engine for them.

use std::path::Path;

use bicraft_engine::Session;
use bicraft_render::hits::HitAction;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::engine_shell;
use super::harness;
use crate::App;
use crate::shell::Caps;

/// Tasks(Phase, Task, Start, End): phases P0 and P1 of twenty tasks each; on the page a Gantt
/// `plan`, 600 × 300 px (the forty tasks do not fit).
fn plan_session() -> Session {
    let mut s = Session::new();
    let rows: Vec<String> = (0..40)
        .map(|i| {
            let (m, d) = (1 + i / 20, 1 + i % 20);
            format!(
                "{{\"P{}\", \"T{i:02}\", #date(2025, {m}, {d}), #date(2025, {m}, {})}}",
                i / 20,
                d + 5
            )
        })
        .collect();
    let m = format!(
        "let Source = #table(type table [Phase = text, Task = text, Start = date, End = date], {{{}}}) in Source",
        rows.join(", ")
    );
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    run(&mut s, "model.new", json!({"name": "Plan"}));
    run(&mut s, "table.import", json!({"name": "Tasks", "m": m}));
    run(&mut s, "report.new", json!({}));
    let page = s
        .report
        .as_ref()
        .map(|r| r.pages[0].name.clone())
        .unwrap_or_default();
    let r = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "ganttChart", "x": 20, "y": 20, "width": 600, "height": 300}),
    );
    let made = r["visual"].as_str().unwrap_or_default().to_string();
    if let Some(v) = s
        .report
        .as_mut()
        .and_then(|r| r.pages.first_mut())
        .and_then(|p| p.visuals.iter_mut().find(|v| v.name == made))
    {
        v.name = "plan".into();
    }
    for (role, field) in [
        ("Task", "Tasks[Task]"),
        ("StartDate", "Min(Tasks[Start])"),
        ("EndDate", "Max(Tasks[End])"),
        ("Category", "Tasks[Phase]"),
    ] {
        run(
            &mut s,
            "visual.bind",
            json!({"page": page, "visual": "plan", "role": role, "field": field}),
        );
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn app(locale: &str) -> Harness<'static, App> {
    let mut app = App::new(locale, engine_shell(Caps::DESKTOP, plan_session)).with_sync_render();
    app.prefs.panes.data = false;
    app.prefs.panes.build = false;
    app.prefs.panes.filters = false;
    app.open_path(Path::new("plan.pbip"));
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

/// The task and category members whose rows are drawn (their selection regions).
fn drawn(h: &Harness<'_, App>) -> Vec<Value> {
    let Some((_, Ok(img))) = h.state().shown.as_ref() else {
        return Vec::new();
    };
    img.hits
        .iter()
        .filter(|x| x.visual == "plan")
        .filter_map(|x| match &x.action {
            HitAction::Send { action, .. } if action["action"] == "select" => {
                action["point"].as_array()?.last().map(|p| p["v"].clone())
            }
            _ => None,
        })
        .collect()
}

/// The Gantt's `gantt_view` region (what the renderer applied).
fn view(h: &Harness<'_, App>) -> Value {
    let Some((_, Ok(img))) = h.state().shown.as_ref() else {
        return Value::Null;
    };
    img.hits
        .iter()
        .find_map(|x| match &x.action {
            HitAction::Menu(m) if m["kind"] == "gantt_view" && x.visual == "plan" => Some(m.clone()),
            _ => None,
        })
        .unwrap_or(Value::Null)
}

fn label_with(h: &Harness<'_, App>, part: &str) -> String {
    h.query_all_by_label_contains(part)
        .next()
        .map(|n| n.accesskit_node().label().unwrap_or_default().to_string())
        .unwrap_or_else(|| panic!("no control labelled with {part}"))
}

#[test]
fn the_scroll_bar_shows_the_tasks_below_the_visual_from_the_keyboard() {
    let mut h = app("en-US");
    assert!(drawn(&h).contains(&json!("T00")));
    assert!(!drawn(&h).contains(&json!("T39")), "{:?}", drawn(&h));
    let bar = label_with(&h, "Scroll the tasks");
    assert_eq!(
        h.get_by_label(&bar).accesskit_node().role(),
        egui::accesskit::Role::ScrollBar
    );
    h.get_by_label(&bar).focus();
    // two frames: the bar keeps the arrows once it has had the focus for a frame
    h.step();
    h.step();
    h.key_press(egui::Key::ArrowDown);
    settle(&mut h);
    assert_eq!(view(&h)["first"], 1, "{}", view(&h));
    h.key_press(egui::Key::End);
    settle(&mut h);
    let d = drawn(&h);
    assert!(d.contains(&json!("T39")) && !d.contains(&json!("T00")), "{d:?}");
    // the label follows: the last rows of the list (two categories and forty tasks)
    let bar = label_with(&h, "Scroll the tasks");
    assert!(bar.ends_with("42 / 42"), "{bar}");
    h.key_press(egui::Key::Home);
    settle(&mut h);
    assert_eq!(view(&h)["first"], 0);
    // view state only: nothing went to the engine
    assert!(h.state().sent.iter().all(|c| c.id != "view.interact"));
}

#[test]
fn the_time_line_zooms_from_the_keyboard_and_home_shows_all_of_it() {
    let mut h = app("en-US");
    let zoom = label_with(&h, "Zoom the time line");
    assert_eq!(
        h.get_by_label(&zoom).accesskit_node().role(),
        egui::accesskit::Role::Slider
    );
    h.get_by_label(&zoom).focus();
    h.step();
    h.step();
    h.key_press(egui::Key::Plus);
    settle(&mut h);
    assert_eq!(view(&h)["zoom"], 2.0, "{}", view(&h));
    assert_eq!(view(&h)["hbar"], true);
    assert!(h.query_by_label("Time line position").is_some());
    h.key_press(egui::Key::ArrowRight);
    settle(&mut h);
    assert!(view(&h)["at"].as_f64().unwrap_or(0.0) > 0.5, "{}", view(&h));
    h.key_press(egui::Key::Home);
    settle(&mut h);
    assert_eq!(
        (view(&h)["zoom"].clone(), view(&h)["hbar"].clone()),
        (json!(1.0), json!(false))
    );
    assert!(h.state().sent.iter().all(|c| c.id != "view.interact"));
}

#[test]
fn a_categorys_button_collapses_and_expands_its_tasks() {
    let mut h = app("en-US");
    assert!(drawn(&h).contains(&json!("T00")));
    let collapse = h.get_by_label("Collapse P0");
    assert_eq!(collapse.accesskit_node().role(), egui::accesskit::Role::Button);
    collapse.focus();
    h.step();
    h.key_press(egui::Key::Enter);
    settle(&mut h);
    assert_eq!(
        h.state().last_action.as_ref().map(|a| a["action"].clone()),
        Some(json!("gantt_toggle"))
    );
    let d = drawn(&h);
    assert!(!d.contains(&json!("T00")) && d.contains(&json!("T20")), "{d:?}");
    h.get_by_label("Expand P0").click();
    settle(&mut h);
    assert!(drawn(&h).contains(&json!("T00")));
    // the report is not changed
    assert!(
        !h.state()
            .sent
            .iter()
            .any(|c| c.id != "view.interact" && c.id.starts_with("visual."))
    );
}

#[test]
fn the_controls_are_labelled_in_french() {
    let h = app("fr-FR");
    assert!(h.query_by_label("Réduire P0").is_some());
    label_with(&h, "Faire défiler les tâches");
    label_with(&h, "Zoom de la chronologie");
}
