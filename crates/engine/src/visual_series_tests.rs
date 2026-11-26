//! Tests of per-series / per-category formatting (`visual_series.rs`): selector shapes, the
//! `visual.series` command, the `series` / `colors` props keys, extract → apply idempotence.

use bicraft_report::{Report, Visual};
use serde_json::{Value, json};

use crate::Session;
use crate::visual_props::{apply_props, extract_props};
use crate::visual_series::same_json;

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

/// Sales(Amount, Qty int64, Region) with [Total] and [Margin], Product(Name), a blank report.
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
            json!({"table": "Sales", "name": "Qty", "data_type": "int64"}),
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

fn page(s: &Session) -> String {
    s.report.as_ref().unwrap().pages[0].name.clone()
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

/// A column chart: Category = Product[Name], Y = [Total], [Margin].
fn chart(s: &mut Session) -> String {
    add(
        s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Product[Name]"}, {"role": "Y", "field": "[Total]"},
               {"role": "Y", "field": "[Margin]"}]),
    )
}

fn visual<'a>(s: &'a Session, name: &str) -> &'a Visual {
    s.report
        .as_ref()
        .unwrap()
        .page(&page(s))
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

fn pbir_round_trip(r: &Report) -> Report {
    let docs = bicraft_pbir::write_documents(r);
    bicraft_pbir::read_documents(&docs).unwrap()
}

fn schema_ok(s: &mut Session) {
    let v = run(s, "report.validate", json!({}));
    assert_eq!(v["schema_issues"], json!([]), "{v}");
    assert_eq!(v["error_count"], 0, "{v}");
}

#[test]
fn colors_per_measure_write_metadata_selectors() {
    let mut s = session();
    let v = chart(&mut s);
    let pg = page(&s);
    let out = run(
        &mut s,
        "visual.series",
        json!({"page": pg, "visual": v, "colors": {"[Total]": "#118DFF", "Sales[Margin]": "#E66C37"}}),
    );
    assert_eq!(out["changed"], true, "{out}");
    assert_eq!(
        objects(&s, &v)["dataPoint"],
        json!([
            {"properties": {"fill": {"solid": {"color": {"expr": {"Literal": {"Value": "'#118DFF'"}}}}}},
             "selector": {"metadata": "Sales.Total"}},
            {"properties": {"fill": {"solid": {"color": {"expr": {"Literal": {"Value": "'#E66C37'"}}}}}},
             "selector": {"metadata": "Sales.Margin"}},
        ])
    );
    assert_eq!(
        out["props"]["series"],
        json!([{"target": "[Total]", "color": "#118DFF"}, {"target": "[Margin]", "color": "#E66C37"}])
    );
    // Same colors (other case) again: no change, report stays clean.
    s.report_dirty = false;
    let again = run(
        &mut s,
        "visual.series",
        json!({"page": pg, "visual": v, "target": "[Total]", "color": "#118dff"}),
    );
    assert_eq!(again["changed"], false, "{again}");
    assert!(!s.report_dirty);
    assert_extract_noop(&s, visual(&s, &v));
    schema_ok(&mut s);
    // null removes one color; the entry goes with it.
    run(
        &mut s,
        "visual.series",
        json!({"page": pg, "visual": v, "colors": {"[Margin]": null}}),
    );
    assert_eq!(objects(&s, &v)["dataPoint"].as_array().unwrap().len(), 1);
}

