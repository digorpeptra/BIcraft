//! The memory budget (`docs/beyond-ram.md`, lot 1): under a budget the answers are the same as
//! without one, evicted columns are read again, nothing keeps an evicted column alive, a column
//! that cannot fit is refused cleanly, and the backup's temporary file goes with its store.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::Session;
use crate::tempdir::TempDir;

pub(super) fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A star of our own: `Sales` (6,000 rows, six columns of similar size) and `Product` (50 rows).
pub(super) fn build(path: &Path) {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Budget"}));
    let sales: Vec<String> = (0..6000)
        .map(|i| {
            format!(
                "{{{}, \"S{}\", {}, {}, {}, \"{}\"}}",
                i % 50,
                i % 20,
                i % 7 + 1,
                (i * 13) % 997,
                i % 365,
                ["red", "green", "blue", "black"][i % 4]
            )
        })
        .collect();
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "m": format!(
            "let Source = #table(type table [ProductKey = Int64.Type, Store = text, Qty = Int64.Type, \
             Amount = Int64.Type, Day = Int64.Type, Color = text], {{{}}}) in Source",
            sales.join(", ")
        )}),
    );
    let products: Vec<String> = (0..50)
        .map(|k| format!("{{{k}, \"C{}\", {}}}", k % 5, 10 + k * 3))
        .collect();
    run(
        &mut s,
        "table.import",
        json!({"name": "Product", "m": format!(
            "let Source = #table(type table [ProductKey = Int64.Type, Category = text, Price = Int64.Type], {{{}}}) in Source",
            products.join(", ")
        )}),
    );
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[ProductKey]", "to": "Product[ProductKey]"}),
    );
    run(&mut s, "file.save_as", json!({"path": path}));
}

pub(super) const QUERIES: &[&str] = &[
    "EVALUATE ROW(\"s\", SUM(Sales[Amount]), \"n\", COUNTROWS(Sales), \"d\", DISTINCTCOUNT(Sales[Store]))",
    "EVALUATE SUMMARIZECOLUMNS(Product[Category], \"a\", SUM(Sales[Amount]), \"q\", SUM(Sales[Qty])) ORDER BY Product[Category]",
    "EVALUATE SUMMARIZECOLUMNS(Sales[Store], Product[Category], \"a\", SUM(Sales[Amount])) ORDER BY Sales[Store], Product[Category]",
    "EVALUATE ROW(\"f\", CALCULATE(SUM(Sales[Amount]), Product[Category] = \"C1\"))",
    "EVALUATE TOPN(5, SUMMARIZECOLUMNS(Sales[Store], \"a\", SUM(Sales[Amount])), [a], DESC) ORDER BY [a] DESC, Sales[Store]",
    "EVALUATE ROW(\"x\", SUMX(Sales, Sales[Qty] * RELATED(Product[Price])))",
    "EVALUATE ROW(\"c\", COUNTROWS(FILTER(Sales, Sales[Day] > 100 && Sales[Color] = \"red\")))",
    "EVALUATE SUMMARIZECOLUMNS(Sales[Color], \"m\", MIN(Sales[Amount]), \"hi\", MAX(Sales[Day]), \"v\", AVERAGE(Sales[Qty])) ORDER BY Sales[Color]",
    "EVALUATE ROW(\"r\", CALCULATE(DISTINCTCOUNT(Sales[ProductKey]), Sales[Color] = \"blue\", Product[Price] > 50))",
];

pub(super) fn answers(s: &mut Session) -> Vec<Value> {
    QUERIES
        .iter()
        .map(|q| run(s, "dax.evaluate", json!({"query": q}))["results"].clone())
        .collect()
}

pub(super) fn memory(s: &mut Session, p: Value) -> Value {
    run(s, "dax.memory", p)
}

fn open(path: &Path) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    s
}

