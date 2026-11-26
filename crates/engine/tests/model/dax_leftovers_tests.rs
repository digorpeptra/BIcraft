//! The G30 leftovers of the DAX evaluator, run by `dax.evaluate`: the subtotal form of
//! `ADDMISSINGITEMS` (`ROLLUPISSUBTOTAL`), the `ties` argument of `TOPNSKIP`, the week balances
//! of calendar-based time intelligence, the visual-calculation forms of the window functions
//! outside a visual calculation, and the wildcards of `SEARCH` and `CONTAINSSTRING`.
//!
//! The expected values follow the semantics of Microsoft Learn's public DAX reference (for
//! `TOPNSKIP`, which has no Learn page, dax.guide's description; the rules kept are in
//! `docs/formats/dax-functions.md`, "Evaluator coverage"). They are documented, **not
//! oracle-checked**: no engine oracle ran for them.

use serde_json::{Value, json};

use bicraft_engine::Session;

/// The small model of `tests/window_functions` (Sales by region and day, `[Total]`).
fn window_session() -> Session {
    let mut s = Session::new();
    s.model = Some(bicraft_tmsl::read(include_str!("../window_functions/model.bim")).unwrap());
    s
}

/// A weekly calendar model: `Cal` holds 2024-01-01 (a Monday) to 2024-01-21 without
/// 2024-01-07 (sparse), weeks W1 (1–6), W2 (8–14), W3 (15–21); `Stock[Qty]` is the day of the
/// month on each day; `Cal[IsWorking]` (tagged in no calendar) is FALSE on weekends,
/// `Cal[Holiday]` (time-related in "Weekly TR") is TRUE on the 14th.
fn calendar_session() -> Session {
    let mut cal_rows = Vec::new();
    let mut stock_rows = Vec::new();
    for d in 1..=21 {
        if d == 7 {
            continue;
        }
        let week = (d - 1) / 7 + 1;
        let working = (d - 1) % 7 < 5;
        cal_rows.push(format!(
            "{{\"2024-01-{d:02}\", \"W{week}\", {week}, 2024, \"2024-01\", {}, {}}}",
            if working { "TRUE" } else { "FALSE" },
            if d == 14 { "TRUE" } else { "FALSE" }
        ));
        stock_rows.push(format!("{{\"2024-01-{d:02}\", {d}}}"));
    }
    let cal = format!(
        "DATATABLE(\"Date\", DATETIME, \"Week\", STRING, \"WeekNo\", INTEGER, \"Year\", INTEGER, \
         \"Month\", STRING, \"IsWorking\", BOOLEAN, \"Holiday\", BOOLEAN, {{{}}})",
        cal_rows.join(", ")
    );
    let stock = format!(
        "DATATABLE(\"Day\", DATETIME, \"Qty\", DOUBLE, {{{}}})",
        stock_rows.join(", ")
    );
    let col = |name: &str, ty: &str| {
        json!({"type": "calculatedTableColumn", "name": name, "dataType": ty,
               "isNameInferred": true, "isDataTypeInferred": true, "sourceColumn": format!("[{name}]")})
    };
    let group = |unit: &str, col: &str| json!({"timeUnit": unit, "primaryColumn": col});
    let model = json!({
        "name": "BicraftWeekBalances",
        "compatibilityLevel": 1702,
        "model": {
            "culture": "en-US",
            "tables": [
                {
                    "name": "Cal",
                    "columns": [col("Date", "dateTime"), col("Week", "string"), col("WeekNo", "int64"),
                                col("Year", "int64"), col("Month", "string"), col("IsWorking", "boolean"),
                                col("Holiday", "boolean")],
                    "partitions": [{"name": "Cal", "source": {"type": "calculated", "expression": cal}}],
                    "calendars": [
                        {"name": "Weekly", "calendarColumnGroups": [
                            group("week", "Week"), group("date", "Date"), group("month", "Month")]},
                        {"name": "Weekly TR", "calendarColumnGroups": [
                            group("week", "Week"), group("date", "Date"), {"columns": ["Holiday"]}]},
                        {"name": "ByYear", "calendarColumnGroups": [
                            group("weekOfYear", "WeekNo"), group("year", "Year"), group("date", "Date")]},
                        {"name": "NoWeek", "calendarColumnGroups": [group("date", "Date")]},
                        {"name": "NoDate", "calendarColumnGroups": [group("week", "Week")]}
                    ]
                },
                {
                    "name": "Stock",
                    "columns": [col("Day", "dateTime"), col("Qty", "double")],
                    "partitions": [{"name": "Stock", "source": {"type": "calculated", "expression": stock}}],
                    "measures": [{"name": "Stock Qty", "expression": "SUM(Stock[Qty])"}]
                }
            ],
            "relationships": [
                {"name": "Stock_Cal", "fromTable": "Stock", "fromColumn": "Day", "toTable": "Cal", "toColumn": "Date"}
            ]
        }
    });
    let mut s = Session::new();
    s.model = Some(bicraft_tmsl::read(&model.to_string()).unwrap());
    s
}

