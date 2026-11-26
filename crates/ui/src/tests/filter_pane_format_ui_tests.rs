//! The Format tab's page cards (`filter_pane_ui.rs`) on an engine session: with nothing
//! selected, Filter pane and Filter cards format the page's Filters pane through
//! `page.format_filter_pane` (a switch, a swatch, the Available / Applied state, Reset to
//! default), and the Filters pane draws with the snapshot's resolved style, for a reader too.

use std::path::Path;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::filters_ui_tests::{last_sent, page, shop_session};
use super::harness;
use crate::App;
use crate::shell::Caps;

fn open_in(lang: &str) -> Harness<'static, App> {
    let mut app = App::new(lang, engine_shell(Caps::DESKTOP, shop_session)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1600.0, 900.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().placed.is_some() {
            break;
        }
    }
    h.state_mut().prefs.panes.build = true;
    h.state_mut().prefs.panes.filters = true;
    h.state_mut().panes.format.tab = crate::format_pane::Tab::Format;
    steps(&mut h, 3);
    h
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
    steps(h, 4);
}

/// Whether the last frame painted a pane-sized rectangle (not a swatch) filled with `hex`.
fn painted_fill(h: &Harness<'_, App>, hex: &str) -> bool {
    let want = crate::form::color(hex).unwrap();
    h.output().shapes.iter().any(|c| match &c.shape {
        egui::epaint::Shape::Rect(r) => r.fill == want && r.rect.width() > 100.0 && r.rect.height() > 300.0,
        _ => false,
    })
}

fn format(h: &Harness<'_, App>) -> Value {
    page(h).get("filter_pane_format").cloned().unwrap_or(Value::Null)
}

#[test]
fn the_format_tab_formats_the_filters_pane_with_nothing_selected() {
    let mut h = open_in("en-US");
    h.get_by_label("Format page");
    // Filter pane: the border between the report and the pane
    click(&mut h, "Filter pane");
    click(&mut h, "Filter pane Border");
    let (id, p) = last_sent(&h);
    assert_eq!(id, "page.format_filter_pane");
    assert_eq!(p["card"], "pane");
    assert_eq!(p["border"], true);
    assert_eq!(format(&h)["pane"]["border"], true, "{}", format(&h));
    // a swatch of the background colour
    click(&mut h, "Filter pane Background colour");
    click(&mut h, "Filter pane Background colour #252423");
    assert_eq!(last_sent(&h).1["background_color"], "#252423");
    assert_eq!(format(&h)["pane"]["background_color"], "#252423");
    // the Filters pane draws with it
    let style = crate::filter_pane_ui::active_style(h.state());
    assert_eq!(style.pane.border, Some(true));
    assert_eq!(style.pane.background, crate::form::color("#252423"));
    assert!(
        painted_fill(&h, "#252423"),
        "the Filters pane is filled with its background"
    );
    // Filter cards (the pane's card folded): the Applied state
    click(&mut h, "Filter pane");
    click(&mut h, "Filter cards");
    click(&mut h, "Filter cards Apply settings to");
    click(&mut h, "Applied");
    click(&mut h, "Filter cards Border");
    let (_, p) = last_sent(&h);
    assert_eq!(
        (p["card"].clone(), p["border"].clone()),
        (json!("applied"), json!(true))
    );
    assert_eq!(format(&h)["applied"]["border"], true);
    assert!(format(&h)["available"].get("border").is_none());
    // Reset to default takes the state's properties away
    click(&mut h, "Reset to default Filter cards");
    assert_eq!(
        last_sent(&h).1,
        json!({"page": page(&h)["name"], "card": "applied", "reset": true})
    );
    assert!(format(&h)["applied"].get("border").is_none(), "{}", format(&h));
    // a reader's pane draws the same style
    h.state_mut().access = Some("reader".into());
    steps(&mut h, 3);
    h.get_by_label("Collapse the Filters pane");
    assert_eq!(
        crate::filter_pane_ui::active_style(h.state()).pane.border,
        Some(true)
    );
}

#[test]
fn the_page_cards_speak_french() {
    let mut h = open_in("fr-FR");
    h.get_by_label("Mettre en forme la page");
    click(&mut h, "Volet Filtre");
    h.get_by_label("Volet Filtre Couleur de police et d'icône");
    h.get_by_label("Volet Filtre Taille du texte du titre");
    click(&mut h, "Volet Filtre Bordure");
    assert_eq!(format(&h)["pane"]["border"], true);
    click(&mut h, "Cartes de filtre");
    h.get_by_label("Cartes de filtre Appliquer les paramètres à");
    h.get_by_label("Cartes de filtre Taille du texte");
    h.get_by_label("Rétablir les valeurs par défaut Cartes de filtre");
}
