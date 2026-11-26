//! `partition.*` and `expression.*` (`partition.rs`), Desktop's default annotations
//! (`desktop_defaults.rs`), round trips through PBIP (TMSL and TMDL) and the PBIX rules.

use std::path::{Path, PathBuf};

use bicraft_model::SemanticModel;
use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn fails(s: &mut Session, id: &str, params: Value) -> String {
    match s.execute(id, &params) {
        Ok(v) => panic!("{id} {params} should fail, got {v}"),
        Err(e) => e,
    }
}

fn model(s: &Session) -> &SemanticModel {
    s.model.as_ref().unwrap()
}

const DATABRICKS: &str = "let\n    Source = Databricks.Catalogs(Host, HttpPath, [Catalog = null, Database = null]),\n    \
                          Navigation = Source{[Name = \"main\", Kind = \"Database\"]}[Data],\n    \
                          sales = Navigation{[Name = \"sales\", Kind = \"Table\"]}[Data]\nin\n    sales";

/// A model shaped like a Databricks project: two parameters, a fact table loaded from
/// Databricks with columns, a measure and a relationship, and a dimension.
fn databricks() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Lake"}));
    for (name, value) in [
        ("Host", "adb-1.azuredatabricks.net"),
        ("HttpPath", "/sql/1.0/warehouses/x"),
    ] {
        run(
            &mut s,
            "expression.add",
            json!({"name": name, "expression": format!("\"{value}\" meta [IsParameterQuery = true, Type = \"Text\", IsParameterQueryRequired = true]")}),
        );
    }
    run(
        &mut s,
        "table.add",
        json!({"name": "Sales", "partition": {"source_type": "m", "expression": DATABRICKS}}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "ProductKey", "data_type": "int64"}),
    );
    run(
        &mut s,
        "table.add",
        json!({"name": "Product", "partition": {"source_type": "m", "expression": DATABRICKS.replace("sales", "product")}}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Product", "name": "ProductKey", "data_type": "int64"}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Product", "name": "Name", "data_type": "string"}),
    );
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[ProductKey]", "to": "Product[ProductKey]"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
    );
    s
}

#[test]
fn partition_set_replaces_the_source_and_keeps_everything_else() {
    let mut s = databricks();
    let before = model(&s).clone();
    let filtered = DATABRICKS.replace(
        "in\n    sales",
        ",\n    #\"Filtered Rows\" = Table.SelectRows(sales, each [Amount] > 0)\nin\n    #\"Filtered Rows\"",
    );
    let filtered = filtered.replacen("[Data]\n,", "[Data],", 1);
    let v = run(
        &mut s,
        "partition.set",
        json!({"table": "Sales", "expression": filtered}),
    );
    assert_eq!(v["changed"], true);
    assert_eq!(v["previous_expression"], DATABRICKS);
    assert_eq!(v["partition"], "Sales");
    assert_eq!(v["m_diagnostics"], json!([]));
    let after = model(&s);
    let (b, a) = (before.table("Sales").unwrap(), after.table("Sales").unwrap());
    assert_eq!(a.partitions.len(), 1);
    assert_eq!(a.partitions[0].expression.as_deref(), Some(filtered.as_str()));
    assert_eq!(a.partitions[0].name, b.partitions[0].name);
    assert_eq!(a.columns, b.columns);
    assert_eq!(a.measures, b.measures);
    assert_eq!(a.lineage_tag, b.lineage_tag);
    assert_eq!(a.annotations, b.annotations);
    assert_eq!(after.relationships, before.relationships);
    // Same text again: no change.
    let v = run(
        &mut s,
        "table.set_partition",
        json!({"table": "Sales", "expression": filtered}),
    );
    assert_eq!(v["changed"], false);
}

