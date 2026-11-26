//! Anomaly detection (`visual.analytics` `anomaly_detection`, `visual_extras_anomaly.rs`): the
//! PBIR shape, read back, PBIP and PBIX round trips, BIcraft's estimate in `data.anomalies`, the
//! limits Microsoft Learn lists, hostile params.

use bicraft_report::Visual;
use serde_json::{Value, json};

use crate::Session;
use crate::tempdir::TempDir;
use crate::visual_props::{apply_props, extract_props};

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn fails(s: &mut Session, v: &str, lines: Value) -> String {
    let before = s.report.clone();
    let err = s
        .execute(
            "visual.analytics",
            &json!({"page": "Main", "visual": v, "lines": lines}),
        )
        .expect_err(&format!("{lines} should fail"));
    assert_eq!(s.report, before, "a failed call changed the report");
    err
}

/// Twelve days of sales, a spike on day 7.
const SALES: &str = "#table(type table [Day = number, Region = text, Amount = number], {\
    {1, \"a\", 100}, {2, \"a\", 102}, {3, \"b\", 99}, {4, \"a\", 101}, {5, \"b\", 103}, {6, \"a\", 100}, \
    {7, \"b\", 190}, {8, \"a\", 98}, {9, \"b\", 101}, {10, \"a\", 102}, {11, \"b\", 100}, {12, \"a\", 99}})";

fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Anomalies"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": SALES}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Day", "data_type": "int64"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Region", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Twice", "expression": "2 * SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "Main"})),
    ] {
        run(&mut s, id, p);
    }
    s
}

fn add(s: &mut Session, visual_type: &str, fields: Value) -> String {
    run(
        s,
        "visual.add",
        json!({"page": "Main", "visual_type": visual_type, "x": 0, "y": 0, "width": 400,
               "height": 300, "fields": fields}),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string()
}

fn line(s: &mut Session) -> String {
    add(
        s,
        "lineChart",
        json!([{"role": "Category", "field": "Sales[Day]"}, {"role": "Y", "field": "[Total]"}]),
    )
}

fn visual<'a>(s: &'a Session, name: &str) -> &'a Visual {
    s.report
        .as_ref()
        .unwrap()
        .pages
        .iter()
        .find(|p| p.display_name == "Main")
        .unwrap()
        .visual(name)
        .unwrap()
}

fn lit(v: &str) -> Value {
    json!({"expr": {"Literal": {"Value": v}}})
}

fn analytics(s: &mut Session, v: &str, lines: Value) -> Value {
    run(
        s,
        "visual.analytics",
        json!({"page": "Main", "visual": v, "lines": lines}),
    )
}

fn snapshot_visual(s: &Session, name: &str) -> Value {
    let snap = s.snapshot();
    for p in snap["report"]["pages"].as_array().cloned().unwrap_or_default() {
        for v in p["visuals"].as_array().cloned().unwrap_or_default() {
            if v["name"] == name {
                return v;
            }
        }
    }
    panic!("visual {name} not in the snapshot")
}

