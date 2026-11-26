//! The Gantt visual (`ganttChart`, docs/formats/gantt.md): the task list (task labels and the
//! detail columns), up to three levels of category rows with their expand/collapse icon, and a
//! time line with one bar per task from its start to its end, a milestone for a task without an
//! end, the progress inside the bar, the axis labels (one auto-scaled unit, or the two units the
//! `axis` object fixes), gridlines and shaded week days. Rows are one per task as the engine
//! evaluates them (at most 1,000).
//!
//! The roles come from a black-box wells test against Desktop 2.159.1231.0; the drawing follows
//! Microsoft Learn's description of the visual and the formatting objects of the published theme
//! schema 2.159. Sizes, default colours and transparencies are BIcraft's own choices, not
//! measured on Desktop's drawing.

use serde_json::Value;

use crate::canvas::{Anchor, Base, Canvas, Rgb, Text};
use crate::charts::Bx;
use crate::data::{Col, Ctx, Dataset, PT, arr, clampf, fin, get, sk};
use crate::fonts::Font;
use crate::hits::{self, HitShape, Point};

/// Most tasks read (Microsoft Learn: a Gantt shows at most 1,000 rows).
pub const MAX_ROWS: usize = 1000;
/// Most category levels (the `Category` well takes three fields).
const MAX_CATS: usize = 3;
/// Most detail and tooltip columns read.
const MAX_COLUMNS: usize = 20;
/// Most axis labels or gridlines drawn per level.
const MAX_TICKS: usize = 2000;
/// Height of one axis label level.
const LEVEL_H: f32 = 20.0;
/// Default task column width (with detail columns: their default width is 80).
const TASK_W: f32 = 150.0;
const DETAIL_W: f32 = 80.0;
/// Default bar height.
const BAR_H: f32 = 14.0;
/// Default indentation of each category level.
const INDENT: f32 = 12.0;
/// Room of the expand/collapse icon.
const ICON_W: f32 = 14.0;
/// Category backgrounds: the row colour at 70 %, 80 % and 90 % transparency (Microsoft Learn).
const CATEGORY_TRANSPARENCY: [f32; 3] = [0.7, 0.8, 0.9];
/// The progress inside a bar: the bar colour mixed with this share of black.
const PROGRESS_SHADE: f32 = 0.35;
/// Days of the serial range DAX accepts (0100-01-01 to 9999-12-31).
const SERIAL_MIN: f64 = -657_434.0;
const SERIAL_MAX: f64 = 2_958_466.0;

/// A date unit of the time axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Day,
    Month,
    Year,
}

impl Unit {
    fn of(s: &str) -> Option<Unit> {
        match s {
            "day" => Some(Unit::Day),
            "month" => Some(Unit::Month),
            "year" => Some(Unit::Year),
            _ => None,
        }
    }

    /// The start of the unit holding serial day `t`.
    fn floor(self, t: f64) -> f64 {
        use bicraft_locale::calendar as cal;
        let d = t.floor() as i64;
        match self {
            Unit::Day => d as f64,
            Unit::Month => cal::month_start(cal::month_index(d)) as f64,
            Unit::Year => {
                let (y, _, _) = cal::civil_from_serial(d);
                cal::serial_from_civil(y, 1, 1) as f64
            }
        }
    }

    /// The start of the next unit after the one holding `t`.
    fn next(self, t: f64) -> f64 {
        use bicraft_locale::calendar as cal;
        let d = t.floor() as i64;
        match self {
            Unit::Day => (d + 1) as f64,
            Unit::Month => cal::month_start(cal::month_index(d).saturating_add(1)) as f64,
            Unit::Year => {
                let (y, _, _) = cal::civil_from_serial(d);
                cal::serial_from_civil(y.saturating_add(1), 1, 1) as f64
            }
        }
    }
}

/// What a line of the list is.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// A category member at `level` (0: the first Category field).
    Category { level: usize, collapsed: bool },
    /// A task: its row in the data.
    Task { row: usize },
}

/// One line of the list, top to bottom.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub kind: Kind,
    /// Indentation steps.
    pub level: usize,
    /// The category members above it (a category: its own last), as the data holds them.
    pub path: Vec<Value>,
    /// Start and end (OLE serial days); a task without an end is a milestone at its start.
    pub start: Option<f64>,
    pub end: Option<f64>,
    /// Share done, 0 to 1.
    pub progress: Option<f64>,
    /// Palette index (its top category's position with `row.colorByCategory`).
    pub color: usize,
    /// Top of the line.
    pub y: f32,
}

/// The bound fields, as data column indices.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Roles {
    pub task: Option<usize>,
    pub start: Option<usize>,
    pub end: Option<usize>,
    pub progress: Option<usize>,
    pub cats: Vec<usize>,
    pub details: Vec<usize>,
    pub tips: Vec<usize>,
}

impl Roles {
    pub fn of(cols: &[Col]) -> Roles {
        let mut r = Roles::default();
        for (i, c) in cols.iter().enumerate() {
            match c.role.to_ascii_lowercase().as_str() {
                "task" if r.task.is_none() => r.task = Some(i),
                "startdate" if r.start.is_none() => r.start = Some(i),
                "enddate" if r.end.is_none() => r.end = Some(i),
                "progress" if r.progress.is_none() => r.progress = Some(i),
                "category" if r.cats.len() < MAX_CATS => r.cats.push(i),
                "columns" if r.details.len() < MAX_COLUMNS => r.details.push(i),
                "tooltips" if r.tips.len() < MAX_COLUMNS => r.tips.push(i),
                _ => {}
            }
        }
        r
    }
}