#[test]
fn partition_set_checks_m_without_running_it() {
    let mut s = databricks();
    let e = fails(
        &mut s,
        "partition.set",
        json!({"table": "Sales", "expression": "let Source = Databricks.Catalogs(Hots, HttpPath, []) in Source"}),
    );
    assert!(e.contains("`Hots` is not defined"), "{e}");
    assert!(e.contains("1:34"), "{e}");
    let e = fails(
        &mut s,
        "partition.set",
        json!({"table": "Sales", "expression": "let a = in a"}),
    );
    assert!(e.contains("invalid M"), "{e}");
    // force applies it, with the diagnostics.
    let v = run(
        &mut s,
        "partition.set",
        json!({"table": "Sales", "expression": "let a = Unknown in a", "force": true}),
    );
    assert_eq!(v["m_diagnostics"][0]["code"], "m-undefined-name");
    // Unknown library functions only warn.
    let v = run(
        &mut s,
        "partition.set",
        json!({"table": "Sales", "expression": "Table.Frobnicate(#table({}, {}))"}),
    );
    assert_eq!(v["m_diagnostics"][0]["severity"], "warning");
    fails(
        &mut s,
        "partition.set",
        json!({"table": "Nope", "expression": "1"}),
    );
    fails(
        &mut s,
        "partition.set",
        json!({"table": "Sales", "partition": "Nope", "expression": "1"}),
    );
    fails(
        &mut s,
        "partition.set",
        json!({"table": "Sales", "expression": "1", "mode": "bogus"}),
    );
    // A kind change would change the columns.
    let e = fails(
        &mut s,
        "partition.set",
        json!({"table": "Sales", "expression": "Product", "kind": "calculated"}),
    );
    assert!(e.contains("changes its columns"), "{e}");
}

#[test]
fn partition_set_on_calculated_tables_binds_dax() {
    let mut s = databricks();
    run(
        &mut s,
        "table.add",
        json!({"name": "Big", "partition": {"source_type": "calculated", "expression": "FILTER(Sales, Sales[Amount] > 10)"}}),
    );
    run(
        &mut s,
        "partition.set",
        json!({"table": "Big", "expression": "FILTER(Sales, Sales[Amount] > 100)"}),
    );
    let e = fails(
        &mut s,
        "partition.set",
        json!({"table": "Big", "expression": "FILTER(Sales, Sales[Nope] > 1)"}),
    );
    assert!(e.contains("invalid DAX"), "{e}");
    // A table without a partition gets one.
    run(&mut s, "table.add", json!({"name": "Empty"}));
    let v = run(
        &mut s,
        "partition.set",
        json!({"table": "Empty", "expression": "#table({\"A\"}, {{1}})", "mode": "import"}),
    );
    assert_eq!(v["partition"], "Empty");
    assert_eq!(v["kind"], "m");
    let t = model(&s).table("Empty").unwrap();
    assert!(
        t.annotations
            .iter()
            .any(|a| a.name == "PBI_ResultType" && a.value == "Table")
    );
}

/// A model with a loadable import table (an M `#table` literal) and a calculated table over it.
fn literal_sales() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.add",
        json!({"name": "Sales", "partition": {"source_type": "m", "expression":
            "#table(type table [Amount = number, Key = Int64.Type, Region = text], {{10, 1, \"N\"}, {20, 2, \"S\"}})"}}),
    );
    for (name, ty) in [("Amount", "double"), ("Key", "int64"), ("Region", "string")] {
        run(
            &mut s,
            "column.add",
            json!({"table": "Sales", "name": name, "data_type": ty}),
        );
    }
    run(
        &mut s,
        "table.add",
        json!({"name": "Big", "partition": {"source_type": "calculated", "expression": "FILTER(Sales, Sales[Amount] > 10)"}}),
    );
    s
}

fn column_names(s: &Session, table: &str) -> Vec<String> {
    model(s)
        .table(table)
        .unwrap()
        .columns
        .iter()
        .filter(|c| c.extra.get("type").and_then(Value::as_str) != Some("rowNumber"))
        .map(|c| c.name.clone())
        .collect()
}

#[test]
fn a_calculated_tables_new_expression_gives_its_columns() {
    let mut s = literal_sales();
    run(
        &mut s,
        "partition.set",
        json!({"table": "Big", "expression": "FILTER(Sales, Sales[Amount] > 5)"}),
    );
    assert_eq!(column_names(&s, "Big"), ["Amount", "Key", "Region"]);
    // a kept column keeps what was set on it
    run(
        &mut s,
        "column.set",
        json!({"table": "Big", "name": "Amount", "description": "kept"}),
    );
    let v = run(
        &mut s,
        "partition.set",
        json!({"table": "Big", "expression": "SELECTCOLUMNS(Sales, \"Amount\", Sales[Amount], \"Twice\", Sales[Amount] * 2)"}),
    );
    assert_eq!(column_names(&s, "Big"), ["Amount", "Twice"], "{v}");
    let big = model(&s).table("Big").unwrap();
    assert_eq!(big.column("Amount").unwrap().description.as_deref(), Some("kept"));
    assert_eq!(v["columns"]["removed"], json!(["Key", "Region"]));
    assert_eq!(v["columns"]["added"], json!(["Twice"]));
    // the expression stays as written (a removal does not select the other columns in it)
    assert!(
        big.partitions[0]
            .expression
            .as_deref()
            .unwrap()
            .starts_with("SELECTCOLUMNS")
    );
}

