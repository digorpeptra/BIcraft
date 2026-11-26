use super::*;
use CellValue::{Bool, DateTime, Int, Null, Real, Text};

fn load(s: &str) -> Result<TableData, Error> {
    load_m(s, &Limits::default())
}

fn vals(t: &TableData, i: usize) -> Vec<CellValue> {
    t.columns()[i].values.clone()
}

fn txt(s: &str) -> CellValue {
    Text(s.into())
}

#[test]
fn demo_project_produit() {
    // Exact text of bicraft's demo project (TMSL joins the expression lines with `\n`).
    let e = "let\n    Source = #table(type table [ProduitID = Int64.Type, Produit = text], {{1, \"Vélo\"}, {2, \"Casque\"}, {3, \"Gourde\"}})\nin\n    Source";
    let t = load(e).unwrap();
    assert_eq!(t.columns()[0].name, "ProduitID");
    assert_eq!(t.columns()[0].data_type, DataType::Int64);
    assert_eq!(vals(&t, 0), vec![Int(1), Int(2), Int(3)]);
    assert_eq!(t.columns()[1].data_type, DataType::String);
    assert_eq!(vals(&t, 1), vec![txt("Vélo"), txt("Casque"), txt("Gourde")]);
}

#[test]
fn demo_project_ventes() {
    let e = "let\r\n    Source = #table(type table [Date = date, ProduitID = Int64.Type, Montant = number], {{#date(2026,1,5), 1, 450}, {#date(2026,1,7), 2, 60}, {#date(2026,2,3), 1, 520}, {#date(2026,2,9), 3, 15}})\r\nin\r\n    Source";
    let t = load(e).unwrap();
    assert_eq!(t.row_count(), 4);
    assert_eq!(t.columns()[0].data_type, DataType::DateTime);
    assert_eq!(vals(&t, 0)[0], DateTime(46_027.0));
    assert_eq!(t.columns()[2].data_type, DataType::Double);
    assert_eq!(
        vals(&t, 2),
        vec![Real(450.0), Real(60.0), Real(520.0), Real(15.0)]
    );
}

#[test]
fn name_list_header_infers_types() {
    let t = load("#table({\"A\", \"B\", \"C\", \"D\"}, {{1, 1, \"x\", null}, {2, 2.5, 3, true}})").unwrap();
    let types: Vec<_> = t.columns().iter().map(|c| c.data_type).collect();
    assert_eq!(
        types,
        vec![
            DataType::Int64,
            DataType::Double,
            DataType::String,
            DataType::Boolean
        ]
    );
    assert_eq!(vals(&t, 2), vec![txt("x"), txt("3")]);
    assert_eq!(vals(&t, 3), vec![Null, Bool(true)]);
}

#[test]
fn column_count_header_and_empty_rows() {
    let t = load("#table(2, {{1, 2}})").unwrap();
    assert_eq!(t.columns()[1].name, "Column2");
    let t = load("#table(type table [A = text], {})").unwrap();
    assert_eq!(t.row_count(), 0);
    assert_eq!(t.columns()[0].data_type, DataType::String);
}

