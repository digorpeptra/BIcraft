//! M11.35: the M language services as commands, over `bicraft_mlang` (BIcraft's catalogue of
//! the functions its evaluator runs):
//!
//! - `m.complete`: the suggestions at a cursor (functions, constants, keywords, the text's
//!   steps, the model's queries, the columns in scope inside `[ ]`);
//! - `m.signature`: the library call the cursor is in and the parameter being typed;
//! - `m.hover`: what the name at a cursor is.
//!
//! Read-only and nothing is evaluated. Cursors and ranges count characters.

use serde_json::{Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, opt_str, str_param};

/// Longest M text the language services take (the editors' limit).
const MAX_TEXT: usize = 1 << 20;
/// Most names a caller may add to the scope.
const MAX_NAMES: usize = 10_000;

/// The text, the cursor (a byte offset) and the names in scope of one call.
struct Input {
    text: String,
    cursor: usize,
    queries: Vec<String>,
    steps: Vec<String>,
    columns: Vec<String>,
}

impl Input {
    fn scope(&self) -> bicraft_mlang::Scope<'_> {
        bicraft_mlang::Scope {
            queries: &self.queries,
            steps: &self.steps,
            columns: &self.columns,
        }
    }
}

fn names_param(p: &Value, key: &str) -> Result<Vec<String>, String> {
    let bad = || format!("parameter `{key}` must be a list of at most {MAX_NAMES} strings");
    match p.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) if a.len() <= MAX_NAMES => a
            .iter()
            .map(|v| v.as_str().map(str::to_string).ok_or_else(bad))
            .collect(),
        Some(_) => Err(bad()),
    }
}

fn opt_index(p: &Value, key: &str) -> Result<Option<usize>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| format!("parameter `{key}` must be a whole number from 0")),
    }
}

/// The byte offset of `cursor` (characters from the start) or of `line` and `column` (from 1,
/// in characters); the end of the text by default. Past the end of a line or of the text: its
/// end.
fn cursor_of(text: &str, p: &Value) -> Result<usize, String> {
    let cursor = opt_index(p, "cursor")?;
    let line = opt_index(p, "line")?;
    let column = opt_index(p, "column")?;
    match (cursor, line, column) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
            Err("pass `cursor` or `line` and `column`, not both".into())
        }
        (Some(c), None, None) => Ok(bicraft_mlang::byte_of_char(text, c)),
        (None, None, None) => Ok(text.len()),
        (None, line, column) => {
            let (line, column) = (line.unwrap_or(1), column.unwrap_or(1));
            if line == 0 || column == 0 {
                return Err("`line` and `column` count from 1".into());
            }
            let mut start = 0;
            for _ in 1..line {
                match text.get(start..).and_then(|r| r.find('\n')) {
                    Some(n) => start += n + 1,
                    None => return Err(format!("the text has no line {line}")),
                }
            }
            let this_line = text.get(start..).unwrap_or("").split('\n').next().unwrap_or("");
            Ok(start + bicraft_mlang::byte_of_char(this_line, column - 1))
        }
    }
}

fn input(s: &Session, p: &Value, keys: &[&str]) -> Result<Input, String> {
    allow(p, keys)?;
    let text = str_param(p, "expression")?;
    if text.len() > MAX_TEXT {
        return Err(format!("`expression` is longer than {MAX_TEXT} bytes"));
    }
    let cursor = cursor_of(text, p)?;
    let mut queries: Vec<String> = s
        .model
        .as_ref()
        .map(|m| crate::mquery::query_names(m).into_iter().collect())
        .unwrap_or_default();
    queries.sort();
    queries.extend(names_param(p, "queries")?);
    let steps = names_param(p, "steps")?;
    let mut columns = names_param(p, "columns")?;
    if let Some(t) = opt_str(p, "table")? {
        let model = s
            .model
            .as_ref()
            .ok_or("`table` needs an open model (file.open first)")?;
        let table = model
            .tables
            .iter()
            .find(|x| x.name.eq_ignore_ascii_case(t))
            .ok_or_else(|| format!("no table `{t}` in the model"))?;
        for c in &table.columns {
            if !columns.contains(&c.name) {
                columns.push(c.name.clone());
            }
        }
    }
    Ok(Input {
        text: text.to_string(),
        cursor,
        queries,
        steps,
        columns,
    })
}

/// A byte range of `text` in characters.
fn char_range(text: &str, r: &std::ops::Range<usize>) -> Value {
    json!({
        "start": bicraft_mlang::char_of_byte(text, r.start),
        "end": bicraft_mlang::char_of_byte(text, r.end),
    })
}

fn params_json(f: &bicraft_mlang::Function) -> Vec<Value> {
    f.params
        .iter()
        .map(|x| json!({"name": x.name, "optional": x.optional}))
        .collect()
}

const SCOPE_KEYS: &[&str] = &[
    "expression",
    "cursor",
    "line",
    "column",
    "table",
    "columns",
    "queries",
    "steps",
];

/// `m.complete {expression, cursor? | line?, column?, table?, columns?, queries?, steps?,
/// explicit?}`.
pub(crate) fn m_complete(s: &Session, p: &Value) -> Result<Value, String> {
    let mut keys = SCOPE_KEYS.to_vec();
    keys.push("explicit");
    let a = input(s, p, &keys)?;
    let explicit = opt_bool(p, "explicit")?.unwrap_or(false);
    let Some(c) = bicraft_mlang::complete(&a.text, a.cursor, &a.scope(), explicit) else {
        let at = bicraft_mlang::char_of_byte(&a.text, a.cursor);
        return Ok(json!({"replace": {"start": at, "end": at}, "items": []}));
    };
    let items: Vec<Value> = c
        .items
        .iter()
        .map(|i| json!({"label": i.label, "insert": i.insert, "kind": i.kind.as_str(), "detail": i.detail}))
        .collect();
    Ok(json!({"replace": char_range(&a.text, &c.replace), "items": items}))
}

/// `m.signature {expression, cursor? | line?, column?}`.
pub(crate) fn m_signature(s: &Session, p: &Value) -> Result<Value, String> {
    let a = input(s, p, &["expression", "cursor", "line", "column"])?;
    Ok(match bicraft_mlang::signature_at(&a.text, a.cursor) {
        None => json!({"function": null, "argument": null, "active": null}),
        Some(h) => json!({
            "function": {
                "name": h.function.name,
                "signature": h.function.signature(),
                "description": h.function.description,
                "params": params_json(h.function),
            },
            "argument": h.argument,
            "active": h.active,
        }),
    })
}

/// `m.hover {expression, cursor? | line?, column?, table?, columns?, queries?, steps?}`.
pub(crate) fn m_hover(s: &Session, p: &Value) -> Result<Value, String> {
    let a = input(s, p, SCOPE_KEYS)?;
    Ok(match bicraft_mlang::hover(&a.text, a.cursor, &a.scope()) {
        None => json!({"hover": null}),
        Some(h) => {
            let mut v = json!({
                "name": h.name,
                "kind": h.kind.as_str(),
                "range": char_range(&a.text, &h.range),
                "signature": h.signature,
                "description": h.description,
            });
            if let Some(f) =
                bicraft_mlang::function(&h.name).filter(|_| h.kind == bicraft_mlang::ItemKind::Function)
            {
                v["params"] = json!(params_json(f));
            }
            json!({"hover": v})
        }
    })
}
