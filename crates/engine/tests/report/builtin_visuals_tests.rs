//! BIcraft's visuals built in by default (ROADMAP M12.4, `builtin_visuals.rs`): `visual.add` and
//! `visual.set_type` take their name and embed the package on first use, a newer build replaces
//! an older one and an embedded newer one is kept, `report.custom_visuals` and the snapshot report
//! them, and a save drops the package of one the report no longer uses.

use bicraft_engine::Session;
use bicraft_engine::spec_visual::{SPEC_GUID, SPEC_PACKAGE_VERSION};
use serde_json::{Value, json};

use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "T", "m": "#table(type table [K = text, V = Int64.Type], {{\"a\", 1}, {\"b\", 2}})"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "T", "name": "Total", "expression": "SUM(T[V])"}),
    );
    run(&mut s, "report.new", json!({}));
    s
}

fn page(s: &Session) -> String {
    s.report.as_ref().unwrap().pages[0].name.clone()
}

fn add(s: &mut Session, visual_type: &str, fields: Value) -> Value {
    let p = page(s);
    run(
        s,
        "visual.add",
        json!({"page": p, "visual_type": visual_type, "x": 10, "y": 10, "width": 300, "height": 200, "fields": fields}),
    )
}

fn spec_entry(s: &mut Session) -> Value {
    let l = run(s, "report.custom_visuals", json!({}));
    l["bicraft"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["guid"] == SPEC_GUID)
        .cloned()
        .unwrap_or_else(|| panic!("{l}"))
}

fn metadata_path() -> String {
    format!("CustomVisuals/{SPEC_GUID}/resources/{SPEC_GUID}.pbiviz.json")
}

/// Rewrites the version inside the embedded `.pbiviz.json` (a build of another BIcraft).
fn set_embedded_version(s: &mut Session, version: &str) {
    let r = s.report.as_mut().unwrap();
    let res = r
        .resources
        .iter_mut()
        .find(|x| x.path == metadata_path())
        .unwrap();
    let mut doc: Value = serde_json::from_slice(&res.bytes).unwrap();
    doc["visual"]["version"] = json!(version);
    res.bytes = doc.to_string().into_bytes();
}

