//! Read-only commands on a shared session (`Session::execute_shared`, `shared.rs`) and the
//! preview forks behind concurrent evaluations (`preview_fork.rs`): runs overlap, answers are
//! the answers of serialized runs, merge-backs never bring a stale entry, budgets and cancel
//! flags stay per query.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde_json::{Value, json};

use super::memory_budget_tests::{QUERIES, answers, build, memory, run};
use super::test_gate::{Gate, remove};
use crate::Session;
use crate::tempdir::TempDir;

fn open(dir: &TempDir) -> Session {
    let path = dir.join("c.pbix");
    if !path.exists() {
        build(&path);
    }
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    s
}

fn shared(s: &Session, q: &str) -> Result<Value, String> {
    s.execute_shared("dax.evaluate", &json!({ "query": q }), None)
        .map(|v| v["results"].clone())
}

#[test]
fn a_session_can_be_shared_between_threads() {
    fn sync_send<T: Sync + Send>() {}
    sync_send::<Session>();
}

/// Two queries stand at one gate at the same time: the second ran on a fork while the first
/// held the main state; both answer what serialized runs answer.
#[test]
fn reads_overlap_and_answer_as_serialized() {
    let dir = TempDir::new("conc-overlap");
    let mut s = open(&dir);
    let reference = answers(&mut s);
    let gate = Gate::new("overlap", 2);
    let (q0, q1) = (QUERIES[1], QUERIES[5]);
    let (a, b) = std::thread::scope(|sc| {
        let a = sc.spawn(|| shared(&s, &format!("{q0} // gate:overlap")));
        let b = sc.spawn(|| shared(&s, &format!("{q1} // gate:overlap")));
        (a.join().unwrap(), b.join().unwrap())
    });
    assert!(!gate.timed_out(), "the two queries did not run at the same time");
    assert_eq!(gate.arrived(), 2);
    remove(&gate);
    assert_eq!(a.unwrap(), reference[1]);
    assert_eq!(b.unwrap(), reference[5]);
    assert!(s.preview.fork_counters().0 >= 1, "one ran on a fork");
    // every query, many at once, many times: the same answers
    for round in 0..3 {
        let got: Vec<Value> = std::thread::scope(|sc| {
            let hs: Vec<_> = QUERIES
                .iter()
                .map(|q| {
                    let s = &s;
                    sc.spawn(move || shared(s, q).unwrap())
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(got, reference, "round {round}");
    }
}

/// Cold: nothing is open in the main state yet; a concurrent query waits for it instead of
/// opening the backup a second time, and the answers are the same.
#[test]
fn cold_parallel_reads_answer_as_serialized() {
    let dir = TempDir::new("conc-cold");
    let mut reference_session = open(&dir);
    let reference = answers(&mut reference_session);
    let s = open(&dir);
    let got: Vec<Value> = std::thread::scope(|sc| {
        let hs: Vec<_> = QUERIES
            .iter()
            .map(|q| {
                let s = &s;
                sc.spawn(move || shared(s, q).unwrap())
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(got, reference);
}

/// A fork's merge-back is applied for its own revision only: one waiting while the session
/// changes is dropped, and the next query answers from the new state.
#[test]
fn a_merge_back_across_a_change_is_dropped() {
    let dir = TempDir::new("conc-stale");
    let mut s = open(&dir);
    // the main state has the store open, not the columns of the query below
    run(&mut s, "dax.evaluate", json!({"query": QUERIES[0]}));
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "M", "expression": "SUM(Sales[Qty])"}),
    );
    let hold = Gate::new("hold", usize::MAX);
    let before = std::thread::scope(|sc| {
        // the main state is held by a query standing at the gate
        let holder = sc.spawn(|| shared(&s, &format!("{} // gate:hold", QUERIES[0])));
        while hold.arrived() == 0 {
            std::thread::yield_now();
        }
        // a fork reads Sales[Qty] and Sales[Color]; the main state is busy: its merge-back waits
        let r = shared(
            &s,
            "EVALUATE SUMMARIZECOLUMNS(Sales[Color], \"m\", [M]) ORDER BY Sales[Color]",
        );
        hold.open();
        holder.join().unwrap().unwrap();
        r.unwrap()
    });
    remove(&hold);
    let (forks, merged, stale) = s.preview.fork_counters();
    assert_eq!((forks, merged, stale), (1, 0, 0), "the merge-back waits");
    // the session changes before any evaluation takes the main state
    run(
        &mut s,
        "measure.set",
        json!({"name": "M", "expression": "SUM(Sales[Amount])"}),
    );
    let after = run(
        &mut s,
        "dax.evaluate",
        json!({"query": "EVALUATE SUMMARIZECOLUMNS(Sales[Color], \"m\", [M]) ORDER BY Sales[Color]"}),
    )["results"]
        .clone();
    let (_, merged, stale) = s.preview.fork_counters();
    assert_eq!((merged, stale), (0, 1), "the waiting merge-back was dropped");
    assert_ne!(after, before["results"], "the new measure answers");
    let expected = run(
        &mut s,
        "dax.evaluate",
        json!({"query": "EVALUATE SUMMARIZECOLUMNS(Sales[Color], \"m\", SUM(Sales[Amount])) ORDER BY Sales[Color]"}),
    )["results"]
        .clone();
    assert_eq!(after, expected);
}

/// A merge-back for the same revision is applied: the main state then has the fork's columns.
#[test]
fn a_merge_back_fills_the_main_state() {
    let dir = TempDir::new("conc-merge");
    let mut s = open(&dir);
    run(&mut s, "dax.evaluate", json!({"query": QUERIES[0]}));
    let hold = Gate::new("hold2", usize::MAX);
    let q = "EVALUATE SUMMARIZECOLUMNS(Sales[Color], \"q\", SUM(Sales[Qty])) ORDER BY Sales[Color]";
    std::thread::scope(|sc| {
        let holder = sc.spawn(|| shared(&s, &format!("{} // gate:hold2", QUERIES[0])));
        while hold.arrived() == 0 {
            std::thread::yield_now();
        }
        shared(&s, q).unwrap();
        hold.open();
        holder.join().unwrap().unwrap();
    });
    remove(&hold);
    let before = s.preview.lock().columns.len();
    // the next evaluation applies it
    run(&mut s, "dax.evaluate", json!({"query": QUERIES[0]}));
    let (_, merged, stale) = s.preview.fork_counters();
    assert_eq!((merged, stale), (1, 0));
    let after = s.preview.lock().columns.len();
    assert!(after > before, "the fork's columns are kept: {before} -> {after}");
}

/// The cancel flag of one shared run stops that run only.
#[test]
fn cancelling_one_read_leaves_the_others() {
    let dir = TempDir::new("conc-cancel");
    let mut s = open(&dir);
    let reference = answers(&mut s);
    let gate = Gate::new("cancel", 2);
    let raised = Arc::new(AtomicBool::new(true));
    let (a, b) = std::thread::scope(|sc| {
        let a = sc.spawn(|| {
            s.execute_shared(
                "dax.evaluate",
                &json!({"query": format!("{} // gate:cancel", QUERIES[5])}),
                Some(Arc::clone(&raised)),
            )
        });
        let b = sc.spawn(|| shared(&s, &format!("{} // gate:cancel", QUERIES[5])));
        (a.join().unwrap(), b.join().unwrap())
    });
    remove(&gate);
    assert!(a.unwrap_err().contains(crate::CANCELLED));
    assert_eq!(b.unwrap(), reference[5]);
    assert!(!s.is_cancelled(), "the session's own flag is untouched");
}

/// Under a memory budget, parallel queries answer as without one, and what the session keeps
/// afterwards is within the budget; a query budget refuses each query on its own.
#[test]
fn budgets_hold_under_parallel_queries() {
    let dir = TempDir::new("conc-budget");
    let mut free = open(&dir);
    let reference = answers(&mut free);
    let largest = memory(&mut free, json!({}))["largest_column_bytes"]
        .as_u64()
        .unwrap();
    let mut s = open(&dir);
    let budget = largest + largest / 4;
    memory(&mut s, json!({"budget": budget}));
    run(&mut s, "dax.evaluate", json!({"query": QUERIES[0]}));
    for _ in 0..2 {
        let got: Vec<Value> = std::thread::scope(|sc| {
            let hs: Vec<_> = QUERIES
                .iter()
                .map(|q| {
                    let s = &s;
                    sc.spawn(move || shared(s, q).unwrap())
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(got, reference);
        // the next evaluation applies the merge-backs, then holds the budget
        run(&mut s, "dax.evaluate", json!({"query": QUERIES[0]}));
        let m = memory(&mut s, json!({}));
        assert!(m["column_bytes"].as_u64().unwrap() <= budget, "{m}");
    }
    // a query budget of one byte: every query is refused, each by its own accounting
    memory(&mut s, json!({"query_budget": 1}));
    let gate = Gate::new("qbudget", 2);
    let (a, b) = std::thread::scope(|sc| {
        let a = sc.spawn(|| shared(&s, &format!("{} // gate:qbudget", QUERIES[2])));
        let b = sc.spawn(|| shared(&s, &format!("{} // gate:qbudget", QUERIES[7])));
        (a.join().unwrap(), b.join().unwrap())
    });
    remove(&gate);
    assert!(a.is_err() && b.is_err(), "{a:?} {b:?}");
    assert!(
        !gate.timed_out(),
        "arrived {} forks {:?}",
        gate.arrived(),
        s.preview.fork_counters()
    );
}

/// Commands a shared session refuses run nowhere; the others answer as `execute` does.
#[test]
fn shared_runs_match_execute() {
    let dir = TempDir::new("conc-match");
    let mut s = open(&dir);
    for id in [
        "model.summary",
        "file.inspect",
        "report.summary",
        "model.validate",
    ] {
        let a = s.execute_shared(id, &json!({}), None);
        let b = s.execute(id, &json!({}));
        assert_eq!(a.is_ok(), b.is_ok(), "{id}");
        if id != "file.inspect" {
            assert_eq!(a.ok(), b.ok(), "{id}");
        }
    }
    let r = s.execute_shared(
        "measure.add",
        &json!({"table": "Sales", "name": "X", "expression": "1"}),
        None,
    );
    assert!(r.is_err());
    assert!(
        s.model
            .as_ref()
            .unwrap()
            .tables
            .iter()
            .all(|t| t.measures.iter().all(|m| m.name != "X"))
    );
    let revision = s.revision;
    s.execute_shared("dax.evaluate", &json!({"query": QUERIES[0]}), None)
        .unwrap();
    assert_eq!(s.revision, revision, "a shared run never bumps the revision");
    assert_eq!(s.last_command_now().unwrap().id, "dax.evaluate");
    s.execute("model.summary", &json!({})).unwrap();
    assert_eq!(s.last_command_now().unwrap().id, "model.summary");
}

/// Concurrent evaluations missing the same stored column read it once: the others take it.
#[test]
fn a_column_missed_at_once_is_read_once() {
    let dir = TempDir::new("conc-dedup");
    let mut s = open(&dir);
    // the backup open, the fact's columns not read yet
    run(
        &mut s,
        "dax.evaluate",
        json!({"query": "EVALUATE ROW(\"n\", COUNTROWS(VALUES(Product[Category])))"}),
    );
    let reference = {
        let mut r = open(&dir);
        run(&mut r, "dax.evaluate", json!({"query": QUERIES[0]}))["results"].clone()
    };
    let gate = Gate::new("dedup", 3);
    let got: Vec<Value> = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..3)
            .map(|_| {
                let s = &s;
                sc.spawn(move || shared(s, &format!("{} // gate:dedup", QUERIES[0])).unwrap())
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(!gate.timed_out());
    remove(&gate);
    for g in &got {
        assert_eq!(g, &reference);
    }
    // three columns (Amount, Store, and the rows), each read once: two takes per column at least
    assert!(s.preview.dedup_count() >= 2, "{}", s.preview.dedup_count());
}
