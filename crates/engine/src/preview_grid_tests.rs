//! The viewer's visuals through the storage engine ([`super::grid`], gap G70): a visual over a
//! table larger than the row-by-row evaluator's cap draws its values, the row cap applies only
//! when the visual falls back to that evaluator (named in its reason), and a varied report gives
//! the same data the old order (the row-by-row evaluator first) gave, on literal and on stored
//! data.

use serde_json::{Value, json};

use crate::Session;
use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn add(s: &mut Session, page: &str, visual_type: &str, fields: Value) -> String {
    let r = run(
        s,
        "visual.add",
        json!({"page": page, "visual_type": visual_type, "x": 0, "y": 0, "width": 300, "height": 200,
               "fields": fields}),
    );
    r["visual"].as_str().map(str::to_string).unwrap_or_default()
}

/// Each visual's `data`, by page and visual name.
type Visuals = Vec<(String, Value)>;

/// Every visual's `data` in `snap`, by page and visual name.
fn all_data(snap: &Value) -> Visuals {
    let mut out = Vec::new();
    for p in snap["report"]["pages"].as_array().into_iter().flatten() {
        for v in p["visuals"].as_array().into_iter().flatten() {
            let id = format!("{}/{} ({})", p["display_name"], v["name"], v["type"]);
            out.push((id, v["data"].clone()));
        }
    }
    out
}

fn find(snap: &Value, visual: &str) -> Value {
    for p in snap["report"]["pages"].as_array().into_iter().flatten() {
        for v in p["visuals"].as_array().into_iter().flatten() {
            if v["name"] == visual {
                return v["data"].clone();
            }
        }
    }
    panic!("visual {visual} not in the snapshot")
}

