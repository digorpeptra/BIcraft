//! Power BI Desktop's "Edit interactions" mode (ROADMAP M11.15, docs/design.md "Edit
//! interactions"). The ribbon's Edit interactions button (Visual tab, Interactions group) turns
//! it on and off, Escape turns it off; readers cannot enter it. While it is on, the visual
//! selected on the canvas is the *source*, and every other visual of the page that takes part in
//! interactions (a visual with fields) shows a small icon set at its top-right corner: Filter,
//! Highlight, None. The icon of the current effective mode is marked; when it is the default (no
//! rule in the page's `visualInteractions`) it is marked with a lighter look and the tooltip says
//! "Default: …". A click on another icon runs `page.interactions` with that one rule (one undo
//! step); a click on the default mode removes the rule (`type: "default"`).
//!
//! The modes offered and the default come from the snapshot, so they never disagree with what
//! the viewer does (`crates/engine/src/interact.rs`, docs/interaction-oracle.md rules 7, 9 and
//! 14): each visual's `interactivity` (`target`, `highlight` from the engine's table
//! of the types that highlight, `slicer`), the page's `interactions` and the report's
//! `settings.default_interaction`. A slicer's selection only filters (Filter, None); a target that
//! cannot highlight shows Filter and None; otherwise all three, the default being Highlight, or
//! Filter when the report cross-filters by default.
//!
//! Live preview: clicks on data points still go to the viewer (`view.interact`), so the source's
//! selection cross-filters or highlights the page under the rules being edited. Leaving the mode
//! clears the page's data-point selection (`view.interact {action: "clear_selection"}`); the
//! slicers keep theirs.

use egui::{CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::Then;
use crate::i18n::Lang;
use crate::icons::{self, Icon};
use crate::ribbon;
use crate::theme::Tokens;

/// The command the mode's icons run (and the ribbon button that toggles the mode).
pub const COMMAND: &str = "page.interactions";

/// How a source's selection reaches a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Filter,
    Highlight,
    None,
}

impl Mode {
    /// The word `page.interactions` takes (and the snapshot's `interactions` use).
    pub fn word(self) -> &'static str {
        match self {
            Mode::Filter => "filter",
            Mode::Highlight => "highlight",
            Mode::None => "none",
        }
    }

    fn from_word(s: &str) -> Option<Mode> {
        match s.to_ascii_lowercase().as_str() {
            "filter" | "datafilter" => Some(Mode::Filter),
            "highlight" | "highlightfilter" => Some(Mode::Highlight),
            "none" | "nofilter" => Some(Mode::None),
            _ => None,
        }
    }

    pub fn label(self, l: Lang) -> &'static str {
        match self {
            Mode::Filter => l.t("Filter", "Filtrer"),
            Mode::Highlight => l.t("Highlight", "Mettre en surbrillance"),
            Mode::None => l.t("None", "Aucune"),
        }
    }

    fn icon(self) -> Icon {
        match self {
            Mode::Filter => Icon::Filter,
            Mode::Highlight => Icon::Highlight,
            Mode::None => Icon::NoInteraction,
        }
    }
}

/// What the icon set of one target shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// The modes offered, in Desktop's order (Filter, Highlight, None).
    pub modes: Vec<Mode>,
    /// The mode without a rule.
    pub default: Mode,
    /// The effective mode now.
    pub current: Mode,
    /// Whether a rule of the page sets it (`false`: the default).
    pub explicit: bool,
}

