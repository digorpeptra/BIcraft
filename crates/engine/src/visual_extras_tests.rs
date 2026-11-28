//! Tests of `visual_extras.rs`: axes, analytics lines, small multiples, report-page tooltips,
//! page types, visual interactions, layer order and mobile layout.

use bicraft_report::{Page, Report, Visual};
use serde_json::{Value, json};

use crate::Session;
use crate::visual_props::{apply_props, extract_props};

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Runs a command that must fail and checks the report and its dirty flag did not change.
fn fails(s: &mut Session, id: &str, params: Value) -> String {
    let before = s.report.clone();
    let dirty = s.report_dirty;
    let err = s
        .execute(id, &params)
        .expect_err(&format!("{id} {params} should fail"));
    assert_eq!(s.report, before, "{id} changed the report although it failed");
    assert_eq!(s.report_dirty, dirty);
    err
}

/// Runs a command a second time: nothing changes, the dirty flag stays clear.
fn idempotent(s: &mut Session, id: &str, params: Value) {
    let before = s.report.clone();
    s.report_dirty = false;
    let out = run(s, id, params);
    assert_eq!(out["changed"], false, "{id} re-run changed something: {out}");
    assert_eq!(s.report, before);
    assert!(!s.report_dirty);
}

/// Sales(Amount, Region) with [Total] and [Margin], Product(Name), a blank report.
fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        ("table.add", json!({"name": "Sales"})),
        ("table.add", json!({"name": "Product"})),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Region", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Name", "data_type": "string"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Margin", "expression": "[Total] * 0.2"}),
        ),
        ("report.new", json!({})),
    ] {
        run(&mut s, id, p);
    }
    s
}

fn report(s: &Session) -> &Report {
    s.report.as_ref().unwrap()
}

fn page(s: &Session) -> String {
    report(s).pages[0].name.clone()
}

fn page_of<'a>(s: &'a Session, name: &str) -> &'a Page {
    report(s).page(name).unwrap()
}

fn add(s: &mut Session, visual_type: &str, fields: Value) -> String {
    let page = page(s);
    let v = run(
        s,
        "visual.add",
        json!({"page": page, "visual_type": visual_type, "x": 10, "y": 20, "width": 400,
               "height": 300, "fields": fields}),
    );
    v["visual"].as_str().unwrap().to_string()
}

fn chart(s: &mut Session, visual_type: &str) -> String {
    add(
        s,
        visual_type,
        json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"},
               {"role": "Y", "field": "[Margin]"}]),
    )
}

fn visual<'a>(s: &'a Session, name: &str) -> &'a Visual {
    page_of(s, &page(s)).visual(name).unwrap()
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

/// PBIR write → read gives the same report, and every document validates.
fn assert_pbir_ok(s: &mut Session) {
    let r = report(s).clone();
    let docs = bicraft_pbir::write_documents(&r);
    let issues = bicraft_pbir::validate_documents(&docs);
    assert!(issues.is_empty(), "{issues:#?}");
    let back = bicraft_pbir::read_documents(&docs).unwrap();
    let mut again = bicraft_pbir::write_documents(&back);
    let mut docs = docs;
    again.sort();
    docs.sort();
    for ((pa, a), (pb, b)) in again.iter().zip(&docs) {
        assert_eq!(pa, pb);
        assert_eq!(String::from_utf8_lossy(a), String::from_utf8_lossy(b), "{pa}");
    }
    assert_eq!(again.len(), docs.len());
    for p in &r.pages {
        let q = back.page(&p.name).unwrap();
        for v in &p.visuals {
            assert_eq!(extract_props(v), extract_props(q.visual(&v.name).unwrap()));
        }
    }
    let v = run(s, "report.validate", json!({}));
    assert_eq!(v["schema_issues"], json!([]), "{v}");
    assert_eq!(v["error_count"], 0, "{v}");
}

// ---------------------------------------------------------------------------------------------
// Axes
// ---------------------------------------------------------------------------------------------

#[test]
fn axis_log_scale_range_and_gridlines() {
    let mut s = session();
    let v = chart(&mut s, "clusteredColumnChart");
    let pg = page(&s);
    let params = json!({"page": pg, "visual": v, "axis": "y", "scale": "log", "min": 10, "max": 100000,
        "gridlines": {"show": true, "color": "#CCCCCC", "style": "dashed"}, "display_units": "thousands",
        "precision": 1, "title": "Revenue", "invert": false});
    let out = run(&mut s, "visual.axis", params.clone());
    assert_eq!(out["changed"], true, "{out}");
    let props = &objects(&s, &v)["valueAxis"][0]["properties"];
    assert_eq!(props["logAxisScale"], lit("true"));
    assert_eq!(
        props["start"]["expr"]["Literal"]["Value"]
            .as_str()
            .unwrap()
            .trim_end_matches(['D', 'L']),
        "10"
    );
    assert_eq!(
        props["end"]["expr"]["Literal"]["Value"]
            .as_str()
            .unwrap()
            .trim_end_matches(['D', 'L']),
        "100000"
    );
    assert_eq!(props["gridlineShow"], lit("true"));
    assert_eq!(props["gridlineStyle"], lit("'dashed'"));
    assert_eq!(props["showAxisTitle"], lit("true"));
    assert_eq!(props["titleText"], lit("'Revenue'"));
    assert_eq!(
        out["props"]["axes"]["y"],
        json!({"title": true, "title_text": "Revenue", "min": 10.0, "max": 100000.0, "scale": "log",
               "gridlines": {"show": true, "color": "#CCCCCC", "style": "dashed"}, "invert": false,
               "display_units": "thousands", "precision": 1})
    );
    idempotent(&mut s, "visual.axis", params);
    assert_extract_noop(&s, visual(&s, &v));
    // x axis: start at zero, shown; null removes one setting.
    run(
        &mut s,
        "visual.axis",
        json!({"page": pg, "visual": v, "axis": "x", "show": true, "font_size": 12}),
    );
    run(
        &mut s,
        "visual.axis",
        json!({"page": pg, "visual": v, "axis": "x", "type": "continuous"}),
    );
    assert_eq!(
        objects(&s, &v)["categoryAxis"][0]["properties"]["axisType"],
        lit("'Scalar'")
    );
    assert_eq!(extract_props(visual(&s, &v))["axes"]["x"]["type"], "continuous");
    fails(
        &mut s,
        "visual.axis",
        json!({"page": pg, "visual": v, "axis": "y", "type": "continuous"}),
    );
    run(
        &mut s,
        "visual.axis",
        json!({"page": pg, "visual": v, "axis": "y", "min": null, "start_at_zero": true}),
    );
    let props = &objects(&s, &v)["valueAxis"][0]["properties"];
    assert_eq!(
        props["start"]["expr"]["Literal"]["Value"]
            .as_str()
            .unwrap()
            .trim_end_matches(['D', 'L']),
        "0"
    );
    assert_pbir_ok(&mut s);
    // clear removes the axis formatting.
    run(
        &mut s,
        "visual.axis",
        json!({"page": pg, "visual": v, "axis": "y", "clear": true}),
    );
    assert!(objects(&s, &v).get("valueAxis").is_none());
}

