//! View › Themes: the gallery of BIcraft's built-in report themes (`bicraft_visuals::
//! BUILTIN_THEMES`), each drawn as a swatch of its background, text and first data colours, as
//! Desktop's Themes dropdown shows its own; a click runs `theme.set {builtin}`. Below the
//! gallery, the theme commands Desktop lists there: Browse for themes (`theme.import`),
//! Customize current theme (the Customize theme dialog, `theme.customize`), Save current theme (`theme.export`), and a
//! custom theme JSON (`theme.set`'s form).

use std::sync::OnceLock;

use egui::{Color32, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::app::App;
use crate::i18n::Lang;
use crate::registry::Cmd;
use crate::theme::{self, Tokens};
use crate::widgets;

/// The gallery's button: the command whose button it is.
pub const COMMAND: &str = "theme.set";

/// A built-in theme as the gallery shows it.
pub struct Swatch {
    pub id: &'static str,
    /// The theme JSON's `name` (what the report records and the snapshot shows).
    pub name: String,
    pub background: Color32,
    pub foreground: Color32,
    pub colors: Vec<Color32>,
}

/// `#RRGGBB` (or `#RGB`) as a colour.
fn hex(s: &str) -> Option<Color32> {
    let h = s.trim().strip_prefix('#')?;
    let full: String = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect(),
        6 | 8 => h.chars().take(6).collect(),
        _ => return None,
    };
    let n = u32::from_str_radix(&full, 16).ok()?;
    Some(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

/// The built-in themes, parsed once.
pub fn swatches() -> &'static [Swatch] {
    static S: OnceLock<Vec<Swatch>> = OnceLock::new();
    S.get_or_init(|| {
        bicraft_visuals::BUILTIN_THEMES
            .iter()
            .filter_map(|t| {
                let v: Value = serde_json::from_str(t.json).ok()?;
                let color = |k: &str| v.get(k).and_then(Value::as_str).and_then(hex);
                Some(Swatch {
                    id: t.id,
                    name: v.get("name")?.as_str()?.to_string(),
                    background: color("background")?,
                    foreground: color("foreground")?,
                    colors: v
                        .get("dataColors")?
                        .as_array()?
                        .iter()
                        .filter_map(|c| c.as_str().and_then(hex))
                        .take(6)
                        .collect(),
                })
            })
            .collect()
    })
}

/// The gallery name of built-in theme `id` (BIcraft's own names, translated).
pub fn label(id: &str, l: Lang) -> &'static str {
    match id {
        "bicraft" => "BIcraft",
        "harbor" => l.t("Harbor", "Port"),
        "orchard" => l.t("Orchard", "Verger"),
        "ember" => l.t("Ember", "Braise"),
        "slate" => l.t("Slate", "Ardoise"),
        "clear-sight" => l.t("Clear sight", "Vision claire"),
        "dusk" => l.t("Dusk", "Crépuscule"),
        "high-contrast" => l.t("High contrast", "Contraste élevé"),
        _ => l.t("Theme", "Thème"),
    }
}

/// The name of the report's theme (the snapshot's resolved theme), if any.
fn current(app: &App) -> Option<String> {
    let l = app.loaded.as_ref()?;
    l.snap
        .get("report")?
        .get("theme")?
        .get("name")?
        .as_str()
        .map(str::to_string)
}

const TILE: egui::Vec2 = vec2(96.0, 74.0);

