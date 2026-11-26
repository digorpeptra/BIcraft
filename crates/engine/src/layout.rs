//! Visual placement (M2.4): grids, shelf packing and the `visual.place` / `page.arrange` commands,
//! plus the overlap check of `report.validate`.
//!
//! FROZEN API (other modules call it; keep the signatures):
//! - [`Rect`]
//! - [`place_all`]: turns per-visual placement requests into rectangles on one page.
//!
//! # Page layout (`layout`)
//!
//! - absent, `null` or `"shelf"`: shelf packing with a 16 px margin and a 16 px gutter.
//! - `{"shelf": {"margin"?: px, "gutter"?: px}}`.
//! - `"grid"`: a 12 x 8 grid with a 16 px margin and gutter.
//! - `{"grid": {"columns"?: 1..100, "rows"?: 1..100, "margin"?: px, "gutter"?: px}}`.
//!
//! Margins and gutters are 0..100000 px, rounded to whole pixels. The *content area* is the page
//! minus the margin on every side.
//!
//! # Item placement (`place`)
//!
//! An object with these optional keys (unknown keys are errors), or `null`:
//!
//! - `null` / `{}` / `{"visual_type": "card"}`: automatic. The visual type picks a default size:
//!   cards, KPIs and slicers 25% x 20% (3 x 2 cells of a 12 x 8 grid), text boxes 100% x 10%
//!   (12 x 1), buttons and navigators 1/6 x 10% (2 x 1), images and shapes 1/6 x 20% (2 x 2),
//!   anything else 50% x 40% (6 x 3). Spans are scaled to the grid's size.
//! - shelf sizes: `{"width": px | "NN%", "height": px | "NN%"}` (either may be omitted and then
//!   comes from the type). A percentage is a share of the content area *gutters included*, so
//!   `50%` + `50%` fill one shelf and `25%` x 4 too; percentages round down.
//! - grid cells: `{"col", "row", "col_span"?: 1, "row_span"?: 1}`, 0-based. `{"col_span"?,
//!   "row_span"?}` without `col`/`row` flows into the first free area (row by row, after the
//!   explicit cells), like CSS grid auto-placement.
//! - fixed rectangle: `{"x", "y", "width", "height"}` in px, in both layouts; it must lie on the
//!   page and is not packed around.
//!
//! Shelf packing places items left to right and starts a new shelf (below the tallest item of
//! the previous one, plus the gutter) when the next item does not fit the width. An item wider than
//! the content area, or one that would cross the bottom margin, is an error that says how many
//! pixels are missing. In a grid, cells outside the grid and overlapping cells are errors.
//!
//! Results are deterministic and whole pixels: every edge is rounded with `f64::round`, and cell
//! edges come from the same formula, so neighbouring cells never overlap.

use std::collections::{HashMap, HashSet};

use bicraft_report::{Page, Position};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, str_param};
use crate::report::{page_at_mut, page_index, parts, position_json, visual_index};

/// Largest page or visual dimension and coordinate accepted, in pixels (as in `report.rs`).
const MAX_SIZE: f64 = 100_000.0;
/// Most items one placement handles.
const MAX_ITEMS: usize = 10_000;
/// Most grid columns or rows.
const MAX_TRACKS: u32 = 100;
const DEFAULT_MARGIN: f64 = 16.0;
const DEFAULT_GUTTER: f64 = 16.0;
const DEFAULT_COLUMNS: u32 = 12;
const DEFAULT_ROWS: u32 = 8;
/// Overlaps and overflows up to this many pixels are rounding noise, not findings.
pub(crate) const TOLERANCE: f64 = 1.0;
/// Most overlap warnings listed per page; the rest are summarised in one more warning.
const MAX_OVERLAPS: usize = 1_000;
/// Most visual pairs compared per page by the overlap check.
const MAX_PAIR_CHECKS: usize = 20_000_000;
/// Deepest group nesting followed.
const MAX_GROUP_DEPTH: usize = 64;
/// Visual types that usually sit behind other visuals as decoration.
const DECORATIVE: &[&str] = &["shape", "basicShape", "image"];

/// A rectangle on a page, in page pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Places `items` on a `page_width` x `page_height` page.
///
/// - `layout` is the page-level layout request (see the module doc); `None` means shelf packing.
/// - each item is that visual's own placement request (see the module doc); `None` means "place
///   it automatically".
///
/// Returns one rectangle per item, in order, or an error naming the item (`item <index>`) that
/// cannot be placed.
pub(crate) fn place_all(
    page_width: f64,
    page_height: f64,
    layout: Option<&serde_json::Value>,
    items: &[Option<&serde_json::Value>],
) -> Result<Vec<Rect>, String> {
    let layout = parse_layout(layout)?;
    if items.len() > MAX_ITEMS {
        return Err(format!("at most {MAX_ITEMS} visuals can be placed at once"));
    }
    let requests = items
        .iter()
        .enumerate()
        .map(|(i, v)| parse_place(*v).map_err(|e| format!("item {i}: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    place(page_width, page_height, &layout, &requests, &|i| {
        format!("item {i}")
    })
}

// ---------------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum Layout {
    Shelf {
        margin: f64,
        gutter: f64,
    },
    Grid {
        columns: u32,
        rows: u32,
        margin: f64,
        gutter: f64,
    },
}

