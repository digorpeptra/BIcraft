//! The ribbon built from the command registry, its contextual tabs, menus and shortcuts, and the
//! M11.3 coverage test: every engine command is reachable from the UI's own tables.

use std::collections::BTreeSet;
use std::path::Path;

use bicraft_engine::Session;
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, right_click, steps};
use super::{frame, harness, shell_app, shop_snapshot};
use crate::App;
use crate::app::{RibbonTab, Tab};
use crate::model_view::Sel;
use crate::registry::{self, At, Tab as RTab};
use crate::ribbon::{self, VisualSel};
use crate::shell::Caps;

/// A model with a table, a measure and a report of two pages, a card on the first.
fn rich_session() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Amount"}, {{"Wheels", 100}, {"Toys", 5}})"#;
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
        ("page.add", json!({"display_name": "Second"})),
    ] {
        s.execute(id, &p).unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute(
        "visual.add",
        &json!({"page": page, "visual_type": "card", "x": 100, "y": 100, "width": 300, "height": 200,
                "fields": [{"role": "Values", "field": "[Total]"}]}),
    )
    .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn rich_app(lang: &str) -> Harness<'static, App> {
    let mut app = App::new(lang, engine_shell(Caps::DESKTOP, rich_session)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1280.0, 800.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    h
}

fn pages(app: &App) -> usize {
    app.page_names().len()
}

fn visuals(app: &App) -> usize {
    app.loaded
        .as_ref()
        .and_then(|l| {
            l.snap
                .pointer("/report/pages/0/visuals")?
                .as_array()
                .map(Vec::len)
        })
        .unwrap_or(0)
}

fn press(h: &mut Harness<'_, App>, key: Key, modifiers: Modifiers) {
    for pressed in [true, false] {
        h.event_modifiers(
            Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            },
            modifiers,
        );
    }
    steps(h, 3);
}

/// Where report position (`x`, `y`) of the page shown is on the screen.
fn on_page(app: &App, x: f32, y: f32) -> Pos2 {
    let p = app.placed.unwrap();
    p.rect.min + egui::vec2(x, y) * p.zoom
}

fn click_at(h: &mut Harness<'_, App>, pos: Pos2) {
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.step();
}

fn show_ribbon(h: &mut Harness<'_, App>, tab: RTab) {
    if let Some(v) = ribbon::view_of(tab) {
        h.state_mut().show_tab(v);
    }
    h.state_mut().ribbon_tab = tab;
    steps(h, 3);
}

/// Selects what the contextual `tab` needs: the card on the canvas, the table or its measure.
fn select_for(h: &mut Harness<'_, App>, tab: RTab) {
    let page = h.state().page_names()[0].0.clone();
    let card = h
        .state()
        .loaded
        .as_ref()
        .and_then(|l| {
            l.snap
                .pointer("/report/pages/0/visuals/0/name")?
                .as_str()
                .map(str::to_string)
        })
        .unwrap();
    match tab {
        RTab::Visual => {
            h.state_mut().show_tab(Tab::Report);
            h.state_mut().cmd.visual = Some(VisualSel { page, visual: card });
        }
        RTab::Table => {
            h.state_mut().show_tab(Tab::Model);
            h.state_mut().model_view.sel = Some(Sel::Table("Sales".into()));
        }
        RTab::Measure => {
            h.state_mut().show_tab(Tab::Model);
            h.state_mut().model_view.sel = Some(Sel::Measure {
                table: "Sales".into(),
                name: "Total".into(),
            });
        }
        _ => {}
    }
    steps(h, 2);
}

