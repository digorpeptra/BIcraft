use super::*;

fn field(kind: Kind, required: bool) -> &'static Param {
    Box::leak(Box::new(Param {
        name: "x",
        kind,
        required,
        default: None,
    }))
}

fn input(kind: Kind, required: bool, text: &str) -> Input {
    Input {
        field: field(kind, required),
        text: text.into(),
        flag: None,
    }
}

fn v(kind: Kind, text: &str) -> Result<Option<Value>, String> {
    value(&input(kind, false, text), Lang::En)
}

#[test]
fn values_are_checked_by_kind() {
    assert_eq!(
        value(&input(Kind::Text, true, " "), Lang::En),
        Err("required".into())
    );
    assert_eq!(
        value(&input(Kind::Text, true, ""), Lang::Fr),
        Err("obligatoire".into())
    );
    assert_eq!(v(Kind::Text, ""), Ok(None));
    assert_eq!(v(Kind::Text, " a b "), Ok(Some(json!(" a b "))));
    assert_eq!(v(Kind::Name(Obj::Table), " Sales "), Ok(Some(json!("Sales"))));
    assert_eq!(v(Kind::Integer, "12"), Ok(Some(json!(12))));
    assert!(v(Kind::Integer, "1.5").is_err() && v(Kind::Integer, "abc").is_err());
    assert_eq!(v(Kind::Number, "1,5"), Ok(Some(json!(1.5))));
    assert!(v(Kind::Number, "NaN").is_err() && v(Kind::Number, "x").is_err());
    let e = Kind::Enum(&["a", "b"]);
    assert_eq!(v(e, "b"), Ok(Some(json!("b"))));
    assert!(v(e, "c").unwrap_err().contains("a, b"));
    assert_eq!(v(Kind::Color, "#118DFF"), Ok(Some(json!("#118DFF"))));
    assert!(v(Kind::Color, "blue").is_err() && v(Kind::Color, "#12345").is_err());
    assert_eq!(v(Kind::List, "a\nb, c\n"), Ok(Some(json!(["a", "b", "c"]))));
    assert_eq!(v(Kind::List, "[1, 2]"), Ok(Some(json!([1, 2]))));
    assert!(v(Kind::List, "[1,").is_err() && v(Kind::List, "[").is_err());
    assert_eq!(v(Kind::Json, r#"{"a": 1}"#), Ok(Some(json!({"a": 1}))));
    assert_eq!(v(Kind::Json, "top"), Ok(Some(json!("top"))), "a bare word");
    assert!(v(Kind::Json, "{oops").is_err());
    assert_eq!(
        v(Kind::Paths, "a.csv\n b.csv"),
        Ok(Some(json!(["a.csv", "b.csv"])))
    );
    assert_eq!(v(Kind::Paths, "a.csv"), Ok(Some(json!("a.csv"))));
    assert!(v(Kind::Paths, "{bad").is_err());
    let mut b = input(Kind::Bool, false, "");
    assert_eq!(
        value(&b, Lang::En),
        Ok(None),
        "an untouched optional flag is left out"
    );
    b.flag = Some(false);
    assert_eq!(value(&b, Lang::En), Ok(Some(json!(false))));
    assert_eq!(
        value(&input(Kind::Bool, true, ""), Lang::En),
        Ok(Some(json!(false)))
    );
    assert_eq!(color("#FF8000"), Some(Color32::from_rgb(255, 128, 0)));
}

#[test]
fn parameters_and_their_problems() {
    let mut f = Form {
        id: "measure.add",
        inputs: vec![
            Input {
                field: field(Kind::Text, true),
                text: String::new(),
                flag: None,
            },
            input(Kind::Integer, false, "x"),
        ],
        extra: Some("[1]".into()),
        tried: false,
        running: None,
        result: None,
        view: None,
        lookups: Vec::new(),
        prefill: Value::Null,
        help: false,
        serial: 0,
    };
    let errs = params(&f, Lang::En).unwrap_err();
    let at: Vec<usize> = errs.iter().map(|e| e.0).collect();
    assert_eq!(at, vec![2, 0, 1], "the JSON field, the required one, the number");
    f.inputs[0].text = "Total".into();
    f.inputs[1].text = "3".into();
    f.extra = Some(r#"{"y": true}"#.into());
    assert_eq!(params(&f, Lang::En), Ok(json!({"x": 3, "y": true})));
}

#[test]
fn path_modes() {
    assert_eq!(path_mode("file.open", "path"), PathMode::Open);
    assert_eq!(path_mode("file.save_as", "path"), PathMode::Save);
    assert_eq!(path_mode("report.render", "out"), PathMode::Save);
    assert_eq!(path_mode("file.convert", "to"), PathMode::Save);
    assert_eq!(path_mode("project.new", "dir"), PathMode::Folder);
    assert_eq!(path_mode("file.open", "data_root"), PathMode::Folder);
}
