//! Applied steps (`steps.rs`) and the transforms' M (`transform.rs`): the split keeps the user's
//! text, edits touch only the step changed, and each transform's step evaluates to the table the
//! M function reference describes.

use super::steps::{self, at_step, insert_step, remove_step, rename_step, set_step};
use super::transform::{Aggregate, ColumnType, FilterOp, JoinKind, Literal, Transform};
use super::*;
use CellValue::{Int, Null, Real, Text};

fn lim() -> Limits {
    Limits::default()
}

/// A query with comments, odd spacing and a quoted step name.
const Q: &str = "let\r\n    // the rows\r\n    Source = #table({\"Name\", \"Qty\", \"City\"}, {{\"a\", 1, \"Paris\"}, {\"b\", 5, \"Lyon\"}, {\"c\", 3, \"Paris\"}}),\r\n    #\"Changed Type\" = Table.TransformColumnTypes(Source,{{\"Qty\", Int64.Type}}) /* typed */,\r\n    Big   =   Table.SelectRows(#\"Changed Type\", each [Qty] > 2)\r\nin\r\n    Big";

fn load(m: &str) -> TableData {
    load_m(m, &lim()).unwrap_or_else(|e| panic!("{e}\n{m}"))
}

fn col(t: &TableData, name: &str) -> Vec<CellValue> {
    t.columns()
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no column {name}"))
        .values
        .clone()
}

fn names(t: &TableData) -> Vec<String> {
    t.columns().iter().map(|c| c.name.clone()).collect()
}

fn s(x: &str) -> CellValue {
    Text(x.into())
}

#[test]
fn steps_split_the_let_as_written() {
    let st = steps::steps(Q, &lim()).unwrap();
    assert!(st.is_let);
    let n: Vec<&str> = st.steps.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(n, ["Source", "Changed Type", "Big"]);
    assert_eq!(
        st.steps[1].expression,
        "Table.TransformColumnTypes(Source,{{\"Qty\", Int64.Type}})"
    );
    assert_eq!(
        st.steps[1].function.as_deref(),
        Some("Table.TransformColumnTypes")
    );
    assert_eq!(
        st.steps[2].expression,
        "Table.SelectRows(#\"Changed Type\", each [Qty] > 2)"
    );
    assert_eq!(&Q[st.steps[1].name_span.clone()], "#\"Changed Type\"");
    assert_eq!(st.result, "Big");
    assert_eq!(st.result_step, Some(2));
    assert_eq!(st.last().map(|x| x.name.as_str()), Some("Big"));
}

#[test]
fn nested_lets_records_and_lists_stay_in_their_step() {
    let q = "let A = let x = 1, y = [a = 2, b = {3, 4}] in x + y[a], B = A * 2 in B";
    let st = steps::steps(q, &lim()).unwrap();
    assert_eq!(st.steps.len(), 2);
    assert_eq!(
        st.steps[0].expression,
        "let x = 1, y = [a = 2, b = {3, 4}] in x + y[a]"
    );
    assert_eq!(st.steps[1].expression, "A * 2");
}

#[test]
fn a_query_that_is_not_a_let_is_one_source_step() {
    let q = "#table({\"A\"}, {{1}, {2}})";
    let st = steps::steps(q, &lim()).unwrap();
    assert!(!st.is_let);
    assert_eq!(st.steps.len(), 1);
    assert_eq!(st.steps[0].name, "Source");
    // the first step added wraps it
    let out = insert_step(q, None, "Kept", "Table.FirstN(Source, 1)", &lim()).unwrap();
    assert_eq!(
        out,
        "let\n    Source = #table({\"A\"}, {{1}, {2}}),\n    Kept = Table.FirstN(Source, 1)\nin\n    Kept"
    );
    assert_eq!(col(&load(&out), "A"), vec![Int(1)]);
}

#[test]
fn bad_text_is_refused() {
    for q in ["let A = in A", "let A = 1,", "let A 1 in A", "", "let in 1"] {
        assert!(steps::steps(q, &lim()).is_err(), "{q}");
    }
}

#[test]
fn at_step_evaluates_the_steps_up_to_it() {
    let m = at_step(Q, "Changed Type", &lim()).unwrap();
    assert!(m.ends_with("/* typed */\r\nin\r\n    #\"Changed Type\""), "{m}");
    assert_eq!(load(&m).row_count(), 3);
    assert_eq!(load(&at_step(Q, "Big", &lim()).unwrap()).row_count(), 2);
    assert!(at_step(Q, "Nope", &lim()).is_err());
    // case is forgiven when one step matches
    assert_eq!(load(&at_step(Q, "big", &lim()).unwrap()).row_count(), 2);
}

