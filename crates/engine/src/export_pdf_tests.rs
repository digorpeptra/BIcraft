//! `report.export_pdf`: the pages picked (all, current, a range, hidden ones only when asked),
//! one PDF page each, read back by the renderer's PDF reader with their titles as text.

use serde_json::{Value, json};

use super::*;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("bicraft-export-pdf-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&d);
    d.join(name)
}

/// A model with a measure and a report of three pages (`Overview`, `Hidden` hidden, `Last`),
/// each with a titled chart.
fn session() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Region", "Amount"}, {{"North", 100}, {"South", 50}, {"East", 75}})"#;
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
        ("page.add", json!({"display_name": "Overview"})),
        ("page.add", json!({"display_name": "Hidden"})),
        ("page.add", json!({"display_name": "Last"})),
    ] {
        run(&mut s, id, p);
    }
    for (page, title) in [
        ("Overview", "Sales by region"),
        ("Hidden", "Secret chart"),
        ("Last", "Closing chart"),
    ] {
        let r = run(
            &mut s,
            "visual.add",
            json!({"page": page, "visual_type": "clusteredColumnChart", "x": 20, "y": 20, "width": 500, "height": 300,
                   "fields": [{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]}),
        );
        let v = r["visual"].as_str().unwrap().to_string();
        run(
            &mut s,
            "visual.set_title",
            json!({"page": page, "visual": v, "title": title}),
        );
    }
    if let Some(r) = s.report.as_mut()
        && let Some(p) = r.pages.iter_mut().find(|p| p.display_name == "Hidden")
    {
        p.visibility = Some("HiddenInViewMode".into());
    }
    // a blank first page from report.new: drop it
    let first = s.report.as_ref().unwrap().pages[0].display_name.clone();
    if first != "Overview" {
        run(&mut s, "page.remove", json!({"page": first}));
    }
    s
}

fn texts(path: &std::path::Path) -> Vec<String> {
    let b = std::fs::read(path).unwrap();
    let info = bicraft_render::pdf_read::inspect(&b).unwrap();
    assert!(info.fonts.iter().all(|f| f.embedded), "{:?}", info.fonts);
    info.pages.into_iter().map(|p| p.text).collect()
}

