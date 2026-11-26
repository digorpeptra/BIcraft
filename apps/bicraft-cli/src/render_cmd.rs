//! `bicraft render FILE [--page NAME|INDEX] [--out PATH] [--scale N] [--font-dir DIR]...
//! [--no-system-fonts] [--no-badges] [--data-root DIR] [--lang TAG] [--culture NAME]
//! [--format-culture NAME] [--text-json] [--lcd-text] [--fail-on-empty] [--cache-abf CACHE.abf]
//! [--key-influencers placeholder|estimate]`: `report.render` to PNG files
//! (`--cache-abf`: a PBIP's data read from that file, read-only, instead of its `.pbi/cache.abf`); `--lang` is the UI
//! language of the generated visual text (auto titles, (Blank), …), `--culture` a model culture
//! whose translated captions name the fields (one without translations, or the model's own
//! culture, renders the stored names and the result lists a warning); numbers and dates follow
//! the model's culture, or `--format-culture`; `--fail-on-empty` exits 1 (after writing) when a
//! visual has no data (`visuals_without_data`).

use std::path::Path;

use bicraft_engine::Session;
use serde_json::{Value, json};

use super::{Failure, INVALID, OK, USAGE_ERROR, print_json, usage};

pub(crate) fn render(args: &[String]) -> Result<u8, Failure> {
    let mut path: Option<&String> = None;
    let mut params = serde_json::Map::new();
    let mut dirs: Vec<Value> = Vec::new();
    let mut data_root: Option<&String> = None;
    let mut cache_abf: Option<&String> = None;
    let mut fail_on_empty = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| usage(format!("{name} needs a value")));
        match a.as_str() {
            "--page" => {
                params.insert("page".into(), json!(val("--page")?));
            }
            "--out" | "-o" => {
                params.insert("out".into(), json!(val("--out")?));
            }
            "--scale" => {
                let s = val("--scale")?;
                let x: f64 = s
                    .parse()
                    .map_err(|_| usage(format!("--scale: `{s}` is not a number")))?;
                params.insert("scale".into(), json!(x));
            }
            "--font-dir" => dirs.push(json!(val("--font-dir")?)),
            "--data-root" => data_root = Some(val("--data-root")?),
            "--cache-abf" => cache_abf = Some(val("--cache-abf")?),
            "--lang" => {
                params.insert("ui_language".into(), json!(val("--lang")?));
            }
            "--culture" => {
                params.insert("culture".into(), json!(val("--culture")?));
            }
            "--as-of" => {
                params.insert("as_of".into(), json!(val("--as-of")?));
            }
            "--format-culture" => {
                params.insert("format_culture".into(), json!(val("--format-culture")?));
            }
            "--key-influencers" => {
                params.insert("key_influencers".into(), json!(val("--key-influencers")?));
            }
            "--no-system-fonts" => {
                params.insert("system_fonts".into(), json!(false));
            }
            "--no-badges" => {
                params.insert("badges".into(), json!(false));
            }
            "--lcd-text" => {
                params.insert("lcd_text".into(), json!(true));
            }
            "--text-json" => {
                params.insert("text_dump".into(), json!(true));
            }
            "--fail-on-empty" => fail_on_empty = true,
            s if s.starts_with('-') => return Err(usage(format!("render: unknown option {s}"))),
            _ if path.is_none() => path = Some(a),
            _ => return Err(usage("render takes one FILE")),
        }
    }
    let path = path.ok_or_else(|| usage("render needs a FILE (.pbix, .pbit or .pbip / project folder)"))?;
    if !dirs.is_empty() {
        params.insert("font_dirs".into(), Value::Array(dirs));
    }
    if !params.contains_key("out") {
        // default: next to the current directory, named after the file
        let stem = Path::new(path.trim_end_matches(['/', '\\']))
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("report")
            .to_string();
        let out = if params.contains_key("page") {
            format!("{stem}.png")
        } else {
            format!("{stem}-pages")
        };
        params.insert("out".into(), json!(out));
    }
    let mut session = Session::new();
    let mut open = json!({"path": path});
    if let Some(d) = data_root {
        open["data_root"] = json!(d);
    }
    if let Some(c) = cache_abf {
        open["cache_abf"] = json!(c);
    }
    session
        .execute("file.open", &open)
        .map_err(|e| (USAGE_ERROR, e))?;
    if session.report.is_none() {
        let why = session.report_error.as_deref().unwrap_or("no report found");
        return Err((USAGE_ERROR, format!("{path}: cannot read the report: {why}")));
    }
    let v = session
        .execute("report.render", &Value::Object(params))
        .map_err(|e| (USAGE_ERROR, e))?;
    print_json(&v);
    // --fail-on-empty: the JSON and PNGs are still written; the exit code says a visual had
    // no data (no_rows, blank_values or not_evaluated, `docs/render.md`, "Data states")
    let empty: u64 = v["pages"]
        .as_array()
        .map(|ps| {
            ps.iter()
                .filter_map(|p| p["visuals_without_data"].as_u64())
                .fold(0u64, u64::saturating_add)
        })
        .unwrap_or(0);
    if fail_on_empty && empty > 0 {
        eprintln!("bicraft render: {empty} visual(s) without data (--fail-on-empty)");
        return Ok(INVALID);
    }
    Ok(OK)
}