/// ROADMAP M11.3, AGENTS.md §9: every command in `COMMANDS` is reachable from the UI's own
/// tables (a ribbon button of a tab the tab row draws, a context-menu item, the File view, or the
/// palette for palette-placed commands), and every id those tables name is a command. It fails
/// listing the missing ids.
#[test]
fn every_command_has_a_button() {
    let ribbon: BTreeSet<&str> = ribbon::ribbon_ids().into_iter().collect();
    let menus: BTreeSet<&str> = crate::menus::ids().into_iter().collect();
    let file: BTreeSet<&str> = crate::backstage::IDS.iter().copied().collect();
    // the command palette's own entries (M11.2): every command, whatever its place
    let palette: BTreeSet<&str> = crate::palette::entries().into_iter().collect();
    let engine: BTreeSet<&str> = bicraft_engine::COMMANDS.iter().map(|c| c.id).collect();
    let mut missing = Vec::new();
    for c in bicraft_engine::COMMANDS {
        let reached = match c.place.at {
            bicraft_engine::At::Ribbon {
                tab: bicraft_engine::Tab::File,
                ..
            } => file.contains(c.id),
            bicraft_engine::At::Ribbon { .. } => ribbon.contains(c.id),
            bicraft_engine::At::Menu(_) => menus.contains(c.id),
            bicraft_engine::At::Palette => palette.contains(c.id),
        };
        if !reached || !palette.contains(c.id) {
            missing.push(c.id);
        }
    }
    assert!(missing.is_empty(), "commands without a button: {missing:?}");
    let mut unknown: Vec<&str> = ribbon
        .iter()
        .chain(&menus)
        .chain(&file)
        .chain(crate::actions::IDS)
        .chain(crate::perf_pane::COMMANDS)
        .chain(crate::pq_editor::COMMANDS)
        .filter(|id| !engine.contains(*id))
        .copied()
        .collect();
    unknown.dedup();
    assert!(unknown.is_empty(), "buttons without a command: {unknown:?}");
}

/// The other direction for the panes and dialogs (Format, Filters, Selection, Bookmarks, Sync
/// slicers, the fx dialog, the Data view…), which run commands from their own code rather than
/// from the ribbon's tables: every string literal of the UI's sources shaped like a command id
/// (`namespace.name`, the namespace one of the engine's) names an engine command, so a button
/// never sends an id the CLI and the MCP server would not know.
#[test]
fn every_command_id_the_ui_names_is_an_engine_command() {
    let engine: BTreeSet<&str> = bicraft_engine::COMMANDS.iter().map(|c| c.id).collect();
    let spaces: BTreeSet<&str> = engine.iter().filter_map(|id| id.split('.').next()).collect();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut unknown = Vec::new();
    let mut files = 0;
    for entry in std::fs::read_dir(&dir).expect("crates/ui/src") {
        let path = entry.expect("an entry").path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // the sources only: tests may name ids on purpose that are not commands
        if !name.ends_with(".rs") || name.contains("test") || name == "registry_table.rs" {
            continue;
        }
        files += 1;
        let text = std::fs::read_to_string(&path).expect("a source file");
        for lit in text.split('"').skip(1).step_by(2) {
            let Some((space, rest)) = lit.split_once('.') else {
                continue;
            };
            let shaped = !rest.is_empty()
                && lit.len() <= 60
                && lit
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.');
            if shaped && spaces.contains(space) && !engine.contains(lit) {
                unknown.push(format!("{name}: {lit}"));
            }
        }
    }
    assert!(files > 20, "the UI's sources were read ({files} files)");
    unknown.sort();
    unknown.dedup();
    assert!(
        unknown.is_empty(),
        "ids that are not engine commands: {unknown:?}"
    );
}

/// Every ribbon tab draws a button for each of its commands, found by its label in the
/// accessibility tree, in English and French; the contextual tabs appear with their selection.
#[test]
fn every_ribbon_button_is_drawn() {
    for (lang, l) in [("en-US", crate::i18n::Lang::En), ("fr-FR", crate::i18n::Lang::Fr)] {
        let mut h = rich_app(lang);
        for tab in ribbon::TABS
            .into_iter()
            .chain([RTab::Visual, RTab::Table, RTab::Measure])
        {
            if tab.contextual() {
                select_for(&mut h, tab);
                tab_node(&h, tab.caption(l)).click();
                steps(&mut h, 3);
            } else {
                tab_node(&h, tab.caption(l)).click();
                steps(&mut h, 3);
            }
            assert_eq!(h.state().ribbon_tab, tab, "{lang}: {tab:?} not shown");
            for (g, cmds) in registry::ribbon_groups(tab) {
                // Model › Parameters is one button whose menu holds the commands
                let menu = tab == RTab::Model && g == crate::param_dialog::GROUP;
                if menu {
                    let new = if l == crate::i18n::Lang::Fr {
                        "Nouveau paramètre"
                    } else {
                        "New parameter"
                    };
                    h.get_by_label(new).click();
                    steps(&mut h, 3);
                }
                for c in cmds {
                    assert!(
                        h.query_all_by_label(c.text(l)).count() >= 1,
                        "{lang}: {tab:?} has no button {} ({})",
                        c.text(l),
                        c.id
                    );
                }
                if menu {
                    h.key_press(egui::Key::Escape);
                    steps(&mut h, 3);
                }
            }
        }
    }
}

