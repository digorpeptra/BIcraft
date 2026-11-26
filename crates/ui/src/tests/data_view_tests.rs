//! The Data view on an engine session (`data_view.rs`, ROADMAP M11.17): the view switch, the
//! grid paged from `table.rows` and scrolled, sort and filter from a header's menu, the Column
//! tools' pickers running `column.set`, New column and New measure on the formula bar, copy as
//! TSV, a query preview from the Data pane and from Home, See records from a data point, and the
//! same frames in both shells.

use std::path::Path;

use bicraft_engine::Session;
use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::frame;
use crate::App;
use crate::app::Tab;
use crate::registry::Tab as RTab;
use crate::shell::Caps;

/// Rows of the Sales table.
const ROWS: usize = 3_000;

/// Sales(Id, Name, Amount, Double = Sales[Amount] * 2) of [`ROWS`] rows made by M; a query
/// `Small`; a column chart of [Total] by Sales[Name].
fn session() -> Session {
    let mut s = Session::new();
    let sales = format!(
        "let n = List.Numbers(1, {ROWS}), t = Table.FromColumns({{n, List.Transform(n, each \"Item \" & \
         Text.From(Number.Mod(_, 7))), List.Transform(n, each Number.Mod(_ * 37, 1000))}}, {{\"Id\", \"Name\", \
         \"Amount\"}}) in t"
    );
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Id", "data_type": "int64"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Name", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Double", "data_type": "double", "expression": "Sales[Amount] * 2"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "expression.add",
            json!({"name": "Small", "expression": "#table({\"A\", \"B\"}, {{1, \"x\"}, {2, \"y\"}})"}),
        ),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute(
        "visual.add",
        &json!({"page": page, "visual_type": "clusteredColumnChart", "x": 40, "y": 40, "width": 600, "height": 400,
                "fields": [{"role": "Category", "field": "Sales[Name]"}, {"role": "Y", "field": "[Total]"}]}),
    )
    .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn open(caps: Caps) -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(caps, session)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 860.0))
        .build_ui_state(
            |ui, app: &mut App| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        );
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() {
            break;
        }
    }
    h
}

/// Steps until `done` holds (the engine answers at once; the window needs a few frames).
fn until(h: &mut Harness<'_, App>, what: &str, done: impl Fn(&App) -> bool) {
    for _ in 0..200 {
        h.step();
        if done(h.state()) {
            h.step();
            return;
        }
    }
    panic!(
        "never: {what} (error {:?}, view {:?}, status {:?}, sent {:?})",
        h.state().error,
        h.state().cmd.data.error,
        h.state().status,
        h.state()
            .sent
            .iter()
            .rev()
            .take(3)
            .map(|c| c.id)
            .collect::<Vec<_>>()
    );
}

/// Opens the Data view from the ribbon's Data tab and waits for the first page.
fn data_view(h: &mut Harness<'_, App>) {
    let tab = h
        .get_all_by_label("Data")
        .min_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y))
        .unwrap();
    tab.click();
    until(h, "the first page", |a| {
        a.tab == Tab::Data && a.cmd.data.row_count.is_some()
    });
}

fn last_params(h: &Harness<'_, App>, id: &str) -> Value {
    h.state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == id)
        .map(|c| c.params.clone())
        .unwrap_or(Value::Null)
}

fn cell(h: &Harness<'_, App>, r: usize, c: usize) -> Value {
    h.state().cmd.data.cell(r, c).cloned().unwrap_or(Value::Null)
}

