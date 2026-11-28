//! Non-chart bodies: cards (classic and new), multi-row cards, text boxes, buttons, shapes,
//! image placeholders and page navigators (slicers: `slicers.rs`). Ported from the live viewer's
//! HTML bodies (`cardBody`, `textBody`…), laid out by hand.

use serde_json::Value;

use crate::canvas::{Anchor, Base, Canvas, Rgb, Text, svg_path};
use crate::charts::{Bx, fit, font};
use crate::data::{Col, Ctx, Dataset, Fmt, PT, Rc, arr, clampf, fin, get, is_num_col, nk, safe_font, sk};
use crate::fmt::fmt_cell;
use crate::fonts::Font;

fn line_text(
    cv: &mut Canvas,
    ctx: &Ctx<'_>,
    s: &str,
    f: Font,
    color: Rgb,
    x: f32,
    y: f32,
    anchor: Anchor,
) -> f32 {
    cv.text(
        ctx.fonts,
        &Text::new(s, f, color).anchor(anchor).base(Base::Central),
        x,
        y,
    )
}

// --- cards -----------------------------------------------------------------------------------

/// Power BI's classic card (`card`): the first measure's value over its name. The callout's
/// display units and decimals (`labels.labelDisplayUnits` / `labelPrecision`, snapshot
/// `format.labels`) apply as on data labels ([`crate::fmt::fmt_label`]); auto (0 or unset) with no
/// decimals keeps the abbreviated value ("910K").
pub fn card_body(cv: &mut Canvas, ds: &Dataset, b: Bx, f: &Fmt, ctx: &Ctx<'_>) {
    let lo = get(&f.raw, "labels");
    let units = fin(get(lo, "display_units")).filter(|u| *u != 0.0);
    let prec = fin(get(lo, "precision")).map(|p| p.clamp(0.0, 10.0) as usize);
    let (cols, rows) = (&ds.cols, &ds.rows);
    let vi = cols
        .iter()
        .enumerate()
        .position(|(i, c)| is_num_col(c, rows, i) && c.rc != crate::data::Rc::Tip)
        .unwrap_or(0);
    let cf = ds.styles.get(&(0, vi));
    let col = cols.get(vi);
    let raw = rows
        .first()
        .and_then(|r| r.get(vi))
        .cloned()
        .unwrap_or(Value::Null);
    let text = if raw.is_null() {
        ctx.blank()
    } else {
        let cf = col.map(|c| c.cf());
        let t = match raw.as_f64() {
            Some(x) if units.is_some() || prec.is_some() => {
                crate::fmt::fmt_label(x, cf.as_ref(), &ctx.loc, units, prec)
            }
            _ => fmt_cell(&raw, cf.as_ref(), &ctx.loc, true),
        };
        if t.is_empty() { ctx.blank() } else { t }
    };
    let label = col.map(|c| c.name.clone()).unwrap_or_default();
    // Desktop's defaults: callout 27 pt (36 px), category label 12 pt (16 px)
    let lpx = clampf(b.h * 0.15, 8.0, 16.0);
    let show_l = !label.is_empty() && b.h > lpx * 3.0;
    let vfont = Font::new(&format!("DIN, {}", ctx.font), 10.0);
    let width_at_10 = ctx.fonts.measure(&vfont, &text).max(1.0) / 10.0;
    let vpx = clampf(
        ((b.h - if show_l { lpx * 1.5 } else { 0.0 }) * 0.78).min(b.w * 0.92 / width_at_10),
        6.0,
        36.0,
    );
    let block = vpx * 1.1 + if show_l { lpx * 1.25 + lpx * 0.25 } else { 0.0 };
    let top = (b.h - block) / 2.0;
    let fg = cf.and_then(|c| c.fg).unwrap_or(ctx.fg);
    line_text(
        cv,
        ctx,
        &text,
        vfont.size(vpx),
        fg,
        b.w / 2.0,
        top + vpx * 0.55,
        Anchor::Middle,
    );
    if show_l {
        let t = fit(ctx, &label, b.w, lpx);
        line_text(
            cv,
            ctx,
            &t,
            font(ctx, lpx),
            cf.and_then(|c| c.cat).unwrap_or(ctx.muted),
            b.w / 2.0,
            top + vpx * 1.1 + lpx * 0.25 + lpx * 0.625,
            Anchor::Middle,
        );
    }
}

/// One new-card tile: label and value as Desktop 2.158 stacks them (two line boxes, 8 px apart in
/// the classic look, 2 px with a theme: Fluent's `spacing`), centred in the tile.
struct TileText<'a> {
    label: &'a str,
    value: &'a str,
    lpx: f32,
    vpx: f32,
    lcolor: Rgb,
    vcolor: Rgb,
    vweight: u16,
    label_below: bool,
    show_label: bool,
    show_value: bool,
    lalign: &'a str,
    valign: &'a str,
}

fn card_tile(cv: &mut Canvas, ctx: &Ctx<'_>, t: &TileText<'_>, x: f32, y: f32, w: f32, h: f32) {
    let inner = (w - 26.6).max(4.0);
    // Desktop truncates a long value with an ellipsis, at its size
    let vf = font(ctx, t.vpx).weight(t.vweight);
    let vpx = t.vpx;
    let lpx = t.lpx;
    let (first, second) = if t.label_below { (vpx, lpx) } else { (lpx, vpx) };
    let (first_on, second_on) = if t.label_below {
        (t.show_value, t.show_label)
    } else {
        (t.show_label, t.show_value)
    };
    // Segoe UI line boxes (1.33 em, baseline 1.06 em down) `gap` apart, centred in the tile
    let gap = if ctx.classic { 8.0 } else { 2.0 };
    let block = match (first_on, second_on) {
        (true, true) => 1.33 * first + gap + 1.33 * second,
        (true, false) => 1.33 * first,
        (false, true) => 1.33 * second,
        (false, false) => 0.0,
    };
    let top = y + (h - block) / 2.0 + if ctx.classic { 2.0 } else { 0.0 };
    let b1 = top + 1.06 * first;
    let b2 = if first_on {
        top + 1.33 * first + gap + 1.06 * second
    } else {
        top + 1.06 * second
    };
    let (lb, vb) = if t.label_below { (b2, b1) } else { (b1, b2) };
    let place = |align: &str| match align {
        "center" => (x + w / 2.0, Anchor::Middle),
        "right" => (x + w - 13.3, Anchor::End),
        _ => (x + 13.3, Anchor::Start),
    };
    if t.show_label {
        let lf = font(ctx, lpx);
        let s = ctx.fonts.fit(&lf, t.label, inner);
        let (lx, la) = place(t.lalign);
        cv.text(
            ctx.fonts,
            &Text::new(&s, lf, t.lcolor).anchor(la).base(Base::Alphabetic),
            lx,
            lb,
        );
    }
    if t.show_value {
        let vf = vf.size(vpx);
        let s = ctx.fonts.fit(&vf, t.value, inner);
        let (vx, va) = place(t.valign);
        cv.text(
            ctx.fonts,
            &Text::new(&s, vf, t.vcolor).anchor(va).base(Base::Alphabetic),
            vx,
            vb,
        );
    }
}

