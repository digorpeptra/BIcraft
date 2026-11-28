//! Canvas editing commands (ROADMAP M11.16): `visual.copy`, `visual.paste`, `visual.duplicate`,
//! `visual.move`, `visual.align`, `visual.group`, `visual.ungroup`, and the multi-selection forms
//! of `visual.remove` and `visual.order`: what they change, one undo step each, PBIP and PBIX
//! round trips, and hostile clipboard documents.

use bicraft_report::{FieldRef, Position, Report, Visual};
use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Runs a command that must fail and checks it changed nothing; returns the error.
fn fails(s: &mut Session, id: &str, params: Value) -> String {
    let before = s.report.clone();
    let dirty = s.report_dirty;
    let err = s
        .execute(id, &params)
        .expect_err(&format!("{id} {params} should fail"));
    assert_eq!(s.report, before, "{id} changed the report although it failed");
    assert_eq!(s.report_dirty, dirty);
    err
}

/// Sales(Category, Amount) from a literal table, [Total], a report of two pages.
fn session() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Amount"}, {{"Wheels", 100}, {"Toys", 5}})"#;
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
        ("page.add", json!({"display_name": "Second"})),
    ] {
        run(&mut s, id, p);
    }
    s
}

fn report(s: &Session) -> &Report {
    s.report.as_ref().unwrap()
}

fn page(s: &Session, i: usize) -> String {
    report(s).pages[i].name.clone()
}

fn visual<'a>(s: &'a Session, page: usize, name: &str) -> &'a Visual {
    report(s).pages[page].visual(name).unwrap()
}

fn pos(s: &Session, page: usize, name: &str) -> (f64, f64, f64, f64) {
    let p = visual(s, page, name).position;
    (p.x, p.y, p.width, p.height)
}

/// A visual of `ty` on page 0 at `rect` (x, y, width, height); returns its name.
fn add(s: &mut Session, ty: &str, rect: [f64; 4], fields: Value) -> String {
    let pg = page(s, 0);
    let v = run(
        s,
        "visual.add",
        json!({"page": pg, "visual_type": ty, "x": rect[0], "y": rect[1], "width": rect[2],
               "height": rect[3], "fields": fields}),
    );
    v["visual"].as_str().unwrap().to_string()
}

fn chart(s: &mut Session, rect: [f64; 4]) -> String {
    add(
        s,
        "clusteredColumnChart",
        rect,
        json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]),
    )
}

