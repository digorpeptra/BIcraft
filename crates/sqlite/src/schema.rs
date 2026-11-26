//! Table descriptions: a small parser for the `CREATE TABLE` statements kept in `sqlite_schema`.
//!
//! It extracts what reading rows needs: column names, declared types, the rowid alias
//! (`INTEGER PRIMARY KEY`), `DEFAULT` literals, generated columns and the `WITHOUT ROWID`
//! option. Expressions (`CHECK`, generated-column bodies, non-literal defaults) are skipped.

use crate::btree::RowLayout;
use crate::record::Value;

/// Statements longer than this are not parsed (the schema text comes from the file).
const MAX_SQL_LEN: usize = 1 << 20;

/// How a table stores its rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableKind {
    /// An ordinary table keyed by rowid; its rows can be read.
    Rowid,
    /// A `WITHOUT ROWID` table, stored as an index b-tree. Listed but not readable.
    WithoutRowid,
    /// A virtual table (`CREATE VIRTUAL TABLE`): no storage of its own. Listed but not readable.
    Virtual,
}

/// A generated column's storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Generated {
    /// Computed on read and not stored; this reader returns `Null` for it.
    Virtual,
    /// Computed on write and stored in the record.
    Stored,
}

/// One declared column.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnInfo {
    pub name: String,
    /// The declared type exactly as written (may be empty), e.g. `VARCHAR(20)`.
    pub decl_type: String,
    /// Type affinity derived from `decl_type`. `Real` columns read stored integers as reals,
    /// as SQLite does (it stores integral reals as integers to save space).
    pub affinity: Affinity,
    /// Part of the PRIMARY KEY (column or table constraint).
    pub primary_key: bool,
    /// An `INTEGER PRIMARY KEY` that aliases the rowid; the stored record holds NULL here and
    /// rows report the rowid instead.
    pub rowid_alias: bool,
    pub generated: Option<Generated>,
    /// The `DEFAULT` value, when it is a literal (after the column's type affinity is applied).
    /// Records written before an `ALTER TABLE ... ADD COLUMN` lack the column and read as this.
    pub default: Option<Value>,
}

/// One table listed in `sqlite_schema`.
#[derive(Debug, Clone, PartialEq)]
pub struct TableInfo {
    pub name: String,
    pub root_page: u32,
    pub sql: String,
    pub kind: TableKind,
    /// Declared columns, in order. Empty for virtual tables and unparseable statements.
    pub columns: Vec<ColumnInfo>,
    /// Why the `CREATE TABLE` statement could not be parsed, if it could not.
    pub parse_error: Option<String>,
}

impl TableInfo {
    pub(crate) fn from_schema(name: String, root_page: u32, sql: String) -> Self {
        let mut info = Self {
            name,
            root_page,
            sql: String::new(),
            kind: TableKind::Rowid,
            columns: Vec::new(),
            parse_error: None,
        };
        match parse_create_table(&sql) {
            Ok(parsed) => {
                info.kind = parsed.kind;
                info.columns = parsed.columns;
            }
            Err(e) => info.parse_error = Some(e),
        }
        info.sql = sql;
        info
    }

    /// The column names, in declaration order.
    pub fn column_names(&self) -> Vec<String> {
        self.columns.iter().map(|c| c.name.clone()).collect()
    }

