use serde_json::{Value, json};

use bicraft_engine::Session;

/// A model with every kind of property `model.get` reports, as TMSL.
fn session() -> Session {
    let bim = json!({
        "name": "M",
        "compatibilityLevel": 1567,
        "model": {
            "culture": "en-US",
            "discourageImplicitMeasures": true,
            "sourceQueryCulture": "fr-FR",
            "defaultPowerBIDataSourceVersion": "powerBI_V3",
            "dataAccessOptions": {"legacyRedirects": true},
            "annotations": [{"name": "PBI_QueryOrder", "value": "[\"Sales\"]"}],
            "tables": [
                {
                    "name": "Sales",
                    "lineageTag": "t-sales",
                    "description": "Fact",
                    "columns": [
                        {"name": "Amount", "dataType": "decimal", "sourceColumn": "Amount",
                         "formatString": "#,0.00", "summarizeBy": "sum", "displayFolder": "Money",
                         "lineageTag": "c-amount", "isAvailableInMdx": false, "encodingHint": "value",
                         "annotations": [{"name": "SummarizationSetBy", "value": "Automatic"}]},
                        {"name": "Key", "dataType": "int64", "sourceColumn": "Key", "isHidden": true,
                         "isKey": true, "dataCategory": "Uncategorized"},
                        {"name": "Label", "dataType": "string", "sourceColumn": "Label",
                         "sortByColumn": "Key"},
                        {"type": "calculated", "name": "Double", "dataType": "decimal",
                         "expression": "Sales[Amount] * 2", "isDataTypeInferred": true}
                    ],
                    "measures": [
                        {"name": "Total", "expression": ["SUM(", "Sales[Amount])"], "formatString": "0.0",
                         "formatStringDefinition": {"expression": "\"0.00\""}, "displayFolder": "KPIs",
                         "description": "All sales", "lineageTag": "m-total",
                         "detailRowsDefinition": {"expression": "Sales"},
                         "annotations": [{"name": "x", "value": "y"}]}
                    ],
                    "hierarchies": [
                        {"name": "H", "levels": [
                            {"name": "L1", "ordinal": 0, "column": "Key"},
                            {"name": "L2", "ordinal": 1, "column": "Label"}
                        ]}
                    ],
                    "partitions": [
                        {"name": "p1", "mode": "import", "source": {"type": "m", "expression": "let x = 1 in x"}}
                    ]
                },
                {
                    "name": "Dim",
                    "dataCategory": "Time",
                    "columns": [{"name": "Key", "dataType": "int64", "sourceColumn": "Key"}],
                    "partitions": [
                        {"name": "d", "source": {"type": "calculated", "expression": "DATATABLE(\"Key\", INTEGER, {{1}})"}}
                    ]
                },
                {
                    "name": "Calc",
                    "calculationGroup": {
                        "precedence": 3,
                        "calculationItems": [
                            {"name": "YTD", "expression": "SELECTEDMEASURE()", "ordinal": 0,
                             "formatStringDefinition": {"expression": "\"0%\""}}
                        ]
                    },
                    "columns": [{"name": "Name", "dataType": "string", "sourceColumn": "Name"}],
                    "partitions": [{"name": "c", "source": {"type": "calculationGroup"}}]
                }
            ],
            "relationships": [
                {"name": "r1", "fromTable": "Sales", "fromColumn": "Key", "toTable": "Dim",
                 "toColumn": "Key", "crossFilteringBehavior": "bothDirections",
                 "securityFilteringBehavior": "bothDirections"},
                {"name": "r2", "fromTable": "Sales", "fromColumn": "Amount", "toTable": "Dim",
                 "toColumn": "Key", "isActive": false, "relyOnReferentialIntegrity": true}
            ],
            "roles": [
                {"name": "R", "modelPermission": "read",
                 "members": [{"memberName": "someone"}],
                 "tablePermissions": [{"name": "Sales", "filterExpression": "Sales[Key] = 1"}]}
            ],
            "perspectives": [{"name": "P", "tables": [{"name": "Sales", "includeAll": true}]}],
            "cultures": [{"name": "fr-FR"}],
            "expressions": [{"name": "Param", "kind": "m", "expression": "1 meta [IsParameterQuery=true]"}]
        }
    });
    let mut s = Session::new();
    s.model = Some(bicraft_tmsl::read(&bim.to_string()).unwrap());
    s
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"))
}

