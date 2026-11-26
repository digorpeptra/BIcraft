//! Visual details in the snapshot: text boxes, buttons, colors, column styles, analytics lines,
//! small multiples, subtitle and axes.

use serde_json::{Value, json};

use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn session() -> Session {
    let mut s = Session::new();
    let steps = [
        ("model.new", json!({"name": "M"})),
        ("table.add", json!({"name": "Sales"})),
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
        ("page.add", json!({"display_name": "Main"})),
    ];
    for (id, p) in steps {
        run(&mut s, id, p);
    }
    s
}

fn add_visual(s: &mut Session, visual_type: &str, fields: Value) -> String {
    let r = run(
        s,
        "visual.add",
        json!({"page": "Main", "visual_type": visual_type, "x": 0, "y": 0, "width": 300, "height": 200,
               "fields": fields}),
    );
    r.get("visual")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| panic!("no visual name in {r}"))
}

fn set_objects(s: &mut Session, visual: &str, objects: Value) {
    if let Some(r) = s.report.as_mut() {
        for v in r.pages.iter_mut().flat_map(|p| p.visuals.iter_mut()) {
            if v.name == visual {
                v.visual_extra.insert("objects".into(), objects.clone());
            }
        }
    }
    s.revision += 1;
}

fn visual_json(s: &Session, visual: &str) -> Value {
    let snap = s.snapshot();
    for p in snap["report"]["pages"].as_array().cloned().unwrap_or_default() {
        for v in p["visuals"].as_array().cloned().unwrap_or_default() {
            if v["name"] == visual {
                return v;
            }
        }
    }
    panic!("visual {visual} not in the snapshot")
}

fn lit(v: &str) -> Value {
    json!({"expr": {"Literal": {"Value": v}}})
}

fn color(c: &str) -> Value {
    json!({"solid": {"color": lit(&format!("'{c}'"))}})
}

#[test]
fn text_boxes_carry_their_paragraphs() {
    let mut s = session();
    let t = add_visual(&mut s, "textbox", json!([]));
    set_objects(
        &mut s,
        &t,
        json!({"general": [{"properties": {"paragraphs": [
            {"textRuns": [{"value": "Hello ", "textStyle": {"fontWeight": "bold", "fontSize": "14pt", "color": "#FF0000"}},
                          {"value": "world", "textStyle": {"fontStyle": "italic", "fontFamily": "Segoe UI"}, "url": "https://example.com"}],
             "horizontalTextAlignment": "center"},
            {"textRuns": [{"value": ""}]}
        ]}}]}),
    );
    let v = visual_json(&s, &t);
    assert_eq!(
        v["text"],
        json!({"paragraphs": [
            {"align": "center", "runs": [
                {"text": "Hello ", "bold": true, "font_size": 14, "color": "#FF0000"},
                {"text": "world", "italic": true, "font_family": "Segoe UI", "url": "https://example.com"}]},
            {"runs": [{"text": ""}]}
        ]})
    );
}

#[test]
fn buttons_carry_their_default_state() {
    let mut s = session();
    let b = add_visual(&mut s, "actionButton", json!([]));
    set_objects(
        &mut s,
        &b,
        json!({
            "text": [{"properties": {"show": lit("true"), "text": lit("'Base'")}},
                     {"properties": {"text": lit("'Go'"), "fontSize": lit("12D"), "fontColor": color("#112233")},
                      "selector": {"id": "default"}}],
            "icon": [{"properties": {"shapeType": lit("'back'"), "show": lit("true")}}],
            "fill": [{"properties": {"show": lit("true"), "fillColor": color("#EEEEEE"), "transparency": lit("20D")}}]
        }),
    );
    assert_eq!(
        visual_json(&s, &b)["button"],
        json!({"text": "Go", "font_size": 12, "font_color": "#112233", "icon": "back", "show_icon": true,
               "show_fill": true, "fill": "#EEEEEE", "fill_transparency": 20})
    );
}

