//! The transforms of `transform_more.rs`: each step, added after a synthetic `#table`, evaluates to
//! the table the M function reference describes.

use super::steps::{self, insert_step};
use super::transform::{FilterOp, Literal, Transform};
use super::transform_more::{COLUMN_OPS, ColumnOp, More, OpArgs, Rule};
use super::*;
use CellValue::{Int, Null, Real, Text};

fn lim() -> Limits {
    Limits::default()
}

fn s(x: &str) -> CellValue {
    Text(x.into())
}

fn col(t: &TableData, name: &str) -> Vec<CellValue> {
    t.columns()
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no column {name}: {:?}", names(t)))
        .values
        .clone()
}

fn names(t: &TableData) -> Vec<String> {
    t.columns().iter().map(|c| c.name.clone()).collect()
}

/// The M of `x` added after the last step of `q`.
fn add(q: &str, x: More) -> String {
    let t = Transform::More(x);
    let st = steps::steps(q, &lim()).unwrap();
    let prev = st.last().unwrap().name.clone();
    let m = t.m(&prev).unwrap();
    insert_step(q, None, &st.free_name(&t.step_name()), &m, &lim()).unwrap()
}

fn apply(q: &str, x: More) -> TableData {
    let m = add(q, x);
    load_m(&m, &lim()).unwrap_or_else(|e| panic!("{e}\n{m}"))
}

const T: &str = "let\n    Source = #table(type table [Name = text, Qty = Int64.Type, City = text, Price = number, Day = date], {{\" ann \", 1, \"Paris\", 1.256, #date(2024, 1, 31)}, {\"BOB\", null, null, -2.5, #date(2024, 5, 4)}, {\"carl\", 3, \"Lyon\", 10, #date(2023, 12, 25)}, {\"dee\", null, null, 4, #date(2024, 7, 1)}, {\"eve\", 5, \"Nice\", 0.5, #date(2024, 2, 29)}})\nin\n    Source";

#[test]
fn remove_and_keep_rows() {
    let n = |x| apply(T, x).row_count();
    assert_eq!(n(More::RemoveTopRows(2)), 3);
    assert_eq!(col(&apply(T, More::RemoveTopRows(2)), "Qty")[0], Int(3));
    assert_eq!(n(More::RemoveBottomRows(1)), 4);
    assert_eq!(n(More::KeepTopRows(2)), 2);
    assert_eq!(col(&apply(T, More::KeepBottomRows(2)), "Qty"), [Null, Int(5)]);
    let t = apply(T, More::KeepRange { first: 2, count: 2 });
    assert_eq!(col(&t, "Name"), [s("BOB"), s("carl")]);
    // from row 2: remove 1, keep 1 → rows 1, 3, 5
    let t = apply(
        T,
        More::RemoveAlternateRows {
            first: 2,
            remove: 1,
            keep: 1,
        },
    );
    assert_eq!(col(&t, "Qty"), [Int(1), Int(3), Int(5)]);
    assert!(More::RemoveTopRows(0).m("Source").is_err());
    assert!(More::KeepRange { first: 0, count: 1 }.m("Source").is_err());
}

#[test]
fn blank_rows_errors_and_fill() {
    let q =
        "let Source = #table({\"A\", \"B\"}, {{1, \"x\"}, {null, \"\"}, {null, null}, {4, null}}) in Source";
    let t = apply(q, More::RemoveBlankRows);
    assert_eq!(col(&t, "A"), [Int(1), Int(4)]);
    let t = apply(q, More::FillDown(vec!["A".into()]));
    assert_eq!(col(&t, "A"), [Int(1), Int(1), Int(1), Int(4)]);
    let t = apply(q, More::FillUp(vec!["A".into()]));
    assert_eq!(col(&t, "A"), [Int(1), Int(4), Int(4), Int(4)]);
    let e = "let Source = #table({\"A\", \"B\"}, {{1, 2}, {3, 4}}), Typed = Table.TransformColumns(Source, {{\"A\", each if _ = 3 then error \"bad\" else _}}) in Typed";
    assert_eq!(apply(e, More::RemoveErrors(vec!["A".into()])).row_count(), 1);
    assert_eq!(apply(e, More::RemoveErrors(vec![])).row_count(), 1);
    assert_eq!(apply(e, More::KeepErrors(vec!["A".into()])).row_count(), 1);
    let t = apply(
        e,
        More::ReplaceErrors {
            columns: vec!["A".into()],
            value: Literal::Number(0.0),
        },
    );
    assert_eq!(col(&t, "A"), [Int(1), Int(0)]);
}

