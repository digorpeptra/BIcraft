//! `report.export_pdf`: report pages to one PDF file, as Power BI Desktop's File > Export >
//! Export to PDF does, through the same snapshot and renderer as `report.render`
//! (`docs/render.md`, "PDF"): one PDF page per report page at its size, shapes and charts as
//! vector paths, text as real text with embedded font subsets (selectable and searchable).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use bicraft_render::{FontBook, RenderOptions};
use serde_json::{Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, opt_str};
use crate::snapshot::SnapshotLimits;

/// Time the data evaluation may take for the whole export.
const DATA_BUDGET_MS: u64 = 120_000;
/// Most pages one export writes.
const MAX_PAGES: usize = 500;

/// A page of the snapshot: its name, display name and whether it is hidden.
struct PageRef {
    name: String,
    display: String,
    hidden: bool,
}

fn snapshot_pages(snap: &Value) -> Vec<PageRef> {
    snap.pointer("/report/pages")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|p| PageRef {
                    name: p.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                    display: p
                        .get("display_name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    hidden: p.get("hidden").and_then(Value::as_bool) == Some(true),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The index of the page named `sel` (name, display name, or a 0-based index).
fn page_index(pages: &[PageRef], sel: &Value) -> Result<usize, String> {
    match sel {
        Value::Number(n) => n
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .filter(|i| *i < pages.len())
            .ok_or_else(|| format!("no page {n} (0-based; the report has {} pages)", pages.len())),
        Value::String(s) => pages
            .iter()
            .position(|p| p.name == *s)
            .or_else(|| pages.iter().position(|p| p.display.eq_ignore_ascii_case(s)))
            .ok_or_else(|| format!("no page `{}`", s.chars().take(80).collect::<String>())),
        _ => Err("a page is a name, a display name or a 0-based index".into()),
    }
}

/// A 1-based range ("2-4", "3", "1-") over `n` pages, as a print dialog takes it.
fn range(s: &str, n: usize) -> Option<Vec<usize>> {
    let mut out = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (a.trim(), b.trim()),
            None => (part, part),
        };
        let a: usize = if a.is_empty() { 1 } else { a.parse().ok()? };
        let b: usize = if b.is_empty() { n } else { b.parse().ok()? };
        if a == 0 || b < a || b > n {
            return None;
        }
        out.extend((a - 1)..b);
    }
    Some(out)
}

/// The pages to export (indexes in order) from `pages` and `hidden`.
fn pick(
    pages: &[PageRef],
    sel: Option<&Value>,
    hidden: bool,
    active: Option<&str>,
) -> Result<Vec<usize>, String> {
    let visible = |i: &usize| hidden || pages.get(*i).is_some_and(|p| !p.hidden);
    let picked: Vec<usize> = match sel {
        None | Some(Value::Null) => (0..pages.len()).filter(visible).collect(),
        Some(Value::String(s)) if s == "all" => (0..pages.len()).filter(visible).collect(),
        Some(Value::String(s)) if s == "current" => {
            let i = active
                .and_then(|a| pages.iter().position(|p| p.name == a))
                .or_else(|| (0..pages.len()).find(visible))
                .ok_or("the report has no page to export")?;
            vec![i]
        }
        Some(Value::String(s))
            if s.chars()
                .all(|c| c.is_ascii_digit() || c == '-' || c == ',' || c == ' ')
                && s.contains('-') =>
        {
            range(s, pages.len())
                .ok_or_else(|| {
                    format!(
                        "bad page range `{s}` (1-based, e.g. \"2-4\" or \"1,3-5\"; the report has {} pages)",
                        pages.len()
                    )
                })?
                .into_iter()
                .filter(visible)
                .collect()
        }
        Some(Value::Array(a)) => {
            if a.len() > MAX_PAGES {
                return Err(format!("at most {MAX_PAGES} pages per export"));
            }
            // pages named one by one are exported even when hidden
            a.iter().map(|x| page_index(pages, x)).collect::<Result<_, _>>()?
        }
        Some(x @ (Value::String(_) | Value::Number(_))) => vec![page_index(pages, x)?],
        Some(_) => {
            return Err(
                "`pages` is \"all\", \"current\", a 1-based range (\"2-4\"), a page name or index, or a list of them"
                    .into(),
            );
        }
    };
    let mut seen = std::collections::HashSet::new();
    Ok(picked
        .into_iter()
        .filter(|i| seen.insert(*i))
        .take(MAX_PAGES)
        .collect())
}

/// `report.export_pdf {out, pages?, hidden?, …}`.
pub(crate) fn report_export_pdf(s: &Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "out",
            "pages",
            "hidden",
            "title",
            "scale",
            "font_dirs",
            "system_fonts",
            "ui_language",
            "format_culture",
            "as_of",
        ],
    )?;
    let out = match opt_str(p, "out")? {
        None => return Err("missing parameter `out` (the .pdf file to write)".into()),
        // "" would name the working directory: never a silent write there
        Some(o) if o.trim().is_empty() => return Err("`out` is empty: give the .pdf file to write".into()),
        Some(o) => PathBuf::from(o),
    };
    if out.is_dir() {
        return Err(format!("`out` is a folder ({}): give a .pdf file", out.display()));
    }
    let hidden = opt_bool(p, "hidden")?.unwrap_or(false);
    let ui_language = crate::ui_lang::param(p)?;
    let as_of = opt_str(p, "as_of")?
        .map(crate::preview::filters::parse_as_of)
        .transpose()?;
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
    let scale = match p.get("scale") {
        None | Some(Value::Null) => 2.0,
        Some(v) => match v.as_f64() {
            Some(x) if x.is_finite() && (1.0..=4.0).contains(&x) => x as f32,
            _ => return Err("`scale` (the resolution of images) must be a number from 1 to 4".into()),
        },
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
    let report = s
        .report
        .as_ref()
        .ok_or("no report is open (file.open a PBIX, PBIT or PBIP with a report)")?;
    let title = match opt_str(p, "title")? {
        Some(t) => t.chars().take(500).collect(),
        None => match &s.source {
            Some(
                crate::Source::Package { path, .. }
                | crate::Source::Project { path, .. }
                | crate::Source::Bim { path },
            ) => path
                .file_stem()
                .and_then(|x| x.to_str())
                .unwrap_or("Report")
                .to_string(),
            None => "Report".to_string(),
        },
    };
    let active = report.active_page.clone();

    let limits = SnapshotLimits {
        data_budget_ms: DATA_BUDGET_MS,
        ..SnapshotLimits::default()
    };
    let snap = {
        let _view = crate::call_scope::view(ui_language, None, as_of);
        s.snapshot_with(&limits)
    };
    let pages = snapshot_pages(&snap);
    let picked = pick(&pages, p.get("pages"), hidden, active.as_deref())?;
    if picked.is_empty() {
        return Err(if pages.is_empty() {
            "the report has no page".into()
        } else {
            "no page to export (every page picked is hidden: pass `hidden: true` to include them)".into()
        });
    }
    let assets: HashMap<String, Arc<[u8]>> =
        s.snapshot_assets().into_iter().map(|a| (a.id, a.bytes)).collect();
    let fonts = FontBook::with(dirs, system);
    let opts = RenderOptions {
        scale,
        system_fonts: system,
        culture: format_culture,
        ..RenderOptions::default()
    };
    let doc = bicraft_render::render_pdf(&snap, &picked, &assets, &fonts, &opts, &title)
        .map_err(|e| e.to_string())?;
    if let Some(dir) = out.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    std::fs::write(&out, &doc.bytes).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    let mut warnings: Vec<String> = Vec::new();
    let not_embedded: Vec<&str> = doc
        .fonts
        .iter()
        .filter(|f| !f.embedded)
        .map(|f| f.name.as_str())
        .collect();
    if !not_embedded.is_empty() {
        warnings.push(format!(
            "the licence of {} forbids embedding: its text is drawn as outlines (still searchable)",
            not_embedded.join(", ")
        ));
    }
    let raster: usize = doc.pages.iter().map(|p| p.raster_parts).sum();
    if raster > 0 {
        warnings.push(format!(
            "{raster} part(s) too large to draw as vectors are placed as images"
        ));
    }
    let missing = fonts.missing_glyphs();
    if !missing.is_empty() {
        let shown: String = missing.iter().take(40).collect();
        warnings.push(format!(
            "{} character(s) have no glyph in the fonts and are drawn as boxes: {shown}",
            missing.len()
        ));
    }
    let mut r = json!({
        "path": out.display().to_string(),
        "bytes": doc.bytes.len(),
        "title": title,
        "pages": doc.pages.iter().map(|pg| json!({
            "index": pg.index,
            "page": pg.page,
            "display_name": pg.display_name,
            "width_pt": pg.width_pt,
            "height_pt": pg.height_pt,
            "visuals": pg.visuals.iter().map(|v| json!({"name": v.name, "type": v.visual_type, "status": v.status})).collect::<Vec<_>>(),
            "raster_parts": pg.raster_parts,
        })).collect::<Vec<_>>(),
        "fonts": doc.fonts.iter().map(|f| json!({"name": f.name, "embedded": f.embedded, "glyphs": f.glyphs})).collect::<Vec<_>>(),
        "skipped_hidden": pages.iter().filter(|x| x.hidden).count().saturating_sub(picked.iter().filter(|i| pages.get(**i).is_some_and(|x| x.hidden)).count()),
    });
    if !warnings.is_empty() {
        r["warnings"] = json!(warnings);
    }
    Ok(r)
}

