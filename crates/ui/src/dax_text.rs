//! DAX text for the formula bar ([`crate::formula`]): a small lexer for the syntax colours, the
//! completion context at the cursor, the suggestions it gets from the model's names and the
//! engine's function list, and the call the cursor is in (the signature help). Pure functions
//! over the text, no engine (the browser links none); any text, any cursor, never a panic.

use std::ops::Range;

/// What a piece of DAX text is, for its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    /// Variables, unknown words, white space.
    Plain,
    Keyword,
    /// A word followed by `(`.
    Function,
    String,
    Number,
    /// `'Table'`, or a bare word followed by `[`.
    Table,
    /// `[Column]` or `[Measure]`.
    Column,
    Comment,
    Operator,
}

/// One token: its byte range in the text and its class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tok {
    pub range: Range<usize>,
    pub class: Class,
}

/// Bare words of DAX expressions and queries (coloured as keywords when no `(` follows).
pub const KEYWORDS: &[&str] = &[
    "VAR", "RETURN", "IN", "NOT", "ASC", "DESC", "DEFINE", "EVALUATE", "MEASURE", "COLUMN", "TABLE", "ORDER",
    "BY", "START", "AT", "TRUE", "FALSE", "FUNCTION",
];

/// The longest text lexed back from the cursor for its context (larger expressions are read
/// from the start of the cursor's line, so a keystroke never lexes megabytes).
const CONTEXT_BYTES: usize = 64 * 1024;

fn byte(b: &[u8], i: usize) -> u8 {
    b.get(i).copied().unwrap_or(0)
}

fn ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn ident_part(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.'
}

/// The end of the quoted text starting at `i` (`"…"`, `'…'`; a doubled quote stays inside). A
/// quoted table name ends at the end of its line.
fn quoted_end(b: &[u8], i: usize, q: u8) -> usize {
    let mut j = i.saturating_add(1);
    while j < b.len() {
        let c = byte(b, j);
        if c == q {
            if byte(b, j + 1) == q {
                j += 2;
                continue;
            }
            return j + 1;
        }
        if q == b'\'' && c == b'\n' {
            return j;
        }
        j += 1;
    }
    b.len()
}

/// The end of `[…]` starting at `i` (`]]` stays inside; a bracket ends at the end of its line).
fn bracket_end(b: &[u8], i: usize) -> usize {
    let mut j = i.saturating_add(1);
    while j < b.len() {
        match byte(b, j) {
            b']' if byte(b, j + 1) == b']' => j += 2,
            b']' => return j + 1,
            b'\n' => return j,
            _ => j += 1,
        }
    }
    b.len()
}

fn number_end(b: &[u8], i: usize) -> usize {
    let mut j = i;
    while byte(b, j).is_ascii_digit() {
        j += 1;
    }
    if byte(b, j) == b'.' {
        j += 1;
        while byte(b, j).is_ascii_digit() {
            j += 1;
        }
    }
    if matches!(byte(b, j), b'e' | b'E') {
        let k = if matches!(byte(b, j + 1), b'+' | b'-') {
            j + 2
        } else {
            j + 1
        };
        if byte(b, k).is_ascii_digit() {
            j = k;
            while byte(b, j).is_ascii_digit() {
                j += 1;
            }
        }
    }
    j
}

/// The tokens of `text`, in order; white space is left out.
pub fn lex(text: &str) -> Vec<Tok> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let start = i;
        let c = byte(b, i);
        let next = byte(b, i + 1);
        let class = match c {
            b'/' if next == b'/' => {
                i = line_end(b, i);
                Class::Comment
            }
            b'-' if next == b'-' => {
                i = line_end(b, i);
                Class::Comment
            }
            b'/' if next == b'*' => {
                i = text
                    .get(i + 2..)
                    .and_then(|r| r.find("*/"))
                    .map_or(b.len(), |k| i + 2 + k + 2);
                Class::Comment
            }
            b'"' => {
                i = quoted_end(b, i, b'"');
                Class::String
            }
            b'\'' => {
                i = quoted_end(b, i, b'\'');
                Class::Table
            }
            b'[' => {
                i = bracket_end(b, i);
                Class::Column
            }
            b'0'..=b'9' => {
                i = number_end(b, i);
                Class::Number
            }
            b'.' if next.is_ascii_digit() => {
                i = number_end(b, i + 1);
                Class::Number
            }
            b' ' | b'\t' | b'\r' | b'\n' => {
                i += 1;
                continue;
            }
            _ => {
                let rest = text.get(i..).unwrap_or("");
                let mut chars = rest.char_indices();
                let first = chars.next().map_or(' ', |(_, ch)| ch);
                if ident_start(first) {
                    let len = rest
                        .char_indices()
                        .find(|(_, ch)| !ident_part(*ch))
                        .map_or(rest.len(), |(k, _)| k);
                    i += len.max(1);
                    word_class(text, start, i)
                } else {
                    i += first.len_utf8().max(1);
                    if first.is_whitespace() {
                        continue;
                    }
                    Class::Operator
                }
            }
        };
        out.push(Tok {
            range: start..i.min(b.len()),
            class,
        });
        i = i.min(b.len()).max(start + 1);
    }
    out
}

