//! `storage-compare`: bicraft's `model.storage_stats` against the engine's storage DMVs
//! (docs/formats/vertipaq-stats.md, "Engine check").
//!
//! `cargo xtask storage-compare CASE.dmv.json CASE.stats.json [--verbose]` prints every differing
//! figure and fails when something differs.

use std::path::Path;

use serde_json::Value;

use super::read_json;

/// A DMV rowset as PowerShell's ConvertTo-Json writes it: a list, one object, or nothing.
fn rows(v: Option<&Value>) -> Vec<&Value> {
    match v {
        Some(Value::Array(a)) => a.iter().collect(),
        Some(Value::Null) | None => Vec::new(),
        Some(Value::Object(m)) if m.is_empty() => Vec::new(),
        Some(x) => vec![x],
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or_default()
}

fn n(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

/// Engine system tables (`H$…`, `R$…`, `U$…`) have `$` second.
fn is_system(table_id: &str) -> bool {
    table_id.as_bytes().get(1) == Some(&b'$')
}

/// A figure as Python's `str()` prints it.
fn show(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(x) if x.is_f64() => super::py_float(x.as_f64().unwrap_or(0.0)),
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        x => x.to_string(),
    }
}

/// Python equality of two JSON values: numbers compare by value (3 == 3.0).
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_i64(), y.as_i64()) {
            (Some(x), Some(y)) => x == y,
            _ => x.as_f64() == y.as_f64(),
        },
        _ => a == b,
    }
}

fn num(x: f64) -> Value {
    if x.fract() == 0.0 && x.abs() < 9e15 {
        Value::from(x as i64)
    } else {
        Value::from(x)
    }
}

struct Checker {
    checked: usize,
    diffs: Vec<String>,
    verbose: bool,
    out: Vec<String>,
}

impl Checker {
    fn check(&mut self, what: &str, ours: &Value, theirs: &Value) {
        self.checked += 1;
        if !same(ours, theirs) {
            self.diffs
                .push(format!("{what}: bicraft {}, engine {}", show(ours), show(theirs)));
        } else if self.verbose {
            self.out.push(format!("ok {what}: {}", show(ours)));
        }
    }
}

