//! View › Themes (the gallery of BIcraft's built-in themes, Customize current theme, Browse
//! for themes) and high contrast: the app's high-contrast theme and the report pages drawn in
//! high contrast, a view option that leaves the file alone.

use std::path::Path;

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use crate::App;
use crate::registry::Tab as RTab;
use crate::shell::Caps;
use crate::theme::{Brightness, Tokens};

fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [("model.new", json!({"name": "Shop"})), ("report.new", json!({}))] {
        s.execute(id, &p).unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute(
        "visual.add",
        &json!({"page": page, "visual_type": "card", "x": 40, "y": 40, "width": 300, "height": 120,
                "force": true}),
    )
    .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

pub(super) fn themed_app(lang: &str, shell: crate::shell::Shell) -> Harness<'static, App> {
    let mut app = App::new(lang, shell).with_sync_render();
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

fn app(lang: &str) -> Harness<'static, App> {
    themed_app(lang, engine_shell(Caps::DESKTOP, session))
}

fn show_ribbon(h: &mut Harness<'_, App>, tab: RTab) {
    h.state_mut().ribbon_tab = tab;
    steps(h, 3);
}

fn theme_name(h: &Harness<'_, App>) -> Value {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/theme/name").cloned())
        .unwrap_or(Value::Null)
}

/// Whether the control labelled `name` is marked (selected or toggled on).
fn marked(h: &Harness<'_, App>, name: &str) -> bool {
    let n = h.get_by_label(name).accesskit_node();
    n.is_selected() == Some(true) || n.toggled() == Some(egui::accesskit::Toggled::True)
}

/// Waits for the document's next snapshot to satisfy `ok`.
fn until(h: &mut Harness<'_, App>, ok: impl Fn(&Harness<'_, App>) -> bool) {
    for _ in 0..300 {
        if ok(h) {
            return;
        }
        h.step();
    }
    panic!("the condition never held");
}

/// The gallery lists the eight built-in themes; a tile installs its theme (theme.set
/// {builtin}), and the report's theme is the one ringed when the gallery opens again.
#[test]
fn the_gallery_applies_a_builtin_theme() {
    let mut h = app("en-US");
    show_ribbon(&mut h, RTab::View);
    h.get_by_label("Themes").click();
    steps(&mut h, 2);
    for name in [
        "BIcraft",
        "Harbor",
        "Orchard",
        "Ember",
        "Slate",
        "Clear sight",
        "Dusk",
        "High contrast",
    ] {
        h.get_by_label(name);
    }
    h.get_by_label("Browse for themes…");
    h.get_by_label("Customize current theme…");
    h.get_by_label("Dusk").click();
    until(&mut h, |h| theme_name(h) == "BIcraft Dusk");
    let bg = h
        .state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/theme/background").cloned());
    assert_eq!(bg, Some(json!("#1E2130")));
    steps(&mut h, 2);
    h.get_by_label("Themes").click();
    steps(&mut h, 2);
    assert!(marked(&h, "Dusk"));
    assert!(!marked(&h, "Harbor"));
}

/// French names, and Customize current theme opens the Customize theme dialog on the theme's
/// name and colours; Cancel closes it and sends nothing.
#[test]
fn customize_starts_from_the_current_theme() {
    let mut h = app("fr-FR");
    show_ribbon(&mut h, RTab::View);
    h.get_by_label("Thèmes").click();
    steps(&mut h, 2);
    h.get_by_label("Crépuscule");
    h.get_by_label("Port").click();
    until(&mut h, |h| theme_name(h) == "BIcraft Harbor");
    steps(&mut h, 2);
    h.get_by_label("Thèmes").click();
    steps(&mut h, 2);
    h.get_by_label("Personnaliser le thème actuel…").click();
    steps(&mut h, 3);
    let d = h.state().cmd.theme.clone().expect("the dialog opens");
    assert_eq!(d.name, "BIcraft Harbor");
    assert_eq!(d.colors.first().map(String::as_str), Some("#0B5CAD"));
    h.get_by_label("Personnaliser le thème");
    for section in ["Texte", "Visuels"] {
        h.get_by_label(section);
    }
    // the colour's text box and its picker
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Couleur 1");
    h.get_by_label("Couleur 1 (#0B5CAD)");
    let sent = h.state().sent.len();
    h.get_by_label("Annuler").click();
    steps(&mut h, 3);
    assert!(h.state().cmd.theme.is_none(), "Cancel closes the dialog");
    assert_eq!(h.state().sent.len(), sent, "Cancel sends nothing");
}

