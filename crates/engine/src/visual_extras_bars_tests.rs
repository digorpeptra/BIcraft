//! Error bars, the scatter's ratio line and symmetry shading, line transparency
//! (`visual.analytics`, `visual_extras_bars.rs`): the PBIR shape, read back, PBIP and PBIX round
//! trips, the bounds the preview evaluates (`data.error_bars`), hostile params.

use bicraft_report::Visual;
use serde_json::{Value, json};

use crate::Session;
use crate::tempdir::TempDir;
use crate::visual_props::{apply_props, extract_props};

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A command that must fail, and leaves the report as it was.
fn fails(s: &mut Session, id: &str, params: Value) -> String {
    let before = s.report.clone();
    let err = s
        .execute(id, &params)
        .expect_err(&format!("{id} {params} should fail"));
    assert_eq!(s.report, before, "{id} changed the report although it failed");
    err
}

const SALES: &str = "#table(type table [Region = text, Amount = number, Hi = number, Lo = number], {\
    {\"a\", 10, 2, 1}, {\"b\", 20, 4, 3}, {\"c\", 30, 6, 5}})";

/// Sales(Region, Amount, Hi, Lo) with [Total], [Upper] and [Lower]; one page "Main".
fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Bars"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": SALES}}),
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
            "column.add",
            json!({"table": "Sales", "name": "Hi", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Lo", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Upper", "expression": "SUM(Sales[Amount]) + SUM(Sales[Hi])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Lower", "expression": "SUM(Sales[Amount]) - SUM(Sales[Lo])"}),
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

fn chart(s: &mut Session, visual_type: &str) -> String {
    add(
        s,
        visual_type,
        json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]),
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

