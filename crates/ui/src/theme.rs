//! Design tokens (light and dark), their application to egui's style, the type scale and the
//! BIcraft mark, after DeckCraft's design system with BIcraft's amber accent (docs/design.md).
//!
//! Ported from DeckCraft `crates/ui-egui/src/theme.rs` @ 0ec0cc5 (MIT OR Apache-2.0, © 2026
//! ArtCraft Team and the DeckCraft contributors): the token names, the neutral values and the
//! structure of [`apply`]. Adapted: the accent family (amber instead of orange, slate text on
//! amber), the text selection colours, the caret, the fonts (Selawik, see `fonts.rs`), and the
//! status colours BIcraft adds.

use egui::{Color32, CornerRadius, FontId, Pos2, Rect, Stroke, Visuals, pos2, vec2};

/// Light (the default), dark, or high contrast (white on black, 7:1 text).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Brightness {
    #[default]
    Light,
    Dark,
    HighContrast,
}

impl Brightness {
    /// `"light"`, `"dark"` or `"high-contrast"`, as kept in the preferences.
    pub fn name(self) -> &'static str {
        match self {
            Brightness::Light => "light",
            Brightness::Dark => "dark",
            Brightness::HighContrast => "high-contrast",
        }
    }

    /// The preference value; anything else is light.
    pub fn from_name(s: &str) -> Brightness {
        match s {
            "dark" => Brightness::Dark,
            "high-contrast" => Brightness::HighContrast,
            _ => Brightness::Light,
        }
    }

    /// The title row's sun and moon: dark goes to light, the others to dark.
    pub fn toggled(self) -> Brightness {
        match self {
            Brightness::Light => Brightness::Dark,
            Brightness::Dark | Brightness::HighContrast => Brightness::Light,
        }
    }
}

/// The brand colours (the logo, `assets/logo/bicraft-icon.svg`).
pub const AMBER: Color32 = Color32::from_rgb(0xf2, 0xa5, 0x16);
pub const DARK_AMBER: Color32 = Color32::from_rgb(0xd9, 0x82, 0x0b);
pub const TEAL: Color32 = Color32::from_rgb(0x1f, 0x9e, 0x8f);
pub const SLATE: Color32 = Color32::from_rgb(0x2b, 0x34, 0x45);

const fn rgb(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

const fn rgba(v: u32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied_const((v >> 16) as u8, (v >> 8) as u8, v as u8, a)
}

/// Every colour the chrome uses (docs/design.md §2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    /// Window background: title row, tab row, side panes, status bar.
    pub chrome: Color32,
    /// Ribbon card, task pane, popovers, dialogs, model cards.
    pub card: Color32,
    pub card_border: Color32,
    /// Around the report page and behind the model diagram.
    pub canvas: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub border: Color32,
    pub separator: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub selected: Color32,
    /// Neutral selection outline.
    pub selection: Color32,
    pub handle_fill: Color32,
    pub handle_stroke: Color32,
    pub text_selection: Color32,
    pub field_bg: Color32,
    pub shadow: Color32,
    /// The amber accent: fills and 2–3 px marks only, never text.
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_pressed: Color32,
    /// Text and icons on an accent fill.
    pub accent_text: Color32,
    /// Accent-coloured text: contextual tabs, the transient status message, links.
    pub contextual: Color32,
    /// Selection ring: selected page, card, focused field.
    pub ring: Color32,
    /// The keyboard focus ring ([`crate::keyboard`]): 2 px around the focused control, at 4.5:1
    /// or more on the chrome, the cards, the canvas and the hover fill.
    pub focus: Color32,
    /// Background of a selected row inside a card.
    pub accent_soft: Color32,
    pub guide: Color32,
    pub err: Color32,
    pub err_soft: Color32,
    pub err_text: Color32,
    pub warn: Color32,
    pub ok: Color32,
    /// The "live" connection dot.
    pub live: Color32,
    /// DAX syntax colours (the formula bar): keywords, functions, strings, numbers, table, column
    /// and measure references, comments; each reads at 4.5:1 on `field_bg` and `card`.
    pub syn_keyword: Color32,
    pub syn_function: Color32,
    pub syn_string: Color32,
    pub syn_number: Color32,
    pub syn_ref: Color32,
    pub syn_comment: Color32,
}