#[test]
fn axis_errors_and_secondary_axis() {
    let mut s = session();
    let v = chart(&mut s, "clusteredColumnChart");
    let pg = page(&s);
    let e = fails(
        &mut s,
        "visual.axis",
        json!({"page": pg, "visual": v, "axis": "x", "label_angle": 45}),
    );
    assert!(e.contains("no such axis setting"), "{e}");
    let e = fails(
        &mut s,
        "visual.axis",
        json!({"page": pg, "visual": v, "axis": "y2", "show": true}),
    );
    assert!(e.contains("secondary"), "{e}");
    fails(
        &mut s,
        "visual.axis",
        json!({"page": pg, "visual": v, "axis": "y", "scale": "sqrt"}),
    );
    fails(
        &mut s,
        "visual.axis",
        json!({"page": pg, "visual": v, "axis": "y", "display_units": 7}),
    );
    fails(
        &mut s,
        "visual.axis",
        json!({"page": pg, "visual": v, "axis": "y", "min": 1, "start_at_zero": true}),
    );
    fails(
        &mut s,
        "visual.axis",
        json!({"page": pg, "visual": v, "axis": "y"}),
    );
    let card = add(&mut s, "card", json!([{"role": "Values", "field": "[Total]"}]));
    let e = fails(
        &mut s,
        "visual.axis",
        json!({"page": pg, "visual": card, "axis": "y", "show": true}),
    );
    assert!(e.contains("no y axis"), "{e}");

    let combo = add(
        &mut s,
        "lineClusteredColumnComboChart",
        json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"},
               {"role": "Y2", "field": "[Margin]"}]),
    );
    run(
        &mut s,
        "visual.axis",
        json!({"page": pg, "visual": combo, "axis": "y2", "show": true, "min": 0, "max": 50, "scale": "log"}),
    );
    let props = &objects(&s, &combo)["valueAxis"][0]["properties"];
    assert_eq!(props["secShow"], lit("true"));
    assert_eq!(props["secLogAxisScale"], lit("true"));
    assert!(
        props.get("secStart").is_some() && props.get("secEnd").is_some(),
        "{props}"
    );
    fails(
        &mut s,
        "visual.axis",
        json!({"page": pg, "visual": combo, "axis": "y2", "gridlines": true}),
    );
    assert_extract_noop(&s, visual(&s, &combo));
    let line = chart(&mut s, "lineChart");
    run(
        &mut s,
        "visual.axis",
        json!({"page": pg, "visual": line, "axis": "y2", "show": true, "title": false}),
    );
    let props = &objects(&s, &line)["y2Axis"][0]["properties"];
    assert_eq!(props["show"], lit("true"));
    assert_eq!(props["secShowAxisTitle"], lit("false"));
    assert_extract_noop(&s, visual(&s, &line));
    assert_pbir_ok(&mut s);
}

// ---------------------------------------------------------------------------------------------
// Analytics lines
// ---------------------------------------------------------------------------------------------

#[test]
fn analytics_lines_desktop_shape() {
    let mut s = session();
    let v = chart(&mut s, "clusteredColumnChart");
    let pg = page(&s);
    let lines = json!([
        {"kind": "constant", "value": 1000, "label": "Target", "color": "#FF0000", "style": "dashed", "width": 2},
        {"kind": "average", "measure": "[Margin]", "label": "Avg margin", "position": "front"},
        {"kind": "percentile", "percentile": 90, "data_label": true},
        {"kind": "trend", "color": "#00AA00"},
    ]);
    let out = run(
        &mut s,
        "visual.analytics",
        json!({"page": pg, "visual": v, "lines": lines}),
    );
    assert_eq!(out["changed"], true, "{out}");
    let o = objects(&s, &v);
    let constant = &o["y1AxisReferenceLine"][0];
    assert_eq!(constant["selector"], json!({"id": "1"}));
    assert_eq!(constant["properties"]["show"], lit("true"));
    assert_eq!(constant["properties"]["displayName"], lit("'Target'"));
    assert_eq!(constant["properties"]["style"], lit("'dashed'"));
    let avg = &o["referenceLine"][0];
    assert_eq!(avg["selector"], json!({"metadata": "Sales.Margin", "id": "1"}));
    assert_eq!(
        avg["properties"]["value"],
        json!({"expr": {"Aggregation": {"Expression": {"SelectRef": {"ExpressionName": "Sales.Margin"}}, "Function": 1}}})
    );
    let pct = &o["referenceLine"][1];
    assert_eq!(pct["selector"], json!({"metadata": "Sales.Total", "id": "2"}));
    assert_eq!(
        pct["properties"]["value"]["expr"]["Percentile"],
        json!({"Expression": {"SelectRef": {"ExpressionName": "Sales.Total"}}, "K": 0.9})
    );
    assert_eq!(o["trend"][0].get("selector"), None);
    assert_eq!(
        out["props"]["analytics"],
        json!([
            {"kind": "constant", "value": 1000.0, "label": "Target", "color": "#FF0000", "style": "dashed", "width": 2.0},
            {"kind": "average", "measure": "[Margin]", "label": "Avg margin", "position": "front"},
            {"kind": "percentile", "measure": "[Total]", "percentile": 90.0, "data_label": true},
            {"kind": "trend", "color": "#00AA00"},
        ])
    );
    idempotent(
        &mut s,
        "visual.analytics",
        json!({"page": pg, "visual": v, "lines": lines}),
    );
    assert_extract_noop(&s, visual(&s, &v));
    assert_pbir_ok(&mut s);

    // Unmodelled settings and ids survive an edit; removing the first line keeps the second's id.
    {
        let r = s.report.as_mut().unwrap();
        let vis = r.pages[0].visual_mut(&v).unwrap();
        vis.visual_extra["objects"]["referenceLine"][1]["properties"]["dataLabelColor"] =
            json!({"solid": {"color": lit("'#123456'")}});
    }
    run(
        &mut s,
        "visual.analytics",
        json!({"page": pg, "visual": v, "lines": [
            {"kind": "percentile", "percentile": 90, "data_label": true},
        ]}),
    );
    let o = objects(&s, &v);
    assert_eq!(o["referenceLine"].as_array().unwrap().len(), 1);
    // Matched by place: the first entry was re-used (its id 1 kept).
    assert_eq!(o["referenceLine"][0]["selector"]["id"], "1");
    assert!(
        o.get("y1AxisReferenceLine").is_none() && o.get("trend").is_none(),
        "{o}"
    );
    // An entry with unmodelled properties extracts raw and re-applies unchanged.
    {
        let r = s.report.as_mut().unwrap();
        let vis = r.pages[0].visual_mut(&v).unwrap();
        vis.visual_extra["objects"]["referenceLine"][0]["properties"]["dataLabelColor"] =
            json!({"solid": {"color": lit("'#123456'")}});
    }
    let props = extract_props(visual(&s, &v));
    assert_eq!(props["analytics"][0]["object"], "referenceLine", "{props}");
    assert_extract_noop(&s, visual(&s, &v));
    // null removes every line.
    run(
        &mut s,
        "visual.analytics",
        json!({"page": pg, "visual": v, "lines": null}),
    );
    assert!(objects(&s, &v).get("referenceLine").is_none());
}

