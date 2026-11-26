//! Key influencers: the statistics, the estimate on planted data (the planted influencers rank
//! first, noise does not), determinism, bounds and hostile visuals.

use serde_json::{Value, json};

use super::stats::{self, Cell, FieldCells, Goal, What};
use crate::Session;

// --- statistics -------------------------------------------------------------------------------

#[test]
fn distributions_match_textbook_values() {
    // two-sided p of Student's t (tables): t = 2.228, df = 10 → 0.05; t = 1.96, df = ∞ → 0.05
    let p = stats::t_two_sided(2.228, 10.0);
    assert!((p - 0.05).abs() < 1e-3, "{p}");
    let p = stats::t_two_sided(1.959_964, 1e7);
    assert!((p - 0.05).abs() < 1e-4, "{p}");
    assert_eq!(stats::t_two_sided(0.0, 5.0), 1.0);
    assert_eq!(stats::t_two_sided(f64::INFINITY, 5.0), 0.0);
    assert_eq!(stats::t_two_sided(f64::NAN, 5.0), 1.0);
    assert_eq!(stats::t_two_sided(1.0, -1.0), 1.0);
    // I_x(a, b): I_0.5(2, 2) = 0.5, I_0.3(1, 1) = 0.3
    assert!((stats::inc_beta(2.0, 2.0, 0.5) - 0.5).abs() < 1e-12);
    assert!((stats::inc_beta(1.0, 1.0, 0.3) - 0.3).abs() < 1e-12);
    assert!(stats::inc_beta(-1.0, 1.0, 0.3).is_nan());
}

#[test]
fn cells_merge_and_split_exactly() {
    let ys = [1.0, 4.0, 4.0, 7.0, 10.0, 2.0];
    let all = ys.iter().fold(Cell::default(), |a, y| a.merge(&Cell::one(*y)));
    let part = ys[..2]
        .iter()
        .fold(Cell::default(), |a, y| a.merge(&Cell::one(*y)));
    let rest = ys[2..]
        .iter()
        .fold(Cell::default(), |a, y| a.merge(&Cell::one(*y)));
    let r = all.minus(&part);
    assert!((r.n - rest.n).abs() < 1e-12 && (r.sum - rest.sum).abs() < 1e-9 && (r.ss - rest.ss).abs() < 1e-9);
    let m = Cell::from_moments(6.0, all.mean(), all.ss / 6.0);
    assert!((m.ss - all.ss).abs() < 1e-9);
    // an indicator: 3 of 4
    let c = Cell::from_counts(4.0, 3.0);
    assert!((c.mean() - 0.75).abs() < 1e-12 && (c.ss - 0.75).abs() < 1e-12);
    // hostile numbers
    let h = Cell::from_moments(f64::NAN, f64::INFINITY, -1.0);
    assert_eq!(h, Cell::default());
    assert_eq!(Cell::default().minus(&c), Cell::default());
}

#[test]
fn steps_and_bins() {
    assert_eq!(stats::nice_step(0.0), 1.0);
    assert_eq!(stats::nice_step(f64::NAN), 1.0);
    assert_eq!(stats::nice_step(13.0), 10.0);
    assert_eq!(stats::nice_step(2.6), 2.0);
    assert_eq!(stats::nice_step(0.06), 0.05);
    let cells: Vec<(f64, Cell)> = (0..100).map(|i| (f64::from(i), Cell::one(1.0))).collect();
    let bins = stats::bin_numbers(&cells, 5);
    assert_eq!(bins.len(), 5);
    assert!(bins.iter().all(|b| (b.2.n - 20.0).abs() < 1e-9), "{bins:?}");
    assert_eq!((bins[0].0, bins[4].1), (0.0, 99.0));
    assert!(stats::bin_numbers(&[], 5).is_empty());
    assert_eq!(stats::bin_numbers(&[(1.0, Cell::one(2.0))], 5).len(), 1);
}