impl Layout {
    fn to_json(self) -> Value {
        match self {
            Self::Shelf { margin, gutter } => json!({"shelf": {"margin": margin, "gutter": gutter}}),
            Self::Grid {
                columns,
                rows,
                margin,
                gutter,
            } => json!({"grid": {"columns": columns, "rows": rows, "margin": margin, "gutter": gutter}}),
        }
    }
}

const LAYOUT_SHAPE: &str = "`layout` must be \"shelf\", \"grid\", {\"shelf\": {margin?, gutter?}} or \
                            {\"grid\": {columns?, rows?, margin?, gutter?}}";

fn parse_layout(v: Option<&Value>) -> Result<Layout, String> {
    let (kind, options) = match v {
        None | Some(Value::Null) => ("shelf", None),
        Some(Value::String(s)) => (s.as_str(), None),
        Some(Value::Object(o)) => {
            let mut entries = o.iter();
            match (entries.next(), entries.next()) {
                (Some((k, opts)), None) => (k.as_str(), Some(opts)),
                _ => return Err(LAYOUT_SHAPE.into()),
            }
        }
        Some(_) => return Err(LAYOUT_SHAPE.into()),
    };
    let empty = Value::Object(Map::new());
    let options = match options {
        None | Some(Value::Null) => &empty,
        Some(o @ Value::Object(_)) => o,
        Some(_) => return Err(LAYOUT_SHAPE.into()),
    };
    match kind {
        "shelf" => {
            allow(options, &["margin", "gutter"]).map_err(|e| format!("shelf layout: {e}"))?;
            Ok(Layout::Shelf {
                margin: spacing(options, "margin", DEFAULT_MARGIN)?,
                gutter: spacing(options, "gutter", DEFAULT_GUTTER)?,
            })
        }
        "grid" => {
            allow(options, &["columns", "rows", "margin", "gutter"])
                .map_err(|e| format!("grid layout: {e}"))?;
            Ok(Layout::Grid {
                columns: tracks(options, "columns", DEFAULT_COLUMNS)?,
                rows: tracks(options, "rows", DEFAULT_ROWS)?,
                margin: spacing(options, "margin", DEFAULT_MARGIN)?,
                gutter: spacing(options, "gutter", DEFAULT_GUTTER)?,
            })
        }
        other => Err(format!("unknown layout `{other}`: {LAYOUT_SHAPE}")),
    }
}

/// A whole number (`3` or `3.0`), if `v` is one.
fn whole(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| {
        v.as_f64()
            .filter(|f| f.is_finite() && *f >= 0.0 && f.fract() == 0.0 && *f <= 1e15)
            .map(|f| f as u64)
    })
}

/// A margin or gutter: 0..MAX_SIZE px, rounded.
fn spacing(o: &Value, key: &str, default: f64) -> Result<f64, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => match v.as_f64() {
            Some(n) if n.is_finite() && (0.0..=MAX_SIZE).contains(&n) => Ok(n.round()),
            _ => Err(format!(
                "`{key}` must be a number of pixels between 0 and {MAX_SIZE}"
            )),
        },
    }
}

/// A grid column or row count: 1..=MAX_TRACKS.
fn tracks(o: &Value, key: &str, default: u32) -> Result<u32, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => whole(v)
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| (1..=MAX_TRACKS).contains(n))
            .ok_or_else(|| format!("`{key}` must be a whole number from 1 to {MAX_TRACKS}")),
    }
}

/// A shelf length: pixels or a percentage of the content area.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Len {
    Px(f64),
    Pct(f64),
}

/// One item's placement request, parsed.
#[derive(Debug, Clone, Default, PartialEq)]
struct Request {
    visual_type: Option<String>,
    x: Option<f64>,
    y: Option<f64>,
    width: Option<Len>,
    height: Option<Len>,
    col: Option<u32>,
    row: Option<u32>,
    col_span: Option<u32>,
    row_span: Option<u32>,
}

impl Request {
    fn is_fixed(&self) -> bool {
        self.x.is_some() || self.y.is_some()
    }
    fn has_cell(&self) -> bool {
        self.col.is_some() || self.row.is_some()
    }
    fn has_span(&self) -> bool {
        self.col_span.is_some() || self.row_span.is_some()
    }
    fn has_size(&self) -> bool {
        self.width.is_some() || self.height.is_some()
    }
}

const PLACE_KEYS: &[&str] = &[
    "visual_type",
    "x",
    "y",
    "width",
    "height",
    "col",
    "row",
    "col_span",
    "row_span",
];

