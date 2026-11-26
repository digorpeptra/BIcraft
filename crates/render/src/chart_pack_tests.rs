//! The chart pack's layouts on their own: the rules each drawing keeps and hostile input. The
//! comparison with the TypeScript is the engine's golden test (tests/report/chart_pack_tests.rs).

use serde_json::{Value, json};

use super::*;

fn prims(kind: &str, input: Value, w: f64, h: f64) -> Vec<Value> {
    // the Modern card behind the plot is no mark
    layout(kind, &input, w, h)
        .prims
        .iter()
        .map(Prim::json)
        .filter(|p| p["role"] != "card")
        .collect()
}

fn num(p: &Value, k: &str) -> f64 {
    p[k].as_f64().unwrap()
}

/// A node's box of the organization chart (not the 4 px accent strip Modern adds to it).
fn org_box(p: &Value) -> bool {
    p["t"] == "rect" && p["w"] != 4.0 && p["role"] != "card"
}

#[test]
fn org_boxes_of_a_level_never_overlap_and_parents_centre_over_children() {
    let mut rows = vec![json!(["ceo", null, "Chief", 10])];
    for i in 0..4 {
        rows.push(json!([format!("vp{i}"), "ceo", format!("VP {i}"), i]));
        for j in 0..3 {
            rows.push(json!([
                format!("m{i}{j}"),
                format!("vp{i}"),
                format!("Manager {i}.{j}"),
                null
            ]));
        }
    }
    let out = layout("org", &json!({"rows": rows}), 900.0, 400.0);
    let boxes: Vec<Value> = out.prims.iter().map(Prim::json).filter(org_box).collect();
    assert_eq!(boxes.len(), 17);
    for a in &boxes {
        for b in &boxes {
            if a != b && num(a, "y") == num(b, "y") {
                let (l, r) = if num(a, "x") < num(b, "x") { (a, b) } else { (b, a) };
                assert!(num(l, "x") + num(l, "w") <= num(r, "x") + 0.01, "{l} {r}");
            }
        }
    }
    // the root is centred between its first and last child
    let centre = |p: &Value| num(p, "x") + num(p, "w") / 2.0;
    let root = &boxes[0];
    let kids: Vec<&Value> = boxes
        .iter()
        .filter(|b| num(b, "y") > num(root, "y") && num(b, "y") < 200.0)
        .collect();
    assert_eq!(kids.len(), 4);
    let mid = (centre(kids[0]) + centre(kids[3])) / 2.0;
    assert!((centre(root) - mid).abs() < 0.02, "{} {mid}", centre(root));
    // each node is a hit; the root and the VPs carry a toggle
    assert_eq!(out.hits.iter().filter(|h| h["toggle"] != json!(true)).count(), 17);
    assert_eq!(out.hits.iter().filter(|h| h["toggle"] == json!(true)).count(), 5);
}

#[test]
fn org_left_to_right_puts_levels_across_and_parents_level_with_their_children() {
    let mut rows = vec![json!(["ceo", null, "Chief", 10])];
    for i in 0..3 {
        rows.push(json!([format!("vp{i}"), "ceo", format!("VP {i}"), i]));
        for j in 0..2 {
            rows.push(json!([format!("m{i}{j}"), format!("vp{i}"), "M", null]));
        }
    }
    let input = json!({"rows": rows, "options": {"leftToRight": true}});
    let out = layout("org", &input, 600.0, 500.0);
    let boxes: Vec<Value> = out.prims.iter().map(Prim::json).filter(org_box).collect();
    assert_eq!(boxes.len(), 10);
    // three levels: three distinct lefts, the root's the smallest; boxes of a level do not overlap
    let mut lefts: Vec<f64> = boxes.iter().map(|b| num(b, "x")).collect();
    lefts.sort_by(f64::total_cmp);
    lefts.dedup();
    assert_eq!(lefts.len(), 3, "{lefts:?}");
    assert_eq!(num(&boxes[0], "x"), lefts[0]);
    for a in &boxes {
        for b in &boxes {
            if a != b && num(a, "x") == num(b, "x") {
                let (t, u) = if num(a, "y") < num(b, "y") { (a, b) } else { (b, a) };
                assert!(num(t, "y") + num(t, "h") <= num(u, "y") + 0.01, "{t} {u}");
            }
        }
    }
    // the root's middle between its first and last child's
    let middle = |p: &Value| num(p, "y") + num(p, "h") / 2.0;
    let kids: Vec<&Value> = boxes.iter().filter(|b| num(b, "x") == lefts[1]).collect();
    assert_eq!(kids.len(), 3);
    let mid = (middle(kids[0]) + middle(kids[2])) / 2.0;
    assert!((middle(&boxes[0]) - mid).abs() < 0.02);
    // the root's button sits on its box's right edge, at its middle
    let root = &boxes[0];
    let button = out
        .hits
        .iter()
        .find(|h| h["toggle"] == json!(true))
        .and_then(|h| h["prim"].as_u64())
        .and_then(|i| out.prims.get(i as usize))
        .map(Prim::json)
        .unwrap();
    assert!(
        (num(&button, "cx") - (num(root, "x") + num(root, "w"))).abs() < 0.02,
        "{button}"
    );
    assert!((num(&button, "cy") - middle(root)).abs() < 0.02, "{button}");
    // top-down by default: one top for the root's level
    let td = layout("org", &json!({"rows": rows}), 600.0, 500.0);
    let tops: Vec<f64> = td
        .prims
        .iter()
        .map(Prim::json)
        .filter(org_box)
        .map(|b| num(&b, "y"))
        .collect();
    assert_eq!(tops.iter().filter(|y| **y == tops[0]).count(), 1);
}

