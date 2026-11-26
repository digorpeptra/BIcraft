//! Text check: the text runs Desktop drew (capture.mjs's `NN-<page>.json`: one entry per DOM
//! text node with its computed style and box) against the runs bicraft drew (`bicraft render
//! --text-json`), matched per visual.
//!
//! Strings are compared after whitespace normalisation (no-break and thin spaces become spaces,
//! runs of spaces collapse, ends trimmed). Equal strings pair up by nearest box centre; a run of
//! ours ending in "…" also matches a Desktop string it is a prefix of (Desktop truncates with CSS,
//! so its DOM keeps the whole string). Font families are reduced to a base family and weight
//! (`Segoe UI Semibold` = `Segoe UI` 600; Desktop's web-font aliases: `wf_segoe-ui_*` = Segoe UI,
//! `wf_standard-font` counted as DIN, `wf_standard-font_light` as Segoe UI 300).

use serde_json::{Value, json};

use super::metrics::{ciede2000, lab};

/// One text run in report pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub text: String,
    pub family: String,
    pub weight: u16,
    pub size: f64,
    pub color: [u8; 3],
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Run {
    fn centre(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    pub fn inside(&self, x: f64, y: f64, w: f64, h: f64) -> bool {
        let (cx, cy) = self.centre();
        cx >= x && cx < x + w && cy >= y && cy < y + h
    }
}

/// Collapses whitespace (incl. no-break, narrow no-break and thin spaces).
pub fn norm_text(s: &str) -> String {
    s.split(|c: char| c.is_whitespace() || matches!(c, '\u{a0}' | '\u{202f}' | '\u{2009}' | '\u{200b}'))
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// (base family, weight) of a CSS font family list and weight.
pub fn canon_font(family: &str, weight: u16) -> (String, u16) {
    let first = family
        .split(',')
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches(['"', '\''])
        .trim();
    let mut f = first.to_lowercase().replace(['_', '-'], " ");
    let mut named: Option<u16> = None;
    if let Some(rest) = f.strip_prefix("wf ") {
        // Desktop's web-font aliases of Segoe UI
        named = if rest.ends_with("light") && !rest.ends_with("semilight") {
            Some(300)
        } else {
            None
        };
        if let Some(suffix) = rest.strip_prefix("standard font") {
            // the classic look's callout font: DIN, its light cut Segoe UI Light
            f = if named.is_some() {
                "segoe ui light".into()
            } else {
                format!("din{suffix}")
            };
        } else if let Some(suffix) = rest.strip_prefix("segoe ui") {
            f = format!("segoe ui{suffix}");
        }
    }
    for (suffix, w) in [
        (" semibold", 600),
        (" demibold", 600),
        (" semilight", 350),
        (" bold", 700),
        (" light", 300),
        (" black", 900),
        (" heavy", 900),
        (" normal", 400),
        (" regular", 400),
    ] {
        if let Some(base) = f.strip_suffix(suffix) {
            if suffix != " normal" && suffix != " regular" {
                named = Some(w);
            }
            f = base.trim().to_string();
            break;
        }
    }
    let w = match named {
        Some(n) if weight == 400 => n,
        _ => weight,
    };
    (f, w)
}

fn css_rgb(s: &str) -> Option<[u8; 3]> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() != 6 {
            return None;
        }
        let p = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
        return Some([p(0)?, p(2)?, p(4)?]);
    }
    let inner = s
        .strip_prefix("rgba(")
        .or_else(|| s.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let mut it = inner.split(',').map(|v| v.trim().parse::<f64>().ok());
    let mut c = [0u8; 3];
    for v in &mut c {
        *v = it.next()??.round().clamp(0.0, 255.0) as u8;
    }
    Some(c)
}

fn px(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().trim_end_matches("px").trim().parse().ok(),
        _ => None,
    }
    .filter(|x: &f64| x.is_finite())
}

fn num(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite())
}

