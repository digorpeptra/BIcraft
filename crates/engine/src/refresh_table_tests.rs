//! `table.add_refresh_info` and the INFO functions: the optional Refresh info table (added,
//! saved to PBIP/TMDL and PBIX, refreshed by `model.refresh`, drawn on a card), and `INFO.*`
//! results with stored data (a PBIX DataModel) and without (blank times), with the engine's
//! restriction of the query-only INFO functions.

use serde_json::{Value, json};

use super::*;
use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn eval(s: &mut Session, query: &str) -> Value {
    run(s, "dax.evaluate", json!({ "query": query }))
}

/// The only value of a one-cell query.
fn scalar(s: &mut Session, expr: &str) -> Value {
    let v = eval(s, &format!("EVALUATE ROW(\"x\", {expr})"));
    v["results"][0]["rows"][0]["[x]"].clone()
}

/// A number of a result, integer or not.
fn n(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn texts(r: &Value) -> Vec<String> {
    r["pages"][0]["texts"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|t| t["t"].as_str().map(str::to_string))
        .collect()
}

/// A new model with one import table `A` (two rows) and a measure.
fn small_model() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "R"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "A", "m": "let Source = #table(type table [K = Int64.Type], {{1}, {2}}) in Source"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "A", "name": "N", "expression": "COUNTROWS(A)"}),
    );
    s
}

#[test]
fn adds_an_optional_refresh_table() {
    let mut s = small_model();
    // off by default: a new model has no such table
    assert!(s.model.as_ref().unwrap().table(DEFAULT_TABLE).is_none());
    let r = run(&mut s, "table.add_refresh_info", json!({}));
    assert_eq!(r["table"], DEFAULT_TABLE);
    assert_eq!(r["measure"]["name"], DEFAULT_MEASURE);
    assert_eq!(
        r["measure"]["expression"],
        "MAX('Refresh info'[Last refresh (UTC)])"
    );
    let m = s.model.as_ref().unwrap();
    let t = m.table(DEFAULT_TABLE).unwrap();
    assert_eq!(t.partitions.len(), 1);
    assert_eq!(t.partitions[0].source_type, "m");
    let expr = t.partitions[0].expression.as_deref().unwrap();
    assert!(
        expr.contains("DateTimeZone.UtcNow()") && expr.contains("DateTime.LocalNow()"),
        "{expr}"
    );
    let cols: Vec<(&str, bicraft_model::DataType)> =
        t.columns.iter().map(|c| (c.name.as_str(), c.data_type)).collect();
    assert_eq!(
        cols,
        [
            (UTC_COLUMN, bicraft_model::DataType::DateTime),
            (LOCAL_COLUMN, bicraft_model::DataType::DateTime)
        ]
    );
    let me = t.measure(DEFAULT_MEASURE).unwrap();
    assert_eq!(me.format_string.as_deref(), Some("General Date"));
    assert!(!t.is_hidden);

    // the M query runs in bicraft too: the measure is a time, not blank
    let v = scalar(&mut s, "[Last refresh]");
    assert!(v.as_str().is_some_and(|t| t.starts_with("20")), "{v}");
    let local = scalar(&mut s, "MAX('Refresh info'[Last refresh (local)])");
    assert!(local.is_string(), "{local}");

    // a second one is refused, and so is a measure name already taken (nothing added)
    let e = s.execute("table.add_refresh_info", &json!({})).unwrap_err();
    assert!(e.contains("already has a table"), "{e}");
    let e = s
        .execute("table.add_refresh_info", &json!({"name": "Other"}))
        .unwrap_err();
    assert!(e.contains("already has a measure"), "{e}");
    assert!(s.model.as_ref().unwrap().table("Other").is_none());
    assert!(
        s.execute("table.add_refresh_info", &json!({"name": " "}))
            .is_err()
    );
    assert!(s.execute("table.add_refresh_info", &json!({"bogus": 1})).is_err());

    // another name (quoted in DAX), hidden, without the measure
    let r = run(
        &mut s,
        "table.add_refresh_info",
        json!({"name": "Kim's refresh", "measure": false, "hidden": true}),
    );
    assert!(r.get("measure").is_none());
    let t = s.model.as_ref().unwrap().table("Kim's refresh").unwrap();
    assert!(t.is_hidden && t.measures.is_empty());
    let r = run(
        &mut s,
        "table.add_refresh_info",
        json!({"name": "Third", "measure_name": "Data as of"}),
    );
    assert_eq!(r["measure"]["expression"], "MAX('Third'[Last refresh (UTC)])");
    let v = scalar(&mut s, "[Data as of]");
    assert!(v.is_string(), "{v}");
    // one undo step removes the table and its measure
    run(&mut s, "edit.undo", json!({}));
    assert!(s.model.as_ref().unwrap().table("Third").is_none());
}

