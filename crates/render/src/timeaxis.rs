//! Continuous date and numeric axes of column, bar, line and area charts (docs/render.md
//! "Continuous date axes" and "Continuous numeric axes"). Power BI's category axis type
//! (`categoryAxis.axisType`: `Scalar` is continuous, `Categorical` a band per value) chooses
//! between the two; a date, date-time or number category is continuous unless set categorical,
//! and an axis expanded down a hierarchy (several category levels) stays categorical, as does a
//! number level of a drill hierarchy. On a continuous scale each category sits at its value,
//! columns are as wide as the smallest gap between two values less the inner padding, lines join
//! the points in value order, and the labels are ticks at a round step (minutes, hours, days,
//! weeks, months, quarters, years; 1, 2 or 5 × 10ⁿ) chosen so that they do not overlap. The
//! axis `start` / `end` widen it and drop the categories outside. On a date axis the label
//! texts, the eight-label limit and the room between labels follow Power BI Desktop
//! 2.159.1231.0 (French UI, 2026-11-24, docs/render.md "Continuous date axes"); the step lists
//! and the numeric axes' labels are BIcraft's (d3 scale style), not measured against Desktop.

use bicraft_locale::calendar::{civil_from_serial, serial_from_civil};
use serde_json::Value;

use crate::charts::tw;
use crate::data::{ChartData, Col, Ctx, Fmt};
use crate::fmt::{ColFmt, Locale, axis_fmt_units, fmt_val, has_time, iso_serial, month_name, serial_text};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Unit {
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Year,
}

/// The tick steps, finest first.
pub(crate) const STEPS: [(Unit, i64); 25] = [
    (Unit::Minute, 1),
    (Unit::Minute, 5),
    (Unit::Minute, 15),
    (Unit::Minute, 30),
    (Unit::Hour, 1),
    (Unit::Hour, 3),
    (Unit::Hour, 6),
    (Unit::Hour, 12),
    (Unit::Day, 1),
    (Unit::Day, 2),
    (Unit::Week, 1),
    (Unit::Month, 1),
    (Unit::Month, 2),
    (Unit::Month, 3),
    (Unit::Month, 6),
    (Unit::Year, 1),
    (Unit::Year, 2),
    (Unit::Year, 5),
    (Unit::Year, 10),
    (Unit::Year, 20),
    (Unit::Year, 50),
    (Unit::Year, 100),
    (Unit::Year, 200),
    (Unit::Year, 500),
    (Unit::Year, 1000),
];

/// The room a date label of a column, line or area chart needs, from one tick to the next, for
/// a widest label of `w` px at `px` px: half a font size more than the label and six font sizes
/// at least. Power BI Desktop 2.159.1231.0 (2026-11-24, 10.67 px labels) kept six-hourly ticks
/// labelled `1 mars 06:00` (60 px) 66 px apart and yearly ticks 69 px apart, and dropped
/// six-monthly ticks (`janv. 2023`, 49 px) 59 px apart. A numeric axis keeps 2.3 font sizes
/// more than the label (BIcraft's, not measured).
pub(crate) fn label_room(ta: &TimeAxis, w: f32, px: f32) -> f32 {
    match ta.scale {
        Scale::Date => (w + px * 0.5).max(px * 6.0),
        Scale::Num { .. } => w + px * 2.3,
    }
}

/// Most labels on a date axis: Desktop 2.159.1231.0 drew 8 (six-hourly over two days on 600 and
/// 1,200 px) but never 9 or more, taking monthly ticks over 9 weekly ones on 1,200 px.
const MAX_LABELS: usize = 8;

/// Most ticks one step may generate (a finer step over a longer span is not tried).
pub(crate) const MAX_TICKS: usize = 400;

pub(crate) fn approx_days(u: Unit, k: i64) -> f64 {
    k as f64
        * match u {
            Unit::Minute => 1.0 / 1440.0,
            Unit::Hour => 1.0 / 24.0,
            Unit::Day => 1.0,
            Unit::Week => 7.0,
            Unit::Month => 30.44,
            Unit::Year => 365.25,
        }
}

