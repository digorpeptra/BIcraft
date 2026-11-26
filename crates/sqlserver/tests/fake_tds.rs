//! The SQL Server connector end to end against the scripted fake TDS server (src/fake.rs):
//! TLS inside PRELOGIN, LOGIN7, navigation, folding into the SELECT sent, native queries, types,
//! errors as M errors. No real SQL Server is needed; `real_server_opt_in` runs against one only
//! when `BICRAFT_SQLSERVER_TEST_SERVER` is set.

use std::sync::Arc;

use bicraft_data::connect::Connectors;
use bicraft_data::m::load_m_with;
use bicraft_data::{CellValue, DataType, Error, SourceOptions, TableData};
use bicraft_sqlserver::fake::{self, FakeServer, Script, tok};
use bicraft_sqlserver::{Credential, Password, Settings, SqlServerConnector, sql, tls};

const USER: &str = "reader";
const PASSWORD: &str = "s3cret!";

const ORDERS_COLUMNS: &str = "SELECT TOP (0) * FROM [dbo].[Orders]";

fn orders_meta(status_type: &[u8]) -> Vec<u8> {
    tok::columns(&[
        ("id", tok::INT),
        ("day", tok::DATE),
        ("qty", tok::INT),
        ("amount", tok::DECIMAL),
        ("status", status_type),
        ("city", tok::NVARCHAR),
    ])
}

fn orders_rows() -> Vec<u8> {
    let mut t = orders_meta(tok::NVARCHAR);
    for (id, day, qty, cents, status, city) in [
        (1, 739_620, 3, 1250, Some("open"), Some("Paris")),
        (2, 739_647, 1, -325, Some("closed"), None),
        (3, 739_615, 5, 10_000, None, Some("Lyon")),
    ] {
        t.extend_from_slice(&tok::row(&[
            &tok::int(Some(id)),
            &tok::date(day),
            &tok::int(Some(qty)),
            &tok::decimal(cents),
            &tok::nvarchar(status),
            &tok::nvarchar(city),
        ]));
    }
    t
}

fn objects() -> Vec<u8> {
    let mut t = tok::columns(&[
        ("name", tok::NVARCHAR),
        ("name", tok::NVARCHAR),
        ("type", tok::NVARCHAR),
    ]);
    for (s, o, k) in [
        ("dbo", "Orders", "U "),
        ("dbo", "Recent", "V "),
        ("sales", "Targets", "U "),
    ] {
        t.extend_from_slice(&tok::row(&[
            &tok::nvarchar(Some(s)),
            &tok::nvarchar(Some(o)),
            &tok::nvarchar(Some(k)),
        ]));
    }
    t
}

/// The answers of the fake `Sales` database.
fn answer(sql: &str) -> Option<Vec<u8>> {
    let all = "SELECT [id], [day], [qty], [amount], CAST([status] AS nvarchar(50)) AS [status], [city] FROM [dbo].[Orders]";
    Some(match sql {
        s if s == sql::objects(None) => objects(),
        ORDERS_COLUMNS => orders_meta(tok::VARCHAR),
        s if s == all => orders_rows(),
        "SELECT TOP (2) [id], [qty] FROM [dbo].[Orders] WHERE [qty] >= 2 AND [day] >= DATEFROMPARTS(2026, 1, 1) ORDER BY [qty] DESC, [id] ASC" =>
        {
            let mut t = tok::columns(&[("id", tok::INT), ("qty", tok::INT)]);
            t.extend_from_slice(&tok::row(&[&tok::int(Some(1)), &tok::int(Some(3))]));
            t
        }
        "SELECT 42 AS n, N'é' AS t" => {
            let mut t = tok::columns(&[("n", tok::INT), ("t", tok::NVARCHAR)]);
            t.extend_from_slice(&tok::row(&[&tok::int(Some(42)), &tok::nvarchar(Some("é"))]));
            t
        }
        "SELECT name FROM sys.databases WHERE HAS_DBACCESS(name) = 1 ORDER BY name" => {
            let mut t = tok::columns(&[("name", tok::NVARCHAR)]);
            for d in ["master", "Sales"] {
                t.extend_from_slice(&tok::row(&[&tok::nvarchar(Some(d))]));
            }
            t
        }
        s if s == sql::objects(Some("Sales")) => objects(),
        "SELECT TOP (0) * FROM [Sales].[dbo].[Orders]" => orders_meta(tok::NVARCHAR),
        "SELECT TOP (1) [id], [day], [qty], [amount], [status], [city] FROM [Sales].[dbo].[Orders]" => {
            let mut t = orders_meta(tok::NVARCHAR);
            t.extend_from_slice(&tok::row(&[
                &tok::int(Some(9)),
                &tok::date(739_620),
                &tok::int(None),
                &tok::decimal(1),
                &tok::nvarchar(None),
                &tok::nvarchar(Some("x")),
            ]));
            t
        }
        "SELECT 1/0" => {
            let mut t = tok::columns(&[("", tok::INT)]);
            t.extend_from_slice(&tok::error(8134, 16, "Divide by zero error encountered."));
            t
        }
        _ => return None,
    })
}

