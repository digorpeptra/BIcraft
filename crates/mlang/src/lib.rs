//! L0 language services for M (Power Query): BIcraft's own catalogue of the library functions
//! its evaluator runs (parameters and a short description each, [`functions`]), the library
//! constants and the keywords, and over M text being typed:
//!
//! - [`complete`]: what may come at the cursor (functions, constants, keywords, the query's
//!   steps, the model's queries, the columns in scope inside `[ ]`), best matches first;
//! - [`signature_at`]: the library call the cursor is in and the argument being typed;
//! - [`hover`]: what the name under the cursor is (a function's signature and description, a
//!   constant, a keyword, a step, a query or a column).
//!
//! The scanner here is forgiving: half-typed text (`Table.`, an open string or bracket) never
//! fails, and every function returns `None` rather than guess. Offsets are byte offsets into
//! the text; [`byte_of_char`] and [`char_of_byte`] convert the character offsets editors use.
//! The engine's `m.complete`, `m.hover` and `m.signature` commands and the app's Power Query
//! editor (Advanced editor, formula bar) share these functions.
//!
//! A foundation crate: no workspace dependency, usable by every layer (`cargo xtask layers`).
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod catalog;

use std::ops::Range;
use std::sync::OnceLock;

/// One parameter of a library function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Param {
    pub name: &'static str,
    pub optional: bool,
    /// Its M type as M writes it: `number`, `nullable text`, `any`, `anynonnull`.
    pub ty: &'static str,
}

/// One library function of the catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    /// `Table.SelectRows`, `#date`.
    pub name: &'static str,
    /// Required parameters first, then the optional ones.
    pub params: Vec<Param>,
    /// Its return type as M writes it (`nullable text`).
    pub returns: &'static str,
    /// What it does, in one sentence (BIcraft's words).
    pub description: &'static str,
}

impl Function {
    /// The number of required parameters.
    pub fn required(&self) -> usize {
        self.params.iter().filter(|p| !p.optional).count()
    }

    /// `Text.PadStart(text, count, optional character)`.
    pub fn signature(&self) -> String {
        let params: Vec<String> = self
            .params
            .iter()
            .map(|p| {
                if p.optional {
                    format!("optional {}", p.name)
                } else {
                    p.name.to_string()
                }
            })
            .collect();
        format!("{}({})", self.name, params.join(", "))
    }
}

/// A library constant or type name (`Order.Descending`, `Int64.Type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Constant {
    pub name: &'static str,
    /// What its group is (`How a sort orders values`).
    pub description: &'static str,
}

/// The catalogue's functions, sorted by name.
pub fn functions() -> &'static [Function] {
    static ALL: OnceLock<Vec<Function>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut out: Vec<Function> = catalog::FUNCTIONS.lines().filter_map(parse_function).collect();
        out.sort_by(|a, b| a.name.cmp(b.name));
        out
    })
}

fn parse_function(line: &'static str) -> Option<Function> {
    let (sig, description) = line.split_once(" | ")?;
    let (name, rest) = sig.split_once('(')?;
    let (inner, returns) = rest.rsplit_once(") as ")?;
    let params = inner
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (optional, p) = match p.strip_prefix("optional ") {
                Some(n) => (true, n),
                None => (false, p),
            };
            let (name, ty) = p.split_once(" as ").unwrap_or((p, "any"));
            Param { name, optional, ty }
        })
        .collect();
    Some(Function {
        name,
        params,
        returns: returns.trim(),
        description: description.trim(),
    })
}

/// The catalogue function named exactly `name` (M names are case-sensitive).
pub fn function(name: &str) -> Option<&'static Function> {
    let all = functions();
    all.binary_search_by(|f| f.name.cmp(name))
        .ok()
        .and_then(|i| all.get(i))
}

/// The library constants and type names the evaluator knows, sorted by name.
pub fn constants() -> &'static [Constant] {
    static ALL: OnceLock<Vec<Constant>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut out = Vec::new();
        for line in catalog::CONSTANTS.lines() {
            let Some((description, names)) = line.split_once(" | ") else {
                continue;
            };
            for name in names.split_whitespace() {
                out.push(Constant { name, description });
            }
        }
        out.sort_by(|a, b| a.name.cmp(b.name));
        out
    })
}

