//! Runtime values of the M evaluator.

use std::rc::Rc;

use super::ast::{Expr, Param};
use super::decimal::Dec;
use super::eval::Env;

/// An M number: a double, or an exact decimal (literals, parsed text and the functions that
/// keep them; see `docs/formats/data-sources.md`, *Numbers*). Arithmetic operators,
/// comparisons and most functions work on the double.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Num {
    F(f64),
    D(Dec),
}

impl Num {
    /// The number as a double (decimals convert the way .NET Framework does).
    pub fn f(self) -> f64 {
        match self {
            Num::F(x) => x,
            Num::D(d) => d.to_f64(),
        }
    }

    /// The number as a decimal: decimals as they are, doubles converted (15 significant
    /// digits). `None` for NaN, infinities and doubles beyond the decimal range.
    pub fn dec(self) -> Option<Dec> {
        match self {
            Num::F(x) => Dec::from_f64(x),
            Num::D(d) => Some(d),
        }
    }

    /// A number written as text, as Power Query reads literals, number text, JSON and Excel
    /// cells: a decimal when it has more than 15 significant digits or more than 15 decimal
    /// places and fits a decimal (`123456789012345678`, `0.1234567890123456789`, `1e-16`;
    /// extra digits rounded), else a double (`2.50`, `1e-15`, `1e30`).
    pub fn parse(s: &str) -> Option<Num> {
        match Dec::parse(s) {
            Some(d) => Some(Num::classify(d, || s.parse::<f64>().ok())),
            None => s.parse::<f64>().ok().map(Num::F),
        }
    }

    /// An exact value: a decimal when it needs more than 15 digits (see [`Num::parse`]), else
    /// the double `as_f64` gives (the decimal's own conversion when it gives none).
    pub fn classify(d: Dec, as_f64: impl FnOnce() -> Option<f64>) -> Num {
        if d.significant_digits() > 15 || d.scale() > 15 {
            Num::D(d)
        } else {
            Num::F(as_f64().unwrap_or_else(|| d.to_f64()))
        }
    }

    /// An integer: a double up to 15 digits, else a decimal when it fits.
    pub fn int(i: i128) -> Num {
        match Dec::from_i128(i) {
            Some(d) => Num::classify(d, || Some(i as f64)),
            None => Num::F(i as f64),
        }
    }

    pub fn neg(self) -> Num {
        match self {
            Num::F(x) => Num::F(-x),
            Num::D(d) => Num::D(d.neg()),
        }
    }
}

impl From<f64> for Num {
    fn from(x: f64) -> Self {
        Num::F(x)
    }
}

/// A catchable M error (`try` sees it as a record `[Reason, Message, Detail]`).
#[derive(Debug, Clone)]
pub(crate) struct MError {
    pub reason: String,
    pub message: String,
    pub detail: Value,
}

/// Why an evaluation stopped: a catchable M error, or a fatal condition
/// (unsupported function, cap hit, sandbox violation) that `try` does not catch.
#[derive(Debug, Clone)]
pub(crate) enum Flow {
    Err(Rc<MError>),
    Fatal(crate::Error),
}

pub(crate) type Res<T> = Result<T, Flow>;

pub(crate) fn err<T>(reason: &str, message: impl Into<String>) -> Res<T> {
    Err(Flow::Err(Rc::new(MError {
        reason: reason.into(),
        message: message.into(),
        detail: Value::Null,
    })))
}

pub(crate) fn expr_err<T>(message: impl Into<String>) -> Res<T> {
    err("Expression.Error", message)
}

pub(crate) fn fatal<T>(e: crate::Error) -> Res<T> {
    Err(Flow::Fatal(e))
}

pub(crate) fn unsupported<T>(what: impl std::fmt::Display) -> Res<T> {
    fatal(crate::Error::UnsupportedSource(what.to_string()))
}

/// Primitive type kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Any,
    AnyNonNull,
    None,
    Null,
    Logical,
    Number,
    Text,
    Date,
    DateTime,
    DateTimeZone,
    Time,
    Duration,
    Binary,
    List,
    Record,
    Table,
    Function,
    Type,
}

