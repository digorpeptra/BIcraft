//! CSV reader (RFC 4180 plus the usual real-world variations) with type
//! inference, and a writer that round-trips its output.

use std::borrow::Cow;

use crate::value::{self, Culture};
use crate::{CellValue, ColumnData, DataType, Error, Limits, TableData};

#[path = "csv_fast.rs"]
mod fast;

/// CSV reading options.
#[derive(Debug, Clone, Default)]
pub struct CsvOptions {
    /// Field delimiter; `None` auto-detects among `,`, `;` and tab.
    pub delimiter: Option<char>,
    /// When `true` (the default via [`CsvOptions::new`]) the first record holds the column names;
    /// otherwise columns are named `Column1`, `Column2`, …
    pub has_header: bool,
    /// Culture for numbers and non-ISO dates.
    pub culture: Culture,
    /// Explicit column types by column name (case-insensitive); other columns are inferred.
    pub column_types: Vec<(String, DataType)>,
    /// When `false`, columns without an explicit type are read as text.
    pub infer_types: bool,
    /// Size caps.
    pub limits: Limits,
}

impl CsvOptions {
    /// Header row, auto-detected delimiter, invariant culture, type inference on.
    pub fn new() -> Self {
        Self {
            has_header: true,
            infer_types: true,
            ..Self::default()
        }
    }

    /// Same as [`CsvOptions::new`] with a culture.
    pub fn with_culture(culture: Culture) -> Self {
        Self {
            culture,
            ..Self::new()
        }
    }
}

/// One raw field: its text, and whether it was quoted (`""` is an empty
/// string, an unquoted empty field is blank).
#[derive(Debug, Clone, PartialEq)]
struct Field {
    text: String,
    quoted: bool,
}

/// Record-at-a-time RFC 4180 tokenizer.
struct Records<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    delim: char,
    line: usize,
    max_cell: usize,
    max_columns: usize,
    done: bool,
}

impl<'a> Records<'a> {
    fn new(text: &'a str, delim: char, limits: &Limits) -> Self {
        Self {
            chars: text.chars().peekable(),
            delim,
            line: 1,
            max_cell: limits.max_cell_bytes,
            max_columns: limits.max_columns,
            done: false,
        }
    }

    fn err(&self, message: impl Into<String>) -> Error {
        Error::Syntax {
            format: "csv",
            line: self.line,
            column: 0,
            message: message.into(),
        }
    }

    /// Consumes a line ending if one is next; returns whether it did.
    fn eat_newline(&mut self) -> bool {
        match self.chars.peek() {
            Some('\r') => {
                self.chars.next();
                if self.chars.peek() == Some(&'\n') {
                    self.chars.next();
                }
                self.line += 1;
                true
            }
            Some('\n') => {
                self.chars.next();
                self.line += 1;
                true
            }
            _ => false,
        }
    }

    fn push(&self, s: &mut String, c: char) -> Result<(), Error> {
        if s.len() + c.len_utf8() > self.max_cell {
            return Err(self.err(format!("a field is longer than {} bytes", self.max_cell)));
        }
        s.push(c);
        Ok(())
    }

    /// The next record with its starting line, or `None` at end of input.
    fn next_record(&mut self) -> Result<Option<(usize, Vec<Field>)>, Error> {
        if self.done || self.chars.peek().is_none() {
            self.done = true;
            return Ok(None);
        }
        let start_line = self.line;
        let mut fields = Vec::new();
        loop {
            let mut text = String::new();
            let mut quoted = false;
            if self.chars.peek() == Some(&'"') {
                quoted = true;
                self.chars.next();
                let open_line = self.line;
                loop {
                    match self.chars.next() {
                        None => {
                            return Err(Error::Syntax {
                                format: "csv",
                                line: open_line,
                                column: 0,
                                message: "unterminated quoted field".into(),
                            });
                        }
                        Some('"') => {
                            if self.chars.peek() == Some(&'"') {
                                self.chars.next();
                                self.push(&mut text, '"')?;
                            } else {
                                break;
                            }
                        }
                        Some(c) => {
                            if c == '\n' || (c == '\r' && self.chars.peek() != Some(&'\n')) {
                                self.line += 1;
                            }
                            self.push(&mut text, c)?;
                        }
                    }
                }
                match self.chars.peek() {
                    None | Some('\r' | '\n') => {}
                    Some(&c) if c == self.delim => {}
                    Some(&c) => {
                        return Err(self.err(format!("unexpected `{c}` after a closing quote")));
                    }
                }
            } else {
                while let Some(&c) = self.chars.peek() {
                    if c == self.delim || c == '\r' || c == '\n' {
                        break;
                    }
                    self.push(&mut text, c)?;
                    self.chars.next();
                }
            }
            fields.push(Field { text, quoted });
            if fields.len() > self.max_columns {
                return Err(Error::LimitExceeded(format!(
                    "line {start_line} has more than {} fields",
                    self.max_columns
                )));
            }
            match self.chars.peek() {
                Some(&c) if c == self.delim => {
                    self.chars.next();
                }
                _ => {
                    self.eat_newline();
                    return Ok(Some((start_line, fields)));
                }
            }
        }
    }
}