#[test]
fn analytics_forecast_and_errors() {
    let mut s = session();
    let col = chart(&mut s, "clusteredColumnChart");
    let pg = page(&s);
    let e = fails(
        &mut s,
        "visual.analytics",
        json!({"page": pg, "visual": col, "lines": [{"kind": "forecast"}]}),
    );
    assert!(e.contains("forecast"), "{e}");
    fails(
        &mut s,
        "visual.analytics",
        json!({"page": pg, "visual": col, "lines": [{"kind": "constant"}]}),
    );
    fails(
        &mut s,
        "visual.analytics",
        json!({"page": pg, "visual": col, "lines": [{"kind": "average", "measure": "Product[Name]"}]}),
    );
    fails(
        &mut s,
        "visual.analytics",
        json!({"page": pg, "visual": col, "lines": [{"kind": "percentile"}]}),
    );
    fails(
        &mut s,
        "visual.analytics",
        json!({"page": pg, "visual": col, "lines": [{"kind": "trend"}, {"kind": "trend"}]}),
    );
    fails(
        &mut s,
        "visual.analytics",
        json!({"page": pg, "visual": col, "lines": [{"kind": "error_bars"}]}),
    );
    fails(
        &mut s,
        "visual.analytics",
        json!({"page": pg, "visual": col, "lines": [{"kind": "constant", "value": 1, "measure": "[Total]"}]}),
    );
    let card = add(&mut s, "card", json!([{"role": "Values", "field": "[Total]"}]));
    fails(
        &mut s,
        "visual.analytics",
        json!({"page": pg, "visual": card, "lines": [{"kind": "constant", "value": 1}]}),
    );

    let line = chart(&mut s, "lineChart");
    let params =
        json!({"page": pg, "visual": line, "lines": [{"kind": "forecast", "length": 6, "confidence": 0.9}]});
    run(&mut s, "visual.analytics", params.clone());
    let f = &objects(&s, &line)["forecast"][0];
    assert_eq!(f["selector"], json!({"metadata": "Sales.Total", "id": "1"}));
    assert_eq!(f["properties"]["transform"]["algorithm"], "Forecast");
    assert_eq!(
        f["properties"]["transform"]["parameters"][1],
        json!({"Literal": {"Value": "6D"}, "Name": "ForecastLength"})
    );
    idempotent(&mut s, "visual.analytics", params);
    assert_extract_noop(&s, visual(&s, &line));
    assert_pbir_ok(&mut s);
}

// ---------------------------------------------------------------------------------------------
// Small multiples
// ---------------------------------------------------------------------------------------------

#[test]
fn small_multiples_bind_rows_and_layout() {
    let mut s = session();
    let v = chart(&mut s, "clusteredColumnChart");
    let pg = page(&s);
    let params = json!({"page": pg, "visual": v, "field": "Product[Name]", "rows": 2, "columns": 3,
                        "title": {"show": true, "position": "bottom"}});
    let out = run(&mut s, "visual.small_multiples", params.clone());
    assert_eq!(out["changed"], true);
    let vis = visual(&s, &v);
    assert!(vis.fields.iter().any(|b| b.role == "Rows"), "{:?}", vis.fields);
    let o = objects(&s, &v);
    assert_eq!(
        o["smallMultiplesLayout"][0]["properties"]["layoutType"],
        lit("'custom'")
    );
    assert_eq!(
        o["smallMultiplesLayout"][0]["properties"]["rowCount"]["expr"]["Literal"]["Value"]
            .as_str()
            .unwrap()
            .trim_end_matches(['D', 'L']),
        "2"
    );
    assert_eq!(o["subheader"][0]["properties"]["position"], lit("'bottom'"));
    assert_eq!(
        out["props"]["small_multiples"],
        json!({"field": "Product[Name]", "rows": 2, "columns": 3, "title": {"show": true, "position": "bottom"}})
    );
    idempotent(&mut s, "visual.small_multiples", params);
    assert_extract_noop(&s, visual(&s, &v));
    assert_pbir_ok(&mut s);
    // The query has the Rows projection.
    let q = bicraft_report::query::effective(visual(&s, &v)).unwrap();
    assert_eq!(
        q["queryState"]["Rows"]["projections"][0]["queryRef"],
        "Product.Name"
    );
    fails(
        &mut s,
        "visual.small_multiples",
        json!({"page": pg, "visual": v, "field": "[Total]"}),
    );
    fails(
        &mut s,
        "visual.small_multiples",
        json!({"page": pg, "visual": v, "rows": 9}),
    );
    let card = add(&mut s, "card", json!([{"role": "Values", "field": "[Total]"}]));
    fails(
        &mut s,
        "visual.small_multiples",
        json!({"page": pg, "visual": card, "field": "Product[Name]"}),
    );
    // field: null removes everything.
    run(
        &mut s,
        "visual.small_multiples",
        json!({"page": pg, "visual": v, "field": null}),
    );
    assert!(!visual(&s, &v).fields.iter().any(|b| b.role == "Rows"));
    let o = objects(&s, &v);
    assert!(
        o.get("smallMultiplesLayout").is_none() && o.get("subheader").is_none(),
        "{o}"
    );
}

// ---------------------------------------------------------------------------------------------
// Tooltip and drillthrough pages
// ---------------------------------------------------------------------------------------------

#[test]
fn tooltip_page_and_visual_tooltip() {
    let mut s = session();
    let v = chart(&mut s, "clusteredColumnChart");
    let pg = page(&s);
    let tip = run(&mut s, "page.add", json!({"display_name": "Tip"}))["page"]
        .as_str()
        .unwrap()
        .to_string();
    // Not a tooltip page yet: works, with a warning.
    let out = run(
        &mut s,
        "visual.tooltip_page",
        json!({"page": pg, "visual": v, "tooltip_page": "Tip"}),
    );
    assert_eq!(out["warnings"].as_array().map(Vec::len), Some(1), "{out}");
    let params = json!({"page": "Tip", "type": "tooltip", "hidden": true});
    let out = run(&mut s, "page.set_type", params.clone());
    assert_eq!(out["changed"], true, "{out}");
    let p = page_of(&s, &tip);
    assert_eq!((p.width, p.height), (320.0, 240.0));
    assert_eq!(p.extra["type"], "Tooltip");
    assert_eq!(p.extra["pageBinding"]["type"], "Tooltip");
    assert!(p.extra["pageBinding"]["name"].is_string());
    idempotent(&mut s, "page.set_type", params);
    let out = run(
        &mut s,
        "visual.tooltip_page",
        json!({"page": pg, "visual": v, "tooltip_page": "Tip"}),
    );
    assert_eq!(out["changed"], false);
    assert!(out.get("warnings").is_none_or(|w| w == &json!([])), "{out}");
    let vco = visual(&s, &v).visual_extra["visualContainerObjects"]["visualTooltip"][0]["properties"].clone();
    assert_eq!(vco["type"], lit("'Canvas'"));
    assert_eq!(vco["section"], lit(&format!("'{tip}'")));
    assert_eq!(out["props"]["tooltip_page"], json!(tip));
    assert_extract_noop(&s, visual(&s, &v));
    assert_pbir_ok(&mut s);
    run(
        &mut s,
        "visual.tooltip_page",
        json!({"page": pg, "visual": v, "tooltip_page": "none"}),
    );
    assert_eq!(extract_props(visual(&s, &v))["tooltip_page"], "none");
    run(
        &mut s,
        "visual.tooltip_page",
        json!({"page": pg, "visual": v, "tooltip_page": "default"}),
    );
    assert!(
        visual(&s, &v)
            .visual_extra
            .get("visualContainerObjects")
            .is_none()
    );
    fails(
        &mut s,
        "visual.tooltip_page",
        json!({"page": pg, "visual": v, "tooltip_page": "Nope"}),
    );
    // Back to a report page.
    run(&mut s, "page.set_type", json!({"page": "Tip", "type": "report"}));
    let p = page_of(&s, &tip);
    assert!(p.extra.get("type").is_none() && p.extra.get("pageBinding").is_none());
}

