//! Tests of conditional formatting (`visual_conditional.rs`): Desktop's JSON shapes for color
//! scales, rules, field values, icons, data bars and web URLs, the `visual.conditional` command,
//! the `conditional` props key, extract → apply idempotence.

use bicraft_report::Visual;
use serde_json::{Value, json};

use crate::Session;
use crate::visual_props::{apply_props, extract_props};

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

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

/// Sales(Amount, Region, Url, Color) with [Total], and a blank report.
fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        ("table.add", json!({"name": "Sales"})),
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
            json!({"table": "Sales", "name": "Url", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Color", "data_type": "string"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
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

/// A table: Sales[Region], [Total].
fn table(s: &mut Session) -> String {
    add(
        s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Region]"}, {"role": "Values", "field": "[Total]"}]),
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

fn schema_ok(s: &mut Session) {
    let v = run(s, "report.validate", json!({}));
    assert_eq!(v["schema_issues"], json!([]), "{v}");
    assert_eq!(v["error_count"], 0, "{v}");
}

fn total() -> Value {
    json!({"Measure": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Total"}})
}

fn lit(s: &str) -> Value {
    json!({"Literal": {"Value": s}})
}

/// Runs `visual.conditional`, checks re-running it is a no-op, and returns the result.
fn cond(s: &mut Session, v: &str, item: Value) -> Value {
    let mut params = item.clone();
    params["page"] = json!(page(s));
    params["visual"] = json!(v);
    let out = run(s, "visual.conditional", params.clone());
    assert_eq!(out["changed"], true, "{out}");
    s.report_dirty = false;
    let again = run(s, "visual.conditional", params);
    assert_eq!(again["changed"], false, "{again}");
    assert!(!s.report_dirty);
    assert_extract_noop(s, visual(s, v));
    out
}

#[test]
fn gradient_background_has_desktop_shape() {
    let mut s = session();
    let v = table(&mut s);
    let out = cond(
        &mut s,
        &v,
        json!({"target": "background", "series": "[Total]",
               "rule": {"gradient": {"field": "[Total]", "min": "#F8696B", "max": {"color": "#63BE7B", "value": 1000}}}}),
    );
    assert_eq!(
        objects(&s, &v)["values"],
        json!([{"properties": {"backColor": {"solid": {"color": {"expr": {"FillRule": {
            "Input": total(),
            "FillRule": {"linearGradient2": {
                "min": {"color": lit("'#F8696B'")},
                "max": {"color": lit("'#63BE7B'"), "value": lit("1000D")},
            }},
        }}}}}},
        "selector": {"data": [{"dataViewWildcard": {"matchingOption": 1}}], "metadata": "Sales.Total"}}])
    );
    assert_eq!(
        out["props"]["conditional"],
        json!([{"target": "background", "series": "[Total]", "rule": {"gradient": {
            "field": "[Total]", "min": "#F8696B", "max": {"color": "#63BE7B", "value": 1000}}}}])
    );
    // Three stops and a blank color; series defaults to the bound rule field.
    cond(
        &mut s,
        &v,
        json!({"target": "font_color",
               "rule": {"gradient": {"field": "[Total]", "min": "#FF0000", "mid": {"color": "#FFFF00", "value": 0},
                                     "max": "#00FF00", "empty": "#CCCCCC"}}}),
    );
    let fc = &objects(&s, &v)["values"][0]["properties"]["fontColor"]["solid"]["color"]["expr"]["FillRule"]["FillRule"]
        ["linearGradient3"];
    assert_eq!(fc["mid"], json!({"color": lit("'#FFFF00'"), "value": lit("0D")}));
    assert_eq!(
        fc["nullColoringStrategy"],
        json!({"strategy": lit("'specificColor'"), "color": lit("'#CCCCCC'")})
    );
    schema_ok(&mut s);
    // rule: null removes one; the other stays.
    let pg = page(&s);
    run(
        &mut s,
        "visual.conditional",
        json!({"page": pg, "visual": v, "target": "font_color", "series": "[Total]", "rule": null}),
    );
    let props = &objects(&s, &v)["values"][0]["properties"];
    assert!(props.get("fontColor").is_none() && props.get("backColor").is_some());
}

#[test]
fn rules_write_conditional_cases() {
    let mut s = session();
    let v = table(&mut s);
    cond(
        &mut s,
        &v,
        json!({"target": "font_color", "series": "Sales[Region]",
               "rule": {"rules": {"field": "[Total]", "cases": [
                   {"op": ">=", "value": 1000, "color": "#00AA00"},
                   {"op": "between", "values": [0, 999.5], "color": "#AAAA00"},
                   {"gte": -100, "lt": 0, "color": "#AA0000"},
                   {"op": "=", "value": null, "color": "#999999"}],
                   "else": "#000000"}}}),
    );
    let o = objects(&s, &v);
    let entry = &o["values"][0];
    assert_eq!(
        entry["selector"],
        json!({"data": [{"dataViewWildcard": {"matchingOption": 1}}], "metadata": "Sales.Region"})
    );
    let c = &entry["properties"]["fontColor"]["solid"]["color"]["expr"]["Conditional"];
    assert_eq!(
        c["Cases"][0],
        json!({"Condition": {"Comparison": {"ComparisonKind": 2, "Left": total(), "Right": lit("1000D")}},
               "Value": lit("'#00AA00'")})
    );
    assert_eq!(
        c["Cases"][1]["Condition"],
        json!({"And": {
            "Left": {"Comparison": {"ComparisonKind": 2, "Left": total(), "Right": lit("0D")}},
            "Right": {"Comparison": {"ComparisonKind": 4, "Left": total(), "Right": lit("999.5D")}}}})
    );
    assert_eq!(c["Cases"][3]["Condition"]["Comparison"]["Right"], lit("null"));
    assert_eq!(c["DefaultValue"], lit("'#000000'"));
    let props = extract_props(visual(&s, &v));
    assert_eq!(
        props["conditional"][0]["rule"]["rules"]["cases"],
        json!([
            {"op": ">=", "value": 1000, "color": "#00AA00"},
            {"op": "between", "values": [0, 999.5], "color": "#AAAA00"},
            {"gte": -100, "lt": 0, "color": "#AA0000"},
            {"op": "=", "value": null, "color": "#999999"}])
    );
    schema_ok(&mut s);
}

#[test]
fn field_value_on_charts_and_web_url_on_tables() {
    let mut s = session();
    let chart = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]),
    );
    // Chart: no series, wildcard selector; a column goes through Min (Desktop's "First").
    cond(
        &mut s,
        &chart,
        json!({"rule": {"field_value": {"field": "Sales[Color]"}}}),
    );
    assert_eq!(
        objects(&s, &chart)["dataPoint"],
        json!([{"properties": {"fill": {"solid": {"color": {"expr": {"Aggregation": {
            "Expression": {"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Color"}},
            "Function": 3}}}}}},
            "selector": {"data": [{"dataViewWildcard": {"matchingOption": 1}}]}}])
    );
    assert_eq!(
        extract_props(visual(&s, &chart))["conditional"],
        json!([{"target": "data_color", "rule": {"field_value": {"field": "Min(Sales[Color])"}}}])
    );
    // Table: web URL from a column.
    let t = add(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Region]"}, {"role": "Values", "field": "Sales[Url]"}]),
    );
    cond(
        &mut s,
        &t,
        json!({"series": "Sales[Url]", "rule": {"web_url": {"field": "Sales[Url]"}}}),
    );
    let e = &objects(&s, &t)["values"][0];
    assert_eq!(e["properties"]["webURL"]["expr"]["Aggregation"]["Function"], 3);
    assert_eq!(e["selector"]["metadata"], "Sales.Url");
    schema_ok(&mut s);
}

