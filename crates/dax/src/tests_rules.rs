//! Engine rules found with the oracle (G4) and the run-time warnings (G20), against the full
//! catalog. Each case mirrors an engine verdict recorded in `tests/oracle` (see
//! `docs/dax-oracle.md`).

use bicraft_model::{Column, DataType, Measure, Partition, Relationship, SemanticModel, Table};

use crate::{Context, ExprKind, Severity, Validation, validate_as, validate_query};

fn col(name: &str, data_type: DataType) -> Column {
    Column {
        name: name.into(),
        data_type,
        ..Default::default()
    }
}

fn data_table(name: &str, columns: Vec<Column>, measures: Vec<Measure>) -> Table {
    Table {
        name: name.into(),
        columns,
        measures,
        partitions: vec![Partition {
            name: name.into(),
            source_type: "m".into(),
            expression: Some("let Source = 1 in Source".into()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn rel(from: &str, from_col: &str, to: &str, to_col: &str) -> Relationship {
    Relationship {
        name: format!("{from}_{to}"),
        from_table: from.into(),
        from_column: from_col.into(),
        to_table: to.into(),
        to_column: to_col.into(),
        is_active: true,
        ..Default::default()
    }
}

fn model() -> SemanticModel {
    SemanticModel {
        name: "Rules".into(),
        tables: vec![
            data_table(
                "Sales",
                vec![
                    col("ProductKey", DataType::Int64),
                    col("CustomerKey", DataType::Int64),
                    col("OrderDate", DataType::DateTime),
                    col("Quantity", DataType::Int64),
                    col("Channel", DataType::String),
                    col("IsReturn", DataType::Boolean),
                ],
                vec![Measure {
                    name: "Total".into(),
                    expression: "SUM(Sales[Quantity])".into(),
                    ..Default::default()
                }],
            ),
            data_table(
                "Product",
                vec![
                    col("ProductKey", DataType::Int64),
                    col("Category", DataType::String),
                ],
                vec![],
            ),
            data_table(
                "Customer",
                vec![col("CustomerKey", DataType::Int64), col("City", DataType::String)],
                vec![],
            ),
            data_table(
                "Date",
                vec![
                    col("Date", DataType::DateTime),
                    col("Year", DataType::Int64),
                    col("Month", DataType::Int64),
                ],
                vec![],
            ),
        ],
        relationships: vec![
            rel("Sales", "ProductKey", "Product", "ProductKey"),
            rel("Sales", "CustomerKey", "Customer", "CustomerKey"),
            rel("Sales", "OrderDate", "Date", "Date"),
        ],
        ..Default::default()
    }
}

const M: ExprKind = ExprKind::Measure;
const CC: ExprKind = ExprKind::CalculatedColumn;
const CT: ExprKind = ExprKind::CalculatedTable;
const RLS: ExprKind = ExprKind::RowFilter;

fn run_on(m: &SemanticModel, kind: ExprKind, text: &str) -> Validation {
    let home = (kind != CT).then_some("Sales");
    validate_as(
        text,
        &Context {
            model: Some(m),
            table: home,
        },
        kind,
    )
}

fn codes(kind: ExprKind, text: &str, severity: Severity) -> Vec<&'static str> {
    run_on(&model(), kind, text)
        .diagnostics
        .iter()
        .filter(|d| d.severity == severity)
        .map(|d| d.code)
        .collect()
}

#[track_caller]
fn rejects(kind: ExprKind, text: &str, code: &str) {
    let errs = codes(kind, text, Severity::Error);
    assert!(
        errs.contains(&code),
        "{kind:?} `{text}`: expected {code}, got {errs:?}"
    );
}

#[track_caller]
fn accepts(kind: ExprKind, text: &str) {
    let v = run_on(&model(), kind, text);
    assert!(v.valid, "{kind:?} `{text}`: {:#?}", v.diagnostics);
}

/// Valid, with exactly this run-time warning among its warnings.
#[track_caller]
fn warns(kind: ExprKind, text: &str, code: &str) {
    let v = run_on(&model(), kind, text);
    assert!(v.valid, "{kind:?} `{text}`: {:#?}", v.diagnostics);
    assert!(
        v.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Warning && d.code == code),
        "{kind:?} `{text}`: expected warning {code}, got {:#?}",
        v.diagnostics
    );
}

/// Valid and without any diagnostic.
#[track_caller]
fn clean(kind: ExprKind, text: &str) {
    let v = run_on(&model(), kind, text);
    assert!(
        v.valid && v.diagnostics.is_empty(),
        "{kind:?} `{text}`: {:#?}",
        v.diagnostics
    );
}

/// A variable wins over a table of the same name, even quoted: `'Product'` denotes
/// `VAR Product` (engine verdicts `var-shadowing-*`). A column qualifier is always a table.
#[test]
fn variables_shadow_tables_even_quoted() {
    #[track_caller]
    fn invalid(kind: ExprKind, text: &str) {
        let v = run_on(&model(), kind, text);
        assert!(!v.valid, "{kind:?} `{text}` should be rejected");
    }
    invalid(M, "VAR Product = 3 RETURN COUNTROWS('Product')");
    invalid(M, "VAR product = 3 RETURN COUNTROWS('Product')");
    invalid(M, "VAR Sales = 1 RETURN SUMX('Sales', Sales[Quantity])");
    invalid(M, "VAR Sales = 1 RETURN CALCULATE([Total], ALL('Sales'))");
    invalid(M, "VAR t = 1 RETURN COUNTROWS('t')");
    invalid(M, "VAR Customer = 2 RETURN CALCULATE([Total], 'Customer')");
    accepts(
        M,
        "VAR Customer = FILTER(Customer, TRUE()) RETURN CALCULATE([Total], 'Customer')",
    );
    invalid(CC, "VAR Product = 1 RETURN COUNTROWS('Product')");
    invalid(CT, "VAR Product = 1 RETURN 'Product'");
    accepts(M, "VAR Product = 3 RETURN Product + COUNTROWS(Sales)");
    accepts(M, "VAR Product = 3 RETURN 'Product' + 1");
    accepts(M, "VAR Product = Sales RETURN COUNTROWS('Product')");
    accepts(M, "VAR t = Sales RETURN COUNTROWS('t')");
    accepts(
        M,
        "VAR x = COUNTROWS('Product') VAR Product = 2 RETURN x + Product",
    );
    // A column qualifier names the table even with a variable of that name in scope.
    accepts(M, "VAR Product = 3 RETURN SUM('Product'[ProductKey]) + Product");
    accepts(M, "VAR Product = 1 RETURN SUMX(VALUES(Product[Category]), 1)");
    invalid(M, "VAR t = Sales RETURN SUM(t[Quantity])");
    accepts(
        CT,
        "VAR Product = FILTER(Product, Product[ProductKey] > 1) RETURN 'Product'",
    );
    let ctx = Context {
        model: Some(&model()),
        table: None,
    };
    assert!(
        !validate_query(
            "DEFINE VAR Product = 3 EVALUATE ROW(\"x\", COUNTROWS('Product'))",
            &ctx
        )
        .valid
    );
    assert!(!validate_query("DEFINE VAR Product = 3 EVALUATE 'Product'", &ctx).valid);
    let v = validate_query(
        "DEFINE VAR Product = FILTER(Product, TRUE()) EVALUATE 'Product'",
        &ctx,
    );
    assert!(v.valid, "{:#?}", v.diagnostics);
}

#[test]
fn reserved_words() {
    rejects(M, "VAR Value = 1 RETURN Value", "reserved-word");
    rejects(M, "VAR sum = 1 RETURN 2", "reserved-word");
    rejects(M, "MAX(Date[Year])", "reserved-word");
    rejects(M, "MAX(date[Year])", "reserved-word");
    clean(M, "VAR Year = 1 RETURN Year");
    clean(M, "VAR Index = 1 RETURN Index");
    clean(M, "MAX('Date'[Year])");
    clean(M, "COUNTROWS(Date)");
    let q = validate_query("EVALUATE ROW(\"x\", VAR Rank = 1 RETURN 1)", &Context::default());
    assert!(!q.valid);
    assert_eq!(q.diagnostics[0].code, "reserved-word");
}

/// Engine keywords shared with MDX/DMX (`Typed`, `Status`, `Name`…) are reserved too, and most
/// of them also refuse a bare table reference (Desktop 2.158 engine, `docs/dax-oracle.md`).
#[test]
fn keyword_table_names_need_quotes() {
    let mut m = model();
    for name in ["Typed", "Status", "Densify"] {
        m.tables
            .push(data_table(name, vec![col("N", DataType::Int64)], vec![]));
    }
    let errs = |kind: ExprKind, text: &str| -> Vec<String> {
        run_on(&m, kind, text)
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| format!("{}: {}", d.code, d.message))
            .collect()
    };
    let reserved = |kind: ExprKind, text: &str| {
        let e = errs(kind, text);
        assert!(
            e.iter().any(|d| d.starts_with("reserved-word")),
            "{kind:?} `{text}`: expected reserved-word, got {e:?}"
        );
        e
    };
    let ok = |kind: ExprKind, text: &str| {
        let v = run_on(&m, kind, text);
        assert!(v.valid, "{kind:?} `{text}`: {:#?}", v.diagnostics);
    };
    let e = reserved(M, "SUM(Typed[N])");
    assert!(e[0].contains("'Typed'"), "hint to quote: {e:?}");
    reserved(M, "sum(typed[N])");
    reserved(M, "COUNTROWS(Typed)");
    reserved(M, "SUM(Status[N])");
    reserved(M, "COUNTROWS(FILTER(Status, Status[N] > 1))");
    reserved(CC, "Typed[N] + 1");
    reserved(M, "VAR Typed = 1 RETURN Typed");
    reserved(M, "VAR Densify = 1 RETURN Densify");
    ok(M, "SUM('Typed'[N])");
    ok(M, "COUNTROWS('Typed') + COUNTROWS('Status')");
    ok(M, "SUM(Densify[N]) + COUNTROWS(Densify)");
    ok(M, "COUNTROWS(Date)");
    // UDF type words and newer keywords are not reserved.
    for w in [
        "Function", "Val", "Expr", "Scalar", "AnyVal", "AnyRef", "Int64", "Numeric",
    ] {
        ok(M, &format!("VAR {w} = 1 RETURN {w}"));
    }
    // Keyword arguments that are reserved words stay fine.
    ok(M, "RANKX(ALL(Product), [Total], , DESC, Skip)");
    ok(M, "CALCULATE([Total], TOPN(1, Product, [Total], ASC))");

    let q = |text: &str| {
        validate_query(
            text,
            &Context {
                model: Some(&m),
                table: None,
            },
        )
    };
    for text in [
        "EVALUATE Typed",
        "DEFINE TABLE Typed2 = {1} TABLE Status = {1} EVALUATE 'Status'",
        "DEFINE MEASURE Typed[m] = 1 EVALUATE ROW(\"x\", [m])",
        "DEFINE VAR Status = 1 EVALUATE ROW(\"x\", 1)",
        "DEFINE FUNCTION Typed = (a) => a EVALUATE ROW(\"x\", 1)",
        "DEFINE FUNCTION Densify = (a) => a EVALUATE ROW(\"x\", 1)",
        "DEFINE FUNCTION F = (Typed) => 1 EVALUATE ROW(\"x\", F(1))",
        "DEFINE FUNCTION F = (Status) => 1 EVALUATE ROW(\"x\", F(1))",
    ] {
        let v = q(text);
        assert!(
            v.diagnostics.iter().any(|d| matches!(
                d.code,
                "reserved-word" | "function-name-reserved" | "invalid-parameter-name"
            )),
            "`{text}`: {:#?}",
            v.diagnostics
        );
    }
    for text in [
        "EVALUATE 'Typed'",
        "DEFINE MEASURE 'Typed'[m] = 1 EVALUATE ROW(\"x\", [m])",
        "DEFINE FUNCTION F = (t : TABLE) => COUNTROWS(t) EVALUATE ROW(\"x\", F('Typed'))",
    ] {
        let v = q(text);
        assert!(v.valid, "`{text}`: {:#?}", v.diagnostics);
    }
}

#[test]
fn unknown_functions_are_errors_against_a_model() {
    rejects(M, "SUMM(Sales[Quantity])", "unknown-function");
    rejects(M, "FOO()", "unknown-function");
}

#[test]
fn week_intervals_need_a_calendar() {
    rejects(
        M,
        "CALCULATE([Total], DATEADD('Date'[Date], -1, WEEK))",
        "interval-needs-calendar",
    );
    rejects(
        M,
        "CALCULATE([Total], PARALLELPERIOD('Date'[Date], -1, WEEK))",
        "interval-needs-calendar",
    );
    rejects(
        M,
        "CALCULATE([Total], DATESINPERIOD('Date'[Date], DATE(2024,1,1), -7, WEEK))",
        "interval-needs-calendar",
    );
    clean(M, "CALCULATE([Total], DATEADD('Date'[Date], -1, DAY))");
    clean(M, "DATEDIFF(DATE(2024,1,1), DATE(2024,2,1), WEEK)");
}

#[test]
fn dates_arguments() {
    for bad in [
        "CALCULATE([Total], DATESYTD('Date'))",
        "CALCULATE([Total], DATEADD(ALL('Date'), -1, YEAR))",
        "CALCULATE([Total], DATESYTD(FILTER('Date', TRUE())))",
        "CALCULATE([Total], DATESYTD(ALL('Date'[Date], 'Date'[Year])))",
        "CALCULATE([Total], DATESYTD(ROW(\"d\", DATE(2024,1,1))))",
        "CALCULATE([Total], DATESYTD(SELECTCOLUMNS('Date', \"d\", 'Date'[Date])))",
        "CALCULATE([Total], DATESYTD(GENERATESERIES(1, 3)))",
        "CALCULATE([Total], DATESYTD(\"2024-01-01\"))",
        "CALCULATE([Total], DATESYTD(TODAY()))",
        "CALCULATE([Total], DATESYTD([Total]))",
    ] {
        rejects(M, bad, "dates-expected");
    }
    for good in [
        "CALCULATE([Total], DATESYTD('Date'[Date]))",
        "CALCULATE([Total], DATEADD(VALUES('Date'[Date]), -1, YEAR))",
        "CALCULATE([Total], DATESYTD(CALCULATETABLE(VALUES('Date'[Date]))))",
        "CALCULATE([Total], DATESYTD(DISTINCT('Date'[Date])))",
        "CALCULATE([Total], DATESYTD(VAR d = 1 RETURN 'Date'[Date]))",
    ] {
        clean(M, good);
    }
    rejects(
        M,
        "TOTALYTD([Total], 'Date'[Date], \"x\")",
        "invalid-year-end-date",
    );
    clean(M, "TOTALYTD([Total], 'Date'[Date], \"6/30\")");
    warns(
        M,
        "CALCULATE([Total], DATESYTD('Date'[Year]))",
        "runtime-dates-type",
    );
    warns(
        M,
        "CALCULATE([Total], DATESYTD(VALUES('Date'[Year])))",
        "runtime-dates-type",
    );
    warns(
        M,
        "CALCULATE([Total], DATEADD({DATE(2024,1,1)}, -1, YEAR))",
        "runtime-dates-type",
    );
}

#[test]
fn groupby_and_currentgroup() {
    for good in [
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", SUMX(CURRENTGROUP(), Sales[Quantity])))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", COUNTX(CURRENTGROUP(), 1)))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", SUMX(CURRENTGROUP(), Sales[Quantity] * 2)))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel]))",
    ] {
        clean(M, good);
    }
    for bad in [
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", SUM(Sales[Quantity])))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", SUMX(CURRENTGROUP(), Sales[Quantity]) + 1))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", 1))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", MEDIANX(CURRENTGROUP(), Sales[Quantity])))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", SUMX(CURRENTGROUP(), [Total])))",
    ] {
        rejects(M, bad, "groupby-expression");
    }
    rejects(M, "COUNTX(CURRENTGROUP(), 1)", "currentgroup-outside-groupby");
    rejects(
        M,
        "COUNTROWS(ADDCOLUMNS(GROUPBY(Sales, Sales[Channel]), \"Q\", SUMX(CURRENTGROUP(), Sales[Quantity])))",
        "currentgroup-outside-groupby",
    );
}