fn eval(s: &mut Session, query: &str) -> Value {
    s.execute("dax.evaluate", &json!({ "query": query }))
        .unwrap_or_else(|e| panic!("{query}: {e}"))
}

/// The values of column `name` of the first result, in order; numbers as doubles (`json!(4.0)`
/// whatever the column's type).
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
        .map(|v| match v.as_f64() {
            Some(x) => json!(x),
            None => v,
        })
        .collect()
}

/// The one value of `EVALUATE ROW("v", expression)`.
fn value(s: &mut Session, expression: &str) -> Value {
    let q = format!("EVALUATE ROW(\"v\", {expression})");
    column(s, &q, "[v]").into_iter().next().unwrap_or(Value::Null)
}

/// The ids of the sales a table expression returns, ascending.
fn ids(s: &mut Session, table: &str) -> Vec<i64> {
    let q = format!("EVALUATE SELECTCOLUMNS({table}, \"Id\", Sales[Id]) ORDER BY [Id]");
    column(s, &q, "[Id]")
        .iter()
        .map(|v| v.as_f64().unwrap_or(f64::NAN) as i64)
        .collect()
}

fn error(s: &mut Session, query: &str) -> String {
    match s.execute("dax.evaluate", &json!({ "query": query })) {
        Ok(v) => panic!("{query}: expected an error, got {v}"),
        Err(e) => e,
    }
}

// --- ADDMISSINGITEMS with ROLLUPISSUBTOTAL -------------------------------------------------------

#[test]
fn addmissingitems_keeps_and_completes_the_subtotal_rows() {
    let mut s = window_session();
    let sc = "SUMMARIZECOLUMNS(ROLLUPADDISSUBTOTAL(Region[Region], \"IsTotal\"), \"T\", [Total])";
    // East 80, West 53 and the total 133 exist; North (no sales) is added as a detail row
    let q = format!(
        "EVALUATE ADDMISSINGITEMS(Region[Region], {sc}, ROLLUPISSUBTOTAL(Region[Region], [IsTotal])) \
         ORDER BY [IsTotal], Region[Region]"
    );
    assert_eq!(
        column(&mut s, &q, "Region[Region]"),
        vec![json!("East"), json!("North"), json!("West"), Value::Null]
    );
    assert_eq!(
        column(&mut s, &q, "[IsTotal]"),
        vec![json!(false), json!(false), json!(false), json!(true)]
    );
    assert_eq!(
        column(&mut s, &q, "[T]"),
        vec![json!(80.0), Value::Null, json!(53.0), json!(133.0)]
    );
    // the same with the group given as ROLLUPGROUP, and show-all columns left out
    let q = format!(
        "EVALUATE ADDMISSINGITEMS({sc}, ROLLUPISSUBTOTAL(ROLLUPGROUP(Region[Region]), [IsTotal])) \
         ORDER BY [IsTotal], Region[Region]"
    );
    assert_eq!(column(&mut s, &q, "[T]").len(), 4);
    // nothing with data: every detail row and the total row come back, blank
    let q = format!(
        "EVALUATE ADDMISSINGITEMS(Region[Region], FILTER({sc}, [T] > 1000), \
         ROLLUPISSUBTOTAL(Region[Region], [IsTotal])) ORDER BY [IsTotal], Region[Region]"
    );
    assert_eq!(
        column(&mut s, &q, "[IsTotal]"),
        vec![json!(false), json!(false), json!(false), json!(true)]
    );
    assert_eq!(column(&mut s, &q, "[T]"), vec![Value::Null; 4]);
    // a filter table limits the items of every level
    let q = "EVALUATE ADDMISSINGITEMS(Region[Region], \
             SUMMARIZECOLUMNS(ROLLUPADDISSUBTOTAL(Region[Region], \"IsTotal\"), \
             TREATAS({\"East\", \"North\"}, Region[Region]), \"T\", [Total]), \
             ROLLUPISSUBTOTAL(Region[Region], [IsTotal]), TREATAS({\"East\", \"North\"}, Region[Region])) \
             ORDER BY [IsTotal], Region[Region]";
    assert_eq!(
        column(&mut s, q, "Region[Region]"),
        vec![json!("East"), json!("North"), Value::Null]
    );
}

