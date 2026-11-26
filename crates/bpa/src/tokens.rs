//! `Tokenize()`: the DAX tokens of an object's expression, typed the way BPA rules test them
//! (`Type = DIV and Next.Type <> INTEGER_LITERAL`). Built on `bicraft_dax`'s lexer; the type
//! names are listed in `docs/formats/bpa.md`.

use bicraft_dax::TokenKind;

use crate::objects::ModelView;

/// Most tokens kept for one object (the rest are dropped with an error).
pub(crate) const MAX_TOKENS: usize = 200_000;

/// One token: its type name and its text as written.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DaxToken {
    pub(crate) ty: &'static str,
    pub(crate) text: String,
}

/// Every token type name `Tokenize()` produces, plus the two ambiguous names that compare equal
/// to their resolved forms ([`same_type`]). Bare names written exactly like this in a rule are
/// token-type constants.
pub(crate) const TYPE_NAMES: &[&str] = &[
    "TABLE",
    "VARIABLE",
    "TABLE_OR_VARIABLE",
    "COLUMN",
    "MEASURE",
    "COLUMN_OR_MEASURE",
    "FUNCTION",
    "STRING_LITERAL",
    "INTEGER_LITERAL",
    "REAL_LITERAL",
    "VAR",
    "RETURN",
    "IN",
    "NOT",
    "TRUE",
    "FALSE",
    "ASC",
    "DESC",
    "PLUS",
    "MINUS",
    "MULT",
    "DIV",
    "EXP",
    "CONCAT",
    "EQUALS",
    "STRICT_EQUALS",
    "NOT_EQUALS",
    "LESS_THAN",
    "LESS_THAN_OR_EQUAL",
    "GREATER_THAN",
    "GREATER_THAN_OR_EQUAL",
    "AND",
    "OR",
    "OPEN_PARENS",
    "CLOSE_PARENS",
    "OPEN_CURLY",
    "CLOSE_CURLY",
    "COMMA",
    "DOT",
    "OTHER",
];

/// Keywords that are their own token type when not called like a function.
const KEYWORDS: &[&str] = &["VAR", "RETURN", "IN", "NOT", "TRUE", "FALSE", "ASC", "DESC"];

pub(crate) fn is_type_name(name: &str) -> bool {
    TYPE_NAMES.contains(&name)
}

/// Whether two token type names match: equal (ignoring case), or an ambiguous name
/// (`COLUMN_OR_MEASURE`, `TABLE_OR_VARIABLE`) against one of its resolved forms.
pub(crate) fn same_type(a: &str, b: &str) -> bool {
    if a.eq_ignore_ascii_case(b) {
        return true;
    }
    let pair = |x: &str, y: &str| {
        (x.eq_ignore_ascii_case("COLUMN_OR_MEASURE")
            && (y.eq_ignore_ascii_case("COLUMN") || y.eq_ignore_ascii_case("MEASURE")))
            || (x.eq_ignore_ascii_case("TABLE_OR_VARIABLE")
                && (y.eq_ignore_ascii_case("TABLE") || y.eq_ignore_ascii_case("VARIABLE")))
    };
    pair(a, b) || pair(b, a)
}

fn op_type(op: &str) -> &'static str {
    match op {
        "+" => "PLUS",
        "-" => "MINUS",
        "*" => "MULT",
        "/" => "DIV",
        "^" => "EXP",
        "&" => "CONCAT",
        "=" => "EQUALS",
        "==" => "STRICT_EQUALS",
        "<>" => "NOT_EQUALS",
        "<" => "LESS_THAN",
        "<=" => "LESS_THAN_OR_EQUAL",
        ">" => "GREATER_THAN",
        ">=" => "GREATER_THAN_OR_EQUAL",
        "&&" => "AND",
        "||" => "OR",
        "." => "DOT",
        _ => "OTHER",
    }
}

/// The tokens of `text`, a DAX expression whose home table is `home`.
pub(crate) fn tokenize(
    view: &ModelView<'_>,
    text: &str,
    home: Option<&str>,
) -> Result<Vec<DaxToken>, String> {
    let toks = bicraft_dax::tokenize(text);
    if toks.len() > MAX_TOKENS {
        return Err(format!("the expression has more than {MAX_TOKENS} DAX tokens"));
    }
    // Variables declared anywhere in the expression (`VAR name = …`).
    let mut vars: Vec<String> = Vec::new();
    for w in toks.windows(2) {
        if let [a, b] = w
            && let (TokenKind::Ident(k), TokenKind::Ident(n)) = (&a.kind, &b.kind)
            && k.eq_ignore_ascii_case("VAR")
        {
            vars.push(n.to_lowercase());
        }
    }
    let is_table = |name: &str| view.table_index(name).is_some();
    let mut out = Vec::with_capacity(toks.len());
    for (i, t) in toks.iter().enumerate() {
        let prev = i.checked_sub(1).and_then(|p| toks.get(p)).map(|p| &p.kind);
        let next = toks.get(i.saturating_add(1)).map(|n| &n.kind);
        let ty: &'static str = match &t.kind {
            TokenKind::Eof => continue,
            TokenKind::Ident(n) => {
                let upper = n.to_ascii_uppercase();
                if matches!(next, Some(TokenKind::LParen)) {
                    "FUNCTION"
                } else if let Some(k) = KEYWORDS.iter().find(|k| **k == upper) {
                    k
                } else if vars.contains(&n.to_lowercase()) {
                    "VARIABLE"
                } else if is_table(n) {
                    "TABLE"
                } else {
                    "TABLE_OR_VARIABLE"
                }
            }
            TokenKind::QuotedTable(_) => "TABLE",
            TokenKind::Bracket(n) => {
                let qualified = matches!(prev, Some(TokenKind::QuotedTable(_) | TokenKind::Op(".")))
                    || matches!(prev, Some(TokenKind::Ident(p)) if !matches!(
                        p.to_ascii_uppercase().as_str(),
                        "VAR" | "RETURN" | "IN" | "NOT" | "ASC" | "DESC" | "TRUE" | "FALSE"
                    ));
                if qualified {
                    "COLUMN"
                } else if view.measure_obj(n).is_some() {
                    "MEASURE"
                } else if home.is_some_and(|h| view.column_obj(h, n).is_some()) {
                    "COLUMN"
                } else {
                    "COLUMN_OR_MEASURE"
                }
            }
            TokenKind::Str(_) => "STRING_LITERAL",
            TokenKind::Number(s) => {
                if s.contains(['.', 'e', 'E']) {
                    "REAL_LITERAL"
                } else {
                    "INTEGER_LITERAL"
                }
            }
            TokenKind::Param(_) => "OTHER",
            TokenKind::Op(o) => op_type(o),
            TokenKind::LParen => "OPEN_PARENS",
            TokenKind::RParen => "CLOSE_PARENS",
            TokenKind::LBrace => "OPEN_CURLY",
            TokenKind::RBrace => "CLOSE_CURLY",
            TokenKind::Comma => "COMMA",
        };
        let text = text.get(t.span.start..t.span.end).unwrap_or_default().to_string();
        out.push(DaxToken { ty, text });
    }
    Ok(out)
}
