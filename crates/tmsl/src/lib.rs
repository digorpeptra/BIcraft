//! L3: TMSL, the JSON form of a tabular model used by `model.bim` (PBIP) and `DataModelSchema`
//! (PBIT).
//!
//! # Lossless reading
//!
//! The reader fills the typed fields of [`SemanticModel`] and keeps everything else in the
//! matching `extra` map (`database_extra` for the root object, `extra` for the `model` object,
//! `source_extra` for a partition's `source`). One rule decides which: **a property is moved into
//! a typed field only when that field can hold it and the writer will emit it again**. So:
//!
//! - A value of an unexpected JSON type (`"isHidden": "yes"`, `"description": null`,
//!   `"compatibilityLevel": "1567"`) stays in `extra` and is written back verbatim.
//! - Explicit defaults the writer omits (`"isHidden": false`, `"isActive": true`) stay in `extra`.
//! - Required names (`name`, `kind`, `source.type`, relationship endpoints, measure expressions)
//!   are consumed only when non-empty; an empty or missing one leaves the typed field empty.
//! - A child collection is modelled only when it is a non-empty array of objects; `annotations`
//!   additionally need every entry to be exactly `{name, value}` with string values. A collection
//!   that does not fit (or `[]`) is kept verbatim in `extra`, and the typed list stays empty.
//! - `dataType` is consumed when recognised (case-insensitively) and written in TOM's canonical
//!   spelling (`dateTime`); unrecognised names stay in `extra`.
//! - **`rowNumber` columns** are kept as ordinary [`Column`]s whose `extra` holds
//!   `"type": "rowNumber"`, like every other column `type` except `calculated`. Code that lists
//!   user-visible columns (DAX binding, engine commands) must skip them.
//! - `"type": "calculated"` on a column with an `expression` is implied by the expression and is
//!   not kept; the writer adds it back for any column with an expression and no `type` in `extra`.
//! - The description of the database object (as opposed to the model) is in `database_extra`;
//!   [`SemanticModel::description`] is the `model` object's description.
//!
//! Collections the model does not type (hierarchies, cultures, perspectives, calculation groups…)
//! stay in `extra` as JSON, untouched. Incomplete relationships are kept with empty endpoint
//! fields rather than dropped, so callers must tolerate unresolvable names.
//!
//! # Expressions
//!
//! `expression`, `source.expression` and `filterExpression` may be a string or an array of lines;
//! the reader joins lines with `\n`. The model does not record which form was used: the writer
//! emits an array when the text contains `\n` and a string otherwise, which is what Desktop does.
//! A one-element array or a multi-line string therefore changes form, but not meaning. Other
//! strings (descriptions, annotation values) are consumed only as strings, so they keep their form.
//!
//! # Output conventions
//!
//! [`write`] produces Desktop-style `model.bim` text: two-space indentation, `"key": value`, LF
//! line endings on every platform and a trailing newline. In every object, at every depth (objects
//! kept in `extra` included), `name` comes first and all other keys follow in ordinal order;
//! arrays keep their order. That is the key sorting Desktop applies to a PBIP `model.bim` (checked
//! on a `model.bim` Desktop saved, docs/formats/tmdl.md "Property order"); TOM's
//! own JSON order, which a PBIT `DataModelSchema` uses, is not reproduced. A typed value wins over
//! an `extra` entry with the same key. Values inside `extra` are otherwise written verbatim.
//! Defaults are omitted (`isHidden: false`, `isActive: true`, empty collections, `None`s).
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod read;
mod write;

use bicraft_model::SemanticModel;
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not a TMSL database: {0}")]
    Shape(&'static str),
}

/// Parses a TMSL database document (`{"name", "compatibilityLevel", "model": {…}}`).
pub fn read(json: &str) -> Result<SemanticModel, Error> {
    let doc: Value = serde_json::from_str(json.trim_start_matches('\u{feff}'))?;
    from_value(&doc)
}

/// Reads a TMSL database document that is already parsed.
pub fn from_value(doc: &Value) -> Result<SemanticModel, Error> {
    read::database(doc)
}