#[test]
fn the_data_view_pages_a_table_as_it_scrolls() {
    let mut h = open(Caps::DESKTOP);
    data_view(&mut h);
    let dv = &h.state().cmd.data;
    assert_eq!(dv.table.as_deref(), Some("Sales"));
    assert_eq!(dv.row_count, Some(ROWS));
    let names: Vec<&str> = dv.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Id", "Name", "Amount", "Double"]);
    assert!(dv.columns[3].calculated);
    // the headers and the first rows, the calculated column computed
    for label in ["Id", "Name", "Amount", "Double", "Sort and filter Name"] {
        h.get_by_label(label);
    }
    assert_eq!(cell(&h, 0, 0), json!(1));
    assert_eq!(cell(&h, 0, 3), json!(74));
    // only one page was asked for
    let pages: Vec<Value> = h
        .state()
        .sent
        .iter()
        .filter(|c| c.id == "table.rows")
        .map(|c| c.params.clone())
        .collect();
    assert_eq!(pages.len(), 1, "{pages:?}");
    assert_eq!(pages[0]["limit"], json!(crate::data_view::PAGE));
    // the status bar has the row count
    let status = crate::data_view::status(h.state()).unwrap();
    assert!(status.contains("3,000 rows"), "{status}");
    // scrolled far down, the pages there are read, not the ones between
    let grid = h.get_by_label("Id").rect().center() + egui::vec2(0.0, 200.0);
    h.hover_at(grid);
    for _ in 0..30 {
        h.event(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -2000.0),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        });
        h.step();
    }
    until(&mut h, "the rows scrolled to", |a| {
        let v = &a.cmd.data.visible;
        v.start > 1_000 && a.cmd.data.cell(v.start, 0).is_some()
    });
    let start = h.state().cmd.data.visible.start;
    assert_eq!(cell(&h, start, 0), json!(start + 1));
    let asked = h.state().sent.iter().filter(|c| c.id == "table.rows").count();
    assert!(asked < ROWS / crate::data_view::PAGE, "{asked} pages asked");
}

#[test]
fn a_headers_menu_sorts_and_filters_the_view() {
    let mut h = open(Caps::DESKTOP);
    data_view(&mut h);
    let rev = h.state().loaded.as_ref().unwrap().snap["revision"].clone();
    h.get_by_label("Sort and filter Name").click();
    steps(&mut h, 3);
    h.get_by_label("Sort descending").click();
    until(&mut h, "the sorted page", |a| {
        a.cmd.data.sort.is_some() && a.cmd.data.cell(0, 1).is_some()
    });
    assert_eq!(
        last_params(&h, "table.rows")["sort"],
        json!({"column": "Name", "descending": true})
    );
    assert_eq!(cell(&h, 0, 1), json!("Item 6"));
    assert_eq!(cell(&h, 0, 0), json!(6));
    // keep one value of Amount... of Name: untick all, tick Item 3, OK
    h.get_by_label("Sort and filter Name").click();
    until(&mut h, "the values", |a| a.cmd.data.values.is_some());
    assert_eq!(last_params(&h, "column.values")["column"], "Name");
    h.get_by_label("(Select all)").click();
    steps(&mut h, 2);
    h.get_by_label("Item 3").click();
    steps(&mut h, 2);
    h.get_by_label("OK").click();
    until(&mut h, "the filtered page", |a| {
        !a.cmd.data.filters.is_empty() && a.cmd.data.row_count.is_some_and(|n| n < ROWS)
    });
    let p = last_params(&h, "table.rows");
    assert_eq!(
        p["filter"],
        json!([{"column": "Name", "values": ["Item 3"]}]),
        "{p}"
    );
    assert_eq!(h.state().cmd.data.row_count, Some(ROWS.div_ceil(7)));
    assert_eq!(cell(&h, 0, 1), json!("Item 3"));
    // view state only: the model is unchanged
    assert_eq!(h.state().loaded.as_ref().unwrap().snap["revision"], rev);
    h.get_by_label("Clear sort and filters").click();
    until(&mut h, "every row again", |a| a.cmd.data.row_count == Some(ROWS));
}

/// Types `text` into the text box labelled `label`.
fn type_into(h: &mut Harness<'_, App>, label: &str, text: &str) {
    h.get_by_label(label).click();
    steps(h, 2);
    h.event(Event::Text(text.into()));
    steps(h, 2);
}

