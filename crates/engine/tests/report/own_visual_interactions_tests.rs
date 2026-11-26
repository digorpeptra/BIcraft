//! BIcraft's fourteen visuals interact with the other visuals of their page inside BIcraft, as their
//! runtime does in Power BI (docs/gaps.md G29): a click on one of their marks (the renderer's hit
//! map, `bicraft_render::own_hits`) selects the mark's rows and filters the other visuals, Ctrl
//! adds, the same click or the background clears, a selection elsewhere highlights the eleven with
//! a measure field (G114) and a slicer filters them, a bookmark captures and restores their
//! selection. Synthetic data only.

use std::collections::HashMap;

use bicraft_engine::Session;
use bicraft_render::hits::{Hit, HitAction};
use bicraft_render::{FontBook, RenderOptions};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// One table whose columns feed every visual: five rows, each a distinct K.
fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    let m = "#table(type table [K = text, G = text, Src = text, Dst = text, Set = text, Id = Int64.Type, \
             Parent = Int64.Type, D = date, V = Int64.Type], {\
             {\"a\", \"x\", \"A\", \"B\", \"S1\", 1, null, #date(2024, 1, 1), 10}, \
             {\"b\", \"x\", \"A\", \"C\", \"S2\", 2, 1, #date(2024, 1, 2), 20}, \
             {\"c\", \"y\", \"B\", \"C\", \"S1\", 3, 1, #date(2024, 1, 3), 35}, \
             {\"d\", \"y\", \"B\", \"D\", \"S2\", 4, 2, #date(2024, 1, 4), 40}, \
             {\"e\", \"y\", \"C\", \"D\", \"S1\", 5, 2, #date(2024, 1, 5), 400}})";
    run(&mut s, "table.import", json!({"name": "T", "m": m}));
    for (name, e) in [
        ("Total", "SUM(T[V])"),
        ("Rows", "COUNTROWS(T)"),
        ("Target", "100"),
    ] {
        run(
            &mut s,
            "measure.add",
            json!({"table": "T", "name": name, "expression": e}),
        );
    }
    run(&mut s, "report.new", json!({}));
    let p = page(&s);
    run(
        &mut s,
        "page.set_type",
        json!({"page": p, "type": "report", "width": 1600, "height": 1300}),
    );
    s
}

fn page(s: &Session) -> String {
    s.report.as_ref().unwrap().pages[0].name.clone()
}

