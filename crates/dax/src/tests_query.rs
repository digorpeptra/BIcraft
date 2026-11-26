//! DAX query tests: `DEFINE … EVALUATE … ORDER BY … START AT …`.

use bicraft_model::{Column, DataType, Measure, SemanticModel, Table};

use crate::ast::{Define, Expr};
use crate::{Context, Diagnostic, Severity, Validation, parse_query, validate_query};

fn model() -> SemanticModel {
    let col = |n: &str, t| Column {
        name: n.into(),
        data_type: t,
        ..Default::default()
    };
    let measure = |n: &str, e: &str| Measure {
        name: n.into(),
        expression: e.into(),
        ..Default::default()
    };
    SemanticModel {
        name: "Test".into(),
        tables: vec![
            Table {
                name: "Sales".into(),
                columns: vec![
                    col("Amount", DataType::Double),
                    col("Quantity", DataType::Int64),
                    col("Date", DataType::DateTime),
                    col("ProductKey", DataType::Int64),
                ],
                measures: vec![
                    measure("Total Sales", "SUM(Sales[Amount])"),
                    measure("Orders", "COUNTROWS(Sales)"),
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
                name: "Date".into(),
                columns: vec![
                    col("Date", DataType::DateTime),
                    col("Fiscal Year", DataType::String),
                    col("Month Name", DataType::String),
                    col("Month of Year", DataType::Int64),
                ],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn check(text: &str) -> Validation {
    let m = model();
    validate_query(
        text,
        &Context {
            model: Some(&m),
            table: None,
        },
    )
}

fn codes(text: &str) -> Vec<&'static str> {
    check(text).diagnostics.iter().map(|d| d.code).collect()
}

fn errors(text: &str) -> Vec<&'static str> {
    check(text)
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| d.code)
        .collect()
}

/// The single diagnostic with `code`.
fn diag(text: &str, code: &str) -> Diagnostic {
    let v = check(text);
    let found: Vec<_> = v.diagnostics.iter().filter(|d| d.code == code).collect();
    assert_eq!(found.len(), 1, "{text}: {:#?}", v.diagnostics);
    found[0].clone()
}

#[track_caller]
fn ok(text: &str) {
    let v = check(text);
    assert!(v.valid, "{text}: {:#?}", v.diagnostics);
    assert!(
        v.diagnostics.is_empty(),
        "{text}: unexpected diagnostics {:#?}",
        v.diagnostics
    );
}

#[test]
fn accepts_simple_queries() {
    ok("EVALUATE Sales");
    ok("EVALUATE 'Product'");
    ok("EVALUATE { [Total Sales] }");
    ok("EVALUATE { COUNTROWS('Sales') }");
    ok("EVALUATE ROW(\"Total\", [Total Sales], \"Orders\", [Orders])");
    ok("evaluate filter(Sales, Sales[Amount] > 100) order by Sales[Amount] desc");
    ok("EVALUATE {1}\nEVALUATE {2}\nEVALUATE Product");
    ok("EVALUATE VAR x = [Total Sales] RETURN { x, x * 2 }");
    ok("EVALUATE TOPN(10, Sales, Sales[Amount], DESC) ORDER BY Sales[Amount] DESC, Sales[Date]");
    ok("EVALUATE CALCULATETABLE(Product, Product[Color] = \"Red\")");
}

#[test]
fn accepts_dax_studio_style_queries() {
    ok("// Clothing orders by month
DEFINE
    MEASURE Sales[Big Orders] = CALCULATE([Orders], Sales[Amount] > 1000)
    MEASURE Sales[Avg Order] = DIVIDE([Total Sales], [Orders])
EVALUATE
SUMMARIZECOLUMNS(
    'Date'[Month Name],
    'Date'[Month of Year],
    'Product'[Category],
    -- optional filters
    FILTER(VALUES('Product'[Category]), [Category] = \"Clothing\"),
    \"Orders\", [Orders],
    \"Avg Profit per Order\", DIVIDE([Total Sales], [Big Orders])
)
/* sort columns must be in the query */
ORDER BY 'Date'[Month of Year] ASC, [Avg Profit per Order] DESC
START AT 3");

    // The docs example: a query table with a query column and a measure on that table.
    ok("DEFINE
    VAR _firstyear = MIN('Date'[Fiscal Year])
    VAR _lastyear = MAX('Date'[Fiscal Year])
    TABLE 'Unbought products' = FILTER('Product', [Orders] + 0 = 0)
    COLUMN 'Unbought products'[Year Range] = _firstyear & \" - \" & _lastyear
    MEASURE 'Unbought products'[Unbought products] = COUNTROWS('Unbought products')
EVALUATE
    'Unbought products'
    ORDER BY 'Unbought products'[Year Range], 'Unbought products'[Color]
EVALUATE
    {[Unbought products]}");

    // Definitions may reference definitions that come after them.
    ok("DEFINE
    MEASURE Sales[Ratio] = DIVIDE([Later], [Total Sales])
    MEASURE Sales[Later] = SUM(Sales[Quantity])
EVALUATE {[Ratio]}");
}

#[test]
fn query_measures_override_model_measures() {
    ok("DEFINE MEASURE Sales[Total Sales] = SUM(Sales[Quantity]) EVALUATE {[Total Sales]}");
    // A measure may live on another table than the model's measure of the same name.
    ok("DEFINE MEASURE Product[Orders] = 1 EVALUATE {[Orders]}");
}

#[test]
fn infers_query_table_and_variable_columns() {
    ok(
        "DEFINE VAR _t = SELECTCOLUMNS(Sales, \"Amt\", Sales[Amount], Sales[Date])
EVALUATE _t ORDER BY [Amt], [Date]",
    );
    ok(
        "DEFINE TABLE Big = ADDCOLUMNS(FILTER(Sales, Sales[Amount] > 10), \"Double\", Sales[Amount] * 2)
EVALUATE Big ORDER BY Big[Double], Big[Quantity]",
    );
    ok("DEFINE TABLE Pairs = { (1, \"a\"), (2, \"b\") } EVALUATE Pairs ORDER BY Pairs[Value2]");
    ok("DEFINE TABLE One = {1, 2, 3} EVALUATE One ORDER BY One[Value] DESC");
    ok("EVALUATE DATATABLE(\"Name\", STRING, \"Score\", INTEGER, {{\"a\", 1}, {\"b\", 2}}) ORDER BY [Score]");
    ok("EVALUATE GENERATESERIES(1, 10) ORDER BY [Value] DESC");
    ok("DEFINE TABLE Cal = CALENDAR(DATE(2024, 1, 1), DATE(2024, 12, 31)) EVALUATE Cal ORDER BY Cal[Date]");
    ok("EVALUATE CROSSJOIN(VALUES(Product[Color]), ROW(\"X\", 1)) ORDER BY [X], Product[Color]");

    let v = check("DEFINE TABLE Pairs = { (1, \"a\") } EVALUATE Pairs ORDER BY Pairs[Value3]");
    assert!(!v.valid);
    assert_eq!(v.diagnostics[0].code, "unknown-column");
    assert_eq!(v.diagnostics[0].column, 59);
}

#[test]
fn opaque_tables_are_warnings_not_errors() {
    let text = "DEFINE TABLE T = MYSTERY(Sales) EVALUATE T ORDER BY T[Anything], [Whatever]";
    let v = check(text);
    let codes: Vec<_> = v.diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(codes, ["opaque-table", "unknown-function", "unchecked-reference"]);
    // Against a model an unknown function is an error; what it hides stays a warning.
    assert!(!v.valid);
    let severities: Vec<_> = v.diagnostics.iter().map(|d| d.severity).collect();
    assert_eq!(
        severities,
        [Severity::Warning, Severity::Error, Severity::Warning]
    );
    assert_eq!(v.diagnostics[0].column, 14);
}

#[test]
fn evaluate_needs_a_table() {
    let d = diag("EVALUATE 1", "evaluate-needs-table");
    assert_eq!((d.line, d.column, d.span.start, d.span.end), (1, 10, 9, 10));
    assert_eq!(errors("EVALUATE [Total Sales]"), ["evaluate-needs-table"]);
    assert_eq!(errors("EVALUATE SUM(Sales[Amount])"), ["evaluate-needs-table"]);
    assert_eq!(errors("EVALUATE Sales[Amount]"), ["evaluate-needs-table"]);
    assert_eq!(errors("EVALUATE \"text\""), ["evaluate-needs-table"]);
    assert_eq!(errors("EVALUATE 1 + 2"), ["evaluate-needs-table"]);
    assert_eq!(errors("DEFINE VAR x = 1 EVALUATE x"), ["evaluate-needs-table"]);
    assert_eq!(errors("EVALUATE VAR x = 1 RETURN x"), ["evaluate-needs-table"]);
    assert_eq!(
        errors("DEFINE FUNCTION Twice = (x : NUMERIC) => x * 2 EVALUATE Twice(2)"),
        ["evaluate-needs-table"]
    );
    // A second statement is reported on its own line.
    let d = diag("EVALUATE {1}\nEVALUATE\n  [Orders]", "evaluate-needs-table");
    assert_eq!((d.line, d.column), (3, 3));
    // Unknown shapes (functions outside the catalog) are not shape errors.
    assert_eq!(errors("EVALUATE MYSTERY(Sales)"), ["unknown-function"]);
}

#[test]
fn definitions_need_the_right_shape() {
    assert_eq!(errors("DEFINE TABLE T = 1 EVALUATE T"), ["expected-table"]);
    assert_eq!(
        errors("DEFINE MEASURE Sales[m] = FILTER(Sales, TRUE) EVALUATE {[m]}"),
        ["expected-scalar"]
    );
    assert_eq!(
        errors("EVALUATE Sales ORDER BY Product"),
        ["order-by-needs-scalar"]
    );
}

#[test]
fn reports_duplicate_and_misplaced_definitions() {
    let text = "DEFINE MEASURE Sales[x] = 1 MEASURE Product[X] = 2 EVALUATE {[x]}";
    let d = diag(text, "duplicate-definition");
    assert_eq!(d.column, 44);
    assert_eq!(
        errors("DEFINE VAR a = 1 VAR A = 2 EVALUATE {a}"),
        ["duplicate-definition"]
    );
    assert_eq!(
        errors("DEFINE TABLE T = {1} TABLE t = {2} EVALUATE T"),
        ["duplicate-definition"]
    );
    assert_eq!(
        errors("DEFINE COLUMN Sales[c] = 1 COLUMN Sales[C] = 2 EVALUATE Sales"),
        ["duplicate-definition"]
    );
    // Same name, different kinds: allowed.
    ok("DEFINE VAR Total = 1 MEASURE Sales[Total] = 2 EVALUATE {Total + [Total]}");

    let d = diag("DEFINE MEASURE Nope[x] = 1 EVALUATE {[x]}", "unknown-table");
    assert_eq!((d.column, d.span.end), (16, 19));
    assert_eq!(
        errors("DEFINE COLUMN Nope[x] = 1 EVALUATE {1}"),
        ["unknown-table"]
    );
}

#[test]
fn reports_order_by_and_start_at_errors() {
    let d = diag("EVALUATE Sales ORDER BY Sales[Nope]", "unknown-column");
    assert_eq!(d.column, 25);
    let d = diag(
        "EVALUATE SUMMARIZECOLUMNS(Product[Category], \"Total\", [Total Sales])\nORDER BY [Totl]",
        "unknown-reference",
    );
    assert_eq!((d.line, d.column), (2, 10));
    ok("EVALUATE SUMMARIZECOLUMNS(Product[Category], \"Total\", [Total Sales]) ORDER BY [Total]");

    let d = diag(
        "EVALUATE Sales ORDER BY Sales[Amount] START AT 1, 2, 3",
        "start-at-too-many-values",
    );
    assert_eq!((d.column, d.span.end), (51, 54));
    ok("EVALUATE Sales ORDER BY Sales[Amount], Sales[Date] START AT 1");
    ok("EVALUATE Sales ORDER BY Sales[Amount], Sales[Date] START AT -1.5, DATE(2024, 1, 31)");
    assert_eq!(
        errors("EVALUATE Sales ORDER BY Sales[Amount] START AT Sales[Amount]"),
        ["start-at-not-constant"]
    );
    assert_eq!(
        errors("EVALUATE Sales ORDER BY Sales[Amount] START AT 1 + 1"),
        ["start-at-not-constant"]
    );
}

#[test]
fn xmla_parameters() {
    ok("EVALUATE FILTER(Sales, Sales[Amount] > @MinAmount) ORDER BY Sales[Amount] START AT @From");
    ok("DEFINE MEASURE Sales[p] = @p & \"x\" EVALUATE {[p]}");
    assert_eq!(errors("EVALUATE @p"), ["evaluate-needs-table"]);
    assert_eq!(errors("EVALUATE {1} // @ alone\n@"), ["unexpected-character"]);
    // Expressions keep rejecting parameters.
    assert!(!crate::validate("@p", &Context::default()).valid);
}

#[test]
fn diagnostics_inside_definitions_map_to_query_positions() {
    let text = "DEFINE\n    MEASURE Sales[x] = SUM(Sales[Nope])\nEVALUATE {[x]}";
    let d = diag(text, "unknown-column");
    assert_eq!((d.line, d.column), (2, 28));
    assert_eq!(text.get(d.span.start..d.span.end), Some("Sales[Nope]"));

    // With a query variable bound by a prefix.
    let text = "DEFINE VAR v = 1\nEVALUATE { v + [Nope] }";
    let d = diag(text, "unknown-reference");
    assert_eq!((d.line, d.column), (2, 16));
    assert_eq!(text.get(d.span.start..d.span.end), Some("[Nope]"));

    // Arity errors inside an ORDER BY key.
    let d = diag("EVALUATE Sales ORDER BY DIVIDE(1)", "wrong-argument-count");
    assert_eq!(d.column, 25);

    // Unknown table variables.
    assert_eq!(errors("EVALUATE Salez"), ["unknown-table"]);
    // A variable must be defined in the query.
    assert_eq!(errors("DEFINE VAR a = 1 EVALUATE {b}"), ["unknown-table"]);
}

#[test]
fn syntax_errors() {
    assert_eq!(codes(""), ["empty-query"]);
    assert_eq!(codes("  -- just a comment\n"), ["empty-query"]);
    assert_eq!(codes("DEFINE EVALUATE {1}"), ["empty-define"]);
    assert_eq!(codes("DEFINE MEASURE Sales[x] = 1"), ["missing-evaluate"]);
    assert_eq!(codes("EVALUATE {1} DEFINE VAR x = 1"), ["misplaced-define"]);
    assert_eq!(
        codes("DEFINE VAR a = 1 DEFINE VAR b = 2 EVALUATE {a}"),
        ["misplaced-define"]
    );
    assert_eq!(codes("EVALUATE Sales START AT 1"), ["start-at-without-order-by"]);
    assert_eq!(codes("{1}"), ["syntax"]);
    assert_eq!(codes("EVALUATE"), ["syntax"]);
    assert_eq!(codes("EVALUATE Sales ORDER Sales[Amount]"), ["syntax"]);
    assert_eq!(codes("EVALUATE Sales ORDER BY"), ["syntax"]);
    assert_eq!(codes("EVALUATE Sales ORDER BY Sales[Amount] START 1"), ["syntax"]);
    assert_eq!(codes("EVALUATE {1} 2"), ["syntax"]);
    assert_eq!(codes("DEFINE MEASURE [x] = 1 EVALUATE {1}"), ["syntax"]);
    assert_eq!(codes("DEFINE MEASURE Sales = 1 EVALUATE {1}"), ["syntax"]);
    assert_eq!(codes("DEFINE VAR = 1 EVALUATE {1}"), ["syntax"]);
    assert_eq!(codes("EVALUATE \"open"), ["unterminated-string"]);

    // A query keyword cannot start an expression: the error points at it.
    let v = check("DEFINE MEASURE Sales[x] =\nEVALUATE {1}");
    let d = &v.diagnostics[0];
    assert_eq!((d.code, d.line, d.column), ("syntax", 2, 1));
}

#[test]
fn user_defined_functions() {
    ok("DEFINE
    /// AddTax takes in amount and returns amount including tax
    /// @param {NUMERIC} amount - The pre-tax value
    FUNCTION AddTax =
        ( amount : NUMERIC, taxRate : NUMERIC = 0.1 ) =>
            amount * ( 1 + taxRate )
EVALUATE
{ AddTax ( 10 ), AddTax ( 10, 0.2 ) }");
    ok("DEFINE
    FUNCTION AddTax = ( amount : NUMERIC ) => amount * 1.1
    FUNCTION AddTaxAndDiscount = ( amount : NUMERIC, discount : NUMERIC ) => AddTax ( amount - discount )
EVALUATE { AddTaxAndDiscount ( 10, 2 ) }");
    ok("DEFINE FUNCTION CastToInt = ( x : SCALAR INT64 VAL ) => x
EVALUATE { CastToInt ( 3.4 ), CastToInt ( \"5\" ) }");
    ok("DEFINE
    FUNCTION CountRowsNow = ( t : TABLE VAL ) => COUNTROWS ( CALCULATETABLE ( t, ALL ( 'Date' ) ) )
    FUNCTION CountRowsLater = ( t : TABLE EXPR ) => COUNTROWS ( CALCULATETABLE ( t, ALL ( 'Date' ) ) )
EVALUATE { CountRowsNow ( 'Sales' ), CountRowsLater ( 'Sales' ) }");
    ok("DEFINE
    FUNCTION Multiply = ( a, b ) => a * b
    FUNCTION RandomInt = () => RANDBETWEEN ( 10, 100 )
    FUNCTION Local.Helpers.Twice = ( m : ANYREF ) => m * 2
EVALUATE { Multiply ( 3, 5 ), RandomInt (), Local.Helpers.Twice([Total Sales]) }");
    // A table-valued function can be evaluated.
    ok("DEFINE FUNCTION Top3 = ( t : TABLE ) => TOPN(3, t) EVALUATE Top3(Sales)");
    // Optional parameters may come before required ones (`MyFunc(1,,3)`).
    ok("DEFINE FUNCTION F = ( a, b = 2, c ) => a + b + c EVALUATE { F(1, , 3) }");
}

#[test]
fn user_defined_function_errors() {
    assert_eq!(
        errors("DEFINE FUNCTION SUM = ( x ) => x EVALUATE {1}"),
        ["function-name-reserved"]
    );
    assert_eq!(
        errors("DEFINE FUNCTION Measure = ( x ) => x EVALUATE {1}"),
        ["function-name-reserved"]
    );
    assert_eq!(
        errors("DEFINE FUNCTION a..b = ( x ) => x EVALUATE {1}"),
        ["invalid-function-name"]
    );
    let d = diag(
        "DEFINE FUNCTION F = ( x : NUMBER ) => x EVALUATE {F(1)}",
        "unknown-type-hint",
    );
    assert_eq!(d.column, 27);
    assert_eq!(
        errors("DEFINE FUNCTION F = ( x : INT64 SCALAR ) => x EVALUATE {F(1)}"),
        ["invalid-type-hint"]
    );
    assert_eq!(
        errors("DEFINE FUNCTION F = ( x : TABLE INT64 ) => x EVALUATE {F(1)}"),
        ["invalid-type-hint"]
    );
    assert_eq!(
        errors("DEFINE FUNCTION F = ( c : COLUMNREF VAL ) => c EVALUATE {F(1)}"),
        ["invalid-type-hint"]
    );
    assert_eq!(
        errors("DEFINE FUNCTION F = ( x, X ) => x EVALUATE {F(1, 2)}"),
        ["duplicate-definition"]
    );
    assert_eq!(
        errors("DEFINE FUNCTION F = ( x ) => F(x) EVALUATE {F(1)}"),
        ["recursive-function"]
    );
    let text = "DEFINE FUNCTION F = ( a, b = 1 ) => a + b EVALUATE { F(), F(1, 2, 3) }";
    assert_eq!(errors(text), ["wrong-argument-count", "wrong-argument-count"]);
    assert_eq!(
        errors("DEFINE FUNCTION F = ( x ) => x + [Nope] EVALUATE {F(1)}"),
        ["unknown-reference"]
    );
    assert_eq!(
        errors("DEFINE FUNCTION F = ( x ) => y EVALUATE {F(1)}"),
        ["unknown-table"]
    );
    assert_eq!(codes("DEFINE FUNCTION F = ( x : ) => x EVALUATE {1}"), ["syntax"]);
    assert_eq!(codes("DEFINE FUNCTION F = ( x ) x EVALUATE {1}"), ["syntax"]);
    assert_eq!(codes("DEFINE FUNCTION F = x => x EVALUATE {1}"), ["syntax"]);
}

#[test]
fn without_a_model_only_structure_is_checked() {
    let v = validate_query(
        "DEFINE MEASURE Nope[x] = SUM(Whatever[Anything]) EVALUATE Whatever ORDER BY Whatever[y]",
        &Context::default(),
    );
    assert!(v.valid, "{:#?}", v.diagnostics);
    let v = validate_query("EVALUATE 1", &Context::default());
    assert_eq!(v.diagnostics[0].code, "evaluate-needs-table");
    let v = validate_query("EVALUATE T ORDER BY T[a] START AT 1, 2", &Context::default());
    assert_eq!(v.diagnostics[0].code, "start-at-too-many-values");
}

#[test]
fn parse_query_builds_the_tree() {
    let text = "DEFINE
    MEASURE 'Sales'[m] = 1
    COLUMN Sales[c] = 2
    VAR v = 3
    TABLE t = {4}
    FUNCTION f = ( a : SCALAR NUMERIC, b = 5 ) => a + b
EVALUATE t
ORDER BY [Value] DESC, 1
START AT @x, 2
EVALUATE Sales";
    let (q, diags) = parse_query(text);
    assert!(diags.is_empty(), "{diags:#?}");
    let q = q.unwrap();
    let kinds: Vec<_> = q.defines.iter().map(Define::kind).collect();
    assert_eq!(kinds, ["measure", "column", "var", "table", "function"]);
    let names: Vec<_> = q.defines.iter().map(Define::name).collect();
    assert_eq!(names, ["m", "c", "v", "t", "f"]);
    match &q.defines[0] {
        Define::Measure {
            table, table_span, ..
        } => {
            assert_eq!(table, "Sales");
            assert_eq!(text.get(table_span.start..table_span.end), Some("'Sales'"));
        }
        other => panic!("{other:?}"),
    }
    match &q.defines[4] {
        Define::Function { params, .. } => {
            assert_eq!(params.len(), 2);
            let hints: Vec<_> = params[0].hints.iter().map(|h| h.word.as_str()).collect();
            assert_eq!(hints, ["SCALAR", "NUMERIC"]);
            assert!(params[0].default.is_none());
            assert!(params[1].default.is_some());
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(q.statements.len(), 2);
    let st = &q.statements[0];
    assert_eq!(st.order_by.len(), 2);
    assert!(st.order_by[0].descending);
    assert!(!st.order_by[1].descending);
    assert_eq!(
        text.get(st.order_by[0].span.start..st.order_by[0].span.end),
        Some("[Value] DESC")
    );
    assert!(matches!(&st.start_at[0], Expr::Name { name, .. } if name == "@x"));
    assert_eq!(
        text.get(st.span.start..st.span.end),
        Some("EVALUATE t\nORDER BY [Value] DESC, 1\nSTART AT @x, 2")
    );
    assert_eq!(q.span.start, 0);
    assert_eq!(q.span.end, text.len());
}

#[test]
fn hostile_queries_do_not_panic() {
    let deep = format!("EVALUATE {}1{}", "(".repeat(100_000), ")".repeat(100_000));
    assert_eq!(codes(&deep), ["too-deep"]);
    let deep_define = format!(
        "DEFINE MEASURE Sales[x] = {}1 EVALUATE {{1}}",
        "- ".repeat(100_000)
    );
    assert_eq!(codes(&deep_define), ["too-deep"]);

    // Many statements and definitions: linear work, no recursion per statement.
    let many = "EVALUATE {1} ".repeat(20_000);
    assert!(check(&many).valid);
    let vars: String = (0..2_000).map(|i| format!("VAR v{i} = {{{i}}} ")).collect();
    assert!(check(&format!("DEFINE {vars} EVALUATE v1999")).valid);

    // Variables chained through each other twice per level: memoized, not exponential.
    let mut chain = String::from("DEFINE VAR t0 = Sales ");
    for i in 1..200 {
        let p = i - 1;
        chain.push_str(&format!("VAR t{i} = CROSSJOIN(t{p}, t{p}) "));
    }
    chain.push_str("EVALUATE t199 ORDER BY [Amount]");
    let v = check(&chain);
    assert!(v.valid, "{:#?}", v.diagnostics);
    let mut nested = String::from("DEFINE VAR s0 = 1 ");
    for i in 1..200 {
        let p = i - 1;
        nested.push_str(&format!("VAR s{i} = VAR a = s{p} VAR b = s{p} RETURN a + b "));
    }
    nested.push_str("EVALUATE s199");
    assert_eq!(errors(&nested), ["evaluate-needs-table"]);
    // Mutually recursive variables and functions.
    assert_eq!(
        errors("DEFINE VAR a = b VAR b = a EVALUATE a"),
        ["circular-definition"]
    );
    assert_eq!(
        errors("DEFINE VAR a = a + 1 EVALUATE {a}"),
        ["circular-definition"]
    );
    let v = check("DEFINE FUNCTION f = () => g() FUNCTION g = () => f() EVALUATE f()");
    assert!(v.diagnostics.iter().all(|d| d.code != "evaluate-needs-table"));

    // Arbitrary bytes and truncated queries never panic.
    let samples = [
        "DEFINE",
        "DEFINE MEASURE",
        "DEFINE MEASURE 'x",
        "DEFINE MEASURE Sales[",
        "DEFINE FUNCTION f = (",
        "DEFINE FUNCTION f = (a :",
        "DEFINE FUNCTION f = (a = ",
        "DEFINE FUNCTION f = () =>",
        "EVALUATE Sales ORDER BY Sales[Amount] START AT",
        "EVALUATE @",
        "EVALUATE @é",
        "EVALUATE {@p}",
        "EVALUATE /* unterminated",
        "EVALUATE é",
        "\u{0}\u{1}EVALUATE",
        "EVALUATE ''[x]",
    ];
    for s in samples {
        let _ = check(s);
        let _ = parse_query(s);
        let _ = validate_query(s, &Context::default());
    }
    let text = "DEFINE MEASURE Sales[x] = 1 FUNCTION f = (a : TABLE, b = 2) => a EVALUATE f(Sales) ORDER BY [x] DESC START AT 1";
    for end in 0..=text.len() {
        if let Some(prefix) = text.get(..end) {
            let _ = check(prefix);
        }
    }
}

/// Validating against the model with the query's result and variable tables beside it gives the
/// diagnostics of validating against a copy holding them.
#[test]
fn queries_validate_the_same_without_a_copy_of_the_model() {
    let m = model();
    let queries = [
        "EVALUATE SUMMARIZECOLUMNS(Product[Category], \"T\", [Total Sales]) ORDER BY [T] DESC, Product[Category]",
        "DEFINE VAR f = TREATAS({\"A\"}, Product[Category]) VAR core = SUMMARIZECOLUMNS(Product[Category], f, \"T\", [Total Sales]) \
         EVALUATE TOPN(10, core, [T], 0, Product[Category], 1) ORDER BY [T] DESC, Product[Category]",
        "DEFINE VAR t = SELECTCOLUMNS(Sales, \"Amt\", Sales[Amount]) EVALUATE FILTER(t, [Amt] > 3) ORDER BY [Amt]",
        "DEFINE VAR t = SELECTCOLUMNS(Sales, \"Amt\", Sales[Amount]) EVALUATE t ORDER BY [Nope]",
        "EVALUATE ROW(\"x\", [Total Sales], \"y\", [Orders]) ORDER BY [x], [Unknown]",
        "EVALUATE VALUES(Product[Category]) ORDER BY Product[Category] START AT \"B\"",
        "EVALUATE Sales ORDER BY Sales[Amount] DESC, [Quantity]",
        "EVALUATE FILTER(Product, Product[Category] = 1 + \"x\")",
        "EVALUATE ADDCOLUMNS(VALUES(Product[Category]), \"c\", CALCULATE([Total Sales])) ORDER BY [c], NoTable[x]",
        "DEFINE VAR v = 3 EVALUATE {v} ORDER BY [Value]",
        "EVALUATE GENERATESERIES(1, 3) ORDER BY [Value] START AT 2, 3",
        "DEFINE VAR a = CALCULATETABLE(Sales, Product[Category] = \"A\") EVALUATE a ORDER BY Sales[Date]",
        "EVALUATE SUMMARIZE(Sales, Product[Category]) ORDER BY [Total Sales]",
        "DEFINE MEASURE Sales[M] = [Total Sales] * 2 EVALUATE ROW(\"m\", [M]) ORDER BY [m]",
        "DEFINE TABLE Q = {1, 2} EVALUATE Q ORDER BY [Value]",
        "EVALUATE 1",
        "EVALUATE SUMMARIZECOLUMNS(Sales[Missing], \"x\", 1)",
    ];
    let ctx = Context {
        model: Some(&m),
        table: None,
    };
    for q in queries {
        let overlay = validate_query(q, &ctx);
        crate::query::FORCE_COPY.with(|c| c.set(true));
        let copy = validate_query(q, &ctx);
        crate::query::FORCE_COPY.with(|c| c.set(false));
        assert_eq!(format!("{overlay:?}"), format!("{copy:?}"), "{q}");
    }
}
