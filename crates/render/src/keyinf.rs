//! The key influencers visual (`keyDriversVisual`, `docs/key-influencers.md`). Power BI computes
//! it with its own models, which BIcraft does not reproduce, so by default the body is a
//! placeholder laid out like Desktop's visual (the "Key influencers" / "Top segments" tabs, the
//! question line naming the target and the goal, the grey analysis panel with its white detail
//! card) that names the analysed fields and says the result comes from Power BI. When the
//! snapshot carries BIcraft's own estimate (`key_influencers.estimate`, asked for by the viewer or
//! `report.render`), it is drawn in the same frame instead, always under a "BIcraft estimate"
//! badge.
//!
//! Layout from a black-box capture of Desktop 2.159.883.0 showing a synthetic report (sizes in
//! report pixels inside the visual): tabs at 20 px (14.5 px text, the active one semibold and
//! underlined 2 px in the accent at 35 px), the question at 64 px (13 px, a 150 px dropdown),
//! a #F3F3F3 panel from 92 px; in it a list (header 19 px down, rows 64 px apart from 81 px, a
//! 4 px accent bar on the selected row, bubbles of radius 26 placed by size of effect up to
//! 49 % of the width) and a white card from 54.6 % of the width (11 px from the panel's top and
//! 12 px from its bottom) with the sentence, the columns (dark, the selected one in the
//! accent) and a red dashed reference line.

use serde_json::Value;

use crate::canvas::{Anchor, Base, Canvas, Rgb, Text};
use crate::charts::{Bx, fit, font, tw};
use crate::data::{Ctx, arr, clip, field_label, get, nk, sk};
use crate::fmt::{fmt_cell, fmt_val};
use crate::fonts::Font;

const PANEL: Rgb = Rgb(0xF3, 0xF3, 0xF3);
const DARK_BAR: Rgb = Rgb(0x37, 0x46, 0x49);
const OTHER_BUBBLE: Rgb = Rgb(0xD9, 0xD9, 0xD9);
const REFERENCE: Rgb = Rgb(0xE8, 0x5A, 0x5A);
const INK: Rgb = Rgb(0x25, 0x24, 0x23);
const SOFT_INK: Rgb = Rgb(0x60, 0x5E, 0x5C);

/// Top of the grey panel under the tabs and the question.
const PANEL_TOP: f32 = 92.0;