#[test]
fn window_relations_and_clauses() {
    for good in [
        "CALCULATE([Total], OFFSET(-1, ALLSELECTED('Date'[Year]), ORDERBY('Date'[Year])))",
        "CALCULATE([Total], OFFSET(-1, ALLSELECTED('Date'[Year], 'Date'[Month]), ORDERBY('Date'[Year]), PARTITIONBY('Date'[Month])))",
        "CALCULATE([Total], OFFSET(-1, ALLSELECTED('Date'), ORDERBY('Date'[Year])))",
        "CALCULATE([Total], OFFSET(-1, ALLSELECTED('Date'[Year]), ORDERBY([Total])))",
        "CALCULATE([Total], OFFSET(-1, ORDERBY('Date'[Year])))",
        "CALCULATE([Total], OFFSET(-1, , ORDERBY('Date'[Year])))",
        "ROWNUMBER(ALLSELECTED('Date'[Year]), ORDERBY('Date'[Year], DESC))",
        "CALCULATE([Total], WINDOW(1, ABS, -1, ABS, ALLSELECTED('Date'[Year]), ORDERBY('Date'[Year])))",
    ] {
        clean(M, good);
    }
    rejects(
        M,
        "CALCULATE([Total], OFFSET(-1, ALLSELECTED('Date'[Year]), ORDERBY('Date'[Month])))",
        "no-row-context",
    );
    rejects(
        M,
        "CALCULATE([Total], OFFSET(-1, ALLSELECTED(Sales), ORDERBY(Product[Category])))",
        "no-row-context",
    );
    rejects(
        M,
        "CALCULATE([Total], OFFSET(-1, ALLSELECTED('Date'), ORDERBY('Date'[Year]), PARTITIONBY(Product[Category])))",
        "window-column-not-in-relation",
    );
    rejects(
        M,
        "CALCULATE([Total], OFFSET(-1, ALLSELECTED('Date'[Year]), ORDERBY('Date'[Year]), MATCHBY('Date'[Month])))",
        "window-column-not-in-relation",
    );
    rejects(
        M,
        "CALCULATE([Total], OFFSET(-1, 'Date'[Year], ORDERBY('Date'[Year])))",
        "window-relation-column",
    );
    rejects(M, "RANK(DENSE, 'Date'[Year])", "window-relation-column");
    rejects(M, "ROWNUMBER('Date'[Year])", "window-relation-column");
    rejects(M, "ORDERBY('Date'[Year])", "clause-outside-window");
    rejects(
        M,
        "CALCULATE([Total], ORDERBY('Date'[Year]))",
        "clause-outside-window",
    );
    rejects(M, "PARTITIONBY('Date'[Year])", "clause-outside-window");
    // An outer row context covers the ORDERBY column.
    accepts(
        CC,
        "CALCULATE(SUM(Sales[Quantity]), OFFSET(-1, ALL('Date'[Year]), ORDERBY(Sales[Quantity])))",
    );
}

