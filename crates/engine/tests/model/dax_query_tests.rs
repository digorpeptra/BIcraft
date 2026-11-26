//! Tests of `dax.evaluate` (M9.4). The engine oracle (values compared with Analysis Services) is
//! `query_oracle_matches_the_engine` below, over `tests/dax_queries`.

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

/// A session on the small literal model of the table-function tests (data from its DATATABLE
/// partitions, no stored data).
fn literal_session() -> Session {
    let mut s = Session::new();
    s.model = Some(bicraft_tmsl::read(include_str!("../table_functions/model.bim")).unwrap());
    s
}

fn eval(s: &mut Session, query: &str) -> Value {
    s.execute("dax.evaluate", &json!({ "query": query }))
        .unwrap_or_else(|e| panic!("{query}: {e}"))
}

fn err(s: &mut Session, query: &str) -> String {
    match s.execute("dax.evaluate", &json!({ "query": query })) {
        Ok(v) => panic!("{query}: expected an error, got {v}"),
        Err(e) => e,
    }
}

/// The oracle model saved as a PBIX (data in the DataModel), opened.
fn oracle_pbix(dir: &TempDir) -> Session {
    let bim = dir.path().join("model.bim");
    std::fs::write(&bim, include_str!("../dax_queries/model.bim")).unwrap();
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": bim})).unwrap();
    let pbix = dir.path().join("q.pbix");
    s.execute("file.save_as", &json!({"path": pbix})).unwrap();
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": pbix})).unwrap();
    s
}

#[test]
fn columns_rows_and_types() {
    let mut s = literal_session();
    let v = eval(
        &mut s,
        "EVALUATE SUMMARIZECOLUMNS(Product[Category], \"Total\", [Total], \"N\", COUNTROWS(Sales)) \
         ORDER BY Product[Category]",
    );
    assert_eq!(v["data_source"], "partitions");
    let r = &v["results"][0];
    assert_eq!(
        r["columns"],
        json!([
            {"name": "Product[Category]", "type": "string"},
            {"name": "[Total]", "type": "double"},
            {"name": "[N]", "type": "int64"},
        ])
    );
    // ProductKey 9 has no product: its sale goes to the blank row, sorted first.
    assert_eq!(
        r["rows"],
        json!([
            {"Product[Category]": null, "[Total]": 50.0, "[N]": 1},
            {"Product[Category]": "Toys", "[Total]": 75.0, "[N]": 4},
            {"Product[Category]": "Wheels", "[Total]": 750.0, "[N]": 4},
        ])
    );
    assert_eq!(r["row_count"], 3);
    assert_eq!(r["truncated"], false);
}

#[test]
fn dates_blanks_and_currency() {
    let mut s = literal_session();
    let v = eval(
        &mut s,
        "EVALUATE ROW(\"d\", DATE(2024, 2, 29), \"b\", BLANK(), \"c\", MAX(Product[Price]), \"t\", TRUE())",
    );
    let r = &v["results"][0];
    assert_eq!(
        r["rows"][0],
        json!({"[d]": "2024-02-29T00:00:00", "[b]": null, "[c]": 250.0, "[t]": true})
    );
    assert_eq!(r["columns"][0]["type"], "dateTime");
    assert_eq!(r["columns"][2]["type"], "decimal");
}

