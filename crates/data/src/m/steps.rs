//! The applied steps of a Power Query (M) query: the variables of its top-level `let`, as Power
//! Query's editor lists them, and text edits of one step that keep the rest of the user's text
//! byte for byte (comments, line breaks, indentation).
//!
//! A query `let A = …, B = …(A…) in B` has the steps `A` and `B`; its result is the `in`
//! expression, usually the last step's name. A query that is not a `let` (one expression) reads as
//! one step named `Source`; the first edit wraps it as `let Source = <text> in Source`.
//!
//! The split works on the lexer's tokens (with their byte ranges), so a step's text is exactly
//! what the user wrote between `=` and the `,` (or `in`) that ends it. Every edit is re-parsed:
//! an edit whose result does not parse is refused. References to a step are the identifier
//! references the parser records ([`super::parser::parse_with_idents`]); a nested function
//! parameter or `let` variable with the same name as a step is rewritten too (rare: Desktop's
//! step names are not parameter names).

use std::ops::Range;

use super::lexer::{Tok, Token, tokenize};
use super::parser::{parse, parse_with_idents};
use super::transform::ident;
use crate::Limits;

/// The step a query that is not a `let` reads as.
pub const SOURCE: &str = "Source";
/// Most steps one query lists.
pub const MAX_STEPS: usize = 2_000;
/// Longest step name, in characters.
pub const MAX_NAME: usize = 256;

/// One applied step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The variable's name (a `#"quoted"` name unquoted).
    pub name: String,
    /// The step's M, as written.
    pub expression: String,
    /// The function the step calls first (`Table.SelectRows`), when it starts with a call.
    pub function: Option<String>,
    /// Byte range of the name in the query (`#"…"` included).
    pub name_span: Range<usize>,
    /// Byte range of the expression in the query.
    pub expr_span: Range<usize>,
    /// Where the `,` (or the `in`) after the step starts: a comment written between the
    /// expression and it stays with the step.
    pub end: usize,
}

/// The steps of a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Steps {
    /// Whether the query is a `let` (else it is one expression, one step `Source`).
    pub is_let: bool,
    pub steps: Vec<Step>,
    /// The `in` expression, as written.
    pub result: String,
    pub result_span: Range<usize>,
    /// The step the result names, when the result is one step's name.
    pub result_step: Option<usize>,
}

impl Steps {
    /// The index of step `name` (exact, else ignoring case when only one step matches).
    pub fn find(&self, name: &str) -> Option<usize> {
        if let Some(i) = self.steps.iter().position(|s| s.name == name) {
            return Some(i);
        }
        let mut it = self
            .steps
            .iter()
            .enumerate()
            .filter(|(_, s)| s.name.eq_ignore_ascii_case(name));
        match (it.next(), it.next()) {
            (Some((i, _)), None) => Some(i),
            _ => None,
        }
    }

