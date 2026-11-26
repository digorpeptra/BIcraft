//! `query.export`: every row of a Power Query (M) query, or of the query at one of its applied
//! steps, written to a CSV file, as the Power Query editor's Home › Output data exports the
//! query's results (seen in Power BI Desktop 2.159.1231.0). The query is evaluated as
//! `query.preview` evaluates it (`query_preview::evaluate`), without a row limit beyond the
//! evaluator's; nothing in the model changes.
//!
//! The CSV is written as `visual.export_data` writes it: UTF-8 with a byte order mark, a header
//! row, CRLF line ends, text that starts with `=`, `+`, `-` or `@` prefixed with `'`.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::Session;
use crate::export_data::{Col, csv_field, csv_text, write_file};
use crate::params::{allow, str_param};

/// Largest file written, in bytes.
const MAX_BYTES: usize = 1 << 30;

/// `query.export {name? | expression?, partition?, step?, queries?, data_root?,
/// allow_connectors?, timeout_ms?, out}`.
pub(crate) fn query_export(s: &Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "name",
            "partition",
            "expression",
            "step",
            "queries",
            "data_root",
            "allow_connectors",
            "timeout_ms",
            "out",
        ],
    )?;
    let out = str_param(p, "out")?.trim();
    if out.is_empty() || !out.to_ascii_lowercase().ends_with(".csv") {
        return Err("`out` must be the .csv file to write".into());
    }
    let ev = crate::query_preview::evaluate(s, p, None, false)?;
    let cols = ev.data.columns();
    let heads: Vec<Col> = cols
        .iter()
        .map(|c| Col::of(&json!({"name": c.name, "data_type": c.data_type.as_tmsl().unwrap_or("")})))
        .collect();
    let rows = cols.first().map_or(0, |c| c.values.len());
    let mut text = String::from("\u{feff}");
    for (i, c) in heads.iter().enumerate() {
        if i > 0 {
            text.push(',');
        }
        csv_field(&mut text, &c.name);
    }
    text.push_str("\r\n");
    for r in 0..rows {
        for (i, (c, h)) in cols.iter().zip(&heads).enumerate() {
            if i > 0 {
                text.push(',');
            }
            let v = c.values.get(r).map_or(Value::Null, crate::preview::cell_json);
            csv_field(&mut text, &csv_text(&v, h));
        }
        text.push_str("\r\n");
        if text.len() > MAX_BYTES {
            return Err(format!("the CSV would be larger than {} MB", MAX_BYTES >> 20));
        }
    }
    let path = PathBuf::from(out);
    write_file(&path, text.as_bytes())?;
    Ok(json!({
        "name": ev.name,
        "step": ev.step,
        "source": ev.kind,
        "path": path.display().to_string(),
        "bytes": text.len(),
        "columns": cols.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
        "rows": rows,
        "elapsed_ms": u64::try_from(ev.started.elapsed().as_millis()).unwrap_or(u64::MAX),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_every_row_of_the_query_or_of_a_step() {
        let dir = std::env::temp_dir().join(format!("bicraft-query-export-{}", std::process::id()));
        let mut s = Session::new();
        s.execute("model.new", &json!({"name": "Shop"})).unwrap();
        let m = "let\n    Source = #table(type table [Product = text, Qty = Int64.Type], {{\"Bike\", 2}, {\"=Car, red\", 1}, {\"Pen\", null}}),\n    Kept = Table.SelectRows(Source, each [Qty] <> 1)\nin\n    Kept";
        s.execute("table.import", &json!({"name": "Sales", "m": m}))
            .unwrap();
        let out = dir.join("sales.csv");
        let v = s
            .execute(
                "query.export",
                &json!({"name": "Sales", "out": out.display().to_string()}),
            )
            .unwrap();
        assert_eq!(v["rows"], 2);
        let text = std::fs::read_to_string(&out).unwrap();
        assert_eq!(text, "\u{feff}Product,Qty\r\nBike,2\r\nPen,\r\n");
        // the query at its first step, text quoted and guarded
        let v = s
            .execute(
                "query.export",
                &json!({"name": "Sales", "step": "Source", "out": out.display().to_string()}),
            )
            .unwrap();
        assert_eq!(v["rows"], 3);
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(text.contains("\r\n\"'=Car, red\",1\r\n"), "{text}");
        for bad in [
            json!({"name": "Sales"}),
            json!({"name": "Sales", "out": "a.txt"}),
            json!({"name": "Sales", "out": 5}),
            json!({"name": "Nope", "out": "a.csv"}),
            json!({"expression": "1 +", "out": "a.csv"}),
            json!({"name": "Sales", "out": "a.csv", "x": 1}),
        ] {
            assert!(s.execute("query.export", &bad).is_err(), "{bad}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