#[test]
fn column_get_reports_every_property() {
    let mut s = session();
    let c = run(&mut s, "column.get", json!({"table": "sales", "name": "amount"}));
    assert_eq!(c["name"], "Amount");
    assert_eq!(c["table"], "Sales");
    assert_eq!(c["kind"], "data");
    assert_eq!(c["data_type"], "decimal");
    assert_eq!(c["format_string"], "#,0.00");
    assert_eq!(c["summarize_by"], "sum");
    assert_eq!(c["display_folder"], "Money");
    assert_eq!(c["is_available_in_mdx"], false);
    assert_eq!(c["encoding_hint"], "value");
    assert_eq!(c["lineage_tag"], "c-amount");
    assert_eq!(c["hidden"], false);
    assert_eq!(c["is_nullable"], true);
    assert_eq!(c["sort_by_column"], Value::Null);
    assert_eq!(c["annotations"][0]["name"], "SummarizationSetBy");
    assert_eq!(c["relationships"], json!(["r2"]));
    assert!(c["extra"].as_object().unwrap().is_empty(), "{c}");

    let k = run(&mut s, "column.get", json!({"table": "Sales", "name": "Key"}));
    assert_eq!(
        (k["hidden"].clone(), k["is_key"].clone()),
        (json!(true), json!(true))
    );
    assert_eq!(k["data_category"], "Uncategorized");
    assert_eq!(k["is_available_in_mdx"], true);
    let l = run(&mut s, "column.get", json!({"table": "Sales", "name": "Label"}));
    assert_eq!(l["sort_by_column"], "Key");
    let d = run(&mut s, "column.get", json!({"table": "Sales", "name": "Double"}));
    assert_eq!(d["kind"], "calculated");
    assert_eq!(d["expression"], "Sales[Amount] * 2");
    assert_eq!(d["is_data_type_inferred"], true);

    assert!(
        s.execute("column.get", &json!({"table": "Sales", "name": "Nope"}))
            .is_err()
    );
    assert!(
        s.execute("column.get", &json!({"table": "Nope", "name": "Key"}))
            .is_err()
    );
    assert!(
        s.execute("column.get", &json!({"table": "Sales", "name": "Key", "x": 1}))
            .is_err()
    );
}

#[test]
fn measure_get_reads_dynamic_format_strings_and_detail_rows() {
    let mut s = session();
    let m = run(&mut s, "measure.get", json!({"name": "total"}));
    assert_eq!(m["name"], "Total");
    assert_eq!(m["table"], "Sales");
    assert_eq!(m["expression"], "SUM(\nSales[Amount])");
    assert_eq!(m["format_string"], "0.0");
    assert_eq!(m["format_string_definition"], "\"0.00\"");
    assert_eq!(m["detail_rows_definition"], "Sales");
    assert_eq!(m["display_folder"], "KPIs");
    assert_eq!(m["description"], "All sales");
    assert_eq!(m["lineage_tag"], "m-total");
    assert_eq!(m["kpi"], Value::Null);
    assert_eq!(m["annotations"], json!([{"name": "x", "value": "y"}]));
    let again = run(&mut s, "measure.get", json!({"name": "Total", "table": "Sales"}));
    assert_eq!(again, m);
    assert!(
        s.execute("measure.get", &json!({"name": "Total", "table": "Dim"}))
            .is_err()
    );
    assert!(s.execute("measure.get", &json!({"name": "None"})).is_err());
}

#[test]
fn relationship_list_reports_filtering_and_cardinalities() {
    let mut s = session();
    let r = run(&mut s, "relationship.list", json!({}));
    assert_eq!(r["count"], 2);
    let r1 = &r["relationships"][0];
    assert_eq!(r1["from"], "Sales[Key]");
    assert_eq!(r1["to"], "Dim[Key]");
    assert_eq!(r1["cross_filter"], "both");
    assert_eq!(r1["cross_filtering_behavior"], "bothDirections");
    assert_eq!(r1["security_filtering_behavior"], "bothDirections");
    assert_eq!(r1["from_cardinality"], "many");
    assert_eq!(r1["to_cardinality"], "one");
    assert_eq!(r1["is_active"], true);
    let r2 = &r["relationships"][1];
    assert_eq!(r2["is_active"], false);
    assert_eq!(r2["cross_filter"], "single");
    assert_eq!(r2["rely_on_referential_integrity"], true);
    let only = run(&mut s, "relationship.list", json!({"table": "Calc"}));
    assert_eq!(only["count"], 0);
    assert!(s.execute("relationship.list", &json!({"table": "Nope"})).is_err());
}