#[test]
fn mekko_series_take_their_colour_option_else_the_looks_palette() {
    let rows = json!([["N", "x", 30], ["N", "y", 10], ["N", "z", 5]]);
    let fills = |o: Value| -> Vec<String> {
        let input =
            json!({"rows": rows, "options": o, "theme": {"colors": ["#3B5BA9", "#C0504D", "#9BBB59"]}});
        layout("mekko", &input, 400.0, 300.0)
            .prims
            .iter()
            .map(Prim::json)
            .filter(|p| p["t"] == "rect" && p["role"] != "card" && num(p, "w") > 10.0)
            .map(|p| p["fill"].as_str().unwrap().to_string())
            .collect()
    };
    let theme = fills(json!({"styleGradient": false}));
    assert_eq!(theme, ["#3B5BA9", "#C0504D", "#9BBB59"]);
    let set =
        fills(json!({"styleGradient": false, "color1": "#112233", "color3": "#aa5500", "color2": "red"}));
    assert_eq!(set, ["#112233", "#C0504D", "#AA5500"]);
}

#[test]
fn org_collapse_hides_the_subtree_and_levels_set_the_default() {
    let rows = json!([
        ["a", "", "A", 1],
        ["b", "a", "B", 2],
        ["c", "b", "C", 3],
        ["d", "a", "D", 4]
    ]);
    let count = |o: Value| {
        prims("org", json!({"rows": rows, "options": o}), 400.0, 300.0)
            .iter()
            .filter(|p| org_box(p))
            .count()
    };
    assert_eq!(count(json!({})), 4);
    assert_eq!(count(json!({"toggled": ["b"]})), 3);
    assert_eq!(count(json!({"toggled": ["a"]})), 1);
    assert_eq!(count(json!({"levels": 2})), 3);
    assert_eq!(count(json!({"levels": 2, "toggled": ["b"]})), 4);
    assert_eq!(count(json!({"levels": 1})), 1);
}

#[test]
fn org_cycles_and_duplicates_still_draw_every_node_once() {
    // a ↔ b is a cycle (no root): the first of it becomes a root; a repeated id is skipped
    let rows = json!([
        ["a", "b", "A"],
        ["b", "a", "B"],
        ["a", "x", "again"],
        ["c", "c", "C"]
    ]);
    let out = prims("org", json!({"rows": rows}), 300.0, 200.0);
    assert_eq!(out.iter().filter(|p| org_box(p)).count(), 3);
}

#[test]
fn venn_counts_each_region_and_keeps_the_three_largest_sets() {
    let rows = json!([
        ["1", "A"],
        ["2", "A"],
        ["3", "A"],
        ["3", "B"],
        ["4", "B"],
        ["5", "C"],
        ["3", "C"],
        ["1", "C"],
        ["9", "D"],
        ["1", "A"]
    ]);
    let out = layout("venn", &json!({"rows": rows}), 400.0, 300.0);
    let regions: Vec<(u64, u64)> = out
        .hits
        .iter()
        .filter(|h| h.get("region").is_some())
        .map(|h| (h["region"].as_u64().unwrap(), h["n"].as_u64().unwrap()))
        .collect();
    // A = {1,2,3}, B = {3,4}, C = {5,3,1}: A only {2}, B only {4}, C only {5}, A and C {1}, all {3}
    assert_eq!(
        regions,
        vec![(1, 1), (2, 1), (3, 0), (4, 1), (5, 1), (6, 0), (7, 1)]
    );
    let texts: Vec<String> = out
        .prims
        .iter()
        .map(Prim::json)
        .filter(|p| p["t"] == "text")
        .map(|p| p["text"].as_str().unwrap().to_string())
        .collect();
    assert!(texts.contains(&"1 more set not drawn".to_string()), "{texts:?}");
    // the layout never depends on the counts: other memberships, same circles
    let circles = |rows: Value| {
        prims("venn", json!({"rows": rows}), 400.0, 300.0)
            .into_iter()
            .filter(|p| p["t"] == "circle")
            .collect::<Vec<_>>()
    };
    assert_eq!(
        circles(json!([["1", "A"], ["2", "B"], ["3", "C"]])),
        circles(json!([["1", "A"], ["1", "B"], ["1", "C"], ["2", "A"]]))
    );
}