fn server() -> FakeServer {
    fake::start(Script::new(USER, PASSWORD, Arc::new(answer))).unwrap()
}

fn connector(user: &str, password: &str, trusted: bool) -> Arc<SqlServerConnector> {
    let (u, p) = (user.to_string(), password.to_string());
    let settings = Settings {
        extra_roots: if trusted {
            tls::pem_certificates(fake::CA_PEM).unwrap()
        } else {
            Vec::new()
        },
        ..Settings::default()
    };
    Arc::new(SqlServerConnector::new(
        Arc::new(move |_| {
            Ok(Credential::Sql {
                user: u.clone(),
                password: Password::new(&p),
            })
        }),
        settings,
    ))
}

fn options(c: &Arc<SqlServerConnector>) -> SourceOptions {
    let mut connectors = Connectors::new();
    connectors.push(c.clone());
    SourceOptions {
        connectors,
        ..SourceOptions::default()
    }
}

fn load(srv: &FakeServer, body: &str) -> Result<TableData, Error> {
    let m = format!(
        "let\n    Source = Sql.Database(\"{}\", \"Sales\"),\n    dbo_Orders = Source{{[Schema=\"dbo\",Item=\"Orders\"]}}[Data]{body}",
        srv.server()
    );
    load_m_with(&m, &options(&connector(USER, PASSWORD, true)))
}

fn col<'t>(t: &'t TableData, name: &str) -> &'t [CellValue] {
    &t.column(name).unwrap().values
}

#[test]
fn desktop_navigation_query_loads_a_table() {
    let srv = server();
    let t = load(&srv, "\nin\n    dbo_Orders").unwrap();
    assert_eq!(t.row_count(), 3);
    assert_eq!(
        col(&t, "id"),
        [CellValue::Int(1), CellValue::Int(2), CellValue::Int(3)]
    );
    // decimal is Decimal.Type, Power BI's "decimal number" (a double)
    assert_eq!(t.column("amount").unwrap().data_type, DataType::Double);
    assert_eq!(
        col(&t, "amount"),
        [
            CellValue::Real(12.5),
            CellValue::Real(-3.25),
            CellValue::Real(100.0)
        ]
    );
    assert_eq!(t.column("day").unwrap().data_type, DataType::DateTime);
    // 2026-01-05 is OLE day 46027
    assert_eq!(col(&t, "day")[0], CellValue::DateTime(46_027.0));
    assert_eq!(
        col(&t, "status"),
        [
            CellValue::Text("open".into()),
            CellValue::Text("closed".into()),
            CellValue::Null
        ]
    );
    let seen = srv.seen();
    assert_eq!(seen.logins, [(USER.to_string(), "Sales".to_string())]);
    // one connection: the listing, the columns (varchar cast to nvarchar), the rows
    assert_eq!(seen.batches.len(), 3);
    assert_eq!(seen.batches[0], sql::objects(None));
    assert_eq!(seen.batches[1], ORDERS_COLUMNS);
    assert!(seen.batches[2].contains("CAST([status] AS nvarchar(50)) AS [status]"));
}

#[test]
fn common_steps_fold_into_one_select() {
    let srv = server();
    let t = load(
        &srv,
        ",\n    Rows = Table.SelectRows(dbo_Orders, each [qty] >= 2 and [day] >= #date(2026, 1, 1)),\n    \
         Cols = Table.SelectColumns(Rows, {\"id\", \"qty\"}),\n    \
         Sorted = Table.Sort(Cols, {{\"id\", Order.Ascending}}),\n    \
         Resorted = Table.Sort(Sorted, {\"qty\", Order.Descending}),\n    \
         Top = Table.FirstN(Resorted, 2)\nin\n    Top",
    )
    .unwrap();
    assert_eq!(col(&t, "id"), [CellValue::Int(1)]);
    let seen = srv.seen();
    assert_eq!(
        seen.batches.last().unwrap(),
        "SELECT TOP (2) [id], [qty] FROM [dbo].[Orders] WHERE [qty] >= 2 AND [day] >= DATEFROMPARTS(2026, 1, 1) ORDER BY [qty] DESC, [id] ASC"
    );
}

