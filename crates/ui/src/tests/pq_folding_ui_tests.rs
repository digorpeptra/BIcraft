//! The Power Query editor's folding indicators and View native query (egui_kittest, no GPU), on
//! an engine session whose query reads the in-process OData test server of `bicraft-webdata`
//! (never the internet): each applied step shows an icon and a word (`query.folding`), and a
//! step's context menu shows its native query (`query.native`), greyed out with why when the
//! step does not fold.

use std::path::Path;

use bicraft_engine::Session;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, right_click, steps};
use super::harness;
use crate::App;
use crate::registry::Tab as RTab;
use crate::shell::Caps;

fn products(url: &str) -> String {
    format!(
        "let\n    Source = OData.Feed(\"{url}\", null, [Implementation=\"2.0\"]),\n    \
         Products = Source{{[Name=\"Products\",Signature=\"table\"]}}[Data],\n    \
         Kept = Table.SelectColumns(Products, {{\"ID\", \"Name\", \"Price\"}}),\n    \
         Renamed = Table.RenameColumns(Kept, {{{{\"Name\", \"Product\"}}}})\nin\n    Renamed"
    )
}

fn app(lang: &str, url: String) -> Harness<'static, App> {
    let make = move || {
        let mut s = Session::new();
        for (id, p) in [
            ("model.new", json!({"name": "Shop"})),
            (
                "expression.add",
                json!({"name": "Products", "expression": products(&url)}),
            ),
            ("report.new", json!({})),
        ] {
            s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
        }
        s.dirty = false;
        s.report_dirty = false;
        s
    };
    let mut app = App::new(lang, engine_shell(Caps::DESKTOP, make)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1500.0, 950.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().loaded.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    h.state_mut().ribbon_tab = RTab::Home;
    steps(&mut h, 3);
    h
}

fn until(h: &mut Harness<'_, App>, what: &str, done: impl Fn(&App) -> bool) {
    for _ in 0..600 {
        if done(h.state()) {
            steps(h, 2);
            return;
        }
        h.step();
    }
    panic!("never: {what}");
}

fn last_sent(h: &Harness<'_, App>, id: &str) -> Option<Value> {
    h.state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == id)
        .map(|c| c.params.clone())
}

/// The editor open on Products, its folding indicators read.
fn opened(lang: &str, button: &str) -> (bicraft_webdata::fake::FakeHttp, Harness<'static, App>) {
    let srv = bicraft_webdata::fake::start_shop().unwrap();
    let mut h = app(lang, srv.url("/odata/"));
    h.get_by_label(button).click();
    steps(&mut h, 3);
    until(&mut h, "the folding indicators", |a| {
        a.cmd
            .pq
            .as_ref()
            .and_then(|e| e.selected.and_then(|i| e.queries.get(i)))
            .is_some_and(|q| matches!(q.folding, Some(Ok(_))))
    });
    (srv, h)
}

/// A right click on step `name` of the Applied steps pane.
fn step_menu(h: &mut Harness<'_, App>, name: &str) {
    let pos = h
        .get_all_by_label(name)
        .find(|n| n.accesskit_node().role() != Role::TextInput)
        .expect("the step")
        .rect()
        .center();
    right_click(h, pos);
    steps(h, 2);
}

