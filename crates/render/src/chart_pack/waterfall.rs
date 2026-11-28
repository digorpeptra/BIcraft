//! BIcraft Waterfall+ (docs/formats/chart-pack.md "Waterfall+"): a running total drawn step by
//! step, each step a bar floating from the total before it to the total after it, with subtotal
//! bars (after each group of the group role, after a row whose subtotal flag is set, or after
//! every N steps) and a final total; vertical or horizontal; steps coloured by sign, by category
//! or by a rule; connectors between bars; value and percent labels; a target drawn as a marker
//! across the final bar with the variance of the total from it. Several measures in the value
//! role become the steps themselves (each measure's total over the rows). Drawn with BIcraft's
//! look (docs/formats/visual-look.md). The Rust twin of
//! `crates/engine/src/chart_pack/waterfall.ts`.

use serde_json::{Value, json};

use super::{
    Out, PAD, cell_label, cell_num, cell_text, est, fit, highlighted, is_fr, line, look_of, opt_bool,
    opt_color, opt_num, options_of, rows_of, text,
};
use crate::data::{Ctx, Dataset};
use crate::look::{Look, dim, mix, part};
use crate::own_hits::{Target, num_text, role_col, tip_line, word};
use crate::spec::{domain, fmt_auto, fmt_fixed, tick_decimals};

/// Most rows read.
pub const MAX_ROWS: usize = 1000;
/// Most bars drawn (steps, subtotals and the total).
pub const MAX_STEPS: usize = 200;
/// Most measures in the value role.
pub const MAX_MEASURES: usize = 8;
/// Values beyond this size are not read (they would make the scale meaningless).
pub const MAX_ABS: f64 = 1e15;
/// The cells before the measures: category, group, subtotal flag, target.
pub const FIRST_MEASURE: usize = 4;
pub const INCREASE: &str = "#1F9D55";
pub const DECREASE: &str = "#D64545";
pub const RULE_ABOVE: &str = "#1C8FCC";
pub const RULE_BELOW: &str = "#BF8700";

/// One bar: a step (`step`), a subtotal (`sub`) or the total (`total`).
struct Item {
    kind: &'static str,
    label: String,
    /// A step's change, a subtotal's or the total's value.
    value: f64,
    start: f64,
    end: f64,
    /// The row of a step (-1: none, several measures or a total).
    row: i64,
    measure: usize,
    /// The step's index among the steps (its colour by category).
    index: usize,
    /// The highlighted part of a step's change (`None`: none).
    hl: Option<f64>,
}

/// A finite number of moderate size.
pub(crate) fn num_of(v: Option<&Value>) -> Option<f64> {
    cell_num(v).filter(|n| n.abs() <= MAX_ABS)
}

/// A number written with a column's whole format string (`f`: scaling commas, quoted and escaped
/// text, sections, as the runtime's `formatNumber` reads them; [`super::gauge::full_format`]), or
/// as BIcraft writes numbers without one. A format `FORMAT` refuses keeps the digits and the
/// currency, a negative value under a one-section format the sign before the formatted size
/// (`-$40.00`), as the runtime's `formatNumber` writes it.
pub(crate) fn fmt_num(v: f64, f: &str, fr: bool) -> String {
    let g: String = f.chars().take(200).collect();
    if g.trim().is_empty() {
        return fmt_auto(v);
    }
    let loc = crate::fmt::Locale::new(if fr { "fr-FR" } else { "en-US" });
    if let Some(s) = super::gauge::full_format(v, &g, &loc) {
        return s;
    }
    if v < 0.0 && !g.contains(';') {
        return format!("-{}", fmt_num(-v, &g, fr));
    }
    let col = crate::fmt::ColFmt {
        fmt: g,
        kind: "measure".into(),
        data_type: "double".into(),
    };
    crate::fmt::fmt_val(v, Some(&col), &loc, false)
}

/// `s` with a plus sign when `v` is above zero and the text has no sign of its own.
pub(crate) fn signed(v: f64, s: String) -> String {
    if v > 0.0 && !s.is_empty() && !s.starts_with('+') && !s.starts_with('-') {
        format!("+{s}")
    } else {
        s
    }
}