/// The new card (`cardVisual`), Desktop 2.158: one tile per measure, in a row or a column
/// (whichever gives tiles nearer a 3:2 shape, or `layout.orientation`: 0 column, 1 row, 2 grid),
/// label above the value at fixed sizes (classic 18 pt both, themed 12 / 28 pt); with a `Rows`
/// category, a table of three rows a page: the category on the left, one tile per measure.
pub fn card_new_body(cv: &mut Canvas, ds: &Dataset, b: Bx, f: &Fmt, ctx: &Ctx<'_>) {
    let card = get(&f.raw, "card");
    let (vo, lo, ab, lay) = (
        get(card, "value"),
        get(card, "label"),
        get(card, "accent_bar"),
        get(card, "layout"),
    );
    let (cols, rows) = (&ds.cols, &ds.rows);
    let vi: Vec<usize> = (0..cols.len())
        .filter(|i| {
            cols.get(*i)
                .is_some_and(|c| is_num_col(c, rows, *i) && c.rc != Rc::Tip)
        })
        .take(12)
        .collect();
    if vi.is_empty() {
        return card_body(cv, ds, b, f, ctx);
    }
    let ci = (0..cols.len()).find(|i| !vi.contains(i) && cols.get(*i).is_some_and(|c| c.rc == Rc::Cat));
    let pt = |v: &Value, d: f32| fin(get(v, "font_size")).map_or(d, |s| clampf(s as f32, 5.0, 120.0) * PT);
    let (dl, dv) = if ctx.classic {
        (24.0, 24.0)
    } else {
        (16.0, 28.0 * PT)
    };
    let units = fin(get(vo, "display_units")).filter(|u| *u != 0.0);
    let prec = fin(get(vo, "precision")).map(|p| p.clamp(0.0, 10.0) as usize);
    let blank_as = get(vo, "blank_as").as_str().map(str::to_string);
    let fmt_v = |raw: &Value, c: &Col| -> String {
        if raw.is_null() {
            return blank_as.clone().unwrap_or_else(|| ctx.blank());
        }
        match raw.as_f64() {
            Some(x) if units.is_some() || prec.is_some() => {
                crate::fmt::fmt_label(x, Some(&c.cf()), &ctx.loc, units, prec)
            }
            // display units "auto", like Desktop's new card ("910K")
            _ => fmt_cell(raw, Some(&c.cf()), &ctx.loc, true),
        }
    };
    let label_text = sk(lo, "text").to_string();
    let draw_tile = |cv: &mut Canvas, label: &str, value: &str, x: f32, y: f32, w: f32, h: f32| {
        let t = TileText {
            label,
            value,
            lpx: pt(lo, dl),
            vpx: pt(vo, dv),
            lcolor: Rgb::from_value(get(lo, "color")).unwrap_or(if ctx.classic {
                Rgb(0x66, 0x66, 0x66)
            } else {
                ctx.muted
            }),
            vcolor: Rgb::from_value(get(vo, "color")).unwrap_or(ctx.fg),
            vweight: if get(vo, "bold").as_bool() == Some(false) {
                400
            } else if ctx.classic {
                700
            } else {
                600
            },
            label_below: sk(lo, "position") == "belowvalue",
            show_label: get(lo, "show").as_bool() != Some(false),
            show_value: get(vo, "show").as_bool() != Some(false),
            lalign: sk(lo, "alignment"),
            valign: sk(vo, "alignment"),
        };
        card_tile(cv, ctx, &t, x, y, w, h);
    };
    let accent = |cv: &mut Canvas, x: f32, y: f32, w: f32, h: f32| {
        if get(ab, "show").as_bool() != Some(true) {
            return;
        }
        let c = Rgb::from_value(get(ab, "color")).unwrap_or(if ctx.classic { ctx.fg } else { ctx.accent });
        let bw = clampf(nk(ab, "width", 2.0) as f32, 1.0, 40.0);
        let alpha = 1.0 - clampf(nk(ab, "transparency", 0.0) as f32 / 100.0, 0.0, 1.0);
        match sk(ab, "position") {
            "right" => cv.rect(x + w - bw, y, bw, h, c, alpha),
            "top" => cv.rect(x, y, w, bw, c, alpha),
            "bottom" => cv.rect(x, y + h - bw, w, bw, c, alpha),
            _ => cv.rect(x, y, bw, h, c, alpha),
        }
    };
    let name_of = |i: usize| {
        if !label_text.is_empty() && vi.len() == 1 {
            label_text.clone()
        } else {
            cols.get(i).map(|c| c.name.clone()).unwrap_or_default()
        }
    };
    if let Some(ci) = ci {
        // table: three rows a page under a 21.7 px band, the category in a bold left column
        let n = rows.len().min(200);
        let shown = n.clamp(1, 3);
        let (top, area) = (21.7f32, (b.h - 42.0).max(8.0));
        let rh = area / shown as f32;
        let cf = font(ctx, 16.0).weight(600);
        let names: Vec<String> = rows
            .iter()
            .take(shown)
            .map(|r| {
                let t = fmt_cell(
                    r.get(ci).unwrap_or(&Value::Null),
                    cols.get(ci).map(|c| c.cf()).as_ref(),
                    &ctx.loc,
                    false,
                );
                if t.is_empty() { ctx.blank() } else { t }
            })
            .collect();
        let cw = (names
            .iter()
            .map(|t| ctx.fonts.measure(&cf, t))
            .fold(0.0f32, f32::max)
            + 16.0)
            .min(b.w * 0.4);
        let grid = ctx.fg.mix(ctx.bg, 0.1);
        let x1 = cw + 1.0;
        let gap = 9.4;
        let tw = ((b.w - x1) - gap * (vi.len() - 1) as f32) / vi.len() as f32;
        for (k, (r, name)) in rows.iter().zip(&names).enumerate() {
            let y = top + k as f32 * rh;
            cv.round_rect(1.5, y + 0.5, cw, rh, 0.0, None, Some((grid, 1.0, None)));
            let t = ctx.fonts.fit(&cf, name, cw - 12.0);
            line_text(cv, ctx, &t, cf.clone(), ctx.fg, 9.7, y + rh / 2.0, Anchor::Start);
            for (j, i) in vi.iter().enumerate() {
                let Some(c) = cols.get(*i) else { continue };
                let x = x1 + j as f32 * (tw + gap);
                cv.round_rect(x + 0.5, y + 0.5, tw - 1.0, rh, 0.0, None, Some((grid, 1.0, None)));
                accent(cv, x, y, tw, rh);
                // the tile's text is clipped to its row (Desktop cuts an overflowing value)
                if let Some(mut sub) = cv.layer(x, y, tw.max(1.0), rh.max(1.0)) {
                    draw_tile(
                        &mut sub,
                        &name_of(*i),
                        &fmt_v(r.get(*i).unwrap_or(&Value::Null), c),
                        0.0,
                        0.0,
                        tw,
                        rh,
                    );
                    cv.draw(&sub, x, y, 1.0);
                }
            }
        }
        if n > shown {
            let (cx, cy) = (b.w / 2.0 + 2.5, top + area + 10.7);
            cv.polyline(
                &[(cx - 7.5, cy - 3.5), (cx, cy + 3.5), (cx + 7.5, cy - 3.5)],
                ctx.fg,
                1.0,
                1.0,
                None,
            );
        }
        return;
    }
    let empty = Vec::new();
    let r = rows.first().unwrap_or(&empty);
    let n = vi.len();
    let gap = 8.7;
    let o = fin(get(lay, "orientation")).map(|x| x.round() as i64);
    let (wf, hf) = (b.w.max(1.0), b.h.max(1.0));
    let row_aspect = ((wf - gap * (n - 1) as f32) / n as f32).max(1.0) / hf;
    let col_aspect = wf / ((hf - gap * (n - 1) as f32) / n as f32).max(1.0);
    let (ncol, nrow) = match o {
        Some(1) => (n, 1),
        Some(0) => (1, n),
        Some(2) => {
            let c = fin(get(lay, "columns")).map_or(0, |c| c.clamp(1.0, 12.0) as usize);
            let c = if c == 0 {
                (n as f32).sqrt().ceil() as usize
            } else {
                c
            }
            .clamp(1, n);
            (c, n.div_ceil(c))
        }
        // Desktop's auto layout: the 1-D arrangement whose tiles are nearer 3:2
        _ => {
            if n == 1 || (row_aspect * col_aspect).sqrt() >= 1.5 {
                (n, 1)
            } else {
                (1, n)
            }
        }
    };
    let tw = (b.w - gap * ncol.saturating_sub(1) as f32) / ncol as f32;
    let th = (b.h - gap * nrow.saturating_sub(1) as f32) / nrow as f32;
    let border = ctx.fg.mix(ctx.bg, 0.7);
    for (k, i) in vi.iter().enumerate() {
        let Some(c) = cols.get(*i) else { continue };
        let x = (k % ncol) as f32 * (tw + gap);
        let y = (k / ncol) as f32 * (th + gap);
        cv.round_rect(
            x + 0.5,
            y + 0.5,
            tw - 1.0,
            th - 1.0,
            0.0,
            Some((ctx.bg, 1.0)),
            Some((border, 1.0, None)),
        );
        accent(cv, x, y, tw, th);
        draw_tile(
            cv,
            &name_of(*i),
            &fmt_v(r.get(*i).unwrap_or(&Value::Null), c),
            x,
            y,
            tw,
            th,
        );
    }
}

