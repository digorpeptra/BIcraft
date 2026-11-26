//! Format painter (`visual.copy_format`, docs/formats/format-painter.md): what is copied and what
//! stays the target's, one undo step, the PBIP and PBIX round trips. Synthetic model only.

use bicraft_engine::Session;
use serde_json::{Value, json};

use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Paint"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "m": "let Source = #table(type table [Region = text, Product = text, Amount = Int64.Type], {{\"West\", \"A\", 5}, {\"East\", \"B\", 7}}) in Source"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
    );
    run(&mut s, "report.new", json!({}));
    s
}

fn page(s: &Session) -> String {
    s.report.as_ref().unwrap().pages[0].name.clone()
}

fn add(s: &mut Session, ty: &str, fields: Value) -> String {
    let p = page(s);
    let out = run(
        s,
        "visual.add",
        json!({"page": p, "visual_type": ty, "x": 0, "y": 0, "width": 300, "height": 200, "fields": fields}),
    );
    out["visual"].as_str().unwrap().to_string()
}

fn extra(s: &Session, name: &str) -> Value {
    let r = s.report.as_ref().unwrap();
    let v = r.pages[0].visuals.iter().find(|v| v.name == name).unwrap();
    json!({
        "objects": v.visual_extra.get("objects").cloned().unwrap_or(Value::Null),
        "container": v.visual_extra.get("visualContainerObjects").cloned().unwrap_or(Value::Null),
        "position": [v.position.x, v.position.y, v.position.width, v.position.height],
        "fields": v.fields.len(),
    })
}

fn prop<'a>(x: &'a Value, group: &str, object: &str, property: &str) -> Option<&'a Value> {
    x[group][object]
        .as_array()?
        .iter()
        .find(|e| e.get("selector").is_none())?
        .get("properties")?
        .get(property)
}

fn west_selector() -> Value {
    json!({"data": [{"scopeId": {"Comparison": {"ComparisonKind": 0,
        "Left": {"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Region"}},
        "Right": {"Literal": {"Value": "'West'"}}}}}]})
}

/// A source column chart formatted everywhere, a column chart on the same field, a card.
fn painted() -> (Session, String, String, String) {
    let mut s = session();
    let src = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let col = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let card = add(&mut s, "card", json!([{"role": "Values", "field": "[Total]"}]));
    let p = page(&s);
    run(
        &mut s,
        "visual.set_title",
        json!({"page": p, "visual": src, "title": "Source", "font_size": 18, "color": "#AA0000"}),
    );
    run(
        &mut s,
        "visual.set_title",
        json!({"page": p, "visual": col, "title": "Target"}),
    );
    run(
        &mut s,
        "visual.format",
        json!({"page": p, "visual": src, "background": {"show": true, "color": "#112233"},
        "border": {"show": true, "width": 3}, "alt_text": "the source",
        "objects": [
            {"object": "labels", "property": "show", "value": true},
            {"object": "labels", "property": "fontSize", "value": 14},
            {"object": "labels", "property": "labelDisplayUnits", "value": 1000},
            {"object": "valueAxis", "property": "start", "value": 50},
            {"object": "valueAxis", "property": "fontSize", "value": 11},
            {"object": "dataPoint", "property": "fill", "value": {"color": "#00AA00"}, "selector": west_selector()},
        ]}),
    );
    run(
        &mut s,
        "visual.format",
        json!({"page": p, "visual": col, "objects": [
            {"object": "valueAxis", "property": "start", "value": 10},
            {"object": "labels", "property": "labelDisplayUnits", "value": 1000000},
        ]}),
    );
    (s, src, col, card)
}