#[test]
fn addmissingitems_subtotal_errors() {
    let mut s = window_session();
    let sc = "SUMMARIZECOLUMNS(ROLLUPADDISSUBTOTAL(Region[Region], \"IsTotal\"), \"T\", [Total])";
    // the subtotal column is not in the table
    let e = error(
        &mut s,
        &format!("EVALUATE ADDMISSINGITEMS(Region[Region], {sc}, ROLLUPISSUBTOTAL(Region[Region], [Nope]))"),
    );
    assert!(e.contains("Nope"), "{e}");
    // the group-by column without its subtotal column
    let e = error(
        &mut s,
        &format!("EVALUATE ADDMISSINGITEMS(Region[Region], {sc}, ROLLUPISSUBTOTAL(Region[Region]))"),
    );
    assert!(e.contains("ROLLUPISSUBTOTAL"), "{e}");
    // two filters for one level (the grand total's here)
    let e = error(
        &mut s,
        &format!(
            "EVALUATE ADDMISSINGITEMS(Region[Region], {sc}, \
             ROLLUPISSUBTOTAL(TREATAS({{\"East\"}}, Region[Region]), \
             TREATAS({{\"West\"}}, Region[Region]), Region[Region], [IsTotal]))"
        ),
    );
    assert!(e.contains("filter"), "{e}");
}

// --- TOPNSKIP ties ---------------------------------------------------------------------------------

#[test]
fn topnskip_ties_rank_the_rows() {
    let mut s = window_session();
    // descending amounts: 30 (3), 25 (6), 20 (2), 20 (7), 15 (5), 10 (1), 8 (8), 5 (4), blank (9);
    // SKIP ranks 1, 2, 3, 3, 5, 6, 7, 8, 9; DENSE ranks 1, 2, 3, 3, 4, 5, 6, 7, 8
    assert_eq!(
        ids(&mut s, "TOPNSKIP(3, 0, Sales, Sales[Amount], DESC, SKIP)"),
        vec![2, 3, 6, 7]
    );
    assert_eq!(
        ids(&mut s, "TOPNSKIP(1, 2, Sales, Sales[Amount], DESC, SKIP)"),
        vec![2, 7]
    );
    // rank 4 is skipped: nothing ranks in (3, 4]
    assert!(ids(&mut s, "TOPNSKIP(1, 3, Sales, Sales[Amount], DESC, SKIP)").is_empty());
    assert_eq!(
        ids(&mut s, "TOPNSKIP(2, 3, Sales, Sales[Amount], DESC, DENSE)"),
        vec![1, 5]
    );
    assert_eq!(
        ids(&mut s, "TOPNSKIP(1, 2, Sales, Sales[Amount], DENSE)"),
        vec![2, 7]
    );
    // ascending: the blank ranks first
    assert_eq!(
        ids(&mut s, "TOPNSKIP(2, 0, Sales, Sales[Amount], ASC, DENSE)"),
        vec![4, 9]
    );
    // text ties without regard to case; zero rows is empty
    assert_eq!(
        ids(
            &mut s,
            "TOPNSKIP(1, 0, Sales, IF(Sales[Id] = 1, \"a\", Sales[Product]), ASC, SKIP)"
        ),
        vec![1, 3, 4, 8]
    );
    assert!(ids(&mut s, "TOPNSKIP(0, 0, Sales, Sales[Amount], DESC, SKIP)").is_empty());
}

