//! Typed-in tables: the M query of Power BI Desktop's "Enter data" (docs/formats/data-sources.md,
//! *Desktop's "Enter data"*), written from a grid of cells and read back into one.
//!
//! **Written shape.** The rows are a JSON list of rows of texts, an empty cell as `""` (never
//! `null`, as Desktop writes them), compressed with raw DEFLATE and base64-encoded, read by
//! `Table.FromRows(Json.Document(Binary.Decompress(…)))` into text columns, then typed by one
//! `Table.TransformColumnTypes` step without a culture: it reads the texts with the model's query
//! culture (`""` is null in a typed column), so every cell is written as that culture reads it
//! back (numbers with its decimal separator and no grouping, dates and times in ISO 8601,
//! logicals as `true`/`false`). A text column holds no null: an empty cell is an empty text.
//! Power BI Desktop 2.159.1231.0's Create Table dialog (Edit of the Source step) failed on rows
//! holding `null` (2026-11-23, docs/formats/data-sources.md).
//!
//! **Read shapes.** That query, Desktop's own (with or without the type step), and the plain
//! `#table` literal `table.import {csv}` writes: any M that names no library function beyond the
//! ones these shapes use ([`is_entered`]). The query is evaluated and its values written back as
//! texts in the culture, so a value reads the same after an edit.

use super::lexer::{Tok, tokenize};
use crate::value::{self as cv, Culture};
use crate::{CellValue, DataType, DateKind, Error, Limits, SourceOptions, TableData};

/// Most cells an entered table holds (written or read back into the grid).
pub const MAX_CELLS: usize = 1_000_000;
/// Most columns.
pub const MAX_COLUMNS: usize = 1_000;
/// Rows of a column that [`infer_texts`] looks at (Power Query detects types from the first 200
/// rows).
pub const DETECT_ROWS: usize = 200;

/// A column type of an entered table, as Desktop's data type menu names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnteredType {
    Text,
    /// Whole number (`Int64.Type`).
    Whole,
    /// Decimal number (`type number`).
    Number,
    /// Fixed decimal number (`Currency.Type`).
    Fixed,
    Date,
    /// Date/time (`type datetime`).
    DateTime,
    /// True/false (`type logical`).
    Logical,
}

impl EnteredType {
    pub const ALL: [EnteredType; 7] = [
        EnteredType::Text,
        EnteredType::Whole,
        EnteredType::Number,
        EnteredType::Fixed,
        EnteredType::Date,
        EnteredType::DateTime,
        EnteredType::Logical,
    ];

    /// The name in commands (`text`, `whole`, `number`, `fixed`, `date`, `datetime`, `logical`).
    pub fn name(self) -> &'static str {
        match self {
            EnteredType::Text => "text",
            EnteredType::Whole => "whole",
            EnteredType::Number => "number",
            EnteredType::Fixed => "fixed",
            EnteredType::Date => "date",
            EnteredType::DateTime => "datetime",
            EnteredType::Logical => "logical",
        }
    }

    /// A type name, or one of its aliases (the model's: `string`, `int64`, `double`, `decimal`,
    /// `dateTime`, `boolean`), any case.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "text" | "string" => EnteredType::Text,
            "whole" | "int64" | "integer" | "int" => EnteredType::Whole,
            "number" | "double" => EnteredType::Number,
            "fixed" | "decimal" | "currency" => EnteredType::Fixed,
            "date" => EnteredType::Date,
            "datetime" => EnteredType::DateTime,
            "logical" | "boolean" | "bool" => EnteredType::Logical,
            _ => return None,
        })
    }

    /// The M type `Table.TransformColumnTypes` gives the column.
    pub fn m_type(self) -> &'static str {
        match self {
            EnteredType::Text => "type text",
            EnteredType::Whole => "Int64.Type",
            EnteredType::Number => "type number",
            EnteredType::Fixed => "Currency.Type",
            EnteredType::Date => "type date",
            EnteredType::DateTime => "type datetime",
            EnteredType::Logical => "type logical",
        }
    }

    /// The model's data type of the column.
    pub fn data_type(self) -> DataType {
        match self {
            EnteredType::Text => DataType::String,
            EnteredType::Whole => DataType::Int64,
            EnteredType::Number => DataType::Double,
            EnteredType::Fixed => DataType::Decimal,
            EnteredType::Date | EnteredType::DateTime => DataType::DateTime,
            EnteredType::Logical => DataType::Boolean,
        }
    }

    /// The type of a loaded column (`None`: binary or unknown, which a grid cannot hold).
    pub fn of_column(data_type: DataType, date_kind: Option<DateKind>, values: &[CellValue]) -> Option<Self> {
        Some(match data_type {
            DataType::String => EnteredType::Text,
            DataType::Int64 => EnteredType::Whole,
            DataType::Double => EnteredType::Number,
            DataType::Decimal => EnteredType::Fixed,
            DataType::Boolean => EnteredType::Logical,
            DataType::DateTime => match date_kind {
                Some(DateKind::Date) => EnteredType::Date,
                Some(_) => EnteredType::DateTime,
                None if values
                    .iter()
                    .all(|v| !matches!(v, CellValue::DateTime(x) if x.fract() != 0.0)) =>
                {
                    EnteredType::Date
                }
                None => EnteredType::DateTime,
            },
            _ => return None,
        })
    }
}