/// Multi-row card, Desktop's defaults: card title 13 pt, values 10 pt in equal columns, labels
/// 9 pt under them, a grey bar on the left of each card, a scroll thumb when cards overflow.
pub fn multi_row_body(cv: &mut Canvas, ds: &Dataset, b: Bx, ctx: &Ctx<'_>) {
    let cols: Vec<_> = ds.cols.iter().take(8).collect();
    let rows = &ds.rows;
    let is_n: Vec<bool> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| is_num_col(c, rows, i))
        .collect();
    let ti = is_n.iter().position(|x| !*x);
    let (tpx, vpx, lpx) = (13.0 * PT, 10.0 * PT, 9.0 * PT);
    let items: Vec<usize> = (0..cols.len()).filter(|i| Some(*i) != ti).collect();
    let x0 = 14.3;
    let per_row = items.len().clamp(1, 3);
    let col_w = (b.w - 12.0) / per_row as f32;
    let item_rows = items.len().div_ceil(per_row).max(1);
    // Desktop's rhythm: values 26 px under the title's centre, labels 16 px under the values,
    // item rows every 41.3 px, the next title 40.7 px after the last label
    let head = if ti.is_some() { 26.0 } else { 0.0 };
    let card_h = head + (item_rows - 1) as f32 * 41.3 + 16.0 + 40.7;
    let first = if ti.is_some() { 13.3 } else { 11.0 };
    let total = rows.len().min(200) as f32 * card_h;
    let label_c = if ctx.classic {
        Rgb(0xa6, 0xa6, 0xa6)
    } else {
        ctx.muted
    };
    let bar_c = ctx.fg.mix(ctx.bg, 0.6);
    let overflow = total - 30.0 > b.h;
    let w_text = if overflow { b.w - 10.0 } else { b.w };
    for (n, r) in rows.iter().take(200).enumerate() {
        let c0 = first + n as f32 * card_h;
        if c0 - 20.0 > b.h {
            break;
        }
        if let Some(ti) = ti {
            let t = fmt_cell(
                r.get(ti).unwrap_or(&Value::Null),
                cols.get(ti).map(|c| c.cf()).as_ref(),
                &ctx.loc,
                false,
            );
            let t = if t.is_empty() { ctx.blank() } else { t };
            let f = font(ctx, tpx).weight(600);
            let s = ctx.fonts.fit(&f, &t, w_text - x0);
            line_text(cv, ctx, &s, f, ctx.muted, x0, c0, Anchor::Start);
        }
        let mut last = c0;
        for (k, i) in items.iter().enumerate() {
            let Some(c) = cols.get(*i) else { continue };
            let vy = c0 + head + (k / per_row) as f32 * 41.3;
            let x = x0 + (k % per_row) as f32 * col_w;
            let t = fmt_cell(
                r.get(*i).unwrap_or(&Value::Null),
                Some(&c.cf()),
                &ctx.loc,
                c.fmt.is_empty(),
            );
            let t = if t.is_empty() { ctx.blank() } else { t };
            let t = fit(ctx, &t, col_w - 6.0, vpx);
            line_text(cv, ctx, &t, font(ctx, vpx), ctx.fg, x, vy, Anchor::Start);
            let lt = fit(ctx, &c.name, col_w - 6.0, lpx);
            line_text(cv, ctx, &lt, font(ctx, lpx), label_c, x, vy + 16.0, Anchor::Start);
            last = vy + 16.0;
        }
        let top = if ti.is_some() { c0 - 11.6 } else { c0 - 9.3 };
        cv.rect(1.0, top, 2.0, (last + 7.0 - top).max(1.0), bar_c, 1.0);
    }
    if overflow && total > 0.0 {
        let h = (b.h / total * b.h).clamp(20.0, b.h);
        cv.rect(b.w - 7.5, 1.7, 5.5, h, Rgb(0x8a, 0x88, 0x86), 1.0);
    }
}

// --- text boxes ------------------------------------------------------------------------------

struct Piece {
    text: String,
    font: Font,
    color: Rgb,
    underline: bool,
    w: f32,
}