#[test]
fn a_new_expression_and_name_are_one_undo_step() {
    let mut s = literal_sales();
    let before = model(&s).clone();
    let v = run(
        &mut s,
        "partition.set",
        json!({"table": "Big", "expression": "Sales", "new_name": "All sales"}),
    );
    assert_eq!(v["renamed"]["to"], "All sales", "{v}");
    let t = model(&s).table("All sales").unwrap();
    assert_eq!(t.partitions[0].expression.as_deref(), Some("Sales"));
    assert_eq!(column_names(&s, "All sales"), ["Amount", "Key", "Region"]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(
        model(&s),
        &before,
        "one undo takes the name and the expression back"
    );
    // a name already taken changes nothing
    let e = fails(
        &mut s,
        "partition.set",
        json!({"table": "Big", "expression": "Sales", "new_name": "sales"}),
    );
    assert!(e.contains("already exists"), "{e}");
    assert_eq!(model(&s), &before);
}

#[test]
fn partitions_add_and_remove() {
    let mut s = databricks();
    let v = run(
        &mut s,
        "partition.add",
        json!({"table": "Sales", "expression": DATABRICKS}),
    );
    assert_eq!(v["partition"], "Sales-2");
    fails(
        &mut s,
        "partition.add",
        json!({"table": "Sales", "expression": "Sales", "kind": "calculated"}),
    );
    fails(
        &mut s,
        "partition.add",
        json!({"table": "Sales", "name": "sales-2", "expression": DATABRICKS}),
    );
    let e = fails(
        &mut s,
        "partition.set",
        json!({"table": "Sales", "expression": DATABRICKS}),
    );
    assert!(e.contains("pass `partition`"), "{e}");
    run(
        &mut s,
        "partition.remove",
        json!({"table": "Sales", "name": "Sales-2"}),
    );
    let e = fails(
        &mut s,
        "partition.remove",
        json!({"table": "Sales", "name": "Sales"}),
    );
    assert!(e.contains("only partition"), "{e}");
}

#[test]
fn expressions_add_set_rename_remove() {
    let mut s = databricks();
    let m = model(&s);
    let host = m.expressions.iter().find(|e| e.name == "Host").unwrap();
    assert_eq!(host.kind, "m");
    assert!(host.lineage_tag.is_some());
    assert_eq!(
        host.annotations
            .iter()
            .map(|a| (a.name.as_str(), a.value.as_str()))
            .collect::<Vec<_>>(),
        [("PBI_ResultType", "Text")]
    );
    fails(
        &mut s,
        "expression.add",
        json!({"name": "sales", "expression": "1"}),
    );
    fails(
        &mut s,
        "expression.add",
        json!({"name": "host", "expression": "1"}),
    );
    fails(&mut s, "expression.add", json!({"name": "X", "expression": "Y"}));
    // Rename: the queries that use it follow, quoted where needed.
    let v = run(
        &mut s,
        "expression.rename",
        json!({"name": "Host", "new_name": "Server Host"}),
    );
    assert_eq!(v["m_rewritten"].as_array().unwrap().len(), 2, "{v}");
    let p = model(&s).table("Sales").unwrap().partitions[0]
        .expression
        .clone()
        .unwrap();
    assert!(
        p.contains("Databricks.Catalogs(#\"Server Host\", HttpPath"),
        "{p}"
    );
    // Remove: refused while used.
    let e = fails(&mut s, "expression.remove", json!({"name": "Server Host"}));
    assert!(e.contains("used by 2 M queries"), "{e}");
    run(
        &mut s,
        "expression.set",
        json!({"name": "Server Host", "description": "the workspace", "query_group": "Parameters"}),
    );
    let e = &model(&s).expressions[0];
    assert_eq!(e.description.as_deref(), Some("the workspace"));
    assert_eq!(e.extra["queryGroup"], "Parameters");
    run(
        &mut s,
        "expression.set",
        json!({"name": "Server Host", "expression": "\"other\""}),
    );
    let v = run(
        &mut s,
        "expression.remove",
        json!({"name": "Server Host", "force": true}),
    );
    assert_eq!(v["used_by"].as_array().unwrap().len(), 2);
    assert!(model(&s).expressions.iter().all(|e| e.name != "Server Host"));
}

#[test]
fn new_objects_get_desktop_default_annotations_and_sets_do_not_touch_them() {
    let mut s = databricks();
    let m = model(&s);
    let sales = m.table("Sales").unwrap();
    let ann = |a: &[bicraft_model::Annotation]| {
        a.iter()
            .map(|a| format!("{}={}", a.name, a.value))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ann(&sales.annotations),
        ["PBI_NavigationStepName=Navigation", "PBI_ResultType=Table"]
    );
    let amount = sales.column("Amount").unwrap();
    assert_eq!(amount.summarize_by.as_deref(), Some("sum"));
    assert!(ann(&amount.annotations).contains(&"SummarizationSetBy=Automatic".to_string()));
    let name = m.table("Product").unwrap().column("Name").unwrap();
    assert_eq!(name.summarize_by.as_deref(), Some("none"));
    assert_eq!(ann(&name.annotations), ["SummarizationSetBy=Automatic"]);
    run(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "Qty", "data_type": "int64", "summarize_by": "none"}),
    );
    let qty = model(&s).table("Sales").unwrap().column("Qty").unwrap();
    assert!(ann(&qty.annotations).contains(&"SummarizationSetBy=User".to_string()));
    // No-op sets leave the model as it is.
    let before = model(&s).clone();
    run(
        &mut s,
        "column.set",
        json!({"table": "Sales", "name": "Amount", "hidden": false}),
    );
    run(
        &mut s,
        "measure.set",
        json!({"name": "Total", "expression": "SUM(Sales[Amount])"}),
    );
    assert_eq!(model(&s), &before);
}

