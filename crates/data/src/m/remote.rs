//! Values of an allowed connector ([`crate::connect`]): lazy navigation tables and remote tables.
//!
//! A connector function call (`DatabricksMultiCloud.Catalogs(…)`) is a navigation root. A key
//! lookup `{[Name = …, Kind = …]}` the connector can answer by itself descends without a query;
//! anything else lists the node into an ordinary table whose `Data` cells are again remote values
//! (so `Table.SelectRows(Schema, each [Name] = "t"){0}[Data]` lists the schema once and keeps the
//! table lazy).
//!
//! A remote table folds, while it has not been fetched:
//! - `Table.SelectColumns` and `Table.RemoveColumns` (missing-field mode `MissingField.Error`)
//!   into the column list;
//! - `Table.SelectRows` whose function is a conjunction of `[column] op constant` comparisons
//!   that the source evaluates exactly as M does ([`crate::connect::Pred`]), before any limit;
//! - `Table.FirstN` with a count into the row limit;
//! - `Table.Sort` by columns (names, `{name, Order.*}` pairs; no comparer, no key function),
//!   before any limit, on columns the source compares as M does and that are not text, when the
//!   source folds sorts ([`Source::folds_sort`]). A later sort puts its keys before the earlier
//!   ones (M's sort is stable). Ties keep the source's order, which a SQL source does not define,
//!   as it does not define the order of an unsorted table.
//!
//! A connector call that is a table itself (a native query, [`Source::root_is_table`]) folds
//! nothing. Any other use fetches the rows once (the result is kept) and continues in the local
//! evaluator, so a query is never approximated: folding only changes where the work is done.

use std::cell::RefCell;
use std::rc::Rc;

use super::ast::{BinOp, Expr, Handler, ListItem, TypeExpr};
use super::decimal::Dec;
use super::eval::{Env, Interp, MS_PER_DAY, whole};
use super::folding::{Folding, Probe, SourceRequest};
use super::value::{
    Bin, Column, Facet, Flow, Func, Kind, MError, MType, Num, Record, Res, Table, Value, err, fatal,
};
use crate::Error;
use crate::connect::{
    Arg, CmpOp, ConnectError, ConnectErrorKind, Literal, NavEntry, Pred, RemoteColumn, RemoteQuery,
    RemoteRows, RemoteType, RemoteValue, SortKey, Source,
};

/// Largest whole number a folded comparison uses (exact in a double).
const MAX_EXACT: f64 = 9_007_199_254_740_992.0;

/// A connector value: a navigation node or a remote table.
pub(crate) struct Remote {
    source: Rc<RefCell<Box<dyn Source>>>,
    /// The connector's family (`Connector::name`).
    connector: &'static str,
    /// Navigation names from the root.
    path: Vec<String>,
    node: Node,
    /// The listing or the rows, once fetched.
    forced: RefCell<Option<Rc<Table>>>,
}

impl std::fmt::Debug for Remote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Remote")
            .field("path", &self.path)
            .field("node", &self.node)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
enum Node {
    Nav,
    Table(Plan),
}

/// What a remote table's query asks for so far.
#[derive(Debug, Clone, Default)]
struct Plan {
    columns: Option<Vec<String>>,
    filter: Vec<Pred>,
    sort: Vec<SortKey>,
    limit: Option<u64>,
    /// A native query: nothing folds.
    fixed: bool,
}

impl Remote {
    fn child(&self, entry: &NavEntry) -> Value {
        let mut path = self.path.clone();
        path.push(entry.name.clone());
        Value::Remote(Rc::new(Remote {
            source: Rc::clone(&self.source),
            connector: self.connector,
            path,
            node: if entry.leaf {
                Node::Table(Plan::default())
            } else {
                Node::Nav
            },
            forced: RefCell::new(None),
        }))
    }

    fn with_plan(&self, plan: Plan) -> Value {
        Value::Remote(Rc::new(Remote {
            source: Rc::clone(&self.source),
            connector: self.connector,
            path: self.path.clone(),
            node: Node::Table(plan),
            forced: RefCell::new(None),
        }))
    }
}