#[test]
fn natural_joins_and_calculated_table_names() {
    rejects(
        M,
        "COUNTROWS(NATURALINNERJOIN(Customer, Product))",
        "no-common-join-columns",
    );
    rejects(
        M,
        "COUNTROWS(NATURALINNERJOIN(Product, VALUES(Sales[ProductKey])))",
        "no-common-join-columns",
    );
    clean(M, "COUNTROWS(NATURALINNERJOIN('Date', Sales))");
    clean(M, "COUNTROWS(NATURALINNERJOIN(Customer, Sales))");
    clean(M, "COUNTROWS(NATURALINNERJOIN(Product, FILTER(Sales, TRUE())))");
    // Duplicate names matter in calculated tables only.
    clean(M, "COUNTROWS(NATURALINNERJOIN(Sales, Product))");
    clean(M, "COUNTROWS(CROSSJOIN(Sales, Product))");
    rejects(CT, "NATURALINNERJOIN(Sales, Product)", "duplicate-column-name");
    rejects(CT, "CROSSJOIN(Sales, Product)", "duplicate-column-name");
    rejects(
        CT,
        "SUMMARIZE(Sales, Sales[ProductKey], Product[ProductKey])",
        "duplicate-column-name",
    );
    rejects(
        CT,
        "SUMMARIZECOLUMNS(Sales[ProductKey], Product[ProductKey])",
        "duplicate-column-name",
    );
    rejects(
        CT,
        "CROSSJOIN(VALUES(Sales[ProductKey]), VALUES(Product[ProductKey]))",
        "duplicate-column-name",
    );
    clean(CT, "NATURALINNERJOIN('Date', Sales)");
    clean(CT, "NATURALINNERJOIN(Product, Product)");
    clean(CT, "CROSSJOIN(Customer, Product)");
    clean(
        CT,
        "UNION(VALUES(Sales[ProductKey]), VALUES(Product[ProductKey]))",
    );
}