#[test]
fn literals_and_escapes() {
    let e = r#"// comment
    let /* block */ #"My Step" = #table(type table [#"Name (x)" = nullable text, Unit Price = Currency.Type, When = datetime, T = time, Z = datetimezone, F = logical, N = type number],
        {{"a""b#(cr,lf)#(tab)#(0041)#(#)", -1.23456, #datetime(2026,1,5,12,0,0), #time(6,0,0), #datetimezone(2026,1,5,13,0,0,1,0), false, -#infinity},
         {null, 0x10, #datetime(2026,1,5,0,0,1.5), #time(24,0,0), #datetimezone(2026,1,5,0,0,0,-5,-30), true, #nan}})
    in #"My Step""#;
    let t = load(e).unwrap();
    assert_eq!(t.columns()[0].name, "Name (x)");
    assert_eq!(t.columns()[1].name, "Unit Price");
    assert_eq!(vals(&t, 0), vec![txt("a\"b\r\n\tA#"), Null]);
    assert_eq!(
        vals(&t, 1),
        vec![CellValue::Decimal(-12_346), CellValue::Decimal(160_000)]
    );
    assert_eq!(
        vals(&t, 2),
        vec![DateTime(46_027.5), DateTime(cv::ole_value(46_027, 1500))]
    );
    assert_eq!(vals(&t, 3), vec![DateTime(0.25), DateTime(1.0)]);
    // A datetimezone loads as its local clock time (Desktop drops the offset).
    assert_eq!(vals(&t, 4)[0], DateTime(cv::ole_value(46_027, 13 * 3_600_000)));
    assert_eq!(vals(&t, 4)[1], DateTime(46_027.0));
    assert_eq!(vals(&t, 5), vec![Bool(false), Bool(true)]);
    assert_eq!(vals(&t, 6)[0], Real(f64::NEG_INFINITY));
    assert!(matches!(vals(&t, 6)[1], Real(r) if r.is_nan()));
}

#[test]
fn let_aliases_and_parens() {
    let t = load("let a = #table({\"A\"}, {{1}}), b = a, c = (b) in c").unwrap();
    assert_eq!(vals(&t, 0), vec![Int(1)]);
    let t = load("let a = let x = #table({\"A\"}, {{2}}) in x in a").unwrap();
    assert_eq!(vals(&t, 0), vec![Int(2)]);
    assert!(matches!(
        load("let a = b, b = a in a"),
        Err(Error::Evaluation { .. })
    ));
    assert!(matches!(
        load("let a = #table({\"A\"}, {}) in zz"),
        Err(Error::UnsupportedSource(_))
    ));
}

#[test]
fn unsupported_sources() {
    for e in [
        // File access is disabled without a file root.
        "let Source = Csv.Document(File.Contents(\"x.csv\")) in Source",
        "#table(type table [A = list], {})",
        "#table(Columns, {})",
        "42",
        "DateTime.LocalNow()",
        "Folder.Files(\"x\")",
    ] {
        let r = load(e);
        assert!(matches!(r, Err(Error::UnsupportedSource(_))), "{e}: {r:?}");
    }
    // External connectors are never run, not even under `try`.
    for e in [
        "let Source = Sql.Database(\"s\", \"d\"), T = Source{[Name=\"t\"]}[Data] in T",
        "let Source = Web.Contents(\"https://example.com\") in Source",
        "try Sql.Database(\"s\", \"d\") otherwise #table({\"A\"}, {})",
        "OData.Feed(\"https://example.com/odata\")",
        "Python.Execute(\"x = 1\")",
        "let S = SharePoint.Files(\"https://x\") in S",
    ] {
        let r = load(e);
        assert!(matches!(r, Err(Error::ExternalSource(_))), "{e}: {r:?}");
    }
    let Err(Error::ExternalSource(m)) = load("Sql.Database(\"s\", \"d\")") else {
        panic!()
    };
    assert!(m.contains("Sql.Database") && m.contains("database"), "{m}");
    let text = Error::ExternalSource(m).to_string();
    assert!(text.starts_with("external source, not evaluated"), "{text}");
}

#[test]
fn type_and_shape_errors() {
    let r = load("#table(type table [A = Int64.Type], {{1}, {2.5}})");
    assert!(
        matches!(&r, Err(Error::Value { row: 2, column, .. }) if column == "A"),
        "{r:?}"
    );
    assert!(matches!(
        load("#table(type table [A = text], {{1}})"),
        Err(Error::Value { row: 1, .. })
    ));
    assert!(matches!(
        load("#table({\"A\", \"B\"}, {{1}})"),
        Err(Error::Evaluation { .. })
    ));
    // An invalid date is an error cell: a blank by default, an error in strict mode.
    let bad_date = "#table({\"A\"}, {{#date(2025,2,29)}})";
    assert_eq!(vals(&load(bad_date).unwrap(), 0), vec![Null]);
    let strict = SourceOptions {
        strict_errors: true,
        ..SourceOptions::default()
    };
    assert!(matches!(
        load_m_with(bad_date, &strict),
        Err(Error::Value { row: 1, .. })
    ));
    assert!(matches!(
        load("#table({\"A\"}, {{\"open}})"),
        Err(Error::Syntax { .. })
    ));
    assert!(matches!(load("#table({\"A\"}, {{1}}"), Err(Error::Syntax { .. })));
    assert!(matches!(
        load("#table({\"A\", \"a\"}, {})"),
        Err(Error::InvalidTable(_))
    ));
    assert!(matches!(load("/* open"), Err(Error::Syntax { .. })));
    assert!(matches!(
        load("#table({\"A\"}, {{\"#(zz)\"}})"),
        Err(Error::Syntax { .. })
    ));
}

#[test]
fn limits_and_hostile_input() {
    let deep = format!("{}#table({{\"A\"}}, {{}}){}", "(".repeat(500), ")".repeat(500));
    assert!(matches!(load(&deep), Err(Error::LimitExceeded(_))));
    let limits = Limits {
        max_rows: 1,
        ..Limits::default()
    };
    assert!(matches!(
        load_m("#table({\"A\"}, {{1}, {2}})", &limits),
        Err(Error::LimitExceeded(_))
    ));
    let limits = Limits {
        max_bytes: 3,
        ..Limits::default()
    };
    assert!(matches!(load_m("#table", &limits), Err(Error::LimitExceeded(_))));
    assert!(matches!(
        load("#table(99999999999, {})"),
        Err(Error::LimitExceeded(_))
    ));
    for s in [
        "",
        "#",
        "#table",
        "#table(",
        "#table(type",
        "#table(type table [",
        "let",
        "let a",
        "let a =",
        "\"",
        "#\"",
        "0x",
        "1e",
        "#date(",
        "-",
        "{",
    ] {
        assert!(load(s).is_err(), "{s}");
        assert!(load(&format!("#table({{\"A\"}}, {{{{{s}}}}})")).is_err(), "{s}");
    }
}

#[test]
fn load_counting_errors_counts_each_columns_error_values_and_loads_them_as_blanks() {
    let e = "let\n    S = Table.TransformColumnTypes(#table({\"A\", \"B\"}, {{\"1\", \"x\"}, {\"x\", \"y\"}, {\"3\", \"4\"}}), {{\"A\", Int64.Type}, {\"B\", Int64.Type}})\nin\n    S";
    let (t, errors) = load_m_counting_errors(
        e,
        &SourceOptions {
            strict_errors: true,
            ..SourceOptions::default()
        },
    )
    .unwrap();
    assert_eq!(errors, vec![1, 2]);
    assert_eq!(vals(&t, 0), vec![Int(1), Null, Int(3)]);
    assert_eq!(vals(&t, 1), vec![Null, Null, Int(4)]);
    // no errors: zeros
    let (_, errors) = load_m_counting_errors("#table({\"A\"}, {{1}})", &SourceOptions::default()).unwrap();
    assert_eq!(errors, vec![0]);
    assert!(load_m_counting_errors("1", &SourceOptions::default()).is_err());
}
