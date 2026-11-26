//! G24: TMDL edge cases found by diffing bicraft's writer and reader against the TOM serializer
//! of Power BI Desktop 2.158 (TOM 17.0.87.18) with the TOM oracle. Each test pins what
//! Desktop writes, or what it accepts on reading, for one case.

use bicraft_model::{Column, Partition, Role, SemanticModel, Table};
use bicraft_tmdl::{read_documents, unwritable_name, write_documents};
use serde_json::{Value, json};

fn props(v: Value) -> bicraft_model::Properties {
    match v {
        Value::Object(m) => m,
        _ => panic!("not an object"),
    }
}

/// The text of document `path`, with LF line endings.
fn doc(model: &SemanticModel, path: &str) -> String {
    let docs = write_documents(model);
    let (_, text) = docs
        .iter()
        .find(|(p, _)| p == path)
        .unwrap_or_else(|| panic!("{path} not written"));
    text.replace("\r\n", "\n")
}

fn back(model: &SemanticModel) -> SemanticModel {
    read_documents(&write_documents(model)).unwrap()
}

fn table(name: &str) -> Table {
    Table {
        name: name.into(),
        columns: vec![Column {
            name: "A".into(),
            source_column: Some("A".into()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn role_members_use_tmdl_member_types() {
    let mut model = SemanticModel::default();
    model.roles.push(Role {
        name: "R".into(),
        extra: props(json!({"members": [
            {"memberName": "u@x", "identityProvider": "AzureAD", "memberType": "user"},
            {"memberName": "g@x", "memberId": "id-1", "identityProvider": "AzureAD", "memberType": "group"},
            {"memberName": "a@x", "identityProvider": "AzureAD"},
            {"memberName": "o@x", "identityProvider": "Other", "memberType": "user"},
            {"memberName": "DOMAIN\\u", "memberId": "S-1-5"}
        ]})),
        ..Default::default()
    });
    let text = doc(&model, "roles/R.tmdl");
    assert!(
        text.contains(
            "\tmember u@x\n\tmember g@x = group\n\tmember a@x = auto\n\tmember o@x\n\t\tidentityProvider: Other\n\tmember DOMAIN\\u = activeDirectory\n"
        ),
        "{text}"
    );
    // `memberId` has no TMDL form (Desktop refuses it); the rest reads back as written.
    assert!(!text.contains("memberId"));
    let members = &back(&model).roles[0].extra["members"];
    assert_eq!(
        members,
        &json!([
            {"memberName": "u@x", "identityProvider": "AzureAD", "memberType": "user"},
            {"memberName": "g@x", "identityProvider": "AzureAD", "memberType": "group"},
            {"memberName": "a@x", "identityProvider": "AzureAD"},
            {"memberName": "o@x", "identityProvider": "Other", "memberType": "user"},
            {"memberName": "DOMAIN\\u"}
        ])
    );
}

/// Regression: a member type without an identity provider is an external member in TMSL's
/// published schema (a Windows member has no member type). It was written `= activeDirectory`
/// and lost its type. It now keeps its type, written as Desktop 2.159 opens and saves it back
/// (no `identityProvider` line: Desktop refuses a project with `identityProvider: ""`), and so
/// reads back with the default provider.
#[test]
fn role_member_type_without_identity_provider_keeps_its_type() {
    let mut model = SemanticModel::default();
    model.roles.push(Role {
        name: "R".into(),
        extra: props(json!({"members": [
            {"memberName": "DOMAIN\\u", "memberType": "user"},
            {"memberName": "DOMAIN\\g", "memberType": "group"},
            {"memberName": "DOMAIN\\a", "memberType": "auto"},
            {"memberName": "DOMAIN\\w"},
            {"memberName": "e@x", "identityProvider": "AzureAD", "memberType": "group"}
        ]})),
        ..Default::default()
    });
    let text = doc(&model, "roles/R.tmdl");
    assert!(
        text.contains(
            "\tmember DOMAIN\\u\n\tmember DOMAIN\\g = group\n\tmember DOMAIN\\a = auto\n\
             \tmember DOMAIN\\w = activeDirectory\n\tmember e@x = group\n"
        ),
        "{text}"
    );
    assert!(!text.contains("identityProvider"), "{text}");
    assert_eq!(
        back(&model).roles[0].extra["members"],
        json!([
            {"memberName": "DOMAIN\\u", "identityProvider": "AzureAD", "memberType": "user"},
            {"memberName": "DOMAIN\\g", "identityProvider": "AzureAD", "memberType": "group"},
            {"memberName": "DOMAIN\\a", "identityProvider": "AzureAD"},
            {"memberName": "DOMAIN\\w"},
            {"memberName": "e@x", "identityProvider": "AzureAD", "memberType": "group"}
        ])
    );
}

#[test]
fn data_source_types_options_and_credentials() {
    let mut model = SemanticModel::default();
    model.extra.insert(
        "dataSources".into(),
        json!([
            {"name": "P", "connectionString": "x", "maxConnections": 2},
            {"type": "structured", "name": "S", "connectionDetails": {"protocol": "tds"},
             "options": {}, "credential": {"AuthenticationKind": "Windows", "EncryptConnection": true}},
            {"type": "structured", "name": "T", "credential": {"kind": "SQL", "Password": "p"}}
        ]),
    );
    let text = doc(&model, "dataSources.tmdl");
    assert!(
        text.starts_with("dataSource P = provider\n\tmaxConnections: 2\n\tconnectionString: x\n"),
        "{text}"
    );
    assert!(
        text.contains("dataSource S\n\tconnectionDetails\n\t\tprotocol: tds\n\toptions = {}\n\tcredential\n\t\tauthenticationKind: Windows\n\t\tencryptConnection\n"),
        "{text}"
    );
    assert!(
        text.contains("\tcredential =\n\t\t\t{\n\t\t\t  \"kind\": \"SQL\"\n\t\t\t}\n\t\tpassword: p\n"),
        "{text}"
    );
    let again = back(&model);
    assert_eq!(again.extra["dataSources"], model.extra["dataSources"]);
}

#[test]
fn policy_range_dates_have_no_time_of_day() {
    let mut model = SemanticModel::default();
    let mut t = table("T");
    t.partitions.push(Partition {
        name: "2024".into(),
        source_type: "policyRange".into(),
        source_extra: props(
            json!({"granularity": "year", "start": "2024-01-01T00:00:00", "end": "2025-01-01"}),
        ),
        ..Default::default()
    });
    model.tables.push(t);
    let text = doc(&model, "tables/T.tmdl");
    // Desktop's reader refuses a time; source properties follow Desktop's order.
    assert!(
        text.contains(
            "\t\tsource\n\t\t\tstart: 2024-01-01\n\t\t\tend: 2025-01-01\n\t\t\tgranularity: year\n"
        ),
        "{text}"
    );
    let p = &back(&model).tables[0].partitions[0];
    assert_eq!(p.source_extra["start"], "2024-01-01T00:00:00");
    assert_eq!(p.source_extra["end"], "2025-01-01T00:00:00");
}

#[test]
fn levels_and_calculation_items_are_written_in_ordinal_order() {
    let mut model = SemanticModel::default();
    let mut t = table("T");
    t.extra = props(json!({
        "hierarchies": [{"name": "H", "levels": [
            {"name": "B", "ordinal": 3, "column": "A"},
            {"name": "A", "ordinal": 1, "column": "A"}
        ]}],
        "calculationGroup": {"calculationItems": [
            {"name": "None", "expression": "1"},
            {"name": "Five", "ordinal": 5, "expression": "5"},
            {"name": "Two", "ordinal": 2, "expression": "2"}
        ]}
    }));
    model.tables.push(t);
    let text = doc(&model, "tables/T.tmdl");
    assert!(!text.contains("ordinal"), "{text}");
    let a = text.find("level A").unwrap();
    assert!(a < text.find("level B").unwrap());
    let two = text.find("calculationItem Two").unwrap();
    let five = text.find("calculationItem Five").unwrap();
    assert!(two < five && five < text.find("calculationItem None").unwrap());
    let again = back(&model);
    let items = &again.tables[0].extra["calculationGroup"]["calculationItems"];
    assert_eq!(items[0]["name"], "Two");
    assert_eq!(items[2]["ordinal"], 2);
}

#[test]
fn calculation_group_partition_and_calculated_table_columns_are_implied() {
    let mut model = SemanticModel::default();
    let mut cg = table("CG");
    cg.extra = props(json!({"calculationGroup": {"precedence": 1}}));
    cg.partitions.push(Partition {
        name: "Partition".into(),
        mode: Some("import".into()),
        source_type: "calculationGroup".into(),
        ..Default::default()
    });
    let mut other = cg.clone();
    other.name = "Named".into();
    other.partitions[0].name = "Custom".into();
    let mut calc = table("Calc");
    calc.columns[0].extra = props(json!({"type": "calculatedTableColumn", "isNameInferred": true}));
    calc.partitions.push(Partition {
        name: "Calc".into(),
        source_type: "calculated".into(),
        expression: Some("{1}".into()),
        ..Default::default()
    });
    model.tables = vec![cg, other, calc];
    assert!(!doc(&model, "tables/CG.tmdl").contains("partition"));
    assert!(doc(&model, "tables/Named.tmdl").contains("partition Custom = calculationGroup"));
    assert!(!doc(&model, "tables/Calc.tmdl").contains("calculatedTableColumn"));
    assert_eq!(back(&model), model);
}

#[test]
fn tmsl_only_sets_are_not_written() {
    let mut model = SemanticModel::default();
    let mut t = table("T");
    t.extra = props(json!({"sets": [{"name": "S", "expression": "{1}"}]}));
    model.tables.push(t);
    model.extra.insert(
        "perspectives".into(),
        json!([{"name": "P", "tables": [{"name": "T", "sets": [{"name": "S"}]}]}]),
    );
    assert!(!doc(&model, "tables/T.tmdl").contains("set"));
    assert!(!doc(&model, "perspectives/P.tmdl").contains("Set"));
}

#[test]
fn text_values_starting_with_a_double_quote_are_quoted_only_when_they_also_end_with_one() {
    let mut model = SemanticModel::default();
    let mut t = table("T");
    t.columns[0].format_string = Some("\"$\"#,0".into());
    t.columns.push(Column {
        name: "B".into(),
        format_string: Some("\"q\"".into()),
        display_folder: Some("\"".into()),
        ..Default::default()
    });
    model.tables.push(t);
    let text = doc(&model, "tables/T.tmdl");
    assert!(text.contains("\t\tformatString: \"$\"#,0\n"), "{text}");
    assert!(text.contains("\t\tformatString: \"\"\"q\"\"\"\n"), "{text}");
    assert_eq!(back(&model), model);
}

#[test]
fn names_with_control_characters_are_reported() {
    let mut model = SemanticModel::default();
    model.tables.push(table("T"));
    assert_eq!(unwritable_name(&model), None);
    model.tables[0].columns[0].name = "line\nbreak".into();
    assert_eq!(unwritable_name(&model).as_deref(), Some("line\nbreak"));
    model.tables[0].columns[0].name = "A".into();
    model
        .extra
        .insert("queryGroups".into(), json!([{"folder": "tab\there"}]));
    assert_eq!(unwritable_name(&model).as_deref(), Some("tab\there"));
}

#[test]
fn space_indented_expressions_alone_give_the_level_width() {
    // The only indented lines are an expression body: two levels of 2 spaces below `expression`.
    let docs = vec![(
        "expressions.tmdl".to_string(),
        "expression E =\n    let\n      x = 1\n    in\n      x\n\nexpression F = 2\n".to_string(),
    )];
    let m = read_documents(&docs).unwrap();
    assert_eq!(m.expressions[0].expression, "let\n  x = 1\nin\n  x");
    assert_eq!(m.expressions[1].expression, "2");
    // Entry lines still win: 4-space levels, expression 8 spaces in.
    let docs = vec![(
        "tables/T.tmdl".to_string(),
        "table T\n    measure M =\n            1 +\n              2\n        formatString: 0\n".to_string(),
    )];
    let m = read_documents(&docs).unwrap();
    assert_eq!(m.tables[0].measures[0].expression, "1 +\n  2");
}

#[test]
fn new_tom_object_kinds_are_written_as_objects() {
    let mut model = SemanticModel::default();
    let mut t = table("T");
    t.partitions.push(Partition {
        name: "P".into(),
        source_type: "m".into(),
        expression: Some("1".into()),
        extra: props(json!({"dataCoverageDefinition": {"description": "d", "expression": "TRUE()"}})),
        ..Default::default()
    });
    t.extra = props(json!({"calendars": [{"name": "C", "calendarColumnGroups": [
        {"timeUnit": "year", "primaryColumn": "A"},
        {"columns": ["A", "B C"]}
    ]}]}));
    model.tables.push(t);
    model.extra.insert(
        "bindingInfoCollection".into(),
        json!([{"type": "dataBindingHint", "name": "B", "connectionId": "c"}]),
    );
    let text = doc(&model, "tables/T.tmdl");
    assert!(
        text.contains("\n\t\t/// d\n\t\tdataCoverageDefinition = TRUE()\n"),
        "{text}"
    );
    assert!(
        text.contains("\tcalendar C\n\n\t\tcalendarColumnGroup = year\n\t\t\tprimaryColumn: A\n\n\t\tcalendarColumnGroup\n\t\t\tcolumn: A\n\t\t\tcolumn: 'B C'\n"),
        "{text}"
    );
    assert!(
        doc(&model, "model.tmdl").contains("\nbindingInfo B\n\ttype: dataBindingHint\n\tconnectionId: c\n")
    );
    assert_eq!(back(&model), model);
}

#[test]
fn field_parameter_group_by_columns_are_one_line_each() {
    let mut model = SemanticModel::default();
    let mut t = table("Param");
    t.columns[0].extra = props(json!({"relatedColumnDetails": {"groupByColumns": [
        {"groupingColumn": "Param Fields"}, {"groupingColumn": "Other"}
    ]}}));
    model.tables.push(t);
    let text = doc(&model, "tables/Param.tmdl");
    assert!(
        text.contains(
            "\n\n\t\trelatedColumnDetails\n\t\t\tgroupByColumn: 'Param Fields'\n\t\t\tgroupByColumn: Other\n"
        ),
        "{text}"
    );
    assert_eq!(back(&model), model);
}

#[test]
fn string_extended_properties_have_no_type_in_tmsl() {
    let mut model = SemanticModel::default();
    model.extra.insert(
        "extendedProperties".into(),
        json!([{"name": "S", "value": "text"}, {"type": "json", "name": "J", "value": {"a": 1}}]),
    );
    let again = back(&model);
    assert_eq!(
        again.extra["extendedProperties"],
        model.extra["extendedProperties"]
    );
}

#[test]
fn a_role_member_with_child_objects_is_followed_by_a_blank_line() {
    let mut model = SemanticModel::default();
    model.roles.push(Role {
        name: "R".into(),
        extra: props(json!({"members": [
            {"memberName": "A", "annotations": [{"name": "N", "value": "1"}, {"name": "M", "value": "2"}]},
            {"memberName": "B"},
            {"memberName": "C"}
        ]})),
        ..Default::default()
    });
    let text = doc(&model, "roles/R.tmdl");
    assert!(
        text.contains("\n\tmember A = activeDirectory\n\t\tannotation N = 1\n\n\t\tannotation M = 2\n\n\tmember B = activeDirectory\n\tmember C = activeDirectory\n"),
        "{text}"
    );
}

/// G16: objects of compatibility level 1000000 read from `metadata.sqlitedb` (shapes from the
/// engine's TOM). Desktop 2.158's client TOM knows only binding infos among them, so TMDL has no
/// reference text for the others; bicraft's own TMDL keeps them all.
#[test]
fn preview_objects_survive_tmdl() {
    let mut model = SemanticModel {
        compatibility_level: Some(1_000_000),
        ..Default::default()
    };
    let mut t = table("T");
    t.extra = props(json!({
        "directLakeIndexingBehavior": "explicit",
        "excludedArtifacts": [{"artifactType": 4, "reference": {"name": "X"}}]
    }));
    model.tables.push(t);
    model.expressions.push(bicraft_model::NamedExpression {
        name: "Q".into(),
        kind: "m".into(),
        expression: "1".into(),
        extra: props(json!({"excludedArtifacts": [{"artifactType": 3, "reference": {"name": "S"}}]})),
        ..Default::default()
    });
    model.extra = props(json!({
        "metadataAccessPolicy": "calculationDefinitions",
        "defaultDirectLakeIndexingBehavior": "full",
        "analyticsAIMetadata": [{"name": "A", "measureAnalysisDefinition": {"v": 1}}],
        "excludedArtifacts": [{"artifactType": 7, "reference": {"name": "R"}}],
        "bindingInfoCollection": [{"type": "dataBindingHint", "name": "B", "description": "d", "connectionId": "c"}]
    }));
    let text = doc(&model, "model.tmdl");
    assert!(
        text.contains("bindingInfo B\n\ttype: dataBindingHint\n\tconnectionId: c\n"),
        "{text}"
    );
    let t = doc(&model, "tables/T.tmdl");
    assert!(
        t.contains("\texcludedArtifact\n\t\tartifactType: 4\n\t\treference\n\t\t\tname: X\n"),
        "{t}"
    );
    assert_eq!(back(&model), model);
}
