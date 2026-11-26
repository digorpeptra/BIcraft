//! The Filters pane and the M11.13 field edits on an engine session (`filters_pane.rs`,
//! `panes.rs`): a field dragged onto a filter section, basic (values from the engine), advanced
//! and top N filters, clear and remove; a new visual from a field (its box with nothing
//! selected, a drop on an empty part of the page); a whole hierarchy dragged to a well; a well's
//! field renamed for its visual by a double click; the same frames in both shells.

use std::path::Path;

use bicraft_engine::Session;
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::panes_ui_tests::{
    center, drag, expand_sales, fields, label_with, on_canvas, pairs, select, visual_named,
};
use super::{frame, harness, loaded, shell_app};
use crate::App;
use crate::ribbon::VisualSel;
use crate::shell::Caps;

/// Sales(Category, Product, Amount) with [Total] and the hierarchy Geo (Category > Product); one
/// page with a column chart (Category, [Total]) and a matrix ([Total]) in its top half.
pub(super) fn shop_session() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Product", "Amount"}, {{"Wheels", "Bike", 100}, {"Wheels", "Car", 1000}, {"Toys", "Ball", 5}, {"Games", "Chess", 20}})"#;
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Category", "data_type": "string"}),
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
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "hierarchy.add",
            json!({"table": "Sales", "name": "Geo", "levels": ["Category", "Product"]}),
        ),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    for (ty, rect, fields) in [
        (
            "clusteredColumnChart",
            [20, 20, 600, 300],
            json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]),
        ),
        (
            "pivotTable",
            [640, 20, 600, 300],
            json!([{"role": "Values", "field": "[Total]"}]),
        ),
    ] {
        s.execute(
            "visual.add",
            &json!({"page": page, "visual_type": ty, "x": rect[0], "y": rect[1], "width": rect[2],
                    "height": rect[3], "fields": fields}),
        )
        .unwrap();
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

pub(super) fn open() -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, shop_session)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1600.0, 900.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().placed.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    h
}

pub(super) fn snap(h: &Harness<'_, App>) -> Value {
    (*h.state().loaded.as_ref().unwrap().snap).clone()
}

pub(super) fn page(h: &Harness<'_, App>) -> Value {
    snap(h)["report"]["pages"][0].clone()
}

