//! Opening a file from disk as the desktop opens it (docs/ui-perf.md, "Opening a file"): a
//! `.pbix` and a `.pbip` that BIcraft wrote itself, opened through the window's real path (the
//! Open command's `App::open_path`, the local source of `src/local.rs` on its worker thread, the
//! render thread), and the same file taken apart stage by stage on the test's thread to see
//! where the time goes: reading the file, unzipping it, decompressing the `DataModel` (XPress9),
//! reading its backup's metadata (ABF, SQLite), `file.open` in full, the snapshots until every
//! visual has its data, and rendering the first page.
//!
//! The gate opens a report of [`ROWS`] rows. `BICRAFT_OPEN_BENCH_ROWS=1000000,10000000` times
//! larger ones, generated once into `BICRAFT_OPEN_BENCH_DIR` when it is set (kept between runs);
//! the files under `BICRAFT_TESTDATA` (`*.pbix`, `*.pbip`) are timed too when it is set, printed
//! by number only. Budgets are generous: they catch an open that becomes minutes.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bicraft_engine::Session;
use bicraft_engine::watch::data_pending;
use egui_kittest::Harness;
use serde_json::{Value, json};

use super::{Temp, harness};
use crate::App;
use bicraft_ui::app::Tab;

/// Fact rows of the gate's files.
const ROWS: usize = 100_000;
/// From `open_path` to the first page drawn with its data, in the gate.
const BUDGET: Duration = Duration::from_secs(60);
/// The longest wait for any one result on a large file.
const WAIT: Duration = Duration::from_secs(900);

/// Sales(Id, Product, Region, Amount, Qty) of `rows` rows made by M, a calculated column, three
/// measures; page `P1` with cards, a slicer, a table, a matrix, a column chart, a line chart and
/// a gauge, page `P2` with a pie chart and a bar chart.
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
        ("model.new", json!({"name": "OpenSpeed"})),
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
    s.execute("page.add", &json!({"display_name": "P2"})).unwrap();
    let pages: Vec<String> = s
        .report
        .as_ref()
        .unwrap()
        .pages
        .iter()
        .map(|p| p.name.clone())
        .collect();
    let by = |cat: &str, val: &str| json!([{"role": "Category", "field": cat}, {"role": "Y", "field": val}]);
    let p1 = vec![
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
            "slicer",
            [640, 10, 200, 220],
            json!([{"role": "Values", "field": "Sales[Region]"}]),
        ),
        (
            "tableEx",
            [850, 10, 420, 340],
            json!([{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "[Total]"}]),
        ),
        (
            "pivotTable",
            [10, 120, 620, 230],
            json!([{"role": "Rows", "field": "Sales[Region]"}, {"role": "Values", "field": "[Average]"}]),
        ),
        (
            "clusteredColumnChart",
            [10, 360, 410, 350],
            by("Sales[Region]", "[Total]"),
        ),
        ("lineChart", [430, 360, 410, 350], by("Sales[Product]", "[Units]")),
        (
            "gauge",
            [850, 360, 420, 350],
            json!([{"role": "Y", "field": "[Total]"}]),
        ),
    ];
    let p2 = vec![
        ("pieChart", [10, 10, 400, 350], by("Sales[Region]", "[Units]")),
        (
            "clusteredBarChart",
            [420, 10, 400, 350],
            by("Sales[Region]", "[Average]"),
        ),
    ];
    for (page, visuals) in pages.iter().zip([p1, p2]) {
        for (ty, r, fields) in visuals {
            s.execute(
                "visual.add",
                &json!({"page": page, "visual_type": ty, "x": r[0], "y": r[1], "width": r[2],
                        "height": r[3], "fields": fields}),
            )
            .unwrap_or_else(|e| panic!("{ty}: {e}"));
        }
    }
    s
}