#[test]
fn calendarauto_needs_a_data_date_column() {
    clean(CT, "CALENDARAUTO()");
    let mut m = model();
    for t in &mut m.tables {
        for p in &mut t.partitions {
            p.source_type = "calculated".into();
        }
    }
    let v = run_on(&m, CT, "CALENDARAUTO()");
    assert!(!v.valid);
    assert_eq!(v.diagnostics[0].code, "calendarauto-no-date-column");
}

#[test]
fn incompatible_comparisons() {
    // Calculated columns and tables: a definition error.
    for bad in [
        "1 = \"1\"",
        "TRUE() = 1",
        "Sales[Channel] = 1",
        "Sales[OrderDate] > \"2024-01-01\"",
        "VAR x = Sales[Channel] RETURN x = 1",
        "MAX(Sales[Channel]) = 1",
        "Sales[Channel] IN {1, 2}",
        "SWITCH(TRUE(), \"a\", 1, 2)",
        "CONTAINS(Sales, Sales[Channel], 1)",
    ] {
        rejects(CC, bad, "incompatible-comparison");
    }
    rejects(CT, "FILTER(Sales, Sales[Channel] = 1)", "incompatible-comparison");
    for good in [
        "Sales[Channel] = \"Web\"",
        "Sales[Quantity] = 1.5",
        "Sales[OrderDate] = 45000",
        "Sales[Channel] < BLANK()",
        "Sales[IsReturn] = TRUE()",
    ] {
        clean(CC, good);
    }
    // Measures and row filters: a run-time error.
    warns(M, "1 = \"1\"", "runtime-incompatible-comparison");
    warns(
        M,
        "CALCULATE([Total], Sales[Channel] = 1)",
        "runtime-incompatible-comparison",
    );
    warns(M, "SWITCH(1, \"a\", 1, 2)", "runtime-incompatible-comparison");
    warns(RLS, "Sales[Channel] = 1", "runtime-incompatible-comparison");
}

#[test]
fn runtime_warnings() {
    for (text, code) in [
        ("\"a\" + 1", "runtime-conversion"),
        ("\"\" + 1", "runtime-conversion"),
        ("-\"x\"", "runtime-conversion"),
        ("\"x\" && TRUE()", "runtime-conversion"),
        ("IF(\"yes\", 1, 0)", "runtime-conversion"),
        ("YEAR(\"abc\")", "runtime-conversion"),
        ("ROUND(\"x\", 0)", "runtime-conversion"),
        ("VALUE(\"twelve\")", "runtime-conversion"),
        ("CONVERT(\"x\", INTEGER)", "runtime-conversion"),
        ("COUNTROWS(GENERATESERIES(1, \"z\"))", "runtime-conversion"),
        ("MOD(5, 0)", "runtime-invalid-argument"),
        ("SQRT(-1)", "runtime-invalid-argument"),
        ("LEFT(\"abc\", -1)", "runtime-invalid-argument"),
        ("MID(\"abc\", 0, 1)", "runtime-invalid-argument"),
        ("SUM(Sales[Channel])", "runtime-aggregation-type"),
        ("SUMX(Sales, \"x\")", "runtime-aggregation-type"),
        ("{1, 2} + 1", "runtime-multiple-values"),
        ("VALUES(Product[Category])", "runtime-multiple-values"),
        ("ALL(Product[Category])", "runtime-multiple-values"),
    ] {
        warns(M, text, code);
    }
    for text in [
        "\"1\" + 1",
        "\"2024-01-01\" + 1",
        "IF(\"TRUE\", 1, 0)",
        "IF(\"\", 1, 0)",
        "YEAR(\"2024-01-01\")",
        "VALUE(\"12\")",
        "CONVERT(\"x\", STRING)",
        "FORMAT(\"x\", \"0\")",
        "DIVIDE(1, 0, \"n/a\")",
        "1 / 0",
        "MAX(Sales[Channel])",
        "SUM(Sales[OrderDate])",
        "COUNTX(Sales, \"a\")",
        "{1}",
    ] {
        clean(M, text);
    }
    // A guarded VALUES is fine at run time (the shape warning is not a run-time one).
    let v = run_on(
        &model(),
        M,
        "IF(HASONEVALUE(Product[Category]), VALUES(Product[Category]))",
    );
    assert!(v.valid && v.diagnostics.iter().all(|d| !d.code.starts_with("runtime-")));
    warns(RLS, "\"TRUE\"", "runtime-non-boolean-filter");
    warns(RLS, "IF(Sales[IsReturn], 1, 0)", "runtime-non-boolean-filter");
    clean(RLS, "Sales[IsReturn]");
    // Text arithmetic in a calculated column fails while computing: still only a warning.
    warns(CC, "\"a\" + 1", "runtime-conversion");
}