/// A cell given to [`build`].
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Null,
    /// Text as typed, read with the input culture.
    Text(String),
    Number(f64),
    Logical(bool),
}

/// An entered table: its columns and types, the cells as the query stores them (texts in the
/// query culture), their typed values, and the M query.
#[derive(Debug, Clone, PartialEq)]
pub struct Entered {
    pub columns: Vec<(String, EnteredType)>,
    /// One text (or `None`, a null) per cell, row by row.
    pub texts: Vec<Vec<Option<String>>>,
    /// The typed value of each cell, column by column (what the query loads).
    pub values: Vec<Vec<CellValue>>,
    pub m: String,
}

/// The type Power Query detects for a column of texts: whole number, decimal number,
/// true/false, date, date/time, else text (empty texts are skipped; a column of empty texts is
/// text). Looks at the first [`DETECT_ROWS`] texts.
pub fn infer_texts<'a>(texts: impl IntoIterator<Item = &'a str>, culture: Culture) -> EnteredType {
    let texts: Vec<&str> = texts
        .into_iter()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .take(DETECT_ROWS)
        .collect();
    if texts.is_empty() {
        return EnteredType::Text;
    }
    let all = |to: DataType| texts.iter().all(|t| cv::parse_text_as(t, to, culture).is_ok());
    if texts.iter().all(|t| cv::parse_int(t).is_some()) {
        return EnteredType::Whole;
    }
    let sep = culture.decimal_separator;
    if texts.iter().all(|t| cv::parse_double(t, sep).is_some()) {
        return EnteredType::Number;
    }
    if texts.iter().all(|t| cv::parse_bool(t).is_some()) {
        return EnteredType::Logical;
    }
    if all(DataType::DateTime) {
        let whole = texts.iter().all(|t| {
            matches!(cv::parse_text_as(t, DataType::DateTime, culture), Ok(CellValue::DateTime(x)) if x.fract() == 0.0)
        });
        return if whole {
            EnteredType::Date
        } else {
            EnteredType::DateTime
        };
    }
    EnteredType::Text
}

/// The type Power Query detects for a loaded column: a text column by [`infer_texts`], a number
/// column whole numbers or decimals, a date/time column dates when every value is a whole day.
pub fn infer_column(
    data_type: DataType,
    date_kind: Option<DateKind>,
    values: &[CellValue],
    culture: Culture,
) -> Option<EnteredType> {
    let head = values.get(..values.len().min(DETECT_ROWS)).unwrap_or(values);
    match data_type {
        DataType::String => Some(infer_texts(
            head.iter().filter_map(|v| match v {
                CellValue::Text(t) => Some(t.as_str()),
                _ => None,
            }),
            culture,
        )),
        DataType::Int64 => Some(EnteredType::Whole),
        DataType::Double => Some(
            if head
                .iter()
                .all(|v| !matches!(v, CellValue::Real(x) if x.fract() != 0.0 || !x.is_finite()))
            {
                EnteredType::Whole
            } else {
                EnteredType::Number
            },
        ),
        DataType::DateTime => Some(match date_kind {
            // a date/time at midnight everywhere is a date (an Excel date cell)
            Some(DateKind::DateTimeZone | DateKind::Time) => EnteredType::DateTime,
            _ => EnteredType::of_column(data_type, None, head)?,
        }),
        other => EnteredType::of_column(other, date_kind, head),
    }
}

