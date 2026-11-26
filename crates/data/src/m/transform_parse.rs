//! A step's settings, read back from its M (the editor's step settings gear): the `{kind, …}`
//! that `query.apply_step` takes and that wrote the step, for the shapes `transform.rs` and
//! `transform_more.rs` write and that ask for values (Filter rows, Replace values, Group by,
//! Merge queries, Split column, Custom column, Keep and Remove rows, Replace errors, Index
//! column, Merge columns, Conditional column, and the column operations that take a value).
//!
//! The step's text is parsed, never evaluated. A step whose M does not have one of these shapes
//! (typed in, or edited in the formula bar) has no settings: it is edited as M.

use serde_json::{Value as Json, json};

use super::ast::{BinOp, Expr, ListItem, UnOp};
use super::parser::parse;
use super::value::Value;
use crate::Limits;

/// Longest step text read for settings, in bytes.
const MAX_TEXT: usize = 64 << 10;
/// Most list items read.
const MAX_ITEMS: usize = 4_096;

/// The settings of a step (`{kind, …}`) and the step it reads (its first argument), when its M
/// has one of the shapes the editor writes.
pub fn settings(step: &str, limits: &Limits) -> Option<(String, Json)> {
    if step.len() > MAX_TEXT {
        return None;
    }
    let e = parse(step, limits).ok()?;
    let (f, args) = call(&e)?;
    let first = args.first()?;
    let prev = match ident(first) {
        Some(p) => p.to_string(),
        // Merge columns in place reads the step through Table.TransformColumnTypes
        None if f == "Table.CombineColumns" => match call(first)? {
            ("Table.TransformColumnTypes", [p, _]) => ident(p)?.to_string(),
            _ => return None,
        },
        None => return None,
    };
    let rest = args.get(1..)?;
    let v = match f {
        "Table.SelectRows" => select_rows(rest)?,
        "Table.ReplaceValue" => replace_value(rest)?,
        "Table.Group" => group(rest)?,
        "Table.NestedJoin" => nested_join(rest)?,
        "Table.SplitColumn" => split(rest)?,
        "Table.AddColumn" => add_column(rest)?,
        "Table.Skip" => json!({"kind": "remove_top_rows", "count": count(rest.first()?)?}),
        "Table.RemoveLastN" => json!({"kind": "remove_bottom_rows", "count": count(rest.first()?)?}),
        "Table.FirstN" => json!({"kind": "keep_top_rows", "count": count(rest.first()?)?}),
        "Table.LastN" => json!({"kind": "keep_bottom_rows", "count": count(rest.first()?)?}),
        "Table.Range" => {
            let [a, b] = rest else { return None };
            json!({"kind": "keep_range", "first": count(a)?.checked_add(1)?, "count": count(b)?})
        }
        "Table.AlternateRows" => {
            let [a, b, c] = rest else { return None };
            json!({"kind": "remove_alternate_rows", "first": count(a)?.checked_add(1)?,
                "remove": count(b)?, "keep": count(c)?})
        }
        "Table.ReplaceErrorValues" => {
            let [pairs] = rest else { return None };
            let mut columns = Vec::new();
            let mut value = None;
            for p in list(pairs)? {
                let pair = list(p)?;
                let [c, v] = pair.as_slice() else { return None };
                let v = literal(v)?;
                if value.as_ref().is_some_and(|x| x != &v) {
                    return None;
                }
                columns.push(text(c)?);
                value = Some(v);
            }
            json!({"kind": "replace_errors", "columns": columns, "value": value?})
        }
        "Table.AddIndexColumn" => {
            let [n, start, step, ..] = rest else { return None };
            json!({"kind": "add_index", "name": text(n)?, "start": int(start)?, "step": int(step)?})
        }
        "Table.CombineColumns" => combine(rest)?,
        "Table.TransformColumns" => transform_columns(rest)?,
        _ => return None,
    };
    Some((prev, v))
}

fn call(e: &Expr) -> Option<(&str, &[Expr])> {
    match e {
        Expr::Call(f, args) => Some((ident(f)?, args.as_slice())),
        _ => None,
    }
}

fn ident(e: &Expr) -> Option<&str> {
    match e {
        Expr::Ident(n) => Some(n),
        _ => None,
    }
}

fn text(e: &Expr) -> Option<String> {
    match e {
        Expr::Const(Value::Text(t)) => Some(t.to_string()),
        _ => None,
    }
}

fn number(e: &Expr) -> Option<f64> {
    match e {
        Expr::Const(Value::Number(n)) => Some(n.f()).filter(|x| x.is_finite()),
        Expr::Unary(UnOp::Neg, x) => number(x).map(|n| -n),
        _ => None,
    }
}

