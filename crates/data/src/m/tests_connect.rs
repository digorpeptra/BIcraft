//! Connector values (`remote.rs`) over an in-memory connector: navigation, folding (checked
//! against the same query evaluated locally), types, errors, and the default refusal.

use std::sync::{Arc, Mutex};

use super::*;
use crate::connect::{
    Arg, CmpOp, ConnectError, ConnectErrorKind, Connector, Connectors, Literal, NavEntry, Pred, RemoteColumn,
    RemoteQuery, RemoteRows, RemoteType, RemoteValue, Source,
};
use CellValue::{Bool, Decimal, Int, Null, Real, Text};

/// One table of the fake catalog `main.sales`.
#[derive(Clone)]
struct FakeTable {
    name: &'static str,
    kind: &'static str,
    columns: Vec<RemoteColumn>,
    rows: Vec<Vec<RemoteValue>>,
}

#[derive(Default)]
struct Log {
    calls: Vec<String>,
    queries: Vec<RemoteQuery>,
}

struct Fake {
    tables: Vec<FakeTable>,
    log: Arc<Mutex<Log>>,
    fail_with: Option<ConnectErrorKind>,
}

impl Connector for Fake {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn handles(&self, function: &str) -> bool {
        function == "Fake.Catalogs"
    }
    fn open(&self, function: &str, args: &[Arg]) -> Result<Box<dyn Source>, ConnectError> {
        self.log
            .lock()
            .unwrap()
            .calls
            .push(format!("open {function} {args:?}"));
        Ok(Box::new(FakeSource {
            tables: self.tables.clone(),
            log: Arc::clone(&self.log),
            fail_with: self.fail_with,
        }))
    }
}

struct FakeSource {
    tables: Vec<FakeTable>,
    log: Arc<Mutex<Log>>,
    fail_with: Option<ConnectErrorKind>,
}

impl FakeSource {
    fn table(&self, path: &[String]) -> Result<&FakeTable, ConnectError> {
        match path {
            [c, s, t] if c == "main" && s == "sales" => self
                .tables
                .iter()
                .find(|x| x.name == t)
                .ok_or_else(|| ConnectError::new(ConnectErrorKind::DataSource, format!("no table {t}"))),
            _ => Err(ConnectError::new(ConnectErrorKind::DataSource, "bad path")),
        }
    }
}

/// The value of `[col] op lit` in M terms (null handling as documented on `Pred`).
fn holds(p: &Pred, cols: &[RemoteColumn], row: &[RemoteValue]) -> bool {
    let cell = |c: &str| {
        cols.iter()
            .position(|x| x.name == c)
            .and_then(|i| row.get(i))
            .cloned()
            .unwrap_or(RemoteValue::Null)
    };
    match p {
        Pred::And(v) => v.iter().all(|p| holds(p, cols, row)),
        Pred::IsNull(c) => cell(c) == RemoteValue::Null,
        Pred::NotNull(c) => cell(c) != RemoteValue::Null,
        Pred::Compare { column, op, value } => {
            let v = cell(column);
            if v == RemoteValue::Null {
                return *op == CmpOp::Ne;
            }
            let ord = match (&v, value) {
                (RemoteValue::Int(a), Literal::Int(b)) => a.cmp(b),
                (RemoteValue::Decimal(a), Literal::Int(b)) => {
                    a.parse::<f64>().unwrap().partial_cmp(&(*b as f64)).unwrap()
                }
                (RemoteValue::Text(a), Literal::Text(b)) => a.as_str().cmp(b.as_str()),
                (RemoteValue::Logical(a), Literal::Logical(b)) => a.cmp(b),
                (
                    RemoteValue::Date { year, month, day },
                    Literal::Date {
                        year: y,
                        month: m,
                        day: d,
                    },
                ) => (year, month, day).cmp(&(y, m, d)),
                _ => panic!("unexpected comparison {v:?} {value:?}"),
            };
            use std::cmp::Ordering::*;
            match op {
                CmpOp::Eq => ord == Equal,
                CmpOp::Ne => ord != Equal,
                CmpOp::Lt => ord == Less,
                CmpOp::Le => ord != Greater,
                CmpOp::Gt => ord == Greater,
                CmpOp::Ge => ord != Less,
            }
        }
    }
}

