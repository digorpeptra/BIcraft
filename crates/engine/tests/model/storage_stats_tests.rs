//! `model.storage_stats` and `model.export_vpax` on PBIX files bicraft builds itself.

use std::collections::HashSet;
use std::path::Path;

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Two related tables, one with a user hierarchy, saved as a PBIX at `path`.
fn build(path: &Path) -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Stats"}));
    let rows: Vec<String> = (0..500)
        .map(|i| {
            format!(
                "{{{i}, \"{}\", {}, {}}}",
                ["red", "green", "blue"][i % 3],
                i % 7,
                (i as f64) * 1.25
            )
        })
        .collect();
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "m": format!(
            "let Source = #table(type table [Id = Int64.Type, Color = text, Product = Int64.Type, Amount = number], {{{}}}) in Source",
            rows.join(", ")
        )}),
    );
    run(
        &mut s,
        "table.import",
        json!({"name": "Product", "m": "let Source = #table(type table [Product = Int64.Type, Name = text, Group = text], {{0, \"a\", \"x\"}, {1, \"b\", \"x\"}, {2, \"c\", \"y\"}, {3, \"d\", \"y\"}, {4, \"e\", \"z\"}, {5, \"f\", \"z\"}, {6, \"g\", \"z\"}}) in Source"}),
    );
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Product]", "to": "Product[Product]"}),
    );
    run(
        &mut s,
        "hierarchy.add",
        json!({"table": "Product", "name": "Groups", "levels": ["Group", "Name"]}),
    );
    run(&mut s, "file.save_as", json!({"path": path}));
    s
}

fn table<'a>(v: &'a Value, name: &str) -> &'a Value {
    v["tables"]
        .as_array()
        .and_then(|a| a.iter().find(|t| t["name"] == name))
        .unwrap_or_else(|| panic!("no table {name} in {v}"))
}

fn column<'a>(t: &'a Value, name: &str) -> &'a Value {
    t["columns"]
        .as_array()
        .and_then(|a| a.iter().find(|c| c["name"] == name))
        .unwrap_or_else(|| panic!("no column {name} in {t}"))
}

