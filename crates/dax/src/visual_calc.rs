//! Visual calculations: DAX expressions defined on a visual and evaluated over its visual matrix
//! (Microsoft Learn, "Visual calculations overview"; see `docs/formats/visual-calculations.md`).
//!
//! A visual calculation refers only to the visual's own fields, by name in brackets
//! (`[Sales Amount]`), and to the visual's structure through axis keywords (`ROWS`, `COLUMNS`,
//! `ROWPAGES` and their combinations such as `ROWS COLUMNS`) and reset keywords (`NONE`,
//! `HIGHESTPARENT`, `LOWESTPARENT`). This module parses such an expression (the combined axis
//! keywords become one name, `ROWS COLUMNS`) and binds it against the visual's field names:
//!
//! - `Table[Column]`, `'Table'` and other model references are errors (`model-reference`): a model
//!   field must be added to the visual first;
//! - a bracketed name that is no field of the visual is an error (`unknown-visual-field`), the
//!   calculation's own name too (`self-reference`);
//! - unknown functions, wrong argument counts and the functions that need model relationships
//!   (`RELATED`, `RELATEDTABLE`, `USERELATIONSHIP`) are errors;
//! - a bare name is a variable in scope or one of the keywords above (or a keyword argument such
//!   as `DESC`, `KEEP`, `SKIP`).

use std::collections::HashSet;

use crate::ast::Expr;
use crate::lexer::{self, Token, TokenKind};
use crate::{Diagnostic, Raw, Severity, Span, parser};

/// The axis keywords: one axis, or two or three combined, in traversal order.
pub const AXES: [&str; 3] = ["ROWS", "COLUMNS", "ROWPAGES"];

/// Reset keywords (`NONE` is 0, `HIGHESTPARENT` 1, `LOWESTPARENT` -1).
pub const RESETS: [&str; 3] = ["NONE", "HIGHESTPARENT", "LOWESTPARENT"];

/// Other bare keywords window functions take as arguments.
const KEYWORDS: [&str; 12] = [
    "ASC", "DESC", "DEFAULT", "FIRST", "LAST", "KEEP", "SKIP", "DENSE", "ABS", "REL", "EXPLICIT", "INFERRED",
];

/// Functions that need the model's relationships, which visual calculations do not have.
const MODEL_ONLY: [&str; 4] = ["RELATED", "RELATEDTABLE", "USERELATIONSHIP", "CROSSFILTER"];

/// Whether `name` is an axis keyword: `ROWS`, `COLUMNS`, `ROWPAGES` or a combination of distinct
/// ones separated by spaces (`ROWS COLUMNS`). Case-insensitive.
pub fn is_axis(name: &str) -> bool {
    let words: Vec<&str> = name.split_whitespace().collect();
    if words.is_empty() || words.len() > AXES.len() {
        return false;
    }
    let mut seen = HashSet::new();
    words
        .iter()
        .all(|w| AXES.iter().any(|a| a.eq_ignore_ascii_case(w)) && seen.insert(w.to_ascii_uppercase()))
}

/// The result of [`check`].
#[derive(Debug, Clone)]
pub struct Check {
    /// Parsed without errors and every name bound.
    pub valid: bool,
    pub diagnostics: Vec<Diagnostic>,
    /// The tree (when it parses), axis combinations merged into one [`Expr::Name`].
    pub expr: Option<Expr>,
    /// The visual's fields the expression refers to, spelled as in `fields`, without repeats.
    pub refs: Vec<String>,
}

/// The tokens of `text` with adjacent axis keywords merged (`ROWS COLUMNS` is one name).
fn tokens(text: &str, raw: &mut Vec<Raw>) -> Vec<Token> {
    let toks = lexer::lex(text, raw);
    let mut out: Vec<Token> = Vec::with_capacity(toks.len());
    for t in toks {
        if let TokenKind::Ident(word) = &t.kind
            && AXES.iter().any(|a| a.eq_ignore_ascii_case(word))
            && let Some(Token {
                kind: TokenKind::Ident(prev),
                span,
            }) = out.last_mut()
            && is_axis(prev)
            && is_axis(&format!("{prev} {word}"))
            // only an axis argument: the next token is not `(` (a function) — checked by the
            // parser, which then reports the merged name as unknown
            && text.get(span.end..t.span.start).is_some_and(|gap| gap.trim().is_empty())
        {
            *prev = format!("{prev} {word}");
            span.end = t.span.end;
            continue;
        }
        out.push(t);
    }
    out
}

