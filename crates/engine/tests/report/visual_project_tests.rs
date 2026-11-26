//! `visual.scaffold`, `visual.build` and `visual.pack` (docs/formats/pbiviz.md "Visual
//! projects"): a project scaffolded here builds with swc and packs into a `.pbiviz` that
//! `visual.import_custom` reads back and that survives PBIP and PBIX round trips; malformed
//! projects and hostile parameters give errors, never panics.

use std::path::Path;

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

const GUID: &str = "bicraftBars0123456789ABCDEF0123456789AB";

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn fails(s: &mut Session, id: &str, params: Value) -> String {
    s.execute(id, &params)
        .expect_err(&format!("{id} {params} should fail"))
}

fn scaffold(s: &mut Session, dir: &Path) -> Value {
    run(
        s,
        "visual.scaffold",
        json!({
            "folder": dir, "name": "bicraftBars", "display_name": "Bars", "guid": GUID,
            "author": "BIcraft tests",
            "data_roles": ["region:Grouping:Region", "sales:Measure", {"name": "target", "kind": "measure"}],
            "format_options": ["bars.fill:color:Bar colour", "bars.showLabels:bool", "labels.size:numeric", "labels.prefix:text"],
        }),
    )
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn a_scaffolded_project_builds_packs_and_imports_back() {
    let dir = TempDir::new("engine-vp-roundtrip");
    let proj = dir.join("bars");
    let mut s = Session::new();
    let out = scaffold(&mut s, &proj);
    assert_eq!(out["guid"], GUID);
    assert_eq!(
        out["files"],
        json!([
            "pbiviz.json",
            "capabilities.json",
            "tsconfig.json",
            "package.json",
            "src/visual.ts",
            "style/visual.css",
            "assets/icon.png"
        ])
    );
    // the capabilities hold the roles and options asked for
    let caps = read_json(&proj.join("capabilities.json"));
    assert_eq!(
        caps["dataRoles"],
        json!([
            {"displayName": "Region", "name": "region", "kind": "Grouping"},
            {"displayName": "Sales", "name": "sales", "kind": "Measure"},
            {"displayName": "Target", "name": "target", "kind": "Measure"},
        ])
    );
    assert_eq!(
        caps["dataViewMappings"][0]["categorical"]["categories"]["for"]["in"],
        "region"
    );
    assert_eq!(
        caps["dataViewMappings"][0]["categorical"]["values"]["select"],
        json!([{"bind": {"to": "sales"}}, {"bind": {"to": "target"}}])
    );
    assert_eq!(
        caps["objects"]["bars"]["properties"]["fill"],
        json!({"displayName": "Bar colour", "type": {"fill": {"solid": {"color": true}}}})
    );
    assert_eq!(
        caps["objects"]["bars"]["properties"]["showLabels"]["type"],
        json!({"bool": true})
    );
    assert_eq!(
        caps["objects"]["labels"]["properties"]["size"]["type"],
        json!({"numeric": true})
    );
    assert_eq!(
        caps["objects"]["labels"]["properties"]["prefix"]["type"],
        json!({"text": true})
    );
    let pbiviz = read_json(&proj.join("pbiviz.json"));
    assert_eq!(pbiviz["visual"]["guid"], GUID);
    assert_eq!(pbiviz["apiVersion"], "5.3.0");
    let ts = std::fs::read_to_string(proj.join("src/visual.ts")).unwrap();
    assert!(
        ts.contains("object: \"bars\", property: \"fill\", kind: \"color\""),
        "{ts}"
    );
    assert!(
        std::fs::read(proj.join("assets/icon.png"))
            .unwrap()
            .starts_with(b"\x89PNG")
    );

    // build: one script, the types gone, the class registered under the guid
    let built = run(&mut s, "visual.build", json!({"folder": proj}));
    assert_eq!(built["entry"], "src/visual.ts");
    assert_eq!(built["modules"], json!(["src/visual.ts"]));
    let js = std::fs::read_to_string(proj.join("dist/visual.js")).unwrap();
    assert_eq!(built["js_bytes"].as_u64().unwrap() as usize, js.len());
    for want in [
        "class Visual",
        "getFormattingModel",
        "visuals.plugins",
        GUID,
        "\"Bars\"",
        "new Visual(options)",
    ] {
        assert!(js.contains(want), "`{want}` missing from:\n{js}");
    }
    for gone in [
        "import ",
        "export ",
        "interface ",
        "powerbi-visuals-api",
        ": string",
        "implements IVisual",
    ] {
        assert!(!js.contains(gone), "`{gone}` left in:\n{js}");
    }
    // the bundle's exports as a global named after the guid, `default` being the plugin:
    // Desktop 2.159 never draws a registered visual without it (docs/formats/pbiviz.md)
    assert!(js.starts_with(&format!("var {GUID} = ")), "{js}");
    assert!(js.contains("default: plugin"), "{js}");

    // pack, then import the package into a report
    let packed = run(&mut s, "visual.pack", json!({"folder": proj}));
    let file = proj.join(format!("dist/{GUID}.1.0.0.0.pbiviz"));
    assert_eq!(packed["out"], file.display().to_string());
    assert_eq!(packed["has_icon"], true);
    assert_eq!(packed["version"], "1.0.0.0");
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(&mut s, "report.new", json!({}));
    let imported = run(&mut s, "visual.import_custom", json!({"path": file}));
    assert_eq!(imported["name"], GUID);
    assert_eq!(imported["display_name"], "Bars");
    assert_eq!(imported["api_version"], "5.3.0");
    assert_eq!(imported["author"], "BIcraft tests");
    assert_eq!(imported["has_icon"], true);
    assert_eq!(imported["has_javascript"], true);
    assert_eq!(imported["skipped"], json!([]));
    assert_eq!(imported["warnings"], json!([]), "{imported}");
    let listed = run(&mut s, "report.custom_visuals", json!({}));
    let e = &listed["embedded"][0];
    assert_eq!(e["registered"], true, "{listed}");
    assert_eq!(e["issues"], json!([]), "{listed}");
    assert_eq!(e["version"], "1.0.0.0");
    assert_eq!(e["data_roles"].as_array().map(Vec::len), Some(3), "{listed}");
    // the package's .pbiviz.json carries the script, the CSS and the icon
    let r = s.report.as_ref().unwrap();
    let body = r
        .resources
        .iter()
        .find(|x| x.path == format!("CustomVisuals/{GUID}/resources/{GUID}.pbiviz.json"))
        .unwrap();
    let body: Value = serde_json::from_slice(&body.bytes).unwrap();
    assert_eq!(body["content"]["js"].as_str(), Some(js.as_str()));
    assert!(
        body["content"]["css"]
            .as_str()
            .unwrap()
            .contains(".bicraft-visual")
    );
    assert!(
        body["content"]["iconBase64"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,iVBORw0KGgo")
    );
    assert_eq!(body["capabilities"], caps);
    // the imported package survives PBIP and PBIX round trips
    for name in ["Shop.pbip", "Shop.pbix"] {
        let out = TempDir::new(&format!("engine-vp-{name}"));
        run(&mut s, "file.save_as", json!({"path": out.join(name)}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": out.join(name)}));
        let l = run(&mut back, "report.custom_visuals", json!({}));
        assert_eq!(l["embedded"][0]["name"], GUID, "{name}: {l}");
        assert_eq!(l["embedded"][0]["registered"], true, "{name}: {l}");
    }
}

#[test]
fn a_project_of_several_modules_and_an_out_path() {
    let dir = TempDir::new("engine-vp-modules");
    let proj = dir.join("p");
    let mut s = Session::new();
    run(
        &mut s,
        "visual.scaffold",
        json!({"folder": proj, "name": "multi", "guid": "multi1"}),
    );
    std::fs::write(
        proj.join("src/visual.ts"),
        "import { label } from \"./text/label\";\nexport class Visual {\n  constructor(o: any) {}\n  update(): string { return label(2); }\n}\n",
    )
    .unwrap();
    std::fs::create_dir_all(proj.join("src/text")).unwrap();
    std::fs::write(
        proj.join("src/text/label.ts"),
        "export const label = (n: number): string => `#${n}`;\n",
    )
    .unwrap();
    // a declaration file and other files are not modules
    std::fs::write(proj.join("src/types.d.ts"), "declare const x: number;\n").unwrap();
    let out = dir.join("elsewhere/bundle.js");
    let b = run(&mut s, "visual.build", json!({"folder": proj, "out": out}));
    assert_eq!(b["modules"], json!(["src/text/label.ts", "src/visual.ts"]));
    let js = std::fs::read_to_string(&out).unwrap();
    assert!(js.contains("#${n}") && js.contains("class Visual"), "{js}");
    let p = run(
        &mut s,
        "visual.pack",
        json!({"folder": proj, "out": dir.join("v.pbiviz")}),
    );
    assert_eq!(p["modules"], b["modules"]);
    assert!(dir.join("v.pbiviz").exists());
    // a scaffold with defaults: two roles, two format options, a generated guid
    let d = run(
        &mut s,
        "visual.scaffold",
        json!({"folder": dir.join("d"), "name": "dflt"}),
    );
    assert_eq!(d["data_roles"].as_array().map(Vec::len), Some(2));
    assert_eq!(d["format_options"].as_array().map(Vec::len), Some(2));
    let g = d["guid"].as_str().unwrap();
    assert!(g.starts_with("dflt") && g.len() == 4 + 32, "{g}");
    run(&mut s, "visual.pack", json!({"folder": dir.join("d")}));
}

/// Writes `rel` = `text` in the project (after a fresh scaffold) and returns the build's error.
/// A malformed project: its name, the files written over a fresh scaffold, the error expected.
type Case<'a> = (&'a str, Vec<(&'a str, String)>, &'a str);

fn broken(s: &mut Session, dir: &TempDir, case: &str, edits: &[(&str, &str)]) -> String {
    let proj = dir.join(case);
    run(
        s,
        "visual.scaffold",
        json!({"folder": proj, "name": "broken", "guid": "broken1"}),
    );
    for (rel, text) in edits {
        let path = proj.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let e = fails(s, "visual.build", json!({"folder": proj}));
    let e2 = fails(s, "visual.pack", json!({"folder": proj}));
    assert_eq!(e, e2, "build and pack refuse alike");
    assert!(!proj.join("dist").exists(), "{case}: nothing written");
    e
}

#[test]
fn malformed_projects_are_refused_with_their_reason() {
    let dir = TempDir::new("engine-vp-broken");
    let mut s = Session::new();
    let manifest =
        |guid: &str, extra: &str| format!("{{\"visual\": {{\"guid\": \"{guid}\", \"name\": \"b\"}}{extra}}}");
    let cases: Vec<Case> = vec![
        ("syntax", vec![("src/visual.ts", "export class Visual {\n  update() { let x = ; }\n}\n".into())], "src/visual.ts:2:"),
        ("npm", vec![("src/visual.ts", "import * as d3 from \"d3\";\nexport class Visual { u() { return d3; } }\n".into())], "`d3`, a package"),
        ("missing-import", vec![("src/visual.ts", "import { x } from \"./x\";\nexport class Visual { u() { return x; } }\n".into())], "`./x`"),
        ("cycle", vec![
            ("src/visual.ts", "import { b } from \"./b\";\nexport const a = 1;\nexport class Visual { u() { return b; } }\n".into()),
            ("src/b.ts", "import { a } from \"./visual\";\nexport const b = a;\n".into()),
        ], "import cycle"),
        ("no-class", vec![("src/visual.ts", "export class Other {}\n".into())], "does not export the class `Visual`"),
        ("no-entry", vec![("tsconfig.json", "{\"files\": [\"src/nope.ts\"]}".into())], "src/nope.ts"),
        ("bad-guid", vec![("pbiviz.json", manifest("../evil", ""))], "visual.guid"),
        ("no-guid", vec![("pbiviz.json", "{\"visual\": {}}".into())], "no `visual.guid`"),
        ("not-json", vec![("pbiviz.json", "{nope".into())], "not valid JSON"),
        ("less", vec![("pbiviz.json", manifest("b1", ", \"style\": \"style/visual.less\""))], "LESS is not compiled"),
        ("escape", vec![("pbiviz.json", manifest("b1", ", \"capabilities\": \"../caps.json\""))], "inside the project folder"),
        ("absolute", vec![("pbiviz.json", manifest("b1", ", \"assets\": {\"icon\": \"/etc/icon.png\"}"))], "inside the project folder"),
        ("icon", vec![("assets/icon.png", "not a png".into())], "not a PNG"),
        ("caps", vec![("capabilities.json", "[1]".into())], "not a JSON object"),
        ("class-name", vec![("pbiviz.json", manifest("b1", "").replace("\"name\"", "\"visualClassName\": \"a-b\", \"name\""))], "not a JavaScript name"),
        ("version", vec![("pbiviz.json", manifest("b1", "").replace("\"name\"", "\"version\": \"1.0;x\", \"name\""))], "visual.version"),
    ];
    for (case, edits, want) in &cases {
        let edits: Vec<(&str, &str)> = edits.iter().map(|(r, t)| (*r, t.as_str())).collect();
        let e = broken(&mut s, &dir, case, &edits);
        assert!(e.contains(want), "{case}: {e}");
    }
    // not a project at all
    let e = fails(&mut s, "visual.pack", json!({"folder": dir.join("nothing")}));
    assert!(e.contains("not a folder"), "{e}");
    std::fs::create_dir_all(dir.join("empty")).unwrap();
    let e = fails(&mut s, "visual.build", json!({"folder": dir.join("empty")}));
    assert!(e.contains("no pbiviz.json"), "{e}");
}

#[test]
fn scaffold_refuses_bad_choices_and_an_existing_project() {
    let dir = TempDir::new("engine-vp-scaffold");
    let mut s = Session::new();
    let f = dir.join("p");
    for (params, want) in [
        (json!({"folder": f, "name": "1abc"}), "visual name"),
        (json!({"folder": f, "name": "a b"}), "visual name"),
        (json!({"folder": f, "name": "ok", "guid": "../x"}), "guid"),
        (
            json!({"folder": f, "name": "ok", "data_roles": ["a:Bogus"]}),
            "data role kind",
        ),
        (
            json!({"folder": f, "name": "ok", "data_roles": ["a", "A"]}),
            "given twice",
        ),
        (
            json!({"folder": f, "name": "ok", "data_roles": ["bad name"]}),
            "data role",
        ),
        (json!({"folder": f, "name": "ok", "data_roles": [1]}), "data role"),
        (
            json!({"folder": f, "name": "ok", "data_roles": "a"}),
            "must be a list",
        ),
        (
            json!({"folder": f, "name": "ok", "format_options": ["nodot:color"]}),
            "object.property",
        ),
        (
            json!({"folder": f, "name": "ok", "format_options": ["a.b:date"]}),
            "format option kind",
        ),
        (
            json!({"folder": f, "name": "ok", "format_options": ["a.b:text", "a.b:bool"]}),
            "given twice",
        ),
        (
            json!({"folder": f, "name": "ok", "display_name": "x\u{0}y"}),
            "control characters",
        ),
        (json!({"folder": f, "name": "ok", "nope": 1}), "unknown parameter"),
        (json!({"folder": f}), "`name`"),
    ] {
        let e = fails(&mut s, "visual.scaffold", params.clone());
        assert!(e.contains(want), "{params}: {e}");
        assert!(!f.exists(), "{params}: nothing written");
    }
    run(&mut s, "visual.scaffold", json!({"folder": f, "name": "ok"}));
    let e = fails(&mut s, "visual.scaffold", json!({"folder": f, "name": "ok"}));
    assert!(e.contains("overwrite"), "{e}");
    run(
        &mut s,
        "visual.scaffold",
        json!({"folder": f, "name": "ok", "overwrite": true}),
    );
    // an empty role list: a visual without fields (a warning when packing)
    let g = dir.join("g");
    run(
        &mut s,
        "visual.scaffold",
        json!({"folder": g, "name": "nofields", "data_roles": [], "format_options": []}),
    );
    let p = run(&mut s, "visual.pack", json!({"folder": g}));
    assert!(p["warnings"].to_string().contains("no data roles"), "{p}");
}

#[test]
fn hostile_parameters_give_errors() {
    let mut s = Session::new();
    for id in ["visual.scaffold", "visual.build", "visual.pack"] {
        for params in [
            json!(null),
            json!([]),
            json!("x"),
            json!({}),
            json!({"folder": 1}),
            json!({"folder": ""}),
            json!({"folder": "\u{0}"}),
            json!({"folder": "x", "out": 5}),
            json!({"folder": "x", "name": {"a": 1}}),
        ] {
            assert!(s.execute(id, &params).is_err(), "{id} {params}");
        }
    }
}