/// A connector error as an M flow: source errors are catchable `DataSource.Error`s, the rest is
/// fatal.
pub(crate) fn flow(e: ConnectError) -> Flow {
    match e.kind {
        ConnectErrorKind::DataSource => Flow::Err(Rc::new(MError {
            reason: "DataSource.Error".into(),
            message: e.message,
            detail: Value::Null,
        })),
        ConnectErrorKind::Limit => Flow::Fatal(Error::LimitExceeded(e.message)),
        ConnectErrorKind::Unsupported => Flow::Fatal(Error::UnsupportedSource(e.message)),
        ConnectErrorKind::Credentials | ConnectErrorKind::Network => Flow::Fatal(Error::Connector(e.message)),
    }
}

fn to_arg(v: &Value) -> Res<Arg> {
    Ok(match v {
        Value::Null => Arg::Null,
        Value::Logical(b) => Arg::Logical(*b),
        Value::Number(n) => Arg::Number(n.f()),
        Value::Text(t) => Arg::Text(t.to_string()),
        Value::Duration(ms) => Arg::Duration(*ms),
        Value::Binary(_) => Arg::Binary(super::lib::misc::binary(v)?.to_vec()),
        Value::Record(r) => Arg::Record(
            r.names
                .iter()
                .zip(r.values.iter())
                .map(|(n, v)| Ok((n.to_string(), to_arg(&v.clone().check()?)?)))
                .collect::<Res<_>>()?,
        ),
        Value::List(l) => Arg::List(
            l.iter()
                .map(|v| to_arg(&v.clone().check()?))
                .collect::<Res<_>>()?,
        ),
        other => Arg::Other(other.type_name()),
    })
}

/// Calls connector function `name`: a navigation root.
pub(crate) fn open(interp: &mut Interp, name: &str, args: &[Value]) -> Res<Value> {
    let Some(connector) = interp.connectors.find(name).cloned() else {
        return fatal(Error::UnsupportedSource(format!(
            "connector `{name}` is not allowed"
        )));
    };
    interp.tick(1)?;
    let mut checked = Vec::with_capacity(args.len());
    for a in args {
        checked.push(to_arg(&a.clone().check()?)?);
    }
    if connector.returns_binary(name) {
        let bytes = connector
            .contents(name, &checked, interp.limits.max_bytes)
            .map_err(flow)?;
        if bytes.len() > interp.limits.max_bytes {
            return fatal(Error::LimitExceeded(format!(
                "`{name}` returned more than {} bytes",
                interp.limits.max_bytes
            )));
        }
        return Ok(Value::bytes(bytes));
    }
    let source = connector.open(name, &checked).map_err(flow)?;
    let node = if source.root_is_table() {
        Node::Table(Plan {
            fixed: true,
            ..Plan::default()
        })
    } else {
        Node::Nav
    };
    Ok(Value::Remote(Rc::new(Remote {
        source: Rc::new(RefCell::new(source)),
        connector: connector.name(),
        path: Vec::new(),
        node,
        forced: RefCell::new(None),
    })))
}

/// Whether a call's arguments hold a connector value (directly or as a list item).
pub(crate) fn has_remote(args: &[Value]) -> bool {
    args.iter().any(|a| match a {
        Value::Remote(_) => true,
        Value::List(l) => l.iter().any(|v| matches!(v, Value::Remote(_))),
        _ => false,
    })
}

/// A connector value fetched into a table; any other value unchanged.
pub(crate) fn force_value(interp: &mut Interp, v: Value) -> Res<Value> {
    match v {
        Value::Remote(r) => Ok(Value::Table(force(interp, &r)?)),
        other => Ok(other),
    }
}

/// Fetches the connector values among a call's arguments (and their list items).
pub(crate) fn force_args(interp: &mut Interp, args: &mut [Value]) -> Res<()> {
    for a in args.iter_mut() {
        match a {
            Value::Remote(r) => {
                let t = force(interp, r)?;
                *a = Value::Table(t);
            }
            Value::List(l) if l.iter().any(|v| matches!(v, Value::Remote(_))) => {
                let mut items = Vec::with_capacity(l.len());
                for v in l.iter() {
                    items.push(force_value(interp, v.clone())?);
                }
                *a = Value::list(items);
            }
            _ => {}
        }
    }
    Ok(())
}