impl Tokens {
    pub const fn light() -> Self {
        Tokens {
            dark: false,
            chrome: rgb(0xf5f5f5),
            card: rgb(0xffffff),
            card_border: rgb(0xe3e3e3),
            canvas: rgb(0xf0f0f0),
            text: rgb(0x242424),
            text_dim: rgb(0x5c5c5c),
            text_faint: rgb(0x999999),
            border: rgb(0xd6d6d6),
            separator: rgb(0xe1e1e1),
            hover: rgb(0xebebeb),
            pressed: rgb(0xdedede),
            selected: rgb(0xe6e6e6),
            selection: rgb(0x7a7a7a),
            handle_fill: rgb(0xffffff),
            handle_stroke: rgb(0x6e6e6e),
            text_selection: rgba(0x5a8fe0, 90),
            field_bg: rgb(0xffffff),
            shadow: Color32::from_black_alpha(28),
            accent: AMBER,
            accent_hover: rgb(0xe6990f),
            accent_pressed: DARK_AMBER,
            accent_text: SLATE,
            contextual: rgb(0x9a5a00),
            ring: DARK_AMBER,
            focus: rgb(0x0b57d0),
            accent_soft: rgba(0xf2a516, 40),
            guide: AMBER,
            err: rgb(0xcf3434),
            err_soft: rgb(0xfde7e9),
            err_text: rgb(0xa4262c),
            warn: rgb(0xa94f07),
            ok: rgb(0x157a6e),
            live: TEAL,
            syn_keyword: rgb(0x0b57d0),
            syn_function: rgb(0x7a3e9d),
            syn_string: rgb(0xa4262c),
            syn_number: rgb(0x0e7a3e),
            syn_ref: rgb(0x8a5300),
            syn_comment: rgb(0x4f6b4f),
        }
    }

    pub const fn dark() -> Self {
        Tokens {
            dark: true,
            chrome: rgb(0x262626),
            card: rgb(0x333333),
            card_border: rgb(0x444444),
            canvas: rgb(0x1e1e1e),
            text: rgb(0xeeeeee),
            text_dim: rgb(0xbbbbbb),
            text_faint: rgb(0x888888),
            border: rgb(0x4a4a4a),
            separator: rgb(0x484848),
            hover: rgb(0x404040),
            pressed: rgb(0x4c4c4c),
            selected: rgb(0x474747),
            selection: rgb(0xaaaaaa),
            handle_fill: rgb(0xffffff),
            handle_stroke: rgb(0x6e6e6e),
            text_selection: rgba(0x5a8fe0, 90),
            field_bg: rgb(0x2a2a2a),
            shadow: Color32::from_black_alpha(90),
            accent: AMBER,
            accent_hover: rgb(0xf5b942),
            accent_pressed: DARK_AMBER,
            accent_text: SLATE,
            contextual: rgb(0xf5b942),
            ring: AMBER,
            focus: rgb(0x8ccbff),
            accent_soft: rgba(0xf2a516, 56),
            guide: rgb(0xf5b942),
            err: rgb(0xff6b6b),
            err_soft: rgba(0xff6b6b, 33),
            err_text: rgb(0xff8a8a),
            warn: rgb(0xf59e4b),
            ok: rgb(0x3cc2b0),
            live: TEAL,
            syn_keyword: rgb(0x6cb6ff),
            syn_function: rgb(0xd2a8ff),
            syn_string: rgb(0xff9b8f),
            syn_number: rgb(0x7ee0a1),
            syn_ref: rgb(0xf5c26b),
            syn_comment: rgb(0x8fb08f),
        }
    }