#[test]
fn refresh_table_round_trips_through_pbip_and_pbix() {
    let dir = TempDir::new("refresh-table-rt");
    let mut s = small_model();
    run(&mut s, "table.add_refresh_info", json!({}));
    let before = s.model.as_ref().unwrap().table(DEFAULT_TABLE).unwrap().clone();

    // PBIP (TMDL)
    run(&mut s, "project.new", json!({"dir": dir.path(), "name": "P"}));
    let tmdl = std::fs::read_to_string(
        dir.join("P.SemanticModel")
            .join("definition")
            .join("tables")
            .join("Refresh info.tmdl"),
    )
    .unwrap();
    assert!(tmdl.contains("DateTimeZone.UtcNow()"), "{tmdl}");
    assert!(tmdl.contains("measure 'Last refresh'"), "{tmdl}");
    let mut p = Session::new();
    run(&mut p, "file.open", json!({"path": dir.join("P.pbip")}));
    let after = p.model.as_ref().unwrap().table(DEFAULT_TABLE).unwrap();
    assert_eq!(after.partitions[0].expression, before.partitions[0].expression);
    assert_eq!(after.measures[0].expression, before.measures[0].expression);
    assert_eq!(after.measures[0].format_string.as_deref(), Some("General Date"));
    let cols: Vec<_> = after.columns.iter().map(|c| (&c.name, c.data_type)).collect();
    let want: Vec<_> = before.columns.iter().map(|c| (&c.name, c.data_type)).collect();
    assert_eq!(cols, want);

    // PBIX: the row is stored in the DataModel with the partition's refresh time
    let pbix = dir.join("r.pbix");
    run(&mut s, "file.save_as", json!({"path": pbix}));
    let mut x = Session::new();
    run(&mut x, "file.open", json!({"path": pbix}));
    let after = x.model.as_ref().unwrap().table(DEFAULT_TABLE).unwrap();
    assert_eq!(after.partitions[0].expression, before.partitions[0].expression);
    assert_eq!(after.measures[0].expression, before.measures[0].expression);
    let q = eval(
        &mut x,
        "EVALUATE ROW(\"v\", [Last refresh], \"r\", MAXX(FILTER(INFO.PARTITIONS(), [Name] = \"Refresh info\"), [RefreshedTime]))",
    );
    assert_eq!(q["data_source"], "DataModel", "{q}");
    let row = &q["results"][0]["rows"][0];
    let (v, r) = (row["[v]"].as_str().unwrap(), row["[r]"].as_str().unwrap());
    // the query ran at import, the partition was stored at save: the same minute or so
    assert!(v.get(..10) == r.get(..10) || v <= r, "{v} {r}");
}

#[test]
fn model_refresh_updates_the_last_refresh() {
    let dir = TempDir::new("refresh-table-refresh");
    let mut s = small_model();
    run(&mut s, "table.add_refresh_info", json!({}));
    run(&mut s, "project.new", json!({"dir": dir.path(), "name": "P"}));
    let first = run(&mut s, "model.refresh", json!({}));
    let v1 = scalar(&mut s, "[Last refresh]");
    let at1 = first["refreshed_at"].as_str().unwrap().trim_end_matches('Z');
    // the refresh time to the second (refreshed_at drops the milliseconds)
    let sec = |v: &Value| v.as_str().and_then(|t| t.get(..19)).map(str::to_string);
    assert_eq!(sec(&v1).as_deref(), Some(at1), "UtcNow is the refresh time");
    // INFO.PARTITIONS reads model.refresh's own time
    let r1 = scalar(&mut s, "MAXX(INFO.PARTITIONS(), [RefreshedTime])");
    assert_eq!(sec(&r1).as_deref(), Some(at1));
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let second = run(&mut s, "model.refresh", json!({}));
    let v2 = scalar(&mut s, "[Last refresh]");
    let at2 = second["refreshed_at"].as_str().unwrap().trim_end_matches('Z');
    assert_eq!(sec(&v2).as_deref(), Some(at2));
    assert!(at2 > at1, "{at1} {at2}");
}

