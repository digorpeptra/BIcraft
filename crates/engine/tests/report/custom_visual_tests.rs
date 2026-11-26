//! `visual.import_custom` and `visual.remove_custom` (docs/formats/pbiviz.md): a `.pbiviz`
//! package our test builds (never a downloaded visual) is registered in the report, listed by
//! `report.custom_visuals` and the snapshot with its icon, kept through PBIP and PBIX round
//! trips, replaced and removed; hostile packages and parameters are refused without a change.

use bicraft_opc::Package;
use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

const GUID: &str = "bicraftTiny7C3E9A41B2D54F60";
/// A 1×1 PNG, the visual's icon.
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Runs a command that must fail and checks it left the report and its dirty flag alone.
fn fails(s: &mut Session, id: &str, params: Value) -> String {
    let before = s.report.clone();
    let dirty = s.report_dirty;
    let err = s
        .execute(id, &params)
        .expect_err(&format!("{id} {params} should fail"));
    assert_eq!(s.report, before, "{id} changed the report although it failed");
    assert_eq!(s.report_dirty, dirty);
    err
}

fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Our own tiny .pbiviz (the public layout), `version` in its manifest.
fn pbiviz(version: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
    let block =
        json!({"name": "bicraftTiny", "displayName": "Tiny visual", "guid": GUID, "version": version});
    let manifest = json!({
        "build": "6.1.0", "version": version, "visual": block, "author": {"name": "BIcraft tests"},
        "resources": [{"resourceId": "rId0", "sourceType": 5, "file": format!("resources/{GUID}.pbiviz.json")}]
    });
    let visual = json!({
        "visual": block, "apiVersion": "5.3.0", "author": {"name": "BIcraft tests"},
        "capabilities": {"dataRoles": [{"name": "value", "kind": "Measure", "displayName": "Value"}]},
        "content": {"js": "var bicraftTiny = 1;", "css": ".tiny{}", "iconBase64": format!("data:image/png;base64,{PNG}")}
    });
    let mut p = Package::default();
    p.put("package.json", manifest.to_string().into_bytes(), false);
    p.put(
        &format!("resources/{GUID}.pbiviz.json"),
        visual.to_string().into_bytes(),
        false,
    );
    for (n, b) in extra {
        p.put(n, b.to_vec(), false);
    }
    p.to_bytes().unwrap()
}

fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "m": "let Source = #table(type table [Region = text, Amount = Int64.Type], {{\"East\", 5}}) in Source"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
    );
    run(&mut s, "report.new", json!({}));
    s
}

fn page(s: &Session) -> String {
    s.report.as_ref().unwrap().pages[0].name.clone()
}

fn resources(s: &Session) -> Vec<(String, Vec<u8>)> {
    let mut v: Vec<(String, Vec<u8>)> = s
        .report
        .as_ref()
        .unwrap()
        .resources
        .iter()
        .filter(|r| r.path.starts_with("CustomVisuals/"))
        .map(|r| (r.path.clone(), r.bytes.clone()))
        .collect();
    v.sort();
    v
}

