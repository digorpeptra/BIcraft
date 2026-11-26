//! The Gantt visual (docs/formats/gantt.md): `ganttChart` is a built-in type from the report
//! theme schema 2.159 with its formatting objects; its data roles come from a black-box wells test
//! against Desktop 2.159.1231.0 (docs/formats/visual-roles-bb.md), so the Build pane and the
//! gallery offer it. Synthetic model only.

use bicraft_engine::Session;
use serde_json::{Value, json};

use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Plan"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Tasks", "m": "let Source = #table(type table [Task = text, Start = date], {{\"Kickoff\", #date(2025, 1, 6)}}) in Source"}),
    );
    run(&mut s, "report.new", json!({}));
    s
}

#[test]
fn a_gantt_visual_is_added_formatted_and_round_trips() {
    let mut s = session();
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let out = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "ganttChart", "x": 0, "y": 0, "width": 600, "height": 400}),
    );
    assert!(
        out["warnings"].as_array().is_none_or(|w| w.is_empty()),
        "a built-in type: {out}"
    );
    let v = out["visual"].as_str().unwrap().to_string();
    // its formatting objects come from the schema: bars, milestones, progress, axis
    let opts = run(&mut s, "visual.options", json!({"visual_type": "ganttChart"}));
    let text = opts.to_string();
    for object in ["bar", "milestone", "progress", "axis", "row", "columnHeaders"] {
        assert!(text.contains(&format!("\"{object}\"")), "{object} in {text:.300}");
    }
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": v, "objects": [
            {"object": "bar", "property": "cornerRadius", "value": 4},
            {"object": "milestone", "property": "shape", "value": "diamond"}]}),
    );
    assert!(
        s.execute(
            "visual.format",
            &json!({"page": page, "visual": v, "objects": [{"object": "bar", "property": "nope", "value": 1}]}),
        )
        .is_err()
    );
    // fields: the roles Desktop saves (bb:gantt-A); a role it does not have is refused
    run(
        &mut s,
        "visual.bind",
        json!({"page": page, "visual": v, "role": "Task", "field": "Tasks[Task]"}),
    );
    run(
        &mut s,
        "visual.bind",
        json!({"page": page, "visual": v, "role": "StartDate", "field": "Min(Tasks[Start])"}),
    );
    assert!(
        s.execute(
            "visual.bind",
            &json!({"page": page, "visual": v, "role": "SomeRole", "field": "Tasks[Task]"}),
        )
        .is_err()
    );
    // offered in the gallery (its wells are known)
    let types = s.snapshot()["report"]["visual_types"].clone();
    assert!(types.to_string().contains("ganttChart"));
    let want = s.report.as_ref().unwrap().pages[0].visuals[0].clone();
    for ext in ["pbip", "pbix"] {
        let dir = TempDir::new(&format!("engine-gantt-{ext}"));
        let path = dir.join(format!("Plan.{ext}"));
        run(&mut s, "file.save_as", json!({"path": path}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        let got = back.report.as_ref().unwrap().pages[0].visuals[0].clone();
        assert_eq!(got.visual_type, want.visual_type, "{ext}");
        assert_eq!(
            got.visual_extra.get("objects"),
            want.visual_extra.get("objects"),
            "{ext}"
        );
    }
}

/// A plan of five rows: `Code` twice (its earliest start and latest end are the task's), a
/// review without an end (a milestone), progress as a measure, an owner in a detail column.
fn plan() -> (Session, String, String) {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Plan"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Tasks", "m": "let Source = #table(type table [Phase = text, Task = text, Start = date, End = date, Done = number, Owner = text], {\
            {\"Build\", \"Design\", #date(2025, 1, 6), #date(2025, 1, 31), 1, \"Ann\"},\
            {\"Build\", \"Code\", #date(2025, 2, 10), #date(2025, 3, 1), 0.2, \"Bob\"},\
            {\"Build\", \"Code\", #date(2025, 2, 3), #date(2025, 3, 28), 0.6, \"Bob\"},\
            {\"Launch\", \"Review\", #date(2025, 4, 1), null, null, \"Ann\"},\
            {\"Launch\", \"Ship\", #date(2025, 4, 7), #date(2025, 4, 30), 0, \"Cy\"}}) in Source"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Tasks", "name": "Progress", "expression": "AVERAGE(Tasks[Done])", "format_string": "0%"}),
    );
    run(&mut s, "report.new", json!({}));
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let out = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "ganttChart", "x": 0, "y": 0, "width": 900, "height": 300}),
    );
    let v = out["visual"].as_str().unwrap().to_string();
    for (role, field) in [
        ("Task", "Tasks[Task]"),
        ("StartDate", "Min(Tasks[Start])"),
        ("EndDate", "Max(Tasks[End])"),
        ("Progress", "Tasks[Progress]"),
        ("Category", "Tasks[Phase]"),
        ("Columns", "Min(Tasks[Owner])"),
    ] {
        run(
            &mut s,
            "visual.bind",
            json!({"page": page, "visual": v, "role": role, "field": field}),
        );
    }
    (s, page, v)
}

