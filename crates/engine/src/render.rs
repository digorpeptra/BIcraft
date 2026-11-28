//! `report.render`: report pages to PNG through `bicraft-render`, from the same snapshot the live
//! viewer draws (`docs/render.md`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bicraft_render::{FontBook, RenderOptions};
use serde_json::{Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, opt_str};
use crate::snapshot::SnapshotLimits;

/// Largest total of inline (base64) PNG bytes one call returns.
const MAX_INLINE_BYTES: usize = 24 << 20;
/// Time the data evaluation may take for one render (the viewer uses 1 s per snapshot).
const DATA_BUDGET_MS: u64 = 60_000;

pub(crate) fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let b = [
            c.first().copied().unwrap_or(0),
            c.get(1).copied().unwrap_or(0),
            c.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if i <= c.len() {
                out.push(char::from(T[((n >> shift) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A file-name-safe page label.
fn slug(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    let t = t.trim_matches('_').to_string();
    if t.is_empty() { "page".into() } else { t }
}

pub(crate) fn report_render(s: &Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "page",
            "out",
            "scale",
            "format",
            "font_dirs",
            "system_fonts",
            "badges",
            "ui_language",
            "culture",
            "format_culture",
            "text_dump",
            "lcd_text",
            "high_contrast",
            "as_of",
            "fail_on_empty",
            "key_influencers",
        ],
    )?;
    let fail_on_empty = opt_bool(p, "fail_on_empty")?.unwrap_or(false);
    let key_influencers = match opt_str(p, "key_influencers")? {
        None => None,
        Some("estimate") => Some(true),
        Some("placeholder") => Some(false),
        Some(x) => {
            return Err(format!(
                "unknown `key_influencers` `{x}` (\"placeholder\", the default, or \"estimate\")"
            ));
        }
    };
    let ui_language = crate::ui_lang::param(p)?;
    let culture = crate::params::opt_str(p, "culture")?.filter(|c| !c.trim().is_empty());
    let as_of = opt_str(p, "as_of")?
        .map(crate::preview::filters::parse_as_of)
        .transpose()?;
    // Numbers and dates follow the model culture (render's default), not `culture`, which
    // only picks translated captions; `format_culture` overrides it.
    let format_culture = match opt_str(p, "format_culture")? {
        None => None,
        Some(c) => match bicraft_locale::Locale::find(c) {
            Some(l) if !l.is_invariant() => Some(c.trim().to_string()),
            _ => {
                return Err(format!(
                    "unknown `format_culture` `{c}` (a culture name such as \"fr-FR\")"
                ));
            }
        },
    };
    // A culture without translations (or the model's own) renders the base captions with a
    // warning; only a malformed name is refused.
    let mut warnings: Vec<String> = Vec::new();
    let culture = match (culture, &s.model) {
        (Some(c), Some(m)) => {
            let (apply, warning) = crate::translations::render_culture(m, c)?;
            warnings.extend(warning);
            apply
        }
        _ => None,
    };
    if s.report.is_none() {
        return Err("no report is open (file.open a PBIX, PBIT or PBIP with a report)".into());
    }
    match opt_str(p, "format")? {
        None | Some("png") => {}
        Some(f) => return Err(format!("unsupported format `{f}` (only \"png\")")),
    }
    let scale = match p.get("scale") {
        None | Some(Value::Null) => 1.0,
        Some(v) => match v.as_f64() {
            Some(x) if x.is_finite() && (0.1..=8.0).contains(&x) => x as f32,
            _ => return Err("`scale` must be a number from 0.1 to 8".into()),
        },
    };
    let page_sel = match p.get("page") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(_) => return Err("`page` must be a page name, display name or 0-based index".into()),
    };
    let mut dirs: Vec<PathBuf> = match p.get("font_dirs") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| {
                x.as_str()
                    .map(PathBuf::from)
                    .ok_or("`font_dirs` must be a list of folder paths")
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("`font_dirs` must be a list of folder paths".into()),
    };
    if let Some(env) = bicraft_env::var_os("BICRAFT_FONT_DIRS") {
        dirs.extend(std::env::split_paths(&env));
    }
    let system = opt_bool(p, "system_fonts")?.unwrap_or(true);
    let badges = opt_bool(p, "badges")?.unwrap_or(true);
    let text_dump = opt_bool(p, "text_dump")?.unwrap_or(false);
    let lcd_text = opt_bool(p, "lcd_text")?.unwrap_or(false);
    let high_contrast = opt_bool(p, "high_contrast")?.unwrap_or(false);
    let out = match opt_str(p, "out")? {
        // "" would name the working directory: never a silent write there
        Some(o) if o.trim().is_empty() => {
            return Err(
                "`out` is empty: give a .png file or a folder, or leave `out` out for inline PNG data".into(),
            );
        }
        o => o.map(PathBuf::from),
    };

    let limits = SnapshotLimits {
        data_budget_ms: DATA_BUDGET_MS,
        ..SnapshotLimits::default()
    };
    // `ui_language`, `culture` (model translations, M10.4: one without translations shows the
    // base captions, even when the session has a display culture) and `as_of` (the date TODAY()
    // returns) apply to this render only: overrides on this thread, the session is not changed
    // (`call_scope.rs`), so a render can run on a shared session.
    let display = (culture.is_some() || !warnings.is_empty()).then_some(culture);
    let snap = {
        let _view = crate::call_scope::view(ui_language, display, as_of);
        let _ki = crate::call_scope::key_influencers(key_influencers);
        s.snapshot_with(&limits)
    };
    let pages: Vec<usize> = match &page_sel {
        Some(sel) => vec![
            bicraft_render::find_page(&snap, sel)
                .ok_or_else(|| format!("no page `{sel}` (use a page name, display name or 0-based index)"))?,
        ],
        None => (0..bicraft_render::page_count(&snap)).collect(),
    };
    let assets: HashMap<String, Arc<[u8]>> =
        s.snapshot_assets().into_iter().map(|a| (a.id, a.bytes)).collect();
    let fonts = FontBook::with(dirs, system);
    let opts = RenderOptions {
        scale,
        system_fonts: system,
        font_dirs: Vec::new(),
        badges,
        text_dump,
        lcd_text,
        high_contrast,
        culture: format_culture,
        ..RenderOptions::default()
    };

    // where each page goes
    let single = pages.len() == 1 && page_sel.is_some();
    let target = |i: usize, name: &str| -> Option<PathBuf> {
        let o = out.as_ref()?;
        let is_png = o.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"));
        Some(if single && is_png {
            o.clone()
        } else if is_png {
            let stem = o.file_stem().and_then(|x| x.to_str()).unwrap_or("page");
            o.with_file_name(format!("{stem}-{}.png", i + 1))
        } else {
            o.join(format!("{:02}-{}.png", i + 1, slug(name)))
        })
    };
    let mut results = Vec::new();
    let mut empty: Vec<String> = Vec::new();
    let mut inline_bytes = 0usize;
    for i in pages {
        let img = bicraft_render::render_page(&snap, i, &assets, &fonts, &opts).map_err(|e| e.to_string())?;
        let label = if img.display_name.is_empty() {
            img.page.clone()
        } else {
            img.display_name.clone()
        };
        let mut r = json!({
            "index": i,
            "page": img.page,
            "display_name": img.display_name,
            "width": img.width,
            "height": img.height,
            "scale": img.scale,
            "visuals": img.visuals.iter().map(visual_json).collect::<Vec<_>>(),
            "visuals_without_data": without_data(&img.visuals),
        });
        for v in img.visuals.iter().filter(|v| v.data.kind.without_data()) {
            empty.push(format!(
                "{label} / {} ({}): {}",
                v.name,
                v.visual_type,
                v.data.kind.as_str()
            ));
        }
        let texts = text_dump.then(|| texts_json(&img.texts));
        match target(i, &label) {
            Some(path) => {
                write_png(&path, &img.png)?;
                r["path"] = json!(path.display().to_string());
                r["bytes"] = json!(img.png.len());
                if let Some(t) = texts {
                    // next to the PNG, as the render oracle reads it (tools/render-oracle)
                    let tp = path.with_extension("json");
                    let doc = json!({"page": img.page, "texts": t});
                    write_png(&tp, doc.to_string().as_bytes())?;
                    r["text_path"] = json!(tp.display().to_string());
                }
            }
            None => {
                if let Some(t) = texts {
                    r["texts"] = t;
                }
                inline_bytes = inline_bytes.saturating_add(img.png.len());
                if inline_bytes > MAX_INLINE_BYTES {
                    return Err("the rendered pages are too large to return inline: pass `out`".into());
                }
                r["image"] = json!({"mime": "image/png", "base64": base64(&img.png)});
            }
        }
        results.push(r);
    }
    let drawn_by = if fonts.has_family("segoe ui") {
        "Segoe UI"
    } else {
        "Selawik (bundled Segoe UI substitute)"
    };
    if fail_on_empty && !empty.is_empty() {
        let n = empty.len();
        let mut list: Vec<String> = empty.into_iter().take(20).collect();
        if n > list.len() {
            list.push(format!("… and {} more", n - list.len()));
        }
        return Err(format!(
            "{n} visual(s) without data (fail_on_empty): {}",
            list.join("; ")
        ));
    }
    let mut r = json!({"pages": results, "font": drawn_by});
    // characters no font has: drawn as boxes, listed once per render (docs/render.md, "Text")
    let missing = fonts.missing_glyphs();
    if !missing.is_empty() {
        let shown: Vec<String> = missing
            .iter()
            .take(40)
            .map(|c| format!("{c} U+{:04X}", u32::from(*c)))
            .collect();
        let more = missing.len().saturating_sub(shown.len());
        warnings.push(format!(
            "{} character(s) have no glyph in the fonts and are drawn as boxes: {}{}",
            missing.len(),
            shown.join(", "),
            if more > 0 {
                format!(" … and {more} more")
            } else {
                String::new()
            }
        ));
        r["missing_glyphs"] = json!(
            missing
                .iter()
                .map(|c| json!({"char": c.to_string(), "codepoint": format!("U+{:04X}", u32::from(*c))}))
                .collect::<Vec<_>>()
        );
    }
    if !warnings.is_empty() {
        r["warnings"] = json!(warnings);
    }
    Ok(r)
}

/// One visual of the result: what was drawn (`status`, `note`) and what its evaluated dataset
/// holds (`data`, `rows`, `value_cells`, `blank_cells`; `docs/render.md`, "Data states").
fn visual_json(v: &bicraft_render::VisualNote) -> Value {
    let mut o = json!({
        "name": v.name, "type": v.visual_type, "status": v.status, "note": v.note,
        "data": v.data.kind.as_str(),
    });
    put_counts(&mut o, &v.data);
    o
}

/// `rows`, `value_cells` and `blank_cells` of a data state that has a dataset.
pub(crate) fn put_counts(o: &mut Value, st: &bicraft_render::DataState) {
    if let Some(rows) = st.rows {
        o["rows"] = json!(rows);
        o["value_cells"] = json!(st.value_cells);
        o["blank_cells"] = json!(st.blank_cells);
    }
}

/// Visuals whose `data` is `no_rows`, `blank_values` or `not_evaluated`.
fn without_data(visuals: &[bicraft_render::VisualNote]) -> usize {
    visuals.iter().filter(|v| v.data.kind.without_data()).count()
}

/// The drawn text runs (`text_dump`): string, font, px size, weight, colour and box in report
/// pixels.
fn texts_json(runs: &[bicraft_render::TextRun]) -> Value {
    let r1 = |x: f32| (f64::from(x) * 100.0).round() / 100.0;
    Value::Array(
        runs.iter()
            .map(|t| {
                json!({
                    "t": t.text,
                    "font": t.family,
                    "size": r1(t.size),
                    "weight": t.weight,
                    "italic": t.italic,
                    "color": format!("#{:02x}{:02x}{:02x}", t.color.0, t.color.1, t.color.2),
                    "alpha": r1(t.alpha),
                    "rotate": r1(t.rotate),
                    "x": r1(t.x),
                    "y": r1(t.y),
                    "w": r1(t.w),
                    "h": r1(t.h),
                })
            })
            .collect(),
    )
}

/// `view.set_as_of {as_of}`: the date and time `TODAY()`/`NOW()` and relative date filters use in
/// the viewer and `report.render` (null: the clock).
pub(crate) fn view_set_as_of(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["as_of"])?;
    if !p.as_object().is_some_and(|o| o.contains_key("as_of")) {
        return Err("`as_of` is required (a date such as \"2026-09-18\", or null for the clock)".into());
    }
    s.as_of = opt_str(p, "as_of")?
        .map(crate::preview::filters::parse_as_of)
        .transpose()?;
    Ok(json!({"as_of": s.as_of.map(as_of_text)}))
}

/// Days since 1899-12-30 as `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM:SS`.
pub(crate) fn as_of_text(x: f64) -> String {
    let days = x.floor();
    let (y, m, d) = bicraft_locale::datefmt::civil_from_days(days as i64 - 25_569);
    let secs = (((x - days) * 86_400.0).round() as i64).min(86_399);
    if secs == 0 {
        format!("{y:04}-{m:02}-{d:02}")
    } else {
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
            secs / 3600,
            (secs / 60) % 60,
            secs % 60
        )
    }
}

fn write_png(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_and_slug() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(slug("Sales / Overview"), "Sales___Overview");
        assert_eq!(slug("///"), "page");
    }

    fn shop() -> Session {
        let mut s = Session::new();
        let mut run = |id: &str, p: Value| {
            s.execute(id, &p)
                .map_err(|e| format!("{id}: {e}"))
                .map(|_| ())
                .unwrap_or_else(|e| panic!("{e}"))
        };
        run("model.new", json!({"name": "Shop"}));
        run(
            "table.import",
            json!({"name": "Sales", "csv": "Region,Amount\nNorth,120\nSouth,80\nEast,45\n"}),
        );
        run(
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
        );
        run("report.new", json!({}));
        drop(run);
        let page = s.report.as_ref().unwrap().pages[0].display_name.clone();
        for (t, x) in [("card", 10), ("clusteredColumnChart", 320)] {
            s.execute(
                "visual.add",
                &json!({"page": page, "visual_type": t, "x": x, "y": 10, "width": 300, "height": 200,
                        "fields": if t == "card" { json!([{"role": "Values", "field": "[Total]"}]) }
                                  else { json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]) }}),
            )
            .unwrap();
        }
        s
    }

    /// An empty `out` once wrote the PNGs into the process's working directory (found by
    /// `every_command_survives_empty_and_hostile_params`).
    #[test]
    fn an_empty_out_is_refused() {
        let mut s = shop();
        for out in ["", " ", "\t"] {
            let e = s
                .execute(
                    "report.render",
                    &json!({"page": 0, "out": out, "system_fonts": false}),
                )
                .unwrap_err();
            assert!(e.contains("`out` is empty"), "{e}");
        }
    }

    #[test]
    fn renders_inline_and_to_files() {
        let mut s = shop();
        let rev = s.revision;
        let r = s
            .execute("report.render", &json!({"page": 0, "system_fonts": false}))
            .unwrap();
        assert_eq!(s.revision, rev, "read-only");
        let p = &r["pages"][0];
        assert_eq!(
            (p["width"].as_u64(), p["height"].as_u64()),
            (Some(1280), Some(720))
        );
        assert!(p["image"]["base64"].as_str().unwrap().starts_with("iVBORw0KGgo"));
        let st: Vec<&str> = p["visuals"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["status"].as_str().unwrap())
            .collect();
        assert_eq!(st, vec!["drawn", "drawn"], "{r}");

        let dir = std::env::temp_dir().join(format!("bicraft-render-{}", std::process::id()));
        let r = s
            .execute(
                "report.render",
                &json!({"out": dir.to_str().unwrap(), "scale": 0.5, "system_fonts": false}),
            )
            .unwrap();
        let path = PathBuf::from(r["pages"][0]["path"].as_str().unwrap());
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"\x89PNG"));
        assert_eq!(r["pages"][0]["width"], 640);
        assert!(r["pages"][0].get("text_path").is_none());
        std::fs::remove_dir_all(&dir).ok();

        // text_dump: inline runs, or a .json beside each PNG
        let r = s
            .execute(
                "report.render",
                &json!({"page": 0, "system_fonts": false, "text_dump": true}),
            )
            .unwrap();
        let texts = r["pages"][0]["texts"].as_array().unwrap();
        assert!(!texts.is_empty(), "{r}");
        let t = &texts[0];
        assert!(
            t["t"].is_string()
                && t["size"].as_f64().unwrap() > 0.0
                && t["color"].as_str().unwrap().starts_with('#')
        );
        let r = s
            .execute(
                "report.render",
                &json!({"out": dir.to_str().unwrap(), "system_fonts": false, "text_dump": true}),
            )
            .unwrap();
        let tp = PathBuf::from(r["pages"][0]["text_path"].as_str().unwrap());
        let doc: Value = serde_json::from_slice(&std::fs::read(&tp).unwrap()).unwrap();
        assert_eq!(doc["texts"].as_array().map(Vec::len), Some(texts.len()));
        assert!(s.execute("report.render", &json!({"text_dump": "yes"})).is_err());
        assert!(s.execute("report.render", &json!({"lcd_text": 1})).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Characters no bundled font has (CJK) are listed once per render; Greek is drawn.
    #[test]
    fn missing_glyphs_are_listed() {
        let mut s = Session::new();
        for (id, p) in [
            ("model.new", json!({"name": "Shop"})),
            (
                "table.import",
                json!({"name": "Sales", "csv": "Region,Amount\n北,120\nΔ South,80\n"}),
            ),
            (
                "measure.add",
                json!({"table": "Sales", "name": "CA Δ N-1", "expression": "SUM(Sales[Amount])"}),
            ),
            ("report.new", json!({})),
        ] {
            s.execute(id, &p).unwrap();
        }
        let page = s.report.as_ref().unwrap().pages[0].display_name.clone();
        s.execute(
            "visual.add",
            &json!({"page": page, "visual_type": "tableEx", "x": 10, "y": 10, "width": 400, "height": 200,
                    "fields": [{"role": "Values", "field": "Sales[Region]"}, {"role": "Values", "field": "[CA Δ N-1]"}]}),
        )
        .unwrap();
        let r = s
            .execute("report.render", &json!({"page": 0, "system_fonts": false}))
            .unwrap();
        let m: Vec<&str> = r["missing_glyphs"]
            .as_array()
            .unwrap_or_else(|| panic!("{r}"))
            .iter()
            .map(|x| x["char"].as_str().unwrap())
            .collect();
        assert_eq!(m, vec!["北"], "{r}");
        assert_eq!(r["missing_glyphs"][0]["codepoint"], "U+5317");
        let w = r["warnings"].to_string();
        assert!(w.contains("北 U+5317") && w.contains("drawn as boxes"), "{w}");
    }

    /// The same input gives byte-identical PNGs and text runs, across fresh sessions (each with
    /// its own hash seeds): tied sort values, a two-table grouping, a table and a slicer.
    #[test]
    fn renders_are_deterministic() {
        fn build() -> Session {
            let mut s = Session::new();
            let mut run = |id: &str, p: Value| {
                s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
            };
            run("model.new", json!({"name": "Ties"}));
            run(
                "table.import",
                json!({"name": "Region", "csv": "Region,Zone\nNorth,A\nSouth,B\nEast,A\nWest,B\nMid,C\n"}),
            );
            run(
                "table.import",
                json!({"name": "Sales", "csv": "Region,Day,Amount\nNorth,2026-01-01,10\nSouth,2026-01-01,10\nEast,2026-01-02,10\nWest,2026-01-02,20\nMid,2026-01-03,20\nNorth,2026-01-03,5\nSouth,2026-01-03,5\n"}),
            );
            run(
                "relationship.add",
                json!({"from": "Sales[Region]", "to": "Region[Region]"}),
            );
            run(
                "measure.add",
                json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
            );
            run("report.new", json!({}));
            drop(run);
            let page = s.report.as_ref().unwrap().pages[0].display_name.clone();
            let visuals = [
                (
                    "clusteredBarChart",
                    json!([{"role": "Category", "field": "Region[Zone]"}, {"role": "Y", "field": "[Total]"}]),
                ),
                (
                    "clusteredColumnChart",
                    json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]),
                ),
                (
                    "tableEx",
                    json!([{"role": "Values", "field": "Sales[Day]"}, {"role": "Values", "field": "Region[Zone]"}, {"role": "Values", "field": "[Total]"}]),
                ),
                ("slicer", json!([{"role": "Values", "field": "Region[Region]"}])),
            ];
            for (i, (t, fields)) in visuals.into_iter().enumerate() {
                let x = 10 + 310 * (i % 2);
                let y = 10 + 230 * (i / 2);
                s.execute(
                    "visual.add",
                    &json!({"page": page, "visual_type": t, "x": x, "y": y, "width": 300, "height": 220, "fields": fields}),
                )
                .unwrap();
            }
            s
        }
        let render = || {
            let mut s = build();
            let r = s
                .execute(
                    "report.render",
                    &json!({"page": 0, "system_fonts": false, "text_dump": true, "as_of": "2026-01-10"}),
                )
                .unwrap();
            let st: Vec<&str> = r["pages"][0]["visuals"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["status"].as_str().unwrap())
                .collect();
            assert!(st.iter().all(|x| *x == "drawn"), "{r}");
            (
                r["pages"][0]["image"]["base64"].as_str().unwrap().to_string(),
                r["pages"][0]["texts"].to_string(),
            )
        };
        let first = render();
        for _ in 0..3 {
            let again = render();
            assert_eq!(again.1, first.1, "text runs differ between runs");
            assert!(again.0 == first.0, "PNG bytes differ between runs");
        }
    }

    /// `data` per visual from the evaluated dataset, whatever the UI language: a card on a
    /// `BLANK()` measure, a chart whose filter removes every row, a normal card, a text box.
    #[test]
    fn data_states_flag_empty_visuals() {
        let mut s = shop();
        s.execute(
            "measure.add",
            &json!({"table": "Sales", "name": "Nothing", "expression": "BLANK()"}),
        )
        .unwrap();
        let page = s.report.as_ref().unwrap().pages[0].display_name.clone();
        let mut add = |t: &str, x: i64, y: i64, fields: Value| -> String {
            let r = s
                .execute(
                    "visual.add",
                    &json!({"page": page, "visual_type": t, "x": x, "y": y, "width": 300, "height": 200, "fields": fields}),
                )
                .unwrap();
            r["visual"].as_str().or(r["name"].as_str()).unwrap().to_string()
        };
        let blank_card = add("card", 10, 230, json!([{"role": "Values", "field": "[Nothing]"}]));
        let chart = add(
            "clusteredColumnChart",
            320,
            230,
            json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]),
        );
        let text = add("textbox", 640, 230, json!([]));
        s.execute(
            "visual.filter",
            &json!({"page": page, "visual": chart, "field": "Sales[Region]", "in": ["Nowhere"]}),
        )
        .unwrap();
        for lang in ["en-US", "fr-FR"] {
            let r = s
                .execute(
                    "report.render",
                    &json!({"page": 0, "system_fonts": false, "ui_language": lang}),
                )
                .unwrap();
            let p = &r["pages"][0];
            let by = |name: &str| {
                p["visuals"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|v| v["name"] == name)
                    .cloned()
                    .unwrap_or_else(|| panic!("{name} missing: {r}"))
            };
            let v = by(&blank_card);
            assert_eq!(
                (v["status"].as_str(), v["data"].as_str()),
                (Some("drawn"), Some("blank_values")),
                "{v}"
            );
            assert_eq!(v["blank_cells"], v["value_cells"], "{v}");
            let v = by(&chart);
            assert_eq!(
                (v["status"].as_str(), v["data"].as_str(), v["rows"].as_u64()),
                (Some("drawn"), Some("no_rows"), Some(0)),
                "{v}"
            );
            let v = by(&text);
            assert_eq!(v["data"], "not_bound", "{v}");
            assert!(v.get("rows").is_none(), "{v}");
            // the two cards and chart of shop(): data
            let oks = p["visuals"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|v| v["data"] == "ok")
                .map(|v| (v["rows"].as_u64(), v["blank_cells"].as_u64()))
                .collect::<Vec<_>>();
            assert_eq!(oks, vec![(Some(1), Some(0)), (Some(3), Some(0))], "{r}");
            assert_eq!(p["visuals_without_data"], 2, "{r}");
        }
        // the live viewer's snapshot carries the same state
        let snap = s.snapshot_with(&SnapshotLimits::default());
        let vs = snap["report"]["pages"][0]["visuals"].as_array().unwrap();
        let state = |name: &str| vs.iter().find(|v| v["name"] == name).unwrap()["data_state"].clone();
        assert_eq!(state(&blank_card)["state"], "blank_values");
        assert_eq!(state(&chart)["state"], "no_rows");
        assert_eq!(state(&chart)["rows"], 0);
        assert_eq!(state(&text), json!({"state": "not_bound"}));
        // fail_on_empty: an error naming the empty visuals; fine once they are gone
        let e = s
            .execute(
                "report.render",
                &json!({"page": 0, "system_fonts": false, "fail_on_empty": true}),
            )
            .unwrap_err();
        assert!(
            e.contains("2 visual(s) without data") && e.contains(&chart) && e.contains("no_rows"),
            "{e}"
        );
        for v in [&blank_card, &chart] {
            s.execute("visual.remove", &json!({"page": page, "visual": v}))
                .unwrap();
        }
        let r = s
            .execute(
                "report.render",
                &json!({"page": 0, "system_fonts": false, "fail_on_empty": true}),
            )
            .unwrap();
        assert_eq!(r["pages"][0]["visuals_without_data"], 0, "{r}");
        assert!(s.execute("report.render", &json!({"fail_on_empty": 1})).is_err());
    }

    /// `high_contrast` draws the app's high-contrast palette: a different picture, and every
    /// text run at 7:1 or more against the page, which turns black; the report is not changed.
    #[test]
    fn high_contrast_draws_the_high_contrast_palette() {
        fn lum(hex: &str) -> f64 {
            let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap();
            let lin = |c: u32| {
                let c = f64::from(c) / 255.0;
                if c <= 0.03928 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * lin(n >> 16 & 255) + 0.7152 * lin(n >> 8 & 255) + 0.0722 * lin(n & 255)
        }
        let ratio = |a: f64, b: f64| (a.max(b) + 0.05) / (a.min(b) + 0.05);
        let mut s = shop();
        let rev = s.revision;
        let render = |s: &mut Session, hc: bool| {
            s.execute(
                "report.render",
                &json!({"page": 0, "system_fonts": false, "text_dump": true, "high_contrast": hc}),
            )
            .unwrap()
        };
        let plain = render(&mut s, false);
        let hc = render(&mut s, true);
        assert_eq!(s.revision, rev, "read-only");
        let png = |r: &Value| r["pages"][0]["image"]["base64"].as_str().unwrap().to_string();
        assert_ne!(png(&plain), png(&hc), "the option changes the picture");
        let colours = |r: &Value| -> Vec<String> {
            r["pages"][0]["texts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["color"].as_str().unwrap().to_string())
                .collect()
        };
        let (plain_c, hc_c) = (colours(&plain), colours(&hc));
        assert!(!hc_c.is_empty());
        // on the black page every run reads at 7:1 (WCAG 1.4.6); by default dark text on white
        for c in &hc_c {
            assert!(ratio(lum(c), 0.0) >= 7.0, "{c} on black");
        }
        assert!(plain_c.iter().any(|c| ratio(lum(c), 0.0) < 7.0), "{plain_c:?}");
        assert!(
            s.execute("report.render", &json!({"high_contrast": "on"}))
                .is_err()
        );
    }

    #[test]
    fn bad_params_are_refused() {
        let mut s = Session::new();
        assert!(
            s.execute("report.render", &json!({}))
                .unwrap_err()
                .contains("no report")
        );
        let mut s = shop();
        assert!(
            s.execute("report.render", &json!({"page": "Nope"}))
                .unwrap_err()
                .contains("no page")
        );
        assert!(s.execute("report.render", &json!({"scale": 100})).is_err());
        assert!(s.execute("report.render", &json!({"format": "jpg"})).is_err());
        assert!(s.execute("report.render", &json!({"pages": 1})).is_err());
        assert!(
            s.execute("report.render", &json!({"format_culture": "zz-Nope"}))
                .unwrap_err()
                .contains("format_culture")
        );
        assert!(
            s.execute(
                "report.render",
                &json!({"page": 0, "system_fonts": false, "format_culture": "fr-FR"})
            )
            .is_ok()
        );
    }
}
