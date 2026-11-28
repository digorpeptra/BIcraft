//! Tables and matrices: header, rows that fit, totals; conditional formatting (backgrounds,
//! font colors, data bars, icons, web URLs) and column styles. Ported from the live viewer's
//! `gridTable`, `tableBody` and `matrixBody`.

use std::collections::HashMap;

use serde_json::Value;

use crate::canvas::{Base, Canvas, Part, Rgb, Text};
use crate::charts::{Bx, font};
use crate::data::{
    CellStyle, Col, Ctx, Dataset, Fmt, LevelTotal, Rc, SparkStyle, clampf, cmp_raw, fin, is_num_col, key_of,
    spark_points, spark_x,
};
use crate::fmt::{fmt_cell, fmt_parts, fmt_val};

/// One cell of the grid.
#[derive(Debug, Clone, Default)]
pub struct Cell {
    pub t: String,
    pub bold: bool,
    pub ind: usize,
    pub bg: Option<Rgb>,
    pub fg: Option<Rgb>,
    pub bar: Option<(f32, f32, Rgb)>,
    /// The data bar's axis line (place in the cell, colour).
    pub bar_axis: Option<(f32, Rgb)>,
    /// Show bar only: no text.
    pub bar_only: bool,
    pub icon: Option<(String, String, String)>,
    pub url: bool,
    /// A matrix row header's +/- button: `Some(true)` expanded (−), `Some(false)` collapsed (+).
    pub btn: Option<bool>,
    /// A sparkline drawn in the cell: its points' values, their places on the column's X range
    /// (0–1, when it has one) and look.
    pub spark: Option<(Vec<Option<f64>>, Option<Vec<f32>>, SparkStyle)>,
}

/// The cell of value `x` in sparkline column `st`: the small chart, no text.
fn spark_cell(x: &Value, st: &SparkStyle) -> Cell {
    let at = st.domain.and_then(|(lo, hi)| {
        x.get("sparkline")
            .and_then(Value::as_array)?
            .iter()
            .take(100)
            .map(|p| {
                let v = spark_x(p)?;
                Some(if hi > lo {
                    ((v - lo) / (hi - lo)) as f32
                } else {
                    0.5
                })
            })
            .collect::<Option<Vec<f32>>>()
    });
    Cell {
        spark: spark_points(x).map(|p| (p, at, st.clone())),
        ..Cell::default()
    }
}

/// Least width of a sparkline column (in text pixels): room for a few points.
const SPARK_W: f32 = 7.0;

/// A sparkline in the box (x, y, w, h): a line through the points (a blank breaks it) or a
/// column per point from zero, with the markers its look asks for (every point, highest, lowest,
/// first, last, negative, isolated). A line with `xs` (each point's place on the column's X
/// range, 0–1) puts its points there, as Desktop does over dates and numbers; else evenly.
pub fn draw_spark(
    cv: &mut Canvas,
    b: (f32, f32, f32, f32),
    ys: &[Option<f64>],
    xs: Option<&[f32]>,
    st: &SparkStyle,
    accent: Rgb,
) {
    let (x, y, w, h) = b;
    let vals: Vec<f64> = ys.iter().flatten().copied().filter(|v| v.is_finite()).collect();
    if vals.is_empty() || w < 2.0 || h < 2.0 {
        return;
    }
    let mut lo = vals.iter().copied().fold(f64::INFINITY, f64::min);
    let mut hi = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if st.column {
        lo = lo.min(0.0);
        hi = hi.max(0.0);
    }
    let span = if hi > lo { hi - lo } else { 1.0 };
    let n = ys.len().max(1);
    let color = st.color.unwrap_or(accent);
    let py = |v: f64| -> f32 {
        let t = if hi > lo { ((v - lo) / span) as f32 } else { 0.5 };
        y + h - t.clamp(0.0, 1.0) * h
    };
    let px = |i: usize| -> f32 {
        if st.column {
            x + (i as f32 + 0.5) * w / n as f32
        } else if let Some(t) = xs.and_then(|a| a.get(i)) {
            x + t.clamp(0.0, 1.0) * w
        } else if n == 1 {
            x + w / 2.0
        } else {
            x + i as f32 * w / (n - 1) as f32
        }
    };
    if st.column {
        let bw = (w / n as f32 * 0.7).max(1.0);
        let zero = py(0.0);
        for (i, v) in ys.iter().enumerate() {
            let Some(v) = v.filter(|v| v.is_finite()) else {
                continue;
            };
            let top = py(v);
            let (a, bot) = if top < zero { (top, zero) } else { (zero, top) };
            cv.rect(px(i) - bw / 2.0, a, bw, (bot - a).max(1.0), color, 1.0);
        }
    } else {
        let mut run: Vec<(f32, f32)> = Vec::new();
        for (i, v) in ys.iter().enumerate() {
            match v.filter(|v| v.is_finite()) {
                Some(v) => run.push((px(i), py(v))),
                None => {
                    if run.len() > 1 {
                        cv.polyline(&run, color, 1.0, st.width, None);
                    }
                    run.clear();
                }
            }
        }
        if run.len() > 1 {
            cv.polyline(&run, color, 1.0, st.width, None);
        } else if let Some((cx, cy)) = run.first() {
            cv.circle(*cx, *cy, st.width.max(1.5), Some((color, 1.0)), None);
        }
    }
    if st.markers == 0 {
        return;
    }
    let shown: Vec<(usize, f64)> = ys
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.filter(|v| v.is_finite()).map(|v| (i, v)))
        .collect();
    let mut at: Vec<usize> = Vec::new();
    let pick = |bit: u32| st.markers & bit != 0;
    if pick(1) {
        at.extend(shown.iter().map(|(i, _)| *i));
    }
    if pick(2)
        && let Some((i, _)) = shown.iter().copied().reduce(|a, b| if b.1 > a.1 { b } else { a })
    {
        at.push(i);
    }
    if pick(4)
        && let Some((i, _)) = shown.iter().copied().reduce(|a, b| if b.1 < a.1 { b } else { a })
    {
        at.push(i);
    }
    if pick(8)
        && let Some((i, _)) = shown.first()
    {
        at.push(*i);
    }
    if pick(16)
        && let Some((i, _)) = shown.last()
    {
        at.push(*i);
    }
    if pick(32) {
        at.extend(shown.iter().filter(|(_, v)| *v < 0.0).map(|(i, _)| *i));
    }
    // isolated: a point with no point next to it on either side
    if pick(128) {
        let blank = |i: Option<usize>| {
            i.and_then(|i| ys.get(i))
                .is_none_or(|v| !v.is_some_and(f64::is_finite))
        };
        at.extend(
            shown
                .iter()
                .filter(|(i, _)| blank(i.checked_sub(1)) && blank(i.checked_add(1)))
                .map(|(i, _)| *i),
        );
    }
    at.sort_unstable();
    at.dedup();
    let mc = st.marker_color.unwrap_or(color);
    let r = st.marker_size / 2.0;
    for i in at {
        let Some(v) = ys.get(i).copied().flatten() else {
            continue;
        };
        let (cx, cy) = (px(i), py(v));
        match st.marker_shape.as_str() {
            "square" => cv.rect(cx - r, cy - r, r * 2.0, r * 2.0, mc, 1.0),
            "diamond" => cv.polygon(&[(cx, cy - r), (cx + r, cy), (cx, cy + r), (cx - r, cy)], mc, 1.0),
            "triangle" => cv.polygon(&[(cx, cy - r), (cx + r, cy + r), (cx - r, cy + r)], mc, 1.0),
            "x" => {
                cv.line(cx - r, cy - r, cx + r, cy + r, mc, 1.5, None);
                cv.line(cx - r, cy + r, cx + r, cy - r, mc, 1.5, None);
            }
            "plus" => {
                cv.line(cx - r, cy, cx + r, cy, mc, 1.5, None);
                cv.line(cx, cy - r, cx, cy + r, mc, 1.5, None);
            }
            "shortDash" => cv.rect(cx - r * 0.6, cy - 0.75, r * 1.2, 1.5, mc, 1.0),
            "longDash" => cv.rect(cx - r, cy - 0.75, r * 2.0, 1.5, mc, 1.0),
            _ => cv.circle(cx, cy, r, Some((mc, 1.0)), None),
        }
    }
}

