//! `apply_model` (model → metadata rows) over the synthetic database of gen_fixture.py.

use bicraft_metadata::{
    Error, MetaRow, MetaValue, MetadataTables, apply_model, apply_model_at, build_model,
    tables_from_sqlitedb, tables_to_rows,
};
use bicraft_model::{Annotation, Measure, Role, SemanticModel, TablePermission};

const FIXTURE: &[u8] = include_bytes!("fixtures/synthetic_metadata.sqlitedb");
/// A fixed FILETIME clock (2026-01-01T00:00:00Z) so outputs are deterministic.
const NOW: i64 = 134_116_992_000_000_000;

fn base() -> MetadataTables {
    tables_from_sqlitedb(FIXTURE).unwrap()
}

fn get<'a>(r: &'a MetaRow, col: &str) -> &'a MetaValue {
    r.get(col).unwrap_or(&MetaValue::Null)
}

fn int(r: &MetaRow, col: &str) -> Option<i64> {
    match get(r, col) {
        MetaValue::Integer(i) => Some(*i),
        _ => None,
    }
}

fn row_by_id<'a>(t: &'a MetadataTables, table: &str, id: i64) -> Option<&'a MetaRow> {
    t.rows(table).iter().find(|r| int(r, "ID") == Some(id))
}

fn maxid(t: &MetadataTables) -> Option<i64> {
    t.rows("DBPROPERTIES")
        .iter()
        .find(|r| get(r, "NAME") == &MetaValue::from("MAXID"))
        .and_then(|r| int(r, "VALUE"))
}

/// Applies `m`, checks the round trip `build_model(apply_model(base, m)) == m`, returns the rows.
fn round_trip(m: &SemanticModel) -> MetadataTables {
    let out = apply_model_at(&base(), m, NOW).unwrap();
    assert_eq!(&build_model(&out).unwrap(), m);
    out
}

