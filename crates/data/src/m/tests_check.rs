//! `analyze`: syntax errors, free names, steps and result types, without evaluation.

use super::*;
use crate::Limits;

fn free(src: &str) -> Vec<String> {
    let a = analyze(src, &Limits::default());
    assert_eq!(a.syntax, None, "{src}");
    a.references.into_iter().map(|r| r.name).collect()
}

#[test]
fn let_steps_and_library_names_are_not_free_but_queries_are() {
    let src = "let\n    Source = Databricks.Catalogs(Host, \"/sql/1.0/x\", [Catalog = null]),\n    \
               Nav = Source{[Name = \"main\", Kind = \"Database\"]}[Data],\n    \
               #\"Filtered Rows\" = Table.SelectRows(Nav, each [Year] > MinYear)\nin\n    #\"Filtered Rows\"";
    assert_eq!(
        free(src),
        ["Databricks.Catalogs", "Host", "Table.SelectRows", "MinYear"]
    );
    let a = analyze(src, &Limits::default());
    assert_eq!(a.steps, ["Source", "Nav", "Filtered Rows"]);
    assert_eq!(a.result_type, Some("Table"));
    let host = a.references.get(1).cloned().unwrap_or_else(|| panic!("no Host"));
    assert_eq!((host.line, host.column), (Some(2), Some(34)));
    let (s, e) = host.span.unwrap_or_default();
    assert_eq!(src.get(s..e), Some("Host"));
}

#[test]
fn scopes_functions_records_each_and_catch() {
    assert_eq!(free("(x, optional y) => x + y + z"), ["z"]);
    assert_eq!(free("[a = 1, b = a + c]"), ["c"]);
    assert!(free("each _ + 1").is_empty());
    assert!(free("each [Amount] * 2").is_empty());
    assert_eq!(free("[Amount]"), ["_"]);
    assert_eq!(free("try e otherwise f"), ["e", "f"]);
    assert!(free("try 1 catch (e) => e[Message]").is_empty());
    assert_eq!(free("try e catch (e) => 1"), ["e"]);
    assert_eq!(
        free("let f = (n) => if n = 0 then 1 else n * @f(n - 1) in f(5)"),
        Vec::<String>::new()
    );
    assert_eq!(free("#\"My Query\" & Other"), ["My Query", "Other"]);
    assert_eq!(free("1 is number and (x as text) = \"\""), ["x"]);
    assert_eq!(free("type table [A = Int64.Type, B = t]"), ["Int64.Type", "t"]);
}

#[test]
fn syntax_errors_have_positions() {
    let a = analyze("let\n  a = 1,\n  b = \nin b", &Limits::default());
    let e = a.syntax.unwrap_or_else(|| panic!("expected a syntax error"));
    assert_eq!(e.line, 4);
    assert!(a.references.is_empty());
    let a = analyze("\"unterminated", &Limits::default());
    assert!(a.syntax.is_some());
}

#[test]
fn result_types() {
    let t = |s: &str| analyze(s, &Limits::default()).result_type;
    assert_eq!(
        t("\"x\" meta [IsParameterQuery = true, Type = \"Text\"]"),
        Some("Text")
    );
    assert_eq!(t("42 meta [IsParameterQuery = true]"), Some("Number"));
    assert_eq!(t("(x) => x"), Some("Function"));
    assert_eq!(t("let a = #table({\"A\"}, {{1}}) in a"), Some("Table"));
    assert_eq!(t("let a = Sql.Database(\"s\", \"d\") in a"), Some("Table"));
    assert_eq!(t("let a = Table.RowCount(x) in a"), None);
    assert_eq!(t("#date(2020, 1, 1)"), Some("Date"));
    assert_eq!(t("Source"), None);
}

#[test]
fn library_names() {
    assert_eq!(library_name("Table.SelectRows"), LibraryName::Evaluated);
    assert_eq!(library_name("Databricks.Catalogs"), LibraryName::Connector);
    assert_eq!(
        library_name("Table.AddFuzzyClusterColumn"),
        LibraryName::Documented
    );
    assert_eq!(library_name("Int64.Type"), LibraryName::Documented);
    assert_eq!(library_name("JoinKind.LeftOuter"), LibraryName::Documented);
    assert_eq!(library_name("Table.NoSuchThing"), LibraryName::Unknown);
    assert_eq!(library_name("Sales"), LibraryName::Unknown);
}

#[test]
fn identifiers_are_quoted_when_needed() {
    assert_eq!(identifier("Sales"), "Sales");
    assert_eq!(identifier("Sales.Fact"), "Sales.Fact");
    assert_eq!(identifier("My Query"), "#\"My Query\"");
    assert_eq!(identifier("let"), "#\"let\"");
    assert_eq!(identifier("2020"), "#\"2020\"");
    assert_eq!(identifier("a\"b"), "#\"a\"\"b\"");
    // The quoted form parses back to the same name.
    assert_eq!(free(&identifier("a\"b")), ["a\"b"]);
}

#[test]
fn hostile_input_is_bounded() {
    let deep = format!("{}1{}", "(".repeat(100_000), ")".repeat(100_000));
    assert!(analyze(&deep, &Limits::default()).syntax.is_some());
    let long = format!("a{}", " + a".repeat(200_000));
    let a = analyze(&long, &Limits::default());
    assert_eq!(a.syntax, None);
    assert_eq!(a.references.len(), 200_001);
}

fn kinds(src: &str) -> Vec<(String, crate::DateKind)> {
    let a = analyze(src, &Limits::default());
    assert_eq!(a.syntax, None, "{src}");
    a.date_kinds
}

#[test]
fn date_kinds_come_from_typed_tables_type_changes_and_added_columns() {
    use crate::DateKind::*;
    let k = |n: &str, d| (n.to_string(), d);
    assert_eq!(
        kinds("#table(type table [d = date, t = nullable time, n = number, x = DateTime.Type], {})"),
        [k("d", Date), k("t", Time), k("x", DateTime)]
    );
    let src = "let\n    Source = Sql.Database(\"s\", \"db\"),\n    T = Source{[Schema = \"dbo\", Item = \"F\"]}[Data],\n    \
               #\"Changed Type\" = Table.TransformColumnTypes(T, {{\"When\", type date}, {\"At\", type datetimezone}, {\"N\", Int64.Type}}),\n    \
               Added = Table.AddColumn(#\"Changed Type\", \"Hour\", each Time.From([At]), type time),\n    \
               Untyped = Table.AddColumn(Added, \"Copy\", each [When]),\n    \
               Renamed = Table.RenameColumns(Untyped, {{\"When\", \"Day\"}}),\n    \
               Back = Table.TransformColumnTypes(Renamed, {\"At\", type text})\nin\n    Back";
    assert_eq!(kinds(src), [k("Hour", Time), k("Day", Date)]);
    // Nested calls run inside out: the outer type change wins.
    assert_eq!(
        kinds("Table.TransformColumnTypes(#table(type table [d = date], {}), {{\"d\", type datetime}})"),
        [k("d", DateTime)]
    );
    assert!(kinds("let a = 1 in a").is_empty());
}
