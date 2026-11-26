//! Tests of perspective upkeep through removals: members of removed tables, columns, measures,
//! hierarchies and calculation groups leave the perspectives, the result reports them, undo
//! brings them back, and PBIP (TMSL, TMDL) and PBIX saves write the trimmed perspectives.

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Sales(Amount, Region, City) with [Total], [Count] and the hierarchy Geo, Product(Name), the
/// calculation group Calc, and the perspective Lite naming all of them.
fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        ("table.add", json!({"name": "Sales"})),
        ("table.add", json!({"name": "Product"})),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Region", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "City", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Name", "data_type": "string"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Count", "expression": "COUNTROWS(Sales)"}),
        ),
        (
            "hierarchy.add",
            json!({"table": "Sales", "name": "Geo", "levels": ["Region", "City"]}),
        ),
        ("calculation_group.add", json!({"name": "Calc"})),
    ] {
        run(&mut s, id, p);
    }
    s.model.as_mut().unwrap().extra.insert(
        "perspectives".into(),
        json!([{"name": "Lite", "tables": [
            {"name": "Sales",
             "columns": [{"name": "Amount"}, {"name": "Region"}, {"name": "City"}],
             "measures": [{"name": "Total"}, {"name": "Count"}],
             "hierarchies": [{"name": "Geo"}]},
            {"name": "Product", "includeAll": true},
            {"name": "Calc", "includeAll": true}
        ]}]),
    );
    s
}

fn lite(s: &Session) -> Value {
    s.model.as_ref().unwrap().extra["perspectives"][0].clone()
}

fn dropped(v: &Value) -> Value {
    v["perspective_entries_dropped"].clone()
}

#[test]
fn removals_drop_their_perspective_entries() {
    let mut s = session();
    let v = run(
        &mut s,
        "column.remove",
        json!({"table": "Sales", "name": "Amount", "force": true}),
    );
    assert_eq!(
        dropped(&v),
        json!([{"perspective": "Lite", "objects": ["column of `Sales` `Amount`"]}])
    );
    let v = run(&mut s, "measure.remove", json!({"name": "Total", "force": true}));
    assert_eq!(dropped(&v)[0]["objects"], json!(["measure of `Sales` `Total`"]));
    let v = run(
        &mut s,
        "hierarchy.remove",
        json!({"table": "Sales", "name": "Geo"}),
    );
    assert_eq!(dropped(&v)[0]["objects"], json!(["hierarchy of `Sales` `Geo`"]));
    let v = run(&mut s, "calculation_group.remove", json!({"name": "Calc"}));
    assert_eq!(dropped(&v)[0]["objects"], json!(["table `Calc`"]));
    let v = run(&mut s, "table.remove", json!({"name": "Product"}));
    assert_eq!(dropped(&v)[0]["objects"], json!(["table `Product`"]));
    assert_eq!(
        lite(&s)["tables"],
        json!([{"name": "Sales",
                "columns": [{"name": "Region"}, {"name": "City"}],
                "measures": [{"name": "Count"}]}])
    );
    // An edit that removes nothing reports nothing.
    let v = run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "N", "expression": "1"}),
    );
    assert!(v.get("perspective_entries_dropped").is_none(), "{v}");
}

#[test]
fn removing_a_table_drops_its_perspective_table_and_undo_restores_it() {
    let mut s = session();
    let before = lite(&s);
    let v = run(&mut s, "table.remove", json!({"name": "Sales", "force": true}));
    assert_eq!(dropped(&v)[0]["objects"], json!(["table `Sales`"]));
    assert_eq!(lite(&s)["tables"][0]["name"], "Product");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(lite(&s), before);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(lite(&s)["tables"].as_array().map(Vec::len), Some(2));
}

#[test]
fn renames_keep_perspective_entries() {
    let mut s = session();
    run(
        &mut s,
        "table.rename",
        json!({"name": "Sales", "new_name": "Orders"}),
    );
    run(
        &mut s,
        "column.rename",
        json!({"table": "Orders", "name": "Amount", "new_name": "Value"}),
    );
    run(
        &mut s,
        "measure.rename",
        json!({"name": "Total", "new_name": "Revenue"}),
    );
    run(
        &mut s,
        "hierarchy.set",
        json!({"table": "Orders", "name": "Geo", "new_name": "Place"}),
    );
    let v = run(
        &mut s,
        "measure.add",
        json!({"table": "Orders", "name": "N", "expression": "1"}),
    );
    assert!(v.get("perspective_entries_dropped").is_none(), "{v}");
    assert_eq!(
        lite(&s)["tables"][0],
        json!({"name": "Orders",
               "columns": [{"name": "Value"}, {"name": "Region"}, {"name": "City"}],
               "measures": [{"name": "Revenue"}, {"name": "Count"}],
               "hierarchies": [{"name": "Place"}]})
    );
}

