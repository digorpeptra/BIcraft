//! BIcraft's own chart visuals (ROADMAP M12.3, docs/formats/bicraft-visuals.md): the histogram,
//! the bullet chart, the box plot and the dumbbell / slope chart, laid out into the Spec visual's
//! drawing primitives ([`Prim`]).
//!
//! Each layout is written twice: here, for `report.render`, PDF export and the app's canvas, and
//! in TypeScript for the custom visual that runs inside Power BI (`crates/engine/src/bvis/`).
//! Both compute the same primitives from the same data and options, which the engine's golden
//! tests compare (`tests/report/bvis_tests.rs`); any change here goes to the TypeScript too. As
//! for the Spec visual, the arithmetic is kept to what both languages compute alike and text
//! widths are estimated from the character count.
//!
//! The data is a list of columns, each bound to one of the visual's data roles (the snapshot's
//! `data`, or the golden tests' `{columns: [{role, name, values}]}`); the options are the
//! visual's format objects as plain JSON (`{bins: {count: 20}}`). Both are hostile: rows,
//! columns and texts are capped and every option is checked, a bad one falling back to its
//! default.

use serde_json::{Value, json};

use crate::canvas::Canvas;
use crate::charts::Bx;
use crate::data::{Ctx, Dataset};
use crate::look::Look;
use crate::own_hits;
use crate::spec::{
    Domain, Prim, date_text, est, fmt_auto, fmt_fixed, inside, parse_iso, r2, text, tick_decimals,
};

#[path = "bvis_box.rs"]
pub mod box_plot;
#[path = "bvis_bullet.rs"]
pub mod bullet;
#[path = "bvis_dumbbell.rs"]
pub mod dumbbell;
#[path = "bvis_histogram.rs"]
pub mod histogram;

/// The visuals, by the kind the snapshot's `bicraft_visual.kind` names.
pub const KINDS: &[&str] = &["histogram", "bullet", "box_plot", "dumbbell"];
/// Most rows read.
pub const MAX_ROWS: usize = 30_000;
/// Most columns read.
pub const MAX_COLUMNS: usize = 16;
/// Most characters of a text cell.
const MAX_CELL: usize = 200;

/// Padding around every visual.
pub(crate) const PAD: f64 = 8.0;

// ---------------------------------------------------------------------------------------------
// The data
// ---------------------------------------------------------------------------------------------

/// A cell: a number, a text (dates written `YYYY-MM-DD[ HH:MM]`) or blank.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Num(f64),
    Text(String),
    Null,
}

/// A column bound to a data role.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Column {
    pub role: String,
    pub name: String,
    /// The column's format string (its labels of dates are written with it).
    pub format: String,
    /// The language its labels are written in (`en`, `fr`: the UI's, as Power BI's `host.locale`).
    pub lang: String,
    pub cells: Vec<Cell>,
    /// The highlighted part of each row (the data view's `highlights` of a value column, another
    /// visual's selection highlighting this one: docs/formats/bicraft-visuals.md "Highlights");
    /// `None` when nothing is highlighted.
    pub hl: Option<Vec<Cell>>,
}

impl Column {
    /// Row `i`'s number (a finite one), else `None`.
    pub fn num(&self, i: usize) -> Option<f64> {
        match self.cells.get(i) {
            Some(Cell::Num(n)) if n.is_finite() => Some(*n),
            _ => None,
        }
    }

    /// Row `i`'s highlighted part (a finite number), else `None` (not highlighted).
    pub fn hl(&self, i: usize) -> Option<f64> {
        match self.hl.as_ref()?.get(i) {
            Some(Cell::Num(n)) if n.is_finite() => Some(*n),
            _ => None,
        }
    }

    /// Row `i` as text (numbers as `fmt_auto` writes them, blanks empty).
    pub fn text(&self, i: usize) -> String {
        match self.cells.get(i) {
            Some(Cell::Num(n)) => fmt_auto(*n),
            Some(Cell::Text(s)) => s.clone(),
            _ => String::new(),
        }
    }

    /// Row `i` as a label: a date written with the column's format string (`date_label`), else
    /// as [`Column::text`] writes it.
    pub fn label(&self, i: usize) -> String {
        if let Some(Cell::Text(s)) = self.cells.get(i)
            && let Some(l) = parse_iso(s).and_then(|ms| crate::fmt::date_label(ms, &self.format, &self.lang))
        {
            return l;
        }
        self.text(i)
    }
}

/// The visual's columns.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Data {
    pub cols: Vec<Column>,
}