fn visual(h: &Harness<'_, App>, sel: &VisualSel) -> Value {
    page(h)["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == sel.visual.as_str())
        .cloned()
        .unwrap()
}

/// The chart's categories, as rendered from the latest snapshot.
fn categories(h: &Harness<'_, App>, sel: &VisualSel) -> Vec<String> {
    let v = visual(h, sel);
    assert_eq!(v["data"]["status"], "ok", "{}", v["data"]);
    let mut out: Vec<String> = v["data"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r[0].as_str().unwrap_or("").to_string())
        .collect();
    out.sort();
    out
}

/// The last edit sent (the values a basic card reads are not one).
pub(super) fn last_sent(h: &Harness<'_, App>) -> (&'static str, Value) {
    let c = h
        .state()
        .sent
        .iter()
        .rev()
        // the pane's value lists and the formula bar's reads (a measure clicked shows its DAX)
        .find(|c| {
            !matches!(
                c.id,
                "column.values"
                    | "measure.get"
                    | "column.get"
                    | "table.get"
                    | "dax.validate"
                    | "dax.functions"
            )
        })
        .unwrap();
    (c.id, c.params.clone())
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
    steps(h, 4);
}

#[test]
fn a_field_dropped_on_a_section_adds_a_card_and_basic_filters_apply() {
    let mut h = open();
    let chart = visual_named(&h, "clusteredColumnChart");
    expand_sales(&mut h);
    h.get_by_label("Filters on this page");
    h.get_by_label("Filters on all pages");
    assert!(
        h.query_by_label("Filters on this visual").is_none(),
        "nothing selected"
    );
    // Product onto Filters on this page: an empty card
    let (from, to) = (
        center(&h, "Product"),
        center(&h, "Add data fields here (Filters on this page)"),
    );
    drag(&mut h, from, to);
    assert_eq!(last_sent(&h).0, "page.filter");
    assert_eq!(page(&h)["filters"][0]["field"], "Sales[Product]");
    assert_eq!(page(&h)["filters"][0]["type"], "none");
    // the card, expanded by the drop: the values come from the engine
    h.get_by_label("Product is (All)");
    steps(&mut h, 6);
    h.get_by_label("Select all");
    for v in ["Ball", "Bike", "Car", "Chess"] {
        h.get_by_label(v);
    }
    // unchecking Car excludes it: the page re-renders without it
    click(&mut h, "Car");
    let (id, p) = last_sent(&h);
    assert_eq!(id, "page.filter");
    assert_eq!(p["not_in"], json!(["Car"]));
    assert_eq!(page(&h)["filters"][0]["condition"], json!({"not_in": ["Car"]}));
    h.get_by_label("Product is not Car");
    // Wheels is still there through Bike; Bike too: Wheels goes
    click(&mut h, "Bike");
    assert_eq!(categories(&h, &chart), vec!["Games", "Toys"]);
    // the search narrows the list
    let search = label_with(
        &h,
        egui::accesskit::Role::TextInput,
        &["Search the values of Product"],
    );
    h.get_by_label(&search).click();
    steps(&mut h, 2);
    h.get_by_label(&search).type_text("ch");
    steps(&mut h, 3);
    assert!(h.query_by_label("Ball").is_none());
    h.get_by_label("Chess");
    // Clear filter (the eraser) keeps the card, everything shown again
    click(&mut h, "Clear filter on Product");
    assert_eq!(last_sent(&h).1["keep_cards"], true);
    assert_eq!(page(&h)["filters"][0]["type"], "none");
    assert_eq!(categories(&h, &chart), vec!["Games", "Toys", "Wheels"]);
    // × removes it
    click(&mut h, "Remove filter on Product");
    assert_eq!(page(&h)["filters"], json!([]));
    // undo brings the card back
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    steps(&mut h, 4);
    assert_eq!(page(&h)["filters"][0]["field"], "Sales[Product]");
    // a measure cannot go to a page filter: the slot does not take it
    let sent = h.state().sent.len();
    let (from, to) = (
        center(&h, "Total"),
        center(&h, "Add data fields here (Filters on all pages)"),
    );
    drag(&mut h, from, to);
    assert!(h.state().sent.len() <= sent + 1);
    assert_eq!(snap(&h)["report"]["filters"], json!([]));
}

/// The pane's search box filters the cards by field, as Desktop's pane search does.
#[test]
fn the_filters_pane_search_filters_the_cards() {
    let mut h = open();
    expand_sales(&mut h);
    // Product onto Filters on this page: a card
    let (from, to) = (
        center(&h, "Product"),
        center(&h, "Add data fields here (Filters on this page)"),
    );
    drag(&mut h, from, to);
    h.get_by_label("Product is (All)");
    // a query on no field hides the card but keeps the sections and their slots
    h.state_mut().filters.find = "zzz".into();
    steps(&mut h, 3);
    assert!(h.query_by_label("Product is (All)").is_none());
    h.get_by_label("Filters on this page");
    h.get_by_label("Filters on all pages");
    // the card's own field brings it back
    h.state_mut().filters.find = "prod".into();
    steps(&mut h, 3);
    h.get_by_label("Product is (All)");
    // clearing the box shows everything again
    h.state_mut().filters.find.clear();
    steps(&mut h, 3);
    h.get_by_label("Product is (All)");
}

#[test]
fn advanced_and_top_n_filters_on_a_visual() {
    let mut h = open();
    let chart = visual_named(&h, "clusteredColumnChart");
    select(&mut h, &chart);
    // the visual's bound fields are its cards
    h.get_by_label("Filters on this visual");
    h.get_by_label("Category is (All)");
    h.get_by_label("Total is (All)");
    // Total: advanced only (a measure)
    click(&mut h, "Total is (All)");
    h.get_by_label("Filter type of Total");
    let cond = label_with(&h, egui::accesskit::Role::ComboBox, &["Condition 1 Total"]);
    click(&mut h, &cond);
    click(&mut h, "is greater than or equal to");
    let value = label_with(&h, egui::accesskit::Role::TextInput, &["Value 1 Total"]);
    h.get_by_label(&value).click();
    steps(&mut h, 2);
    h.get_by_label(&value).type_text("20");
    steps(&mut h, 2);
    click(&mut h, "Apply filter on Total");
    let (id, p) = last_sent(&h);
    assert_eq!(id, "visual.filter");
    assert_eq!((p["op"].clone(), p["value"].clone()), (json!(">="), json!(20.0)));
    assert_eq!(categories(&h, &chart), vec!["Games", "Wheels"]);
    h.get_by_label("Total is greater than or equal to 20");
    // Category: Top 1 by Total
    click(&mut h, "Category is (All)");
    click(&mut h, "Filter type of Category");
    click(&mut h, "Top N");
    let n = label_with(
        &h,
        egui::accesskit::Role::TextInput,
        &["Number of items of Category"],
    );
    h.get_by_label(&n).click();
    steps(&mut h, 2);
    h.key_press(Key::Backspace);
    h.key_press(Key::Backspace);
    h.get_by_label(&n).type_text("1");
    steps(&mut h, 2);
    // By value: Total dragged from the Data pane
    expand_sales(&mut h);
    let by = label_with(&h, egui::accesskit::Role::TextInput, &["By value of Category"]);
    let (from, to) = (center(&h, "Total"), center(&h, &by));
    drag(&mut h, from, to);
    click(&mut h, "Apply filter on Category");
    let (id, p) = last_sent(&h);
    assert_eq!(id, "visual.filter");
    assert_eq!((p["top"].clone(), p["by"].clone()), (json!(1), json!("[Total]")));
    assert_eq!(categories(&h, &chart), vec!["Wheels"]);
    let v = visual(&h, &chart);
    let types: Vec<Value> = v["filters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["type"].clone())
        .collect();
    assert!(
        types.contains(&json!("topn")) && types.contains(&json!("advanced")),
        "{types:?}"
    );
    h.get_by_label("Category Top 1 by Total");
    // a basic filter on the same card replaces it
    click(&mut h, "Filter type of Category");
    click(&mut h, "Basic filtering");
    steps(&mut h, 6);
    click(&mut h, "Games");
    assert_eq!(last_sent(&h).1["not_in"], json!(["Games"]));
}

#[test]
fn a_field_makes_a_new_visual_of_desktops_type() {
    let mut h = open();
    expand_sales(&mut h);
    let before = page(&h)["visuals"].as_array().unwrap().len();
    // the box of a text column with nothing selected: a table, selected
    click(&mut h, "Product");
    let (id, p) = last_sent(&h);
    assert_eq!(id, "visual.add");
    assert_eq!(p["visual_type"], "tableEx");
    assert_eq!(
        p["fields"],
        json!([{"role": "Values", "field": "Sales[Product]"}])
    );
    let visuals = page(&h)["visuals"].as_array().unwrap().clone();
    assert_eq!(visuals.len(), before + 1);
    let made = visuals.last().unwrap().clone();
    // the first free spot: under the two visuals
    assert!(made["y"].as_f64().unwrap() >= 320.0, "{made}");
    let sel = h.state().cmd.visual.clone().expect("the new visual is selected");
    assert_eq!(sel.visual, made["name"].as_str().unwrap());
    // the next box goes into it
    click(&mut h, "Total");
    assert!(fields(&h, &sel).contains(&("Values".into(), "[Total]".into())));
    // a measure dropped on an empty part of the page: a column chart there
    h.state_mut().cmd.visual = None;
    steps(&mut h, 2);
    let placed = h.state().placed.unwrap();
    let to = placed.rect.min + egui::vec2(700.0, 420.0) * placed.zoom;
    let (from, _) = (center(&h, "Total"), ());
    drag(&mut h, from, to);
    let (id, p) = last_sent(&h);
    assert_eq!(id, "visual.add");
    assert_eq!(p["visual_type"], "clusteredColumnChart");
    assert_eq!(p["fields"], json!([{"role": "Y", "field": "[Total]"}]));
    assert!((p["x"].as_f64().unwrap() - 700.0).abs() <= 2.0, "{p}");
    assert!((p["y"].as_f64().unwrap() - 420.0).abs() <= 2.0, "{p}");
    // then a category: a clustered column chart of a measure by a category
    let sel = h.state().cmd.visual.clone().unwrap();
    click(&mut h, "Category");
    assert_eq!(
        fields(&h, &sel),
        pairs(&[("Y", "[Total]"), ("Category", "Sales[Category]")])
    );
    assert_eq!(visual(&h, &sel)["data"]["status"], "ok");
    // a number column: summed
    let f = crate::panes::Field {
        text: "Sales[Amount]".into(),
        table: "Sales".into(),
        name: "Amount".into(),
        measure: false,
        numeric: true,
        summarize: None,
        category: None,
    };
    assert_eq!(
        crate::panes::new_visual_for(&f),
        ("clusteredColumnChart", "Y", "Sum(Sales[Amount])".into())
    );
    let city = crate::panes::Field {
        numeric: false,
        category: Some("City".into()),
        ..f
    };
    assert_eq!(crate::panes::new_visual_for(&city).0, "map");
}

#[test]
fn a_hierarchy_dragged_to_a_well_binds_its_levels() {
    let mut h = open();
    let matrix = visual_named(&h, "pivotTable");
    select(&mut h, &matrix);
    expand_sales(&mut h);
    let (from, to) = (center(&h, "Geo"), center(&h, "Rows"));
    drag(&mut h, from, to);
    let (id, p) = last_sent(&h);
    assert_eq!(id, "visual.bind_hierarchy");
    assert_eq!(
        (p["hierarchy"].clone(), p["role"].clone()),
        (json!("Geo"), json!("Rows"))
    );
    let v = visual(&h, &matrix);
    let rows: Vec<&Value> = v["fields"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["role"] == "Rows")
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["kind"], "hierarchy_level");
    assert_eq!(v["data"]["status"], "ok", "{}", v["data"]);
    // its levels are in the well, removable by position
    h.get_by_label("Category in Rows");
    let x = label_with(&h, egui::accesskit::Role::Button, &["Remove", "Product"]);
    click(&mut h, &x);
    assert_eq!(last_sent(&h).1["index"], 1);
    // onto the chart on the canvas: its axis
    let chart = visual_named(&h, "clusteredColumnChart");
    let (from, to) = (center(&h, "Geo"), on_canvas(&h, &chart));
    drag(&mut h, from, to);
    let (id, p) = last_sent(&h);
    assert_eq!(id, "visual.bind_hierarchy");
    assert_eq!(p["role"], "Category");
    assert_eq!(h.state().cmd.visual.as_ref(), Some(&chart));
}

#[test]
fn a_double_click_renames_a_field_for_its_visual() {
    let mut h = open();
    let chart = visual_named(&h, "clusteredColumnChart");
    select(&mut h, &chart);
    let at = center(&h, "Total in Y-axis");
    double_click(&mut h, at);
    steps(&mut h, 3);
    assert!(
        h.state().panes.renaming.is_some(),
        "a double click starts the rename"
    );
    assert!(editing(&h, "Total"), "the box holds the name");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text("Revenue".into()));
    steps(&mut h, 2);
    h.key_press(Key::Enter);
    steps(&mut h, 5);
    let (id, p) = last_sent(&h);
    assert_eq!(id, "visual.rename_field");
    assert_eq!(
        (p["name"].clone(), p["field"].clone()),
        (json!("Revenue"), json!("[Total]"))
    );
    let v = visual(&h, &chart);
    assert_eq!(v["fields"][1]["display_name"], "Revenue");
    assert_eq!(v["auto_title"], "Revenue by Category");
    h.get_by_label("Revenue in Y-axis");
    // Escape keeps the name
    let at = center(&h, "Revenue in Y-axis");
    double_click(&mut h, at);
    steps(&mut h, 3);
    let sent = h.state().sent.len();
    assert!(editing(&h, "Revenue"));
    h.event(Event::Text("x".into()));
    h.step();
    h.key_press(Key::Escape);
    steps(&mut h, 4);
    assert_eq!(h.state().sent.len(), sent);
    h.get_by_label("Revenue in Y-axis");
}