#[test]
fn colors_styles_lines_and_small_multiples() {
    let mut s = session();
    let c = add_visual(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Product]"}, {"role": "Y", "field": "[Total]"}]),
    );
    run(
        &mut s,
        "visual.series",
        json!({"page": "Main", "visual": c, "target": "[Total]", "color": "#123456"}),
    );
    run(
        &mut s,
        "visual.series",
        json!({"page": "Main", "visual": c, "target": "Sales[Product]=Bike", "color": "#654321"}),
    );
    run(
        &mut s,
        "visual.analytics",
        json!({"page": "Main", "visual": c, "lines": [
            {"kind": "constant", "value": 100, "color": "#FF0000", "label": "Goal"},
            {"kind": "average"}]}),
    );
    run(
        &mut s,
        "visual.axis",
        json!({"page": "Main", "visual": c, "axis": "y", "min": 0, "max": 500, "scale": "log", "title": false}),
    );
    let v = visual_json(&s, &c);
    let f = &v["format"];
    let colors = f["series_colors"].as_array().cloned().unwrap_or_default();
    // a single-series chart's color is the default data point color
    assert!(colors.contains(&json!({"color": "#123456"})), "{f}");
    assert!(
        colors
            .contains(&json!({"field": "Sales[Product]", "column": 0, "value": "Bike", "color": "#654321"})),
        "{f}"
    );
    let lines = f["reference_lines"].as_array().cloned().unwrap_or_default();
    assert_eq!(lines.len(), 2, "{f}");
    assert_eq!(lines[0]["kind"], "constant");
    assert_eq!(lines[0]["value"], 100);
    assert_eq!(lines[0]["label"], "Goal");
    assert_eq!(lines[1]["kind"], "average");
    assert_eq!(lines[1]["column"], 1);
    assert_eq!(f["y_axis"]["min"], 0, "{f}");
    assert_eq!(f["y_axis"]["max"], 500);
    assert_eq!(f["y_axis"]["log"], true);
    assert_eq!(f["y_axis"]["title"], false);

    // small multiples split by a column bound to the chart's Rows role (bound after Category)
    let sm = add_visual(
        &mut s,
        "lineChart",
        json!([{"role": "Category", "field": "Sales[Amount]"}, {"role": "Y", "field": "[Total]"}]),
    );
    run(
        &mut s,
        "visual.small_multiples",
        json!({"page": "Main", "visual": sm, "field": "Sales[Product]", "rows": 2, "columns": 3}),
    );
    assert_eq!(
        visual_json(&s, &sm)["format"]["small_multiples"],
        json!({"column": 1, "rows": 2, "columns": 3})
    );
}

#[test]
fn table_column_styles_and_subtitles() {
    let mut s = session();
    let t = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "[Total]"}]),
    );
    run(
        &mut s,
        "visual.format",
        json!({"page": "Main", "visual": t, "subtitle": {"text": "Sub", "show": true}}),
    );
    let mut objects = json!({"values": [{"properties": {"backColor": color("#ABCDEF"), "fontColor": color("#010203")},
                                          "selector": {"metadata": "Sales.Total"}}]});
    // the query refs come from the PBIR query: give the visual one
    if let Some(r) = s.report.as_mut() {
        for v in r.pages.iter_mut().flat_map(|p| p.visuals.iter_mut()) {
            if v.name == t {
                v.visual_extra.insert(
                    "query".into(),
                    bicraft_report::query::regenerate(None, &v.fields).unwrap_or(Value::Null),
                );
                if let Some(o) = v.visual_extra.get("objects").cloned()
                    && let (Some(a), Some(b)) = (o.as_object(), objects.as_object_mut())
                {
                    for (k, x) in a {
                        b.entry(k.clone()).or_insert(x.clone());
                    }
                }
            }
        }
    }
    set_objects(&mut s, &t, objects);
    let v = visual_json(&s, &t);
    assert_eq!(
        v["format"]["column_styles"],
        json!([{"bg": "#ABCDEF", "fg": "#010203", "column": 1}])
    );
    assert_eq!(v["format"]["subtitle"]["text"], "Sub", "{}", v["format"]);
    assert_eq!(v["format"]["subtitle"]["show"], true);
}

/// Text runs, a slicer selection, a subtitle and a divider, as Desktop writes them.
fn paragraphs() -> Value {
    json!([
        {"textRuns": [{"value": "Title", "textStyle": {"fontWeight": "700", "fontSize": "18px", "color": "#123456"}}],
         "horizontalTextAlignment": "right"},
        {"textRuns": [{"value": "see "}, {"value": "docs", "url": "https://example.com/docs",
                       "textStyle": {"textDecoration": "underline"}}]}
    ])
}

