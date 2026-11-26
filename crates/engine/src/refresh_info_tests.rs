//! Partition refresh state: FILETIME conversion, a synthetic `metadata.sqlitedb`, and the
//! `refresh` blocks of `model.storage_stats`, `model.get` and `table.get` on PBIX files bicraft
//! builds and saves.

use bicraft_sqlite::{DatabaseBuilder, Value as Sql};
use serde_json::{Value, json};

use super::*;
use crate::tempdir::TempDir;

/// 2026-10-14T09:30:00Z as FILETIME ticks.
const T1: i64 = (1_791_970_200 + 11_644_473_600) * 10_000_000;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

#[test]
fn filetime_to_iso() {
    assert_eq!(filetime_iso(T1).as_deref(), Some("2026-10-14T09:30:00Z"));
    assert_eq!(
        filetime_iso(T1 + 1_234_567).as_deref(),
        Some("2026-10-14T09:30:00.123Z")
    );
    // 1970-01-01, 2000-02-29 (leap day), 1601-era and "never" values.
    assert_eq!(
        filetime_iso(116_444_736_000_000_000).as_deref(),
        Some("1970-01-01T00:00:00Z")
    );
    assert_eq!(
        filetime_iso(125_962_560_000_000_000).as_deref(),
        Some("2000-02-29T00:00:00Z")
    );
    assert_eq!(filetime_iso(NEVER), None);
    assert_eq!(filetime_iso(0), None);
    assert_eq!(filetime_iso(-5), None);
    assert_eq!(filetime_iso(i64::MAX), None);
    assert_eq!(filetime_iso(LATEST).as_deref(), Some("9999-12-31T23:59:59Z"));
    assert_eq!(state_name(1), "Ready");
    assert_eq!(state_name(4), "CalculationNeeded");
    assert_eq!(state_name(99), "Unknown");
}

/// A database with two user tables (three partitions in several states), a system table and
/// the change markers.
fn synthetic_db() -> Vec<u8> {
    let mut b = DatabaseBuilder::new(4096).unwrap();
    b.create_table(
        "DBPROPERTIES",
        "CREATE TABLE [DBPROPERTIES] ([NAME] STRING, [VALUE] INTEGER, PRIMARY KEY ([NAME] ASC))",
    )
    .unwrap();
    b.create_table(
        "Model",
        "CREATE TABLE [Model]( [ID] INTEGER , [Name] TEXT , [ModifiedTime] INTEGER , [StructureModifiedTime] INTEGER , PRIMARY KEY(\"ID\" ASC) )",
    )
    .unwrap();
    b.create_table(
        "Table",
        "CREATE TABLE [Table]( [ID] INTEGER , [Name] TEXT , [ModifiedTime] INTEGER , [StructureModifiedTime] INTEGER , [SystemFlags] INTEGER , PRIMARY KEY(\"ID\" ASC) )",
    )
    .unwrap();
    b.create_table(
        "Partition",
        "CREATE TABLE [Partition]( [ID] INTEGER , [TableID] INTEGER , [Name] TEXT , [State] INTEGER , [ModifiedTime] INTEGER , [RefreshedTime] INTEGER , [ErrorMessage] TEXT , PRIMARY KEY(\"ID\" ASC) )",
    )
    .unwrap();
    let t = |s: &str| Sql::Text(s.into());
    let i = Sql::Integer;
    b.insert("DBPROPERTIES", 1, vec![t("MAXID"), i(30)]).unwrap();
    b.insert("DBPROPERTIES", 7, vec![t("LASTCUBEDATACHANGE"), i(T1 + 50)])
        .unwrap();
    b.insert("DBPROPERTIES", 8, vec![t("LASTCUBESCHEMACHANGE"), i(T1)])
        .unwrap();
    b.insert(
        "Model",
        1,
        vec![i(1), t("Model"), i(T1 - 10_000_000), i(T1 - 20_000_000)],
    )
    .unwrap();
    b.insert("Table", 10, vec![i(10), t("Sales"), i(T1), i(T1), i(0)])
        .unwrap();
    b.insert("Table", 11, vec![i(11), t("Calc"), i(T1), i(T1), i(2)])
        .unwrap();
    b.insert("Table", 12, vec![i(12), t("H$Sales (5)"), i(T1), i(T1), i(1)])
        .unwrap();
    let p = |id: i64, table: i64, name: &str, state: i64, refreshed: i64, err: Sql| {
        vec![i(id), i(table), t(name), i(state), i(T1), i(refreshed), err]
    };
    b.insert("Partition", 20, p(20, 10, "Sales-2025", 1, T1, Sql::Null))
        .unwrap();
    b.insert("Partition", 21, p(21, 10, "Sales-2026", 3, NEVER, Sql::Null))
        .unwrap();
    b.insert(
        "Partition",
        22,
        p(22, 11, "Calc", 5, T1 - 600_000_000, t("bad expression")),
    )
    .unwrap();
    b.insert("Partition", 23, p(23, 12, "H$", 1, T1 + 99, Sql::Null))
        .unwrap();
    b.finish().unwrap()
}

