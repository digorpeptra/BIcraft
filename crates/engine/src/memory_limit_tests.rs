//! The process-wide memory limit (`docs/beyond-ram.md`, lot 5), on sessions given a pool of their
//! own (so that the limit does not reach the other tests of the process): answers under a limit
//! are the answers without one; what does not fit is refused cleanly and the session goes on;
//! reservations come back to zero; two sessions share one limit; one session's kept memory is
//! freed for another's query; background work yields; the decompression buffers are reserved.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};

use super::memory_budget_tests::{QUERIES, answers, build, files_in, memory, run, with_spill_dir};
use crate::Session;
use crate::tempdir::TempDir;

/// A pool of its own with `limit` (leaked: reservations hold `&'static` pools).
fn pool(limit: Option<usize>) -> &'static bicraft_mem::Pool {
    let p: &'static bicraft_mem::Pool = Box::leak(Box::new(bicraft_mem::Pool::new()));
    p.set_limit(limit);
    p
}

/// A session over `path` whose reservations go to `p`.
fn open_in(path: &Path, p: &'static bicraft_mem::Pool) -> Session {
    let mut s = Session::new();
    s.preview.lock().memory.pool = Some(p);
    run(&mut s, "file.open", json!({ "path": path }));
    s
}

/// What the unlimited session reads: its answers and its largest column's bytes.
fn reference(path: &Path) -> (Vec<Value>, u64, u64) {
    let mut free = Session::new();
    free.preview.lock().memory.pool = Some(pool(None));
    run(&mut free, "file.open", json!({ "path": path }));
    let r = answers(&mut free);
    let m = memory(&mut free, json!({}));
    (
        r,
        m["largest_column_bytes"].as_u64().unwrap(),
        m["column_bytes"].as_u64().unwrap(),
    )
}

fn evaluate(s: &mut Session, q: &str) -> Result<Value, String> {
    s.execute("dax.evaluate", &json!({ "query": q }))
        .map(|v| v["results"].clone())
}

#[test]
fn answers_under_a_memory_limit_are_the_answers_without_one() {
    let dir = TempDir::new("limit-same");
    let path = dir.join("limit.pbix");
    build(&path);
    let (reference, largest, total) = reference(&path);
    let spill = TempDir::new("limit-same-spill");
    with_spill_dir(spill.path(), || {
        // a limit whose half (the default budget) holds the largest column, not all of them
        let limit = usize::try_from(largest * 5 / 2).unwrap();
        assert!(total > largest * 5 / 4, "the columns must not all fit");
        let p = pool(Some(limit));
        let mut s = open_in(&path, p);
        for round in 0..3 {
            assert_eq!(answers(&mut s), reference, "round {round}");
            run(&mut s, "dax.clear_cache", json!({}));
        }
        let m = memory(&mut s, json!({}));
        assert_eq!(m["budget"], json!(limit / 2), "{m}");
        assert_eq!(m["budget_source"], "memory limit", "{m}");
        assert_eq!(m["limit"]["limit"], json!(limit), "{m}");
        assert!(m["evictions"].as_u64().unwrap() > 0, "{m}");
        let st = p.stats();
        assert!(st.peak <= limit, "peak {} over the limit {limit}: {m}", st.peak);
        assert!(st.peak > 0, "{m}");
        assert_eq!(
            st.class(bicraft_mem::Class::Query),
            0,
            "no query left anything reserved: {m}"
        );
        assert_eq!(st.active_queries, 0);
        assert_eq!(st.refused, 0, "{m}");
        // what the session holds is what it reports
        assert_eq!(
            st.used as u64,
            m["limit"]["session_reserved_bytes"].as_u64().unwrap(),
            "{m}"
        );
        drop(s);
        let st = p.stats();
        assert_eq!(
            (st.used, st.by_class),
            (0, [0; 4]),
            "everything released with the session"
        );
    });
    assert_eq!(files_in(spill.path()), 0);
}

