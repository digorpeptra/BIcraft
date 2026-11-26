//! Local file formats: `Csv.Document`, `Json.Document`, `Excel.Workbook` (file access itself
//! is in [`super::files`]).

use std::rc::Rc;

use super::misc::{binary, decode};
use super::{Builtin, arg, b, conv_err, names, opt_number};
use crate::m::eval::Interp;
use crate::m::lib::table::{make, promote_all};
use crate::m::value::{Column, MError, MType, Res, Value, err, expr_err, fatal, unsupported};
use crate::m::xml::{self, Event};

pub(crate) static FUNCS: &[Builtin] = &[
    b("Csv.Document", 1, 5, csv_document),
    b("Json.Document", 1, 2, json_document),
    b("Excel.Workbook", 1, 3, excel_workbook),
];

/// Text of a `binary` or `text` source.
fn source_text(interp: &Interp, v: &Value, encoding: Option<f64>) -> Res<String> {
    let s = match v {
        Value::Text(t) => t.to_string(),
        Value::Binary(_) => decode(&binary(v)?, encoding)?,
        other => return conv_err(other, "Binary"),
    };
    if s.len() > interp.limits.max_bytes {
        return fatal(crate::Error::LimitExceeded(format!(
            "source text is larger than {} bytes",
            interp.limits.max_bytes
        )));
    }
    Ok(s)
}

// ---------------------------------------------------------------------------
// CSV.

/// Splits CSV text into records. With `quotes` (QuoteStyle.Csv) a quoted field
/// may contain delimiters and line breaks; otherwise line breaks always end a
/// record but quotes still group delimiters within a line.
fn csv_records(interp: &mut Interp, t: &str, delim: &str, quotes_span_lines: bool) -> Res<Vec<Vec<String>>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut rest = t;
    let mut field_start = true;
    let mut row_has_content = false;
    while !rest.is_empty() {
        if field_start && rest.starts_with('"') {
            let mut body = rest.get(1..).unwrap_or("");
            loop {
                let stop = if quotes_span_lines {
                    body.find('"')
                } else {
                    body.find(['"', '\r', '\n'])
                };
                match stop {
                    None => {
                        field.push_str(body);
                        body = "";
                        break;
                    }
                    Some(i) => {
                        field.push_str(body.get(..i).unwrap_or(""));
                        let at = body.get(i..).unwrap_or("");
                        if !at.starts_with('"') {
                            body = at;
                            break;
                        }
                        let after = body.get(i + 1..).unwrap_or("");
                        if let Some(r) = after.strip_prefix('"') {
                            field.push('"');
                            body = r;
                        } else {
                            body = after;
                            break;
                        }
                    }
                }
                if field.len() > interp.limits.max_cell_bytes {
                    interp.check_text(field.len())?;
                }
            }
            rest = body;
            field_start = false;
            row_has_content = true;
            continue;
        }
        if !delim.is_empty() && rest.starts_with(delim) {
            row.push(std::mem::take(&mut field));
            rest = rest.get(delim.len()..).unwrap_or("");
            field_start = true;
            row_has_content = true;
            continue;
        }
        if rest.starts_with('\r') || rest.starts_with('\n') {
            let skip = if rest.starts_with("\r\n") { 2 } else { 1 };
            rest = rest.get(skip..).unwrap_or("");
            row.push(std::mem::take(&mut field));
            rows.push(std::mem::take(&mut row));
            interp.tick(1)?;
            interp.check_rows(rows.len())?;
            field_start = true;
            row_has_content = false;
            continue;
        }
        let mut chars = rest.chars();
        if let Some(c) = chars.next() {
            field.push(c);
        }
        rest = chars.as_str();
        field_start = false;
        row_has_content = true;
        if field.len() > interp.limits.max_cell_bytes {
            interp.check_text(field.len())?;
        }
    }
    if row_has_content || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

