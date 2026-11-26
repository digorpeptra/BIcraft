//! Type inference and evaluation-context rules (M6.2). The rules are tested against a small
//! signature table that follows the catalog's conventions, so they do not depend on the exact
//! state of the full catalog; `degrades_without_signatures` covers the catalog-less fallback.

use bicraft_model::{Column, DataType, Measure, Partition, Relationship, SemanticModel, Table};

use crate::types::{Param, ParamKind, Returns, ScalarType, Signature, ValueType};
use crate::{Context, ExprKind, Severity, Validation, validate_with};

const fn p(name: &'static str, kind: ParamKind) -> Param {
    Param {
        name,
        kind,
        optional: false,
        repeating: false,
        group: 0,
    }
}
const fn o(name: &'static str, kind: ParamKind) -> Param {
    Param {
        optional: true,
        ..p(name, kind)
    }
}
const fn r(name: &'static str, kind: ParamKind) -> Param {
    Param {
        repeating: true,
        ..p(name, kind)
    }
}
const fn ro(name: &'static str, kind: ParamKind) -> Param {
    Param {
        optional: true,
        repeating: true,
        ..p(name, kind)
    }
}
const fn sig(
    name: &'static str,
    params: &'static [Param],
    repeat_from: Option<u8>,
    returns: Returns,
) -> Signature {
    Signature {
        name,
        params,
        repeat_from,
        returns,
        category: "test",
        deprecated: false,
        min_args: None,
    }
}

const SCALAR: ParamKind = ParamKind::Scalar(None);
const TABLE: ParamKind = ParamKind::Table;
const COL: ParamKind = ParamKind::ColumnRef;
const ROW: ParamKind = ParamKind::RowExpression;
const R_TABLE: Returns = Returns::Fixed(ValueType::Table);
const R_DEC: Returns = Returns::Fixed(ValueType::Scalar(ScalarType::Decimal));
const R_INT: Returns = Returns::Fixed(ValueType::Scalar(ScalarType::Integer));

static SIGS: &[Signature] = &[
    sig(
        "SELECTCOLUMNS",
        &[
            p("table", TABLE),
            ro("name", ParamKind::NewColumnName),
            ro("expression", ROW),
        ],
        Some(1),
        R_TABLE,
    ),
    sig(
        "CROSSJOIN",
        &[p("table", TABLE), r("table", TABLE)],
        Some(1),
        R_TABLE,
    ),
    sig(
        "SUMMARIZE",
        &[
            p("table", TABLE),
            ro("group_by", COL),
            ro("name", ParamKind::NewColumnName),
            ro("expression", SCALAR),
        ],
        Some(1),
        R_TABLE,
    ),
    sig(
        "TOPN",
        &[p("n", SCALAR), p("table", TABLE), ro("order_by", ROW)],
        Some(2),
        R_TABLE,
    ),
    sig(
        "USERELATIONSHIP",
        &[p("column1", COL), p("column2", COL)],
        None,
        Returns::Fixed(ValueType::Unknown),
    ),
    sig("UNION", &[p("table", TABLE), r("table", TABLE)], Some(1), R_TABLE),
    sig(
        "ROW",
        &[r("name", ParamKind::NewColumnName), r("expression", SCALAR)],
        Some(0),
        R_TABLE,
    ),
    sig(
        "DATATABLE",
        &[
            r("name", ParamKind::NewColumnName),
            r("type", ParamKind::Keyword(&["INTEGER", "STRING"])),
            p("data", ParamKind::Any),
        ],
        Some(0),
        R_TABLE,
    ),
    sig(
        "TOTALYTD",
        &[
            p("expression", SCALAR),
            p("dates", ParamKind::Any),
            o("filter", ParamKind::Filter),
            o("year_end_date", SCALAR),
        ],
        None,
        Returns::SameAsArg(0),
    ),
    sig(
        "MAXX",
        &[p("table", TABLE), p("expression", ROW)],
        None,
        Returns::SameAsArg(1),
    ),
    sig("SUM", &[p("column", COL)], None, Returns::SameAsArg(0)),
    sig(
        "MAX",
        &[p("column_or_scalar1", ParamKind::Any), o("scalar2", SCALAR)],
        None,
        Returns::CommonOfArgs(&[0, 1]),
    ),
    sig("SUMX", &[p("table", TABLE), p("expression", ROW)], None, R_DEC),
    sig("FILTER", &[p("table", TABLE), p("filter", ROW)], None, R_TABLE),
    sig(
        "CALCULATE",
        &[p("expression", SCALAR), ro("filter", ParamKind::Filter)],
        Some(1),
        Returns::SameAsArg(0),
    ),
    sig(
        "CALCULATETABLE",
        &[p("table", TABLE), ro("filter", ParamKind::Filter)],
        Some(1),
        R_TABLE,
    ),
    sig(
        "KEEPFILTERS",
        &[p("expression", ParamKind::Filter)],
        None,
        R_TABLE,
    ),
    sig("COUNTROWS", &[o("table", TABLE)], None, R_INT),
    sig(
        "VALUES",
        &[p("table_or_column", ParamKind::TableOrColumnRef)],
        None,
        R_TABLE,
    ),
    sig(
        "ALL",
        &[ro("table_or_column", ParamKind::TableOrColumnRef)],
        Some(0),
        R_TABLE,
    ),
    sig(
        "ADDCOLUMNS",
        &[
            p("table", TABLE),
            r("name", ParamKind::NewColumnName),
            r("expression", ROW),
        ],
        Some(1),
        R_TABLE,
    ),
    sig("RELATED", &[p("column", COL)], None, Returns::SameAsArg(0)),
    sig(
        "EARLIER",
        &[
            p("column", COL),
            o("number", ParamKind::Scalar(Some(ScalarType::Integer))),
        ],
        None,
        Returns::SameAsArg(0),
    ),
    sig(
        "IF",
        &[
            p("test", ParamKind::Scalar(Some(ScalarType::Boolean))),
            p("then", SCALAR),
            o("else", SCALAR),
        ],
        None,
        Returns::CommonOfArgs(&[1, 2]),
    ),
    sig(
        "SWITCH",
        &[
            p("expression", SCALAR),
            r("value", SCALAR),
            r("result", SCALAR),
            o("else", SCALAR),
        ],
        Some(1),
        Returns::CommonOfArgs(&[2, 3]),
    ),
    sig(
        "DATEADD",
        &[
            p("dates", COL),
            p("number", SCALAR),
            p(
                "interval",
                ParamKind::Keyword(&["DAY", "MONTH", "QUARTER", "YEAR"]),
            ),
        ],
        None,
        R_TABLE,
    ),
    sig(
        "RANKX",
        &[
            p("table", TABLE),
            p("expression", ROW),
            o("value", SCALAR),
            // A sort order also takes 0/1 and booleans, as in the real catalog.
            o("order", ParamKind::KeywordOrScalar(&["ASC", "DESC"])),
            o("ties", ParamKind::Keyword(&["SKIP", "DENSE"])),
        ],
        None,
        R_INT,
    ),
    sig(
        "YEAR",
        &[p("date", ParamKind::Scalar(Some(ScalarType::DateTime)))],
        None,
        R_INT,
    ),
    sig(
        "DIVIDE",
        &[
            p("numerator", SCALAR),
            p("denominator", SCALAR),
            o("alternate", SCALAR),
        ],
        None,
        R_DEC,
    ),
    sig(
        "GENERATE",
        &[p("table1", TABLE), p("table2", TABLE)],
        None,
        R_TABLE,
    ),
    sig(
        "SUMMARIZECOLUMNS",
        &[
            ro("group_by", COL),
            ro("filter", TABLE),
            ro("name", ParamKind::NewColumnName),
            ro("expression", SCALAR),
        ],
        Some(0),
        R_TABLE,
    ),
    sig("DETAILROWS", &[p("measure", ParamKind::Any)], None, R_TABLE),
    sig(
        "BLANK",
        &[],
        None,
        Returns::Fixed(ValueType::Scalar(ScalarType::Variant)),
    ),
];

