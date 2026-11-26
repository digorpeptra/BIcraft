//! BIcraft Calendar Heatmap (docs/formats/chart-pack.md "Calendar heatmap"): one cell per day
//! of each year of the data (or of its months only), weeks as columns and weekdays (Monday or
//! Sunday first) as rows, coloured from the low to the high colour by the day's value; months and
//! weekdays labelled in English or French. The Rust twin of `crates/engine/src/chart_pack/calendar.ts`.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::{
    MS_PER_DAY, Out, PAD, cell_num, est, highlighted, hl_num, is_fr, lerp_color, look_of, opt_bool,
    opt_color, options_of, rows_of, text,
};
use crate::look::{Look, mix, text_w};
use crate::spec::{Prim, civil_from_days, days_from_civil, fmt_auto, parse_iso, r2};

pub const MAX_ROWS: usize = 4000;
/// Most years drawn (the latest ones).
pub const MAX_YEARS: i64 = 4;

const MONTHS_EN: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTHS_FR: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc.",
];
const WEEKDAYS_EN: [&str; 3] = ["Mon", "Wed", "Fri"];
const WEEKDAYS_FR: [&str; 3] = ["lun.", "mer.", "ven."];

/// Monday 0 … Sunday 6 of a day number (1970-01-01 was a Thursday).
pub fn weekday(z: i64) -> i64 {
    (z + 3).rem_euclid(7)
}

fn day_of(v: Option<&Value>) -> Option<i64> {
    let ms = match v {
        Some(Value::String(s)) => parse_iso(s),
        Some(Value::Number(n)) => n.as_f64().filter(|x| x.is_finite()),
        _ => None,
    }?;
    (ms.abs() <= 8.64e15).then(|| (ms / MS_PER_DAY).floor() as i64)
}

/// What the modern drawing of the days needs.
struct Grid<'a> {
    first_year: i64,
    last_year: i64,
    weeks: i64,
    sunday_first: bool,
    show_legend: bool,
    vmin: f64,
    vmax: f64,
    low: &'a str,
    high: &'a str,
    empty: &'a str,
    fr: bool,
    /// The highlighted part of each day (`None`: nothing is highlighted).
    parts: Option<&'a HashMap<i64, f64>>,
}

/// Day `z`'s cell (`b`: its box), coloured `c`, for its first row `row` and value `v`. When the
/// visual is highlighted (`parts`), the full cell is dimmed and the day's highlighted part fills
/// it from its foot in proportion (docs/formats/bicraft-visuals.md "Highlights").
#[allow(clippy::too_many_arguments)]
fn day_cell(
    out: &mut Out,
    lk: &Look,
    (x, y, w, h): (f64, f64, f64, f64),
    c: &str,
    corner: f64,
    (z, row, v): (i64, usize, f64),
    parts: Option<&HashMap<i64, f64>>,
) {
    out.hit(row as i64, json!({"day": z, "value": v}));
    let tile = lk.tile(x, y, w, h, c, corner, false);
    let Some(parts) = parts else {
        out.prims.push(tile);
        return;
    };
    out.prims.push(crate::look::dim(tile));
    if let Some(p) = crate::look::part(v, parts.get(&z).copied()) {
        let ph = h * p / v;
        out.hit(row as i64, json!({"day": z, "value": v}));
        out.prims.push(lk.tile(x, y + h - ph, w, ph, c, corner, false));
    }
}

/// A polyline (a month's separator).
fn path_p(points: &[(f64, f64)], stroke: &str, width: f64) -> Prim {
    Prim::Path {
        points: points.iter().map(|(x, y)| (r2(*x), r2(*y))).collect(),
        stroke: stroke.to_string(),
        width: r2(width),
    }
}

