//! The Build pane's Format visual tab on an engine session (`format_pane.rs`, ROADMAP M11.22):
//! the sub-tabs, the title typed as free text, a card's switch, a colour swatch, an axis, the
//! search, Reset to default, undo; the ribbon's Format button; the title form's free text field
//! (the regression of the title drawn as a JSON box); every card's edits accepted by the engine
//! for its visual type; readers; damaged snapshots; the same frames in both shells.

use std::path::Path;

use bicraft_engine::Session;
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::{fixed, frame, harness, loaded, shell_app, shop_snapshot};
use crate::App;
use crate::format_pane::{self, Ctl, Group, Tab, To};
use crate::ribbon::VisualSel;
use crate::shell::Caps;

/// The visual types the tests put on the page, one per family of cards.
const TYPES: &[&str] = &[
    "clusteredColumnChart",
    "lineChart",
    "pieChart",
    "funnel",
    "waterfallChart",
    "card",
    "tableEx",
    "pivotTable",
    "slicer",
    "shape",
];

/// Sales(Category, Amount) with [Total]; one page with a titled column chart, a card, a slicer
/// and a table; with `all`, one visual of every type of [`TYPES`].
fn session_with(all: bool) -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Amount"}, {{"Wheels", 100}, {"Toys", 5}, {"Games", 20}})"#;
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
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "Tips"})),
    ] {
        s.execute(id, &p).unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let tips = s.report.as_ref().unwrap().pages[1].name.clone();
    s.execute("page.set_type", &json!({"page": tips, "type": "tooltip"}))
        .unwrap();
    let types: Vec<&str> = if all {
        TYPES.to_vec()
    } else {
        vec!["clusteredColumnChart", "card", "slicer"]
    };
    for (i, ty) in types.iter().enumerate() {
        let fields = match *ty {
            "shape" => json!([]),
            "card" => json!([{"role": "Values", "field": "[Total]"}]),
            "slicer" => json!([{"role": "Values", "field": "Sales[Category]"}]),
            "tableEx" => {
                json!([{"role": "Values", "field": "Sales[Category]"}, {"role": "Values", "field": "[Total]"}])
            }
            "pivotTable" => {
                json!([{"role": "Rows", "field": "Sales[Category]"}, {"role": "Values", "field": "[Total]"}])
            }
            "pieChart" | "funnel" => {
                json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}])
            }
            _ => json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]),
        };
        let (x, y) = (20 + (i % 3) as i64 * 400, 20 + (i / 3) as i64 * 240);
        let out = s
            .execute(
                "visual.add",
                &json!({"page": page, "visual_type": ty, "x": x, "y": y, "width": 380, "height": 220,
                        "fields": fields}),
            )
            .unwrap();
        if *ty == "clusteredColumnChart" {
            let v = out["visual"].as_str().unwrap().to_string();
            s.execute(
                "visual.set_title",
                &json!({"page": page, "visual": v, "title": "Sales by category"}),
            )
            .unwrap();
        }
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

/// The window on a fresh session, the report shown, a reader's when `reader`.
fn open(reader: bool) -> Harness<'static, App> {
    open_with(reader, false)
}

/// [`open`] on [`session_with`]`(all)`.
fn open_with(reader: bool, all: bool) -> Harness<'static, App> {
    let mut app =
        App::new("en-US", engine_shell(Caps::DESKTOP, move || session_with(all))).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1500.0, 900.0));
    steps(&mut h, 6);
    if reader {
        h.state_mut().access = Some("reader".into());
    }
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().placed.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    h
}

/// The visual of type `ty` on the first page.
fn visual_of(h: &Harness<'_, App>, ty: &str) -> VisualSel {
    let snap = h.state().loaded.as_ref().unwrap().snap.clone();
    let page = &snap["report"]["pages"][0];
    let v = page["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["type"] == ty)
        .unwrap();
    VisualSel {
        page: page["name"].as_str().unwrap().into(),
        visual: v["name"].as_str().unwrap().into(),
    }
}

/// The snapshot entry of visual `sel`.
fn snap_visual(h: &Harness<'_, App>, sel: &VisualSel) -> Value {
    let snap = h.state().loaded.as_ref().unwrap().snap.clone();
    snap["report"]["pages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|p| p["visuals"].as_array().cloned().unwrap_or_default())
        .find(|v| v["name"] == sel.visual.as_str())
        .unwrap()
}

/// The last edit sent (the tab's `visual.props` and `column.values` reads left out), without page
/// and visual.
fn last_edit(h: &Harness<'_, App>) -> (&'static str, Value) {
    let c = h
        .state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id != "visual.props" && c.id != "column.values")
        .expect("an edit was sent");
    let mut p = c.params.clone();
    if let Some(o) = p.as_object_mut() {
        o.shift_remove("page");
        o.shift_remove("visual");
    }
    (c.id, p)
}

/// Steps until no command is waiting.
fn settle(h: &mut Harness<'_, App>) {
    for _ in 0..100 {
        h.step();
        if h.state().calls_pending() == 0 {
            break;
        }
    }
    steps(h, 3);
}

/// Selects `sel` and shows the Format visual tab on `group`, by clicks.
fn format_tab(h: &mut Harness<'_, App>, sel: &VisualSel, group: &str) {
    h.state_mut().cmd.visual = Some(sel.clone());
    steps(h, 3);
    h.get_by_role_and_label(Role::Button, "Format visual").click();
    steps(h, 3);
    assert_eq!(h.state().panes.format.tab, Tab::Format);
    h.get_by_label(group).click();
    settle(h);
}

/// Opens card `title` (its header).
fn open_card(h: &mut Harness<'_, App>, title: &str) {
    h.get_by_role_and_label(Role::Button, title).click();
    steps(h, 3);
}

/// Closes card `title` (its header) once its rows are no longer needed, so the cards below it
/// stay in the pane's viewport (a click on a clipped header is not delivered).
fn close_card(h: &mut Harness<'_, App>, title: &str, key: &str) {
    open_card(h, title);
    assert!(!h.state().panes.format.open.contains(key), "{title} is closed");
}

/// What `visual.props` answers for `object.property`, from the `format.objects` list.
fn object_value(h: &Harness<'_, App>, object: &str, property: &str) -> Value {
    let own = h.state().panes.format.own.as_ref().expect("props read");
    own.props["format"]["objects"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|o| o["object"] == object && o["property"] == property)
        .map(|o| o["value"].clone())
        .unwrap_or(Value::Null)
}

/// The regression of the title edited as a JSON box: the title form (the palette's Title, the
/// ribbon's Title before M11.22) gives a one-line free text field holding the visual's title;
/// what is typed there is the title.
#[test]
fn the_title_form_is_a_free_text_field_with_the_title() {
    let mut h = open(false);
    let chart = visual_of(&h, "clusteredColumnChart");
    h.state_mut().cmd.visual = Some(chart.clone());
    steps(&mut h, 3);
    let c = crate::registry::find("visual.set_title").unwrap();
    let p = crate::ribbon::prefill(h.state(), c, None);
    crate::ribbon::invoke(h.state_mut(), c.id, p);
    steps(&mut h, 3);
    let field = h.get_by_label("title");
    assert_eq!(
        field.accesskit_node().role(),
        Role::TextInput,
        "the title is a one-line text field, not a code box"
    );
    assert_eq!(
        field.accesskit_node().value().as_deref(),
        Some("Sales by category"),
        "the field holds the visual's title"
    );
    field.focus();
    steps(&mut h, 1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text("Ventes".into()));
    steps(&mut h, 2);
    h.get_by_label("Run").click();
    settle(&mut h);
    let f = h.state().form.as_ref().unwrap();
    assert!(matches!(f.result, Some(Ok(_))), "{:?}", f.result);
    assert_eq!(last_edit(&h), ("visual.set_title", json!({"title": "Ventes"})));
    assert_eq!(snap_visual(&h, &chart)["title"], "Ventes");
}

/// The Format visual tab: the title typed as free text (Enter commits one command, undo takes it
/// back), the subtitle's switch, a colour swatch, the X-axis switch, Reset to default.
#[test]
fn the_format_tab_edits_a_visual_with_one_command_per_edit() {
    let mut h = open(false);
    let chart = visual_of(&h, "clusteredColumnChart");
    // the tabs: Build visual, then Format visual
    h.state_mut().cmd.visual = Some(chart.clone());
    steps(&mut h, 3);
    h.get_by_role_and_label(Role::Button, "Build visual");
    assert!(
        h.query_by_label("Clustered column chart").is_some(),
        "Build visual shows the gallery cell of the selected type"
    );
    format_tab(&mut h, &chart, "General cards");
    assert!(h.query_by_label("Clustered column chart").is_none());
    // General › Title: the text is free text
    open_card(&mut h, "Title");
    let text = h.get_by_label("Title Text");
    assert_eq!(text.accesskit_node().role(), Role::TextInput);
    assert_eq!(
        text.accesskit_node().value().as_deref(),
        Some("Sales by category")
    );
    text.focus();
    steps(&mut h, 1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text("Ventes par catégorie".into()));
    steps(&mut h, 1);
    let before = h.state().sent.iter().filter(|c| c.id != "visual.props").count();
    h.key_press(Key::Enter);
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.set_title",
            json!({"title": {"text": "Ventes par catégorie"}})
        )
    );
    assert_eq!(
        h.state().sent.iter().filter(|c| c.id != "visual.props").count(),
        before + 1,
        "one command for the typed text"
    );
    assert_eq!(snap_visual(&h, &chart)["title"], "Ventes par catégorie");
    assert_eq!(
        h.get_by_label("Title Text").accesskit_node().value().as_deref(),
        Some("Ventes par catégorie")
    );
    // undo takes the text back
    h.state_mut().run(
        crate::actions::Call {
            id: "edit.undo",
            params: json!({}),
        },
        crate::app::Then::Nothing,
    );
    settle(&mut h);
    assert_eq!(snap_visual(&h, &chart)["title"], "Sales by category");
    assert_eq!(
        h.get_by_label("Title Text").accesskit_node().value().as_deref(),
        Some("Sales by category"),
        "the field follows the undone title"
    );
    // the title's switch (the header's toggle) and its bold button
    h.get_by_role_and_label(Role::CheckBox, "Title").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        ("visual.set_title", json!({"title": {"show": false}}))
    );
    assert_eq!(snap_visual(&h, &chart)["format"]["title"]["show"], false);
    h.get_by_label("Title Bold").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        ("visual.set_title", json!({"title": {"bold": true}}))
    );
    // Effects › Background colour: a swatch of the popup is one command
    // Title closed, Effects open: its rows stay in view
    open_card(&mut h, "Title");
    open_card(&mut h, "Effects");
    h.get_by_label("Effects Background Colour").click();
    steps(&mut h, 3);
    h.get_by_label("Effects Background Colour #252423").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        ("visual.format", json!({"background": {"color": "#252423"}}))
    );
    assert_eq!(
        snap_visual(&h, &chart)["format"]["background"]["color"],
        "#252423"
    );
    // the shadow's switch
    h.get_by_role_and_label(Role::CheckBox, "Shadow").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        ("visual.format", json!({"shadow": {"show": true}}))
    );
    // Reset to default puts Effects back in one command
    h.get_by_label("Reset to default Effects").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.format",
            json!({"background": null, "border": null, "shadow": null})
        )
    );
    assert!(snap_visual(&h, &chart)["format"]["background"]["color"] != "#252423");
    // Visual › X-axis: its switch through visual.axis
    h.get_by_label("Visual cards").click();
    settle(&mut h);
    h.get_by_role_and_label(Role::CheckBox, "X-axis").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.format",
            json!({"objects": [{"object": "categoryAxis", "property": "show", "value": false}]})
        )
    );
    assert_eq!(snap_visual(&h, &chart)["format"]["x_axis"]["show"], false);
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
}