impl Cell {
    fn text(t: impl Into<String>) -> Cell {
        Cell {
            t: t.into(),
            ..Cell::default()
        }
    }
    fn styled(t: String, st: Option<&CellStyle>, cs: Option<&(Option<Rgb>, Option<Rgb>)>) -> Cell {
        Cell {
            t,
            bg: st.and_then(|s| s.bg).or(cs.and_then(|c| c.0)),
            fg: st.and_then(|s| s.fg).or(cs.and_then(|c| c.1)),
            bar: st.and_then(|s| s.bar),
            bar_axis: st.and_then(|s| s.bar_axis),
            bar_only: st.is_some_and(|s| s.bar_only),
            icon: st.and_then(|s| s.icon.clone()),
            url: st.is_some_and(|s| s.url),
            ..Cell::default()
        }
    }
}

pub struct Head {
    pub t: String,
    pub num: bool,
    /// Drawn bold (a matrix's total column).
    pub bold: bool,
}

/// Most column groups of a matrix (Desktop scrolls further ones into view).
const MAX_MATRIX_COLUMNS: usize = 60;
/// Most levels of a matrix's row or column hierarchy drawn (the engine evaluates as many).
const MAX_MATRIX_LEVELS: usize = 8;

const ICON_GREEN: Rgb = Rgb(0x1a, 0xab, 0x40);
const ICON_AMBER: Rgb = Rgb(0xf2, 0xc8, 0x11);
const ICON_RED: Rgb = Rgb(0xd6, 0x45, 0x54);
const ICON_GRAY: Rgb = Rgb(0x8a, 0x88, 0x86);

/// A conditional-format icon (Desktop's icon sets) in a `size` box at (x, y).
pub fn draw_icon(cv: &mut Canvas, name: &str, x: f32, y: f32, size: f32) {
    let n = name.to_lowercase();
    let has = |k: &str| n.contains(k);
    let ends = |k: &str| n.ends_with(k);
    let level = if has("gray") || has("grey") {
        "gray"
    } else if has("high") || has("upright") || ends("up") || has("check") || has("full") || has("green") {
        "high"
    } else if has("low")
        || has("downright")
        || ends("down")
        || has("cross")
        || has("empty")
        || has("red")
        || ends("x")
    {
        "low"
    } else {
        "mid"
    };
    let color = match level {
        "gray" => ICON_GRAY,
        "high" => ICON_GREEN,
        "low" => ICON_RED,
        _ => ICON_AMBER,
    };
    let k = size / 16.0;
    let t = tiny_skia::Transform::from_translate(x, y).pre_scale(k, k);
    let fill = |cv: &mut Canvas, d: &str, c: Rgb, a: f32| {
        if let Some(p) = crate::canvas::svg_path(d) {
            cv.fill_path_tf(&p, c, a, t);
        }
    };
    let stroke = |cv: &mut Canvas, d: &str, c: Rgb, w: f32, rot: f32| {
        if let Some(p) = crate::canvas::svg_path(d) {
            cv.stroke_tf(
                &p,
                c,
                w,
                t.pre_concat(tiny_skia::Transform::from_rotate_at(rot, 8.0, 8.0)),
            );
        }
    };
    if has("arrow") {
        let rot = if has("upright") {
            -45.0
        } else if has("downright") {
            45.0
        } else if ends("up") {
            -90.0
        } else if ends("down") {
            90.0
        } else {
            0.0
        };
        stroke(cv, "M2 8h10M8.5 4l4 4-4 4", color, 2.2, rot);
    } else if has("triangle") {
        let d = match level {
            "high" => "M8 2.5l6 10H2z",
            "low" => "M8 13.5l6-10H2z",
            _ => "M3 6.5h10v3H3z",
        };
        fill(cv, d, color, 1.0);
    } else if has("flag") {
        stroke(cv, "M4 2v12", Rgb(0x60, 0x5e, 0x5c), 1.4, 0.0);
        fill(cv, "M4.7 2.5h8l-2 3 2 3h-8z", color, 1.0);
    } else if has("star") {
        let d = "M8 1.5l1.9 4.2 4.6.5-3.4 3.1 1 4.5L8 11.5l-4.1 2.3 1-4.5-3.4-3.1 4.6-.5z";
        if level != "low" {
            fill(cv, d, ICON_AMBER, if level == "mid" { 0.45 } else { 1.0 });
        }
        stroke(cv, d, ICON_AMBER, 1.0, 0.0);
    } else if has("sign") && level == "mid" {
        fill(cv, "M8 2l6.5 11.5h-13z", color, 1.0);
    } else if has("sign") && level == "low" {
        fill(cv, "M8 1.5l6.5 6.5L8 14.5 1.5 8z", color, 1.0);
    } else if has("symbol") || has("check") || has("cross") || has("exclamation") {
        fill(cv, "M8 1.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13z", color, 1.0);
        let mark = match level {
            "high" => "M5 8.2l2 2 4-4.4",
            "low" => "M5.5 5.5l5 5M10.5 5.5l-5 5",
            _ => "M8 4.5v4.5M8 11v.2",
        };
        stroke(cv, mark, Rgb::WHITE, 1.8, 0.0);
    } else {
        fill(cv, "M8 2a6 6 0 1 0 0 12 6 6 0 0 0 0-12z", color, 1.0);
        if has("traffic") {
            stroke(
                cv,
                "M8 2a6 6 0 1 0 0 12 6 6 0 0 0 0-12z",
                Rgb(0x25, 0x24, 0x23),
                1.5,
                0.0,
            );
        }
    }
}

/// Table/matrix layout settings from the snapshot's `format.table` (theme and visual):
/// grid row padding, value and header font sizes (pt), explicit column widths (by grid column)
/// and the sorted grid columns.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub row_padding: Option<f32>,
    pub font_pt: Option<f32>,
    pub header_pt: Option<f32>,
    pub widths: HashMap<usize, f32>,
    /// (grid column, descending)
    pub sort: Vec<(usize, bool)>,
    /// Rows of the whole grid when more than those drawn (the engine's `total_rows`): the
    /// scrollbar's content.
    pub rows: Option<usize>,
    /// A table's rows as data points (the viewer's `rowHit`): each row's grouping columns and
    /// its tooltip; empty: rows are not clickable.
    pub row_hits: Vec<(crate::hits::Point, Vec<(String, String)>)>,
    /// A matrix's row headers, per body row: the member path and its +/- state (`None` inside:
    /// no button); `None`: a total row. Each becomes a menu hit (and a button hit).
    pub row_menu: Vec<Option<(Vec<Value>, Option<bool>)>>,
}