/// What was drawn: `true` = the estimate.
pub fn key_influencers_body(cv: &mut Canvas, v: &Value, b: Bx, ctx: &Ctx<'_>) -> bool {
    let ki = get(v, "key_influencers");
    let fields = arr(get(v, "fields"));
    let label = |i: &Value| -> String {
        i.as_u64()
            .and_then(|i| fields.get(i as usize))
            .map(|f| clip(&field_label(f), 120))
            .unwrap_or_default()
    };
    let target = label(get(ki, "target"));
    let target = if target.is_empty() {
        // a snapshot without the object: the first Target field
        fields
            .iter()
            .find(|f| sk(f, "role") == "Target")
            .map(|f| clip(&field_label(f), 120))
            .unwrap_or_default()
    } else {
        target
    };
    let role_labels = |key: &str, role: &str| -> Vec<String> {
        let listed: Vec<String> = arr(get(ki, key))
            .iter()
            .map(label)
            .filter(|s| !s.is_empty())
            .collect();
        if listed.is_empty() && !ki.is_object() {
            fields
                .iter()
                .filter(|f| sk(f, "role") == role)
                .map(|f| clip(&field_label(f), 120))
                .collect()
        } else {
            listed
        }
    };
    let explain = role_labels("explain_by", "ExplainBy");
    let details = role_labels("details", "Details");
    let est = get(ki, "estimate");
    let has_estimate = sk(est, "status") == "ok";
    let segments_tab = sk(ki, "analysis") == "top_segments";
    let accent = ctx.color(0);

    // tabs: the active one semibold and underlined; the placeholder draws both inactive (grey)
    let mut x = 13.0;
    for (i, (key, text)) in [
        ("ki_tab_influencers", "Key influencers"),
        ("ki_tab_segments", "Top segments"),
    ]
    .iter()
    .enumerate()
    {
        let t = ctx.t(key, text);
        let active = (i == 1) == segments_tab;
        let f = Font::new(&ctx.font, 14.5).weight(if has_estimate && active { 600 } else { 400 });
        let w = ctx.fonts.measure(&f, &t);
        let color = if has_estimate { INK } else { ctx.muted };
        cv.text(ctx.fonts, &Text::new(&t, f, color).base(Base::Central), x, 20.0);
        if active {
            let c = if has_estimate { accent } else { ctx.faint };
            cv.rect(x, 34.0, w, 2.0, c, 1.0);
        }
        x += w + 16.0;
        if x > b.w {
            break;
        }
    }

    // the question, with the goal in a dropdown box
    let is_class = sk(ki, "goal") == "class";
    let goal = match sk(ki, "goal") {
        "decrease" => ctx.t("ki_decrease", "Decrease"),
        "class" => {
            let c = sk(est, "class");
            let c = if c.is_empty() { sk(ki, "class") } else { c };
            if c.is_empty() {
                "…".to_string()
            } else {
                clip(c, 60)
            }
        }
        _ => ctx.t("ki_increase", "Increase"),
    };
    let q = if segments_tab {
        if is_class {
            ctx.t("ki_q_segments_class", "Segments where {target} is most often")
        } else {
            ctx.t("ki_q_segments", "Segments where {target} tends to")
        }
    } else if is_class {
        ctx.t("ki_q_class", "What drives {target} to be")
    } else {
        ctx.t("ki_q", "What drives {target} to")
    };
    let q = q.replace("{target}", if target.is_empty() { "…" } else { &target });
    let qy = 64.0;
    let qf = Font::new(&ctx.font, 13.0);
    let gw = (tw(ctx, &goal, 13.0) + 34.0).max(150.0);
    let qs = ctx.fonts.fit(&qf, &q, (b.w - gw - 30.0).max(10.0));
    let qw = cv.text(ctx.fonts, &Text::new(&qs, qf, INK).base(Base::Central), 13.0, qy);
    let gx = 13.0 + qw + 8.0;
    if gx + 30.0 < b.w {
        let bw = gw.min(b.w - gx - 2.0);
        cv.rect(gx, qy - 12.5, bw, 25.0, Rgb::WHITE, 1.0);
        cv.round_rect(
            gx + 0.5,
            qy - 12.0,
            bw - 1.0,
            24.0,
            0.0,
            None,
            Some((Rgb(0xC8, 0xC6, 0xC4), 1.0, None)),
        );
        let g = fit(ctx, &goal, bw - 30.0, 13.0);
        cv.text(
            ctx.fonts,
            &Text::new(&g, Font::new(&ctx.font, 13.0), INK),
            gx + 5.0,
            qy,
        );
        chevron(cv, gx + bw - 12.0, qy, 3.0, INK);
    }
    if has_estimate {
        badge(cv, ctx, b.w, 12.0);
    }

    // the grey panel
    let area = Bx {
        w: b.w,
        h: (b.h - PANEL_TOP).max(0.0),
    };
    if area.h < 20.0 {
        return has_estimate;
    }
    let Some(mut body) = cv.layer(0.0, PANEL_TOP, area.w, area.h) else {
        return has_estimate;
    };
    body.fill(PANEL, 1.0);
    if has_estimate {
        if segments_tab {
            segments(&mut body, est, area, ctx, &label, &target, is_class);
        } else {
            influencers(
                &mut body,
                est,
                area,
                ctx,
                &label,
                &target,
                is_class,
                sk(ki, "goal"),
            );
        }
    } else {
        placeholder(&mut body, ki, area, ctx, &target, &explain, &details);
    }
    cv.draw(&body, 0.0, PANEL_TOP, 1.0);
    has_estimate
}

/// A small downward chevron centred on (x, y).
fn chevron(cv: &mut Canvas, x: f32, y: f32, r: f32, c: Rgb) {
    cv.polyline(
        &[
            (x - r * 1.6, y - r * 0.6),
            (x, y + r * 0.9),
            (x + r * 1.6, y - r * 0.6),
        ],
        c,
        1.0,
        1.0,
        None,
    );
}

