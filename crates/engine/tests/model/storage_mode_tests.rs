//! Storage mode: `table.set_storage_mode` writes every partition's `mode`, refuses what Desktop
//! does not allow (calculated tables, sources with no connector, Import tables a PBIX stores),
//! warns about limited relationships and Dual on a fact table, sets the DirectQuery dimension
//! tables to Dual on request; the snapshot shows each table's mode; visuals say a DirectQuery
//! table is not evaluated; TMDL, TMSL and PBIX keep the mode.

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

const SQL: &str =
    "let Source = Sql.Database(\"srv.example\", \"db\"), T = Source{[Schema=\"dbo\",Item=\"T\"]}[Data] in T";

/// Sales (DirectQuery, SQL) many-to-one Product (DirectQuery, SQL) many-to-one Category
/// (DirectQuery, SQL); Notes (Import, #table); Calc (a calculated table).
fn composite() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    for (table, cols) in [
        ("Sales", &["ProductKey", "Amount"][..]),
        ("Product", &["ProductKey", "CategoryKey"][..]),
        ("Category", &["CategoryKey"][..]),
    ] {
        run(
            &mut s,
            "table.add",
            json!({"name": table, "partition": {"source_type": "m", "expression": SQL}}),
        );
        for c in cols {
            run(
                &mut s,
                "column.add",
                json!({"table": table, "name": c, "data_type": "int64"}),
            );
        }
        run(
            &mut s,
            "table.set_storage_mode",
            json!({"table": table, "mode": "directQuery"}),
        );
    }
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[ProductKey]", "to": "Product[ProductKey]"}),
    );
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Product[CategoryKey]", "to": "Category[CategoryKey]"}),
    );
    run(
        &mut s,
        "table.import",
        json!({"name": "Notes", "m": "#table(type table [Id = Int64.Type], {{1}, {2}})"}),
    );
    run(
        &mut s,
        "table.add",
        json!({"name": "Calc", "partition": {"source_type": "calculated", "expression": "{1, 2}"}}),
    );
    s
}

fn modes(s: &Session, table: &str) -> Vec<Option<String>> {
    s.model
        .as_ref()
        .unwrap()
        .tables
        .iter()
        .find(|t| t.name == table)
        .unwrap()
        .partitions
        .iter()
        .map(|p| p.mode.clone())
        .collect()
}

fn snap_mode(s: &Session, table: &str) -> Value {
    s.snapshot()["model"]["tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == table)
        .unwrap()["storage_mode"]
        .clone()
}

#[test]
fn set_storage_mode_writes_partition_modes_and_the_snapshot_shows_them() {
    let mut s = composite();
    assert_eq!(modes(&s, "Sales"), vec![Some("directQuery".to_string())]);
    assert_eq!(snap_mode(&s, "Sales"), "directQuery");
    assert_eq!(snap_mode(&s, "Notes"), "import");
    let out = run(
        &mut s,
        "table.set_storage_mode",
        json!({"table": "product", "mode": "DUAL"}),
    );
    assert_eq!(out["mode"], "dual");
    assert_eq!(out["previous_mode"], "directQuery");
    assert_eq!(snap_mode(&s, "Product"), "dual");
    // Dual on the many side of a relationship is meant for dimension tables
    let w = out["warnings"].to_string();
    assert!(w.contains("dimension tables"), "{out}");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(snap_mode(&s, "Product"), "directQuery");
}

#[test]
fn rules_refuse_what_desktop_does_not_allow() {
    let mut s = composite();
    for (p, want) in [
        (
            json!({"table": "Calc", "mode": "directQuery"}),
            "calculated table",
        ),
        (json!({"table": "Notes", "mode": "dual"}), "no data source"),
        (json!({"table": "Sales", "mode": "push"}), "unknown mode"),
        (json!({"table": "Nope", "mode": "import"}), "Nope"),
        (json!({"table": "Sales"}), "mode"),
        (json!({"table": "Sales", "mode": 3}), "mode"),
        (
            json!({"table": "Sales", "mode": "import", "extra": 1}),
            "unknown parameter",
        ),
    ] {
        let e = s.execute("table.set_storage_mode", &p).unwrap_err();
        assert!(e.contains(want), "{p}: {e}");
    }
    // a calculated table may be set to Import (it is already)
    run(
        &mut s,
        "table.set_storage_mode",
        json!({"table": "Calc", "mode": "import"}),
    );
}

#[test]
fn switching_to_import_reports_limited_relationships_and_offers_dual() {
    let mut s = composite();
    let out = run(
        &mut s,
        "table.set_storage_mode",
        json!({"table": "Sales", "mode": "import"}),
    );
    // Sales (Import) → Product (DirectQuery) is limited; Product and Category could be Dual
    assert_eq!(
        out["limited_relationships"],
        json!(["Sales[ProductKey] → Product[ProductKey]"])
    );
    assert_eq!(out["dual_candidates"], json!(["Product", "Category"]));
    assert!(out["warnings"].to_string().contains("limited"), "{out}");
    run(&mut s, "edit.undo", json!({}));
    // with dual_related, in one undo step
    let out = run(
        &mut s,
        "table.set_storage_mode",
        json!({"table": "Sales", "mode": "import", "dual_related": true}),
    );
    assert_eq!(out["set_to_dual"], json!(["Product", "Category"]));
    assert_eq!(out["limited_relationships"], json!([]));
    assert_eq!(snap_mode(&s, "Product"), "dual");
    assert_eq!(snap_mode(&s, "Category"), "dual");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(snap_mode(&s, "Category"), "directQuery");
    assert_eq!(snap_mode(&s, "Sales"), "directQuery");
}

#[test]
fn a_direct_query_table_is_not_evaluated_in_visuals() {
    let mut s = composite();
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
    );
    run(&mut s, "report.new", json!({}));
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let v = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "card", "x": 0, "y": 0, "width": 200, "height": 100,
               "fields": [{"role": "Values", "field": "[Total]"}]}),
    );
    let snap = s.snapshot();
    let visual = snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == v["visual"])
        .unwrap()
        .clone();
    assert_eq!(visual["data"]["status"], "unavailable", "{visual}");
    let reason = visual["data"]["reason"].as_str().unwrap();
    assert!(reason.contains("not evaluated: DirectQuery"), "{reason}");
    let e = s
        .execute("dax.evaluate", &json!({"query": "EVALUATE ROW(\"t\", [Total])"}))
        .unwrap_err();
    assert!(e.contains("not evaluated: DirectQuery"), "{e}");
    // the Import table still evaluates
    let ok = run(
        &mut s,
        "dax.evaluate",
        json!({"query": "EVALUATE ROW(\"n\", COUNTROWS(Notes))"}),
    );
    assert!(ok.to_string().contains('2'), "{ok}");
}

