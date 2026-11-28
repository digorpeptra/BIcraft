//! The export flows (M11.21) through the accessibility tree (egui_kittest, no GPU): Backstage ›
//! Export to PDF with its page choice, a visual's Export data dialog from its header's More
//! options and its right-click menu, the report's Export data setting, the browser's download.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use crate::App;
use crate::backstage::{Page, Task};
use crate::ribbon::VisualSel;
use crate::shell::{Caps, Shell};
use crate::source::{Driver, Link, Loaded, Request, Running, Update};

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("bicraft-ui-exports-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&d);
    d.join(name)
}

/// A model with data and a report of two pages (`Overview` with a chart `c1`, `Second`).
fn session() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Region", "Amount"}, {{"North", 100}, {"South", 50}})"#;
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Region", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap();
    }
    let first = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute("page.rename", &json!({"page": first, "display_name": "Overview"}))
        .unwrap();
    s.execute("page.add", &json!({"display_name": "Second"})).unwrap();
    s.execute(
        "visual.add",
        &json!({"page": "Overview", "visual_type": "clusteredColumnChart", "x": 20, "y": 20,
                "width": 400, "height": 250,
                "fields": [{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]}),
    )
    .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn app_with(shell: Shell) -> Harness<'static, App> {
    let mut app = App::new("en-US", shell);
    app.open_path(Path::new("r.pbip"));
    let mut h = harness(app, (1280.0, 860.0));
    steps(&mut h, 6);
    h
}

fn sent(h: &Harness<'_, App>) -> Vec<&'static str> {
    h.state().sent.iter().map(|c| c.id).collect()
}

fn last_params(h: &Harness<'_, App>, id: &str) -> Value {
    h.state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == id)
        .map(|c| c.params.clone())
        .unwrap_or(Value::Null)
}

/// The visual name of the chart (visual.add may have kept or replaced `c1`).
fn chart(h: &Harness<'_, App>) -> String {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/pages/0/visuals/0/name"))
        .and_then(Value::as_str)
        .unwrap_or("c1")
        .to_string()
}

#[test]
fn export_to_pdf_picks_pages_and_writes_the_file() {
    let pdf = tmp("report.pdf");
    let _ = std::fs::remove_file(&pdf);
    let mut shell = engine_shell(Caps::DESKTOP, session);
    let p2 = pdf.clone();
    shell.pick_save = Some(Box::new(move |_, _, _, _| Some(p2.clone())));
    let mut h = app_with(shell);
    let mut bs = crate::backstage::Backstage::new(Page::Export);
    bs.task = Task::Pdf;
    h.state_mut().backstage = Some(bs);
    steps(&mut h, 3);
    for label in ["All pages", "Pages:"] {
        h.get_by_label(label);
    }
    // the task row and its heading
    assert!(h.get_all_by_label("Export to PDF").count() >= 2);
    h.get_all_by_label("Export data setting").next().unwrap();
    h.get_by_label_contains("Current page only:");
    // all pages
    h.get_by_label("Choose where…").click();
    steps(&mut h, 4);
    assert_eq!(sent(&h).last(), Some(&"report.export_pdf"));
    assert_eq!(last_params(&h, "report.export_pdf")["pages"], "all");
    let info = bicraft_render::pdf_read::inspect(&std::fs::read(&pdf).unwrap()).unwrap();
    assert_eq!(info.pages.len(), 2);
    h.get_by_label_contains("Written:");
    // the current page only: its name
    h.get_by_label_contains("Current page only:").click();
    steps(&mut h, 2);
    h.get_by_label("Choose where…").click();
    steps(&mut h, 4);
    let p = last_params(&h, "report.export_pdf");
    assert!(
        p["pages"].is_string() && p["pages"] != "all" && p["pages"] != "current",
        "{p}"
    );
    let info = bicraft_render::pdf_read::inspect(&std::fs::read(&pdf).unwrap()).unwrap();
    assert_eq!(info.pages.len(), 1);
    assert!(info.pages[0].text.contains("North"), "{:?}", info.pages[0].text);
    // a range: page 2
    if let Some(b) = h.state_mut().backstage.as_mut() {
        b.pdf_pages = "range";
        b.pdf_range = "2".into();
    }
    steps(&mut h, 2);
    h.get_by_label("Choose where…").click();
    steps(&mut h, 4);
    assert_eq!(last_params(&h, "report.export_pdf")["pages"], "2-2");
    let info = bicraft_render::pdf_read::inspect(&std::fs::read(&pdf).unwrap()).unwrap();
    assert_eq!(info.pages.len(), 1);
    assert_eq!(info.conformance.pdf_a, None);
    // the archival box asks for PDF/A-2b
    h.get_by_label_contains("Archival PDF (PDF/A-2b)").click();
    steps(&mut h, 2);
    h.get_by_label("Choose where…").click();
    steps(&mut h, 4);
    assert_eq!(last_params(&h, "report.export_pdf")["pdf_a"], true);
    let info = bicraft_render::pdf_read::inspect(&std::fs::read(&pdf).unwrap()).unwrap();
    assert_eq!(info.conformance.pdf_a.as_deref(), Some("2B"));
    // a range that is not one waits for numbers
    if let Some(b) = h.state_mut().backstage.as_mut() {
        b.pdf_range = "abc".into();
    }
    steps(&mut h, 2);
    h.get_by_label("Type page numbers, as 1-3, 5.");
    let _ = std::fs::remove_file(&pdf);
}

