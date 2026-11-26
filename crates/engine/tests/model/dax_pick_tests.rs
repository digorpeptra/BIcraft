//! `ADDMISSINGITEMS`, `SUBSTITUTEWITHINDEX`, `SAMPLE` and `TOPNSKIP`, run by `dax.evaluate` on
//! the small model of `tests/window_functions` (data from its own partitions).
//!
//! The expected values follow the semantics of Microsoft Learn's public DAX reference (for
//! `TOPNSKIP`, which has no Learn page, the rules of `preview_eval_pick.rs`); they are documented,
//! not oracle-checked (the engine oracle's cases for these functions are in
//! `coverage_oracle_matches_the_engine`).
//!
//! Sales (Id, Region, Product, Day, Amount): 1 East A 10, 2 East B 20, 3 East A 30, 4 West A 5,
//! 5 West B 15, 6 West B 25, 7 East B 20, 8 West A 8, 9 West B blank; Region: East, West, North
//! (no sales).

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

/// The ids of the sales a table expression returns, ascending.
fn ids(s: &mut Session, table: &str) -> Vec<Option<f64>> {
    let q = format!("EVALUATE SELECTCOLUMNS({table}, \"Id\", Sales[Id]) ORDER BY [Id]");
    numbers(s, &q, "[Id]")
}

/// `COUNTROWS` of a table expression (blank: 0).
fn rows(s: &mut Session, table: &str) -> f64 {
    let q = format!("EVALUATE ROW(\"v\", COUNTROWS({table}) + 0)");
    numbers(s, &q, "[v]")
        .first()
        .copied()
        .flatten()
        .unwrap_or(f64::NAN)
}

fn error(s: &mut Session, query: &str) -> String {
    match s.execute("dax.evaluate", &json!({ "query": query })) {
        Ok(v) => panic!("{query}: expected an error, got {v}"),
        Err(e) => e,
    }
}

fn some(v: &[f64]) -> Vec<Option<f64>> {
    v.iter().map(|x| Some(*x)).collect()
}

#[test]
fn sample_keeps_the_first_last_and_evenly_spaced_rows() {
    let mut s = session();
    // the reference's example shape: 3 rows of 8 are the 1st, 5th and 8th in order
    let eight = "FILTER(Sales, Sales[Id] <= 8)";
    assert_eq!(
        ids(&mut s, &format!("SAMPLE(3, {eight}, Sales[Id])")),
        some(&[1.0, 5.0, 8.0])
    );
    assert_eq!(
        ids(&mut s, &format!("SAMPLE(3, {eight}, Sales[Id], ASC)")),
        some(&[1.0, 5.0, 8.0])
    );
    // descending: 8, 4, 1 (the 1st, 5th and 8th from the top)
    assert_eq!(
        ids(&mut s, &format!("SAMPLE(3, {eight}, Sales[Id], DESC)")),
        some(&[1.0, 4.0, 8.0])
    );
    assert_eq!(
        ids(&mut s, &format!("SAMPLE(3, {eight}, Sales[Id], 0)")),
        some(&[1.0, 4.0, 8.0])
    );
    assert_eq!(
        ids(&mut s, "SAMPLE(4, Sales, Sales[Id])"),
        some(&[1.0, 4.0, 6.0, 9.0])
    );
    // one row: the first; more rows than the table: all of it; a fraction is truncated
    assert_eq!(ids(&mut s, "SAMPLE(1, Sales, Sales[Id], DESC)"), some(&[9.0]));
    assert_eq!(rows(&mut s, "SAMPLE(20, Sales, Sales[Id])"), 9.0);
    assert_eq!(
        ids(&mut s, &format!("SAMPLE(2.9, {eight}, Sales[Id])")),
        some(&[1.0, 8.0])
    );
    // a blank key sorts first ascending
    assert_eq!(ids(&mut s, "SAMPLE(2, Sales, Sales[Amount])"), some(&[3.0, 9.0]));
    // ties on the first key are ordered by the next one
    assert_eq!(
        ids(
            &mut s,
            "SAMPLE(3, Sales, Sales[Product], ASC, Sales[Amount], DESC)"
        ),
        some(&[3.0, 6.0, 9.0])
    );
}

