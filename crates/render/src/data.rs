//! The render context (theme, culture, model formats), each visual's format and its data set:
//! the snapshot's evaluated rows, or deterministic made-up values when there are none. Ported
//! from the live viewer (`crates/view/src/page.html`: `renderCtx`, `visFormat`, `dataset`,
//! `chartData`).

use std::collections::HashMap;

use serde_json::Value;

use crate::canvas::Rgb;
use crate::fmt::{ColFmt, Locale, fmt_cell, fmt_parts, iso_serial, month_name};
use crate::fonts::FontBook;

pub const PT: f32 = 4.0 / 3.0;
/// The viewer's placeholder phase range (kept as written there, not 2π).
#[allow(clippy::approx_constant)]
const PHASE_SPAN: f64 = 6.28;

/// The data colours of the snapshot's report theme `t` (`report.theme`), in series order: the
/// theme's own, else the classic palette for a report without a theme collection (as Desktop
/// draws it), else the default theme's.
pub(crate) fn palette(t: &Value) -> Vec<Rgb> {
    let colors: Vec<Rgb> = arr(get(t, "data_colors"))
        .iter()
        .take(64)
        .filter_map(Rgb::from_value)
        .collect();
    if !colors.is_empty() {
        colors
    } else if !t.is_object() {
        CLASSIC_COLORS.to_vec()
    } else {
        DEFAULT_COLORS.to_vec()
    }
}

pub const DEFAULT_COLORS: [Rgb; 20] = [
    Rgb(0x11, 0x8d, 0xff),
    Rgb(0x12, 0x23, 0x9e),
    Rgb(0xe6, 0x6c, 0x37),
    Rgb(0x6b, 0x00, 0x7b),
    Rgb(0xe0, 0x44, 0xa7),
    Rgb(0x74, 0x4e, 0xc2),
    Rgb(0xd9, 0xb3, 0x00),
    Rgb(0xd6, 0x45, 0x50),
    Rgb(0x19, 0x72, 0x78),
    Rgb(0x1a, 0xab, 0x40),
    Rgb(0x15, 0xc6, 0xf4),
    Rgb(0x40, 0x92, 0xff),
    Rgb(0xff, 0xa0, 0x58),
    Rgb(0xbe, 0x5d, 0xc9),
    Rgb(0xf4, 0x72, 0xd0),
    Rgb(0xb5, 0xa1, 0xff),
    Rgb(0xc4, 0xa2, 0x00),
    Rgb(0xff, 0x80, 0x80),
    Rgb(0x00, 0xdb, 0xbc),
    Rgb(0x5b, 0xd6, 0x67),
];
/// Title of a report without a theme (Desktop 2.158: light grey, regular, 9 pt = 12 px).
pub const CLASSIC_TITLE: Rgb = Rgb(0xc8, 0xc8, 0xc8);
pub const CLASSIC_TITLE_PX: f32 = 12.0;
/// Label size of a report without a theme: Desktop 2.158 draws axis labels, legends, table and
/// matrix cells and data labels at 8 pt (10.67 px; its DOM's computed styles).
pub const CLASSIC_LABEL_PT: f64 = 8.0;
/// Desktop's palette for reports without a theme (Desktop 2.158).
pub const CLASSIC_COLORS: [Rgb; 20] = [
    Rgb(0x01, 0xb8, 0xaa),
    Rgb(0x37, 0x46, 0x49),
    Rgb(0xfd, 0x62, 0x5e),
    Rgb(0xf2, 0xc8, 0x0f),
    Rgb(0x5f, 0x6b, 0x6d),
    Rgb(0x8a, 0xd4, 0xeb),
    Rgb(0xfe, 0x96, 0x66),
    Rgb(0xa6, 0x69, 0x99),
    Rgb(0x35, 0x99, 0xb8),
    Rgb(0xdf, 0xbf, 0xbf),
    // Desktop 2.158 goes on with lighter variants (measured on 18-slice pies and treemaps)
    Rgb(0x4a, 0xc5, 0xbb),
    Rgb(0x5f, 0x6b, 0x6d),
    Rgb(0xfb, 0x82, 0x81),
    Rgb(0xf4, 0xd2, 0x5a),
    Rgb(0x7f, 0x89, 0x8a),
    Rgb(0xa4, 0xdd, 0xee),
    Rgb(0xfd, 0xab, 0x89),
    Rgb(0xb6, 0x87, 0xac),
    Rgb(0x28, 0x73, 0x8a),
    Rgb(0xa7, 0x8f, 0x8f),
];
/// Classic "good" colour (Desktop 2.158, report without a theme).
pub const CLASSIC_GOOD: Rgb = Rgb(0x3b, 0xb4, 0x4a);
pub const GOOD: Rgb = Rgb(0x1a, 0xab, 0x40);
pub const BAD: Rgb = Rgb(0xd6, 0x45, 0x54);

// --- JSON helpers --------------------------------------------------------------------------

pub fn get<'a>(v: &'a Value, k: &str) -> &'a Value {
    v.get(k).unwrap_or(&Value::Null)
}
pub fn arr(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
/// Text of a string, finite number or boolean (JS `str`).
pub fn st(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n
            .as_f64()
            .filter(|f| f.is_finite())
            .map(|f| {
                if f.fract() == 0.0 && f.abs() < 1e15 {
                    format!("{}", f as i64)
                } else {
                    format!("{f}")
                }
            })
            .unwrap_or_default(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}
pub fn sk<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}
pub fn num(v: &Value, d: f64) -> f64 {
    v.as_f64().filter(|f| f.is_finite()).unwrap_or(d)
}
pub fn nk(v: &Value, k: &str, d: f64) -> f64 {
    num(get(v, k), d)
}
pub fn fin(v: &Value) -> Option<f64> {
    v.as_f64().filter(|f| f.is_finite())
}
pub fn clip(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        let mut t: String = s.chars().take(n.saturating_sub(1)).collect();
        t.push('…');
        t
    } else {
        s.to_string()
    }
}
pub fn clampf(v: f32, a: f32, b: f32) -> f32 {
    if v.is_nan() { a } else { v.max(a).min(b) }
}

/// Case-insensitive value key (Power BI compares text without case).
pub fn sel_key(x: &Value) -> String {
    match x {
        Value::String(s) => format!("s:{}", s.to_lowercase()),
        _ => key_of(x),
    }
}
pub fn key_of(x: &Value) -> String {
    match x {
        Value::Null => "null".into(),
        Value::String(s) => format!("string:{s}"),
        Value::Number(n) => format!("number:{}", n.as_f64().unwrap_or(0.0)),
        Value::Bool(b) => format!("boolean:{b}"),
        other => format!("object:{}", clip(&other.to_string(), 200)),
    }
}
/// Order of raw values: blanks first, numbers numerically, text by its characters.
pub fn cmp_raw(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a.is_null(), b.is_null()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        _ => {}
    }
    if let (Some(x), Some(y)) = (a.as_f64(), b.as_f64()) {
        return x.partial_cmp(&y).unwrap_or(Ordering::Equal);
    }
    let (x, y) = (st(a), st(b));
    x.to_lowercase().cmp(&y.to_lowercase()).then(x.cmp(&y))
}

// --- context -------------------------------------------------------------------------------

/// Theme, culture and model formats, resolved once per render.
pub struct Ctx<'a> {
    pub fonts: &'a FontBook,
    pub colors: Vec<Rgb>,
    pub bg: Rgb,
    pub fg: Rgb,
    pub fg2: Rgb,
    pub muted: Rgb,
    pub faint: Rgb,
    pub grid: Rgb,
    pub track: Rgb,
    pub band: Rgb,
    pub accent: Rgb,
    pub font: String,
    /// Label font size (px).
    pub px: f32,
    /// Chart text size (px): axis labels, legends, data labels. Desktop 2.158 draws them at 9 pt
    /// whatever the theme's `textClasses.label` or `visualStyles` font sizes say (observed with
    /// 10.5 pt in both, docs/render.md); 8 pt in the classic look. A visual's own font sizes win.
    pub chart_px: f32,
    /// Text box font size (px).
    pub text_px: f32,
    pub loc: Locale,
    fmts: HashMap<(String, String), String>,
    fmts_by_name: HashMap<String, String>,
    types: HashMap<(String, String), String>,
    /// Column format strings (`Long Date`, `#,0.00`): a column field and its implicit measures
    /// (other than the counts) display with them, as in Desktop.
    col_fmts: HashMap<(String, String), String>,
    /// The report's pages, for page navigators.
    pub pages: Vec<NavPage>,
    /// The report's bookmarks in their order (the snapshot's `bookmarks`), for bookmark
    /// navigators.
    pub bookmarks: Vec<NavBookmark>,
    /// The bookmark the reader applied last (`interaction.bookmark`), empty when none.
    pub bookmark: String,
    /// No theme: Desktop's classic look (light grey regular titles).
    pub classic: bool,
    /// The snapshot's UI phrases (`ui.strings`: blank, total, and, all…), in Desktop's UI
    /// language (M10.2).
    strings: Value,
    /// The language of those phrases (`ui.strings_language`, e.g. `fr`), empty when absent.
    pub ui_lang: String,
    /// The hit map being recorded (`RenderOptions::hits`) and the visual being drawn.
    pub ix: crate::hits::Ix,
    /// Where the reader's pointer is (`RenderOptions::pointer`): the button, shape or navigator
    /// button drawn in its hover or press state.
    pub pointer: Option<crate::states::Pointer>,
}

/// A page as a page navigator lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct NavPage {
    pub name: String,
    pub label: String,
    /// Hidden in view mode.
    pub hidden: bool,
    /// A tooltip page.
    pub tooltip: bool,
}

/// A bookmark as a bookmark navigator lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct NavBookmark {
    pub name: String,
    pub label: String,
    /// Its group's display name and name, empty when it has none.
    pub group: String,
    pub group_name: String,
}

/// The display name of a page or bookmark of the snapshot, else its name.
fn label_of(x: &Value) -> &str {
    if sk(x, "display_name").is_empty() {
        sk(x, "name")
    } else {
        sk(x, "display_name")
    }
}

