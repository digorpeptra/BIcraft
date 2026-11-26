//! Power Query (M) evaluation.
//!
//! A small, sandboxed evaluator for the subset of the M language that agents
//! and Power BI Desktop commonly write: `let`/`in`, records, lists, `each`,
//! functions, `if`, `try`/`otherwise`, the operators, typed `#table` literals,
//! Desktop's compressed "Enter data" tables, and the common `Table.*`,
//! `List.*`, `Text.*`, `Number.*`, `Date.*` library functions (see
//! `docs/formats/data-sources.md` for the list).
//!
//! Evaluation is pure: there is no network, environment or clock access, and
//! files are read only under the explicit [`SourceOptions::file_roots`].
//! Anything outside the supported surface is [`Error::UnsupportedSource`]
//! naming the function. Steps, time, nesting, rows and text sizes are capped by
//! [`crate::Limits`].

mod ast;
mod catalog;
mod check;
mod coltypes;
mod convert;
mod decimal;
pub mod entered;
mod eval;
mod fastcsv;
pub mod folding;
mod format;
mod json;
mod lexer;
mod lib;
pub mod navigate;
mod parser;
mod remote;
pub mod scan;
pub mod sources;
pub mod steps;
pub mod transform;
pub mod transform_more;
pub mod transform_parse;
mod value;
mod xml;

use std::rc::Rc;

pub use catalog::{LibraryName, identifier, library_name};
pub use check::{Analysis, Reference, SyntaxError, analyze, analyze_all};
pub use format::{FormatError, FormatOptions, MAX_INDENT_WIDTH, format};

use crate::value::{self as cv, Coercion};
use crate::{CellValue, ColumnData, DataType, Error, Limits, SourceOptions, TableData};
use value::{Facet, Flow, Kind, MType, Num, Table, Value};

/// Stack size of the evaluation thread: deep but bounded recursion
/// (`Limits::max_eval_depth`) must never overflow the caller's stack.
const EVAL_STACK: usize = 256 << 20;

/// Loads a Power Query expression with no file access and no shared queries.
pub fn load_m(expression: &str, limits: &Limits) -> Result<TableData, Error> {
    load_m_with(
        expression,
        &SourceOptions {
            limits: *limits,
            ..SourceOptions::default()
        },
    )
}

/// Evaluates a Power Query expression (a partition source) to table data.
///
/// The expression may reference the named expressions in
/// [`SourceOptions::shared`] (other queries, parameters). It runs on a
/// dedicated thread with a large stack; every cap in [`Limits`] applies.
pub fn load_m_with(expression: &str, opts: &SourceOptions) -> Result<TableData, Error> {
    opts.limits.check_bytes(expression.len(), "M expression")?;
    for (name, e) in &opts.shared {
        opts.limits
            .check_bytes(e.len(), &format!("M expression `{name}`"))?;
    }
    let expr = expression.to_string();
    let opts = opts.clone();
    on_eval_thread(move || evaluate(&expr, &opts))
}

/// [`load_m_with`], also counting each column's error values (loaded as blanks, whatever
/// [`SourceOptions::strict_errors`] says): the Power Query editor's column quality. The CSV fast
/// path is not taken, so a value that does not convert is counted as an error.
pub fn load_m_counting_errors(
    expression: &str,
    opts: &SourceOptions,
) -> Result<(TableData, Vec<usize>), Error> {
    opts.limits.check_bytes(expression.len(), "M expression")?;
    for (name, e) in &opts.shared {
        opts.limits
            .check_bytes(e.len(), &format!("M expression `{name}`"))?;
    }
    let expr = expression.to_string();
    let opts = opts.clone();
    on_eval_thread(move || {
        let t = evaluate_table(&expr, &opts)?;
        let errors = (0..t.columns.len())
            .map(|ci| {
                t.rows
                    .iter()
                    .filter(|r| matches!(r.get(ci), Some(Value::Error(_))))
                    .count()
            })
            .collect();
        Ok((to_table_data(&t, true)?, errors))
    })
}

/// Runs `f` on a dedicated evaluation thread with a large stack (a panic there is an error).
fn on_eval_thread<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    let handle = std::thread::Builder::new()
        .name("bicraft-m".into())
        .stack_size(EVAL_STACK)
        .spawn(f);
    match handle {
        Ok(h) => h
            .join()
            .unwrap_or_else(|_| Err(Error::LimitExceeded("M evaluation aborted".into()))),
        Err(e) => Err(Error::LimitExceeded(format!(
            "could not start the M evaluation thread: {e}"
        ))),
    }
}