#[test]
fn category_values_write_scope_id_selectors_typed_by_column() {
    let mut s = session();
    let v = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let pg = page(&s);
    run(
        &mut s,
        "visual.series",
        json!({"page": pg, "visual": v, "series": [
            {"target": "Sales[Region]=North", "color": "#E66C37"},
            {"target": "Sales[Qty]=3", "color": "#111111"},
            {"target": {"field": "Sales[Region]", "value": null}, "color": "#222222"},
            {"target": "Sales[Region]=a=b's", "color": "#333333"},
        ]}),
    );
    let dp = objects(&s, &v)["dataPoint"].clone();
    assert_eq!(
        dp[0]["selector"],
        json!({"data": [{"scopeId": {"Comparison": {"ComparisonKind": 0,
            "Left": {"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Region"}},
            "Right": {"Literal": {"Value": "'North'"}}}}}]})
    );
    assert_eq!(
        dp[1]["selector"]["data"][0]["scopeId"]["Comparison"]["Right"]["Literal"]["Value"],
        "3L"
    );
    assert_eq!(
        dp[2]["selector"]["data"][0]["scopeId"]["Comparison"]["Right"]["Literal"]["Value"],
        "null"
    );
    assert_eq!(
        dp[3]["selector"]["data"][0]["scopeId"]["Comparison"]["Right"]["Literal"]["Value"],
        "'a=b''s'"
    );
    let props = extract_props(visual(&s, &v));
    assert_eq!(
        props["series"],
        json!([
            {"target": "Sales[Region]=North", "color": "#E66C37"},
            {"target": {"field": "Sales[Qty]", "value": 3}, "color": "#111111"},
            {"target": {"field": "Sales[Region]", "value": null}, "color": "#222222"},
            {"target": "Sales[Region]=a=b's", "color": "#333333"},
        ])
    );
    assert_extract_noop(&s, visual(&s, &v));
    // PBIR round trip keeps the selectors.
    let back = pbir_round_trip(s.report.as_ref().unwrap());
    assert_eq!(
        back.page(&page(&s)).unwrap().visual(&v).unwrap().visual_extra["objects"],
        objects(&s, &v)
    );
    schema_ok(&mut s);
    // A typed column refuses a value of another type.
    let e = fails(
        &mut s,
        "visual.series",
        json!({"page": pg, "visual": v, "target": "Sales[Qty]=many", "color": "#000000"}),
    );
    assert!(e.contains("not a number"), "{e}");
}

#[test]
fn line_series_style_and_labels() {
    let mut s = session();
    let v = add(
        &mut s,
        "lineChart",
        json!([{"role": "Category", "field": "Product[Name]"}, {"role": "Y", "field": "[Total]"},
               {"role": "Y", "field": "[Margin]"}]),
    );
    let pg = page(&s);
    let out = run(
        &mut s,
        "visual.series",
        json!({"page": pg, "visual": v, "target": "[Margin]",
               "line": {"width": 4, "style": "Dashed", "show_marker": true, "marker_shape": "diamond"},
               "labels": {"show": true, "color": "#444444"}}),
    );
    let o = objects(&s, &v);
    let ls = &o["lineStyles"][0];
    assert_eq!(ls["selector"], json!({"metadata": "Sales.Margin"}));
    assert_eq!(
        ls["properties"]["lineStyle"]["expr"]["Literal"]["Value"],
        "'dashed'"
    );
    assert_eq!(ls["properties"]["showMarker"]["expr"]["Literal"]["Value"], "true");
    assert_eq!(
        ls["properties"]["markerShape"]["expr"]["Literal"]["Value"],
        "'diamond'"
    );
    let w = ls["properties"]["strokeWidth"]["expr"]["Literal"]["Value"]
        .as_str()
        .unwrap();
    assert!(w == "4D" || w == "4L", "{w}");
    assert_eq!(o["labels"][0]["selector"], json!({"metadata": "Sales.Margin"}));
    assert_eq!(
        out["props"]["series"],
        json!([{"target": "[Margin]",
                "line": {"width": 4, "style": "dashed", "show_marker": true, "marker_shape": "diamond"},
                "labels": {"show": true, "color": "#444444"}}])
    );
    assert_extract_noop(&s, visual(&s, &v));
    // A whole group set to null removes that object's entry for the series.
    run(
        &mut s,
        "visual.series",
        json!({"page": pg, "visual": v, "target": "[Margin]", "line": null}),
    );
    assert!(objects(&s, &v).get("lineStyles").is_none());
    // clear removes everything for the target.
    let out = run(
        &mut s,
        "visual.series",
        json!({"page": pg, "visual": v, "target": "[Margin]", "clear": true}),
    );
    assert_eq!(out["changed"], true);
    assert_eq!(objects(&s, &v), Value::Null);
}