    /// The index of the column called `name` (ASCII case-insensitive, like SQLite).
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(name))
    }

    pub(crate) fn layout(&self) -> RowLayout {
        let mut storage = Vec::with_capacity(self.columns.len());
        let mut next = 0usize;
        for c in &self.columns {
            if c.generated == Some(Generated::Virtual) {
                storage.push(None);
            } else {
                storage.push(Some(next));
                next += 1;
            }
        }
        RowLayout {
            storage,
            rowid_alias: self.columns.iter().position(|c| c.rowid_alias),
            real: self
                .columns
                .iter()
                .map(|c| c.affinity == Affinity::Real)
                .collect(),
            defaults: self
                .columns
                .iter()
                .map(|c| c.default.clone().unwrap_or(Value::Null))
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Tokenizer

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// An identifier or keyword; `quoted` when written as "x", [x] or `x`.
    Ident {
        text: String,
        quoted: bool,
    },
    /// A 'string literal'.
    Str(String),
    Num(String),
    Blob(Vec<u8>),
    Punct(char),
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    start: usize,
    end: usize,
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || !c.is_ascii()
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$' || !c.is_ascii()
}

fn tokenize(sql: &str) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let mut it = sql.char_indices().peekable();
    while let Some((start, c)) = it.next() {
        let tok = match c {
            c if c.is_whitespace() => continue,
            '-' if it.peek().map(|p| p.1) == Some('-') => {
                for (_, c) in it.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
                continue;
            }
            '/' if it.peek().map(|p| p.1) == Some('*') => {
                it.next();
                let mut prev = '\0';
                for (_, c) in it.by_ref() {
                    if prev == '*' && c == '/' {
                        break;
                    }
                    prev = c;
                }
                continue;
            }
            '\'' => Tok::Str(quoted(&mut it, '\'')?),
            '"' => Tok::Ident {
                text: quoted(&mut it, '"')?,
                quoted: true,
            },
            '`' => Tok::Ident {
                text: quoted(&mut it, '`')?,
                quoted: true,
            },
            '[' => {
                let mut s = String::new();
                loop {
                    match it.next() {
                        Some((_, ']')) => break,
                        Some((_, c)) => s.push(c),
                        None => return Err("unterminated [identifier]".into()),
                    }
                }
                Tok::Ident {
                    text: s,
                    quoted: true,
                }
            }
            'x' | 'X' if it.peek().map(|p| p.1) == Some('\'') => {
                it.next();
                let hex = quoted(&mut it, '\'')?;
                Tok::Blob(parse_hex(&hex).ok_or("malformed blob literal")?)
            }
            c if c.is_ascii_digit() || (c == '.' && it.peek().is_some_and(|p| p.1.is_ascii_digit())) => {
                let mut s = String::from(c);
                while let Some(&(_, d)) = it.peek() {
                    let exp_sign = (d == '+' || d == '-') && s.ends_with(['e', 'E']) && !s.starts_with("0x");
                    if d.is_ascii_alphanumeric() || d == '.' || d == '_' || exp_sign {
                        s.push(d);
                        it.next();
                    } else {
                        break;
                    }
                }
                Tok::Num(s)
            }
            c if is_ident_start(c) => {
                let mut s = String::from(c);
                while let Some(&(_, d)) = it.peek() {
                    if is_ident_char(d) {
                        s.push(d);
                        it.next();
                    } else {
                        break;
                    }
                }
                Tok::Ident {
                    text: s,
                    quoted: false,
                }
            }
            c => Tok::Punct(c),
        };
        let end = it.peek().map_or(sql.len(), |p| p.0);
        out.push(Token { tok, start, end });
    }
    Ok(out)
}

/// Read a quoted token body up to the closing `q`; a doubled `q` stands for one.
fn quoted(it: &mut std::iter::Peekable<std::str::CharIndices<'_>>, q: char) -> Result<String, String> {
    let mut s = String::new();
    loop {
        match it.next() {
            Some((_, c)) if c == q => {
                if it.peek().map(|p| p.1) == Some(q) {
                    it.next();
                    s.push(q);
                } else {
                    return Ok(s);
                }
            }
            Some((_, c)) => s.push(c),
            None => return Err(format!("unterminated {q}quoted{q} token")),
        }
    }
}

fn parse_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let digits: Option<Vec<u8>> = s
        .chars()
        .map(|c| c.to_digit(16).and_then(|d| u8::try_from(d).ok()))
        .collect();
    let digits = digits?;
    Some(
        digits
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[hi, lo]| (hi << 4) | lo)
            .collect(),
    )
}

