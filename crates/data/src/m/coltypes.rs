//! The date/time types an M query declares for its columns, read from the text without
//! evaluating it (so external sources are fine): typed `#table` literals,
//! `Table.TransformColumnTypes`, typed `Table.AddColumn`, and `Table.RenameColumns` carrying a
//! declared type along. Power BI Desktop derives a `dateTime` column's
//! `UnderlyingDateTimeDataType` annotation and default format string from that type.
//!
//! Steps are taken in evaluation order (children before the call that uses them, `let`
//! variables in order), the last declaration of a column wins.

use super::ast::{Expr, ListItem, TypeExpr};
use super::value::{Kind, Value};
use crate::DateKind;

/// Nodes visited at most (the parser already caps the depth; this caps the work).
const MAX_NODES: usize = 1_000_000;
/// Columns tracked at most.
const MAX_COLUMNS: usize = 10_000;

enum Work<'a> {
    Visit(&'a Expr),
    Done(&'a Expr),
}

/// The declared kind of a type value: `Some(Some(k))` a date/time type, `Some(None)` another
/// type, `None` not known from the text.
type Declared = Option<Option<DateKind>>;

fn prim(kind: Kind) -> Option<DateKind> {
    match kind {
        Kind::Date => Some(DateKind::Date),
        Kind::Time => Some(DateKind::Time),
        Kind::DateTime => Some(DateKind::DateTime),
        Kind::DateTimeZone => Some(DateKind::DateTimeZone),
        _ => None,
    }
}

fn type_expr(t: &TypeExpr) -> Declared {
    let mut t = t;
    for _ in 0..16 {
        match t {
            TypeExpr::Prim(m) => return Some(prim(m.kind)),
            TypeExpr::Nullable(x) => t = x,
            TypeExpr::Expr(e) => return type_value(e),
            TypeExpr::List(_) | TypeExpr::Record(_) | TypeExpr::Table(_) => return Some(None),
        }
    }
    None
}

fn type_value(e: &Expr) -> Declared {
    let mut e = e;
    for _ in 0..16 {
        match e {
            Expr::Type(t) => return type_expr(t),
            Expr::Meta(x, _) => e = x,
            Expr::Ident(n) => {
                return match &**n {
                    "Date.Type" => Some(Some(DateKind::Date)),
                    "Time.Type" => Some(Some(DateKind::Time)),
                    "DateTime.Type" => Some(Some(DateKind::DateTime)),
                    "DateTimeZone.Type" => Some(Some(DateKind::DateTimeZone)),
                    other if other.ends_with(".Type") => Some(None),
                    _ => None,
                };
            }
            _ => return None,
        }
    }
    None
}

fn text(e: &Expr) -> Option<&str> {
    match e {
        Expr::Const(Value::Text(t)) => Some(t),
        _ => None,
    }
}

fn items(e: &Expr) -> Option<Vec<&Expr>> {
    match e {
        Expr::List(items) => items
            .iter()
            .map(|i| match i {
                ListItem::One(x) => Some(x),
                ListItem::Range(..) => None,
            })
            .collect(),
        _ => None,
    }
}

fn pair<'a>(l: &[&'a Expr]) -> Option<(&'a str, &'a Expr)> {
    match l {
        [a, b] => text(a).map(|a| (a, *b)),
        _ => None,
    }
}

/// `{"a", x}` or `{{"a", x}, {"b", y}}`: the pairs.
fn pairs(e: &Expr) -> Vec<(&str, &Expr)> {
    let Some(list) = items(e) else {
        return Vec::new();
    };
    if list.first().is_some_and(|x| text(x).is_some()) {
        return pair(&list).into_iter().collect();
    }
    list.iter()
        .filter_map(|x| items(x).and_then(|l| pair(&l)))
        .collect()
}

struct Columns(Vec<(String, Option<DateKind>)>);

impl Columns {
    fn declare(&mut self, name: &str, kind: Option<DateKind>) {
        if let Some(c) = self.0.iter_mut().find(|(n, _)| n == name) {
            c.1 = kind;
        } else if self.0.len() < MAX_COLUMNS {
            self.0.push((name.to_string(), kind));
        }
    }