#[test]
fn a_planted_category_ranks_first_and_noise_is_filtered() {
    // field 0: "A" adds 10; field 1: pure noise (same distribution in each category)
    let mut f0 = Vec::new();
    let mut f1 = Vec::new();
    for (k, shift) in [("A", 10.0), ("B", 0.0), ("C", 0.0)] {
        let mut c = Cell::default();
        for i in 0..60 {
            c = c.merge(&Cell::one(shift + f64::from(i % 7)));
        }
        f0.push((k.to_string(), c));
    }
    for k in ["x", "y", "z"] {
        let mut c = Cell::default();
        for i in 0..60 {
            c = c.merge(&Cell::one(f64::from((i * 3) % 7)));
        }
        f1.push((k.to_string(), c));
    }
    let r = [
        stats::analyse_field(0, &FieldCells::Categories(f0), Goal::Increase),
        stats::analyse_field(1, &FieldCells::Categories(f1), Goal::Increase),
    ];
    let ranked = stats::rank(&r, Goal::Increase, 10);
    assert_eq!(ranked.len(), 1, "{ranked:?}");
    assert_eq!(ranked[0].what, What::Is("A".into()));
    assert!((ranked[0].effect - 10.0).abs() < 1e-9);
    // decrease: B and C lower the target
    let dec = stats::rank(&r, Goal::Decrease, 10);
    assert!(dec.iter().all(|i| i.effect < 0.0 && i.field == 0), "{dec:?}");
}

// --- the estimate through a session --------------------------------------------------------------

/// A deterministic pseudo-random stream in [0, 1).
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
    }
}

/// 1 200 rows: Score = 50 + 20·[Region = North] + 0.5·Age + noise; Churn = Yes mostly on the
/// Web channel; Color is noise.
fn planted_csv() -> String {
    let mut r = Lcg(42);
    let mut csv = String::from("Id,Region,Channel,Color,Age,Score,Churn\n");
    for i in 0..1200 {
        let region = ["North", "South", "East", "West"][(r.next() * 4.0) as usize % 4];
        let channel = ["Web", "Store", "Phone"][(r.next() * 3.0) as usize % 3];
        let color = ["Red", "Green", "Blue"][(r.next() * 3.0) as usize % 3];
        let age = 20 + (r.next() * 40.0) as i64;
        let noise = (r.next() - 0.5) * 10.0;
        let north = if region == "North" { 20.0 } else { 0.0 };
        let score = 50.0 + north + 0.5 * age as f64 + noise;
        let churn = if (channel == "Web" && r.next() < 0.6) || r.next() < 0.1 {
            "Yes"
        } else {
            "No"
        };
        csv.push_str(&format!(
            "{i},{region},{channel},{color},{age},{score:.3},{churn}\n"
        ));
    }
    csv
}

fn session(target: &str, explain: &[&str], objects: Option<Value>) -> Session {
    let mut s = Session::new();
    s.execute("model.new", &json!({"name": "Ki"})).unwrap();
    s.execute("table.import", &json!({"name": "T", "csv": planted_csv()}))
        .unwrap();
    s.execute("report.new", &json!({})).unwrap();
    let page = s.report.as_ref().unwrap().pages[0].display_name.clone();
    let mut fields = vec![json!({"role": "Target", "field": target})];
    fields.extend(explain.iter().map(|f| json!({"role": "ExplainBy", "field": f})));
    let r = s
        .execute(
            "visual.add",
            &json!({"page": page, "visual_type": "keyDriversVisual", "x": 0, "y": 0, "width": 900,
                    "height": 500, "fields": fields, "force": true}),
        )
        .unwrap();
    if let Some(objects) = objects {
        let name = r["visual"]
            .as_str()
            .or(r["name"].as_str())
            .unwrap_or_default()
            .to_string();
        s.execute(
            "visual.format",
            &json!({"page": page, "visual": name, "objects": objects}),
        )
        .unwrap();
    }
    s
}

fn ki(snap: &Value) -> Value {
    snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["type"] == "keyDriversVisual")
        .unwrap()["key_influencers"]
        .clone()
}

fn estimate(s: &mut Session) -> Value {
    s.execute(
        "view.interact",
        &json!({"action": "key_influencers", "mode": "estimate"}),
    )
    .unwrap();
    let limits = crate::snapshot::SnapshotLimits {
        data_budget_ms: 60_000,
        ..Default::default()
    };
    ki(&s.snapshot_with(&limits))
}