/// Desktop's text nodes (capture.mjs), converted to report pixels for a page `page_width` wide.
/// Entries flagged `"vis": false` (visually hidden accessibility text) are skipped.
pub fn desktop_runs(doc: &Value, page_width: f64) -> Vec<Run> {
    let Some(items) = doc.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|e| e.get("vis") != Some(&Value::Bool(false)))
        .filter_map(|e| {
            let text = norm_text(e.get("t")?.as_str()?);
            if text.is_empty() {
                return None;
            }
            let z = num(e, "z").filter(|z| *z > 0.0).unwrap_or(page_width);
            let f = page_width / z;
            let scale = num(e, "scale").filter(|s| *s > 0.0).unwrap_or(1.0);
            let size = px(e.get("fs")?)? * scale * f;
            let weight = e
                .get("fw")
                .and_then(|w| px(w).map(|x| x.clamp(1.0, 1000.0) as u16))
                .unwrap_or(400);
            let (family, weight) = canon_font(e.get("ff").and_then(Value::as_str).unwrap_or(""), weight);
            let tag = e.get("tag").and_then(Value::as_str).unwrap_or("");
            let fill = e.get("fill").and_then(Value::as_str).and_then(css_rgb);
            let color = e.get("color").and_then(Value::as_str).and_then(css_rgb);
            // SVG text paints with `fill`; HTML text with `color` (its `fill` is inherited noise)
            let color = if matches!(tag, "text" | "tspan" | "textPath") {
                fill.or(color)
            } else {
                color
            }
            .unwrap_or([0; 3]);
            Some(Run {
                text,
                family,
                weight,
                size,
                color,
                x: num(e, "x")? * f,
                y: num(e, "y")? * f,
                w: num(e, "w")? * f,
                h: num(e, "h")? * f,
            })
        })
        .collect()
}

/// bicraft's text dump (`report.render {text_dump: true}`), already in report pixels.
pub fn our_runs(doc: &Value) -> Vec<Run> {
    let items = doc
        .get("texts")
        .and_then(Value::as_array)
        .or_else(|| doc.as_array());
    let Some(items) = items else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|e| {
            let text = norm_text(e.get("t")?.as_str()?);
            if text.is_empty() {
                return None;
            }
            let weight = num(e, "weight").unwrap_or(400.0).clamp(1.0, 1000.0) as u16;
            let (family, weight) = canon_font(e.get("font").and_then(Value::as_str).unwrap_or(""), weight);
            Some(Run {
                text,
                family,
                weight,
                size: num(e, "size")?,
                color: e
                    .get("color")
                    .and_then(Value::as_str)
                    .and_then(css_rgb)
                    .unwrap_or([0; 3]),
                x: num(e, "x")?,
                y: num(e, "y")?,
                w: num(e, "w")?,
                h: num(e, "h")?,
            })
        })
        .collect()
}

/// A matched pair's differences (ours − Desktop).
#[derive(Debug, Clone, PartialEq)]
pub struct Pair {
    pub text: String,
    pub truncated: bool,
    pub dsize: f64,
    pub de: f64,
    pub dweight: i32,
    pub family: Option<(String, String)>,
    pub dx: f64,
    pub dy: f64,
}