// --- week balances (calendar-based time intelligence) ----------------------------------------------

#[test]
fn closing_and_opening_week_balances_follow_the_calendar() {
    let mut s = calendar_session();
    let closing = "CLOSINGBALANCEWEEK([Stock Qty], Weekly)";
    let opening = "OPENINGBALANCEWEEK([Stock Qty], Weekly)";
    // per week: the closing balance is the week's last day (W1 ends on the 6th, the 7th is not in
    // the calendar), the opening balance the last day before the week
    let q = format!(
        "EVALUATE SUMMARIZECOLUMNS(Cal[Week], \"c\", {closing}, \"o\", {opening}) ORDER BY Cal[Week]"
    );
    assert_eq!(
        column(&mut s, &q, "[c]"),
        vec![json!(6.0), json!(14.0), json!(21.0)]
    );
    assert_eq!(
        column(&mut s, &q, "[o]"),
        vec![Value::Null, json!(6.0), json!(14.0)]
    );
    // one day: the balances of its week
    let on_day = |f: &str| format!("CALCULATE({f}, Cal[Date] = DATE(2024, 1, 10))");
    assert_eq!(value(&mut s, &on_day(closing)), json!(14.0));
    assert_eq!(value(&mut s, &on_day(opening)), json!(6.0));
    // the whole calendar: the last week's end, nothing before the first week
    assert_eq!(value(&mut s, closing), json!(21.0));
    assert_eq!(value(&mut s, opening), Value::Null);
    // context transition in an iterator: 6 + 14 + 21
    assert_eq!(
        value(&mut s, &format!("SUMX(VALUES(Cal[Week]), {closing})")),
        json!(41.0)
    );
    // the filter argument applies after the day is set
    assert_eq!(
        value(
            &mut s,
            "CLOSINGBALANCEWEEK([Stock Qty], Weekly, Stock[Qty] > 100)"
        ),
        Value::Null
    );
    // the week of year and the year identify the weeks just as well; a quoted name works
    assert_eq!(
        value(&mut s, &on_day("CLOSINGBALANCEWEEK([Stock Qty], 'ByYear')")),
        json!(14.0)
    );
    // a month balance over the calendar: its Month category
    assert_eq!(
        value(&mut s, &on_day("CLOSINGBALANCEMONTH([Stock Qty], Weekly)")),
        json!(21.0)
    );
}

#[test]
fn week_balances_clear_the_tagged_columns_and_keep_the_others() {
    let mut s = calendar_session();
    // IsWorking is tagged in no calendar: its filter stays, and the week's last day (a Sunday)
    // is not a working day, so the balance is blank
    let q = "CALCULATE(CLOSINGBALANCEWEEK([Stock Qty], Weekly), Cal[Week] = \"W2\", Cal[IsWorking] = TRUE())";
    assert_eq!(value(&mut s, q), Value::Null);
    // Holiday is a time-related column of a calendar of the table: cleared for every calendar of
    // the table, so the 14th counts although it is a holiday
    for cal in ["Weekly", "Weekly TR"] {
        let q = format!(
            "CALCULATE(CLOSINGBALANCEWEEK([Stock Qty], '{cal}'), Cal[Week] = \"W2\", Cal[Holiday] = FALSE())"
        );
        assert_eq!(value(&mut s, &q), json!(14.0), "{cal}");
    }
    // a filter on a tagged column (the week) is replaced by the balance's day
    assert_eq!(
        value(
            &mut s,
            "CALCULATE(OPENINGBALANCEWEEK([Stock Qty], Weekly), Cal[Week] = \"W3\")"
        ),
        json!(14.0)
    );
}

