//! Slicers drawn like Power BI Desktop 2.158: the classic slicer in each of its modes (vertical
//! list with check boxes, "Select all", search box and hierarchy chevrons; horizontal tiles;
//! dropdown; between / less-or-equal / greater-or-equal with the two inputs and the slider;
//! relative date), the new list slicer (`listSlicer`), the button slicer
//! (`advancedSlicerVisual`) and the text slicer (`textSlicer`).
//!
//! Geometry and colours were measured on Desktop's own rendering (`tools/render-oracle`, the
//! slicer pages of `docs/render.md`); positions are in body pixels, the body being the visual
//! inside its container padding (5 px for slicers).

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::canvas::{Anchor, Base, Canvas, Rgb, Text, poly, svg_path};
use crate::charts::Bx;
use crate::data::{Ctx, Dataset, Fmt, PT, arr, cmp_raw, get, key_of, sel_key, sk};
use crate::fmt::{fmt_cell, is_iso_date, iso_days};
use crate::fonts::Font;
use crate::hits::{HitAction, HitShape};

/// Most items drawn (rows of a list, tiles, buttons).
const MAX_ITEMS: usize = 300;
/// Most selected values read.
const MAX_SELECTED: usize = 1000;

// Desktop 2.158 classic look (a report without a theme); a theme's foreground replaces the
// greys.
const HEADER_C: Rgb = Rgb(0, 0, 0);
const ITEM_C: Rgb = Rgb(0x66, 0x66, 0x66);
const BOX_ON_C: Rgb = Rgb(0x21, 0x21, 0x21);
const PARTIAL_C: Rgb = Rgb(0xc8, 0xc8, 0xc8);
const INPUT_BORDER_C: Rgb = Rgb(0xea, 0xea, 0xea);
const DISABLED_C: Rgb = Rgb(0xb2, 0xb2, 0xb2);
const CHEVRON_C: Rgb = Rgb(0x6e, 0x6e, 0x6e);
const EXPAND_C: Rgb = Rgb(0x82, 0x80, 0x7e);
const SEARCH_C: Rgb = Rgb(0x75, 0x75, 0x75);
const RULE_C: Rgb = Rgb(0xd7, 0xd7, 0xd7);
const TILE_ON_C: Rgb = Rgb(0x33, 0x33, 0x33);
const NEW_TEXT_C: Rgb = Rgb(0x33, 0x33, 0x33);
const BUTTON_BORDER_C: Rgb = Rgb(0xaa, 0xaa, 0xaa);
const FIELD_C: Rgb = Rgb(0xea, 0xea, 0xea);
const ARROW_C: Rgb = Rgb(0xc2, 0xc2, 0xc2);

/// The labels Desktop shows in its UI language: the snapshot's `ui.strings` when it has them,
/// else (relative-date and text-slicer words it does not carry yet) by the model culture.
struct Words {
    select_all: String,
    search: String,
    all: String,
    multiple: String,
    last: String,
    select: String,
    no_filter: String,
    enter: String,
    fr: bool,
}

fn words(ctx: &Ctx<'_>) -> Words {
    // Desktop's UI language (the snapshot's `ui`), else the formatting culture's
    let lang = if ctx.ui_lang.is_empty() {
        &ctx.loc.tag
    } else {
        &ctx.ui_lang
    }
    .to_ascii_lowercase();
    let fr = lang.split(['-', '_']).next() == Some("fr");
    let pick = |key: &str, en: &str, frs: &str| ctx.t(key, if fr { frs } else { en });
    Words {
        select_all: pick("select_all", "Select all", "Sélectionner tout"),
        search: pick("search", "Search", "Rechercher"),
        all: pick("all", "All", "Tout"),
        multiple: pick(
            "multiple_selections",
            "Multiple selections",
            "Plusieurs sélections",
        ),
        last: pick("relative_last", "Last", "Dernier"),
        select: pick("relative_select", "Select", "Sélectionner"),
        no_filter: pick("no_filters_applied", "No active filter", "Aucun filtre actif"),
        enter: pick("text_slicer_enter", "Enter", "Entrée"),
        fr,
    }
}

/// Colours of one slicer: Desktop's classic greys, or the theme's foreground.
#[derive(Clone, Copy)]
struct Pal {
    header: Rgb,
    item: Rgb,
    on: Rgb,
    value: Rgb,
}

fn pal(ctx: &Ctx<'_>) -> Pal {
    if ctx.classic {
        Pal {
            header: HEADER_C,
            item: ITEM_C,
            on: BOX_ON_C,
            value: HEADER_C,
        }
    } else {
        Pal {
            header: ctx.fg,
            item: ctx.fg,
            on: ctx.fg,
            value: ctx.fg,
        }
    }
}

fn base_text(cv: &mut Canvas, ctx: &Ctx<'_>, s: &str, f: Font, c: Rgb, x: f32, y: f32, a: Anchor) -> f32 {
    cv.text(
        ctx.fonts,
        &Text::new(s, f, c).anchor(a).base(Base::Alphabetic),
        x,
        y,
    )
}

/// The slicer state the snapshot gives (`docs/view-snapshot.md`, "slicer").
struct State {
    mode: String,
    /// The mode was given by the snapshot (else guessed from the type, data and size).
    explicit: bool,
    sel: HashSet<String>,
    sel_raw: Vec<Value>,
    inverted: bool,
    search: String,
    single: Option<bool>,
    select_all: bool,
    search_box: bool,
    header_show: Option<bool>,
    range: Value,
    domain: Value,
    relative: Value,
}

fn state(v: &Value, default_mode: &str) -> State {
    let sl = get(v, "slicer");
    let sm = sk(sl, "mode").to_ascii_lowercase();
    let mode = match sm.as_str() {
        "" => default_mode.to_string(),
        "tile" | "tiles" => "tiles".into(),
        m => m.to_string(),
    };
    let sel_raw: Vec<Value> = arr(get(sl, "selected"))
        .iter()
        .take(MAX_SELECTED)
        .cloned()
        .collect();
    State {
        explicit: !sm.is_empty(),
        mode,
        sel: sel_raw.iter().map(sel_key).collect(),
        sel_raw,
        inverted: get(sl, "inverted") == &Value::Bool(true),
        search: sk(sl, "search").chars().take(200).collect(),
        single: get(sl, "single_select").as_bool(),
        select_all: get(sl, "select_all") == &Value::Bool(true),
        search_box: get(sl, "search_box") == &Value::Bool(true),
        header_show: get(sl, "header_show").as_bool(),
        range: get(sl, "range").clone(),
        domain: get(sl, "domain").clone(),
        relative: get(sl, "relative").clone(),
    }
}