/// Text box paragraphs with runs: wrapped to the box (CSS `pre-wrap`, `overflow-wrap: anywhere`).
pub fn text_body(cv: &mut Canvas, v: &Value, b: Bx, ctx: &Ctx<'_>) {
    let t = get(v, "text");
    let paras: Vec<(String, Vec<(String, Font, Rgb, bool)>)> =
        if let Some(ps) = t.get("paragraphs").and_then(Value::as_array) {
            let mut budget = 3000usize;
            let mut number = 0u32;
            ps.iter()
                .take(500)
                .filter(|p| p.is_object())
                .map(|p| {
                    let al = sk(p, "align").to_lowercase();
                    let list = sk(p, "list").to_lowercase();
                    let mut runs: Vec<(String, Font, Rgb, bool)> = arr(get(p, "runs"))
                        .iter()
                        .filter(|r| r.is_object())
                        .filter_map(|r| {
                            if budget == 0 {
                                return None;
                            }
                            budget -= 1;
                            let fam = sk(r, "font_family");
                            let lf = fam.to_lowercase();
                            let bold = get(r, "bold") == &Value::Bool(true);
                            let weight = if bold {
                                700
                            } else if lf.contains("semibold") {
                                600
                            } else if lf.contains("bold") {
                                700
                            } else if lf.contains("light") {
                                300
                            } else {
                                400
                            };
                            let sz = nk(r, "font_size", 0.0);
                            let size = if sz > 0.0 {
                                clampf(sz as f32, 4.0, 200.0) * PT
                            } else {
                                ctx.text_px
                            };
                            let family = if fam.is_empty() {
                                ctx.font.clone()
                            } else {
                                safe_font(fam)
                            };
                            let f = Font::new(&family, size)
                                .weight(weight)
                                .italic(get(r, "italic") == &Value::Bool(true));
                            let color = Rgb::from_value(get(r, "color")).unwrap_or(ctx.fg);
                            let ul = get(r, "underline") == &Value::Bool(true) || !sk(r, "url").is_empty();
                            Some((sk(r, "text").chars().take(20_000).collect(), f, color, ul))
                        })
                        .collect();
                    // a list item: its marker in the first run's font and colour (`list`: bullet,
                    // number; numbers count the consecutive numbered paragraphs)
                    if list == "bullet" || list == "number" {
                        number = if list == "number" {
                            number.saturating_add(1)
                        } else {
                            0
                        };
                        let marker = if list == "number" {
                            format!("{number}. ")
                        } else {
                            "\u{2022} ".to_string()
                        };
                        let (f, c) = runs
                            .first()
                            .map_or((Font::new(&ctx.font, ctx.text_px), ctx.fg), |r| {
                                (r.1.clone(), r.2)
                            });
                        runs.insert(0, (marker, f, c, false));
                    } else {
                        number = 0;
                    }
                    (al, runs)
                })
                .collect()
        } else {
            let raw = match t {
                Value::Array(a) => a
                    .iter()
                    .take(200)
                    .map(crate::data::st)
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => {
                    let s = crate::data::st(t);
                    if s.is_empty() {
                        sk(get(v, "format"), "text").to_string()
                    } else {
                        s
                    }
                }
            };
            if raw.is_empty() {
                // no text in the snapshot: a faint placeholder bar
                let h = (ctx.text_px * 0.7).min(b.h * 0.5);
                cv.round_rect(0.0, 2.0, b.w * 0.62, h, 3.0, Some((ctx.fg, 0.07)), None);
                return;
            }
            raw.chars()
                .take(5000)
                .collect::<String>()
                .split('\n')
                .map(|l| {
                    (
                        String::new(),
                        vec![(l.to_string(), Font::new(&ctx.font, ctx.text_px), ctx.fg, false)],
                    )
                })
                .collect()
        };
    let mut y = 0.0f32;
    for (align, runs) in &paras {
        if y > b.h {
            break;
        }
        // split runs into words and spaces
        let mut pieces: Vec<Piece> = Vec::new();
        for (text, f, c, ul) in runs {
            let mut cur = String::new();
            let mut cur_space = None;
            for ch in text.chars() {
                let sp = ch == ' ' || ch == '\t';
                if cur_space.is_some_and(|s| s != sp) || ch == '\n' {
                    let w = ctx.fonts.measure(f, &cur);
                    pieces.push(Piece {
                        text: std::mem::take(&mut cur),
                        font: f.clone(),
                        color: *c,
                        underline: *ul,
                        w,
                    });
                }
                if ch == '\n' {
                    pieces.push(Piece {
                        text: "\n".into(),
                        font: f.clone(),
                        color: *c,
                        underline: false,
                        w: 0.0,
                    });
                    cur_space = None;
                    continue;
                }
                cur.push(ch);
                cur_space = Some(sp);
            }
            if !cur.is_empty() {
                let w = ctx.fonts.measure(f, &cur);
                pieces.push(Piece {
                    text: cur,
                    font: f.clone(),
                    color: *c,
                    underline: *ul,
                    w,
                });
            }
        }
        let default_size = runs.first().map_or(ctx.text_px, |r| r.1.size);
        // lines
        let mut lines: Vec<Vec<Piece>> = vec![Vec::new()];
        let mut lw = 0.0f32;
        for p in pieces {
            if p.text == "\n" {
                lines.push(Vec::new());
                lw = 0.0;
                continue;
            }
            let space = p.text.starts_with(' ');
            if lw + p.w > b.w + 0.01 && !space && lines.last().is_some_and(|l| !l.is_empty()) {
                lines.push(Vec::new());
                lw = 0.0;
            }
            if p.w > b.w && !space {
                // a word longer than the line: break it anywhere
                let mut cur = String::new();
                for ch in p.text.chars() {
                    let mut t = cur.clone();
                    t.push(ch);
                    if ctx.fonts.measure(&p.font, &t) > b.w - lw && !cur.is_empty() {
                        let w = ctx.fonts.measure(&p.font, &cur);
                        if let Some(l) = lines.last_mut() {
                            l.push(Piece {
                                text: std::mem::take(&mut cur),
                                font: p.font.clone(),
                                color: p.color,
                                underline: p.underline,
                                w,
                            });
                        }
                        lines.push(Vec::new());
                        lw = 0.0;
                    }
                    cur.push(ch);
                }
                let w = ctx.fonts.measure(&p.font, &cur);
                lw += w;
                if let Some(l) = lines.last_mut() {
                    l.push(Piece {
                        text: cur,
                        font: p.font,
                        color: p.color,
                        underline: p.underline,
                        w,
                    });
                }
                continue;
            }
            lw += p.w;
            if let Some(l) = lines.last_mut() {
                l.push(p);
            }
        }
        for line in &lines {
            let size = line.iter().map(|p| p.font.size).fold(0.0f32, f32::max);
            let size = if size > 0.0 { size } else { default_size };
            let lh = size * 1.25;
            let (asc, desc) = line
                .iter()
                .map(|p| ctx.fonts.metrics(&p.font))
                .fold((0.0f32, 0.0f32), |a, m| (a.0.max(m.0), a.1.max(m.1)));
            let (asc, desc) = if asc > 0.0 {
                (asc, desc)
            } else {
                ctx.fonts.metrics(&Font::new(&ctx.font, size))
            };
            let baseline = y + (lh - (asc + desc)) / 2.0 + asc;
            // trailing spaces hang
            let mut end = line.len();
            while end > 0 && line.get(end - 1).is_some_and(|p| p.text.trim().is_empty()) {
                end -= 1;
            }
            let width: f32 = line.iter().take(end).map(|p| p.w).sum();
            let mut x = match align.as_str() {
                "center" => (b.w - width) / 2.0,
                "right" => b.w - width,
                _ => 0.0,
            };
            for p in line.iter().take(end) {
                let t = Text::new(&p.text, p.font.clone(), p.color)
                    .base(Base::Alphabetic)
                    .underline(p.underline);
                cv.text(ctx.fonts, &t, x, baseline);
                x += p.w;
            }
            y += lh;
        }
    }
}