fn snapshot_visual(s: &Session, name: &str) -> Value {
    let snap = s.snapshot();
    snap["report"]["pages"][0]["visuals"]
        .as_array()
        .and_then(|vs| vs.iter().find(|v| v["name"] == name).cloned())
        .unwrap_or_else(|| panic!("{name} not in the snapshot"))
}

#[test]
fn a_gantt_is_evaluated_one_row_per_task_by_start() {
    let (s, _, v) = plan();
    let d = snapshot_visual(&s, &v)["data"].clone();
    assert_eq!(d["status"], "ok", "{d}");
    let roles: Vec<&str> = d["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["role"].as_str().unwrap())
        .collect();
    assert_eq!(
        roles,
        ["Task", "StartDate", "EndDate", "Progress", "Category", "Columns"]
    );
    // one row per task, earliest start first; Code takes its earliest start and latest end
    let rows = d["rows"].as_array().unwrap();
    let tasks: Vec<&str> = rows.iter().map(|r| r[0].as_str().unwrap()).collect();
    assert_eq!(tasks, ["Design", "Code", "Review", "Ship"], "{d}");
    assert_eq!(rows[1][1], "2025-02-03");
    assert_eq!(rows[1][2], "2025-03-28");
    assert!((rows[1][3].as_f64().unwrap() - 0.4).abs() < 1e-9);
    assert_eq!(
        (rows[1][4].clone(), rows[1][5].clone()),
        (json!("Build"), json!("Bob"))
    );
    // the review has no end: a milestone
    assert!(rows[2][2].is_null(), "{d}");
}

#[test]
fn a_gantt_carries_its_formatting_and_row_states_and_is_drawn() {
    let (mut s, page, v) = plan();
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": v, "objects": [
            {"object": "bar", "property": "cornerRadius", "value": 4},
            {"object": "milestone", "property": "shape", "value": "square"},
            {"object": "axis", "property": "autoScale", "value": false},
            {"object": "axis", "property": "primaryUnit", "value": "year"},
            {"object": "row", "property": "colorByCategory", "value": true}]}),
    );
    // the Launch phase collapsed (a `row` entry with a data selector), weekends shaded
    {
        let vis = s
            .report
            .as_mut()
            .and_then(|r| r.page_mut(&page))
            .and_then(|p| p.visual_mut(&v))
            .unwrap();
        let objects = vis.visual_extra.entry("objects").or_insert_with(|| json!({}));
        objects["row"].as_array_mut().unwrap().push(json!({
            "properties": {"isCollapsed": {"expr": {"Literal": {"Value": "true"}}}},
            "selector": {"data": [{"scopeId": {"Comparison": {"ComparisonKind": 0,
                "Left": {"Column": {"Expression": {"SourceRef": {"Entity": "Tasks"}}, "Property": "Phase"}},
                "Right": {"Literal": {"Value": "'Launch'"}}}}}]}}));
        objects["axis"].as_array_mut().unwrap()[0]["properties"]["shadingDays"] =
            json!({"expr": {"Literal": {"Value": "'Saturday, Sunday'"}}});
    }
    s.revision += 1;
    let vis = snapshot_visual(&s, &v);
    let g = &vis["format"]["gantt"];
    assert_eq!(g["bar"]["corner_radius"], 4, "{g}");
    assert_eq!(g["milestone"]["shape"], "square");
    assert_eq!(g["axis"]["auto_scale"], false);
    assert_eq!(g["axis"]["primary_unit"], "year");
    assert_eq!(g["axis"]["shading_days"], json!([0, 6]));
    assert_eq!(g["row"]["color_by_category"], true);
    assert_eq!(
        vis["format"]["gantt_rows"],
        json!([{"path": [{"c": 4, "v": "Launch"}], "collapsed": true}])
    );
    let r = run(
        &mut s,
        "report.render",
        json!({"page": page, "system_fonts": false}),
    );
    let note = &r["pages"][0]["visuals"][0];
    assert_eq!(note["status"], "drawn", "{note}");
}