/// The contextual tabs follow the selection: a click on a visual shows Visual, a click on
/// nothing or Escape hides it; a table or a measure selected in the Model view shows Table or
/// Measure.
#[test]
fn contextual_tabs_follow_the_selection() {
    let mut h = rich_app("en-US");
    assert!(h.query_by_label("Visual").is_none());
    let pos = on_page(h.state(), 200.0, 200.0);
    click_at(&mut h, pos);
    assert!(h.state().cmd.visual.is_some(), "the card is selected");
    h.get_by_label("Visual").click();
    steps(&mut h, 2);
    assert_eq!(h.state().ribbon_tab, RTab::Visual);
    h.get_by_label("Format");
    let empty = on_page(h.state(), 1000.0, 600.0);
    click_at(&mut h, empty);
    assert!(h.state().cmd.visual.is_none());
    steps(&mut h, 2);
    assert!(h.query_by_label("Visual").is_none());
    assert_eq!(h.state().ribbon_tab, RTab::Report, "back to the view's tab");
    click_at(&mut h, pos);
    press(&mut h, Key::Escape, Modifiers::NONE);
    assert!(h.state().cmd.visual.is_none());
    select_for(&mut h, RTab::Table);
    assert_eq!(ribbon::contextual(h.state()), vec![RTab::Table]);
    tab_node(&h, "Table");
    select_for(&mut h, RTab::Measure);
    assert_eq!(ribbon::contextual(h.state()), vec![RTab::Measure]);
    tab_node(&h, "Measure");
}

/// A button runs its command on the session: New page adds one (and Undo removes it); a
/// command the session cannot run is greyed and says why.
#[test]
fn buttons_run_their_commands_and_disabled_ones_say_why() {
    let mut h = rich_app("en-US");
    let n = pages(h.state());
    show_ribbon(&mut h, RTab::Home);
    // a command a PBIP-less session cannot run: greyed, with the engine's reason
    assert!(h.get_by_label("Refresh").accesskit_node().is_disabled());
    show_ribbon(&mut h, RTab::Report);
    h.get_by_label("New page").click();
    steps(&mut h, 4);
    assert_eq!(pages(h.state()), n + 1);
    assert_eq!(h.state().sent.last().map(|c| c.id), Some("page.add"));
    show_ribbon(&mut h, RTab::Home);
    let undo = h.get_by_label("Undo");
    assert!(!undo.accesskit_node().is_disabled());
    undo.click();
    steps(&mut h, 4);
    assert_eq!(pages(h.state()), n);
    // the reason is the tooltip of the greyed button
    show_ribbon(&mut h, RTab::Report);
    let rect = h.get_by_label("New report").rect();
    h.hover_at(rect.center());
    for _ in 0..30 {
        h.step();
    }
    assert!(
        h.query_all_by_label_contains("the session already has a report")
            .count()
            >= 1,
        "the tooltip says why New report is greyed"
    );
}