#[test]
fn mekko_areas_are_shares_of_the_whole() {
    let rows = json!([
        ["N", "x", 30],
        ["N", "y", 10],
        ["S", "x", 20],
        ["S", "y", 40],
        ["S", "z", -5]
    ]);
    let out = layout("mekko", &json!({"rows": rows}), 400.0, 300.0);
    let segs: Vec<Value> = out
        .prims
        .iter()
        .map(Prim::json)
        .filter(|p| p["t"] == "rect" && num(p, "w") > 10.0 && p["role"] != "card")
        .collect();
    assert_eq!(segs.len(), 4);
    let area = |p: &Value| (num(p, "w") + 1.0) * num(p, "h");
    let total: f64 = segs.iter().map(area).sum();
    // N|x is 30 of 100
    assert!(
        (area(&segs[0]) / total - 0.3).abs() < 0.01,
        "{}",
        area(&segs[0]) / total
    );
    assert_eq!(out.hits.len(), 4);
}

#[test]
fn calendar_draws_every_day_of_the_years_with_months_in_the_language() {
    let rows = json!([
        ["2024-02-29", 5],
        ["2024-03-01T10:00:00", 1],
        ["2024-03-01T18:00:00", 2],
        ["bad", 3]
    ]);
    let out = layout("calendar", &json!({"rows": rows, "lang": "fr"}), 900.0, 200.0);
    let cells = out
        .prims
        .iter()
        .map(Prim::json)
        .filter(|p| p["t"] == "rect" && num(p, "w") > 0.0 && p["role"] != "card")
        .count();
    // every day of 2024, and the legend's five steps
    assert_eq!(cells, 366 + 5);
    assert_eq!(out.hits.len(), 2);
    // two rows of one day add up
    assert!(
        out.hits.iter().any(|h| h["value"] == json!(3.0)),
        "{:?}",
        out.hits
    );
    let texts: Vec<String> = out
        .prims
        .iter()
        .map(Prim::json)
        .filter_map(|p| p["text"].as_str().map(str::to_string))
        .collect();
    assert!(
        texts.contains(&"févr.".to_string()) && texts.contains(&"2024".to_string()),
        "{texts:?}"
    );
    assert_eq!(calendar::weekday(0), 3);
}

#[test]
fn org_compaction_packs_narrow_subtrees_without_overlap() {
    // a chain whose four leaves fit under the chain's neighbours' gap
    let rows = json!([
        ["r", null, "R"],
        ["a", "r", "A"],
        ["a1", "a", "A1"],
        ["a11", "a1", "x"],
        ["a12", "a1", "x"],
        ["a13", "a1", "x"],
        ["a14", "a1", "x"],
        ["b", "r", "B"],
        ["b1", "b", "x"],
        ["b2", "b", "x"],
        ["b3", "b", "x"],
        ["b4", "b", "x"]
    ]);
    let boxes = |o: Value| -> Vec<Value> {
        prims("org", json!({"rows": rows, "options": o}), 400.0, 300.0)
            .into_iter()
            .filter(org_box)
            .collect()
    };
    let compact = boxes(json!({}));
    let slots = boxes(json!({"compact": false}));
    assert_eq!((compact.len(), slots.len()), (12, 12));
    // 6.5 units instead of 8 leaves: wider boxes
    let ratio = num(&compact[0], "w") / num(&slots[0], "w");
    assert!((ratio - 8.0 / 6.5).abs() < 0.01, "{ratio}");
    for list in [&compact, &slots] {
        for a in list.iter() {
            for b in list.iter() {
                if a != b && num(a, "y") == num(b, "y") {
                    let (l, r) = if num(a, "x") < num(b, "x") { (a, b) } else { (b, a) };
                    assert!(num(l, "x") + num(l, "w") <= num(r, "x") + 0.01, "{l} {r}");
                }
            }
        }
    }
}

#[test]
fn venn_proportional_areas_follow_the_counts() {
    // A has 9 items, B 6, 3 shared
    let rows: Vec<Value> = (1..=9)
        .map(|i| json!([format!("{i}"), "A"]))
        .chain((7..=12).map(|i| json!([format!("{i}"), "B"])))
        .collect();
    let c: Vec<Value> = prims(
        "venn",
        json!({"rows": rows, "options": {"proportional": true}}),
        400.0,
        300.0,
    )
    .into_iter()
    .filter(|p| p["t"] == "circle")
    .collect();
    assert_eq!(c.len(), 2);
    let (ra, rb) = (num(&c[0], "r"), num(&c[1], "r"));
    assert!(((ra * ra) / (rb * rb) - 1.5).abs() < 0.01, "{ra} {rb}");
    // the overlap's area is 3 / 9 of A's
    let d = num(&c[1], "cx") - num(&c[0], "cx");
    let (ca, cb) = (
        (d * d + ra * ra - rb * rb) / (2.0 * d * ra),
        (d * d + rb * rb - ra * ra) / (2.0 * d * rb),
    );
    let k = (-d + ra + rb) * (d + ra - rb) * (d - ra + rb) * (d + ra + rb);
    let lens = ra * ra * ca.acos() + rb * rb * cb.acos() - 0.5 * k.sqrt();
    assert!((lens / (std::f64::consts::PI * ra * ra) - 1.0 / 3.0).abs() < 0.01);
    // no shared item: touching; all shared: one inside the other
    assert!((venn::apart(2.0, 1.0, 0.0) - 3.0).abs() < 1e-12);
    assert!((venn::apart(2.0, 1.0, 1.0) - 1.0).abs() < 1e-12);
}