fn temp(name: &str) -> TempDir {
    TempDir::new(&format!("engine-partition-{name}"))
}

fn reopen(path: &Path) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    assert_eq!(s.model_error, None);
    s
}

#[test]
fn pbip_round_trips_keep_edited_partitions_and_query_order() {
    for format in ["tmsl", "tmdl"] {
        let dir = temp(format);
        let mut s = databricks();
        run(
            &mut s,
            "project.new",
            json!({"dir": dir.path(), "name": "Lake", "format": format}),
        );
        let pbip: PathBuf = dir.join("Lake.pbip");
        let mut s = reopen(&pbip);
        let changed = DATABRICKS.replace("\"main\"", "\"gold\"");
        run(
            &mut s,
            "partition.set",
            json!({"table": "Sales", "expression": changed}),
        );
        run(
            &mut s,
            "expression.rename",
            json!({"name": "HttpPath", "new_name": "Path"}),
        );
        run(&mut s, "file.save", json!({}));
        let again = reopen(&pbip);
        assert_eq!(again.model, s.model, "{format}");
        let m = model(&again);
        let order = m.annotations.iter().find(|a| a.name == "PBI_QueryOrder").unwrap();
        assert_eq!(
            order.value,
            r#"["Host","HttpPath","Sales","Product"]"#.replace("HttpPath", "Path")
        );
        let p = m.table("Sales").unwrap().partitions[0]
            .expression
            .clone()
            .unwrap();
        assert!(p.contains("\"gold\"") && p.contains("(Host, Path,"), "{p}");
        // A second save of the unchanged project writes the same bytes.
        let files = |d: &Path| -> Vec<(PathBuf, Vec<u8>)> {
            let mut out = Vec::new();
            let mut todo = vec![d.to_path_buf()];
            while let Some(d) = todo.pop() {
                for e in std::fs::read_dir(&d).unwrap() {
                    let p = e.unwrap().path();
                    if p.is_dir() {
                        todo.push(p);
                    } else {
                        out.push((p.clone(), std::fs::read(&p).unwrap()));
                    }
                }
            }
            out.sort();
            out
        };
        let first = files(&dir.join("Lake.SemanticModel"));
        let mut again = reopen(&pbip);
        run(
            &mut again,
            "column.set",
            json!({"table": "Sales", "name": "Amount", "hidden": false}),
        );
        run(&mut again, "file.save", json!({}));
        assert_eq!(files(&dir.join("Lake.SemanticModel")), first, "{format}");
    }
}

const SYNTHETIC_METADATA: &[u8] =
    include_bytes!("../../../metadata/tests/fixtures/synthetic_metadata.sqlitedb");