#[test]
fn generic_objects_and_visual_format_and_report_apply() {
    let mut s = session();
    let v = add(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Product[Name]"}, {"role": "Values", "field": "[Total]"}]),
    );
    let pg = page(&s);
    // Through visual.format: series next to ordinary format groups.
    run(
        &mut s,
        "visual.format",
        json!({"page": pg, "visual": v, "border": true,
               "series": [{"target": "Product[Name]", "objects": [
                   {"object": "columnWidth", "property": "value", "value": 180},
                   {"object": "values", "property": "fontColor", "value": {"color": "#FF0000"}}]}]}),
    );
    let o = objects(&s, &v);
    assert_eq!(
        o["columnWidth"][0]["selector"],
        json!({"metadata": "Product.Name"})
    );
    let props = extract_props(visual(&s, &v));
    assert_eq!(
        props["series"],
        json!([{"target": "Product[Name]", "objects": [
            {"object": "columnWidth", "property": "value", "value": 180},
            {"object": "values", "property": "fontColor", "value": {"color": "#FF0000"}}]}])
    );
    assert_eq!(props["format"]["border"], json!({"show": true}));
    assert_extract_noop(&s, visual(&s, &v));

    // report.extract -> report.apply: no change.
    let spec = run(&mut s, "report.extract", json!({}));
    let plan = run(&mut s, "report.check", json!({ "spec": spec }));
    assert_eq!(plan["in_sync"], true, "{plan}");

    // report.apply takes series in props.
    let mut s2 = session();
    let pg = page(&s2);
    run(
        &mut s2,
        "report.apply",
        json!({"spec": {"pages": [{"key": pg, "visuals": [{"key": "c", "type": "clusteredColumnChart",
            "x": 0, "y": 0, "width": 300, "height": 200,
            "fields": [{"role": "Category", "field": "Product[Name]"}, {"role": "Y", "field": "[Total]"}],
            "props": {"colors": {"[Total]": "#00FF00", "Product[Name]=Bikes": "#FF00FF"}}}]}]}}),
    );
    let r = s2.report.as_ref().unwrap();
    let vis = r
        .pages
        .iter()
        .flat_map(|p| &p.visuals)
        .find(|v| v.visual_type.as_deref() == Some("clusteredColumnChart"))
        .unwrap();
    assert_eq!(
        vis.visual_extra["objects"]["dataPoint"].as_array().unwrap().len(),
        2
    );
}

#[test]
fn bad_series_are_rejected_without_changes() {
    let mut s = session();
    let v = chart(&mut s);
    let p = page(&s);
    for (params, needle) in [
        (json!({"target": "[Nope]", "color": "#000000"}), "no measure"),
        (
            json!({"target": "Sales[Amount]", "color": "#000000"}),
            "not bound",
        ),
        (json!({"target": "[Total]", "color": "red"}), "color"),
        (json!({"target": "[Total]", "line": {"style": "wavy"}}), "solid"),
        (json!({"target": "[Total]", "line": {"width": -1}}), "number"),
        (json!({"target": "[Total]", "sparkle": true}), "unknown"),
        (json!({"target": "[Total]=3", "color": "#000000"}), "column"),
        (json!({"color": "#000000"}), "target"),
        (
            json!({"target": "[Total]", "objects": [{"object": "bad name", "property": "x", "value": 1}]}),
            "formatting name",
        ),
        (
            json!({"series": [{"target": "[Total]", "color": "#000000"}, {"target": "Sales[Total]", "color": "#111111"}]}),
            "again",
        ),
        (json!({}), "nothing to do"),
    ] {
        let mut params = params;
        params["page"] = json!(p);
        params["visual"] = json!(v);
        let e = fails(&mut s, "visual.series", params.clone());
        assert!(e.contains(needle), "{params}: {e}");
    }
    // An object the catalog says takes no selector is refused.
    let e = fails(
        &mut s,
        "visual.series",
        json!({"page": p, "visual": v, "target": "[Total]",
               "objects": [{"object": "legend", "property": "show", "value": true}]}),
    );
    assert!(e.contains("per series"), "{e}");
    // Unknown objects of built-in types are refused with a hint.
    let e = fails(
        &mut s,
        "visual.series",
        json!({"page": p, "visual": v, "target": "[Total]",
               "objects": [{"object": "dataPiont", "property": "fill", "value": {"color": "#000000"}}]}),
    );
    assert!(e.contains("dataPoint"), "{e}");
}