#[test]
fn reads_partition_states_and_times() {
    let bytes = synthetic_db();
    let db = bicraft_sqlite::Database::from_bytes(&bytes).unwrap();
    let info = from_db(&db).unwrap();
    assert_eq!(info.tables.len(), 2, "system table left out");
    let sales = info.table("sales").unwrap();
    assert_eq!(sales.partitions.len(), 2);
    assert_eq!(sales.partition("Sales-2026").unwrap().refreshed, None);
    let v = info.json();
    assert_eq!(v["last_refresh"], "2026-10-14T09:30:00Z");
    assert_eq!(v["last_data_change"], "2026-10-14T09:30:00Z");
    assert_eq!(v["last_schema_change"], "2026-10-14T09:30:00Z");
    assert_eq!(v["modified_time"], "2026-10-14T09:29:59Z");
    assert_eq!(v["structure_modified_time"], "2026-10-14T09:29:58Z");
    assert_eq!(v["partition_count"], 3);
    assert_eq!(
        v["partitions_not_ready"],
        json!([
            {"table": "Sales", "partition": "Sales-2026", "state": "NoData"},
            {"table": "Calc", "partition": "Calc", "state": "SemanticError"},
        ])
    );

    let mut table = json!({"name": "Calc", "partitions": [{"name": "calc"}, {"name": "other"}]});
    annotate_table(&info, &mut table);
    assert_eq!(
        table["partitions"][0]["refresh"],
        json!({
            "state": "SemanticError", "state_code": 5,
            "refreshed_time": "2026-10-14T09:29:00Z",
            "modified_time": "2026-10-14T09:30:00Z",
            "error_message": "bad expression",
        })
    );
    assert!(table["partitions"][1].get("refresh").is_none());
    assert_eq!(table["refresh"]["last_refresh"], "2026-10-14T09:29:00Z");

    let mut model = json!({"tables": [{"name": "Unknown", "partitions": []}]});
    annotate_model(&info, &mut model);
    assert!(model["tables"][0].get("refresh").is_none());
    assert_eq!(model["refresh"]["partition_count"], 3);
}

#[test]
fn hostile_rows_read_as_absent() {
    let mut b = DatabaseBuilder::new(4096).unwrap();
    b.create_table(
        "Partition",
        "CREATE TABLE [Partition]( [ID] INTEGER , [TableID] INTEGER , [Name] TEXT , [State] INTEGER , [RefreshedTime] INTEGER , PRIMARY KEY(\"ID\" ASC) )",
    )
    .unwrap();
    b.create_table(
        "Table",
        "CREATE TABLE [Table]( [ID] INTEGER , [Name] TEXT , [SystemFlags] INTEGER , PRIMARY KEY(\"ID\" ASC) )",
    )
    .unwrap();
    b.insert(
        "Table",
        1,
        vec![Sql::Integer(1), Sql::Integer(7), Sql::Text("x".into())],
    )
    .unwrap();
    b.insert(
        "Partition",
        2,
        vec![
            Sql::Integer(2),
            Sql::Integer(1),
            Sql::Null,
            Sql::Text("1".into()),
            Sql::Integer(i64::MIN),
        ],
    )
    .unwrap();
    let bytes = b.finish().unwrap();
    let info = from_db(&bicraft_sqlite::Database::from_bytes(&bytes).unwrap()).unwrap();
    let p = &info.tables[0].partitions[0];
    assert_eq!((p.state, p.refreshed), (None, None));
    let v = info.json();
    assert_eq!(v["last_refresh"], Value::Null);
    assert_eq!(v["partitions_not_ready"][0]["state"], Value::Null);
}

fn iso(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("not a time: {v}"))
}