#[test]
fn pbix_refuses_sources_it_cannot_reload() {
    let dir = temp("pbix");
    let path = dir.join("synthetic.pbix");
    let pkg = bicraft_pbix::synthetic::pbix(SYNTHETIC_METADATA).unwrap();
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    let mut s = reopen(&path);
    let table = model(&s)
        .tables
        .iter()
        .find(|t| t.partitions.iter().any(|p| p.source_type == "m"))
        .map(|t| t.name.clone())
        .unwrap();
    let e = fails(
        &mut s,
        "partition.set",
        json!({"table": table, "expression": "let Source = Sql.Database(\"srv\", \"db\") in Source"}),
    );
    assert!(e.contains("Sql.Database") && e.contains("PBIP"), "{e}");
    let e = fails(&mut s, "expression.add", json!({"name": "P", "expression": "1"}));
    assert!(e.contains("PBIP"), "{e}");
    // A source bicraft runs is accepted; file.save reloads the table.
    let v = run(
        &mut s,
        "partition.set",
        json!({"table": table, "expression": "#table({\"A\"}, {{1}})", "force": true}),
    );
    assert!(v["warnings"].to_string().contains("reloads the table"), "{v}");

    // A PBIX bicraft built: the save reloads the table with the new query and keeps its text.
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "P"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "T", "m": "let Source = #table(type table [A = Int64.Type], {{1}, {2}}) in Source"}),
    );
    let built = dir.join("built.pbix");
    run(&mut s, "file.save_as", json!({"path": built}));
    let mut s = reopen(&built);
    let new = "let\n    Source = #table(type table [A = Int64.Type], {{1}, {2}, {3}})\nin\n    Source";
    run(&mut s, "partition.set", json!({"table": "T", "expression": new}));
    run(&mut s, "file.save", json!({"backup": false}));
    let mut again = reopen(&built);
    let t = model(&again).table("T").unwrap();
    assert_eq!(t.partitions[0].expression.as_deref(), Some(new));
    assert_eq!(t.columns, model(&s).table("T").unwrap().columns);
    let v = run(
        &mut again,
        "dax.evaluate",
        json!({"query": "EVALUATE ROW(\"n\", COUNTROWS(T))"}),
    );
    assert!(v.to_string().contains('3'), "{v}");
}