/// A change as a signed percentage with one decimal.
pub(crate) fn pct_text(p: f64, fr: bool) -> String {
    let s = fmt_fixed(p * 100.0, 1);
    signed(
        p,
        if fr {
            format!("{}\u{a0}%", s.replacen('.', ",", 1))
        } else {
            format!("{s}%")
        },
    )
}

/// A value axis tick with `d` decimals, its digits grouped and its decimals marked by the
/// culture: `1,000.5` in English, `1 000,5` in French (a no-break space, as the runtime's
/// `formatNumber` groups them).
pub(crate) fn tick_text(t: f64, d: u32, fr: bool) -> String {
    let s = fmt_fixed(t, d);
    if !fr {
        return s;
    }
    s.chars()
        .map(|c| match c {
            ',' => '\u{a0}',
            '.' => ',',
            c => c,
        })
        .collect()
}

/// Whether a subtotal flag cell is set: true, a number other than 0, or a text other than "",
/// "0", "false", "no", "non".
pub(crate) fn flag_set(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x.is_finite() && x != 0.0),
        Some(Value::String(s)) => {
            let t = s.trim().to_lowercase();
            !matches!(t.as_str(), "" | "0" | "false" | "no" | "non")
        }
        _ => false,
    }
}

pub(crate) fn format_at(input: &Value, k: usize) -> String {
    input
        .get("formats")
        .and_then(|l| l.get(k))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// The highlighted part of measure `m` of row `i` (`None`: not highlighted).
fn hl_at(input: &Value, i: usize, m: usize) -> Option<f64> {
    if !highlighted(input) {
        return None;
    }
    num_of(input.get("highlights")?.get(i)?.as_array()?.get(m))
}

/// The steps, subtotals and total of the input.
fn items_of(input: &Value, o: &Value, fr: bool) -> Vec<Item> {
    let rows = rows_of(input, MAX_ROWS);
    let mut nm = 0usize;
    for r in &rows {
        nm = nm.max(r.len().saturating_sub(FIRST_MEASURE).min(MAX_MEASURES));
    }
    let names: Vec<String> = (0..nm)
        .map(|m| {
            input
                .get("names")
                .and_then(|l| l.get(m))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        })
        .collect();
    let every = opt_num(o, "subtotalEvery", 0.0, 100.0, 0.0).floor() as usize;
    let sub_name = if fr { "Sous-total" } else { "Subtotal" };
    let mut items: Vec<Item> = Vec::new();
    let mut run = 0.0;
    let mut steps = 0usize;
    let room = |items: &Vec<Item>| items.len() < MAX_STEPS - 1;
    let sub = |items: &mut Vec<Item>, label: String, run: f64| {
        if items.last().is_some_and(|l| l.kind == "step") && items.len() < MAX_STEPS - 1 {
            items.push(Item {
                kind: "sub",
                label,
                value: run,
                start: 0.0,
                end: run,
                row: -1,
                measure: 0,
                index: usize::MAX,
                hl: None,
            });
        }
    };
    let mut step = |items: &mut Vec<Item>, run: &mut f64, label: String, v: f64, row: i64, m: usize, hl| {
        items.push(Item {
            kind: "step",
            label,
            value: v,
            start: *run,
            end: *run + v,
            row,
            measure: m,
            index: steps,
            hl,
        });
        *run += v;
        steps += 1;
        steps
    };
    if nm == 1 {
        let mut group: Option<String> = None;
        let mut group_label = String::new();
        for (i, r) in rows.iter().enumerate() {
            if !room(&items) {
                break;
            }
            let Some(v) = num_of(r.get(FIRST_MEASURE)) else {
                continue;
            };
            let g = cell_text(r.get(1));
            if group.as_ref().is_some_and(|x| *x != g) {
                sub(&mut items, group_label.clone(), run);
            }
            if !room(&items) {
                break;
            }
            let n = step(
                &mut items,
                &mut run,
                cell_label(input, 0, r.first()),
                v,
                i as i64,
                0,
                hl_at(input, i, 0),
            );
            group = Some(g);
            group_label = cell_label(input, 1, r.get(1));
            if flag_set(r.get(2)) || (every > 0 && n.is_multiple_of(every)) {
                sub(&mut items, sub_name.to_string(), run);
            }
        }
    } else if nm > 1 {
        for (m, name) in names.iter().enumerate() {
            if !room(&items) {
                break;
            }
            let (mut sum, mut any, mut hs, mut lit) = (0.0, false, 0.0, false);
            for (i, r) in rows.iter().enumerate() {
                if let Some(v) = num_of(r.get(FIRST_MEASURE + m)) {
                    sum += v;
                    any = true;
                }
                if let Some(h) = hl_at(input, i, m) {
                    hs += h;
                    lit = true;
                }
            }
            if !any {
                continue;
            }
            let n = step(&mut items, &mut run, name.clone(), sum, -1, m, lit.then_some(hs));
            if every > 0 && n.is_multiple_of(every) {
                sub(&mut items, sub_name.to_string(), run);
            }
        }
    }
    if steps > 0 && opt_bool(o, "showTotal", true) {
        if items.last().is_some_and(|l| l.kind == "sub") {
            items.pop();
        }
        items.push(Item {
            kind: "total",
            label: "Total".into(),
            value: run,
            start: 0.0,
            end: run,
            row: -1,
            measure: 0,
            index: usize::MAX,
            hl: None,
        });
    }
    items
}

/// The fill of an item.
fn fill_of(k: &Look, o: &Value, it: &Item) -> String {
    let total = opt_color(o, "totalFill", &k.color(0));
    match it.kind {
        "total" => return total,
        "sub" => return mix(&total, "#FFFFFF", 0.35),
        _ => {}
    }
    match o.get("colorBy").and_then(Value::as_str) {
        Some("category") => k.color(it.index),
        Some("rule") => {
            let t = opt_num(o, "ruleValue", -MAX_ABS, MAX_ABS, 0.0);
            if it.value >= t {
                opt_color(o, "ruleAbove", RULE_ABOVE)
            } else {
                opt_color(o, "ruleBelow", RULE_BELOW)
            }
        }
        _ => {
            if it.value >= 0.0 {
                opt_color(o, "increaseFill", INCREASE)
            } else {
                opt_color(o, "decreaseFill", DECREASE)
            }
        }
    }
}

/// The label of an item for the label mode (`value`, `percent`, `both`, `none`).
fn label_of(input: &Value, it: &Item, mode: &str, fr: bool) -> String {
    if mode == "none" {
        return String::new();
    }
    let f = format_at(input, FIRST_MEASURE + it.measure);
    let v = if it.kind == "step" {
        signed(it.value, fmt_num(it.value, &f, fr))
    } else {
        fmt_num(it.value, &f, fr)
    };
    let p = if it.kind == "step" && it.start != 0.0 {
        pct_text(it.value / it.start.abs(), fr)
    } else {
        String::new()
    };
    if mode == "percent" {
        return if p.is_empty() { v } else { p };
    }
    if mode == "both" && !p.is_empty() {
        return format!("{v} ({p})");
    }
    v
}

fn item_hit(i: usize, it: &Item) -> Value {
    json!({"item": i, "kind": it.kind, "label": it.label, "value": it.value, "start": it.start,
           "end": it.end, "measure": it.measure})
}

/// Lays out `{rows: [[category, group, subtotal, target, value…]], names, formats, options}` in
/// a `w`×`h` visual.
pub fn layout(input: &Value, w: f64, h: f64) -> Out {
    let o = options_of(input);
    let k = look_of(input);
    let fr = is_fr(input);
    let mut out = Out::default();
    let items = items_of(input, o, fr);
    let Some(last) = items.last() else {
        return out;
    };
    let mut target = 0.0;
    let mut has_target = false;
    for r in rows_of(input, MAX_ROWS) {
        if let Some(t) = num_of(r.get(3)) {
            target += t;
            has_target = true;
        }
    }
    let size = opt_num(o, "fontSize", 8.0, 20.0, k.value);
    let mode = o
        .get("labelMode")
        .and_then(Value::as_str)
        .filter(|m| matches!(*m, "percent" | "both" | "none"))
        .unwrap_or("value");
    let horizontal = o.get("orientation").and_then(Value::as_str) == Some("horizontal");
    let connectors = opt_bool(o, "showConnectors", true);
    let lit = highlighted(input);
    let mut levels: Vec<f64> = Vec::new();
    for it in &items {
        levels.push(it.start);
        levels.push(it.end);
    }
    if has_target {
        levels.push(target);
    }
    let dom = domain(&levels, true, None, None, 5.0);
    let span = if dom.hi - dom.lo > 0.0 {
        dom.hi - dom.lo
    } else {
        1.0
    };
    let dec = tick_decimals(dom.step);
    let ticks: Vec<String> = dom.ticks.iter().map(|t| tick_text(*t, dec, fr)).collect();
    let labels: Vec<String> = items.iter().map(|it| label_of(input, it, mode, fr)).collect();
    k.push_card(&mut out.prims, w, h);
    let fin = last.end;
    // the variance of the total from the target, in the header
    let mut header = String::new();
    if has_target {
        let f = format_at(input, FIRST_MEASURE);
        let diff = fin - target;
        header = format!(
            "{}{}",
            if fr {
                "Écart à la cible : "
            } else {
                "vs target: "
            },
            signed(diff, fmt_num(diff, &f, fr))
        );
        if target != 0.0 {
            header.push_str(&format!(" ({})", pct_text(diff / target.abs(), fr)));
        }
    }
    let top = PAD + if header.is_empty() { 0.0 } else { k.name + 8.0 };
    if !header.is_empty() {
        let s = fit(&header, k.name, w - 2.0 * PAD);
        if !s.is_empty() {
            out.hit(
                -1,
                json!({"kind": "variance", "value": fin - target, "total": fin, "target": target}),
            );
            out.prims
                .push(text(w - PAD, PAD + k.name / 2.0 + 1.0, s, k.name, k.label, "end"));
        }
    }
    let n = items.len();
    let pills = if k.pills { 10.0 } else { 0.0 };
    let lw = |s: &str| est(s, size) + pills;
    let target_fill = opt_color(o, "targetFill", k.label);
    let target_w = if k.minimal() { 2.0 } else { 3.0 };
    if !horizontal {
        let mut gutter: f64 = 0.0;
        for t in &ticks {
            gutter = gutter.max(est(t, k.tick));
        }
        let x0 = PAD + gutter + 6.0;
        let x1 = w - PAD;
        let y0 = top + if mode == "none" { 4.0 } else { size + 8.0 };
        let y1 = h - PAD - k.tick - 6.0;
        let pw = x1 - x0;
        let ph = y1 - y0;
        if !(pw >= 20.0 && ph >= 20.0) {
            return out;
        }
        let y_of = |v: f64| y1 - ((v - dom.lo) / span) * ph;
        for (t, s) in dom.ticks.iter().zip(&ticks) {
            let y = y_of(*t);
            out.prims.push(line(x0, y, x1, y, k.grid, 1.0));
            out.prims
                .push(text(x0 - 4.0, y, s.clone(), k.tick, k.text, "end"));
        }
        let slot = pw / n as f64;
        let bw = slot * if k.minimal() { 0.6 } else { 0.68 };
        for (i, (it, s)) in items.iter().zip(&labels).enumerate() {
            let cx = x0 + slot * (i as f64 + 0.5);
            let a = y_of(it.start);
            let b = y_of(it.end);
            let y = a.min(b);
            let bh = (a - b).abs().max(1.0);
            let fill = fill_of(&k, o, it);
            let sq = if it.kind == "step" {
                ""
            } else if it.value >= 0.0 {
                "b"
            } else {
                "t"
            };
            out.hit(it.row, item_hit(i, it));
            let mark = k.bar(cx - bw / 2.0, y, bw, bh, &fill, sq);
            let step = it.kind == "step";
            out.prims.push(if lit && step { dim(mark) } else { mark });
            if let Some(hv) = part(it.value, it.hl.filter(|_| lit && step)) {
                let c = y_of(it.start + hv);
                out.hit(it.row, item_hit(i, it));
                out.prims
                    .push(k.bar(cx - bw / 2.0, a.min(c), bw, (a - c).abs().max(1.0), &fill, ""));
            }
            if connectors && i + 1 < n {
                let ly = y_of(it.end);
                out.prims
                    .push(line(cx + bw / 2.0, ly, cx + slot - bw / 2.0, ly, k.axis, 1.0));
            }
            if !s.is_empty() && lw(s) <= slot {
                if it.value >= 0.0 {
                    k.value_label(&mut out.prims, cx, y, s.clone(), 'n', 4.0, size);
                } else {
                    k.value_label(&mut out.prims, cx, y + bh, s.clone(), 's', 4.0, size);
                }
            }
            let name = fit(&it.label, k.tick, slot - 2.0);
            if !name.is_empty() {
                out.prims
                    .push(text(cx, y1 + 4.0 + k.tick / 2.0, name, k.tick, k.text, "middle"));
            }
            if has_target && i == n - 1 {
                let ty = y_of(target);
                out.hit(-1, json!({"kind": "target", "value": target, "total": fin}));
                out.prims.push(line(
                    cx - bw / 2.0 - 4.0,
                    ty,
                    cx + bw / 2.0 + 4.0,
                    ty,
                    &target_fill,
                    target_w,
                ));
            }
        }
        let zy = y_of(0.0);
        out.prims.push(line(x0, zy, x1, zy, k.axis, 1.0));
        return out;
    }
    // horizontal: the steps top to bottom, the values along x
    let mut gutter: f64 = 0.0;
    for it in &items {
        gutter = gutter.max(est(&it.label, k.tick));
    }
    gutter = gutter.min(w * 0.3);
    let mut right: f64 = 0.0;
    for s in &labels {
        right = right.max(lw(s));
    }
    right = right.min(w * 0.25);
    let x0 = PAD + gutter + 6.0;
    let x1 = w - PAD - if right > 0.0 { right + 6.0 } else { 0.0 };
    let y0 = top + 2.0;
    let y1 = h - PAD - k.tick - 6.0;
    let pw = x1 - x0;
    let ph = y1 - y0;
    if !(pw >= 20.0 && ph >= 20.0) {
        return out;
    }
    let x_of = |v: f64| x0 + ((v - dom.lo) / span) * pw;
    for (t, s) in dom.ticks.iter().zip(&ticks) {
        let x = x_of(*t);
        out.prims.push(line(x, y0, x, y1, k.grid, 1.0));
        let half = est(s, k.tick) / 2.0;
        out.prims.push(text(
            x.max(half).min(w - half),
            y1 + 4.0 + k.tick / 2.0,
            s.clone(),
            k.tick,
            k.text,
            "middle",
        ));
    }
    let slot = ph / n as f64;
    let bh = slot * if k.minimal() { 0.6 } else { 0.68 };
    for (i, (it, s)) in items.iter().zip(&labels).enumerate() {
        let cy = y0 + slot * (i as f64 + 0.5);
        let a = x_of(it.start);
        let b = x_of(it.end);
        let x = a.min(b);
        let bw = (a - b).abs().max(1.0);
        let fill = fill_of(&k, o, it);
        let sq = if it.kind == "step" {
            ""
        } else if it.value >= 0.0 {
            "l"
        } else {
            "r"
        };
        out.hit(it.row, item_hit(i, it));
        let mark = k.bar(x, cy - bh / 2.0, bw, bh, &fill, sq);
        let step = it.kind == "step";
        out.prims.push(if lit && step { dim(mark) } else { mark });
        if let Some(hv) = part(it.value, it.hl.filter(|_| lit && step)) {
            let c = x_of(it.start + hv);
            out.hit(it.row, item_hit(i, it));
            out.prims
                .push(k.bar(a.min(c), cy - bh / 2.0, (a - c).abs().max(1.0), bh, &fill, ""));
        }
        if connectors && i + 1 < n {
            let lx = x_of(it.end);
            out.prims
                .push(line(lx, cy + bh / 2.0, lx, cy + slot - bh / 2.0, k.axis, 1.0));
        }
        // a rise's label after its bar's end; a fall's before its bar, inside the plot so it
        // keeps clear of the category names, else after the bar's other end
        let room = |from: f64| from + 4.0 + lw(s) <= w - 2.0;
        if !s.is_empty() && slot >= size + 2.0 {
            if it.value >= 0.0 && room(x + bw) {
                k.value_label(&mut out.prims, x + bw, cy, s.clone(), 'e', 4.0, size);
            } else if it.value < 0.0 && x - 4.0 - lw(s) >= x0 + 2.0 {
                k.value_label(&mut out.prims, x, cy, s.clone(), 'w', 4.0, size);
            } else if it.value < 0.0 && room(x + bw) {
                k.value_label(&mut out.prims, x + bw, cy, s.clone(), 'e', 4.0, size);
            }
        }
        let name = fit(&it.label, k.tick, gutter + 2.0);
        if !name.is_empty() && slot >= k.tick {
            out.prims.push(text(x0 - 6.0, cy, name, k.tick, k.text, "end"));
        }
        if has_target && i == n - 1 {
            let tx = x_of(target);
            out.hit(-1, json!({"kind": "target", "value": target, "total": fin}));
            out.prims.push(line(
                tx,
                cy - bh / 2.0 - 4.0,
                tx,
                cy + bh / 2.0 + 4.0,
                &target_fill,
                target_w,
            ));
        }
    }
    let zx = x_of(0.0);
    out.prims.push(line(zx, y0, zx, y1, k.axis, 1.0));
    out
}

/// What each hit selects and shows, as `waterfall_visual.ts` `point` does: a step its row (none
/// with several measures), a subtotal, the total, the target and the variance only a tooltip.
pub(crate) fn targets(input: &Value, out: &Out, ds: &Dataset, ctx: &Ctx<'_>) -> Vec<Target> {
    let fr = ctx.ui_lang.to_ascii_lowercase().starts_with("fr");
    let names: Vec<String> = input
        .get("names")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .take(MAX_MEASURES)
                .map(|x| x.as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default();
    let measure_cols: Vec<usize> = ds
        .cols
        .iter()
        .enumerate()
        .filter(|(_, c)| c.role.eq_ignore_ascii_case("values"))
        .map(|(i, _)| i)
        .collect();
    let first = measure_cols.first().copied();
    let value_name = |m: usize| {
        names
            .get(m)
            .filter(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| word(ctx, "Value", "Valeur"))
    };
    let num = |h: &Value, k: &str| h.get(k).and_then(Value::as_f64).unwrap_or(f64::NAN);
    let mut list = Vec::new();
    for h in &out.hits {
        let Some(prim) = h
            .get("prim")
            .and_then(Value::as_u64)
            .and_then(|p| usize::try_from(p).ok())
        else {
            continue;
        };
        let row = h
            .get("row")
            .and_then(Value::as_i64)
            .and_then(|r| usize::try_from(r).ok());
        let m = h
            .get("measure")
            .and_then(Value::as_u64)
            .and_then(|x| usize::try_from(x).ok())
            .unwrap_or(0);
        let col = measure_cols.get(m).copied().or(first);
        let value = |x: f64| num_text(ds, col, x, ctx);
        let kind = h.get("kind").and_then(Value::as_str).unwrap_or("");
        let mut tip: Vec<(String, String)> = Vec::new();
        match kind {
            "step" => {
                if let Some(r) = row {
                    tip.extend(tip_line(ds, role_col(ds, "category"), r, ctx));
                    tip.extend(tip_line(ds, role_col(ds, "group"), r, ctx));
                } else {
                    let label = h.get("label").and_then(Value::as_str).unwrap_or("").to_string();
                    tip.push((word(ctx, "Step", "Étape"), label));
                }
                let v = num(h, "value");
                tip.push((value_name(m), signed(v, value(v))));
                tip.push((word(ctx, "Running total", "Cumul"), value(num(h, "end"))));
                let start = num(h, "start");
                if start != 0.0 && start.is_finite() {
                    tip.push((word(ctx, "Change", "Variation"), pct_text(v / start.abs(), fr)));
                }
            }
            "sub" | "total" => {
                let label = h.get("label").and_then(Value::as_str).unwrap_or("").to_string();
                let name = if kind == "total" {
                    "Total".to_string()
                } else {
                    word(ctx, "Subtotal", "Sous-total")
                };
                tip.push((name, label));
                tip.push((value_name(0), value(num(h, "value"))));
            }
            "target" | "variance" => {
                let t = num(h, "target");
                let t = if t.is_finite() { t } else { num(h, "value") };
                tip.push((word(ctx, "Target", "Cible"), value(t)));
                let total = num(h, "total");
                tip.push(("Total".to_string(), value(total)));
                let diff = total - t;
                let mut s = signed(diff, value(diff));
                if t != 0.0 {
                    s.push_str(&format!(" ({})", pct_text(diff / t.abs(), fr)));
                }
                tip.push((word(ctx, "Variance", "Écart"), s));
            }
            _ => continue,
        }
        list.push(Target {
            rows: row.map(|r| vec![r]).unwrap_or_default(),
            prims: vec![prim],
            at: Vec::new(),
            tip,
            faded: false,
            send: None,
        });
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(input: Value) -> Vec<(String, f64)> {
        items_of(&input, options_of(&input), false)
            .into_iter()
            .map(|i| (format!("{}:{}", i.kind, i.label), i.value))
            .collect()
    }

    /// Subtotals come after a flagged row and every N steps, never twice in a row nor first, and
    /// the total replaces a subtotal that would end the steps.
    #[test]
    fn subtotals_follow_flags_and_counts() {
        let rows = json!([
            ["a", null, "x", null, 1],
            ["b", null, null, null, 2],
            ["c", null, null, null, 3]
        ]);
        let got = kinds(json!({"rows": rows, "options": {"subtotalEvery": 1}}));
        let want: Vec<(String, f64)> = [
            ("step:a", 1.0),
            ("sub:Subtotal", 1.0),
            ("step:b", 2.0),
            ("sub:Subtotal", 3.0),
            ("step:c", 3.0),
            ("total:Total", 6.0),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        assert_eq!(got, want);
        let got = kinds(json!({"rows": rows, "options": {"showTotal": false}}));
        assert_eq!(got.len(), 4, "{got:?}");
        assert!(flag_set(Some(&json!("Oui"))) && !flag_set(Some(&json!("non"))));
        assert!(!flag_set(Some(&json!(0))) && flag_set(Some(&json!(-1))) && !flag_set(None));
    }

    /// The steps stop at [`MAX_STEPS`] bars and at [`MAX_MEASURES`] measures.
    #[test]
    fn caps_hold() {
        let many: Vec<Value> = (0..5000)
            .map(|i| json!([format!("c{i}"), null, null, null, 1]))
            .collect();
        assert_eq!(kinds(json!({"rows": many})).len(), MAX_STEPS);
        let cells: Vec<Value> = (0..20)
            .map(|i| if i < 4 { Value::Null } else { json!(1) })
            .collect();
        assert_eq!(kinds(json!({"rows": [cells]})).len(), MAX_MEASURES + 1);
    }

    #[test]
    fn numbers_take_their_format() {
        assert_eq!(fmt_num(1234.5, "", false), "1,234.50");
        assert_eq!(fmt_num(1234.5, "#,0", false), "1,235");
        // scaling commas, at the end of the digits or after the decimals, and quoted text
        assert_eq!(fmt_num(1_234_567.0, "#,##0,\"K\"", false), "1,235K");
        assert_eq!(fmt_num(7_433_915.0, "$#,0.00,,\"M\"", false), "$7.43M");
        assert_eq!(fmt_num(-7_433_915.0, "$#,0.0,,\"M\"", false), "-$7.4M");
        assert_eq!(signed(3.0, "3".into()), "+3");
        assert_eq!(signed(-3.0, "-3".into()), "-3");
        assert_eq!(pct_text(-0.4, false), "-40.0%");
    }
}
