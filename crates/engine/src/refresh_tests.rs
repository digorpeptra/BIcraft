//! `model.refresh` over a fake Databricks warehouse (`testsupport/fake_databricks.rs`): the
//! snapshot it writes, the DAX evaluator reading it, staleness, partial refreshes, refusals.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::*;
use crate::tempdir::TempDir;

#[path = "../../../testsupport/fake_databricks.rs"]
mod fake_databricks;
use fake_databricks::{FakeServer, FakeTable, Warehouse};

const HOST: &str = "dbc-perso.cloud.databricks.com";

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn warehouse() -> Warehouse {
    let mut tables = std::collections::HashMap::new();
    tables.insert(
        "main.comptes.v_transactions".to_string(),
        FakeTable::new(
            &[
                ("id", "STRING", "string"),
                ("date", "DATE", "date"),
                ("montant", "DECIMAL", "decimal(12,2)"),
                ("categorie_id", "STRING", "string"),
            ],
            vec![
                vec![json!("t1"), json!("2026-09-01"), json!("-42.10"), json!("c1")],
                vec![json!("t2"), json!("2026-09-03"), json!("2500.00"), json!("c2")],
                vec![json!("t3"), json!("2026-10-02"), json!("-7.90"), json!("c1")],
            ],
        ),
    );
    tables.insert(
        "main.comptes.dim_categories".to_string(),
        FakeTable::new(
            &[("id", "STRING", "string"), ("nom", "STRING", "string")],
            vec![
                vec![json!("c1"), json!("Courses")],
                vec![json!("c2"), json!("Salaire")],
            ],
        ),
    );
    Warehouse {
        token: "engine-test-token-0000".into(),
        warehouse_id: "0123abcd".into(),
        tables,
        ..Warehouse::default()
    }
}

fn partition(table: &str, columns: &[&str], types: &str) -> String {
    let cols = columns
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "let\n    Source = DatabricksMultiCloud.Catalogs(DatabricksHost, DatabricksHttpPath, [Catalog = null, Database = null, QueryTags = null, EnableAutomaticProxyDiscovery = null, Implementation = \"2.0\", Protocol = null]),\n    \
         Catalogue = Source{{[Name = DatabricksCatalog, Kind = \"Database\"]}}[Data],\n    \
         Schema = Catalogue{{[Name = DatabricksSchema, Kind = \"Schema\"]}}[Data],\n    \
         TableSource = Table.SelectRows(Schema, each [Name] = \"{table}\"){{0}}[Data],\n    \
         Colonnes = Table.SelectColumns(TableSource, {{{cols}}}),\n    \
         Types = Table.TransformColumnTypes(Colonnes, {{{types}}}),\n    \
         Limite = if RowLimit = null then Types else Table.FirstN(Types, RowLimit)\nin\n    Limite"
    )
}