/// A font family list, sanitized like the viewer's `safeFont` (names only, at most 4), with
/// Segoe UI behind.
pub fn safe_font(v: &str) -> String {
    let mut names: Vec<String> = v
        .split(',')
        .map(|s| {
            s.chars()
                .filter(|c| c.is_ascii_alphanumeric() || " _-".contains(*c))
                .collect::<String>()
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .take(4)
        .collect();
    names.push("Segoe UI".into());
    names.join(", ")
}

impl<'a> Ctx<'a> {
    /// `culture`: the formatting culture (numbers, dates, automatic titles); `None` is the
    /// snapshot's model culture (Desktop formats in it, whatever the translation shown), en-US
    /// when absent. The single place where a culture enters the renderer (localisation: M10).
    pub fn new(snap: &Value, fonts: &'a FontBook, culture: Option<&str>) -> Ctx<'a> {
        let report = get(snap, "report");
        let t = get(report, "theme");
        let model = get(snap, "model");
        let colors = palette(t);
        // a report without a theme collection: Desktop draws it with the classic palette; a
        // custom theme without a base theme (`classic`) keeps the classic look under its colours
        let classic = !t.is_object() || get(t, "classic").as_bool() == Some(true);
        let bg = Rgb::from_value(get(t, "background")).unwrap_or(Rgb::WHITE);
        let fg = Rgb::from_value(get(t, "foreground")).unwrap_or(Rgb(0x25, 0x24, 0x23));
        let default = fg == Rgb(0x25, 0x24, 0x23) && bg == Rgb::WHITE;
        let mut fmts = HashMap::new();
        let mut fmts_by_name = HashMap::new();
        let mut types = HashMap::new();
        let mut col_fmts = HashMap::new();
        let mut n = 0usize;
        'outer: for tb in arr(get(model, "tables")) {
            let tn = sk(tb, "name").to_string();
            for m in arr(get(tb, "measures")) {
                n += 1;
                if n > 60_000 {
                    break 'outer;
                }
                let (mn, f) = (sk(m, "name"), sk(m, "format_string"));
                if mn.is_empty() || f.is_empty() {
                    continue;
                }
                fmts.insert((tn.clone(), mn.to_string()), f.to_string());
                fmts_by_name
                    .entry(mn.to_string())
                    .or_insert_with(|| f.to_string());
            }
            for c in arr(get(tb, "columns")) {
                n += 1;
                if n > 60_000 {
                    break 'outer;
                }
                types.insert(
                    (tn.clone(), sk(c, "name").to_string()),
                    sk(c, "data_type").to_string(),
                );
                if !sk(c, "format_string").is_empty() {
                    col_fmts.insert(
                        (tn.clone(), sk(c, "name").to_string()),
                        clip(sk(c, "format_string"), 200),
                    );
                }
            }
        }
        let pages = arr(get(report, "pages"))
            .iter()
            .filter(|p| p.is_object())
            .take(100)
            .map(|p| NavPage {
                name: clip(sk(p, "name"), 200),
                label: clip(label_of(p), 60),
                hidden: get(p, "hidden") == &Value::Bool(true),
                tooltip: sk(p, "page_type") == "tooltip",
            })
            .collect();
        let bookmarks = arr(get(get(snap, "bookmarks"), "bookmarks"))
            .iter()
            .filter(|b| !sk(b, "name").is_empty())
            .take(200)
            .map(|b| NavBookmark {
                name: clip(sk(b, "name"), 200),
                label: clip(label_of(b), 60),
                group: clip(sk(b, "group"), 200),
                group_name: clip(sk(b, "group_name"), 200),
            })
            .collect();
        let bookmark = clip(sk(get(snap, "interaction"), "bookmark"), 200);
        let accent = Rgb::from_value(get(t, "table_accent"))
            .unwrap_or(colors.first().copied().unwrap_or(DEFAULT_COLORS[0]));
        Ctx {
            fonts,
            ix: crate::hits::Ix::default(),
            pointer: None,
            bg,
            fg,
            classic,
            strings: get(get(snap, "ui"), "strings").clone(),
            ui_lang: sk(get(snap, "ui"), "strings_language").chars().take(20).collect(),
            fg2: if default {
                Rgb(0x60, 0x5e, 0x5c)
            } else {
                fg.mix(bg, 0.2)
            },
            muted: if classic {
                Rgb(0x77, 0x77, 0x77)
            } else if default {
                Rgb(0x60, 0x5e, 0x5c)
            } else {
                fg.mix(bg, 0.3)
            },
            faint: fg.mix(bg, 0.65),
            grid: if classic {
                Rgb(0xea, 0xea, 0xea)
            } else {
                fg.mix(bg, 0.92)
            },
            track: fg.mix(bg, 0.88),
            band: fg.mix(bg, 0.93),
            accent,
            colors,
            font: safe_font(sk(t, "font_family")),
            px: clampf(
                nk(t, "font_size", if classic { CLASSIC_LABEL_PT } else { 9.0 }) as f32,
                6.0,
                24.0,
            ) * PT,
            text_px: clampf(nk(t, "font_size", 10.0) as f32, 6.0, 40.0) * PT,
            chart_px: if classic { CLASSIC_LABEL_PT as f32 } else { 9.0 } * PT,
            loc: Locale::new(
                culture
                    .or_else(|| get(model, "culture").as_str())
                    .unwrap_or("en-US"),
            ),
            fmts,
            fmts_by_name,
            types,
            col_fmts,
            pages,
            bookmarks,
            bookmark,
        }
    }

    /// A UI phrase from the snapshot (`ui.strings`), else the English one.
    pub fn t(&self, key: &str, default: &str) -> String {
        match self.strings.get(key).and_then(Value::as_str) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => default.to_string(),
        }
    }

    /// "(Blank)" in the UI language.
    pub fn blank(&self) -> String {
        self.t("blank", BLANK)
    }

    /// An aggregation's name for a field ("Sum of Amount"), from `ui.strings.aggregations`.
    pub fn agg_name(&self, func: &str, field: &str) -> Option<String> {
        let key = func.to_lowercase();
        if let Some(t) = self
            .strings
            .get("aggregations")
            .and_then(|a| a.get(key.as_str()))
            .and_then(Value::as_str)
        {
            return Some(t.replace("{0}", field));
        }
        agg_prefix(func).map(|p| format!("{p}{field}"))
    }

    /// The "good" sentiment colour (KPI met, waterfall increase).
    pub fn good(&self) -> Rgb {
        if self.classic { CLASSIC_GOOD } else { GOOD }
    }

    pub fn color(&self, i: usize) -> Rgb {
        self.colors
            .get(i % self.colors.len().max(1))
            .copied()
            .unwrap_or(DEFAULT_COLORS[0])
    }
}

// --- columns ---------------------------------------------------------------------------------

/// The role class of a data role name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rc {
    Cat,
    Col,
    Leg,
    Tip,
    Y2,
    Goal,
    Min,
    Max,
    Val,
    None,
}

