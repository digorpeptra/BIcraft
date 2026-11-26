//! The model's user-defined functions (TMSL/TMDL `functions`, kept in the model's unmodelled
//! properties): their signature, the validation of their definition, the checks of their call
//! sites, and the inline expansion the evaluator runs them by. The rules are those of the public
//! "DAX user-defined functions" page of the DAX reference (`docs/formats/dax-udf.md`).
//!
//! A function's stored expression is `( params ) => body`, the text after `FUNCTION Name =` in a
//! DAX query. It is parsed and validated as the query `DEFINE FUNCTION Name = <expression>
//! EVALUATE {0}`, so the query checker's rules (names, type hints, recursion, the body bound with
//! its parameters) apply, and the diagnostics are mapped back onto the expression.

use bicraft_model::SemanticModel;
use serde::Serialize;

use crate::ast::{Define, Expr};
use crate::index::Names;
use crate::{Context, Diagnostic, Raw, Severity, Span, Validation};

/// Most rounds of inline expansion: a function calling a function … this deep is refused
/// (recursion is not allowed, so a real chain stops long before).
const MAX_ROUNDS: usize = 32;
/// Largest expanded text, so a chain of functions that each use a lazy parameter several times
/// cannot blow up.
const MAX_EXPANDED_BYTES: usize = 4 * 1024 * 1024;

/// Effective parameter type, after the defaults of the Learn page's table.
pub const TYPES: &[&str] = &[
    "ANYVAL",
    "SCALAR",
    "TABLE",
    "ANYREF",
    "CALENDARREF",
    "COLUMNREF",
    "MEASUREREF",
    "TABLEREF",
];
/// Scalar subtypes.
pub const SUBTYPES: &[&str] = &[
    "VARIANT", "INT64", "DECIMAL", "DOUBLE", "STRING", "DATETIME", "BOOLEAN", "NUMERIC",
];
/// Parameter passing modes.
pub const MODES: &[&str] = &["VAL", "EXPR"];

/// One parameter of a user-defined function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UdfParam {
    pub name: String,
    /// The type hint words as written (`["SCALAR", "NUMERIC", "VAL"]`, upper case).
    pub hints: Vec<String>,
    /// Effective type: `ANYVAL` when none is written, `SCALAR` when only a subtype is.
    #[serde(rename = "type")]
    pub kind: String,
    pub subtype: Option<String>,
    /// Effective mode: `EXPR` for reference types, else `VAL` unless `EXPR` is written.
    pub mode: String,
    /// The default expression's text: the parameter is optional.
    pub default: Option<String>,
}

impl UdfParam {
    pub fn is_ref(&self) -> bool {
        self.kind.ends_with("REF")
    }

    pub fn is_lazy(&self) -> bool {
        self.mode == "EXPR"
    }

    /// The parameter as written in a signature: `name : SCALAR NUMERIC VAL = 0`.
    pub fn text(&self) -> String {
        let mut s = self.name.clone();
        if !self.hints.is_empty() {
            s.push_str(" : ");
            s.push_str(&self.hints.join(" "));
        }
        if let Some(d) = &self.default {
            s.push_str(" = ");
            s.push_str(d);
        }
        s
    }
}

/// A user-defined function's signature and body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UdfSignature {
    pub params: Vec<UdfParam>,
    pub body: String,
}

impl UdfSignature {
    /// Fewest arguments a call passes: up to the rightmost parameter without a default.
    pub fn min_args(&self) -> usize {
        self.params
            .iter()
            .rposition(|p| p.default.is_none())
            .map_or(0, |i| i.saturating_add(1))
    }
}

/// `( p1, p2 : TABLE ) => body` from parameters and a body.
pub fn compose(params: &[UdfParam], body: &str) -> String {
    let list: Vec<String> = params.iter().map(UdfParam::text).collect();
    format!("({}) => {}", list.join(", "), body.trim())
}

