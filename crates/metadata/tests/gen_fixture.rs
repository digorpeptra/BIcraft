//! Regenerate `tests/fixtures/synthetic_metadata.sqlitedb`.
//!
//! A tiny metadata database shaped like the TMSCHEMA tables of an Analysis Services
//! `metadata.sqlitedb`: bracket-quoted names, INTEGER/TEXT/REAL columns, and `ID` as an
//! `INTEGER` primary key declared as a table constraint. Only a subset of the columns is
//! created; the reader must cope with missing ones. All data is invented for these tests.
//!
//! SQLite itself writes it (the `rusqlite` dev-dependency, with SQLite compiled from its bundled
//! source). Ignored by default, since it rewrites a committed file:
//!
//! ```text
//! cargo test -p bicraft-metadata --test gen_fixture -- --ignored
//! ```
//!
//! The statements run as the fixture was first written: the first `CREATE TABLE` commits on its
//! own, the first `INSERT` opens a transaction that holds every later statement until the
//! commit, then `VACUUM`. The file header records the SQLite version that wrote it.

use std::path::{Path, PathBuf};

use rusqlite::types::Value as V;
use rusqlite::{Connection, params_from_iter};

fn out_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("synthetic_metadata.sqlitedb")
}

const SCHEMA: &[(&str, &str)] = &[
    (
        "Model",
        "ID INTEGER, Name TEXT, Culture TEXT, DefaultMode INTEGER, \
         DefaultPowerBIDataSourceVersion INTEGER, DataAccessOptions TEXT, ValueFilterBehavior INTEGER",
    ),
    (
        "Table",
        "ID INTEGER, ModelID INTEGER, Name TEXT, IsHidden INTEGER, SystemFlags INTEGER, \
         LineageTag TEXT, Description TEXT",
    ),
    (
        "Column",
        "ID INTEGER, TableID INTEGER, ExplicitName TEXT, InferredName TEXT, \
         ExplicitDataType INTEGER, InferredDataType INTEGER, IsHidden INTEGER, IsUnique INTEGER, \
         IsKey INTEGER, IsNullable INTEGER, SummarizeBy INTEGER, Type INTEGER, SourceColumn TEXT, \
         Expression TEXT, FormatString TEXT, SortByColumnID INTEGER, LineageTag TEXT",
    ),
    (
        "Measure",
        "ID INTEGER, TableID INTEGER, Name TEXT, Description TEXT, DataType INTEGER, \
         Expression TEXT, FormatString TEXT, IsHidden INTEGER, State INTEGER, ModifiedTime INTEGER, \
         StructureModifiedTime INTEGER, KPIID INTEGER, IsSimpleMeasure INTEGER, ErrorMessage TEXT, \
         DisplayFolder TEXT, DetailRowsDefinitionID INTEGER, DataCategory TEXT, \
         FormatStringDefinitionID INTEGER, LineageTag TEXT, SourceLineageTag TEXT",
    ),
    (
        "Partition",
        "ID INTEGER, TableID INTEGER, Name TEXT, QueryDefinition TEXT, Type INTEGER, \
         Mode INTEGER, DataView INTEGER, RangeStart REAL",
    ),
    (
        "Relationship",
        "ID INTEGER, ModelID INTEGER, Name TEXT, IsActive INTEGER, \
         CrossFilteringBehavior INTEGER, FromTableID INTEGER, FromColumnID INTEGER, \
         FromCardinality INTEGER, ToTableID INTEGER, ToColumnID INTEGER, ToCardinality INTEGER",
    ),
    (
        "Expression",
        "ID INTEGER, ModelID INTEGER, Name TEXT, Kind INTEGER, Expression TEXT",
    ),
    (
        "Annotation",
        "ID INTEGER, ObjectID INTEGER, ObjectType INTEGER, Name TEXT, Value TEXT, \
         ModifiedTime INTEGER",
    ),
    (
        "Role",
        "ID INTEGER, ModelID INTEGER, Name TEXT, Description TEXT, ModelPermission INTEGER, \
         ModifiedTime INTEGER",
    ),
    (
        "TablePermission",
        "ID INTEGER, RoleID INTEGER, TableID INTEGER, FilterExpression TEXT, \
         ModifiedTime INTEGER, State INTEGER, ErrorMessage TEXT, MetadataPermission INTEGER",
    ),
    (
        "RoleMembership",
        "ID INTEGER, RoleID INTEGER, MemberName TEXT, MemberID TEXT, \
         IdentityProvider TEXT, MemberType INTEGER, ModifiedTime INTEGER",
    ),
    // Storage table: ignored by the reader.
    (
        "StorageFile",
        "ID INTEGER, OwnerID INTEGER, OwnerType INTEGER, FileName TEXT",
    ),
];