/// Tables whose rows changed.
fn changed(out: &MetadataTables) -> Vec<String> {
    let b = base();
    tables_to_rows(out)
        .changed_tables(&b)
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn unsupported(r: Result<MetadataTables, Error>) -> Vec<String> {
    match r {
        Err(Error::UnsupportedChange(list)) => list.iter().map(ToString::to_string).collect(),
        other => panic!("expected UnsupportedChange, got {other:?}"),
    }
}

#[test]
fn identity_keeps_every_row() {
    let b = base();
    let m = build_model(&b).unwrap();
    assert_eq!(apply_model(&b, &m).unwrap(), b);
    assert_eq!(apply_model_at(&b, &m, NOW).unwrap(), b);
}

#[test]
fn adding_a_measure_appends_one_row() {
    let mut m = build_model(&base()).unwrap();
    let sales = m.table_mut("Sales").unwrap();
    sales.measures.push(Measure {
        name: "One".into(),
        expression: "1".into(),
        format_string: Some("0".into()),
        display_folder: Some("Tests".into()),
        description: Some("A constant".into()),
        lineage_tag: Some("00000000-0000-4000-8000-000000000001".into()),
        annotations: vec![Annotation {
            name: "PBI_FormatHint".into(),
            value: "{}".into(),
        }],
        ..Default::default()
    });
    let out = round_trip(&m);
    assert_eq!(changed(&out), ["Annotation", "DBPROPERTIES", "Measure"]);
    let b = base();
    assert_eq!(out.rows("Measure").len(), b.rows("Measure").len() + 1);
    // Untouched rows are identical; the new ones take IDs above MAXID (52).
    assert_eq!(out.rows("Measure").get(..2), b.rows("Measure").get(..2));
    let new = row_by_id(&out, "Measure", 53).unwrap();
    assert_eq!(int(new, "TableID"), Some(3));
    assert_eq!(int(new, "DataType"), Some(6), "int64 literal");
    assert_eq!(int(new, "State"), Some(1));
    assert_eq!(int(new, "ModifiedTime"), Some(NOW));
    assert_eq!(int(new, "StructureModifiedTime"), Some(NOW));
    for zero in [
        "KPIID",
        "IsSimpleMeasure",
        "DetailRowsDefinitionID",
        "FormatStringDefinitionID",
        "IsHidden",
    ] {
        assert_eq!(int(new, zero), Some(0), "{zero}");
    }
    assert_eq!(get(new, "ErrorMessage"), &MetaValue::Null);
    assert_eq!(
        new.len(),
        b.rows("Measure")[0].len(),
        "same columns as existing rows"
    );
    let ann = row_by_id(&out, "Annotation", 54).unwrap();
    assert_eq!(
        (int(ann, "ObjectID"), int(ann, "ObjectType")),
        (Some(53), Some(8))
    );
    assert_eq!(int(ann, "ModifiedTime"), Some(NOW));
    assert_eq!(maxid(&out), Some(54));
}

#[test]
fn new_measures_get_a_generated_lineage_tag() {
    let mut m = build_model(&base()).unwrap();
    m.table_mut("Calendar").unwrap().measures.push(Measure {
        name: "Days".into(),
        expression: "COUNTROWS(Calendar)".into(),
        ..Default::default()
    });
    let out = apply_model_at(&base(), &m, NOW).unwrap();
    let back = build_model(&out).unwrap();
    let tag = back.measure("Days").unwrap().1.lineage_tag.clone().unwrap();
    assert_eq!(tag.len(), 36);
    assert_eq!(tag, tag.to_lowercase());
    let row = row_by_id(&out, "Measure", 53).unwrap();
    assert_eq!(
        int(row, "DataType"),
        Some(20),
        "variant when the type needs DAX inference"
    );
    // Deterministic for a fixed clock; filled in, the model round-trips.
    assert_eq!(apply_model_at(&base(), &m, NOW).unwrap(), out);
    m.table_mut("Calendar").unwrap().measures[0].lineage_tag = Some(tag);
    assert_eq!(back, m);
}

#[test]
fn changing_a_measure_updates_its_row_only() {
    let mut m = build_model(&base()).unwrap();
    let sales = m.table_mut("Sales").unwrap();
    let total = sales.measure_mut("Total").unwrap();
    total.expression = "SUM(Sales[Amount]) * 2".into();
    total.format_string = None;
    total.display_folder = Some("Other".into());
    total.description = Some("Doubled".into());
    total.is_hidden = true;
    total.extra.insert("dataCategory".into(), "Money".into());
    let rows = sales.measure_mut("Rows").unwrap();
    rows.description = Some("Changed".into());
    let out = round_trip(&m);
    assert_eq!(changed(&out), ["Measure"]);
    let b = base();
    let (old, new) = (
        row_by_id(&b, "Measure", 10).unwrap(),
        row_by_id(&out, "Measure", 10).unwrap(),
    );
    for (k, v) in old {
        let expect_change = [
            "Expression",
            "FormatString",
            "DisplayFolder",
            "Description",
            "IsHidden",
            "DataCategory",
            "ModifiedTime",
            "StructureModifiedTime",
        ]
        .contains(&k.as_str());
        assert_eq!(get(new, k) != v, expect_change, "{k}");
    }
    let rows = row_by_id(&out, "Measure", 9).unwrap();
    assert_eq!(int(rows, "ModifiedTime"), Some(NOW));
    assert_ne!(
        int(rows, "StructureModifiedTime"),
        Some(NOW),
        "description is not structural"
    );
}

#[test]
fn removing_a_measure_removes_its_annotations() {
    let mut m = build_model(&base()).unwrap();
    m.table_mut("Sales")
        .unwrap()
        .measures
        .retain(|x| x.name != "Total");
    let out = round_trip(&m);
    assert!(row_by_id(&out, "Measure", 10).is_none());
    assert!(row_by_id(&out, "Annotation", 19).is_none());
    assert_eq!(changed(&out), ["Annotation", "Measure"]);
    assert_eq!(maxid(&out), Some(52), "no ID allocated");
}

#[test]
fn moving_a_measure_changes_its_table() {
    let mut m = build_model(&base()).unwrap();
    let total = {
        let sales = m.table_mut("Sales").unwrap();
        let i = sales.measures.iter().position(|x| x.name == "Total").unwrap();
        sales.measures.remove(i)
    };
    m.table_mut("Calendar").unwrap().measures.push(total);
    let out = round_trip(&m);
    let row = row_by_id(&out, "Measure", 10).unwrap();
    assert_eq!(int(row, "TableID"), Some(2));
    assert_eq!(int(row, "StructureModifiedTime"), Some(NOW));
}

#[test]
fn column_and_table_properties() {
    let mut m = build_model(&base()).unwrap();
    m.description = Some("Model description".into());
    let cal = m.table_mut("Calendar").unwrap();
    cal.description = None;
    cal.is_hidden = false;
    let month = cal.column_mut("Month").unwrap();
    month.sort_by_column = None;
    month.description = Some("Month name".into());
    let sales = m.table_mut("Sales").unwrap();
    let amount = sales.column_mut("Amount").unwrap();
    amount.summarize_by = Some("average".into());
    amount.format_string = Some("#,0".into());
    amount.display_folder = Some("Values".into());
    amount.data_category = Some("Currency".into());
    amount.is_hidden = true;
    let day = sales.column_mut("Day").unwrap();
    day.sort_by_column = Some("Amount".into());
    day.summarize_by = None;
    let out = round_trip(&m);
    assert_eq!(changed(&out), ["Column", "Model", "Table"]);
    let amount = row_by_id(&out, "Column", 4).unwrap();
    assert_eq!(int(amount, "SummarizeBy"), Some(7));
    assert_eq!(
        int(row_by_id(&out, "Column", 6).unwrap(), "SortByColumnID"),
        Some(4)
    );
    assert_eq!(
        int(row_by_id(&out, "Column", 8).unwrap(), "SortByColumnID"),
        Some(0)
    );
}

#[test]
fn annotations_are_added_changed_and_removed() {
    let mut m = build_model(&base()).unwrap();
    m.annotations[0].value = r#"["Sales","Calendar"]"#.into();
    m.annotations.push(Annotation {
        name: "PBIDesktopVersion".into(),
        value: "test".into(),
    });
    m.table_mut("Sales").unwrap().annotations.clear();
    m.table_mut("Calendar").unwrap().annotations.push(Annotation {
        name: "Note".into(),
        value: "x".into(),
    });
    let out = round_trip(&m);
    assert_eq!(changed(&out), ["Annotation", "DBPROPERTIES"]);
    let b = base();
    assert_eq!(out.rows("Annotation").len(), b.rows("Annotation").len() + 1);
    let changed_row = row_by_id(&out, "Annotation", 15).unwrap();
    assert_eq!(int(changed_row, "ModifiedTime"), Some(NOW));
    assert!(row_by_id(&out, "Annotation", 16).is_none());
    assert_eq!(
        row_by_id(&out, "Annotation", 17),
        row_by_id(&b, "Annotation", 17),
        "untouched"
    );
}

#[test]
fn roles_and_row_level_security() {
    let mut m = build_model(&base()).unwrap();
    m.roles[0].table_permissions[0].filter_expression = Some("Sales[Amount] > 10".into());
    m.roles[0].table_permissions.push(TablePermission {
        table: "Calendar".into(),
        filter_expression: Some("Calendar[Date] >= DATE(2020,6,1)".into()),
        ..Default::default()
    });
    m.roles.push(Role {
        name: "Nobody".into(),
        model_permission: Some("read".into()),
        table_permissions: vec![TablePermission {
            table: "Sales".into(),
            filter_expression: Some("FALSE()".into()),
            ..Default::default()
        }],
        ..Default::default()
    });
    m.roles[1]
        .extra
        .insert("description".into(), "Sees nothing".into());
    let out = round_trip(&m);
    assert_eq!(changed(&out), ["DBPROPERTIES", "Role", "TablePermission"]);
    let p = row_by_id(&out, "TablePermission", 21).unwrap();
    assert_eq!(int(p, "ModifiedTime"), Some(NOW));
    let new_role = out
        .rows("Role")
        .iter()
        .find(|r| get(r, "Name") == &MetaValue::from("Nobody"))
        .unwrap();
    assert_eq!(int(new_role, "ModelPermission"), Some(2));
    assert_eq!(int(new_role, "ModelID"), Some(1));

    // Removing the existing role removes its permissions and members.
    let mut m = build_model(&base()).unwrap();
    m.roles.clear();
    let out = round_trip(&m);
    assert!(out.rows("Role").is_empty());
    assert!(out.rows("TablePermission").is_empty());
    assert!(out.rows("RoleMembership").is_empty());
}

/// `relationship.set` and date tables (M9.2): relationship flags, a table's data category and a
/// column's `isKey` are metadata on their rows.
#[test]
fn relationship_flags_data_category_and_key_in_place() {
    let mut m = build_model(&base()).unwrap();
    let r = &mut m.relationships[0];
    r.is_active = !r.is_active;
    r.cross_filtering_behavior = Some("bothDirections".into());
    r.extra.insert(
        "securityFilteringBehavior".into(),
        serde_json::json!("bothDirections"),
    );
    let t = m.table_mut("Calendar").unwrap();
    t.extra.insert("dataCategory".into(), serde_json::json!("Time"));
    let c = t.columns.iter_mut().find(|c| !c.is_row_number()).unwrap();
    c.extra.insert("isKey".into(), serde_json::json!(true));
    let out = round_trip(&m);
    assert_eq!(changed(&out), ["Column", "Relationship", "Table"]);
}

#[test]
fn shared_expression_text() {
    let mut m = build_model(&base()).unwrap();
    m.expressions[0].expression = "1.5 meta [IsParameterQuery=true]".into();
    m.expressions[0].description = Some("Rate".into());
    let out = round_trip(&m);
    assert_eq!(changed(&out), ["Expression"]);
}

#[test]
fn unsupported_changes_are_listed() {
    let b = base();
    let mut m = build_model(&b).unwrap();
    let mut extra_table = m.tables[0].clone();
    extra_table.name = "New".into();
    m.tables.push(extra_table);
    let sales = m.table_mut("Sales").unwrap();
    sales.columns.retain(|c| c.name != "Day");
    sales.column_mut("Amount").unwrap().data_type = bicraft_model::DataType::Int64;
    sales.partitions[0].expression = Some("let x = 1 in x".into());
    sales.measures.push(Measure {
        name: "Kpi".into(),
        expression: "1".into(),
        extra: serde_json::from_str(r#"{"kpi": {"bogus": 1}}"#).unwrap(),
        ..Default::default()
    });
    let cal = m.table_mut("Calendar").unwrap();
    cal.column_mut("Month").unwrap().name = "Mon".into();
    cal.column_mut("Mon").unwrap().expression = Some("1".into());
    m.relationships[0].from_cardinality = Some("one".into());
    m.expressions[0].kind = "dax".into();
    m.culture = Some("fr-FR".into());
    let got = unsupported(apply_model_at(&b, &m, NOW));
    let expect = [
        "model: culture changed",
        // New measures go first (perspectives and translations may name them).
        "measure 'Sales'[Kpi]: property `bogus` of a KPI is not supported",
        "column 'Calendar'[Mon]: column added",
        "column 'Calendar'[Month]: column removed",
        "table 'Sales': partitions changed",
        "column 'Sales'[Amount]: dataType changed",
        "column 'Sales'[Day]: column removed",
        "table 'New': table added",
        "relationship 'rel-day': relationship ends or cardinality changed",
        "expression 'Rate': kind changed",
    ];
    assert_eq!(got, expect);
}

#[test]
fn hostile_input_is_an_error_not_a_panic() {
    // Duplicate IDs across tables.
    let mut b = base();
    let mut dup = b.rows("Measure")[0].clone();
    dup.insert("ID".into(), MetaValue::Integer(3));
    dup.insert("Name".into(), MetaValue::from("Dup"));
    b.push_row("Measure", dup);
    let mut m = build_model(&b).unwrap();
    m.description = Some("x".into());
    assert!(matches!(apply_model_at(&b, &m, NOW), Err(Error::Invalid(_))));

    // No model row.
    let mut b = base();
    b.insert_table("Model", Vec::new());
    assert!(matches!(
        apply_model_at(&b, &SemanticModel::default(), NOW),
        Err(Error::NoModel)
    ));

    // ID counter at the end of its range.
    let mut b = base();
    let mut big = b.rows("Expression")[0].clone();
    big.insert("ID".into(), MetaValue::Integer(i64::MAX));
    big.insert("Name".into(), MetaValue::from("Big"));
    b.push_row("Expression", big);
    let mut m = build_model(&b).unwrap();
    m.table_mut("Sales").unwrap().measures.push(Measure {
        name: "X".into(),
        expression: "1".into(),
        ..Default::default()
    });
    assert!(matches!(apply_model_at(&b, &m, NOW), Err(Error::Invalid(_))));

    // Rows of the wrong types everywhere: whatever happens, no panic.
    let mut b = MetadataTables::new();
    for table in bicraft_metadata::TABLES {
        let mut r = MetaRow::new();
        for col in [
            "ID",
            "ModelID",
            "TableID",
            "RoleID",
            "ObjectID",
            "ObjectType",
            "Name",
            "Value",
        ] {
            r.insert(col.into(), MetaValue::Blob(vec![1, 2]));
        }
        b.push_row(table, r.clone());
        r.insert("ID".into(), MetaValue::Text("7".into()));
        b.push_row(table, r);
    }
    if let Ok(mut m) = build_model(&b) {
        m.description = Some("x".into());
        m.roles.push(Role {
            name: "R".into(),
            ..Default::default()
        });
        let _ = apply_model_at(&b, &m, NOW);
    }
}

#[test]
fn rows_for_reproduces_the_stored_rows() {
    let db = bicraft_sqlite::Database::from_bytes(FIXTURE).unwrap();
    let b = base();
    let rows = tables_to_rows(&b);
    let mut checked = 0;
    for info in db.tables() {
        let original = db.rows_vec(&info.name).unwrap();
        let got = if info.columns.iter().any(|c| c.rowid_alias) {
            rows.rows_for(info).unwrap()
        } else {
            rows.rows_for_keyed(info, &original).unwrap()
        };
        if let Some(got) = got {
            assert_eq!(got, original, "{}", info.name);
            checked += 1;
        }
    }
    assert!(checked >= 10);
    // A value in a column the table does not declare is refused.
    let mut b = base();
    let mut r = b.rows("Role")[0].clone();
    r.insert("Bogus".into(), MetaValue::Integer(1));
    b.insert_table("Role", vec![r]);
    let info = db.table("Role").unwrap();
    assert!(matches!(
        tables_to_rows(&b).rows_for(info),
        Err(Error::Invalid(_))
    ));
}