/// A well's value field: right click > Show value as > Percent of grand total sends
/// `visual.show_value_as`, the chart then shows shares and the well `%GT Total`; a chart offers
/// no column or row total, a category no Show value as.
#[test]
fn the_well_menu_shows_a_value_as_a_percentage() {
    let mut h = open();
    let chart = visual_named(&h, "clusteredColumnChart");
    select(&mut h, &chart);
    let at = center(&h, "Total in Y-axis");
    super::backstage_tests::right_click(&mut h, at);
    steps(&mut h, 2);
    // the submenu's button carries its arrow
    let sub = label_with(&h, egui::accesskit::Role::Button, &["Show value as"]);
    click(&mut h, &sub);
    assert!(
        h.query_by_label("Percent of column total").is_none(),
        "a chart has no column total"
    );
    click(&mut h, "Percent of grand total");
    steps(&mut h, 4);
    let (id, p) = last_sent(&h);
    assert_eq!(id, "visual.show_value_as");
    assert_eq!(
        (p["role"].clone(), p["field"].clone(), p["as"].clone()),
        (json!("Y"), json!("[Total]"), json!("percent_of_grand_total"))
    );
    let v = visual(&h, &chart);
    assert_eq!(v["fields"][1]["show_as"], "percent_of_grand_total");
    assert_eq!(v["data"]["columns"][1]["format_string"], "0.00%");
    let shares: f64 = v["data"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r[1].as_f64().unwrap_or(0.0))
        .sum();
    assert!(
        (shares - 1.0).abs() < 1e-9,
        "the shares add up to 100 %: {shares}"
    );
    h.get_by_label("%GT Total in Y-axis");
    let at = center(&h, "Category in X-axis");
    super::backstage_tests::right_click(&mut h, at);
    steps(&mut h, 2);
    assert!(h.query_by_label(&sub).is_none(), "a category is no value");
}

