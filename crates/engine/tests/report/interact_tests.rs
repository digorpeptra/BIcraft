//! The viewer's interactions (`view.interact`) on a synthetic model with literal tables: slicer
//! clicks, cross-filtering with `visualInteractions`, Ctrl+click, "Apply all slicers", "Clear
//! all slicers", page navigation, and the rule that nothing reaches the report.

use bicraft_report::Report;
use serde_json::{Value, json};

use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Sales(Product, Amount, Qty) many-to-one Product(Product, Category). Total = 1162.5;
/// Wheels = 1150.5 (Bike 150.5, Car 1000), Toys = 5, Kite (no product row) = 7.
fn shop() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Product", "Amount", "Qty"}, {{"Bike", 100, 1}, {"Bike", 50.5, 2}, {"Car", 1000, 1}, {"Ball", 5, 10}, {"Kite", 7, 1}})"#;
    let product =
        r#"#table({"Product", "Category"}, {{"Bike", "Wheels"}, {"Car", "Wheels"}, {"Ball", "Toys"}})"#;
    let steps = [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "table.add",
            json!({"name": "Product", "partition": {"source_type": "m", "expression": product}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Product", "data_type": "string"}),
        ),
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
            json!({"table": "Product", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Category", "data_type": "string"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Lines", "expression": "COUNTROWS(Sales)"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[Product]", "to": "Product[Product]"}),
        ),
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "Main"})),
        ("page.add", json!({"display_name": "Second"})),
    ];
    for (id, p) in steps {
        run(&mut s, id, p);
    }
    s
}

fn add_visual(s: &mut Session, page: &str, visual_type: &str, fields: Value) -> String {
    let r = run(
        s,
        "visual.add",
        json!({"page": page, "visual_type": visual_type, "x": 0, "y": 0, "width": 100, "height": 100,
               "fields": fields}),
    );
    r.get("visual")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| panic!("no visual name in {r}"))
}

fn page_name(s: &Session, display: &str) -> String {
    s.report
        .as_ref()
        .and_then(|r| r.pages.iter().find(|p| p.display_name == display))
        .map(|p| p.name.clone())
        .unwrap_or_else(|| panic!("no page {display}"))
}

/// Edits the report directly (what Desktop would have written).
fn edit(s: &mut Session, f: impl FnOnce(&mut Report)) {
    if let Some(r) = s.report.as_mut() {
        f(r);
    }
    s.revision += 1;
}

fn visual_json(snap: &Value, visual: &str) -> Value {
    for p in snap["report"]["pages"].as_array().cloned().unwrap_or_default() {
        for v in p["visuals"].as_array().cloned().unwrap_or_default() {
            if v["name"] == visual {
                return v;
            }
        }
    }
    panic!("visual {visual} not in the snapshot")
}

fn rows_of(snap: &Value, visual: &str) -> Vec<Value> {
    let d = visual_json(snap, visual)["data"].clone();
    assert_eq!(d["status"], "ok", "{d}");
    d["rows"].as_array().cloned().unwrap_or_default()
}

fn rows(s: &Session, visual: &str) -> Vec<Value> {
    rows_of(&s.snapshot(), visual)
}

fn act(s: &mut Session, p: Value) -> Value {
    run(s, "view.interact", p)
}