fn names(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

fn undo(s: &mut Session) {
    run(s, "edit.undo", json!({}));
}

#[test]
fn copy_then_paste_keeps_the_visual_and_moves_off_a_taken_place() {
    let mut s = session();
    let c = chart(&mut s, [10.0, 20.0, 400.0, 300.0]);
    let (p0, p1) = (page(&s, 0), page(&s, 1));
    run(
        &mut s,
        "visual.set_title",
        json!({"page": p0, "visual": c, "title": {"text": "Sales", "font_size": 14}}),
    );
    run(
        &mut s,
        "visual.filter",
        json!({"page": p0, "visual": c, "field": "Sales[Category]", "in": ["Toys"]}),
    );
    let copied = run(&mut s, "visual.copy", json!({"page": p0, "visual": c}));
    assert_eq!(copied["count"], 1);
    assert_eq!(copied["clip"]["bicraft"], "visuals");
    assert_eq!(copied["clip"]["version"], 1);
    // copying changes nothing: no undo step, no revision
    let rev = s.revision;
    assert_eq!(s.revision, rev);

    // another page: the same place, everything kept, a new name
    let out = run(
        &mut s,
        "visual.paste",
        json!({"page": p1, "clip": copied["clip"]}),
    );
    let pasted = names(&out["visuals"]);
    assert_eq!(pasted.len(), 1);
    assert_ne!(pasted[0], c);
    assert!(out["warnings"].as_array().unwrap().is_empty(), "{out}");
    let (orig, copy) = (visual(&s, 0, &c).clone(), visual(&s, 1, &pasted[0]).clone());
    assert_eq!(pos(&s, 1, &pasted[0]), (10.0, 20.0, 400.0, 300.0));
    assert_eq!(copy.fields, orig.fields);
    assert_eq!(copy.visual_extra, orig.visual_extra, "formatting kept");
    assert_eq!(copy.filter_config, orig.filter_config, "filters kept");
    assert_eq!(out["selectable"], json!(pasted));

    // the same page: the place is taken, so 20 px right and down, then 40
    let out = run(&mut s, "visual.paste", json!({"page": p0, "clip": copied}));
    let a = names(&out["visuals"]);
    assert_eq!(pos(&s, 0, &a[0]), (30.0, 40.0, 400.0, 300.0));
    let out = run(
        &mut s,
        "visual.paste",
        json!({"page": p0, "clip": copied["clip"]}),
    );
    let b = names(&out["visuals"]);
    assert_eq!(pos(&s, 0, &b[0]), (50.0, 60.0, 400.0, 300.0));
    // on top of the page
    let zs: Vec<f64> = report(&s).pages[0].visuals.iter().map(|v| v.position.z).collect();
    assert!(visual(&s, 0, &b[0]).position.z >= zs.iter().copied().fold(0.0, f64::max));
    // at a given corner, kept on the page
    let out = run(
        &mut s,
        "visual.paste",
        json!({"page": p1, "clip": copied["clip"], "x": 1200, "y": 5}),
    );
    let d = names(&out["visuals"]);
    assert_eq!(
        pos(&s, 1, &d[0]),
        (880.0, 5.0, 400.0, 300.0),
        "pulled back onto the page"
    );
    // each paste is one undo step
    let before = report(&s).pages[1].visuals.len();
    undo(&mut s);
    assert_eq!(report(&s).pages[1].visuals.len(), before - 1);
}

#[test]
fn paste_into_another_model_keeps_broken_fields_with_warnings() {
    let mut s = session();
    let c = chart(&mut s, [10.0, 20.0, 400.0, 300.0]);
    let p0 = page(&s, 0);
    let clip = run(&mut s, "visual.copy", json!({"page": p0, "visuals": [c]}))["clip"].clone();
    // another document whose model has no Sales table
    let mut t = Session::new();
    run(&mut t, "model.new", json!({"name": "Other"}));
    run(&mut t, "table.add", json!({"name": "Stock"}));
    run(&mut t, "report.new", json!({}));
    let tp = t.report.as_ref().unwrap().pages[0].name.clone();
    let out = run(&mut t, "visual.paste", json!({"page": tp, "clip": clip}));
    let w: Vec<&str> = out["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    assert_eq!(w.len(), 2, "{w:?}");
    assert!(w.iter().all(|m| m.contains("broken reference")), "{w:?}");
    assert!(w.iter().any(|m| m.contains("Sales[Category]")), "{w:?}");
    let v = names(&out["visuals"]);
    let pasted = t.report.as_ref().unwrap().pages[0].visual(&v[0]).unwrap();
    assert!(matches!(&pasted.fields[0].field, FieldRef::Column { table, .. } if table == "Sales"));
    // the report says so too
    let checked = run(&mut t, "report.validate", json!({}));
    assert!(checked.to_string().contains("Sales"), "{checked}");
}

#[test]
fn groups_are_copied_with_their_members_and_relinked() {
    let mut s = session();
    let a = chart(&mut s, [10.0, 20.0, 200.0, 100.0]);
    let b = chart(&mut s, [300.0, 20.0, 200.0, 100.0]);
    let p0 = page(&s, 0);
    let g = run(&mut s, "visual.group", json!({"page": p0, "visuals": [a, b]}));
    let group = g["group"].as_str().unwrap().to_string();
    assert_eq!(g["display_name"], "Group 1");
    assert_eq!(pos(&s, 0, &group), (10.0, 20.0, 490.0, 100.0));
    let out = run(&mut s, "visual.duplicate", json!({"page": p0, "visual": group}));
    let made = names(&out["visuals"]);
    assert_eq!(made.len(), 3);
    assert_eq!(names(&out["top"]).len(), 1, "one new group at the top level");
    assert_eq!(names(&out["selectable"]).len(), 2);
    let new_group = names(&out["top"])[0].clone();
    let pg = &report(&s).pages[0];
    let members: Vec<&Visual> = pg
        .visuals
        .iter()
        .filter(|v| v.parent_group.as_deref() == Some(new_group.as_str()))
        .collect();
    assert_eq!(members.len(), 2);
    assert!(pg.visual(&new_group).unwrap().extra.contains_key("visualGroup"));
    assert_eq!(pos(&s, 0, &new_group), (30.0, 40.0, 490.0, 100.0));
    // copying a member alone makes a top-level copy
    let clip = run(&mut s, "visual.copy", json!({"page": p0, "visual": a}));
    assert_eq!(clip["count"], 1);
    assert!(clip["clip"]["visuals"][0].get("parent_group").is_none());
}

#[test]
fn move_resizes_several_visuals_in_one_step_and_groups_follow() {
    let mut s = session();
    let a = chart(&mut s, [10.0, 20.0, 200.0, 100.0]);
    let b = chart(&mut s, [300.0, 20.0, 200.0, 100.0]);
    let c = chart(&mut s, [10.0, 300.0, 100.0, 100.0]);
    let p0 = page(&s, 0);
    let out = run(
        &mut s,
        "visual.move",
        json!({"page": p0, "visuals": [{"visual": a, "x": 50, "y": 60}, {"visual": b, "width": 250}]}),
    );
    assert_eq!(out["moved"].as_array().unwrap().len(), 2);
    assert_eq!(pos(&s, 0, &a), (50.0, 60.0, 200.0, 100.0));
    assert_eq!(pos(&s, 0, &b), (300.0, 20.0, 250.0, 100.0));
    undo(&mut s);
    assert_eq!(
        pos(&s, 0, &a),
        (10.0, 20.0, 200.0, 100.0),
        "one undo step for both"
    );
    assert_eq!(pos(&s, 0, &b), (300.0, 20.0, 200.0, 100.0));
    // a group moves and scales its members
    let g = run(&mut s, "visual.group", json!({"page": p0, "visuals": [a, b]}))["group"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut s,
        "visual.move",
        json!({"page": p0, "visuals": [{"visual": g, "x": 110, "width": 980}]}),
    );
    assert_eq!(pos(&s, 0, &a), (110.0, 20.0, 400.0, 100.0));
    assert_eq!(pos(&s, 0, &b), (690.0, 20.0, 400.0, 100.0));
    // a member moved alone: its group is refitted
    run(
        &mut s,
        "visual.move",
        json!({"page": p0, "visuals": [{"visual": b, "y": 220}]}),
    );
    assert_eq!(pos(&s, 0, &g), (110.0, 20.0, 980.0, 300.0));
    // a group and its member cannot both be listed; nothing listed twice; unknown keys refused
    fails(
        &mut s,
        "visual.move",
        json!({"page": p0, "visuals": [{"visual": g, "x": 0}, {"visual": a, "x": 0}]}),
    );
    fails(
        &mut s,
        "visual.move",
        json!({"page": p0, "visuals": [{"visual": c, "x": 0}, {"visual": c, "y": 0}]}),
    );
    fails(
        &mut s,
        "visual.move",
        json!({"page": p0, "visuals": [{"visual": c, "z": 0}]}),
    );
    fails(
        &mut s,
        "visual.move",
        json!({"page": p0, "visuals": [{"visual": c}]}),
    );
    fails(
        &mut s,
        "visual.move",
        json!({"page": p0, "visuals": [{"visual": c, "width": 0}]}),
    );
    // a no-op changes nothing
    let rev = s.revision;
    let out = run(
        &mut s,
        "visual.move",
        json!({"page": p0, "visuals": [{"visual": c, "x": 10}]}),
    );
    assert_eq!(out["changed"], false);
    assert!(s.revision >= rev);
}

#[test]
fn align_and_distribute() {
    let mut s = session();
    let a = chart(&mut s, [10.0, 20.0, 100.0, 50.0]);
    let b = chart(&mut s, [300.0, 200.0, 200.0, 80.0]);
    let c = chart(&mut s, [150.0, 100.0, 50.0, 40.0]);
    let p0 = page(&s, 0);
    let all = json!([a, b, c]);
    let al = |s: &mut Session, to: &str| {
        run(s, "visual.align", json!({"page": p0, "visuals": all, "to": to}));
    };
    al(&mut s, "left");
    assert_eq!([pos(&s, 0, &a).0, pos(&s, 0, &b).0, pos(&s, 0, &c).0], [10.0; 3]);
    undo(&mut s);
    assert_eq!(pos(&s, 0, &b).0, 300.0, "one undo step");
    al(&mut s, "right");
    assert_eq!(
        [pos(&s, 0, &a).0, pos(&s, 0, &b).0, pos(&s, 0, &c).0],
        [400.0, 300.0, 450.0]
    );
    undo(&mut s);
    al(&mut s, "center");
    assert_eq!(
        [pos(&s, 0, &a).0, pos(&s, 0, &b).0, pos(&s, 0, &c).0],
        [205.0, 155.0, 230.0]
    );
    undo(&mut s);
    al(&mut s, "top");
    assert_eq!([pos(&s, 0, &a).1, pos(&s, 0, &b).1, pos(&s, 0, &c).1], [20.0; 3]);
    undo(&mut s);
    al(&mut s, "bottom");
    assert_eq!(
        [pos(&s, 0, &a).1, pos(&s, 0, &b).1, pos(&s, 0, &c).1],
        [230.0, 200.0, 240.0]
    );
    undo(&mut s);
    al(&mut s, "middle");
    assert_eq!(
        [pos(&s, 0, &a).1, pos(&s, 0, &b).1, pos(&s, 0, &c).1],
        [125.0, 110.0, 130.0]
    );
    undo(&mut s);
    // distribute horizontally: a [10, 110], c [150, 200], b [300, 500]: span 490, sizes 350,
    // gaps 70: c moves to 180
    al(&mut s, "distribute_horizontal");
    assert_eq!(
        [pos(&s, 0, &a).0, pos(&s, 0, &c).0, pos(&s, 0, &b).0],
        [10.0, 180.0, 300.0]
    );
    // vertically: a [20, 70], c [100, 140], b [200, 280]: span 260, sizes 170, gaps 45
    al(&mut s, "distribute_vertical");
    assert_eq!(
        [pos(&s, 0, &a).1, pos(&s, 0, &c).1, pos(&s, 0, &b).1],
        [20.0, 115.0, 200.0]
    );
    // one visual aligns to the page (1280 x 720)
    run(
        &mut s,
        "visual.align",
        json!({"page": p0, "visual": b, "to": "right"}),
    );
    assert_eq!(pos(&s, 0, &b).0, 1080.0);
    run(
        &mut s,
        "visual.align",
        json!({"page": p0, "visual": b, "to": "middle"}),
    );
    assert_eq!(pos(&s, 0, &b).1, 320.0);
    let e = fails(
        &mut s,
        "visual.align",
        json!({"page": p0, "visuals": [a, b], "to": "distribute_vertical"}),
    );
    assert!(e.contains("three"), "{e}");
    fails(
        &mut s,
        "visual.align",
        json!({"page": p0, "visual": a, "to": "diagonal"}),
    );
    fails(&mut s, "visual.align", json!({"page": p0, "to": "left"}));
}

#[test]
fn group_and_ungroup_round_trip_through_pbip() {
    let dir = TempDir::new("canvas-group-pbip");
    let mut s = session();
    let a = chart(&mut s, [10.0, 20.0, 200.0, 100.0]);
    let b = chart(&mut s, [300.0, 20.0, 200.0, 100.0]);
    let p0 = page(&s, 0);
    let g = run(
        &mut s,
        "visual.group",
        json!({"page": p0, "visuals": [a, b], "display_name": "Pair"}),
    );
    let group = g["group"].as_str().unwrap().to_string();
    run(
        &mut s,
        "project.new",
        json!({"dir": dir.path(), "name": "Shop", "format": "tmsl"}),
    );
    let mut fresh = Session::new();
    run(&mut fresh, "file.open", json!({"path": dir.join("Shop.pbip")}));
    let gv = visual(&fresh, 0, &group);
    assert!(gv.visual_type.is_none());
    assert_eq!(gv.extra["visualGroup"]["displayName"], "Pair");
    assert_eq!(
        visual(&fresh, 0, &a).parent_group.as_deref(),
        Some(group.as_str())
    );
    assert_eq!(
        visual(&fresh, 0, &b).parent_group.as_deref(),
        Some(group.as_str())
    );
    let checked = run(&mut fresh, "report.validate", json!({}));
    assert_eq!(checked["valid"], true, "{checked}");
    // ungroup through a member
    let out = run(&mut fresh, "visual.ungroup", json!({"page": p0, "visual": a}));
    assert_eq!(out["ungrouped"], json!(group));
    assert!(report(&fresh).pages[0].visual(&group).is_none());
    assert!(visual(&fresh, 0, &a).parent_group.is_none());
    run(&mut fresh, "file.save", json!({}));
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": dir.join("Shop.pbip")}));
    assert!(report(&again).pages[0].visual(&group).is_none());
    assert_eq!(pos(&again, 0, &b), (300.0, 20.0, 200.0, 100.0));
    // errors
    fails(&mut again, "visual.group", json!({"page": p0, "visual": a}));
    fails(&mut again, "visual.ungroup", json!({"page": p0, "visual": a}));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pasted_and_moved_visuals_round_trip_through_pbix() {
    let dir = TempDir::new("canvas-pbix");
    let mut s = session();
    let a = chart(&mut s, [10.0, 20.0, 200.0, 100.0]);
    let (p0, p1) = (page(&s, 0), page(&s, 1));
    let clip = run(&mut s, "visual.copy", json!({"page": p0, "visual": a}));
    let pasted = names(&run(&mut s, "visual.paste", json!({"page": p1, "clip": clip}))["visuals"]);
    run(
        &mut s,
        "visual.move",
        json!({"page": p1, "visuals": [{"visual": pasted[0], "x": 500, "y": 400, "width": 300}]}),
    );
    let path = dir.join("Canvas.pbix");
    run(&mut s, "file.save_as", json!({"path": path}));
    let mut fresh = Session::new();
    run(&mut fresh, "file.open", json!({"path": path}));
    let v = visual(&fresh, 1, &pasted[0]);
    assert_eq!(
        (v.position.x, v.position.y, v.position.width),
        (500.0, 400.0, 300.0)
    );
    assert_eq!(v.fields, visual(&s, 0, &a).fields);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hostile_clips_are_refused_or_tamed() {
    let mut s = session();
    let a = chart(&mut s, [10.0, 20.0, 200.0, 100.0]);
    let p0 = page(&s, 0);
    let good = run(&mut s, "visual.copy", json!({"page": p0, "visual": a}))["clip"].clone();
    let with = |f: &dyn Fn(&mut Value)| {
        let mut c = good.clone();
        f(&mut c);
        c
    };
    let bad = [
        json!(null),
        json!("text"),
        json!([]),
        json!({}),
        with(&|c| c["bicraft"] = json!("pages")),
        with(&|c| c["version"] = json!(2)),
        with(&|c| c["visuals"] = json!([])),
        with(&|c| c["visuals"] = json!("x")),
        with(&|c| c["visuals"] = Value::Array(vec![c["visuals"][0].clone(); 501])),
        with(&|c| c["visuals"][0]["position"]["x"] = json!(1e300)),
        with(&|c| c["visuals"][0]["position"]["width"] = json!(-5)),
        with(&|c| c["visuals"][0]["position"] = json!("here")),
        with(&|c| c["visuals"][0]["visual_type"] = json!("<script>")),
        with(&|c| c["visuals"][0]["name"] = json!("")),
        with(&|c| c["visuals"][0]["name"] = json!("x".repeat(300))),
        with(&|c| c["visuals"][0]["fields"] = json!([{"role": 1}])),
        with(&|c| {
            let v = c["visuals"][0].clone();
            c["visuals"] = json!([v.clone(), v]);
        }),
    ];
    for clip in bad {
        fails(&mut s, "visual.paste", json!({"page": p0, "clip": clip}));
    }
    fails(&mut s, "visual.paste", json!({"page": p0, "clip": good, "x": 5}));
    fails(&mut s, "visual.paste", json!({"page": p0}));
    // a cycle of groups and keys the typed record writes are cut, not followed
    let g1 = json!({"name": "g1", "position": {"x": 0, "y": 0, "width": 10, "height": 10},
                    "parent_group": "g2"});
    let g2 = json!({"name": "g2", "position": {"x": 0, "y": 0, "width": 10, "height": 10},
                    "parent_group": "g1"});
    let mut v = good["visuals"][0].clone();
    v["parent_group"] = json!("g1");
    v["extra"] = json!({"parentGroupName": "elsewhere", "name": "evil", "howCreated": "Copilot",
                        "position": {"x": 999, "angle": 0}});
    let clip = json!({"bicraft": "visuals", "version": 1, "visuals": [g1, g2, v]});
    let out = run(&mut s, "visual.paste", json!({"page": p0, "clip": clip}));
    let made = names(&out["visuals"]);
    let pg = &report(&s).pages[0];
    for n in &made[..2] {
        assert!(pg.visual(n).unwrap().parent_group.is_none(), "cycle cut");
        assert!(pg.visual(n).unwrap().extra.contains_key("visualGroup"));
    }
    let pasted = pg.visual(&made[2]).unwrap();
    assert!(!pasted.extra.contains_key("parentGroupName"));
    assert!(!pasted.extra.contains_key("name"));
    assert_eq!(pasted.extra["howCreated"], "Copilot");
    assert_eq!(pasted.extra["position"], json!({"angle": 0}));
    let checked = run(&mut s, "report.validate", json!({}));
    assert!(!checked.to_string().contains("parentGroupName"), "{checked}");
}

#[test]
fn several_visuals_are_removed_and_reordered_in_one_step() {
    let mut s = session();
    let a = chart(&mut s, [10.0, 20.0, 100.0, 50.0]);
    let b = chart(&mut s, [300.0, 200.0, 200.0, 80.0]);
    let c = chart(&mut s, [150.0, 100.0, 50.0, 40.0]);
    let d = chart(&mut s, [150.0, 400.0, 50.0, 40.0]);
    let p0 = page(&s, 0);
    let z = |s: &Session, n: &str| visual(s, 0, n).position.z;
    // a and b to the front together, keeping their order (b in front of a)
    run(
        &mut s,
        "visual.order",
        json!({"page": p0, "visual": a, "visuals": [b], "to": "front"}),
    );
    assert!(z(&s, &b) > z(&s, &a) && z(&s, &a) > z(&s, &d) && z(&s, &d) > z(&s, &c));
    run(
        &mut s,
        "visual.order",
        json!({"page": p0, "visuals": [a, b], "to": "back"}),
    );
    assert!(z(&s, &d) > z(&s, &c) && z(&s, &c) > z(&s, &b) && z(&s, &b) > z(&s, &a));
    run(
        &mut s,
        "visual.order",
        json!({"page": p0, "visuals": [a, b], "to": "forward"}),
    );
    assert!(z(&s, &d) > z(&s, &b) && z(&s, &b) > z(&s, &a) && z(&s, &a) > z(&s, &c));
    run(
        &mut s,
        "visual.order",
        json!({"page": p0, "visuals": [a, b], "to": "backward"}),
    );
    assert!(z(&s, &d) > z(&s, &c) && z(&s, &c) > z(&s, &b) && z(&s, &b) > z(&s, &a));
    fails(
        &mut s,
        "visual.order",
        json!({"page": p0, "visuals": [a, 3], "to": "front"}),
    );
    // removed together, one undo step
    let out = run(
        &mut s,
        "visual.remove",
        json!({"page": p0, "visual": a, "visuals": [b, c]}),
    );
    assert_eq!(out["removed"].as_array().unwrap().len(), 3);
    assert_eq!(report(&s).pages[0].visuals.len(), 1);
    undo(&mut s);
    assert_eq!(report(&s).pages[0].visuals.len(), 4);
    fails(
        &mut s,
        "visual.remove",
        json!({"page": p0, "visual": a, "visuals": ["nope"]}),
    );
    fails(
        &mut s,
        "visual.remove",
        json!({"page": p0, "visual": a, "visuals": "b"}),
    );
    let _ = Position::default();
}

/// `visual.cut`: the copy's clip, the visuals removed in the same step (one undo), pasted back.
#[test]
fn cut_copies_and_removes_in_one_undo_step() {
    let mut s = session();
    let a = chart(&mut s, [10.0, 20.0, 200.0, 100.0]);
    let b = chart(&mut s, [300.0, 20.0, 200.0, 100.0]);
    let p0 = page(&s, 0);
    let out = run(&mut s, "visual.cut", json!({"page": p0, "visuals": [a]}));
    assert_eq!(out["count"], 1);
    assert_eq!(names(&out["removed"]), vec![a.clone()]);
    assert_eq!(report(&s).pages[0].visuals.len(), 1);
    assert!(report(&s).pages[0].visual(&b).is_some());
    undo(&mut s);
    assert_eq!(report(&s).pages[0].visuals.len(), 2, "one undo step");
    run(&mut s, "visual.cut", json!({"page": p0, "visual": a}));
    let p1 = page(&s, 1);
    run(&mut s, "visual.paste", json!({"page": p1, "clip": out["clip"]}));
    assert_eq!(report(&s).pages[1].visuals.len(), 1);
    fails(&mut s, "visual.cut", json!({"page": p0, "visual": "nope"}));
    fails(&mut s, "visual.cut", json!({"page": p0}));
}

/// Removing every member of a group removes the group too (no empty group left); removing
/// some keeps it, refitted to the members left.
#[test]
fn removing_all_members_of_a_group_removes_the_group() {
    let mut s = session();
    let a = chart(&mut s, [10.0, 20.0, 200.0, 100.0]);
    let b = chart(&mut s, [300.0, 20.0, 200.0, 100.0]);
    let c = chart(&mut s, [10.0, 300.0, 200.0, 100.0]);
    let p0 = page(&s, 0);
    let g = run(&mut s, "visual.group", json!({"page": p0, "visuals": [a, b]}));
    let group = g["group"].as_str().unwrap().to_string();
    assert_eq!(pos(&s, 0, &group), (10.0, 20.0, 490.0, 100.0));
    run(&mut s, "visual.remove", json!({"page": p0, "visual": a}));
    assert!(report(&s).pages[0].visual(&group).is_some(), "b is still in it");
    assert_eq!(pos(&s, 0, &group), (300.0, 20.0, 200.0, 100.0), "refitted to b");
    let out = run(&mut s, "visual.remove", json!({"page": p0, "visual": b}));
    assert!(report(&s).pages[0].visual(&group).is_none(), "{out}");
    assert_eq!(report(&s).pages[0].visuals.len(), 1);
    assert!(report(&s).pages[0].visual(&c).is_some());
    undo(&mut s);
    assert!(report(&s).pages[0].visual(&group).is_some());
    assert!(report(&s).pages[0].visual(&b).is_some());
}

/// A 1×1 PNG and a small SVG, in base64.
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
const SVG: &str = "PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSI0MCIgaGVpZ2h0PSIyMCI+PHJlY3Qgd2lkdGg9IjQwIiBoZWlnaHQ9IjIwIiBmaWxsPSIjMTE4REZGIi8+PC9zdmc+";

/// The image item an image visual shows.
fn shown_item(v: &Visual) -> String {
    v.visual_extra["objects"]["general"][0]["properties"]["imageUrl"]["expr"]["ResourcePackageItem"]["ItemName"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The bytes of the report's image item `item`.
fn item_bytes(s: &Session, item: &str) -> Option<Vec<u8>> {
    report(s)
        .resources
        .iter()
        .find(|r| r.path.ends_with(&format!("/{item}")))
        .map(|r| r.bytes.clone())
}

/// An image visual pasted into another report keeps its picture: the clip carries the image and
/// the target registers it, reused when it has the same image, under a file of its own when
/// another image has its name; a clip image that is not an image is left out with a warning,
/// and malformed `images` are refused.
#[test]
fn pasted_images_keep_their_picture_in_another_report() {
    let mut a = session();
    let pa = page(&a, 0);
    let out = run(
        &mut a,
        "image.add",
        json!({"page": pa, "data": PNG, "name": "logo.png"}),
    );
    let img = out["visual"].as_str().unwrap().to_string();
    let item = out["resource"].as_str().unwrap().to_string();
    let copy = run(&mut a, "visual.copy", json!({"page": pa, "visual": img}));
    assert_eq!(copy["images"], 1, "{copy}");
    // another report, without the image
    let mut b = session();
    let pb = page(&b, 0);
    let pasted = run(&mut b, "visual.paste", json!({"page": pb, "clip": copy["clip"]}));
    let first = shown_item(visual(&b, 0, &names(&pasted["visuals"])[0]));
    assert!(
        item_bytes(&b, &first).is_some(),
        "the picture came with the visual"
    );
    assert_eq!(item_bytes(&b, &first), item_bytes(&a, &item));
    // pasted again: the same file
    let files = report(&b).resources.len();
    let again = run(&mut b, "visual.paste", json!({"page": pb, "clip": copy["clip"]}));
    assert_eq!(shown_item(visual(&b, 0, &names(&again["visuals"])[0])), first);
    assert_eq!(report(&b).resources.len(), files);
    // another image under that name: the pasted one gets a file of its own
    let mut clip = copy["clip"].clone();
    clip["images"][0]["data"] = json!(SVG);
    let other = run(&mut b, "visual.paste", json!({"page": pb, "clip": clip}));
    let second = shown_item(visual(&b, 0, &names(&other["visuals"])[0]));
    assert_ne!(second, first);
    assert!(second.ends_with(".svg"), "{second}");
    assert!(item_bytes(&b, &second).is_some_and(|x| x.starts_with(b"<svg")));
    assert_eq!(
        item_bytes(&b, &first),
        item_bytes(&a, &item),
        "the first is unchanged"
    );
    // hostile images
    clip["images"][0]["data"] = json!("bm90IGFuIGltYWdl");
    let bad = run(&mut b, "visual.paste", json!({"page": pb, "clip": clip}));
    assert!(bad["warnings"].to_string().contains("not pasted"), "{bad}");
    clip["images"][0]["data"] = json!("@@@");
    let e = fails(&mut b, "visual.paste", json!({"page": pb, "clip": clip}));
    assert!(e.contains("base64"), "{e}");
    clip["images"] = json!(3);
    let e = fails(&mut b, "visual.paste", json!({"page": pb, "clip": clip}));
    assert!(e.contains("must be a list"), "{e}");
}