/// The serials of step `(u, k)` within `[a, b]`: whole minutes or hours from midnight, days
/// (every `k`-th day of the month), Sundays, the first of every `k`-th month, the first of
/// January of years divisible by `k`.
pub(crate) fn steps_of(u: Unit, k: i64, a: f64, b: f64) -> Vec<f64> {
    let mut out = Vec::new();
    if !a.is_finite() || !b.is_finite() || b < a || k <= 0 || b - a > 4e6 {
        return out;
    }
    match u {
        Unit::Minute | Unit::Hour => {
            let per = (if u == Unit::Minute { k } else { k * 60 }) as f64;
            let mut m = (a * 1440.0 / per).ceil();
            while out.len() < MAX_TICKS {
                let x = m * per / 1440.0;
                if x > b + 1e-9 {
                    break;
                }
                out.push(x);
                m += 1.0;
            }
        }
        Unit::Day | Unit::Week => {
            let (d0, end) = (a.ceil() as i64, b.floor() as i64);
            let mut d = d0;
            while d <= end && out.len() < MAX_TICKS && d - d0 < 8 * MAX_TICKS as i64 {
                let keep = if u == Unit::Week {
                    // serial 1 (1899-12-31) was a Sunday
                    d.rem_euclid(7) == 1
                } else {
                    let (_, _, day) = civil_from_serial(d);
                    (day - 1) % k == 0
                };
                if keep {
                    out.push(d as f64);
                }
                d += 1;
            }
        }
        Unit::Month => {
            let (mut y, mut m, _) = civil_from_serial(a.floor() as i64);
            for _ in 0..(MAX_TICKS as i64 * k + 2).min(20_000) {
                let x = serial_from_civil(y, m, 1) as f64;
                if x > b {
                    break;
                }
                if x >= a && (m - 1) % k == 0 {
                    out.push(x);
                }
                (y, m) = if m >= 12 { (y + 1, 1) } else { (y, m + 1) };
            }
        }
        Unit::Year => {
            let (mut y, _, _) = civil_from_serial(a.floor() as i64);
            for _ in 0..20_000 {
                let x = serial_from_civil(y, 1, 1) as f64;
                if x > b || out.len() >= MAX_TICKS {
                    break;
                }
                if x >= a && y.rem_euclid(k) == 0 {
                    out.push(x);
                }
                y += 1;
            }
        }
    }
    out
}

/// A day tick: the column's date format, its short date when the format writes a time; `short`,
/// the day and the month's short name in the culture's order ("Jan 5", "5 janv.").
fn day_text(x: f64, fmt: &str, loc: &Locale, short: bool) -> String {
    if short {
        let (_, m, d) = civil_from_serial(x.floor() as i64);
        let mon = month_name(usize::try_from(m - 1).unwrap_or(0), loc);
        return if loc.date_order == 'm' {
            format!("{mon} {d}")
        } else {
            format!("{d} {mon}")
        };
    }
    let f = if fmt.trim().is_empty() || has_time(fmt) {
        "Short Date"
    } else {
        fmt
    };
    serial_text(x.floor(), f, loc)
        .or_else(|| serial_text(x.floor(), "Short Date", loc))
        .unwrap_or_default()
}

/// The label of a tick of unit `u` at serial `x` (`short`: days as "Jan 5"). On an axis whose
/// dates carry times (`timed`), hour and day ticks write the short day and the time, midnight
/// included ("1 mars 00:00"), as Power BI Desktop 2.159.1231.0 labels them (2026-11-24).
fn label(u: Unit, x: f64, fmt: &str, loc: &Locale, short: bool, timed: bool) -> String {
    let (y, m, _) = civil_from_serial(x.floor() as i64);
    match u {
        Unit::Year => y.to_string(),
        // as a categorical axis of first-of-month dates reads ("Jan 2025")
        Unit::Month => format!("{} {y}", month_name(usize::try_from(m - 1).unwrap_or(0), loc)),
        Unit::Day | Unit::Week | Unit::Minute | Unit::Hour if timed => {
            // a tick within a millisecond of midnight is that midnight
            let x = if (x - x.round()).abs() < 1e-7 {
                x.round()
            } else {
                x
            };
            let time = serial_text(x, "Short Time", loc).unwrap_or_default();
            format!("{} {time}", day_text(x, fmt, loc, true))
        }
        Unit::Day | Unit::Week => day_text(x, fmt, loc, short),
        // the day at midnight, else the time
        Unit::Minute | Unit::Hour => {
            if (x - x.round()).abs() < 1e-7 {
                day_text(x.round(), fmt, loc, short)
            } else {
                serial_text(x, "Short Time", loc).unwrap_or_default()
            }
        }
    }
}