#[test]
fn table_shapes() {
    let q = "let Source = #table({\"A\", \"B\"}, {{1, \"x\"}, {2, \"y\"}}) in Source";
    assert_eq!(col(&apply(q, More::ReverseRows), "A"), [Int(2), Int(1)]);
    let t = apply(q, More::Transpose);
    assert_eq!(t.row_count(), 2);
    assert_eq!(names(&t), ["Column1", "Column2"]);
    let t = apply(q, More::DemoteHeaders);
    assert_eq!(t.row_count(), 3);
    assert_eq!(col(&t, "Column1")[0], s("A"));
    let t = apply(q, More::ReorderColumns(vec!["B".into(), "A".into()]));
    assert_eq!(names(&t), ["B", "A"]);
    let t = apply(
        q,
        More::DuplicateColumn {
            column: "B".into(),
            name: "B - Copy".into(),
        },
    );
    assert_eq!(names(&t), ["A", "B", "B - Copy"]);
    assert_eq!(col(&t, "B - Copy"), [s("x"), s("y")]);
    let t = apply(
        q,
        More::AddIndex {
            name: "Index".into(),
            start: 1,
            step: 1,
        },
    );
    assert_eq!(col(&t, "Index"), [Int(1), Int(2)]);
}

#[test]
fn merge_and_conditional_columns() {
    let t = apply(
        T,
        More::MergeColumns {
            columns: vec!["City".into(), "Qty".into()],
            delimiter: "-".into(),
            name: "Merged".into(),
            add: false,
        },
    );
    assert_eq!(names(&t), ["Name", "Merged", "Price", "Day"]);
    assert_eq!(col(&t, "Merged")[0], s("Paris-1"));
    let t = apply(
        T,
        More::MergeColumns {
            columns: vec!["Name".into(), "Qty".into()],
            delimiter: "/".into(),
            name: "Both".into(),
            add: true,
        },
    );
    assert_eq!(col(&t, "Both")[2], s("carl/3"));
    assert_eq!(col(&t, "Both")[1], s("BOB"), "null parts are left out");
    let t = apply(
        T,
        More::ConditionalColumn {
            name: "Size".into(),
            rules: vec![
                Rule {
                    column: "Qty".into(),
                    op: FilterOp::Greater,
                    value: Literal::Number(2.0),
                    output: Literal::Text("big".into()),
                },
                Rule {
                    column: "City".into(),
                    op: FilterOp::Equals,
                    value: Literal::Text("Paris".into()),
                    output: Literal::Text("paris".into()),
                },
            ],
            otherwise: Literal::Null,
        },
    );
    assert_eq!(col(&t, "Size"), [s("paris"), Null, s("big"), Null, s("big")]);
}

fn op(name: &str, a: OpArgs) -> ColumnOp {
    ColumnOp::from_name(name, &a).unwrap()
}

fn in_place(name: &str, a: OpArgs, column: &str) -> Vec<CellValue> {
    let t = apply(
        T,
        More::ColumnOp {
            op: op(name, a),
            columns: vec![column.into()],
            add: false,
            name: None,
        },
    );
    col(&t, column)
}