pub fn role_class(role: &str) -> Rc {
    let r: String = role
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_')
        .collect();
    match r.as_str() {
        "category" | "axis" | "row" | "rows" | "group" | "details" | "location" | "trendline" | "xaxis" => {
            Rc::Cat
        }
        "columns" | "column" => Rc::Col,
        "legend" | "series" => Rc::Leg,
        "tooltips" | "tooltip" => Rc::Tip,
        "y2" | "linevalues" | "secondaryy" => Rc::Y2,
        "goal" | "goals" | "target" | "targetvalue" => Rc::Goal,
        "minvalue" | "min" => Rc::Min,
        "maxvalue" | "max" => Rc::Max,
        "y" | "value" | "values" | "size" | "indicator" | "data" | "measure" | "measures" | "field"
        | "fields" => Rc::Val,
        _ => Rc::None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Col {
    pub name: String,
    pub role: String,
    pub rc: Rc,
    pub kind: String,
    pub typ: String,
    pub fmt: String,
    pub func: String,
    /// A sparkline column's look (its cells are drawn as small charts).
    pub spark: Option<SparkStyle>,
}

impl Col {
    pub fn cf(&self) -> ColFmt {
        ColFmt {
            fmt: self.fmt.clone(),
            kind: self.kind.clone(),
            data_type: self.typ.clone(),
        }
    }
    pub fn is_num_type(&self) -> bool {
        let t = self.typ.to_lowercase();
        ["int", "double", "decimal", "currency", "number", "float"]
            .iter()
            .any(|k| t.contains(k))
            || ((self.kind == "measure" || self.kind == "aggregation")
                && !["string", "text", "date", "time", "bool"]
                    .iter()
                    .any(|k| t.contains(k)))
    }
    pub fn is_date(&self) -> bool {
        let t = self.typ.to_lowercase();
        t.contains("date") || t.contains("time")
    }
}

pub fn is_num_col(c: &Col, rows: &[Vec<Value>], i: usize) -> bool {
    if !c.typ.is_empty() || c.kind == "measure" || c.kind == "aggregation" {
        return c.is_num_type();
    }
    for r in rows.iter().take(20) {
        match r.get(i) {
            Some(Value::Null) | None => continue,
            Some(x) => return x.is_number(),
        }
    }
    false
}

pub(crate) fn field_label(f: &Value) -> String {
    // the snapshot's own label (the visual's rename or Desktop's caption, translated), then
    // the translated caption, then the model name
    if !sk(f, "label").is_empty() {
        return sk(f, "label").to_string();
    }
    let name = if sk(f, "caption").is_empty() {
        sk(f, "name")
    } else {
        sk(f, "caption")
    };
    let func = sk(f, "function");
    let base = if name.is_empty() {
        if sk(f, "text").is_empty() {
            "?"
        } else {
            sk(f, "text")
        }
    } else {
        name
    };
    if !func.is_empty() && !name.is_empty() {
        format!("{func}({name})")
    } else {
        base.to_string()
    }
}

/// The data column name the engine gives an aggregation field (`crates/engine` preview).
fn engine_agg_name(f: &Value) -> String {
    let func = sk(f, "function");
    let label = match func {
        "Avg" => "Average",
        "Count" => "Count (Distinct)",
        "CountNonNull" => "Count",
        "StandardDeviation" => "Standard deviation",
        other => other,
    };
    format!("{label} of {}", sk(f, "name"))
}

fn agg_prefix(func: &str) -> Option<&'static str> {
    Some(match func.to_lowercase().as_str() {
        "sum" => "Sum of ",
        "avg" | "average" => "Average of ",
        "min" => "Min of ",
        "max" => "Max of ",
        "count" | "countnonnull" => "Count of ",
        "distinctcount" => "Count of distinct ",
        _ => return None,
    })
}

fn field_col(f: &Value, ctx: &Ctx<'_>) -> Col {
    let (kind, t, n, func) = (sk(f, "kind"), sk(f, "table"), sk(f, "name"), sk(f, "function"));
    let typ = match kind {
        "measure" => "double".to_string(),
        "aggregation" => {
            if func.to_lowercase().contains("count") {
                "int64".into()
            } else {
                "double".into()
            }
        }
        _ => ctx
            .types
            .get(&(t.to_string(), n.to_string()))
            .cloned()
            .unwrap_or_else(|| {
                if kind == "column" {
                    "string".into()
                } else {
                    String::new()
                }
            }),
    };
    let fmt = if kind == "measure" {
        ctx.fmts
            .get(&(t.to_string(), n.to_string()))
            .or_else(|| ctx.fmts_by_name.get(n))
            .cloned()
            .unwrap_or_default()
    } else if kind == "column" || (kind == "aggregation" && !func.to_lowercase().contains("count")) {
        // a column and its implicit Sum / Average / Min / Max… keep the column's format string
        ctx.col_fmts
            .get(&(t.to_string(), n.to_string()))
            .cloned()
            .unwrap_or_default()
    } else {
        String::new()
    };
    let shown = if sk(f, "caption").is_empty() {
        n
    } else {
        sk(f, "caption")
    };
    let name = match ctx.agg_name(func, shown) {
        Some(a) if !n.is_empty() && sk(f, "label").is_empty() => a,
        _ => field_label(f),
    };
    Col {
        name: clip(&name, 200),
        role: sk(f, "role").to_string(),
        rc: role_class(sk(f, "role")),
        kind: kind.to_string(),
        typ,
        fmt,
        func: func.to_string(),
        spark: None,
    }
}

fn data_col(c: &Value, fields: &[Value], ctx: &Ctx<'_>) -> Col {
    let (name, role) = (sk(c, "name"), sk(c, "role"));
    let f = fields
        .iter()
        .find(|x| sk(x, "name") == name && (role.is_empty() || sk(x, "role") == role))
        .or_else(|| {
            fields
                .iter()
                .find(|x| sk(x, "name") == name || field_label(x) == name || sk(x, "text") == name)
        })
        .or_else(|| {
            // an aggregation's data column carries the engine's English name ("Count (Distinct)
            // of Amount")
            fields
                .iter()
                .find(|x| sk(x, "kind") == "aggregation" && engine_agg_name(x) == name)
        });
    let base = match f {
        Some(f) => field_col(f, ctx),
        None => Col {
            name: String::new(),
            role: String::new(),
            rc: Rc::None,
            kind: String::new(),
            typ: String::new(),
            fmt: ctx.fmts_by_name.get(name).cloned().unwrap_or_default(),
            func: String::new(),
            spark: None,
        },
    };
    let rc = match role_class(role) {
        Rc::None => base.rc,
        r => r,
    };
    // the field's label in the UI language ("Count of Amount" for a distinct count, as Desktop
    // 2.158 names both counts; the data column keeps the engine's own name)
    let label = f.map(|f| sk(f, "label")).filter(|l| !l.is_empty());
    Col {
        name: if name.is_empty() {
            if base.name.is_empty() {
                "?".into()
            } else {
                base.name.clone()
            }
        } else if !sk(c, "caption").is_empty() {
            // a translated data column (M10.4): shown by its caption
            clip(sk(c, "caption"), 200)
        } else if let Some(l) = label {
            clip(l, 200)
        } else {
            clip(name, 200)
        },
        role: role.to_string(),
        rc,
        kind: if sk(c, "kind").is_empty() {
            base.kind.clone()
        } else {
            sk(c, "kind").to_string()
        },
        typ: if sk(c, "data_type").is_empty() {
            base.typ.clone()
        } else {
            sk(c, "data_type").to_string()
        },
        // a data column's own format wins (a "Show value as" percentage: `0.00%`)
        fmt: if sk(c, "format_string").is_empty() {
            base.fmt
        } else {
            clip(sk(c, "format_string"), 200)
        },
        func: base.func,
        spark: (sk(c, "kind") == "sparkline").then(|| SparkStyle::of(get(c, "sparkline"))),
    }
}

/// A sparkline column's look (`data.columns[].sparkline`, from the visual's
/// `objects.sparklines`): Desktop's defaults where it sets nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct SparkStyle {
    /// A column chart (else a line).
    pub column: bool,
    /// `None`: the theme's first data colour.
    pub color: Option<Rgb>,
    pub width: f32,
    /// The markers shown, as Power BI Desktop 2.159.1231.0 writes `markers`: bits 1 every point,
    /// 2 highest, 4 lowest, 8 first, 16 last, 32 negative, 128 isolated (the default, when
    /// `markers` is not written).
    pub markers: u32,
    pub marker_color: Option<Rgb>,
    /// `circle`, `square`, `diamond`, `triangle`, `x`, `shortDash`, `longDash`, `plus`.
    pub marker_shape: String,
    pub marker_size: f32,
    /// The X range of every cell of the column (the rows', totals' and subtotals'), when every
    /// point's X is a number or a date: a line places its points on it, as Desktop does.
    pub domain: Option<(f64, f64)>,
}

impl SparkStyle {
    pub fn of(v: &Value) -> SparkStyle {
        let shape = sk(v, "marker_shape");
        SparkStyle {
            column: sk(v, "chart_type").eq_ignore_ascii_case("column"),
            color: Rgb::from_value(get(v, "color")),
            width: clampf(nk(v, "width", 1.5) as f32, 0.5, 10.0),
            markers: get(v, "markers")
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .map_or(128, |n| (n as u32) & 0xFF),
            marker_color: Rgb::from_value(get(v, "marker_color")),
            marker_shape: if shape.is_empty() || shape.len() > 20 {
                "circle".into()
            } else {
                shape.to_string()
            },
            marker_size: clampf(nk(v, "marker_size", 4.0) as f32, 1.0, 50.0),
            domain: None,
        }
    }
}

/// The X of a sparkline point `[x, y]` as a number: a number, or a date's OLE serial.
pub fn spark_x(p: &Value) -> Option<f64> {
    match p.get(0)? {
        Value::Number(n) => n.as_f64().filter(|x| x.is_finite()),
        Value::String(s) => iso_serial(s),
        _ => None,
    }
}

/// The X range over every cell of sparkline column `i` (rows, totals, subtotal levels); `None`
/// when a point's X is neither a number nor a date (a text axis) or there is no point.
fn spark_domain(ds: &Dataset, i: usize) -> Option<(f64, f64)> {
    let cells = ds
        .rows
        .iter()
        .chain(ds.totals.iter())
        .chain(ds.subtotals.iter().flat_map(|l| l.data.iter()))
        .filter_map(|r| r.get(i));
    let mut range: Option<(f64, f64)> = None;
    for c in cells {
        for p in get(c, "sparkline").as_array().into_iter().flatten().take(100) {
            let x = spark_x(p)?;
            range = Some(range.map_or((x, x), |(lo, hi)| (lo.min(x), hi.max(x))));
        }
    }
    range
}

/// The points of a sparkline cell (`{"sparkline": [[x, y], …]}`): the y of each, in order
/// (blank: `None`), at most 52 drawn as the engine sends them (a few more tolerated).
pub fn spark_points(x: &Value) -> Option<Vec<Option<f64>>> {
    let pts = get(x, "sparkline").as_array()?;
    Some(pts.iter().take(100).map(|p| p.get(1).and_then(fin)).collect())
}

/// Conditional formatting of one cell.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CellStyle {
    pub bg: Option<Rgb>,
    pub fg: Option<Rgb>,
    pub fill: Option<Rgb>,
    pub bar: Option<(f32, f32, Rgb)>,
    /// The data bar's axis line: its place in the cell (0–1) and colour.
    pub bar_axis: Option<(f32, Rgb)>,
    /// Data bars' Show bar only: the value's text is not drawn.
    pub bar_only: bool,
    pub icon: Option<(String, String, String)>,
    pub url: bool,
    /// A chart's data label colour for this point.
    pub label: Option<Rgb>,
    /// A card's category label colour.
    pub cat: Option<Rgb>,
}

#[derive(Debug, Clone, Default)]
pub struct Dataset {
    pub fake: bool,
    pub empty: bool,
    pub cols: Vec<Col>,
    pub rows: Vec<Vec<Value>>,
    pub truncated: bool,
    pub styles: HashMap<(usize, usize), CellStyle>,
    /// Filters were ignored (the reason).
    pub note: Option<String>,
    /// Why the values are made up.
    pub reason: Option<String>,
    /// The engine's total row (one cell per column, evaluated in the grand-total filter
    /// context); `None`: the renderer adds the additive columns up.
    pub totals: Option<Vec<Value>>,
    /// Why conditional formatting that reads measures was not applied.
    pub unformatted: Option<String>,
    /// A matrix's subtotal levels from the engine (`data.subtotals`), empty without them.
    pub subtotals: Vec<Level>,
    /// Displayed levels of a matrix's row hierarchy (`data.row_levels`); `None`: every level.
    /// (One level of column groups is drawn.)
    pub row_levels: Option<usize>,
    /// The matrix's row and column expansion (`data.row_expansion` / `column_expansion`).
    pub row_exp: Option<Expansion>,
    pub col_exp: Option<Expansion>,
    /// Per-level subtotal switches (`data.row_level_totals` / `column_level_totals`), by level
    /// (the top field's entry is the grand total); empty when per-level settings are off.
    pub row_level_totals: Vec<LevelTotal>,
    pub col_level_totals: Vec<LevelTotal>,
    /// Rows of the whole result (`data.total_rows`), more than `rows` when truncated.
    pub total_rows: Option<usize>,
    /// `data.highlights`: the highlighted part of each row (a selection in another visual).
    pub hl: Option<Vec<Vec<Value>>>,
    /// `data.error_bars`: each error bar's series column and its bounds per row.
    pub errs: Vec<ErrRows>,
}

/// One error bar's bounds (`data.error_bars` entry): per row, the upper and lower value.
#[derive(Debug, Clone, Default)]
pub struct ErrRows {
    pub column: usize,
    pub upper: Vec<Option<f64>>,
    pub lower: Vec<Option<f64>>,
}