#[test]
fn query_cycles_count_when_evaluated() {
    let m = model();
    let ctx = Context {
        model: Some(&m),
        table: None,
    };
    let codes = |q: &str| -> Vec<&'static str> {
        validate_query(q, &ctx)
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| d.code)
            .collect()
    };
    assert_eq!(
        codes("DEFINE MEASURE Sales[A] = [B] MEASURE Sales[B] = [A] EVALUATE ROW(\"x\", [A])"),
        ["circular-dependency"]
    );
    assert_eq!(
        codes("DEFINE MEASURE Sales[A] = [B] MEASURE Sales[B] = [A] VAR v = [A] EVALUATE ROW(\"x\", v)"),
        ["circular-dependency"]
    );
    assert_eq!(
        codes("DEFINE MEASURE Sales[Total] = [Total] + 1 EVALUATE ROW(\"x\", [Total])"),
        ["circular-dependency"]
    );
    // Cycles nothing evaluates are ignored.
    assert!(codes("DEFINE MEASURE Sales[A] = [B] MEASURE Sales[B] = [A] EVALUATE ROW(\"x\", 1)").is_empty());
    assert!(codes("DEFINE MEASURE Sales[A] = [A] EVALUATE ROW(\"x\", 1)").is_empty());
    // XMLA parameters stand for values of any type: no type warning about their stand-in.
    let v = validate_query("EVALUATE FILTER(Sales, Sales[Quantity] > @Min)", &ctx);
    assert!(v.valid && v.diagnostics.is_empty(), "{:#?}", v.diagnostics);
}

#[test]
fn multi_section_functions() {
    for good in [
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], FILTER(Sales, TRUE()), \"x\", [Total]))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], TREATAS({\"Web\"}, Sales[Channel]), \"x\", [Total]))",
        "COUNTROWS(SUMMARIZE(Sales, ROLLUP(Sales[Channel]), \"x\", SUM(Sales[Quantity])))",
        "COUNTROWS(SUMMARIZECOLUMNS(ROLLUPADDISSUBTOTAL(Sales[Channel], \"IsTotal\"), \"x\", [Total]))",
        "COUNTROWS(SUMMARIZECOLUMNS(\"x\", [Total]))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], Product))",
        "COUNTROWS(SUMMARIZE(Sales, Sales[Channel], \"x\", Sales[Channel]))",
        "COUNTROWS(SUMMARIZE(Sales, Product[Category], \"x\", Product[Category]))",
    ] {
        clean(M, good);
    }
    rejects(
        M,
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], [Total]))",
        "column-ref-expected",
    );
    rejects(
        M,
        "COUNTROWS(SUMMARIZE(Sales, Sales[Channel] & \"\"))",
        "column-ref-expected",
    );
    rejects(
        M,
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], 1))",
        "table-expected",
    );
    rejects(
        M,
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], \"x\", Sales))",
        "scalar-expected",
    );
    rejects(
        M,
        "COUNTROWS(SUMMARIZE(Sales, Sales[Channel], \"x\"))",
        "missing-expression",
    );
    rejects(
        M,
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], \"x\", [Total], Sales[Quantity]))",
        "section-order",
    );
    rejects(
        M,
        "COUNTROWS(SUMMARIZE(Sales, Sales[Channel], Sales[Channel]))",
        "duplicate-group-by-column",
    );
    rejects(
        M,
        "COUNTROWS(SUMMARIZE(Sales, ROLLUPADDISSUBTOTAL(Sales[Channel], \"IsTotal\"), \"x\", 1))",
        "column-ref-expected",
    );
    // SUMMARIZE's expressions see the group-by columns only; SUMMARIZECOLUMNS' see none.
    rejects(
        M,
        "COUNTROWS(SUMMARIZE(Sales, Sales[Channel], \"x\", Sales[Quantity]))",
        "no-row-context",
    );
    rejects(
        M,
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], \"x\", Sales[Channel]))",
        "no-row-context",
    );
    rejects(M, "COUNTROWS(SUMMARIZE(Sales))", "wrong-argument-count");
    rejects(M, "COUNTROWS(SUMMARIZECOLUMNS())", "wrong-argument-count");
}

#[test]
fn text_operands_warn() {
    warns(M, "SUMX(Sales, Sales[Channel] + 1)", "runtime-conversion");
    warns(CC, "Sales[Channel] && TRUE()", "runtime-conversion");
    clean(M, "SUMX(Sales, Sales[Quantity] + 1)");
}

#[test]
fn filter_modifiers() {
    for good in [
        "CALCULATE([Total], USERELATIONSHIP(Sales[OrderDate], 'Date'[Date]))",
        "TOTALYTD([Total], 'Date'[Date], USERELATIONSHIP(Sales[OrderDate], 'Date'[Date]))",
        "CALCULATE([Total], KEEPFILTERS(REMOVEFILTERS(Sales)))",
        "CALCULATE([Total], ALLSELECTED())",
        "CALCULATE([Total], KEEPFILTERS(ALL()))",
        "SUMX(KEEPFILTERS(VALUES(Sales[Channel])), 1)",
        "COUNTROWS(FILTER(KEEPFILTERS(VALUES(Sales[Channel])), TRUE()))",
        "COUNTROWS(GENERATE(KEEPFILTERS(VALUES(Sales[Channel])), VALUES(Product[Category])))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], REMOVEFILTERS(Product), \"x\", [Total]))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], KEEPFILTERS(REMOVEFILTERS(Product)), \"x\", [Total]))",
        "COUNTROWS(ALLSELECTED(Sales))",
    ] {
        clean(M, good);
    }
    for bad in [
        "USERELATIONSHIP(Sales[OrderDate], 'Date'[Date])",
        "CALCULATE([Total], KEEPFILTERS(USERELATIONSHIP(Sales[OrderDate], 'Date'[Date])))",
        "VAR r = USERELATIONSHIP(Sales[OrderDate], 'Date'[Date]) RETURN CALCULATE([Total], r)",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], CROSSFILTER(Sales[ProductKey], Product[ProductKey], BOTH), \"x\", [Total]))",
        "SUMX(REMOVEFILTERS(Sales), Sales[Quantity])",
        "CALCULATE([Total], IF(TRUE(), REMOVEFILTERS(Sales)))",
        "COUNTROWS(CALCULATETABLE(REMOVEFILTERS(Sales[Channel])))",
        "COUNTROWS(FILTER(ALLSELECTED(), TRUE()))",
        "VAR a = ALL() RETURN CALCULATE([Total], a)",
        "COUNTROWS(KEEPFILTERS(VALUES(Sales[Channel])))",
        "COUNTROWS(FILTER(Sales, KEEPFILTERS(Sales[Channel] = \"Web\")))",
        "COUNTROWS(GENERATE(VALUES(Sales[Channel]), KEEPFILTERS(VALUES(Product[Category]))))",
        "COUNTROWS(NONVISUAL(VALUES(Sales[Channel])))",
    ] {
        rejects(M, bad, "misplaced-filter-modifier");
    }
    // Measures defer some misplacements to evaluation; other objects reject them at definition.
    for deferred in [
        "COUNTROWS(REMOVEFILTERS(Sales))",
        "ISEMPTY(REMOVEFILTERS(Sales))",
        "SUMX(ALL(), 1)",
        "SUMX(ALLCROSSFILTERED(Sales), 1)",
        "CALCULATE([Total], NONVISUAL(VALUES(Sales[Channel])))",
    ] {
        warns(M, deferred, "runtime-misplaced-filter-modifier");
    }
    rejects(CC, "COUNTROWS(REMOVEFILTERS(Sales))", "misplaced-filter-modifier");
    rejects(
        RLS,
        "COUNTROWS(REMOVEFILTERS(Sales)) > 0",
        "misplaced-filter-modifier",
    );
    rejects(CT, "ROW(\"x\", COUNTROWS(ALL()))", "misplaced-filter-modifier");
    rejects(CT, "KEEPFILTERS(Sales)", "misplaced-filter-modifier");
    // The result type of a modifier is unchanged (internal marker only).
    let v = run_on(&model(), ExprKind::Expression, "REMOVEFILTERS(Sales)");
    assert_eq!(v.result_type, Some(crate::types::ValueType::Unknown));
    // A query statement fails on a deferred misplacement.
    let m = model();
    let ctx = Context {
        model: Some(&m),
        table: None,
    };
    let v = validate_query("EVALUATE ROW(\"x\", COUNTROWS(REMOVEFILTERS(Sales)))", &ctx);
    assert!(
        v.diagnostics
            .iter()
            .any(|d| d.code == "misplaced-filter-modifier" && d.severity == Severity::Error),
        "{:#?}",
        v.diagnostics
    );
}

