//! The connector against a local fake warehouse (`testsupport/fake_databricks.rs`), through the
//! M evaluator: the query Power BI Desktop writes, polling, chunks, folding, types, errors.

#[path = "../../../testsupport/fake_databricks.rs"]
mod fake_databricks;

use std::sync::Arc;
use std::time::Duration;

use bicraft_data::connect::Connectors;
use bicraft_data::{CellValue, DataType, Error, SourceOptions, TableData, m::load_m_with};
use bicraft_databricks::{DatabricksConnector, Settings, Token};
use fake_databricks::{FakeServer, FakeTable, Warehouse};
use serde_json::json;

const HOST: &str = "dbc-test.cloud.databricks.com";
const TOKEN: &str = "test-token-not-a-secret-1234";

fn transactions() -> FakeTable {
    FakeTable::new(
        &[
            ("id", "STRING", "string"),
            ("date", "DATE", "date"),
            ("montant", "DECIMAL", "decimal(18,2)"),
            ("statut", "STRING", "string"),
            ("annee", "INT", "int"),
            ("date_chargement", "TIMESTAMP", "timestamp"),
            ("ratio", "DOUBLE", "double"),
            ("ok", "BOOLEAN", "boolean"),
        ],
        vec![
            vec![
                json!("t1"),
                json!("2026-01-05"),
                json!("-12.50"),
                json!("ok"),
                json!("2026"),
                json!("2026-01-05T10:00:00.000Z"),
                json!("0.25"),
                json!("true"),
            ],
            vec![
                json!("t2"),
                json!("2026-02-01"),
                json!("1200.00"),
                json!(null),
                json!("2026"),
                json!(null),
                json!("NaN"),
                json!("false"),
            ],
            vec![
                json!("t3"),
                json!("2025-12-31"),
                json!("0.01"),
                json!("rejet"),
                json!("2025"),
                json!("2025-12-31T23:59:59.999Z"),
                json!(null),
                json!(null),
            ],
        ],
    )
}

fn warehouse() -> Warehouse {
    let mut tables = std::collections::HashMap::new();
    tables.insert("main.comptes.v_transactions".to_string(), {
        let mut t = transactions();
        t.kind = "VIEW".into();
        t
    });
    tables.insert(
        "main.comptes.dim_categories".to_string(),
        FakeTable::new(
            &[("id", "STRING", "string"), ("nom", "STRING", "string")],
            vec![vec![json!("c1"), json!("Courses")]],
        ),
    );
    Warehouse {
        token: TOKEN.into(),
        warehouse_id: "abc123".into(),
        tables,
        ..Warehouse::default()
    }
}

fn settings() -> Settings {
    Settings {
        poll_initial: Duration::from_millis(5),
        poll_max: Duration::from_millis(20),
        retry_delay: Duration::from_millis(1),
        max_wait: Duration::from_secs(10),
        ..Settings::default()
    }
}

fn connector(server: &FakeServer, token: Option<&str>, settings: Settings) -> Arc<DatabricksConnector> {
    let token = token.map(str::to_string);
    let provider = Arc::new(move || {
        token
            .as_deref()
            .and_then(Token::new)
            .ok_or_else(|| "no Databricks token: set BICRAFT_DATABRICKS_TOKEN".to_string())
    });
    Arc::new(
        DatabricksConnector::new(provider, settings)
            .with_test_endpoint(HOST, &server.base)
            .unwrap(),
    )
}

fn options(c: &Arc<DatabricksConnector>) -> SourceOptions {
    let mut connectors = Connectors::new();
    connectors.push(c.clone());
    SourceOptions {
        connectors,
        culture: Some("fr-FR".into()),
        shared: vec![
            (
                "DatabricksHost".into(),
                format!("\"{HOST}\" meta [IsParameterQuery=true, Type=\"Text\"]"),
            ),
            (
                "DatabricksHttpPath".into(),
                "\"/sql/1.0/warehouses/abc123\" meta [IsParameterQuery=true]".into(),
            ),
            (
                "DatabricksCatalog".into(),
                "\"main\" meta [IsParameterQuery=true]".into(),
            ),
            (
                "DatabricksSchema".into(),
                "\"comptes\" meta [IsParameterQuery=true]".into(),
            ),
            (
                "RowLimit".into(),
                "null meta [IsParameterQuery=true, Type=\"Number\"]".into(),
            ),
        ],
        ..SourceOptions::default()
    }
}