/// The navigation table or the rows of `r` (fetched once).
pub(crate) fn force(interp: &mut Interp, r: &Remote) -> Res<Rc<Table>> {
    if let Some(t) = r.forced.borrow().as_ref() {
        return Ok(Rc::clone(t));
    }
    let t = match &r.node {
        Node::Nav => {
            let started = interp.requests.is_some().then(std::time::Instant::now);
            let entries = r.source.borrow_mut().list(&r.path).map_err(flow)?;
            if let Some(log) = interp.requests.as_mut() {
                log.push(SourceRequest::new(
                    r.connector,
                    "list",
                    if r.path.is_empty() {
                        "(root)".to_string()
                    } else {
                        r.path.join(" › ")
                    },
                    entries.len(),
                    started,
                ));
            }
            interp.check_rows(entries.len())?;
            interp.tick(entries.len() as u64)?;
            let fields = entries.first().map(|e| e.fields.as_slice()).unwrap_or_default();
            let columns = nav_columns(fields);
            let mut rows = Vec::with_capacity(entries.len());
            for e in &entries {
                if e.fields.len() != fields.len()
                    || e.fields.iter().zip(fields).any(|((a, _), (b, _))| a != b)
                {
                    return fatal(Error::Connector(
                        "the source listed entries with different columns".into(),
                    ));
                }
                rows.push(nav_row(r, e));
            }
            Table { columns, rows }
        }
        Node::Table(plan) => {
            if interp.probe {
                // folding indicators never read rows: the query continues in the evaluator
                interp.probe_hit = Some(r.connector);
                return fatal(Error::Connector(
                    "the rows of the source's query are needed (not read for folding indicators)".into(),
                ));
            }
            fetch(interp, r, plan)?
        }
    };
    let t = Rc::new(t);
    *r.forced.borrow_mut() = Some(Rc::clone(&t));
    Ok(t)
}

/// `Name`, `Data`, the entries' own fields, `Kind`.
fn nav_columns(fields: &[(String, String)]) -> Vec<Column> {
    let text = |n: &str| Column {
        name: n.into(),
        ty: MType::prim(Kind::Text),
    };
    let mut out = vec![
        text("Name"),
        Column {
            name: "Data".into(),
            ty: MType::prim(Kind::Table),
        },
    ];
    out.extend(fields.iter().map(|(n, _)| text(n)));
    out.push(text("Kind"));
    out
}

fn nav_row(r: &Remote, e: &NavEntry) -> Vec<Value> {
    let mut row = Vec::with_capacity(3 + e.fields.len());
    row.push(Value::text(e.name.as_str()));
    row.push(r.child(e));
    row.extend(e.fields.iter().map(|(_, v)| Value::text(v.as_str())));
    row.push(Value::text(e.kind.as_str()));
    row
}

/// `node{[Name = …, Kind = …]}` answered by the connector without listing the node.
pub(crate) fn lookup(interp: &mut Interp, r: &Remote, key: &Record) -> Res<Option<Value>> {
    if !matches!(r.node, Node::Nav) || r.forced.borrow().is_some() {
        return Ok(None);
    }
    let mut name = None;
    let mut kind = None;
    for (n, v) in key.names.iter().zip(key.values.iter()) {
        match (&**n, v) {
            ("Name", Value::Text(t)) => name = Some(t.to_string()),
            ("Kind", Value::Text(t)) => kind = Some(t.to_string()),
            _ => return Ok(None),
        }
    }
    let Some(name) = name else {
        return Ok(None);
    };
    interp.tick(1)?;
    let found = r
        .source
        .borrow_mut()
        .lookup(&r.path, &name, kind.as_deref())
        .map_err(flow)?;
    Ok(found.map(|e| {
        let row = nav_row(r, &e);
        let names: Vec<Rc<str>> = nav_columns(&e.fields).into_iter().map(|c| c.name).collect();
        Value::Record(Rc::new(Record {
            names: Rc::new(names),
            values: row,
        }))
    }))
}