impl Source for FakeSource {
    fn lookup(
        &mut self,
        path: &[String],
        name: &str,
        kind: Option<&str>,
    ) -> Result<Option<NavEntry>, ConnectError> {
        self.log
            .lock()
            .unwrap()
            .calls
            .push(format!("lookup {path:?} {name} {kind:?}"));
        Ok(match (path.len(), kind) {
            (0, Some("Database")) => Some(NavEntry {
                name: name.into(),
                kind: "Database".into(),
                leaf: false,
                fields: Vec::new(),
            }),
            (1, Some("Schema")) => Some(NavEntry {
                name: name.into(),
                kind: "Schema".into(),
                leaf: false,
                fields: Vec::new(),
            }),
            _ => None,
        })
    }
    fn list(&mut self, path: &[String]) -> Result<Vec<NavEntry>, ConnectError> {
        self.log.lock().unwrap().calls.push(format!("list {path:?}"));
        if let Some(kind) = self.fail_with {
            return Err(ConnectError::new(kind, "listing failed"));
        }
        Ok(match path.len() {
            0 => vec![NavEntry {
                name: "main".into(),
                kind: "Database".into(),
                leaf: false,
                fields: Vec::new(),
            }],
            1 => vec![NavEntry {
                name: "sales".into(),
                kind: "Schema".into(),
                leaf: false,
                fields: Vec::new(),
            }],
            _ => self
                .tables
                .iter()
                .map(|t| NavEntry {
                    name: t.name.into(),
                    kind: t.kind.into(),
                    leaf: true,
                    fields: Vec::new(),
                })
                .collect(),
        })
    }
    fn columns(&mut self, path: &[String]) -> Result<Vec<RemoteColumn>, ConnectError> {
        self.log.lock().unwrap().calls.push(format!("columns {path:?}"));
        Ok(self.table(path)?.columns.clone())
    }
    fn fetch(&mut self, q: &RemoteQuery, max_rows: usize) -> Result<RemoteRows, ConnectError> {
        self.log.lock().unwrap().queries.push(q.clone());
        let t = self.table(&q.table)?;
        let mut rows: Vec<Vec<RemoteValue>> = t
            .rows
            .iter()
            .filter(|r| q.filter.iter().all(|p| holds(p, &t.columns, r)))
            .cloned()
            .collect();
        if let Some(n) = q.limit {
            rows.truncate(n as usize);
        }
        if rows.len() > max_rows {
            return Err(ConnectError::new(ConnectErrorKind::Limit, "too many rows"));
        }
        let (columns, rows) = match &q.columns {
            None => (t.columns.clone(), rows),
            Some(names) => {
                // Like SQL: names resolve case-insensitively and come back as stored.
                let idx: Vec<usize> = names
                    .iter()
                    .map(|n| {
                        t.columns
                            .iter()
                            .position(|c| c.name.eq_ignore_ascii_case(n))
                            .ok_or_else(|| {
                                ConnectError::new(ConnectErrorKind::DataSource, "unresolved column")
                            })
                    })
                    .collect::<Result<_, _>>()?;
                (
                    idx.iter().map(|&i| t.columns[i].clone()).collect(),
                    rows.into_iter()
                        .map(|r| idx.iter().map(|&i| r[i].clone()).collect())
                        .collect(),
                )
            }
        };
        Ok(RemoteRows { columns, rows })
    }
    fn native(&mut self, q: &RemoteQuery) -> Result<Option<String>, ConnectError> {
        let cols = q.columns.as_ref().map_or("*".to_string(), |c| c.join(", "));
        let mut text = format!("SELECT {cols} FROM {}", q.table.join("."));
        if !q.filter.is_empty() {
            text.push_str(&format!(" WHERE {:?}", q.filter));
        }
        if let Some(n) = q.limit {
            text.push_str(&format!(" LIMIT {n}"));
        }
        Ok(Some(text))
    }
}

