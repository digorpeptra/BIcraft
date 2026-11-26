//! `page.set_display_option` (View › Page view): the page's `displayOption` (public page
//! schema: "Defines how the page is scaled"), shown by the snapshot and kept by a PBIP, a PBIX
//! with PBIR parts and a legacy layout.

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

const SYNTHETIC_METADATA: &[u8] =
    include_bytes!("../../../metadata/tests/fixtures/synthetic_metadata.sqlitedb");

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn first_page(s: &Session) -> String {
    s.report.as_ref().unwrap().pages[0].name.clone()
}

fn option(s: &Session) -> Option<String> {
    s.report.as_ref().unwrap().pages[0].display_option.clone()
}

#[test]
fn page_view_sets_the_display_option_and_checks_its_parameters() {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(&mut s, "report.new", json!({}));
    let page = first_page(&s);
    s.report_dirty = false;
    let v = run(
        &mut s,
        "page.set_display_option",
        json!({"page": page, "display_option": "fittowidth"}),
    );
    assert_eq!(v["display_option"], "FitToWidth");
    assert_eq!(v["changed"], true);
    assert!(s.report_dirty);
    assert_eq!(option(&s).as_deref(), Some("FitToWidth"));
    assert_eq!(s.snapshot()["report"]["pages"][0]["display_option"], "FitToWidth");
    // the same value again changes nothing
    s.report_dirty = false;
    let v = run(
        &mut s,
        "page.set_display_option",
        json!({"page": page, "display_option": "FitToWidth"}),
    );
    assert_eq!(v["changed"], false);
    assert!(!s.report_dirty);
    // one undo step
    run(
        &mut s,
        "page.set_display_option",
        json!({"page": page, "display_option": "ActualSize"}),
    );
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(option(&s).as_deref(), Some("FitToWidth"));
    for bad in [
        json!({"page": page, "display_option": "Zoom"}),
        json!({"page": page}),
        json!({"display_option": "FitToPage"}),
        json!({"page": "nope", "display_option": "FitToPage"}),
        json!({"page": page, "display_option": 3}),
        json!({"page": page, "display_option": "FitToPage", "extra": 1}),
    ] {
        assert!(s.execute("page.set_display_option", &bad).is_err(), "{bad}");
    }
    assert_eq!(option(&s).as_deref(), Some("FitToWidth"));
}

#[test]
fn pbip_and_pbix_keep_the_page_view() {
    let dir = TempDir::new("page-view-files");
    for (file, want) in [("Shop.pbip", "ActualSize"), ("Shop.pbix", "FitToWidth")] {
        let mut s = Session::new();
        run(&mut s, "model.new", json!({"name": "Shop"}));
        run(&mut s, "report.new", json!({}));
        let page = first_page(&s);
        run(
            &mut s,
            "page.set_display_option",
            json!({"page": page, "display_option": want}),
        );
        run(&mut s, "file.save_as", json!({"path": dir.join(file)}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": dir.join(file)}));
        assert_eq!(option(&back).as_deref(), Some(want), "{file}");
    }
}

#[test]
fn a_legacy_layout_and_pbir_parts_keep_the_page_view() {
    let dir = TempDir::new("page-view-legacy");
    for pbir in [false, true] {
        let path = dir.join(if pbir { "Pbir.pbix" } else { "Legacy.pbix" });
        let pkg = if pbir {
            bicraft_pbix::synthetic::pbir_pbix(SYNTHETIC_METADATA)
        } else {
            bicraft_pbix::synthetic::pbix(SYNTHETIC_METADATA)
        }
        .unwrap();
        std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
        let mut s = Session::new();
        run(&mut s, "file.open", json!({"path": path}));
        let page = first_page(&s);
        for want in ["FitToWidth", "ActualSize"] {
            run(
                &mut s,
                "page.set_display_option",
                json!({"page": page, "display_option": want}),
            );
            run(&mut s, "file.save", json!({}));
            let mut back = Session::new();
            run(&mut back, "file.open", json!({"path": path}));
            assert_eq!(option(&back).as_deref(), Some(want), "pbir {pbir}");
        }
    }
}
