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
    let job = PdfJob {
        title: "T",
        cap: 40,
        conf: PdfConformance::default(),
    };
    let small = pdf_with_cap(&s, &[0], &assets, &fonts, &opts(), &job).unwrap();
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
    // without alt text, a chart's labels are paragraphs of their own and its marks a figure
    // labelled with its title
    let plain = pdf_read::inspect(&pdf(&sample(), &[0]).bytes).unwrap();
    assert!(
        plain
            .structure
            .iter()
            .filter(|n| n.role == "Figure")
            .all(|n| n.alt.as_deref() == Some("Total by Region") && n.text.is_empty())
    );
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

fn table(name: &str, x: i64, y: i64) -> Value {
    json!({"name": name, "type": "tableEx", "x": x, "y": y, "width": 400, "height": 200, "title": "Sales table",
           "format": {"title": {"show": false}},
           "data": {"status": "ok", "rows": [["North", 120.0], ["South", 80.0]],
                    "totals": [null, 200.0],
                    "columns": [{"name": "Region", "role": "Values", "kind": "column", "data_type": "string"},
                                {"name": "Total", "role": "Values", "kind": "measure", "data_type": "double"}]}})
}

fn pdf_as(s: &Value, conf: PdfConformance) -> PdfDocument {
    let fonts = FontBook::bundled_only();
    render_pdf_as(
        s,
        &[0],
        &HashMap::<String, Vec<u8>>::new(),
        &fonts,
        &opts(),
        "Sales",
        conf,
    )
    .unwrap()
}

#[test]
fn a_table_visual_is_tagged_as_a_table_of_header_and_data_cells() {
    let s = report(
        json!([{"name": "p1", "display_name": "P", "width": 640, "height": 360,
        "visuals": [table("t", 20, 20)]}]),
    );
    let info = pdf_read::inspect(&pdf(&s, &[0]).bytes).unwrap();
    assert!(info.structure_issues.is_empty(), "{:?}", info.structure_issues);
    let under_table: Vec<(usize, &str, &str, Option<&str>)> = info
        .structure
        .iter()
        .skip_while(|n| n.role != "Table")
        .map(|n| (n.depth, n.role.as_str(), n.text.as_str(), n.scope.as_deref()))
        .collect();
    let d = under_table.first().map_or(0, |n| n.0);
    let head: Vec<(usize, &str, &str, Option<&str>)> = under_table
        .iter()
        .take(9)
        .map(|n| (n.0 - d, n.1, n.2, n.3))
        .collect();
    assert_eq!(
        head,
        [
            (0, "Table", "", None),
            (1, "TR", "", None),
            (2, "TH", "Region", Some("Column")),
            (2, "TH", "Total", Some("Column")),
            (1, "TR", "", None),
            (2, "TD", "North", None),
            (2, "TD", "120", None),
            (1, "TR", "", None),
            (2, "TD", "South", None),
        ],
        "{:?}",
        info.structure
    );
    // every row has a cell per column, an empty one included (the total row's first cell
    // when it has no label), so the cells stay under their headers
    let mut cells = Vec::new();
    for n in &under_table {
        match n.1 {
            "TR" => cells.push(0),
            "TH" | "TD" => {
                if let Some(c) = cells.last_mut() {
                    *c += 1;
                }
            }
            _ => {}
        }
    }
    assert!(cells.len() >= 3 && cells.iter().all(|c| *c == 2), "{cells:?}");
    assert!(info.ua_issues().is_empty(), "{:?}", info.ua_issues());
}

#[test]
fn a_chart_without_alt_text_is_a_figure_labelled_with_its_title_and_keeps_its_text() {
    let info = pdf_read::inspect(&pdf(&sample(), &[0]).bytes).unwrap();
    let fig = info.structure.iter().find(|n| n.role == "Figure").unwrap();
    assert_eq!(fig.alt.as_deref(), Some("Total by Region"));
    // its labels stay paragraphs a reader reads (only real alt text stands for them)
    assert!(fig.text.is_empty(), "{:?}", fig.text);
    assert!(info.structure.iter().any(|n| n.role == "P" && n.text == "North"));
    // a text box is no figure
    assert_eq!(info.structure.iter().filter(|n| n.role == "Figure").count(), 1);
}

