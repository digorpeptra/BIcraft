//! Data-source calls in M text, found without evaluating it (`sources.list`, `sources.rebase`,
//! `env.apply` in the engine).
//!
//! The scan works on tokens, so every position is a byte range of the text as written: a rewrite
//! replaces exactly one literal and leaves every other byte alone. It recognizes the connector
//! and file functions of the public Power Query M function reference, splits their arguments at
//! top-level commas, and tells for each argument whether it is a text literal, a name (a step of
//! the query, another query or a parameter), a concatenation of those with `&`, a nested call, or
//! something computed, which is reported and never guessed.
//!
//! It also reads a parameter query (`value meta [IsParameterQuery = true, Type = "Text", …]`):
//! the value's range, its declared type and its list of allowed values.

use std::collections::HashMap;

use super::lexer::{Tok, Token, tokenize};
use crate::Limits;

/// What an argument of a source function holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A file path (`File.Contents`).
    Path,
    /// A folder path (`Folder.Files`, `Folder.Contents`).
    Folder,
    /// A database server name.
    Server,
    /// A database name.
    Database,
    /// A Databricks workspace host name.
    Host,
    /// A Databricks SQL warehouse HTTP path.
    HttpPath,
    /// A URL (web, OData, SharePoint, storage).
    Url,
    /// A storage account name.
    Account,
    /// A Snowflake warehouse.
    Warehouse,
    /// An ODBC or OLE DB connection string.
    ConnectionString,
    /// The binary content a format function reads (`Excel.Workbook`, `Csv.Document`, …).
    Content,
}

impl Role {
    /// The role's name in results (`path`, `server`, …).
    pub fn name(self) -> &'static str {
        match self {
            Role::Path => "path",
            Role::Folder => "folder",
            Role::Server => "server",
            Role::Database => "database",
            Role::Host => "host",
            Role::HttpPath => "http_path",
            Role::Url => "url",
            Role::Account => "account",
            Role::Warehouse => "warehouse",
            Role::ConnectionString => "connection_string",
            Role::Content => "content",
        }
    }

    /// Whether the argument names a place that changes between environments.
    pub fn is_location(self) -> bool {
        self != Role::Content
    }
}

/// The source functions and the roles of their leading arguments.
const FUNCTIONS: &[(&str, &[Role])] = &[
    ("File.Contents", &[Role::Path]),
    ("Folder.Files", &[Role::Folder]),
    ("Folder.Contents", &[Role::Folder]),
    ("Excel.Workbook", &[Role::Content]),
    ("Csv.Document", &[Role::Content]),
    ("Json.Document", &[Role::Content]),
    ("Xml.Tables", &[Role::Content]),
    ("Xml.Document", &[Role::Content]),
    ("Parquet.Document", &[Role::Content]),
    ("Pdf.Tables", &[Role::Content]),
    ("Lines.FromBinary", &[Role::Content]),
    ("Access.Database", &[Role::Content]),
    ("Sql.Database", &[Role::Server, Role::Database]),
    ("Sql.Databases", &[Role::Server]),
    ("PostgreSQL.Database", &[Role::Server, Role::Database]),
    ("MySQL.Database", &[Role::Server, Role::Database]),
    ("Oracle.Database", &[Role::Server]),
    ("Teradata.Database", &[Role::Server]),
    ("Sybase.Database", &[Role::Server, Role::Database]),
    ("SapHana.Database", &[Role::Server]),
    ("AnalysisServices.Database", &[Role::Server, Role::Database]),
    ("AnalysisServices.Databases", &[Role::Server]),
    ("Snowflake.Databases", &[Role::Server, Role::Warehouse]),
    ("Databricks.Catalogs", &[Role::Host, Role::HttpPath]),
    ("Databricks.Contents", &[Role::Host, Role::HttpPath]),
    ("Databricks.Query", &[Role::Host, Role::HttpPath]),
    ("DatabricksMultiCloud.Catalogs", &[Role::Host, Role::HttpPath]),
    ("Web.Contents", &[Role::Url]),
    ("OData.Feed", &[Role::Url]),
    ("SharePoint.Files", &[Role::Url]),
    ("SharePoint.Contents", &[Role::Url]),
    ("SharePoint.Tables", &[Role::Url]),
    ("AzureStorage.Blobs", &[Role::Account]),
    ("AzureStorage.Tables", &[Role::Account]),
    ("AzureStorage.DataLake", &[Role::Url]),
    ("Hdfs.Files", &[Role::Url]),
    ("Hdfs.Contents", &[Role::Url]),
    ("Odbc.DataSource", &[Role::ConnectionString]),
    ("Odbc.Query", &[Role::ConnectionString]),
    ("OleDb.DataSource", &[Role::ConnectionString]),
    ("OleDb.Query", &[Role::ConnectionString]),
];