#[test]
fn insert_after_the_last_step_keeps_the_rest_of_the_text() {
    let out = insert_step(
        Q,
        None,
        "Sorted Rows",
        "Table.Sort(Big,{{\"Qty\", Order.Descending}})",
        &lim(),
    )
    .unwrap();
    let expected = Q.replace(
        "each [Qty] > 2)\r\nin\r\n    Big",
        "each [Qty] > 2),\r\n    #\"Sorted Rows\" = Table.Sort(Big,{{\"Qty\", Order.Descending}})\r\nin\r\n    #\"Sorted Rows\"",
    );
    assert_eq!(out, expected);
    assert_eq!(col(&load(&out), "Qty"), vec![Int(5), Int(3)]);
}

#[test]
fn insert_in_the_middle_rewires_the_next_step() {
    let out = insert_step(
        Q,
        Some("Changed Type"),
        "Paris",
        "Table.SelectRows(#\"Changed Type\", each [City] = \"Paris\")",
        &lim(),
    )
    .unwrap();
    assert!(out.contains(
        "/* typed */,\r\n    Paris = Table.SelectRows(#\"Changed Type\", each [City] = \"Paris\"),"
    ));
    assert!(
        out.contains("Big   =   Table.SelectRows(Paris, each [Qty] > 2)"),
        "{out}"
    );
    assert!(out.contains("// the rows"));
    assert_eq!(col(&load(&out), "Name"), vec![s("c")]);
    // a name already taken is refused
    assert!(insert_step(Q, None, "Big", "Big", &lim()).is_err());
}

#[test]
fn remove_a_step_and_its_users_read_the_one_before() {
    let out = remove_step(Q, "Changed Type", &lim()).unwrap();
    assert_eq!(
        out,
        "let\r\n    // the rows\r\n    Source = #table({\"Name\", \"Qty\", \"City\"}, {{\"a\", 1, \"Paris\"}, {\"b\", 5, \"Lyon\"}, {\"c\", 3, \"Paris\"}}),\r\n    Big   =   Table.SelectRows(Source, each [Qty] > 2)\r\nin\r\n    Big"
    );
    assert_eq!(load(&out).row_count(), 2);
    // the last step: the result names the step before
    let out = remove_step(Q, "Big", &lim()).unwrap();
    assert!(
        out.ends_with("/* typed */\r\nin\r\n    #\"Changed Type\""),
        "{out}"
    );
    assert_eq!(load(&out).row_count(), 3);
    // the first step, used by the next one
    assert!(remove_step(Q, "Source", &lim()).is_err());
    // an unused first step goes
    let q = "let A = 1, B = 2 in B";
    assert_eq!(remove_step(q, "A", &lim()).unwrap(), "let B = 2 in B");
    assert!(remove_step("let A = 1 in A", "A", &lim()).is_err());
}

#[test]
fn rename_follows_the_references() {
    let out = rename_step(Q, "Changed Type", "Typed", &lim()).unwrap();
    assert!(out.contains("    Typed = Table.TransformColumnTypes(Source,"));
    assert!(out.contains("Table.SelectRows(Typed, each [Qty] > 2)"));
    let out = rename_step(Q, "Big", "Big ones", &lim()).unwrap();
    assert!(out.ends_with("in\r\n    #\"Big ones\""), "{out}");
    assert!(rename_step(Q, "Big", "Source", &lim()).is_err());
    assert!(rename_step(Q, "Big", " ", &lim()).is_err());
    assert_eq!(load(&out).row_count(), 2);
}

#[test]
fn set_step_replaces_only_its_expression() {
    let out = set_step(
        Q,
        "Big",
        "= Table.SelectRows(#\"Changed Type\", each [Qty] > 4)",
        &lim(),
    )
    .unwrap();
    assert_eq!(out, Q.replace("each [Qty] > 2)", "each [Qty] > 4)"));
    assert_eq!(load(&out).row_count(), 1);
    assert!(set_step(Q, "Big", "Table.SelectRows(", &lim()).is_err());
    assert!(set_step(Q, "Big", "  ", &lim()).is_err());
    assert_eq!(
        set_step(
            Q,
            "Big",
            "Table.SelectRows(#\"Changed Type\", each [Qty] > 2)",
            &lim()
        )
        .unwrap(),
        Q
    );
}