#[test]
fn applied_steps_show_a_folding_indicator_each() {
    let (_srv, h) = opened("en-US", "Transform data");
    let p = last_sent(&h, "query.folding").expect("query.folding ran");
    assert_eq!(p["allow_connectors"], json!(["odata"]), "{p}");
    // the preview reads the source too, with the connector the query calls
    assert_eq!(
        last_sent(&h, "query.preview").unwrap()["allow_connectors"],
        json!(["odata"])
    );
    let e = h.state().cmd.pq.clone().unwrap();
    let q = e.selected.and_then(|i| e.queries.get(i)).unwrap();
    let words: Vec<String> = match &q.folding {
        Some(Ok(f)) => f.iter().map(|x| format!("{} {}", x.name, x.folding)).collect(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        words,
        [
            "Source folded",
            "Products folded",
            "Kept folded",
            "Renamed partial"
        ]
    );
    // a word next to each step, not a colour alone
    assert_eq!(h.get_all_by_label("Folded").count(), 3);
    h.get_by_label("Partially folded");
}

#[test]
fn view_native_query_shows_the_url_of_a_folded_step() {
    let (srv, mut h) = opened("en-US", "Transform data");
    step_menu(&mut h, "Kept");
    h.get_by_label("View native query").click();
    steps(&mut h, 3);
    let p = last_sent(&h, "query.native").expect("query.native ran");
    assert_eq!(p["step"], "Kept");
    assert_eq!(p["allow_connectors"], json!(["odata"]));
    until(&mut h, "the native query", |a| {
        a.cmd
            .pq
            .as_ref()
            .and_then(|e| e.native.as_ref())
            .is_some_and(|n| n.text.is_some())
    });
    let text = h
        .get_by(|n| n.role() == Role::MultilineTextInput)
        .value()
        .unwrap_or_default();
    assert!(text.starts_with(&srv.url("/odata/Products?")), "{text}");
    assert!(text.contains("$select=ID,Name,Price"), "{text}");
    h.get_by_label("Close native query").click();
    steps(&mut h, 3);
    assert!(h.state().cmd.pq.as_ref().unwrap().native.is_none());
}

#[test]
fn view_native_query_is_greyed_out_with_why_when_the_step_does_not_fold() {
    let (_srv, mut h) = opened("en-US", "Transform data");
    step_menu(&mut h, "Renamed");
    assert!(h.get_by_label("View native query").accesskit_node().is_disabled());
    h.get_by_label("This step does not fold: BIcraft runs it, not the source.");
    let before = h.state().sent.iter().filter(|c| c.id == "query.native").count();
    h.get_by_label("View native query").click();
    steps(&mut h, 3);
    assert_eq!(
        h.state().sent.iter().filter(|c| c.id == "query.native").count(),
        before
    );
}

#[test]
fn the_indicators_and_the_menu_speak_french() {
    let (_srv, mut h) = opened("fr-FR", "Transformer les données");
    assert_eq!(h.get_all_by_label("Plié").count(), 3);
    h.get_by_label("Partiellement plié");
    step_menu(&mut h, "Renamed");
    assert!(
        h.get_by_label("Afficher la requête native")
            .accesskit_node()
            .is_disabled()
    );
}

#[test]
fn diagnose_step_shows_bicrafts_time_and_the_source_requests() {
    let (_srv, mut h) = opened("en-US", "Transform data");
    step_menu(&mut h, "Kept");
    h.get_by_label("Diagnose step").click();
    steps(&mut h, 3);
    let p = last_sent(&h, "query.diagnose").expect("query.diagnose ran");
    assert_eq!(p["step"], "Kept");
    assert_eq!(p["allow_connectors"], json!(["odata"]));
    until(&mut h, "the diagnosis", |a| {
        a.cmd
            .pq
            .as_ref()
            .and_then(|e| e.diagnosis.as_ref())
            .is_some_and(|d| d.result.is_some())
    });
    let d = h.state().cmd.pq.clone().unwrap().diagnosis.unwrap();
    let x = d.result.unwrap().unwrap();
    assert_eq!(x.rows, Some(5));
    assert!(
        x.requests
            .iter()
            .any(|r| r.0 == "query" && r.1 == "odata" && r.4.contains("$select=ID,Name,Price")),
        "{:?}",
        x.requests
    );
    h.get_by_label_contains("BIcraft's time:");
    h.get_by_label("Close diagnostics").click();
    steps(&mut h, 3);
    assert!(h.state().cmd.pq.as_ref().unwrap().diagnosis.is_none());
}