#[test]
fn table_get_reads_partitions_hierarchies_and_calculation_groups() {
    let mut s = session();
    let t = run(&mut s, "table.get", json!({"name": "Sales"}));
    assert_eq!(t["kind"], "data");
    assert_eq!(t["lineage_tag"], "t-sales");
    assert_eq!(t["columns"].as_array().unwrap().len(), 4);
    assert_eq!(t["measures"][0]["name"], "Total");
    assert_eq!(t["partitions"][0]["source_type"], "m");
    assert_eq!(t["partitions"][0]["mode"], "import");
    assert_eq!(t["partitions"][0]["expression"], "let x = 1 in x");
    assert_eq!(t["hierarchies"][0]["levels"][1]["column"], "Label");
    assert_eq!(t["calculation_group"], Value::Null);

    let d = run(&mut s, "table.get", json!({"name": "Dim"}));
    assert_eq!(d["kind"], "calculated");
    assert_eq!(d["data_category"], "Time");
    assert_eq!(d["partitions"][0]["source_type"], "calculated");

    let c = run(&mut s, "table.get", json!({"name": "Calc"}));
    assert_eq!(c["kind"], "calculation_group");
    assert_eq!(c["calculation_group"]["precedence"], 3);
    assert_eq!(c["calculation_group"]["items"][0]["name"], "YTD");
    assert_eq!(
        c["calculation_group"]["items"][0]["format_string_definition"],
        "\"0%\""
    );
}

#[test]
fn model_get_summary_and_full() {
    let mut s = session();
    let brief = run(&mut s, "model.get", json!({}));
    assert_eq!(brief["name"], "M");
    assert_eq!(brief["discourage_implicit_measures"], true);
    assert_eq!(brief["source_query_culture"], "fr-FR");
    assert_eq!(
        brief["counts"],
        json!({"tables": 3, "columns": 6, "measures": 1, "hierarchies": 1, "partitions": 3,
               "calculation_groups": 1, "relationships": 2, "roles": 1, "expressions": 1,
               "perspectives": 1, "cultures": 1})
    );
    assert_eq!(brief["tables"][2]["kind"], "calculation_group");
    assert!(brief["tables"][0].get("columns").unwrap().is_u64());

    let full = run(&mut s, "model.get", json!({"detail": "full"}));
    assert_eq!(full["compatibility_level"], 1567);
    assert_eq!(full["culture"], "en-US");
    assert_eq!(full["default_power_bi_data_source_version"], "powerBI_V3");
    assert_eq!(full["data_access_options"]["legacyRedirects"], true);
    assert_eq!(full["annotations"][0]["name"], "PBI_QueryOrder");
    assert_eq!(full["perspectives"][0]["name"], "P");
    assert_eq!(full["cultures"][0]["name"], "fr-FR");
    assert_eq!(full["roles"][0]["members"][0]["memberName"], "someone");
    assert_eq!(
        full["roles"][0]["table_permissions"][0]["filter_expression"],
        "Sales[Key] = 1"
    );
    assert_eq!(full["expressions"][0]["name"], "Param");
    assert_eq!(
        full["tables"][0],
        run(&mut s, "table.get", json!({"name": "Sales"}))
    );
    assert_eq!(
        full["relationships"],
        run(&mut s, "relationship.list", json!({}))["relationships"]
    );

    assert!(s.execute("model.get", &json!({"detail": "everything"})).is_err());
    // Read-only: the revision does not move.
    let before = s.revision;
    run(&mut s, "model.get", json!({"detail": "full"}));
    assert_eq!(s.revision, before);
    assert!(Session::new().execute("model.get", &json!({})).is_err());
}