fn parse_place(v: Option<&Value>) -> Result<Request, String> {
    let v = match v {
        None | Some(Value::Null) => return Ok(Request::default()),
        Some(v @ Value::Object(_)) => v,
        Some(_) => return Err("a placement must be an object or null".into()),
    };
    allow(v, PLACE_KEYS)?;
    let visual_type = match v.get("visual_type") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.len() <= 200 => Some(s.clone()),
        Some(_) => return Err("`visual_type` must be a visual type name".into()),
    };
    let coordinate = |key: &str| -> Result<Option<f64>, String> {
        match v.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(n) => match n.as_f64() {
                Some(n) if n.is_finite() && (-MAX_SIZE..=MAX_SIZE).contains(&n) => Ok(Some(n)),
                _ => Err(format!(
                    "`{key}` must be a number between -{MAX_SIZE} and {MAX_SIZE}"
                )),
            },
        }
    };
    let length = |key: &str| -> Result<Option<Len>, String> {
        let bad =
            || format!("`{key}` must be a number of pixels (0 to {MAX_SIZE}) or a percentage like \"50%\"");
        match v.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => {
                let pct: f64 = s
                    .trim()
                    .strip_suffix('%')
                    .and_then(|n| n.trim().parse().ok())
                    .ok_or_else(bad)?;
                if pct.is_finite() && pct > 0.0 && pct <= 100.0 {
                    Ok(Some(Len::Pct(pct)))
                } else {
                    Err(format!("`{key}` must be a percentage above 0% and at most 100%"))
                }
            }
            Some(n) => match n.as_f64() {
                Some(n) if n.is_finite() && n > 0.0 && n <= MAX_SIZE => Ok(Some(Len::Px(n))),
                _ => Err(bad()),
            },
        }
    };
    let index = |key: &str, min: u32| -> Result<Option<u32>, String> {
        match v.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(n) => whole(n)
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| (min..=MAX_TRACKS).contains(n))
                .map(Some)
                .ok_or_else(|| format!("`{key}` must be a whole number from {min} to {MAX_TRACKS}")),
        }
    };
    let r = Request {
        visual_type,
        x: coordinate("x")?,
        y: coordinate("y")?,
        width: length("width")?,
        height: length("height")?,
        col: index("col", 0)?,
        row: index("row", 0)?,
        col_span: index("col_span", 1)?,
        row_span: index("row_span", 1)?,
    };
    if r.is_fixed() {
        if r.x.is_none() || r.y.is_none() || r.has_cell() || r.has_span() {
            return Err("a fixed placement needs x, y, width and height, and no grid cell".into());
        }
        if !matches!((r.width, r.height), (Some(Len::Px(_)), Some(Len::Px(_)))) {
            return Err("a fixed placement needs width and height in pixels".into());
        }
    } else if r.col.is_some() != r.row.is_some() {
        return Err("a grid cell needs both `col` and `row` (0-based)".into());
    }
    if (r.has_cell() || r.has_span()) && r.has_size() {
        return Err(
            "give either a grid cell or span (col, row, col_span, row_span) or a shelf size \
                    (width, height), not both"
                .into(),
        );
    }
    Ok(r)
}

// ---------------------------------------------------------------------------------------------
// Placement
// ---------------------------------------------------------------------------------------------

/// Default size of an automatically placed visual: (width %, height %, columns of 12, rows of 8).
fn default_size(visual_type: Option<&str>) -> (f64, f64, u32, u32) {
    match visual_type.unwrap_or("") {
        "card" | "cardVisual" | "kpi" | "slicer" | "listSlicer" | "textSlicer" | "advancedSlicerVisual" => {
            (25.0, 20.0, 3, 2)
        }
        "textbox" => (100.0, 10.0, 12, 1),
        "actionButton" | "pageNavigator" | "bookmarkNavigator" => (100.0 / 6.0, 10.0, 2, 1),
        "image" | "shape" | "basicShape" => (100.0 / 6.0, 20.0, 2, 2),
        _ => (50.0, 40.0, 6, 3),
    }
}

/// `pct` of `total` with gutters counted in, rounded down, at least 1 px and at most `total`.
fn share(pct: f64, total: f64, gutter: f64) -> f64 {
    let max = total.floor().max(1.0);
    ((pct / 100.0) * (total + gutter) - gutter)
        .floor()
        .clamp(1.0, max)
}

/// Scales a span given for `base` tracks to `tracks` tracks.
fn scale_span(span: u32, base: u32, tracks: u32) -> u32 {
    let scaled = (f64::from(span) * f64::from(tracks) / f64::from(base)).round();
    let scaled = if scaled.is_finite() && scaled >= 1.0 {
        scaled.min(f64::from(tracks)) as u32
    } else {
        1
    };
    scaled.clamp(1, tracks.max(1))
}

