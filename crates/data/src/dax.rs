//! DAX literal tables for calculated-table partitions.
//!
//! Supported (keywords are case-insensitive):
//!
//! - `DATATABLE("A", STRING, "B", INTEGER, {{"x", 1}, {"y", 2}})`
//! - table constructors `{1, 2, 3}` (column `Value`) and
//!   `{(1, "a"), (2, "b")}` (columns `Value1`, `Value2`, …)
//! - `ROW("A", 1, "B", "x")`
//! - `CALENDAR(DATE(2026,1,1), DATE(2026,12,31))` (column `Date`)
//! - `GENERATESERIES(0, 20, 0.5)` over literal numbers (column `Value`; whole numbers when the
//!   three are, a what-if parameter's table)
//!
//! Values are literals: numbers, strings, `TRUE`/`FALSE` (with or without
//! `()`), `BLANK()`, `DATE(y,m,d)`, `TIME(h,m,s)`, `DT"2026-01-05"` and `NAMEOF('T'[C])` (the
//! text `'T'[C]`, as a field parameter stores it; the reference must name its table). Any other
//! expression is [`Error::UnsupportedSource`]: bicraft does not evaluate DAX.

use crate::value::{self, Coercion};
use crate::{CellValue, ColumnData, DataType, Error, Limits, TableData};

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Text(String),
    /// `DT"…"` datetime literal.
    Dt(String),
    Int(i64),
    Real(f64),
    Punct(char),
    /// `'Table'` or `[Column]` references (never literal).
    Reference(String),
    Eof,
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    line: usize,
    column: usize,
}

fn syntax(line: usize, column: usize, message: impl Into<String>) -> Error {
    Error::Syntax {
        format: "DAX",
        line,
        column,
        message: message.into(),
    }
}

fn unsupported(what: impl std::fmt::Display) -> Error {
    Error::UnsupportedSource(format!(
        "DAX expression uses {what}; only literal tables (DATATABLE, table constructors, ROW, CALENDAR over literal dates) load (bicraft does not evaluate DAX)"
    ))
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => format!("`{s}`"),
        Tok::Text(s) => format!("text \"{}\"", value::truncate(s)),
        Tok::Dt(s) => format!("DT\"{}\"", value::truncate(s)),
        Tok::Int(n) => format!("`{n}`"),
        Tok::Real(r) => format!("`{r}`"),
        Tok::Punct(c) => format!("`{c}`"),
        Tok::Reference(s) => format!("the reference `{}`", value::truncate(s)),
        Tok::Eof => "end of input".into(),
    }
}

