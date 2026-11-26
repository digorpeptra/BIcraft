//! PBIX files built without an existing `DataModel` (M5), on synthetic models: written by
//! `file.save_as`, then re-read by bicraft's own reader (model and stored values).
//!
//! With `BICRAFT_FRESH_OUT=<dir>` every built file is also copied there, for the engine and
//! Desktop checks of the maintainers' development oracles (kept outside this repository).

use std::path::Path;

use bicraft_pbix::structural::DataModelEditor;
use bicraft_vertipaq::CellValue;
use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Copies a built file to `BICRAFT_FRESH_OUT` when set.
fn keep(path: &Path) {
    if let Some(out) = bicraft_env::var_os("BICRAFT_FRESH_OUT") {
        let out = Path::new(&out);
        let _ = std::fs::create_dir_all(out);
        if let Some(name) = path.file_name() {
            let _ = std::fs::copy(path, out.join(name));
        }
    }
}

fn editor(path: &Path) -> DataModelEditor {
    let pkg = bicraft_opc::Package::open(path).unwrap();
    let part = pkg.get(bicraft_pbix::DATA_MODEL).unwrap();
    DataModelEditor::open(&part.data, &bicraft_pbix::DataModelLimits::default()).unwrap()
}

fn values(e: &DataModelEditor, table: &str, column: &str) -> Vec<CellValue> {
    e.column_values(table, column)
        .unwrap_or_else(|err| panic!("{table}[{column}]: {err}"))
}

fn texts(v: &[&str]) -> Vec<CellValue> {
    v.iter()
        .map(|s| {
            if s.is_empty() {
                CellValue::Null
            } else {
                CellValue::Text((*s).to_string())
            }
        })
        .collect()
}

#[test]
fn one_literal_table() {
    let dir = TempDir::new("fresh-one");
    let path = dir.join("fresh_one.pbix");
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "One"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Product", "m": "let Source = #table(type table [Key = Int64.Type, Name = text, Price = number], {{1, \"Apple\", 1.5}, {2, \"Banana\", 0.25}, {3, null, 2}}) in Source"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Product", "name": "Rows", "expression": "COUNTROWS(Product)"}),
    );
    let out = run(&mut s, "file.save_as", json!({"path": path}));
    assert_eq!(out["built_from_scratch"], true, "{out}");
    assert_eq!(out["converted_from"], "new");
    keep(&path);
    // The session now holds the PBIX; it reads back with the data.
    let m = s.model.as_ref().unwrap();
    assert_eq!(m.tables.len(), 1);
    assert_eq!(m.tables[0].measures.len(), 1);
    let e = editor(&path);
    assert_eq!(e.row_count("Product").unwrap(), Some(3));
    assert_eq!(
        values(&e, "Product", "Key"),
        [CellValue::Int(1), CellValue::Int(2), CellValue::Int(3)]
    );
    assert_eq!(values(&e, "Product", "Name"), texts(&["Apple", "Banana", ""]));
    assert_eq!(
        values(&e, "Product", "Price"),
        [CellValue::Real(1.5), CellValue::Real(0.25), CellValue::Real(2.0)]
    );
    // A second save of the reopened file is an ordinary PBIX save.
    run(
        &mut s,
        "measure.add",
        json!({"table": "Product", "name": "Top", "expression": "MAX(Product[Price])"}),
    );
    run(&mut s, "file.save", json!({"backup": false}));
    let back = bicraft_pbix::read_model(&bicraft_opc::Package::open(&path).unwrap()).unwrap();
    assert_eq!(back.tables[0].measures.len(), 2);
}