#[test]
fn summarizecolumns_nonvisual_and_values() {
    let sc = |s: &str| format!("COUNTROWS(SUMMARIZECOLUMNS({s}))");
    for good in [
        "Sales[Channel], NONVISUAL(TREATAS({\"Web\"}, Sales[Channel])), \"x\", [Total]",
        "Sales[Channel], NONVISUAL(ALL(Sales)), \"x\", [Total]",
        "Sales[Channel], Product[Category], NONVISUAL(CROSSJOIN(VALUES(Sales[Channel]), VALUES(Product[Category]))), \"x\", [Total]",
        "Sales[Channel], KEEPFILTERS(NONVISUAL(TREATAS({\"Web\"}, Sales[Channel]))), \"x\", [Total]",
        "Sales[Channel], NONVISUAL(SELECTCOLUMNS(Sales, \"c\", Sales[Channel])), \"x\", [Total]",
    ] {
        clean(M, &sc(good));
    }
    for bad in [
        "Sales[Channel], NONVISUAL(FILTER(Sales, TRUE())), \"x\", [Total]",
        "Sales[Channel], NONVISUAL(TREATAS({\"A\"}, Product[Category])), \"x\", [Total]",
        "NONVISUAL(TREATAS({\"Web\"}, Sales[Channel])), \"x\", [Total]",
        "ROLLUPADDISSUBTOTAL(Sales[Channel], \"T\"), NONVISUAL(TREATAS({\"Web\"}, Sales[Channel])), \"x\", [Total]",
        "Sales[Channel], NONVISUAL(KEEPFILTERS(FILTER(Sales, TRUE()))), \"x\", [Total]",
    ] {
        rejects(M, &sc(bad), "nonvisual-not-group-by");
    }
    for failing in [
        "Sales[Channel], \"x\", VALUES(Sales[Quantity])",
        "\"x\", VALUES(Sales[Quantity])",
        "Sales[Channel], \"x\", ALL(Sales[Channel])",
        "Sales[Channel], \"x\", 1 + VALUES(Sales[Quantity]) * 2",
        "Sales[Channel], \"x\", VAR v = VALUES(Sales[Quantity]) RETURN v",
        "Sales[ProductKey], \"x\", VALUES(Product[Category])",
        "Product[ProductKey], \"x\", VALUES(Sales[Quantity])",
        "Sales[Channel], TREATAS({\"Web\"}, Sales[Channel]), \"x\", VALUES(Sales[Quantity])",
    ] {
        warns(M, &sc(failing), "runtime-multiple-values");
    }
    for fine in [
        "Sales[Channel], \"x\", VALUES(Sales[Channel])",
        "Product[ProductKey], \"x\", VALUES(Product[Category])",
        "Sales[Channel], \"x\", IF(HASONEVALUE(Sales[Quantity]), VALUES(Sales[Quantity]))",
        "Sales[Channel], \"x\", COUNTROWS(VALUES(Sales[Quantity]))",
        "Sales[Channel], TREATAS({1}, Sales[Quantity]), \"x\", VALUES(Sales[Quantity])",
        "Sales[Channel], FILTER(Sales, Sales[Quantity] = 1), \"x\", VALUES(Sales[Quantity])",
    ] {
        // A guarded VALUES keeps its shape warning, which is not a run-time one.
        let v = run_on(&model(), M, &sc(fine));
        assert!(
            v.valid && v.diagnostics.iter().all(|d| !d.code.starts_with("runtime-")),
            "{fine}: {:#?}",
            v.diagnostics
        );
    }
}