fn cell_text(s: &str) -> Cell {
    Cell::Text(s.chars().take(MAX_CELL).collect())
}

fn cell_of(v: &Value) -> Cell {
    match v {
        Value::Number(n) => n.as_f64().filter(|x| x.is_finite()).map_or(Cell::Null, Cell::Num),
        Value::String(s) => cell_text(s),
        Value::Bool(b) => Cell::Text(b.to_string()),
        _ => Cell::Null,
    }
}

impl Data {
    /// The golden tests' form: `{locale?, columns: [{role, name?, format?, values: [number |
    /// text | null]}]}`.
    pub fn from_json(v: &Value) -> Data {
        let lang = v.get("locale").and_then(Value::as_str).unwrap_or("");
        let cols = v
            .get("columns")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .take(MAX_COLUMNS)
                    .map(|c| Column {
                        role: c.get("role").and_then(Value::as_str).unwrap_or("").to_string(),
                        name: c
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .chars()
                            .take(MAX_CELL)
                            .collect(),
                        format: format_of(c.get("format").and_then(Value::as_str).unwrap_or("")),
                        lang: lang_of(lang),
                        cells: c
                            .get("values")
                            .and_then(Value::as_array)
                            .map(|r| r.iter().take(MAX_ROWS).map(cell_of).collect())
                            .unwrap_or_default(),
                        hl: c
                            .get("highlights")
                            .and_then(Value::as_array)
                            .map(|r| r.iter().take(MAX_ROWS).map(cell_of).collect()),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Data { cols }
    }

    /// Rows: the longest column's.
    pub fn rows(&self) -> usize {
        self.cols
            .iter()
            .map(|c| c.cells.len())
            .max()
            .unwrap_or(0)
            .min(MAX_ROWS)
    }

    /// The `k`-th column of `role` (roles compared ignoring case).
    pub fn col(&self, role: &str, k: usize) -> Option<&Column> {
        self.cols
            .iter()
            .filter(|c| c.role.eq_ignore_ascii_case(role))
            .nth(k)
    }

    /// The columns of `role`.
    pub fn count(&self, role: &str) -> usize {
        self.cols
            .iter()
            .filter(|c| c.role.eq_ignore_ascii_case(role))
            .count()
    }

    /// Whether another visual's selection highlights this one (a column has `highlights`).
    pub fn highlighted(&self) -> bool {
        self.cols.iter().any(|c| c.hl.is_some())
    }

    /// Whether row `i` is highlighted: a value column has a highlighted part for it.
    pub fn lit(&self, i: usize) -> bool {
        self.cols.iter().any(|c| c.hl(i).is_some())
    }

    /// The units none of whose rows is highlighted while the visual is (drawn dimmed: the
    /// visuals whose marks have no highlighted part to draw, docs/formats/bicraft-visuals.md
    /// "Highlights").
    pub(crate) fn fade(&self, u: &mut Units) {
        if !self.highlighted() {
            return;
        }
        u.faded = u
            .units
            .iter()
            .enumerate()
            .filter(|(_, x)| !x.rows.iter().any(|r| self.lit(*r)))
            .map(|(k, _)| k)
            .collect();
    }
}

/// A column's format string as the layouts read it (its first 200 characters).
fn format_of(f: &str) -> String {
    f.chars().take(MAX_CELL).collect()
}

/// The labels' language: `fr` for a French tag, else `en`.
fn lang_of(lang: &str) -> String {
    if crate::fmt::is_fr(lang) { "fr" } else { "en" }.to_string()
}

/// The data of a visual's snapshot dataset: one column per bound field, with its role, labels
/// in the UI's language `lang`.
pub(crate) fn data_of(ds: &Dataset, lang: &str) -> Data {
    let cols = ds
        .cols
        .iter()
        .enumerate()
        .take(MAX_COLUMNS)
        .map(|(k, c)| {
            let date = c.is_date();
            Column {
                role: c.role.clone(),
                name: c.name.chars().take(MAX_CELL).collect(),
                format: format_of(&c.fmt),
                lang: lang_of(lang),
                cells: ds
                    .rows
                    .iter()
                    .take(MAX_ROWS)
                    .map(|r| match r.get(k) {
                        Some(Value::String(s)) if date => parse_iso(s).map_or_else(
                            || cell_text(s),
                            |ms| Cell::Text(date_text(ms, (ms / 86_400_000.0).floor() * 86_400_000.0 != ms)),
                        ),
                        Some(v) => cell_of(v),
                        None => Cell::Null,
                    })
                    .collect(),
                // the highlights of a value column (the engine writes null under the grouping ones)
                hl: ds
                    .hl
                    .as_ref()
                    .filter(|_| c.kind == "measure" || c.kind == "aggregation")
                    .map(|h| {
                        (0..ds.rows.len().min(MAX_ROWS))
                            .map(|r| h.get(r).and_then(|x| x.get(k)).map_or(Cell::Null, cell_of))
                            .collect()
                    }),
            }
        })
        .collect();
    Data { cols }
}

// ---------------------------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------------------------

/// The visual's format objects as plain JSON: `{object: {property: value}}`.
#[derive(Debug, Clone, Copy)]
pub struct Opts<'a>(pub &'a Value);

impl Opts<'_> {
    fn get(&self, obj: &str, prop: &str) -> Option<&Value> {
        self.0.get(obj)?.get(prop)
    }