/// One theme's tile: the swatch (its background, a text line, bars in its data colours) and its
/// name; ringed when it is the report's theme.
fn tile(ui: &mut Ui, s: &Swatch, name: &str, on: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(TILE, Sense::click());
    let p = ui.painter_at(rect.expand(2.0));
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(5), t.hover);
    }
    let sw = Rect::from_min_size(rect.min + vec2(6.0, 5.0), vec2(TILE.x - 12.0, 46.0));
    p.rect_filled(sw, CornerRadius::same(3), s.background);
    p.rect_stroke(
        sw,
        CornerRadius::same(3),
        Stroke::new(1.0, t.border),
        egui::StrokeKind::Inside,
    );
    // a title line in the theme's text colour
    p.rect_filled(
        Rect::from_min_size(sw.min + vec2(6.0, 6.0), vec2(34.0, 4.0)),
        CornerRadius::same(1),
        s.foreground,
    );
    let n = s.colors.len().max(1) as f32;
    let bw = (sw.width() - 12.0) / n;
    for (i, c) in s.colors.iter().enumerate() {
        let h = 10.0 + 18.0 * (((i * 7) % 5) as f32 / 4.0);
        let x = sw.min.x + 6.0 + bw * i as f32;
        p.rect_filled(
            Rect::from_min_max(
                pos2(x + 1.0, sw.max.y - 5.0 - h),
                pos2(x + bw - 1.0, sw.max.y - 5.0),
            ),
            CornerRadius::ZERO,
            *c,
        );
    }
    if on {
        p.rect_stroke(
            rect.shrink(1.0),
            CornerRadius::same(5),
            Stroke::new(2.0, t.ring),
            egui::StrokeKind::Inside,
        );
    }
    p.text(
        pos2(rect.center().x, sw.max.y + 4.0),
        egui::Align2::CENTER_TOP,
        name,
        theme::font(11.0),
        t.text,
    );
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, name));
    resp
}

/// View › Themes: the big button and its gallery.
pub fn button(app: &mut App, ui: &mut Ui, c: &'static Cmd) {
    let l = app.lang;
    let st = crate::ribbon::state(app, c.id);
    let r = widgets::big_button(ui, c.icon(), c.text(l), st.is_ok()).on_hover_text(c.tooltip(
        l,
        &st,
        app.caps().reserved_keys,
    ));
    let tipped = crate::key_tips::button(app, ui, c.id, &r);
    if st.is_err() {
        if tipped {
            // greyed: the command says why in the status bar
            let p = crate::ribbon::prefill(app, c, None);
            crate::ribbon::invoke(app, c.id, p);
        }
        return;
    }
    let now = current(app);
    let mut picked: Option<(&'static str, Value)> = None;
    crate::key_tips::menu(app, &r, tipped).show(|ui| {
        ui.label(egui::RichText::new(l.t("BIcraft themes", "Thèmes BIcraft")).font(theme::bold(12.0)));
        egui::Grid::new("bicraft-theme-gallery")
            .spacing(vec2(4.0, 4.0))
            .show(ui, |ui| {
                for (i, s) in swatches().iter().enumerate() {
                    let on = now.as_deref() == Some(s.name.as_str());
                    if tile(ui, s, label(s.id, l), on).clicked() {
                        picked = Some((COMMAND, json!({"builtin": s.id})));
                        ui.close();
                    }
                    if i % 4 == 3 {
                        ui.end_row();
                    }
                }
            });
        ui.separator();
        for (id, en, fr) in [
            ("theme.import", "Browse for themes…", "Rechercher des thèmes…"),
            (
                crate::theme_dialog::COMMAND,
                "Customize current theme…",
                "Personnaliser le thème actuel…",
            ),
            (
                "theme.export",
                "Save current theme…",
                "Enregistrer le thème actuel…",
            ),
            (COMMAND, "Custom theme JSON…", "JSON de thème personnalisé…"),
        ] {
            if ui.button(l.t(en, fr)).clicked() {
                picked = Some((id, Value::Null));
                ui.close();
            }
        }
    });
    match picked {
        Some((crate::theme_dialog::COMMAND, Value::Null)) => crate::theme_dialog::open(app),
        Some((id, Value::Null)) => crate::form::open(app, id, json!({})),
        Some((id, params)) => crate::ribbon::invoke(app, id, params),
        None => {}
    }
}

/// `theme.set` without a theme (the palette, the control channel) opens its form.
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    if id == COMMAND && params.get("theme").is_none() && params.get("builtin").is_none() {
        crate::form::open(app, id, params.clone());
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_theme_has_a_swatch_and_a_name() {
        let s = swatches();
        assert_eq!(s.len(), bicraft_visuals::BUILTIN_THEMES.len());
        for sw in s {
            assert!(sw.colors.len() >= 6, "{}", sw.id);
            assert_ne!(label(sw.id, Lang::En), "Theme", "{}", sw.id);
            assert_ne!(label(sw.id, Lang::Fr), "Thème", "{}", sw.id);
        }
        assert_eq!(hex("#abc"), Some(Color32::from_rgb(0xaa, 0xbb, 0xcc)));
        assert_eq!(hex("red"), None);
    }
}