impl Layout {
    /// Reads `format.table`; `col_of` maps a data column to its grid column.
    pub fn of(f: &Fmt, col_of: &dyn Fn(usize) -> Option<usize>) -> Layout {
        let t = f.raw.get("table").unwrap_or(&Value::Null);
        let num =
            |k: &str, lo: f64, hi: f64| t.get(k).and_then(Value::as_f64).map(|x| x.clamp(lo, hi) as f32);
        let idx = |e: &Value| {
            e.get("column")
                .and_then(Value::as_u64)
                .and_then(|c| usize::try_from(c).ok())
                .and_then(col_of)
        };
        let mut widths = HashMap::new();
        for e in t
            .get("column_widths")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(200)
        {
            if let (Some(i), Some(w)) = (idx(e), e.get("width").and_then(Value::as_f64)) {
                widths.insert(i, w.clamp(1.0, 10_000.0) as f32);
            }
        }
        let sort = t
            .get("sort")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(50)
            .filter_map(|e| {
                Some((
                    idx(e)?,
                    e.get("descending").and_then(Value::as_bool) == Some(true),
                ))
            })
            .collect();
        Layout {
            row_padding: num("row_padding", 0.0, 40.0),
            font_pt: num("font_size", 6.0, 40.0),
            header_pt: num("header_font_size", 6.0, 40.0),
            widths,
            sort,
            rows: None,
            row_hits: Vec::new(),
            row_menu: Vec::new(),
        }
    }
}

/// Text line height of a Segoe UI `px` font in a Desktop 2.158 grid cell (19 px at 14 px).
fn grid_line(px: f32) -> f32 {
    (px * 1.36).round()
}

/// Room for the vertical scrollbar of a themed grid whose rows overflow (Desktop 2.158).
const SCROLL_W: f32 = 19.0;
/// Scrollbar thumbs (Desktop 2.158): 8 px pills in #AFAEAD along the body's right and bottom
/// edges, their track 1.5 px in from the start and 2.5 px from the end, never shorter than
/// their width; the length is the visible share of the content.
const THUMB_W: f32 = 8.0;
const THUMB_COLOR: Rgb = Rgb(0xaf, 0xae, 0xad);
/// A matrix's stepped-layout indentation per level (Desktop 2.158: 10 px at any font size,
/// `oracle-gen expand`).
const STEP_INDENT: f32 = 10.0;
/// Extra header height that holds the sort arrow.
const SORT_ROOM: f32 = 8.0;

/// A scrollbar thumb along a `len` long edge showing `view` of `content`.
fn thumb_len(len: f32, view: f32, content: f32) -> f32 {
    let track = (len - 4.0).max(THUMB_W);
    (track * (view / content.max(1.0))).clamp(THUMB_W.min(track), track)
}