#[test]
fn calendar_months_only_and_sunday_first() {
    // 2024-03-11 to 2024-05-21: March, April and May drawn, 92 days
    let rows: Vec<Value> = (0..72)
        .map(|i| {
            let (y, m, d) = crate::spec::civil_from_days(19_793 + i);
            json!([format!("{y:04}-{m:02}-{d:02}"), 1])
        })
        .collect();
    let cells = |o: Value| {
        prims("calendar", json!({"rows": rows, "options": o}), 600.0, 160.0)
            .into_iter()
            .filter(|p| p["t"] == "rect" && num(p, "w") != 10.0)
            .collect::<Vec<_>>()
    };
    assert_eq!(cells(json!({"showLegend": false})).len(), 366);
    let months = cells(json!({"monthsOnly": true, "showLegend": false}));
    assert_eq!(months.len(), 31 + 30 + 31);
    // 2024-03-01 was a Friday: row 4 Monday first, row 5 Sunday first
    let first_y = |o: Value| {
        let c = cells(o);
        num(&c[0], "y")
    };
    let monday = first_y(json!({"monthsOnly": true, "showLegend": false}));
    let sunday = first_y(json!({"monthsOnly": true, "sundayFirst": true, "showLegend": false}));
    let c = num(&months[1], "y") - num(&months[0], "y");
    assert!((sunday - monday - c).abs() < 0.02, "{monday} {sunday} {c}");
}

#[test]
fn hostile_input_never_panics() {
    let odd = [
        json!(null),
        json!({}),
        json!({"rows": 5}),
        json!({"rows": [1, "x", null, [], [[]], [{"a": 1}], [1e308, -1e308, 1e308, 1e308]]}),
        json!({"rows": [["a", "a", "a", "a"]], "options": {"fontSize": 1e300, "levels": -5, "toggled": "x", "color": "#GGGGGG"}}),
        json!({"rows": [["-271821-04-20", 1], [8.64e15, 1], [-8.64e15, 2], ["9999-12-31", 1e308]], "options": []}),
        json!({"rows": [["-271821-04-20", 1], ["275760-09-13", 1], ["b", "b"], ["a", "b"]], "options": {"compact": true, "proportional": true, "monthsOnly": true, "sundayFirst": true, "sortColumns": true, "widthAxis": true}}),
        json!({"rows": [["a", "b"], ["b", "a"], ["c", "a"], ["d", "c"]], "options": {"compact": false, "proportional": true}}),
    ];
    for kind in KINDS {
        for input in &odd {
            for (w, h) in [
                (0.0, 0.0),
                (1.0, 1.0),
                (-5.0, 10.0),
                (1e6, 1e6),
                (f64::NAN, 50.0),
                (300.0, 200.0),
            ] {
                let _ = layout(kind, input, w, h);
            }
        }
        // the caps hold
        let many: Vec<Value> = (0..20_000)
            .map(|i| json!([format!("{i}"), format!("{}", i / 3), "x", i]))
            .collect();
        let out = layout(kind, &json!({"rows": many}), 2000.0, 2000.0);
        assert!(out.prims.len() < 30_000, "{kind}: {}", out.prims.len());
    }
    assert!(layout("other", &json!({}), 10.0, 10.0).prims.is_empty());
}

fn sankey_nodes(out: &Out) -> Vec<(String, Value)> {
    out.hits
        .iter()
        .filter(|h| h.get("node").is_some())
        .map(|h| {
            let p = out.prims[h["prim"].as_u64().unwrap() as usize].json();
            (h["name"].as_str().unwrap().to_string(), p)
        })
        .collect()
}

fn sankey_node(out: &Out, name: &str) -> Value {
    sankey_nodes(out)
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, p)| p)
        .unwrap_or_else(|| panic!("no node {name}"))
}