fn csv_document(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let mut columns = arg(a, 1).clone();
    let mut delimiter = arg(a, 2).clone();
    let mut extra = opt_number(arg(a, 3))?;
    let mut encoding = opt_number(arg(a, 4))?;
    let mut quote_style = 1.0;
    if let Value::Record(r) = arg(a, 1) {
        columns = r.get("Columns").cloned().unwrap_or(Value::Null);
        delimiter = r.get("Delimiter").cloned().unwrap_or(Value::Null);
        if let Some(e) = r.get("ExtraValues") {
            extra = opt_number(e)?;
        }
        if let Some(e) = r.get("Encoding") {
            encoding = opt_number(e)?;
        }
        if let Some(q) = r.get("QuoteStyle") {
            quote_style = opt_number(q)?.unwrap_or(1.0);
        }
        if let Some(Value::Number(n)) = r.get("CsvStyle")
            && n.f() != 0.0
        {
            return unsupported("`Csv.Document` with `CsvStyle.QuoteAlways`");
        }
    }
    let delim = match &delimiter {
        Value::Null => ",".to_string(),
        Value::Text(t) => t.to_string(),
        other => return conv_err(other, "Text"),
    };
    let src = source_text(interp, arg(a, 0), encoding)?;
    let records = csv_records(interp, &src, &delim, quote_style == 1.0)?;
    let cols: Vec<Column> = match &columns {
        Value::Null => {
            // Without `Columns`, the first record gives the width (as Desktop does).
            let n = records.first().map_or(0, Vec::len);
            crate::m::lib::table::column_count_arg(interp, &Value::num(n as f64))?;
            default_cols(n)
        }
        Value::Number(_) => default_cols(crate::m::lib::table::column_count_arg(interp, &columns)?),
        Value::List(_) => names(&columns)?
            .into_iter()
            .map(|n| Column {
                name: n,
                ty: MType::any(),
            })
            .collect(),
        Value::Type(t) => match &t.fields {
            Some(f) => f
                .iter()
                .map(|(n, ty)| Column {
                    name: n.clone(),
                    ty: ty.clone(),
                })
                .collect(),
            None => return expr_err("A table type was expected for the columns."),
        },
        other => return conv_err(other, "Number"),
    };
    let w = cols.len();
    let mut rows = Vec::with_capacity(records.len());
    for (i, r) in records.into_iter().enumerate() {
        interp.tick(1)?;
        let mut row: Vec<Value> = Vec::with_capacity(w);
        let n = r.len();
        for f in r.into_iter().take(w) {
            row.push(Value::text(f));
        }
        if n > w {
            match extra.map(|e| e as i64) {
                Some(1) => {
                    let e = Value::Error(Rc::new(MError {
                        reason: "Expression.Error".into(),
                        message: format!(
                            "There were more columns in the result than expected (row {}).",
                            i + 1
                        ),
                        detail: Value::Null,
                    }));
                    if let Some(last) = row.last_mut() {
                        *last = e;
                    }
                }
                Some(0) => {
                    return unsupported("`Csv.Document` with `ExtraValues.List`");
                }
                _ => {}
            }
        }
        // Missing fields of a short record are empty texts.
        row.resize(w, Value::text(""));
        rows.push(row);
    }
    make(interp, cols, rows)
}