#[test]
fn visual_add_takes_the_name_and_embeds_the_package_once() {
    let mut s = session();
    let before = spec_entry(&mut s);
    assert_eq!(before["name"], "bicraftSpec");
    assert_eq!(before["embedded"], false);
    assert_eq!(before["version"], SPEC_PACKAGE_VERSION);
    let snap = s.snapshot();
    let cell = &snap["report"]["bicraft_visuals"][0];
    assert_eq!(cell["guid"], SPEC_GUID, "{cell}");
    assert_eq!(cell["display_name"], "BIcraft Spec");
    assert_eq!(cell["embedded"], false);
    assert!(cell["icon"]["id"].is_string(), "{cell}");
    // the role names are the visual's own, whatever their case
    let out = add(
        &mut s,
        "BICRAFTSPEC",
        json!([{"role": "Category", "field": "T[K]"}, {"role": "VALUES", "field": "[Total]"}]),
    );
    assert_eq!(out["visual_type"], SPEC_GUID, "{out}");
    assert_eq!(out["installed"], true);
    assert_eq!(out["warnings"], json!([]), "{out}");
    let roles: Vec<&str> = out["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["category", "values"], "{out}");
    let after = spec_entry(&mut s);
    assert_eq!(after["embedded"], true, "{after}");
    assert_eq!(after["embedded_version"], SPEC_PACKAGE_VERSION);
    assert_eq!(after["current"], true);
    assert_eq!(after["used_by"].as_array().unwrap().len(), 1);
    let l = run(&mut s, "report.custom_visuals", json!({}));
    assert_eq!(l["embedded"][0]["bicraft"], "bicraftSpec", "{l}");
    assert_eq!(l["embedded"][0]["issues"], json!([]), "{l}");
    assert_eq!(s.snapshot()["report"]["bicraft_visuals"][0]["embedded"], true);
    // by its guid too, the package already there
    let again = add(&mut s, SPEC_GUID, json!([]));
    assert_eq!(again["installed"], false, "{again}");
    // a role the visual does not have: refused, the report unchanged
    let before = s.report.clone();
    let p = page(&s);
    let err = s
        .execute(
            "visual.add",
            &json!({"page": p, "visual_type": "bicraftSpec", "x": 0, "y": 0, "width": 100, "height": 100,
                    "fields": [{"role": "Y", "field": "[Total]"}]}),
        )
        .unwrap_err();
    assert!(err.contains("no role `Y`"), "{err}");
    assert_eq!(s.report, before);
}

#[test]
fn an_older_build_is_replaced_and_a_newer_one_kept() {
    let mut s = session();
    add(&mut s, "bicraftSpec", json!([]));
    set_embedded_version(&mut s, "1.0.0.0");
    assert_eq!(spec_entry(&mut s)["current"], false);
    let out = add(&mut s, "bicraftSpec", json!([]));
    assert_eq!(out["installed"], true, "an older build is replaced");
    assert_eq!(spec_entry(&mut s)["embedded_version"], SPEC_PACKAGE_VERSION);
    set_embedded_version(&mut s, "99.0.0.0");
    let out = add(&mut s, "bicraftSpec", json!([]));
    assert_eq!(out["installed"], false, "a newer build is kept");
    assert_eq!(spec_entry(&mut s)["embedded_version"], "99.0.0.0");
    // visual.set_spec follows the same rule
    let v = out["visual"].as_str().unwrap().to_string();
    let p = page(&s);
    let sp = run(&mut s, "visual.spec_template", json!({}))["spec"].clone();
    let set = run(
        &mut s,
        "visual.set_spec",
        json!({"page": p, "visual": v, "spec": sp}),
    );
    assert_eq!(set["installed"], false, "{set}");
}

#[test]
fn set_type_moves_fields_into_the_visual_roles_and_embeds_it() {
    let mut s = session();
    let v = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "T[K]"}, {"role": "Y", "field": "[Total]"}]),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string();
    let p = page(&s);
    // a refused change embeds nothing
    let err = s
        .execute(
            "visual.set_type",
            &json!({"page": p, "visual": "nope", "visual_type": "bicraftSpec"}),
        )
        .unwrap_err();
    assert!(!err.is_empty());
    assert_eq!(spec_entry(&mut s)["embedded"], false);
    let out = run(
        &mut s,
        "visual.set_type",
        json!({"page": p, "visual": v, "visual_type": "bicraftSpec"}),
    );
    assert_eq!(out["visual_type"], SPEC_GUID, "{out}");
    assert_eq!(out["installed"], true);
    assert_eq!(
        out["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["role"].clone())
            .collect::<Vec<_>>(),
        [json!("category"), json!("values")],
        "{out}"
    );
    assert_eq!(out["dropped"], json!([]));
    let again = s
        .execute(
            "visual.set_type",
            &json!({"page": p, "visual": v, "visual_type": "bicraftSpec"}),
        )
        .unwrap_err();
    assert!(again.contains("already"), "{again}");
}

#[test]
fn a_save_drops_the_package_of_an_unused_bicraft_visual() {
    let dir = TempDir::new("builtin-visuals");
    let mut s = session();
    let kept = add(&mut s, "bicraftSpec", json!([]))["visual"]
        .as_str()
        .unwrap()
        .to_string();
    let p = page(&s);
    // used: the package survives the save
    run(&mut s, "file.save_as", json!({"path": dir.join("Used.pbip")}));
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": dir.join("Used.pbip")}));
    assert_eq!(spec_entry(&mut back)["embedded"], true);
    // no longer used: the next save drops it
    run(&mut s, "visual.remove", json!({"page": p, "visual": kept}));
    assert_eq!(spec_entry(&mut s)["embedded"], true, "kept until the save");
    run(&mut s, "file.save", json!({}));
    assert_eq!(spec_entry(&mut s)["embedded"], false);
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": dir.join("Used.pbip")}));
    let l = run(&mut back, "report.custom_visuals", json!({}));
    assert_eq!(l["embedded"], json!([]), "{l}");
    assert_eq!(l["count"], 0);
}