/// Draws the grid: column widths from the content (like the viewer) or the visual's
/// `columnWidth`, header row(s), the rows that fit and the total row. Rows that overflow are
/// Desktop's: the row cut by the bottom is drawn clipped, the total row stays at the bottom and
/// a scrollbar thumb shows the overflow; columns wider than the visual keep their width, are
/// cut at its right edge and get a horizontal scrollbar. `top` are a matrix's column-group
/// header rows (text, column span, bold).
#[allow(clippy::too_many_arguments)]
pub fn grid_table(
    cv: &mut Canvas,
    ctx: &Ctx<'_>,
    b: Bx,
    heads: &[Head],
    body: &[Vec<Cell>],
    total: Option<&[Cell]>,
    top: &[Vec<(String, usize, bool)>],
    row_header: bool,
    lay: &Layout,
) {
    // Desktop 2.158: a report without a theme gets compact tables (columns sized to their
    // content, accent rules); themed reports get taller rows, a shaded header and banded rows
    let classic = ctx.classic;
    // classic grids write in #333333 (Desktop 2.158)
    let fg = if classic { Rgb(0x33, 0x33, 0x33) } else { ctx.fg };
    let px = lay.font_pt.map_or(ctx.px, |p| p * crate::data::PT);
    // the header keeps its own size (a visual's value font size does not change it)
    let hpx = lay.header_pt.map_or(ctx.px, |p| p * crate::data::PT);
    // classic headers always hold the sort arrow's room; themed ones when sorted
    let sorted = !lay.sort.is_empty() && (classic || lay.row_padding.is_some());
    let arrow_room = sorted || classic;
    // themed rows end with a 1 px rule
    let rule_px = if classic { 0.0 } else { 1.0 };
    // classic: 8 pt text in 17 px rows (matrices 16 px), 6 px cell padding (Desktop 2.158)
    let row_h = match lay.row_padding {
        Some(p) => grid_line(px) + 2.0 * p + rule_px,
        None if classic && row_header => (px * 1.5).round(),
        None if classic => (px * 1.6).round(),
        None => (px * 2.3).round(),
    };
    let head_h = match lay.row_padding {
        Some(p) => grid_line(hpx) + 2.0 * p + rule_px + if arrow_room { SORT_ROOM } else { 0.0 },
        None if classic => ((hpx * 1.6).round() * 1.4).round(),
        None => (hpx * 2.6).round(),
    };
    let pad = if classic { px * 0.5625 } else { px * 0.45 };
    let tops = row_h * top.len() as f32;
    let total_h = if total.is_some() { row_h } else { 0.0 };
    let f_norm = font(ctx, px);
    let f_head = font(ctx, hpx);
    let head_weight = if classic { 400 } else { 600 };
    // natural widths: the widest text of each column (header, rows, total), plus padding
    let sample: Vec<&Vec<Cell>> = body.iter().take(201).collect();
    let natural: Vec<f32> = heads
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let hw = if h.bold { 700 } else { head_weight };
            let mut m = ctx
                .fonts
                .measure(&f_head.clone().weight(hw), &h.t)
                .min(hpx * 16.0);
            // a one-column label of the column-group row (the column field's name)
            for top_row in top {
                let mut at = 0usize;
                for (t, span, bold) in top_row {
                    if at == i && *span <= 1 {
                        let w = if *bold { 700 } else { head_weight };
                        m = m.max(ctx.fonts.measure(&f_head.clone().weight(w), t).min(hpx * 16.0));
                    }
                    at = at.saturating_add((*span).max(1));
                }
            }
            // the total row is drawn bold
            let cells = sample
                .iter()
                .filter_map(|r| r.get(i))
                .map(|c| (c, c.bold))
                .chain(total.and_then(|t| t.get(i)).map(|c| (c, true)));
            for (c, bold) in cells {
                let f = f_norm.clone().weight(if bold { 700 } else { 400 });
                let icon = if c.icon.is_some() { px * 1.3 } else { 0.0 };
                let icon = icon + if c.btn.is_some() { px * 1.3 } else { 0.0 };
                m = m.max(ctx.fonts.measure(&f, &c.t) + icon + c.ind as f32 * STEP_INDENT);
                if c.spark.is_some() {
                    m = m.max(px * SPARK_W);
                }
            }
            // +1 px: rounding must not turn the widest text into an ellipsis
            m.min(px * 30.0) + pad * 2.0 + 1.0
        })
        .collect();
    let sum: f32 = natural.iter().sum::<f32>().max(1.0);
    let set_w: f32 = natural
        .iter()
        .enumerate()
        .map(|(i, w)| lay.widths.get(&i).copied().unwrap_or(*w))
        .sum();
    // columns wider than the visual scroll sideways (Desktop keeps their widths)
    let content_w = if lay.widths.is_empty() { sum } else { set_w };
    let hscroll = content_w > b.w + 0.5;
    // the rows' room, under a horizontal scrollbar's strip
    let grid_h = if hscroll { (b.h - THUMB_W).max(0.0) } else { b.h };
    let room = (grid_h - head_h - tops - total_h).max(0.0);
    let fit_n = clampf((room / row_h).floor(), 0.0, 200.0) as usize;
    let n_rows = lay.rows.unwrap_or(body.len()).max(body.len());
    // Desktop's way: the rows that start inside the body (the last one cut), then a scrollbar
    let scrolls = n_rows > fit_n && (classic || lay.row_padding.is_some());
    let shown_n = if scrolls {
        clampf((room / row_h).ceil(), 0.0, 201.0) as usize
    } else {
        fit_n
    };
    let rows: Vec<&Vec<Cell>> = body.iter().take(shown_n).collect();
    let full_w = b.w;
    // the grid's visible width: left of the vertical scrollbar
    let view_w = if scrolls {
        (b.w - if classic { THUMB_W } else { SCROLL_W }).max(1.0)
    } else {
        b.w
    };
    // themed tables grow to the visual's width; classic ones keep their natural width; the
    // visual's own column widths are kept as set
    let k = if !hscroll && !classic { view_w / sum } else { 1.0 };
    let col_w: Vec<f32> = if lay.widths.is_empty() {
        natural.iter().map(|w| w * k).collect()
    } else {
        natural
            .iter()
            .enumerate()
            .map(|(i, w)| lay.widths.get(&i).copied().unwrap_or(*w))
            .collect()
    };
    let table_w: f32 = col_w.iter().sum();
    let col_x: Vec<f32> = col_w
        .iter()
        .scan(0.0f32, |acc, w| {
            let x = *acc;
            *acc += w;
            Some(x)
        })
        .collect();
    let draw_cell = |cv: &mut Canvas,
                     c: &Cell,
                     i: usize,
                     y: f32,
                     h: f32,
                     base: &crate::fonts::Font,
                     weight: u16,
                     color: Rgb,
                     num: bool| {
        let (x, w) = (
            col_x.get(i).copied().unwrap_or(0.0),
            col_w.get(i).copied().unwrap_or(0.0),
        );
        if x > view_w {
            return;
        }
        if let Some(bg) = c.bg {
            cv.rect(x, y, w, h, bg, 1.0);
        }
        if let Some((from, to, bc)) = c.bar {
            cv.rect(x + w * from, y, (w * (to - from)).max(0.0), h, bc, 1.0);
        }
        if let Some((at, ac)) = c.bar_axis {
            cv.rect((x + w * at).floor(), y, 1.0, h, ac, 1.0);
        }
        if let Some((ys, at, st)) = &c.spark {
            let inset = (h * 0.18).max(2.0);
            let b = (
                x + pad,
                y + inset,
                (w - pad * 2.0).max(0.0),
                (h - inset * 2.0).max(0.0),
            );
            draw_spark(cv, b, ys, at.as_deref(), st, ctx.color(0));
            return;
        }
        let fnt = base.clone().weight(if c.bold { weight.max(700) } else { weight });
        let mut left_pad = pad + c.ind as f32 * STEP_INDENT;
        let icon_sz = px;
        if let Some(open) = c.btn {
            expand_button(cv, x + left_pad, y + h / 2.0, px * 0.85, open, color);
            left_pad += px * 1.3;
        }
        let mut avail = w - left_pad - pad;
        let only = c.icon.as_ref().is_some_and(|i| i.1 == "only");
        if c.icon.is_some() && !only {
            avail -= icon_sz + px * 0.3;
        }
        let t = if only || c.bar_only {
            String::new()
        } else {
            ctx.fonts.fit(&fnt, &c.t, avail)
        };
        let tw = ctx.fonts.measure(&fnt, &t);
        let content_w = tw
            + if c.icon.is_some() {
                icon_sz + if only { 0.0 } else { px * 0.3 }
            } else {
                0.0
            };
        let mut cx = if num {
            x + w - pad - content_w
        } else {
            x + left_pad
        };
        let cy = y + h / 2.0;
        let after = c.icon.as_ref().is_some_and(|i| i.1 == "after");
        if let Some((name, _, _)) = &c.icon
            && !after
        {
            draw_icon(cv, name, cx, cy - icon_sz / 2.0, icon_sz);
            cx += icon_sz + if only { 0.0 } else { px * 0.3 };
        }
        if !t.is_empty() {
            cv.text(
                ctx.fonts,
                &Text::new(&t, fnt, c.fg.unwrap_or(color))
                    .base(Base::Central)
                    .underline(c.url),
                cx,
                cy,
            );
            cx += tw + px * 0.3;
        }
        if let Some((name, _, _)) = &c.icon
            && after
        {
            draw_icon(cv, name, cx, cy - icon_sz / 2.0, icon_sz);
        }
    };
    // a grid cut at the vertical scrollbar is drawn apart, then composited
    let mut cut = if view_w < b.w && table_w > view_w {
        cv.layer(0.0, 0.0, view_w, grid_h.max(1.0))
    } else {
        None
    };
    {
        let g: &mut Canvas = match cut.as_mut() {
            Some(c) => c,
            None => &mut *cv,
        };
        let rule = ctx.accent;
        let mut y = 0.0f32;
        // a table visual is a table of the PDF's structure: a header row, then the rows and the
        // total row shown (a matrix's spanning headers are read as paragraphs)
        let tag = !row_header && top.is_empty();
        let tag_row = |c: &Canvas, open: bool| {
            if tag {
                if open {
                    c.begin_part(Part::Row);
                } else {
                    c.end_part();
                }
            }
        };
        let tag_cell = |c: &Canvas, part: Part| {
            if tag {
                c.begin_part(part);
            }
        };
        let untag = |c: &Canvas| {
            if tag {
                c.end_part();
            }
        };
        tag_cell(g, Part::Table);
        if !classic {
            g.rect(0.0, 0.0, table_w, tops + head_h, ctx.band, 1.0);
        }
        for top_row in top {
            let mut i = 0usize;
            for (t, span, bold) in top_row {
                let span = (*span).max(1);
                let x = col_x.get(i).copied().unwrap_or(0.0);
                let w: f32 = col_w.iter().skip(i).take(span).sum();
                let fnt = f_head.clone().weight(if *bold { 700 } else { head_weight });
                let s = ctx.fonts.fit(&fnt, t, w - pad * 2.0);
                if x <= view_w {
                    g.text(
                        ctx.fonts,
                        &Text::new(&s, fnt, fg).base(Base::Central),
                        x + pad,
                        y + row_h / 2.0,
                    );
                }
                i += span;
            }
            y += row_h;
        }
        let text_h = head_h - if arrow_room { SORT_ROOM } else { 0.0 };
        tag_row(g, true);
        for (i, h) in heads.iter().enumerate() {
            // headers are left aligned (Desktop 2.158, classic and themed)
            let hw = if h.bold { 700 } else { head_weight };
            tag_cell(g, Part::HeadCell);
            draw_cell(g, &Cell::text(h.t.clone()), i, y, text_h, &f_head, hw, fg, false);
            // the sort arrow under the header text (Desktop 2.158)
            if sorted && let Some((_, desc)) = lay.sort.iter().find(|(c, _)| *c == i) {
                let x = col_x.get(i).copied().unwrap_or(0.0) + pad;
                let (y0, y1) = (y + text_h - 3.0, y + text_h + 2.0);
                let pts = if *desc {
                    [(x, y0), (x + 8.0, y0), (x + 4.0, y1)]
                } else {
                    [(x, y1), (x + 8.0, y1), (x + 4.0, y0)]
                };
                g.polygon(&pts, fg, 1.0);
            }
            untag(g);
        }
        tag_row(g, false);
        y += head_h;
        if classic {
            g.rect(0.0, y - 1.0, table_w, 1.0, rule, 1.0);
        }
        let body_top = y;
        // the body, clipped to its room when the rows overflow
        let mut clip = if scrolls {
            g.layer(0.0, body_top, table_w.max(1.0), room.max(1.0))
        } else {
            None
        };
        let mut ry = if clip.is_some() { 0.0 } else { y };
        for (k, r) in rows.iter().enumerate() {
            let target: &mut Canvas = match clip.as_mut() {
                Some(c) => c,
                None => &mut *g,
            };
            if k % 2 == 1 {
                target.rect(0.0, ry, table_w, row_h, ctx.band, 1.0);
            }
            tag_row(target, true);
            for (i, h) in heads.iter().enumerate() {
                tag_cell(target, Part::Cell);
                if let Some(c) = r.get(i) {
                    draw_cell(target, c, i, ry, row_h, &f_norm, 400, fg, h.num);
                }
                untag(target);
            }
            tag_row(target, false);
            if let Some((pt, tip)) = lay.row_hits.get(k) {
                // a row the selection leaves out fades under a white veil
                if ctx.ix.selected(pt) == Some(false) {
                    target.rect(0.0, ry, table_w, row_h, Rgb::WHITE, 1.0 - crate::hits::DIM);
                }
                let shape = crate::hits::HitShape::Rect {
                    x: 0.0,
                    y: ry,
                    w: table_w,
                    h: row_h,
                };
                target.hit(ctx.ix.hit(shape, ctx.ix.select(pt), tip.clone()));
            }
            if let Some(Some((path, btn))) = lay.row_menu.get(k) {
                // the row header: its menu (expand, collapse, drill); then its +/- button on top
                let (x0, w0) = (
                    col_x.first().copied().unwrap_or(0.0),
                    col_w.first().copied().unwrap_or(0.0),
                );
                let shape = crate::hits::HitShape::Rect {
                    x: x0,
                    y: ry,
                    w: w0,
                    h: row_h,
                };
                let info = serde_json::json!({"path": path, "open": btn});
                target.hit(ctx.ix.hit(shape, ctx.ix.menu("matrix_row", info), Vec::new()));
                if let Some(open) = btn {
                    let ind = r.first().map_or(0, |c| c.ind) as f32;
                    let bx = x0 + pad + ind * STEP_INDENT - px * 0.2;
                    let shape = crate::hits::HitShape::Rect {
                        x: bx,
                        y: ry,
                        w: px * 1.3,
                        h: row_h,
                    };
                    let (page, visual) = ctx.ix.page_visual();
                    let action = serde_json::json!({
                        "action": if *open { "matrix_collapse" } else { "matrix_expand" },
                        "page": page, "visual": visual, "path": path,
                    });
                    target.hit(ctx.ix.hit(shape, ctx.ix.plain(action), Vec::new()));
                }
            }
            ry += row_h;
        }
        match &clip {
            Some(c) => {
                g.draw(c, 0.0, body_top, 1.0);
                y = body_top + room;
            }
            None => y = ry,
        }
        if let Some(total) = total
            && (fit_n > 0 || scrolls)
        {
            if classic {
                g.rect(0.0, y, table_w, 1.0, rule, 1.0);
            }
            tag_row(g, true);
            for (i, h) in heads.iter().enumerate() {
                tag_cell(g, Part::Cell);
                if let Some(c) = total.get(i) {
                    draw_cell(g, c, i, y, row_h, &f_norm, 700, fg, h.num);
                }
                untag(g);
            }
            tag_row(g, false);
            y += row_h;
        }
        untag(g);
        if row_header && let Some(w0) = col_w.first() {
            g.rect(
                w0 - 1.0,
                body_top,
                1.0,
                y - body_top,
                if classic { rule } else { ctx.grid },
                1.0,
            );
        }
    }
    if let Some(c) = &cut {
        cv.draw(c, 0.0, 0.0, 1.0);
    }
    if scrolls {
        let all = (n_rows as f32 * row_h).max(1.0);
        let len = thumb_len(grid_h, room, all);
        cv.round_rect(
            full_w - THUMB_W,
            1.5,
            THUMB_W,
            len,
            THUMB_W / 2.0,
            Some((THUMB_COLOR, 1.0)),
            None,
        );
    }
    if hscroll {
        let len = thumb_len(b.w, view_w, table_w);
        cv.round_rect(
            1.5,
            b.h - THUMB_W,
            len,
            THUMB_W,
            THUMB_W / 2.0,
            Some((THUMB_COLOR, 1.0)),
            None,
        );
    }
}

