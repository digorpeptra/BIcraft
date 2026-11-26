//! BIcraft Line / Area+ (docs/formats/chart-pack.md "Line / Area+"): lines or areas by series
//! on a continuous date axis (or a numeric or category axis), with a band between two measures
//! coloured by which one is above, shading beyond a threshold, an area fill formatted apart from
//! its line, labels on all, the last, or the lowest and highest points, line colours and widths
//! by series or by rule, a dashed stretch from a forecast date on, and series named at the end
//! of their line instead of in a legend. The Rust twin of
//! `crates/engine/src/chart_pack/linearea.ts`; its date ticks take the steps of
//! [`crate::timeaxis`] (`steps_of`), labelled with estimated widths as the TypeScript does.

use std::cmp::Ordering;
use std::collections::HashMap;

use serde_json::{Value, json};

use super::{
    MS_PER_DAY, Out, PAD, cell_label, cell_num, cell_text, circle, compact, dashes, est, fit, highlighted,
    hl_num, is_fr, legend, line, look_of, opt_bool, opt_color, opt_enum, opt_num, opt_text, options_of, poly,
    rows_of, text,
};
use crate::look::{dim, mix, text_w};
use crate::spec::{Domain, Prim, civil_from_days, date_text, domain, inside, parse_iso, r2, tick_decimals};
use crate::timeaxis::{MAX_TICKS, STEPS, Unit, approx_days, steps_of};

pub const MAX_ROWS: usize = 20_000;
pub const MAX_SERIES: usize = 12;
/// Most points of one series (distinct axis values).
pub const MAX_POINTS: usize = 2000;
/// Most points with their own hit target (above it, each line is one target).
pub const MAX_HIT_POINTS: usize = 3000;
/// Most labels of one series in the `all` mode (above it, none).
pub const MAX_ALL_LABELS: usize = 60;

// ---------------------------------------------------------------------------------------------
// Date ticks over OLE serials (days)

/// The OLE serial of 1970-01-01.
const EPOCH: f64 = 25_569.0;
const MONTHS_EN: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTHS_FR: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc.",
];

fn civil_of(serial: f64) -> (i64, i64, i64) {
    civil_from_days(serial as i64 - EPOCH as i64)
}

fn month(m: i64, fr: bool) -> &'static str {
    let l = if fr { &MONTHS_FR } else { &MONTHS_EN };
    usize::try_from(m - 1)
        .ok()
        .and_then(|i| l.get(i))
        .copied()
        .unwrap_or("")
}

/// A day tick: the column's date format when it writes at most 12 characters, else ISO;
/// `short`, the day and the month's short name ("Jan 5", "5 janv.").
fn day_text(x: f64, fmt: &str, fr: bool, short: bool) -> String {
    let day = x.floor();
    if short {
        let (_, m, d) = civil_of(day);
        return if fr {
            format!("{d} {}", month(m, true))
        } else {
            format!("{} {d}", month(m, false))
        };
    }
    let ms = (day - EPOCH) * MS_PER_DAY;
    let l = if fmt.is_empty() {
        None
    } else {
        crate::fmt::date_label(ms, fmt, if fr { "fr" } else { "en" })
    };
    match l {
        Some(l) if l.chars().count() <= 12 => l,
        _ => date_text(ms, false),
    }
}

fn tick_label(u: Unit, x: f64, fmt: &str, fr: bool, short: bool) -> String {
    let (y, m, _) = civil_of(x.floor());
    match u {
        Unit::Year => y.to_string(),
        Unit::Month => format!("{} {y}", month(m, fr)),
        Unit::Day | Unit::Week => day_text(x, fmt, fr, short),
        Unit::Minute | Unit::Hour => {
            let whole = (x + 0.5).floor();
            if (x - whole).abs() < 1e-7 {
                day_text(whole, fmt, fr, short)
            } else {
                date_text((x - EPOCH) * MS_PER_DAY, true)
                    .chars()
                    .skip(11)
                    .collect()
            }
        }
    }
}