/// A model of several tables with every feature a new PBIX carries.
fn star_bim(data: &Path) -> String {
    let csv = data.join("sales.csv");
    let csv = csv.display().to_string().replace('\\', "\\\\");
    let m_sales = format!(
        "let Source = Csv.Document(File.Contents(\"{csv}\"), [Delimiter=\",\", Encoding=65001, QuoteStyle=QuoteStyle.Csv]), \
         Promoted = Table.PromoteHeaders(Source, [PromoteAllScalars=true]), \
         Typed = Table.TransformColumnTypes(Promoted, {{{{\"Key\", Int64.Type}}, {{\"Date\", type date}}, {{\"Qty\", Int64.Type}}, \
         {{\"Price\", Currency.Type}}, {{\"Discount\", type number}}, {{\"Promo\", type logical}}, {{\"Note\", type text}}}}) in Typed"
    );
    let bim = json!({
        "name": "Star",
        "compatibilityLevel": 1606,
        "model": {
            "culture": "en-US",
            "defaultPowerBIDataSourceVersion": "powerBI_V3",
            "sourceQueryCulture": "en-US",
            "dataAccessOptions": {"legacyRedirects": true, "returnErrorValuesAsNull": true},
            "tables": [
                {
                    "name": "Product",
                    "columns": [
                        {"name": "Key", "dataType": "int64", "sourceColumn": "Key", "summarizeBy": "none"},
                        {"name": "Name", "dataType": "string", "sourceColumn": "Name"},
                        {"name": "Category", "dataType": "string", "sourceColumn": "Category"}
                    ],
                    "partitions": [{"name": "Product", "mode": "import", "source": {"type": "m",
                        "expression": "let Source = #table(type table [Key = Int64.Type, Name = text, Category = text], {{1, \"Apple\", \"Fruit\"}, {2, \"Banana\", \"Fruit\"}, {3, \"Leek\", \"Vegetable\"}, {4, \"Kale\", null}}) in Source"}}],
                    "measures": [{"name": "Products", "expression": "COUNTROWS(Product)"}],
                    "hierarchies": [{"name": "Catalog", "levels": [
                        {"name": "Category", "ordinal": 0, "column": "Category"},
                        {"name": "Name", "ordinal": 1, "column": "Name"}
                    ]}]
                },
                {
                    "name": "Sales",
                    "columns": [
                        {"name": "Key", "dataType": "int64", "sourceColumn": "Key", "summarizeBy": "none"},
                        {"name": "Date", "dataType": "dateTime", "sourceColumn": "Date", "formatString": "Long Date"},
                        {"name": "Qty", "dataType": "int64", "sourceColumn": "Qty"},
                        {"name": "Price", "dataType": "decimal", "sourceColumn": "Price", "formatString": "\\$#,0.00"},
                        {"name": "Discount", "dataType": "double", "sourceColumn": "Discount"},
                        {"name": "Promo", "dataType": "boolean", "sourceColumn": "Promo"},
                        {"name": "Note", "dataType": "string", "sourceColumn": "Note"},
                        {"name": "Amount", "dataType": "decimal", "type": "calculated", "expression": "Sales[Qty] * Sales[Price]"}
                    ],
                    "partitions": [{"name": "Sales", "mode": "import", "source": {"type": "m", "expression": m_sales}}],
                    "measures": [
                        {"name": "Total", "expression": "SUM(Sales[Amount])", "formatString": "\\$#,0.00"},
                        {"name": "Avg Discount", "expression": "AVERAGE(Sales[Discount])"}
                    ]
                },
                {
                    "name": "Calendar",
                    "columns": [
                        {"name": "Month", "dataType": "int64", "type": "calculated", "expression": "MONTH('Calendar'[Date])"}
                    ],
                    "partitions": [{"name": "Calendar", "source": {"type": "calculated",
                        "expression": "CALENDAR(DATE(2026, 1, 1), DATE(2026, 2, 28))"}}],
                    "dataCategory": "Time"
                },
                {
                    "name": "Big Sales",
                    "partitions": [{"name": "Big Sales", "source": {"type": "calculated",
                        "expression": "FILTER(Sales, Sales[Qty] >= 3)"}}]
                }
            ],
            "relationships": [
                {"name": "6c1a0d52-2b55-4b1c-a3c9-8f3a5d7e9b01", "fromTable": "Sales", "fromColumn": "Key", "toTable": "Product", "toColumn": "Key"},
                {"name": "3e9b7f10-4d2a-4c8e-b6f1-0a2c4e6f8b13", "fromTable": "Sales", "fromColumn": "Date", "toTable": "Calendar", "toColumn": "Date"}
            ],
            "roles": [
                {"name": "Fruit only", "modelPermission": "read",
                 "tablePermissions": [{"name": "Product", "filterExpression": "Product[Category] = \"Fruit\""}]}
            ],
            "expressions": [
                {"name": "Threshold", "kind": "m", "expression": "3 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
                 "queryGroup": "Parameters"}
            ],
            "queryGroups": [{"folder": "Parameters"}],
            "annotations": [{"name": "PBI_QueryOrder", "value": "[\"Threshold\",\"Product\",\"Sales\"]"}]
        }
    });
    serde_json::to_string_pretty(&bim).unwrap()
}