// ---------------------------------------------------------------------------------------------
// Parser

struct Parsed {
    kind: TableKind,
    columns: Vec<ColumnInfo>,
    extras: TableExtras,
}

/// One column of a `PRIMARY KEY` / `UNIQUE` constraint or of a `CREATE INDEX`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyColumn {
    pub name: String,
    /// An explicit `COLLATE` on this key column.
    pub collation: Option<String>,
    pub desc: bool,
}

/// A `PRIMARY KEY` or `UNIQUE` constraint, in the order it appears in `CREATE TABLE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyConstraint {
    pub primary: bool,
    /// Written as a column constraint (its collation is then the column's final collation).
    pub column_constraint: bool,
    pub columns: Vec<KeyColumn>,
}

/// What the writer needs from a `CREATE TABLE` beyond what reading rows needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TableExtras {
    /// The table name as written in the statement (unquoted).
    pub table_name: String,
    /// The `COLLATE` of each declared column, if any.
    pub collations: Vec<Option<String>>,
    pub keys: Vec<KeyConstraint>,
    pub autoincrement: bool,
}

/// Parse the parts of a `CREATE TABLE` statement that the writer needs (constraint indexes,
/// collations, `AUTOINCREMENT`).
pub(crate) fn table_extras(sql: &str) -> Result<TableExtras, String> {
    parse_create_table(sql).map(|p| p.extras)
}

/// A parsed `CREATE INDEX` statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexSpec {
    pub unique: bool,
    pub name: String,
    pub table: String,
    pub columns: Vec<KeyColumn>,
    /// Has a `WHERE` clause (a partial index).
    pub partial: bool,
}

/// Why a `CREATE INDEX` statement cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IndexParseError {
    Malformed(String),
    /// Valid SQL that the writer does not support (expression indexes).
    Unsupported(String),
}

/// Parse `CREATE [UNIQUE] INDEX [IF NOT EXISTS] [schema.]name ON table (col [COLLATE c]
/// [ASC|DESC], ...) [WHERE ...]`. Index columns that are expressions are reported as
/// unsupported.
pub(crate) fn parse_create_index(sql: &str) -> Result<IndexSpec, IndexParseError> {
    use IndexParseError::{Malformed, Unsupported};
    if sql.len() > MAX_SQL_LEN {
        return Err(Malformed(format!("statement longer than {MAX_SQL_LEN} bytes")));
    }
    let toks = tokenize(sql).map_err(Malformed)?;
    let mut c = Cursor { toks: &toks, pos: 0 };
    c.expect_kw("CREATE").map_err(Malformed)?;
    let unique = c.eat_kw("UNIQUE");
    c.expect_kw("INDEX").map_err(Malformed)?;
    if c.eat_kw("IF") {
        c.expect_kw("NOT").map_err(Malformed)?;
        c.expect_kw("EXISTS").map_err(Malformed)?;
    }
    let mut name = c.name().map_err(Malformed)?;
    if c.eat_punct('.') {
        name = c.name().map_err(Malformed)?;
    }
    c.expect_kw("ON").map_err(Malformed)?;
    let table = c.name().map_err(Malformed)?;
    if !c.eat_punct('(') {
        return Err(Malformed("expected ( after the table name".into()));
    }
    let mut columns = Vec::new();
    loop {
        let col_name = match c.bump().map(|t| &t.tok) {
            Some(Tok::Ident { text, .. }) => text.clone(),
            Some(Tok::Punct(')')) | None => {
                return Err(Malformed("empty or unterminated column list".into()));
            }
            Some(_) => return Err(Unsupported(format!("index `{name}` has an expression column"))),
        };
        let mut col = KeyColumn {
            name: col_name,
            collation: None,
            desc: false,
        };
        loop {
            if c.eat_kw("COLLATE") {
                col.collation = Some(c.name().map_err(Malformed)?);
            } else if c.eat_kw("DESC") {
                col.desc = true;
            } else if c.eat_kw("ASC") {
                col.desc = false;
            } else {
                break;
            }
        }
        columns.push(col);
        if c.eat_punct(',') {
            continue;
        }
        if c.eat_punct(')') {
            break;
        }
        return Err(match c.peek() {
            None => Malformed("unterminated column list".into()),
            Some(_) => Unsupported(format!("index `{name}` has an expression column")),
        });
    }
    let partial = c.eat_kw("WHERE");
    if !partial {
        let _ = c.eat_punct(';');
        if c.peek().is_some() {
            return Err(Malformed("unexpected text after the column list".into()));
        }
    }
    Ok(IndexSpec {
        unique,
        name,
        table,
        columns,
        partial,
    })
}

