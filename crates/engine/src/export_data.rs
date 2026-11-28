//! `visual.export_data`: a visual's data to CSV or XLSX, as Power BI's "More options (...) >
//! Export data" does (`docs/mcp.md`), and the report setting that allows it
//! (`report.settings {export_data}`: `report.json` `settings.exportDataMode`, the PBIR report
//! schema's values).
//!
//! - **Summarized data**: the fields of the visual in their order, aggregated as the visual
//!   aggregates them, under its filters, slicers and cross-filters; every level of a hierarchy.
//! - **Underlying data**: each aggregated column grouped instead of aggregated (a row per
//!   distinct combination), measures evaluated on each row. Only when the report allows it.
//! - **Data with current layout** (XLSX only): the summarized data in the visual's shape. A
//!   matrix with column fields is a cross table: a header row per column field (each label
//!   merged over its values), then the row fields' and values' names, then a row per distinct
//!   combination of the row fields with each column key's values; other visuals are the
//!   summarized table. Microsoft Learn (same page) says both exports have the same granularity,
//!   only the shape differs, and limits this one to 150,000 data intersections (cells of
//!   values). No subtotal or total rows.
//! - **Limits** (Microsoft Learn, "Export data from visualizations", read 2026-11-06): at most
//!   30,000 rows to CSV and 150,000 to XLSX; past them the file holds the first rows and the
//!   result says `truncated`.
//! - CSV is UTF-8 with a byte order mark, CRLF lines, RFC 4180 quoting; a text value starting
//!   with `=`, `+`, `-` or `@` gets a leading `'` so a spreadsheet does not run it as a formula.
//! - XLSX is a minimal SpreadsheetML workbook (ECMA-376: one sheet, inline strings, dates as
//!   serial numbers with a date format) written through `bicraft-opc`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use bicraft_report::Report;
use serde_json::{Value, json};

use crate::Session;
use crate::params::{allow, opt_str, str_param};

/// Most rows written to a CSV file (Power BI's limit).
pub(crate) const CSV_ROWS: usize = 30_000;
/// Most rows written to an XLSX file (Power BI's limit).
pub(crate) const XLSX_ROWS: usize = 150_000;
/// Most columns written.
const MAX_COLUMNS: usize = 1_000;

/// What a report lets its readers export (`settings.exportDataMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Summarized,
    SummarizedAndUnderlying,
    None,
}

impl Mode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Mode::Summarized => "AllowSummarized",
            Mode::SummarizedAndUnderlying => "AllowSummarizedAndUnderlying",
            Mode::None => "None",
        }
    }

    pub(crate) fn parse(s: &str) -> Option<Mode> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "allowsummarized" | "summarized" => Mode::Summarized,
            "allowsummarizedandunderlying" | "summarized_and_underlying" | "underlying" => {
                Mode::SummarizedAndUnderlying
            }
            "none" => Mode::None,
            _ => return None,
        })
    }
}

/// The report's export setting; a report without one allows summarized data (what Desktop
/// writes for a new report).
pub(crate) fn mode_of(r: &Report) -> Mode {
    r.extra
        .get("settings")
        .and_then(|s| s.get("exportDataMode"))
        .and_then(Value::as_str)
        .and_then(Mode::parse)
        .unwrap_or(Mode::Summarized)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Csv,
    Xlsx,
}

