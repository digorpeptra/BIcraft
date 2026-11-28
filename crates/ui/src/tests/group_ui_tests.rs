//! New group / Edit groups through the accessibility tree (egui_kittest, no GPU): the Data pane's
//! column menu opening the Groups dialog and a list group made with Group and run as
//! `column.group`, bins from the Data view's Column tools and from the Model view's Table tab,
//! and Edit groups in French starting from the group read back.

use std::path::Path;

use bicraft_engine::Session;
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, right_click, steps};
use super::harness;
use crate::App;
use crate::app::Tab;
use crate::group_dialog::Kind;
use crate::model_view::Sel;
use crate::registry::Tab as RTab;
use crate::shell::Caps;

/// Sales(Product, Amount) with [Total], and a report of one page.
fn session() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Product", "Amount"}, {{"Bike", 100}, {"Car", 1000}, {"Ball", 5}, {"Kite", 42}})"#;
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
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
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap();
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn app(lang: &str) -> Harness<'static, App> {
    let mut app = App::new(lang, engine_shell(Caps::DESKTOP, session)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1400.0, 900.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    h
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
    steps(h, 3);
}

fn type_into(h: &mut Harness<'_, App>, label: &str, text: &str) {
    h.get_by(|n| n.role() == Role::TextInput && n.label().as_deref() == Some(label))
        .click();
    steps(h, 2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by(|n| n.role() == Role::TextInput && n.label().as_deref() == Some(label))
        .type_text(text);
    steps(h, 2);
}

fn last_sent(h: &Harness<'_, App>, id: &str) -> Option<Value> {
    h.state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == id)
        .map(|c| c.params.clone())
}

fn columns(h: &Harness<'_, App>) -> Vec<String> {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| {
            l.snap
                .pointer("/model/tables/0/columns")
                .and_then(Value::as_array)
                .cloned()
        })
        .unwrap_or_default()
        .iter()
        .filter_map(|c| c.get("name").and_then(Value::as_str).map(str::to_string))
        .collect()
}

/// Steps until the dialog has the column's values.
fn values_read(h: &mut Harness<'_, App>) {
    for _ in 0..100 {
        h.step();
        if h.state().cmd.group.as_ref().is_some_and(|d| d.values.is_some()) {
            steps(h, 2);
            return;
        }
    }
    panic!("the Groups dialog never got the column's values");
}

#[test]
fn the_data_pane_menu_makes_a_list_group() {
    let mut h = app("en-US");
    // unroll Sales in the Data pane, then the right-click menu of Product
    click(&mut h, "Sales");
    let at = h.get_all_by_label("Product").next().unwrap().rect().center();
    right_click(&mut h, at);
    steps(&mut h, 2);
    click(&mut h, "New group");
    let d = h.state().cmd.group.clone().expect("the Groups dialog is open");
    assert_eq!(
        (d.kind, d.column.as_str(), d.name.as_str()),
        (Kind::List, "Product", "Product (groups)")
    );
    values_read(&mut h);
    h.get_by_label("Ungrouped values");
    // OK without a group says why and sends nothing
    click(&mut h, "OK");
    h.get_by_label_contains("Make at least one group");
    assert!(last_sent(&h, "column.group").is_none());
    click(&mut h, "Bike");
    click(&mut h, "Car");
    click(&mut h, "Group ›");
    // the new group, named after its members, selected: its name is edited
    assert_eq!(
        h.state().cmd.group.as_ref().unwrap().groups,
        vec![("Bike & Car".to_string(), vec![json!("Bike"), json!("Car")])]
    );
    type_into(&mut h, "Group name", "Wheels");
    click(&mut h, "OK");
    steps(&mut h, 6);
    let p = last_sent(&h, "column.group").expect("column.group was not sent");
    assert_eq!(
        p,
        json!({"table": "Sales", "column": "Product", "name": "Product (groups)", "kind": "list",
               "other": true, "groups": [{"name": "Wheels", "values": ["Bike", "Car"]}]})
    );
    assert!(h.state().cmd.group.is_none());
    assert!(
        columns(&h).contains(&"Product (groups)".to_string()),
        "{:?}",
        columns(&h)
    );
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
}