/// The "BIcraft estimate" badge at the top right.
fn badge(cv: &mut Canvas, ctx: &Ctx<'_>, w: f32, y: f32) {
    let text = ctx.t("ki_badge", "BIcraft estimate");
    let f = Font::new(&ctx.font, 10.0).weight(600);
    let tw = ctx.fonts.measure(&f, &text);
    let (bw, bh) = (tw + 12.0, 16.0);
    let bx = w - bw - 8.0;
    if bx < 0.0 {
        return;
    }
    cv.round_rect(bx, y, bw, bh, 8.0, Some((Rgb(0x8a, 0x4b, 0x08), 1.0)), None);
    cv.text(
        ctx.fonts,
        &Text::new(&text, f, Rgb::WHITE)
            .anchor(Anchor::Middle)
            .base(Base::Central),
        bx + bw / 2.0,
        y + bh / 2.0,
    );
}

/// The white detail card on the right of the panel: (x, y, w, h).
fn card(cv: &mut Canvas, b: Bx) -> Option<(f32, f32, f32, f32)> {
    let x = (b.w * 0.546).round();
    let (y, w, h) = (11.0, b.w - x - 16.0, b.h - 23.0);
    if w < 60.0 || h < 30.0 {
        return None;
    }
    // a soft shadow, then the card
    cv.rect(x + 1.0, y + 1.0, w, h, Rgb(0xD0, 0xD0, 0xD0), 1.0);
    cv.rect(x, y, w, h, Rgb::WHITE, 1.0);
    Some((x, y, w, h))
}

/// The placeholder: what the visual analyses on the left of the panel, the note in the card.
fn placeholder(
    cv: &mut Canvas,
    ki: &Value,
    b: Bx,
    ctx: &Ctx<'_>,
    target: &str,
    explain: &[String],
    details: &[String],
) {
    let px = 12.0;
    let right = card(cv, b);
    let lw = right.map_or(b.w, |r| r.0) - 30.0;
    let mut y = 19.0;
    let line = |cv: &mut Canvas, head: &str, text: &str, y: &mut f32| {
        if *y > b.h - px {
            return;
        }
        let hf = Font::new(&ctx.font, 13.0).weight(600);
        let hs = ctx.fonts.fit(&hf, head, lw);
        cv.text(ctx.fonts, &Text::new(&hs, hf, SOFT_INK), 13.0, *y);
        *y += 18.0;
        if *y > b.h - px {
            return;
        }
        let s = fit(ctx, text, lw, 13.0);
        cv.text(
            ctx.fonts,
            &Text::new(&s, Font::new(&ctx.font, 13.0), ctx.muted),
            13.0,
            *y,
        );
        *y += 26.0;
    };
    let none = ctx.t("ki_none", "(none)");
    line(
        cv,
        &ctx.t("ki_analyze", "Analyze"),
        if target.is_empty() { &none } else { target },
        &mut y,
    );
    let ex = if explain.is_empty() {
        none.clone()
    } else {
        explain.join(", ")
    };
    line(cv, &ctx.t("ki_explain_by", "Explain by"), &ex, &mut y);
    if !details.is_empty() {
        line(
            cv,
            &ctx.t("ki_expand_by", "Expand by"),
            &details.join(", "),
            &mut y,
        );
    }
    let goal = match sk(ki, "goal") {
        "decrease" => ctx.t("ki_decrease", "Decrease"),
        "class" if !sk(ki, "class").is_empty() => clip(sk(ki, "class"), 60),
        "class" => ctx.t("ki_class_default", "(the first value)"),
        _ => ctx.t("ki_increase", "Increase"),
    };
    line(cv, &ctx.t("ki_target", "Target"), &goal, &mut y);
    // the note, centred in the card (or the panel when there is no room for a card)
    let (cx, cy, cw, ch) = right.unwrap_or((0.0, 0.0, b.w, b.h));
    let rx = cx + cw / 2.0;
    let rw = cw - 24.0;
    let note = ctx.t("ki_note", "Computed by Power BI — not reproduced by BIcraft");
    let f = Font::new(&ctx.font, 13.0).weight(600);
    let lines = wrap(ctx, &f, &note, rw);
    let est = get(ki, "estimate");
    let second = if est.is_object() {
        let reason = sk(est, "reason");
        format!(
            "{}: {}",
            ctx.t("ki_no_estimate", "No BIcraft estimate"),
            if reason.is_empty() { "?" } else { reason }
        )
    } else {
        ctx.t(
            "ki_hint",
            "BIcraft can draw its own estimate (key_influencers: estimate)",
        )
    };
    let f2 = Font::new(&ctx.font, 11.0);
    let more = wrap(ctx, &f2, &clip(&second, 300), rw);
    let total = lines.len() as f32 * 18.0 + more.len() as f32 * 15.0 + 8.0;
    let mut y = cy + ((ch - total) / 2.0).max(8.0) + 9.0;
    for l in lines.iter().take(4) {
        cv.text(
            ctx.fonts,
            &Text::new(l, f.clone(), SOFT_INK).anchor(Anchor::Middle),
            rx,
            y,
        );
        y += 18.0;
    }
    y += 8.0;
    for l in more.iter().take(4) {
        if y > cy + ch - 6.0 {
            break;
        }
        cv.text(
            ctx.fonts,
            &Text::new(l, f2.clone(), ctx.muted).anchor(Anchor::Middle),
            rx,
            y,
        );
        y += 15.0;
    }
}