/// The constant named exactly `name`.
pub fn constant(name: &str) -> Option<&'static Constant> {
    let all = constants();
    all.binary_search_by(|c| c.name.cmp(name))
        .ok()
        .and_then(|i| all.get(i))
}

/// The M keywords with what each does.
pub fn keywords() -> &'static [(&'static str, &'static str)] {
    catalog::KEYWORDS
}

fn keyword(name: &str) -> Option<&'static str> {
    catalog::KEYWORDS
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, d)| *d)
}

/// `name` as an M identifier: as written when it is a regular identifier (letters, digits and
/// `_`, not starting with a digit, dotted parts allowed, not a keyword), else `#"…"` with `"`
/// doubled.
pub fn identifier(name: &str) -> String {
    let regular = !name.is_empty()
        && keyword(name).is_none()
        && name.split('.').all(|part| {
            let mut chars = part.chars();
            chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
                && chars.all(|c| c.is_alphanumeric() || c == '_')
        });
    if regular {
        name.to_string()
    } else {
        format!("#\"{}\"", name.replace('"', "\"\""))
    }
}

/// The byte offset of character `ch` of `text` (the end when past it).
pub fn byte_of_char(text: &str, ch: usize) -> usize {
    text.char_indices().nth(ch).map_or(text.len(), |(b, _)| b)
}

/// The character offset of byte `b` of `text` (a byte inside a character counts as that
/// character; past the end, the number of characters).
pub fn char_of_byte(text: &str, b: usize) -> usize {
    text.char_indices().take_while(|(i, _)| *i < b).count()
}

/// The largest character boundary of `text` at or before `b`.
fn floor(text: &str, b: usize) -> usize {
    let mut b = b.min(text.len());
    while b > 0 && !text.is_char_boundary(b) {
        b -= 1;
    }
    b
}

// ---------------------------------------------------------------------------------------------
// The forgiving scanner

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum K {
    /// A name, dotted parts included (`Table.SelectRows`; a trailing `.` at the scan's end).
    Ident,
    /// `#date`, or `#` alone.
    Hash,
    /// `#"…"`.
    Quoted {
        closed: bool,
    },
    Text {
        closed: bool,
    },
    Number,
    Comment {
        closed: bool,
    },
    /// Punctuation and operators (`=`, `=>`, `(`, `,` …).
    Punct,
}

#[derive(Debug, Clone, Copy)]
struct Tok {
    k: K,
    start: usize,
    end: usize,
}

/// The tokens of `text[..limit]`, never failing.
fn scan(text: &str, limit: usize) -> Vec<Tok> {
    let src = text.get(..floor(text, limit)).unwrap_or("");
    let b = src.as_bytes();
    let at = |i: usize| b.get(i).copied().unwrap_or(0);
    let char_at = |i: usize| src.get(i..).and_then(|s| s.chars().next());
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let Some(c) = char_at(i) else { break };
        let start = i;
        let k = if c.is_whitespace() || c == '\u{feff}' {
            i += c.len_utf8();
            continue;
        } else if c == '/' && at(i + 1) == b'/' {
            i = src.get(i..).and_then(|s| s.find('\n')).map_or(b.len(), |n| i + n);
            K::Comment { closed: i < b.len() }
        } else if c == '/' && at(i + 1) == b'*' {
            match src.get(i + 2..).and_then(|s| s.find("*/")) {
                Some(n) => {
                    i += 2 + n + 2;
                    K::Comment { closed: true }
                }
                None => {
                    i = b.len();
                    K::Comment { closed: false }
                }
            }
        } else if c == '"' {
            let (end, closed) = string_end(b, i + 1);
            i = end;
            K::Text { closed }
        } else if c == '#' && at(i + 1) == b'"' {
            let (end, closed) = string_end(b, i + 2);
            i = end;
            K::Quoted { closed }
        } else if c == '#' {
            i += 1;
            i = ident_end(src, i);
            K::Hash
        } else if c.is_alphabetic() || c == '_' {
            i = ident_end(src, i);
            K::Ident
        } else if c.is_ascii_digit() {
            while i < b.len()
                && (at(i).is_ascii_alphanumeric() || (at(i) == b'.' && at(i + 1).is_ascii_digit()))
            {
                i += 1;
            }
            K::Number
        } else {
            let rest = src.get(i..).unwrap_or("");
            let n = ["...", "=>", "<=", ">=", "<>", "..", "??"]
                .iter()
                .find(|p| rest.starts_with(**p))
                .map_or(c.len_utf8(), |p| p.len());
            i += n;
            K::Punct
        };
        out.push(Tok { k, start, end: i });
    }
    out
}