/// A well's category field: right click > Show items with no data sends
/// `visual.show_items_no_data`; the menu then shows it checked, and a click turns it off.
#[test]
fn the_well_menu_shows_items_with_no_data() {
    let mut h = open();
    let chart = visual_named(&h, "clusteredColumnChart");
    select(&mut h, &chart);
    let at = center(&h, "Category in X-axis");
    super::backstage_tests::right_click(&mut h, at);
    steps(&mut h, 2);
    click(&mut h, "Show items with no data");
    let (id, p) = last_sent(&h);
    assert_eq!(id, "visual.show_items_no_data");
    assert_eq!(
        (p["role"].clone(), p["show"].clone()),
        (json!("Category"), json!(true))
    );
    assert_eq!(visual(&h, &chart)["fields"][0]["show_all"], true);
    // checked now: a click turns it off
    let at = center(&h, "Category in X-axis");
    super::backstage_tests::right_click(&mut h, at);
    steps(&mut h, 2);
    click(&mut h, "Show items with no data");
    assert_eq!(last_sent(&h).1["show"], false);
    assert!(visual(&h, &chart)["fields"][0].get("show_all").is_none());
    // a value has no items to show
    let at = center(&h, "Total in Y-axis");
    super::backstage_tests::right_click(&mut h, at);
    steps(&mut h, 2);
    h.get_by_label("Rename for this visual");
    assert!(h.query_by_label("Show items with no data").is_none());
}