#[test]
fn order_by_start_at_and_several_evaluates() {
    let mut s = literal_session();
    let v = eval(
        &mut s,
        "EVALUATE Product ORDER BY Product[Price] DESC START AT 100 \
         EVALUATE {1, 2} \
         EVALUATE SELECTCOLUMNS(Customer, \"Who\", Customer[Name]) ORDER BY [Who] DESC",
    );
    let results = v["results"].as_array().unwrap();
    assert_eq!(results.len(), 3);
    let names: Vec<&str> = results[0]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["Product[Product]"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Bike", "Kite", "Ball"]);
    assert_eq!(results[1]["columns"][0]["name"], "[Value]");
    assert_eq!(results[1]["rows"], json!([{"[Value]": 1}, {"[Value]": 2}]));
    assert_eq!(results[2]["rows"][0]["[Who]"], "Cid");
}

#[test]
fn query_definitions() {
    let mut s = literal_session();
    let v = eval(
        &mut s,
        "DEFINE \
           MEASURE Sales[Big] = CALCULATE([Total], Sales[Amount] >= 100) \
           MEASURE Sales[Total] = SUM(Sales[Amount]) * 2 \
           VAR Threshold = 100 \
           TABLE Cheap = FILTER(Product, Product[Price] < Threshold) \
           COLUMN Sales[Double Qty] = Sales[Qty] * 2 \
         EVALUATE ROW(\"Big\", [Big], \"Cheap\", COUNTROWS(Cheap), \"Q2\", SUM(Sales[Double Qty]))",
    );
    // The query's [Total] replaces the model's: [Big] doubles too.
    assert_eq!(
        v["results"][0]["rows"][0],
        json!({"[Big]": 1500.0, "[Cheap]": 2, "[Q2]": 34})
    );
    // Nothing is changed in the model.
    let m = s.model.as_ref().unwrap();
    let sales = m.tables.iter().find(|t| t.name == "Sales").unwrap();
    assert!(sales.columns.iter().all(|c| c.name != "Double Qty"));
    assert!(sales.measures.iter().all(|x| x.name != "Big"));
}

/// User-defined functions (Learn, "DAX user-defined functions"): the query's and the model's,
/// `val` parameters evaluated once in the caller's context, `expr` ones in the body's.
#[test]
fn user_defined_functions_are_evaluated() {
    let mut s = literal_session();
    let v = eval(
        &mut s,
        "DEFINE \
           FUNCTION AddTax = (amount : NUMERIC, rate : NUMERIC = 0.1) => amount * (1 + rate) \
           FUNCTION Twice = (x) => AddTax(x, 1) \
         EVALUATE ROW(\"a\", AddTax(10), \"b\", AddTax(10, 0.5), \"c\", Twice(3))",
    );
    assert_eq!(
        v["results"][0]["rows"][0],
        json!({"[a]": 11.0, "[b]": 15.0, "[c]": 6.0})
    );
    // a model function called by a model measure, with a lazy table and an eager one
    s.execute(
        "function.add",
        &json!({"name": "RowsAll", "expression": "(t : TABLE EXPR) => CALCULATE(COUNTROWS(t), ALL(Product))"}),
    )
    .unwrap();
    s.execute(
        "function.add",
        &json!({"name": "RowsNow", "expression": "(t : TABLE VAL) => CALCULATE(COUNTROWS(t), ALL(Product))"}),
    )
    .unwrap();
    s.execute(
        "measure.add",
        &json!({"table": "Sales", "name": "Lazy", "expression": "RowsAll(Sales)"}),
    )
    .unwrap();
    s.execute(
        "measure.add",
        &json!({"table": "Sales", "name": "Eager", "expression": "RowsNow(Sales)"}),
    )
    .unwrap();
    let v = eval(
        &mut s,
        "EVALUATE ROW(\"lazy\", CALCULATE([Lazy], Product[Category] = \"Toys\"), \
                      \"eager\", CALCULATE([Eager], Product[Category] = \"Toys\"))",
    );
    let row = &v["results"][0]["rows"][0];
    assert_eq!(row["[eager]"], 4, "{row}");
    assert_eq!(row["[lazy]"], 9, "{row}");
}

#[test]
fn orphan_keys_and_treatas() {
    let mut s = literal_session();
    let v = eval(
        &mut s,
        "EVALUATE EXCEPT(VALUES(Sales[ProductKey]), VALUES(Product[ProductKey])) \
         EVALUATE FILTER(Sales, ISBLANK(RELATED(Product[Product]))) \
         EVALUATE SUMMARIZECOLUMNS(Customer[City], TREATAS({\"Web\"}, Sales[Channel]), \"T\", [Total])",
    );
    assert_eq!(v["results"][0]["rows"], json!([{"Sales[ProductKey]": 9}]));
    assert_eq!(v["results"][1]["row_count"], 1);
    assert_eq!(v["results"][1]["rows"][0]["Sales[SalesKey]"], 4);
    let web = &v["results"][2]["rows"];
    assert_eq!(web.as_array().unwrap().len(), 2, "{web}");
}

#[test]
fn max_rows_truncates() {
    let mut s = literal_session();
    let v = s
        .execute(
            "dax.evaluate",
            &json!({"query": "EVALUATE 'Date'", "max_rows": 5}),
        )
        .unwrap();
    let r = &v["results"][0];
    assert_eq!(r["rows"].as_array().unwrap().len(), 5);
    assert_eq!(r["row_count"], 731);
    assert_eq!(r["truncated"], true);
}

#[test]
fn errors_name_the_problem() {
    let mut s = literal_session();
    // an engine function bicraft does not evaluate: named, no number
    let e = err(&mut s, "EVALUATE ROW(\"x\", RAND())");
    assert!(e.contains("RAND"), "{e}");
    // invalid queries are refused with their diagnostics
    let e = err(&mut s, "EVALUATE ROW(\"x\", [Nope])");
    assert!(e.starts_with("the query is not valid"), "{e}");
    for bad in [
        json!({"query": "EVALUATE {1}", "max_rows": 0}),
        json!({"query": "EVALUATE {1}", "timeout_ms": "x"}),
        json!({"query": "EVALUATE {1}", "allow_cache_abf": 1}),
        json!({"query": "EVALUATE {1}", "other": 1}),
        json!({}),
    ] {
        assert!(s.execute("dax.evaluate", &bad).is_err(), "{bad}");
    }
    let mut empty = Session::new();
    assert!(err(&mut empty, "EVALUATE {1}").contains("no model"));
    // a read-only command: the session is unchanged
    let rev = s.revision;
    eval(&mut s, "EVALUATE {1}");
    assert_eq!(s.revision, rev);
}

#[test]
fn time_budget_is_enforced() {
    let mut s = literal_session();
    let q = "EVALUATE ROW(\"x\", SUMX(GENERATESERIES(1, 1500000), [Value] * 2))";
    let e = s
        .execute("dax.evaluate", &json!({"query": q, "timeout_ms": 1}))
        .unwrap_err();
    assert!(e.contains("budget"), "{e}");
}

#[test]
fn stored_data_of_a_pbix_and_a_pbip_cache() {
    let dir = TempDir::new("engine-dax-query-stored");
    let mut s = oracle_pbix(&dir);
    let q = "EVALUATE ROW(\"n\", COUNTROWS(Sales), \"t\", [Total Sales])";
    let v = eval(&mut s, q);
    assert_eq!(v["data_source"], "DataModel");
    assert_eq!(v["results"][0]["rows"][0]["[n]"], 600);
    let total = v["results"][0]["rows"][0]["[t]"].clone();

    // the same data from a PBIP's .pbi/cache.abf; refused with allow_cache_abf: false
    let pbip = dir.path().join("proj").join("Q.pbip");
    s.execute("file.save_as", &json!({"path": pbip})).unwrap();
    let mut p = Session::new();
    p.execute("file.open", &json!({"path": pbip})).unwrap();
    let v = eval(&mut p, q);
    assert_eq!(v["data_source"], "cache.abf");
    assert_eq!(v["results"][0]["rows"][0]["[t]"], total);
    let e = p
        .execute("dax.evaluate", &json!({"query": q, "allow_cache_abf": false}))
        .unwrap_err();
    assert!(e.contains("cache.abf"), "{e}");
}

/// The query corpus of `tests/dax_queries` against the results Analysis Services gave
/// (`expected.json`, recorded with the development oracles; see docs/dax-oracle.md).
#[test]
fn query_oracle_matches_the_engine() {
    let doc: Value = serde_json::from_str(include_str!("../dax_queries/expected.json")).unwrap();
    let dir = TempDir::new("engine-dax-query-oracle");
    let mut s = oracle_pbix(&dir);
    let mut agree = 0usize;
    let mut refused = Vec::new();
    let mut wrong = Vec::new();
    let mut type_diffs = Vec::new();
    let entries = doc["queries"].as_array().unwrap();
    for q in entries {
        let id = q["id"].as_str().unwrap();
        let query = q["query"].as_str().unwrap();
        let got = s.execute("dax.evaluate", &json!({"query": query, "max_rows": 100_000}));
        if q.get("error").is_some() {
            match got {
                Err(_) => agree += 1,
                Ok(_) => wrong.push(format!("{id}: the engine fails, bicraft returns rows")),
            }
            continue;
        }
        let got = match got {
            Ok(v) => v,
            Err(e) => {
                refused.push(format!("{id}: {e}"));
                continue;
            }
        };
        let ordered = q["ordered"].as_bool().unwrap_or(false);
        match compare(&got["results"], &q["results"], ordered) {
            Ok(types) => {
                agree += 1;
                if !types.is_empty() {
                    type_diffs.push(format!("{id}: {}", types.join(", ")));
                }
            }
            Err(d) => wrong.push(format!("{id}: {d}")),
        }
    }
    let n = entries.len();
    println!(
        "dax.evaluate oracle: {agree}/{n} agree, {} refused (unsupported), {} wrong",
        refused.len(),
        wrong.len()
    );
    for r in &refused {
        println!("  refused  {r}");
    }
    for t in &type_diffs {
        println!("  type     {t}");
    }
    assert!(
        wrong.is_empty(),
        "results that differ from the engine:\n{}",
        wrong.join("\n")
    );
    assert!(
        type_diffs.is_empty(),
        "column types that differ:\n{}",
        type_diffs.join("\n")
    );
    assert!(agree * 100 >= n * 95, "agreement below 95%: {agree}/{n}");
}

/// Function coverage (`tests/dax_coverage`, Desktop 2.159): math, information, conversion,
/// date and text conversions, calculation groups, format strings. Unsupported functions are
/// counted, never wrong; `KNOWN` are the remaining value or type differences, with their reason.
#[test]
fn coverage_oracle_matches_the_engine() {
    const KNOWN: &[(&str, &str)] = &[
        (
            "fn-abs-blank",
            "a blank of an integer expression is typed int64 by the engine",
        ),
        (
            "fn-coalesce-blank",
            "a blank of an integer expression is typed int64 by the engine",
        ),
        (
            "fn-blank-times",
            "a blank of an integer expression is typed int64 by the engine",
        ),
        (
            "fn-blank-plus",
            "bicraft has one number type for integers and doubles",
        ),
        (
            "fn-iserror",
            "ISERROR of an infinite division is TRUE in the engine",
        ),
        (
            "fn-averagex-blank",
            "AVERAGEX over Currency values is typed decimal by the engine",
        ),
    ];
    let doc: Value = serde_json::from_str(include_str!("../dax_coverage/expected.json")).unwrap();
    let dir = TempDir::new("engine-dax-coverage-oracle");
    let bim = dir.path().join("model.bim");
    std::fs::write(&bim, include_str!("../dax_coverage/model.bim")).unwrap();
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": bim})).unwrap();
    let pbix = dir.path().join("c.pbix");
    s.execute("file.save_as", &json!({"path": pbix})).unwrap();
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": pbix})).unwrap();
    let (mut agree, mut refused, mut known, mut wrong) = (0usize, Vec::new(), 0usize, Vec::new());
    let entries = doc["queries"].as_array().unwrap();
    for q in entries {
        let id = q["id"].as_str().unwrap();
        let query = q["query"].as_str().unwrap();
        let got = s.execute(
            "dax.evaluate",
            &json!({"query": query, "max_rows": 100_000, "as_of": "2026-10-23T16:43:44", "utc_offset_minutes": 120}),
        );
        let verdict = if q.get("error").is_some() {
            match got {
                Err(_) => Ok(()),
                Ok(_) => Err("the engine fails, bicraft returns rows".to_string()),
            }
        } else {
            match got {
                Err(e) if e.contains("not supported") => {
                    refused.push(format!("{id}: {e}"));
                    continue;
                }
                Err(e) => Err(format!("bicraft fails: {e}")),
                Ok(v) => match compare(
                    &v["results"],
                    &q["results"],
                    q["ordered"].as_bool().unwrap_or(false),
                ) {
                    Ok(types) if types.is_empty() => Ok(()),
                    Ok(types) => Err(types.join(", ")),
                    Err(d) => Err(d),
                },
            }
        };
        match verdict {
            Ok(()) => agree += 1,
            Err(_) if KNOWN.iter().any(|(k, _)| *k == id) => known += 1,
            Err(d) => wrong.push(format!("{id}: {d}")),
        }
    }
    println!(
        "dax coverage oracle: {agree}/{} agree, {} refused (unsupported), {known} known differences",
        entries.len(),
        refused.len()
    );
    for r in &refused {
        println!("  refused  {r}");
    }
    assert!(
        wrong.is_empty(),
        "results that differ from the engine:\n{}",
        wrong.join("\n")
    );
    // the window, balance, GROUPBY and natural-join queries are evaluated (lot 20), then
    // ADDMISSINGITEMS, SUBSTITUTEWITHINDEX, SAMPLE and TOPNSKIP: no more refusals than the two
    // functions still left out
    assert!(
        refused.len() <= 2,
        "more queries refused than before:\n{}",
        refused.join("\n")
    );
}

/// Compares our results with the engine's; returns the column type differences.
fn compare(ours: &Value, theirs: &Value, ordered: bool) -> Result<Vec<String>, String> {
    let (ours, theirs) = (
        ours.as_array().ok_or("no results")?,
        theirs.as_array().ok_or("no results")?,
    );
    if ours.len() != theirs.len() {
        return Err(format!("{} results, engine {}", ours.len(), theirs.len()));
    }
    let mut types = Vec::new();
    for (k, (o, t)) in ours.iter().zip(theirs).enumerate() {
        let names = |v: &Value| -> Vec<String> {
            v["columns"]
                .as_array()
                .map(|c| {
                    c.iter()
                        .map(|x| x["name"].as_str().unwrap_or("").to_string())
                        .collect()
                })
                .unwrap_or_default()
        };
        if names(o) != names(t) {
            return Err(format!(
                "result {k}: columns {:?}, engine {:?}",
                names(o),
                names(t)
            ));
        }
        for (a, b) in o["columns"]
            .as_array()
            .unwrap()
            .iter()
            .zip(t["columns"].as_array().unwrap())
        {
            // The engine's client reports a currency column as decimal and every integer as int64.
            if a["type"] != b["type"] && !(b["type"].is_null()) {
                types.push(format!("{} {} (engine {})", a["name"], a["type"], b["type"]));
            }
        }
        let cols = names(o);
        let row = |r: &Value| -> Vec<Value> { cols.iter().map(|c| r[c.as_str()].clone()).collect() };
        let mut ro: Vec<Vec<Value>> = o["rows"].as_array().unwrap().iter().map(row).collect();
        let mut rt: Vec<Vec<Value>> = t["rows"].as_array().unwrap().iter().map(row).collect();
        if ro.len() != rt.len() {
            return Err(format!("result {k}: {} rows, engine {}", ro.len(), rt.len()));
        }
        if !ordered {
            let key =
                |r: &Vec<Value>| serde_json::to_string(&r.iter().map(canon).collect::<Vec<_>>()).unwrap();
            ro.sort_by_key(key);
            rt.sort_by_key(key);
        }
        for (i, (a, b)) in ro.iter().zip(&rt).enumerate() {
            for (j, (x, y)) in a.iter().zip(b).enumerate() {
                if !same(x, y) {
                    return Err(format!("result {k} row {i} column {}: {x}, engine {y}", cols[j]));
                }
            }
        }
    }
    Ok(types)
}

/// A value for sorting rows: numbers rounded so tiny float differences sort alike.
fn canon(v: &Value) -> Value {
    match v.as_f64() {
        Some(x) => json!(format!("{:.6e}", x)),
        None => v.clone(),
    }
}

fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y || (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0),
        _ => a == b,
    }
}

