//! Custom visuals in the Visualizations pane (`custom_visuals.rs`, docs/formats/pbiviz.md) on an
//! engine session: "â€¦" â€º Import a visual from a file through the shell's upload (a stub that hands
//! over a tiny `.pbiviz` the test builds), the gallery's cell with the package's icon adding a
//! visual of that type, Insert â€º Import a visual, "â€¦" â€º Remove a custom visual and its
//! confirmation; the browser's upload, which answers later; cancelled, failed and too large files.

use std::path::Path;

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use crate::App;
use crate::shell::{Caps, Picked, UploadSlot};

pub(crate) const GUID: &str = "bicraftTiny7C3E9A41B2D54F60";

/// A 20Ã—20 icon of our own: a blue square with an orange bar.
pub(crate) fn icon_png() -> Vec<u8> {
    let mut pm = tiny_skia::Pixmap::new(20, 20).unwrap();
    pm.fill(tiny_skia::Color::from_rgba8(17, 141, 255, 255));
    let bar = tiny_skia::Rect::from_xywh(3.0, 12.0, 14.0, 4.0).unwrap();
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(255, 140, 0, 255);
    pm.fill_rect(bar, &paint, tiny_skia::Transform::identity(), None);
    pm.encode_png().unwrap()
}

/// Our own tiny `.pbiviz` (the public layout: package.json and resources/<guid>.pbiviz.json).
pub(crate) fn pbiviz(display: &str) -> Vec<u8> {
    let block = json!({"name": "bicraftTiny", "displayName": display, "guid": GUID, "version": "1.0.0.0"});
    let manifest = json!({
        "visual": block, "author": {"name": "BIcraft tests"},
        "resources": [{"resourceId": "rId0", "sourceType": 5, "file": format!("resources/{GUID}.pbiviz.json")}]
    });
    let visual = json!({
        "visual": block, "apiVersion": "5.3.0",
        "capabilities": {"dataRoles": [{"name": "value", "kind": "Measure"}]},
        "content": {"js": "var tiny = 1;", "css": "",
                    "iconBase64": format!("data:image/png;base64,{}", crate::rich::base64(&icon_png()))}
    });
    let mut p = bicraft_opc::Package::default();
    p.put("package.json", manifest.to_string().into_bytes(), false);
    p.put(
        &format!("resources/{GUID}.pbiviz.json"),
        visual.to_string().into_bytes(),
        false,
    );
    p.to_bytes().unwrap()
}

fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.import",
            json!({"name": "Sales", "m": "#table(type table [Region = text, Amount = number], {{\"E\", 10}})"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap();
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn open_with(caps: Caps, setup: impl FnOnce(&mut App)) -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(caps, session)).with_sync_render();
    setup(&mut app);
    app.open_path(Path::new("shop.pbip"));
    // tall enough for the gallery to show its "…" cell after BIcraft's visuals and an import
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            |ui, app: &mut App| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        );
    steps(&mut h, 6);
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().placed.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    h
}

/// An upload stub that hands over `bytes` named `name` at once.
fn giving(app: &mut App, bytes: Vec<u8>) {
    app.shell.upload = Some(Box::new(move |_, _, exts, _, slot| {
        assert_eq!(exts, ["pbiviz"]);
        *slot.borrow_mut() = Some(Picked::File {
            name: "tiny.pbiviz".into(),
            bytes: bytes.clone(),
        });
    }));
}

fn until(h: &mut Harness<'_, App>, what: &str, ok: impl Fn(&App) -> bool) {
    for _ in 0..400 {
        if ok(h.state()) {
            return;
        }
        h.step();
    }
    panic!("never: {what}");
}