/// `visual.export_data {visual, page?, format?, kind?, out}`.
pub(crate) fn visual_export_data(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "format", "kind", "out"])?;
    let visual = str_param(p, "visual")?;
    let out = match opt_str(p, "out")? {
        None => return Err("missing parameter `out` (the .csv or .xlsx file to write)".into()),
        Some(o) if o.trim().is_empty() => {
            return Err("`out` is empty: give the .csv or .xlsx file to write".into());
        }
        Some(o) => PathBuf::from(o),
    };
    let ext = out
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let format = match (opt_str(p, "format")?, ext.as_deref()) {
        (Some("csv"), _) | (None, Some("csv")) => Format::Csv,
        (Some("xlsx"), _) | (None, Some("xlsx")) => Format::Xlsx,
        (None, _) => {
            return Err(
                "`format` is required when `out` is not a .csv or .xlsx file (\"csv\" or \"xlsx\")".into(),
            );
        }
        (Some(f), _) => {
            return Err(format!(
                "unknown `format` `{}` (\"csv\" or \"xlsx\")",
                f.chars().take(40).collect::<String>()
            ));
        }
    };
    let (underlying, layout) = match opt_str(p, "kind")? {
        None | Some("summarized") => (false, false),
        Some("underlying") => (true, false),
        Some("current_layout") => (false, true),
        Some(k) => {
            return Err(format!(
                "unknown `kind` `{}` (\"summarized\", \"underlying\" or \"current_layout\")",
                k.chars().take(40).collect::<String>()
            ));
        }
    };
    if layout && format == Format::Csv {
        return Err(
            "the current layout is exported to XLSX only (a CSV file has no layout): give a .xlsx `out` or `format: \"xlsx\"`"
                .into(),
        );
    }
    let base = s
        .report
        .as_ref()
        .ok_or("no report is open (file.open a PBIX, PBIT or PBIP with a report)")?;
    if s.model.is_none() {
        return Err("the session has no model: there is no data to export".into());
    }
    let mode = mode_of(base);
    match (mode, underlying) {
        (Mode::None, _) => {
            return Err(
                "the report does not allow exporting data (report settings: exportDataMode is None; report.settings {export_data} changes it)"
                    .into(),
            );
        }
        (Mode::Summarized, true) => {
            return Err(
                "the report allows summarized data only (report settings: exportDataMode is AllowSummarized; report.settings {export_data: \"AllowSummarizedAndUnderlying\"} allows underlying data)"
                    .into(),
            );
        }
        _ => {}
    }
    // the visual, on the page given or the only page that has one of that name
    let page_key = opt_str(p, "page")?;
    let found: Vec<(&bicraft_report::Page, &bicraft_report::Visual)> = base
        .pages
        .iter()
        .filter(|pg| {
            page_key
                .is_none_or(|k| pg.name.eq_ignore_ascii_case(k) || pg.display_name.eq_ignore_ascii_case(k))
        })
        .flat_map(|pg| {
            pg.visuals
                .iter()
                .filter(|v| v.name == visual)
                .map(move |v| (pg, v))
        })
        .collect();
    let (page, v) = match found.as_slice() {
        [one] => *one,
        [] => {
            return Err(match page_key {
                Some(k) => format!("no visual `{visual}` on page `{k}`"),
                None => format!("no visual `{visual}` in the report"),
            });
        }
        _ => return Err(format!("several pages have a visual `{visual}`: give `page`")),
    };
    if v.fields.is_empty() {
        return Err(format!("visual `{visual}` has no fields: nothing to export"));
    }
    // the reader's state (slicer selections, cross-filters), as the visual shows it
    let effective = s.interactions.effective(base);
    let data_report = effective.as_ref().map_or(base, |e| &e.data);
    let (page, v) = data_report
        .page(&page.name)
        .and_then(|pg| pg.visual(&v.name).map(|v| (pg, v)))
        .unwrap_or((page, v));
    let limit = match format {
        Format::Csv => CSV_ROWS,
        Format::Xlsx => XLSX_ROWS,
    };
    let d = crate::preview::export_rows(s, data_report, page, v, underlying, limit);
    if d.get("status").and_then(Value::as_str) != Some("ok") {
        let why = d
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("the visual's data could not be computed");
        return Err(format!("cannot export visual `{visual}`: {why}"));
    }
    let columns: Vec<&Value> = d
        .get("columns")
        .and_then(Value::as_array)
        .map(|a| a.iter().take(MAX_COLUMNS).collect())
        .unwrap_or_default();
    let rows: &[Value] = d.get("rows").and_then(Value::as_array).map_or(&[], Vec::as_slice);
    let truncated = d.get("truncated").and_then(Value::as_bool).unwrap_or(false);
    let total = d
        .get("total_rows")
        .and_then(Value::as_u64)
        .unwrap_or(rows.len() as u64);
    let cols: Vec<Col> = columns.iter().map(|c| Col::of(c)).collect();
    // the role of each exported column: the visual's fields, in the order export_rows keeps
    let mut roles: Vec<(&bicraft_report::FieldRef, &str)> = Vec::new();
    for f in &v.fields {
        if bicraft_report::query::sparkline(&f.field).is_none() && !roles.iter().any(|r| *r.0 == f.field) {
            roles.push((&f.field, f.role.as_str()));
        }
    }
    let roles: Vec<&str> = roles.into_iter().map(|r| r.1).collect();
    let mut truncated = truncated;
    let crossed = layout && v.visual_type.as_deref() == Some("pivotTable") && roles.contains(&"Columns");
    let bytes = match format {
        Format::Csv => csv(&cols, rows).into_bytes(),
        Format::Xlsx if crossed => {
            let grid = cross_table(&cols, &roles, rows);
            truncated |= grid.truncated;
            xlsx(&grid.head, &grid.cols, &grid.rows, &grid.merges, truncated)?
        }
        Format::Xlsx => {
            let head = vec![cols.iter().map(|c| c.name.clone()).collect::<Vec<_>>()];
            xlsx(&head, &cols, rows, &[], truncated)?
        }
    };
    write_file(&out, &bytes)?;
    let mut r = json!({
        "path": out.display().to_string(),
        "bytes": bytes.len(),
        "format": match format { Format::Csv => "csv", Format::Xlsx => "xlsx" },
        "kind": if underlying { "underlying" } else if layout { "current_layout" } else { "summarized" },
        "page": page.name,
        "visual": v.name,
        "columns": cols.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
        "rows": rows.len(),
        "total_rows": total,
        "limit": limit,
        "truncated": truncated,
        "export_data_mode": mode.as_str(),
    });
    if truncated {
        r["warnings"] = json!([format!(
            "the visual has {total} rows; the {} file holds the first {limit} (Power BI's limit)",
            match format {
                Format::Csv => "CSV",
                Format::Xlsx => "XLSX",
            }
        )]);
    }
    if d.get("filters_ignored").and_then(Value::as_bool) == Some(true) {
        let why = d
            .get("filters_ignored_reason")
            .and_then(Value::as_str)
            .unwrap_or("");
        if r.get("warnings").is_none() {
            r["warnings"] = json!([]);
        }
        if let Some(a) = r.get_mut("warnings").and_then(Value::as_array_mut) {
            a.push(json!(format!("some filters were not applied: {why}")));
        }
    }
    Ok(r)
}