/// `ORDER BY` keys that name a result column are read in place; they order (and `START AT`
/// cuts) exactly as the same keys evaluated as expressions (`IF(TRUE(), key)`), ascending and
/// descending, on text (case and accents), numbers, blanks, and next to evaluated keys.
#[test]
fn order_by_result_columns_matches_evaluated_keys() {
    fn slow(keys: &str) -> String {
        keys.split(", ")
            .map(|k| match k.strip_suffix(" DESC") {
                Some(k) => format!("IF(TRUE(), {k}) DESC"),
                None => format!("IF(TRUE(), {k})"),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
    let mut s = literal_session();
    let table = "{(1, \"b\", 2), (2, \"B\", BLANK()), (3, \"a\", 2), (4, \"\u{e9}\", -1), (5, \"E\", 2), \
                 (6, BLANK(), 0), (7, \"A\", 2.5), (8, \"\u{c9}\", -1), (9, \"b\", 1), (10, \"\", BLANK())}";
    let keys = [
        "[Value2], [Value1] DESC",
        "[Value3] DESC, [Value2], [Value1]",
        "[Value2] DESC, [Value3], [Value1]",
        "[Value3], [Value1] DESC",
        "[Value2], [Value1] + 0",
    ];
    for k in keys {
        let a = eval(&mut s, &format!("EVALUATE {table} ORDER BY {k}"));
        let b = eval(&mut s, &format!("EVALUATE {table} ORDER BY {}", slow(k)));
        assert_eq!(a["results"][0]["rows"], b["results"][0]["rows"], "{k}");
    }
    let cuts = [
        ("[Value2], [Value1]", "\"b\""),
        ("[Value2] DESC", "\"B\""),
        ("[Value3] DESC, [Value1]", "2, 5"),
    ];
    for (k, at) in cuts {
        let a = eval(&mut s, &format!("EVALUATE {table} ORDER BY {k} START AT {at}"));
        let b = eval(
            &mut s,
            &format!("EVALUATE {table} ORDER BY {} START AT {at}", slow(k)),
        );
        assert_eq!(a["results"][0]["rows"], b["results"][0]["rows"], "{k}");
        assert_eq!(a["results"][0]["row_count"], b["results"][0]["row_count"], "{k}");
    }
    // model columns of a grouping, and a measure that is not a result column
    for k in [
        "Sales[Channel] DESC, Product[Category]",
        "[Units] DESC, Product[Category]",
    ] {
        let q = "EVALUATE SUMMARIZECOLUMNS(Product[Category], Sales[Channel], \"T\", [Total]) ORDER BY ";
        let a = eval(&mut s, &format!("{q}{k}"));
        let b = eval(&mut s, &format!("{q}{}", slow(k)));
        assert_eq!(a["results"][0]["rows"], b["results"][0]["rows"], "{k}");
        assert!(a["results"][0]["row_count"].as_u64().unwrap() > 1);
    }
}

/// The result object keeps its key order (clients see the serialized text): `results`,
/// `data_source`, `elapsed_ms`; per statement `columns`, `rows`, `row_count`, `truncated`; each
/// row's keys in column order.
#[test]
fn result_keys_keep_their_order() {
    let mut s = literal_session();
    let v = eval(
        &mut s,
        "EVALUATE {(2, \"b\", TRUE), (1, \"a\", FALSE)} ORDER BY [Value1] EVALUATE ROW(\"x\", 1)",
    );
    let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(keys(&v), ["results", "data_source", "elapsed_ms"]);
    for r in v["results"].as_array().unwrap() {
        assert_eq!(keys(r), ["columns", "rows", "row_count", "truncated"]);
    }
    let text = serde_json::to_string(&v["results"]).unwrap();
    assert_eq!(
        text,
        "[{\"columns\":[{\"name\":\"[Value1]\",\"type\":\"int64\"},{\"name\":\"[Value2]\",\"type\":\"string\"},\
         {\"name\":\"[Value3]\",\"type\":\"boolean\"}],\"rows\":[{\"[Value1]\":1,\"[Value2]\":\"a\",\"[Value3]\":false},\
         {\"[Value1]\":2,\"[Value2]\":\"b\",\"[Value3]\":true}],\"row_count\":2,\"truncated\":false},\
         {\"columns\":[{\"name\":\"[x]\",\"type\":\"int64\"}],\"rows\":[{\"[x]\":1}],\"row_count\":1,\"truncated\":false}]"
    );
}

/// The cancel flag (`Session::set_cancel`, raised by the MCP HTTP transport when the client
/// closes the response stream) stops an evaluation where its time budget is checked.
#[test]
fn a_raised_cancel_flag_stops_the_evaluation() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};
    let mut s = literal_session();
    let small = "EVALUATE ROW(\"x\", SUMX(GENERATESERIES(1, 1000), [Value]))";
    let flag = Arc::new(AtomicBool::new(true));
    s.set_cancel(Some(Arc::clone(&flag)));
    assert!(s.is_cancelled());
    let e = err(&mut s, small);
    assert!(e.contains(bicraft_engine::CANCELLED), "{e}");
    flag.store(false, Ordering::SeqCst);
    assert_eq!(eval(&mut s, small)["results"][0]["rows"][0]["[x]"], 500500.0);
    // raised from another thread while a long evaluation runs
    let long =
        "EVALUATE ROW(\"x\", SUMX(GENERATESERIES(1, 100000), SUMX(GENERATESERIES(1, 100000), [Value])))";
    let raiser = {
        let flag = Arc::clone(&flag);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            flag.store(true, Ordering::SeqCst);
        })
    };
    let started = Instant::now();
    let e = s
        .execute("dax.evaluate", &json!({"query": long, "timeout_ms": 600_000}))
        .unwrap_err();
    raiser.join().unwrap();
    assert!(e.contains(bicraft_engine::CANCELLED), "{e}");
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "{:?}",
        started.elapsed()
    );
    s.set_cancel(None);
    assert!(!s.is_cancelled());
}