fn evaluate(expression: &str, opts: &SourceOptions) -> Result<TableData, Error> {
    evaluate_with(expression, opts, true)
}

/// The parsed expression and shared queries.
type Parsed = (ast::Expr, Vec<(Rc<str>, Rc<ast::Expr>)>);

fn parsed(expression: &str, opts: &SourceOptions) -> Result<Parsed, Error> {
    let main = parser::parse(expression, &opts.limits)?;
    let mut shared = Vec::with_capacity(opts.shared.len());
    for (name, text) in &opts.shared {
        let e = parser::parse(text, &opts.limits)?;
        shared.push((Rc::<str>::from(name.as_str()), Rc::new(e)));
    }
    Ok((main, shared))
}

/// An evaluation's outcome as a table.
fn finish_table(r: Result<Value, Flow>) -> Result<Rc<Table>, Error> {
    let v = match r {
        Ok(v) => v,
        Err(Flow::Fatal(e)) => return Err(e),
        Err(Flow::Err(e)) => {
            return Err(Error::Evaluation {
                reason: e.reason.clone(),
                message: e.message.clone(),
            });
        }
    };
    match v {
        Value::Table(t) => Ok(t),
        other => Err(Error::UnsupportedSource(format!(
            "the M expression evaluates to a {} value, not a table",
            other.type_name()
        ))),
    }
}

/// An evaluation's outcome as table data.
fn finish(r: Result<Value, Flow>, opts: &SourceOptions) -> Result<TableData, Error> {
    let t = finish_table(r)?;
    to_table_data(&t, !opts.strict_errors)
}

/// The table an expression evaluates to, without the CSV fast path.
fn evaluate_table(expression: &str, opts: &SourceOptions) -> Result<Rc<Table>, Error> {
    let (main, shared) = parsed(expression, opts)?;
    evaluate_parsed(&main, shared, opts)
}

fn evaluate_parsed(
    main: &ast::Expr,
    shared: Vec<(Rc<str>, Rc<ast::Expr>)>,
    opts: &SourceOptions,
) -> Result<Rc<Table>, Error> {
    let mut interp = eval::Interp::new(opts, shared);
    let r = interp
        .eval(main, &None)
        .and_then(|v| remote::force_value(&mut interp, v));
    finish_table(r)
}

/// [`evaluate`], with or without the CSV fast path (`fastcsv`; the tests compare both).
fn evaluate_with(expression: &str, opts: &SourceOptions, fast: bool) -> Result<TableData, Error> {
    let (main, shared) = parsed(expression, opts)?;
    if fast {
        let mut interp = eval::Interp::new(opts, shared.clone());
        if let Some(r) = fastcsv::try_load(&main, &mut interp, opts) {
            return r;
        }
    }
    let t = evaluate_parsed(&main, shared, opts)?;
    to_table_data(&t, !opts.strict_errors)
}

/// Column type for an M column type; `None` means "infer from the values".
fn column_type(t: &MType) -> Result<Option<DataType>, String> {
    Ok(Some(match t.kind {
        Kind::Any | Kind::AnyNonNull => return Ok(None),
        Kind::Text => DataType::String,
        Kind::Number => match t.facet {
            Facet::Int64 | Facet::Int32 | Facet::Int16 | Facet::Int8 | Facet::Byte => DataType::Int64,
            Facet::Currency => DataType::Decimal,
            _ => DataType::Double,
        },
        Kind::Date | Kind::DateTime | Kind::DateTimeZone | Kind::Time => DataType::DateTime,
        Kind::Logical => DataType::Boolean,
        Kind::Duration => DataType::Double,
        Kind::Binary => DataType::Binary,
        other => {
            return Err(format!(
                "columns of type `{}` cannot be loaded",
                MType::prim(other).name()
            ));
        }
    }))
}