/// Most data intersections (cells of values) a current-layout export writes (Microsoft Learn).
const LAYOUT_CELLS: usize = 150_000;

/// A matrix's data as a cross table: its header rows, its columns (row fields, then a value
/// per column key), its rows, the merged header cells (first row, first column, last column,
/// 0-based, of the header rows) and whether rows were left out.
struct Grid {
    head: Vec<Vec<String>>,
    cols: Vec<Col>,
    rows: Vec<Value>,
    merges: Vec<(usize, usize, usize)>,
    truncated: bool,
}

/// The cross table of a matrix's summarized `rows` (`roles` says which columns are row fields,
/// column fields and values): column keys and row keys in the order they first appear.
fn cross_table(cols: &[Col], roles: &[&str], rows: &[Value]) -> Grid {
    let pick = |want: &dyn Fn(&str) -> bool| -> Vec<usize> {
        (0..cols.len())
            .filter(|i| want(roles.get(*i).copied().unwrap_or("Values")))
            .collect()
    };
    let r_i = pick(&|r| r == "Rows");
    let c_i = pick(&|r| r == "Columns");
    let v_i = pick(&|r| r != "Rows" && r != "Columns");
    let key = |row: &Value, idx: &[usize]| -> Vec<Value> {
        idx.iter()
            .map(|i| row.get(*i).cloned().unwrap_or(Value::Null))
            .collect()
    };
    let mut col_keys: Vec<Vec<Value>> = Vec::new();
    let mut row_keys: Vec<Vec<Value>> = Vec::new();
    let mut cells: std::collections::HashMap<(usize, usize), Vec<Value>> = Default::default();
    let mut col_at: std::collections::HashMap<String, usize> = Default::default();
    let mut row_at: std::collections::HashMap<String, usize> = Default::default();
    for row in rows {
        let ck = key(row, &c_i);
        let rk = key(row, &r_i);
        let c = *col_at
            .entry(Value::from(ck.clone()).to_string())
            .or_insert_with(|| {
                col_keys.push(ck);
                col_keys.len() - 1
            });
        let r = *row_at
            .entry(Value::from(rk.clone()).to_string())
            .or_insert_with(|| {
                row_keys.push(rk);
                row_keys.len() - 1
            });
        cells.insert((r, c), key(row, &v_i));
    }
    let per_row = (col_keys.len() * v_i.len()).max(1);
    let keep = (LAYOUT_CELLS / per_row).min(row_keys.len());
    let truncated = keep < row_keys.len();
    let label = |v: &Value| match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let lead = r_i.len();
    let mut head = Vec::new();
    let mut merges = Vec::new();
    for (j, _) in c_i.iter().enumerate() {
        let mut line = vec![String::new(); lead];
        for (k, ck) in col_keys.iter().enumerate() {
            let first = lead + k * v_i.len();
            line.push(ck.get(j).map(label).unwrap_or_default());
            line.extend(std::iter::repeat_n(String::new(), v_i.len().saturating_sub(1)));
            if v_i.len() > 1 {
                merges.push((j, first, first + v_i.len() - 1));
            }
        }
        head.push(line);
    }
    let name = |i: &usize| cols.get(*i).map(|c| c.name.clone()).unwrap_or_default();
    let mut last: Vec<String> = r_i.iter().map(name).collect();
    let mut out_cols: Vec<Col> = r_i.iter().filter_map(|i| cols.get(*i).cloned()).collect();
    for _ in &col_keys {
        last.extend(v_i.iter().map(name));
        out_cols.extend(v_i.iter().filter_map(|i| cols.get(*i).cloned()));
    }
    head.push(last);
    let out_rows = row_keys
        .iter()
        .take(keep)
        .enumerate()
        .map(|(r, rk)| {
            let mut line = rk.clone();
            for c in 0..col_keys.len() {
                match cells.get(&(r, c)) {
                    Some(vals) => line.extend(vals.iter().cloned()),
                    None => line.extend(std::iter::repeat_n(Value::Null, v_i.len())),
                }
            }
            Value::from(line)
        })
        .collect();
    Grid {
        head,
        cols: out_cols,
        rows: out_rows,
        merges,
        truncated,
    }
}

