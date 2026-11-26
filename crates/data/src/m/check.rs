//! Static checks of M expressions without evaluating them (`m.validate`, `model.validate`): the
//! syntax, and the names an expression uses that it does not define itself (its free names:
//! other queries, parameters, library functions). Nothing is evaluated, so external sources are
//! fine.
//!
//! Scoping follows the M specification: the variables of a `let` are in scope in all its
//! variable expressions and its body, the fields of a record expression in all its field
//! expressions, function parameters in the body, `each` defines `_`, and `catch` its parameter.

use std::collections::HashMap;
use std::rc::Rc;

use super::ast::{Expr, Handler, ListItem, Param, TypeExpr};
use super::parser::{IdentAt, parse_with_idents};
use super::value::Value;
use crate::{Error, Limits};

/// Stack size of the analysis thread (dropping a deep tree recurses).
const CHECK_STACK: usize = 256 << 20;

/// A syntax error, 1-based position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

/// A name the expression uses but does not define. The position is that of the reference
/// (`None` for the implicit `_` of a `[field]` access outside `each`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub name: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
    /// Byte range of the identifier as written (`#"quoted"` included), for rewrites.
    pub span: Option<(usize, usize)>,
}

/// What [`analyze`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Analysis {
    /// The first syntax error (the rest of the expression is then not checked).
    pub syntax: Option<SyntaxError>,
    /// Free names in source order, every occurrence.
    pub references: Vec<Reference>,
    /// The variable names of the outermost `let` (Power Query's steps), in order.
    pub steps: Vec<String>,
    /// The kind of value the expression evaluates to when it can be told from its text without
    /// evaluating it (`Table`, `Text`, `Number`, `Logical`, `Date`, `DateTime`, `Time`,
    /// `Duration`, `List`, `Record`, `Function`), as Power BI Desktop names it in the
    /// `PBI_ResultType` annotation.
    pub result_type: Option<&'static str>,
    /// The columns the query declares a date/time type for (typed `#table`,
    /// `Table.TransformColumnTypes`, typed `Table.AddColumn`), last declaration winning.
    pub date_kinds: Vec<(String, crate::DateKind)>,
}

/// Analyzes several expressions on one thread with a large stack.
pub fn analyze_all(expressions: Vec<String>, limits: &Limits) -> Vec<Analysis> {
    let limits = *limits;
    let n = expressions.len();
    let handle = std::thread::Builder::new()
        .name("bicraft-m-check".into())
        .stack_size(CHECK_STACK)
        .spawn(move || {
            expressions
                .iter()
                .map(|e| analyze_here(e, &limits))
                .collect::<Vec<_>>()
        });
    let failed = |message: String| {
        (0..n)
            .map(|_| Analysis {
                syntax: Some(SyntaxError {
                    line: 1,
                    column: 1,
                    message: message.clone(),
                }),
                ..Analysis::default()
            })
            .collect()
    };
    match handle {
        Ok(h) => h
            .join()
            .unwrap_or_else(|_| failed("the M check was aborted".into())),
        Err(e) => failed(format!("could not start the M check thread: {e}")),
    }
}

/// Analyzes one expression (see [`analyze_all`]).
pub fn analyze(expression: &str, limits: &Limits) -> Analysis {
    analyze_all(vec![expression.to_string()], limits)
        .into_iter()
        .next()
        .unwrap_or_default()
}

fn analyze_here(expression: &str, limits: &Limits) -> Analysis {
    if let Err(Error::LimitExceeded(m)) = limits.check_bytes(expression.len(), "M expression") {
        return Analysis {
            syntax: Some(SyntaxError {
                line: 1,
                column: 1,
                message: m,
            }),
            ..Analysis::default()
        };
    }
    let (tree, idents) = match parse_with_idents(expression, limits) {
        Ok(x) => x,
        Err(e) => {
            let (line, column, message) = match e {
                Error::Syntax {
                    line,
                    column,
                    message,
                    ..
                } => (line, column, message),
                other => (1, 1, other.to_string()),
            };
            return Analysis {
                syntax: Some(SyntaxError {
                    line,
                    column,
                    message,
                }),
                ..Analysis::default()
            };
        }
    };
    let at: HashMap<usize, &IdentAt> = idents.iter().map(|i| (key(&i.name), i)).collect();
    let mut references = free_names(&tree, &at);
    references.sort_by_key(|r| r.span.map_or(usize::MAX, |s| s.0));
    let steps = match &tree {
        Expr::Let(bindings, _) => bindings.iter().map(|(n, _)| n.to_string()).collect(),
        _ => Vec::new(),
    };
    Analysis {
        syntax: None,
        references,
        steps,
        result_type: result_type(&tree),
        date_kinds: super::coltypes::date_kinds(&tree),
    }
}