    /// A number within `lo..=hi`, else `def`.
    pub fn num(&self, obj: &str, prop: &str, def: f64, lo: f64, hi: f64) -> f64 {
        match self.get(obj, prop).and_then(Value::as_f64) {
            Some(v) if v.is_finite() => v.clamp(lo, hi),
            _ => def,
        }
    }

    /// A whole number within `lo..=hi` (rounded down), else `def`.
    pub fn int(&self, obj: &str, prop: &str, def: f64, lo: f64, hi: f64) -> f64 {
        self.num(obj, prop, def, lo, hi).floor()
    }

    pub fn flag(&self, obj: &str, prop: &str, def: bool) -> bool {
        self.get(obj, prop).and_then(Value::as_bool).unwrap_or(def)
    }

    /// The look (`crate::look`): the `style` object's options and the theme the renderer adds
    /// as `$theme` (`{colors}`).
    pub fn look(&self) -> Look {
        Look::of(self.0.get("style"), self.0.get("$theme"))
    }

    /// A `#RRGGBB` colour (upper case), else `def`.
    pub fn color(&self, obj: &str, prop: &str, def: &str) -> String {
        match self.get(obj, prop).and_then(Value::as_str) {
            Some(c) if is_color(c) => c.to_ascii_uppercase(),
            _ => def.to_string(),
        }
    }
}

/// `#RRGGBB`.
pub fn is_color(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c.bytes().skip(1).all(|b| b.is_ascii_hexdigit())
}

// ---------------------------------------------------------------------------------------------
// Drawing helpers (the same in common.ts)
// ---------------------------------------------------------------------------------------------

pub(crate) fn path(points: &[(f64, f64)], stroke: &str, width: f64) -> Prim {
    Prim::Path {
        points: points.iter().map(|(x, y)| (r2(*x), r2(*y))).collect(),
        stroke: stroke.to_string(),
        width: r2(width),
    }
}

/// A linear scale: domain `d` onto pixels `a` (at `d.lo`) to `b` (at `d.hi`).
pub(crate) fn scale(d: &Domain, a: f64, b: f64, v: f64) -> f64 {
    a + (v - d.lo) / (d.hi - d.lo) * (b - a)
}

/// Tick labels of a vertical axis at `x` (right-aligned), one per tick of `d` between `bottom`
/// (`d.lo`) and `top` (`d.hi`).
pub(crate) fn y_labels(out: &mut Vec<Prim>, k: &Look, d: &Domain, x: f64, top: f64, bottom: f64) {
    let dec = tick_decimals(d.step);
    for t in &d.ticks {
        out.push(text(
            x,
            scale(d, bottom, top, *t),
            fmt_fixed(*t, dec),
            k.tick,
            k.text,
            "end",
            0.0,
        ));
    }
}

/// Horizontal gridlines at the ticks of `d`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn y_grid(
    out: &mut Vec<Prim>,
    k: &Look,
    d: &Domain,
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
) {
    for t in &d.ticks {
        let y = scale(d, bottom, top, *t);
        out.push(crate::spec::line(left, y, right, y, k.grid, 1.0));
    }
}

/// A horizontal axis at `bottom`: its line, and tick labels centred on their ticks, kept inside
/// the visual's width `w`, a label that would come within 4 px of the previous one skipped.
#[allow(clippy::too_many_arguments)]
pub(crate) fn x_axis(out: &mut Vec<Prim>, k: &Look, d: &Domain, left: f64, right: f64, bottom: f64, w: f64) {
    out.push(crate::spec::line(left, bottom, right, bottom, k.axis, 1.0));
    let dec = tick_decimals(d.step);
    let mut end = f64::NEG_INFINITY;
    for t in &d.ticks {
        let s = fmt_fixed(*t, dec);
        let half = est(&s, k.tick) / 2.0;
        let x = inside(scale(d, left, right, *t), &s, k.tick, w);
        if x - half < end + 4.0 {
            continue;
        }
        end = x + half;
        out.push(text(x, bottom + 9.0, s, k.tick, k.text, "middle", 0.0));
    }
}