/// Number facets carried by the library type names (`Int64.Type`, `Currency.Type`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Facet {
    None,
    Int64,
    Int32,
    Int16,
    Int8,
    Byte,
    Double,
    Single,
    Decimal,
    Currency,
    Percentage,
}

/// Record fields or table columns of a type: names and types, in order.
pub(crate) type Fields = Rc<Vec<(Rc<str>, MType)>>;

/// An M type value.
#[derive(Debug, Clone)]
pub(crate) struct MType {
    pub kind: Kind,
    pub nullable: bool,
    pub facet: Facet,
    /// Record fields or table columns.
    pub fields: Option<Fields>,
}

impl MType {
    pub fn prim(kind: Kind) -> Self {
        Self {
            kind,
            nullable: matches!(kind, Kind::Any | Kind::Null),
            facet: Facet::None,
            fields: None,
        }
    }

    pub fn any() -> Self {
        Self::prim(Kind::Any)
    }

    pub fn facet(kind: Kind, facet: Facet) -> Self {
        Self {
            facet,
            ..Self::prim(kind)
        }
    }

    pub fn nullable(mut self) -> Self {
        self.nullable = true;
        self
    }

    pub fn table(cols: Vec<(Rc<str>, MType)>) -> Self {
        Self {
            fields: Some(Rc::new(cols)),
            ..Self::prim(Kind::Table)
        }
    }

    pub fn name(&self) -> &'static str {
        match self.kind {
            Kind::Any => "any",
            Kind::AnyNonNull => "anynonnull",
            Kind::None => "none",
            Kind::Null => "null",
            Kind::Logical => "logical",
            Kind::Number => "number",
            Kind::Text => "text",
            Kind::Date => "date",
            Kind::DateTime => "datetime",
            Kind::DateTimeZone => "datetimezone",
            Kind::Time => "time",
            Kind::Duration => "duration",
            Kind::Binary => "binary",
            Kind::List => "list",
            Kind::Record => "record",
            Kind::Table => "table",
            Kind::Function => "function",
            Kind::Type => "type",
        }
    }
}

/// A record: ordered fields.
#[derive(Debug, Clone)]
pub(crate) struct Record {
    pub names: Rc<Vec<Rc<str>>>,
    pub values: Vec<Value>,
}

impl Record {
    pub fn new(fields: Vec<(Rc<str>, Value)>) -> Self {
        let (names, values): (Vec<_>, Vec<_>) = fields.into_iter().unzip();
        Self {
            names: Rc::new(names),
            values,
        }
    }

    pub fn index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| &**n == name)
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.index(name).and_then(|i| self.values.get(i))
    }
}

/// One table column.
#[derive(Debug, Clone)]
pub(crate) struct Column {
    pub name: Rc<str>,
    pub ty: MType,
}

/// A table: typed columns and rows of values.
#[derive(Debug, Clone)]
pub(crate) struct Table {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
}

impl Table {
    pub fn index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| &*c.name == name)
    }

    pub fn names(&self) -> Rc<Vec<Rc<str>>> {
        Rc::new(self.columns.iter().map(|c| c.name.clone()).collect())
    }

    pub fn ty(&self) -> MType {
        MType::table(
            self.columns
                .iter()
                .map(|c| (c.name.clone(), c.ty.clone()))
                .collect(),
        )
    }
}

