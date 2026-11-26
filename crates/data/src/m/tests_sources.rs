//! `sources.rs`: source calls, argument forms, parameters, M text literals.

use super::sources::*;

fn calls(m: &str) -> Vec<SourceCall> {
    source_calls(m).unwrap()
}

#[test]
fn forms_of_arguments() {
    let c = calls(
        "let\n  P = \"C:\\a\\b.csv\",\n  Q = Text.Upper(P),\n  A = Csv.Document(File.Contents(P)),\n  \
         B = File.Contents(Q),\n  C = File.Contents(Root & \"x.csv\"),\n  D = File.Contents(Text.Combine({Root, \"x\"})),\n  \
         E = Sql.Database(\"srv\", Db, [Query = \"select 1\"]),\n  F = File.Contents(#\"My Root\")\nin\n  A",
    );
    let f: Vec<&str> = c.iter().map(|c| c.function.as_str()).collect();
    assert_eq!(
        f,
        [
            "Csv.Document",
            "File.Contents",
            "File.Contents",
            "File.Contents",
            "File.Contents",
            "Sql.Database",
            "File.Contents"
        ]
    );
    assert!(matches!(&c[0].args[0].form, Form::Call(n) if n == "File.Contents"));
    assert!(matches!(&c[1].args[0].form, Form::StepText(n, l) if n == "P" && l.value == "C:\\a\\b.csv"));
    assert_eq!(c[1].format.as_deref(), Some("Csv.Document"));
    assert!(matches!(&c[2].args[0].form, Form::Name { name, step: true } if name == "Q"));
    assert!(matches!(&c[3].args[0].form, Form::Concat(p) if p.len() == 2));
    assert_eq!(c[4].args[0].form, Form::Call("Text.Combine".into()));
    assert_eq!(c[5].args.len(), 2, "the options record has no role");
    assert_eq!(c[5].args[0].role, Role::Server);
    assert!(matches!(&c[5].args[1].form, Form::Name { name, step: false } if name == "Db"));
    assert!(matches!(&c[6].args[0].form, Form::Name { name, step: false } if name == "My Root"));
}

#[test]
fn record_fields_and_shadowed_steps_are_not_literals() {
    // `P` is a record field, not a step; `S` is bound twice (nested lets): not resolved
    let c = calls(
        "let R = [P = \"C:\\x\"], S = \"C:\\a\", T = let S = \"C:\\b\" in File.Contents(S), U = File.Contents(P) in U",
    );
    assert!(matches!(&c[0].args[0].form, Form::Name { name, step: true } if name == "S"));
    assert!(matches!(&c[1].args[0].form, Form::Name { name, step: false } if name == "P"));
}

#[test]
fn format_through_a_step() {
    let c = calls("let Source = File.Contents(\"/d/a.xlsx\"), Wb = Excel.Workbook(Source) in Wb");
    assert_eq!(c[0].format.as_deref(), Some("Excel.Workbook"));
}

#[test]
fn parameters() {
    let p = parameter_info(
        "\"C:\\x\\\" meta [IsParameterQuery=true, Type=\"Text\", IsParameterQueryRequired=true]",
    )
    .unwrap();
    assert_eq!(p.value, ParamValue::Text("C:\\x\\".into()));
    assert_eq!(p.declared_type.as_deref(), Some("Text"));
    assert_eq!((p.value_start, p.value_end), (0, 7));
    let p = parameter_info("-3.5 meta [IsParameterQuery = true]").unwrap();
    assert_eq!(p.value, ParamValue::Number(-3.5));
    let p = parameter_info("#date(2024, 1, 2) meta [IsParameterQuery = true, Type = \"Date\"]").unwrap();
    assert_eq!(p.value, ParamValue::Other("#date(2024, 1, 2)".into()));
    let p = parameter_info(
        "\"EU\" meta [IsParameterQuery = true, List = {\"EU\", \"US\"}, DefaultValue = \"EU\"]",
    )
    .unwrap();
    assert_eq!(p.allowed, Some(vec!["\"EU\"".into(), "\"US\"".into()]));
    for not in [
        "1",
        "\"x\" meta [IsParameterQuery = false]",
        "let a = 1 meta [IsParameterQuery = true] in a",
        "x meta",
        "x meta [IsParameterQuery = true",
        "\"unterminated",
    ] {
        assert!(parameter_info(not).is_none(), "{not}");
    }
}

#[test]
fn literals_round_trip() {
    for t in ["", "a\"\"b", "#(", "#(#)", "x\u{7f}y", "\u{1f600}", "tab\there"] {
        let m = format!("File.Contents({})", text_literal(t));
        let c = calls(&m);
        assert!(
            matches!(&c[0].args[0].form, Form::Text(l) if l.value == t),
            "{t:?} -> {m}"
        );
    }
}

#[test]
fn hostile_text_never_panics() {
    let deep = format!("File.Contents({}\"x\"{})", "(".repeat(20_000), ")".repeat(20_000));
    let _ = source_calls(&deep);
    let many = "File.Contents(\"a\"),".repeat(20_000);
    let c = source_calls(&format!("{{{many} 1}}")).unwrap();
    assert!(c.len() <= 10_000);
    for t in [
        "File.Contents(",
        ")))(((",
        "File.Contents(,)",
        "let in",
        "meta [",
        "[[[[",
        "let x = let y = in",
    ] {
        let _ = source_calls(t);
        let _ = parameter_info(t);
    }
}