#[test]
fn steps_that_do_not_fold_run_locally() {
    let srv = server();
    // text equality does not fold (SQL Server's collations and trailing blanks differ from M);
    // the sort after FirstN and the sort on text do not fold either
    let t = load(
        &srv,
        ",\n    Rows = Table.SelectRows(dbo_Orders, each [city] = \"Paris\" or [city] = null),\n    \
         Sorted = Table.Sort(Rows, {\"city\", Order.Descending})\nin\n    Sorted",
    )
    .unwrap();
    assert_eq!(col(&t, "id"), [CellValue::Int(1), CellValue::Int(2)]);
    let last = srv.seen().batches.last().cloned().unwrap();
    assert!(!last.contains("WHERE") && !last.contains("ORDER BY"), "{last}");
}

#[test]
fn native_query_and_databases_navigation() {
    let srv = server();
    let c = connector(USER, PASSWORD, true);
    let m = format!(
        "Table.SelectRows(Sql.Database(\"{}\", \"Sales\", [Query=\"SELECT 42 AS n, N'é' AS t\"]), each [n] > 0)",
        srv.server()
    );
    let t = load_m_with(&m, &options(&c)).unwrap();
    assert_eq!(col(&t, "n"), [CellValue::Int(42)]);
    assert_eq!(col(&t, "t"), [CellValue::Text("é".into())]);
    // the native query is sent as written, the filter runs locally
    assert_eq!(srv.seen().batches, ["SELECT 42 AS n, N'é' AS t"]);

    let m = format!(
        "let\n    Source = Sql.Databases(\"{}\"),\n    Sales = Source{{[Name=\"Sales\"]}}[Data],\n    \
         T = Sales{{[Schema=\"dbo\",Item=\"Orders\"]}}[Data],\n    Top = Table.FirstN(T, 1)\nin\n    Top",
        srv.server()
    );
    let t = load_m_with(&m, &options(&c)).unwrap();
    assert_eq!(col(&t, "id"), [CellValue::Int(9)]);
    assert_eq!(col(&t, "qty"), [CellValue::Null]);
    let m = format!(
        "Table.SelectColumns(Sql.Databases(\"{}\"), {{\"Name\", \"Kind\"}})",
        srv.server()
    );
    let t = load_m_with(&m, &options(&c)).unwrap();
    assert_eq!(
        col(&t, "Name"),
        [CellValue::Text("master".into()), CellValue::Text("Sales".into())]
    );
    // the navigation table of a database: Name, Data, Schema, Item, Kind
    let m = format!(
        "Table.RemoveColumns(Sql.Database(\"{}\", \"Sales\"), {{\"Data\"}})",
        srv.server()
    );
    let t = load_m_with(&m, &options(&c)).unwrap();
    let names: Vec<&str> = t.columns().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Name", "Schema", "Item", "Kind"]);
    assert_eq!(
        col(&t, "Name"),
        [
            CellValue::Text("dbo.Orders".into()),
            CellValue::Text("dbo.Recent".into()),
            CellValue::Text("sales.Targets".into())
        ]
    );
    assert_eq!(col(&t, "Kind")[1], CellValue::Text("View".into()));
}

#[test]
fn server_errors_are_catchable_m_errors() {
    let srv = server();
    let c = connector(USER, PASSWORD, true);
    let m = format!(
        "let r = try Table.RowCount(Sql.Database(\"{}\", \"Sales\", [Query=\"SELECT 1/0\"])) in #table({{\"reason\", \"message\"}}, {{{{r[Error][Reason], r[Error][Message]}}}})",
        srv.server()
    );
    let t = load_m_with(&m, &options(&c)).unwrap();
    assert_eq!(col(&t, "reason"), [CellValue::Text("DataSource.Error".into())]);
    assert_eq!(
        col(&t, "message"),
        [CellValue::Text(
            "Microsoft SQL: Divide by zero error encountered.".into()
        )]
    );
    let m = format!(
        "Sql.Database(\"{}\", \"Sales\", [Query=\"SELECT * FROM nope\"])",
        srv.server()
    );
    match load_m_with(&m, &options(&c)) {
        Err(e) => assert!(
            e.to_string().contains("Microsoft SQL: Invalid object name"),
            "{e}"
        ),
        Ok(_) => panic!("loaded a missing table"),
    }
}

