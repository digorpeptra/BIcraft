//! BIcraft's chart pack (ROADMAP M12.3, docs/formats/chart-pack.md): the layouts of five of
//! BIcraft's custom visuals, the organization chart, the Venn diagram, the Marimekko, the
//! calendar heatmap and the Sankey diagram, and their drawing by the renderer (`report.render`, PDF export, the app).
//!
//! The same layouts are written in TypeScript for the visuals that run inside Power BI
//! (`crates/engine/src/chart_pack/*.ts`): both compute the same primitives and hits from the same
//! input, `{rows: [[cell…]…], options: {…}, lang: "en" | "fr"}`, which the engine's golden tests
//! compare (`crates/engine/tests/report/chart_pack_tests.rs`). Any change here goes to the
//! TypeScript too. The arithmetic stays within what both languages compute alike (IEEE doubles,
//! `floor`, `sqrt`, our own number formatting); text widths are estimated, never measured.
//!
//! The input is hostile: rows, texts and lists are capped and every loop is bounded.

use serde_json::{Map, Value, json};

use crate::canvas::Canvas;
use crate::charts::Bx;
use crate::data::{Ctx, Dataset, get, sk};
use crate::look::Look;
use crate::spec::{Prim, fmt_auto, r2};

pub mod calendar;
pub mod gauge;
pub mod linearea;
pub mod mekko;
pub mod org;
pub mod sankey;
pub mod scatter;
pub mod venn;
pub mod waterfall;

pub(crate) const PAD: f64 = 8.0;
pub(crate) const MS_PER_DAY: f64 = 86_400_000.0;

/// The kinds of the pack, as the snapshot's `chart_pack.kind` and the layouts name them.
pub const KINDS: &[&str] = &[
    "org",
    "venn",
    "mekko",
    "calendar",
    "sankey",
    "scatter",
    "linearea",
    "waterfall",
    "gauge",
];

/// The data roles each kind reads, in the order of its input's cells.
pub fn roles(kind: &str) -> &'static [&'static str] {
    match kind {
        "org" => &["id", "parent", "label", "value"],
        "venn" => &["item", "set"],
        "mekko" => &["category", "series", "value"],
        "calendar" => &["date", "value"],
        "sankey" => &["source", "destination", "weight"],
        "scatter" => &["details", "legend", "x", "y", "size"],
        "linearea" => &["axis", "series", "value", "value2"],
        "waterfall" => &["category", "group", "subtotal", "target", "values"],
        "gauge" => &[
            "trend",
            "value",
            "target",
            "min",
            "max",
            "threshold1",
            "threshold2",
            "threshold3",
            "previous",
        ],
        _ => &[],
    }
}

/// The role of a kind that takes several fields, the last of [`roles`] (each of its fields one
/// cell from there on, its fields' names the input's `names`), and the most fields it takes.
pub fn multi_role(kind: &str) -> Option<(&'static str, usize)> {
    match kind {
        "waterfall" => Some(("values", waterfall::MAX_MEASURES)),
        _ => None,
    }
}

/// A layout's result: the primitives and the elements that belong to data.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Out {
    pub prims: Vec<Prim>,
    /// `{row, prim, …}`: `prim` indexes `prims`, `row` the input row (or -1).
    pub hits: Vec<Value>,
}

impl Out {
    /// The golden tests' form (the TypeScript writes the same object).
    pub fn json(&self) -> Value {
        json!({
            "prims": self.prims.iter().map(Prim::json).collect::<Vec<_>>(),
            "hits": self.hits,
        })
    }

    pub(crate) fn hit(&mut self, row: i64, extra: Value) {
        self.hit_at(row, self.prims.len(), extra);
    }

    /// A hit on the primitive `prim` (one already drawn).
    pub(crate) fn hit_at(&mut self, row: i64, prim: usize, extra: Value) {
        let mut o = Map::new();
        o.insert("row".into(), json!(row));
        o.insert("prim".into(), json!(prim));
        if let Value::Object(e) = extra {
            o.extend(e);
        }
        self.hits.push(Value::Object(o));
    }
}