fn test_sig(name: &str) -> Option<&'static Signature> {
    SIGS.iter().find(|s| s.name.eq_ignore_ascii_case(name))
}

fn no_sig(_: &str) -> Option<&'static Signature> {
    None
}

fn col(name: &str, data_type: DataType) -> Column {
    Column {
        name: name.into(),
        data_type,
        ..Default::default()
    }
}

fn calc(name: &str, expression: &str) -> Column {
    Column {
        name: name.into(),
        expression: Some(expression.into()),
        ..Default::default()
    }
}

fn measure(name: &str, expression: &str) -> Measure {
    Measure {
        name: name.into(),
        expression: expression.into(),
        ..Default::default()
    }
}

pub(crate) fn model() -> SemanticModel {
    SemanticModel {
        name: "Types".into(),
        tables: vec![
            Table {
                name: "Sales".into(),
                columns: vec![
                    col("Amount", DataType::Double),
                    col("Date", DataType::DateTime),
                    col("ProductKey", DataType::Int64),
                    col("Qty", DataType::Int64),
                    col("Price", DataType::Decimal),
                    calc("Line", "Sales[Qty] * Sales[Price]"),
                ],
                measures: vec![
                    measure("Total Sales", "SUM(Sales[Amount])"),
                    measure("Units", "SUM(Sales[Qty])"),
                    measure("Twice", "[Total Sales] * 2"),
                    measure("Label", "\"Sales: \" & [Total Sales]"),
                    measure("Loop A", "[Loop B] + 1"),
                    measure("Loop B", "[Loop A] + 1"),
                ],
                ..Default::default()
            },
            Table {
                name: "Product".into(),
                columns: vec![
                    col("ProductKey", DataType::Int64),
                    col("Category", DataType::String),
                    col("Color", DataType::String),
                ],
                ..Default::default()
            },
            Table {
                name: "Single".into(),
                columns: vec![col("Value", DataType::Int64)],
                ..Default::default()
            },
            Table {
                name: "Calc".into(),
                partitions: vec![Partition {
                    name: "Calc".into(),
                    source_type: "calculated".into(),
                    expression: Some("FILTER(Sales, Sales[Amount] > 0)".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        relationships: vec![Relationship {
            name: "Sales_Product".into(),
            from_table: "Sales".into(),
            from_column: "ProductKey".into(),
            to_table: "Product".into(),
            to_column: "ProductKey".into(),
            is_active: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn run(kind: ExprKind, text: &str) -> Validation {
    let m = model();
    let home = match kind {
        ExprKind::CalculatedTable => None,
        _ => Some("Sales"),
    };
    validate_with(
        text,
        &Context {
            model: Some(&m),
            table: home,
        },
        kind,
        test_sig,
    )
}

fn codes(kind: ExprKind, text: &str, severity: Severity) -> Vec<&'static str> {
    run(kind, text)
        .diagnostics
        .iter()
        .filter(|d| d.severity == severity)
        .map(|d| d.code)
        .collect()
}

fn errors(kind: ExprKind, text: &str) -> Vec<&'static str> {
    codes(kind, text, Severity::Error)
}

fn warnings(kind: ExprKind, text: &str) -> Vec<&'static str> {
    codes(kind, text, Severity::Warning)
}

#[track_caller]
fn clean(kind: ExprKind, text: &str) {
    let v = run(kind, text);
    assert!(
        v.valid && v.diagnostics.is_empty(),
        "{kind:?} `{text}`: {:#?}",
        v.diagnostics
    );
}

#[track_caller]
fn rejects(kind: ExprKind, text: &str, code: &str) {
    let errs = errors(kind, text);
    assert!(
        errs.contains(&code),
        "{kind:?} `{text}`: expected {code}, got {errs:?}"
    );
}

fn ty(kind: ExprKind, text: &str) -> Option<ValueType> {
    run(kind, text).result_type
}

const M: ExprKind = ExprKind::Measure;
const CC: ExprKind = ExprKind::CalculatedColumn;
const CT: ExprKind = ExprKind::CalculatedTable;
const E: ExprKind = ExprKind::Expression;
const RLS: ExprKind = ExprKind::RowFilter;

fn scalar(t: ScalarType) -> Option<ValueType> {
    Some(ValueType::Scalar(t))
}

#[test]
fn infers_literal_and_operator_types() {
    assert_eq!(ty(E, "1"), scalar(ScalarType::Integer));
    assert_eq!(ty(E, "1.5"), scalar(ScalarType::Decimal));
    assert_eq!(ty(E, "1e3"), scalar(ScalarType::Decimal));
    assert_eq!(ty(E, "99999999999999999999"), scalar(ScalarType::Decimal));
    assert_eq!(ty(E, "\"a\""), scalar(ScalarType::String));
    assert_eq!(ty(E, "TRUE"), scalar(ScalarType::Boolean));
    assert_eq!(ty(E, "1 + 2"), scalar(ScalarType::Integer));
    assert_eq!(ty(E, "1 + 2.5"), scalar(ScalarType::Decimal));
    assert_eq!(ty(E, "1 / 2"), scalar(ScalarType::Decimal));
    assert_eq!(ty(E, "2 ^ 2"), scalar(ScalarType::Decimal));
    assert_eq!(ty(E, "-TRUE"), scalar(ScalarType::Integer));
    assert_eq!(ty(E, "1 & 2"), scalar(ScalarType::String));
    assert_eq!(ty(E, "1 < 2 && NOT FALSE"), scalar(ScalarType::Boolean));
    assert_eq!(ty(E, "1 IN {1, 2}"), scalar(ScalarType::Boolean));
    assert_eq!(ty(E, "{1, 2}"), Some(ValueType::Table));
    assert_eq!(ty(E, "VAR x = 1.5 RETURN x * 2"), scalar(ScalarType::Decimal));
    assert_eq!(ty(E, "Sales"), Some(ValueType::Table));
    assert_eq!(ty(E, "UNKNOWNFN(1)"), Some(ValueType::Unknown));
}

#[test]
fn infers_model_and_function_types() {
    // Column types come from the model; `decimal` is the engine's currency type.
    assert_eq!(ty(CC, "Sales[Amount]"), scalar(ScalarType::Decimal));
    assert_eq!(ty(CC, "Sales[Price]"), scalar(ScalarType::Currency));
    assert_eq!(ty(CC, "Sales[Date] + 1"), scalar(ScalarType::DateTime));
    assert_eq!(ty(CC, "[Category]"), scalar(ScalarType::String));
    // A calculated column without a data type is typed from its expression.
    assert_eq!(ty(CC, "Sales[Line]"), scalar(ScalarType::Currency));
    // Measures are typed from their expressions, recursively.
    assert_eq!(ty(M, "[Total Sales]"), scalar(ScalarType::Decimal));
    assert_eq!(ty(M, "[Units]"), scalar(ScalarType::Integer));
    assert_eq!(ty(M, "[Twice]"), scalar(ScalarType::Decimal));
    assert_eq!(ty(M, "[Label]"), scalar(ScalarType::String));
    // Mutually recursive measures do not loop; their type is unknown.
    assert_eq!(ty(M, "[Loop A]"), scalar(ScalarType::Variant));
    // Function return types: fixed, same as an argument, common of arguments.
    assert_eq!(ty(M, "SUM(Sales[Qty])"), scalar(ScalarType::Integer));
    assert_eq!(ty(M, "MAX(Sales[Date])"), scalar(ScalarType::DateTime));
    assert_eq!(ty(M, "COUNTROWS(Sales)"), scalar(ScalarType::Integer));
    assert_eq!(ty(M, "IF(TRUE, 1, 2.5)"), scalar(ScalarType::Decimal));
    assert_eq!(ty(M, "IF(TRUE, 1)"), scalar(ScalarType::Integer));
    assert_eq!(ty(M, "IF(TRUE, 1, \"a\")"), scalar(ScalarType::Variant));
    assert_eq!(
        ty(M, "SWITCH(1, 1, \"a\", 2, \"b\", \"c\")"),
        scalar(ScalarType::String)
    );
    assert_eq!(ty(M, "CALCULATE(SUM(Sales[Qty]))"), scalar(ScalarType::Integer));
    assert_eq!(ty(CT, "FILTER(Sales, TRUE)"), Some(ValueType::Table));
    // Without a model, references are unknown but the shape still is.
    let v = validate_with("SUM(T[C]) + 1", &Context::default(), M, test_sig);
    assert_eq!(v.result_type, scalar(ScalarType::Variant));
    assert!(v.valid);
}

#[test]
fn table_expected() {
    rejects(E, "COUNTROWS(1)", "table-expected");
    rejects(E, "COUNTROWS([Total Sales])", "table-expected");
    rejects(E, "VAR x = 1 RETURN COUNTROWS(x)", "table-expected");
    rejects(E, "SUMX(\"a\", 1)", "table-expected");
    rejects(E, "FILTER(1 + 1, TRUE)", "table-expected");
    clean(E, "COUNTROWS(Sales)");
    clean(E, "COUNTROWS(FILTER(Sales, Sales[Amount] > 0))");
    clean(E, "VAR t = FILTER(Sales, TRUE) RETURN COUNTROWS(t)");
    clean(E, "COUNTROWS({1, 2})");
    // Unknown shapes are given the benefit of the doubt.
    clean(E, "COUNTROWS(DETAILROWS([Total Sales]))");
    assert_eq!(errors(E, "COUNTROWS(MYSTERY())"), ["unknown-function"]);
}

#[test]
fn scalar_expected_depends_on_the_table_width() {
    // Several columns can never become a scalar: error.
    rejects(M, "COUNTROWS(Sales) + Sales", "scalar-expected");
    rejects(E, "DIVIDE(Sales, 2)", "scalar-expected");
    rejects(
        E,
        "VAR t = ADDCOLUMNS(VALUES(Sales[Date]), \"x\", 1) RETURN t + 1",
        "scalar-expected",
    );
    rejects(E, "IF(TRUE, 1) + {(1, 2)}", "scalar-expected");
    // One column converts at run time when there is one row: silent.
    clean(M, "VALUES(Sales[Date]) + 1");
    clean(M, "DIVIDE({1}, 2)");
    clean(M, "COUNTROWS(Sales) + Single");
    // Unknown width: warning only.
    assert_eq!(warnings(M, "DETAILROWS([Total Sales]) + 1"), ["scalar-expected"]);
    assert!(run(M, "DETAILROWS([Total Sales]) + 1").valid);
}

#[test]
fn column_reference_expected() {
    rejects(M, "SUM([Total Sales])", "column-ref-expected");
    rejects(M, "SUM(Sales[Total Sales])", "column-ref-expected");
    rejects(M, "SUM(1)", "column-ref-expected");
    rejects(M, "SUM(Sales[Amount] * 2)", "column-ref-expected");
    rejects(M, "SUM(Sales)", "column-ref-expected");
    rejects(M, "SUM(VALUES(Sales[Amount]))", "column-ref-expected");
    rejects(M, "VAR c = 1 RETURN SUM(c)", "column-ref-expected");
    rejects(M, "VALUES(1)", "column-ref-expected");
    rejects(M, "ALL(Sales[Amount], [Total Sales])", "column-ref-expected");
    clean(M, "SUM(Sales[Amount])");
    clean(M, "SUM([Amount])");
    clean(M, "COUNTROWS(VALUES(Sales))");
    clean(M, "COUNTROWS(ALL(Sales[Amount], Sales[Date]))");
    clean(M, "SUMX(ADDCOLUMNS(Sales, \"x\", 1), [x])");
    // The column is not double-reported as lacking a row context.
    assert_eq!(errors(M, "SUM(Sales[Amount] * 2)"), ["column-ref-expected"]);
    // An unknown column is the binder's business only.
    assert_eq!(errors(M, "SUM(Sales[Nope])"), ["unknown-column"]);
    // `MAX(a, b)` is the scalar form.
    clean(M, "MAX(1, 2)");
}

#[test]
fn keywords_and_new_column_names() {
    clean(M, "CALCULATE([Total Sales], DATEADD(Sales[Date], -1, YEAR))");
    clean(M, "RANKX(ALL(Product), [Total Sales], , DESC, Dense)");
    rejects(
        M,
        "CALCULATE([Total Sales], DATEADD(Sales[Date], -1, DESC))",
        "invalid-keyword",
    );
    rejects(M, "RANKX(ALL(Product), [Total Sales], , YEAR)", "invalid-keyword");
    rejects(
        M,
        "RANKX(ALL(Product), [Total Sales], , ASC, Product)",
        "invalid-keyword",
    );
    // A name that is no keyword at all is reported once, by the binder.
    assert_eq!(
        errors(M, "CALCULATE([Total Sales], DATEADD(Sales[Date], -1, YEARS))"),
        ["unknown-table"]
    );
    // Expressions in keyword positions are not keywords.
    clean(M, "RANKX(ALL(Product), [Total Sales], , 1)");

    clean(
        CT,
        "ADDCOLUMNS(Product, \"Sales\", [Total Sales], \"Units\", [Units])",
    );
    rejects(
        CT,
        "ADDCOLUMNS(Product, 1, [Total Sales])",
        "new-column-name-expected",
    );
    rejects(
        CT,
        "ADDCOLUMNS(Product, \"a\", 1, [Units], 2)",
        "new-column-name-expected",
    );
    // Flattened multi-section units (SUMMARIZECOLUMNS) are matched by shape in the engine;
    // nothing is checked positionally there.
    clean(
        CT,
        "SUMMARIZECOLUMNS(Sales[Date], Product[Category], Product[Color], Sales[Qty])",
    );
    clean(
        CT,
        "SUMMARIZECOLUMNS(Product[Category], \"Sales\", [Total Sales])",
    );
}

#[test]
fn bare_columns_need_a_row_context() {
    rejects(M, "Sales[Amount]", "no-row-context");
    rejects(M, "Sales[Amount] + 1", "no-row-context");
    rejects(M, "[Amount] * 2", "no-row-context");
    rejects(M, "IF(Sales[Amount] > 0, 1)", "no-row-context");
    rejects(M, "VAR x = Sales[Amount] RETURN x", "no-row-context");
    rejects(M, "COUNTROWS({ Sales[Amount] })", "no-row-context");
    rejects(M, "Product[Category] IN { \"Bikes\" }", "no-row-context");
    rejects(CT, "{ Sales[Amount] }", "no-row-context");
    // Iterators create a row context over their table...
    clean(M, "SUMX(Sales, Sales[Amount] * Sales[Qty])");
    clean(M, "SUMX(FILTER(Sales, Sales[Amount] > 0), [Amount])");
    clean(M, "COUNTROWS(FILTER(ALL(Sales[Date]), YEAR(Sales[Date]) = 2024))");
    clean(M, "SUMX(Sales, VAR a = Sales[Amount] RETURN a * 2)");
    clean(M, "RANKX(ALL(Product), [Total Sales])");
    clean(CT, "ADDCOLUMNS(Sales, \"Double\", Sales[Amount] * 2)");
    // ...but only over that table.
    rejects(M, "SUMX(Product, Sales[Amount])", "no-row-context");
    rejects(M, "SUMX(Sales, Product[Category] = \"x\")", "no-row-context");
    // An outer row context still counts.
    clean(
        M,
        "SUMX(Product, SUMX(Sales, Sales[Amount]) + Product[ProductKey])",
    );
    clean(
        M,
        "SUMX(Product, SUMX(FILTER(Sales, Sales[ProductKey] = Product[ProductKey]), Sales[Amount]))",
    );
    // GENERATE evaluates its second table in the row context of the first.
    clean(
        M,
        "COUNTROWS(GENERATE(Product, FILTER(Sales, Sales[ProductKey] = Product[ProductKey])))",
    );
    // As a calculated table, its two [ProductKey] columns would clash.
    rejects(
        CT,
        "GENERATE(Product, FILTER(Sales, Sales[ProductKey] = Product[ProductKey]))",
        "duplicate-column-name",
    );
    // Calculated columns and row filters have a row context over their home table.
    clean(CC, "Sales[Amount] * 2");
    clean(CC, "[Amount] * [Qty]");
    clean(RLS, "Sales[Amount] > 0");
    rejects(CC, "Product[Category]", "no-row-context");
    // Aggregations and column-reference parameters need no row context.
    clean(M, "SUM(Sales[Amount]) + MAX(Sales[Date])");
    clean(M, "COUNTROWS(VALUES(Sales[Date]))");
    // A bare expression has no known role: context rules are not applied.
    clean(E, "Sales[Amount] + 1");
    clean(E, "Product[Category] IN { \"Bikes\" }");
    // Inside a function the catalog does not type, nothing is assumed.
    assert_eq!(errors(M, "MYSTERY(Sales[Amount] + 1)"), ["unknown-function"]);
}

#[test]
fn related_and_earlier() {
    clean(CC, "RELATED(Product[Category])");
    clean(M, "SUMX(Sales, RELATED(Product[ProductKey]))");
    rejects(M, "RELATED(Product[Category])", "no-row-context");
    rejects(M, "IF(TRUE, RELATED(Product[Category]))", "no-row-context");

    clean(
        CC,
        "COUNTROWS(FILTER(Sales, Sales[Amount] > EARLIER(Sales[Amount])))",
    );
    clean(
        M,
        "SUMX(Sales, COUNTROWS(FILTER(Sales, Sales[Amount] > EARLIER(Sales[Amount]))))",
    );
    rejects(CC, "EARLIER(Sales[Amount])", "no-outer-row-context");
    rejects(M, "SUMX(Sales, EARLIER(Sales[Amount]))", "no-outer-row-context");
    rejects(M, "EARLIER(Sales[Amount])", "no-outer-row-context");
    rejects(
        CC,
        "COUNTROWS(FILTER(Sales, Sales[Amount] > EARLIER(Sales[Amount], 2)))",
        "no-outer-row-context",
    );
    clean(
        CC,
        "COUNTROWS(FILTER(Sales, COUNTROWS(FILTER(Sales, Sales[Qty] = EARLIER(Sales[Qty], 2))) > 0))",
    );
    // A boolean CALCULATE filter is a row context too.
    clean(
        CC,
        "CALCULATE(SUM(Sales[Amount]), Sales[Qty] = EARLIER(Sales[Qty]))",
    );
}

#[test]
fn boolean_calculate_filters() {
    clean(M, "CALCULATE([Total Sales], Sales[Amount] > 100)");
    clean(
        M,
        "CALCULATE([Total Sales], Product[Category] = \"Bikes\", Sales[Qty] > 1)",
    );
    clean(M, "CALCULATE([Total Sales], NOT (Sales[Date] = BLANK()))");
    // Several columns of one table are allowed (engine versions since 2021).
    clean(M, "CALCULATE([Total Sales], Sales[Amount] > Sales[Qty])");
    // Aggregations inside the filter are evaluated outside it.
    clean(M, "CALCULATE([Total Sales], Sales[Date] = MAX(Sales[Date]))");
    clean(
        M,
        "CALCULATE([Total Sales], Sales[Qty] > SUMX(Product, Product[ProductKey]))",
    );
    // Measures must go through a variable or FILTER.
    clean(
        M,
        "VAR t = [Total Sales] RETURN CALCULATE([Units], Sales[Amount] > t)",
    );
    clean(
        M,
        "CALCULATE([Units], FILTER(Sales, Sales[Amount] > [Total Sales]))",
    );
    rejects(
        M,
        "CALCULATE([Units], Sales[Amount] > [Total Sales])",
        "filter-uses-measure",
    );
    rejects(M, "CALCULATE([Units], [Total Sales] > 0)", "filter-uses-measure");
    rejects(
        M,
        "CALCULATE([Units], Sales[Amount] > CALCULATE(SUM(Sales[Amount])))",
        "filter-uses-measure",
    );
    rejects(
        M,
        "CALCULATE([Units], KEEPFILTERS(Sales[Amount] > [Total Sales]))",
        "filter-uses-measure",
    );
    rejects(
        M,
        "CALCULATETABLE(Sales, Sales[Amount] > [Total Sales])",
        "filter-uses-measure",
    );
    // Columns of two tables in one boolean filter.
    rejects(
        M,
        "CALCULATE([Units], Sales[Amount] > 1 || Product[Category] = \"x\")",
        "filter-multiple-tables",
    );
    rejects(
        M,
        "CALCULATE([Units], Sales[Qty] = Product[ProductKey])",
        "filter-multiple-tables",
    );
    // Table filters are not boolean filters.
    clean(
        M,
        "CALCULATE([Units], ALL(Product), DATEADD(Sales[Date], -1, YEAR))",
    );
}

#[test]
fn measures_columns_and_tables_have_a_result_shape() {
    rejects(M, "Sales", "measure-returns-table");
    rejects(M, "FILTER(Sales, TRUE)", "measure-returns-table");
    rejects(
        M,
        "ADDCOLUMNS(VALUES(Sales[Date]), \"x\", 1)",
        "measure-returns-table",
    );
    rejects(CC, "ALL(Sales)", "column-returns-table");
    // A one-column table is a valid measure; it fails at run time when several values show.
    assert!(errors(M, "VALUES(Sales[Date])").is_empty());
    assert_eq!(warnings(M, "VALUES(Sales[Date])"), ["runtime-multiple-values"]);
    clean(M, "{ 1 }");
    assert_eq!(
        warnings(M, "DETAILROWS([Total Sales])"),
        ["measure-returns-table"]
    );
    // A bare expression may be anything.
    clean(E, "FILTER(Sales, TRUE)");

    rejects(CT, "1 + 1", "calculated-table-returns-scalar");
    rejects(CT, "[Total Sales]", "calculated-table-returns-scalar");
    clean(CT, "FILTER(Sales, Sales[Amount] > 0)");
    clean(CT, "{ (1, \"a\") }");
    clean(CT, "Calc");
    clean(CT, "DETAILROWS([Total Sales])");
}

#[test]
fn degrades_without_signatures() {
    let m = model();
    let check = |kind, text: &str| {
        validate_with(
            text,
            &Context {
                model: Some(&m),
                table: Some("Sales"),
            },
            kind,
            no_sig,
        )
    };
    // Without signatures arguments are unchecked and calls are of unknown type...
    for text in [
        "SUM([Total Sales])",
        "COUNTROWS(1)",
        "SUMX(Product, Sales[Amount])",
        "CALCULATE([Units], Sales[Amount] > [Total Sales])",
        "RELATED(Product[Category])",
        "FILTER(Sales, TRUE)",
    ] {
        let v = check(M, text);
        assert!(
            v.diagnostics
                .iter()
                .all(|d| d.severity == Severity::Warning || d.code == "no-row-context"),
            "{text}: {:#?}",
            v.diagnostics
        );
        assert_eq!(v.result_type, Some(ValueType::Unknown), "{text}");
    }
    // ...but operators, literals and references are still typed and checked.
    let v = check(M, "Sales[Amount] + 1");
    assert_eq!(v.diagnostics.first().map(|d| d.code), Some("no-row-context"));
    assert_eq!(
        check(M, "[Total Sales] & \"\"").result_type,
        scalar(ScalarType::String)
    );
}

#[test]
fn diagnostics_point_at_the_offending_text() {
    let text = "CALCULATE([Units], Sales[Amount] > [Total Sales])";
    let v = run(M, text);
    let d = v
        .diagnostics
        .iter()
        .find(|d| d.code == "filter-uses-measure")
        .map(|d| (d.span.start, d.span.end));
    assert_eq!(d, Some((35, 48)));
    assert_eq!(text.get(35..48), Some("[Total Sales]"));
    let v = run(M, "1 +\n Sales[Amount]");
    assert_eq!(
        v.diagnostics.first().map(|d| (d.code, d.line, d.column)),
        Some(("no-row-context", 2, 2))
    );
}

#[test]
fn deep_measure_chains_do_not_exhaust_the_stack() {
    let mut m = model();
    let n = 5000;
    if let Some(t) = m.tables.first_mut() {
        for i in 0..n {
            t.measures
                .push(measure(&format!("Chain {i}"), &format!("[Chain {}] + 1", i + 1)));
        }
        t.measures
            .push(measure(&format!("Chain {n}"), "SUM(Sales[Amount])"));
    }
    let v = validate_with(
        "[Chain 0] * 2",
        &Context {
            model: Some(&m),
            table: Some("Sales"),
        },
        M,
        test_sig,
    );
    assert!(v.valid, "{:#?}", v.diagnostics);
    assert!(matches!(v.result_type, Some(ValueType::Scalar(_))));
    // Deeply nested expressions inside a measure chain stay bounded too.
    let deep = format!("{}1{}", "(".repeat(100), ")".repeat(100));
    let v = validate_with(
        &format!("{deep} + [Chain 0]"),
        &Context {
            model: Some(&m),
            table: Some("Sales"),
        },
        M,
        test_sig,
    );
    assert!(v.valid);
}

#[test]
fn real_catalog_entry_points_agree_with_validate() {
    // `validate` is `validate_as(.., Expression)`: no context rules.
    let m = model();
    let ctx = Context {
        model: Some(&m),
        table: Some("Sales"),
    };
    assert!(crate::validate("Sales[Amount] + 1", &ctx).valid);
    assert!(!crate::validate_as("Sales[Amount] + 1", &ctx, M).valid);
    assert!(crate::validate_as("Sales[Amount] + 1", &ctx, CC).valid);
    assert_eq!(
        crate::validate_as("1 + 1", &ctx, M).result_type,
        scalar(ScalarType::Integer)
    );
}

#[test]
fn return_types_follow_the_arguments_like_the_engine() {
    // As the engine types measures (checked on Desktop 2.158's engine, see docs/formats/bpa.md).
    let m = model();
    let ctx = Context {
        model: Some(&m),
        table: Some("Sales"),
    };
    let real = |text: &str| crate::validate_as(text, &ctx, M).result_type;
    assert_eq!(real("AVERAGE(Sales[Price])"), scalar(ScalarType::Currency));
    assert_eq!(
        real("AVERAGEX(Sales, Sales[Price])"),
        scalar(ScalarType::Currency)
    );
    assert_eq!(real("ROUND(SUM(Sales[Price]), 0)"), scalar(ScalarType::Currency));
    assert_eq!(real("INT(SUM(Sales[Price]))"), scalar(ScalarType::Currency));
    assert_eq!(real("INT(SUM(Sales[Amount]))"), scalar(ScalarType::Integer));
    assert_eq!(real("DIVIDE(SUM(Sales[Price]), 2)"), scalar(ScalarType::Currency));
    assert_eq!(real("DIVIDE(1, 2)"), scalar(ScalarType::Decimal));
    assert_eq!(real("CONVERT(1, DOUBLE)"), scalar(ScalarType::Decimal));
    assert_eq!(
        real("IF(TRUE, BLANK(), SUM(Sales[Price]))"),
        scalar(ScalarType::Currency)
    );
    assert_eq!(real("IF(TRUE, BLANK(), BLANK())"), scalar(ScalarType::Variant));
}

#[test]
fn related_follows_many_to_one_relationships() {
    clean(CC, "RELATED(Product[Category])");
    clean(M, "CONCATENATEX(Sales, RELATED(Product[Category]) & \"\")");
    // Same table, or against the relationship's direction.
    rejects(CC, "RELATED(Sales[Amount])", "no-relationship");
    rejects(M, "SUMX(Product, RELATED(Sales[Amount]))", "no-relationship");
    rejects(M, "SUMX(Product, RELATED(Product[Color]))", "no-relationship");
    rejects(M, "SUMX(Single, RELATED(Product[Color]))", "no-relationship");
    // Nested row contexts: any of them may reach the table.
    clean(M, "SUMX(Single, COUNTX(Sales, RELATED(Product[Color])))");

    clean(
        M,
        "CALCULATE([Units], USERELATIONSHIP(Sales[ProductKey], Product[ProductKey]))",
    );
    clean(
        M,
        "CALCULATE([Units], USERELATIONSHIP(Product[ProductKey], Sales[ProductKey]))",
    );
    rejects(
        M,
        "CALCULATE([Units], USERELATIONSHIP(Sales[Qty], Product[ProductKey]))",
        "no-relationship",
    );
}

#[test]
fn column_subsets_are_narrower_row_contexts() {
    clean(M, "SUMX(VALUES(Sales[Date]), YEAR(Sales[Date]))");
    clean(M, "SUMX(ALL(Sales[Date], Sales[Qty]), Sales[Qty])");
    rejects(M, "SUMX(VALUES(Sales[Date]), Sales[Qty])", "no-row-context");
    // A whole table still covers all its columns.
    clean(M, "SUMX(ALL(Sales), Sales[Qty])");
}

#[test]
fn table_parameters_reject_column_values() {
    // The engine reads a column where a table is expected as a value.
    rejects(M, "COUNTROWS(Sales[Qty])", "no-row-context");
    rejects(CC, "COUNTROWS(Sales[Qty])", "table-expected");
    assert_eq!(errors(M, "COUNTROWS(Sales[Qty])"), ["no-row-context"]);
    // One-argument MIN/MAX aggregate a column.
    rejects(M, "MAX(Sales)", "column-ref-expected");
    rejects(M, "MAX([Total Sales])", "column-ref-expected");
    clean(M, "MAX([Total Sales], 1)");
    // A calculated table that is a bare column.
    rejects(CT, "Sales[Amount]", "no-row-context");
}

#[test]
fn calculate_filters_need_a_column() {
    rejects(M, "CALCULATE([Units], TRUE)", "filter-without-column");
    rejects(M, "CALCULATE([Units], 1)", "filter-without-column");
    rejects(M, "CALCULATE([Units], \"Web\")", "filter-without-column");
    rejects(
        M,
        "CALCULATE([Units], MAX(Sales[Qty]) > 1)",
        "filter-without-column",
    );
    // Only one diagnostic for a measure-only filter.
    assert_eq!(
        errors(M, "CALCULATE([Units], [Total Sales] > 0)"),
        ["filter-uses-measure"]
    );
    // Elsewhere a constant may be another optional argument.
    clean(M, "TOTALYTD([Units], Sales[Date], \"6/30\")");
    // Empty filter arguments.
    rejects(M, "CALCULATE([Units], , )", "empty-argument");
    clean(M, "RANKX(ALL(Product), [Total Sales], , DESC)");
}

#[test]
fn keywords_are_bare_words() {
    rejects(
        M,
        "CALCULATE([Units], DATEADD(Sales[Date], -1, \"YEAR\"))",
        "invalid-keyword",
    );
    clean(M, "RANKX(ALL(Product), [Total Sales], , TRUE)");
}

#[test]
fn table_shapes() {
    // Table constructors are rectangular.
    rejects(E, "MAXX({(1, 2), (3)}, [Value1])", "ragged-table-constructor");
    clean(E, "MAXX({(1, 2), (3, 4)}, [Value1])");
    // One column is [Value]; several are [Value1]…[ValueN].
    assert_eq!(errors(E, "SUMX({1, 2}, [Value1])"), ["unknown-reference"]);
    clean(E, "SUMX({1, 2}, [Value])");
    // DATATABLE takes constants only.
    clean(
        CT,
        "DATATABLE(\"k\", INTEGER, \"s\", STRING, {{1, \"a\"}, {-2, BLANK()}})",
    );
    rejects(
        CT,
        "DATATABLE(\"k\", INTEGER, {{[Total Sales]}})",
        "datatable-constant-expected",
    );
    // New column names are unique.
    rejects(CT, "ROW(\"a\", 1, \"A\", 2)", "duplicate-column-name");
    rejects(CT, "ADDCOLUMNS(Product, \"Color\", 1)", "duplicate-column-name");
    clean(CT, "ADDCOLUMNS(Product, \"Colour\", 1)");
    // Set operations need tables of the same width.
    rejects(CT, "UNION({1, 2}, {(3, 4)})", "table-width-mismatch");
    rejects(CT, "UNION(Product, Single)", "table-width-mismatch");
    clean(CT, "UNION({1}, Single, VALUES(Product[Color]))");
}

#[test]
fn in_operator_shapes() {
    clean(M, "IF(1 IN {1, 2}, 1, 0)");
    clean(M, "IF((1, 2) IN {(1, 2)}, 1, 0)");
    clean(M, "IF(1 IN VALUES(Sales[Qty]), 1, 0)");
    rejects(M, "IF(1 IN {(1, 2)}, 1, 0)", "in-arity-mismatch");
    rejects(M, "IF(1 IN Sales, 1, 0)", "in-arity-mismatch");
    rejects(M, "IF(1 IN {}, 1, 0)", "in-arity-mismatch");
    rejects(M, "IF(1 IN 1, 1, 0)", "table-expected");
}

#[test]
fn variables_are_not_tables() {
    rejects(E, "VAR a = 1 VAR a = 2 RETURN a", "duplicate-variable");
    clean(E, "VAR a = 1 RETURN VAR b = a RETURN b");
    rejects(
        M,
        "VAR t = FILTER(Sales, Sales[Qty] > 1) RETURN SUMX(t, t[Qty])",
        "unknown-table",
    );
    rejects(E, "VAR x = 1 RETURN x[Value]", "unknown-table");
    clean(M, "VAR t = FILTER(Sales, Sales[Qty] > 1) RETURN SUMX(t, [Qty])");
}

#[test]
fn result_columns_are_unique_and_reachable() {
    rejects(
        CT,
        "SELECTCOLUMNS(Product, \"Who\", Product[Color], \"Who\", Product[Category])",
        "duplicate-column-name",
    );
    rejects(
        CT,
        "SELECTCOLUMNS(Product, Product[Color], \"Color\", 1)",
        "duplicate-column-name",
    );
    clean(
        CT,
        "SELECTCOLUMNS(Product, \"A\", Product[Color], Product[Category])",
    );
    rejects(CT, "CROSSJOIN(Product, Product)", "duplicate-column-name");
    rejects(
        CT,
        "GENERATE(VALUES(Product[Category]), TOPN(1, Product, Product[ProductKey]))",
        "duplicate-column-name",
    );
    clean(CT, "CROSSJOIN(VALUES(Product[Category]), VALUES(Product[Color]))");
    clean(CT, "CROSSJOIN(Product, Single)");
    // SUMMARIZE groups by columns of the expanded table only.
    clean(CT, "SUMMARIZE(Sales, Product[Category], Sales[Date])");
    rejects(CT, "SUMMARIZE(Product, Sales[Date])", "no-relationship");
    rejects(CT, "SUMMARIZE(Single, Product[Category])", "no-relationship");
}