// --- buttons, shapes, images, navigators -----------------------------------------------------

fn btn_icon(name: &str) -> &'static [&'static str] {
    match name {
        "back" | "leftarrow" => &["M10 3 5 8l5 5"],
        "next" | "forward" | "rightarrow" => &["M6 3l5 5-5 5"],
        "information" => &[
            "M8 1.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13z",
            "M8 7v4.5",
            "M8 4.6v.1",
        ],
        "help" => &[
            "M8 1.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13z",
            "M6 6.2a2 2 0 1 1 2.7 1.9c-.5.2-.7.6-.7 1.1v.6",
            "M8 11.6v.1",
        ],
        "bookmark" | "bookmarks" => &["M4.5 2h7v12L8 11l-3.5 3z"],
        "reset" => &["M3 8a5 5 0 1 0 1.5-3.6", "M3 2.5v3h3"],
        // an eraser tilted 45 degrees, its tip marked off (Desktop 2.158)
        "clearallslicers" => &["M1.6 10.4 10.4 1.6l4 4-8.8 8.8z", "M4.2 7.8l4 4"],
        "filter" | "applyallslicers" => &["M2 3h12l-4.5 5.5V13l-3 1.5V8.5z"],
        "qna" => &["M2.5 3h11v7.5h-6L4 13v-2.5H2.5z"],
        "drillthrough" => &["M3 8h9", "M9 5l3 3-3 3"],
        _ => &[],
    }
}

/// The images a button draws in its current state: its fill image and its custom icon
/// (`button.fill_image`, `button.icon_image` of the snapshot, resolved by the caller).
#[derive(Default, Clone, Copy)]
pub struct ButtonImages<'a> {
    pub fill: Option<&'a crate::imgs::Image>,
    pub icon: Option<&'a crate::imgs::Image>,
}

/// The outline of button `bt` in its `w` × `h` box at (`x`, `y`): its tile shape
/// (`button.shape`), else the rectangle rounded by `radius`.
fn button_path(bt: &Value, x: f32, y: f32, w: f32, h: f32, radius: f32) -> Option<tiny_skia::Path> {
    let shape = get(bt, "shape");
    let tile = sk(shape, "tile");
    if tile.is_empty() || tile == "rectangle" {
        crate::canvas::round_rect_path(x, y, w.max(1.0), h.max(1.0), radius)
    } else {
        nav_path(shape, x, y, w, h)
    }
}

/// The corner radius of a button's rectangle (the visual's border radius).
fn button_radius(v: &Value, b: Bx) -> f32 {
    clampf(
        nk(get(get(v, "format"), "border"), "radius", 0.0) as f32,
        0.0,
        b.h / 2.0,
    )
}

/// A button's shadow and glow in its current state (`button.show_shadow`, `button.show_glow`
/// and their properties), drawn on `cv` (the page, under the visual: they reach past its box)
/// around its outline at (`x`, `y`). The same drawing and defaults as the navigators'
/// buttons: BIcraft's own (`docs/formats/buttons.md`, "Shape, glow and shadow").
pub fn button_effects(cv: &mut Canvas, v: &Value, x: f32, y: f32, b: Bx, ctx: &Ctx<'_>) {
    let bt = get(v, "button");
    let Some(m) = bt.as_object() else {
        return;
    };
    let st = NavStyle { m: m.clone() };
    if st.flag("show_shadow") != Some(true) && st.flag("show_glow") != Some(true) {
        return;
    }
    let Some(p) = button_path(bt, x, y, b.w, b.h, button_radius(v, b)) else {
        return;
    };
    nav_shadow(cv, &st, &p);
    nav_glow(cv, &st, &p, ctx);
}