/// The text a cell of type `ty` is stored as, so that `Table.TransformColumnTypes` reads it back
/// with `culture` as the same value.
pub fn cell_text(v: &CellValue, ty: EnteredType, culture: Culture) -> Option<String> {
    let sep = culture.decimal_separator;
    Some(match (v, ty) {
        (CellValue::Null, _) => return None,
        (CellValue::Text(t), _) => t.clone(),
        (CellValue::Int(n), _) => n.to_string(),
        (CellValue::Real(x), _) => {
            if x.is_finite() && x.fract() == 0.0 && x.abs() < 1e15 {
                format!("{}", *x as i64)
            } else {
                format!("{x}").replace('.', &sep.to_string())
            }
        }
        (CellValue::Decimal(n), _) => cv::format_decimal(*n, sep),
        (CellValue::Bool(b), _) => if *b { "true" } else { "false" }.to_string(),
        (CellValue::DateTime(x), EnteredType::Date) => cv::format_datetime(x.floor()),
        (CellValue::DateTime(x), _) => cv::format_datetime(*x),
    })
}

/// The typed value of `cell` in a column of type `ty`, reading text with `culture`.
fn typed(cell: &Cell, ty: EnteredType, culture: Culture) -> Result<CellValue, String> {
    let to = ty.data_type();
    let v = match cell {
        Cell::Null => return Ok(CellValue::Null),
        Cell::Text(t) if ty == EnteredType::Text => return Ok(CellValue::Text(t.clone())),
        Cell::Text(t) => cv::parse_text_as(t, to, culture)?,
        Cell::Number(x) if !x.is_finite() => return Err(format!("{x} is not a number a cell can hold")),
        Cell::Number(x) => match ty {
            EnteredType::Text => {
                CellValue::Text(cell_text(&CellValue::Real(*x), ty, Culture::INVARIANT).unwrap_or_default())
            }
            EnteredType::Whole => match cv::int_from_f64(*x) {
                Some(n) => CellValue::Int(n),
                None => return Err(format!("{x} is not a whole number")),
            },
            EnteredType::Number => CellValue::Real(*x),
            EnteredType::Fixed => match cv::decimal_from_f64(*x) {
                Some(n) => CellValue::Decimal(n),
                None => return Err(format!("{x} is out of the fixed decimal range")),
            },
            EnteredType::Date | EnteredType::DateTime | EnteredType::Logical => {
                return Err(format!("{x} is a number, not a {}", ty.name()));
            }
        },
        Cell::Logical(b) => match ty {
            EnteredType::Logical => CellValue::Bool(*b),
            EnteredType::Text => CellValue::Text(b.to_string()),
            _ => return Err(format!("{b} is true/false, not a {}", ty.name())),
        },
    };
    Ok(match (v, ty) {
        // a time typed into a date column: the day
        (CellValue::DateTime(x), EnteredType::Date) => CellValue::DateTime(x.floor()),
        (v, _) => v,
    })
}

