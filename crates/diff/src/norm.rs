//! Expression normalisation: is a DAX or M change only cosmetic (whitespace, comments, the case
//! of DAX names and keywords, CRLF)?

use std::borrow::Cow;

use bicraft_dax::TokenKind;

/// Longest expression normalised; longer ones are compared as text (after CRLF folding).
pub const MAX_NORMALISED_BYTES: usize = 4 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Same {
    /// The same text (line endings aside).
    Identical,
    /// Only whitespace, comments or (DAX) name and keyword case differ.
    Cosmetic,
    Different,
}

/// The text with CRLF and lone CR turned into LF.
pub fn lf(s: &str) -> Cow<'_, str> {
    if s.contains('\r') {
        Cow::Owned(s.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(s)
    }
}

fn text_same(a: &str, b: &str) -> bool {
    a == b || lf(a) == lf(b)
}

/// Compares two DAX expressions token by token (comments dropped, names and keywords
/// case-insensitive, `'Sales'` = `Sales`).
pub fn dax(a: &str, b: &str) -> Same {
    if text_same(a, b) {
        return Same::Identical;
    }
    if a.len() > MAX_NORMALISED_BYTES || b.len() > MAX_NORMALISED_BYTES {
        return Same::Different;
    }
    let ta = bicraft_dax::tokenize(a);
    let tb = bicraft_dax::tokenize(b);
    if ta.len() == tb.len() && ta.iter().zip(&tb).all(|(x, y)| token_eq(&x.kind, &y.kind)) {
        Same::Cosmetic
    } else {
        Same::Different
    }
}

fn ci_eq(a: &str, b: &str) -> bool {
    a.chars()
        .flat_map(char::to_lowercase)
        .eq(b.chars().flat_map(char::to_lowercase))
}

fn token_eq(a: &TokenKind, b: &TokenKind) -> bool {
    use TokenKind::*;
    match (a, b) {
        (Ident(x) | QuotedTable(x), Ident(y) | QuotedTable(y)) | (Bracket(x), Bracket(y)) => ci_eq(x, y),
        _ => a == b,
    }
}

/// A key equal for cosmetically equal DAX expressions (for content matching); empty for an
/// expression too long to normalise.
pub fn dax_key(text: &str) -> String {
    if text.len() > MAX_NORMALISED_BYTES {
        return String::new();
    }
    let mut out = String::new();
    for t in bicraft_dax::tokenize(text) {
        use TokenKind::*;
        let part = match &t.kind {
            Ident(s) | QuotedTable(s) => format!("i:{}", s.to_lowercase()),
            Bracket(s) => format!("b:{}", s.to_lowercase()),
            Str(s) => format!("s:{s}"),
            Number(s) => format!("n:{s}"),
            Param(s) => format!("p:{s}"),
            Op(s) => format!("o:{s}"),
            LParen => "(".into(),
            RParen => ")".into(),
            LBrace => "{".into(),
            RBrace => "}".into(),
            Comma => ",".into(),
            Eof => continue,
        };
        out.push_str(&part);
        out.push('\u{1}');
    }
    out
}

/// Compares two M (Power Query) expressions with whitespace and comments outside strings
/// dropped (M is case-sensitive).
pub fn m(a: &str, b: &str) -> Same {
    if text_same(a, b) {
        return Same::Identical;
    }
    if a.len() > MAX_NORMALISED_BYTES || b.len() > MAX_NORMALISED_BYTES {
        return Same::Different;
    }
    if m_key(a) == m_key(b) {
        Same::Cosmetic
    } else {
        Same::Different
    }
}

fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.'
}

/// The M text without comments, with whitespace kept only where it separates two words.
pub fn m_key(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_NORMALISED_BYTES));
    let mut chars = text.chars().peekable();
    let mut gap = false;
    let push = |out: &mut String, c: char, gap: &mut bool| {
        if *gap && out.chars().last().is_some_and(word) && word(c) {
            out.push(' ');
        }
        *gap = false;
        out.push(c);
    };
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                push(&mut out, c, &mut gap);
                // a string (or #"quoted identifier"): `""` escapes a quote
                while let Some(s) = chars.next() {
                    out.push(s);
                    if s == '"' {
                        if chars.peek() == Some(&'"') {
                            if let Some(q) = chars.next() {
                                out.push(q);
                            }
                        } else {
                            break;
                        }
                    }
                }
            }
            '/' if chars.peek() == Some(&'/') => {
                for s in chars.by_ref() {
                    if s == '\n' {
                        break;
                    }
                }
                gap = true;
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut star = false;
                for s in chars.by_ref() {
                    if star && s == '/' {
                        break;
                    }
                    star = s == '*';
                }
                gap = true;
            }
            c if c.is_whitespace() => gap = true,
            c => push(&mut out, c, &mut gap),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dax_cosmetic_changes() {
        assert_eq!(dax("SUM(Sales[Amount])", "SUM(Sales[Amount])"), Same::Identical);
        assert_eq!(
            dax("SUM(Sales[Amount])\r\n", "SUM(Sales[Amount])\n"),
            Same::Identical
        );
        assert_eq!(
            dax("SUM(Sales[Amount])", "sum ( 'Sales'[amount] ) // total"),
            Same::Cosmetic
        );
        assert_eq!(
            dax("SUM(Sales[Amount])", "/* x */ SUM(Sales[Amount])"),
            Same::Cosmetic
        );
        assert_eq!(dax("SUM(Sales[Amount])", "SUM(Sales[Qty])"), Same::Different);
        assert_eq!(dax("\"a\"", "\"A\""), Same::Different);
        assert_eq!(dax_key("SUM ( Sales[Amount] )"), dax_key("sum(Sales[AMOUNT])"));
    }

    #[test]
    fn m_cosmetic_changes() {
        let a = "let\n    Source = #\"Sales  Data\",\n    x = 1 // one\nin\n    x";
        let b = "let Source = #\"Sales  Data\", /* c */ x = 1 in x";
        assert_eq!(m(a, b), Same::Cosmetic);
        assert_eq!(m(a, &b.replace("Sales  Data", "Sales Data")), Same::Different);
        assert_eq!(m("each _", "each_"), Same::Different);
        assert_eq!(m("\"a\"\"b\" ", "\"a\"\"b\""), Same::Cosmetic);
        // unterminated strings and comments end the text
        assert_eq!(m_key("\"abc"), "\"abc");
        assert_eq!(m_key("a /* b"), "a");
    }
}