fn line_end(b: &[u8], i: usize) -> usize {
    b.get(i..)
        .and_then(|r| r.iter().position(|c| *c == b'\n'))
        .map_or(b.len(), |k| i + k)
}

/// A word's class from what follows it: `(` a function, `[` a table, else a keyword or a name.
fn word_class(text: &str, start: usize, end: usize) -> Class {
    let after = text.get(end..).unwrap_or("").trim_start_matches([' ', '\t']);
    if after.starts_with('(') {
        return Class::Function;
    }
    if after.starts_with('[') {
        return Class::Table;
    }
    let word = text.get(start..end).unwrap_or("");
    if KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(word)) {
        Class::Keyword
    } else {
        Class::Plain
    }
}

/// `text` cut into pieces that cover it all, each with its class (gaps are [`Class::Plain`]).
pub fn pieces(text: &str) -> Vec<(Range<usize>, Class)> {
    let mut out = Vec::new();
    let mut at = 0;
    for t in lex(text) {
        if t.range.start > at {
            out.push((at..t.range.start, Class::Plain));
        }
        at = t.range.end;
        out.push((t.range, t.class));
    }
    if at < text.len() {
        out.push((at..text.len(), Class::Plain));
    }
    out
}

/// The byte index of character `ch` of `text` (its length past the end).
pub fn byte_of_char(text: &str, ch: usize) -> usize {
    text.char_indices().nth(ch).map_or(text.len(), |(b, _)| b)
}

/// The character index of byte `b` of `text` (rounded down to a character boundary).
pub fn char_of_byte(text: &str, b: usize) -> usize {
    let mut b = b.min(text.len());
    while b > 0 && !text.is_char_boundary(b) {
        b -= 1;
    }
    text.get(..b).map_or(0, |s| s.chars().count())
}

/// The bracket at byte `cursor` (the one just after it, else the one just before) and its
/// partner, as two one-byte ranges: `(`/`)` and `{`/`}` by nesting, a `[name]` its own two
/// ends. Brackets inside strings, comments and quoted names do not count; `None` when there is
/// no bracket at the cursor or it has no partner.
pub fn matching_bracket(text: &str, cursor: usize) -> Option<(Range<usize>, Range<usize>)> {
    let toks = lex(text);
    let one = |t: &Tok| text.get(t.range.clone()).filter(|s| s.len() == 1);
    let is_bracket = |t: &Tok| match t.class {
        Class::Operator => one(t).is_some_and(|s| "(){}".contains(s)),
        Class::Column => t.range.len() >= 2 && text.get(t.range.clone()).is_some_and(|s| s.ends_with(']')),
        _ => false,
    };
    let at = toks
        .iter()
        .position(|t| t.range.start == cursor && is_bracket(t))
        .or_else(|| toks.iter().position(|t| t.range.end == cursor && is_bracket(t)))?;
    let t = toks.get(at)?;
    if t.class == Class::Column {
        let (a, b) = (t.range.start, t.range.end);
        return Some((a..a + 1, b - 1..b));
    }
    let c = one(t)?;
    let (open, close, forward) = match c {
        "(" => ("(", ")", true),
        ")" => ("(", ")", false),
        "{" => ("{", "}", true),
        _ => ("{", "}", false),
    };
    let mut depth = 0usize;
    let mut step = |u: &Tok| -> Option<bool> {
        if u.class != Class::Operator {
            return Some(false);
        }
        let s = one(u)?;
        let (inner, outer) = if forward { (open, close) } else { (close, open) };
        if s == inner {
            depth = depth.saturating_add(1);
        } else if s == outer {
            if depth == 0 {
                return Some(true);
            }
            depth -= 1;
        }
        Some(false)
    };
    let partner = if forward {
        toks.get(at + 1..)?.iter().find(|u| step(u) == Some(true))
    } else {
        toks.get(..at)?.iter().rev().find(|u| step(u) == Some(true))
    }?;
    Some((t.range.clone(), partner.range.clone()))
}