#[test]
fn icons_and_data_bars() {
    let mut s = session();
    let v = table(&mut s);
    cond(
        &mut s,
        &v,
        json!({"series": "[Total]", "rule": {"icons": {"field": "[Total]", "preset": "traffic",
               "thresholds": [100, 500], "position": "after", "align": "middle"}}}),
    );
    let icon = &objects(&s, &v)["values"][0]["properties"]["icon"];
    assert_eq!(icon["kind"], "Icon");
    assert_eq!(icon["layout"], json!({"expr": lit("'After'")}));
    assert_eq!(icon["verticalAlignment"], json!({"expr": lit("'Middle'")}));
    let cases = &icon["value"]["expr"]["Conditional"]["Cases"];
    assert_eq!(cases[0]["Value"], lit("'TrafficHigh'"));
    assert_eq!(
        cases[1]["Condition"]["And"]["Right"]["Comparison"]["ComparisonKind"],
        3
    );
    assert_eq!(cases[2]["Value"], lit("'TrafficLow'"));

    cond(
        &mut s,
        &v,
        json!({"series": "[Total]", "rule": {"data_bars": {"positive": "#5B9BD5", "show_bar_only": true, "max": 1000}}}),
    );
    assert_eq!(
        objects(&s, &v)["columnFormatting"],
        json!([{"properties": {"dataBars": {
            "maxValue": {"expr": lit("1000D")},
            "positiveColor": {"solid": {"color": {"expr": lit("'#5B9BD5'")}}},
            "negativeColor": {"solid": {"color": {"expr": lit("'#D64550'")}}},
            "axisColor": {"solid": {"color": {"expr": lit("'#000000'")}}},
            "reverseDirection": {"expr": lit("false")},
            "hideText": {"expr": lit("true")},
        }}, "selector": {"metadata": "Sales.Total"}}])
    );
    let props = extract_props(visual(&s, &v));
    let kinds: Vec<&str> = props["conditional"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["target"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["icon", "data_bars"]);
    // Data bars are conditional formatting, not a series property.
    assert!(props.get("series").is_none(), "{props}");
    schema_ok(&mut s);
}

#[test]
fn legacy_metadata_selectors_are_reused() {
    let mut s = session();
    let v = table(&mut s);
    let pg = page(&s);
    // An older report: the gradient scoped by metadata alone.
    let gradient = json!({"solid": {"color": {"expr": {"FillRule": {"Input": total(),
        "FillRule": {"linearGradient2": {"min": {"color": lit("'#FFFFFF'")}, "max": {"color": lit("'#000000'")}}}}}}}});
    run(
        &mut s,
        "visual.format",
        json!({"page": pg, "visual": v, "objects": [{"object": "values", "property": "backColor",
               "value": gradient, "selector": {"metadata": "Sales.Total"}}]}),
    );
    let props = extract_props(visual(&s, &v));
    assert_eq!(props["conditional"][0]["target"], "background", "{props}");
    assert!(props.get("series").is_none(), "{props}");
    assert_extract_noop(&s, visual(&s, &v));
    // Changing it edits that entry; no second entry appears.
    run(
        &mut s,
        "visual.conditional",
        json!({"page": pg, "visual": v, "series": "[Total]",
               "rule": {"gradient": {"field": "[Total]", "min": "#FFFFFF", "max": "#FF0000"}}}),
    );
    assert_eq!(objects(&s, &v)["values"].as_array().unwrap().len(), 1);
    run(
        &mut s,
        "visual.conditional",
        json!({"page": pg, "visual": v, "series": "[Total]", "rule": null}),
    );
    assert_eq!(objects(&s, &v), Value::Null);
}

#[test]
fn conditional_through_props_and_apply_to() {
    let mut s = session();
    let v = table(&mut s);
    let pg = page(&s);
    run(
        &mut s,
        "visual.format",
        json!({"page": pg, "visual": v, "conditional": [
            {"series": "[Total]", "apply_to": "values_and_totals",
             "rule": {"gradient": {"field": "[Total]", "min": "#FFFFFF", "max": "#0000FF", "empty": "zero"}}}]}),
    );
    let e = &objects(&s, &v)["values"][0];
    assert_eq!(e["selector"]["data"][0]["dataViewWildcard"]["matchingOption"], 0);
    let props = extract_props(visual(&s, &v));
    assert_eq!(props["conditional"][0]["apply_to"], "values_and_totals");
    assert_eq!(props["conditional"][0]["rule"]["gradient"]["empty"], "zero");
    assert_extract_noop(&s, visual(&s, &v));
    // report.extract / report.check agree.
    let spec = run(&mut s, "report.extract", json!({}));
    let check = run(&mut s, "report.check", json!({ "spec": spec }));
    assert_eq!(check["in_sync"], true, "{check}");
}

#[test]
fn bad_conditional_formats_are_rejected_without_changes() {
    let mut s = session();
    let v = table(&mut s);
    let chart = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let pg = page(&s);
    let grad = json!({"gradient": {"field": "[Total]", "min": "#FFFFFF", "max": "#000000"}});
    for (visual, params, needle) in [
        (
            &v,
            json!({"rule": {"gradient": {"field": "Sales[Amount]", "min": "#FFFFFF", "max": "#000000"}}}),
            "column",
        ),
        (
            &v,
            json!({"rule": {"gradient": {"field": "[Total]", "min": "#FFFFFF"}}}),
            "max",
        ),
        (
            &v,
            json!({"rule": {"gradient": {"field": "[Nope]", "min": "#FFFFFF", "max": "#000000"}}}),
            "no measure",
        ),
        (
            &v,
            json!({"rule": {"rules": {"field": "[Total]", "cases": [{"op": "~", "value": 1, "color": "#000000"}]}}}),
            "op",
        ),
        (
            &v,
            json!({"rule": {"rules": {"field": "[Total]", "cases": [{"op": ">", "value": null, "color": "#000000"}]}}}),
            "blank",
        ),
        (
            &v,
            json!({"rule": {"rules": {"field": "[Total]", "cases": []}}}),
            "cases",
        ),
        (&v, json!({"rule": {"gradient": {}, "rules": {}}}), "exactly one"),
        (
            &v,
            json!({"target": "data_color", "series": "[Total]", "rule": grad.clone()}),
            "not a target",
        ),
        (
            &v,
            json!({"target": "icon", "series": "[Total]", "rule": grad.clone()}),
            "cannot go on",
        ),
        (
            &v,
            json!({"series": "Sales[Amount]", "rule": grad.clone()}),
            "not bound",
        ),
        (
            &v,
            json!({"rule": {"data_bars": {"min": 5, "max": 1}}, "series": "[Total]"}),
            "min",
        ),
        (
            &v,
            json!({"rule": {"icons": {"field": "[Total]", "preset": "traffic"}}, "series": "[Total]"}),
            "thresholds",
        ),
        (
            &v,
            json!({"rule": {"icons": {"field": "[Total]", "cases": [{"op": ">", "value": 1, "icon": "a b"}]}}, "series": "[Total]"}),
            "icon",
        ),
        (
            &v,
            json!({"series": "[Total]", "apply_to": "all", "rule": grad.clone()}),
            "apply_to",
        ),
        (&v, json!({"series": "[Total]"}), "rule"),
        (
            &chart,
            json!({"target": "background", "rule": grad.clone()}),
            "not a target",
        ),
        (
            &chart,
            json!({"target": {"object": "legend", "property": "show"}, "rule": grad.clone()}),
            "cannot go on",
        ),
    ] {
        let mut p = params.clone();
        p["page"] = json!(pg);
        p["visual"] = json!(visual);
        let e = fails(&mut s, "visual.conditional", p);
        assert!(e.contains(needle), "{params}: {e}");
    }
    // A field value goes on any literal-typed property: Power BI Desktop 2.158 keeps a measure on
    // every one of them when it saves (reproducibility oracle, docs/formats/visual-options.md).
    run(
        &mut s,
        "visual.conditional",
        json!({"page": pg, "visual": chart, "target": {"object": "legend", "property": "show"},
               "rule": {"field_value": {"field": "[Total]"}}}),
    );
    // A table needs to know which column to format.
    let mut p =
        json!({"rule": {"gradient": {"field": "Sum(Sales[Amount])", "min": "#FFFFFF", "max": "#000000"}}});
    p["page"] = json!(pg);
    p["visual"] = json!(v);
    let e = fails(&mut s, "visual.conditional", p);
    assert!(e.contains("series"), "{e}");
    // Too many cases.
    let cases: Vec<Value> = (0..51)
        .map(|i| json!({"op": ">", "value": i, "color": "#000000"}))
        .collect();
    let e = fails(
        &mut s,
        "visual.conditional",
        json!({"page": pg, "visual": v, "series": "[Total]",
               "rule": {"rules": {"field": "[Total]", "cases": cases}}}),
    );
    assert!(e.contains("1 to 50"), "{e}");
}

/// Desktop's data bars with explicit bounds (`minValue`/`maxValue` after `hideText`, numbers such
/// as `1.2e+06D`) on the second binding of a measure (Desktop gives it its own `queryRef`):
/// extracted with `min`/`max` and `occurrence`, extract → apply changes nothing, and bicraft
/// writes the same shape back.
#[test]
fn desktop_data_bars_with_bounds_round_trip() {
    let mut s = session();
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Rate (pct)", "expression": "DIVIDE([Total], 100)"}),
    );
    let v = add(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Region]"}, {"role": "Values", "field": "[Rate (pct)]"}]),
    );
    let pg = page(&s);
    // The measure bound a second time, as Desktop does when a field is added twice.
    let second = "Sales.Rate (pct) (bar)";
    {
        let vis = s
            .report
            .as_mut()
            .unwrap()
            .page_mut(&pg)
            .unwrap()
            .visual_mut(&v)
            .unwrap();
        let mut query = bicraft_report::query::effective(vis).unwrap();
        let values = query["queryState"]["Values"]["projections"]
            .as_array_mut()
            .unwrap();
        let mut dup = values[1].clone();
        dup["queryRef"] = json!(second);
        dup["nativeQueryRef"] = json!("Rate (pct) (bar)");
        values.push(dup);
        let field = vis.fields[1].clone();
        vis.fields.push(field);
        vis.visual_extra.insert("query".into(), query);
    }
    let color = |c: &str| json!({"solid": {"color": {"expr": lit(c)}}});
    let bars = |min: &str, max: &str| {
        json!({
            "positiveColor": color("'#118DFF'"),
            "negativeColor": color("'#D64550'"),
            "axisColor": color("'#000000'"),
            "reverseDirection": {"expr": lit("false")},
            "hideText": {"expr": lit("true")},
            "minValue": {"expr": lit(min)},
            "maxValue": {"expr": lit(max)},
        })
    };
    for (min, max, want_max) in [("0D", "1.2e+06D", json!(1_200_000)), ("0D", "0.5D", json!(0.5))] {
        let entry = json!([{"properties": {"dataBars": bars(min, max)}, "selector": {"metadata": second}}]);
        let vis = s
            .report
            .as_mut()
            .unwrap()
            .page_mut(&pg)
            .unwrap()
            .visual_mut(&v)
            .unwrap();
        vis.visual_extra
            .insert("objects".into(), json!({"columnFormatting": entry.clone()}));
        let props = extract_props(visual(&s, &v));
        let list = props["conditional"].as_array().cloned().unwrap_or_default();
        assert_eq!(list.len(), 1, "{props}");
        assert_eq!(list[0]["target"], "data_bars", "{props}");
        assert_eq!(list[0]["series"], "[Rate (pct)]", "{props}");
        assert_eq!(list[0]["occurrence"], 2, "{props}");
        let db = &list[0]["rule"]["data_bars"];
        assert_eq!(db["min"], json!(0), "{props}");
        assert_eq!(db["max"], want_max, "{props}");
        assert_extract_noop(&s, visual(&s, &v));
        // Written back by bicraft: the same entry, in Desktop's key order.
        let mut p = list[0].clone();
        p["page"] = json!(pg);
        p["visual"] = json!(v);
        run(
            &mut s,
            "visual.conditional",
            json!({"page": pg, "visual": v, "target": "data_bars", "series": "[Rate (pct)]", "occurrence": 2,
                   "rule": null}),
        );
        assert_eq!(objects(&s, &v), Value::Null);
        run(&mut s, "visual.conditional", p);
        assert_eq!(objects(&s, &v)["columnFormatting"], entry);
        let written = &objects(&s, &v)["columnFormatting"][0]["properties"]["dataBars"];
        let keys: Vec<&str> = written.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "positiveColor",
                "negativeColor",
                "axisColor",
                "reverseDirection",
                "hideText",
                "minValue",
                "maxValue"
            ]
        );
    }
    // The first binding is still addressable, and a binding that is not there is refused.
    let e = fails(
        &mut s,
        "visual.conditional",
        json!({"page": pg, "visual": v, "series": "[Rate (pct)]", "occurrence": 3, "rule": {"data_bars": {}}}),
    );
    assert!(e.contains("not bound 3 times"), "{e}");
}

