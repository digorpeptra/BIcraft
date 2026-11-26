//! Incremental refresh: `table.set_refresh_policy` writes a table's `refreshPolicy` when the
//! model has the RangeStart and RangeEnd Date/Time parameters and the table's query filters on
//! them, and says what is missing otherwise; `table.remove_refresh_policy` removes it; the
//! snapshot shows the policy and the readiness; TMDL, TMSL and PBIX keep it.

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

const PARAM: &str = "#datetime(2024, 1, 1, 0, 0, 0) meta [IsParameterQuery=true, Type=\"DateTime\", IsParameterQueryRequired=true]";
const FILTERED: &str = "let Source = Sql.Database(\"srv.example\", \"db\"), Orders = Source{[Schema=\"dbo\",Item=\"Orders\"]}[Data], Filtered = Table.SelectRows(Orders, each [OrderDate] >= RangeStart and [OrderDate] < RangeEnd) in Filtered";
const UNFILTERED: &str =
    "let Source = Sql.Database(\"srv.example\", \"db\"), T = Source{[Schema=\"dbo\",Item=\"T\"]}[Data] in T";

/// Orders (Import, filtered on RangeStart/RangeEnd; OrderDate, Modified, Amount), Plain (Import,
/// not filtered); `params`: whether RangeStart and RangeEnd exist.
fn model(params: bool) -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    if params {
        for n in ["RangeStart", "RangeEnd"] {
            run(&mut s, "expression.add", json!({"name": n, "expression": PARAM}));
        }
    }
    for (table, m) in [("Orders", FILTERED), ("Plain", UNFILTERED)] {
        run(
            &mut s,
            "table.add",
            json!({"name": table, "partition": {"source_type": "m", "expression": m}}),
        );
        for (c, ty) in [
            ("OrderDate", "dateTime"),
            ("Modified", "dateTime"),
            ("Amount", "int64"),
        ] {
            run(
                &mut s,
                "column.add",
                json!({"table": table, "name": c, "data_type": ty}),
            );
        }
    }
    s
}