/// Most matches [`find_all`] returns.
pub const MAX_FOUND: usize = 10_000;

/// Where `needle` occurs in `text`, without overlaps, at most [`MAX_FOUND`]; `case` matches
/// letter case exactly, else ASCII letters match in either case (DAX names ignore case).
pub fn find_all(text: &str, needle: &str, case: bool) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let (h, n) = (text.as_bytes(), needle.as_bytes());
    let mut i = 0;
    while i + n.len() <= h.len() && out.len() < MAX_FOUND {
        let same = h
            .get(i..i + n.len())
            .is_some_and(|w| if case { w == n } else { w.eq_ignore_ascii_case(n) });
        if same && text.is_char_boundary(i) && text.is_char_boundary(i + n.len()) {
            out.push(i..i + n.len());
            i += n.len();
        } else {
            i += 1;
        }
    }
    out
}

/// The tokens before byte `cursor` (at most [`CONTEXT_BYTES`] back, from a line start).
fn tokens_before(text: &str, cursor: usize) -> (usize, Vec<Tok>) {
    let mut cursor = cursor.min(text.len());
    while cursor > 0 && !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let mut from = 0;
    if cursor > CONTEXT_BYTES {
        let floor = cursor - CONTEXT_BYTES;
        from = text
            .get(..cursor)
            .and_then(|s| s.get(floor..).map(|r| (floor, r)))
            .and_then(|(f, r)| r.find('\n').map(|k| f + k + 1))
            .unwrap_or(cursor);
        while from < cursor && !text.is_char_boundary(from) {
            from += 1;
        }
    }
    let part = text.get(from..cursor).unwrap_or("");
    let toks = lex(part)
        .into_iter()
        .map(|t| Tok {
            range: t.range.start + from..t.range.end + from,
            class: t.class,
        })
        .collect();
    (from, toks)
}

/// What the word at the cursor names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Want {
    /// A word: functions, keywords, tables, `'Table'[Column]` and `[Measure]`.
    Word(String),
    /// Inside an unclosed `'…`: tables.
    Table(String),
    /// Inside an unclosed `[…`, after the table written before it, if any: its columns and
    /// measures, else the measures (and the home table's columns).
    Bracket { table: Option<String>, prefix: String },
}

/// The completion at the cursor: what it wants and the byte range a suggestion replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub want: Want,
    pub replace: Range<usize>,
}

/// A table name as written in DAX: `Sales` from `'Sales'` or `Sales`.
pub fn unquote_table(t: &str) -> String {
    let t = t.trim();
    match t.strip_prefix('\'') {
        Some(r) => r.strip_suffix('\'').unwrap_or(r).replace("''", "'"),
        None => t.to_string(),
    }
}