#[test]
fn column_tools_and_the_model_views_table_tab_make_bins() {
    let mut h = app("en-US");
    // the Data view's Column tools on Amount
    h.state_mut().show_tab(Tab::Data);
    steps(&mut h, 6);
    let ctx = h.ctx.clone();
    crate::data_view::select_column(h.state_mut(), &ctx, "Amount");
    steps(&mut h, 3);
    click(&mut h, "Data groups");
    let d = h.state().cmd.group.clone().expect("the Groups dialog is open");
    assert_eq!((d.kind, d.name.as_str()), (Kind::Bin, "Amount (bins)"));
    values_read(&mut h);
    // the values' minimum and maximum are shown
    h.get_by_label("1000");
    type_into(&mut h, "Bin size", "250");
    click(&mut h, "OK");
    steps(&mut h, 6);
    let p = last_sent(&h, "column.group").expect("column.group was not sent");
    assert_eq!(
        p,
        json!({"table": "Sales", "column": "Amount", "name": "Amount (bins)", "kind": "bin",
               "unit": "number", "bin_size": 250.0})
    );
    assert!(columns(&h).contains(&"Amount (bins)".to_string()));
    // the Model view's Table tab, a column selected: Data groups, by number of bins
    h.state_mut().show_tab(Tab::Model);
    h.state_mut().model_view.sel = Some(Sel::Column {
        table: "Sales".into(),
        name: "Amount".into(),
    });
    h.state_mut().ribbon_tab = RTab::Table;
    steps(&mut h, 3);
    click(&mut h, "Data groups");
    values_read(&mut h);
    h.state_mut().cmd.group.as_mut().unwrap().name = "Amount bands".into();
    click(&mut h, "Number of bins");
    type_into(&mut h, "Bin count", "4");
    click(&mut h, "OK");
    steps(&mut h, 6);
    let p = last_sent(&h, "column.group").unwrap();
    assert_eq!(p["bin_count"], json!(4.0));
    assert_eq!(p["name"], "Amount bands");
    assert!(columns(&h).contains(&"Amount bands".to_string()));
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
}

#[test]
fn edit_groups_starts_from_the_group_and_speaks_french() {
    let mut h = app("fr-FR");
    h.state_mut().run(
        crate::actions::Call {
            id: "column.group",
            params: json!({"table": "Sales", "column": "Product", "groups": [{"name": "Roues", "values": ["Bike", "Car"]}]}),
        },
        crate::app::Then::Nothing,
    );
    steps(&mut h, 6);
    // the palette's entry (or a menu) on the group column: Edit groups
    crate::ribbon::invoke(
        h.state_mut(),
        "column.group",
        json!({"table": "Sales", "column": "Product (groups)"}),
    );
    steps(&mut h, 3);
    let d = h.state().cmd.group.clone().expect("the Groups dialog is open");
    assert!(d.edit);
    assert_eq!(d.column, "Product");
    assert_eq!(
        d.groups,
        vec![("Roues".to_string(), vec![json!("Bike"), json!("Car")])]
    );
    values_read(&mut h);
    h.get_by_label("Modifier les groupes");
    h.get_by_label("Valeurs non groupées");
    h.get_by_label("Inclure le groupe Autre");
    // Ball joins Roues: the group selected, then Grouper
    click(&mut h, "Roues");
    click(&mut h, "Ball");
    click(&mut h, "Grouper ›");
    click(&mut h, "Inclure le groupe Autre");
    click(&mut h, "OK");
    steps(&mut h, 6);
    let p = last_sent(&h, "column.group").unwrap();
    assert_eq!(p["replace"], true);
    assert_eq!(p["other"], false);
    assert_eq!(
        p["groups"],
        json!([{"name": "Roues", "values": ["Bike", "Car", "Ball"]}])
    );
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
    // a menu click without a column says what to do
    crate::ribbon::invoke(h.state_mut(), "column.group", json!({}));
    steps(&mut h, 2);
    assert!(h.state().cmd.group.is_none());
    assert!(h.state().error.as_deref().is_some_and(|e| e.contains("colonne")));
}