fn col(name: &str, ty: RemoteType) -> RemoteColumn {
    RemoteColumn {
        name: name.into(),
        ty,
        comparable: true,
    }
}

fn date(year: i32, month: u8, day: u8) -> RemoteValue {
    RemoteValue::Date { year, month, day }
}

fn orders() -> FakeTable {
    use RemoteValue as R;
    let mut lcase = col("label", RemoteType::Text);
    lcase.comparable = false;
    FakeTable {
        name: "orders",
        kind: "Table",
        columns: vec![
            col("id", RemoteType::Int64),
            col("day", RemoteType::Date),
            col("amount", RemoteType::Decimal),
            col("status", RemoteType::Text),
            col("paid", RemoteType::Logical),
            col("ratio", RemoteType::Double),
            lcase,
            col("at", RemoteType::DateTime),
        ],
        rows: vec![
            vec![
                R::Int(1),
                date(2026, 1, 5),
                R::Decimal("12.50".into()),
                R::Text("open".into()),
                R::Logical(true),
                R::Double(0.5),
                R::Text("A".into()),
                R::DateTime {
                    year: 2026,
                    month: 1,
                    day: 5,
                    millis: 3_600_000,
                },
            ],
            vec![
                R::Int(2),
                date(2026, 2, 1),
                R::Decimal("-3.1234567".into()),
                R::Null,
                R::Logical(false),
                R::Double(1.5),
                R::Text("b".into()),
                R::Null,
            ],
            vec![
                R::Int(3),
                date(2025, 12, 31),
                R::Null,
                R::Text("closed".into()),
                R::Null,
                R::Null,
                R::Null,
                R::Null,
            ],
        ],
    }
}

fn fake(fail_with: Option<ConnectErrorKind>) -> (SourceOptions, Arc<Mutex<Log>>) {
    let log = Arc::new(Mutex::new(Log::default()));
    let mut connectors = Connectors::new();
    connectors.push(Arc::new(Fake {
        tables: vec![
            orders(),
            FakeTable {
                name: "v_orders",
                kind: "View",
                ..orders()
            },
        ],
        log: Arc::clone(&log),
        fail_with,
    }));
    let opts = SourceOptions {
        connectors,
        shared: vec![
            ("Cat".into(), "\"main\" meta [IsParameterQuery=true]".into()),
            ("Sch".into(), "\"sales\" meta [IsParameterQuery=true]".into()),
            ("RowLimit".into(), "null meta [IsParameterQuery=true]".into()),
        ],
        ..SourceOptions::default()
    };
    (opts, log)
}

/// The query shape of a Databricks partition template (Desktop's Databricks query), over `steps`.
fn template(table: &str, steps: &str) -> String {
    format!(
        "let\n    Source = Fake.Catalogs(\"host\", \"/sql/1.0/warehouses/abc\", [Catalog = null, Implementation = \"2.0\"]),\n    \
         Catalogue = Source{{[Name = Cat, Kind = \"Database\"]}}[Data],\n    \
         Schema = Catalogue{{[Name = Sch, Kind = \"Schema\"]}}[Data],\n    \
         TableSource = Table.SelectRows(Schema, each [Name] = \"{table}\"){{0}}[Data],\n    \
         {steps}\nin\n    Result"
    )
}

