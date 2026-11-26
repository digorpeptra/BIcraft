//! The list of keyboard shortcuts (Shift+?, as Desktop's): the registry's declared shortcuts
//! (each command's chord, the browser's where a browser keeps the registry's), then the window's
//! own navigation keys, searchable by key or by text, in English and French. A dialog: Escape or
//! Close closes it and the focus goes back where it was. Pure view state.

use egui::{Context, Key, RichText};

use crate::App;
use crate::i18n::Lang;
use crate::registry;
use crate::theme::Tokens;

/// The window's own keys (crate::keyboard, crate::key_tips, the palette): keys, English, French.
const NAVIGATION: &[(&str, &str, &str)] = &[
    ("Ctrl+Shift+P", "Command palette", "Palette de commandes"),
    ("Alt, F10", "Key tips on the ribbon", "Touches d'accès du ruban"),
    ("Shift+?", "This list of shortcuts", "Cette liste de raccourcis"),
    (
        "Shift+F10",
        "Context menu of the focused item",
        "Menu contextuel de l'élément actif",
    ),
    (
        "Ctrl+F6",
        "Next section of the window",
        "Section suivante de la fenêtre",
    ),
    (
        "Ctrl+Shift+F6",
        "Previous section of the window",
        "Section précédente de la fenêtre",
    ),
    ("F6", "Selection pane", "Volet Sélection"),
    (
        "Tab, Shift+Tab",
        "Next or previous control of the section",
        "Contrôle suivant ou précédent de la section",
    ),
    (
        "Right, Left",
        "Unroll or roll up a tree row",
        "Développer ou réduire une ligne d'arborescence",
    ),
    (
        "Enter, Space",
        "Activate the focused control",
        "Activer le contrôle actif",
    ),
    (
        "Escape",
        "Close the dialog or the menu",
        "Fermer la boîte de dialogue ou le menu",
    ),
    (
        "Ctrl+Right",
        "Enter the data points of the selected visual (arrows move, Enter or Space selects, Ctrl+Space adds, Escape clears)",
        "Entrer dans les points de données du visuel sélectionné (flèches pour se déplacer, Entrée ou Espace pour sélectionner, Ctrl+Espace pour ajouter, Échap pour effacer)",
    ),
    ("Alt+F", "File view", "Mode Fichier"),
    ("Ctrl+Tab", "Next open file", "Fichier ouvert suivant"),
];

/// A row of the list: its keys, what they do.
pub type Row = (String, String);

/// The open list: its search text.
#[derive(Debug, Default)]
pub struct List {
    pub query: String,
}

/// The rows the list shows for `query`: (keys, what they do), the registry's first. A row matches
/// when its keys or its text hold the query (case ignored).
pub fn rows(l: Lang, browser: bool, query: &str) -> (Vec<Row>, Vec<Row>) {
    let q = query.trim().to_lowercase();
    let keep =
        |k: &str, t: &str| q.is_empty() || k.to_lowercase().contains(&q) || t.to_lowercase().contains(&q);
    let commands = registry::COMMANDS
        .iter()
        .filter_map(|c| Some((c.shortcut_for(browser)?.to_string(), c.text(l).to_string())))
        .filter(|(k, t)| keep(k, t))
        .collect();
    let navigation = NAVIGATION
        .iter()
        .map(|(k, en, fr)| ((*k).to_string(), l.t(en, fr).to_string()))
        .filter(|(k, t)| keep(k, t))
        .collect();
    (commands, navigation)
}

/// Shift+? (a `?` typed outside a text field, no dialog open) opens the list.
pub fn keys(app: &mut App, ctx: &Context) {
    if app.keyboard.shortcuts.is_some() || app.keyboard.in_dialog() || ctx.egui_wants_keyboard_input() {
        return;
    }
    let hit = ctx.input(|i| {
        i.key_pressed(Key::Questionmark)
            || i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Text(t) if t == "?"))
    });
    if hit {
        ctx.input_mut(|i| {
            i.events
                .retain(|e| !matches!(e, egui::Event::Text(t) if t == "?"));
        });
        app.keyboard.shortcuts = Some(List::default());
        app.keyboard.keyboard = true;
    }
}

/// Draws the list while it is open.
pub fn show(app: &mut App, ctx: &Context) {
    let Some(mut list) = app.keyboard.shortcuts.take() else {
        return;
    };
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let (commands, navigation) = rows(l, app.caps().reserved_keys, &list.query);
    let mut open = true;
    let mut close = false;
    egui::Window::new(l.t("Keyboard shortcuts", "Raccourcis clavier"))
        .id(egui::Id::unique("bicraft-shortcuts"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(520.0);
            let before = list.query.clone();
            let name = l.t("Search shortcuts", "Rechercher un raccourci");
            let edit = ui.add(
                egui::TextEdit::singleline(&mut list.query)
                    .hint_text(name)
                    .desired_width(f32::INFINITY),
            );
            edit.widget_info(|| egui::WidgetInfo::text_edit(true, &before, &list.query, name));
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                for (head, rows, grid) in [
                    (
                        l.t("Commands", "Commandes"),
                        &commands,
                        "bicraft-shortcuts-commands",
                    ),
                    (
                        l.t("Navigation", "Navigation"),
                        &navigation,
                        "bicraft-shortcuts-navigation",
                    ),
                ] {
                    if rows.is_empty() {
                        continue;
                    }
                    ui.label(RichText::new(head).font(crate::theme::bold(13.0)));
                    egui::Grid::new(grid)
                        .num_columns(2)
                        .spacing([16.0, 4.0])
                        .show(ui, |ui| {
                            for (k, what) in rows {
                                ui.label(RichText::new(k).monospace().color(t.text));
                                ui.label(RichText::new(what).color(t.text));
                                ui.end_row();
                            }
                        });
                    ui.add_space(6.0);
                }
                if commands.is_empty() && navigation.is_empty() {
                    ui.label(
                        RichText::new(l.t("No shortcut matches", "Aucun raccourci ne correspond"))
                            .color(t.text_dim),
                    );
                }
            });
            ui.add_space(4.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(l.t("Close", "Fermer")).clicked() {
                    close = true;
                }
            });
        });
    if !open || close || ctx.input(|i| i.key_pressed(Key::Escape)) {
        return;
    }
    app.keyboard.shortcuts = Some(list);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_holds_the_registrys_shortcuts_and_filters() {
        let (cmds, nav) = rows(Lang::En, false, "");
        let declared = registry::COMMANDS.iter().filter(|c| c.shortcut.is_some()).count();
        assert_eq!(cmds.len(), declared);
        assert!(cmds.iter().any(|(k, t)| k == "Ctrl+Z" && t == "Undo"), "{cmds:?}");
        assert!(nav.iter().any(|(k, _)| k == "Shift+F10"));
        // the browser's chord
        let (cmds, _) = rows(Lang::En, true, "alt+n");
        assert_eq!(cmds.len(), 1, "{cmds:?}");
        // French text, searched by text
        let (cmds, nav) = rows(Lang::Fr, false, "annuler");
        assert!(cmds.iter().any(|(k, _)| k == "Ctrl+Z"), "{cmds:?}");
        assert!(nav.is_empty());
        let (cmds, nav) = rows(Lang::En, false, "zzzz");
        assert!(cmds.is_empty() && nav.is_empty());
    }
}