#[test]
fn placeholder_describes_the_visual_without_estimating() {
    let s = session("Sum(T[Score])", &["T[Region]", "T[Age]"], None);
    let k = ki(&s.snapshot());
    assert_eq!(k["mode"], "placeholder");
    assert_eq!(k["target"], 0);
    assert_eq!(k["explain_by"], json!([1, 2]));
    assert_eq!(k["target_kind"], "numeric");
    assert_eq!(k["goal"], "increase");
    assert_eq!(k["analysis"], "key_influencers");
    assert!(k.get("estimate").is_none(), "{k}");
    // a categorical target and a saved class
    let s = session(
        "T[Churn]",
        &["T[Channel]"],
        Some(
            json!([{"object": "keyDrivers", "property": "targetValue", "value": "Yes"},
                    {"object": "keyDrivers", "property": "selectedAnalysis", "value": "topProfiles"}]),
        ),
    );
    let k = ki(&s.snapshot());
    assert_eq!(
        (k["target_kind"].as_str(), k["goal"].as_str()),
        (Some("categorical"), Some("class"))
    );
    assert_eq!(k["class"], "Yes");
    assert_eq!(k["analysis"], "top_segments");
}

#[test]
fn planted_numeric_influencers_rank_first() {
    let mut s = session(
        "Sum(T[Score])",
        &["T[Color]", "T[Region]", "T[Age]", "T[Channel]"],
        None,
    );
    let k = estimate(&mut s);
    let e = &k["estimate"];
    assert_eq!(e["status"], "ok", "{e}");
    assert_eq!(e["rows"], 1200);
    let inf = e["influencers"].as_array().unwrap();
    assert!(!inf.is_empty(), "{e}");
    // Region = North (+20) first
    assert_eq!(
        (inf[0]["field"].as_u64(), inf[0]["kind"].as_str()),
        (Some(2), Some("is")),
        "{e}"
    );
    assert_eq!(inf[0]["value"], "North");
    let eff = inf[0]["effect"].as_f64().unwrap();
    assert!((eff - 20.0).abs() < 3.0, "{eff}");
    // Age: + 0.5 per year, given per step
    let age = inf
        .iter()
        .find(|i| i["field"] == 3)
        .expect("Age is an influencer");
    let (step, per) = (age["step"].as_f64().unwrap(), age["effect"].as_f64().unwrap());
    assert!((per / step - 0.5).abs() < 0.1, "{age}");
    // the noise fields are not influencers
    assert!(inf.iter().all(|i| i["field"] != 1 && i["field"] != 4), "{e}");
    // detail bars for every field, segments led by North
    assert_eq!(e["fields"].as_array().unwrap().len(), 4);
    let seg = &e["segments"][0];
    assert!(
        seg["conditions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["field"] == 2 && c["op"] == "is" && c["value"] == "North"),
        "{seg}"
    );
}

#[test]
fn decrease_goal_ranks_the_lowering_groups() {
    let mut s = session(
        "Sum(T[Score])",
        &["T[Region]"],
        Some(json!([{"object": "keyDrivers", "property": "numericTargetSelectedKind", "value": 1}])),
    );
    let k = estimate(&mut s);
    assert_eq!(k["goal"], "decrease");
    let inf = k["estimate"]["influencers"].as_array().unwrap().clone();
    assert!(!inf.is_empty());
    assert!(
        inf.iter()
            .all(|i| i["effect"].as_f64().unwrap() < 0.0 && i["value"] != "North"),
        "{inf:?}"
    );
}

#[test]
fn planted_class_influencer_ranks_first() {
    let mut s = session(
        "T[Churn]",
        &["T[Color]", "T[Channel]", "T[Age]"],
        Some(json!([{"object": "keyDrivers", "property": "targetValue", "value": "Yes"}])),
    );
    let k = estimate(&mut s);
    let e = &k["estimate"];
    assert_eq!(e["status"], "ok", "{e}");
    assert_eq!(e["class"], "Yes");
    let inf = e["influencers"].as_array().unwrap();
    assert_eq!(
        (inf[0]["field"].as_u64(), inf[0]["value"].as_str()),
        (Some(2), Some("Web")),
        "{e}"
    );
    // P(Yes | Web) ≈ 0.64 against ≈ 0.10: about 6 times as likely
    let ratio = inf[0]["effect"].as_f64().unwrap();
    assert!(ratio > 4.0 && ratio < 9.0, "{ratio}");
    assert!(inf.iter().all(|i| i["field"] != 1), "{e}");
    // no saved class: the first class in order ("No")
    let mut s = session("T[Churn]", &["T[Channel]"], None);
    assert_eq!(estimate(&mut s)["estimate"]["class"], "No");
}

#[test]
fn estimates_are_deterministic() {
    let a = estimate(&mut session(
        "Sum(T[Score])",
        &["T[Region]", "T[Age]", "T[Channel]"],
        None,
    ));
    let b = estimate(&mut session(
        "Sum(T[Score])",
        &["T[Region]", "T[Age]", "T[Channel]"],
        None,
    ));
    assert_eq!(a.to_string(), b.to_string());
}

#[test]
fn hostile_visuals_never_fail() {
    // only a target; only explain-by; a measure target without details; an unknown field
    for (target, explain) in [
        ("Sum(T[Score])", vec![]),
        ("T[Nope]", vec!["T[Region]"]),
        ("T[Score]", vec!["T[Nope]", "T[Region]"]),
    ] {
        let mut s = session(target, &explain, None);
        let k = estimate(&mut s);
        assert!(k["estimate"].is_object(), "{k}");
    }
    let mut s = session("Sum(T[Score])", &["T[Region]"], None);
    s.execute(
        "measure.add",
        &json!({"table": "T", "name": "Avg Score", "expression": "AVERAGE(T[Score])"}),
    )
    .unwrap();
    let page = s.report.as_ref().unwrap().pages[0].display_name.clone();
    s.execute(
        "visual.add",
        &json!({"page": page, "visual_type": "keyDriversVisual", "x": 0, "y": 600, "width": 400, "height": 100,
                "fields": [{"role": "Target", "field": "[Avg Score]"}, {"role": "ExplainBy", "field": "T[Region]"}]}),
    )
    .unwrap();
    s.execute(
        "view.interact",
        &json!({"action": "key_influencers", "mode": "estimate"}),
    )
    .unwrap();
    let snap = s.snapshot_with(&crate::snapshot::SnapshotLimits {
        data_budget_ms: 60_000,
        ..Default::default()
    });
    let measure = snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["type"] == "keyDriversVisual")
        .nth(1)
        .unwrap()["key_influencers"]["estimate"]
        .clone();
    assert_eq!(measure["status"], "unsupported", "{measure}");
    // bad modes are refused
    assert!(
        s.execute(
            "view.interact",
            &json!({"action": "key_influencers", "mode": "x"})
        )
        .is_err()
    );
    assert!(
        s.execute("report.render", &json!({"key_influencers": "maybe"}))
            .is_err()
    );
}