fn added(name: &str, a: OpArgs, column: &str) -> (Vec<String>, Vec<CellValue>) {
    let o = op(name, a);
    let new = o.column_name();
    let t = apply(
        T,
        More::ColumnOp {
            op: o,
            columns: vec![column.into()],
            add: true,
            name: None,
        },
    );
    (names(&t), col(&t, new))
}

#[test]
fn text_operations() {
    let none = OpArgs::default;
    let text = |t: &str| OpArgs {
        text: Some(t.into()),
        ..OpArgs::default()
    };
    let count = |n| OpArgs {
        count: Some(n),
        ..OpArgs::default()
    };
    assert_eq!(in_place("lower", none(), "Name")[1], s("bob"));
    assert_eq!(in_place("upper", none(), "Name")[2], s("CARL"));
    assert_eq!(in_place("proper", none(), "Name")[2], s("Carl"));
    assert_eq!(in_place("trim", none(), "Name")[0], s("ann"));
    assert_eq!(in_place("prefix", text("x-"), "Name")[1], s("x-BOB"));
    assert_eq!(in_place("suffix", text("!"), "Name")[1], s("BOB!"));
    assert_eq!(in_place("length", none(), "Name")[0], Int(5));
    assert_eq!(in_place("first_chars", count(2), "City")[0], s("Pa"));
    assert_eq!(in_place("first_chars", count(2), "City")[1], Null);
    assert_eq!(in_place("last_chars", count(2), "City")[2], s("on"));
    let range = OpArgs {
        start: Some(1),
        count: Some(2),
        ..OpArgs::default()
    };
    assert_eq!(in_place("text_range", range, "City")[0], s("ar"));
    assert_eq!(in_place("before_delimiter", text("r"), "City")[0], s("Pa"));
    assert_eq!(in_place("after_delimiter", text("r"), "City")[0], s("is"));
    let (n, v) = added("upper", none(), "City");
    assert_eq!(n.last().map(String::as_str), Some("UPPERCASE"));
    assert_eq!(v[2], s("LYON"));
    assert!(ColumnOp::from_name("prefix", &none()).is_err());
    assert!(ColumnOp::from_name("nope", &none()).is_err());
}

#[test]
fn number_operations() {
    let none = OpArgs::default;
    let number = |n| OpArgs {
        number: Some(n),
        ..OpArgs::default()
    };
    let digits = |n| OpArgs {
        count: Some(n),
        ..OpArgs::default()
    };
    assert_eq!(in_place("round", digits(2), "Price")[0], Real(1.26));
    assert_eq!(in_place("round_up", none(), "Price")[0], Int(2));
    assert_eq!(in_place("round_down", none(), "Price")[1], Int(-3));
    assert_eq!(in_place("abs", none(), "Price")[1], Real(2.5));
    assert_eq!(in_place("add", number(1.0), "Qty")[0], Real(2.0));
    assert_eq!(in_place("subtract", number(1.0), "Qty")[2], Real(2.0));
    assert_eq!(in_place("multiply", number(2.0), "Price")[1], Real(-5.0));
    assert_eq!(in_place("divide", number(2.0), "Price")[2], Real(5.0));
    assert!(ColumnOp::from_name("divide", &number(0.0)).is_err());
    let (_, v) = added("multiply", number(10.0), "Qty");
    assert_eq!(v[2], Real(30.0));
}