/// The completion context at byte `cursor`; `explicit` (Ctrl+Space) also offers words where
/// nothing is typed yet.
pub fn context(text: &str, cursor: usize, explicit: bool) -> Option<Context> {
    let (_, toks) = tokens_before(text, cursor);
    let mut cursor = cursor.min(text.len());
    while cursor > 0 && !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let last = toks.last().filter(|t| t.range.end == cursor);
    let src = |r: &Range<usize>| text.get(r.clone()).unwrap_or("");
    let Some(t) = last else {
        return explicit.then(|| Context {
            want: Want::Word(String::new()),
            replace: cursor..cursor,
        });
    };
    let s = src(&t.range);
    match t.class {
        Class::Table if s.starts_with('\'') => {
            let closed = s.len() >= 2 && s.ends_with('\'') && !s.ends_with("''");
            (!closed).then(|| Context {
                want: Want::Table(s.get(1..).unwrap_or("").replace("''", "'")),
                replace: t.range.clone(),
            })
        }
        Class::Column => {
            if s.ends_with(']') && !s.ends_with("]]") && s.len() >= 2 {
                return None;
            }
            let prev = toks
                .len()
                .checked_sub(2)
                .and_then(|k| toks.get(k))
                .filter(|p| p.range.end == t.range.start && p.class == Class::Table);
            Some(Context {
                want: Want::Bracket {
                    table: prev.map(|p| unquote_table(src(&p.range))),
                    prefix: s.get(1..).unwrap_or("").replace("]]", "]"),
                },
                replace: prev.map_or(t.range.start, |p| p.range.start)..cursor,
            })
        }
        Class::Plain | Class::Keyword | Class::Function | Class::Table => Some(Context {
            want: Want::Word(s.to_string()),
            replace: t.range.clone(),
        }),
        _ if explicit && matches!(t.class, Class::Operator) => Some(Context {
            want: Want::Word(String::new()),
            replace: cursor..cursor,
        }),
        _ => None,
    }
}

/// The call the cursor is in: the function's name (upper case) and the argument it is in
/// (0-based), for the signature help.
pub fn call_at(text: &str, cursor: usize) -> Option<(String, usize)> {
    let (_, toks) = tokens_before(text, cursor);
    let mut depth = 0usize;
    let mut commas = 0usize;
    for (k, t) in toks.iter().enumerate().rev() {
        if t.class != Class::Operator {
            continue;
        }
        match text.get(t.range.clone()).unwrap_or("") {
            ")" => depth = depth.saturating_add(1),
            "(" if depth > 0 => depth -= 1,
            "(" => {
                let f = k.checked_sub(1).and_then(|p| toks.get(p));
                match f {
                    Some(f) if f.class == Class::Function => {
                        let name = text.get(f.range.clone()).unwrap_or("").to_uppercase();
                        return Some((name, commas));
                    }
                    // a parenthesised expression: the call is further out
                    _ => commas = 0,
                }
            }
            "," if depth == 0 => commas = commas.saturating_add(1),
            _ => {}
        }
    }
    None
}

/// One DAX function of the engine's list (`dax.functions`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Func {
    pub name: String,
    pub signature: String,
    /// (name, optional, repeating) of each parameter.
    pub params: Vec<(String, bool, bool)>,
    /// BIcraft's one-sentence description, in English and in French (empty: none).
    pub description: (String, String),
}

/// The model's names the suggestions come from: (table, columns, measures).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Names {
    pub tables: Vec<(String, Vec<String>, Vec<String>)>,
}

/// What a suggestion is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Measure,
    Column,
    Table,
    Function,
    Keyword,
}

/// One suggestion: its label in the list, the text it inserts, what it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub label: String,
    pub insert: String,
    pub kind: Kind,
}

/// `'Sales'` (quotes doubled inside).
pub fn quote_table(t: &str) -> String {
    format!("'{}'", t.replace('\'', "''"))
}

/// `[Amount]` (`]` doubled inside).
pub fn bracket(n: &str) -> String {
    format!("[{}]", n.replace(']', "]]"))
}

/// Most suggestions listed.
pub const MAX_SUGGESTIONS: usize = 200;

/// How well `key` matches what is typed: 0 it starts with it, 1 it contains it.
fn rank(prefix: &str, key: &str) -> Option<u8> {
    let p = prefix.to_lowercase();
    let k = key.to_lowercase();
    if k.starts_with(&p) {
        Some(0)
    } else if !p.is_empty() && k.contains(&p) {
        Some(1)
    } else {
        None
    }
}