/// A matrix row header's +/- button (Desktop's "+/- icons"): a square of side `s` whose left
/// edge is at `x`, centred on `cy`; a minus when expanded, a plus when collapsed.
fn expand_button(cv: &mut Canvas, x: f32, cy: f32, s: f32, open: bool, color: Rgb) {
    let y = cy - s / 2.0;
    let line = (s / 9.0).max(1.0);
    cv.rect(x, y, s, line, color, 1.0);
    cv.rect(x, y + s - line, s, line, color, 1.0);
    cv.rect(x, y, line, s, color, 1.0);
    cv.rect(x + s - line, y, line, s, color, 1.0);
    let arm = s * 0.55;
    cv.rect(x + (s - arm) / 2.0, cy - line / 2.0, arm, line, color, 1.0);
    if !open {
        cv.rect(x + s / 2.0 - line / 2.0, cy - arm / 2.0, line, arm, color, 1.0);
    }
}

fn additive(c: &Col, ds: &Dataset) -> bool {
    !ds.truncated
        && if ds.fake || c.kind == "measure" {
            !fmt_parts(&c.fmt).pct
        } else {
            c.kind == "aggregation"
                && matches!(c.func.to_lowercase().as_str(), "sum" | "count" | "countnonnull")
        }
}

fn sum_col(rows: &[Vec<Value>], i: usize) -> f64 {
    rows.iter().filter_map(|r| r.get(i).and_then(fin)).sum()
}

pub fn table_body(cv: &mut Canvas, ds: &Dataset, b: Bx, f: &Fmt, ctx: &Ctx<'_>) {
    let cols: Vec<&Col> = ds.cols.iter().take(24).collect();
    let rows = &ds.rows;
    let is_n: Vec<bool> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| is_num_col(c, rows, i))
        .collect();
    let heads: Vec<Head> = cols
        .iter()
        .zip(&is_n)
        .map(|(c, n)| Head {
            t: c.name.clone(),
            num: *n,
            bold: false,
        })
        .collect();
    let body: Vec<Vec<Cell>> = rows
        .iter()
        .take(200)
        .enumerate()
        .map(|(k, r)| {
            cols.iter()
                .enumerate()
                .map(|(i, c)| {
                    if let Some(st) = &c.spark {
                        return spark_cell(r.get(i).unwrap_or(&Value::Null), st);
                    }
                    let t = fmt_cell(r.get(i).unwrap_or(&Value::Null), Some(&c.cf()), &ctx.loc, false);
                    Cell::styled(t, ds.styles.get(&(k, i)), f.col_styles.get(&i))
                })
                .collect()
        })
        .collect();
    let sums: Vec<bool> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| is_n.get(i).copied().unwrap_or(false) && additive(c, ds))
        .collect();
    // the engine's total row (evaluated like Desktop's, not added up) when it has one
    let engine_total = ds
        .totals
        .as_ref()
        .filter(|t| !ds.fake && t.iter().any(|x| !x.is_null()));
    let total: Option<Vec<Cell>> = if let Some(t) = engine_total {
        (rows.len() > 1).then(|| {
            cols.iter()
                .enumerate()
                .map(|(i, c)| match (t.get(i).filter(|x| !x.is_null()), &c.spark) {
                    // the total row's sparkline (Desktop draws one there)
                    (Some(x), Some(st)) => spark_cell(x, st),
                    (Some(x), None) => Cell::text(fmt_cell(x, Some(&c.cf()), &ctx.loc, false)),
                    (None, _) if i == 0 => Cell::text(ctx.t("total", "Total")),
                    (None, _) => Cell::text(String::new()),
                })
                .collect()
        })
    } else {
        (rows.len() > 1 && sums.iter().any(|x| *x)).then(|| {
            cols.iter()
                .enumerate()
                .map(|(i, c)| {
                    if sums.get(i).copied().unwrap_or(false) {
                        Cell::text(fmt_val(sum_col(rows, i), Some(&c.cf()), &ctx.loc, false))
                    } else {
                        Cell::text(if i == 0 {
                            ctx.t("total", "Total")
                        } else {
                            String::new()
                        })
                    }
                })
                .collect()
        })
    };
    let n = cols.len();
    let mut lay = Layout::of(f, &|c| (c < n).then_some(c));
    lay.rows = ds.total_rows;
    // a row is a point: its grouping columns (the selected rows stay, the others fade)
    if !ds.fake && ctx.ix.on() {
        lay.row_hits = rows
            .iter()
            .take(201)
            .map(|r| {
                let pt: crate::hits::Point = cols
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.kind == "column")
                    .map(|(i, _)| (i, r.get(i).cloned().unwrap_or(Value::Null)))
                    .collect();
                let tip = cols
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        (
                            c.name.clone(),
                            crate::hits::cell(r.get(i).unwrap_or(&Value::Null), c, ctx),
                        )
                    })
                    .collect();
                (pt, tip)
            })
            .collect();
    }
    grid_table(cv, ctx, b, &heads, &body, total.as_deref(), &[], false, &lay);
}