#[test]
fn drillthrough_page_fields_and_back_button() {
    let mut s = session();
    chart(&mut s, "clusteredColumnChart");
    let dt = run(&mut s, "page.add", json!({"display_name": "Detail"}))["page"]
        .as_str()
        .unwrap()
        .to_string();
    let params = json!({"page": "Detail", "type": "drillthrough", "fields": ["Product[Name]", "Sum(Sales[Amount])"],
                        "keep_all_filters": false, "back_button": true});
    let out = run(&mut s, "page.set_type", params.clone());
    assert_eq!(out["changed"], true, "{out}");
    assert_eq!(out["fields"], json!(["Product[Name]", "Sum(Sales[Amount])"]));
    let p = page_of(&s, &dt).clone();
    assert_eq!(p.extra["type"], "Drillthrough");
    let b = &p.extra["pageBinding"];
    assert_eq!(b["type"], "Drillthrough");
    assert_eq!(b["acceptsFilterContext"], "None");
    let filters = p.filter_config.as_ref().unwrap()["filters"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(filters.len(), 2);
    assert_eq!(filters[0]["howCreated"], "Drillthrough");
    assert_eq!(b["parameters"][0]["boundFilter"], filters[0]["name"]);
    assert_eq!(b["parameters"][1]["asAggregation"], true);
    assert_eq!(
        p.visuals
            .iter()
            .filter(|v| v.visual_type.as_deref() == Some("actionButton"))
            .count(),
        1
    );
    idempotent(&mut s, "page.set_type", params);
    assert_pbir_ok(&mut s);
    // Drop one field: its filter and parameter go, the other keeps its names.
    run(
        &mut s,
        "page.set_type",
        json!({"page": "Detail", "type": "drillthrough", "fields": ["Product[Name]"]}),
    );
    let q = page_of(&s, &dt);
    assert_eq!(
        q.filter_config.as_ref().unwrap()["filters"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        q.filter_config.as_ref().unwrap()["filters"][0]["name"],
        filters[0]["name"]
    );
    fails(
        &mut s,
        "page.set_type",
        json!({"page": "Detail", "type": "drillthrough", "fields": ["[Total]"]}),
    );
    fails(
        &mut s,
        "page.set_type",
        json!({"page": "Detail", "type": "report", "fields": ["Product[Name]"]}),
    );
    fails(
        &mut s,
        "page.set_type",
        json!({"page": "Detail", "type": "tooltip", "cross_report": true}),
    );
    fails(
        &mut s,
        "page.set_type",
        json!({"page": "Detail", "type": "popup"}),
    );
    // Back to report: binding, type and drillthrough filters go; back_button false removes it.
    run(
        &mut s,
        "page.set_type",
        json!({"page": "Detail", "type": "report", "back_button": false}),
    );
    let q = page_of(&s, &dt);
    assert!(
        q.extra.get("pageBinding").is_none() && q.filter_config.is_none(),
        "{q:?}"
    );
    assert!(q.visuals.is_empty());
}

// ---------------------------------------------------------------------------------------------
// Interactions, order, mobile
// ---------------------------------------------------------------------------------------------

#[test]
fn interactions_merge_and_remove() {
    let mut s = session();
    let a = chart(&mut s, "clusteredColumnChart");
    let b = chart(&mut s, "lineChart");
    let pg = page(&s);
    let params = json!({"page": pg, "rules": [{"source": a, "target": b, "type": "none"},
                                              {"source": b, "target": a, "type": "filter"}]});
    let out = run(&mut s, "page.interactions", params.clone());
    assert_eq!(out["changed"], true);
    assert_eq!(
        page_of(&s, &pg).extra["visualInteractions"],
        json!([{"source": a, "target": b, "type": "NoFilter"}, {"source": b, "target": a, "type": "DataFilter"}])
    );
    idempotent(&mut s, "page.interactions", params);
    assert_pbir_ok(&mut s);
    run(
        &mut s,
        "page.interactions",
        json!({"page": pg, "rules": [{"source": b, "target": a, "type": "default"}]}),
    );
    assert_eq!(
        page_of(&s, &pg).extra["visualInteractions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    fails(
        &mut s,
        "page.interactions",
        json!({"page": pg, "rules": [{"source": a, "target": a, "type": "none"}]}),
    );
    fails(
        &mut s,
        "page.interactions",
        json!({"page": pg, "rules": [{"source": a, "target": "zzz", "type": "none"}]}),
    );
    fails(
        &mut s,
        "page.interactions",
        json!({"page": pg, "rules": [{"source": a, "target": b, "type": "blink"}]}),
    );
    run(&mut s, "page.interactions", json!({"page": pg, "clear": true}));
    assert!(page_of(&s, &pg).extra.get("visualInteractions").is_none());
}

#[test]
fn a_group_as_source_or_target_sets_a_rule_per_member() {
    let mut s = session();
    let a = chart(&mut s, "clusteredColumnChart");
    let b = chart(&mut s, "lineChart");
    let c = add(&mut s, "card", json!([{"role": "Values", "field": "[Total]"}]));
    let d = chart(&mut s, "pieChart");
    let pg = page(&s);
    let group = |s: &mut Session, members: Value| {
        run(s, "visual.group", json!({"page": pg, "visuals": members}))["group"]
            .as_str()
            .unwrap()
            .to_string()
    };
    // inner = {a, b}; outer = {inner, c}; d stays outside
    let inner = group(&mut s, json!([a, b]));
    let outer = group(&mut s, json!([inner, c]));
    let rules = |s: &Session| -> Vec<(String, String, String)> {
        let mut v: Vec<_> = page_of(s, &pg)
            .extra
            .get("visualInteractions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|r| {
                (
                    r["source"].as_str().unwrap().to_string(),
                    r["target"].as_str().unwrap().to_string(),
                    r["type"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        v.sort();
        v
    };
    let triple = |x: &str, y: &str, t: &str| (x.to_string(), y.to_string(), t.to_string());
    // a group as the source: every visual of it (nested groups too) reaches the target
    let params = json!({"page": pg, "rules": [{"source": outer, "target": d, "type": "none"}]});
    let out = run(&mut s, "page.interactions", params.clone());
    assert_eq!(out["changed"], true);
    let mut want = vec![
        triple(&a, &d, "NoFilter"),
        triple(&b, &d, "NoFilter"),
        triple(&c, &d, "NoFilter"),
    ];
    want.sort();
    assert_eq!(rules(&s), want);
    idempotent(&mut s, "page.interactions", params);
    // a group as the target, a later visual rule overriding one member
    run(
        &mut s,
        "page.interactions",
        json!({"page": pg, "clear": true, "rules": [
            {"source": d, "target": inner, "type": "filter"},
            {"source": d, "target": b, "type": "highlight"}]}),
    );
    let mut want = vec![triple(&d, &a, "DataFilter"), triple(&d, &b, "HighlightFilter")];
    want.sort();
    assert_eq!(rules(&s), want);
    // a group to a group: every pair across them, none inside one visual
    run(
        &mut s,
        "page.interactions",
        json!({"page": pg, "clear": true, "rules": [{"source": inner, "target": c, "type": "none"}]}),
    );
    assert_eq!(rules(&s).len(), 2);
    // the default removes every member's rule
    run(
        &mut s,
        "page.interactions",
        json!({"page": pg, "rules": [{"source": inner, "target": c, "type": "default"}]}),
    );
    assert!(page_of(&s, &pg).extra.get("visualInteractions").is_none());
    assert_pbir_ok(&mut s);
    // a group and one of its own members have no pair between them
    let err = fails(
        &mut s,
        "page.interactions",
        json!({"page": pg, "rules": [{"source": outer, "target": a, "type": "none"}]}),
    );
    assert!(err.contains("no pair"), "{err}");
    fails(
        &mut s,
        "page.interactions",
        json!({"page": pg, "rules": [{"source": outer, "target": outer, "type": "none"}]}),
    );
}

#[test]
fn layer_order_moves_z_and_tab_order_together() {
    let mut s = session();
    let a = chart(&mut s, "clusteredColumnChart");
    let b = chart(&mut s, "lineChart");
    let c = add(&mut s, "card", json!([{"role": "Values", "field": "[Total]"}]));
    let pg = page(&s);
    let z = |s: &Session, n: &str| visual(s, n).position.z;
    let tab = |s: &Session, n: &str| visual(s, n).position.tab_order;
    assert!(z(&s, &c) > z(&s, &b) && z(&s, &b) > z(&s, &a));
    let params = json!({"page": pg, "visuals": [a, c, b]});
    let out = run(&mut s, "visual.order", params.clone());
    assert_eq!(out["front_to_back"], json!([a, c, b]));
    assert!(z(&s, &a) > z(&s, &c) && z(&s, &c) > z(&s, &b));
    assert!(tab(&s, &a) > tab(&s, &c) && tab(&s, &c) > tab(&s, &b));
    idempotent(&mut s, "visual.order", params);
    run(
        &mut s,
        "visual.order",
        json!({"page": pg, "visual": a, "to": "back"}),
    );
    assert!(z(&s, &a) < z(&s, &b));
    run(
        &mut s,
        "visual.order",
        json!({"page": pg, "visual": a, "to": "forward"}),
    );
    let out = run(
        &mut s,
        "visual.order",
        json!({"page": pg, "visual": c, "to": "front"}),
    );
    assert_eq!(out["front_to_back"][0], json!(c));
    fails(&mut s, "visual.order", json!({"page": pg, "visuals": [a, a]}));
    fails(
        &mut s,
        "visual.order",
        json!({"page": pg, "visual": a, "to": "up"}),
    );
    assert_pbir_ok(&mut s);
}

#[test]
fn selection_pane_hides_shows_renames_groups_and_sets_tab_order() {
    let mut s = session();
    let a = chart(&mut s, "clusteredColumnChart");
    let b = chart(&mut s, "lineChart");
    let pg = page(&s);
    let hidden = |s: &Session, n: &str| visual(s, n).is_hidden;
    let tab = |s: &Session, n: &str| visual(s, n).position.tab_order;

    // The eye: hide and show several visuals in one command, idempotent.
    let out = run(
        &mut s,
        "visual.set_hidden",
        json!({"page": pg, "visuals": [a, b], "hidden": true}),
    );
    assert_eq!(out["changed"], json!(true));
    assert_eq!(out["hidden"], json!(true));
    assert!(hidden(&s, &a) && hidden(&s, &b));
    idempotent(
        &mut s,
        "visual.set_hidden",
        json!({"page": pg, "visuals": [a, b], "hidden": true}),
    );
    run(
        &mut s,
        "visual.set_hidden",
        json!({"page": pg, "visual": a, "hidden": false}),
    );
    assert!(!hidden(&s, &a) && hidden(&s, &b));
    fails(&mut s, "visual.set_hidden", json!({"page": pg, "visual": a}));
    fails(
        &mut s,
        "visual.set_hidden",
        json!({"page": pg, "visual": "nope", "hidden": true}),
    );
    fails(
        &mut s,
        "visual.set_hidden",
        json!({"page": pg, "visual": a, "hidden": "yes"}),
    );
    fails(&mut s, "visual.set_hidden", json!({"page": pg, "hidden": true}));

    // A group's name is its display name; a plain visual has none.
    let g = run(
        &mut s,
        "visual.group",
        json!({"page": pg, "visuals": [a, b], "display_name": "Pair"}),
    )["group"]
        .as_str()
        .unwrap()
        .to_string();
    let out = run(
        &mut s,
        "visual.rename_group",
        json!({"page": pg, "group": g, "display_name": "Two"}),
    );
    assert_eq!(out["changed"], json!(true));
    assert_eq!(visual(&s, &g).extra["visualGroup"]["displayName"], "Two");
    idempotent(
        &mut s,
        "visual.rename_group",
        json!({"page": pg, "group": g, "display_name": "Two"}),
    );
    let err = fails(
        &mut s,
        "visual.rename_group",
        json!({"page": pg, "group": g, "display_name": "  "}),
    );
    assert!(err.contains("display_name"), "{err}");
    let err = fails(
        &mut s,
        "visual.rename_group",
        json!({"page": pg, "group": a, "display_name": "Not a group"}),
    );
    assert!(err.contains("not a group"), "{err}");

    // Tab order: the listed visuals in that order, 0, 1000, 2000…
    let out = run(
        &mut s,
        "visual.tab_order",
        json!({"page": pg, "visuals": [b, g, a]}),
    );
    assert_eq!(out["changed"], json!(true));
    assert_eq!(
        out["tab_order"],
        json!([{"visual": b, "tab_order": 0}, {"visual": g, "tab_order": 1000},
               {"visual": a, "tab_order": 2000}])
    );
    assert_eq!(tab(&s, &b), Some(0));
    assert_eq!(tab(&s, &g), Some(1000));
    assert_eq!(tab(&s, &a), Some(2000));
    idempotent(
        &mut s,
        "visual.tab_order",
        json!({"page": pg, "visuals": [b, g, a]}),
    );
    fails(&mut s, "visual.tab_order", json!({"page": pg, "visuals": [a, a]}));
    fails(&mut s, "visual.tab_order", json!({"page": pg, "visuals": []}));
    fails(
        &mut s,
        "visual.tab_order",
        json!({"page": pg, "visuals": ["nope"]}),
    );
    fails(&mut s, "visual.tab_order", json!({"page": pg, "visuals": a}));
    assert_pbir_ok(&mut s);
}

#[test]
fn mobile_layout_document() {
    let mut s = session();
    let v = chart(&mut s, "clusteredColumnChart");
    let pg = page(&s);
    fails(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visual": v, "x": 0, "y": 0}),
    );
    let params = json!({"page": pg, "visual": v, "x": 0, "y": 10, "width": 320, "height": 200});
    let out = run(&mut s, "visual.mobile", params.clone());
    assert_eq!(out["changed"], true);
    assert_eq!(
        out["mobile"],
        json!({"x": 0, "y": 10, "width": 320, "height": 200})
    );
    let path = format!("definition/pages/{pg}/visuals/{v}/mobile.json");
    let res = report(&s).resources.iter().find(|r| r.path == path).unwrap();
    let doc: Value = serde_json::from_slice(&res.bytes).unwrap();
    assert!(
        doc["$schema"]
            .as_str()
            .unwrap()
            .contains("visualContainerMobileState")
    );
    idempotent(&mut s, "visual.mobile", params);
    assert_pbir_ok(&mut s);
    run(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visual": v, "height": 150}),
    );
    assert_eq!(
        crate::visual_extras::mobile_position(report(&s), &pg, &v).unwrap()["height"],
        150
    );
    run(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visual": v, "layout": null}),
    );
    assert!(!report(&s).resources.iter().any(|r| r.path == path));
}

#[test]
fn mobile_layout_snaps_to_desktops_phone_grid() {
    // Power BI Desktop 2.159.1231.0's phone canvas (2026-11-23): 324 wide, grid cells 18 wide by
    // 20 high (docs/gaps.md G72)
    let mut s = session();
    let v = chart(&mut s, "clusteredColumnChart");
    let pg = page(&s);
    let out = run(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visual": v, "x": 10, "y": 31, "width": 300, "height": 107, "snap": true}),
    );
    assert_eq!(
        out["mobile"],
        json!({"x": 18, "y": 40, "width": 306, "height": 100})
    );
    // kept on the phone's width: a moved visual stops at its right edge
    let out = run(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visual": v, "x": 250, "width": 100, "snap": true}),
    );
    assert_eq!(
        out["mobile"],
        json!({"x": 216, "y": 40, "width": 108, "height": 100})
    );
    // at least one cell, at most the phone's width
    let out = run(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visuals": [{"visual": v, "x": 5, "width": 400, "height": 3}], "snap": true}),
    );
    assert_eq!(
        out["visuals"][0]["mobile"],
        json!({"x": 0, "y": 40, "width": 324, "height": 20})
    );
    // without snap the values are kept as given
    let out = run(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visual": v, "x": 3, "width": 101}),
    );
    assert_eq!(
        out["mobile"],
        json!({"x": 3, "y": 40, "width": 101, "height": 20})
    );
    fails(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visual": v, "x": 0, "snap": "yes"}),
    );
    fails(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visual": v, "layout": null, "snap": true}),
    );
    assert_pbir_ok(&mut s);
}

/// The z of a visual's mobile layout.
fn mobile_z(s: &Session, pg: &str, v: &str) -> f64 {
    crate::visual_extras::mobile_doc(report(s), pg, v).unwrap()["position"]["z"]
        .as_f64()
        .unwrap()
}

#[test]
fn mobile_layout_list_is_one_step_and_new_visuals_go_on_top() {
    let mut s = session();
    let a = chart(&mut s, "clusteredColumnChart");
    let b = chart(&mut s, "lineChart");
    let pg = page(&s);
    run(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visual": a, "x": 0, "y": 0, "width": 320, "height": 200}),
    );
    run(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visual": b, "x": 0, "y": 200, "width": 320, "height": 200}),
    );
    // each placed visual is its own layer, on top of the others
    assert!(mobile_z(&s, &pg, &b) > mobile_z(&s, &pg, &a));
    let out = run(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visuals": [{"visual": a, "x": 8}, {"visual": b, "y": 216, "height": 120}]}),
    );
    assert_eq!(out["changed"], true, "{out}");
    assert_eq!(out["visuals"][1]["mobile"]["height"], 120);
    assert_eq!(
        crate::visual_extras::mobile_position(report(&s), &pg, &a).unwrap()["x"],
        8
    );
    idempotent(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visuals": [{"visual": a, "x": 8}, {"visual": b, "y": 216}]}),
    );
    // every item is checked first: a bad one leaves the others alone
    fails(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visuals": [{"visual": a, "x": 40}, {"visual": "nope", "x": 0}]}),
    );
    fails(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visuals": [{"visual": a, "x": 40}, {"visual": a, "x": 0}]}),
    );
    fails(&mut s, "visual.mobile", json!({"page": pg, "visuals": []}));
    fails(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visual": a, "visuals": [{"visual": a, "x": 0}]}),
    );
    fails(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "x": 0, "visuals": [{"visual": a, "x": 0}]}),
    );
    // removing both in one step
    run(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visuals": [{"visual": a, "layout": null}, {"visual": b, "layout": null}]}),
    );
    assert!(crate::visual_extras::mobile_doc(report(&s), &pg, &a).is_none());
    assert!(crate::visual_extras::mobile_doc(report(&s), &pg, &b).is_none());
    assert_pbir_ok(&mut s);
}

#[test]
fn mobile_format_overrides_the_visuals_own_on_the_phone_only() {
    let mut s = session();
    let v = chart(&mut s, "clusteredColumnChart");
    let pg = page(&s);
    run(
        &mut s,
        "visual.set_title",
        json!({"page": pg, "visual": v, "title": {"text": "Sales", "font_size": 16}}),
    );
    // not on the phone canvas yet: refused
    fails(
        &mut s,
        "visual.mobile_format",
        json!({"page": pg, "visual": v, "title": {"font_size": 9}}),
    );
    run(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visual": v, "x": 0, "y": 0, "width": 320, "height": 200}),
    );
    let params = json!({"page": pg, "visual": v, "title": {"font_size": 9},
                        "background": {"color": "#FFEEDD"}});
    let out = run(&mut s, "visual.mobile_format", params.clone());
    assert_eq!(out["changed"], true, "{out}");
    // the answer and visual.props layout mobile read the merged props: the text stays the
    // visual's, the size is the phone's
    assert_eq!(out["props"]["title"]["font_size"], 9.0, "{out}");
    assert_eq!(out["props"]["title"]["text"], "Sales", "{out}");
    let mobile = run(
        &mut s,
        "visual.props",
        json!({"page": pg, "visual": v, "layout": "mobile"}),
    );
    assert_eq!(mobile["props"]["title"]["font_size"], 9.0, "{mobile}");
    assert_eq!(
        mobile["mobile_objects"],
        json!(["title", "background"]),
        "{mobile}"
    );
    // the desktop layout keeps its own
    let desk = run(&mut s, "visual.props", json!({"page": pg, "visual": v}));
    assert_eq!(desk["props"]["title"]["font_size"], 16.0, "{desk}");
    assert!(desk["props"]["format"].get("background").is_none(), "{desk}");
    // the document holds the overrides only
    let doc = crate::visual_extras::mobile_doc(report(&s), &pg, &v).unwrap();
    let title = &doc["visualContainerObjects"]["title"][0]["properties"];
    assert!(title.get("fontSize").is_some(), "{doc}");
    assert!(title.get("text").is_none(), "{doc}");
    assert_eq!(doc["position"]["width"], 320);
    idempotent(&mut s, "visual.mobile_format", params);
    assert_pbir_ok(&mut s);
    // the snapshot carries the phone's place and format
    let snap = s.snapshot();
    let entry = snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == v.as_str())
        .unwrap()
        .clone();
    assert_eq!(entry["mobile"]["width"], 320.0, "{entry}");
    assert_eq!(entry["mobile"]["formatted"], true, "{entry}");
    assert_eq!(entry["mobile"]["format"]["title"]["font_size"], 9.0, "{entry}");
    assert_eq!(entry["format"]["title"]["font_size"], 16.0, "{entry}");
    // only formatting: fields and sorts stay the visual's
    fails(
        &mut s,
        "visual.mobile_format",
        json!({"page": pg, "visual": v, "sort": {"field": "[Total]"}}),
    );
    fails(
        &mut s,
        "visual.mobile_format",
        json!({"page": pg, "visual": v, "clear": true, "title": {"font_size": 9}}),
    );
    fails(&mut s, "visual.mobile_format", json!({"page": pg, "visual": v}));
    // null clears one mobile property; clear: true every one
    run(
        &mut s,
        "visual.mobile_format",
        json!({"page": pg, "visual": v, "background": null}),
    );
    let doc = crate::visual_extras::mobile_doc(report(&s), &pg, &v).unwrap();
    assert!(doc["visualContainerObjects"].get("background").is_none(), "{doc}");
    let out = run(
        &mut s,
        "visual.mobile_format",
        json!({"page": pg, "visual": v, "clear": true}),
    );
    assert_eq!(out["changed"], true);
    assert_eq!(out["props"]["title"]["font_size"], 16.0, "{out}");
    let doc = crate::visual_extras::mobile_doc(report(&s), &pg, &v).unwrap();
    assert!(doc.get("visualContainerObjects").is_none(), "{doc}");
    assert!(doc.get("objects").is_none(), "{doc}");
    assert_pbir_ok(&mut s);
}

// ---------------------------------------------------------------------------------------------
// report.apply
// ---------------------------------------------------------------------------------------------

#[test]
fn report_apply_props_round_trip() {
    let mut s = session();
    let spec = json!({"pages": [{"key": "p1", "display_name": "Main", "visuals": [{
        "key": "v1", "type": "lineChart", "x": 0, "y": 0, "width": 600, "height": 400,
        "fields": [{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}],
        "props": {
            "axes": {"y": {"scale": "log", "min": 1, "max": 1000}},
            "analytics": [{"kind": "constant", "value": 50}, {"kind": "average"}, {"kind": "trend"}],
            "small_multiples": {"field": "Product[Name]", "rows": 2, "columns": 2},
            "tooltip_page": "abc"
        }
    }]}]});
    let out = run(&mut s, "report.apply", json!({"spec": spec}));
    assert!(out["change_count"].as_u64().unwrap() > 0, "{out}");
    let before = s.report.clone().unwrap();
    let again = run(&mut s, "report.apply", json!({"spec": spec}));
    if again["change_count"] != 0 {
        let after = s.report.clone().unwrap();
        let (a, b) = (
            &before.pages.last().unwrap().visuals[0],
            &after.pages.last().unwrap().visuals[0],
        );
        panic!(
            "{}
{}
{:?}
{:?}",
            serde_json::to_string(&a.visual_extra).unwrap(),
            serde_json::to_string(&b.visual_extra).unwrap(),
            a.fields,
            b.fields
        );
    }
    let extracted = run(&mut s, "report.extract", json!({}));
    let again = run(&mut s, "report.apply", json!({"spec": extracted}));
    assert_eq!(again["change_count"], 0, "{again}");
}

/// The shape Power BI Desktop 2.158 writes when its forecast switch is turned on (re-saved
/// report), next to an entry the bicraft of the time wrote: both survive extract → apply.
#[test]
fn analytics_desktop_forecast_entry_round_trips() {
    let mut s = session();
    let line = chart(&mut s, "lineChart");
    let desktop = json!({"properties": {
        "show": lit("true"),
        "displayName": lit("'Forecast 1'"),
        "transform": {"algorithm": "Forecast", "parameters": [
            {"Literal": {"Value": "7D"}, "Name": "Unit"},
            {"Literal": {"Value": "10D"}, "Name": "ForecastLength"},
            {"Literal": {"Value": "0D"}, "Name": "IgnoreLast"},
            {"Literal": {"Value": "0.95D"}, "Name": "ConfidenceLevel"}]}},
        "selector": {"metadata": "Sales.Total", "id": "2"}});
    let stale = json!({"properties": {"show": lit("true"), "transform": {"algorithm": "Forecast",
        "parameters": [{"Literal": {"Value": "7L"}, "Name": "Unit"}]}}, "selector": {"id": "1"}});
    {
        let r = s.report.as_mut().unwrap();
        let v = r.pages[0].visual_mut(&line).unwrap();
        v.visual_extra
            .insert("objects".into(), json!({"forecast": [stale, desktop]}));
    }
    let props = extract_props(visual(&s, &line));
    assert_eq!(props["analytics"][0]["object"], "forecast", "{props}");
    assert_eq!(
        props["analytics"][1],
        json!({"kind": "forecast", "measure": "[Total]", "unit": 7, "length": 10, "ignore_last": 0,
               "confidence": 0.95, "label": "Forecast 1"})
    );
    assert_extract_noop(&s, visual(&s, &line));
}

#[test]
fn matrix_rows_are_not_small_multiples() {
    // `Rows` is the row role of a matrix: extracting and re-applying its props must not try
    // to set small multiples (regression: real matrices failed `report.extract` → apply).
    let mut s = session();
    let name = add(
        &mut s,
        "pivotTable",
        json!([{"role": "Rows", "field": "Sales[Region]"}, {"role": "Values", "field": "[Total]"}]),
    );
    let v = visual(&s, &name).clone();
    assert!(extract_props(&v).get("small_multiples").is_none());
    assert_extract_noop(&s, &v);
}

/// A visual of `visual_type` at (`x`, `y`), `w` × `h` on the desktop page.
fn add_at(s: &mut Session, visual_type: &str, fields: Value, rect: (i64, i64, i64, i64)) -> String {
    let page = page(s);
    let (x, y, w, h) = rect;
    let v = run(
        s,
        "visual.add",
        json!({"page": page, "visual_type": visual_type, "x": x, "y": y, "width": w,
               "height": h, "fields": fields}),
    );
    v["visual"].as_str().unwrap().to_string()
}

/// The phone rectangle (x, y, width, height) of visual `v`, when it is on the mobile layout.
fn phone_rect(s: &Session, pg: &str, v: &str) -> Option<(f64, f64, f64, f64)> {
    let m = crate::visual_extras::mobile_position(report(s), pg, v)?;
    let n = |k: &str| m[k].as_f64().unwrap();
    Some((n("x"), n("y"), n("width"), n("height")))
}

fn overlap(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// A page laid out as a dashboard: two cards on top, a wide chart, a slicer and a table side by
/// side under it, a hidden line chart; returns (cards, chart, slicer, table, hidden).
fn dashboard(s: &mut Session) -> (Vec<String>, String, String, String, String) {
    let total = json!([{"role": "Values", "field": "[Total]"}]);
    let card2 = add_at(s, "card", total.clone(), (300, 10, 250, 120));
    let card1 = add_at(s, "card", total, (10, 10, 250, 120));
    let chart = add_at(
        s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]),
        (10, 150, 1200, 300),
    );
    let table = add_at(
        s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Region]"}]),
        (300, 470, 200, 900),
    );
    let slicer = add_at(
        s,
        "slicer",
        json!([{"role": "Values", "field": "Sales[Region]"}]),
        (10, 470, 250, 200),
    );
    let hidden = add_at(
        s,
        "lineChart",
        json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]),
        (600, 470, 400, 300),
    );
    let pg = page(s);
    run(
        s,
        "visual.set_hidden",
        json!({"page": pg, "visual": hidden, "hidden": true}),
    );
    (vec![card1, card2], chart, slicer, table, hidden)
}