#[test]
fn date_operations() {
    let none = OpArgs::default;
    assert_eq!(in_place("year", none(), "Day")[2], Int(2023));
    assert_eq!(in_place("month", none(), "Day")[1], Int(5));
    assert_eq!(in_place("day", none(), "Day")[0], Int(31));
    assert_eq!(in_place("quarter", none(), "Day")[3], Int(3));
    assert_eq!(in_place("month_name", none(), "Day")[0], s("January"));
    assert_eq!(in_place("day_name", none(), "Day")[1], s("Saturday"));
    assert_eq!(in_place("day_of_year", none(), "Day")[4], Int(60));
    let (n, v) = added("year", none(), "Day");
    assert_eq!(n.last().map(String::as_str), Some("Year"));
    assert_eq!(v[0], Int(2024));
    let start = in_place("start_of_month", none(), "Day");
    let first = load_m("#table({\"D\"}, {{#date(2024, 1, 1)}})", &lim()).unwrap();
    assert_eq!(start[0], col(&first, "D")[0]);
    let dt =
        "let Source = #table(type table [At = datetime], {{#datetime(2024, 3, 9, 14, 25, 30)}}) in Source";
    let one = |o: &str| {
        let t = apply(
            dt,
            More::ColumnOp {
                op: op(o, none()),
                columns: vec!["At".into()],
                add: false,
                name: None,
            },
        );
        col(&t, "At")[0].clone()
    };
    assert_eq!(one("hour"), Int(14));
    assert_eq!(one("minute"), Int(25));
    assert_eq!(one("second"), Real(30.0));
    assert_eq!(one("year"), Int(2024));
    let d = load_m("#table({\"D\"}, {{#date(2024, 3, 9)}})", &lim()).unwrap();
    assert_eq!(one("date_only"), col(&d, "D")[0]);
}

/// Every operation name builds, writes M that parses, and names its step.
#[test]
fn every_operation_writes_m() {
    let a = OpArgs {
        text: Some(",".into()),
        end: Some(";".into()),
        count: Some(2),
        start: Some(0),
        number: Some(3.0),
    };
    for (name, _) in COLUMN_OPS {
        let o = ColumnOp::from_name(name, &a).unwrap();
        assert_eq!(o.name(), name);
        for add in [false, true] {
            let x = More::ColumnOp {
                op: o.clone(),
                columns: vec!["Name".into()],
                add,
                name: None,
            };
            let m = add_m(x);
            assert!(steps::steps(&m, &lim()).is_ok(), "{name}: {m}");
        }
    }
}

fn add_m(x: More) -> String {
    add(T, x)
}

#[test]
fn dates_in_filters() {
    let t = load_m(&add_filter(Literal::Date(2024, 5, 4)), &lim()).unwrap();
    assert_eq!(t.row_count(), 1);
    let q = "let Source = #table(type table [At = datetime], {{#datetime(2024, 3, 9, 14, 25, 30)}, {#datetime(2024, 3, 9, 0, 0, 0)}}) in Source";
    let f = Transform::FilterValues {
        column: "At".into(),
        values: vec![Literal::DateTime(2024, 3, 9, 0, 0, 0)],
        exclude: false,
    };
    let m = f.m("Source").unwrap();
    let out = insert_step(q, None, "Filtered Rows", &m, &lim()).unwrap();
    assert_eq!(load_m(&out, &lim()).unwrap().row_count(), 1);
}

fn add_filter(v: Literal) -> String {
    let f = Transform::FilterValues {
        column: "Day".into(),
        values: vec![v],
        exclude: false,
    };
    let m = f.m("Source").unwrap();
    insert_step(T, None, "Filtered Rows", &m, &lim()).unwrap()
}

/// One operation over one column of `q`, in place: the column's values.
fn over(q: &str, name: &str, a: OpArgs, column: &str) -> Vec<CellValue> {
    let t = apply(
        q,
        More::ColumnOp {
            op: op(name, a),
            columns: vec![column.into()],
            add: false,
            name: None,
        },
    );
    col(&t, column)
}

fn close(v: &CellValue, x: f64) -> bool {
    match v {
        Real(r) => (r - x).abs() < 1e-9,
        Int(i) => (*i as f64 - x).abs() < 1e-9,
        _ => false,
    }
}