/// A matrix's `subTotals` options (`format.table`): row and column subtotals (on by default;
/// off also hides the grand total row / column, as Desktop 2.158) and row subtotals below
/// their members instead of on the member's own row.
#[derive(Debug, Clone)]
struct TotalsOpt {
    rows: bool,
    cols: bool,
    bottom: bool,
    /// Custom subtotal labels (`rowSubtotalsLabel`, `columnSubtotalsLabel`): every row /
    /// column subtotal and the grand total (Desktop 2.158).
    row_label: Option<String>,
    col_label: Option<String>,
}

impl TotalsOpt {
    fn of(f: &Fmt) -> TotalsOpt {
        let t = f.raw.get("table").unwrap_or(&Value::Null);
        let label = |t: &Value, k: &str| {
            t.get(k)
                .and_then(Value::as_str)
                .map(|s| s.chars().take(200).collect::<String>())
        };
        TotalsOpt {
            rows: t.get("row_subtotals").and_then(Value::as_bool) != Some(false),
            cols: t.get("column_subtotals").and_then(Value::as_bool) != Some(false),
            bottom: t
                .get("row_subtotals_position")
                .and_then(Value::as_str)
                .is_some_and(|p| p.eq_ignore_ascii_case("bottom")),
            row_label: label(t, "row_subtotals_label"),
            col_label: label(t, "column_subtotals_label"),
        }
    }
}

/// Most row members of one level (and leaves) a matrix lays out.
const MAX_MATRIX_ROWS: usize = 500;

/// The key of a member path: its values' keys, joined.
fn path_key<'a>(vals: impl Iterator<Item = &'a Value>) -> String {
    vals.map(key_of).collect::<Vec<_>>().join("\u{1}")
}

/// Every (row depth, column depth) level of a matrix, keyed by member path: the leaf rows
/// (with their index, for conditional formatting), the engine's subtotal levels, and sums of
/// the additive values for the levels the engine did not send.
struct Cube {
    cells: HashMap<(usize, usize, String), (Vec<Value>, Option<usize>)>,
    /// Row member paths of each row depth (levels sent by the engine: in its order).
    order: HashMap<usize, Vec<Vec<Value>>>,
}

impl Cube {
    fn new(ds: &Dataset, r_i: &[usize], c_i: &[usize], v_i: &[usize], need: (usize, usize)) -> Cube {
        let (nr, nc) = (r_i.len(), c_i.len());
        let path = |row: &[Value], r: usize, c: usize| {
            path_key(
                r_i.iter()
                    .take(r)
                    .chain(c_i.iter().take(c))
                    .map(|i| row.get(*i).unwrap_or(&Value::Null)),
            )
        };
        let mut cells = HashMap::new();
        for (k, row) in ds.rows.iter().enumerate() {
            cells
                .entry((nr, nc, path(row, nr, nc)))
                .or_insert_with(|| (row.clone(), Some(k)));
        }
        let mut order: HashMap<usize, Vec<Vec<Value>>> = HashMap::new();
        let mut sent = std::collections::HashSet::new();
        for l in &ds.subtotals {
            if l.rows > nr || l.cols > nc || (l.rows, l.cols) == (nr, nc) {
                continue;
            }
            sent.insert((l.rows, l.cols));
            for row in &l.data {
                cells
                    .entry((l.rows, l.cols, path(row, l.rows, l.cols)))
                    .or_insert_with(|| (row.clone(), None));
            }
            if l.cols == 0 && l.rows > 0 {
                order.insert(
                    l.rows,
                    l.data
                        .iter()
                        .take(MAX_MATRIX_ROWS)
                        .map(|row| prefix(row, r_i, l.rows))
                        .collect(),
                );
            }
        }
        // the levels the engine did not send: additive values added up
        for r in 0..=need.0.min(nr) {
            for c in 0..=need.1.min(nc) {
                if (r, c) == (nr, nc) || sent.contains(&(r, c)) {
                    continue;
                }
                let mut sums: HashMap<String, Vec<Value>> = HashMap::new();
                for row in &ds.rows {
                    let e = sums.entry(path(row, r, c)).or_insert_with(|| {
                        let mut s = vec![Value::Null; row.len()];
                        for i in r_i.iter().take(r).chain(c_i.iter().take(c)) {
                            if let (Some(slot), Some(x)) = (s.get_mut(*i), row.get(*i)) {
                                *slot = x.clone();
                            }
                        }
                        s
                    });
                    for vi in v_i {
                        let add = ds.cols.get(*vi).is_some_and(|c| additive(c, ds));
                        if let (true, Some(x)) = (add, row.get(*vi).and_then(fin))
                            && let Some(slot) = e.get_mut(*vi)
                        {
                            *slot = json_num(slot.as_f64().unwrap_or(0.0) + x);
                        }
                    }
                }
                for (k, s) in sums {
                    cells.insert((r, c, k), (s, None));
                }
            }
        }
        Cube { cells, order }
    }

    fn get(&self, r: usize, c: usize, key: &str) -> Option<&(Vec<Value>, Option<usize>)> {
        self.cells.get(&(r, c, key.to_string()))
    }
}

fn json_num(x: f64) -> Value {
    serde_json::Number::from_f64(x).map_or(Value::Null, Value::Number)
}

fn prefix(row: &[Value], r_i: &[usize], d: usize) -> Vec<Value> {
    r_i.iter()
        .take(d)
        .map(|i| row.get(*i).cloned().unwrap_or(Value::Null))
        .collect()
}