/// The partition a Databricks partition template renders (the query Desktop writes for Databricks).
fn partition(table: &str, columns: &[&str], types: &str, extra: &str) -> String {
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
         Colonnes = Table.SelectColumns(TableSource, {{{cols}}}),\n{extra}    \
         Types = Table.TransformColumnTypes(Colonnes, {{{types}}}),\n    \
         Limite = if RowLimit = null then Types else Table.FirstN(Types, RowLimit)\nin\n    Limite"
    )
}

fn transactions_query(extra: &str) -> String {
    partition(
        "v_transactions",
        &["id", "date", "montant", "statut", "date_chargement"],
        "{\"id\", type text}, {\"date\", type date}, {\"montant\", Currency.Type}, {\"statut\", type text}, {\"date_chargement\", type datetime}",
        extra,
    )
}

fn col<'t>(t: &'t TableData, name: &str) -> &'t [CellValue] {
    &t.column(name).unwrap().values
}

#[test]
fn desktop_partition_loads_through_the_statement_api() {
    let server = FakeServer::start(Warehouse {
        pending_polls: 3,
        chunk_rows: 2,
        ..warehouse()
    });
    let c = connector(&server, Some(TOKEN), settings());
    let t = load_m_with(&transactions_query(""), &options(&c)).unwrap();
    assert_eq!(t.row_count(), 3);
    assert_eq!(
        col(&t, "id"),
        [
            CellValue::Text("t1".into()),
            CellValue::Text("t2".into()),
            CellValue::Text("t3".into())
        ]
    );
    assert_eq!(t.column("montant").unwrap().data_type, DataType::Decimal);
    assert_eq!(
        col(&t, "montant"),
        [
            CellValue::Decimal(-125_000),
            CellValue::Decimal(12_000_000),
            CellValue::Decimal(100)
        ]
    );
    assert_eq!(col(&t, "date")[0], CellValue::DateTime(46_027.0));
    assert_eq!(col(&t, "statut")[1], CellValue::Null);
    assert_eq!(
        col(&t, "date_chargement")[0],
        CellValue::DateTime(46_027.0 + 10.0 / 24.0)
    );
    // 23:59:59.999 stays on its day
    match col(&t, "date_chargement")[2] {
        CellValue::DateTime(x) => assert!(x > 46_022.999 && x < 46_023.0, "{x}"),
        ref v => panic!("{v:?}"),
    }

    // Two statements: the schema listing (the template filters it by name) and one SELECT of
    // the selected columns; nothing else.
    let st = server.statements();
    assert_eq!(st.len(), 2, "{st:?}");
    assert!(
        st[0].contains("FROM `main`.information_schema.tables WHERE table_schema = :p0"),
        "{st:?}"
    );
    assert_eq!(
        st[1],
        "SELECT `id`, `date`, `montant`, `statut`, `date_chargement` FROM `main`.`comptes`.`v_transactions`"
    );
    let reqs = server.requests();
    // Polls, then the second chunk.
    assert!(
        reqs.iter()
            .any(|r| r.method == "GET" && r.path.ends_with("/result/chunks/1"))
    );
    assert!(
        reqs.iter()
            .filter(|r| r.method == "GET" && !r.path.contains("/result/"))
            .count()
            >= 3
    );
    for r in &reqs {
        assert_eq!(
            r.header("authorization"),
            Some(format!("Bearer {TOKEN}").as_str())
        );
    }
    let post = reqs.iter().find(|r| r.method == "POST").unwrap().json();
    assert_eq!(post["warehouse_id"], "abc123");
    assert_eq!(post["format"], "JSON_ARRAY");
    assert_eq!(post["disposition"], "INLINE");
    assert_eq!(post["on_wait_timeout"], "CONTINUE");
    assert_eq!(
        post["parameters"][0],
        json!({"name": "p0", "value": "comptes", "type": "STRING"})
    );
    let log = c.take_log();
    assert_eq!(log.len(), 2);
    assert_eq!(log[1].rows, 3);
    assert!(log[0].sql.ends_with("-- p0 = \"comptes\""), "{}", log[0].sql);
}

