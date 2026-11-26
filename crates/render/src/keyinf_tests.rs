//! The key influencers visual: the placeholder names the fields and says Power BI computes it;
//! the estimate is drawn under its badge; hostile objects never crash.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::*;

fn page(visual: Value) -> Value {
    json!({
        "version": 1,
        "model": {"culture": "en-US", "tables": []},
        "report": {"theme": null, "pages": [{"name": "p1", "display_name": "Page 1", "width": 1000, "height": 560,
                   "visuals": [visual]}]}
    })
}

fn fields() -> Value {
    json!([
        {"role": "Target", "kind": "aggregation", "table": "T", "name": "Score", "function": "Sum", "label": "Score"},
        {"role": "ExplainBy", "kind": "column", "table": "T", "name": "Region", "label": "Region"},
        {"role": "ExplainBy", "kind": "column", "table": "T", "name": "Age", "label": "Age"}
    ])
}

fn visual(ki: Value) -> Value {
    json!({"name": "k", "type": "keyDriversVisual", "x": 10, "y": 10, "width": 900, "height": 500,
           "fields": fields(), "key_influencers": ki,
           "data": {"status": "unsupported", "reason": "visual type `keyDriversVisual` is not previewed"}})
}

fn run(s: &Value) -> PageImage {
    let fonts = FontBook::bundled_only();
    let o = RenderOptions {
        system_fonts: false,
        text_dump: true,
        ..RenderOptions::default()
    };
    render_page(s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &o).unwrap()
}

fn all_text(img: &PageImage) -> String {
    img.texts
        .iter()
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn estimate() -> Value {
    json!({"status": "ok", "method": "bicraft-1", "rows": 1200, "overall": 71.4, "tests": 6, "alpha": 0.008,
           "influencers": [
               {"field": 1, "kind": "is", "value": "North", "value_mean": 89.9, "rest_mean": 69.8, "effect": 20.1, "count": 300, "p": 0.0},
               {"field": 2, "kind": "up", "step": 10, "value_mean": 5.0, "rest_mean": 71.4, "effect": 5.0, "count": 1200, "p": 0.0}],
           "fields": [{"field": 1, "numeric": false, "bars": [
               {"value": "North", "mean": 89.9, "count": 300}, {"value": "East", "mean": 70.1, "count": 300},
               {"value": "South", "mean": 69.9, "count": 300}, {"value": "West", "mean": 69.5, "count": 300}]}],
           "segments": [{"conditions": [{"field": 1, "op": "is", "value": "North"}, {"field": 2, "op": "above", "value": 40}],
                         "mean": 99.0, "count": 150, "share": 0.125}]})
}

#[test]
fn the_placeholder_names_the_fields_and_power_bi() {
    let ki = json!({"analysis": "key_influencers", "target": 0, "explain_by": [1, 2], "details": [],
                    "target_kind": "numeric", "goal": "increase", "class": null, "mode": "placeholder"});
    let img = run(&page(visual(ki)));
    assert_eq!(img.visuals[0].status, "placeholder");
    let t = all_text(&img);
    for want in [
        "Key influencers",
        "Top segments",
        "Score to",
        "Increase",
        "Score",
        "Region, Age",
        "Computed by Power BI",
    ] {
        assert!(t.contains(want), "{want} missing in\n{t}");
    }
    assert!(!t.contains("BIcraft estimate"), "{t}");
    // a categorical target names its class; decrease is named
    let ki = json!({"target": 0, "explain_by": [1], "goal": "class", "class": "Yes", "target_kind": "categorical"});
    let t = all_text(&run(&page(visual(ki))));
    assert!(t.contains("Score to be") && t.contains("Yes"), "{t}");
    let ki = json!({"target": 0, "explain_by": [1], "goal": "decrease"});
    assert!(all_text(&run(&page(visual(ki)))).contains("Decrease"));
    // an estimate that is not there says why
    let ki = json!({"target": 0, "explain_by": [1], "mode": "estimate",
                    "estimate": {"status": "unsupported", "reason": "no Explain by field"}});
    let img = run(&page(visual(ki)));
    assert_eq!(img.visuals[0].status, "placeholder");
    assert!(all_text(&img).contains("no Explain by field"));
    // a snapshot without the object still names the fields from their roles
    let t = all_text(&run(&page(visual(Value::Null))));
    assert!(
        t.contains("Region, Age") && t.contains("Computed by Power BI"),
        "{t}"
    );
}

#[test]
fn the_estimate_is_drawn_under_its_badge() {
    let ki = json!({"analysis": "key_influencers", "target": 0, "explain_by": [1, 2], "goal": "increase",
                    "mode": "estimate", "estimate": estimate()});
    let img = run(&page(visual(ki.clone())));
    assert_eq!(img.visuals[0].status, "estimate");
    let t = all_text(&img);
    for want in [
        "BIcraft estimate",
        "Region is North",
        "20.1",
        "Age goes up 10",
        "Score tends to be higher when Region is North",
        "Average of the other values",
        "Influencers only",
    ] {
        assert!(t.contains(want), "{want} missing in\n{t}");
    }
    // the top segments tab
    let mut seg = ki;
    seg["analysis"] = json!("top_segments");
    let t = all_text(&run(&page(visual(seg))));
    for want in [
        "BIcraft estimate",
        "Segment 1",
        "Region is North",
        "Age is more than 40",
        "150 rows",
    ] {
        assert!(t.contains(want), "{want} missing in\n{t}");
    }
}

#[test]
fn hostile_key_influencers_never_crash() {
    let bad = [
        json!({"target": 999, "explain_by": ["x", -1, 1e300], "goal": 5, "mode": "estimate",
               "estimate": {"status": "ok", "influencers": [{"field": 99, "effect": "NaN"}, 3, null],
                            "fields": [{"field": 99, "bars": [{"mean": 1e308}, {"mean": -1e308}, "x"]}],
                            "segments": [{"conditions": "x", "count": -5}, {"conditions": [{"field": null}]}]}}),
        json!({"estimate": {"status": "ok", "influencers": [], "segments": []}, "analysis": "top_segments"}),
        json!({"estimate": {"status": "ok", "influencers": []}}),
        json!("text"),
        json!([1, 2]),
    ];
    for ki in bad {
        for (w, h) in [(900.0, 500.0), (40.0, 30.0), (3.0, 3.0), (2000.0, 60.0)] {
            let mut v = visual(ki.clone());
            v["width"] = json!(w);
            v["height"] = json!(h);
            let img = run(&page(v));
            assert_eq!(img.visuals.len(), 1);
        }
    }
}