fn add(s: &mut Session, visual_type: &str, fields: Value, x: i64, y: i64) -> String {
    let p = page(s);
    run(
        s,
        "visual.add",
        json!({"page": p, "visual_type": visual_type, "x": x, "y": y, "width": 380, "height": 230, "fields": fields}),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The fourteen visuals, each with its fields: (name, kind, fields).
fn visuals() -> Vec<(&'static str, Value)> {
    vec![
        (
            "bicraftHistogram",
            json!([{"role": "values", "field": "T[V]"}, {"role": "frequency", "field": "[Rows]"}]),
        ),
        (
            "bicraftBullet",
            json!([{"role": "category", "field": "T[K]"}, {"role": "value", "field": "[Total]"}, {"role": "target", "field": "[Target]"}]),
        ),
        (
            "bicraftBoxPlot",
            json!([{"role": "category", "field": "T[G]"}, {"role": "sample", "field": "T[K]"}, {"role": "value", "field": "[Total]"}]),
        ),
        (
            "bicraftDumbbell",
            json!([{"role": "category", "field": "T[K]"}, {"role": "start", "field": "[Total]"}, {"role": "end", "field": "[Target]"}]),
        ),
        (
            "bicraftSpec",
            json!([{"role": "category", "field": "T[K]"}, {"role": "values", "field": "[Total]"}]),
        ),
        (
            "bicraftOrgChart",
            json!([{"role": "id", "field": "T[Id]"}, {"role": "parent", "field": "T[Parent]"}, {"role": "label", "field": "T[K]"}, {"role": "value", "field": "[Total]"}]),
        ),
        (
            "bicraftVenn",
            json!([{"role": "item", "field": "T[K]"}, {"role": "set", "field": "T[Set]"}]),
        ),
        (
            "bicraftMarimekko",
            json!([{"role": "category", "field": "T[G]"}, {"role": "series", "field": "T[K]"}, {"role": "value", "field": "[Total]"}]),
        ),
        (
            "bicraftCalendarHeatmap",
            json!([{"role": "date", "field": "T[D]"}, {"role": "value", "field": "[Total]"}]),
        ),
        (
            "bicraftSankey",
            json!([{"role": "source", "field": "T[Src]"}, {"role": "destination", "field": "T[Dst]"}, {"role": "weight", "field": "[Total]"}]),
        ),
        (
            "bicraftScatterPlus",
            json!([{"role": "details", "field": "T[K]"}, {"role": "x", "field": "[Rows]"}, {"role": "y", "field": "[Total]"}]),
        ),
        (
            "bicraftLineAreaPlus",
            json!([{"role": "axis", "field": "T[D]"}, {"role": "value", "field": "[Total]"}]),
        ),
        (
            "bicraftWaterfallPlus",
            json!([{"role": "category", "field": "T[K]"}, {"role": "values", "field": "[Total]"}, {"role": "target", "field": "[Target]"}]),
        ),
        (
            "bicraftGaugeKpi",
            json!([{"role": "trend", "field": "T[D]"}, {"role": "value", "field": "[Total]"}, {"role": "target", "field": "[Target]"}]),
        ),
    ]
}

/// The page with every visual: the fourteen, a table of every column ("all") and a slicer on G.
fn page_with_all(s: &mut Session) -> (Vec<(String, &'static str)>, String, String) {
    let mut ours = Vec::new();
    for (i, (kind, fields)) in visuals().into_iter().enumerate() {
        let (x, y) = (10 + (i as i64 % 4) * 395, 10 + (i as i64 / 4) * 245);
        ours.push((add(s, kind, fields, x, y), kind));
    }
    let all = add(
        s,
        "tableEx",
        json!([{"role": "Values", "field": "T[K]"}, {"role": "Values", "field": "T[G]"},
               {"role": "Values", "field": "T[Src]"}, {"role": "Values", "field": "T[Dst]"},
               {"role": "Values", "field": "T[Set]"}, {"role": "Values", "field": "T[Id]"},
               {"role": "Values", "field": "T[Parent]"}, {"role": "Values", "field": "T[D]"},
               {"role": "Values", "field": "T[V]"}]),
        10,
        760,
    );
    let slicer = add(
        s,
        "slicer",
        json!([{"role": "Values", "field": "T[G]"}]),
        800,
        760,
    );
    (ours, all, slicer)
}

fn visual_json(snap: &Value, visual: &str) -> Value {
    snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == visual)
        .cloned()
        .unwrap_or_else(|| panic!("visual {visual} not in the snapshot"))
}

fn rows(s: &Session, visual: &str) -> Vec<Value> {
    let d = visual_json(&s.snapshot(), visual)["data"].clone();
    assert_eq!(d["status"], "ok", "{visual}: {d}");
    d["rows"].as_array().cloned().unwrap_or_default()
}

/// The page's hit map as the app draws it.
fn hits(s: &Session) -> Vec<Hit> {
    let fonts = FontBook::bundled_only();
    let o = RenderOptions {
        system_fonts: false,
        hits: true,
        ..RenderOptions::default()
    };
    bicraft_render::render_page(&s.snapshot(), 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &o)
        .unwrap()
        .hits
}

/// The distinct `select` actions of visual `v`'s data points, in the hit map's order.
fn selects(s: &Session, v: &str) -> Vec<(Value, Vec<(String, String)>)> {
    let mut out: Vec<(Value, Vec<(String, String)>)> = Vec::new();
    for h in hits(s) {
        if h.visual != v {
            continue;
        }
        if let HitAction::Send { action, .. } = &h.action
            && action["action"] == "select"
            && !action["point"].is_null()
            && !out.iter().any(|(a, _)| a == action)
        {
            out.push((action.clone(), h.tip.clone()));
        }
    }
    out
}

/// The points a select action sends: `points` when several, else `point`.
fn points_of(a: &Value) -> Vec<Value> {
    match a["points"].as_array() {
        Some(l) => l.clone(),
        None => vec![a["point"].clone()],
    }
}

fn selected_points(s: &Session, v: &str) -> Vec<Value> {
    visual_json(&s.snapshot(), v)["interaction"]["selected"]["points"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Every row of the "all" table matches one of the points (by the clicked visual's field names).
fn all_rows_match(s: &Session, ours: &str, all: &str, points: &[Value]) {
    let snap = s.snapshot();
    let fields = visual_json(&snap, ours)["fields"].clone();
    let cols = visual_json(&snap, all)["data"]["columns"].clone();
    let col_of = |c: u64| {
        let name = fields[c as usize]["name"].as_str().unwrap().to_string();
        cols.as_array()
            .unwrap()
            .iter()
            .position(|x| x["name"] == name.as_str())
            .unwrap_or_else(|| panic!("{name} not in the table: {cols}"))
    };
    for r in rows(s, all) {
        let hit = points.iter().any(|p| {
            p.as_array().unwrap().iter().all(|e| {
                let got = &r[col_of(e["c"].as_u64().unwrap())];
                got == &e["v"]
                    || got
                        .as_str()
                        .zip(e["v"].as_str())
                        .is_some_and(|(a, b)| a.starts_with(b) || b.starts_with(a))
            })
        });
        assert!(hit, "{ours}: row {r} matches none of {points:?}");
    }
}

/// Direction 1: a click on a mark of each of the fourteen selects it and filters the page; Ctrl adds
/// a second mark; the same click clears; the background clears.
#[test]
fn a_click_on_each_bicraft_visual_selects_and_filters_the_page() {
    let mut s = session();
    let p = page(&s);
    let (ours, all, _) = page_with_all(&mut s);
    assert_eq!(rows(&s, &all).len(), 5);
    for (v, kind) in &ours {
        let acts = selects(&s, v);
        assert!(
            acts.len() >= 2,
            "{kind}: two data points or more to click, got {acts:?}"
        );
        let (first, tip) = acts[0].clone();
        assert!(!tip.is_empty(), "{kind}: a tooltip on its marks");
        assert!(
            tip.iter().all(|(k, v)| !k.is_empty() && !v.is_empty()),
            "{kind}: {tip:?}"
        );
        // a plain click selects the mark's rows; the table keeps only them
        let mut a = first.clone();
        a["ctrl"] = json!(false);
        run(&mut s, "view.interact", a.clone());
        let pts = points_of(&first);
        assert_eq!(selected_points(&s, v).len(), pts.len(), "{kind}");
        let left = rows(&s, &all).len();
        assert!((1..5).contains(&left), "{kind}: the table keeps {left} rows");
        all_rows_match(&s, v, &all, &pts);
        // the visual itself shows the selection: its selected rows are reported for drawing
        let sel = visual_json(&s.snapshot(), v)["interaction"]["selected"]["rows"].clone();
        assert!(!sel.as_array().unwrap().is_empty(), "{kind}: {sel}");
        // Ctrl+click on another mark adds it
        let second = acts
            .iter()
            .map(|(a, _)| a.clone())
            .find(|a| points_of(a).iter().any(|q| !pts.contains(q)))
            .unwrap_or_else(|| panic!("{kind}: a second mark"));
        let mut b = second.clone();
        b["ctrl"] = json!(true);
        run(&mut s, "view.interact", b);
        let more = selected_points(&s, v).len();
        assert!(more > pts.len(), "{kind}: Ctrl adds ({more})");
        assert!(rows(&s, &all).len() >= left, "{kind}");
        // a plain click on the first mark selects it alone, a second click clears
        run(&mut s, "view.interact", a.clone());
        assert_eq!(selected_points(&s, v).len(), pts.len(), "{kind}");
        run(&mut s, "view.interact", a.clone());
        assert!(selected_points(&s, v).is_empty(), "{kind}: the same click clears");
        assert_eq!(rows(&s, &all).len(), 5, "{kind}");
        // the background of the visual clears its selection
        run(&mut s, "view.interact", a);
        let bg = hits(&s)
            .into_iter()
            .find(|h| {
                &h.visual == v
                    && h.action_for(false)
                        .is_some_and(|x| x["action"] == "select" && x["point"].is_null())
            })
            .unwrap_or_else(|| panic!("{kind}: a background hit"));
        run(&mut s, "view.interact", bg.action_for(false).unwrap());
        assert!(selected_points(&s, v).is_empty(), "{kind}: the background clears");
        assert_eq!(rows(&s, &all).len(), 5, "{kind}");
        let _ = &p;
    }
}

/// The rows of a visual's data that another visual's selection highlights (a value of the row's
/// `highlights` that is not null).
fn lit_rows(s: &Session, visual: &str) -> usize {
    let d = visual_json(&s.snapshot(), visual)["data"].clone();
    d["highlights"]
        .as_array()
        .map(|l| {
            l.iter()
                .filter(|r| r.as_array().is_some_and(|c| c.iter().any(Value::is_number)))
                .count()
        })
        .unwrap_or(0)
}

/// Direction 2: a selection in a built-in visual highlights each of the eleven with a measure role
/// (they declare `supportsHighlight` since G114: every row kept, the selected one's highlighted
/// part in `highlights`, the layouts drawing it); the Venn diagram, without a value column, is
/// filtered; a slicer filters all fourteen.
#[test]
fn a_table_selection_highlights_and_a_slicer_filters_each_bicraft_visual() {
    let mut s = session();
    let p = page(&s);
    let (ours, all, slicer) = page_with_all(&mut s);
    let before: Vec<usize> = ours.iter().map(|(v, _)| rows(&s, v).len()).collect();
    for ((_, kind), n) in ours.iter().zip(&before) {
        assert!(*n >= 2, "{kind}: {n} rows before");
    }
    // the table's row "a"
    run(
        &mut s,
        "view.interact",
        json!({"action": "select", "page": p, "visual": all, "point": [{"c": 0, "v": "a"}]}),
    );
    for ((v, kind), n) in ours.iter().zip(&before) {
        let mode = visual_json(&s.snapshot(), v)["interaction"]["mode"].clone();
        if *kind == "bicraftVenn" {
            assert_eq!(rows(&s, v).len(), 1, "{kind}: only row a");
            assert_eq!(mode, "filter", "{kind}");
        } else {
            assert_eq!(rows(&s, v).len(), *n, "{kind}: every row kept");
            assert_eq!(mode, "highlight", "{kind}");
            assert_eq!(lit_rows(&s, v), 1, "{kind}: row a highlighted");
        }
        // still drawn, with its hits
        assert!(
            !selects(&s, v).is_empty(),
            "{kind}: its remaining mark is clickable"
        );
    }
    let r = run(&mut s, "report.render", json!({"page": p, "system_fonts": false}));
    for v in r["pages"][0]["visuals"].as_array().unwrap() {
        assert_eq!(v["status"], "drawn", "{v}");
    }
    run(
        &mut s,
        "view.interact",
        json!({"action": "clear_selection", "page": p}),
    );
    // the slicer on G = x: rows a and b
    run(
        &mut s,
        "view.interact",
        json!({"action": "slicer_set", "page": p, "visual": slicer, "values": ["x"]}),
    );
    for (v, kind) in &ours {
        assert_eq!(rows(&s, v).len(), 2, "{kind}: rows a and b");
    }
    // the report draws every one
    let r = run(&mut s, "report.render", json!({"page": p, "system_fonts": false}));
    for v in r["pages"][0]["visuals"].as_array().unwrap() {
        assert_eq!(v["status"], "drawn", "{v}");
    }
}

/// A BIcraft visual that declares `supportsHighlight` with no field in a measure role is highlighted
/// all the same, as Power BI Desktop 2.159.1231.0 does (2026-11-24): Edit interactions offers and
/// defaults to Highlight, and the selection leaves it unchanged (every row, no `highlights`, nothing
/// dimmed). The report's cross-filter default filters it, and a `HighlightFilter` rule then
/// highlights it.
#[test]
fn a_bicraft_visual_without_a_measure_field_is_highlighted_unchanged() {
    let mut s = session();
    let p = page(&s);
    let bare = add(
        &mut s,
        "bicraftHistogram",
        json!([{"role": "values", "field": "T[V]"}]),
        10,
        10,
    );
    let bullet = add(
        &mut s,
        "bicraftBullet",
        json!([{"role": "category", "field": "T[K]"}, {"role": "value", "field": "[Total]"}]),
        400,
        10,
    );
    let table = add(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "T[K]"}]),
        10,
        300,
    );
    let mode = |s: &Session, v: &str| visual_json(&s.snapshot(), v)["interaction"]["mode"].clone();
    let all_rows = rows(&s, &bare).len();
    assert!(all_rows >= 2, "{all_rows}");
    run(
        &mut s,
        "view.interact",
        json!({"action": "select", "page": p, "visual": table, "point": [{"c": 0, "v": "b"}]}),
    );
    assert_eq!(mode(&s, &bare), "highlight");
    assert_eq!(rows(&s, &bare).len(), all_rows);
    let data = visual_json(&s.snapshot(), &bare)["data"].clone();
    assert!(data["highlights"].is_null(), "{data}");
    assert_eq!(mode(&s, &bullet), "highlight");
    assert_eq!(rows(&s, &bullet).len(), 5);
    let snap = s.snapshot();
    let ix = |v: &str| visual_json(&snap, v)["interactivity"]["highlight"].clone();
    assert_eq!((ix(&bare), ix(&bullet)), (json!(true), json!(true)));
    // the report cross-filters by default: filtered, unless a rule asks for the highlight
    run(
        &mut s,
        "report.settings",
        json!({"default_interaction": "filter"}),
    );
    assert_eq!(mode(&s, &bullet), "filter");
    run(
        &mut s,
        "page.interactions",
        json!({"page": p, "rules": [{"source": table, "target": bullet, "type": "highlight"}]}),
    );
    assert_eq!(mode(&s, &bullet), "highlight");
}

/// A bookmark captures a selection made in one of the fourteen and restores it, in the reader and with
/// `bookmark.apply`.
#[test]
fn a_bookmark_restores_a_selection_in_a_bicraft_visual() {
    let mut s = session();
    let p = page(&s);
    let (ours, all, _) = page_with_all(&mut s);
    for (i, (v, kind)) in ours.iter().enumerate() {
        let (a, _) = selects(&s, v)[0].clone();
        run(&mut s, "view.interact", a);
        let want = selected_points(&s, v);
        let left = rows(&s, &all).len();
        let name = format!("Sel {i}");
        let cap = run(&mut s, "bookmark.capture", json!({"display_name": name}));
        let bm = cap["bookmark"].as_str().unwrap_or(&name).to_string();
        let doc = s
            .report
            .as_ref()
            .unwrap()
            .bookmarks
            .last()
            .unwrap()
            .document
            .clone();
        let h = doc["explorationState"]["sections"][&p]["visualContainers"][v]["highlight"].clone();
        assert!(h["selection"].is_array(), "{kind}: {h}");
        run(
            &mut s,
            "view.interact",
            json!({"action": "clear_selection", "page": p}),
        );
        assert!(selected_points(&s, v).is_empty());
        run(
            &mut s,
            "view.interact",
            json!({"action": "bookmark", "bookmark": bm}),
        );
        assert_eq!(selected_points(&s, v).len(), want.len(), "{kind}: restored");
        assert_eq!(rows(&s, &all).len(), left, "{kind}: the page filtered again");
        run(
            &mut s,
            "view.interact",
            json!({"action": "clear_selection", "page": p}),
        );
        run(&mut s, "bookmark.apply", json!({"bookmark": bm}));
        assert_eq!(
            selected_points(&s, v).len(),
            want.len(),
            "{kind}: bookmark.apply restores"
        );
        run(
            &mut s,
            "view.interact",
            json!({"action": "clear_selection", "page": p}),
        );
    }
}

/// `points` is checked like `point`: at most 1,000, each a valid point of the visual.
#[test]
fn several_points_are_checked() {
    let mut s = session();
    let p = page(&s);
    let (ours, _, _) = page_with_all(&mut s);
    let venn = &ours[6].0;
    let bad = s
        .execute(
            "view.interact",
            &json!({"action": "select", "page": p, "visual": venn, "point": [{"c": 1, "v": "S1"}],
                    "points": [[{"c": 1, "v": "S1"}], [{"c": 9, "v": "S2"}]]}),
        )
        .unwrap_err();
    assert!(bad.contains("no data column 9"), "{bad}");
    let many: Vec<Value> = (0..1001)
        .map(|i| json!([{"c": 1, "v": format!("S{i}")}]))
        .collect();
    let bad = s
        .execute(
            "view.interact",
            &json!({"action": "select", "page": p, "visual": venn, "point": [{"c": 1, "v": "S1"}], "points": many}),
        )
        .unwrap_err();
    assert!(bad.contains("at most 1000"), "{bad}");
}

/// The org chart's collapse buttons in the hit map (`tree_toggle` for their node).
fn toggles(s: &Session, v: &str) -> Vec<(Value, Vec<(String, String)>)> {
    let mut out: Vec<(Value, Vec<(String, String)>)> = Vec::new();
    for h in hits(s) {
        if let HitAction::Send { action, ctrl } = &h.action
            && h.visual == v
            && action["action"] == "tree_toggle"
            && !out.iter().any(|(a, _)| a == action)
        {
            assert!(!ctrl, "{action}");
            out.push((action.clone(), h.tip.clone()));
        }
    }
    out
}

/// The org chart's expand and collapse buttons are clickable in BIcraft (docs/gaps.md G29): a
/// reader's click (`view.interact` `tree_toggle`, the viewer's) collapses the node in this view
/// only; `visual.toggle_node` (the app's editing view) saves `tree.toggled` as the visual's
/// TypeScript persists it, undoable.
#[test]
fn the_org_charts_collapse_buttons_toggle_their_nodes() {
    let mut s = session();
    let p = page(&s);
    let (ours, _, _) = page_with_all(&mut s);
    let org = ours[5].0.clone();
    // nodes 1 and 2 have children: two buttons, both open (Collapse)
    let t = toggles(&s, &org);
    assert_eq!(t.len(), 2, "{t:?}");
    assert_eq!(
        t[0].0,
        json!({"action": "tree_toggle", "page": p, "visual": org, "id": "1"})
    );
    assert_eq!(t[0].1, vec![("Collapse".to_string(), "a".to_string())]);
    let nodes = |s: &Session| selects(s, &org).len();
    assert_eq!(nodes(&s), 5);
    // the reader collapses node 2: its two children are no longer drawn, the report unchanged
    let before = s.report.clone();
    run(&mut s, "view.interact", t[1].0.clone());
    assert_eq!(s.report, before);
    assert_eq!(nodes(&s), 3);
    let t = toggles(&s, &org);
    assert_eq!(t[1].1, vec![("Expand".to_string(), "b".to_string())]);
    assert_eq!(
        visual_json(&s.snapshot(), &org)["chart_pack"]["options"]["toggled"],
        json!(["2"])
    );
    // again: back to the stored tree; reset drops the reader's toggles too
    run(&mut s, "view.interact", t[1].0.clone());
    assert_eq!(nodes(&s), 5);
    run(&mut s, "view.interact", t[1].0.clone());
    run(&mut s, "view.interact", json!({"action": "reset"}));
    assert_eq!(nodes(&s), 5);
    // the editing view saves it
    let r = run(
        &mut s,
        "visual.toggle_node",
        json!({"page": p, "visual": org, "id": "1"}),
    );
    assert_eq!(r["toggled"], json!(["1"]));
    assert_eq!(nodes(&s), 1);
    let v = s.report.as_ref().unwrap().pages[0]
        .visuals
        .iter()
        .find(|v| v.name == org)
        .unwrap()
        .clone();
    assert_eq!(
        v.visual_extra["objects"]["tree"][0]["properties"]["toggled"]["expr"]["Literal"]["Value"],
        json!("'[\"1\"]'")
    );
    let r = run(
        &mut s,
        "visual.toggle_node",
        json!({"page": p, "visual": org, "id": "1"}),
    );
    assert_eq!(r["toggled"], json!([]));
    assert_eq!(nodes(&s), 5);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(nodes(&s), 1);
    // not an org chart, no id, a bad id
    for bad in [
        json!({"page": p, "visual": ours[0].0, "id": "1"}),
        json!({"page": p, "visual": org}),
        json!({"page": p, "visual": org, "id": ""}),
        json!({"page": p, "visual": org, "id": "x".repeat(101)}),
    ] {
        assert!(s.execute("visual.toggle_node", &bad).is_err(), "{bad}");
        let mut a = bad.clone();
        a["action"] = json!("tree_toggle");
        assert!(s.execute("view.interact", &a).is_err(), "{a}");
    }
}

/// See records on a mark that stands for several points (a histogram bin: one point per value)
/// lists the rows of every point, not only the first's (docs/gaps.md G29).
#[test]
fn see_records_of_a_mark_of_several_points_lists_all_its_rows() {
    let mut s = session();
    let p = page(&s);
    let (ours, _, _) = page_with_all(&mut s);
    let bins = &ours[0].0;
    let (a, _) = selects(&s, bins)
        .into_iter()
        .find(|(a, _)| a["points"].as_array().is_some_and(|l| l.len() == 4))
        .expect("the 10 – 49 bin: four values");
    let r = run(
        &mut s,
        "visual.records",
        json!({"page": p, "visual": bins, "point": a["point"], "points": a["points"]}),
    );
    assert_eq!(r["rows"].as_array().unwrap().len(), 4, "{r}");
    assert_eq!(r["points"], 4);
    let first = run(
        &mut s,
        "visual.records",
        json!({"page": p, "visual": bins, "point": a["point"]}),
    );
    assert_eq!(first["rows"].as_array().unwrap().len(), 1);
}

/// The live viewer's page draws each of the fourteen from the publisher's `drawing`: an image served as
/// an asset and the hit map in the visual's pixels, with the same select actions as the app's.
#[test]
fn the_viewer_gets_a_drawing_and_hits_for_each_bicraft_visual() {
    let mut s = session();
    let (ours, all, _) = page_with_all(&mut s);
    let mut snap = s.snapshot();
    let assets = bicraft_engine::view_drawings::add_drawings(&mut snap, &HashMap::new());
    assert_eq!(assets.len(), ours.len(), "one image per visual");
    for (v, kind) in &ours {
        let d = visual_json(&snap, v)["drawing"].clone();
        let id = d["asset"].as_str().unwrap_or_else(|| panic!("{kind}: {d}"));
        let png = &assets.iter().find(|a| a.id == id).unwrap().bytes;
        assert!(png.starts_with(b"\x89PNG"), "{kind}");
        assert_eq!(d["width"], 380.0, "{kind}");
        let hits = d["hits"].as_array().unwrap();
        let app: Vec<Value> = selects(&s, v).into_iter().map(|(a, _)| a).collect();
        let sent: Vec<&Value> = hits
            .iter()
            .map(|x| &x["action"])
            .filter(|a| a["action"] == "select" && !a["point"].is_null())
            .collect();
        for a in &app {
            assert!(sent.contains(&a), "{kind}: {a} in the viewer's hits");
        }
        // in the visual's own pixels
        for x in hits {
            if let Some(r) = x["shape"]["rect"].as_array() {
                assert!(
                    r[0].as_f64().unwrap() >= -1.0 && r[0].as_f64().unwrap() <= 381.0,
                    "{kind}: {x}"
                );
            }
        }
    }
    assert!(
        visual_json(&snap, &all).get("drawing").is_none(),
        "built-in visuals are the page's own"
    );
}