/// The `INFO.*` functions other than the four `INFO.VIEW.*` only run in DAX queries (Microsoft's
/// DAX reference, "INFO functions"; documented, not measured: `docs/formats/dax-functions.md`).
#[test]
fn info_functions_outside_queries() {
    for kind in [M, CC, CT, RLS] {
        for bad in [
            "COUNTROWS(INFO.TABLES())",
            "MAXX(INFO.PARTITIONS(), [RefreshedTime])",
            "COUNTROWS(info.measures(\"Name\", \"Total\"))",
        ] {
            let text = if kind == CT {
                format!("ROW(\"x\", {bad})")
            } else if kind == RLS {
                format!("{bad} > 0")
            } else {
                bad.to_string()
            };
            rejects(kind, &text, "info-query-only");
        }
    }
    rejects(CT, "INFO.MODEL()", "info-query-only");
    for good in [
        "COUNTROWS(INFO.VIEW.TABLES())",
        "COUNTROWS(FILTER(INFO.VIEW.TABLES(), NOT ISBLANK([Expression])))",
        "COUNTROWS(FILTER(INFO.VIEW.COLUMNS(), [DataType] = \"Text\"))",
        "COUNTROWS(INFO.VIEW.MEASURES()) + COUNTROWS(INFO.VIEW.RELATIONSHIPS())",
    ] {
        accepts(M, good);
    }
    accepts(
        CT,
        "SELECTCOLUMNS(INFO.VIEW.MEASURES(), \"Measure\", [Name], [Expression])",
    );
    accepts(CT, "INFO.VIEW.TABLES()");
    // Queries run every INFO function, also inside a query measure.
    let m = model();
    let ctx = Context {
        model: Some(&m),
        table: None,
    };
    for q in [
        "EVALUATE INFO.PARTITIONS()",
        "EVALUATE ROW(\"Last\", MAXX(INFO.PARTITIONS(), [RefreshedTime]))",
        "EVALUATE SELECTCOLUMNS(INFO.TABLES(), \"T\", [Name], [ModifiedTime])",
        "DEFINE MEASURE Sales[N] = COUNTROWS(INFO.COLUMNS()) EVALUATE ROW(\"n\", [N])",
    ] {
        let v = validate_query(q, &ctx);
        assert!(v.valid, "{q}: {:#?}", v.diagnostics);
    }
    assert!(crate::is_query_only_info("INFO.PARTITIONS"));
    assert!(crate::is_query_only_info("info.tables"));
    assert!(!crate::is_query_only_info("INFO.VIEW.TABLES"));
    assert!(!crate::is_query_only_info("INFO."));
    assert!(!crate::is_query_only_info("SUM"));
    assert!(!crate::is_query_only_info("INFOÉ.X"));
    // the same rule as one scan, for the edits that validate a model object's text
    let d = crate::query_only_info_calls(
        "VAR t = INFO.TABLES() RETURN COUNTROWS(t) + COUNTROWS(INFO.VIEW.TABLES()) + MAXX(info.partitions(), 1)",
    );
    assert_eq!(
        d.iter().map(|d| d.code).collect::<Vec<_>>(),
        ["info-query-only", "info-query-only"]
    );
    assert!(
        d[1].message.starts_with("INFO.PARTITIONS can only"),
        "{}",
        d[1].message
    );
    assert!(crate::query_only_info_calls("COUNTROWS(INFO.VIEW.MEASURES())").is_empty());
    let _ = crate::query_only_info_calls("INFO.TABLES(");
}

// ------------------------------------------------------------------------------------------------
// Rules found with the signature corpus (M11.40, `tests/oracle/signatures.json`): each case
// mirrors a recorded engine verdict.

fn query_codes(q: &str) -> Vec<&'static str> {
    let m = model();
    let v = validate_query(
        q,
        &Context {
            model: Some(&m),
            table: None,
        },
    );
    v.diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| d.code)
        .collect()
}

#[test]
fn visual_calculation_functions_outside_visual_calculations() {
    for f in [
        "RUNNINGSUM([Total])",
        "MOVINGAVERAGE([Total], 3)",
        "COLLAPSE([Total], ROWS)",
        "EXPANDALL([Total], ROWS)",
        "ISATLEVEL(Sales[Channel])",
        "RANGE(1)",
        "PREVIOUS(Sales[Quantity])",
        "FIRST(Sales[Quantity])",
    ] {
        rejects(M, f, "visual-calc-only");
    }
    assert!(query_codes("EVALUATE ROW(\"x\", RUNNINGSUM([Total]))").contains(&"visual-calc-only"));
    // The keywords FIRST and LAST stay keywords.
    accepts(
        M,
        "COUNTROWS(INDEX(1, ALL(Sales[Quantity]), ORDERBY(Sales[Quantity]), LAST))",
    );
}

#[test]
fn keyword_parameters_refuse_constants() {
    rejects(M, "DATEDIFF(TODAY(), TODAY(), 1)", "invalid-keyword");
    rejects(M, "CONVERT(1, 1)", "invalid-keyword");
    rejects(M, "RANKX(Sales, [Total], , DESC, 1)", "invalid-keyword");
    rejects(M, "NAMEOF(Sales[Quantity], 1)", "invalid-keyword");
    accepts(M, "DATEDIFF(TODAY(), TODAY(), DAY)");
    accepts(M, "RANKX(Sales, [Total], , DESC, DENSE)");
    // Sort orders take 0/1 and booleans.
    accepts(M, "RANKX(Sales, [Total], , 0)");
    accepts(M, "RANKX(Sales, [Total], , TRUE)");
}

#[test]
fn sort_orders_of_shape_bound_units() {
    rejects(
        M,
        "COUNTROWS(TOPN(1, Sales, Sales[Quantity], \"a\"))",
        "invalid-keyword",
    );
    rejects(M, "COUNTROWS(TOPN(1, Sales, 'Product', ASC))", "scalar-expected");
    rejects(M, "ISAFTER(1, 1, \"a\")", "invalid-keyword");
    rejects(M, "ISONORAFTER('Product', 1, ASC)", "scalar-expected");
    rejects(
        M,
        "CONCATENATEX(Sales, Sales[Channel], \",\", Sales[Quantity], \"a\")",
        "invalid-keyword",
    );
    rejects(
        M,
        "COUNTROWS(INDEX(1, ALL(Sales[Quantity]), ORDERBY(Sales[Quantity], \"a\")))",
        "invalid-keyword",
    );
    accepts(
        M,
        "COUNTROWS(TOPN(1, Sales, Sales[Quantity], ASC, Sales[Channel], 0))",
    );
    accepts(M, "COUNTROWS(TOPN(1, Sales, Sales[Quantity], Sales[Channel]))");
    accepts(M, "ISONORAFTER(1, 2, DESC, \"a\", \"b\")");
    accepts(
        M,
        "CONCATENATEX(Sales, Sales[Channel], \",\", Sales[Quantity], DESC)",
    );
}

#[test]
fn selectcolumns_names_its_expressions() {
    rejects(
        M,
        "COUNTROWS(SELECTCOLUMNS(Sales, 1, Sales[Quantity]))",
        "new-column-name-expected",
    );
    rejects(
        M,
        "COUNTROWS(SELECTCOLUMNS(Sales, \"N\", 'Product'))",
        "scalar-expected",
    );
    accepts(
        M,
        "COUNTROWS(SELECTCOLUMNS(Sales, Sales[Quantity], \"N\", Sales[Quantity] * 2))",
    );
}