fn flag(v: &Value, k: &str) -> bool {
    v.get("interactivity")
        .and_then(|i| i.get(k))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn name(v: &Value) -> &str {
    v.get("name").and_then(Value::as_str).unwrap_or("")
}

/// The choice for `target` (snapshot visuals) when `source` is the source, on `page` (a snapshot
/// page) of a report that cross-filters by default when `cross_filter`. `None`: the target takes
/// no part (no fields, a group, hidden, the source itself).
pub fn choice(page: &Value, source: &Value, target: &Value, cross_filter: bool) -> Option<Choice> {
    if name(source) == name(target)
        || !flag(source, "target")
        || !flag(target, "target")
        || target.get("hidden").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    let highlight = !flag(source, "slicer") && flag(target, "highlight");
    let modes = if highlight {
        vec![Mode::Filter, Mode::Highlight, Mode::None]
    } else {
        vec![Mode::Filter, Mode::None]
    };
    let default = if highlight && !cross_filter {
        Mode::Highlight
    } else {
        Mode::Filter
    };
    let rule = page
        .get("interactions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|r| {
            r.get("source").and_then(Value::as_str) == Some(name(source))
                && r.get("target").and_then(Value::as_str) == Some(name(target))
        })
        .and_then(|r| r.get("type").and_then(Value::as_str))
        .and_then(Mode::from_word);
    // a rule the pair cannot follow (Highlight on a card) acts as Filter, as in the viewer
    let current = match rule {
        Some(Mode::Highlight) if !highlight => Some(Mode::Filter),
        r => r,
    };
    Some(Choice {
        current: current.unwrap_or(default),
        explicit: current.is_some(),
        modes,
        default,
    })
}

/// Whether the active document is in the mode.
pub fn on(app: &App) -> bool {
    app.cmd.edit_interactions
}

/// The ribbon button's click: on, or off (the selection made while editing is cleared).
pub fn toggle(app: &mut App) {
    if on(app) {
        leave(app);
        return;
    }
    if let Err(why) = ribbon::state(app, COMMAND) {
        app.status = format!("{} {why}", app.lang.t("Unavailable:", "Indisponible :"));
        return;
    }
    app.cmd.edit_interactions = true;
    app.status = app
        .lang
        .t(
            "Edit interactions: select a visual, then pick how it affects each of the others.",
            "Modifier les interactions : sélectionnez un visuel, puis choisissez son effet sur chacun des autres.",
        )
        .to_string();
}

/// Turns the mode off; a data-point selection of the page shown is cleared.
pub fn leave(app: &mut App) {
    if !on(app) {
        return;
    }
    app.cmd.edit_interactions = false;
    app.status = String::new();
    let Some(page) = shown_page(app) else { return };
    let selected = page
        .get("visuals")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|v| v.pointer("/interaction/selected").is_some());
    if selected && let Some(n) = page.get("name").and_then(Value::as_str) {
        app.interact(json!({"action": "clear_selection", "page": n}));
    }
}

fn shown_page(app: &App) -> Option<Value> {
    app.loaded
        .as_ref()?
        .snap
        .pointer(&format!("/report/pages/{}", app.active_page()))
        .cloned()
}

/// Side of one icon of the set (points).
pub const ICON: f32 = 20.0;
/// Gap between the icons.
const GAP: f32 = 2.0;

/// The icon set's place over a target drawn at `r` (screen): its top-right corner, inside.
pub fn strip(r: Rect, n: usize) -> Rect {
    let w = n as f32 * ICON + n.saturating_sub(1) as f32 * GAP + 4.0;
    let h = ICON + 4.0;
    Rect::from_min_size(pos2(r.max.x - w - 2.0, r.min.y + 2.0), vec2(w, h))
}

/// The accessible label of one icon: "Highlight: Sales by Category".
pub fn icon_label(mode: Mode, target_title: &str, l: Lang) -> String {
    format!("{}: {target_title}", mode.label(l))
}

/// The name the icons' labels give a target: its title, else its automatic title, else its name.
pub fn title(v: &Value) -> String {
    let t = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
    };
    crate::app::clip(
        &t("title")
            .or_else(|| t("auto_title"))
            .unwrap_or_else(|| name(v).to_string()),
        60,
    )
}