/// The end of a `"…"` body starting at `i` (after the opening quote), `""` read as a quote.
fn string_end(b: &[u8], mut i: usize) -> (usize, bool) {
    while i < b.len() {
        if b.get(i) == Some(&b'"') {
            if b.get(i + 1) == Some(&b'"') {
                i += 2;
                continue;
            }
            return (i + 1, true);
        }
        i += 1;
    }
    (b.len(), false)
}

/// The end of a name starting at `i`: letters, digits, `_`, and `.` before a letter (or at the
/// end of the text: `Table.` being typed).
fn ident_end(src: &str, mut i: usize) -> usize {
    while let Some(c) = src.get(i..).and_then(|s| s.chars().next()) {
        if c.is_alphanumeric() || c == '_' {
            i += c.len_utf8();
        } else if c == '.' {
            let next = src.get(i + 1..).and_then(|s| s.chars().next());
            match next {
                Some(n) if n.is_alphabetic() || n == '_' => i += 1,
                None => i += 1,
                _ => break,
            }
        } else {
            break;
        }
    }
    i
}

fn slice<'a>(text: &'a str, t: &Tok) -> &'a str {
    text.get(t.start..t.end).unwrap_or("")
}

fn significant(t: &Tok) -> bool {
    !matches!(t.k, K::Comment { .. })
}

/// The inner text of a quoted identifier token, `""` read as `"`.
fn quoted_name(text: &str, t: &Tok) -> String {
    let raw = slice(text, t);
    let inner = raw.get(2..).unwrap_or("");
    let inner = match t.k {
        K::Quoted { closed: true } => inner.strip_suffix('"').unwrap_or(inner),
        _ => inner,
    };
    inner.replace("\"\"", "\"")
}

/// The names `let` defines in `text` (its steps), in order, each once. A record's fields and a
/// function's parameters are not steps.
pub fn bindings(text: &str) -> Vec<String> {
    let toks: Vec<Tok> = scan(text, text.len()).into_iter().filter(significant).collect();
    // the open brackets, `l` for a let
    let mut stack: Vec<u8> = Vec::new();
    let mut out: Vec<String> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        let s = slice(text, t);
        match (t.k, s) {
            (K::Ident, "let") => stack.push(b'l'),
            (K::Ident, "in") => {
                if let Some(p) = stack.iter().rposition(|c| *c == b'l') {
                    stack.truncate(p);
                }
            }
            (K::Punct, "(" | "[" | "{") => stack.push(s.as_bytes().first().copied().unwrap_or(b'(')),
            (K::Punct, ")" | "]" | "}") => {
                let open = match s {
                    ")" => b'(',
                    "]" => b'[',
                    _ => b'{',
                };
                if let Some(p) = stack.iter().rposition(|c| *c == open) {
                    stack.truncate(p);
                }
            }
            (K::Ident | K::Quoted { closed: true }, _) if stack.last() == Some(&b'l') => {
                let before = i.checked_sub(1).and_then(|j| toks.get(j));
                let after_let_or_comma = before.is_some_and(|b| {
                    let bs = slice(text, b);
                    (b.k == K::Ident && bs == "let") || (b.k == K::Punct && bs == ",")
                });
                let then_equals = toks
                    .get(i + 1)
                    .is_some_and(|n| n.k == K::Punct && slice(text, n) == "=");
                if after_let_or_comma && then_equals {
                    let name = if t.k == K::Ident {
                        s.to_string()
                    } else {
                        quoted_name(text, t)
                    };
                    if !name.is_empty() && !out.contains(&name) {
                        out.push(name);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Completion

/// What a suggestion is (also the order of equally good matches: local names first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ItemKind {
    Column,
    Step,
    Query,
    Keyword,
    Function,
    Constant,
}

impl ItemKind {
    /// `column`, `step`, `query`, `keyword`, `function`, `constant`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Column => "column",
            Self::Step => "step",
            Self::Query => "query",
            Self::Keyword => "keyword",
            Self::Function => "function",
            Self::Constant => "constant",
        }
    }
}

/// One suggestion: its label, the text it inserts in place of the completion's range, what it
/// is, and a detail (a function's signature, a constant's or keyword's description).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub label: String,
    pub insert: String,
    pub kind: ItemKind,
    pub detail: String,
}

/// The suggestions at a cursor and the bytes a picked one replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub replace: Range<usize>,
    pub items: Vec<Item>,
}