/// Decodes the input: UTF-8 (BOM stripped) or UTF-16LE/BE with a BOM.
fn decode(bytes: &[u8]) -> Result<Cow<'_, str>, Error> {
    let utf16 = |be: bool| -> Result<Cow<'_, str>, Error> {
        let units: Vec<u16> = bytes
            .get(2..)
            .unwrap_or_default()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&p| {
                if be {
                    u16::from_be_bytes(p)
                } else {
                    u16::from_le_bytes(p)
                }
            })
            .collect();
        String::from_utf16(&units)
            .map(Cow::Owned)
            .map_err(|_| Error::Syntax {
                format: "csv",
                line: 0,
                column: 0,
                message: "invalid UTF-16 text".into(),
            })
    };
    match bytes {
        [0xFF, 0xFE, ..] => utf16(false),
        [0xFE, 0xFF, ..] => utf16(true),
        _ => {
            let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
            std::str::from_utf8(body).map(Cow::Borrowed).map_err(|e| {
                let line = body
                    .get(..e.valid_up_to())
                    .map_or(0, |b| b.iter().filter(|&&c| c == b'\n').count() + 1);
                Error::Syntax {
                    format: "csv",
                    line,
                    column: 0,
                    message: format!("invalid UTF-8 at byte {}", e.valid_up_to()),
                }
            })
        }
    }
}

/// Picks the delimiter whose first records split most consistently.
fn detect_delimiter(text: &str, limits: &Limits) -> char {
    const SAMPLE: usize = 20;
    let mut best = (',', 0usize, false);
    for cand in ['\t', ';', ','] {
        let mut rec = Records::new(text, cand, limits);
        let mut counts = Vec::new();
        while counts.len() < SAMPLE {
            match rec.next_record() {
                Ok(Some((_, f))) => {
                    if !(f.len() == 1 && f.first().is_some_and(|x| x.text.is_empty() && !x.quoted)) {
                        counts.push(f.len());
                    }
                }
                Ok(None) => break,
                Err(_) => {
                    counts.clear();
                    break;
                }
            }
        }
        let Some(&first) = counts.first() else { continue };
        let consistent = counts.iter().all(|&n| n == first);
        if first > 1 && ((consistent && !best.2) || (consistent == best.2 && first > best.1)) {
            best = (cand, first, consistent);
        }
    }
    best.0
}

/// Reads CSV bytes into a [`TableData`].
///
/// Unquoted empty cells are blank (`Null`); a quoted empty cell (`""`) is an
/// empty string in text columns. Blank lines are skipped unless the table has a
/// single column. Rows must have as many fields as the header.
///
/// The input is parsed in parallel chunks and typed straight into the columns
/// (`docs/formats/data-sources.md`, *Fast CSV loading*); the result, errors included, is the
/// same as a sequential read. Inputs whose table would need more memory than
/// [`Limits::max_load_bytes`] are refused.
pub fn read_csv(bytes: &[u8], opts: &CsvOptions) -> Result<TableData, Error> {
    let limits = &opts.limits;
    limits.check_bytes(bytes.len(), "CSV input")?;
    let text = decode(bytes)?;
    let delim = match opts.delimiter {
        Some(d) if d == '"' || d == '\r' || d == '\n' => {
            return Err(Error::UnsupportedSource(format!(
                "`{}` cannot be a CSV delimiter",
                d.escape_default()
            )));
        }
        Some(d) => d,
        None => detect_delimiter(&text, limits),
    };
    match fast::read(&text, delim, opts)? {
        Some(t) => Ok(t),
        // The fast reader found a malformed record: the sequential tokenizer names it exactly
        // (without keeping the cells), else the sequential reader decides.
        None => {
            structure(&text, delim, opts)?;
            read_text(&text, delim, opts)
        }
    }
}

/// The record-level checks of [`read_text`] in its order (syntax, field count, row count,
/// row length), keeping nothing: the first error, exactly as `read_text` reports it.
fn structure(text: &str, delim: char, opts: &CsvOptions) -> Result<(), Error> {
    let limits = &opts.limits;
    let mut records = Records::new(text, delim, limits);
    let Some((_, first)) = records.next_record()? else {
        return Ok(());
    };
    let width = first.len();
    let mut rows = usize::from(!opts.has_header);
    while let Some((_, fields)) = records.next_record()? {
        let blank = fields.len() == 1 && fields.first().is_some_and(|f| f.text.is_empty() && !f.quoted);
        if blank && width > 1 {
            continue;
        }
        rows += 1;
        limits.check_rows(rows)?;
        if fields.len() != width {
            return Err(Error::RowLength {
                row: rows,
                expected: width,
                found: fields.len(),
            });
        }
    }
    Ok(())
}

