//! The Performance analyzer (`perf.*`): a recording holds one event per visual evaluated, with
//! BIcraft's own times (structure only: never an exact duration), its DAX query text, and the
//! render time handed back by the app.

use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Sales(Product, Amount) with a Total measure; page `Main` with a table of Product and Total,
/// a card of Sum(Amount) and an empty card (no field).
fn shop() -> (Session, String, String) {
    let mut s = Session::new();
    let sales = r#"#table({"Product", "Amount"}, {{"Bike", 100}, {"Bike", 50}, {"Car", 1000}, {"Ball", 5}})"#;
    let steps = [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
    ];
    for (id, p) in steps {
        run(&mut s, id, p);
    }
    let first = s.report.as_ref().unwrap().pages[0].name.clone();
    run(
        &mut s,
        "page.rename",
        json!({"page": first, "display_name": "Main"}),
    );
    let add = |s: &mut Session, t: &str, fields: Value| {
        let r = run(
            s,
            "visual.add",
            json!({"page": "Main", "visual_type": t, "x": 0, "y": 0, "width": 300, "height": 200, "fields": fields}),
        );
        r["visual"].as_str().unwrap().to_string()
    };
    let table = add(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "[Total]"}]),
    );
    let card = add(
        &mut s,
        "card",
        json!([{"role": "Values", "field": "Sum(Sales[Amount])"}]),
    );
    let _empty = add(&mut s, "card", json!([]));
    (s, table, card)
}

