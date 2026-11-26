//! The fast reader behind [`super::read_csv`]: chunks parsed in parallel by
//! [`crate::csvscan`] (RFC 4180 dialect) and typed straight into the columns, without a
//! `Field` per cell. Columns without an explicit type keep their texts until the whole column
//! is known, then are inferred and converted in place.
//!
//! The result is [`super::read_text`]'s: same names, types, values and value errors. A
//! malformed record (syntax, field count, row length, too many rows) makes [`read`] return
//! `None` and the caller reports it with the sequential tokenizer.

use super::{CsvOptions, Inference};
use crate::csvscan::{self, Budget, Dialect, Quoting, Rec, Scan, Stop};
use crate::value::{self, Culture};
use crate::{CellValue, ColumnData, DataType, Error, TableData};

/// How a column is filled.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Typed(DataType),
    /// Texts now, the inferred type at the end.
    Infer,
}

/// One chunk's columns.
struct Part {
    cols: Vec<Vec<CellValue>>,
    rows: usize,
    /// First value error per column: (row in the chunk, 1-based; message).
    errs: Vec<Option<(usize, String)>>,
    /// A record the sequential reader must report.
    bad: bool,
    text_bytes: usize,
}

fn stringish(t: DataType) -> bool {
    matches!(t, DataType::String | DataType::Unknown | DataType::Variant)
}

/// `read_text`'s `cell_value`, with the fast forms.
fn cell(s: &str, quoted: bool, t: DataType, culture: Culture) -> Result<CellValue, String> {
    if s.is_empty() && !(quoted && stringish(t)) {
        return Ok(CellValue::Null);
    }
    if stringish(t) {
        return Ok(CellValue::Text(s.to_string()));
    }
    value::parse_text_as_fast(s, t, culture)
}

/// Reads `text` (decoded, delimiter chosen); `None` when a record is malformed.
pub(super) fn read(text: &str, delim: char, opts: &CsvOptions) -> Result<Option<TableData>, Error> {
    let limits = &opts.limits;
    let culture = opts.culture;
    let mut dbuf = [0u8; 4];
    let dbytes = delim.encode_utf8(&mut dbuf).as_bytes();
    let d = Dialect {
        quoting: Quoting::Rfc,
        delim: dbytes,
        max_cell: limits.max_cell_bytes,
        max_fields: limits.max_columns,
    };
    let mut rec = Rec::new();
    let after_first = match csvscan::scan(text, 0, true, &d, usize::MAX, &mut rec) {
        Scan::Record(next) => next,
        Scan::End => return TableData::new(Vec::new()).map(Some),
        _ => return Ok(None),
    };
    let width = rec.count();
    let names: Vec<String> = if opts.has_header {
        (0..width)
            .map(|i| {
                let name = rec.field(text, i).map_or("", |f| f.0).trim();
                if name.is_empty() {
                    format!("Column{}", i + 1)
                } else {
                    name.to_string()
                }
            })
            .collect()
    } else {
        (1..=width).map(|i| format!("Column{i}")).collect()
    };
    let data_start = if opts.has_header { after_first } else { 0 };
    let kinds: Vec<Kind> = names
        .iter()
        .map(|name| {
            let explicit = opts
                .column_types
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, t)| *t);
            match explicit {
                Some(t) => Kind::Typed(t),
                None if opts.infer_types => Kind::Infer,
                None => Kind::Typed(DataType::String),
            }
        })
        .collect();

    // The table's memory, estimated from the first records, before anything is kept.
    let mut budget = Budget::new(limits);
    let text_cols: Vec<bool> = kinds
        .iter()
        .map(|k| matches!(k, Kind::Infer) || matches!(k, Kind::Typed(t) if stringish(*t)))
        .collect();
    let (n, bytes, tbytes) = csvscan::sample(text, data_start, true, &d, width, &text_cols, 1000);
    if n > 0 {
        let rest = text.len().saturating_sub(data_start);
        let rows = (rest as f64 * n as f64 / bytes.max(1) as f64) as usize + 1;
        budget.need(
            csvscan::table_bytes(rows, width, tbytes as f64 / n as f64),
            "the CSV table",
        )?;
    }

    let threads = csvscan::threads();
    let work = |from: usize, nominal_end: usize| {
        csvscan::parse_chunk(
            text,
            from,
            nominal_end,
            true,
            &d,
            width + 1,
            || Part {
                cols: (0..width).map(|_| Vec::new()).collect(),
                rows: 0,
                errs: vec![None; width],
                bad: false,
                text_bytes: 0,
            },
            |p, rec| {
                let count = rec.count();
                if count == 1 && width > 1 && rec.field(text, 0).is_some_and(|(s, q)| s.is_empty() && !q) {
                    return true;
                }
                if count != width {
                    p.bad = true;
                    return false;
                }
                p.rows += 1;
                for (c, (col, kind)) in p.cols.iter_mut().zip(&kinds).enumerate() {
                    let (s, quoted) = rec.field(text, c).unwrap_or(("", false));
                    let v = match kind {
                        Kind::Typed(t) => match cell(s, quoted, *t, culture) {
                            Ok(v) => v,
                            Err(m) => {
                                if let Some(e @ None) = p.errs.get_mut(c) {
                                    *e = Some((p.rows, m));
                                }
                                CellValue::Null
                            }
                        },
                        Kind::Infer if s.is_empty() && !quoted => CellValue::Null,
                        Kind::Infer => CellValue::Text(s.to_string()),
                    };
                    if let CellValue::Text(t) = &v {
                        p.text_bytes += t.len() + csvscan::TEXT_OVERHEAD;
                    }
                    col.push(v);
                }
                true
            },
        )
    };
    let mut parts = csvscan::run_region(text, data_start, threads, &work);
    if parts.last().is_none_or(|p| p.stop != Stop::End) || parts.iter().any(|p| p.out.bad) {
        return Ok(None);
    }
    let total: usize = parts.iter().map(|p| p.out.rows).sum();
    if total > limits.max_rows {
        return Ok(None);
    }
    let text_bytes: usize = parts.iter().map(|p| p.out.text_bytes).sum();
    budget.need(
        csvscan::table_bytes(total, width, 0.0).saturating_add(text_bytes),
        "the CSV table",
    )?;

    // First value error per column, in rows of the table.
    let mut errs: Vec<Option<(usize, String)>> = vec![None; width];
    let mut offset = 0usize;
    for p in &mut parts {
        for (e, pe) in errs.iter_mut().zip(p.out.errs.iter_mut()) {
            if e.is_none()
                && let Some((r, m)) = pe.take()
            {
                *e = Some((offset + r, m));
            }
        }
        offset += p.out.rows;
    }

    let mut columns = Vec::with_capacity(width);
    for (c, (name, kind)) in names.into_iter().zip(&kinds).enumerate() {
        let mut values = Vec::with_capacity(total);
        for p in &mut parts {
            if let Some(col) = p.out.cols.get_mut(c) {
                values.append(col);
                *col = Vec::new();
            }
        }
        let data_type = match kind {
            Kind::Typed(t) => *t,
            Kind::Infer => {
                let (t, err) = infer_and_convert(&mut values, culture, threads);
                if let (Some(e), Some(slot)) = (err, errs.get_mut(c)) {
                    *slot = Some(e);
                }
                t
            }
        };
        columns.push(ColumnData {
            name,
            data_type,
            values,
        });
    }
    if let Some((c, (row, message))) = errs.into_iter().enumerate().find_map(|(c, e)| e.map(|e| (c, e))) {
        let column = columns.get(c).map(|col| col.name.clone()).unwrap_or_default();
        return Err(Error::Value { row, column, message });
    }
    TableData::new(columns).map(Some)
}