fn subtitle_and_divider() -> Value {
    json!({
        "subTitle": [{"properties": {"show": lit("true"), "text": lit("'Second line'"),
            "fontFamily": lit("'Segoe UI Light'"), "italic": lit("true"), "fontSize": lit("9D"),
            "fontColor": color("#666666"), "alignment": lit("'center'")}}],
        "divider": [{"properties": {"show": lit("true"), "color": color("#C8C6C4"),
            "width": lit("2D"), "style": lit("'Dashed'")}}]
    })
}

#[test]
fn subtitles_carry_their_font_and_dividers_their_line() {
    let mut s = session();
    let t = add_visual(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Product]"}, {"role": "Y", "field": "[Total]"}]),
    );
    if let Some(r) = s.report.as_mut() {
        for v in r.pages.iter_mut().flat_map(|p| p.visuals.iter_mut()) {
            if v.name == t {
                v.visual_extra
                    .insert("visualContainerObjects".into(), subtitle_and_divider());
            }
        }
    }
    s.revision += 1;
    let f = &visual_json(&s, &t)["format"];
    assert_eq!(
        f["subtitle"],
        json!({"show": true, "text": "Second line", "font_family": "Segoe UI Light", "italic": true,
               "font_size": 9, "color": "#666666", "alignment": "center"}),
        "{f}"
    );
    assert_eq!(
        f["divider"],
        json!({"show": true, "color": "#C8C6C4", "width": 2, "style": "dashed"})
    );
}

#[test]
fn legacy_layout_text_boxes_slicers_and_subtitles() {
    let textbox = json!({"name": "tb", "singleVisual": {"visualType": "textbox",
        "objects": {"general": [{"properties": {"paragraphs": paragraphs()}}]},
        "vcObjects": subtitle_and_divider()}});
    let filter = json!({"Version": 2, "From": [{"Name": "s", "Entity": "Sales", "Type": 0}],
        "Where": [{"Condition": {"In": {
            "Expressions": [{"Column": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Product"}}],
            "Values": [[{"Literal": {"Value": "'Bike'"}}], [{"Literal": {"Value": "'Car'"}}]]}}}]});
    let slicer = json!({"name": "sl", "singleVisual": {"visualType": "slicer",
        "projections": {"Values": [{"queryRef": "Sales.Product"}]},
        "prototypeQuery": {"Version": 2, "From": [{"Name": "s", "Entity": "Sales", "Type": 0}],
            "Select": [{"Column": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Product"},
                        "Name": "Sales.Product"}]},
        "objects": {"general": [{"properties": {"filter": {"filter": filter}}}]}}});
    let container = |cfg: &Value| json!({"x": 0, "y": 0, "z": 0, "width": 200, "height": 100, "config": cfg.to_string(), "filters": "[]"});
    let layout = json!({"config": "{}", "sections": [{"name": "ReportSection", "displayName": "Main",
        "width": 1280, "height": 720, "config": "{}", "filters": "[]",
        "visualContainers": [container(&textbox), container(&slicer)]}]});
    let report = bicraft_layout::read_layout(&layout.to_string()).expect("legacy layout reads");
    let mut s = session();
    s.report = Some(report);
    s.revision += 1;
    let tb = visual_json(&s, "tb");
    assert_eq!(
        tb["text"],
        json!({"paragraphs": [
            {"align": "right", "runs": [{"text": "Title", "bold": true, "font_size": 13.5, "color": "#123456"}]},
            {"runs": [{"text": "see "}, {"text": "docs", "underline": true, "url": "https://example.com/docs"}]}
        ]})
    );
    assert_eq!(tb["format"]["subtitle"]["text"], "Second line");
    assert_eq!(tb["format"]["divider"]["style"], "dashed");
    assert_eq!(
        visual_json(&s, "sl")["slicer"],
        json!({"mode": "list", "selected": ["Bike", "Car"]})
    );
}

#[test]
fn a_date_slicer_without_a_mode_is_a_range() {
    // Power BI Desktop 2.159.1231.0 (2026-11-24) drew a slicer on a date-time column written
    // without `data.mode` as the between range (two date inputs and a slider), not a list
    let mut s = session();
    run(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "Day", "data_type": "dateTime"}),
    );
    let day = add_visual(
        &mut s,
        "slicer",
        json!([{"role": "Values", "field": "Sales[Day]"}]),
    );
    let product = add_visual(
        &mut s,
        "slicer",
        json!([{"role": "Values", "field": "Sales[Product]"}]),
    );
    assert_eq!(visual_json(&s, &day)["slicer"]["mode"], "between");
    assert_eq!(visual_json(&s, &product)["slicer"]["mode"], "list");
    // a mode written by Desktop stays as written
    set_objects(
        &mut s,
        &day,
        json!({"data": [{"properties": {"mode": lit("'Basic'")}}]}),
    );
    assert_eq!(visual_json(&s, &day)["slicer"]["mode"], "list");
}