#[test]
fn refused_logins_and_connections() {
    let srv = server();
    let q = format!(
        "Sql.Database(\"{}\", \"Sales\", [Query=\"SELECT 42 AS n, N'é' AS t\"])",
        srv.server()
    );
    // a wrong password: a credentials error, which `try` does not catch, without the password
    let e = load_m_with(
        &format!("try Table.RowCount({q}) otherwise null"),
        &options(&connector(USER, "wrong", true)),
    )
    .unwrap_err();
    let text = e.to_string();
    assert!(
        matches!(e, Error::Connector(_)) && text.contains("Login failed for user 'reader'"),
        "{text}"
    );
    assert!(!text.contains("wrong"));
    // a certificate of an unknown CA
    let e = load_m_with(&q, &options(&connector(USER, PASSWORD, false))).unwrap_err();
    assert!(e.to_string().contains("TLS"), "{e}");
    // a server without encryption
    let plain = fake::start(Script {
        encryption: 0x02,
        ..Script::new(USER, PASSWORD, Arc::new(answer))
    })
    .unwrap();
    let q2 = format!(
        "Sql.Database(\"{}\", \"Sales\", [Query=\"SELECT 1\"])",
        plain.server()
    );
    let e = load_m_with(&q2, &options(&connector(USER, PASSWORD, true))).unwrap_err();
    assert!(e.to_string().contains("encrypted"), "{e}");
    assert!(plain.seen().logins.is_empty());
    // Windows authentication
    let win = Arc::new(SqlServerConnector::new(
        Arc::new(|_| Ok(Credential::Windows)),
        Settings::default(),
    ));
    let e = load_m_with(&q, &options(&win)).unwrap_err();
    assert!(
        e.to_string().contains("Windows (integrated) authentication"),
        "{e}"
    );
    // no credentials
    let none = Arc::new(SqlServerConnector::new(
        Arc::new(|_| Err("no credentials for this server".into())),
        Settings::default(),
    ));
    let e = load_m_with(&q, &options(&none)).unwrap_err();
    assert!(e.to_string().contains("no credentials"), "{e}");
    // a closed port
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let q3 = format!("Sql.Database(\"127.0.0.1,{port}\", \"Sales\", [Query=\"SELECT 1\"])");
    assert!(load_m_with(&q3, &options(&connector(USER, PASSWORD, true))).is_err());
}

/// Against a real SQL Server, only when `BICRAFT_SQLSERVER_TEST_SERVER` (and
/// `BICRAFT_SQLSERVER_USER`, `BICRAFT_SQLSERVER_PASSWORD`, optionally
/// `BICRAFT_SQLSERVER_TEST_DATABASE`, default `master`) are set.
#[test]
fn real_server_opt_in() {
    let Ok(server) = std::env::var("BICRAFT_SQLSERVER_TEST_SERVER") else {
        eprintln!("note: BICRAFT_SQLSERVER_TEST_SERVER is unset; the real SQL Server test is skipped");
        return;
    };
    let (Ok(user), Ok(password)) = (
        std::env::var(bicraft_sqlserver::USER_ENV),
        std::env::var(bicraft_sqlserver::PASSWORD_ENV),
    ) else {
        eprintln!("note: BICRAFT_SQLSERVER_USER / _PASSWORD are unset; the real SQL Server test is skipped");
        return;
    };
    let db = std::env::var("BICRAFT_SQLSERVER_TEST_DATABASE").unwrap_or_else(|_| "master".into());
    let settings = Settings::from_env().unwrap();
    let c = Arc::new(SqlServerConnector::new(
        Arc::new(move |_| {
            Ok(Credential::Sql {
                user: user.clone(),
                password: Password::new(&password),
            })
        }),
        settings,
    ));
    let m = format!(
        "Sql.Database(\"{server}\", \"{db}\", [Query=\"SELECT CAST(1 AS int) AS i, CAST(2 AS bigint) AS b, \
         CAST(1.5 AS decimal(9,2)) AS d, CAST('é' AS varchar(5)) AS v, N'ü' AS n, CAST('2026-11-18' AS date) AS dt, \
         CAST('2026-11-18T10:20:30.123' AS datetime2(3)) AS dt2, CAST('10:20:30' AS time) AS t, NEWID() AS g\"])"
    );
    let t = load_m_with(&m, &options(&c)).unwrap();
    assert_eq!(col(&t, "i"), [CellValue::Int(1)]);
    assert_eq!(col(&t, "n"), [CellValue::Text("ü".into())]);
    let m = format!("Table.FirstN(Sql.Database(\"{server}\", \"{db}\"), 5)");
    load_m_with(&m, &options(&c)).unwrap();
}
