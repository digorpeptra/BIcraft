//! The filter tests that need the engine's private items (`preview::filters`); the others are the `report`
//! integration test (`tests/report/preview_filters_tests.rs`), under the same module path.

use serde_json::{Value, json};

fn col(t: &str, c: &str) -> Value {
    json!({"Column": {"Expression": {"SourceRef": {"Entity": t}}, "Property": c}})
}

fn lit(v: &str) -> Value {
    json!({"Literal": {"Value": v}})
}

fn where_(cond: Value) -> Value {
    json!({"Version": 2, "From": [], "Where": [{"Condition": cond}]})
}

#[test]
fn slicer_exclusive_date_bound_shows_the_last_included_day() {
    let f = where_(json!({"And": {
        "Left": {"Comparison": {"ComparisonKind": 2, "Left": col("Sales", "Day"),
                                "Right": lit("datetime'2025-01-01T00:00:00'")}},
        "Right": {"Comparison": {"ComparisonKind": 3, "Left": col("Sales", "Day"),
                                 "Right": lit("datetime'2025-10-01T00:00:00'")}}}}));
    let mut v = bicraft_report::Visual {
        visual_type: Some("slicer".into()),
        ..Default::default()
    };
    v.visual_extra.insert(
        "objects".into(),
        json!({"general": [{"properties": {"filter": {"filter": f}}}]}),
    );
    assert_eq!(
        crate::preview::filters::selection_json(&v),
        Some(json!({"range": {"from": "2025-01-01", "to": "2025-09-30"}}))
    );
}

#[test]
fn relative_date_slicers_cover_the_days_desktop_shows() {
    use crate::preview::filters::{iso_day, relative_days};
    // Wednesday 2026-10-14 (as checked against Desktop 2.158)
    let today = 46_309.0;
    assert_eq!(iso_day(today), "2026-10-14");
    let days = |r: &str, n: f64, p: &str| {
        relative_days(r, n, p, today).map(|(a, b)| format!("{} {}", iso_day(a), iso_day(b)))
    };
    assert_eq!(
        days("last", 3.0, "months").as_deref(),
        Some("2026-07-15 2026-10-14")
    );
    assert_eq!(
        days("last", 7.0, "days").as_deref(),
        Some("2026-10-08 2026-10-14")
    );
    assert_eq!(
        days("this", 1.0, "months").as_deref(),
        Some("2026-10-01 2026-10-31")
    );
    assert_eq!(
        days("this", 1.0, "weeks").as_deref(),
        Some("2026-10-11 2026-10-17")
    );
    assert_eq!(
        days("last", 2.0, "calendar months").as_deref(),
        Some("2026-08-01 2026-09-30")
    );
    assert_eq!(
        days("next", 1.0, "calendaryears").as_deref(),
        Some("2027-01-01 2027-12-31")
    );
    assert_eq!(
        days("next", 7.0, "days").as_deref(),
        Some("2026-10-14 2026-10-20")
    );
    assert_eq!(days("last", 1.0, "minutes"), None);
    // hostile values never panic
    for (n, t) in [
        (f64::NAN, today),
        (1e300, today),
        (-5.0, today),
        (3.0, f64::INFINITY),
        (3.0, 1e18),
    ] {
        let _ = relative_days("last", n, "years", t);
        let _ = relative_days("next", n, "calendar weeks", t);
    }
    assert_eq!(iso_day(f64::NAN), "");
}
