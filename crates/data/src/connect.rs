//! External connectors the M evaluator may run, only when the caller registers them
//! ([`SourceOptions::connectors`](crate::SourceOptions)).
//!
//! Without a registered connector every connector function (`Sql.Database`,
//! `DatabricksMultiCloud.Catalogs`, …) is [`Error::ExternalSource`](crate::Error) and nothing
//! reaches the network; that stays the default. A connector implements one family of M
//! functions (Databricks: `DatabricksMultiCloud.Catalogs`, `Databricks.Catalogs`) over a
//! navigation tree (catalogs → schemas → tables) and answers the queries the evaluator folds into
//! a [`RemoteQuery`]: a column selection, a row filter in M semantics ([`Pred`]), a sort (when
//! the source says it can, [`Source::folds_sort`]) and a row limit. Everything else runs locally
//! in the evaluator over the fetched rows.
//!
//! A connector function may also return a binary value instead ([`Connector::returns_binary`]:
//! `Web.Contents`), which the evaluator's own readers (`Json.Document`, `Csv.Document`, …) then
//! read.
//!
//! This crate never talks to the network itself: the implementations live in higher layers
//! (`bicraft-databricks`, `bicraft-sqlserver`, `bicraft-webdata`). The types here are plain data so the evaluator, which is
//! single-threaded and uses `Rc` values, only sees `Send` connectors through this interface.
//! See `docs/formats/data-sources.md`, *Connectors*.

use std::fmt;
use std::sync::Arc;

/// A connector family the caller explicitly allowed (`--allow-connector databricks`).
pub trait Connector: Send + Sync {
    /// The name the caller allows it by (`databricks`).
    fn name(&self) -> &'static str;
    /// Whether the M function `function` (`DatabricksMultiCloud.Catalogs`) is this connector's.
    fn handles(&self, function: &str) -> bool;
    /// One call of `function` with its evaluated arguments: a navigation root. No network
    /// access is needed here (arguments are checked, the connection is opened lazily).
    fn open(&self, function: &str, args: &[Arg]) -> Result<Box<dyn Source>, ConnectError>;
    /// Whether the M function `function` returns a binary value (`Web.Contents`) rather than a
    /// navigation root: the evaluator then calls [`Connector::contents`] instead of `open`.
    fn returns_binary(&self, function: &str) -> bool {
        let _ = function;
        false
    }
    /// The bytes one call of a binary function returns, at most `max_bytes` (more is
    /// [`ConnectErrorKind::Limit`]).
    fn contents(&self, function: &str, args: &[Arg], max_bytes: usize) -> Result<Vec<u8>, ConnectError> {
        let _ = (args, max_bytes);
        Err(ConnectError::new(
            ConnectErrorKind::Unsupported,
            format!("`{function}` does not return a binary value"),
        ))
    }
}

/// The navigation tree and queries behind one connector call. Used from one evaluation thread.
pub trait Source {
    /// The child of the node at `path` (names from the root) named `name`, when the connector
    /// can name it without listing the node (`{[Name = "main", Kind = "Database"]}`). `Ok(None)`:
    /// the evaluator lists the node ([`Source::list`]) and looks the key up there.
    fn lookup(
        &mut self,
        path: &[String],
        name: &str,
        kind: Option<&str>,
    ) -> Result<Option<NavEntry>, ConnectError>;
    /// The children of the node at `path`.
    fn list(&mut self, path: &[String]) -> Result<Vec<NavEntry>, ConnectError>;
    /// The columns of the table at `path` (for folding decisions), in table order.
    fn columns(&mut self, path: &[String]) -> Result<Vec<RemoteColumn>, ConnectError>;
    /// Runs a query; at most `max_rows` rows (more is [`ConnectErrorKind::Limit`]).
    fn fetch(&mut self, query: &RemoteQuery, max_rows: usize) -> Result<RemoteRows, ConnectError>;
    /// Whether [`RemoteQuery::sort`] may be set (the evaluator folds `Table.Sort` only then).
    fn folds_sort(&self) -> bool {
        false
    }
    /// Whether the connector call itself is a table (a native query, `Sql.Database(…, [Query =
    /// …])`) rather than a navigation table: its rows are [`Source::fetch`] of an empty `table`
    /// path, and nothing folds into it.
    fn root_is_table(&self) -> bool {
        false
    }
    /// The text [`Source::fetch`] would send for `query` (a SQL statement, a URL), without
    /// running it: Power Query's *View native query*. Looking a table up may still read the
    /// source's metadata. `Ok(None)`: the connector cannot show it.
    fn native(&mut self, query: &RemoteQuery) -> Result<Option<String>, ConnectError> {
        let _ = query;
        Ok(None)
    }
}