/// A typical refreshed-project shape: two Databricks tables, a refresh-time table, a calculated
/// table, a calculated column and measures, saved as a PBIP project in `dir`.
fn project(dir: &TempDir) -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Perso", "culture": "fr-FR"}));
    for (name, value) in [
        ("DatabricksHost", format!("\"{HOST}\"")),
        ("DatabricksHttpPath", "\"/sql/1.0/warehouses/0123abcd\"".into()),
        ("DatabricksCatalog", "\"main\"".into()),
        ("DatabricksSchema", "\"comptes\"".into()),
    ] {
        run(
            &mut s,
            "expression.add",
            json!({"name": name, "expression": format!("{value} meta [IsParameterQuery=true, Type=\"Text\", IsParameterQueryRequired=true]")}),
        );
    }
    run(
        &mut s,
        "expression.add",
        json!({"name": "RowLimit", "expression": "null meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=false]"}),
    );
    run(
        &mut s,
        "table.add",
        json!({"name": "Transactions", "partition": {"source_type": "m", "expression": partition(
            "v_transactions",
            &["id", "date", "montant", "categorie_id"],
            "{\"id\", type text}, {\"date\", type date}, {\"montant\", Currency.Type}, {\"categorie_id\", type text}",
        )}}),
    );
    for (name, ty, src) in [
        ("id", "string", "id"),
        ("Date", "dateTime", "date"),
        ("montant", "decimal", "montant"),
        ("categorie_id", "string", "categorie_id"),
    ] {
        run(
            &mut s,
            "column.add",
            json!({"table": "Transactions", "name": name, "data_type": ty, "source_column": src}),
        );
    }
    run(
        &mut s,
        "column.add",
        json!({"table": "Transactions", "name": "Mois", "data_type": "int64", "expression": "MONTH(Transactions[Date])"}),
    );
    run(
        &mut s,
        "table.add",
        json!({"name": "Catégorie", "partition": {"source_type": "m", "expression": partition(
            "dim_categories", &["id", "nom"], "{\"id\", type text}, {\"nom\", type text}")}}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Catégorie", "name": "id", "data_type": "string"}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Catégorie", "name": "Catégorie", "data_type": "string", "source_column": "nom"}),
    );
    run(
        &mut s,
        "table.add",
        json!({"name": "_Actualisation", "partition": {"source_type": "m", "expression":
            "let N = DateTimeZone.RemoveZone(DateTimeZone.UtcNow()) in #table(type table [#\"Actualisé le\" = datetime], {{N}})"}}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "_Actualisation", "name": "Actualisé le", "data_type": "dateTime"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Transactions", "name": "Dépenses", "expression": "-CALCULATE(SUM(Transactions[montant]), Transactions[montant] < 0)"}),
    );
    run(
        &mut s,
        "table.add",
        json!({"name": "Résumé", "partition": {"source_type": "calculated", "expression":
            "ROW(\"Lignes\", COUNTROWS(Transactions), \"Total\", SUM(Transactions[montant]))"}}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Résumé", "name": "Lignes", "data_type": "int64", "source_column": "[Lignes]"}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Résumé", "name": "Total", "data_type": "decimal", "source_column": "[Total]"}),
    );
    run(&mut s, "project.new", json!({"dir": dir.path(), "name": "Perso"}));
    s
}

fn connect(s: &mut Session, server: &FakeServer, token: &'static str) {
    s.test_endpoint = Some(TestEndpoint {
        host: HOST.into(),
        base: server.base.clone(),
        settings: bicraft_databricks::Settings {
            poll_initial: Duration::from_millis(5),
            poll_max: Duration::from_millis(20),
            retry_delay: Duration::from_millis(1),
            ..bicraft_databricks::Settings::default()
        },
    });
    s.token_source = Some(TokenSource(Arc::new(move || {
        bicraft_databricks::Token::new(token).ok_or_else(|| "no token".to_string())
    })));
}

fn snapshot_file(s: &Session) -> std::path::PathBuf {
    match &s.source {
        Some(Source::Project { project, .. }) => snapshot_path(project).unwrap(),
        other => panic!("{other:?}"),
    }
}

fn eval(s: &mut Session, query: &str) -> Value {
    run(s, "dax.evaluate", json!({"query": query}))
}