#[test]
fn slicer_clicks_follow_desktop_and_never_touch_the_report() {
    let mut s = shop();
    let main = page_name(&s, "Main");
    let slicer = add_visual(
        &mut s,
        "Main",
        "slicer",
        json!([{"role": "Values", "field": "Product[Category]"}]),
    );
    let card = add_visual(
        &mut s,
        "Main",
        "card",
        json!([{"role": "Values", "field": "[Total]"}]),
    );
    let stored = s.report.clone();
    let (revision, dirty) = (s.revision, (s.dirty, s.report_dirty));
    assert_eq!(rows(&s, &card), vec![json!([1162.5])]);

    // a click selects that item alone
    let r = act(
        &mut s,
        json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": "Wheels"}),
    );
    assert_eq!(r["changed"], true);
    let snap = s.snapshot();
    assert_eq!(rows_of(&snap, &card), vec![json!([1150.5])]);
    assert_eq!(
        visual_json(&snap, &slicer)["slicer"]["selected"],
        json!(["Wheels"])
    );
    // the slicer is not filtered by its own selection (the all-blank row is dropped)
    assert_eq!(rows_of(&snap, &slicer).len(), 2);
    // Ctrl+click adds an item; a plain click on another one replaces the selection
    act(
        &mut s,
        json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": "Toys", "ctrl": true}),
    );
    assert_eq!(rows(&s, &card), vec![json!([1155.5])]);
    act(
        &mut s,
        json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": "toys"}),
    );
    assert_eq!(rows(&s, &card), vec![json!([5])]);
    // a click on the only selected item clears the selection
    act(
        &mut s,
        json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": "Toys"}),
    );
    assert_eq!(rows(&s, &card), vec![json!([1162.5])]);
    // a blank item is selectable
    act(
        &mut s,
        json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": null}),
    );
    assert_eq!(rows(&s, &card), vec![json!([7])]);
    // "Select all" clears it
    act(
        &mut s,
        json!({"action": "slicer_select_all", "page": main, "visual": slicer}),
    );
    assert_eq!(rows(&s, &card), vec![json!([1162.5])]);

    // the report, the dirty flags and the revision never move
    assert_eq!(s.report, stored);
    assert_eq!(s.revision, revision);
    assert_eq!((s.dirty, s.report_dirty), dirty);
    assert!(s.execute("view.interact", &json!({"action": "nope"})).is_err());
    assert!(
        s.execute(
            "view.interact",
            &json!({"action": "slicer_toggle", "page": main, "visual": card, "value": 1})
        )
        .is_err()
    );
}

#[test]
fn single_select_and_ctrl_free_multi_select() {
    let mut s = shop();
    let main = page_name(&s, "Main");
    let slicer = add_visual(
        &mut s,
        "Main",
        "slicer",
        json!([{"role": "Values", "field": "Product[Category]"}]),
    );
    let card = add_visual(
        &mut s,
        "Main",
        "card",
        json!([{"role": "Values", "field": "[Total]"}]),
    );
    let set = |s: &mut Session, prop: &str, value: &str| {
        let (sl, prop, value) = (slicer.clone(), prop.to_string(), value.to_string());
        edit(s, |r| {
            for p in &mut r.pages {
                for v in &mut p.visuals {
                    if v.name == sl {
                        let o = v.visual_extra.entry("objects".to_string()).or_insert(json!({}));
                        o["selection"] = json!([{"properties": {prop.as_str(): {"expr": {"Literal": {"Value": value.as_str()}}}}}]);
                    }
                }
            }
        });
    };
    // single select: Ctrl+click still selects one item, a re-click keeps it
    set(&mut s, "strictSingleSelect", "true");
    act(
        &mut s,
        json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": "Wheels"}),
    );
    act(
        &mut s,
        json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": "Toys", "ctrl": true}),
    );
    assert_eq!(rows(&s, &card), vec![json!([5])]);
    act(
        &mut s,
        json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": "Toys"}),
    );
    assert_eq!(rows(&s, &card), vec![json!([5])]);
    // "Multi-select with Ctrl" off: a plain click toggles
    act(&mut s, json!({"action": "reset"}));
    set(&mut s, "singleSelect", "false");
    act(
        &mut s,
        json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": "Wheels"}),
    );
    act(
        &mut s,
        json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": "Toys"}),
    );
    assert_eq!(rows(&s, &card), vec![json!([1155.5])]);
    act(
        &mut s,
        json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": "Wheels"}),
    );
    assert_eq!(rows(&s, &card), vec![json!([5])]);
}