fn int(e: &Expr) -> Option<i64> {
    let n = number(e)?;
    // whole numbers within i64's exact range
    (n.fract() == 0.0 && n.abs() < 9.0e15).then_some(n as i64)
}

fn count(e: &Expr) -> Option<u64> {
    u64::try_from(int(e)?).ok()
}

fn list(e: &Expr) -> Option<Vec<&Expr>> {
    match e {
        Expr::List(items) if items.len() <= MAX_ITEMS => items
            .iter()
            .map(|i| match i {
                ListItem::One(x) => Some(x),
                ListItem::Range(..) => None,
            })
            .collect(),
        _ => None,
    }
}

fn texts(e: &Expr) -> Option<Vec<String>> {
    list(e)?.into_iter().map(text).collect()
}

/// A literal as `query.apply_step` takes it (`#date(…)` / `#datetime(…)` as objects).
fn literal(e: &Expr) -> Option<Json> {
    match e {
        Expr::Const(Value::Null) => Some(Json::Null),
        Expr::Const(Value::Logical(b)) => Some(json!(b)),
        Expr::Const(Value::Text(t)) => Some(json!(t.to_string())),
        Expr::Const(Value::Number(_)) | Expr::Unary(UnOp::Neg, _) => number(e).map(|n| json!(n)),
        Expr::Call(f, args) => {
            let n: Vec<u64> = args.iter().map(count).collect::<Option<_>>()?;
            match (ident(f)?, n.as_slice()) {
                ("#date", [y, m, d]) => Some(json!({"date": format!("{y:04}-{m:02}-{d:02}")})),
                ("#datetime", [y, m, d, h, mi, s]) => Some(json!({
                    "datetime": format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}")
                })),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The body of `each …`.
fn each(e: &Expr) -> Option<&Expr> {
    match e {
        Expr::Func(params, body) if params.len() == 1 && params.first().is_some_and(|p| &*p.name == "_") => {
            Some(body)
        }
        _ => None,
    }
}

/// `[Column]` inside `each`.
fn field(e: &Expr) -> Option<String> {
    match e {
        Expr::Field(of, name, false) if ident(of) == Some("_") => Some(name.to_string()),
        _ => None,
    }
}

/// One filter comparison: (column, op, value).
fn condition(e: &Expr) -> Option<(String, &'static str, Json)> {
    match e {
        Expr::Binary(op, a, b) => {
            let column = field(a)?;
            let v = literal(b)?;
            let op = match (op, v.is_null()) {
                (BinOp::Eq, true) => "is_null",
                (BinOp::Ne, true) => "is_not_null",
                (BinOp::Eq, false) => "equals",
                (BinOp::Ne, false) => "not_equals",
                (BinOp::Gt, _) => "greater",
                (BinOp::Ge, _) => "greater_or_equal",
                (BinOp::Lt, _) => "less",
                (BinOp::Le, _) => "less_or_equal",
                _ => return None,
            };
            Some((column, op, v))
        }
        Expr::Unary(UnOp::Not, x) => {
            let (c, op, v) = condition(x)?;
            (op == "contains").then_some((c, "not_contains", v))
        }
        Expr::Call(..) => {
            let (f, args) = call(e)?;
            let [a, b] = args else { return None };
            let op = match f {
                "Text.Contains" => "contains",
                "Text.StartsWith" => "begins_with",
                "Text.EndsWith" => "ends_with",
                _ => return None,
            };
            Some((field(a)?, op, json!(text(b)?)))
        }
        _ => None,
    }
}

fn select_rows(rest: &[Expr]) -> Option<Json> {
    let [f] = rest else { return None };
    let (column, op, value) = condition(each(f)?)?;
    let mut v = json!({"kind": "filter_rows", "column": column, "op": op});
    if !op.starts_with("is_") {
        v["value"] = value;
    }
    Some(v)
}

fn replace_value(rest: &[Expr]) -> Option<Json> {
    let [find, replace, replacer, columns] = rest else {
        return None;
    };
    if !matches!(ident(replacer)?, "Replacer.ReplaceText" | "Replacer.ReplaceValue") {
        return None;
    }
    Some(
        json!({"kind": "replace_values", "columns": texts(columns)?, "find": literal(find)?,
        "replace": literal(replace)?}),
    )
}

fn group(rest: &[Expr]) -> Option<Json> {
    let [keys, aggs] = rest else { return None };
    let mut out = Vec::new();
    for a in list(aggs)? {
        let items = list(a)?;
        let name = text(items.first()?)?;
        let body = each(items.get(1)?)?;
        let agg = match body {
            Expr::Ident(n) if &**n == "_" => json!({"name": name, "op": "all_rows"}),
            _ => {
                let (f, args) = call(body)?;
                match (f, args) {
                    ("Table.RowCount", [x]) if ident(x) == Some("_") => json!({"name": name, "op": "count"}),
                    ("Table.RowCount", [x]) => {
                        let (g, inner) = call(x)?;
                        if g != "Table.Distinct"
                            || inner.len() != 1
                            || inner.first().and_then(ident) != Some("_")
                        {
                            return None;
                        }
                        json!({"name": name, "op": "count_distinct"})
                    }
                    (f, [x]) => {
                        let op = match f {
                            "List.Sum" => "sum",
                            "List.Average" => "average",
                            "List.Median" => "median",
                            "List.Min" => "min",
                            "List.Max" => "max",
                            _ => return None,
                        };
                        json!({"name": name, "op": op, "column": field(x)?})
                    }
                    _ => return None,
                }
            }
        };
        out.push(agg);
    }
    Some(json!({"kind": "group_by", "keys": texts(keys)?, "aggregations": out}))
}

fn nested_join(rest: &[Expr]) -> Option<Json> {
    let [keys, other, other_keys, column, kind] = rest else {
        return None;
    };
    let join = match ident(kind)? {
        "JoinKind.LeftOuter" => "left_outer",
        "JoinKind.RightOuter" => "right_outer",
        "JoinKind.FullOuter" => "full_outer",
        "JoinKind.Inner" => "inner",
        "JoinKind.LeftAnti" => "left_anti",
        "JoinKind.RightAnti" => "right_anti",
        _ => return None,
    };
    Some(
        json!({"kind": "merge", "other": ident(other)?, "keys": texts(keys)?,
        "other_keys": texts(other_keys)?, "join": join, "column": text(column)?}),
    )
}

fn split(rest: &[Expr]) -> Option<Json> {
    let [column, splitter, names] = rest else {
        return None;
    };
    let (f, args) = call(splitter)?;
    if f != "Splitter.SplitTextByDelimiter" {
        return None;
    }
    let delimiter = text(args.first()?)?;
    Some(
        json!({"kind": "split_column", "column": text(column)?, "delimiter": delimiter,
        "parts": list(names)?.len()}),
    )
}

/// The parts of `a & b & c`, in order.
fn concat<'a>(e: &'a Expr, out: &mut Vec<&'a Expr>) {
    match e {
        Expr::Binary(BinOp::Concat, a, b) => {
            concat(a, out);
            concat(b, out);
        }
        other => out.push(other),
    }
}

fn add_column(rest: &[Expr]) -> Option<Json> {
    let name = text(rest.first()?)?;
    let body = each(rest.get(1)?)?;
    if let Some(v) = conditional(&name, body) {
        return Some(v);
    }
    if let Some(v) = merged_texts(&name, body) {
        return Some(v);
    }
    if rest.len() == 3
        && let Some((op, column, mut v)) = column_op(body)
    {
        v["kind"] = json!("column_op");
        v["op"] = json!(op);
        v["columns"] = json!([column]);
        v["add"] = json!(true);
        v["name"] = json!(name);
        return Some(v);
    }
    // a custom column: its formula is the M text, which settings do not keep
    None
}

fn conditional(name: &str, body: &Expr) -> Option<Json> {
    let mut rules = Vec::new();
    let mut e = body;
    while let Expr::If(c, then, otherwise) = e {
        let (column, op, value) = condition(c)?;
        let mut r = json!({"column": column, "op": op, "output": literal(then)?});
        if !op.starts_with("is_") {
            r["value"] = value;
        }
        rules.push(r);
        e = otherwise;
        if rules.len() > 100 {
            return None;
        }
    }
    if rules.is_empty() {
        return None;
    }
    Some(json!({"kind": "conditional_column", "name": name, "rules": rules, "otherwise": literal(e)?}))
}

/// `Text.Combine({Text.From([A]), …}, "d")` (Add column › Merge columns).
fn merged_texts(name: &str, body: &Expr) -> Option<Json> {
    let (f, args) = call(body)?;
    let [parts, d] = args else { return None };
    if f != "Text.Combine" {
        return None;
    }
    let columns: Vec<String> = list(parts)?
        .into_iter()
        .map(|p| {
            let (g, a) = call(p)?;
            (g == "Text.From" && a.len() == 1).then(|| a.first().and_then(field))?
        })
        .collect::<Option<_>>()?;
    Some(
        json!({"kind": "merge_columns", "columns": columns, "delimiter": text(d)?, "name": name,
        "add": true}),
    )
}

fn combine(rest: &[Expr]) -> Option<Json> {
    let [columns, combiner, name] = rest else {
        return None;
    };
    let (f, args) = call(combiner)?;
    if f != "Combiner.CombineTextByDelimiter" {
        return None;
    }
    Some(
        json!({"kind": "merge_columns", "columns": texts(columns)?, "delimiter": text(args.first()?)?,
        "name": text(name)?, "add": false}),
    )
}

/// A column operation that takes a value: (op, column read, its values `{text|count|start|number}`).
fn column_op(body: &Expr) -> Option<(&'static str, String, Json)> {
    if let Expr::Binary(op, a, b) = body {
        let (x, n) = (field(a), number(b));
        let numeric = |name: &'static str| Some((name, x.clone()?, json!({"number": n?})));
        return match op {
            BinOp::Add => numeric("add"),
            BinOp::Sub => numeric("subtract"),
            BinOp::Mul => numeric("multiply"),
            BinOp::Div => numeric("divide"),
            BinOp::Concat => {
                let mut parts = Vec::new();
                concat(body, &mut parts);
                let [a, b] = parts.as_slice() else { return None };
                match (text(a), field(a), text(b), field(b)) {
                    (Some(p), _, _, Some(c)) => Some(("prefix", c, json!({"text": p}))),
                    (_, Some(c), Some(s), _) => Some(("suffix", c, json!({"text": s}))),
                    _ => None,
                }
            }
            _ => None,
        };
    }
    let (f, args) = call(body)?;
    let column = field(args.first()?)?;
    let v = match (f, args.get(1..)?) {
        ("Text.Start", [n]) => ("first_chars", json!({"count": count(n)?})),
        ("Text.End", [n]) => ("last_chars", json!({"count": count(n)?})),
        ("Text.Middle", [s, n]) => ("text_range", json!({"start": count(s)?, "count": count(n)?})),
        ("Text.BeforeDelimiter", [d]) => ("before_delimiter", json!({"text": text(d)?})),
        ("Text.AfterDelimiter", [d]) => ("after_delimiter", json!({"text": text(d)?})),
        ("Number.Round", [d]) => ("round", json!({"count": count(d)?})),
        ("Text.BetweenDelimiters", [s, e]) => {
            ("between_delimiters", json!({"text": text(s)?, "end": text(e)?}))
        }
        ("Number.IntegerDivide", [n]) => ("integer_divide", json!({"number": number(n)?})),
        ("Number.Mod", [n]) => ("modulo", json!({"number": number(n)?})),
        ("Number.Power", [n]) => ("power", json!({"number": number(n)?})),
        _ => return None,
    };
    Some((v.0, column, v.1))
}

/// `Table.TransformColumns(prev, {{"C", each …, type}, …})` of one operation that takes a value.
fn transform_columns(rest: &[Expr]) -> Option<Json> {
    let [items] = rest else { return None };
    let mut columns = Vec::new();
    let mut found: Option<(&'static str, Json)> = None;
    for it in list(items)? {
        let parts = list(it)?;
        let c = text(parts.first()?)?;
        let body = each(parts.get(1)?)?;
        // the operation reads `_`: read it as a field of `_` named after the column
        let (op, _, v) = column_op(&rename_underscore(body)?)?;
        if found.as_ref().is_some_and(|(o, x)| *o != op || x != &v) {
            return None;
        }
        found = Some((op, v));
        columns.push(c);
    }
    let (op, mut v) = found?;
    v["kind"] = json!("column_op");
    v["op"] = json!(op);
    v["columns"] = json!(columns);
    v["add"] = json!(false);
    Some(v)
}

/// `body` with the bare `_` written as `[_]` (one level: the shapes of [`column_op`]).
fn rename_underscore(body: &Expr) -> Option<Expr> {
    let bare = |e: &Expr| -> Option<Expr> {
        match e {
            Expr::Ident(n) if &**n == "_" => {
                Some(Expr::Field(Box::new(Expr::Ident("_".into())), "_".into(), false))
            }
            Expr::Const(v) => Some(Expr::Const(v.clone())),
            Expr::Unary(UnOp::Neg, x) => match &**x {
                Expr::Const(v) => Some(Expr::Unary(UnOp::Neg, Box::new(Expr::Const(v.clone())))),
                _ => None,
            },
            _ => None,
        }
    };
    match body {
        Expr::Binary(op, a, b) => Some(Expr::Binary(*op, Box::new(bare(a)?), Box::new(bare(b)?))),
        Expr::Call(f, args) => Some(Expr::Call(
            Box::new(Expr::Ident(ident(f)?.into())),
            args.iter().map(bare).collect::<Option<_>>()?,
        )),
        other => bare(other),
    }
}