#[test]
fn pbix_saves_report_refresh_state() {
    let dir = TempDir::new("refresh-pbix");
    let path = dir.join("r.pbix");
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "R"}));
    let no_file = run(&mut s, "model.get", json!({"detail": "full"}));
    assert!(
        no_file.get("refresh").is_none(),
        "no backup before the first save"
    );
    run(
        &mut s,
        "table.import",
        json!({"name": "A", "m": "let Source = #table(type table [K = Int64.Type], {{1}, {2}}) in Source"}),
    );
    run(&mut s, "file.save_as", json!({"path": path}));

    // PBIX from scratch: every partition Ready, refreshed at save time.
    let stats = run(&mut s, "model.storage_stats", json!({"columns": false}));
    let p = &stats["tables"][0]["partitions"][0];
    assert_eq!(p["state"], "Ready", "{p}");
    let first = iso(&p["refreshed_time"]).to_string();
    assert!(first.starts_with("20") && first.ends_with('Z'), "{first}");
    assert!(p["modified_time"].is_string());
    assert_eq!(stats["refresh"]["last_refresh"], first.as_str());
    assert_eq!(stats["refresh"]["partitions_not_ready"], json!([]));
    assert_eq!(stats["refresh"]["source"], "DataModel");

    let full = run(&mut s, "model.get", json!({"detail": "full"}));
    assert_eq!(full["refresh"]["last_refresh"], first.as_str());
    assert_eq!(full["tables"][0]["partitions"][0]["refresh"]["state"], "Ready");
    let t = run(&mut s, "table.get", json!({"name": "A"}));
    assert_eq!(t["partitions"][0]["refresh"]["refreshed_time"], first.as_str());
    assert_eq!(t["refresh"]["last_refresh"], first.as_str());
    let summary = run(&mut s, "model.get", json!({}));
    assert!(summary.get("refresh").is_none());

    // Metadata-only save: the stored refresh time is kept.
    std::thread::sleep(std::time::Duration::from_millis(20));
    run(
        &mut s,
        "measure.add",
        json!({"table": "A", "name": "N", "expression": "COUNTROWS(A)"}),
    );
    run(&mut s, "file.save", json!({}));
    let t = run(&mut s, "table.get", json!({"name": "A"}));
    assert_eq!(t["partitions"][0]["refresh"]["refreshed_time"], first.as_str());

    // Structural save: the new table's partition is refreshed now, the old one keeps its time.
    run(
        &mut s,
        "table.import",
        json!({"name": "B", "m": "let Source = #table(type table [V = text], {{\"x\"}}) in Source"}),
    );
    run(&mut s, "file.save", json!({}));
    let full = run(&mut s, "model.get", json!({"detail": "full"}));
    let a = &full["tables"][0];
    let b = &full["tables"][1];
    assert_eq!(a["name"], "A");
    assert_eq!(a["partitions"][0]["refresh"]["refreshed_time"], first.as_str());
    let second = iso(&b["partitions"][0]["refresh"]["refreshed_time"]);
    assert!(second >= first.as_str(), "{second} < {first}");
    assert_eq!(full["refresh"]["last_refresh"], second);

    // A PBIP project keeps the data in .pbi/cache.abf: same values from there.
    run(
        &mut s,
        "file.save_as",
        json!({"path": dir.join("proj").join("R.pbip")}),
    );
    let full = run(&mut s, "model.get", json!({"detail": "full"}));
    assert_eq!(full["refresh"]["source"], ".pbi/cache.abf");
    assert_eq!(
        full["tables"][0]["partitions"][0]["refresh"]["refreshed_time"],
        first.as_str()
    );

    // A PBIT holds no data: no refresh blocks.
    let pbit = dir.join("r.pbit");
    run(&mut s, "file.save_as", json!({"path": pbit}));
    let mut t = Session::new();
    run(&mut t, "file.open", json!({"path": pbit}));
    let full = run(&mut t, "model.get", json!({"detail": "full"}));
    assert!(full.get("refresh").is_none());
    assert!(full["tables"][0]["partitions"][0].get("refresh").is_none());
}

#[test]
fn filetime_to_dax_serial() {
    // 2026-10-14T09:30Z: serial 46309 plus 9.5 hours
    let x = filetime_serial(T1).unwrap();
    assert!((x - (46_309.0 + 9.5 / 24.0)).abs() < 1e-9, "{x}");
    // 1899-12-30 is serial 0, 1970-01-01 is 25569
    assert_eq!(filetime_serial(116_444_736_000_000_000), Some(25_569.0));
    assert_eq!(filetime_serial(NEVER), None);
    assert_eq!(filetime_serial(0), None);
    assert_eq!(filetime_serial(i64::MAX), None);
}