#[test]
fn standard_scientific_trigonometry_and_information() {
    use CellValue::Bool;
    let none = OpArgs::default;
    let number = |n| OpArgs {
        number: Some(n),
        ..OpArgs::default()
    };
    let n = "let Source = #table(type table [N = number], {{7}, {-2}, {0}, {null}}) in Source";
    assert_eq!(
        over(n, "integer_divide", number(2.0), "N")[..3],
        [Int(3), Int(-1), Int(0)]
    );
    assert_eq!(over(n, "modulo", number(3.0), "N")[0], Real(1.0));
    assert_eq!(over(n, "percentage", number(50.0), "N")[0], Real(3.5));
    assert_eq!(over(n, "percent_of", number(14.0), "N")[0], Real(50.0));
    assert_eq!(over(n, "power", number(2.0), "N")[..2], [Real(49.0), Real(4.0)]);
    assert_eq!(over(n, "sign", none(), "N"), [Int(1), Int(-1), Int(0), Null]);
    assert_eq!(
        over(n, "is_even", none(), "N")[..3],
        [Bool(false), Bool(true), Bool(true)]
    );
    assert_eq!(over(n, "is_odd", none(), "N")[0], Bool(true));
    assert_eq!(over(n, "factorial", none(), "N")[0], Real(5040.0));
    assert_eq!(over(n, "factorial", none(), "N")[2], Real(1.0));
    let p = "let Source = #table(type table [P = number], {{100}, {1}}) in Source";
    assert_eq!(over(p, "square_root", none(), "P")[0], Real(10.0));
    assert_eq!(over(p, "log10", none(), "P")[0], Real(2.0));
    assert_eq!(over(p, "ln", none(), "P")[1], Real(0.0));
    assert!(close(&over(p, "exponent", none(), "P")[1], std::f64::consts::E));
    let z = "let Source = #table(type table [A = number], {{0}, {1}}) in Source";
    assert_eq!(over(z, "sin", none(), "A")[0], Real(0.0));
    assert_eq!(over(z, "cos", none(), "A")[0], Real(1.0));
    assert_eq!(over(z, "tan", none(), "A")[0], Real(0.0));
    assert_eq!(over(z, "asin", none(), "A")[0], Real(0.0));
    assert!(close(&over(z, "acos", none(), "A")[1], 0.0));
    assert!(close(
        &over(z, "atan", none(), "A")[1],
        std::f64::consts::FRAC_PI_4
    ));
    for bad in ["integer_divide", "modulo", "percent_of"] {
        assert!(ColumnOp::from_name(bad, &number(0.0)).is_err(), "{bad}");
    }
    assert!(ColumnOp::from_name("power", &none()).is_err());
    // the M as the ribbon writes it
    let m = add_m(More::ColumnOp {
        op: op("power", number(2.0)),
        columns: vec!["Qty".into()],
        add: false,
        name: None,
    });
    assert!(
        m.contains("{{\"Qty\", each Number.Power(_, 2), type number}}"),
        "{m}"
    );
    let m = add_m(More::ColumnOp {
        op: op("is_even", none()),
        columns: vec!["Qty".into()],
        add: true,
        name: None,
    });
    assert!(
        m.contains("\"Is Even\", each Number.IsEven([Qty]), type logical)"),
        "{m}"
    );
}

#[test]
fn factorial_of_a_fraction_or_a_negative_is_an_error_value() {
    let n = "let Source = #table(type table [N = number], {{-1}, {1.5}, {200}}) in Source";
    let t = apply(
        n,
        More::ColumnOp {
            op: op("factorial", OpArgs::default()),
            columns: vec!["N".into()],
            add: true,
            name: None,
        },
    );
    // error values load as blanks; 200! overflows to infinity
    let v = col(&t, "Factorial");
    assert_eq!(v[..2], [Null, Null]);
    assert!(matches!(v[2], Real(x) if x.is_infinite()), "{v:?}");
}