/// The header menu's Text filters and Number filters open the Custom filter window; its tests
/// go to `table.rows` (docs/gaps.md G49).
#[test]
fn a_headers_menu_has_text_and_number_filters() {
    let mut h = open(Caps::DESKTOP);
    data_view(&mut h);
    // Name: Text filters › Ends with…
    h.get_by_label("Sort and filter Name").click();
    steps(&mut h, 3);
    assert!(h.query_by_label("Number filters").is_none(), "a text column");
    h.get_by_label("Text filters").click();
    // the section opens with an animation
    steps(&mut h, 20);
    h.get_by_label("Ends with…").click();
    steps(&mut h, 3);
    assert!(h.state().cmd.data.menu.is_none(), "the menu closes");
    assert!(h.query_by_label("Custom filter").is_some(), "the window");
    type_into(&mut h, "First test: value", "3");
    h.get_by_label("OK").click();
    until(&mut h, "the filtered page", |a| {
        a.cmd.data.custom.is_none() && a.cmd.data.row_count.is_some_and(|n| n < ROWS)
    });
    let p = last_params(&h, "table.rows");
    assert_eq!(
        p["filter"],
        json!([{"column": "Name", "op": "ends_with", "value": "3"}]),
        "{p}"
    );
    assert_eq!(h.state().cmd.data.row_count, Some(ROWS.div_ceil(7)));
    // Amount: Number filters › Between… (two tests with And), then Or
    h.get_by_label("Sort and filter Amount").click();
    steps(&mut h, 3);
    h.get_by_label("Number filters").click();
    steps(&mut h, 20);
    h.get_by_label("Between…").click();
    steps(&mut h, 3);
    type_into(&mut h, "First test: value", "100");
    type_into(&mut h, "Second test: value", "199,5");
    h.get_by_label("OK").click();
    until(&mut h, "both filters", |a| {
        a.cmd.data.custom.is_none() && a.cmd.data.filters.len() == 2
    });
    let p = last_params(&h, "table.rows");
    assert_eq!(
        p["filter"],
        json!([
            {"column": "Amount", "op": ">=", "value": 100.0},
            {"column": "Amount", "op": "<=", "value": 199.5},
            {"column": "Name", "op": "ends_with", "value": "3"},
        ]),
        "{p}"
    );
    until(&mut h, "the rows", |a| a.cmd.data.cell(0, 2).is_some());
    let a = cell(&h, 0, 2).as_f64().unwrap();
    assert!((100.0..=199.5).contains(&a), "{a}");
    // the same column again: the window opens on the filter set, Or this time
    h.get_by_label("Sort and filter Amount").click();
    steps(&mut h, 3);
    // the section stays open as it was left
    assert!(h.query_by_label("Custom filter…").is_some());
    h.get_by_label("Custom filter…").click();
    steps(&mut h, 3);
    let c = h.state().cmd.data.custom.clone().unwrap();
    assert_eq!(c.first, (">=".to_string(), "100".to_string()));
    assert_eq!(c.second, ("<=".to_string(), "199.5".to_string()));
    h.get_by_label("Or").click();
    steps(&mut h, 2);
    h.get_by_label("OK").click();
    until(&mut h, "the Or filter", |a| a.cmd.data.custom.is_none());
    steps(&mut h, 3);
    let p = last_params(&h, "table.rows");
    assert_eq!(
        p["filter"][0],
        json!({"column": "Amount", "any": [{"op": ">=", "value": 100.0}, {"op": "<=", "value": 199.5}]}),
        "{p}"
    );
    // Cancel changes nothing; Clear sort and filters clears every filter
    h.get_by_label("Sort and filter Name").click();
    steps(&mut h, 3);
    h.get_by_label("Contains…").click();
    steps(&mut h, 3);
    h.get_by_label("Cancel").click();
    steps(&mut h, 3);
    assert!(h.state().cmd.data.custom.is_none());
    assert_eq!(h.state().cmd.data.filters.len(), 2);
    h.get_by_label("Clear sort and filters").click();
    until(&mut h, "every row again", |a| a.cmd.data.row_count == Some(ROWS));
}