/// The connectors an evaluation may run, in the order they were registered.
#[derive(Clone, Default)]
pub struct Connectors(Vec<Arc<dyn Connector>>);

impl Connectors {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, c: Arc<dyn Connector>) {
        self.0.push(c);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The connector that implements `function`.
    pub fn find(&self, function: &str) -> Option<&Arc<dyn Connector>> {
        self.0.iter().find(|c| c.handles(function))
    }

    /// The registered connector names.
    pub fn names(&self) -> Vec<&'static str> {
        self.0.iter().map(|c| c.name()).collect()
    }
}

impl fmt::Debug for Connectors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.names()).finish()
    }
}

impl PartialEq for Connectors {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len() && self.0.iter().zip(&other.0).all(|(a, b)| Arc::ptr_eq(a, b))
    }
}

/// An evaluated argument of a connector function.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Null,
    Logical(bool),
    Number(f64),
    /// A duration, in milliseconds.
    Duration(i64),
    Text(String),
    /// The bytes of a binary value (a request body).
    Binary(Vec<u8>),
    /// Field names and values, in order (an options record).
    Record(Vec<(String, Arg)>),
    List(Vec<Arg>),
    /// Any other value, by its M type name.
    Other(&'static str),
}

/// One row of a navigation table: the `Name` and `Kind` columns and whether `Data` is a table of
/// rows (a leaf) or another navigation table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavEntry {
    pub name: String,
    pub kind: String,
    pub leaf: bool,
    /// More text columns of the navigation table, between `Data` and `Kind` (`Schema` and `Item`
    /// for SQL Server), in order; the entries of one listing name the same fields.
    pub fields: Vec<(String, String)>,
}

/// The M type a connector gives a column (before any `Table.TransformColumnTypes`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteType {
    Text,
    Logical,
    Int8,
    Int16,
    Int32,
    Int64,
    Single,
    Double,
    /// `Decimal.Type`: exact decimal values.
    Decimal,
    Date,
    DateTime,
    /// `DateTimeZone.Type`.
    DateTimeZone,
    /// `Time.Type`.
    Time,
    Binary,
    /// `Record.Type`: a record of values ([`RemoteValue::Record`]).
    Record,
    /// `Table.Type`: a nested table ([`RemoteValue::Rows`]).
    Table,
    /// `any`: no type (e.g. a column of the SQL type `NULL`).
    Any,
}

/// A column of a remote table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteColumn {
    pub name: String,
    pub ty: RemoteType,
    /// Whether the source compares values of this column exactly as M does (equality and order
    /// of the values M sees): false for text with a non-binary collation, single-precision
    /// floats and the like. Filters are folded only on such columns.
    pub comparable: bool,
}

