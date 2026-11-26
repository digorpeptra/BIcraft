//! The worker pool: results in order, panics caught, the background pool kept apart, and the
//! micro-benches behind its design (`docs/storage-engine.md`, "Worker pool"):
//! `cargo test --release -p bicraft-scan pool_ -- --ignored --nocapture`.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use std::time::{Duration, Instant};

use super::*;
use crate::*;

fn thread_name() -> String {
    std::thread::current().name().unwrap_or("").to_string()
}

#[test]
fn pool_maps_in_order_on_any_number_of_tasks() {
    for n in [0usize, 1, 2, 3, 7, 64, 300] {
        let got = map(n, |i| i * 3).unwrap();
        assert_eq!(got, (0..n).map(|i| i * 3).collect::<Vec<_>>());
    }
    let mut items: Vec<u64> = vec![0; 1000];
    assert!(each(items.chunks_mut(37).collect(), |i, c: &mut [u64]| c
        .fill(i as u64)));
    for (r, v) in items.iter().enumerate() {
        assert_eq!(*v, (r / 37) as u64);
    }
}

#[test]
fn pool_task_panic_is_a_failure_not_a_crash() {
    for n in [1usize, 2, 9] {
        assert!(!run(n, |i| {
            if i == n - 1 {
                panic!("task {i} fails on purpose");
            }
        }));
        assert!(
            map(n, |i| if i == 0 {
                panic!("first task fails on purpose")
            } else {
                i
            })
            .is_none()
        );
    }
    // the pool's workers survive: the next loops run in full
    for _ in 0..3 {
        assert_eq!(map(16, |i| i).unwrap(), (0..16).collect::<Vec<_>>());
    }
}

#[test]
fn pool_nested_loops_complete() {
    let got = map(6, |i| map(5, |j| i * 10 + j).unwrap().iter().sum::<usize>()).unwrap();
    let want: Vec<usize> = (0..6).map(|i| (0..5).map(|j| i * 10 + j).sum()).collect();
    assert_eq!(got, want);
}

#[test]
fn pool_background_loops_stay_off_the_foreground_workers() {
    let names = std::thread::spawn(|| {
        set_background(Some(3));
        assert!(is_background());
        // nested loops of a background task stay in the background pool too
        let names = map(12, |_| {
            let mut n = map(4, |_| thread_name()).unwrap();
            n.push(thread_name());
            std::thread::sleep(Duration::from_millis(2));
            n
        })
        .unwrap();
        set_background(None);
        names.concat()
    })
    .join()
    .unwrap();
    assert!(names.iter().any(|n| n.starts_with("bicraft-bg-")), "{names:?}");
    assert!(!names.iter().any(|n| n.starts_with("bicraft-scan-")), "{names:?}");
    let fg = std::thread::spawn(|| {
        map(12, |_| {
            std::thread::sleep(Duration::from_millis(2));
            thread_name()
        })
        .unwrap()
    })
    .join()
    .unwrap();
    assert!(!fg.iter().any(|n| n.starts_with("bicraft-bg-")), "{fg:?}");
}

/// Waking the pool against starting scoped threads, per loop of `n` empty tasks.
#[test]
#[ignore]
fn pool_start_cost() {
    let reps = 200;
    for n in [1usize, 2, 4, 10, 20] {
        let t = Instant::now();
        for _ in 0..reps {
            std::thread::scope(|sc| {
                for _ in 0..n {
                    sc.spawn(|| std::hint::black_box(0));
                }
            });
        }
        let scoped = t.elapsed().as_secs_f64() / reps as f64;
        run(n, |i| {
            std::hint::black_box(i);
        });
        let t = Instant::now();
        for _ in 0..reps {
            run(n, |i| {
                std::hint::black_box(i);
            });
        }
        let pooled = t.elapsed().as_secs_f64() / reps as f64;
        println!(
            "tasks {n:>2}  scoped threads {:>8.1} µs  pool {:>6.1} µs",
            scoped * 1e6,
            pooled * 1e6
        );
    }
}

/// A foreground scan's latency alone, while a background loop of scans runs on the background
/// pool (the aggregation build's arrangement), and while the same loop shares the foreground
/// pool (what a single pool would do).
#[test]
#[ignore]
fn pool_foreground_latency_under_background_load() {
    let n = 32_000_000usize;
    let codes: Vec<u32> = (0..n as u32)
        .map(|i| i.wrapping_mul(2_654_435_761) % 1000)
        .collect();
    let lut: Vec<u32> = (0..1000u32).map(|c| c % 10).collect();
    let per_code: Vec<Option<i64>> = (0..1000).map(Some).collect();
    let sum = |codes: &[u32]| {
        let all = Selection::all(codes.len());
        let key = Key::new(vec![KeyPart {
            codes: Col::Plain(codes),
            lut: Some(&lut),
            card: 10,
        }])
        .unwrap();
        scan(
            &all,
            &key,
            Some(Col::Plain(codes)),
            threads_for(codes.len(), 240),
            || IntSums::new(10, &per_code),
        )
        .unwrap()
    };
    let fg: &[u32] = &codes[..1_000_000];
    let measure = |label: &str| {
        let mut t: Vec<f64> = (0..60)
            .map(|_| {
                let s = Instant::now();
                std::hint::black_box(sum(fg));
                std::thread::sleep(Duration::from_millis(3));
                s.elapsed().as_secs_f64() - 0.003
            })
            .collect();
        t.sort_by(f64::total_cmp);
        println!(
            "{label:<44} median {:>6.2} ms  p90 {:>6.2} ms",
            t[t.len() / 2] * 1e3,
            t[t.len() * 9 / 10] * 1e3
        );
    };
    sum(fg);
    measure("foreground alone");
    for background in [true, false] {
        let stop = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|sc| {
            sc.spawn(|| {
                let quarter = (configured_threads() / 4).max(1);
                set_threads(Some(quarter));
                if background {
                    set_background(Some(quarter));
                }
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    std::hint::black_box(sum(&codes));
                }
            });
            std::thread::sleep(Duration::from_millis(50));
            measure(if background {
                "with a build on the background pool"
            } else {
                "with a build sharing the foreground pool"
            });
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        });
    }
}

#[test]
fn pool_rows_per_thread_follow_the_wake_cost() {
    let r = crate::rows_per_thread();
    if bicraft_env::var("BICRAFT_SCAN_ROWS_PER_THREAD").is_err() {
        assert!(r.is_power_of_two() && ((1 << 16)..=(1 << 19)).contains(&r), "{r}");
    }
    println!("wake round trip {} ns, {r} rows per worker", wake_round_trip_ns());
}