/// A parameter from its parts: `type`, `subtype`, `mode` (any case, each optional).
pub fn param(
    name: &str,
    kind: Option<&str>,
    subtype: Option<&str>,
    mode: Option<&str>,
    default: Option<&str>,
) -> Result<UdfParam, String> {
    let pick = |v: Option<&str>, list: &[&str], what: &str| -> Result<Option<String>, String> {
        match v.map(str::trim).filter(|v| !v.is_empty()) {
            None => Ok(None),
            Some(v) => {
                let up = v.to_ascii_uppercase();
                if list.contains(&up.as_str()) {
                    Ok(Some(up))
                } else {
                    Err(format!(
                        "parameter `{name}`: unknown {what} `{v}` (one of {})",
                        list.join(", ")
                    ))
                }
            }
        }
    };
    let kind = pick(kind, TYPES, "type")?;
    let subtype = pick(subtype, SUBTYPES, "subtype")?;
    let mode = pick(mode, MODES, "mode")?;
    let mut hints = Vec::new();
    hints.extend(kind.iter().cloned());
    hints.extend(subtype.iter().cloned());
    hints.extend(mode.iter().cloned());
    Ok(effective(name, hints, default.map(|d| d.trim().to_string())))
}

fn effective(name: &str, hints: Vec<String>, default: Option<String>) -> UdfParam {
    let written_type = hints.iter().find(|w| TYPES.contains(&w.as_str())).cloned();
    let subtype = hints.iter().find(|w| SUBTYPES.contains(&w.as_str())).cloned();
    let kind = written_type.unwrap_or_else(|| {
        if subtype.is_some() {
            "SCALAR".to_string()
        } else {
            "ANYVAL".to_string()
        }
    });
    let mode = if kind.ends_with("REF") || hints.iter().any(|w| w == "EXPR") {
        "EXPR"
    } else {
        "VAL"
    };
    UdfParam {
        name: name.to_string(),
        hints,
        kind,
        subtype,
        mode: mode.to_string(),
        default,
    }
}