/// `data.error_bars`, at most 12 entries of 5000 rows.
fn err_rows(d: &Value) -> Vec<ErrRows> {
    let nums = |x: &Value| -> Vec<Option<f64>> {
        arr(x)
            .iter()
            .take(5000)
            .map(|n| fin(n).map(|n| n.clamp(-1e15, 1e15)))
            .collect()
    };
    arr(get(d, "error_bars"))
        .iter()
        .take(12)
        .filter_map(|e| {
            Some(ErrRows {
                column: usize::try_from(get(e, "column").as_u64()?).ok()?,
                upper: nums(get(e, "upper")),
                lower: nums(get(e, "lower")),
            })
        })
        .collect()
}

/// One subtotal level of a matrix: the leading `rows` row fields and `cols` column fields kept,
/// one row per member combination (a cell per column, null under the dropped fields).
#[derive(Debug, Clone, Default)]
pub struct Level {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<Vec<Value>>,
}

/// `data.subtotals` (at most 64 levels of 5000 rows).
fn levels(d: &Value) -> Vec<Level> {
    arr(get(d, "subtotals"))
        .iter()
        .take(64)
        .filter_map(|l| {
            Some(Level {
                rows: usize::try_from(get(l, "rows").as_u64()?).ok()?,
                cols: usize::try_from(get(l, "columns").as_u64()?).ok()?,
                data: arr(get(l, "data"))
                    .iter()
                    .filter_map(Value::as_array)
                    .take(5000)
                    .cloned()
                    .collect(),
            })
        })
        .collect()
}

/// One hierarchy's expansion (`data.row_expansion`): per level whether its members are
/// expanded, and the member paths that are the exceptions (keys of `key_of`, joined by U+0001).
#[derive(Debug, Clone, Default)]
pub struct Expansion {
    pub open: Vec<bool>,
    pub toggled: std::collections::HashSet<String>,
}

impl Expansion {
    /// Whether the member at `path` (top value first) of a `n`-level hierarchy shows its
    /// children.
    pub fn expanded(&self, path: &[Value], n: usize) -> bool {
        let d = path.len();
        if d == 0 || d >= n {
            return false;
        }
        let key = path.iter().map(key_of).collect::<Vec<_>>().join("\u{1}");
        self.open.get(d - 1).copied().unwrap_or(false) ^ self.toggled.contains(&key)
    }
}

fn expansion(v: &Value) -> Option<Expansion> {
    let o = v.as_object()?;
    Some(Expansion {
        open: arr(o.get("open").unwrap_or(&Value::Null))
            .iter()
            .take(8)
            .map(|x| x.as_bool() == Some(true))
            .collect(),
        toggled: arr(o.get("toggled").unwrap_or(&Value::Null))
            .iter()
            .take(2000)
            .filter_map(Value::as_array)
            .map(|p| p.iter().take(8).map(key_of).collect::<Vec<_>>().join("\u{1}"))
            .collect(),
    })
}

/// One level's subtotal switch and label (`data.row_level_totals`).
#[derive(Debug, Clone, Default)]
pub struct LevelTotal {
    pub show: bool,
    pub label: Option<String>,
}

fn level_totals(v: &Value) -> Vec<LevelTotal> {
    arr(v)
        .iter()
        .take(8)
        .map(|l| LevelTotal {
            show: get(l, "show").as_bool() != Some(false),
            label: get(l, "label").as_str().map(|s| clip(s, 200)),
        })
        .collect()
}

fn web_url(v: &Value) -> bool {
    let t = st(v);
    !t.is_empty()
        && t.len() <= 2048
        && !t.chars().any(char::is_whitespace)
        && (t.to_ascii_lowercase().starts_with("http://") || t.to_ascii_lowercase().starts_with("https://"))
}

pub fn dataset(v: &Value, kind: &str, ctx: &Ctx<'_>) -> Dataset {
    let fields: Vec<Value> = arr(get(v, "fields"))
        .iter()
        .filter(|f| f.is_object())
        .take(40)
        .cloned()
        .collect();
    let d = get(v, "data");
    if d.is_object() && sk(d, "status") == "ok" {
        let cols: Vec<Col> = arr(get(d, "columns"))
            .iter()
            .take(40)
            .map(|c| data_col(c, &fields, ctx))
            .collect();
        if !cols.is_empty() {
            let rows: Vec<Vec<Value>> = arr(get(d, "rows"))
                .iter()
                .filter_map(Value::as_array)
                .take(5000)
                .cloned()
                .collect();
            let mut styles = HashMap::new();
            for s in arr(get(d, "cell_styles")).iter().take(50_000) {
                let (Some(r), Some(c)) = (get(s, "r").as_u64(), get(s, "c").as_u64()) else {
                    continue;
                };
                let bar = get(s, "bar");
                let bc = Rgb::from_value(get(bar, "color"));
                let ic = get(s, "icon");
                let iname = sk(ic, "name");
                let icon = (!iname.is_empty()
                    && iname.len() <= 100
                    && iname
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
                .then(|| {
                    (
                        iname.to_string(),
                        sk(ic, "position").to_string(),
                        sk(ic, "align").to_string(),
                    )
                });
                styles.insert(
                    (r as usize, c as usize),
                    CellStyle {
                        bg: Rgb::from_value(get(s, "bg")),
                        fg: Rgb::from_value(get(s, "fg")),
                        fill: Rgb::from_value(get(s, "fill")),
                        bar: bc.map(|c| {
                            (
                                clampf(nk(bar, "from", 0.0) as f32, 0.0, 1.0),
                                clampf(nk(bar, "to", 0.0) as f32, 0.0, 1.0),
                                c,
                            )
                        }),
                        bar_axis: Rgb::from_value(get(bar, "axis_color"))
                            .filter(|_| bc.is_some())
                            .map(|c| (clampf(nk(bar, "axis", 0.0) as f32, 0.0, 1.0), c)),
                        bar_only: bc.is_some() && get(bar, "only") == &Value::Bool(true),
                        icon,
                        url: web_url(get(s, "url")),
                        label: Rgb::from_value(get(s, "label")),
                        cat: Rgb::from_value(get(s, "cat")),
                    },
                );
            }
            let note = (get(d, "filters_ignored") == &Value::Bool(true)).then(|| {
                let r = sk(d, "filters_ignored_reason");
                clip(if r.is_empty() { "filters not applied" } else { r }, 600)
            });
            let mut ds = Dataset {
                fake: false,
                empty: false,
                cols,
                rows,
                truncated: get(d, "truncated") == &Value::Bool(true),
                styles,
                note,
                reason: None,
                totals: get(d, "totals").as_array().cloned(),
                unformatted: Some(sk(d, "formatting_ignored_reason"))
                    .filter(|r| !r.is_empty())
                    .map(|r| clip(r, 600)),
                subtotals: levels(d),
                row_levels: get(d, "row_levels")
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok()),
                row_exp: expansion(get(d, "row_expansion")),
                col_exp: expansion(get(d, "column_expansion")),
                row_level_totals: level_totals(get(d, "row_level_totals")),
                col_level_totals: level_totals(get(d, "column_level_totals")),
                total_rows: get(d, "total_rows")
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .map(|n| n.min(1_000_000)),
                errs: err_rows(d),
                hl: get(d, "highlights").as_array().map(|a| {
                    // as many rows as BIcraft's own visuals read
                    a.iter()
                        .take(30_000)
                        .map(|x| {
                            x.as_array()
                                .map(|r| r.iter().take(40).cloned().collect())
                                .unwrap_or_default()
                        })
                        .collect()
                }),
            };
            for i in 0..ds.cols.len() {
                if ds.cols.get(i).is_some_and(|c| c.spark.is_some()) {
                    let d = spark_domain(&ds, i);
                    if let Some(st) = ds.cols.get_mut(i).and_then(|c| c.spark.as_mut()) {
                        st.domain = d;
                    }
                }
            }
            // hidden fields (visual calculations' inputs) stay in the data, not on the visual
            let hidden: Vec<bool> = arr(get(d, "columns"))
                .iter()
                .take(40)
                .map(|c| get(c, "hidden") == &Value::Bool(true))
                .collect();
            if hidden.iter().any(|h| *h) {
                drop_hidden(&mut ds, &hidden);
            }
            return ds;
        }
    }
    if fields.is_empty() {
        return Dataset {
            empty: true,
            ..Dataset::default()
        };
    }
    let why = if d.is_object() {
        let mut parts = vec![if sk(d, "status").is_empty() {
            "?".to_string()
        } else {
            sk(d, "status").to_string()
        }];
        let r = clip(sk(d, "reason"), 400);
        if !r.is_empty() {
            parts.push(r);
        }
        parts.join(": ")
    } else {
        "no data in the snapshot".to_string()
    };
    let mut ds = fake_data(v, kind, &fields, ctx);
    ds.fake = true;
    ds.reason = Some(format!("made-up values: {why}"));
    ds
}

/// `ds` without the columns `hidden` flags (their cells, totals, subtotals, styles, highlights
/// and error bars).
fn drop_hidden(ds: &mut Dataset, hidden: &[bool]) {
    let shown = |i: usize| !hidden.get(i).copied().unwrap_or(false);
    // old column index -> new
    let mut new_of: Vec<Option<usize>> = Vec::with_capacity(ds.cols.len());
    let mut n = 0usize;
    for i in 0..ds.cols.len() {
        if shown(i) {
            new_of.push(Some(n));
            n += 1;
        } else {
            new_of.push(None);
        }
    }
    let keep_row = |r: &[Value]| -> Vec<Value> {
        r.iter()
            .enumerate()
            .filter(|(i, _)| shown(*i))
            .map(|(_, x)| x.clone())
            .collect()
    };
    ds.cols = std::mem::take(&mut ds.cols)
        .into_iter()
        .enumerate()
        .filter(|(i, _)| shown(*i))
        .map(|(_, c)| c)
        .collect();
    for r in &mut ds.rows {
        *r = keep_row(r);
    }
    if let Some(t) = &mut ds.totals {
        *t = keep_row(t);
    }
    for l in &mut ds.subtotals {
        for r in &mut l.data {
            *r = keep_row(r);
        }
    }
    if let Some(h) = &mut ds.hl {
        for r in h.iter_mut() {
            *r = keep_row(r);
        }
    }
    ds.styles = std::mem::take(&mut ds.styles)
        .into_iter()
        .filter_map(|((r, c), st)| new_of.get(c).copied().flatten().map(|c| ((r, c), st)))
        .collect();
    ds.errs = std::mem::take(&mut ds.errs)
        .into_iter()
        .filter_map(|mut e| {
            e.column = new_of.get(e.column).copied().flatten()?;
            Some(e)
        })
        .collect();
}