fn m_type(t: RemoteType) -> MType {
    let num = |f: Facet| MType::facet(Kind::Number, f).nullable();
    match t {
        RemoteType::Text => MType::prim(Kind::Text).nullable(),
        RemoteType::Logical => MType::prim(Kind::Logical).nullable(),
        RemoteType::Int8 => num(Facet::Int8),
        RemoteType::Int16 => num(Facet::Int16),
        RemoteType::Int32 => num(Facet::Int32),
        RemoteType::Int64 => num(Facet::Int64),
        RemoteType::Single => num(Facet::Single),
        RemoteType::Double => num(Facet::Double),
        RemoteType::Decimal => num(Facet::Decimal),
        RemoteType::Date => MType::prim(Kind::Date).nullable(),
        RemoteType::DateTime => MType::prim(Kind::DateTime).nullable(),
        RemoteType::DateTimeZone => MType::prim(Kind::DateTimeZone).nullable(),
        RemoteType::Time => MType::prim(Kind::Time).nullable(),
        RemoteType::Binary => MType::prim(Kind::Binary).nullable(),
        RemoteType::Record => MType::prim(Kind::Record).nullable(),
        RemoteType::Table => MType::prim(Kind::Table).nullable(),
        RemoteType::Any => MType::any(),
    }
}

fn day(year: i32, month: u8, day: u8) -> Result<i64, String> {
    crate::value::ole_date(i64::from(year), i64::from(month), i64::from(day))
}

fn m_value(interp: &Interp, connector: &'static str, v: RemoteValue) -> Res<Value> {
    Ok(match v {
        RemoteValue::Null => Value::Null,
        RemoteValue::Logical(b) => Value::Logical(b),
        RemoteValue::Int(i) => Value::Number(Num::int(i128::from(i))),
        RemoteValue::Double(x) => Value::num(x),
        RemoteValue::Decimal(text) => match Dec::parse(text.trim()) {
            Some(d) => Value::dec(d),
            None => match text.trim().parse::<f64>() {
                Ok(x) => Value::num(x),
                Err(_) => return err("DataFormat.Error", format!("Invalid decimal value `{text}`.")),
            },
        },
        RemoteValue::Text(t) => {
            interp.check_text(t.len())?;
            Value::text(t)
        }
        RemoteValue::Date { year, month, day: d } => match day(year, month, d) {
            Ok(n) => Value::Date(n),
            Err(e) => return err("DataFormat.Error", e),
        },
        RemoteValue::DateTime {
            year,
            month,
            day: d,
            millis,
        } => match day(year, month, d) {
            Ok(n) if millis < 86_400_000 => Value::DateTime(n * MS_PER_DAY + i64::from(millis)),
            Ok(_) => return err("DataFormat.Error", "Invalid time of day."),
            Err(e) => return err("DataFormat.Error", e),
        },
        RemoteValue::DateTimeZone {
            year,
            month,
            day: d,
            millis,
            offset_minutes,
        } => match day(year, month, d) {
            Ok(n) if millis < 86_400_000 && (-14 * 60..=14 * 60).contains(&offset_minutes) => {
                Value::DateTimeZone(n * MS_PER_DAY + i64::from(millis), i32::from(offset_minutes))
            }
            Ok(_) => return err("DataFormat.Error", "Invalid time of day or offset."),
            Err(e) => return err("DataFormat.Error", e),
        },
        RemoteValue::Time { millis } if millis < 86_400_000 => Value::Time(i64::from(millis)),
        RemoteValue::Time { .. } => return err("DataFormat.Error", "Invalid time of day."),
        RemoteValue::Binary(b) => {
            interp.check_text(b.len())?;
            Value::bytes(b)
        }
        RemoteValue::Content(c) => Value::Binary(Rc::new(Bin::remote(c.0, interp.limits.max_bytes))),
        RemoteValue::Rows(rows) => Value::Remote(Rc::new(Remote {
            source: Rc::new(RefCell::new(Box::new(RowsSource {
                rows: rows.0,
                columns: None,
            }))),
            connector,
            path: Vec::new(),
            node: Node::Table(Plan {
                fixed: true,
                ..Plan::default()
            }),
            forced: RefCell::new(None),
        })),
        RemoteValue::Record(fields) => {
            interp.check_rows(fields.len())?;
            let mut names = Vec::with_capacity(fields.len());
            let mut values = Vec::with_capacity(fields.len());
            for (n, v) in fields {
                names.push(Rc::<str>::from(n.as_str()));
                values.push(Value::store(m_value(interp, connector, v))?);
            }
            Value::Record(Rc::new(Record {
                names: Rc::new(names),
                values,
            }))
        }
    })
}