/// A value of a remote row.
#[derive(Debug, Clone, PartialEq)]
pub enum RemoteValue {
    Null,
    Logical(bool),
    Int(i64),
    Double(f64),
    /// A decimal number as exact text (`-123.4500`).
    Decimal(String),
    Text(String),
    Date {
        year: i32,
        month: u8,
        day: u8,
    },
    /// A date and time without offset (the source's clock, UTC for Databricks).
    DateTime {
        year: i32,
        month: u8,
        day: u8,
        millis: u32,
    },
    /// A local date and time and its offset from UTC, in minutes.
    DateTimeZone {
        year: i32,
        month: u8,
        day: u8,
        millis: u32,
        offset_minutes: i16,
    },
    /// A time of day, in milliseconds since midnight.
    Time {
        millis: u32,
    },
    Binary(Vec<u8>),
    /// A binary value read only when it is used (a file of a SharePoint folder).
    Content(Lazy<dyn LazyContent>),
    /// A table read only when it is used (a subfolder, a list's items): a table of its own that
    /// folds nothing.
    Rows(Lazy<dyn LazyRows>),
    /// A record (field names and values, in order).
    Record(Vec<(String, RemoteValue)>),
}

/// The bytes behind a [`RemoteValue::Content`], read the first time the value is used.
pub trait LazyContent: Send + Sync + fmt::Debug {
    /// At most `max_bytes` (more is [`ConnectErrorKind::Limit`]).
    fn read(&self, max_bytes: usize) -> Result<Vec<u8>, ConnectError>;
}

/// The rows behind a [`RemoteValue::Rows`], read the first time the table is used.
pub trait LazyRows: Send + Sync + fmt::Debug {
    /// At most `max_rows` rows (more is [`ConnectErrorKind::Limit`]).
    fn rows(&self, max_rows: usize) -> Result<RemoteRows, ConnectError>;
}

/// A shared lazy value; two are equal when they are the same value.
#[derive(Debug)]
pub struct Lazy<T: ?Sized>(pub Arc<T>);

impl<T: ?Sized> Clone for Lazy<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: ?Sized> PartialEq for Lazy<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Rows of a remote query.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteRows {
    pub columns: Vec<RemoteColumn>,
    /// Row-major, one value per column.
    pub rows: Vec<Vec<RemoteValue>>,
}

/// A literal a folded filter compares with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Literal {
    Logical(bool),
    /// A whole number (|n| ≤ 2^53, exact in a double).
    Int(i64),
    Text(String),
    Date {
        year: i32,
        month: u8,
        day: u8,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// A row filter, in **M semantics**: a row is kept when the M expression is true.
///
/// - `Compare { Eq }` with a non-null literal: a null cell never matches;
/// - `Compare { Ne }`: a null cell matches (`null <> 1` is true in M);
/// - the order comparisons: a null cell never matches (`null < 1` is null, which
///   `Table.SelectRows` treats as false);
/// - `IsNull` / `NotNull`: `[c] = null` / `[c] <> null`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pred {
    Compare {
        column: String,
        op: CmpOp,
        value: Literal,
    },
    IsNull(String),
    NotNull(String),
    /// Every predicate holds.
    And(Vec<Pred>),
}

/// A key of a folded `Table.Sort`: a column and whether it sorts descending. Nulls come first
/// ascending and last descending, as in M.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortKey {
    pub column: String,
    pub descending: bool,
}

/// What the evaluator asks a connector for: the table at `table` (navigation names from the
/// root), only `columns` (in that order; `None`: all, in table order), the rows where every
/// `filter` holds, ordered by `sort` (empty: the source's order; set only when
/// [`Source::folds_sort`]), at most `limit` rows (taken after the sort).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteQuery {
    pub table: Vec<String>,
    pub columns: Option<Vec<String>>,
    pub filter: Vec<Pred>,
    pub sort: Vec<SortKey>,
    pub limit: Option<u64>,
}

/// How a connector call failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectErrorKind {
    /// No or rejected credentials: fatal (`try` cannot catch it).
    Credentials,
    /// The source refused the query (SQL error, missing table): a catchable M
    /// `DataSource.Error`, as in Desktop.
    DataSource,
    /// The network or the service failed (timeouts, HTTP errors): fatal.
    Network,
    /// A size cap: fatal.
    Limit,
    /// A function, option or argument the connector does not support: fatal.
    Unsupported,
}

/// A connector error. The message never carries a credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectError {
    pub kind: ConnectErrorKind,
    pub message: String,
}

impl ConnectError {
    pub fn new(kind: ConnectErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ConnectError {}
