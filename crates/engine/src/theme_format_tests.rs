use bicraft_report::{FieldBinding, FieldRef, Visual};
use serde_json::{Map, Value, json};

use super::super::{Theme, builtin, visual_format};
use super::series_format;

fn lit(v: &str) -> Value {
    json!({"expr": {"Literal": {"Value": v}}})
}

fn color(c: &str) -> Value {
    json!({"solid": {"color": lit(&format!("'{c}'"))}})
}

fn visual(ty: &str, objects: Value) -> Visual {
    let mut v = Visual {
        name: "v".into(),
        visual_type: Some(ty.into()),
        ..Visual::default()
    };
    v.visual_extra.insert("objects".into(), objects);
    v
}

fn theme(styles: Value) -> Theme {
    let mut t = builtin("default");
    t.styles.push(styles);
    t
}

fn fmt(v: &Visual, t: Option<&Theme>) -> Map<String, Value> {
    match visual_format(v, t) {
        Value::Object(m) => m,
        other => panic!("no format: {other}"),
    }
}

#[test]
fn labels_from_theme_then_visual() {
    let t = theme(json!({"lineChart": {"*": {"labels": [{
        "labelDisplayUnits": 1000, "labelPrecision": 1, "enableBackground": true,
        "backgroundColor": {"solid": {"color": "#EEEEEE"}}, "labelDensity": 50
    }]}}}));
    let v = visual(
        "lineChart",
        json!({"labels": [{"properties": {
            "show": lit("true"), "labelDisplayUnits": lit("1000000D"),
            "labelPosition": lit("'Above'"), "backgroundTransparency": lit("40D")
        }}]}),
    );
    let f = fmt(&v, Some(&t));
    let l = &f["labels"];
    assert_eq!(l["display_units"], json!(1_000_000));
    assert_eq!(l["precision"], json!(1));
    assert_eq!(l["position"], json!("above"));
    assert_eq!(l["density"], json!(50));
    assert_eq!(l["background"], json!(true));
    assert_eq!(l["background_color"], json!("#EEEEEE"));
    assert_eq!(l["background_transparency"], json!(40));
    assert_eq!(l["show"], json!(true));
    assert_eq!(f["data_labels"], json!(true));
}

#[test]
fn pie_label_style_background_and_slices() {
    let v = visual(
        "donutChart",
        json!({
            "labels": [{"properties": {
                "labelStyle": lit("'Category, percent of total'"),
                "position": lit("'preferOutside'"), "background": lit("'on'"),
                "percentageLabelPrecision": lit("2L")
            }}],
            "slices": [{"properties": {"innerRadiusRatio": lit("60L"), "startAngle": lit("90L")}}]
        }),
    );
    let f = fmt(&v, None);
    assert_eq!(f["labels"]["label_style"], json!("Category, percent of total"));
    assert_eq!(f["labels"]["position"], json!("preferoutside"));
    assert_eq!(f["labels"]["background"], json!(true));
    assert_eq!(f["labels"]["percent_precision"], json!(2));
    assert_eq!(f["pie"], json!({"inner_radius": 60, "start_angle": 90}));
    // `auto` clears a theme's `on`
    let t = theme(json!({"donutChart": {"*": {"labels": [{"background": "on"}]}}}));
    let v = visual(
        "donutChart",
        json!({"labels": [{"properties": {"background": lit("'auto'")}}]}),
    );
    let f = visual_format(&v, Some(&t));
    assert!(f.get("labels").and_then(|l| l.get("background")).is_none(), "{f}");
}

#[test]
fn line_markers_style_and_steps() {
    let t = theme(json!({"*": {"*": {"lineStyles": [{"strokeWidth": 3, "showMarker": false}]}}}));
    let v = visual(
        "lineChart",
        json!({"lineStyles": [{"properties": {
            "showMarker": lit("true"), "markerShape": lit("'diamond'"), "markerSize": lit("6D"),
            "lineStyle": lit("'Dashed'"), "lineChartType": lit("'step'")
        }}]}),
    );
    let f = fmt(&v, Some(&t));
    let l = &f["lines"];
    assert_eq!(l["width"], json!(3));
    assert_eq!(l["show_marker"], json!(true));
    assert_eq!(l["marker_shape"], json!("diamond"));
    assert_eq!(l["marker_size"], json!(6));
    assert_eq!(l["style"], json!("dashed"));
    assert_eq!(l["chart_type"], json!("step"));
    assert_eq!(l["stepped"], json!(true));
}

#[test]
fn per_series_labels_and_lines() {
    let v = visual(
        "lineChart",
        json!({
            "labels": [
                {"properties": {"show": lit("true")}},
                {"selector": {"metadata": "Sales.Total"},
                 "properties": {"labelDisplayUnits": lit("1000D"), "labelPrecision": lit("2L")}},
                {"selector": {"metadata": "Unknown.X"}, "properties": {"show": lit("false")}}
            ],
            "lineStyles": [{"selector": {"metadata": "Sales.Total"},
                            "properties": {"showMarker": lit("true"), "markerShape": lit("'square'"),
                                           "lineStyle": lit("'dotted'")}}]
        }),
    );
    let refs = vec![Some("Sales.Date".to_string()), Some("Sales.Total".to_string())];
    let out: Map<String, Value> = series_format(&v, &refs).into_iter().collect();
    assert_eq!(
        out["series_labels"],
        json!([{"column": 1, "display_units": 1000, "precision": 2}])
    );
    assert_eq!(
        out["series_lines"],
        json!([{"column": 1, "show_marker": true, "marker_shape": "square", "style": "dotted"}])
    );
    // the per-series entry does not leak into the whole-visual labels
    assert!(fmt(&v, None)["labels"].get("display_units").is_none());
}