#[test]
fn mobile_auto_create_stacks_the_shown_visuals_inside_the_phone() {
    let mut s = session();
    let (cards, chart, slicer, table, hidden) = dashboard(&mut s);
    let pg = page(&s);
    // a visual already on the phone with its own formatting keeps it
    run(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visual": hidden, "x": 0, "y": 0, "width": 100, "height": 100}),
    );
    run(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visual": chart, "x": 50, "y": 900, "width": 100, "height": 100}),
    );
    run(
        &mut s,
        "visual.mobile_format",
        json!({"page": pg, "visual": chart, "background": {"color": "#FFEEDD"}}),
    );
    let before = s.report.clone();
    let out = run(&mut s, "visual.mobile_auto", json!({"page": pg}));
    assert_eq!(out["changed"], true, "{out}");
    let shown: Vec<String> = cards.iter().chain([&chart, &slicer, &table]).cloned().collect();
    let rects: Vec<(f64, f64, f64, f64)> = shown
        .iter()
        .map(|v| phone_rect(&s, &pg, v).unwrap_or_else(|| panic!("{v} not placed: {out}")))
        .collect();
    let phone = bicraft_render::phone::WIDTH;
    for (i, a) in rects.iter().enumerate() {
        // inside the phone's width, on its grid
        assert!(a.0 >= 0.0 && a.0 + a.2 <= phone, "{a:?}");
        assert!(a.1 >= 0.0 && a.2 > 0.0 && a.3 > 0.0, "{a:?}");
        let snapped = bicraft_render::phone::snap_rect(a.0, a.1, a.2, a.3);
        assert_eq!(snapped, *a, "off the grid");
        for b in rects.iter().skip(i + 1) {
            assert!(!overlap(*a, *b), "{a:?} overlaps {b:?}");
        }
    }
    let [c1, c2, ch, sl, tb] = rects[..] else { panic!() };
    // reading order: the cards share the top row, left to right; then the chart, full width
    assert!(c1.1 == c2.1 && c1.0 < c2.0, "{c1:?} {c2:?}");
    assert!(ch.1 > c1.1 && ch.2 > c1.2 && ch.2 == phone, "{ch:?}");
    // the slicer (left on the desktop) comes before the table, alone in its row
    assert!(sl.1 > ch.1 && tb.1 > sl.1, "{sl:?} {tb:?}");
    // the wide chart keeps a wide shape, the tall table a tall one, both within limits
    assert!(ch.2 > ch.3 && tb.3 > tb.2 && tb.3 < 900.0, "{ch:?} {tb:?}");
    // the hidden visual is taken off; the chart keeps its mobile formatting
    assert!(phone_rect(&s, &pg, &hidden).is_none());
    assert_eq!(out["removed"], json!([hidden]));
    let doc = crate::visual_extras::mobile_doc(report(&s), &pg, &chart).unwrap();
    assert!(doc.get("visualContainerObjects").is_some(), "{doc}");
    idempotent(&mut s, "visual.mobile_auto", json!({"page": pg}));
    assert_pbir_ok(&mut s);
    // one undo step brings the earlier layout back
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.report, before);
    fails(&mut s, "visual.mobile_auto", json!({"page": "nope"}));
    fails(&mut s, "visual.mobile_auto", json!({"page": pg, "x": 1}));
}