/// The layout of a visual of the pack by its kind (an unknown kind draws nothing).
pub fn layout(kind: &str, input: &Value, w: f64, h: f64) -> Out {
    match kind {
        "org" => org::layout(input, w, h),
        "venn" => venn::layout(input, w, h),
        "mekko" => mekko::layout(input, w, h),
        "calendar" => calendar::layout(input, w, h),
        "sankey" => sankey::layout(input, w, h),
        "scatter" => scatter::layout(input, w, h),
        "linearea" => linearea::layout(input, w, h),
        "waterfall" => waterfall::layout(input, w, h),
        "gauge" => gauge::layout(input, w, h),
        _ => Out::default(),
    }
}

// ---------------------------------------------------------------------------------------------
// Shared helpers (the same in kit.ts)
// ---------------------------------------------------------------------------------------------

pub(crate) fn chars(s: &str) -> usize {
    s.chars().count()
}

/// A text's estimated width (no font is measured, so both drawers agree).
pub(crate) fn est(s: &str, size: f64) -> f64 {
    chars(s) as f64 * size * 0.55
}

pub(crate) fn clip(s: &str, n: usize) -> String {
    if chars(s) <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// `s` shortened with an ellipsis so its estimated width fits `width` ("" when not 2
/// characters fit).
pub(crate) fn fit(s: &str, size: f64, width: f64) -> String {
    if est(s, size) <= width {
        return s.to_string();
    }
    let n = (width / (size * 0.55)).floor();
    if n < 2.0 || !n.is_finite() {
        return String::new();
    }
    clip(s, n.min(1e6) as usize)
}

/// A cell as text: numbers as BIcraft writes them, booleans as true / false, else "".
pub(crate) fn cell_text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n
            .as_f64()
            .filter(|x| x.is_finite())
            .map(fmt_auto)
            .unwrap_or_default(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

/// Cell `k` of a row as a label: a date written with its column's format string (the input's
/// `formats[k]`, [`crate::fmt::date_label`]), else as [`cell_text`] writes it.
pub(crate) fn cell_label(input: &Value, k: usize, v: Option<&Value>) -> String {
    let f: String = input
        .get("formats")
        .and_then(|l| l.get(k))
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .take(200)
        .collect();
    if let Some(Value::String(s)) = v
        && !f.is_empty()
        && let Some(l) = crate::spec::parse_iso(s)
            .and_then(|ms| crate::fmt::date_label(ms, &f, if is_fr(input) { "fr" } else { "en" }))
    {
        return l;
    }
    cell_text(v)
}

pub(crate) fn cell_num(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|x| x.is_finite())
}

/// Whether another visual's selection highlights the visual: the input has `highlights`, the
/// highlighted part of each row's measure (docs/formats/bicraft-visuals.md "Highlights").
pub(crate) fn highlighted(input: &Value) -> bool {
    input.get("highlights").is_some_and(Value::is_array)
}

/// Row `i`'s highlighted part (`None`: not highlighted).
pub(crate) fn hl_num(input: &Value, i: usize) -> Option<f64> {
    cell_num(input.get("highlights").and_then(|h| h.get(i)))
}

/// The role whose `highlights` the input carries: the kind's measure (none for the Venn
/// diagram, which has no measure).
pub fn measure_role(kind: &str) -> Option<&'static str> {
    match kind {
        "org" | "mekko" | "calendar" | "linearea" | "gauge" => Some("value"),
        "scatter" => Some("y"),
        "sankey" => Some("weight"),
        "waterfall" => Some("values"),
        _ => None,
    }
}

const NO_ROW: &[Value] = &[];