#[test]
fn refresh_writes_a_snapshot_the_dax_evaluator_reads() {
    let dir = TempDir::new("engine-refresh-main");
    let server = FakeServer::start(Warehouse {
        pending_polls: 2,
        chunk_rows: 2,
        ..warehouse()
    });
    let mut s = project(&dir);
    connect(&mut s, &server, "engine-test-token-0000");
    let r = run(
        &mut s,
        "model.refresh",
        json!({"allow_connectors": ["databricks"]}),
    );
    let tables: Vec<(String, String, u64)> = r["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["table"].as_str().unwrap().to_string(),
                t["origin"].as_str().unwrap().to_string(),
                t["rows"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        tables,
        [
            ("Transactions".to_string(), "m".to_string(), 3),
            ("Catégorie".into(), "m".into(), 2),
            ("_Actualisation".into(), "m".into(), 1),
            ("Résumé".into(), "dax".into(), 1),
        ]
    );
    assert_eq!(r["not_computed"], json!([]), "{r}");
    // project.new wrote a .gitignore that ignores the snapshot
    assert_eq!(r["warnings"], json!([]), "{r}");
    assert_eq!(r["connectors"], json!(["databricks"]));
    let sql: Vec<&str> = r["tables"][0]["statements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|st| st["sql"].as_str().unwrap())
        .collect();
    assert_eq!(sql.len(), 2, "{sql:?}");
    assert_eq!(
        sql[1],
        "SELECT `id`, `date`, `montant`, `categorie_id` FROM `main`.`comptes`.`v_transactions`"
    );
    let path = snapshot_file(&s);
    assert!(path.is_file());
    assert!(
        !path.with_file_name("cache.abf").exists(),
        "cache.abf is never written"
    );
    let text = r.to_string();
    assert!(!text.contains("engine-test-token-0000"));

    let q = eval(
        &mut s,
        "EVALUATE ROW(\"d\", [Dépenses], \"n\", COUNTROWS('Résumé'), \"t\", MAX('Résumé'[Total]), \"m\", MAX(Transactions[Mois]), \"c\", COUNTROWS('Catégorie'))",
    );
    assert_eq!(q["data_source"], "bicraft-data");
    let row = &q["results"][0]["rows"][0];
    assert_eq!(row["[d]"], json!(50.0), "{q}");
    assert_eq!(row["[n]"], json!(1));
    assert_eq!(row["[t]"], json!(2450.0));
    assert_eq!(row["[m]"], json!(10));
    assert_eq!(row["[c]"], json!(2));

    // Another session reads the snapshot without any network access.
    drop(server);
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": dir.join("Perso.pbip")}));
    let q = eval(
        &mut again,
        "EVALUATE ROW(\"d\", [Dépenses], \"a\", COUNTROWS('_Actualisation'))",
    );
    assert_eq!(q["data_source"], "bicraft-data");
    assert_eq!(q["results"][0]["rows"][0]["[d]"], json!(50.0));
    assert_eq!(q["results"][0]["rows"][0]["[a]"], json!(1));

    // A changed query makes its table stale: it is not read from the snapshot any more.
    let expr = partition("v_transactions", &["id", "date", "montant", "categorie_id"], "{\"id\", type text}, {\"date\", type date}, {\"montant\", Currency.Type}, {\"categorie_id\", type text}")
        .replace("Limite = if", "Limite2 = 1,\n    Limite = if");
    run(
        &mut again,
        "partition.set",
        json!({"table": "Transactions", "expression": expr}),
    );
    let e = again
        .execute(
            "dax.evaluate",
            &json!({"query": "EVALUATE ROW(\"d\", [Dépenses])"}),
        )
        .unwrap_err();
    assert!(
        e.contains("external source") || e.contains("not available"),
        "{e}"
    );
    let q = eval(&mut again, "EVALUATE ROW(\"c\", COUNTROWS('Catégorie'))");
    assert_eq!(q["results"][0]["rows"][0]["[c]"], json!(2));
}

#[test]
fn partial_refresh_keeps_the_other_tables() {
    let dir = TempDir::new("engine-refresh-partial");
    let server = FakeServer::start(warehouse());
    let mut s = project(&dir);
    connect(&mut s, &server, "engine-test-token-0000");
    run(
        &mut s,
        "model.refresh",
        json!({"allow_connectors": ["databricks"]}),
    );
    let before = server.statements().len();
    let r = run(
        &mut s,
        "model.refresh",
        json!({"tables": ["Catégorie"], "allow_connectors": "databricks"}),
    );
    assert_eq!(r["tables"].as_array().unwrap().len(), 1);
    assert_eq!(r["kept"], json!(3), "{r}");
    // the schema listing and one SELECT
    assert_eq!(server.statements().len(), before + 2);
    let q = eval(
        &mut s,
        "EVALUATE ROW(\"d\", [Dépenses], \"c\", COUNTROWS('Catégorie'))",
    );
    assert_eq!(q["results"][0]["rows"][0]["[d]"], json!(50.0));
}

#[test]
fn refresh_refusals_write_nothing() {
    let dir = TempDir::new("engine-refresh-refused");
    let server = FakeServer::start(warehouse());
    let mut s = project(&dir);
    connect(&mut s, &server, "engine-test-token-0000");
    // Without the connector allowed: the external source is refused, with the hint.
    let e = s.execute("model.refresh", &json!({})).unwrap_err();
    assert!(
        e.contains("external source") && e.contains("allow_connectors"),
        "{e}"
    );
    assert!(server.requests().is_empty());
    // A rejected token: nothing written, the token never shown.
    connect(&mut s, &server, "a-wrong-token-9999");
    let e = s
        .execute("model.refresh", &json!({"allow_connectors": ["databricks"]}))
        .unwrap_err();
    assert!(e.contains("HTTP 401") && !e.contains("a-wrong-token-9999"), "{e}");
    assert!(!snapshot_file(&s).exists());
    // Unknown connector, unknown table, not a project.
    assert!(
        s.execute("model.refresh", &json!({"allow_connectors": ["snowflake"]}))
            .unwrap_err()
            .contains("unknown connector")
    );
    assert!(
        s.execute("model.refresh", &json!({"tables": ["Nope"]}))
            .unwrap_err()
            .contains("not in the model")
    );
    let mut m = Session::new();
    run(&mut m, "model.new", json!({"name": "X"}));
    assert!(
        m.execute("model.refresh", &json!({}))
            .unwrap_err()
            .contains("PBIP")
    );
}

#[test]
fn a_newer_cache_abf_wins_over_the_snapshot() {
    let dir = TempDir::new("engine-refresh-abf");
    let server = FakeServer::start(warehouse());
    let mut s = project(&dir);
    connect(&mut s, &server, "engine-test-token-0000");
    run(
        &mut s,
        "model.refresh",
        json!({"allow_connectors": ["databricks"]}),
    );
    assert!(active_snapshot(&s).is_some());
    // Desktop refreshed afterwards (simulated with an empty file newer than the snapshot; it is
    // never read by this check).
    let abf = snapshot_file(&s).with_file_name("cache.abf");
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(&abf, b"").unwrap();
    let later = std::time::SystemTime::now() + Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&abf)
        .unwrap()
        .set_modified(later)
        .unwrap();
    assert!(active_snapshot(&s).is_none());
    std::fs::remove_file(&abf).unwrap();
    assert!(active_snapshot(&s).is_some());
    // An explicit cache_abf wins too.
    let other = TempDir::new("engine-refresh-abf-explicit");
    let explicit = other.join("cache.abf");
    std::fs::write(&explicit, b"").unwrap();
    s.cache_abf = Some(explicit);
    assert!(active_snapshot(&s).is_none());
}

#[test]
fn fingerprints_follow_the_definition() {
    let dir = TempDir::new("engine-refresh-fp");
    let s = project(&dir);
    let m = s.model.as_ref().unwrap();
    let t = m.tables.iter().find(|t| t.name == "Transactions").unwrap();
    let a = table_fingerprint(m, t);
    assert_eq!(a, table_fingerprint(m, t));
    let mut m2 = m.clone();
    if let Some(e) = m2.expressions.iter_mut().find(|e| e.name == "DatabricksSchema") {
        e.expression = "\"autre\" meta [IsParameterQuery=true]".into();
    }
    let t2 = m2.tables.iter().find(|t| t.name == "Transactions").unwrap();
    assert_ne!(a, table_fingerprint(&m2, t2), "a referenced parameter changes it");
    let c = m.tables.iter().find(|t| t.name == "Catégorie").unwrap();
    let mut m3 = m.clone();
    if let Some(e) = m3.expressions.iter_mut().find(|e| e.name == "RowLimit") {
        e.description = Some("doc".into());
    }
    assert_eq!(
        table_fingerprint(m, c),
        table_fingerprint(&m3, c),
        "descriptions do not"
    );
}