#[test]
fn sankey_chains_go_to_later_columns_and_heights_follow_the_flow() {
    let rows = json!([
        ["A", "B", 10],
        ["B", "C", 6],
        ["A", "C", 4],
        ["D", "C", 30],
        ["A", "E", 5]
    ]);
    let out = layout("sankey", &json!({"rows": rows}), 600.0, 300.0);
    let x = |n: &str| num(&sankey_node(&out, n), "x");
    let h = |n: &str| num(&sankey_node(&out, n), "h");
    // A and D have no incoming link; B follows A; C follows B (longest path), E follows A
    assert_eq!(x("A"), x("D"));
    assert!(
        x("B") > x("A") && x("C") > x("B"),
        "{} {} {}",
        x("A"),
        x("B"),
        x("C")
    );
    assert_eq!(x("E"), x("B"));
    // heights are max(in, out): A 19 out, B 10, C 40 in, D 30, E 5
    let unit = h("E") / 5.0;
    for (n, v) in [("A", 19.0), ("B", 10.0), ("C", 40.0), ("D", 30.0)] {
        assert!((h(n) - v * unit).abs() < 0.05, "{n}: {} vs {}", h(n), v * unit);
    }
    // one ribbon per link, each a closed band of 34 points a column it spans, inside the visual
    let ribbons: Vec<Value> = out
        .hits
        .iter()
        .filter(|h| h.get("link").is_some())
        .map(|h| out.prims[h["prim"].as_u64().unwrap() as usize].json())
        .collect();
    assert_eq!(ribbons.len(), 5);
    for r in &ribbons {
        assert_eq!(r["t"], "poly");
        let pts = r["points"].as_array().unwrap();
        assert!(!pts.is_empty() && pts.len() % 34 == 0, "{}", pts.len());
        for p in pts {
            let (px, py) = (p[0].as_f64().unwrap(), p[1].as_f64().unwrap());
            assert!((0.0..=600.0).contains(&px) && (0.0..=300.0).contains(&py), "{p}");
        }
    }
    // no node of a column overlaps another (the padding stays between them)
    for (a, pa) in sankey_nodes(&out) {
        for (b, pb) in sankey_nodes(&out) {
            if a < b && num(&pa, "x") == num(&pb, "x") {
                let (t, u) = if num(&pa, "y") < num(&pb, "y") {
                    (&pa, &pb)
                } else {
                    (&pb, &pa)
                };
                assert!(num(t, "y") + num(t, "h") <= num(u, "y") + 0.02, "{a} {b}");
            }
        }
    }
}

#[test]
fn sankey_breaks_cycles_and_says_so() {
    let rows = json!([["A", "B", 3], ["B", "C", 2], ["C", "A", 1], ["S", "S", 4]]);
    let out = layout("sankey", &json!({"rows": rows, "lang": "fr"}), 400.0, 200.0);
    // the walk from A breaks C → A; the self link S → S leaves S without flow (not drawn)
    let names: Vec<String> = sankey_nodes(&out).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["A", "B", "C"]);
    assert_eq!(out.hits.iter().filter(|h| h.get("link").is_some()).count(), 2);
    let texts: Vec<Value> = out
        .prims
        .iter()
        .map(Prim::json)
        .filter(|p| p["t"] == "text")
        .collect();
    assert!(
        texts
            .iter()
            .any(|t| t["text"] == "2 liens en boucle non dessinés"),
        "{texts:?}"
    );
    let out = layout(
        "sankey",
        &json!({"rows": [["A", "B", 1], ["B", "A", 1]]}),
        400.0,
        200.0,
    );
    let texts: Vec<Value> = out
        .prims
        .iter()
        .map(Prim::json)
        .filter(|p| p["t"] == "text")
        .collect();
    assert!(
        texts.iter().any(|t| t["text"] == "1 link in a cycle not drawn"),
        "{texts:?}"
    );
}

#[test]
fn sankey_sweeps_reduce_crossings() {
    // seen order puts Z, Y, X in the second column; A links to Z and X, B to Y: X moves above Y
    let rows = json!([["A", "Z", 1], ["B", "Y", 5], ["A", "X", 5]]);
    let out = layout("sankey", &json!({"rows": rows}), 400.0, 300.0);
    let y = |n: &str| num(&sankey_node(&out, n), "y");
    assert!(y("A") < y("B"));
    assert!(y("X") < y("Y"), "{} {}", y("X"), y("Y"));
    // and it is deterministic
    assert_eq!(out, layout("sankey", &json!({"rows": rows}), 400.0, 300.0));
}

#[test]
fn sankey_options_and_caps() {
    let rows = json!([["A", "B", 2], ["A", "C", 1]]);
    let o = json!({"linkColor": "fixed", "linkFill": "#123456", "linkOpacity": 70, "nodeWidth": 20, "showLabels": false});
    let out = layout("sankey", &json!({"rows": rows, "options": o}), 400.0, 200.0);
    let prims: Vec<Value> = out.prims.iter().map(Prim::json).collect();
    assert!(
        prims
            .iter()
            .filter(|p| p["t"] == "poly")
            .all(|p| p["fill"] == "#123456" && p["opacity"] == 0.7)
    );
    assert!(
        prims
            .iter()
            .filter(|p| p["t"] == "rect" && p["role"] != "card")
            .all(|p| p["w"] == 20.0)
    );
    assert!(!prims.iter().any(|p| p["t"] == "text"));
    let dest = layout(
        "sankey",
        &json!({"rows": rows, "options": {"linkColor": "destination"}}),
        400.0,
        200.0,
    );
    let fills: Vec<Value> = dest
        .prims
        .iter()
        .map(Prim::json)
        .filter(|p| p["t"] == "poly")
        .map(|p| p["fill"].clone())
        .collect();
    assert_eq!(
        fills,
        [
            json!(crate::look::BICRAFT_PALETTE[1]),
            json!(crate::look::BICRAFT_PALETTE[2])
        ]
    );
    // 20,000 distinct links: rows, nodes and links stay capped
    let many: Vec<Value> = (0..20_000)
        .map(|i| json!([format!("s{}", i % 150), format!("t{i}"), 1 + i % 7]))
        .collect();
    let out = layout("sankey", &json!({"rows": many}), 2000.0, 2000.0);
    assert!(out.hits.iter().filter(|h| h.get("node").is_some()).count() <= sankey::MAX_NODES);
    assert!(out.hits.iter().filter(|h| h.get("link").is_some()).count() <= sankey::MAX_LINKS);
}