/// Where everything goes, in body pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub roles: Roles,
    /// The lines shown (collapsed categories hide their members), at most what fits.
    pub lines: Vec<Line>,
    /// Lines below the body, not drawn.
    pub cut: usize,
    /// The task column and each detail column.
    pub task_w: f32,
    pub detail_w: Vec<f32>,
    /// Left edge and width of the time line.
    pub chart_x: f32,
    pub chart_w: f32,
    /// Header band (column headers, axis labels on top) and axis labels at the bottom.
    pub header_h: f32,
    pub footer_h: f32,
    pub row_h: f32,
    pub bar_h: f32,
    /// The time line's first and last day (serials); `None` without dates.
    pub domain: Option<(f64, f64)>,
    /// Axis label levels, top first.
    pub units: Vec<Unit>,
    /// Labels on top and at the bottom.
    pub top: bool,
    pub bottom: bool,
    /// Time runs right to left.
    pub invert: bool,
}

impl Layout {
    /// The x of serial day `t`.
    pub fn x_of(&self, t: f64) -> f32 {
        let Some((lo, hi)) = self.domain else {
            return self.chart_x;
        };
        let k = ((t - lo) / (hi - lo).max(1e-9)) as f32;
        let k = if self.invert { 1.0 - k } else { k };
        self.chart_x + k * self.chart_w
    }

    /// The bar of a line: `(x, w)`, `None` for a milestone or a line without dates.
    pub fn bar(&self, l: &Line) -> Option<(f32, f32)> {
        let (s, e) = (l.start?, l.end?);
        if e <= s {
            return None;
        }
        let (a, b) = (self.x_of(s), self.x_of(e));
        let (a, b) = (a.min(b), a.max(b));
        Some((a, (b - a).max(2.0)))
    }

    /// The milestone of a task: its centre x (a task with a start and no end, or an end not
    /// after its start).
    pub fn milestone(&self, l: &Line) -> Option<f32> {
        if !matches!(l.kind, Kind::Task { .. }) {
            return None;
        }
        let s = l.start?;
        match l.end {
            Some(e) if e > s => None,
            _ => Some(self.x_of(s)),
        }
    }
}

fn gantt(v: &Value) -> &Value {
    get(get(v, "format"), "gantt")
}

fn opt<'a>(v: &'a Value, object: &str, key: &str) -> &'a Value {
    get(get(gantt(v), object), key)
}

fn flag(x: &Value, default: bool) -> bool {
    x.as_bool().unwrap_or(default)
}

fn number(x: &Value, default: f32, lo: f32, hi: f32) -> f32 {
    fin(x).map_or(default, |n| clampf(n as f32, lo, hi))
}

/// A date cell as an OLE serial day: ISO text, or a number taken as a serial.
pub fn serial(x: &Value) -> Option<f64> {
    let t = match x {
        Value::String(s) => crate::fmt::iso_serial(s),
        Value::Number(_) => fin(x),
        _ => None,
    }?;
    (SERIAL_MIN..=SERIAL_MAX).contains(&t).then_some(t)
}

/// A progress cell as a share, 0 to 1.
fn share(x: &Value) -> Option<f64> {
    fin(x).map(|p| p.clamp(0.0, 1.0))
}

fn cell(ds: &Dataset, row: usize, col: Option<usize>) -> &Value {
    col.and_then(|c| ds.rows.get(row).and_then(|r| r.get(c)))
        .unwrap_or(&Value::Null)
}

/// Whether the visual's saved state collapses the category `path` (its last member at
/// `path.len() - 1`): a `row` entry naming it, else the whole-visual `row.isCollapsed`.
fn collapsed(v: &Value, roles: &Roles, path: &[Value]) -> bool {
    let level = path.len().saturating_sub(1);
    let mut out = flag(opt(v, "row", "collapsed"), false);
    for e in arr(get(get(v, "format"), "gantt_rows")).iter().take(500) {
        let pairs = arr(get(e, "path"));
        let names_level = roles
            .cats
            .get(level)
            .is_some_and(|c| pairs.iter().any(|p| get(p, "c").as_u64() == Some(*c as u64)));
        let all_match = pairs.iter().all(|p| {
            let c = get(p, "c").as_u64();
            roles
                .cats
                .iter()
                .take(level + 1)
                .position(|x| Some(*x as u64) == c)
                .and_then(|k| path.get(k))
                .is_some_and(|m| hits::same(m, get(p, "v")))
        });
        if names_level && all_match && !pairs.is_empty() {
            out = flag(get(e, "collapsed"), out);
        }
    }
    out
}

/// The lines of the list: categories in the order their first task comes, each followed by its
/// members unless collapsed; tasks in data order.
fn lines(v: &Value, ds: &Dataset, roles: &Roles) -> Vec<Line> {
    let rows: Vec<usize> = (0..ds.rows.len().min(MAX_ROWS)).collect();
    let by_cat = flag(opt(v, "row", "color_by_category"), false);
    let mut out = Vec::new();
    group(v, ds, roles, &rows, &mut Vec::new(), by_cat, 0, &mut out);
    out
}