/// The first pack's visuals (M12.3, parts A and B) are built in too: `visual.add` takes their
/// names.
/// The snapshot entry of visual `name`.
fn snap_visual(s: &Session, name: &str) -> Value {
    let snap = s.snapshot();
    snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no visual {name}"))
}

/// G111: `report.apply` takes BIcraft's visuals by name or guid as visual.add does: the guid,
/// their own roles, the package embedded on first use; re-applying changes nothing.
#[test]
fn report_apply_takes_bicraft_visual_names() {
    let mut s = session();
    let p = page(&s);
    let spec = json!({"pages": [{"name": p, "visuals": [
        {"key": "spec", "type": "bicraftSpec", "x": 0, "y": 0, "width": 300, "height": 200,
         "fields": [{"role": "Category", "field": "T[K]"}, {"role": "VALUES", "field": "[Total]"}]},
        {"key": "pack", "type": bicraft_engine::chart_pack::VISUALS[0].name, "x": 320, "y": 0,
         "width": 300, "height": 200}
    ]}]});
    let out = run(&mut s, "report.apply", json!({"spec": spec}));
    assert_eq!(out["warning_count"], 0, "{out}");
    assert_eq!(
        out["installed"],
        json!(["bicraftSpec", bicraft_engine::chart_pack::VISUALS[0].name]),
        "{out}"
    );
    let r = s.report.as_ref().unwrap();
    let v = &r.pages[0].visuals[0];
    assert_eq!(v.visual_type.as_deref(), Some(SPEC_GUID));
    let roles: Vec<&str> = v.fields.iter().map(|f| f.role.as_str()).collect();
    assert_eq!(roles, ["category", "values"]);
    assert_eq!(
        r.pages[0].visuals[1].visual_type.as_deref(),
        Some(bicraft_engine::chart_pack::VISUALS[0].guid)
    );
    assert_eq!(spec_entry(&mut s)["embedded"], true);
    // in sync on a second run, nothing installed again
    let again = run(&mut s, "report.apply", json!({"spec": spec}));
    assert_eq!(again["change_count"], 0, "{again}");
    assert_eq!(again["installed"], json!([]));
    // a role the visual does not have: refused, the report unchanged
    let before = s.report.clone();
    let bad = json!({"pages": [{"name": p, "visuals": [
        {"key": "bad", "type": "bicraftSpec", "x": 0, "y": 300, "width": 100, "height": 100,
         "fields": [{"role": "Y", "field": "[Total]"}]}]}]});
    let err = s.execute("report.apply", &json!({"spec": bad})).unwrap_err();
    assert!(err.contains("no role `Y`"), "{err}");
    assert_eq!(s.report, before);
}