fn key(name: &Rc<str>) -> usize {
    Rc::as_ptr(name).cast::<u8>() as usize
}

enum Work<'a> {
    Expr(&'a Expr),
    Type(&'a TypeExpr),
    /// Enters a scope: function parameters.
    Enter(&'a Rc<Vec<Param>>),
    /// Leaves a scope: drop names down to this length.
    Pop(usize),
}

/// The free names of `tree`, walked with an explicit stack (no recursion).
fn free_names(tree: &Expr, at: &HashMap<usize, &IdentAt>) -> Vec<Reference> {
    let mut out = Vec::new();
    let mut scope: Vec<Rc<str>> = Vec::new();
    let mut stack = vec![Work::Expr(tree)];
    // Children are pushed in reverse so they are visited in source order.
    while let Some(w) = stack.pop() {
        let e = match w {
            Work::Pop(n) => {
                scope.truncate(n);
                continue;
            }
            Work::Enter(params) => {
                scope.extend(params.iter().map(|p| p.name.clone()));
                continue;
            }
            Work::Type(t) => {
                match t {
                    TypeExpr::Prim(_) => {}
                    TypeExpr::Nullable(x) | TypeExpr::List(x) | TypeExpr::Table(x) => {
                        stack.push(Work::Type(x));
                    }
                    TypeExpr::Record(fields) => {
                        for (_, x) in fields.iter().rev() {
                            stack.push(Work::Type(x));
                        }
                    }
                    TypeExpr::Expr(x) => stack.push(Work::Expr(x)),
                }
                continue;
            }
            Work::Expr(e) => e,
        };
        match e {
            Expr::Const(_) | Expr::NotImplemented => {}
            Expr::Ident(name) => {
                if !scope.iter().any(|s| **s == **name) {
                    let pos = at.get(&key(name));
                    out.push(Reference {
                        name: name.to_string(),
                        line: pos.map(|p| p.line),
                        column: pos.map(|p| p.column),
                        span: pos.map(|p| (p.start, p.end)),
                    });
                }
            }
            Expr::Let(bindings, body) => {
                stack.push(Work::Pop(scope.len()));
                scope.extend(bindings.iter().map(|(n, _)| n.clone()));
                stack.push(Work::Expr(body));
                for (_, x) in bindings.iter().rev() {
                    stack.push(Work::Expr(x));
                }
            }
            Expr::Record(bindings) => {
                stack.push(Work::Pop(scope.len()));
                scope.extend(bindings.iter().map(|(n, _)| n.clone()));
                for (_, x) in bindings.iter().rev() {
                    stack.push(Work::Expr(x));
                }
            }
            Expr::Func(params, body) => {
                stack.push(Work::Pop(scope.len()));
                scope.extend(params.iter().map(|p| p.name.clone()));
                stack.push(Work::Expr(body));
            }
            Expr::If(c, t, f) => {
                stack.push(Work::Expr(f));
                stack.push(Work::Expr(t));
                stack.push(Work::Expr(c));
            }
            Expr::Call(f, args) => {
                for a in args.iter().rev() {
                    stack.push(Work::Expr(a));
                }
                stack.push(Work::Expr(f));
            }
            Expr::Field(x, _, _) | Expr::Project(x, _, _) => stack.push(Work::Expr(x)),
            Expr::Item(x, i, _) => {
                stack.push(Work::Expr(i));
                stack.push(Work::Expr(x));
            }
            Expr::List(items) => {
                for item in items.iter().rev() {
                    match item {
                        ListItem::One(x) => stack.push(Work::Expr(x)),
                        ListItem::Range(a, b) => {
                            stack.push(Work::Expr(b));
                            stack.push(Work::Expr(a));
                        }
                    }
                }
            }
            Expr::Unary(_, x) | Expr::Error(x) => stack.push(Work::Expr(x)),
            Expr::Binary(_, a, b) | Expr::Meta(a, b) => {
                stack.push(Work::Expr(b));
                stack.push(Work::Expr(a));
            }
            Expr::Is(x, t) | Expr::As(x, t) => {
                stack.push(Work::Type(t));
                stack.push(Work::Expr(x));
            }
            Expr::Type(t) => stack.push(Work::Type(t)),
            Expr::Try(x, handler) => {
                match handler {
                    Handler::None => {}
                    Handler::Otherwise(h) => stack.push(Work::Expr(h)),
                    Handler::Catch(params, body) => {
                        // The parameter is in scope in the handler only, entered once the
                        // protected expression (visited first) left the scope as it was.
                        stack.push(Work::Pop(scope.len()));
                        stack.push(Work::Expr(body));
                        stack.push(Work::Enter(params));
                    }
                }
                stack.push(Work::Expr(x));
            }
        }
    }
    out
}

/// The value kind of `e` when its text tells it.
fn result_type(e: &Expr) -> Option<&'static str> {
    let mut e = e;
    // Follow `let … in Step` to the step's expression (a bounded number of times).
    for _ in 0..64 {
        match e {
            Expr::Let(bindings, body) => {
                let next = match &**body {
                    Expr::Ident(n) => bindings.iter().rev().find(|(b, _)| **b == **n).map(|(_, x)| &**x),
                    _ => None,
                };
                e = next.unwrap_or(body);
            }
            Expr::Meta(x, _) => e = x,
            _ => break,
        }
    }
    Some(match e {
        Expr::Const(Value::Text(_)) => "Text",
        Expr::Const(Value::Number(_)) => "Number",
        Expr::Const(Value::Logical(_)) => "Logical",
        Expr::Func(..) => "Function",
        Expr::List(_) => "List",
        Expr::Record(_) => "Record",
        Expr::Call(f, _) => match &**f {
            Expr::Ident(name) => call_result(name)?,
            _ => return None,
        },
        // `Source{[Name = "…"]}[Data]`: a navigation step.
        Expr::Field(x, field, _) if &**field == "Data" && matches!(&**x, Expr::Item(..)) => "Table",
        _ => return None,
    })
}

fn call_result(name: &str) -> Option<&'static str> {
    Some(match name {
        "#table" => "Table",
        "#date" => "Date",
        "#datetime" => "DateTime",
        "#time" => "Time",
        "#duration" => "Duration",
        _ if name.starts_with("Table.") => match name.get(6..).unwrap_or_default() {
            "RowCount"
            | "ColumnCount"
            | "ApproximateRowCount"
            | "IsEmpty"
            | "IsDistinct"
            | "Contains"
            | "ContainsAll"
            | "ContainsAny"
            | "MatchesAllRows"
            | "MatchesAnyRows"
            | "HasColumns" => {
                return None;
            }
            "ColumnNames" | "Column" | "ToList" | "ToRecords" | "ToRows" | "ToColumns" | "PositionOf"
            | "PositionOfAny" | "ColumnsOfType" | "Keys" | "Split" | "SplitAt" => "List",
            "First" | "Last" | "Max" | "Min" | "SingleRow" => "Record",
            "FirstValue" => return None,
            _ => "Table",
        },
        _ if super::lib::files::external(name).is_some() => "Table",
        "Csv.Document"
        | "Excel.CurrentWorkbook"
        | "Excel.Workbook"
        | "Json.Document"
        | "Xml.Tables"
        | "Html.Table"
        | "Parquet.Document"
        | "Pdf.Tables"
        | "Folder.Files"
        | "Folder.Contents" => "Table",
        _ => return None,
    })
}