/// The search box keeps the cards whose name or properties match, opened.
#[test]
fn the_search_box_filters_the_cards() {
    let mut h = open(false);
    let chart = visual_of(&h, "clusteredColumnChart");
    format_tab(&mut h, &chart, "Visual cards");
    for c in ["X-axis", "Y-axis", "Legend", "Data labels", "Data colours"] {
        h.get_by_role_and_label(Role::Button, c);
    }
    h.get_by_label("Search the format cards").focus();
    steps(&mut h, 1);
    h.event(egui::Event::Text("posit".into()));
    steps(&mut h, 3);
    assert!(h.query_by_label("X-axis").is_none());
    h.get_by_role_and_label(Role::Button, "Legend");
    // shown open: its properties are there
    h.get_by_label("Legend Position");
    h.state_mut().panes.format.search = "nothing like it".into();
    steps(&mut h, 3);
    h.get_by_label("No card matches.");
}

/// The Visual group follows the visual's type: a card's callout value, a slicer's header.
#[test]
fn the_visual_cards_follow_the_type() {
    let mut h = open(false);
    let card = visual_of(&h, "card");
    format_tab(&mut h, &card, "Visual cards");
    h.get_by_role_and_label(Role::Button, "Callout value");
    assert!(h.query_by_label("X-axis").is_none());
    open_card(&mut h, "Callout value");
    h.get_by_label("Callout value Font size").click();
    steps(&mut h, 3);
    h.get_by_label("12").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.format",
            json!({"objects": [{"object": "labels", "property": "fontSize", "value": 12.0}]})
        )
    );
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
    let slicer = visual_of(&h, "slicer");
    h.state_mut().cmd.visual = Some(slicer);
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Slicer header");
}

/// The Visual group follows the visual's type, object by object: a pie has no axes, a table no
/// legend, a gauge a Gauge axis and a Target, a card a Callout value; and every card's switch and
/// rows name an object and a property the catalog gives that very type.
#[test]
fn the_visual_cards_follow_each_type() {
    let titles =
        |ty: &str| -> Vec<&'static str> { format_pane::visual_cards(ty).iter().map(|c| c.en).collect() };
    let pie = titles("pieChart");
    assert!(pie.contains(&"Legend") && pie.contains(&"Detail labels"));
    assert!(!pie.iter().any(|c| c.ends_with("-axis")), "{pie:?}");
    let table = titles("tableEx");
    assert!(table.contains(&"Values") && table.contains(&"Grid"));
    assert!(!table.contains(&"Legend"), "{table:?}");
    let gauge = titles("gauge");
    assert!(
        gauge.contains(&"Gauge axis") && gauge.contains(&"Target"),
        "{gauge:?}"
    );
    assert!(titles("slicer").contains(&"Slicer header"));
    assert!(titles("card").contains(&"Callout value"));
    let column = titles("clusteredColumnChart");
    assert!(column.contains(&"X-axis") && column.contains(&"Y-axis"));
    assert!(
        column.iter().position(|c| *c == "Legend") < column.iter().position(|c| *c == "Data colours"),
        "{column:?}"
    );
    // no card twice, and every target is an object and property of the catalog for that type
    for t in TYPES {
        let cards = format_pane::visual_cards(t);
        let mut seen = std::collections::BTreeSet::new();
        for c in &cards {
            assert!(seen.insert(c.key), "{t}: {} twice", c.key);
            if let Some((To::Object(obj, prop), _)) = c.switch {
                assert!(
                    bicraft_visuals::property(t, obj, prop).is_some(),
                    "{t} › {obj}.{prop}"
                );
            }
            for sec in c.sections.iter() {
                for p in sec.props.iter() {
                    let To::Object(obj, prop) = p.to else {
                        panic!("{t} › {}: {:?} is not an object property", c.key, p.to);
                    };
                    assert!(
                        bicraft_visuals::property(t, obj, prop).is_some(),
                        "{t} › {obj}.{prop} is not in the catalog"
                    );
                }
            }
        }
    }
}