/// Draws the icon sets over the page (`rect` on screen, `zoom` screen points per report pixel)
/// and runs the rule a click picks. Called after the selection outline.
pub fn overlay(app: &mut App, ui: &Ui, rect: Rect, zoom: f32) {
    if !on(app) {
        return;
    }
    // a role that changed to reader leaves the mode
    if ribbon::state(app, COMMAND).is_err() {
        leave(app);
        return;
    }
    let Some(sel) = app.cmd.visual.clone() else { return };
    let Some(page) = shown_page(app) else { return };
    if page.get("name").and_then(Value::as_str) != Some(sel.page.as_str()) {
        return;
    }
    let cross_filter = app
        .loaded
        .as_ref()
        .and_then(|l| {
            l.snap
                .pointer("/report/settings/default_interaction")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .as_deref()
        == Some("filter");
    let visuals: Vec<Value> = page
        .get("visuals")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let Some(source) = visuals.iter().find(|v| name(v) == sel.visual) else {
        return;
    };
    let t = Tokens::of(app.prefs.theme);
    let l = app.lang;
    let mut picked: Option<(String, Mode, Choice)> = None;
    for target in visuals.iter().take(2000) {
        let Some(c) = choice(&page, source, target, cross_filter) else {
            continue;
        };
        let n = |k: &str| {
            target
                .get(k)
                .and_then(Value::as_f64)
                .filter(|x| x.is_finite())
                .unwrap_or(0.0) as f32
        };
        let r = Rect::from_min_size(
            rect.min + vec2(n("x"), n("y")) * zoom,
            vec2(n("width"), n("height")) * zoom,
        );
        let s = strip(r, c.modes.len());
        if !rect.intersects(s) {
            continue;
        }
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(s, CornerRadius::same(4), t.card);
        painter.rect_stroke(
            s,
            CornerRadius::same(4),
            Stroke::new(1.0, t.border),
            StrokeKind::Inside,
        );
        let target_title = title(target);
        for (i, m) in c.modes.iter().enumerate() {
            let at = pos2(s.min.x + 2.0 + i as f32 * (ICON + GAP), s.min.y + 2.0);
            let b = Rect::from_min_size(at, vec2(ICON, ICON));
            let id = ui.id().with(("bicraft-interaction", name(target), m.word()));
            let resp = ui.interact(b, id, Sense::click());
            let marked = *m == c.current;
            if marked {
                // the default is marked with the soft fill only; a rule adds the ring
                painter.rect_filled(b, CornerRadius::same(3), t.accent_soft);
                if c.explicit {
                    painter.rect_stroke(
                        b,
                        CornerRadius::same(3),
                        Stroke::new(1.5, t.ring),
                        StrokeKind::Inside,
                    );
                }
            } else if resp.hovered() {
                painter.rect_filled(b, CornerRadius::same(3), t.hover);
            }
            icons::paint(&painter, b.shrink(3.0), m.icon(), t.text, false);
            let label = icon_label(*m, &target_title, l);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, marked, &label));
            let tip = if *m == c.default {
                format!("{} {}", l.t("Default:", "Par défaut :"), m.label(l))
            } else {
                m.label(l).to_string()
            };
            let clicked = resp.clicked();
            let _ = resp.on_hover_text(tip);
            if clicked {
                picked = Some((name(target).to_string(), *m, c.clone()));
            }
        }
    }
    let Some((target, m, c)) = picked else { return };
    let word = match rule_for(m, &c) {
        Some(w) => w,
        None => return,
    };
    let params = json!({"page": sel.page, "rules": [{"source": sel.visual, "target": target, "type": word}]});
    app.run(Call { id: COMMAND, params }, Then::Nothing);
}

/// The rule type a click on mode `m` sends for a target in state `c`: the default removes the
/// rule (`"default"`), another mode sets it; `None` when nothing would change.
pub fn rule_for(m: Mode, c: &Choice) -> Option<&'static str> {
    if m == c.default {
        c.explicit.then_some("default")
    } else if m == c.current && c.explicit {
        None
    } else {
        Some(m.word())
    }
}