/// A table a connector reads when it is used ([`RemoteValue::Rows`]): folds nothing.
#[derive(Debug)]
struct RowsSource {
    rows: std::sync::Arc<dyn crate::connect::LazyRows>,
    columns: Option<Vec<RemoteColumn>>,
}

impl Source for RowsSource {
    fn lookup(&mut self, _: &[String], _: &str, _: Option<&str>) -> Result<Option<NavEntry>, ConnectError> {
        Ok(None)
    }

    fn list(&mut self, _: &[String]) -> Result<Vec<NavEntry>, ConnectError> {
        Err(ConnectError::new(
            ConnectErrorKind::Unsupported,
            "a table is not a navigation table",
        ))
    }

    fn columns(&mut self, _: &[String]) -> Result<Vec<RemoteColumn>, ConnectError> {
        if let Some(c) = &self.columns {
            return Ok(c.clone());
        }
        let got = self.rows.rows(usize::MAX)?;
        self.columns = Some(got.columns.clone());
        Ok(got.columns)
    }

    fn fetch(&mut self, _: &RemoteQuery, max_rows: usize) -> Result<RemoteRows, ConnectError> {
        self.rows.rows(max_rows)
    }

    fn root_is_table(&self) -> bool {
        true
    }
}

fn query_of(r: &Remote, plan: &Plan) -> RemoteQuery {
    RemoteQuery {
        table: r.path.clone(),
        columns: plan.columns.clone(),
        filter: plan.filter.clone(),
        sort: plan.sort.clone(),
        limit: plan.limit,
    }
}

/// The folding indicator of a value that is still a connector value: everything up to it goes to
/// the source; a table's query as the source would be sent it.
pub(crate) fn describe(r: &Remote) -> Probe {
    let mut p = Probe {
        folding: Folding::Folded,
        connector: Some(r.connector.to_string()),
        native: None,
        reason: None,
    };
    match &r.node {
        Node::Nav => {
            p.reason = Some("a navigation table: the source lists its items, no query is sent yet".into());
        }
        Node::Table(plan) => match r.source.borrow_mut().native(&query_of(r, plan)) {
            Ok(Some(text)) => p.native = Some(text),
            Ok(None) => p.reason = Some(format!("the {} connector does not show its queries", r.connector)),
            Err(e) => p.reason = Some(e.message),
        },
    }
    p
}

/// Runs a remote table's query and converts the rows.
fn fetch(interp: &mut Interp, r: &Remote, plan: &Plan) -> Res<Table> {
    let q = query_of(r, plan);
    let started = interp.requests.is_some().then(std::time::Instant::now);
    let got = r
        .source
        .borrow_mut()
        .fetch(&q, interp.limits.max_rows)
        .map_err(flow)?;
    if interp.requests.is_some() {
        let mut req = SourceRequest::new(r.connector, "query", String::new(), got.rows.len(), started);
        // the text the source was sent, as View native query shows it
        req.text = match r.source.borrow_mut().native(&q) {
            Ok(Some(t)) => t,
            _ => r.path.join(" › "),
        };
        if let Some(log) = interp.requests.as_mut() {
            log.push(req);
        }
    }
    interp.check_rows(got.rows.len())?;
    if let Some(want) = &plan.columns {
        for (i, w) in want.iter().enumerate() {
            if got.columns.get(i).is_none_or(|c| c.name != *w) {
                return err(
                    "Expression.Error",
                    format!("The column '{w}' of the table wasn't found."),
                );
            }
        }
    }
    if let Err(e) = interp.limits.check_columns(got.columns.len()) {
        return fatal(e);
    }
    let width = got.columns.len();
    let mut rows = Vec::with_capacity(got.rows.len());
    for row in got.rows {
        interp.tick(1)?;
        if row.len() != width {
            return fatal(Error::Connector(format!(
                "the source returned a row of {} values for {width} columns",
                row.len()
            )));
        }
        let mut out = Vec::with_capacity(width);
        for v in row {
            out.push(Value::store(m_value(interp, r.connector, v))?);
        }
        rows.push(out);
    }
    Ok(Table {
        columns: got
            .columns
            .iter()
            .map(|c| Column {
                name: c.name.as_str().into(),
                ty: m_type(c.ty),
            })
            .collect(),
        rows,
    })
}