struct Cursor<'t> {
    toks: &'t [Token],
    pos: usize,
}

impl<'t> Cursor<'t> {
    fn peek(&self) -> Option<&'t Tok> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }

    fn bump(&mut self) -> Option<&'t Token> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    /// Is the next token the (unquoted) keyword `kw`?
    fn at_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident { text, quoted: false }) if text.eq_ignore_ascii_case(kw))
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        let yes = self.at_kw(kw);
        if yes {
            self.pos += 1;
        }
        yes
    }

    fn expect_kw(&mut self, kw: &str) -> Result<(), String> {
        if self.eat_kw(kw) {
            Ok(())
        } else {
            Err(format!("expected {kw}"))
        }
    }

    fn at_punct(&self, p: char) -> bool {
        self.peek() == Some(&Tok::Punct(p))
    }

    fn eat_punct(&mut self, p: char) -> bool {
        let yes = self.at_punct(p);
        if yes {
            self.pos += 1;
        }
        yes
    }

    /// Consume a name (identifier or string literal).
    fn name(&mut self) -> Result<String, String> {
        match self.bump().map(|t| &t.tok) {
            Some(Tok::Ident { text, .. }) | Some(Tok::Str(text)) => Ok(text.clone()),
            other => Err(format!("expected a name, found {other:?}")),
        }
    }

    /// Skip a parenthesized group; the cursor is on its `(`.
    fn skip_group(&mut self) -> Result<(), String> {
        let mut depth = 0usize;
        while let Some(t) = self.bump() {
            match t.tok {
                Tok::Punct('(') => depth += 1,
                Tok::Punct(')') => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
        Err("unbalanced parentheses".into())
    }

    /// Is the cursor at the end of the current definition (`,` or `)` at depth 0)?
    fn at_def_end(&self) -> bool {
        matches!(self.peek(), None | Some(Tok::Punct(',')) | Some(Tok::Punct(')')))
    }
}

/// Keywords that end a column's type name and start its constraints.
const COLUMN_CONSTRAINT_KWS: &[&str] = &[
    "CONSTRAINT",
    "PRIMARY",
    "NOT",
    "NULL",
    "UNIQUE",
    "CHECK",
    "DEFAULT",
    "COLLATE",
    "REFERENCES",
    "GENERATED",
    "AS",
];

/// Keywords that start a table constraint.
const TABLE_CONSTRAINT_KWS: &[&str] = &["CONSTRAINT", "PRIMARY", "UNIQUE", "CHECK", "FOREIGN"];

fn is_kw_in(tok: Option<&Tok>, list: &[&str]) -> bool {
    matches!(tok, Some(Tok::Ident { text, quoted: false }) if list.iter().any(|k| text.eq_ignore_ascii_case(k)))
}

fn parse_create_table(sql: &str) -> Result<Parsed, String> {
    if sql.len() > MAX_SQL_LEN {
        return Err(format!("statement longer than {MAX_SQL_LEN} bytes"));
    }
    let toks = tokenize(sql)?;
    let mut c = Cursor { toks: &toks, pos: 0 };
    c.expect_kw("CREATE")?;
    let _ = c.eat_kw("TEMP") || c.eat_kw("TEMPORARY");
    if c.eat_kw("VIRTUAL") {
        return Ok(Parsed {
            kind: TableKind::Virtual,
            columns: Vec::new(),
            extras: TableExtras::default(),
        });
    }
    c.expect_kw("TABLE")?;
    if c.eat_kw("IF") {
        c.expect_kw("NOT")?;
        c.expect_kw("EXISTS")?;
    }
    let mut extras = TableExtras {
        table_name: c.name()?,
        ..TableExtras::default()
    };
    if c.eat_punct('.') {
        extras.table_name = c.name()?;
    }
    if c.at_kw("AS") {
        return Err("CREATE TABLE ... AS SELECT has no column list".into());
    }
    if !c.eat_punct('(') {
        return Err("expected ( after the table name".into());
    }

    let mut columns: Vec<ColumnInfo> = Vec::new();
    let mut column_pk_desc: Vec<bool> = Vec::new();
    let mut table_pk: Option<Vec<String>> = None;
    loop {
        if is_kw_in(c.peek(), TABLE_CONSTRAINT_KWS) {
            if let Some(cols) = table_constraint(&mut c, &mut extras)? {
                table_pk = Some(cols);
            }
        } else {
            let (col, desc) = column_def(&mut c, sql, &mut extras)?;
            columns.push(col);
            column_pk_desc.push(desc);
        }
        if c.eat_punct(',') {
            continue;
        }
        if c.eat_punct(')') {
            break;
        }
        return Err("expected , or ) after a definition".into());
    }

    let mut without_rowid = false;
    loop {
        if c.eat_kw("WITHOUT") {
            c.expect_kw("ROWID")?;
            without_rowid = true;
        } else if c.eat_kw("STRICT") {
        } else if c.peek().is_none() || c.eat_punct(';') {
            break;
        } else {
            return Err("unexpected text after the column list".into());
        }
        let _ = c.eat_punct(',');
    }

    if columns.is_empty() {
        return Err("table has no columns".into());
    }
    if let Some(pk) = &table_pk {
        for name in pk {
            if let Some(col) = columns.iter_mut().find(|col| col.name.eq_ignore_ascii_case(name)) {
                col.primary_key = true;
            }
        }
    }
    if !without_rowid {
        // An INTEGER PRIMARY KEY aliases the rowid, except the `INTEGER PRIMARY KEY DESC`
        // column-constraint form, which keeps a separate rowid.
        let single_table_pk = table_pk
            .as_ref()
            .filter(|pk| pk.len() == 1)
            .and_then(|pk| pk.first());
        let alias = columns.iter().enumerate().position(|(i, col)| {
            if !col.decl_type.eq_ignore_ascii_case("INTEGER") {
                return false;
            }
            match single_table_pk {
                Some(name) => col.name.eq_ignore_ascii_case(name),
                None => {
                    col.primary_key && table_pk.is_none() && !column_pk_desc.get(i).copied().unwrap_or(false)
                }
            }
        });
        // Only a single-column primary key can be the rowid.
        let pk_count = columns.iter().filter(|col| col.primary_key).count();
        if let (Some(i), 1) = (alias, pk_count)
            && let Some(col) = columns.get_mut(i)
        {
            col.rowid_alias = true;
        }
    }
    Ok(Parsed {
        kind: if without_rowid {
            TableKind::WithoutRowid
        } else {
            TableKind::Rowid
        },
        columns,
        extras,
    })
}

/// Parse a table constraint; returns the column names when it is the PRIMARY KEY.
fn table_constraint(c: &mut Cursor<'_>, extras: &mut TableExtras) -> Result<Option<Vec<String>>, String> {
    if c.eat_kw("CONSTRAINT") {
        c.name()?;
    }
    let mut pk = None;
    let primary = c.eat_kw("PRIMARY");
    if primary {
        c.expect_kw("KEY")?;
    }
    if primary || c.eat_kw("UNIQUE") {
        if !c.eat_punct('(') {
            return Err("expected ( after PRIMARY KEY or UNIQUE".into());
        }
        let mut cols = Vec::new();
        loop {
            let mut col = KeyColumn {
                name: c.name()?,
                collation: None,
                desc: false,
            };
            // COLLATE x / ASC / DESC / AUTOINCREMENT up to the next , or )
            while !c.at_def_end() {
                if c.eat_kw("COLLATE") {
                    col.collation = Some(c.name()?);
                } else if c.eat_kw("DESC") {
                    col.desc = true;
                } else if c.eat_kw("AUTOINCREMENT") {
                    extras.autoincrement = true;
                } else if c.at_punct('(') {
                    c.skip_group()?;
                } else {
                    c.bump();
                }
            }
            cols.push(col);
            if c.eat_punct(',') {
                continue;
            }
            if c.eat_punct(')') {
                break;
            }
            return Err("unterminated PRIMARY KEY or UNIQUE column list".into());
        }
        if primary {
            pk = Some(cols.iter().map(|k| k.name.clone()).collect());
        }
        extras.keys.push(KeyConstraint {
            primary,
            column_constraint: false,
            columns: cols,
        });
    }
    skip_to_def_end(c)?;
    Ok(pk)
}

fn skip_to_def_end(c: &mut Cursor<'_>) -> Result<(), String> {
    while !c.at_def_end() {
        if c.at_punct('(') {
            c.skip_group()?;
        } else {
            c.bump();
        }
    }
    Ok(())
}

/// Parse one column definition; also returns whether its PRIMARY KEY is `DESC`.
fn column_def(c: &mut Cursor<'_>, sql: &str, extras: &mut TableExtras) -> Result<(ColumnInfo, bool), String> {
    let name = c.name()?;
    // Type name: identifiers up to a constraint keyword, then an optional (n) or (n, m).
    let mut span: Option<(usize, usize)> = None;
    while let Some(t) = c.toks.get(c.pos) {
        let is_type_word = matches!(t.tok, Tok::Ident { .. } | Tok::Str(_))
            && !is_kw_in(Some(&t.tok), COLUMN_CONSTRAINT_KWS);
        if !is_type_word {
            break;
        }
        span = Some((span.map_or(t.start, |s| s.0), t.end));
        c.pos += 1;
    }
    if span.is_some() && c.at_punct('(') {
        let open = c.pos;
        c.skip_group()?;
        if let Some(last) = c.toks.get(c.pos.saturating_sub(1))
            && let Some(s) = span.as_mut()
            && c.pos > open
        {
            s.1 = last.end;
        }
    }
    let decl_type = span.and_then(|(s, e)| sql.get(s..e)).unwrap_or("").to_string();
    let mut col = ColumnInfo {
        name,
        affinity: affinity(&decl_type),
        decl_type,
        primary_key: false,
        rowid_alias: false,
        generated: None,
        default: None,
    };
    let mut desc = false;
    let mut collation = None;
    while !c.at_def_end() {
        if c.eat_kw("CONSTRAINT") {
            c.name()?;
        } else if c.eat_kw("PRIMARY") {
            c.expect_kw("KEY")?;
            col.primary_key = true;
            if c.eat_kw("DESC") {
                desc = true;
            }
            extras.keys.push(KeyConstraint {
                primary: true,
                column_constraint: true,
                columns: vec![KeyColumn {
                    name: col.name.clone(),
                    collation: None,
                    desc,
                }],
            });
        } else if c.eat_kw("UNIQUE") {
            extras.keys.push(KeyConstraint {
                primary: false,
                column_constraint: true,
                columns: vec![KeyColumn {
                    name: col.name.clone(),
                    collation: None,
                    desc: false,
                }],
            });
        } else if c.eat_kw("COLLATE") {
            collation = Some(c.name()?);
        } else if c.eat_kw("AUTOINCREMENT") {
            extras.autoincrement = true;
        } else if c.eat_kw("DEFAULT") {
            col.default = default_literal(c)?.map(|v| apply_affinity(v, col.affinity));
        } else if c.eat_kw("AS") {
            if c.at_punct('(') {
                c.skip_group()?;
            }
            col.generated = Some(if c.eat_kw("STORED") {
                Generated::Stored
            } else {
                let _ = c.eat_kw("VIRTUAL");
                Generated::Virtual
            });
        } else if c.at_punct('(') {
            c.skip_group()?;
        } else {
            c.bump();
        }
    }
    extras.collations.push(collation);
    Ok((col, desc))
}

/// Parse the expression after DEFAULT; returns the value when it is a literal.
fn default_literal(c: &mut Cursor<'_>) -> Result<Option<Value>, String> {
    if c.at_punct('(') {
        // `DEFAULT (literal)`: look inside; anything else is an expression we do not evaluate.
        let start = c.pos;
        c.skip_group()?;
        let inner = c.toks.get(start + 1..c.pos.saturating_sub(1)).unwrap_or(&[]);
        let mut sub = Cursor { toks: inner, pos: 0 };
        let v = literal(&mut sub);
        return Ok(if sub.pos == inner.len() { v } else { None });
    }
    let v = literal(c);
    Ok(v)
}

fn literal(c: &mut Cursor<'_>) -> Option<Value> {
    let negative = if c.eat_punct('-') {
        true
    } else {
        let _ = c.eat_punct('+');
        false
    };
    let t = c.peek()?.clone();
    let v = match t {
        Tok::Num(s) => {
            let v = parse_number(&s)?;
            match (v, negative) {
                (Value::Integer(i), true) => i
                    .checked_neg()
                    .map(Value::Integer)
                    .unwrap_or(Value::Real(-(i as f64))),
                (Value::Real(r), true) => Value::Real(-r),
                (v, _) => v,
            }
        }
        Tok::Str(s) if !negative => Value::Text(s),
        Tok::Blob(b) if !negative => Value::Blob(b),
        Tok::Ident { text, quoted: false } if !negative => {
            if text.eq_ignore_ascii_case("NULL") {
                Value::Null
            } else if text.eq_ignore_ascii_case("TRUE") {
                Value::Integer(1)
            } else if text.eq_ignore_ascii_case("FALSE") {
                Value::Integer(0)
            } else {
                return None;
            }
        }
        _ => return None,
    };
    c.pos += 1;
    Some(v)
}

fn parse_number(s: &str) -> Option<Value> {
    let s = s.replace('_', "");
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16)
            .ok()
            .map(|u| Value::Integer(u as i64));
    }
    if let Ok(i) = s.parse::<i64>() {
        return Some(Value::Integer(i));
    }
    s.parse::<f64>().ok().map(Value::Real)
}