impl State {
    fn is_on(&self, x: &Value) -> bool {
        !self.sel.is_empty() && self.sel.contains(&sel_key(x)) != self.inverted
    }
}

/// Distinct values of column `c` in row order (at most [`MAX_ITEMS`]).
fn distinct(rows: &[Vec<Value>], c: usize) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for r in rows {
        let x = r.get(c).cloned().unwrap_or(Value::Null);
        if seen.insert(key_of(&x)) {
            out.push(x);
            if out.len() >= MAX_ITEMS {
                break;
            }
        }
    }
    out
}

/// Draws a slicer body. `default_mode`: `slicer` (the classic slicer, whose snapshot gives the
/// mode), `list` (`listSlicer`), `tiles` (`advancedSlicerVisual`) or `text` (`textSlicer`).
pub fn slicer_v2(
    cv: &mut Canvas,
    ds: &Dataset,
    b: Bx,
    f: &Fmt,
    ctx: &Ctx<'_>,
    v: &Value,
    default_mode: &str,
) {
    let st = state(v, default_mode);
    let vals = distinct(&ds.rows, 0);
    let col = ds.cols.first();
    let cf = col.map(|c| c.cf());
    let label = |x: &Value| {
        let t = fmt_cell(x, cf.as_ref(), &ctx.loc, false);
        if t.is_empty() { ctx.blank() } else { t }
    };
    let labels: Vec<String> = vals.iter().map(label).collect();
    if matches!(default_mode, "list" | "tiles" | "text") {
        // Desktop shows these visuals' title (the field name) by default; drawn here when the
        // container did not draw one and the report does not hide it
        let name = col.map(|c| c.name.as_str()).unwrap_or("");
        let hidden = get(get(get(v, "format"), "title"), "show") == &Value::Bool(false);
        let top = if !f.titled && !hidden && !name.is_empty() {
            let hf = Font::new(&ctx.font, 10.0 * PT);
            let t = ctx.fonts.fit(&hf, name, b.w.max(0.0));
            base_text(cv, ctx, &t, hf, f.title.color, 0.3, 8.3, Anchor::Start);
            16.3
        } else {
            0.0
        };
        let b2 = Bx {
            w: b.w,
            h: (b.h - top).max(0.0),
        };
        let mut sub = match cv.layer(0.0, top, b2.w.max(1.0), b2.h.max(1.0)) {
            Some(c) => c,
            None => return,
        };
        match default_mode {
            "list" => new_list(&mut sub, ctx, b2, &st, &vals, &labels),
            "tiles" => buttons(&mut sub, ctx, b2, &st, &vals, &labels),
            _ => text_slicer(&mut sub, ctx, b2, &st, name),
        }
        cv.draw(&sub, 0.0, top, 1.0);
        return;
    }
    // the classic slicer: its header, then the body of its mode
    let names: Vec<&str> = ds.cols.iter().take(8).map(|c| c.name.as_str()).collect();
    // the visual's `header.text`, else the field names
    let header = match sk(get(v, "slicer"), "header_text") {
        "" => names.join(", "),
        t => t.to_string(),
    };
    let p = pal(ctx);
    let mut top = 0.0f32;
    // the header: 12 px black (Desktop 2.158 computed style)
    if st.header_show.unwrap_or(!f.titled) && !header.is_empty() {
        let hf = Font::new(&ctx.font, 9.0 * PT);
        let t = ctx.fonts.fit(&hf, &header, (b.w - 9.0).max(0.0));
        base_text(cv, ctx, &t, hf, p.header, 9.0, 13.7, Anchor::Start);
        top = 17.0;
    }
    let numeric = col.is_some_and(|c| c.is_date() || c.is_num_type())
        && vals
            .iter()
            .all(|x| x.as_f64().is_some_and(f64::is_finite) || x.as_str().is_some_and(is_iso_date));
    let mode = if st.explicit {
        st.mode.clone()
    } else if st.range.is_object() || (numeric && vals.len() > 1 && b.h >= 60.0) {
        "between".to_string()
    } else if b.h < 50.0 {
        "dropdown".to_string()
    } else {
        "list".to_string()
    };
    match mode.as_str() {
        "dropdown" => dropdown(cv, ctx, b, top, &st, &vals, &labels),
        "tiles" => tiles(cv, ctx, b, top, &st, &vals, &labels),
        "between" | "before" | "after" => {
            // a decimal column without a format string shows two decimals in the inputs
            let mut rcf = cf.clone();
            if let Some(c) = rcf.as_mut() {
                let t = c.data_type.to_ascii_lowercase();
                // the date inputs keep the culture's short date whatever the column's format
                let lower = c.fmt.to_ascii_lowercase();
                if lower.ends_with(" date")
                    || lower.ends_with(" time")
                    || bicraft_locale::vb::is_date_format(&c.fmt)
                {
                    c.fmt.clear();
                }
                if c.fmt.is_empty() && ["double", "decimal", "currency"].iter().any(|k| t.contains(k)) {
                    c.fmt = "#,0.00".into();
                }
            }
            let fmt = |x: &Value| fmt_cell(x, rcf.as_ref(), &ctx.loc, false);
            range(cv, ctx, b, top, &st, &mode, &vals, &fmt)
        }
        "relative" => {
            // the covered days in the culture's short date
            let mut dcf = cf.clone();
            if let Some(c) = dcf.as_mut() {
                c.fmt.clear();
            }
            let fmt = |x: &Value| fmt_cell(x, dcf.as_ref(), &ctx.loc, false);
            relative(cv, ctx, b, top, &st.relative, &fmt)
        }
        _ => {
            let level2 = ds.cols.len() > 1;
            list(cv, ctx, b, top, &st, &vals, &labels, level2)
        }
    }
}

/// The gap above a dropdown, range or relative-date input: 6.7 px under the header, 5.7 px
/// at the top when the header is off (Desktop 2.158).
fn inset(top: f32) -> f32 {
    if top > 0.0 { 6.7 } else { 5.7 }
}

// --- classic list --------------------------------------------------------------------------

/// A check box: empty, filled (selected) or partial (some selected, for "Select all").
fn check_box(cv: &mut Canvas, x: f32, y: f32, s: f32, state: u8, p: Pal) {
    match state {
        1 => cv.rect(x, y, s, s, p.on, 1.0),
        2 => {
            cv.round_rect(
                x + 0.5,
                y + 0.5,
                s - 1.0,
                s - 1.0,
                0.0,
                None,
                Some((PARTIAL_C, 1.0, None)),
            );
            let i = s * 0.25;
            cv.rect(x + i, y + i, s - 2.0 * i, s - 2.0 * i, PARTIAL_C, 1.0);
        }
        _ => cv.round_rect(
            x + 0.5,
            y + 0.5,
            s - 1.0,
            s - 1.0,
            0.0,
            None,
            Some((p.item, 1.0, None)),
        ),
    }
}