/// Every control kind of a generated Visual card commits one object property: the header switch,
/// a list, a numeric list (written as a number), a style button, a colour swatch and Reset to
/// default. Whatever the visual's type, a card's widget writes through `visual.format`.
#[test]
fn the_generated_cards_controls_commit_their_object() {
    let mut h = open(false);
    let chart = visual_of(&h, "clusteredColumnChart");
    format_tab(&mut h, &chart, "Visual cards");
    // the header switch: Data labels on, and the card reads it back
    h.get_by_role_and_label(Role::CheckBox, "Data labels").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.format",
            json!({"objects": [{"object": "labels", "property": "show", "value": true}]})
        )
    );
    assert_eq!(
        object_value(&h, "labels", "show"),
        json!(true),
        "the switch reads its value back"
    );
    // Legend › Position: a list of the catalog's values
    open_card(&mut h, "Legend");
    h.get_by_label("Legend Position").click();
    steps(&mut h, 3);
    h.get_by_label("Right").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.format",
            json!({"objects": [{"object": "legend", "property": "position", "value": "Right"}]})
        )
    );
    assert_eq!(
        object_value(&h, "legend", "position"),
        json!("Right"),
        "the list reads its value back"
    );
    close_card(&mut h, "Legend", "legend");
    // Y-axis › Display units: a numeric list, written as a number (`1000D` in PBIR)
    open_card(&mut h, "Y-axis");
    h.get_by_label("Y-axis Display units").click();
    steps(&mut h, 3);
    h.get_by_label("Thousands").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.format",
            json!({"objects": [{"object": "valueAxis", "property": "labelDisplayUnits", "value": 1000.0}]})
        )
    );
    assert_eq!(
        object_value(&h, "valueAxis", "labelDisplayUnits"),
        json!({"literal": "1000D"}),
        "the numeric list reads its value back"
    );
    close_card(&mut h, "Y-axis", "valueAxis");
    // Data labels › Bold: a style button
    open_card(&mut h, "Data labels");
    assert!(
        h.state().panes.format.open.contains("labels"),
        "the click on the header opens the card: {:?}",
        h.state().panes.format.open
    );
    h.get_by_label("Data labels Bold").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.format",
            json!({"objects": [{"object": "labels", "property": "bold", "value": true}]})
        )
    );
    assert_eq!(
        object_value(&h, "labels", "bold"),
        json!(true),
        "the style button reads its value back"
    );
    close_card(&mut h, "Data labels", "labels");
    // Data colours › Colour: a swatch of the popup, written as `{color}`
    open_card(&mut h, "Data colours");
    h.get_by_label("Data colours Colour").click();
    steps(&mut h, 3);
    h.get_by_label("Data colours Colour #252423").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.format",
            json!({"objects": [{"object": "dataPoint", "property": "fill", "value": {"color": "#252423"}}]})
        )
    );
    assert_eq!(
        object_value(&h, "dataPoint", "fill"),
        json!({"color": "#252423"}),
        "the swatch reads its value back"
    );
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
    // Reset to default: one command clearing this card's properties
    h.get_by_label("Reset to default Data colours").click();
    settle(&mut h);
    let (id, p) = last_edit(&h);
    assert_eq!(id, "visual.format");
    let objs = p["objects"].as_array().expect("objects");
    assert_eq!(objs.len(), 6, "{p}");
    assert!(
        objs.iter()
            .all(|o| o["object"] == "dataPoint" && o["value"].is_null()),
        "{p}"
    );
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
}