pub fn matrix_body(cv: &mut Canvas, ds: &Dataset, b: Bx, f: &Fmt, ctx: &Ctx<'_>) {
    let (cols, rows) = (&ds.cols, &ds.rows);
    let is_n: Vec<bool> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| is_num_col(c, rows, i))
        .collect();
    let r_i: Vec<usize> = (0..cols.len())
        .filter(|i| cols.get(*i).is_some_and(|c| c.rc == Rc::Cat))
        .take(MAX_MATRIX_LEVELS)
        .collect();
    let c_i: Vec<usize> = (0..cols.len())
        .filter(|i| cols.get(*i).is_some_and(|c| c.rc == Rc::Col))
        .take(MAX_MATRIX_LEVELS)
        .collect();
    // the value columns: numbers, and sparklines (drawn in every cell, the totals' too)
    let v_i: Vec<usize> = (0..cols.len())
        .filter(|i| {
            (is_n.get(*i).copied().unwrap_or(false) || cols.get(*i).is_some_and(|c| c.spark.is_some()))
                && cols
                    .get(*i)
                    .is_some_and(|c| !matches!(c.rc, Rc::Cat | Rc::Col | Rc::Tip))
        })
        .take(4)
        .collect();
    if r_i.is_empty() && c_i.is_empty() {
        return table_body(cv, ds, b, f, ctx);
    }
    let opt = TotalsOpt::of(f);
    let (nr, nc) = (r_i.len(), c_i.len());
    // which members show their children: the engine's expansion (Desktop's `expansionStates`),
    // else a uniform depth (`row_levels`; one level of column groups)
    let uniform_r = ds.row_levels.unwrap_or(nr).clamp(usize::from(nr > 0), nr);
    let row_open = |p: &[Value]| match &ds.row_exp {
        Some(e) => e.expanded(p, nr),
        None => !p.is_empty() && p.len() < uniform_r,
    };
    let col_open = |p: &[Value]| ds.col_exp.as_ref().is_some_and(|e| e.expanded(p, nc));
    // per-level subtotal switches and labels: the entry of the field a subtotal totals over
    // (level 0: the grand total), else the visual's label, else "Total"
    let total_word = ctx.t("total", "Total");
    let lvl_show = |l: &[LevelTotal], d: usize| l.get(d).is_none_or(|x| x.show);
    let lvl_label = |l: &[LevelTotal], d: usize, all: &Option<String>| {
        l.get(d)
            .and_then(|x| x.label.clone())
            .or_else(|| all.clone())
            .unwrap_or_else(|| total_word.clone())
    };
    let row_sub = |d: usize| opt.rows && lvl_show(&ds.row_level_totals, d);
    let col_sub = |d: usize| opt.cols && lvl_show(&ds.col_level_totals, d);
    let row_label = |d: usize| lvl_label(&ds.row_level_totals, d, &opt.row_label);
    let col_label = |d: usize| lvl_label(&ds.col_level_totals, d, &opt.col_label);
    let cube = Cube::new(ds, &r_i, &c_i, &v_i, (nr, nc));
    let engine = !ds.subtotals.is_empty() && !ds.fake;
    let n_v = v_i.len().max(1);
    let col_cf = |vi: Option<&usize>| vi.and_then(|i| cols.get(*i)).map(|c| c.cf());
    // column member paths per depth, ascending
    let col_paths = |d: usize| -> Vec<Vec<Value>> {
        let mut seen = std::collections::HashSet::new();
        let mut out: Vec<Vec<Value>> = Vec::new();
        for r in rows {
            let p = prefix(r, &c_i, d);
            if seen.insert(path_key(p.iter())) {
                out.push(p);
                if out.len() >= MAX_MATRIX_COLUMNS * 4 {
                    break;
                }
            }
        }
        out.sort_by(|a, b| {
            a.iter()
                .zip(b.iter())
                .map(|(x, y)| cmp_raw(x, y))
                .find(|o| o.is_ne())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    };
    let col_levels: Vec<Vec<Vec<Value>>> = (1..=nc).map(col_paths).collect();
    let ck_n = col_levels.first().map_or(0, Vec::len);
    let any_add = v_i.iter().any(|i| cols.get(*i).is_some_and(|c| additive(c, ds)));
    // the column slots: members (collapsed or leaves), then an expanded member's children and
    // its subtotal, then the grand total (Desktop 2.158)
    let mut slots_c: Vec<ColSlot> = Vec::new();
    if nc == 0 {
        slots_c.push(ColSlot::Leaf(Vec::new()));
    } else {
        let mut stack: Vec<(Vec<Value>, bool)> = col_levels
            .first()
            .into_iter()
            .flatten()
            .rev()
            .map(|p| (p.clone(), false))
            .collect();
        while let Some((p, closing)) = stack.pop() {
            if slots_c.len() >= MAX_MATRIX_COLUMNS {
                break;
            }
            if closing {
                slots_c.push(ColSlot::Sub(p));
                continue;
            }
            if !col_open(&p) {
                slots_c.push(ColSlot::Leaf(p));
                continue;
            }
            if col_sub(p.len()) && !v_i.is_empty() {
                stack.push((p.clone(), true));
            }
            let kids: Vec<&Vec<Value>> = col_levels
                .get(p.len())
                .into_iter()
                .flatten()
                .filter(|c| c.starts_with(&p))
                .collect();
            for k in kids.into_iter().rev() {
                stack.push((k.clone(), false));
            }
        }
        if col_sub(0) && !v_i.is_empty() && (engine || (ck_n > 1 && any_add)) {
            slots_c.push(ColSlot::Sub(Vec::new()));
        }
    }
    // the value columns: (column slot, value field)
    let slots: Vec<(usize, usize)> = (0..slots_c.len())
        .flat_map(|s| (0..n_v).map(move |j| (s, j)))
        .collect();
    let col_text = |d: usize, x: &Value| fmt_cell(x, col_cf(c_i.get(d)).as_ref(), &ctx.loc, false);
    let value_name = |j: usize| {
        v_i.get(j)
            .and_then(|i| cols.get(*i))
            .map(|c| c.name.clone())
            .unwrap_or_default()
    };
    // header rows: one per displayed column level (the last one merged with the value names
    // when there is one value), a member's label over its columns, "Total" over a subtotal
    let depth_l = slots_c
        .iter()
        .map(|s| match s {
            ColSlot::Leaf(p) => p.len(),
            ColSlot::Sub(p) => p.len() + 1,
        })
        .max()
        .unwrap_or(0)
        .max(usize::from(nc > 0));
    // (group key, text, bold) of slot `s` at header row `k`
    let head_at = |s: usize, k: usize| -> (String, String, bool) {
        match slots_c.get(s) {
            Some(ColSlot::Leaf(p) | ColSlot::Sub(p)) if k < p.len() => (
                format!("m{}", path_key(p.iter().take(k + 1))),
                p.get(k).map(|x| col_text(k, x)).unwrap_or_default(),
                false,
            ),
            Some(ColSlot::Sub(p)) if k == p.len() => (format!("t{s}"), col_label(p.len()), true),
            _ => (format!("b{s}"), String::new(), false),
        }
    };
    let row_head = r_i
        .first()
        .and_then(|i| cols.get(*i))
        .map(|c| c.name.clone())
        .unwrap_or_default();
    let col_name = |k: usize| {
        c_i.get(k)
            .and_then(|i| cols.get(*i))
            .map(|c| c.name.clone())
            .unwrap_or_default()
    };
    let top_n = if nc == 0 {
        0
    } else if n_v > 1 {
        depth_l
    } else {
        depth_l.saturating_sub(1)
    };
    let mut tops: Vec<Vec<(String, usize, bool)>> = Vec::new();
    for k in 0..top_n {
        let mut row = vec![(col_name(k), 1usize, false)];
        let mut last: Option<String> = None;
        for s in 0..slots_c.len() {
            let (key, t, bold) = head_at(s, k);
            match row.last_mut() {
                Some(cell) if last.as_ref() == Some(&key) => cell.1 += n_v,
                _ => row.push((t, n_v, bold)),
            }
            last = Some(key);
        }
        tops.push(row);
    }
    let mut heads = vec![Head {
        t: row_head,
        num: false,
        bold: false,
    }];
    for (s, j) in &slots {
        let total = matches!(slots_c.get(*s), Some(ColSlot::Sub(_)));
        let (t, bold) = if nc == 0 || n_v > 1 {
            (value_name(*j), total)
        } else {
            let (_, t, bold) = head_at(*s, depth_l.saturating_sub(1));
            (t, bold)
        };
        heads.push(Head { t, num: true, bold });
    }
    // the cells of a row member path
    let cells_of = |p: &[Value], bold: bool| -> Vec<Cell> {
        let r = p.len();
        slots
            .iter()
            .map(|(s, j)| {
                let vi = v_i.get(*j);
                let (cp, leaf) = match slots_c.get(*s) {
                    Some(ColSlot::Leaf(c)) => (c.as_slice(), true),
                    Some(ColSlot::Sub(c)) => (c.as_slice(), false),
                    None => (&[][..], false),
                };
                let key = path_key(p.iter().chain(cp.iter()));
                let hit = cube.get(r, cp.len(), &key);
                let x = hit.and_then(|(row, _)| vi.and_then(|i| row.get(*i)));
                if let Some(st) = vi.and_then(|i| ds.cols.get(*i)).and_then(|c| c.spark.as_ref()) {
                    return spark_cell(x.unwrap_or(&Value::Null), st);
                }
                let t = x
                    .filter(|x| !x.is_null())
                    .map(|x| fmt_cell(x, col_cf(vi).as_ref(), &ctx.loc, false))
                    .unwrap_or_default();
                let st = hit
                    .and_then(|(_, k)| *k)
                    .and_then(|k| vi.and_then(|i| ds.styles.get(&(k, *i))));
                let cs = vi.and_then(|i| f.col_styles.get(i));
                let mut cell = Cell::styled(t, if leaf { st } else { None }, cs);
                cell.bold = bold || !leaf;
                cell
            })
            .collect()
    };
    let blank_cells = || -> Vec<Cell> { slots.iter().map(|_| Cell::default()).collect() };
    let label = |d: usize, x: &Value| -> String {
        let t = fmt_cell(x, col_cf(r_i.get(d)).as_ref(), &ctx.loc, false);
        if t.is_empty() { ctx.blank() } else { t }
    };
    // row member paths per depth: the engine's order for subtotal levels, else the data's
    let paths_at = |d: usize| -> Vec<Vec<Value>> {
        let mut seen = std::collections::HashSet::new();
        let src: Vec<Vec<Value>> = match cube.order.get(&d) {
            Some(o) if d < nr => o.clone(),
            _ => rows.iter().map(|r| prefix(r, &r_i, d)).collect(),
        };
        src.into_iter()
            .filter(|p| seen.insert(path_key(p.iter())))
            .take(MAX_MATRIX_ROWS)
            .collect()
    };
    let levels: Vec<Vec<Vec<Value>>> = (1..=nr).map(paths_at).collect();
    // depth-first: a member's row (with its subtotal on top, Desktop's default), its children,
    // then its subtotal row when subtotals sit at the bottom
    let mut items: Vec<(Vec<Value>, RowKind)> = Vec::new();
    let mut stack: Vec<(Vec<Value>, bool)> = levels
        .first()
        .into_iter()
        .flatten()
        .rev()
        .map(|p| (p.clone(), false))
        .collect();
    while let Some((p, closing)) = stack.pop() {
        if items.len() > 300 {
            break;
        }
        if closing {
            items.push((p, RowKind::Closing));
            continue;
        }
        if !row_open(&p) {
            items.push((p, RowKind::Shut));
            continue;
        }
        if opt.bottom && row_sub(p.len()) {
            stack.push((p.clone(), true));
        }
        let kids: Vec<&Vec<Value>> = levels
            .get(p.len())
            .into_iter()
            .flatten()
            .filter(|c| c.starts_with(&p))
            .collect();
        for k in kids.into_iter().rev() {
            stack.push((k.clone(), false));
        }
        items.push((p, RowKind::Open));
    }
    // a level above the deepest one shown is bold, expanded or not (Desktop 2.158)
    let deepest = items.iter().map(|(p, _)| p.len()).max().unwrap_or(0);
    // the row headers' +/- buttons ("+/- icons") and menus: a member with children, or one of
    // the deepest level shown when the hierarchy has more levels (the drill state says so)
    let buttons = f
        .raw
        .get("table")
        .and_then(|t| t.get("expand_buttons"))
        .and_then(Value::as_bool)
        == Some(true);
    let more_levels = f.drill.get("matrix").and_then(Value::as_bool) == Some(true)
        && f.drill
            .get("role")
            .and_then(Value::as_str)
            .is_some_and(|r| r.eq_ignore_ascii_case("Rows"))
        && f.drill.get("can_down").and_then(Value::as_bool) == Some(true);
    let mut menu: Vec<Option<(Vec<Value>, Option<bool>)>> = Vec::new();
    let mut body: Vec<Vec<Cell>> = Vec::new();
    if r_i.is_empty() {
        let mut row = vec![Cell::text(String::new())];
        row.extend(cells_of(&[], false));
        body.push(row);
        menu.push(None);
    }
    for (p, kind) in &items {
        let d = p.len();
        match kind {
            RowKind::Closing => {
                // "Total" at the children's indent (Desktop 2.158)
                let mut row = vec![Cell {
                    t: row_label(d),
                    bold: true,
                    ind: d,
                    ..Cell::default()
                }];
                row.extend(cells_of(p, true));
                body.push(row);
                menu.push(None);
            }
            RowKind::Open | RowKind::Shut => {
                let bold = d < deepest;
                let expandable = d < nr || (d == nr && more_levels);
                let btn = (buttons && expandable).then_some(matches!(kind, RowKind::Open));
                menu.push(Some((p.clone(), btn)));
                let mut row = vec![Cell {
                    t: label(d.saturating_sub(1), p.last().unwrap_or(&Value::Null)),
                    bold,
                    ind: d.saturating_sub(1),
                    btn,
                    ..Cell::default()
                }];
                if matches!(kind, RowKind::Shut) || (!opt.bottom && row_sub(d)) {
                    row.extend(cells_of(p, bold));
                } else {
                    row.extend(blank_cells());
                }
                body.push(row);
            }
        }
    }
    let n_top = levels.first().map_or(0, Vec::len);
    let total: Option<Vec<Cell>> = (!r_i.is_empty() && row_sub(0) && (engine || (n_top > 1 && any_add)))
        .then(|| {
            let mut t = vec![Cell::text(row_label(0))];
            t.extend(cells_of(&[], true));
            t
        });
    // the row headers are grid column 0; value columns have no width of their own here
    let first_row = r_i.first().copied();
    let mut lay = Layout::of(f, &|c| (Some(c) == first_row).then_some(0));
    lay.widths.clear();
    lay.row_menu = menu;
    grid_table(cv, ctx, b, &heads, &body, total.as_deref(), &tops, true, &lay);
}

/// A matrix column: a column member shown with its values (a leaf, or collapsed), or the
/// subtotal of an expanded member (empty path: the grand total).
enum ColSlot {
    Leaf(Vec<Value>),
    Sub(Vec<Value>),
}

/// A matrix row: an expanded member, a collapsed one (or a leaf), or the subtotal row of an
/// expanded member at the bottom.
enum RowKind {
    Open,
    Shut,
    Closing,
}

#[cfg(test)]
#[path = "tables_tests.rs"]
mod tests;
