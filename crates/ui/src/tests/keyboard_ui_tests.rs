//! Keyboard navigation of the app (`crate::keyboard`), driven by key presses only, read back
//! through the focus and the accessibility tree (egui_kittest, no GPU): Ctrl+F6 cycles the
//! window's sections (ribbon, formula bar, view, panes, page tabs, status bar) and Ctrl+Shift+F6
//! goes back; Tab stays inside a section; Right and Left unroll and roll the Data pane's tables;
//! the ribbon's Manage relationships opens from Enter, keeps the focus inside the dialog through
//! Tab and gives it back to the button on Escape (Manage security roles and Customize theme
//! too); every button, check box, text field and combo box has an accessible name in every view
//! and those dialogs, in English and French.

use std::path::Path;

use bicraft_engine::Session;
use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, Order, Rect};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use crate::App;
use crate::app::Tab;
use crate::keyboard::{self, Section};
use crate::registry::Tab as RTab;
use crate::shell::Caps;

/// Sales(Category, Amount) with a measure, Product, a relationship, a role, a report with a
/// column chart.
fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.import",
            json!({"name": "Sales", "m": "#table(type table [ProductKey = number, Category = text, Amount = number], {{1, \"A\", 10}, {2, \"B\", 5}})"}),
        ),
        (
            "table.import",
            json!({"name": "Product", "m": "#table(type table [ProductKey = number, Name = text], {{1, \"Bike\"}, {2, \"Car\"}})"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[ProductKey]", "to": "Product[ProductKey]"}),
        ),
        ("role.add", json!({"name": "East"})),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute(
        "visual.add",
        &json!({"page": page, "visual_type": "clusteredColumnChart", "x": 20, "y": 20,
                "width": 400, "height": 300,
                "fields": [{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]}),
    )
    .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

/// The app in `lang` on view `view` with ribbon tab `rtab`.
fn app(lang: &str, view: Tab, rtab: RTab) -> Harness<'static, App> {
    app_with(lang, view, rtab, session)
}

/// [`app`] over the session `make` builds.
fn app_with(lang: &str, view: Tab, rtab: RTab, make: fn() -> Session) -> Harness<'static, App> {
    let mut app = App::new(lang, engine_shell(Caps::DESKTOP, make)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1400.0, 900.0));
    for _ in 0..200 {
        h.step();
        if h.state().loaded.is_some() {
            break;
        }
    }
    h.state_mut().show_tab(view);
    h.state_mut().ribbon_tab = rtab;
    steps(&mut h, 6);
    h
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

fn ctrl_f6(h: &mut Harness<'_, App>, back: bool) {
    let m = if back {
        Modifiers::CTRL | Modifiers::SHIFT
    } else {
        Modifiers::CTRL
    };
    press(h, Key::F6, m);
}

fn tab(h: &mut Harness<'_, App>, back: bool) {
    let m = if back { Modifiers::SHIFT } else { Modifiers::NONE };
    press(h, Key::Tab, m);
}

/// The focused control's rectangle.
fn focused(h: &Harness<'_, App>) -> Option<Rect> {
    keyboard::focused(&h.ctx).map(|(_, r)| r)
}

/// The section holding the focused control (the smallest one).
fn section_of_focus(h: &Harness<'_, App>) -> Option<Section> {
    let r = focused(h)?;
    keyboard::sections(h.state())
        .into_iter()
        .filter(|(_, s)| s.contains(r.center()))
        .min_by(|a, b| a.1.area().total_cmp(&b.1.area()))
        .map(|(s, _)| s)
}

fn rect_of(h: &Harness<'_, App>, role: Role, label: &str) -> Rect {
    h.get_all_by_role_and_label(role, label)
        .map(|n| n.rect())
        .min_by(|a, b| a.min.y.total_cmp(&b.min.y))
        .unwrap_or_else(|| panic!("no {role:?} `{label}`"))
}

fn same(a: Rect, b: Rect) -> bool {
    (a.center() - b.center()).length() < 2.0
}

/// Tab (from the section's first control) until `target` has the focus.
fn tab_to(h: &mut Harness<'_, App>, target: Rect) {
    for _ in 0..120 {
        if focused(h).is_some_and(|r| same(r, target)) {
            return;
        }
        tab(h, false);
    }
    panic!("Tab never reached {target:?}; focus at {:?}", focused(h));
}

/// Ctrl+F6 until the focus is in section `s`.
fn f6_to(h: &mut Harness<'_, App>, s: Section) {
    for _ in 0..12 {
        if section_of_focus(h) == Some(s) {
            return;
        }
        ctrl_f6(h, false);
    }
    panic!("Ctrl+F6 never reached {s:?}");
}

#[test]
fn ctrl_f6_cycles_the_sections_and_back() {
    let mut h = app("en-US", Tab::Report, RTab::Home);
    assert!(focused(&h).is_none());
    let mut seen = Vec::new();
    for _ in 0..10 {
        ctrl_f6(&mut h, false);
        let s = section_of_focus(&h).expect("Ctrl+F6 focuses a control of a section");
        if seen.first() == Some(&s) {
            break;
        }
        seen.push(s);
    }
    assert_eq!(seen.first(), Some(&Section::Ribbon), "{seen:?}");
    for s in [Section::Main, Section::Pane, Section::PageTabs, Section::Status] {
        assert!(seen.contains(&s), "{s:?} not in the cycle {seen:?}");
    }
    // in the cycle's order
    let mut sorted = seen.clone();
    sorted.sort();
    assert_eq!(seen, sorted);
    // the ring follows the keyboard
    assert!(h.state().keyboard.keyboard);
    // Ctrl+Shift+F6 goes back: from the ribbon to the status bar
    assert_eq!(section_of_focus(&h), Some(Section::Ribbon));
    ctrl_f6(&mut h, true);
    assert_eq!(section_of_focus(&h), Some(Section::Status));
    ctrl_f6(&mut h, true);
    assert_eq!(section_of_focus(&h), Some(Section::PageTabs));
    // plain F6 without the Selection pane moves as Ctrl+F6
    press(&mut h, Key::F6, Modifiers::NONE);
    assert_eq!(section_of_focus(&h), Some(Section::Status));
}

#[test]
fn f6_activates_the_selection_pane() {
    let mut h = app("en-US", Tab::Report, RTab::Home);
    h.state_mut().prefs.panes.selection = true;
    steps(&mut h, 4);
    press(&mut h, Key::F6, Modifiers::NONE);
    assert_eq!(section_of_focus(&h), Some(Section::Selection));
}

#[test]
fn tab_stays_inside_a_section() {
    let mut h = app("en-US", Tab::Report, RTab::Home);
    f6_to(&mut h, Section::PageTabs);
    let first = focused(&h).unwrap();
    // Tab goes round the page tabs and back to the first, never out of the section
    let mut back = false;
    for _ in 0..20 {
        tab(&mut h, false);
        assert_eq!(section_of_focus(&h), Some(Section::PageTabs));
        if focused(&h).is_some_and(|r| same(r, first)) {
            back = true;
            break;
        }
    }
    assert!(back, "Tab wraps inside the page tabs");
    // Shift+Tab from the first goes to the section's last, still inside
    tab(&mut h, true);
    assert_eq!(section_of_focus(&h), Some(Section::PageTabs));
    assert!(!focused(&h).is_some_and(|r| same(r, first)));
    // the ribbon too
    f6_to(&mut h, Section::Ribbon);
    for _ in 0..80 {
        tab(&mut h, false);
        assert_eq!(section_of_focus(&h), Some(Section::Ribbon));
    }
}

#[test]
fn arrows_unroll_and_roll_a_data_pane_table() {
    let mut h = app("en-US", Tab::Report, RTab::Home);
    let sales = rect_of(&h, Role::Button, "Sales");
    f6_to(&mut h, Section::Pane);
    // the Data pane is one of the panes: Ctrl+F6 to the one that holds Sales
    for _ in 0..6 {
        let in_pane = keyboard::sections(h.state()).into_iter().any(|(s, r)| {
            s == Section::Pane
                && r.contains(sales.center())
                && focused(&h).is_some_and(|f| r.contains(f.center()))
        });
        if in_pane {
            break;
        }
        ctrl_f6(&mut h, false);
    }
    tab_to(&mut h, sales);
    assert!(!h.state().panes.open.contains("t:Sales"));
    press(&mut h, Key::ArrowRight, Modifiers::NONE);
    assert!(h.state().panes.open.contains("t:Sales"), "Right unrolls");
    h.get_by_label("Amount");
    assert!(
        focused(&h).is_some_and(|r| same(r, sales)),
        "the focus stays on the row"
    );
    press(&mut h, Key::ArrowLeft, Modifiers::NONE);
    assert!(!h.state().panes.open.contains("t:Sales"), "Left rolls up");
    // Enter toggles it too
    press(&mut h, Key::Enter, Modifiers::NONE);
    assert!(h.state().panes.open.contains("t:Sales"), "Enter unrolls");
}

/// The focus is on a dialog (a `Middle` layer).
fn in_dialog(h: &Harness<'_, App>) -> bool {
    keyboard::focused(&h.ctx).is_some_and(|(l, _)| l.order == Order::Middle)
}

/// Tab and Shift+Tab 30 times: the focus never leaves the dialog.
fn trapped(h: &mut Harness<'_, App>) {
    for i in 0..30 {
        tab(h, i % 7 == 6);
        assert!(
            in_dialog(h),
            "Tab #{i} left the dialog: {:?}",
            keyboard::focused(&h.ctx)
        );
    }
}

#[test]
fn manage_relationships_opens_from_enter_traps_tab_and_escape_gives_the_focus_back() {
    let mut h = app("en-US", Tab::Model, RTab::Model);
    let button = rect_of(&h, Role::Button, "Manage relationships");
    f6_to(&mut h, Section::Ribbon);
    tab_to(&mut h, button);
    press(&mut h, Key::Enter, Modifiers::NONE);
    steps(&mut h, 4);
    assert!(h.state().cmd.rels.is_some(), "Enter opens the dialog");
    assert!(in_dialog(&h), "the focus moves into the dialog");
    trapped(&mut h);
    // the canvas under it does not take the keyboard: Ctrl+F6 stays in the dialog
    ctrl_f6(&mut h, false);
    assert!(in_dialog(&h));
    press(&mut h, Key::Escape, Modifiers::NONE);
    steps(&mut h, 2);
    assert!(h.state().cmd.rels.is_none(), "Escape closes it");
    assert!(
        focused(&h).is_some_and(|r| same(r, button)),
        "the focus is back on the button: {:?}",
        focused(&h)
    );
}

#[test]
fn manage_roles_and_customize_theme_trap_the_focus_and_give_it_back() {
    let mut h = app("en-US", Tab::Model, RTab::Model);
    let button = rect_of(&h, Role::Button, "Manage relationships");
    f6_to(&mut h, Section::Ribbon);
    tab_to(&mut h, button);
    for open in [
        (|app: &mut App| crate::roles_dialog::open(app, None)) as fn(&mut App),
        crate::theme_dialog::open,
    ] {
        open(h.state_mut());
        steps(&mut h, 6);
        assert!(in_dialog(&h), "the focus moves into the dialog");
        trapped(&mut h);
        press(&mut h, Key::Escape, Modifiers::NONE);
        steps(&mut h, 2);
        assert!(h.state().cmd.roles.is_none() && h.state().cmd.theme.is_none());
        assert!(
            focused(&h).is_some_and(|r| same(r, button)),
            "the focus is back where it was: {:?}",
            focused(&h)
        );
    }
}

#[test]
fn a_pointer_press_hides_the_focus_ring() {
    let mut h = app("en-US", Tab::Report, RTab::Home);
    ctrl_f6(&mut h, false);
    assert!(h.state().keyboard.keyboard);
    let at = egui::pos2(700.0, 500.0);
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    assert!(!h.state().keyboard.keyboard);
}

#[test]
fn the_dax_query_view_is_a_section() {
    let mut h = app("en-US", Tab::Query, RTab::Home);
    // the editor may hold the focus already: go round from the ribbon
    f6_to(&mut h, Section::Ribbon);
    f6_to(&mut h, Section::Main);
}

#[test]
fn the_model_views_panes_are_sections_of_their_own() {
    let mut h = app("en-US", Tab::Model, RTab::Model);
    f6_to(&mut h, Section::Ribbon);
    f6_to(&mut h, Section::Main);
    let main = keyboard::sections(h.state())
        .into_iter()
        .find(|(s, _)| *s == Section::Main)
        .map(|(_, r)| r)
        .unwrap();
    // two panes inside the view: the Properties pane, then the Data pane at the right
    let panes: Vec<Rect> = keyboard::sections(h.state())
        .into_iter()
        .filter(|(s, r)| *s == Section::Pane && main.contains(r.center()))
        .map(|(_, r)| r)
        .collect();
    assert_eq!(panes.len(), 2, "{panes:?}");
    let mut seen = Vec::new();
    for _ in 0..2 {
        ctrl_f6(&mut h, false);
        let f = focused(&h).unwrap();
        assert_eq!(section_of_focus(&h), Some(Section::Pane));
        let i = panes
            .iter()
            .position(|p| p.contains(f.center()))
            .expect("in a pane");
        seen.push(i);
    }
    seen.sort();
    assert_eq!(seen, [0, 1], "Ctrl+F6 visits both panes");
}

/// Alt pressed and released alone.
fn alt(h: &mut Harness<'_, App>) {
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    steps(h, 3);
}

/// The key tip drawn over `rect`.
fn tip_over(h: &Harness<'_, App>, rect: Rect) -> Option<String> {
    h.state()
        .keyboard
        .tips
        .drawn
        .iter()
        .find(|(_, b)| {
            rect.expand(2.0).intersects(*b) && (b.center().x - rect.center().x).abs() < rect.width()
        })
        .map(|(c, _)| c.clone())
}

fn type_code(h: &mut Harness<'_, App>, code: &str) {
    for c in code.chars() {
        let key = Key::from_name(&c.to_string()).expect("a letter key");
        press(h, key, Modifiers::NONE);
    }
}

#[test]
fn alt_shows_key_tips_that_select_a_tab_and_press_its_buttons() {
    use crate::key_tips::{self, Level};
    for lang in ["en-US", "fr-FR"] {
        let mut h = app(lang, Tab::Report, RTab::Home);
        let l = h.state().lang;
        assert!(h.state().keyboard.tips.drawn.is_empty());
        alt(&mut h);
        assert_eq!(h.state().keyboard.tips.level, Level::Tabs);
        // a letter over each tab, generated from the registry
        let model_code = key_tips::tab_codes(l)
            .into_iter()
            .find(|(t, _)| *t == RTab::Model)
            .map(|(_, c)| c)
            .unwrap();
        let model_tab = rect_of(&h, Role::Button, RTab::Model.caption(l));
        assert_eq!(tip_over(&h, model_tab).as_deref(), Some(model_code.as_str()));
        let drawn: Vec<String> = h
            .state()
            .keyboard
            .tips
            .drawn
            .iter()
            .map(|d| d.0.clone())
            .collect();
        let mut unique = drawn.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), drawn.len(), "{drawn:?}");
        // the tab's letter selects it and shows its buttons' letters
        type_code(&mut h, &model_code);
        assert_eq!(h.state().ribbon_tab, RTab::Model);
        assert_eq!(h.state().tab, Tab::Model);
        assert_eq!(h.state().keyboard.tips.level, Level::Buttons(RTab::Model));
        let codes = key_tips::button_codes(RTab::Model, l);
        let (_, rel) = codes.iter().find(|(id, _)| *id == "relationship.list").unwrap();
        let button = rect_of(&h, Role::Button, registry_text("relationship.list", l));
        assert_eq!(tip_over(&h, button).as_deref(), Some(rel.as_str()));
        // Escape backs out: to the tabs, then off
        press(&mut h, Key::Escape, Modifiers::NONE);
        assert_eq!(h.state().keyboard.tips.level, Level::Tabs);
        press(&mut h, Key::Escape, Modifiers::NONE);
        assert_eq!(h.state().keyboard.tips.level, Level::Off);
        assert!(h.state().keyboard.tips.drawn.is_empty());
        // F10 shows them too; the button's letter runs what its click runs
        press(&mut h, Key::F10, Modifiers::NONE);
        assert_eq!(h.state().keyboard.tips.level, Level::Tabs);
        type_code(&mut h, &model_code);
        type_code(&mut h, rel);
        steps(&mut h, 4);
        assert!(h.state().cmd.rels.is_some(), "Manage relationships opens");
        assert_eq!(h.state().keyboard.tips.level, Level::Off);
        press(&mut h, Key::Escape, Modifiers::NONE);
        steps(&mut h, 2);
        assert!(h.state().cmd.rels.is_none());
    }
}