/// The sequential reader (the reference the fast reader matches): `text` decoded, `delim`
/// chosen.
fn read_text(text: &str, delim: char, opts: &CsvOptions) -> Result<TableData, Error> {
    let limits = &opts.limits;
    let mut records = Records::new(text, delim, limits);

    let first = records.next_record()?;
    let Some((_, first)) = first else {
        return TableData::new(Vec::new());
    };
    let width = first.len();
    let mut names: Vec<String> = Vec::with_capacity(width);
    let mut cells: Vec<Vec<Field>> = (0..width).map(|_| Vec::new()).collect();
    let mut rows = 0usize;
    if opts.has_header {
        for (i, f) in first.into_iter().enumerate() {
            let name = f.text.trim().to_string();
            names.push(if name.is_empty() {
                format!("Column{}", i + 1)
            } else {
                name
            });
        }
    } else {
        names.extend((1..=width).map(|i| format!("Column{i}")));
        add_row(&mut cells, first, 1, width)?;
        rows = 1;
    }
    while let Some((_, fields)) = records.next_record()? {
        let blank = fields.len() == 1 && fields.first().is_some_and(|f| f.text.is_empty() && !f.quoted);
        if blank && width > 1 {
            continue;
        }
        rows += 1;
        limits.check_rows(rows)?;
        add_row(&mut cells, fields, rows, width)?;
    }

    let mut columns = Vec::with_capacity(width);
    for (name, raw) in names.into_iter().zip(cells) {
        let explicit = opts
            .column_types
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(&name))
            .map(|(_, t)| *t);
        let data_type = match explicit {
            Some(t) => t,
            None if opts.infer_types => infer_type(&raw, opts.culture),
            None => DataType::String,
        };
        let mut values = Vec::with_capacity(raw.len());
        for (i, f) in raw.into_iter().enumerate() {
            values.push(
                cell_value(f, data_type, opts.culture).map_err(|message| Error::Value {
                    row: i + 1,
                    column: name.clone(),
                    message,
                })?,
            );
        }
        columns.push(ColumnData {
            name,
            data_type,
            values,
        });
    }
    TableData::new(columns)
}

fn add_row(cells: &mut [Vec<Field>], fields: Vec<Field>, row: usize, width: usize) -> Result<(), Error> {
    if fields.len() != width {
        return Err(Error::RowLength {
            row,
            expected: width,
            found: fields.len(),
        });
    }
    for (col, f) in cells.iter_mut().zip(fields) {
        col.push(f);
    }
    Ok(())
}

fn cell_value(f: Field, t: DataType, culture: Culture) -> Result<CellValue, String> {
    if f.text.is_empty()
        && !(f.quoted && matches!(t, DataType::String | DataType::Unknown | DataType::Variant))
    {
        return Ok(CellValue::Null);
    }
    match t {
        DataType::String | DataType::Unknown | DataType::Variant => Ok(CellValue::Text(f.text)),
        _ => value::parse_text_as(&f.text, t, culture),
    }
}

/// First type in the order int64 → double → dateTime → boolean that every
/// non-empty cell parses as; else string. All-empty columns are strings. Numbers written with
/// the culture's currency symbol (`$2.675`, `2,675 €`) make a fixed decimal (currency) column.
fn infer_type(cells: &[Field], culture: Culture) -> DataType {
    let mut inf = Inference::new();
    for f in cells {
        if !inf.add(&f.text, culture) {
            break;
        }
    }
    inf.result()
}

/// Type inference over a column's texts, in pieces that merge (the fast reader infers each
/// chunk of a column on its own thread).
#[derive(Debug, Clone, Copy)]
struct Inference {
    alive: [bool; 4],
    any: bool,
    currency: bool,
}

impl Inference {
    const CANDIDATES: [DataType; 4] = [
        DataType::Int64,
        DataType::Double,
        DataType::DateTime,
        DataType::Boolean,
    ];

    fn new() -> Self {
        Self {
            alive: [true; 4],
            any: false,
            currency: false,
        }
    }

