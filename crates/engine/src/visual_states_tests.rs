//! Tests of formatting states (`visual_states.rs`): the writers put stateful objects' properties
//! in the `default` state entry (or the state named), `extract_props` round-trips them,
//! `report.validate` warns about state-less ones; plus the gauge / KPI auto titles and the
//! `goal` UI string.

use bicraft_report::{FieldBinding, FieldRef, Visual};
use serde_json::{Value, json};

use crate::Session;
use crate::ui_lang::{UiLanguage, visual_auto_title};
use crate::visual_props::{apply_props, extract_props};

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

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
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
    ] {
        run(&mut s, id, p);
    }
    s
}

/// Adds a visual of `ty` binding `fields`; returns (page, visual).
fn add(s: &mut Session, ty: &str, fields: Value) -> (String, String) {
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let v = run(
        s,
        "visual.add",
        json!({"page": page, "visual_type": ty, "x": 10, "y": 20, "width": 300, "height": 200,
               "fields": fields}),
    );
    (page, v["visual"].as_str().unwrap().to_string())
}

fn visual<'a>(s: &'a Session, page: &str, name: &str) -> &'a Visual {
    s.report
        .as_ref()
        .unwrap()
        .page(page)
        .unwrap()
        .visual(name)
        .unwrap()
}

fn lit(v: &str) -> Value {
    json!({"expr": {"Literal": {"Value": v}}})
}

fn color(c: &str) -> Value {
    json!({"solid": {"color": lit(&format!("'{c}'"))}})
}

fn assert_extract_noop(v: &Visual) {
    let props = extract_props(v);
    let mut copy = v.clone();
    let changes = apply_props(&mut copy, &props, None).unwrap_or_else(|e| panic!("{props}: {e}"));
    assert!(changes.is_empty(), "{props}: {changes:?}");
    assert_eq!(&copy, v);
}

#[test]
fn card_objects_go_to_the_default_state() {
    let mut s = session();
    let (page, v) = add(
        &mut s,
        "cardVisual",
        json!([{"role": "Data", "field": "[Total]"}]),
    );
    let out = run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": v, "objects": [
            {"object": "accentBar", "property": "show", "value": true},
            {"object": "accentBar", "property": "color", "value": {"color": "#0000FF"}},
            {"object": "value", "property": "fontColor", "value": {"color": "#FF0000"}},
            // not stateful: stays without selector
            {"object": "overFlow", "property": "overFlowStyle", "value": {"literal": "0D"}},
        ]}),
    );
    assert!(
        out["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "objects.accentBar.color (state default) = #0000FF"),
        "{out}"
    );
    let o = &visual(&s, &page, &v).visual_extra["objects"];
    // `show` in the default state and, as Desktop writes it, without selector
    assert_eq!(
        o["accentBar"],
        json!([
            {"properties": {"show": lit("true"), "color": color("#0000FF")}, "selector": {"id": "default"}},
            {"properties": {"show": lit("true")}},
        ])
    );
    assert_eq!(
        o["value"],
        json!([{"properties": {"fontColor": color("#FF0000")}, "selector": {"id": "default"}}])
    );
    assert_eq!(
        o["overFlow"],
        json!([{"properties": {"overFlowStyle": lit("0D")}}])
    );
    // read back as state items (the mirrored `show` is not listed twice), and idempotent
    let vis = visual(&s, &page, &v).clone();
    let props = extract_props(&vis);
    assert_eq!(
        props["format"]["objects"],
        json!([
            {"object": "overFlow", "property": "overFlowStyle", "value": {"literal": "0D"}},
            {"object": "accentBar", "property": "show", "value": true},
            {"object": "accentBar", "property": "color", "value": {"color": "#0000FF"}},
            {"object": "value", "property": "fontColor", "value": {"color": "#FF0000"}},
        ])
    );
    assert_extract_noop(&vis);
    // the same edit again changes nothing
    let again = run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": v, "objects": [
            {"object": "accentBar", "property": "show", "value": true},
            {"object": "value", "property": "fontColor", "value": {"color": "#FF0000"}},
        ]}),
    );
    assert_eq!(again["changed"], false, "{again}");
    // removing `show` removes both copies
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": v, "objects": [
            {"object": "accentBar", "property": "show", "value": null},
        ]}),
    );
    assert_eq!(
        visual(&s, &page, &v).visual_extra["objects"]["accentBar"],
        json!([{"properties": {"color": color("#0000FF")}, "selector": {"id": "default"}}])
    );
}