#[test]
fn same_json_compares_by_meaning() {
    let a = json!({"expr": {"Literal": {"Value": "12L"}}});
    let b = json!({"expr": {"Literal": {"Value": "12D"}}});
    assert!(same_json(&a, &b));
    let c = json!({"solid": {"color": {"expr": {"Literal": {"Value": "'#ABCDEF'"}}}}});
    let d = json!({"solid": {"color": {"expr": {"Literal": {"Value": "'#abcdef'"}}}}});
    assert!(same_json(&c, &d));
    assert!(!same_json(&json!({"Value": "'x'"}), &json!({"Value": "'X'"})));
    assert!(same_json(&json!({"a": 1, "b": 2}), &json!({"b": 2, "a": 1.0})));
    assert!(!same_json(&json!([1, 2]), &json!([2, 1])));
}

#[test]
fn unknown_selectors_are_left_alone() {
    let mut s = session();
    let v = chart(&mut s);
    let p = page(&s);
    run(
        &mut s,
        "visual.format",
        json!({"page": p, "visual": v, "objects": [
            {"object": "dataPoint", "property": "fill", "value": {"color": "#123456"}, "selector": {"id": "x"}},
            {"object": "dataPoint", "property": "fill", "value": {"color": "#654321"},
             "selector": {"metadata": "Stale.Ref"}}]}),
    );
    let props = extract_props(visual(&s, &v));
    assert!(props.get("series").is_none(), "{props}");
    assert_extract_noop(&s, visual(&s, &v));
    // clear on a series leaves those entries.
    run(
        &mut s,
        "visual.series",
        json!({"page": p, "visual": v, "target": "[Total]", "color": "#000000"}),
    );
    run(
        &mut s,
        "visual.series",
        json!({"page": p, "visual": v, "target": "[Total]", "clear": true}),
    );
    assert_eq!(objects(&s, &v)["dataPoint"].as_array().unwrap().len(), 2);
}

