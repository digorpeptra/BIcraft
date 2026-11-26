//! The fast path of the common CSV query (`docs/formats/data-sources.md`, *Fast CSV loading*):
//!
//! ```text
//! let
//!     Source = Csv.Document(File.Contents("…"), [Delimiter = ",", Columns = 6, Encoding = 65001, QuoteStyle = QuoteStyle.None]),
//!     #"Promoted Headers" = Table.PromoteHeaders(Source, [PromoteAllScalars = true]),
//!     #"Changed Type" = Table.TransformColumnTypes(#"Promoted Headers", {{"Date", type date}, …}, "en-US")
//! in
//!     #"Changed Type"
//! ```
//!
//! (also without the last step or the last two, and as nested calls). Instead of building an M
//! value per cell, the file is streamed in regions, each parsed in parallel chunks by
//! [`crate::csvscan`] with `Csv.Document`'s rules and typed straight into the columns with the
//! conversions `Table.TransformColumnTypes` and the load make (`convert::convert_to`, then
//! `super::cell` and the coercion), with exact fast forms for the usual texts.
//!
//! The result is the evaluator's: same names, types, values, errors as blanks (or the first
//! value error with `strict_errors`), the same row, cell, step and time limits. Anything else
//! (another shape, an option or type this path does not handle, an error the evaluator
//! reports) returns `None` and the evaluator runs the query as before. The file is not capped
//! by [`crate::Limits::max_bytes`] here but by `max_file_bytes` and the memory budget
//! `max_load_bytes`.

use std::io::Read;
use std::path::PathBuf;
use std::rc::Rc;

use super::ast::{Bindings, Expr};
use super::convert::{self, CultureInfo};
use super::eval::{Env, Interp};
use super::lib::{self, opt_number, text};
use super::value::{Column, Facet, Flow, Func, Kind, MType, Num, Value};
use crate::csvscan::{self, Budget, Dialect, Quoting, Rec, Scan, Stop};
use crate::value::{self as cv, Coercion};
use crate::{CellValue, ColumnData, DataType, DateKind, Error, SourceOptions, TableData};

/// Bytes read per region (tiny in tests, so small files exercise the streaming).
#[cfg(not(test))]
const REGION: usize = 64 << 20;
#[cfg(test)]
const REGION: usize = 3001;

/// The recognised query.
struct Plan {
    path: String,
    full: PathBuf,
    size: u64,
    delim: String,
    quoting: Quoting,
    encoding: Option<f64>,
    /// `Columns = n`.
    width: Option<usize>,
    /// `ExtraValues.Error`: a record wider than the table falls back to the evaluator.
    extra_error: bool,
    promote: bool,
    ops: Vec<(String, MType)>,
    culture: CultureInfo,
}

/// Why the fast path stops.
enum Outcome {
    /// The evaluator runs the query.
    Fallback,
    /// The evaluator's error for this query.
    Fail(Error),
}

impl From<Error> for Outcome {
    fn from(e: Error) -> Self {
        Outcome::Fail(e)
    }
}

impl From<Flow> for Outcome {
    fn from(f: Flow) -> Self {
        match f {
            Flow::Fatal(e) => Outcome::Fail(e),
            Flow::Err(_) => Outcome::Fallback,
        }
    }
}

/// Loads `main` by the fast path when it has the shape; `None` otherwise (the caller evaluates
/// it). `interp` is a fresh interpreter for `opts`.
pub(super) fn try_load(
    main: &Expr,
    interp: &mut Interp,
    opts: &SourceOptions,
) -> Option<Result<TableData, Error>> {
    let plan = match plan(main, interp, opts) {
        Ok(p) => p,
        Err(Outcome::Fallback) => return None,
        Err(Outcome::Fail(e)) => return Some(Err(e)),
    };
    match load(&plan, interp, opts) {
        Ok(t) => Some(Ok(t)),
        Err(Outcome::Fallback) => None,
        Err(Outcome::Fail(e)) => Some(Err(e)),
    }
}

/// The steps of the query, read through the `let` bindings without evaluating them.
struct Steps<'e> {
    bindings: Option<&'e Bindings>,
    shared: Vec<Rc<str>>,
    env: Env,
}