/// A legend row at the top: a 10 px swatch and the name of each item, as the Spec visual draws
/// it; items that do not fit are left out.
pub(crate) fn legend(out: &mut Vec<Prim>, k: &Look, items: &[(String, String)], w: f64) {
    let mut x = PAD;
    for (j, (name, c)) in items.iter().enumerate() {
        let name = crate::spec::clip(name, 30);
        let wd = 14.0 + est(&name, k.name) + 12.0;
        if j > 0 && x + wd - 12.0 > w - PAD {
            break;
        }
        out.push(k.swatch(x, PAD + 4.0, 10.0, c));
        out.push(text(x + 14.0, PAD + 9.0, name, k.name, k.text, "start", 0.0));
        x += wd;
    }
}

/// The width of a label column: the widest of `labels` at `size` plus 8 px, at most `max`.
pub(crate) fn label_width(labels: &[String], size: f64, max: f64) -> f64 {
    (labels.iter().map(|l| est(l, size)).fold(0.0, f64::max) + 8.0).min(max)
}

/// `s` clipped to the characters that fit in `width` at `size` (at least one).
pub(crate) fn fit(s: &str, width: f64, size: f64) -> String {
    let n = (width / (size * 0.55)).floor().clamp(1.0, 60.0) as usize;
    crate::spec::clip(s, n)
}

// ---------------------------------------------------------------------------------------------
// Layout and drawing
// ---------------------------------------------------------------------------------------------

/// A unit the user selects and hovers (a bin, a bullet, a box, a row): its data rows and numbers
/// (the TypeScript's `Unit`; `stats` as each visual's tooltip reads them).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Unit {
    pub rows: Vec<usize>,
    pub stats: Vec<f64>,
}

/// What a layout records for interactions besides its primitives (the TypeScript's `Out`): the
/// units and `(unit, prim)` for each primitive drawn for a unit, the unit's first one first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Units {
    pub units: Vec<Unit>,
    pub hits: Vec<(usize, usize)>,
    /// The units drawn dimmed because another visual's selection highlights none of their rows
    /// ([`Data::fade`]), ascending.
    pub faded: Vec<usize>,
}

impl Units {
    /// A new unit of `rows` drawn by the primitives `prims` (its focus first).
    pub(crate) fn unit(&mut self, rows: Vec<usize>, stats: Vec<f64>, prims: &[usize]) {
        let k = self.units.len();
        self.units.push(Unit { rows, stats });
        self.hits.extend(prims.iter().map(|p| (k, *p)));
    }
}

/// Lays out visual `kind` over `data` with `opts` in a `w`×`h` visual.
pub fn layout(kind: &str, data: &Data, opts: &Value, w: f64, h: f64) -> Result<Vec<Prim>, String> {
    layout_units(kind, data, opts, w, h).map(|(p, _)| p)
}

/// [`layout`] with the units it draws (the primitives are the same).
pub fn layout_units(
    kind: &str,
    data: &Data,
    opts: &Value,
    w: f64,
    h: f64,
) -> Result<(Vec<Prim>, Units), String> {
    let o = Opts(opts);
    let mut out = Vec::new();
    let mut u = Units::default();
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
        return Ok((out, u));
    }
    match kind {
        // highlighted, the bullet draws each value's highlighted part; the others dim the units
        // none of whose rows is highlighted (docs/formats/bicraft-visuals.md "Highlights")
        "histogram" => {
            histogram::layout(data, o, w, h, &mut out, &mut u);
            data.fade(&mut u);
        }
        "bullet" => bullet::layout(data, o, w, h, &mut out, &mut u),
        "box_plot" => {
            box_plot::layout(data, o, w, h, &mut out, &mut u);
            data.fade(&mut u);
        }
        "dumbbell" => {
            dumbbell::layout(data, o, w, h, &mut out, &mut u);
            data.fade(&mut u);
        }
        _ => {
            return Err(format!(
                "unknown BIcraft visual `{}` (one of {})",
                kind.chars().take(40).collect::<String>(),
                KINDS.join(", ")
            ));
        }
    }
    Ok((out, u))
}