#[test]
fn the_column_tools_run_column_set_and_the_formula_bar() {
    let mut h = open(Caps::DESKTOP);
    data_view(&mut h);
    h.get_by_label("Amount").click();
    steps(&mut h, 3);
    assert_eq!(h.state().cmd.data.column.as_deref(), Some("Amount"));
    assert_eq!(h.state().ribbon_tab, RTab::Table);
    // Data category: City
    combo(&h, "Data category").click();
    steps(&mut h, 2);
    h.get_by_label("City").click();
    until(&mut h, "the category", |a| {
        a.cmd
            .data
            .columns
            .iter()
            .any(|c| c.name == "Amount" && c.data_category.as_deref() == Some("City"))
    });
    let p = last_params(&h, "column.set");
    assert_eq!(
        p,
        json!({"table": "Sales", "name": "Amount", "data_category": "City"})
    );
    // Format: thousands
    combo(&h, "Format").click();
    steps(&mut h, 2);
    h.get_by_label("Thousands (#,0)").click();
    until(&mut h, "the format", |a| {
        a.cmd
            .data
            .columns
            .iter()
            .any(|c| c.name == "Amount" && c.format_string.as_deref() == Some("#,0"))
    });
    // Summarization: average
    combo(&h, "Summarization").click();
    steps(&mut h, 2);
    h.get_by_label("Average").click();
    until(&mut h, "the summarization", |a| {
        a.cmd
            .data
            .columns
            .iter()
            .any(|c| c.name == "Amount" && c.summarize_by.as_deref() == Some("average"))
    });
    // a calculated column goes on the formula bar
    h.get_by_label("Double").click();
    until(&mut h, "the bar", |a| {
        a.formula
            .target
            .as_ref()
            .is_some_and(|t| t.name == "Double" && !t.new)
    });
    // New column: on the bar, committed with column.add (its type from the expression)
    h.get_by_label("New column").click();
    steps(&mut h, 3);
    let t = h.state().formula.target.clone().unwrap();
    assert!(
        t.new && t.kind == crate::formula::Kind::Column && t.table == "Sales",
        "{t:?}"
    );
    h.state_mut().formula.text = "Triple = Sales[Amount] * 3".into();
    assert!(crate::formula::commit(h.state_mut()));
    until(&mut h, "the new column", |a| {
        a.cmd.data.columns.iter().any(|c| c.name == "Triple")
    });
    assert_eq!(
        last_params(&h, "column.add"),
        json!({"table": "Sales", "name": "Triple", "expression": "Sales[Amount] * 3", "force": true})
    );
    let triple = h
        .state()
        .cmd
        .data
        .columns
        .iter()
        .position(|c| c.name == "Triple")
        .unwrap();
    assert_eq!(h.state().cmd.data.columns[triple].data_type, "double");
    assert_eq!(cell(&h, 0, triple), json!(111));
    // New measure: on the bar, in the table shown
    h.get_by_label("Triple").click();
    steps(&mut h, 3);
    h.get_by_label("New measure").click();
    steps(&mut h, 3);
    let t = h.state().formula.target.clone().unwrap();
    assert!(
        t.new && t.kind == crate::formula::Kind::Measure && t.table == "Sales",
        "{t:?}"
    );
}

#[test]
fn cells_are_copied_as_tab_separated_text() {
    let mut h = open(Caps::DESKTOP);
    data_view(&mut h);
    {
        let dv = &mut h.state_mut().cmd.data;
        dv.sel = Some(((0, 0), (2, 1)));
    }
    let (tsv, missing) = h.state().cmd.data.tsv();
    assert_eq!(tsv, "Id\tName\n1\tItem 1\n2\tItem 2\n3\tItem 3");
    assert_eq!(missing, 0);
    // a click selects a cell, Shift+click extends, Ctrl+C copies
    h.state_mut().cmd.data.sel = None;
    let first = h.get_by_label("Id").rect();
    let at = |row: usize| Pos2::new(first.center().x, first.max.y + 22.0 * row as f32 + 11.0);
    click_at(&mut h, at(0), Modifiers::NONE);
    click_at(&mut h, at(1), Modifiers::SHIFT);
    assert_eq!(h.state().cmd.data.sel, Some(((0, 0), (1, 0))));
    h.event(Event::Copy);
    steps(&mut h, 2);
    let copied = h.output().platform_output.commands.iter().find_map(|c| match c {
        egui::OutputCommand::CopyText(t) => Some(t.clone()),
        _ => None,
    });
    assert!(h.state().status.contains("Copied"), "{}", h.state().status);
    if let Some(t) = copied {
        assert_eq!(t, "Id\n1\n2");
    }
}

fn click_at(h: &mut Harness<'_, App>, pos: Pos2, modifiers: Modifiers) {
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event_modifiers(
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers,
            },
            modifiers,
        );
        h.step();
    }
}

#[test]
fn a_query_preview_shows_in_the_grid() {
    let mut h = open(Caps::DESKTOP);
    data_view(&mut h);
    h.get_by_label("Preview query Small").click();
    until(&mut h, "the preview", |a| a.cmd.data.stat.is_some());
    let s = h.state().cmd.data.stat.clone().unwrap();
    assert!(s.title.contains("Small"), "{}", s.title);
    assert_eq!(
        s.rows,
        vec![vec![json!(1), json!("x")], vec![json!(2), json!("y")]]
    );
    assert_eq!(last_params(&h, "query.preview"), json!({"name": "Small"}));
    // nothing changed in the model
    assert!(
        !h.state().loaded.as_ref().unwrap().snap["dirty"]["model"]
            .as_bool()
            .unwrap()
    );
    h.get_by_label("Close").click();
    until(&mut h, "the table again", |a| {
        a.cmd.data.stat.is_none() && a.cmd.data.row_count.is_some()
    });
    // Home > Preview query: the view and the pane to pick from
    h.state_mut().show_tab(Tab::Report);
    h.state_mut().select_ribbon_tab(RTab::Home);
    steps(&mut h, 2);
    h.get_by_label("Preview query").click();
    steps(&mut h, 3);
    assert_eq!(h.state().tab, Tab::Data);
    assert!(h.state().status.contains("Pick a query"), "{}", h.state().status);
}