/// Greedy word wrap, at most 8 lines.
fn wrap(ctx: &Ctx<'_>, f: &Font, text: &str, w: f32) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace().take(200) {
        let next = if cur.is_empty() {
            word.to_string()
        } else {
            format!("{cur} {word}")
        };
        if ctx.fonts.measure(f, &next) > w && !cur.is_empty() {
            out.push(std::mem::replace(&mut cur, word.to_string()));
            if out.len() >= 8 {
                return out;
            }
        } else {
            cur = next;
        }
    }
    if !cur.is_empty() {
        out.push(ctx.fonts.fit(f, &cur, w.max(1.0)));
    }
    out
}

/// A number with at most `d` decimals in the culture.
fn num(ctx: &Ctx<'_>, x: f64, d: i32) -> String {
    if !x.is_finite() {
        return String::new();
    }
    let k = 10f64.powi(d);
    let r = (x * k).round() / k;
    fmt_val(r, None, &ctx.loc, false)
}

/// How an influencer reads: "Region is North", "Age goes up 10", "Age is 20 to 30".
fn condition(ctx: &Ctx<'_>, i: &Value, field: &str) -> String {
    match sk(i, "kind") {
        "up" => ctx
            .t("ki_goes_up", "{field} goes up {step}")
            .replace("{field}", field)
            .replace("{step}", &num(ctx, nk(i, "step", 1.0), 4)),
        "range" => ctx
            .t("ki_range", "{field} is {from} to {to}")
            .replace("{field}", field)
            .replace("{from}", &num(ctx, nk(i, "from", 0.0), 2))
            .replace("{to}", &num(ctx, nk(i, "to", 0.0), 2)),
        _ => {
            let v = fmt_cell(get(i, "value"), None, &ctx.loc, false);
            ctx.t("ki_is", "{field} is {value}")
                .replace("{field}", field)
                .replace("{value}", &if v.is_empty() { ctx.blank() } else { clip(&v, 80) })
        }
    }
}