#[test]
fn sample_edges() {
    let mut s = session();
    assert_eq!(rows(&mut s, "SAMPLE(0, Sales, Sales[Id])"), 0.0);
    assert_eq!(rows(&mut s, "SAMPLE(-4, Sales, Sales[Id])"), 0.0);
    assert_eq!(rows(&mut s, "SAMPLE(BLANK(), Sales, Sales[Id])"), 0.0);
    assert_eq!(rows(&mut s, "SAMPLE(3, FILTER(Sales, FALSE()), Sales[Id])"), 0.0);
    // the sample keeps the table's columns (and their lineage: it filters)
    let q = "EVALUATE ROW(\"v\", CALCULATE([Total], SAMPLE(2, Sales, Sales[Id])))";
    assert_eq!(numbers(&mut s, q, "[v]"), vec![Some(10.0)]);
    let q = "EVALUATE ROW(\"v\", CALCULATE([Total], SAMPLE(2, FILTER(Sales, Sales[Id] <= 8), Sales[Id])))";
    assert_eq!(numbers(&mut s, q, "[v]"), vec![Some(18.0)]);
}

#[test]
fn topnskip_skips_then_takes_without_ties() {
    let mut s = session();
    // descending by default: 30 (3), 25 (6), 20 (2), 20 (7)…; skip one, take two, no tie added
    assert_eq!(
        ids(&mut s, "TOPNSKIP(2, 1, Sales, Sales[Amount])"),
        some(&[2.0, 6.0])
    );
    assert_eq!(rows(&mut s, "TOPN(3, Sales, Sales[Amount])"), 4.0);
    assert_eq!(rows(&mut s, "TOPNSKIP(3, 0, Sales, Sales[Amount])"), 3.0);
    // ascending: the blank first
    assert_eq!(
        ids(&mut s, "TOPNSKIP(3, 0, Sales, Sales[Amount], ASC)"),
        some(&[4.0, 8.0, 9.0])
    );
    assert_eq!(
        ids(&mut s, "TOPNSKIP(2, 7, Sales, Sales[Id], 1)"),
        some(&[8.0, 9.0])
    );
    // a negative skip is no skip; past the end is empty; no rows is empty
    assert_eq!(
        ids(&mut s, "TOPNSKIP(1, -5, Sales, Sales[Id], ASC)"),
        some(&[1.0])
    );
    assert_eq!(rows(&mut s, "TOPNSKIP(2, 100, Sales, Sales[Id])"), 0.0);
    assert_eq!(rows(&mut s, "TOPNSKIP(0, 0, Sales, Sales[Id])"), 0.0);
    assert_eq!(rows(&mut s, "TOPNSKIP(-1, 0, Sales, Sales[Id])"), 0.0);
    assert_eq!(rows(&mut s, "TOPNSKIP(BLANK(), 0, Sales, Sales[Id])"), 0.0);
    assert_eq!(
        rows(&mut s, "TOPNSKIP(5, 0, FILTER(Sales, FALSE()), Sales[Id])"),
        0.0
    );
    // without an order key: the table's order
    assert_eq!(rows(&mut s, "TOPNSKIP(4, 2, Sales)"), 4.0);
    // identical rows tie with the last one kept and come along
    assert_eq!(
        rows(
            &mut s,
            "TOPNSKIP(1, 0, SELECTCOLUMNS(Sales, \"P\", Sales[Product]), [P])"
        ),
        5.0
    );
    // the ties argument: `dax_leftovers_tests.rs`
}