#[test]
fn data_bar_bounds_are_written_like_desktop() {
    use crate::visual_conditional::bound_text;
    for (x, t) in [
        (1_200_000.0, "1.2e+06"),
        (1000.0, "1000"),
        (0.5, "0.5"),
        (0.0, "0"),
        (-2.5e7, "-2.5e+07"),
        (0.00001, "1e-05"),
        (1_234_567.0, "1234567"),
        (0.123456789, "0.123456789"),
    ] {
        assert_eq!(bound_text(x), t, "{x}");
    }
}

fn container(s: &Session, name: &str) -> Value {
    visual(s, name)
        .visual_extra
        .get("visualContainerObjects")
        .cloned()
        .unwrap_or(Value::Null)
}

/// The container's targets (title, subtitle, background, border) write one value for the whole
/// visual in `visualContainerObjects`, in the object's entry without selector, beside its plain
/// properties; they read back with their names and take no series.
#[test]
fn container_targets_write_visual_container_objects() {
    let mut s = session();
    let chart = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let pg = page(&s);
    run(
        &mut s,
        "visual.set_title",
        json!({"page": pg, "visual": chart, "title": {"text": "Sales", "font_size": 14}}),
    );
    cond(
        &mut s,
        &chart,
        json!({"target": "title_color", "rule": {"rules": {"field": "[Total]",
            "cases": [{"op": ">", "value": 100, "color": "#00AA00"}], "else": "#AA0000"}}}),
    );
    cond(
        &mut s,
        &chart,
        json!({"target": "title_text", "rule": {"field_value": {"field": "[Total]"}}}),
    );
    cond(
        &mut s,
        &chart,
        json!({"target": "visual_background",
               "rule": {"gradient": {"field": "[Total]", "min": "#FFFFFF", "max": "#118DFF"}}}),
    );
    cond(
        &mut s,
        &chart,
        json!({"target": "border_color", "rule": {"field_value": {"field": "Sales[Color]"}}}),
    );
    cond(
        &mut s,
        &chart,
        json!({"target": "subtitle_color", "rule": {"field_value": {"field": "Sales[Color]"}}}),
    );
    let c = container(&s, &chart);
    let title = &c["title"];
    assert_eq!(title.as_array().unwrap().len(), 1, "{c}");
    assert!(title[0].get("selector").is_none(), "{c}");
    let tp = &title[0]["properties"];
    assert_eq!(tp["text"], json!({"expr": total()}), "{c}");
    let rules = &tp["fontColor"]["solid"]["color"]["expr"]["Conditional"];
    assert_eq!(rules["Cases"][0]["Value"], lit("'#00AA00'"), "{c}");
    assert_eq!(rules["DefaultValue"], lit("'#AA0000'"), "{c}");
    // the plain title properties stay beside the rule
    assert!(tp.get("fontSize").is_some(), "{c}");
    let bg = &c["background"][0];
    assert!(bg.get("selector").is_none(), "{c}");
    assert!(
        bg["properties"]["color"]["solid"]["color"]["expr"]["FillRule"]["FillRule"]
            .get("linearGradient2")
            .is_some(),
        "{c}"
    );
    assert_eq!(
        c["border"][0]["properties"]["color"]["solid"]["color"]["expr"]["Aggregation"]["Function"], 3,
        "{c}"
    );
    assert!(c["subTitle"][0]["properties"]["fontColor"].is_object(), "{c}");
    // nothing went to the visual's objects
    assert!(objects(&s, &chart).get("title").is_none());
    let conditional = extract_props(visual(&s, &chart))["conditional"].clone();
    let mut targets: Vec<&str> = conditional
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["target"].as_str())
        .collect();
    targets.sort_unstable();
    assert_eq!(
        targets,
        [
            "border_color",
            "subtitle_color",
            "title_color",
            "title_text",
            "visual_background"
        ],
        "{conditional}"
    );
    assert!(
        conditional
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i.get("series").is_none())
    );
    schema_ok(&mut s);
    // a series or apply_to is refused
    let e = fails(
        &mut s,
        "visual.conditional",
        json!({"page": pg, "visual": chart, "target": "title_color", "series": "[Total]",
               "rule": {"field_value": {"field": "Sales[Color]"}}}),
    );
    assert!(e.contains("no series"), "{e}");
    let e = fails(
        &mut s,
        "visual.conditional",
        json!({"page": pg, "visual": chart, "target": "visual_background", "apply_to": "totals",
               "rule": {"field_value": {"field": "Sales[Color]"}}}),
    );
    assert!(e.contains("apply_to"), "{e}");
    // a gradient cannot give a title's text
    fails(
        &mut s,
        "visual.conditional",
        json!({"page": pg, "visual": chart, "target": "title_text",
               "rule": {"gradient": {"field": "[Total]", "min": "#FFFFFF", "max": "#000000"}}}),
    );
    // rule null removes one; the title's text and size stay
    run(
        &mut s,
        "visual.conditional",
        json!({"page": pg, "visual": chart, "target": "title_color", "rule": null}),
    );
    let tp = &container(&s, &chart)["title"][0]["properties"];
    assert!(tp.get("fontColor").is_none() && tp.get("text").is_some() && tp.get("fontSize").is_some());
    // on a table, a container target needs no series either
    let t = table(&mut s);
    cond(
        &mut s,
        &t,
        json!({"target": "title_color", "rule": {"field_value": {"field": "Sales[Color]"}}}),
    );
    assert!(container(&s, &t)["title"][0]["properties"]["fontColor"].is_object());
}