/// Extract → apply is a no-op on the real reports of the local test data (skipped without it).
#[test]
fn series_extract_then_apply_is_a_no_op_on_real_visuals() {
    let Some(dir) = bicraft_env::var_os("BICRAFT_TESTDATA") else {
        eprintln!("BICRAFT_TESTDATA not set: skipped");
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let (mut checked, mut with_series, mut with_conditional) = (0, 0, 0);
    for name in ["sample1/report.pbix"] {
        let path = dir.join(name);
        if !path.exists() {
            eprintln!("{} missing: skipped", path.display());
            continue;
        }
        let mut s = Session::new();
        if s.execute("file.open", &json!({"path": path})).is_err() {
            continue;
        }
        let Some(r) = s.report.clone() else { continue };
        for page in &r.pages {
            for v in &page.visuals {
                assert_extract_noop(&s, v);
                let props = extract_props(v);
                with_series += usize::from(props.get("series").is_some());
                with_conditional += usize::from(props.get("conditional").is_some());
                checked += 1;
            }
        }
    }
    eprintln!(
        "{checked} real visuals checked: {with_series} with series, {with_conditional} with conditional"
    );
}

fn fill(c: &str) -> Value {
    json!({"solid": {"color": {"expr": {"Literal": {"Value": format!("'{c}'")}}}}})
}

/// Desktop colors a chart's only series with the chart's default color (the `dataPoint` entry
/// without a selector) and ignores a per-measure one there.
#[test]
fn the_only_series_of_a_chart_takes_the_default_color() {
    let mut s = session();
    let pg = page(&s);
    let v = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Product[Name]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let out = run(
        &mut s,
        "visual.series",
        json!({"page": pg, "visual": v, "colors": {"[Total]": "#118DFF"}}),
    );
    assert_eq!(out["changed"], true, "{out}");
    assert_eq!(
        objects(&s, &v)["dataPoint"],
        json!([{"properties": {"fill": fill("#118DFF")}}])
    );
    assert_eq!(
        out["props"]["series"],
        json!([{"target": "[Total]", "color": "#118DFF"}])
    );
    assert_extract_noop(&s, visual(&s, &v));
    s.report_dirty = false;
    let again = run(
        &mut s,
        "visual.series",
        json!({"page": pg, "visual": v, "series": [{"target": "[Total]", "color": "#118dff"}]}),
    );
    assert_eq!(again["changed"], false, "{again}");
    assert!(!s.report_dirty);
    schema_ok(&mut s);
    let back = pbir_round_trip(s.report.as_ref().unwrap());
    assert_eq!(
        back.page(&pg).unwrap().visual(&v).unwrap().visual_extra["objects"],
        objects(&s, &v)
    );

    // A per-measure color already there (Desktop ignores it) is kept, and extracted as such.
    run(
        &mut s,
        "visual.series",
        json!({"page": pg, "visual": v, "series": [{"target": "[Total]", "objects": [
            {"object": "dataPoint", "property": "fill", "value": {"color": "#FF0000"}}]}]}),
    );
    assert_eq!(objects(&s, &v)["dataPoint"].as_array().unwrap().len(), 2);
    let props = extract_props(visual(&s, &v));
    assert_eq!(
        props["series"],
        json!([{"target": "[Total]", "objects": [
            {"object": "dataPoint", "property": "fill", "value": {"color": "#FF0000"}}], "color": "#118DFF"}]),
        "{props}"
    );
    assert_extract_noop(&s, visual(&s, &v));
    // A new color changes the default entry only.
    run(
        &mut s,
        "visual.series",
        json!({"page": pg, "visual": v, "colors": {"[Total]": "#00AA00"}}),
    );
    let dp = objects(&s, &v)["dataPoint"].clone();
    assert_eq!(dp[0], json!({"properties": {"fill": fill("#00AA00")}}));
    assert_eq!(dp[1]["selector"], json!({"metadata": "Sales.Total"}));
    assert_extract_noop(&s, visual(&s, &v));
    // null removes both.
    run(
        &mut s,
        "visual.series",
        json!({"page": pg, "visual": v, "colors": {"[Total]": null}}),
    );
    assert!(visual(&s, &v).visual_extra.get("objects").is_none());

    // A legend splits the series: the per-measure selector is used.
    let legend = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Product[Name]"}, {"role": "Series", "field": "Sales[Region]"},
               {"role": "Y", "field": "[Total]"}]),
    );
    run(
        &mut s,
        "visual.series",
        json!({"page": pg, "visual": legend, "colors": {"[Total]": "#118DFF"}}),
    );
    assert_eq!(
        objects(&s, &legend)["dataPoint"][0]["selector"],
        json!({"metadata": "Sales.Total"})
    );
    // So is a pie (its colors are per category); charts with two measures:
    // `colors_per_measure_…`.
    let pie = add(
        &mut s,
        "pieChart",
        json!([{"role": "Category", "field": "Product[Name]"}, {"role": "Y", "field": "[Total]"}]),
    );
    run(
        &mut s,
        "visual.series",
        json!({"page": pg, "visual": pie, "series": [{"target": "[Total]", "color": "#111111"}]}),
    );
    assert_eq!(
        objects(&s, &pie)["dataPoint"][0]["selector"],
        json!({"metadata": "Sales.Total"})
    );
    // Line charts use the per-measure color (Desktop ignores the default one there).
    let line = add(
        &mut s,
        "lineChart",
        json!([{"role": "Category", "field": "Product[Name]"}, {"role": "Y", "field": "[Margin]"}]),
    );
    run(
        &mut s,
        "visual.format",
        json!({"page": pg, "visual": line, "colors": {"[Margin]": "#ABCDEF"}}),
    );
    assert_eq!(
        objects(&s, &line)["dataPoint"][0]["selector"],
        json!({"metadata": "Sales.Margin"})
    );
    // A funnel's only series takes the default color, through visual.format.
    let funnel = add(
        &mut s,
        "funnel",
        json!([{"role": "Category", "field": "Product[Name]"}, {"role": "Y", "field": "[Margin]"}]),
    );
    run(
        &mut s,
        "visual.format",
        json!({"page": pg, "visual": funnel, "colors": {"[Margin]": "#ABCDEF"}}),
    );
    assert_eq!(
        objects(&s, &funnel)["dataPoint"],
        json!([{"properties": {"fill": fill("#ABCDEF")}}])
    );
    assert_extract_noop(&s, visual(&s, &funnel));
}

