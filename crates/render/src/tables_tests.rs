//! Tests of tables and matrices (`tables.rs`): Desktop 2.158's grid layout (row height from the
//! row padding, clipped last row, pinned total, scrollbar, sort arrow, column widths) and
//! matrices with many column groups.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::{FontBook, RenderOptions, render_page};

fn texts(visual: Value, theme: Value) -> Vec<crate::canvas::TextRun> {
    let s = json!({
        "version": 1,
        "model": {"culture": "en-US", "tables": []},
        "report": {"theme": theme, "pages": [{"name": "p1", "display_name": "Page 1", "width": 1200, "height": 600,
                   "visuals": [visual]}]}
    });
    let fonts = FontBook::bundled_only();
    let o = RenderOptions {
        system_fonts: false,
        text_dump: true,
        ..RenderOptions::default()
    };
    render_page(&s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &o)
        .unwrap()
        .texts
}

fn theme() -> Value {
    json!({"name": "T", "data_colors": ["#118DFF"], "foreground": "#252423", "background": "#FFFFFF"})
}

/// A matrix of 12 months × 14 families (the first one blank), one value each.
fn matrix() -> Value {
    let families: Vec<Value> = std::iter::once(Value::Null)
        .chain((1..14).map(|i| json!(format!("F{i:02}"))))
        .collect();
    let mut rows = Vec::new();
    for m in 1..=12 {
        for f in &families {
            rows.push(json!([format!("M{m:02}"), f, 1.0]));
        }
    }
    json!({"name": "m", "type": "pivotTable", "x": 0, "y": 0, "width": 1200, "height": 460, "title": "",
        "format": {"title": {"show": false}, "table": {"row_padding": 3, "font_size": 10.5, "sort": [{"column": 0, "descending": false}]}},
        "data": {"status": "ok", "rows": rows, "columns": [
            {"name": "Month", "role": "Rows", "kind": "column", "data_type": "string"},
            {"name": "Family", "role": "Columns", "kind": "column", "data_type": "string"},
            {"name": "Spend", "role": "Values", "kind": "measure", "data_type": "double"}]}})
}

#[test]
fn matrix_shows_every_column_group_and_month() {
    let t = texts(matrix(), theme());
    let text: Vec<&str> = t.iter().map(|r| r.text.as_str()).collect();
    for i in 1..14 {
        let h = format!("F{i:02}");
        assert!(text.contains(&h.as_str()), "{h} missing: {text:?}");
    }
    for m in 1..=12 {
        let h = format!("M{m:02}");
        assert!(text.contains(&h.as_str()), "{h} missing: {text:?}");
    }
    // the grand total covers every cell; the blank family's header is empty, not "(Blank)"
    assert!(text.contains(&"168"), "{text:?}");
    assert!(!text.iter().any(|x| x.contains("Blank")), "{text:?}");
    // rows are line (19 px at 10.5 pt) + 2 × padding + rule
    let y = |s: &str| t.iter().find(|r| r.text == s).map(|r| r.y).unwrap();
    assert!(
        (y("M02") - y("M01") - 26.0).abs() < 0.6,
        "{} {}",
        y("M01"),
        y("M02")
    );
}

#[test]
fn overflowing_table_clips_its_last_row_and_pins_the_total() {
    let rows: Vec<Value> = (0..30).map(|i| json!([format!("R{i:02}"), 1.0])).collect();
    let v = json!({"name": "t", "type": "tableEx", "x": 0, "y": 0, "width": 600, "height": 324, "title": "",
        "format": {"title": {"show": false},
                   "table": {"row_padding": 6, "font_size": 10.5, "column_widths": [{"column": 0, "width": 300}],
                             "sort": [{"column": 0, "descending": true}]}},
        "data": {"status": "ok", "rows": rows, "columns": [
            {"name": "Name", "role": "Values", "kind": "column", "data_type": "string"},
            {"name": "Amount", "role": "Values", "kind": "measure", "data_type": "double"}]}});
    let t = texts(v, theme());
    let shown: Vec<&str> = t
        .iter()
        .filter(|r| r.text.starts_with('R'))
        .map(|r| r.text.as_str())
        .collect();
    // a 40 px header (with the sort arrow), 32 px rows and total: 7 rows whole, the 8th cut
    // (its text still drawn), the total at the bottom
    let pos: Vec<(String, f32)> = t.iter().map(|r| (r.text.clone(), r.y)).collect();
    assert_eq!(shown.len(), 8, "{pos:?}");
    let total = t.iter().find(|r| r.text == "Total").unwrap();
    let last = t.iter().find(|r| r.text == "R07").unwrap();
    assert!(
        total.y > last.y + 16.0 && total.y < 324.0,
        "{} {}",
        total.y,
        last.y
    );
    // the column keeps its width: the second column starts at 300
    let amount = t.iter().find(|r| r.text == "Amount").unwrap();
    assert!(amount.x >= 300.0, "{}", amount.x);
}