#[test]
fn the_reading_order_is_the_tab_order_then_top_to_bottom_and_left_to_right() {
    // drawn back to front: c, b, a; read a, b, c (top to bottom)
    let mut a = textbox("a", "First");
    a["y"] = json!(10);
    a["z"] = json!(3);
    let mut b = textbox("b", "Second");
    b["y"] = json!(100);
    b["z"] = json!(2);
    let mut c = textbox("c", "Third");
    c["y"] = json!(200);
    c["z"] = json!(1);
    let read = |v: Vec<Value>| -> Vec<String> {
        let s =
            report(json!([{"name": "p1", "display_name": "P", "width": 640, "height": 360, "visuals": v}]));
        let info = pdf_read::inspect(&pdf(&s, &[0]).bytes).unwrap();
        info.structure
            .iter()
            .filter(|n| n.role == "P")
            .map(|n| n.text.clone())
            .collect()
    };
    assert_eq!(
        read(vec![c.clone(), b.clone(), a.clone()]),
        ["First", "Second", "Third"]
    );
    // the tab order goes first, whatever the position
    c["tab_order"] = json!(0);
    b["tab_order"] = json!(1000);
    assert_eq!(read(vec![c, b, a]), ["Third", "Second", "First"]);
}

#[test]
fn the_default_file_is_identified_as_pdf_ua_and_passes_our_checks() {
    let mut s = sample();
    if let Some(v) = s["report"]["pages"][0]["visuals"].as_array_mut() {
        v.push(table("t", 500, 400));
    }
    let doc = pdf(&s, &[0, 1]);
    let info = pdf_read::inspect(&doc.bytes).unwrap();
    let c = &info.conformance;
    assert_eq!(c.pdf_ua.as_deref(), Some("1"));
    assert_eq!(c.pdf_a, None);
    assert_eq!(c.xmp_title.as_deref(), Some("Sales"));
    assert!(c.display_doc_title && c.file_id && !c.xmp_filtered);
    assert!(info.ua_issues().is_empty(), "{:?}", info.ua_issues());
    // not PDF/A unless asked
    assert!(!info.pdfa_issues().is_empty());
    // PDF/UA off: no identification (the tags stay)
    let off = pdf_as(
        &s,
        PdfConformance {
            ua: false,
            archive: false,
        },
    );
    let off = pdf_read::inspect(&off.bytes).unwrap();
    assert_eq!(off.conformance.pdf_ua, None);
    assert!(off.marked && !off.structure.is_empty());
}

#[test]
fn pdf_a_2b_has_metadata_an_srgb_output_intent_and_an_id() {
    let s = sample();
    let both = PdfConformance {
        ua: true,
        archive: true,
    };
    let doc = pdf_as(&s, both);
    let info = pdf_read::inspect(&doc.bytes).unwrap();
    let c = &info.conformance;
    assert_eq!(c.pdf_a.as_deref(), Some("2B"));
    assert_eq!(c.pdf_ua.as_deref(), Some("1"));
    assert!(c.ua_schema_described);
    assert!(c.producer.is_some() && c.producer == c.xmp_producer);
    assert!(info.pdfa_issues().is_empty(), "{:?}", info.pdfa_issues());
    assert!(info.ua_issues().is_empty(), "{:?}", info.ua_issues());
    let p = c.output_intents.first().and_then(|o| o.profile.as_ref()).unwrap();
    assert_eq!(
        (p.class.as_str(), p.colour_space.as_str(), p.n),
        ("mntr", "RGB ", Some(3))
    );
    // the same input still gives the same bytes
    assert_eq!(doc.bytes, pdf_as(&s, both).bytes);
    // PDF/A alone: no PDF/UA identification, no schema to describe
    let a = pdf_as(
        &s,
        PdfConformance {
            ua: false,
            archive: true,
        },
    );
    let a = pdf_read::inspect(&a.bytes).unwrap();
    assert_eq!(a.conformance.pdf_ua, None);
    assert!(a.pdfa_issues().is_empty(), "{:?}", a.pdfa_issues());
    // a title with XML's characters round-trips through the metadata
    let fonts = FontBook::bundled_only();
    let t = render_pdf_as(
        &s,
        &[0],
        &HashMap::<String, Vec<u8>>::new(),
        &fonts,
        &opts(),
        "Q&A <2025>",
        both,
    )
    .unwrap();
    let ti = pdf_read::inspect(&t.bytes).unwrap();
    assert_eq!(ti.conformance.xmp_title.as_deref(), Some("Q&A <2025>"));
    assert!(ti.pdfa_issues().is_empty(), "{:?}", ti.pdfa_issues());
}

