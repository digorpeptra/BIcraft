//! Key tips on the ribbon (docs/desktop.md, "Keyboard navigation"): Alt pressed and released
//! alone (or F10) shows a letter over each ribbon tab; typing a tab's letter selects it and shows
//! a letter over each of its buttons; typing a button's letter runs what a click on it runs (the
//! same command id, its menu for a menu button). Escape backs out one level, Alt, F10 or a click
//! hides them.
//!
//! The letters come from the registry ([`crate::registry`]): the tabs' captions and the buttons'
//! texts in the UI's language, in the registry's order, so they are the same in every session and
//! never collide within a level ([`assign`]). The UI's own view buttons (zoom, fit, Show
//! validation) have no command and no letter.

use egui::{Context, Event, Id, Key, LayerId, Order, Rect, Response, Ui};

use crate::App;
use crate::i18n::Lang;
use crate::registry::{self, Tab};
use crate::theme::Tokens;

/// Which letters show.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Level {
    #[default]
    Off,
    /// A letter over each ribbon tab.
    Tabs,
    /// A letter over each button of this tab.
    Buttons(Tab),
}

/// The key tips' state (view state).
#[derive(Debug, Default)]
pub struct Tips {
    pub level: Level,
    /// The letters typed of a code longer than one.
    typed: String,
    /// Alt is down.
    alt_down: bool,
    /// Alt went down alone and nothing else happened since: its release shows the tips.
    alt_alone: bool,
    /// The command whose letter was typed: its button runs it in this frame, else
    /// [`finish`] does.
    press: Option<&'static str>,
    /// A menu opened by its letter, whose first item takes the focus once it is drawn.
    pub(crate) menu: Option<(Id, u8)>,
    /// The tabs drawn in this frame (File too), in global coordinates.
    tabs: Vec<(Tab, Rect)>,
    /// The command buttons drawn in this frame, in global coordinates.
    buttons: Vec<(&'static str, Rect)>,
    /// The badges drawn in the last frame: code and rectangle (tests).
    pub(crate) drawn: Vec<(String, Rect)>,
}

/// The characters a code is made of, in the order they are handed out.
const POOL: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// `c` as a key-tip character: an ASCII letter or digit in upper case (French accents folded).
fn fold(c: char) -> Option<char> {
    let c = match c.to_lowercase().next().unwrap_or(c) {
        'à' | 'â' | 'ä' | 'á' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'î' | 'ï' | 'í' => 'i',
        'ô' | 'ö' | 'ó' | 'œ' => 'o',
        'ù' | 'û' | 'ü' | 'ú' => 'u',
        'ç' => 'c',
        c => c,
    };
    c.is_ascii_alphanumeric().then(|| c.to_ascii_uppercase())
}

/// The characters `text` suggests, best first: its words' initials, then its other letters.
fn candidates(text: &str) -> Vec<char> {
    let mut initials = Vec::new();
    let mut rest = Vec::new();
    let mut start = true;
    for c in text.chars() {
        match fold(c) {
            Some(f) if start => {
                initials.push(f);
                start = false;
            }
            Some(f) => rest.push(f),
            None => start = !c.is_alphanumeric(),
        }
    }
    let mut out: Vec<char> = Vec::new();
    for c in initials.into_iter().chain(rest) {
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// One code for each of `texts`, in order: a character of its own text where one is free (the
/// initials first), else the first free one of A–Z, 0–9. Codes are one character while the texts
/// fit in the 36, else two for every text, so no code is the start of another.
pub fn assign(texts: &[&str]) -> Vec<String> {
    let pool: Vec<char> = POOL.iter().map(|b| char::from(*b)).collect();
    let mut used: Vec<String> = Vec::with_capacity(texts.len());
    for text in texts {
        let mine = candidates(text);
        let code = if texts.len() <= pool.len() {
            mine.iter()
                .chain(pool.iter())
                .map(|c| c.to_string())
                .find(|c| !used.contains(c))
        } else {
            let firsts: Vec<char> = mine.iter().chain(pool.iter()).copied().collect();
            firsts
                .iter()
                .flat_map(|a| firsts.iter().map(move |b| format!("{a}{b}")))
                .find(|c| c.chars().count() == 2 && !used.contains(c))
        };
        // more than 36 × 36 texts: no code (never in the registry)
        used.push(code.unwrap_or_default());
    }
    used
}

/// The tabs' codes in `l`: every tab of the registry (File and the contextual ones too, so a
/// tab's letter does not change when a selection shows another).
pub fn tab_codes(l: Lang) -> Vec<(Tab, String)> {
    let texts: Vec<&str> = Tab::ALL.iter().map(|t| t.caption(l)).collect();
    Tab::ALL.iter().copied().zip(assign(&texts)).collect()
}

/// The codes of tab `tab`'s buttons in `l`: each command the registry places on it, in the
/// registry's order.
pub fn button_codes(tab: Tab, l: Lang) -> Vec<(&'static str, String)> {
    let cmds: Vec<&'static registry::Cmd> = registry::ribbon_groups(tab)
        .into_iter()
        .flat_map(|(_, c)| c)
        .collect();
    let texts: Vec<&str> = cmds.iter().map(|c| c.text(l)).collect();
    cmds.iter().map(|c| c.id).zip(assign(&texts)).collect()
}

/// The ribbon is there and takes the keys: no File view, dialog, palette or form.
fn available(app: &App) -> bool {
    app.backstage.is_none() && app.palette.is_none() && app.form.is_none() && !app.keyboard.in_dialog()
}

/// The character a key types in a code (letters and digits).
fn key_char(k: Key) -> Option<char> {
    let mut s = k.name().chars();
    match (s.next(), s.next()) {
        (Some(c), None) if c.is_ascii_alphanumeric() => Some(c.to_ascii_uppercase()),
        _ => None,
    }
}

/// The start of a frame, before the ribbon is drawn: Alt alone and F10 show or hide the tips,
/// a letter selects a tab or presses a button, Escape backs out. While the tips show, the keys
/// they take reach nothing else.
pub fn input(app: &mut App, ctx: &Context) {
    app.keyboard.tips.tabs.clear();
    app.keyboard.tips.buttons.clear();
    let events = ctx.input(|i| i.events.clone());
    let mut toggle = false;
    let mut back = false;
    let mut keys = Vec::new();
    let mut off = false;
    let active = app.keyboard.tips.level != Level::Off;
    let tips = &mut app.keyboard.tips;
    for e in &events {
        match e {
            Event::ModifiersChanged(m) => {
                let alone = m.alt && !m.ctrl && !m.command && !m.shift && !m.mac_cmd;
                if alone && !tips.alt_down {
                    tips.alt_alone = true;
                } else if !m.alt && tips.alt_down && tips.alt_alone {
                    toggle = !toggle;
                }
                if !alone {
                    tips.alt_alone = false;
                }
                tips.alt_down = m.alt;
            }
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                tips.alt_alone = false;
                if *key == Key::F10 && modifiers.is_none() {
                    toggle = !toggle;
                } else if *key == Key::Escape {
                    back = true;
                } else if !modifiers.ctrl
                    && !modifiers.command
                    && let Some(c) = key_char(*key)
                {
                    keys.push(c);
                }
            }
            Event::PointerButton { pressed: true, .. } => {
                tips.alt_alone = false;
                off = true;
            }
            Event::WindowFocused(true) => {}
            // Alt held through a key's release, a drag (no snapping), a wheel or text: not
            // Alt alone
            _ => tips.alt_alone = false,
        }
    }
    if !available(app) {
        app.keyboard.tips.level = Level::Off;
        return;
    }
    let l = app.lang;
    let tips = &mut app.keyboard.tips;
    if toggle {
        tips.level = if tips.level == Level::Off {
            Level::Tabs
        } else {
            Level::Off
        };
        tips.typed.clear();
    }
    if off {
        tips.level = Level::Off;
    }
    let shown = active || tips.level != Level::Off;
    if tips.level != Level::Off && back {
        tips.level = match tips.level {
            Level::Buttons(_) => Level::Tabs,
            _ => Level::Off,
        };
        tips.typed.clear();
        keys.clear();
    }
    for c in keys {
        let level = app.keyboard.tips.level;
        if level == Level::Off {
            break;
        }
        app.keyboard.tips.typed.push(c);
        let typed = app.keyboard.tips.typed.clone();
        match level {
            Level::Tabs => {
                let codes = tab_codes(l);
                if let Some((tab, _)) = codes.iter().find(|(_, k)| *k == typed) {
                    app.keyboard.tips.typed.clear();
                    pick_tab(app, *tab);
                } else if !codes.iter().any(|(_, k)| k.starts_with(&typed)) {
                    app.keyboard.tips.typed.clear();
                }
            }
            Level::Buttons(tab) => {
                let codes = button_codes(tab, l);
                if let Some((id, _)) = codes.iter().find(|(_, k)| *k == typed) {
                    app.keyboard.tips.typed.clear();
                    app.keyboard.tips.press = Some(id);
                    app.keyboard.tips.level = Level::Off;
                } else if !codes.iter().any(|(_, k)| k.starts_with(&typed)) {
                    app.keyboard.tips.typed.clear();
                }
            }
            Level::Off => {}
        }
    }
    if shown {
        // the keys the tips took reach nothing else (a text field, Alt+F, the shortcuts)
        ctx.input_mut(|i| {
            i.events.retain(|e| match e {
                Event::Key { key, .. } => {
                    !(matches!(key, Key::Escape | Key::F10) || key_char(*key).is_some())
                }
                Event::Text(_) => false,
                _ => true,
            });
        });
    }
}

/// A tab's letter: File opens the File view, a tab the tab row shows is selected (its view too,
/// as a click) and its buttons' letters show.
fn pick_tab(app: &mut App, tab: Tab) {
    if tab == Tab::File {
        app.keyboard.tips.level = Level::Off;
        app.select_ribbon_tab(tab);
        return;
    }
    if !crate::ribbon::tabs(app).contains(&tab) {
        return;
    }
    app.select_ribbon_tab(tab);
    app.keyboard.tips.level = Level::Buttons(tab);
}

/// Notes that ribbon tab `tab` is drawn at `rect` of `ui` (its letter shows over it).
pub fn place_tab(app: &mut App, ui: &Ui, tab: Tab, rect: Rect) {
    let r = global(ui, rect);
    app.keyboard.tips.tabs.push((tab, r));
}

/// The button of command `id` drawn as `resp`: notes where its letter goes and returns true when
/// its letter was typed in this frame (the caller does what a click does).
pub fn button(app: &mut App, ui: &Ui, id: &'static str, resp: &Response) -> bool {
    if ui.clip_rect().intersects(resp.rect) {
        let r = global(ui, resp.rect);
        app.keyboard.tips.buttons.push((id, r));
    }
    take(app, id)
}

/// Whether command `id`'s letter was typed in this frame (and not run yet); true once.
pub fn take(app: &mut App, id: &str) -> bool {
    if app.keyboard.tips.press == Some(id) {
        app.keyboard.tips.press = None;
        return true;
    }
    false
}

/// The menu of button `resp` (`Popup::menu`), opened as well when its letter was typed
/// (`tipped`), where its first item takes the focus.
pub fn menu<'a>(app: &mut App, resp: &Response, tipped: bool) -> egui::Popup<'a> {
    let popup = egui::Popup::menu(resp);
    if !tipped {
        return popup;
    }
    app.keyboard.keyboard = true;
    app.keyboard.tips.menu = Some((egui::Popup::default_response_id(resp), MENU_FRAMES));
    popup.open_memory(egui::SetOpenCommand::Bool(true))
}

/// Frames a menu opened from the keyboard waits for its items.
pub(crate) const MENU_FRAMES: u8 = 4;

fn global(ui: &Ui, r: Rect) -> Rect {
    ui.ctx()
        .layer_transform_to_global(ui.layer_id())
        .map_or(r, |t| t.mul_rect(r))
}

/// The end of a frame: a typed letter whose button was not drawn runs its command as the button
/// would; the letters are drawn over what they press.
pub fn finish(app: &mut App, ctx: &Context) {
    if let Some(id) = app.keyboard.tips.press.take()
        && let Some(c) = registry::find(id)
    {
        let p = crate::ribbon::prefill(app, c, None);
        crate::ribbon::invoke(app, id, p);
    }
    paint(app, ctx);
}

/// Badge size: a letter in an 11 pt monospace font on a rounded fill.
const BADGE: egui::Vec2 = egui::vec2(18.0, 16.0);

/// Draws the letters of the level shown: under the middle of each tab, over the bottom of each
/// button (several commands on one button side by side).
fn paint(app: &mut App, ctx: &Context) {
    let l = app.lang;
    let tips = &app.keyboard.tips;
    let marks: Vec<(String, Rect)> = match tips.level {
        Level::Off => Vec::new(),
        Level::Tabs => tab_codes(l)
            .into_iter()
            .filter_map(|(tab, code)| {
                let (_, r) = tips.tabs.iter().find(|(t, _)| *t == tab)?;
                Some((code, *r))
            })
            .collect(),
        Level::Buttons(tab) => button_codes(tab, l)
            .into_iter()
            .filter_map(|(id, code)| {
                let (_, r) = tips.buttons.iter().find(|(i, _)| *i == id)?;
                Some((code, *r))
            })
            .collect(),
    };
    let t = Tokens::of(app.prefs.theme);
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("bicraft-key-tips")));
    let mut drawn: Vec<(String, Rect)> = Vec::new();
    for (code, r) in marks {
        let w = BADGE.x.max(10.0 + 8.0 * code.chars().count() as f32);
        let center = egui::pos2(r.center().x, r.bottom() - BADGE.y / 2.0);
        let mut badge = Rect::from_center_size(center, egui::vec2(w, BADGE.y));
        // several commands on one button (Model › New parameter): side by side
        while drawn.iter().any(|(_, b)| b.intersects(badge)) {
            badge = badge.translate(egui::vec2(w + 2.0, 0.0));
        }
        painter.rect_filled(badge, egui::CornerRadius::same(3), t.text);
        painter.text(
            badge.center(),
            egui::Align2::CENTER_CENTER,
            &code,
            egui::FontId::monospace(11.0),
            t.card,
        );
        drawn.push((code, badge));
    }
    app.keyboard.tips.drawn = drawn;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_prefix_free_on_every_level() {
        for l in [Lang::En, Lang::Fr] {
            let tabs: Vec<String> = tab_codes(l).into_iter().map(|(_, c)| c).collect();
            check(&tabs, "tabs");
            for tab in Tab::ALL {
                let codes: Vec<String> = button_codes(tab, l).into_iter().map(|(_, c)| c).collect();
                assert_eq!(
                    codes.len(),
                    registry::ribbon_groups(tab)
                        .iter()
                        .map(|g| g.1.len())
                        .sum::<usize>()
                );
                check(&codes, &format!("{tab:?}"));
            }
        }
    }