#[allow(clippy::too_many_arguments)]
fn group(
    v: &Value,
    ds: &Dataset,
    roles: &Roles,
    rows: &[usize],
    path: &mut Vec<Value>,
    by_cat: bool,
    color: usize,
    out: &mut Vec<Line>,
) {
    let level = path.len();
    let Some(&col) = roles.cats.get(level) else {
        for &r in rows {
            let start = serial(cell(ds, r, roles.start));
            let end = serial(cell(ds, r, roles.end));
            let mut p = path.clone();
            p.push(cell(ds, r, roles.task).clone());
            out.push(Line {
                kind: Kind::Task { row: r },
                level,
                path: p,
                start,
                end,
                progress: share(cell(ds, r, roles.progress)),
                color,
                y: 0.0,
            });
        }
        return;
    };
    // members in the order of their first row
    let mut members: Vec<(Value, Vec<usize>)> = Vec::new();
    for &r in rows {
        let m = cell(ds, r, Some(col));
        match members.iter_mut().find(|(x, _)| hits::same(x, m)) {
            Some((_, list)) => list.push(r),
            None => members.push((m.clone(), vec![r])),
        }
    }
    for (k, (m, list)) in members.into_iter().enumerate() {
        let color = if by_cat && level == 0 { k } else { color };
        path.push(m);
        let starts = list.iter().filter_map(|r| serial(cell(ds, *r, roles.start)));
        let ends = list
            .iter()
            .filter_map(|r| serial(cell(ds, *r, roles.end)).or_else(|| serial(cell(ds, *r, roles.start))));
        let start = starts.fold(None, |a: Option<f64>, x| Some(a.map_or(x, |a| a.min(x))));
        let end = ends.fold(None, |a: Option<f64>, x| Some(a.map_or(x, |a| a.max(x))));
        let shut = collapsed(v, roles, path);
        out.push(Line {
            kind: Kind::Category {
                level,
                collapsed: shut,
            },
            level,
            path: path.clone(),
            start,
            end,
            progress: None,
            color,
            y: 0.0,
        });
        if !shut {
            group(v, ds, roles, &list, path, by_cat, color, out);
        }
        path.pop();
    }
}

/// A fixed axis bound (`axis.start` / `axis.end`): an ISO date or a serial.
fn bound(x: &Value) -> Option<f64> {
    match x {
        Value::String(s) => {
            let s = s.trim().trim_matches('\'');
            crate::fmt::iso_serial(s)
                .or_else(|| s.parse::<f64>().ok())
                .filter(|t| t.is_finite() && (SERIAL_MIN..=SERIAL_MAX).contains(t))
        }
        _ => serial(x),
    }
}

/// The layout of a Gantt whose snapshot visual is `v` and data `ds`, in a body of `w` × `h`.
pub fn layout(v: &Value, ds: &Dataset, w: f32, h: f32) -> Layout {
    let roles = Roles::of(&ds.cols);
    let mut lines = lines(v, ds, &roles);
    // the task list: the task column, then each detail column
    let headers_size = number(opt(v, "column_headers", "default_width"), DETAIL_W, 10.0, 2000.0);
    let widths = arr(get(get(v, "format"), "gantt_widths"));
    let width_of = |c: usize| {
        widths
            .iter()
            .find(|x| get(x, "column").as_u64() == Some(c as u64))
            .and_then(|x| fin(get(x, "width")))
            .map(|x| clampf(x as f32, 10.0, 2000.0))
    };
    let mut detail_w: Vec<f32> = roles
        .details
        .iter()
        .map(|c| width_of(*c).unwrap_or(headers_size))
        .collect();
    let details: f32 = detail_w.iter().sum();
    let panel = match fin(opt(v, "column_panel", "width")) {
        Some(p) => clampf(p as f32, 0.0, w),
        None => {
            let task = roles.task.and_then(width_of).unwrap_or(TASK_W);
            (task + details).min(w * 0.6)
        }
    };
    let task_w = (panel - details).max(40.0).min(panel);
    // detail columns that do not fit are narrowed together
    let room = (panel - task_w).max(0.0);
    if details > room && details > 0.0 {
        let k = room / details;
        for d in &mut detail_w {
            *d *= k;
        }
    }
    let chart_x = panel + if panel > 0.0 { 1.0 } else { 0.0 };
    let chart_w = (w - chart_x).max(1.0);
    // the time line: every task's dates (collapsed ones too), or the fixed bounds
    let mut lo: Option<f64> = None;
    let mut hi: Option<f64> = None;
    for r in 0..ds.rows.len().min(MAX_ROWS) {
        for t in [serial(cell(ds, r, roles.start)), serial(cell(ds, r, roles.end))]
            .into_iter()
            .flatten()
        {
            lo = Some(lo.map_or(t, |x| x.min(t)));
            hi = Some(hi.map_or(t, |x| x.max(t)));
        }
    }
    let fixed_lo = bound(opt(v, "axis", "start"));
    let fixed_hi = bound(opt(v, "axis", "end"));
    let auto = flag(opt(v, "axis", "auto_scale"), true);
    let mut domain = match (fixed_lo.or(lo), fixed_hi.or(hi)) {
        (Some(a), Some(b)) if b > a => Some((a, b)),
        (Some(a), _) => Some((a, a + 1.0)),
        (None, Some(b)) => Some((b - 1.0, b)),
        (None, None) => None,
    };
    let units: Vec<Unit> = if auto {
        let span = domain.map_or(30.0, |(a, b)| (b - a).max(1.0));
        let per_day = f64::from(chart_w) / span;
        vec![if per_day >= 18.0 {
            Unit::Day
        } else if per_day * 30.44 >= 34.0 {
            Unit::Month
        } else {
            Unit::Year
        }]
    } else {
        let p = Unit::of(sk(gantt_axis(v), "primary_unit")).unwrap_or(Unit::Month);
        match Unit::of(sk(gantt_axis(v), "secondary_unit")) {
            Some(s) if s != p => vec![p, s],
            _ => vec![p],
        }
    };
    // a computed time line starts and ends on whole units of the finest level
    let finest = units
        .iter()
        .copied()
        .min_by_key(|u| match u {
            Unit::Day => 0,
            Unit::Month => 1,
            Unit::Year => 2,
        })
        .unwrap_or(Unit::Month);
    if let Some((a, b)) = domain {
        let a = if fixed_lo.is_some() { a } else { finest.floor(a) };
        let b = if fixed_hi.is_some() {
            b
        } else if finest.floor(b) == b {
            b.max(a + 1.0)
        } else {
            finest.next(b)
        };
        domain = Some((a, b.max(a + 1.0)));
    }
    let position = sk(gantt_axis(v), "position");
    let show_axis = flag(opt(v, "axis_tick", "show"), true);
    let (top, bottom) = match position {
        "bottom" => (false, true),
        "both" => (true, true),
        _ => (true, false),
    };
    let (top, bottom) = (top && show_axis, bottom && show_axis);
    let levels = units.len() as f32 * LEVEL_H;
    let headers = flag(opt(v, "column_headers", "show"), true);
    let header_h = if top {
        levels
    } else if headers {
        LEVEL_H
    } else {
        0.0
    };
    let footer_h = if bottom { levels } else { 0.0 };
    let bar_h = number(opt(v, "bar", "height"), BAR_H, 2.0, 80.0);
    let value_px = number(opt(v, "column_value", "font_size"), 9.0, 5.0, 40.0) * PT;
    let row_h = (bar_h + 10.0).max(value_px * 1.9).round();
    let mut y = header_h;
    let mut kept = 0usize;
    for l in &mut lines {
        if y + row_h > h - footer_h + 0.5 {
            break;
        }
        l.y = y;
        y += row_h;
        kept += 1;
    }
    let cut = lines.len().saturating_sub(kept);
    lines.truncate(kept);
    Layout {
        roles,
        lines,
        cut,
        task_w,
        detail_w,
        chart_x,
        chart_w,
        header_h,
        footer_h,
        row_h,
        bar_h,
        domain,
        units,
        top,
        bottom,
        invert: flag(opt(v, "axis", "invert"), false),
    }
}