/// G111: a Spec visual added from the gallery (visual.add, visual.set_type, report.apply)
/// starts with visual.spec_template's spec for its fields, so it draws instead of showing a
/// missing-spec error; a visual.set_spec replaces it.
#[test]
fn a_new_spec_visual_starts_with_a_template_spec() {
    let mut s = session();
    let one = add(
        &mut s,
        "bicraftSpec",
        json!([{"role": "category", "field": "T[K]"}, {"role": "values", "field": "[Total]"}]),
    );
    assert_eq!(one["spec_template"], "bar", "{one}");
    let v = snap_visual(&s, one["visual"].as_str().unwrap());
    assert!(v.get("spec_error").is_none(), "{v}");
    assert_eq!(v["spec"]["marks"][0]["type"], "bar", "{v}");
    // two values: the bar template too, which draws each measure as a series of its own
    let two = add(
        &mut s,
        "bicraftSpec",
        json!([{"role": "category", "field": "T[K]"}, {"role": "values", "field": "[Total]"},
               {"role": "values", "field": "Sum(T[V])"}]),
    );
    assert_eq!(two["spec_template"], "bar", "{two}");
    let v = snap_visual(&s, two["visual"].as_str().unwrap());
    assert_eq!(v["spec"]["marks"][0]["y"], "value", "{v}");
    // visual.set_type
    let col = add(&mut s, "clusteredColumnChart", json!([]))["visual"]
        .as_str()
        .unwrap()
        .to_string();
    let p = page(&s);
    let set = run(
        &mut s,
        "visual.set_type",
        json!({"page": p, "visual": col, "visual_type": "bicraftSpec"}),
    );
    assert_eq!(set["spec_template"], "bar", "{set}");
    assert!(snap_visual(&s, &col).get("spec_error").is_none());
    // report.apply
    let spec = json!({"pages": [{"name": p, "visuals": [
        {"key": "s", "type": "bicraftSpec", "x": 0, "y": 300, "width": 300, "height": 200}]}]});
    run(&mut s, "report.apply", json!({"spec": spec}));
    let name = s.report.as_ref().unwrap().pages[0]
        .visuals
        .last()
        .unwrap()
        .name
        .clone();
    let v = snap_visual(&s, &name);
    assert_eq!(v["spec"]["marks"][0]["type"], "bar", "{v}");
    // a set spec is kept as it is
    let line = run(&mut s, "visual.spec_template", json!({"kind": "line"}))["spec"].clone();
    let one = one["visual"].as_str().unwrap().to_string();
    run(
        &mut s,
        "visual.set_spec",
        json!({"page": p, "visual": one, "spec": line}),
    );
    assert_eq!(snap_visual(&s, &one)["spec"]["marks"][0]["type"], "line");
}

#[test]
fn first_pack_visuals_are_added_by_name() {
    let mut s = session();
    let pack = bicraft_engine::chart_pack::VISUALS
        .iter()
        .map(|d| (d.name, d.guid));
    let charts = bicraft_engine::bvis::DEFS.iter().map(|d| (d.name, d.guid));
    for (name, guid) in charts.chain(pack) {
        let out = add(&mut s, name, json!([]));
        assert_eq!(out["visual_type"], guid, "{out}");
        assert_eq!(out["installed"], true, "{name}");
    }
    let l = run(&mut s, "report.custom_visuals", json!({}));
    assert_eq!(l["embedded"].as_array().unwrap().len(), 13, "{l}");
    assert_eq!(
        s.snapshot()["report"]["bicraft_visuals"]
            .as_array()
            .unwrap()
            .len(),
        14
    );
    // visual.set_bicraft_visual finds the package already there
    let v = add(&mut s, "clusteredColumnChart", json!([]))["visual"]
        .as_str()
        .unwrap()
        .to_string();
    let p = page(&s);
    let set = run(
        &mut s,
        "visual.set_bicraft_visual",
        json!({"page": p, "visual": v, "kind": bicraft_engine::bvis::DEFS[0].kind}),
    );
    assert_eq!(set["installed"], false, "{set}");
    // a role the calendar heatmap does not have
    let p = page(&s);
    let err = s
        .execute(
            "visual.add",
            &json!({"page": p, "visual_type": "bicraftCalendarHeatmap", "x": 0, "y": 0, "width": 100,
                    "height": 100, "fields": [{"role": "category", "field": "T[K]"}]}),
        )
        .unwrap_err();
    assert!(err.contains("no role `category`"), "{err}");
}