#[test]
fn other_states_and_the_stateless_entry() {
    let mut s = session();
    let (page, v) = add(&mut s, "actionButton", json!([]));
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": v, "objects": [
            {"object": "fill", "property": "fillColor", "value": {"color": "#00FF00"}, "state": "hover"},
            {"object": "fill", "property": "show", "value": true, "state": "press"},
            {"object": "text", "property": "fontSize", "value": 14, "state": "none"},
        ]}),
    );
    let vis = visual(&s, &page, &v).clone();
    let o = &vis.visual_extra["objects"];
    assert_eq!(
        o["fill"],
        json!([
            {"properties": {"fillColor": color("#00FF00")}, "selector": {"id": "hover"}},
            {"properties": {"show": lit("true")}, "selector": {"id": "press"}},
        ])
    );
    assert_eq!(o["text"], json!([{"properties": {"fontSize": lit("14D")}}]));
    let props = extract_props(&vis);
    assert_eq!(
        props["format"]["objects"],
        json!([
            {"object": "fill", "property": "fillColor", "value": {"color": "#00FF00"}, "state": "hover"},
            {"object": "fill", "property": "show", "value": true, "state": "press"},
            {"object": "text", "property": "fontSize", "value": {"literal": "14D"}, "state": "none"},
        ])
    );
    assert_extract_noop(&vis);
    // report.apply round trip: extract, then apply to a fresh report
    let spec = run(&mut s, "report.extract", json!({}));
    let mut t = session();
    run(&mut t, "report.apply", json!({"spec": spec, "prune": "all"}));
    let page2 = t.report.as_ref().unwrap().pages[0].name.clone();
    assert_eq!(visual(&t, &page2, &v).visual_extra["objects"], *o);
}

#[test]
fn non_stateful_objects_and_types_keep_no_selector() {
    let mut s = session();
    let (page, v) = add(
        &mut s,
        "textSlicer",
        json!([{"role": "Values", "field": "Sales[Region]"}]),
    );
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": v, "objects": [
            {"object": "inputTextBox", "property": "backShow", "value": true},
        ]}),
    );
    assert_eq!(
        visual(&s, &page, &v).visual_extra["objects"]["inputTextBox"],
        json!([{"properties": {"backShow": lit("true")}}])
    );
    assert_eq!(
        extract_props(visual(&s, &page, &v))["format"]["objects"],
        json!([{"object": "inputTextBox", "property": "backShow", "value": true}]),
        "a non-stateful object is read back plain, without a selector or a state"
    );
    // advancedSlicerVisual layout is read without selector
    let (page, b) = add(
        &mut s,
        "advancedSlicerVisual",
        json!([{"role": "Values", "field": "Sales[Region]"}]),
    );
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": b, "objects": [
            {"object": "layout", "property": "columnCount", "value": {"literal": "1L"}},
            {"object": "value", "property": "fontColor", "value": {"color": "#FF0000"}},
        ]}),
    );
    let o = &visual(&s, &page, &b).visual_extra["objects"];
    assert_eq!(o["layout"], json!([{"properties": {"columnCount": lit("1L")}}]));
    assert_eq!(o["value"][0]["selector"], json!({"id": "default"}));
    // an explicit selector is kept as given
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": b, "objects": [
            {"object": "value", "property": "bold", "value": true, "selector": {"metadata": "Sales.Region"}},
        ]}),
    );
    assert_eq!(
        visual(&s, &page, &b).visual_extra["objects"]["value"][1]["selector"],
        json!({"metadata": "Sales.Region"})
    );
}

#[test]
fn bad_states_are_refused() {
    let mut s = session();
    let (page, v) = add(
        &mut s,
        "cardVisual",
        json!([{"role": "Data", "field": "[Total]"}]),
    );
    let (_, t) = add(
        &mut s,
        "textSlicer",
        json!([{"role": "Values", "field": "Sales[Region]"}]),
    );
    let before = s.report.clone();
    for (visual, item, needle) in [
        (
            &v,
            json!({"object": "value", "property": "bold", "value": true, "state": "hover"}),
            "no state `hover`",
        ),
        (
            &v,
            json!({"object": "overFlow", "property": "overFlowStyle", "value": 1, "state": "default"}),
            "no formatting states",
        ),
        (
            &t,
            json!({"object": "inputTextBox", "property": "backShow", "value": true, "state": "default"}),
            "no formatting states",
        ),
        (
            &v,
            json!({"object": "title", "property": "show", "value": true, "state": "default"}),
            "container object",
        ),
        (
            &v,
            json!({"object": "value", "property": "bold", "value": true, "state": "default",
                    "selector": {"id": "default"}}),
            "not both",
        ),
        (
            &v,
            json!({"object": "value", "property": "bold", "value": true, "state": "de fault"}),
            "state must be",
        ),
        (
            &v,
            json!({"object": "value", "property": "bold", "value": true, "state": 3}),
            "state must be",
        ),
    ] {
        let e = s
            .execute(
                "visual.format",
                &json!({"page": page, "visual": visual, "objects": [item]}),
            )
            .expect_err("should fail");
        assert!(e.contains(needle), "{item}: {e}");
    }
    assert_eq!(s.report, before);
}