fn page_name(s: &Session) -> String {
    s.report.as_ref().unwrap().pages[0].name.clone()
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

#[test]
fn refresh_visuals_times_every_visual_with_fields_and_writes_its_query() {
    let (mut s, table, card) = shop();
    let page = page_name(&s);
    let rev = s.revision;
    run(&mut s, "perf.start", json!({}));
    let out = run(&mut s, "perf.refresh", json!({}));
    assert_eq!(out["page"], page);
    let visuals = out["visuals"].as_array().unwrap();
    // the card without a field is not evaluated, as in the viewer
    let names: Vec<&str> = visuals.iter().map(|v| v["visual"].as_str().unwrap()).collect();
    assert_eq!(names, vec![table.as_str(), card.as_str()]);
    for v in visuals {
        assert_eq!(v["cause"], "refresh");
        assert_eq!(v["status"], "ok", "{v}");
        assert!(num(&v["evaluation_ms"]) >= 0.0);
        assert_eq!(v["render_ms"], Value::Null);
        assert_eq!(num(&v["total_ms"]), num(&v["evaluation_ms"]));
        assert!(num(&v["at_ms"]) >= 0.0);
    }
    assert_eq!(visuals[0]["rows"], 3);
    let q = visuals[0]["query"].as_str().unwrap();
    assert!(q.starts_with("EVALUATE\n\tSUMMARIZECOLUMNS("), "{q}");
    assert!(
        q.contains("'Sales'[Product]") && q.contains("\"Total\", [Total]"),
        "{q}"
    );
    let q = visuals[1]["query"].as_str().unwrap();
    assert!(
        q.contains("\"Sum of Amount\", CALCULATE(SUM('Sales'[Amount]))"),
        "{q}"
    );
    // events are ordered, and the recording is no edit
    let res = run(&mut s, "perf.results", json!({}));
    let events = res["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert!(events[0]["seq"].as_u64() < events[1]["seq"].as_u64());
    assert!(num(&events[0]["at_ms"]) <= num(&events[1]["at_ms"]));
    assert_eq!(res["recording"], true);
    assert_eq!(res["measured_by"], "BIcraft");
    assert_eq!(s.revision, rev, "perf commands bump no revision");
}

#[test]
fn the_query_text_runs_in_bicrafts_dax_evaluator_with_the_visuals_filters() {
    let (mut s, table, _) = shop();
    let page = page_name(&s);
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": table, "field": "Sales[Product]", "in": ["Bike", "Car"]}),
    );
    run(
        &mut s,
        "page.filter",
        json!({"page": page, "field": "Sales[Amount]", "op": ">", "value": 10}),
    );
    run(&mut s, "perf.start", json!({}));
    let out = run(&mut s, "perf.refresh", json!({"visual": table}));
    let v = &out["visuals"][0];
    assert_eq!(out["visuals"].as_array().unwrap().len(), 1);
    let q = v["query"].as_str().unwrap().to_string();
    assert!(q.starts_with("DEFINE\n"), "{q}");
    assert!(
        q.contains("TREATAS({\"Bike\", \"Car\"}, 'Sales'[Product])"),
        "{q}"
    );
    assert!(
        q.contains("FILTER(ALL('Sales'[Amount]), 'Sales'[Amount] > 10)"),
        "{q}"
    );
    assert_eq!(v["query_notes"], json!([]));
    // the text is a query BIcraft's own evaluator accepts, and gives the visual's rows
    run(&mut s, "dax.validate_query", json!({"query": q}));
    let r = run(&mut s, "dax.evaluate", json!({"query": q}));
    let rows = r["results"][0]["rows"].as_array().unwrap().len() as u64;
    assert_eq!(Some(rows), v["rows"].as_u64(), "{r}");
    assert_eq!(rows, 2, "Bike and Car");
}

#[test]
fn refresh_needs_a_recording_and_known_names() {
    let (mut s, _, _) = shop();
    let e = s.execute("perf.refresh", &json!({})).unwrap_err();
    assert!(e.contains("perf.start"), "{e}");
    run(&mut s, "perf.start", json!({}));
    assert!(s.execute("perf.refresh", &json!({"page": "Nope"})).is_err());
    assert!(s.execute("perf.refresh", &json!({"visual": "nope"})).is_err());
    assert!(s.execute("perf.refresh", &json!({"page": 3})).is_err());
    assert!(s.execute("perf.refresh", &json!({"other": 1})).is_err());
    // a page found by its display name
    let out = run(&mut s, "perf.refresh", json!({"page": "Main"}));
    assert_eq!(out["visuals"].as_array().unwrap().len(), 2);
    let mut empty = Session::new();
    run(&mut empty, "perf.start", json!({}));
    assert!(empty.execute("perf.refresh", &json!({})).is_err());
}

#[test]
fn a_recording_keeps_what_the_viewer_computes_until_it_stops() {
    let (mut s, table, _) = shop();
    let page = page_name(&s);
    run(&mut s, "perf.start", json!({}));
    let _ = s.snapshot();
    let res = run(&mut s, "perf.results", json!({}));
    let events = res["events"].as_array().unwrap();
    assert_eq!(events.len(), 2, "{res}");
    assert!(
        events
            .iter()
            .all(|e| e["cause"] == "view" && e["page"] == page.as_str())
    );
    // answered from the cache: nothing more
    let _ = s.snapshot();
    assert_eq!(
        run(&mut s, "perf.results", json!({}))["events"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // stopped: a new evaluation is not recorded
    run(&mut s, "perf.stop", json!({}));
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": table, "field": "Sales[Product]", "in": ["Bike"]}),
    );
    let _ = s.snapshot();
    let res = run(&mut s, "perf.results", json!({}));
    assert_eq!(res["recording"], false);
    assert_eq!(res["events"].as_array().unwrap().len(), 2);
    // stopped, refresh is refused
    assert!(s.execute("perf.refresh", &json!({})).is_err());
}

#[test]
fn render_times_complete_the_latest_events_once() {
    let (mut s, table, card) = shop();
    let page = page_name(&s);
    run(&mut s, "perf.start", json!({}));
    run(&mut s, "perf.refresh", json!({}));
    let out = run(
        &mut s,
        "perf.render",
        json!({"page": page, "times": {table.clone(): 2.5, "unknown": 1.0}}),
    );
    assert_eq!(out["updated"], 1);
    // a second time for the same event is not taken
    let out = run(
        &mut s,
        "perf.render",
        json!({"page": page, "times": {table.clone(): 9.0}}),
    );
    assert_eq!(out["updated"], 0);
    let res = run(&mut s, "perf.results", json!({}));
    let e = res["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["visual"] == table.as_str())
        .unwrap()
        .clone();
    assert_eq!(e["render_ms"], 2.5);
    assert!((num(&e["total_ms"]) - (num(&e["evaluation_ms"]) + 2.5)).abs() < 0.002);
    let c = res["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["visual"] == card.as_str())
        .unwrap()
        .clone();
    assert_eq!(c["render_ms"], Value::Null);
    // hostile times
    for bad in [
        json!({"page": page}),
        json!({"page": page, "times": [1]}),
        json!({"page": page, "times": {"a": -1}}),
        json!({"page": page, "times": {"a": "x"}}),
        json!({"page": page, "times": {"a": 1e300}}),
        json!({"times": {}}),
    ] {
        assert!(s.execute("perf.render", &bad).is_err(), "{bad}");
    }
}

#[test]
fn clear_empties_and_export_writes_the_results() {
    let (mut s, _, _) = shop();
    run(&mut s, "perf.start", json!({}));
    run(&mut s, "perf.refresh", json!({}));
    let dir = std::env::temp_dir().join(format!("bicraft-perf-{}", std::process::id()));
    let out = dir.join("sub").join("perf.json");
    let r = run(&mut s, "perf.export", json!({"out": out.display().to_string()}));
    assert_eq!(r["events"], 2);
    let doc: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(doc["events"].as_array().unwrap().len(), 2);
    assert_eq!(doc["clock"], "monotonic");
    assert!(s.execute("perf.export", &json!({})).is_err());
    assert!(s.execute("perf.export", &json!({"out": " "})).is_err());
    assert!(
        s.execute("perf.export", &json!({"out": dir.display().to_string()}))
            .is_err()
    );
    let r = run(&mut s, "perf.clear", json!({}));
    assert_eq!(r["cleared"], 2);
    let res = run(&mut s, "perf.results", json!({}));
    assert_eq!(res["events"], json!([]));
    assert_eq!(res["recording"], true);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Sales(Product, Amount, Day) with a Total measure, a role `NoCars`, and page `Main` with a
/// table of Product and Total, a column chart of the same, and a slicer on Product; the session's
/// clock is 2024-06-15.
fn dated_shop() -> (Session, String, String, String) {
    let mut s = Session::new();
    s.as_of = Some(45_458.5);
    let sales = r#"#table(type table [Product = text, Amount = number, Day = date],
        {{"Bike", 100, #date(2024, 6, 10)}, {"Bike", 50, #date(2024, 1, 5)}, {"Car", 1000, #date(2024, 6, 1)},
         {"Ball", 5, #date(2024, 6, 14)}, {"Kite", 30, #date(2023, 3, 1)}})"#;
    let steps = [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Day", "data_type": "dateTime"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("role.add", json!({"name": "NoCars"})),
        (
            "role.set_filter",
            json!({"role": "NoCars", "table": "Sales", "filter_expression": "Sales[Product] <> \"Car\""}),
        ),
        ("report.new", json!({})),
    ];
    for (id, p) in steps {
        run(&mut s, id, p);
    }
    let first = s.report.as_ref().unwrap().pages[0].name.clone();
    run(
        &mut s,
        "page.rename",
        json!({"page": first, "display_name": "Main"}),
    );
    let add = |s: &mut Session, t: &str, fields: Value| {
        let r = run(
            s,
            "visual.add",
            json!({"page": "Main", "visual_type": t, "x": 0, "y": 0, "width": 300, "height": 200, "fields": fields}),
        );
        r["visual"].as_str().unwrap().to_string()
    };
    let by = json!([{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "[Total]"}]);
    let table = add(&mut s, "tableEx", by);
    let chart = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Product]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let slicer = add(
        &mut s,
        "slicer",
        json!([{"role": "Values", "field": "Sales[Product]"}]),
    );
    (s, table, chart, slicer)
}

/// A row as comparable text: numbers as numbers, whatever their JSON form.
fn row_key(cells: Vec<&Value>) -> String {
    cells
        .iter()
        .map(|c| match c.as_f64() {
            Some(x) => format!("{x}"),
            None => c.to_string(),
        })
        .collect::<Vec<_>>()
        .join("|")
}

/// The rows the viewer shows for `visual`, and those its copied query returns in BIcraft's DAX
/// evaluator, each sorted; with the query's notes (what it leaves out).
fn shown_and_copied(s: &mut Session, visual: &str) -> (Vec<String>, Vec<String>, Value) {
    let snap = s.snapshot();
    let v = snap["report"]["pages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|p| p["visuals"].as_array().unwrap().iter())
        .find(|v| v["name"] == visual)
        .unwrap()
        .clone();
    assert_eq!(v["data"]["status"], "ok", "{}", v["data"]);
    let mut shown: Vec<String> = v["data"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| row_key(r.as_array().unwrap().iter().collect()))
        .collect();
    shown.sort();
    run(s, "perf.clear", json!({}));
    run(s, "perf.start", json!({}));
    let out = run(s, "perf.refresh", json!({"visual": visual}));
    let ev = &out["visuals"][0];
    let q = ev["query"]
        .as_str()
        .unwrap_or_else(|| panic!("no query: {ev}"))
        .to_string();
    let r = s
        .execute("dax.evaluate", &json!({"query": q}))
        .unwrap_or_else(|e| panic!("{e}\n{q}"));
    let mut copied: Vec<String> = r["results"][0]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| row_key(r.as_object().unwrap().values().collect()))
        .collect();
    copied.sort();
    (shown, copied, ev["query_notes"].clone())
}

type Setup = fn(&mut Session, &str, &str, &str, &str);

#[test]
fn the_copied_query_returns_the_visuals_rows_under_every_filter() {
    let cases: [(&str, Setup); 8] = [
        ("measure filter", |s, page, table, _, _| {
            run(
                s,
                "visual.filter",
                json!({"page": page, "visual": table, "field": "[Total]", "op": ">", "value": 40}),
            );
        }),
        ("aggregation filter", |s, page, table, _, _| {
            run(
                s,
                "visual.filter",
                json!({"page": page, "visual": table, "field": "Sum(Sales[Amount])", "op": "<", "value": 200}),
            );
        }),
        ("top N", |s, page, table, _, _| {
            run(
                s,
                "visual.filter",
                json!({"page": page, "visual": table, "field": "Sales[Product]", "top": 2, "by": "[Total]"}),
            );
        }),
        ("bottom N", |s, page, table, _, _| {
            run(
                s,
                "visual.filter",
                json!({"page": page, "visual": table, "field": "Sales[Product]", "top": 1, "by": "Sum(Sales[Amount])", "direction": "bottom"}),
            );
        }),
        ("slicer selection", |s, _, _, _, slicer| {
            run(
                s,
                "view.interact",
                json!({"action": "slicer_set", "page": "Main", "visual": slicer, "values": ["Bike", "Kite"]}),
            );
        }),
        ("relative date page filter", |s, page, _, _, _| {
            run(
                s,
                "page.filter",
                json!({"page": page, "field": "Sales[Day]", "relative": {"range": "last", "count": 30, "unit": "days", "include_today": true}}),
            );
        }),
        ("cross-filter and View as", |s, _, _, chart, _| {
            run(
                s,
                "view.interact",
                json!({"action": "view_as", "roles": ["NoCars"]}),
            );
            run(
                s,
                "view.interact",
                json!({"action": "select", "page": "Main", "visual": chart, "point": [{"c": 0, "v": "Bike"}]}),
            );
            run(
                s,
                "view.interact",
                json!({"action": "select", "page": "Main", "visual": chart, "point": [{"c": 0, "v": "Ball"}], "ctrl": true}),
            );
        }),
        (
            "report, page and visual filters together",
            |s, page, table, _, _| {
                run(
                    s,
                    "report.filter",
                    json!({"field": "Sales[Product]", "not_in": ["Kite"]}),
                );
                run(
                    s,
                    "page.filter",
                    json!({"page": page, "field": "Sales[Day]", "relative": {"range": "last", "count": 30, "unit": "days", "include_today": true}}),
                );
                run(
                    s,
                    "visual.filter",
                    json!({"page": page, "visual": table, "field": "Sales[Product]", "top": 2, "by": "[Total]"}),
                );
                run(
                    s,
                    "visual.filter",
                    json!({"page": page, "visual": table, "field": "[Total]", "op": ">", "value": 120}),
                );
            },
        ),
    ];
    let mut failed = Vec::new();
    for (what, setup) in cases {
        let (mut s, table, chart, slicer) = dated_shop();
        let page = page_name(&s);
        setup(&mut s, &page, &table, &chart, &slicer);
        let (shown, copied, notes) = shown_and_copied(&mut s, &table);
        assert!(!shown.is_empty(), "{what}: the visual shows rows");
        if copied != shown || notes != json!([]) {
            failed.push(format!(
                "{what}: shown {shown:?}, copied {copied:?}, notes {notes}"
            ));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

#[test]
fn the_snapshot_names_the_viewed_visuals_still_without_a_render_time() {
    let (mut s, table, card) = shop();
    let page = page_name(&s);
    run(&mut s, "perf.start", json!({}));
    // the viewer evaluates the page: both visuals wait for their drawing's time
    let snap = s.snapshot();
    let mut waiting: Vec<&str> = snap["perf"]["unrendered"][&page]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    waiting.sort();
    let mut both = vec![table.as_str(), card.as_str()];
    both.sort();
    assert_eq!(waiting, both, "{}", snap["perf"]);
    run(
        &mut s,
        "perf.render",
        json!({"page": page, "times": {table.clone(): 1.5}}),
    );
    let snap = s.snapshot();
    assert_eq!(snap["perf"]["unrendered"][&page], json!([card]));
    run(
        &mut s,
        "perf.render",
        json!({"page": page, "times": {card.clone(): 0.5}}),
    );
    assert_eq!(s.snapshot()["perf"]["unrendered"], json!({}));
    // a new evaluation of the table waits again
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": table, "field": "Sales[Product]", "in": ["Bike"]}),
    );
    assert_eq!(s.snapshot()["perf"]["unrendered"][&page], json!([table]));
    run(&mut s, "perf.stop", json!({}));
    assert!(s.snapshot().get("perf").is_none());
}