fn tokenize(src: &str, limits: &Limits) -> Result<Vec<Token>, Error> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line, mut col) = (0usize, 1usize, 1usize);
    let at = |k: usize| chars.get(k).copied();
    macro_rules! adv {
        () => {{
            if at(i) == Some('\n') {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
            i += 1;
        }};
    }
    loop {
        // Whitespace and comments.
        loop {
            match (at(i), at(i + 1)) {
                (Some(c), _) if c.is_whitespace() => adv!(),
                (Some('-'), Some('-')) | (Some('/'), Some('/')) => {
                    while at(i).is_some_and(|c| c != '\n') {
                        adv!();
                    }
                }
                (Some('/'), Some('*')) => {
                    let (l, c) = (line, col);
                    adv!();
                    adv!();
                    loop {
                        match (at(i), at(i + 1)) {
                            (None, _) => return Err(syntax(l, c, "unterminated comment")),
                            (Some('*'), Some('/')) => {
                                adv!();
                                adv!();
                                break;
                            }
                            _ => adv!(),
                        }
                    }
                }
                _ => break,
            }
        }
        let (tl, tc) = (line, col);
        let Some(ch) = at(i) else {
            out.push(Token {
                tok: Tok::Eof,
                line: tl,
                column: tc,
            });
            return Ok(out);
        };
        let quoted = |i: &mut usize, line: &mut usize, col: &mut usize, close: char, doubled: bool| {
            // Reads after the opening delimiter up to `close`; `doubled` allows `""`-style escapes.
            let mut s = String::new();
            loop {
                let Some(ch) = at(*i) else {
                    return Err(syntax(tl, tc, "unterminated literal"));
                };
                if ch == '\n' {
                    *line += 1;
                    *col = 1;
                } else {
                    *col += 1;
                }
                *i += 1;
                if ch == close {
                    if doubled && at(*i) == Some(close) {
                        *i += 1;
                        *col += 1;
                    } else {
                        return Ok(s);
                    }
                }
                s.push(ch);
                if s.len() > limits.max_cell_bytes {
                    return Err(Error::LimitExceeded(format!(
                        "a literal is longer than {} bytes",
                        limits.max_cell_bytes
                    )));
                }
            }
        };
        let tok = match ch {
            '"' => {
                adv!();
                Tok::Text(quoted(&mut i, &mut line, &mut col, '"', true)?)
            }
            '\'' => {
                adv!();
                Tok::Reference(quoted(&mut i, &mut line, &mut col, '\'', true)?)
            }
            '[' => {
                adv!();
                Tok::Reference(quoted(&mut i, &mut line, &mut col, ']', true)?)
            }
            c if c.is_ascii_digit() || (c == '.' && at(i + 1).is_some_and(|d| d.is_ascii_digit())) => {
                let mut s = String::new();
                let mut real = false;
                while let Some(d) = at(i) {
                    if d.is_ascii_digit() {
                        s.push(d);
                    } else if d == '.' && !real && !s.contains('e') {
                        real = true;
                        s.push('.');
                    } else if (d == 'e' || d == 'E') && !s.contains('e') {
                        real = true;
                        s.push('e');
                        if let Some(sign @ ('+' | '-')) = at(i + 1) {
                            adv!();
                            s.push(sign);
                        }
                    } else {
                        break;
                    }
                    adv!();
                    if s.len() > 400 {
                        return Err(syntax(tl, tc, "number literal too long"));
                    }
                }
                if !real && let Ok(n) = s.parse::<i64>() {
                    Tok::Int(n)
                } else {
                    Tok::Real(
                        s.parse::<f64>()
                            .map_err(|_| syntax(tl, tc, format!("bad number `{s}`")))?,
                    )
                }
            }
            c if c.is_alphabetic() || c == '_' => {
                let mut s = String::new();
                while let Some(d) = at(i).filter(|d| d.is_alphanumeric() || *d == '_' || *d == '.') {
                    s.push(d);
                    adv!();
                }
                if s.eq_ignore_ascii_case("dt") && at(i) == Some('"') {
                    adv!();
                    Tok::Dt(quoted(&mut i, &mut line, &mut col, '"', true)?)
                } else {
                    Tok::Ident(s)
                }
            }
            c => {
                adv!();
                Tok::Punct(c)
            }
        };
        out.push(Token {
            tok,
            line: tl,
            column: tc,
        });
    }
}

struct Parser<'a> {
    toks: Vec<Token>,
    pos: usize,
    limits: &'a Limits,
    depth: usize,
}

const EOF: Token = Token {
    tok: Tok::Eof,
    line: 0,
    column: 0,
};

