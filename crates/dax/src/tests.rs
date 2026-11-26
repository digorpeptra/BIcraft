use bicraft_model::{Column, DataType, Measure, SemanticModel, Table};

use crate::{Context, Severity, validate};

fn model() -> SemanticModel {
    let col = |n: &str, t| Column {
        name: n.into(),
        data_type: t,
        ..Default::default()
    };
    SemanticModel {
        name: "Test".into(),
        tables: vec![
            Table {
                name: "Sales".into(),
                columns: vec![
                    col("Amount", DataType::Double),
                    col("Date", DataType::DateTime),
                    col("ProductKey", DataType::Int64),
                ],
                measures: vec![Measure {
                    name: "Total Sales".into(),
                    expression: "SUM(Sales[Amount])".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Table {
                name: "Product Catalog".into(),
                columns: vec![
                    col("ProductKey", DataType::Int64),
                    col("Category", DataType::String),
                ],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
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
    v.diagnostics.iter().map(|d| d.code).collect()
}

fn ok(text: &str) {
    let m = model();
    let v = validate(
        text,
        &Context {
            model: Some(&m),
            table: Some("Sales"),
        },
    );
    assert!(v.valid, "{text}: {:#?}", v.diagnostics);
    assert!(v.diagnostics.iter().all(|d| d.severity != Severity::Error));
}

#[test]
fn accepts_common_measures() {
    ok("SUM(Sales[Amount])");
    ok("CALCULATE([Total Sales], 'Product Catalog'[Category] = \"Bikes\")");
    ok("DIVIDE([Total Sales], CALCULATE([Total Sales], ALL('Product Catalog')), 0)");
    ok("VAR x = [Total Sales] VAR y = x * 2 RETURN IF(x > 0, y, BLANK())");
    ok("SUMX(FILTER(Sales, Sales[Amount] > 100 && NOT ISBLANK(Sales[Date])), Sales[Amount] * 1.2e0)");
    ok("TOTALYTD([Total Sales], Sales[Date])");
    ok("CALCULATE([Total Sales], DATEADD(Sales[Date], -1, YEAR))");
    ok("RANKX(ALL('Product Catalog'), [Total Sales], , DESC, Dense)");
    ok("'Product Catalog'[Category] IN { \"Bikes\", \"Helmets\" }");
    ok("SUMX(ADDCOLUMNS(Sales, \"Double\", Sales[Amount] * 2), [Double])");
    ok("COUNTROWS({ (1, \"a\"), (2, \"b\") })");
    ok("TRUE && FALSE || NOT(TRUE)");
    ok("\"He said \"\"hi\"\"\" & \"!\"");
    ok("-- comment\n/* block */ 1 + 2 // trailing");
}

#[test]
fn reports_syntax_errors_with_positions() {
    let v = validate("SUM(Sales[Amount]", &Context::default());
    assert!(!v.valid);
    let d = &v.diagnostics[0];
    assert_eq!(d.code, "syntax");
    assert_eq!((d.line, d.column), (1, 18));

    let v = validate("1 +\n  * 2", &Context::default());
    assert_eq!((v.diagnostics[0].line, v.diagnostics[0].column), (2, 3));

    assert_eq!(codes(""), ["empty-expression"]);
    assert_eq!(codes("VAR x = 1"), ["syntax"]);
    assert_eq!(codes("\"open"), ["unterminated-string"]);
    assert_eq!(codes("Sales[Amount"), ["unterminated-bracket"]);
    assert_eq!(codes("1 2"), ["unexpected-token"]);
    assert_eq!(codes("1 # 2"), ["unexpected-character"]);
}

#[test]
fn reports_unknown_references() {
    assert_eq!(codes("SUM(Sales[Amnt])"), ["unknown-column"]);
    assert_eq!(codes("SUM(Sale[Amount])"), ["unknown-table"]);
    assert_eq!(codes("[Nope] + 1"), ["unknown-reference"]);
    assert_eq!(codes("COUNTROWS(Nope)"), ["unknown-table"]);
    // A variable is only visible after its definition.
    assert_eq!(codes("VAR a = b VAR b = 1 RETURN a"), ["unknown-table"]);
}

#[test]
fn calendar_names_are_references() {
    let mut m = model();
    if let Some(t) = m.tables.first_mut() {
        t.extra.insert(
            "calendars".into(),
            serde_json::json!([{"name": "Fiscal Weeks", "calendarColumnGroups": [
                {"timeUnit": "date", "primaryColumn": "Date"}]}]),
        );
    }
    let codes_of = |text: &str| -> Vec<&'static str> {
        let v = validate(
            text,
            &Context {
                model: Some(&m),
                table: Some("Sales"),
            },
        );
        v.diagnostics.iter().map(|d| d.code).collect()
    };
    assert!(codes_of("CLOSINGBALANCEWEEK([Total Sales], 'Fiscal Weeks')").is_empty());
    assert_eq!(
        codes_of("CLOSINGBALANCEWEEK([Total Sales], 'Other Weeks')"),
        ["unknown-table"]
    );
}

#[test]
fn checks_arity_and_flags_unknown_functions() {
    assert_eq!(codes("DIVIDE(1)"), ["wrong-argument-count"]);
    assert_eq!(codes("IF(TRUE(), 1, 2, 3)"), ["wrong-argument-count"]);
    let m = model();
    let v = validate(
        "MYSTERY(1)",
        &Context {
            model: Some(&m),
            table: None,
        },
    );
    assert!(!v.valid, "against a model, unknown functions are errors");
    assert_eq!(v.diagnostics[0].code, "unknown-function");
    assert_eq!(v.diagnostics[0].severity, Severity::Error);
    // Without a model it stays a warning.
    let v = validate("MYSTERY(1)", &Context::default());
    assert!(v.valid, "{:?}", v.diagnostics);
    assert_eq!(v.diagnostics[0].severity, Severity::Warning);
    // A user-defined function of the model is no unknown function.
    let mut m = model();
    m.extra.insert(
        "functions".into(),
        serde_json::json!([{ "name": "Mystery", "expression": "(x) => x" }]),
    );
    let v = validate(
        "MYSTERY(1)",
        &Context {
            model: Some(&m),
            table: None,
        },
    );
    assert!(v.valid, "{:?}", v.diagnostics);
    assert!(v.diagnostics.is_empty(), "{:?}", v.diagnostics);
}

#[test]
fn without_a_model_only_syntax_and_arity_are_checked() {
    let v = validate("SUM(Whatever[Anything])", &Context::default());
    assert!(v.valid);
    assert!(v.diagnostics.is_empty());
}

#[test]
fn hostile_input_does_not_overflow_the_stack() {
    let deep = format!("{}1{}", "(".repeat(100_000), ")".repeat(100_000));
    assert_eq!(codes(&deep), ["too-deep"]);
    let long_chain = vec!["1"; 100_000].join("+");
    assert_eq!(codes(&long_chain), ["too-deep"]);
    // `- ` with a space: `--` would start a comment.
    let negations = format!("{}1", "- ".repeat(100_000));
    assert_eq!(codes(&negations), ["too-deep"]);
    let nots = format!("{}TRUE", "NOT ".repeat(100_000));
    assert_eq!(codes(&nots), ["too-deep"]);
}

#[test]
fn line_and_column_count_characters_not_bytes() {
    let v = validate(
        "\"é\" & [Nope]",
        &Context {
            model: Some(&model()),
            table: None,
        },
    );
    let d = &v.diagnostics[0];
    assert_eq!(d.code, "unknown-reference");
    assert_eq!(d.column, 7);
}

/// The formula bar's signature help: optional params in brackets, the repeating unit then `, …`.
#[test]
fn signatures_read_as_a_reader_writes_them() {
    let text = |n: &str| crate::signature_text(crate::signature(n).unwrap());
    assert_eq!(
        text("DIVIDE"),
        "DIVIDE(numerator, denominator[, alternate_result])"
    );
    assert_eq!(text("SUMX"), "SUMX(table, expression)");
    assert_eq!(text("CALCULATE"), "CALCULATE(expression[, filter], …)");
    assert_eq!(text("SWITCH"), "SWITCH(expression, value, result, …[, else])");
    let all = crate::all_functions();
    assert!(all.len() > 300, "{} functions", all.len());
    assert!(all.windows(2).all(|w| w[0].name < w[1].name), "sorted by name");
}