#[test]
fn answers_under_a_tiny_budget_are_the_answers_without_one() {
    let dir = TempDir::new("budget-same");
    let path = dir.join("budget.pbix");
    build(&path);

    let mut free = open(&path);
    memory(&mut free, json!({"budget": "unlimited"}));
    let reference = answers(&mut free);
    let m = memory(&mut free, json!({}));
    assert_eq!(m["evictions"], 0);
    assert_eq!(m["backup"]["where"], "memory", "no budget: nothing is written");
    let largest = m["largest_column_bytes"].as_u64().unwrap();
    let total = m["column_bytes"].as_u64().unwrap();
    assert!(
        total > largest + largest / 4,
        "the columns do not fit together: {m}"
    );

    // a budget that holds the largest column, not all of them
    let mut tight = open(&path);
    let m = memory(&mut tight, json!({"budget": largest + largest / 4}));
    assert_eq!(m["budget_source"], "session");
    // the store opens with the first query; its backup moves to the file after the answer
    let first = run(
        &mut tight,
        "dax.evaluate",
        json!({"query": QUERIES.first().copied().unwrap()}),
    );
    assert_eq!(first["results"], reference[0]);
    assert_eq!(memory(&mut tight, json!({}))["backup"]["where"], "memory");
    tight.background_work();
    let mut moved = false;
    for _ in 0..200 {
        if memory(&mut tight, json!({}))["backup"]["where"] == "file" {
            moved = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(moved, "background work moves the backup to its file");
    for round in 0..2 {
        assert_eq!(answers(&mut tight), reference, "round {round}");
        run(&mut tight, "dax.clear_cache", json!({}));
    }
    // warm (the storage caches kept, when they fit), then cold again
    assert_eq!(answers(&mut tight), reference);
    let m = memory(&mut tight, json!({}));
    assert_eq!(m["backup"]["where"], "file", "{m}");
    assert!(m["spill_error"].is_null(), "{m}");
    assert!(m["evictions"].as_u64().unwrap() > 0, "{m}");
    assert!(
        m["column_bytes"].as_u64().unwrap() <= largest + largest / 4,
        "kept columns over the budget: {m}"
    );
    assert_eq!(m["refused"], 0);
}

#[test]
fn an_evicted_column_is_not_kept_alive_and_comes_back_the_same() {
    let dir = TempDir::new("budget-weak");
    let path = dir.join("budget.pbix");
    build(&path);
    let mut s = open(&path);
    memory(&mut s, json!({"budget": "1G"}));
    let before = answers(&mut s);
    // every column read, the relationship indexes built from some of them
    let weak: Vec<std::sync::Weak<_>> = {
        let st = s.preview.lock();
        st.columns
            .values()
            .filter_map(|c| c.as_ref().ok().map(Arc::downgrade))
            .collect()
    };
    assert!(weak.len() >= 6);
    // one byte: every column not in use goes, with what recognizes it by address
    let m = memory(&mut s, json!({"budget": 1}));
    assert_eq!(m["columns"], 0, "{m}");
    assert!(
        weak.iter().all(|w| w.upgrade().is_none()),
        "an evicted column is still held somewhere"
    );
    // read again: the same answers, the relationship indexes rebuilt
    memory(&mut s, json!({"budget": "1G"}));
    assert_eq!(answers(&mut s), before);
}

#[test]
fn a_column_over_the_budget_is_refused_and_the_session_goes_on() {
    let dir = TempDir::new("budget-refuse");
    let path = dir.join("budget.pbix");
    build(&path);
    let mut s = open(&path);
    memory(&mut s, json!({"budget": "1K"}));
    let q = json!({"query": QUERIES.first().copied().unwrap()});
    let e = s.execute("dax.evaluate", &q).expect_err("over the budget");
    assert!(e.contains("memory budget"), "{e}");
    assert!(e.contains("Sales"), "{e}");
    assert!(memory(&mut s, json!({}))["refused"].as_u64().unwrap() > 0);
    // back to no budget: the same session answers
    memory(&mut s, json!({"budget": 0}));
    let v = run(&mut s, "dax.evaluate", q);
    assert_eq!(v["results"][0]["row_count"], 1);
}

#[test]
fn bad_budgets_are_refused() {
    let mut s = Session::new();
    for bad in [json!("lots"), json!(-1), json!(1.5), json!(true), json!("5X")] {
        assert!(s.execute("dax.memory", &json!({"budget": bad})).is_err(), "{bad}");
    }
    assert!(s.execute("dax.memory", &json!({"other": 1})).is_err());
    let m = run(&mut s, "dax.memory", json!({"budget": "300M"}));
    assert_eq!(m["budget"], 300u64 << 20);
    let m = run(&mut s, "dax.memory", json!({"budget": null}));
    assert_ne!(m["budget_source"], "session");
}

/// Re-reads below the working set: a query over three columns whose budget holds about two and
/// a half of them. Least recently used first, each run (storage caches cleared between runs)
/// evicted mid-query the very columns the query read next, and read all three again; now a run
/// reads again at most one. Without clearing, the small storage cache stays beside the columns.
/// The answers never change.
#[test]
fn a_query_repeated_over_more_columns_than_fit_reads_again_only_one() {
    let dir = TempDir::new("budget-repeat");
    let path = dir.join("budget.pbix");
    build(&path);
    let q = json!({"query": "EVALUATE ROW(\"a\", SUM(Sales[Amount]), \"q\", SUM(Sales[Qty]), \"d\", MAX(Sales[Day]))"});
    let mut free = open(&path);
    memory(&mut free, json!({"budget": "unlimited"}));
    let reference = run(&mut free, "dax.evaluate", q.clone())["results"].clone();
    let m = memory(&mut free, json!({}));
    let three = m["column_bytes"].as_u64().unwrap();
    assert_eq!(m["columns"], 3, "{m}");

    let mut s = open(&path);
    memory(&mut s, json!({"budget": three * 5 / 6}));
    let reads = |s: &mut Session| memory(s, json!({}))["column_reads"].as_u64().unwrap();
    let mut per_run = Vec::new();
    for _ in 0..5 {
        let before = reads(&mut s);
        assert_eq!(run(&mut s, "dax.evaluate", q.clone())["results"], reference);
        per_run.push(reads(&mut s) - before);
        let m = memory(&mut s, json!({}));
        assert!(
            m["column_bytes"].as_u64().unwrap() <= three * 5 / 6,
            "kept columns over the budget: {m}"
        );
        run(&mut s, "dax.clear_cache", json!({}));
    }
    assert_eq!(per_run[0], 3);
    assert!(
        per_run[1..].iter().all(|n| *n <= 1),
        "columns read per run: {per_run:?}"
    );
    // without clearing, the small storage cache stays beside the columns
    run(&mut s, "dax.evaluate", q.clone());
    assert_eq!(run(&mut s, "dax.evaluate", q)["results"], reference);
    let m = memory(&mut s, json!({}));
    assert!(m["storage_cache_bytes"].as_u64().unwrap() > 0, "{m}");
    assert!(m["column_bytes"].as_u64().unwrap() <= three * 5 / 6, "{m}");
}

/// Columns used often stay ahead of a column used once: after a one-off query over another
/// column, the queries of the usual working set do not read their columns again.
#[test]
fn a_one_off_query_does_not_evict_the_usual_columns() {
    let dir = TempDir::new("budget-oneoff");
    let path = dir.join("budget.pbix");
    build(&path);
    let usual = [
        "EVALUATE ROW(\"a\", SUM(Sales[Amount]))",
        "EVALUATE ROW(\"q\", SUM(Sales[Qty]), \"a\", MIN(Sales[Amount]))",
    ];
    let mut free = open(&path);
    memory(&mut free, json!({"budget": "unlimited"}));
    for q in usual {
        run(&mut free, "dax.evaluate", json!({ "query": q }));
    }
    let two = memory(&mut free, json!({}))["column_bytes"].as_u64().unwrap();
    let mut s = open(&path);
    // the two usual columns and a little more
    memory(&mut s, json!({"budget": two + two / 4}));
    let reads = |s: &mut Session| memory(s, json!({}))["column_reads"].as_u64().unwrap();
    for _ in 0..4 {
        for q in usual {
            run(&mut s, "dax.evaluate", json!({ "query": q }));
            run(&mut s, "dax.clear_cache", json!({}));
        }
    }
    // a one-off query over a third column, then the usual ones: one of them is read again at
    // most (the one-off's column may stay as the last query's)
    run(
        &mut s,
        "dax.evaluate",
        json!({"query": "EVALUATE ROW(\"d\", MAX(Sales[Day]))"}),
    );
    run(&mut s, "dax.clear_cache", json!({}));
    let before = reads(&mut s);
    for _ in 0..3 {
        for q in usual {
            run(&mut s, "dax.evaluate", json!({ "query": q }));
            run(&mut s, "dax.clear_cache", json!({}));
        }
    }
    assert!(
        reads(&mut s) - before <= 1,
        "usual columns read again: {}",
        reads(&mut s) - before
    );
}

/// The backup moved to a file gives the same columns, and the file goes with the store.
#[test]
fn a_spilled_backup_reads_the_same_columns_and_its_file_goes_with_it() {
    let dir = TempDir::new("budget-spill");
    let path = dir.join("budget.pbix");
    build(&path);
    let s = open(&path);
    let part = super::stored_backup(&s).expect("a DataModel").into_owned();
    let mem = super::store::VertipaqStore::open(&part, super::MAX_BACKUP_BYTES).unwrap();
    let file = super::store::VertipaqStore::open(&part, super::MAX_BACKUP_BYTES).unwrap();
    let spill = TempDir::new("budget-spill-files");
    file.spill(spill.path()).unwrap();
    assert_eq!(file.backing().0, "file");
    assert_eq!(mem.backing().0, "memory");
    assert_eq!(file.backing().1, mem.backing().1);
    for c in ["ProductKey", "Store", "Qty", "Amount", "Day", "Color"] {
        let a = mem.column("Sales", c, usize::MAX).unwrap();
        let b = file.column("Sales", c, usize::MAX).unwrap();
        assert_eq!(a.values, b.values, "{c}");
        assert_eq!(a.codes, b.codes, "{c}");
        assert!(file.column_file_bytes("Sales", c).unwrap() > 0);
    }
    let copy = file.clone();
    drop(file);
    assert!(
        copy.column("Sales", "Qty", usize::MAX).is_ok(),
        "a clone shares the file"
    );
    drop(copy);
    // a clone made before the move shares it too
    let before = mem.clone();
    before.spill(spill.path()).unwrap();
    assert_eq!(mem.backing().0, "file");
    assert!(mem.column("Sales", "Day", usize::MAX).is_ok());
    drop((before, mem));
    let left = std::fs::read_dir(spill.path()).unwrap().count();
    assert_eq!(left, 0, "the temporary file is removed with its last store");
}

/// Runs `f` with this thread's spill folder set to `dir`.
pub(super) fn with_spill_dir<R>(dir: &Path, f: impl FnOnce() -> R) -> R {
    super::memory::TEST_SPILL_DIR.with(|d| *d.borrow_mut() = Some(dir.to_path_buf()));
    let r = f();
    super::memory::TEST_SPILL_DIR.with(|d| *d.borrow_mut() = None);
    r
}

pub(super) fn files_in(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |d| d.count())
}

/// Lot 2: a budget set before `file.open` leaves the `DataModel` part in the package file and
/// decompresses the backup as a stream straight to its temporary file; the answers are the same,
/// a save still writes the data, and the file goes with the session.
#[test]
fn a_budget_at_open_streams_the_backup_to_its_file_and_answers_the_same() {
    let dir = TempDir::new("budget-open");
    let path = dir.join("budget.pbix");
    build(&path);
    let reference = answers(&mut open(&path));
    let spill = TempDir::new("budget-open-spill");
    with_spill_dir(spill.path(), || {
        let mut s = Session::new();
        let m = memory(&mut s, json!({"budget": "64M"}));
        assert_eq!(m["budget_source"], "session");
        run(&mut s, "file.open", json!({"path": path}));
        {
            let Some(crate::Source::Package { package, .. }) = s.source.as_ref() else {
                panic!("a package")
            };
            let part = package.get(bicraft_pbix::DATA_MODEL).unwrap();
            assert!(part.is_deferred(), "the DataModel stays in the file");
            assert!(part.data.is_empty());
        }
        assert_eq!(s.model.as_ref().unwrap().tables.len(), 2);
        for round in 0..2 {
            assert_eq!(answers(&mut s), reference, "round {round}");
            run(&mut s, "dax.clear_cache", json!({}));
        }
        let m = memory(&mut s, json!({}));
        assert_eq!(m["backup"]["where"], "file", "{m}");
        assert!(m["spill_error"].is_null(), "{m}");
        // a model edit saved: the data is read from the package file for the write
        run(
            &mut s,
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        );
        let saved = dir.join("saved.pbix");
        run(&mut s, "file.save_as", json!({"path": saved}));
        let mut back = open(&saved);
        assert_eq!(answers(&mut back), reference);
        let v = run(
            &mut back,
            "dax.evaluate",
            json!({"query": "EVALUATE ROW(\"t\", [Total])"}),
        );
        let w = run(
            &mut back,
            "dax.evaluate",
            json!({"query": "EVALUATE ROW(\"t\", SUM(Sales[Amount]))"}),
        );
        assert_eq!(v["results"][0]["rows"], w["results"][0]["rows"], "{v}");
        assert_eq!(v["results"][0]["row_count"], 1, "{v}");
        drop(s);
        drop(back);
    });
    assert_eq!(
        files_in(spill.path()),
        0,
        "the temporary file goes with the session"
    );
}

/// The streamed store reads the same columns as the store decompressed in memory, and a broken
/// part leaves no temporary file behind.
#[test]
fn a_streamed_backup_reads_the_same_columns_and_errors_leave_no_file() {
    let dir = TempDir::new("budget-stream");
    let path = dir.join("budget.pbix");
    build(&path);
    let s = open(&path);
    let part = super::stored_backup(&s).expect("a DataModel").into_owned();
    let mem = super::store::VertipaqStore::open(&part, super::MAX_BACKUP_BYTES).unwrap();
    let spill = TempDir::new("budget-stream-files");
    let mut opener = || Ok(Box::new(std::io::Cursor::new(part.clone())) as Box<dyn std::io::Read + Send>);
    let (file, sqlitedb) =
        super::store::VertipaqStore::open_streamed(&mut opener, spill.path(), super::MAX_BACKUP_BYTES)
            .unwrap();
    assert!(!sqlitedb.is_empty());
    assert_eq!(file.backing().0, "file");
    assert_eq!(file.backing().1, mem.backing().1);
    for c in ["ProductKey", "Store", "Qty", "Amount", "Day", "Color"] {
        let a = mem.column("Sales", c, usize::MAX).unwrap();
        let b = file.column("Sales", c, usize::MAX).unwrap();
        assert_eq!(a.values, b.values, "{c}");
        assert_eq!(a.codes, b.codes, "{c}");
    }
    drop(file);
    assert_eq!(files_in(spill.path()), 0);
    // broken parts: an error, no file left
    let mut broken = part.clone();
    let mid = broken.len() / 2;
    for b in broken.iter_mut().skip(mid).take(64) {
        *b ^= 0xA5;
    }
    for bad in [broken, part[..part.len() / 3].to_vec(), b"garbage".to_vec()] {
        let mut opener = || Ok(Box::new(std::io::Cursor::new(bad.clone())) as Box<dyn std::io::Read + Send>);
        let r =
            super::store::VertipaqStore::open_streamed(&mut opener, spill.path(), super::MAX_BACKUP_BYTES);
        // a flipped byte may still decode (XPress9 has no checksum): then the index must fail or
        // the columns read; either way nothing is left once dropped
        drop(r);
        assert_eq!(files_in(spill.path()), 0);
    }
    // over the limit: refused, no file left
    let mut opener = || Ok(Box::new(std::io::Cursor::new(part.clone())) as Box<dyn std::io::Read + Send>);
    assert!(super::store::VertipaqStore::open_streamed(&mut opener, spill.path(), 1000).is_err());
    assert_eq!(files_in(spill.path()), 0);
    // a folder that cannot take the file: an error
    let mut opener = || Ok(Box::new(std::io::Cursor::new(part.clone())) as Box<dyn std::io::Read + Send>);
    assert!(
        super::store::VertipaqStore::open_streamed(
            &mut opener,
            &spill.path().join("missing"),
            super::MAX_BACKUP_BYTES
        )
        .is_err()
    );
}

/// A corrupted `DataModel` opened under a budget: the open reports the model error as without
/// one, queries fail cleanly, and no temporary file is left.
#[test]
fn a_broken_datamodel_under_a_budget_fails_cleanly() {
    let dir = TempDir::new("budget-broken");
    let path = dir.join("budget.pbix");
    build(&path);
    let mut pkg = bicraft_opc::Package::open(&path).unwrap();
    let part = pkg.get(bicraft_pbix::DATA_MODEL).unwrap().data.clone();
    let mut bad = part[..part.len() / 2].to_vec();
    bad.extend_from_slice(&[0u8; 100]);
    pkg.put(bicraft_pbix::DATA_MODEL, bad, true);
    let broken = dir.join("broken.pbix");
    std::fs::write(&broken, pkg.to_bytes().unwrap()).unwrap();
    let plain = {
        let mut s = Session::new();
        run(&mut s, "file.open", json!({"path": broken}))
    };
    let spill = TempDir::new("budget-broken-spill");
    with_spill_dir(spill.path(), || {
        let mut s = Session::new();
        memory(&mut s, json!({"budget": "64M"}));
        let v = run(&mut s, "file.open", json!({"path": broken}));
        assert_eq!(v["model_error"].is_null(), plain["model_error"].is_null(), "{v}");
        assert!(!v["model_error"].is_null(), "{v}");
        assert!(s.execute("dax.evaluate", &json!({"query": QUERIES[0]})).is_err());
        assert_eq!(files_in(spill.path()), 0);
    });
}

/// The bench differential (`docs/beyond-ram.md`): with `BICRAFT_BUDGET_BENCH` naming a bench
/// PBIX (`cargo xtask engine-bench`), the 22 queries of `xtask/engine-bench/queries.json` run
/// twice in one session under each budget of `BICRAFT_BUDGET_LIST` (default
/// `300M,80M,30M`; the budget is set before `file.open`) and must equal a session without a
/// budget. Ignored by default (local data); run in release with `--ignored --nocapture`.
#[test]
#[ignore]
fn bench_queries_under_budgets_equal_the_unlimited_answers() {
    let Some(pbix) = bicraft_env::var_os("BICRAFT_BUDGET_BENCH") else {
        eprintln!("BICRAFT_BUDGET_BENCH is not set: skipped");
        return;
    };
    let pbix = std::path::PathBuf::from(pbix);
    let list = bicraft_env::var("BICRAFT_BUDGET_LIST").unwrap_or_else(|_| "300M,80M,30M".into());
    let queries: Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../xtask/engine-bench/queries.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let queries: Vec<(String, String)> = queries["queries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| {
            (
                q["id"].as_str().unwrap().to_string(),
                q["dax"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    // BICRAFT_BUDGET_REPEAT: each query run that many times in a row (the bench's warm runs);
    // BICRAFT_BUDGET_TRACE: one line per run (time, columns read, segments read and found)
    let repeat: usize = bicraft_env::var("BICRAFT_BUDGET_REPEAT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
        .max(1);
    let trace = bicraft_env::var("BICRAFT_BUDGET_TRACE").is_ok();
    let rounds = |budget: Option<&str>| -> (Vec<Result<Value, String>>, Value, f64) {
        let t = std::time::Instant::now();
        let mut s = Session::new();
        if let Some(b) = budget {
            memory(&mut s, json!({ "budget": b }));
        }
        run(&mut s, "file.open", json!({ "path": pbix }));
        let mut out = Vec::new();
        let counters = |s: &mut Session| {
            let m = memory(s, json!({}));
            let n = |v: &Value| v.as_u64().unwrap_or(0);
            [
                n(&m["column_reads"]),
                n(&m["lazy_reads"]),
                n(&m["evictions"]),
                n(&m["segment_cache"]["loads"]),
                n(&m["segment_cache"]["hits"]),
            ]
        };
        for round in 0..2 {
            for (id, q) in &queries {
                for k in 0..repeat {
                    let before = if trace { counters(&mut s) } else { [0; 5] };
                    let tq = std::time::Instant::now();
                    let r = s
                        .execute(
                            "dax.evaluate",
                            &json!({"query": q, "max_rows": 1_000_000, "timeout_ms": 600_000}),
                        )
                        .map(|v| v["results"].clone());
                    let ms = tq.elapsed().as_secs_f64() * 1000.0;
                    out.push(r);
                    s.background_work();
                    if trace {
                        let after = counters(&mut s);
                        let d: Vec<u64> = after.iter().zip(&before).map(|(a, b)| a - b).collect();
                        let log = std::mem::take(&mut s.preview.lock().memory.evicted_log);
                        eprintln!(
                            "  {} r{round} {id}#{k}: {ms:8.1} ms  reads {} (lazy {}) evict {}  seg loads {} hits {}  {log:?}",
                            budget.unwrap_or("none"),
                            d[0],
                            d[1],
                            d[2],
                            d[3],
                            d[4]
                        );
                    }
                }
            }
        }
        let m = memory(&mut s, json!({}));
        (out, m, t.elapsed().as_secs_f64())
    };
    let (reference, m, secs) = rounds(None);
    eprintln!("none: {secs:.2} s, {}", m);
    for b in list.split(',').map(str::trim).filter(|b| !b.is_empty()) {
        let (got, m, secs) = rounds(Some(b));
        let mut diffs = 0;
        let mut errors = 0;
        assert_eq!(reference.len(), got.len());
        for (k, (a, g)) in reference.iter().zip(&got).enumerate() {
            let id = &queries[(k / repeat) % queries.len()].0;
            let round = k / (repeat * queries.len()) + 1;
            match (a, g) {
                (Ok(a), Ok(g)) if a == g => {}
                (Ok(_), Err(e)) => {
                    errors += 1;
                    eprintln!("  {b} {id} (round {round}): error {e}");
                }
                _ => {
                    diffs += 1;
                    eprintln!("  {b} {id} (round {round}): DIFFERENT");
                }
            }
        }
        eprintln!("{b}: {secs:.2} s, {diffs} differences, {errors} errors; {m}");
        assert_eq!(diffs, 0, "budget {b}: answers differ");
    }
}

/// Lot 3: a column read segment by segment gives the codes and values of the column read whole.
#[test]
fn a_column_read_segment_by_segment_gives_the_same_codes() {
    let dir = TempDir::new("budget-lazy-codes");
    let path = dir.join("budget.pbix");
    build(&path);
    let s = open(&path);
    let part = super::stored_backup(&s).expect("a DataModel").into_owned();
    let mem = super::store::VertipaqStore::open(&part, super::MAX_BACKUP_BYTES).unwrap();
    let spill = TempDir::new("budget-lazy-codes-files");
    let mut opener = || Ok(Box::new(std::io::Cursor::new(part.clone())) as Box<dyn std::io::Read + Send>);
    let (file, _) =
        super::store::VertipaqStore::open_streamed(&mut opener, spill.path(), super::MAX_BACKUP_BYTES)
            .unwrap();
    let cache = bicraft_scan::lazy::SegCache::new(1 << 20);
    let limits = super::store::LazyLimits {
        max_rows: usize::MAX,
        max_segment: 1 << 20,
        batch: 1,
    };
    for c in ["ProductKey", "Store", "Qty", "Amount", "Day", "Color"] {
        let a = mem.column("Sales", c, usize::MAX).unwrap();
        for store in [&mem, &file] {
            let b = store.column_lazy("Sales", c, &cache, limits).unwrap();
            assert!(b.codes.as_packed().unwrap().is_lazy());
            assert_eq!(a.values, b.values, "{c}");
            assert_eq!(a.codes, b.codes, "{c}");
            assert_eq!(a.codes.known_blank(), b.codes.known_blank(), "{c}");
        }
    }
    // a segment over the room left: refused before it is read
    let tight = super::store::LazyLimits {
        max_segment: 8,
        ..limits
    };
    let e = file.column_lazy("Sales", "Amount", &cache, tight).unwrap_err();
    assert!(
        matches!(&e, super::store::Miss::Error(t) if t.contains("segment")),
        "{e:?}"
    );
    drop((mem, file));
    assert_eq!(files_in(spill.path()), 0);
}

/// Lot 3: under a budget smaller than the largest column, the columns over half of it are read
/// segment by segment and the answers are the same; evicting them lets their segments go.
#[test]
fn answers_with_columns_read_segment_by_segment_are_the_same() {
    let dir = TempDir::new("budget-lazy-same");
    let path = dir.join("budget.pbix");
    build(&path);
    // queries over the columns of few values (their values fit any budget that holds a segment)
    const LOW: &[&str] = &[
        "EVALUATE SUMMARIZECOLUMNS(Product[Category], \"q\", SUM(Sales[Qty])) ORDER BY Product[Category]",
        "EVALUATE ROW(\"n\", COUNTROWS(Sales), \"d\", DISTINCTCOUNT(Sales[Store]))",
        "EVALUATE ROW(\"c\", COUNTROWS(FILTER(Sales, Sales[Color] = \"red\" && Sales[Qty] > 3)))",
        "EVALUATE ROW(\"r\", CALCULATE(DISTINCTCOUNT(Sales[ProductKey]), Sales[Color] = \"blue\"))",
        "EVALUATE ROW(\"x\", SUMX(Sales, Sales[Qty] * RELATED(Product[Price])))",
        "EVALUATE TOPN(5, SUMMARIZECOLUMNS(Sales[Store], \"q\", SUM(Sales[Qty])), [q], DESC) ORDER BY [q] DESC, Sales[Store]",
        "EVALUATE SUMMARIZECOLUMNS(Sales[Color], Sales[Qty], \"k\", DISTINCTCOUNT(Sales[ProductKey])) ORDER BY Sales[Color], Sales[Qty]",
    ];
    let answers = |s: &mut Session| -> Vec<Value> {
        LOW.iter()
            .map(|q| run(s, "dax.evaluate", json!({ "query": q }))["results"].clone())
            .collect()
    };
    let mut free = open(&path);
    let reference = answers(&mut free);
    // a budget under which `ProductKey` (its files over half of it) is read segment by segment
    let part = super::stored_backup(&free).expect("a DataModel").into_owned();
    let mem = super::store::VertipaqStore::open(&part, super::MAX_BACKUP_BYTES).unwrap();
    let file_bytes = mem.column_file_bytes("Sales", "ProductKey").unwrap() as u64;
    let largest = (2 * file_bytes - 1) * 4 / 3;
    let spill = TempDir::new("budget-lazy-same-spill");
    with_spill_dir(spill.path(), || {
        let mut s = Session::new();
        // (the queries' own tables are not what this test bounds: no query cap, lot 4)
        memory(&mut s, json!({ "budget": largest * 3 / 4, "query_budget": 0 }));
        run(&mut s, "file.open", json!({"path": path}));
        for round in 0..3 {
            assert_eq!(answers(&mut s), reference, "round {round}");
            run(&mut s, "dax.clear_cache", json!({}));
        }
        let m = memory(&mut s, json!({}));
        assert!(m["lazy_reads"].as_u64().unwrap() > 0, "{m}");
        assert!(m["segment_cache"]["loads"].as_u64().unwrap() > 0, "{m}");
        assert!(
            m["segment_cache"]["bytes"].as_u64().unwrap() <= m["segment_cache"]["cap"].as_u64().unwrap(),
            "{m}"
        );
        assert_eq!(m["segment_load_failures"], 0, "{m}");
        // every column evicted: the segments read on demand go with them
        let m = memory(&mut s, json!({"budget": 1}));
        assert_eq!(m["columns"], 0, "{m}");
        assert_eq!(m["segment_cache"]["segments"], 0, "{m}");
        // and come back the same
        memory(&mut s, json!({ "budget": largest * 3 / 4 }));
        assert_eq!(answers(&mut s), reference);
        drop(s);
    });
    assert_eq!(files_in(spill.path()), 0);
}

/// Lot 3, never crash: a budget smaller than one segment (or a dictionary) is a clear error, the
/// session goes on, and nothing is left on disk.
#[test]
fn a_budget_below_one_segment_is_a_clear_error() {
    let dir = TempDir::new("budget-lazy-tiny");
    let path = dir.join("budget.pbix");
    build(&path);
    let spill = TempDir::new("budget-lazy-tiny-spill");
    with_spill_dir(spill.path(), || {
        let mut s = Session::new();
        memory(&mut s, json!({"budget": 64}));
        run(&mut s, "file.open", json!({"path": path}));
        for q in QUERIES {
            let e = s
                .execute("dax.evaluate", &json!({"query": q}))
                .expect_err("below one segment");
            assert!(e.contains("memory budget"), "{e}");
        }
        let m = memory(&mut s, json!({}));
        assert!(m["refused"].as_u64().unwrap() > 0, "{m}");
        assert_eq!(m["columns"], 0, "{m}");
        memory(&mut s, json!({"budget": 0}));
        let v = run(&mut s, "dax.evaluate", json!({"query": QUERIES[0]}));
        assert_eq!(v["results"][0]["row_count"], 1);
        drop(s);
    });
    assert_eq!(files_in(spill.path()), 0);
}