// --- placeholder values (the viewer's deterministic PRNG) ------------------------------------

fn hash32(s: &str) -> u32 {
    let mut x: u32 = 2_166_136_261;
    for c in s.encode_utf16() {
        x ^= u32::from(c);
        x = x.wrapping_mul(16_777_619);
    }
    x
}

/// mulberry32, seeded like the viewer.
struct Rng(u32);
impl Rng {
    fn new(seed: &str) -> Rng {
        let h = hash32(seed);
        Rng(if h == 0 { 1 } else { h })
    }
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6D2B_79F5);
        let a = self.0;
        let mut t = (a ^ (a >> 15)).wrapping_mul(1 | a);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
}

fn domain_size(kind: &str, rc: Rc) -> usize {
    if rc == Rc::Leg || rc == Rc::Col {
        return 3;
    }
    match kind {
        "line" | "area" | "combo" | "comboS" | "kpi" => 12,
        "pie" | "donut" => 5,
        "table" => 10,
        "matrix" => 4,
        "slicer" | "slicerList" | "slicerTiles" | "slicerText" => 6,
        "multiRow" => 3,
        "card" | "cardNew" | "gauge" => 1,
        _ => 6,
    }
}

fn fake_domain(c: &Col, n: usize, ctx: &Ctx<'_>) -> Vec<Value> {
    let nm = c.name.to_lowercase();
    let t = c.typ.to_lowercase();
    (0..n)
        .map(|i| {
            if t.contains("date") || t.contains("time") {
                Value::from(format!("2025-{:02}-01", i % 12 + 1))
            } else if ["année", "annee", "year", "jahr", "año", "anno"]
                .iter()
                .any(|k| nm.contains(k))
            {
                Value::from(2026 - n as i64 + i as i64)
            } else if c.is_num_type() {
                Value::from(i as i64 + 1)
            } else if t.contains("bool") {
                Value::from(i % 2 == 0)
            } else if ["mois", "month", "monat", "mes"].iter().any(|k| nm.contains(k)) {
                Value::from(month_name(i, &ctx.loc))
            } else {
                let letter = char::from(b'A' + (i % 26) as u8);
                let suffix = if i >= 26 {
                    (i / 26).to_string()
                } else {
                    String::new()
                };
                Value::from(format!("{} {letter}{suffix}", clip(&c.name, 14)))
            }
        })
        .collect()
}

const ZIP: [&str; 6] = [
    "table",
    "slicer",
    "slicerList",
    "slicerTiles",
    "slicerText",
    "multiRow",
];

fn fake_data(v: &Value, kind: &str, fields: &[Value], ctx: &Ctx<'_>) -> Dataset {
    let seed = if sk(v, "name").is_empty() {
        if sk(v, "type").is_empty() {
            "visual"
        } else {
            sk(v, "type")
        }
    } else {
        sk(v, "name")
    };
    let mut rng = Rng::new(seed);
    let cols: Vec<Col> = fields.iter().map(|f| field_col(f, ctx)).collect();
    let is_n: Vec<bool> = cols.iter().map(Col::is_num_type).collect();
    let dims: Vec<usize> = cols
        .iter()
        .zip(&is_n)
        .map(|(c, n)| if *n { 0 } else { domain_size(kind, c.rc) })
        .collect();
    let domains: Vec<Vec<Value>> = cols
        .iter()
        .zip(&dims)
        .map(|(c, d)| fake_domain(c, *d, ctx))
        .collect();
    const BASES: [f64; 7] = [
        900.0,
        4200.0,
        12500.0,
        86000.0,
        240_000.0,
        1_350_000.0,
        4_700_000.0,
    ];
    let bases: Vec<f64> = cols
        .iter()
        .map(|c| {
            if fmt_parts(&c.fmt).pct {
                0.15 + rng.next() * 0.5
            } else {
                BASES.get((rng.next() * 7.0) as usize).copied().unwrap_or(900.0)
            }
        })
        .collect();
    let phases: Vec<f64> = cols.iter().map(|_| rng.next() * PHASE_SPAN).collect();
    let dim_idx: Vec<usize> = (0..cols.len())
        .filter(|i| !is_n.get(*i).copied().unwrap_or(true))
        .collect();
    let mut combos: Vec<Vec<usize>> = vec![Vec::new()];
    if !dim_idx.is_empty() && ZIP.contains(&kind) {
        let m = dim_idx
            .iter()
            .map(|i| dims.get(*i).copied().unwrap_or(1))
            .max()
            .unwrap_or(1);
        combos = (0..m)
            .map(|r| {
                dim_idx
                    .iter()
                    .map(|i| r % dims.get(*i).copied().unwrap_or(1).max(1))
                    .collect()
            })
            .collect();
    } else {
        for i in &dim_idx {
            let mut next = Vec::new();
            for c in &combos {
                for k in 0..dims.get(*i).copied().unwrap_or(0) {
                    if next.len() >= 240 {
                        break;
                    }
                    let mut c2 = c.clone();
                    c2.push(k);
                    next.push(c2);
                }
            }
            combos = next;
        }
    }
    let rows = combos
        .iter()
        .enumerate()
        .map(|(r, c)| {
            cols.iter()
                .enumerate()
                .map(|(i, col)| {
                    if !is_n.get(i).copied().unwrap_or(false) {
                        let pos = dim_idx.iter().position(|d| *d == i).unwrap_or(0);
                        let k = c.get(pos).copied().unwrap_or(0);
                        return domains
                            .get(i)
                            .and_then(|d| d.get(k))
                            .cloned()
                            .unwrap_or(Value::Null);
                    }
                    let base = bases.get(i).copied().unwrap_or(1.0);
                    let ph = phases.get(i).copied().unwrap_or(0.0);
                    let x = base * (0.62 + 0.25 * (r as f64 * 0.7 + ph).sin() + 0.22 * rng.next());
                    if fmt_parts(&col.fmt).pct {
                        Value::from((x * 1000.0).round() / 1000.0)
                    } else {
                        Value::from(x.round())
                    }
                })
                .collect()
        })
        .collect();
    Dataset {
        cols,
        rows,
        ..Dataset::default()
    }
}

// --- visual format ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct FontSpec {
    pub family: Option<String>,
    pub weight: Option<u16>,
    pub italic: bool,
    pub underline: bool,
}

fn font_of(o: &Value, title: bool) -> FontSpec {
    let fam = sk(o, "font_family");
    let lf = fam.to_lowercase();
    FontSpec {
        family: (!fam.is_empty()).then(|| safe_font(fam)),
        weight: match get(o, "bold") {
            Value::Bool(true) => Some(700),
            Value::Bool(false) if title => Some(400),
            _ if lf.contains("semibold") => Some(600),
            _ if lf.contains("light") => Some(300),
            _ => None,
        },
        italic: get(o, "italic") == &Value::Bool(true),
        underline: get(o, "underline") == &Value::Bool(true),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}
pub fn align_of(s: &str) -> Align {
    match s.to_lowercase().as_str() {
        "center" => Align::Center,
        "right" => Align::Right,
        _ => Align::Left,
    }
}

#[derive(Debug, Clone, Default)]
pub struct Axis {
    pub show: bool,
    pub title: bool,
    pub text: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub log: bool,
    /// `labelDisplayUnits` (0 auto, 1 none, 1000…).
    pub units: Option<f64>,
    pub gridlines: bool,
    pub color: Option<Rgb>,
    /// The axis title's colour (`titleColor`) when the report sets one.
    pub title_color: Option<Rgb>,
    /// Label font size (px) when the report sets one.
    pub px: Option<f32>,
    /// Category axis: gap between bars, 0..0.9 of the category width (`innerPadding` %).
    pub inner_padding: Option<f32>,
    /// Category axis: minimum category width (px) before the chart scrolls.
    pub category_width: Option<f32>,
    /// Category axis: most of the plot the labels may take (`maxMarginFactor` %, 0..1).
    pub max_margin: Option<f32>,
    /// Category axis type (`axisType`): `Some(true)` continuous (`Scalar`), `Some(false)`
    /// categorical, `None` unset (a date category is then continuous, as in Desktop).
    pub continuous: Option<bool>,
    /// Category axis `start` / `end` read as dates (OLE serials, days): an ISO date text (a
    /// `datetime'…'` literal), an OLE serial, or milliseconds since 1970 (above 1e8).
    pub min_date: Option<f64>,
    pub max_date: Option<f64>,
}

/// A date bound of a continuous category axis as an OLE serial.
fn date_bound(v: &Value) -> Option<f64> {
    match v {
        Value::String(s) => crate::fmt::iso_serial(s.trim()),
        _ => fin(v).map(|x| {
            if x.abs() > 1e8 {
                x / 86_400_000.0 + 25_569.0
            } else {
                x
            }
        }),
    }
    .filter(|x| x.is_finite() && (-700_000.0..=3_000_000.0).contains(x))
}

fn axis_of(a: &Value) -> Axis {
    Axis {
        show: get(a, "show") != &Value::Bool(false),
        title: get(a, "title") == &Value::Bool(true),
        text: clip(sk(a, "title_text"), 200),
        min: fin(get(a, "min")),
        max: fin(get(a, "max")),
        log: get(a, "log") == &Value::Bool(true),
        units: fin(get(a, "display_units")).filter(|u| *u > 0.0),
        gridlines: get(a, "gridlines") != &Value::Bool(false),
        color: Rgb::from_value(get(a, "labels_color")),
        title_color: Rgb::from_value(get(a, "title_color")),
        px: fin(get(a, "font_size")).map(|p| clampf(p as f32, 6.0, 40.0) * PT),
        inner_padding: fin(get(a, "inner_padding")).map(|p| clampf(p as f32 / 100.0, 0.0, 0.9)),
        category_width: fin(get(a, "category_width")).map(|p| clampf(p as f32, 1.0, 500.0)),
        max_margin: fin(get(a, "max_margin")).map(|p| clampf(p as f32 / 100.0, 0.05, 0.9)),
        continuous: match sk(a, "axis_type")
            .trim_matches('\'')
            .to_ascii_lowercase()
            .as_str()
        {
            "scalar" | "continuous" => Some(true),
            "categorical" => Some(false),
            _ => None,
        },
        min_date: date_bound(get(a, "min")),
        max_date: date_bound(get(a, "max")),
    }
}

#[derive(Debug, Clone)]
pub struct TitleF {
    pub show: bool,
    pub size: f32,
    pub color: Rgb,
    pub align: Align,
    pub font: FontSpec,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct Colors {
    pub def: Option<Rgb>,
    pub by_col: HashMap<usize, Rgb>,
    pub by_val: HashMap<String, Rgb>,
}

#[derive(Debug, Clone)]
pub struct ImgRef {
    pub id: String,
    pub w: f32,
    pub h: f32,
}
pub fn img_of(v: &Value) -> Option<ImgRef> {
    let id = sk(v, "id");
    if id.len() < 16 || id.len() > 64 || !id.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
        return None;
    }
    let (w, h) = (nk(v, "width", 0.0), nk(v, "height", 0.0));
    let ok = w > 0.0 && h > 0.0 && w <= 1e6 && h <= 1e6;
    Some(ImgRef {
        id: id.to_string(),
        w: if ok { w as f32 } else { 0.0 },
        h: if ok { h as f32 } else { 0.0 },
    })
}
pub fn fit_of(v: &str) -> &'static str {
    match v.to_lowercase().as_str() {
        "fit" => "fit",
        "fill" => "fill",
        _ => "normal",
    }
}
pub fn opacity_of(t: &Value) -> f32 {
    clampf(1.0 - num(t, 0.0) as f32 / 100.0, 0.0, 1.0)
}