#[test]
fn new_card_parts() {
    let v = visual(
        "cardVisual",
        json!({
            "accentBar": [{"properties": {"show": lit("true"), "position": lit("'Left'"),
                                          "color": color("#FF0000"), "width": lit("6D")}}],
            "value": [{"selector": {"id": "default"},
                       "properties": {"fontSize": lit("28D"), "labelDisplayUnits": lit("1000D"),
                                      "labelPrecision": lit("1L"), "fontColor": color("#123456")}}],
            "label": [{"properties": {"show": lit("false"), "position": lit("'aboveValue'")}}],
            "layout": [{"properties": {"rowCount": lit("2L"), "columnCount": lit("3L"),
                                       "style": lit("'Cards'"), "orientation": lit("'1'")}}],
            "image": [{"properties": {"show": lit("true"), "imageType": lit("'imageUrl'"),
                                      "imageUrl": lit("'https://example.com/a.png'"),
                                      "position": lit("'Left'")}}],
            "referenceLabelLayout": [{"properties": {"position": lit("'right'"),
                                                     "arrangement": lit("'columns'")}}],
            "referenceLabelValue": [{"properties": {"valueDisplayUnits": lit("1000000D")}}],
            "referenceLabelTitle": [{"properties": {"show": lit("true")}}]
        }),
    );
    let f = fmt(&v, None);
    let c = &f["card"];
    assert_eq!(
        c["accent_bar"],
        json!({"show": true, "position": "left", "color": "#FF0000", "width": 6})
    );
    assert_eq!(c["value"]["font_size"], json!(28));
    assert_eq!(c["value"]["display_units"], json!(1000));
    assert_eq!(c["value"]["precision"], json!(1));
    assert_eq!(c["value"]["color"], json!("#123456"));
    assert_eq!(c["label"], json!({"show": false, "position": "abovevalue"}));
    assert_eq!(
        c["layout"],
        json!({"rows": 2, "columns": 3, "style": "cards", "orientation": 1})
    );
    assert_eq!(c["image"]["url"], json!("https://example.com/a.png"));
    assert_eq!(c["image"]["position"], json!("left"));
    assert_eq!(c["reference_label"]["position"], json!("right"));
    assert_eq!(c["reference_label"]["arrangement"], json!("columns"));
    assert_eq!(c["reference_label"]["value"]["display_units"], json!(1_000_000));
    assert_eq!(c["reference_label"]["title"]["show"], json!(true));
    // not for other visual types
    let other = visual("card", v.visual_extra.get("objects").cloned().unwrap_or_default());
    assert!(visual_format(&other, None).get("card").is_none());
}

#[test]
fn gauge_axis_literal_and_measure_bound() {
    let mut v = visual(
        "gauge",
        json!({
            "axis": [{"properties": {
                "min": lit("0D"), "max": lit("500D"),
                "target": {"expr": {"Measure": {"Expression": {"SourceRef": {"Entity": "Sales"}},
                                                "Property": "Goal"}}}
            }}],
            "calloutValue": [{"properties": {"labelDisplayUnits": lit("1000D"), "labelPrecision": lit("0L"),
                                             "color": color("#333333")}}],
            "dataPoint": [{"properties": {"target": color("#00FF00")}}]
        }),
    );
    v.fields.push(FieldBinding {
        display_name: None,
        show_as: None,
        role: "Y".into(),
        field: FieldRef::Measure {
            table: "Sales".into(),
            measure: "Total".into(),
        },
    });
    v.fields.push(FieldBinding {
        display_name: None,
        show_as: None,
        role: "TargetValue".into(),
        field: FieldRef::Measure {
            table: "Sales".into(),
            measure: "Goal".into(),
        },
    });
    let f = fmt(&v, None);
    let g = &f["gauge"];
    assert_eq!(g["min"], json!(0));
    assert_eq!(g["max"], json!(500));
    assert_eq!(g["target"], json!({"field": "[Goal]", "column": 1}));
    assert_eq!(
        g["callout"],
        json!({"display_units": 1000, "precision": 0, "color": "#333333"})
    );
    assert_eq!(g["target_color"], json!("#00FF00"));
}

#[test]
fn hostile_values_do_not_crash() {
    let v = visual(
        "gauge",
        json!({
            "axis": [{"properties": {"min": lit("'abc'"), "max": {"expr": 5}, "target": lit("1e999D")}}],
            "labels": "nope",
            "lineStyles": [1, null, {"selector": 7}],
            "slices": [{"properties": null}]
        }),
    );
    let f = visual_format(&v, None);
    assert!(f.get("gauge").is_none(), "{f}");
    assert!(series_format(&v, &[]).is_empty());
}

#[test]
fn ribbon_bands_spacing_and_totals() {
    let v = visual(
        "ribbonChart",
        json!({
            "ribbonBands": [{"properties": {
                "fillMatchColor": lit("false"), "fillColor": color("#808080"),
                "fillTransparency": lit("50D"), "borderShow": lit("true"), "borderSize": lit("2D")
            }}],
            "layout": [{"properties": {"ribbonGapSize": lit("5L")}}],
            "totals": [{"properties": {"show": lit("true"), "labelDisplayUnits": lit("1000D"), "labelPrecision": lit("1L")}}]
        }),
    );
    let f = fmt(&v, None);
    assert_eq!(
        f["ribbon"],
        json!({"match_color": false, "color": "#808080", "transparency": 50, "border": true, "border_width": 2, "gap": 5})
    );
    assert_eq!(
        f["totals"],
        json!({"show": true, "display_units": 1000, "precision": 1})
    );
    // other types do not get them
    let c = visual(
        "columnChart",
        json!({"totals": [{"properties": {"show": lit("true")}}]}),
    );
    assert!(visual_format(&c, None).get("totals").is_none());
}