/// Power BI Desktop 2.159.1231.0 (2026-11-24) titles BIcraft's visuals by the kind of their roles:
/// the measure roles' fields by the grouping roles' fields, each in the visual's role order, or the
/// grouping fields alone ("People par Score", "Total et Goal par Dept", "Total par Id, Parent et
/// Name", "Id et Skill" in a French Desktop); the snapshot's `auto_title` says the same.
#[test]
fn bicraft_visuals_take_desktop_auto_titles() {
    let mut s = session();
    run(
        &mut s,
        "measure.add",
        json!({"table": "T", "name": "Goal", "expression": "SUM(T[V]) * 2"}),
    );
    let cases = [
        (
            "bicraftBullet",
            json!([{"role": "target", "field": "[Goal]"}, {"role": "category", "field": "T[K]"},
                   {"role": "value", "field": "[Total]"}]),
            "Total and Goal by K",
        ),
        (
            "bicraftHistogram",
            json!([{"role": "values", "field": "T[V]"}, {"role": "frequency", "field": "[Total]"}]),
            "Total by V",
        ),
        (
            "bicraftOrgChart",
            json!([{"role": "id", "field": "T[K]"}, {"role": "parent", "field": "T[V]"},
                   {"role": "label", "field": "T[K]"}, {"role": "value", "field": "[Total]"}]),
            "Total by K, V and K",
        ),
        (
            "bicraftVenn",
            json!([{"role": "item", "field": "T[K]"}, {"role": "set", "field": "T[V]"}]),
            "K and V",
        ),
        (
            "bicraftSankey",
            json!([{"role": "source", "field": "T[K]"}, {"role": "destination", "field": "T[V]"},
                   {"role": "weight", "field": "[Total]"}]),
            "Total by K and V",
        ),
    ];
    for (name, fields, title) in cases {
        let v = add(&mut s, name, fields)["visual"].as_str().unwrap().to_string();
        let snap = s.snapshot();
        let got = snap["report"]["pages"][0]["visuals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["name"] == v.as_str())
            .map(|x| x["auto_title"].clone())
            .unwrap();
        assert_eq!(got, title, "{name}");
    }
}

/// The `.pbiviz.json` of a package.
fn pbiviz_json(bytes: &[u8]) -> Value {
    let read = bicraft_pbix::pbiviz::read_pbiviz(bytes).unwrap();
    let name = format!("resources/{}.pbiviz.json", read.guid);
    let (_, body) = read.files.iter().find(|(n, _)| *n == name).unwrap();
    serde_json::from_slice(body).unwrap()
}

/// `visual.export_bicraft_visual` (ROADMAP M12.5): every BIcraft visual written into a folder
/// as `<guid>.<version>.pbiviz`, its capabilities valid against the API schema, read back by
/// `visual.import_custom` as the very package BIcraft embeds (so `visual.add` finds it current);
/// the session is left as it was.
#[test]
fn every_bicraft_visual_exports_and_imports_back() {
    let dir = TempDir::new("builtin-visuals-export");
    let mut s = session();
    let revision = s.revision;
    let out = run(
        &mut s,
        "visual.export_bicraft_visual",
        json!({"name": "all", "out": dir.join("visuals")}),
    );
    assert_eq!(out["count"], 14, "{out}");
    assert_eq!(s.revision, revision);
    let schema_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/report/powerbi-visuals-api/schema.capabilities.json");
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(schema_path).unwrap()).unwrap();
    for f in out["files"].as_array().unwrap() {
        let (guid, version) = (f["guid"].as_str().unwrap(), f["version"].as_str().unwrap());
        let path = dir.join("visuals").join(format!("{guid}.{version}.pbiviz"));
        assert_eq!(f["path"], path.display().to_string());
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(f["bytes"], bytes.len());
        let pkg = pbiviz_json(&bytes);
        assert_eq!(pkg["visual"]["guid"], guid);
        assert_eq!(pkg["visual"]["version"], version);
        let issues = bicraft_pbir::JsonSchema::new(schema.clone()).validate(&pkg["capabilities"]);
        assert!(issues.is_empty(), "{guid}: {issues:?}");
        // every data role, format card and option has a `displayNameKey` with its English (the
        // `displayName`) and French in the package's `stringResources` (G109, G110)
        let caps = &pkg["capabilities"];
        let strings = &pkg["stringResources"];
        let mut named: Vec<&Value> = caps["dataRoles"].as_array().unwrap().iter().collect();
        for o in caps["objects"].as_object().unwrap().values() {
            named.push(o);
            named.extend(o["properties"].as_object().unwrap().values());
        }
        let mut french = 0;
        for n in named {
            let key = n["displayNameKey"]
                .as_str()
                .unwrap_or_else(|| panic!("{guid}: {n}"));
            assert_eq!(strings["en-US"][key], n["displayName"], "{guid}: {key}");
            let fr = strings["fr-FR"][key]
                .as_str()
                .unwrap_or_else(|| panic!("{guid}: {key}"));
            french += usize::from(fr != n["displayName"].as_str().unwrap());
        }
        assert!(french > 0, "{guid}: nothing in French");
        let imported = run(&mut s, "visual.import_custom", json!({"path": path}));
        assert_eq!(imported["name"], guid, "{imported}");
        assert_eq!(imported["has_javascript"], true);
    }
    let listed = run(&mut s, "report.custom_visuals", json!({}));
    for b in listed["bicraft"].as_array().unwrap() {
        assert_eq!(b["embedded"], true, "{b}");
        assert_eq!(b["current"], true, "{b}");
    }
    // the imported package is the one BIcraft embeds: nothing to install
    for b in listed["bicraft"].as_array().unwrap() {
        let added = add(&mut s, b["name"].as_str().unwrap(), json!([]));
        assert_eq!(added["installed"], false, "{added}");
    }
}