/// The Filters pane draws the same frames in the desktop and the browser shells: nothing
/// selected, a visual selected with a card open, light and dark.
#[test]
fn the_filters_pane_draws_the_same_frames_in_both_shells() {
    let mut s = shop_session();
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute(
        "page.filter",
        &json!({"page": page, "field": "Sales[Category]", "in": ["Toys", "Wheels"]}),
    )
    .unwrap();
    s.execute(
        "report.filter",
        &json!({"field": "Sales[Product]", "op": "contains", "value": "a"}),
    )
    .unwrap();
    let snap = loaded(s.snapshot());
    let chart = {
        let p = &snap.snap["report"]["pages"][0];
        let v = p["visuals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["type"] == "clusteredColumnChart")
            .unwrap();
        VisualSel {
            page: p["name"].as_str().unwrap().into(),
            visual: v["name"].as_str().unwrap().into(),
        }
    };
    for dark in [false, true] {
        let mut desktop = shell_app(Caps::DESKTOP, &snap, "en-US");
        let mut browser = shell_app(Caps::BROWSER, &snap, "en-US");
        for sel in [None, Some(chart.clone())] {
            for h in [&mut desktop, &mut browser] {
                if dark {
                    h.state_mut().set_theme(crate::theme::Brightness::Dark);
                }
                h.state_mut().cmd.visual = sel.clone();
                let page = chart.page.clone();
                h.state_mut()
                    .filters
                    .open
                    .insert("report:::Sales[Product]".to_string());
                h.state_mut()
                    .filters
                    .open
                    .insert(format!("visual:{page}:{}:[Total]", chart.visual));
                steps(h, 3);
            }
            assert!(desktop.query_by_label("Filters on all pages").is_some());
            assert!(desktop.query_by_label("Category is Toys, Wheels").is_some());
            let (a, b) = (frame(&desktop), frame(&browser));
            assert!(a.0 == b.0, "dark {dark} {sel:?}: the meshes differ");
            assert!(a.1 == b.1, "dark {dark} {sel:?}: the font atlases differ");
        }
    }
}

/// Damaged filter lists never break the pane.
#[test]
fn damaged_filter_lists_never_break_the_pane() {
    let _: Pos2 = Pos2::ZERO;
    for bad in [
        json!({"report": {"filters": [1, {"field": 3}, {"field": "x", "type": "basic", "condition": {"in": 5}}],
                          "pages": [{"name": "p", "filters": {"a": 1}, "visuals": [{"name": "v", "type": "x",
                          "fields": [{"role": "R", "kind": "column", "text": 9}], "filters": [{"field": "T[C]",
                          "kind": "column", "table": "T", "column": "C", "type": "topn", "condition": {"top": "x"}}]}]}]}}),
        json!({"report": {"filters": "x", "pages": [{"name": "p"}]}}),
    ] {
        let mut app = App::new("en-US", super::fixed(Caps::DESKTOP, None));
        app.loaded = Some(loaded(bad));
        app.cmd.visual = Some(VisualSel {
            page: "p".into(),
            visual: "v".into(),
        });
        app.filters.open.insert("visual:p:v:T[C]".into());
        app.filters.open.insert("report:::x".into());
        let mut h = harness(app, (1400.0, 800.0));
        steps(&mut h, 4);
    }
}

/// Two quick primary clicks at `pos`.
fn double_click(h: &mut Harness<'_, App>, pos: Pos2) {
    // a harness step is 0.25 s and may take two passes: a person's quick double click
    h.ctx
        .options_mut(|o| o.input_options.max_double_click_delay = 2.0);
    h.hover_at(pos);
    h.step();
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        h.step();
    }
}

