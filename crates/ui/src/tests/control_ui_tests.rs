//! The control channel inside the window (no transport, no GPU): requests sent through the
//! inbox the shells attach run as the buttons do, on an engine session behind the source.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::SyncSender;

use bicraft_engine::Session;
use egui_kittest::Harness;
use serde_json::{Value, json};

use super::harness;
use crate::App;
use crate::app::Tab;
use crate::control::{self, Job, Request as CRequest, Ticket};
use crate::shell::{Caps, Shell};
use crate::source::{Driver, Link, Loaded, Request, Running, Update};

struct Engine {
    session: RefCell<Session>,
    link: Link,
    seq: Cell<u64>,
}

impl Engine {
    fn publish(&self) {
        let snap = self.session.borrow().snapshot();
        self.seq.set(self.seq.get() + 1);
        self.link.send(Update::Snapshot(Loaded {
            snap: Arc::new(snap),
            assets: Arc::new(Default::default()),
            seq: self.seq.get(),
        }));
    }
}

impl Driver for Engine {
    fn request(&self, r: Request) {
        match r {
            Request::Command { tag, id, params } => {
                let result = self.session.borrow_mut().execute(&id, &params);
                if result.is_ok() {
                    self.publish();
                }
                self.link.send(Update::Ran { tag, result });
            }
            Request::Interact(action) => {
                let r = self.session.borrow_mut().execute("view.interact", &action);
                self.publish();
                self.link.send(Update::Interacted(r));
            }
            _ => {}
        }
    }
}