fn customs(app: &App) -> Vec<Value> {
    app.loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/custom_visuals"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// The gallery's "â€¦" cell, then menu item `item`.
fn more(h: &mut Harness<'_, App>, item: &str) {
    h.get_by_label("More visuals").click();
    steps(h, 3);
    h.get_by_label(item).click();
    steps(h, 3);
}

#[test]
fn import_a_visual_from_a_file_adds_its_gallery_cell() {
    let mut h = open_with(Caps::DESKTOP, |app| giving(app, pbiviz("Tiny visual")));
    assert!(h.query_by_label("Tiny visual").is_none());
    more(&mut h, "Import a visual from a file");
    until(&mut h, "the custom visual", |a| !customs(a).is_empty());
    let c = h.state().sent.last().unwrap();
    assert_eq!(c.id, "visual.import_custom");
    assert_eq!(c.params["data"], crate::rich::base64(&pbiviz("Tiny visual")));
    steps(&mut h, 3);
    // its cell, with the package's icon (decoded from the snapshot's asset)
    let icon = customs(h.state())[0]["icon"]["id"].as_str().unwrap().to_string();
    assert!(h.state().loaded.as_ref().unwrap().assets.contains_key(&icon));
    assert!(
        h.state().cmd.custom_visuals.has_icon(&icon),
        "the icon is a texture"
    );
    // a click adds a visual of its type, drawn as its placeholder
    h.get_by_label("Tiny visual").click();
    steps(&mut h, 5);
    let c = h.state().sent.last().unwrap();
    assert_eq!(
        (c.id, c.params["visual_type"].as_str()),
        ("visual.add", Some(GUID))
    );
    until(&mut h, "the visual", |a| a.cmd.visual.is_some());
    // Insert â€º Import a visual runs the same upload; the same guid replaces it
    h.state_mut().cmd.visual = None;
    steps(&mut h, 2);
    giving(h.state_mut(), pbiviz("Tiny visual 2"));
    h.get_all_by_label("Insert")
        .min_by(|a, b| a.rect().center().y.total_cmp(&b.rect().center().y))
        .unwrap()
        .click();
    steps(&mut h, 3);
    h.get_by_label("Import a visual").click();
    until(&mut h, "the new version", |a| {
        customs(a)
            .first()
            .is_some_and(|c| c["display_name"] == "Tiny visual 2")
    });
    assert_eq!(customs(h.state()).len(), 1);
}

#[test]
fn remove_a_custom_visual_asks_when_visuals_use_it() {
    let mut h = open_with(Caps::DESKTOP, |app| giving(app, pbiviz("Tiny visual")));
    // nothing to remove yet: the item is disabled
    h.get_by_label("More visuals").click();
    steps(&mut h, 3);
    assert!(
        h.get_by_label("Remove a custom visual")
            .accesskit_node()
            .is_disabled()
    );
    h.get_by_label("Import a visual from a file").click();
    until(&mut h, "the custom visual", |a| !customs(a).is_empty());
    steps(&mut h, 3);
    h.get_by_label("Tiny visual").click();
    until(&mut h, "its visual", |a| a.cmd.visual.is_some());
    h.state_mut().cmd.visual = None;
    steps(&mut h, 3);
    more(&mut h, "Remove a custom visual");
    assert!(h.state().cmd.custom_visuals.remove.is_some());
    h.get_by_label("Remove Tiny visual").click();
    steps(&mut h, 3);
    // a visual uses it: nothing sent before the confirmation
    assert_ne!(h.state().sent.last().unwrap().id, "visual.remove_custom");
    assert!(h.query_by_label_contains("drawn as placeholders").is_some());
    h.get_by_label("Remove it, keep the visuals").click();
    until(&mut h, "the removal", |a| customs(a).is_empty());
    let c = h.state().sent.last().unwrap();
    assert_eq!(c.id, "visual.remove_custom");
    assert_eq!(c.params, json!({"name": GUID, "force": true}));
    steps(&mut h, 3);
    assert!(h.query_by_label("Tiny visual").is_none(), "its cell is gone");
    h.get_by_label("Close").click();
    steps(&mut h, 2);
    assert!(h.state().cmd.custom_visuals.remove.is_none());
}

#[test]
fn the_browser_upload_answers_later_and_bad_files_send_nothing() {
    // the browser's file input: the slot is filled once the page has read the file
    let held: std::rc::Rc<std::cell::RefCell<Option<UploadSlot>>> = Default::default();
    let keep = held.clone();
    let mut h = open_with(Caps::BROWSER, move |app| {
        app.shell.upload = Some(Box::new(move |_, _, _, _, slot| {
            *keep.borrow_mut() = Some(slot);
        }));
    });
    more(&mut h, "Import a visual from a file");
    let n = h.state().sent.len();
    steps(&mut h, 5);
    assert_eq!(h.state().sent.len(), n, "nothing until the file is read");
    let slot = held.borrow_mut().take().unwrap();
    *slot.borrow_mut() = Some(Picked::File {
        name: "tiny.pbiviz".into(),
        bytes: pbiviz("Tiny visual"),
    });
    until(&mut h, "the custom visual", |a| !customs(a).is_empty());
    // cancelled, unreadable, too large: nothing sent
    for picked in [
        Picked::Cancelled,
        Picked::Failed("tiny.pbiviz: cannot be read".into()),
        Picked::File {
            name: "big.pbiviz".into(),
            bytes: vec![0u8; crate::custom_visuals::MAX_PBIVIZ_BYTES + 1],
        },
    ] {
        let failed = !matches!(picked, Picked::Cancelled);
        h.state_mut().error = None;
        h.state_mut().shell.upload = Some(Box::new(move |_, _, _, _, slot| {
            *slot.borrow_mut() = Some(picked.clone());
        }));
        let n = h.state().sent.len();
        crate::custom_visuals::import(h.state_mut());
        steps(&mut h, 3);
        assert_eq!(h.state().sent.len(), n);
        assert_eq!(h.state().error.is_some(), failed, "{:?}", h.state().error);
    }
    // a package the engine refuses: its error in the error bar
    h.state_mut().error = None;
    giving(h.state_mut(), b"not a zip".to_vec());
    crate::custom_visuals::import(h.state_mut());
    until(&mut h, "the engine's error", |a| {
        a.error.as_deref().is_some_and(|e| e.contains("pbiviz"))
    });
}

/// Clicks the Insert tab, then its button `label`, and checks that command's form opened.
fn insert_form(h: &mut Harness<'_, App>, label: &str, id: &str) {
    if h.query_by_label(label).is_some() {
        // the Insert tab is open already (a second click would fold the ribbon)
        h.get_by_label(label).click();
        steps(h, 3);
        assert_eq!(h.state().form.as_ref().map(|f| f.id), Some(id), "{label}");
        return;
    }
    h.get_all_by_label("Insert")
        .min_by(|a, b| a.rect().center().y.total_cmp(&b.rect().center().y))
        .unwrap()
        .click();
    steps(h, 3);
    h.get_by_label(label).click();
    steps(h, 3);
    assert_eq!(h.state().form.as_ref().map(|f| f.id), Some(id), "{label}");
}

/// Fills form fields (their text) and runs the form; waits for the engine's answer.
fn run_form(h: &mut Harness<'_, App>, fields: &[(&str, &str)]) -> Value {
    {
        let f = h.state_mut().form.as_mut().unwrap();
        for (name, text) in fields {
            f.input_mut(name).unwrap().text = (*text).to_string();
        }
    }
    h.get_by_label("Run").click();
    for _ in 0..400 {
        if h.state().form.as_ref().is_some_and(|f| f.result.is_some()) {
            break;
        }
        h.step();
    }
    let f = h.state().form.as_ref().unwrap();
    match &f.result {
        Some(Ok(v)) => v.clone(),
        other => panic!("{}: {other:?}", f.id),
    }
}

/// Insert › New visual project, Compile a visual and Package a visual: each opens its form and runs
/// its command through the engine (M11.30); the package then imports through "…" › Import a
/// visual from a file and gets its gallery cell.
#[test]
fn insert_new_build_and_package_a_visual_project() {
    let dir = std::env::temp_dir().join(format!("bicraft-ui-visual-project-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let proj = dir.join("bars");
    let folder = proj.display().to_string();
    let mut h = open_with(Caps::DESKTOP, |_| {});
    insert_form(&mut h, "New visual project", "visual.scaffold");
    let out = run_form(
        &mut h,
        &[
            ("folder", folder.as_str()),
            ("name", "uiBars"),
            ("display_name", "UI bars"),
            ("data_roles", "region:Grouping\nsales:Measure"),
            ("format_options", "bars.fill:color"),
        ],
    );
    let c = h.state().sent.last().unwrap();
    assert_eq!(c.id, "visual.scaffold");
    assert_eq!(
        c.params["data_roles"],
        json!(["region:Grouping", "sales:Measure"])
    );
    assert!(proj.join("pbiviz.json").exists(), "{out}");
    h.state_mut().form = None;
    steps(&mut h, 2);
    insert_form(&mut h, "Compile a visual", "visual.build");
    let built = run_form(&mut h, &[("folder", folder.as_str())]);
    assert_eq!(built["entry"], "src/visual.ts");
    assert!(proj.join("dist/visual.js").exists());
    h.state_mut().form = None;
    steps(&mut h, 2);
    insert_form(&mut h, "Package a visual", "visual.pack");
    let packed = run_form(&mut h, &[("folder", folder.as_str())]);
    let file = std::path::PathBuf::from(packed["out"].as_str().unwrap());
    let bytes = std::fs::read(&file).unwrap();
    h.state_mut().form = None;
    steps(&mut h, 2);
    giving(h.state_mut(), bytes);
    more(&mut h, "Import a visual from a file");
    until(&mut h, "the packaged visual", |a| !customs(a).is_empty());
    steps(&mut h, 3);
    assert!(h.query_by_label("UI bars").is_some(), "its gallery cell");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Insert › Export a BIcraft visual (ROADMAP M12.5): its form writes the chosen visual's `.pbiviz`
/// through the engine, which then imports through "…" › Import a visual from a file.
#[test]
fn insert_export_a_bicraft_visual() {
    let dir = std::env::temp_dir().join(format!("bicraft-ui-visual-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let file = dir.join("venn.pbiviz");
    let mut h = open_with(Caps::DESKTOP, |_| {});
    insert_form(&mut h, "Export a BIcraft visual", "visual.export_bicraft_visual");
    let out = run_form(
        &mut h,
        &[
            ("name", "bicraftVenn"),
            ("out", file.display().to_string().as_str()),
        ],
    );
    assert_eq!(out["count"], 1, "{out}");
    let bytes = std::fs::read(&file).unwrap();
    h.state_mut().form = None;
    steps(&mut h, 2);
    giving(h.state_mut(), bytes);
    more(&mut h, "Import a visual from a file");
    until(&mut h, "the exported visual", |a| {
        bicraft_visuals(a)
            .iter()
            .any(|b| b["name"] == "bicraftVenn" && b["embedded"] == true)
    });
    let _ = std::fs::remove_dir_all(&dir);
}

fn bicraft_visuals(app: &App) -> Vec<Value> {
    app.loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/bicraft_visuals"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// BIcraft's visuals are built in (ROADMAP M12.4): the BIcraft Spec cell shows on a report that
/// embeds nothing, with its icon (drawn in the theme's ink, not decoded from the package's PNG,
/// which is the same drawing); a click adds one by its name, which embeds the package, and the
/// cell stays single (not listed again as a custom visual).
#[test]
fn bicraft_visuals_are_in_the_gallery_before_any_import() {
    let mut h = open_with(Caps::DESKTOP, |_| {});
    let spec = bicraft_visuals(h.state())
        .into_iter()
        .find(|b| b["name"] == "bicraftSpec")
        .unwrap();
    assert_eq!(spec["embedded"], false);
    assert!(customs(h.state()).is_empty(), "nothing embedded yet");
    let icon = spec["icon"]["id"].as_str().unwrap().to_string();
    assert!(h.state().loaded.as_ref().unwrap().assets.contains_key(&icon));
    assert!(
        !h.state().cmd.custom_visuals.has_icon(&icon),
        "its icon is drawn, not a texture"
    );
    h.get_by_label("BIcraft Spec").click();
    steps(&mut h, 5);
    let c = h.state().sent.last().unwrap();
    assert_eq!(
        (c.id, c.params["visual_type"].as_str()),
        ("visual.add", Some("bicraftSpec"))
    );
    until(&mut h, "the package", |a| !customs(a).is_empty());
    until(&mut h, "the visual", |a| a.cmd.visual.is_some());
    let guid = spec["guid"].as_str().unwrap();
    assert_eq!(customs(h.state())[0]["name"], guid);
    assert_eq!(bicraft_visuals(h.state())[0]["embedded"], true);
    steps(&mut h, 3);
    assert_eq!(h.get_all_by_label("BIcraft Spec").count(), 1, "one cell");
}