fn table_snap(s: &Session, table: &str) -> Value {
    s.snapshot()["model"]["tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == table)
        .unwrap()
        .clone()
}

fn policy(s: &Session, table: &str) -> Option<Value> {
    s.model
        .as_ref()
        .unwrap()
        .tables
        .iter()
        .find(|t| t.name == table)
        .unwrap()
        .extra
        .get("refreshPolicy")
        .cloned()
}

#[test]
fn set_writes_the_policy_and_remove_takes_it_away() {
    let mut s = model(true);
    let snap = s.snapshot();
    assert_eq!(snap["model"]["incremental_refresh"]["parameters"], true);
    assert_eq!(table_snap(&s, "Orders")["range_filtered"], true);
    assert_eq!(table_snap(&s, "Plain")["range_filtered"], false);
    let out = run(
        &mut s,
        "table.set_refresh_policy",
        json!({"table": "orders", "store": {"n": 5, "unit": "years"}, "refresh": {"n": 10, "unit": "day"},
               "detect_changes": "Modified", "only_complete_periods": true}),
    );
    assert_eq!(out["table"], "Orders");
    assert!(out["warnings"].to_string().contains("service"), "{out}");
    let p = policy(&s, "Orders").unwrap();
    assert_eq!(p["policyType"], "basic");
    assert_eq!(p["rollingWindowGranularity"], "year");
    assert_eq!(p["rollingWindowPeriods"], 5);
    assert_eq!(p["incrementalGranularity"], "day");
    assert_eq!(p["incrementalPeriods"], 10);
    assert_eq!(p["incrementalPeriodsOffset"], -1);
    assert!(p.get("mode").is_none());
    assert_eq!(p["sourceExpression"], FILTERED);
    assert!(p["pollingExpression"].as_str().unwrap().contains("[Modified]"));
    let t = table_snap(&s, "Orders");
    assert_eq!(
        t["refresh_policy"],
        json!({"store": {"n": 5, "unit": "year"}, "refresh": {"n": 10, "unit": "day"},
               "only_complete_periods": true, "realtime": false, "detect_changes": "Modified",
               "polling_expression": true})
    );
    // the partition is kept as it is
    assert_eq!(t["storage_mode"], "import");
    // real time: a hybrid policy, complete periods on, compatibility level 1565
    run(
        &mut s,
        "table.set_refresh_policy",
        json!({"table": "Orders", "store": {"n": 2, "unit": "quarter"}, "refresh": {"n": 1, "unit": "month"},
               "realtime": true}),
    );
    let p = policy(&s, "Orders").unwrap();
    assert_eq!(p["mode"], "hybrid");
    assert_eq!(p["incrementalPeriodsOffset"], -1);
    assert!(p.get("pollingExpression").is_none());
    assert!(s.model.as_ref().unwrap().compatibility_level.unwrap_or(0) >= 1565);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(policy(&s, "Orders").unwrap()["rollingWindowPeriods"], 5);
    run(&mut s, "table.remove_refresh_policy", json!({"table": "Orders"}));
    assert!(policy(&s, "Orders").is_none());
    assert!(table_snap(&s, "Orders").get("refresh_policy").is_none());
    let e = s
        .execute("table.remove_refresh_policy", &json!({"table": "Orders"}))
        .unwrap_err();
    assert!(e.contains("no incremental refresh policy"), "{e}");
    run(&mut s, "edit.undo", json!({}));
    assert!(policy(&s, "Orders").is_some());
}

#[test]
fn what_is_missing_is_explained() {
    let periods = json!({"n": 1, "unit": "year"});
    // no parameters
    let mut s = model(false);
    assert_eq!(s.snapshot()["model"]["incremental_refresh"]["parameters"], false);
    let e = s
        .execute(
            "table.set_refresh_policy",
            &json!({"table": "Orders", "store": periods, "refresh": periods}),
        )
        .unwrap_err();
    assert!(e.contains("no `RangeStart` parameter"), "{e}");
    assert!(e.contains("no `RangeEnd` parameter"), "{e}");
    assert!(e.contains("expression.add"), "{e}");
    // a wrongly cased parameter
    run(
        &mut s,
        "expression.add",
        json!({"name": "rangestart", "expression": PARAM}),
    );
    run(
        &mut s,
        "expression.add",
        json!({"name": "RangeEnd", "expression": "1 meta [IsParameterQuery=true, Type=\"Number\"]"}),
    );
    let e = s
        .execute(
            "table.set_refresh_policy",
            &json!({"table": "Orders", "store": periods, "refresh": periods}),
        )
        .unwrap_err();
    assert!(e.contains("case-sensitive"), "{e}");
    assert!(e.contains("Date/Time parameter"), "{e}");
    // no filter step; a DirectQuery table; a bad change detection column
    let mut s = model(true);
    for (p, want) in [
        (
            json!({"table": "Plain", "store": periods, "refresh": periods}),
            "no filter step on RangeStart and RangeEnd",
        ),
        (
            json!({"table": "Orders", "store": periods, "refresh": periods, "detect_changes": "Amount"}),
            "not a date/time column",
        ),
        (
            json!({"table": "Orders", "store": periods, "refresh": periods, "detect_changes": "Nope"}),
            "no column `Nope`",
        ),
        (
            json!({"table": "Orders", "store": periods, "refresh": periods, "realtime": true, "only_complete_periods": false}),
            "realtime needs only_complete_periods",
        ),
        (
            json!({"table": "Orders", "refresh": periods}),
            "missing parameter `store`",
        ),
        (
            json!({"table": "Orders", "store": {"n": 0, "unit": "day"}, "refresh": periods}),
            "store.n",
        ),
        (
            json!({"table": "Orders", "store": {"n": 1, "unit": "week"}, "refresh": periods}),
            "unknown unit",
        ),
        (
            json!({"table": "Orders", "store": 3, "refresh": periods}),
            "must be an object",
        ),
        (
            json!({"table": "Orders", "store": periods, "refresh": periods, "x": 1}),
            "unknown parameter",
        ),
        (
            json!({"table": "Nope", "store": periods, "refresh": periods}),
            "Nope",
        ),
        (json!({}), "table"),
    ] {
        let e = s.execute("table.set_refresh_policy", &p).unwrap_err();
        assert!(e.contains(want), "{p}: {e}");
    }
    run(
        &mut s,
        "table.set_storage_mode",
        json!({"table": "Orders", "mode": "directQuery"}),
    );
    let e = s
        .execute(
            "table.set_refresh_policy",
            &json!({"table": "Orders", "store": periods, "refresh": periods}),
        )
        .unwrap_err();
    assert!(e.contains("Import tables"), "{e}");
    for p in [json!({}), json!({"table": 1}), json!({"table": "Orders", "x": 1})] {
        assert!(s.execute("table.remove_refresh_policy", &p).is_err(), "{p}");
    }
    // nothing was changed
    assert!(policy(&s, "Orders").is_none());
}

#[test]
fn the_policy_is_saved_to_tmdl_tmsl_and_pbix() {
    let tmp = TempDir::new("engine-refresh-policy-save");
    for (format, file) in [
        ("pbip-tmdl", "Tmdl.pbip"),
        ("pbip-tmsl", "Tmsl.pbip"),
        ("pbix", "shop.pbix"),
    ] {
        let mut s = model(true);
        run(
            &mut s,
            "table.set_refresh_policy",
            json!({"table": "Orders", "store": {"n": 3, "unit": "year"}, "refresh": {"n": 7, "unit": "day"},
                   "detect_changes": "Modified", "realtime": true}),
        );
        let path = tmp.path().join(file);
        let mut p = json!({"path": path, "format": format});
        if format == "pbix" {
            p["empty_tables"] = json!(true);
        }
        run(&mut s, "file.save_as", p);
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        assert_eq!(back.model_error, None, "{format}");
        assert_eq!(
            table_snap(&back, "Orders")["refresh_policy"],
            table_snap(&s, "Orders")["refresh_policy"],
            "{format}"
        );
        let (a, b) = (policy(&back, "Orders").unwrap(), policy(&s, "Orders").unwrap());
        for k in [
            "mode",
            "rollingWindowPeriods",
            "incrementalGranularity",
            "incrementalPeriodsOffset",
        ] {
            assert_eq!(a[k], b[k], "{format} {k}");
        }
        // TMDL may split the M text into lines
        let text = |v: &Value| match v {
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n"),
            v => v.as_str().unwrap_or_default().to_string(),
        };
        assert_eq!(text(&a["sourceExpression"]).trim(), FILTERED, "{format}");
        assert!(text(&a["pollingExpression"]).contains("List.Max"), "{format}");
    }
}

#[test]
fn a_policy_is_added_to_and_removed_from_an_opened_pbix() {
    let tmp = TempDir::new("engine-refresh-policy-pbix");
    let path = tmp.path().join("shop.pbix");
    let mut s = model(true);
    run(
        &mut s,
        "file.save_as",
        json!({"path": path, "empty_tables": true}),
    );
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    run(
        &mut s,
        "table.set_refresh_policy",
        json!({"table": "Orders", "store": {"n": 2, "unit": "year"}, "refresh": {"n": 3, "unit": "month"}}),
    );
    run(&mut s, "file.save", json!({}));
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": path}));
    assert_eq!(
        table_snap(&back, "Orders")["refresh_policy"]["refresh"],
        json!({"n": 3, "unit": "month"})
    );
    run(
        &mut back,
        "table.remove_refresh_policy",
        json!({"table": "Orders"}),
    );
    run(&mut back, "file.save", json!({}));
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": path}));
    assert!(table_snap(&again, "Orders").get("refresh_policy").is_none());
}