#[test]
fn a_query_over_the_limit_is_refused_cleanly_and_the_session_goes_on() {
    let dir = TempDir::new("limit-refused");
    let path = dir.join("limit.pbix");
    build(&path);
    let (reference, largest, _) = reference(&path);
    let p = pool(None);
    let mut s = open_in(&path, p);
    // far below one column: every query reading the data is refused before it reads it
    p.set_limit(Some(usize::try_from(largest / 4).unwrap().max(1)));
    let mut refused = 0;
    for q in QUERIES {
        match evaluate(&mut s, q) {
            Ok(_) => {}
            Err(e) => {
                assert!(
                    e.contains("memory limit") || e.contains("query's cap") || e.contains("memory budget"),
                    "a clear error: {e}"
                );
                refused += 1;
            }
        }
        let st = p.stats();
        assert_eq!(st.class(bicraft_mem::Class::Query), 0, "{q}");
        assert_eq!(st.active_queries, 0, "{q}");
    }
    assert_eq!(refused, QUERIES.len(), "every query reads a column");
    let m = memory(&mut s, json!({}));
    let counted = m["refused"].as_u64().unwrap()
        + m["query"]["refused"].as_u64().unwrap()
        + m["limit"]["refused"].as_u64().unwrap();
    assert!(counted >= refused as u64, "{m}");
    assert_eq!(m["limit"]["by_class"]["query"], 0, "{m}");
    // the limit lifted: the same answers
    p.set_limit(None);
    assert_eq!(answers(&mut s), reference);
    // set again at a workable size: the same answers
    p.set_limit(Some(usize::try_from(largest * 4).unwrap()));
    assert_eq!(answers(&mut s), reference);
    drop(s);
    assert_eq!(p.stats().used, 0);
}

#[test]
fn two_sessions_share_one_limit() {
    let dir = TempDir::new("limit-two");
    let path = dir.join("limit.pbix");
    build(&path);
    let (reference, largest, _) = reference(&path);
    let limit = usize::try_from(largest * 4).unwrap();
    let p = pool(Some(limit));
    let spill = TempDir::new("limit-two-spill");
    let spill_path = spill.path().to_path_buf();
    let reference = Arc::new(reference);
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let (path, spill_path, reference) = (path.clone(), spill_path.clone(), Arc::clone(&reference));
            std::thread::spawn(move || {
                with_spill_dir(&spill_path, || {
                    let mut s = open_in(&path, p);
                    let mut refused = 0;
                    for _ in 0..4 {
                        for (q, r) in QUERIES.iter().zip(reference.iter()) {
                            match evaluate(&mut s, q) {
                                Ok(v) => assert_eq!(&v, r, "{q}"),
                                Err(e) => {
                                    assert!(e.contains("memory limit") || e.contains("share"), "{e}");
                                    refused += 1;
                                }
                            }
                        }
                    }
                    refused
                })
            })
        })
        .collect();
    let refused: Vec<usize> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    let st = p.stats();
    assert!(st.peak <= limit, "peak {} over {limit}", st.peak);
    assert_eq!(st.used, 0, "both sessions dropped: {st:?}");
    assert_eq!(st.active_queries, 0);
    // the limit holds both sessions' working sets: nothing refused
    assert_eq!(refused, vec![0, 0], "{st:?}");
    assert_eq!(files_in(spill.path()), 0);
}

#[test]
fn one_session_frees_what_it_keeps_for_another() {
    let dir = TempDir::new("limit-evict");
    let path = dir.join("limit.pbix");
    build(&path);
    let (reference, largest, total) = reference(&path);
    let p = pool(None);
    // A keeps every column (a budget of its own over all of them)
    let mut a = open_in(&path, p);
    memory(&mut a, json!({ "budget": total * 4 }));
    assert_eq!(answers(&mut a), reference);
    let held = memory(&mut a, json!({}))["columns"].as_u64().unwrap();
    assert!(held > 1);
    // then a limit just over what A keeps: B's columns need A's to go
    p.set_limit(Some(usize::try_from(total + largest).unwrap()));
    memory(&mut a, json!({}));
    let mut b = open_in(&path, p);
    assert_eq!(answers(&mut b), reference);
    let ma = memory(&mut a, json!({}));
    assert!(
        ma["columns"].as_u64().unwrap() < held,
        "A's columns were evicted for B: {ma}"
    );
    assert!(p.stats().evictions > 0);
    assert!(
        p.stats().peak <= usize::try_from(total + largest).unwrap(),
        "{:?}",
        p.stats()
    );
    // A still answers (its columns read again)
    assert_eq!(answers(&mut a), reference);
    drop(a);
    drop(b);
    assert_eq!(p.stats().used, 0);
}