fn search_glass(cv: &mut Canvas, x: f32, y: f32, size: f32, c: Rgb) {
    // a lens up right, the handle down left (Desktop's search icon)
    if let Some(path) = svg_path("M7 1a4 4 0 1 1 0 8 4 4 0 0 1 0-8zM4.2 7.8L0.8 11.2") {
        let k = size / 12.0;
        cv.stroke_tf(
            &path,
            c,
            1.0 / k.max(0.1),
            tiny_skia::Transform::from_translate(x, y).pre_scale(k, k),
        );
    }
}

fn chevron_down(cv: &mut Canvas, x: f32, y: f32, w: f32, h: f32, c: Rgb, width: f32) {
    if let Some(path) = poly(&[(x, y), (x + w / 2.0, y + h), (x + w, y)], false) {
        cv.stroke(&path, c, 1.0, width, None);
    }
}

fn chevron_right(cv: &mut Canvas, x: f32, y: f32, w: f32, h: f32, c: Rgb) {
    if let Some(path) = poly(&[(x, y), (x + w, y + h / 2.0), (x, y + h)], false) {
        cv.stroke(&path, c, 1.0, 1.0, None);
    }
}

#[allow(clippy::too_many_arguments)]
fn list(
    cv: &mut Canvas,
    ctx: &Ctx<'_>,
    b: Bx,
    top: f32,
    st: &State,
    vals: &[Value],
    labels: &[String],
    hierarchy: bool,
) {
    let p = pal(ctx);
    let w = words(ctx);
    let fnt = Font::new(&ctx.font, 10.0 * PT);
    let (bx, bs, pitch) = (7.7f32, 13.3f32, 19.0f32);
    // with the header (17 px), the first box starts at 22.3
    let mut y = top + 5.3;
    if st.search_box || !st.search.is_empty() {
        search_glass(cv, 7.7, y + 0.3, 13.3, ITEM_C);
        let (t, c) = if st.search.is_empty() {
            (w.search.clone(), SEARCH_C)
        } else {
            (st.search.clone(), p.item)
        };
        let t = ctx.fonts.fit(&fnt, &t, (b.w - 40.0).max(0.0));
        base_text(cv, ctx, &t, fnt.clone(), c, 31.0, y + 12.0, Anchor::Start);
        cv.rect(0.3, y + 16.7, (b.w - 0.3).max(0.0), 0.7, RULE_C, 1.0);
        y += 20.7;
    }
    let (cx, tx) = if hierarchy {
        (bx + 21.3, bx + 43.3)
    } else {
        (bx, bx + 22.7)
    };
    let row = |cv: &mut Canvas, y: f32, s: &str, state: u8, act: HitAction| {
        cv.hit(ctx.ix.hit(
            HitShape::Rect {
                x: 0.0,
                y: y - 2.8,
                w: b.w,
                h: pitch,
            },
            act,
            Vec::new(),
        ));
        if hierarchy {
            chevron_right(cv, 11.8, y + 1.8, 4.6, 9.6, EXPAND_C);
        }
        check_box(cv, cx, y, bs, state, p);
        let t = ctx.fonts.fit(&fnt, s, (b.w - tx - 2.0).max(0.0));
        base_text(cv, ctx, &t, fnt.clone(), p.item, tx, y + 12.5, Anchor::Start);
    };
    if st.select_all && st.single != Some(true) {
        if y + bs > b.h {
            return;
        }
        let on = vals.iter().filter(|x| st.is_on(x)).count();
        let state = if on == 0 {
            0
        } else if on >= vals.len() {
            1
        } else {
            2
        };
        row(
            cv,
            y,
            &w.select_all,
            state,
            ctx.ix.slicer("slicer_select_all", None),
        );
        y += pitch;
    }
    for (x, s) in vals.iter().zip(labels) {
        if y + bs > b.h {
            break;
        }
        row(
            cv,
            y,
            s,
            u8::from(st.is_on(x)),
            ctx.ix.slicer("slicer_toggle", Some(x)),
        );
        y += pitch;
    }
}

// --- classic tiles (horizontal list) --------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn tiles(cv: &mut Canvas, ctx: &Ctx<'_>, b: Bx, top: f32, st: &State, vals: &[Value], labels: &[String]) {
    let p = pal(ctx);
    let fnt = Font::new(&ctx.font, 10.0 * PT);
    let (x0, x1) = (18.5f32, (b.w - 17.5).max(19.5));
    let y0 = top + 2.0;
    let th = (b.h - y0 + 1.0).max(8.0);
    // as many tiles as fit at 60 px or more each
    let n = vals.len().clamp(1, (((x1 - x0) / 60.0).floor() as usize).max(1));
    let pitch = (x1 - x0) / n as f32;
    for (k, (x, s)) in vals.iter().zip(labels).take(n).enumerate() {
        let tx = x0 + k as f32 * pitch;
        let on = st.is_on(x);
        if on {
            cv.rect(tx + 0.8, y0, pitch - 1.6, th, TILE_ON_C, 1.0);
        }
        let shape = HitShape::Rect {
            x: tx + 0.8,
            y: y0,
            w: pitch - 1.6,
            h: th,
        };
        cv.hit(
            ctx.ix
                .hit(shape, ctx.ix.slicer("slicer_toggle", Some(x)), Vec::new()),
        );
        let t = ctx.fonts.fit(&fnt, s, (pitch - 12.0).max(0.0));
        let c = if on { Rgb::WHITE } else { p.item };
        base_text(
            cv,
            ctx,
            &t,
            fnt.clone(),
            c,
            tx + pitch / 2.0,
            y0 + th / 2.0 + 4.7,
            Anchor::Middle,
        );
    }
}