#[test]
fn table_references_are_not_table_expressions() {
    for f in [
        "COUNTROWS(RELATEDTABLE(FILTER(Sales, TRUE())))",
        "CALCULATE(1, ALLEXCEPT(FILTER(Sales, TRUE()), Sales[Channel]))",
        "CALCULATE(1, ALLCROSSFILTERED(FILTER(Sales, TRUE())))",
    ] {
        rejects(M, f, "table-ref-expected");
    }
    accepts(M, "CALCULATE(1, ALLEXCEPT(Sales, Sales[Channel]))");
}

#[test]
fn constant_filters_and_measure_parameters() {
    rejects(M, "CALCULATE(1, \"a\")", "filter-without-column");
    rejects(M, "TOTALMTD(1, 'Date'[Date], \"a\")", "filter-without-column");
    // A year-end date may stand in the filter's place.
    accepts(M, "TOTALYTD([Total], 'Date'[Date], \"6/30\")");
    // ... and the engine takes a table as the year-end date.
    accepts(
        M,
        "TOTALYTD([Total], 'Date'[Date], Sales[Quantity] > 1, 'Product')",
    );
    rejects(M, "COUNTROWS(DETAILROWS(1))", "measure-ref-expected");
    rejects(M, "ISSELECTEDMEASURE(1)", "measure-ref-expected");
    rejects(M, "COUNTROWS(DISTINCT(1))", "table-expected");
    accepts(M, "COUNTROWS(DISTINCT(Sales[Channel]))");
}

#[test]
fn aggregations_that_take_a_measure() {
    for f in [
        "AVERAGEA([Total])",
        "MAXA([Total])",
        "MINA([Total])",
        "MEDIAN([Total])",
        "PERCENTILE.INC([Total], 0.5)",
    ] {
        accepts(M, f);
    }
    rejects(M, "AVERAGE([Total])", "column-ref-expected");
    accepts(M, "COUNTROWS(FIRSTNONBLANK(Sales, Sales[Quantity]))");
    // FIRSTNONBLANKVALUE evaluates its expression with no row context.
    rejects(
        M,
        "FIRSTNONBLANKVALUE(Sales[Quantity], Sales[Quantity])",
        "no-row-context",
    );
    accepts(M, "FIRSTNONBLANKVALUE(Sales[Quantity], [Total])");
    accepts(M, "COUNTROWS(LINEST(Sales[Quantity], Sales[ProductKey], TRUE()))");
    accepts(M, "EVALUATEANDLOG(1, \"a\", 'Product')");
    clean(M, "COUNTROWS(TOPNSKIP(1, \"a\", Sales, Sales[Quantity]))");
}

#[test]
fn calendar_only_arguments() {
    rejects(M, "COUNTROWS(DATESWTD('Date'[Date]))", "calendar-expected");
    rejects(M, "TOTALWTD([Total], 1)", "calendar-expected");
    rejects(
        M,
        "COUNTROWS(DATEADD('Date'[Date], 1, DAY, EXTENDING))",
        "argument-needs-calendar",
    );
    rejects(
        M,
        "COUNTROWS(DATESINPERIOD('Date'[Date], TODAY(), 1, DAY, PRECISE))",
        "argument-needs-calendar",
    );
    accepts(M, "COUNTROWS(DATEADD('Date'[Date], 1, DAY))");
    rejects(
        M,
        "COUNTROWS(PARALLELPERIOD('Date'[Date], 1, DAY))",
        "invalid-keyword",
    );
    accepts(M, "COUNTROWS(PARALLELPERIOD('Date'[Date], 1, MONTH))");
}

#[test]
fn window_functions_outside_visual_calculations() {
    rejects(M, "COUNTROWS(INDEX(1))", "window-needs-relation");
    rejects(M, "RANK()", "window-needs-relation");
    rejects(
        M,
        "COUNTROWS(INDEX(1, ALL(Sales[Quantity]), ORDERBY(Sales[Quantity]), , , , NONE))",
        "window-reset-with-clauses",
    );
    rejects(M, "COUNTROWS(INDEX(1, 1))", "window-argument-expected");
    rejects(
        M,
        "COUNTROWS(INDEX(1, ALL(Sales[Quantity]), 1))",
        "window-argument-expected",
    );
    accepts(M, "COUNTROWS(INDEX(1, ORDERBY(Sales[Quantity])))");
    accepts(M, "COUNTROWS(INDEX(1, ALL(Sales[Quantity])))");
}

#[test]
fn functions_bound_to_one_parent() {
    rejects(M, "IGNORE(1)", "misplaced-function");
    rejects(M, "NATURALJOINUSAGE(1)", "misplaced-function");
    rejects(
        M,
        "COUNTROWS(ROLLUPADDISSUBTOTAL(Sales[Channel], \"T\"))",
        "misplaced-function",
    );
    let q = "EVALUATE SUMMARIZECOLUMNS(ROLLUPADDISSUBTOTAL(Sales[Channel], \"T\"), \"x\", IGNORE([Total]))";
    assert!(
        !query_codes(q).contains(&"misplaced-function"),
        "{:?}",
        query_codes(q)
    );
}

#[test]
fn argument_shapes_of_specific_functions() {
    rejects(
        M,
        "CALCULATE(1, CROSSFILTER(Sales[Quantity], Sales[Quantity], BOTH))",
        "no-relationship",
    );
    accepts(
        M,
        "CALCULATE(1, CROSSFILTER(Sales[ProductKey], 'Product'[ProductKey], BOTH))",
    );
    rejects(M, "CONTAINSROW(Sales, 1)", "table-shape");
    accepts(M, "CONTAINSROW(VALUES(Sales[Channel]), \"a\")");
    rejects(M, "COUNTROWS(TREATAS(Sales, 'Product'[Category]))", "table-shape");
    accepts(
        M,
        "COUNTROWS(TREATAS(VALUES(Sales[Channel]), 'Product'[Category]))",
    );
    rejects(
        CT,
        "DATATABLE(\"a\", STRING, \"b\", {{\"x\"}})",
        "datatable-pairs",
    );
    rejects(M, "PATHITEM(\"a|b\", 1, \"a\")", "invalid-path-item-type");
    accepts(M, "PATHITEM(\"a|b\", 1, 1)");
    assert!(query_codes("EVALUATE INFO.TABLES(1, 2)").contains(&"info-restriction-name"));
    assert!(query_codes("EVALUATE INFO.TABLES(\"Name\", 'Product')").contains(&"scalar-expected"));
    assert!(query_codes("EVALUATE INFO.TABLES(\"Name\", \"Sales\")").is_empty());
}