#[test]
fn export_data_from_the_header_and_the_setting() {
    let out = tmp("chart.xlsx");
    let _ = std::fs::remove_file(&out);
    let mut shell = engine_shell(Caps::DESKTOP, session);
    let o2 = out.clone();
    shell.pick_save = Some(Box::new(move |_, _, _, _| Some(o2.clone())));
    let mut h = app_with(shell);
    let page = h
        .state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/pages/0/name"))
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let visual = chart(&h);
    h.state_mut().cmd.visual = Some(VisualSel {
        page: page.clone(),
        visual: visual.clone(),
    });
    steps(&mut h, 3);
    // the header's More options opens the visual's menu, with Export data
    h.get_by_label("More options").click();
    steps(&mut h, 2);
    h.get_by_label("Export data").click();
    steps(&mut h, 3);
    assert!(h.state().cmd.export_data.is_some());
    h.get_by_label("Which data do you want to export?");
    // underlying data is greyed: the report allows summarized data only
    assert!(
        h.get_by_label_contains("Underlying data")
            .accesskit_node()
            .is_disabled(),
        "underlying is offered"
    );
    h.get_by_label("Export…").click();
    steps(&mut h, 4);
    assert_eq!(sent(&h).last(), Some(&"visual.export_data"));
    let p = last_params(&h, "visual.export_data");
    assert_eq!(
        (p["kind"].as_str(), p["visual"].as_str()),
        (Some("summarized"), Some(visual.as_str()))
    );
    assert!(out.exists());
    h.get_by_label_contains("Written:");
    h.get_by_label("Cancel").click();
    steps(&mut h, 2);
    assert!(h.state().cmd.export_data.is_none());
    // File > Export > Export data setting: allow underlying data
    let mut bs = crate::backstage::Backstage::new(Page::Export);
    bs.task = Task::Data;
    h.state_mut().backstage = Some(bs);
    steps(&mut h, 3);
    h.get_by_label("Allow summarized and underlying data").click();
    steps(&mut h, 4);
    assert_eq!(sent(&h).last(), Some(&"report.settings"));
    assert_eq!(
        h.state()
            .loaded
            .as_ref()
            .and_then(|l| l.snap.pointer("/report/export_data_mode"))
            .and_then(Value::as_str),
        Some("AllowSummarizedAndUnderlying")
    );
    h.state_mut().backstage = None;
    // the dialog again (the palette's route: invoke without `out`), now with underlying data
    crate::ribbon::invoke(
        h.state_mut(),
        "visual.export_data",
        json!({"page": page, "visual": visual}),
    );
    steps(&mut h, 3);
    let under = h.get_by_label_contains("Underlying data");
    assert!(!under.accesskit_node().is_disabled());
    under.click();
    steps(&mut h, 2);
    h.get_by_label(".csv (up to 30,000 rows)").click();
    steps(&mut h, 2);
    h.get_by_label("Export…").click();
    steps(&mut h, 4);
    let p = last_params(&h, "visual.export_data");
    assert_eq!(p["kind"], "underlying");
    let _ = std::fs::remove_file(&out);
}

/// A driven source that answers every command `Ok` with the `out` it was given, without
/// writing anything (the browser test: the file is the server's).
struct Answering {
    link: Link,
}

impl Driver for Answering {
    fn request(&self, r: Request) {
        if let Request::Command { tag, params, .. } = r {
            let out = params.get("out").cloned().unwrap_or(Value::Null);
            self.link.send(Update::Ran {
                tag,
                result: Ok(json!({"path": out, "rows": 2, "truncated": false})),
            });
        }
    }
}