#[test]
fn template_query_folds_columns_and_types_locally() {
    let (opts, log) = fake(None);
    let e = template(
        "v_orders",
        "Colonnes = Table.SelectColumns(TableSource, {\"id\", \"day\", \"amount\", \"status\"}),\n    \
         Types = Table.TransformColumnTypes(Colonnes, {{\"id\", type text}, {\"day\", type date}, {\"amount\", Currency.Type}, {\"status\", type text}}),\n    \
         Result = if RowLimit = null then Types else Table.FirstN(Types, RowLimit)",
    );
    let t = load_m_with(&e, &opts).unwrap();
    let names: Vec<_> = t.columns().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "day", "amount", "status"]);
    assert_eq!(
        t.columns()[0].values,
        vec![Text("1".into()), Text("2".into()), Text("3".into())]
    );
    assert_eq!(t.columns()[1].data_type, DataType::DateTime);
    assert_eq!(t.columns()[1].values[0], CellValue::DateTime(46_027.0));
    assert_eq!(t.columns()[2].data_type, DataType::Decimal);
    // Currency rounds half to even at 4 places, from the exact decimal.
    assert_eq!(
        t.columns()[2].values,
        vec![Decimal(125_000), Decimal(-31_235), Null]
    );
    assert_eq!(
        t.columns()[3].values,
        vec![Text("open".into()), Null, Text("closed".into())]
    );
    let log = log.lock().unwrap();
    // Catalog and schema by key without a listing; the schema listed once; one query.
    assert!(
        log.calls
            .iter()
            .any(|c| c.starts_with("lookup [] main Some(\"Database\")"))
    );
    assert_eq!(log.calls.iter().filter(|c| c.starts_with("list")).count(), 1);
    assert_eq!(
        log.queries,
        vec![RemoteQuery {
            table: vec!["main".into(), "sales".into(), "v_orders".into()],
            columns: Some(vec!["id".into(), "day".into(), "amount".into(), "status".into()]),
            filter: vec![],
            limit: None,
            ..Default::default()
        }]
    );
}

#[test]
fn untyped_load_uses_the_connector_types() {
    let (opts, _) = fake(None);
    let e = template("orders", "Result = TableSource");
    let t = load_m_with(&e, &opts).unwrap();
    let types: Vec<_> = t.columns().iter().map(|c| c.data_type).collect();
    assert_eq!(
        types,
        [
            DataType::Int64,
            DataType::DateTime,
            DataType::Double,
            DataType::String,
            DataType::Boolean,
            DataType::Double,
            DataType::String,
            DataType::DateTime,
        ]
    );
    assert_eq!(t.columns()[2].values, vec![Real(12.5), Real(-3.1234567), Null]);
    assert_eq!(t.columns()[4].values, vec![Bool(true), Bool(false), Null]);
    assert_eq!(t.date_kind(1), Some(crate::DateKind::Date));
    assert_eq!(t.date_kind(7), Some(crate::DateKind::DateTime));
    assert_eq!(
        t.columns()[7].values[0],
        CellValue::DateTime(46_027.0 + 1.0 / 24.0)
    );
}

/// Rows of `filter` over the orders table, folded and evaluated locally (`Table.Buffer` fetches
/// first), with the queries the folded run sent.
fn both(filter: &str) -> (TableData, TableData, Vec<RemoteQuery>) {
    let (opts, log) = fake(None);
    let folded = load_m_with(
        &template(
            "orders",
            &format!("Result = {}", filter.replace("$T", "TableSource")),
        ),
        &opts,
    )
    .unwrap();
    let queries = log.lock().unwrap().queries.clone();
    let local = load_m_with(
        &template(
            "orders",
            &format!("Result = {}", filter.replace("$T", "Table.Buffer(TableSource)")),
        ),
        &opts,
    )
    .unwrap();
    (folded, local, queries)
}

#[test]
fn row_filters_fold_with_m_semantics() {
    let cases = [
        ("Table.SelectRows($T, each [status] = \"open\")", 1),
        // null <> "open" is true in M: rows with a null status are kept.
        ("Table.SelectRows($T, each [status] <> \"open\")", 2),
        ("Table.SelectRows($T, each [status] = null)", 1),
        ("Table.SelectRows($T, each null <> [status])", 2),
        ("Table.SelectRows($T, each [id] >= 2 and [paid] = false)", 1),
        ("Table.SelectRows($T, each 2 > [id])", 1),
        ("Table.SelectRows($T, each [day] < #date(2026, 1, 31))", 2),
        ("Table.SelectRows($T, each [amount] > 0)", 1),
        ("Table.SelectRows($T, (r) => r[id] <> 3)", 2),
        (
            "Table.SelectRows(Table.SelectColumns($T, {\"id\", \"paid\"}), each [paid] = true)",
            1,
        ),
        ("Table.FirstN(Table.SelectRows($T, each [id] > 1), 1)", 1),
        (
            "Table.RemoveColumns(Table.SelectRows($T, each [id] = 2), {\"label\", \"at\"})",
            1,
        ),
    ];
    for (filter, rows) in cases {
        let (folded, local, queries) = both(filter);
        assert_eq!(folded, local, "{filter}");
        assert_eq!(folded.row_count(), rows, "{filter}");
        assert_eq!(queries.len(), 1, "{filter}");
        assert!(!queries[0].filter.is_empty(), "{filter}: not folded");
    }
    let (_, _, q) = both("Table.SelectRows($T, each [status] <> \"open\")");
    assert_eq!(
        q[0].filter,
        vec![Pred::Compare {
            column: "status".into(),
            op: CmpOp::Ne,
            value: Literal::Text("open".into())
        }]
    );
    let (t, _, q) = both("Table.RemoveColumns(Table.SelectRows($T, each [id] = 2), {\"label\", \"at\"})");
    assert_eq!(t.columns().len(), 6);
    assert_eq!(q[0].columns.as_ref().map(Vec::len), Some(6));
}