#[test]
fn free_names_count_up_as_the_editor_does() {
    let st = steps::steps(Q, &lim()).unwrap();
    assert_eq!(st.free_name("Changed Type"), "Changed Type1");
    assert_eq!(st.free_name("Sorted Rows"), "Sorted Rows");
}

/// Applies `t` after the last step of `q` and loads the result.
fn apply(q: &str, t: &Transform) -> TableData {
    let st = steps::steps(q, &lim()).unwrap();
    let prev = st.last().unwrap().name.clone();
    let m = t.m(&prev).unwrap();
    let out = insert_step(q, None, &st.free_name(&t.step_name()), &m, &lim()).unwrap();
    load(&out)
}

const T: &str = "let\n    Source = #table(type table [Name = text, Qty = Int64.Type, City = text, Tags = text], {{\"a\", 1, \"Paris\", \"x,y\"}, {\"b\", 5, \"Lyon\", \"z\"}, {\"c\", 3, \"Paris\", null}, {\"a\", 1, \"Paris\", \"x,y\"}})\nin\n    Source";

#[test]
fn filter_rows() {
    let f = |op, v| Transform::FilterRows {
        column: "City".into(),
        op,
        value: v,
    };
    assert_eq!(
        apply(T, &f(FilterOp::Equals, Literal::Text("Paris".into()))).row_count(),
        3
    );
    assert_eq!(
        apply(T, &f(FilterOp::NotEquals, Literal::Text("Paris".into()))).row_count(),
        1
    );
    assert_eq!(
        apply(T, &f(FilterOp::BeginsWith, Literal::Text("Ly".into()))).row_count(),
        1
    );
    assert_eq!(
        apply(T, &f(FilterOp::Contains, Literal::Text("ar".into()))).row_count(),
        3
    );
    assert_eq!(
        apply(T, &f(FilterOp::NotContains, Literal::Text("ar".into()))).row_count(),
        1
    );
    assert_eq!(
        apply(T, &f(FilterOp::EndsWith, Literal::Text("on".into()))).row_count(),
        1
    );
    let q = |op, n| Transform::FilterRows {
        column: "Qty".into(),
        op,
        value: Literal::Number(n),
    };
    assert_eq!(apply(T, &q(FilterOp::Greater, 1.0)).row_count(), 2);
    assert_eq!(apply(T, &q(FilterOp::GreaterOrEqual, 3.0)).row_count(), 2);
    assert_eq!(apply(T, &q(FilterOp::Less, 3.0)).row_count(), 2);
    assert_eq!(apply(T, &q(FilterOp::LessOrEqual, 3.0)).row_count(), 3);
    let n = Transform::FilterRows {
        column: "Tags".into(),
        op: FilterOp::IsNull,
        value: Literal::Null,
    };
    assert_eq!(apply(T, &n).row_count(), 1);
}

/// The value list of a column's filter menu: kept values joined by `or`, left-out ones by `and`.
#[test]
fn filter_values() {
    let f = |values: Vec<Literal>, exclude| Transform::FilterValues {
        column: "City".into(),
        values,
        exclude,
    };
    let kept = f(
        vec![Literal::Text("Paris".into()), Literal::Text("Lyon".into())],
        false,
    );
    assert_eq!(
        kept.m("Source").unwrap(),
        "Table.SelectRows(Source, each ([City] = \"Paris\" or [City] = \"Lyon\"))"
    );
    assert_eq!(kept.step_name(), "Filtered Rows");
    assert_eq!(kept.kind(), "filter_values");
    assert_eq!(apply(T, &kept).row_count(), 4);
    let out = f(vec![Literal::Text("Lyon".into())], true);
    assert_eq!(
        out.m("Source").unwrap(),
        "Table.SelectRows(Source, each ([City] <> \"Lyon\"))"
    );
    assert_eq!(apply(T, &out).row_count(), 3);
    let nulls = Transform::FilterValues {
        column: "Tags".into(),
        values: vec![Literal::Null, Literal::Text("z".into())],
        exclude: false,
    };
    assert_eq!(apply(T, &nulls).row_count(), 2);
    let qty = Transform::FilterValues {
        column: "Qty".into(),
        values: vec![Literal::Number(1.0)],
        exclude: true,
    };
    assert_eq!(apply(T, &qty).row_count(), 2);
    assert!(f(Vec::new(), false).m("Source").is_err());
    let unnamed = Transform::FilterValues {
        column: String::new(),
        values: vec![Literal::Null],
        exclude: false,
    };
    assert!(unnamed.m("Source").is_err());
}