/// The control labelled `name` (the rightmost: the dialog's tab rather than its section, the
/// dialog's control rather than the ribbon's).
fn dialog_node<'a>(h: &'a Harness<'_, App>, name: &'a str) -> egui_kittest::Node<'a> {
    h.get_all_by_label(name)
        .max_by(|a, b| a.rect().min.x.total_cmp(&b.rect().min.x))
        .unwrap_or_else(|| panic!("no `{name}`"))
}

/// Types `text` in the text box labelled `name`, replacing what it holds.
fn retype(h: &mut Harness<'_, App>, name: &str, text: &str) {
    dialog_node(h, name).click();
    steps(h, 2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    steps(h, 2);
}

/// A pointer click at `at` (hover, press, release).
fn pointer_click(h: &mut Harness<'_, App>, at: egui::Pos2) {
    h.hover_at(at);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
    }
    steps(h, 2);
}

fn last_customize(h: &Harness<'_, App>) -> Option<Value> {
    h.state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == "theme.customize")
        .map(|c| c.params.clone())
}

/// The dialog's gestures: the name and a theme colour typed, a section and its tab picked, a
/// font family from the list, the visuals' border switched on, an applied filter card's colour;
/// Apply sends them as one theme.customize, the report takes them, and one undo takes them all
/// back.
#[test]
fn customize_applies_every_section_in_one_undo_step() {
    let mut h = app("en-US");
    show_ribbon(&mut h, RTab::View);
    // the ribbon's Customize current theme button opens the dialog too
    dialog_node(&h, "Customize current theme").click();
    steps(&mut h, 3);
    assert!(h.state().cmd.theme.is_some(), "the dialog opens");
    let before = theme_name(&h);
    retype(&mut h, "Name", "Harbour lights");
    retype(&mut h, "Color 2", "#123456");
    retype(&mut h, "Name and colors › Sentiment colors › Positive", "#1AAB40");
    // Text › Title: the font family
    dialog_node(&h, "Text").click();
    steps(&mut h, 2);
    dialog_node(&h, "Title").click();
    steps(&mut h, 2);
    // the combo box and its list's item, clicked with the pointer
    let at = dialog_node(&h, "Title › Font family").rect().center();
    pointer_click(&mut h, at);
    let at = dialog_node(&h, "Arial").rect().center();
    pointer_click(&mut h, at);
    // Visuals › Border: on
    dialog_node(&h, "Visuals").click();
    steps(&mut h, 2);
    dialog_node(&h, "Border").click();
    steps(&mut h, 2);
    dialog_node(&h, "Border › Border").click();
    steps(&mut h, 2);
    // Filter pane › Filter cards: the applied cards' background
    dialog_node(&h, "Filter pane").click();
    steps(&mut h, 2);
    dialog_node(&h, "Filter cards").click();
    steps(&mut h, 2);
    retype(
        &mut h,
        "Filter cards › Applied filter cards › Background color",
        "#E6E6E6",
    );
    dialog_node(&h, "Apply").click();
    steps(&mut h, 3);
    assert!(h.state().cmd.theme.is_none(), "Apply closes the dialog");
    let p = last_customize(&h).expect("Apply sends theme.customize");
    assert_eq!(p["name"], "Harbour lights");
    assert_eq!(p["data_colors"][1], "#123456");
    assert_eq!(p["good"], "#1AAB40");
    assert_eq!(p["text_classes"], json!({"title": {"font_face": "Arial"}}));
    assert_eq!(
        p["styles"],
        json!([
            {"visual_type": "*", "object": "border", "property": "show", "value": true},
            {"visual_type": "page", "object": "filterCard", "property": "backgroundColor",
             "value": {"color": "#E6E6E6"}, "state": "Applied"}
        ])
    );
    until(&mut h, |h| theme_name(h) == "Harbour lights");
    let t = h
        .state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/theme").cloned())
        .unwrap();
    assert_eq!(t["good"], "#1AAB40");
    assert_eq!(t["text_classes"]["title"]["font_face"], "Arial");
    assert_eq!(
        t["styles"]["page"]["filterCard.Applied"]["backgroundColor"],
        "#E6E6E6"
    );
    // the palette's run (no settings) opens the dialog again, on what was applied
    crate::ribbon::invoke(h.state_mut(), "theme.customize", json!({}));
    steps(&mut h, 3);
    let d = h.state().cmd.theme.clone().expect("the dialog opens");
    assert_eq!(d.name, "Harbour lights");
    assert_eq!(d.values["vis.border.show"], true);
    h.state_mut().cmd.theme = None;
    // one undo step
    crate::ribbon::invoke(h.state_mut(), "edit.undo", json!({}));
    until(&mut h, |h| theme_name(h) == before);
}