fn gantt_axis(v: &Value) -> &Value {
    get(gantt(v), "axis")
}

/// The ticks of `unit` in the time line: (start serial, label).
fn ticks(lay: &Layout, unit: Unit, v: &Value, ctx: &Ctx<'_>, single: bool) -> Vec<(f64, String)> {
    let Some((lo, hi)) = lay.domain else {
        return Vec::new();
    };
    let own = match unit {
        Unit::Day => sk(gantt_axis(v), "format_day"),
        Unit::Month => sk(gantt_axis(v), "format_month"),
        Unit::Year => sk(gantt_axis(v), "format_year"),
    };
    let culture = ctx.loc.culture;
    let mut out = Vec::new();
    let mut t = unit.floor(lo);
    let mut first = true;
    while t < hi && out.len() < MAX_TICKS {
        let (y, m, d) = bicraft_locale::calendar::civil_from_serial(t as i64);
        let label = if !own.is_empty() {
            bicraft_locale::vb::format_date(t, own, culture).unwrap_or_default()
        } else {
            let month = culture
                .months_abbr
                .get(usize::try_from(m - 1).unwrap_or(0))
                .copied()
                .unwrap_or("");
            match unit {
                Unit::Year => y.to_string(),
                // one level: the year with the first month and each January
                Unit::Month if single && (first || m == 1) => format!("{month} {y}"),
                Unit::Month => month.to_string(),
                Unit::Day if single && (first || d == 1) => format!("{d} {month}"),
                Unit::Day => d.to_string(),
            }
        };
        out.push((t, label));
        first = false;
        let n = unit.next(t);
        if n <= t {
            break;
        }
        t = n;
    }
    out
}

fn color_of(x: &Value) -> Option<Rgb> {
    Rgb::from_value(x)
}

/// `transparency` (percent) as an opacity.
fn alpha_of(x: &Value, default: f32) -> f32 {
    1.0 - number(x, default, 0.0, 100.0) / 100.0
}

fn text_font(v: &Value, object: &str, ctx: &Ctx<'_>, px: f32) -> Font {
    // the first family of a CSS-style list ("'Segoe UI', wf_segoe-ui_normal")
    let fam = opt(v, object, "font_family")
        .as_str()
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().trim_matches(['\'', '"']).to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| ctx.font.clone());
    let mut f = Font::new(&fam, px);
    if flag(opt(v, object, "bold"), false) {
        f = f.weight(700);
    }
    f.italic = flag(opt(v, object, "italic"), false);
    f
}