/// Renames rewrite the queryRefs Desktop derives from field names, everywhere a visual names
/// them: projections, selectors (series colors and labels, conditional formats, analytics
/// lines), `SelectRef` values, the phone layout.
#[test]
fn renames_rewrite_query_refs_in_selectors_analytics_and_sort() {
    let mut s = session();
    let pg = page(&s);
    let chart = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Product[Name]"}, {"role": "Y", "field": "[Total]"},
               {"role": "Y", "field": "Sum(Sales[Amount])"}]),
    );
    let table = add(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Region]"}, {"role": "Values", "field": "[Total]"},
               {"role": "Values", "field": "Sum(Sales[Amount])"}]),
    );
    run(
        &mut s,
        "visual.format",
        json!({"page": pg, "visual": chart,
               "colors": {"[Total]": "#118DFF", "Sum(Sales[Amount])": "#E66C37"},
               "series": [{"target": "[Total]", "labels": {"color": "#222222"}}]}),
    );
    run(
        &mut s,
        "visual.set_sort",
        json!({"page": pg, "visual": chart, "field": "[Total]"}),
    );
    run(
        &mut s,
        "visual.analytics",
        json!({"page": pg, "visual": chart, "lines": [{"kind": "average", "measure": "[Total]"},
                                                      {"kind": "max", "measure": "Sum(Sales[Amount])"}]}),
    );
    run(
        &mut s,
        "visual.conditional",
        json!({"page": pg, "visual": table, "conditional": [
            {"target": "background", "series": "[Total]",
             "rule": {"gradient": {"field": "[Total]", "min": "#FFFFFF", "max": "#00AA00"}}},
            {"target": "font_color", "series": "Sum(Sales[Amount])",
             "rule": {"rules": {"field": "Sum(Sales[Amount])", "cases": [{"op": ">", "value": 10, "color": "#FF0000"}]}}},
            {"series": "Sum(Sales[Amount])", "rule": {"data_bars": {}}}]}),
    );
    run(
        &mut s,
        "visual.set_sort",
        json!({"page": pg, "visual": table, "field": "Sum(Sales[Amount])"}),
    );
    run(
        &mut s,
        "visual.mobile",
        json!({"page": pg, "visual": chart, "x": 0, "y": 0, "width": 300, "height": 200}),
    );
    // A phone-layout format of the series (mobile.json `objects`).
    let suffix = format!("{chart}/mobile.json");
    {
        let r = s.report.as_mut().unwrap();
        let res = r
            .resources
            .iter_mut()
            .find(|x| x.path.ends_with(&suffix))
            .unwrap();
        let mut doc: Value = serde_json::from_slice(&res.bytes).unwrap();
        doc["objects"] = json!({"labels": [{"properties": {"show": {"expr": {"Literal": {"Value": "true"}}}},
                                             "selector": {"metadata": "Sales.Total"}}]});
        res.bytes = serde_json::to_vec_pretty(&doc).unwrap();
    }
    // The table's query is stored, as in files Desktop saved; the chart's is derived from its
    // bindings.
    {
        let r = s.report.as_mut().unwrap();
        let vis = r.pages[0].visual_mut(&table).unwrap();
        let query = bicraft_report::query::effective(vis).unwrap();
        vis.visual_extra.insert("query".into(), query);
    }
    let props_of = |s: &Session, v: &str| extract_props(visual(s, v)).to_string();
    let (chart0, table0) = (props_of(&s, &chart), props_of(&s, &table));
    let refs_of = |s: &Session, v: &str| -> Vec<String> {
        let vis = visual(s, v);
        let query = bicraft_report::query::effective(vis);
        let mut rest = vis.visual_extra.clone();
        rest.shift_remove("query");
        let text = format!("{}{}", json!(rest), json!(query));
        [
            "Sales.Total",
            "Sales.Revenue",
            "Sum(Sales.Amount)",
            "Sum(Sales.Price)",
            "Fact.Revenue",
            "Sum(Fact.Price)",
        ]
        .iter()
        .filter_map(|r| {
            let n = text.matches(&format!("\"{r}\"")).count();
            (n > 0).then(|| format!("{r}:{n}"))
        })
        .collect()
    };
    let mobile = |s: &Session| {
        let r = s.report.as_ref().unwrap();
        let res = r.resources.iter().find(|x| x.path.ends_with(&suffix)).unwrap();
        String::from_utf8(res.bytes.clone()).unwrap()
    };
    // [Total]: queryRef, color, labels, average line (selector and SelectRef);
    // Sum: queryRef, color, max line (selector and SelectRef).
    assert_eq!(refs_of(&s, &chart), ["Sales.Total:5", "Sum(Sales.Amount):4"]);
    // queryRef + gradient selector; queryRef + rules + data bars selectors.
    assert_eq!(refs_of(&s, &table), ["Sales.Total:2", "Sum(Sales.Amount):3"]);

    let out = run(
        &mut s,
        "measure.set",
        json!({"name": "Total", "new_name": "Revenue", "update_report": true}),
    );
    assert_eq!(out["report"]["query_refs_updated"], 8, "{out}");
    assert_eq!(refs_of(&s, &chart), ["Sales.Revenue:5", "Sum(Sales.Amount):4"]);
    assert_eq!(refs_of(&s, &table), ["Sales.Revenue:2", "Sum(Sales.Amount):3"]);
    assert!(mobile(&s).contains("\"Sales.Revenue\""), "{}", mobile(&s));

    run(
        &mut s,
        "column.set",
        json!({"table": "Sales", "name": "Amount", "new_name": "Price", "update_report": true}),
    );
    assert_eq!(refs_of(&s, &chart), ["Sales.Revenue:5", "Sum(Sales.Price):4"]);
    run(
        &mut s,
        "table.rename",
        json!({"name": "Sales", "new_name": "Fact", "update_report": true}),
    );
    assert_eq!(refs_of(&s, &chart), ["Fact.Revenue:5", "Sum(Fact.Price):4"]);
    assert_eq!(refs_of(&s, &table), ["Fact.Revenue:2", "Sum(Fact.Price):3"]);
    assert!(mobile(&s).contains("\"Fact.Revenue\""));

    // The props read the same, with the new names, and re-apply as a no-op.
    let renamed = |t: String| {
        t.replace("[Total]", "[Revenue]")
            .replace("Sales[Amount]", "Fact[Price]")
            .replace("Sales[", "Fact[")
    };
    assert_eq!(props_of(&s, &chart), renamed(chart0));
    assert_eq!(props_of(&s, &table), renamed(table0));
    for v in [&chart, &table] {
        assert_extract_noop(&s, visual(&s, v));
    }
    schema_ok(&mut s);
    let back = pbir_round_trip(s.report.as_ref().unwrap());
    for v in [&chart, &table] {
        assert_eq!(
            back.page(&pg).unwrap().visual(v).unwrap().visual_extra["objects"],
            visual(&s, v).visual_extra["objects"]
        );
    }
}

