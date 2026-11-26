//! Regression: a model property the order table does not name (e.g. `valueFilterBehavior`, from a
//! PBIX) must come before the nested `dataAccessOptions` block; Power BI Desktop 2.158 rejects a
//! property after it with "invalid indentation".

use bicraft_model::SemanticModel;
use serde_json::json;

#[test]
fn unknown_model_properties_precede_data_access_options() {
    let mut model = SemanticModel {
        name: "M".into(),
        culture: Some("en-US".into()),
        ..Default::default()
    };
    model.extra.insert(
        "dataAccessOptions".into(),
        json!({"legacyRedirects": true, "returnErrorValuesAsNull": true}),
    );
    model
        .extra
        .insert("valueFilterBehavior".into(), json!("independent"));
    let docs = bicraft_tmdl::write_documents(&model);
    let (_, text) = docs.iter().find(|(p, _)| p == "model.tmdl").unwrap();
    let vfb = text.find("valueFilterBehavior").unwrap();
    let dao = text.find("dataAccessOptions").unwrap();
    assert!(vfb < dao, "{text}");
}