/// What a continuous category axis measures.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Scale {
    /// Dates and times, as OLE serials (days).
    Date,
    /// Numbers; `int`: a whole-number column (ticks at least 1 apart).
    Num { int: bool },
}

/// A continuous category axis (time or numeric scale): the categories' values, dates as OLE
/// serials (days).
#[derive(Debug, Clone)]
pub(crate) struct TimeAxis {
    /// Category `i`'s value.
    pub t: Vec<f64>,
    /// The axis' ends: the data's, or the axis `start` / `end` where set.
    pub lo: f64,
    pub hi: f64,
    /// The smallest gap between two different values.
    pub gap: f64,
    pub scale: Scale,
    /// The category column's format string.
    fmt: String,
    /// The category column's format (numeric tick labels).
    cf: ColFmt,
    /// The axis' display units (`labelDisplayUnits`), numeric axes.
    units: Option<f64>,
    /// Some date has a time of day (not midnight).
    timed: bool,
}

/// The continuous scale of a chart's category axis and its values, `None` when the axis is
/// categorical: set `Categorical`, expanded down a hierarchy, fewer than two categories, a
/// column neither date nor number, a number level of a drill hierarchy, or a value that is no
/// date or number.
fn values(cd: &ChartData, cols: &[Col], f: &Fmt) -> Option<(Scale, Vec<f64>)> {
    if f.x.continuous == Some(false) || !cd.more.is_empty() || cd.raw.len() < 2 {
        return None;
    }
    if cd.cat_date {
        let t = cd
            .raw
            .iter()
            .map(|v| v.as_str().and_then(iso_serial).filter(|x| x.is_finite()))
            .collect::<Option<_>>()?;
        return Some((Scale::Date, t));
    }
    // a number level of a drill hierarchy (an auto date/time Year or Day) stays categorical
    if !f.drill.is_null() {
        return None;
    }
    let col = cd.ci.and_then(|i| cols.get(i))?;
    if col.kind != "column" || !col.is_num_type() {
        return None;
    }
    let t = cd
        .raw
        .iter()
        .map(|v| match v {
            Value::String(s) => s.trim().parse::<f64>().ok(),
            _ => v.as_f64(),
        })
        .map(|x| x.filter(|x| x.is_finite() && x.abs() < 1e15))
        .collect::<Option<_>>()?;
    let int = col.typ.to_ascii_lowercase().contains("int");
    Some((Scale::Num { int }, t))
}

/// The axis `start` and `end` (`categoryAxis`) of scale `sc`, `None` each when unset; both
/// dropped when `start` is not before `end`.
fn bounds(f: &Fmt, sc: Scale) -> (Option<f64>, Option<f64>) {
    let (a, b) = match sc {
        Scale::Date => (f.x.min_date, f.x.max_date),
        Scale::Num { .. } => (f.x.min, f.x.max),
    };
    match (a, b) {
        (Some(a), Some(b)) if a >= b => (None, None),
        ab => ab,
    }
}

/// Drops the categories outside a continuous axis' `start` / `end` (the axis shows only its
/// range); nothing changes on a categorical axis or without bounds.
pub(crate) fn clip_to_range(cd: &mut ChartData, cols: &[Col], f: &Fmt) {
    let Some((sc, t)) = values(cd, cols, f) else {
        return;
    };
    let (a, b) = bounds(f, sc);
    if a.is_none() && b.is_none() {
        return;
    }
    let (a, b) = (a.unwrap_or(f64::NEG_INFINITY), b.unwrap_or(f64::INFINITY));
    let keep: Vec<bool> = t.iter().map(|x| *x >= a - 1e-9 && *x <= b + 1e-9).collect();
    if keep.iter().all(|k| *k) {
        return;
    }
    fn retain<T>(v: &mut Vec<T>, keep: &[bool]) {
        *v = std::mem::take(v)
            .into_iter()
            .enumerate()
            .filter(|(i, _)| keep.get(*i).copied().unwrap_or(false))
            .map(|(_, x)| x)
            .collect();
    }
    retain(&mut cd.cats, &keep);
    retain(&mut cd.raw, &keep);
    retain(&mut cd.raw_more, &keep);
    for s in &mut cd.series {
        retain(&mut s.vals, &keep);
        retain(&mut s.pts, &keep);
        retain(&mut s.lpts, &keep);
        retain(&mut s.err, &keep);
        if let Some(v) = s.base.as_mut() {
            retain(v, &keep);
        }
        if let Some(v) = s.hvals.as_mut() {
            retain(v, &keep);
        }
    }
}