/// `bicraft export-pdf FILE [--out FILE.pdf] [--pages all|current|2-4|NAME[,NAME]] [--hidden]
/// [--title TEXT] [--scale N] [--font-dir DIR]... [--no-system-fonts] [--data-root DIR]
/// [--lang TAG] [--format-culture NAME] [--as-of DATE]`: `report.export_pdf`, one PDF of the
/// report's pages (default `FILE`'s stem `.pdf` in the current directory, every page not
/// hidden).
pub(crate) fn export_pdf(args: &[String]) -> Result<u8, Failure> {
    let mut path: Option<&String> = None;
    let mut params = serde_json::Map::new();
    let mut dirs: Vec<Value> = Vec::new();
    let mut data_root: Option<&String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| usage(format!("{name} needs a value")));
        match a.as_str() {
            "--out" | "-o" => {
                params.insert("out".into(), json!(val("--out")?));
            }
            "--pages" => {
                let p = val("--pages")?;
                // a list of names or indexes, or one selector ("all", "current", "2-4", a name)
                let v = if p.contains(',')
                    && !p
                        .chars()
                        .all(|c| c.is_ascii_digit() || c == ',' || c == '-' || c == ' ')
                {
                    Value::Array(p.split(',').map(|x| json!(x.trim())).collect())
                } else {
                    json!(p)
                };
                params.insert("pages".into(), v);
            }
            "--hidden" => {
                params.insert("hidden".into(), json!(true));
            }
            "--title" => {
                params.insert("title".into(), json!(val("--title")?));
            }
            "--scale" => {
                let s = val("--scale")?;
                let x: f64 = s
                    .parse()
                    .map_err(|_| usage(format!("--scale: `{s}` is not a number")))?;
                params.insert("scale".into(), json!(x));
            }
            "--font-dir" => dirs.push(json!(val("--font-dir")?)),
            "--data-root" => data_root = Some(val("--data-root")?),
            "--lang" => {
                params.insert("ui_language".into(), json!(val("--lang")?));
            }
            "--format-culture" => {
                params.insert("format_culture".into(), json!(val("--format-culture")?));
            }
            "--as-of" => {
                params.insert("as_of".into(), json!(val("--as-of")?));
            }
            "--no-system-fonts" => {
                params.insert("system_fonts".into(), json!(false));
            }
            s if s.starts_with('-') => return Err(usage(format!("export-pdf: unknown option {s}"))),
            _ if path.is_none() => path = Some(a),
            _ => return Err(usage("export-pdf takes one FILE")),
        }
    }
    let path =
        path.ok_or_else(|| usage("export-pdf needs a FILE (.pbix, .pbit or .pbip / project folder)"))?;
    if !dirs.is_empty() {
        params.insert("font_dirs".into(), Value::Array(dirs));
    }
    if !params.contains_key("out") {
        let stem = Path::new(path.trim_end_matches(['/', '\\']))
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("report")
            .to_string();
        params.insert("out".into(), json!(format!("{stem}.pdf")));
    }
    let mut session = Session::new();
    let mut open = json!({"path": path});
    if let Some(d) = data_root {
        open["data_root"] = json!(d);
    }
    session
        .execute("file.open", &open)
        .map_err(|e| (USAGE_ERROR, e))?;
    if session.report.is_none() {
        let why = session.report_error.as_deref().unwrap_or("no report found");
        return Err((USAGE_ERROR, format!("{path}: cannot read the report: {why}")));
    }
    let v = session
        .execute("report.export_pdf", &Value::Object(params))
        .map_err(|e| (USAGE_ERROR, e))?;
    print_json(&v);
    Ok(OK)
}
