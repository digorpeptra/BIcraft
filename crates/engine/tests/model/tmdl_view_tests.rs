//! `tmdl.script` and `tmdl.apply` (the TMDL view): scripts of model objects, createOrReplace
//! applied in one undo step, positioned errors that change nothing, and what an applied script
//! writes saved to PBIP and PBIX.

use serde_json::{Value, json};

use bicraft_engine::Session;

use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn shop() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.import",
            json!({"name": "Sales", "m": "#table(type table [Amount = number], {{10}, {5}, {7}})"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "0"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Count", "expression": "COUNTROWS(Sales)"}),
        ),
    ] {
        run(&mut s, id, p);
    }
    s
}

fn expression(s: &mut Session, name: &str) -> Option<String> {
    let m = s.execute("measure.get", &json!({"name": name})).ok()?;
    m.get("expression").and_then(Value::as_str).map(str::to_string)
}

#[test]
fn script_edit_and_apply_a_measure_in_one_undo_step() {
    let mut s = shop();
    let r = run(
        &mut s,
        "tmdl.script",
        json!({"objects": [{"kind": "measure", "table": "Sales", "name": "Total"}]}),
    );
    let script = r["script"].as_str().unwrap().to_string();
    assert!(
        script
            .starts_with("createOrReplace\n\n\tref table Sales\n\n\t\tmeasure Total = SUM(Sales[Amount])\n"),
        "{script}"
    );
    assert!(!script.contains("Count"), "{script}");
    let edited = script.replace("SUM(Sales[Amount])", "SUM(Sales[Amount]) * 2")
        + "\n\t\tmeasure Average = DIVIDE([Total], [Count])\n\n\tfunction Half = (x : NUMERIC) => x / 2\n";
    let rev = s.revision;
    let r = run(&mut s, "tmdl.apply", json!({"script": edited}));
    assert_eq!(
        r["applied"],
        json!([
            {"kind": "measure", "table": "Sales", "name": "Total", "action": "replaced"},
            {"kind": "measure", "table": "Sales", "name": "Average", "action": "added"},
            {"kind": "function", "name": "Half", "action": "added"},
        ]),
        "{r}"
    );
    assert_eq!(
        expression(&mut s, "Total").as_deref(),
        Some("SUM(Sales[Amount]) * 2")
    );
    assert_eq!(
        expression(&mut s, "Count").as_deref(),
        Some("COUNTROWS(Sales)"),
        "untouched"
    );
    assert!(s.revision > rev);
    let v = run(
        &mut s,
        "dax.evaluate",
        json!({"query": "EVALUATE ROW(\"x\", Half([Average]))"}),
    );
    // [Average] = (2 × 22) / 3 with the new [Total]; Half halves it
    let x = v["results"][0]["rows"][0]["[x]"].as_f64().unwrap();
    assert!((x - 22.0 / 3.0).abs() < 1e-9, "{v}");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(expression(&mut s, "Total").as_deref(), Some("SUM(Sales[Amount])"));
    assert_eq!(expression(&mut s, "Average"), None);
}

#[test]
fn a_scripted_table_replaces_the_whole_table() {
    let mut s = shop();
    let r = run(
        &mut s,
        "tmdl.script",
        json!({"objects": [{"kind": "table", "name": "Sales"}]}),
    );
    let script = r["script"].as_str().unwrap();
    // drop the Count measure from the script: createOrReplace replaces every descendant
    let without: String = {
        let mut out = Vec::new();
        let mut skip = false;
        for l in script.lines() {
            if l.starts_with("\t\tmeasure Count") {
                skip = true;
                continue;
            }
            if skip && (l.starts_with("\t\t\t") || l.is_empty()) {
                continue;
            }
            skip = false;
            out.push(l);
        }
        out.join("\n")
    };
    let r = run(&mut s, "tmdl.apply", json!({"script": without}));
    assert_eq!(
        r["applied"][0],
        json!({"kind": "table", "name": "Sales", "action": "replaced"})
    );
    assert_eq!(expression(&mut s, "Count"), None);
    assert_eq!(expression(&mut s, "Total").as_deref(), Some("SUM(Sales[Amount])"));
    // the whole model scripts and applies back to the same model
    let before = s.model.clone();
    let all = run(&mut s, "tmdl.script", json!({}));
    run(&mut s, "tmdl.apply", json!({"script": all["script"]}));
    assert_eq!(s.model.as_ref().unwrap().tables, before.unwrap().tables);
}