fn registry_text(id: &str, l: crate::i18n::Lang) -> &'static str {
    crate::registry::find(id).map(|c| c.text(l)).unwrap()
}

#[test]
fn shift_question_mark_lists_the_shortcuts() {
    for (lang, title, undo) in [
        ("en-US", "Keyboard shortcuts", "Undo"),
        ("fr-FR", "Raccourcis clavier", "Annuler"),
    ] {
        let mut h = app(lang, Tab::Report, RTab::Home);
        h.event(Event::Text("?".into()));
        press(&mut h, Key::Questionmark, Modifiers::SHIFT);
        steps(&mut h, 4);
        assert!(
            h.state().keyboard.shortcuts.is_some(),
            "{lang}: Shift+? opens the list"
        );
        h.get_by_label(title);
        h.get_by_label("Ctrl+Z");
        // the row's text (the ribbon's Undo button has it too)
        assert!(h.get_all_by_label(undo).count() >= 2, "{lang}: the row of Ctrl+Z");
        // the search box has the focus: typing filters
        assert!(in_dialog(&h), "the focus is in the list");
        h.event(Event::Text("F10".into()));
        steps(&mut h, 3);
        assert!(h.query_by_label("Ctrl+Z").is_none(), "filtered out");
        h.get_by_label("Shift+F10");
        press(&mut h, Key::Escape, Modifiers::NONE);
        steps(&mut h, 2);
        assert!(h.state().keyboard.shortcuts.is_none(), "Escape closes it");
    }
}

