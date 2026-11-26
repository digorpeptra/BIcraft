//! Tests of the rich content commands (`rich.rs`, ROADMAP M11.20): text boxes with runs and
//! dynamic values, images stored as registered resources, buttons and their actions, the page's
//! background, wallpaper, size and alignment; their PBIP and PBIX round trips, the snapshot and
//! the reader's clicks; hostile parameters and image bytes. Synthetic model only.

use bicraft_report::{Report, Visual};
use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

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

/// A 1×1 PNG.
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#118DFF"/></svg>"##;

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

/// A model with a measure and a report with two pages (Main, Detail).
fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "m": "let Source = #table(type table [Region = text, Amount = Int64.Type], {{\"East\", 5}, {\"West\", 7}}) in Source"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
    );
    run(&mut s, "report.new", json!({}));
    let first = page(&s, 0);
    run(
        &mut s,
        "page.rename",
        json!({"page": first, "display_name": "Main"}),
    );
    run(&mut s, "page.add", json!({"display_name": "Detail"}));
    s
}

fn page(s: &Session, i: usize) -> String {
    s.report.as_ref().unwrap().pages[i].name.clone()
}

fn report(s: &Session) -> &Report {
    s.report.as_ref().unwrap()
}

fn find<'a>(r: &'a Report, name: &str) -> &'a Visual {
    r.pages
        .iter()
        .flat_map(|p| &p.visuals)
        .find(|v| v.name == name)
        .unwrap_or_else(|| panic!("no visual {name}"))
}

fn snap_visual(s: &Session, name: &str) -> Value {
    let snap = s.snapshot();
    for p in snap["report"]["pages"].as_array().cloned().unwrap_or_default() {
        for v in p["visuals"].as_array().cloned().unwrap_or_default() {
            if v["name"] == name {
                return v;
            }
        }
    }
    panic!("visual {name} not in the snapshot")
}

fn temp(name: &str) -> TempDir {
    TempDir::new(&format!("engine-rich-{name}"))
}

/// The report after a save as `file` and a fresh open.
fn reopened(s: &mut Session, dir: &TempDir, file: &str) -> Session {
    run(s, "file.save_as", json!({"path": dir.join(file)}));
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": dir.join(file)}));
    back
}

fn rich_paragraphs() -> Value {
    json!([
        {"align": "center", "runs": [
            {"text": "Sales ", "bold": true, "font_size": 14},
            {"text": "this year", "italic": true, "color": "#118dff", "font_family": "Segoe UI"}
        ]},
        {"list": "bullet", "runs": [{"text": "Total: "}, {"value": "[Total]", "bold": true}]},
        {"list": "bullet", "runs": [{"text": "Docs", "underline": true, "url": "https://example.com/docs"}]},
        {"runs": []}
    ])
}

