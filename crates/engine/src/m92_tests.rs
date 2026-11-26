//! The M9.2 tests that need the engine's private items (`relationship_set::check_unique`, `date_table::check_dates`); the others are the `model`
//! integration test (`tests/model/m92_tests.rs`), under the same module path.

#[test]
fn ambiguity_check_counts_paths() {
    use super::relationship_set::check_unique;
    use bicraft_vertipaq::CellValue;
    assert!(check_unique(&[CellValue::Int(1), CellValue::Int(2)]).is_ok());
    assert!(check_unique(&[CellValue::Text("A".into()), CellValue::Text("a".into())]).is_err());
    assert!(check_unique(&[CellValue::Null]).is_err());
}

#[test]
fn date_rules() {
    use super::date_table::check_dates;
    use bicraft_vertipaq::CellValue::{DateTime, Null};
    assert!(check_dates(&[DateTime(45_658.0), DateTime(45_659.0)]).is_ok());
    assert!(check_dates(&[DateTime(45_659.0), DateTime(45_658.0)]).is_ok());
    assert!(check_dates(&[DateTime(45_658.0), DateTime(45_658.0)]).is_err());
    assert!(check_dates(&[DateTime(45_658.0), DateTime(45_660.0)]).is_err());
    assert!(check_dates(&[DateTime(45_658.0), DateTime(45_659.5)]).is_err());
    assert!(check_dates(&[DateTime(45_658.0), Null]).is_err());
    assert!(check_dates(&[]).is_err());
}