/// The input's rows (arrays), at most `max`; a row that is not an array is empty.
pub(crate) fn rows_of(input: &Value, max: usize) -> Vec<&[Value]> {
    input
        .get("rows")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .take(max)
                .map(|r| r.as_array().map_or(NO_ROW, Vec::as_slice))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn options_of(input: &Value) -> &Value {
    match input.get("options") {
        Some(o @ Value::Object(_)) => o,
        _ => &Value::Null,
    }
}

/// The look of the visual (docs/formats/visual-look.md): its flat Style options and the
/// input's `theme` (`{colors}`, the report theme's).
pub(crate) fn look_of(input: &Value) -> Look {
    Look::flat(options_of(input), input.get("theme"))
}

pub(crate) fn is_fr(input: &Value) -> bool {
    input.get("lang").and_then(Value::as_str) == Some("fr")
}

pub(crate) fn opt_num(o: &Value, k: &str, lo: f64, hi: f64, def: f64) -> f64 {
    o.get(k)
        .and_then(Value::as_f64)
        .filter(|x| x.is_finite())
        .map_or(def, |v| v.max(lo).min(hi))
}

pub(crate) fn opt_bool(o: &Value, k: &str, def: bool) -> bool {
    o.get(k).and_then(Value::as_bool).unwrap_or(def)
}

pub(crate) fn opt_color(o: &Value, k: &str, def: &str) -> String {
    match o.get(k).and_then(Value::as_str) {
        Some(c) if c.len() == 7 && c.starts_with('#') && c.bytes().skip(1).all(|b| b.is_ascii_hexdigit()) => {
            c.to_ascii_uppercase()
        }
        _ => def.to_string(),
    }
}

/// A list of texts (at most `max`).
pub(crate) fn opt_texts(o: &Value, k: &str, max: usize) -> Vec<String> {
    o.get(k)
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .take(max)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn rect(x: f64, y: f64, w: f64, h: f64, fill: &str, opacity: f64) -> Prim {
    Prim::Rect {
        x: r2(x),
        y: r2(y),
        w: r2(w),
        h: r2(h),
        fill: fill.to_string(),
        opacity,
        deco: crate::spec::Deco::default(),
    }
}

pub(crate) fn line(x1: f64, y1: f64, x2: f64, y2: f64, stroke: &str, width: f64) -> Prim {
    Prim::Line {
        x1: r2(x1),
        y1: r2(y1),
        x2: r2(x2),
        y2: r2(y2),
        stroke: stroke.to_string(),
        width: r2(width),
    }
}

/// A filled polygon (a Sankey link's ribbon).
pub(crate) fn poly(points: &[(f64, f64)], fill: &str, opacity: f64) -> Prim {
    Prim::Poly {
        points: points.iter().map(|&(x, y)| (r2(x), r2(y))).collect(),
        fill: fill.to_string(),
        opacity,
    }
}

pub(crate) fn circle(cx: f64, cy: f64, r: f64, fill: &str, opacity: f64) -> Prim {
    Prim::Circle {
        cx: r2(cx),
        cy: r2(cy),
        r: r2(r),
        fill: fill.to_string(),
        opacity,
        deco: crate::spec::Deco::default(),
    }
}

pub(crate) fn text(x: f64, y: f64, s: String, size: f64, fill: &str, anchor: &'static str) -> Prim {
    Prim::Text {
        x: r2(x),
        y: r2(y),
        text: s,
        size,
        fill: fill.to_string(),
        anchor,
        rotate: 0.0,
        weight: 400,
        font: crate::spec::TextFont::default(),
    }
}

fn hex(c: &str) -> (f64, f64, f64) {
    let p = |i: usize| {
        c.get(i..i + 2)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
            .map_or(0.0, f64::from)
    };
    (p(1), p(3), p(5))
}

pub(crate) fn lerp_color(a: &str, b: &str, t: f64) -> String {
    let (a, b) = (hex(a), hex(b));
    let ch = |x: f64, y: f64| (x + (y - x) * t + 0.5).floor().clamp(0.0, 255.0) as u8;
    format!("#{:02X}{:02X}{:02X}", ch(a.0, b.0), ch(a.1, b.1), ch(a.2, b.2))
}

/// A legend row at `y` (swatch 10×10 and name in 11 px) from `x0`, items dropped once the row
/// is full.
#[allow(clippy::too_many_arguments)]
pub(crate) fn legend(
    out: &mut Vec<Prim>,
    k: &Look,
    names: &[String],
    colors: &[String],
    x0: f64,
    y: f64,
    right: f64,
) {
    let mut x = x0;
    for (name, color) in names.iter().zip(colors) {
        let name = fit(name, k.name, 140.0);
        let wd = 14.0 + est(&name, k.name);
        if x + wd > right {
            break;
        }
        out.push(k.swatch(x, y, 10.0, color));
        out.push(text(x + 14.0, y + 5.0, name, k.name, k.label, "start"));
        x += wd + 14.0;
    }
}

/// One of `items` (`def` when the option is missing or not one of them).
pub(crate) fn opt_enum(o: &Value, k: &str, items: &[&'static str], def: &'static str) -> &'static str {
    let v = o.get(k).and_then(Value::as_str);
    items.iter().copied().find(|i| Some(*i) == v).unwrap_or(def)
}

/// A text option, at most `max` characters ("" when missing).
pub(crate) fn opt_text(o: &Value, k: &str, max: usize) -> String {
    o.get(k)
        .and_then(Value::as_str)
        .map(|s| s.chars().take(max).collect())
        .unwrap_or_default()
}

/// A number short enough for an axis or a label: thousands as K, millions as M, billions as B
/// (one decimal when not whole), else with `d` decimals (`None`: 0 or 2, as [`fmt_auto`]).
pub(crate) fn compact(v: f64, d: Option<u32>) -> String {
    let a = v.abs();
    let scaled = |x: f64, unit: &str| {
        let d = if (x - x.floor()).abs() < 1e-9 { 0 } else { 1 };
        format!("{}{unit}", crate::spec::fmt_fixed(x, d))
    };
    if a >= 1e9 {
        scaled(v / 1e9, "B")
    } else if a >= 1e6 {
        scaled(v / 1e6, "M")
    } else if a >= 1e4 {
        scaled(v / 1e3, "K")
    } else {
        match d {
            None => fmt_auto(v),
            Some(d) => crate::spec::fmt_fixed(v, d),
        }
    }
}

/// Most dashes one call draws.
pub(crate) const MAX_DASHES: usize = 4000;

/// A dashed polyline as line primitives (`on` pixels drawn, `off` skipped, the pattern carried
/// across corners), at most [`MAX_DASHES`]; pushed onto `out`.
pub(crate) fn dashes(
    out: &mut Vec<Prim>,
    points: &[(f64, f64)],
    stroke: &str,
    width: f64,
    on: f64,
    off: f64,
) {
    let mut phase = 0.0;
    let mut n = 0;
    let period = on + off;
    if on.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) || off.is_nan() || off < 0.0 {
        return;
    }
    for w in points.windows(2) {
        if n >= MAX_DASHES {
            break;
        }
        let [a, b] = w else { continue };
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt();
        if len.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) || !len.is_finite() {
            continue;
        }
        let mut s = 0.0;
        while s < len && n < MAX_DASHES {
            let in_on = phase < on;
            let step = (if in_on { on - phase } else { period - phase }).min(len - s);
            if in_on {
                let (t0, t1) = (s / len, (s + step) / len);
                out.push(line(
                    a.0 + dx * t0,
                    a.1 + dy * t0,
                    a.0 + dx * t1,
                    a.1 + dy * t1,
                    stroke,
                    width,
                ));
                n += 1;
            }
            s += step;
            phase += step;
            if phase >= period - 1e-9 {
                phase = 0.0;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Drawing (report.render, PDF export, the app's canvas)
// ---------------------------------------------------------------------------------------------

/// The input of a visual's layout from its snapshot (`chart_pack: {kind, options}`) and data:
/// one row per data row with the cells of the kind's roles in order (null when unbound).
pub fn input_of(v: &Value, ds: &Dataset, lang: &str) -> Value {
    let cp = get(v, "chart_pack");
    let kind = sk(cp, "kind");
    let multi = multi_role(kind);
    let mut cols: Vec<Option<usize>> = roles(kind)
        .iter()
        .filter(|r| multi.is_none_or(|(m, _)| m != **r))
        .map(|r| ds.cols.iter().position(|c| c.role.eq_ignore_ascii_case(r)))
        .collect();
    // a role of several fields: one cell for each, their names in `names`
    let many: Vec<usize> = multi
        .map(|(m, max)| {
            (0..ds.cols.len())
                .filter(|i| ds.cols.get(*i).is_some_and(|c| c.role.eq_ignore_ascii_case(m)))
                .take(max)
                .collect()
        })
        .unwrap_or_default();
    cols.extend(many.iter().map(|i| Some(*i)));
    let rows: Vec<Value> = if ds.empty {
        Vec::new()
    } else {
        ds.rows
            .iter()
            .map(|r| {
                Value::Array(
                    cols.iter()
                        .map(|c| c.and_then(|i| r.get(i)).cloned().unwrap_or(Value::Null))
                        .collect(),
                )
            })
            .collect()
    };
    let formats: Vec<&str> = cols
        .iter()
        .map(|c| c.and_then(|i| ds.cols.get(i)).map_or("", |c| c.fmt.as_str()))
        .collect();
    // the fields' names (Scatter+ titles its axes with them)
    let names: Vec<String> = cols
        .iter()
        .map(|c| {
            c.and_then(|i| ds.cols.get(i))
                .map_or_else(String::new, |c| c.name.chars().take(80).collect())
        })
        .collect();
    let mut input = json!({
        "rows": rows,
        "formats": formats,
        "names": names,
        "options": get(cp, "options"),
        "lang": if lang.to_ascii_lowercase().starts_with("fr") { "fr" } else { "en" },
    });
    // today, as the snapshot resolved it (Line / Area+'s forecast from today)
    if let Some(t) = cp.get("today").and_then(Value::as_f64).filter(|t| t.is_finite()) {
        input["today"] = json!(t);
    }
    if multi.is_some() {
        let names: Vec<&str> = many
            .iter()
            .map(|i| ds.cols.get(*i).map_or("", |c| c.name.as_str()))
            .collect();
        input["names"] = json!(names);
        // highlighted: each row's parts of the role's fields
        if let (Some(h), false) = (&ds.hl, ds.empty) {
            let parts: Vec<Value> = (0..ds.rows.len())
                .map(|r| {
                    Value::Array(
                        many.iter()
                            .map(|c| h.get(r).and_then(|x| x.get(*c)).cloned().unwrap_or(Value::Null))
                            .collect(),
                    )
                })
                .collect();
            input["highlights"] = Value::Array(parts);
        }
        return input;
    }
    // another visual's selection highlighting this one: the measure's highlighted parts
    let measure =
        measure_role(kind).and_then(|r| ds.cols.iter().position(|c| c.role.eq_ignore_ascii_case(r)));
    if let (Some(h), Some(m), false) = (&ds.hl, measure, ds.empty) {
        let parts: Vec<Value> = (0..ds.rows.len())
            .map(|r| h.get(r).and_then(|x| x.get(m)).cloned().unwrap_or(Value::Null))
            .collect();
        input["highlights"] = Value::Array(parts);
    }
    input
}

/// The body of a visual of the pack: its layout over its data, drawn as its TypeScript draws
/// it in Power BI (unselected, every node as its saved collapse state says).
pub(crate) fn body(cv: &mut Canvas, v: &Value, ds: &Dataset, b: Bx, ctx: &Ctx<'_>) -> Result<(), String> {
    let kind = sk(get(v, "chart_pack"), "kind");
    if !KINDS.contains(&kind) {
        return Err(format!("unknown chart pack kind `{}`", clip(kind, 40)));
    }
    let mut input = input_of(v, ds, &ctx.ui_lang);
    input["theme"] = crate::spec::theme_value(ctx);
    let out = layout(kind, &input, f64::from(b.w), f64::from(b.h));
    let targets = targets(kind, &input, &out, ds, ctx);
    // the columns of the selection ids (the TypeScript's `idOf` columns)
    let id_roles: &[&str] = match kind {
        "org" => &["id"],
        "venn" => &["set"],
        "mekko" => &["category", "series"],
        "calendar" => &["date"],
        "scatter" => &["details", "legend"],
        "linearea" => &["axis", "series"],
        "waterfall" => &["category", "group", "subtotal"],
        "gauge" => &["trend"],
        _ => &["source", "destination"],
    };
    let ids = crate::own_hits::id_cols(ds, id_roles);
    crate::own_hits::draw(cv, &out.prims, &targets, v, ds, &ids, ctx);
    Ok(())
}

/// What each hit of a layout selects and shows, as the visual's TypeScript (`*_visual.ts`
/// `point`) does: an org chart node its row, a Venn circle every row of its set (a region shows
/// its count only), a Marimekko segment and a calendar day their row, a Sankey link its source
/// and destination's rows and a node every row it is in. An org chart's collapse button selects
/// nothing: its click sends `view.interact` `tree_toggle` for its node (the TypeScript's `flip`),
/// which the app saves as `tree.toggled` (`visual.toggle_node`).
fn targets(
    kind: &str,
    input: &Value,
    out: &Out,
    ds: &Dataset,
    ctx: &Ctx<'_>,
) -> Vec<crate::own_hits::Target> {
    use crate::own_hits::{Target, num_text, role_col, tip_line, word};
    match kind {
        "waterfall" => return waterfall::targets(input, out, ds, ctx),
        "gauge" => return gauge::targets(out, ds, ctx),
        _ => {}
    }
    let rows = rows_of(input, 100_000);
    let col = |role: &str| role_col(ds, role);
    let text_at = |r: usize, k: usize| cell_text(rows.get(r).and_then(|x| x.get(k)));
    let field = |k: usize, en: &'static str, fr: &'static str| {
        let role = roles(kind).get(k).copied().unwrap_or("");
        col(role)
            .and_then(|c| ds.cols.get(c))
            .map(|c| c.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| word(ctx, en, fr))
    };
    let num = |h: &Value, k: &str| h.get(k).and_then(Value::as_f64).unwrap_or(f64::NAN);
    let s = |h: &Value, k: &str| h.get(k).map(|x| cell_text(Some(x))).unwrap_or_default();
    // the drawn Venn sets' names, by their index in the layout
    let mut sets: Vec<String> = Vec::new();
    if kind == "venn" {
        for h in &out.hits {
            if let (Some(j), Some(r)) = (
                h.get("set").and_then(Value::as_u64),
                h.get("row").and_then(Value::as_u64),
            ) {
                let j = usize::try_from(j).unwrap_or(usize::MAX).min(8);
                if sets.len() <= j {
                    sets.resize(j + 1, String::new());
                }
                if let Some(x) = sets.get_mut(j) {
                    *x = text_at(usize::try_from(r).unwrap_or(usize::MAX), 1);
                }
            }
        }
    }
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
        let one = |r: Option<usize>| r.map(|r| vec![r]).unwrap_or_default();
        let at = |c: Option<usize>| row.map(|r| tip_line(ds, c, r, ctx)).unwrap_or_default();
        let (rows_sel, tip): (Vec<usize>, Vec<(String, String)>) = match kind {
            "org" => {
                if h.get("toggle").and_then(Value::as_bool) == Some(true) {
                    // the collapse button (its dot, then its sign): a click toggles the node,
                    // `view.interact` `tree_toggle` (the app saves it, `visual.toggle_node`)
                    let open =
                        matches!(out.prims.get(prim + 1), Some(Prim::Text { text, .. }) if text == "−");
                    let label = row.map(|r| text_at(r, 2)).filter(|s| !s.is_empty());
                    let id = s(h, "id");
                    let what = if open {
                        word(ctx, "Collapse", "Réduire")
                    } else {
                        word(ctx, "Expand", "Développer")
                    };
                    list.push(Target {
                        prims: vec![prim, prim + 1],
                        tip: vec![(what, label.unwrap_or_else(|| id.clone()))],
                        send: Some(json!({"action": "tree_toggle", "id": id})),
                        ..Target::default()
                    });
                    continue;
                }
                let mut tip = at(col("label"));
                tip.extend(at(col("id")));
                tip.extend(at(col("parent")));
                tip.extend(at(col("value")));
                (one(row), tip)
            }
            "venn" => {
                let items = (
                    word(ctx, "Items", "Éléments"),
                    num_text(ds, None, num(h, "n"), ctx),
                );
                match h.get("set").and_then(Value::as_u64) {
                    Some(_) => {
                        let name = row.map(|r| text_at(r, 1)).unwrap_or_default();
                        let members: Vec<usize> =
                            (0..rows.len()).filter(|r| text_at(*r, 1) == name).collect();
                        let mut tip = at(col("set"));
                        tip.push(items);
                        (members, tip)
                    }
                    None => {
                        let m = h.get("region").and_then(Value::as_u64).unwrap_or(0);
                        let (mut inside, mut outside) = (Vec::new(), Vec::new());
                        for (j, n) in sets.iter().enumerate() {
                            if m & (1 << j) != 0 {
                                inside.push(n.clone());
                            } else {
                                outside.push(n.clone());
                            }
                        }
                        let mut name = inside.join(" ∩ ");
                        if !outside.is_empty() {
                            name.push_str(&format!(
                                " ({}{})",
                                word(ctx, "not ", "sans "),
                                outside.join(", ")
                            ));
                        }
                        (Vec::new(), vec![(word(ctx, "Region", "Zone"), name), items])
                    }
                }
            }
            "mekko" => {
                let mut tip = at(col("category"));
                tip.extend(at(col("series")));
                tip.extend(at(col("value")));
                let share = num(h, "share");
                if share.is_finite() {
                    let pct = crate::fmt::ColFmt {
                        fmt: "0.0%".into(),
                        kind: String::new(),
                        data_type: "double".into(),
                    };
                    tip.push((
                        word(ctx, "Share of the category", "Part de la catégorie"),
                        crate::fmt::fmt_cell(&json!(share), Some(&pct), &ctx.loc, false),
                    ));
                }
                (one(row), tip)
            }
            "calendar" => {
                let mut tip = at(col("date"));
                if let Some(vc) = col("value") {
                    let name = ds.cols.get(vc).map(|c| c.name.clone()).unwrap_or_default();
                    tip.push((name, num_text(ds, Some(vc), num(h, "value"), ctx)));
                }
                (one(row), tip)
            }
            "scatter" => match h.get("rows").and_then(Value::as_array) {
                // a density cell: its rows, and how many points it holds
                Some(list) => {
                    let sel = list
                        .iter()
                        .filter_map(|r| r.as_u64().and_then(|r| usize::try_from(r).ok()))
                        .collect();
                    let tip = vec![(
                        word(ctx, "Points", "Points"),
                        num_text(ds, None, num(h, "n"), ctx),
                    )];
                    (sel, tip)
                }
                None => {
                    let mut tip = at(col("details"));
                    for role in ["legend", "x", "y", "size"] {
                        tip.extend(at(col(role)));
                    }
                    (one(row), tip)
                }
            },
            "linearea" => {
                if h.get("line").is_some() {
                    // a whole line (too many points for one target each): its series' rows
                    let name = row.map(|r| text_at(r, 1)).unwrap_or_default();
                    let sel = (0..rows.len()).filter(|r| text_at(*r, 1) == name).collect();
                    (sel, at(col("series")))
                } else {
                    let mut tip = at(col("axis"));
                    for role in ["series", "value", "value2"] {
                        tip.extend(at(col(role)));
                    }
                    (one(row), tip)
                }
            }
            _ => {
                let wc = col("weight");
                let value = |x: f64| num_text(ds, wc, x, ctx);
                if h.get("link").is_some() {
                    let (src, dst) = (s(h, "source"), s(h, "target"));
                    let sel = (0..rows.len())
                        .filter(|r| text_at(*r, 0) == src && text_at(*r, 1) == dst)
                        .collect();
                    let tip = vec![
                        (field(0, "Source", "Source"), s(h, "sourceLabel")),
                        (field(1, "Destination", "Destination"), s(h, "targetLabel")),
                        (field(2, "Weight", "Poids"), value(num(h, "value"))),
                    ];
                    (sel, tip)
                } else {
                    let name = s(h, "name");
                    let sel = (0..rows.len())
                        .filter(|r| text_at(*r, 0) == name || text_at(*r, 1) == name)
                        .collect();
                    let mut tip = vec![(word(ctx, "Node", "Nœud"), s(h, "label"))];
                    for (k, en, fr) in [
                        ("inflow", "Incoming", "Entrant"),
                        ("outflow", "Outgoing", "Sortant"),
                    ] {
                        let x = num(h, k);
                        if x > 0.0 {
                            tip.push((word(ctx, en, fr), value(x)));
                        }
                    }
                    (sel, tip)
                }
            }
        };
        list.push(Target {
            rows: rows_sel,
            prims: vec![prim],
            at: Vec::new(),
            tip,
            faded: h.get("fade").and_then(Value::as_bool) == Some(true),
            send: None,
        });
    }
    list
}

#[cfg(test)]
#[path = "chart_pack_tests.rs"]
mod tests;