#[test]
fn filters_that_do_not_fold_run_locally() {
    let cases = [
        // a text order comparison, a non-binary collation, a datetime, a fractional number,
        // `or`, a function call, a limit before the filter, a column of another row shape
        "Table.SelectRows($T, each [status] < \"p\")",
        "Table.SelectRows($T, each [label] = \"a\")",
        "Table.SelectRows($T, each [at] = null or [id] = 1)",
        "Table.SelectRows($T, each [ratio] = 0.5)",
        "Table.SelectRows($T, each Text.StartsWith([status] ?? \"\", \"o\"))",
        "Table.SelectRows(Table.FirstN($T, 2), each [id] = 2)",
        "Table.SelectRows($T, each [id] = [id])",
    ];
    for filter in cases {
        let (folded, local, queries) = both(filter);
        assert_eq!(folded, local, "{filter}");
        assert!(queries[0].filter.is_empty(), "{filter}: folded");
    }
    let (t, _, q) = both("Table.SelectRows(Table.FirstN($T, 2), each [id] = 2)");
    assert_eq!(t.row_count(), 1);
    assert_eq!(q[0].limit, Some(2));
}

#[test]
fn constants_come_from_the_environment() {
    let (mut opts, log) = fake(None);
    opts.shared.push((
        "Since".into(),
        "#date(2026, 1, 1) meta [IsParameterQuery=true]".into(),
    ));
    let e = template(
        "orders",
        "Result = Table.SelectRows(TableSource, each [day] >= Since)",
    );
    let t = load_m_with(&e, &opts).unwrap();
    assert_eq!(t.row_count(), 2);
    let q = log.lock().unwrap().queries.clone();
    assert_eq!(
        q[0].filter,
        vec![Pred::Compare {
            column: "day".into(),
            op: CmpOp::Ge,
            value: Literal::Date {
                year: 2026,
                month: 1,
                day: 1
            }
        }]
    );
}

#[test]
fn column_names_are_case_sensitive_as_in_m() {
    let (opts, _) = fake(None);
    let e = template("orders", "Result = Table.SelectColumns(TableSource, {\"ID\"})");
    let r = load_m_with(&e, &opts);
    assert!(
        matches!(&r, Err(Error::Evaluation { reason, message }) if reason == "Expression.Error" && message.contains("'ID'")),
        "{r:?}"
    );
}

#[test]
fn navigation_tables_behave_like_tables() {
    let (opts, _) = fake(None);
    let e = "let S = Fake.Catalogs(\"h\", \"p\"), C = S{[Name = \"main\"]}[Data], Sc = C{0}[Data] in \
             Table.SelectColumns(Sc, {\"Name\", \"Kind\"})";
    let t = load_m_with(e, &opts).unwrap();
    assert_eq!(
        t.columns()[0].values,
        vec![Text("orders".into()), Text("v_orders".into())]
    );
    assert_eq!(
        t.columns()[1].values,
        vec![Text("Table".into()), Text("View".into())]
    );
    // Row count of a remote table fetches it.
    let e = template(
        "orders",
        "Result = #table({\"n\"}, {{Table.RowCount(TableSource)}})",
    );
    assert_eq!(load_m_with(&e, &opts).unwrap().columns()[0].values, vec![Int(3)]);
}