fn json_text(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// M text literal body (`"` doubled, control characters escaped).
fn m_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\"\""),
            '#' => out.push_str("#(#)"),
            '\r' => out.push_str("#(cr)"),
            '\n' => out.push_str("#(lf)"),
            '\t' => out.push_str("#(tab)"),
            c if c.is_control() => out.push_str(&format!("#({:04X})", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// M keywords, which a bare field name cannot be.
const KEYWORDS: &[&str] = &[
    "and",
    "as",
    "each",
    "else",
    "error",
    "false",
    "if",
    "in",
    "is",
    "let",
    "meta",
    "not",
    "null",
    "or",
    "otherwise",
    "section",
    "shared",
    "then",
    "true",
    "try",
    "type",
];

/// A field name of a record type: bare when it is a plain identifier, else `#"…"`.
pub(crate) fn m_field(name: &str) -> String {
    let mut chars = name.chars();
    let plain = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !KEYWORDS.contains(&name);
    if plain {
        name.to_string()
    } else {
        format!("#{}", m_text(name))
    }
}

/// A text literal for M (`"C:\data\a.csv"`).
pub(crate) fn m_literal(s: &str) -> String {
    m_text(s)
}

/// The `{{"A", type text}, {"B", Int64.Type}}` list of a `Table.TransformColumnTypes` step.
pub(crate) fn type_list(columns: &[(String, EnteredType)]) -> String {
    let pairs: Vec<String> = columns
        .iter()
        .map(|(n, t)| format!("{{{}, {}}}", m_text(n), t.m_type()))
        .collect();
    format!("{{{}}}", pairs.join(", "))
}

fn check_columns(columns: &[(String, Option<EnteredType>)]) -> Result<(), Error> {
    let bad = |m: String| Err(Error::InvalidTable(m));
    if columns.is_empty() {
        return bad("an entered table needs at least one column".into());
    }
    if columns.len() > MAX_COLUMNS {
        return bad(format!("an entered table has at most {MAX_COLUMNS} columns"));
    }
    for (i, (n, _)) in columns.iter().enumerate() {
        if n.trim().is_empty() {
            return bad(format!("column {} has no name", i + 1));
        }
        if n.len() > 1024 {
            return bad(format!("column {} has a name longer than 1024 bytes", i + 1));
        }
        if columns
            .iter()
            .take(i)
            .any(|(m, _)| m.to_lowercase() == n.to_lowercase())
        {
            return bad(format!("two columns are named `{n}`"));
        }
    }
    Ok(())
}

/// The entered table of `columns` (a `None` type is detected from the cells, [`infer_texts`])
/// and `rows` (shorter rows are padded with nulls): cells typed as text are read with
/// `input_culture`, then stored as `query_culture` reads them back.
pub fn build(
    columns: &[(String, Option<EnteredType>)],
    rows: &[Vec<Cell>],
    input_culture: Culture,
    query_culture: Culture,
) -> Result<Entered, Error> {
    check_columns(columns)?;
    let width = columns.len();
    if rows.iter().any(|r| r.len() > width) {
        return Err(Error::InvalidTable(format!(
            "a row has more cells than the {width} columns"
        )));
    }
    if rows.len().saturating_mul(width) > MAX_CELLS {
        return Err(Error::LimitExceeded(format!(
            "an entered table holds at most {MAX_CELLS} cells"
        )));
    }
    let cell = |r: &Vec<Cell>, c: usize| r.get(c).cloned().unwrap_or(Cell::Null);
    let mut typed_cols = Vec::with_capacity(width);
    let mut values = Vec::with_capacity(width);
    for (ci, (name, ty)) in columns.iter().enumerate() {
        let ty = match ty {
            Some(t) => *t,
            None => {
                // numbers and logicals given as JSON count as their text
                let texts: Vec<String> = rows
                    .iter()
                    .filter_map(|r| match cell(r, ci) {
                        Cell::Text(t) => Some(t),
                        Cell::Number(x) => cell_text(&CellValue::Real(x), EnteredType::Number, input_culture),
                        Cell::Logical(b) => Some(b.to_string()),
                        Cell::Null => None,
                    })
                    .collect();
                infer_texts(texts.iter().map(String::as_str), input_culture)
            }
        };
        let mut col = Vec::with_capacity(rows.len());
        for (ri, r) in rows.iter().enumerate() {
            let c = cell(r, ci);
            // an empty text is a null outside text columns; a text column has no null (Desktop
            // stores an empty cell as an empty text)
            let c = match c {
                Cell::Text(t) if ty != EnteredType::Text && t.trim().is_empty() => Cell::Null,
                Cell::Null if ty == EnteredType::Text => Cell::Text(String::new()),
                c => c,
            };
            col.push(typed(&c, ty, input_culture).map_err(|message| Error::Value {
                row: ri + 1,
                column: name.clone(),
                message,
            })?);
        }
        typed_cols.push((name.clone(), ty));
        values.push(col);
    }
    let texts: Vec<Vec<Option<String>>> = (0..rows.len())
        .map(|ri| {
            typed_cols
                .iter()
                .zip(&values)
                .map(|((_, ty), col)| col.get(ri).and_then(|v| cell_text(v, *ty, query_culture)))
                .collect()
        })
        .collect();
    let m = write(&typed_cols, &texts);
    Ok(Entered {
        columns: typed_cols,
        texts,
        values,
        m,
    })
}

/// The M query of an entered table (see the module docs).
pub fn write(columns: &[(String, EnteredType)], texts: &[Vec<Option<String>>]) -> String {
    let mut json = String::from("[");
    for (i, row) in texts.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        json.push('[');
        for (j, c) in row.iter().enumerate() {
            if j > 0 {
                json.push(',');
            }
            // Desktop writes an empty cell as "" (its Create Table dialog fails on a null);
            // the type step reads "" as null in a typed column
            json_text(c.as_deref().unwrap_or(""), &mut json);
        }
        json.push(']');
    }
    json.push(']');
    let packed = miniz_oxide::deflate::compress_to_vec(json.as_bytes(), 9);
    let b64 = super::lib::misc::base64_encode(&packed);
    let fields: Vec<String> = columns
        .iter()
        .map(|(n, _)| format!("{} = _t", m_field(n)))
        .collect();
    format!(
        "let\n    Source = Table.FromRows(Json.Document(Binary.Decompress(Binary.FromText(\"{b64}\", \
         BinaryEncoding.Base64), Compression.Deflate)), let _t = ((type nullable text) meta \
         [Serialized.Text = true]) in type table [{}]),\n    #\"Changed Type\" = \
         Table.TransformColumnTypes(Source,{})\nin\n    #\"Changed Type\"",
        fields.join(", "),
        type_list(columns)
    )
}

/// The library names an entered table's query may use.
const ALLOWED: &[&str] = &[
    "Table.FromRows",
    "Json.Document",
    "Binary.Decompress",
    "Binary.FromText",
    "BinaryEncoding.Base64",
    "Compression.Deflate",
    "Table.TransformColumnTypes",
    "Serialized.Text",
    "Int64.Type",
    "Int32.Type",
    "Int16.Type",
    "Int8.Type",
    "Currency.Type",
    "Percentage.Type",
    "Single.Type",
    "Double.Type",
    "Decimal.Type",
];

/// Whether `m` is an entered table's query: Desktop's Enter data (compressed rows) or a
/// `#table` literal, with only a type step after it (no file, connector or other query is
/// read: every dotted name is one these shapes use).
pub fn is_entered(m: &str) -> bool {
    let limits = Limits::default();
    let Ok(tokens) = tokenize(m, &limits) else {
        return false;
    };
    let mut data = false;
    for t in &tokens {
        match &t.tok {
            Tok::Ident(n) if n.contains('.') => {
                if !ALLOWED.contains(&n.as_str()) {
                    return false;
                }
                if n == "Binary.Decompress" {
                    data = true;
                }
            }
            Tok::Hash(h) if h == "#table" => data = true,
            Tok::Hash(h) if h == "#shared" || h == "#sections" => return false,
            _ => {}
        }
    }
    data
}

/// An entered table read back from its query: the columns and types of the loaded data, the
/// cells as texts in `opts`' culture, and `m` as it is. Fails when `m` is not an entered table
/// ([`is_entered`]) or does not load.
pub fn read(m: &str, opts: &SourceOptions) -> Result<Entered, Error> {
    if !is_entered(m) {
        return Err(Error::UnsupportedSource(
            "the query is not an entered table (Enter data or a #table literal, then a type step)".into(),
        ));
    }
    let mut opts = opts.clone();
    // nothing to read outside the query
    opts.file_roots.clear();
    opts.shared.clear();
    opts.connectors = Default::default();
    let data: TableData = super::load_m_with(m, &opts)?;
    let culture = Culture::from_name(opts.culture.as_deref().unwrap_or("en-US"));
    let cols = data.columns();
    if data.row_count().saturating_mul(cols.len()) > MAX_CELLS {
        return Err(Error::LimitExceeded(format!(
            "the table has more than {MAX_CELLS} cells to edit"
        )));
    }
    let mut columns = Vec::with_capacity(cols.len());
    for (i, c) in cols.iter().enumerate() {
        let ty = EnteredType::of_column(c.data_type, data.date_kind(i), &c.values).ok_or_else(|| {
            Error::UnsupportedSource(format!("column `{}` holds values a grid cannot edit", c.name))
        })?;
        columns.push((c.name.clone(), ty));
    }
    let texts = (0..data.row_count())
        .map(|ri| {
            cols.iter()
                .zip(&columns)
                .map(|(c, (_, ty))| c.values.get(ri).and_then(|v| cell_text(v, *ty, culture)))
                .collect()
        })
        .collect();
    let values = cols.iter().map(|c| c.values.clone()).collect();
    Ok(Entered {
        columns,
        texts,
        values,
        m: m.to_string(),
    })
}
