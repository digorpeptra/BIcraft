//! Column variations (auto date/time): `'Sales'[OrderDate].[Year]` reads the column `Year` of the
//! table the column's variation points to. The verdicts below are the engine's (Power BI Desktop
//! 2.158.1304.0, Analysis Services 17.0.87.18) on the same shapes, recorded with the maintainers'
//! development oracles on a model built like Desktop's auto date/time (see
//! docs/formats/dax-functions.md).

use bicraft_model::{Column, DataType, Measure, Relationship, SemanticModel, Table, Variation};

use crate::ast::Expr;
use crate::deps::{ObjectRef, expression_dependencies};
use crate::types::{ScalarType, ValueType};
use crate::{Context, ExprKind, Severity, Validation, Validator, parse_expression, validate, validate_as};

const LDT: &str = "LocalDateTable_1";

fn col(name: &str, data_type: DataType) -> Column {
    Column {
        name: name.into(),
        data_type,
        ..Default::default()
    }
}

fn model() -> SemanticModel {
    let mut order_date = col("OrderDate", DataType::DateTime);
    order_date.set_variations(&[Variation {
        name: "Variation".into(),
        relationship: Some("rel-ldt".into()),
        default_hierarchy: Some((LDT.into(), "Date Hierarchy".into())),
        is_default: true,
        ..Default::default()
    }]);
    let table = |name: &str, columns: Vec<Column>, measures: Vec<Measure>| Table {
        name: name.into(),
        columns,
        measures,
        ..Default::default()
    };
    SemanticModel {
        tables: vec![
            table(
                "Sales",
                vec![
                    order_date,
                    col("Quantity", DataType::Int64),
                    col("UnitPrice", DataType::Decimal),
                ],
                vec![Measure {
                    name: "Total".into(),
                    expression: "SUM(Sales[Quantity])".into(),
                    ..Default::default()
                }],
            ),
            table("Customer", vec![col("BirthDate", DataType::DateTime)], vec![]),
            table(
                LDT,
                vec![
                    col("Date", DataType::DateTime),
                    col("Year", DataType::Int64),
                    col("MonthNo", DataType::Int64),
                    col("Month", DataType::String),
                    col("QuarterNo", DataType::Int64),
                    col("Quarter", DataType::String),
                    col("Day", DataType::Int64),
                ],
                vec![],
            ),
        ],
        relationships: vec![Relationship {
            name: "rel-ldt".into(),
            from_table: "Sales".into(),
            from_column: "OrderDate".into(),
            to_table: LDT.into(),
            to_column: "Date".into(),
            is_active: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn check(m: &SemanticModel, text: &str, table: &str, kind: ExprKind) -> Validation {
    let ctx = Context {
        model: Some(m),
        table: Some(table),
    };
    let v = validate_as(text, &ctx, kind);
    // The whole-model validator gives the same answer.
    let mut validator = Validator::new(m);
    let w = validator.validate_as(text, Some(table), kind);
    assert_eq!(v.diagnostics, w.diagnostics, "{text}");
    v
}

fn errors(v: &Validation) -> Vec<&'static str> {
    v.diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| d.code)
        .collect()
}

#[test]
fn parses_variation_references() {
    let text = "MAX('Sales'[OrderDate] . [Year])";
    let (expr, diags) = parse_expression(text);
    assert!(diags.is_empty(), "{diags:?}");
    let Some(Expr::Call { args, .. }) = expr else {
        panic!("not a call");
    };
    let Some(Some(Expr::Column {
        table,
        name,
        variation: Some(v),
        span,
        ..
    })) = args.first()
    else {
        panic!("not a variation reference: {args:?}");
    };
    assert_eq!(
        (table.as_deref(), name.as_str(), v.name.as_str()),
        (Some("Sales"), "OrderDate", "Year")
    );
    assert_eq!(&text[span.start..span.end], "'Sales'[OrderDate] . [Year]");
    assert_eq!(&text[v.span.start..v.span.end], "[Year]");

    for ok in [
        "Sales[OrderDate].[Date]",
        "[OrderDate].[Year]",
        "'My T'[C].[Month No]",
    ] {
        assert!(parse_expression(ok).0.is_some(), "{ok}");
    }
    // Only `.[name]` after a column, one level deep.
    for bad in [
        "MAX('Sales'[OrderDate].Year)",
        "MAX('Sales'[OrderDate].[Date].[Year])",
        "'Sales'.[Year]",
        "1 .[Year]",
        "Sales[OrderDate].",
        "SUM(Sales[OrderDate]).[Year]",
    ] {
        let (expr, diags) = parse_expression(bad);
        assert!(expr.is_none(), "{bad} parses");
        assert!(diags.iter().any(|d| d.severity == Severity::Error), "{bad}");
    }
    // A number's dot is still part of the number.
    assert!(parse_expression("1.5 + .5").0.is_some());
    assert!(matches!(
        parse_expression("Sales[Quantity]").0,
        Some(Expr::Column { variation: None, .. })
    ));
}

#[test]
fn engine_accepted_variation_measures_validate() {
    let m = model();
    let cases: &[(&str, ScalarType)] = &[
        (
            "CALCULATE(SUM(Sales[Quantity]), DATESYTD('Sales'[OrderDate].[Date]))",
            ScalarType::Integer,
        ),
        (
            "CALCULATE(SUM(Sales[Quantity]), DATESYTD(Sales[OrderDate].[Date]))",
            ScalarType::Integer,
        ),
        ("MAX('Sales'[OrderDate].[Year])", ScalarType::Integer),
        ("MAX('Sales'[OrderDate].[Month])", ScalarType::String),
        ("MAX('Sales'[OrderDate].[MonthNo])", ScalarType::Integer),
        ("MAX('Sales'[OrderDate].[Quarter])", ScalarType::String),
        ("MAX('Sales'[OrderDate].[QuarterNo])", ScalarType::Integer),
        ("MAX('Sales'[OrderDate].[Day])", ScalarType::Integer),
        ("MAX('Sales'[OrderDate].[Date])", ScalarType::DateTime),
        ("MAX('Sales'[OrderDate] .[Year])", ScalarType::Integer),
        ("MAX('Sales'[OrderDate].[year])", ScalarType::Integer),
        ("MAX([OrderDate].[Year])", ScalarType::Integer),
        (
            "COUNTROWS(VALUES('Sales'[OrderDate].[Year]))",
            ScalarType::Integer,
        ),
        (
            "CALCULATE(SUM(Sales[Quantity]), 'Sales'[OrderDate].[Year] = 2024)",
            ScalarType::Integer,
        ),
        ("SUMX(Sales, Sales[OrderDate].[MonthNo])", ScalarType::Integer),
        (
            "SUMX('LocalDateTable_1', Sales[OrderDate].[Year])",
            ScalarType::Integer,
        ),
        (
            "COUNTROWS(FILTER(ALL(Sales), Sales[OrderDate].[Year] = 2024))",
            ScalarType::Integer,
        ),
        (
            "TOTALYTD(SUM(Sales[Quantity]), 'Sales'[OrderDate].[Date])",
            ScalarType::Integer,
        ),
        (
            "CALCULATE(SUM(Sales[Quantity]), SAMEPERIODLASTYEAR('Sales'[OrderDate].[Date]))",
            ScalarType::Integer,
        ),
        ("DISTINCTCOUNT(Sales[OrderDate].[Month])", ScalarType::Integer),
        (
            "CALCULATE(SUM(Sales[Quantity]), Sales[OrderDate].[Year] = 2024, Sales[OrderDate].[Month] = \"January\")",
            ScalarType::Integer,
        ),
        (
            "CALCULATE(SUM(Sales[Quantity]), Sales[OrderDate].[Year] = 2024 && 'LocalDateTable_1'[MonthNo] = 1)",
            ScalarType::Integer,
        ),
        (
            "CALCULATE(SUM(Sales[Quantity]), ALL(Sales[OrderDate].[Year]))",
            ScalarType::Integer,
        ),
        ("SELECTEDVALUE(Sales[OrderDate].[Year])", ScalarType::Integer),
        (
            "CALCULATE(SUM(Sales[Quantity]), DATEADD(Sales[OrderDate].[Date], -1, YEAR))",
            ScalarType::Integer,
        ),
        (
            "COUNTROWS(FILTER(VALUES(Sales[OrderDate].[Year]), Sales[OrderDate].[Year] > 2000))",
            ScalarType::Integer,
        ),
    ];
    for (text, ty) in cases {
        let v = check(&m, text, "Sales", ExprKind::Measure);
        assert!(v.valid, "{text}: {:?}", v.diagnostics);
        assert_eq!(v.result_type, Some(ValueType::Scalar(*ty)), "{text}");
    }
    for (table, text) in [
        ("Sales", "Sales[OrderDate].[Year]"),
        ("Sales", "[OrderDate].[MonthNo]"),
    ] {
        let v = check(&m, text, table, ExprKind::CalculatedColumn);
        assert!(v.valid, "{text}: {:?}", v.diagnostics);
        assert_eq!(v.result_type, Some(ValueType::Scalar(ScalarType::Integer)));
    }
}

#[test]
fn engine_rejected_variation_measures_are_errors() {
    let m = model();
    let cases: &[(&str, &str)] = &[
        ("MAX('Sales'[OrderDate].[Foo])", "unknown-variation"),
        ("MAX('Sales'[OrderDate].[Variation])", "unknown-variation"),
        ("MAX('Sales'[OrderDate].[Date Hierarchy])", "unknown-variation"),
        ("MAX('Customer'[BirthDate].[Year])", "column-without-variations"),
        ("MAX('Sales'[Quantity].[Year])", "column-without-variations"),
        (
            "MAX('LocalDateTable_1'[Date].[Year])",
            "column-without-variations",
        ),
        ("Sales[Total].[Year]", "variation-on-measure"),
        ("[Total].[Year]", "variation-on-measure"),
        ("Sales[OrderDate].[Year]", "no-row-context"),
        ("SUMX(Customer, Sales[OrderDate].[Year])", "no-row-context"),
        (
            "CALCULATE(SUM(Sales[Quantity]), Sales[OrderDate].[Year] = 2024 && Sales[Quantity] > 1)",
            "filter-multiple-tables",
        ),
        ("MAX('Sales'[Nope].[Year])", "unknown-column"),
        ("MAX('Nope'[OrderDate].[Year])", "unknown-table"),
    ];
    for (text, code) in cases {
        let v = check(&m, text, "Sales", ExprKind::Measure);
        assert!(!v.valid, "{text} is valid");
        assert_eq!(errors(&v), [*code], "{text}: {:?}", v.diagnostics);
        for d in &v.diagnostics {
            assert!(!d.message.contains("  "), "{}", d.message);
        }
    }
    let v = check(&m, "MAX('Sales'[OrderDate].[Foo])", "Sales", ExprKind::Measure);
    let msg = &v.diagnostics[0].message;
    assert!(
        msg.contains("'LocalDateTable_1'") && msg.contains("Year"),
        "{msg}"
    );
    assert_eq!(v.diagnostics[0].column, 24, "points at [Foo]");

    let v = check(
        &m,
        "Sales[OrderDate].[Year]",
        "Customer",
        ExprKind::CalculatedColumn,
    );
    assert_eq!(errors(&v), ["no-row-context"]);
    let v = check(
        &m,
        "Customer[BirthDate].[Year]",
        "Customer",
        ExprKind::CalculatedColumn,
    );
    assert_eq!(errors(&v), ["column-without-variations"]);
    // RELATED of a variation: the engine finds no relationship for it.
    let v = check(
        &m,
        "SUMX(Sales, RELATED(Sales[OrderDate].[Year]))",
        "Sales",
        ExprKind::Measure,
    );
    assert!(!v.valid);
}

#[test]
fn variations_without_a_model_or_metadata() {
    // No model: syntax only.
    let v = validate("MAX('Sales'[OrderDate].[Year])", &Context::default());
    assert!(v.valid, "{:?}", v.diagnostics);
    // A model without variation metadata (e.g. read from a source that drops it): a precise
    // diagnostic, not a parse error.
    let mut m = model();
    for t in &mut m.tables {
        for c in &mut t.columns {
            c.set_variations(&[]);
        }
    }
    let v = check(&m, "MAX('Sales'[OrderDate].[Year])", "Sales", ExprKind::Measure);
    assert_eq!(errors(&v), ["column-without-variations"]);
    // A variation whose relationship and default hierarchy lead nowhere.
    let mut m = model();
    m.relationships.clear();
    m.tables.retain(|t| t.name != LDT);
    let v = check(&m, "MAX('Sales'[OrderDate].[Year])", "Sales", ExprKind::Measure);
    assert_eq!(errors(&v), ["unknown-variation"]);
}

#[test]
fn variation_dependencies_include_the_variation_column() {
    let m = model();
    let mut deps = expression_dependencies(&m, "MAX('Sales'[OrderDate].[Year])", Some("Sales"), false);
    deps.sort_by_key(ToString::to_string);
    let column = |t: &str, c: &str| ObjectRef::Column {
        table: t.into(),
        name: c.into(),
    };
    assert_eq!(deps, [column(LDT, "Year"), column("Sales", "OrderDate")]);
}