/// A column of the export: its header and whether it holds text or dates.
#[derive(Clone)]
pub(crate) struct Col {
    pub(crate) name: String,
    text: bool,
    date: bool,
}

impl Col {
    pub(crate) fn of(c: &Value) -> Col {
        let ty = c
            .get("data_type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        Col {
            name: c.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
            text: ty == "string" || ty == "text",
            date: ty.contains("date"),
        }
    }
}

pub(crate) fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// A cell as CSV text (before quoting).
pub(crate) fn csv_text(v: &Value, col: &Col) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => if *b { "True" } else { "False" }.into(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            if col.text && s.starts_with(['=', '+', '-', '@']) {
                format!("'{s}")
            } else {
                s.clone()
            }
        }
        other => other.to_string(),
    }
}

pub(crate) fn csv_field(out: &mut String, s: &str) {
    if s.contains([',', '"', '\n', '\r']) {
        out.push('"');
        out.push_str(&s.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(s);
    }
}

/// The CSV file: a header row, then the rows.
fn csv(cols: &[Col], rows: &[Value]) -> String {
    let mut out = String::from("\u{feff}");
    for (i, c) in cols.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        csv_field(&mut out, &c.name);
    }
    out.push_str("\r\n");
    for r in rows {
        let cells = r.as_array().map_or(&[][..], Vec::as_slice);
        for (i, c) in cols.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            csv_field(&mut out, &csv_text(cells.get(i).unwrap_or(&Value::Null), c));
        }
        out.push_str("\r\n");
    }
    out
}

fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            // characters XML 1.0 cannot carry
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            '\u{fffe}' | '\u{ffff}' => {}
            c => o.push(c),
        }
    }
    o
}

/// The spreadsheet column letters of 0-based column `i` (A, B, …, Z, AA, …).
fn col_letters(mut i: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (i % 26) as u8);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

/// An ISO date or date-time (`2025-01-31`, `2025-01-31T13:45:00`) as a spreadsheet serial
/// number (days since 1899-12-30) and whether it has a time.
fn serial(s: &str) -> Option<(f64, bool)> {
    let (d, t) = s.split_once('T').unwrap_or((s, ""));
    let mut it = d.split('-');
    let (y, m, dd): (i64, i64, i64) = (
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    );
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&dd) || !(1..=9999).contains(&y) {
        return None;
    }
    // days from civil (Howard Hinnant's algorithm), then from 1899-12-30
    let (yy, mm) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * mm + 2) / 5 + dd - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468 + 25_569;
    let mut frac = 0.0;
    let t = t.trim_end_matches('Z');
    if !t.is_empty() {
        let mut p = t.split(':');
        let h: f64 = p.next()?.parse().ok()?;
        let mi: f64 = p.next().unwrap_or("0").parse().ok()?;
        let se: f64 = p.next().unwrap_or("0").parse().ok()?;
        frac = (h * 3600.0 + mi * 60.0 + se) / 86_400.0;
    }
    Some((days as f64 + frac, frac != 0.0))
}