/// A chart's data label colour and a card's category label colour take a rule per data point
/// (a wildcard selector, as the data colour).
#[test]
fn data_label_and_category_label_colours() {
    let mut s = session();
    let chart = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]),
    );
    cond(
        &mut s,
        &chart,
        json!({"target": "label_color",
               "rule": {"gradient": {"field": "[Total]", "min": "#000000", "max": "#FF0000"}}}),
    );
    let l = &objects(&s, &chart)["labels"][0];
    assert_eq!(
        l["selector"],
        json!({"data": [{"dataViewWildcard": {"matchingOption": 1}}]}),
        "{l}"
    );
    assert!(l["properties"]["color"]["solid"]["color"]["expr"]["FillRule"].is_object());
    let card = add(&mut s, "card", json!([{"role": "Values", "field": "[Total]"}]));
    cond(
        &mut s,
        &card,
        json!({"target": "category_color", "rule": {"field_value": {"field": "Sales[Color]"}}}),
    );
    assert!(objects(&s, &card)["categoryLabels"][0]["properties"]["color"].is_object());
    assert_eq!(
        extract_props(visual(&s, &card))["conditional"][0]["target"],
        "category_color"
    );
    schema_ok(&mut s);
}

/// The axes', legend's, gauge's and KPI's colours (G54) are one value for the whole visual,
/// written in their object's entry without selector beside its plain properties; they read
/// back by name and need the property in the type's catalog.
#[test]
fn whole_visual_targets_write_their_object_without_selector() {
    let mut s = session();
    let chart = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let pg = page(&s);
    run(
        &mut s,
        "visual.format",
        json!({"page": pg, "visual": chart,
               "objects": [{"object": "categoryAxis", "property": "fontSize", "value": 11}]}),
    );
    let rules = json!({"rules": {"field": "[Total]",
        "cases": [{"op": ">", "value": 100, "color": "#00AA00"}], "else": "#AA0000"}});
    cond(
        &mut s,
        &chart,
        json!({"target": "category_axis_color", "rule": rules}),
    );
    cond(
        &mut s,
        &chart,
        json!({"target": "value_title_color", "rule": {"field_value": {"field": "Sales[Color]"}}}),
    );
    cond(
        &mut s,
        &chart,
        json!({"target": "legend_color",
               "rule": {"gradient": {"field": "[Total]", "min": "#FFFFFF", "max": "#118DFF"}}}),
    );
    // named as {object, property}, it reads back under its name
    cond(
        &mut s,
        &chart,
        json!({"target": {"object": "valueAxis", "property": "labelColor"},
               "rule": {"field_value": {"field": "Sales[Color]"}}}),
    );
    let o = objects(&s, &chart);
    let cat = o["categoryAxis"].as_array().unwrap();
    assert_eq!(cat.len(), 1, "{o}");
    assert!(cat[0].get("selector").is_none(), "{o}");
    assert!(cat[0]["properties"]["fontSize"].is_object(), "{o}");
    assert!(
        cat[0]["properties"]["labelColor"]["solid"]["color"]["expr"]["Conditional"].is_object(),
        "{o}"
    );
    assert!(o["valueAxis"][0].get("selector").is_none(), "{o}");
    assert!(o["valueAxis"][0]["properties"]["titleColor"].is_object(), "{o}");
    assert!(o["valueAxis"][0]["properties"]["labelColor"].is_object(), "{o}");
    assert!(o["legend"][0]["properties"]["labelColor"]["solid"]["color"]["expr"]["FillRule"].is_object());
    let conditional = extract_props(visual(&s, &chart))["conditional"].clone();
    let mut targets: Vec<&str> = conditional
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["target"].as_str())
        .collect();
    targets.sort_unstable();
    assert_eq!(
        targets,
        [
            "category_axis_color",
            "legend_color",
            "value_axis_color",
            "value_title_color"
        ],
        "{conditional}"
    );
    schema_ok(&mut s);
    // a series is refused; a column chart has no gauge target
    let e = fails(
        &mut s,
        "visual.conditional",
        json!({"page": pg, "visual": chart, "target": "legend_color", "series": "[Total]",
               "rule": {"field_value": {"field": "Sales[Color]"}}}),
    );
    assert!(e.contains("no series"), "{e}");
    let e = fails(
        &mut s,
        "visual.conditional",
        json!({"page": pg, "visual": chart, "target": "target_color",
               "rule": {"field_value": {"field": "Sales[Color]"}}}),
    );
    assert!(e.contains("has no dataPoint.target"), "{e}");
    // rule null removes it; the axis's font size stays
    run(
        &mut s,
        "visual.conditional",
        json!({"page": pg, "visual": chart, "target": "category_axis_color", "rule": null}),
    );
    let cat = &objects(&s, &chart)["categoryAxis"][0]["properties"];
    assert!(
        cat.get("labelColor").is_none() && cat.get("fontSize").is_some(),
        "{cat}"
    );
    // a gauge's target, target label and callout; a KPI's status, indicator, goal and distance
    let gauge = add(
        &mut s,
        "gauge",
        json!([{"role": "Y", "field": "[Total]"}, {"role": "TargetValue", "field": "[Total]"}]),
    );
    for t in ["target_color", "target_label_color", "callout_color"] {
        cond(&mut s, &gauge, json!({"target": t, "rule": rules}));
    }
    let o = objects(&s, &gauge);
    assert!(o["dataPoint"][0].get("selector").is_none(), "{o}");
    assert!(o["dataPoint"][0]["properties"]["target"].is_object(), "{o}");
    assert!(o["target"][0]["properties"]["color"].is_object(), "{o}");
    assert!(o["calloutValue"][0]["properties"]["color"].is_object(), "{o}");
    let kpi = add(
        &mut s,
        "kpi",
        json!([{"role": "Indicator", "field": "[Total]"}, {"role": "TrendLine", "field": "Sales[Region]"}]),
    );
    for t in [
        "good_color",
        "neutral_color",
        "bad_color",
        "indicator_color",
        "goal_color",
        "distance_color",
    ] {
        cond(&mut s, &kpi, json!({"target": t, "rule": rules}));
    }
    let o = objects(&s, &kpi);
    for (obj, prop) in [
        ("status", "goodColor"),
        ("status", "badColor"),
        ("indicator", "fontColor"),
        ("goals", "goalFontColor"),
        ("goals", "distanceFontColor"),
    ] {
        assert!(o[obj][0].get("selector").is_none(), "{o}");
        assert!(o[obj][0]["properties"][prop].is_object(), "{obj}.{prop}: {o}");
    }
    assert_eq!(
        extract_props(visual(&s, &kpi))["conditional"]
            .as_array()
            .map(Vec::len),
        Some(6)
    );
    schema_ok(&mut s);
}