/// The new value when `name(args)` folds into the remote table `args[0]`; `None`: run it
/// locally over the fetched rows.
pub(crate) fn fold(interp: &mut Interp, name: &str, args: &[Value]) -> Res<Option<Value>> {
    let Some(Value::Remote(r)) = args.first() else {
        return Ok(None);
    };
    let Node::Table(plan) = &r.node else {
        return Ok(None);
    };
    if plan.fixed || r.forced.borrow().is_some() || args.iter().skip(1).any(|a| matches!(a, Value::Remote(_)))
    {
        return Ok(None);
    }
    let next = match name {
        "Table.SelectColumns" => fold_select(plan, args),
        "Table.RemoveColumns" => fold_remove(r, plan, args)?,
        "Table.FirstN" => fold_first(plan, args),
        "Table.SelectRows" => fold_rows(interp, r, plan, args)?,
        "Table.Sort" => fold_sort(r, plan, args)?,
        _ => None,
    };
    Ok(next.map(|p| r.with_plan(p)))
}

/// A column list argument (text or list of texts), without duplicates.
fn names_arg(v: Option<&Value>) -> Option<Vec<String>> {
    let names: Vec<String> = match v? {
        Value::Text(t) => vec![t.to_string()],
        Value::List(l) => l
            .iter()
            .map(|v| match v {
                Value::Text(t) => Some(t.to_string()),
                _ => None,
            })
            .collect::<Option<_>>()?,
        _ => return None,
    };
    let distinct = names
        .iter()
        .enumerate()
        .all(|(i, n)| !names.iter().take(i).any(|m| m == n));
    distinct.then_some(names)
}

/// `MissingField.Error` (the default) only: the other modes need the local evaluator.
fn missing_is_error(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::Number(n)) => n.f() == 0.0,
        _ => false,
    }
}

fn fold_select(plan: &Plan, args: &[Value]) -> Option<Plan> {
    if !missing_is_error(args.get(2)) {
        return None;
    }
    let names = names_arg(args.get(1))?;
    if let Some(cur) = &plan.columns
        && !names.iter().all(|n| cur.contains(n))
    {
        return None;
    }
    Some(Plan {
        columns: Some(names),
        ..plan.clone()
    })
}

/// The visible columns of a remote table: its selection, else the source's columns.
fn visible(r: &Remote, plan: &Plan) -> Res<Vec<RemoteColumn>> {
    let all = r.source.borrow_mut().columns(&r.path).map_err(flow)?;
    Ok(match &plan.columns {
        None => all,
        Some(names) => names
            .iter()
            .filter_map(|n| all.iter().find(|c| c.name == *n).cloned())
            .collect(),
    })
}

fn fold_remove(r: &Remote, plan: &Plan, args: &[Value]) -> Res<Option<Plan>> {
    if !missing_is_error(args.get(2)) {
        return Ok(None);
    }
    let Some(removed) = names_arg(args.get(1)) else {
        return Ok(None);
    };
    let cur: Vec<String> = visible(r, plan)?.into_iter().map(|c| c.name).collect();
    if !removed.iter().all(|n| cur.contains(n)) {
        return Ok(None);
    }
    Ok(Some(Plan {
        columns: Some(cur.into_iter().filter(|c| !removed.contains(c)).collect()),
        ..plan.clone()
    }))
}

fn fold_first(plan: &Plan, args: &[Value]) -> Option<Plan> {
    let Some(Value::Number(n)) = args.get(1) else {
        return None;
    };
    let n = whole(n.f()).and_then(|n| u64::try_from(n).ok())?;
    Some(Plan {
        limit: Some(plan.limit.map_or(n, |l| l.min(n))),
        ..plan.clone()
    })
}