pub fn button_body(cv: &mut Canvas, v: &Value, b: Bx, ctx: &Ctx<'_>, images: ButtonImages<'_>) {
    let bt = get(v, "button");
    let px = ctx.px;
    // the visual's own state first, then the format (theme visual styles included)
    let ff = get(get(v, "format"), "button_fill");
    let ft = get(get(v, "format"), "button_text");
    let fill = if get(bt, "show_fill") == &Value::Bool(true) || !sk(bt, "fill").is_empty() {
        Rgb::from_value(get(bt, "fill"))
    } else if get(bt, "show_fill") != &Value::Bool(false) && get(ff, "show") != &Value::Bool(false) {
        Rgb::from_value(get(ff, "color"))
    } else {
        None
    };
    let transparency = get(bt, "fill_transparency")
        .as_f64()
        .or_else(|| get(ff, "transparency").as_f64())
        .unwrap_or(0.0);
    let alpha = clampf(1.0 - transparency as f32 / 100.0, 0.0, 1.0);
    let radius = button_radius(v, b);
    // a tile shape (Format › Shape) instead of the rounded rectangle
    let tile = sk(get(bt, "shape"), "tile");
    let shaped = !tile.is_empty() && tile != "rectangle";
    if let Some(c) = fill {
        if shaped {
            if let Some(p) = button_path(bt, 0.0, 0.0, b.w, b.h, radius) {
                cv.fill_path(&p, c, alpha);
            }
        } else {
            cv.round_rect(0.0, 0.0, b.w, b.h, radius, Some((c, alpha)), None);
        }
    }
    // the fill image of the state (Format › Style › Fill › Image), over the fill colour, in the
    // button's box (not cut to a tile shape)
    if get(bt, "show_fill") != &Value::Bool(false)
        && let Some(img) = images.fill
    {
        crate::draw_image_in(cv, img, sk(get(bt, "fill_image"), "fit"), 0.0, 0.0, b.w, b.h);
    }
    // the button's own outline (Style › Border), drawn when its state turns it on
    if get(bt, "show_outline") == &Value::Bool(true)
        && let Some(c) = Rgb::from_value(get(bt, "outline"))
    {
        let w = clampf(nk(bt, "outline_weight", 1.0) as f32, 0.0, 20.0);
        let a = clampf(1.0 - nk(bt, "outline_transparency", 0.0) as f32 / 100.0, 0.0, 1.0);
        if w > 0.0 && a > 0.0 {
            if shaped {
                if let Some(p) = button_path(
                    bt,
                    w / 2.0,
                    w / 2.0,
                    (b.w - w).max(1.0),
                    (b.h - w).max(1.0),
                    radius,
                ) {
                    cv.stroke(&p, c, a, w, None);
                }
            } else {
                cv.round_rect(
                    w / 2.0,
                    w / 2.0,
                    (b.w - w).max(0.0),
                    (b.h - w).max(0.0),
                    radius,
                    None,
                    Some((c.mix(ctx.bg, 1.0 - a), w, None)),
                );
            }
        }
    }
    let color = Rgb::from_value(get(bt, "font_color"))
        .or_else(|| Rgb::from_value(get(ft, "color")))
        .unwrap_or(ctx.fg2);
    let size = clampf(
        get(bt, "font_size")
            .as_f64()
            .or_else(|| get(ft, "font_size").as_f64())
            .unwrap_or(10.0) as f32,
        5.0,
        72.0,
    ) * PT;
    let family = if sk(ft, "font_family").is_empty() {
        ctx.font.clone()
    } else {
        safe_font(sk(ft, "font_family"))
    };
    let weight = if get(ft, "bold") == &Value::Bool(true)
        || sk(ft, "font_family").to_lowercase().contains("semibold")
    {
        600
    } else {
        400
    };
    let text: String = if get(bt, "show_text") == &Value::Bool(false) {
        String::new()
    } else {
        sk(bt, "text").chars().take(300).collect()
    };
    let icon_name = sk(bt, "icon").to_lowercase();
    let paths = if get(bt, "show_icon") == &Value::Bool(false) {
        &[][..]
    } else {
        btn_icon(&icon_name)
    };
    // a custom icon (Icon › Custom, its image) takes the icon's place
    let custom = images
        .icon
        .filter(|_| icon_name == "custom" && get(bt, "show_icon") != &Value::Bool(false));
    let has_icon = !paths.is_empty() || custom.is_some();
    let isz = (b.h * 0.6).min((px * 1.2).max(size * 1.1));
    let f = Font::new(&family, size).weight(weight);
    let gap = if !has_icon || text.is_empty() {
        0.0
    } else {
        size * 0.4
    };
    let icon_w = if has_icon { isz } else { 0.0 };
    let tmax = (b.w - icon_w - gap).max(0.0);
    let t = ctx.fonts.fit(&f, &text, tmax);
    let tw_ = ctx.fonts.measure(&f, &t);
    let total = icon_w + gap + tw_;
    let al = sk(bt, "align");
    let mut x = match al {
        "left" => 0.0,
        "right" => b.w - total,
        _ => (b.w - total) / 2.0,
    };
    let right = sk(bt, "icon_placement") == "right";
    let ic = Rgb::from_value(get(bt, "icon_color")).unwrap_or(color);
    let draw_icon = |cv: &mut Canvas, x: f32| {
        if let Some(img) = custom {
            let fit = sk(get(bt, "icon_image"), "fit");
            crate::draw_image_in(cv, img, fit, x, (b.h - isz) / 2.0, isz, isz);
            return;
        }
        let k = isz / 16.0;
        let tf = tiny_skia::Transform::from_translate(x, (b.h - isz) / 2.0).pre_scale(k, k);
        for d in paths {
            if let Some(p) = svg_path(d) {
                cv.stroke_tf(&p, ic, 1.5, tf);
            }
        }
    };
    if !right && has_icon {
        draw_icon(cv, x);
        x += icon_w + gap;
    }
    if !t.is_empty() {
        line_text(cv, ctx, &t, f, color, x, b.h / 2.0, Anchor::Start);
        x += tw_ + gap;
    }
    if right && has_icon {
        draw_icon(cv, x);
    }
}

pub fn image_placeholder(cv: &mut Canvas, b: Bx, ctx: &Ctx<'_>) {
    cv.round_rect(
        0.5,
        0.5,
        (b.w - 1.0).max(0.0),
        (b.h - 1.0).max(0.0),
        0.0,
        None,
        Some((ctx.fg.mix(ctx.bg, 0.88), 1.0, Some((4.0, 4.0)))),
    );
    let m = b.w.min(b.h);
    let k = (m * 0.3).min(48.0);
    let (x0, y0) = (b.w - k - 6.0, 6.0);
    let c = ctx.fg.mix(ctx.bg, 0.88);
    cv.polygon(
        &[
            (x0, y0 + k * 0.7),
            (x0 + k * 0.35, y0 + k * 0.25),
            (x0 + k * 0.6, y0 + k * 0.5),
            (x0 + k * 0.75, y0 + k * 0.38),
            (x0 + k, y0 + k * 0.7),
        ],
        c,
        1.0,
    );
    cv.circle(x0 + k * 0.78, y0 + k * 0.08, k * 0.09, Some((c, 1.0)), None);
}

/// A visual bicraft does not draw: a labelled box (type and title).
pub fn placeholder(cv: &mut Canvas, v: &Value, b: Bx, ctx: &Ctx<'_>) {
    placeholder_frame(cv, b, ctx);
    let typ = sk(v, "type");
    let label = if typ.is_empty() {
        "Visual".to_string()
    } else {
        type_label(typ)
    };
    placeholder_lines(cv, &[label, sk(v, "title").to_string()], b, ctx);
}

/// The placeholder's dashed, faintly filled box.
pub fn placeholder_frame(cv: &mut Canvas, b: Bx, ctx: &Ctx<'_>) {
    cv.round_rect(
        0.5,
        0.5,
        (b.w - 1.0).max(0.0),
        (b.h - 1.0).max(0.0),
        3.0,
        Some((ctx.fg, 0.03)),
        Some((ctx.faint, 1.0, Some((4.0, 3.0)))),
    );
}

/// The placeholder's lines (empty ones left out), centred in `b`, the first semibold.
pub fn placeholder_lines(cv: &mut Canvas, lines: &[String], b: Bx, ctx: &Ctx<'_>) {
    let px = ctx.px;
    let lines: Vec<&String> = lines.iter().filter(|l| !l.is_empty()).collect();
    let total = lines.len() as f32 * px * 1.4;
    let mut y = (b.h - total) / 2.0 + px * 0.7;
    for (i, l) in lines.iter().enumerate() {
        let f = font(ctx, px).weight(if i == 0 { 600 } else { 400 });
        let t = ctx.fonts.fit(&f, l, b.w - 8.0);
        line_text(cv, ctx, &t, f, ctx.muted, b.w / 2.0, y, Anchor::Middle);
        y += px * 1.4;
    }
}

pub fn type_label(t: &str) -> String {
    match t {
        "tableEx" => return "Table".into(),
        "pivotTable" => return "Matrix".into(),
        "textbox" => return "Text box".into(),
        "multiRowCard" => return "Multi-row card".into(),
        "cardVisual" => return "Card (new)".into(),
        "actionButton" => return "Button".into(),
        "basicShape" => return "Shape".into(),
        "kpi" => return "KPI".into(),
        _ => {}
    }
    let mut out = String::new();
    let mut prev_lower = false;
    for c in t.chars().take(80) {
        if c.is_ascii_uppercase() && prev_lower {
            out.push(' ');
        }
        prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        out.push(c.to_ascii_lowercase());
    }
    let mut cs = out.chars();
    match cs.next() {
        Some(f) => f.to_uppercase().collect::<String>() + cs.as_str(),
        None => out,
    }
}