/// Library functions implemented in Rust that carry data (returned by
/// `Splitter.*`, `Combiner.*`, `Comparer.*`, …).
#[derive(Debug, Clone)]
pub(crate) enum Native {
    /// `Splitter.SplitTextByDelimiter(delimiter, quoteStyle)`.
    SplitByDelimiter(Rc<str>, bool),
    /// `Splitter.SplitTextByAnyDelimiter(delimiters, quoteStyle)`.
    SplitByAnyDelimiter(Rc<Vec<Rc<str>>>, bool),
    /// `Splitter.SplitByNothing()`.
    SplitByNothing,
    /// `Splitter.SplitTextByPositions(positions)`.
    SplitByPositions(Rc<Vec<usize>>),
    /// `Splitter.SplitTextByLengths(lengths)`.
    SplitByLengths(Rc<Vec<usize>>),
    /// `Splitter.SplitTextByEachDelimiter(delimiters, quoteStyle, startAtEnd)`.
    SplitByEachDelimiter(Rc<Vec<Rc<str>>>, bool, bool),
    /// `Splitter.SplitTextByRanges(ranges, startAtEnd)`: (offset, count) pairs.
    SplitByRanges(Rc<Vec<(usize, usize)>>, bool),
    /// `Splitter.SplitTextByRepeatedLengths(length, startAtEnd)`.
    SplitByRepeatedLengths(usize, bool),
    /// `Splitter.SplitTextByWhitespace(quoteStyle)`.
    SplitByWhitespace(bool),
    /// `Splitter.SplitTextByCharacterTransition(before, after)`.
    SplitByTransition(Rc<(super::lib::misc_more::CharTest, super::lib::misc_more::CharTest)>),
    /// `Combiner.CombineTextByDelimiter(delimiter)`.
    CombineByDelimiter(Rc<str>),
    /// `Comparer.FromCulture(culture, ignoreCase)` / `Comparer.Ordinal` / `Comparer.OrdinalIgnoreCase`.
    Comparer { ignore_case: bool },
}

/// A function value.
#[derive(Debug)]
pub(crate) enum Func {
    Closure {
        params: Rc<Vec<Param>>,
        body: Rc<Expr>,
        env: Env,
    },
    Builtin(&'static super::lib::Builtin),
    Native(Native),
    /// A function of an allowed connector (`crate::connect`), by its M name.
    Connector(Rc<str>),
}

/// A file `Folder.Files` / `Folder.Contents` lists: read only when its content is used.
#[derive(Debug)]
pub(crate) struct LazyFile {
    /// The resolved path (checked to lie under an allowed root when listed).
    pub path: std::path::PathBuf,
    /// Size cap (`Limits::max_bytes`).
    pub max: usize,
}

/// The bytes of a `binary` value: in memory, a sandboxed file read on first use, or a
/// connector's content read on first use (`crate::connect::RemoteValue::Content`).
#[derive(Debug)]
pub(crate) struct Bin {
    bytes: std::cell::OnceCell<Rc<Vec<u8>>>,
    file: Option<LazyFile>,
    remote: Option<(std::sync::Arc<dyn crate::connect::LazyContent>, usize)>,
}

impl Bin {
    pub fn file(file: LazyFile) -> Self {
        Self {
            bytes: std::cell::OnceCell::new(),
            file: Some(file),
            remote: None,
        }
    }

    /// A connector's content, read the first time it is used, at most `max` bytes.
    pub fn remote(content: std::sync::Arc<dyn crate::connect::LazyContent>, max: usize) -> Self {
        Self {
            bytes: std::cell::OnceCell::new(),
            file: None,
            remote: Some((content, max)),
        }
    }