#[test]
fn text_boxes_write_desktop_paragraphs_and_round_trip() {
    let mut s = session();
    let main = page(&s, 0);
    let out = run(
        &mut s,
        "textbox.add",
        json!({"page": main, "x": 20, "y": 30, "paragraphs": rich_paragraphs()}),
    );
    let name = out["visual"].as_str().unwrap().to_string();
    let v = find(report(&s), &name).clone();
    assert_eq!(v.visual_type.as_deref(), Some("textbox"));
    assert_eq!(
        (v.position.x, v.position.y, v.position.width, v.position.height),
        (20.0, 30.0, 280.0, 110.0)
    );
    assert_eq!(v.extra["howCreated"], "InsertVisualButton");
    let objects = &v.visual_extra["objects"];
    assert_eq!(
        objects["general"][0]["properties"]["paragraphs"],
        json!([
            {"textRuns": [
                {"value": "Sales ", "textStyle": {"fontSize": "14pt", "fontWeight": "bold"}},
                {"value": "this year", "textStyle": {"fontFamily": "Segoe UI", "fontStyle": "italic", "color": "#118DFF"}}
            ], "horizontalTextAlignment": "center"},
            {"textRuns": [
                {"value": "Total: "},
                {"value": {"propertyIdentifier": {"objectName": "values", "propertyName": "expr"}, "selector": {"id": "value1"}},
                 "textStyle": {"fontWeight": "bold"}}
            ], "listType": "bullet"},
            {"textRuns": [{"value": "Docs", "textStyle": {"textDecoration": "underline"}, "url": "https://example.com/docs"}],
             "listType": "bullet"},
            {"textRuns": [{"value": ""}]}
        ])
    );
    assert_eq!(
        objects["values"],
        json!([{"properties": {"expr": {"expr": {"Measure": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Total"}}}},
                "selector": {"id": "value1"}}])
    );
    // the snapshot: runs, the dynamic value evaluated and its field for the editor
    let sv = snap_visual(&s, &name);
    let paras = &sv["text"]["paragraphs"];
    assert_eq!(paras[0]["align"], "center");
    assert_eq!(paras[0]["runs"][0]["bold"], true);
    assert_eq!(paras[0]["runs"][0]["font_size"], 14);
    assert_eq!(paras[0]["runs"][1]["color"], "#118DFF");
    assert_eq!(paras[1]["list"], "bullet");
    assert_eq!(paras[1]["runs"][1]["field"], "[Total]");
    assert_eq!(paras[1]["runs"][1]["text"], "12", "{sv}");
    assert_eq!(paras[2]["runs"][0]["url"], "https://example.com/docs");
    // PBIP and PBIX keep every byte of it
    let dir = temp("textbox");
    for file in ["Shop.pbip", "Shop.pbix"] {
        let back = reopened(&mut s, &dir, file);
        let w = find(report(&back), &name);
        assert_eq!(w.visual_extra["objects"], v.visual_extra["objects"], "{file}");
        assert_eq!(snap_visual(&back, &name)["text"], sv["text"], "{file}");
    }
    // a plain text replaces it all, the dynamic value goes
    let set = run(
        &mut s,
        "textbox.set",
        json!({"page": main, "visual": name, "text": "One\nTwo"}),
    );
    assert_eq!(set["paragraphs"], 2);
    let v = find(report(&s), &name);
    assert!(v.visual_extra["objects"].get("values").is_none());
    assert_eq!(
        v.visual_extra["objects"]["general"][0]["properties"]["paragraphs"],
        json!([{"textRuns": [{"value": "One"}]}, {"textRuns": [{"value": "Two"}]}])
    );
    // the same text again changes nothing
    s.report_dirty = false;
    let again = run(
        &mut s,
        "textbox.set",
        json!({"page": main, "visual": name, "text": "One\nTwo"}),
    );
    assert_eq!(again["changed"], false);
    assert!(!s.report_dirty);
    // undo takes the edit back
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(
        find(report(&s), &name).visual_extra["objects"]["values"][0]["selector"]["id"],
        "value1"
    );
}

#[test]
fn text_box_edits_refuse_bad_runs() {
    let mut s = session();
    let main = page(&s, 0);
    let name = run(&mut s, "textbox.add", json!({"page": main}))["visual"]
        .as_str()
        .unwrap()
        .to_string();
    let card = run(
        &mut s,
        "visual.add",
        json!({"page": main, "visual_type": "card", "x": 0, "y": 0, "width": 100, "height": 100}),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string();
    let set = |runs: Value| json!({"page": main, "visual": name, "paragraphs": [{"runs": runs}]});
    for (p, why) in [
        (set(json!([{"text": "x", "color": "red"}])), "colour"),
        (set(json!([{"text": "x", "url": "javascript:alert(1)"}])), "http"),
        (set(json!([{"text": "x", "url": "file:///etc/passwd"}])), "http"),
        (set(json!([{"text": "x", "value": "[Total]"}])), "not both"),
        (set(json!([{"value": "Sales[Region]"}])), "dynamic value"),
        (set(json!([{"value": "[Nope]"}])), "Nope"),
        (set(json!([{"text": "x", "font_size": 0}])), "font_size"),
        (set(json!([{"text": "x", "font_size": 1e300}])), "font_size"),
        (set(json!([{"text": "x", "bold": "yes"}])), "bold"),
        (set(json!([{"text": "x", "weird": 1}])), "unknown key"),
        (set(json!([{"text": 5}])), "text"),
        (set(json!("runs")), "runs"),
        (
            json!({"page": main, "visual": name, "paragraphs": [{"align": "justify", "runs": []}]}),
            "align",
        ),
        (
            json!({"page": main, "visual": name, "paragraphs": [{"list": "roman", "runs": []}]}),
            "list",
        ),
        (
            json!({"page": main, "visual": name, "paragraphs": "text"}),
            "list of",
        ),
        (
            json!({"page": main, "visual": name, "text": "a", "paragraphs": []}),
            "not both",
        ),
        (json!({"page": main, "visual": name}), "needs"),
        (
            json!({"page": main, "visual": card, "text": "a"}),
            "not a text box",
        ),
        (
            json!({"page": main, "visual": name, "text": "x".repeat(100_001)}),
            "characters",
        ),
        (
            json!({"page": main, "visual": name, "paragraphs": vec![json!({"runs": []}); 501]}),
            "paragraphs",
        ),
    ] {
        let e = fails(&mut s, "textbox.set", p.clone());
        assert!(e.contains(why), "{p}: {e}");
    }
}

#[test]
fn images_are_registered_resources_and_round_trip() {
    let mut s = session();
    let main = page(&s, 0);
    let out = run(
        &mut s,
        "image.add",
        json!({"page": main, "data": format!("data:image/png;base64,{PNG}"), "name": "logo.png", "scaling": "fit"}),
    );
    let name = out["visual"].as_str().unwrap().to_string();
    let item = out["resource"].as_str().unwrap().to_string();
    assert!(
        item.starts_with("logo") && item.ends_with(".png") && item.len() == "logo".len() + 16 + 4,
        "{item}"
    );
    assert_eq!(out["mime"], "image/png");
    assert_eq!(out["image_size"], json!({"width": 1.0, "height": 1.0}));
    let r = report(&s);
    let path = format!("StaticResources/RegisteredResources/{item}");
    assert!(r.resources.iter().any(|x| x.path == path), "{:?}", r.resources);
    assert_eq!(
        r.extra["resourcePackages"],
        json!([{"name": "RegisteredResources", "type": "RegisteredResources",
                "items": [{"name": item, "path": item, "type": "Image"}]}])
    );
    let v = find(r, &name);
    assert_eq!(v.visual_type.as_deref(), Some("image"));
    assert_eq!(
        v.visual_extra["objects"],
        json!({
            "general": [{"properties": {"imageUrl": {"expr": {"ResourcePackageItem":
                {"PackageName": "RegisteredResources", "PackageType": 1, "ItemName": item}}}}}],
            "imageScaling": [{"properties": {"imageScalingType": {"expr": {"Literal": {"Value": "'Fit'"}}}}}]
        })
    );
    let sv = snap_visual(&s, &name);
    assert_eq!(sv["image"]["mime"], "image/png", "{sv}");
    assert_eq!(sv["image"]["scaling"], "fit");
    let v0 = v.clone();
    // the same bytes again: the same file
    let again = run(
        &mut s,
        "image.add",
        json!({"page": main, "data": PNG, "name": "logo.png"}),
    );
    assert_eq!(again["resource"], item);
    assert_eq!(report(&s).resources.iter().filter(|x| x.path == path).count(), 1);
    // PBIP: the file on disk; PBIX: the package part and the legacy item type
    let dir = temp("image");
    for file in ["Shop.pbip", "Shop.pbix"] {
        let mut back = reopened(&mut s, &dir, file);
        let r = report(&back);
        let res = r
            .resources
            .iter()
            .find(|x| x.path == path)
            .unwrap_or_else(|| panic!("{file}: {:?}", r.resources));
        assert_eq!(res.bytes.len(), 70, "{file}");
        assert_eq!(
            find(r, &name).visual_extra["objects"],
            v0.visual_extra["objects"],
            "{file}"
        );
        assert_eq!(snap_visual(&back, &name)["image"]["mime"], "image/png", "{file}");
        let v = run(&mut back, "report.validate", json!({}));
        assert_eq!(v["error_count"], 0, "{file}: {v:#}");
    }
    assert!(
        dir.join("Shop.Report/StaticResources/RegisteredResources")
            .join(&item)
            .is_file()
    );
    // an SVG replaces it: the old file goes (nothing else shows it… but the second image does)
    let set = run(
        &mut s,
        "image.set",
        json!({"page": main, "visual": name, "data": b64(SVG.as_bytes()), "name": "chart.svg", "scaling": "fill"}),
    );
    let svg_item = set["resource"].as_str().unwrap().to_string();
    assert!(svg_item.starts_with("chart") && svg_item.ends_with(".svg"));
    assert!(
        report(&s).resources.iter().any(|x| x.path == path),
        "still shown by the second image"
    );
    let second = again["visual"].as_str().unwrap();
    run(
        &mut s,
        "image.set",
        json!({"page": main, "visual": second, "data": b64(SVG.as_bytes()), "name": "chart.svg"}),
    );
    assert!(
        !report(&s).resources.iter().any(|x| x.path == path),
        "the PNG is no longer shown"
    );
    let items = report(&s).extra["resourcePackages"][0]["items"].clone();
    assert_eq!(
        items,
        json!([{"name": svg_item, "path": svg_item, "type": "Image"}])
    );
    // scaling only
    let sc = run(
        &mut s,
        "image.set",
        json!({"page": main, "visual": second, "scaling": "normal"}),
    );
    assert_eq!(sc["changed"], true);
}

#[test]
fn hostile_images_are_refused() {
    let mut s = session();
    let main = page(&s, 0);
    let dir = temp("hostile-images");
    std::fs::write(dir.join("notes.png"), b"just text, named .png").unwrap();
    let mut huge = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    huge.extend_from_slice(&100_000u32.to_be_bytes());
    huge.extend_from_slice(&100_000u32.to_be_bytes());
    huge.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    let mut truncated = bicraft_test_png();
    truncated.truncate(40);
    let big = "A".repeat(23 << 20);
    for (p, why) in [
        (json!({"page": main}), "needs"),
        (json!({"page": main, "data": ""}), "needs"),
        (json!({"page": main, "data": "!!!"}), "base64"),
        (json!({"page": main, "data": "data:image/png,%89PNG"}), "base64"),
        (json!({"page": main, "data": b64(b"hello world")}), "not an image"),
        (json!({"page": main, "data": b64(&huge)}), "cannot be decoded"),
        (
            json!({"page": main, "data": b64(&truncated)}),
            "cannot be decoded",
        ),
        (
            json!({"page": main, "data": b64(b"<svg><g>")}),
            "cannot be decoded",
        ),
        (json!({"page": main, "data": b64(b"BM\0\0\0\0")}), "not an image"),
        (json!({"page": main, "data": big}), "longer than"),
        (
            json!({"page": main, "path": dir.join("notes.png")}),
            "not an image",
        ),
        (
            json!({"page": main, "path": dir.join("missing.png")}),
            "cannot read",
        ),
        (
            json!({"page": main, "path": dir.join("x.png"), "data": PNG}),
            "not both",
        ),
        (
            json!({"page": main, "data": PNG, "scaling": "stretch"}),
            "scaling",
        ),
        (json!({"page": "Nope", "data": PNG}), "no page"),
    ] {
        let e = fails(&mut s, "image.add", p.clone());
        assert!(
            e.contains(why),
            "{}: {e}",
            p.to_string().chars().take(200).collect::<String>()
        );
    }
    // a file larger than the cap is refused before it is read
    let f = std::fs::File::create(dir.join("big.png")).unwrap();
    f.set_len(17 << 20).unwrap();
    let e = fails(
        &mut s,
        "image.add",
        json!({"page": main, "path": dir.join("big.png")}),
    );
    assert!(e.contains("limit"), "{e}");
    // an SVG naming a local file is stored but drawn without it (the renderer reads nothing else)
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10"><image width="10" height="10" xlink:href="{}"/></svg>"#,
        dir.join("notes.png").display()
    );
    run(
        &mut s,
        "image.add",
        json!({"page": main, "data": b64(svg.as_bytes())}),
    );
}

/// The 1×1 PNG's bytes.
fn bicraft_test_png() -> Vec<u8> {
    let s = PNG;
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => break,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

#[test]
fn buttons_actions_and_reader_clicks() {
    let mut s = session();
    let (main, detail) = (page(&s, 0), page(&s, 1));
    // every kind Desktop's Insert › Buttons lists
    for kind in [
        "left_arrow",
        "right_arrow",
        "reset",
        "back",
        "information",
        "help",
        "qna",
        "bookmark",
        "blank",
        "apply_all_slicers",
        "clear_all_slicers",
        "page_navigator",
        "bookmark_navigator",
    ] {
        let out = run(&mut s, "button.add", json!({"page": main, "kind": kind}));
        let v = find(report(&s), out["visual"].as_str().unwrap());
        match kind {
            "page_navigator" => assert_eq!(v.visual_type.as_deref(), Some("pageNavigator")),
            "bookmark_navigator" => assert_eq!(v.visual_type.as_deref(), Some("bookmarkNavigator")),
            _ => {
                assert_eq!(v.visual_type.as_deref(), Some("actionButton"));
                assert_eq!(
                    v.visual_extra["objects"]["icon"][0]["selector"],
                    json!({"id": "default"})
                );
                assert_eq!((v.position.width, v.position.height), (100.0, 40.0));
            }
        }
    }
    let back = find(report(&s), &report(&s).pages[0].visuals[3].name).clone();
    assert_eq!(
        back.visual_extra["visualContainerObjects"]["visualLink"],
        json!([{"properties": {"show": {"expr": {"Literal": {"Value": "true"}}}, "type": {"expr": {"Literal": {"Value": "'Back'"}}}}}])
    );
    assert_eq!(
        back.visual_extra["objects"]["icon"][0]["properties"]["shapeType"]["expr"]["Literal"]["Value"],
        "'back'"
    );
    // a blank button navigates to Detail
    let blank = report(&s).pages[0].visuals[8].name.clone();
    let out = run(
        &mut s,
        "visual.set_action",
        json!({"page": main, "visual": blank, "type": "page_navigation", "destination": "Detail", "tooltip": "Go"}),
    );
    assert_eq!(out["destination"], detail);
    let link =
        &find(report(&s), &blank).visual_extra["visualContainerObjects"]["visualLink"][0]["properties"];
    assert_eq!(link["type"]["expr"]["Literal"]["Value"], "'PageNavigation'");
    assert_eq!(
        link["navigationSection"]["expr"]["Literal"]["Value"],
        format!("'{detail}'")
    );
    assert_eq!(link["tooltip"]["expr"]["Literal"]["Value"], "'Go'");
    let sv = snap_visual(&s, &blank);
    assert_eq!(
        sv["action"],
        json!({"type": "page_navigation", "destination": detail, "tooltip": "Go"})
    );
    assert_eq!(sv["interaction"]["button"]["kind"], "PageNavigation", "{sv}");
    let r = run(
        &mut s,
        "view.interact",
        json!({"action": "button", "page": main, "visual": blank}),
    );
    assert_eq!(r["state"]["page"], detail, "{r}");
    run(&mut s, "view.interact", json!({"action": "reset"}));
    // a web link: the reader is told the URL, nothing else changes
    run(
        &mut s,
        "visual.set_action",
        json!({"page": main, "visual": blank, "type": "web_url", "url": "https://example.com/"}),
    );
    let r = run(
        &mut s,
        "view.interact",
        json!({"action": "button", "page": main, "visual": blank}),
    );
    assert_eq!(r["url"], "https://example.com/");
    assert_eq!(r["changed"], false);
    // drill through goes to a drillthrough page only
    let e = fails(
        &mut s,
        "visual.set_action",
        json!({"page": main, "visual": blank, "type": "drillthrough", "destination": "Detail"}),
    );
    assert!(e.contains("drillthrough page"), "{e}");
    run(
        &mut s,
        "page.set_type",
        json!({"page": "Detail", "type": "drillthrough", "fields": ["Sales[Region]"]}),
    );
    run(
        &mut s,
        "visual.set_action",
        json!({"page": main, "visual": blank, "type": "drillthrough", "destination": "Detail"}),
    );
    // the reader drills through the selected data point (none here: refused, as Desktop greys it)
    let e = s
        .execute(
            "view.interact",
            &json!({"action": "button", "page": main, "visual": blank}),
        )
        .unwrap_err();
    assert!(e.contains("select one data point"), "{e}");
    // bookmarks are written, not applied by the reader yet
    let out = run(
        &mut s,
        "visual.set_action",
        json!({"page": main, "visual": blank, "type": "bookmark", "bookmark": "B1"}),
    );
    assert!(out["warnings"][0].as_str().unwrap().contains("no bookmark"));
    let e = s
        .execute(
            "view.interact",
            &json!({"action": "button", "page": main, "visual": blank}),
        )
        .unwrap_err();
    assert!(e.contains("bookmark"), "{e}");
    // an image takes an action too; off again with none
    let img = run(&mut s, "image.add", json!({"page": main, "data": PNG}))["visual"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut s,
        "visual.set_action",
        json!({"page": main, "visual": img, "type": "back"}),
    );
    assert_eq!(snap_visual(&s, &img)["interaction"]["button"]["kind"], "Back");
    run(
        &mut s,
        "visual.set_action",
        json!({"page": main, "visual": img, "type": "none"}),
    );
    assert_eq!(snap_visual(&s, &img)["action"], json!({"type": null}));
    assert!(snap_visual(&s, &img)["interaction"].get("button").is_none());
    // refusals
    let card = run(
        &mut s,
        "visual.add",
        json!({"page": main, "visual_type": "card", "x": 0, "y": 0, "width": 100, "height": 100}),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string();
    for (p, why) in [
        (
            json!({"page": main, "visual": card, "type": "back"}),
            "buttons, images and shapes",
        ),
        (
            json!({"page": main, "visual": blank, "type": "teleport"}),
            "must be one of",
        ),
        (
            json!({"page": main, "visual": blank, "type": "web_url", "url": "javascript:x"}),
            "http",
        ),
        (
            json!({"page": main, "visual": blank, "type": "back", "url": "https://x.org"}),
            "does not apply",
        ),
        (
            json!({"page": main, "visual": blank, "type": "page_navigation", "destination": "Nope"}),
            "no page",
        ),
        (
            json!({"page": main, "visual": blank, "type": "back", "tooltip": "t".repeat(1001)}),
            "longer",
        ),
    ] {
        let e = fails(&mut s, "visual.set_action", p.clone());
        assert!(e.contains(why), "{p}: {e}");
    }
    let e = fails(&mut s, "button.add", json!({"page": main, "kind": "rocket"}));
    assert!(e.contains("must be one of"), "{e}");
    // PBIX and PBIP keep the actions
    let dir = temp("buttons");
    for file in ["Shop.pbip", "Shop.pbix"] {
        let back = reopened(&mut s, &dir, file);
        assert_eq!(
            find(report(&back), &blank).visual_extra["visualContainerObjects"],
            find(report(&s), &blank).visual_extra["visualContainerObjects"],
            "{file}"
        );
    }
}

#[test]
fn page_format_background_wallpaper_size_and_alignment() {
    let mut s = session();
    let main = page(&s, 0);
    let out = run(
        &mut s,
        "page.format",
        json!({"page": "Main", "background_color": "#fafafa", "background_transparency": 20,
               "background_image": PNG, "background_scaling": "fill",
               "wallpaper_color": "#203040", "wallpaper_transparency": 0,
               "size": "4:3", "vertical_alignment": "middle"}),
    );
    assert_eq!(
        (out["width"].as_f64(), out["height"].as_f64()),
        (Some(960.0), Some(720.0))
    );
    let item = out["images"]["background"].as_str().unwrap().to_string();
    let p = &report(&s).pages[0];
    let lit = |v: &str| json!({"expr": {"Literal": {"Value": v}}});
    assert_eq!(
        p.extra["objects"],
        json!({
            "background": [{"properties": {
                "color": {"solid": {"color": lit("'#FAFAFA'")}},
                "transparency": lit("20D"),
                "image": {"image": {"name": lit(&format!("'{item}'")),
                    "url": {"expr": {"ResourcePackageItem": {"PackageName": "RegisteredResources", "PackageType": 1, "ItemName": item}}},
                    "scaling": lit("'Fill'")}}
            }}],
            "outspace": [{"properties": {"color": {"solid": {"color": lit("'#203040'")}}, "transparency": lit("0D")}}],
            "displayArea": [{"properties": {"verticalAlignment": lit("'Middle'")}}]
        })
    );
    let snap = s.snapshot();
    let sp = &snap["report"]["pages"][0];
    assert_eq!(sp["background"], "#FAFAFA", "{sp}");
    assert_eq!(sp["background_transparency"], 20);
    assert_eq!(sp["background_image"]["fit"], "fill");
    assert_eq!(sp["wallpaper"]["color"], "#203040");
    assert_eq!(sp["size"], "4:3");
    assert_eq!(sp["vertical_alignment"], "middle");
    // drawn: the page's background colour is in the PNG
    let r = run(
        &mut s,
        "report.render",
        json!({"page": "Main", "system_fonts": false}),
    );
    assert!(r["pages"][0]["width"].as_f64().is_some_and(|w| w > 0.0), "{r}");
    // round trips
    let dir = temp("page-format");
    for file in ["Shop.pbip", "Shop.pbix"] {
        let back = reopened(&mut s, &dir, file);
        let bp = &report(&back).pages[0];
        assert_eq!(
            bp.extra["objects"],
            report(&s).pages[0].extra["objects"],
            "{file}"
        );
        assert_eq!((bp.width, bp.height), (960.0, 720.0), "{file}");
        assert!(
            report(&back).resources.iter().any(|x| x.path.ends_with(&item)),
            "{file}"
        );
    }
    // a custom size, then the image removed: its resource goes
    run(
        &mut s,
        "page.format",
        json!({"page": main, "size": "custom", "width": 1000, "height": 2000}),
    );
    assert_eq!(
        (report(&s).pages[0].width, report(&s).pages[0].height),
        (1000.0, 2000.0)
    );
    assert_eq!(s.snapshot()["report"]["pages"][0]["size"], "custom");
    run(
        &mut s,
        "page.format",
        json!({"page": main, "background_image": null, "background_color": null}),
    );
    let p = &report(&s).pages[0];
    assert_eq!(
        p.extra["objects"]["background"],
        json!([{"properties": {"transparency": lit("20D")}}])
    );
    assert!(!report(&s).resources.iter().any(|x| x.path.ends_with(&item)));
    // refusals
    for (p, why) in [
        (json!({"page": main}), "needs something"),
        (json!({"page": main, "background_color": "blue"}), "colour"),
        (json!({"page": main, "background_transparency": 101}), "0 to 100"),
        (json!({"page": main, "size": "a4"}), "must be one of"),
        (json!({"page": main, "size": "16:9", "width": 10}), "custom"),
        (json!({"page": main, "size": "custom"}), "needs"),
        (json!({"page": main, "width": -5}), "width"),
        (
            json!({"page": main, "vertical_alignment": "bottom"}),
            "top or middle",
        ),
        (
            json!({"page": main, "wallpaper_image": b64(b"GIF89a")}),
            "decoded",
        ),
        (
            json!({"page": main, "wallpaper_scaling": "zoom"}),
            "normal, fit or fill",
        ),
    ] {
        let e = fails(&mut s, "page.format", p.clone());
        assert!(e.contains(why), "{p}: {e}");
    }
}

#[test]
fn image_add_reads_a_file() {
    let mut s = session();
    let main = page(&s, 0);
    let dir = temp("replay");
    std::fs::write(dir.join("logo.png"), bicraft_test_png()).unwrap();
    let out = run(
        &mut s,
        "image.add",
        json!({"page": main, "path": dir.join("logo.png")}),
    );
    assert!(out["resource"].as_str().unwrap().starts_with("logo"));
    assert_eq!(out["position"]["width"], 1.0);
}

#[test]
fn shapes_are_inserted_formatted_and_round_trip() {
    let mut s = session();
    let main = page(&s, 0);
    // tile shapes of Insert › Shapes
    for tile in [
        "rectangle",
        "rectangleRounded",
        "oval",
        "line",
        "triangleIsoc",
        "triangleRight",
        "arrow",
        "heart",
        "tabRoundTopCorners",
    ] {
        let out = run(&mut s, "shape.add", json!({"page": main, "shape": tile}));
        assert_eq!(out["shape"], tile);
        let v = find(report(&s), out["visual"].as_str().unwrap());
        assert_eq!(v.visual_type.as_deref(), Some("shape"));
        // the tile shape is written without selector, where Desktop reads it
        assert_eq!(
            v.visual_extra["objects"]["shape"],
            json!([{"properties": {"tileShape": {"expr": {"Literal": {"Value": format!("'{tile}'")}}}}}])
        );
        assert_eq!(v.extra["howCreated"], "InsertVisualButton");
        let size = (v.position.width, v.position.height);
        assert_eq!(
            size,
            if tile == "line" {
                (300.0, 40.0)
            } else {
                (300.0, 200.0)
            }
        );
        assert_eq!(snap_visual(&s, &v.name)["shape"]["tile"], tile);
    }
    // the case of the name does not matter; an unknown shape or key is refused
    let oval = run(
        &mut s,
        "shape.add",
        json!({"page": main, "shape": "OVAL", "x": 10, "y": 20}),
    );
    assert_eq!(oval["shape"], "oval");
    assert_eq!(oval["position"]["x"], 10.0);
    let e = fails(&mut s, "shape.add", json!({"page": main, "shape": "cloud"}));
    assert!(e.contains("must be one of"), "{e}");
    let e = fails(
        &mut s,
        "shape.add",
        json!({"page": main, "shape": "oval", "colour": "red"}),
    );
    assert!(e.contains("colour"), "{e}");
    // the Format pane's cards: fill and outline in the default state, rotation without selector
    let name = oval["visual"].as_str().unwrap().to_string();
    run(
        &mut s,
        "visual.format",
        json!({"page": main, "visual": name, "objects": [
            {"object": "fill", "property": "fillColor", "value": {"color": "#FF0000"}},
            {"object": "fill", "property": "transparency", "value": 25},
            {"object": "outline", "property": "show", "value": false},
            {"object": "rotation", "property": "shapeAngle", "value": 45},
            {"object": "shape", "property": "tileShape", "value": "hexagon"},
        ]}),
    );
    let v = find(report(&s), &name).clone();
    let objects = &v.visual_extra["objects"];
    assert_eq!(
        objects["fill"][0]["selector"],
        json!({"id": "default"}),
        "{objects}"
    );
    assert!(objects["rotation"][0].get("selector").is_none(), "{objects}");
    assert_eq!(objects["shape"].as_array().unwrap().len(), 1, "{objects}");
    let sh = snap_visual(&s, &name)["shape"].clone();
    assert_eq!(sh["tile"], "hexagon", "{sh}");
    assert_eq!(sh["fill"], "#FF0000", "{sh}");
    assert_eq!(sh["fill_transparency"], 25.0, "{sh}");
    assert_eq!(sh["show_line"], false, "{sh}");
    assert_eq!(sh["angle"], 45.0, "{sh}");
    // PBIP and PBIX keep the shape and its formatting
    let dir = temp("shapes");
    for file in ["Shop.pbip", "Shop.pbix"] {
        let back = reopened(&mut s, &dir, file);
        assert_eq!(
            find(report(&back), &name).visual_extra["objects"],
            v.visual_extra["objects"],
            "{file}"
        );
        assert_eq!(snap_visual(&back, &name)["shape"], sh, "{file}");
    }
}