/// A column that cannot be read whole beside what is already reserved (its files do not fit in
/// what the limit leaves) is read segment by segment instead, which keeps only its values and id
/// table: the query answers, within the limit. It is refused only when even that does not fit.
#[test]
fn a_column_refused_whole_under_the_limit_is_read_segment_by_segment() {
    let dir = TempDir::new("limit-fallback");
    let path = dir.join("limit.pbix");
    build(&path);
    let q = "EVALUATE ROW(\"m\", MAX(Sales[Qty]), \"n\", COUNTROWS(Sales))";
    let mut free = open_in(&path, pool(None));
    let reference = evaluate(&mut free, q).unwrap();
    let part = super::stored_backup(&free).expect("a DataModel").into_owned();
    let store = super::store::VertipaqStore::open(&part, super::MAX_BACKUP_BYTES).unwrap();
    let file = store.column_file_bytes("Sales", "Qty").unwrap();
    let whole = super::memory::base_bytes(&store.column("Sales", "Qty", usize::MAX).unwrap());
    let spill = TempDir::new("limit-fallback-spill");
    with_spill_dir(spill.path(), || {
        // a limit far above the column (its half, the budget, too), most of it held elsewhere:
        // what is left is under the column's files and under its decoded size
        let limit = 4 << 20;
        let room = file.min(whole) - 1;
        let p = pool(Some(limit));
        let mut s = open_in(&path, p);
        let hold = p.reserve(bicraft_mem::Class::Kept, limit - room).unwrap();
        let got = evaluate(&mut s, q);
        assert_eq!(
            got,
            Ok(reference.clone()),
            "files {file}, decoded {whole}, room {room}"
        );
        let m = memory(&mut s, json!({}));
        assert!(
            m["lazy_reads"].as_u64().unwrap() >= 1,
            "read segment by segment: {m}"
        );
        assert_eq!(m["refused"], 0, "{m}");
        assert!(p.stats().peak <= limit, "{:?}", p.stats());
        assert_eq!(p.stats().class(bicraft_mem::Class::Query), 0);
        drop(s);
        // with almost nothing left, even its values do not fit: a clean refusal
        let mut s = open_in(&path, p);
        drop(hold);
        let hold = p.reserve(bicraft_mem::Class::Kept, limit - 16).unwrap();
        let e = evaluate(&mut s, q).expect_err("nothing fits");
        assert!(e.contains("memory limit"), "{e}");
        assert!(p.stats().peak <= limit);
        // the session answers once the memory is back
        drop(hold);
        assert_eq!(evaluate(&mut s, q).unwrap(), reference);
        drop(s);
        assert_eq!(p.stats().used, 0);
    });
    assert_eq!(files_in(spill.path()), 0);
}

#[test]
fn background_work_waits_while_a_query_runs() {
    let p = pool(Some(1 << 30));
    let activity = Arc::new(super::general::storage::aggs::Activity::default());
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ticket = p.enter_query();
    let (act, can) = (Arc::clone(&activity), Arc::clone(&cancel));
    // when the waiter went on (it cannot before the query's ticket goes)
    let waiter = std::thread::spawn(move || {
        let ok = super::general::storage::aggs::wait_idle_for_tests(&act, &can, p);
        (ok, std::time::Instant::now())
    });
    std::thread::sleep(std::time::Duration::from_millis(400));
    assert!(!waiter.is_finished(), "a query runs: background work waits");
    let released = std::time::Instant::now();
    drop(ticket);
    let (ok, went_on) = waiter.join().unwrap();
    assert!(ok);
    assert!(went_on >= released, "it went on only once the query ended");
    // a background reservation never gets memory by eviction, and stays under 3/4 of the limit
    let small = pool(Some(1000));
    assert!(small.reserve(bicraft_mem::Class::Background, 800).is_err());
    assert!(small.reserve(bicraft_mem::Class::Background, 700).is_ok());
}