#[test]
fn errors_are_positioned_and_change_nothing() {
    let mut s = shop();
    let rev = s.revision;
    for (script, why) in [
        ("", "line 1"),
        (
            "delete\n\ttable Sales\n",
            "line 1, column 1: a TMDL script starts with a command",
        ),
        (
            "createOrReplace\n\n\tref table Nope\n\t\tmeasure X = 1\n",
            "line 3, column 2: ref table `Nope`",
        ),
        (
            "createOrReplace\n\n\tref table Sales\n\t\tmeasure X = 1\n\n\t\tmeasure Y = SUM(Sales[Nope])\n",
            "line 6, column 19: measure [Y]",
        ),
        (
            "createOrReplace\n\n\tref table Sales\n\t\tmeasure Z =\n\t\t\t\tVAR a = 1\n\t\t\t\tRETURN a + [Nope]\n",
            "line 6, column 16: measure [Z]",
        ),
        (
            "createOrReplace\n\n\tfunction Bad = (x) => x + [Nope]\n",
            "line 3",
        ),
        (
            "createOrReplace\n\n\tref table Sales\n\t\tcolumn Amount\n\t\tmeasure 'x = 1\n",
            "line 5",
        ),
        ("createOrReplace\n\n\tkpi Foo\n", "cannot be applied"),
    ] {
        let e = s.execute("tmdl.apply", &json!({"script": script})).unwrap_err();
        assert!(e.contains(why), "{script:?}: {e}");
        assert!(
            e.contains("nothing was applied") || e.contains("cannot be applied") || e.contains("ref table"),
            "{e}"
        );
        assert_eq!(s.revision, rev, "{script:?} changed the session");
        assert_eq!(expression(&mut s, "X"), None);
    }
    for bad in [
        json!({}),
        json!({"script": 1}),
        json!({"script": "x", "other": 1}),
    ] {
        assert!(s.execute("tmdl.apply", &bad).is_err(), "{bad}");
    }
    for bad in [
        json!({"objects": []}),
        json!({"objects": "Sales"}),
        json!({"objects": [{"kind": "measure", "name": "Total"}]}),
        json!({"objects": [{"kind": "table", "name": "Nope"}]}),
        json!({"objects": [{"kind": "thing", "name": "x"}]}),
        json!({"objects": [1]}),
    ] {
        assert!(s.execute("tmdl.script", &bad).is_err(), "{bad}");
    }
    // force applies invalid DAX with a warning
    let r = run(
        &mut s,
        "tmdl.apply",
        json!({"script": "createOrReplace\n\tref table Sales\n\t\tmeasure Y = [Nope]\n", "force": true}),
    );
    assert!(r["warnings"][0].as_str().unwrap().contains("line 3"), "{r}");
}

#[test]
fn what_a_script_applies_is_saved_to_pbip_and_pbix() {
    let tmp = TempDir::new("engine-tmdl-apply-save");
    for (format, file) in [("pbip-tmdl", "Shop.pbip"), ("pbix", "Shop.pbix")] {
        let mut s = shop();
        run(
            &mut s,
            "tmdl.apply",
            json!({"script": "createOrReplace\n\tref table Sales\n\t\tmeasure Avg = DIVIDE([Total], [Count])\n\t\t\tformatString: 0.0\n"}),
        );
        let path = tmp.path().join(file);
        run(&mut s, "file.save_as", json!({"path": path, "format": format}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        assert_eq!(back.model_error, None, "{format}");
        let m = run(&mut back, "measure.get", json!({"name": "Avg"}));
        assert_eq!(m["expression"], "DIVIDE([Total], [Count])", "{format}");
        assert_eq!(m["format_string"], "0.0", "{format}: {m}");
    }
}