#[test]
fn stored_selections_are_the_starting_point() {
    let mut s = shop();
    let main = page_name(&s, "Main");
    let slicer = add_visual(
        &mut s,
        "Main",
        "slicer",
        json!([{"role": "Values", "field": "Product[Category]"}]),
    );
    let card = add_visual(
        &mut s,
        "Main",
        "card",
        json!([{"role": "Values", "field": "[Total]"}]),
    );
    let sl = slicer.clone();
    edit(&mut s, |r| {
        for p in &mut r.pages {
            for v in &mut p.visuals {
                if v.name == sl {
                    let f = json!({"Version": 2, "From": [{"Name": "p", "Entity": "Product", "Type": 0}],
                        "Where": [{"Condition": {"In": {"Expressions": [{"Column": {"Expression": {"SourceRef": {"Source": "p"}}, "Property": "Category"}}],
                                                         "Values": [[{"Literal": {"Value": "'Wheels'"}}]]}}}]});
                    let o = v.visual_extra.entry("objects".to_string()).or_insert(json!({}));
                    o["general"] = json!([{"properties": {"filter": {"filter": f}}}]);
                }
            }
        }
    });
    assert_eq!(rows(&s, &card), vec![json!([1150.5])]);
    // Ctrl+click adds to the stored selection; clicking the stored item alone clears it
    act(
        &mut s,
        json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": "Toys", "ctrl": true}),
    );
    assert_eq!(rows(&s, &card), vec![json!([1155.5])]);
    act(&mut s, json!({"action": "reset"}));
    act(
        &mut s,
        json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": "Wheels"}),
    );
    assert_eq!(rows(&s, &card), vec![json!([1162.5])]);
}

#[test]
fn range_text_and_relative_slicers() {
    let mut s = shop();
    let main = page_name(&s, "Main");
    let range = add_visual(
        &mut s,
        "Main",
        "slicer",
        json!([{"role": "Values", "field": "Sales[Qty]"}]),
    );
    let text = add_visual(
        &mut s,
        "Main",
        "slicer",
        json!([{"role": "Values", "field": "Sales[Product]"}]),
    );
    let lines = add_visual(
        &mut s,
        "Main",
        "card",
        json!([{"role": "Values", "field": "[Lines]"}]),
    );
    act(
        &mut s,
        json!({"action": "slicer_range", "page": main, "visual": range, "from": 2, "to": 10}),
    );
    let snap = s.snapshot();
    assert_eq!(rows_of(&snap, &lines), vec![json!([2])]);
    let shown = &visual_json(&snap, &range)["slicer"]["range"];
    assert_eq!(
        (shown["from"].as_f64(), shown["to"].as_f64()),
        (Some(2.0), Some(10.0)),
        "{shown}"
    );
    // an open bound
    act(
        &mut s,
        json!({"action": "slicer_range", "page": main, "visual": range, "from": null, "to": 1}),
    );
    assert_eq!(rows(&s, &lines), vec![json!([3])]);
    assert!(
        s.execute(
            "view.interact",
            &json!({"action": "slicer_range", "page": main, "visual": range, "from": "x"})
        )
        .is_err()
    );
    act(
        &mut s,
        json!({"action": "slicer_clear", "page": main, "visual": range}),
    );
    assert_eq!(rows(&s, &lines), vec![json!([5])]);
    // a text slicer: contains, without case
    act(
        &mut s,
        json!({"action": "slicer_text", "page": main, "visual": text, "text": "i"}),
    );
    assert_eq!(rows(&s, &lines), vec![json!([3])]);
    act(
        &mut s,
        json!({"action": "slicer_text", "page": main, "visual": text, "text": ""}),
    );
    assert_eq!(rows(&s, &lines), vec![json!([5])]);
    // relative dates need a date column
    assert!(
        s.execute(
            "view.interact",
            &json!({"action": "slicer_relative", "page": main, "visual": range,
                                           "range": "last", "duration": 0, "period": "days"})
        )
        .is_err()
    );
}