/// `open-<rows>.pbix` and `open-<rows>.pbip` in `dir`, written by BIcraft (the project's
/// `.pbi/cache.abf` from the package's data); kept when they are already there.
fn fixtures(dir: &Path, rows: usize) -> (PathBuf, PathBuf) {
    let pbix = dir.join(format!("open-{rows}.pbix"));
    let pbip = dir.join(format!("open-{rows}.pbip"));
    if !pbix.exists() || !pbip.exists() {
        let t = Instant::now();
        let mut s = session(rows);
        s.execute("file.save_as", &json!({"path": pbix, "overwrite": true}))
            .unwrap_or_else(|e| panic!("save the pbix: {e}"));
        s.execute("file.open", &json!({"path": pbix})).unwrap();
        s.execute("file.save_as", &json!({"path": pbip, "overwrite": true}))
            .unwrap_or_else(|e| panic!("save the pbip: {e}"));
        println!(
            "generated the {rows}-row files in {:.1} s",
            t.elapsed().as_secs_f64()
        );
    }
    (pbix, pbip)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn line(what: &str, d: Duration) {
    println!("  {what:<40} {:>10.1} ms", ms(d));
}

fn timed<T>(what: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let v = f();
    line(what, t.elapsed());
    v
}

/// Whether every visual of page `page` of `snap` has its data (or binds no field).
fn page_ready(snap: &Value, page: usize) -> bool {
    snap.pointer(&format!("/report/pages/{page}/visuals"))
        .and_then(Value::as_array)
        .is_some_and(|vs| {
            vs.iter().all(|v| {
                let bound = v["fields"].as_array().is_some_and(|f| !f.is_empty());
                !bound || !v["data"].is_null()
            })
        })
}

/// The file taken apart on this thread, stage by stage; `own`: a file this test wrote (its errors
/// are printed; a local file's could quote its content).
fn stages(path: &Path, own: bool) {
    let pbix = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pbix"));
    if pbix {
        let bytes = timed("read the file", || std::fs::read(path).unwrap_or_default());
        println!("  {:<40} {:>10.1} MB", "file size", bytes.len() as f64 / 1e6);
        drop(bytes);
        let pkg = timed("unzip (opc::Package::open)", || bicraft_opc::Package::open(path));
        if let Ok(pkg) = pkg
            && let Some(part) = pkg.get(bicraft_pbix::DATA_MODEL)
        {
            let abf = timed("decompress the DataModel (XPress9)", || {
                bicraft_xpress9::decompress_datamodel(&part.data, usize::MAX)
            });
            if let Ok(abf) = abf {
                println!("  {:<40} {:>10.1} MB", "backup size", abf.len() as f64 / 1e6);
                timed("metadata (ABF, SQLite, model)", || {
                    bicraft_abf::Abf::parse(&abf)
                        .ok()
                        .and_then(|a| a.metadata_sqlitedb().map(bicraft_metadata::read_sqlitedb))
                });
            }
            let _ = timed("report (PBIR or layout)", || bicraft_pbix::read_report(&pkg));
        }
    }
    let mut s = Session::new();
    let r = timed("file.open", || s.execute("file.open", &json!({"path": path})));
    if let Err(e) = r {
        println!("  file.open failed: {e}");
        return;
    }
    // the Performance analyzer records each visual's evaluation (BIcraft's own times)
    let _ = s.execute("perf.start", &json!({}));
    let t = Instant::now();
    let mut snap = timed("first snapshot", || s.snapshot());
    let mut n = 1;
    while !page_ready(&snap, 0) && t.elapsed() < WAIT {
        snap = s.snapshot();
        n += 1;
    }
    line(&format!("snapshots to page 1's data ({n})"), t.elapsed());
    while data_pending(&snap) && t.elapsed() < WAIT {
        snap = s.snapshot();
        n += 1;
    }
    line(&format!("snapshots to every page's data ({n})"), t.elapsed());
    // a visual whose data failed is fast and wrong: counted, so a fast line is not misread
    let statuses: Vec<String> = snap
        .pointer("/report/pages/0/visuals")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|v| !v["data"].is_null())
        .map(|v| v["data"]["status"].as_str().unwrap_or("?").to_string())
        .collect();
    let ok = statuses.iter().filter(|s| *s == "ok").count();
    println!(
        "  {:<40} {ok} of {} ok",
        "page 1's visuals with data",
        statuses.len()
    );
    if own
        && ok < statuses.len()
        && let Some(e) = snap
            .pointer("/report/pages/0/visuals")
            .and_then(Value::as_array)
            .and_then(|vs| {
                vs.iter()
                    .find(|v| v["data"]["status"] != "ok" && !v["data"].is_null())
            })
    {
        let d = e["data"].to_string();
        println!("  first failure: {}", d.get(..300).unwrap_or(&d));
    }
    if let Ok(r) = s.execute("perf.results", &json!({})) {
        for e in r["events"].as_array().into_iter().flatten() {
            let ty = e["visual_type"].as_str().unwrap_or("?");
            let ms = e["evaluation_ms"].as_f64().unwrap_or(f64::NAN);
            println!("    evaluate {ty:<34} {ms:>10.1} ms");
        }
    }
    timed("one more snapshot (all data cached)", || s.snapshot());
    let assets = s.snapshot_assets().into_iter().map(|a| (a.id, a.bytes)).collect();
    let fonts = bicraft_ui::pages::font_book();
    let key = bicraft_ui::pages::Key::new(1, 0, 1.0);
    let _ = timed("render page 1 (scale 1)", || {
        bicraft_ui::pages::render(&snap, &assets, &fonts, key).map(|_| ())
    });
}