#[test]
fn connector_errors() {
    // A source error is a catchable DataSource.Error.
    let (opts, _) = fake(None);
    let e = "let S = Fake.Catalogs(\"h\", \"p\"), T = S{[Name = \"main\", Kind = \"Database\"]}[Data]{[Name = \"sales\", Kind = \"Schema\"]}[Data], \
             R = try Table.RowCount(Table.SelectRows(T, each [Name] = \"orders\"){0}[Data]) otherwise -1 in #table({\"n\"}, {{R}})";
    assert_eq!(load_m_with(e, &opts).unwrap().columns()[0].values, vec![Int(3)]);
    let (opts, _) = fake(Some(ConnectErrorKind::DataSource));
    let e = template("orders", "Result = TableSource");
    let r = load_m_with(&e, &opts);
    assert!(
        matches!(&r, Err(Error::Evaluation { reason, .. }) if reason == "DataSource.Error"),
        "{r:?}"
    );
    let e2 = "let S = Fake.Catalogs(\"h\", \"p\"), R = try Table.RowCount(S{0}[Data]) otherwise -1 in #table({\"n\"}, {{R}})";
    assert_eq!(load_m_with(e2, &opts).unwrap().columns()[0].values, vec![Int(-1)]);
    // Credentials and network failures are fatal: `try` does not turn them into fallback data.
    for kind in [ConnectErrorKind::Credentials, ConnectErrorKind::Network] {
        let (opts, _) = fake(Some(kind));
        let r = load_m_with(e2, &opts);
        assert!(
            matches!(&r, Err(Error::Connector(m)) if m == "listing failed"),
            "{r:?}"
        );
    }
    let (opts, _) = fake(Some(ConnectErrorKind::Limit));
    assert!(matches!(load_m_with(e2, &opts), Err(Error::LimitExceeded(_))));
}

#[test]
fn connectors_run_only_when_registered() {
    // Not registered: still an external source, never run.
    let r = load_m(
        "let S = DatabricksMultiCloud.Catalogs(\"h\", \"p\", []) in S",
        &Limits::default(),
    );
    assert!(matches!(r, Err(Error::ExternalSource(_))), "{r:?}");
    // A registered connector does not enable another family.
    let (opts, _) = fake(None);
    let r = load_m_with(
        "let S = DatabricksMultiCloud.Catalogs(\"h\", \"p\", []) in S",
        &opts,
    );
    assert!(matches!(r, Err(Error::ExternalSource(_))), "{r:?}");
}

#[test]
fn arguments_reach_the_connector() {
    let (opts, log) = fake(None);
    let _ = load_m_with(
        "let S = Fake.Catalogs(\"h\", \"/p\", [A = 1, B = null]) in S",
        &opts,
    );
    let calls = log.lock().unwrap().calls.clone();
    assert!(
        calls[0].contains("[Text(\"h\"), Text(\"/p\"), Record([(\"A\", Number(1.0)), (\"B\", Null)])]"),
        "{calls:?}"
    );
}

#[test]
fn utc_clock_only_when_given() {
    let e = "#table(type table [T = datetimezone], {{DateTimeZone.UtcNow()}})";
    assert!(matches!(
        load_m(e, &Limits::default()),
        Err(Error::UnsupportedSource(_))
    ));
    let opts = SourceOptions {
        clock_utc_ms: Some(46_027 * 86_400_000 + 3_600_000),
        ..SourceOptions::default()
    };
    let t = load_m_with(e, &opts).unwrap();
    assert_eq!(
        t.columns()[0].values,
        vec![CellValue::DateTime(46_027.0 + 1.0 / 24.0)]
    );
    let local = "#table(type table [T = datetime], {{DateTime.LocalNow()}})";
    assert!(matches!(
        load_m_with(local, &opts),
        Err(Error::UnsupportedSource(_))
    ));
    // with an offset: the UTC clock moved by it (`datetime`), or with the zone
    let opts = SourceOptions {
        clock_local_offset_minutes: Some(60),
        ..opts
    };
    let t = load_m_with(local, &opts).unwrap();
    assert_eq!(
        t.columns()[0].values,
        vec![CellValue::DateTime(46_027.0 + 2.0 / 24.0)]
    );
    let zoned = "#table(type table [T = datetimezone], {{DateTimeZone.LocalNow()}})";
    let t = load_m_with(zoned, &opts).unwrap();
    assert_eq!(
        t.columns()[0].values,
        vec![CellValue::DateTime(46_027.0 + 2.0 / 24.0)],
        "a datetimezone loads as its local time, offset dropped"
    );
}