/// theme.set without a theme (the palette) opens its form instead of failing.
#[test]
fn theme_set_without_a_theme_opens_its_form() {
    let mut h = app("en-US");
    crate::ribbon::invoke(h.state_mut(), "theme.set", json!({}));
    steps(&mut h, 2);
    assert_eq!(h.state().form.as_ref().map(|f| f.id), Some("theme.set"));
}

/// Home › High contrast switches the app to the high-contrast tokens and the pages to the
/// high-contrast palette; View › High-contrast colours turns the pages back, Light turns both
/// off, and the file never changes.
#[test]
fn high_contrast_in_the_app_and_for_the_report() {
    let mut h = app("en-US");
    show_ribbon(&mut h, RTab::Home);
    h.get_by_label("High contrast").click();
    steps(&mut h, 3);
    assert_eq!(h.state().prefs.theme, Brightness::HighContrast);
    assert_eq!(Tokens::get(&h.ctx), Tokens::high_contrast());
    assert!(h.state().prefs.report_contrast);
    until(&mut h, |h| {
        h.state().shown.as_ref().is_some_and(|(k, _)| k.high_contrast)
    });
    // the page is drawn black
    let page = h.state().shown.clone().unwrap().1.unwrap();
    assert_eq!(&page.rgba[..3], &[0, 0, 0]);
    show_ribbon(&mut h, RTab::View);
    h.get_by_label("High-contrast colours").click();
    until(&mut h, |h| {
        h.state().shown.as_ref().is_some_and(|(k, _)| !k.high_contrast)
    });
    assert_eq!(h.state().prefs.theme, Brightness::HighContrast);
    let page = h.state().shown.clone().unwrap().1.unwrap();
    assert_eq!(&page.rgba[..3], &[255, 255, 255]);
    h.get_by_label("High-contrast colours").click();
    steps(&mut h, 3);
    assert!(h.state().prefs.report_contrast);
    show_ribbon(&mut h, RTab::Home);
    h.get_by_label("Light").click();
    steps(&mut h, 3);
    assert_eq!(Tokens::get(&h.ctx), Tokens::light());
    assert!(!h.state().prefs.report_contrast);
    // a view option: the document is not dirty
    let dirty = h
        .state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/dirty").cloned());
    assert_eq!(
        dirty,
        Some(json!({"model": false, "report": false, "queries": false}))
    );
}

/// A browser whose system asks for high contrast starts in it, until a theme is chosen.
#[test]
fn the_system_high_contrast_is_followed_until_a_theme_is_chosen() {
    let mut shell = engine_shell(Caps::BROWSER, session);
    shell.system_high_contrast = true;
    let h = themed_app("en-US", shell);
    assert_eq!(h.state().prefs.theme, Brightness::HighContrast);
    assert!(h.state().prefs.report_contrast);
    assert_eq!(Tokens::get(&h.ctx), Tokens::high_contrast());

    let store = super::MemStore::default();
    *store.0.borrow_mut() = json!({"theme": "dark"});
    let mut shell = engine_shell(Caps::BROWSER, session);
    shell.system_high_contrast = true;
    shell.prefs = Some(Box::new(store));
    let h = themed_app("en-US", shell);
    assert_eq!(h.state().prefs.theme, Brightness::Dark);
    assert!(!h.state().prefs.report_contrast);
}