/// Every built-in type of the Visualizations pane's gallery offers Visual format cards, so the
/// pane never falls back to the "no Visual cards" note for a type the gallery can add. The script
/// visuals and RDL have no formatting object of their own yet: their note says so.
#[test]
fn every_gallery_type_has_visual_cards() {
    let h = open(false);
    let types = h.state().loaded.as_ref().unwrap().snap["report"]["visual_types"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(types.len() >= 20, "the catalog lists the built-in types");
    for t in &types {
        let name = t["name"].as_str().unwrap_or("");
        if matches!(name, "pythonVisual" | "scriptVisual" | "rdlVisual") {
            continue;
        }
        assert!(
            !format_pane::visual_cards(name).is_empty(),
            "{name}: no Visual format card"
        );
    }
}

/// The ribbon's Format button opens the Build pane on the Format visual tab (the Visual tab no
/// longer holds the formatting commands).
#[test]
fn the_ribbon_format_button_opens_the_format_tab() {
    let mut h = open(false);
    let chart = visual_of(&h, "clusteredColumnChart");
    h.state_mut().cmd.visual = Some(chart);
    h.state_mut().prefs.panes.build = false;
    steps(&mut h, 3);
    h.state_mut().select_ribbon_tab(crate::registry::Tab::Visual);
    steps(&mut h, 3);
    for gone in [
        "Title",
        "Axes",
        "Series format",
        "Conditional formatting",
        "Tooltip page",
    ] {
        assert!(
            h.query_by_label(gone).is_none(),
            "{gone} is no ribbon button any more"
        );
    }
    h.get_by_label("Format").click();
    steps(&mut h, 3);
    assert!(h.state().prefs.panes.build, "the Build pane opens");
    assert_eq!(h.state().panes.format.tab, Tab::Format);
    h.get_by_label("General cards");
}

/// A reader has no Format visual tab, and the control channel's buildTab says so.
#[test]
fn readers_see_no_format_tab() {
    let mut h = open(true);
    let chart = visual_of(&h, "clusteredColumnChart");
    h.state_mut().cmd.visual = Some(chart);
    h.state_mut().panes.format.tab = Tab::Format;
    steps(&mut h, 3);
    assert!(h.query_by_label("Format visual").is_none());
    assert_eq!(h.state().panes.format.tab, Tab::Build);
    assert!(h.query_by_label("Search the format cards").is_none());
    assert!(!format_pane::intercept(
        h.state_mut(),
        "visual.format",
        &json!({})
    ));
}

/// Every edit and reset of every card is accepted by the engine for its visual type (the cards
/// name the catalog's objects and properties), and each is one command.
#[test]
fn every_card_edit_is_accepted_by_the_engine() {
    let mut s = session_with(true);
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let visuals: Vec<(String, String)> = s.report.as_ref().unwrap().pages[0]
        .visuals
        .iter()
        .map(|v| (v.name.clone(), v.visual_type.clone().unwrap_or_default()))
        .collect();
    let tips = s.report.as_ref().unwrap().pages[1].name.clone();
    let sample = |ctl: Ctl| -> Value {
        match ctl {
            Ctl::Text => json!("Some text"),
            Ctl::Size => json!(12.0),
            Ctl::Font => json!("Arial"),
            Ctl::Color => json!("#123456"),
            Ctl::Align => json!("center"),
            Ctl::Style(_) | Ctl::Toggle => json!(true),
            Ctl::Percent => json!(40.0),
            Ctl::Number(lo, hi) => json!(((lo + hi) / 2.0).round()),
            Ctl::Choice(v) | Ctl::Numbered(v) => json!(v.last().map_or("", |x| x.0)),
            Ctl::Pages => json!(tips),
            Ctl::Run(..) | Ctl::RunWith(..) | Ctl::Fx(_) => Value::Null,
        }
    };
    let mut ran = 0;
    for (v, ty) in &visuals {
        for group in [Group::General, Group::Visual] {
            let cards = format_pane::cards(group, ty);
            if group == Group::Visual {
                assert!(!cards.is_empty(), "{ty} has Visual cards");
            }
            for card in cards {
                let mut edits: Vec<(&str, Value)> = Vec::new();
                if let Some((to, _)) = card.switch {
                    edits.push(format_pane::edit(to, Ctl::Toggle, json!(false)));
                }
                for sec in card.sections.iter() {
                    if let Some((to, _)) = sec.switch {
                        edits.push(format_pane::edit(to, Ctl::Toggle, json!(true)));
                    }
                    for p in sec.props.iter() {
                        if matches!(p.ctl, Ctl::Run(..) | Ctl::RunWith(..) | Ctl::Fx(_)) {
                            continue;
                        }
                        edits.push(format_pane::edit(p.to, p.ctl, sample(p.ctl)));
                    }
                }
                edits.extend(format_pane::reset(&card));
                for (id, params) in edits {
                    let mut p = params.clone();
                    p["page"] = json!(page);
                    p["visual"] = json!(v);
                    let r = s.execute(id, &p);
                    assert!(r.is_ok(), "{ty} › {}: {id} {params}: {r:?}", card.key);
                    if let Ok(out) = &r {
                        let warnings = out["warnings"].as_array().cloned().unwrap_or_default();
                        assert!(
                            warnings.is_empty(),
                            "{ty} › {}: {id} {params}: {warnings:?}",
                            card.key
                        );
                    }
                    ran += 1;
                }
            }
        }
    }
    assert!(ran > 200, "{ran} edits");
    // To::Scaling and the image card need an image: their command and shape are checked here
    assert_eq!(
        format_pane::edit(To::Scaling, Ctl::Choice(&[]), json!("fit")),
        ("image.set", json!({"scaling": "fit"}))
    );
}

/// Damaged snapshots and odd `visual.props` answers never break the Format visual tab.
#[test]
fn damaged_snapshots_never_break_the_format_tab() {
    let odd_props = [
        json!(null),
        json!({"title": 3, "format": {"background": "x", "padding": {"top": "a"}, "objects": [1, {"object": 2}]},
               "axes": {"x": [1]}, "tooltip_page": {"a": 1}}),
        json!({"format": [], "axes": "y", "title": {"show": "maybe", "font_size": "big", "color": {"color": 4}}}),
    ];
    for snap in [
        json!({"revision": 2, "report": {"theme": {"data_colors": [1, "#zzz", "#ABCDEF"]}, "pages": [{"name": "p", "visuals": [
            {"name": "v", "type": "clusteredColumnChart", "title": 5,
             "format": {"title": {"show": "yes", "font_size": "huge", "font_family": 4}, "x_axis": [1],
                        "background": {"color": "#12"}, "series_colors": [{"color": 3}, "x"]}}]},
            {"name": 7}, {"display_name": "nameless"}]}}),
        json!({"report": {"pages": [{"name": "p", "visuals": [{"name": "v", "type": "card", "format": null}]}]}}),
        json!({"report": {"pages": [{"name": "p", "visuals": [{"name": "v"}]}]}}),
        json!({"report": {"pages": [{"name": "p", "visuals": [{"name": "v", "type": "image", "image": {"scaling": 9}}]}]}}),
    ] {
        for props in &odd_props {
            let mut app = App::new("en-US", fixed(Caps::DESKTOP, None));
            app.loaded = Some(loaded(snap.clone()));
            app.cmd.visual = Some(VisualSel {
                page: "p".into(),
                visual: "v".into(),
            });
            app.panes.format.tab = Tab::Format;
            app.panes.format.own = Some(format_pane::Own {
                page: "p".into(),
                visual: "v".into(),
                revision: 2,
                props: props.clone(),
            });
            for k in [
                "title",
                "effects",
                "categoryAxis",
                "valueAxis",
                "legend",
                "labels",
                "image",
                "tooltips",
            ] {
                app.panes.format.open.insert(k.into());
            }
            let mut h = harness(app, (1200.0, 760.0));
            steps(&mut h, 4);
            h.state_mut().panes.format.group = Group::General;
            steps(&mut h, 3);
            h.state_mut().panes.format.search = "ti".into();
            steps(&mut h, 2);
        }
    }
}

/// The Format visual tab draws the same frames in the desktop and browser shells.
#[test]
fn the_format_tab_draws_the_same_frames_in_both_shells() {
    let snap = shop_snapshot();
    let page = &snap.snap["report"]["pages"][0];
    let chart = page["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["type"] == "clusteredColumnChart")
        .unwrap();
    let sel = VisualSel {
        page: page["name"].as_str().unwrap().into(),
        visual: chart["name"].as_str().unwrap().into(),
    };
    for dark in [false, true] {
        let mut desktop = shell_app(Caps::DESKTOP, &snap, "en-US");
        let mut browser = shell_app(Caps::BROWSER, &snap, "en-US");
        for group in [Group::Visual, Group::General] {
            for h in [&mut desktop, &mut browser] {
                if dark {
                    h.state_mut().set_theme(crate::theme::Brightness::Dark);
                }
                h.state_mut().cmd.visual = Some(sel.clone());
                let f = &mut h.state_mut().panes.format;
                f.tab = Tab::Format;
                f.group = group;
                for k in ["title", "effects", "categoryAxis", "legend"] {
                    f.open.insert(k.into());
                }
                steps(h, 3);
            }
            assert!(desktop.query_by_label("Search the format cards").is_some());
            let (a, b) = (frame(&desktop), frame(&browser));
            assert!(a.0 == b.0, "dark {dark} {group:?}: the meshes differ");
            assert!(a.1 == b.1, "dark {dark} {group:?}: the font atlases differ");
        }
    }
}

/// The Data colours card lists one colour per category value, as Desktop does under the default
/// colour: a pie's slices always, a funnel's stages once Show all is on (the catalog gives the
/// funnel `showAllDataPoints`). A swatch writes that value's colour through `visual.series` (a
/// `{field, value}` target, one undo step), and the row reads it back from the snapshot.
#[test]
fn the_data_colours_card_colours_each_value() {
    let mut h = open_with(false, true);
    let pie = visual_of(&h, "pieChart");
    format_tab(&mut h, &pie, "Visual cards");
    open_card(&mut h, "Data colours");
    settle(&mut h);
    for v in ["Games", "Toys", "Wheels"] {
        assert!(
            h.query_by_label(&format!("Data colours {v}")).is_some(),
            "a row for {v}"
        );
    }
    h.get_by_label("Data colours Toys").click();
    steps(&mut h, 3);
    h.get_by_label("Data colours Toys #252423").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.series",
            json!({"target": {"field": "Sales[Category]", "value": "Toys"}, "color": "#252423"})
        )
    );
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
    let colors = snap_visual(&h, &pie)["format"]["series_colors"].clone();
    assert!(
        colors
            .as_array()
            .into_iter()
            .flatten()
            .any(|c| c["field"] == "Sales[Category]" && c["value"] == "Toys" && c["color"] == "#252423"),
        "the colour is the value's: {colors}"
    );
    // a funnel: no value rows until Show all is on
    let funnel = visual_of(&h, "funnel");
    h.state_mut().cmd.visual = Some(funnel);
    settle(&mut h);
    if !h.state().panes.format.open.contains("dataPoint") {
        open_card(&mut h, "Data colours");
    }
    assert!(
        h.query_by_label("Data colours Toys").is_none(),
        "no value rows while Show all is off"
    );
    h.get_by_label("Data colours Show all").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.format",
            json!({"objects": [{"object": "dataPoint", "property": "showAllDataPoints", "value": true}]})
        )
    );
    settle(&mut h);
    assert!(
        h.query_by_label("Data colours Toys").is_some(),
        "the value rows once Show all is on"
    );
}

/// Which rows the Data colours card lists: several measures one each, a legend's values, a pie's
/// categories always, a single measure only with Show all.
#[test]
fn the_data_colours_rows_follow_the_fields() {
    use format_pane::{Colouring, colouring, series_rows};
    let col = |role: &str| json!({"role": role, "kind": "column", "table": "Sales", "name": "Category", "text": "Sales[Category]"});
    let m = |name: &str| json!({"role": "Y", "kind": "measure", "table": "Sales", "name": name, "text": format!("Sales[{name}]"), "label": name});
    let values = Colouring::Values {
        field: "Sales[Category]".into(),
        table: "Sales".into(),
        column: "Category".into(),
    };
    let one = [col("Category"), m("Total")];
    assert_eq!(colouring("clusteredColumnChart", &one, false), Colouring::None);
    assert_eq!(colouring("clusteredColumnChart", &one, true), values);
    assert_eq!(colouring("pieChart", &one, false), values);
    assert_eq!(
        colouring("clusteredColumnChart", &[m("Total"), col("Series")], false),
        values
    );
    let two = [col("Category"), m("Total"), m("Cost")];
    let c = colouring("lineChart", &two, false);
    assert!(matches!(&c, Colouring::Measures(v) if v.len() == 2), "{c:?}");
    // the measures' colours: the visual's own, else the theme's in order
    let snap = json!({"format": {"series_colors": [{"column": 2, "color": "#FF0000"}]}});
    let rows = series_rows(
        &c,
        &snap,
        None,
        &["#111111".into(), "#222222".into()],
        crate::i18n::Lang::En,
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (
            rows[0].label.as_str(),
            rows[0].color.as_deref(),
            rows[0].def.as_str()
        ),
        ("Total", None, "#111111")
    );
    assert_eq!(
        (rows[1].target.clone(), rows[1].color.as_deref()),
        (json!("Sales[Cost]"), Some("#FF0000"))
    );
    // a damaged snapshot or no values: no rows, no panic
    assert!(series_rows(&values, &json!(null), None, &[], crate::i18n::Lang::En).is_empty());
}