#[test]
fn expand_by_makes_rows_observations() {
    let mut s = Session::new();
    s.execute("model.new", &json!({"name": "Ki"})).unwrap();
    s.execute("table.import", &json!({"name": "T", "csv": planted_csv()}))
        .unwrap();
    s.execute(
        "measure.add",
        &json!({"table": "T", "name": "Avg Score", "expression": "AVERAGE(T[Score])"}),
    )
    .unwrap();
    s.execute("report.new", &json!({})).unwrap();
    let page = s.report.as_ref().unwrap().pages[0].display_name.clone();
    s.execute(
        "visual.add",
        &json!({"page": page, "visual_type": "keyDriversVisual", "x": 0, "y": 0, "width": 900, "height": 500,
                "fields": [{"role": "Target", "field": "[Avg Score]"}, {"role": "ExplainBy", "field": "T[Region]"},
                           {"role": "Details", "field": "T[Id]"}]}),
    )
    .unwrap();
    let k = estimate(&mut s);
    let e = &k["estimate"];
    assert_eq!(e["status"], "ok", "{e}");
    assert_eq!(e["rows"], 1200);
    assert_eq!(e["influencers"][0]["value"], "North", "{e}");
}

#[test]
fn the_toggle_is_in_the_snapshot_and_kept_by_reset() {
    let mut s = session("Sum(T[Score])", &["T[Region]"], None);
    assert_eq!(s.snapshot()["interaction"]["key_influencers"], "placeholder");
    s.execute(
        "view.interact",
        &json!({"action": "key_influencers", "mode": "estimate"}),
    )
    .unwrap();
    s.execute("view.interact", &json!({"action": "reset"})).unwrap();
    let snap = s.snapshot();
    assert_eq!(snap["interaction"]["key_influencers"], "estimate");
    assert_eq!(ki(&snap)["mode"], "estimate");
    // the render's parameter wins for that render only
    let r = s
        .execute(
            "report.render",
            &json!({"page": 0, "system_fonts": false, "key_influencers": "placeholder"}),
        )
        .unwrap();
    assert_eq!(r["pages"][0]["visuals"][0]["status"], "placeholder", "{r}");
    let r = s
        .execute("report.render", &json!({"page": 0, "system_fonts": false}))
        .unwrap();
    assert_eq!(r["pages"][0]["visuals"][0]["status"], "estimate", "{r}");
}