impl<'e> Steps<'e> {
    fn bound(&self, name: &str) -> bool {
        self.bindings.is_some_and(|b| b.iter().any(|(n, _)| &**n == name))
            || self.shared.iter().any(|n| &**n == name)
    }

    fn resolve(&self, mut e: &'e Expr) -> &'e Expr {
        for _ in 0..64 {
            let Expr::Ident(n) = e else { break };
            let Some(x) = self.bindings.and_then(|b| b.iter().find(|(name, _)| name == n)) else {
                break;
            };
            e = &x.1;
        }
        e
    }

    /// The arguments of `e` when it calls the library function `name` (by a name no binding
    /// shadows) with an accepted number of arguments.
    fn call(&self, interp: &mut Interp, e: &'e Expr, name: &str) -> Option<&'e [Expr]> {
        let Expr::Call(f, args) = e else { return None };
        let Expr::Ident(n) = &**f else { return None };
        if self.bound(n) {
            return None;
        }
        let Ok(Value::Function(func)) = interp.eval(f, &self.env) else {
            return None;
        };
        match &*func {
            Func::Builtin(b) if b.name == name && args.len() >= b.min && args.len() <= b.max => Some(args),
            _ => None,
        }
    }

    fn eval(&self, interp: &mut Interp, e: Option<&Expr>) -> Result<Value, Outcome> {
        match e {
            None => Ok(Value::Null),
            Some(e) => interp.eval(e, &self.env).map_err(|_| Outcome::Fallback),
        }
    }
}

fn supported(t: &MType) -> bool {
    t.fields.is_none()
        && matches!(
            t.kind,
            Kind::Any
                | Kind::AnyNonNull
                | Kind::Text
                | Kind::Number
                | Kind::Logical
                | Kind::Date
                | Kind::DateTime
                | Kind::Time
                | Kind::Duration
        )
}

fn plan(main: &Expr, interp: &mut Interp, opts: &SourceOptions) -> Result<Plan, Outcome> {
    use Outcome::Fallback;
    let (bindings, body) = match main {
        Expr::Let(b, body) => (Some(b), &**body),
        e => (None, e),
    };
    let env = bindings.map(|b| interp.lazy_frame(b, &None));
    let steps = Steps {
        bindings,
        shared: opts.shared.iter().map(|(n, _)| Rc::from(n.as_str())).collect(),
        env,
    };
    let top = steps.resolve(body);
    let (types_args, after_types) = match steps.call(interp, top, "Table.TransformColumnTypes") {
        Some(a) => (Some(a), steps.resolve(a.first().ok_or(Fallback)?)),
        None => (None, top),
    };
    let (promote_args, after_promote) = match steps.call(interp, after_types, "Table.PromoteHeaders") {
        Some(a) => (Some(a), steps.resolve(a.first().ok_or(Fallback)?)),
        None => (None, after_types),
    };
    let csv_args = steps
        .call(interp, after_promote, "Csv.Document")
        .ok_or(Fallback)?;
    let content = steps.resolve(csv_args.first().ok_or(Fallback)?);
    let fc_args = steps.call(interp, content, "File.Contents").ok_or(Fallback)?;
    if fc_args.len() != 1 {
        return Err(Fallback);
    }
    let path_v = steps.eval(interp, fc_args.first())?;
    let path = text(&path_v).map_err(|_| Fallback)?.to_string();

    // Csv.Document's options, read as `csv_document` reads them.
    let a: Vec<Value> = (1..5)
        .map(|i| steps.eval(interp, csv_args.get(i)))
        .collect::<Result<_, _>>()?;
    let get = |i: usize| a.get(i - 1).cloned().unwrap_or(Value::Null);
    let mut columns = get(1);
    let mut delimiter = get(2);
    let mut extra = opt_number(&get(3)).map_err(|_| Fallback)?;
    let mut encoding = opt_number(&get(4)).map_err(|_| Fallback)?;
    let mut quote_style = 1.0;
    if let Value::Record(r) = get(1) {
        columns = r.get("Columns").cloned().unwrap_or(Value::Null);
        delimiter = r.get("Delimiter").cloned().unwrap_or(Value::Null);
        if let Some(e) = r.get("ExtraValues") {
            extra = opt_number(e).map_err(|_| Fallback)?;
        }
        if let Some(e) = r.get("Encoding") {
            encoding = opt_number(e).map_err(|_| Fallback)?;
        }
        if let Some(q) = r.get("QuoteStyle") {
            quote_style = opt_number(q).map_err(|_| Fallback)?.unwrap_or(1.0);
        }
        if let Some(Value::Number(n)) = r.get("CsvStyle")
            && n.f() != 0.0
        {
            return Err(Fallback);
        }
    }
    let delim = match &delimiter {
        Value::Null => ",".to_string(),
        Value::Text(t) => t.to_string(),
        _ => return Err(Fallback),
    };
    if !Dialect::valid_delim(delim.as_bytes()) {
        return Err(Fallback);
    }
    let width = match &columns {
        Value::Null => None,
        Value::Number(_) => Some(lib::table::column_count_arg(interp, &columns).map_err(|_| Fallback)?),
        _ => return Err(Fallback),
    };
    let extra = extra.map(|e| e as i64);
    if extra == Some(0) {
        return Err(Fallback);
    }
    if !matches!(
        encoding.map(|c| c as i64),
        None | Some(65001 | 1200 | 1201 | 1252 | 28591 | 20127)
    ) {
        return Err(Fallback);
    }

    if let Some(pa) = promote_args {
        let o = steps.eval(interp, pa.get(1))?;
        lib::culture_arg(interp, &o).map_err(|_| Fallback)?;
    }
    let mut ops = Vec::new();
    let mut culture = interp.culture.clone();
    if let Some(ta) = types_args {
        let list = steps.eval(interp, ta.get(1))?;
        let c = steps.eval(interp, ta.get(2))?;
        culture = lib::culture_arg(interp, &c).map_err(|_| Fallback)?;
        for p in lib::table::pairs(&list).map_err(|_| Fallback)? {
            let name = text(p.first().unwrap_or(&Value::Null))
                .map_err(|_| Fallback)?
                .to_string();
            let ty = lib::mtype(p.get(1).unwrap_or(&Value::Null)).map_err(|_| Fallback)?;
            if !supported(&ty) || ops.iter().any(|(n, _): &(String, MType)| *n == name) {
                return Err(Fallback);
            }
            ops.push((name, ty));
        }
    }
    let (full, size) = lib::files::file_to_stream(interp, &path).map_err(|_| Fallback)?;
    if size > interp.limits.max_file_bytes {
        return Err(Outcome::Fail(Error::LimitExceeded(format!(
            "file `{path}` is larger than {} bytes",
            interp.limits.max_file_bytes
        ))));
    }
    Ok(Plan {
        path,
        full,
        size,
        delim,
        quoting: if quote_style == 1.0 {
            Quoting::MCsv
        } else {
            Quoting::MNone
        },
        encoding,
        width,
        extra_error: extra == Some(1),
        promote: promote_args.is_some(),
        ops,
        culture,
    })
}

/// The fast form of a column's conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fast {
    /// Untyped, `any` or `text`: the text.
    Text,
    Int,
    Currency,
    Number,
    Date,
    General,
}

/// One column's conversion.
#[derive(Debug, Clone, Copy)]
struct Spec {
    kind: Kind,
    nullable: bool,
    facet: Facet,
    declared: DataType,
    fast: Fast,
}

impl Spec {
    fn text() -> Self {
        Spec {
            kind: Kind::Any,
            nullable: true,
            facet: Facet::None,
            declared: DataType::String,
            fast: Fast::Text,
        }
    }