// --- dropdown -----------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn dropdown(cv: &mut Canvas, ctx: &Ctx<'_>, b: Bx, top: f32, st: &State, vals: &[Value], labels: &[String]) {
    let p = pal(ctx);
    let w = words(ctx);
    let picked: Vec<&String> = vals
        .iter()
        .zip(labels)
        .filter(|(x, _)| st.is_on(x))
        .map(|(_, s)| s)
        .collect();
    let text = if st.sel.is_empty() {
        w.all.clone()
    } else if !st.inverted && st.sel.len() == 1 {
        match picked.first() {
            Some(s) => (*s).clone(),
            None => st
                .sel_raw
                .first()
                .map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string()))
                .unwrap_or_default(),
        }
    } else {
        w.multiple.clone()
    };
    let (x0, y0, bw, bh) = (5.7f32, top + inset(top), (b.w - 10.7).max(20.0), 26.6f32);
    cv.round_rect(
        x0 + 0.5,
        y0 + 0.5,
        bw - 1.0,
        bh - 1.0,
        0.0,
        None,
        Some((INPUT_BORDER_C, 1.0, None)),
    );
    let fnt = Font::new(&ctx.font, 10.0 * PT);
    let t = ctx.fonts.fit(&fnt, &text, (bw - 34.0).max(0.0));
    base_text(cv, ctx, &t, fnt, p.item, x0 + 6.0, y0 + 20.6, Anchor::Start);
    chevron_down(cv, x0 + bw - 18.3, y0 + 11.6, 12.0, 6.0, CHEVRON_C, 1.0);
    // the list of items with their state, "Select all" first (the viewer's `slDropPopup`)
    if ctx.ix.on() {
        let items: Vec<Value> = vals
            .iter()
            .zip(labels)
            .take(500)
            .map(|(x, s)| json!({"label": s, "value": x, "on": st.is_on(x)}))
            .collect();
        let select_all = (st.select_all && st.single != Some(true))
            .then(|| json!({"label": w.select_all, "on": st.sel.is_empty()}));
        let popup = ctx
            .ix
            .popup("dropdown", json!({"items": items, "select_all": select_all}));
        cv.hit(ctx.ix.hit(
            HitShape::Rect {
                x: x0,
                y: y0,
                w: bw,
                h: bh,
            },
            popup,
            Vec::new(),
        ));
    }
}

// --- between / less or equal / greater or equal --------------------------------------------

#[allow(clippy::too_many_arguments)]
fn range(
    cv: &mut Canvas,
    ctx: &Ctx<'_>,
    b: Bx,
    top: f32,
    st: &State,
    mode: &str,
    vals: &[Value],
    fmt: &dyn Fn(&Value) -> String,
) {
    let p = pal(ctx);
    let mut sorted: Vec<Value> = vals.to_vec();
    sorted.sort_by(cmp_raw);
    // the column's exact bounds when the snapshot gives them, else those of the rows it holds
    let lo_v = match st.domain.get("min") {
        Some(x) if !x.is_null() => x.clone(),
        _ => sorted.first().cloned().unwrap_or(Value::Null),
    };
    let hi_v = match st.domain.get("max") {
        Some(x) if !x.is_null() => x.clone(),
        _ => sorted.last().cloned().unwrap_or(Value::Null),
    };
    // a range held by the filter only is shown as the whole domain (as Desktop does); the
    // exclusive flags of strict filter bounds never reach the display
    let shown = |k: &str| match st.range.get(k) {
        Some(x) if st.range.get("filter_only") != Some(&Value::Bool(true)) => Some(x.clone()),
        _ => None,
    };
    let from = shown("from").unwrap_or_else(|| lo_v.clone());
    let to = shown("to").unwrap_or_else(|| hi_v.clone());
    let fnt = Font::new(&ctx.font, 10.0 * PT);
    let (t0, t1) = (fmt(&from), fmt(&to));
    // two inputs side by side, sized to their text (at least 73 px)
    let gap = 6.7f32;
    let need = ctx.fonts.measure(&fnt, &t0).max(ctx.fonts.measure(&fnt, &t1)) + 16.0;
    let iw = need.max(72.7).min(((b.w - 9.0 - gap) / 2.0).max(10.0));
    let (y0, ih) = (top + inset(top), 26.6f32);
    for (k, t) in [t0, t1].iter().enumerate() {
        let x = 9.0 + k as f32 * (iw + gap);
        cv.round_rect(
            x + 0.5,
            y0 + 0.5,
            iw - 1.0,
            ih - 1.0,
            0.0,
            None,
            Some((INPUT_BORDER_C, 1.0, None)),
        );
        let off = (mode == "before" && k == 0) || (mode == "after" && k == 1);
        let s = ctx.fonts.fit(&fnt, t, (iw - 10.0).max(0.0));
        base_text(
            cv,
            ctx,
            &s,
            fnt.clone(),
            if off { DISABLED_C } else { p.value },
            x + 6.7,
            y0 + 18.0,
            Anchor::Start,
        );
    }
    if ctx.ix.on() {
        let iso = |x: &Value| x.as_str().is_some_and(is_iso_date);
        let is_date = vals.iter().any(iso) || iso(&lo_v);
        let rg = |k: &str| st.range.get(k).cloned().unwrap_or(Value::Null);
        let popup = ctx.ix.popup(
            "range",
            json!({"mode": mode, "is_date": is_date, "from": rg("from"), "to": rg("to")}),
        );
        let shape = HitShape::Rect {
            x: 9.0,
            y: y0,
            w: (b.w - 9.0).max(1.0),
            h: (b.h - y0).clamp(1.0, 60.0),
        };
        cv.hit(ctx.ix.hit(shape, popup, Vec::new()));
    }
    // the slider: a 3 px track between 4 x 14 px handles
    let ty = top + if top > 0.0 { 48.0 } else { 46.3 };
    if ty + 7.0 > b.h {
        return;
    }
    let nv = |x: &Value| {
        x.as_f64()
            .or_else(|| x.as_str().and_then(iso_days))
            .unwrap_or(f64::NAN)
    };
    let (lo, hi) = (nv(&lo_v), nv(&hi_v));
    let frac = |x: &Value, d: f32| {
        let x = nv(x);
        if x.is_finite() && hi > lo {
            ((x - lo) / (hi - lo)).clamp(0.0, 1.0) as f32
        } else {
            d
        }
    };
    let (xa, xb) = (8.3f32, (b.w - 10.0).max(9.0));
    let span = xb - xa;
    let fa = if mode == "before" { 0.0 } else { frac(&from, 0.0) };
    let fb = if mode == "after" { 1.0 } else { frac(&to, 1.0) };
    let (ha, hb) = (xa + fa * span, xa + fb * span);
    if fa > 0.0 || fb < 1.0 {
        cv.rect(xa, ty - 1.5, span, 3.0, ctx.track, 1.0);
    }
    cv.rect(ha, ty - 1.5, (hb - ha).max(0.0), 3.0, ITEM_C, 1.0);
    if mode != "before" {
        cv.round_rect(ha - 2.25, ty - 7.5, 4.5, 15.0, 2.2, Some((ITEM_C, 1.0)), None);
    }
    if mode != "after" {
        cv.round_rect(hb - 2.25, ty - 7.5, 4.5, 15.0, 2.2, Some((ITEM_C, 1.0)), None);
    }
}