    /// High contrast: white text on black at 21:1, every text token at 7:1 or more (WCAG
    /// 1.4.6), white borders, yellow marks and links, as the common high-contrast schemes do.
    pub const fn high_contrast() -> Self {
        Tokens {
            dark: true,
            chrome: rgb(0x000000),
            card: rgb(0x000000),
            card_border: rgb(0xffffff),
            canvas: rgb(0x000000),
            text: rgb(0xffffff),
            text_dim: rgb(0xe6e6e6),
            text_faint: rgb(0xb3b3b3),
            border: rgb(0xffffff),
            separator: rgb(0xb3b3b3),
            hover: rgb(0x333333),
            pressed: rgb(0x4d4d4d),
            selected: rgb(0x00395c),
            selection: rgb(0x1aebff),
            handle_fill: rgb(0x000000),
            handle_stroke: rgb(0xffff00),
            text_selection: rgba(0x1aebff, 110),
            field_bg: rgb(0x000000),
            shadow: Color32::from_black_alpha(0),
            accent: rgb(0xffff00),
            accent_hover: rgb(0xffff66),
            accent_pressed: rgb(0xe6e600),
            accent_text: rgb(0x000000),
            contextual: rgb(0xffff00),
            ring: rgb(0xffff00),
            focus: rgb(0x1aebff),
            accent_soft: rgba(0x1aebff, 70),
            guide: rgb(0x1aebff),
            err: rgb(0xff8c8c),
            err_soft: rgb(0x3d0000),
            err_text: rgb(0xffb3b3),
            warn: rgb(0xffb000),
            ok: rgb(0x3ff23f),
            live: rgb(0x3ff23f),
            syn_keyword: rgb(0x8cc8ff),
            syn_function: rgb(0xe0c2ff),
            syn_string: rgb(0xffb3a8),
            syn_number: rgb(0x8cf0b0),
            syn_ref: rgb(0xffe08a),
            syn_comment: rgb(0xc8dcc8),
        }
    }

    pub const fn of(b: Brightness) -> Self {
        match b {
            Brightness::Light => Self::light(),
            Brightness::Dark => Self::dark(),
            Brightness::HighContrast => Self::high_contrast(),
        }
    }

    /// The tokens of the theme `ctx` uses (high contrast is the dark style on a black chrome).
    pub fn get(ctx: &egui::Context) -> Self {
        let v = &ctx.global_style().visuals;
        if v.dark_mode && v.panel_fill == Self::high_contrast().chrome {
            Self::high_contrast()
        } else if v.dark_mode {
            Self::dark()
        } else {
            Self::light()
        }
    }
}

/// Applies `t` to egui's style (docs/design.md §2.4).
pub fn apply(ctx: &egui::Context, t: &Tokens) {
    let mut v = if t.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = t.chrome;
    v.window_fill = t.card;
    v.extreme_bg_color = t.field_bg;
    v.text_edit_bg_color = Some(t.field_bg);
    v.faint_bg_color = t.hover;
    v.code_bg_color = t.hover;
    v.window_stroke = Stroke::new(1.0, t.card_border);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    // egui uses the selection colours for selected text and `selectable_label`: a soft amber
    // with the text colour stays readable in both themes
    v.selection.bg_fill = t.accent_soft.gamma_multiply(1.6);
    v.selection.stroke = Stroke::new(1.0, t.text);
    v.hyperlink_color = t.contextual;
    v.warn_fg_color = t.warn;
    v.error_fg_color = t.err;
    v.override_text_color = Some(t.text);
    v.text_cursor.stroke = Stroke::new(2.0, t.text);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(5);
        w.fg_stroke.color = t.text;
    }
    v.widgets.noninteractive.bg_fill = t.card;
    v.widgets.noninteractive.weak_bg_fill = t.card;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.separator);
    v.widgets.noninteractive.fg_stroke.color = t.text_dim;
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_fill = t.field_bg;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, t.border);
    v.widgets.hovered.weak_bg_fill = t.hover;
    v.widgets.hovered.bg_fill = t.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.text_faint);
    v.widgets.active.weak_bg_fill = t.pressed;
    v.widgets.active.bg_fill = t.pressed;
    v.widgets.active.bg_stroke = Stroke::new(1.0, t.ring);
    v.widgets.open.weak_bg_fill = t.pressed;
    v.widgets.open.bg_fill = t.pressed;
    v.widgets.open.bg_stroke = Stroke::NONE;
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 18,
        spread: 0,
        color: t.shadow,
    };
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 3],
        blur: 12,
        spread: 0,
        color: t.shadow,
    };
    // an explicit theme: the app chooses light or dark, not the operating system
    let theme = if t.dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_theme(theme);
    ctx.set_visuals_of(theme, v);
    ctx.style_mut_of(theme, |s| {
        s.spacing.item_spacing = vec2(6.0, 4.0);
        s.spacing.button_padding = vec2(6.0, 3.0);
        s.spacing.interact_size.y = 22.0;
        s.text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(13.0));
        s.text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(13.0));
        s.text_styles
            .insert(egui::TextStyle::Small, FontId::proportional(11.0));
        s.text_styles
            .insert(egui::TextStyle::Monospace, FontId::monospace(12.0));
        s.text_styles.insert(egui::TextStyle::Heading, bold(18.0));
    });
}