/// The suggestions for `want`: names that start with what is typed first, then those that
/// contain it, each group in alphabetical order. `home` is the expression's table: its columns
/// come bare with `[` when `columns_bare` (a calculated column's row context).
pub fn suggestions(
    want: &Want,
    names: &Names,
    funcs: &[Func],
    keywords: &[String],
    home: Option<&str>,
    columns_bare: bool,
) -> Vec<Suggestion> {
    let mut out: Vec<(u8, String, Suggestion)> = Vec::new();
    let mut push = |r: Option<u8>, key: &str, insert: String, kind: Kind| {
        if let Some(r) = r {
            out.push((
                r,
                key.to_lowercase(),
                Suggestion {
                    label: insert.trim_end_matches(['(', ' ']).to_string(),
                    insert,
                    kind,
                },
            ));
        }
    };
    match want {
        Want::Word(prefix) => {
            for f in funcs {
                push(
                    rank(prefix, &f.name),
                    &f.name,
                    format!("{}(", f.name),
                    Kind::Function,
                );
            }
            for k in keywords {
                push(rank(prefix, k), k, format!("{k} "), Kind::Keyword);
            }
            for (t, cols, measures) in &names.tables {
                push(rank(prefix, t), t, quote_table(t), Kind::Table);
                for c in cols {
                    // by the column's name, else by its table's
                    let r = rank(prefix, c).or_else(|| rank(prefix, t).map(|r| r.max(1)));
                    push(r, c, format!("{}{}", quote_table(t), bracket(c)), Kind::Column);
                }
                for m in measures {
                    push(rank(prefix, m), m, bracket(m), Kind::Measure);
                }
            }
        }
        Want::Table(prefix) => {
            for (t, _, _) in &names.tables {
                push(rank(prefix, t), t, quote_table(t), Kind::Table);
            }
        }
        Want::Bracket { table, prefix } => {
            for (t, cols, measures) in &names.tables {
                let this = table.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(t));
                let home_cols =
                    table.is_none() && columns_bare && home.is_some_and(|h| h.eq_ignore_ascii_case(t));
                let lead = if table.is_some() {
                    quote_table(t)
                } else {
                    String::new()
                };
                if this || home_cols {
                    for c in cols {
                        push(rank(prefix, c), c, format!("{lead}{}", bracket(c)), Kind::Column);
                    }
                }
                if table.is_none() || this {
                    for m in measures {
                        push(rank(prefix, m), m, format!("{lead}{}", bracket(m)), Kind::Measure);
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.kind.cmp(&b.2.kind))
    });
    out.dedup_by(|a, b| a.2 == b.2);
    out.into_iter().take(MAX_SUGGESTIONS).map(|x| x.2).collect()
}

/// The parameter of `f` that argument `arg` fills (repeating units counted again and again).
pub fn param_index(f: &Func, arg: usize) -> Option<usize> {
    let Some(first) = f.params.iter().position(|p| p.2) else {
        return (arg < f.params.len()).then_some(arg);
    };
    let unit = f.params.iter().skip(first).take_while(|p| p.2).count().max(1);
    if arg < first + unit {
        return Some(arg);
    }
    Some(first + (arg - first) % unit)
}

/// `Name = expression` cut at its first `=` outside quotes, brackets and strings: (the name,
/// the byte where the expression starts).
pub fn split_definition(text: &str) -> Option<(String, usize)> {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match byte(b, i) {
            b'\'' => i = quoted_end(b, i, b'\''),
            b'"' => i = quoted_end(b, i, b'"'),
            b'[' => i = bracket_end(b, i),
            b'=' => {
                let name = text.get(..i).unwrap_or("").trim();
                let name = name
                    .strip_prefix('[')
                    .and_then(|n| n.strip_suffix(']'))
                    .map(|n| n.replace("]]", "]"))
                    .or_else(|| {
                        name.strip_prefix('\'')
                            .and_then(|n| n.strip_suffix('\''))
                            .map(|n| n.replace("''", "'"))
                    })
                    .unwrap_or_else(|| name.to_string());
                return Some((name, i + 1));
            }
            _ => i += 1,
        }
    }
    None
}