#[test]
fn card_shows_the_last_refresh() {
    let mut s = small_model();
    run(&mut s, "table.add_refresh_info", json!({}));
    run(&mut s, "report.new", json!({}));
    run(&mut s, "page.add", json!({"display_name": "Main"}));
    let r = run(
        &mut s,
        "visual.add",
        json!({"page": "Main", "visual_type": "card", "x": 0, "y": 0, "width": 300, "height": 120,
               "fields": [{"role": "Values", "field": "[Last refresh]"}]}),
    );
    assert!(r["visual"].is_string(), "{r}");
    let value = scalar(&mut s, "[Last refresh]");
    let iso = value.as_str().unwrap().to_string();
    let (y, mo, d) = (&iso[..4], &iso[5..7], &iso[8..10]);
    let strip = |t: &str| t.trim_start_matches('0').to_string();
    // en-US General Date: month/day/year and the time
    let r = run(
        &mut s,
        "report.render",
        json!({"page": "Main", "system_fonts": false, "text_dump": true, "format_culture": "en-US"}),
    );
    let en = format!("{}/{}/{y}", strip(mo), strip(d));
    assert!(
        texts(&r).iter().any(|t| t.contains(&en) && t.contains(':')),
        "{en}: {:?}",
        texts(&r)
    );
    assert_eq!(r["pages"][0]["visuals"][0]["data"], "ok", "{r}");
    // fr-FR: day/month/year
    let r = run(
        &mut s,
        "report.render",
        json!({"page": "Main", "system_fonts": false, "text_dump": true, "format_culture": "fr-FR"}),
    );
    let fr = format!("{d}/{mo}/{y}");
    assert!(texts(&r).iter().any(|t| t.contains(&fr)), "{fr}: {:?}", texts(&r));
}