/// `Sales` of `rows` rows made by M (`Id`, `Product`, `Region` with blanks, `Day`, `Amount` with
/// blanks, `Price` as currency, `Qty`) and `Product(Product, Category)` that lacks some products.
fn model(rows: usize) -> Session {
    let mut s = Session::new();
    let sales = format!(
        "let n = List.Numbers(1, {rows}), t = Table.FromColumns({{n, \
         List.Transform(n, each \"P\" & Text.From(Number.Mod(_ * 7, 23))), \
         List.Transform(n, each if Number.Mod(_, 17) = 0 then null else \"R\" & Text.From(Number.Mod(_, 5))), \
         List.Transform(n, each Date.AddDays(#date(2024, 1, 1), Number.Mod(_, 40))), \
         List.Transform(n, each if Number.Mod(_, 13) = 0 then null else Number.Mod(_ * 37, 1000) / 10), \
         List.Transform(n, each Number.Mod(_ * 11, 97) / 4), \
         List.Transform(n, each Number.Mod(_, 9) + 1)}}, \
         {{\"Id\", \"Product\", \"Region\", \"Day\", \"Amount\", \"Price\", \"Qty\"}}) in t"
    );
    let product = "let n = List.Numbers(0, 20) in Table.FromColumns({List.Transform(n, each \"P\" & Text.From(_)), \
                   List.Transform(n, each \"C\" & Text.From(Number.Mod(_, 4)))}, {\"Product\", \"Category\"})";
    let mut steps = vec![
        ("model.new", json!({"name": "Grid"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "table.add",
            json!({"name": "Product", "partition": {"source_type": "m", "expression": product}}),
        ),
    ];
    for (t, name, ty) in [
        ("Sales", "Id", "int64"),
        ("Sales", "Product", "string"),
        ("Sales", "Region", "string"),
        ("Sales", "Day", "dateTime"),
        ("Sales", "Amount", "double"),
        ("Sales", "Price", "decimal"),
        ("Sales", "Qty", "int64"),
        ("Product", "Product", "string"),
        ("Product", "Category", "string"),
    ] {
        steps.push(("column.add", json!({"table": t, "name": name, "data_type": ty})));
    }
    for (name, e) in [
        ("Total", "SUM(Sales[Amount])"),
        ("Units", "SUM(Sales[Qty])"),
        ("Lines", "COUNTROWS(Sales)"),
        ("Average", "DIVIDE([Total], [Units])"),
        ("R1", "CALCULATE([Total], Sales[Region] = \"R1\")"),
        ("Share", "DIVIDE([Total], CALCULATE([Total], ALL(Product)))"),
        ("Products", "DISTINCTCOUNT(Sales[Product])"),
        ("Last day", "MAX(Sales[Day])"),
        ("Cost", "SUM(Sales[Price])"),
        (
            "Mixed",
            "VAR t = [Total] RETURN IF(t > 500, t - [Cost] * 2, BLANK())",
        ),
    ] {
        steps.push((
            "measure.add",
            json!({"table": "Sales", "name": name, "expression": e}),
        ));
    }
    steps.extend([
        (
            "relationship.add",
            json!({"from": "Sales[Product]", "to": "Product[Product]"}),
        ),
        ("report.new", json!({})),
    ]);
    for (id, p) in steps {
        run(&mut s, id, p);
    }
    s
}

/// [`model`] with two pages of visuals of every kind the viewer computes: cards, tables with
/// totals and implicit aggregations, matrices with subtotals, charts with series, sorts, top N,
/// measure and page filters, a slicer, items with no data, a value shown as a percentage, and a
/// reader's slicer selection and cross-filtering selection.
fn report(rows: usize) -> Session {
    let mut s = model(rows);
    let first = s
        .report
        .as_ref()
        .map(|r| r.pages[0].name.clone())
        .unwrap_or_default();
    run(
        &mut s,
        "page.rename",
        json!({"page": first, "display_name": "Main"}),
    );
    run(&mut s, "page.add", json!({"display_name": "More"}));
    let by = |cat: &str, val: &str| json!([{"role": "Category", "field": cat}, {"role": "Y", "field": val}]);
    add(
        &mut s,
        "Main",
        "card",
        json!([{"role": "Values", "field": "[Total]"}]),
    );
    add(
        &mut s,
        "Main",
        "card",
        json!([{"role": "Values", "field": "[Products]"}]),
    );
    add(
        &mut s,
        "Main",
        "card",
        json!([{"role": "Values", "field": "[Last day]"}]),
    );
    let slicer = add(
        &mut s,
        "Main",
        "slicer",
        json!([{"role": "Values", "field": "Sales[Region]"}]),
    );
    add(
        &mut s,
        "Main",
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "[Total]"},
               {"role": "Values", "field": "[Units]"}, {"role": "Values", "field": "[Average]"},
               {"role": "Values", "field": "[Cost]"}, {"role": "Values", "field": "[Mixed]"}]),
    );
    add(
        &mut s,
        "Main",
        "tableEx",
        json!([{"role": "Values", "field": "Product[Category]"},
               {"role": "Values", "field": "Sum(Sales[Amount])"}, {"role": "Values", "field": "Avg(Sales[Amount])"},
               {"role": "Values", "field": "Count(Sales[Region])"}, {"role": "Values", "field": "CountNonNull(Sales[Amount])"},
               {"role": "Values", "field": "Min(Sales[Amount])"}, {"role": "Values", "field": "Max(Sales[Day])"},
               {"role": "Values", "field": "Sum(Sales[Price])"}]),
    );
    add(
        &mut s,
        "Main",
        "pivotTable",
        json!([{"role": "Rows", "field": "Product[Category]"}, {"role": "Rows", "field": "Sales[Region]"},
               {"role": "Values", "field": "[Total]"}, {"role": "Values", "field": "[Share]"}]),
    );
    let column = add(
        &mut s,
        "Main",
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Series", "field": "Product[Category]"},
               {"role": "Y", "field": "[Total]"}]),
    );
    let sorted = add(
        &mut s,
        "Main",
        "clusteredBarChart",
        by("Sales[Product]", "[Units]"),
    );
    run(
        &mut s,
        "visual.set_sort",
        json!({"page": "Main", "visual": sorted, "field": "[Units]", "direction": "desc"}),
    );
    add(&mut s, "Main", "lineChart", by("Sales[Day]", "[Lines]"));
    add(
        &mut s,
        "Main",
        "tableEx",
        json!([{"role": "Values", "field": "Product[Category]"}, {"role": "Values", "field": "Sales[Region]"}]),
    );
    add(&mut s, "Main", "pieChart", by("Product[Category]", "[R1]"));
    // page 2: filters, top N, items with no data, a percentage
    let top = add(
        &mut s,
        "More",
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "[Total]"}]),
    );
    run(
        &mut s,
        "visual.filter",
        json!({"page": "More", "visual": top, "field": "Sales[Product]", "top": 5, "by": "[Total]"}),
    );
    let measure = add(
        &mut s,
        "More",
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Region]"}, {"role": "Values", "field": "Product[Category]"},
               {"role": "Values", "field": "[Units]"}]),
    );
    run(
        &mut s,
        "visual.filter",
        json!({"page": "More", "visual": measure, "field": "[Units]", "op": ">", "value": 200}),
    );
    let basic = add(
        &mut s,
        "More",
        "clusteredColumnChart",
        by("Product[Category]", "[Lines]"),
    );
    run(
        &mut s,
        "visual.filter",
        json!({"page": "More", "visual": basic, "field": "Sales[Region]", "in": ["R1", "R3"]}),
    );
    let nodata = add(
        &mut s,
        "More",
        "clusteredBarChart",
        by("Product[Product]", "[R1]"),
    );
    run(
        &mut s,
        "visual.show_items_no_data",
        json!({"page": "More", "visual": nodata, "role": "Category", "show": true}),
    );
    let pct = add(&mut s, "More", "pieChart", by("Sales[Region]", "[Units]"));
    run(
        &mut s,
        "visual.show_value_as",
        json!({"page": "More", "visual": pct, "role": "Y", "index": 0, "as": "percent_of_grand_total"}),
    );
    add(
        &mut s,
        "More",
        "pivotTable",
        json!([{"role": "Rows", "field": "Sales[Region]"}, {"role": "Columns", "field": "Product[Category]"},
               {"role": "Values", "field": "[Units]"}]),
    );
    run(
        &mut s,
        "page.filter",
        json!({"page": "More", "field": "Sales[Qty]", "op": ">=", "value": 3}),
    );
    // a reader's selections on page 1: a slicer value, then a data point of the column chart
    run(
        &mut s,
        "view.interact",
        json!({"action": "slicer_set", "page": "Main", "visual": slicer, "values": ["R0", "R2", "R4"]}),
    );
    run(
        &mut s,
        "view.interact",
        json!({"action": "select", "page": "Main", "visual": column, "point": [{"c": 0, "v": "R2"}]}),
    );
    s
}

