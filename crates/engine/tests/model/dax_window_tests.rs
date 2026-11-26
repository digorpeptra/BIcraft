//! The window functions (`OFFSET`, `INDEX`, `WINDOW`, `RANK`, `ROWNUMBER`), `GROUPBY` with
//! `CURRENTGROUP()`, the natural joins and the balance functions, run by `dax.evaluate` on the
//! small model of `tests/window_functions` (data from its own partitions).
//!
//! The expected values follow the semantics of Microsoft Learn's public DAX reference; they are
//! documented, not oracle-checked (the engine oracle's cases for these functions are in
//! `coverage_oracle_matches_the_engine`).
//!
//! Sales (Id, Region, Day, Amount): 1 East 01-05 10, 2 East 01-20 20, 3 East 02-10 30,
//! 4 West 01-15 5, 5 West 02-29 15, 6 West 03-31 25, 7 East 03-31 20, 8 West 04-10 8,
//! 9 West 04-20 blank; Date is 2024-01-01 to 2024-04-30; Region: East (Ann), West (Bob),
//! North (Cid, no sales).

use serde_json::{Value, json};

use bicraft_engine::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.model = Some(bicraft_tmsl::read(include_str!("../window_functions/model.bim")).unwrap());
    s
}

fn eval(s: &mut Session, query: &str) -> Value {
    s.execute("dax.evaluate", &json!({ "query": query }))
        .unwrap_or_else(|e| panic!("{query}: {e}"))
}