    fn of(t: &MType) -> Result<Self, Outcome> {
        let declared = match super::column_type(t) {
            Ok(Some(d)) => d,
            Ok(None) => return Ok(Self::text()),
            Err(_) => return Err(Outcome::Fallback),
        };
        let fast = match (t.kind, t.facet) {
            (Kind::Text, _) => Fast::Text,
            (Kind::Number, Facet::Int64) => Fast::Int,
            (Kind::Number, Facet::Currency) => Fast::Currency,
            (Kind::Number, Facet::None | Facet::Double) => Fast::Number,
            (Kind::Date | Kind::DateTime, _) => Fast::Date,
            _ => Fast::General,
        };
        Ok(Spec {
            kind: t.kind,
            nullable: t.nullable,
            facet: t.facet,
            declared,
            fast,
        })
    }
}

/// Why a cell did not convert.
enum CellErr {
    /// `super::cell` refused the value (`strict_errors`).
    Cell(String),
    /// The coercion to the column type failed.
    Coerce(String),
    /// The conversion raised what the evaluator would stop on.
    Fatal,
}

/// The evaluator's conversion of one text cell.
fn general(s: &str, spec: &Spec, c: &CultureInfo, nulls: bool) -> Result<CellValue, CellErr> {
    let ty = MType {
        kind: spec.kind,
        nullable: spec.nullable,
        facet: spec.facet,
        fields: None,
    };
    let v = match Value::store(convert::convert_to(&Value::text(s), &ty, c)) {
        Ok(v) => v,
        Err(_) => return Err(CellErr::Fatal),
    };
    let cell = super::cell(&v, Some(spec.declared), nulls).map_err(CellErr::Cell)?;
    cv::coerce(
        cell,
        spec.declared,
        Coercion {
            parse_text: false,
            format_text: false,
        },
    )
    .map_err(CellErr::Coerce)
}

/// One text cell, as `TransformColumnTypes` and the load convert it.
fn convert_cell(
    s: &str,
    spec: &Spec,
    c: &CultureInfo,
    sep: Option<u8>,
    nulls: bool,
    buf: &mut String,
) -> Result<CellValue, CellErr> {
    match spec.fast {
        Fast::Text => return Ok(CellValue::Text(s.to_string())),
        Fast::General => {}
        // An empty text converts to an error, loaded as a blank.
        _ if s.is_empty() && nulls => return Ok(CellValue::Null),
        Fast::Int => {
            if let Some(i) = cv::fast_int(s) {
                return Ok(CellValue::Int(i));
            }
        }
        Fast::Currency => {
            if let Some(v) = sep.and_then(|sep| cv::fast_fixed(s, sep)) {
                return Ok(CellValue::Decimal(v));
            }
        }
        Fast::Number => {
            if let Some(n) = sep
                .and_then(|sep| cv::fast_plain_number(s, sep, buf))
                .and_then(Num::parse)
            {
                return Ok(CellValue::Real(n.f()));
            }
        }
        Fast::Date => {
            if let Some(d) = cv::fast_iso_day(s) {
                return Ok(CellValue::DateTime(d as f64));
            }
        }
    }
    general(s, spec, c, nulls)
}

/// One chunk's columns.
struct Part {
    cols: Vec<Vec<CellValue>>,
    records: usize,
    /// First errors per column (row in the chunk, 1-based; message).
    cell_errs: Vec<Option<(usize, String)>>,
    coerce_errs: Vec<Option<(usize, String)>>,
    fallback: bool,
    text_bytes: usize,
}

/// The table being loaded.
struct Loader<'p> {
    plan: &'p Plan,
    nulls: bool,
    threads: usize,
    header: Option<Header>,
    cols: Vec<Vec<CellValue>>,
    rows: usize,
    records: usize,
    text_bytes: usize,
    cell_errs: Vec<Option<(usize, String)>>,
    coerce_errs: Vec<Option<(usize, String)>>,
    budget: Budget,
    sized: bool,
}