/// A queryRef is renamed only when it is Desktop's name for the field, and never onto a name
/// another projection of the visual has.
#[test]
fn renames_keep_custom_and_clashing_query_refs() {
    let mut s = session();
    let pg = page(&s);
    let v = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Product[Name]"}, {"role": "Y", "field": "[Total]"},
               {"role": "Y", "field": "[Margin]"}]),
    );
    run(
        &mut s,
        "visual.series",
        json!({"page": pg, "visual": v, "colors": {"[Total]": "#118DFF", "[Margin]": "#E66C37"}}),
    );
    // [Margin]'s queryRef (and its selector) is not Desktop's name for it (a raw query, as in a
    // file Desktop saved).
    {
        let r = s.report.as_mut().unwrap();
        let vis = r.pages[0].visual_mut(&v).unwrap();
        let query = bicraft_report::query::effective(vis).unwrap();
        vis.visual_extra.insert("query".into(), query);
        let text = serde_json::to_string(&vis.visual_extra)
            .unwrap()
            .replace("\"Sales.Margin\"", "\"Sales.Revenue\"");
        vis.visual_extra = serde_json::from_str(&text).unwrap();
    }
    let count = |s: &Session, r: &str| {
        serde_json::to_string(&visual(s, &v).visual_extra)
            .unwrap()
            .matches(&format!("\"{r}\""))
            .count()
    };
    // [Total] -> [Revenue]: `Sales.Revenue` is taken, so `Sales.Total` stays.
    run(
        &mut s,
        "measure.set",
        json!({"name": "Total", "new_name": "Revenue", "update_report": true}),
    );
    assert_eq!((count(&s, "Sales.Total"), count(&s, "Sales.Revenue")), (2, 2));
    // [Margin] -> [Profit]: its queryRef is not `Sales.Margin`, so it stays.
    run(
        &mut s,
        "measure.set",
        json!({"name": "Margin", "new_name": "Profit", "update_report": true}),
    );
    assert_eq!((count(&s, "Sales.Revenue"), count(&s, "Sales.Profit")), (2, 0));
    assert_extract_noop(&s, visual(&s, &v));
    assert_eq!(
        extract_props(visual(&s, &v))["series"],
        json!([{"target": "[Revenue]", "color": "#118DFF"}, {"target": "[Profit]", "color": "#E66C37"}])
    );
}