/// The names in scope that the text alone does not tell: the model's queries, more step names
/// (the steps before the formula bar's), the columns of the table being transformed.
#[derive(Debug, Clone, Copy, Default)]
pub struct Scope<'a> {
    pub queries: &'a [String],
    pub steps: &'a [String],
    pub columns: &'a [String],
}

/// Most suggestions returned.
pub const MAX_ITEMS: usize = 200;

/// How well `key` matches `prefix`, case ignored: 0 it starts with it, 1 a dotted part of it
/// does (`Sel` for `Table.SelectRows`), 2 it contains it.
fn rank(prefix: &str, key: &str) -> Option<u8> {
    let p = prefix.to_lowercase();
    let k = key.to_lowercase();
    if k.starts_with(&p) {
        Some(0)
    } else if !p.is_empty() && k.split('.').skip(1).any(|part| part.starts_with(&p)) {
        Some(1)
    } else if !p.is_empty() && k.contains(&p) {
        Some(2)
    } else {
        None
    }
}

/// The suggestions at byte `cursor` of `text`. Without `explicit` (Ctrl+Space) the list opens
/// only on a name being typed or inside `[`; none in a comment or a text.
pub fn complete(text: &str, cursor: usize, scope: &Scope, explicit: bool) -> Option<Completion> {
    let c = floor(text, cursor);
    let toks = scan(text, c);
    let last = toks.last().copied();
    if let Some(t) = last {
        match t.k {
            K::Comment { closed: false } if t.end >= c => return None,
            K::Comment { closed: true } if t.end > c => return None,
            K::Text { closed: false } => return None,
            K::Text { closed: true } | K::Number if t.end == c => return None,
            K::Quoted { closed: false } => return quoted_completion(text, &t, c, scope),
            _ => {}
        }
    }
    if let Some(open) = field_open(text, &toks) {
        return field_completion(text, open, c, scope, explicit);
    }
    let (replace, prefix) = match last {
        Some(t) if matches!(t.k, K::Ident | K::Hash) && t.end == c => (t.start..c, slice(text, &t)),
        _ if explicit => (c..c, ""),
        _ => return None,
    };
    let mut steps = bindings(text);
    steps.retain(|s| s != prefix);
    for s in scope.steps {
        if !steps.contains(s) {
            steps.push(s.clone());
        }
    }
    let mut out: Vec<(u8, Item)> = Vec::new();
    let mut push = |r: Option<u8>, item: Item| {
        if let Some(r) = r {
            out.push((r, item));
        }
    };
    for s in &steps {
        push(rank(prefix, s), local(s, ItemKind::Step));
    }
    for q in scope.queries {
        push(rank(prefix, q), local(q, ItemKind::Query));
    }
    for f in functions() {
        let insert = if f.params.is_empty() {
            format!("{}()", f.name)
        } else {
            format!("{}(", f.name)
        };
        push(
            rank(prefix, f.name),
            Item {
                label: f.name.to_string(),
                insert,
                kind: ItemKind::Function,
                detail: f.signature(),
            },
        );
    }
    for k in constants() {
        push(
            rank(prefix, k.name),
            Item {
                label: k.name.to_string(),
                insert: k.name.to_string(),
                kind: ItemKind::Constant,
                detail: k.description.to_string(),
            },
        );
    }
    if !prefix.contains(['.', '#']) {
        for (k, d) in keywords() {
            let insert = if matches!(*k, "true" | "false" | "null") {
                (*k).to_string()
            } else {
                format!("{k} ")
            };
            push(
                rank(prefix, k).filter(|r| *r == 0),
                Item {
                    label: (*k).to_string(),
                    insert,
                    kind: ItemKind::Keyword,
                    detail: (*d).to_string(),
                },
            );
        }
    }
    finish(out, prefix, explicit, replace)
}

fn local(name: &str, kind: ItemKind) -> Item {
    Item {
        label: name.to_string(),
        insert: identifier(name),
        kind,
        detail: String::new(),
    }
}