/// The roles of `function`'s leading arguments, when it is a source function.
pub fn source_roles(function: &str) -> Option<&'static [Role]> {
    FUNCTIONS.iter().find(|(f, _)| *f == function).map(|(_, r)| *r)
}

/// A text literal: its decoded value and its byte range (quotes included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextLit {
    pub value: String,
    pub start: usize,
    pub end: usize,
}

/// One operand of a `&` concatenation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Text(TextLit),
    /// A name, with its byte range.
    Name(String, usize, usize),
}

/// What an argument is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Form {
    /// A text literal.
    Text(TextLit),
    /// A name that is a step of this query bound to a text literal (`Path = "C:\x.csv"`).
    StepText(String, TextLit),
    /// Another name: a step computed in this query, or a query or parameter of the model.
    Name { name: String, step: bool },
    /// Text literals and names joined with `&`.
    Concat(Vec<Part>),
    /// A call of another function (`File.Contents(…)` inside `Excel.Workbook(…)`).
    Call(String),
    /// Anything else (a number, an expression): reported, never rewritten.
    Computed,
}

/// One argument of a source call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arg {
    pub role: Role,
    pub form: Form,
    /// Byte range of the argument's tokens.
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub column: usize,
}

/// One call of a source function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCall {
    pub function: String,
    /// Byte range of the call, from the function name to its closing parenthesis.
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub column: usize,
    /// The arguments that have a role (the others, such as option records, are left out).
    pub args: Vec<Arg>,
    /// The format function that reads this call's content directly (`Excel.Workbook`), or
    /// through a step of the same query.
    pub format: Option<String>,
}

/// Most source calls reported for one expression.
const MAX_CALLS: usize = 10_000;

/// A `let` binding found by the scan: the value's token range.
struct Binding {
    name: String,
    /// The value's first token.
    from: usize,
    /// Whether the value is that one token.
    single: bool,
}

fn name_of(t: &Tok) -> Option<&str> {
    match t {
        Tok::Ident(s) | Tok::Quoted(s) => Some(s),
        _ => None,
    }
}

fn is_punct(t: Option<&Token>, p: &str) -> bool {
    matches!(t, Some(Token { tok: Tok::Punct(x), .. }) if *x == p)
}

fn is_ident(t: Option<&Token>, w: &str) -> bool {
    matches!(t, Some(Token { tok: Tok::Ident(x), .. }) if x == w)
}