/// The key influencers tab: the ranked list (bubbles placed by the size of the effect) on the
/// left, the first influencer's columns in the card.
fn influencers(
    cv: &mut Canvas,
    est: &Value,
    b: Bx,
    ctx: &Ctx<'_>,
    label: &dyn Fn(&Value) -> String,
    target: &str,
    is_class: bool,
    goal: &str,
) {
    let accent = ctx.color(0);
    let list = arr(get(est, "influencers"));
    let right = card(cv, b);
    let left_w = right.map_or(b.w, |r| r.0);
    if list.is_empty() {
        let t = ctx.t("ki_none_found", "No influencer passes the significance test");
        let s = fit(ctx, &t, left_w - 26.0, 13.0);
        cv.text(
            ctx.fonts,
            &Text::new(&s, Font::new(&ctx.font, 13.0), SOFT_INK).anchor(Anchor::Middle),
            left_w / 2.0,
            b.h / 2.0,
        );
        return;
    }
    // header: the condition column and the effect column
    let hf = Font::new(&ctx.font, 12.0);
    cv.text(
        ctx.fonts,
        &Text::new(&ctx.t("ki_when", "If…"), hf.clone(), SOFT_INK),
        13.0,
        19.0,
    );
    let then = if is_class {
        ctx.t(
            "ki_then_class",
            "...the likelihood of {target} being {class} increases by",
        )
        .replace("{class}", sk(est, "class"))
    } else if goal == "decrease" {
        ctx.t("ki_then_down", "…the average of {target} falls by")
    } else {
        ctx.t("ki_then", "…the average of {target} rises by")
    }
    .replace("{target}", target);
    let hx = (left_w * 0.72).round();
    for (k, l) in wrap(ctx, &hf, &then, (left_w - hx - 10.0).max(40.0))
        .iter()
        .take(3)
        .enumerate()
    {
        cv.text(
            ctx.fonts,
            &Text::new(l, hf.clone(), SOFT_INK),
            hx,
            19.0 + 16.0 * k as f32,
        );
    }
    // rows
    let (r, pitch) = (26.0f32, 64.0f32);
    let max_eff = list
        .iter()
        .map(|i| strength(nk(i, "effect", 0.0), is_class))
        .fold(0.0f64, f64::max)
        .max(1e-12);
    let line_x0 = (left_w * 0.31).round();
    let bubble_max = (left_w * 0.90 - r).max(line_x0 + r);
    let bubble_min = (left_w * 0.40).min(bubble_max);
    let mut y = 81.0;
    for (n, i) in list.iter().enumerate() {
        if y + r > b.h {
            break;
        }
        let field = label(get(i, "field"));
        let text = condition(ctx, i, &field);
        let eff = nk(i, "effect", 0.0);
        let value = if is_class {
            format!("{}x", num(ctx, eff, 2))
        } else {
            num(ctx, eff.abs(), 2)
        };
        let selected = n == 0;
        if selected {
            cv.rect(13.0, y - 9.0, 4.0, 18.0, accent, 1.0);
        }
        let tf = Font::new(&ctx.font, 13.0).weight(if selected { 600 } else { 400 });
        let s = ctx.fonts.fit(&tf, &text, (line_x0 - 30.0).max(10.0));
        cv.text(ctx.fonts, &Text::new(&s, tf, INK), 25.0, y);
        // the bubble: further right for a stronger effect
        let share = (strength(eff, is_class) / max_eff).clamp(0.0, 1.0) as f32;
        let bx = bubble_min + (bubble_max - bubble_min) * share;
        let lc = Rgb(0xC8, 0xC6, 0xC4);
        cv.line(line_x0, y, bx - r - 4.0, y, lc, 1.0, None);
        cv.polygon(
            &[
                (bx - r - 4.0, y - 3.5),
                (bx - r + 2.0, y),
                (bx - r - 4.0, y + 3.5),
            ],
            lc,
            1.0,
        );
        let fill = if selected { accent } else { OTHER_BUBBLE };
        cv.circle(bx, y, r, Some((fill, 1.0)), None);
        let vf = Font::new(&ctx.font, 13.0);
        let vs = ctx.fonts.fit(&vf, &value, r * 2.0 - 4.0);
        cv.text(
            ctx.fonts,
            &Text::new(&vs, vf, fill.on()).anchor(Anchor::Middle),
            bx,
            y,
        );
        y += pitch;
    }
    // the card: the first influencer's field
    let Some((x0, y0, w, h)) = right else { return };
    let Some(first) = list.first() else { return };
    let fi = get(first, "field");
    let field = label(fi);
    let sentence = sentence(ctx, first, &field, target, sk(est, "class"), is_class, goal);
    let sf = Font::new(&ctx.font, 13.0);
    cv.text(ctx.fonts, &Text::new("←", sf.clone(), INK), x0 + 16.0, y0 + 20.0);
    let ss = ctx.fonts.fit(&sf, &sentence, (w - 54.0).max(10.0));
    cv.text(ctx.fonts, &Text::new(&ss, sf, INK), x0 + 40.0, y0 + 20.0);
    let Some(f) = arr(get(est, "fields")).iter().find(|f| get(f, "field") == fi) else {
        return;
    };
    let chart = Bx {
        w: (w - 24.0).max(0.0),
        h: (h - 40.0 - 40.0).max(0.0),
    };
    if chart.w < 40.0 || chart.h < 40.0 {
        return;
    }
    if let Some(mut c) = cv.layer(x0 + 12.0, y0 + 40.0, chart.w, chart.h) {
        bar_chart(
            &mut c,
            arr(get(f, "bars")),
            first,
            est,
            chart,
            ctx,
            is_class,
            &field,
        );
        cv.draw(&c, x0 + 12.0, y0 + 40.0, 1.0);
    }
    // the influencers-only check box (not interactive here)
    let cy = y0 + h - 22.0;
    cv.round_rect(
        x0 + 16.5,
        cy - 8.5,
        17.0,
        17.0,
        1.0,
        Some((Rgb::WHITE, 1.0)),
        Some((Rgb(0x8A, 0x88, 0x86), 1.0, None)),
    );
    let t = ctx.t("ki_only_influencers", "Influencers only");
    let ts = fit(ctx, &t, (w - 60.0).max(10.0), 13.0);
    cv.text(
        ctx.fonts,
        &Text::new(&ts, Font::new(&ctx.font, 13.0), INK),
        x0 + 46.0,
        cy,
    );
}