/// The UI font at `size` points (Selawik).
pub fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

/// The UI's emphasis font: Selawik Semibold (DeckCraft's `bold()`, Inter SemiBold there).
pub fn bold(size: f32) -> FontId {
    crate::fonts::semibold_px(size)
}

/// WCAG contrast ratio of two opaque colours (1 to 21).
pub fn contrast(a: Color32, b: Color32) -> f32 {
    fn lum(c: Color32) -> f32 {
        let ch = |v: u8| {
            let s = f32::from(v) / 255.0;
            if s <= 0.039_28 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
    }
    let (x, y) = (lum(a), lum(b));
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

/// The BIcraft mark, painted from the proportions of `assets/logo/bicraft-icon.svg`: a slate
/// square, an amber frame, three bars in teal, amber and dark amber.
pub fn paint_logo(p: &egui::Painter, r: Rect) {
    let s = r.width().min(r.height());
    if !s.is_finite() || s < 1.0 {
        return;
    }
    let sq = Rect::from_center_size(r.center(), vec2(s, s));
    let u = s / 100.0;
    let at = |x: f32, y: f32| -> Pos2 { pos2(sq.min.x + (x + 50.0) * u, sq.min.y + (y + 50.0) * u) };
    let cr = |v: f32| CornerRadius::same((v * u).round().clamp(0.0, 255.0) as u8);
    p.rect_filled(sq, cr(18.0), SLATE);
    p.rect_stroke(
        Rect::from_min_max(at(-40.0, -40.0), at(40.0, 40.0)),
        cr(11.0),
        Stroke::new((3.0 * u).max(0.75), AMBER),
        egui::StrokeKind::Middle,
    );
    for (x, y, h, c) in [
        (-24.5, 3.0, 22.0, TEAL),
        (-8.5, -11.0, 36.0, AMBER),
        (7.5, -25.0, 50.0, DARK_AMBER),
    ] {
        p.rect_filled(Rect::from_min_max(at(x, y), at(x + 11.0, y + h)), cr(2.0), c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keyboard focus ring reads at 4.5:1 on every surface it is drawn over, in every theme
    /// (7:1 on the high-contrast chrome).
    #[test]
    fn the_focus_ring_meets_4_5_to_1() {
        let hc = Tokens::high_contrast();
        assert!(contrast(hc.focus, hc.chrome) >= 7.0);
        for (t, min) in [(Tokens::light(), 4.5), (Tokens::dark(), 4.5), (hc, 4.5)] {
            for (bg, on) in [
                (t.chrome, "chrome"),
                (t.card, "card"),
                (t.canvas, "canvas"),
                (t.field_bg, "field_bg"),
                (t.hover, "hover"),
                (t.pressed, "pressed"),
                (t.selected, "selected"),
            ] {
                let c = contrast(t.focus, bg);
                assert!(c >= min, "dark {}: focus on {on} is {c:.2}:1", t.dark);
            }
        }
    }

    /// docs/design.md §11: every text/background pair reads (4.5:1), accent marks are visible.
    #[test]
    fn tokens_meet_their_contrast() {
        for (t, name) in [
            (Tokens::light(), "light"),
            (Tokens::dark(), "dark"),
            (Tokens::high_contrast(), "high-contrast"),
        ] {
            for (fg, what) in [
                (t.text, "text"),
                (t.text_dim, "text_dim"),
                (t.contextual, "contextual"),
                (t.err, "err"),
                (t.warn, "warn"),
                (t.ok, "ok"),
            ] {
                for (bg, on) in [(t.chrome, "chrome"), (t.card, "card")] {
                    let c = contrast(fg, bg);
                    assert!(c >= 4.5, "{name}: {what} on {on} is {c:.2}:1");
                }
            }
            for (fg, what) in [
                (t.syn_keyword, "syn_keyword"),
                (t.syn_function, "syn_function"),
                (t.syn_string, "syn_string"),
                (t.syn_number, "syn_number"),
                (t.syn_ref, "syn_ref"),
                (t.syn_comment, "syn_comment"),
            ] {
                for (bg, on) in [(t.field_bg, "field_bg"), (t.card, "card")] {
                    let c = contrast(fg, bg);
                    assert!(c >= 4.5, "{name}: {what} on {on} is {c:.2}:1");
                }
            }
            for bg in [t.accent, t.accent_hover] {
                let c = contrast(t.accent_text, bg);
                assert!(c >= 4.5, "{name}: accent_text on the accent is {c:.2}:1");
            }
            assert!(contrast(t.err_text, t.err_soft.to_opaque()) >= 4.5 || t.dark);
            let ring = contrast(t.ring, t.card);
            assert!(ring >= 2.9, "{name}: ring on card is {ring:.2}:1");
        }
        // white on amber does not read: the reason for slate text
        assert!(contrast(Color32::WHITE, AMBER) < 3.0);
    }

    /// High contrast: every text token reads at 7:1 (WCAG 1.4.6) on the chrome, the cards, the
    /// fields and the hover and selected rows; borders and marks at 7:1 on the chrome.
    #[test]
    fn high_contrast_tokens_meet_7_to_1() {
        let t = Tokens::high_contrast();
        for (fg, what) in [
            (t.text, "text"),
            (t.text_dim, "text_dim"),
            (t.text_faint, "text_faint"),
            (t.contextual, "contextual"),
            (t.err, "err"),
            (t.err_text, "err_text"),
            (t.warn, "warn"),
            (t.ok, "ok"),
            (t.syn_keyword, "syn_keyword"),
            (t.syn_function, "syn_function"),
            (t.syn_string, "syn_string"),
            (t.syn_number, "syn_number"),
            (t.syn_ref, "syn_ref"),
            (t.syn_comment, "syn_comment"),
        ] {
            for (bg, on) in [
                (t.chrome, "chrome"),
                (t.card, "card"),
                (t.field_bg, "field_bg"),
                (t.canvas, "canvas"),
            ] {
                let c = contrast(fg, bg);
                assert!(c >= 7.0, "{what} on {on} is {c:.2}:1");
            }
        }
        for (bg, on) in [
            (t.hover, "hover"),
            (t.pressed, "pressed"),
            (t.selected, "selected"),
        ] {
            let c = contrast(t.text, bg);
            assert!(c >= 7.0, "text on {on} is {c:.2}:1");
        }
        assert!(contrast(t.err_text, t.err_soft) >= 7.0);
        for bg in [t.accent, t.accent_hover, t.accent_pressed] {
            assert!(contrast(t.accent_text, bg) >= 7.0);
        }
        for (c, what) in [
            (t.border, "border"),
            (t.ring, "ring"),
            (t.selection, "selection"),
            (t.accent, "accent"),
        ] {
            assert!(contrast(c, t.chrome) >= 7.0, "{what}");
        }
    }

    #[test]
    fn light_is_the_default_and_names_round_trip() {
        assert_eq!(Brightness::default(), Brightness::Light);
        for b in [Brightness::Light, Brightness::Dark] {
            assert_eq!(Brightness::from_name(b.name()), b);
            assert_eq!(b.toggled().toggled(), b);
        }
        let hc = Brightness::HighContrast;
        assert_eq!(Brightness::from_name(hc.name()), hc);
        assert_eq!(hc.toggled(), Brightness::Light);
        assert_eq!(Brightness::from_name("purple"), Brightness::Light);
        assert_eq!(Brightness::from_name(""), Brightness::Light);
    }

    #[test]
    fn apply_sets_the_tokens() {
        let ctx = egui::Context::default();
        for b in [
            Brightness::Light,
            Brightness::Dark,
            Brightness::HighContrast,
            Brightness::Dark,
        ] {
            let t = Tokens::of(b);
            apply(&ctx, &t);
            assert_eq!(Tokens::get(&ctx), t);
            let v = ctx.global_style().visuals.clone();
            assert_eq!(v.panel_fill, t.chrome);
            assert_eq!(v.window_fill, t.card);
            assert_eq!(v.override_text_color, Some(t.text));
            assert_eq!(v.hyperlink_color, t.contextual);
        }
    }

    #[test]
    fn the_logo_paints_at_any_size() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            for s in [0.0, 0.5, 8.0, 18.0, 64.0, 10_000.0, f32::NAN, f32::INFINITY] {
                paint_logo(ui.painter(), Rect::from_min_size(pos2(0.0, 0.0), vec2(s, s)));
            }
        });
        out.textures_delta.clear();
    }
}