/// A command that needs values opens the parameters form with what the selection gives; a read
/// command shows its answer.
#[test]
fn forms_and_answers() {
    let mut h = rich_app("en-US");
    show_ribbon(&mut h, RTab::Home);
    h.get_by_label("Import table").click();
    steps(&mut h, 2);
    let f = h.state().form.clone().expect("the form is open");
    assert_eq!(f.id, "table.import");
    assert!(h.state().sent.iter().all(|c| c.id != "table.import"));
    press(&mut h, Key::Escape, Modifiers::NONE);
    assert!(h.state().form.is_none());
    // New measure is written in the formula bar, as in Desktop (no form)
    h.get_by_label("New measure").click();
    steps(&mut h, 2);
    assert!(h.state().form.is_none());
    assert!(h.state().formula.target.as_ref().is_some_and(|t| t.new));
    assert!(h.state().sent.iter().all(|c| c.id != "measure.add"));
    h.state_mut().formula = Default::default();
    // the Table tab: the selected table is in the form
    select_for(&mut h, RTab::Table);
    show_ribbon(&mut h, RTab::Table);
    h.get_by_label("Table properties").click();
    steps(&mut h, 2);
    let f = h.state().form.clone().expect("the form is open");
    assert_eq!((f.id, &f.prefill), ("table.set", &json!({"name": "Sales"})));
    h.state_mut().form = None;
    // a read command: its answer in a window
    show_ribbon(&mut h, RTab::Analyze);
    h.get_by_label("Validate model").click();
    steps(&mut h, 4);
    let (id, answer) = h.state().cmd.result.clone().expect("the answer is shown");
    assert_eq!(id, "model.validate");
    assert!(matches!(answer, Some(Ok(Value::Object(_)))), "{answer:?}");
    h.get_by_label("model.validate");
    // the Validation view is in Analyze
    h.state_mut().cmd.result = None;
    steps(&mut h, 2);
    h.get_by_label("Show validation").click();
    steps(&mut h, 2);
    assert_eq!(h.state().tab, Tab::Validation);
    assert_eq!(h.state().ribbon_tab, RTab::Analyze);
}

/// The right-click menus: a visual's (Delete visual removes it), a page tab's (Open on this
/// page), a model table's and the diagram's.
#[test]
fn context_menus_run_their_commands() {
    let mut h = rich_app("en-US");
    let pos = on_page(h.state(), 200.0, 200.0);
    right_click(&mut h, pos);
    assert!(h.state().cmd.visual.is_some());
    h.get_by_label("Sort");
    h.get_by_label("Delete visual").click();
    steps(&mut h, 4);
    assert_eq!(visuals(h.state()), 0, "the card is deleted");
    // a page tab
    let tab = super::backstage_tests::page_tab(&h, "Second");
    right_click(&mut h, tab.center());
    h.get_by_label("Open on this page").click();
    steps(&mut h, 4);
    assert_eq!(h.state().sent.last().map(|c| c.id), Some("page.set_active"));
    // the model diagram: a table card, then the background
    h.state_mut().show_tab(Tab::Model);
    steps(&mut h, 4);
    let head = h.state().model_view.diagram().unwrap().cards[0].head.center();
    let mv = &h.state().model_view;
    let at = mv.canvas.min + mv.pan + head.to_vec2() * mv.zoom;
    right_click(&mut h, at);
    assert_eq!(h.state().model_view.sel, Some(Sel::Table("Sales".into())));
    h.get_by_label("Rename table");
    h.get_by_label("Delete table").click();
    steps(&mut h, 3);
    let f = h.state().form.clone().expect("Delete table asks for its options");
    assert_eq!((f.id, &f.prefill), ("table.remove", &json!({"name": "Sales"})));
}

/// The registry's shortcuts: Ctrl+M adds a page, Ctrl+Z undoes it, Delete removes the selected
/// visual, F5 reloads the file (model.refresh needs a PBIP project).
#[test]
fn shortcuts_run_their_commands() {
    let mut h = rich_app("en-US");
    let n = pages(h.state());
    press(&mut h, Key::M, Modifiers::COMMAND);
    assert_eq!(pages(h.state()), n + 1);
    press(&mut h, Key::Z, Modifiers::COMMAND);
    assert_eq!(pages(h.state()), n);
    press(&mut h, Key::Delete, Modifiers::NONE);
    assert_eq!(
        visuals(h.state()),
        1,
        "Delete without a selection deletes nothing"
    );
    let pos = on_page(h.state(), 200.0, 200.0);
    click_at(&mut h, pos);
    press(&mut h, Key::Delete, Modifiers::NONE);
    assert_eq!(visuals(h.state()), 0);
    // F5 without a PBIP project: a reload, not model.refresh
    press(&mut h, Key::F5, Modifiers::NONE);
    assert!(h.state().sent.iter().all(|c| c.id != "model.refresh"));
}