/// `n` scatter rows: a name, two groups, x = i and y = (i * 7) mod 101.
fn scatter_input(n: usize, options: Value) -> Value {
    let rows: Vec<Value> = (0..n)
        .map(|i| {
            json!([
                format!("p{i}"),
                if i % 2 == 0 { "a" } else { "b" },
                i,
                (i * 7) % 101
            ])
        })
        .collect();
    json!({"rows": rows, "options": options})
}

fn texts(out: &Out) -> Vec<String> {
    out.prims
        .iter()
        .map(Prim::json)
        .filter(|p| p["t"] == "text")
        .filter_map(|p| p["text"].as_str().map(str::to_string))
        .collect()
}

fn stroked(p: &Value) -> bool {
    p.get("stroke")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty())
}

#[test]
fn scatter_draws_every_point_up_to_the_cap_and_notes_the_rest() {
    let out = layout(
        "scatter",
        &scatter_input(1500, json!({"density": "off"})),
        600.0,
        400.0,
    );
    let points = out.hits.iter().filter(|h| h.get("group").is_some()).count();
    assert_eq!(points, 1500, "every point drawn, none sampled");
    assert!(!texts(&out).iter().any(|t| t.starts_with("First")));
    let many = layout(
        "scatter",
        &scatter_input(12_000, json!({"density": "off"})),
        600.0,
        400.0,
    );
    let points = many.hits.iter().filter(|h| h.get("group").is_some()).count();
    assert_eq!(points, scatter::MAX_POINTS);
    assert!(
        texts(&many).iter().any(|t| t == "First 10,000 of 12,000 points"),
        "{:?}",
        texts(&many)
    );
}

#[test]
fn scatter_switches_to_density_cells_above_the_threshold() {
    let below = layout("scatter", &scatter_input(1500, json!({})), 600.0, 400.0);
    assert!(below.hits.iter().all(|h| h.get("cell").is_none()));
    let above = layout("scatter", &scatter_input(2500, json!({})), 600.0, 400.0);
    let cells: Vec<&Value> = above.hits.iter().filter(|h| h.get("cell").is_some()).collect();
    assert!(!cells.is_empty());
    assert!(
        above.hits.iter().all(|h| h.get("group").is_none()),
        "no single points"
    );
    let n: u64 = cells.iter().map(|h| h["n"].as_u64().unwrap()).sum();
    assert_eq!(n, 2500, "every point counted in one cell");
    assert!(
        cells
            .iter()
            .all(|h| h["rows"].as_array().unwrap().len() <= scatter::MAX_CELL_ROWS)
    );
    assert!(texts(&above).iter().any(|t| t.contains("density view")));
    // forced off: points again; a lower threshold: cells
    let off = layout(
        "scatter",
        &scatter_input(2500, json!({"density": "off"})),
        600.0,
        400.0,
    );
    assert!(off.hits.iter().all(|h| h.get("cell").is_none()));
    let low = layout(
        "scatter",
        &scatter_input(500, json!({"densityThreshold": 100})),
        600.0,
        400.0,
    );
    assert!(low.hits.iter().any(|h| h.get("cell").is_some()));
}

#[test]
fn scatter_markers_are_filled_bordered_or_both() {
    let marks = |o: Value| -> Vec<Value> {
        let out = layout("scatter", &scatter_input(20, o), 600.0, 400.0);
        out.hits
            .iter()
            .filter(|h| h.get("group").is_some())
            .map(|h| out.prims[h["prim"].as_u64().unwrap() as usize].json())
            .collect()
    };
    let both = marks(json!({}));
    assert!(
        both.iter()
            .all(|p| p["t"] == "circle" && stroked(p) && num(p, "opacity") > 0.0)
    );
    let border = marks(json!({"markerFill": false}));
    assert!(border.iter().all(|p| stroked(p) && num(p, "opacity") == 0.0));
    let fill = marks(json!({"markerBorder": false}));
    assert!(fill.iter().all(|p| !stroked(p) && num(p, "opacity") > 0.0));
    // neither asked: still filled, never invisible
    let none = marks(json!({"markerFill": false, "markerBorder": false}));
    assert!(none.iter().all(|p| num(p, "opacity") > 0.0));
    // a size field: radii follow it
    let rows: Vec<Value> = (0..5)
        .map(|i| json!([format!("p{i}"), "", i, i, i * i]))
        .collect();
    let out = layout("scatter", &json!({"rows": rows}), 600.0, 400.0);
    let radii: Vec<f64> = out
        .hits
        .iter()
        .map(|h| num(&out.prims[h["prim"].as_u64().unwrap() as usize].json(), "r"))
        .collect();
    assert!(
        radii.iter().any(|r| *r < 4.0) && radii.iter().any(|r| *r > 10.0),
        "{radii:?}"
    );
}