/// How strong an effect is: a ratio's distance from 1, else the size of the change.
fn strength(effect: f64, is_class: bool) -> f64 {
    if !effect.is_finite() {
        return 0.0;
    }
    if is_class {
        (effect - 1.0).max(0.0)
    } else {
        effect.abs()
    }
}

/// The card's sentence, in BIcraft's own words: "Score tends to be higher when Region is North,
/// on average."
fn sentence(
    ctx: &Ctx<'_>,
    i: &Value,
    field: &str,
    target: &str,
    class: &str,
    is_class: bool,
    goal: &str,
) -> String {
    let cond = condition(ctx, i, field);
    let s = if is_class {
        ctx.t("ki_sentence_class", "{target} is {class} more often when {cond}")
            .replace("{class}", class)
    } else if goal == "decrease" {
        ctx.t(
            "ki_sentence_down",
            "{target} tends to be lower when {cond}, on average",
        )
    } else {
        ctx.t(
            "ki_sentence",
            "{target} tends to be higher when {cond}, on average",
        )
    };
    format!("{}.", s.replace("{target}", target).replace("{cond}", &cond))
}

/// Columns of a field's means (rates), the influencer's in the accent colour, the others dark,
/// and the mean of the other rows as a red dashed line.
fn bar_chart(
    cv: &mut Canvas,
    bars: &[Value],
    inf: &Value,
    est: &Value,
    b: Bx,
    ctx: &Ctx<'_>,
    is_class: bool,
    field: &str,
) {
    let px = 12.0;
    let accent = ctx.color(0);
    let n = bars.len().min(12);
    if n == 0 {
        return;
    }
    // the reference: the other rows' mean for a group, the overall one for a slope
    let reference = match sk(inf, "kind") {
        "up" => nk(est, "overall", 0.0),
        _ => nk(inf, "rest_mean", nk(est, "overall", 0.0)),
    };
    let means: Vec<f64> = bars.iter().take(n).map(|x| nk(x, "mean", 0.0)).collect();
    let hi = means.iter().copied().fold(reference, f64::max).max(1e-12);
    let vmin = means.iter().copied().fold(0.0f64, f64::min);
    // round ticks (1, 2 or 5 × 10ⁿ) with the top one above the highest column
    let step = nice((hi - vmin) / 2.0);
    let ticks = ((hi - vmin) / step).ceil().clamp(1.0, 10.0);
    let vmax = vmin + step * ticks;
    let shown = |v: f64| {
        if is_class {
            format!("{}%", num(ctx, v * 100.0, 2))
        } else {
            num(ctx, v, 2)
        }
    };
    // y axis: three ticks
    let (left, top, bottom) = (44.0, 8.0, 36.0);
    let ph = (b.h - top - bottom).max(1.0);
    let y_of = |v: f64| top + ph * ((vmax - v) / (vmax - vmin).max(1e-12)) as f32;
    let tick = |v: f64| {
        if is_class {
            format!("{}%", num(ctx, v * 100.0, 1))
        } else {
            num(ctx, v, 2)
        }
    };
    for k in 0..=(ticks as i32) {
        let v = vmin + step * f64::from(k);
        let y = y_of(v);
        cv.line(left, y, b.w, y, Rgb(0xEA, 0xEA, 0xEA), 1.0, None);
        let s = fit(ctx, &tick(v), left - 6.0, px);
        cv.text(
            ctx.fonts,
            &Text::new(&s, font(ctx, px), SOFT_INK).anchor(Anchor::End),
            left - 6.0,
            y,
        );
    }
    let pw = (b.w - left).max(1.0);
    let slot = pw / n as f32;
    let is_inf = |x: &Value| match sk(inf, "kind") {
        "is" => get(x, "value") == get(inf, "value"),
        "range" => nk(x, "from", f64::NAN) == nk(inf, "from", f64::NAN),
        _ => false,
    };
    for (k, x) in bars.iter().take(n).enumerate() {
        let v = nk(x, "mean", 0.0);
        let (y1, y0) = (y_of(v.max(0.0)), y_of(v.min(0.0)));
        let bw = (slot * 0.76).max(1.0);
        let bx = left + k as f32 * slot + (slot - bw) / 2.0;
        let color = if is_inf(x) { accent } else { DARK_BAR };
        cv.rect(bx, y1, bw, (y0 - y1).max(0.5), color, 1.0);
        let lab = if get(x, "from").is_number() {
            format!(
                "{}–{}",
                num(ctx, nk(x, "from", 0.0), 2),
                num(ctx, nk(x, "to", 0.0), 2)
            )
        } else {
            let s = fmt_cell(get(x, "value"), None, &ctx.loc, false);
            if s.is_empty() { ctx.blank() } else { s }
        };
        let ls = fit(ctx, &lab, slot - 2.0, px);
        cv.text(
            ctx.fonts,
            &Text::new(&ls, font(ctx, px), SOFT_INK).anchor(Anchor::Middle),
            bx + bw / 2.0,
            b.h - bottom + 11.0,
        );
    }
    let ft = fit(ctx, field, pw, px);
    cv.text(
        ctx.fonts,
        &Text::new(&ft, font(ctx, px), SOFT_INK).anchor(Anchor::Middle),
        left + pw / 2.0,
        b.h - 8.0,
    );
    // the reference line
    let ry = y_of(reference);
    cv.line(left, ry, b.w, ry, REFERENCE, 2.0, Some((5.0, 4.0)));
    let key = if sk(inf, "kind") == "up" {
        ctx.t("ki_overall", "Average")
    } else {
        ctx.t("ki_excluding", "Average of the other values")
    };
    let t = format!("{key}: {}", shown(reference));
    let ts = fit(ctx, &t, pw, px);
    cv.text(
        ctx.fonts,
        &Text::new(&ts, font(ctx, px).weight(600), REFERENCE),
        left + 4.0,
        ry - 10.0,
    );
}