// --- relative date ---------------------------------------------------------------------------

fn calendar_icon(cv: &mut Canvas, x: f32, y: f32, c: Rgb) {
    cv.round_rect(x + 0.5, y + 1.5, 10.0, 9.5, 0.5, None, Some((c, 0.8, None)));
    cv.line(x + 0.5, y + 4.0, x + 10.5, y + 4.0, c, 0.8, None);
    cv.line(x + 3.0, y, x + 3.0, y + 2.5, c, 0.8, None);
    cv.line(x + 8.0, y, x + 8.0, y + 2.5, c, 0.8, None);
    cv.circle(x + 9.5, y + 9.5, 3.2, Some((Rgb::WHITE, 1.0)), Some((c, 0.8)));
    cv.line(x + 9.5, y + 8.0, x + 9.5, y + 9.7, c, 0.7, None);
    cv.line(x + 9.5, y + 9.7, x + 10.8, y + 9.7, c, 0.7, None);
}

/// "Next" / "This" in the UI language (`ui.strings` first).
fn range_word(ctx: &Ctx<'_>, r: &str, fr: bool) -> Option<String> {
    let (key, en, frs) = match r {
        "next" => ("relative_next", "Next", "Suivant"),
        "this" => ("relative_this", "This", "Ce"),
        _ => return None,
    };
    Some(ctx.t(key, if fr { frs } else { en }))
}

/// The period list's entry for a snapshot `period` / `time_period` ("calendar months" or
/// "calendarmonths" both read).
fn period_word(p: &str, fr: bool, this: bool) -> Option<&'static str> {
    let k: String = p
        .chars()
        .filter(|c| !c.is_whitespace())
        .take(40)
        .collect::<String>()
        .to_ascii_lowercase();
    const T: [(&str, &str, &str); 9] = [
        ("minutes", "Minutes", "Minutes"),
        ("hours", "Hours", "Heures"),
        ("days", "Days", "Jours"),
        ("weeks", "Weeks", "Semaines"),
        ("calendarweeks", "Weeks (Calendar)", "Semaines (calendaires)"),
        ("months", "Months", "Mois"),
        ("calendarmonths", "Months (Calendar)", "Mois (calendaires)"),
        ("years", "Years", "Années"),
        ("calendaryears", "Years (Calendar)", "Années (calendaires)"),
    ];
    // after "This" the period is singular ("This week")
    const ONE: [(&str, &str, &str); 6] = [
        ("minutes", "Minute", "Minute"),
        ("hours", "Hour", "Heure"),
        ("days", "Day", "Jour"),
        ("weeks", "Week", "Semaine"),
        ("months", "Month", "Mois"),
        ("years", "Year", "Année"),
    ];
    let first: &[(&str, &str, &str)] = if this { &ONE } else { &[] };
    first
        .iter()
        .chain(T.iter())
        .find(|(key, _, _)| *key == k)
        .map(|(_, en, frs)| if fr { *frs } else { *en })
}

/// `rel`: the snapshot's `relative` settings (`{range, duration, period}`), absent before one is
/// chosen (Desktop then shows "Last", 1 and an empty period).
fn relative(cv: &mut Canvas, ctx: &Ctx<'_>, b: Bx, top: f32, rel: &Value, fmt: &dyn Fn(&Value) -> String) {
    let p = pal(ctx);
    let w = words(ctx);
    let fnt = Font::new(&ctx.font, 9.0 * PT);
    let (y0, ih) = (top + inset(top), 25.3f32);
    let (x0, x1) = (9.0f32, (b.w - 8.3).max(40.0));
    let total = x1 - x0;
    let cap = |t: &str| {
        let mut c = t.chars();
        c.next()
            .map(|f| f.to_uppercase().chain(c).collect::<String>())
            .unwrap_or_default()
    };
    let range = match sk(rel, "range") {
        "" | "last" => w.last.clone(),
        r => range_word(ctx, r, w.fr).unwrap_or_else(|| cap(&r.chars().take(40).collect::<String>())),
    };
    let duration = match get(rel, "duration").as_f64() {
        Some(d) if d.is_finite() => format!("{}", d.clamp(-1e9, 1e9).round() as i64),
        _ => "1".to_string(),
    };
    let period = match (sk(rel, "period"), sk(rel, "time_period")) {
        ("", "") => w.select.clone(),
        ("", t) | (t, _) => period_word(t, w.fr, sk(rel, "range") == "this")
            .map_or_else(|| cap(&t.chars().take(40).collect::<String>()), str::to_string),
    };
    let set = rel.is_object();
    let boxes = [
        (x0, total * 0.2917, range.as_str(), true),
        (x0 + total * 0.311, total * 0.1825, duration.as_str(), false),
        (x0 + total * 0.51, total * 0.49, period.as_str(), true),
    ];
    for (bx, bw, t, chev) in boxes {
        cv.round_rect(
            bx + 0.5,
            y0 + 0.5,
            bw - 1.0,
            ih - 1.0,
            0.0,
            None,
            Some((INPUT_BORDER_C, 1.0, None)),
        );
        let s = ctx.fonts.fit(&fnt, t, (bw - 30.0).max(0.0));
        base_text(
            cv,
            ctx,
            &s,
            fnt.clone(),
            p.value,
            bx + 6.7,
            y0 + 16.7,
            Anchor::Start,
        );
        if chev {
            chevron_down(cv, bx + bw - 17.0, y0 + 10.7, 8.0, 4.0, p.value, 0.8);
        }
    }
    if ctx.ix.on() {
        let popup = ctx.ix.popup(
            "relative",
            json!({"range": get(rel, "range"), "duration": get(rel, "duration"), "period": get(rel, "period")}),
        );
        cv.hit(ctx.ix.hit(
            HitShape::Rect {
                x: x0,
                y: y0,
                w: total,
                h: ih,
            },
            popup,
            Vec::new(),
        ));
    }
    let y1 = y0 + ih + 10.7;
    // the days the settings cover (`relative.from` / `to`, resolved against today by the
    // snapshot), else "No active filter"
    let covered = match (get(rel, "from"), get(rel, "to")) {
        (a, b) if a.is_string() && b.is_string() => format!("{} - {}", fmt(a), fmt(b)),
        _ => String::new(),
    };
    if y1 + 12.0 <= b.h && (!set || !covered.is_empty()) {
        calendar_icon(cv, 9.0, y1, ITEM_C);
        if covered.is_empty() {
            base_text(cv, ctx, &w.no_filter, fnt, p.value, 25.7, y1 + 9.3, Anchor::Start);
        } else {
            let t = ctx.fonts.fit(&fnt, &covered, (b.w - 30.0).max(0.0));
            base_text(cv, ctx, &t, fnt, p.value, 25.7, y1 + 9.3, Anchor::Start);
        }
    }
}