    /// The bytes, reading the file the first time. A file that cannot be read is a catchable
    /// `DataSource.Error`; one over the size cap is fatal.
    pub fn get(&self) -> Res<Rc<Vec<u8>>> {
        if let Some(b) = self.bytes.get() {
            return Ok(Rc::clone(b));
        }
        if let Some((content, max)) = &self.remote {
            let data = content.read(*max).map_err(super::remote::flow)?;
            if data.len() > *max {
                return fatal(crate::Error::LimitExceeded(format!(
                    "a connector's content is larger than {max} bytes"
                )));
            }
            return Ok(Rc::clone(self.bytes.get_or_init(|| Rc::new(data))));
        }
        let Some(f) = &self.file else {
            return Ok(Rc::new(Vec::new()));
        };
        let shown = f.path.display().to_string();
        let too_large = || {
            fatal(crate::Error::LimitExceeded(format!(
                "file `{shown}` is larger than {} bytes",
                f.max
            )))
        };
        match std::fs::metadata(&f.path) {
            Ok(m) if m.is_file() && m.len() > f.max as u64 => return too_large(),
            Ok(m) if m.is_file() => {}
            Ok(_) => return err("DataSource.Error", format!("'{shown}' is not a file.")),
            Err(e) => return err("DataSource.Error", format!("Could not read file '{shown}': {e}.")),
        }
        let data = match std::fs::read(&f.path) {
            Ok(d) if d.len() <= f.max => Rc::new(d),
            Ok(_) => return too_large(),
            Err(e) => return err("DataSource.Error", format!("Could not read file '{shown}': {e}.")),
        };
        Ok(Rc::clone(self.bytes.get_or_init(|| data)))
    }
}

/// An M value.
#[derive(Debug, Clone)]
pub(crate) enum Value {
    Null,
    Logical(bool),
    Number(Num),
    Text(Rc<str>),
    /// Days since 1899-12-30.
    Date(i64),
    /// Milliseconds since 1899-12-30T00:00.
    DateTime(i64),
    /// Local milliseconds since 1899-12-30T00:00 and the offset in minutes.
    DateTimeZone(i64, i32),
    /// Milliseconds since midnight.
    Time(i64),
    /// Milliseconds.
    Duration(i64),
    Binary(Rc<Bin>),
    List(Rc<Vec<Value>>),
    Record(Rc<Record>),
    Table(Rc<Table>),
    Function(Rc<Func>),
    Type(Rc<MType>),
    /// An error stored inside a list, record or table; reading it raises it.
    Error(Rc<MError>),
    /// A table or navigation table of an allowed connector, fetched on first use (`remote.rs`).
    Remote(Rc<super::remote::Remote>),
}

impl Value {
    /// A double.
    pub fn num(x: f64) -> Self {
        Self::Number(Num::F(x))
    }

    /// A decimal.
    pub fn dec(d: Dec) -> Self {
        Self::Number(Num::D(d))
    }

    /// A binary value holding `bytes`.
    pub fn bytes(bytes: Vec<u8>) -> Self {
        let cell = std::cell::OnceCell::new();
        let _ = cell.set(Rc::new(bytes));
        Self::Binary(Rc::new(Bin {
            bytes: cell,
            file: None,
            remote: None,
        }))
    }

    pub fn text(s: impl Into<Rc<str>>) -> Self {
        Self::Text(s.into())
    }

    pub fn list(v: Vec<Value>) -> Self {
        Self::List(Rc::new(v))
    }

    pub fn record(fields: Vec<(&str, Value)>) -> Self {
        Self::Record(Rc::new(Record::new(
            fields.into_iter().map(|(n, v)| (Rc::from(n), v)).collect(),
        )))
    }

    pub fn table(t: Table) -> Self {
        Self::Table(Rc::new(t))
    }

    pub fn ty(t: MType) -> Self {
        Self::Type(Rc::new(t))
    }

    /// Raises a stored error; any other value passes through.
    pub fn check(self) -> Res<Value> {
        match self {
            Value::Error(e) => Err(Flow::Err(e)),
            v => Ok(v),
        }
    }

    /// Stores a result inside a container: catchable errors become error values.
    pub fn store(r: Res<Value>) -> Res<Value> {
        match r {
            Ok(v) => Ok(v),
            Err(Flow::Err(e)) => Ok(Value::Error(e)),
            Err(f) => Err(f),
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Value::Null => Kind::Null,
            Value::Logical(_) => Kind::Logical,
            Value::Number(_) => Kind::Number,
            Value::Text(_) => Kind::Text,
            Value::Date(_) => Kind::Date,
            Value::DateTime(_) => Kind::DateTime,
            Value::DateTimeZone(..) => Kind::DateTimeZone,
            Value::Time(_) => Kind::Time,
            Value::Duration(_) => Kind::Duration,
            Value::Binary(_) => Kind::Binary,
            Value::List(_) => Kind::List,
            Value::Record(_) => Kind::Record,
            Value::Table(_) | Value::Remote(_) => Kind::Table,
            Value::Function(_) => Kind::Function,
            Value::Type(_) => Kind::Type,
            Value::Error(_) => Kind::None,
        }
    }

    pub fn type_name(&self) -> &'static str {
        MType::prim(self.kind()).name()
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}