/// [`session`] with a column chart of [Total] by Product on its page.
fn chart_session() -> Session {
    let mut s = session();
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute(
        "visual.add",
        &json!({"page": page, "visual_type": "clusteredColumnChart", "x": 20, "y": 20, "width": 600, "height": 400,
                "fields": [{"role": "Category", "field": "Sales[Product]"}, {"role": "Y", "field": "[Total]"}]}),
    )
    .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

/// The screen centre and the point of each bar the chart's hit map selects, in hit order.
fn bars(h: &Harness<'_, App>) -> Vec<(egui::Pos2, Value)> {
    let app = h.state();
    let (Some(placed), Some((_, Ok(img)))) = (app.placed, app.shown.as_ref()) else {
        return Vec::new();
    };
    img.hits
        .iter()
        .filter_map(|x| {
            let a = x.action_for(false)?;
            (a["action"] == "select" && !a["point"].is_null()).then(|| {
                let (bx, by, w, hh) = x.shape.bounds();
                let pos = egui::pos2(
                    placed.rect.min.x + (bx + w / 2.0) * placed.zoom,
                    placed.rect.min.y + (by + hh / 2.0) * placed.zoom,
                );
                (pos, a["point"].clone())
            })
        })
        .collect()
}

/// Group data (G75): two bars Ctrl+clicked, then the bar's right-click menu › Group data makes a
/// list group of exactly their categories through `visual.group_points`.
#[test]
fn group_data_on_selected_bars_makes_a_group_of_their_values() {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, chart_session)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1400.0, 900.0));
    for _ in 0..200 {
        h.step();
        if bars(&h).len() >= 4 {
            break;
        }
    }
    let b = bars(&h);
    assert!(b.len() >= 4, "the chart's bars: {b:?}");
    let picked: Vec<(egui::Pos2, Value)> = b.into_iter().take(2).collect();
    let mut want: Vec<Value> = picked.iter().map(|(_, p)| p[0]["v"].clone()).collect();
    // Ctrl+click on each bar: the reader's selection of the two
    for (pos, _) in &picked {
        let placed = h.state().placed.unwrap();
        let (x, y) = (
            (pos.x - placed.rect.min.x) / placed.zoom,
            (pos.y - placed.rect.min.y) / placed.zoom,
        );
        h.state_mut().click_report(x, y, true, *pos);
        steps(&mut h, 3);
    }
    // the right-click menu of the second bar
    let at = picked[1].0;
    h.hover_at(at);
    h.step();
    right_click(&mut h, at);
    steps(&mut h, 2);
    click(&mut h, "Group data");
    steps(&mut h, 6);
    let p = last_sent(&h, "visual.group_points").unwrap_or_else(|| panic!("visual.group_points was not sent: state {:?} status {:?} error {:?} visual {:?} point {:?} sent {:?}", crate::ribbon::state(h.state(), "visual.group_points"), h.state().status, h.state().error, h.state().cmd.visual, h.state().cmd.point, h.state().sent.iter().rev().take(5).map(|c| c.id).collect::<Vec<_>>()));
    assert!(p["page"].is_string() && p["visual"].is_string(), "{p}");
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
    assert!(
        columns(&h).contains(&"Product (groups)".to_string()),
        "{:?}",
        columns(&h)
    );
    // the group read back holds exactly the two bars' values
    let def = h
        .state()
        .loaded
        .as_ref()
        .and_then(|l| {
            l.snap
                .pointer("/model/tables/0/columns")?
                .as_array()?
                .iter()
                .find(|c| c["name"] == "Product (groups)")
                .map(|c| c["group"].clone())
        })
        .unwrap();
    let mut got: Vec<Value> = def["groups"][0]["values"].as_array().unwrap().clone();
    let key = |v: &Value| v.as_str().unwrap_or_default().to_string();
    got.sort_by_key(key);
    want.sort_by_key(key);
    assert_eq!(got, want, "{def}");
    assert_eq!(def["groups"].as_array().unwrap().len(), 1, "{def}");
}