    /// Adds one cell's text; `false` once no candidate is left (the column is text).
    fn add(&mut self, text: &str, culture: Culture) -> bool {
        let parses = |t: DataType, s: &str| match t {
            DataType::Int64 => value::fast_int(s).is_some() || value::parse_int(s).is_some(),
            DataType::Double => {
                value::parse_double(s, culture.decimal_separator).is_some()
                    || culture
                        .without_currency(s)
                        .is_some_and(|n| value::parse_double(&n, culture.decimal_separator).is_some())
            }
            DataType::DateTime => value::parse_datetime(s, culture).is_some(),
            _ => value::parse_bool(s).is_some(),
        };
        let s = text.trim();
        if s.is_empty() {
            return true;
        }
        self.any = true;
        if culture.without_currency(s).is_some() {
            self.currency = true;
        }
        for (ok, t) in self.alive.iter_mut().zip(Self::CANDIDATES) {
            if *ok && !parses(t, s) {
                *ok = false;
            }
        }
        self.alive.contains(&true)
    }

    fn merge(&mut self, other: &Inference) {
        for (a, b) in self.alive.iter_mut().zip(other.alive) {
            *a &= b;
        }
        self.any |= other.any;
        self.currency |= other.currency;
    }

    /// First type in the order int64 → double → dateTime → boolean that every non-empty cell
    /// parses as; else string (also when every cell is empty).
    fn result(&self) -> DataType {
        if !self.any || !self.alive.contains(&true) {
            return DataType::String;
        }
        match self
            .alive
            .iter()
            .zip(Self::CANDIDATES)
            .find(|(ok, _)| **ok)
            .map_or(DataType::String, |(_, t)| t)
        {
            DataType::Double if self.currency => DataType::Decimal,
            t => t,
        }
    }
}

/// CSV writing options.
#[derive(Debug, Clone)]
pub struct CsvWriteOptions {
    pub delimiter: char,
    /// Decimal separator of numbers; dates are always ISO 8601.
    pub culture: Culture,
    pub header: bool,
    /// `"\r\n"` (RFC 4180, the default) or `"\n"`.
    pub line_ending: &'static str,
}

impl Default for CsvWriteOptions {
    fn default() -> Self {
        Self {
            delimiter: ',',
            culture: Culture::INVARIANT,
            header: true,
            line_ending: "\r\n",
        }
    }
}