#[test]
fn the_browser_downloads_what_an_export_wrote() {
    let snap = session().snapshot();
    let downloads: Rc<RefCell<Vec<String>>> = Rc::default();
    let d2 = Rc::clone(&downloads);
    let s2 = snap.clone();
    let mut shell = Shell::new(Caps::BROWSER, move |_o, link| {
        link.send(Update::Snapshot(Loaded {
            snap: std::sync::Arc::new(s2.clone()),
            assets: std::sync::Arc::new(Default::default()),
            seq: 1,
        }));
        Running::Driven(Box::new(Answering { link }))
    });
    shell.download = Some(Box::new(move |rel: &str| d2.borrow_mut().push(rel.to_string())));
    let mut h = app_with(shell);
    // Export to PDF: a workspace-relative name, then the download of that file
    let mut bs = crate::backstage::Backstage::new(Page::Export);
    bs.task = Task::Pdf;
    h.state_mut().backstage = Some(bs);
    steps(&mut h, 3);
    h.get_by_label("Run").click();
    steps(&mut h, 3);
    assert_eq!(sent(&h).last(), Some(&"report.export_pdf"));
    assert_eq!(downloads.borrow().len(), 1);
    assert!(
        downloads.borrow()[0].ends_with(".pdf"),
        "{:?}",
        downloads.borrow()
    );
    assert_eq!(
        last_params(&h, "report.export_pdf")["out"].as_str(),
        Some(downloads.borrow()[0].as_str())
    );
    // a visual's Export data
    h.state_mut().backstage = None;
    let page = snap
        .pointer("/report/pages/0/name")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let visual = snap
        .pointer("/report/pages/0/visuals/0/name")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    h.state_mut().cmd.visual = Some(VisualSel { page, visual });
    crate::export_dialog::open(h.state_mut());
    steps(&mut h, 3);
    h.get_by_label("File name");
    h.get_by_label("Export").click();
    steps(&mut h, 3);
    assert_eq!(sent(&h).last(), Some(&"visual.export_data"));
    assert_eq!(downloads.borrow().len(), 2);
    assert!(
        downloads.borrow()[1].ends_with(".xlsx"),
        "{:?}",
        downloads.borrow()
    );
}

#[test]
fn print_writes_a_pdf_of_the_pages_and_hands_it_to_the_os() {
    // the session's OS opener records what it would open: nothing is launched
    let opened: std::sync::Arc<std::sync::Mutex<Vec<std::path::PathBuf>>> = Default::default();
    let o2 = std::sync::Arc::clone(&opened);
    let shell = engine_shell(Caps::DESKTOP, move || {
        let mut s = session();
        let o3 = std::sync::Arc::clone(&o2);
        s.external_opener = Some(bicraft_engine::ExternalOpener(std::sync::Arc::new(
            move |p: &Path| {
                o3.lock().unwrap().push(p.to_path_buf());
                Ok(())
            },
        )));
        s
    });
    let mut h = app_with(shell);
    // the ribbon's File › Export › Print opens its task in the File view
    crate::ribbon::invoke(h.state_mut(), "report.print", json!({}));
    steps(&mut h, 3);
    assert_eq!(h.state().backstage.as_ref().map(|b| b.task), Some(Task::Print));
    h.get_by_label_contains("opens it in your PDF application");
    h.get_by_label("All pages");
    // the page shown only
    h.get_by_label_contains("Current page only:").click();
    steps(&mut h, 2);
    h.get_by_label("Print…").click();
    steps(&mut h, 6);
    assert_eq!(sent(&h).last(), Some(&"report.print"));
    let p = last_params(&h, "report.print");
    assert!(p.get("out").is_none(), "{p}");
    assert!(p["pages"].is_string() && p["pages"] != "all", "{p}");
    let seen = opened.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "{seen:?}");
    let info = bicraft_render::pdf_read::inspect(&std::fs::read(&seen[0]).unwrap()).unwrap();
    assert_eq!(info.pages.len(), 1);
    assert!(info.marked && !info.outline.is_empty());
    h.get_by_label_contains("Written:");
    h.get_by_label("Opened in your PDF application: print it from there.");
    let _ = std::fs::remove_file(&seen[0]);
}

#[test]
fn the_browser_prints_the_pdf_print_wrote_in_the_workspace() {
    let snap = session().snapshot();
    let printed: Rc<RefCell<Vec<String>>> = Rc::default();
    let p2 = Rc::clone(&printed);
    let mut shell = Shell::new(Caps::BROWSER, move |_o, link| {
        link.send(Update::Snapshot(Loaded {
            snap: std::sync::Arc::new(snap.clone()),
            assets: std::sync::Arc::new(Default::default()),
            seq: 1,
        }));
        Running::Driven(Box::new(Answering { link }))
    });
    shell.download = Some(Box::new(|_: &str| {}));
    shell.print = Some(Box::new(move |rel: &str| p2.borrow_mut().push(rel.to_string())));
    let mut h = app_with(shell);
    let mut bs = crate::backstage::Backstage::new(Page::Export);
    bs.task = Task::Print;
    h.state_mut().backstage = Some(bs);
    steps(&mut h, 3);
    h.get_by_label_contains("your browser's print dialog");
    h.get_by_label("Print…").click();
    steps(&mut h, 3);
    assert_eq!(sent(&h).last(), Some(&"report.print"));
    let out = last_params(&h, "report.print")["out"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(out.ends_with("-print.pdf"), "{out}");
    assert_eq!(printed.borrow().as_slice(), [out]);
}