/// Modern and Bold: the year's weeks fill the visual's width and its weekdays its height (cells
/// at most 48 px and no more than twice as wide as tall or 1.6 times as tall as wide), centred;
/// each year headed by its number and its months, the months parted by a line along their first
/// days, three weekdays named, and a legend of five steps under the grid (`calendar.ts`
/// `modern`).
#[allow(clippy::too_many_arguments)]
fn modern(
    out: &mut Out,
    lk: &Look,
    g: &Grid<'_>,
    days: &HashMap<i64, (f64, usize)>,
    row: &dyn Fn(i64) -> i64,
    span: &dyn Fn(i64) -> (i64, i64),
    w: f64,
    h: f64,
) {
    let years = (g.last_year - g.first_year + 1) as f64;
    let tick = lk.tick;
    let (months, wds) = if g.fr {
        (MONTHS_FR, WEEKDAYS_FR)
    } else {
        (MONTHS_EN, WEEKDAYS_EN)
    };
    let mut gutter = est(&g.last_year.to_string(), lk.name) + 10.0;
    for s in wds {
        gutter = gutter.max(est(s, tick - 1.0) + 10.0);
    }
    let hdr = tick + 10.0;
    let year_gap = 12.0;
    let legend_h = if g.show_legend { 30.0 } else { 0.0 };
    let aw = w - 2.0 * PAD - gutter;
    let ah = h - 2.0 * PAD - legend_h - (years - 1.0) * year_gap;
    let mut cw = (aw / g.weeks as f64).min(48.0);
    let mut ch = ((ah / years - hdr) / 7.0).min(48.0);
    cw = cw.min(ch * 2.0);
    ch = ch.min(cw * 1.6);
    if !(cw >= 3.0 && ch >= 3.0) {
        return;
    }
    let m = cw.min(ch);
    let gap = if m >= 12.0 {
        3.0
    } else if m >= 6.0 {
        2.0
    } else {
        1.0
    };
    let corner = ((m - gap) * 0.3).min(6.0);
    let gw = g.weeks as f64 * cw;
    let block_h = hdr + 7.0 * ch;
    let total = years * block_h + (years - 1.0) * year_gap;
    let x0 = PAD + gutter + ((aw - gw) / 2.0).max(0.0);
    let y0 = PAD + ((h - 2.0 * PAD - legend_h - total) / 2.0).max(0.0);
    lk.push_card(&mut out.prims, w, h);
    for y in g.first_year..=g.last_year {
        let top = y0 + (y - g.first_year) as f64 * (block_h + year_gap);
        let ty = top + hdr;
        let (z0, z1) = span(y);
        let start = z0 - row(z0);
        out.prims.push(text_w(
            x0 - 8.0,
            top + hdr / 2.0,
            y.to_string(),
            lk.name,
            lk.label,
            "end",
            0.0,
            600,
        ));
        for (mo, label) in (1i64..).zip(months) {
            let m0 = days_from_civil(y, mo, 1);
            let m1 = if mo == 12 {
                days_from_civil(y, 12, 31)
            } else {
                days_from_civil(y, mo + 1, 1) - 1
            };
            if m1 < z0 || m0 > z1 {
                continue;
            }
            let col = (m0.max(z0) - start).div_euclid(7) as f64;
            if cw * 4.0 >= est(label, tick) {
                out.prims.push(text(
                    x0 + col * cw,
                    top + hdr / 2.0,
                    label.to_string(),
                    tick,
                    lk.text,
                    "start",
                ));
            }
            if m0 > z0 && m >= 8.0 {
                // the line between the month before and this one, along the gaps (cells of 8 px or more)
                let r = row(m0) as f64;
                let xb = x0 + col * cw - gap / 2.0;
                let yb = ty + 7.0 * ch - gap / 2.0;
                let pts: Vec<(f64, f64)> = if r == 0.0 {
                    vec![(xb, ty - gap / 2.0), (xb, yb)]
                } else {
                    let yr = ty + r * ch - gap / 2.0;
                    vec![(xb + cw, ty - gap / 2.0), (xb + cw, yr), (xb, yr), (xb, yb)]
                };
                out.prims.push(path_p(&pts, lk.axis, 1.5));
            }
        }
        if ch >= 9.0 {
            for (k, label) in wds.iter().enumerate() {
                let r = 2.0 * k as f64 + if g.sunday_first { 1.0 } else { 0.0 };
                out.prims.push(text(
                    x0 - 8.0,
                    ty + r * ch + (ch - gap) / 2.0,
                    label.to_string(),
                    tick - 1.0,
                    lk.text,
                    "end",
                ));
            }
        }
        for z in z0..=z1 {
            let x = x0 + (z - start).div_euclid(7) as f64 * cw;
            let yy = ty + row(z) as f64 * ch;
            match days.get(&z) {
                Some(&(v, r)) => {
                    let t = if g.vmax > g.vmin {
                        (v - g.vmin) / (g.vmax - g.vmin)
                    } else {
                        1.0
                    };
                    let c = lerp_color(g.low, g.high, t);
                    day_cell(
                        out,
                        lk,
                        (x, yy, cw - gap, ch - gap),
                        &c,
                        corner,
                        (z, r, v),
                        g.parts,
                    );
                }
                None => out
                    .prims
                    .push(lk.tile(x, yy, cw - gap, ch - gap, g.empty, corner, false)),
            }
        }
    }
    if g.show_legend {
        // the lowest value, five steps and the highest, right-aligned under the grid
        let min_text = fmt_auto(g.vmin);
        let max_text = fmt_auto(g.vmax);
        let s = 12.0;
        let ly = y0 + total + (legend_h - s) / 2.0;
        let right = (x0 + gw - gap).min(w - PAD);
        let xs = right - est(&max_text, tick) - 6.0 - 5.0 * (s + 3.0) + 3.0;
        if xs - 6.0 - est(&min_text, tick) >= PAD {
            out.prims
                .push(text(xs - 6.0, ly + s / 2.0, min_text, tick, lk.text, "end"));
            for k in 0..5 {
                let kf = f64::from(k);
                let c = lerp_color(g.low, g.high, kf / 4.0);
                out.prims
                    .push(lk.tile(xs + kf * (s + 3.0), ly, s, s, &c, 3.0, false));
            }
            out.prims.push(text(
                xs + 5.0 * (s + 3.0) - 3.0 + 6.0,
                ly + s / 2.0,
                max_text,
                tick,
                lk.text,
                "start",
            ));
        }
    }
}