/// The keys of a `Table.Sort` criteria argument naming columns only: `"a"`, `{"a", Order.*}`,
/// `{"a", "b"}`, `{{"a", Order.*}, …}`.
fn sort_keys(v: Option<&Value>) -> Option<Vec<SortKey>> {
    let order = |v: Option<&Value>| match v {
        None | Some(Value::Null) => Some(false),
        Some(Value::Number(n)) if n.f() == 0.0 => Some(false),
        Some(Value::Number(n)) if n.f() == 1.0 => Some(true),
        _ => None,
    };
    let key = |column: &str, descending: bool| SortKey {
        column: column.to_string(),
        descending,
    };
    let one = |x: &Value| -> Option<SortKey> {
        match x {
            Value::Text(t) => Some(key(t, false)),
            Value::List(p) if (1..=2).contains(&p.len()) => match p.first() {
                Some(Value::Text(t)) => Some(key(t, order(p.get(1))?)),
                _ => None,
            },
            _ => None,
        }
    };
    let keys = match v? {
        Value::List(l) if l.len() == 2 && matches!(l.get(1), Some(Value::Number(_))) => vec![one(v?)?],
        Value::List(l) => l.iter().map(one).collect::<Option<Vec<_>>>()?,
        other => vec![one(other)?],
    };
    (!keys.is_empty()).then_some(keys)
}

fn fold_sort(r: &Remote, plan: &Plan, args: &[Value]) -> Res<Option<Plan>> {
    if plan.limit.is_some() || !r.source.borrow().folds_sort() {
        return Ok(None);
    }
    let Some(keys) = sort_keys(args.get(1)) else {
        return Ok(None);
    };
    let columns = visible(r, plan)?;
    let sortable = |k: &SortKey| {
        columns.iter().find(|c| c.name == k.column).is_some_and(|c| {
            c.comparable && !matches!(c.ty, RemoteType::Text | RemoteType::Any | RemoteType::Binary)
        })
    };
    if !keys.iter().all(sortable) {
        return Ok(None);
    }
    // the new keys first, then the earlier ones (a stable sort keeps their order among ties)
    let mut sort: Vec<SortKey> = Vec::with_capacity(keys.len() + plan.sort.len());
    for k in keys.into_iter().chain(plan.sort.iter().cloned()) {
        if !sort.iter().any(|s| s.column == k.column) {
            sort.push(k);
        }
    }
    Ok(Some(Plan { sort, ..plan.clone() }))
}

fn fold_rows(interp: &mut Interp, r: &Remote, plan: &Plan, args: &[Value]) -> Res<Option<Plan>> {
    if plan.limit.is_some() {
        return Ok(None);
    }
    let Some(Value::Function(f)) = args.get(1) else {
        return Ok(None);
    };
    let Func::Closure { params, body, env } = &**f else {
        return Ok(None);
    };
    let [param] = params.as_slice() else {
        return Ok(None);
    };
    let columns = visible(r, plan)?;
    let Some(pred) = predicate(interp, body, &param.name, env, &columns) else {
        return Ok(None);
    };
    let mut filter = plan.filter.clone();
    filter.push(pred);
    Ok(Some(Plan {
        filter,
        ..plan.clone()
    }))
}

/// The filter `e` (the body of `each …`, row parameter `param`) stands for, when every part of it
/// folds exactly.
fn predicate(
    interp: &mut Interp,
    e: &Expr,
    param: &str,
    env: &Env,
    columns: &[RemoteColumn],
) -> Option<Pred> {
    match e {
        Expr::Binary(BinOp::And, a, b) => {
            let a = predicate(interp, a, param, env, columns)?;
            let b = predicate(interp, b, param, env, columns)?;
            Some(Pred::And(vec![a, b]))
        }
        Expr::Binary(op, a, b) => {
            let op = cmp_op(*op)?;
            let (column, other, op) = match (field_of(a, param), field_of(b, param)) {
                (Some(c), None) => (c, b, op),
                (None, Some(c)) => (c, a, flip(op)),
                _ => return None,
            };
            if mentions(other, param) {
                return None;
            }
            let col = columns.iter().find(|c| c.name == column)?;
            let value = interp.eval(other, env).ok()?;
            atom(col, op, &value)
        }
        _ => None,
    }
}

fn cmp_op(op: BinOp) -> Option<CmpOp> {
    Some(match op {
        BinOp::Eq => CmpOp::Eq,
        BinOp::Ne => CmpOp::Ne,
        BinOp::Lt => CmpOp::Lt,
        BinOp::Le => CmpOp::Le,
        BinOp::Gt => CmpOp::Gt,
        BinOp::Ge => CmpOp::Ge,
        _ => return None,
    })
}