#[test]
fn imported_columns_get_desktop_formats_from_their_m_types() {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Types"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "T", "m": "#table(type table [I = Int64.Type, N = number, D = date, DT = datetime, Tm = time, S = text, B = logical], \
               {{1, 1.5, #date(2024, 1, 2), #datetime(2024, 1, 2, 3, 4, 5), #time(1, 2, 3), \"a\", true}})"}),
    );
    let t = model(&s).table("T").unwrap();
    let shape = |name: &str| {
        let c = t.column(name).unwrap();
        let ann: Vec<String> = c
            .annotations
            .iter()
            .map(|a| format!("{}={}", a.name, a.value))
            .collect();
        (c.format_string.clone(), c.summarize_by.clone(), ann)
    };
    let auto = "SummarizationSetBy=Automatic".to_string();
    assert_eq!(
        shape("I"),
        (Some("0".into()), Some("sum".into()), vec![auto.clone()])
    );
    assert_eq!(
        shape("N"),
        (
            None,
            Some("sum".into()),
            vec![auto.clone(), "PBI_FormatHint={\"isGeneralNumber\":true}".into()]
        )
    );
    assert_eq!(
        shape("D"),
        (
            Some("Long Date".into()),
            Some("none".into()),
            vec![auto.clone(), "UnderlyingDateTimeDataType=Date".into()]
        )
    );
    assert_eq!(
        shape("DT"),
        (
            Some("General Date".into()),
            Some("none".into()),
            vec![auto.clone()]
        )
    );
    assert_eq!(
        shape("Tm"),
        (
            Some("Long Time".into()),
            Some("none".into()),
            vec![auto.clone(), "UnderlyingDateTimeDataType=Time".into()]
        )
    );
    assert_eq!(shape("S"), (None, Some("none".into()), vec![auto.clone()]));
    assert_eq!(shape("B").0.as_deref(), Some("\"TRUE\";\"TRUE\";\"FALSE\""));
    assert_eq!(
        t.annotations
            .iter()
            .map(|a| format!("{}={}", a.name, a.value))
            .collect::<Vec<_>>(),
        ["PBI_ResultType=Table"]
    );
    // CSV: a date-only column is written as an M `date` column, and annotated as one.
    run(
        &mut s,
        "table.import",
        json!({"name": "C", "csv": "Day,Amount\n2024-01-02,3\n2024-01-03,4\n"}),
    );
    let day = model(&s).table("C").unwrap().column("Day").unwrap().clone();
    assert_eq!(day.format_string.as_deref(), Some("Long Date"));
    assert!(
        day.annotations
            .iter()
            .any(|a| a.name == "UnderlyingDateTimeDataType" && a.value == "Date")
    );
}

/// `column.add` / `column.set` of a `dateTime` data column: Desktop's format string and
/// `UnderlyingDateTimeDataType` follow the type the table's M query declares (read from the
/// query text, not evaluated), and survive PBIP round trips.
#[test]
fn added_date_columns_get_desktop_defaults_from_the_query_text() {
    let query = "let\n    Source = Sql.Database(\"srv\", \"db\"),\n    F = Source{[Schema = \"dbo\", Item = \"F\"]}[Data],\n    \
                 Typed = Table.TransformColumnTypes(F, {{\"When\", type date}, {\"At\", type time}, {\"Stamp\", type datetime}, {\"Txt\", type text}}),\n    \
                 Added = Table.AddColumn(Typed, \"Day\", each Date.From([Stamp]), type date)\nin\n    Added";
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Dates"}));
    run(
        &mut s,
        "table.add",
        json!({"name": "F", "partition": {"source_type": "m", "expression": query}}),
    );
    run(
        &mut s,
        "table.add",
        json!({"name": "L", "partition": {"source_type": "m",
               "expression": "#table(type table [d = date], {{#date(2026, 1, 1)}})"}}),
    );
    for (table, name) in [
        ("F", "When"),
        ("F", "At"),
        ("F", "Stamp"),
        ("F", "Day"),
        ("L", "d"),
    ] {
        run(
            &mut s,
            "column.add",
            json!({"table": table, "name": name, "data_type": "dateTime"}),
        );
    }
    // a given format string wins; an unknown source column gets nothing
    run(
        &mut s,
        "column.add",
        json!({"table": "F", "name": "Other", "data_type": "dateTime", "source_column": "When",
               "format_string": "yyyy"}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "F", "name": "Unknown", "data_type": "dateTime"}),
    );
    // a type change follows the query: text → dateTime of a `date` source
    run(
        &mut s,
        "column.add",
        json!({"table": "F", "name": "Txt", "data_type": "string"}),
    );
    run(
        &mut s,
        "column.set",
        json!({"table": "F", "name": "Txt", "data_type": "dateTime", "source_column": "When"}),
    );
    let shape = |m: &SemanticModel, t: &str, c: &str| {
        let c = m.table(t).unwrap().column(c).unwrap();
        let u = c
            .annotations
            .iter()
            .find(|a| a.name == "UnderlyingDateTimeDataType")
            .map(|a| a.value.clone());
        (c.format_string.clone(), u)
    };
    let check = |m: &SemanticModel, what: &str| {
        let date = (Some("Long Date".to_string()), Some("Date".to_string()));
        assert_eq!(shape(m, "F", "When"), date, "{what}");
        assert_eq!(
            shape(m, "F", "At"),
            (Some("Long Time".into()), Some("Time".into())),
            "{what}"
        );
        assert_eq!(
            shape(m, "F", "Stamp"),
            (Some("General Date".into()), None),
            "{what}"
        );
        assert_eq!(shape(m, "F", "Day"), date, "{what}");
        assert_eq!(shape(m, "L", "d"), date, "{what}");
        assert_eq!(
            shape(m, "F", "Other"),
            (Some("yyyy".into()), Some("Date".into())),
            "{what}"
        );
        assert_eq!(shape(m, "F", "Unknown"), (None, None), "{what}");
        assert_eq!(shape(m, "F", "Txt"), date, "{what}");
    };
    check(model(&s), "session");
    // back to text: the date defaults go
    let mut back = Session::new();
    back.model = s.model.clone();
    run(
        &mut back,
        "column.set",
        json!({"table": "F", "name": "Txt", "data_type": "string"}),
    );
    assert_eq!(shape(model(&back), "F", "Txt"), (None, None));
    for format in ["tmsl", "tmdl"] {
        let dir = temp(&format!("dates-{format}"));
        let mut p = Session::new();
        p.model = s.model.clone();
        run(
            &mut p,
            "project.new",
            json!({"dir": dir.path(), "name": "Dates", "format": format}),
        );
        let again = reopen(&dir.join("Dates.pbip"));
        check(model(&again), format);
    }
}