// ----- the fx buttons and the conditional-formatting dialog -----

/// Whether the control labelled `name` is marked (selected or toggled on).
fn marked(h: &Harness<'_, App>, name: &str) -> bool {
    let n = h.get_by_label(name).accesskit_node();
    n.is_selected() == Some(true) || n.toggled() == Some(egui::accesskit::Toggled::True)
}

/// The visual's conditional formats, as `visual.props` answered.
fn conditional(h: &Harness<'_, App>) -> Value {
    let own = h.state().panes.format.own.as_ref().expect("props read");
    own.props["conditional"].clone()
}

const CHART_FX: &str = "Data colours Colour Conditional formatting";

/// Desktop's fx buttons sit beside the colours it formats conditionally: a chart's data colour, a
/// table's values background and text colour, a card's callout colour; not beside the others.
/// A click opens the dialog on the visual's first number field.
#[test]
fn the_fx_button_opens_the_conditional_dialog() {
    let mut h = open_with(false, true);
    let chart = visual_of(&h, "clusteredColumnChart");
    format_tab(&mut h, &chart, "Visual cards");
    open_card(&mut h, "Data colours");
    assert!(!marked(&h, CHART_FX), "no conditional format yet");
    h.get_by_label(CHART_FX).click();
    steps(&mut h, 3);
    let d = h.state().panes.format.cf.clone().expect("the dialog is open");
    assert_eq!(
        (d.target, d.field.as_str(), d.existing),
        ("data_color", "[Total]", false)
    );
    h.get_by_role_and_label(Role::ComboBox, "Format style");
    h.get_by_role_and_label(Role::ComboBox, "What field should we base this on?");
    h.get_by_label("Lowest value colour");
    h.get_by_label("Highest value colour");
    // Cancel closes it, nothing sent
    let sent = h.state().sent.len();
    h.get_by_label("Cancel").click();
    steps(&mut h, 3);
    assert!(h.state().panes.format.cf.is_none());
    assert_eq!(h.state().sent.len(), sent);
    // a table's values: background and text colour, not the alternate background
    let table = visual_of(&h, "tableEx");
    h.state_mut().cmd.visual = Some(table);
    settle(&mut h);
    if !h.state().panes.format.open.contains("values") {
        open_card(&mut h, "Values");
    }
    h.get_by_label("Values Background Conditional formatting");
    h.get_by_label("Values Text colour Conditional formatting");
    assert!(
        h.query_by_label("Values Alternate background Conditional formatting")
            .is_none()
    );
    // a card's callout value
    let card = visual_of(&h, "card");
    h.state_mut().cmd.visual = Some(card);
    settle(&mut h);
    if !h.state().panes.format.open.contains("labels") {
        open_card(&mut h, "Callout value");
    }
    h.get_by_label("Callout value Colour Conditional formatting");
}

/// The dialog's Gradient (its default style): OK sends one `visual.conditional` with the field
/// and the colours picked; the engine accepts it, the visual's props list it, and the fx button
/// is marked.
#[test]
fn the_fx_dialog_applies_a_gradient() {
    let mut h = open(false);
    let chart = visual_of(&h, "clusteredColumnChart");
    format_tab(&mut h, &chart, "Visual cards");
    open_card(&mut h, "Data colours");
    h.get_by_label(CHART_FX).click();
    steps(&mut h, 3);
    h.get_by_label("Highest value colour").click();
    steps(&mut h, 3);
    h.get_by_label("Highest value colour #252423").click();
    steps(&mut h, 3);
    h.get_by_label("OK").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.conditional",
            json!({"target": "data_color",
                   "rule": {"gradient": {"field": "[Total]", "min": "#FFFFFF", "max": "#252423"}}})
        )
    );
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
    assert!(h.state().panes.format.cf.is_none(), "OK closes the dialog");
    let c = conditional(&h);
    assert_eq!(c[0]["target"], "data_color", "{c}");
    assert_eq!(c[0]["rule"]["gradient"]["max"], "#252423", "{c}");
    assert!(marked(&h, CHART_FX), "the fx button shows the format");
    // opened again, the dialog reads it back
    h.get_by_label(CHART_FX).click();
    steps(&mut h, 3);
    let d = h.state().panes.format.cf.clone().expect("open");
    assert!(d.existing);
    assert_eq!(d.max.color, "#252423");
}

/// The dialog's Rules on a table's values background: the series, a rule typed (operator, number,
/// colour), OK as one `visual.conditional`; the engine accepts it.
#[test]
fn the_fx_dialog_applies_rules() {
    let mut h = open_with(false, true);
    let table = visual_of(&h, "tableEx");
    format_tab(&mut h, &table, "Visual cards");
    open_card(&mut h, "Values");
    h.get_by_label("Values Background Conditional formatting").click();
    steps(&mut h, 3);
    h.get_by_role_and_label(Role::ComboBox, "Apply to series");
    h.get_by_role_and_label(Role::ComboBox, "Format style").click();
    steps(&mut h, 3);
    h.get_by_label("Rules").click();
    steps(&mut h, 3);
    h.get_by_label("+ New rule").click();
    steps(&mut h, 3);
    h.get_by_label("Rule 1 value").focus();
    steps(&mut h, 1);
    h.event(egui::Event::Text("10".into()));
    steps(&mut h, 2);
    h.get_by_label("Rule 1 colour").click();
    steps(&mut h, 3);
    h.get_by_label("Rule 1 colour #252423").click();
    steps(&mut h, 3);
    h.get_by_label("OK").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.conditional",
            json!({"target": "background", "series": "[Total]",
                   "rule": {"rules": {"field": "[Total]",
                                      "cases": [{"op": ">", "value": 10.0, "color": "#252423"}]}}})
        )
    );
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
    let c = conditional(&h);
    assert_eq!(
        (
            c[0]["target"].clone(),
            c[0]["series"].clone(),
            c[0]["rule"]["rules"]["cases"][0]["color"].clone()
        ),
        (json!("background"), json!("[Total]"), json!("#252423")),
        "{c}"
    );
    assert!(marked(&h, "Values Background Conditional formatting"));
    assert!(!marked(&h, "Values Text colour Conditional formatting"));
}

/// Remove turns a conditional format off with one `visual.conditional {rule: null}`.
#[test]
fn the_fx_dialog_removes_the_format() {
    let mut h = open(false);
    let chart = visual_of(&h, "clusteredColumnChart");
    h.state_mut().run(
        crate::actions::Call {
            id: "visual.conditional",
            params: json!({"page": chart.page, "visual": chart.visual, "target": "data_color",
                           "rule": {"field_value": {"field": "[Total]"}}}),
        },
        crate::app::Then::Nothing,
    );
    settle(&mut h);
    format_tab(&mut h, &chart, "Visual cards");
    open_card(&mut h, "Data colours");
    settle(&mut h);
    assert!(marked(&h, CHART_FX), "the format the visual has");
    h.get_by_label(CHART_FX).click();
    steps(&mut h, 3);
    let d = h.state().panes.format.cf.clone().expect("open");
    assert_eq!(d.style, crate::conditional_dialog::Style::FieldValue);
    h.get_by_label("Remove conditional formatting").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.conditional",
            json!({"target": "data_color", "rule": null})
        )
    );
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
    assert!(conditional(&h).is_null(), "{}", conditional(&h));
    assert!(!marked(&h, CHART_FX));
}

const TITLE_COLOR_FX: &str = "Title Text colour Conditional formatting";
const TITLE_TEXT_FX: &str = "Title Text Conditional formatting";