#[test]
fn the_decompression_buffers_are_reserved_and_a_refusal_leaves_no_file() {
    let dir = TempDir::new("limit-io");
    let path = dir.join("limit.pbix");
    build(&path);
    let s = Session::new();
    let part = {
        let mut s = s;
        run(&mut s, "file.open", json!({ "path": path }));
        super::stored_backup(&s).expect("a DataModel").into_owned()
    };
    let spill = TempDir::new("limit-io-spill");
    let mut opener = || -> Result<Box<dyn std::io::Read + Send + '_>, String> { Ok(Box::new(&part[..])) };
    // too small for one thread's buffers: refused, nothing written
    let tiny = pool(Some(1 << 20));
    let e = match super::store::VertipaqStore::open_streamed_in(
        &mut opener,
        spill.path(),
        super::MAX_BACKUP_BYTES,
        tiny,
    ) {
        Ok(_) => panic!("refused"),
        Err(e) => e,
    };
    assert!(e.contains("decompressing the stored data needs about"), "{e}");
    assert!(e.contains("memory limit"), "{e}");
    assert_eq!(files_in(spill.path()), 0);
    assert_eq!(tiny.stats().used, 0);
    // room for it: opened, its buffers reserved while it decodes, then released
    let roomy = pool(Some(1 << 30));
    let (store, _) = super::store::VertipaqStore::open_streamed_in(
        &mut opener,
        spill.path(),
        super::MAX_BACKUP_BYTES,
        roomy,
    )
    .unwrap();
    let st = roomy.stats();
    assert!(st.peak > 0, "buffers were reserved");
    assert_eq!(st.used, 0, "and released");
    drop(store);
    assert_eq!(files_in(spill.path()), 0);
}

/// The bench differential under memory limits (`docs/beyond-ram.md`, lot 5): with
/// `BICRAFT_BUDGET_BENCH` naming a bench PBIX, the 22 queries of `xtask/engine-bench/queries.json`
/// run twice in one session whose pool has each limit of `BICRAFT_LIMIT_LIST` (default
/// `600M,300M,150M`; set before `file.open`), and must equal a session without a limit; the pool's
/// peak reservation stays within the limit and nothing stays reserved. Ignored by default (local
/// data); run in release with `--ignored --nocapture`.
#[test]
#[ignore]
fn bench_queries_under_limits_equal_the_unlimited_answers() {
    let Some(pbix) = bicraft_env::var_os("BICRAFT_BUDGET_BENCH") else {
        eprintln!("BICRAFT_BUDGET_BENCH is not set: skipped");
        return;
    };
    let pbix = std::path::PathBuf::from(pbix);
    let list = bicraft_env::var("BICRAFT_LIMIT_LIST").unwrap_or_else(|_| "600M,300M,150M".into());
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
    // BICRAFT_BUDGET_REPEAT: each query run that many times in a row (the bench's warm runs)
    let repeat: usize = bicraft_env::var("BICRAFT_BUDGET_REPEAT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
        .max(1);
    let rounds = |p: &'static bicraft_mem::Pool| -> (Vec<Result<Value, String>>, Vec<f64>, f64) {
        let t = std::time::Instant::now();
        let mut s = Session::new();
        s.preview.lock().memory.pool = Some(p);
        run(&mut s, "file.open", json!({ "path": pbix }));
        let mut out = Vec::new();
        let mut times = Vec::new();
        for _ in 0..2 {
            for (_, q) in &queries {
                for _ in 0..repeat {
                    let tq = std::time::Instant::now();
                    let r = s
                        .execute(
                            "dax.evaluate",
                            &json!({"query": q, "max_rows": 1_000_000, "timeout_ms": 600_000}),
                        )
                        .map(|v| v["results"].clone());
                    times.push(tq.elapsed().as_secs_f64() * 1000.0);
                    out.push(r);
                    s.background_work();
                }
            }
        }
        drop(s);
        (out, times, t.elapsed().as_secs_f64())
    };
    let (reference, ref_times, secs) = rounds(pool(None));
    eprintln!("none: {secs:.2} s");
    for l in list.split(',').map(str::trim).filter(|l| !l.is_empty()) {
        let limit = usize::try_from(bicraft_mem::parse_bytes(l).unwrap().unwrap()).unwrap();
        let p = pool(Some(limit));
        let (got, times, secs) = rounds(p);
        let mut diffs = 0;
        let mut errors = 0;
        for (k, (a, g)) in reference.iter().zip(&got).enumerate() {
            let id = &queries[(k / repeat) % queries.len()].0;
            match (a, g) {
                (Ok(a), Ok(g)) if a == g => {}
                (Ok(_), Err(e)) => {
                    errors += 1;
                    eprintln!(
                        "  {l} {id} (round {}): error {e}",
                        k / (repeat * queries.len()) + 1
                    );
                }
                _ => {
                    diffs += 1;
                    eprintln!(
                        "  {l} {id} (round {}): DIFFERENT",
                        k / (repeat * queries.len()) + 1
                    );
                }
            }
        }
        let slow: Vec<String> = times
            .iter()
            .zip(&ref_times)
            .enumerate()
            .filter(|(_, (t, r))| **t > **r * 2.0 && **t > 20.0)
            .map(|(k, (t, r))| format!("{} {:.0}→{:.0} ms", queries[(k / repeat) % queries.len()].0, r, t))
            .collect();
        let st = p.stats();
        eprintln!(
            "{l}: {secs:.2} s, {diffs} differences, {errors} errors; peak reserved {} of {}, refused {}, \
             evictions {}; left reserved {}; slower than twice: {slow:?}",
            bicraft_mem::size_text(st.peak),
            bicraft_mem::size_text(limit),
            st.refused,
            st.evictions,
            st.used
        );
        assert_eq!(diffs, 0, "limit {l}: answers differ");
        assert!(
            st.peak <= limit,
            "limit {l}: peak reservation {} over it",
            st.peak
        );
        assert_eq!(st.used, 0, "limit {l}: reservations left");
    }
}