/// The text of a stored function expression: a string, or a TMSL array of lines.
pub fn expression_text(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(lines) => Some(
            lines
                .iter()
                .map(|l| l.as_str().unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

/// The model's user-defined function `name` (case-insensitive): its name as stored and its
/// expression text.
pub fn model_function(model: &SemanticModel, name: &str) -> Option<(String, String)> {
    model
        .extra
        .get("functions")
        .and_then(|f| f.as_array())?
        .iter()
        .find_map(|f| {
            let n = f.get("name").and_then(|n| n.as_str())?;
            if !n.eq_ignore_ascii_case(name) {
                return None;
            }
            Some((n.to_string(), f.get("expression").and_then(expression_text)?))
        })
}

/// Names of the model's user-defined functions, in model order.
pub fn model_function_names(model: &SemanticModel) -> Vec<String> {
    model
        .extra
        .get("functions")
        .and_then(|f| f.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|f| f.get("name").and_then(|n| n.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The wrapper query a function expression is parsed and validated as.
fn wrap(name: &str, expression: &str) -> (String, usize) {
    let prefix = format!("DEFINE FUNCTION {name} = ");
    let start = prefix.len();
    (format!("{prefix}{expression}\nEVALUATE {{0}}"), start)
}

/// Moves a diagnostic of the wrapper query onto the expression (offsets before it go to 0,
/// after it to its end).
fn shift(d: Diagnostic, start: usize, expression: &str) -> Diagnostic {
    let len = expression.len();
    let at = |o: usize| o.saturating_sub(start).min(len);
    Raw {
        severity: d.severity,
        code: d.code,
        message: d.message,
        span: Span {
            start: at(d.span.start),
            end: at(d.span.end),
        },
    }
    .locate(expression)
}

/// Parses a function expression `( params ) => body`.
pub fn parse_function(expression: &str) -> Result<UdfSignature, Vec<Diagnostic>> {
    let (query, start) = wrap("BicraftUdf", expression);
    let (parsed, diags) = crate::parse_query(&query);
    let errors: Vec<Diagnostic> = diags
        .into_iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| shift(d, start, expression))
        .collect();
    let not_a_function = || {
        vec![
            Raw::error(
                "not-a-function",
                Span::default(),
                "a function is written `( parameters ) => body`",
            )
            .locate(expression),
        ]
    };
    let Some(q) = parsed.filter(|_| errors.is_empty()) else {
        return Err(if errors.is_empty() {
            not_a_function()
        } else {
            errors
        });
    };
    let Some(Define::Function {
        params, body_span, ..
    }) = q.defines.first()
    else {
        return Err(not_a_function());
    };
    if q.defines.len() != 1 || q.statements.len() != 1 {
        return Err(not_a_function());
    }
    let slice = |s: Span| query.get(s.start..s.end).map(|t| t.trim().to_string());
    let mut out = Vec::new();
    for p in params {
        let hints = p.hints.iter().map(|h| h.word.to_ascii_uppercase()).collect();
        let default = p.default_span.and_then(slice);
        out.push(effective(&p.name, hints, default));
    }
    let body = slice(*body_span).ok_or_else(not_a_function)?;
    Ok(UdfSignature { params: out, body })
}

/// Validates the definition of the function `name` against `model`: the name, the parameters
/// and their type hints, recursion, and the body bound with its parameters. Diagnostics are on
/// the expression text (problems with the name itself point at its start).
pub fn validate_function(name: &str, expression: &str, model: Option<&SemanticModel>) -> Validation {
    let (query, start) = wrap(name, expression);
    let v = crate::validate_query(&query, &Context { model, table: None });
    let mut diagnostics: Vec<Diagnostic> = v
        .diagnostics
        .into_iter()
        .map(|d| shift(d, start, expression))
        .collect();
    if diagnostics.is_empty()
        && let Err(e) = parse_function(expression)
    {
        diagnostics = e;
    }
    let valid = diagnostics.iter().all(|d| d.severity != Severity::Error);
    Validation {
        valid: valid && v.valid,
        diagnostics,
        result_type: None,
    }
}

/// The model functions `expression` calls (names as stored, each once).
pub fn called_functions(model: &SemanticModel, expression: &str) -> Vec<String> {
    let tokens = crate::tokenize(expression);
    let mut out: Vec<String> = Vec::new();
    for w in tokens.windows(2) {
        if let [a, b] = w
            && let (crate::TokenKind::Ident(n), crate::TokenKind::LParen) = (&a.kind, &b.kind)
            && crate::lookup_function(n).is_none()
            && let Some((name, _)) = model_function(model, n)
            && !out.iter().any(|x| x.eq_ignore_ascii_case(&name))
        {
            out.push(name);
        }
    }
    out
}

/// A cycle of calls among the model's functions through `name` (with `expression` as its
/// definition), e.g. `["A", "B", "A"]`: user-defined functions cannot be recursive.
pub fn call_cycle(model: &SemanticModel, name: &str, expression: &str) -> Option<Vec<String>> {
    fn walk(
        model: &SemanticModel,
        root: &str,
        cur: &str,
        text: &str,
        path: &mut Vec<String>,
        depth: usize,
    ) -> Option<Vec<String>> {
        if depth > MAX_ROUNDS {
            return None;
        }
        for callee in called_functions(model, text) {
            if callee.eq_ignore_ascii_case(root) {
                let mut cycle = path.clone();
                cycle.push(root.to_string());
                return Some(cycle);
            }
            if path.iter().any(|p| p.eq_ignore_ascii_case(&callee)) || callee.eq_ignore_ascii_case(cur) {
                continue;
            }
            if let Some((_, t)) = model_function(model, &callee) {
                path.push(callee.clone());
                if let Some(c) = walk(model, root, &callee, &t, path, depth.saturating_add(1)) {
                    return Some(c);
                }
                path.pop();
            }
        }
        None
    }
    let mut path = vec![name.to_string()];
    walk(model, name, name, expression, &mut path, 0)
}

// ---------------------------------------------------------------------------------------------
// Call sites (the binder)
// ---------------------------------------------------------------------------------------------

/// Checks a call of the model function `name`: the argument count, and the argument kinds the
/// type hints rule out (a table for a scalar, a literal for a table or a reference, a measure
/// for a column reference…). Arguments of untyped (`AnyVal`) parameters are not checked.
pub(crate) fn check_call(
    names: Names<'_>,
    name: &str,
    args: &[Option<Expr>],
    span: Span,
    is_var: &dyn Fn(&str) -> bool,
    diags: &mut Vec<Raw>,
) {
    let Some((stored, text)) = model_function(names.model(), name) else {
        return;
    };
    let Ok(sig) = parse_function(&text) else {
        // the function's own definition is reported where it is defined
        return;
    };
    let (min, max, argc) = (sig.min_args(), sig.params.len(), args.len());
    if argc < min || argc > max {
        let expected = if min == max {
            format!("{max}")
        } else {
            format!("{min} to {max}")
        };
        diags.push(Raw::error(
            "wrong-argument-count",
            span,
            format!("{stored} takes {expected} argument(s), got {argc}"),
        ));
        return;
    }
    for (p, a) in sig.params.iter().zip(args) {
        let Some(a) = a else { continue };
        if let Some(problem) = argument_problem(names, p, a, is_var) {
            diags.push(Raw::error(
                "argument-type",
                a.span(),
                format!("{stored}: parameter `{}` ({}) {problem}", p.name, p.hints_label()),
            ));
        }
    }
}

impl UdfParam {
    fn hints_label(&self) -> String {
        match &self.subtype {
            Some(s) => format!("{} {s} {}", self.kind, self.mode),
            None => format!("{} {}", self.kind, self.mode),
        }
    }
}

/// What an argument is, as far as its syntax and the model tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arg {
    Literal,
    Scalar,
    Table,
    TableName,
    Column,
    Measure,
    Unknown,
}

fn classify(names: Names<'_>, a: &Expr, is_var: &dyn Fn(&str) -> bool) -> Arg {
    match a {
        Expr::Number { .. } | Expr::Str { .. } | Expr::Bool { .. } => Arg::Literal,
        Expr::Unary { .. } | Expr::Binary { .. } => Arg::Scalar,
        Expr::TableCtor { .. } => Arg::Table,
        Expr::Name { name, .. } => {
            if is_var(name) {
                Arg::Unknown
            } else if names.table(name).is_some() {
                Arg::TableName
            } else {
                Arg::Unknown
            }
        }
        Expr::Column { table, name, .. } => {
            if table.as_deref().is_some_and(is_var) {
                return Arg::Unknown;
            }
            let measure = match table {
                Some(t) => names
                    .table_at(t)
                    .and_then(|tt| names.measure_of(tt, name))
                    .is_some(),
                None => names.measure(name).is_some(),
            };
            let column = match table {
                Some(t) => names
                    .table_at(t)
                    .and_then(|tt| names.column_of(tt, name))
                    .is_some(),
                None => names.any_column(name).is_some(),
            };
            match (column, measure) {
                (true, _) => Arg::Column,
                (false, true) => Arg::Measure,
                _ => Arg::Unknown,
            }
        }
        Expr::Call { name, .. } => match crate::signature(name).map(|s| &s.returns) {
            Some(crate::types::Returns::Fixed(crate::types::ValueType::Table)) => Arg::Table,
            Some(crate::types::Returns::Fixed(crate::types::ValueType::Scalar(_))) => Arg::Scalar,
            _ => Arg::Unknown,
        },
        Expr::Var { .. } | Expr::Tuple { .. } => Arg::Unknown,
    }
}

fn argument_problem(
    names: Names<'_>,
    p: &UdfParam,
    a: &Expr,
    is_var: &dyn Fn(&str) -> bool,
) -> Option<&'static str> {
    let arg = classify(names, a, is_var);
    match p.kind.as_str() {
        "SCALAR" => matches!(arg, Arg::Table | Arg::TableName).then_some("takes a scalar, not a table"),
        "TABLE" => matches!(arg, Arg::Literal | Arg::Scalar | Arg::Column | Arg::Measure)
            .then_some("takes a table, not a scalar"),
        "COLUMNREF" => (!matches!(arg, Arg::Column | Arg::Unknown)).then_some("takes a column reference"),
        "MEASUREREF" => (!matches!(arg, Arg::Measure | Arg::Unknown)).then_some("takes a measure reference"),
        "TABLEREF" => (!matches!(arg, Arg::TableName | Arg::Unknown)).then_some("takes a table reference"),
        "ANYREF" | "CALENDARREF" => {
            matches!(arg, Arg::Literal | Arg::Scalar).then_some("takes a reference, not a value")
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Inline expansion (the evaluator)
// ---------------------------------------------------------------------------------------------

/// `text` with every call of a model function replaced by its body: a `val` parameter becomes a
/// variable bound to the argument (evaluated once, in the caller's contexts), an `expr` parameter
/// is replaced by the argument's text (evaluated where the body uses it, in the body's filter
/// context), a missing argument by the default expression or `BLANK()`. `Ok(None)`: the text
/// calls no model function. `Err`: a call does not match its function, or the expansion does not
/// end (recursion) or grows too large.
pub fn inline_calls(text: &str, model: &SemanticModel) -> Result<Option<String>, String> {
    if model_function_names(model).is_empty() {
        return Ok(None);
    }
    let mut cur = text.to_string();
    let mut counter = 0usize;
    for round in 0..MAX_ROUNDS {
        let (Some(expr), _) = crate::parse_expression(&cur) else {
            return if round == 0 {
                Ok(None)
            } else {
                Err("the expansion of a user-defined function does not parse".into())
            };
        };
        let mut calls: Vec<(&str, &[Option<Expr>], Span)> = Vec::new();
        let mut stack = vec![&expr];
        while let Some(e) = stack.pop() {
            if let Expr::Call { name, args, span, .. } = e
                && crate::lookup_function(name).is_none()
                && model_function(model, name).is_some()
            {
                calls.push((name, args, *span));
                continue;
            }
            crate::query::push_children(e, &mut stack);
        }
        if calls.is_empty() {
            return Ok((round > 0).then_some(cur));
        }
        calls.sort_by_key(|c| c.2.start);
        let mut out = String::with_capacity(cur.len());
        let mut at = 0usize;
        for (name, args, span) in calls {
            if span.start < at {
                continue;
            }
            out.push_str(cur.get(at..span.start).unwrap_or_default());
            counter = counter.saturating_add(1);
            out.push_str(&expand(model, &cur, name, args, counter)?);
            at = span.end;
        }
        out.push_str(cur.get(at..).unwrap_or_default());
        if out.len() > MAX_EXPANDED_BYTES {
            return Err("the expansion of the user-defined functions is too large".into());
        }
        cur = out;
    }
    Err(format!(
        "user-defined functions call each other more than {MAX_ROUNDS} levels deep (recursion is not allowed)"
    ))
}

fn expand(
    model: &SemanticModel,
    text: &str,
    name: &str,
    args: &[Option<Expr>],
    k: usize,
) -> Result<String, String> {
    let (stored, def) = model_function(model, name).ok_or_else(|| format!("unknown function {name}"))?;
    let sig = parse_function(&def).map_err(|d| {
        let first = d.first().map(|d| d.message.clone()).unwrap_or_default();
        format!("function {stored} does not parse: {first}")
    })?;
    let (min, max) = (sig.min_args(), sig.params.len());
    if args.len() < min || args.len() > max {
        return Err(format!(
            "{stored} takes {min} to {max} argument(s), got {}",
            args.len()
        ));
    }
    let arg_text = |a: &Expr| {
        let s = a.span();
        text.get(s.start..s.end).unwrap_or_default().to_string()
    };
    let (Some(body), _) = crate::parse_expression(&sig.body) else {
        return Err(format!("the body of function {stored} does not parse"));
    };
    // what each parameter stands for in the body, and the variables of the `val` ones
    let mut bindings = Vec::new();
    let mut subst: Vec<(String, String, bool)> = Vec::new(); // (upper name, text, simple)
    for (i, p) in sig.params.iter().enumerate() {
        let given = args.get(i).and_then(Option::as_ref);
        let (value, simple) = match given {
            Some(a) => (
                arg_text(a),
                matches!(a, Expr::Column { .. } | Expr::Name { .. } | Expr::Call { .. }),
            ),
            None => match &p.default {
                Some(d) => (d.clone(), false),
                None => ("BLANK()".to_string(), true),
            },
        };
        if p.is_lazy() {
            subst.push((p.name.to_ascii_uppercase(), value, simple));
        } else {
            let var = format!("__udf{k}_{i}");
            bindings.push(format!("VAR {var} = ({value})"));
            subst.push((p.name.to_ascii_uppercase(), var, true));
        }
    }
    // the body with its parameters replaced
    let mut edits: Vec<(Span, String)> = Vec::new();
    let find = |n: &str| subst.iter().find(|(u, _, _)| *u == n.to_ascii_uppercase());
    let mut stack = vec![&body];
    while let Some(e) = stack.pop() {
        match e {
            Expr::Name {
                name,
                quoted: false,
                span,
            } => {
                if let Some((_, t, simple)) = find(name) {
                    let t = if *simple { t.clone() } else { format!("({t})") };
                    edits.push((*span, t));
                }
            }
            Expr::Column {
                table: Some(t),
                table_span: Some(ts),
                ..
            } => {
                if let Some((_, value, _)) = find(t) {
                    edits.push((*ts, value.clone()));
                }
            }
            _ => crate::query::push_children(e, &mut stack),
        }
    }
    edits.sort_by_key(|(s, _)| s.start);
    let mut new_body = String::with_capacity(sig.body.len());
    let mut at = 0usize;
    for (s, t) in edits {
        if s.start < at {
            continue;
        }
        new_body.push_str(sig.body.get(at..s.start).unwrap_or_default());
        new_body.push_str(&t);
        at = s.end;
    }
    new_body.push_str(sig.body.get(at..).unwrap_or_default());
    Ok(if bindings.is_empty() {
        format!("({new_body})")
    } else {
        format!("({}\nRETURN ({new_body}))", bindings.join("\n"))
    })
}