/// Gradient Percent stops (G54): `{color, value, percent: true}` writes the stop's value as the
/// Percent rules' bound (inferred shape), reads back as written, and takes 0 to 100 only.
#[test]
fn gradient_stops_take_percents() {
    let mut s = session();
    let chart = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let rule = json!({"gradient": {"field": "[Total]",
        "min": {"color": "#FF0000", "value": 20, "percent": true},
        "mid": {"color": "#FFFF00", "value": 50, "percent": true},
        "max": {"color": "#00AA00", "value": 1000}}});
    cond(&mut s, &chart, json!({"target": "data_color", "rule": rule}));
    let g = &objects(&s, &chart)["dataPoint"][0]["properties"]["fill"]["solid"]["color"]["expr"]["FillRule"]
        ["FillRule"]["linearGradient3"];
    let agg = |f: u64| json!({"Aggregation": {"Expression": total(), "Function": f}});
    // a Percent stop's value is the Percent rules' bound of the gradient's input
    assert_eq!(
        g["mid"]["value"],
        json!({"Arithmetic": {
            "Left": agg(3),
            "Right": {"Arithmetic": {
                "Left": {"Arithmetic": {"Left": agg(4), "Right": agg(3), "Operator": 1}},
                "Right": lit("0.5D"),
                "Operator": 2}},
            "Operator": 0}}),
        "{g}"
    );
    assert_eq!(g["max"]["value"], lit("1000D"), "{g}");
    let props = extract_props(visual(&s, &chart));
    assert_eq!(props["conditional"][0]["rule"], rule, "{props}");
    schema_ok(&mut s);
    let pg = page(&s);
    for bad in [
        json!({"color": "#000000", "value": 120, "percent": true}),
        json!({"color": "#000000", "percent": true}),
        json!({"color": "#000000", "value": 5, "percent": 1}),
    ] {
        fails(
            &mut s,
            "visual.conditional",
            json!({"page": pg, "visual": chart, "target": "data_color",
                   "rule": {"gradient": {"field": "[Total]", "min": bad, "max": "#FFFFFF"}}}),
        );
    }
}

