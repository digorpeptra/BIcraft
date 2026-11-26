//! The Report view's Data and Build panes on an engine session (`panes.rs`): a field dragged to
//! a well, onto a visual on the canvas, reordered, removed by × and by dragging it out; the box
//! of a field; the type picker; undo; a reader's read-only panes; the same frames in both shells.

use std::path::Path;

use bicraft_engine::Session;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::{fixed, frame, harness, loaded, shell_app, shop_snapshot};
use crate::App;
use crate::ribbon::VisualSel;
use crate::shell::Caps;

/// Sales(Category, Amount) with [Total]; one page with a slicer (Category), a card ([Total]) and
/// a column chart (Category, [Total]).
fn shop_session() -> Session {
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
    ] {
        s.execute(id, &p).unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    for (ty, rect, fields) in [
        (
            "slicer",
            [20, 20, 220, 260],
            json!([{"role": "Values", "field": "Sales[Category]"}]),
        ),
        (
            "card",
            [260, 20, 240, 120],
            json!([{"role": "Values", "field": "[Total]"}]),
        ),
        (
            "clusteredColumnChart",
            [260, 160, 600, 400],
            json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]),
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

/// The window on a fresh shop session, the report shown, a reader's when `reader`.
fn open(reader: bool) -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, shop_session)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1400.0, 860.0));
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
pub(super) fn visual_named(h: &Harness<'_, App>, ty: &str) -> VisualSel {
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

pub(super) fn select(h: &mut Harness<'_, App>, sel: &VisualSel) {
    h.state_mut().cmd.visual = Some(sel.clone());
    steps(h, 3);
}

/// (role, field text) of every field of visual `sel`, from the latest snapshot.
pub(super) fn fields(h: &Harness<'_, App>, sel: &VisualSel) -> Vec<(String, String)> {
    let snap = h.state().loaded.as_ref().unwrap().snap.clone();
    snap["report"]["pages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|p| p["visuals"].as_array().cloned().unwrap_or_default())
        .find(|v| v["name"] == sel.visual.as_str())
        .map(|v| {
            v["fields"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| {
                    (
                        f["role"].as_str().unwrap().to_string(),
                        f["text"].as_str().unwrap().to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(r, f)| (r.to_string(), f.to_string())).collect()
}

/// A pointer drag from `from` to `to`, as a person does it: press, move in steps, release (the
/// pointer stays).
pub(super) fn drag(h: &mut Harness<'_, App>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.step();
    h.event(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    for i in 1..=6 {
        let t = i as f32 / 6.0;
        h.hover_at(from + (to - from) * t);
        h.step();
    }
    h.event(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    steps(h, 4);
}

pub(super) fn center(h: &Harness<'_, App>, label: &str) -> Pos2 {
    h.get_by_label(label).rect().center()
}

/// The screen centre of visual `sel` on the page shown.
pub(super) fn on_canvas(h: &Harness<'_, App>, sel: &VisualSel) -> Pos2 {
    let snap = h.state().loaded.as_ref().unwrap().snap.clone();
    let v = snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == sel.visual.as_str())
        .unwrap()
        .clone();
    let placed = h.state().placed.unwrap();
    let n = |k: &str| v[k].as_f64().unwrap() as f32;
    placed.rect.min + Vec2::new(n("x") + n("width") / 2.0, n("y") + n("height") / 2.0) * placed.zoom
}

/// The accessible label of the node of `role` whose label contains every part of `parts`.
pub(super) fn label_with(h: &Harness<'_, App>, role: egui::accesskit::Role, parts: &[&str]) -> String {
    h.get_all_by_role(role)
        .filter_map(|n| n.accesskit_node().label())
        .find(|l| parts.iter().all(|p| l.contains(p)))
        .unwrap_or_else(|| panic!("no {role:?} labelled with {parts:?}"))
}

/// The well item of the field whose label contains `field`, in well `well`.
pub(super) fn item(h: &Harness<'_, App>, field: &str, well: &str) -> String {
    let suffix = format!(" in {well}");
    label_with(h, egui::accesskit::Role::Button, &[field, &suffix])
}

pub(super) fn expand_sales(h: &mut Harness<'_, App>) {
    if h.query_by_label("Amount").is_none() {
        h.get_by_label("Sales").click();
        steps(h, 3);
    }
    assert!(h.query_by_label("Amount").is_some(), "Sales lists its columns");
}

#[test]
fn drag_and_drop_edits_the_selected_visual() {
    let mut h = open(false);
    let chart = visual_named(&h, "clusteredColumnChart");
    let slicer = visual_named(&h, "slicer");
    select(&mut h, &chart);
    expand_sales(&mut h);
    // the wells of a column chart, by their captions
    for well in ["X-axis", "Y-axis", "Legend", "Small multiples", "Tooltips"] {
        h.get_by_label(well);
    }
    h.get_by_label("Total in Y-axis");

    // a number column onto Tooltips: bound there, summarized
    let (from, to) = (center(&h, "Amount"), center(&h, "Tooltips"));
    drag(&mut h, from, to);
    assert_eq!(
        fields(&h, &chart),
        pairs(&[
            ("Category", "Sales[Category]"),
            ("Y", "[Total]"),
            ("Tooltips", "Sum(Sales[Amount])"),
        ])
    );
    assert_eq!(h.state().sent.last().map(|c| c.id), Some("visual.bind"));

    // a well's field onto another well: moved (one command)
    let (from, to) = (center(&h, &item(&h, "Amount", "Tooltips")), center(&h, "Y-axis"));
    drag(&mut h, from, to);
    assert_eq!(h.state().sent.last().map(|c| c.id), Some("visual.move_field"));
    assert_eq!(
        fields(&h, &chart),
        pairs(&[
            ("Category", "Sales[Category]"),
            ("Y", "Sum(Sales[Amount])"),
            ("Y", "[Total]"),
        ]),
        "dropped on the caption: first in the well"
    );

    // reorder inside the well: [Total] above
    let total = h.get_by_label("Total in Y-axis").rect();
    let sum = h.get_by_label(&item(&h, "Amount", "Y-axis")).rect();
    drag(&mut h, total.center(), Pos2::new(sum.center().x, sum.min.y + 2.0));
    assert_eq!(
        fields(&h, &chart)[1..],
        pairs(&[("Y", "[Total]"), ("Y", "Sum(Sales[Amount])")])[..]
    );

    // the aggregation of a column: Average
    let agg = label_with(&h, egui::accesskit::Role::ComboBox, &["Summarize", "Amount"]);
    h.get_by_label(&agg).click();
    steps(&mut h, 3);
    h.get_by_label("Average").click();
    steps(&mut h, 4);
    assert_eq!(fields(&h, &chart)[2], ("Y".into(), "Avg(Sales[Amount])".into()));

    // × removes it
    let label = label_with(&h, egui::accesskit::Role::Button, &["Remove", "Amount"]);
    h.get_by_label(&label).click();
    steps(&mut h, 4);
    assert_eq!(h.state().sent.last().map(|c| c.id), Some("visual.unbind"));
    assert_eq!(fields(&h, &chart).len(), 2);

    // a well's field dragged out (onto the Data pane): removed
    let (from, to) = (center(&h, "Total in Y-axis"), center(&h, "Amount"));
    drag(&mut h, from, to);
    assert_eq!(fields(&h, &chart), pairs(&[("Category", "Sales[Category]")]));

    // a field onto another visual on the canvas: bound to its default role, and selected
    let (from, to) = (center(&h, "Amount"), on_canvas(&h, &slicer));
    drag(&mut h, from, to);
    assert_eq!(
        fields(&h, &slicer),
        pairs(&[("Values", "Sales[Category]"), ("Values", "Sales[Amount]")])
    );
    assert_eq!(h.state().cmd.visual.as_ref(), Some(&slicer));

    // undo: one step back, the slicer as it was
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    steps(&mut h, 4);
    assert_eq!(fields(&h, &slicer), pairs(&[("Values", "Sales[Category]")]));
}

#[test]
fn the_box_of_a_field_adds_it_to_the_default_role_and_removes_it() {
    let mut h = open(false);
    let chart = visual_named(&h, "clusteredColumnChart");
    select(&mut h, &chart);
    expand_sales(&mut h);
    // bound: checked
    let node = h.get_by_label("Category");
    assert_eq!(
        node.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    h.get_by_label("Category").click();
    steps(&mut h, 4);
    assert_eq!(fields(&h, &chart), pairs(&[("Y", "[Total]")]));
    h.get_by_label("Category").click();
    steps(&mut h, 4);
    assert_eq!(
        fields(&h, &chart),
        pairs(&[("Y", "[Total]"), ("Category", "Sales[Category]")])
    );
    // a number column: summarized into the values
    h.get_by_label("Amount").click();
    steps(&mut h, 4);
    assert!(fields(&h, &chart).contains(&("Y".into(), "Sum(Sales[Amount])".into())));
}

#[test]
fn the_type_picker_changes_the_type_and_undo_restores_it() {
    let mut h = open(false);
    let chart = visual_named(&h, "clusteredColumnChart");
    select(&mut h, &chart);
    // the gallery's cell of the type (Desktop's grid of icons)
    let shown = "Matrix";
    h.get_by_label(shown).click();
    steps(&mut h, 4);
    let ty = |h: &Harness<'_, App>| {
        h.state().loaded.as_ref().unwrap().snap["report"]["pages"][0]["visuals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == chart.visual.as_str())
            .unwrap()["type"]
            .clone()
    };
    assert_eq!(ty(&h), "pivotTable");
    assert_eq!(
        fields(&h, &chart),
        pairs(&[("Rows", "Sales[Category]"), ("Values", "[Total]")])
    );
    h.get_by_label("Rows");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    steps(&mut h, 4);
    assert_eq!(ty(&h), "clusteredColumnChart");
}

/// The label the gallery gives a type: BIcraft's name for the common ones, the catalog's
/// display name otherwise.
fn gallery_label(t: &Value) -> String {
    let name = t["name"].as_str().unwrap_or("");
    crate::panes::type_label(name, crate::i18n::Lang::En)
        .map(str::to_string)
        .or_else(|| {
            t["display_name"]
                .as_str()
                .filter(|d| !d.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| name.to_string())
}

/// The Visualizations pane's gallery, as Desktop draws it: a cell for every built-in type that
/// takes fields, the selected visual's type marked, and a click on a cell adding that visual when
/// nothing is selected.
#[test]
fn the_gallery_lists_every_built_in_type_and_adds_a_visual() {
    let mut h = open(false);
    let types = h.state().loaded.as_ref().unwrap().snap["report"]["visual_types"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(types.len() >= 20, "the catalog lists the built-in types");
    for t in &types {
        let label = gallery_label(t);
        assert!(h.query_by_label(&label).is_some(), "no gallery cell for {label}");
    }
    // nothing selected: a cell makes that visual, at Desktop's default size
    let before = h.state().loaded.as_ref().unwrap().snap["report"]["pages"][0]["visuals"]
        .as_array()
        .map_or(0, Vec::len);
    h.get_by_label("Card").click();
    steps(&mut h, 5);
    assert_eq!(h.state().sent.last().map(|c| c.id), Some("visual.add"));
    let page = &h.state().loaded.as_ref().unwrap().snap["report"]["pages"][0];
    let visuals = page["visuals"].as_array().cloned().unwrap_or_default();
    assert_eq!(visuals.len(), before + 1);
    let made = visuals.last().unwrap();
    assert_eq!(made["type"], "card");
    assert_eq!(made["width"].as_f64(), Some(400.0));
    assert_eq!(made["height"].as_f64(), Some(300.0));
    assert!(h.state().cmd.visual.is_some(), "the new visual is the selection");
}

/// The gallery's cells keep a fixed size a little larger than their icon, tight as Desktop's
/// grid, whatever the pane's width: a wider pane fits more cells a row, not wider cells.
#[test]
fn the_gallery_cells_are_tight() {
    let h = open(false);
    let a = h.get_by_label("Card").rect();
    assert!(a.width() <= 32.0 && a.height() <= 30.0, "cell {a:?}");
    // cells of a row are whole cells apart (cell and gap), not a fifth of the pane each
    let b = h.get_by_label("Matrix").rect();
    let c = h.get_by_label("Table").rect();
    if (b.center().y - c.center().y).abs() < 1.0 {
        let dx = (b.center().x - c.center().x).abs();
        assert!((dx / 32.0 - (dx / 32.0).round()).abs() < 0.05, "{b:?} {c:?}");
    }
}

/// Every type of the gallery shows the data wells the model gives it: one caption per role of the
/// type, and the note when the type takes no field (the per-visual options the pane offers).
#[test]
fn every_visual_type_shows_its_data_wells() {
    let mut h = open(false);
    let chart = visual_named(&h, "clusteredColumnChart");
    select(&mut h, &chart);
    let types = h.state().loaded.as_ref().unwrap().snap["report"]["visual_types"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(types.len() >= 20);
    for t in &types {
        let name = t["name"].as_str().unwrap_or("").to_string();
        h.state_mut().run(
            crate::actions::Call {
                id: "visual.set_type",
                params: json!({"page": chart.page, "visual": chart.visual, "visual_type": name}),
            },
            crate::app::Then::Nothing,
        );
        steps(&mut h, 4);
        let snap = h.state().loaded.as_ref().unwrap().snap.clone();
        let Some(v) = snap["report"]["pages"][0]["visuals"]
            .as_array()
            .and_then(|a| a.iter().find(|v| v["name"] == chart.visual.as_str()))
            .cloned()
        else {
            panic!("{name}: the visual is gone");
        };
        assert_eq!(v["type"], name, "the gallery switched the type");
        let roles: Vec<String> = v["roles"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|r| r["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if roles.is_empty() {
            assert!(
                h.query_by_label("This visual takes no fields.").is_some(),
                "{name}: the pane says a type with no role takes no field"
            );
            continue;
        }
        for role in &roles {
            let cap = crate::panes::caption(&name, role, crate::i18n::Lang::En);
            assert!(
                h.query_all_by_label(&cap).count() >= 1,
                "{name}: no `{cap}` well for role `{role}`"
            );
        }
    }
}

#[test]
fn the_panes_collapse_and_keep_their_state() {
    let mut h = open(false);
    h.get_by_label("Collapse the Data pane").click();
    steps(&mut h, 3);
    assert!(!h.state().prefs.panes.data);
    assert!(h.query_by_label("Sales").is_none());
    h.get_by_label("Collapse the Visualizations pane").click();
    steps(&mut h, 3);
    assert!(!h.state().prefs.panes.build);
    // both closed panes share one rail at the window's edge, one icon each (not one strip each)
    let (data, build) = (
        h.get_by_label("Open the Data pane").rect(),
        h.get_by_label("Open the Visualizations pane").rect(),
    );
    assert!((data.left() - build.left()).abs() <= 1.0, "{data:?} {build:?}");
    assert!(build.top() > data.top(), "{data:?} {build:?}");
    h.get_by_label("Open the Data pane").click();
    steps(&mut h, 3);
    assert!(h.state().prefs.panes.data);
    h.get_by_label("Sales");
    let kept = crate::prefs::Prefs::from_json(&h.state().prefs.to_json());
    assert!(kept.panes.data && !kept.panes.build);
}

#[test]
fn a_reader_sees_read_only_panes() {
    let mut h = open(true);
    let chart = visual_named(&h, "clusteredColumnChart");
    select(&mut h, &chart);
    expand_sales(&mut h);
    let sent = h.state().sent.len();
    // the box is disabled, × absent, the type picker greyed
    assert!(h.get_by_label("Category").accesskit_node().is_disabled());
    assert!(h.get_all_by_role(egui::accesskit::Role::Button).all(|n| {
        !n.accesskit_node()
            .label()
            .is_some_and(|l| l.starts_with("Remove"))
    }));
    h.get_by_label("Category").click();
    steps(&mut h, 3);
    // dragging does nothing
    let (from, to) = (center(&h, "Amount"), center(&h, "Tooltips"));
    drag(&mut h, from, to);
    let (from, to) = (center(&h, "Total in Y-axis"), center(&h, "Amount"));
    drag(&mut h, from, to);
    assert_eq!(h.state().sent.len(), sent, "a reader sends no command");
    assert_eq!(
        fields(&h, &chart),
        pairs(&[("Category", "Sales[Category]"), ("Y", "[Total]")])
    );
}

/// The panes draw the same frames in the desktop and the browser shells: nothing selected, then
/// a chart selected, light and dark.
#[test]
fn the_panes_draw_the_same_frames_in_both_shells() {
    let snap = shop_snapshot();
    let pick = |ty: &str| -> VisualSel {
        let page = &snap.snap["report"]["pages"][0];
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
    };
    for dark in [false, true] {
        let mut desktop = shell_app(Caps::DESKTOP, &snap, "en-US");
        let mut browser = shell_app(Caps::BROWSER, &snap, "en-US");
        for sel in [None, Some(pick("clusteredColumnChart")), Some(pick("slicer"))] {
            for h in [&mut desktop, &mut browser] {
                if dark {
                    h.state_mut().set_theme(crate::theme::Brightness::Dark);
                }
                h.state_mut().cmd.visual = sel.clone();
                h.state_mut().panes.open.insert("t:Sales".into());
                steps(h, 3);
            }
            assert!(
                desktop.query_by_label("Total").is_some(),
                "the Data pane lists Sales"
            );
            let (a, b) = (frame(&desktop), frame(&browser));
            assert!(a.0 == b.0, "dark {dark} {sel:?}: the meshes differ");
            assert!(a.1 == b.1, "dark {dark} {sel:?}: the font atlases differ");
        }
    }
}

/// Damaged snapshots (no roles, odd fields, missing names) never break the panes.
#[test]
fn damaged_snapshots_never_break_the_panes() {
    for snap in [
        json!({"report": {"pages": [{"name": "p", "visuals": [
            {"name": "v", "type": "x", "fields": [{"role": 3}, {"kind": "other"}, {"role": "R", "text": 1}],
             "roles": [{"name": null}, {"name": "R", "kind": 7, "max": -1}]}]}]},
               "model": {"tables": [{"name": "T", "columns": [{"name": 1}, {}], "measures": "x",
                                      "hierarchy_levels": [{"name": "H", "levels": [{"column": 3}]}]}]}}),
        json!({"report": {"pages": [{"name": "p", "visuals": [{"name": "v"}]}]}, "model": null}),
        json!({"report": {}, "model": {"tables": 4}}),
    ] {
        let mut app = App::new("en-US", fixed(Caps::DESKTOP, None));
        app.loaded = Some(loaded(snap));
        app.cmd.visual = Some(VisualSel {
            page: "p".into(),
            visual: "v".into(),
        });
        app.panes.open.insert("t:T".into());
        app.panes.show_hidden = true;
        let mut h = harness(app, (1200.0, 700.0));
        steps(&mut h, 4);
        h.state_mut().panes.search = "h".into();
        steps(&mut h, 2);
    }
    let _: Option<Value> = None;
}

/// A table of the Data pane unrolls on a real pointer click (press and release on its row, as a
/// mouse does), and rolls up on a second one.
#[test]
fn a_mouse_click_unrolls_a_table_of_the_data_pane() {
    let mut h = open(false);
    assert!(h.query_by_label("Amount").is_none(), "Sales starts rolled up");
    let at = center(&h, "Sales");
    for round in 0..2 {
        h.hover_at(at);
        h.step();
        for pressed in [true, false] {
            h.event(Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            h.step();
        }
        steps(&mut h, 3);
        assert_eq!(
            h.query_by_label("Amount").is_some(),
            round == 0,
            "after click {round}: open = {:?}",
            h.state().panes.open
        );
        steps(&mut h, 40);
    }
}

/// A hidden table and a table whose fields are all hidden are left out of the Data pane (nothing
/// to unroll, as in Desktop); a link at the end shows them.
#[test]
fn tables_with_nothing_visible_are_left_out_and_one_click_shows_them() {
    let mut h = open(false);
    assert!(h.query_by_label("Sales").is_some());
    h.state_mut().run(
        crate::actions::Call {
            id: "table.set",
            params: json!({"name": "Sales", "hidden": true}),
        },
        crate::app::Then::Nothing,
    );
    for _ in 0..200 {
        h.step();
        if h.state().calls_pending() == 0 && h.query_by_label("Sales").is_none() {
            break;
        }
    }
    assert!(
        h.query_by_label("Sales").is_none(),
        "the hidden table is left out: error {:?}, hidden {:?}",
        h.state().error,
        h.state()
            .loaded
            .as_ref()
            .map(|l| l.snap.pointer("/model/tables/0/hidden").cloned())
    );
    h.get_by_label("Show hidden tables (1)").click();
    steps(&mut h, 3);
    assert!(h.state().panes.show_hidden);
    assert!(h.query_by_label("Sales").is_some(), "shown again");
}

/// The status bar's Actual size shows the page at 100 % (one report pixel per point, the sharpest
/// text on a 100 % screen), and Fit page fits it again.
#[test]
fn actual_size_shows_the_page_at_one_hundred_percent() {
    use crate::app::ViewZoom;
    let mut h = open(false);
    let fitted = h.state().placed.unwrap().zoom;
    assert!((fitted - 1.0).abs() > 0.01, "the page starts fitted ({fitted})");
    h.get_by_label("Actual size").click();
    steps(&mut h, 4);
    assert_eq!(h.state().view_zoom, Some(ViewZoom::Actual));
    assert_eq!(h.state().placed.unwrap().zoom, 1.0);
    h.get_by_label("Fit page").click();
    steps(&mut h, 4);
    assert_eq!(h.state().view_zoom, Some(ViewZoom::FitPage));
    assert!((h.state().placed.unwrap().zoom - fitted).abs() < 1e-3);
}
