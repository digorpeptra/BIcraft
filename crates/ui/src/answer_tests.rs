use super::*;

use serde_json::json;

#[test]
fn records_become_a_grid_in_first_seen_column_order() {
    let a = Answer::new(&json!([{"name": "b", "rows": 2}, {"name": "a", "rows": 10, "extra": true}]));
    let g = a.grid().unwrap();
    assert!(matches!(a.root, Node::Grid(0)));
    assert_eq!(g.columns, ["name", "rows", "extra"]);
    assert_eq!(g.rows.len(), 2);
    assert_eq!(g.cell(0, 2), Some(""), "a missing key is an empty cell");
    assert_eq!(g.cell(1, 2), Some("true"));
}

#[test]
fn a_dax_result_uses_its_columns_order() {
    let v = json!({"results": [{"columns": [{"name": "T[B]", "type": "int"}, {"name": "[A]", "type": "int"}],
                                "rows": [{"[A]": 1, "T[B]": "x"}, {"[A]": 2, "T[B]": null}],
                                "row_count": 2, "truncated": false}],
                   "elapsed_ms": 3});
    let a = Answer::new(&v);
    let g = a.grid().unwrap();
    assert_eq!(g.columns, ["T[B]", "[A]"]);
    assert_eq!(g.cell(0, 1), Some("1"));
    assert!(g.rows[1][0].null);
    // the tree keeps the other keys, not `columns`
    let Node::Object(top, _) = &a.root else {
        panic!("{:?}", a.root)
    };
    assert_eq!(top[0].0, "results");
    assert!(
        matches!(&top[0].1, Node::List(items, 0) if items.len() == 1),
        "one table per EVALUATE"
    );
}

#[test]
fn sorting_cycles_ascending_descending_none_numbers_before_text_nulls_last() {
    let mut a = Answer::new(&json!([
        {"v": 10}, {"v": "b"}, {"v": null}, {"v": 2}, {"v": "A"}, {"v": 2.5}
    ]));
    let g = a.grid_mut().unwrap();
    let col = |g: &Grid| {
        (0..g.order.len())
            .map(|r| g.cell(r, 0).unwrap().to_string())
            .collect::<Vec<_>>()
    };
    g.click(0);
    assert_eq!(col(g), ["2", "2.5", "10", "A", "b", ""]);
    g.click(0);
    assert_eq!(col(g), ["b", "A", "10", "2.5", "2", ""]);
    g.click(0);
    assert_eq!(g.sort, None);
    assert_eq!(col(g), ["10", "b", "", "2", "A", "2.5"]);
}

#[test]
fn tsv_follows_the_display_order_and_cleans_cells() {
    let mut a = Answer::new(&json!([{"k": "b\tc", "n": 2}, {"k": "a\nz", "n": 1}]));
    let g = a.grid_mut().unwrap();
    assert_eq!(g.tsv(), "k\tn\nb c\t2\na z\t1");
    g.click(1);
    assert_eq!(g.tsv(), "k\tn\na z\t1\nb c\t2");
}

#[test]
fn objects_lists_and_scalars() {
    let a = Answer::new(
        &json!({"count": 3, "valid": true, "names": ["a", "b"], "nested": {"x": null},
                                "many": (0..20).collect::<Vec<_>>(), "mixed": [1, {"a": 1}], "empty": []}),
    );
    let Node::Object(o, 0) = &a.root else { panic!() };
    let get = |k: &str| &o.iter().find(|(n, _)| n == k).unwrap().1;
    assert!(matches!(get("count"), Node::Value(s, Tone::Number) if s == "3"));
    assert!(matches!(get("valid"), Node::Value(s, Tone::Flag) if s == "true"));
    assert!(matches!(get("names"), Node::Value(s, Tone::Text) if s == "a, b"));
    assert!(matches!(get("nested"), Node::Object(..)));
    assert!(
        matches!(get("many"), Node::Grid(_)),
        "a long list of values is a one-column grid"
    );
    assert!(matches!(get("mixed"), Node::List(items, 0) if items.len() == 2));
    assert!(matches!(get("empty"), Node::Value(s, Tone::Null) if s == "[]"));
    assert!(Answer::new(&Value::Null).is_empty());
    assert!(Answer::new(&json!({})).is_empty());
    assert!(!Answer::new(&json!("text")).is_empty());
}

#[test]
fn hostile_answers_are_capped() {
    // deep nesting
    let mut v = json!(1);
    for _ in 0..500 {
        v = json!({ "a": v });
    }
    let _ = Answer::new(&v);
    // too many rows and columns
    let wide: Map<String, Value> = (0..100).map(|i| (format!("c{i}"), json!(i))).collect();
    let rows: Vec<Value> = (0..MAX_ROWS + 5).map(|_| Value::Object(wide.clone())).collect();
    let a = Answer::new(&Value::Array(rows));
    let g = a.grid().unwrap();
    assert_eq!(g.rows.len(), MAX_ROWS);
    assert_eq!(g.rows_left_out, 5);
    assert_eq!(g.columns.len(), 64);
    assert_eq!(g.columns_left_out, 36);
    // a long text is clipped
    let a = Answer::new(&json!([{"t": "x".repeat(10_000)}]));
    assert!(a.grid().unwrap().rows[0][0].text.chars().count() <= MAX_CELL);
}

#[test]
fn counts_are_grouped() {
    assert_eq!(count(100_000, Lang::En), "100,000");
    assert_eq!(count(1_234_567, Lang::Fr), "1 234 567");
    assert_eq!(count(12, Lang::En), "12");
}