/// Under a limit, codes unpacked one per row are reserved first; refused, a column gives no codes
/// and the evaluation is failed (never an answer computed from them).
#[test]
fn unpacked_codes_are_reserved_and_a_refusal_fails_the_evaluation() {
    let dir = TempDir::new("limit-unpack");
    let path = dir.join("limit.pbix");
    build(&path);
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": path }));
    let part = super::stored_backup(&s).expect("a DataModel").into_owned();
    let store = super::store::VertipaqStore::open(&part, super::MAX_BACKUP_BYTES).unwrap();
    let gate = |p: &'static bicraft_mem::Pool| super::store::UnpackGate {
        pool: p,
        class: bicraft_mem::Class::Kept,
        held: Vec::new(),
        refused: None,
    };
    // refused: no codes, the refusal recorded, nothing reserved
    let col = store.column("Sales", "Amount", usize::MAX).unwrap();
    assert!(col.codes.as_packed().is_some(), "a compressed column");
    let tiny = pool(Some(1024));
    super::store::UNPACK.with(|u| *u.borrow_mut() = Some(gate(tiny)));
    assert!(col.codes.plain().is_empty());
    let g = super::store::UNPACK.with(|u| u.borrow_mut().take()).unwrap();
    assert!(
        g.refused
            .as_deref()
            .unwrap()
            .contains("unpacking a column's codes"),
        "{:?}",
        g.refused
    );
    assert!(g.held.is_empty());
    assert_eq!(tiny.stats().used, 0);
    // admitted: the codes, reserved until the evaluation ends
    let roomy = pool(Some(1 << 30));
    super::store::UNPACK.with(|u| *u.borrow_mut() = Some(gate(roomy)));
    assert_eq!(col.codes.plain().len(), 6000);
    let g = super::store::UNPACK.with(|u| u.borrow_mut().take()).unwrap();
    assert!(g.refused.is_none());
    assert_eq!(roomy.stats().used, 6000 * 4);
    drop(g);
    assert_eq!(roomy.stats().used, 0);
    // outside an evaluation under a limit: unpacked as before
    assert_eq!(col.codes.plain().len(), 6000);
}