/// The General cards' fx buttons: the title's and subtitle's text and colour, the background's
/// and the border's colour (not the shadow's); the title colour's dialog has no series (one
/// value for the whole visual, even on a table) and OK sends one `visual.conditional` the engine
/// writes in `visualContainerObjects`; the button is then marked.
#[test]
fn the_general_cards_fx_buttons_format_the_container() {
    let mut h = open_with(false, true);
    let table = visual_of(&h, "tableEx");
    format_tab(&mut h, &table, "General cards");
    open_card(&mut h, "Title");
    h.get_by_label(TITLE_TEXT_FX);
    assert!(!marked(&h, TITLE_COLOR_FX));
    close_card(&mut h, "Title", "title");
    open_card(&mut h, "Subtitle");
    h.get_by_label("Subtitle Text Conditional formatting");
    h.get_by_label("Subtitle Text colour Conditional formatting");
    close_card(&mut h, "Subtitle", "subtitle");
    open_card(&mut h, "Effects");
    h.get_by_label("Effects Background Colour Conditional formatting");
    h.get_by_label("Effects Visual border Colour Conditional formatting");
    assert!(
        h.query_by_label("Effects Shadow Colour Conditional formatting")
            .is_none(),
        "no fx where Desktop has none"
    );
    close_card(&mut h, "Effects", "effects");
    open_card(&mut h, "Title");
    h.get_by_label(TITLE_COLOR_FX).click();
    steps(&mut h, 3);
    let d = h.state().panes.format.cf.clone().expect("the dialog is open");
    assert_eq!((d.target, d.table), ("title_color", false));
    assert!(
        h.query_by_label("Apply to series").is_none(),
        "a title is one value for the visual"
    );
    h.get_by_label("Highest value colour").click();
    steps(&mut h, 3);
    h.get_by_label("Highest value colour #252423").click();
    steps(&mut h, 3);
    h.get_by_label("OK").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.conditional",
            json!({"target": "title_color",
                   "rule": {"gradient": {"field": "[Total]", "min": "#FFFFFF", "max": "#252423"}}})
        )
    );
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
    let c = conditional(&h);
    assert_eq!(c[0]["target"], "title_color", "{c}");
    assert!(c[0].get("series").is_none(), "{c}");
    assert!(marked(&h, TITLE_COLOR_FX), "the fx button shows the format");
    assert!(!marked(&h, TITLE_TEXT_FX));
}

/// The title text's fx: Field value only (Desktop's "Title text" dialog); a measure picked, OK
/// sends the title from that field; the engine writes it and reads it back.
#[test]
fn the_title_text_fx_takes_a_field() {
    let mut h = open(false);
    let chart = visual_of(&h, "clusteredColumnChart");
    format_tab(&mut h, &chart, "General cards");
    open_card(&mut h, "Title");
    h.get_by_label(TITLE_TEXT_FX).click();
    steps(&mut h, 3);
    let d = h.state().panes.format.cf.clone().expect("the dialog is open");
    assert_eq!(
        (d.target, d.style, d.field.as_str()),
        ("title_text", crate::conditional_dialog::Style::FieldValue, "")
    );
    // no field yet: OK cannot run
    let sent = h.state().sent.len();
    h.get_by_label("OK").click();
    steps(&mut h, 3);
    assert_eq!(h.state().sent.len(), sent);
    // only Field value is offered
    h.get_by_role_and_label(Role::ComboBox, "Format style").click();
    steps(&mut h, 3);
    assert!(h.query_by_label("Gradient").is_none() && h.query_by_label("Rules").is_none());
    h.get_by_role_and_label(Role::ComboBox, "Format style").click();
    steps(&mut h, 3);
    h.get_by_role_and_label(Role::ComboBox, "What field should we base this on?")
        .click();
    steps(&mut h, 3);
    h.get_by_label("Sales › Total").click();
    steps(&mut h, 3);
    h.get_by_label("OK").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.conditional",
            json!({"target": "title_text", "rule": {"field_value": {"field": "[Total]"}}})
        )
    );
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
    assert_eq!(conditional(&h)[0]["target"], "title_text");
    assert!(marked(&h, TITLE_TEXT_FX));
}

/// A chart's data labels colour and a card's category label colour have their fx.
#[test]
fn the_label_colours_have_their_fx() {
    let mut h = open(false);
    let chart = visual_of(&h, "clusteredColumnChart");
    format_tab(&mut h, &chart, "Visual cards");
    open_card(&mut h, "Data labels");
    h.get_by_label("Data labels Colour Conditional formatting")
        .click();
    steps(&mut h, 3);
    let d = h.state().panes.format.cf.clone().expect("the dialog is open");
    assert_eq!(d.target, "label_color");
    h.get_by_label("Cancel").click();
    steps(&mut h, 3);
    let card = visual_of(&h, "card");
    h.state_mut().cmd.visual = Some(card);
    settle(&mut h);
    if !h.state().panes.format.open.contains("categoryLabels") {
        open_card(&mut h, "Category label");
    }
    h.get_by_label("Category label Colour Conditional formatting");
}

const BARS_FX: &str = "Cell elements Data bars Conditional formatting";
const ICONS_FX: &str = "Cell elements Icons Conditional formatting";
const URL_FX: &str = "Cell elements Web URL Conditional formatting";
const BG_FX: &str = "Values Background Conditional formatting";

/// The table on the Format visual tab with only card `key` open (the others collapsed, so its
/// rows stay in the pane's viewport).
fn table_card(key: &str) -> Harness<'static, App> {
    let mut h = open_with(false, true);
    let table = visual_of(&h, "tableEx");
    format_tab(&mut h, &table, "Visual cards");
    {
        let f = &mut h.state_mut().panes.format;
        f.open.clear();
        f.open.insert(key.into());
    }
    settle(&mut h);
    h
}

/// Picks `item` in the combo box labelled `combo`.
fn pick(h: &mut Harness<'_, App>, combo: &str, item: &str) {
    h.get_by_role_and_label(Role::ComboBox, combo).click();
    steps(h, 3);
    h.get_by_label(item).click();
    steps(h, 3);
}

/// Types `text` in the text box labelled `name`.
fn type_in(h: &mut Harness<'_, App>, name: &str, text: &str) {
    h.get_by_label(name).focus();
    steps(h, 1);
    h.event(egui::Event::Text(text.into()));
    steps(h, 2);
}

/// Clicks `name` then steps.
fn click(h: &mut Harness<'_, App>, name: &str) {
    h.get_by_label(name).click();
    steps(h, 3);
}

/// OK, then the edit sent; the engine accepted it.
fn ok(h: &mut Harness<'_, App>) -> (&'static str, Value) {
    h.get_by_label("OK").click();
    settle(h);
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
    assert!(h.state().panes.format.cf.is_none(), "OK closes the dialog");
    last_edit(h)
}

/// A table's Cell elements card has Desktop's data bars, icons and web URL, each with its fx
/// button opening its own dialog: data bars' options, icons' layout and style, a web URL's field;
/// each with its preview (web URL: the field's own values).
#[test]
fn the_cell_elements_open_the_data_bars_icons_and_web_url_dialogs() {
    let mut h = table_card("cellElements");
    for (fx, target, controls) in [
        (
            BARS_FX,
            "data_bars",
            &[
                "Show bar only",
                "Minimum type",
                "Maximum type",
                "Positive bar colour",
                "Negative bar colour",
                "Axis colour",
                "Bar direction",
            ][..],
        ),
        (
            ICONS_FX,
            "icon",
            &[
                "Icon layout",
                "Icon alignment",
                "Style",
                "Rule 1 icon",
                "Rule 2 second operator",
            ][..],
        ),
        (URL_FX, "web_url", &["What field should we base this on?"][..]),
    ] {
        assert!(!marked(&h, fx), "{fx}");
        click(&mut h, fx);
        let d = h.state().panes.format.cf.clone().expect("the dialog is open");
        assert_eq!(d.target, target);
        for c in controls {
            assert!(h.query_all_by_label(c).next().is_some(), "{c}");
        }
        if target == "web_url" {
            assert!(h.query_by_label_contains("Preview:").is_none());
        } else {
            h.get_by_label_contains("Preview:");
        }
        click(&mut h, "Cancel");
        assert!(h.state().panes.format.cf.is_none());
    }
}