// --- new list slicer (listSlicer) ------------------------------------------------------------

fn new_list(cv: &mut Canvas, ctx: &Ctx<'_>, b: Bx, st: &State, vals: &[Value], labels: &[String]) {
    let fnt = Font::new(&ctx.font, 8.0 * PT);
    let text_c = if ctx.classic { NEW_TEXT_C } else { ctx.fg };
    // single select by default (radio buttons); multi select shows check boxes
    let radio = st.single != Some(false);
    let (pitch, d) = (34.0f32, 16.0f32);
    let mut y = 9.4f32;
    for (x, s) in vals.iter().zip(labels) {
        if y + d > b.h {
            break;
        }
        let shape = HitShape::Rect {
            x: 0.0,
            y: y - 9.0,
            w: b.w,
            h: pitch,
        };
        cv.hit(
            ctx.ix
                .hit(shape, ctx.ix.slicer("slicer_toggle", Some(x)), Vec::new()),
        );
        let on = st.is_on(x);
        if radio {
            cv.circle(
                1.0 + d / 2.0,
                y + d / 2.0,
                d / 2.0 - 0.5,
                None,
                Some((text_c, 1.0)),
            );
            if on {
                cv.circle(1.0 + d / 2.0, y + d / 2.0, d / 4.0, Some((text_c, 1.0)), None);
            }
        } else if on {
            cv.round_rect(1.0, y, d, d, 2.0, Some((text_c, 1.0)), None);
        } else {
            cv.round_rect(
                1.5,
                y + 0.5,
                d - 1.0,
                d - 1.0,
                2.0,
                None,
                Some((text_c, 1.0, None)),
            );
        }
        let t = ctx.fonts.fit(&fnt, s, (b.w - 30.0).max(0.0));
        base_text(
            cv,
            ctx,
            &t,
            fnt.clone(),
            text_c,
            25.0,
            y + d / 2.0 + 4.7,
            Anchor::Start,
        );
        y += pitch;
    }
}

// --- button slicer (advancedSlicerVisual) ----------------------------------------------------

fn buttons(cv: &mut Canvas, ctx: &Ctx<'_>, b: Bx, st: &State, vals: &[Value], labels: &[String]) {
    let fnt = Font::new(&ctx.font, 8.0 * PT).weight(600);
    let text_c = if ctx.classic { NEW_TEXT_C } else { ctx.fg };
    let n = vals.len().max(1);
    let gap = 8.0f32;
    let cols = n.min(((b.w + gap) / 100.0).floor().max(1.0) as usize).max(1);
    let rows = n.div_ceil(cols);
    let bw = ((b.w + gap) / cols as f32 - gap).max(4.0);
    let bh = ((b.h + gap) / rows as f32 - gap).max(4.0);
    for (k, (x, s)) in vals.iter().zip(labels).enumerate().take(MAX_ITEMS) {
        let (cx, cy) = ((k % cols) as f32 * (bw + gap), (k / cols) as f32 * (bh + gap));
        if cy >= b.h {
            break;
        }
        let shape = HitShape::Rect {
            x: cx,
            y: cy,
            w: bw,
            h: bh,
        };
        cv.hit(
            ctx.ix
                .hit(shape, ctx.ix.slicer("slicer_toggle", Some(x)), Vec::new()),
        );
        let on = st.is_on(x);
        let fill = if on { Some((TILE_ON_C, 1.0)) } else { None };
        cv.round_rect(
            cx + 0.5,
            cy + 0.5,
            bw - 1.0,
            bh - 1.0,
            0.0,
            fill,
            Some((BUTTON_BORDER_C, 1.0, None)),
        );
        let t = ctx.fonts.fit(&fnt, s, (bw - 26.0).max(0.0));
        let c = if on { Rgb::WHITE } else { text_c };
        base_text(
            cv,
            ctx,
            &t,
            fnt.clone(),
            c,
            cx + 13.0,
            cy + bh / 2.0 + 3.7,
            Anchor::Start,
        );
    }
}

// --- text slicer (textSlicer) ------------------------------------------------------------------

fn arrow_right(cv: &mut Canvas, x: f32, y: f32, w: f32, c: Rgb) {
    let h = w * 0.6;
    cv.line(x, y, x + w, y, c, 1.0, None);
    if let Some(path) = poly(
        &[
            (x + w - h / 2.0, y - h / 2.0),
            (x + w, y),
            (x + w - h / 2.0, y + h / 2.0),
        ],
        false,
    ) {
        cv.stroke(&path, c, 1.0, 1.0, None);
    }
}