/// The page's content area: (left, top, width, height).
fn content_area(page_width: f64, page_height: f64, margin: f64) -> Result<(f64, f64, f64, f64), String> {
    for (what, v) in [("width", page_width), ("height", page_height)] {
        if !(v.is_finite() && v > 0.0 && v <= MAX_SIZE) {
            return Err(format!(
                "the page {what} ({v}) must be greater than 0 and at most {MAX_SIZE}"
            ));
        }
    }
    let width = page_width - 2.0 * margin;
    let height = page_height - 2.0 * margin;
    if width < 1.0 || height < 1.0 {
        return Err(format!(
            "a {margin} px margin leaves no room on a {page_width} x {page_height} page"
        ));
    }
    Ok((margin, margin, width, height))
}

fn place(
    page_width: f64,
    page_height: f64,
    layout: &Layout,
    requests: &[Request],
    name: &dyn Fn(usize) -> String,
) -> Result<Vec<Rect>, String> {
    if requests.len() > MAX_ITEMS {
        return Err(format!("at most {MAX_ITEMS} visuals can be placed at once"));
    }
    let margin = match layout {
        Layout::Shelf { margin, .. } | Layout::Grid { margin, .. } => *margin,
    };
    let area = content_area(page_width, page_height, margin)?;
    let mut out: Vec<Option<Rect>> = vec![None; requests.len()];

    // Fixed rectangles: kept as given (rounded), on the page.
    for (i, r) in requests.iter().enumerate() {
        if let (Some(x), Some(y), Some(Len::Px(w)), Some(Len::Px(h))) = (r.x, r.y, r.width, r.height) {
            let rect = Rect {
                x: x.round(),
                y: y.round(),
                width: w.round().max(1.0),
                height: h.round().max(1.0),
            };
            if rect.x < 0.0
                || rect.y < 0.0
                || rect.x + rect.width > page_width
                || rect.y + rect.height > page_height
            {
                return Err(format!(
                    "{}: the fixed rectangle ({}, {}, {} x {}) does not lie on the {page_width} x \
                     {page_height} page",
                    name(i),
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height
                ));
            }
            if let Some(slot) = out.get_mut(i) {
                *slot = Some(rect);
            }
        }
    }

    match *layout {
        Layout::Shelf { gutter, .. } => shelf(page_height, area, gutter, requests, name, &mut out)?,
        Layout::Grid {
            columns,
            rows,
            gutter,
            ..
        } => grid(area, columns, rows, gutter, requests, name, &mut out)?,
    }
    out.into_iter()
        .enumerate()
        .map(|(i, r)| r.ok_or_else(|| format!("internal: {} was not placed", name(i))))
        .collect()
}

fn shelf(
    page_height: f64,
    (left, top, width, height): (f64, f64, f64, f64),
    gutter: f64,
    requests: &[Request],
    name: &dyn Fn(usize) -> String,
    out: &mut [Option<Rect>],
) -> Result<(), String> {
    let right = left + width;
    let bottom = top + height;
    let (mut x, mut y, mut shelf_height, mut shelf_used) = (left, top, 0.0_f64, false);
    for (i, r) in requests.iter().enumerate() {
        if r.is_fixed() {
            continue;
        }
        if r.has_cell() || r.has_span() {
            return Err(format!(
                "{}: grid cells and spans (col, row, col_span, row_span) need a grid layout, \
                 e.g. layout {{\"grid\": {{\"columns\": 12, \"rows\": 8}}}}",
                name(i)
            ));
        }
        let (dw, dh, _, _) = default_size(r.visual_type.as_deref());
        let w = match r.width {
            Some(Len::Px(px)) => px.round().max(1.0),
            Some(Len::Pct(p)) => share(p, width, gutter),
            None => share(dw, width, gutter),
        };
        let h = match r.height {
            Some(Len::Px(px)) => px.round().max(1.0),
            Some(Len::Pct(p)) => share(p, height, gutter),
            None => share(dh, height, gutter),
        };
        if w > width {
            return Err(format!(
                "{} is {w} px wide but the page has room for {} px between its margins ({} px \
                 missing): make it narrower or the page wider",
                name(i),
                width.floor(),
                (w - width).ceil()
            ));
        }
        if shelf_used && x + w > right {
            y += shelf_height + gutter;
            x = left;
            shelf_height = 0.0;
        }
        if y + h > bottom {
            return Err(format!(
                "{} does not fit on the page: it needs {h} px of height from y = {y} but the \
                 content ends at y = {bottom} ({} px missing); make the page taller (it is \
                 {page_height} px), use smaller sizes, or split the visuals over several pages",
                name(i),
                (y + h - bottom).ceil()
            ));
        }
        if let Some(slot) = out.get_mut(i) {
            *slot = Some(Rect {
                x,
                y,
                width: w,
                height: h,
            });
        }
        x += w + gutter;
        shelf_height = shelf_height.max(h);
        shelf_used = true;
    }
    Ok(())
}

