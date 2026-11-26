//! The interaction tests that need the engine's private items (`interact::value_literal`, `date_literal`, `iso_serial`); the others are the `report`
//! integration test (`tests/report/interact_tests.rs`), under the same module path.

use serde_json::json;

#[test]
fn literals_follow_the_column_type() {
    use super::{DataType, value_literal};
    assert_eq!(value_literal(&json!("O'Neil"), DataType::String), "'O''Neil'");
    assert_eq!(value_literal(&json!(3), DataType::Int64), "3L");
    assert_eq!(value_literal(&json!(2.5), DataType::Double), "2.5D");
    assert_eq!(value_literal(&json!(2.5), DataType::Decimal), "2.5M");
    assert_eq!(value_literal(&json!(12), DataType::String), "'12'");
    assert_eq!(
        value_literal(&json!("2025-01-31"), DataType::DateTime),
        "datetime'2025-01-31T00:00:00'"
    );
    assert_eq!(
        value_literal(&json!("2025-01-31"), DataType::String),
        "'2025-01-31'"
    );
    assert_eq!(value_literal(&json!(null), DataType::String), "null");
    assert_eq!(value_literal(&json!(true), DataType::Boolean), "true");
    assert_eq!(
        super::date_literal("2025-12-31", 1).unwrap(),
        "datetime'2026-01-01T00:00:00'"
    );
    assert!(super::iso_serial("2025-13-01").is_none());
    assert!(super::iso_serial("2025-01-01x").is_none());
}

/// The built-in types that highlight are the ones Desktop highlighted in the highlight probe
/// (`testsupport/interaction_oracle/highlight_types.json`, docs/interaction-oracle.md
/// "Highlight probe"): every type it saw highlight is in the table, no type it saw filter is, and
/// the table holds no type the probe did not settle.
#[test]
fn highlight_types_are_the_ones_desktop_highlighted() {
    let probe: serde_json::Value = serde_json::from_str(include_str!(
        "../../../testsupport/interaction_oracle/highlight_types.json"
    ))
    .unwrap();
    let types = probe["types"].as_object().unwrap();
    for (ty, verdict) in types {
        let want = verdict == "highlight";
        if verdict == "highlight" || verdict == "filter" {
            assert_eq!(
                super::supports_highlight(Some(ty)),
                want,
                "{ty}: Desktop {verdict}s"
            );
        }
    }
    for ty in super::HIGHLIGHT_TYPES {
        assert_eq!(
            types.get(ty).and_then(|v| v.as_str()),
            Some("highlight"),
            "{ty}: not settled by the probe"
        );
    }
}