#[test]
fn mobile_auto_create_needs_a_shown_visual() {
    let mut s = session();
    let pg = page(&s);
    fails(&mut s, "visual.mobile_auto", json!({"page": pg}));
}

#[test]
fn mobile_remove_all_empties_the_layout_in_one_step() {
    let mut s = session();
    let (cards, chart, ..) = dashboard(&mut s);
    let pg = page(&s);
    run(&mut s, "visual.mobile_auto", json!({"page": pg}));
    let before = s.report.clone();
    let visuals_before = report(&s).pages[0].visuals.len();
    let out = run(&mut s, "visual.mobile_remove_all", json!({"page": pg}));
    assert_eq!(out["changed"], true, "{out}");
    for v in &report(&s).pages[0].visuals {
        assert!(
            crate::visual_extras::mobile_doc(report(&s), &pg, &v.name).is_none(),
            "{} still on the phone",
            v.name
        );
    }
    assert!(
        !report(&s)
            .resources
            .iter()
            .any(|r| r.path.ends_with("mobile.json"))
    );
    // the visuals stay on the page
    assert_eq!(report(&s).pages[0].visuals.len(), visuals_before);
    idempotent(&mut s, "visual.mobile_remove_all", json!({"page": pg}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.report, before);
    assert!(phone_rect(&s, &pg, &chart).is_some() && phone_rect(&s, &pg, &cards[0]).is_some());
    fails(&mut s, "visual.mobile_remove_all", json!({"page": "nope"}));
}

#[test]
fn mobile_paste_and_duplicate_keep_the_visual_on_its_page_and_phone() {
    let mut s = session();
    let v = chart(&mut s, "clusteredColumnChart");
    let pg = page(&s);
    run(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visual": v, "x": 0, "y": 40, "width": 306, "height": 200}),
    );
    run(
        &mut s,
        "visual.mobile_format",
        json!({"page": pg, "visual": v, "background": {"color": "#FFEEDD"}}),
    );
    let copied = run(&mut s, "visual.copy", json!({"page": pg, "visual": v}));
    assert!(copied["clip"]["mobile"][&v].is_object(), "{copied}");
    let before = s.report.clone();
    let out = run(
        &mut s,
        "visual.paste",
        json!({"page": pg, "clip": copied["clip"], "mobile": true}),
    );
    let new = out["visuals"][0].as_str().unwrap().to_string();
    assert_ne!(new, v);
    // on the page and on the phone, not on top of the original, inside the phone
    assert!(page_of(&s, &pg).visual(&new).is_some());
    let (a, b) = (
        phone_rect(&s, &pg, &v).unwrap(),
        phone_rect(&s, &pg, &new).unwrap(),
    );
    assert!(!(a.0 == b.0 && a.1 == b.1), "{a:?} {b:?}");
    assert!(b.0 >= 0.0 && b.0 + b.2 <= bicraft_render::phone::WIDTH, "{b:?}");
    assert!(mobile_z(&s, &pg, &new) > mobile_z(&s, &pg, &v));
    let doc = crate::visual_extras::mobile_doc(report(&s), &pg, &new).unwrap();
    assert!(doc.get("visualContainerObjects").is_some(), "{doc}");
    assert_pbir_ok(&mut s);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.report, before);
    // duplicate on the phone: the same
    let out = run(
        &mut s,
        "visual.duplicate",
        json!({"page": pg, "visual": v, "mobile": true}),
    );
    let dup = out["visuals"][0].as_str().unwrap().to_string();
    let d = phone_rect(&s, &pg, &dup).unwrap();
    assert!(!overlap(a, d) || (a.0, a.1) != (d.0, d.1), "{a:?} {d:?}");
    assert_eq!(out["mobile"][0]["visual"], dup.as_str());
    // a paste without `mobile`, or of a visual the phone does not hold, places it under the layout
    let plain = run(
        &mut s,
        "visual.paste",
        json!({"page": pg, "clip": copied["clip"]}),
    );
    let p = plain["visuals"][0].as_str().unwrap();
    assert!(phone_rect(&s, &pg, p).is_none());
    run(&mut s, "visual.mobile_remove_all", json!({"page": pg}));
    let fresh = run(&mut s, "visual.copy", json!({"page": pg, "visual": v}));
    assert!(fresh["clip"].get("mobile").is_none(), "{fresh}");
    run(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visual": v, "x": 0, "y": 0, "width": 324, "height": 200}),
    );
    let out = run(
        &mut s,
        "visual.paste",
        json!({"page": pg, "clip": fresh["clip"], "mobile": true}),
    );
    let under = phone_rect(&s, &pg, out["visuals"][0].as_str().unwrap()).unwrap();
    assert!(
        under.1 >= 200.0 && under.2 <= bicraft_render::phone::WIDTH,
        "{under:?}"
    );
    // a hostile clip's phone place is refused before anything changes
    let mut bad = copied["clip"].clone();
    bad["mobile"][&v]["position"]["width"] = json!(-5);
    fails(
        &mut s,
        "visual.paste",
        json!({"page": pg, "clip": bad, "mobile": true}),
    );
    let mut bad = copied["clip"].clone();
    bad["mobile"] = json!([1]);
    fails(&mut s, "visual.paste", json!({"page": pg, "clip": bad}));
}