#[test]
fn filters_fold_into_parameterized_sql() {
    let server = FakeServer::start(warehouse());
    server.set(|w| {
        w.filtered.insert(
            "(`statut` <> :p0 OR `statut` IS NULL) AND `date` >= :p1|rejet|2026-01-01".into(),
            vec![0, 1],
        );
    });
    let c = connector(&server, Some(TOKEN), settings());
    let q = transactions_query(
        "    Filtre = Table.SelectRows(Colonnes, each [statut] <> \"rejet\" and [date] >= #date(2026, 1, 1)),\n",
    )
    .replace("TransformColumnTypes(Colonnes", "TransformColumnTypes(Filtre");
    let t = load_m_with(&q, &options(&c)).unwrap();
    assert_eq!(t.row_count(), 2);
    let st = server.statements();
    assert_eq!(st.len(), 3, "{st:?}");
    assert_eq!(
        st[2],
        "SELECT `id`, `date`, `montant`, `statut`, `date_chargement` FROM `main`.`comptes`.`v_transactions` \
         WHERE (`statut` <> :p0 OR `statut` IS NULL) AND `date` >= :p1"
    );
    // The column list for folding came from one `LIMIT 0` statement.
    assert!(
        st.iter().any(|s| s.ends_with("`v_transactions` LIMIT 0")),
        "{st:?}"
    );
    let post = server
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST")
        .map(|r| r.json())
        .find(|b| {
            b["statement"]
                .as_str()
                .is_some_and(|s| s.contains("`v_transactions` WHERE"))
        })
        .unwrap();
    assert_eq!(
        post["parameters"],
        json!([
            {"name": "p0", "value": "rejet", "type": "STRING"},
            {"name": "p1", "value": "2026-01-01", "type": "DATE"}
        ])
    );
    // A RowLimit folds into LIMIT only when it applies to the remote table.
    let mut o = options(&c);
    o.shared.retain(|(n, _)| n != "RowLimit");
    o.shared
        .push(("RowLimit".into(), "2 meta [IsParameterQuery=true]".into()));
    let t = load_m_with(&transactions_query(""), &o).unwrap();
    assert_eq!(t.row_count(), 2);
}

#[test]
fn untyped_columns_get_the_databricks_types() {
    let server = FakeServer::start(warehouse());
    let c = connector(&server, Some(TOKEN), settings());
    let q = "let S = DatabricksMultiCloud.Catalogs(DatabricksHost, DatabricksHttpPath, null), \
             T = S{[Name = \"main\", Kind = \"Database\"]}[Data]{[Name = \"comptes\", Kind = \"Schema\"]}[Data]{[Name = \"v_transactions\", Kind = \"View\"]}[Data] in T";
    let t = load_m_with(q, &options(&c)).unwrap();
    let types: Vec<DataType> = t.columns().iter().map(|c| c.data_type).collect();
    assert_eq!(
        types,
        [
            DataType::String,
            DataType::DateTime,
            DataType::Double,
            DataType::String,
            DataType::Int64,
            DataType::DateTime,
            DataType::Double,
            DataType::Boolean
        ]
    );
    assert_eq!(col(&t, "annee")[0], CellValue::Int(2026));
    assert!(matches!(col(&t, "ratio")[1], CellValue::Real(x) if x.is_nan()));
    assert_eq!(
        col(&t, "ok"),
        [CellValue::Bool(true), CellValue::Bool(false), CellValue::Null]
    );
    // Lookups by name and kind need no listing statement.
    assert_eq!(
        server.statements(),
        ["SELECT * FROM `main`.`comptes`.`v_transactions`"]
    );
}