    fn check(codes: &[String], what: &str) {
        for (i, a) in codes.iter().enumerate() {
            assert!(!a.is_empty(), "{what}: an empty code");
            for b in codes.iter().skip(i + 1) {
                assert!(
                    !b.starts_with(a.as_str()) && !a.starts_with(b.as_str()),
                    "{what}: {a} and {b}"
                );
            }
        }
    }

    #[test]
    fn codes_follow_the_texts() {
        let en: Vec<(Tab, String)> = tab_codes(Lang::En);
        let code = |t: Tab| en.iter().find(|(x, _)| *x == t).map(|(_, c)| c.clone());
        assert_eq!(code(Tab::File).as_deref(), Some("F"));
        assert_eq!(code(Tab::Home).as_deref(), Some("H"));
        assert_eq!(code(Tab::Model).as_deref(), Some("M"));
        // initials first, then the text's other letters, French accents folded
        assert_eq!(assign(&["New measure", "New model", "Next"]), ["N", "M", "E"]);
        assert_eq!(assign(&["Modèle", "Mesure"]), ["M", "E"]);
        assert_eq!(assign(&["Écrire"]), ["E"]);
        // stable: the same texts give the same codes
        assert_eq!(tab_codes(Lang::Fr), tab_codes(Lang::Fr));
        // beyond 36 texts every code has two characters
        let many: Vec<String> = (0..40).map(|i| format!("Item {i}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        let codes = assign(&refs);
        assert!(codes.iter().all(|c| c.chars().count() == 2));
        check(&codes, "40 texts");
    }

    #[test]
    fn the_badges_text_meets_4_5_to_1() {
        for t in [Tokens::light(), Tokens::dark(), Tokens::high_contrast()] {
            assert!(crate::theme::contrast(t.card, t.text) >= 4.5);
        }
    }
}