#[test]
fn pbir_projects_keep_text_boxes_slicers_and_subtitles() {
    let mut s = session();
    let t = add_visual(&mut s, "textbox", json!([]));
    let sl = add_visual(
        &mut s,
        "slicer",
        json!([{"role": "Values", "field": "Sales[Product]"}]),
    );
    let filter = json!({"Version": 2, "From": [{"Name": "s", "Entity": "Sales", "Type": 0}],
        "Where": [{"Condition": {"In": {
            "Expressions": [{"Column": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Product"}}],
            "Values": [[{"Literal": {"Value": "'Bike'"}}]]}}}]});
    set_objects(
        &mut s,
        &t,
        json!({"general": [{"properties": {"paragraphs": paragraphs()}}]}),
    );
    set_objects(
        &mut s,
        &sl,
        json!({"general": [{"properties": {"filter": {"filter": filter}}}],
               "data": [{"properties": {"mode": lit("'Dropdown'")}}]}),
    );
    if let Some(r) = s.report.as_mut() {
        for v in r.pages.iter_mut().flat_map(|p| p.visuals.iter_mut()) {
            if v.name == t {
                v.visual_extra
                    .insert("visualContainerObjects".into(), subtitle_and_divider());
            }
        }
    }
    s.revision += 1;
    let before = (visual_json(&s, &t), visual_json(&s, &sl));
    assert_eq!(
        before.1["slicer"],
        json!({"mode": "dropdown", "selected": ["Bike"]})
    );

    let tmp = crate::tempdir::TempDir::new("engine-snapshot-visual");
    run(&mut s, "project.new", json!({"dir": tmp.path(), "name": "Shop"}));
    let mut fresh = Session::new();
    run(
        &mut fresh,
        "file.open",
        json!({"path": tmp.path().join("Shop.pbip")}),
    );
    let after = (visual_json(&fresh, &t), visual_json(&fresh, &sl));
    assert_eq!(before.0["text"], after.0["text"]);
    assert_eq!(before.0["format"]["subtitle"], after.0["format"]["subtitle"]);
    assert_eq!(before.0["format"]["divider"], after.0["format"]["divider"]);
    assert_eq!(before.1["slicer"], after.1["slicer"]);
}

#[test]
fn category_layout_and_secondary_axis_keys() {
    let mut s = session();
    let c = add_visual(
        &mut s,
        "lineClusteredColumnComboChart",
        json!([{"role": "Category", "field": "Sales[Product]"}, {"role": "Y", "field": "[Total]"}]),
    );
    set_objects(
        &mut s,
        &c,
        json!({
            "categoryAxis": [{"properties": {"innerPadding": lit("35L"), "preferredCategoryWidth": lit("30D"),
                                             "maxMarginFactor": lit("40L"), "concatenateLabels": lit("false")}}],
            "valueAxis": [{"properties": {"secShow": lit("false"), "secStart": lit("10D"), "secEnd": lit("90D"),
                                          "secLabelDisplayUnits": lit("1000D")}}]
        }),
    );
    let f = visual_json(&s, &c)["format"].clone();
    assert_eq!(f["x_axis"]["inner_padding"], 35, "{f}");
    assert_eq!(f["x_axis"]["category_width"], 30);
    assert_eq!(f["x_axis"]["max_margin"], 40);
    assert_eq!(f["x_axis"]["concatenate"], false);
    assert_eq!(f["y2_axis"]["show"], false);
    assert_eq!(f["y2_axis"]["min"], 10);
    assert_eq!(f["y2_axis"]["max"], 90);
    assert_eq!(f["y2_axis"]["display_units"], 1000);
    assert!(f["y_axis"].get("min").is_none(), "{f}");
}
