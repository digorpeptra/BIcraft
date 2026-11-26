//! Section documents and text literal scans (`scan.rs`).

use super::scan::{section_members, text_literals};
use crate::Limits;

#[test]
fn section_members_split_on_top_level_semicolons() {
    let doc = "section Section1;\r\n\r\nshared Sales = let\r\n    Source = Csv.Document(\"a;b\"),\r\n    \
               T = Table.SelectRows(Source, each [x] = \";\")\r\nin\r\n    T;\r\n\
               [ Description = \"x\" ] shared #\"My query\" = 1 + (2; 3);\r\nHelper = 5;\r\n";
    let (name, members) = section_members(doc, &Limits::default()).unwrap();
    assert_eq!(name, "Section1");
    assert_eq!(members.len(), 3);
    assert_eq!(members[0].name, "Sales");
    assert!(members[0].shared);
    assert!(members[0].expression.starts_with("let"));
    assert!(members[0].expression.ends_with("T"));
    assert_eq!(members[1].name, "My query");
    assert_eq!(members[1].expression, "1 + (2; 3)");
    assert_eq!(members[2].name, "Helper");
    assert!(!members[2].shared);
}

#[test]
fn section_members_refuse_what_is_not_a_section() {
    let l = Limits::default();
    assert!(section_members("let a = 1 in a", &l).is_err());
    assert!(section_members("section S; shared A = (1", &l).is_err());
    assert!(section_members("section S; shared = 1;", &l).is_err());
    assert!(section_members("", &l).is_err());
    // deep nesting is capped, never a stack overflow
    let deep = format!("section S; shared A = {};", "(".repeat(100_000));
    assert!(section_members(&deep, &l).is_err());
    let small = Limits {
        max_bytes: 10,
        ..Limits::default()
    };
    assert!(section_members("section S; shared A = 1;", &small).is_err());
}

#[test]
fn text_literals_know_their_call_and_field() {
    let m = "let Source = Sql.Database(\"srv\", \"db\", [Query=\"select 1\"]), \
             W = Web.Contents(Url, [Headers=[Authorization=\"Bearer x\"]]), \
             F = Table.SelectRows(Source, each [City] = \"Paris\") in F";
    let (lits, idents) = text_literals(m, &Limits::default()).unwrap();
    let find = |v: &str| lits.iter().find(|l| l.value == v).unwrap();
    assert_eq!(find("srv").call, Some(("Sql.Database".into(), 0)));
    assert_eq!(find("db").call, Some(("Sql.Database".into(), 1)));
    assert_eq!(find("select 1").field.as_deref(), Some("Query"));
    assert_eq!(find("Bearer x").field.as_deref(), Some("Authorization"));
    assert_eq!(find("Paris").call, None);
    assert!(
        idents
            .iter()
            .any(|i| i.name == "Url" && i.call == "Web.Contents" && i.position == 0)
    );
    let deep = "(".repeat(100_000);
    assert!(text_literals(&deep, &Limits::default()).is_err());
}