fn objects(s: &Session, name: &str) -> Value {
    visual(s, name)
        .visual_extra
        .get("objects")
        .cloned()
        .unwrap_or(Value::Null)
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

/// `apply_props(extract_props(v))` changes nothing, with and without the model.
fn assert_extract_noop(s: &Session, v: &Visual) {
    let props = extract_props(v);
    for model in [s.model.as_ref(), None] {
        let mut copy = v.clone();
        let changes = apply_props(&mut copy, &props, model).unwrap_or_else(|e| panic!("{props}: {e}"));
        assert!(
            changes.is_empty(),
            "extract -> apply changed {changes:?}\nprops: {props}"
        );
        assert_eq!(&copy, v);
    }
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
fn error_bars_write_read_back_and_round_trip_through_pbip_and_pbix() {
    let mut s = session();
    let col = chart(&mut s, "clusteredColumnChart");
    let lines = json!([{"kind": "error_bars", "upper": "[Upper]", "lower": "[Lower]", "color": "#A0A0A0",
                        "width": 3, "marker_shape": "circle", "data_label": true}]);
    let out = analytics(&mut s, &col, lines.clone());
    assert_eq!(out["changed"], true, "{out}");
    let e = &objects(&s, &col)["error"][0];
    assert_eq!(e["selector"], json!({"metadata": "Sales.Total"}));
    let p = &e["properties"];
    assert_eq!(p["enabled"], lit("true"));
    assert_eq!(p["errorRange"]["kind"], "ErrorRange");
    assert_eq!(p["errorRange"]["explicit"]["isRelative"], lit("false"));
    assert_eq!(
        p["errorRange"]["explicit"]["upperBound"],
        json!({"expr": {"Measure": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Upper"}}})
    );
    assert_eq!(p["barColor"], json!({"solid": {"color": lit("'#A0A0A0'")}}));
    assert_eq!(p["markerShape"], lit("'circle'"));
    assert_eq!(p["labelShow"], lit("true"));
    let read = json!([{"kind": "error_bars", "measure": "[Total]", "upper": "[Upper]", "lower": "[Lower]",
                       "color": "#A0A0A0", "width": 3.0, "marker_shape": "circle", "data_label": true}]);
    assert_eq!(out["props"]["analytics"], read);
    // the same params change nothing; extract → apply changes nothing, even without the model
    let again = analytics(&mut s, &col, lines);
    assert_eq!(again["changed"], false, "{again}");
    assert_extract_noop(&s, visual(&s, &col));
    // the snapshot lists them for the renderer, the data their bounds
    let v = snapshot_visual(&s, &col);
    let rl = &v["format"]["reference_lines"][0];
    assert_eq!(rl["kind"], "error_bars", "{v}");
    assert_eq!(rl["column"], 1);
    let d = &v["data"];
    assert_eq!(d["status"], "ok", "{d}");
    let rows = d["rows"].as_array().unwrap();
    let eb = &d["error_bars"][0];
    assert_eq!(eb["column"], 1, "{d}");
    for (k, r) in rows.iter().enumerate() {
        let (total, hi, lo) = match r[0].as_str().unwrap() {
            "a" => (10.0, 2.0, 1.0),
            "b" => (20.0, 4.0, 3.0),
            _ => (30.0, 6.0, 5.0),
        };
        assert_eq!(r[1].as_f64(), Some(total));
        assert_eq!(eb["upper"][k].as_f64(), Some(total + hi), "{d}");
        assert_eq!(eb["lower"][k].as_f64(), Some(total - lo), "{d}");
    }
    // relative bounds: the series' value plus the bound's
    analytics(
        &mut s,
        &col,
        json!([{"kind": "error_bars", "upper": "Sum(Sales[Hi])", "lower": "Sum(Sales[Lo])", "relative": true}]),
    );
    let d = snapshot_visual(&s, &col)["data"].clone();
    let rows = d["rows"].as_array().unwrap();
    for (k, r) in rows.iter().enumerate() {
        let (up, lo) = match r[0].as_str().unwrap() {
            "a" => (12.0, 11.0),
            "b" => (24.0, 23.0),
            _ => (36.0, 35.0),
        };
        assert_eq!(d["error_bars"][0]["upper"][k].as_f64(), Some(up), "{d}");
        assert_eq!(d["error_bars"][0]["lower"][k].as_f64(), Some(lo), "{d}");
    }
    // the PBIP project and the PBIX file keep them
    analytics(
        &mut s,
        &col,
        json!([{"kind": "error_bars", "upper": "[Upper]", "lower": "[Lower]", "marker": false, "match_series": true,
                "show": false}]),
    );
    // a line chart's error bars have a band (a column chart's have none)
    let line = chart(&mut s, "lineChart");
    let band = json!([{"kind": "error_bars", "lower": "[Lower]", "band": true, "band_style": "fillLine",
                       "band_color": "#00AAFF", "transparency": 40}]);
    analytics(&mut s, &line, band);
    assert_eq!(
        objects(&s, &line)["error"][0]["properties"]["shadeBandStyle"],
        lit("'fillLine'")
    );
    assert_extract_noop(&s, visual(&s, &line));
    let props = extract_props(visual(&s, &col));
    let line_props = extract_props(visual(&s, &line));
    assert_eq!(line_props["analytics"][0]["band"], true, "{line_props}");
    let dir = TempDir::new("error-bars");
    for path in [dir.join("pbip").join("Bars.pbip"), dir.join("Bars.pbix")] {
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
        let v = page.visual(&col).unwrap();
        assert_eq!(
            extract_props(v)["analytics"],
            props["analytics"],
            "{}",
            path.display()
        );
        assert_eq!(
            extract_props(page.visual(&line).unwrap())["analytics"],
            line_props["analytics"],
            "{}",
            path.display()
        );
        assert_eq!(
            v.visual_extra["objects"]["error"],
            visual(&s, &col).visual_extra["objects"]["error"]
        );
    }
}

#[test]
fn error_bars_hostile_params_are_errors() {
    let mut s = session();
    let col = chart(&mut s, "clusteredColumnChart");
    let card = add(&mut s, "card", json!([{"role": "Values", "field": "[Total]"}]));
    let bad = |s: &mut Session, v: &str, line: Value| -> String {
        fails(
            s,
            "visual.analytics",
            json!({"page": "Main", "visual": v, "lines": [line]}),
        )
    };
    assert!(bad(&mut s, &card, json!({"kind": "error_bars", "upper": "[Upper]"})).contains("no error bars"));
    assert!(bad(&mut s, &col, json!({"kind": "error_bars"})).contains("`upper` or `lower`"));
    assert!(bad(&mut s, &col, json!({"kind": "error_bars", "upper": "Sales[Hi]"})).contains("not a measure"));
    assert!(
        bad(&mut s, &col, json!({"kind": "error_bars", "upper": "[Nope]"})).contains("analytics[0].upper")
    );
    for line in [
        json!({"kind": "error_bars", "upper": 3}),
        json!({"kind": "error_bars", "upper": "[Upper]", "marker_shape": "star"}),
        json!({"kind": "error_bars", "upper": "[Upper]", "band_style": 1}),
        // no band on a column chart's error bars
        json!({"kind": "error_bars", "upper": "[Upper]", "band": true}),
        json!({"kind": "error_bars", "upper": "[Upper]", "width": -1}),
        json!({"kind": "error_bars", "upper": "[Upper]", "transparency": 101}),
        json!({"kind": "error_bars", "upper": "[Upper]", "color": "red"}),
        json!({"kind": "error_bars", "upper": "[Upper]", "relative": "yes"}),
        json!({"kind": "error_bars", "upper": "[Upper]", "value": 3}),
        json!({"kind": "error_bars", "upper": "[Upper]", "measure": "[Total]", "series": "[Total]"}),
        json!({"kind": "error_bars", "upper": "[Upper]", "measure": "[Upper]"}),
        json!({"kind": "symmetry_shading"}),
        json!({"kind": "ratio_line"}),
        json!({"kind": "constant", "value": 1, "transparency": "x"}),
        json!({"kind": "forecast", "transparency": 10}),
    ] {
        bad(&mut s, &col, line);
    }
    let two = json!([{"kind": "error_bars", "upper": "[Upper]"}, {"kind": "error_bars", "lower": "[Lower]"}]);
    let e = fails(
        &mut s,
        "visual.analytics",
        json!({"page": "Main", "visual": col, "lines": two}),
    );
    assert!(e.contains("at most one set of error bars"), "{e}");
}

#[test]
fn scatter_ratio_line_symmetry_shading_and_line_transparency() {
    let mut s = session();
    let sc = add(
        &mut s,
        "scatterChart",
        json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "X", "field": "[Total]"},
               {"role": "Y", "field": "[Upper]"}]),
    );
    let lines = json!([
        {"kind": "constant", "axis": "x", "value": 15, "transparency": 25},
        {"kind": "ratio_line", "color": "#123456", "style": "dashed", "transparency": 10},
        {"kind": "symmetry_shading", "upper_color": "#00FF00", "lower_color": "#FF0000", "transparency": 60},
    ]);
    let out = analytics(&mut s, &sc, lines.clone());
    let o = objects(&s, &sc);
    assert_eq!(o["ratioLine"][0].get("selector"), None, "{o}");
    assert_eq!(
        o["ratioLine"][0]["properties"]["lineColor"],
        json!({"solid": {"color": lit("'#123456'")}})
    );
    assert_eq!(o["plotAreaShading"][0].get("selector"), None);
    assert_eq!(
        o["plotAreaShading"][0]["properties"]["upperShadingColor"],
        json!({"solid": {"color": lit("'#00FF00'")}})
    );
    assert_eq!(
        o["xAxisReferenceLine"][0]["properties"]["transparency"]["expr"]["Literal"]["Value"]
            .as_str()
            .map(|t| t.trim_end_matches(['D', 'L'])),
        Some("25")
    );
    assert_eq!(
        out["props"]["analytics"],
        json!([
            {"kind": "constant", "axis": "x", "value": 15.0, "transparency": 25.0},
            {"kind": "ratio_line", "color": "#123456", "style": "dashed", "transparency": 10.0},
            {"kind": "symmetry_shading", "upper_color": "#00FF00", "lower_color": "#FF0000", "transparency": 60.0},
        ])
    );
    assert_eq!(analytics(&mut s, &sc, lines)["changed"], false);
    assert_extract_noop(&s, visual(&s, &sc));
    let kinds: Vec<Value> = snapshot_visual(&s, &sc)["format"]["reference_lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["kind"].clone())
        .collect();
    assert_eq!(
        kinds,
        [json!("constant"), json!("ratio_line"), json!("symmetry_shading")]
    );
    // one of each at most; none on a column chart
    let e = fails(
        &mut s,
        "visual.analytics",
        json!({"page": "Main", "visual": sc, "lines": [{"kind": "ratio_line"}, {"kind": "ratio_line"}]}),
    );
    assert!(e.contains("at most one ratio line"), "{e}");
    let col = chart(&mut s, "clusteredColumnChart");
    let e = fails(
        &mut s,
        "visual.analytics",
        json!({"page": "Main", "visual": col, "lines": [{"kind": "symmetry_shading"}]}),
    );
    assert!(e.contains("no symmetry shading"), "{e}");
}
