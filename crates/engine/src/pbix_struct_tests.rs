use super::*;
use crate::tempdir::TempDir;
use serde_json::{Value, json};

fn session_with_model() -> Session {
    let mut s = Session::new();
    s.execute("model.new", &json!({"name": "M"})).unwrap();
    s
}

#[test]
fn m_literal_round_trips() {
    let csv =
        "Id,Name,Price,When,Ok\n1,\"a \"\"b\"\"\",1.5,2026-01-02,true\n2,#(x),,2026-01-03T10:30:00,false\n";
    let data = read_csv(csv.as_bytes(), &CsvOptions::new()).unwrap();
    let m = m_literal(&data).unwrap();
    let back = bicraft_data::m::load_m(&m, &bicraft_data::Limits::default()).unwrap();
    assert_eq!(back, data);
    assert!(m.contains("#\"Id\" = Int64.Type"));
}

#[test]
fn import_csv_text_adds_a_literal_import_table() {
    let mut s = session_with_model();
    let out = s
        .execute(
            "table.import",
            &json!({"name": "Region", "csv": "Code;Label\n1;North\n2;South\n"}),
        )
        .unwrap();
    assert_eq!(out["rows"], 2);
    assert_eq!(out["replaced"], false);
    let t = s.model.as_ref().unwrap().table("Region").unwrap();
    assert_eq!(t.columns.len(), 2);
    assert_eq!(t.columns[0].data_type, DataType::Int64);
    assert_eq!(t.partitions[0].source_type, "m");
    let loaded = bicraft_data::load_partition(&t.partitions[0], &t.columns).unwrap();
    assert_eq!(loaded.row_count(), 2);
    // A second import of the same name needs replace.
    assert!(
        s.execute("table.import", &json!({"name": "Region", "csv": "Code\n1\n"}))
            .is_err()
    );
    let out = s
        .execute(
            "table.set_data",
            &json!({"name": "region", "csv": "Code,Label,Extra\n3,East,x\n"}),
        )
        .unwrap();
    assert_eq!(out["replaced"], true);
    let t = s.model.as_ref().unwrap().table("Region").unwrap();
    assert_eq!(t.columns.len(), 3);
}

#[test]
fn import_dax_makes_a_calculated_table() {
    let mut s = session_with_model();
    s.execute(
        "table.import",
        &json!({"name": "T", "dax": "DATATABLE(\"K\", INTEGER, \"V\", STRING, {{1, \"x\"}, {2, \"y\"}})"}),
    )
    .unwrap();
    let t = s.model.as_ref().unwrap().table("T").unwrap();
    assert_eq!(t.partitions[0].source_type, "calculated");
    assert_eq!(t.columns[0].source_column.as_deref(), Some("[K]"));
}

#[test]
fn import_rejects_bad_parameters() {
    let mut s = session_with_model();
    assert!(s.execute("table.import", &json!({"name": "T"})).is_err());
    assert!(
        s.execute("table.import", &json!({"name": "T", "csv": "a\n1\n", "m": "x"}))
            .is_err()
    );
    assert!(
        s.execute("table.set_data", &json!({"name": "Nope", "csv": "a\n1\n"}))
            .is_err()
    );
    assert!(
        s.execute(
            "table.import",
            &json!({"name": "T", "csv": "a\n1\n", "types": {"a": "blob"}})
        )
        .is_err()
    );
}

#[test]
fn coerce_and_infer() {
    let (v, t) = coerce(vec![CellValue::Real(1.0), CellValue::Null], DataType::Unknown).unwrap();
    assert_eq!(t, DataType::Int64);
    assert_eq!(v, vec![CellValue::Int(1), CellValue::Null]);
    assert!(coerce(vec![CellValue::Real(1.5)], DataType::Int64).is_err());
    assert!(
        coerce(
            vec![CellValue::Int(1), CellValue::Text("a".into())],
            DataType::Unknown
        )
        .is_err()
    );
    let (v, _) = coerce(vec![CellValue::Real(2.5)], DataType::Decimal).unwrap();
    assert_eq!(v, vec![CellValue::Decimal(25_000)]);
}

#[test]
fn civil_dates() {
    assert_eq!(civil(45_658), (2025, 1, 1));
    assert_eq!(civil(2), (1900, 1, 1));
    assert_eq!(
        m_value(&CellValue::DateTime(45_658.5), false).unwrap(),
        "#datetime(2025, 1, 1, 12, 0, 0)"
    );
}

// --- saves into a PBIX ----------------------------------------------------------------------

const SYNTHETIC_METADATA: &[u8] = include_bytes!("../../metadata/tests/fixtures/synthetic_metadata.sqlitedb");