impl Parser<'_> {
    fn peek(&self) -> &Token {
        self.toks.get(self.pos).unwrap_or(&EOF)
    }

    fn peek_at(&self, k: usize) -> &Tok {
        self.toks.get(self.pos + k).map_or(&Tok::Eof, |t| &t.tok)
    }

    fn bump(&mut self) -> Token {
        let t = self.peek().clone();
        if self.pos < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn err_here(&self, message: impl Into<String>) -> Error {
        let t = self.peek();
        syntax(t.line, t.column, message)
    }

    fn eat(&mut self, c: char) -> bool {
        let hit = self.peek().tok == Tok::Punct(c);
        if hit {
            self.bump();
        }
        hit
    }

    fn expect(&mut self, c: char) -> Result<(), Error> {
        if self.eat(c) {
            Ok(())
        } else {
            Err(self.err_here(format!("expected `{c}`, found {}", describe(&self.peek().tok))))
        }
    }

    fn keyword(&self) -> Option<String> {
        match &self.peek().tok {
            Tok::Ident(s) => Some(s.to_ascii_uppercase()),
            _ => None,
        }
    }

    fn enter(&mut self) -> Result<(), Error> {
        self.depth += 1;
        if self.depth > self.limits.max_depth {
            return Err(Error::LimitExceeded(format!(
                "DAX nesting deeper than {}",
                self.limits.max_depth
            )));
        }
        Ok(())
    }

    /// The table expression.
    fn table(&mut self) -> Result<TableData, Error> {
        self.enter()?;
        let t = match (self.keyword().as_deref(), self.peek_at(1)) {
            (_, _) if self.peek().tok == Tok::Punct('(') => {
                self.bump();
                let t = self.table()?;
                self.expect(')')?;
                t
            }
            (_, _) if self.peek().tok == Tok::Punct('{') => self.constructor()?,
            (Some("DATATABLE"), Tok::Punct('(')) => self.datatable()?,
            (Some("ROW"), Tok::Punct('(')) => self.row()?,
            (Some("CALENDAR"), Tok::Punct('(')) => self.calendar()?,
            (Some("GENERATESERIES"), Tok::Punct('(')) => self.series()?,
            (Some(_), Tok::Punct('(')) => return Err(unsupported(describe(&self.peek().tok))),
            _ => return Err(unsupported(describe(&self.peek().tok))),
        };
        self.depth -= 1;
        Ok(t)
    }

    /// Fails unless the value ends here.
    fn check_value_end(&self) -> Result<(), Error> {
        match &self.peek().tok {
            Tok::Punct(',' | ')' | '}') | Tok::Eof => Ok(()),
            other => Err(unsupported(format!("an operator or call ({})", describe(other)))),
        }
    }

    fn empty_call(&mut self) -> Result<(), Error> {
        self.expect('(')?;
        self.expect(')')
    }

    fn scalar(&mut self) -> Result<CellValue, Error> {
        self.enter()?;
        let t = self.bump();
        let v = match t.tok {
            Tok::Text(s) => CellValue::Text(s),
            Tok::Int(n) => CellValue::Int(n),
            Tok::Real(r) => CellValue::Real(r),
            Tok::Dt(s) => value::parse_iso_datetime(s.trim())
                .map(CellValue::DateTime)
                .ok_or_else(|| {
                    syntax(
                        t.line,
                        t.column,
                        format!("bad DT literal `{}`", value::truncate(&s)),
                    )
                })?,
            Tok::Punct(sign @ ('-' | '+')) => match (sign, self.scalar()?) {
                ('+', v @ (CellValue::Int(_) | CellValue::Real(_))) => v,
                (_, CellValue::Int(n)) => n
                    .checked_neg()
                    .map_or(CellValue::Real(-(n as f64)), CellValue::Int),
                (_, CellValue::Real(r)) => CellValue::Real(-r),
                (_, other) => return Err(unsupported(format!("a sign applied to {other:?}"))),
            },
            Tok::Ident(name) => {
                let upper = name.to_ascii_uppercase();
                match upper.as_str() {
                    "TRUE" | "FALSE" => {
                        if self.peek().tok == Tok::Punct('(') {
                            self.empty_call()?;
                        }
                        CellValue::Bool(upper == "TRUE")
                    }
                    "BLANK" => {
                        self.empty_call()?;
                        CellValue::Null
                    }
                    "DATE" => {
                        let [y, m, d] = self.int_args::<3>("DATE")?;
                        CellValue::DateTime(dax_date(y, m, d).map_err(|e| syntax(t.line, t.column, e))?)
                    }
                    "TIME" => {
                        let [h, m, s] = self.int_args::<3>("TIME")?;
                        CellValue::DateTime(dax_time(h, m, s).map_err(|e| syntax(t.line, t.column, e))?)
                    }
                    "NAMEOF" => CellValue::Text(self.nameof()?),
                    _ => return Err(unsupported(format!("`{name}`"))),
                }
            }
            other => return Err(unsupported(describe(&other))),
        };
        self.depth -= 1;
        Ok(v)
    }

    fn int_args<const N: usize>(&mut self, name: &str) -> Result<[i64; N], Error> {
        self.expect('(')?;
        let mut args = Vec::new();
        loop {
            let v = self.scalar()?;
            let n = match v {
                CellValue::Int(n) => n,
                CellValue::Real(r) => r.trunc() as i64,
                other => return Err(unsupported(format!("{other:?} as a `{name}` argument"))),
            };
            args.push(n);
            if args.len() > N || self.eat(')') {
                break;
            }
            self.expect(',')?;
        }
        args.try_into()
            .map_err(|_| self.err_here(format!("`{name}` takes {N} arguments")))
    }

    /// `NAMEOF('T'[C])` (after the name): the escaped full name `'T'[C]`. A bare table name
    /// (`T[C]`) is quoted the same way; an unqualified `[C]` needs the model and is unsupported.
    fn nameof(&mut self) -> Result<String, Error> {
        self.expect('(')?;
        let table = match self.bump().tok {
            Tok::Reference(t) if self.peek().tok != Tok::Punct(')') => t,
            Tok::Ident(t) => t,
            other => return Err(unsupported(format!("NAMEOF of {}", describe(&other)))),
        };
        let name = match self.bump().tok {
            Tok::Reference(n) => n,
            other => return Err(unsupported(format!("NAMEOF of {}", describe(&other)))),
        };
        self.expect(')')?;
        Ok(format!(
            "'{}'[{}]",
            table.replace('\'', "''"),
            name.replace(']', "]]")
        ))
    }

    /// `GENERATESERIES(start, end [, increment])` over literal numbers.
    fn series(&mut self) -> Result<TableData, Error> {
        self.bump();
        self.expect('(')?;
        let mut args = Vec::new();
        loop {
            args.push(self.cell()?);
            if args.len() > 3 || self.eat(')') {
                break;
            }
            self.expect(',')?;
        }
        let num = |v: &CellValue| match v {
            CellValue::Int(n) => Ok((*n as f64, true)),
            CellValue::Real(r) if r.is_finite() => Ok((*r, r.fract() == 0.0)),
            other => Err(unsupported(format!("GENERATESERIES over {other:?}"))),
        };
        let (start, end, step) = match args.as_slice() {
            [a, b] => (num(a)?, num(b)?, (1.0, true)),
            [a, b, c] => (num(a)?, num(b)?, num(c)?),
            _ => return Err(self.err_here("GENERATESERIES takes 2 or 3 arguments")),
        };
        if step.0 <= 0.0 {
            return Err(Error::UnsupportedSource(
                "GENERATESERIES needs an increment greater than 0".into(),
            ));
        }
        let span = ((end.0 - start.0) / step.0 + 1e-9).floor();
        let n = if span < 0.0 {
            0
        } else if span < self.limits.max_rows as f64 {
            span as usize + 1
        } else {
            usize::MAX
        };
        self.limits.check_rows(n)?;
        let whole = start.1 && end.1 && step.1;
        let values = (0..n)
            .map(|i| {
                let x = start.0 + i as f64 * step.0;
                if whole {
                    CellValue::Int(x as i64)
                } else {
                    // the decimal the series means, not its binary drift (0.1 * 3)
                    CellValue::Real((x * 1e12).round() / 1e12)
                }
            })
            .collect();
        TableData::new(vec![ColumnData {
            name: "Value".into(),
            data_type: if whole { DataType::Int64 } else { DataType::Double },
            values,
        }])
    }

    /// A value inside a table: a literal followed by `,`, `)` or `}`.
    fn cell(&mut self) -> Result<CellValue, Error> {
        let v = self.scalar()?;
        self.check_value_end()?;
        Ok(v)
    }

    fn constructor(&mut self) -> Result<TableData, Error> {
        self.expect('{')?;
        let mut rows: Vec<Vec<CellValue>> = Vec::new();
        loop {
            let row = if self.eat('(') {
                let mut r = Vec::new();
                loop {
                    r.push(self.cell()?);
                    self.limits.check_columns(r.len())?;
                    if self.eat(')') {
                        break;
                    }
                    self.expect(',')?;
                }
                r
            } else {
                vec![self.cell()?]
            };
            if let Some(first) = rows.first()
                && first.len() != row.len()
            {
                return Err(Error::RowLength {
                    row: rows.len() + 1,
                    expected: first.len(),
                    found: row.len(),
                });
            }
            rows.push(row);
            self.limits.check_rows(rows.len())?;
            if self.eat('}') {
                break;
            }
            self.expect(',')?;
        }
        let width = rows.first().map_or(0, Vec::len);
        let names: Vec<String> = if width == 1 {
            vec!["Value".into()]
        } else {
            (1..=width).map(|i| format!("Value{i}")).collect()
        };
        build(names.into_iter().map(|n| (n, None)).collect(), rows)
    }

    fn datatable(&mut self) -> Result<TableData, Error> {
        self.bump();
        self.expect('(')?;
        let mut header = Vec::new();
        while let Tok::Text(name) = self.peek().tok.clone() {
            self.bump();
            self.expect(',')?;
            let t = self.bump();
            let ty = match &t.tok {
                Tok::Ident(s) => match s.to_ascii_uppercase().as_str() {
                    "STRING" => DataType::String,
                    "INTEGER" => DataType::Int64,
                    "DOUBLE" => DataType::Double,
                    "CURRENCY" => DataType::Decimal,
                    "DATETIME" => DataType::DateTime,
                    "BOOLEAN" => DataType::Boolean,
                    _ => return Err(syntax(t.line, t.column, format!("unknown DATATABLE type `{s}`"))),
                },
                other => {
                    return Err(syntax(
                        t.line,
                        t.column,
                        format!("expected a type, found {}", describe(other)),
                    ));
                }
            };
            self.expect(',')?;
            header.push((name, Some(ty)));
            self.limits.check_columns(header.len())?;
        }
        if header.is_empty() {
            return Err(self.err_here("DATATABLE needs at least one column"));
        }
        self.expect('{')?;
        let mut rows = Vec::new();
        if !self.eat('}') {
            loop {
                self.expect('{')?;
                let mut r = Vec::new();
                if !self.eat('}') {
                    loop {
                        r.push(self.cell()?);
                        self.limits.check_columns(r.len())?;
                        if self.eat('}') {
                            break;
                        }
                        self.expect(',')?;
                    }
                }
                if r.len() != header.len() {
                    return Err(Error::RowLength {
                        row: rows.len() + 1,
                        expected: header.len(),
                        found: r.len(),
                    });
                }
                rows.push(r);
                self.limits.check_rows(rows.len())?;
                if self.eat('}') {
                    break;
                }
                self.expect(',')?;
            }
        }
        self.expect(')')?;
        build(header, rows)
    }

    fn row(&mut self) -> Result<TableData, Error> {
        self.bump();
        self.expect('(')?;
        let mut names = Vec::new();
        let mut row = Vec::new();
        loop {
            let t = self.bump();
            let Tok::Text(name) = t.tok else {
                return Err(syntax(
                    t.line,
                    t.column,
                    format!("expected a column name, found {}", describe(&t.tok)),
                ));
            };
            self.expect(',')?;
            row.push(self.cell()?);
            names.push((name, None));
            self.limits.check_columns(names.len())?;
            if self.eat(')') {
                break;
            }
            self.expect(',')?;
        }
        build(names, vec![row])
    }

    fn calendar(&mut self) -> Result<TableData, Error> {
        self.bump();
        self.expect('(')?;
        let start = self.cell()?;
        self.expect(',')?;
        let end = self.cell()?;
        self.expect(')')?;
        let day = |v: &CellValue| match v {
            CellValue::DateTime(d) | CellValue::Real(d) if d.is_finite() && d.abs() < 4.0e6 => {
                Ok(d.floor() as i64)
            }
            CellValue::Int(n) if n.unsigned_abs() < 4_000_000 => Ok(*n),
            other => Err(unsupported(format!("CALENDAR over {other:?}"))),
        };
        let (a, b) = (day(&start)?, day(&end)?);
        if b < a {
            return Err(Error::UnsupportedSource(
                "CALENDAR end date is before its start date".into(),
            ));
        }
        let n = usize::try_from(b - a + 1).unwrap_or(usize::MAX);
        self.limits.check_rows(n)?;
        let values = (a..=b)
            .map(|d| CellValue::DateTime(value::ole_value(d, 0)))
            .collect();
        TableData::new(vec![ColumnData {
            name: "Date".into(),
            data_type: DataType::DateTime,
            values,
        }])
    }
}

