//! `model.validate`: type and context rules, circular dependencies, every kind of DAX object.

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn open(name: &str, bim: &str) -> (Session, TempDir) {
    let dir = TempDir::new(&format!("validate-{name}"));
    let path = dir.join("model.bim");
    std::fs::write(&path, bim).unwrap();
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path})).unwrap();
    (s, dir)
}

fn entry<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["objects_with_diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["name"] == name)
        .unwrap_or_else(|| panic!("no entry for {name}: {report:#}"))
}

fn codes(o: &Value) -> Vec<String> {
    o["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn reports_cycles_and_keeps_the_report_shape() {
    let (mut s, dir) = open(
        "cycles",
        r#"{"name":"M","model":{"tables":[{"name":"Sales",
            "columns":[{"name":"Amount","dataType":"double"},
                       {"name":"Loop","dataType":"double","type":"calculated","expression":"[Back]"}],
            "measures":[{"name":"Total","expression":"SUM(Sales[Amount])"},
                        {"name":"Self","expression":"[Self] + 1"},
                        {"name":"Back","expression":"SUM(Sales[Loop])"}]},
            {"name":"Top","partitions":[{"name":"Top","source":{"type":"calculated","expression":"TOPN(5, Sales, [Total])"}}]}],
          "roles":[{"name":"Reader","modelPermission":"read",
                    "tablePermissions":[{"name":"Sales","filterExpression":"Sales[Amount] > 0"}]}]}}"#,
    );
    let report = s.execute("model.validate", &json!({})).unwrap();
    // Backward-compatible fields.
    assert_eq!(report["valid"], false);
    assert_eq!(report["invalid_count"], 3, "{report:#}");
    // New fields: measures, calculated columns, the calculated table and the row filter.
    assert_eq!(report["checked_count"], 6);
    let cycles = report["cycles"].as_array().unwrap();
    assert_eq!(cycles.len(), 2, "{report:#}");
    assert_eq!(
        cycles[0][0],
        json!({"kind": "column", "table": "Sales", "name": "Loop"})
    );

    let self_ref = entry(&report, "Self");
    assert_eq!(self_ref["kind"], "measure");
    assert_eq!(self_ref["valid"], false);
    assert_eq!(codes(self_ref), ["circular-dependency"]);
    assert_eq!(self_ref["result_type"]["kind"], "scalar");
    assert_eq!(codes(entry(&report, "Loop")), ["circular-dependency"]);
    assert_eq!(codes(entry(&report, "Back")), ["circular-dependency"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn applies_context_rules_by_object_kind() {
    let (mut s, dir) = open(
        "context",
        r#"{"name":"M","model":{"tables":[{"name":"Sales",
            "columns":[{"name":"Amount","dataType":"double"},
                       {"name":"Double","dataType":"double","type":"calculated","expression":"Sales[Amount] * 2"}],
            "measures":[{"name":"Bare","expression":"Sales[Amount] * 2"},
                        {"name":"Fine","expression":"SUM(Sales[Amount])"}]},
            {"name":"Copy","partitions":[{"name":"Copy","source":{"type":"calculated","expression":"1 + 1"}}]}],
          "roles":[{"name":"Reader","modelPermission":"read",
                    "tablePermissions":[{"name":"Sales","filterExpression":"Sales[Amount] > 0"}]}]}}"#,
    );
    let report = s.execute("model.validate", &json!({})).unwrap();
    // The same text is fine in a calculated column but not in a measure.
    assert_eq!(codes(entry(&report, "Bare")), ["no-row-context"]);
    assert_eq!(codes(entry(&report, "Copy")), ["calculated-table-returns-scalar"]);
    assert_eq!(entry(&report, "Copy")["kind"], "calculated table");
    assert_eq!(report["invalid_count"], 2, "{report:#}");
    assert_eq!(report["cycles"], json!([]));
    let names: Vec<&str> = report["objects_with_diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap())
        .collect();
    assert!(
        !names.contains(&"Double") && !names.contains(&"Reader"),
        "{names:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `model.validate` (one shared validator for the pass, gap G5) reports exactly what validating
/// each object on its own did before: same diagnostics, validity, result types and counts.
#[test]
fn whole_model_pass_matches_per_object_validation() {
    use bicraft_dax::{Context, ExprKind, deps, validate_as};
    use bicraft_model::{Column, DataType, Measure, Role, SemanticModel, Table, TablePermission};

    let measure = |name: String, expression: String| Measure {
        name,
        expression,
        ..Default::default()
    };
    let mut tables = Vec::new();
    for t in 0..3 {
        let name = format!("T{t}");
        let mut measures = Vec::new();
        for i in 0..40 {
            // Chains of 10 across tables, a 3-cycle in T1, a self-reference in T2.
            let expression = match (t, i) {
                (1, 0) => "[T1 M2] + 1".to_string(),
                (1, 1) => "[T1 M0] * 2".to_string(),
                (1, 2) => "[T1 M1] - [T0 M5]".to_string(),
                (2, 7) => "[T2 M7] + 1".to_string(),
                (_, i) if i % 10 == 0 => format!("SUM({name}[Amount])"),
                (_, i) if i % 4 == 0 => format!("IF([T{t} M{}] > 0, VALUES({name}[Name]))", i - 1),
                (_, i) if i % 3 == 0 => format!("CALCULATE([t{t} m{}], {name}[Qty] > [T0 M1])", i - 1),
                (_, i) => format!("[T{t} M{}] + {i}", i - 1),
            };
            measures.push(measure(format!("T{t} M{i}"), expression));
        }
        tables.push(Table {
            name: name.clone(),
            columns: vec![
                Column {
                    name: "Amount".into(),
                    data_type: DataType::Double,
                    ..Default::default()
                },
                Column {
                    name: "Qty".into(),
                    data_type: DataType::Int64,
                    ..Default::default()
                },
                Column {
                    name: "Name".into(),
                    data_type: DataType::String,
                    ..Default::default()
                },
                Column {
                    name: "Calc".into(),
                    expression: Some(format!("{name}[Qty] * [T{t} M3]")),
                    ..Default::default()
                },
            ],
            measures,
            ..Default::default()
        });
    }
    let model = SemanticModel {
        name: "M".into(),
        tables,
        roles: vec![Role {
            name: "R".into(),
            table_permissions: vec![TablePermission {
                table: "T0".into(),
                filter_expression: Some("T0[Qty] > [T0 M4]".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    // The per-object pass, as `model.validate` ran it before.
    let mut cycle_diags = deps::cycle_diagnostics(&model);
    let mut expected = Vec::new();
    let (mut invalid, mut checked) = (0, 0);
    let mut objects = Vec::new();
    for t in &model.tables {
        for x in &t.measures {
            let id = deps::ObjectRef::Measure {
                table: t.name.clone(),
                name: x.name.clone(),
            };
            objects.push((
                "measure",
                ExprKind::Measure,
                t.name.clone(),
                x.name.clone(),
                x.expression.clone(),
                id,
            ));
        }
        for c in &t.columns {
            if let Some(e) = &c.expression {
                let id = deps::ObjectRef::Column {
                    table: t.name.clone(),
                    name: c.name.clone(),
                };
                objects.push((
                    "calculated column",
                    ExprKind::CalculatedColumn,
                    t.name.clone(),
                    c.name.clone(),
                    e.clone(),
                    id,
                ));
            }
        }
    }
    let id = deps::ObjectRef::RowFilter {
        role: "R".into(),
        table: "T0".into(),
    };
    objects.push((
        "row filter",
        ExprKind::RowFilter,
        "T0".into(),
        "R".into(),
        "T0[Qty] > [T0 M4]".into(),
        id,
    ));
    for (label, kind, table, name, expression, id) in objects {
        checked += 1;
        let ctx = Context {
            model: Some(&model),
            table: Some(&table),
        };
        let v = validate_as(&expression, &ctx, kind);
        let mut diagnostics = v.diagnostics;
        let mut valid = v.valid;
        while let Some(i) = cycle_diags.iter().position(|(o, _)| *o == id) {
            diagnostics.push(cycle_diags.swap_remove(i).1);
            valid = false;
        }
        if !valid {
            invalid += 1;
        }
        if !diagnostics.is_empty() {
            let mut entry = json!({"table": table, "kind": label, "name": name, "valid": valid,
                "diagnostics": diagnostics, "result_type": v.result_type});
            if kind == ExprKind::RowFilter {
                entry["role"] = json!(name);
            }
            expected.push(entry);
        }
    }
    let mut s = Session::new();
    s.model = Some(model.clone());
    let report = s.execute("model.validate", &json!({})).unwrap();
    assert_eq!(report["objects_with_diagnostics"], json!(expected), "{report:#}");
    assert_eq!(report["invalid_count"], invalid);
    assert_eq!(report["checked_count"], checked);
    assert_eq!(report["valid"], invalid == 0);
    assert!(invalid >= 4, "{report:#}");
    assert_eq!(report["cycles"], json!(deps::find_cycles(&model)));
}

/// `dax.validate {kind}` checks the expression as that object (the formula bar's live check),
/// `dax.functions` lists the catalog with signatures, and snapshots flag calculated tables.
#[test]
fn the_formula_bar_checks_as_the_object_and_lists_functions() {
    let (mut s, _dir) = open(
        "formula-bar",
        r#"{"name":"M","model":{"tables":[{"name":"Sales",
            "columns":[{"name":"Amount","dataType":"double"}],
            "partitions":[{"name":"p","source":{"type":"m","expression":"x"}}]}]}}"#,
    );
    let valid = |s: &mut Session, e: &str, kind: Option<&str>| -> bool {
        let mut p = json!({"expression": e, "table": "Sales"});
        if let Some(k) = kind {
            p["kind"] = json!(k);
        }
        s.execute("dax.validate", &p).unwrap()["valid"].as_bool().unwrap()
    };
    // a bare column reference needs a row context: fine in a column, not in a measure
    assert!(valid(&mut s, "Sales[Amount] * 2", Some("calculated_column")));
    assert!(!valid(&mut s, "Sales[Amount] * 2", Some("measure")));
    assert!(valid(&mut s, "SUM(Sales[Amount])", Some("measure")));
    // a calculated table returns a table
    assert!(valid(
        &mut s,
        "FILTER(Sales, Sales[Amount] > 1)",
        Some("calculated_table")
    ));
    assert!(!valid(&mut s, "1 + 1", Some("calculated_table")));
    let e = s
        .execute("dax.validate", &json!({"expression": "1", "kind": "visual"}))
        .unwrap_err();
    assert!(e.contains("unknown kind"), "{e}");
    let bad = s
        .execute(
            "dax.validate",
            &json!({"expression": "SUM(Sales[Nope])", "kind": "measure"}),
        )
        .unwrap();
    let d = &bad["diagnostics"][0];
    assert_eq!(d["span"]["start"], 4, "{bad:#}");
    assert!(d["span"]["end"].as_u64().unwrap() > 4);

    let all = s.execute("dax.functions", &json!({})).unwrap();
    let fns = all["functions"].as_array().unwrap();
    assert!(fns.len() > 300);
    let divide = fns.iter().find(|f| f["name"] == "DIVIDE").unwrap();
    assert_eq!(
        divide["signature"],
        "DIVIDE(numerator, denominator[, alternate_result])"
    );
    assert_eq!(divide["params"][2]["optional"], true);
    assert!(all["keywords"].as_array().unwrap().contains(&json!("RETURN")));
    let some = s.execute("dax.functions", &json!({"query": "divi"})).unwrap();
    assert_eq!(some["functions"].as_array().unwrap().len(), 1);

    let snap = s.snapshot();
    assert_eq!(snap["model"]["tables"][0]["calculated"], false);
    s.execute(
        "table.add",
        &json!({"name": "Big", "partition": {"source_type": "calculated", "expression": "FILTER(Sales, Sales[Amount] > 1)"}}),
    )
    .unwrap();
    let snap = s.snapshot();
    let big = snap["model"]["tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "Big")
        .unwrap();
    assert_eq!(big["calculated"], true);
}