/// The ribbon tabs draw the same frames in the desktop and the browser shells, the contextual
/// ones too (Home differs: its File group follows the shell's capabilities).
#[test]
fn the_ribbon_tabs_draw_the_same_frames_in_both_shells() {
    let snap = shop_snapshot();
    let card = snap
        .snap
        .pointer("/report/pages/0/visuals/1/name")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let page = snap
        .snap
        .pointer("/report/pages/0/name")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    for dark in [false, true] {
        let mut desktop = shell_app(Caps::DESKTOP, &snap, "en-US");
        let mut browser = shell_app(Caps::BROWSER, &snap, "en-US");
        for tab in [
            RTab::Report,
            RTab::Model,
            RTab::Data,
            RTab::Analyze,
            RTab::View,
            RTab::Visual,
            RTab::Table,
            RTab::Measure,
        ] {
            for h in [&mut desktop, &mut browser] {
                if dark {
                    h.state_mut().set_theme(crate::theme::Brightness::Dark);
                }
                match tab {
                    RTab::Visual => {
                        h.state_mut().show_tab(Tab::Report);
                        h.state_mut().cmd.visual = Some(VisualSel {
                            page: page.clone(),
                            visual: card.clone(),
                        });
                    }
                    RTab::Table => {
                        h.state_mut().show_tab(Tab::Model);
                        h.state_mut().model_view.sel = Some(Sel::Table("Sales".into()));
                    }
                    RTab::Measure => {
                        h.state_mut().show_tab(Tab::Model);
                        h.state_mut().model_view.sel = Some(Sel::Measure {
                            table: "Sales".into(),
                            name: "Total".into(),
                        });
                    }
                    _ => {}
                }
                show_ribbon(h, tab);
                assert_eq!(h.state().ribbon_tab, tab);
            }
            let (a, b) = (frame(&desktop), frame(&browser));
            assert!(a.0 == b.0, "{tab:?} dark={dark}: the meshes differ");
            assert!(a.1 == b.1, "{tab:?}: the font atlases differ");
            assert!(a.2 == b.2, "{tab:?}: the pages differ");
        }
    }
}

/// The view's tab and the ribbon's stay together: the Validation view is Analyze's, a
/// contextual tab whose selection goes away gives way to its view's tab.
#[test]
fn the_ribbon_follows_the_view() {
    assert_eq!(ribbon::tab_of(Tab::Validation), RTab::Analyze);
    assert_eq!(ribbon::view_of(RTab::Visual), Some(Tab::Report));
    // the Data tab shows the Data view (M11.17); Analyze keeps the view shown
    assert_eq!(ribbon::view_of(RTab::Data), Some(Tab::Data));
    assert_eq!(ribbon::view_of(RTab::Analyze), None);
    let mut h = rich_app("en-US");
    h.state_mut().show_tab(Tab::Validation);
    steps(&mut h, 2);
    assert_eq!(h.state().ribbon_tab, RibbonTab::Analyze);
    // every place the registry names is a tab the UI knows
    for c in registry::COMMANDS {
        if let At::Ribbon { tab, .. } = c.at {
            assert!(
                tab == RTab::File || ribbon::TABS.contains(&tab) || tab.contextual(),
                "{}",
                c.id
            );
        }
    }
    assert!(ribbon::chord("Ctrl+Shift+S").is_some_and(|(m, k)| m.shift && m.command && k == Key::S));
    assert_eq!(ribbon::chord("F5").map(|x| x.1), Some(Key::F5));
    assert!(ribbon::chord("Hyper+Nope").is_none());
}

/// The tab row's tab `caption` (the topmost node of that name: a pane may have one too).
fn tab_node<'a>(h: &'a Harness<'_, App>, caption: &'a str) -> egui_kittest::Node<'a> {
    h.get_all_by_label(caption)
        .min_by(|a, b| a.rect().center().y.total_cmp(&b.rect().center().y))
        .unwrap()
}