fn text_slicer(cv: &mut Canvas, ctx: &Ctx<'_>, b: Bx, st: &State, name: &str) {
    let w = words(ctx);
    let fnt = Font::new(&ctx.font, 10.0 * PT);
    cv.round_rect(
        0.5,
        0.5,
        (b.w - 1.0).max(1.0),
        (b.h - 1.0).max(1.0),
        0.0,
        None,
        Some((RULE_C, 1.0, None)),
    );
    let (y0, h) = (4.4f32, 26.0f32);
    if y0 + h > b.h + 4.0 {
        return;
    }
    // the drop-down button (match kind), the field, the clear cross and the apply arrow
    cv.rect(9.7, y0, 23.3, h, FIELD_C, 1.0);
    chevron_down(cv, 17.3, y0 + 11.0, 8.0, 4.4, TILE_ON_C, 1.6);
    let (fx, fr) = (34.6f32, (b.w - 34.6).max(40.0));
    let popup = ctx.ix.popup("text", json!({"text": st.search}));
    cv.hit(ctx.ix.hit(
        HitShape::Rect {
            x: 0.0,
            y: 0.0,
            w: b.w,
            h: b.h.max(1.0),
        },
        popup,
        Vec::new(),
    ));
    let typed = !st.search.is_empty();
    if typed {
        cv.rect(fx, y0, fr - fx, h, FIELD_C, 1.0);
        let cx = fr - 11.0;
        cv.line(cx - 4.5, y0 + 8.5, cx + 4.5, y0 + 17.5, ITEM_C, 1.0, None);
        cv.line(cx - 4.5, y0 + 17.5, cx + 4.5, y0 + 8.5, ITEM_C, 1.0, None);
    }
    let (t, c) = if typed {
        (st.search.clone(), TILE_ON_C)
    } else {
        (format!("{} {}", w.enter, name), ITEM_C)
    };
    let t = ctx.fonts.fit(&fnt, &t, (fr - fx - 30.0).max(0.0));
    base_text(cv, ctx, &t, fnt, c, fx + 7.0, y0 + 18.0, Anchor::Start);
    arrow_right(cv, (b.w - 31.0).max(fr), y0 + 12.0, 20.0, ARROW_C);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FontBook;
    use crate::data::{dataset, vis_format};
    use serde_json::json;

    fn render(v: &Value, w: f32, h: f32) -> Canvas {
        let fonts = FontBook::bundled_only();
        let snap = json!({"report": {"theme": null, "pages": []}});
        let ctx = Ctx::new(&snap, &fonts, None);
        let kind = match sk(v, "type") {
            "listSlicer" => "list",
            "advancedSlicerVisual" => "tiles",
            "textSlicer" => "text",
            _ => "slicer",
        };
        let ds = dataset(v, "slicer", &ctx);
        let f = vis_format(v, "slicer", &ctx);
        let mut cv = Canvas::new(w, h, 1.0).unwrap();
        slicer_v2(&mut cv, &ds, Bx { w, h }, &f, &ctx, v, kind);
        cv
    }

    fn visual(ty: &str, col: Value, rows: Value, slicer: Value) -> Value {
        json!({"type": ty, "name": "s", "x": 0, "y": 0, "width": 240, "height": 200,
               "fields": [{"role": "Values", "kind": "column", "table": "T", "name": col["name"]}],
               "data": {"status": "ok", "columns": [col], "rows": rows}, "slicer": slicer})
    }

    fn text_col() -> Value {
        json!({"name": "Region", "role": "Values", "kind": "column", "data_type": "string"})
    }

    fn regions() -> Value {
        json!([["North"], ["South"], ["East"]])
    }

    /// Darkest grey in a rectangle (0 black .. 255 white over white).
    fn darkest(cv: &Canvas, x0: f32, y0: f32, x1: f32, y1: f32) -> u8 {
        let mut d = 255u8;
        for y in (y0 as u32)..(y1 as u32) {
            for x in (x0 as u32)..(x1 as u32) {
                if let Some(p) = cv.pm.pixel(x, y) {
                    // premultiplied over white
                    let a = p.alpha();
                    let g = ((p.red() as u32 + p.green() as u32 + p.blue() as u32) / 3) as u8;
                    let over = 255u32.saturating_sub(a as u32) + g as u32;
                    d = d.min(over.min(255) as u8);
                }
            }
        }
        d
    }

    #[test]
    fn list_boxes_show_the_selection() {
        let v = visual(
            "slicer",
            text_col(),
            regions(),
            json!({"mode": "list", "selected": ["South"]}),
        );
        let cv = render(&v, 230.0, 190.0);
        // header ink, an empty box (row 1: only its border), a filled box (row 2)
        assert!(darkest(&cv, 9.0, 3.0, 60.0, 16.0) < 80);
        assert!(
            darkest(&cv, 10.0, 25.0, 19.0, 33.0) > 200,
            "row 1 box is empty inside"
        );
        assert!(darkest(&cv, 10.0, 44.0, 19.0, 52.0) < 60, "row 2 box is filled");
    }

    #[test]
    fn select_all_and_search_rows() {
        let v = visual(
            "slicer",
            text_col(),
            regions(),
            json!({"mode": "list", "selected": ["North"], "select_all": true, "search": "o", "match": "contains"}),
        );
        let cv = render(&v, 230.0, 190.0);
        // the search rule under the search row
        assert!(darkest(&cv, 100.0, 38.0, 200.0, 41.0) < 230);
        // "Select all" is partial: a light grey square inside its box
        let inner = darkest(&cv, 11.0, 47.0, 18.0, 54.0);
        assert!(inner > 150 && inner < 230, "partial select-all box: {inner}");
    }

    #[test]
    fn dropdown_texts() {
        for (sel, inv) in [
            (json!([]), false),
            (json!(["North"]), false),
            (json!(["North", "East"]), true),
        ] {
            let v = visual(
                "slicer",
                text_col(),
                regions(),
                json!({"mode": "dropdown", "selected": sel, "inverted": inv}),
            );
            let cv = render(&v, 230.0, 60.0);
            assert!(darkest(&cv, 10.0, 30.0, 120.0, 48.0) < 160, "dropdown text drawn");
            assert!(darkest(&cv, 205.0, 33.0, 220.0, 42.0) < 200, "chevron drawn");
        }
    }

    #[test]
    fn range_modes_draw_inputs_and_handles() {
        let col = json!({"name": "Qty", "role": "Values", "kind": "column", "data_type": "int64"});
        let rows = json!([[3], [9], [25]]);
        for (mode, left_handle, right_handle) in [
            ("between", true, true),
            ("before", false, true),
            ("after", true, false),
        ] {
            let v = visual("slicer", col.clone(), rows.clone(), json!({"mode": mode}));
            let cv = render(&v, 290.0, 110.0);
            assert_eq!(
                darkest(&cv, 6.0, 58.0, 11.0, 62.0) < 150,
                left_handle,
                "{mode} left"
            );
            assert_eq!(
                darkest(&cv, 278.0, 58.0, 283.0, 62.0) < 150,
                right_handle,
                "{mode} right"
            );
        }
        // a selected range moves the handles inward
        let v = visual(
            "slicer",
            col,
            rows,
            json!({"mode": "between", "range": {"from": 9, "to": 20}}),
        );
        let cv = render(&v, 290.0, 110.0);
        assert!(
            darkest(&cv, 6.0, 58.0, 11.0, 63.0) > 200,
            "no handle at the start"
        );
    }

    #[test]
    fn a_date_slicer_without_a_mode_is_a_range() {
        // Power BI Desktop 2.159.1231.0 (2026-11-24) drew a slicer on a date and on a date-time
        // column, written without a mode, as the between range (two date inputs and a slider)
        for (ty, fmt, rows) in [
            (
                "dateTime",
                "Long Date",
                json!([
                    ["2026-01-05T00:00:00"],
                    ["2026-03-02T00:00:00"],
                    ["2026-06-01T00:00:00"]
                ]),
            ),
            (
                "dateTime",
                "General Date",
                json!([["2026-01-05T00:00:00"], ["2026-01-06T14:30:00"]]),
            ),
        ] {
            let col = json!({"name": "Day", "role": "Values", "kind": "column", "data_type": ty, "format_string": fmt});
            let v = visual("slicer", col, rows, json!({}));
            let cv = render(&v, 290.0, 150.0);
            assert!(
                darkest(&cv, 6.0, 58.0, 11.0, 62.0) < 150,
                "{fmt}: the left handle"
            );
        }
    }

    #[test]
    fn tiles_relative_and_new_slicers_draw() {
        let cases = [
            (
                "slicer",
                json!({"mode": "tile", "selected": ["East"]}),
                590.0,
                70.0,
            ),
            ("slicer", json!({"mode": "relative"}), 390.0, 110.0),
            (
                "listSlicer",
                json!({"mode": "list", "selected": ["North"]}),
                270.0,
                270.0,
            ),
            ("advancedSlicerVisual", json!({"mode": "tile"}), 590.0, 170.0),
            (
                "textSlicer",
                json!({"mode": "text", "search": "or", "match": "contains"}),
                290.0,
                50.0,
            ),
            ("textSlicer", json!({"mode": "text"}), 290.0, 50.0),
        ];
        for (ty, sl, w, h) in cases {
            let v = visual(ty, text_col(), regions(), sl);
            let cv = render(&v, w, h);
            assert!(darkest(&cv, 0.0, 0.0, w, h) < 120, "{ty} draws");
        }
    }

    #[test]
    fn header_text_replaces_the_field_name() {
        let v = visual(
            "slicer",
            text_col(),
            regions(),
            json!({"mode": "dropdown", "header_text": "Area"}),
        );
        let cv = render(&v, 230.0, 60.0);
        let t: Vec<&str> = cv.texts.iter().map(|r| r.text.as_str()).collect();
        assert!(t.contains(&"Area") && !t.contains(&"Region"), "{t:?}");
    }

    #[test]
    fn snapshot_settings_are_used() {
        // header switched off: the first row moves up to the top
        let v = visual(
            "slicer",
            text_col(),
            regions(),
            json!({"mode": "list", "header_show": false}),
        );
        let cv = render(&v, 230.0, 190.0);
        assert!(
            darkest(&cv, 9.0, 3.0, 60.0, 16.0) < 120,
            "first row drawn where the header was"
        );
        // relative settings fill the boxes and drop the "no filter" line
        let col = json!({"name": "Day", "role": "Values", "kind": "column", "data_type": "dateTime"});
        let rows = json!([["2025-01-01"], ["2025-02-01"]]);
        let rel =
            json!({"mode": "relative", "relative": {"range": "next", "duration": 3, "period": "months"}});
        let cv = render(&visual("slicer", col.clone(), rows.clone(), rel), 390.0, 110.0);
        assert!(darkest(&cv, 9.0, 60.0, 200.0, 75.0) > 240, "no 'no filter' line");
        // the domain sets the slider ends: a range starting at the domain minimum has its handle
        // at the start even though the rows begin later
        let dom = json!({"mode": "between", "range": {"from": "2024-01-01", "to": "2025-02-01"},
                         "domain": {"min": "2024-01-01", "max": "2025-02-01"}});
        let cv = render(&visual("slicer", col, rows, dom), 290.0, 110.0);
        assert!(
            darkest(&cv, 6.0, 58.0, 11.0, 62.0) < 150,
            "handle at the domain start"
        );
    }

    #[test]
    fn relative_covered_days_and_filter_only_ranges() {
        let col = json!({"name": "Day", "role": "Values", "kind": "column", "data_type": "dateTime"});
        let rows = json!([["2025-01-01"], ["2025-02-01"]]);
        // the covered days replace "No active filter" under the inputs
        let rel = json!({"mode": "relative", "relative": {"range": "last", "duration": 7, "period": "days",
                         "from": "2026-10-08", "to": "2026-10-14"}});
        let cv = render(&visual("slicer", col.clone(), rows.clone(), rel), 390.0, 110.0);
        assert!(darkest(&cv, 25.0, 60.0, 150.0, 75.0) < 120, "covered days drawn");
        assert_eq!(
            period_word("Calendar Years", true, false),
            Some("Années (calendaires)")
        );
        assert_eq!(period_word("months", false, false), Some("Months"));
        assert_eq!(period_word("weeks", true, true), Some("Semaine"));
        assert_eq!(period_word("fortnights", false, false), None);
        // a range held by the filter only shows the whole domain: both handles at the ends
        let rg = json!({"mode": "between", "range": {"from": "2025-01-15", "filter_only": true},
                        "domain": {"min": "2025-01-01", "max": "2025-02-01"}});
        let cv = render(&visual("slicer", col, rows, rg), 290.0, 110.0);
        assert!(
            darkest(&cv, 6.0, 58.0, 11.0, 62.0) < 150,
            "handle at the domain start"
        );
    }

    #[test]
    fn hierarchy_slicer_shows_parents_with_chevrons() {
        let mut v = visual(
            "slicer",
            text_col(),
            json!([["A", "x"], ["A", "y"], ["B", "x"]]),
            json!({"mode": "list"}),
        );
        v["data"]["columns"] =
            json!([text_col(), {"name": "Sub", "role": "Values", "kind": "column", "data_type": "string"}]);
        let cv = render(&v, 230.0, 190.0);
        assert!(darkest(&cv, 11.0, 24.0, 16.0, 34.0) < 200, "chevron");
        assert!(darkest(&cv, 11.0, 62.0, 16.0, 72.0) > 240, "two parents only");
    }

    #[test]
    fn hostile_slicers_never_panic() {
        let long = "x".repeat(50_000);
        let rows: Vec<Value> = (0..2000).map(|i| json!([format!("v{i}")])).collect();
        let cases = [
            json!({"mode": "list", "selected": [null, 1, {"a": 1}], "search": long, "select_all": true}),
            json!({"mode": "between", "range": {"from": "zz", "to": [1]}}),
            json!({"mode": "before", "range": 5}),
            json!({"mode": "tile", "selected": rows.clone()}),
            json!({"mode": "nonsense"}),
            json!(null),
        ];
        for sl in cases {
            for ty in ["slicer", "listSlicer", "advancedSlicerVisual", "textSlicer"] {
                let v = visual(ty, text_col(), Value::Array(rows.clone()), sl.clone());
                for (w, h) in [(230.0, 190.0), (4.0, 4.0), (2000.0, 3.0)] {
                    let _ = render(&v, w, h);
                }
            }
        }
        // numbers: NaN-like and huge
        let col = json!({"name": "Amount", "role": "Values", "kind": "column", "data_type": "double"});
        let v = visual(
            "slicer",
            col,
            json!([[1e308], [-1e308], [null], ["x"]]),
            json!({"mode": "between"}),
        );
        let _ = render(&v, 290.0, 110.0);
    }
}
