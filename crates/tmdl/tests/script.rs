//! TMDL scripts: `createOrReplace` scripts of model objects, and reading them back.

use bicraft_tmdl::{Scripted, read_documents, read_script, script};

const MODEL: &str = "model Model\n\tculture: en-US\n\n\
table Sales\n\tlineageTag: t1\n\n\
\tmeasure Total = SUM(Sales[Amount])\n\t\tformatString: 0.00\n\n\
\tmeasure Count = COUNTROWS(Sales)\n\n\
\tcolumn Amount\n\t\tdataType: double\n\t\tsourceColumn: Amount\n\n\
\tpartition Sales = m\n\t\tmode: import\n\t\tsource = #table({\"Amount\"}, {{1}})\n\n\
/// Adds the tax\n\
function AddTax = (amount : NUMERIC) => amount * 1.1\n\tlineageTag: f1\n";

fn model() -> bicraft_model::SemanticModel {
    read_documents(&[("model.tmdl".into(), MODEL.into())]).unwrap()
}

#[test]
fn scripts_a_table_its_members_under_ref_table_and_a_function() {
    let m = model();
    let s = script(
        &m,
        &[
            Scripted::Measure {
                table: "Sales".into(),
                name: "Total".into(),
            },
            Scripted::Function("addtax".into()),
        ],
    )
    .unwrap();
    assert!(s.starts_with("createOrReplace\n\n\tref table Sales\n"), "{s}");
    assert!(
        s.contains("\t\tmeasure Total = SUM(Sales[Amount])\n\t\t\tformatString: 0.00\n"),
        "{s}"
    );
    assert!(!s.contains("Count"), "only the scripted measure: {s}");
    assert!(
        s.contains("\t/// Adds the tax\n\tfunction AddTax = (amount : NUMERIC) => amount * 1.1\n"),
        "{s}"
    );
    assert!(!s.contains('\r'));

    let read = read_script(&s).unwrap();
    let kinds: Vec<(&str, bool, usize)> = read
        .entries
        .iter()
        .map(|e| (e.kind.as_str(), e.is_ref, e.members.len()))
        .collect();
    assert_eq!(kinds, [("table", true, 1), ("function", false, 0)]);
    let t = &read.model.tables[0];
    assert_eq!(t.name, "Sales");
    assert_eq!(t.measures.len(), 1);
    assert_eq!(t.measures[0].format_string.as_deref(), Some("0.00"));
    assert_eq!(read.model.extra["functions"][0]["description"], "Adds the tax");
}

#[test]
fn a_whole_table_and_the_whole_model_read_back() {
    let m = model();
    let s = script(&m, &[Scripted::Table("Sales".into())]).unwrap();
    let read = read_script(&s).unwrap();
    assert_eq!(read.model.tables[0], m.tables[0]);
    let s = script(&m, &[Scripted::Model]).unwrap();
    let read = read_script(&s).unwrap();
    assert_eq!(read.model.tables, m.tables);
    assert_eq!(read.model.extra["functions"], m.extra["functions"]);
}

#[test]
fn bad_scripts_are_errors_with_their_line() {
    let line = |text: &str| match read_script(text) {
        Err(bicraft_tmdl::Error::Syntax { line, .. }) => line,
        other => panic!("{other:?}"),
    };
    assert_eq!(line(""), 1);
    assert_eq!(line("delete\n\ttable Sales\n"), 1);
    assert_eq!(line("createOrReplace\n"), 1);
    assert_eq!(line("createOrReplace\n\ttable Sales\ntable Other\n"), 3);
    assert_eq!(line("createOrReplace\n\n\tformatString: 0\n"), 3);
    assert_eq!(line("createOrReplace\n\ttable Sales\n\t\tmeasure 'x = 1\n"), 3);
    assert!(script(&model(), &[Scripted::Table("Nope".into())]).is_err());
}