#[test]
fn scatter_trend_quadrants_and_labels() {
    // y = 2x + 1 exactly: R² = 1.00
    let rows: Vec<Value> = (0..10)
        .map(|i| json!([format!("p{i}"), "", i, 2 * i + 1]))
        .collect();
    let o = json!({"trendLine": true, "quadrants": true, "labelTopRight": "Stars", "labelBottomLeft": "Laggards",
                   "labelMode": "top", "labelCount": 3});
    let out = layout("scatter", &json!({"rows": rows, "options": o}), 600.0, 400.0);
    let t = texts(&out);
    assert!(t.iter().any(|s| s == "R² = 1.00"), "{t:?}");
    // each quadrant's label with its share of the points (on y = 2x + 1, half above both means)
    assert!(t.iter().any(|s| s == "Stars · 50%"), "{t:?}");
    assert!(t.iter().any(|s| s == "Laggards · 50%"), "{t:?}");
    assert!(t.iter().any(|s| s == "0%"), "{t:?}");
    // the three highest points by y are named
    for name in ["p9", "p8", "p7"] {
        assert!(t.iter().any(|s| s == name), "{name} in {t:?}");
    }
    assert!(!t.iter().any(|s| s == "p6"), "{t:?}");
    // selected rows: their names only
    let sel = layout(
        "scatter",
        &json!({"rows": rows, "selected": [2, 4, 99, -1, "x"], "options": {"labelMode": "selected"}}),
        600.0,
        400.0,
    );
    let t = texts(&sel);
    assert!(
        t.iter().any(|s| s == "p2") && t.iter().any(|s| s == "p4"),
        "{t:?}"
    );
    assert!(!t.iter().any(|s| s == "p3"), "{t:?}");
    // marginal strips add marks outside the plot
    let plain = layout("scatter", &json!({"rows": rows}), 600.0, 400.0)
        .prims
        .len();
    for kind in ["histogram", "box"] {
        let m = layout(
            "scatter",
            &json!({"rows": rows, "options": {"marginals": kind}}),
            600.0,
            400.0,
        );
        assert!(m.prims.len() > plain, "{kind}");
    }
}

/// Monthly rows from 2024-01: a wave, and a rising plan that crosses it.
fn line_input(months: usize, series: &[&str], options: Value) -> Value {
    let mut rows = Vec::new();
    for s in series {
        for m in 0..months {
            let (y, mo) = (2024 + m / 12, m % 12 + 1);
            let v = 100.0 + 20.0 * ((m as f64) * 0.9).sin();
            rows.push(json!([
                format!("{y}-{mo:02}-01T00:00:00"),
                s,
                v,
                100.0 + m as f64
            ]));
        }
    }
    json!({"rows": rows, "options": options})
}