/// Sorted best first, repeated suggestions dropped, capped; none when the only one is what is
/// typed already (unless asked for).
fn finish(
    mut out: Vec<(u8, Item)>,
    prefix: &str,
    explicit: bool,
    replace: Range<usize>,
) -> Option<Completion> {
    out.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.kind.cmp(&b.1.kind))
            .then_with(|| a.1.label.to_lowercase().cmp(&b.1.label.to_lowercase()))
    });
    out.dedup_by(|a, b| a.1.label == b.1.label && a.1.kind == b.1.kind);
    let items: Vec<Item> = out.into_iter().take(MAX_ITEMS).map(|x| x.1).collect();
    let only_itself = items.len() == 1 && items.first().is_some_and(|i| i.label == prefix);
    (!items.is_empty() && (explicit || !only_itself)).then_some(Completion { replace, items })
}

/// `#"Ren` being typed: the steps and queries, quoted.
fn quoted_completion(text: &str, t: &Tok, c: usize, scope: &Scope) -> Option<Completion> {
    let prefix = quoted_name(text, t);
    let mut out = Vec::new();
    let mut names: Vec<(String, ItemKind)> = bindings(text)
        .into_iter()
        .chain(scope.steps.iter().cloned())
        .map(|s| (s, ItemKind::Step))
        .collect();
    names.extend(scope.queries.iter().map(|q| (q.clone(), ItemKind::Query)));
    for (name, kind) in names {
        if name == prefix {
            continue;
        }
        if let Some(r) = rank(&prefix, &name) {
            out.push((
                r,
                Item {
                    insert: format!("#\"{}\"", name.replace('"', "\"\"")),
                    label: name,
                    kind,
                    detail: String::new(),
                },
            ));
        }
    }
    finish(out, &prefix, true, t.start..c)
}

/// The `[` the cursor is in when what follows it is a field name being typed (names, numbers
/// and spaces only): its byte offset.
fn field_open(text: &str, toks: &[Tok]) -> Option<usize> {
    for t in toks.iter().rev() {
        match t.k {
            K::Ident | K::Number => continue,
            K::Punct if slice(text, t) == "[" => return Some(t.end),
            _ => return None,
        }
    }
    None
}

/// The columns in scope after `[`: `Name]` (the `]` unless one follows).
fn field_completion(text: &str, open: usize, c: usize, scope: &Scope, explicit: bool) -> Option<Completion> {
    let typed = text.get(open..c).unwrap_or("");
    let start = open + (typed.len() - typed.trim_start().len());
    let prefix = typed.trim_start();
    let closes = text
        .get(c..)
        .is_some_and(|rest| rest.trim_start().starts_with(']'));
    let out = scope
        .columns
        .iter()
        .filter_map(|col| {
            rank(prefix, col).map(|r| {
                (
                    r,
                    Item {
                        label: col.clone(),
                        insert: if closes {
                            col.replace(']', "]]")
                        } else {
                            format!("{}]", col.replace(']', "]]"))
                        },
                        kind: ItemKind::Column,
                        detail: String::new(),
                    },
                )
            })
        })
        .collect();
    finish(out, prefix, explicit || prefix.is_empty(), start..c)
}

// ---------------------------------------------------------------------------------------------
// Signature help

/// The library call the cursor is in: the function and the parameter being typed (none past
/// the last parameter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureHelp {
    pub function: &'static Function,
    /// The argument's position (from 0) among those typed so far.
    pub argument: usize,
    /// The parameter it fills (none past the last).
    pub active: Option<usize>,
}

/// The innermost library call around byte `cursor` (an unknown function's call is looked
/// through to the one around it); none in a comment.
pub fn signature_at(text: &str, cursor: usize) -> Option<SignatureHelp> {
    let c = floor(text, cursor);
    let toks = scan(text, c);
    if toks.last().is_some_and(|t| {
        matches!(t.k, K::Comment { closed: false }) || (matches!(t.k, K::Comment { .. }) && t.end > c)
    }) {
        return None;
    }
    // (bracket, the name before a `(`, commas at this level)
    let mut stack: Vec<(u8, Option<&str>, usize)> = Vec::new();
    let mut prev: Option<Tok> = None;
    for t in toks.iter().filter(|t| significant(t)) {
        if t.k == K::Punct {
            match slice(text, t) {
                "(" => {
                    let name = prev
                        .filter(|p| matches!(p.k, K::Ident | K::Hash))
                        .map(|p| slice(text, &p));
                    stack.push((b'(', name, 0));
                }
                "[" => stack.push((b'[', None, 0)),
                "{" => stack.push((b'{', None, 0)),
                ")" | "]" | "}" => {
                    stack.pop();
                }
                "," => {
                    if let Some(top) = stack.last_mut() {
                        top.2 += 1;
                    }
                }
                _ => {}
            }
        }
        prev = Some(*t);
    }
    stack.iter().rev().find_map(|(b, name, args)| {
        let f = name.filter(|_| *b == b'(').and_then(function)?;
        Some(SignatureHelp {
            function: f,
            argument: *args,
            active: (*args < f.params.len()).then_some(*args),
        })
    })
}

