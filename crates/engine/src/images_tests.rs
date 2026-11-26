//! Snapshot images: wallpaper, page background, image visuals and visual backgrounds resolved
//! from report resources, and the assets the viewer serves.

use std::sync::Arc;

use bicraft_report::{Page, Report, Resource, Visual};
use serde_json::{Value, json};

use super::{MAX_IMAGE_BYTES, sha256};
use crate::Session;

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in bytes {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 == 1 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

/// A real, minimal RGBA PNG (stored deflate block) of `w` x `h` pixels of one colour.
fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let row: Vec<u8> = std::iter::once(0)
        .chain(rgba.iter().copied().cycle().take(4 * w as usize))
        .collect();
    let raw = row.repeat(h as usize);
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    let len = u16::try_from(raw.len()).unwrap();
    let mut z = vec![0x78, 0x01, 0x01];
    z.extend_from_slice(&len.to_le_bytes());
    z.extend_from_slice(&(!len).to_le_bytes());
    z.extend_from_slice(&raw);
    z.extend_from_slice(&(b << 16 | a).to_be_bytes());
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in [(&b"IHDR"[..], ihdr), (b"IDAT", z), (b"IEND", Vec::new())] {
        out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(&data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    }
    out
}

fn id_of(bytes: &[u8]) -> String {
    sha256(bytes)
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The snapshot's assets without the gallery icons of BIcraft's own visuals, which every report
/// carries (ROADMAP M12.4): the report's images.
fn report_assets(s: &Session) -> Vec<super::SnapshotAsset> {
    let own: Vec<String> = crate::builtin_visuals::VISUALS
        .iter()
        .filter_map(|b| (b.icon)(b.guid).ok())
        .map(|png| id_of(&png))
        .collect();
    s.snapshot_assets()
        .into_iter()
        .filter(|a| !own.contains(&a.id))
        .collect()
}

fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn lit(v: &str) -> Value {
    json!({"expr": {"Literal": {"Value": v}}})
}

fn item(name: &str) -> Value {
    json!({"expr": {"ResourcePackageItem": {"PackageName": "RegisteredResources", "PackageType": 1, "ItemName": name}}})
}

fn image_prop(url: Value, scaling: &str) -> Value {
    json!({"image": {"name": lit("'pic.png'"), "url": url, "scaling": lit(scaling)}})
}

fn resource(path: &str, bytes: Vec<u8>) -> Resource {
    Resource {
        path: path.into(),
        bytes,
    }
}

fn image_visual(name: &str, url: Value, scaling: Option<&str>) -> Visual {
    let mut v = Visual {
        name: name.into(),
        visual_type: Some("image".into()),
        ..Default::default()
    };
    let mut objects = json!({"general": [{"properties": {"imageUrl": url}}]});
    if let Some(s) = scaling {
        objects["imageScaling"] = json!([{"properties": {"imageScalingType": lit(s)}}]);
    }
    v.visual_extra.insert("objects".into(), objects);
    v
}

/// One page with a wallpaper, a page background image, two image visuals (one points at the
/// same file as the page background), and a table with a background image given as a data URL.
fn report_with_images() -> (Report, Vec<u8>, Vec<u8>, Vec<u8>) {
    let band = png(4, 2, [0x10, 0x20, 0x60, 0xFF]);
    let overlay = png(3, 5, [0, 0, 0, 0x80]);
    let tile = png(2, 2, [0xFF, 0, 0, 0xFF]);
    let mut page = Page {
        name: "p1".into(),
        display_name: "One".into(),
        width: 1920.0,
        height: 1080.0,
        ..Default::default()
    };
    let bg = image_prop(item("band-123.png"), "'Fit'");
    page.extra.insert(
        "objects".into(),
        json!({
            "background": [{"properties": {
                "color": {"solid": {"color": lit("'#F6F7FA'")}}, "transparency": lit("20D"), "image": bg}}],
            "outspace": [{"properties": {
                "color": {"solid": {"color": lit("'#EAEAF3'")}}, "transparency": lit("0D"),
                "image": image_prop(item("overlay.png"), "'Fill'")}}]
        }),
    );
    let mut table = Visual {
        name: "t1".into(),
        visual_type: Some("tableEx".into()),
        ..Default::default()
    };
    let data_url = format!("'data:image/png;base64,{}'", base64(&tile));
    table.visual_extra.insert(
        "visualContainerObjects".into(),
        json!({"background": [{"properties": {"show": lit("true"), "transparency": lit("0D"),
                                              "image": image_prop(lit(&data_url), "'Normal'")}}]}),
    );
    page.visuals = vec![
        image_visual("i1", item("overlay.png"), Some("'Fit'")),
        image_visual("i2", item("band-123.png"), None),
        table,
    ];
    let report = Report {
        pages: vec![page],
        extra: serde_json::from_value(json!({"resourcePackages": [{
            "name": "RegisteredResources", "type": "RegisteredResources",
            "items": [
                {"name": "band-123.png", "path": "band-123.png", "type": "Image"},
                {"name": "overlay.png", "path": "sub/overlay-file.png", "type": "Image"}
            ]
        }]}))
        .unwrap(),
        resources: vec![
            resource("StaticResources/RegisteredResources/band-123.png", band.clone()),
            resource(
                "StaticResources/RegisteredResources/sub/overlay-file.png",
                overlay.clone(),
            ),
            resource(
                "StaticResources/RegisteredResources/theme.json",
                b"{\"name\":\"t\"}".to_vec(),
            ),
        ],
        ..Default::default()
    };
    (report, band, overlay, tile)
}

fn session(report: Report) -> Session {
    Session {
        report: Some(report),
        ..Default::default()
    }
}

#[test]
fn snapshot_carries_wallpaper_backgrounds_and_image_visuals() {
    let (report, band, overlay, tile) = report_with_images();
    let s = session(report);
    let snap = s.snapshot();
    let page = &snap["report"]["pages"][0];
    let band_img = json!({"id": id_of(&band), "mime": "image/png", "width": 4, "height": 2});
    let overlay_img = json!({"id": id_of(&overlay), "mime": "image/png", "width": 3, "height": 5});
    let tile_img = json!({"id": id_of(&tile), "mime": "image/png", "width": 2, "height": 2});
    assert_eq!(
        page["wallpaper"],
        json!({"color": "#EAEAF3", "transparency": 0, "image": overlay_img, "fit": "fill"})
    );
    assert_eq!(page["background"], "#F6F7FA");
    assert_eq!(page["background_transparency"], 20);
    assert_eq!(
        page["background_image"],
        json!({"image": band_img, "fit": "fit", "transparency": 20})
    );
    let visuals = &page["visuals"];
    let mut i1 = overlay_img.clone();
    i1["scaling"] = json!("fit");
    assert_eq!(visuals[0]["image"], i1);
    // no imageScaling: `normal` (Desktop draws it like CSS `contain`)
    let mut i2 = band_img.clone();
    i2["scaling"] = json!("normal");
    assert_eq!(visuals[1]["image"], i2);
    let bg = &visuals[2]["format"]["background"];
    assert_eq!(bg["image"], tile_img);
    assert_eq!(bg["image_fit"], "normal");
    assert_eq!(bg["show"], true);
    assert!(visuals[2].get("image").is_none());

    // The assets are exactly the referenced images, deduplicated, ids matching the snapshot.
    let assets = report_assets(&s);
    let mut got: Vec<(String, &str, Vec<u8>)> = assets
        .iter()
        .map(|a| (a.id.clone(), a.mime, a.bytes.to_vec()))
        .collect();
    got.sort();
    let mut want = vec![
        (id_of(&band), "image/png", band),
        (id_of(&overlay), "image/png", overlay),
        (id_of(&tile), "image/png", tile),
    ];
    want.sort();
    assert_eq!(got, want);
    // Image bytes never appear in the snapshot.
    let text = snap.to_string();
    assert!(!text.contains("base64"), "data URL leaked into the snapshot");
    assert!(text.len() < 20_000);
}

#[test]
fn assets_are_cached_per_revision() {
    let (report, ..) = report_with_images();
    let mut s = session(report);
    let a = report_assets(&s);
    let b = report_assets(&s);
    assert_eq!(a.len(), 3);
    assert!(a.iter().zip(&b).all(|(x, y)| Arc::ptr_eq(&x.bytes, &y.bytes)));
    // A new revision with another picture: recomputed.
    let other = png(1, 1, [1, 2, 3, 4]);
    if let Some(r) = s.report.as_mut() {
        r.resources[0].bytes = other.clone();
    }
    s.revision += 1;
    let c = report_assets(&s);
    assert!(c.iter().any(|x| x.id == id_of(&other)));
    assert_eq!(
        s.snapshot()["report"]["pages"][0]["background_image"]["image"]["id"],
        id_of(&other)
    );
}

#[test]
fn hostile_or_unsupported_images_are_left_out() {
    let (mut report, ..) = report_with_images();
    let huge = {
        let mut b = png(1, 1, [0, 0, 0, 0]);
        b.resize(MAX_IMAGE_BYTES + 1, 0);
        b
    };
    report
        .resources
        .push(resource("StaticResources/RegisteredResources/huge.png", huge));
    report.resources.push(resource(
        "StaticResources/RegisteredResources/page.html",
        b"<html><script>alert(1)</script></html>".to_vec(),
    ));
    report
        .resources
        .push(resource("StaticResources/secret.png", png(1, 1, [9, 9, 9, 9])));
    let urls = [
        item("missing.png"),
        item("theme.json"),
        item("huge.png"),
        item("page.html"),
        item("../secret.png"),
        lit("'https://example.com/x.png'"),
        lit("'data:image/png;base64,@@@'"),
        lit("'data:text/html,<script>alert(1)</script>'"),
        json!({"expr": {"ResourcePackageItem": {"PackageName": 5}}}),
        json!("not an expression"),
        json!(null),
    ];
    let page = &mut report.pages[0];
    page.extra.clear();
    page.visuals = urls
        .iter()
        .enumerate()
        .map(|(i, u)| image_visual(&format!("bad{i}"), u.clone(), Some("'Stretchy'")))
        .collect();
    let s = session(report);
    let snap = s.snapshot();
    let page = &snap["report"]["pages"][0];
    for (i, v) in page["visuals"].as_array().unwrap().iter().enumerate() {
        assert!(v.get("image").is_none(), "visual {i}: {v}");
    }
    assert!(page.get("wallpaper").is_none());
    assert!(page.get("background_image").is_none());
    assert!(report_assets(&s).is_empty());
}

#[test]
fn theme_page_styles_provide_wallpaper_and_background_images() {
    let pic = png(2, 3, [0, 0x80, 0, 0xFF]);
    let theme = json!({
        "name": "Pics",
        "visualStyles": {"page": {"*": {
            "outspace": [{"color": {"solid": {"color": "#101820"}}, "transparency": 30,
                          "image": {"name": "w.png", "url": format!("data:image/png;base64,{}", base64(&pic)), "scaling": "Normal"}}],
            "background": [{"transparency": 10,
                            "image": {"name": "b.png", "url": format!("data:image/png;base64,{}", base64(&pic)), "scaling": "Fill"}}]
        }}}
    });
    let page = Page {
        name: "p".into(),
        display_name: "P".into(),
        width: 100.0,
        height: 100.0,
        ..Default::default()
    };
    let s = session(Report {
        pages: vec![page],
        theme_collection: Some(json!({"customTheme": {"name": "Pics.json", "type": "RegisteredResources"}})),
        resources: vec![resource(
            "StaticResources/RegisteredResources/Pics.json",
            theme.to_string().into_bytes(),
        )],
        ..Default::default()
    });
    let snap = s.snapshot();
    let page = &snap["report"]["pages"][0];
    let img = json!({"id": id_of(&pic), "mime": "image/png", "width": 2, "height": 3});
    assert_eq!(
        page["wallpaper"],
        json!({"color": "#101820", "transparency": 30, "image": img, "fit": "normal"})
    );
    assert_eq!(
        page["background_image"],
        json!({"image": img, "fit": "fill", "transparency": 10})
    );
    // one distinct picture
    assert_eq!(report_assets(&s).len(), 1);
}

#[test]
fn a_page_with_only_a_wallpaper_color() {
    let mut page = Page {
        name: "p".into(),
        display_name: "P".into(),
        ..Default::default()
    };
    page.extra.insert(
        "objects".into(),
        json!({"outspace": [{"properties": {"color": {"solid": {"color": lit("'#123456'")}}}}]}),
    );
    let s = session(Report {
        pages: vec![page, Page::default()],
        ..Default::default()
    });
    let snap = s.snapshot();
    assert_eq!(
        snap["report"]["pages"][0]["wallpaper"],
        json!({"color": "#123456", "transparency": 0, "image": null})
    );
    // nothing set: no wallpaper key (the viewer keeps its default)
    assert!(snap["report"]["pages"][1].get("wallpaper").is_none());
    assert!(report_assets(&s).is_empty());
    assert!(Session::new().snapshot_assets().is_empty());
}