/// The tooltip rows of a line.
fn tip(lay: &Layout, ds: &Dataset, l: &Line, ctx: &Ctx<'_>) -> Vec<(String, String)> {
    let r = &lay.roles;
    let name = |c: usize| ds.cols.get(c).map(|c| c.name.clone()).unwrap_or_default();
    let text = |row: usize, c: usize| -> String {
        let x = cell(ds, row, Some(c));
        match ds.cols.get(c) {
            _ if x.is_null() => ctx.blank(),
            Some(col) => hits::cell(x, col, ctx),
            None => String::new(),
        }
    };
    let mut out = Vec::new();
    match l.kind {
        Kind::Category { level, .. } => {
            if let (Some(c), Some(m)) = (r.cats.get(level), l.path.last()) {
                let col = ds.cols.get(*c);
                let label = match col {
                    _ if m.is_null() => ctx.blank(),
                    Some(col) => hits::cell(m, col, ctx),
                    None => String::new(),
                };
                out.push((name(*c), label));
            }
        }
        Kind::Task { row } => {
            for c in r.cats.iter().chain(r.task.iter()) {
                out.push((name(*c), text(row, *c)));
            }
            for c in [r.start, r.end, r.progress]
                .into_iter()
                .flatten()
                .chain(r.details.iter().copied())
                .chain(r.tips.iter().copied())
            {
                out.push((name(c), text(row, c)));
            }
        }
    }
    out
}

/// The selection point of a line: its categories and, for a task, the task.
fn point(lay: &Layout, l: &Line) -> Point {
    let r = &lay.roles;
    let mut cols: Vec<usize> = r.cats.clone();
    if matches!(l.kind, Kind::Task { .. })
        && let Some(t) = r.task
    {
        cols.push(t);
    }
    cols.into_iter().zip(l.path.iter().cloned()).collect()
}

/// The row colour of a line.
fn row_color(v: &Value, ctx: &Ctx<'_>, l: &Line) -> Rgb {
    if flag(opt(v, "row", "color_by_category"), false) {
        return ctx.color(l.color);
    }
    color_of(opt(v, "row", "color")).unwrap_or_else(|| ctx.color(0))
}

/// The fill of a shape object (`bar`, `milestone`, `progress`): its own colour when customized.
fn shape_color(v: &Value, object: &str, base: Rgb) -> Rgb {
    // a colour set without the switch (the Format pane writes only the colour) applies too
    if flag(opt(v, object, "customize_color"), true) {
        color_of(opt(v, object, "color")).unwrap_or(base)
    } else {
        base
    }
}

/// A shape's border, when shown: (colour, width).
fn border(v: &Value, object: &str, fill: Rgb, ctx: &Ctx<'_>) -> Option<(Rgb, f32)> {
    if !flag(opt(v, object, "border_show"), false) {
        return None;
    }
    let c = if flag(opt(v, object, "border_match"), false) {
        fill
    } else {
        color_of(opt(v, object, "border_color")).unwrap_or(ctx.fg)
    };
    Some((c, number(opt(v, object, "border_width"), 1.0, 0.0, 20.0)))
}

fn dash_of(style: &str, width: f32) -> crate::canvas::Dash {
    match style {
        "dashed" => Some((width * 4.0, width * 3.0)),
        "dotted" => Some((width, width * 2.0)),
        _ => None,
    }
}

/// The expand/collapse icon of a category row (`row.expandCollapseToggleType`: 0 chevron, 1
/// plus/minus, 2 caret).
fn toggle_icon(cv: &mut Canvas, kind: u64, x: f32, cy: f32, open: bool, c: Rgb) {
    let s = 3.5;
    let cx = x + ICON_W / 2.0 - 2.0;
    match kind {
        1 => {
            cv.round_rect(cx - s, cy - s, 2.0 * s, 2.0 * s, 0.0, None, Some((c, 1.0, None)));
            cv.line(cx - s + 1.5, cy, cx + s - 1.5, cy, c, 1.0, None);
            if !open {
                cv.line(cx, cy - s + 1.5, cx, cy + s - 1.5, c, 1.0, None);
            }
        }
        2 => {
            let pts = if open {
                [
                    (cx - s, cy - s / 2.0),
                    (cx + s, cy - s / 2.0),
                    (cx, cy + s / 2.0 + 1.0),
                ]
            } else {
                [
                    (cx - s / 2.0, cy - s),
                    (cx - s / 2.0, cy + s),
                    (cx + s / 2.0 + 1.0, cy),
                ]
            };
            cv.polygon(&pts, c, 1.0);
        }
        _ => {
            let pts = if open {
                vec![(cx - s, cy - s / 2.0), (cx, cy + s / 2.0), (cx + s, cy - s / 2.0)]
            } else {
                vec![(cx - s / 2.0, cy - s), (cx + s / 2.0, cy), (cx - s / 2.0, cy + s)]
            };
            cv.polyline(&pts, c, 1.0, 1.3, None);
        }
    }
}

/// A milestone shape centred on `(cx, cy)` of size `s`.
fn milestone_shape(
    cv: &mut Canvas,
    shape: &str,
    cx: f32,
    cy: f32,
    s: f32,
    fill: (Rgb, f32),
    line: Option<(Rgb, f32)>,
) {
    let r = s / 2.0;
    let pts: Vec<(f32, f32)> = match shape {
        "circle" => {
            cv.circle(cx, cy, r, Some(fill), line);
            return;
        }
        "square" => vec![
            (cx - r, cy - r),
            (cx + r, cy - r),
            (cx + r, cy + r),
            (cx - r, cy + r),
        ],
        "triangle" => vec![(cx, cy - r), (cx + r, cy + r), (cx - r, cy + r)],
        _ => vec![(cx, cy - r), (cx + r, cy), (cx, cy + r), (cx - r, cy)],
    };
    cv.polygon(&pts, fill.0, fill.1);
    if let Some((c, w)) = line {
        let mut closed = pts.clone();
        if let Some(p) = pts.first() {
            closed.push(*p);
        }
        cv.polyline(&closed, c, 1.0, w, None);
    }
}