#[test]
fn info_functions_without_stored_data() {
    let mut s = small_model();
    let q = eval(&mut s, "EVALUATE INFO.PARTITIONS()");
    let cols: Vec<&str> = q["results"][0]["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(cols.len(), 23);
    assert_eq!(cols[0], "[ID]");
    assert!(cols.contains(&"[RefreshedTime]") && cols.contains(&"[ModifiedTime]"));
    let types: Vec<&str> = q["results"][0]["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["type"].as_str().unwrap())
        .collect();
    assert_eq!(types[0], "int64");
    assert_eq!(types[12], "dateTime", "{types:?}");
    let row = &q["results"][0]["rows"][0];
    assert_eq!(row["[Name]"], "A");
    assert_eq!(n(&row["[TableID]"]), 1.0);
    assert_eq!(n(&row["[Type]"]), 4.0, "an M partition");
    assert_eq!(row["[RefreshedTime]"], Value::Null, "no stored data: blank");
    assert_eq!(
        scalar(&mut s, "MAXX(INFO.PARTITIONS(), [RefreshedTime])"),
        Value::Null
    );
    assert_eq!(scalar(&mut s, "MAXX(INFO.TABLES(), [ModifiedTime])"), Value::Null);
    assert_eq!(n(&scalar(&mut s, "COUNTROWS(INFO.MODEL())")), 1.0);
    assert_eq!(scalar(&mut s, "MAXX(INFO.MODEL(), [Name])"), "R");
    assert_eq!(n(&scalar(&mut s, "COUNTROWS(INFO.COLUMNS())")), 1.0);
    assert_eq!(
        n(&scalar(&mut s, "MAXX(INFO.COLUMNS(), [ExplicitDataType])")),
        6.0
    );
    assert_eq!(
        scalar(&mut s, "MAXX(INFO.MEASURES(), [Expression])"),
        "COUNTROWS(A)"
    );
    assert_eq!(
        scalar(&mut s, "MAXX(INFO.VIEW.COLUMNS(), [DataType])"),
        "Whole Number"
    );
    assert_eq!(
        scalar(&mut s, "MAXX(INFO.VIEW.TABLES(), [StorageMode])"),
        "Import"
    );
    assert_eq!(
        scalar(&mut s, "COUNTROWS(INFO.VIEW.RELATIONSHIPS())"),
        Value::Null
    );
    // restrictions filter on a column, by name
    assert_eq!(n(&scalar(&mut s, "COUNTROWS(INFO.TABLES(\"Name\", \"a\"))")), 1.0);
    assert_eq!(
        scalar(&mut s, "COUNTROWS(INFO.TABLES(\"Name\", \"zz\"))"),
        Value::Null
    );
    assert_eq!(
        n(&scalar(&mut s, "COUNTROWS(INFO.PARTITIONS(\"TableID\", 1))")),
        1.0
    );
    for bad in [
        "EVALUATE INFO.TABLES(\"Bogus\", 1)",
        "EVALUATE INFO.TABLES(\"Name\")",
        "EVALUATE INFO.VIEW.TABLES(\"Name\", \"A\")",
        "EVALUATE INFO.CALCDEPENDENCY()",
    ] {
        assert!(
            s.execute("dax.evaluate", &json!({"query": bad})).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn info_functions_read_the_stored_refresh_state() {
    let dir = TempDir::new("refresh-table-info");
    let mut s = small_model();
    run(&mut s, "file.save_as", json!({"path": dir.join("r.pbix")}));
    let stats = run(&mut s, "model.storage_stats", json!({"columns": false}));
    let stored = stats["tables"][0]["partitions"][0]["refreshed_time"]
        .as_str()
        .unwrap()
        .to_string();
    let r = scalar(&mut s, "MAXX(INFO.PARTITIONS(), [RefreshedTime])");
    let r = r.as_str().unwrap();
    // same second (the ISO form of the storage stats may add milliseconds)
    assert_eq!(r.get(..19), stored.get(..19), "{r} {stored}");
    assert_eq!(
        n(&scalar(&mut s, "MAXX(INFO.PARTITIONS(), [State])")),
        1.0,
        "Ready"
    );
    assert!(scalar(&mut s, "MAXX(INFO.TABLES(), [ModifiedTime])").is_string());
    assert!(scalar(&mut s, "MAXX(INFO.TABLES(), [StructureModifiedTime])").is_string());
    assert!(scalar(&mut s, "MAXX(INFO.MODEL(), [ModifiedTime])").is_string());
    assert!(scalar(&mut s, "MAXX(INFO.COLUMNS(), [RefreshedTime])").is_string());
}

#[test]
fn query_only_info_functions_are_refused_outside_queries() {
    let mut s = small_model();
    // the engine's rule: INFO.PARTITIONS is for DAX queries only
    let e = s
        .execute(
            "measure.add",
            &json!({"table": "A", "name": "Bad", "expression": "MAXX(INFO.PARTITIONS(), [RefreshedTime])"}),
        )
        .unwrap_err();
    assert!(
        e.contains("info-query-only") || e.contains("only be used in a DAX query"),
        "{e}"
    );
    let e = s
        .execute(
            "table.add",
            &json!({"name": "T", "partition": {"source_type": "calculated", "expression": "INFO.TABLES()"}}),
        )
        .unwrap_err();
    assert!(e.contains("DAX query"), "{e}");
    // INFO.VIEW.* is allowed in measures and calculated tables
    run(
        &mut s,
        "measure.add",
        json!({"table": "A", "name": "Measures", "expression": "COUNTROWS(INFO.VIEW.MEASURES())"}),
    );
    assert_eq!(n(&scalar(&mut s, "[Measures]")), 2.0);
    run(
        &mut s,
        "table.add",
        json!({"name": "Doc", "partition": {"source_type": "calculated", "expression":
            "SELECTCOLUMNS(INFO.VIEW.TABLES(), \"Table\", [Name])"}}),
    );
    // a measure forced into the model fails when evaluated, as in the engine; the same text
    // defined by a query runs
    run(
        &mut s,
        "measure.add",
        json!({"table": "A", "name": "Forced", "expression": "MAXX(INFO.TABLES(), [Name])", "force": true}),
    );
    let e = s
        .execute("dax.evaluate", &json!({"query": "EVALUATE ROW(\"x\", [Forced])"}))
        .unwrap_err();
    assert!(e.contains("only be used in a DAX query"), "{e}");
    let q = eval(
        &mut s,
        "DEFINE MEASURE A[Q] = MAXX(INFO.TABLES(), [Name]) EVALUATE ROW(\"x\", [Q])",
    );
    assert_eq!(q["results"][0]["rows"][0]["[x]"], "Doc");
}
