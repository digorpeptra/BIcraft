//! UI speed (docs/ui-perf.md): the window driven as the desktop runs it, the engine session on a
//! worker thread and the pages rendered on the render thread, over a generated report (a fact
//! table of many rows, pages of tables, matrices, charts, cards and slicers). Each scenario
//! measures the window's frames (egui's layout and the tessellation of what it draws; no GPU) and
//! the wall time from an input until its result is on screen:
//!
//! - opening the report: the first frame, the first snapshot, the first page drawn with its data;
//! - frames while nothing happens;
//! - switching pages from the page tabs;
//! - dragging a visual on the canvas (the frames of the drag, then the move drawn);
//! - resizing a visual by its corner handle (the frames of the drag, how many of them draw the
//!   visual's content at another size than its outline, the release until it is drawn resized);
//! - opening the Format visual pane;
//! - typing in the DAX formula bar;
//! - scrolling the Data view's grid over the whole table;
//! - Home › Transform data: the Power Query editor's preview.
//!
//! Budgets are generous (the test profile, a machine shared with builds): they catch a frame
//! that becomes seconds, not a few per cent. The numbers are printed (`--nocapture`); the
//! measured ones are in docs/ui-perf.md. The gate runs [`ROWS`]; the nightly path
//! (`cargo xtask heavy`) runs [`HEAVY_ROWS`].

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use bicraft_engine::Session;
use bicraft_engine::watch::data_pending;
use egui::accesskit::Role;
use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::page_tab;
use crate::App;
use crate::app::Tab;
use crate::registry::Tab as RTab;
use crate::shell::{Caps, Shell};
use crate::source::{self, Link, Loaded, Request, Update};

/// Fact rows of the gate's report.
const ROWS: usize = 100_000;
/// Fact rows of the nightly report.
const HEAVY_ROWS: usize = 1_000_000;
/// Report pages, each with the visuals of [`page_visuals`].
const PAGES: usize = 8;

/// The longest wait for any one result (a hung source fails the test instead of the job).
const WAIT: Duration = Duration::from_secs(180);

/// Budgets: generous bounds (the test profile on a loaded machine), each several times what
/// docs/ui-perf.md measured.
struct Budget {
    /// From the first frame to the first page drawn with its data.
    open: Duration,
    /// Median frame while nothing happens, while dragging, typing and scrolling.
    frame: Duration,
    /// Slowest single frame of those (one frame may lay out a new page or a new grid).
    worst_frame: Duration,
    /// From an input (page tab, drag release, Format visual, Transform data) to its result drawn.
    latency: Duration,
}

const GATE: Budget = Budget {
    open: Duration::from_secs(30),
    frame: Duration::from_millis(250),
    worst_frame: Duration::from_millis(2000),
    latency: Duration::from_secs(10),
};

const HEAVY: Budget = Budget {
    open: Duration::from_secs(120),
    frame: Duration::from_millis(250),
    worst_frame: Duration::from_millis(2000),
    latency: Duration::from_secs(60),
};