/// Builds columns from row-major values; `None` types are inferred.
fn build(header: Vec<(String, Option<DataType>)>, rows: Vec<Vec<CellValue>>) -> Result<TableData, Error> {
    let width = header.len();
    let mut cells: Vec<Vec<CellValue>> = (0..width).map(|_| Vec::with_capacity(rows.len())).collect();
    for row in rows {
        for (col, v) in cells.iter_mut().zip(row) {
            col.push(v);
        }
    }
    let mode = Coercion {
        parse_text: true,
        format_text: true,
    };
    let mut columns = Vec::with_capacity(width);
    for ((name, ty), values) in header.into_iter().zip(cells) {
        let data_type = ty.unwrap_or_else(|| value::infer_from_values(&values));
        let mut out = Vec::with_capacity(values.len());
        for (i, v) in values.into_iter().enumerate() {
            out.push(value::coerce(v, data_type, mode).map_err(|message| Error::Value {
                row: i + 1,
                column: name.clone(),
                message,
            })?);
        }
        columns.push(ColumnData {
            name,
            data_type,
            values: out,
        });
    }
    TableData::new(columns)
}

/// DAX `DATE`: years 0–1899 are offset by 1900; months and days overflow into
/// the next year/month as in DAX.
fn dax_date(y: i64, m: i64, d: i64) -> Result<f64, String> {
    let y = if (0..1900).contains(&y) { y + 1900 } else { y };
    if !(1..=9999).contains(&y) || m.unsigned_abs() > 120_000 || d.unsigned_abs() > 4_000_000 {
        return Err(format!("DATE({y}, {m}, {d}) is out of range"));
    }
    let months = y * 12 + (m - 1);
    let (yy, mm) = (months.div_euclid(12), months.rem_euclid(12) + 1);
    let first = value::ole_date(yy, mm, 1)?;
    let day = first + (d - 1);
    // 0001-01-01 ..= 9999-12-31
    if !(-693_593..=2_958_465).contains(&day) {
        return Err(format!("DATE({y}, {m}, {d}) is out of range"));
    }
    Ok(value::ole_value(day, 0))
}