/// One visual: to the `.pbiviz` file named, or into a folder; a list writes into a folder; bad
/// names and outputs are refused before anything is written.
#[test]
fn export_one_visual_a_list_and_refusals() {
    let dir = TempDir::new("builtin-visuals-export-one");
    let mut s = Session::new();
    let file = dir.join("venn.pbiviz");
    let out = run(
        &mut s,
        "visual.export_bicraft_visual",
        json!({"name": "BICRAFTVENN", "out": file}),
    );
    assert_eq!(out["files"][0]["name"], "bicraftVenn");
    let pkg = pbiviz_json(&std::fs::read(&file).unwrap());
    assert_eq!(pkg["visual"]["displayName"], out["files"][0]["display_name"]);
    let out = run(
        &mut s,
        "visual.export_bicraft_visual",
        json!({"name": SPEC_GUID, "out": dir.join("one")}),
    );
    let spec = dir
        .join("one")
        .join(format!("{SPEC_GUID}.{SPEC_PACKAGE_VERSION}.pbiviz"));
    assert_eq!(out["files"][0]["path"], spec.display().to_string());
    assert!(spec.is_file());
    let out = run(
        &mut s,
        "visual.export_bicraft_visual",
        json!({"name": ["bicraftBullet", "bicraftOrgChart", "bicraftBullet"], "out": dir.join("two")}),
    );
    assert_eq!(out["count"], 2, "{out}");
    assert_eq!(std::fs::read_dir(dir.join("two")).unwrap().count(), 2);
    for p in [
        json!({"out": dir.join("x")}),
        json!({"name": "card", "out": dir.join("x")}),
        json!({"name": [], "out": dir.join("x")}),
        json!({"name": [1], "out": dir.join("x")}),
        json!({"name": ["bicraftVenn", "nope"], "out": dir.join("x")}),
        json!({"name": 3, "out": dir.join("x")}),
        json!({"name": "all"}),
        json!({"name": "all", "out": " "}),
        json!({"name": "all", "out": dir.join("all.pbiviz")}),
        json!({"name": "all", "out": file}),
        json!({"name": "bicraftVenn", "out": dir.join("x"), "extra": 1}),
    ] {
        assert!(s.execute("visual.export_bicraft_visual", &p).is_err(), "{p}");
    }
    assert!(!dir.join("x").exists());
    assert!(!dir.join("all.pbiviz").exists());
}