/// The top segments tab: one bubble per segment on the left (sized by its rows), the first
/// segment's conditions in the card.
fn segments(
    cv: &mut Canvas,
    est: &Value,
    b: Bx,
    ctx: &Ctx<'_>,
    label: &dyn Fn(&Value) -> String,
    target: &str,
    is_class: bool,
) {
    let accent = ctx.color(0);
    let list = arr(get(est, "segments"));
    let right = card(cv, b);
    let left_w = right.map_or(b.w, |r| r.0);
    if list.is_empty() {
        let t = ctx.t("ki_no_segments", "No segment stands out");
        let s = fit(ctx, &t, left_w - 26.0, 13.0);
        cv.text(
            ctx.fonts,
            &Text::new(&s, Font::new(&ctx.font, 13.0), SOFT_INK).anchor(Anchor::Middle),
            left_w / 2.0,
            b.h / 2.0,
        );
        return;
    }
    let shown = |v: f64| {
        if is_class {
            format!("{}%", num(ctx, v * 100.0, 1))
        } else {
            num(ctx, v, 2)
        }
    };
    let n = list.len().min(6);
    let maxc = list
        .iter()
        .take(n)
        .map(|s| nk(s, "count", 0.0))
        .fold(1.0f64, f64::max);
    let cols = n.clamp(1, 3);
    let rows = n.div_ceil(cols);
    let cw = (left_w - 20.0) / cols as f32;
    let rh = (b.h / rows as f32).max(1.0);
    let rmax = (cw.min(rh - 26.0) / 2.0 - 6.0).max(6.0);
    for (k, s) in list.iter().take(n).enumerate() {
        let (cx, cy) = (
            10.0 + cw * ((k % cols) as f32 + 0.5),
            rh * (k / cols) as f32 + rh / 2.0 - 8.0,
        );
        let r = (rmax * ((nk(s, "count", 0.0) / maxc).sqrt() as f32)).clamp(6.0, rmax);
        let fill = if k == 0 { accent } else { OTHER_BUBBLE };
        cv.circle(cx, cy, r, Some((fill, 1.0)), None);
        let vf = Font::new(&ctx.font, 13.0);
        let vs = ctx.fonts.fit(&vf, &shown(nk(s, "mean", 0.0)), r * 2.0 - 2.0);
        cv.text(
            ctx.fonts,
            &Text::new(&vs, vf, fill.on()).anchor(Anchor::Middle),
            cx,
            cy,
        );
        let name = ctx
            .t("ki_segment", "Segment {n}")
            .replace("{n}", &(k + 1).to_string());
        let ns = fit(ctx, &name, cw, 12.0);
        cv.text(
            ctx.fonts,
            &Text::new(&ns, Font::new(&ctx.font, 12.0), SOFT_INK).anchor(Anchor::Middle),
            cx,
            cy + r + 11.0,
        );
    }
    // the card: the first segment
    let Some((x0, y0, w, h)) = right else { return };
    let Some(s) = list.first() else { return };
    let x = x0 + 16.0;
    let tw_ = (w - 32.0).max(10.0);
    let mut y = y0 + 20.0;
    let head = if is_class {
        ctx.t(
            "ki_seg_head_class",
            "Segment 1: {target} is {class} in {value} of the rows",
        )
        .replace("{class}", sk(est, "class"))
    } else {
        ctx.t("ki_seg_head", "Segment 1: the average of {target} is {value}")
    }
    .replace("{target}", target)
    .replace("{value}", &shown(nk(s, "mean", 0.0)));
    let hf = Font::new(&ctx.font, 13.0).weight(600);
    let hs = ctx.fonts.fit(&hf, &head, tw_);
    cv.text(ctx.fonts, &Text::new(&hs, hf, INK), x, y);
    y += 26.0;
    for c in arr(get(s, "conditions")).iter().take(8) {
        if y > y0 + h - 30.0 {
            break;
        }
        let field = label(get(c, "field"));
        let value = if get(c, "value").is_number() {
            num(ctx, nk(c, "value", 0.0), 4)
        } else {
            let s = fmt_cell(get(c, "value"), None, &ctx.loc, false);
            if s.is_empty() { ctx.blank() } else { clip(&s, 80) }
        };
        let t = match sk(c, "op") {
            "is_not" => ctx.t("ki_is_not", "{field} is not {value}"),
            "at_most" => ctx.t("ki_at_most", "{field} is at most {value}"),
            "above" => ctx.t("ki_above", "{field} is more than {value}"),
            _ => ctx.t("ki_is", "{field} is {value}"),
        }
        .replace("{field}", &field)
        .replace("{value}", &value);
        cv.text(
            ctx.fonts,
            &Text::new(
                &fit(ctx, &format!("• {t}"), tw_, 13.0),
                Font::new(&ctx.font, 13.0),
                INK,
            ),
            x,
            y,
        );
        y += 20.0;
    }
    let share = format!(
        "{} ({}%)",
        ctx.t("ki_seg_rows", "{n} rows")
            .replace("{n}", &num(ctx, nk(s, "count", 0.0), 0)),
        num(ctx, nk(s, "share", 0.0) * 100.0, 1)
    );
    if y < y0 + h - 10.0 {
        cv.text(
            ctx.fonts,
            &Text::new(&fit(ctx, &share, tw_, 13.0), Font::new(&ctx.font, 13.0), SOFT_INK),
            x,
            y + 4.0,
        );
    }
}

/// A round step near `x` (1, 2 or 5 × 10ⁿ), 1 for nonsense.
fn nice(x: f64) -> f64 {
    if !(x.is_finite() && x > 0.0) {
        return 1.0;
    }
    let base = 10f64.powf(x.log10().floor());
    let m = x / base;
    let k = if m <= 1.0 {
        1.0
    } else if m <= 2.0 {
        2.0
    } else if m <= 5.0 {
        5.0
    } else {
        10.0
    };
    k * base
}