/// DAX `TIME`: hours wrap at 24; negative totals are an error.
fn dax_time(h: i64, m: i64, s: i64) -> Result<f64, String> {
    if h.unsigned_abs() > 1_000_000 || m.unsigned_abs() > 1_000_000 || s.unsigned_abs() > 100_000_000 {
        return Err(format!("TIME({h}, {m}, {s}) is out of range"));
    }
    let total = h * 3600 + m * 60 + s;
    if total < 0 {
        return Err(format!("TIME({h}, {m}, {s}) is negative"));
    }
    Ok(value::ole_value(0, total.rem_euclid(86_400) * 1000))
}

/// Loads a calculated-table DAX expression that is a literal table.
pub fn load_dax(expression: &str, limits: &Limits) -> Result<TableData, Error> {
    limits.check_bytes(expression.len(), "DAX expression")?;
    let toks = tokenize(expression, limits)?;
    let mut p = Parser {
        toks,
        pos: 0,
        limits,
        depth: 0,
    };
    let t = p.table()?;
    if p.peek().tok != Tok::Eof {
        return Err(unsupported(format!(
            "an operator or call ({})",
            describe(&p.peek().tok)
        )));
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use CellValue::{Bool, DateTime, Decimal, Int, Null, Real, Text};

    fn load(s: &str) -> Result<TableData, Error> {
        load_dax(s, &Limits::default())
    }

    fn vals(t: &TableData, i: usize) -> Vec<CellValue> {
        t.columns()[i].values.clone()
    }

    fn types(t: &TableData) -> Vec<DataType> {
        t.columns().iter().map(|c| c.data_type).collect()
    }

    #[test]
    fn datatable() {
        let t = load(
            r#"DATATABLE (
                "Name", STRING, "Qty", INTEGER, "Price", DOUBLE, "Cost", CURRENCY, "When", DATETIME, "Ok", BOOLEAN,
                {
                    { "a""b", 1, 1.5, 2.25, "2026-01-05", TRUE },
                    { BLANK(), -2, 3, -0.5, DATE(2026, 1, 5), false() }, -- comment
                    { "c", 3e0, -1e-3, 7, "1/5/2026 12:00", BLANK() }
                }
            )"#,
        )
        .unwrap();
        assert_eq!(
            types(&t),
            vec![
                DataType::String,
                DataType::Int64,
                DataType::Double,
                DataType::Decimal,
                DataType::DateTime,
                DataType::Boolean
            ]
        );
        assert_eq!(vals(&t, 0), vec![Text("a\"b".into()), Null, Text("c".into())]);
        assert_eq!(vals(&t, 1), vec![Int(1), Int(-2), Int(3)]);
        assert_eq!(vals(&t, 2), vec![Real(1.5), Real(3.0), Real(-0.001)]);
        assert_eq!(
            vals(&t, 3),
            vec![Decimal(22_500), Decimal(-5000), Decimal(70_000)]
        );
        assert_eq!(
            vals(&t, 4),
            vec![DateTime(46_027.0), DateTime(46_027.0), DateTime(46_027.5)]
        );
        assert_eq!(vals(&t, 5), vec![Bool(true), Bool(false), Null]);
        let t = load("datatable(\"A\", string, {})").unwrap();
        assert_eq!(t.row_count(), 0);
    }

    #[test]
    fn table_constructors() {
        let t = load("{1, 2, 3}").unwrap();
        assert_eq!(t.columns()[0].name, "Value");
        assert_eq!(vals(&t, 0), vec![Int(1), Int(2), Int(3)]);
        let t = load("{ (1, \"a\", TRUE), (2.5, \"b\", FALSE) }").unwrap();
        assert_eq!(t.columns()[2].name, "Value3");
        assert_eq!(
            types(&t),
            vec![DataType::Double, DataType::String, DataType::Boolean]
        );
        let t = load("{ (1, \"a\"), (\"x\", BLANK()) }").unwrap();
        assert_eq!(vals(&t, 0), vec![Text("1".into()), Text("x".into())]);
        let t = load("{ DT\"2026-01-05T12:00:00\", DATE(2026,1,5), TIME(6,0,0) }").unwrap();
        assert_eq!(
            vals(&t, 0),
            vec![DateTime(46_027.5), DateTime(46_027.0), DateTime(0.25)]
        );
        let t = load("({(1)})").unwrap();
        assert_eq!(vals(&t, 0), vec![Int(1)]);
    }

    #[test]
    fn row_and_calendar() {
        let t = load("ROW(\"A\", 1, \"B\", \"x\")").unwrap();
        assert_eq!(t.row_count(), 1);
        assert_eq!(types(&t), vec![DataType::Int64, DataType::String]);
        // Power BI's auto date table placeholder.
        let t = load("Calendar(Date(2015,1,1), Date(2015,1,1))").unwrap();
        assert_eq!(t.columns()[0].name, "Date");
        assert_eq!(t.row_count(), 1);
        let t = load("CALENDAR(DATE(2024,2,1), DATE(2024,2,29))").unwrap();
        assert_eq!(t.row_count(), 29);
        assert!(load("CALENDAR(DATE(2024,2,2), DATE(2024,2,1))").is_err());
        let limits = Limits {
            max_rows: 10,
            ..Limits::default()
        };
        assert!(matches!(
            load_dax("CALENDAR(DATE(1900,1,1), DATE(9999,12,31))", &limits),
            Err(Error::LimitExceeded(_))
        ));
    }

    #[test]
    fn series_and_nameof() {
        let t = load("GENERATESERIES(0, 20, 5)").unwrap();
        assert_eq!(t.columns()[0].name, "Value");
        assert_eq!(types(&t), vec![DataType::Int64]);
        assert_eq!(vals(&t, 0), vec![Int(0), Int(5), Int(10), Int(15), Int(20)]);
        let t = load("generateseries(0.1, 0.35, 0.1)").unwrap();
        assert_eq!(vals(&t, 0), vec![Real(0.1), Real(0.2), Real(0.3)]);
        assert_eq!(load("GENERATESERIES(-2, 0)").unwrap().row_count(), 3);
        assert_eq!(load("GENERATESERIES(3, 1, 1)").unwrap().row_count(), 0);
        for bad in [
            "GENERATESERIES(0, 10, 0)",
            "GENERATESERIES(0, 10, -1)",
            "GENERATESERIES(0, \"a\")",
            "GENERATESERIES(0)",
            "GENERATESERIES(0, 1, 1, 1)",
            "GENERATESERIES(0, 1e300, 1e-300)",
            "{ NAMEOF([Total]) }",
            "{ NAMEOF('Sales') }",
            "{ NAMEOF(1) }",
        ] {
            assert!(load(bad).is_err(), "{bad}");
        }
        let t = load("{(\"Total\", NAMEOF('Sales'[Total]), 0), (\"It's\", NAMEOF(Sales[A]]b]), 1)}").unwrap();
        assert_eq!(
            vals(&t, 1),
            vec![Text("'Sales'[Total]".into()), Text("'Sales'[A]]b]".into())]
        );
        assert_eq!(
            types(&t),
            vec![DataType::String, DataType::String, DataType::Int64]
        );
        let t = load("{ NAMEOF('O''Brien'[x]) }").unwrap();
        assert_eq!(vals(&t, 0), vec![Text("'O''Brien'[x]".into())]);
    }

    #[test]
    fn dax_date_semantics() {
        assert_eq!(dax_date(2026, 13, 1), dax_date(2027, 1, 1));
        assert_eq!(dax_date(2026, 1, 0), dax_date(2025, 12, 31));
        assert_eq!(dax_date(26, 1, 5), dax_date(1926, 1, 5));
        assert!(dax_date(10_000, 1, 1).is_err());
        assert!(dax_date(9999, 13, 1).is_err());
        assert!(dax_date(i64::MIN, i64::MIN, i64::MIN).is_err());
        assert!(dax_time(i64::MIN, 0, 0).is_err());
        assert_eq!(value::ole_date(1, 1, 1), Ok(-693_593));
        assert_eq!(dax_time(27, 0, 0), Ok(0.125));
        assert!(dax_time(-1, 0, 0).is_err());
    }

    #[test]
    fn unsupported_sources() {
        for e in [
            "Calendar(Date(Year(MIN('Ventes'[Date])), 1, 1), Date(Year(MAX('Ventes'[Date])), 12, 31))",
            "CALENDARAUTO()",
            "FILTER(Sales, Sales[Qty] > 1)",
            "'Sales'",
            "{1 + 2}",
            "{ [Measure] }",
            "{1} & {2}",
            "SUMMARIZECOLUMNS(Sales[Product])",
            "{ NOW() }",
        ] {
            let r = load(e);
            assert!(matches!(r, Err(Error::UnsupportedSource(_))), "{e}: {r:?}");
        }
    }

    #[test]
    fn errors() {
        let r = load("DATATABLE(\"A\", INTEGER, {{1}, {\"x\"}})");
        assert!(
            matches!(&r, Err(Error::Value { row: 2, column, .. }) if column == "A"),
            "{r:?}"
        );
        assert!(matches!(
            load("DATATABLE(\"A\", INTEGER, \"B\", STRING, {{1}})"),
            Err(Error::RowLength { .. })
        ));
        assert!(matches!(
            load("{(1, 2), (3)}"),
            Err(Error::RowLength { row: 2, .. })
        ));
        assert!(matches!(
            load("DATATABLE(\"A\", NUMBER, {})"),
            Err(Error::Syntax { .. })
        ));
        assert!(matches!(
            load("DATATABLE(\"A\", STRING, \"a\", STRING, {})"),
            Err(Error::InvalidTable(_))
        ));
        assert!(matches!(load("{\"open}"), Err(Error::Syntax { .. })));
        assert!(matches!(load("{DT\"nope\"}"), Err(Error::Syntax { .. })));
        assert!(matches!(load("/* x"), Err(Error::Syntax { .. })));
        let deep = format!("{}{{1}}{}", "(".repeat(200), ")".repeat(200));
        assert!(matches!(load(&deep), Err(Error::LimitExceeded(_))));
        let minus = format!("{{{}1}}", "- ".repeat(200));
        assert!(matches!(load(&minus), Err(Error::LimitExceeded(_))));
        for s in [
            "",
            "{",
            "{(",
            "{1,",
            "DATATABLE(",
            "DATATABLE(\"A\"",
            "ROW(",
            "ROW(\"A\",",
            "CALENDAR(",
            "{DATE(",
            "{TRUE(",
            "{1e}",
            "{-}",
            "{9999999999999999999999}",
        ] {
            let _ = load(s);
        }
        assert_eq!(
            vals(&load("{9223372036854775808}").unwrap(), 0),
            vec![Real(9_223_372_036_854_775_808.0)]
        );
        assert_eq!(
            vals(&load("{-9223372036854775808}").unwrap(), 0),
            vec![Real(-9_223_372_036_854_775_808.0)]
        );
    }
}