/// Removals inside `batch` go through the same commands.
#[test]
fn batch_removals_drop_perspective_entries() {
    let mut s = session();
    let v = s
        .batch(&[
            ("table.remove".to_string(), json!({"name": "Product"})),
            ("measure.remove".to_string(), json!({"name": "Count"})),
        ])
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(dropped(&v[0])[0]["objects"], json!(["table `Product`"]));
    assert_eq!(
        lite(&s)["tables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["Sales", "Calc"]
    );
    assert_eq!(lite(&s)["tables"][0]["measures"], json!([{"name": "Total"}]));
}

/// TMSL and TMDL saves write the trimmed perspective and read it back.
#[test]
fn pbip_saves_write_the_trimmed_perspective() {
    let tmp = TempDir::new("engine-perspectives-pbip");
    let mut s = session();
    run(&mut s, "table.remove", json!({"name": "Product"}));
    run(&mut s, "measure.remove", json!({"name": "Total", "force": true}));
    let want = lite(&s);
    for (format, path) in [
        ("pbip-tmdl", tmp.path().join("Tmdl.pbip")),
        ("pbip-tmsl", tmp.path().join("Tmsl.pbip")),
    ] {
        run(&mut s, "file.save_as", json!({"path": path, "format": format}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        assert_eq!(back.model_error, None, "{format}");
        let got = &back.model.as_ref().unwrap().extra["perspectives"][0];
        assert_eq!(got["name"], want["name"], "{format}");
        let names = |v: &Value, key: &str| -> Vec<String> {
            v[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x["name"].as_str().map(str::to_string))
                .collect()
        };
        assert_eq!(names(got, "tables"), ["Sales", "Calc"], "{format}");
        assert_eq!(names(&got["tables"][0], "measures"), ["Count"], "{format}");
        assert_eq!(
            names(&got["tables"][0], "columns"),
            ["Amount", "Region", "City"],
            "{format}"
        );
    }
}

/// A PBIX built from scratch with a perspective over two imported tables, then edited in place:
/// removing a table, a measure and a hierarchy rewrites the metadata without their perspective
/// rows, and the perspective reads back trimmed. With `BICRAFT_FRESH_OUT=<dir>` the edited file
/// is copied there for the engine check (the maintainers' development oracles).
#[test]
fn pbix_saves_write_the_trimmed_perspective() {
    let tmp = TempDir::new("engine-perspectives-pbix");
    let path = tmp.path().join("persp_removed.pbix");
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.import",
            json!({"name": "Sales", "m": "let Source = #table(type table [Amount = number, Region = text, City = text], {{1.5, \"N\", \"A\"}, {2, \"S\", \"B\"}}) in Source"}),
        ),
        (
            "table.import",
            json!({"name": "Product", "m": "let Source = #table(type table [Name = text], {{\"X\"}, {\"Y\"}}) in Source"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Count", "expression": "COUNTROWS(Sales)"}),
        ),
        (
            "hierarchy.add",
            json!({"table": "Sales", "name": "Geo", "levels": ["Region", "City"]}),
        ),
    ] {
        run(&mut s, id, p);
    }
    s.model.as_mut().unwrap().extra.insert(
        "perspectives".into(),
        json!([{"name": "Lite", "tables": [
            {"name": "Sales",
             "columns": [{"name": "Amount"}, {"name": "Region"}],
             "measures": [{"name": "Total"}, {"name": "Count"}],
             "hierarchies": [{"name": "Geo"}]},
            {"name": "Product", "includeAll": true}
        ]}]),
    );
    run(&mut s, "file.save_as", json!({"path": path, "format": "pbix"}));
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    assert_eq!(lite(&s)["tables"].as_array().map(Vec::len), Some(2));
    let v = run(&mut s, "table.remove", json!({"name": "Product"}));
    assert_eq!(dropped(&v)[0]["objects"], json!(["table `Product`"]));
    let v = run(&mut s, "measure.remove", json!({"name": "Total", "force": true}));
    assert_eq!(dropped(&v)[0]["objects"], json!(["measure of `Sales` `Total`"]));
    let v = run(
        &mut s,
        "hierarchy.remove",
        json!({"table": "Sales", "name": "Geo"}),
    );
    assert_eq!(dropped(&v)[0]["objects"], json!(["hierarchy of `Sales` `Geo`"]));
    run(&mut s, "file.save", json!({"backup": false}));
    if let Some(out) = bicraft_env::var_os("BICRAFT_FRESH_OUT") {
        let out = std::path::Path::new(&out);
        let _ = std::fs::create_dir_all(out);
        let _ = std::fs::copy(&path, out.join("persp_removed.pbix"));
    }
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": path}));
    assert_eq!(back.model_error, None);
    let p = lite(&back);
    let names = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x["name"].as_str().map(str::to_string))
            .collect()
    };
    assert_eq!(names(&p["tables"]), ["Sales"], "{p}");
    assert_eq!(names(&p["tables"][0]["measures"]), ["Count"], "{p}");
    assert!(names(&p["tables"][0]["hierarchies"]).is_empty(), "{p}");
    assert_eq!(names(&p["tables"][0]["columns"]), ["Amount", "Region"], "{p}");
}