#[test]
fn copies_formatting_and_keeps_text_ranges_and_units() {
    let (mut s, src, col, card) = painted();
    let p = page(&s);
    let before_col = extra(&s, &col);
    let out = run(
        &mut s,
        "visual.copy_format",
        json!({"page": p, "from": src, "to": [col, card]}),
    );
    assert_eq!(out["changed"], true);
    let x = extra(&s, &col);
    // container objects: title options but its text, background, border; alt text stays
    assert_eq!(
        prop(&x, "container", "title", "text"),
        prop(&before_col, "container", "title", "text")
    );
    assert!(prop(&x, "container", "title", "fontSize").is_some());
    assert!(prop(&x, "container", "background", "color").is_some());
    assert!(prop(&x, "container", "border", "width").is_some());
    assert!(prop(&x, "container", "general", "altText").is_none());
    // visual objects: labels without display units, axes without start
    assert!(prop(&x, "objects", "labels", "fontSize").is_some());
    assert_eq!(
        prop(&x, "objects", "labels", "labelDisplayUnits"),
        prop(&before_col, "objects", "labels", "labelDisplayUnits")
    );
    assert_eq!(
        prop(&x, "objects", "valueAxis", "start"),
        prop(&before_col, "objects", "valueAxis", "start")
    );
    assert!(prop(&x, "objects", "valueAxis", "fontSize").is_some());
    // the colour of West: the target binds Sales[Region]
    let dp = x["objects"]["dataPoint"].as_array().unwrap();
    assert!(dp.iter().any(|e| e.get("selector") == Some(&west_selector())));
    // fields, size and position stay
    assert_eq!(x["position"], before_col["position"]);
    assert_eq!(x["fields"], before_col["fields"]);
    // the card: no axis, no West colour (it does not bind Region), but the title and background
    let c = extra(&s, &card);
    assert!(c["objects"]["valueAxis"].is_null());
    assert!(c["objects"]["dataPoint"].is_null());
    assert!(prop(&c, "container", "background", "color").is_some());
    assert!(prop(&c, "container", "title", "text").is_none());
}

#[test]
fn is_one_undo_step_and_checks_targets_first() {
    let (mut s, src, col, card) = painted();
    let p = page(&s);
    let a = (extra(&s, &col), extra(&s, &card));
    // a bad target changes nothing
    assert!(
        s.execute(
            "visual.copy_format",
            &json!({"page": p, "from": src, "to": [col, "nope"]})
        )
        .is_err()
    );
    assert_eq!((extra(&s, &col), extra(&s, &card)), a);
    run(
        &mut s,
        "visual.copy_format",
        json!({"page": p, "from": src, "to": [col, card]}),
    );
    assert_ne!((extra(&s, &col), extra(&s, &card)), a);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!((extra(&s, &col), extra(&s, &card)), a);
    // the source onto itself changes nothing
    let out = run(
        &mut s,
        "visual.copy_format",
        json!({"page": p, "from": src, "to": src}),
    );
    assert_eq!(out["changed"], false);
    // bad params are errors
    for bad in [
        json!({"page": p, "from": src}),
        json!({"page": p, "from": src, "to": []}),
        json!({"page": p, "from": src, "to": [1]}),
        json!({"page": p, "from": src, "to": col, "to_page": 3}),
        json!({"page": p, "from": src, "to": col, "x": 1}),
    ] {
        assert!(s.execute("visual.copy_format", &bad).is_err(), "{bad}");
    }
}

#[test]
fn painted_formatting_round_trips_through_pbip_and_pbix() {
    for ext in ["pbip", "pbix"] {
        let (mut s, src, col, card) = painted();
        let p = page(&s);
        run(
            &mut s,
            "visual.copy_format",
            json!({"page": p, "from": src, "to": [col, card]}),
        );
        let want = (extra(&s, &col), extra(&s, &card));
        let dir = TempDir::new(&format!("engine-format-painter-{ext}"));
        let path = dir.join(format!("Paint.{ext}"));
        run(&mut s, "file.save_as", json!({"path": path}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        assert_eq!((extra(&back, &col), extra(&back, &card)), want, "{ext}");
    }
}