struct Header {
    names: Vec<Rc<str>>,
    specs: Vec<Spec>,
    width: usize,
}

impl Loader<'_> {
    fn dialect(&self, max_cell: usize) -> Dialect<'_> {
        Dialect {
            quoting: self.plan.quoting,
            delim: self.plan.delim.as_bytes(),
            max_cell,
            max_fields: usize::MAX,
        }
    }

    fn too_long(interp: &Interp) -> Outcome {
        Outcome::Fail(Error::LimitExceeded(format!(
            "a text value is longer than {} bytes",
            interp.limits.max_cell_bytes
        )))
    }

    fn check_records(&self, interp: &Interp, records: usize) -> Result<(), Outcome> {
        interp.check_rows(records).map_err(Outcome::from)
    }

    /// Reads the first record (names and width); the offset where data starts, or `None` when
    /// the text ends inside it.
    fn header(&mut self, interp: &mut Interp, text: &str, eof: bool) -> Result<Option<usize>, Outcome> {
        let d = self.dialect(interp.limits.max_cell_bytes);
        let mut rec = Rec::new();
        let next = match csvscan::scan(text, 0, eof, &d, usize::MAX, &mut rec) {
            Scan::Record(next) => next,
            Scan::Incomplete => return Ok(None),
            Scan::Bad(_) => return Err(Self::too_long(interp)),
            // An empty file: the evaluator's table has no columns.
            Scan::End => return Err(Outcome::Fallback),
        };
        let n = rec.count();
        let width = match self.plan.width {
            Some(w) => w,
            None => {
                lib::table::column_count_arg(interp, &Value::num(n as f64)).map_err(|_| Outcome::Fallback)?
            }
        };
        if self.plan.extra_error && n > width {
            return Err(Outcome::Fallback);
        }
        let mut names: Vec<Rc<str>> = Vec::with_capacity(width);
        if self.plan.promote {
            let mut cols: Vec<Column> = Vec::with_capacity(width);
            for i in 0..width {
                let s = rec.field(text, i).map_or("", |f| f.0);
                let default = format!("Column{}", i + 1);
                let name = lib::table::unique_name(&cols, if s.is_empty() { &default } else { s });
                cols.push(Column {
                    name: name.clone(),
                    ty: MType::any(),
                });
                names.push(name);
            }
            self.records = 1;
            interp.tick(2).map_err(Outcome::from)?;
        } else {
            names.extend((1..=width).map(|i| Rc::from(format!("Column{i}"))));
        }
        let mut specs = vec![Spec::text(); width];
        for (name, ty) in &self.plan.ops {
            let i = names
                .iter()
                .position(|n| **n == **name)
                .ok_or(Outcome::Fallback)?;
            if let Some(s) = specs.get_mut(i) {
                *s = Spec::of(ty)?;
            }
        }
        self.cols = (0..width).map(|_| Vec::new()).collect();
        self.cell_errs = vec![None; width];
        self.coerce_errs = vec![None; width];
        self.header = Some(Header { names, specs, width });
        Ok(Some(if self.plan.promote { next } else { 0 }))
    }

    /// Loads the records of `text` (a region); returns the bytes consumed and whether the
    /// input is done.
    fn region(
        &mut self,
        interp: &mut Interp,
        text: &str,
        eof: bool,
        buffers: usize,
    ) -> Result<(usize, bool), Outcome> {
        let start = match &self.header {
            Some(_) => 0,
            None => match self.header(interp, text, eof)? {
                Some(s) => s,
                None => return Ok((0, false)),
            },
        };
        let Some(h) = &self.header else {
            return Err(Outcome::Fallback);
        };
        let width = h.width;
        let specs = &h.specs;
        let d = self.dialect(interp.limits.max_cell_bytes);
        let culture = &self.plan.culture;
        let sep = u8::try_from(culture.decimal).ok().filter(u8::is_ascii);
        let nulls = self.nulls;
        let extra_error = self.plan.extra_error;
        let work = |from: usize, nominal_end: usize| {
            let mut buf = String::new();
            csvscan::parse_chunk(
                text,
                from,
                nominal_end,
                eof,
                &d,
                width,
                || Part {
                    cols: (0..width).map(|_| Vec::new()).collect(),
                    records: 0,
                    cell_errs: vec![None; width],
                    coerce_errs: vec![None; width],
                    fallback: false,
                    text_bytes: 0,
                },
                |p, rec| {
                    let n = rec.count();
                    if extra_error && n > width {
                        p.fallback = true;
                        return false;
                    }
                    p.records += 1;
                    for (i, (col, spec)) in p.cols.iter_mut().zip(specs).enumerate() {
                        let s = rec.field(text, i).map_or("", |f| f.0);
                        let v = match convert_cell(s, spec, culture, sep, nulls, &mut buf) {
                            Ok(v) => v,
                            Err(CellErr::Fatal) => {
                                p.fallback = true;
                                return false;
                            }
                            Err(CellErr::Cell(m)) => {
                                if let Some(e @ None) = p.cell_errs.get_mut(i) {
                                    *e = Some((p.records, m));
                                }
                                CellValue::Null
                            }
                            Err(CellErr::Coerce(m)) => {
                                if let Some(e @ None) = p.coerce_errs.get_mut(i) {
                                    *e = Some((p.records, m));
                                }
                                CellValue::Null
                            }
                        };
                        if let CellValue::Text(t) = &v {
                            p.text_bytes += t.len() + csvscan::TEXT_OVERHEAD;
                        }
                        col.push(v);
                    }
                    true
                },
            )
        };
        let parts = csvscan::run_region(text, start, self.threads, &work);
        let Some(last) = parts.last() else {
            return Err(Outcome::Fallback);
        };
        let (end, stop) = (last.end, last.stop);
        let region_records: usize = parts.iter().map(|p| p.out.records).sum();
        if parts.iter().any(|p| p.out.fallback) {
            return Err(Outcome::Fallback);
        }
        // Limits in the evaluator's order: rows as records are read, then a long text.
        self.check_records(interp, self.records.saturating_add(region_records))?;
        if let Stop::Bad(_) = stop {
            return Err(Self::too_long(interp));
        }
        let ops = self.plan.ops.len() as u64;
        interp
            .tick((region_records as u64).saturating_mul(2 + ops))
            .map_err(Outcome::from)?;

        let region_text: usize = parts.iter().map(|p| p.out.text_bytes).sum();
        if !self.sized && region_records > 0 {
            // The table's size, estimated from this region, before keeping anything.
            self.sized = true;
            let consumed = end.saturating_sub(start).max(1);
            let est = if eof {
                region_records
            } else {
                (self.plan.size as f64 * region_records as f64 / consumed as f64) as usize + 1
            };
            let per_row = region_text as f64 / region_records.max(1) as f64;
            self.budget.need(
                csvscan::table_bytes(est, width, per_row).saturating_add(buffers),
                &format!("loading `{}`", self.plan.path),
            )?;
            let cap = (est as f64 * 1.03) as usize + 1024;
            let cap = cap.min(interp.limits.max_rows.saturating_add(1));
            for c in &mut self.cols {
                c.reserve_exact(cap);
            }
        }
        for mut p in parts {
            for (errs, perrs) in [
                (&mut self.cell_errs, &mut p.out.cell_errs),
                (&mut self.coerce_errs, &mut p.out.coerce_errs),
            ] {
                for (e, pe) in errs.iter_mut().zip(perrs.iter_mut()) {
                    if e.is_none()
                        && let Some((r, m)) = pe.take()
                    {
                        *e = Some((self.rows + r, m));
                    }
                }
            }
            for (col, mut pc) in self.cols.iter_mut().zip(p.out.cols.drain(..)) {
                col.append(&mut pc);
            }
            self.rows += p.out.records;
        }
        self.records = self.records.saturating_add(region_records);
        self.text_bytes = self.text_bytes.saturating_add(region_text);
        let held: usize = self
            .cols
            .iter()
            .map(|c| c.capacity().saturating_mul(csvscan::CELL_BYTES))
            .fold(self.text_bytes, usize::saturating_add);
        self.budget.need(
            held.saturating_add(buffers),
            &format!("loading `{}`", self.plan.path),
        )?;
        Ok(match stop {
            Stop::End => (text.len(), true),
            _ => (end, false),
        })
    }

    fn finish(self) -> Result<TableData, Outcome> {
        let Some(h) = self.header else {
            return Err(Outcome::Fallback);
        };
        for ((name, ce), oe) in h.names.iter().zip(self.cell_errs).zip(self.coerce_errs) {
            if let Some((row, message)) = ce.or(oe) {
                return Err(Outcome::Fail(Error::Value {
                    row,
                    column: name.to_string(),
                    message,
                }));
            }
        }
        let mut types: Vec<MType> = vec![MType::any(); h.width];
        for (name, ty) in &self.plan.ops {
            if let Some(i) = h.names.iter().position(|n| **n == **name)
                && let Some(t) = types.get_mut(i)
            {
                *t = ty.clone();
            }
        }
        let kinds = types
            .iter()
            .map(|t| match t.kind {
                Kind::Date => Some(DateKind::Date),
                Kind::Time => Some(DateKind::Time),
                Kind::DateTime => Some(DateKind::DateTime),
                Kind::DateTimeZone => Some(DateKind::DateTimeZone),
                _ => None,
            })
            .collect();
        let columns = h
            .names
            .iter()
            .zip(&h.specs)
            .zip(self.cols)
            .map(|((name, spec), mut values)| {
                if values.capacity() > values.len().saturating_add(values.len() / 8).saturating_add(4096) {
                    values.shrink_to_fit();
                }
                ColumnData {
                    name: name.to_string(),
                    data_type: spec.declared,
                    values,
                }
            })
            .collect();
        let mut data = TableData::new(columns).map_err(Outcome::Fail)?;
        data.set_date_kinds(kinds);
        Ok(data)
    }
}