#[test]
fn imports_lists_snapshots_and_round_trips() {
    let mut s = session();
    let out = run(
        &mut s,
        "visual.import_custom",
        json!({"data": b64(&pbiviz("1.0.0.0", &[("readme.md", b"x")]))}),
    );
    assert_eq!(out["name"], GUID);
    assert_eq!(out["display_name"], "Tiny visual");
    assert_eq!(out["api_version"], "5.3.0");
    assert_eq!(out["has_icon"], true);
    assert_eq!(out["replaced"], false);
    assert_eq!(out["skipped"], json!(["readme.md"]));
    assert!(s.report_dirty);
    // stored as Desktop stores an embedded visual: the package and its CustomVisual entry
    let r = s.report.as_ref().unwrap();
    assert_eq!(
        resources(&s).iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(),
        [
            format!("CustomVisuals/{GUID}/package.json"),
            format!("CustomVisuals/{GUID}/resources/{GUID}.pbiviz.json")
        ]
    );
    let pkg = r.extra["resourcePackages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["type"] == "CustomVisual")
        .unwrap();
    // the item is named as its file: Power BI Desktop 2.159.1231.0's save writes an item's name as
    // its path, so an item named by the guid alone became `path: <guid>` and the visual was gone
    // when the file was reopened (2026-11-23)
    assert_eq!(
        pkg,
        &json!({"name": GUID, "type": "CustomVisual",
                "items": [{"name": format!("{GUID}.pbiviz.json"), "path": format!("{GUID}.pbiviz.json"), "type": "CustomVisualMetadata"}]})
    );
    let listed = run(&mut s, "report.custom_visuals", json!({}));
    let e = &listed["embedded"][0];
    assert_eq!(e["registered"], true, "{listed}");
    assert_eq!(e["issues"], json!([]), "{listed}");
    assert_eq!(e["display_name"], "Tiny visual");
    // the snapshot: the Visualizations pane's entry with its icon, served as an asset
    let snap = s.snapshot();
    let cv = &snap["report"]["custom_visuals"][0];
    assert_eq!(cv["name"], GUID);
    assert_eq!(cv["display_name"], "Tiny visual");
    let icon = cv["icon"]["id"].as_str().unwrap().to_string();
    assert_eq!(cv["icon"]["mime"], "image/png");
    assert!(s.snapshot_assets().iter().any(|a| a.id == icon));
    // a visual of its type carries the entry (drawn as a placeholder with the icon)
    let p = page(&s);
    let v = run(
        &mut s,
        "visual.add",
        json!({"page": p, "visual_type": GUID, "x": 10, "y": 10, "width": 200, "height": 120}),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string();
    let snap = s.snapshot();
    let sv = snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == v.as_str())
        .unwrap()
        .clone();
    assert_eq!(sv["custom_visual"]["icon"]["id"], icon.as_str());
    let before = resources(&s);
    for file in ["Shop.pbip", "Shop.pbix"] {
        let dir = TempDir::new(&format!("engine-cv-{file}"));
        run(&mut s, "file.save_as", json!({"path": dir.join(file)}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": dir.join(file)}));
        assert_eq!(
            resources(&back),
            before,
            "{file}: the package's files byte for byte"
        );
        let l = run(&mut back, "report.custom_visuals", json!({}));
        assert_eq!(l["embedded"][0]["registered"], true, "{file}: {l}");
        assert_eq!(l["embedded"][0]["issues"], json!([]), "{file}: {l}");
        assert_eq!(l["embedded"][0]["used_by"][0]["visual"], v.as_str(), "{file}");
        assert_eq!(back.snapshot()["report"]["custom_visuals"][0]["name"], GUID);
        let valid = run(&mut back, "report.validate", json!({}));
        assert!(
            !valid.to_string().contains("CustomVisuals"),
            "{file}: the validator finds the item's file: {valid}"
        );
    }
}

#[test]
fn reimport_replaces_and_remove_needs_force_while_used() {
    let mut s = session();
    run(
        &mut s,
        "visual.import_custom",
        json!({"data": b64(&pbiviz("1.0.0.0", &[]))}),
    );
    let out = run(
        &mut s,
        "visual.import_custom",
        json!({"data": format!("data:application/octet-stream;base64,{}", b64(&pbiviz("2.0.0.0", &[])))}),
    );
    assert_eq!(out["replaced"], true);
    assert_eq!(out["version"], "2.0.0.0");
    let r = s.report.as_ref().unwrap();
    let packages = r.extra["resourcePackages"].as_array().unwrap();
    assert_eq!(packages.iter().filter(|p| p["type"] == "CustomVisual").count(), 1);
    assert_eq!(resources(&s).len(), 2);
    let p = page(&s);
    run(
        &mut s,
        "visual.add",
        json!({"page": p, "visual_type": GUID, "x": 0, "y": 0, "width": 100, "height": 100}),
    );
    let e = fails(&mut s, "visual.remove_custom", json!({"name": GUID}));
    assert!(e.contains("force"), "{e}");
    let out = run(
        &mut s,
        "visual.remove_custom",
        json!({"name": GUID.to_lowercase(), "force": true}),
    );
    assert_eq!(out["removed_package"], true);
    assert_eq!(out["removed_files"], 2);
    assert_eq!(out["kept_visuals"].as_array().unwrap().len(), 1);
    assert!(resources(&s).is_empty());
    assert_eq!(
        run(&mut s, "report.custom_visuals", json!({}))["embedded"],
        json!([])
    );
    assert_eq!(s.snapshot()["report"]["custom_visuals"], json!([]));
    // undo brings it back
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(resources(&s).len(), 2);
    let e = fails(&mut s, "visual.remove_custom", json!({"name": "nothing"}));
    assert!(e.contains("no custom visual"), "{e}");
}

#[test]
fn hostile_packages_and_parameters_are_refused() {
    let mut s = Session::new();
    assert!(
        s.execute("visual.import_custom", &json!({"data": b64(&pbiviz("1", &[]))}))
            .unwrap_err()
            .contains("report"),
        "no report open"
    );
    let mut s = session();
    for p in [
        json!({}),
        json!({"data": 3}),
        json!({"data": "@@@"}),
        json!({"data": "", "path": ""}),
        json!({"data": "QQ==", "path": "x.pbiviz"}),
        json!({"data": b64(b"not a zip")}),
        json!({"path": "does-not-exist.pbiviz"}),
        json!({"data": "QQ==", "other": 1}),
        json!({"data": "A".repeat(50 << 20)}),
    ] {
        fails(&mut s, "visual.import_custom", p);
    }
    // a guid that would leave its folder, and a ZIP bomb
    let mut p = Package::default();
    p.put(
        "package.json",
        br#"{"visual": {"guid": "..\\..\\evil"}}"#.to_vec(),
        false,
    );
    let e = fails(
        &mut s,
        "visual.import_custom",
        json!({"data": b64(&p.to_bytes().unwrap())}),
    );
    assert!(e.contains("guid"), "{e}");
    let bomb = vec![b'a'; 4 << 20];
    let e = fails(
        &mut s,
        "visual.import_custom",
        json!({"data": b64(&pbiviz("1", &[("resources/bomb.js", &bomb)]))}),
    );
    assert!(e.contains("ZIP bomb"), "{e}");
    for p in [
        json!({}),
        json!({"name": 1}),
        json!({"name": ""}),
        json!({"name": GUID, "force": "yes"}),
    ] {
        fails(&mut s, "visual.remove_custom", p);
    }
}

#[test]
fn imports_from_a_file() {
    let dir = TempDir::new("engine-cv-file");
    let path = dir.join("tiny.pbiviz");
    std::fs::write(&path, pbiviz("1.0.0.0", &[])).unwrap();
    let mut s = session();
    let out = run(&mut s, "visual.import_custom", json!({"path": path}));
    assert_eq!(out["name"], GUID);
}