fn default_cols(n: usize) -> Vec<Column> {
    (1..=n)
        .map(|i| Column {
            name: format!("Column{i}").into(),
            ty: MType::any(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// JSON.

fn json_document(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let enc = opt_number(arg(a, 1))?;
    let src = match arg(a, 0) {
        Value::Null => return Ok(Value::Null),
        v => source_text(interp, v, enc)?,
    };
    interp.tick((src.len() / 16) as u64)?;
    let max_items = interp.limits.max_rows.saturating_mul(16);
    match crate::m::json::parse(
        src.trim_start_matches('\u{feff}'),
        interp.limits.max_eval_depth.min(512),
        max_items,
    ) {
        Ok(v) => Ok(v),
        Err(e) => err(
            "DataFormat.Error",
            format!("We found an error while parsing the JSON: {}", e.0),
        ),
    }
}

// ---------------------------------------------------------------------------
// Excel.

fn part<'p>(pkg: &'p bicraft_opc::Package, name: &str) -> Option<&'p str> {
    let lower = name.trim_start_matches('/').to_ascii_lowercase();
    pkg.parts
        .iter()
        .find(|p| p.name.to_ascii_lowercase() == lower)
        .and_then(|p| std::str::from_utf8(&p.data).ok())
}

/// Relationship id → target path, resolved against `base_dir`.
fn rels(pkg: &bicraft_opc::Package, rels_path: &str, base_dir: &str) -> Vec<(String, String)> {
    let Some(x) = part(pkg, rels_path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in xml::Reader::new(x) {
        if let Event::Start(n, a) = e
            && n == "Relationship"
            && let (Some(id), Some(t)) = (xml::attr(&a, "Id"), xml::attr(&a, "Target"))
        {
            out.push((id.to_string(), resolve(base_dir, t)));
        }
    }
    out
}

fn resolve(base_dir: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut parts: Vec<&str> = base_dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in target.split('/') {
        match seg {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn col_index(r: &str) -> Option<(usize, usize)> {
    let letters: String = r.chars().take_while(char::is_ascii_alphabetic).collect();
    let digits = r.get(letters.len()..)?;
    let mut c = 0usize;
    for ch in letters.chars() {
        c = c
            .checked_mul(26)?
            .checked_add(usize::from(ch.to_ascii_uppercase() as u8 - b'A') + 1)?;
    }
    let row: usize = digits.parse().ok()?;
    Some((row.checked_sub(1)?, c.checked_sub(1)?))
}

fn is_date_format(id: u32, custom: &[(u32, String)]) -> bool {
    if matches!(id, 14..=22 | 27..=36 | 45..=47 | 50..=58) {
        return true;
    }
    let Some((_, code)) = custom.iter().find(|(i, _)| *i == id) else {
        return false;
    };
    let mut in_quote = false;
    let mut in_bracket = false;
    for c in code.chars() {
        match c {
            '"' => in_quote = !in_quote,
            '[' if !in_quote => in_bracket = true,
            ']' if !in_quote => in_bracket = false,
            'y' | 'Y' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' | 'm' | 'M' if !in_quote && !in_bracket => {
                return true;
            }
            _ => {}
        }
    }
    false
}

struct Sheet {
    name: String,
    hidden: bool,
    path: String,
}

fn excel_workbook(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let bytes = binary(arg(a, 0))?;
    let use_headers = matches!(arg(a, 1), Value::Logical(true));
    let limits = bicraft_opc::ReadLimits {
        max_part_bytes: interp.limits.max_bytes as u64,
        max_total_bytes: (interp.limits.max_bytes as u64).saturating_mul(4),
    };
    let pkg = match bicraft_opc::Package::read_bytes_with(&bytes, &limits) {
        Ok(p) => p,
        Err(bicraft_opc::Error::PartTooLarge { .. } | bicraft_opc::Error::PackageTooLarge { .. }) => {
            return fatal(crate::Error::LimitExceeded(
                "the workbook decompresses to too many bytes".into(),
            ));
        }
        Err(e) => {
            return err(
                "DataFormat.Error",
                format!("The file is not a valid Excel workbook: {e}."),
            );
        }
    };
    let Some(wb) = part(&pkg, "xl/workbook.xml") else {
        if part(&pkg, "xl/workbook.bin").is_some() {
            return unsupported("binary Excel workbooks (.xlsb)");
        }
        return err("DataFormat.Error", "The file is not a valid Excel workbook.");
    };
    let wb_rels = rels(&pkg, "xl/_rels/workbook.xml.rels", "xl");
    let mut sheets = Vec::new();
    let mut date1904 = false;
    for e in xml::Reader::new(wb) {
        if let Event::Start(n, a) = e {
            if n == "workbookPr" {
                date1904 = matches!(xml::attr(&a, "date1904"), Some("1" | "true"));
            }
            if n == "sheet"
                && let (Some(name), Some(id)) = (xml::attr(&a, "name"), xml::attr(&a, "id"))
                && let Some((_, path)) = wb_rels.iter().find(|(i, _)| i == id)
            {
                sheets.push(Sheet {
                    name: name.to_string(),
                    hidden: matches!(xml::attr(&a, "state"), Some("hidden" | "veryHidden")),
                    path: path.clone(),
                });
            }
        }
    }
    // Shared strings.
    let mut shared: Vec<Rc<str>> = Vec::new();
    if let Some(sst) = wb_rels
        .iter()
        .find(|(_, p)| p.to_ascii_lowercase().ends_with("sharedstrings.xml"))
        .and_then(|(_, p)| part(&pkg, p))
        .or_else(|| part(&pkg, "xl/sharedStrings.xml"))
    {
        let mut cur = String::new();
        let (mut in_si, mut in_t, mut in_rph) = (false, false, false);
        for e in xml::Reader::new(sst) {
            match e {
                Event::Start(n, _) if n == "si" => {
                    in_si = true;
                    cur.clear();
                }
                Event::Start(n, _) if n == "rPh" => in_rph = true,
                Event::End(n) if n == "rPh" => in_rph = false,
                Event::Start(n, _) if n == "t" => in_t = true,
                Event::End(n) if n == "t" => in_t = false,
                Event::Text(t) if in_si && in_t && !in_rph => cur.push_str(&t),
                Event::End(n) if n == "si" => {
                    in_si = false;
                    shared.push(std::mem::take(&mut cur).into());
                    interp.tick(1)?;
                }
                _ => {}
            }
        }
    }
    // Styles: which cell formats are dates.
    let mut date_styles: Vec<bool> = Vec::new();
    if let Some(st) = part(&pkg, "xl/styles.xml") {
        let mut custom: Vec<(u32, String)> = Vec::new();
        let mut in_xfs = false;
        let mut ids: Vec<u32> = Vec::new();
        for e in xml::Reader::new(st) {
            match e {
                Event::Start(n, a) if n == "numFmt" => {
                    if let (Some(id), Some(code)) = (xml::attr(&a, "numFmtId"), xml::attr(&a, "formatCode")) {
                        custom.push((id.parse().unwrap_or(0), code.to_string()));
                    }
                }
                Event::Start(n, _) if n == "cellXfs" => in_xfs = true,
                Event::End(n) if n == "cellXfs" => in_xfs = false,
                Event::Start(n, a) if n == "xf" && in_xfs => {
                    ids.push(
                        xml::attr(&a, "numFmtId")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0),
                    );
                }
                _ => {}
            }
        }
        date_styles = ids.iter().map(|&id| is_date_format(id, &custom)).collect();
    }
    let epoch_shift = if date1904 { 1462 } else { 0 };
    let mut nav_rows = Vec::new();
    for s in &sheets {
        let Some(x) = part(&pkg, &s.path) else { continue };
        let grid = read_sheet(interp, x, &shared, &date_styles, epoch_shift)?;
        let data = grid_table(interp, grid, use_headers)?;
        nav_rows.push(vec![
            Value::text(s.name.as_str()),
            data,
            Value::text(s.name.as_str()),
            Value::text("Sheet"),
            Value::Logical(s.hidden),
        ]);
        // Excel tables defined on this sheet.
        let dir = s.path.rsplit_once('/').map_or("", |(d, _)| d);
        let file = s.path.rsplit_once('/').map_or(s.path.as_str(), |(_, f)| f);
        for (_, tpath) in rels(&pkg, &format!("{dir}/_rels/{file}.rels"), dir) {
            if !tpath.to_ascii_lowercase().contains("tables/") {
                continue;
            }
            let Some(tx) = part(&pkg, &tpath) else { continue };
            let mut name = String::new();
            let mut range = String::new();
            for e in xml::Reader::new(tx) {
                if let Event::Start(n, a) = e
                    && n == "table"
                {
                    name = xml::attr(&a, "displayName")
                        .or(xml::attr(&a, "name"))
                        .unwrap_or("")
                        .to_string();
                    range = xml::attr(&a, "ref").unwrap_or("").to_string();
                    break;
                }
            }
            let Some((from, to)) = range
                .split_once(':')
                .and_then(|(f, t)| Some((col_index(f)?, col_index(t)?)))
            else {
                continue;
            };
            let grid = read_sheet(interp, x, &shared, &date_styles, epoch_shift)?;
            let sub: Vec<Vec<Value>> = (from.0..=to.0)
                .map(|r| (from.1..=to.1).map(|c| cell_at(&grid, r, c)).collect())
                .collect();
            let data = grid_table(
                interp,
                Grid {
                    origin: (0, 0),
                    rows: sub,
                },
                true,
            )?;
            nav_rows.push(vec![
                Value::text(name.as_str()),
                data,
                Value::text(name.as_str()),
                Value::text("Table"),
                Value::Logical(false),
            ]);
        }
    }
    let cols = ["Name", "Data", "Item", "Kind", "Hidden"]
        .iter()
        .map(|n| Column {
            name: (*n).into(),
            ty: MType::any(),
        })
        .collect();
    make(interp, cols, nav_rows)
}

/// Cells of a sheet, from its first used cell.
struct Grid {
    /// (row, column) of the top-left cell in sheet coordinates.
    origin: (usize, usize),
    rows: Vec<Vec<Value>>,
}

fn cell_at(g: &Grid, r: usize, c: usize) -> Value {
    r.checked_sub(g.origin.0)
        .and_then(|r| g.rows.get(r))
        .and_then(|row| c.checked_sub(g.origin.1).and_then(|c| row.get(c)))
        .cloned()
        .unwrap_or(Value::Null)
}

fn read_sheet(interp: &mut Interp, x: &str, shared: &[Rc<str>], dates: &[bool], shift: i64) -> Res<Grid> {
    let mut cells: Vec<(usize, usize, Value)> = Vec::new();
    let (mut row, mut col) = (0usize, 0usize);
    let mut ctype = String::new();
    let mut style = 0usize;
    let mut val: Option<String> = None;
    let mut inline = String::new();
    let (mut in_v, mut in_is_t, mut in_c) = (false, false, false);
    let mut next_row = 0usize;
    for e in xml::Reader::new(x) {
        match e {
            Event::Start(n, a) if n == "row" => {
                row = xml::attr(&a, "r")
                    .and_then(|r| r.parse::<usize>().ok())
                    .map_or(next_row, |r| r.saturating_sub(1));
                next_row = row + 1;
                col = 0;
            }
            Event::Start(n, a) if n == "c" => {
                in_c = true;
                if let Some((r, c)) = xml::attr(&a, "r").and_then(col_index) {
                    row = r;
                    col = c;
                }
                ctype = xml::attr(&a, "t").unwrap_or("n").to_string();
                style = xml::attr(&a, "s").and_then(|s| s.parse().ok()).unwrap_or(0);
                val = None;
                inline.clear();
            }
            Event::Start(n, _) if n == "v" && in_c => in_v = true,
            Event::End(n) if n == "v" => in_v = false,
            Event::Start(n, _) if n == "t" && in_c => in_is_t = true,
            Event::End(n) if n == "t" => in_is_t = false,
            Event::Text(t) if in_v => val.get_or_insert_with(String::new).push_str(&t),
            Event::Text(t) if in_is_t => inline.push_str(&t),
            Event::End(n) if n == "c" => {
                in_c = false;
                let v = match (ctype.as_str(), &val) {
                    ("inlineStr", _) => Value::text(inline.as_str()),
                    (_, None) => Value::Null,
                    ("s", Some(v)) => match v.trim().parse::<usize>().ok().and_then(|i| shared.get(i)) {
                        Some(s) => Value::Text(s.clone()),
                        None => Value::Null,
                    },
                    ("str", Some(v)) => Value::text(v.as_str()),
                    ("b", Some(v)) => Value::Logical(v.trim() == "1"),
                    ("e", Some(v)) => Value::Error(Rc::new(MError {
                        reason: "DataFormat.Error".into(),
                        message: v.trim().to_string(),
                        detail: Value::Null,
                    })),
                    ("d", Some(v)) => match crate::m::convert::datetime_from_text(v, &interp.culture) {
                        Some((ms, _)) => Value::DateTime(ms),
                        None => Value::text(v.as_str()),
                    },
                    (_, Some(v)) => match v.trim().parse::<f64>() {
                        Ok(n) if dates.get(style).copied().unwrap_or(false) => {
                            // Serials 1..60 of the 1900 system sit before Excel's phantom
                            // 1900-02-29: one day later in OLE terms (60 itself → 1900-03-01).
                            let phantom = if shift == 0 && (1.0..61.0).contains(&n) {
                                1.0
                            } else {
                                0.0
                            };
                            let days = n + shift as f64 + phantom;
                            match crate::m::convert::ole_ms(days) {
                                Some(ms) if days >= 1.0 || ms >= 0 => Value::DateTime(ms),
                                _ => Value::num(n),
                            }
                        }
                        // Desktop keeps the cell's text as a decimal when it fits one.
                        Ok(n) => match crate::m::value::Num::parse(v.trim()) {
                            Some(x) => Value::Number(x),
                            None => Value::num(n),
                        },
                        Err(_) => Value::text(v.as_str()),
                    },
                };
                if !v.is_null() {
                    cells.push((row, col, v));
                    interp.tick(1)?;
                    interp.check_rows(cells.len() / 16)?;
                }
                col += 1;
            }
            _ => {}
        }
    }
    let min_r = cells.iter().map(|c| c.0).min().unwrap_or(0);
    let min_c = cells.iter().map(|c| c.1).min().unwrap_or(0);
    let max_r = cells.iter().map(|c| c.0).max().unwrap_or(0);
    let max_c = cells.iter().map(|c| c.1).max().unwrap_or(0);
    if cells.is_empty() {
        return Ok(Grid {
            origin: (0, 0),
            rows: Vec::new(),
        });
    }
    let h = max_r - min_r + 1;
    let w = max_c - min_c + 1;
    interp.check_rows(h)?;
    if w > interp.limits.max_columns {
        return fatal(crate::Error::LimitExceeded(format!(
            "a sheet has more than {} columns",
            interp.limits.max_columns
        )));
    }
    interp.tick((h as u64).saturating_mul(w as u64))?;
    let mut rows = vec![vec![Value::Null; w]; h];
    for (r, c, v) in cells {
        if let Some(slot) = rows.get_mut(r - min_r).and_then(|row| row.get_mut(c - min_c)) {
            *slot = v;
        }
    }
    Ok(Grid {
        origin: (min_r, min_c),
        rows,
    })
}

fn grid_table(interp: &mut Interp, g: Grid, headers: bool) -> Res<Value> {
    let w = g.rows.first().map_or(0, Vec::len);
    let t = make(interp, default_cols(w), g.rows)?;
    if headers { promote_all(interp, &t) } else { Ok(t) }
}