/// A fresh folder per call, removed when the guard drops (see `testsupport/tempdir.rs`).
fn temp(name: &str) -> TempDir {
    TempDir::new(&format!("engine-struct-{name}"))
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn open(path: &Path) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": path }));
    assert_eq!(s.model_error, None);
    s
}

fn stored(path: &Path, table: &str, column: &str) -> Vec<CellValue> {
    let pkg = Package::open(path).unwrap();
    let part = &pkg.get(bicraft_pbix::DATA_MODEL).unwrap().data;
    DataModelEditor::open(part, &Default::default())
        .unwrap()
        .column_values(table, column)
        .unwrap()
}

#[test]
fn imported_table_is_saved_into_a_pbix_with_its_data() {
    let dir = temp("import");
    let path = dir.join("Shop.pbix");
    let pkg = bicraft_pbix::synthetic::pbix(&bicraft_pbix::synthetic::storage_database().unwrap()).unwrap();
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    let mut s = open(&path);
    run(
        &mut s,
        "table.import",
        json!({"name": "Targets", "csv": "Code,Label,Goal\n1,North,10.5\n2,South,20\n1,North,1\n"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Targets", "name": "Total goal", "expression": "SUM(Targets[Goal])"}),
    );
    let out = run(&mut s, "file.save", json!({"backup": false}));
    assert_eq!(out["model_saved"], true);
    assert!(!s.dirty);
    let s2 = open(&path);
    let m = s2.model.as_ref().unwrap();
    assert!(m.table("Targets").is_some());
    assert!(m.measure("Total goal").is_some());
    assert_eq!(
        stored(&path, "Targets", "Label"),
        vec![
            CellValue::Text("North".into()),
            CellValue::Text("South".into()),
            CellValue::Text("North".into())
        ]
    );
    assert_eq!(
        stored(&path, "Targets", "Goal"),
        vec![CellValue::Real(10.5), CellValue::Real(20.0), CellValue::Real(1.0)]
    );

    // New data for the table: replaced in place.
    let mut s3 = open(&path);
    run(
        &mut s3,
        "table.set_data",
        json!({"name": "Targets", "csv": "Code,Label,Goal\n7,East,3\n"}),
    );
    run(&mut s3, "file.save", json!({"backup": false}));
    assert_eq!(stored(&path, "Targets", "Code"), vec![CellValue::Int(7)]);
    assert!(
        open(&path)
            .model
            .as_ref()
            .unwrap()
            .measure("Total goal")
            .is_some()
    );

    // Removing the table writes a model without it.
    let mut s4 = open(&path);
    run(&mut s4, "table.remove", json!({"name": "Targets"}));
    run(&mut s4, "file.save", json!({"backup": false}));
    assert!(open(&path).model.as_ref().unwrap().table("Targets").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Calculated tables and columns evaluated by bicraft's DAX evaluator, saved with their values.
#[test]
fn calculated_objects_are_evaluated_and_saved() {
    let dir = temp("calc");
    let path = dir.join("Calc.pbix");
    let pkg = bicraft_pbix::synthetic::pbix(&bicraft_pbix::synthetic::storage_database().unwrap()).unwrap();
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    let mut s = open(&path);
    run(
        &mut s,
        "table.import",
        json!({"name": "Dim", "csv": "K,Name\n1,a\n2,b\n3,c\n"}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Dim", "name": "K2", "data_type": "int64", "expression": "Dim[K] * 2"}),
    );
    run(
        &mut s,
        "table.add",
        json!({"name": "Big", "partition": {"source_type": "calculated",
               "expression": "FILTER(Dim, Dim[K2] >= 4)"}}),
    );
    // A calculated column of the calculated table, reading a measure through context transition.
    run(
        &mut s,
        "measure.add",
        json!({"table": "Dim", "name": "Total K", "expression": "SUM(Dim[K])"}),
    );
    run(&mut s, "file.save", json!({"backup": false}));
    let mut s2 = open(&path);
    let big = s2.model.as_ref().unwrap().table("Big").unwrap().clone();
    let names: Vec<&str> = big
        .columns
        .iter()
        .filter(|c| !is_row_number(c))
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(names, ["K", "Name", "K2"]);
    assert_eq!(
        stored(&path, "Dim", "K2"),
        vec![CellValue::Int(2), CellValue::Int(4), CellValue::Int(6)]
    );
    assert_eq!(
        stored(&path, "Big", "K"),
        vec![CellValue::Int(2), CellValue::Int(3)]
    );
    // Now a calculated column on the stored calculated table.
    run(
        &mut s2,
        "column.add",
        json!({"table": "Big", "name": "Share", "data_type": "int64",
               "expression": "Big[K] + [Total K]"}),
    );
    run(&mut s2, "file.save", json!({"backup": false}));
    // [Total K] in the row context of Big: context transition filters Dim? No relationship, so
    // the measure sees every Dim row: 1 + 2 + 3.
    assert_eq!(
        stored(&path, "Big", "Share"),
        vec![CellValue::Int(8), CellValue::Int(9)]
    );
    // Table functions: a series, columns added to the rows of a model table.
    let mut s3 = open(&path);
    run(
        &mut s3,
        "table.add",
        json!({"name": "Series", "partition": {"source_type": "calculated",
               "expression": "GENERATESERIES(1, 3)"}}),
    );
    run(
        &mut s3,
        "table.add",
        json!({"name": "Dim Plus", "partition": {"source_type": "calculated",
               "expression": "ADDCOLUMNS(Dim, \"Total\", [Total K], \"Twice\", Dim[K] * 2)"}}),
    );
    run(&mut s3, "file.save", json!({"backup": false}));
    assert_eq!(
        stored(&path, "Series", "Value"),
        vec![CellValue::Int(1), CellValue::Int(2), CellValue::Int(3)]
    );
    // [Total K] with context transition: the row's own K.
    assert_eq!(
        stored(&path, "Dim Plus", "Total"),
        vec![CellValue::Int(1), CellValue::Int(2), CellValue::Int(3)]
    );
    assert_eq!(
        stored(&path, "Dim Plus", "Twice"),
        vec![CellValue::Int(2), CellValue::Int(4), CellValue::Int(6)]
    );
    // Unsupported expressions are refused with the evaluator's reason.
    let mut s3 = open(&path);
    run(
        &mut s3,
        "table.add",
        json!({"name": "Odd", "partition": {"source_type": "calculated",
               "expression": "ADDCOLUMNS(Dim, \"R\", RAND())"}}),
    );
    let err = s3.execute("file.save", &json!({})).unwrap_err();
    assert!(err.contains("Odd"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn data_column_added_to_an_existing_table_is_refused_with_the_reason() {
    let dir = temp("refuse");
    let path = dir.join("Shop.pbix");
    let bytes = bicraft_pbix::synthetic::pbix(SYNTHETIC_METADATA)
        .unwrap()
        .to_bytes()
        .unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let mut s = open(&path);
    let table = s.model.as_ref().unwrap().tables[0].name.clone();
    run(
        &mut s,
        "column.add",
        json!({"table": table, "name": "Extra", "data_type": "string"}),
    );
    let err = s.execute("file.save", &json!({})).unwrap_err();
    assert!(err.contains("table.set_data"), "{err}");
    assert!(err.contains("nothing was written"), "{err}");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A field parameter as Desktop builds it (a calculated table of `NAMEOF` references, the
/// display column grouped by the fields column, `ParameterMetadata`) is re-evaluated when a
/// measure and a column it names are renamed: the save succeeds, stores the new names spelled
/// as the engine spells them, and keeps the parameter's metadata.
#[test]
fn field_parameter_follows_renames_into_a_pbix() {
    let dir = temp("field-param");
    let path = dir.join("Fields.pbix");
    let pkg = bicraft_pbix::synthetic::pbix(&bicraft_pbix::synthetic::storage_database().unwrap()).unwrap();
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    let mut s = open(&path);
    run(
        &mut s,
        "table.import",
        json!({"name": "Dim", "csv": "K,Name
1,a
2,b
"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Dim", "name": "Total", "expression": "SUM(Dim[K])"}),
    );
    let column = |name: &str, source: &str, data_type: &str, extra: Value| {
        let mut c = json!({"type": "calculatedTableColumn", "name": name, "dataType": data_type,
            "isNameInferred": false, "isDataTypeInferred": true, "sourceColumn": source});
        if let (Some(c), Some(extra)) = (c.as_object_mut(), extra.as_object()) {
            c.extend(extra.clone());
        }
        c
    };
    let tmsl = json!({"name": "X", "compatibilityLevel": 1567, "model": {"tables": [{
        "name": "Fields",
        "columns": [
            column("Fields", "[Value1]", "string",
                json!({"relatedColumnDetails": {"groupByColumns": [{"groupingColumn": "Fields Fields"}]}})),
            column("Fields Fields", "[Value2]", "string", json!({"isHidden": true,
                "sortByColumn": "Fields Order", "extendedProperties": [{"type": "json",
                "name": "ParameterMetadata", "value": {"version": 3, "kind": 2}}]})),
            column("Fields Order", "[Value3]", "int64", json!({"isHidden": true})),
        ],
        "partitions": [{"name": "Fields", "source": {"type": "calculated",
            "expression": "{(\"Total\", NAMEOF('Dim'[Total]), 0), (\"Key\", NAMEOF('Dim'[K]), 1)}"}}]
    }]}});
    let fields = bicraft_tmsl::read(&tmsl.to_string()).unwrap().tables.remove(0);
    s.model.as_mut().unwrap().tables.push(fields);
    s.dirty = true;
    run(&mut s, "file.save", json!({"backup": false}));
    let text = |v: &[&str]| -> Vec<CellValue> { v.iter().map(|x| CellValue::Text((*x).into())).collect() };
    assert_eq!(
        stored(&path, "Fields", "Fields Fields"),
        text(&["'Dim'[Total]", "'Dim'[K]"])
    );
    let mut s2 = open(&path);
    run(
        &mut s2,
        "measure.rename",
        json!({"name": "Total", "new_name": "Rev]enue"}),
    );
    run(
        &mut s2,
        "column.rename",
        json!({"table": "Dim", "name": "K", "new_name": "Key"}),
    );
    run(&mut s2, "file.save", json!({"backup": false}));
    assert_eq!(
        stored(&path, "Fields", "Fields Fields"),
        text(&["'Dim'[Rev]]enue]", "'Dim'[Key]"])
    );
    assert_eq!(stored(&path, "Fields", "Fields"), text(&["Total", "Key"]));
    let back = open(&path);
    let fields = back.model.as_ref().unwrap().table("Fields").unwrap();
    assert_eq!(
        fields.partitions[0].expression.as_deref(),
        Some("{(\"Total\", NAMEOF('Dim'[Rev]]enue]), 0), (\"Key\", NAMEOF('Dim'[Key]), 1)}")
    );
    let display = fields.columns.iter().find(|c| c.name == "Fields").unwrap();
    assert_eq!(
        display.extra["relatedColumnDetails"]["groupByColumns"][0]["groupingColumn"],
        "Fields Fields"
    );
    let field_col = fields.columns.iter().find(|c| c.name == "Fields Fields").unwrap();
    assert_eq!(field_col.sort_by_column.as_deref(), Some("Fields Order"));
    assert_eq!(
        field_col.extra["extendedProperties"][0]["name"],
        "ParameterMetadata"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Calculated-table columns named after their source column follow its rename into a PBIX, as
/// the engine does (`crate::rename_inferred`); an unnamed `SELECTCOLUMNS` column is `Table_Column`.
#[test]
fn inferred_columns_follow_a_rename_into_a_pbix() {
    let dir = temp("inferred");
    let path = dir.join("Inferred.pbix");
    let pkg = bicraft_pbix::synthetic::pbix(&bicraft_pbix::synthetic::storage_database().unwrap()).unwrap();
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    let mut s = open(&path);
    run(
        &mut s,
        "table.import",
        json!({"name": "Dim", "csv": "K,Name
1,a
2,b
"}),
    );
    for (name, expression) in [("C", "Dim"), ("S", "SELECTCOLUMNS(Dim, Dim[K])")] {
        run(
            &mut s,
            "table.add",
            json!({"name": name, "partition": {"source_type": "calculated", "expression": expression}}),
        );
    }
    run(&mut s, "file.save", json!({"backup": false}));
    let names = |s: &Session, t: &str| -> Vec<String> {
        s.model
            .as_ref()
            .unwrap()
            .table(t)
            .unwrap()
            .columns
            .iter()
            .filter(|c| !is_row_number(c))
            .map(|c| c.name.clone())
            .collect()
    };
    let mut s2 = open(&path);
    assert_eq!(names(&s2, "S"), ["Dim_K"]);
    run(
        &mut s2,
        "table.add",
        json!({"name": "F", "partition": {"source_type": "calculated", "expression": "FILTER(C, C[K] > 1)"}}),
    );
    run(
        &mut s2,
        "measure.add",
        json!({"table": "C", "name": "Total", "expression": "SUM(C[K])"}),
    );
    run(&mut s2, "file.save", json!({"backup": false}));
    assert_eq!(names(&open(&path), "F"), ["K", "Name"]);
    let mut s3 = open(&path);
    let v = run(
        &mut s3,
        "column.rename",
        json!({"table": "Dim", "name": "K", "new_name": "Key"}),
    );
    assert_eq!(v["inferred_renamed"].as_array().map(Vec::len), Some(3), "{v}");
    run(&mut s3, "file.save", json!({"backup": false}));
    let back = open(&path);
    assert_eq!(names(&back, "C"), ["Key", "Name"]);
    assert_eq!(names(&back, "S"), ["Dim_Key"]);
    assert_eq!(names(&back, "F"), ["Key", "Name"]);
    let m = back.model.as_ref().unwrap();
    assert_eq!(m.measure("Total").unwrap().1.expression, "SUM(C[Key])");
    assert_eq!(
        m.table("C").unwrap().column("Key").unwrap().extra["isNameInferred"],
        true
    );
    assert_eq!(stored(&path, "F", "Key"), vec![CellValue::Int(2)]);
    // Renamed by hand: the name becomes explicit, and the column inferred from it follows.
    let mut s4 = open(&path);
    run(
        &mut s4,
        "column.rename",
        json!({"table": "C", "name": "Name", "new_name": "Label"}),
    );
    run(&mut s4, "file.save", json!({"backup": false}));
    let back = open(&path);
    let m = back.model.as_ref().unwrap();
    assert!(
        m.table("C")
            .unwrap()
            .column("Label")
            .unwrap()
            .extra
            .get("isNameInferred")
            .is_none()
    );
    let f = m.table("F").unwrap().column("Label").unwrap();
    assert_eq!(f.extra["isNameInferred"], true);
    assert_eq!(f.source_column.as_deref(), Some("C[Label]"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The full flow on the local Desktop sample (skipped without `BICRAFT_TESTDATA`): an imported
/// table related to an existing one, a calculated table and a calculated column evaluated by
/// bicraft, a rename; everything reads back with the expected values.
#[test]
fn real_pbix_structural_edits_through_commands() {
    let Some(dir) = bicraft_env::var_os("BICRAFT_TESTDATA").map(std::path::PathBuf::from) else {
        eprintln!("note: BICRAFT_TESTDATA is not set; skipping");
        return;
    };
    let sample = dir.join("sample1/report.pbix");
    if !sample.exists() {
        eprintln!("note: sample1/report.pbix not found; skipping");
        return;
    }
    let work = temp("real");
    let path = work.join("edited.pbix");
    std::fs::copy(&sample, &path).unwrap();
    let mut s = open(&path);
    let m = s.model.clone().unwrap();
    // An int64 "one" end of an existing relationship.
    let (dim, key) = m
        .relationships
        .iter()
        .find_map(|r| {
            let c = m
                .table(&r.to_table)?
                .columns
                .iter()
                .find(|c| c.name == r.to_column)?;
            (c.data_type == DataType::Int64).then(|| (r.to_table.clone(), r.to_column.clone()))
        })
        .expect("an int64 key");
    let keys: Vec<i64> = stored(&path, &dim, &key)
        .into_iter()
        .filter_map(|v| match v {
            CellValue::Int(n) => Some(n),
            _ => None,
        })
        .take(3)
        .collect();
    let csv: String = std::iter::once("K,V\n".to_string())
        .chain(keys.iter().enumerate().map(|(i, k)| format!("{k},{}\n", i + 1)))
        .collect();
    run(&mut s, "table.import", json!({"name": "bicraft T", "csv": csv}));
    run(
        &mut s,
        "relationship.add",
        json!({"from": "'bicraft T'[K]", "to": format!("'{dim}'[{key}]")}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": dim, "name": "bicraft x10", "data_type": "int64",
               "expression": format!("'{dim}'[{key}] * 10")}),
    );
    run(
        &mut s,
        "table.add",
        json!({"name": "bicraft C", "partition": {"source_type": "calculated",
               "expression": format!("FILTER('{dim}', '{dim}'[{key}] <= {})", keys[0])}}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "bicraft T", "name": "W", "data_type": "int64",
               "expression": "'bicraft T'[V] + 100"}),
    );
    run(&mut s, "file.save", json!({"backup": false}));
    let s2 = open(&path);
    let m2 = s2.model.as_ref().unwrap();
    assert!(m2.relationships.iter().any(|r| r.from_table == "bicraft T"));
    let c = m2.table("bicraft C").unwrap();
    assert!(c.columns.iter().any(|x| x.name == key));
    let dim_keys = stored(&path, &dim, &key);
    let x10 = stored(&path, &dim, "bicraft x10");
    for (k, x) in dim_keys.iter().zip(&x10) {
        match (k, x) {
            (CellValue::Int(k), CellValue::Int(x)) => assert_eq!(*x, k * 10),
            (CellValue::Null, CellValue::Null) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    let w = stored(&path, "bicraft T", "W");
    assert_eq!(w.first(), Some(&CellValue::Int(101)));
    let _ = std::fs::remove_dir_all(&work);
}

/// A column without an attribute hierarchy reads back from a PBIX as TOM spells the flag
/// (`isAvailableInMdx`) and leaves for a PBIT or a project with that spelling only. The engine's
/// spelling (`isAvailableInMDX`), written by older versions, is still accepted on input.
#[test]
fn available_in_mdx_uses_the_tom_spelling() {
    for key in ["isAvailableInMdx", "isAvailableInMDX"] {
        let dir = temp("mdx");
        let path = dir.join("Shop.pbix");
        let pkg =
            bicraft_pbix::synthetic::pbix(&bicraft_pbix::synthetic::storage_database().unwrap()).unwrap();
        std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
        let mut s = open(&path);
        run(
            &mut s,
            "table.import",
            json!({"name": "Targets", "csv": "Code,Label\n1,North\n2,South\n"}),
        );
        let label = s
            .model
            .as_mut()
            .unwrap()
            .tables
            .iter_mut()
            .find(|t| t.name == "Targets")
            .unwrap()
            .columns
            .iter_mut()
            .find(|c| c.name == "Label")
            .unwrap();
        label.extra.insert(key.into(), json!(false));
        run(&mut s, "file.save", json!({"backup": false}));
        assert_eq!(
            stored(&path, "Targets", "Label"),
            vec![CellValue::Text("North".into()), CellValue::Text("South".into())]
        );
        let mut s2 = open(&path);
        let targets = s2.model.as_ref().unwrap().table("Targets").unwrap();
        let extra = &targets.columns.iter().find(|c| c.name == "Label").unwrap().extra;
        assert_eq!(extra.get("isAvailableInMdx"), Some(&json!(false)), "{key}");
        assert!(!extra.contains_key("isAvailableInMDX"), "{key}");

        // PBIX → PBIT: DataModelSchema (UTF-16LE TMSL).
        run(&mut s2, "file.save_as", json!({"path": dir.join("Shop.pbit")}));
        let pbit = Package::open(&dir.join("Shop.pbit")).unwrap();
        let schema = &pbit.get("DataModelSchema").unwrap().data;
        let units: Vec<u16> = schema
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes(*b))
            .collect();
        let schema = String::from_utf16_lossy(&units);
        assert!(schema.contains("\"isAvailableInMdx\": false"), "{key}");
        assert!(!schema.contains("isAvailableInMDX"), "{key}");

        // PBIX → PBIP, TMDL and TMSL.
        let mut s3 = open(&path);
        run(
            &mut s3,
            "file.save_as",
            json!({"path": dir.join("tmdl").join("Shop.pbip")}),
        );
        let tmdl =
            std::fs::read_to_string(dir.join("tmdl/Shop.SemanticModel/definition/tables/Targets.tmdl"))
                .unwrap();
        assert!(tmdl.contains("isAvailableInMdx: false"), "{key}: {tmdl}");
        assert!(!tmdl.contains("isAvailableInMDX"), "{key}");
        let mut s4 = open(&path);
        run(
            &mut s4,
            "file.save_as",
            json!({"path": dir.join("tmsl").join("Shop.pbip"), "format": "pbip-tmsl"}),
        );
        let bim = std::fs::read_to_string(dir.join("tmsl/Shop.SemanticModel/model.bim")).unwrap();
        assert!(bim.contains("\"isAvailableInMdx\": false"), "{key}");
        assert!(!bim.contains("isAvailableInMDX"), "{key}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// `table.import {dax}` beyond literal tables: the DAX evaluator over the model's data (G21).
#[test]
fn import_dax_over_model_data() {
    let mut s = session_with_model();
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "csv": "Region,Amount\nNorth,10\nSouth,5\nNorth,7\n"}),
    );
    let out = run(
        &mut s,
        "table.import",
        json!({"name": "By region", "dax": "SUMMARIZECOLUMNS(Sales[Region], \"Total\", SUM(Sales[Amount]))"}),
    );
    assert_eq!(out["rows"], 2);
    assert_eq!(out["source_type"], "calculated");
    assert_eq!(
        out["columns"],
        json!([{"name": "Region", "data_type": "string"}, {"name": "Total", "data_type": "int64"}])
    );
    let out = run(
        &mut s,
        "table.import",
        json!({"name": "Big", "dax": "TOPN(1, ADDCOLUMNS(VALUES(Sales[Region]), \"N\", CALCULATE(COUNTROWS(Sales))), [N], DESC)"}),
    );
    assert_eq!(out["rows"], 1);
    // What the evaluator does not support is refused with its reason.
    let err = s
        .execute(
            "table.import",
            &json!({"name": "Odd", "dax": "ADDCOLUMNS(VALUES(Sales[Region]), \"R\", RAND())"}),
        )
        .unwrap_err();
    assert!(err.starts_with("dax: "), "{err}");
    assert!(s.model.as_ref().unwrap().table("Odd").is_none());
}

/// Connectors are never run: the error names the external source (G21).
#[test]
fn import_m_external_source_is_not_evaluated() {
    let mut s = session_with_model();
    let err = s
        .execute(
            "table.import",
            &json!({"name": "Db", "m": "let S = Sql.Database(\"srv\", \"db\"), T = S{[Schema=\"dbo\", Item=\"t\"]}[Data] in T"}),
        )
        .unwrap_err();
    assert!(err.contains("external source, not evaluated"), "{err}");
    assert!(err.contains("Sql.Database"), "{err}");
    // A PBIX save of a model holding such a table (added as metadata) is refused with the reason;
    // the query is kept as written.
    let dir = temp("external");
    let path = dir.join("Ext.pbix");
    let pkg = bicraft_pbix::synthetic::pbix(&bicraft_pbix::synthetic::storage_database().unwrap()).unwrap();
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    let mut s = open(&path);
    let query = "let S = Web.Contents(\"https://example.com/a.csv\"), C = Csv.Document(S) in C";
    run(
        &mut s,
        "table.add",
        json!({"name": "Web", "partition": {"source_type": "m", "expression": query}}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Web", "name": "Column1", "data_type": "string", "source_column": "Column1"}),
    );
    let err = s.execute("file.save", &json!({"backup": false})).unwrap_err();
    assert!(err.contains("external source, not evaluated"), "{err}");
    let t = s.model.as_ref().unwrap().table("Web").unwrap();
    assert_eq!(t.partitions[0].expression.as_deref(), Some(query));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `Folder.Files` under the data root: Desktop's "Combine files" shape (G21).
#[test]
fn import_m_folder_files_under_the_data_root() {
    let dir = temp("folder");
    let data = dir.join("data");
    std::fs::create_dir_all(data.join("2026")).unwrap();
    std::fs::write(data.join("jan.csv"), "Day,Qty\n1,4\n2,5\n").unwrap();
    std::fs::write(data.join("2026").join("feb.csv"), "Day,Qty\n3,6\n").unwrap();
    std::fs::write(data.join("notes.txt"), "not data").unwrap();
    let mut s = session_with_model();
    let m = format!(
        "let Source = Folder.Files(\"{}\"),
            Csv = Table.SelectRows(Source, each [Extension] = \".csv\"),
            Tables = Table.AddColumn(Csv, \"T\", each Table.PromoteHeaders(Csv.Document([Content]))),
            Combined = Table.Combine(Tables[T]),
            Typed = Table.TransformColumnTypes(Combined, {{{{\"Day\", Int64.Type}}, {{\"Qty\", Int64.Type}}}})
        in Typed",
        data.display().to_string().replace('"', "\"\"")
    );
    // Without a data root, file access is refused with the hint.
    let err = s
        .execute("table.import", &json!({"name": "Qty", "m": m}))
        .unwrap_err();
    assert!(err.contains("data_root"), "{err}");
    let out = run(
        &mut s,
        "table.import",
        json!({"name": "Qty", "m": m, "data_root": data.display().to_string()}),
    );
    assert_eq!(out["rows"], 3);
    let _ = std::fs::remove_dir_all(&dir);
}

/// One stored value as the oracle records it: numbers to 6 decimals, dates as their serial.
fn oracle_text(v: &Value) -> String {
    match v {
        Value::Null => "∅".into(),
        Value::Number(n) => format!("{:.6}", n.as_f64().unwrap_or(f64::NAN)),
        Value::Object(o) => oracle_text(o.get("date").unwrap_or(&Value::Null)),
        Value::String(s) => format!("{s:?}"),
        other => other.to_string(),
    }
}

fn cell_text(c: &CellValue) -> String {
    oracle_text(&match c {
        CellValue::Null => Value::Null,
        CellValue::Int(n) => json!(*n),
        CellValue::Real(x) | CellValue::DateTime(x) => json!(*x),
        CellValue::Decimal(n) => json!(*n as f64 / 10_000.0),
        CellValue::Text(s) => json!(s),
        CellValue::Bool(b) => json!(b),
    })
}

/// Calculated tables as a structural PBIX save materializes them (the DAX evaluator, then the
/// columns a table without declared columns gets: name, data type, source column; the stored
/// values), compared with what the Analysis Services engine of Power BI Desktop 2.158 created
/// for the same calculated tables on the same model (`tests/calc_tables/expected.json`,
/// recorded with the maintainers' development oracles). Expressions the engine refuses must be
/// refused too.
#[test]
fn calculated_tables_match_the_engine() {
    let doc: Value = serde_json::from_str(include_str!("../tests/calc_tables/expected.json")).unwrap();
    let mut base = calc_base();
    if doc["events_loaded"] != json!(true) {
        // The local engine instance could not run the M table: it stayed empty.
        base.tables.retain(|t| t.name != "Events");
    }
    let mut failures = Vec::new();
    for q in doc["probes"].as_array().unwrap() {
        let id = q["id"].as_str().unwrap();
        let expr = q["expression"].as_str().unwrap();
        let mut m = base.clone();
        m.tables.push(Table {
            name: "P".into(),
            partitions: vec![Partition {
                name: "P".into(),
                source_type: "calculated".into(),
                expression: Some(expr.into()),
                ..Default::default()
            }],
            ..Default::default()
        });
        let mut ev = StructEval::new(None, m.clone(), MAX_ROWS);
        let got = ev
            .table(expr)
            .and_then(|r| calculated_table(&mut m, "P", r.columns, &r.sources));
        if q.get("error").is_some() {
            if got.is_ok() {
                failures.push(format!("{id}: the engine refuses it, bicraft evaluates it"));
            }
            continue;
        }
        let (rows, cols) = match got {
            Ok(x) => x,
            Err(e) => {
                failures.push(format!("{id}: {e}"));
                continue;
            }
        };
        let cols = stored_cols(&m, false, cols);
        let t = find_table(&m, "P").unwrap();
        let ours: Vec<String> = t
            .columns
            .iter()
            .map(|c| {
                format!(
                    "{}:{}:{}",
                    c.name,
                    c.data_type.as_tmsl().unwrap_or("?"),
                    c.source_column.as_deref().unwrap_or("")
                )
            })
            .collect();
        let theirs: Vec<String> = q["columns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                format!(
                    "{}:{}:{}",
                    c["name"].as_str().unwrap(),
                    c["dataType"].as_str().unwrap(),
                    c["sourceColumn"].as_str().unwrap()
                )
            })
            .collect();
        let (mut a, mut b) = (ours.clone(), theirs.clone());
        a.sort();
        b.sort();
        if a != b {
            failures.push(format!(
                "{id}: columns\n  bicraft {ours:?}\n  engine   {theirs:?}"
            ));
            continue;
        }
        // Rows, as multisets, in the engine's column order.
        let order: Vec<String> = q["rowColumns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap().to_string())
            .collect();
        let mut got_rows: Vec<String> = (0..rows)
            .map(|r| {
                order
                    .iter()
                    .map(|n| {
                        cols.iter()
                            .find(|(c, _)| c.eq_ignore_ascii_case(n))
                            .map_or("<missing>".into(), |(_, v)| cell_text(&v[r]))
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .collect();
        let mut want_rows: Vec<String> = q["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                r.as_array()
                    .unwrap()
                    .iter()
                    .map(oracle_text)
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .collect();
        got_rows.sort();
        want_rows.sort();
        if got_rows != want_rows {
            let diff: Vec<String> = got_rows
                .iter()
                .zip(&want_rows)
                .filter(|(a, b)| a != b)
                .take(3)
                .map(|(a, b)| format!("\n  bicraft {a}\n  engine   {b}"))
                .collect();
            failures.push(format!(
                "{id}: rows ({} vs {}) {}",
                got_rows.len(),
                want_rows.len(),
                diff.join("")
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The import (M) table the calculated-table oracle adds (see docs/dax-oracle.md).
const EVENTS_M: &str =
    "#table(type table [When = date, N = Int64.Type], {{#date(2022, 3, 4), 1}, {#date(2025, 11, 30), 2}})";

/// `tests/table_functions/model.bim` plus the `Events` import table.
fn calc_base() -> SemanticModel {
    let mut m = bicraft_tmsl::read(include_str!("../tests/table_functions/model.bim")).unwrap();
    let col = |name: &str, data_type| Column {
        name: name.into(),
        data_type,
        source_column: Some(name.into()),
        ..Default::default()
    };
    m.tables.push(Table {
        name: "Events".into(),
        columns: vec![col("When", DataType::DateTime), col("N", DataType::Int64)],
        partitions: vec![Partition {
            name: "Events".into(),
            source_type: "m".into(),
            expression: Some(EVENTS_M.into()),
            ..Default::default()
        }],
        ..Default::default()
    });
    m
}

/// `CALENDARAUTO` over the dates of an import (M) table, as Power BI Desktop 2.158 computed the
/// same calculated tables on the same model (Desktop round 4 in docs/formats/data-sources.md).
#[test]
fn calendarauto_over_an_import_table_matches_desktop() {
    for (expr, rows, first, last) in [
        ("CALENDARAUTO()", 1461, 44_562.0, 46_022.0),
        ("CALENDARAUTO(6)", 1826, 44_378.0, 46_203.0),
    ] {
        let mut ev = StructEval::new(None, calc_base(), MAX_ROWS);
        let t = ev.table(expr).unwrap();
        let [(name, _, values)] = t.columns.as_slice() else {
            panic!("{expr}: one column expected")
        };
        assert_eq!(name, "Date");
        assert_eq!(values.len(), rows, "{expr}");
        assert_eq!(values.first(), Some(&CellValue::DateTime(first)), "{expr}");
        assert_eq!(values.last(), Some(&CellValue::DateTime(last)), "{expr}");
    }
}