/// Parses a visual calculation without binding it.
pub fn parse(text: &str) -> (Option<Expr>, Vec<Diagnostic>) {
    let mut raw = Vec::new();
    let toks = tokens(text, &mut raw);
    let expr = parser::parse(&toks, &mut raw);
    (expr, raw.into_iter().map(|r| r.locate(text)).collect())
}

/// Parses `text` and binds it against the visual's field names `fields` (case-insensitive);
/// `own` is the calculation's own name (a reference to it is a self-reference).
pub fn check(text: &str, fields: &[&str], own: &str) -> Check {
    let mut raw = Vec::new();
    let toks = tokens(text, &mut raw);
    let expr = parser::parse(&toks, &mut raw);
    let mut refs = Vec::new();
    if let Some(e) = &expr {
        let mut b = Binder {
            fields,
            own,
            vars: Vec::new(),
            refs: &mut refs,
            diags: &mut raw,
        };
        b.walk(e, 0);
    }
    let diagnostics: Vec<Diagnostic> = raw.into_iter().map(|r| r.locate(text)).collect();
    let valid = expr.is_some() && diagnostics.iter().all(|d| d.severity != Severity::Error);
    Check {
        valid,
        diagnostics,
        expr,
        refs,
    }
}

struct Binder<'a> {
    fields: &'a [&'a str],
    own: &'a str,
    vars: Vec<String>,
    refs: &'a mut Vec<String>,
    diags: &'a mut Vec<Raw>,
}

impl Binder<'_> {
    fn err(&mut self, code: &'static str, span: Span, message: String) {
        self.diags.push(Raw::error(code, span, message));
    }

    fn walk(&mut self, e: &Expr, depth: usize) {
        if depth > parser::MAX_DEPTH {
            return;
        }
        let d = depth.saturating_add(1);
        match e {
            Expr::Number { .. } | Expr::Str { .. } | Expr::Bool { .. } => {}
            Expr::Column {
                table: Some(t), span, ..
            } => self.err(
                "model-reference",
                *span,
                format!(
                    "visual calculations refer to the visual's fields only: add the field of `{t}` to the visual, then write its name in brackets"
                ),
            ),
            Expr::Column {
                table: None,
                name,
                span,
                ..
            } => {
                if name.eq_ignore_ascii_case(self.own) {
                    self.err(
                        "self-reference",
                        *span,
                        format!("visual calculation [{name}] refers to itself"),
                    );
                    return;
                }
                match self.fields.iter().find(|f| f.eq_ignore_ascii_case(name)) {
                    Some(f) => {
                        if !self.refs.iter().any(|r| r == f) {
                            self.refs.push((*f).to_string());
                        }
                    }
                    None => self.err(
                        "unknown-visual-field",
                        *span,
                        format!("the visual has no field named [{name}]"),
                    ),
                }
            }
            Expr::Name { name, quoted, span } => {
                let up = name.to_ascii_uppercase();
                let known = !quoted
                    && (self.vars.iter().any(|v| v.eq_ignore_ascii_case(name))
                        || is_axis(name)
                        || RESETS.contains(&up.as_str())
                        || KEYWORDS.contains(&up.as_str()));
                if !known {
                    self.err(
                        if *quoted { "model-reference" } else { "unknown-name" },
                        *span,
                        if *quoted {
                            format!("visual calculations cannot refer to table '{name}'")
                        } else {
                            format!("`{name}` is no variable, axis (ROWS, COLUMNS) or keyword")
                        },
                    );
                }
            }
            Expr::Call {
                name,
                name_span,
                args,
                ..
            } => {
                let up = name.to_ascii_uppercase();
                if MODEL_ONLY.contains(&up.as_str()) {
                    self.err(
                        "not-in-visual-calculation",
                        *name_span,
                        format!("{up} needs model relationships, which visual calculations do not have"),
                    );
                } else {
                    match crate::functions::lookup(name) {
                        None => self.err("unknown-function", *name_span, format!("unknown function {name}")),
                        Some(a) => {
                            let n = args.len();
                            let few = n < usize::from(a.min);
                            let many = a.max.is_some_and(|m| n > usize::from(m));
                            if few || many {
                                let want = match a.max {
                                    Some(m) if m == a.min => format!("{m}"),
                                    Some(m) => format!("{} to {m}", a.min),
                                    None => format!("at least {}", a.min),
                                };
                                self.err(
                                    "wrong-arity",
                                    *name_span,
                                    format!("{up} takes {want} arguments, not {n}"),
                                );
                            }
                        }
                    }
                }
                for a in args.iter().flatten() {
                    self.walk(a, d);
                }
            }
            Expr::Unary { operand, .. } => self.walk(operand, d),
            Expr::Binary { lhs, rhs, .. } => {
                self.walk(lhs, d);
                self.walk(rhs, d);
            }
            Expr::Var { bindings, body, .. } => {
                let before = self.vars.len();
                for b in bindings {
                    self.walk(&b.value, d);
                    self.vars.push(b.name.clone());
                }
                self.walk(body, d);
                self.vars.truncate(before);
            }
            Expr::TableCtor { rows, .. } => {
                for r in rows {
                    self.walk(r, d);
                }
            }
            Expr::Tuple { items, .. } => {
                for i in items {
                    self.walk(i, d);
                }
            }
        }
    }
}