/// `Name = expression` for the bar (a name that needs it in brackets or quotes, as the
/// engine reads it back).
pub fn definition(name: &str, expression: &str) -> String {
    let plain = !name.is_empty() && !name.contains(['=', '[', ']', '\'', '"']) && name.trim() == name;
    let name = if plain { name.to_string() } else { bracket(name) };
    format!("{name} = {expression}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(text: &str) -> Vec<(&str, Class)> {
        lex(text)
            .into_iter()
            .map(|t| (&text[t.range.clone()], t.class))
            .collect()
    }

    /// The partner of the bracket at the cursor skips nested pairs and brackets in strings and
    /// comments; a `[name]` pairs its own ends.
    #[test]
    fn the_bracket_at_the_cursor_finds_its_partner() {
        let text = "CALCULATE(SUM(T[A]), \")\" /* ( */, {1, (2)})";
        let pair = |at: usize| matching_bracket(text, at).map(|(a, b)| (a.start, b.start));
        let last = text.len() - 1;
        // before the outer `(` and after the outer `)`
        assert_eq!(pair(9), Some((9, last)));
        assert_eq!(pair(text.len()), Some((last, 9)));
        // `{` holds a nested pair
        let brace = text.find('{').unwrap();
        assert_eq!(pair(brace), Some((brace, text.rfind('}').unwrap())));
        // a column reference
        let col = text.find('[').unwrap();
        assert_eq!(pair(col), Some((col, col + 2)));
        // nothing at a letter, nothing for an unmatched bracket
        assert_eq!(pair(1), None);
        assert_eq!(matching_bracket("SUM(T[A]", 3), None);
    }

    #[test]
    fn find_ignores_ascii_case_unless_asked() {
        let text = "Sales[a] + SALES[b] + sales";
        assert_eq!(find_all(text, "sales", false).len(), 3);
        assert_eq!(find_all(text, "sales", true), vec![22..27]);
        assert!(find_all(text, "", false).is_empty());
        // no overlaps
        assert_eq!(find_all("aaaa", "aa", false).len(), 2);
    }

    #[test]
    fn the_lexer_colours_every_kind() {
        let text = "VAR x = SUM('Sales Table'[Amount]) // note\nRETURN x * 1.5e3 + \"a\"\"b\" -- end\n/* c */ Sales[Qty] + [Total]";
        let c = classes(text);
        assert_eq!(c[0], ("VAR", Class::Keyword));
        assert_eq!(c[1], ("x", Class::Plain));
        assert_eq!(c[2], ("=", Class::Operator));
        assert_eq!(c[3], ("SUM", Class::Function));
        assert_eq!(c[5], ("'Sales Table'", Class::Table));
        assert_eq!(c[6], ("[Amount]", Class::Column));
        assert_eq!(c[8], ("// note", Class::Comment));
        assert_eq!(c[9], ("RETURN", Class::Keyword));
        assert!(c.contains(&("1.5e3", Class::Number)));
        assert!(c.contains(&("\"a\"\"b\"", Class::String)));
        assert!(c.contains(&("-- end", Class::Comment)));
        assert!(c.contains(&("/* c */", Class::Comment)));
        assert!(c.contains(&("Sales", Class::Table)));
        assert!(c.contains(&("[Total]", Class::Column)));
        // the pieces cover the text
        let p = pieces(text);
        assert_eq!(p.first().unwrap().0.start, 0);
        assert_eq!(p.last().unwrap().0.end, text.len());
        assert!(p.windows(2).all(|w| w[0].0.end == w[1].0.start));
    }

    #[test]
    fn any_text_lexes_without_a_panic() {
        for text in [
            "",
            "'",
            "[",
            "\"",
            "/*",
            "é'ü[ö",
            "1e",
            "1e+",
            ".5",
            "a.b(",
            "]]]",
            "''''",
            "\u{1F600}x",
            "--",
        ] {
            let p = pieces(text);
            assert_eq!(p.iter().map(|x| x.0.len()).sum::<usize>(), text.len(), "{text:?}");
            for c in 0..=text.len() + 2 {
                let _ = context(text, c, true);
                let _ = call_at(text, c);
            }
            let _ = split_definition(text);
        }
    }

    #[test]
    fn the_context_at_the_cursor() {
        let at = |t: &str| context(t, t.len(), false);
        assert_eq!(at("SU").unwrap().want, Want::Word("SU".into()));
        assert_eq!(at("SUM(").map(|c| c.want), None);
        assert_eq!(context("SUM(", 4, true).unwrap().want, Want::Word(String::new()));
        assert_eq!(at("SUM('Sa").unwrap().want, Want::Table("Sa".into()));
        assert_eq!(at("SUM('Sales'").map(|c| c.want), None);
        let c = at("SUM('Sales'[Am").unwrap();
        assert_eq!(
            c.want,
            Want::Bracket {
                table: Some("Sales".into()),
                prefix: "Am".into()
            }
        );
        assert_eq!(c.replace, 4.."SUM('Sales'[Am".len());
        let c = at("1 + [To").unwrap();
        assert_eq!(
            c.want,
            Want::Bracket {
                table: None,
                prefix: "To".into()
            }
        );
        assert_eq!(c.replace, 4..7);
        assert_eq!(at("\"SU").map(|c| c.want), None, "inside a string");
        assert_eq!(at("// SU").map(|c| c.want), None, "inside a comment");
        assert_eq!(at("[Total]").map(|c| c.want), None);
    }

    #[test]
    fn the_call_and_argument_at_the_cursor() {
        let at = |t: &str| call_at(t, t.len());
        assert_eq!(at("DIVIDE("), Some(("DIVIDE".into(), 0)));
        assert_eq!(at("DIVIDE(SUM(x), "), Some(("DIVIDE".into(), 1)));
        assert_eq!(at("DIVIDE(SUM(x, "), Some(("SUM".into(), 1)));
        assert_eq!(at("DIVIDE(1, (2 + 3), \"a,b\", "), Some(("DIVIDE".into(), 3)));
        assert_eq!(at("SUM(x)"), None);
        assert_eq!(at("1 + 2"), None);
        let f = Func {
            name: "SWITCH".into(),
            signature: String::new(),
            params: vec![
                ("expression".into(), false, false),
                ("value".into(), false, true),
                ("result".into(), false, true),
                ("else".into(), true, false),
            ],
            ..Func::default()
        };
        assert_eq!(param_index(&f, 0), Some(0));
        assert_eq!(param_index(&f, 2), Some(2));
        assert_eq!(param_index(&f, 3), Some(1), "the unit again");
        assert_eq!(param_index(&f, 4), Some(2));
    }

    #[test]
    fn suggestions_come_from_the_model_and_the_functions() {
        let names = Names {
            tables: vec![(
                "Sales".into(),
                vec!["Amount".into(), "Category".into()],
                vec!["Total".into()],
            )],
        };
        let funcs = vec![
            Func {
                name: "SUM".into(),
                signature: "SUM(column)".into(),
                params: vec![("column".into(), false, false)],
                ..Func::default()
            },
            Func {
                name: "SUMX".into(),
                signature: "SUMX(table, expression)".into(),
                params: vec![],
                ..Func::default()
            },
        ];
        let kw = vec!["RETURN".to_string()];
        let labels = |w: Want| -> Vec<String> {
            suggestions(&w, &names, &funcs, &kw, Some("Sales"), true)
                .into_iter()
                .map(|s| s.insert)
                .collect()
        };
        assert_eq!(labels(Want::Word("su".into())), ["SUM(", "SUMX("]);
        assert_eq!(labels(Want::Word("am".into())), ["'Sales'[Amount]"]);
        assert_eq!(labels(Want::Word("re".into())), ["RETURN "]);
        assert!(labels(Want::Word("sa".into())).contains(&"'Sales'".to_string()));
        assert_eq!(labels(Want::Word("tot".into())), ["[Total]"]);
        assert_eq!(
            labels(Want::Bracket {
                table: Some("sales".into()),
                prefix: "c".into()
            }),
            ["'Sales'[Category]"]
        );
        assert_eq!(
            labels(Want::Bracket {
                table: None,
                prefix: String::new()
            }),
            ["[Amount]", "[Category]", "[Total]"]
        );
        // contains after starts-with
        assert_eq!(labels(Want::Word("ou".into())), ["'Sales'[Amount]"]);
    }

    #[test]
    fn definitions_split_at_their_first_equals_sign() {
        assert_eq!(split_definition("Total = SUM(x)"), Some(("Total".into(), 7)));
        assert_eq!(split_definition("[A = B] = 1 = 1"), Some(("A = B".into(), 9)));
        assert_eq!(split_definition("'It''s' = 1"), Some(("It's".into(), 9)));
        assert_eq!(split_definition("no equals"), None);
        assert_eq!(definition("Total", "1"), "Total = 1");
        assert_eq!(definition("A = B", "1"), "[A = B] = 1");
        let d = definition("x]y", "2");
        let (name, at) = split_definition(&d).unwrap();
        assert_eq!((name.as_str(), d[at..].trim()), ("x]y", "2"));
    }
}