    fn rename(&mut self, old: &str, new: &str) {
        let kind = match self.0.iter().position(|(n, _)| n == old) {
            Some(i) => self.0.remove(i).1,
            None => return,
        };
        self.declare(new, kind);
    }
}

fn call(name: &str, args: &[Expr], out: &mut Columns) {
    match name {
        "#table" => {
            if let Some(Expr::Type(t)) = args.first()
                && let TypeExpr::Table(row) = &**t
                && let TypeExpr::Record(fields) = &**row
            {
                for (n, t) in fields {
                    if let Some(k) = type_expr(t) {
                        out.declare(n, k);
                    }
                }
            }
        }
        "Table.TransformColumnTypes" => {
            for (n, t) in args.get(1).map(pairs).unwrap_or_default() {
                if let Some(k) = type_value(t) {
                    out.declare(n, k);
                }
            }
        }
        "Table.AddColumn" => {
            // An untyped added column is `any`: no date/time type.
            if let Some(n) = args.get(1).and_then(text) {
                out.declare(n, args.get(3).and_then(type_value).flatten());
            }
        }
        "Table.RenameColumns" => {
            for (old, new) in args.get(1).map(pairs).unwrap_or_default() {
                if let Some(new) = text(new) {
                    out.rename(old, new);
                }
            }
        }
        _ => {}
    }
}

/// The columns `tree` declares a date/time type for, in first-declaration order.
pub(crate) fn date_kinds(tree: &Expr) -> Vec<(String, DateKind)> {
    let mut out = Columns(Vec::new());
    let mut stack = vec![Work::Visit(tree)];
    let mut visited = 0usize;
    while let Some(w) = stack.pop() {
        let e = match w {
            Work::Done(Expr::Call(f, args)) => {
                if let Expr::Ident(n) = &**f {
                    call(n, args, &mut out);
                }
                continue;
            }
            Work::Done(_) => continue,
            Work::Visit(e) => e,
        };
        visited = visited.saturating_add(1);
        if visited > MAX_NODES {
            break;
        }
        // Post-order: the node's own work after its children, children in source order.
        stack.push(Work::Done(e));
        let mut children: Vec<&Expr> = Vec::new();
        match e {
            Expr::Const(_) | Expr::Ident(_) | Expr::NotImplemented | Expr::Type(_) => {}
            Expr::Let(bindings, body) => {
                children.extend(bindings.iter().map(|(_, x)| &**x));
                children.push(body);
            }
            Expr::Record(bindings) => children.extend(bindings.iter().map(|(_, x)| &**x)),
            Expr::Func(_, body) => children.push(body),
            Expr::If(c, t, f) => children.extend([&**c, &**t, &**f]),
            Expr::Call(f, args) => {
                children.push(f);
                children.extend(args.iter());
            }
            Expr::Field(x, _, _) | Expr::Project(x, _, _) => children.push(x),
            Expr::Item(x, i, _) => children.extend([&**x, &**i]),
            Expr::List(list) => {
                for item in list {
                    match item {
                        ListItem::One(x) => children.push(x),
                        ListItem::Range(a, b) => children.extend([a, b]),
                    }
                }
            }
            Expr::Unary(_, x) | Expr::Error(x) | Expr::Is(x, _) | Expr::As(x, _) => children.push(x),
            Expr::Binary(_, a, b) | Expr::Meta(a, b) => children.extend([&**a, &**b]),
            Expr::Try(x, handler) => {
                children.push(x);
                match handler {
                    super::ast::Handler::None => {}
                    super::ast::Handler::Otherwise(h) => children.push(h),
                    super::ast::Handler::Catch(_, body) => children.push(body),
                }
            }
        }
        for c in children.into_iter().rev() {
            stack.push(Work::Visit(c));
        }
    }
    out.0.into_iter().filter_map(|(n, k)| k.map(|k| (n, k))).collect()
}