/// The state-selector warnings `report.validate` gives a `ty` visual holding `objects`:
/// (object, message) pairs.
fn state_warnings(ty: &str, fields: Value, objects: Value) -> Vec<(String, String)> {
    let mut s = session();
    let (page, v) = add(&mut s, ty, fields);
    {
        let r = s.report.as_mut().unwrap();
        let p = r.pages.iter_mut().find(|p| p.name == page).unwrap();
        let vis = p.visuals.iter_mut().find(|x| x.name == v).unwrap();
        vis.visual_extra.insert("objects".into(), objects);
    }
    let out = run(&mut s, "report.validate", json!({}));
    assert_eq!(out["valid"], true, "{out}");
    out["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["part"] == "objects" && i["visual"] == json!(v))
        .map(|i| {
            (
                i["object"].as_str().unwrap().to_string(),
                i["message"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn validate_warns_about_ignored_state_entries() {
    let card = || json!([{"role": "Data", "field": "[Total]"}]);
    let region = || json!([{"role": "Values", "field": "Sales[Region]"}]);
    let t = lit("true");
    let dflt = json!({"id": "default"});
    // A file written before state selectors: the color is only without selector.
    let w = state_warnings(
        "cardVisual",
        card(),
        json!({"value": [{"properties": {"fontColor": color("#FF0000"), "show": t}},
                         {"properties": {"bold": t}, "selector": dflt}],
               "overFlow": [{"properties": {"overFlowStyle": lit("0D")}}]}),
    );
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(w[0].0, "value");
    assert!(
        w[0].1.contains("`fontColor`") && !w[0].1.contains("`show`"),
        "{w:?}"
    );
    // 1. A lone `show` without selector draws the accent bar (Desktop 2.158): no warning; with
    // `width` beside it, only `width` is ignored.
    let w = state_warnings(
        "cardVisual",
        card(),
        json!({"accentBar": [{"properties": {"show": t}}]}),
    );
    assert!(w.is_empty(), "{w:?}");
    let w = state_warnings(
        "cardVisual",
        card(),
        json!({"accentBar": [{"properties": {"show": t, "width": lit("4D")}}]}),
    );
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].1.contains("`width`") && !w[0].1.contains("`show`"), "{w:?}");
    // 2. Per-callout formatting with `id` added to `metadata`.
    let w = state_warnings(
        "cardVisual",
        card(),
        json!({"value": [{"properties": {"fontColor": color("#FF0000")},
                          "selector": {"metadata": "Sales.Total", "id": "default"}}]}),
    );
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].1.contains("`metadata`"), "{w:?}");
    // the plain per-callout selector is fine
    let w = state_warnings(
        "cardVisual",
        card(),
        json!({"value": [{"properties": {"fontColor": color("#FF0000")},
                          "selector": {"metadata": "Sales.Total"}}]}),
    );
    assert!(w.is_empty(), "{w:?}");
    // 3. A button fill whose `show` is only in the default state.
    let fill = json!({"show": t, "fillColor": color("#FF0000")});
    let w = state_warnings(
        "actionButton",
        json!([]),
        json!({"fill": [{"properties": fill, "selector": dflt}]}),
    );
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].1.contains("toggle"), "{w:?}");
    // as bicraft and Desktop write it: no warning
    let w = state_warnings(
        "actionButton",
        json!([]),
        json!({"fill": [{"properties": fill, "selector": dflt}, {"properties": {"show": t}}]}),
    );
    assert!(w.is_empty(), "{w:?}");
    // a shape's fill needs no toggle without selector
    let w = state_warnings(
        "shape",
        json!([]),
        json!({"fill": [{"properties": fill, "selector": dflt}]}),
    );
    assert!(w.is_empty(), "{w:?}");
    // A button's tile shape is stateful, a shape visual's is read without selector.
    let oval = json!({"tileShape": lit("'oval'")});
    let w = state_warnings(
        "actionButton",
        json!([]),
        json!({"shape": [{"properties": oval}]}),
    );
    assert_eq!(w.len(), 1, "{w:?}");
    let w = state_warnings(
        "shape",
        json!([]),
        json!({"shape": [{"properties": oval, "selector": dflt}]}),
    );
    assert_eq!(w.len(), 1, "{w:?}");
    let w = state_warnings("shape", json!([]), json!({"shape": [{"properties": oval}]}));
    assert!(w.is_empty(), "{w:?}");
    // 4. / 5. Objects Desktop reads only without selector, written only in the default state.
    let w = state_warnings(
        "advancedSlicerVisual",
        region(),
        json!({"layout": [{"properties": {"columnCount": lit("1L")}, "selector": dflt}]}),
    );
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].1.contains("reads this object without selector"), "{w:?}");
    let w = state_warnings(
        "textSlicer",
        region(),
        json!({"inputTextBox": [{"properties": {"backColor": color("#FF0000")}, "selector": dflt}]}),
    );
    assert_eq!(w.len(), 1, "{w:?}");
    // Desktop's own form (both entries) and plain state-less entries: no warning
    for (ty, fields, objects) in [
        (
            "cardVisual",
            card(),
            json!({"value": [{"properties": {"fontColor": color("#FF0000")}},
                             {"properties": {"fontColor": color("#FF0000")}, "selector": dflt}]}),
        ),
        (
            "advancedSlicerVisual",
            region(),
            json!({"layout": [{"properties": {"columnCount": lit("1L")}},
                              {"properties": {"columnCount": lit("1L")}, "selector": dflt}]}),
        ),
        (
            "textSlicer",
            region(),
            json!({"inputTextBox": [{"properties": {"backColor": color("#FF0000")}}]}),
        ),
    ] {
        let w = state_warnings(ty, fields, objects);
        assert!(w.is_empty(), "{ty}: {w:?}");
    }
}