/// A scatter chart with two measures switched to the Spec visual keeps both in `values`, and
/// its starting spec draws both: as a clustered column chart does, each measure is a series of
/// its own with a legend (docs/formats/spec-visual.md "Series").
#[test]
fn a_scatter_switched_to_the_spec_visual_draws_both_measures() {
    let mut s = session();
    let sc = add(
        &mut s,
        "scatterChart",
        json!([{"role": "Category", "field": "T[K]"}, {"role": "X", "field": "[Total]"},
               {"role": "Y", "field": "Sum(T[V])"}]),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string();
    let p = page(&s);
    let set = run(
        &mut s,
        "visual.set_type",
        json!({"page": p, "visual": sc, "visual_type": "bicraftSpec"}),
    );
    let v = snap_visual(&s, &sc);
    let values = v["fields"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["role"] == "values")
        .count();
    assert_eq!(values, 2, "{set} {v}");
    // the spec over the two measures: bars of two colours and a legend naming both
    let parsed = bicraft_render::spec::parse(&v["spec"]).unwrap();
    let data = bicraft_render::spec::SpecData::from_json(&json!({"categories": ["a", "b"],
        "values": [[400000, 150000], [5200, 1800]], "names": ["Total", "Sum of V"]}));
    let prims = bicraft_render::spec::layout_json(&parsed, &data, 400.0, 240.0);
    let texts: Vec<&str> = prims
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["text"].as_str())
        .collect();
    assert!(
        texts.contains(&"Total") && texts.contains(&"Sum of V"),
        "{texts:?}"
    );
    let fills: std::collections::BTreeSet<String> = prims
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["t"] == "rect" && p["role"].is_null() && p["w"].as_f64().unwrap() > 10.0)
        .map(|p| p["fill"].to_string())
        .collect();
    assert!(fills.len() >= 2, "{fills:?}");
}

/// The Spec visual's Data labels card: a text size set in the format pane (`labels.fontSize`)
/// reaches the spec the renderer draws, and the package declares the text cards with the
/// formatting types Desktop's font, size and display units controls read.
#[test]
fn the_spec_visual_takes_the_data_label_text_size_from_the_format_pane() {
    let mut s = session();
    let v = add(
        &mut s,
        "bicraftSpec",
        json!([{"role": "category", "field": "T[K]"}, {"role": "values", "field": "[Total]"}]),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string();
    let p = page(&s);
    run(
        &mut s,
        "visual.format",
        json!({"page": p, "visual": v, "objects": [
            {"object": "labels", "property": "fontSize", "value": 18},
            {"object": "labels", "property": "labelDisplayUnits", "value": 1000},
            {"object": "valueAxis", "property": "labelColor", "value": "#aa0000"}
        ]}),
    );
    let snap = snap_visual(&s, &v);
    assert_eq!(snap["spec"]["text"]["labels"]["size"], json!(18.0), "{snap}");
    assert_eq!(snap["spec"]["text"]["labels"]["units"], json!(1000.0), "{snap}");
    assert_eq!(snap["spec"]["text"]["y"]["color"], "#AA0000", "{snap}");
    let parsed = bicraft_render::spec::parse(&snap["spec"]).unwrap();
    let data = bicraft_render::spec::SpecData::from_json(
        &json!({"categories": ["a", "b"], "values": [[1500, 2600]]}),
    );
    let prims = bicraft_render::spec::layout_json(&parsed, &data, 300.0, 200.0);
    let label = prims
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["text"] == "1.50K")
        .unwrap_or_else(|| panic!("{prims}"));
    assert_eq!(label["size"], json!(18.0));
    let caps = bicraft_engine::spec_visual::capabilities_json();
    for (object, property, kind) in [
        ("labels", "fontSize", "fontSize"),
        ("labels", "fontFamily", "fontFamily"),
        ("labels", "labelDisplayUnits", "labelDisplayUnits"),
        ("categoryAxis", "titleFontSize", "fontSize"),
        ("legend", "fontSize", "fontSize"),
    ] {
        assert_eq!(
            caps["objects"][object]["properties"][property]["type"]["formatting"][kind],
            json!(true),
            "{object}.{property}"
        );
    }
}
