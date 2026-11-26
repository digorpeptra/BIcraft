//! Round trips of a realistic, self-authored `model.bim` written in Desktop's conventions.

use bicraft_model::DataType;
use serde_json::{Value, json};

/// The fixture with line endings normalized, in case a checkout converted them.
fn fixture() -> String {
    include_str!("fixtures/sales.bim").replace("\r\n", "\n")
}

#[test]
fn read_write_read_gives_an_equal_model() {
    let model = bicraft_tmsl::read(&fixture()).unwrap();
    let again = bicraft_tmsl::read(&bicraft_tmsl::write(&model)).unwrap();
    assert_eq!(again, model);
}

#[test]
fn read_write_gives_semantically_equal_json() {
    let input: Value = serde_json::from_str(&fixture()).unwrap();
    let model = bicraft_tmsl::from_value(&input).unwrap();
    assert_eq!(bicraft_tmsl::to_value(&model), input);
}

#[test]
fn desktop_formatted_input_comes_back_byte_identical() {
    let text = fixture();
    let written = bicraft_tmsl::write(&bicraft_tmsl::read(&text).unwrap());
    if written != text {
        let line = written
            .lines()
            .zip(text.lines())
            .position(|(a, b)| a != b)
            .map_or(0, |i| i + 1);
        panic!("output differs from the fixture at line {line}:\n{written}");
    }
}

#[test]
fn writing_is_byte_stable_and_platform_neutral() {
    let model = bicraft_tmsl::read(&fixture()).unwrap();
    let first = bicraft_tmsl::write(&model);
    let second = bicraft_tmsl::write(&bicraft_tmsl::read(&first).unwrap());
    assert_eq!(first.as_bytes(), second.as_bytes());
    assert!(!first.contains('\r'), "LF only, on every platform");
    assert!(first.ends_with("}\n") && !first.ends_with("\n\n"));
}

#[test]
fn typed_fields_are_filled() {
    let m = bicraft_tmsl::read(&fixture()).unwrap();
    assert_eq!(m.name, "SalesModel");
    assert_eq!(m.compatibility_level, Some(1567));
    assert_eq!(m.culture.as_deref(), Some("en-US"));
    assert_eq!(m.annotations.len(), 3);
    assert!(m.database_extra.contains_key("id"));
    for key in [
        "cultures",
        "dataAccessOptions",
        "futureModelSetting",
        "queryGroups",
    ] {
        assert!(m.extra.contains_key(key), "{key} kept in model extra");
    }

    let sales = m.table("Sales").unwrap();
    assert_eq!(sales.columns.len(), 8);
    assert_eq!(sales.columns[0].extra.get("type"), Some(&json!("rowNumber")));
    let region = sales.column("Region").unwrap();
    assert_eq!(region.data_category.as_deref(), Some("StateOrProvince"));
    assert_eq!(region.display_folder.as_deref(), Some("Geography"));
    assert_eq!(region.data_type, DataType::String);
    let tax = sales.column("Amount With Tax").unwrap();
    assert!(tax.expression.as_deref().unwrap().starts_with("\nVAR rate"));
    assert!(!tax.extra.contains_key("type"));
    let yoy = sales.measure("Sales YoY %").unwrap();
    assert!(yoy.extra.contains_key("formatStringDefinition"));
    assert!(sales.measure("Hidden Helper").unwrap().is_hidden);
    let part = &sales.partitions[0];
    assert_eq!(part.source_type, "m");
    assert_eq!(part.mode.as_deref(), Some("import"));
    assert!(part.expression.as_deref().unwrap().starts_with("let\n"));
    assert!(sales.extra.contains_key("refreshPolicy"));

    let product = m.table("Product").unwrap();
    assert!(product.extra.contains_key("hierarchies"));
    assert_eq!(product.description.as_deref(), Some("One row per product."));
    assert_eq!(
        product.column("Category").unwrap().sort_by_column.as_deref(),
        Some("Category Order")
    );
    let part = &product.partitions[0];
    assert!(part.extra.contains_key("queryGroup"));
    assert!(part.source_extra.contains_key("futureSourceOption"));

    let date = m.table("Date").unwrap();
    assert_eq!(date.partitions[0].source_type, "calculated");
    assert_eq!(
        date.column("Date").unwrap().extra.get("type"),
        Some(&json!("calculatedTableColumn"))
    );
    assert!(m.table("Product Details").unwrap().is_hidden);

    let rels = &m.relationships;
    assert_eq!(rels.len(), 4);
    assert_eq!(
        rels[1].cross_filtering_behavior.as_deref(),
        Some("bothDirections")
    );
    assert!(rels[1].extra.contains_key("joinOnDateBehavior"));
    assert!(!rels[2].is_active);
    assert_eq!(rels[3].from_cardinality.as_deref(), Some("one"));
    assert_eq!(rels[3].to_cardinality.as_deref(), Some("one"));

    let west = &m.roles[0];
    assert_eq!(west.model_permission.as_deref(), Some("read"));
    assert_eq!(west.table_permissions[0].table, "Sales");
    assert_eq!(
        west.table_permissions[1].filter_expression.as_deref(),
        Some("VAR allowed = { \"Bikes\", \"Helmets\" }\nRETURN\n    [Category] IN allowed")
    );
    assert!(west.table_permissions[1].extra.contains_key("metadataPermission"));
    assert!(m.roles[1].extra.contains_key("members"));

    let folder = &m.expressions[1];
    assert_eq!(folder.kind, "m");
    assert_eq!(
        folder.description.as_deref(),
        Some("Where the CSV extracts live.")
    );
    assert!(folder.expression.starts_with("let\n"));
    assert_eq!(m.expressions[0].annotations[0].value, "DateTime");
}