/// Serializes a model as a TMSL database document (`model.bim` / `DataModelSchema` text), with
/// LF line endings and a trailing newline. The bytes depend only on the model, not the platform.
pub fn write(model: &SemanticModel) -> String {
    // Serializing a `Value` cannot fail: its keys are always strings.
    let mut text = serde_json::to_string_pretty(&to_value(model)).unwrap_or_default();
    text.push('\n');
    text
}

/// The TMSL database document as a JSON value, keys in output order.
pub fn to_value(model: &SemanticModel) -> Value {
    write::database(model)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bicraft_model::DataType;
    use serde_json::json;

    const BIM: &str = r#"{
      "name": "Sample",
      "compatibilityLevel": 1567,
      "model": {
        "culture": "fr-FR",
        "tables": [
          {
            "name": "Sales",
            "columns": [
              {"name": "RowNumber-2662979B", "dataType": "int64", "type": "rowNumber", "isHidden": true},
              {"name": "Amount", "dataType": "double"},
              {"name": "Double", "dataType": "double", "type": "calculated", "expression": ["Sales[Amount]", "* 2"]}
            ],
            "measures": [{"name": "Total", "expression": "SUM(Sales[Amount])", "formatString": "0.00"}]
          },
          {"name": "Date", "isHidden": true}
        ],
        "relationships": [
          {"name": "r1", "fromTable": "Sales", "fromColumn": "Date", "toTable": "Date", "toColumn": "Date"},
          {"name": "broken"}
        ]
      }
    }"#;

    fn round_trip(json: &str) -> (Value, Value) {
        let input: Value = serde_json::from_str(json).unwrap();
        let output = to_value(&from_value(&input).unwrap());
        (input, output)
    }

    #[test]
    fn reads_tables_columns_measures_and_relationships() {
        let m = read(BIM).unwrap();
        assert_eq!(m.name, "Sample");
        assert_eq!(m.compatibility_level, Some(1567));
        assert_eq!(m.culture.as_deref(), Some("fr-FR"));
        let sales = m.table("Sales").unwrap();
        assert_eq!(sales.columns.len(), 3, "rowNumber columns are kept");
        let row_number = &sales.columns[0];
        assert_eq!(row_number.extra.get("type"), Some(&json!("rowNumber")));
        assert!(row_number.is_hidden);
        let double = sales.column("Double").unwrap();
        assert_eq!(double.expression.as_deref(), Some("Sales[Amount]\n* 2"));
        assert_eq!(double.data_type, DataType::Double);
        assert!(double.extra.is_empty(), "`type: calculated` is implied");
        assert_eq!(sales.measures[0].format_string.as_deref(), Some("0.00"));
        assert!(m.table("Date").unwrap().is_hidden);
        assert_eq!(m.relationships.len(), 2, "incomplete relationships are kept");
        assert_eq!(m.relationships[0].to_table, "Date");
        assert!(m.relationships[0].is_active);
        assert_eq!(m.relationships[1].from_table, "");
    }

    #[test]
    fn tolerates_bom_and_rejects_wrong_shapes() {
        assert!(read(&format!("\u{feff}{BIM}")).is_ok());
        assert!(matches!(read("[]"), Err(Error::Shape(_))));
        assert!(matches!(read("{}"), Err(Error::Shape(_))));
        assert!(matches!(read(r#"{"model": []}"#), Err(Error::Shape(_))));
        assert!(matches!(read("{"), Err(Error::Json(_))));
    }

    #[test]
    fn small_document_round_trips() {
        let (input, output) = round_trip(BIM);
        assert_eq!(input, output);
        assert_eq!(read(&write(&read(BIM).unwrap())).unwrap(), read(BIM).unwrap());
    }

    #[test]
    fn values_of_unexpected_types_survive_verbatim() {
        let doc = r#"{
          "name": 42,
          "compatibilityLevel": "1567",
          "description": "the database",
          "model": {
            "tables": [
              {
                "name": "",
                "isHidden": "yes",
                "description": null,
                "columns": [
                  {"name": "A", "dataType": "someFutureType", "isHidden": false},
                  {"name": "B", "dataType": "DATETIME", "expression": ["x"], "type": "calculatedTableColumn"}
                ],
                "measures": [{"name": "M", "expression": ""}, {"name": "N", "expression": [1, 2]}],
                "partitions": [{"name": "P", "source": "not an object"}, {"name": "Q", "source": {}}],
                "annotations": [{"name": "A", "value": "1", "extra": true}]
              },
              {"name": "Empty", "columns": [], "measures": [1]}
            ],
            "relationships": [{"fromTable": "T", "isActive": true, "toCardinality": 3}],
            "roles": [{"name": "R", "tablePermissions": [{"name": "T", "filterExpression": {"x": 1}}]}],
            "expressions": [{"name": "E", "kind": "", "expression": "1"}]
          }
        }"#;
        let model = read(doc).unwrap();
        assert_eq!(model.name, "");
        assert_eq!(model.compatibility_level, None);
        let t = &model.tables[0];
        assert!(!t.is_hidden);
        assert_eq!(t.columns[0].data_type, DataType::Unknown);
        assert_eq!(t.columns[1].data_type, DataType::DateTime);
        assert_eq!(t.columns[1].expression.as_deref(), Some("x"));
        assert_eq!(t.measures[0].expression, "");
        assert!(t.annotations.is_empty());
        assert!(model.tables[1].measures.is_empty());

        let written = to_value(&model);
        let col_b = &written["model"]["tables"][0]["columns"][1];
        assert_eq!(col_b["dataType"], json!("dateTime"), "canonical spelling");
        assert_eq!(col_b["expression"], json!("x"), "one-line text is a string");
        assert_eq!(col_b["type"], json!("calculatedTableColumn"));
        // Apart from those two documented normalizations, the document comes back unchanged.
        let mut expected: Value = serde_json::from_str(doc).unwrap();
        expected["model"]["tables"][0]["columns"][1]["dataType"] = json!("dateTime");
        expected["model"]["tables"][0]["columns"][1]["expression"] = json!("x");
        assert_eq!(written, expected);
        assert_eq!(read(&write(&model)).unwrap(), model);
    }

    #[test]
    fn writer_orders_keys_name_first_then_alphabetically() {
        let (_, output) = round_trip(BIM);
        let column = &output["model"]["tables"][0]["columns"][2];
        let keys: Vec<_> = column.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["name", "dataType", "expression", "type"]);
        let rel = &output["model"]["relationships"][0];
        let keys: Vec<_> = rel.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["name", "fromColumn", "fromTable", "toColumn", "toTable"]);
        let root: Vec<_> = output.as_object().unwrap().keys().cloned().collect();
        assert_eq!(root, ["name", "compatibilityLevel", "model"]);
    }

    #[test]
    fn writer_adds_calculated_type_and_skips_defaults() {
        use bicraft_model::{Column, Relationship, Table};
        let model = SemanticModel {
            name: "M".into(),
            tables: vec![Table {
                name: "T".into(),
                columns: vec![Column {
                    name: "C".into(),
                    data_type: DataType::Int64,
                    expression: Some("1\n+ 1".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            relationships: vec![Relationship {
                name: "r".into(),
                from_table: "T".into(),
                from_column: "C".into(),
                to_table: "U".into(),
                to_column: "C".into(),
                is_active: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let value = to_value(&model);
        assert_eq!(
            value,
            json!({
                "name": "M",
                "model": {
                    "relationships": [{
                        "name": "r", "fromColumn": "C", "fromTable": "T",
                        "toColumn": "C", "toTable": "U"
                    }],
                    "tables": [{
                        "name": "T",
                        "columns": [{
                            "name": "C", "dataType": "int64",
                            "expression": ["1", "+ 1"], "type": "calculated"
                        }]
                    }]
                }
            })
        );
        assert_eq!(read(&write(&model)).unwrap(), model);
    }

    #[test]
    fn output_format_is_two_space_lf_with_trailing_newline() {
        let text = write(&read(BIM).unwrap());
        assert!(text.starts_with("{\n  \"name\": \"Sample\",\n  \"compatibilityLevel\": 1567,\n"));
        assert!(text.ends_with("}\n"));
        assert!(!text.contains('\r'));
        assert_eq!(text, write(&read(&text).unwrap()), "byte-stable");
    }
}