#[test]
fn substitutewithindex_replaces_the_common_columns_by_an_index() {
    let mut s = session();
    // A is 0 and B is 1 ascending, in the place of the Product column
    let q = "EVALUATE SUBSTITUTEWITHINDEX(Sales, \"i\", VALUES(Sales[Product]), Sales[Product], ASC) \
             ORDER BY Sales[Id]";
    let want = some(&[0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0]);
    assert_eq!(numbers(&mut s, q, "[i]"), want);
    let names: Vec<Value> = eval(&mut s, q)["results"][0]["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].clone())
        .collect();
    assert_eq!(
        names,
        vec![
            json!("Sales[Id]"),
            json!("Sales[Region]"),
            json!("[i]"),
            json!("Sales[Day]"),
            json!("Sales[Amount]"),
        ]
    );
    // descending when the order is left out: B is 0
    let q = "EVALUATE SUBSTITUTEWITHINDEX(Sales, \"i\", VALUES(Sales[Product]), Sales[Product]) \
             ORDER BY Sales[Id]";
    assert_eq!(
        numbers(&mut s, q, "[i]"),
        some(&[1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0])
    );
    // a left semijoin: the rows without a match are dropped
    let only_a = "FILTER(VALUES(Sales[Product]), Sales[Product] = \"A\")";
    assert_eq!(
        rows(
            &mut s,
            &format!("SUBSTITUTEWITHINDEX(Sales, \"i\", {only_a}, Sales[Product])")
        ),
        4.0
    );
    // two common columns (by name), a right column the left table lacks is left out
    let pairs = "DATATABLE(\"Region\", STRING, \"Product\", STRING, \"Extra\", INTEGER, \
                 {{\"West\", \"B\", 1}, {\"East\", \"A\", 2}})";
    let q = format!("EVALUATE SUBSTITUTEWITHINDEX(Sales, \"i\", {pairs}, [Region], ASC) ORDER BY Sales[Id]");
    assert_eq!(numbers(&mut s, &q, "[i]"), some(&[0.0, 0.0, 1.0, 1.0, 1.0]));
    assert_eq!(numbers(&mut s, &q, "Sales[Id]"), some(&[1.0, 3.0, 5.0, 6.0, 9.0]));
    // an empty index table: no row
    assert_eq!(
        rows(
            &mut s,
            "SUBSTITUTEWITHINDEX(Sales, \"i\", FILTER(VALUES(Sales[Product]), FALSE()), Sales[Product])"
        ),
        0.0
    );
}

#[test]
fn substitutewithindex_errors() {
    let mut s = session();
    // same name, another data type: nothing in common
    let e = error(
        &mut s,
        "EVALUATE SUBSTITUTEWITHINDEX(Sales, \"i\", DATATABLE(\"Id\", STRING, {{\"1\"}}), [Id])",
    );
    assert!(e.contains("no column in common"), "{e}");
    // the index name clashes with a column that stays
    let e = error(
        &mut s,
        "EVALUATE SUBSTITUTEWITHINDEX(Sales, \"Amount\", VALUES(Sales[Product]), Sales[Product])",
    );
    assert!(e.contains("Amount"), "{e}");
}

#[test]
fn addmissingitems_adds_the_combinations_without_data() {
    let mut s = session();
    let sc = "SUMMARIZECOLUMNS(Region[Region], \"T\", [Total])";
    let q = format!("EVALUATE ADDMISSINGITEMS(Region[Region], {sc}, Region[Region]) ORDER BY Region[Region]");
    assert_eq!(
        column(&mut s, &q, "Region[Region]"),
        vec![json!("East"), json!("North"), json!("West")]
    );
    assert_eq!(numbers(&mut s, &q, "[T]"), vec![Some(80.0), None, Some(53.0)]);
    // without show-all columns: the group-by columns
    let q = format!("EVALUATE ADDMISSINGITEMS({sc}, Region[Region]) ORDER BY Region[Region]");
    assert_eq!(numbers(&mut s, &q, "[T]"), vec![Some(80.0), None, Some(53.0)]);
    // a filter table limits the items added
    let q = "EVALUATE ADDMISSINGITEMS(Region[Region], SUMMARIZECOLUMNS(Region[Region], \
             TREATAS({\"East\", \"North\"}, Region[Region]), \"T\", [Total]), Region[Region], \
             TREATAS({\"East\", \"North\"}, Region[Region])) ORDER BY Region[Region]";
    assert_eq!(
        column(&mut s, q, "Region[Region]"),
        vec![json!("East"), json!("North")]
    );
    // columns of two tables: their items crossed (3 regions × 2 products)
    let q = "ADDMISSINGITEMS(Region[Region], Sales[Product], SUMMARIZECOLUMNS(Region[Region], \
             Sales[Product], \"T\", [Total]), Region[Region], Sales[Product])";
    assert_eq!(rows(&mut s, q), 6.0);
    // nothing with data: every item, blank values
    let q = format!("ADDMISSINGITEMS(Region[Region], FILTER({sc}, FALSE()), Region[Region])");
    assert_eq!(rows(&mut s, &q), 3.0);
    // in a measure, under the filter context
    let q = format!(
        "EVALUATE ROW(\"v\", CALCULATE(COUNTROWS(ADDMISSINGITEMS(Region[Region], {sc}, \
         Region[Region])), Region[Region] <> \"West\"))"
    );
    assert_eq!(numbers(&mut s, &q, "[v]"), vec![Some(2.0)]);
    // the group-by column missing from the table, a subtotal form without its subtotal column
    // (the subtotal form itself: `dax_leftovers_tests.rs`)
    let e = error(
        &mut s,
        "EVALUATE ADDMISSINGITEMS(Region[Region], SUMMARIZECOLUMNS(Sales[Product], \"T\", [Total]), \
         Region[Region])",
    );
    assert!(e.contains("Region"), "{e}");
    let e = error(
        &mut s,
        "EVALUATE ADDMISSINGITEMS(Region[Region], SUMMARIZECOLUMNS(Region[Region], \"T\", [Total]), \
         ROLLUPISSUBTOTAL(Region[Region], Region[Manager]))",
    );
    assert!(e.contains("ROLLUPISSUBTOTAL"), "{e}");
}