#[test]
fn see_records_shows_the_rows_behind_a_data_point() {
    let mut h = open(Caps::DESKTOP);
    until(&mut h, "the chart's data", |a| {
        a.shown.as_ref().is_some_and(|(_, img)| {
            img.as_ref().is_ok_and(|i| {
                i.hits.iter().any(|x| {
                    x.action_for(false)
                        .is_some_and(|a| a["action"] == "select" && !a["point"].is_null())
                })
            })
        })
    });
    // the screen position of the first bar
    let (pos, point) = {
        let app = h.state();
        let placed = app.placed.unwrap();
        let (_, img) = app.shown.as_ref().unwrap();
        let hit = img
            .as_ref()
            .unwrap()
            .hits
            .iter()
            .find(|x| {
                x.action_for(false)
                    .is_some_and(|a| a["action"] == "select" && !a["point"].is_null())
            })
            .unwrap();
        let (x, y, w, hh) = hit.shape.bounds();
        (
            Pos2::new(
                placed.rect.min.x + (x + w / 2.0) * placed.zoom,
                placed.rect.min.y + (y + hh / 2.0) * placed.zoom,
            ),
            hit.action_for(false).unwrap()["point"].clone(),
        )
    };
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    steps(&mut h, 2);
    assert_eq!(h.state().cmd.point.as_ref(), Some(&point));
    h.get_by_label("See records").click();
    until(&mut h, "the records", |a| a.cmd.data.stat.is_some());
    assert_eq!(h.state().tab, Tab::Data);
    let p = last_params(&h, "visual.records");
    assert_eq!(p["point"], point);
    let s = h.state().cmd.data.stat.clone().unwrap();
    let category = point[0]["v"].as_str().unwrap().to_string();
    assert!(!s.rows.is_empty());
    let name = s.columns.iter().position(|c| c.name == "Name").unwrap();
    assert!(s.rows.iter().all(|r| r[name] == json!(category)), "{category}");
    let k: usize = category.trim_start_matches("Item ").parse().unwrap();
    assert_eq!(s.rows.len(), (1..=ROWS).filter(|n| n % 7 == k).count());
    assert!(s.note.contains("no other filter applies"), "{}", s.note);
}

/// The desktop's and the browser's shells draw the same Data view: grid, menu, Column tools.
#[test]
fn the_data_view_draws_the_same_in_both_shells() {
    let mut desktop = open(Caps::DESKTOP);
    let mut browser = open(Caps::BROWSER);
    for h in [&mut desktop, &mut browser] {
        data_view(h);
        h.get_by_label("Amount").click();
        steps(h, 4);
    }
    let (a, b) = (frame(&desktop), frame(&browser));
    assert!(!a.0.is_empty());
    assert!(a.0 == b.0, "the meshes differ");
    assert!(a.1 == b.1, "the font atlases differ");
    for h in [&mut desktop, &mut browser] {
        h.get_by_label("Sort and filter Name").click();
        until(h, "the values", |a| a.cmd.data.values.is_some());
        steps(h, 2);
    }
    let (a, b) = (frame(&desktop), frame(&browser));
    assert!(a.0 == b.0, "the meshes differ with the menu open");
}

/// A reader sees the rows and the menus, but the Column tools are greyed.
#[test]
fn a_reader_reads_but_does_not_edit() {
    let mut h = open(Caps::DESKTOP);
    h.state_mut().access = Some("reader".into());
    data_view(&mut h);
    h.get_by_label("Amount").click();
    steps(&mut h, 3);
    let n = combo(&h, "Data category");
    assert!(
        n.accesskit_node().is_disabled(),
        "the picker is enabled for a reader"
    );
    assert_eq!(crate::ribbon::state(h.state(), "table.rows"), Ok(()));
}

/// A Column tools picker: the node of that name right of its label.
fn combo<'a>(h: &'a Harness<'_, App>, label: &'a str) -> egui_kittest::Node<'a> {
    h.get_all_by_label(label)
        .max_by(|a, b| a.rect().min.x.total_cmp(&b.rect().min.x))
        .unwrap()
}