#[derive(Debug, Clone)]
pub struct Fmt {
    pub title: TitleF,
    pub sub: TitleF,
    pub divider: (bool, Rgb, f32, String),
    pub bg_show: bool,
    pub bg: Rgb,
    pub bg_alpha: f32,
    pub bg_img: Option<ImgRef>,
    pub bg_fit: &'static str,
    pub border: (bool, Rgb, f32),
    pub labels: Option<bool>,
    pub legend_show: Option<bool>,
    pub legend_pos: String,
    pub legend_title: Option<bool>,
    pub legend_text: String,
    /// The legend's text colour (`legend.labelColor`) when the report sets one.
    pub legend_color: Option<Rgb>,
    pub x: Axis,
    pub y: Axis,
    /// The secondary value axis (combo, line and area charts).
    pub y2: Axis,
    pub colors: Colors,
    pub col_styles: HashMap<usize, (Option<Rgb>, Option<Rgb>)>,
    pub lines: Vec<Value>,
    pub sm: Option<(usize, usize, usize)>,
    pub titled: bool,
    /// Container padding (top, bottom, left, right) when the report or theme sets it.
    pub padding: Option<(f32, f32, f32, f32)>,
    /// The snapshot `format` itself, for the renderer formatting keys (labels, lines, card,
    /// gauge, pie: docs/view-snapshot.md "Renderer formatting").
    pub raw: Value,
    /// The visual's drill hierarchy (`drill` in the snapshot, `Null` without one).
    pub drill: Value,
}

pub const NO_TITLE: [&str; 15] = [
    "bmnav",
    "decomp",
    "card",
    "cardNew",
    "multiRow",
    "text",
    "shape",
    "image",
    "button",
    "nav",
    "slicer",
    "slicerList",
    "slicerTiles",
    "slicerText",
    "table",
];
pub const NO_BG: [&str; 5] = ["shape", "image", "button", "nav", "bmnav"];

pub fn vis_format(v: &Value, kind: &str, ctx: &Ctx<'_>) -> Fmt {
    let f = get(v, "format");
    let (t, b, bo, lg, sub, dv) = (
        get(f, "title"),
        get(f, "background"),
        get(f, "border"),
        get(f, "legend"),
        get(f, "subtitle"),
        get(f, "divider"),
    );
    let dl = get(f, "data_labels");
    let mut colors = Colors::default();
    for c in arr(get(f, "series_colors")).iter().take(500) {
        let Some(col) = Rgb::from_value(get(c, "color")) else {
            continue;
        };
        if c.get("value").is_some() {
            colors.by_val.insert(sel_key(get(c, "value")), col);
        } else if let Some(i) = get(c, "column").as_u64() {
            colors.by_col.insert(i as usize, col);
        } else {
            colors.def = Some(col);
        }
    }
    let mut col_styles = HashMap::new();
    for c in arr(get(f, "column_styles")).iter().take(200) {
        if let Some(i) = get(c, "column").as_u64() {
            col_styles.insert(
                i as usize,
                (Rgb::from_value(get(c, "bg")), Rgb::from_value(get(c, "fg"))),
            );
        }
    }
    let sm = get(f, "small_multiples");
    let is_matrix_no_title = kind == "matrix";
    let title_show = match get(t, "show") {
        Value::Bool(b) => *b,
        _ => !NO_TITLE.contains(&kind) && !is_matrix_no_title,
    };
    let dstyle = sk(dv, "style").to_lowercase();
    let mut fmt = Fmt {
        title: TitleF {
            show: title_show,
            size: if get(t, "font_size").is_null() {
                if ctx.classic { CLASSIC_TITLE_PX } else { 14.0 }
            } else {
                clampf(nk(t, "font_size", 10.5) as f32, 5.0, 72.0) * PT
            },
            color: Rgb::from_value(get(t, "color")).unwrap_or(if ctx.classic {
                CLASSIC_TITLE
            } else {
                ctx.fg
            }),
            align: align_of(sk(t, "alignment")),
            font: {
                let mut f = font_of(t, true);
                if ctx.classic && f.weight.is_none() {
                    f.weight = Some(400);
                }
                f
            },
            text: String::new(),
        },
        sub: TitleF {
            show: get(sub, "show") == &Value::Bool(true),
            size: clampf(nk(sub, "font_size", 10.0) as f32, 5.0, 72.0) * PT,
            color: Rgb::from_value(get(sub, "color")).unwrap_or(ctx.muted),
            align: align_of(sk(sub, "alignment")),
            font: font_of(sub, false),
            text: clip(sk(sub, "text"), 300),
        },
        divider: (
            get(dv, "show") == &Value::Bool(true),
            Rgb::from_value(get(dv, "color")).unwrap_or(ctx.grid),
            clampf(nk(dv, "width", 1.0) as f32, 1.0, 12.0),
            if dstyle == "dashed" || dstyle == "dotted" {
                dstyle
            } else {
                "solid".into()
            },
        ),
        bg_show: match get(b, "show") {
            Value::Bool(x) => *x,
            _ => !NO_BG.contains(&kind),
        },
        bg: Rgb::from_value(get(b, "color")).unwrap_or(ctx.bg),
        bg_alpha: 1.0 - clampf(nk(b, "transparency", 0.0) as f32, 0.0, 100.0) / 100.0,
        bg_img: img_of(get(b, "image")),
        bg_fit: fit_of(sk(b, "image_fit")),
        border: (
            get(bo, "show") == &Value::Bool(true),
            Rgb::from_value(get(bo, "color")).unwrap_or(ctx.grid),
            clampf(nk(bo, "radius", 0.0) as f32, 0.0, 400.0),
        ),
        labels: match dl {
            Value::Bool(x) => Some(*x),
            Value::Object(o) => o.get("show").and_then(Value::as_bool),
            _ => None,
        },
        legend_show: get(lg, "show").as_bool(),
        legend_pos: sk(lg, "position").to_lowercase(),
        legend_title: get(lg, "title").as_bool(),
        legend_text: clip(sk(lg, "title_text"), 200),
        legend_color: Rgb::from_value(get(lg, "labels_color")),
        x: axis_of(get(f, "x_axis")),
        y: axis_of(get(f, "y_axis")),
        y2: axis_of(get(f, "y2_axis")),
        colors,
        col_styles,
        lines: arr(get(f, "reference_lines"))
            .iter()
            .filter(|l| l.is_object())
            .take(20)
            .cloned()
            .collect(),
        sm: get(sm, "column").as_u64().map(|c| {
            (
                c as usize,
                clampf(nk(sm, "rows", 0.0) as f32, 0.0, 6.0) as usize,
                clampf(nk(sm, "columns", 0.0) as f32, 0.0, 6.0) as usize,
            )
        }),
        titled: false,
        raw: f.clone(),
        drill: get(v, "drill").clone(),
        padding: {
            let p = get(f, "padding");
            p.is_object().then(|| {
                let side = |k: &str| clampf(nk(p, k, 0.0) as f32, 0.0, 200.0);
                (side("top"), side("bottom"), side("left"), side("right"))
            })
        },
    };
    visual_styles(&mut fmt, get(get(v, "data"), "visual_styles"));
    fmt
}

/// The whole-visual conditional formats the producer evaluated (`data.visual_styles`: a title
/// from a field, the title's, subtitle's, background's and border's colours, the axes' label
/// and title colours, the legend's, a gauge's target and callout and a KPI's indicator, goal and
/// status colours by rule) over the visual's static format.
fn visual_styles(f: &mut Fmt, s: &Value) {
    if !s.is_object() {
        return;
    }
    let color = |k: &str| Rgb::from_value(get(s, k));
    if let Some(c) = color("category_axis_color") {
        f.x.color = Some(c);
    }
    if let Some(c) = color("category_title_color") {
        f.x.title_color = Some(c);
    }
    if let Some(c) = color("value_axis_color") {
        f.y.color = Some(c);
    }
    if let Some(c) = color("value_title_color") {
        f.y.title_color = Some(c);
    }
    if let Some(c) = color("legend_color") {
        f.legend_color = Some(c);
    }
    // the gauge's and KPI's colours live in the snapshot `format` the bodies read (`raw`)
    let patches: [(&str, &[&str]); 9] = [
        ("target_color", &["gauge", "target_color"]),
        ("target_label_color", &["gauge", "target_label", "color"]),
        ("callout_color", &["gauge", "callout", "color"]),
        ("indicator_color", &["kpi", "color"]),
        ("goal_color", &["kpi", "goal_color"]),
        ("distance_color", &["kpi", "distance_color"]),
        ("good_color", &["kpi", "good_color"]),
        ("neutral_color", &["kpi", "neutral_color"]),
        ("bad_color", &["kpi", "bad_color"]),
    ];
    for (key, path) in patches {
        if let Some(c) = get(s, key).as_str().filter(|_| color(key).is_some()) {
            set_path(&mut f.raw, path, Value::String(c.to_string()));
        }
    }
    let text = |k: &str| Some(sk(s, k)).filter(|t| !t.is_empty()).map(|t| clip(t, 300));
    if let Some(t) = text("title_text") {
        f.title.text = t;
    }
    if let Some(t) = text("subtitle_text") {
        f.sub.text = t;
    }
    if let Some(c) = Rgb::from_value(get(s, "title_color")) {
        f.title.color = c;
    }
    if let Some(c) = Rgb::from_value(get(s, "subtitle_color")) {
        f.sub.color = c;
    }
    if let Some(c) = Rgb::from_value(get(s, "visual_background")) {
        f.bg = c;
    }
    if let Some(c) = Rgb::from_value(get(s, "border_color")) {
        f.border.1 = c;
    }
}