#[test]
fn columns_remove_keep_rename_type() {
    let t = apply(T, &Transform::RemoveColumns(vec!["Tags".into(), "City".into()]));
    assert_eq!(names(&t), ["Name", "Qty"]);
    let t = apply(T, &Transform::KeepColumns(vec!["City".into(), "Name".into()]));
    assert_eq!(names(&t), ["City", "Name"]);
    let t = apply(
        T,
        &Transform::RenameColumns(vec![("Qty".into(), "Quantity \"n\"".into())]),
    );
    assert_eq!(names(&t), ["Name", "Quantity \"n\"", "City", "Tags"]);
    let t = apply(
        T,
        &Transform::ChangeType(vec![
            ("Qty".into(), ColumnType::Text),
            ("Name".into(), ColumnType::Text),
        ]),
    );
    assert_eq!(col(&t, "Qty")[1], s("5"));
    assert_eq!(t.columns()[1].data_type, DataType::String);
    for ty in ColumnType::ALL {
        assert!(
            Transform::ChangeType(vec![("Qty".into(), ty)])
                .m("Source")
                .is_ok()
        );
    }
}

#[test]
fn promote_headers_sort_distinct_replace() {
    let raw = "let Source = #table({\"Column1\", \"Column2\"}, {{\"A\", \"B\"}, {\"x\", \"2\"}, {\"y\", \"1\"}}) in Source";
    let t = apply(raw, &Transform::PromoteHeaders);
    assert_eq!(names(&t), ["A", "B"]);
    assert_eq!(t.row_count(), 2);
    let t = apply(
        T,
        &Transform::Sort(vec![("Qty".into(), true), ("Name".into(), false)]),
    );
    assert_eq!(col(&t, "Qty"), vec![Int(5), Int(3), Int(1), Int(1)]);
    assert_eq!(apply(T, &Transform::RemoveDuplicates(vec![])).row_count(), 3);
    assert_eq!(
        apply(T, &Transform::RemoveDuplicates(vec!["City".into()])).row_count(),
        2
    );
    let t = apply(
        T,
        &Transform::ReplaceValues {
            columns: vec!["City".into()],
            find: Literal::Text("Par".into()),
            replace: Literal::Text("Mar".into()),
        },
    );
    assert_eq!(col(&t, "City")[0], s("Maris"));
    let t = apply(
        T,
        &Transform::ReplaceValues {
            columns: vec!["Qty".into()],
            find: Literal::Number(1.0),
            replace: Literal::Number(-7.0),
        },
    );
    assert_eq!(col(&t, "Qty")[0], Int(-7));
}

#[test]
fn group_by() {
    let t = apply(
        T,
        &Transform::GroupBy {
            keys: vec!["City".into()],
            aggregations: vec![
                ("Count".into(), Aggregate::Count, None),
                ("Total".into(), Aggregate::Sum, Some("Qty".into())),
                ("Most".into(), Aggregate::Max, Some("Qty".into())),
            ],
        },
    );
    assert_eq!(names(&t), ["City", "Count", "Total", "Most"]);
    assert_eq!(col(&t, "City"), vec![s("Paris"), s("Lyon")]);
    assert_eq!(col(&t, "Count"), vec![Int(3), Int(1)]);
    assert_eq!(col(&t, "Total"), vec![Real(5.0), Real(5.0)]);
    let bad = Transform::GroupBy {
        keys: vec!["City".into()],
        aggregations: vec![("S".into(), Aggregate::Sum, None)],
    };
    assert!(bad.m("Source").is_err());
}

/// `T` and a second query `Cities` as shared expressions.
fn with_cities(main: &str) -> TableData {
    let opts = SourceOptions {
        shared: vec![
            (
                "Cities".into(),
                "#table(type table [City = text, Country = text], {{\"Paris\", \"FR\"}, {\"Lyon\", \"FR\"}})"
                    .into(),
            ),
            (
                "More".into(),
                "#table(type table [Name = text, Qty = Int64.Type, City = text, Tags = text], {{\"d\", 9, \"Nice\", null}})"
                    .into(),
            ),
        ],
        ..SourceOptions::default()
    };
    load_m_with(main, &opts).unwrap_or_else(|e| panic!("{e}\n{main}"))
}