/// Every shape of the last frame, groups flattened.
fn shapes(h: &Harness<'_, App>) -> Vec<egui::Shape> {
    fn flat(s: &egui::Shape, out: &mut Vec<egui::Shape>) {
        match s {
            egui::Shape::Vec(v) => v.iter().for_each(|s| flat(s, out)),
            s => out.push(s.clone()),
        }
    }
    let mut out = Vec::new();
    for c in &h.output().shapes {
        flat(&c.shape, &mut out);
    }
    out
}

/// The grid is drawn as Desktop's: on the report surface (`card`, light and dark) without
/// stripes, with a line under every row and between every column, the numbers in `text` (they
/// were in `accent_text`, unreadable in dark); the header stays on `chrome`.
#[test]
fn the_grid_is_white_with_row_and_column_lines() {
    for b in [crate::theme::Brightness::Light, crate::theme::Brightness::Dark] {
        let mut h = open(Caps::DESKTOP);
        h.state_mut().set_theme(b);
        data_view(&mut h);
        steps(&mut h, 2);
        let t = crate::theme::Tokens::of(b);
        let head = h.get_by_label("Id").rect();
        let shapes = shapes(&h);
        // the second row's cell under Id: the topmost fill there is the surface, not a stripe
        let p = egui::pos2(head.center().x, head.max.y + crate::data_view::ROW_H * 1.5);
        let fill = shapes
            .iter()
            .rev()
            .find_map(|s| match s {
                egui::Shape::Rect(r) if r.rect.contains(p) && r.fill.a() > 0 => Some(r.fill),
                _ => None,
            })
            .unwrap();
        assert_eq!(fill, t.card, "{b:?}: the grid's background");
        // the numbers read on it: drawn in `text`, as the other cells
        let num = shapes
            .iter()
            .find_map(|s| match s {
                egui::Shape::Text(x) if x.galley.text() == "2" && x.pos.y > head.max.y => {
                    x.galley.job.sections.first().map(|s| s.format.color)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(num, t.text, "{b:?}: the numbers' colour");
        let lines = |vertical: bool| {
            shapes
                .iter()
                .filter(|s| match s {
                    egui::Shape::LineSegment {
                        points: [a, c],
                        stroke,
                    } => {
                        let below = a.y.min(c.y) >= head.max.y - 0.5;
                        let across = a.x.min(c.x) <= p.x && a.x.max(c.x) >= p.x;
                        stroke.color == t.separator
                            && below
                            && if vertical {
                                a.x == c.x
                            } else {
                                a.y == c.y && across
                            }
                    }
                    _ => false,
                })
                .count()
        };
        assert!(lines(false) >= 10, "{b:?}: {} row lines", lines(false));
        assert!(lines(true) >= 4 * 10, "{b:?}: {} column lines", lines(true));
    }
}

/// A long error wraps in a banner of bounded height above the Data view, which lays out below
/// it: neither the Data pane's title nor the grid's header is covered.
#[test]
fn the_error_banner_wraps_and_does_not_cover_the_data_view() {
    let mut h = open(Caps::DESKTOP);
    data_view(&mut h);
    let msg: String = (0..40).map(|i| format!("word{i} ")).collect::<String>() + "end";
    h.state_mut().error = Some(msg.clone());
    steps(&mut h, 3);
    let banner = h.get_by_label(msg.as_str()).rect();
    assert!(banner.max.x <= 1400.0, "{banner:?}");
    assert!(banner.height() > 30.0, "the message is on one line: {banner:?}");
    let title = h
        .get_all_by_label("Data")
        .max_by(|a, b| a.rect().min.x.total_cmp(&b.rect().min.x))
        .unwrap()
        .rect();
    let head = h.get_by_label("Id").rect();
    assert!(
        title.min.y >= banner.max.y,
        "the Data title {title:?} under the banner {banner:?}"
    );
    assert!(
        head.min.y >= banner.max.y,
        "the header {head:?} under the banner {banner:?}"
    );
    // a very long message scrolls inside the banner instead of pushing the view off the window
    let long: String = (0..2000).map(|i| format!("w{i} ")).collect();
    h.state_mut().error = Some(long);
    steps(&mut h, 3);
    let head = h.get_by_label("Id").rect();
    assert!(head.min.y < 400.0, "the banner grew to {head:?}");
}