/// The XLSX workbook: one sheet `Data`, the header rows (bold; `merges` joins header cells:
/// row, first and last column, 0-based), the rows; a last row notes a truncation.
fn xlsx(
    head: &[Vec<String>],
    cols: &[Col],
    rows: &[Value],
    merges: &[(usize, usize, usize)],
    truncated: bool,
) -> Result<Vec<u8>, String> {
    let mut sheet = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData>",
    );
    let inline = |out: &mut String, r: &str, s: &str, style: u8| {
        let _ = write!(
            out,
            "<c r=\"{r}\" t=\"inlineStr\"{}><is><t xml:space=\"preserve\">{}</t></is></c>",
            if style > 0 {
                format!(" s=\"{style}\"")
            } else {
                String::new()
            },
            xml_escape(s)
        );
    };
    let mut n = 0usize;
    for line in head {
        n += 1;
        let _ = write!(sheet, "<row r=\"{n}\">");
        for (i, t) in line.iter().enumerate() {
            if !t.is_empty() {
                inline(&mut sheet, &format!("{}{n}", col_letters(i)), t, 3);
            }
        }
        sheet.push_str("</row>");
    }
    for r in rows {
        n += 1;
        let _ = write!(sheet, "<row r=\"{n}\">");
        let cells = r.as_array().map_or(&[][..], Vec::as_slice);
        for (i, c) in cols.iter().enumerate() {
            let at = format!("{}{n}", col_letters(i));
            match cells.get(i).unwrap_or(&Value::Null) {
                Value::Null => {}
                Value::Number(x) => {
                    let _ = write!(sheet, "<c r=\"{at}\"><v>{x}</v></c>");
                }
                Value::Bool(b) => {
                    let _ = write!(sheet, "<c r=\"{at}\" t=\"b\"><v>{}</v></c>", u8::from(*b));
                }
                Value::String(s) => match (c.date, serial(s)) {
                    (true, Some((v, time))) => {
                        let _ = write!(
                            sheet,
                            "<c r=\"{at}\" s=\"{}\"><v>{v}</v></c>",
                            if time { 2 } else { 1 }
                        );
                    }
                    _ => inline(&mut sheet, &at, s, 0),
                },
                other => inline(&mut sheet, &at, &other.to_string(), 0),
            }
        }
        sheet.push_str("</row>");
    }
    if truncated {
        n += 1;
        let _ = write!(sheet, "<row r=\"{n}\">");
        inline(
            &mut sheet,
            &format!("A{n}"),
            &format!("Only the first {} rows were exported (row limit).", rows.len()),
            0,
        );
        sheet.push_str("</row>");
    }
    sheet.push_str("</sheetData>");
    if !merges.is_empty() {
        let _ = write!(sheet, "<mergeCells count=\"{}\">", merges.len());
        for (r, a, b) in merges {
            let _ = write!(
                sheet,
                "<mergeCell ref=\"{}{}:{}{}\"/>",
                col_letters(*a),
                r + 1,
                col_letters(*b),
                r + 1
            );
        }
        sheet.push_str("</mergeCells>");
    }
    sheet.push_str("</worksheet>");
    let mut pkg = bicraft_opc::Package::default();
    pkg.put(
        "[Content_Types].xml",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>"#
            .to_vec(),
        false,
    );
    pkg.put(
        "_rels/.rels",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
            .to_vec(),
        false,
    );
    pkg.put(
        "xl/workbook.xml",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#
            .to_vec(),
        false,
    );
    pkg.put(
        "xl/_rels/workbook.xml.rels",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#
            .to_vec(),
        false,
    );
    // styles: 0 general, 1 date, 2 date and time, 3 bold (the header)
    pkg.put(
        "xl/styles.xml",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="4"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="14" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="22" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/></cellXfs></styleSheet>"#
            .to_vec(),
        false,
    );
    pkg.put("xl/worksheets/sheet1.xml", sheet.into_bytes(), false);
    pkg.to_bytes()
        .map_err(|e| format!("cannot write the workbook: {e}"))
}

#[cfg(test)]
#[path = "export_data_tests.rs"]
mod tests;