fn flip(op: CmpOp) -> CmpOp {
    match op {
        CmpOp::Lt => CmpOp::Gt,
        CmpOp::Le => CmpOp::Ge,
        CmpOp::Gt => CmpOp::Lt,
        CmpOp::Ge => CmpOp::Le,
        o => o,
    }
}

/// `[column]` (that is `param[column]`).
fn field_of<'e>(e: &'e Expr, param: &str) -> Option<&'e str> {
    match e {
        Expr::Field(target, name, false) => match &**target {
            Expr::Ident(p) if &**p == param => Some(name),
            _ => None,
        },
        _ => None,
    }
}

/// One comparison, when the source evaluates it as M does.
fn atom(col: &RemoteColumn, op: CmpOp, value: &Value) -> Option<Pred> {
    let column = col.name.clone();
    let equality = matches!(op, CmpOp::Eq | CmpOp::Ne);
    if value.is_null() {
        return match op {
            CmpOp::Eq => Some(Pred::IsNull(column)),
            CmpOp::Ne => Some(Pred::NotNull(column)),
            _ => None,
        };
    }
    if !col.comparable {
        return None;
    }
    let literal = match (col.ty, value) {
        (RemoteType::Text, Value::Text(t)) if equality => Literal::Text(t.to_string()),
        (RemoteType::Logical, Value::Logical(b)) if equality => Literal::Logical(*b),
        (
            RemoteType::Int8
            | RemoteType::Int16
            | RemoteType::Int32
            | RemoteType::Int64
            | RemoteType::Decimal
            | RemoteType::Double,
            Value::Number(n),
        ) => {
            let x = n.f();
            if x.is_nan() || x.abs() > MAX_EXACT {
                return None;
            }
            Literal::Int(whole(x)?)
        }
        (RemoteType::Date, Value::Date(d)) => {
            let (y, m, dd) = crate::value::civil_from_days(d.checked_add(crate::value::OLE_EPOCH)?);
            Literal::Date {
                year: i32::try_from(y).ok()?,
                month: u8::try_from(m).ok()?,
                day: u8::try_from(dd).ok()?,
            }
        }
        _ => return None,
    };
    Some(Pred::Compare {
        column,
        op,
        value: literal,
    })
}

/// Whether `e` refers to the name `name` anywhere (conservatively: shadowing is ignored).
fn mentions(e: &Expr, name: &str) -> bool {
    let m = |x: &Expr| mentions(x, name);
    match e {
        Expr::Const(_) | Expr::NotImplemented => false,
        Expr::Ident(n) => &**n == name,
        Expr::Let(b, body) => b.iter().any(|(_, x)| m(x)) || m(body),
        Expr::Record(b) => b.iter().any(|(_, x)| m(x)),
        Expr::If(c, t, f) => m(c) || m(t) || m(f),
        Expr::Func(_, body) => m(body),
        Expr::Call(f, args) => m(f) || args.iter().any(m),
        Expr::Field(t, ..) | Expr::Project(t, ..) => m(t),
        Expr::Item(a, b, _) | Expr::Meta(a, b) => m(a) || m(b),
        Expr::List(items) => items.iter().any(|i| match i {
            ListItem::One(x) => m(x),
            ListItem::Range(a, b) => m(a) || m(b),
        }),
        Expr::Unary(_, x) | Expr::Error(x) => m(x),
        Expr::Binary(_, a, b) => m(a) || m(b),
        Expr::Is(x, t) | Expr::As(x, t) => m(x) || type_mentions(t, name),
        Expr::Type(t) => type_mentions(t, name),
        Expr::Try(x, h) => {
            m(x) || match h {
                Handler::None => false,
                Handler::Otherwise(o) => m(o),
                Handler::Catch(_, body) => m(body),
            }
        }
    }
}

fn type_mentions(t: &TypeExpr, name: &str) -> bool {
    match t {
        TypeExpr::Prim(_) => false,
        TypeExpr::Nullable(t) | TypeExpr::List(t) | TypeExpr::Table(t) => type_mentions(t, name),
        TypeExpr::Record(fields) => fields.iter().any(|(_, t)| type_mentions(t, name)),
        TypeExpr::Expr(e) => mentions(e, name),
    }
}