/// Draws the Gantt body: the number of lines below the body, not drawn; `None` when it has no
/// task field to list.
pub fn gantt_body(cv: &mut Canvas, v: &Value, ds: &Dataset, b: Bx, ctx: &Ctx<'_>) -> Option<usize> {
    let lay = layout(v, ds, b.w, b.h);
    lay.roles.task?;
    let (w, h) = (b.w, b.h);
    let rows_bottom = h - lay.footer_h;
    let rows_end = lay
        .lines
        .last()
        .map_or(lay.header_h, |l| l.y + lay.row_h)
        .min(rows_bottom);
    let value_px = number(opt(v, "column_value", "font_size"), 9.0, 5.0, 40.0) * PT;
    let header_px = number(opt(v, "column_headers", "font_size"), 9.0, 5.0, 40.0) * PT;
    let tick_px = number(opt(v, "axis_tick", "font_size"), 9.0, 5.0, 40.0) * PT;
    let ink = color_of(opt(v, "column_value", "color")).unwrap_or(ctx.fg);
    let head_ink = color_of(opt(v, "column_headers", "color")).unwrap_or(ctx.fg);
    let tick_ink = color_of(opt(v, "axis_tick", "color")).unwrap_or(ctx.fg2);
    let grid = color_of(opt(v, "axis", "gridline_color")).unwrap_or(ctx.grid);
    let indent = number(opt(v, "row", "indent"), INDENT, 0.0, 100.0);
    let toggle = fin(opt(v, "row", "toggle_type")).map_or(0, |t| t.clamp(0.0, 2.0) as u64);
    let icon_ink = color_of(opt(v, "row", "icon_color")).unwrap_or(ctx.fg2);

    // the column headers' band
    if lay.header_h > 0.0
        && let Some(bg) = color_of(opt(v, "column_headers", "background_color"))
    {
        let a = alpha_of(opt(v, "column_headers", "background_transparency"), 0.0);
        cv.rect(0.0, 0.0, lay.chart_x, lay.header_h, bg, a);
    }
    // category backgrounds: the row colour, lighter at each level
    let bg_show = flag(opt(v, "row", "background_show"), true);
    for l in &lay.lines {
        let Kind::Category { level, .. } = l.kind else {
            continue;
        };
        if !bg_show {
            break;
        }
        let custom = flag(opt(v, "row", "background_customize"), true);
        let (c, t) = match color_of(opt(v, "row", "background_color")) {
            Some(c) if custom => (c, alpha_of(opt(v, "row", "background_transparency"), 70.0)),
            _ => (
                row_color(v, ctx, l),
                1.0 - CATEGORY_TRANSPARENCY.get(level).copied().unwrap_or(0.9),
            ),
        };
        cv.rect(0.0, l.y, w, lay.row_h, c, t);
    }
    // shaded week days (where a day is at least 2 px wide)
    let shading = flag(opt(v, "axis", "shading_show"), false);
    if shading && let Some((lo, hi)) = lay.domain {
        let days: Vec<i64> = match get(gantt_axis(v), "shading_days").as_array() {
            Some(a) => a
                .iter()
                .filter_map(Value::as_i64)
                .filter(|d| (0..7).contains(d))
                .collect(),
            None => vec![0, 6],
        };
        let day_w = lay.chart_w / (hi - lo).max(1e-9) as f32;
        if day_w >= 2.0 && !days.is_empty() {
            let c = color_of(opt(v, "axis", "shading_color")).unwrap_or(ctx.band);
            let a = alpha_of(opt(v, "axis", "shading_transparency"), 0.0);
            let mut t = lo.floor();
            let mut n = 0;
            while t < hi && n < MAX_TICKS * 4 {
                let wd = bicraft_locale::calendar::weekday_of_serial(t as i64);
                if days.contains(&wd) {
                    let (x0, x1) = (lay.x_of(t.max(lo)), lay.x_of((t + 1.0).min(hi)));
                    cv.rect(
                        x0.min(x1),
                        lay.header_h,
                        (x1 - x0).abs(),
                        rows_end - lay.header_h,
                        c,
                        a,
                    );
                }
                t += 1.0;
                n += 1;
            }
        }
    }
    // gridlines at the finest level's units
    if flag(opt(v, "axis", "gridline_show"), true)
        && let Some(u) = lay.units.last().copied()
    {
        let width = number(opt(v, "axis", "gridline_thickness"), 1.0, 0.0, 10.0);
        let a = alpha_of(opt(v, "axis", "gridline_transparency"), 0.0);
        let dash = dash_of(sk(gantt_axis(v), "gridline_style"), width);
        let ts = ticks(&lay, u, v, ctx, lay.units.len() == 1);
        // too dense: every k-th
        let step = (ts.len() as f32 * 4.0 / lay.chart_w.max(1.0)).ceil().max(1.0) as usize;
        for (t, _) in ts.iter().step_by(step) {
            let x = lay.x_of(*t);
            cv.line_a(x, lay.header_h, x, rows_end, grid, a, width, dash, false);
        }
    }
    // row dividers
    if flag(opt(v, "row", "divider_show"), true) {
        let c = color_of(opt(v, "row", "divider_color")).unwrap_or(ctx.grid);
        let width = number(opt(v, "row", "divider_width"), 1.0, 0.0, 10.0);
        let a = alpha_of(opt(v, "row", "divider_transparency"), 0.0);
        let dash = dash_of(sk(get(gantt(v), "row"), "divider_style"), width);
        for l in &lay.lines {
            let y = l.y + lay.row_h - width / 2.0;
            cv.line_a(0.0, y, w, y, c, a, width, dash, false);
        }
    }
    // the separators of the task list and the axis band
    if lay.chart_x > 0.0 {
        cv.line(
            lay.chart_x - 0.5,
            0.0,
            lay.chart_x - 0.5,
            rows_bottom,
            ctx.grid,
            1.0,
            None,
        );
    }
    if lay.header_h > 0.0 {
        cv.line(
            0.0,
            lay.header_h - 0.5,
            w,
            lay.header_h - 0.5,
            ctx.grid,
            1.0,
            None,
        );
    }
    // column headers
    let pad = 4.0;
    if flag(opt(v, "column_headers", "show"), true) && lay.header_h > 0.0 {
        let f = text_font(v, "column_headers", ctx, header_px).weight(600);
        let f = if flag(opt(v, "column_headers", "bold"), false) {
            f.weight(700)
        } else {
            f
        };
        let y = lay.header_h - LEVEL_H / 2.0;
        let mut x = 0.0;
        let heads = lay.roles.task.into_iter().zip([lay.task_w]).chain(
            lay.roles
                .details
                .iter()
                .copied()
                .zip(lay.detail_w.iter().copied()),
        );
        for (c, cw) in heads {
            let name = ds.cols.get(c).map(|c| c.name.as_str()).unwrap_or("");
            let s = ctx.fonts.fit(&f, name, (cw - 2.0 * pad).max(0.0));
            cv.text(
                ctx.fonts,
                &Text::new(&s, f.clone(), head_ink).base(Base::Central),
                x + pad,
                y,
            );
            x += cw;
        }
    }
    // axis labels
    let tick_font = text_font(v, "axis_tick", ctx, tick_px);
    let single = lay.units.len() == 1;
    for (k, u) in lay.units.iter().enumerate() {
        let ts = ticks(&lay, *u, v, ctx, single);
        let widest = ts
            .iter()
            .map(|(_, s)| ctx.fonts.measure(&tick_font, s))
            .fold(0.0f32, f32::max);
        let Some((lo, hi)) = lay.domain else {
            break;
        };
        let unit_w = lay.chart_w / ts.len().max(1) as f32;
        let step = ((widest + 6.0) / unit_w.max(0.1)).ceil().max(1.0) as usize;
        let mut bands = Vec::new();
        if lay.top {
            bands.push(k as f32 * LEVEL_H);
        }
        if lay.bottom {
            bands.push(rows_bottom + k as f32 * LEVEL_H);
        }
        for by in bands {
            for (i, (t, s)) in ts.iter().enumerate() {
                let x = lay.x_of(t.max(lo));
                if k + 1 < lay.units.len() {
                    // an upper level: its boundaries across the band
                    cv.line(x, by, x, by + LEVEL_H, ctx.grid, 1.0, None);
                }
                if i % step != 0 {
                    continue;
                }
                let end = lay.x_of(u.next(*t).min(hi));
                let (x0, x1) = (x.min(end), x.max(end));
                let room = if step == 1 { x1 - x0 - 4.0 } else { widest + 4.0 };
                let s = ctx.fonts.fit(&tick_font, s, room.max(0.0));
                let tx = (x0 + 3.0).max(lay.chart_x + 2.0);
                if tx > w - 2.0 {
                    continue;
                }
                cv.text(
                    ctx.fonts,
                    &Text::new(&s, tick_font.clone(), tick_ink).base(Base::Central),
                    tx,
                    by + LEVEL_H / 2.0,
                );
            }
        }
    }
    // the lines: labels, details, bars, milestones, progress
    let value_font = text_font(v, "column_value", ctx, value_px);
    let bar_show = flag(opt(v, "bar", "show"), true);
    let ms_show = flag(opt(v, "milestone", "show"), true);
    let pr_show = flag(opt(v, "progress", "show"), true) && lay.roles.progress.is_some();
    let radius = number(opt(v, "bar", "corner_radius"), 2.0, 0.0, 40.0);
    let bar_a = alpha_of(opt(v, "bar", "transparency"), 0.0);
    let pr_a = alpha_of(opt(v, "progress", "transparency"), 0.0);
    let pr_rel = number(opt(v, "progress", "relative_height"), 100.0, 1.0, 100.0) / 100.0;
    let pr_radius = number(opt(v, "progress", "corner_radius"), radius, 0.0, 40.0);
    let ms_size = number(opt(v, "milestone", "size"), lay.bar_h, 2.0, 80.0);
    let ms_alpha = alpha_of(opt(v, "milestone", "transparency"), 0.0);
    let ms_shape = sk(get(gantt(v), "milestone"), "shape");
    for l in &lay.lines {
        let pt = point(&lay, l);
        let mut alpha = ctx.ix.alpha(&pt);
        if let (Some(hl), Kind::Task { row }) = (&ds.hl, &l.kind) {
            let lit = hl.get(*row).is_some_and(|r| {
                [lay.roles.start, lay.roles.end, lay.roles.progress]
                    .into_iter()
                    .flatten()
                    .any(|c| r.get(c).is_some_and(|x| !x.is_null()))
            });
            if !lit {
                alpha = hits::DIM;
            }
        }
        let cy = l.y + lay.row_h / 2.0;
        let is_cat = matches!(l.kind, Kind::Category { .. });
        // the label, indented, with its icon
        let x = pad + l.level as f32 * indent;
        let lx = if let Kind::Category { collapsed, .. } = l.kind {
            toggle_icon(cv, toggle, x, cy, !collapsed, icon_ink);
            x + ICON_W
        } else if lay.roles.cats.is_empty() {
            x
        } else {
            x + ICON_W
        };
        let raw = l.path.last().unwrap_or(&Value::Null);
        let col = if is_cat {
            if let Kind::Category { level, .. } = l.kind {
                lay.roles.cats.get(level).and_then(|c| ds.cols.get(*c))
            } else {
                None
            }
        } else {
            lay.roles.task.and_then(|c| ds.cols.get(c))
        };
        let label = match col {
            _ if raw.is_null() => ctx.blank(),
            Some(c) => hits::cell(raw, c, ctx),
            None => String::new(),
        };
        let f = if is_cat {
            value_font.clone().weight(600)
        } else {
            value_font.clone()
        };
        let s = ctx.fonts.fit(&f, &label, (lay.task_w - lx - pad).max(0.0));
        cv.text(
            ctx.fonts,
            &Text {
                alpha,
                ..Text::new(&s, f.clone(), ink).base(Base::Central)
            },
            lx,
            cy,
        );
        if let Kind::Task { row } = l.kind {
            let mut dx = lay.task_w;
            for (c, cw) in lay.roles.details.iter().zip(&lay.detail_w) {
                let x = cell(ds, row, Some(*c));
                let s = match ds.cols.get(*c) {
                    _ if x.is_null() => String::new(),
                    Some(col) => hits::cell(x, col, ctx),
                    None => String::new(),
                };
                let num = ds.cols.get(*c).is_some_and(Col::is_num_type);
                let s = ctx.fonts.fit(&value_font, &s, (cw - 2.0 * pad).max(0.0));
                let (tx, anchor) = match sk(get(gantt(v), "column_value"), "alignment") {
                    "center" => (dx + cw / 2.0, Anchor::Middle),
                    "right" => (dx + cw - pad, Anchor::End),
                    "left" => (dx + pad, Anchor::Start),
                    _ if num => (dx + cw - pad, Anchor::End),
                    _ => (dx + pad, Anchor::Start),
                };
                cv.text(
                    ctx.fonts,
                    &Text {
                        alpha,
                        ..Text::new(&s, value_font.clone(), ink)
                            .anchor(anchor)
                            .base(Base::Central)
                    },
                    tx,
                    cy,
                );
                dx += cw;
            }
        }
        // the bar, clipped to the time line
        let base = row_color(v, ctx, l);
        let (cx0, cx1) = (lay.chart_x, lay.chart_x + lay.chart_w);
        if bar_show && let Some((bx, bw)) = lay.bar(l) {
            let (x0, x1) = (bx.max(cx0), (bx + bw).min(cx1));
            if x1 > x0 {
                if is_cat {
                    // a category: a thin summary bar over its members' dates
                    let th = (lay.bar_h * 0.45).max(2.0);
                    cv.rect(x0, cy - th / 2.0, x1 - x0, th, base, alpha);
                } else {
                    let fill = shape_color(v, "bar", base);
                    let y0 = cy - lay.bar_h / 2.0;
                    cv.round_rect(
                        x0,
                        y0,
                        x1 - x0,
                        lay.bar_h,
                        radius,
                        Some((fill, bar_a * alpha)),
                        border(v, "bar", fill, ctx).map(|(c, w)| (c, w, None)),
                    );
                    if pr_show && let Some(p) = l.progress.filter(|p| *p > 0.0) {
                        // done: a darker shade of the bar unless the progress has its own colour
                        let pf = shape_color(v, "progress", fill.mix(Rgb(0, 0, 0), PROGRESS_SHADE));
                        let ph = lay.bar_h * pr_rel;
                        let pw = bw * p as f32;
                        // done from the start: the left end, or the right when inverted
                        let (p0, p1) = if lay.invert {
                            (bx + bw - pw, bx + bw)
                        } else {
                            (bx, bx + pw)
                        };
                        let (p0, p1) = (p0.max(cx0), p1.min(cx1));
                        if p1 > p0 {
                            cv.round_rect(
                                p0,
                                cy - ph / 2.0,
                                p1 - p0,
                                ph,
                                pr_radius,
                                Some((pf, pr_a * alpha)),
                                border(v, "progress", pf, ctx).map(|(c, w)| (c, w, None)),
                            );
                        }
                    }
                }
            }
        }
        if ms_show
            && let Some(mx) = lay.milestone(l)
            && mx >= cx0
            && mx <= cx1
        {
            let fill = shape_color(v, "milestone", base);
            milestone_shape(
                cv,
                ms_shape,
                mx,
                cy,
                ms_size,
                (fill, ms_alpha * alpha),
                border(v, "milestone", fill, ctx),
            );
        }
        if ctx.ix.on() {
            cv.hit(ctx.ix.hit(
                HitShape::Rect {
                    x: 0.0,
                    y: l.y,
                    w,
                    h: lay.row_h,
                },
                ctx.ix.select(&pt),
                tip(&lay, ds, l, ctx),
            ));
        }
    }
    Some(lay.cut)
}