/// Rules' Percent bounds (G54): `percent: true` writes each bound as `Min + (Max - Min) * p`
/// of the rule field, reads back as written, and takes 0 to 100 only.
#[test]
fn rules_take_percent_bounds() {
    let mut s = session();
    let chart = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let cases = json!([
        {"op": ">=", "value": 67, "percent": true, "color": "#00AA00"},
        {"gte": 33, "lt": 67, "percent": true, "color": "#AAAA00"},
        {"op": "<", "value": 10, "color": "#AA0000"}]);
    cond(
        &mut s,
        &chart,
        json!({"target": "data_color", "rule": {"rules": {"field": "[Total]", "cases": cases}}}),
    );
    let c =
        &objects(&s, &chart)["dataPoint"][0]["properties"]["fill"]["solid"]["color"]["expr"]["Conditional"];
    let agg = |f: u64| json!({"Aggregation": {"Expression": total(), "Function": f}});
    assert_eq!(
        c["Cases"][0]["Condition"]["Comparison"]["Right"],
        json!({"Arithmetic": {
            "Left": agg(3),
            "Right": {"Arithmetic": {
                "Left": {"Arithmetic": {"Left": agg(4), "Right": agg(3), "Operator": 1}},
                "Right": lit("0.67D"),
                "Operator": 2}},
            "Operator": 0}}),
        "{c}"
    );
    assert_eq!(
        c["Cases"][2]["Condition"]["Comparison"]["Right"],
        lit("10D"),
        "{c}"
    );
    let props = extract_props(visual(&s, &chart));
    assert_eq!(
        props["conditional"][0]["rule"]["rules"]["cases"], cases,
        "{props}"
    );
    schema_ok(&mut s);
    let pg = page(&s);
    for bad in [
        json!({"op": ">", "value": 150, "percent": true, "color": "#000000"}),
        json!({"op": ">", "value": 5, "percent": "yes", "color": "#000000"}),
    ] {
        fails(
            &mut s,
            "visual.conditional",
            json!({"page": pg, "visual": chart, "target": "data_color",
                   "rule": {"rules": {"field": "[Total]", "cases": [bad]}}}),
        );
    }
}