impl TimeAxis {
    /// The continuous axis of a chart, `None` when its category axis is categorical: set
    /// `Categorical`, expanded down a hierarchy, not a single date or number column, fewer than
    /// two different values, or a value that is no date or number. The axis runs from the data's
    /// first to last value, or from its `start` / `end` where set ([`clip_to_range`] drops the
    /// categories outside them first).
    pub(crate) fn of(cd: &ChartData, cols: &[Col], f: &Fmt) -> Option<TimeAxis> {
        let (scale, t) = values(cd, cols, f)?;
        let mut s = t.clone();
        s.sort_by(f64::total_cmp);
        let (d0, d1) = (*s.first()?, *s.last()?);
        let gap = s
            .windows(2)
            .filter_map(|w| match w {
                [a, b] if b - a > 1e-9 * a.abs().max(1.0) => Some(b - a),
                _ => None,
            })
            .fold(f64::INFINITY, f64::min);
        let (a, b) = bounds(f, scale);
        let (lo, hi) = (a.map_or(d0, |a| a.min(d0)), b.map_or(d1, |b| b.max(d1)));
        if d1 <= d0 || hi <= lo || !gap.is_finite() || !(hi - lo).is_finite() {
            return None;
        }
        let col = cd.ci.and_then(|i| cols.get(i));
        let timed = scale == Scale::Date && t.iter().any(|x| (x - x.round()).abs() > 1e-7);
        Some(TimeAxis {
            t,
            lo,
            hi,
            gap,
            scale,
            fmt: col.map(|c| c.fmt.clone()).unwrap_or_default(),
            cf: col.map(Col::cf).unwrap_or_default(),
            units: f.x.units,
            timed,
        })
    }

    /// Category `i`'s date.
    pub(crate) fn at(&self, i: usize) -> f64 {
        self.t.get(i).copied().unwrap_or(self.lo)
    }

    /// The categories in time order (lines join their points in this order).
    pub(crate) fn order(&self) -> Vec<usize> {
        let mut o: Vec<usize> = (0..self.t.len()).collect();
        o.sort_by(|a, b| self.at(*a).total_cmp(&self.at(*b)));
        o
    }

    /// Columns along `len` px: (px per day, bar width). The first and last bars stop half a pixel
    /// inside the ends; a bar is as wide as the smallest gap less the inner padding (Desktop
    /// 2.158 drew monthly columns 0.92 of the mean month wide at 0 % padding: February's share).
    pub(crate) fn bars(&self, len: f32, inner: f32) -> (f64, f32) {
        let k = f64::from(1.0 - inner.clamp(0.0, 0.9));
        let s = (f64::from(len.max(1.0)) - 1.0).max(0.1) / (self.hi - self.lo + k * self.gap);
        (s, (k * self.gap * s) as f32)
    }

