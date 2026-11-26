//! Opening an Excel Power Pivot workbook from the app (M11.33): the File view's Open dialog picks a
//! `.xlsx`, the window shows its imported model; a workbook without a data model is an error.

use std::path::{Path, PathBuf};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::{Temp, harness, wait};
use crate::App;
use bicraft_ui::app::Tab;

/// The committed synthetic workbook of `bicraft-powerpivot` (Product, Sales; two measures).
pub const SALES_XLSX: &[u8] = include_bytes!("../../../../crates/powerpivot/tests/fixtures/sales.xlsx");

pub fn workbook(dir: &Path) -> PathBuf {
    let path = dir.join("Sales book.xlsx");
    std::fs::write(&path, SALES_XLSX).unwrap();
    path
}

/// The app whose Open dialog answers `path`, after the File view's "Open a file…" (Ctrl+O once a
/// file is open).
pub fn open_from_file_view(path: &Path, lang: &str, size: (f32, f32)) -> Harness<'static, App> {
    let mut app = crate::app(lang, None).with_sync_render();
    let picked = path.to_path_buf();
    app.shell.pick_file = Some(Box::new(move |_| Some(picked.clone())));
    let mut h = harness(app, size);
    h.step();
    h.step();
    click_open(&mut h, lang);
    h
}

/// Clicks the File view's "Open a file…".
pub fn click_open(h: &mut Harness<'_, App>, lang: &str) {
    let label = if lang.starts_with("fr") {
        "Ouvrir un fichier…"
    } else {
        "Open a file…"
    };
    h.get_by_label(label).click();
    h.step();
}

#[test]
fn the_open_dialog_opens_a_power_pivot_workbook() {
    let dir = Temp::new("workbook-open");
    let path = workbook(dir.path());
    let mut h = open_from_file_view(&path, "en-US", (1280.0, 800.0));
    wait(&mut h, "the snapshot", |a| a.loaded.is_some());
    assert_eq!(h.state().error, None);
    assert_eq!(h.state().title(), "Sales book.xlsx · rev 0");
    let model = h.state().loaded.as_ref().unwrap().snap["model"].clone();
    let tables: Vec<&str> = model["tables"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(
        tables.contains(&"Product") && tables.contains(&"Sales"),
        "{model}"
    );
    h.state_mut().tab = Tab::Model;
    wait(&mut h, "the diagram", |a| a.model_view.diagram().is_some());
    let d = h.state().model_view.diagram().unwrap();
    assert!(d.card_named("Sales").is_some() && d.card_named("Product").is_some());
}

#[test]
fn a_workbook_without_a_data_model_shows_an_error() {
    let dir = Temp::new("workbook-none");
    let path = dir.path().join("plain.xlsx");
    // an OPC package with xl/workbook.xml and no xl/model/item.data
    let pkg = bicraft_opc::Package {
        parts: vec![bicraft_opc::Part::new(
            "xl/workbook.xml",
            b"<workbook/>".to_vec(),
            false,
        )],
    };
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    let mut h = open_from_file_view(&path, "en-US", (900.0, 600.0));
    wait(&mut h, "the error", |a| a.error.is_some());
    let err = h.state().error.clone().unwrap();
    assert!(err.contains("no Power Pivot data model"), "{err}");
}