/// Lays out `{rows: [[date, value]], options, lang}` in a `w`×`h` visual.
pub fn layout(input: &Value, w: f64, h: f64) -> Out {
    let o = options_of(input);
    let lk = look_of(input);
    let (low, high, empty) = if lk.minimal() {
        (
            opt_color(o, "low", "#D6E6EE"),
            opt_color(o, "high", "#1F6F8B"),
            opt_color(o, "empty", "#EFEFEF"),
        )
    } else {
        // one hue, light to dark: the palette's first colour and its tint
        let high = opt_color(o, "high", &lk.color(0));
        let tint = mix(&high, if lk.night { &lk.surface } else { "#FFFFFF" }, 0.86);
        (opt_color(o, "low", &tint), high, opt_color(o, "empty", lk.empty))
    };
    let show_legend = opt_bool(o, "showLegend", true);
    let months_only = opt_bool(o, "monthsOnly", false);
    let sunday_first = opt_bool(o, "sundayFirst", false);
    // the row of a day: Monday (or Sunday) 0
    let row = |z: i64| {
        if sunday_first {
            (z + 4).rem_euclid(7)
        } else {
            weekday(z)
        }
    };
    let fr = is_fr(input);
    let mut out = Out::default();
    // the value (summed) and first row of each day, and its highlighted part
    let mut days: HashMap<i64, (f64, usize)> = HashMap::new();
    let mut part_of: HashMap<i64, f64> = HashMap::new();
    let mut max_day = i64::MIN;
    for (i, r) in rows_of(input, MAX_ROWS).into_iter().enumerate() {
        let (Some(z), Some(v)) = (day_of(r.first()), cell_num(r.get(1))) else {
            continue;
        };
        days.entry(z).and_modify(|d| d.0 += v).or_insert((v, i));
        if let Some(hv) = hl_num(input, i) {
            *part_of.entry(z).or_insert(0.0) += hv;
        }
        max_day = max_day.max(z);
    }
    let parts = highlighted(input).then_some(&part_of);
    if days.is_empty() {
        return out;
    }
    let last_year = civil_from_days(max_day).0;
    let mut first_year = last_year;
    for z in days.keys() {
        first_year = first_year.min(civil_from_days(*z).0);
    }
    first_year = first_year.max(last_year - (MAX_YEARS - 1));
    let from = days_from_civil(first_year, 1, 1);
    let mut vmin = f64::INFINITY;
    let mut vmax = f64::NEG_INFINITY;
    let mut min_day = max_day;
    for (z, d) in &days {
        if *z >= from {
            vmin = vmin.min(d.0);
            vmax = vmax.max(d.0);
            min_day = min_day.min(*z);
        }
    }
    // the days drawn: whole years, or from the first month with data to the last
    let month_end = |y: i64, m: i64| {
        if m >= 12 {
            days_from_civil(y, 12, 31)
        } else {
            days_from_civil(y, m + 1, 1) - 1
        }
    };
    let (first_day, last_day) = if months_only {
        let a = civil_from_days(min_day);
        let b = civil_from_days(max_day);
        first_year = a.0;
        (days_from_civil(a.0, a.1, 1), month_end(b.0, b.1))
    } else {
        (from, days_from_civil(last_year, 12, 31))
    };
    let span = |y: i64| {
        (
            days_from_civil(y, 1, 1).max(first_day),
            days_from_civil(y, 12, 31).min(last_day),
        )
    };
    let years = (last_year - first_year + 1) as f64;
    let legend_h = if show_legend { 18.0 } else { 0.0 };
    let gutter = 30.0;
    let mut weeks = 0i64;
    for y in first_year..=last_year {
        let (z0, z1) = span(y);
        let start = z0 - row(z0);
        weeks = weeks.max((z1 - start).div_euclid(7) + 1);
    }
    if !lk.minimal() {
        // Modern and Bold: the cells fill the visual
        let g = Grid {
            first_year,
            last_year,
            weeks,
            sunday_first,
            show_legend,
            vmin,
            vmax,
            low: &low,
            high: &high,
            empty: &empty,
            fr,
            parts,
        };
        modern(&mut out, &lk, &g, &days, &row, &span, w, h);
        return out;
    }
    let block = (h - 2.0 * PAD - legend_h) / years;
    let c = 24f64
        .min((w - 2.0 * PAD - gutter) / weeks as f64)
        .min((block - 22.0) / 7.0);
    if c.partial_cmp(&2.0).is_none_or(|o| o == std::cmp::Ordering::Less) {
        return out;
    }
    let gap = if c >= 10.0 {
        2.0
    } else if c >= 4.0 {
        1.0
    } else {
        0.0
    };
    let cs = c - gap;
    let corner = (cs * 0.25).min(3.0);
    let x0 = PAD + gutter;
    let (months, wds) = if fr {
        (MONTHS_FR, WEEKDAYS_FR)
    } else {
        (MONTHS_EN, WEEKDAYS_EN)
    };
    for y in first_year..=last_year {
        let top = PAD + (y - first_year) as f64 * (14.0 + 7.0 * c + 8.0);
        let (z0, z1) = span(y);
        let start = z0 - row(z0);
        out.prims
            .push(text(PAD, top + 6.0, y.to_string(), 11.0, lk.label, "start"));
        for (m, label) in (1i64..).zip(months) {
            let m0 = days_from_civil(y, m, 1);
            if c * 4.0 >= est(label, 9.0) && month_end(y, m) >= z0 && m0 <= z1 {
                let col = (m0.max(z0) - start).div_euclid(7);
                out.prims.push(text(
                    x0 + col as f64 * c,
                    top + 6.0,
                    label.to_string(),
                    9.0,
                    lk.text,
                    "start",
                ));
            }
        }
        if c >= 7.0 {
            for (k, label) in wds.iter().enumerate() {
                let r = 2.0 * k as f64 + if sunday_first { 1.0 } else { 0.0 };
                let yy = top + 14.0 + r * c + cs / 2.0;
                out.prims
                    .push(text(x0 - 4.0, yy, label.to_string(), 9.0, lk.text, "end"));
            }
        }
        for z in z0..=z1 {
            let col = (z - start).div_euclid(7);
            let x = x0 + col as f64 * c;
            let yy = top + 14.0 + row(z) as f64 * c;
            match days.get(&z) {
                Some(&(v, row)) => {
                    let t = if vmax > vmin {
                        (v - vmin) / (vmax - vmin)
                    } else {
                        1.0
                    };
                    let c = lerp_color(&low, &high, t);
                    day_cell(&mut out, &lk, (x, yy, cs, cs), &c, corner, (z, row, v), parts);
                }
                None => out.prims.push(lk.tile(x, yy, cs, cs, &empty, corner, false)),
            }
        }
    }
    if show_legend {
        let ly = h - PAD - 12.0;
        let min_text = fmt_auto(vmin);
        let max_text = fmt_auto(vmax);
        let xs = w - PAD - est(&max_text, 9.0) - 4.0 - 5.0 * 12.0;
        if xs - 4.0 - est(&min_text, 9.0) >= PAD {
            out.prims
                .push(text(xs - 4.0, ly + 5.0, min_text, 9.0, lk.text, "end"));
            for k in 0..5 {
                let kf = f64::from(k);
                out.prims.push(lk.tile(
                    xs + kf * 12.0,
                    ly,
                    10.0,
                    10.0,
                    &lerp_color(&low, &high, kf / 4.0),
                    2.0,
                    false,
                ));
            }
            out.prims.push(text(
                xs + 5.0 * 12.0 + 2.0,
                ly + 5.0,
                max_text,
                9.0,
                lk.text,
                "start",
            ));
        }
    }
    out
}