impl Pair {
    /// Differs visibly: size by more than 0.5 px, colour by ΔE00 > 3, weight, family, or the box
    /// centre by more than 3 px.
    pub fn differs(&self) -> bool {
        self.dsize.abs() > 0.5
            || self.de > 3.0
            || self.dweight != 0
            || self.family.is_some()
            || self.dx.hypot(self.dy) > 3.0
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextReport {
    pub desktop: usize,
    pub ours: usize,
    pub pairs: Vec<Pair>,
    pub missing: Vec<String>,
    pub extra: Vec<String>,
}

impl TextReport {
    /// Matched share of the larger side (1 when neither side has text).
    pub fn score(&self) -> f64 {
        let n = self.desktop.max(self.ours);
        if n == 0 {
            1.0
        } else {
            self.pairs.len() as f64 / n as f64
        }
    }

    pub fn json(&self) -> Value {
        let mean = |f: &dyn Fn(&Pair) -> f64| {
            if self.pairs.is_empty() {
                Value::Null
            } else {
                json!(round(
                    self.pairs.iter().map(f).sum::<f64>() / self.pairs.len() as f64,
                    3
                ))
            }
        };
        json!({
            "score": round(self.score(), 4),
            "desktop": self.desktop,
            "ours": self.ours,
            "matched": self.pairs.len(),
            "missing": self.missing,
            "extra": self.extra,
            "mean_abs_size_diff": mean(&|p| p.dsize.abs()),
            "mean_colour_de": mean(&|p| p.de),
            "mean_offset": mean(&|p| p.dx.hypot(p.dy)),
            "different": self.pairs.iter().filter(|p| p.differs()).map(|p| json!({
                "t": p.text,
                "truncated": p.truncated,
                "size": round(p.dsize, 2),
                "de": round(p.de, 2),
                "weight": p.dweight,
                "family": p.family.as_ref().map(|(d, o)| json!({"desktop": d, "ours": o})),
                "dx": round(p.dx, 1),
                "dy": round(p.dy, 1),
            })).collect::<Vec<_>>(),
        })
    }
}

pub fn round(x: f64, digits: i32) -> f64 {
    let k = 10f64.powi(digits);
    (x * k).round() / k
}

fn pair(d: &Run, o: &Run, truncated: bool) -> Pair {
    let ((dcx, dcy), (ocx, ocy)) = (d.centre(), o.centre());
    Pair {
        text: d.text.clone(),
        truncated,
        dsize: o.size - d.size,
        de: ciede2000(lab(d.color), lab(o.color)),
        dweight: i32::from(o.weight) - i32::from(d.weight),
        family: (d.family != o.family).then(|| (d.family.clone(), o.family.clone())),
        dx: ocx - dcx,
        dy: ocy - dcy,
    }
}

/// Matches the runs of one visual (see the module notes).
pub fn match_runs(desk: &[&Run], ours: &[&Run]) -> TextReport {
    let mut cand: Vec<(f64, usize, usize, bool)> = Vec::new();
    for (i, d) in desk.iter().enumerate() {
        for (j, o) in ours.iter().enumerate() {
            let exact = d.text == o.text;
            let trunc = !exact
                && o.text
                    .strip_suffix('…')
                    .map(str::trim_end)
                    .is_some_and(|p| !p.is_empty() && d.text.starts_with(p));
            if exact || trunc {
                let ((a, b), (c, e)) = (d.centre(), o.centre());
                // exact matches first, then by distance
                let dist = (a - c).hypot(b - e) + if exact { 0.0 } else { 1e9 };
                cand.push((dist, i, j, trunc));
            }
        }
    }
    cand.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut dused = vec![false; desk.len()];
    let mut oused = vec![false; ours.len()];
    let mut pairs = Vec::new();
    for (_, i, j, trunc) in cand {
        if dused.get(i).copied().unwrap_or(true) || oused.get(j).copied().unwrap_or(true) {
            continue;
        }
        if let (Some(d), Some(o)) = (desk.get(i), ours.get(j)) {
            pairs.push(pair(d, o, trunc));
        }
        if let Some(u) = dused.get_mut(i) {
            *u = true;
        }
        if let Some(u) = oused.get_mut(j) {
            *u = true;
        }
    }
    let left = |runs: &[&Run], used: &[bool]| -> Vec<String> {
        runs.iter()
            .zip(used)
            .filter(|(_, u)| !**u)
            .map(|(r, _)| r.text.clone())
            .collect()
    };
    TextReport {
        desktop: desk.len(),
        ours: ours.len(),
        missing: left(desk, &dused),
        extra: left(ours, &oused),
        pairs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(t: &str, x: f64, size: f64) -> Run {
        Run {
            text: t.into(),
            family: "segoe ui".into(),
            weight: 400,
            size,
            color: [0; 3],
            x,
            y: 10.0,
            w: 40.0,
            h: 14.0,
        }
    }

    #[test]
    fn normalises_spaces_and_fonts() {
        assert_eq!(norm_text("  65\u{a0}204 \u{202f}k\n"), "65 204 k");
        assert_eq!(
            canon_font("\"Segoe UI Semibold\", wf_segoe-ui_semibold, helvetica", 400),
            ("segoe ui".into(), 600)
        );
        assert_eq!(
            canon_font("wf_standard-font_light, helvetica", 400),
            ("segoe ui".into(), 300)
        );
        assert_eq!(
            canon_font("wf_standard-font, helvetica, arial", 400),
            ("din".into(), 400)
        );
        assert_eq!(
            canon_font("\"Segoe UI\", wf_segoe-ui_normal", 700),
            ("segoe ui".into(), 700)
        );
        assert_eq!(canon_font("Segoe UI Bold", 400), ("segoe ui".into(), 700));
        assert_eq!(canon_font("DIN", 400), ("din".into(), 400));
        assert_eq!(css_rgb("rgb(51, 51, 51)"), Some([51, 51, 51]));
        assert_eq!(css_rgb("rgba(1, 2, 3, 0.5)"), Some([1, 2, 3]));
        assert_eq!(css_rgb("#01B8AA"), Some([1, 184, 170]));
        assert_eq!(css_rgb("none"), None);
    }

    #[test]
    fn desktop_entries_convert_to_report_pixels() {
        let doc = json!([
            {"t": "910K", "tag": "text", "ff": "wf_standard-font", "fs": "36px", "fw": "400",
             "color": "rgb(0, 0, 0)", "fill": "rgb(51, 51, 51)", "x": 130.0, "y": 70.0, "w": 76.0,
             "h": 38.0, "scale": 1.0291627, "z": 1317.323},
            {"t": "Total", "tag": "H4", "ff": "Segoe UI", "fs": "24px", "fw": "400",
             "color": "rgb(102, 102, 102)", "fill": "rgb(0, 0, 0)", "x": 0, "y": 0, "w": 10, "h": 10, "z": 1280},
            {"t": "hidden", "tag": "H2", "ff": "Segoe UI", "fs": "28px", "fw": "400", "color": "rgb(0, 0, 0)",
             "x": 0, "y": 0, "w": 10, "h": 10, "z": 1280, "vis": false},
            {"t": "   ", "x": 0}
        ]);
        let r = desktop_runs(&doc, 1280.0);
        assert_eq!(r.len(), 2);
        assert!((r[0].size - 36.0).abs() < 0.01);
        assert_eq!(r[0].color, [51, 51, 51]);
        assert!((r[0].x - 130.0 * 1280.0 / 1317.323).abs() < 1e-6);
        assert_eq!(r[1].color, [102, 102, 102]);
        assert!(desktop_runs(&json!({"not": "a list"}), 1280.0).is_empty());
    }

    #[test]
    fn matching_reports_missing_extra_and_differences() {
        let d = [
            run("North", 0.0, 10.67),
            run("North", 100.0, 10.67),
            run("Total", 50.0, 12.0),
        ];
        let mut o = [
            run("North", 101.0, 10.67),
            run("North", 2.0, 12.0),
            run("Sum", 50.0, 12.0),
        ];
        o[0].color = [102, 102, 102];
        let dr: Vec<&Run> = d.iter().collect();
        let or: Vec<&Run> = o.iter().collect();
        let t = match_runs(&dr, &or);
        assert_eq!(t.pairs.len(), 2);
        assert_eq!(t.missing, vec!["Total".to_string()]);
        assert_eq!(t.extra, vec!["Sum".to_string()]);
        // nearest pairing: x 100 ↔ 101 (colour differs), x 0 ↔ 2 (size differs)
        let near = t.pairs.iter().find(|p| p.dx.abs() < 1.5).unwrap();
        assert!(near.de > 3.0 && near.differs());
        let far = t.pairs.iter().find(|p| p.dx > 1.5).unwrap();
        assert!((far.dsize - 1.33).abs() < 1e-9);
        assert!((t.score() - 2.0 / 3.0).abs() < 1e-12);
        assert_eq!(match_runs(&[], &[]).score(), 1.0);
    }

    #[test]
    fn ellipsis_matches_the_full_desktop_string() {
        let d = [run("Total et Goal par Month", 0.0, 12.0)];
        let o = [run("Total et Goal par…", 0.0, 12.0)];
        let t = match_runs(&d.iter().collect::<Vec<_>>(), &o.iter().collect::<Vec<_>>());
        assert_eq!(t.pairs.len(), 1);
        assert!(t.pairs[0].truncated && !t.pairs[0].differs());
    }

    #[test]
    fn our_dump_parses() {
        let doc = json!({"texts": [{"t": "A\u{a0}B", "font": "Segoe UI Semibold", "size": 12.0, "weight": 400,
            "color": "#666666", "x": 1, "y": 2, "w": 3, "h": 4}]});
        let r = our_runs(&doc);
        assert_eq!(r.len(), 1);
        assert_eq!(
            (r[0].text.as_str(), r[0].weight, r[0].color),
            ("A B", 600, [102, 102, 102])
        );
    }
}