/// The order to evaluate the calculations `calcs` (name, the fields it refers to) in: each one
/// after the calculations it refers to. `Err` names a calculation in a reference cycle.
pub fn order(calcs: &[(String, Vec<String>)]) -> Result<Vec<usize>, String> {
    let n = calcs.len();
    let mut state = vec![0u8; n]; // 0 new, 1 visiting, 2 done
    let mut out = Vec::with_capacity(n);
    for start in 0..n {
        // iterative depth-first search: (node, next reference to visit)
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some((node, k)) = stack.last().copied() {
            match state.get(node).copied() {
                Some(2) if k == 0 => {
                    stack.pop();
                    continue;
                }
                Some(0) => {
                    if let Some(s) = state.get_mut(node) {
                        *s = 1;
                    }
                }
                _ => {}
            }
            let refs = calcs.get(node).map(|c| c.1.as_slice()).unwrap_or_default();
            if let Some(r) = refs.get(k) {
                if let Some(top) = stack.last_mut() {
                    top.1 = k.saturating_add(1);
                }
                if let Some(j) = calcs.iter().position(|c| c.0.eq_ignore_ascii_case(r)) {
                    match state.get(j).copied() {
                        Some(1) => {
                            let who = calcs.get(j).map(|c| c.0.clone()).unwrap_or_default();
                            return Err(format!(
                                "visual calculation [{who}] refers to itself through others"
                            ));
                        }
                        Some(0) => stack.push((j, 0)),
                        _ => {}
                    }
                }
            } else {
                if let Some(s) = state.get_mut(node) {
                    *s = 2;
                }
                if !out.contains(&node) {
                    out.push(node);
                }
                stack.pop();
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(c: &Check) -> Vec<&'static str> {
        c.diagnostics.iter().map(|d| d.code).collect()
    }

    const FIELDS: [&str; 4] = ["Year", "Month", "Sales Amount", "Profit"];

    /// The documented forms bind: axes (combined too), resets, templates of Microsoft Learn.
    #[test]
    fn documented_forms_bind() {
        for t in [
            "RUNNINGSUM([Sales Amount])",
            "RUNNINGSUM([Sales Amount], ROWS)",
            "RUNNINGSUM([Sales Amount], ROWS COLUMNS)",
            "RUNNINGSUM([Sales Amount], COLUMNS ROWS, HIGHESTPARENT)",
            "RUNNINGSUM([Sales Amount], [Year])",
            "RUNNINGSUM([Sales Amount], -1)",
            "MOVINGAVERAGE([Sales Amount], 2, FALSE, ROWS, KEEP)",
            "DIVIDE([Sales Amount], COLLAPSE([Sales Amount], ROWS))",
            "DIVIDE([Sales Amount], COLLAPSEALL([Sales Amount], ROWS))",
            "EXPAND(AVERAGE([Sales Amount]), ROWS)",
            "EXPANDALL(MAX([Sales Amount]), ROWS)",
            "AVERAGEX(RANGE(1), [Sales Amount])",
            "CALCULATE(SUM([Sales Amount]), RANGE(-5, ROWS))",
            "ISATLEVEL([Year])",
            "[Profit] - PREVIOUS([Profit], 2, COLUMNS)",
            "[Profit] - NEXT([Profit])",
            "[Profit] - FIRST([Profit]) + LAST([Profit])",
            "COLLAPSE([Sales Amount], [Month])",
            "SUMX(WINDOW(1, ABS, 0, REL, ROWS, HIGHESTPARENT), [Profit])",
            "CALCULATE(SUM([Profit]), INDEX(1, , -1))",
            "LOOKUP([Profit], [Year], 2024, EXPLICIT)",
            "LOOKUPWITHTOTALS([Profit], [Year], 2024)",
            "RANK(DENSE, ORDERBY([Profit], DESC), PARTITIONBY([Year]))",
            "ROWNUMBER(ROWPAGES ROWS)",
            "VAR p = PREVIOUS([Profit]) RETURN IF(ISBLANK(p), BLANK(), [Profit] - p)",
        ] {
            let c = check(t, &FIELDS, "Calc");
            assert!(c.valid, "{t}: {:?}", c.diagnostics);
        }
        let c = check("[Profit] - PREVIOUS([profit]) + [Year]", &FIELDS, "Calc");
        assert_eq!(c.refs, vec!["Profit".to_string(), "Year".to_string()]);
    }

    #[test]
    fn errors_name_the_problem() {
        for (t, code) in [
            ("Sales[Amount] * 2", "model-reference"),
            ("COUNTROWS('Sales')", "model-reference"),
            ("[Nope] + 1", "unknown-visual-field"),
            ("[Calc] + 1", "self-reference"),
            ("RUNNINGSUM([Profit], SIDEWAYS)", "unknown-name"),
            ("RUNNINGSUM([Profit], ROWS ROWS)", "unexpected-token"),
            ("NOSUCH([Profit])", "unknown-function"),
            ("PREVIOUS()", "wrong-arity"),
            ("RELATED([Profit])", "not-in-visual-calculation"),
            ("[Profit] +", "unexpected-token"),
        ] {
            let c = check(t, &FIELDS, "Calc");
            assert!(!c.valid, "{t}");
            assert!(
                codes(&c).contains(&code) || (code == "unexpected-token" && !c.diagnostics.is_empty()),
                "{t}: {:?}",
                c.diagnostics
            );
        }
    }

    #[test]
    fn axis_words() {
        assert!(is_axis("rows") && is_axis("ROWS COLUMNS") && is_axis("COLUMNS ROWPAGES ROWS"));
        assert!(!is_axis("ROWS ROWS") && !is_axis("") && !is_axis("ROWS X"));
    }

    #[test]
    fn calculations_are_ordered_by_their_references() {
        let c = |n: &str, r: &[&str]| {
            (
                n.to_string(),
                r.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(),
            )
        };
        let calcs = vec![c("A", &["B", "Sales"]), c("B", &["C"]), c("C", &[])];
        assert_eq!(order(&calcs), Ok(vec![2, 1, 0]));
        let cyc = vec![c("A", &["B"]), c("B", &["a"])];
        assert!(order(&cyc).is_err());
    }
}