/// Infers the type of a column of texts and blanks (`Text("")` is a quoted empty field), then
/// converts it in place; returns the type and the first value error (row, message).
fn infer_and_convert(
    values: &mut [CellValue],
    culture: Culture,
    threads: usize,
) -> (DataType, Option<(usize, String)>) {
    let pieces = if values.len() >= 200_000 {
        threads.max(1)
    } else {
        1
    };
    let size = values.len().div_ceil(pieces).max(1);
    let infer = |slice: &[CellValue]| {
        let mut inf = Inference::new();
        for v in slice {
            if let CellValue::Text(s) = v
                && !inf.add(s, culture)
            {
                break;
            }
        }
        inf
    };
    let mut inf = Inference::new();
    if pieces == 1 {
        inf = infer(values);
    } else {
        let parts: Vec<Option<Inference>> = std::thread::scope(|s| {
            let handles: Vec<_> = values.chunks(size).map(|c| s.spawn(move || infer(c))).collect();
            handles.into_iter().map(|h| h.join().ok()).collect()
        });
        for (part, chunk) in parts.into_iter().zip(values.chunks(size)) {
            inf.merge(&part.unwrap_or_else(|| infer(chunk)));
        }
    }
    let t = inf.result();
    if stringish(t) {
        return (t, None);
    }
    let convert = |slice: &mut [CellValue]| -> Option<(usize, String)> {
        let mut first = None;
        for (i, v) in slice.iter_mut().enumerate() {
            let CellValue::Text(s) = v else { continue };
            let nv = if s.is_empty() {
                Ok(CellValue::Null)
            } else {
                value::parse_text_as_fast(s, t, culture)
            };
            *v = match nv {
                Ok(nv) => nv,
                Err(m) => {
                    if first.is_none() {
                        first = Some((i + 1, m));
                    }
                    CellValue::Null
                }
            };
        }
        first
    };
    let err = if pieces == 1 {
        convert(values)
    } else {
        let firsts: Vec<Option<Option<(usize, String)>>> = std::thread::scope(|s| {
            let handles: Vec<_> = values
                .chunks_mut(size)
                .map(|c| s.spawn(move || convert(c)))
                .collect();
            handles.into_iter().map(|h| h.join().ok()).collect()
        });
        // A worker that panicked is redone here (converted cells are skipped).
        let mut firsts: Vec<Option<(usize, String)>> = firsts
            .into_iter()
            .zip(values.chunks_mut(size))
            .map(|(e, chunk)| e.unwrap_or_else(|| convert(chunk)))
            .collect();
        firsts
            .iter_mut()
            .enumerate()
            .find_map(|(k, e)| e.take().map(|(r, m)| (k * size + r, m)))
    };
    (t, err)
}