fn render(visual: Value, theme: Value) -> (tiny_skia::Pixmap, Vec<crate::canvas::TextRun>) {
    let s = json!({
        "version": 1,
        "model": {"culture": "en-US", "tables": [{"name": "Sales", "columns": [], "measures": [
            {"name": "Amount", "format_string": "#,0"}, {"name": "Rate", "format_string": "0.0%"}]}]},
        "report": {"theme": theme, "pages": [{"name": "p1", "display_name": "Page 1", "width": 1200, "height": 600,
                   "visuals": [visual]}]}
    });
    let fonts = FontBook::bundled_only();
    let o = RenderOptions {
        system_fonts: false,
        text_dump: true,
        ..RenderOptions::default()
    };
    let p = render_page(&s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &o).unwrap();
    (tiny_skia::Pixmap::decode_png(&p.png).unwrap(), p.texts)
}

/// Whether a pixel is the scrollbar thumb's grey (#AFAEAD).
fn thumb_grey(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> bool {
    pm.pixel(x, y)
        .is_some_and(|p| (i32::from(p.red()) - 0xaf).abs() < 6 && (i32::from(p.blue()) - 0xad).abs() < 6)
}

/// Rows of the thumb's grey in pixel column `x`.
fn thumb_rows(pm: &tiny_skia::Pixmap, x: u32) -> Vec<u32> {
    (0..pm.height()).filter(|y| thumb_grey(pm, x, *y)).collect()
}

/// A table of `n` items and `cols` value columns, `h` px high, in the classic look.
fn items(n: usize, h: u32, cols: usize) -> Value {
    let rows: Vec<Value> = (1..=n)
        .map(|i| {
            let mut r = vec![json!(format!("Item {i:04}"))];
            r.extend((0..cols).map(|k| json!(100_000.0 - i as f64 * 7.0 + k as f64 * 1_000_000.0)));
            Value::Array(r)
        })
        .collect();
    let mut columns =
        vec![json!({"name": "Name", "role": "Values", "kind": "column", "data_type": "string"})];
    columns.extend((0..cols).map(|k| {
        json!({"name": format!("Value measure {k:02}"), "role": "Values", "kind": "measure",
               "data_type": "double", "format_string": "#,0.00"})
    }));
    let w = if cols > 1 { 400 } else { 200 };
    json!({"name": "t", "type": "tableEx", "x": 0, "y": 0, "width": w, "height": h,
        "title": "", "format": {"title": {"show": false}},
        "data": {"status": "ok", "rows": rows, "columns": columns, "total_rows": n}})
}

#[test]
fn classic_tables_scroll_with_desktops_thumb() {
    // Desktop 2.158 (no theme): a 300 px table of 20 rows shows 15 (the last cut) and a thumb
    // of 211 px from 6.5 px under the visual's top, 13 to 5 px from its right edge; 59 rows:
    // 72 px; 500 rows: the 8 px minimum
    for (n, len) in [(20usize, 211.0f32), (59, 72.0), (500, 8.0)] {
        let (pm, t) = render(items(n, 300, 1), Value::Null);
        assert!(t.iter().any(|r| r.text == "Item 0015"), "{n}");
        assert!(!t.iter().any(|r| r.text == "Item 0016"), "{n}");
        let ys = thumb_rows(&pm, 191);
        let (a, z) = (ys.first().copied().unwrap_or(0), ys.last().copied().unwrap_or(0));
        assert!((a as f32 - 6.5).abs() <= 1.5, "{n}: thumb starts at {a}");
        assert!(
            ((z + 1 - a) as f32 - len).abs() <= 3.0,
            "{n}: thumb {a}..{z}, want {len}"
        );
    }
    // 10 rows fit: no thumb
    let (pm, _) = render(items(10, 300, 1), Value::Null);
    assert!(thumb_rows(&pm, 191).is_empty());
}

#[test]
fn wide_grids_keep_their_widths_and_scroll_sideways() {
    // 11 columns in a 400 px table: columns keep their natural widths (no ellipsis), the grid
    // is cut at the visual's edge and a horizontal thumb runs under it
    let (pm, t) = render(items(10, 300, 10), Value::Null);
    let text: Vec<&str> = t.iter().map(|r| r.text.as_str()).collect();
    assert!(text.contains(&"Value measure 00"), "{text:?}");
    assert!(!text.iter().any(|x| x.ends_with('…')), "{text:?}");
    assert!(t.iter().all(|r| r.x < 400.0));
    // the thumb: 8 px high at the bottom (y 287..295), from x 6.5, about the visible share
    let row: Vec<u32> = (0..400).filter(|x| thumb_grey(&pm, *x, 291)).collect();
    assert!(row.len() > 20 && row.len() < 300, "{}", row.len());
    assert!(
        row.first().is_some_and(|x| (5..=8).contains(x)),
        "{:?}",
        row.first()
    );
}

/// Two row fields, one column field, a sum and a ratio, with the engine's levels: the rows
/// collapsed to Region (`row_levels` 1), the totals evaluated (the ratio's are no sums).
fn region_matrix(format: Value) -> Value {
    let mut rows = Vec::new();
    for (r, k) in [("East", 1.0), ("West", 2.0)] {
        for c in ["Bikes", "Clothing"] {
            for ch in ["Online", "Store"] {
                rows.push(json!([r, c, ch, 100.0 * k, 0.5]));
            }
        }
    }
    let lvl = |rows: usize, cols: usize, data: Value| json!({"rows": rows, "columns": cols, "data": data});
    json!({"name": "m", "type": "pivotTable", "x": 0, "y": 0, "width": 600, "height": 300, "title": "",
        "format": {"title": {"show": false}, "table": format},
        "data": {"status": "ok", "rows": rows, "row_levels": 1, "columns": [
            {"name": "Region", "role": "Rows", "kind": "column", "data_type": "string"},
            {"name": "Category", "role": "Rows", "kind": "column", "data_type": "string"},
            {"name": "Channel", "role": "Columns", "kind": "column", "data_type": "string"},
            {"name": "Amount", "role": "Values", "kind": "measure", "data_type": "double", "format_string": "#,0"},
            {"name": "Rate", "role": "Values", "kind": "measure", "data_type": "double", "format_string": "0.0%"}],
            "subtotals": [
                lvl(0, 0, json!([[null, null, null, 1200.0, 0.41]])),
                lvl(0, 1, json!([[null, null, "Online", 600.0, 0.42], [null, null, "Store", 600.0, 0.43]])),
                lvl(1, 0, json!([["East", null, null, 400.0, 0.44], ["West", null, null, 800.0, 0.45]])),
                lvl(1, 1, json!([["East", null, "Online", 200.0, 0.46], ["East", null, "Store", 200.0, 0.47],
                                 ["West", null, "Online", 400.0, 0.48], ["West", null, "Store", 400.0, 0.49]]))]}})
}

#[test]
fn matrix_draws_the_engines_subtotals_collapsed() {
    let (_, t) = render(region_matrix(json!({})), Value::Null);
    let text: Vec<&str> = t.iter().map(|r| r.text.as_str()).collect();
    // collapsed to Region: no categories; the cells and totals are the engine's levels
    assert!(!text.contains(&"Bikes"), "{text:?}");
    for s in [
        "46.0%", "49.0%", "44.0%", "45.0%", "42.0%", "43.0%", "41.0%", "1,200", "400",
    ] {
        assert!(text.contains(&s), "{s} missing: {text:?}");
    }
    // the total column's cells are bold
    let bold = |s: &str| t.iter().filter(|r| r.text == s).all(|r| r.weight >= 700);
    assert!(bold("41.0%") && bold("44.0%"), "{text:?}");
    assert_eq!(text.iter().filter(|x| **x == "Total").count(), 2, "{text:?}");
}

#[test]
fn matrix_subtotal_switches() {
    // rowSubtotals off: no total row; columnSubtotals off: no total column
    let (_, t) = render(region_matrix(json!({"row_subtotals": false})), Value::Null);
    let text: Vec<&str> = t.iter().map(|r| r.text.as_str()).collect();
    assert!(!text.contains(&"41.0%") && !text.contains(&"42.0%"), "{text:?}");
    assert!(text.contains(&"44.0%"), "{text:?}");
    let (_, t) = render(region_matrix(json!({"column_subtotals": false})), Value::Null);
    let text: Vec<&str> = t.iter().map(|r| r.text.as_str()).collect();
    assert!(!text.contains(&"44.0%") && !text.contains(&"41.0%"), "{text:?}");
    assert!(text.contains(&"42.0%"), "{text:?}");
    assert_eq!(text.iter().filter(|x| **x == "Total").count(), 1, "{text:?}");
}

#[test]
fn expanded_matrix_puts_subtotals_on_top_or_bottom() {
    // every level shown (no row_levels): a member's row carries its subtotal (top), or a
    // "Total" row follows its children (bottom)
    let mut m = region_matrix(json!({}));
    if let Some(d) = m["data"].as_object_mut() {
        d.remove("row_levels");
    }
    let (_, t) = render(m.clone(), Value::Null);
    let text: Vec<&str> = t.iter().map(|r| r.text.as_str()).collect();
    assert!(text.contains(&"Bikes") && text.contains(&"44.0%"), "{text:?}");
    let y = |t: &[crate::canvas::TextRun], s: &str| t.iter().find(|r| r.text == s).map(|r| r.y).unwrap();
    assert!(y(&t, "44.0%") < y(&t, "Bikes"), "{text:?}");
    m["format"]["table"] = json!({"row_subtotals_position": "bottom"});
    let (_, t) = render(m, Value::Null);
    assert!(y(&t, "44.0%") > y(&t, "Bikes"));
}

/// `region_matrix` with every level the engine would send for an expanded hierarchy.
fn expanded(format: Value, data: Value) -> Value {
    let mut m = region_matrix(format);
    if let (Some(d), Some(extra)) = (m["data"].as_object_mut(), data.as_object()) {
        d.remove("row_levels");
        for (k, v) in extra {
            d.insert(k.clone(), v.clone());
        }
    }
    m
}

#[test]
fn single_expanded_member_shows_its_children_only() {
    // Desktop's "Expand > Selection" on West: East stays collapsed, both top rows bold
    let m = expanded(
        json!({}),
        json!({"row_expansion": {"open": [false, false], "toggled": [["West"]]}}),
    );
    let (_, t) = render(m, Value::Null);
    let text: Vec<&str> = t.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(text.iter().filter(|x| **x == "Bikes").count(), 1, "{text:?}");
    let run = |s: &str| t.iter().find(|r| r.text == s).unwrap();
    assert!(
        run("Bikes").y > run("West").y && run("East").weight >= 700,
        "{text:?}"
    );
}

#[test]
fn per_level_subtotals_and_labels() {
    // bottom subtotals labelled per level; the grand total switched off by the top level
    let m = expanded(
        json!({"row_subtotals_position": "bottom"}),
        json!({"row_level_totals": [{"show": false}, {"label": "Sous-cat"}]}),
    );
    let (_, t) = render(m, Value::Null);
    let text: Vec<&str> = t.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(text.iter().filter(|x| **x == "Sous-cat").count(), 2, "{text:?}");
    assert!(!text.contains(&"41.0%"), "{text:?}");
    // the subtotal row sits at its children's indent
    let x = |s: &str| t.iter().find(|r| r.text == s).map(|r| r.x).unwrap();
    assert!((x("Sous-cat") - x("Bikes")).abs() < 0.5, "{text:?}");
    // a visual-wide label names every subtotal and the grand total
    let m = expanded(
        json!({"row_subtotals_position": "bottom", "row_subtotals_label": "ST"}),
        json!({}),
    );
    let (_, t) = render(m, Value::Null);
    assert_eq!(t.iter().filter(|r| r.text == "ST").count(), 3);
}

#[test]
fn expanded_column_levels_get_header_rows() {
    // Channel expanded over Category: member labels over their children, a bold "Total"
    // after each member's children, the grand total last
    let mut rows = Vec::new();
    for r in ["East", "West"] {
        for ch in ["Online", "Store"] {
            for c in ["Bikes", "Clothing"] {
                rows.push(json!([r, ch, c, 10.0]));
            }
        }
    }
    let m = json!({"name": "m", "type": "pivotTable", "x": 0, "y": 0, "width": 900, "height": 300, "title": "",
        "format": {"title": {"show": false}, "table": {"column_subtotals_label": "SousCol"}},
        "data": {"status": "ok", "rows": rows, "column_levels": 1,
            "column_expansion": {"open": [true, false], "toggled": []}, "columns": [
            {"name": "Region", "role": "Rows", "kind": "column", "data_type": "string"},
            {"name": "Channel", "role": "Columns", "kind": "column", "data_type": "string"},
            {"name": "Category", "role": "Columns", "kind": "column", "data_type": "string"},
            {"name": "Amount", "role": "Values", "kind": "measure", "data_type": "double", "format_string": "#,0"}]}});
    let (_, t) = render(m, Value::Null);
    let text: Vec<&str> = t.iter().map(|r| r.text.as_str()).collect();
    for s in ["Channel", "Online", "Store", "Region"] {
        assert_eq!(text.iter().filter(|x| **x == s).count(), 1, "{s}: {text:?}");
    }
    assert_eq!(text.iter().filter(|x| **x == "Bikes").count(), 2, "{text:?}");
    // two member subtotals and the grand total, all labelled
    assert_eq!(text.iter().filter(|x| **x == "SousCol").count(), 3, "{text:?}");
    let run = |s: &str| t.iter().find(|r| r.text == s).unwrap();
    assert!(
        run("Online").y < run("Bikes").y && run("Channel").y < run("Region").y,
        "{text:?}"
    );
    // 40 = East's sum over Channel x Category, from the summed levels
    assert!(text.contains(&"40"), "{text:?}");
}

#[test]
fn five_row_levels_are_drawn_each_at_its_indent() {
    // regression: the renderer read four row fields at most, so a fifth level was dropped and
    // its parent's rows were drawn as leaves
    let names = ["L1", "L2", "L3", "L4", "L5"];
    let mut columns: Vec<Value> = names
        .iter()
        .map(|n| json!({"name": n, "role": "Rows", "kind": "column", "data_type": "string"}))
        .collect();
    columns.push(json!({"name": "Amount", "role": "Values", "kind": "measure", "data_type": "double", "format_string": "#,0"}));
    let m = json!({"name": "m", "type": "pivotTable", "x": 0, "y": 0, "width": 600, "height": 500, "title": "",
        "format": {"title": {"show": false}},
        "data": {"status": "ok", "rows": [["a", "b", "c", "d", "leaf1", 3.0], ["a", "b", "c", "d", "leaf2", 4.0]],
            "row_expansion": {"open": [true, true, true, true, false], "toggled": []},
            "subtotals": [
                {"rows": 0, "columns": 0, "data": [[null, null, null, null, null, 7.0]]},
                {"rows": 4, "columns": 0, "data": [["a", "b", "c", "d", null, 7.0]]}],
            "columns": columns}});
    let (_, t) = render(m, Value::Null);
    let run = |s: &str| {
        t.iter().find(|r| r.text == s).unwrap_or_else(|| {
            panic!(
                "{s} not drawn: {:?}",
                t.iter().map(|r| &r.text).collect::<Vec<_>>()
            )
        })
    };
    // the fifth level under the fourth, indented one more step
    assert!(run("leaf1").x > run("d").x + 5.0);
    assert!(run("leaf1").y > run("d").y);
    assert!(run("leaf2").y > run("leaf1").y);
}

/// A two-level matrix (Region > Category) collapsed to its top level, with +/- icons on and a
/// drill state saying the hierarchy has a level below the one shown.
fn plus_minus_matrix(buttons: bool) -> Value {
    json!({"name": "m", "type": "pivotTable", "x": 0, "y": 0, "width": 600, "height": 300, "title": "",
        "drill": {"role": "Rows", "matrix": true, "can_down": false, "top": 0, "bottom": 1},
        "format": {"title": {"show": false}, "table": {"expand_buttons": buttons}},
        "data": {"status": "ok", "rows": [["East", "Bikes", 10.0], ["East", "Toys", 5.0], ["West", "Bikes", 7.0]],
            "row_expansion": {"open": [false, false], "toggled": [["West"]]},
            "columns": [
                {"name": "Region", "role": "Rows", "kind": "column", "data_type": "string"},
                {"name": "Category", "role": "Rows", "kind": "column", "data_type": "string"},
                {"name": "Amount", "role": "Values", "kind": "measure", "data_type": "double"}]}})
}

fn page_hits(visual: Value) -> crate::PageImage {
    let s = json!({
        "version": 1,
        "model": {"culture": "en-US", "tables": []},
        "report": {"theme": theme(), "pages": [{"name": "p1", "display_name": "Page 1", "width": 1200, "height": 600,
                   "visuals": [visual]}]}
    });
    let o = RenderOptions {
        system_fonts: false,
        hits: true,
        text_dump: true,
        ..RenderOptions::default()
    };
    render_page(
        &s,
        0,
        &HashMap::<String, Vec<u8>>::new(),
        &FontBook::bundled_only(),
        &o,
    )
    .unwrap()
}

#[test]
fn matrix_row_headers_get_plus_minus_buttons_and_menus() {
    use crate::hits::HitAction;
    let img = page_hits(plus_minus_matrix(true));
    let menus: Vec<&Value> = img
        .hits
        .iter()
        .filter_map(|h| match &h.action {
            HitAction::Menu(m) => Some(m),
            _ => None,
        })
        .collect();
    // East (collapsed), West (expanded) and West's Bikes (the deepest level: no button)
    let paths: Vec<Value> = menus.iter().map(|m| m["path"].clone()).collect();
    assert!(
        paths.contains(&json!(["East"])) && paths.contains(&json!(["West"])),
        "{paths:?}"
    );
    assert!(paths.contains(&json!(["West", "Bikes"])), "{paths:?}");
    let open = |p: Value| menus.iter().find(|m| m["path"] == p).map(|m| m["open"].clone());
    assert_eq!(open(json!(["East"])), Some(json!(false)));
    assert_eq!(open(json!(["West"])), Some(json!(true)));
    assert_eq!(open(json!(["West", "Bikes"])), Some(Value::Null));
    // the buttons: + expands East, − collapses West
    let sends: Vec<Value> = img
        .hits
        .iter()
        .filter_map(|h| h.action_for(false))
        .filter(|a| a["action"].as_str().is_some_and(|s| s.starts_with("matrix_")))
        .collect();
    assert!(
        sends.contains(&json!({"action": "matrix_expand", "page": "p1", "visual": "m", "path": ["East"]})),
        "{sends:?}"
    );
    assert!(
        sends.contains(&json!({"action": "matrix_collapse", "page": "p1", "visual": "m", "path": ["West"]})),
        "{sends:?}"
    );
    // the label moves right of its button
    let x = |s: &str| img.texts.iter().find(|r| r.text == s).map(|r| r.x).unwrap();
    let plain = page_hits(plus_minus_matrix(false));
    let x0 = plain
        .texts
        .iter()
        .find(|r| r.text == "East")
        .map(|r| r.x)
        .unwrap();
    assert!(x("East") > x0 + 5.0, "{} {}", x("East"), x0);
    // icons off: menus only
    assert!(!plain.hits.iter().any(|h| {
        h.action_for(false)
            .is_some_and(|a| a["action"] == "matrix_expand")
    }));
    assert!(plain.hits.iter().any(|h| matches!(h.action, HitAction::Menu(_))));
}

#[test]
fn an_axis_expanded_down_a_level_concatenates_its_labels() {
    // "Expand all down one level": two category columns, Desktop's concatenated labels
    let v = json!({"name": "c", "type": "clusteredColumnChart", "x": 0, "y": 0, "width": 600, "height": 300, "title": "",
        "format": {"title": {"show": false}},
        "data": {"status": "ok", "rows": [["Toys", "Ball", 5.0], ["Toys", "Kite", 7.0], ["Wheels", "Bike", 150.5]],
            "columns": [
                {"name": "Category", "role": "Category", "kind": "column", "data_type": "string"},
                {"name": "Product", "role": "Category", "kind": "column", "data_type": "string"},
                {"name": "Total", "role": "Y", "kind": "measure", "data_type": "double"}]}});
    let img = page_hits(v);
    let text: Vec<&str> = img.texts.iter().map(|r| r.text.as_str()).collect();
    assert!(text.iter().any(|t| t.contains("Toys Ball")), "{text:?}");
    assert!(text.iter().any(|t| t.contains("Wheels Bike")), "{text:?}");
    // a point names both levels
    let pts: Vec<Value> = img
        .hits
        .iter()
        .filter_map(|h| h.action_for(false))
        .filter(|a| a["action"] == "select")
        .map(|a| a["point"].clone())
        .collect();
    assert!(
        pts.contains(&json!([{"c": 0, "v": "Toys"}, {"c": 1, "v": "Kite"}])),
        "{pts:?}"
    );
}

/// A table of products with a sum and a sparkline (red, markers in blue: `markers` as Desktop
/// writes them, 2 the highest point; null: none written).
fn sparkline_table_with(chart: &str, markers: Value) -> Value {
    json!({"name": "t", "type": "tableEx", "x": 0, "y": 0, "width": 600, "height": 300, "title": "",
        "format": {"title": {"show": false}},
        "data": {"status": "ok", "rows": [
                ["Bike", 425.0, {"sparkline": [[1, 100.0], [2, 300.0], [3, 25.0]]}],
                ["Car", 10.0, null],
                ["Ball", 5.0, {"sparkline": [[1, 5.0]]}]],
            "columns": [
                {"name": "Product", "role": "Values", "kind": "column", "data_type": "string"},
                {"name": "Sum of Amount", "role": "Values", "kind": "measure", "data_type": "double"},
                {"name": "Sum of Amount by Month", "role": "Values", "kind": "sparkline", "data_type": "sparkline",
                 "sparkline": {"chart_type": chart, "color": "#FF0000", "markers": markers, "marker_color": "#0000FF"}}]}})
}

/// A sparkline table with the highest point marked.
fn sparkline_table(chart: &str) -> Value {
    sparkline_table_with(chart, json!(2.0))
}

fn red(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> bool {
    pm.pixel(x, y)
        .is_some_and(|p| p.red() > 200 && p.green() < 80 && p.blue() < 80)
}

fn blue(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> bool {
    pm.pixel(x, y)
        .is_some_and(|p| p.blue() > 200 && p.red() < 80 && p.green() < 80)
}

/// The pixels of colour `f` in rows `y0..y1`, right of `x0`.
fn pixels_in(
    pm: &tiny_skia::Pixmap,
    f: fn(&tiny_skia::Pixmap, u32, u32) -> bool,
    x0: u32,
    y0: u32,
    y1: u32,
) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for y in y0..y1.min(pm.height()) {
        for x in x0..pm.width() {
            if f(pm, x, y) {
                out.push((x, y));
            }
        }
    }
    out
}

#[test]
fn sparkline_cells_are_small_charts() {
    for chart in ["column", "line"] {
        let (pm, t) = render(sparkline_table(chart), theme());
        let text: Vec<&str> = t.iter().map(|r| r.text.as_str()).collect();
        assert!(text.contains(&"Sum of Amount by Month"), "{chart}: {text:?}");
        // the points are drawn, never written out
        assert!(
            !text.iter().any(|x| x.contains("sparkline") || x.contains('[')),
            "{chart}: {text:?}"
        );
        let run = |s: &str| t.iter().find(|r| r.text == s).unwrap();
        let (bike, car, ball) = (run("Bike"), run("Car"), run("Ball"));
        let value_x = run("425").x as u32 + 20;
        // a row's band: from a little above its text to a little above the next row's
        let band = |r: &crate::canvas::TextRun, next: f32| ((r.y - 4.0) as u32, (next - 4.0) as u32);
        let (b0, b1) = band(bike, car.y);
        let (c0, c1) = band(car, ball.y);
        let (l0, l1) = band(ball, ball.y + (car.y - bike.y));
        let bike_red = pixels_in(&pm, red, value_x, b0, b1);
        assert!(!bike_red.is_empty(), "{chart}: no sparkline in Bike's row");
        assert!(
            pixels_in(&pm, red, value_x, c0, c1).is_empty(),
            "{chart}: a blank cell is drawn"
        );
        // a single point: a bar or a dot (here under its highest-point marker)
        assert!(
            !pixels_in(&pm, red, value_x, l0, l1).is_empty()
                || !pixels_in(&pm, blue, value_x, l0, l1).is_empty(),
            "{chart}: a one-point sparkline is not drawn"
        );
        // the highest point's marker
        assert!(
            !pixels_in(&pm, blue, value_x, b0, b1).is_empty(),
            "{chart}: no marker"
        );
        if chart == "column" {
            // the tallest bar (300) is the middle one of three
            let (x0, x1) = bike_red
                .iter()
                .fold((u32::MAX, 0), |(a, b), (x, _)| (a.min(*x), b.max(*x)));
            let top = bike_red.iter().min_by_key(|(_, y)| *y).map(|(x, _)| *x).unwrap();
            let third = (x1 - x0) / 3;
            assert!(
                top >= x0 + third && top <= x1 - third,
                "{chart}: tallest at {top} in {x0}..{x1}"
            );
        }
    }
}

/// The Total row's sparkline (Desktop 2.159.1231.0 draws one in a table's and a matrix's Total
/// row: docs/formats/sparklines.md "Seen in Desktop"), from the engine's `totals` or a matrix's
/// grand-total level.
#[test]
fn the_total_row_draws_its_sparkline() {
    let total_pts = json!({"sparkline": [[1, 105.0], [2, 300.0], [3, 25.0]]});
    let mut table = sparkline_table("line");
    table["data"]["totals"] = json!([null, 440.0, total_pts]);
    let mut matrix = sparkline_table("line");
    matrix["type"] = json!("pivotTable");
    matrix["data"]["columns"][0]["role"] = json!("Rows");
    matrix["data"]["subtotals"] = json!([{"rows": 0, "columns": 0, "data": [[null, 440.0, total_pts]]}]);
    for (what, v) in [("table", table), ("matrix", matrix)] {
        let (pm, t) = render(v, theme());
        let run = |s: &str| {
            t.iter().find(|r| r.text == s).unwrap_or_else(|| {
                panic!(
                    "{what}: no {s} in {:?}",
                    t.iter().map(|r| &r.text).collect::<Vec<_>>()
                )
            })
        };
        let (bike, car, total) = (run("Bike"), run("Car"), run("Total"));
        let value_x = run("440").x as u32 + 20;
        let row_h = car.y - bike.y;
        let band = ((total.y - 4.0) as u32, (total.y + row_h - 4.0) as u32);
        // the rows' sparklines are drawn too (a matrix drew none before 2026-11-24)
        let (b0, b1) = ((bike.y - 4.0) as u32, (car.y - 4.0) as u32);
        assert!(
            !pixels_in(&pm, red, value_x, b0, b1).is_empty(),
            "{what}: no sparkline in Bike's row"
        );
        assert!(
            !pixels_in(&pm, red, value_x, band.0, band.1).is_empty(),
            "{what}: no sparkline in the Total row"
        );
    }
}

/// A line sparkline over dates or numbers places each point by its X value on one X range for
/// every cell of the visual, as Power BI Desktop 2.159.1231.0 draws it (docs/formats/sparklines.md
/// "Seen in Desktop", 2026-11-24): a row whose points cover the first tenth of the visual's dates
/// stays in the first tenth of its cell, a single point on the last date sits at the right.
#[test]
fn sparkline_points_share_the_visuals_x_range() {
    let mut v = sparkline_table_with("line", json!(0.0));
    v["data"]["rows"] = json!([
        ["Bike", 30.0, {"sparkline": [["2026-01-01T00:00:00", 10.0], ["2026-01-11T00:00:00", 20.0]]}],
        ["Car", 10.0, null],
        ["Ball", 5.0, {"sparkline": [["2026-04-11T00:00:00", 5.0]]}]]);
    let (pm, t) = render(v, theme());
    let run = |s: &str| t.iter().find(|r| r.text == s).unwrap();
    let (bike, car, ball) = (run("Bike"), run("Car"), run("Ball"));
    let value_x = run("30").x as u32 + 20;
    let span = |y0: f32, y1: f32| {
        let px = pixels_in(&pm, red, value_x, (y0 - 4.0) as u32, (y1 - 4.0) as u32);
        px.iter()
            .fold((u32::MAX, 0), |(a, b), (x, _)| (a.min(*x), b.max(*x)))
    };
    let (a0, a1) = span(bike.y, car.y);
    let (b0, b1) = span(ball.y, ball.y + (car.y - bike.y));
    assert!(a1 > a0 && b1 >= b0, "nothing drawn: {a0}..{a1}, {b0}..{b1}");
    let b = (b0 + b1) / 2;
    // the cell's range is a0 (1 January) to b (11 April): Bike's line covers a tenth of it
    assert!(
        b > a1,
        "Ball's point at {b} is not right of Bike's line {a0}..{a1}"
    );
    assert!(
        (a1 - a0) * 5 < b - a0,
        "Bike's line {a0}..{a1} is not in the first tenth of {a0}..{b}"
    );
}

/// The `markers` bits as Power BI Desktop 2.159.1231.0 writes and draws them
/// (docs/formats/sparklines.md): 1 every point, 2 highest, 4 lowest, 8 first, 16 last,
/// 32 negative, 128 isolated; without `markers` only isolated points are marked.
#[test]
fn sparkline_marker_bits_are_desktops() {
    // the distinct marker columns (blue pixels more than 3 px apart) in Bike's and Ball's rows
    let marks = |markers: Value| {
        let (pm, t) = render(sparkline_table_with("line", markers), theme());
        let run = |s: &str| t.iter().find(|r| r.text == s).unwrap();
        let (bike, car, ball) = (run("Bike"), run("Car"), run("Ball"));
        let value_x = run("425").x as u32 + 20;
        let groups = |y0: f32, y1: f32| {
            let mut xs: Vec<u32> = pixels_in(&pm, blue, value_x, (y0 - 4.0) as u32, (y1 - 4.0) as u32)
                .iter()
                .map(|(x, _)| *x)
                .collect();
            xs.sort_unstable();
            xs.dedup();
            xs.windows(2).filter(|w| w[1] > w[0] + 3).count() + usize::from(!xs.is_empty())
        };
        (groups(bike.y, car.y), groups(ball.y, ball.y + (car.y - bike.y)))
    };
    // every point: Bike's three points
    assert_eq!(marks(json!(1.0)).0, 3);
    // highest and lowest, then first and last: two each
    assert_eq!(marks(json!(6.0)).0, 2);
    assert_eq!(marks(json!(24.0)).0, 2);
    // first only
    assert_eq!(marks(json!(8.0)).0, 1);
    // none written: isolated points only (Ball's single point, none on Bike's line)
    assert_eq!(marks(Value::Null), (0, 1));
    // 0: no marker at all
    assert_eq!(marks(json!(0.0)), (0, 0));
}