/// The values of column `name` of the first result, in order.
fn column(s: &mut Session, query: &str, name: &str) -> Vec<Value> {
    let v = eval(s, query);
    v["results"][0]["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("{query}: no rows in {v}"))
        .iter()
        .map(|r| {
            r.get(name)
                .cloned()
                .unwrap_or_else(|| panic!("{query}: no column {name} in {r}"))
        })
        .collect()
}

/// Numbers (blank: `None`) of column `name`.
fn numbers(s: &mut Session, query: &str, name: &str) -> Vec<Option<f64>> {
    column(s, query, name).iter().map(Value::as_f64).collect()
}

fn one(s: &mut Session, query: &str) -> Option<f64> {
    numbers(s, query, "[v]").first().copied().flatten()
}

fn error(s: &mut Session, query: &str) -> String {
    match s.execute("dax.evaluate", &json!({ "query": query })) {
        Ok(v) => panic!("{query}: expected an error, got {v}"),
        Err(e) => e,
    }
}

#[test]
fn window_running_totals() {
    let mut s = session();
    // a running total in a row context (relation given) and in a measure (relation omitted:
    // ALLSELECTED of the ORDERBY column)
    let q = "EVALUATE ADDCOLUMNS(SUMMARIZECOLUMNS('Date'[Month], \"S\", [Total]), \"Run\", \
             SUMX(WINDOW(1, ABS, 0, REL, ALLSELECTED('Date'[Month]), ORDERBY('Date'[Month])), \
             CALCULATE([Total]))) ORDER BY 'Date'[Month]";
    let want = vec![Some(35.0), Some(80.0), Some(125.0), Some(133.0)];
    assert_eq!(numbers(&mut s, q, "[Run]"), want);
    let q = "DEFINE MEASURE Sales[Run] = SUMX(WINDOW(1, ABS, 0, REL, ORDERBY('Date'[Month])), \
             CALCULATE([Total])) \
             EVALUATE SUMMARIZECOLUMNS('Date'[Month], \"Run\", [Run]) ORDER BY 'Date'[Month]";
    assert_eq!(numbers(&mut s, q, "[Run]"), want);
    // a moving window of two months, clipped at the start
    let q = "DEFINE MEASURE Sales[Two] = SUMX(WINDOW(-1, REL, 0, REL, ORDERBY('Date'[Month])), \
             CALCULATE([Total])) \
             EVALUATE SUMMARIZECOLUMNS('Date'[Month], \"Two\", [Two]) ORDER BY 'Date'[Month]";
    assert_eq!(
        numbers(&mut s, q, "[Two]"),
        vec![Some(35.0), Some(80.0), Some(90.0), Some(53.0)]
    );
    // absolute bounds: the last two months, the same on every row
    let q = "EVALUATE ROW(\"v\", SUMX(WINDOW(-2, ABS, -1, ABS, ALL('Date'[Month]), \
             ORDERBY('Date'[Month])), CALCULATE([Total])))";
    assert_eq!(one(&mut s, q), Some(53.0));
}

#[test]
fn offset_previous_row() {
    let mut s = session();
    let q = "DEFINE MEASURE Sales[Prev] = CALCULATE([Total], OFFSET(-1, ORDERBY('Date'[Month]))) \
             EVALUATE SUMMARIZECOLUMNS('Date'[Month], \"S\", [Total], \"Prev\", [Prev]) \
             ORDER BY 'Date'[Month]";
    assert_eq!(
        numbers(&mut s, q, "[Prev]"),
        vec![None, Some(35.0), Some(45.0), Some(45.0)]
    );
    // within each region, by day, in the row context of each sale
    let q = "EVALUATE ADDCOLUMNS(Sales, \"PrevAmt\", SELECTCOLUMNS(OFFSET(-1, Sales, \
             ORDERBY(Sales[Day]), PARTITIONBY(Sales[Region])), \"a\", Sales[Amount])) \
             ORDER BY Sales[Id]";
    assert_eq!(
        numbers(&mut s, q, "[PrevAmt]"),
        vec![
            None,
            Some(10.0),
            Some(20.0),
            None,
            Some(5.0),
            Some(15.0),
            Some(30.0),
            Some(25.0),
            Some(8.0)
        ]
    );
    // past the end of the partition: empty
    let q = "EVALUATE ROW(\"v\", CALCULATE(COUNTROWS(OFFSET(1, ALL('Date'[Month]), \
             ORDERBY('Date'[Month]))), 'Date'[Month] = 4))";
    assert_eq!(one(&mut s, q), None);
}

#[test]
fn rank_ties_and_partitions() {
    let mut s = session();
    let q = "EVALUATE ADDCOLUMNS(Sales, \
             \"R\", RANK(SKIP, Sales, ORDERBY(Sales[Amount], DESC), PARTITIONBY(Sales[Region])), \
             \"D\", RANK(DENSE, ALL(Sales), ORDERBY(Sales[Amount], DESC)), \
             \"F\", RANK(SKIP, Sales, ORDERBY(Sales[Amount], DESC, FIRST), PARTITIONBY(Sales[Region]))) \
             ORDER BY Sales[Id]";
    let int = |xs: &[i64]| xs.iter().map(|x| Some(*x as f64)).collect::<Vec<_>>();
    // East 30 > 20 = 20 > 10 (SKIP: 1, 2, 2, 4); West 25 > 15 > 8 > 5 > blank (blank last when
    // descending by default)
    assert_eq!(numbers(&mut s, q, "[R]"), int(&[4, 2, 1, 4, 2, 1, 2, 3, 5]));
    // dense over every sale: 30, 25, 20, 15, 10, 8, 5, blank
    assert_eq!(numbers(&mut s, q, "[D]"), int(&[5, 3, 1, 7, 4, 2, 3, 6, 8]));
    // blanks FIRST: the blank sale of the West ranks first
    assert_eq!(numbers(&mut s, q, "[F]"), int(&[4, 2, 1, 5, 3, 2, 2, 4, 1]));
    // ascending is the default
    let q = "EVALUATE ROW(\"v\", CALCULATE(MAXX(Sales, RANK(DENSE, ALL(Sales), \
             ORDERBY(Sales[Amount]))), Sales[Id] = 3))";
    assert_eq!(one(&mut s, q), Some(8.0));
    // a rank in a measure, by month, from the filter context
    let q = "DEFINE MEASURE Sales[Rk] = RANK(DENSE, ALLSELECTED('Date'[Month]), \
             ORDERBY([Total], DESC)) \
             EVALUATE SUMMARIZECOLUMNS('Date'[Month], \"Rk\", [Rk]) ORDER BY 'Date'[Month]";
    // totals 35, 45, 45, 8
    assert_eq!(numbers(&mut s, q, "[Rk]"), int(&[2, 1, 1, 3]));
}

#[test]
fn rownumber_breaks_ties() {
    let mut s = session();
    // the East's tie at 20 (sales 2 and 7) is broken by the fewest columns that tell the rows
    // apart: Id, ascending
    let q = "EVALUATE ADDCOLUMNS(Sales, \"N\", ROWNUMBER(Sales, ORDERBY(Sales[Amount], DESC), \
             PARTITIONBY(Sales[Region]))) ORDER BY Sales[Id]";
    let int = |xs: &[i64]| xs.iter().map(|x| Some(*x as f64)).collect::<Vec<_>>();
    assert_eq!(numbers(&mut s, q, "[N]"), int(&[4, 2, 1, 4, 2, 1, 3, 3, 5]));
    let q = "DEFINE MEASURE Sales[N] = ROWNUMBER(ORDERBY('Date'[Month], DESC)) \
             EVALUATE SUMMARIZECOLUMNS('Date'[Month], \"N\", [N]) ORDER BY 'Date'[Month]";
    assert_eq!(numbers(&mut s, q, "[N]"), int(&[4, 3, 2, 1]));
}

#[test]
fn index_first_and_last() {
    let mut s = session();
    let q = "EVALUATE ROW(\
             \"first\", MAXX(INDEX(1, ALL(Sales), ORDERBY(Sales[Day])), Sales[Id]), \
             \"last\", MAXX(INDEX(-1, ALL(Sales), ORDERBY(Sales[Day])), Sales[Id]), \
             \"second\", MAXX(INDEX(2, ALL(Sales), ORDERBY(Sales[Amount], DESC)), Sales[Id]), \
             \"zero\", COUNTROWS(INDEX(0, ALL(Sales), ORDERBY(Sales[Day]))), \
             \"far\", COUNTROWS(INDEX(99, ALL(Sales), ORDERBY(Sales[Day]))))";
    let v = eval(&mut s, q);
    let row = &v["results"][0]["rows"][0];
    assert_eq!(row["[first]"].as_f64(), Some(1.0), "{row}");
    assert_eq!(row["[last]"].as_f64(), Some(9.0), "{row}");
    assert_eq!(row["[second]"].as_f64(), Some(6.0), "{row}");
    assert_eq!(row["[zero]"], Value::Null, "{row}");
    assert_eq!(row["[far]"], Value::Null, "{row}");
    // the last day of each region: the partition from the filter context
    let q = "EVALUATE SUMMARIZECOLUMNS(Sales[Region], \"Last\", MAXX(INDEX(-1, \
             ALLSELECTED(Sales[Region], Sales[Day]), ORDERBY(Sales[Day]), \
             PARTITIONBY(Sales[Region])), Sales[Day])) ORDER BY Sales[Region]";
    assert_eq!(
        column(&mut s, q, "[Last]"),
        vec![json!("2024-03-31T00:00:00"), json!("2024-04-20T00:00:00")]
    );
}

#[test]
fn groupby_with_currentgroup() {
    let mut s = session();
    let q = "EVALUATE GROUPBY(Sales, Sales[Region], \"Total\", SUMX(CURRENTGROUP(), Sales[Amount]), \
             \"N\", COUNTX(CURRENTGROUP(), Sales[Id])) ORDER BY Sales[Region]";
    assert_eq!(
        column(&mut s, q, "Sales[Region]"),
        vec![json!("East"), json!("West")]
    );
    assert_eq!(numbers(&mut s, q, "[Total]"), vec![Some(80.0), Some(53.0)]);
    assert_eq!(numbers(&mut s, q, "[N]"), vec![Some(4.0), Some(5.0)]);
    assert_eq!(
        eval(&mut s, q)["results"][0]["columns"],
        json!([
            {"name": "Sales[Region]", "type": "string"},
            {"name": "[Total]", "type": "double"},
            {"name": "[N]", "type": "int64"},
        ])
    );
    // a column of a table the sales reach through a relationship
    let q = "EVALUATE GROUPBY(Sales, Region[Manager], \"T\", SUMX(CURRENTGROUP(), Sales[Amount])) \
             ORDER BY Region[Manager]";
    assert_eq!(numbers(&mut s, q, "[T]"), vec![Some(80.0), Some(53.0)]);
    // group keys keep their lineage: the result filters
    let q = "EVALUATE ROW(\"v\", CALCULATE([Total], GROUPBY(FILTER(Sales, Sales[Amount] > 25), \
             Sales[Region])))";
    assert_eq!(one(&mut s, q), Some(80.0));
    let e = error(&mut s, "EVALUATE ROW(\"v\", SUMX(CURRENTGROUP(), 1))");
    assert!(e.to_lowercase().contains("currentgroup"), "{e}");
}

#[test]
fn balances_on_the_date_table() {
    let mut s = session();
    let q = "EVALUATE ROW(\
             \"cm\", CALCULATE(CLOSINGBALANCEMONTH([Total], 'Date'[Date]), 'Date'[Month] = 3), \
             \"om\", CALCULATE(OPENINGBALANCEMONTH([Total], 'Date'[Date]), 'Date'[Month] = 3), \
             \"cmw\", CALCULATE(CLOSINGBALANCEMONTH([Total], 'Date'[Date], Sales[Region] = \"West\"), \
                'Date'[Month] = 3), \
             \"cq\", CALCULATE(CLOSINGBALANCEQUARTER([Total], 'Date'[Date]), 'Date'[Month] = 2), \
             \"oq\", CALCULATE(OPENINGBALANCEQUARTER([Total], 'Date'[Date]), 'Date'[Month] = 2), \
             \"oq2\", CALCULATE(OPENINGBALANCEQUARTER([Total], 'Date'[Date]), 'Date'[Month] = 4), \
             \"cy\", CALCULATE(CLOSINGBALANCEYEAR([Total], 'Date'[Date]), 'Date'[Month] = 1), \
             \"cyf\", CALCULATE(CLOSINGBALANCEYEAR([Total], 'Date'[Date], \"3/31\"), 'Date'[Month] = 1), \
             \"oy\", CALCULATE(OPENINGBALANCEYEAR([Total], 'Date'[Date], \"2/29\"), 'Date'[Month] = 3))";
    let v = eval(&mut s, q);
    let row = &v["results"][0]["rows"][0];
    let n = |k: &str| row[k].as_f64();
    // the last day of March: 25 + 20
    assert_eq!(n("[cm]"), Some(45.0), "{row}");
    // the end of February (2024-02-29)
    assert_eq!(n("[om]"), Some(15.0), "{row}");
    assert_eq!(n("[cmw]"), Some(25.0), "{row}");
    // the quarter of February closes on March 31
    assert_eq!(n("[cq]"), Some(45.0), "{row}");
    // the day before the first quarter is not in the date table
    assert_eq!(n("[oq]"), None, "{row}");
    // the second quarter opens on March 31
    assert_eq!(n("[oq2]"), Some(45.0), "{row}");
    // the year's last date in the table is 2024-04-30: no sale
    assert_eq!(n("[cy]"), None, "{row}");
    // a fiscal year ending on March 31
    assert_eq!(n("[cyf]"), Some(45.0), "{row}");
    // a fiscal year ending on February 29: the one holding March opens on 2024-02-29
    assert_eq!(n("[oy]"), Some(15.0), "{row}");
}

#[test]
fn natural_joins() {
    let mut s = session();
    let q = "EVALUATE ROW(\"inner\", COUNTROWS(NATURALINNERJOIN(Sales, Region)), \
             \"left\", COUNTROWS(NATURALLEFTOUTERJOIN(Region, Sales)))";
    let v = eval(&mut s, q);
    let row = &v["results"][0]["rows"][0];
    assert_eq!(row["[inner]"].as_f64(), Some(9.0), "{row}");
    // North has no sale: one more row, blank on the sales side
    assert_eq!(row["[left]"].as_f64(), Some(10.0), "{row}");
    let q = "EVALUATE FILTER(NATURALLEFTOUTERJOIN(Region, Sales), ISBLANK(Sales[Id]))";
    assert_eq!(column(&mut s, q, "Region[Region]"), vec![json!("North")]);
    let q = "EVALUATE ROW(\"v\", SUMX(NATURALINNERJOIN(Region, Sales), \
             IF(Region[Manager] = \"Bob\", Sales[Amount])))";
    assert_eq!(one(&mut s, q), Some(53.0));
    // two tables sharing no column lineage
    let e = error(&mut s, "EVALUATE NATURALINNERJOIN(Targets, Region)");
    assert!(e.to_lowercase().contains("common"), "{e}");
}

#[test]
fn hostile_window_arguments_never_panic() {
    let mut s = session();
    for q in [
        "EVALUATE ROW(\"v\", COUNTROWS(OFFSET(1E300, ALL(Sales), ORDERBY(Sales[Id]))))",
        "EVALUATE ROW(\"v\", COUNTROWS(OFFSET(-9223372036854775808, ALL(Sales), ORDERBY(Sales[Id]))))",
        "EVALUATE ROW(\"v\", COUNTROWS(INDEX(-1E308, ALL(Sales), ORDERBY(Sales[Id]))))",
        "EVALUATE ROW(\"v\", COUNTROWS(INDEX(BLANK(), ALL(Sales), ORDERBY(Sales[Id]))))",
        "EVALUATE ROW(\"v\", COUNTROWS(INDEX(1, FILTER(ALL(Sales), FALSE()), ORDERBY(Sales[Id]))))",
        "EVALUATE ROW(\"v\", RANK(DENSE, FILTER(ALL(Sales), FALSE()), ORDERBY(Sales[Id])))",
        "EVALUATE ROW(\"v\", ROWNUMBER(FILTER(ALL(Sales), FALSE()), ORDERBY(Sales[Id])))",
        "EVALUATE ROW(\"v\", COUNTROWS(GROUPBY(FILTER(Sales, FALSE()), Sales[Region])))",
        "EVALUATE ROW(\"v\", COUNTROWS(NATURALINNERJOIN(FILTER(Sales, FALSE()), Region)))",
    ] {
        assert_eq!(one(&mut s, q), None, "{q}");
    }
    // the whole relation, whatever the size of the bounds
    let q = "EVALUATE ROW(\"v\", COUNTROWS(WINDOW(-1E18, ABS, 1E18, ABS, ALL(Sales), \
             ORDERBY(Sales[Id]))))";
    assert_eq!(one(&mut s, q), Some(9.0));
    // OFFSET nested in its own relation, far deeper than the evaluator follows: an error
    let mut nested = "ALL(Sales)".to_string();
    for _ in 0..200 {
        nested = format!("OFFSET(0, {nested}, ORDERBY(Sales[Id]))");
    }
    let e = error(&mut s, &format!("EVALUATE {nested}"));
    assert!(e.contains("deep"), "{e}");
    // rows nothing tells apart
    let e = error(
        &mut s,
        "EVALUATE ROW(\"v\", COUNTROWS(OFFSET(1, SELECTCOLUMNS(Sales, \"P\", Sales[Product]), \
         ORDERBY([P]))))",
    );
    assert!(!e.is_empty());
}
