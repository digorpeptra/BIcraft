//! `table.rows` (`preview_rows.rs`): pages, sorts and filters against a full evaluation of the
//! table (every row built, filtered and sorted the plain way), on columns kept compressed as a
//! stored model's are and in memory; a calculated column the data does not hold; the order kept
//! between pages; and the micro-bench:
//!
//! `cargo test --release -p bicraft-engine table_rows_bench -- --ignored --nocapture` pages a
//! table of `BICRAFT_ROWS_BENCH_ROWS` rows (default 10 M): first page, sorted (cold, then the
//! next page), filtered, against the plain path (every row read into JSON, then sorted and
//! sliced).

use std::time::Instant;

use bicraft_vertipaq::CellValue;
use serde_json::{Value, json};

use super::super::store::{Codes, ColData};
use super::{Cond, compare, matches, parse_filter};
use crate::Session;

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The columns of the test table, one value per row: Id (whole), Name (text, repeated, some
/// blank), Amount (fixed decimal, some blank), Day (date).
fn columns(rows: usize) -> Vec<(&'static str, Vec<CellValue>)> {
    let mut id = Vec::with_capacity(rows);
    let mut name = Vec::with_capacity(rows);
    let mut amount = Vec::with_capacity(rows);
    let mut day = Vec::with_capacity(rows);
    for k in 0..rows as u64 {
        let r = splitmix(k);
        id.push(CellValue::Int(k as i64 + 1));
        name.push(match r % 41 {
            0 => CellValue::Null,
            n if n % 2 == 0 => CellValue::Text(format!("item {}", n % 13)),
            n => CellValue::Text(format!("Item {}", n % 13)),
        });
        amount.push(match (r >> 8) % 29 {
            0 => CellValue::Null,
            _ => CellValue::Decimal(((r >> 16) % 100_000) as i64 * 100),
        });
        day.push(CellValue::DateTime(45_000.0 + ((r >> 32) % 900) as f64));
    }
    vec![("Id", id), ("Name", name), ("Amount", amount), ("Day", day)]
}

/// A session with table T(Id, Name, Amount, Day, Double = T[Amount] * 2) of `rows` rows, its
/// columns compressed (`packed`) or in memory.
fn session(rows: usize, packed: bool) -> (Session, Vec<(&'static str, Vec<CellValue>)>) {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Rows"})),
        (
            "table.add",
            json!({"name": "T", "partition": {"source_type": "m", "expression": "#table({}, {})"}}),
        ),
        (
            "column.add",
            json!({"table": "T", "name": "Id", "data_type": "int64"}),
        ),
        (
            "column.add",
            json!({"table": "T", "name": "Name", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "T", "name": "Amount", "data_type": "decimal"}),
        ),
        (
            "column.add",
            json!({"table": "T", "name": "Day", "data_type": "dateTime"}),
        ),
        (
            "column.add",
            json!({"table": "T", "name": "Double", "data_type": "decimal", "expression": "T[Amount] * 2"}),
        ),
    ] {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    let cols = columns(rows);
    let data: Vec<(String, ColData)> = cols
        .iter()
        .map(|(n, v)| {
            let c = ColData::from_stored(v.clone()).unwrap();
            let c = if packed {
                ColData {
                    codes: Codes::pack(&c, 4096),
                    ..c
                }
            } else {
                c
            };
            ((*n).to_string(), c)
        })
        .collect();
    super::super::inject_table(&s, "T", rows, data);
    (s, cols)
}

/// The rows a full evaluation gives: every row built (Double computed), filtered, sorted (stable),
/// as JSON, with the columns in the table's order.
fn full(cols: &[(&str, Vec<CellValue>)], sort: Option<(&str, bool)>, filter: &Value) -> Vec<Vec<Value>> {
    let n = cols.first().map_or(0, |c| c.1.len());
    let names: Vec<&str> = cols.iter().map(|c| c.0).chain(["Double"]).collect();
    let mut rows: Vec<Vec<CellValue>> = (0..n)
        .map(|i| {
            let mut r: Vec<CellValue> = cols.iter().map(|c| c.1[i].clone()).collect();
            r.push(match &r[2] {
                CellValue::Decimal(x) => CellValue::Decimal(x * 2),
                _ => CellValue::Null,
            });
            r
        })
        .collect();
    let conds: Vec<Cond> = parse_filter(Some(filter)).unwrap();
    rows.retain(|r| {
        conds.iter().all(|c| {
            let i = names
                .iter()
                .position(|n| n.eq_ignore_ascii_case(&c.column))
                .unwrap();
            matches(&r[i], c)
        })
    });
    if let Some((col, desc)) = sort {
        let i = names.iter().position(|n| *n == col).unwrap();
        rows.sort_by(|a, b| {
            let o = compare(&a[i], &b[i]);
            if desc { o.reverse() } else { o }
        });
    }
    rows.iter()
        .map(|r| r.iter().map(super::super::cell_json).collect())
        .collect()
}

fn page(s: &mut Session, p: Value) -> Value {
    s.execute("table.rows", &p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn rows_of(v: &Value) -> Vec<Vec<Value>> {
    v["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_array().unwrap().clone())
        .collect()
}

#[test]
fn table_rows_pages_match_a_full_evaluation() {
    for packed in [true, false] {
        let rows = 20_000;
        let (mut s, cols) = session(rows, packed);
        let cases: Vec<(Option<(&str, bool)>, Value)> = vec![
            (None, json!([])),
            (Some(("Name", false)), json!([])),
            (Some(("Name", true)), json!([])),
            (Some(("Amount", false)), json!([])),
            (Some(("Day", true)), json!([])),
            (Some(("Double", true)), json!([])),
            (
                None,
                json!([{"column": "Name", "values": ["item 4", "Item 5", null]}]),
            ),
            (
                Some(("Amount", true)),
                json!([{"column": "Amount", "op": ">=", "value": 500}, {"column": "Name", "op": "contains", "value": "1"}]),
            ),
            (
                Some(("Id", true)),
                json!([{"column": "Day", "op": "<", "value": "2023-03-01"}]),
            ),
            (None, json!([{"column": "Amount", "op": "blank"}])),
            (
                Some(("Name", false)),
                json!([{"column": "Double", "op": ">", "value": 1000}]),
            ),
            (
                None,
                json!([{"column": "Name", "op": "not_in", "values": ["item 2"]}, {"column": "Name", "op": "not_blank"}]),
            ),
            (
                Some(("Amount", false)),
                json!([{"column": "Name", "any": [{"op": "ends_with", "value": "3"}, {"op": "blank"}]}]),
            ),
        ];
        for (sort, filter) in cases {
            let want = full(&cols, sort, &filter);
            for (offset, limit) in [
                (0usize, 100usize),
                (137, 250),
                (want.len().saturating_sub(5), 100),
                (rows + 10, 10),
            ] {
                let mut p = json!({"table": "T", "offset": offset, "limit": limit, "filter": filter});
                if let Some((c, d)) = sort {
                    p["sort"] = json!({"column": c, "descending": d});
                }
                let got = page(&mut s, p.clone());
                assert_eq!(got["row_count"].as_u64(), Some(want.len() as u64), "{p}");
                assert_eq!(got["total_rows"].as_u64(), Some(rows as u64), "{p}");
                let end = offset.saturating_add(limit).min(want.len());
                let expect = want.get(offset.min(end)..end).unwrap_or(&[]);
                assert_eq!(rows_of(&got), expect, "packed {packed}, {p}");
            }
        }
        // the column metadata
        let v = page(&mut s, json!({"table": "T", "limit": 1}));
        let names: Vec<&str> = v["columns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["Id", "Name", "Amount", "Day", "Double"]);
        assert_eq!(v["columns"][4]["kind"], "calculated");
        assert_eq!(v["columns"][2]["data_type"], "decimal");
    }
}

#[test]
fn table_rows_keeps_the_order_until_the_model_changes() {
    let (mut s, _) = session(5_000, true);
    let p = json!({"table": "T", "sort": {"column": "Name"}, "offset": 0, "limit": 50});
    let a = page(&mut s, p.clone());
    assert_eq!(a["order_cached"], false);
    let mut next = p.clone();
    next["offset"] = json!(50);
    let b = page(&mut s, next);
    assert_eq!(b["order_cached"], true);
    s.execute(
        "measure.add",
        &json!({"table": "T", "name": "N", "expression": "COUNTROWS(T)"}),
    )
    .unwrap();
    let c = page(&mut s, p);
    assert_eq!(c["order_cached"], false);
    assert_eq!(c["rows"], a["rows"]);
    // a sort on a calculated column computes it once
    let d = page(
        &mut s,
        json!({"table": "T", "sort": {"column": "Double", "order": "desc"}, "limit": 3}),
    );
    assert_eq!(d["rows"].as_array().unwrap().len(), 3);
    assert!(s.preview.lock().rows_kept.len() >= 2);
}

#[test]
fn table_rows_refuses_bad_params_and_says_when_there_is_no_data() {
    let (mut s, _) = session(10, false);
    for bad in [
        json!({}),
        json!({"table": "Nope"}),
        json!({"table": "T", "limit": 0}),
        json!({"table": "T", "limit": 10_001}),
        json!({"table": "T", "offset": -1}),
        json!({"table": "T", "columns": ["Nope"]}),
        json!({"table": "T", "sort": {"column": "Nope"}}),
        json!({"table": "T", "sort": {"column": "Id", "order": "up"}}),
        json!({"table": "T", "filter": [{"column": "Id", "op": "~", "value": 1}]}),
        json!({"table": "T", "filter": [{"column": "Id", "op": "blank", "value": 1}]}),
        json!({"table": "T", "filter": [{"column": "Id", "value": [1]}]}),
        json!({"table": "T", "filter": "Id = 1"}),
        json!({"table": "T", "extra": 1}),
    ] {
        assert!(s.execute("table.rows", &bad).is_err(), "{bad}");
    }
    // a page past the end is empty
    let v = page(&mut s, json!({"table": "T", "offset": 4_000_000_000u64}));
    assert_eq!(v["rows"], json!([]));
    // a table without data says why
    s.execute(
        "table.add",
        &json!({"name": "Empty", "partition": {"source_type": "m", "expression": "Web.Contents(\"https://example.com\")"}}),
    )
    .unwrap();
    s.execute(
        "column.add",
        &json!({"table": "Empty", "name": "A", "data_type": "string"}),
    )
    .unwrap();
    let v = page(&mut s, json!({"table": "Empty"}));
    assert_eq!(v["available"], false, "{v}");
    assert!(v["reason"].as_str().is_some_and(|r| !r.is_empty()));
}

#[test]
fn table_rows_reads_a_literal_table_with_a_calculated_column() {
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Amount"}, {{"Wheels", 100}, {"Toys", 5}, {"Games", 20}})"#;
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Category", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Double", "data_type": "double", "expression": "Sales[Amount] * 2"}),
        ),
    ] {
        s.execute(id, &p).unwrap();
    }
    let v = page(
        &mut s,
        json!({"table": "Sales", "sort": {"column": "Double", "descending": true}}),
    );
    assert_eq!(
        v["rows"],
        json!([["Wheels", 100, 200], ["Games", 20, 40], ["Toys", 5, 10]]),
        "{v}"
    );
    let v = page(
        &mut s,
        json!({"table": "Sales", "filter": [{"column": "Category", "values": ["toys"]}]}),
    );
    assert_eq!(v["rows"], json!([["Toys", 5, 10]]), "{v}");
    // the header menu's text and number filters: ends with, and a custom filter with Or
    let v = page(
        &mut s,
        json!({"table": "Sales", "filter": [{"column": "Category", "op": "ends_with", "value": "S"}]}),
    );
    assert_eq!(
        v["rows"],
        json!([["Wheels", 100, 200], ["Toys", 5, 10], ["Games", 20, 40]]),
        "{v}"
    );
    let v = page(
        &mut s,
        json!({"table": "Sales", "filter": [{"column": "Category", "op": "not_ends_with", "value": "ys"}]}),
    );
    assert_eq!(v["rows"], json!([["Wheels", 100, 200], ["Games", 20, 40]]), "{v}");
    let v = page(
        &mut s,
        json!({"table": "Sales", "filter": [{"column": "Amount", "any": [{"op": "<", "value": 10}, {"op": ">=", "value": 100}]}]}),
    );
    assert_eq!(v["rows"], json!([["Wheels", 100, 200], ["Toys", 5, 10]]), "{v}");
    // with And, as two conditions on the column
    let v = page(
        &mut s,
        json!({"table": "Sales", "filter": [{"column": "Amount", "op": ">", "value": 10}, {"column": "Amount", "op": "<", "value": 100}]}),
    );
    assert_eq!(v["rows"], json!([["Games", 20, 40]]), "{v}");
    for bad in [
        json!({"table": "Sales", "filter": [{"column": "Amount", "any": []}]}),
        json!({"table": "Sales", "filter": [{"column": "Amount", "any": [{"op": ">", "value": 1}], "op": "<"}]}),
        json!({"table": "Sales", "filter": [{"column": "Amount", "any": [{"op": ">", "value": 1, "x": 2}]}]}),
        json!({"table": "Sales", "filter": [{"column": "Amount", "any": [{"op": ">"}]}]}),
        json!({"table": "Sales", "filter": [{"column": "Amount", "any": "x"}]}),
        json!({"table": "Sales", "filter": [{"column": "Amount", "any": vec![json!({"op": ">", "value": 1}); 9]}]}),
    ] {
        assert!(s.execute("table.rows", &bad).is_err(), "{bad}");
    }
    // a shared run gives the same answer
    let shared = s
        .execute_shared("table.rows", &json!({"table": "Sales", "limit": 2}), None)
        .unwrap();
    assert_eq!(shared["rows"], json!([["Wheels", 100, 200], ["Toys", 5, 10]]));
}

fn env_rows(key: &str, default: usize) -> usize {
    bicraft_env::var(key)
        .ok()
        .and_then(|v| v.replace('_', "").parse().ok())
        .unwrap_or(default)
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// The plain path: every row of the table read into JSON (as `dax.evaluate` of the whole table
/// returns it), sorted, then the page sliced.
fn naive_page(
    cols: &[(&str, Vec<CellValue>)],
    sort: Option<usize>,
    offset: usize,
    limit: usize,
) -> Vec<Vec<Value>> {
    let n = cols.first().map_or(0, |c| c.1.len());
    let mut rows: Vec<(usize, Vec<Value>)> = (0..n)
        .map(|i| (i, cols.iter().map(|c| super::super::cell_json(&c.1[i])).collect()))
        .collect();
    if let Some(k) = sort {
        rows.sort_by(|a, b| compare(&cols[k].1[a.0], &cols[k].1[b.0]));
    }
    rows.into_iter().skip(offset).take(limit).map(|r| r.1).collect()
}

#[test]
#[ignore]
fn table_rows_bench() {
    let rows = env_rows("BICRAFT_ROWS_BENCH_ROWS", 10_000_000);
    let t = Instant::now();
    let (mut s, cols) = session(rows, true);
    println!(
        "table of {rows} rows (4 columns kept compressed) built in {:.0} ms",
        ms(t)
    );
    let mut time = |label: &str, p: Value| -> (f64, Value) {
        let t = Instant::now();
        let v = page(&mut s, p);
        let m = ms(t);
        println!("{label:<44} {m:>9.1} ms  (rows {})", v["row_count"]);
        (m, v)
    };
    let (first, _) = time(
        "first page (100 rows)",
        json!({"table": "T", "columns": ["Id", "Name", "Amount", "Day"]}),
    );
    let (mid, _) = time(
        "a page in the middle",
        json!({"table": "T", "offset": rows / 2, "columns": ["Id", "Name", "Amount", "Day"]}),
    );
    let sort =
        json!({"table": "T", "sort": {"column": "Amount"}, "columns": ["Id", "Name", "Amount", "Day"]});
    let (cold, v_sorted) = time("sorted on Amount, cold (orders every row)", sort.clone());
    let mut next = sort.clone();
    next["offset"] = json!(rows / 3);
    let (warm, _) = time("sorted, another page (order kept)", next);
    let (filtered, _) = time(
        "filtered (Name in 2 values), sorted on Name desc",
        json!({"table": "T", "sort": {"column": "Name", "descending": true},
               "filter": [{"column": "Name", "values": ["item 4", "Item 5"]}],
               "columns": ["Id", "Name", "Amount", "Day"]}),
    );
    let t = Instant::now();
    let plain_first = naive_page(&cols, None, 0, 100);
    let plain_unsorted = ms(t);
    let t = Instant::now();
    let plain = naive_page(&cols, Some(2), 0, 100);
    let plain_sorted = ms(t);
    assert_eq!(rows_of(&v_sorted), plain);
    assert_eq!(plain_first.len(), 100);
    println!("plain path, first page (every row to JSON)   {plain_unsorted:>9.1} ms");
    println!("plain path, sorted on Amount                 {plain_sorted:>9.1} ms");
    let x = |a: f64, b: f64| b / a.max(0.001);
    println!(
        "speedups: first page x{:.0}, middle page x{:.0}, cold sort x{:.0}, next sorted page x{:.0}, filtered x{:.0}",
        x(first, plain_unsorted),
        x(mid, plain_unsorted),
        x(cold, plain_sorted),
        x(warm, plain_sorted),
        x(filtered, plain_sorted)
    );
}
