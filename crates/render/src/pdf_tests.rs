//! PDF output tests: every page becomes one PDF page at its size, read back by our own
//! structural reader (`pdf_read.rs`): cross-reference table, objects, fonts embedded with a
//! ToUnicode map, text extracted, images, and the raster fallback of a visual too large to record.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::*;

fn opts() -> RenderOptions {
    RenderOptions {
        system_fonts: false,
        ..RenderOptions::default()
    }
}

fn report(pages: Value) -> Value {
    json!({
        "version": 1,
        "model": {"culture": "en-US", "tables": []},
        "report": {"theme": null, "pages": pages}
    })
}

fn chart(name: &str, title: &str) -> Value {
    json!({"name": name, "type": "clusteredColumnChart", "x": 20, "y": 60, "width": 400, "height": 250, "title": title,
           "fields": [{"role": "Category", "kind": "column", "table": "Sales", "name": "Region"},
                      {"role": "Y", "kind": "measure", "table": "Sales", "name": "Total"}],
           "data": {"status": "ok",
                    "columns": [{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"},
                                {"name": "Total", "role": "Y", "kind": "measure"}],
                    "rows": [["North", 120.0], ["South", 80.0], ["East", 45.5]]}})
}

fn textbox(name: &str, text: &str) -> Value {
    json!({"name": name, "type": "textbox", "x": 20, "y": 10, "width": 500, "height": 40,
           "text": {"paragraphs": [{"runs": [{"text": text, "font_size": 18, "bold": true}]}]}})
}

fn sample() -> Value {
    report(json!([
        {"name": "p1", "display_name": "Overview", "width": 1280, "height": 720,
         "visuals": [textbox("t1", "Sales overview Δ 2025"), chart("c1", "Total by Region")]},
        {"name": "p2", "display_name": "Details", "width": 800, "height": 1200,
         "visuals": [textbox("t2", "Details page")]}
    ]))
}

fn pdf(s: &Value, pages: &[usize]) -> PdfDocument {
    let fonts = FontBook::bundled_only();
    render_pdf(
        s,
        pages,
        &HashMap::<String, Vec<u8>>::new(),
        &fonts,
        &opts(),
        "Sales",
    )
    .unwrap()
}

#[test]
fn pages_fonts_and_text_read_back() {
    let doc = pdf(&sample(), &[0, 1]);
    let info = pdf_read::inspect(&doc.bytes).unwrap();
    assert_eq!(info.version, "1.7");
    assert_eq!(info.title.as_deref(), Some("Sales"));
    // one PDF page per report page, at its aspect ratio (0.75 pt per report pixel)
    assert_eq!(info.pages.len(), 2);
    assert_eq!((info.pages[0].width, info.pages[0].height), (960.0, 540.0));
    assert_eq!((info.pages[1].width, info.pages[1].height), (600.0, 900.0));
    assert_eq!((doc.pages[1].width_pt, doc.pages[1].height_pt), (600.0, 900.0));
    // the text is real text: titles, labels and a Greek letter from the bundled Noto fallback
    let t0 = &info.pages[0].text;
    for want in [
        "Sales overview Δ 2025",
        "Total by Region",
        "North",
        "South",
        "East",
    ] {
        assert!(t0.contains(want), "`{want}` not in {t0:?}");
    }
    assert!(
        info.pages[1].text.contains("Details page"),
        "{:?}",
        info.pages[1].text
    );
    assert!(!info.pages[1].text.contains("North"));
    // every font is embedded as a subset, with a ToUnicode map
    assert!(!info.fonts.is_empty());
    for f in &info.fonts {
        assert!(f.embedded && f.to_unicode && f.subtype == "Type0", "{f:?}");
        assert!(f.base_font.chars().nth(6) == Some('+'), "{f:?}");
    }
    assert!(
        info.fonts.iter().any(|f| f.base_font.contains("Selawik")),
        "{:?}",
        info.fonts
    );
    assert!(
        info.fonts.iter().any(|f| f.base_font.contains("Noto")),
        "{:?}",
        info.fonts
    );
    assert!(doc.fonts.iter().all(|f| f.embedded && f.glyphs > 0));
    // vector, not images: no image drawn, nothing placed as a raster
    assert_eq!(info.pages[0].images, 0);
    assert_eq!(doc.pages[0].raster_parts, 0);
    // size sanity: font subsets keep the file small
    assert!(doc.bytes.len() < 200_000, "{} bytes", doc.bytes.len());
    assert!(doc.bytes.len() > 2_000);
}

#[test]
fn output_is_deterministic_and_structured() {
    let a = pdf(&sample(), &[0]);
    let b = pdf(&sample(), &[0]);
    assert_eq!(a.bytes, b.bytes);
    // the golden structure: header, objects, xref table, trailer
    let s = String::from_utf8_lossy(&a.bytes);
    assert!(s.starts_with("%PDF-1.7\n"));
    assert!(s.ends_with("%%EOF\n"));
    for key in [
        "/Type /Catalog",
        "/Type /Pages",
        "/Type /Page ",
        "/Subtype /Type0",
        "/Encoding /Identity-H",
        "/Subtype /CIDFontType2",
        "/CIDToGIDMap /Identity",
        "/FontFile2",
        "/ToUnicode",
        "/Type /FontDescriptor",
        "/Producer (BIcraft",
    ] {
        assert!(s.contains(key), "{key} missing");
    }
    let info = pdf_read::inspect(&a.bytes).unwrap();
    // catalog, pages, info, resources, a content stream and a page, and per font: type 0,
    // descendant, descriptor, file and ToUnicode
    assert!(info.objects >= 6 + 5 * info.fonts.len(), "{}", info.objects);
}

#[test]
fn transparency_images_and_previews() {
    let mut pm = tiny_skia::Pixmap::new(4, 4).unwrap();
    pm.fill(tiny_skia::Color::from_rgba8(0, 0, 255, 128));
    let id = "0123456789abcdef0123456789abcdef".to_string();
    let mut assets = HashMap::new();
    assets.insert(id.clone(), pm.encode_png().unwrap());
    let mut no_data = chart("c2", "Preview chart");
    no_data["data"] = json!({"status": "not_evaluated"});
    let mut s = report(
        json!([{"name": "p1", "display_name": "P", "width": 640, "height": 360,
        "visuals": [no_data, {"name": "im", "type": "image", "x": 450, "y": 10, "width": 100, "height": 100,
                              "image": {"id": id, "mime": "image/png", "width": 4, "height": 4}}]}]),
    );
    s["report"]["pages"][0]["background_image"] =
        json!({"image": {"id": id, "mime": "image/png", "width": 4, "height": 4}, "fit": "fit"});
    let fonts = FontBook::bundled_only();
    let doc = render_pdf(&s, &[0], &assets, &fonts, &opts(), "T").unwrap();
    let info = pdf_read::inspect(&doc.bytes).unwrap();
    let text = &info.pages[0].text;
    // the faded preview is a transparency group whose text is still there
    assert!(text.contains("Preview chart"), "{text:?}");
    assert!(String::from_utf8_lossy(&doc.bytes).contains("/S /Transparency"));
    assert!(String::from_utf8_lossy(&doc.bytes).contains("/SMask"));
    assert!(info.pages[0].images >= 1, "{:?}", info.pages[0]);
}

#[test]
fn a_visual_too_large_to_record_is_placed_as_an_image() {
    let s = sample();
    let fonts = FontBook::bundled_only();
    let assets = HashMap::<String, Vec<u8>>::new();
    let small = pdf_with_cap(&s, &[0], &assets, &fonts, &opts(), "T", 40).unwrap();
    let full = pdf(&s, &[0]);
    assert_eq!(full.pages[0].raster_parts, 0);
    assert!(small.pages[0].raster_parts >= 1, "{:?}", small.pages[0]);
    let info = pdf_read::inspect(&small.bytes).unwrap();
    assert!(info.pages[0].images >= 1);
    // the text box still records as text
    assert!(
        info.pages[0].text.contains("Sales overview"),
        "{:?}",
        info.pages[0].text
    );
}

#[test]
fn missing_glyphs_and_hostile_pages_do_not_crash() {
    let s = report(
        json!([{"name": "p1", "display_name": "P", "width": 1e9, "height": 400,
        "visuals": [textbox("t", "北京 \u{200b}\u{fe0f} ok"), {"name": "x", "type": "card", "x": f64::MAX}]}]),
    );
    let doc = pdf(&s, &[0]);
    let info = pdf_read::inspect(&doc.bytes).unwrap();
    assert_eq!(info.pages.len(), 1);
    assert!(info.pages[0].text.contains("ok"), "{:?}", info.pages[0].text);
    // a page index past the end is an error, not a panic
    let fonts = FontBook::bundled_only();
    assert!(render_pdf(&s, &[7], &HashMap::<String, Vec<u8>>::new(), &fonts, &opts(), "").is_err());
    // no page: an empty document that still reads
    let empty = pdf(&s, &[]);
    assert_eq!(pdf_read::inspect(&empty.bytes).unwrap().pages.len(), 0);
}

#[test]
fn every_truncation_of_a_pdf_is_an_error_or_a_reading() {
    let doc = pdf(&sample(), &[0]);
    for cut in (0..doc.bytes.len()).step_by(61) {
        let _ = pdf_read::inspect(&doc.bytes[..cut]);
    }
    let mut flipped = doc.bytes.clone();
    for i in (0..flipped.len()).step_by(97) {
        flipped[i] ^= 0x55;
    }
    let _ = pdf_read::inspect(&flipped);
}

#[test]
fn rendering_raster_is_unchanged_by_recording() {
    // the PNG of a page is the same whether or not canvases record vectors
    let s = sample();
    let fonts = FontBook::bundled_only();
    let a = render_page(&s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &opts()).unwrap();
    let b = {
        let _v = canvas::VectorScope::new();
        render_page(&s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &opts()).unwrap()
    };
    assert_eq!(a.png, b.png);
}

/// Selawik Bold (the text boxes' bold runs) with its `OS/2` `fsType` set to `fs`.
fn selawik_with_fs_type(fs: u16) -> Vec<u8> {
    let mut b = crate::fonts::SELAWIK_BOLD.to_vec();
    let n = u16::from_be_bytes([b[4], b[5]]) as usize;
    for i in 0..n {
        let r = 12 + i * 16;
        if &b[r..r + 4] == b"OS/2" {
            let off = u32::from_be_bytes([b[r + 8], b[r + 9], b[r + 10], b[r + 11]]) as usize;
            b[off + 8..off + 10].copy_from_slice(&fs.to_be_bytes());
        }
    }
    b
}

#[test]
fn a_face_whose_licence_forbids_embedding_is_drawn_as_outlines_and_stays_searchable() {
    let dir = std::env::temp_dir().join(format!("bicraft-pdf-fstype-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("restricted.ttf"), selawik_with_fs_type(0x0002)).unwrap();
    let fonts = FontBook::with(vec![dir.clone()], false);
    let s = report(
        json!([{"name": "p1", "display_name": "P", "width": 640, "height": 360,
        "visuals": [textbox("t", "Restricted face")]}]),
    );
    let doc = render_pdf(&s, &[0], &HashMap::<String, Vec<u8>>::new(), &fonts, &opts(), "T").unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let info = pdf_read::inspect(&doc.bytes).unwrap();
    // the restricted face is listed as not embedded; the text is invisible Helvetica over the
    // outlines, so a search still finds it
    assert!(
        doc.fonts
            .iter()
            .any(|f| !f.embedded && f.name.contains("Selawik")),
        "{:?}",
        doc.fonts
    );
    assert!(
        info.fonts
            .iter()
            .any(|f| f.base_font == "Helvetica" && !f.embedded),
        "{:?}",
        info.fonts
    );
    assert!(
        info.pages[0].text.contains("Restricted"),
        "{:?}",
        info.pages[0].text
    );
}

#[test]
fn the_outline_has_an_entry_per_page_that_goes_to_it() {
    let doc = pdf(&sample(), &[1, 0]);
    let info = pdf_read::inspect(&doc.bytes).unwrap();
    let got: Vec<(usize, &str, Option<usize>)> = info
        .outline
        .iter()
        .map(|o| (o.depth, o.title.as_str(), o.page))
        .collect();
    assert_eq!(got, [(0, "Details", Some(0)), (0, "Overview", Some(1))]);
    // shown beside the pages when there are several, not for one page
    assert!(String::from_utf8_lossy(&doc.bytes).contains("/PageMode /UseOutlines"));
    let one = pdf(&sample(), &[0]);
    assert!(!String::from_utf8_lossy(&one.bytes).contains("/PageMode"));
    assert_eq!(pdf_read::inspect(&one.bytes).unwrap().outline.len(), 1);
    // a page without a display name is listed by its name
    let s = report(json!([{"name": "p9", "display_name": "", "width": 640, "height": 360, "visuals": []}]));
    let info = pdf_read::inspect(&pdf(&s, &[0]).bytes).unwrap();
    assert_eq!(info.outline[0].title, "p9");
}

#[test]
fn the_document_is_tagged_with_a_section_per_page_and_visual() {
    let mut s = sample();
    s["report"]["pages"][0]["visuals"][1]["alt_text"] = json!("Column chart of total sales by region");
    let doc = pdf(&s, &[0, 1]);
    let info = pdf_read::inspect(&doc.bytes).unwrap();
    assert!(info.marked);
    assert_eq!(info.lang.as_deref(), Some("en-US"));
    assert!(info.structure_issues.is_empty(), "{:?}", info.structure_issues);
    let tree: Vec<(usize, &str, Option<&str>)> = info
        .structure
        .iter()
        .filter(|n| n.role != "P")
        .map(|n| (n.depth, n.role.as_str(), n.title.as_deref()))
        .collect();
    assert_eq!(
        tree,
        [
            (0, "Document", None),
            (1, "Sect", Some("Overview")),
            (2, "Sect", Some("textbox")),
            (2, "Sect", Some("Total by Region")),
            (3, "Figure", None),
            (1, "Sect", Some("Details")),
            (2, "Sect", Some("textbox")),
        ]
    );
    // the chart (it has alt text) is a figure, its marks and labels inside it
    let fig = info.structure.iter().find(|n| n.role == "Figure").unwrap();
    assert_eq!(fig.alt.as_deref(), Some("Column chart of total sales by region"));
    assert_eq!(fig.page, Some(0));
    assert!(fig.text.contains("North"), "{:?}", fig.text);
    // a text box's words are one paragraph per line
    let paras: Vec<&str> = info
        .structure
        .iter()
        .filter(|n| n.role == "P")
        .map(|n| n.text.as_str())
        .collect();
    assert_eq!(paras, ["Sales overview Δ 2025", "Details page"]);
    // without alt text, a chart's labels are paragraphs of their own and its marks artifacts
    let plain = pdf_read::inspect(&pdf(&sample(), &[0]).bytes).unwrap();
    assert!(plain.structure.iter().all(|n| n.role != "Figure"));
    let labels: Vec<&str> = plain
        .structure
        .iter()
        .filter(|n| n.role == "P")
        .map(|n| n.text.as_str())
        .collect();
    for want in ["Total by Region", "North", "South", "East"] {
        assert!(labels.contains(&want), "`{want}` not in {labels:?}");
    }
    // every piece of text is tagged or an artifact, and marked content nests
    for p in &info.pages {
        assert!(p.untagged_text.is_empty(), "{:?}", p.untagged_text);
        assert!(!p.unbalanced_marks);
    }
    // the document's language follows the formatting culture
    let fonts = FontBook::bundled_only();
    let fr = RenderOptions {
        culture: Some("fr-FR".into()),
        ..opts()
    };
    let d = render_pdf(&s, &[0], &HashMap::<String, Vec<u8>>::new(), &fonts, &fr, "T").unwrap();
    assert_eq!(
        pdf_read::inspect(&d.bytes).unwrap().lang.as_deref(),
        Some("fr-FR")
    );
}

#[test]
fn text_inside_a_transparency_group_is_tagged_through_the_group() {
    // a preview chart (no data) is drawn faded: a form XObject with marked content of its own
    let mut c = chart("c2", "Preview chart");
    c["data"] = json!({"status": "not_evaluated"});
    let s = report(json!([{"name": "p1", "display_name": "P", "width": 640, "height": 360, "visuals": [c]}]));
    let doc = pdf(&s, &[0]);
    let b = String::from_utf8_lossy(&doc.bytes);
    assert!(b.contains("/S /Transparency"));
    assert!(b.contains("/Stm "), "no marked content reference into a form");
    let info = pdf_read::inspect(&doc.bytes).unwrap();
    assert!(info.structure_issues.is_empty(), "{:?}", info.structure_issues);
    assert!(
        info.structure
            .iter()
            .any(|n| n.role == "P" && n.text.contains("Preview chart")),
        "{:?}",
        info.structure
    );
    assert!(info.pages[0].untagged_text.is_empty());
}

#[test]
fn calibri_text_embeds_the_carlito_subset() {
    let s = report(json!([
        {"name": "p1", "display_name": "Overview", "width": 1280, "height": 720,
         "visuals": [{"name": "t1", "type": "textbox", "x": 20, "y": 10, "width": 500, "height": 40,
                      "text": {"paragraphs": [{"runs": [{"text": "Calibri text", "font_size": 18,
                                                          "font_family": "Calibri"}]}]}}]}
    ]));
    let doc = pdf(&s, &[0]);
    let info = pdf_read::inspect(&doc.bytes).unwrap();
    assert!(
        info.pages[0].text.contains("Calibri text"),
        "{:?}",
        info.pages[0].text
    );
    // the renamed subset (Carlito's Reserved Font Name is not used), not Selawik
    assert!(
        info.fonts
            .iter()
            .any(|f| f.base_font.contains("BIcraftMetricSans")),
        "{:?}",
        info.fonts
    );
    assert!(
        !info.fonts.iter().any(|f| f.base_font.contains("Selawik")),
        "{:?}",
        info.fonts
    );
    assert!(
        !info.fonts.iter().any(|f| f.base_font.contains("Carlito")),
        "{:?}",
        info.fonts
    );
}