fn fills(out: &Out, t: &str) -> Vec<String> {
    out.prims
        .iter()
        .map(Prim::json)
        .filter(|p| p["t"] == t)
        .filter_map(|p| p["fill"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn line_band_is_coloured_by_which_value_is_above() {
    let out = layout("linearea", &line_input(12, &[""], json!({})), 600.0, 300.0);
    let f = fills(&out, "poly");
    assert!(f.iter().any(|c| c == "#0E9F84"), "{f:?}");
    assert!(f.iter().any(|c| c == "#E23D4F"), "{f:?}");
    let off = layout(
        "linearea",
        &line_input(12, &[""], json!({"band": false})),
        600.0,
        300.0,
    );
    assert!(fills(&off, "poly").is_empty());
}

#[test]
fn line_threshold_area_and_forecast() {
    let t = layout(
        "linearea",
        &line_input(
            12,
            &[""],
            json!({"band": false, "threshold": true, "thresholdValue": 105, "thresholdColor": "#112233"}),
        ),
        600.0,
        300.0,
    );
    assert!(fills(&t, "poly").iter().any(|c| c == "#112233"));
    let a = layout(
        "linearea",
        &line_input(
            12,
            &[""],
            json!({"band": false, "area": true, "areaFill": "#445566"}),
        ),
        600.0,
        300.0,
    );
    assert!(
        fills(&a, "poly").iter().any(|c| c == "#445566"),
        "the area's own fill"
    );
    // a forecast from 2024-07-01: the rest of the line is dashed (short segments as wide as the
    // line, not the 1 px mark at the cutoff)
    let lines = |o: Value| {
        layout("linearea", &line_input(12, &[""], o), 600.0, 300.0)
            .prims
            .iter()
            .map(Prim::json)
            .filter(|p| p["t"] == "line" && num(p, "width") > 1.5)
            .count()
    };
    let plain = lines(json!({"band": false}));
    assert!(lines(json!({"band": false, "forecastFrom": "2024-07-01"})) > plain + 5);
    // nonsense dates draw as no forecast
    assert_eq!(lines(json!({"band": false, "forecastFrom": "not a date"})), plain);
}

#[test]
fn line_end_labels_name_each_series_and_caps_hold() {
    let out = layout(
        "linearea",
        &line_input(6, &["North", "South"], json!({"band": false})),
        600.0,
        300.0,
    );
    let t = texts(&out);
    assert!(
        t.iter().any(|s| s == "North") && t.iter().any(|s| s == "South"),
        "{t:?}"
    );
    let hits = out.hits.iter().filter(|h| h.get("point").is_some()).count();
    assert_eq!(hits, 12, "a target per point");
    // past MAX_HIT_POINTS: one target per line
    let rows: Vec<Value> = (0..2)
        .flat_map(|s| (0..linearea::MAX_POINTS).map(move |i| json!([i, format!("s{s}"), i % 17])))
        .collect();
    let big = layout("linearea", &json!({"rows": rows}), 800.0, 400.0);
    assert!(big.hits.iter().all(|h| h["line"] == true), "one target per line");
    assert_eq!(big.hits.len(), 2);
    // more series than the cap
    let rows: Vec<Value> = (0..40).map(|i| json!([1, format!("s{i}"), i])).collect();
    let capped = layout("linearea", &json!({"rows": rows}), 800.0, 400.0);
    let series: std::collections::HashSet<u64> =
        capped.hits.iter().filter_map(|h| h["series"].as_u64()).collect();
    assert!(series.len() <= linearea::MAX_SERIES);
}

#[test]
fn line_and_scatter_hostile_cells_never_panic() {
    let odd = [
        json!({"rows": [["2024-01-01", "a", "NaN", 1e308], ["2024-01-02", "a", 1e308, -1e308], ["9999-12-31T23:59:59", null, 5, null], ["-271821-04-20", [], {}, 3]]}),
        json!({"rows": [[1e308, 1e308, 1e308, 1e308, 1e308], [-1e308, "x", -1e308, -1e308, -1e308]], "options": {"quadrants": true, "quadrantAt": "value", "quadrantX": 1e300, "trendLine": true, "marginals": "box"}}),
        json!({"rows": [[1, "a", 1, 1, 1], [1, "a", 1, 1, 1]], "options": {"trendLine": true, "marginals": "histogram", "density": "on", "hexSize": 0}}),
        json!({"rows": [["a", "s", 1, 2]], "options": {"forecastFrom": "today", "colorRule": "above", "threshold": true, "labelMode": "all", "lineWidth": "x"}, "today": "x"}),
        json!({"rows": [["2024-01-01", "s", 1, 2]], "options": {"forecastFrom": "2024-01-01", "labelMode": "minmax", "area": true}, "today": 1e300}),
    ];
    for kind in ["scatter", "linearea"] {
        for input in &odd {
            for (w, h) in [
                (0.0, 0.0),
                (30.0, 30.0),
                (600.0, 300.0),
                (1e7, 10.0),
                (f64::NAN, f64::INFINITY),
            ] {
                let out = layout(kind, input, w, h);
                for p in out.prims.iter().map(Prim::json) {
                    for k in ["x", "y", "cx", "cy", "w", "h", "r"] {
                        if let Some(v) = p.get(k).and_then(Value::as_f64) {
                            assert!(v.is_finite(), "{kind} {input}: {p}");
                        }
                    }
                }
            }
        }
    }
}

/// Another visual's selection that highlights one point of a line draws that point as a dot over
/// the muted line (no point was drawn in full colour in Desktop 2.159.1231.0 nor in BIcraft,
/// 2026-11-25); two neighbours still draw a segment, with markers on the marker stays.
#[test]
fn line_highlight_of_a_lone_point_draws_a_dot() {
    let mut input = line_input(6, &[""], json!({"band": false}));
    let hl: Vec<Value> = (0..6)
        .map(|i| if i == 2 { json!(90) } else { Value::Null })
        .collect();
    input["highlights"] = json!(hl);
    let out = layout("linearea", &input, 600.0, 300.0);
    let full: Vec<Value> = out
        .prims
        .iter()
        .map(Prim::json)
        .filter(|p| p["t"] == "circle" && p["opacity"] == 1.0)
        .collect();
    assert_eq!(full.len(), 1, "{full:?}");
    let mut two = input.clone();
    two["highlights"][3] = json!(80);
    let out = layout("linearea", &two, 600.0, 300.0);
    assert!(
        !out.prims
            .iter()
            .map(Prim::json)
            .any(|p| p["t"] == "circle" && p["opacity"] == 1.0)
    );
}