#[test]
fn week_balance_errors() {
    let mut s = calendar_session();
    for (q, want) in [
        ("CLOSINGBALANCEWEEK([Stock Qty], NoWeek)", "week"),
        ("OPENINGBALANCEWEEK([Stock Qty], NoDate)", "Date"),
        ("CLOSINGBALANCEWEEK([Stock Qty], Cal[Date])", "calendar"),
    ] {
        let e = error(&mut s, &format!("EVALUATE ROW(\"v\", {q})"));
        assert!(e.contains(want), "{q}: {e}");
    }
}

// --- visual-calculation forms outside a visual calculation ----------------------------------------

#[test]
fn axes_and_resets_are_errors_outside_visual_calculations() {
    let mut s = window_session();
    let forms = [
        "COUNTROWS(OFFSET(1, ALL(Sales[Id]), ORDERBY(Sales[Id]), ROWS))",
        "COUNTROWS(WINDOW(1, ABS, 2, ABS, ALL(Sales[Id]), ORDERBY(Sales[Id]), HIGHESTPARENT))",
        "COUNTROWS(OFFSET(1, ALL(Sales[Id]), ORDERBY(Sales[Id]), DEFAULT, PARTITIONBY(Sales[Region]), \
         ALL(Sales[Id])))",
    ];
    // in a query the validator refuses them already
    let e = error(&mut s, &format!("EVALUATE ROW(\"v\", {})", forms[0]));
    assert!(e.contains("ROWS"), "{e}");
    // a model measure holding them (a file, or `force`) is an evaluation error, not "not
    // supported"
    if let Some(t) = s
        .model
        .as_mut()
        .and_then(|m| m.tables.iter_mut().find(|t| t.name == "Sales"))
    {
        for (i, f) in forms.iter().enumerate() {
            t.measures.push(bicraft_model::Measure {
                name: format!("Form{i}"),
                expression: (*f).to_string(),
                ..Default::default()
            });
        }
    }
    for i in 0..forms.len() {
        let e = error(&mut s, &format!("EVALUATE ROW(\"v\", [Form{i}])"));
        assert!(!e.contains("not supported"), "{i}: {e}");
        assert!(e.contains("visual calculation"), "{i}: {e}");
    }
}

// --- wildcards -----------------------------------------------------------------------------------

#[test]
fn search_and_containsstring_read_wildcards() {
    let mut s = window_session();
    for (expr, want) in [
        ("SEARCH(\"b?d\", \"abcbed\")", json!(4.0)),
        ("SEARCH(\"*\", \"abc\")", json!(1.0)),
        ("SEARCH(\"C*E\", \"abcde\")", json!(3.0)),
        ("SEARCH(\"a*\", \"banana\", 3)", json!(4.0)),
        ("SEARCH(\"~*\", \"a*b\")", json!(2.0)),
        ("SEARCH(\"~~\", \"a~b\")", json!(2.0)),
        ("SEARCH(\"~?\", \"ab\", 1, -1)", json!(-1.0)),
        ("SEARCH(\"b?\", \"ab\", 1, 0)", json!(0.0)),
        ("SEARCH(\"x*\", \"abc\", 1, 0)", json!(0.0)),
        // FIND has no wildcards
        ("FIND(\"*\", \"a*b\")", json!(2.0)),
        ("FIND(\"?\", \"ab\", 1, 0)", json!(0.0)),
        ("CONTAINSSTRING(\"Bonjour\", \"b*j?ur\")", json!(true)),
        ("CONTAINSSTRING(\"Bonjour\", \"j*x\")", json!(false)),
        ("CONTAINSSTRING(\"a~b\", \"~\")", json!(true)),
        ("CONTAINSSTRING(\"ab\", \"a~*\")", json!(false)),
        ("CONTAINSSTRINGEXACT(\"a*b\", \"*\")", json!(true)),
        ("CONTAINSSTRINGEXACT(\"ab\", \"?\")", json!(false)),
    ] {
        assert_eq!(value(&mut s, expr), want, "{expr}");
    }
    let e = error(&mut s, "EVALUATE ROW(\"v\", SEARCH(\"b?\", \"ab\"))");
    assert!(e.contains("not found"), "{e}");
}