/// Whether a text box holding `value` is on screen (the rename box).
fn editing(h: &Harness<'_, App>, value: &str) -> bool {
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .any(|n| n.accesskit_node().value().as_deref() == Some(value))
}

/// Sales(Category, Day, Amount) with [Total]; a column chart (Category, [Total]); the clock pinned
/// to Wednesday 2026-11-18 10:30.
fn dated_session() -> Session {
    let mut s = Session::new();
    let sales = "#table(type table [Category = text, Day = datetime, Amount = number], {\
        {\"Wheels\", #datetime(2026, 11, 18, 9, 0, 0), 100}, {\"Toys\", #datetime(2026, 11, 16, 0, 0, 0), 5}, \
        {\"Games\", #datetime(2026, 10, 2, 0, 0, 0), 20}})";
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Category", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Day", "data_type": "dateTime"}),
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
        ("view.set_as_of", json!({"as_of": "2026-11-18T10:30:00"})),
    ] {
        s.execute(id, &p).unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute(
        "visual.add",
        &json!({"page": page, "visual_type": "clusteredColumnChart", "x": 20, "y": 20, "width": 600,
                "height": 300, "fields": [{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]}),
    )
    .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn open_dated() -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, dated_session)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1600.0, 900.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().placed.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    h
}

/// Day dropped on Filters on this page, its card switched to Relative date filtering.
fn relative_day_card(h: &mut Harness<'_, App>) {
    expand_sales(h);
    let (from, to) = (
        center(h, "Day"),
        center(h, "Add data fields here (Filters on this page)"),
    );
    drag(h, from, to);
    assert_eq!(last_sent(h).0, "page.filter");
    h.get_by_label("Day is (All)");
    click(h, "Filter type of Day");
    click(h, "Relative date filtering");
}