/// An open popup menu (a `Foreground` layer with a focused item).
fn menu_focused(h: &Harness<'_, App>) -> bool {
    keyboard::focused(&h.ctx).is_some_and(|(l, _)| l.order == Order::Foreground)
}

#[test]
fn shift_f10_opens_the_focused_items_context_menu() {
    let mut h = app("en-US", Tab::Report, RTab::Home);
    // a Data pane table: its right-click menu
    let sales = rect_of(&h, Role::Button, "Sales");
    f6_to(&mut h, Section::Pane);
    for _ in 0..6 {
        let in_pane = keyboard::sections(h.state()).into_iter().any(|(s, r)| {
            s == Section::Pane
                && r.contains(sales.center())
                && focused(&h).is_some_and(|f| r.contains(f.center()))
        });
        if in_pane {
            break;
        }
        ctrl_f6(&mut h, false);
    }
    tab_to(&mut h, sales);
    assert!(h.query_by_label("New quick measure").is_none());
    press(&mut h, Key::F10, Modifiers::SHIFT);
    steps(&mut h, 4);
    h.get_by_label("New quick measure");
    assert!(
        menu_focused(&h),
        "the menu's first item has the focus: {:?}",
        keyboard::focused(&h.ctx)
    );
    press(&mut h, Key::Escape, Modifiers::NONE);
    steps(&mut h, 3);
    assert!(
        h.query_by_label("New quick measure").is_none(),
        "Escape closes it"
    );
    assert!(
        focused(&h).is_some_and(|r| same(r, sales)),
        "the focus is back on the row: {:?}",
        focused(&h)
    );
    // a selected visual on the canvas: its menu, as a right click on it
    let visual = h
        .state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/pages/0/visuals/0/name"))
        .and_then(serde_json::Value::as_str)
        .unwrap()
        .to_string();
    h.state_mut().cmd.visual = Some(crate::ribbon::VisualSel {
        page: h.state().page_names()[0].0.clone(),
        visual: visual.clone(),
    });
    f6_to(&mut h, Section::Main);
    press(&mut h, Key::F10, Modifiers::SHIFT);
    steps(&mut h, 4);
    h.get_by_label(registry_text("visual.remove", crate::i18n::Lang::En));
    assert_eq!(
        h.state().cmd.visual.as_ref().map(|v| v.visual.as_str()),
        Some(visual.as_str()),
        "the selection stays"
    );
}