#[test]
fn storage_stats_of_a_built_pbix() {
    let dir = TempDir::new("stats-pbix");
    let path = dir.join("stats.pbix");
    let mut s = build(&path);
    let v = run(&mut s, "model.storage_stats", json!({"segments": true}));
    assert_eq!(v["source"], "DataModel");
    assert_eq!(v["streamed"], false);
    assert_eq!(v["table_count"], 2);
    let sales = table(&v, "Sales");
    assert_eq!(sales["rows"], 500);
    assert_eq!(sales["partitions"][0]["rows"], 500);
    let id = column(sales, "Id");
    assert_eq!(id["cardinality"], 500);
    assert_eq!(id["data_type"], "Int64");
    let color = column(sales, "Color");
    assert_eq!(color["cardinality"], 3);
    assert_eq!(color["encoding"], "hash");
    assert!(color["dictionary_size"].as_u64().unwrap() > 0);
    assert!(color["hierarchy_size"].as_u64().unwrap() > 0, "{color}");
    assert_eq!(column(sales, "Product")["cardinality"], 7);
    for c in sales["columns"].as_array().unwrap() {
        assert_eq!(c["is_row_number"], false);
        let total = c["total_size"].as_u64().unwrap();
        let parts = c["dictionary_size"].as_u64().unwrap()
            + c["data_size"].as_u64().unwrap()
            + c["hierarchy_size"].as_u64().unwrap();
        assert_eq!(total, parts, "{c}");
        let seg_rows: u64 = c["segments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["rows"].as_u64().unwrap())
            .sum();
        assert_eq!(seg_rows, 500, "{c}");
        assert!(c["data_size"].as_u64().unwrap() > 0, "{c}");
    }
    let product = table(&v, "Product");
    assert_eq!(product["rows"], 7);
    assert_eq!(product["user_hierarchies"][0]["name"], "Groups");
    assert!(
        product["user_hierarchies"][0]["size"].as_u64().unwrap() > 0,
        "{product}"
    );
    assert_eq!(v["relationships"].as_array().unwrap().len(), 1);
    let rel = &v["relationships"][0];
    assert_eq!(rel["from"], "Sales[Product]");
    assert!(rel["size_from"].as_u64().unwrap() > 0, "{rel}");
    assert_eq!(sales["relationships_size"], rel["size_from"]);
    let total: u64 = v["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["total_size"].as_u64().unwrap())
        .sum();
    assert_eq!(v["total_size"].as_u64().unwrap(), total);

    // Filters and options.
    let only = run(
        &mut s,
        "model.storage_stats",
        json!({"table": "product", "columns": false}),
    );
    assert_eq!(only["tables"].as_array().unwrap().len(), 1);
    assert!(only["tables"][0].get("columns").is_none());
    let with_rn = run(
        &mut s,
        "model.storage_stats",
        json!({"table": "Sales", "row_number": true}),
    );
    assert!(
        with_rn["tables"][0]["columns"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["is_row_number"] == true)
    );
    assert!(
        s.execute("model.storage_stats", &json!({"table": "Nope"}))
            .is_err()
    );
    assert!(s.execute("model.storage_stats", &json!({"bogus": 1})).is_err());
}

#[test]
fn storage_stats_of_a_project_cache_and_refusals() {
    let dir = TempDir::new("stats-pbip");
    let path = dir.join("stats.pbix");
    let mut s = build(&path);
    let from_pbix = run(&mut s, "model.storage_stats", json!({}));
    run(
        &mut s,
        "file.save_as",
        json!({"path": dir.join("proj").join("Stats.pbip")}),
    );
    let from_cache = run(&mut s, "model.storage_stats", json!({}));
    assert_eq!(from_cache["source"], ".pbi/cache.abf");
    assert_eq!(table(&from_cache, "Sales")["rows"], 500);
    assert_eq!(
        table(&from_cache, "Sales")["columns"],
        table(&from_pbix, "Sales")["columns"]
    );

    let mut fresh = Session::new();
    assert!(fresh.execute("model.storage_stats", &json!({})).is_err());
    run(&mut fresh, "model.new", json!({"name": "N"}));
    assert!(fresh.execute("model.storage_stats", &json!({})).is_err());
}

/// Every `$ref` of a Json.NET document points to an `$id` written before it.
fn check_refs(v: &Value, ids: &mut HashSet<String>) {
    match v {
        Value::Object(o) => {
            if let Some(id) = o.get("$id").and_then(Value::as_str) {
                assert!(ids.insert(id.to_string()), "duplicate $id {id}");
            }
            if let Some(r) = o.get("$ref").and_then(Value::as_str) {
                assert!(ids.contains(r), "$ref {r} before its $id");
                assert_eq!(o.len(), 1);
            }
            for x in o.values() {
                check_refs(x, ids);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| check_refs(x, ids)),
        _ => {}
    }
}

#[test]
fn vpax_export() {
    let dir = TempDir::new("stats-vpax");
    let path = dir.join("stats.pbix");
    let mut s = build(&path);
    let out = dir.join("stats.vpax");
    let r = run(&mut s, "model.export_vpax", json!({"path": out}));
    assert_eq!(r["model_bim"], true);
    assert!(
        s.execute("model.export_vpax", &json!({"path": out})).is_err(),
        "exists"
    );
    run(
        &mut s,
        "model.export_vpax",
        json!({"path": out, "overwrite": true}),
    );

    let pkg = bicraft_opc::Package::open(&out).unwrap();
    let names: Vec<&str> = pkg.parts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["[Content_Types].xml", "DaxModel.json", "Model.bim"]);
    let dax: Value = serde_json::from_slice(&pkg.get("DaxModel.json").unwrap().data).unwrap();
    check_refs(&dax, &mut HashSet::new());
    assert_eq!(dax["ModelName"], "stats", "the file name when the model has none");
    assert_eq!(dax["ExtractorApp"], "bicraft");
    let tables = dax["Tables"]["$values"].as_array().unwrap();
    let sales = tables.iter().find(|t| t["TableName"] == "Sales").unwrap();
    assert_eq!(sales["RowsCount"], 500);
    let cols = sales["Columns"]["$values"].as_array().unwrap();
    assert_eq!(cols[0]["IsRowNumber"], true);
    let color = cols.iter().find(|c| c["ColumnName"] == "Color").unwrap();
    assert_eq!(color["ColumnCardinality"], 3);
    assert_eq!(color["Encoding"], "HASH");
    assert_eq!(color["DataType"], "String");
    let stats = run(&mut s, "model.storage_stats", json!({"table": "Sales"}));
    let c = column(&stats["tables"][0], "Color");
    assert_eq!(color["DictionarySize"], c["dictionary_size"]);
    assert_eq!(color["DataSize"], c["data_size"]);
    let segs = color["ColumnSegments"]["$values"].as_array().unwrap();
    assert!(segs[0]["Partition"].is_object());
    let rel = &dax["Relationships"]["$values"][0];
    assert!(rel["FromColumn"]["$ref"].is_string());
    assert_eq!(rel["FromCardinalityType"], "Many");
    assert!(rel["UsedSizeFrom"].as_u64().unwrap() > 0);
    let product = tables.iter().find(|t| t["TableName"] == "Product").unwrap();
    let h = &product["UserHierarchies"]["$values"][0];
    assert_eq!(h["HierarchyName"], "Groups");
    assert_eq!(h["Levels"]["$values"].as_array().unwrap().len(), 2);
    let bim: Value = serde_json::from_slice(&pkg.get("Model.bim").unwrap().data).unwrap();
    assert_eq!(bim["model"]["tables"].as_array().unwrap().len(), 2);

    let no_bim = dir.join("nobim.vpax");
    run(
        &mut s,
        "model.export_vpax",
        json!({"path": no_bim, "model_bim": false}),
    );
    let pkg = bicraft_opc::Package::open(&no_bim).unwrap();
    assert!(pkg.get("Model.bim").is_none());
}

/// DAX Studio 3.6's "Import Metrics" dereferences every description and measure display
/// folder and parses every measure `DataType` (a missing one failed the import with a
/// NullReferenceException / ArgumentNullException); the row-number column is the key with
/// cardinality 0, as VertiPaq Analyzer's extractor writes it.
#[test]
fn vpax_fields_dax_studio_needs() {
    let dir = TempDir::new("stats-vpax-ds");
    let path = dir.join("stats.pbix");
    let mut s = build(&path);
    for (name, expr) in [
        ("Total", "SUM(Sales[Amount])"),
        ("Rows", "COUNTROWS(Sales)"),
        ("Label", "\"x\""),
        ("Nothing", "BLANK()"),
    ] {
        run(
            &mut s,
            "measure.add",
            json!({"table": "Sales", "name": name, "expression": expr}),
        );
    }
    run(
        &mut s,
        "column.set",
        json!({"table": "Product", "name": "Product", "is_key": true}),
    );
    let out = dir.join("ds.vpax");
    run(&mut s, "model.export_vpax", json!({"path": out}));
    let pkg = bicraft_opc::Package::open(&out).unwrap();
    let dax: Value = serde_json::from_slice(&pkg.get("DaxModel.json").unwrap().data).unwrap();
    for t in dax["Tables"]["$values"].as_array().unwrap() {
        assert!(t["Description"].is_string(), "{}", t["TableName"]);
        for c in t["Columns"]["$values"].as_array().unwrap() {
            for k in ["Description", "DisplayFolder", "FormatString", "EncodingHint"] {
                assert!(c[k].is_string(), "{} {k}", c["ColumnName"]);
            }
        }
        for m in t["Measures"]["$values"].as_array().unwrap() {
            for k in ["Description", "DisplayFolder", "FormatString", "DataType"] {
                assert!(m[k].is_string(), "{} {k}", m["MeasureName"]);
            }
        }
        let rn = &t["Columns"]["$values"][0];
        assert_eq!(rn["IsRowNumber"], true);
        assert!(rn.get("ColumnCardinality").is_none());
        assert_eq!(rn["IsUnique"], true);
        assert_eq!(rn["IsKey"], t["TableName"] == "Sales", "a user key replaces it");
    }
    let sales = &dax["Tables"]["$values"][0];
    let types: Vec<(&str, &str)> = sales["Measures"]["$values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["MeasureName"].as_str().unwrap(),
                m["DataType"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        types,
        [
            ("Total", "Double"),
            ("Rows", "Int64"),
            ("Label", "String"),
            ("Nothing", "Int64")
        ]
    );
}