// Folding indicators, View native query and diagnostics (`folding.rs`).

fn probe_at(steps: &str) -> folding::Probe {
    let (opts, _) = fake(None);
    folding::probe(&template("orders", steps), &opts).unwrap()
}

#[test]
fn a_folded_step_shows_its_native_query_and_reads_no_rows() {
    let (opts, log) = fake(None);
    let e = template(
        "orders",
        "Kept = Table.SelectColumns(TableSource, {\"id\", \"amount\"}),\n    \
         Result = Table.FirstN(Kept, 2)",
    );
    let p = folding::probe(&e, &opts).unwrap();
    assert_eq!(p.folding, folding::Folding::Folded);
    assert_eq!(p.connector.as_deref(), Some("fake"));
    assert_eq!(
        p.native.as_deref(),
        Some("SELECT id, amount FROM main.sales.orders LIMIT 2")
    );
    assert!(log.lock().unwrap().queries.is_empty(), "no rows were read");
}

#[test]
fn a_step_that_needs_the_rows_is_partially_folded() {
    let p = probe_at(
        "Kept = Table.SelectColumns(TableSource, {\"id\", \"amount\"}),\n    \
         Result = Table.RenameColumns(Kept, {{\"id\", \"Id\"}})",
    );
    assert_eq!(p.folding, folding::Folding::Partial);
    assert_eq!(p.native, None);
    assert_eq!(p.connector.as_deref(), Some("fake"));
    // a navigation table folds, with no native query yet
    let (opts, _) = fake(None);
    let nav = folding::probe("Fake.Catalogs(\"host\", \"/sql/1.0/warehouses/abc\")", &opts).unwrap();
    assert_eq!(nav.folding, folding::Folding::Folded);
    assert_eq!(nav.native, None);
    assert!(nav.reason.unwrap().contains("navigation"));
}

#[test]
fn a_query_without_a_folding_source_is_not_folded_and_errors_are_unknown() {
    let (opts, _) = fake(None);
    let p = folding::probe("#table({\"a\"}, {{1}})", &opts).unwrap();
    assert_eq!(p.folding, folding::Folding::NotFolded);
    assert_eq!(p.connector, None);
    assert!(folding::probe("error \"boom\"", &opts).is_err());
    // without the connector: not evaluated, so unknown
    assert!(
        folding::probe(
            &template("orders", "Result = TableSource"),
            &SourceOptions::default()
        )
        .is_err()
    );
}

#[test]
fn diagnose_keeps_the_source_requests() {
    let (opts, _) = fake(None);
    let e = template(
        "orders",
        "Kept = Table.SelectRows(TableSource, each [id] > 1),\n    \
         Result = Table.RowCount(Kept)",
    );
    // a number: the evaluation's error, the requests still kept
    let d = folding::diagnose(&e, &opts).unwrap();
    assert!(d.result.is_err());
    let kinds: Vec<&str> = d.requests.iter().map(|r| r.kind).collect();
    assert_eq!(kinds, ["list", "query"], "{:?}", d.requests);
    let q = &d.requests[1];
    assert_eq!(q.connector, "fake");
    assert_eq!(q.rows, 2);
    assert!(
        q.text.starts_with("SELECT * FROM main.sales.orders WHERE"),
        "{}",
        q.text
    );
    let d = folding::diagnose(
        &template("orders", "Result = Table.FirstN(TableSource, 1)"),
        &opts,
    )
    .unwrap();
    assert_eq!(d.result.unwrap().columns()[0].values.len(), 1);
}