/// The interactive nodes without an accessible name: role and rectangle.
fn unlabelled(h: &Harness<'_, App>) -> Vec<String> {
    let roles = [
        Role::Button,
        Role::CheckBox,
        Role::TextInput,
        Role::MultilineTextInput,
        Role::ComboBox,
        Role::RadioButton,
        Role::Slider,
        Role::SpinButton,
        Role::Tab,
        Role::Link,
        Role::MenuItem,
        Role::Switch,
    ];
    h.root()
        .children_recursive()
        .filter(|n| roles.contains(&n.accesskit_node().role()))
        .filter(|n| n.accesskit_node().label().is_none_or(|l| l.trim().is_empty()))
        .map(|n| format!("{:?} at {:?}", n.accesskit_node().role(), n.rect()))
        .collect()
}

fn assert_labelled(h: &Harness<'_, App>, what: &str) {
    let bad = unlabelled(h);
    assert!(
        bad.is_empty(),
        "{what}: {} unlabelled controls: {bad:#?}",
        bad.len()
    );
}

#[test]
fn every_control_has_an_accessible_name() {
    for lang in ["en-US", "fr-FR"] {
        let mut h = app(lang, Tab::Report, RTab::Home);
        assert_labelled(&h, "Report view");
        // a visual selected: the Build pane's wells and the Format pane
        let visual = h
            .state()
            .loaded
            .as_ref()
            .and_then(|l| l.snap.pointer("/report/pages/0/visuals/0/name"))
            .and_then(serde_json::Value::as_str)
            .expect("the chart")
            .to_string();
        h.state_mut().cmd.visual = Some(crate::ribbon::VisualSel {
            page: h.state().page_names()[0].0.clone(),
            visual,
        });
        steps(&mut h, 6);
        assert_labelled(&h, "Report view, a visual selected");
        for (view, rtab) in [
            (Tab::Data, RTab::Data),
            (Tab::Model, RTab::Model),
            (Tab::Query, RTab::Home),
        ] {
            h.state_mut().show_tab(view);
            h.state_mut().ribbon_tab = rtab;
            steps(&mut h, 6);
            assert_labelled(&h, &format!("{lang} {view:?} view"));
        }
        h.state_mut().show_tab(Tab::Model);
        h.state_mut().ribbon_tab = RTab::Model;
        steps(&mut h, 4);
        for (name, open) in [
            (
                "Manage relationships",
                crate::relationships_dialog::open as fn(&mut App),
            ),
            ("Manage security roles", |app: &mut App| {
                crate::roles_dialog::open(app, None)
            }),
            ("Customize theme", crate::theme_dialog::open),
        ] {
            open(h.state_mut());
            steps(&mut h, 8);
            assert_labelled(&h, &format!("{lang} {name}"));
            press(&mut h, Key::Escape, Modifiers::NONE);
            steps(&mut h, 2);
        }
    }
}