    /// The ticks along an axis of `len` px where date `t` sits at `a + (t - lo) * s`: their
    /// positions (from the start of the axis) and labels, within the data's dates. The finest
    /// step of at most eight ticks whose labels keep `need(widest label)` px apart wins, days
    /// written "Jan 5" (with the time when the dates carry times); steps finer than the data's
    /// smallest gap are skipped.
    pub(crate) fn ticks(
        &self,
        ctx: &Ctx<'_>,
        len: f32,
        a: f32,
        s: f64,
        px: f32,
        need: &dyn Fn(f32) -> f32,
    ) -> Vec<(f32, String)> {
        if !s.is_finite() || s <= 0.0 {
            return Vec::new();
        }
        let ta = self.lo - f64::from(a) / s;
        let tb = self.lo + f64::from(len - a) / s;
        let pos = |t: f64| a + ((t - self.lo) * s) as f32;
        if let Scale::Num { int } = self.scale {
            return self.num_ticks(ctx, ta, tb, int, &pos, px, need);
        }
        // date ticks within the axis' range, the data's dates or the axis start / end where set
        // (Desktop 2.159.1231.0 labels none in the margins beyond the data)
        let (ta, tb) = (ta.max(self.lo), tb.min(self.hi));
        let mut last: Vec<(f32, String)> = Vec::new();
        for (u, k) in STEPS {
            let d = approx_days(u, k);
            if d < self.gap * 0.9 || (tb - ta) / d > MAX_TICKS as f64 {
                continue;
            }
            let mut ts = steps_of(u, k, ta, tb);
            if ts.is_empty() {
                continue;
            }
            let mut u = u;
            if ts.len() < 2 {
                // one round date at most: the first and last dates, as Desktop 2.159.1231.0 drew
                // twenty days on 240 px ("1 janv.", "20 janv.")
                ts = vec![self.lo, self.hi];
                u = if self.timed { Unit::Hour } else { Unit::Day };
            }
            // days always as "5 janv." whatever the column's format (Desktop 2.159.1231.0 wrote
            // Long Date and General Date columns' day ticks so)
            let ticks: Vec<(f32, String)> = ts
                .iter()
                .map(|t| (pos(*t), label(u, *t, &self.fmt, &ctx.loc, true, self.timed)))
                .collect();
            let widest = ticks.iter().map(|(_, l)| tw(ctx, l, px)).fold(0.0f32, f32::max);
            if ticks.len() <= MAX_LABELS && tick_room(&ticks, f32::INFINITY) >= need(widest) {
                return ticks;
            }
            last = ticks;
        }
        // nothing fits: the first tick of the coarsest step, else the first date
        match last.into_iter().next() {
            Some(t) => vec![t],
            None => vec![(a, day_text(self.lo, &self.fmt, &ctx.loc, false))],
        }
    }

    /// A numeric axis' ticks within `[ta, tb]`: the finest round step (1, 2 or 5 × 10ⁿ, at
    /// least 1 on a whole-number column) whose labels keep `need(widest label)` px apart,
    /// labelled with the column's format string (without one: the fewest decimals that write
    /// every tick, in the axis' display units).
    #[allow(clippy::too_many_arguments)]
    fn num_ticks(
        &self,
        ctx: &Ctx<'_>,
        ta: f64,
        tb: f64,
        int: bool,
        pos: &dyn Fn(f64) -> f32,
        px: f32,
        need: &dyn Fn(f32) -> f32,
    ) -> Vec<(f32, String)> {
        let span = tb - ta;
        if !span.is_finite() || span <= 0.0 {
            return Vec::new();
        }
        let e0 = (span / MAX_TICKS as f64).log10().floor() as i32;
        let mut last: Vec<(f32, String)> = Vec::new();
        for e in e0.clamp(-300, 300)..=(e0.clamp(-300, 300) + 20) {
            for m in [1.0, 2.0, 5.0] {
                let step = m * 10f64.powi(e);
                if (int && step < 1.0) || span / step > MAX_TICKS as f64 || !step.is_finite() {
                    continue;
                }
                let (k0, k1) = ((ta / step).ceil(), (tb / step).floor());
                if k1 < k0 {
                    continue;
                }
                let label = self.num_label(ctx, ta, tb, step);
                let ticks: Vec<(f32, String)> = (0..=((k1 - k0) as usize).min(MAX_TICKS))
                    .map(|j| {
                        // whole multiples of the step, -0 written as 0
                        let x = (k0 + j as f64) * step + 0.0;
                        (pos(x), label(x))
                    })
                    .collect();
                let widest = ticks.iter().map(|(_, l)| tw(ctx, l, px)).fold(0.0f32, f32::max);
                if tick_room(&ticks, f32::INFINITY) >= need(widest) {
                    return ticks;
                }
                last = ticks;
            }
        }
        last.into_iter().take(1).collect()
    }