/// Snapshots of `s` until every visual has its data (the time budget may leave some for later).
fn settled(s: &Session, limits: &crate::SnapshotLimits) -> Value {
    let mut snap = s.snapshot_with(limits);
    for _ in 0..50 {
        if !crate::watch::data_pending(&snap) {
            break;
        }
        snap = s.snapshot_with(limits);
    }
    snap
}

/// The data of every visual with the old order (row-by-row first) and the new one, each on a
/// fresh session.
fn old_and_new(make: &dyn Fn() -> Session) -> (Visuals, Visuals) {
    let limits = crate::SnapshotLimits::default();
    let old = {
        let s = make();
        super::set_row_by_row_first(true);
        let snap = settled(&s, &limits);
        super::set_row_by_row_first(false);
        all_data(&snap)
    };
    let new = all_data(&settled(&make(), &limits));
    (old, new)
}

/// Whether `a` and `b` are the same JSON, numbers within a relative 1e-12: real-number sums
/// differ in their last digits with the order rows are added in (the storage engine adds them in
/// parallel, with compensated sums: `docs/storage-engine.md`).
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => x == y || (x - y).abs() <= 1e-12 * x.abs().max(y.abs()),
            _ => x == y,
        },
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same(a, b)),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, a)| y.get(k).is_some_and(|b| same(a, b)))
        }
        _ => a == b,
    }
}

fn assert_same(old: &[(String, Value)], new: &[(String, Value)]) {
    assert_eq!(old.len(), new.len());
    let mut ok = 0;
    for ((id, a), (_, b)) in old.iter().zip(new) {
        assert!(same(a, b), "{id}: old\n{a}\nnew\n{b}");
        if a["status"] == "ok" {
            ok += 1;
        }
    }
    // the report computes: the comparison is not of errors
    assert!(
        ok >= old.len().saturating_sub(1),
        "{ok} of {} visuals ok",
        old.len()
    );
}

#[test]
fn a_varied_report_gives_the_same_data_through_the_storage_engine() {
    let (old, new) = old_and_new(&|| report(3000));
    assert_same(&old, &new);
}