/// Column type affinity, derived from the declared type ("Datatypes In SQLite" §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Affinity {
    Integer,
    Text,
    Blob,
    Real,
    Numeric,
}

fn affinity(decl: &str) -> Affinity {
    let t = decl.to_ascii_uppercase();
    if t.contains("INT") {
        Affinity::Integer
    } else if t.contains("CHAR") || t.contains("CLOB") || t.contains("TEXT") {
        Affinity::Text
    } else if t.contains("BLOB") || t.is_empty() {
        Affinity::Blob
    } else if t.contains("REAL") || t.contains("FLOA") || t.contains("DOUB") {
        Affinity::Real
    } else {
        Affinity::Numeric
    }
}

fn apply_affinity(v: Value, aff: Affinity) -> Value {
    match (aff, v) {
        (Affinity::Integer | Affinity::Numeric | Affinity::Real, Value::Text(s)) => {
            match parse_number(s.trim()) {
                Some(Value::Integer(i)) if aff == Affinity::Real => Value::Real(i as f64),
                Some(n) => n,
                None => Value::Text(s),
            }
        }
        (Affinity::Real, Value::Integer(i)) => Value::Real(i as f64),
        (Affinity::Text, Value::Integer(i)) => Value::Text(i.to_string()),
        (Affinity::Text, Value::Real(r)) => Value::Text(format!("{r:?}")),
        (_, v) => v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(sql: &str) -> Vec<ColumnInfo> {
        parse_create_table(sql).unwrap().columns
    }

    #[test]
    fn quoted_names_and_types() {
        let c = cols(
            "CREATE TABLE \"a\"\"b\" ([x y] VARCHAR ( 10 ), `q``r` UNSIGNED BIG INT, 'z', \"w\" NOT NULL)",
        );
        let names: Vec<_> = c.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["x y", "q`r", "z", "w"]);
        assert_eq!(c[0].decl_type, "VARCHAR ( 10 )");
        assert_eq!(c[1].decl_type, "UNSIGNED BIG INT");
        assert_eq!(c[2].decl_type, "");
        assert_eq!(c[3].decl_type, "");
    }

    #[test]
    fn rowid_alias_rules() {
        assert!(cols("CREATE TABLE t(id INTEGER PRIMARY KEY, v)")[0].rowid_alias);
        assert!(cols("create table t(id integer primary key asc)")[0].rowid_alias);
        assert!(!cols("CREATE TABLE t(id INTEGER PRIMARY KEY DESC, v)")[0].rowid_alias);
        assert!(!cols("CREATE TABLE t(id INT PRIMARY KEY, v)")[0].rowid_alias);
        assert!(cols("CREATE TABLE t(id INTEGER, v, PRIMARY KEY(id DESC))")[0].rowid_alias);
        assert!(!cols("CREATE TABLE t(id INTEGER, v, PRIMARY KEY(id, v))")[0].rowid_alias);
        let p = parse_create_table("CREATE TABLE t(id INTEGER PRIMARY KEY, v) WITHOUT ROWID").unwrap();
        assert_eq!(p.kind, TableKind::WithoutRowid);
        assert!(!p.columns[0].rowid_alias);
    }

    #[test]
    fn defaults_and_generated() {
        let c = cols(
            "CREATE TABLE t(a INTEGER DEFAULT -3, b TEXT DEFAULT 'x''y', c REAL DEFAULT (2), \
             d DEFAULT x'00ff', e DEFAULT CURRENT_TIMESTAMP, f INT AS (a+1) STORED, g GENERATED ALWAYS AS (a))",
        );
        assert_eq!(c[0].default, Some(Value::Integer(-3)));
        assert_eq!(c[1].default, Some(Value::Text("x'y".into())));
        assert_eq!(c[2].default, Some(Value::Real(2.0)));
        assert_eq!(c[3].default, Some(Value::Blob(vec![0, 0xff])));
        assert_eq!(c[4].default, None);
        assert_eq!(c[5].generated, Some(Generated::Stored));
        assert_eq!(c[6].generated, Some(Generated::Virtual));
    }

    #[test]
    fn rejects_garbage() {
        for sql in [
            "",
            "CREATE",
            "CREATE TABLE",
            "CREATE TABLE t",
            "CREATE TABLE t(",
            "CREATE TABLE t(a",
            "CREATE TABLE t()",
            "CREATE TABLE t AS SELECT 1",
            "CREATE TABLE t(a) junk",
            "CREATE TABLE t('a",
            "CREATE TABLE t([a",
        ] {
            assert!(parse_create_table(sql).is_err(), "{sql}");
        }
        assert_eq!(
            parse_create_table("CREATE VIRTUAL TABLE f USING fts5(a)")
                .unwrap()
                .kind,
            TableKind::Virtual
        );
    }
}