#[test]
fn sql_errors_are_catchable_data_source_errors() {
    let server = FakeServer::start(warehouse());
    let c = connector(&server, Some(TOKEN), settings());
    let q = partition("absente", &["id"], "{\"id\", type text}", "");
    // The listing has no such table: M's own error ({0} on an empty table).
    let r = load_m_with(&q, &options(&c));
    assert!(
        matches!(&r, Err(Error::Evaluation { reason, .. }) if reason == "Expression.Error"),
        "{r:?}"
    );
    // A table the warehouse refuses: DataSource.Error with Databricks' message, and `try` catches it.
    let direct = "let S = DatabricksMultiCloud.Catalogs(DatabricksHost, DatabricksHttpPath, []), \
                  T = S{[Name = \"main\", Kind = \"Database\"]}[Data]{[Name = \"comptes\", Kind = \"Schema\"]}[Data]{[Name = \"absente\", Kind = \"Table\"]}[Data] in T";
    let r = load_m_with(direct, &options(&c));
    assert!(
        matches!(&r, Err(Error::Evaluation { reason, message }) if reason == "DataSource.Error" && message.contains("TABLE_OR_VIEW_NOT_FOUND")),
        "{r:?}"
    );
    let caught = direct.replace(
        " in T",
        ", R = try Table.RowCount(T) otherwise -1 in #table({\"n\"}, {{R}})",
    );
    let t = load_m_with(&caught, &options(&c)).unwrap();
    assert_eq!(col(&t, "n"), [CellValue::Int(-1)]);
    // An unresolved column (SelectColumns folded) is an error, not a silent empty column.
    let q = partition("v_transactions", &["inconnue"], "{\"inconnue\", type text}", "");
    let r = load_m_with(&q, &options(&c));
    assert!(
        matches!(&r, Err(Error::Evaluation { message, .. }) if message.contains("UNRESOLVED_COLUMN")),
        "{r:?}"
    );
}

#[test]
fn credentials_are_never_shown() {
    let server = FakeServer::start(warehouse());
    // A wrong token: fatal, the message names the variable, not the token.
    let wrong = "wrong-token-value-5678";
    let c = connector(&server, Some(wrong), settings());
    let r = load_m_with(&transactions_query(""), &options(&c));
    let Err(Error::Connector(m)) = &r else {
        panic!("{r:?}")
    };
    assert!(
        m.contains("HTTP 401") && m.contains("BICRAFT_DATABRICKS_TOKEN"),
        "{m}"
    );
    assert!(!m.contains(wrong), "{m}");
    assert!(!format!("{r:?}").contains(wrong));
    // `try` does not turn a credentials error into fallback data.
    let q = format!(
        "try ({}) otherwise #table({{\"x\"}}, {{}})",
        transactions_query("")
    );
    assert!(matches!(load_m_with(&q, &options(&c)), Err(Error::Connector(_))));
    // No token at all: no request is made.
    let server = FakeServer::start(warehouse());
    let c = connector(&server, None, settings());
    let r = load_m_with(&transactions_query(""), &options(&c));
    assert!(
        matches!(&r, Err(Error::Connector(m)) if m.contains("BICRAFT_DATABRICKS_TOKEN")),
        "{r:?}"
    );
    assert!(server.requests().is_empty());
}

#[test]
fn throttling_is_retried() {
    let server = FakeServer::start(Warehouse {
        throttle_first: 2,
        ..warehouse()
    });
    let c = connector(&server, Some(TOKEN), settings());
    let t = load_m_with(&transactions_query(""), &options(&c)).unwrap();
    assert_eq!(t.row_count(), 3);
    // Retries exhausted: a fatal error naming the status.
    let busy = FakeServer::start(Warehouse {
        throttle_first: 100,
        ..warehouse()
    });
    let c = connector(
        &busy,
        Some(TOKEN),
        Settings {
            retries: 1,
            ..settings()
        },
    );
    let r = load_m_with(&transactions_query(""), &options(&c));
    assert!(
        matches!(&r, Err(Error::Connector(m)) if m.contains("HTTP 429")),
        "{r:?}"
    );
    assert_eq!(busy.requests().len(), 2);
}