#[test]
fn anomaly_detection_writes_reads_back_and_round_trips_through_pbip_and_pbix() {
    let mut s = session();
    let v = line(&mut s);
    let lines = json!([{"kind": "anomaly_detection", "sensitivity": 80, "marker_shape": "diamond",
                        "marker_size": 7, "marker_color": "#C00000", "band_style": "line",
                        "band_color": "#118DFF", "transparency": 60, "label": "Spikes"}]);
    let out = analytics(&mut s, &v, lines.clone());
    assert_eq!(out["changed"], true, "{out}");
    let e = &visual(&s, &v).visual_extra["objects"]["anomalyDetection"][0];
    assert_eq!(e["selector"], json!({"metadata": "Sales.Total"}), "{e}");
    let p = &e["properties"];
    assert_eq!(p["show"], lit("true"));
    assert_eq!(p["displayName"], lit("'Spikes'"));
    assert_eq!(p["markerShape"], lit("'diamond'"));
    assert_eq!(p["confidenceBandStyle"], lit("'line'"));
    assert_eq!(
        p["confidenceBandColor"],
        json!({"solid": {"color": lit("'#118DFF'")}})
    );
    assert_eq!(p["transform"]["algorithm"], "AnomalyDetection");
    assert_eq!(p["transform"]["parameters"][0]["Name"], "Sensitivity");
    let read = &out["props"]["analytics"][0];
    assert_eq!(read["kind"], "anomaly_detection", "{read}");
    assert_eq!(read["measure"], "[Total]");
    assert_eq!(read["sensitivity"], 80.0);
    assert_eq!(read["marker_size"], 7.0);
    // the same params change nothing; extract → apply changes nothing, even without the model
    assert_eq!(analytics(&mut s, &v, lines)["changed"], false);
    let vis = visual(&s, &v).clone();
    let props = extract_props(&vis);
    for model in [s.model.as_ref(), None] {
        let mut copy = vis.clone();
        let changes = apply_props(&mut copy, &props, model).unwrap();
        assert!(changes.is_empty(), "{changes:?}");
        assert_eq!(copy, vis);
    }
    // the snapshot: the options for the renderer, BIcraft's estimate in the data
    let snap = snapshot_visual(&s, &v);
    let rl = &snap["format"]["reference_lines"][0];
    assert_eq!(rl["kind"], "anomaly_detection", "{snap}");
    assert_eq!(rl["column"], 1);
    assert_eq!(rl["sensitivity"], 80.0);
    let a = &snap["data"]["anomalies"];
    assert_eq!(a["estimate"], "bicraft", "{}", snap["data"]);
    assert_eq!(a["column"], 1);
    let rows = snap["data"]["rows"].as_array().unwrap();
    let flagged: Vec<i64> = rows
        .iter()
        .enumerate()
        .filter(|(k, _)| a["anomaly"][*k] == true)
        .map(|(_, r)| r[0].as_i64().unwrap())
        .collect();
    assert_eq!(flagged, vec![7], "{a}");
    for k in 0..rows.len() {
        assert!(a["low"][k].as_f64().unwrap() <= a["high"][k].as_f64().unwrap());
    }
    // hidden: no estimate
    analytics(&mut s, &v, json!([{"kind": "anomaly_detection", "show": false}]));
    assert!(snapshot_visual(&s, &v)["data"].get("anomalies").is_none());
    analytics(
        &mut s,
        &v,
        json!([{"kind": "anomaly_detection", "sensitivity": 40}]),
    );
    let props = extract_props(visual(&s, &v));
    let dir = TempDir::new("anomalies");
    for path in [
        dir.join("pbip").join("Anomalies.pbip"),
        dir.join("Anomalies.pbix"),
    ] {
        run(&mut s, "file.save_as", json!({"path": path}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        let page = back
            .report
            .as_ref()
            .unwrap()
            .pages
            .iter()
            .find(|p| p.display_name == "Main")
            .unwrap();
        let bv = page.visual(&v).unwrap();
        assert_eq!(
            extract_props(bv)["analytics"],
            props["analytics"],
            "{}",
            path.display()
        );
        assert_eq!(
            bv.visual_extra["objects"]["anomalyDetection"],
            visual(&s, &v).visual_extra["objects"]["anomalyDetection"]
        );
    }
}

#[test]
fn anomaly_detection_follows_the_limits_microsoft_learn_lists() {
    let mut s = session();
    let v = line(&mut s);
    // not with a forecast, nor a statistical line
    for other in [
        json!({"kind": "forecast"}),
        json!({"kind": "average"}),
        json!({"kind": "percentile", "percentile": 90}),
    ] {
        let e = fails(&mut s, &v, json!([{"kind": "anomaly_detection"}, other]));
        assert!(e.contains("does not work with a forecast"), "{e}");
    }
    // a constant line and a trend line are fine
    analytics(
        &mut s,
        &v,
        json!([{"kind": "anomaly_detection"}, {"kind": "constant", "value": 100}, {"kind": "trend"}]),
    );
    // at most one
    let e = fails(
        &mut s,
        &v,
        json!([{"kind": "anomaly_detection"}, {"kind": "anomaly_detection"}]),
    );
    assert!(e.contains("at most one anomaly detection"), "{e}");
    // line charts only, one value, no legend
    let col = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Day]"}, {"role": "Y", "field": "[Total]"}]),
    );
    assert!(fails(&mut s, &col, json!([{"kind": "anomaly_detection"}])).contains("no anomaly detection"));
    let legend = add(
        &mut s,
        "lineChart",
        json!([{"role": "Category", "field": "Sales[Day]"}, {"role": "Series", "field": "Sales[Region]"},
               {"role": "Y", "field": "[Total]"}]),
    );
    assert!(fails(&mut s, &legend, json!([{"kind": "anomaly_detection"}])).contains("legend"));
    let two = add(
        &mut s,
        "lineChart",
        json!([{"role": "Category", "field": "Sales[Day]"}, {"role": "Y", "field": "[Total]"},
               {"role": "Y", "field": "[Twice]"}]),
    );
    assert!(fails(&mut s, &two, json!([{"kind": "anomaly_detection"}])).contains("one value"));
}

#[test]
fn anomaly_detection_hostile_params_are_errors() {
    let mut s = session();
    let v = line(&mut s);
    for bad in [
        json!({"kind": "anomaly_detection", "sensitivity": 101}),
        json!({"kind": "anomaly_detection", "sensitivity": "high"}),
        json!({"kind": "anomaly_detection", "marker_shape": "star"}),
        json!({"kind": "anomaly_detection", "marker_size": 0}),
        json!({"kind": "anomaly_detection", "band_style": "fillLine"}),
        json!({"kind": "anomaly_detection", "band_color": "blue"}),
        json!({"kind": "anomaly_detection", "transparency": -1}),
        json!({"kind": "anomaly_detection", "value": 3}),
        json!({"kind": "anomaly_detection", "measure": "[Twice]"}),
        json!({"kind": "anomaly_detection", "measure": "[Total]", "series": "[Total]"}),
        json!({"kind": "anomaly_detection", "label": 3}),
    ] {
        fails(&mut s, &v, json!([bad]));
    }
    // an entry BIcraft does not write is kept raw
    let raw = json!({"properties": {"show": lit("true"), "explainBy": lit("'x'")},
                     "selector": {"metadata": "Sales.Total"}});
    let out = analytics(
        &mut s,
        &v,
        json!([{"object": "anomalyDetection", "raw": raw.clone()}]),
    );
    assert_eq!(
        out["props"]["analytics"],
        json!([{"object": "anomalyDetection", "raw": raw}])
    );
}