/// The index of the token closing the bracket opened at `open` (`(`, `[` or `{`).
fn matching(toks: &[Token], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, t) in toks.iter().enumerate().skip(open) {
        if let Tok::Punct(p) = t.tok {
            match p {
                "(" | "[" | "{" => depth = depth.saturating_add(1),
                ")" | "]" | "}" => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// The `let` variables of the expression (every `let`, nested ones included).
fn bindings(toks: &[Token]) -> Vec<Binding> {
    // `true` for a `let` frame, `false` for a bracket
    let mut stack: Vec<bool> = Vec::new();
    let mut out = Vec::new();
    let mut i = 0usize;
    while let Some(t) = toks.get(i) {
        match &t.tok {
            Tok::Ident(w) if w == "let" => stack.push(true),
            Tok::Ident(w) if w == "in" => {
                if stack.last() == Some(&true) {
                    stack.pop();
                }
            }
            Tok::Punct("(" | "[" | "{") => stack.push(false),
            Tok::Punct(")" | "]" | "}") => {
                // a `let` left open inside brackets ends with them
                while stack.last() == Some(&true) {
                    stack.pop();
                }
                stack.pop();
            }
            tok => {
                let starts = i
                    .checked_sub(1)
                    .and_then(|p| toks.get(p))
                    .is_some_and(|p| is_ident(Some(p), "let") || is_punct(Some(p), ","));
                if stack.last() == Some(&true)
                    && starts
                    && let Some(name) = name_of(tok)
                    && is_punct(toks.get(i.saturating_add(1)), "=")
                {
                    let from = i.saturating_add(2);
                    let next = toks.get(from.saturating_add(1));
                    out.push(Binding {
                        name: name.to_string(),
                        from,
                        single: is_punct(next, ",") || is_ident(next, "in"),
                    });
                }
            }
        }
        i = i.saturating_add(1);
    }
    out
}

fn text_lit(t: &Token) -> Option<TextLit> {
    match &t.tok {
        Tok::Text(v) => Some(TextLit {
            value: v.clone(),
            start: t.start,
            end: t.end,
        }),
        _ => None,
    }
}

/// The closing token of every opening bracket (`None` for other tokens and unbalanced ones), in
/// one pass.
fn closers(toks: &[Token]) -> Vec<Option<usize>> {
    let mut out = vec![None; toks.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        match t.tok {
            Tok::Punct("(" | "[" | "{") => stack.push(i),
            Tok::Punct(")" | "]" | "}") => {
                if let Some(o) = stack.pop()
                    && let Some(slot) = out.get_mut(o)
                {
                    *slot = Some(i);
                }
            }
            _ => {}
        }
    }
    out
}

/// The `let` variables by name: how many bindings have the name, and the first one.
type BindIndex<'b> = HashMap<&'b str, (usize, &'b Binding)>;

fn index_bindings(binds: &[Binding]) -> BindIndex<'_> {
    let mut out: BindIndex<'_> = HashMap::new();
    for b in binds {
        out.entry(b.name.as_str())
            .and_modify(|(n, _)| *n = n.saturating_add(1))
            .or_insert((1, b));
    }
    out
}

/// Classifies the argument made of tokens `from..to`.
fn classify(toks: &[Token], from: usize, to: usize, close: &[Option<usize>], binds: &BindIndex<'_>) -> Form {
    let Some(slice) = toks.get(from..to) else {
        return Form::Computed;
    };
    match slice {
        [t] => {
            if let Some(l) = text_lit(t) {
                return Form::Text(l);
            }
            match &t.tok {
                Tok::Ident(w) if matches!(w.as_str(), "null" | "true" | "false") => Form::Computed,
                Tok::Ident(n) | Tok::Quoted(n) => match binds.get(n.as_str()) {
                    Some((1, b)) if b.single => match toks.get(b.from).and_then(text_lit) {
                        Some(l) => Form::StepText(n.clone(), l),
                        None => Form::Name {
                            name: n.clone(),
                            step: true,
                        },
                    },
                    found => Form::Name {
                        name: n.clone(),
                        step: found.is_some(),
                    },
                },
                _ => Form::Computed,
            }
        }
        [first, second, ..]
            if matches!(first.tok, Tok::Ident(_))
                && is_punct(Some(second), "(")
                && close.get(from.saturating_add(1)).copied().flatten() == Some(to.saturating_sub(1)) =>
        {
            Form::Call(name_of(&first.tok).unwrap_or_default().to_string())
        }
        _ if slice.len() >= 3 && slice.len() % 2 == 1 => {
            let mut parts = Vec::new();
            for (i, t) in slice.iter().enumerate() {
                if i % 2 == 1 {
                    if !is_punct(Some(t), "&") {
                        return Form::Computed;
                    }
                    continue;
                }
                match (&t.tok, text_lit(t)) {
                    (_, Some(l)) => parts.push(Part::Text(l)),
                    (Tok::Ident(n), None) if !matches!(n.as_str(), "null" | "true" | "false") => {
                        parts.push(Part::Name(n.clone(), t.start, t.end))
                    }
                    (Tok::Quoted(n), None) => parts.push(Part::Name(n.clone(), t.start, t.end)),
                    _ => return Form::Computed,
                }
            }
            Form::Concat(parts)
        }
        _ => Form::Computed,
    }
}

/// The source calls of `text`, in source order. `Err` when the text does not tokenize (a
/// syntax error); an unknown construct is never an error, it is reported as computed. The work
/// is linear in the number of tokens: brackets are matched once, and each call walks only its
/// own top-level tokens.
pub fn source_calls(text: &str) -> Result<Vec<SourceCall>, String> {
    let toks = tokenize(text, &Limits::default()).map_err(|e| e.to_string())?;
    let close_of = closers(&toks);
    let binds = bindings(&toks);
    let index = index_bindings(&binds);
    let mut calls = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        let Tok::Ident(function) = &t.tok else {
            continue;
        };
        let Some(roles) = source_roles(function) else {
            continue;
        };
        let open = i.saturating_add(1);
        if !is_punct(toks.get(open), "(") {
            continue;
        }
        let Some(close) = close_of.get(open).copied().flatten() else {
            continue;
        };
        if calls.len() >= MAX_CALLS {
            break;
        }
        let mut args = Vec::new();
        let mut from = open.saturating_add(1);
        let mut arg = 0usize;
        let mut j = from;
        while j <= close {
            let tok = toks.get(j).map(|t| &t.tok);
            if matches!(tok, Some(Tok::Punct("(" | "[" | "{"))) {
                // skip the nested brackets in one step
                match close_of.get(j).copied().flatten() {
                    Some(c) if c > j && c < close => {
                        j = c.saturating_add(1);
                        continue;
                    }
                    _ => break,
                }
            }
            if j == close || matches!(tok, Some(Tok::Punct(","))) {
                if let (Some(role), Some(first), Some(last)) = (
                    roles.get(arg),
                    toks.get(from),
                    j.checked_sub(1).and_then(|l| toks.get(l)),
                ) && from < j
                {
                    args.push(Arg {
                        role: *role,
                        form: classify(&toks, from, j, &close_of, &index),
                        start: first.start,
                        end: last.end,
                        line: first.line,
                        column: first.column,
                    });
                }
                arg = arg.saturating_add(1);
                from = j.saturating_add(1);
            }
            j = j.saturating_add(1);
        }
        calls.push(SourceCall {
            function: function.clone(),
            start: t.start,
            end: toks.get(close).map_or(t.end, |c| c.end),
            line: t.line,
            column: t.column,
            args,
            format: None,
        });
    }
    link_formats(&mut calls, &toks, &index);
    Ok(calls)
}

/// Sets `format` on the calls a format function reads, directly or through a step.
fn link_formats(calls: &mut [SourceCall], toks: &[Token], binds: &BindIndex<'_>) {
    let mut links: Vec<(usize, String)> = Vec::new();
    for c in calls.iter() {
        let Some(arg) = c.args.first().filter(|a| a.role == Role::Content) else {
            continue;
        };
        let target = match &arg.form {
            Form::Call(_) => Some(arg.start),
            Form::Name { name, step: true } => match binds.get(name.as_str()) {
                Some((1, b)) => toks.get(b.from).map(|t| t.start),
                _ => None,
            },
            _ => None,
        };
        if let Some(start) = target {
            links.push((start, c.function.clone()));
        }
    }
    let at: HashMap<usize, usize> = calls.iter().enumerate().map(|(i, c)| (c.start, i)).collect();
    for (start, format) in links {
        if let Some(c) = at.get(&start).and_then(|&i| calls.get_mut(i))
            && c.format.is_none()
        {
            c.format = Some(format);
        }
    }
}

/// A parameter query's value.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    Text(String),
    Number(f64),
    Logical(bool),
    Null,
    /// Anything else (`#date(2024, 1, 1)`), as written.
    Other(String),
}

/// A parameter query (`value meta [IsParameterQuery = true, …]`).
#[derive(Debug, Clone, PartialEq)]
pub struct ParamInfo {
    pub value: ParamValue,
    /// Byte range of the value before `meta`.
    pub value_start: usize,
    pub value_end: usize,
    /// The `Type` field of the metadata (`Text`, `Number`, `Date`, …).
    pub declared_type: Option<String>,
    /// The `List` of allowed values, when every item is a literal (as written in M).
    pub allowed: Option<Vec<String>>,
}

/// Reads a parameter query; `None` when `text` is not one.
pub fn parameter_info(text: &str) -> Option<ParamInfo> {
    let toks = tokenize(text, &Limits::default()).ok()?;
    // the top-level `meta`
    let mut depth = 0usize;
    let mut meta = None;
    for (i, t) in toks.iter().enumerate() {
        match &t.tok {
            Tok::Punct("(" | "[" | "{") => depth = depth.saturating_add(1),
            Tok::Punct(")" | "]" | "}") => depth = depth.saturating_sub(1),
            Tok::Ident(w) if (w == "let" || w == "each") && depth == 0 => return None,
            Tok::Ident(w) if w == "meta" && depth == 0 => {
                meta = Some(i);
                break;
            }
            _ => {}
        }
    }
    let meta = meta?;
    let value_toks = toks.get(..meta)?;
    let (first, last) = (value_toks.first()?, value_toks.last()?);
    let open = meta.checked_add(1)?;
    if !is_punct(toks.get(open), "[") {
        return None;
    }
    let close = matching(&toks, open)?;
    let mut is_param = false;
    let mut declared_type = None;
    let mut allowed = None;
    // fields `key = value` at depth 1
    let mut i = open.checked_add(1)?;
    while i < close {
        let key = toks.get(i).and_then(|t| name_of(&t.tok));
        let vstart = i.checked_add(2)?;
        let mut j = vstart;
        let mut d = 0usize;
        while j < close {
            match toks.get(j).map(|t| &t.tok) {
                Some(Tok::Punct("(" | "[" | "{")) => d = d.saturating_add(1),
                Some(Tok::Punct(")" | "]" | "}")) => d = d.saturating_sub(1),
                Some(Tok::Punct(",")) if d == 0 => break,
                _ => {}
            }
            j = j.checked_add(1)?;
        }
        if let (Some(key), true) = (key, is_punct(toks.get(i.checked_add(1)?), "=")) {
            let v = toks.get(vstart..j).unwrap_or(&[]);
            match (key, v) {
                ("IsParameterQuery", [t]) => is_param = is_ident(Some(t), "true"),
                ("Type", [t]) => {
                    if let Tok::Text(s) = &t.tok {
                        declared_type = Some(s.clone());
                    }
                }
                ("List", [o, items @ .., c]) if is_punct(Some(o), "{") && is_punct(Some(c), "}") => {
                    let mut list = Vec::new();
                    let mut ok = true;
                    for (k, t) in items.iter().enumerate() {
                        if k % 2 == 1 {
                            ok &= is_punct(Some(t), ",");
                        } else if matches!(t.tok, Tok::Text(_) | Tok::Number(_)) {
                            list.push(text.get(t.start..t.end).unwrap_or_default().to_string());
                        } else {
                            ok = false;
                        }
                    }
                    if ok {
                        allowed = Some(list);
                    }
                }
                _ => {}
            }
        }
        i = j.checked_add(1)?;
    }
    if !is_param {
        return None;
    }
    let value = match value_toks {
        [t] => match &t.tok {
            Tok::Text(s) => ParamValue::Text(s.clone()),
            Tok::Number(n) => ParamValue::Number(n.f()),
            Tok::Ident(w) if w == "true" => ParamValue::Logical(true),
            Tok::Ident(w) if w == "false" => ParamValue::Logical(false),
            Tok::Ident(w) if w == "null" => ParamValue::Null,
            _ => ParamValue::Other(text.get(t.start..t.end).unwrap_or_default().to_string()),
        },
        [m, t] if is_punct(Some(m), "-") && matches!(t.tok, Tok::Number(_)) => match &t.tok {
            Tok::Number(n) => ParamValue::Number(-n.f()),
            _ => ParamValue::Null,
        },
        _ => ParamValue::Other(text.get(first.start..last.end).unwrap_or_default().to_string()),
    };
    Some(ParamInfo {
        value,
        value_start: first.start,
        value_end: last.end,
        declared_type,
        allowed,
    })
}

/// `s` as an M text literal: quotes doubled, control characters and `#(` escaped.
pub fn text_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len().saturating_add(2));
    out.push('"');
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => out.push_str("\"\""),
            '\r' => out.push_str("#(cr)"),
            '\n' => out.push_str("#(lf)"),
            '\t' => out.push_str("#(tab)"),
            '#' if chars.peek() == Some(&'(') => out.push_str("#(#)"),
            c if c.is_control() => out.push_str(&format!("#({:04X})", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