#[test]
fn a_statement_that_never_finishes_is_cancelled() {
    let server = FakeServer::start(Warehouse {
        never_finish: true,
        ..warehouse()
    });
    let c = connector(
        &server,
        Some(TOKEN),
        Settings {
            max_wait: Duration::from_millis(150),
            ..settings()
        },
    );
    let r = load_m_with(&transactions_query(""), &options(&c));
    assert!(
        matches!(&r, Err(Error::Connector(m)) if m.contains("cancelled")),
        "{r:?}"
    );
    assert!(
        server
            .requests()
            .iter()
            .any(|r| r.method == "POST" && r.path.ends_with("/cancel"))
    );
}

#[test]
fn truncated_inline_results_are_refused() {
    let server = FakeServer::start(Warehouse {
        truncated: true,
        ..warehouse()
    });
    let c = connector(&server, Some(TOKEN), settings());
    let r = load_m_with(&transactions_query(""), &options(&c));
    assert!(
        matches!(&r, Err(Error::LimitExceeded(m)) if m.contains("external links")),
        "{r:?}"
    );
}

#[test]
fn external_links_are_fetched_without_the_token() {
    let server = FakeServer::start(Warehouse {
        chunk_rows: 2,
        ..warehouse()
    });
    let c = connector(
        &server,
        Some(TOKEN),
        Settings {
            external_links: true,
            ..settings()
        },
    );
    let t = load_m_with(&transactions_query(""), &options(&c)).unwrap();
    assert_eq!(t.row_count(), 3);
    let reqs = server.requests();
    let ext: Vec<_> = reqs.iter().filter(|r| r.path.starts_with("/ext/")).collect();
    assert!(ext.len() >= 3, "{reqs:?}");
    for r in ext {
        assert_eq!(r.header("authorization"), None);
        assert_eq!(r.header("x-ms-blob-type"), Some("BlockBlob"));
    }
    let post = reqs.iter().find(|r| r.method == "POST").unwrap().json();
    assert_eq!(post["disposition"], "EXTERNAL_LINKS");
}

#[test]
fn warehouse_paths_and_hosts_are_checked() {
    let server = FakeServer::start(warehouse());
    let c = connector(&server, Some(TOKEN), settings());
    let mut o = options(&c);
    o.shared.retain(|(n, _)| n != "DatabricksHttpPath");
    o.shared.push((
        "DatabricksHttpPath".into(),
        "\"/sql/protocolv1/o/123/0123-456789-abc\"".into(),
    ));
    let r = load_m_with(&transactions_query(""), &o);
    assert!(
        matches!(&r, Err(Error::UnsupportedSource(m)) if m.contains("SQL warehouse")),
        "{r:?}"
    );
    let mut o = options(&c);
    o.shared.retain(|(n, _)| n != "DatabricksHost");
    o.shared
        .push(("DatabricksHost".into(), "\"evil.example/path\"".into()));
    let r = load_m_with(&transactions_query(""), &o);
    assert!(matches!(&r, Err(Error::UnsupportedSource(_))), "{r:?}");
    assert!(server.requests().is_empty());
}

#[test]
fn hive_metastore_listing_falls_back_to_show_tables() {
    let server = FakeServer::start(warehouse());
    server.set(|w| {
        w.fail = Some((
            "information_schema".into(),
            "TABLE_OR_VIEW_NOT_FOUND".into(),
            "no information_schema in hive_metastore".into(),
        ));
    });
    let c = connector(&server, Some(TOKEN), settings());
    let q = "let S = DatabricksMultiCloud.Catalogs(DatabricksHost, DatabricksHttpPath, []), \
             Sc = S{[Name = \"main\", Kind = \"Database\"]}[Data]{[Name = \"comptes\", Kind = \"Schema\"]}[Data] in \
             Table.SelectColumns(Sc, {\"Name\", \"Kind\"})";
    let t = load_m_with(q, &options(&c)).unwrap();
    assert_eq!(
        col(&t, "Name"),
        [
            CellValue::Text("dim_categories".into()),
            CellValue::Text("v_transactions".into())
        ]
    );
    assert!(
        server
            .statements()
            .iter()
            .any(|s| s == "SHOW TABLES IN `main`.`comptes`")
    );
}
