use serde_json::json;

use super::*;

/// Points are grouped by the group cells, sorted by X (numbers, then text) and cut at 52.
#[test]
fn points_are_grouped_sorted_and_capped() {
    let mut rows = vec![
        json!(["A", "2025-02-01", 2]),
        json!(["A", "2025-01-01", 1]),
        json!(["B", null, 5]),
        json!(["B", 3, 4]),
    ];
    for i in 0..60 {
        rows.push(json!(["C", i, i]));
    }
    let d = json!({"status": "ok", "rows": rows});
    let (pts, cut) = points_by_group(&d, 1, 1).unwrap();
    assert!(cut);
    assert_eq!(
        pts.get(&key_of([json!("A")].iter())).cloned(),
        Some(vec![json!(["2025-01-01", 1]), json!(["2025-02-01", 2])])
    );
    assert_eq!(
        pts.get(&key_of([json!("B")].iter())).cloned(),
        Some(vec![json!([null, 5]), json!([3, 4])])
    );
    let c = pts.get(&key_of([json!("C")].iter())).cloned().unwrap_or_default();
    assert_eq!(c.len(), 52);
    assert_eq!(c.last(), Some(&json!([51, 51])));
    // a failed evaluation is the reason
    let e = points_by_group(&json!({"status": "unsupported", "reason": "why"}), 1, 1).unwrap_err();
    assert_eq!(e, "why");
    // hostile shapes are no rows
    let (pts, _) = points_by_group(&json!({"status": "ok", "rows": [1, "x", null, [], [[]]]}), 3, 2).unwrap();
    assert!(pts.values().all(|p| p.len() <= 2));
}