const SALES_CSV: &str = "Key,Date,Qty,Price,Discount,Promo,Note\n\
1,2026-01-05,2,1.50,0.1,true,first\n\
2,2026-01-06,5,0.25,,false,\n\
1,2026-02-10,3,1.50,0.25,,\"quoted, note\"\n\
3,2026-02-11,,2.00,0,true,last\n";

#[test]
fn star_model_from_a_model_bim() {
    let dir = TempDir::new("fresh-star");
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("sales.csv"), SALES_CSV).unwrap();
    let bim = dir.join("model.bim");
    std::fs::write(&bim, star_bim(&data)).unwrap();
    let path = dir.join("fresh_star.pbix");
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": bim, "data_root": data}));
    let out = run(&mut s, "file.save_as", json!({"path": path}));
    assert_eq!(out["built_from_scratch"], true, "{out}");
    keep(&path);
    let e = editor(&path);
    let m = e.base();
    let names: Vec<&str> = m.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Product", "Sales", "Calendar", "Big Sales"]);
    assert_eq!(m.relationships.len(), 2);
    assert_eq!(m.roles.len(), 1);
    assert_eq!(m.expressions.len(), 1);
    assert_eq!(m.expressions[0].extra["queryGroup"], "Parameters");
    assert_eq!(m.annotations.len(), 1);
    assert_eq!(m.extra["defaultPowerBIDataSourceVersion"], "powerBI_V3");
    // Values, blanks included.
    assert_eq!(e.row_count("Sales").unwrap(), Some(4));
    assert_eq!(
        values(&e, "Sales", "Qty"),
        [
            CellValue::Int(2),
            CellValue::Int(5),
            CellValue::Int(3),
            CellValue::Null
        ]
    );
    assert_eq!(
        values(&e, "Sales", "Price"),
        [
            CellValue::Decimal(15_000),
            CellValue::Decimal(2_500),
            CellValue::Decimal(15_000),
            CellValue::Decimal(20_000)
        ]
    );
    assert_eq!(
        values(&e, "Sales", "Amount"),
        [
            CellValue::Decimal(30_000),
            CellValue::Decimal(12_500),
            CellValue::Decimal(45_000),
            CellValue::Null
        ]
    );
    assert_eq!(
        values(&e, "Sales", "Promo"),
        [
            CellValue::Bool(true),
            CellValue::Bool(false),
            CellValue::Null,
            CellValue::Bool(true)
        ]
    );
    // An empty CSV field is empty text (Csv.Document), not a blank.
    assert_eq!(
        values(&e, "Sales", "Note"),
        ["first", "", "quoted, note", "last"].map(|s| CellValue::Text(s.into()))
    );
    assert_eq!(
        values(&e, "Sales", "Date")[0],
        CellValue::DateTime(46_027.0),
        "2026-01-05"
    );
    assert_eq!(e.row_count("Calendar").unwrap(), Some(59));
    assert_eq!(values(&e, "Calendar", "Month")[58], CellValue::Int(2));
    assert_eq!(e.row_count("Big Sales").unwrap(), Some(2));
    assert_eq!(
        values(&e, "Product", "Category"),
        texts(&["Fruit", "Fruit", "Vegetable", ""])
    );
}