/// Sets `path` in `v` (objects made where missing; a non-object on the way is replaced).
fn set_path(v: &mut Value, path: &[&str], x: Value) {
    let Some((last, head)) = path.split_last() else {
        return;
    };
    let mut at = v;
    for k in head {
        if !at.is_object() {
            *at = Value::Object(serde_json::Map::new());
        }
        let Some(o) = at.as_object_mut() else {
            return;
        };
        at = o
            .entry((*k).to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
    }
    if !at.is_object() {
        *at = Value::Object(serde_json::Map::new());
    }
    if let Some(o) = at.as_object_mut() {
        o.insert((*last).to_string(), x);
    }
}

/// The title Power BI shows when the visual has none ("Sales by Region"), in the model
/// culture's language (Desktop writes it in its UI language).
pub fn auto_title(v: &Value, loc: &crate::fmt::Locale) -> String {
    // the producer's title (translated captions, Desktop's phrasing) when it gives one
    if let Some(t) = v.get("auto_title").and_then(Value::as_str) {
        return t.to_string();
    }
    let fs: Vec<&Value> = arr(get(v, "fields"))
        .iter()
        .filter(|f| f.is_object())
        .take(40)
        .collect();
    let by = |rcs: &[Rc]| -> Vec<String> {
        fs.iter()
            .filter(|f| rcs.contains(&role_class(sk(f, "role"))))
            .map(|f| field_label(f))
            .collect()
    };
    let vals = by(&[Rc::Val, Rc::Y2, Rc::Goal, Rc::Min, Rc::Max]);
    let cats = by(&[Rc::Cat, Rc::Leg, Rc::Col]);
    let lang = loc.tag.to_ascii_lowercase();
    let (and, by_w) = match lang.split(['-', '_']).next().unwrap_or("en") {
        "fr" => (" et ", " par "),
        "de" => (" und ", " nach "),
        "es" => (" y ", " por "),
        "it" => (" e ", " per "),
        "nl" => (" en ", " per "),
        "pt" => (" e ", " por "),
        _ => (" and ", " by "),
    };
    if vals.is_empty() {
        return cats.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    }
    let mut t = vals.iter().take(3).cloned().collect::<Vec<_>>().join(and);
    if !cats.is_empty() {
        t.push_str(by_w);
        t.push_str(&cats.iter().take(2).cloned().collect::<Vec<_>>().join(and));
    }
    t
}

// --- chart data ------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Series {
    pub name: String,
    pub col: Col,
    pub vals: Vec<Option<f64>>,
    pub y2: bool,
    pub raw: Value,
    pub j: Option<usize>,
    pub pts: Vec<Option<Rgb>>,
    /// Conditional data label colour of each category (`labels.color` by rule).
    pub lpts: Vec<Option<Rgb>>,
    pub color: Rgb,
    pub base: Option<Vec<f64>>,
    /// The highlighted part of each category (`data.highlights`), when the visual is highlighted.
    pub hvals: Option<Vec<Option<f64>>>,
    /// Error bar bounds per category, (upper, lower); empty without error bars.
    pub err: Vec<(Option<f64>, Option<f64>)>,
}