/// Data bars: Show bar only, a number as the minimum, the negative bar's colour, right to left;
/// OK is one `visual.conditional` the engine accepts, and the fx button is then marked.
#[test]
fn the_data_bars_dialog_applies_its_settings() {
    let mut h = table_card("cellElements");
    click(&mut h, BARS_FX);
    click(&mut h, "Show bar only");
    pick(&mut h, "Minimum type", "Number");
    type_in(&mut h, "Minimum value", "-10");
    h.get_by_label("Negative bar colour").click();
    steps(&mut h, 3);
    click(&mut h, "Negative bar colour #252423");
    pick(&mut h, "Bar direction", "Right to left");
    // the preview: the first sample (-10) a bar in the negative colour, without its text
    h.get_by_label_contains("Preview: -10 91–100 %");
    let (id, params) = ok(&mut h);
    assert_eq!(id, "visual.conditional");
    let positive = params["rule"]["data_bars"]["positive"].clone();
    assert_eq!(
        params,
        json!({"target": "data_bars", "series": "[Total]", "rule": {"data_bars": {
            "positive": positive, "negative": "#252423", "axis": "#000000", "reverse": true,
            "show_bar_only": true, "min": -10.0}}})
    );
    let c = conditional(&h);
    assert_eq!(c[0]["target"], "data_bars", "{c}");
    assert_eq!(c[0]["rule"]["data_bars"]["reverse"], true, "{c}");
    assert!(marked(&h, BARS_FX));
    // opened again, it reads them back
    click(&mut h, BARS_FX);
    let d = h.state().panes.format.cf.clone().expect("open");
    assert!(d.existing && d.bars.bar_only && d.bars.reverse);
    assert_eq!(d.bars.min.as_deref(), Some("-10"));
}

/// Icons: a style (its three rules), the layout and the alignment; OK sends the icons' rules.
#[test]
fn the_icons_dialog_applies_a_style_layout_and_alignment() {
    let mut h = table_card("cellElements");
    click(&mut h, ICONS_FX);
    pick(&mut h, "Style", "Arrows");
    pick(&mut h, "Icon layout", "Right of data");
    pick(&mut h, "Icon alignment", "Middle");
    h.get_by_label_contains("ColoredArrowUp");
    let (_, params) = ok(&mut h);
    assert_eq!(
        params,
        json!({"target": "icon", "series": "[Total]", "rule": {"icons": {
            "field": "[Total]", "position": "after", "align": "middle", "cases": [
                {"op": ">=", "value": 67.0, "icon": "ColoredArrowUp"},
                {"gte": 33.0, "lt": 67.0, "icon": "ColoredArrowRight"},
                {"op": "<", "value": 33.0, "icon": "ColoredArrowDown"}]}}})
    );
    assert_eq!(conditional(&h)[0]["rule"]["icons"]["position"], "after");
    assert!(marked(&h, ICONS_FX));
}

/// Web URL: the field picked is the link of the series' cells.
#[test]
fn the_web_url_dialog_applies_a_field() {
    let mut h = table_card("cellElements");
    click(&mut h, URL_FX);
    pick(&mut h, "What field should we base this on?", "Sales › Category");
    let (_, params) = ok(&mut h);
    assert_eq!(
        params,
        json!({"target": "web_url", "series": "Sales[Category]",
               "rule": {"web_url": {"field": "Sales[Category]"}}})
    );
    assert!(marked(&h, URL_FX));
}

/// Gradient: numbers as the minimum and maximum, empty values as zero, applied to values and
/// totals; the preview shows the lowest sample in the minimum's colour.
#[test]
fn the_gradient_dialog_takes_numbers_blanks_and_totals() {
    let mut h = table_card("values");
    click(&mut h, BG_FX);
    pick(&mut h, "Minimum type", "Number");
    type_in(&mut h, "Minimum value", "10");
    pick(&mut h, "Maximum type", "Number");
    type_in(&mut h, "Maximum value", "90");
    pick(&mut h, "How should we format empty values?", "As zero");
    pick(&mut h, "Apply to", "Values and totals");
    h.get_by_label_contains("Preview: 10 #FFFFFF, 30 ");
    let (_, params) = ok(&mut h);
    let max = params["rule"]["gradient"]["max"]["color"].clone();
    assert_eq!(
        params,
        json!({"target": "background", "series": "[Total]", "apply_to": "values_and_totals",
               "rule": {"gradient": {"field": "[Total]", "min": {"color": "#FFFFFF", "value": 10.0},
                                     "max": {"color": max, "value": 90.0}, "empty": "zero"}}})
    );
    let c = conditional(&h);
    assert_eq!(c[0]["apply_to"], "values_and_totals", "{c}");
    assert_eq!(c[0]["rule"]["gradient"]["empty"], "zero", "{c}");
}

/// Rules: a rule's second condition ("and is less than"), a second rule, Reverse colour order.
#[test]
fn the_rules_take_a_second_condition_and_reverse_their_colours() {
    let mut h = table_card("values");
    click(&mut h, BG_FX);
    pick(&mut h, "Format style", "Rules");
    click(&mut h, "+ New rule");
    click(&mut h, "+ New rule");
    pick(&mut h, "Rule 1 operator", "is greater than or equal to");
    type_in(&mut h, "Rule 1 value", "0");
    pick(&mut h, "Rule 1 second operator", "is less than");
    type_in(&mut h, "Rule 1 second value", "50");
    pick(&mut h, "Rule 2 operator", "is greater than or equal to");
    type_in(&mut h, "Rule 2 value", "50");
    let before: Vec<String> = h
        .state()
        .panes
        .format
        .cf
        .as_ref()
        .expect("open")
        .cases
        .iter()
        .map(|c| c.color.clone())
        .collect();
    assert_ne!(before[0], before[1]);
    click(&mut h, "⇅ Reverse colour order");
    let (_, params) = ok(&mut h);
    assert_eq!(
        params["rule"]["rules"]["cases"],
        json!([{"gte": 0.0, "lt": 50.0, "color": before[1]}, {"op": ">=", "value": 50.0, "color": before[0]}])
    );
    let c = conditional(&h);
    assert_eq!(c[0]["rule"]["rules"]["cases"][0]["lt"], 50.0, "{c}");
}

/// [`open`] with a gauge and a KPI on the first page too.
fn open_gauge_kpi() -> Harness<'static, App> {
    let make = || {
        let mut s = session_with(false);
        let page = s.report.as_ref().unwrap().pages[0].name.clone();
        for (i, (ty, fields)) in [
            (
                "gauge",
                json!([{"role": "Y", "field": "[Total]"}, {"role": "TargetValue", "field": "[Total]"}]),
            ),
            (
                "kpi",
                json!([{"role": "Indicator", "field": "[Total]"}, {"role": "TrendLine", "field": "Sales[Category]"}]),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            s.execute(
                "visual.add",
                &json!({"page": page, "visual_type": ty, "x": 20 + i as i64 * 400, "y": 500,
                        "width": 380, "height": 220, "fields": fields}),
            )
            .unwrap();
        }
        s.dirty = false;
        s.report_dirty = false;
        s
    };
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, make)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1500.0, 900.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().placed.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    h
}

/// Shows only card `key` of the Format visual tab open, so its rows stay in the viewport.
fn only_card(h: &mut Harness<'_, App>, key: &str) {
    {
        let f = &mut h.state_mut().panes.format;
        f.open.clear();
        f.open.insert(key.into());
    }
    settle(h);
}