/// How the file's bytes become text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// UTF-8, streamed (invalid bytes switch to [`Mode::Whole`]).
    Utf8,
    /// A single-byte code page whose ASCII bytes are ASCII, streamed while the text is ASCII.
    Ascii,
    /// Decoded whole by the evaluator's `decode` (UTF-16, other bytes of a code page, invalid
    /// UTF-8).
    Whole,
}

fn load(plan: &Plan, interp: &mut Interp, opts: &SourceOptions) -> Result<TableData, Outcome> {
    interp.tick(plan.size / 64).map_err(Outcome::from)?;
    let mut file = std::fs::File::open(&plan.full).map_err(|_| Outcome::Fallback)?;
    let mut buf: Vec<u8> = Vec::new();
    let read = |file: &mut std::fs::File, buf: &mut Vec<u8>, n: usize| -> Result<bool, Outcome> {
        buf.try_reserve(n).map_err(|_| Outcome::Fallback)?;
        let got = file
            .by_ref()
            .take(n as u64)
            .read_to_end(buf)
            .map_err(|_| Outcome::Fallback)?;
        Ok(got < n)
    };
    let mut eof = read(&mut file, &mut buf, REGION)?;
    let mode = if buf.starts_with(&[0xEF, 0xBB, 0xBF]) {
        buf.drain(..3);
        Mode::Utf8
    } else if buf.starts_with(&[0xFF, 0xFE]) || buf.starts_with(&[0xFE, 0xFF]) {
        Mode::Whole
    } else {
        match plan.encoding.map(|c| c as i64) {
            None | Some(65001) => Mode::Utf8,
            Some(1252 | 28591 | 20127) => Mode::Ascii,
            _ => Mode::Whole,
        }
    };
    let limits = interp.limits;
    let new_loader = || Loader {
        plan,
        nulls: !opts.strict_errors,
        threads: csvscan::threads(),
        header: None,
        cols: Vec::new(),
        rows: 0,
        records: 0,
        text_bytes: 0,
        cell_errs: Vec::new(),
        coerce_errs: Vec::new(),
        budget: Budget::new(&limits),
        sized: false,
    };
    let mut loader = new_loader();
    if mode != Mode::Whole {
        loop {
            let (valid, region_eof) = match mode {
                Mode::Ascii if !buf.is_ascii() => break,
                Mode::Ascii => (buf.len(), eof),
                _ => match std::str::from_utf8(&buf) {
                    Ok(_) => (buf.len(), eof),
                    Err(e) if e.error_len().is_none() && !eof => (e.valid_up_to(), false),
                    Err(_) => break,
                },
            };
            let text =
                std::str::from_utf8(buf.get(..valid).unwrap_or_default()).map_err(|_| Outcome::Fallback)?;
            let buffers = buf.capacity();
            let (consumed, done) = loader.region(interp, text, region_eof, buffers)?;
            if done {
                return loader.finish();
            }
            buf.drain(..consumed.min(buf.len()));
            if eof {
                // Only an incomplete character can be left at the end: invalid UTF-8.
                break;
            }
            eof = read(&mut file, &mut buf, REGION)?;
        }
        // Not text the stream can take: decode the whole file as the evaluator does.
        loader = new_loader();
    }
    drop(buf);
    file = std::fs::File::open(&plan.full).map_err(|_| Outcome::Fallback)?;
    let size = usize::try_from(plan.size).unwrap_or(usize::MAX);
    loader
        .budget
        .need(size.saturating_mul(4), &format!("loading `{}`", plan.path))?;
    let mut bytes = Vec::new();
    read(&mut file, &mut bytes, size.saturating_add(1))?;
    let text = lib::misc::decode(&bytes, plan.encoding).map_err(Outcome::from)?;
    drop(bytes);
    let buffers = text.len();
    let (_, done) = loader.region(interp, &text, true, buffers)?;
    if !done {
        return Err(Outcome::Fallback);
    }
    drop(text);
    loader.finish()
}