#[test]
fn data_points_cross_filter_the_page_by_visual_interactions() {
    let mut s = shop();
    let main = page_name(&s, "Main");
    let chart = add_visual(
        &mut s,
        "Main",
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Product[Category]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let card = add_visual(
        &mut s,
        "Main",
        "card",
        json!([{"role": "Values", "field": "[Total]"}]),
    );
    let other = add_visual(
        &mut s,
        "Main",
        "card",
        json!([{"role": "Values", "field": "[Lines]"}]),
    );
    let products = add_visual(
        &mut s,
        "Main",
        "slicer",
        json!([{"role": "Values", "field": "Product[Product]"}]),
    );
    let all = rows(&s, &chart);

    act(
        &mut s,
        json!({"action": "select", "page": main, "visual": chart, "point": [{"c": 0, "v": "Wheels"}]}),
    );
    let snap = s.snapshot();
    // the other visuals are filtered, slicers too; the source keeps every point
    assert_eq!(rows_of(&snap, &card), vec![json!([1150.5])]);
    assert_eq!(rows_of(&snap, &other), vec![json!([3])]);
    assert_eq!(rows_of(&snap, &products), vec![json!(["Bike"]), json!(["Car"])]);
    assert_eq!(rows_of(&snap, &chart), all);
    let selected = &visual_json(&snap, &chart)["interaction"]["selected"];
    let at = all.iter().position(|r| r[0] == "Wheels").unwrap();
    assert_eq!(selected["rows"], json!([at]));
    assert_eq!(
        visual_json(&snap, &card)["interaction"]["filtered_by"],
        json!(chart)
    );
    assert_eq!(snap["interaction"]["seq"].as_u64(), Some(1));

    // Ctrl+click adds a point, Ctrl+click on it again removes it
    act(
        &mut s,
        json!({"action": "select", "page": main, "visual": chart, "point": [{"c": 0, "v": "Toys"}], "ctrl": true}),
    );
    assert_eq!(rows(&s, &card), vec![json!([1155.5])]);
    act(
        &mut s,
        json!({"action": "select", "page": main, "visual": chart, "point": [{"c": 0, "v": "Toys"}], "ctrl": true}),
    );
    assert_eq!(rows(&s, &card), vec![json!([1150.5])]);

    // NoFilter turns one target off; HighlightFilter on a card (which cannot highlight) filters
    let (src, a, b) = (chart.clone(), card.clone(), other.clone());
    edit(&mut s, |r| {
        if let Some(p) = r.pages.iter_mut().find(|p| p.display_name == "Main") {
            p.extra.insert(
                "visualInteractions".into(),
                json!([{"source": src, "target": a, "type": "NoFilter"},
                       {"source": src, "target": b, "type": "HighlightFilter"}]),
            );
        }
    });
    let snap = s.snapshot();
    assert_eq!(rows_of(&snap, &card), vec![json!([1162.5])]);
    assert!(visual_json(&snap, &card)["interaction"].is_null());
    assert_eq!(rows_of(&snap, &other), vec![json!([3])]);
    assert_eq!(visual_json(&snap, &other)["interaction"]["mode"], "filter");

    // a click on the only selected point clears it; a click on the background too
    act(
        &mut s,
        json!({"action": "select", "page": main, "visual": chart, "point": [{"c": 0, "v": "Wheels"}]}),
    );
    assert_eq!(rows(&s, &other), vec![json!([5])]);
    act(
        &mut s,
        json!({"action": "select", "page": main, "visual": chart, "point": [{"c": 0, "v": "Toys"}]}),
    );
    assert_eq!(rows(&s, &other), vec![json!([1])]);
    act(
        &mut s,
        json!({"action": "select", "page": main, "visual": chart, "point": null}),
    );
    assert_eq!(rows(&s, &other), vec![json!([5])]);
    // a measure column is not a grouping value
    assert!(
        s.execute(
            "view.interact",
            &json!({"action": "select", "page": main, "visual": chart, "point": [{"c": 1, "v": 5}]})
        )
        .is_err()
    );
}

#[test]
fn points_over_several_tables_and_slicers_together() {
    let mut s = shop();
    let main = page_name(&s, "Main");
    let table = add_visual(
        &mut s,
        "Main",
        "tableEx",
        json!([{"role": "Values", "field": "Product[Category]"}, {"role": "Values", "field": "Sales[Product]"},
               {"role": "Values", "field": "[Total]"}]),
    );
    let card = add_visual(
        &mut s,
        "Main",
        "card",
        json!([{"role": "Values", "field": "[Total]"}]),
    );
    let slicer = add_visual(
        &mut s,
        "Main",
        "slicer",
        json!([{"role": "Values", "field": "Sales[Qty]"}]),
    );
    act(
        &mut s,
        json!({"action": "select", "page": main, "visual": table,
               "point": [{"c": 0, "v": "Wheels"}, {"c": 1, "v": "Bike"}]}),
    );
    assert_eq!(rows(&s, &card), vec![json!([150.5])]);
    // a slicer and the selection both filter the card
    act(
        &mut s,
        json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": 2}),
    );
    assert_eq!(rows(&s, &card), vec![json!([50.5])]);
    // and the slicer filters the table the selection came from
    assert_eq!(rows(&s, &table).len(), 1);
}

#[test]
fn apply_and_clear_all_slicers_buttons() {
    let mut s = shop();
    let main = page_name(&s, "Main");
    let slicer = add_visual(
        &mut s,
        "Main",
        "slicer",
        json!([{"role": "Values", "field": "Product[Category]"}]),
    );
    let card = add_visual(
        &mut s,
        "Main",
        "card",
        json!([{"role": "Values", "field": "[Total]"}]),
    );
    let apply = add_visual(&mut s, "Main", "actionButton", json!([]));
    let clear = add_visual(&mut s, "Main", "actionButton", json!([]));
    let (a, c) = (apply.clone(), clear.clone());
    edit(&mut s, |r| {
        for p in &mut r.pages {
            for v in &mut p.visuals {
                let kind = if v.name == a {
                    "ApplyAllSlicers"
                } else if v.name == c {
                    "ClearAllSlicers"
                } else {
                    continue;
                };
                v.visual_extra.insert(
                    "visualContainerObjects".into(),
                    json!({"visualLink": [{"properties": {
                        "show": {"expr": {"Literal": {"Value": "true"}}},
                        "type": {"expr": {"Literal": {"Value": format!("'{kind}'")}}}}}]}),
                );
            }
        }
    });
    // the snapshot says what each button does
    let snap = s.snapshot();
    assert_eq!(
        visual_json(&snap, &apply)["interaction"]["button"]["kind"],
        "ApplyAllSlicers"
    );
    assert_eq!(snap["interaction"]["apply_pages"], json!([main]));
    // with an "Apply all slicers" button the change waits
    let r = act(
        &mut s,
        json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": "Toys"}),
    );
    assert_eq!(r["pending"], true);
    let snap = s.snapshot();
    assert_eq!(rows_of(&snap, &card), vec![json!([1162.5])]);
    assert_eq!(visual_json(&snap, &slicer)["slicer"]["selected"], json!(["Toys"]));
    assert_eq!(visual_json(&snap, &slicer)["slicer"]["pending"], true);
    assert_eq!(snap["interaction"]["pending_pages"], json!([main]));
    // the button applies it
    act(&mut s, json!({"action": "button", "page": main, "visual": apply}));
    let snap = s.snapshot();
    assert_eq!(rows_of(&snap, &card), vec![json!([5])]);
    assert!(visual_json(&snap, &slicer)["slicer"]["pending"].is_null());
    // "Clear all slicers" clears applied and pending selections at once
    act(
        &mut s,
        json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": "Wheels", "ctrl": true}),
    );
    act(&mut s, json!({"action": "button", "page": main, "visual": clear}));
    let snap = s.snapshot();
    assert_eq!(rows_of(&snap, &card), vec![json!([1162.5])]);
    assert!(visual_json(&snap, &slicer)["slicer"]["selected"].is_null());
    // a button without an action is refused
    assert!(
        s.execute(
            "view.interact",
            &json!({"action": "button", "page": main, "visual": card})
        )
        .is_err()
    );
}

#[test]
fn navigation_keeps_each_pages_state() {
    let mut s = shop();
    let (main, second) = (page_name(&s, "Main"), page_name(&s, "Second"));
    let slicer = add_visual(
        &mut s,
        "Main",
        "slicer",
        json!([{"role": "Values", "field": "Product[Category]"}]),
    );
    let card = add_visual(
        &mut s,
        "Main",
        "card",
        json!([{"role": "Values", "field": "[Total]"}]),
    );
    let card2 = add_visual(
        &mut s,
        "Second",
        "card",
        json!([{"role": "Values", "field": "[Total]"}]),
    );
    let nav = add_visual(&mut s, "Main", "actionButton", json!([]));
    let (n, target, home) = (nav.clone(), second.clone(), main.clone());
    edit(&mut s, |r| {
        r.active_page = Some(home);
        for p in &mut r.pages {
            for v in &mut p.visuals {
                if v.name == n {
                    v.visual_extra.insert(
                        "visualContainerObjects".into(),
                        json!({"visualLink": [{"properties": {
                            "type": {"expr": {"Literal": {"Value": "'PageNavigation'"}}},
                            "navigationSection": {"expr": {"Literal": {"Value": format!("'{target}'")}}}}}]}),
                    );
                }
            }
        }
    });
    act(
        &mut s,
        json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": "Toys"}),
    );
    act(&mut s, json!({"action": "button", "page": main, "visual": nav}));
    let snap = s.snapshot();
    assert_eq!(snap["report"]["active_page"], json!(second));
    // a slicer filters its own page only
    assert_eq!(rows_of(&snap, &card2), vec![json!([1162.5])]);
    act(&mut s, json!({"action": "back"}));
    let snap = s.snapshot();
    assert_eq!(snap["report"]["active_page"], json!(main));
    assert_eq!(rows_of(&snap, &card), vec![json!([5])]);
    // by display name, and the state listing
    act(&mut s, json!({"action": "page", "page": "Second"}));
    let st = run(&mut s, "view.interactions", json!({}));
    assert_eq!(st["page"], json!(second));
    assert_eq!(st["slicers"][0]["values"], json!(["Toys"]));
    assert!(
        s.execute("view.interact", &json!({"action": "page", "page": "Nope"}))
            .is_err()
    );
    // reset drops everything
    act(&mut s, json!({"action": "reset"}));
    let snap = s.snapshot();
    assert_eq!(snap["report"]["active_page"], json!(main));
    assert_eq!(rows_of(&snap, &card), vec![json!([1162.5])]);
}

#[test]
fn charts_that_can_highlight_get_full_and_highlighted_values() {
    // oracle rules 9, 11, 14: a column chart and a pie highlight by default, keep their full values
    // and get the highlighted part; DataFilter makes the pie filter instead
    let mut s = shop();
    let main = page_name(&s, "Main");
    let bar = add_visual(
        &mut s,
        "Main",
        "clusteredBarChart",
        json!([{"role": "Category", "field": "Product[Category]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let col = add_visual(
        &mut s,
        "Main",
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Product]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let pie = add_visual(
        &mut s,
        "Main",
        "pieChart",
        json!([{"role": "Category", "field": "Sales[Product]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let all = rows(&s, &col);
    act(
        &mut s,
        json!({"action": "select", "page": main, "visual": bar, "point": [{"c": 0, "v": "Wheels"}]}),
    );
    let snap = s.snapshot();
    let v = visual_json(&snap, &col);
    assert_eq!(v["interaction"]["mode"], "highlight");
    // the full values stay; the highlighted part of each row, null where nothing is highlighted
    assert_eq!(rows_of(&snap, &col), all);
    let hl = v["data"]["highlights"].as_array().cloned().unwrap();
    for (r, h) in all.iter().zip(&hl) {
        let want = match r[0].as_str() {
            Some("Bike") => json!([null, 150.5]),
            Some("Car") => json!([null, 1000]),
            _ => json!([null, null]),
        };
        assert_eq!(h, &want, "{r}");
    }
    assert!(visual_json(&snap, &pie)["data"]["highlights"].is_array());
    // a slicer is filtered by the selection, never highlighted
    let sl = add_visual(
        &mut s,
        "Main",
        "slicer",
        json!([{"role": "Values", "field": "Sales[Product]"}]),
    );
    let snap = s.snapshot();
    assert_eq!(visual_json(&snap, &sl)["interaction"]["mode"], "filter");
    assert!(visual_json(&snap, &sl)["data"]["highlights"].is_null());
    assert_eq!(rows_of(&snap, &sl), vec![json!(["Bike"]), json!(["Car"])]);
    // DataFilter: the pie is filtered (only the Wheels products), without highlights
    let (src, dst) = (bar.clone(), pie.clone());
    edit(&mut s, |r| {
        if let Some(p) = r.pages.iter_mut().find(|p| p.display_name == "Main") {
            p.extra.insert(
                "visualInteractions".into(),
                json!([{"source": src, "target": dst, "type": "DataFilter"}]),
            );
        }
    });
    let snap = s.snapshot();
    let p = visual_json(&snap, &pie);
    assert_eq!(p["interaction"]["mode"], "filter");
    assert!(p["data"]["highlights"].is_null());
    assert_eq!(rows_of(&snap, &pie).len(), 2);
}

#[test]
fn synced_slicers_follow_a_click_on_every_page() {
    // oracle rule 6: a click on a synced slicer reaches its copies (hidden ones too)
    let mut s = shop();
    let (main, second) = (page_name(&s, "Main"), page_name(&s, "Second"));
    let a = add_visual(
        &mut s,
        "Main",
        "slicer",
        json!([{"role": "Values", "field": "Product[Category]"}]),
    );
    let b = add_visual(
        &mut s,
        "Second",
        "slicer",
        json!([{"role": "Values", "field": "Product[Category]"}]),
    );
    let card2 = add_visual(
        &mut s,
        "Second",
        "card",
        json!([{"role": "Values", "field": "[Total]"}]),
    );
    let (x, y) = (a.clone(), b.clone());
    edit(&mut s, |r| {
        for p in &mut r.pages {
            for v in &mut p.visuals {
                if v.name == x || v.name == y {
                    v.visual_extra.insert(
                        "syncGroup".into(),
                        json!({"groupName": "cat", "fieldChanges": true, "filterChanges": true}),
                    );
                    v.is_hidden = v.name == y;
                }
            }
        }
    });
    act(
        &mut s,
        json!({"action": "slicer_toggle", "page": main, "visual": a, "value": "Toys"}),
    );
    let snap = s.snapshot();
    assert_eq!(rows_of(&snap, &card2), vec![json!([5])]);
    assert_eq!(visual_json(&snap, &b)["slicer"]["selected"], json!(["Toys"]));
    let _ = second;
}

#[test]
fn private_view_states_over_one_session_never_mix() {
    // a server keeps one view state per viewer over a shared session (`interact_view`,
    // `snapshot_view`): one viewer's slicer changes neither the session nor another viewer
    let mut s = shop();
    let main = page_name(&s, "Main");
    let slicer = add_visual(
        &mut s,
        "Main",
        "slicer",
        json!([{"role": "Values", "field": "Product[Category]"}]),
    );
    let card = add_visual(
        &mut s,
        "Main",
        "card",
        json!([{"role": "Values", "field": "[Total]"}]),
    );
    let (revision, last) = (s.revision, s.last_command.clone());
    let mut a = bicraft_engine::Interactions::default();
    let b = bicraft_engine::Interactions::default();
    let r = s
        .interact_view(
            &mut a,
            &json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": "Toys"}),
        )
        .unwrap();
    assert_eq!(r["changed"], true);
    assert!(!a.is_empty());
    assert_eq!(rows_of(&s.snapshot_view(&a), &card), vec![json!([5])]);
    assert_eq!(rows_of(&s.snapshot_view(&b), &card), vec![json!([1162.5])]);
    // the session's own state, revision and last command are untouched
    assert!(s.interactions.is_empty());
    assert_eq!(rows(&s, &card), vec![json!([1162.5])]);
    assert_eq!((s.revision, s.last_command.clone()), (revision, last));
    // a refused action leaves the private state as it was
    let before = a.state_json();
    assert!(s.interact_view(&mut a, &json!({"action": "nope"})).is_err());
    assert_eq!(a.state_json(), before);
    // view.interact on the session still works on the session's own state
    act(
        &mut s,
        json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": "Wheels"}),
    );
    assert_eq!(rows(&s, &card), vec![json!([1150.5])]);
    assert_eq!(rows_of(&s.snapshot_view(&a), &card), vec![json!([5])]);
}

#[test]
fn the_report_can_cross_filter_by_default() {
    // Desktop's report option "Change default visual interaction from cross highlighting to
    // cross filtering" (settings.defaultFilterActionIsDataFilter): visuals that can highlight are
    // filtered unless their rule says HighlightFilter; the other rules are unchanged
    let mut s = shop();
    let main = page_name(&s, "Main");
    let bar = add_visual(
        &mut s,
        "Main",
        "clusteredBarChart",
        json!([{"role": "Category", "field": "Product[Category]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let col = add_visual(
        &mut s,
        "Main",
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Product]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let pie = add_visual(
        &mut s,
        "Main",
        "pieChart",
        json!([{"role": "Category", "field": "Sales[Product]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let card = add_visual(
        &mut s,
        "Main",
        "card",
        json!([{"role": "Values", "field": "[Total]"}]),
    );
    let snap = s.snapshot();
    assert_eq!(snap["report"]["settings"]["default_interaction"], "highlight");
    assert_eq!(
        visual_json(&snap, &col)["interactivity"],
        json!({"target": true, "highlight": true, "slicer": false})
    );
    assert_eq!(visual_json(&snap, &card)["interactivity"]["highlight"], false);
    let out = run(
        &mut s,
        "report.settings",
        json!({"default_interaction": "filter"}),
    );
    assert_eq!(out["changed"], true);
    assert_eq!(
        s.report.as_ref().unwrap().extra["settings"]["defaultFilterActionIsDataFilter"],
        json!(true)
    );
    // asking again changes nothing; no parameters reads the settings
    assert_eq!(
        run(
            &mut s,
            "report.settings",
            json!({"default_interaction": "filter"})
        )["changed"],
        false
    );
    assert_eq!(
        run(&mut s, "report.settings", json!({}))["default_interaction"],
        "filter"
    );
    run(
        &mut s,
        "page.interactions",
        json!({"page": main, "rules": [{"source": bar, "target": pie, "type": "highlight"}]}),
    );
    act(
        &mut s,
        json!({"action": "select", "page": main, "visual": bar, "point": [{"c": 0, "v": "Wheels"}]}),
    );
    let snap = s.snapshot();
    assert_eq!(snap["report"]["settings"]["default_interaction"], "filter");
    let pg = snap["report"]["pages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == main.as_str())
        .cloned()
        .unwrap();
    assert_eq!(
        pg["interactions"],
        json!([{"source": bar, "target": pie, "type": "highlight"}])
    );
    let c = visual_json(&snap, &col);
    assert_eq!(c["interaction"]["mode"], "filter");
    assert!(c["data"]["highlights"].is_null());
    assert_eq!(rows_of(&snap, &col).len(), 2, "only the Wheels products");
    let p = visual_json(&snap, &pie);
    assert_eq!(p["interaction"]["mode"], "highlight", "an explicit rule wins");
    assert!(p["data"]["highlights"].is_array());
    assert_eq!(visual_json(&snap, &card)["interaction"]["mode"], "filter");
    // back to highlighting: the setting is removed from the file
    run(
        &mut s,
        "report.settings",
        json!({"default_interaction": "highlight", "drill_filters_other_visuals": true}),
    );
    let r = s.report.as_ref().unwrap();
    assert!(
        r.extra["settings"]
            .get("defaultFilterActionIsDataFilter")
            .is_none()
    );
    assert_eq!(r.extra["settings"]["defaultDrillFilterOtherVisuals"], json!(true));
    let snap = s.snapshot();
    assert_eq!(visual_json(&snap, &col)["interaction"]["mode"], "highlight");
    assert_eq!(snap["report"]["settings"]["drill_filters_other_visuals"], true);
    // one undo step each
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(
        s.report.as_ref().unwrap().extra["settings"]["defaultFilterActionIsDataFilter"],
        json!(true)
    );
    for bad in [
        json!({"default_interaction": "none"}),
        json!({"drill_filters_other_visuals": "yes"}),
        json!({"x": 1}),
    ] {
        assert!(s.execute("report.settings", &bad).is_err(), "{bad}");
    }
}