/// G54: the axes' label and title colours and the legend's colour have their fx; the dialog
/// formats them as one value for the whole visual (no series), its OK is one
/// `visual.conditional` the engine accepts, and the button is marked after.
#[test]
fn the_axes_and_legend_colours_have_their_fx() {
    let mut h = open(false);
    let chart = visual_of(&h, "clusteredColumnChart");
    format_tab(&mut h, &chart, "Visual cards");
    for (card, key, label, target) in [
        (
            "X-axis",
            "categoryAxis",
            "X-axis Colour Conditional formatting",
            "category_axis_color",
        ),
        (
            "X-axis",
            "categoryAxis",
            "X-axis Title colour Conditional formatting",
            "category_title_color",
        ),
        (
            "Y-axis",
            "valueAxis",
            "Y-axis Colour Conditional formatting",
            "value_axis_color",
        ),
        (
            "Y-axis",
            "valueAxis",
            "Y-axis Title colour Conditional formatting",
            "value_title_color",
        ),
        (
            "Legend",
            "legend",
            "Legend Colour Conditional formatting",
            "legend_color",
        ),
    ] {
        only_card(&mut h, key);
        assert!(!marked(&h, label), "{card}: no conditional format yet");
        click(&mut h, label);
        let d = h.state().panes.format.cf.clone().expect("the dialog is open");
        assert_eq!(d.target, target);
        assert!(!d.table, "{target} is one value for the visual");
        pick(&mut h, "Format style", "Rules");
        click(&mut h, "+ New rule");
        type_in(&mut h, "Rule 1 value", "10");
        let (id, params) = ok(&mut h);
        assert_eq!(id, "visual.conditional");
        assert_eq!(params["target"], target, "{params}");
        assert!(params.get("series").is_none(), "{params}");
        assert!(
            conditional(&h)
                .as_array()
                .is_some_and(|a| a.iter().any(|i| i["target"] == target)),
            "{}",
            conditional(&h)
        );
        assert!(marked(&h, label), "{label} is marked");
    }
}

/// G54: a gauge's target, target label and callout colours and a KPI's indicator, goal,
/// distance and status colours have their fx.
#[test]
fn the_gauge_and_kpi_colours_have_their_fx() {
    let mut h = open_gauge_kpi();
    let gauge = visual_of(&h, "gauge");
    format_tab(&mut h, &gauge, "Visual cards");
    for (key, label, target) in [
        (
            "calloutValue",
            "Callout value Colour Conditional formatting",
            "callout_color",
        ),
        (
            "target",
            "Target Colour Conditional formatting",
            "target_label_color",
        ),
    ] {
        only_card(&mut h, key);
        click(&mut h, label);
        let d = h.state().panes.format.cf.clone().expect("the dialog is open");
        assert_eq!(d.target, target);
        let (_, params) = ok(&mut h);
        assert_eq!(params["target"], target, "{params}");
    }
    let kpi = visual_of(&h, "kpi");
    h.state_mut().cmd.visual = Some(kpi);
    settle(&mut h);
    for (key, label, target) in [
        (
            "indicator",
            "Indicator Colour Conditional formatting",
            "indicator_color",
        ),
        ("goals", "Goals Goal colour Conditional formatting", "goal_color"),
        (
            "goals",
            "Goals Distance colour Conditional formatting",
            "distance_color",
        ),
        (
            "status",
            "Colour coding Good colour Conditional formatting",
            "good_color",
        ),
        (
            "status",
            "Colour coding Bad colour Conditional formatting",
            "bad_color",
        ),
    ] {
        only_card(&mut h, key);
        click(&mut h, label);
        let d = h.state().panes.format.cf.clone().expect("the dialog is open");
        assert_eq!(d.target, target);
        let (_, params) = ok(&mut h);
        assert_eq!(params["target"], target, "{params}");
        assert!(marked(&h, label), "{label} is marked");
    }
}

/// G54: a rule's numbers can be percents of the range (Number / Percent beside them): OK sends
/// `percent: true`, the dialog reads it back, a percent above 100 is refused.
#[test]
fn the_rules_take_percent_bounds() {
    let mut h = table_card("values");
    click(&mut h, BG_FX);
    pick(&mut h, "Format style", "Rules");
    click(&mut h, "+ New rule");
    pick(&mut h, "Rule 1 operator", "is greater than or equal to");
    type_in(&mut h, "Rule 1 value", "150");
    pick(&mut h, "Rule 1 type", "Percent");
    h.get_by_label("OK").click();
    settle(&mut h);
    assert!(
        h.state().panes.format.cf.is_some(),
        "150 % is refused, the dialog stays"
    );
    h.state_mut().panes.format.cf.as_mut().expect("open").cases[0].value = "50".into();
    steps(&mut h, 2);
    let (_, params) = ok(&mut h);
    assert_eq!(
        params["rule"]["rules"]["cases"],
        json!([{"op": ">=", "value": 50.0, "percent": true, "color": params["rule"]["rules"]["cases"][0]["color"]}]),
        "{params}"
    );
    let c = conditional(&h);
    assert_eq!(c[0]["rule"]["rules"]["cases"][0]["percent"], true, "{c}");
    // the dialog opens on it again with Percent picked
    click(&mut h, BG_FX);
    let d = h.state().panes.format.cf.clone().expect("open");
    assert!(d.cases.first().is_some_and(|c| c.percent), "{:?}", d.cases);
}

/// G55: a navigator's Format visual tab has the buttons' Shape, Glow and Shadow cards (an action
/// button does not), and each control writes the navigator's object with `visual.format`.
#[test]
fn a_navigators_shape_glow_and_shadow_cards_format_it() {
    for ty in ["pageNavigator", "bookmarkNavigator"] {
        let keys: Vec<&str> = format_pane::visual_cards(ty).iter().map(|c| c.key).collect();
        for k in ["shape", "glow", "shadow"] {
            assert!(keys.contains(&k), "{ty}: {keys:?}");
        }
    }
    let button: Vec<&str> = format_pane::visual_cards("actionButton")
        .iter()
        .map(|c| c.key)
        .collect();
    // an action button has them too (its glow and shadow drawn by state, G55)
    for k in ["shape", "glow", "shadow"] {
        assert!(button.contains(&k), "{button:?}");
    }
    let make = || {
        let mut s = session_with(false);
        let page = s.report.as_ref().unwrap().pages[0].name.clone();
        s.execute(
            "button.add",
            &json!({"page": page, "kind": "page_navigator", "x": 20, "y": 500, "width": 400, "height": 60}),
        )
        .unwrap();
        s.dirty = false;
        s.report_dirty = false;
        s
    };
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, make)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1500.0, 900.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().placed.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    let nav = visual_of(&h, "pageNavigator");
    format_tab(&mut h, &nav, "Visual cards");
    only_card(&mut h, "shape");
    pick(&mut h, "Shape Shape", "Rounded rectangle");
    assert_eq!(
        last_edit(&h),
        (
            "visual.format",
            json!({"objects": [{"object": "shape", "property": "tileShape", "value": "rectangleRounded"}]})
        )
    );
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
    only_card(&mut h, "shadow");
    h.get_by_label("Shape shadow Colour");
    only_card(&mut h, "glow");
    h.get_by_label("Shape glow Colour");
    let n = snap_visual(&h, &nav)["navigator"].clone();
    assert_eq!(n["shape"]["tile"], "rectangleRounded", "{n}");
}

/// The pane's name ("Mettre en forme le visuel" in French) stays left of the sub-tab icons in the
/// narrowest Build pane: it is cut short instead of running under them.
#[test]
fn the_pane_name_keeps_clear_of_the_tab_icons_in_a_narrow_french_pane() {
    let mut app =
        App::new("fr-FR", engine_shell(Caps::DESKTOP, move || session_with(false))).with_sync_render();
    app.prefs.panes.build_width = *crate::prefs::PANE_WIDTHS.start();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1500.0, 900.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().placed.is_some() {
            break;
        }
    }
    let chart = visual_of(&h, "clusteredColumnChart");
    h.state_mut().cmd.visual = Some(chart);
    steps(&mut h, 3);
    h.get_by_role_and_label(Role::Button, "Mettre en forme le visuel")
        .click();
    steps(&mut h, 3);
    assert_eq!(h.state().panes.format.tab, Tab::Format);
    let icons_left = ["Générer un visuel", "Mettre en forme le visuel", "Analyse"]
        .iter()
        .map(|l| h.get_by_role_and_label(Role::Button, l).rect().min.x)
        .fold(f32::INFINITY, f32::min);
    let name = h
        .get_all_by_label("Mettre en forme le visuel")
        .find(|n| n.accesskit_node().role() != Role::Button)
        .expect("the pane's name is drawn");
    assert!(
        name.rect().max.x <= icons_left + 0.5,
        "the name ends at {} but the icons start at {icons_left}",
        name.rect().max.x
    );
}