/// A page navigator: one button per page of the report (hidden and tooltip pages only when its
/// `pages` options show them), the page drawn on highlighted; a click sends `view.interact
/// {action: "page"}`.
pub fn nav_body(cv: &mut Canvas, v: &Value, b: Bx, ctx: &Ctx<'_>, page_name: &str) {
    let opts = get(v, "navigator");
    let show_hidden = get(opts, "show_hidden_pages") == &Value::Bool(true);
    let show_tips = get(opts, "show_tooltip_pages") == &Value::Bool(true);
    let mut items: Vec<(String, String)> = ctx
        .pages
        .iter()
        .filter(|p| (show_hidden || !p.hidden) && (show_tips || !p.tooltip))
        .map(|p| (p.name.clone(), p.label.clone()))
        .collect();
    if items.is_empty() {
        items.push((page_name.to_string(), "Page".into()));
    }
    let visual = sk(v, "name");
    nav_buttons(
        cv,
        opts,
        b,
        ctx,
        &items,
        Some(page_name),
        crate::states::nav_item(v, ctx),
        |key| serde_json::json!({"action": "page", "page": key, "visual": visual}),
    );
}

/// A bookmark navigator: one button per bookmark of its group (`navigator.bookmark_group`, a
/// group's name or display name), or of the report when it names none, in the bookmarks' order;
/// the bookmark the reader applied last highlighted (else the navigator's `selected_bookmark`);
/// a click sends `view.interact {action: "bookmark"}`. Without bookmarks it is an empty frame.
pub fn bookmark_nav_body(cv: &mut Canvas, v: &Value, b: Bx, ctx: &Ctx<'_>, page_name: &str) {
    let opts = get(v, "navigator");
    let group = sk(opts, "bookmark_group");
    // Allow deselection: a click on the selected button launches the deselection bookmark (or,
    // without one, leaves no button selected); Hide deselection bookmark leaves it out
    let deselect = sk(opts, "deselection_bookmark");
    let deselect = ctx
        .bookmarks
        .iter()
        .find(|x| !deselect.is_empty() && (x.name == deselect || x.label == deselect))
        .map(|x| x.name.clone());
    let allow_deselect = get(opts, "allow_deselection") == &Value::Bool(true);
    let hide_deselect = allow_deselect && get(opts, "hide_deselection_bookmark") == &Value::Bool(true);
    let items: Vec<(String, String)> = ctx
        .bookmarks
        .iter()
        .filter(|x| {
            group.is_empty() || x.group_name == group || (x.group_name.is_empty() && x.group == group)
        })
        .filter(|x| !hide_deselect || deselect.as_deref() != Some(x.name.as_str()))
        .map(|x| (x.name.clone(), x.label.clone()))
        .collect();
    if items.is_empty() {
        cv.round_rect(
            0.5,
            0.5,
            (b.w - 1.0).max(0.0),
            (b.h - 1.0).max(0.0),
            0.0,
            None,
            Some((ctx.fg.mix(ctx.bg, 0.88), 1.0, Some((4.0, 4.0)))),
        );
        return;
    }
    let selected = if items.iter().any(|(k, _)| *k == ctx.bookmark) {
        Some(ctx.bookmark.as_str())
    } else {
        Some(sk(opts, "selected_bookmark")).filter(|s| !s.is_empty())
    };
    let visual = sk(v, "name");
    let current = selected.map(str::to_string);
    nav_buttons(
        cv,
        opts,
        b,
        ctx,
        &items,
        selected,
        crate::states::nav_item(v, ctx),
        |key| {
            if allow_deselect && current.as_deref() == Some(key) {
                return match &deselect {
                    Some(d) => {
                        serde_json::json!({"action": "bookmark", "bookmark": d, "page": page_name, "visual": visual})
                    }
                    None => {
                        serde_json::json!({"action": "bookmark_deselect", "page": page_name, "visual": visual})
                    }
                };
            }
            serde_json::json!({"action": "bookmark", "bookmark": key, "page": page_name, "visual": visual})
        },
    );
}

/// The buttons of a navigator: in a row (`horizontal`), a column (`vertical`) or a grid of
/// `columns` (`navigator.orientation`; not set: a column when the box is much taller than wide,
/// else a row), `selected` highlighted, the one under the pointer (`hot`: its key, pressed) in
/// its hover or press state, each a hit sending `action(key)`.
#[allow(clippy::too_many_arguments)]
fn nav_buttons(
    cv: &mut Canvas,
    opts: &Value,
    b: Bx,
    ctx: &Ctx<'_>,
    items: &[(String, String)],
    selected: Option<&str>,
    hot: Option<(&str, bool)>,
    action: impl Fn(&str) -> Value,
) {
    let n = items.len().max(1);
    let (cols, rows) = match sk(opts, "orientation") {
        "vertical" => (1, n),
        "horizontal" => (n, 1),
        "grid" => {
            let given = |k: &str| get(opts, k).as_u64().map(|x| x.clamp(1, 100) as usize);
            let cols = given("columns")
                .or_else(|| given("rows").map(|r| n.div_ceil(r)))
                .unwrap_or_else(|| (n as f32).sqrt().ceil() as usize)
                .clamp(1, n);
            (cols, n.div_ceil(cols))
        }
        _ if b.h > b.w * 1.4 => (1, n),
        _ => (n, 1),
    };
    let gap = get(opts, "padding")
        .as_f64()
        .map_or(6.0, |p| clampf(p as f32, 0.0, 100.0));
    let bw = ((b.w - gap * (cols as f32 - 1.0)) / cols as f32).max(1.0);
    let bh = ((b.h - gap * (rows as f32 - 1.0)) / rows as f32).max(1.0);
    let fs = clampf((bh * 0.4).min(ctx.px), 6.0, 18.0);
    let styles = get(opts, "styles");
    let shape = get(opts, "shape");
    for (i, (key, label)) in items.iter().enumerate() {
        let (x, y) = ((i % cols) as f32 * (bw + gap), (i / cols) as f32 * (bh + gap));
        if ctx.ix.on() {
            let a = ctx.ix.plain(action(key));
            let shape = crate::hits::HitShape::Rect {
                x,
                y,
                w: bw.max(1.0),
                h: bh.max(1.0),
            };
            cv.hit(ctx.ix.hit(shape, a, Vec::new()).map(|mut h| {
                h.item = Some(key.clone());
                h
            }));
        }
        let on = selected == Some(key.as_str());
        let st = NavStyle::of(styles, on, hot.filter(|(k, _)| *k == key).map(|(_, p)| p));
        let show_outline = st.flag("show_outline").unwrap_or(st.has("outline"));
        let ow = if show_outline {
            clampf(st.num("outline_weight").unwrap_or(1.0), 0.0, 20.0)
        } else {
            0.0
        };
        // the button's outline: its tile shape (`navigator.shape`), else a rectangle, inset by
        // half the border so that it stays in the cell
        let path = nav_path(shape, x + ow / 2.0, y + ow / 2.0, bw - ow, bh - ow);
        // shadow and glow under the button (BIcraft's drawing: no Desktop default was observed)
        if let Some(p) = &path {
            nav_shadow(cv, &st, p);
            nav_glow(cv, &st, p, ctx);
        }
        // fill, then outline
        let show_fill = st.flag("show_fill").unwrap_or(st.has("fill"));
        if show_fill
            && let Some(c) = Rgb::from_value(st.get("fill"))
            && let Some(p) = &path
        {
            let alpha = clampf(1.0 - st.num("fill_transparency").unwrap_or(0.0) / 100.0, 0.0, 1.0);
            cv.fill_path(p, c, alpha);
        }
        let ot = st.num("outline_transparency").unwrap_or(0.0);
        if show_outline
            && ow > 0.0
            && ot < 100.0
            && let Some(c) = Rgb::from_value(st.get("outline"))
            && let Some(p) = &path
        {
            cv.stroke(p, c, clampf(1.0 - ot / 100.0, 0.0, 1.0), ow, None);
        }
        // the accent bar: Desktop's selected mark, by default along the bottom of the selected
        // item only
        let show_accent = st.flag("show_accent").unwrap_or(on);
        if show_accent {
            let c = Rgb::from_value(st.get("accent")).unwrap_or(ctx.accent);
            let w = clampf(st.num("accent_width").unwrap_or(3.0), 0.0, bw.min(bh));
            let (ax, ay, aw, ah) = match st.text("accent_position") {
                "top" => (x, y, bw, w),
                "left" => (x, y, w, bh),
                "right" => (x + bw - w, y, w, bh),
                _ => (x, y + bh - w, bw, w),
            };
            if w > 0.0 {
                cv.rect(ax, ay, aw, ah, c, 1.0);
            }
        }
        if st.flag("show_text") == Some(false) {
            continue;
        }
        let size = st.num("font_size").map_or(fs, |pt| clampf(pt * PT, 5.0, 96.0));
        let bold = st.flag("bold").unwrap_or(on);
        let f = font(ctx, size)
            .weight(if bold { 600 } else { 400 })
            .italic(st.flag("italic") == Some(true));
        let t = ctx.fonts.fit(&f, label, bw - 12.0);
        let color = Rgb::from_value(st.get("font_color")).unwrap_or(if on { ctx.accent } else { ctx.fg });
        line_text(cv, ctx, &t, f, color, x + bw / 2.0, y + bh / 2.0, Anchor::Middle);
    }
}