/// A minimal OpenType face with CFF outlines, built here (no font file from elsewhere): family
/// "Cefftest", `.notdef`, space and the ASCII letters, each glyph an empty charstring 600 units
/// wide.
fn cff_face() -> Vec<u8> {
    fn index(items: &[Vec<u8>]) -> Vec<u8> {
        let mut b = (items.len() as u16).to_be_bytes().to_vec();
        if items.is_empty() {
            return b;
        }
        b.push(4);
        let mut off = 1u32;
        b.extend_from_slice(&off.to_be_bytes());
        for it in items {
            off += it.len() as u32;
            b.extend_from_slice(&off.to_be_bytes());
        }
        for it in items {
            b.extend_from_slice(it);
        }
        b
    }
    fn int5(v: usize) -> Vec<u8> {
        let mut b = vec![29];
        b.extend_from_slice(&(v as i32).to_be_bytes());
        b
    }
    fn be16(v: u16) -> Vec<u8> {
        v.to_be_bytes().to_vec()
    }
    fn be32(v: u32) -> Vec<u8> {
        v.to_be_bytes().to_vec()
    }
    let n = 1 + 1 + 26 + 26;
    // CFF: header, Name, Top DICT (CharStrings, Private), String and Global Subr INDEXes,
    // CharStrings (each `endchar`), Private DICT (defaultWidthX 0)
    let header = vec![1u8, 0, 4, 4];
    let name = index(&[b"Cefftest-Regular".to_vec()]);
    let strings = index(&[]);
    let gsubrs = index(&[]);
    let charstrings = index(&vec![vec![14u8]; n]);
    let private = vec![139u8, 20];
    let top_index_len = 2 + 1 + 8 + 17;
    let cs_off = header.len() + name.len() + top_index_len + strings.len() + gsubrs.len();
    let priv_off = cs_off + charstrings.len();
    let mut top = int5(cs_off);
    top.push(17);
    top.extend(int5(private.len()));
    top.extend(int5(priv_off));
    top.push(18);
    let cff = [header, name, index(&[top]), strings, gsubrs, charstrings, private].concat();
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[38..40].copy_from_slice(&(-200i16).to_be_bytes());
    head[40..42].copy_from_slice(&600i16.to_be_bytes());
    head[42..44].copy_from_slice(&800i16.to_be_bytes());
    head[48..50].copy_from_slice(&2i16.to_be_bytes());
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[10..12].copy_from_slice(&600u16.to_be_bytes());
    hhea[18..20].copy_from_slice(&1i16.to_be_bytes());
    hhea[34..36].copy_from_slice(&(n as u16).to_be_bytes());
    let maxp = [be32(0x0000_5000), be16(n as u16)].concat();
    let hmtx: Vec<u8> = (0..n).flat_map(|_| [be16(600), be16(0)].concat()).collect();
    // cmap: one format 12 table (Windows, full Unicode)
    let groups: [(u32, u32, u32); 3] = [(0x20, 0x20, 1), (0x41, 0x5A, 2), (0x61, 0x7A, 28)];
    let mut sub = [be16(12), be16(0), be32(16 + 12 * 3), be32(0), be32(3)].concat();
    for (a, b, g) in groups {
        sub.extend([be32(a), be32(b), be32(g)].concat());
    }
    let cmap = [be16(0), be16(1), be16(3), be16(10), be32(12), sub].concat();
    // name: family, subfamily, PostScript name (Windows, UTF-16BE)
    let names = [(1u16, "Cefftest"), (2, "Regular"), (6, "Cefftest-Regular")];
    let mut storage = Vec::new();
    let mut recs = Vec::new();
    for (id, t) in names {
        let u: Vec<u8> = t.encode_utf16().flat_map(u16::to_be_bytes).collect();
        recs.extend(
            [
                be16(3),
                be16(1),
                be16(0x409),
                be16(id),
                be16(u.len() as u16),
                be16(storage.len() as u16),
            ]
            .concat(),
        );
        storage.extend(u);
    }
    let name_t = [be16(0), be16(3), be16(6 + 12 * 3), recs, storage].concat();
    let mut os2 = vec![0u8; 96];
    os2[0..2].copy_from_slice(&4u16.to_be_bytes());
    os2[4..6].copy_from_slice(&400u16.to_be_bytes());
    os2[6..8].copy_from_slice(&5u16.to_be_bytes());
    os2[62..64].copy_from_slice(&0x40u16.to_be_bytes());
    os2[68..70].copy_from_slice(&800i16.to_be_bytes());
    os2[70..72].copy_from_slice(&(-200i16).to_be_bytes());
    os2[74..76].copy_from_slice(&800u16.to_be_bytes());
    os2[76..78].copy_from_slice(&200u16.to_be_bytes());
    os2[88..90].copy_from_slice(&700i16.to_be_bytes());
    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
    crate::ttf_subset::sfnt(
        u32::from_be_bytes(*b"OTTO"),
        vec![
            (*b"CFF ", cff),
            (*b"OS/2", os2),
            (*b"cmap", cmap),
            (*b"head", head),
            (*b"hhea", hhea),
            (*b"hmtx", hmtx),
            (*b"maxp", maxp),
            (*b"name", name_t),
            (*b"post", post),
        ],
    )
}

