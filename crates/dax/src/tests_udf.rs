//! The model's user-defined functions: signatures, definitions, call sites, inline expansion.

use bicraft_model::{Column, DataType, Measure, SemanticModel, Table};
use serde_json::json;

use crate::udf::{call_cycle, compose, inline_calls, param, parse_function, validate_function};
use crate::{Context, Severity, validate};

fn model() -> SemanticModel {
    let col = |n: &str, t| Column {
        name: n.into(),
        data_type: t,
        ..Default::default()
    };
    let mut m = SemanticModel {
        name: "Test".into(),
        tables: vec![Table {
            name: "Sales".into(),
            columns: vec![col("Amount", DataType::Double), col("Qty", DataType::Int64)],
            measures: vec![Measure {
                name: "Total".into(),
                expression: "SUM(Sales[Amount])".into(),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    m.extra.insert(
        "functions".into(),
        json!([
            {"name": "AddTax", "expression": "(amount : NUMERIC, taxRate : NUMERIC = 0.1) => amount * (1 + taxRate)"},
            {"name": "RowsOf", "expression": "(t : TABLE EXPR) => COUNTROWS(t)"},
            {"name": "SumOf", "expression": "(c : COLUMNREF) => SUM(c)"},
            {"name": "Twice", "expression": "(m : MEASUREREF) => m * 2"},
            {"name": "Local.Nested", "expression": "(x) => AddTax(x) + 1"},
        ]),
    );
    m
}

fn codes(text: &str) -> Vec<&'static str> {
    let m = model();
    let v = validate(
        text,
        &Context {
            model: Some(&m),
            table: Some("Sales"),
        },
    );
    v.diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| d.code)
        .collect()
}

#[test]
fn parses_a_signature_with_the_effective_types_and_modes() {
    let s =
        parse_function("( amount : NUMERIC, t : TABLE EXPR, c : COLUMNREF, x, y = 2 ) => amount").unwrap();
    type Row<'a> = (&'a str, &'a str, Option<&'a str>, &'a str, Option<&'a str>);
    let p: Vec<Row<'_>> = s
        .params
        .iter()
        .map(|p| {
            (
                p.name.as_str(),
                p.kind.as_str(),
                p.subtype.as_deref(),
                p.mode.as_str(),
                p.default.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        p,
        [
            ("amount", "SCALAR", Some("NUMERIC"), "VAL", None),
            ("t", "TABLE", None, "EXPR", None),
            ("c", "COLUMNREF", None, "EXPR", None),
            ("x", "ANYVAL", None, "VAL", None),
            ("y", "ANYVAL", None, "VAL", Some("2")),
        ]
    );
    assert_eq!(s.body, "amount");
    assert_eq!(s.min_args(), 4);
    assert!(parse_function("amount * 2").is_err());
    assert!(parse_function("(x) =>").is_err());
}

#[test]
fn composes_a_signature_from_parts() {
    let p = vec![
        param("amount", Some("scalar"), Some("numeric"), Some("val"), None).unwrap(),
        param("rate", None, None, None, Some("0.1")).unwrap(),
    ];
    assert_eq!(
        compose(&p, " amount * (1 + rate) "),
        "(amount : SCALAR NUMERIC VAL, rate = 0.1) => amount * (1 + rate)"
    );
    assert!(param("x", Some("number"), None, None, None).is_err());
}

#[test]
fn validates_a_definition_against_the_model_with_positions_on_the_expression() {
    let m = model();
    assert!(validate_function("F", "(x : NUMERIC) => x + [Total]", Some(&m)).valid);
    let v = validate_function("F", "(x) => x + [Nope]", Some(&m));
    assert!(!v.valid);
    let d = v
        .diagnostics
        .iter()
        .find(|d| d.code == "unknown-reference")
        .unwrap();
    assert_eq!((d.line, d.column), (1, 12));
    // a built-in name, a bad hint, a self call
    assert!(!validate_function("SUM", "() => 1", Some(&m)).valid);
    assert!(!validate_function("F", "(x : NUMBER) => x", Some(&m)).valid);
    assert!(!validate_function("F", "(x) => F(x)", Some(&m)).valid);
}

#[test]
fn finds_cycles_through_other_model_functions() {
    let mut m = model();
    m.extra.insert(
        "functions".into(),
        json!([{"name": "A", "expression": "(x) => B(x)"}, {"name": "B", "expression": "(x) => x"}]),
    );
    assert_eq!(
        call_cycle(&m, "B", "(x) => A(x)"),
        Some(vec!["B".into(), "A".into(), "B".into()])
    );
    assert_eq!(call_cycle(&m, "B", "(x) => x + 1"), None);
}

#[test]
fn checks_call_sites_arity_and_argument_kinds() {
    assert!(codes("AddTax([Total])").is_empty());
    assert!(codes("AddTax([Total], 0.2)").is_empty());
    assert!(codes("Local.Nested(1)").is_empty());
    assert_eq!(codes("AddTax()"), ["wrong-argument-count"]);
    assert_eq!(codes("AddTax(1, 2, 3)"), ["wrong-argument-count"]);
    assert_eq!(codes("AddTax(Sales)"), ["argument-type"]);
    assert_eq!(codes("RowsOf(1)"), ["argument-type"]);
    assert!(codes("RowsOf(FILTER(Sales, Sales[Qty] > 1))").is_empty());
    assert!(codes("SumOf(Sales[Amount])").is_empty());
    assert_eq!(codes("SumOf([Total])"), ["argument-type"]);
    assert!(codes("Twice([Total])").is_empty());
    assert_eq!(codes("Twice(Sales[Amount])"), ["argument-type"]);
    // a variable may hold anything: not checked
    assert!(codes("VAR t = Sales RETURN RowsOf(t)").is_empty());
}

#[test]
fn inlines_calls_val_as_variables_expr_as_text() {
    let m = model();
    assert_eq!(inline_calls("SUM(Sales[Amount])", &m).unwrap(), None);
    let out = inline_calls("AddTax([Total])", &m).unwrap().unwrap();
    assert_eq!(
        out,
        "(VAR __udf1_0 = ([Total])\nVAR __udf1_1 = (0.1)\nRETURN (__udf1_0 * (1 + __udf1_1)))"
    );
    let out = inline_calls("SumOf(Sales[Amount]) + Twice([Total])", &m)
        .unwrap()
        .unwrap();
    assert_eq!(out, "(SUM(Sales[Amount])) + ([Total] * 2)");
    // a function calling a function: expanded until none is left, and it validates
    let out = inline_calls("Local.Nested(2)", &m).unwrap().unwrap();
    assert!(!out.to_ascii_lowercase().contains("addtax"), "{out}");
    let v = validate(
        &out,
        &Context {
            model: Some(&m),
            table: Some("Sales"),
        },
    );
    assert!(v.valid, "{out}: {:?}", v.diagnostics);
    // mismatched calls and recursion are errors, not panics
    assert!(inline_calls("AddTax(1, 2, 3)", &m).is_err());
    let mut r = model();
    r.extra.insert(
        "functions".into(),
        json!([{"name": "Loop", "expression": "(x) => Loop(x)"}]),
    );
    assert!(inline_calls("Loop(1)", &r).is_err());
}