#[test]
fn all_pages_but_hidden_ones_with_their_text() {
    let mut s = session();
    let out = tmp("all.pdf");
    let r = run(
        &mut s,
        "report.export_pdf",
        json!({"out": out.display().to_string(), "system_fonts": false}),
    );
    assert_eq!(r["pages"].as_array().unwrap().len(), 2, "{r}");
    assert_eq!(r["skipped_hidden"], 1);
    assert_eq!(r["pages"][0]["width_pt"], 960.0);
    assert!(
        r["fonts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["embedded"] == true),
        "{r}"
    );
    let t = texts(&out);
    assert_eq!(t.len(), 2);
    assert!(
        t[0].contains("Sales by region") && t[0].contains("North"),
        "{t:?}"
    );
    assert!(t[1].contains("Closing chart"), "{t:?}");
    assert!(!t.iter().any(|x| x.contains("Secret")));
    // with hidden pages
    let r = run(
        &mut s,
        "report.export_pdf",
        json!({"out": out.display().to_string(), "hidden": true, "system_fonts": false}),
    );
    assert_eq!(r["pages"].as_array().unwrap().len(), 3);
    assert!(texts(&out)[1].contains("Secret chart"));
    // the session is not changed: no revision, no undo step
    assert!(!s.report_dirty || s.execute("edit.undo", &json!({})).is_ok());
}

#[test]
fn current_page_ranges_and_lists() {
    let mut s = session();
    let out = tmp("pick.pdf");
    let o = out.display().to_string();
    run(&mut s, "page.set_active", json!({"page": "Last"}));
    let r = run(
        &mut s,
        "report.export_pdf",
        json!({"out": o, "pages": "current", "system_fonts": false}),
    );
    assert_eq!(r["pages"][0]["display_name"], "Last");
    // a 1-based range skips hidden pages unless asked
    let r = run(
        &mut s,
        "report.export_pdf",
        json!({"out": o, "pages": "1-3", "system_fonts": false}),
    );
    let names: Vec<&str> = r["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["display_name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Overview", "Last"]);
    // a page named on its own is exported even when hidden
    let r = run(
        &mut s,
        "report.export_pdf",
        json!({"out": o, "pages": ["Hidden", 0], "system_fonts": false}),
    );
    let names: Vec<&str> = r["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["display_name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Hidden", "Overview"]);
    assert_eq!(texts(&out).len(), 2);
}

#[test]
fn hostile_and_empty_params_are_errors() {
    let mut s = session();
    let o = tmp("bad.pdf").display().to_string();
    for p in [
        json!({}),
        json!({"out": ""}),
        json!({"out": "   "}),
        json!({"out": std::env::temp_dir().display().to_string()}),
        json!({"out": o, "pages": "9-12"}),
        json!({"out": o, "pages": "0-1"}),
        json!({"out": o, "pages": "Nope"}),
        json!({"out": o, "pages": 99}),
        json!({"out": o, "pages": {"a": 1}}),
        json!({"out": o, "pages": [true]}),
        json!({"out": o, "scale": 0}),
        json!({"out": o, "scale": "big"}),
        json!({"out": o, "format_culture": "xx"}),
        json!({"out": o, "font_dirs": "x"}),
        json!({"out": o, "extra": true}),
    ] {
        assert!(s.execute("report.export_pdf", &p).is_err(), "{p}");
    }
    let mut empty = Session::new();
    assert!(empty.execute("report.export_pdf", &json!({"out": o})).is_err());
    assert_eq!(range("2-", 3), Some(vec![1, 2]));
    assert_eq!(range("1,3", 3), Some(vec![0, 2]));
    assert_eq!(range("3-2", 3), None);
}

/// A session whose OS opener records what it would open (nothing is launched).
fn recording(s: &mut Session) -> std::sync::Arc<std::sync::Mutex<Vec<PathBuf>>> {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<PathBuf>::new()));
    let s2 = std::sync::Arc::clone(&seen);
    s.external_opener = Some(crate::desktop_open::ExternalOpener(std::sync::Arc::new(
        move |p: &std::path::Path| {
            s2.lock().unwrap().push(p.to_path_buf());
            Ok(())
        },
    )));
    seen
}

#[test]
fn print_writes_the_pages_to_a_pdf_and_hands_it_to_the_os() {
    let mut s = session();
    let seen = recording(&mut s);
    // the current page, to the print folder, opened
    let r = run(
        &mut s,
        "report.print",
        json!({"pages": "Last", "title": "Shop: Q4"}),
    );
    assert_eq!(r["opened"], true);
    let path = PathBuf::from(r["path"].as_str().unwrap());
    assert!(
        path.starts_with(std::env::temp_dir().join("bicraft-print")),
        "{path:?}"
    );
    assert_eq!(path.file_name().and_then(|n| n.to_str()), Some("Shop_ Q4.pdf"));
    assert_eq!(seen.lock().unwrap().as_slice(), std::slice::from_ref(&path));
    let info = bicraft_render::pdf_read::inspect(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(info.pages.len(), 1);
    assert!(info.pages[0].text.contains("Closing chart"));
    assert_eq!(info.outline.len(), 1);
    // all pages but hidden ones; a dry run opens nothing
    let o = tmp("print.pdf");
    let r = run(
        &mut s,
        "report.print",
        json!({"out": o.display().to_string(), "dry_run": true}),
    );
    assert_eq!(r["opened"], false);
    assert_eq!(seen.lock().unwrap().len(), 1);
    let info = bicraft_render::pdf_read::inspect(&std::fs::read(&o).unwrap()).unwrap();
    let titles: Vec<&str> = info.outline.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(titles, ["Overview", "Last"]);
    // the opener's error is the command's
    s.external_opener = Some(crate::desktop_open::ExternalOpener(std::sync::Arc::new(|_| {
        Err("no PDF application".to_string())
    })));
    let e = s
        .execute("report.print", &json!({"out": o.display().to_string()}))
        .unwrap_err();
    assert!(e.contains("no PDF application"), "{e}");
    // bad parameters: nothing written, nothing opened
    let seen = recording(&mut s);
    for p in [
        json!({"pages": "9-12"}),
        json!({"pages": {"a": 1}}),
        json!({"out": ""}),
        json!({"dry_run": "yes"}),
        json!({"scale": 2}),
        json!({"hidden": {"__wrong__": []}}),
    ] {
        assert!(s.execute("report.print", &p).is_err(), "{p}");
    }
    assert!(seen.lock().unwrap().is_empty());
    assert!(
        Session::new()
            .execute("report.print", &json!({"dry_run": true}))
            .is_err()
    );
    // only a .pdf is handed over
    assert!(
        crate::desktop_open::open_pdf_with_default_app(std::path::Path::new("C:/Windows/notepad.exe"))
            .is_err()
    );
    assert!(crate::desktop_open::open_pdf_with_default_app(&tmp("missing.pdf")).is_err());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&o);
}

#[test]
fn print_on_a_server_writes_the_workspace_file_and_opens_nothing() {
    let dir = tmp("print-server");
    let _ = std::fs::create_dir_all(&dir);
    let mut s = session();
    s.server = Some(crate::ServerConfig::new(&dir).unwrap());
    let seen = recording(&mut s);
    let e = s.execute("report.print", &json!({})).unwrap_err();
    assert!(e.contains("`out`"), "{e}");
    let r = run(&mut s, "report.print", json!({"out": "printed.pdf"}));
    assert_eq!(r["opened"], false);
    assert!(seen.lock().unwrap().is_empty());
    assert!(dir.join("printed.pdf").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The alt text set on a visual is saved in the PBIR and PBIX files and comes back as the PDF's
/// figure for that visual.
#[test]
fn alt_text_round_trips_through_pbir_and_pbix_into_the_pdf_figure() {
    let mut s = session();
    let v = s.report.as_ref().unwrap().pages[0].visuals[0].name.clone();
    run(
        &mut s,
        "visual.format",
        json!({"page": "Overview", "visual": v, "alt_text": "Bars of total sales per region"}),
    );
    for (name, ext) in [("alt-pbip", "pbip"), ("alt-pbix", "pbix")] {
        let dir = tmp(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("Shop.{ext}"));
        run(
            &mut s,
            "file.save_as",
            json!({"path": path.display().to_string()}),
        );
        let mut back = Session::new();
        run(
            &mut back,
            "file.open",
            json!({"path": path.display().to_string()}),
        );
        let o = dir.join("out.pdf");
        run(
            &mut back,
            "report.export_pdf",
            json!({"out": o.display().to_string(), "pages": "Overview"}),
        );
        let info = bicraft_render::pdf_read::inspect(&std::fs::read(&o).unwrap()).unwrap();
        assert!(
            info.marked && info.structure_issues.is_empty(),
            "{ext}: {:?}",
            info.structure_issues
        );
        let fig = info
            .structure
            .iter()
            .find(|n| n.role == "Figure")
            .unwrap_or_else(|| panic!("{ext}: no figure in {:?}", info.structure));
        assert_eq!(
            fig.alt.as_deref(),
            Some("Bars of total sales per region"),
            "{ext}"
        );
        assert!(
            info.structure
                .iter()
                .any(|n| n.role == "Sect" && n.title.as_deref() == Some("Sales by region")),
            "{ext}: {:?}",
            info.structure
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn pdf_ua_by_default_and_pdf_a_on_request() {
    let mut s = session();
    let out = tmp("ua.pdf");
    let r = run(
        &mut s,
        "report.export_pdf",
        json!({"out": out.display().to_string(), "system_fonts": false}),
    );
    assert_eq!(r["conformance"], json!({"pdf_ua": true, "pdf_a": null}));
    let info = bicraft_render::pdf_read::inspect(&std::fs::read(&out).unwrap()).unwrap();
    assert!(info.ua_issues().is_empty(), "{:?}", info.ua_issues());
    let r = run(
        &mut s,
        "report.export_pdf",
        json!({"out": out.display().to_string(), "system_fonts": false, "pdf_a": true, "pdf_ua": false}),
    );
    assert_eq!(r["conformance"], json!({"pdf_ua": false, "pdf_a": "2b"}));
    let info = bicraft_render::pdf_read::inspect(&std::fs::read(&out).unwrap()).unwrap();
    assert!(info.pdfa_issues().is_empty(), "{:?}", info.pdfa_issues());
    assert_eq!(info.conformance.pdf_ua, None);
    let e = s
        .execute(
            "report.export_pdf",
            &json!({"out": out.display().to_string(), "pdf_a": "yes"}),
        )
        .unwrap_err();
    assert!(e.contains("pdf_a"), "{e}");
}