// ---------------------------------------------------------------------------------------------
// Hover

/// What the name under the cursor is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hover {
    /// The bytes of the name.
    pub range: Range<usize>,
    pub name: String,
    pub kind: ItemKind,
    /// A function's signature.
    pub signature: Option<String>,
    /// A function's, constant's or keyword's description (empty for local names).
    pub description: String,
}

/// The name at byte `cursor` (on it or just after it) and what it is; none for anything else.
pub fn hover(text: &str, cursor: usize, scope: &Scope) -> Option<Hover> {
    let c = floor(text, cursor);
    let toks = scan(text, text.len());
    let named = |t: &Tok| matches!(t.k, K::Ident | K::Hash | K::Quoted { .. });
    let i = toks
        .iter()
        .position(|t| t.start <= c && c < t.end && named(t))
        .or_else(|| toks.iter().position(|t| t.end == c && named(t)))?;
    let t = *toks.get(i)?;
    let found = |name: String, kind: ItemKind, range: Range<usize>| {
        Some(Hover {
            range,
            name,
            kind,
            signature: None,
            description: String::new(),
        })
    };
    match t.k {
        K::Ident | K::Hash => {
            // a field name `[Unit Price]` (names between the brackets)
            if let Some((range, name)) = field_around(text, &toks, i)
                && scope.columns.contains(&name)
            {
                return found(name, ItemKind::Column, range);
            }
            let name = slice(text, &t).trim_end_matches('.');
            let range = t.start..t.start + name.len();
            if let Some(f) = function(name) {
                return Some(Hover {
                    range,
                    name: name.to_string(),
                    kind: ItemKind::Function,
                    signature: Some(f.signature()),
                    description: f.description.to_string(),
                });
            }
            if let Some(k) = constant(name) {
                return Some(Hover {
                    range,
                    name: name.to_string(),
                    kind: ItemKind::Constant,
                    signature: None,
                    description: k.description.to_string(),
                });
            }
            if let Some(d) = keyword(name) {
                return Some(Hover {
                    range,
                    name: name.to_string(),
                    kind: ItemKind::Keyword,
                    signature: None,
                    description: d.to_string(),
                });
            }
            local_hover(text, name.to_string(), range, scope)
        }
        K::Quoted { .. } => local_hover(text, quoted_name(text, &t), t.start..t.end, scope),
        _ => None,
    }
}

fn local_hover(text: &str, name: String, range: Range<usize>, scope: &Scope) -> Option<Hover> {
    let kind = if bindings(text).contains(&name) || scope.steps.contains(&name) {
        ItemKind::Step
    } else if scope.queries.contains(&name) {
        ItemKind::Query
    } else {
        return None;
    };
    Some(Hover {
        range,
        name,
        kind,
        signature: None,
        description: String::new(),
    })
}

/// When token `i` is inside `[ … ]` holding only names and numbers: the range between the
/// brackets (trimmed) and its text.
fn field_around(text: &str, toks: &[Tok], i: usize) -> Option<(Range<usize>, String)> {
    let plain = |t: &Tok| matches!(t.k, K::Ident | K::Number);
    let mut a = i;
    while a > 0 && toks.get(a - 1).is_some_and(plain) {
        a -= 1;
    }
    let mut z = i;
    while toks.get(z + 1).is_some_and(plain) {
        z += 1;
    }
    let open = toks.get(a.checked_sub(1)?)?;
    let close = toks.get(z + 1)?;
    if slice(text, open) != "[" || slice(text, close) != "]" {
        return None;
    }
    let (s, e) = (toks.get(a)?.start, toks.get(z)?.end);
    Some((s..e, text.get(s..e)?.to_string()))
}

#[cfg(test)]
mod tests;