impl Series {
    /// Color of category `i` (conditional or per-category first).
    pub fn ptc(&self, i: usize) -> Rgb {
        self.pts.get(i).copied().flatten().unwrap_or(self.color)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChartData {
    pub cats: Vec<String>,
    pub raw: Vec<Value>,
    pub series: Vec<Series>,
    pub cat_name: String,
    pub legend_name: String,
    pub truncated: bool,
    /// The category column holds dates (a continuous axis in Desktop).
    pub cat_date: bool,
    /// The `data.columns` index of the category and of the legend.
    pub ci: Option<usize>,
    pub li: Option<usize>,
    /// Further category columns (an axis expanded down a level: Desktop's concatenated labels)
    /// and their values, per category.
    pub more: Vec<usize>,
    pub raw_more: Vec<Vec<Value>>,
}

pub const BLANK: &str = "(Blank)";

fn add_to(a: &mut Vec<Option<f64>>, i: usize, x: &Value) {
    if a.len() <= i {
        a.resize(i + 1, None);
    }
    if let (Some(x), Some(slot)) = (fin(x), a.get_mut(i)) {
        *slot = Some(slot.unwrap_or(0.0) + x.clamp(-1e15, 1e15));
    }
}

/// Categories × series. A legend splits the first value column into series.
pub fn chart_data(
    ds: &Dataset,
    ctx: &Ctx<'_>,
    f: Option<&Fmt>,
    sort_desc: bool,
    max_cats: usize,
) -> ChartData {
    let (cols, rows) = (&ds.cols, &ds.rows);
    let is_n: Vec<bool> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| is_num_col(c, rows, i))
        .collect();
    let isn = |i: usize| is_n.get(i).copied().unwrap_or(false);
    let ci = cols.iter().position(|c| c.rc == Rc::Cat).or_else(|| {
        cols.iter()
            .enumerate()
            .position(|(i, c)| !isn(i) && c.rc != Rc::Leg && c.rc != Rc::Tip && c.rc != Rc::Col)
    });
    let li = cols
        .iter()
        .enumerate()
        .position(|(i, c)| c.rc == Rc::Leg && Some(i) != ci);
    // an axis expanded down a level: every displayed level groups the categories
    let more: Vec<usize> = match ci {
        Some(c0) if cols.get(c0).is_some_and(|c| c.rc == Rc::Cat) => cols
            .iter()
            .enumerate()
            .filter(|(i, c)| *i != c0 && c.rc == Rc::Cat)
            .map(|(i, _)| i)
            .take(3)
            .collect(),
        _ => Vec::new(),
    };
    let vi: Vec<usize> = (0..cols.len())
        .filter(|&i| {
            isn(i)
                && Some(i) != ci
                && Some(i) != li
                && cols
                    .get(i)
                    .is_some_and(|c| matches!(c.rc, Rc::Val | Rc::Y2 | Rc::None))
        })
        .collect();
    let mut cd = ChartData {
        truncated: ds.truncated,
        ..ChartData::default()
    };
    let mut at: HashMap<String, usize> = HashMap::new();
    let ci_col = ci.and_then(|i| cols.get(i));
    let mut cat_of = |r: &Vec<Value>, cd: &mut ChartData| -> Option<usize> {
        let x = ci.and_then(|i| r.get(i)).cloned().unwrap_or(Value::Null);
        let xs: Vec<Value> = more
            .iter()
            .map(|j| r.get(*j).cloned().unwrap_or(Value::Null))
            .collect();
        let k = if xs.is_empty() {
            key_of(&x)
        } else {
            std::iter::once(&x)
                .chain(xs.iter())
                .map(key_of)
                .collect::<Vec<_>>()
                .join("{1}")
        };
        if let Some(i) = at.get(&k) {
            return Some(*i);
        }
        if cd.cats.len() >= max_cats {
            cd.truncated = true;
            return None;
        }
        let i = cd.cats.len();
        at.insert(k, i);
        let label = match ci_col {
            // first-of-month dates read as months, like Desktop's date axis ("Jan 2025")
            Some(c)
                if c.is_date()
                    && x.as_str()
                        .is_some_and(|s| s.get(8..10) == Some("01") && crate::fmt::is_iso_date(s)) =>
            {
                let s = x.as_str().unwrap_or("");
                let m: usize = s.get(5..7).and_then(|m| m.parse().ok()).unwrap_or(1);
                format!(
                    "{} {}",
                    month_name(m.saturating_sub(1), &ctx.loc),
                    s.get(0..4).unwrap_or("")
                )
            }
            Some(c) => {
                let t = fmt_cell(&x, Some(&c.cf()), &ctx.loc, false);
                if t.is_empty() { ctx.blank() } else { t }
            }
            None => String::new(),
        };
        let label = if xs.is_empty() {
            label
        } else {
            let mut parts = vec![label];
            for (j, y) in more.iter().zip(&xs) {
                let t = fmt_cell(y, cols.get(*j).map(Col::cf).as_ref(), &ctx.loc, false);
                parts.push(if t.is_empty() { ctx.blank() } else { t });
            }
            parts.join(" ")
        };
        cd.raw_more.push(xs);
        cd.raw.push(x);
        cd.cats.push(label);
        Some(i)
    };
    let v1: Vec<usize> = vi
        .iter()
        .copied()
        .filter(|i| cols.get(*i).is_some_and(|c| c.rc != Rc::Y2))
        .collect();
    let mut series: Vec<Series> = Vec::new();
    let mk = |name: String, col: &Col, y2: bool, raw: Value, j: Option<usize>| Series {
        name,
        col: col.clone(),
        vals: Vec::new(),
        y2,
        raw,
        j,
        pts: Vec::new(),
        lpts: Vec::new(),
        color: DEFAULT_COLORS[0],
        base: None,
        hvals: None,
        err: Vec::new(),
    };
    // the highlighted part of each row's values (`data.highlights`), as the viewer's `ds.hl`
    let hl = |k: usize, j: usize| -> Option<&Value> {
        ds.hl
            .as_ref()
            .map(|h| h.get(k).and_then(|r| r.get(j)).unwrap_or(&Value::Null))
    };
    if let (Some(li), Some(&v0)) = (li, v1.first()) {
        let mut s_at: HashMap<String, usize> = HashMap::new();
        for (ri, r) in rows.iter().enumerate() {
            let Some(i) = cat_of(r, &mut cd) else { continue };
            let lv = r.get(li).cloned().unwrap_or(Value::Null);
            let k = key_of(&lv);
            let si = match s_at.get(&k) {
                Some(s) => *s,
                None => {
                    if s_at.len() >= 12 {
                        continue;
                    }
                    let (Some(lc), Some(vc)) = (cols.get(li), cols.get(v0)) else {
                        continue;
                    };
                    let t = fmt_cell(&lv, Some(&lc.cf()), &ctx.loc, false);
                    series.push(mk(
                        if t.is_empty() { ctx.blank() } else { t },
                        vc,
                        false,
                        lv.clone(),
                        None,
                    ));
                    s_at.insert(k, series.len() - 1);
                    series.len() - 1
                }
            };
            if let Some(s) = series.get_mut(si) {
                add_to(&mut s.vals, i, r.get(v0).unwrap_or(&Value::Null));
                if let Some(h) = hl(ri, v0) {
                    add_to(s.hvals.get_or_insert_with(Vec::new), i, h);
                }
            }
        }
        series.sort_by(|a, b| cmp_raw(&a.raw, &b.raw));
        let extra: Vec<usize> = vi
            .iter()
            .copied()
            .filter(|i| cols.get(*i).is_some_and(|c| c.rc == Rc::Y2))
            .take(4)
            .collect();
        for j in extra {
            let Some(c) = cols.get(j) else { continue };
            let mut s = mk(c.name.clone(), c, true, Value::Null, Some(j));
            let mut n: Vec<usize> = Vec::new();
            for r in rows {
                let Some(i) = cat_of(r, &mut cd) else { continue };
                let x = r.get(j).unwrap_or(&Value::Null);
                if fin(x).is_some() {
                    add_to(&mut s.vals, i, x);
                    if n.len() <= i {
                        n.resize(i + 1, 0);
                    }
                    if let Some(k) = n.get_mut(i) {
                        *k += 1;
                    }
                }
            }
            if fmt_parts(&c.fmt).pct {
                for (i, x) in s.vals.iter_mut().enumerate() {
                    if let Some(v) = x {
                        *v /= n.get(i).copied().unwrap_or(1).max(1) as f64;
                    }
                }
            }
            series.push(s);
        }
    } else {
        series = vi
            .iter()
            .take(12)
            .filter_map(|&j| {
                cols.get(j)
                    .map(|c| mk(c.name.clone(), c, c.rc == Rc::Y2, Value::Null, Some(j)))
            })
            .collect();
        for (k, r) in rows.iter().enumerate() {
            let Some(i) = cat_of(r, &mut cd) else { continue };
            for s in &mut series {
                let j = s.j.unwrap_or(0);
                add_to(&mut s.vals, i, r.get(j).unwrap_or(&Value::Null));
                if let Some(h) = hl(k, j) {
                    add_to(s.hvals.get_or_insert_with(Vec::new), i, h);
                }
                // the error bars of this series: the bounds of the category's row
                if let Some(e) = ds.errs.iter().find(|e| e.column == j) {
                    if s.err.len() <= i {
                        s.err.resize(i + 1, (None, None));
                    }
                    if let Some(slot) = s.err.get_mut(i) {
                        *slot = (
                            e.upper.get(k).copied().flatten(),
                            e.lower.get(k).copied().flatten(),
                        );
                    }
                }
                if let Some(c) = ds.styles.get(&(k, j)).and_then(|c| c.fill) {
                    if s.pts.len() <= i {
                        s.pts.resize(i + 1, None);
                    }
                    if let Some(p) = s.pts.get_mut(i) {
                        *p = Some(c);
                    }
                }
                if let Some(c) = ds.styles.get(&(k, j)).and_then(|c| c.label) {
                    if s.lpts.len() <= i {
                        s.lpts.resize(i + 1, None);
                    }
                    if let Some(p) = s.lpts.get_mut(i) {
                        *p = Some(c);
                    }
                }
            }
        }
    }
    let n = cd.cats.len();
    for s in &mut series {
        s.vals.resize(n, None);
        if !s.err.is_empty() {
            s.err.resize(n, (None, None));
        }
        if ds.hl.is_some() {
            s.hvals.get_or_insert_with(Vec::new).resize(n, None);
        }
    }
    // Desktop sorts text categories by the measure, descending; dates and numbers keep their order.
    if sort_desc
        && ds.fake
        && ci.is_some_and(|i| !isn(i) && !cols.get(i).is_some_and(Col::is_date))
        && !series.is_empty()
        && n > 1
    {
        let weight = |i: usize| -> f64 {
            if li.is_some() {
                series
                    .iter()
                    .map(|s| s.vals.get(i).copied().flatten().unwrap_or(0.0))
                    .sum()
            } else {
                series
                    .first()
                    .and_then(|s| s.vals.get(i).copied().flatten())
                    .unwrap_or(0.0)
            }
        };
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|a, b| {
            weight(*b)
                .partial_cmp(&weight(*a))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.cmp(b))
        });
        cd.cats = order.iter().filter_map(|i| cd.cats.get(*i).cloned()).collect();
        cd.raw = order.iter().filter_map(|i| cd.raw.get(*i).cloned()).collect();
        if !cd.raw_more.is_empty() {
            cd.raw_more = order
                .iter()
                .filter_map(|i| cd.raw_more.get(*i).cloned())
                .collect();
        }
        for s in &mut series {
            s.vals = order.iter().map(|i| s.vals.get(*i).copied().flatten()).collect();
            if !s.pts.is_empty() {
                s.pts = order.iter().map(|i| s.pts.get(*i).copied().flatten()).collect();
            }
            if !s.lpts.is_empty() {
                s.lpts = order.iter().map(|i| s.lpts.get(*i).copied().flatten()).collect();
            }
        }
    }
    let colors = f.map(|f| &f.colors);
    let m = series.len();
    for (i, s) in series.iter_mut().enumerate() {
        let mut c = None;
        if let Some(cs) = colors {
            c = if li.is_some() && !s.y2 {
                cs.by_val.get(&sel_key(&s.raw)).copied()
            } else {
                s.j.and_then(|j| cs.by_col.get(&j).copied())
            };
            if c.is_none() && (m == 1 || i == 0) {
                c = cs.def;
            }
        }
        s.color = c.unwrap_or_else(|| ctx.color(i));
    }
    if let Some(cs) = colors
        && !cs.by_val.is_empty()
        && ci.is_some()
        && li.is_none()
        && m == 1
        && let Some(s) = series.first_mut()
    {
        s.pts.resize(n, None);
        for (i, x) in cd.raw.iter().enumerate() {
            if let (Some(c), Some(p)) = (cs.by_val.get(&sel_key(x)), s.pts.get_mut(i))
                && p.is_none()
            {
                *p = Some(*c);
            }
        }
    }
    cd.cat_date = more.is_empty() && ci.and_then(|i| cols.get(i)).is_some_and(Col::is_date);
    cd.more = more;
    cd.cat_name = ci
        .and_then(|i| cols.get(i))
        .map(|c| c.name.clone())
        .unwrap_or_default();
    cd.legend_name = li
        .and_then(|i| cols.get(i))
        .map(|c| c.name.clone())
        .unwrap_or_default();
    cd.ci = ci;
    cd.li = li;
    cd.series = series;
    cd
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rng_matches_the_viewer() {
        // mulberry32 seeded with FNV-1a("visual"), as in page.html
        let mut r = Rng::new("visual");
        let a = r.next();
        assert!((0.0..1.0).contains(&a));
        let mut r2 = Rng::new("visual");
        assert_eq!(a, r2.next());
    }

    #[test]
    fn chart_data_groups_by_legend() {
        let fonts = FontBook::bundled_only();
        let snap = json!({});
        let ctx = Ctx::new(&snap, &fonts, None);
        let v = json!({"fields": [], "data": {"status": "ok",
            "columns": [{"name": "Cat", "role": "Category", "data_type": "string"},
                        {"name": "Leg", "role": "Series", "data_type": "string"},
                        {"name": "Val", "role": "Y", "kind": "measure"}],
            "rows": [["a", "x", 1], ["a", "y", 2], ["b", "x", 3]]}});
        let ds = dataset(&v, "col", &ctx);
        let cd = chart_data(&ds, &ctx, None, true, 60);
        assert_eq!(cd.cats, vec!["a", "b"]);
        assert_eq!(cd.series.len(), 2);
        assert_eq!(cd.series[0].vals, vec![Some(1.0), Some(3.0)]);
        assert_eq!(cd.series[1].vals, vec![Some(2.0), None]);
    }

    /// A hidden field (a visual calculation's input) is in the data but not drawn.
    #[test]
    fn hidden_columns_are_dropped() {
        let fonts = FontBook::bundled_only();
        let snap = json!({});
        let ctx = Ctx::new(&snap, &fonts, None);
        let v = json!({"fields": [], "data": {"status": "ok",
            "columns": [{"name": "Cat", "role": "Category", "data_type": "string"},
                        {"name": "Val", "role": "Y", "kind": "measure", "hidden": true},
                        {"name": "Run", "role": "Y", "kind": "visual_calculation"}],
            "rows": [["a", 1, 1], ["b", 3, 4]],
            "totals": [null, 4, 4]}});
        let ds = dataset(&v, "col", &ctx);
        assert_eq!(ds.cols.len(), 2);
        assert_eq!(
            ds.rows,
            vec![vec![json!("a"), json!(1)], vec![json!("b"), json!(4)]]
        );
        assert_eq!(ds.totals, Some(vec![Value::Null, json!(4)]));
        let cd = chart_data(&ds, &ctx, None, true, 60);
        assert_eq!(cd.series.len(), 1);
        assert_eq!(cd.series[0].vals, vec![Some(1.0), Some(4.0)]);
    }

    #[test]
    fn fake_data_is_deterministic() {
        let fonts = FontBook::bundled_only();
        let snap = json!({});
        let ctx = Ctx::new(&snap, &fonts, None);
        let v = json!({"name": "abc", "fields": [{"role": "Category", "kind": "column", "name": "Region"},
                                                 {"role": "Y", "kind": "measure", "name": "Sales"}]});
        let a = dataset(&v, "col", &ctx);
        let b = dataset(&v, "col", &ctx);
        assert!(a.fake && a.rows.len() == 6);
        assert_eq!(a.rows, b.rows);
    }
}