/// The golden tests' form of [`layout`].
pub fn layout_json(kind: &str, data: &Data, opts: &Value, w: f64, h: f64) -> Value {
    match layout(kind, data, opts, w, h) {
        Ok(p) => Value::Array(p.iter().map(Prim::json).collect()),
        Err(e) => json!({"error": e}),
    }
}

/// The body of one of BIcraft's visuals: the snapshot's `bicraft_visual` (`{kind, options}`) over
/// its data.
pub(crate) fn body(cv: &mut Canvas, v: &Value, ds: &Dataset, b: Bx, ctx: &Ctx<'_>) -> Result<(), String> {
    let bv = crate::data::get(v, "bicraft_visual");
    let kind = bv.get("kind").and_then(Value::as_str).unwrap_or("");
    let data = if ds.empty {
        Data::default()
    } else {
        data_of(ds, &ctx.ui_lang)
    };
    let mut opts = bv.get("options").cloned().unwrap_or(Value::Null);
    if !opts.is_object() {
        opts = json!({});
    }
    if let Some(o) = opts.as_object_mut() {
        o.insert("$theme".into(), crate::spec::theme_value(ctx));
    }
    let (prims, units) = layout_units(kind, &data, &opts, f64::from(b.w), f64::from(b.h))?;
    let mut targets: Vec<own_hits::Target> = units
        .units
        .iter()
        .enumerate()
        .map(|(k, u)| own_hits::Target {
            rows: u.rows.clone(),
            tip: tooltip(kind, ds, u, ctx),
            faded: units.faded.contains(&k),
            ..own_hits::Target::default()
        })
        .collect();
    for (k, p) in &units.hits {
        if let Some(t) = targets.get_mut(*k) {
            t.prims.push(*p);
        }
    }
    // the role whose column gives the selection ids (the TypeScript's `idRole`)
    let id_role = if kind == "histogram" { "values" } else { "category" };
    let ids = own_hits::id_cols(ds, &[id_role]);
    own_hits::draw(cv, &prims, &targets, v, ds, &ids, ctx);
    Ok(())
}

/// A unit's tooltip, as each visual's TypeScript writes it.
fn tooltip(kind: &str, ds: &Dataset, u: &Unit, ctx: &Ctx<'_>) -> Vec<(String, String)> {
    let w = |en, fr| own_hits::word(ctx, en, fr);
    let stat = |k: usize| u.stats.get(k).copied().unwrap_or(f64::NAN);
    match kind {
        "histogram" => {
            let vc = own_hits::role_col(ds, "values");
            let fc = own_hits::role_col(ds, "frequency");
            let name = |c: Option<usize>, def: String| {
                c.and_then(|c| ds.cols.get(c))
                    .map(|c| crate::data::clip(&c.name, 120))
                    .unwrap_or(def)
            };
            let mut count = name(fc, w("Count", "Nombre"));
            if stat(3) == 1.0 {
                count.push_str(&w(" (cumulative)", " (cumulé)"));
            }
            vec![
                (
                    name(vc, w("Bin", "Intervalle")),
                    format!(
                        "{} – {}",
                        own_hits::num_text(ds, vc, stat(0), ctx),
                        own_hits::num_text(ds, vc, stat(1), ctx)
                    ),
                ),
                (count, own_hits::num_text(ds, fc, stat(2), ctx)),
            ]
        }
        "box_plot" => {
            let vc = own_hits::role_col(ds, "value");
            let mut out = match u.rows.first() {
                Some(r) => own_hits::tip_line(ds, own_hits::role_col(ds, "category"), *r, ctx),
                None => Vec::new(),
            };
            out.push((w("Count", "Nombre"), own_hits::num_text(ds, None, stat(0), ctx)));
            for (en, fr, k) in [
                ("Maximum", "Maximum", 5),
                ("Third quartile", "Troisième quartile", 4),
                ("Median", "Médiane", 3),
                ("First quartile", "Premier quartile", 2),
                ("Minimum", "Minimum", 1),
                ("Mean", "Moyenne", 6),
            ] {
                out.push((w(en, fr), own_hits::num_text(ds, vc, stat(k), ctx)));
            }
            out.push((
                w("Outliers", "Valeurs aberrantes"),
                own_hits::num_text(ds, None, stat(7), ctx),
            ));
            out
        }
        _ => u
            .rows
            .first()
            .map(|r| own_hits::row_tip(ds, *r, ctx))
            .unwrap_or_default(),
    }
}

#[cfg(test)]
#[path = "bvis_tests.rs"]
mod tests;