/// One M value as a VertiPaq cell (before conversion to the column type).
fn cell(v: &Value, declared: Option<DataType>, errors_as_null: bool) -> Result<CellValue, String> {
    let untyped = declared.is_none();
    Ok(match v {
        Value::Error(_) if errors_as_null => CellValue::Null,
        Value::Null => CellValue::Null,
        Value::Logical(b) => CellValue::Bool(*b),
        // Decimals load exactly into whole-number and currency columns; anything else loads
        // the double (converted as .NET Framework does).
        Value::Number(Num::D(d)) if declared == Some(DataType::Int64) && d.to_i128().is_some() => {
            match d.to_i128().and_then(|i| i64::try_from(i).ok()) {
                Some(i) => CellValue::Int(i),
                None => CellValue::Real(d.to_f64()),
            }
        }
        // Currency: the decimal (doubles converted to 15 digits) rounded half to even.
        Value::Number(n) if declared == Some(DataType::Decimal) && n.dec().is_some() => {
            let d = n.dec().unwrap_or(decimal::Dec::ZERO);
            match convert::dec_round_even(d, 4)
                .and_then(|r| r.mul(decimal::Dec::pow10(4)?).ok())
                .and_then(|r| r.to_i128())
                .and_then(|i| i64::try_from(i).ok())
            {
                Some(i) => CellValue::Decimal(i),
                None => CellValue::Real(n.f()),
            }
        }
        Value::Number(n) => {
            let n = n.f();
            match cv::int_from_f64(n) {
                Some(i) if untyped && n.abs() < 9.0e15 => CellValue::Int(i),
                _ => CellValue::Real(n),
            }
        }
        Value::Text(t) => CellValue::Text(t.to_string()),
        Value::Date(d) => CellValue::DateTime(*d as f64),
        Value::DateTime(ms) => {
            let (d, t) = convert::split_ms(*ms);
            CellValue::DateTime(cv::ole_value(d, t))
        }
        // Desktop loads the local clock time and drops the offset.
        Value::DateTimeZone(ms, _) => {
            let (d, t) = convert::split_ms(*ms);
            CellValue::DateTime(cv::ole_value(d, t))
        }
        Value::Time(ms) => CellValue::DateTime(cv::ole_value(0, *ms)),
        Value::Duration(ms) => CellValue::Real(*ms as f64 / eval::MS_PER_DAY as f64),
        Value::Error(e) => return Err(format!("{}: {}", e.reason, e.message)),
        other => {
            return Err(format!(
                "a {} value cannot be loaded into a column",
                other.type_name()
            ));
        }
    })
}

fn to_table_data(t: &Table, errors_as_null: bool) -> Result<TableData, Error> {
    let mut columns = Vec::with_capacity(t.columns.len());
    let kinds = t
        .columns
        .iter()
        .map(|c| match c.ty.kind {
            Kind::Date => Some(crate::DateKind::Date),
            Kind::Time => Some(crate::DateKind::Time),
            Kind::DateTime => Some(crate::DateKind::DateTime),
            Kind::DateTimeZone => Some(crate::DateKind::DateTimeZone),
            _ => None,
        })
        .collect();
    for (ci, c) in t.columns.iter().enumerate() {
        let declared = column_type(&c.ty).map_err(Error::UnsupportedSource)?;
        let mut values = Vec::with_capacity(t.rows.len());
        for (ri, row) in t.rows.iter().enumerate() {
            let v = row.get(ci).unwrap_or(&Value::Null);
            values.push(cell(v, declared, errors_as_null).map_err(|message| Error::Value {
                row: ri + 1,
                column: c.name.to_string(),
                message,
            })?);
        }
        let (data_type, mode) = match declared {
            Some(t) => (
                t,
                Coercion {
                    parse_text: false,
                    format_text: false,
                },
            ),
            None => (
                cv::infer_from_values(&values),
                Coercion {
                    parse_text: false,
                    format_text: true,
                },
            ),
        };
        let mut out = Vec::with_capacity(values.len());
        for (i, v) in values.into_iter().enumerate() {
            out.push(cv::coerce(v, data_type, mode).map_err(|message| Error::Value {
                row: i + 1,
                column: c.name.to_string(),
                message,
            })?);
        }
        columns.push(ColumnData {
            name: c.name.to_string(),
            data_type,
            values: out,
        });
    }
    let mut data = TableData::new(columns)?;
    data.set_date_kinds(kinds);
    Ok(data)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_check;
#[cfg(test)]
mod tests_connect;
#[cfg(test)]
mod tests_desktop;
#[cfg(test)]
mod tests_fastcsv;
#[cfg(test)]
mod tests_format;
#[cfg(test)]
mod tests_library;
#[cfg(test)]
mod tests_navigate;
#[cfg(test)]
mod tests_safety;
#[cfg(test)]
mod tests_scan;
#[cfg(test)]
mod tests_sources;
#[cfg(test)]
mod tests_steps;
#[cfg(test)]
mod tests_transform_more;