/// Sales(Id, Product, Region, Amount, Qty, Margin = Amount * 0.3) of `rows` rows made by M,
/// measures Total, Units, Average; [`PAGES`] pages `P1`… of [`page_visuals`].
fn session(rows: usize) -> Session {
    let mut s = Session::new();
    let sales = format!(
        "let n = List.Numbers(1, {rows}), t = Table.FromColumns({{n, \
         List.Transform(n, each \"Product \" & Text.From(Number.Mod(_ * 7, 500))), \
         List.Transform(n, each \"Region \" & Text.From(Number.Mod(_, 12))), \
         List.Transform(n, each Number.Mod(_ * 37, 1000) / 10), \
         List.Transform(n, each Number.Mod(_, 9) + 1)}}, \
         {{\"Id\", \"Product\", \"Region\", \"Amount\", \"Qty\"}}) in t"
    );
    let mut steps = vec![
        ("model.new", json!({"name": "Speed"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
    ];
    for (name, ty) in [
        ("Id", "int64"),
        ("Product", "string"),
        ("Region", "string"),
        ("Amount", "double"),
        ("Qty", "int64"),
    ] {
        steps.push((
            "column.add",
            json!({"table": "Sales", "name": name, "data_type": ty}),
        ));
    }
    steps.extend([
        (
            "column.add",
            json!({"table": "Sales", "name": "Margin", "data_type": "double", "expression": "Sales[Amount] * 0.3"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0.0"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Units", "expression": "SUM(Sales[Qty])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Average", "expression": "DIVIDE([Total], [Units])"}),
        ),
        ("report.new", json!({})),
    ]);
    for (id, p) in steps {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    let first = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute("page.rename", &json!({"page": first, "display_name": "P1"}))
        .unwrap();
    for i in 2..=PAGES {
        s.execute("page.add", &json!({"display_name": format!("P{i}")}))
            .unwrap();
    }
    let pages: Vec<String> = s
        .report
        .as_ref()
        .unwrap()
        .pages
        .iter()
        .map(|p| p.name.clone())
        .collect();
    for page in &pages {
        for (ty, r, fields) in page_visuals() {
            s.execute(
                "visual.add",
                &json!({"page": page, "visual_type": ty, "x": r[0], "y": r[1], "width": r[2],
                        "height": r[3], "fields": fields}),
            )
            .unwrap_or_else(|e| panic!("{ty}: {e}"));
        }
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

/// The visuals of every page: (type, [x, y, width, height], fields), filling a 1280 × 720 page.
fn page_visuals() -> Vec<(&'static str, [i64; 4], Value)> {
    let by = |cat: &str, val: &str| json!([{"role": "Category", "field": cat}, {"role": "Y", "field": val}]);
    vec![
        (
            "card",
            [10, 10, 200, 100],
            json!([{"role": "Values", "field": "[Total]"}]),
        ),
        (
            "card",
            [220, 10, 200, 100],
            json!([{"role": "Values", "field": "[Units]"}]),
        ),
        (
            "card",
            [430, 10, 200, 100],
            json!([{"role": "Values", "field": "[Average]"}]),
        ),
        (
            "slicer",
            [640, 10, 200, 220],
            json!([{"role": "Values", "field": "Sales[Region]"}]),
        ),
        (
            "tableEx",
            [850, 10, 420, 340],
            json!([{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "[Total]"},
                   {"role": "Values", "field": "[Units]"}]),
        ),
        (
            "pivotTable",
            [10, 120, 620, 230],
            json!([{"role": "Rows", "field": "Sales[Region]"}, {"role": "Values", "field": "[Total]"},
                   {"role": "Values", "field": "[Average]"}]),
        ),
        (
            "clusteredColumnChart",
            [10, 360, 410, 350],
            by("Sales[Region]", "[Total]"),
        ),
        ("lineChart", [430, 360, 410, 350], by("Sales[Product]", "[Units]")),
        ("pieChart", [850, 360, 200, 350], by("Sales[Region]", "[Units]")),
        (
            "clusteredBarChart",
            [1060, 360, 210, 350],
            by("Sales[Region]", "[Average]"),
        ),
    ]
}

/// The session's snapshot and images, as the desktop's local source publishes them
/// (`apps/bicraft-desktop/src/local.rs`); whether visuals still wait for their data.
fn publish(session: &Session, seq: &mut u64, link: &Link) -> bool {
    let pending = send(session, session.snapshot(), seq, link);
    session.background_work();
    pending
}

/// The report just opened, as `local.rs` publishes it: its pages without data, then with.
fn publish_opened(session: &Session, seq: &mut u64, link: &Link) -> bool {
    let layout = bicraft_engine::SnapshotLimits {
        data_budget_ms: 0,
        ..Default::default()
    };
    send(session, session.snapshot_with(&layout), seq, link) && publish(session, seq, link)
}

fn send(session: &Session, snap: Value, seq: &mut u64, link: &Link) -> bool {
    let assets = session
        .snapshot_assets()
        .into_iter()
        .map(|a| (a.id, a.bytes))
        .collect();
    let pending = data_pending(&snap);
    *seq += 1;
    link.send(Update::Snapshot(Loaded {
        snap: Arc::new(snap),
        assets: Arc::new(assets),
        seq: *seq,
    }));
    pending
}

/// The desktop's local source over an in-memory session: commands and clicks run on the worker
/// thread, the snapshot is published again after each change and while visuals wait for data.
fn serve(mut session: Session, link: &Link) {
    let mut seq = 0;
    let mut left = publish_opened(&session, &mut seq, link);
    loop {
        match link.requests.recv_timeout(Duration::from_millis(50)) {
            Ok(Request::Interact(action)) => {
                let r = session.execute("view.interact", &action);
                if r.is_ok() {
                    left = publish(&session, &mut seq, link);
                }
                link.send(Update::Interacted(r));
            }
            Ok(Request::Command { tag, id, params }) => {
                let before = session.revision;
                let result = session.execute(&id, &params);
                if session.revision != before {
                    left = publish(&session, &mut seq, link);
                }
                link.send(Update::Ran { tag, result });
            }
            Ok(Request::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {
                if left {
                    left = publish(&session, &mut seq, link);
                }
            }
        }
    }
}

/// A desktop shell whose source is [`serve`] over [`session`]`(rows)` on a worker thread.
fn threaded_shell(rows: usize) -> Shell {
    Shell::new(Caps::DESKTOP, move |_origin, link| {
        source::spawn(link, move |link| serve(session(rows), link))
    })
}

/// Frame times of one scenario.
#[derive(Default)]
struct Frames(Vec<Duration>);

impl Frames {
    fn sorted(&self) -> Vec<Duration> {
        let mut v = self.0.clone();
        v.sort();
        v
    }
    fn median(&self) -> Duration {
        let v = self.sorted();
        v.get(v.len() / 2).copied().unwrap_or_default()
    }
    fn p95(&self) -> Duration {
        let v = self.sorted();
        v.get((v.len() * 95).div_ceil(100).saturating_sub(1))
            .copied()
            .unwrap_or_default()
    }
    fn max(&self) -> Duration {
        self.0.iter().max().copied().unwrap_or_default()
    }
}

/// What a run measured, printed as one table.
#[derive(Default)]
struct Report {
    lines: Vec<String>,
}

impl Report {
    fn frames(&mut self, what: &str, f: &Frames, b: &Budget) {
        let line = format!(
            "{what:<34} {:>4} frames  median {:>7.2} ms  p95 {:>7.2} ms  max {:>7.2} ms",
            f.0.len(),
            ms(f.median()),
            ms(f.p95()),
            ms(f.max())
        );
        println!("{line}");
        self.lines.push(line.clone());
        assert!(!f.0.is_empty(), "{what}: no frame measured");
        assert!(f.median() < b.frame, "{line}: median over {:?}", b.frame);
        assert!(
            f.max() < b.worst_frame,
            "{line}: a frame over {:?}",
            b.worst_frame
        );
    }
    fn latency(&mut self, what: &str, d: Duration, budget: Duration) {
        let line = format!("{what:<34} {:>10.1} ms", ms(d));
        println!("{line}");
        self.lines.push(line.clone());
        assert!(d < budget, "{line}: over {budget:?}");
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// One frame as the window draws it, without the GPU: egui's pass, then the tessellation of its
/// shapes into meshes.
fn frame(h: &mut Harness<'_, App>) -> Duration {
    let t = Instant::now();
    h.step();
    let out = h.output();
    let prims = h.ctx.tessellate(out.shapes.clone(), out.pixels_per_point);
    std::hint::black_box(prims);
    t.elapsed()
}

/// Frames until `done` holds, with a short pause between them (the worker and render threads
/// answer in their own time, as in the window); the wall time it took, and the frames drawn.
fn until(h: &mut Harness<'_, App>, what: &str, done: impl Fn(&App) -> bool) -> (Duration, Frames) {
    let t = Instant::now();
    let mut frames = Frames::default();
    while !done(h.state()) {
        assert!(
            t.elapsed() < WAIT,
            "never: {what} (error {:?}, status {:?})",
            h.state().error,
            h.state().status
        );
        frames.0.push(frame(h));
        std::thread::sleep(Duration::from_millis(1));
    }
    (t.elapsed(), frames)
}

/// `n` frames without input.
fn idle(h: &mut Harness<'_, App>, n: usize) -> Frames {
    Frames((0..n).map(|_| frame(h)).collect())
}

/// Whether the page on screen is page `page` of the latest snapshot, drawn with all its data.
fn drawn(a: &App, page: usize) -> bool {
    let (Some(l), Some((key, Ok(_)))) = (&a.loaded, &a.shown) else {
        return false;
    };
    key.page == page && key.seq == l.seq && !data_pending(&l.snap) && a.active_page() == page
}

fn press(h: &mut Harness<'_, App>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    });
}

/// A visual of type `ty` on the page shown: its name and its centre on screen.
fn visual_at(a: &App, ty: &str) -> (String, f64, Pos2) {
    let page = a.active_page();
    let snap = &a.loaded.as_ref().unwrap().snap;
    let v = snap["report"]["pages"][page]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["type"] == ty)
        .unwrap();
    let n = |k: &str| v[k].as_f64().unwrap();
    let pl = a.placed.unwrap();
    let centre = pl.rect.min
        + vec2(
            (n("x") + n("width") / 2.0) as f32,
            (n("y") + n("height") / 2.0) as f32,
        ) * pl.zoom;
    (v["name"].as_str().unwrap().to_string(), n("x"), centre)
}

fn visual_x(a: &App, name: &str) -> Option<f64> {
    let page = a.active_page();
    a.loaded.as_ref()?.snap["report"]["pages"][page]["visuals"]
        .as_array()?
        .iter()
        .find(|v| v["name"] == name)?["x"]
        .as_f64()
}

/// The rectangle (report pixels) of visual `name` in the latest snapshot.
fn visual_rect(a: &App, name: &str) -> Option<egui::Rect> {
    let page = a.active_page();
    let v = a.loaded.as_ref()?.snap["report"]["pages"][page]["visuals"]
        .as_array()?
        .iter()
        .find(|v| v["name"] == name)?
        .clone();
    let n = |k: &str| v[k].as_f64().map(|x| x as f32);
    Some(egui::Rect::from_min_size(
        egui::pos2(n("x")?, n("y")?),
        vec2(n("width")?, n("height")?),
    ))
}

/// Where the content of visual `name` is drawn on the page now (report pixels): scaled from the
/// page image into its outline while it is resized (`canvas::previews`); else the page image
/// shown, when it was rendered from the latest snapshot, holds it at the snapshot's rectangle;
/// an older image holds it at a size the test no longer knows (`None`, stale).
fn content_rect(a: &App, name: &str) -> Option<egui::Rect> {
    if let Some((_, _, to)) = crate::canvas::previews(a)
        .unwrap_or_default()
        .into_iter()
        .find(|(n, _, _)| n == name)
    {
        return Some(to);
    }
    let (key, Ok(_)) = a.shown.as_ref()? else {
        return None;
    };
    if a.loaded.as_ref().map(|l| l.seq) != Some(key.seq) {
        return None;
    }
    visual_rect(a, name)
}

/// `r` on whole report pixels, as the canvas sends a move (`canvas.rs`, `round`).
fn whole(r: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(r.min.x.round(), r.min.y.round()),
        vec2(r.width().round().max(1.0), r.height().round().max(1.0)),
    )
}

/// Whether `a` and `b` are the same rectangle within half a report pixel.
fn same_rect(a: Option<egui::Rect>, b: egui::Rect) -> bool {
    a.is_some_and(|a| (a.min - b.min).length() < 0.75 && (a.max - b.max).length() < 0.75)
}

/// Resizes the column chart by its bottom-right handle: the frames of the drag, the frames whose
/// content is not drawn at the frame's size (the selection's outline and the content apart), and
/// the release until the visual is drawn at its new size.
fn resize(h: &mut Harness<'_, App>, r: &mut Report, b: &Budget) {
    let (chart, _, centre) = visual_at(h.state(), "clusteredColumnChart");
    h.hover_at(centre);
    frame(h);
    press(h, centre, true);
    press(h, centre, false);
    until(h, "the chart selected", |a| {
        a.cmd.visual.as_ref().is_some_and(|v| v.visual == chart)
    });
    idle(h, 3);
    let pl = h.state().placed.unwrap();
    let r0 = visual_rect(h.state(), &chart).unwrap();
    let corner = pl.rect.min + r0.max.to_vec2() * pl.zoom;
    h.hover_at(corner);
    frame(h);
    press(h, corner, true);
    frame(h);
    let mut frames = Frames::default();
    let mut stale = 0;
    let steps = 60;
    for k in 1..=steps {
        h.event(Event::PointerMoved(corner + vec2(2.0 * k as f32, k as f32)));
        frames.0.push(frame(h));
        let a = h.state();
        let ghost = a
            .canvas
            .gesture
            .as_ref()
            .and_then(|g| g.now.iter().find(|(n, _)| *n == chart).map(|(_, r)| *r));
        if let Some(g) = ghost
            && !same_rect(content_rect(a, &chart), g)
        {
            stale += 1;
        }
    }
    let to = corner + vec2(2.0 * steps as f32, steps as f32);
    let end = h
        .state()
        .canvas
        .gesture
        .as_ref()
        .and_then(|g| g.now.iter().find(|(n, _)| *n == chart).map(|(_, r)| *r))
        .unwrap();
    let t = Instant::now();
    press(h, to, false);
    let mut after = 0;
    loop {
        frame(h);
        let a = h.state();
        let resized = visual_rect(a, &chart).is_some_and(|r| r.width() > r0.width() + 1.0);
        let drawn_now = drawn(a, 0) && same_rect(content_rect(a, &chart), whole(end));
        if resized && drawn_now {
            break;
        }
        if !same_rect(content_rect(a, &chart), whole(end)) {
            after += 1;
        }
        assert!(t.elapsed() < WAIT, "never: the resized visual drawn");
        std::thread::sleep(Duration::from_millis(1));
    }
    let released = t.elapsed();
    r.frames("resize a visual: frames of the drag", &frames, b);
    r.latency("resize a visual: release to drawn", released, b.latency);
    let line = format!(
        "{:<34} {stale:>4} of {steps} frames, {after} after the release",
        "resize a visual: stale content"
    );
    println!("{line}");
    r.lines.push(line.clone());
    // the content follows the handle: drawn at the outline's size on every frame of the drag and
    // until the page rendered at the new size is on screen
    assert_eq!((stale, after), (0, 0), "{line}");
    idle(h, 3);
}

/// Opens the report of `rows` rows and runs every scenario against `b`.
fn run(rows: usize, b: &Budget) -> Report {
    let mut r = Report::default();
    println!(
        "UI speed, {rows} rows, {PAGES} pages x {} visuals",
        page_visuals().len()
    );
    let t0 = Instant::now();
    let mut app = App::new("en-US", threaded_shell(rows));
    app.tab = Tab::Report;
    app.open_path(Path::new("speed.pbip"));
    // 60 frames a second, as a window draws while something moves
    let mut h = Harness::builder()
        .with_size(vec2(1600.0, 960.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            |ui, app: &mut App| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        );

    // --- opening
    let first = frame(&mut h);
    r.latency("open: first frame", first, b.latency);
    let (snap, _) = until(&mut h, "the first snapshot", |a| a.loaded.is_some());
    r.latency("open: first snapshot", first + snap, b.open);
    let (page, _) = until(&mut h, "the first page with its data", |a| drawn(a, 0));
    let open = t0.elapsed();
    r.latency("open: first page drawn with data", first + snap + page, b.open);
    assert!(open < b.open + Duration::from_secs(5), "opened in {open:?}");

    // --- idle
    idle(&mut h, 5);
    let still = idle(&mut h, 120);
    r.frames("idle, Report view", &still, b);
    // a settled window asks for no more frames: it draws again only on input or new data
    assert!(!h.ctx.has_requested_repaint(), "an idle window keeps repainting");

    // --- page tabs
    let mut switches = Vec::new();
    let mut switch_frames = Frames::default();
    for (label, index) in [("P2", 1), ("P5", 4), ("P1", 0)] {
        let tab = page_tab(&h, label).center();
        h.hover_at(tab);
        frame(&mut h);
        let t = Instant::now();
        press(&mut h, tab, true);
        press(&mut h, tab, false);
        let (_, f) = until(&mut h, label, |a| drawn(a, index));
        switches.push(t.elapsed());
        switch_frames.0.extend(f.0);
        idle(&mut h, 3);
    }
    switches.sort();
    r.latency(
        "page tab: median click to drawn",
        switches[switches.len() / 2],
        b.latency,
    );
    r.latency(
        "page tab: slowest click to drawn",
        switches[switches.len() - 1],
        b.latency,
    );
    r.frames("page tab: frames while waiting", &switch_frames, b);

    // --- dragging a visual
    let (chart, x0, from) = visual_at(h.state(), "clusteredColumnChart");
    h.hover_at(from);
    frame(&mut h);
    press(&mut h, from, true);
    frame(&mut h);
    let mut drag = Frames::default();
    for k in 1..=60 {
        h.event(Event::PointerMoved(from + vec2(k as f32, k as f32 / 2.0)));
        drag.0.push(frame(&mut h));
    }
    let to = from + vec2(60.0, 30.0);
    let t = Instant::now();
    press(&mut h, to, false);
    until(&mut h, "the moved visual drawn", |a| {
        visual_x(a, &chart).is_some_and(|x| x != x0) && drawn(a, 0)
    });
    r.frames("drag a visual: frames of the drag", &drag, b);
    r.latency("drag a visual: release to drawn", t.elapsed(), b.latency);
    idle(&mut h, 3);

    // --- resizing a visual by its bottom-right handle
    resize(&mut h, &mut r, b);

    // --- dragging a field from the Data pane onto a visual
    h.state_mut().panes.open.insert("t:Sales".into());
    idle(&mut h, 3);
    let field = h.get_by_label("Average").rect().center();
    let (chart, _, onto) = visual_at(h.state(), "clusteredColumnChart");
    let bound = |a: &App| {
        let page = a.active_page();
        a.loaded.as_ref().and_then(|l| {
            l.snap["report"]["pages"][page]["visuals"]
                .as_array()?
                .iter()
                .find(|v| v["name"] == chart.as_str())?["fields"]
                .as_array()
                .map(Vec::len)
        })
    };
    let before = bound(h.state());
    h.hover_at(field);
    frame(&mut h);
    press(&mut h, field, true);
    frame(&mut h);
    let mut fdrag = Frames::default();
    for k in 1..=60 {
        let p = field + (onto - field) * (k as f32 / 60.0);
        h.event(Event::PointerMoved(p));
        fdrag.0.push(frame(&mut h));
    }
    let t = Instant::now();
    press(&mut h, onto, false);
    let (_, f) = until(&mut h, "the field bound", |a| bound(a) != before);
    let to_bound = t.elapsed();
    let (_, g) = until(&mut h, "the visual drawn with the field", |a| drawn(a, 0));
    r.frames("drag a field: frames of the drag", &fdrag, b);
    r.latency("drag a field: release to bound", to_bound, b.latency);
    r.latency("drag a field: release to drawn", t.elapsed(), b.latency);
    let mut after = f;
    after.0.extend(g.0);
    r.frames("drag a field: frames while waiting", &after, b);
    idle(&mut h, 3);

    // --- the Format visual pane
    let (table, _, at) = visual_at(h.state(), "tableEx");
    h.hover_at(at);
    frame(&mut h);
    press(&mut h, at, true);
    press(&mut h, at, false);
    until(&mut h, "the table selected", |a| {
        a.cmd.visual.as_ref().is_some_and(|v| v.visual == table)
    });
    idle(&mut h, 3);
    let t = Instant::now();
    h.get_by_role_and_label(Role::Button, "Format visual").click();
    let (_, f) = until(&mut h, "the Format pane read", |a| {
        a.panes.format.tab == crate::format_pane::Tab::Format
            && a.panes.format.own.is_some()
            && a.calls_pending() == 0
    });
    r.latency("Format visual: click to pane read", t.elapsed(), b.latency);
    let mut pane = f;
    pane.0.extend(idle(&mut h, 30).0);
    r.frames("Format visual: frames", &pane, b);

    // --- the DAX formula bar
    h.state_mut().cmd.visual = None;
    h.state_mut().panes.open.insert("t:Sales".into());
    idle(&mut h, 3);
    h.get_by_label("Average").click();
    until(&mut h, "Average's DAX in the bar", |a| {
        a.formula.text == "Average = DIVIDE([Total], [Units])"
    });
    idle(&mut h, 3);
    let edit = h.get_by_role(Role::MultilineTextInput).rect();
    let pos = edit.min + vec2(8.0, 6.0);
    h.hover_at(pos);
    frame(&mut h);
    press(&mut h, pos, true);
    press(&mut h, pos, false);
    idle(&mut h, 2);
    let end = h.state().formula.text.chars().count();
    h.state_mut().formula.cursor_to = Some(end);
    idle(&mut h, 2);
    assert!(crate::formula::has_focus(&h.ctx), "the bar has the keyboard");
    let mut typing = Frames::default();
    for c in " * 1 + 0 * [Total] ".chars() {
        h.event(Event::Text(c.to_string()));
        typing.0.push(frame(&mut h));
        typing.0.push(frame(&mut h));
    }
    assert!(
        h.state().formula.text.ends_with("* 1 + 0 * [Total] "),
        "{}",
        h.state().formula.text
    );
    r.frames("formula bar: typing, per frame", &typing, b);
    h.key_press(egui::Key::Escape);
    idle(&mut h, 3);

    // --- the Data view
    let t = Instant::now();
    h.state_mut().tab = Tab::Data;
    let (_, f) = until(&mut h, "the Data view's first rows", |a| {
        a.cmd.data.row_count == Some(rows) && a.cmd.data.cell(0, 0).is_some()
    });
    r.latency("Data view: switch to first rows", t.elapsed(), b.latency);
    let mut grid = f;
    grid.0.extend(idle(&mut h, 3).0);
    let over = h.get_by_label("Product").rect().center() + vec2(0.0, 200.0);
    h.hover_at(over);
    frame(&mut h);
    let mut scroll = Frames::default();
    for _ in 0..60 {
        h.event(Event::MouseWheel {
            source: egui::MouseWheelSource::Wheel,
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -(rows as f32) / 4.0),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        });
        scroll.0.push(frame(&mut h));
    }
    let t = Instant::now();
    until(&mut h, "the rows scrolled to", |a| {
        let v = &a.cmd.data.visible;
        v.start > rows / 2 && a.cmd.data.cell(v.start, 0).is_some()
    });
    r.frames("Data view: scrolling, per frame", &scroll, b);
    r.latency("Data view: scrolled rows read", t.elapsed(), b.latency);

    // --- the Power Query editor
    h.state_mut().tab = Tab::Report;
    h.state_mut().ribbon_tab = RTab::Home;
    idle(&mut h, 4);
    let t = Instant::now();
    h.get_by_label("Transform data").click();
    let (_, f) = until(&mut h, "the query preview", |a| {
        a.cmd
            .pq
            .as_ref()
            .is_some_and(|e| !e.busy && matches!(e.preview, Some(Ok(_))))
    });
    r.latency("Transform data: click to preview", t.elapsed(), b.latency);
    let mut pq = f;
    pq.0.extend(idle(&mut h, 30).0);
    r.frames("Power Query editor: frames", &pq, b);
    let shown = h
        .get_all_by(|n| n.role() == Role::Cell || n.role() == Role::Label)
        .count();
    println!("Power Query editor: {shown} nodes");
    r
}

/// The gate's run: a report of [`ROWS`] rows.
#[test]
fn a_large_report_stays_responsive() {
    run(ROWS, &GATE);
}

/// The nightly run (`cargo xtask heavy`): a report of [`HEAVY_ROWS`] rows.
#[test]
fn a_million_row_report_stays_responsive() {
    run(HEAVY_ROWS, &HEAVY);
}