    /// The label writer of a numeric tick every `step` within `[lo, hi]`.
    fn num_label(&self, ctx: &Ctx<'_>, lo: f64, hi: f64, step: f64) -> Box<dyn Fn(f64) -> String> {
        let loc = ctx.loc.clone();
        let cf = self.cf.clone();
        if !cf.fmt.trim().is_empty() || cf.data_type.to_ascii_lowercase().contains("int") {
            // the column's format string, as its values are written elsewhere
            Box::new(move |x| fmt_val(x, Some(&cf), &loc, false))
        } else {
            Box::new(axis_fmt_units(lo, hi, step, Some(&cf), &loc, self.units))
        }
    }
}

/// The smallest distance between two ticks (the room each label may take).
pub(crate) fn tick_room(ticks: &[(f32, String)], len: f32) -> f32 {
    ticks
        .windows(2)
        .filter_map(|w| match w {
            [p, q] => Some(q.0 - p.0),
            _ => None,
        })
        .fold(len.max(1.0), f32::min)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(y: i64, m: i64, d: i64) -> f64 {
        serial_from_civil(y, m, d) as f64
    }

    #[test]
    fn steps_fall_on_round_dates() {
        let months = steps_of(Unit::Month, 3, s(2025, 1, 15), s(2026, 1, 1));
        assert_eq!(
            months,
            vec![s(2025, 4, 1), s(2025, 7, 1), s(2025, 10, 1), s(2026, 1, 1)]
        );
        let weeks = steps_of(Unit::Week, 1, s(2026, 1, 1), s(2026, 1, 20));
        // 2026-01-04 is a Sunday
        assert_eq!(weeks, vec![s(2026, 1, 4), s(2026, 1, 11), s(2026, 1, 18)]);
        let hours = steps_of(Unit::Hour, 6, s(2026, 1, 5) + 0.1, s(2026, 1, 6));
        assert_eq!(hours.len(), 4);
        assert!((hours[0] - (s(2026, 1, 5) + 0.25)).abs() < 1e-9);
        let years = steps_of(Unit::Year, 5, s(2001, 3, 1), s(2021, 1, 1));
        assert_eq!(
            years,
            vec![s(2005, 1, 1), s(2010, 1, 1), s(2015, 1, 1), s(2020, 1, 1)]
        );
        // hostile bounds generate nothing, or a bounded list
        assert!(steps_of(Unit::Day, 1, f64::NAN, 3.0).is_empty());
        assert!(steps_of(Unit::Minute, 1, 0.0, 1e300).is_empty());
        assert!(steps_of(Unit::Day, 1, 0.0, 1e6).len() <= MAX_TICKS);
    }

    fn lbl(u: Unit, x: f64, fmt: &str, loc: &Locale) -> String {
        label(u, x, fmt, loc, false, false)
    }

    #[test]
    fn tick_labels_by_unit() {
        let en = Locale::new("en-US");
        let fr = Locale::new("fr-FR");
        let x = s(2026, 1, 5);
        assert_eq!(lbl(Unit::Year, x, "", &en), "2026");
        assert_eq!(lbl(Unit::Month, x, "General Date", &en), "Jan 2026");
        assert_eq!(lbl(Unit::Month, x, "General Date", &fr), "janv. 2026");
        // a General Date column's day ticks drop the midnight time
        assert_eq!(lbl(Unit::Day, x, "General Date", &fr), "05/01/2026");
        assert_eq!(lbl(Unit::Day, x, "Long Date", &en), "Monday, January 5, 2026");
        assert_eq!(lbl(Unit::Hour, x + 0.25, "General Date", &fr), "06:00");
        assert_eq!(lbl(Unit::Hour, x, "General Date", &fr), "05/01/2026");
        // short day ticks, in the culture's order
        assert_eq!(label(Unit::Week, x, "General Date", &en, true, false), "Jan 5");
        assert_eq!(label(Unit::Day, x, "Short Date", &fr, true, false), "5 janv.");
        // an axis whose dates carry times writes the day and the time on every tick
        assert_eq!(
            label(Unit::Hour, x + 0.5, "General Date", &fr, false, true),
            "5 janv. 12:00"
        );
        assert_eq!(
            label(Unit::Day, x, "General Date", &fr, false, true),
            "5 janv. 00:00"
        );
    }
}