#[test]
fn a_cff_face_is_embedded_whole_as_opentype() {
    let dir = std::env::temp_dir().join(format!("bicraft-pdf-cff-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("cefftest.otf"), cff_face()).unwrap();
    let fonts = FontBook::with(vec![dir.clone()], false);
    let mut t = textbox("t", "Cff Text");
    t["text"]["paragraphs"][0]["runs"][0]["font_family"] = json!("Cefftest");
    let s = report(json!([{"name": "p1", "display_name": "P", "width": 640, "height": 360, "visuals": [t]}]));
    let both = PdfConformance {
        ua: true,
        archive: true,
    };
    let doc = render_pdf_as(
        &s,
        &[0],
        &HashMap::<String, Vec<u8>>::new(),
        &fonts,
        &opts(),
        "T",
        both,
    )
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let info = pdf_read::inspect(&doc.bytes).unwrap();
    assert!(
        doc.fonts
            .iter()
            .any(|f| f.embedded && f.name == "Cefftest-Regular"),
        "{:?}",
        doc.fonts
    );
    let f = info
        .fonts
        .iter()
        .find(|f| f.base_font == "Cefftest-Regular")
        .unwrap_or_else(|| panic!("{:?}", info.fonts));
    assert!(f.embedded && f.to_unicode, "{f:?}");
    assert_eq!(f.file.as_deref(), Some("FontFile3/OpenType"));
    assert_eq!(f.cid_subtype.as_deref(), Some("CIDFontType0"));
    // no Helvetica stand-in: the text is the face's own
    assert!(
        info.fonts.iter().all(|f| f.base_font != "Helvetica"),
        "{:?}",
        info.fonts
    );
    assert!(
        info.pages[0].text.contains("Cff Text"),
        "{:?}",
        info.pages[0].text
    );
    assert!(info.pdfa_issues().is_empty(), "{:?}", info.pdfa_issues());
}