/// Steps the window until `done` holds; `None` when the window shows an error first.
fn until(h: &mut Harness<'_, App>, what: &str, limit: Duration, done: impl Fn(&App) -> bool) -> Option<()> {
    let t = Instant::now();
    while !done(h.state()) {
        if let Some(e) = &h.state().error {
            println!("  window: an error before {what}: {} characters", e.len());
            return None;
        }
        assert!(
            t.elapsed() < limit,
            "never: {what} (status {:?})",
            h.state().status
        );
        h.step();
        std::thread::sleep(Duration::from_millis(1));
    }
    Some(())
}

/// Opens `path` in the window; returns the time to the first page drawn with its data (`None`:
/// the window showed an error).
fn window(path: &Path, limit: Duration) -> Option<Duration> {
    let t0 = Instant::now();
    let mut app = crate::app("en-US", None);
    app.tab = Tab::Report;
    app.open_path(path);
    let mut h = harness(app, (1600.0, 960.0));
    h.step();
    line("window: first frame", t0.elapsed());
    until(&mut h, "the first snapshot", limit, |a| a.loaded.is_some())?;
    line("window: first snapshot", t0.elapsed());
    let page_drawn = |a: &App| {
        let (Some(l), Some((key, Ok(_)))) = (&a.loaded, &a.shown) else {
            return false;
        };
        key.page == 0 && key.seq == l.seq && page_ready(&l.snap, 0)
    };
    until(
        &mut h,
        "page 1 drawn",
        limit,
        |a| matches!((&a.loaded, &a.shown), (Some(l), Some((k, Ok(_)))) if k.page == 0 && k.seq == l.seq),
    )?;
    line("window: page 1 drawn (with or without data)", t0.elapsed());
    until(&mut h, "page 1 drawn with its data", limit, page_drawn)?;
    let first = t0.elapsed();
    line("window: page 1 drawn with its data", first);
    until(&mut h, "every page's data", limit, |a| {
        a.loaded.as_ref().is_some_and(|l| !data_pending(&l.snap))
    })?;
    line("window: every page's data", t0.elapsed());
    Some(first)
}

fn bench(label: &str, path: &Path, limit: Duration, own: bool) -> Option<Duration> {
    println!("{label}");
    stages(path, own);
    window(path, limit)
}

/// The files under `BICRAFT_TESTDATA`: `*.pbix` and `*.pbip`, three folders deep at most.
fn testdata() -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth > 0 {
                    walk(&p, depth - 1, out);
                }
            } else if p
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("pbix") || e.eq_ignore_ascii_case("pbip"))
            {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    if let Some(root) = bicraft_env::var_os("BICRAFT_TESTDATA") {
        walk(Path::new(&root), 3, &mut out);
    }
    out.sort();
    out
}

#[test]
fn opening_files_from_disk() {
    let rows: Vec<usize> = bicraft_env::var("BICRAFT_OPEN_BENCH_ROWS")
        .ok()
        .map(|v| v.split(',').filter_map(|n| n.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![ROWS]);
    let temp = Temp::new("open-speed");
    let dir = bicraft_env::var_os("BICRAFT_OPEN_BENCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| temp.path().to_path_buf());
    let _ = std::fs::create_dir_all(&dir);
    for n in rows {
        let (pbix, pbip) = fixtures(&dir, n);
        let limit = if n <= ROWS { BUDGET } else { WAIT };
        for (kind, path) in [("pbix", &pbix), ("pbip", &pbip)] {
            let d = bench(&format!("{n} rows, .{kind}"), path, limit, true)
                .unwrap_or_else(|| panic!("{n} rows .{kind}: the window showed an error"));
            assert!(d < limit, "{n} rows .{kind}: page 1 drawn after {d:?}");
        }
    }
    for (i, path) in testdata().iter().enumerate() {
        // by number only: the files and their names stay local (AGENTS.md §2)
        bench(&format!("BICRAFT_TESTDATA file {}", i + 1), path, WAIT, false);
    }
}
