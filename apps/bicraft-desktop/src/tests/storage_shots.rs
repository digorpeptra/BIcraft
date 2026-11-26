//! Screenshots of storage mode in the Model view drawn with wgpu (`--features gpu-tests`), saved
//! into `BICRAFT_DESKTOP_SHOTS` when set: the diagram's mode badges with the Properties pane's
//! Storage mode list, and the switch-to-Import confirmation, in English light, French and dark.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use egui_kittest::Harness;
use serde_json::{Value, json};

use super::Temp;
use super::modelsec_shots::{open, steps};
use crate::App;

fn save(h: &mut Harness<'_, App>, dir: &Option<PathBuf>, name: &str) {
    let img = h.render().unwrap();
    assert!(img.width() > 0 && img.height() > 0);
    let Some(dir) = dir else { return };
    std::fs::create_dir_all(dir).unwrap();
    let size = tiny_skia::IntSize::from_wh(img.width(), img.height()).unwrap();
    let pm = tiny_skia::Pixmap::from_vec(img.as_raw().clone(), size).unwrap();
    pm.save_png(dir.join(format!("{name}.png"))).unwrap();
}

const SQL: &str =
    "let Source = Sql.Database(\"srv.example\", \"db\"), T = Source{[Schema=\"dbo\",Item=\"T\"]}[Data] in T";

/// Sales (DirectQuery) → Product (Dual) → Category (DirectQuery), Region (Import, entered).
pub(super) fn composite(dir: &Path, name: &str, extra: Vec<(&str, Value)>) -> PathBuf {
    let mut s = Session::new();
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    run(&mut s, "model.new", json!({"name": "Shop"}));
    for (table, cols, mode) in [
        ("Sales", &["ProductKey", "Amount"][..], "directQuery"),
        ("Product", &["ProductKey", "CategoryKey"][..], "dual"),
        ("Category", &["CategoryKey"][..], "directQuery"),
    ] {
        run(
            &mut s,
            "table.add",
            json!({"name": table, "partition": {"source_type": "m", "expression": SQL}}),
        );
        for c in cols {
            run(
                &mut s,
                "column.add",
                json!({"table": table, "name": c, "data_type": "int64"}),
            );
        }
        run(
            &mut s,
            "table.set_storage_mode",
            json!({"table": table, "mode": mode}),
        );
    }
    for (id, p) in [
        (
            "relationship.add",
            json!({"from": "Sales[ProductKey]", "to": "Product[ProductKey]"}),
        ),
        (
            "relationship.add",
            json!({"from": "Product[CategoryKey]", "to": "Category[CategoryKey]"}),
        ),
        (
            "table.import",
            json!({"name": "Region", "m": "#table(type table [Region = text, Manager = text], {{\"E\", \"a\"}, {\"W\", \"b\"}})"}),
        ),
        ("report.new", json!({})),
    ]
    .into_iter()
    .chain(extra)
    {
        run(&mut s, id, p);
    }
    let path = dir.join(name);
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

#[test]
fn storage_mode_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("storemode-shots");
    let path = composite(dir.path(), "storemode.pbip", Vec::new());
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let model = if lang == "fr-FR" { "Modèle" } else { "Model" };
        let mut h = open(&path, lang, dark, model);
        h.state_mut().model_view.sel = Some(bicraft_ui::model_view::Sel::Table("Sales".into()));
        steps(&mut h);
        steps(&mut h);
        save(&mut h, &shots, &format!("storemode-diagram-{suffix}"));
        h.state_mut().cmd.storage = Some(bicraft_ui::storage_ui::Confirm {
            table: "Sales".into(),
            candidates: vec!["Category".into()],
            dual: true,
        });
        steps(&mut h);
        save(&mut h, &shots, &format!("storemode-confirm-{suffix}"));
    }
}

const PARAM: &str = "#datetime(2024, 1, 1, 0, 0, 0) meta [IsParameterQuery=true, Type=\"DateTime\", IsParameterQueryRequired=true]";
const FILTERED: &str = "let Source = Sql.Database(\"srv.example\", \"db\"), Orders = Source{[Schema=\"dbo\",Item=\"Orders\"]}[Data], Filtered = Table.SelectRows(Orders, each [OrderDate] >= RangeStart and [OrderDate] < RangeEnd) in Filtered";

/// Orders (Import, filtered on RangeStart and RangeEnd), with the two parameters.
fn incremental(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    run(&mut s, "model.new", json!({"name": "Shop"}));
    for n in ["RangeStart", "RangeEnd"] {
        run(&mut s, "expression.add", json!({"name": n, "expression": PARAM}));
    }
    run(
        &mut s,
        "table.add",
        json!({"name": "Orders", "partition": {"source_type": "m", "expression": FILTERED}}),
    );
    for (c, ty) in [
        ("OrderDate", "dateTime"),
        ("Modified", "dateTime"),
        ("Amount", "int64"),
    ] {
        run(
            &mut s,
            "column.add",
            json!({"table": "Orders", "name": c, "data_type": ty}),
        );
    }
    run(&mut s, "report.new", json!({}));
    let path = dir.join("refresh.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

#[test]
fn refresh_policy_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("storemode-refresh-shots");
    let path = incremental(dir.path());
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let model = if lang == "fr-FR" { "Modèle" } else { "Model" };
        let mut h = open(&path, lang, dark, model);
        bicraft_ui::refresh_policy_dialog::open(h.state_mut(), "Orders");
        if let Some(d) = h.state_mut().cmd.refresh_policy.as_mut() {
            d.on = true;
            d.complete = true;
            d.detect = true;
            d.detect_column = Some("Modified".into());
        }
        steps(&mut h);
        steps(&mut h);
        save(&mut h, &shots, &format!("storemode-refresh-{suffix}"));
    }
}