/// A report of two pages ("Overview" with a card, "Detail"), over a one-table model.
fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": "#table({\"Amount\"}, {{1}, {2}})"}}),
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
        ("page.add", json!({"display_name": "Detail"})),
    ] {
        s.execute(id, &p).unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute("page.rename", &json!({"page": page, "display_name": "Overview"}))
        .unwrap();
    s.execute(
        "visual.add",
        &json!({"page": page, "visual_type": "card", "x": 10, "y": 10, "width": 200, "height": 100,
                "fields": [{"role": "Values", "field": "[Total]"}]}),
    )
    .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn shell() -> Shell {
    Shell::new(Caps::DESKTOP, move |_o, link| {
        let d = Engine {
            session: RefCell::new(session()),
            link,
            seq: Cell::new(0),
        };
        d.publish();
        Running::Driven(Box::new(d))
    })
}

struct Driven {
    h: Harness<'static, App>,
    tx: SyncSender<Job>,
}

impl Driven {
    fn new() -> Driven {
        let mut app = App::new("en-US", shell()).with_sync_render();
        let (tx, rx) = std::sync::mpsc::sync_channel(control::MAX_QUEUED);
        app.control.attach(rx);
        let h = harness(app, (1280.0, 800.0));
        Driven { h, tx }
    }

    fn send(&mut self, method: &str, params: Value) -> Ticket {
        let (job, ticket) = Job::new(CRequest {
            id: json!(method),
            method: method.into(),
            params,
        });
        self.tx.try_send(job).unwrap();
        ticket
    }

    /// Sends a request and steps the window until its reply (at most 400 frames).
    fn call(&mut self, method: &str, params: Value) -> Value {
        let t = self.send(method, params);
        for _ in 0..400 {
            self.h.step();
            if let Some(r) = t.try_take() {
                assert_eq!(r["id"], method);
                return r;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        panic!("no reply to {method}");
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let r = self.call(method, params);
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["result"].clone()
    }

    fn fails(&mut self, method: &str, params: Value) -> String {
        let r = self.call(method, params);
        assert_eq!(r["ok"], false, "{method}: {r}");
        r["error"].as_str().unwrap().to_string()
    }
}

#[test]
fn an_agent_opens_a_file_and_drives_the_window() {
    let mut d = Driven::new();
    let s = d.ok("ui.inspect", json!({}));
    assert_eq!(s["shell"], "desktop");
    assert_eq!(s["docs"], json!([]));
    assert_eq!(s["backstage"], "home");
    assert!(d.ok("methods", json!({})).as_array().unwrap().len() >= 10);
    assert!(d.fails("nope", json!({})).contains("unknown method"));

    // app.open waits for the snapshot and the page drawn
    let s = d.ok("app.open", json!({"path": "shop.pbip"}));
    assert_eq!(s["idle"], true, "{s}");
    assert_eq!(s["view"], "report");
    assert_eq!(s["backstage"], Value::Null);
    let pages = s["pages"].as_array().unwrap().clone();
    assert_eq!(pages.len(), 2);
    assert_eq!(s["page"], pages[0]["name"]);
    let detail = pages[1]["name"].as_str().unwrap().to_string();
    let overview = pages[0]["name"].as_str().unwrap().to_string();

    // a page by its display name: the page tab's action, the reply once it is shown
    let s = d.ok("ui.set", json!({"page": "Detail"}));
    assert_eq!(s["page"], detail.as_str());
    assert!(
        d.h.state()
            .last_action
            .as_ref()
            .is_some_and(|a| a["action"] == "page")
    );
    let s = d.ok("ui.set", json!({"page": 0}));
    assert_eq!(s["page"], overview.as_str());

    // the visual selected: the contextual Visual tab appears and can be chosen
    let card = d.h.state().loaded.as_ref().unwrap().snap["report"]["pages"][0]["visuals"][0]["name"]
        .as_str()
        .unwrap()
        .to_string();
    let s = d.ok("ui.set", json!({"visual": card, "ribbonTab": "visual"}));
    assert_eq!(s["visual"]["visual"], card.as_str());
    assert_eq!(s["ribbonTab"], "visual");
    assert!(d.fails("ui.set", json!({"visual": "nope"})).contains("no visual"));
    let s = d.ok(
        "ui.set",
        json!({"visual": null, "view": "model", "ribbonTab": "model"}),
    );
    assert_eq!(s["view"], "model");
    assert_eq!(d.h.state().tab, Tab::Model);
    assert!(
        d.fails("ui.set", json!({"ribbonTab": "measure"}))
            .contains("not shown")
    );
    let s = d.ok(
        "ui.set",
        json!({"model": {"table": "Sales", "measure": "Total"}, "ribbonTab": "measure"}),
    );
    assert_eq!(s["ribbonTab"], "measure");
    let s = d.ok("ui.set", json!({"theme": "dark", "view": "report"}));
    assert_eq!(s["theme"], "dark");
    assert!(d.fails("ui.set", json!({"colour": 1})).contains("unknown key"));
    assert!(
        d.fails("ui.set", json!({"backstage": "nowhere"}))
            .contains("backstage")
    );
    let s = d.ok("ui.set", json!({"ribbonTab": "file"}));
    assert_eq!(s["backstage"], "home");
    let s = d.ok("ui.close", json!({}));
    assert_eq!(s["backstage"], Value::Null);
}

#[test]
fn commands_run_as_their_buttons_do() {
    let mut d = Driven::new();
    d.ok("app.open", json!({"path": "shop.pbip"}));
    // a command with every value: sent to the source, the reply carries its answer
    let r = d.ok(
        "command",
        json!({"command": "measure.add", "params": {"table": "Sales", "name": "Twice", "expression": "2 * [Total]"}}),
    );
    assert_eq!(r["command"], "measure.add");
    assert!(d.h.state().sent.iter().any(|c| c.id == "measure.add"));
    let snap = d.h.state().loaded.as_ref().unwrap().snap.clone();
    assert!(snap.to_string().contains("Twice"));
    // the engine's refusal comes back as the reply's error
    let e = d.fails(
        "command",
        json!({"command": "measure.add", "params": {"table": "Nope", "name": "X", "expression": "1"}}),
    );
    assert!(e.contains("measure.add"), "{e}");
    // values missing: New measure opens the formula bar, as a click does
    let r = d.ok("command", json!({"command": "measure.add", "params": {}}));
    assert_eq!(r["opened"], "formula_bar");
    assert_eq!(r["text"], "Measure = ");
    d.h.state_mut().formula = Default::default();
    // a command whose values are missing opens its form, as a click would
    let r = d.ok("ui.form.open", json!({"command": "measure.add", "prefill": {}}));
    assert_eq!(r["command"], "measure.add", "{r}");
    assert_eq!(d.h.state().form.as_ref().map(|f| f.id), Some("measure.add"));
    // the form: typed values, Run, the answer
    let f = d.ok(
        "ui.form.set",
        json!({"fields": {"table": "Sales", "name": "Thrice", "expression": "3 * [Total]"}}),
    );
    assert_eq!(f["fields"]["name"], "Thrice");
    assert!(
        d.fails("ui.form.set", json!({"fields": {"nope": "x"}}))
            .contains("no field")
    );
    let r = d.ok("ui.form.run", json!({}));
    assert_eq!(r["command"], "measure.add");
    assert!(
        d.h.state()
            .loaded
            .as_ref()
            .unwrap()
            .snap
            .to_string()
            .contains("Thrice")
    );
    // a form with a value missing: the problems, nothing sent
    d.ok(
        "ui.form.open",
        json!({"command": "measure.add", "prefill": {"table": "Sales"}}),
    );
    let n = d.h.state().sent.len();
    assert!(d.fails("ui.form.run", json!({})).contains("not valid"));
    assert_eq!(d.h.state().sent.len(), n);
    d.ok("ui.close", json!({"what": "form"}));
    assert!(d.h.state().form.is_none());
    // file.open never opens a dialog here; unknown commands are refused
    assert!(
        d.fails("command", json!({"command": "file.open"}))
            .contains("never opens a dialog")
    );
    assert!(
        d.fails("command", json!({"command": "no.such"}))
            .contains("unknown command")
    );
    assert!(d.fails("command", json!({})).contains("needs a string"));
    // the palette: opened with a query, a row chosen as Enter does
    let s = d.ok("ui.palette", json!({"query": "add measure"}));
    assert!(
        s["palette"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "measure.add")
    );
    let r = d.ok(
        "ui.palette.choose",
        json!({"command": "measure.add", "form": true}),
    );
    assert_eq!(r["opened"], "form");
    assert!(d.h.state().palette.is_none());
    let rows = d.ok("ui.commands", json!({"query": "measure"}));
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == "measure.add" && r["enabled"] == true)
    );
    // ui.wait: nothing left to do
    assert_eq!(d.ok("ui.wait", json!({}))["idle"], true);
}

#[test]
fn a_screenshot_without_a_gpu_renderer_says_so() {
    // kittest without wgpu draws no frame for egui's screenshot command: the request is answered
    // with an error once its time is up, and the window keeps working (the GPU test of
    // bicraft-desktop takes the real one)
    let mut d = Driven::new();
    let e = d.fails("ui.screenshot", json!({}));
    assert!(e.contains("no frame was captured"), "{e}");
    assert_eq!(d.h.state().control.waiting(), 0);
    d.ok("ui.inspect", json!({}));
}

#[test]
fn requests_a_transport_gave_up_on_never_run() {
    let mut d = Driven::new();
    let t = d.send("app.open", json!({"path": "shop.pbip"}));
    let r = t.give_up(60);
    assert!(r["error"].as_str().unwrap().contains("will not run"));
    for _ in 0..10 {
        d.h.step();
    }
    assert!(!d.h.state().has_doc(), "the abandoned app.open ran");
    // hostile parameters: errors, never a panic
    for (m, p) in [
        ("ui.set", json!({"page": {"x": 1}, "doc": -1})),
        ("ui.set", json!({"doc": 99})),
        ("ui.set", json!({"model": {"relationship": "x"}})),
        ("ui.form.set", json!({"fields": []})),
        ("ui.form.run", json!({})),
        ("ui.palette.choose", json!({"command": 5})),
        ("ui.form.open", json!({"command": "nope"})),
        ("ui.close", json!({"what": "everything"})),
        ("app.open", json!({"path": ""})),
        ("app.open", json!({"path": "a\u{0}b"})),
        ("command", json!({"command": "measure.add", "params": [1]})),
    ] {
        let r = d.call(m, p.clone());
        assert_eq!(r["ok"], false, "{m} {p}: {r}");
    }
    assert!(!Path::new("a").exists());
}

/// A request the browser relay addressed to a document view runs in that file's tab: the window
/// switches to it first, as a click on the tab; a file with no tab open is refused.
#[test]
fn a_request_for_a_document_view_runs_in_its_tab() {
    use crate::source::Origin;
    let mut d = Driven::new();
    for file in ["a.pbix", "b.pbix"] {
        d.h.state_mut().open_origin(Origin::Workspace {
            ws: "ws1".into(),
            file: file.into(),
        });
        for _ in 0..5 {
            d.h.step();
        }
    }
    assert_eq!(d.h.state().doc_tabs().0.len(), 2);
    assert_eq!(d.h.state().active_doc(), 1);
    assert_eq!(d.h.state().workspace_doc("a.pbix"), Some(0));
    let send = |d: &mut Driven, doc: &str| {
        let (job, ticket) = Job::new(CRequest {
            id: json!(doc),
            method: "ui.inspect".into(),
            params: json!({}),
        });
        d.tx.try_send(job.in_doc(Some(doc.into()))).unwrap();
        for _ in 0..50 {
            d.h.step();
            if let Some(r) = ticket.try_take() {
                return r;
            }
        }
        panic!("no reply");
    };
    let r = send(&mut d, "a.pbix");
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(d.h.state().active_doc(), 0, "switched to a.pbix's tab");
    let r = send(&mut d, "b.pbix");
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(d.h.state().active_doc(), 1);
    let r = send(&mut d, "gone.pbix");
    assert_eq!(r["ok"], false);
    assert!(
        r["error"].as_str().unwrap().contains("not open in this page"),
        "{r}"
    );
    assert_eq!(d.h.state().active_doc(), 1, "nothing switched");
}
