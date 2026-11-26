//! The DAX oracle corpus: every expression, by category and role (see `dax_corpus`). All of them
//! are original, written for the synthetic model in `crates/dax/tests/oracle/model.bim` (tables
//! Customer, Product, Date, Sales, Top Products).
//!
//! Role codes:
//!   m          measure on Sales            m:T   measure on table T
//!   c:T        calculated column of T      t     calculated table
//!   r:T        row-level security filter on T
//!   q          DAX query (DEFINE / EVALUATE)
//! The probe object is named `Probe` (measure / column), `Probe Table` (table) or role
//! `Probe Role`. `add_with` adds helper objects next to the probe, e.g. the other half of a
//! circular pair. `many_sc` wraps each item as `COUNTROWS(SUMMARIZECOLUMNS(<item>))`.

use serde_json::json;

use super::dax_corpus::Corpus;

pub fn fill(e: &mut Corpus) {
    // --- syntax ---------------------------------------------------------------------------------
    e.many(
        "syntax",
        "m",
        &[
            "SUM(Sales[Quantity]",
            "SUM Sales[Quantity])",
            "1 +",
            "* 2",
            "CALCULATE([Total Sales],, )",
            "IF(TRUE(), 1, 2))",
            "\"unterminated",
            "Sales[Quantity",
            "'Sales[Quantity]",
            "VAR a = 1",
            "VAR = 1 RETURN 2",
            "VAR a 1 RETURN a",
            "RETURN 1",
            "1 2",
            "SUM(Sales[Quantity]) SUM(Sales[UnitPrice])",
            "[Total Sales] + + 1",
            "(([Total Sales])",
            "{1, 2",
            "{(1, 2), (3)}",
            "SUMX(Sales; Sales[Quantity])",
            "1 === 1",
            "IF(1 < 2 THEN 1 ELSE 0)",
            "CALCULATE([Total Sales] Sales[Channel] = \"Web\")",
            "Sales[[Quantity]]",
            "MAX(Sales[Quantity]) // trailing comment",
            "/* block comment */ MIN(Sales[Quantity])",
            "-- line comment\nCOUNTROWS(Sales)",
            "/* never closed MIN(Sales[Quantity])",
            "@x + 1",
            "1 &&",
            "NOT",
            "1 + 2 3",
        ],
    );

    // --- unknown objects ------------------------------------------------------------------------
    e.many(
        "unknown-object",
        "m",
        &[
            "SUM(Sales[Qty])",
            "SUM(Sale[Quantity])",
            "SUM('Sales Data'[Quantity])",
            "[Total Revenue]",
            "[Total Sales] + [Margin]",
            "COUNTROWS(Orders)",
            "COUNTROWS('Order Lines')",
            "SUMM(Sales[Quantity])",
            "CALCULATEX([Total Sales])",
            "TOTALMTD([Total Sales], 'Date'[Day])",
            "AVERAGE(Product[Price])",
            "SUM(sales[quantity])",
            "SUM('SALES'[QUANTITY])",
            "[total sales] * 2",
            "Sales[Total Sales]",
            "Customer[Total Sales]",
            "MAX('Top Products'[ListPrice])",
            "MAX('Top Products'[Category])",
            "MAX('Date'[Weekday])",
            "COUNTROWS(Probe)",
            "SUM([Quantity])",
            "DISTINCTCOUNT(Customer[Email])",
            "LOOKUPVALUE(Product[Category], Product[SKU], 10)",
            "FOO()",
            "Sales",
        ],
    );

    // --- arity ----------------------------------------------------------------------------------
    e.many(
        "arity",
        "m",
        &[
            "SUM(Sales[Quantity], Sales[UnitPrice])",
            "SUM()",
            "DIVIDE([Total Sales])",
            "DIVIDE([Total Sales], 2, 0, 1)",
            "IF(TRUE())",
            "IF(TRUE(), 1, 2, 3)",
            "BLANK(1)",
            "TODAY(1)",
            "CALCULATE()",
            "SUMX(Sales)",
            "FILTER(Sales)",
            "LEFT(\"abc\")",
            "LEFT(\"abc\", 1, 2)",
            "ROUND(1.5)",
            "SWITCH(1)",
            "SWITCH(1, 1)",
            "COALESCE(1)",
            "COALESCE(BLANK(), 1)",
            "RANKX(ALL(Product))",
            "DATE(2024, 1)",
            "FORMAT(1)",
            "CONCATENATE(\"a\")",
            "CONCATENATE(\"a\", \"b\", \"c\")",
            "MAXX(Sales)",
            "TRUE(1)",
        ],
    );

    // --- types ----------------------------------------------------------------------------------
    e.many(
        "types",
        "m",
        &[
            "SUM(1)",
            "SUM(Sales)",
            "SUM([Total Sales])",
            "AVERAGE(\"abc\")",
            "COUNTROWS(Sales[Quantity])",
            "COUNTROWS(1)",
            "SUMX(1, 2)",
            "SUMX([Total Sales], 1)",
            "CALCULATE(Sales)",
            "ALL(Product)",
            "VALUES(Product[Category])",
            "Product",
            "FILTER(Sales, TRUE())",
            "DISTINCTCOUNT(1)",
            "MAX(Sales)",
            "LEFT(Sales, 2)",
            "\"a\" + 1",
            "\"5\" + 1",
            "\"a\" * \"b\"",
            "TRUE() + 1",
            "1 & 2",
            "DATE(2024, 1, 1) + 1",
            "DATE(2024, 1, 1) - DATE(2023, 1, 1)",
            "\"2024-01-01\" + 0",
            "UPPER(123)",
            "LEN(42)",
            "YEAR(\"abc\")",
            "YEAR(\"2024-03-01\")",
            "ABS(\"x\")",
            "ABS(TRUE())",
            "EOMONTH(\"x\", 0)",
            "INT(\"12\")",
            "VALUE(\"12.5\") * 2",
            "VALUE(\"twelve\")",
            "1 = \"1\"",
            "1 < \"2\"",
            "TRUE() = 1",
            "DATE(2024, 1, 1) > \"a\"",
            "ROW(\"a\", 1)",
            "{1, 2, 3}",
            "{1}",
        ],
    );

    // --- row context ----------------------------------------------------------------------------
    e.many(
        "row-context",
        "m",
        &[
            "Sales[Quantity]",
            "Sales[Quantity] * 2",
            "Sales[Quantity] * Sales[UnitPrice]",
            "IF(Sales[IsReturn], 0, 1)",
            "Customer[City] & \"!\"",
            "CALCULATE(Sales[Quantity])",
            "SUMX(Sales, Sales[Quantity] * Sales[UnitPrice])",
            "SUMX(Product, Sales[Quantity])",
            "SUMX(Product, RELATED(Sales[Quantity]))",
            "SUMX(Sales, Product[ListPrice])",
            "SUMX(Sales, RELATED(Product[ListPrice]))",
            "MAXX(Customer, Customer[BirthDate])",
            "COUNTX(Sales, Customer[Name])",
            "Sales[Line Amount] / [Total Sales]",
            "SELECTEDVALUE(Product[Category])",
            "HASONEVALUE(Product[Category])",
            "MIN(Sales[Quantity]) + Sales[Quantity]",
        ],
    );
    e.many(
        "row-context",
        "c:Sales",
        &[
            "Sales[Quantity] * 2",
            "[Quantity] * [UnitPrice]",
            "Product[ListPrice]",
            "RELATED(Product[ListPrice])",
            "RELATED(Product[ListPrice]) * Sales[Quantity]",
            "Customer[Name]",
            "RELATED(Customer[Name])",
            "[Total Sales]",
            "SUM(Sales[Quantity])",
            "CALCULATE(SUM(Sales[Quantity]))",
            "IF(Sales[IsReturn], -1, 1) * Sales[Line Amount]",
            "'Date'[Year]",
            "RELATED('Date'[Year])",
            "YEAR(Sales[OrderDate])",
            "Sales[OrderDate] - RELATED(Customer[BirthDate])",
        ],
    );
    e.many(
        "row-context",
        "c:Product",
        &[
            "Product[ListPrice] * 1.2",
            "Sales[Quantity]",
            "COUNTROWS(RELATEDTABLE(Sales))",
            "SUMX(RELATEDTABLE(Sales), Sales[Quantity])",
            "CALCULATE(SUM(Sales[Quantity]))",
            "[Total Quantity]",
            "RELATED(Sales[Quantity])",
            "UPPER(LEFT(Product[Product], 3))",
        ],
    );
    e.many(
        "row-context",
        "c:Customer",
        &[
            "DATEDIFF(Customer[BirthDate], DATE(2025, 1, 1), YEAR)",
            "IF(Customer[IsActive], \"Active\", \"Dormant\")",
            "[Customer Count]",
            "CALCULATE([Total Sales])",
            "MAXX(RELATEDTABLE(Sales), Sales[OrderDate])",
        ],
    );

    // --- RELATED / EARLIER ----------------------------------------------------------------------
    e.many(
        "related-earlier",
        "m",
        &[
            "RELATED(Product[Category])",
            "EARLIER(Sales[Quantity])",
            "SUMX(Sales, EARLIER(Sales[Quantity]))",
            "SUMX(Sales, SUMX(Sales, EARLIER(Sales[Quantity])))",
            "SUMX(Customer, RELATED(Product[ListPrice]))",
            "COUNTROWS(RELATEDTABLE(Sales))",
            "SUMX(Customer, COUNTROWS(RELATEDTABLE(Sales)))",
            "RELATED(1)",
            "RELATED(Product)",
            "EARLIEST(Sales[Quantity])",
            "SUMX(Product, RELATED(Product[ListPrice]))",
            "EARLIER(Sales[Quantity], 2)",
        ],
    );
    e.many(
        "related-earlier",
        "c:Sales",
        &[
            "EARLIER(Sales[Quantity])",
            "COUNTROWS(FILTER(Sales, Sales[CustomerKey] = EARLIER(Sales[CustomerKey])))",
            "SUMX(FILTER(Sales, Sales[OrderDate] <= EARLIER(Sales[OrderDate])), Sales[Quantity])",
            "COUNTROWS(FILTER(Sales, Sales[Quantity] > EARLIER(Sales[Quantity], 2)))",
            "RELATED(Product[Category]) & \"/\" & RELATED(Customer[Segment])",
            "RELATED(Sales[Quantity])",
            "RELATED('Top Products'[Product])",
            "RELATEDTABLE(Product)",
            "COUNTROWS(RELATEDTABLE(Product))",
            "EARLIEST(Sales[OrderDate])",
        ],
    );
    e.many(
        "related-earlier",
        "c:Customer",
        &[
            "RELATED(Sales[Quantity])",
            "COUNTROWS(FILTER(Customer, Customer[City] = EARLIER(Customer[City])))",
            "RANKX(Customer, Customer[BirthDate])",
        ],
    );

    // --- CALCULATE filter rules -----------------------------------------------------------------
    e.many(
        "calculate",
        "m",
        &[
            "CALCULATE([Total Sales], Sales[Channel] = \"Web\")",
            "CALCULATE([Total Sales], Sales[Channel] = \"Web\", Sales[IsReturn] = FALSE())",
            "CALCULATE([Total Sales], Sales[Quantity] > 1 && Sales[Quantity] < 5)",
            "CALCULATE([Total Sales], Sales[Quantity] > 1 && Sales[UnitPrice] > 10)",
            "CALCULATE([Total Sales], Sales[Quantity] > 1 || Product[Category] = \"Office\")",
            "CALCULATE([Total Sales], [Order Count] > 1)",
            "CALCULATE([Total Sales], Sales[Quantity] > [Total Quantity])",
            "CALCULATE([Total Sales], Sales[Quantity] > AVERAGE(Sales[Quantity]))",
            "CALCULATE([Total Sales], FILTER(Sales, Sales[Quantity] > AVERAGE(Sales[Quantity])))",
            "CALCULATE([Total Sales], FILTER(ALL(Product), [Total Sales] > 100))",
            "CALCULATE([Total Sales], Product[Category] IN {\"Office\", \"Outdoor\"})",
            "CALCULATE([Total Sales], NOT Sales[IsReturn])",
            "CALCULATE([Total Sales], Sales[IsReturn])",
            "CALCULATE([Total Sales], TRUE())",
            "CALCULATE([Total Sales], 1)",
            "CALCULATE([Total Sales], \"Web\")",
            "CALCULATE([Total Sales], ALL(Sales))",
            "CALCULATE([Total Sales], REMOVEFILTERS())",
            "CALCULATE([Total Sales], REMOVEFILTERS(Product[Category]))",
            "CALCULATE([Total Sales], ALLEXCEPT(Sales, Product[Category]))",
            "CALCULATE([Total Sales], KEEPFILTERS(Product[Category] = \"Kitchen\"))",
            "CALCULATE([Total Sales], USERELATIONSHIP(Sales[OrderDate], 'Date'[Date]))",
            "CALCULATE([Total Sales], USERELATIONSHIP(Sales[Quantity], Product[Weight]))",
            "CALCULATE([Total Sales], CROSSFILTER(Sales[ProductKey], Product[ProductKey], BOTH))",
            "CALCULATE([Total Sales], Product[Category] = \"Office\", ALL(Product[Category]))",
            "CALCULATE(SUM(Sales[Quantity]), Sales[Channel] = Customer[City])",
            "CALCULATE([Total Sales], Sales[OrderDate] >= DATE(2025, 1, 1))",
            "CALCULATE([Total Sales], Sales[OrderDate] >= TODAY() - 365)",
            "CALCULATE([Total Sales], LEFT(Sales[Channel], 1) = \"W\")",
            "CALCULATE([Total Sales], Sales[Quantity] * 2 > 3)",
            "CALCULATE([Total Sales], SEARCH(\"o\", Sales[Channel], 1, 0) > 0)",
            "CALCULATETABLE(Sales, Sales[Channel] = \"Web\")",
            "COUNTROWS(CALCULATETABLE(Sales, Sales[Channel] = \"Web\"))",
            "CALCULATE([Total Sales], VALUES(Product[Category]))",
            "CALCULATE([Total Sales], Product[Category] = SELECTEDVALUE(Customer[Segment]))",
        ],
    );

    // --- iterators ------------------------------------------------------------------------------
    e.many(
        "iterators",
        "m",
        &[
            "SUMX(Sales, Sales[Quantity])",
            "SUMX(Sales, [Total Sales])",
            "AVERAGEX(Product, [Total Sales])",
            "MAXX(VALUES(Product[Category]), [Total Sales])",
            "MINX(FILTER(Customer, Customer[IsActive]), [Total Sales])",
            "COUNTX(Sales, Sales[Discount])",
            "COUNTAX(Sales, Sales[Channel])",
            "PRODUCTX(Product, 1 + Product[Weight] / 100)",
            "CONCATENATEX(Product, Product[Product], \", \")",
            "CONCATENATEX(Product, Product[Product], \", \", Product[ListPrice], DESC)",
            "CONCATENATEX(Product, Product, \", \")",
            "SUMX(Sales, Sales)",
            "SUMX(VALUES(Sales[Channel]), Sales[Quantity])",
            "SUMX(VALUES(Sales[Channel]), CALCULATE(SUM(Sales[Quantity])))",
            "SUMX(ADDCOLUMNS(Product, \"@Rev\", [Total Sales]), [@Rev])",
            "SUMX(SUMMARIZE(Sales, Product[Category], \"Q\", SUM(Sales[Quantity])), [Q])",
            "SUMX(GENERATESERIES(1, 5), [Value] * 2)",
            "AVERAGEX(TOPN(2, Product, [Total Sales]), [Total Sales])",
            "RANKX(ALL(Product[Product]), [Total Sales])",
            "RANKX(ALL(Product[Product]), [Total Sales], , ASC, Dense)",
            "RANKX(ALL(Product[Product]), [Total Sales], , SIDEWAYS)",
            "MAXX(Sales, Sales[Quantity], Sales[UnitPrice])",
            "SUMX(Sales, Sales[Quantity] * RELATED(Product[ListPrice]))",
            "SUMX(FILTER(Sales, RELATED(Customer[IsActive])), Sales[Line Amount])",
            "AVERAGEX(Sales, DATEDIFF(RELATED(Customer[BirthDate]), Sales[OrderDate], YEAR))",
        ],
    );

    // --- time intelligence ----------------------------------------------------------------------
    e.many(
        "time-intelligence",
        "m",
        &[
            "TOTALYTD([Total Sales], 'Date'[Date])",
            "TOTALYTD([Total Sales], 'Date'[Date], \"06-30\")",
            "TOTALQTD([Total Sales], 'Date'[Date])",
            "TOTALMTD([Total Sales], 'Date'[Date])",
            "CALCULATE([Total Sales], DATESYTD('Date'[Date]))",
            "CALCULATE([Total Sales], DATEADD('Date'[Date], -1, YEAR))",
            "CALCULATE([Total Sales], DATEADD('Date'[Date], -1, WEEK))",
            "CALCULATE([Total Sales], DATEADD('Date'[Date], -3, MONTH))",
            "CALCULATE([Total Sales], DATEADD('Date'[Date], 1, DECADE))",
            "CALCULATE([Total Sales], PARALLELPERIOD('Date'[Date], -1, QUARTER))",
            "CALCULATE([Total Sales], SAMEPERIODLASTYEAR('Date'[Date]))",
            "CALCULATE([Total Sales], PREVIOUSMONTH('Date'[Date]))",
            "CALCULATE([Total Sales], NEXTYEAR('Date'[Date]))",
            "CALCULATE([Total Sales], DATESINPERIOD('Date'[Date], MAX('Date'[Date]), -12, MONTH))",
            "CALCULATE([Total Sales], DATESBETWEEN('Date'[Date], DATE(2024, 1, 1), DATE(2024, 6, 30)))",
            "CALCULATE([Total Sales], DATESMTD(Sales[OrderDate]))",
            "CALCULATE([Total Sales], DATESYTD('Date'[Year]))",
            "CALCULATE([Total Sales], DATEADD('Date'[Date], -1, \"YEAR\"))",
            "CALCULATE([Total Sales], DATEADD('Date'[Date], \"one\", YEAR))",
            "CALCULATE([Total Sales], DATEADD('Date', -1, YEAR))",
            "CLOSINGBALANCEMONTH([Total Sales], 'Date'[Date])",
            "OPENINGBALANCEYEAR([Total Sales], 'Date'[Date])",
            "FIRSTDATE('Date'[Date])",
            "LASTDATE('Date'[Date]) - FIRSTDATE('Date'[Date])",
            "STARTOFYEAR('Date'[Date])",
            "ENDOFMONTH(Sales[OrderDate])",
            "DIVIDE([Total Sales] - [Sales LY], [Sales LY])",
            "CALCULATE([Total Sales], DATESYTD(Date[Date]))",
            "CALCULATE([Total Sales], DATEADD(LASTDATE('Date'[Date]), -1, YEAR))",
            "CALCULATE([Total Sales], SAMEPERIODLASTYEAR(VALUES('Date'[Date])))",
        ],
    );

    // --- table constructors ---------------------------------------------------------------------
    e.many(
        "table-constructor",
        "m",
        &[
            "COUNTROWS({1, 2, 3})",
            "COUNTROWS({(1, \"a\"), (2, \"b\")})",
            "SUMX({1, 2, 3}, [Value])",
            "SUMX({(1, 2), (3, 4)}, [Value1] + [Value2])",
            "SUMX({(1, 2), (3, 4)}, [Value3])",
            "SUMX({1, 2}, [Value1])",
            "MAXX({(1, 2), (3)}, [Value1])",
            "COUNTROWS({})",
            "COUNTROWS({BLANK()})",
            "COUNTROWS({[Total Sales], [Order Count]})",
            "COUNTROWS(DATATABLE(\"k\", INTEGER, {{1}, {2}}))",
            "COUNTROWS(DATATABLE(\"k\", INTEGER, \"v\", STRING, {{1, \"a\"}, {2, \"b\"}}))",
            "COUNTROWS(DATATABLE(\"k\", WHOLE, {{1}}))",
            "COUNTROWS(DATATABLE(\"k\", INTEGER, {{[Total Sales]}}))",
            "COUNTROWS(ROW(\"a\", 1, \"b\", 2))",
            "COUNTROWS(ROW(\"a\", 1, \"a\", 2))",
            "COUNTROWS(ROW(\"a\"))",
            "COUNTROWS(GENERATESERIES(1, 10, 2))",
            "COUNTROWS(GENERATESERIES(\"a\", \"z\"))",
            "COUNTROWS(UNION({1, 2}, {3}))",
            "COUNTROWS(UNION({1, 2}, {(3, 4)}))",
        ],
    );

    // --- variables ------------------------------------------------------------------------------
    e.many(
        "variables",
        "m",
        &[
            "VAR total = [Total Sales] RETURN total * 2",
            "VAR a = 1 VAR b = a + 1 RETURN a + b",
            "VAR a = b + 1 VAR b = 1 RETURN a",
            "VAR a = 1 VAR a = 2 RETURN a",
            "VAR a = 1 RETURN b",
            "VAR t = FILTER(Sales, Sales[Quantity] > 1) RETURN COUNTROWS(t)",
            "VAR t = FILTER(Sales, Sales[Quantity] > 1) RETURN SUMX(t, Sales[Quantity])",
            "VAR t = FILTER(Sales, Sales[Quantity] > 1) RETURN SUMX(t, t[Quantity])",
            "VAR t = ADDCOLUMNS(Product, \"@x\", 1) RETURN SUMX(t, [@x])",
            "VAR t = Sales RETURN t",
            "VAR x = 1 RETURN VAR y = x + 1 RETURN y",
            "SUMX(Sales, VAR q = Sales[Quantity] RETURN q * 2)",
            "VAR q = Sales[Quantity] RETURN q",
            "VAR Sales = 1 RETURN Sales",
            "VAR SUM = 1 RETURN SUM",
            "VAR _x = 2 RETURN _x ^ 2",
            "VAR 1x = 2 RETURN 1x",
            "VAR x = 1 RETURN CALCULATE([Total Sales], Sales[Quantity] > x)",
            "VAR m = MAX(Sales[OrderDate]) RETURN CALCULATE([Total Sales], 'Date'[Date] <= m)",
            "VAR cat = \"Office\" RETURN CALCULATE([Total Sales], Product[Category] = cat)",
            "VAR x = 1 RETURN x[Value]",
            "VAR x = {1, 2} RETURN SUMX(x, [Value])",
            "VAR x = 1 RETURN",
            "VAR `x` = 1 RETURN 1",
            "VAR x = 1 VAR y = SUMX(Sales, x) RETURN y",
        ],
    );

    // --- coercion -------------------------------------------------------------------------------
    e.many(
        "coercion",
        "m",
        &[
            "\"10\" * 2",
            "\"10\" & 2",
            "10 & \" units\"",
            "\"abc\" & TRUE()",
            "TRUE() & \"x\"",
            "\"1\" = 1",
            "VALUE(\"3\") + 1",
            "CONVERT(\"12\", INTEGER)",
            "CONVERT(1.7, STRING)",
            "CONVERT(\"x\", DOUBLE)",
            "CONVERT(1, TEXT)",
            "FORMAT([Total Sales], \"0.00\")",
            "FORMAT(TODAY(), \"yyyy-mm-dd\")",
            "INT(TRUE())",
            "CURRENCY(1.23456)",
            "FIXED(1234.5, 1)",
            "DATEVALUE(\"2024-02-29\")",
            "DATEVALUE(\"not a date\")",
            "TIMEVALUE(\"10:30\")",
            "DATE(2024, 13, 1)",
            "UNICHAR(65) & UNICODE(\"A\")",
            "IF(1, \"yes\", \"no\")",
            "IF(\"x\", 1, 0)",
            "NOT 5",
            "-\"3\"",
        ],
    );

    // --- blanks ---------------------------------------------------------------------------------
    e.many(
        "blank",
        "m",
        &[
            "BLANK()",
            "BLANK() + 1",
            "BLANK() & \"x\"",
            "IF(ISBLANK([Total Sales]), 0, [Total Sales])",
            "COALESCE([Total Sales], 0)",
            "[Total Sales] = BLANK()",
            "[Total Sales] == BLANK()",
            "DIVIDE(1, 0)",
            "1 / 0",
            "DIVIDE(1, BLANK(), -1)",
            "IF(FALSE(), 1)",
            "ISBLANK()",
            "BLANK() = 0",
            "NOT ISBLANK(SELECTEDVALUE(Product[Category]))",
            "IFERROR(1 / 0, BLANK())",
        ],
    );

    // --- operators ------------------------------------------------------------------------------
    e.many(
        "operators",
        "m",
        &[
            "1 + 2 * 3",
            "(1 + 2) * 3",
            "2 ^ 3 ^ 2",
            "-2 ^ 2",
            "- -1",
            "10 % 3",
            "MOD(10, 3)",
            "1 <> 2",
            "1 != 2",
            "1 =< 2",
            "1 >= 2 && 3 <= 4",
            "TRUE() || FALSE() && FALSE()",
            "NOT TRUE() && FALSE()",
            "NOT(TRUE())",
            "\"a\" & \"b\" & \"c\"",
            "1 + 2 & 3",
            "AND(TRUE(), FALSE())",
            "AND(TRUE(), FALSE(), TRUE())",
            "OR(1 > 0, 2 > 3)",
            "1 + TRUE",
            "TRUE",
            "true() && FALSE()",
            "1.5e3 + .5",
            "1,5 + 2",
            "1 & & 2",
            "+5",
            "[Total Sales] / [Order Count] * 100",
        ],
    );

    // --- IN -------------------------------------------------------------------------------------
    e.many(
        "in-operator",
        "m",
        &[
            "IF(\"Web\" IN {\"Web\", \"Store\"}, 1, 0)",
            "IF(SELECTEDVALUE(Sales[Channel]) IN {\"Web\", \"Phone\"}, [Total Sales])",
            "IF((1, 2) IN {(1, 2), (3, 4)}, 1, 0)",
            "IF(1 IN {(1, 2)}, 1, 0)",
            "IF(1 IN 1, 1, 0)",
            "IF(1 IN VALUES(Sales[Quantity]), 1, 0)",
            "IF(1 IN Sales, 1, 0)",
            "IF(NOT (2 IN {1, 2}), 1, 0)",
            "CONTAINSROW({1, 2}, 2)",
            "IF(1 IN {}, 1, 0)",
            "IF(1 in {1}, 1, 0)",
        ],
    );

    // --- deprecated / unusual functions ---------------------------------------------------------
    e.many(
        "deprecated",
        "m",
        &[
            "CALCULATE([Total Sales], ALLNOBLANKROW(Product))",
            "COUNTROWS(ALLNOBLANKROW(Product[Category]))",
            "IF(CONTAINS(Product, Product[Category], \"Office\"), 1, 0)",
            "EXACT(\"a\", \"A\")",
            "COUNTROWS(SUMMARIZE(Sales, Sales[Channel], \"Q\", SUM(Sales[Quantity])))",
            "FIRSTNONBLANK(Product[Product], [Total Sales])",
            "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", SUMX(CURRENTGROUP(), Sales[Quantity])))",
            "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", SUM(Sales[Quantity])))",
            "USERNAME()",
            "USERPRINCIPALNAME()",
            "CUSTOMDATA()",
            "ISERROR(1 / 0)",
        ],
    );

    // --- window functions -----------------------------------------------------------------------
    e.many("window", "m", &[
        "CALCULATE([Total Sales], OFFSET(-1, ALLSELECTED('Date'[Year]), ORDERBY('Date'[Year])))",
        "CALCULATE([Total Sales], OFFSET(-1, ALLSELECTED('Date'[Year])))",
        "CALCULATE([Total Sales], WINDOW(1, ABS, 0, REL, ALLSELECTED('Date'[Month]), ORDERBY('Date'[Month])))",
        "CALCULATE([Total Sales], WINDOW(1, ABS, 0, ALLSELECTED('Date'[Month]), ORDERBY('Date'[Month])))",
        "CALCULATE([Total Sales], INDEX(1, ALLSELECTED(Product[Product]), ORDERBY([Total Sales], DESC)))",
        "RANK(DENSE, ALLSELECTED(Product[Product]), ORDERBY([Total Sales], DESC))",
        "ROWNUMBER(ALLSELECTED(Product[Product]), ORDERBY(Product[Product]))",
        "CALCULATE([Total Sales], OFFSET(-1, ALLSELECTED('Date'[Year]), ORDERBY('Date'[Year]), PARTITIONBY(Product[Category])))",
        "CALCULATE([Total Sales], OFFSET(\"x\", ALLSELECTED('Date'[Year]), ORDERBY('Date'[Year])))",
        "ORDERBY('Date'[Year])",
        "CALCULATE([Total Sales], OFFSET(-1, 'Date'[Year], ORDERBY('Date'[Year])))",
        "MOVINGAVERAGE([Total Sales], 3)",
        "RUNNINGSUM([Total Sales])",
    ]);

    // --- circular references --------------------------------------------------------------------
    e.add("circular", "m", "[Probe] + 1");
    e.add_with(
        "circular",
        "m",
        "[Helper A] * 2",
        json!([{"kind": "measure", "table": "Sales", "name": "Helper A", "expression": "[Probe] + 1"}]),
    );
    e.add_with(
        "circular",
        "m",
        "[Helper A] * 2",
        json!([{"kind": "measure", "table": "Sales", "name": "Helper A", "expression": "[Total Sales] + 1"}]),
    );
    e.add_with(
        "circular",
        "m",
        "[Helper A]",
        json!([
            {"kind": "measure", "table": "Sales", "name": "Helper A", "expression": "[Helper B]"},
            {"kind": "measure", "table": "Sales", "name": "Helper B", "expression": "[Probe]"},
        ]),
    );
    e.add("circular", "c:Sales", "Sales[Probe] + 1");
    e.add_with(
        "circular",
        "c:Sales",
        "Sales[Helper Col] + 1",
        json!([{"kind": "column", "table": "Sales", "name": "Helper Col", "expression": "Sales[Probe] * 2"}]),
    );
    e.add_with("circular", "c:Sales", "Sales[Helper Col] + 1", json!([{"kind": "column", "table": "Sales", "name": "Helper Col", "expression": "Sales[Quantity] * 2"}]));
    e.add_with("circular", "c:Sales", "CALCULATE(SUM(Sales[Quantity]))", json!([{"kind": "column", "table": "Sales", "name": "Helper Col", "expression": "CALCULATE(SUM(Sales[Quantity]))"}]));
    e.add_with(
        "circular",
        "m",
        "SUMX(Sales, Sales[Helper Col])",
        json!([{"kind": "column", "table": "Sales", "name": "Helper Col", "expression": "[Probe]"}]),
    );
    e.add("circular", "c:Product", "[Probe]");
    e.add("circular", "c:Sales", "COUNTROWS(Sales)");

    // --- calculated tables ----------------------------------------------------------------------
    e.many(
        "calc-table",
        "t",
        &[
            "Product",
            "FILTER(Sales, Sales[Quantity] > 1)",
            "ALL(Product[Category])",
            "VALUES(Customer[City])",
            "DISTINCT(Sales[Channel])",
            "SUMMARIZECOLUMNS(Product[Category], \"Revenue\", [Total Sales])",
            "SUMMARIZECOLUMNS(Product[Category], \"Revenue\", 1 + 1)",
            "ADDCOLUMNS(VALUES(Product[Category]), \"Revenue\", [Total Sales])",
            "ADDCOLUMNS(VALUES(Product[Category]), \"Revenue\")",
            "SELECTCOLUMNS(Customer, \"Who\", Customer[Name], \"Where\", Customer[City])",
            "SELECTCOLUMNS(Customer, \"Who\", Customer[Name], \"Who\", Customer[City])",
            "SELECTCOLUMNS(Customer, Customer[Name])",
            "CALENDAR(DATE(2024, 1, 1), DATE(2024, 3, 31))",
            "CALENDARAUTO()",
            "GENERATESERIES(0, 100, 5)",
            "{1, 2, 3}",
            "{(\"a\", 1), (\"b\", 2)}",
            "CROSSJOIN(VALUES(Product[Category]), VALUES(Sales[Channel]))",
            "CROSSJOIN(Product, Product)",
            "NATURALINNERJOIN(Sales, Product)",
            "TOPN(2, Customer, Customer[BirthDate], ASC)",
            "UNION(VALUES(Customer[City]), VALUES(Product[Category]))",
            "EXCEPT(VALUES(Customer[City]), {\"Lyon\"})",
            "INTERSECT(VALUES(Customer[CustomerKey]), VALUES(Sales[CustomerKey]))",
            "GENERATE(VALUES(Product[Category]), TOPN(1, Product, Product[ListPrice]))",
            "ROW(\"Total\", [Total Sales])",
            "1",
            "[Total Sales]",
            "Sales[Quantity]",
            "FILTER(Sales, [Probe] > 0)",
            "TREATAS({\"Office\"}, Product[Category])",
            "DATATABLE(\"Code\", STRING, \"Rate\", DOUBLE, {{\"A\", 0.1}, {\"B\", 0.2}})",
            "SUMMARIZE(Sales, Product[Category])",
            "SUMMARIZE(Sales, Product[Category], Customer[City])",
            "SUMMARIZE(Product, Customer[City])",
        ],
    );

    // --- row-level security ---------------------------------------------------------------------
    e.many(
        "rls",
        "r:Customer",
        &[
            "Customer[City] = \"Lyon\"",
            "Customer[IsActive]",
            "TRUE()",
            "FALSE()",
            "Customer[Segment] IN {\"Retail\", \"Online\"}",
            "Customer[Name] = USERPRINCIPALNAME()",
            "[Customer Count] > 0",
            "Customer[City]",
            "1",
            "Customer[Nope] = 1",
            "Customer[BirthDate] < DATE(1990, 1, 1)",
            "LOOKUPVALUE(Customer[City], Customer[CustomerKey], 1) = Customer[City]",
            "Sales[Channel] = \"Web\"",
            "SUM(Customer[CustomerKey]) > 0",
        ],
    );
    e.many(
        "rls",
        "r:Sales",
        &[
            "Sales[Channel] <> \"Phone\"",
            "RELATED(Product[Category]) = \"Office\"",
            "Product[Category] = \"Office\"",
            "Sales[OrderDate] >= DATE(2025, 1, 1)",
        ],
    );

    // --- queries --------------------------------------------------------------------------------
    e.many(
        "query",
        "q",
        &[
            "EVALUATE Product",
            "EVALUATE VALUES(Product[Category])",
            "EVALUATE ROW(\"x\", [Total Sales])",
            "EVALUATE [Total Sales]",
            "EVALUATE {[Total Sales]}",
            "EVALUATE SUMMARIZECOLUMNS(Product[Category], \"Rev\", [Total Sales]) ORDER BY [Rev] DESC",
            "EVALUATE SUMMARIZECOLUMNS(Product[Category], \"Rev\", [Total Sales]) ORDER BY Product[Category]",
            "EVALUATE Product ORDER BY Product[ListPrice] DESC START AT 50",
            "DEFINE MEASURE Sales[Double] = [Total Sales] * 2 EVALUATE ROW(\"d\", [Double])",
            "DEFINE MEASURE Sales[Double] = [Total Sales] * 2",
            "DEFINE VAR x = 3 EVALUATE ROW(\"x\", x)",
            "DEFINE VAR t = FILTER(Product, Product[ListPrice] > 30) EVALUATE t",
            "DEFINE COLUMN Product[Pricey] = Product[ListPrice] > 30 EVALUATE Product",
            "DEFINE TABLE Cats = VALUES(Product[Category]) EVALUATE Cats",
            "DEFINE MEASURE Nope[X] = 1 EVALUATE ROW(\"x\", 1)",
            "DEFINE MEASURE Sales[Total Sales] = 1 EVALUATE ROW(\"x\", [Total Sales])",
            "DEFINE MEASURE Sales[A] = [B] MEASURE Sales[B] = [A] EVALUATE ROW(\"x\", [A])",
            "EVALUATE Product EVALUATE Customer",
            "EVALUATE FILTER(Sales, Sales[Quantity] > 2)",
            "EVALUATE Nope",
            "EVALUATE",
            "EVALUATE CALCULATETABLE(Sales, Product[Category] = \"Office\")",
            "EVALUATE TOPN(3, Product, [Total Sales]) ORDER BY [Total Sales] DESC",
            "DEFINE VAR x = 1 VAR x = 2 EVALUATE ROW(\"x\", x)",
            "EVALUATE ROW(\"x\", 1) ORDER BY",
            "evaluate row(\"x\", 1)",
            "DEFINE FUNCTION Twice = (v) => v * 2 EVALUATE ROW(\"x\", Twice(21))",
            "EVALUATE SELECTCOLUMNS(Sales, \"k\", Sales[SalesKey]) ORDER BY [k]",
        ],
    );

    // --- realistic valid measures ---------------------------------------------------------------
    e.many("realistic", "m", &[
        "DIVIDE([Total Sales], [Order Count])",
        "DIVIDE([Total Sales], CALCULATE([Total Sales], ALL(Product)))",
        "DIVIDE([Total Sales], CALCULATE([Total Sales], ALLSELECTED(Product[Category])))",
        "SUMX(VALUES(Customer[CustomerKey]), IF([Total Sales] > 100, 1, 0))",
        "CALCULATE(DISTINCTCOUNT(Sales[CustomerKey]), Sales[IsReturn] = FALSE())",
        "VAR Cur = [Total Sales] VAR Prev = [Sales LY] RETURN IF(NOT ISBLANK(Prev), DIVIDE(Cur - Prev, Prev))",
        "SWITCH(TRUE(), [Total Sales] > 500, \"High\", [Total Sales] > 100, \"Mid\", \"Low\")",
        "SWITCH(SELECTEDVALUE(Sales[Channel]), \"Web\", 1, \"Store\", 2, 0)",
        "CALCULATE([Total Sales], FILTER(VALUES(Customer[CustomerKey]), [Order Count] >= 2))",
        "AVERAGEX(VALUES('Date'[Month]), [Total Sales])",
        "MAXX(TOPN(1, VALUES(Product[Product]), [Total Sales]), Product[Product])",
        "CONCATENATEX(TOPN(3, VALUES(Customer[Name]), [Total Sales]), Customer[Name], \", \")",
        "CALCULATE([Total Quantity], Sales[Discount] > 0)",
        "SUMX(Sales, Sales[Quantity] * (RELATED(Product[ListPrice]) - Sales[UnitPrice]))",
        "CALCULATE([Customer Count], Customer[IsActive] = TRUE())",
        "IF(HASONEVALUE(Product[Category]), [Total Sales], BLANK())",
        "IF(ISINSCOPE(Product[Product]), [Total Sales], [Total Sales] / 2)",
        "CALCULATE(MAX(Sales[OrderDate]), ALL(Sales))",
        "DATEDIFF(MIN(Sales[OrderDate]), MAX(Sales[OrderDate]), DAY)",
        "COUNTROWS(FILTER(Customer, CALCULATE(COUNTROWS(Sales)) = 0))",
        "CALCULATE([Total Sales], TREATAS(VALUES(Customer[City]), Customer[City]))",
        "SUMX(KEEPFILTERS(VALUES(Product[Category])), [Total Sales])",
        "VAR Threshold = PERCENTILEX.INC(Product, Product[ListPrice], 0.5) RETURN CALCULATE([Total Sales], Product[ListPrice] >= Threshold)",
        "MEDIAN(Sales[UnitPrice])",
        "STDEV.P(Sales[Quantity])",
        "ROUND(DIVIDE([Total Sales], [Total Quantity]), 2)",
        "FORMAT(DIVIDE([Total Sales], 1000), \"#,0.0\") & \"k\"",
        "LOOKUPVALUE(Product[Category], Product[ProductKey], 10)",
        "CALCULATE(SUM(Sales[Quantity]), Sales[OrderDate] >= DATE(2025, 1, 1), Sales[OrderDate] < DATE(2026, 1, 1))",
        "SELECTEDVALUE(Customer[Segment], \"Several\")",
        "ISFILTERED(Product[Category])",
        "ISCROSSFILTERED(Product)",
        "COUNTROWS(ALLSELECTED(Product))",
        "CALCULATE([Total Sales], ALL('Date'), VALUES('Date'[Year]))",
        "NOT ISEMPTY(Sales)",
        "MINX(ALL('Date'[Year]), 'Date'[Year])",
        "CALCULATE([Total Sales], Customer[Segment] = \"Retail\" || Customer[Segment] = \"Online\")",
        "UPPER(SELECTEDVALUE(Product[Category]))",
        "SUBSTITUTE(SELECTEDVALUE(Customer[Name]), \"a\", \"A\")",
        "LEN(CONCATENATEX(VALUES(Sales[Channel]), Sales[Channel]))",
        "EOMONTH(MAX(Sales[OrderDate]), 0)",
        "WEEKDAY(MAX(Sales[OrderDate]), 2)",
        "SUMX(VALUES('Date'[Year]), CALCULATE(MAX(Sales[Quantity])))",
        "COUNTBLANK(Sales[Discount])",
        "DISTINCTCOUNTNOBLANK(Sales[Channel])",
    ]);
    e.many("realistic", "m:Customer", &[
        "[Customer Count] + 0",
        "COUNTROWS(Customer)",
        "CALCULATE([Customer Count], Sales[Channel] = \"Web\")",
        "CALCULATE([Customer Count], CROSSFILTER(Sales[CustomerKey], Customer[CustomerKey], BOTH), Sales[Channel] = \"Web\")",
    ]);
    e.many(
        "realistic",
        "c:Sales",
        &[
            "IF(Sales[Discount] > 0, \"Discounted\", \"Full price\")",
            "SWITCH(Sales[Channel], \"Web\", \"Online\", \"Offline\")",
            "FORMAT(Sales[OrderDate], \"yyyy-mm\")",
            "Sales[Line Amount] / CALCULATE(SUM(Sales[Line Amount]), ALLEXCEPT(Sales, Sales[CustomerKey]))",
            "RANKX(ALL(Sales), Sales[Line Amount])",
            "LOOKUPVALUE(Product[Category], Product[ProductKey], Sales[ProductKey])",
        ],
    );
    e.many(
        "realistic",
        "c:Date",
        &[
            "'Date'[Year] * 100 + 'Date'[Month]",
            "FORMAT('Date'[Date], \"ddd\")",
            "WEEKNUM('Date'[Date], 2)",
            "IF(WEEKDAY('Date'[Date], 2) > 5, \"Weekend\", \"Weekday\")",
        ],
    );

    // --- column variations (auto date/time) -----------------------------------------------------
    // Sales[OrderDate] has a variation to the hidden LocalDateTable_0001 (as Desktop's auto date/time
    // creates it); Customer[BirthDate] has none.
    e.many("variations", "m", &[
        "CALCULATE([Total Quantity], DATESYTD('Sales'[OrderDate].[Date]))",
        "CALCULATE([Total Quantity], DATESYTD(Sales[OrderDate].[Date]))",
        "MAX('Sales'[OrderDate].[Year])",
        "MAX('Sales'[OrderDate].[Month])",
        "MAX('Sales'[OrderDate].[MonthNo])",
        "MAX('Sales'[OrderDate].[Quarter])",
        "MAX('Sales'[OrderDate].[QuarterNo])",
        "MAX('Sales'[OrderDate].[Day])",
        "MAX('Sales'[OrderDate].[Date])",
        "MAX('Sales'[OrderDate] .[Year])",
        "MAX('Sales'[OrderDate].[year])",
        "MAX([OrderDate].[Year])",
        "COUNTROWS(VALUES('Sales'[OrderDate].[Year]))",
        "CALCULATE([Total Quantity], 'Sales'[OrderDate].[Year] = 2024)",
        "SUMX(Sales, Sales[OrderDate].[MonthNo])",
        "SUMX('LocalDateTable_0001', Sales[OrderDate].[Year])",
        "COUNTROWS(FILTER(ALL(Sales), Sales[OrderDate].[Year] = 2024))",
        "TOTALYTD([Total Quantity], 'Sales'[OrderDate].[Date])",
        "CALCULATE([Total Quantity], SAMEPERIODLASTYEAR('Sales'[OrderDate].[Date]))",
        "CALCULATE([Total Quantity], DATEADD(Sales[OrderDate].[Date], -1, YEAR))",
        "DISTINCTCOUNT(Sales[OrderDate].[Month])",
        "CALCULATE([Total Quantity], Sales[OrderDate].[Year] = 2024, Sales[OrderDate].[Month] = \"January\")",
        "CALCULATE([Total Quantity], Sales[OrderDate].[Year] = 2024 && 'LocalDateTable_0001'[MonthNo] = 1)",
        "CALCULATE([Total Quantity], ALL(Sales[OrderDate].[Year]))",
        "SELECTEDVALUE(Sales[OrderDate].[Year])",
        "COUNTROWS(FILTER(VALUES(Sales[OrderDate].[Year]), Sales[OrderDate].[Year] > 2000))",
        "MAX('Sales'[OrderDate].[Foo])",
        "MAX('Sales'[OrderDate].[Variation])",
        "MAX('Sales'[OrderDate].[Date Hierarchy])",
        "MAX('Customer'[BirthDate].[Year])",
        "MAX('Sales'[Quantity].[Year])",
        "MAX('LocalDateTable_0001'[Date].[Year])",
        "MAX('Sales'[OrderDate].Year)",
        "MAX('Sales'[OrderDate].[Date].[Year])",
        "Sales[OrderDate].[Year]",
        "SUMX(Customer, Sales[OrderDate].[Year])",
        "SUMX(Sales, RELATED(Sales[OrderDate].[Year]))",
        "CALCULATE([Total Quantity], Sales[OrderDate].[Year] = 2024 && Sales[Quantity] > 1)",
        "Sales[Total Sales].[Year]",
        "[Total Sales].[Year]",
    ]);
    e.many(
        "variations",
        "c:Sales",
        &[
            "Sales[OrderDate].[Year]",
            "[OrderDate].[MonthNo]",
            "Sales[OrderDate].[Foo]",
        ],
    );
    e.many(
        "variations",
        "c:Customer",
        &["Sales[OrderDate].[Year]", "Customer[BirthDate].[Year]"],
    );
    e.many(
        "variations",
        "q",
        &[
            "EVALUATE SUMMARIZECOLUMNS('Sales'[OrderDate].[Year], \"q\", [Total Quantity])",
            "EVALUATE VALUES('Sales'[OrderDate].[Month])",
            "EVALUATE ROW(\"y\", MAX('Customer'[BirthDate].[Year]))",
        ],
    );

    // --- reserved words (G4) ----------------------------------------------------------------------
    // Some names cannot be variables or unquoted table qualifiers; most function names can.
    e.many(
        "reserved-words",
        "m",
        &[
            "VAR Value = 1 RETURN Value",
            "VAR Rank = 1 RETURN Rank",
            "VAR Filter = 1 RETURN Filter",
            "VAR Values = 1 RETURN Values",
            "VAR Text = 1 RETURN Text",
            "VAR Asc = 1 RETURN Asc",
            "VAR Date = 1 RETURN Date",
            "VAR Year = 1 RETURN Year",
            "VAR Index = 1 RETURN Index",
            "VAR Total = 1 RETURN Total",
            "VAR Calendar = 1 RETURN Calendar",
            "VAR Window = 1 RETURN Window",
            "VAR Currency = 1 RETURN Currency",
            "MAX(Date[Year])",
            "MAX(date[Year])",
            "CALCULATE([Total Sales], Date[Year] = 2024)",
            "COUNTROWS(Date)",
            "SUMX(Date, 1)",
        ],
    );
    e.many(
        "reserved-words",
        "q",
        &[
            "EVALUATE Date",
            "EVALUATE ROW(\"x\", VAR Sum = 1 RETURN 1)",
            "EVALUATE ROW(\"x\", VAR Product = 1 RETURN 1)",
        ],
    );

    // --- GROUPBY and CURRENTGROUP (G4) ------------------------------------------------------------
    e.many("groupby", "m", &[
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", AVERAGEX(CURRENTGROUP(), Sales[Quantity])))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", COUNTX(CURRENTGROUP(), 1)))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", SUMX(CURRENTGROUP(), Sales[Quantity] * 2)))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", STDEVX.P(CURRENTGROUP(), Sales[Quantity])))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel]))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", SUMX(CURRENTGROUP(), Sales[Quantity]) + 1))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", 1))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", [Total Sales]))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", SUMX(Sales, Sales[Quantity])))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", COUNTROWS(CURRENTGROUP())))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", CONCATENATEX(CURRENTGROUP(), Sales[Channel])))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", MEDIANX(CURRENTGROUP(), Sales[Quantity])))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", MAXX(FILTER(CURRENTGROUP(), TRUE()), Sales[Quantity])))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", SUMX(CURRENTGROUP(), [Total Sales])))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", VAR x = 1 RETURN SUMX(CURRENTGROUP(), Sales[Quantity])))",
        "COUNTX(CURRENTGROUP(), 1)",
        "COUNTROWS(ADDCOLUMNS(GROUPBY(Sales, Sales[Channel]), \"Q\", SUMX(CURRENTGROUP(), Sales[Quantity])))",
    ]);

    // --- window functions: relation, clauses (G4) -------------------------------------------------
    e.many("window-rules", "m", &[
        "CALCULATE([Total Sales], OFFSET(-1, ALLSELECTED('Date'[Year]), ORDERBY('Date'[Month])))",
        "CALCULATE([Total Sales], OFFSET(-1, ALLSELECTED('Date'[Year], 'Date'[Month]), ORDERBY('Date'[Year]), PARTITIONBY('Date'[Month])))",
        "CALCULATE([Total Sales], OFFSET(-1, ALLSELECTED('Date'), ORDERBY('Date'[Year])))",
        "CALCULATE([Total Sales], OFFSET(-1, ALLSELECTED('Date'), ORDERBY('Date'[Year]), PARTITIONBY(Product[Category])))",
        "CALCULATE([Total Sales], OFFSET(-1, ALLSELECTED('Date'[Year]), ORDERBY('Date'[Year]), MATCHBY('Date'[Month])))",
        "CALCULATE([Total Sales], OFFSET(-1, ALLSELECTED(Sales), ORDERBY(Product[Category])))",
        "CALCULATE([Total Sales], OFFSET(-1, ALLSELECTED(Sales), ORDERBY(Sales[SalesKey]), PARTITIONBY(Product[Category])))",
        "CALCULATE([Total Sales], OFFSET(-1, ALLSELECTED('Date'[Year]), ORDERBY([Total Sales])))",
        "CALCULATE([Total Sales], OFFSET(-1, ALLSELECTED('Date'[Year]), ORDERBY(MAX(Sales[Quantity]))))",
        "CALCULATE([Total Sales], OFFSET(-1, ALLSELECTED('Date'[Year]), ORDERBY('Date'[Year] * 2)))",
        "CALCULATE([Total Sales], OFFSET(-1, ORDERBY('Date'[Year])))",
        "CALCULATE([Total Sales], OFFSET(-1, , ORDERBY('Date'[Year])))",
        "CALCULATE([Total Sales], OFFSET(-1, FILTER(ALLSELECTED('Date'[Year]), TRUE()), ORDERBY('Date'[Month])))",
        "CALCULATE([Total Sales], OFFSET(-1, ALLSELECTED('Date'[Year]), , , PARTITIONBY('Date'[Month])))",
        "CALCULATE([Total Sales], WINDOW(1, ABS, -1, ABS, ALLSELECTED('Date'[Year]), ORDERBY('Date'[Year]), PARTITIONBY('Date'[Month])))",
        "CALCULATE([Total Sales], INDEX(1, ALLSELECTED('Date'[Year]), ORDERBY('Date'[Year]), PARTITIONBY('Date'[Year])))",
        "CALCULATE([Total Sales], INDEX(1, 'Date'[Year]))",
        "RANK(DENSE, 'Date'[Year])",
        "ROWNUMBER('Date'[Year])",
        "RANK(ALLSELECTED('Date'[Year]), ORDERBY('Date'[Month]))",
        "ROWNUMBER(ALLSELECTED('Date'[Year]), ORDERBY('Date'[Year], DESC))",
        "ROWNUMBER(ALLSELECTED('Date'[Year]), ORDERBY('Date'[Year]), DEFAULT, PARTITIONBY('Date'[Month]))",
        "PARTITIONBY('Date'[Year])",
        "MATCHBY('Date'[Year])",
        "CALCULATE([Total Sales], ORDERBY('Date'[Year]))",
        "COUNTROWS(ORDERBY('Date'[Year]))",
    ]);
    e.many(
        "window-rules",
        "c:Date",
        &[
            "CALCULATE(SUM(Sales[Quantity]), OFFSET(-1, ALL('Date'[Year]), ORDERBY('Date'[Month])))",
            "RANK(DENSE, 'Date'[Year])",
        ],
    );

    // --- time-intelligence dates arguments (G4, G20) ----------------------------------------------
    e.many(
        "dates-argument",
        "m",
        &[
            "CALCULATE([Total Sales], PARALLELPERIOD('Date'[Date], -1, WEEK))",
            "CALCULATE([Total Sales], DATESINPERIOD('Date'[Date], DATE(2024,1,1), -7, WEEK))",
            "CALCULATE([Total Sales], DATEADD(Sales[OrderDate], -1, WEEK))",
            "CALCULATE([Total Sales], DATEADD(VALUES('Date'[Date]), -1, WEEK))",
            "CALCULATE([Total Sales], DATEADD('Date'[Date], -1, DAY))",
            "DATEDIFF(DATE(2024,1,1), DATE(2024,2,1), WEEK)",
            "CALCULATE([Total Sales], DATEADD(VALUES('Date'[Date]), -1, YEAR))",
            "CALCULATE([Total Sales], DATEADD(ALL('Date'), -1, YEAR))",
            "CALCULATE([Total Sales], DATESYTD('Date'))",
            "CALCULATE([Total Sales], SAMEPERIODLASTYEAR('Date'))",
            "TOTALYTD([Total Sales], 'Date')",
            "CALCULATE([Total Sales], DATESYTD(FILTER('Date', TRUE())))",
            "CALCULATE([Total Sales], DATESYTD(ALL('Date'[Date], 'Date'[Year])))",
            "CALCULATE([Total Sales], DATESYTD(CALCULATETABLE(VALUES('Date'[Date]))))",
            "CALCULATE([Total Sales], DATESYTD(DISTINCT('Date'[Date])))",
            "CALCULATE([Total Sales], DATESYTD(Sales[OrderDate].[Date]))",
            "CALCULATE([Total Sales], DATESYTD(VAR d = 1 RETURN 'Date'[Date]))",
            "CALCULATE([Total Sales], DATESYTD(Customer[BirthDate]))",
            "CALCULATE([Total Sales], DATESYTD(ROW(\"d\", DATE(2024,1,1))))",
            "CALCULATE([Total Sales], DATESYTD(SELECTCOLUMNS('Date', \"d\", 'Date'[Date])))",
            "CALCULATE([Total Sales], DATESYTD(GENERATESERIES(1, 3)))",
            "CALCULATE([Total Sales], DATESYTD(\"2024-01-01\"))",
            "CALCULATE([Total Sales], DATESYTD(TODAY()))",
            "CALCULATE([Total Sales], DATESYTD(1))",
            "CALCULATE([Total Sales], DATESYTD([Total Sales]))",
            "CALCULATE([Total Sales], DATEADD({DATE(2024,1,1)}, -1, YEAR))",
            "CALCULATE([Total Sales], DATESYTD(Sales[Channel]))",
            "CALCULATE([Total Sales], DATEADD('Date'[Year], -1, YEAR))",
            "CALCULATE([Total Sales], DATESYTD(VALUES('Date'[Year])))",
            "TOTALYTD([Total Sales], 'Date'[Date], \"x\")",
            "TOTALYTD([Total Sales], 'Date'[Date], ALL('Date'), \"6/30\")",
        ],
    );

    // --- natural joins and calculated-table column names (G4) -------------------------------------
    e.many(
        "joins",
        "m",
        &[
            "COUNTROWS(NATURALINNERJOIN(Customer, Product))",
            "COUNTROWS(NATURALINNERJOIN(Product, 'Top Products'))",
            "COUNTROWS(NATURALLEFTOUTERJOIN(Customer, Product))",
            "COUNTROWS(NATURALINNERJOIN('Date', Sales))",
            "COUNTROWS(NATURALINNERJOIN(Customer, Sales))",
            "COUNTROWS(NATURALINNERJOIN(Sales, Product))",
            "COUNTROWS(NATURALINNERJOIN(VALUES(Product[Category]), VALUES(Customer[City])))",
            "COUNTROWS(NATURALINNERJOIN('Date', LocalDateTable_0001))",
            "COUNTROWS(NATURALINNERJOIN(Product, VALUES(Sales[ProductKey])))",
            "COUNTROWS(NATURALINNERJOIN(Product, FILTER(Sales, TRUE())))",
            "COUNTROWS(SUMMARIZE(Sales, Sales[ProductKey], Product[ProductKey]))",
            "COUNTROWS(CROSSJOIN(Sales, Product))",
        ],
    );
    e.many(
        "joins",
        "t",
        &[
            "NATURALLEFTOUTERJOIN(Sales, Product)",
            "NATURALINNERJOIN(Sales, Customer)",
            "NATURALINNERJOIN('Date', Sales)",
            "NATURALINNERJOIN(Product, Product)",
            "NATURALINNERJOIN(Product, VALUES(Product[Category]))",
            "NATURALINNERJOIN(Product, VALUES(Customer[City]))",
            "CROSSJOIN(Sales, Product)",
            "CROSSJOIN(Customer, Product)",
            "CROSSJOIN('Date', LocalDateTable_0001)",
            "GENERATE(Customer, Product)",
            "GENERATE(Sales, Product)",
            "SUMMARIZE(Sales, Sales[ProductKey], Product[ProductKey])",
            "SUMMARIZECOLUMNS(Sales[ProductKey], Product[ProductKey])",
            "CROSSJOIN(VALUES(Sales[ProductKey]), VALUES(Product[ProductKey]))",
            "UNION(VALUES(Sales[ProductKey]), VALUES(Product[ProductKey]))",
            "ADDCOLUMNS(Sales, \"ProductKey2\", 1)",
            "CALENDARAUTO(6)",
            "CALENDAR(DATE(2024,1,1), DATE(2024,12,31))",
        ],
    );
    e.add("joins", "m", "COUNTROWS(CALENDARAUTO())");

    // --- query cycles (G4) ------------------------------------------------------------------------
    e.many("query-cycles", "q", &[
        "DEFINE MEASURE Sales[A] = [B] + 1 MEASURE Sales[B] = [A] EVALUATE ROW(\"x\", [B])",
        "DEFINE MEASURE Sales[A] = [A] EVALUATE ROW(\"x\", 1)",
        "DEFINE MEASURE Sales[A] = [A] EVALUATE ROW(\"x\", [A])",
        "DEFINE MEASURE Sales[A] = [B] MEASURE Sales[B] = [A] EVALUATE ROW(\"x\", 1)",
        "DEFINE MEASURE Sales[A] = [B] MEASURE Sales[B] = [C] MEASURE Sales[C] = [A] EVALUATE ROW(\"x\", [C])",
        "DEFINE MEASURE Sales[A] = [B] MEASURE Sales[B] = [A] MEASURE Sales[C] = [A] EVALUATE ROW(\"x\", [C])",
        "DEFINE MEASURE Sales[A] = [B] MEASURE Sales[B] = [A] EVALUATE ROW(\"x\", 1) EVALUATE ROW(\"y\", [B])",
        "DEFINE MEASURE Sales[A] = [B] MEASURE Sales[B] = [A] VAR v = [A] EVALUATE ROW(\"x\", 1)",
        "DEFINE MEASURE Sales[A] = [B] MEASURE Sales[B] = [A] VAR v = [A] EVALUATE ROW(\"x\", v)",
        "DEFINE MEASURE Sales[A] = [B] MEASURE Sales[B] = [A] EVALUATE ROW(\"x\", IF(FALSE(), [A]))",
        "DEFINE MEASURE Sales[A] = [B] MEASURE Sales[B] = [A] EVALUATE ROW(\"x\", 1) ORDER BY [A]",
        "DEFINE MEASURE Sales[A] = [B] MEASURE Sales[B] = [A] TABLE T = ROW(\"x\", [A]) EVALUATE ROW(\"x\", 1)",
        "DEFINE MEASURE Sales[Total Sales] = [Total Sales] + 1 EVALUATE ROW(\"x\", [Total Sales])",
        "DEFINE MEASURE Sales[Total Sales] = [Total Sales] EVALUATE ROW(\"x\", [Sales LY])",
        "DEFINE COLUMN Sales[C1] = [C2] COLUMN Sales[C2] = [C1] EVALUATE ROW(\"x\", 1)",
        "DEFINE COLUMN Sales[C1] = [C2] COLUMN Sales[C2] = [C1] EVALUATE ROW(\"x\", MAX(Sales[C1]))",
    ]);

    // --- comparisons of incompatible types: definition errors in columns and tables (G4),
    // --- run-time errors elsewhere (G20) -----------------------------------------------------------
    e.many(
        "comparisons",
        "c:Sales",
        &[
            "1 = \"1\"",
            "TRUE() = 1",
            "IF(Sales[Quantity] > \"1\", 1, 0)",
            "Sales[OrderDate] > \"2024-01-01\"",
            "COUNTROWS(FILTER(Sales, Sales[Channel] = 1))",
            "[Total Sales] = \"a\"",
            "SWITCH(TRUE(), \"a\", 1, 2)",
            "Sales[IsReturn] = 1",
            "Sales[Channel] = \"Web\"",
            "Sales[Quantity] = 1.5",
            "Sales[OrderDate] = 45000",
            "Sales[Channel] < BLANK()",
            "VAR x = Sales[Channel] RETURN x = 1",
            "Sales[Channel] IN {1, 2}",
            "CONTAINS(Sales, Sales[Channel], 1)",
            "MAX(Sales[Channel]) = 1",
            "Sales[IsReturn] = TRUE()",
            "\"a\" + 1",
            "NOT \"x\"",
        ],
    );
    e.many(
        "comparisons",
        "t",
        &[
            "FILTER(Sales, Sales[Channel] = 1)",
            "FILTER(Sales, Sales[Channel] = \"Web\")",
        ],
    );
    e.many(
        "comparisons",
        "r:Customer",
        &["Customer[City] = 1", "Customer[CustomerKey] = \"1\""],
    );
    e.many(
        "comparisons",
        "m",
        &[
            "CALCULATE([Total Sales], Sales[Channel] = 1)",
            "COUNTROWS(FILTER(Sales, Sales[Channel] IN {1, 2}))",
            "SWITCH(1, \"a\", 1, 2)",
            "COUNTROWS(FILTER(Sales, Sales[IsReturn] = 1))",
            "COUNTROWS(FILTER('Date', 'Date'[Date] > 45000))",
            "DATE(2024, 1, 1) = 45292",
            "BLANK() = \"a\"",
        ],
    );

    // --- run-time errors a static check can see (G20) ---------------------------------------------
    e.many(
        "runtime",
        "m",
        &[
            "\"\" + 1",
            "\"1\" + 1",
            "\"2024-01-01\" + 1",
            "DATE(2024, 1, 1) + \"x\"",
            "\"x\" && TRUE()",
            "-\"x\"",
            "IF(\"yes\", 1, 0)",
            "IF(\"TRUE\", 1, 0)",
            "IF(\"\", 1, 0)",
            "ROUND(\"x\", 0)",
            "LEFT(\"abc\", \"x\")",
            "DATE(\"x\", 1, 1)",
            "EDATE(\"x\", 1)",
            "YEAR(\"\")",
            "YEAR(\"2024-01-01\")",
            "ABS(\"12\")",
            "VALUE(\"\")",
            "VALUE(\"12\")",
            "DATEVALUE(\"\")",
            "CONVERT(\"x\", INTEGER)",
            "CONVERT(\"12\", DOUBLE)",
            "CONVERT(\"x\", STRING)",
            "FORMAT(\"x\", \"0\")",
            "DIVIDE(1, 0, \"n/a\")",
            "MOD(5, 0)",
            "QUOTIENT(1, 0)",
            "SQRT(-1)",
            "LN(0)",
            "LOG(-1)",
            "LEFT(\"abc\", -1)",
            "REPT(\"a\", -1)",
            "MID(\"abc\", 0, 1)",
            "SUM(Sales[Channel])",
            "AVERAGE(Sales[IsReturn])",
            "SUMX(Sales, \"x\")",
            "MAX(Sales[Channel])",
            "SUM(Sales[OrderDate])",
            "COUNTX(Sales, \"a\")",
            "COUNTROWS(GENERATESERIES(1, \"z\"))",
            "COUNTROWS(GENERATESERIES(DATE(2024,1,1), DATE(2024,1,5)))",
            "{1, 2} + 1",
            "VAR t = {1, 2} RETURN t",
            "DISTINCT(Product[Category])",
            "ALL(Product[Category])",
            "CALCULATE(VALUES(Product[Category]), Product[Category] = \"A\")",
            "IF(HASONEVALUE(Product[Category]), VALUES(Product[Category]))",
            "TOPN(1, VALUES(Product[Category]))",
        ],
    );
    e.many(
        "runtime",
        "r:Customer",
        &[
            "\"TRUE\"",
            "IF(Customer[IsActive], 1, 0)",
            "Customer[CustomerKey]",
        ],
    );
    e.many(
        "runtime",
        "c:Sales",
        &["Sales[Channel] + 1", "Sales[Channel] && TRUE()"],
    );

    // --- multi-section functions: SUMMARIZE, GROUPBY, SUMMARIZECOLUMNS (G19) ---------------------
    e.many("sections", "m", &[
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], [Total Sales]))",
        "COUNTROWS(SUMMARIZE(Sales, [Total Sales]))",
        "COUNTROWS(GROUPBY(Sales, [Total Sales]))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], 1))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], \"x\", Sales))",
        "COUNTROWS(SUMMARIZE(Sales, Sales[Channel], \"x\", Sales))",
        "COUNTROWS(SUMMARIZE(Sales, Sales[Channel] & \"\"))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], FILTER(Sales, TRUE()), \"x\", [Total Sales]))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], TREATAS({\"Web\"}, Sales[Channel]), \"x\", [Total Sales]))",
        "COUNTROWS(SUMMARIZE(Sales, ROLLUP(Sales[Channel]), \"x\", SUM(Sales[Quantity])))",
        "COUNTROWS(SUMMARIZECOLUMNS(ROLLUPADDISSUBTOTAL(Sales[Channel], \"IsTotal\"), \"x\", [Total Sales]))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], \"x\", [Total Sales], \"y\", 1))",
        "COUNTROWS(SUMMARIZECOLUMNS(\"x\", [Total Sales]))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], VALUES(Product[Category])))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], Product))",
        "COUNTROWS(SUMMARIZE(Sales, Sales[Channel], \"x\"))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], \"x\"))",
        "COUNTROWS(GROUPBY(Sales, \"x\", SUMX(CURRENTGROUP(), Sales[Quantity])))",
        "COUNTROWS(SUMMARIZE(Sales))",
        "COUNTROWS(SUMMARIZECOLUMNS())",
        "COUNTROWS(GROUPBY(Sales))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], Sales[Quantity] > 1))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], \"x\", IGNORE([Total Sales])))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], NONVISUAL(FILTER(Sales, TRUE())), \"x\", [Total Sales]))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], KEEPFILTERS(FILTER(Sales, TRUE())), \"x\", [Total Sales]))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], \"x\", [Total Sales], Sales[Quantity]))",
        "COUNTROWS(SUMMARIZE(Sales, \"x\", SUM(Sales[Quantity]), Sales[Channel]))",
        "COUNTROWS(SUMMARIZECOLUMNS(FILTER(Sales, TRUE()), Sales[Channel], \"x\", [Total Sales]))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], \"x\", Sales[Quantity]))",
        "COUNTROWS(SUMMARIZE(Sales, Sales[Channel], \"x\", Sales[Quantity]))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], \"x\", VALUES(Sales[Quantity])))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], \"x\", 1 + \"a\"))",
        "COUNTROWS(SUMMARIZE(Sales, Sales[Channel], \"x\", [Total Sales]))",
        "COUNTROWS(SUMMARIZE(Sales, Sales[Channel], Sales[Channel]))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], Sales[Channel]))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", SUMX(CURRENTGROUP(), Sales[Quantity]), Sales[ProductKey]))",
        "COUNTROWS(ADDMISSINGITEMS(Sales[Channel], SUMMARIZECOLUMNS(Sales[Channel], \"x\", [Total Sales]), Sales[Channel]))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], \"x\", [Total Sales], FILTER(Sales, TRUE())))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", VARX.S(CURRENTGROUP(), Sales[Quantity])))",
        "COUNTROWS(GROUPBY(Sales, Sales[Channel], \"Q\", MINX(CURRENTGROUP(), Sales[Quantity]), \"R\", MAXX(CURRENTGROUP(), Sales[Quantity])))",
    ]);
    e.many(
        "runtime",
        "m",
        &[
            "MAX(Sales[Channel]) + 1",
            "SUMX(Sales, Sales[Channel] + 1)",
            "COUNTROWS(FILTER(Sales, Sales[Channel] * 1 > 0))",
            "SUMX(Sales, Sales[Quantity] + 1)",
        ],
    );
    e.many("sections", "m", &[
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], \"x\", Sales[Channel]))",
        "COUNTROWS(SUMMARIZE(Sales, Sales[Channel], \"x\", Sales[Channel]))",
        "COUNTROWS(SUMMARIZE(Sales, Sales[Channel], \"x\", Product[Category]))",
        "COUNTROWS(SUMMARIZE(Sales, Product[Category], \"x\", Product[Category]))",
        "COUNTROWS(SUMMARIZE(Sales, Sales[Channel], \"x\", SUMX(Sales, Sales[Quantity])))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], Product[Category], \"x\", Sales[Channel] & Product[Category]))",
        "COUNTROWS(SUMMARIZECOLUMNS(ROLLUPADDISSUBTOTAL(ALLSELECTED(Sales), Sales[Channel], \"IsTotal\"), \"x\", [Total Sales]))",
        "COUNTROWS(SUMMARIZE(Sales, ROLLUPADDISSUBTOTAL(Sales[Channel], \"IsTotal\"), \"x\", SUM(Sales[Quantity])))",
    ]);
    e.many(
        "sections",
        "c:Sales",
        &[
            "COUNTROWS(SUMMARIZE(Sales, Sales[Channel], \"x\", Sales[Quantity]))",
            "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], \"x\", Sales[Quantity]))",
        ],
    );
    e.many(
        "comparisons",
        "q",
        &[
            "EVALUATE FILTER(Sales, Sales[Channel] = 1)",
            "EVALUATE FILTER(Sales, Sales[Channel] = \"Web\")",
            "EVALUATE ROW(\"x\", 1 = \"1\")",
            "EVALUATE ROW(\"x\", IF(FALSE(), 1 = \"1\"))",
            "DEFINE COLUMN Sales[X] = Sales[Channel] = 1 EVALUATE ROW(\"x\", 1)",
            "DEFINE MEASURE Sales[M] = IF(1 = \"1\", 1) EVALUATE ROW(\"x\", 1)",
        ],
    );

    // --- a variable shadows a table of the same name, even quoted ---------------------------------
    // Added 2026-10-14: `'Product'` denotes `VAR Product` when one is in scope (found with Desktop).
    e.many(
        "var-shadowing",
        "m",
        &[
            "VAR Product = 3 RETURN COUNTROWS('Product')",
            "VAR product = 3 RETURN COUNTROWS('Product')",
            "VAR Product = 3 RETURN Product + COUNTROWS(Sales)",
            "VAR Product = 3 RETURN 'Product' + 1",
            "VAR Product = FILTER(Product, Product[ListPrice] > 10) RETURN COUNTROWS('Product')",
            "VAR Product = Sales RETURN COUNTROWS('Product')",
            "VAR Product = 3 RETURN SUM('Product'[ListPrice]) + Product",
            "VAR x = COUNTROWS('Product') VAR Product = 2 RETURN x + Product",
            "VAR Sales = 1 RETURN SUMX('Sales', Sales[Quantity])",
            "VAR Sales = 1 RETURN CALCULATE([Total Sales], ALL('Sales'))",
            "VAR Customer = 2 RETURN CALCULATE([Total Sales], 'Customer')",
            "VAR t = 1 RETURN COUNTROWS('t')",
            "VAR t = Sales RETURN COUNTROWS('t')",
            "SUMX(VALUES(Product[Category]), VAR Product = 1 RETURN Product)",
            "VAR Product = 1 RETURN SUMX(VALUES('Product'[Category]), 1)",
            "VAR Product = 1 RETURN COUNTROWS(FILTER('Product', TRUE()))",
        ],
    );
    e.add(
        "var-shadowing",
        "c:Sales",
        "VAR Product = 1 RETURN COUNTROWS('Product')",
    );
    e.add(
        "var-shadowing",
        "c:Sales",
        "VAR Product = 1 RETURN Product + COUNTROWS(Product)",
    );
    e.add(
        "var-shadowing",
        "r:Product",
        "VAR Product = 1 RETURN COUNTROWS('Product') > 0",
    );
    e.add(
        "var-shadowing",
        "r:Product",
        "VAR Customer = 1 RETURN Customer = 1",
    );
    e.add("var-shadowing", "t", "VAR Product = 1 RETURN 'Product'");
    e.add(
        "var-shadowing",
        "t",
        "VAR Customer = FILTER(Customer, Customer[IsActive]) RETURN 'Customer'",
    );
    e.many(
        "var-shadowing",
        "q",
        &[
            "DEFINE VAR Product = 3 EVALUATE ROW(\"x\", COUNTROWS('Product'))",
            "EVALUATE ROW(\"x\", VAR Product = 3 RETURN COUNTROWS('Product'))",
            "DEFINE VAR Product = FILTER(Product, Product[ListPrice] > 10) EVALUATE 'Product'",
            "DEFINE VAR Product = 3 EVALUATE 'Product'",
        ],
    );

    // --- SUMMARIZECOLUMNS: NONVISUAL filters and table expressions as values (sections-024/-031) ----
    // Added 2026-10-14.
    e.many_sc("sections-2", "m", &[
        "Sales[Channel], NONVISUAL(TREATAS({\"Web\"}, Sales[Channel])), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(VALUES(Sales[Channel])), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(FILTER(ALL(Sales[Channel]), Sales[Channel] = \"Web\")), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(ALL(Sales[Channel])), \"x\", [Total Sales]",
        "Product[Category], NONVISUAL(TREATAS({\"Web\"}, Sales[Channel])), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(TREATAS({\"Kitchen\"}, Product[Category])), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(FILTER(ALL(Sales[Channel], Sales[Quantity]), Sales[Quantity] > 1)), \"x\", [Total Sales]",
        "Sales[Channel], Sales[Quantity], NONVISUAL(FILTER(ALL(Sales[Channel], Sales[Quantity]), Sales[Quantity] > 1)), \"x\", [Total Sales]",
        "Product[Category], NONVISUAL(FILTER(Product, TRUE())), \"x\", [Total Sales]",
        "Product[ProductKey], Product[Product], Product[Category], Product[ListPrice], Product[Weight], NONVISUAL(FILTER(Product, TRUE())), \"x\", [Total Sales]",
        "NONVISUAL(TREATAS({\"Web\"}, Sales[Channel])), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(KEEPFILTERS(TREATAS({\"Web\"}, Sales[Channel]))), \"x\", [Total Sales]",
        "Sales[Channel], KEEPFILTERS(NONVISUAL(TREATAS({\"Web\"}, Sales[Channel]))), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(TREATAS({\"Web\"}, Sales[Channel])), NONVISUAL(TREATAS({\"Kitchen\"}, Product[Category])), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(Product), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(FILTER(VALUES(Sales[Channel]), Sales[Channel] <> \"Web\")), \"x\", [Total Sales]",
        "Sales[Channel], Product[Category], NONVISUAL(TREATAS({(\"Web\", \"Kitchen\")}, Sales[Channel], Product[Category])), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(TREATAS({(\"Web\", \"Kitchen\")}, Sales[Channel], Product[Category])), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(CALCULATETABLE(VALUES(Sales[Channel]))), \"x\", [Total Sales]",
        "ROLLUPADDISSUBTOTAL(Sales[Channel], \"T\"), NONVISUAL(TREATAS({\"Web\"}, Sales[Channel])), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(FILTER(ALL(Product[Category]), Product[Category] = \"Kitchen\")), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(FILTER(ALL(Sales[Quantity]), Sales[Quantity] > 1)), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(FILTER(Sales, TRUE()))",
    ]);
    e.many("sections-2", "m", &[
        "VAR f = TREATAS({\"Web\"}, Sales[Channel]) RETURN COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], NONVISUAL(f), \"x\", [Total Sales]))",
        "COUNTROWS(NONVISUAL(VALUES(Sales[Channel])))",
        "CALCULATE([Total Sales], NONVISUAL(VALUES(Sales[Channel])))",
    ]);
    e.many("sections-2", "q", &[
        "EVALUATE SUMMARIZECOLUMNS(Sales[Channel], NONVISUAL(FILTER(Sales, TRUE())), \"x\", [Total Sales])",
        "EVALUATE SUMMARIZECOLUMNS(Sales[Channel], NONVISUAL(TREATAS({\"Web\"}, Sales[Channel])), \"x\", [Total Sales])",
        "EVALUATE SUMMARIZECOLUMNS(Sales[Channel], NONVISUAL(TREATAS({\"Kitchen\"}, Product[Category])), \"x\", [Total Sales])",
    ]);
    e.many("sections-2", "t", &[
        "SUMMARIZECOLUMNS(Sales[Channel], NONVISUAL(FILTER(Sales, TRUE())), \"x\", [Total Sales])",
        "SUMMARIZECOLUMNS(Sales[Channel], NONVISUAL(TREATAS({\"Web\"}, Sales[Channel])), \"x\", [Total Sales])",
    ]);
    // A table expression as a SUMMARIZECOLUMNS / SUMMARIZE value: fails when evaluated if a group sees
    // several values.
    e.many_sc(
        "sections-2",
        "m",
        &[
            "Sales[Channel], \"x\", VALUES(Sales[Channel])",
            "Sales[Channel], \"x\", VALUES(Product[Category])",
            "Product[Category], \"x\", VALUES(Product[Category])",
            "Product[ProductKey], \"x\", VALUES(Product[Category])",
            "\"x\", VALUES(Sales[Quantity])",
            "Sales[Channel], \"x\", DISTINCT(Sales[Quantity])",
            "Sales[Channel], \"x\", ALL(Sales[Quantity])",
            "Sales[Channel], \"x\", ALL(Sales[Channel])",
            "Sales[Channel], \"x\", VALUES(Sales[Quantity]) + 0",
            "Sales[Channel], \"x\", SELECTEDVALUE(Sales[Quantity])",
            "Sales[Channel], \"x\", IF(HASONEVALUE(Sales[Quantity]), VALUES(Sales[Quantity]))",
            "Customer[City], \"x\", VALUES(Customer[Segment])",
            "Customer[CustomerKey], \"x\", VALUES(Customer[City])",
            "Sales[Channel], \"x\", VALUES(Sales[Channel]) & \"\"",
            "Sales[Channel], Sales[Quantity], \"x\", VALUES(Sales[Quantity])",
            "'Date'[Year], \"x\", VALUES('Date'[Month])",
            "'Date'[Date], \"x\", VALUES('Date'[Year])",
            "Sales[OrderDate], \"x\", VALUES('Date'[Year])",
            "Sales[ProductKey], \"x\", VALUES(Product[Category])",
            "Product[ProductKey], \"x\", VALUES(Sales[Quantity])",
            "Customer[CustomerKey], \"x\", VALUES(Sales[Channel])",
            "Sales[Channel], \"x\", [Total Sales], \"y\", VALUES(Sales[Quantity])",
            "Sales[Channel], TREATAS({\"Phone\"}, Sales[Channel]), \"x\", VALUES(Sales[Quantity])",
        ],
    );
    e.many(
        "sections-2",
        "m",
        &[
            "COUNTROWS(SUMMARIZE(Sales, Sales[Channel], \"x\", VALUES(Sales[Quantity])))",
            "COUNTROWS(SUMMARIZE(Sales, Sales[Channel], \"x\", VALUES(Sales[Channel])))",
            "COUNTROWS(SUMMARIZE(Sales, Product[Category], \"x\", VALUES(Product[Category])))",
            "COUNTROWS(ADDCOLUMNS(VALUES(Sales[Channel]), \"x\", VALUES(Sales[Quantity])))",
            "COUNTROWS(ADDCOLUMNS(VALUES(Sales[Channel]), \"x\", CALCULATE(VALUES(Sales[Channel]))))",
            "VAR v = VALUES(Sales[Quantity]) RETURN COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], \"x\", v))",
        ],
    );
    e.many(
        "sections-2",
        "q",
        &[
            "EVALUATE SUMMARIZECOLUMNS(Sales[Channel], \"x\", VALUES(Sales[Quantity]))",
            "EVALUATE SUMMARIZECOLUMNS(Sales[Channel], \"x\", VALUES(Sales[Channel]))",
        ],
    );
    e.many(
        "sections-2",
        "t",
        &[
            "SUMMARIZECOLUMNS(Sales[Channel], \"x\", VALUES(Sales[Quantity]))",
            "SUMMARIZECOLUMNS(Product[ProductKey], \"x\", VALUES(Product[Category]))",
        ],
    );

    // --- filter modifiers outside CALCULATE (G19) --------------------------------------------------
    // Added 2026-10-14.
    e.many("modifiers", "m", &[
        "COUNTROWS(REMOVEFILTERS(Sales))",
        "COUNTROWS(REMOVEFILTERS(Sales[Channel]))",
        "COUNTROWS(REMOVEFILTERS())",
        "COUNTROWS(KEEPFILTERS(VALUES(Sales[Channel])))",
        "COUNTROWS(KEEPFILTERS(Sales))",
        "USERELATIONSHIP(Sales[OrderDate], 'Date'[Date])",
        "COUNTROWS(USERELATIONSHIP(Sales[OrderDate], 'Date'[Date]))",
        "CROSSFILTER(Sales[ProductKey], Product[ProductKey], BOTH)",
        "IF(TRUE(), CROSSFILTER(Sales[ProductKey], Product[ProductKey], BOTH))",
        "SUMX(REMOVEFILTERS(Sales), Sales[Quantity])",
        "COUNTROWS(FILTER(REMOVEFILTERS(Sales), Sales[Quantity] > 1))",
        "COUNTROWS(CALCULATETABLE(VALUES(Sales[Channel]), REMOVEFILTERS(Sales)))",
        "COUNTROWS(CALCULATETABLE(Sales, USERELATIONSHIP(Sales[OrderDate], 'Date'[Date])))",
        "COUNTROWS(CALCULATETABLE(Sales, CROSSFILTER(Sales[ProductKey], Product[ProductKey], NONE)))",
        "COUNTROWS(ALLSELECTED())",
        "COUNTROWS(ALL())",
        "COUNTROWS(ALLSELECTED(Sales))",
        "COUNTROWS(ALLCROSSFILTERED(Sales))",
        "CALCULATE([Total Sales], ALLCROSSFILTERED(Sales))",
        "COUNTROWS(FILTER(Sales, KEEPFILTERS(Sales[Channel] = \"Web\")))",
        "CALCULATE([Total Sales], KEEPFILTERS(USERELATIONSHIP(Sales[OrderDate], 'Date'[Date])))",
        "VAR r = REMOVEFILTERS(Sales) RETURN CALCULATE([Total Sales], r)",
        "VAR r = USERELATIONSHIP(Sales[OrderDate], 'Date'[Date]) RETURN CALCULATE([Total Sales], r)",
        "TOTALYTD([Total Sales], 'Date'[Date], REMOVEFILTERS(Product))",
        "COUNTROWS(FILTER(Sales, USERELATIONSHIP(Sales[OrderDate], 'Date'[Date])))",
        "[Total Sales] + USERELATIONSHIP(Sales[OrderDate], 'Date'[Date])",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], REMOVEFILTERS(Product), \"x\", [Total Sales]))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], USERELATIONSHIP(Sales[OrderDate], 'Date'[Date]), \"x\", [Total Sales]))",
        "SUMX(Sales, CALCULATE([Total Sales], REMOVEFILTERS(Sales[Channel])))",
        "CALCULATE([Total Sales], IF(TRUE(), REMOVEFILTERS(Sales)))",
        "CALCULATE([Total Sales], CALCULATETABLE(REMOVEFILTERS(Sales)))",
        "COUNTROWS(CALCULATETABLE(REMOVEFILTERS(Sales[Channel])))",
        "COUNTROWS(ALLNOBLANKROW(Sales))",
        "COUNTROWS(ALLEXCEPT(Sales, Sales[Channel]))",
        "CALCULATE([Total Sales], FILTER(ALL(Sales), TRUE()))",
        "MAXX(KEEPFILTERS(ALL(Sales[Channel])), Sales[Channel])",
    ]);
    e.many(
        "modifiers",
        "t",
        &["REMOVEFILTERS(Sales)", "KEEPFILTERS(Sales)", "ALLSELECTED(Sales)"],
    );
    e.many("modifiers", "q", &[
        "EVALUATE REMOVEFILTERS(Sales)",
        "EVALUATE ROW(\"x\", COUNTROWS(REMOVEFILTERS(Sales)))",
        "EVALUATE ALL()",
        "EVALUATE KEEPFILTERS(VALUES(Sales[Channel]))",
        "EVALUATE CALCULATETABLE(VALUES(Sales[Channel]), USERELATIONSHIP(Sales[OrderDate], 'Date'[Date]))",
    ]);
    e.many(
        "modifiers",
        "c:Sales",
        &[
            "COUNTROWS(REMOVEFILTERS(Sales))",
            "CALCULATE(SUM(Sales[Quantity]), REMOVEFILTERS(Sales))",
        ],
    );

    // Second round, around the rules the first one showed.
    e.many(
        "sections-2",
        "m",
        &[
            "COUNTROWS(FILTER(NONVISUAL(VALUES(Sales[Channel])), TRUE()))",
            "COUNTROWS(CALCULATETABLE(Sales, NONVISUAL(VALUES(Sales[Channel]))))",
            "SUMX(NONVISUAL(VALUES(Sales[Channel])), 1)",
        ],
    );
    e.many_sc("sections-2", "m", &[
        "Sales[Channel], KEEPFILTERS(NONVISUAL(TREATAS({\"Kitchen\"}, Product[Category]))), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(TREATAS({1}, Sales[Quantity])), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(ALLSELECTED(Sales[Channel])), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(ALL(Sales)), \"x\", [Total Sales]",
        "ROLLUPADDISSUBTOTAL(Sales[Channel], \"T\"), Sales[Quantity], NONVISUAL(TREATAS({1}, Sales[Quantity])), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(CROSSJOIN(VALUES(Sales[Channel]), VALUES(Product[Category]))), \"x\", [Total Sales]",
        "Sales[Channel], Product[Category], NONVISUAL(CROSSJOIN(VALUES(Sales[Channel]), VALUES(Product[Category]))), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(DATATABLE(\"c\", STRING, {{\"Web\"}})), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(SELECTCOLUMNS(Sales, \"c\", Sales[Channel])), \"x\", [Total Sales]",
        "Sales[Channel], Sales[Quantity], NONVISUAL(TREATAS({\"Web\"}, Sales[Channel])), \"x\", [Total Sales]",
        "Sales[Channel], NONVISUAL(FILTER(ALL(Sales[Channel]), [Total Sales] > 0)), \"x\", [Total Sales]",
        "Sales[OrderDate].[Year], NONVISUAL(VALUES(LocalDateTable_0001[Year])), \"x\", [Total Sales]",
        "Sales[Channel], TREATAS({1}, Sales[Quantity]), \"x\", VALUES(Sales[Quantity])",
        "Sales[Channel], FILTER(Sales, Sales[SalesKey] = 1), \"x\", VALUES(Sales[Quantity])",
        "Product[Category], \"x\", VALUES(Product[Product])",
        "Product[Category], \"x\", CONCATENATEX(VALUES(Product[Product]), Product[Product])",
        "Sales[Channel], \"x\", 1 + VALUES(Sales[Quantity]) * 2",
        "Sales[Channel], \"x\", VAR v = VALUES(Sales[Quantity]) RETURN v",
        "Sales[Channel], \"x\", DISTINCT(Sales[Channel])",
        "Sales[Channel], \"x\", COUNTROWS(VALUES(Sales[Quantity]))",
    ]);
    e.many("sections-2", "t", &["NONVISUAL(VALUES(Sales[Channel]))"]);
    e.many(
        "sections-2",
        "q",
        &[
            "EVALUATE NONVISUAL(VALUES(Sales[Channel]))",
            "EVALUATE SUMMARIZECOLUMNS(Product[ProductKey], \"x\", VALUES(Product[Category]))",
        ],
    );
    e.many("modifiers", "m", &[
        "TOTALYTD([Total Sales], 'Date'[Date], USERELATIONSHIP(Sales[OrderDate], 'Date'[Date]))",
        "TOTALYTD([Total Sales], 'Date'[Date], KEEPFILTERS(Product[Category] = \"Kitchen\"))",
        "CALCULATE([Total Sales], KEEPFILTERS(REMOVEFILTERS(Sales)))",
        "ISEMPTY(REMOVEFILTERS(Sales))",
        "CONCATENATEX(REMOVEFILTERS(Sales[Channel]), Sales[Channel])",
        "COUNTROWS(TOPN(1, REMOVEFILTERS(Sales), Sales[Quantity]))",
        "COUNTROWS(DISTINCT(REMOVEFILTERS(Sales[Channel])))",
        "REMOVEFILTERS(Sales[Channel])",
        "IF(TRUE(), COUNTROWS(REMOVEFILTERS(Sales)))",
        "COUNTROWS(REMOVEFILTERS(Sales)) + 1",
        "SUMX(ALL(), 1)",
        "COUNTROWS(FILTER(ALLSELECTED(), TRUE()))",
        "CALCULATE([Total Sales], ALLSELECTED())",
        "COUNTROWS(FILTER(KEEPFILTERS(VALUES(Sales[Channel])), TRUE()))",
        "COUNTROWS(ADDCOLUMNS(KEEPFILTERS(VALUES(Sales[Channel])), \"x\", 1))",
        "COUNTROWS(CALCULATETABLE(KEEPFILTERS(VALUES(Sales[Channel]))))",
        "CALCULATE([Total Sales], FILTER(KEEPFILTERS(VALUES(Sales[Channel])), TRUE()))",
        "COUNTROWS(CALCULATETABLE(Sales, KEEPFILTERS(Sales[Channel] = \"Web\")))",
        "VAR k = KEEPFILTERS(VALUES(Sales[Channel])) RETURN CALCULATE([Total Sales], k)",
        "COUNTROWS(GENERATE(VALUES(Sales[Channel]), KEEPFILTERS(VALUES(Product[Category]))))",
        "SUMX(KEEPFILTERS(FILTER(Sales, Sales[Quantity] > 1)), Sales[Quantity])",
        "CALCULATE([Total Sales], CROSSFILTER(Sales[ProductKey], Product[ProductKey], BOTH), USERELATIONSHIP(Sales[OrderDate], 'Date'[Date]))",
        "COUNTROWS(ALLCROSSFILTERED(Sales)) + 0",
        "MAXX(ALLSELECTED(), 1)",
    ]);
    e.many(
        "modifiers",
        "c:Sales",
        &[
            "CALCULATE(SUM(Sales[Quantity]), USERELATIONSHIP(Sales[OrderDate], 'Date'[Date]))",
            "COUNTROWS(ALL())",
            "SUMX(KEEPFILTERS(VALUES(Sales[Channel])), 1)",
        ],
    );
    e.many(
        "modifiers",
        "q",
        &[
            "EVALUATE ROW(\"x\", CALCULATE([Total Sales], USERELATIONSHIP(Sales[OrderDate], 'Date'[Date])))",
            "EVALUATE ROW(\"x\", COUNTROWS(ALLCROSSFILTERED(Sales)))",
            "EVALUATE CALCULATETABLE(VALUES(Sales[Channel]), KEEPFILTERS(Sales[Channel] = \"Web\"))",
            "EVALUATE FILTER(KEEPFILTERS(VALUES(Sales[Channel])), TRUE())",
        ],
    );
    e.many(
        "modifiers",
        "t",
        &[
            "ALL()",
            "ALLCROSSFILTERED(Sales)",
            "CALCULATETABLE(Sales, REMOVEFILTERS(Product))",
        ],
    );

    // Third round: the edges of the placement rules.
    e.many("modifiers", "m", &[
        "SUMX(ALLCROSSFILTERED(Sales), 1)",
        "COUNTROWS(FILTER(ALLCROSSFILTERED(Sales), TRUE()))",
        "COUNTROWS(FILTER(ALL(), TRUE()))",
        "ALL()",
        "ALLSELECTED()",
        "VAR a = ALL() RETURN CALCULATE([Total Sales], a)",
        "CALCULATE([Total Sales], KEEPFILTERS(ALL()))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], ALL(), \"x\", [Total Sales]))",
        "COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], KEEPFILTERS(REMOVEFILTERS(Product)), \"x\", [Total Sales]))",
        "COUNTROWS(TOPN(1, KEEPFILTERS(VALUES(Sales[Channel])), 1))",
        "RANKX(KEEPFILTERS(ALL(Product[Category])), [Total Sales])",
        "COUNTROWS(GENERATE(KEEPFILTERS(VALUES(Sales[Channel])), VALUES(Product[Category])))",
        "CONCATENATEX(KEEPFILTERS(VALUES(Sales[Channel])), Sales[Channel])",
        "COUNTROWS(SELECTCOLUMNS(KEEPFILTERS(VALUES(Sales[Channel])), \"c\", Sales[Channel]))",
        "ISEMPTY(KEEPFILTERS(VALUES(Sales[Channel])))",
    ]);
    e.many(
        "modifiers",
        "r:Sales",
        &[
            "COUNTROWS(REMOVEFILTERS(Sales)) > 0",
            "SUMX(REMOVEFILTERS(Sales), 1) > 0",
        ],
    );
    e.many("modifiers", "c:Sales", &["SUMX(REMOVEFILTERS(Sales), 1)"]);
    e.many(
        "modifiers",
        "t",
        &[
            "ROW(\"x\", COUNTROWS(REMOVEFILTERS(Sales)))",
            "ROW(\"x\", COUNTROWS(ALL()))",
        ],
    );
    e.many(
        "sections-2",
        "m",
        &[
            "ISEMPTY(NONVISUAL(VALUES(Sales[Channel])))",
            "NONVISUAL(VALUES(Sales[Channel]))",
        ],
    );
    e.many_sc(
        "sections-2",
        "m",
        &[
            "Sales[Channel], NONVISUAL(TOPN(1, VALUES(Sales[Channel]), 1)), \"x\", [Total Sales]",
            "Sales[Channel], NONVISUAL(CALCULATETABLE(VALUES(Product[Category]))), \"x\", [Total Sales]",
            "Sales[Channel], NONVISUAL(VALUES(Sales)), \"x\", [Total Sales]",
            "Sales[Channel], NONVISUAL(ALL(Product[Category])), \"x\", [Total Sales]",
            "Sales[Channel], NONVISUAL(KEEPFILTERS(FILTER(Sales, TRUE()))), \"x\", [Total Sales]",
        ],
    );
    e.many(
        "sections-2",
        "c:Sales",
        &["CALCULATE(SUM(Sales[Quantity]), NONVISUAL(VALUES(Sales[Channel])))"],
    );
    e.many(
        "sections-2",
        "t",
        &["CALCULATETABLE(Sales, NONVISUAL(VALUES(Sales[Channel])))"],
    );

    // --- engine keywords shared with MDX/DMX as table and variable names (G4) ---------------------
    // `Typed`, `Status`, `Name`… cannot qualify a column, name a table or a variable, or (mostly) be
    // a bare table reference without quotes; the user-defined-function type words are not reserved.
    e.many(
        "keyword-names",
        "q",
        &[
            "DEFINE TABLE 'Typed' = {1} EVALUATE ROW(\"x\", SUM(Typed[Value]))",
            "DEFINE TABLE 'Typed' = {1} EVALUATE ROW(\"x\", SUM('Typed'[Value]))",
            "DEFINE TABLE 'Typed' = {1} EVALUATE ROW(\"x\", COUNTROWS(Typed))",
            "DEFINE TABLE 'Typed' = {1} EVALUATE ROW(\"x\", COUNTROWS('Typed'))",
            "DEFINE TABLE Typed = {1} EVALUATE 'Typed'",
            "DEFINE TABLE 'Status' = {1} EVALUATE Status",
            "DEFINE TABLE 'Status' = {1} EVALUATE 'Status'",
            "DEFINE TABLE 'Name' = {1} EVALUATE ROW(\"x\", MAX(Name[Value]))",
            "DEFINE TABLE 'Type' = {1} EVALUATE FILTER(Type, TRUE())",
            "DEFINE TABLE 'Values' = {1} EVALUATE ROW(\"x\", COUNTROWS(Values))",
            "DEFINE TABLE 'Values' = {1} EVALUATE ROW(\"x\", MAX(Values[Value]))",
            "DEFINE TABLE 'Densify' = {1} EVALUATE ROW(\"x\", MAX(Densify[Value]))",
            "DEFINE TABLE 'Scalar' = {1} EVALUATE ROW(\"x\", MAX(Scalar[Value]))",
            "EVALUATE ROW(\"x\", VAR Densify = 1 RETURN Densify)",
            "DEFINE FUNCTION F = (Typed) => Typed + 1 EVALUATE ROW(\"x\", F(1))",
            "DEFINE FUNCTION Status = (a) => a + 1 EVALUATE ROW(\"x\", Status(1))",
            "DEFINE FUNCTION F = (Val) => Val + 1 EVALUATE ROW(\"x\", F(1))",
        ],
    );
    e.many(
        "keyword-names",
        "m",
        &[
            "VAR Typed = 1 RETURN Typed",
            "VAR Status = 1 RETURN Status",
            "VAR Key = 1 RETURN Key",
            "VAR Function = 1 RETURN Function",
            "VAR Scalar = 1 RETURN Scalar",
            "VAR AnyVal = 1 RETURN AnyVal",
            "VAR Int64 = 1 RETURN Int64",
        ],
    );
}