/// Most print files tried in the print folder before giving up (earlier ones may be open in a
/// PDF viewer that locks them).
const MAX_PRINT_NAMES: usize = 50;

/// A file name for `title`: letters, digits, spaces, `-` and `_` kept, the rest `_`.
fn safe_stem(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    let s = s.trim().to_string();
    if s.is_empty() { "report".into() } else { s }
}

/// Where a print's PDF goes on the desktop: `<temp>/bicraft-print/<title>.pdf`, or the first
/// `<title> (n).pdf` that is free or can be replaced.
fn print_path(title: &str) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("bicraft-print");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let stem = safe_stem(title);
    for n in 1..=MAX_PRINT_NAMES {
        let name = if n == 1 {
            format!("{stem}.pdf")
        } else {
            format!("{stem} ({n}).pdf")
        };
        let p = dir.join(name);
        if !p.exists() || std::fs::remove_file(&p).is_ok() {
            return Ok(p);
        }
    }
    Err(format!(
        "no free file name for the print in {} (close the PDFs printed before)",
        dir.display()
    ))
}

/// `report.print {pages?, hidden?, title?, out?, dry_run?}`: the pages as `report.export_pdf`
/// writes them, to a PDF handed to the operating system's default application for printing
/// (on a server, written only: the browser prints it).
pub(crate) fn report_print(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["pages", "hidden", "title", "out", "dry_run", "ui_language"])?;
    let dry_run = opt_bool(p, "dry_run")?.unwrap_or(false);
    let server = s.server_config().is_some();
    let mut q = serde_json::Map::new();
    for k in ["pages", "hidden", "title", "ui_language"] {
        if let Some(v) = p.get(k) {
            q.insert(k.into(), v.clone());
        }
    }
    let title_for_name = opt_str(p, "title")?.map(str::to_string);
    let out = match opt_str(p, "out")? {
        Some(o) if !o.trim().is_empty() => PathBuf::from(o),
        Some(_) => return Err("`out` is empty: give the .pdf file to write, or leave it out".into()),
        None if server => {
            return Err("missing parameter `out` (on a server, the workspace's .pdf file to write for the browser to print)".into());
        }
        None => {
            let stem = title_for_name.clone().unwrap_or_else(|| match &s.source {
                Some(
                    crate::Source::Package { path, .. }
                    | crate::Source::Project { path, .. }
                    | crate::Source::Bim { path },
                ) => path
                    .file_stem()
                    .and_then(|x| x.to_str())
                    .unwrap_or("report")
                    .to_string(),
                None => "report".into(),
            });
            print_path(&stem)?
        }
    };
    q.insert("out".into(), json!(out.display().to_string()));
    let mut r = report_export_pdf(s, &Value::Object(q))?;
    let path = PathBuf::from(r.get("path").and_then(Value::as_str).unwrap_or_default());
    let open = !dry_run && !server;
    if open {
        match &s.external_opener {
            Some(crate::desktop_open::ExternalOpener(o)) => o(&path)?,
            None => crate::desktop_open::open_pdf_with_default_app(&path)?,
        }
    }
    r["opened"] = json!(open);
    Ok(r)
}

#[cfg(test)]
#[path = "export_pdf_tests.rs"]
mod tests;