/// Types `text` into the text box labelled `label` (its old text removed).
fn retype(h: &mut Harness<'_, App>, label: &str, text: &str) {
    let n = label_with(h, egui::accesskit::Role::TextInput, &[label]);
    h.get_by_label(&n).click();
    steps(h, 2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.key_press(Key::Backspace);
    h.get_by_label(&n).type_text(text);
    steps(h, 2);
}

/// Relative date filtering on a page card: is in the last N days, Include today, this month,
/// relative time; each Apply filter one `page.filter {relative}`; the page re-renders with the
/// window; the card names the condition.
#[test]
fn a_relative_date_card_filters_the_page() {
    let mut h = open_dated();
    let chart = visual_named(&h, "clusteredColumnChart");
    relative_day_card(&mut h);
    h.get_by_label("Range of Day");
    h.get_by_label("Period of Day");
    retype(&mut h, "Number of periods of Day", "3");
    click(&mut h, "Apply filter on Day");
    let (id, p) = last_sent(&h);
    assert_eq!(id, "page.filter");
    assert_eq!(
        p["relative"],
        json!({"range": "last", "count": 3, "unit": "days", "include_today": true})
    );
    assert_eq!(page(&h)["filters"][0]["type"], "relative");
    // 11-16 and today are in the last 3 days, 10-02 is not
    assert_eq!(categories(&h, &chart), vec!["Toys", "Wheels"]);
    h.get_by_label("Day is in the last 3 days");
    // today left out
    click(&mut h, "Include today (Day)");
    click(&mut h, "Apply filter on Day");
    assert_eq!(last_sent(&h).1["relative"]["include_today"], false);
    assert_eq!(categories(&h, &chart), vec!["Toys"]);
    h.get_by_label("Day is in the last 3 days (not including today)");
    // this month: a range without a count
    click(&mut h, "Range of Day");
    click(&mut h, "is in this");
    assert!(
        label_with_opt(&h, "Number of periods of Day").is_none(),
        "this takes no count"
    );
    click(&mut h, "Period of Day");
    click(&mut h, "Months");
    click(&mut h, "Apply filter on Day");
    assert_eq!(
        last_sent(&h).1["relative"],
        json!({"range": "this", "count": 1, "unit": "months"})
    );
    assert_eq!(categories(&h, &chart), vec!["Toys", "Wheels"]);
    h.get_by_label("Day is in this month");
    // relative time: the last 2 hours keep 9:00 today only
    click(&mut h, "Filter type of Day");
    click(&mut h, "Relative time filtering");
    click(&mut h, "Range of Day");
    click(&mut h, "is in the last");
    retype(&mut h, "Number of periods of Day", "2");
    click(&mut h, "Period of Day");
    click(&mut h, "Hours");
    assert!(h.query_by_label("Include today (Day)").is_none());
    click(&mut h, "Apply filter on Day");
    assert_eq!(
        last_sent(&h).1["relative"],
        json!({"range": "last", "count": 2, "unit": "hours"})
    );
    assert_eq!(categories(&h, &chart), vec!["Wheels"]);
    h.get_by_label("Day is in the last 2 hours");
    // undo goes back one Apply at a time
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    steps(&mut h, 4);
    assert_eq!(categories(&h, &chart), vec!["Toys", "Wheels"]);
    h.get_by_label("Day is in this month");
}

/// A text column's card offers no relative filtering.
#[test]
fn relative_filtering_is_offered_on_date_columns_only() {
    let mut h = open_dated();
    expand_sales(&mut h);
    let (from, to) = (
        center(&h, "Category"),
        center(&h, "Add data fields here (Filters on this page)"),
    );
    drag(&mut h, from, to);
    h.get_by_label("Category is (All)");
    click(&mut h, "Filter type of Category");
    h.get_by_label("Advanced filtering");
    assert!(h.query_by_label("Relative date filtering").is_none());
    assert!(h.query_by_label("Relative time filtering").is_none());
}

/// The text box whose label holds `part`, if one is on screen.
fn label_with_opt(h: &Harness<'_, App>, part: &str) -> Option<String> {
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .filter_map(|n| n.accesskit_node().label())
        .find(|l| l.contains(part))
}

/// The lock and the eye on a card: each click is one `page.filter {field, locked | hidden}`, the
/// condition kept; a reader does not see the hidden card and sees the lock of a locked one.
#[test]
fn the_lock_and_the_eye_of_a_card() {
    let mut h = open_dated();
    let chart = visual_named(&h, "clusteredColumnChart");
    relative_day_card(&mut h);
    click(&mut h, "Apply filter on Day");
    assert_eq!(categories(&h, &chart), vec!["Wheels"], "today only");
    click(&mut h, "Lock filter on Day");
    let (id, p) = last_sent(&h);
    assert_eq!((id, p["locked"].clone()), ("page.filter", json!(true)));
    assert!(p.get("relative").is_none());
    assert_eq!(page(&h)["filters"][0]["locked"], true);
    assert_eq!(
        page(&h)["filters"][0]["type"],
        "relative",
        "the condition is kept"
    );
    h.get_by_label("Unlock filter on Day");
    // the eye
    click(&mut h, "Hide filter on Day from readers");
    assert_eq!(last_sent(&h).1["hidden"], true);
    assert_eq!(page(&h)["filters"][0]["hidden"], true);
    h.get_by_label("Show filter on Day to readers");
    assert_eq!(
        categories(&h, &chart),
        vec!["Wheels"],
        "a hidden filter still applies"
    );
    // undo: one toggle
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    steps(&mut h, 4);
    assert_eq!(page(&h)["filters"][0]["hidden"], false);
    h.get_by_label("Hide filter on Day from readers");
    // a reader: the locked card shows its lock, no toggles
    h.state_mut().access = Some("reader".into());
    steps(&mut h, 3);
    h.get_by_label("Locked filter: Day");
    assert!(h.query_by_label("Unlock filter on Day").is_none());
    assert!(h.query_by_label("Hide filter on Day from readers").is_none());
    // hidden again (as an editor): the reader no longer sees the card
    h.state_mut().access = None;
    steps(&mut h, 3);
    click(&mut h, "Hide filter on Day from readers");
    h.state_mut().access = Some("reader".into());
    steps(&mut h, 3);
    assert!(h.query_by_label("Locked filter: Day").is_none());
    assert!(h.query_by_label("Day is in the last 1 day").is_none());
    h.get_by_label("Filters on this page");
}