fn binding(role: &str, measure: &str) -> FieldBinding {
    FieldBinding {
        display_name: None,
        show_as: None,
        role: role.into(),
        field: FieldRef::Measure {
            table: "Sales".into(),
            measure: measure.into(),
        },
    }
}

#[test]
fn gauge_and_kpi_auto_titles() {
    let title = |ty: &str, fields: Vec<FieldBinding>, lang: &str| {
        let v = Visual {
            visual_type: Some(ty.into()),
            fields,
            ..Default::default()
        };
        let labels: Vec<Option<String>> = v
            .fields
            .iter()
            .map(|b| match &b.field {
                FieldRef::Measure { measure, .. } => Some(measure.clone()),
                FieldRef::Column { column, .. } => Some(column.clone()),
                _ => None,
            })
            .collect();
        visual_auto_title(&UiLanguage::parse(lang).unwrap(), &v, &labels).unwrap_or_default()
    };
    // Desktop 2.158 (French UI): "Total, Low, High et Goal", whatever the binding order
    let gauge = vec![
        binding("TargetValue", "Goal"),
        binding("Y", "Total"),
        binding("MaxValue", "High"),
        binding("MinValue", "Low"),
    ];
    assert_eq!(title("gauge", gauge.clone(), "fr-FR"), "Total, Low, High et Goal");
    assert_eq!(title("gauge", gauge, "en-US"), "Total, Low, High and Goal");
    // "Total et Goal par Month"
    let kpi = vec![
        binding("Indicator", "Total"),
        FieldBinding {
            display_name: None,
            show_as: None,
            role: "TrendLine".into(),
            field: FieldRef::Column {
                table: "Sales".into(),
                column: "Month".into(),
            },
        },
        binding("Goal", "Goal"),
    ];
    assert_eq!(title("kpi", kpi, "fr-FR"), "Total et Goal par Month");
    // other visuals still name at most three values
    let many: Vec<FieldBinding> = (0..5).map(|i| binding("Y", &format!("V{i}"))).collect();
    assert_eq!(title("clusteredColumnChart", many, "en-US"), "V0, V1 and V2");
}

#[test]
fn goal_ui_string_per_language() {
    // The translations are BIcraft's own choices: each language has its own word for "Goal".
    let goal = |tag: &str| UiLanguage::parse(tag).unwrap().snapshot_json()["strings"]["goal"].clone();
    assert_eq!(goal("en-US"), "Goal");
    for tag in ["fr-FR", "de-DE", "es-ES", "it-IT", "nl-NL", "pt-BR", "ja-JP"] {
        let word = goal(tag);
        let word = word.as_str().unwrap_or_default();
        assert!(!word.is_empty() && word != "Goal", "{tag}: {word:?}");
    }
}