/// The ticks of a date axis from serial `lo` to `hi` over [`x0`, `x1`]: the finest step whose
/// labels keep 8 px apart (steps finer than the data's smallest `gap` skipped).
#[allow(clippy::too_many_arguments)]
fn time_ticks(
    lo: f64,
    hi: f64,
    gap: f64,
    x0: f64,
    x1: f64,
    size: f64,
    fmt: &str,
    fr: bool,
) -> Vec<(f64, String)> {
    let s = (x1 - x0) / (hi - lo);
    let pos = |t: f64| x0 + (t - lo) * s;
    let mut last: Vec<(f64, String)> = Vec::new();
    for (u, k) in STEPS {
        let d = approx_days(u, k);
        if d < gap * 0.9 || (hi - lo) / d > MAX_TICKS as f64 {
            continue;
        }
        let ts = steps_of(u, k, lo, hi);
        if ts.is_empty() {
            continue;
        }
        let shorts: &[bool] = if matches!(u, Unit::Month | Unit::Year) {
            &[false]
        } else {
            &[false, true]
        };
        for &short in shorts {
            let ticks: Vec<(f64, String)> = ts
                .iter()
                .map(|t| (pos(*t), tick_label(u, *t, fmt, fr, short)))
                .collect();
            let mut widest: f64 = 0.0;
            let mut room = x1 - x0;
            for (i, t) in ticks.iter().enumerate() {
                widest = widest.max(est(&t.1, size));
                if i > 0
                    && let Some(prev) = ticks.get(i - 1)
                {
                    room = room.min(t.0 - prev.0);
                }
            }
            if room >= widest + 8.0 {
                return ticks;
            }
            last = ticks;
        }
    }
    match last.into_iter().next() {
        Some(t) => vec![t],
        None => vec![(x0, day_text(lo, fmt, fr, false))],
    }
}

// ---------------------------------------------------------------------------------------------
// The layout

#[derive(Debug, Clone)]
struct Lp {
    key: f64,
    v: Option<f64>,
    v2: Option<f64>,
    hl: Option<f64>,
    row: usize,
    /// The point's own value (`v` is its top when stacked).
    own: Option<f64>,
    /// Stacked: the top of the series below at this point (0 for the first).
    base: f64,
}

fn path_p(points: &[(f64, f64)], stroke: &str, width: f64) -> Prim {
    Prim::Path {
        points: points.iter().map(|(x, y)| (r2(*x), r2(*y))).collect(),
        stroke: stroke.to_string(),
        width: r2(width),
    }
}

fn add(a: Option<f64>, b: f64) -> Option<f64> {
    Some(a.unwrap_or(0.0) + b)
}

/// Shades between `a` and `b` (per point, `None` breaks a run) over the points' `xs`: each run
/// where `a - b` keeps its sign is one polygon, split where the two cross, in `color_of(sign)`
/// (`None`: not shaded).
fn shade(
    p: &mut Vec<Prim>,
    xs: &[f64],
    a: &[Option<f64>],
    b: &[Option<f64>],
    sy: &dyn Fn(f64) -> f64,
    color_of: &dyn Fn(i32) -> Option<String>,
    opacity: f64,
) {
    let mut top: Vec<(f64, f64)> = Vec::new();
    let mut bot: Vec<(f64, f64)> = Vec::new();
    let mut sign = 0;
    let flush = |p: &mut Vec<Prim>, top: &mut Vec<(f64, f64)>, bot: &mut Vec<(f64, f64)>, sign: &mut i32| {
        let c = if *sign != 0 && top.len() >= 2 {
            color_of(*sign)
        } else {
            None
        };
        if let Some(c) = c {
            let mut shape = top.clone();
            shape.extend(bot.iter().rev().copied());
            p.push(poly(&shape, &c, opacity));
        }
        top.clear();
        bot.clear();
        *sign = 0;
    };
    let (mut px, mut pa, mut pb) = (0.0, 0.0, 0.0);
    for (i, &x) in xs.iter().enumerate() {
        let (Some(va), Some(vb)) = (a.get(i).copied().flatten(), b.get(i).copied().flatten()) else {
            flush(p, &mut top, &mut bot, &mut sign);
            continue;
        };
        let d = va - vb;
        let s = if d > 0.0 {
            1
        } else if d < 0.0 {
            -1
        } else {
            0
        };
        if !top.is_empty() && s != 0 && sign != 0 && s != sign {
            let d0 = pa - pb;
            let t = d0 / (d0 - d);
            let xc = px + (x - px) * t;
            let yc = sy(pa + (va - pa) * t);
            top.push((xc, yc));
            bot.push((xc, yc));
            flush(p, &mut top, &mut bot, &mut sign);
            top.push((xc, yc));
            bot.push((xc, yc));
        }
        top.push((x, sy(va)));
        bot.push((x, sy(vb)));
        if s != 0 {
            sign = s;
        }
        px = x;
        pa = va;
        pb = vb;
    }
    flush(p, &mut top, &mut bot, &mut sign);
}

