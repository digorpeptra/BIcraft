//! model -> TMDL -> model over a TMSL-shaped model built in code: typed fields, `extra` values of
//! every JSON type, unmodelled collections, and names that need quoting or would be unsafe as
//! file names.

use std::path::{Path, PathBuf};

use bicraft_model::{
    Annotation, Column, DataType, Measure, NamedExpression, Partition, Properties, Relationship, Role,
    SemanticModel, Table, TablePermission,
};
use bicraft_tmdl::{file_stem, read_documents, read_folder, write_documents, write_folder};
use serde_json::{Value, json};

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;
use tempdir::TempDir;

fn props(v: Value) -> Properties {
    match v {
        Value::Object(m) => m,
        _ => Properties::new(),
    }
}

fn data_table(name: &str) -> Table {
    Table {
        name: name.into(),
        columns: vec![Column {
            name: "c".into(),
            data_type: DataType::String,
            source_column: Some("c".into()),
            ..Default::default()
        }],
        partitions: vec![Partition {
            name: name.into(),
            mode: Some("import".into()),
            source_type: "m".into(),
            expression: Some("#table({\"c\"}, {})".into()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn sample() -> SemanticModel {
    let facts = Table {
        name: "Order Lines".into(),
        description: Some("Line one\n\nLine three with trailing space ".into()),
        lineage_tag: Some("lt-1".into()),
        is_hidden: true,
        columns: vec![
            Column {
                name: "Key".into(),
                data_type: DataType::Int64,
                source_column: Some("Key".into()),
                format_string: Some("0".into()),
                summarize_by: Some("none".into()),
                lineage_tag: Some("lt-2".into()),
                is_hidden: true,
                extra: props(json!({"isKey": true, "isNullable": false})),
                ..Default::default()
            },
            Column {
                name: "Amount".into(),
                data_type: DataType::Decimal,
                source_column: Some("Amount".into()),
                format_string: Some("\\$#,0.00;(\\$#,0.00)".into()),
                display_folder: Some(" Money".into()),
                sort_by_column: Some("Key".into()),
                data_category: Some("Currency".into()),
                description: Some("The amount.".into()),
                annotations: vec![Annotation {
                    name: "PBI_FormatHint".into(),
                    value: "{\"currencyCulture\":\"en-US\"}".into(),
                }],
                // An explicit default and a property TMDL never writes for Desktop.
                extra: props(json!({"isHidden": false, "isDataTypeInferred": true})),
                ..Default::default()
            },
            Column {
                name: "Indented".into(),
                data_type: DataType::Double,
                expression: Some("    [Amount]\n        * 2".into()),
                ..Default::default()
            },
            Column {
                name: "Empty calc".into(),
                expression: Some(String::new()),
                ..Default::default()
            },
            Column {
                name: "From calc table".into(),
                data_type: DataType::DateTime,
                source_column: Some("[Date]".into()),
                extra: props(json!({"type": "calculatedTableColumn", "isNameInferred": true})),
                ..Default::default()
            },
        ],
        measures: vec![
            Measure {
                name: "Owner's \"Cut\"".into(),
                expression: "VAR x = 1\nRETURN\n    x\n".into(),
                format_string: Some("\"q\"".into()),
                display_folder: Some("A\\B".into()),
                description: Some(" padded ".into()),
                lineage_tag: Some("lt-3".into()),
                is_hidden: true,
                annotations: vec![
                    Annotation {
                        name: "multi line".into(),
                        value: "first\nsecond".into(),
                    },
                    Annotation {
                        name: "empty".into(),
                        value: String::new(),
                    },
                ],
                extra: props(json!({
                    "kpi": {
                        "targetExpression": "1000",
                        "statusGraphic": "Traffic Light - Single",
                        "statusExpression": "VAR s = [x]\nRETURN s"
                    },
                    "formatStringDefinition": {"expression": "\"0.0\""},
                    "dataCategory": "WebUrl",
                    "extendedProperties": [
                        {"type": "json", "name": "Json", "value": {"a": [1, 2.5, null, "x"]}},
                        {"name": "Text", "value": "{not json"}
                    ]
                })),
            },
            Measure {
                name: "Fence inside".into(),
                expression: "a\n```\nb".into(),
                ..Default::default()
            },
            Measure {
                name: "Empty".into(),
                expression: String::new(),
                ..Default::default()
            },
        ],
        partitions: vec![
            Partition {
                name: "M part".into(),
                mode: Some("import".into()),
                source_type: "m".into(),
                expression: Some("let\n    Source = 1\nin\n    Source".into()),
                extra: props(json!({"description": "pd", "queryGroup": "Group 1"})),
                ..Default::default()
            },
            Partition {
                name: "Query part".into(),
                source_type: "query".into(),
                source_extra: props(json!({"query": "SELECT 1", "dataSource": "DS 1"})),
                ..Default::default()
            },
            Partition {
                name: "Lake".into(),
                mode: Some("directLake".into()),
                source_type: "entity".into(),
                source_extra: props(json!({
                    "entityName": "fact_lines",
                    "schemaName": "dbo",
                    "expressionSource": "DatabaseQuery"
                })),
                ..Default::default()
            },
        ],
        annotations: vec![Annotation {
            name: "PBI_ResultType".into(),
            value: "Table".into(),
        }],
        extra: props(json!({
            "hierarchies": [{
                "name": "Keys",
                "lineageTag": "h-1",
                "levels": [
                    {"name": "Key", "ordinal": 0, "column": "Key", "lineageTag": "l-1"},
                    {"name": "Amount level", "ordinal": 1, "column": "Amount"}
                ]
            }],
            "refreshPolicy": {
                "policyType": "basic",
                "rollingWindowGranularity": "year",
                "rollingWindowPeriods": 3,
                "sourceExpression": "let\n    x = 1\nin\n    x"
            },
            "excludeFromModelRefresh": true,
            "numberLikeText": "42",
            "boolLikeText": "true",
            "nullValue": null,
            "float": 2.5,
            "widgets": [{"name": "W 1", "size": 3}]
        })),
    };

    let calc = Table {
        name: "Time Intelligence".into(),
        columns: vec![
            Column {
                name: "Name".into(),
                data_type: DataType::String,
                source_column: Some("Name".into()),
                sort_by_column: Some("Ordinal".into()),
                ..Default::default()
            },
            Column {
                name: "Ordinal".into(),
                data_type: DataType::Int64,
                source_column: Some("Ordinal".into()),
                is_hidden: true,
                ..Default::default()
            },
        ],
        partitions: vec![Partition {
            name: "Partition".into(),
            mode: Some("import".into()),
            source_type: "calculationGroup".into(),
            ..Default::default()
        }],
        extra: props(json!({
            "calculationGroup": {
                "precedence": 2,
                "calculationItems": [
                    {"name": "Current", "ordinal": 0, "expression": "SELECTEDMEASURE()"},
                    {"name": "YTD", "ordinal": 1, "expression": "TOTALYTD(SELECTEDMEASURE(), 'Date'[Date])",
                     "formatStringDefinition": {"expression": "\"#,0\""}}
                ]
            }
        })),
        ..Default::default()
    };

    let mut tables = vec![facts, calc];
    for name in [
        "../escape",
        "..",
        "/etc/passwd",
        "C:\\Windows\\win.ini",
        "a/b",
        "a%2Fb",
        "CON",
        "nul.txt",
        " lead",
        "trail.",
        "tab\tname",
        "ctl\u{1}x",
        "2024 Targets",
        "ünï 日本",
        "Sales.Archive",
        ":colon",
        "\"quoted\"",
    ] {
        tables.push(data_table(name));
    }
    tables.push(data_table(&"long".repeat(120)));

    SemanticModel {
        name: "Contoso / Hostile".into(),
        compatibility_level: Some(1702),
        culture: Some("en-US".into()),
        description: Some("Model description".into()),
        tables,
        relationships: vec![
            Relationship {
                name: "r1".into(),
                from_table: "Order Lines".into(),
                from_column: "Key".into(),
                to_table: "../escape".into(),
                to_column: "c".into(),
                is_active: true,
                ..Default::default()
            },
            Relationship {
                name: "rel two".into(),
                from_table: "\"quoted\"".into(),
                from_column: "c".into(),
                to_table: "Sales.Archive".into(),
                to_column: "c".into(),
                is_active: false,
                cross_filtering_behavior: Some("bothDirections".into()),
                from_cardinality: Some("one".into()),
                to_cardinality: Some("many".into()),
                annotations: vec![Annotation {
                    name: "A".into(),
                    value: "1".into(),
                }],
                extra: props(
                    json!({"relyOnReferentialIntegrity": true, "joinOnDateBehavior": "datePartOnly"}),
                ),
            },
            // Incomplete: the endpoint text is kept as written.
            Relationship {
                name: "incomplete".into(),
                is_active: true,
                extra: props(json!({"fromColumn": "Order Lines", "isActive": true})),
                ..Default::default()
            },
        ],
        expressions: vec![
            NamedExpression {
                name: "Server Name".into(),
                kind: "m".into(),
                expression:
                    "\"localhost\" meta [IsParameterQuery=true, Type=\"Text\", IsParameterQueryRequired=true]"
                        .into(),
                lineage_tag: Some("e-1".into()),
                annotations: vec![Annotation {
                    name: "PBI_ResultType".into(),
                    value: "Text".into(),
                }],
                extra: props(json!({"queryGroup": "Group 1"})),
                ..Default::default()
            },
            NamedExpression {
                name: "DatabaseQuery".into(),
                kind: "m".into(),
                expression: "let
    database = Sql.Database(\"srv\", \"db\")
in
    database"
                    .into(),
                lineage_tag: Some("e-2".into()),
                ..Default::default()
            },
            NamedExpression {
                name: "Staging".into(),
                kind: "m".into(),
                expression: "let\n    Source = 1\nin\n    Source".into(),
                description: Some("Staging query.".into()),
                ..Default::default()
            },
        ],
        roles: vec![
            Role {
                name: "../Readers".into(),
                model_permission: Some("read".into()),
                table_permissions: vec![
                    TablePermission {
                        table: "Order Lines".into(),
                        filter_expression: Some("'Order Lines'[Key] > 0\n    && TRUE()".into()),
                        extra: props(json!({"metadataPermission": "read"})),
                    },
                    TablePermission {
                        table: "Sales.Archive".into(),
                        filter_expression: None,
                        extra: props(json!({"metadataPermission": "none"})),
                    },
                ],
                annotations: vec![Annotation {
                    name: "Owner".into(),
                    value: "Finance".into(),
                }],
                extra: props(json!({
                    "description": "Readers.",
                    "members": [
                        {"memberName": "a@contoso.com", "memberType": "user", "identityProvider": "AzureAD"},
                        {"memberName": "b@contoso.com", "memberType": "group", "identityProvider": "AzureAD"}
                    ]
                })),
            },
            Role {
                name: "Admins".into(),
                model_permission: Some("administrator".into()),
                ..Default::default()
            },
        ],
        annotations: vec![Annotation {
            name: "PBI_QueryOrder".into(),
            value: "[\"Server Name\",\"Staging\"]".into(),
        }],
        database_extra: props(json!({"description": "The database.", "compatibilityMode": "powerBI"})),
        extra: props(json!({
            "defaultPowerBIDataSourceVersion": "powerBI_V3",
            "discourageImplicitMeasures": true,
            "dataAccessOptions": {"legacyRedirects": true, "returnErrorValuesAsNull": true},
            "queryGroups": [{"folder": "Group 1", "description": "First."}, {"folder": "Group 1\\Sub"}],
            "dataSources": [{"name": "DS 1", "type": "structured", "connectionDetails": {"protocol": "tds", "address": {"server": "srv", "database": "1234"}}}],
            "functions": [{"name": "AddOne", "expression": "(x : NUMERIC) => x + 1"}],
            "perspectives": [{"name": "Sales / View", "tables": [{"name": "Order Lines", "includeAll": true, "measures": [{"name": "Empty"}]}]}],
            "cultures": [
                {"name": "en-US", "linguisticMetadata": {"content": {"Version": "1.0.0", "Language": "en-US"}, "contentType": "json"}},
                {"name": "fr-FR", "translations": {"model": {"name": "Model", "translatedCaption": "Modèle", "tables": [
                    {"name": "Order Lines", "translatedCaption": "Lignes", "columns": [{"name": "Key", "translatedCaption": "Clé", "translatedDisplayFolder": "Dossier"}]}
                ]}}}
            ],
            "extendedProperties": [{"name": "E", "value": "v"}]
        })),
    }
}

/// A guarded temp folder and a path inside it that does not exist yet.
fn temp_dir(tag: &str) -> (TempDir, PathBuf) {
    let tmp = TempDir::new(&format!("tmdl-{tag}"));
    let dir = tmp.join("out");
    (tmp, dir)
}

/// Every file below `dir`, as paths relative to it.
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path.strip_prefix(dir).unwrap().to_path_buf());
            }
        }
    }
    out.sort();
    out
}

#[test]
fn model_to_tmdl_to_model_is_lossless() {
    let model = sample();
    let docs = write_documents(&model);
    let back = read_documents(&docs).unwrap();
    for (a, b) in model.tables.iter().zip(&back.tables) {
        assert_eq!(a, b, "table {:?}", a.name);
    }
    assert_eq!(back.relationships, model.relationships);
    assert_eq!(back.expressions, model.expressions);
    assert_eq!(back.roles, model.roles);
    for (k, v) in &model.extra {
        assert_eq!(back.extra.get(k), Some(v), "model extra {k}");
    }
    assert_eq!(back.extra, model.extra);
    assert_eq!(back.database_extra, model.database_extra);
    assert_eq!(back, model);
    // ... and the second write is byte-identical to the first.
    assert_eq!(write_documents(&back), docs);
    assert_eq!(write_documents(&model), docs);
}

#[test]
fn layout_follows_desktop() {
    let docs = write_documents(&sample());
    let paths: Vec<&str> = docs.iter().map(|(p, _)| p.as_str()).collect();
    for expected in [
        "database.tmdl",
        "model.tmdl",
        "relationships.tmdl",
        "expressions.tmdl",
        "dataSources.tmdl",
        "functions.tmdl",
        "tables/Order Lines.tmdl",
        "tables/..%2Fescape.tmdl",
        "tables/%2Fetc%2Fpasswd.tmdl",
        "tables/C%3A%5CWindows%5Cwin.ini.tmdl",
        "tables/a%2Fb.tmdl",
        "tables/a%2Fb~2.tmdl",
        "tables/%43ON.tmdl",
        "tables/%20lead.tmdl",
        "tables/tab%09name.tmdl",
        "roles/..%2FReaders.tmdl",
        "perspectives/Sales %2F View.tmdl",
        "cultures/en-US.tmdl",
        "cultures/fr-FR.tmdl",
    ] {
        assert!(paths.contains(&expected), "{expected} missing from {paths:#?}");
    }
    for (path, text) in &docs {
        assert!(!text.starts_with('\u{feff}'), "{path}: no BOM");
        assert!(text.ends_with("\r\n\r\n"), "{path}: ends with a blank line");
        assert!(!text.replace("\r\n", "").contains('\n'), "{path}: CRLF only");
        assert!(path.split('/').all(|c| c != ".." && c != "."), "{path}");
    }
    let model_tmdl = &docs.iter().find(|(p, _)| p == "model.tmdl").unwrap().1;
    assert!(
        model_tmdl.contains(
            "ref table 'Order Lines'\r\nref table 'Time Intelligence'\r\nref table '../escape'\r\n"
        )
    );
    assert!(model_tmdl.contains("\r\n\r\nref role '../Readers'\r\nref role Admins\r\n\r\n"));
    assert!(model_tmdl.contains(
        "ref perspective 'Sales / View'\r\n\r\nref cultureInfo en-US\r\nref cultureInfo fr-FR\r\n\r\n"
    ));
    assert!(model_tmdl.starts_with("/// Model description\r\nmodel Model\r\n\tculture: en-US\r\n\tdefaultPowerBIDataSourceVersion: powerBI_V3\r\n\tdiscourageImplicitMeasures\r\n"));
}

#[test]
fn hostile_names_stay_inside_the_folder() {
    let (_tmp, root) = temp_dir("hostile");
    let dir = root.join("Model.SemanticModel/definition");
    // Names with control characters have no TMDL form Desktop reads (G24): the folder writer
    // refuses them before touching the disk.
    let refused = write_folder(&sample(), &dir).unwrap_err();
    assert!(
        matches!(refused, bicraft_tmdl::Error::UnwritableName(ref n) if n == "tab	name"),
        "{refused}"
    );
    assert!(!dir.exists());
    let mut model = sample();
    model.tables.retain(|t| !t.name.chars().any(char::is_control));
    assert_eq!(bicraft_tmdl::unwritable_name(&model), None);
    write_folder(&model, &dir).unwrap();
    // Nothing was written next to or above `definition/`.
    assert_eq!(
        files(&root.join("Model.SemanticModel"))
            .iter()
            .filter(|p| !p.starts_with("definition"))
            .count(),
        0
    );
    let written = files(&dir);
    assert_eq!(written.len(), write_documents(&model).len());
    for path in &written {
        let mut parts = path.components();
        assert!(parts.clone().count() <= 2, "{path:?}");
        assert!(
            parts.all(|c| matches!(c, std::path::Component::Normal(_))),
            "{path:?}"
        );
    }
    assert_eq!(read_folder(&dir).unwrap(), model);

    // Rewriting is stable and removes only stale `.tmdl` files.
    let mut smaller = model.clone();
    smaller.tables.retain(|t| t.name != "a/b" && t.name != "CON");
    write_folder(&smaller, &dir).unwrap();
    assert!(!dir.join("tables/%43ON.tmdl").exists());
    assert_eq!(read_folder(&dir).unwrap(), smaller);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn file_stems_are_exposed() {
    assert_eq!(file_stem("a/b"), "a%2Fb");
    assert_eq!(file_stem(".."), "..");
}

#[cfg(unix)]
#[test]
fn symlinked_sub_folders_are_refused() {
    let (_tmp, root) = temp_dir("symlink");
    let outside = root.join("outside");
    let dir = root.join("definition");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("tables")).unwrap();
    assert!(write_folder(&sample(), &dir).is_err());
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
    let _ = std::fs::remove_dir_all(&root);
}

/// Dev aid (Windows): `BICRAFT_TMDL_DUMP=<dir> cargo test -p bicraft-tmdl -- --ignored dump`
/// writes the sample so Desktop's TMDL serializer can be asked to load it.
#[test]
#[ignore = "writes to the folder named by BICRAFT_TMDL_DUMP"]
fn dump_sample_for_desktop_validation() {
    if let Ok(dir) = bicraft_env::var("BICRAFT_TMDL_DUMP") {
        write_folder(&sample(), Path::new(&dir)).unwrap();
    }
}

/// G23: stale files are found without canonicalizing every path; a name differing only in case
/// is the same file on a case-insensitive file system (kept) and a stale one elsewhere (removed).
#[test]
fn case_only_name_differences_leave_one_table_file() {
    let model = SemanticModel {
        name: "M".into(),
        tables: vec![data_table("Sales")],
        ..Default::default()
    };
    let (_tmp, dir) = temp_dir("case");
    write_folder(&model, &dir).unwrap();
    std::fs::write(dir.join("tables/sales.tmdl"), "table Sales\n").unwrap();
    write_folder(&model, &dir).unwrap();
    assert_eq!(files(&dir.join("tables")).len(), 1);
    assert_eq!(read_folder(&dir).unwrap(), model);
    let _ = std::fs::remove_dir_all(&dir);
}