/// Windows FILETIME ticks (100 ns since 1601-01-01), as the engine stores ModifiedTime.
const T0: i64 = 133800000000000000;
const T1: i64 = 133800000010000000;

const M_QUERY: &str = "let\n    Source = #table({\"Amount\"}, {{1.5}})\nin\n    Source";

fn i(v: i64) -> V {
    V::Integer(v)
}

fn t(s: &str) -> V {
    V::Text(s.into())
}

fn r(v: f64) -> V {
    V::Real(v)
}

const N: V = V::Null;

#[rustfmt::skip]
fn rows(table: &str) -> Vec<Vec<V>> {
    match table {
        "Model" => vec![vec![
            i(1),
            t("Model"),
            t("en-US"),
            i(0),
            i(2),
            t(r#"{"legacyRedirects": true}"#),
            i(1),
        ]],
        "Table" => vec![
            vec![i(3), i(1), t("Sales"), i(0), i(0), t("lt-sales"), N],
            vec![i(2), i(1), t("Calendar"), i(1), i(2), N, t("Days")],
            vec![i(50), i(0), t("H$Sales (3)$Amount (4)"), i(1), i(1), N, N],
        ],
        "Column" => vec![
            vec![
                i(4), i(3), t("Amount"), N, i(8), N, i(0), i(0), i(0), i(1), i(3), i(1),
                t("Amount"), N, t("0.0"), i(0), t("lt-amount"),
            ],
            vec![
                i(5), i(3), t("RowNumber-0"), N, i(6), i(19), i(1), i(1), i(1), i(0), i(1), i(3),
                N, N, N, i(0), N,
            ],
            vec![
                i(6), i(3), t("Day"), N, i(9), N, i(0), i(0), i(0), i(1), i(1), i(1), t("Day"), N,
                N, i(0), N,
            ],
            vec![
                i(7), i(2), N, t("Date"), i(1), i(9), i(0), i(0), i(0), i(1), i(2), i(4),
                t("[Date]"), N, N, i(0), N,
            ],
            vec![
                i(8), i(2), t("Month"), N, i(2), i(2), i(0), i(0), i(0), i(1), i(2), i(2), N,
                t("FORMAT([Date], \"MMM\")"), N, i(7), N,
            ],
            vec![
                i(51), i(50), t("POS_TO_ID"), N, i(6), N, i(0), i(0), i(0), i(1), i(1), i(1), N,
                N, N, i(0), N,
            ],
        ],
        "Measure" => vec![
            vec![
                i(10), i(3), t("Total"), N, i(8), t("SUM(Sales[Amount])"), t("#,0.0"), i(0), i(1),
                i(T1), i(T0), i(0), i(0), N, t("KPIs"), i(0), N, i(0), t("lt-total"), N,
            ],
            vec![
                i(9), i(3), t("Rows"), t("Row count"), i(6), t("COUNTROWS(Sales)"), N, i(1),
                i(1), i(T1), i(T0), i(0), i(0), N, N, i(0), N, i(0), N, N,
            ],
        ],
        "Partition" => vec![
            vec![i(11), i(3), t("Sales"), t(M_QUERY), i(4), i(0), i(3), r(0.0)],
            vec![
                i(12),
                i(2),
                t("Calendar"),
                t("CALENDAR(DATE(2020,1,1), DATE(2020,12,31))"),
                i(2),
                i(0),
                i(3),
                r(0.0),
            ],
            vec![i(52), i(50), t("sys"), N, i(3), i(2), i(3), r(0.0)],
        ],
        "Relationship" => vec![vec![
            i(13),
            i(1),
            t("rel-day"),
            i(1),
            i(1),
            i(3),
            i(6),
            i(2),
            i(2),
            i(7),
            i(1),
        ]],
        "Expression" => vec![vec![
            i(14),
            i(1),
            t("Rate"),
            i(0),
            t("1.2 meta [IsParameterQuery=true]"),
        ]],
        "Annotation" => vec![
            vec![i(15), i(1), i(1), t("PBI_QueryOrder"), t(r#"["Sales"]"#), i(T0)],
            vec![i(16), i(3), i(3), t("PBI_ResultType"), t("Table"), i(T0)],
            vec![i(17), i(7), i(4), t("SummarizationSetBy"), t("Automatic"), i(T0)],
            vec![
                i(19),
                i(10),
                i(8),
                t("PBI_FormatHint"),
                t(r#"{"isGeneralNumber":true}"#),
                i(T0),
            ],
        ],
        "Role" => vec![vec![i(20), i(1), t("Readers"), N, i(2), i(T0)]],
        "TablePermission" => vec![vec![
            i(21),
            i(20),
            i(3),
            t("Sales[Amount] > 0"),
            i(T0),
            i(1),
            N,
            i(0),
        ]],
        "RoleMembership" => vec![vec![
            i(22),
            i(20),
            t("someone@example.com"),
            N,
            N,
            i(2),
            i(T0),
        ]],
        "StorageFile" => vec![vec![i(18), i(4), i(4), t("Amount.idf")]],
        _ => Vec::new(),
    }
}

const DBPROPERTIES: &[(&str, i64)] = &[("MAXID", 52), ("SCHEMAVERSION", 123)];

/// Runs statements with an implicit transaction opened before the first `INSERT`, as the
/// fixture was first written.
struct Writer {
    con: Connection,
    in_txn: bool,
}

impl Writer {
    fn ddl(&self, sql: &str) {
        self.con.execute(sql, []).unwrap();
    }

    fn insert(&mut self, sql: &str, values: &[V]) {
        if !self.in_txn {
            self.con.execute("BEGIN", []).unwrap();
            self.in_txn = true;
        }
        self.con.execute(sql, params_from_iter(values)).unwrap();
    }
}

#[test]
#[ignore = "rewrites the committed fixture: cargo test -p bicraft-metadata --test gen_fixture -- --ignored"]
fn regenerate_fixture() {
    let out = out_path();
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    if out.exists() {
        std::fs::remove_file(&out).unwrap();
    }
    let con = Connection::open(&out).unwrap();
    // SQLite's default; the bundled build turns foreign keys on.
    con.execute("PRAGMA foreign_keys = OFF", []).unwrap();
    con.execute("PRAGMA page_size = 4096", []).unwrap();
    let mut w = Writer { con, in_txn: false };
    for (table, cols) in SCHEMA {
        let decl: Vec<String> = cols
            .split(", ")
            .map(|c| {
                let mut parts = c.split_whitespace();
                let name = parts.next().unwrap_or_default();
                let ty = parts.next().unwrap_or_default();
                format!("[{name}] {ty}")
            })
            .collect();
        w.ddl(&format!(
            "CREATE TABLE [{table}]( {} ,  PRIMARY KEY(\"ID\" ASC) )",
            decl.join(" ,  ")
        ));
        for row in rows(table) {
            let marks = vec!["?"; row.len()].join(", ");
            w.insert(&format!("INSERT INTO [{table}] VALUES ({marks})"), &row);
        }
    }
    // The engine's ID counter: a name/value table without a rowid alias.
    w.ddl("CREATE TABLE [DBPROPERTIES] ([NAME] STRING, [VALUE] INTEGER, PRIMARY KEY ([NAME] ASC))");
    for (name, value) in DBPROPERTIES {
        w.insert("INSERT INTO [DBPROPERTIES] VALUES (?, ?)", &[t(name), i(*value)]);
    }
    if w.in_txn {
        w.con.execute("COMMIT", []).unwrap();
    }
    w.con.execute("VACUUM", []).unwrap();
    w.con.close().unwrap();
    eprintln!("{} written by SQLite {}", out.display(), rusqlite::version());
}