#[test]
fn templates_and_projects_without_a_cache_become_pbix() {
    let dir = TempDir::new("fresh-conv");
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Conv"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Region", "csv": "Code,Label,Since\n1,North,2020-01-01\n2,South,\n"}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Region", "name": "Next", "data_type": "int64", "expression": "Region[Code] + 1"}),
    );
    run(&mut s, "role.add", json!({"name": "North"}));
    run(
        &mut s,
        "role.set_filter",
        json!({"role": "North", "table": "Region", "filter_expression": "Region[Label] = \"North\""}),
    );
    let pbit = dir.join("conv.pbit");
    run(&mut s, "file.save_as", json!({"path": pbit}));
    // PBIT → PBIX.
    let from_pbit = dir.join("fresh_from_pbit.pbix");
    let out = run(&mut s, "file.convert", json!({"from": pbit, "to": from_pbit}));
    assert_eq!(out["converted_from"], "pbit", "{out}");
    assert_eq!(out["built_from_scratch"], true, "{out}");
    keep(&from_pbit);
    let e = editor(&from_pbit);
    assert_eq!(
        values(&e, "Region", "Next"),
        [CellValue::Int(2), CellValue::Int(3)]
    );
    assert_eq!(e.base().roles.len(), 1);
    // PBIP without a cache → PBIX.
    let mut p = Session::new();
    run(&mut p, "file.open", json!({"path": pbit}));
    run(&mut p, "file.save_as", json!({"path": dir.join("Conv.pbip")}));
    let from_pbip = dir.join("fresh_from_pbip.pbix");
    let out = run(&mut p, "file.save_as", json!({"path": from_pbip}));
    assert_eq!(out["converted_from"], "pbip", "{out}");
    keep(&from_pbip);
    let e = editor(&from_pbip);
    assert_eq!(e.row_count("Region").unwrap(), Some(2));
    assert_eq!(
        values(&e, "Region", "Since"),
        [CellValue::DateTime(43_831.0), CellValue::Null]
    );
}

#[test]
fn external_sources_are_refused_or_written_empty() {
    let dir = TempDir::new("fresh-ext");
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Ext"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Local", "m": "#table(type table [A = Int64.Type], {{1}, {2}})"}),
    );
    run(
        &mut s,
        "table.add",
        json!({"name": "Remote", "partition": {"source_type": "m",
        "expression": "let Source = Sql.Database(\"server.example\", \"db\"), T = Source{[Schema=\"dbo\",Item=\"T\"]}[Data] in T"}}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Remote", "name": "Id", "data_type": "int64"}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Remote", "name": "Label", "data_type": "string"}),
    );
    let path = dir.join("fresh_external.pbix");
    let e = s.execute("file.save_as", &json!({"path": path})).unwrap_err();
    assert!(e.contains("'Remote'") && e.contains("empty_tables"), "{e}");
    assert!(!path.exists());
    let out = run(
        &mut s,
        "file.save_as",
        json!({"path": path, "empty_tables": true}),
    );
    assert_eq!(out["empty_tables"], json!(["Remote"]), "{out}");
    keep(&path);
    let e = editor(&path);
    assert_eq!(e.row_count("Remote").unwrap(), Some(0));
    assert_eq!(e.row_count("Local").unwrap(), Some(2));
    // The real query is kept, for Desktop's refresh.
    let remote = e.base().tables.iter().find(|t| t.name == "Remote").unwrap();
    assert!(
        remote.partitions[0]
            .expression
            .as_deref()
            .unwrap()
            .contains("Sql.Database")
    );
}