/// The comparison; returns the printed lines and whether everything matched.
pub fn compare(dmv: &Value, stats: &Value, verbose: bool) -> (Vec<String>, bool) {
    let stats = stats.get("results").and_then(|r| r.get(0)).unwrap_or(stats);
    let tables = rows(dmv.get("tables"));
    let columns = rows(dmv.get("columns"));
    let segments = rows(dmv.get("segments"));
    let mut ck = Checker {
        checked: 0,
        diffs: Vec::new(),
        verbose,
        out: Vec::new(),
    };
    let null = Value::Null;
    let arr = |v: &Value, k: &str| -> Vec<Value> {
        v.get(k).and_then(Value::as_array).cloned().unwrap_or_default()
    };
    for t in arr(stats, "tables") {
        let name = s(&t, "name");
        let user: Vec<&&Value> = tables
            .iter()
            .filter(|x| s(x, "DIMENSION_NAME") == name && !is_system(s(x, "TABLE_ID")))
            .collect();
        let rows_sum: f64 = user.iter().map(|x| n(x, "ROWS_COUNT")).sum();
        ck.check(
            &format!("{name} rows"),
            t.get("rows").unwrap_or(&null),
            &num(rows_sum),
        );
        for c in arr(&t, "columns") {
            let sid = s(&c, "storage_name");
            let what = format!("{name}[{}]", s(&c, "name"));
            let ours = |k: &str| c.get(k).cloned().unwrap_or(Value::Null);
            let dc = columns.iter().find(|x| {
                s(x, "DIMENSION_NAME") == name
                    && s(x, "COLUMN_ID") == sid
                    && !is_system(s(x, "TABLE_ID"))
                    && s(x, "COLUMN_TYPE") == "BASIC_DATA"
            });
            if let Some(dc) = dc {
                ck.check(
                    &format!("{what} dictionary_size"),
                    &ours("dictionary_size"),
                    dc.get("DICTIONARY_SIZE").unwrap_or(&null),
                );
                let enc = match dc.get("COLUMN_ENCODING").and_then(Value::as_f64) {
                    Some(1.0) => "hash",
                    Some(2.0) => "value",
                    _ => "none",
                };
                ck.check(&format!("{what} encoding"), &ours("encoding"), &Value::from(enc));
            }
            let mut segs: Vec<&&Value> = segments
                .iter()
                .filter(|x| {
                    s(x, "DIMENSION_NAME") == name && s(x, "COLUMN_ID") == sid && !is_system(s(x, "TABLE_ID"))
                })
                .collect();
            segs.sort_by(|a, b| {
                (n(a, "TABLE_PARTITION_NUMBER"), n(a, "SEGMENT_NUMBER"))
                    .partial_cmp(&(n(b, "TABLE_PARTITION_NUMBER"), n(b, "SEGMENT_NUMBER")))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let our_segs = arr(&c, "segments");
            ck.check(
                &format!("{what} segment count"),
                &Value::from(our_segs.len()),
                &Value::from(segs.len()),
            );
            for (o, sg) in our_segs.iter().zip(&segs) {
                let w = format!(
                    "{what} segment {}/{}",
                    show(sg.get("TABLE_PARTITION_NUMBER").unwrap_or(&null)),
                    show(sg.get("SEGMENT_NUMBER").unwrap_or(&null))
                );
                for (ok, tk) in [
                    ("rows", "RECORDS_COUNT"),
                    ("used_size", "USED_SIZE"),
                    ("bits", "BITS_COUNT"),
                    ("bookmark_bits", "BOOKMARK_BITS_COUNT"),
                    ("compression", "COMPRESSION_TYPE"),
                    ("state", "VERTIPAQ_STATE"),
                ] {
                    ck.check(
                        &format!("{w} {ok}"),
                        o.get(ok).unwrap_or(&null),
                        sg.get(tk).unwrap_or(&null),
                    );
                }
            }
            if c.get("is_row_number").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            let suffix = format!("${sid}");
            let h = tables.iter().find(|x| {
                let id = s(x, "TABLE_ID");
                s(x, "DIMENSION_NAME") == name && id.starts_with("H$") && id.ends_with(&suffix)
            });
            if let Some(h) = h {
                ck.check(
                    &format!("{what} cardinality"),
                    &ours("cardinality"),
                    &num(n(h, "ROWS_COUNT") - 3.0),
                );
                let hid = s(h, "TABLE_ID");
                let size: f64 = segments
                    .iter()
                    .filter(|x| s(x, "TABLE_ID") == hid)
                    .map(|x| n(x, "USED_SIZE"))
                    .sum();
                ck.check(
                    &format!("{what} hierarchy_size"),
                    &ours("hierarchy_size"),
                    &num(size),
                );
            }
        }
        for uh in arr(&t, "user_hierarchies") {
            let un = s(&uh, "name");
            let needle = format!("${un} (");
            let size: f64 = segments
                .iter()
                .filter(|x| {
                    let id = s(x, "TABLE_ID");
                    s(x, "DIMENSION_NAME") == name && id.starts_with("U$") && id.contains(&needle)
                })
                .map(|x| n(x, "USED_SIZE"))
                .sum();
            ck.check(
                &format!("{name} user hierarchy {un}"),
                uh.get("size").unwrap_or(&null),
                &num(size),
            );
        }
    }
    let rel_ours: f64 = arr(stats, "relationships").iter().map(|r| n(r, "size")).sum();
    let rel_theirs: f64 = segments
        .iter()
        .filter(|x| s(x, "TABLE_ID").starts_with("R$"))
        .map(|x| n(x, "USED_SIZE"))
        .sum();
    ck.check("relationships size", &num(rel_ours), &num(rel_theirs));
    let mut out = ck.out;
    out.extend(ck.diffs.iter().cloned());
    out.push(format!(
        "{} figures checked, {} differ",
        ck.checked,
        ck.diffs.len()
    ));
    (out, ck.diffs.is_empty())
}

pub fn cmd(args: &[String]) -> Result<(), String> {
    let verbose = args.iter().any(|a| a == "--verbose");
    let files: Vec<&String> = args.iter().filter(|a| *a != "--verbose").collect();
    let [dmv, stats] = files.as_slice() else {
        return Err("usage: cargo xtask storage-compare CASE.dmv.json CASE.stats.json [--verbose]".into());
    };
    let (lines, ok) = compare(
        &read_json(Path::new(dmv))?,
        &read_json(Path::new(stats))?,
        verbose,
    );
    for l in &lines {
        println!("{l}");
    }
    if ok {
        Ok(())
    } else {
        Err("storage figures differ".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn storage_compare_reports_differences() {
        let stats = json!({"results": [{"tables": [{"name": "T", "rows": 10, "user_hierarchies": [],
            "columns": [{"name": "A", "storage_name": "A (1)", "dictionary_size": 100,
                         "encoding": "hash", "is_row_number": false, "cardinality": 4,
                         "hierarchy_size": 24, "segments": [{"rows": 10, "used_size": 8, "bits": 2,
                         "bookmark_bits": 0, "compression": 1, "state": "COMPLETED"}]}]}],
            "relationships": []}]});
        let dmv = json!({
            "tables": [{"DIMENSION_NAME": "T", "TABLE_ID": "T (1)", "ROWS_COUNT": 10},
                       {"DIMENSION_NAME": "T", "TABLE_ID": "H$T (1)$A (1)", "ROWS_COUNT": 7}],
            "columns": {"DIMENSION_NAME": "T", "TABLE_ID": "T (1)", "COLUMN_ID": "A (1)",
                        "COLUMN_TYPE": "BASIC_DATA", "DICTIONARY_SIZE": 100, "COLUMN_ENCODING": 2},
            "segments": [{"DIMENSION_NAME": "T", "TABLE_ID": "T (1)", "COLUMN_ID": "A (1)",
                          "TABLE_PARTITION_NUMBER": 0, "SEGMENT_NUMBER": 0, "RECORDS_COUNT": 10,
                          "USED_SIZE": 8, "BITS_COUNT": 2, "BOOKMARK_BITS_COUNT": 0,
                          "COMPRESSION_TYPE": 1.0, "VERTIPAQ_STATE": "COMPLETED"},
                         {"DIMENSION_NAME": "T", "TABLE_ID": "H$T (1)$A (1)", "COLUMN_ID": "POS_TO_ID",
                          "USED_SIZE": 24}]});
        let (lines, ok) = compare(&dmv, &stats, false);
        assert!(!ok);
        assert_eq!(
            lines,
            [
                "T[A] encoding: bicraft hash, engine value",
                "13 figures checked, 1 differ"
            ]
        );
    }
}