/// Draws a stretch of a line, solid or dashed, in its colour or the rule's (`pens`: line
/// colour, rule colour, width), noting the series' first primitive in `fp`.
fn emit(
    p: &mut Vec<Prim>,
    fp: &mut Option<usize>,
    pp: &[(f64, f64)],
    (dashed, ruled): (bool, bool),
    pens: (&str, &str, f64),
) {
    if fp.is_none() {
        *fp = Some(p.len());
    }
    let stroke = if ruled { pens.1 } else { pens.0 };
    if dashed {
        dashes(p, pp, stroke, pens.2, 6.0, 4.0);
    } else {
        p.push(path_p(pp, stroke, pens.2));
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Kind {
    Time,
    Number,
    Category,
}

/// Lays out `{rows: [[axis, series, value, value2]], formats, options, today?}` in a `w`×`h`
/// visual.
pub fn layout(input: &Value, w: f64, h: f64) -> Out {
    let o = options_of(input);
    let k = look_of(input);
    let fr = is_fr(input);
    let mut out = Out::default();
    let rows = rows_of(input, MAX_ROWS);
    // the axis: dates, numbers, else categories
    let axis_type = opt_enum(o, "axisType", &["auto", "category"], "auto");
    let (mut any, mut dates, mut nums) = (false, true, true);
    for r in &rows {
        let Some(c) = r.first().filter(|c| !c.is_null()) else {
            continue;
        };
        any = true;
        if c.as_str().and_then(parse_iso).is_none() {
            dates = false;
        }
        if cell_num(Some(c)).is_none() {
            nums = false;
        }
    }
    let kind = if axis_type == "category" || !any {
        Kind::Category
    } else if dates {
        Kind::Time
    } else if nums {
        Kind::Number
    } else {
        Kind::Category
    };
    let mut cats: Vec<String> = Vec::new();
    let mut cat_ix: HashMap<String, usize> = HashMap::new();
    let mut series: Vec<String> = Vec::new();
    let mut ser_ix: HashMap<String, usize> = HashMap::new();
    let mut pts: Vec<Vec<Lp>> = Vec::new();
    let mut key_ix: Vec<HashMap<u64, usize>> = Vec::new();
    let lit = highlighted(input);
    for (i, r) in rows.iter().enumerate() {
        let c = r.first();
        let key = match kind {
            Kind::Time => match c.and_then(Value::as_str).and_then(parse_iso) {
                Some(ms) => ms + 0.0,
                None => continue,
            },
            Kind::Number => match cell_num(c) {
                Some(n) => n + 0.0,
                None => continue,
            },
            Kind::Category => {
                let t = cell_text(c);
                match cat_ix.get(&t) {
                    Some(&ci) => ci as f64,
                    None => {
                        if cats.len() >= MAX_POINTS {
                            continue;
                        }
                        cat_ix.insert(t, cats.len());
                        cats.push(cell_label(input, 0, c));
                        (cats.len() - 1) as f64
                    }
                }
            }
        };
        let sk = cell_text(r.get(1));
        let si = match ser_ix.get(&sk) {
            Some(&si) => si,
            None => {
                if series.len() >= MAX_SERIES {
                    continue;
                }
                ser_ix.insert(sk, series.len());
                series.push(cell_label(input, 1, r.get(1)));
                pts.push(Vec::new());
                key_ix.push(HashMap::new());
                series.len() - 1
            }
        };
        let (Some(l), Some(ix)) = (pts.get_mut(si), key_ix.get_mut(si)) else {
            continue;
        };
        let pi = match ix.get(&key.to_bits()) {
            Some(&pi) => pi,
            None => {
                if l.len() >= MAX_POINTS {
                    continue;
                }
                ix.insert(key.to_bits(), l.len());
                l.push(Lp {
                    key,
                    v: None,
                    v2: None,
                    hl: None,
                    row: i,
                    own: None,
                    base: 0.0,
                });
                l.len() - 1
            }
        };
        let Some(q) = l.get_mut(pi) else { continue };
        if let Some(v) = cell_num(r.get(2)) {
            q.v = add(q.v, v);
        }
        if let Some(v2) = cell_num(r.get(3)) {
            q.v2 = add(q.v2, v2);
        }
        if let Some(hv) = hl_num(input, i) {
            q.hl = add(q.hl, hv);
        }
    }
    for l in &mut pts {
        l.sort_by(|a, b| a.key.partial_cmp(&b.key).unwrap_or(Ordering::Equal));
    }
    // stacked: each series' values on the ones before it at the same axis value (rises on the
    // rises, falls under the falls), no second measure; labels keep each point's own value
    let stacked = opt_bool(o, "stacked", false) && pts.len() > 1;
    let (mut ups, mut downs): (HashMap<u64, f64>, HashMap<u64, f64>) = (HashMap::new(), HashMap::new());
    for q in pts.iter_mut().flatten() {
        q.own = q.v;
        if !stacked {
            continue;
        }
        q.v2 = None;
        let Some(v) = q.v else { continue };
        let m = if v >= 0.0 { &mut ups } else { &mut downs };
        let b = m.get(&q.key.to_bits()).copied().unwrap_or(0.0);
        q.base = b;
        q.v = Some(b + v);
        q.hl = q.hl.map(|h| b + h);
        m.insert(q.key.to_bits(), b + v);
    }
    let mut vals: Vec<f64> = Vec::new();
    let mut has_v2 = false;
    let mut count = 0usize;
    for q in pts.iter().flatten() {
        if let Some(v) = q.v {
            vals.push(v);
            count += 1;
        }
        if let Some(v2) = q.v2 {
            vals.push(v2);
            has_v2 = true;
        }
    }
    if count == 0 {
        return out;
    }
    let area = opt_bool(o, "area", false);
    let area_opacity = opt_num(o, "areaOpacity", 0.0, 100.0, 25.0) / 100.0;
    let area_fill = opt_color(o, "areaFill", "");
    let line_color = opt_color(o, "lineColor", "");
    let width = opt_num(o, "lineWidth", 0.5, 8.0, k.line_width());
    let markers = opt_bool(o, "markers", false);
    let label_mode = opt_enum(o, "labelMode", &["last", "minmax", "all", "none"], "last");
    let font_size = opt_num(o, "fontSize", 8.0, 20.0, k.value);
    let end_labels = opt_bool(o, "endLabels", true);
    let show_legend = opt_bool(o, "showLegend", true);
    let band = opt_bool(o, "band", true);
    let band_up = opt_color(o, "bandUp", "#0E9F84");
    let band_down = opt_color(o, "bandDown", "#E23D4F");
    let band_opacity = opt_num(o, "bandOpacity", 5.0, 100.0, 30.0) / 100.0;
    let threshold = opt_bool(o, "threshold", false);
    let tv = opt_num(o, "thresholdValue", -1e15, 1e15, 0.0);
    let threshold_side = opt_enum(o, "thresholdSide", &["above", "below"], "above");
    let threshold_color = opt_color(o, "thresholdColor", "#E23D4F");
    let color_rule = opt_enum(o, "colorRule", &["none", "above", "below"], "none");
    let rv = opt_num(o, "ruleValue", -1e15, 1e15, 0.0);
    let rule_color = opt_color(o, "ruleColor", "#E23D4F");
    let emphasize = opt_text(o, "emphasize", 200);
    let forecast_from = opt_text(o, "forecastFrom", 40);
    let y_zero = opt_bool(o, "yZero", area || stacked);
    if threshold {
        vals.push(tv);
    }
    let yd = domain(&vals, y_zero, None, None, 5.0);
    let named = !(series.len() == 1 && series.first().is_some_and(String::is_empty));
    let colors: Vec<String> = (0..series.len())
        .map(|i| {
            if !line_color.is_empty() && !named {
                line_color.clone()
            } else {
                k.color(i)
            }
        })
        .collect();
    k.push_card(&mut out.prims, w, h);
    let mut top = PAD;
    if named && !end_labels && show_legend {
        legend(&mut out.prims, &k, &series, &colors, PAD, PAD + 4.0, w - PAD);
        top = PAD + 20.0;
    }
    top += if label_mode != "none" {
        font_size + 6.0
    } else {
        6.0
    };
    let y_labels: Vec<String> = yd
        .ticks
        .iter()
        .map(|t| compact(*t, Some(tick_decimals(yd.step))))
        .collect();
    let mut lw: f64 = 0.0;
    for l in &y_labels {
        lw = lw.max(est(l, k.tick));
    }
    let x0 = PAD + lw.max(10.0) + 6.0;
    // the end labels' room
    let ends: Vec<String> = series.iter().map(|s| fit(s, k.name, 110.0)).collect();
    let mut end_w: f64 = 0.0;
    if named && end_labels {
        for s in &ends {
            end_w = end_w.max(est(s, k.name));
        }
        end_w = if end_w > 0.0 {
            (end_w + 20.0).min(130.0)
        } else {
            0.0
        };
    }
    let x1 = w - PAD - if end_w > 0.0 { end_w } else { 4.0 };
    let y0 = top;
    let y1 = h - PAD - 18.0;
    if x1 - x0 < 20.0 || y1 - y0 < 20.0 {
        return out;
    }
    let sy = |v: f64| y1 - ((v - yd.lo) / (yd.hi - yd.lo)) * (y1 - y0);
    // the x scale
    let (mut lo, mut hi, mut gap) = (0.0, 1.0, 1.0);
    let mut xd: Option<Domain> = None;
    match kind {
        Kind::Time => {
            let mut keys: Vec<f64> = pts.iter().flatten().map(|q| q.key / MS_PER_DAY + EPOCH).collect();
            keys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            lo = keys.first().copied().unwrap_or(0.0);
            hi = keys.last().copied().unwrap_or(1.0);
            gap = f64::INFINITY;
            for w2 in keys.windows(2) {
                if let [a, b] = w2
                    && b - a > 1e-9
                {
                    gap = gap.min(b - a);
                }
            }
            if hi.partial_cmp(&lo) != Some(Ordering::Greater) {
                lo -= 1.0;
                hi += 1.0;
            }
            if !gap.is_finite() {
                gap = 1.0;
            }
        }
        Kind::Number => {
            let keys: Vec<f64> = pts.iter().flatten().map(|q| q.key).collect();
            xd = Some(domain(&keys, false, None, None, 5.0));
        }
        Kind::Category => {}
    }
    let n = cats.len();
    let sx = |key: f64| -> f64 {
        match (kind, &xd) {
            (Kind::Time, _) => x0 + ((key / MS_PER_DAY + EPOCH - lo) * (x1 - x0)) / (hi - lo),
            (_, Some(d)) => x0 + ((key - d.lo) / (d.hi - d.lo)) * (x1 - x0),
            _ if n <= 1 => (x0 + x1) / 2.0,
            _ => x0 + (key * (x1 - x0)) / (n - 1) as f64,
        }
    };
    let p = &mut out.prims;
    // grid, y ticks, x ticks
    for (t, l) in yd.ticks.iter().zip(&y_labels) {
        let yy = sy(*t);
        p.push(line(x0, yy, x1, yy, k.grid, 1.0));
        p.push(text(x0 - 4.0, yy, l.clone(), k.tick, k.text, "end"));
    }
    let xt: Vec<(f64, String)> = match (kind, &xd) {
        (Kind::Time, _) => {
            let fmt: String = input
                .get("formats")
                .and_then(|l| l.get(0))
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(200)
                .collect();
            time_ticks(lo, hi, gap, x0, x1, k.tick, &fmt, fr)
        }
        (_, Some(d)) => d
            .ticks
            .iter()
            .map(|t| (sx(*t), compact(*t, Some(tick_decimals(d.step)))))
            .collect(),
        _ => cats
            .iter()
            .enumerate()
            .map(|(i, c)| (sx(i as f64), c.clone()))
            .collect(),
    };
    let mut widest: f64 = 0.0;
    for t in &xt {
        widest = widest.max(est(&t.1, k.tick));
    }
    let room = if xt.len() > 1 {
        (x1 - x0) / (xt.len() - 1) as f64
    } else {
        x1 - x0
    };
    let every = if kind == Kind::Time {
        1
    } else {
        ((widest + 6.0) / room.max(1.0)).ceil().max(1.0) as usize
    };
    for (i, (x, l)) in xt.iter().enumerate() {
        if i % every != 0 {
            continue;
        }
        let s = if kind == Kind::Category {
            fit(l, k.tick, (room * every as f64 - 4.0).max(0.0))
        } else {
            l.clone()
        };
        if s.is_empty() {
            continue;
        }
        p.push(line(*x, y1, *x, y1 + 3.0, k.axis, 1.0));
        p.push(text(
            inside(*x, &s, k.tick, w),
            y1 + 10.0,
            s,
            k.tick,
            k.text,
            "middle",
        ));
    }
    p.push(line(x0, y1, x1, y1, k.axis, 1.0));
    // the forecast cutoff (dates only, inside the data's range)
    let mut cutoff: Option<f64> = None;
    if kind == Kind::Time && !forecast_from.is_empty() {
        let ms = if forecast_from == "today" {
            input
                .get("today")
                .and_then(Value::as_f64)
                .filter(|x| x.is_finite())
        } else {
            parse_iso(&forecast_from)
        };
        if let Some(ms) = ms {
            let sr = ms / MS_PER_DAY + EPOCH;
            if sr > lo && sr <= hi {
                cutoff = Some(sx(ms));
            }
        }
    }
    // emphasis: one series wider, the others muted
    let hot: Option<usize> = if emphasize.is_empty() {
        None
    } else {
        series.iter().position(|s| *s == emphasize)
    };
    let xs: Vec<Vec<f64>> = pts
        .iter()
        .map(|l| l.iter().map(|q| sx(q.key)).collect())
        .collect();
    let empty: Vec<f64> = Vec::new();
    let xs_of = |si: usize| xs.get(si).unwrap_or(&empty);
    let color_of = |si: usize| colors.get(si).cloned().unwrap_or_else(|| k.color(0));
    // shading beyond the threshold and between the two measures
    if threshold {
        for (si, l) in pts.iter().enumerate() {
            let a: Vec<Option<f64>> = l.iter().map(|q| q.v).collect();
            let b: Vec<Option<f64>> = l.iter().map(|_| Some(tv)).collect();
            let above = threshold_side == "above";
            let tc = threshold_color.clone();
            shade(
                p,
                xs_of(si),
                &a,
                &b,
                &sy,
                &|s| (if above { s > 0 } else { s < 0 }).then(|| tc.clone()),
                0.25,
            );
        }
    }
    if has_v2 && band {
        for (si, l) in pts.iter().enumerate() {
            let a: Vec<Option<f64>> = l.iter().map(|q| q.v).collect();
            let b: Vec<Option<f64>> = l.iter().map(|q| q.v2).collect();
            shade(
                p,
                xs_of(si),
                &a,
                &b,
                &sy,
                &|s| Some(if s > 0 { band_up.clone() } else { band_down.clone() }),
                band_opacity,
            );
        }
    }
    // areas, under each run of the line
    let base = sy(0f64.max(yd.lo).min(yd.hi));
    if area {
        for (si, l) in pts.iter().enumerate() {
            let fill = if area_fill.is_empty() {
                color_of(si)
            } else {
                area_fill.clone()
            };
            let mut run: Vec<(f64, f64)> = Vec::new();
            // stacked: down to the series below, else to zero
            let mut low: Vec<(f64, f64)> = Vec::new();
            let flush = |p: &mut Vec<Prim>, run: &mut Vec<(f64, f64)>, low: &mut Vec<(f64, f64)>| {
                if let (true, Some(&first), Some(&last)) = (run.len() >= 2, run.first(), run.last()) {
                    let mut shape = run.clone();
                    if stacked {
                        shape.extend(low.iter().rev());
                    } else {
                        shape.push((last.0, base));
                        shape.push((first.0, base));
                    }
                    let m = poly(&shape, &fill, area_opacity);
                    p.push(if lit { dim(m) } else { m });
                }
                run.clear();
                low.clear();
            };
            for (i, q) in l.iter().enumerate() {
                match q.v {
                    None => flush(p, &mut run, &mut low),
                    Some(v) => {
                        let x = xs_of(si).get(i).copied().unwrap_or(x0);
                        run.push((x, sy(v)));
                        low.push((x, sy(q.base.max(yd.lo).min(yd.hi))));
                    }
                }
            }
            flush(p, &mut run, &mut low);
        }
    }
    // the second measure's lines
    if has_v2 {
        for (si, l) in pts.iter().enumerate() {
            let c = mix(&color_of(si), &k.surface, 0.45);
            let wd = (width * 0.6).max(1.0);
            let mut run: Vec<(f64, f64)> = Vec::new();
            for (i, q) in l.iter().enumerate() {
                match q.v2 {
                    None => {
                        if run.len() >= 2 {
                            p.push(path_p(&run, &c, wd));
                        }
                        run.clear();
                    }
                    Some(v2) => run.push((xs_of(si).get(i).copied().unwrap_or(x0), sy(v2))),
                }
            }
            if run.len() >= 2 {
                p.push(path_p(&run, &c, wd));
            }
        }
    }
    // the lines: runs of points with a value, split where the rule or the forecast changes
    let mut first_prim: Vec<Option<usize>> = vec![None; pts.len()];
    for (si, l) in pts.iter().enumerate() {
        let muted = lit || hot.is_some_and(|h| h != si);
        let t = if lit { 0.6 } else { 0.55 };
        let base_c = color_of(si);
        let c = if muted {
            mix(&base_c, &k.surface, t)
        } else {
            base_c.clone()
        };
        let wd = match hot {
            Some(h) if h == si => width * 1.6,
            Some(_) => (width * 0.7).max(1.0),
            None => width,
        };
        let rc = if muted {
            mix(&rule_color, &k.surface, t)
        } else {
            rule_color.clone()
        };
        let xsi = xs_of(si);
        let mut fp: Option<usize> = None;
        let pens = (c.as_str(), rc.as_str(), wd);
        let mut runs: Vec<Vec<usize>> = Vec::new();
        let mut run: Vec<usize> = Vec::new();
        for (i, q) in l.iter().enumerate() {
            if q.v.is_none() {
                runs.push(std::mem::take(&mut run));
            } else {
                run.push(i);
            }
        }
        runs.push(run);
        for run in runs {
            if let [only] = run.as_slice()
                && let Some(v) = l.get(*only).and_then(|q| q.v)
                && !markers
            {
                if fp.is_none() {
                    fp = Some(p.len());
                }
                p.push(circle(
                    xsi.get(*only).copied().unwrap_or(x0),
                    sy(v),
                    wd / 2.0 + 1.5,
                    &c,
                    1.0,
                ));
            }
            let mut cur: Vec<(f64, f64)> = Vec::new();
            let (mut cur_dashed, mut cur_ruled) = (false, false);
            for pair in run.windows(2) {
                let [a, b] = pair else { continue };
                let (Some(&ax), Some(&bx)) = (xsi.get(*a), xsi.get(*b)) else {
                    continue;
                };
                let (Some(va), Some(vb)) = (l.get(*a).and_then(|q| q.v), l.get(*b).and_then(|q| q.v)) else {
                    continue;
                };
                let mut ts: Vec<f64> = vec![0.0, 1.0];
                if let Some(cx) = cutoff
                    && ax < cx
                    && bx > cx
                {
                    ts.push((cx - ax) / (bx - ax));
                }
                if color_rule != "none" && (va - rv) * (vb - rv) < 0.0 {
                    ts.push((rv - va) / (vb - va));
                }
                ts.sort_by(|x, y| x.partial_cmp(y).unwrap_or(Ordering::Equal));
                for tw in ts.windows(2) {
                    let [ta, tb] = tw else { continue };
                    let (ta, tb) = (*ta, *tb);
                    if tb.partial_cmp(&ta) != Some(Ordering::Greater) {
                        continue;
                    }
                    let tm = (ta + tb) / 2.0;
                    let dashed = cutoff.is_some_and(|cx| ax + (bx - ax) * tm >= cx);
                    let mv = va + (vb - va) * tm;
                    let ruled = match color_rule {
                        "above" => mv > rv,
                        "below" => mv < rv,
                        _ => false,
                    };
                    let pa = (ax + (bx - ax) * ta, sy(va + (vb - va) * ta));
                    let pb = (ax + (bx - ax) * tb, sy(va + (vb - va) * tb));
                    if !cur.is_empty() && (dashed != cur_dashed || ruled != cur_ruled) {
                        emit(p, &mut fp, &cur, (cur_dashed, cur_ruled), pens);
                        cur.clear();
                    }
                    if cur.is_empty() {
                        cur.push(pa);
                        cur_dashed = dashed;
                        cur_ruled = ruled;
                    }
                    cur.push(pb);
                }
            }
            if cur.len() >= 2 {
                emit(p, &mut fp, &cur, (cur_dashed, cur_ruled), pens);
            }
        }
        if let Some(slot) = first_prim.get_mut(si) {
            *slot = fp;
        }
        // another visual's selection: the highlighted parts over the muted line
        // (a lone highlighted point is a dot, as a lone point of the line is)
        if lit {
            let hflush = |p: &mut Vec<Prim>, hr: &mut Vec<(f64, f64)>| {
                if hr.len() >= 2 {
                    p.push(path_p(hr, &base_c, wd));
                } else if let [(x, y)] = hr.as_slice()
                    && !markers
                {
                    p.push(circle(*x, *y, wd / 2.0 + 1.5, &base_c, 1.0));
                }
                hr.clear();
            };
            let mut hr: Vec<(f64, f64)> = Vec::new();
            for (i, q) in l.iter().enumerate() {
                match (q.hl, q.v) {
                    (Some(hv), Some(_)) => hr.push((xsi.get(i).copied().unwrap_or(x0), sy(hv))),
                    _ => hflush(p, &mut hr),
                }
            }
            hflush(p, &mut hr);
        }
    }
    if threshold && tv >= yd.lo && tv <= yd.hi {
        let ty = sy(tv);
        dashes(p, &[(x0, ty), (x1, ty)], &threshold_color, 1.5, 6.0, 4.0);
        p.push(text(
            x1 - 2.0,
            (ty - 8.0).max(y0 + 4.0),
            compact(tv, None),
            k.tick,
            k.text,
            "end",
        ));
    }
    if let Some(cx) = cutoff {
        dashes(p, &[(cx, y0), (cx, y1)], k.axis, 1.0, 4.0, 3.0);
        let s = fit(if fr { "Prévision" } else { "Forecast" }, k.tick, x1 - cx - 6.0);
        if !s.is_empty() {
            p.push(text(cx + 4.0, y0 + 6.0, s, k.tick, k.text, "start"));
        }
    }
    // the points' targets: markers (or invisible ones), else each line
    let hit_points = count <= MAX_HIT_POINTS;
    let mut hits: Vec<(i64, usize, Value)> = Vec::new();
    for (si, l) in pts.iter().enumerate() {
        if !hit_points {
            if let (Some(Some(fp)), Some(q)) = (first_prim.get(si), l.first()) {
                hits.push((q.row as i64, *fp, json!({"series": si, "line": true})));
            }
            continue;
        }
        for (i, q) in l.iter().enumerate() {
            let Some(v) = q.v else { continue };
            let x = xs_of(si).get(i).copied().unwrap_or(x0);
            let y = sy(v);
            let c = color_of(si);
            let m = if markers {
                k.dot(x, y, (width + 1.0).max(3.0), &c, 1.0)
            } else {
                circle(x, y, 4.0, &c, 0.0)
            };
            let fade = lit && q.hl.is_none();
            let mut extra = json!({"series": si, "point": i});
            if fade {
                extra["fade"] = json!(true);
            }
            hits.push((q.row as i64, p.len(), extra));
            p.push(if fade { dim(m) } else { m });
        }
    }
    // value labels
    if label_mode != "none" {
        for (si, l) in pts.iter().enumerate() {
            let (mut last, mut lo2, mut hi2): (Option<usize>, Option<usize>, Option<usize>) =
                (None, None, None);
            let mut nv = 0;
            let val = |i: Option<usize>| i.and_then(|i| l.get(i)).and_then(|q| q.own);
            for (i, q) in l.iter().enumerate() {
                let Some(v) = q.own else { continue };
                nv += 1;
                last = Some(i);
                if val(lo2).is_none_or(|m| v < m) {
                    lo2 = Some(i);
                }
                if val(hi2).is_none_or(|m| v > m) {
                    hi2 = Some(i);
                }
            }
            let mut at: Vec<(usize, char)> = Vec::new();
            match label_mode {
                "last" => at.extend(last.map(|i| (i, 'n'))),
                "minmax" => {
                    if let Some(hi2) = hi2 {
                        at.push((hi2, 'n'));
                        if let Some(lo2) = lo2.filter(|l| *l != hi2) {
                            at.push((lo2, 's'));
                        }
                    }
                }
                _ if nv <= MAX_ALL_LABELS => {
                    at.extend(
                        l.iter()
                            .enumerate()
                            .filter(|(_, q)| q.v.is_some())
                            .map(|(i, _)| (i, 'n')),
                    );
                }
                _ => {}
            }
            for (i, side) in at {
                let (Some(v), Some(top)) = (val(Some(i)), l.get(i).and_then(|q| q.v)) else {
                    continue;
                };
                let s = compact(v, None);
                let half = est(&s, font_size) / 2.0 + if k.pills { 5.0 } else { 0.0 };
                let x = xs_of(si).get(i).copied().unwrap_or(x0).min(w - half).max(half);
                k.value_label(p, x, sy(top), s, side, 6.0, font_size);
            }
        }
    }
    // the series' names at the end of their lines
    if named && end_labels && end_w > 0.0 {
        let mut ends2: Vec<(usize, f64)> = Vec::new();
        for (si, l) in pts.iter().enumerate() {
            if let Some(v) = l.iter().rev().find_map(|q| q.v) {
                ends2.push((si, sy(v)));
            }
        }
        ends2.sort_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        let gap_y = k.name + 3.0;
        for i in 1..ends2.len() {
            let prev = ends2.get(i - 1).map_or(f64::NEG_INFINITY, |e| e.1);
            if let Some(e) = ends2.get_mut(i) {
                e.1 = e.1.max(prev + gap_y);
            }
        }
        for i in (0..ends2.len()).rev() {
            let lim = if i + 1 == ends2.len() {
                y1
            } else {
                ends2.get(i + 1).map_or(y1, |e| e.1 - gap_y)
            };
            if let Some(e) = ends2.get_mut(i) {
                e.1 = e.1.min(lim);
            }
        }
        for (si, y) in ends2 {
            let yy = y.max(k.name / 2.0 + 1.0);
            let s = fit(ends.get(si).map_or("", String::as_str), k.name, w - x1 - 16.0);
            if s.is_empty() {
                continue;
            }
            p.push(circle(x1 + 8.0, yy, 3.5, &color_of(si), 1.0));
            p.push(text_w(x1 + 15.0, yy, s, k.name, k.label, "start", 0.0, 600));
        }
    }
    for (row, prim, extra) in hits {
        out.hit_at(row, prim, extra);
    }
    out
}