#[test]
fn more_dates_times_and_durations() {
    let none = OpArgs::default;
    let lit = |e: &str| {
        let t = load_m(&format!("#table({{\"D\"}}, {{{{{e}}}}})"), &lim()).unwrap();
        col(&t, "D")[0].clone()
    };
    // 2024-05-04 is a Saturday
    let d = |o: &str| in_place(o, none(), "Day")[1].clone();
    assert_eq!(d("start_of_quarter"), lit("#date(2024, 4, 1)"));
    assert_eq!(d("end_of_quarter"), lit("#date(2024, 6, 30)"));
    assert_eq!(d("start_of_week"), lit("#date(2024, 4, 28)"));
    assert_eq!(d("end_of_week"), lit("#date(2024, 5, 4)"));
    assert_eq!(d("days_in_month"), Int(31));
    assert_eq!(in_place("days_in_month", none(), "Day")[4], Int(29));
    assert_eq!(d("week_of_month"), Int(1));
    let dt =
        "let Source = #table(type table [At = datetime], {{#datetime(2024, 3, 9, 14, 25, 30)}}) in Source";
    let at = |o: &str| over(dt, o, none(), "At")[0].clone();
    assert_eq!(at("time_only"), lit("#time(14, 25, 30)"));
    assert_eq!(at("start_of_hour"), lit("#datetime(2024, 3, 9, 14, 0, 0)"));
    assert_eq!(at("start_of_day"), lit("#datetime(2024, 3, 9, 0, 0, 0)"));
    assert!(matches!(at("end_of_hour"), CellValue::DateTime(_)));
    assert!(matches!(at("end_of_day"), CellValue::DateTime(_)));
    let du = "let Source = #table(type table [L = duration], {{#duration(2, 3, 4, 5)}, {#duration(730, 0, 0, 0)}}) in Source";
    let l = |o: &str| over(du, o, none(), "L");
    assert_eq!(l("duration_days")[0], Int(2));
    assert_eq!(l("duration_hours")[0], Int(3));
    assert_eq!(l("duration_minutes")[0], Int(4));
    assert_eq!(l("duration_seconds")[0], Real(5.0));
    assert!(close(&l("total_hours")[0], 51.0 + 4.0 / 60.0 + 5.0 / 3600.0));
    assert!(close(&l("total_minutes")[0], 3064.0 + 5.0 / 60.0));
    assert!(close(&l("total_seconds")[0], 183_845.0));
    assert!(close(&l("total_days")[1], 730.0));
    assert!(close(&l("total_years")[1], 2.0));
    // Age reads the clock: today minus the date
    let m = add_m(More::ColumnOp {
        op: op("age", none()),
        columns: vec!["Day".into()],
        add: true,
        name: None,
    });
    assert!(
        m.contains("\"Age\", each Date.From(DateTime.LocalNow()) - [Day], type duration)"),
        "{m}"
    );
    // the clock at OLE day 46 030, 01:00 UTC; the first date, 2024-01-31, is OLE day 45 322
    let opts = SourceOptions {
        clock_utc_ms: Some(46_030 * 86_400_000 + 3_600_000),
        clock_local_offset_minutes: Some(0),
        ..SourceOptions::default()
    };
    let t = load_m_with(&m, &opts).unwrap_or_else(|e| panic!("{e}\n{m}"));
    let age = col(&t, "Age");
    assert!(close(&age[0], 708.0), "{age:?}");
}

#[test]
fn text_between_delimiters_and_its_settings() {
    let a = OpArgs {
        text: Some("[".into()),
        end: Some("]".into()),
        ..OpArgs::default()
    };
    let q = "let Source = #table(type table [T = text], {{\"a [b] c\"}}) in Source";
    assert_eq!(over(q, "between_delimiters", a.clone(), "T"), [s("b")]);
    let only = OpArgs {
        text: Some("[".into()),
        ..OpArgs::default()
    };
    assert!(ColumnOp::from_name("between_delimiters", &only).is_err());
    let m = add_m(More::ColumnOp {
        op: op("between_delimiters", a),
        columns: vec!["City".into()],
        add: false,
        name: None,
    });
    assert!(
        m.contains("{{\"City\", each Text.BetweenDelimiters(_, \"[\", \"]\"), type text}}"),
        "{m}"
    );
}