#[test]
fn a_varied_report_on_stored_data_gives_the_same_data_through_the_storage_engine() {
    let dir = TempDir::new("engine-grid-stored");
    let path = dir.path().join("grid.pbix");
    {
        let mut s = report(3000);
        run(&mut s, "file.save_as", json!({"path": path, "overwrite": true}));
    }
    let open = || {
        let mut s = Session::new();
        run(&mut s, "file.open", json!({"path": path}));
        s
    };
    let (old, new) = old_and_new(&open);
    assert_same(&old, &new);
}

/// A table over the row-by-row evaluator's cap: its visuals are drawn from the storage engine
/// (before, every visual over it was an error naming the cap).
#[test]
fn a_visual_over_a_table_larger_than_the_row_cap_draws_its_values() {
    let mut s = model(500);
    let first = s
        .report
        .as_ref()
        .map(|r| r.pages[0].name.clone())
        .unwrap_or_default();
    run(
        &mut s,
        "page.rename",
        json!({"page": first, "display_name": "Main"}),
    );
    let card = add(
        &mut s,
        "Main",
        "card",
        json!([{"role": "Values", "field": "[Units]"}]),
    );
    let table = add(
        &mut s,
        "Main",
        "tableEx",
        json!([{"role": "Values", "field": "Product[Category]"}, {"role": "Values", "field": "[Lines]"}]),
    );
    let pairs = add(
        &mut s,
        "Main",
        "tableEx",
        json!([{"role": "Values", "field": "Product[Category]"}, {"role": "Values", "field": "Sales[Region]"}]),
    );
    let capped = crate::SnapshotLimits {
        data_table_rows: 100,
        ..crate::SnapshotLimits::default()
    };
    let snap = settled(&s, &capped);
    let d = find(&snap, &card);
    assert_eq!(d["status"], "ok", "{d}");
    // Qty is 1 to 9 by row number: 55 rows of each full cycle of 9 sum to 45
    let units = d["rows"][0][0].as_f64().unwrap_or_default();
    assert!(units > 500.0, "{d}");
    let t = find(&snap, &table);
    assert_eq!(t["status"], "ok", "{t}");
    let lines: f64 = t["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r[1].as_f64())
        .sum();
    assert_eq!(lines, 500.0, "{t}");
    // columns of two tables only: their combinations come from the row-by-row evaluator, which
    // says why it gives none rather than another list
    let p = find(&snap, &pairs);
    assert_eq!(p["status"], "error", "{p}");
    assert!(
        p["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("several tables"),
        "{p}"
    );
    // the same values as without the cap
    let free = settled(&model_again(&s), &crate::SnapshotLimits::default());
    assert_eq!(find(&free, &card)["rows"], d["rows"]);
}

/// The same session's report on a fresh session (nothing cached).
fn model_again(s: &Session) -> Session {
    let mut t = model(500);
    t.report = s.report.clone();
    t
}

/// `Sales` of 25,000 rows and a table of its 25,000 ids: more grouping combinations than the
/// general evaluator takes for one visual.
fn ids() -> (Session, String) {
    let mut s = model(25_000);
    let first = s
        .report
        .as_ref()
        .map(|r| r.pages[0].name.clone())
        .unwrap_or_default();
    run(
        &mut s,
        "page.rename",
        json!({"page": first, "display_name": "Main"}),
    );
    let v = add(
        &mut s,
        "Main",
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Id]"}, {"role": "Values", "field": "[Units]"}]),
    );
    (s, v)
}

/// A visual the storage engine refuses falls back to the row-by-row evaluator: under the cap it
/// gives the values, over it the reason names the cap instead of showing anything else.
#[test]
fn the_row_cap_applies_only_to_the_row_by_row_fallback() {
    let (s, v) = ids();
    let d = find(&settled(&s, &crate::SnapshotLimits::default()), &v);
    assert_eq!(d["status"], "ok", "{d}");
    assert_eq!(d["total_rows"], 25_000, "{}", d["total_rows"]);

    let (s, v) = ids();
    let capped = crate::SnapshotLimits {
        data_table_rows: 10_000,
        ..crate::SnapshotLimits::default()
    };
    let d = find(&settled(&s, &capped), &v);
    assert_eq!(d["status"], "unsupported", "{d}");
    let reason = d["reason"].as_str().unwrap_or_default();
    assert!(reason.contains("combinations"), "{reason}");
    assert!(reason.contains("10000 rows (preview limit)"), "{reason}");
}