/// The pixel edges of `span` tracks starting at `start`: (start px, length px).
fn track_edges(origin: f64, track: f64, gutter: f64, start: u32, span: u32) -> (f64, f64) {
    let a = (origin + f64::from(start) * (track + gutter)).round();
    let end = start.saturating_add(span);
    let b = (origin + f64::from(end) * (track + gutter) - gutter).round();
    (a, (b - a).max(1.0))
}

fn grid(
    (left, top, width, height): (f64, f64, f64, f64),
    columns: u32,
    rows: u32,
    gutter: f64,
    requests: &[Request],
    name: &dyn Fn(usize) -> String,
    out: &mut [Option<Rect>],
) -> Result<(), String> {
    if columns == 0 || rows == 0 {
        return Err("a grid needs at least one column and one row".into());
    }
    let cell_w = (width - gutter * f64::from(columns.saturating_sub(1))) / f64::from(columns);
    let cell_h = (height - gutter * f64::from(rows.saturating_sub(1))) / f64::from(rows);
    if cell_w < 1.0 || cell_h < 1.0 {
        return Err(format!(
            "a {columns} x {rows} grid with a {gutter} px gutter does not fit the {width} x {height} \
             px content area: use fewer columns or rows, or a smaller gutter or margin"
        ));
    }
    let cols = columns as usize;
    // At most MAX_TRACKS² cells.
    let mut taken: Vec<Option<usize>> = vec![None; cols.saturating_mul(rows as usize)];
    let cell = |c: u32, r: u32| (r as usize).saturating_mul(cols).saturating_add(c as usize);
    let rect = |c: u32, r: u32, cs: u32, rs: u32| {
        let (x, w) = track_edges(left, cell_w, gutter, c, cs);
        let (y, h) = track_edges(top, cell_h, gutter, r, rs);
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    };
    let check_kind = |i: usize, r: &Request| -> Result<(), String> {
        if r.has_size() {
            return Err(format!(
                "{}: shelf sizes (width, height) cannot be used in a grid layout; give col_span \
                 and row_span",
                name(i)
            ));
        }
        Ok(())
    };

    // Explicit cells first, so automatic items flow around them.
    for (i, r) in requests.iter().enumerate() {
        let (Some(c), Some(rw)) = (r.col, r.row) else {
            continue;
        };
        check_kind(i, r)?;
        let cs = r.col_span.unwrap_or(1);
        let rs = r.row_span.unwrap_or(1);
        let c_end = c.saturating_add(cs);
        let r_end = rw.saturating_add(rs);
        if c_end > columns {
            return Err(format!(
                "{}: columns {c} to {} are outside the grid's {columns} columns (0 to {})",
                name(i),
                c_end.saturating_sub(1),
                columns.saturating_sub(1)
            ));
        }
        if r_end > rows {
            return Err(format!(
                "{}: rows {rw} to {} are outside the grid's {rows} rows (0 to {}): add rows to the \
                 grid or split the visuals over several pages",
                name(i),
                r_end.saturating_sub(1),
                rows.saturating_sub(1)
            ));
        }
        for yy in rw..r_end {
            for xx in c..c_end {
                let slot = taken
                    .get_mut(cell(xx, yy))
                    .ok_or("internal: grid cell out of range")?;
                if let Some(j) = *slot {
                    return Err(format!(
                        "{}: cell (col {xx}, row {yy}) is already taken by {}",
                        name(i),
                        name(j)
                    ));
                }
                *slot = Some(i);
            }
        }
        if let Some(slot) = out.get_mut(i) {
            *slot = Some(rect(c, rw, cs, rs));
        }
    }

    // Then automatic items, row by row from a cursor that only moves forward.
    let total = taken.len();
    let mut cursor = 0usize;
    for (i, r) in requests.iter().enumerate() {
        if r.is_fixed() || r.has_cell() {
            continue;
        }
        check_kind(i, r)?;
        let (_, _, dc, dr) = default_size(r.visual_type.as_deref());
        let cs = r
            .col_span
            .unwrap_or_else(|| scale_span(dc, DEFAULT_COLUMNS, columns));
        let rs = r.row_span.unwrap_or_else(|| scale_span(dr, DEFAULT_ROWS, rows));
        if cs > columns || rs > rows {
            return Err(format!(
                "{}: a {cs} x {rs} span is larger than the {columns} x {rows} grid",
                name(i)
            ));
        }
        let fits = |p: usize, taken: &[Option<usize>]| -> bool {
            let c = (p % cols) as u32;
            let rw = (p / cols) as u32;
            let (c_end, r_end) = (c.saturating_add(cs), rw.saturating_add(rs));
            if c_end > columns || r_end > rows {
                return false;
            }
            (rw..r_end).all(|yy| (c..c_end).all(|xx| taken.get(cell(xx, yy)).is_some_and(Option::is_none)))
        };
        let Some(p) = (cursor..total).find(|&p| fits(p, &taken)) else {
            let free = taken.iter().filter(|t| t.is_none()).count();
            return Err(format!(
                "{} does not fit: no free area of {cs} x {rs} cells is left in the {columns} x \
                 {rows} grid ({free} cells free, {} needed); add rows to the grid, use smaller \
                 spans, or split the visuals over several pages",
                name(i),
                cs.saturating_mul(rs)
            ));
        };
        let c = (p % cols) as u32;
        let rw = (p / cols) as u32;
        for yy in rw..rw.saturating_add(rs) {
            for xx in c..c.saturating_add(cs) {
                if let Some(slot) = taken.get_mut(cell(xx, yy)) {
                    *slot = Some(i);
                }
            }
        }
        if let Some(slot) = out.get_mut(i) {
            *slot = Some(rect(c, rw, cs, rs));
        }
        cursor = p;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Overlaps (report.validate)
// ---------------------------------------------------------------------------------------------

/// Lowercased group chain of each visual (its group, that group's group, …), bounded.
fn ancestors(page: &Page) -> Vec<Vec<String>> {
    let parent: HashMap<String, String> = page
        .visuals
        .iter()
        .filter_map(|v| {
            v.parent_group
                .as_ref()
                .map(|g| (v.name.to_lowercase(), g.to_lowercase()))
        })
        .collect();
    page.visuals
        .iter()
        .map(|v| {
            let mut chain = Vec::new();
            let mut at = v.name.to_lowercase();
            while chain.len() < MAX_GROUP_DEPTH {
                match parent.get(&at) {
                    Some(g) if !chain.contains(g) => {
                        chain.push(g.clone());
                        at = g.clone();
                    }
                    _ => break,
                }
            }
            chain
        })
        .collect()
}

fn contains(outer: &Position, inner: &Position) -> bool {
    inner.x >= outer.x - TOLERANCE
        && inner.y >= outer.y - TOLERANCE
        && inner.x + inner.width <= outer.x + outer.width + TOLERANCE
        && inner.y + inner.height <= outer.y + outer.height + TOLERANCE
}

/// Reports every pair of visible visuals on `page` that overlap by more than [`TOLERANCE`] in
/// both directions, as warnings `(location, message)` passed to `warn`. Skipped: hidden visuals,
/// visuals without a size, a visual and a group it belongs to (at any depth), and a decorative
/// shape or image that wholly contains the other visual (a background).
pub(crate) fn overlap_warnings(page: &Page, mut warn: impl FnMut(Value, String)) {
    let chains = ancestors(page);
    let mut boxes: Vec<(usize, &Position)> = page
        .visuals
        .iter()
        .enumerate()
        .filter(|(_, v)| {
            let p = &v.position;
            !v.is_hidden
                && p.width > 0.0
                && p.height > 0.0
                && [p.x, p.y, p.width, p.height].iter().all(|n| n.is_finite())
        })
        .map(|(i, v)| (i, &v.position))
        .collect();
    boxes.sort_by(|a, b| a.1.x.total_cmp(&b.1.x).then(a.0.cmp(&b.0)));
    let (mut found, mut checks) = (0usize, 0usize);
    let mut stopped = false;
    'outer: for (k, (ia, a)) in boxes.iter().enumerate() {
        for (ib, b) in boxes.iter().skip(k.saturating_add(1)) {
            if b.x >= a.x + a.width - TOLERANCE {
                break;
            }
            checks = checks.saturating_add(1);
            if checks > MAX_PAIR_CHECKS {
                stopped = true;
                break 'outer;
            }
            let w = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
            let h = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
            if w <= TOLERANCE || h <= TOLERANCE {
                continue;
            }
            let (Some(va), Some(vb)) = (page.visuals.get(*ia), page.visuals.get(*ib)) else {
                continue;
            };
            let (na, nb) = (va.name.to_lowercase(), vb.name.to_lowercase());
            let related = chains.get(*ia).is_some_and(|c| c.contains(&nb))
                || chains.get(*ib).is_some_and(|c| c.contains(&na));
            let decorative = |v: &bicraft_report::Visual| {
                v.visual_type.as_deref().is_some_and(|t| DECORATIVE.contains(&t))
            };
            let background = (decorative(va) && contains(a, b)) || (decorative(vb) && contains(b, a));
            if related || background {
                continue;
            }
            found = found.saturating_add(1);
            if found > MAX_OVERLAPS {
                stopped = true;
                break 'outer;
            }
            // Report it on the visual drawn first in page order.
            let (first, second) = if ia <= ib { (va, vb) } else { (vb, va) };
            warn(
                json!({"page": page.display_name, "visual": first.name, "other": second.name}),
                format!(
                    "the visual overlaps visual `{}` ({} x {} px)",
                    second.name,
                    w.round(),
                    h.round()
                ),
            );
        }
    }
    if stopped {
        warn(
            json!({"page": page.display_name}),
            format!(
                "the overlap check stopped after {} overlaps ({} pairs compared): the page has too \
                 many overlapping visuals to list",
                found.min(MAX_OVERLAPS),
                checks.min(MAX_PAIR_CHECKS)
            ),
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

/// New positions for `targets` (visual indices on `page`) placed in `rects`, plus the members of
/// any group among them, moved and scaled with their group. Returns `(index, position)` pairs.
fn moves(page: &Page, targets: &[usize], rects: &[Rect]) -> Vec<(usize, Position)> {
    let chains = ancestors(page);
    let mut out = Vec::new();
    for (&t, rect) in targets.iter().zip(rects) {
        let Some(v) = page.visuals.get(t) else {
            continue;
        };
        let old = v.position;
        let new = Position {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            ..old
        };
        out.push((t, new));
        if v.visual_type.is_some() {
            continue;
        }
        // A group: move its members along.
        let group = v.name.to_lowercase();
        let sx = if old.width > 0.0 {
            new.width / old.width
        } else {
            1.0
        };
        let sy = if old.height > 0.0 {
            new.height / old.height
        } else {
            1.0
        };
        for (i, m) in page.visuals.iter().enumerate() {
            if !chains.get(i).is_some_and(|c| c.contains(&group)) {
                continue;
            }
            let p = m.position;
            let moved = Position {
                x: (new.x + (p.x - old.x) * sx).round(),
                y: (new.y + (p.y - old.y) * sy).round(),
                width: (p.width * sx).round().max(1.0),
                height: (p.height * sy).round().max(1.0),
                ..p
            };
            if [moved.x, moved.y, moved.width, moved.height]
                .iter()
                .all(|n| n.is_finite() && n.abs() <= MAX_SIZE)
            {
                out.push((i, moved));
            }
        }
    }
    out
}

/// Overlap warnings for `page` with `changes` applied, as strings.
fn overlap_texts(page: &Page, changes: &[(usize, Position)]) -> Vec<String> {
    let mut after = Page {
        name: page.name.clone(),
        display_name: page.display_name.clone(),
        width: page.width,
        height: page.height,
        visuals: page
            .visuals
            .iter()
            .map(|v| bicraft_report::Visual {
                name: v.name.clone(),
                position: v.position,
                visual_type: v.visual_type.clone(),
                parent_group: v.parent_group.clone(),
                is_hidden: v.is_hidden,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    for (i, p) in changes {
        if let Some(v) = after.visuals.get_mut(*i) {
            v.position = *p;
        }
    }
    let mut out = Vec::new();
    overlap_warnings(&after, |at, m| {
        if out.len() < 100 {
            let who = at.get("visual").and_then(Value::as_str).unwrap_or("");
            out.push(if who.is_empty() {
                m
            } else {
                format!("visual `{who}`: {m}")
            });
        }
    });
    out
}

/// A required whole-number parameter.
fn index_param(p: &Value, key: &str, min: u32) -> Result<Option<u32>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => whole(v)
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| (min..=MAX_TRACKS).contains(n))
            .map(Some)
            .ok_or_else(|| format!("parameter `{key}` must be a whole number from {min} to {MAX_TRACKS}")),
    }
}

pub(crate) fn visual_place(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &["page", "visual", "col", "row", "col_span", "row_span", "grid"],
    )?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let col = index_param(p, "col", 0)?.ok_or("missing whole-number parameter `col` (0-based)")?;
    let row = index_param(p, "row", 0)?.ok_or("missing whole-number parameter `row` (0-based)")?;
    let col_span = index_param(p, "col_span", 1)?;
    let row_span = index_param(p, "row_span", 1)?;
    let layout = match p.get("grid") {
        None | Some(Value::Null) => parse_layout(Some(&json!("grid")))?,
        Some(g @ Value::Object(_)) => parse_layout(Some(&json!({"grid": g})))?,
        Some(_) => return Err("`grid` must be an object {columns?, rows?, margin?, gutter?}".into()),
    };
    let request = Request {
        col: Some(col),
        row: Some(row),
        col_span,
        row_span,
        ..Default::default()
    };
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let vi = visual_index(page, visual)?;
    let name = page
        .visuals
        .get(vi)
        .map(|v| v.name.clone())
        .ok_or("internal: visual index out of range")?;
    let rects = place(page.width, page.height, &layout, &[request], &|_| {
        format!("visual `{name}`")
    })?;
    let changes = moves(page, &[vi], &rects);
    let warnings = overlap_texts(page, &changes);
    for (i, pos) in &changes {
        if let Some(v) = page.visuals.get_mut(*i) {
            v.position = *pos;
        }
    }
    let position = page.visuals.get(vi).map(|v| position_json(&v.position));
    let out = json!({
        "page": page.name,
        "visual": name,
        "position": position,
        "cell": {"col": col, "row": row, "col_span": col_span.unwrap_or(1), "row_span": row_span.unwrap_or(1)},
        "layout": layout.to_json(),
        "members_moved": changes.len().saturating_sub(1),
        "warnings": warnings,
    });
    s.report_dirty = true;
    Ok(out)
}

pub(crate) fn page_arrange(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "layout", "visuals", "dry_run"])?;
    let key = str_param(p, "page")?;
    let layout = parse_layout(p.get("layout"))?;
    let dry_run = opt_bool(p, "dry_run")?.unwrap_or(false);
    let listed: Option<&Vec<Value>> = match p.get("visuals") {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) if a.len() <= MAX_ITEMS => Some(a),
        Some(Value::Array(_)) => return Err(format!("at most {MAX_ITEMS} visuals can be arranged at once")),
        Some(_) => {
            return Err("`visuals` must be an array of visual names or {visual, place?} objects".into());
        }
    };
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;

    // Which visuals, in which order, with which placement requests.
    let mut targets: Vec<usize> = Vec::new();
    let mut requests: Vec<Request> = Vec::new();
    match listed {
        Some(items) => {
            let mut seen = HashSet::new();
            for (n, item) in items.iter().enumerate() {
                let (name, place_req) = match item {
                    Value::String(name) => (name.as_str(), None),
                    Value::Object(_) => {
                        allow(item, &["visual", "place"]).map_err(|e| format!("visuals[{n}]: {e}"))?;
                        let name = str_param(item, "visual").map_err(|e| format!("visuals[{n}]: {e}"))?;
                        (name, item.get("place"))
                    }
                    _ => {
                        return Err(format!(
                            "visuals[{n}] must be a visual name or {{visual, place?}}"
                        ));
                    }
                };
                let vi = visual_index(page, name)?;
                let v = page
                    .visuals
                    .get(vi)
                    .ok_or("internal: visual index out of range")?;
                if let Some(g) = &v.parent_group {
                    return Err(format!(
                        "visual `{}` is inside group `{g}`: arrange the group instead",
                        v.name
                    ));
                }
                if !seen.insert(vi) {
                    return Err(format!("visual `{}` is listed twice", v.name));
                }
                let mut req = parse_place(place_req).map_err(|e| format!("visual `{}`: {e}", v.name))?;
                if req.visual_type.is_none() {
                    req.visual_type.clone_from(&v.visual_type);
                }
                targets.push(vi);
                requests.push(req);
            }
        }
        None => {
            // Every visible top-level visual, in reading order (top to bottom, left to right).
            let mut all: Vec<usize> = page
                .visuals
                .iter()
                .enumerate()
                .filter(|(_, v)| v.parent_group.is_none() && !v.is_hidden)
                .map(|(i, _)| i)
                .collect();
            if all.len() > MAX_ITEMS {
                return Err(format!("the page has more than {MAX_ITEMS} visuals to arrange"));
            }
            all.sort_by(|&a, &b| {
                let (pa, pb) = (
                    page.visuals.get(a).map(|v| v.position),
                    page.visuals.get(b).map(|v| v.position),
                );
                match (pa, pb) {
                    (Some(pa), Some(pb)) => pa.y.total_cmp(&pb.y).then(pa.x.total_cmp(&pb.x)),
                    _ => std::cmp::Ordering::Equal,
                }
                .then(a.cmp(&b))
            });
            for i in all {
                let visual_type = page.visuals.get(i).and_then(|v| v.visual_type.clone());
                targets.push(i);
                requests.push(Request {
                    visual_type,
                    ..Default::default()
                });
            }
        }
    }

    let names: Vec<String> = targets
        .iter()
        .map(|&i| page.visuals.get(i).map(|v| v.name.clone()).unwrap_or_default())
        .collect();
    let rects = place(page.width, page.height, &layout, &requests, &|i| {
        format!("visual `{}`", names.get(i).map_or("?", String::as_str))
    })?;
    let changes = moves(page, &targets, &rects);
    let warnings = overlap_texts(page, &changes);
    let arranged: Vec<Value> = targets
        .iter()
        .zip(&changes_for(&changes, &targets))
        .filter_map(|(&i, new)| {
            let v = page.visuals.get(i)?;
            Some(json!({
                "visual": v.name,
                "visual_type": v.visual_type,
                "position": position_json(new),
                "previous": position_json(&v.position),
            }))
        })
        .collect();
    let members_moved = changes.len().saturating_sub(targets.len());
    let out = json!({
        "page": page.name,
        "display_name": page.display_name,
        "layout": layout.to_json(),
        "dry_run": dry_run,
        "arranged": arranged,
        "members_moved": members_moved,
        "warnings": warnings,
    });
    if !dry_run {
        for (i, pos) in &changes {
            if let Some(v) = page.visuals.get_mut(*i) {
                v.position = *pos;
            }
        }
        s.report_dirty = true;
    }
    Ok(out)
}

/// The new position of each target, in target order.
fn changes_for(changes: &[(usize, Position)], targets: &[usize]) -> Vec<Position> {
    let by_index: HashMap<usize, Position> = changes.iter().copied().collect();
    targets.iter().filter_map(|t| by_index.get(t).copied()).collect()
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