/// [`session`] with three visuals whose tab order is not their reading order: the titled
/// column chart top left, a card top right, a table under them, visited table, chart, card.
fn canvas_session() -> Session {
    let mut s = session();
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    for (ty, x, y, field) in [
        ("card", 500, 20, "[Total]"),
        ("tableEx", 20, 400, "Sales[Category]"),
    ] {
        s.execute(
            "visual.add",
            &json!({"page": page, "visual_type": ty, "x": x, "y": y, "width": 300, "height": 200,
                    "fields": [{"role": "Values", "field": field}]}),
        )
        .unwrap();
    }
    let names: Vec<String> = s.report.as_ref().unwrap().pages[0]
        .visuals
        .iter()
        .map(|v| v.name.clone())
        .collect();
    let [chart, card, table] = names.as_slice() else {
        panic!("three visuals: {names:?}");
    };
    s.execute(
        "visual.set_title",
        &json!({"page": page, "visual": chart, "title": "Sales overview"}),
    )
    .unwrap();
    s.execute(
        "visual.tab_order",
        &json!({"page": page, "visuals": [table, chart, card]}),
    )
    .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

/// The node that has the focus in the accessibility tree: its role, its name, whether it is
/// selected.
fn focused_node(h: &Harness<'_, App>) -> Option<(Role, String, Option<bool>)> {
    h.root()
        .children_recursive()
        .find(|n| n.accesskit_node().is_focused())
        .map(|n| {
            let a = n.accesskit_node();
            (a.role(), a.label().unwrap_or_default(), a.is_selected())
        })
}

/// The visuals of the page shown, in the snapshot's order: (name, x).
fn visuals(h: &Harness<'_, App>) -> Vec<(String, f64)> {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/pages/0/visuals"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| Some((v["name"].as_str()?.to_string(), v["x"].as_f64()?)))
        .collect()
}

/// Tab until a canvas visual has the focus; its node.
fn tab_to_visual(h: &mut Harness<'_, App>) -> (Role, String, Option<bool>) {
    for _ in 0..40 {
        if let Some(n) = focused_node(h).filter(|n| n.0 == Role::Figure) {
            return n;
        }
        tab(h, false);
    }
    panic!("Tab never reached a visual; focus on {:?}", focused_node(h));
}

/// The status bar's live region: its text.
fn live_text(h: &Harness<'_, App>) -> String {
    let n = h
        .root()
        .children_recursive()
        .find(|n| n.accesskit_node().role() == Role::Status)
        .expect("the status bar's live region");
    assert_eq!(n.accesskit_node().live(), egui::accesskit::Live::Polite);
    n.accesskit_node().label().unwrap_or_default()
}

/// The selected visual's name.
fn selected(h: &Harness<'_, App>) -> Option<String> {
    h.state().cmd.visual.as_ref().map(|v| v.visual.clone())
}

#[test]
fn tab_walks_the_canvas_visuals_in_tab_order_and_escape_returns_to_the_page() {
    let mut h = app_with("en-US", Tab::Report, RTab::Home, canvas_session);
    let names: Vec<String> = visuals(&h).into_iter().map(|(n, _)| n).collect();
    let [chart, card, table] = names.as_slice() else {
        panic!("three visuals: {names:?}");
    };
    f6_to(&mut h, Section::Main);
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut node = tab_to_visual(&mut h);
    for _ in 0..3 {
        assert_eq!(node.0, Role::Figure, "{node:?}");
        assert_eq!(node.2, Some(true), "the focused visual is selected: {node:?}");
        let sel = selected(&h).expect("a selection");
        // egui's focus itself is on the visual's widget
        assert_eq!(
            h.ctx.memory(|m| m.focused()),
            Some(crate::visual_focus::id(&sel)),
            "the focus is on {sel}"
        );
        seen.push((sel, node.1.clone()));
        tab(&mut h, false);
        node = focused_node(&h).expect("a focused node");
    }
    let order: Vec<&str> = seen.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(order, [table.as_str(), chart.as_str(), card.as_str()], "{seen:?}");
    // the chart's name starts with its title
    assert!(seen[1].1.starts_with("Sales overview"), "{seen:?}");
    // Shift+Tab goes back to the card
    tab(&mut h, true);
    assert_eq!(focused_node(&h).map(|n| n.1), Some(seen[2].1.clone()));
    // Escape: back to the page, nothing selected
    press(&mut h, Key::Escape, Modifiers::NONE);
    steps(&mut h, 2);
    let page = focused_node(&h).expect("the page has the focus");
    assert_eq!(page.0, Role::Group, "{page:?}");
    assert!(page.1.starts_with("Report page"), "{page:?}");
    assert!(selected(&h).is_none());
}

#[test]
fn a_focused_visual_moves_with_the_arrows_and_opens_its_menu_from_the_menu_key() {
    let mut h = app_with("en-US", Tab::Report, RTab::Home, canvas_session);
    f6_to(&mut h, Section::Main);
    let node = tab_to_visual(&mut h);
    let name = selected(&h).expect("a selection");
    let x = |h: &Harness<'_, App>| visuals(h).into_iter().find(|(n, _)| *n == name).map(|v| v.1);
    let x0 = x(&h).unwrap();
    // the arrows move it (one `visual.move`), the focus stays on it
    press(&mut h, Key::ArrowRight, Modifiers::SHIFT);
    steps(&mut h, 6);
    let x1 = x(&h).unwrap();
    assert!(x1 > x0, "moved right: {x0} -> {x1}");
    assert_eq!(focused_node(&h).map(|n| n.1), Some(node.1.clone()));
    // the Menu key opens its menu, first item focused; Escape gives the focus back to it
    press(&mut h, Key::Menu, Modifiers::NONE);
    steps(&mut h, 4);
    h.get_by_label(registry_text("visual.remove", crate::i18n::Lang::En));
    assert!(menu_focused(&h), "{:?}", keyboard::focused(&h.ctx));
    press(&mut h, Key::Escape, Modifiers::NONE);
    steps(&mut h, 4);
    assert_eq!(
        focused_node(&h).map(|n| n.1),
        Some(node.1.clone()),
        "the focus is back on the visual"
    );
    // Delete removes the focused visual; the focus goes to the page
    press(&mut h, Key::Delete, Modifiers::NONE);
    steps(&mut h, 6);
    assert!(x(&h).is_none(), "removed");
    let page = focused_node(&h);
    assert!(
        page.as_ref().is_some_and(|n| n.1.starts_with("Report page")),
        "{page:?}"
    );
}

/// Ctrl+F6 until the focus is in the side pane that holds `inside`.
fn f6_to_pane_holding(h: &mut Harness<'_, App>, inside: Rect) {
    for _ in 0..12 {
        let there = keyboard::sections(h.state()).into_iter().any(|(s, r)| {
            s == Section::Pane
                && r.contains(inside.center())
                && focused(h).is_some_and(|f| r.contains(f.center()))
        });
        if there {
            return;
        }
        ctrl_f6(h, false);
    }
    panic!("Ctrl+F6 never reached the pane at {inside:?}");
}

#[test]
fn the_menu_key_opens_the_focused_items_context_menu() {
    let mut h = app("en-US", Tab::Report, RTab::Home);
    let sales = rect_of(&h, Role::Button, "Sales");
    f6_to_pane_holding(&mut h, sales);
    tab_to(&mut h, sales);
    assert!(h.query_by_label("New quick measure").is_none());
    press(&mut h, Key::Menu, Modifiers::NONE);
    steps(&mut h, 4);
    h.get_by_label("New quick measure");
    assert!(menu_focused(&h), "{:?}", keyboard::focused(&h.ctx));
    press(&mut h, Key::Escape, Modifiers::NONE);
    steps(&mut h, 3);
    assert!(focused(&h).is_some_and(|r| same(r, sales)));
}

#[test]
fn the_live_region_names_the_section_ctrl_f6_moves_to() {
    for (lang, data) in [("en-US", "Focus: Data pane"), ("fr-FR", "Focus : Volet Données")] {
        let mut h = app(lang, Tab::Report, RTab::Home);
        let sales = rect_of(&h, Role::Button, "Sales");
        f6_to_pane_holding(&mut h, sales);
        assert_eq!(live_text(&h), data, "{lang}");
    }
}

/// The Data pane rows named `label` in the accessibility tree.
fn rows_named(h: &Harness<'_, App>, label: &str) -> usize {
    h.query_all_by_label(label).count()
}

#[test]
fn alt_shift_1_and_9_collapse_and_expand_every_table() {
    for (lang, view, rtab, collapsed, expanded) in [
        (
            "en-US",
            Tab::Report,
            RTab::Home,
            "All tables collapsed",
            "All tables expanded",
        ),
        (
            "fr-FR",
            Tab::Report,
            RTab::Home,
            "Toutes les tables réduites",
            "Toutes les tables développées",
        ),
        (
            "en-US",
            Tab::Model,
            RTab::Model,
            "All tables collapsed",
            "All tables expanded",
        ),
    ] {
        let mut h = app(lang, view, rtab);
        let alt_shift = |h: &mut Harness<'_, App>, k: Key| {
            press(h, k, Modifiers::ALT | Modifiers::SHIFT);
            steps(h, 3);
        };
        let before = rows_named(&h, "Amount");
        alt_shift(&mut h, Key::Num9);
        let open = rows_named(&h, "Amount");
        assert!(
            open > before,
            "{lang} {view:?}: Sales's columns shown ({before} -> {open})"
        );
        assert_eq!(live_text(&h), expanded, "{lang} {view:?}");
        alt_shift(&mut h, Key::Num1);
        let shut = rows_named(&h, "Amount");
        assert!(
            shut < open,
            "{lang} {view:?}: Sales's columns hidden ({open} -> {shut})"
        );
        assert_eq!(live_text(&h), collapsed, "{lang} {view:?}");
    }
}