    fn get(&self, name: &str) -> Result<(usize, &Step), String> {
        let i = self.find(name).ok_or_else(|| {
            format!(
                "the query has no step `{name}` (its steps: {})",
                self.steps
                    .iter()
                    .take(20)
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
        let s = self.steps.get(i).ok_or("internal: step index out of range")?;
        Ok((i, s))
    }

    /// The last step shown: the one the result names, else the last one.
    pub fn last(&self) -> Option<&Step> {
        self.result_step
            .and_then(|i| self.steps.get(i))
            .or_else(|| self.steps.last())
    }

    /// `base`, else `base1`, `base2`… the first name no step has (ignoring case), as Power Query
    /// names a second "Changed Type" step "Changed Type1".
    pub fn free_name(&self, base: &str) -> String {
        let base = if base.trim().is_empty() {
            "Custom"
        } else {
            base.trim()
        };
        let taken = |n: &str| self.steps.iter().any(|s| s.name.eq_ignore_ascii_case(n));
        if !taken(base) {
            return base.to_string();
        }
        (1..100_000)
            .map(|i| format!("{base}{i}"))
            .find(|n| !taken(n))
            .unwrap_or_else(|| format!("{base}_"))
    }
}

fn syntax(e: crate::Error) -> String {
    e.to_string()
}

fn is_kw(t: &Tok, kw: &str) -> bool {
    matches!(t, Tok::Ident(s) if s == kw)
}

fn tok(toks: &[Token], i: usize) -> Result<&Token, String> {
    toks.get(i)
        .ok_or_else(|| "internal: token index out of range".to_string())
}

/// The steps of query `src`.
pub fn steps(src: &str, limits: &Limits) -> Result<Steps, String> {
    limits.check_bytes(src.len(), "M expression").map_err(syntax)?;
    // the whole text must be M
    parse(src, limits).map_err(syntax)?;
    let toks = tokenize(src, limits).map_err(syntax)?;
    let first = tok(&toks, 0)?;
    if !is_kw(&first.tok, "let") {
        let last_end = toks
            .iter()
            .rev()
            .find(|t| t.tok != Tok::Eof)
            .map_or(first.start, |t| t.end);
        let span = first.start..last_end;
        let text = src.get(span.clone()).unwrap_or_default().to_string();
        return Ok(Steps {
            is_let: false,
            steps: vec![Step {
                name: SOURCE.into(),
                function: leading_call(&toks, 0),
                expression: text.clone(),
                name_span: span.start..span.start,
                end: span.end,
                expr_span: span.clone(),
            }],
            result: text,
            result_span: span,
            result_step: Some(0),
        });
    }
    let mut steps = Vec::new();
    let mut i = 1usize;
    loop {
        // a binding: name `=` expression
        let n = tok(&toks, i)?;
        let name = match &n.tok {
            Tok::Ident(s) => s.clone(),
            Tok::Quoted(s) => s.clone(),
            _ => return Err("this `let` does not read as steps (a variable name was expected)".into()),
        };
        let eq = tok(&toks, i + 1)?;
        if eq.tok != Tok::Punct("=") {
            return Err(format!("step `{name}`: `=` was expected after its name"));
        }
        let start = i + 2;
        let (end, stop) = binding_end(&toks, start)?;
        let first_tok = tok(&toks, start)?;
        let last_tok = tok(&toks, end.checked_sub(1).ok_or("internal: empty step")?)?;
        if end <= start {
            return Err(format!("step `{name}` has no expression"));
        }
        let expr_span = first_tok.start..last_tok.end;
        steps.push(Step {
            name,
            expression: src.get(expr_span.clone()).unwrap_or_default().to_string(),
            function: leading_call(&toks, start),
            name_span: n.start..n.end,
            expr_span,
            end: tok(&toks, end)?.start,
        });
        if steps.len() > MAX_STEPS {
            return Err(format!("the query has more than {MAX_STEPS} steps"));
        }
        if stop {
            // `in`: the result runs to the end
            let r0 = tok(&toks, end + 1)?;
            let last_end = toks
                .iter()
                .rev()
                .find(|t| t.tok != Tok::Eof)
                .map_or(r0.start, |t| t.end);
            if r0.tok == Tok::Eof {
                return Err("the query's `in` has no result".into());
            }
            let result_span = r0.start..last_end;
            let result_step = match (&r0.tok, toks.get(end + 2).map(|t| &t.tok)) {
                (Tok::Ident(x) | Tok::Quoted(x), Some(Tok::Eof)) => {
                    steps.iter().rposition(|st| &st.name == x)
                }
                _ => None,
            };
            return Ok(Steps {
                is_let: true,
                steps,
                result: src.get(result_span.clone()).unwrap_or_default().to_string(),
                result_span,
                result_step,
            });
        }
        i = end + 1;
    }
}

/// The index of the token that ends the binding starting at `start` (a `,` or the `in` of the
/// top-level `let`), and whether it is the `in`.
fn binding_end(toks: &[Token], start: usize) -> Result<(usize, bool), String> {
    let mut brackets = 0usize;
    let mut lets = 0usize;
    for (i, t) in toks.iter().enumerate().skip(start) {
        match &t.tok {
            Tok::Punct("(" | "[" | "{") => brackets = brackets.saturating_add(1),
            Tok::Punct(")" | "]" | "}") => brackets = brackets.saturating_sub(1),
            Tok::Ident(s) if s == "let" => lets = lets.saturating_add(1),
            Tok::Ident(s) if s == "in" => {
                if lets > 0 {
                    lets -= 1;
                } else if brackets == 0 {
                    return Ok((i, true));
                }
            }
            Tok::Punct(",") if brackets == 0 && lets == 0 => return Ok((i, false)),
            Tok::Eof => return Err("the query's `let` has no `in`".into()),
            _ => {}
        }
    }
    Err("the query's `let` has no `in`".into())
}

/// `Table.SelectRows` for a step that starts `Table.SelectRows(`.
fn leading_call(toks: &[Token], start: usize) -> Option<String> {
    match (
        toks.get(start).map(|t| &t.tok),
        toks.get(start + 1).map(|t| &t.tok),
    ) {
        (Some(Tok::Ident(f)), Some(Tok::Punct("("))) if f.contains('.') => Some(f.clone()),
        _ => None,
    }
}

/// The line break the query uses (`\r\n` when it has one).
fn newline(src: &str) -> &'static str {
    if src.contains("\r\n") { "\r\n" } else { "\n" }
}

/// The spaces and tabs before byte `at` on its line.
fn indent_before(src: &str, at: usize) -> String {
    let head = src.get(..at).unwrap_or_default();
    let line = head.rsplit('\n').next().unwrap_or_default();
    line.chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// Checks a step name.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("a step name cannot be empty".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("a step name has at most {MAX_NAME} characters"));
    }
    if name.chars().any(char::is_control) {
        return Err("a step name cannot hold control characters".into());
    }
    Ok(())
}

/// Text edits applied from the end (they must not overlap).
fn splice(src: &str, mut edits: Vec<(Range<usize>, String)>) -> Result<String, String> {
    edits.sort_by_key(|e| std::cmp::Reverse(e.0.start));
    let mut out = src.to_string();
    let mut floor = usize::MAX;
    for (r, text) in edits {
        if r.end > floor || r.start > r.end || r.end > out.len() {
            return Err("internal: overlapping edits".into());
        }
        if !out.is_char_boundary(r.start) || !out.is_char_boundary(r.end) {
            return Err("internal: edit inside a character".into());
        }
        out.replace_range(r.clone(), &text);
        floor = r.start;
    }
    Ok(out)
}

/// The references to step `name` inside `within` (byte range of the query).
fn references(
    src: &str,
    name: &str,
    within: &Range<usize>,
    limits: &Limits,
) -> Result<Vec<Range<usize>>, String> {
    let (_, idents) = parse_with_idents(src, limits).map_err(syntax)?;
    Ok(idents
        .into_iter()
        .filter(|r| &*r.name == name && r.start >= within.start && r.end <= within.end)
        .map(|r| r.start..r.end)
        .collect())
}

/// The query as a `let` (a one-expression query becomes `let Source = <text> in Source`).
pub fn as_let(src: &str, limits: &Limits) -> Result<String, String> {
    let s = steps(src, limits)?;
    if s.is_let {
        return Ok(src.to_string());
    }
    let nl = newline(src);
    let text = s.steps.first().map_or("", |x| x.expression.as_str());
    Ok(format!("let{nl}    {SOURCE} = {text}{nl}in{nl}    {SOURCE}"))
}

/// Where step `st`'s text ends: its expression and the comments after it, before the `,` or
/// `in` (and the white space before that).
fn tail(src: &str, st: &Step) -> usize {
    src.get(..st.end)
        .map_or(st.expr_span.end, |h| h.trim_end().len())
        .max(st.expr_span.end)
}

fn checked(out: String, limits: &Limits) -> Result<String, String> {
    parse(&out, limits).map_err(|e| format!("the edited query does not parse: {e}"))?;
    Ok(out)
}

/// The query evaluated at step `name`: the steps up to it, and `in <name>`.
pub fn at_step(src: &str, name: &str, limits: &Limits) -> Result<String, String> {
    let s = steps(src, limits)?;
    let (_, st) = s.get(name)?;
    if !s.is_let {
        return Ok(src.to_string());
    }
    let nl = newline(src);
    let head = src.get(..tail(src, st)).ok_or("internal: step span")?;
    checked(format!("{head}{nl}in{nl}    {}", ident(&st.name)), limits)
}

/// Inserts step `name = expression` after step `after` (default: the last step shown). The new
/// step takes the place of `after` where the next step (or the result) used it, so it applies
/// to what follows. `expression` should read `after` (the transforms of [`super::transform`] do).
pub fn insert_step(
    src: &str,
    after: Option<&str>,
    name: &str,
    expression: &str,
    limits: &Limits,
) -> Result<String, String> {
    check_name(name)?;
    if expression.trim().is_empty() {
        return Err("the new step has no expression".into());
    }
    let src = as_let(src, limits)?;
    let s = steps(&src, limits)?;
    let (i, prev) = match after {
        Some(a) => s.get(a)?,
        None => {
            let i = match s.result_step {
                Some(i) => i,
                None => s.steps.len().checked_sub(1).ok_or("the query has no steps")?,
            };
            (i, s.steps.get(i).ok_or("internal: step index out of range")?)
        }
    };
    if s.steps.iter().any(|x| x.name == name) {
        return Err(format!("the query already has a step `{name}`"));
    }
    let nl = newline(&src);
    let indent = indent_before(&src, prev.name_span.start);
    let new_ref = ident(name);
    let at = tail(&src, prev);
    let mut edits = vec![(at..at, format!(",{nl}{indent}{new_ref} = {}", expression.trim()))];
    // what used `after` next now uses the new step
    let next_span = match s.steps.get(i + 1) {
        Some(n) => Some(n.expr_span.clone()),
        None if s.result_step == Some(i) || s.result_step.is_none() => Some(s.result_span.clone()),
        None => None,
    };
    if let Some(span) = next_span {
        for r in references(&src, &prev.name, &span, limits)? {
            edits.push((r, new_ref.clone()));
        }
    }
    checked(splice(&src, edits)?, limits)
}

/// Removes step `name`. What used it reads the step before it instead (the first step cannot be
/// removed while another step uses it); the result names the step before when it named this one.
pub fn remove_step(src: &str, name: &str, limits: &Limits) -> Result<String, String> {
    let s = steps(src, limits)?;
    if !s.is_let || s.steps.len() < 2 {
        return Err("a query keeps at least one step".into());
    }
    let (i, st) = s.get(name)?;
    let after = st.expr_span.end..src.len();
    let users = references(src, &st.name, &after, limits)?;
    let mut edits = Vec::new();
    match i.checked_sub(1).and_then(|p| s.steps.get(p)) {
        Some(prev) => {
            let r = ident(&prev.name);
            for u in users {
                edits.push((u, r.clone()));
            }
            // `, name = expression` goes, from the end of the step before
            edits.push((tail(src, prev)..tail(src, st), String::new()));
        }
        None => {
            if !users.is_empty() {
                return Err(format!(
                    "step `{}` is the first step and the next steps use it: edit them first",
                    st.name
                ));
            }
            let next = s.steps.get(1).ok_or("internal: step index out of range")?;
            edits.push((st.name_span.start..next.name_span.start, String::new()));
        }
    }
    checked(splice(src, edits)?, limits)
}

/// Renames step `name` to `new_name`, and the references to it.
pub fn rename_step(src: &str, name: &str, new_name: &str, limits: &Limits) -> Result<String, String> {
    check_name(new_name)?;
    let src = as_let(src, limits)?;
    let s = steps(&src, limits)?;
    let (_, st) = s.get(name)?;
    if st.name == new_name {
        return Ok(src);
    }
    if s.steps.iter().any(|x| x.name == new_name) {
        return Err(format!("the query already has a step `{new_name}`"));
    }
    let new_ref = ident(new_name);
    let after = st.expr_span.end..src.len();
    let mut edits = vec![(st.name_span.clone(), new_ref.clone())];
    for u in references(&src, &st.name, &after, limits)? {
        edits.push((u, new_ref.clone()));
    }
    checked(splice(&src, edits)?, limits)
}

/// Replaces step `name`'s expression (a leading `=`, as the formula bar shows it, is dropped).
pub fn set_step(src: &str, name: &str, expression: &str, limits: &Limits) -> Result<String, String> {
    let text = expression.trim();
    let text = text.strip_prefix('=').map_or(text, str::trim_start);
    if text.is_empty() {
        return Err("the step's expression cannot be empty".into());
    }
    let s = steps(src, limits)?;
    let (_, st) = s.get(name)?;
    if st.expression == text {
        return Ok(src.to_string());
    }
    checked(
        splice(src, vec![(st.expr_span.clone(), text.to_string())])?,
        limits,
    )
}