#[test]
fn storage_modes_are_saved_to_tmdl_tmsl_and_pbix() {
    let tmp = TempDir::new("engine-storage-mode-save");
    for (format, file) in [
        ("pbip-tmdl", "Tmdl.pbip"),
        ("pbip-tmsl", "Tmsl.pbip"),
        ("pbix", "shop.pbix"),
    ] {
        let mut s = composite();
        run(
            &mut s,
            "table.set_storage_mode",
            json!({"table": "Category", "mode": "dual"}),
        );
        let path = tmp.path().join(file);
        // the Dual table's SQL source is not run: written without rows (Desktop loads them)
        let mut p = json!({"path": path, "format": format});
        if format == "pbix" {
            p["empty_tables"] = json!(true);
        }
        run(&mut s, "file.save_as", p);
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        assert_eq!(back.model_error, None, "{format}");
        for t in ["Sales", "Product", "Category", "Notes"] {
            assert_eq!(modes(&back, t), modes(&s, t), "{format} {t}");
            assert_eq!(snap_mode(&back, t), snap_mode(&s, t), "{format} {t}");
        }
    }
}

#[test]
fn an_import_table_a_pbix_stores_is_not_turned_into_direct_query() {
    let tmp = TempDir::new("engine-storage-mode-pbix");
    let mut s = composite();
    run(
        &mut s,
        "table.add",
        json!({"name": "Live", "partition": {"source_type": "m", "expression": SQL}}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Live", "name": "Id", "data_type": "int64"}),
    );
    let path = tmp.path().join("shop.pbix");
    run(
        &mut s,
        "file.save_as",
        json!({"path": path, "empty_tables": true}),
    );
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": path}));
    assert_eq!(snap_mode(&back, "Live"), "import");
    let e = back
        .execute(
            "table.set_storage_mode",
            &json!({"table": "Live", "mode": "directQuery"}),
        )
        .unwrap_err();
    assert!(e.contains("PBIP"), "{e}");
}

#[test]
fn a_new_pbix_writes_direct_query_tables_without_rows() {
    let tmp = TempDir::new("engine-storage-mode-dq-pbix");
    // every SQL table is DirectQuery: no source needs to run, so no `empty_tables`
    let mut s = composite();
    let path = tmp.path().join("dq.pbix");
    let out = run(&mut s, "file.save_as", json!({"path": path}));
    assert_eq!(
        out["empty_tables"],
        json!(["Sales", "Product", "Category"]),
        "{out}"
    );
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": path}));
    assert_eq!(snap_mode(&back, "Sales"), "directQuery");
    let m = back.model.as_ref().unwrap();
    let sales = m.tables.iter().find(|t| t.name == "Sales").unwrap();
    // the query is kept for Desktop
    assert!(
        sales.partitions[0]
            .expression
            .as_deref()
            .unwrap()
            .contains("Sql.Database")
    );
}