/// The outline of a navigator button at (`x`, `y`), `w` × `h`: its tile shape (`shape.tile`,
/// as the shape visual draws it), else a rectangle.
fn nav_path(shape: &Value, x: f32, y: f32, w: f32, h: f32) -> Option<tiny_skia::Path> {
    let (w, h) = (w.max(1.0), h.max(1.0));
    let tile = sk(shape, "tile");
    let p = if tile.is_empty() || tile == "rectangle" {
        tiny_skia::Rect::from_xywh(0.0, 0.0, w, h).map(tiny_skia::PathBuilder::from_rect)
    } else {
        crate::shapes::outline(tile, shape, w, h)
            .or_else(|| tiny_skia::Rect::from_xywh(0.0, 0.0, w, h).map(tiny_skia::PathBuilder::from_rect))
    };
    p?.transform(tiny_skia::Transform::from_translate(x, y))
}

/// A button's shadow (`show_shadow`): three widening translucent copies of its outline, moved
/// `shadow_distance` px (3 by default) towards `shadow_angle` degrees (45, down right, by
/// default), `shadow_blur` px wide (5), in `shadow` (black) at `shadow_transparency` (70 %).
/// BIcraft's drawing of the report theme schema's properties; inner shadows are drawn outer.
fn nav_shadow(cv: &mut Canvas, st: &NavStyle, p: &tiny_skia::Path) {
    if st.flag("show_shadow") != Some(true) || crate::canvas::high_contrast_on() {
        return;
    }
    let c = Rgb::from_value(st.get("shadow")).unwrap_or(Rgb(0, 0, 0));
    let alpha = clampf(
        1.0 - st.num("shadow_transparency").unwrap_or(70.0) / 100.0,
        0.0,
        1.0,
    );
    let d = clampf(st.num("shadow_distance").unwrap_or(3.0), 0.0, 100.0);
    let a = st.num("shadow_angle").unwrap_or(45.0).to_radians();
    let blur = clampf(st.num("shadow_blur").unwrap_or(5.0), 0.0, 100.0);
    let moved = p
        .clone()
        .transform(tiny_skia::Transform::from_translate(d * a.cos(), d * a.sin()));
    let Some(moved) = moved else {
        return;
    };
    cv.fill_path(&moved, c, alpha / 2.0);
    for k in 1..=3 {
        let w = blur * k as f32 / 3.0;
        if w > 0.0 {
            cv.stroke(&moved, c, alpha / 6.0, w, None);
        }
    }
}

/// A button's glow (`show_glow`): its outline stroked three times, widening to `glow_blur` px
/// (5 by default), in `glow` (the theme's accent) at `glow_transparency` (50 %). BIcraft's
/// drawing of the report theme schema's properties.
fn nav_glow(cv: &mut Canvas, st: &NavStyle, p: &tiny_skia::Path, ctx: &Ctx<'_>) {
    if st.flag("show_glow") != Some(true) || crate::canvas::high_contrast_on() {
        return;
    }
    let c = Rgb::from_value(st.get("glow")).unwrap_or(ctx.accent);
    let alpha = clampf(
        1.0 - st.num("glow_transparency").unwrap_or(50.0) / 100.0,
        0.0,
        1.0,
    );
    let blur = clampf(st.num("glow_blur").unwrap_or(5.0), 0.0, 100.0);
    for k in (1..=3).rev() {
        let w = blur * 2.0 * k as f32 / 3.0;
        if w > 0.0 {
            cv.stroke(p, c, alpha / 3.0, w, None);
        }
    }
}

/// A navigator item's style (`navigator.styles` of the snapshot): the selected state's
/// properties over the default state's.
/// A navigator button's style: its states' properties merged (`crate::states::nav_layers`).
struct NavStyle {
    m: serde_json::Map<String, Value>,
}

const NULL: Value = Value::Null;

impl NavStyle {
    fn of(styles: &Value, on: bool, hot: Option<bool>) -> Self {
        NavStyle {
            m: crate::states::nav_layers(styles, on, hot),
        }
    }

    fn get(&self, k: &str) -> &Value {
        self.m.get(k).unwrap_or(&NULL)
    }

    fn has(&self, k: &str) -> bool {
        !self.get(k).is_null()
    }

    fn flag(&self, k: &str) -> Option<bool> {
        self.get(k).as_bool()
    }

    fn num(&self, k: &str) -> Option<f32> {
        self.get(k).as_f64().map(|x| x as f32).filter(|x| x.is_finite())
    }

    fn text(&self, k: &str) -> &str {
        self.get(k).as_str().unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_labels() {
        assert_eq!(type_label("clusteredColumnChart"), "Clustered column chart");
        assert_eq!(type_label("tableEx"), "Table");
        assert_eq!(type_label(""), "");
    }
}