/// Writes a table as CSV. Blanks are empty fields, empty strings are `""`,
/// numbers use the culture's decimal separator (doubles keep full precision),
/// dates are ISO 8601, booleans `true`/`false`. Reading the output back with
/// the same culture and the table's column types yields the same table.
pub fn write_csv(data: &TableData, opts: &CsvWriteOptions) -> String {
    let mut out = String::new();
    let sep = opts.culture.decimal_separator;
    let push_field = |out: &mut String, s: &str, force: bool| {
        let needs = force
            || s.contains(opts.delimiter)
            || s.contains(['"', '\r', '\n'])
            || (out.is_empty() && s.starts_with('\u{feff}'));
        if needs {
            out.push('"');
            out.push_str(&s.replace('"', "\"\""));
            out.push('"');
        } else {
            out.push_str(s);
        }
    };
    let cols = data.columns();
    if opts.header {
        for (i, c) in cols.iter().enumerate() {
            if i > 0 {
                out.push(opts.delimiter);
            }
            push_field(&mut out, &c.name, false);
        }
        out.push_str(opts.line_ending);
    }
    for r in 0..data.row_count() {
        for (i, c) in cols.iter().enumerate() {
            if i > 0 {
                out.push(opts.delimiter);
            }
            match c.values.get(r).unwrap_or(&CellValue::Null) {
                CellValue::Null => {}
                CellValue::Text(t) => push_field(&mut out, t, t.is_empty()),
                CellValue::Int(n) => push_field(&mut out, &n.to_string(), false),
                CellValue::Real(x) => push_field(&mut out, &value::format_double(*x, sep), false),
                CellValue::Decimal(n) => push_field(&mut out, &value::format_decimal(*n, sep), false),
                CellValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
                CellValue::DateTime(d) => push_field(&mut out, &value::format_datetime(*d), false),
            }
        }
        out.push_str(opts.line_ending);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use CellValue::{Bool, DateTime, Int, Null, Real, Text};

    fn read(s: &str) -> TableData {
        read_csv(s.as_bytes(), &CsvOptions::new()).unwrap()
    }

    fn types(t: &TableData) -> Vec<DataType> {
        t.columns().iter().map(|c| c.data_type).collect()
    }

    fn values(t: &TableData, i: usize) -> Vec<CellValue> {
        t.columns()[i].values.clone()
    }

    #[test]
    fn basic_inference() {
        let t = read(
            "id,name,price,when,ok\r\n1,Vélo,450.5,2026-01-05,true\r\n2,Casque,60,2026-01-07T10:30:00,FALSE\r\n",
        );
        assert_eq!(
            types(&t),
            vec![
                DataType::Int64,
                DataType::String,
                DataType::Double,
                DataType::DateTime,
                DataType::Boolean
            ]
        );
        assert_eq!(values(&t, 0), vec![Int(1), Int(2)]);
        assert_eq!(values(&t, 2), vec![Real(450.5), Real(60.0)]);
        assert_eq!(values(&t, 3), vec![DateTime(46_027.0), DateTime(46_029.437_5)]);
        assert_eq!(values(&t, 4), vec![Bool(true), Bool(false)]);
    }

    #[test]
    fn quotes_escapes_and_embedded_newlines() {
        let t = read("a,b\n\"x, y\",\"say \"\"hi\"\"\"\n\"multi\r\nline\",\"\"\n,plain\n");
        assert_eq!(
            values(&t, 0),
            vec![Text("x, y".into()), Text("multi\r\nline".into()), Null]
        );
        assert_eq!(
            values(&t, 1),
            vec![
                Text("say \"hi\"".into()),
                Text(String::new()),
                Text("plain".into())
            ]
        );
    }

    #[test]
    fn fr_fr_semicolon_and_decimal_comma() {
        let csv = "Produit;Prix;Date\nVélo;1,5;05/01/2026\nCasque;-2,25;31/12/2026\n";
        let t = read_csv(csv.as_bytes(), &CsvOptions::with_culture(Culture::fr_fr())).unwrap();
        assert_eq!(
            types(&t),
            vec![DataType::String, DataType::Double, DataType::DateTime]
        );
        assert_eq!(values(&t, 1), vec![Real(1.5), Real(-2.25)]);
        assert_eq!(values(&t, 2)[0], DateTime(46_027.0));
        // The same file read as invariant keeps the numbers as text.
        let t = read(csv);
        assert_eq!(
            types(&t),
            vec![DataType::String, DataType::String, DataType::String]
        );
    }

    #[test]
    fn currency_symbols_make_fixed_decimal_columns() {
        // M10.3: "$2.675" is 2.675 in en-US (as Number.FromText reads it in Desktop).
        let csv = "Item,Price,Qty\nA,$2.675,1\nB,-$1.5,2\nC,3,3\n";
        let t = read_csv(
            csv.as_bytes(),
            &CsvOptions::with_culture(Culture::from_name("en-US")),
        )
        .unwrap();
        assert_eq!(
            types(&t),
            vec![DataType::String, DataType::Decimal, DataType::Int64]
        );
        assert_eq!(
            values(&t, 1),
            vec![
                CellValue::Decimal(26_750),
                CellValue::Decimal(-15_000),
                CellValue::Decimal(30_000)
            ]
        );
        let csv = "Article;Prix\nA;2,675 €\nB;1,5\n";
        let t = read_csv(
            csv.as_bytes(),
            &CsvOptions::with_culture(Culture::from_name("fr-FR")),
        )
        .unwrap();
        assert_eq!(types(&t), vec![DataType::String, DataType::Decimal]);
        // Another culture's symbol is text.
        let t = read_csv(
            b"P\n\xc2\xa32\n",
            &CsvOptions::with_culture(Culture::from_name("en-US")),
        )
        .unwrap();
        assert_eq!(types(&t), vec![DataType::String]);
    }

    #[test]
    fn delimiter_detection() {
        assert_eq!(read("a\tb\n1\t2\n").columns().len(), 2);
        assert_eq!(read("a;b\n\"1;x\";2\n").columns().len(), 2);
        assert_eq!(read("a,b;c\n1,2;3\n4,5;6\n").columns().len(), 2);
        let t = read("a;b,c\n1;2,5\n");
        assert_eq!(t.columns()[1].name, "b,c");
        assert_eq!(read("single\n1\n2\n").columns().len(), 1);
    }

    #[test]
    fn bom_crlf_lone_cr_and_trailing_lines() {
        let t = read_csv(b"\xEF\xBB\xBFa,b\r1,2\r\n3,4", &CsvOptions::new()).unwrap();
        assert_eq!(t.columns()[0].name, "a");
        assert_eq!(values(&t, 1), vec![Int(2), Int(4)]);
        let t = read("a,b\n1,2\n\n\n3,4\n\n");
        assert_eq!(t.row_count(), 2);
        // UTF-16LE with BOM (Excel "Unicode text").
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend("x\ty\n1\t2\n".encode_utf16().flat_map(u16::to_le_bytes));
        let t = read_csv(&bytes, &CsvOptions::new()).unwrap();
        assert_eq!(values(&t, 1), vec![Int(2)]);
    }

    #[test]
    fn single_column_blank_lines_are_nulls() {
        let t = read("a\n1\n\n3\n");
        assert_eq!(values(&t, 0), vec![Int(1), Null, Int(3)]);
    }

    #[test]
    fn no_header_and_explicit_types() {
        let mut o = CsvOptions::new();
        o.has_header = false;
        o.column_types = vec![("column2".into(), DataType::Decimal)];
        let t = read_csv(b"1,2.5\n3,4", &o).unwrap();
        assert_eq!(t.columns()[0].name, "Column1");
        assert_eq!(
            values(&t, 1),
            vec![CellValue::Decimal(25_000), CellValue::Decimal(40_000)]
        );

        let mut o = CsvOptions::new();
        o.column_types = vec![("a".into(), DataType::Int64)];
        let err = read_csv(b"a\n1\nx\n", &o).unwrap_err();
        assert!(
            matches!(&err, Error::Value { row: 2, column, .. } if column == "a"),
            "{err}"
        );

        let mut o = CsvOptions::new();
        o.infer_types = false;
        assert_eq!(types(&read_csv(b"a\n1\n", &o).unwrap()), vec![DataType::String]);
    }

    #[test]
    fn inference_order_and_mixtures() {
        let t = read("a,b,c,d,e,f\n1,1,2026-01-05,true,007,x\n2,1.5,,false,8,\n,,1/5/2026 12:00,,9,1\n");
        assert_eq!(
            types(&t),
            vec![
                DataType::Int64,
                DataType::Double,
                DataType::DateTime,
                DataType::Boolean,
                DataType::Int64,
                DataType::String
            ]
        );
        assert_eq!(values(&t, 2)[2], DateTime(46_027.5));
        let t = read("a,b\n,\n,\n");
        assert_eq!(types(&t), vec![DataType::String, DataType::String]);
        assert_eq!(values(&t, 0), vec![Null, Null]);
        let t = read("big\n99999999999999999999\n");
        assert_eq!(types(&t), vec![DataType::Double]);
    }

    #[test]
    fn malformed_input_is_an_error() {
        assert!(matches!(
            read_csv(b"a,b\n\"open,1\n", &CsvOptions::new()),
            Err(Error::Syntax { line: 2, .. })
        ));
        assert!(matches!(
            read_csv(b"a,b\n\"x\"y,1\n", &CsvOptions::new()),
            Err(Error::Syntax { .. })
        ));
        assert!(matches!(
            read_csv(b"a,b\n1,2,3\n", &CsvOptions::new()),
            Err(Error::RowLength { row: 1, .. })
        ));
        assert!(matches!(
            read_csv(b"a,b\n\xFF,1\n", &CsvOptions::new()),
            Err(Error::Syntax { .. })
        ));
        assert!(matches!(
            read_csv(b"a,a\n1,2\n", &CsvOptions::new()),
            Err(Error::InvalidTable(_))
        ));
        let mut o = CsvOptions::new();
        o.delimiter = Some('"');
        assert!(read_csv(b"a", &o).is_err());
        assert_eq!(read_csv(b"", &CsvOptions::new()).unwrap().columns().len(), 0);
        assert_eq!(
            read_csv(b"\xFF\xFE", &CsvOptions::new()).unwrap().columns().len(),
            0
        );
        assert!(read_csv(b"\xFF\xFE\x00", &CsvOptions::new()).is_ok());
    }

    #[test]
    fn limits_are_enforced() {
        let mut o = CsvOptions::new();
        o.limits.max_bytes = 4;
        assert!(matches!(
            read_csv(b"a,b\n1,2\n", &o),
            Err(Error::LimitExceeded(_))
        ));
        let mut o = CsvOptions::new();
        o.limits.max_rows = 1;
        assert!(matches!(read_csv(b"a\n1\n2\n", &o), Err(Error::LimitExceeded(_))));
        let mut o = CsvOptions::new();
        o.limits.max_columns = 2;
        assert!(matches!(read_csv(b"a,b,c\n", &o), Err(Error::LimitExceeded(_))));
        let mut o = CsvOptions::new();
        o.limits.max_cell_bytes = 3;
        assert!(matches!(read_csv(b"a\nabcd\n", &o), Err(Error::Syntax { .. })));
    }

    #[test]
    fn writer_quotes_and_formats() {
        let t = TableData::new(vec![
            ColumnData {
                name: "t,x".into(),
                data_type: DataType::String,
                values: vec![Text("a\"b".into()), Text(String::new()), Null],
            },
            ColumnData {
                name: "n".into(),
                data_type: DataType::Double,
                values: vec![Real(1.5), Real(2.0), Real(1e300)],
            },
            ColumnData {
                name: "d".into(),
                data_type: DataType::DateTime,
                values: vec![DateTime(46_027.5), DateTime(46_027.0), Null],
            },
        ])
        .unwrap();
        let s = write_csv(&t, &CsvWriteOptions::default());
        assert_eq!(
            s,
            "\"t,x\",n,d\r\n\"a\"\"b\",1.5,2026-01-05T12:00:00\r\n\"\",2.0,2026-01-05\r\n,1e300,\r\n"
        );
        let fr = CsvWriteOptions {
            delimiter: ';',
            culture: Culture::fr_fr(),
            ..CsvWriteOptions::default()
        };
        assert!(write_csv(&t, &fr).contains("1,5;"));
    }

    /// Tiny deterministic PRNG (xorshift64*) for property-style tests.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn random_value(rng: &mut Rng, t: DataType) -> CellValue {
        if rng.below(6) == 0 {
            return Null;
        }
        match t {
            DataType::Int64 => Int(rng.next() as i64 >> rng.below(63)),
            DataType::Double => {
                let r = f64::from_bits(rng.next());
                if r.is_finite() {
                    Real(r)
                } else {
                    Real(rng.below(1000) as f64 / 7.0)
                }
            }
            DataType::Decimal => CellValue::Decimal(rng.next() as i64 >> rng.below(63)),
            DataType::DateTime => {
                let day = rng.below(2_958_465) as i64 - 600_000;
                let ms = if rng.below(2) == 0 {
                    0
                } else {
                    rng.below(value::MS_PER_DAY as u64) as i64
                };
                DateTime(value::ole_value(day, ms))
            }
            DataType::Boolean => Bool(rng.below(2) == 1),
            _ => {
                const ALPHABET: &[char] = &[
                    'a', 'é', ',', ';', '"', '\n', '\r', ' ', '\t', '1', '.', '€', '\u{feff}',
                ];
                let len = rng.below(8) as usize;
                Text(
                    (0..len)
                        .map(|_| ALPHABET[rng.below(ALPHABET.len() as u64) as usize])
                        .collect(),
                )
            }
        }
    }

    #[test]
    fn round_trip_property() {
        let all = [
            DataType::Int64,
            DataType::Double,
            DataType::Decimal,
            DataType::DateTime,
            DataType::Boolean,
            DataType::String,
        ];
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for case in 0..300 {
            let ncols = 1 + rng.below(5) as usize;
            let nrows = rng.below(12) as usize;
            let columns: Vec<ColumnData> = (0..ncols)
                .map(|i| {
                    let t = all[rng.below(all.len() as u64) as usize];
                    ColumnData {
                        name: format!("c{i};\"x\""),
                        data_type: t,
                        values: (0..nrows).map(|_| random_value(&mut rng, t)).collect(),
                    }
                })
                .collect();
            let table = TableData::new(columns).unwrap();
            let (delimiter, culture) = match case % 3 {
                0 => (',', Culture::INVARIANT),
                1 => (';', Culture::fr_fr()),
                _ => ('\t', Culture::fr_fr()),
            };
            let line_ending = if case % 2 == 0 { "\r\n" } else { "\n" };
            let text = write_csv(
                &table,
                &CsvWriteOptions {
                    delimiter,
                    culture,
                    header: true,
                    line_ending,
                },
            );
            let mut o = CsvOptions::with_culture(culture);
            o.delimiter = Some(delimiter);
            o.column_types = table
                .columns()
                .iter()
                .map(|c| (c.name.clone(), c.data_type))
                .collect();
            let back = read_csv(text.as_bytes(), &o).unwrap_or_else(|e| panic!("case {case}: {e}\n{text}"));
            assert_eq!(back, table, "case {case}\n{text}");
        }
    }

    /// The fast reader against the sequential one ([`read_text`]) on generated text: quotes,
    /// embedded line breaks, empty and blank fields, malformed records, cultures, inferred and
    /// explicit types, with and without a header. Chunks are tiny in tests (`MIN_CHUNK`).
    #[test]
    fn fast_reader_matches_the_sequential_reader() {
        const CELLS: &[&str] = &[
            "1",
            "-12",
            "007",
            "+3",
            "123456789012345678",
            "99999999999999999999",
            "1.5",
            "-0.25",
            "1,5",
            "$2.675",
            "-$1.5",
            "2,675 €",
            "1e3",
            "12.34567",
            "",
            "",
            " 7 ",
            "abc",
            "2026-01-05",
            "2026-02-30",
            "05/01/2026",
            "1/5/2026 12:00",
            "2026-01-05T10:30:00",
            "true",
            "FALSE",
            "é",
            "NaN",
            "inf",
            "5.",
            ".5",
            "-",
        ];
        let mut rng = Rng(0x5EED_CAFE);
        let types = [
            DataType::Int64,
            DataType::Double,
            DataType::Decimal,
            DataType::DateTime,
            DataType::Boolean,
            DataType::String,
        ];
        let mut ok = 0;
        for case in 0..400 {
            let delim = [',', ';', '\t', '§'][rng.below(4) as usize];
            let cols = 1 + rng.below(4) as usize;
            let rows = rng.below(if case % 10 == 0 { 300 } else { 30 }) as usize;
            let eol = ["\n", "\r\n"][rng.below(2) as usize];
            let mut s = String::new();
            let names: Vec<String> = (0..cols).map(|i| format!("c{i}")).collect();
            s.push_str(&names.join(&delim.to_string()));
            s.push_str(eol);
            for _ in 0..rows {
                let n = match rng.below(40) {
                    0 => 0,
                    1 => cols + 1,
                    _ => cols,
                };
                let fields: Vec<String> = (0..n)
                    .map(|_| {
                        let base = match rng.below(12) {
                            0 => format!("a{delim}b"),
                            1 => "two\nlines".to_string(),
                            2 => "say \"hi\"".to_string(),
                            _ => CELLS[rng.below(CELLS.len() as u64) as usize].to_string(),
                        };
                        match rng.below(8) {
                            0 => format!("\"{}\"", base.replace('"', "\"\"")),
                            1 if case % 7 == 0 => format!("\"{base}\"x"),
                            _ if base.contains(['\n', '"', delim]) => {
                                format!("\"{}\"", base.replace('"', "\"\""))
                            }
                            _ => base,
                        }
                    })
                    .collect();
                s.push_str(&fields.join(&delim.to_string()));
                s.push_str(if rng.below(25) == 0 { "\r" } else { eol });
            }
            if case % 5 == 0 {
                s.push_str("\"open");
            }
            let mut o = CsvOptions::with_culture(
                [Culture::INVARIANT, Culture::fr_fr(), Culture::from_name("en-US")][rng.below(3) as usize],
            );
            o.delimiter = Some(delim);
            o.has_header = rng.below(5) != 0;
            o.infer_types = rng.below(6) != 0;
            for n in &names {
                if rng.below(2) == 0 {
                    o.column_types
                        .push((n.to_uppercase(), types[rng.below(types.len() as u64) as usize]));
                }
            }
            if case % 9 == 0 {
                o.limits.max_rows = rows / 2;
            }
            let fast = read_csv(s.as_bytes(), &o);
            let slow = read_text(&s, delim, &o);
            match (&fast, &slow) {
                (Ok(f), Ok(t)) => {
                    ok += 1;
                    assert!(
                        case % 9 == 0 || matches!(fast::read(&s, delim, &o), Ok(Some(_))),
                        "case {case}: not read by the fast reader"
                    );
                    assert_eq!(f.columns().len(), t.columns().len(), "case {case}\n{s}");
                    for (a, b) in f.columns().iter().zip(t.columns()) {
                        assert_eq!((&a.name, a.data_type), (&b.name, b.data_type), "case {case}\n{s}");
                        for (r, (x, y)) in a.values.iter().zip(&b.values).enumerate() {
                            let nan = matches!((x, y), (Real(p), Real(q)) if p.is_nan() && q.is_nan());
                            assert!(
                                nan || x == y,
                                "case {case} column {} row {}: {x:?} != {y:?}\n{s}",
                                a.name,
                                r + 1
                            );
                        }
                    }
                }
                _ => assert_eq!(fast, slow, "case {case}\n{s}"),
            }
        }
        assert!(ok > 60, "{ok} cases loaded");
    }

    #[test]
    fn memory_budget_refuses_cleanly() {
        let mut s = String::from("a,b\n");
        for i in 0..5000 {
            s.push_str(&format!("{i},text {i}\n"));
        }
        let mut o = CsvOptions::new();
        o.limits.max_load_bytes = 50_000;
        let e = read_csv(s.as_bytes(), &o).unwrap_err();
        assert!(
            matches!(&e, Error::LimitExceeded(m) if m.contains("load budget")),
            "{e}"
        );
        o.limits.max_load_bytes = 50_000_000;
        assert_eq!(read_csv(s.as_bytes(), &o).unwrap().row_count(), 5000);
    }

    #[test]
    fn hostile_bytes_never_panic() {
        let mut rng = Rng(42);
        let pieces: &[&[u8]] = &[
            b",",
            b";",
            b"\t",
            b"\"",
            b"\r",
            b"\n",
            b"1",
            b"a",
            b"\xC3\xA9",
            b"\xFF",
            b"\xEF\xBB\xBF",
            b"2026-01-05",
            b"1,5",
        ];
        for _ in 0..2000 {
            let n = rng.below(30) as usize;
            let mut bytes = Vec::new();
            for _ in 0..n {
                bytes.extend_from_slice(pieces[rng.below(pieces.len() as u64) as usize]);
            }
            let _ = read_csv(&bytes, &CsvOptions::new());
            let _ = read_csv(&bytes, &CsvOptions::with_culture(Culture::fr_fr()));
        }
    }
}