#[test]
fn edits_survive_and_extras_are_kept() {
    let mut m = bicraft_tmsl::read(&fixture()).unwrap();
    if let Some(t) = m.table_mut("Product") {
        t.description = None;
        t.is_hidden = true;
    }
    let out = bicraft_tmsl::to_value(&m);
    let product = &out["model"]["tables"][1];
    assert_eq!(product["isHidden"], json!(true));
    assert!(product.get("description").is_none());
    assert_eq!(product["hierarchies"][0]["name"], json!("Catalog"));
}

/// Auto date/time: a column's `variations` and its local date table survive a round trip, and the
/// model resolves `'Sales'[OrderDate].[Year]` through them.
#[test]
fn column_variations_round_trip() {
    let input = json!({
        "name": "V",
        "compatibilityLevel": 1567,
        "model": {
            "tables": [
                {
                    "name": "Sales",
                    "columns": [{
                        "name": "OrderDate",
                        "dataType": "dateTime",
                        "sourceColumn": "OrderDate",
                        "variations": [{
                            "name": "Variation",
                            "relationship": "0b5c2f8e-2222",
                            "defaultHierarchy": {"table": "LocalDateTable_1", "hierarchy": "Date Hierarchy"},
                            "isDefault": true
                        }]
                    }]
                },
                {
                    "name": "LocalDateTable_1",
                    "isHidden": true,
                    "showAsVariationsOnly": true,
                    "columns": [
                        {"name": "Date", "dataType": "dateTime", "isHidden": true, "sourceColumn": "Date"},
                        {"type": "calculated", "name": "Year", "dataType": "int64", "isHidden": true, "expression": "YEAR([Date])"}
                    ],
                    "hierarchies": [{"name": "Date Hierarchy", "levels": [{"name": "Year", "ordinal": 0, "column": "Year"}]}]
                }
            ],
            "relationships": [{
                "name": "0b5c2f8e-2222",
                "fromTable": "Sales",
                "fromColumn": "OrderDate",
                "toTable": "LocalDateTable_1",
                "toColumn": "Date",
                "joinOnDateBehavior": "datePartOnly"
            }]
        }
    });
    let model = bicraft_tmsl::from_value(&input).unwrap();
    assert_eq!(bicraft_tmsl::to_value(&model), input);
    let column = model.table("Sales").and_then(|t| t.column("OrderDate")).unwrap();
    let variations = column.variations();
    assert_eq!(variations.len(), 1);
    assert_eq!(
        variations[0].default_hierarchy,
        Some(("LocalDateTable_1".into(), "Date Hierarchy".into()))
    );
    let (t, c) = model.variation_column("Sales", "OrderDate", "Year").unwrap();
    assert_eq!((t.name.as_str(), c.name.as_str()), ("LocalDateTable_1", "Year"));
}

/// Every object's keys of `v`, at every depth, in reverse order.
fn reverse_keys(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(m) => {
            let mut entries: Vec<_> = m.into_iter().collect();
            entries.reverse();
            serde_json::Value::Object(entries.into_iter().map(|(k, v)| (k, reverse_keys(v))).collect())
        }
        serde_json::Value::Array(a) => serde_json::Value::Array(a.into_iter().map(reverse_keys).collect()),
        other => other,
    }
}

/// `desktop-sorted.bim` is the `model.bim` Power BI Desktop 2.160.1151.0 saved for a project
/// whose keys were all in reverse order (fixtures/SOURCE.txt). Read with its keys reversed
/// again, bicraft must write Desktop's text back: `name` first, then ordinal order, at every
/// depth, arrays unchanged (docs/formats/tmdl.md "Property order").
#[test]
fn reversed_keys_are_written_in_the_order_desktop_saved() {
    let desktop = include_str!("fixtures/desktop-sorted.bim").replace("\r\n", "\n");
    let parsed: serde_json::Value = serde_json::from_str(&desktop).unwrap();
    let reversed = serde_json::to_string_pretty(&reverse_keys(parsed)).unwrap();
    assert_ne!(reversed.trim_end(), desktop.trim_end());
    let written = bicraft_tmsl::write(&bicraft_tmsl::read(&reversed).unwrap());
    let (want, got) = (desktop.trim_end(), written.trim_end());
    if want != got {
        let line = got
            .lines()
            .zip(want.lines())
            .position(|(a, b)| a != b)
            .map_or(0, |i| i + 1);
        panic!(
            "differs from Desktop's saved model.bim at line {line}: want {:?}\n{got}",
            want.lines().nth(line.saturating_sub(1))
        );
    }
}

/// `tom-props.bim` is TOM's `JsonSerializer` output for the "every property set" model of
/// the maintainers' development oracles (TOM's key order, nested objects included);
/// `tom-props.pbip.bim` is the same text sorted by the rule Desktop applies to a PBIP
/// `model.bim` (see the test above): it extends that check to every property TOM writes.
#[test]
fn tom_ordered_input_is_written_like_a_desktop_pbip_model_bim() {
    let tom = include_str!("fixtures/tom-props.bim").replace("\r\n", "\n");
    let pbip = include_str!("fixtures/tom-props.pbip.bim").replace("\r\n", "\n");
    let written = bicraft_tmsl::write(&bicraft_tmsl::read(&tom).unwrap());
    let (want, got) = (pbip.trim_end(), written.trim_end());
    if want != got {
        let line = got
            .lines()
            .zip(want.lines())
            .position(|(a, b)| a != b)
            .map_or(0, |i| i + 1);
        panic!(
            "differs from Desktop's PBIP sorting at line {line}: want {:?}\n{got}",
            want.lines().nth(line.saturating_sub(1))
        );
    }
}