fn add(q: &str, t: &Transform) -> String {
    let st = steps::steps(q, &lim()).unwrap();
    let prev = st.last().unwrap().name.clone();
    insert_step(
        q,
        None,
        &st.free_name(&t.step_name()),
        &t.m(&prev).unwrap(),
        &lim(),
    )
    .unwrap()
}

#[test]
fn merge_expand_and_append() {
    let merged = add(
        T,
        &Transform::Merge {
            other: "Cities".into(),
            keys: vec!["City".into()],
            other_keys: vec!["City".into()],
            kind: JoinKind::LeftOuter,
            column: "Cities".into(),
        },
    );
    assert!(merged.contains("#\"Merged Queries\" = Table.NestedJoin(Source, {\"City\"}, Cities, {\"City\"}, \"Cities\", JoinKind.LeftOuter)"));
    let expanded = add(
        &merged,
        &Transform::Expand {
            column: "Cities".into(),
            columns: vec!["Country".into()],
            prefix: true,
        },
    );
    let t = with_cities(&expanded);
    assert_eq!(names(&t), ["Name", "Qty", "City", "Tags", "Cities.Country"]);
    assert_eq!(
        col(&t, "Cities.Country"),
        vec![s("FR"), s("FR"), s("FR"), s("FR")]
    );
    let appended = add(T, &Transform::Append(vec!["More".into()]));
    let t = with_cities(&appended);
    assert_eq!(t.row_count(), 5);
    assert_eq!(col(&t, "Qty")[4], Int(9));
    assert!(Transform::Append(vec![]).m("Source").is_err());
}

#[test]
fn custom_column_split_unpivot_pivot() {
    let t = apply(
        T,
        &Transform::AddColumn {
            name: "Double".into(),
            formula: "= [Qty] * 2".into(),
        },
    );
    assert_eq!(col(&t, "Double")[1], Int(10));
    let t = apply(
        T,
        &Transform::SplitColumn {
            column: "Tags".into(),
            delimiter: ",".into(),
            parts: 2,
        },
    );
    assert_eq!(col(&t, "Tags.1")[0], s("x"));
    assert_eq!(col(&t, "Tags.2")[0], s("y"));
    assert_eq!(col(&t, "Tags.2")[1], Null);
    let small = "let Source = #table(type table [Id = Int64.Type, A = Int64.Type, B = Int64.Type], {{1, 10, 20}, {2, 30, 40}}) in Source";
    let t = apply(
        small,
        &Transform::Unpivot {
            columns: vec!["Id".into()],
            other: true,
        },
    );
    assert_eq!(names(&t), ["Id", "Attribute", "Value"]);
    assert_eq!(t.row_count(), 4);
    let t = apply(
        small,
        &Transform::Unpivot {
            columns: vec!["A".into()],
            other: false,
        },
    );
    assert_eq!(names(&t), ["Id", "B", "Attribute", "Value"]);
    let long = "let Source = #table(type table [Id = Int64.Type, K = text, V = Int64.Type], {{1, \"a\", 10}, {1, \"b\", 20}, {2, \"a\", 30}, {1, \"a\", 5}}) in Source";
    let t = apply(
        long,
        &Transform::Pivot {
            column: "K".into(),
            values: "V".into(),
            aggregate: Some(Aggregate::Sum),
        },
    );
    assert_eq!(names(&t), ["Id", "a", "b"]);
    assert_eq!(col(&t, "a")[0], Int(15));
    let t = apply(T, &Transform::Custom);
    assert_eq!(t.row_count(), 4);
}

#[test]
fn literals_and_fields_are_escaped() {
    assert_eq!(Literal::Text("a\"b\n#(c)".into()).m(), "\"a\"\"b#(lf)#(#)(c)\"");
    assert_eq!(super::transform::field("My Col"), "[My Col]");
    assert_eq!(super::transform::field("a]b"), "[#\"a]b\"]");
    let q = "let Source = #table({\"a]b\", \"#(#)(x)\"}, {{1, \"#(#)(x)\"}, {2, \"y\"}}) in Source";
    let t = apply(
        q,
        &Transform::FilterRows {
            column: "a]b".into(),
            op: FilterOp::Equals,
            value: Literal::Number(2.0),
        },
    );
    assert_eq!(t.row_count(), 1);
    let t = apply(
        q,
        &Transform::FilterRows {
            column: "#(x)".into(),
            op: FilterOp::Equals,
            value: Literal::Text("#(x)".into()),
        },
    );
    assert_eq!(t.row_count(), 1);
}
