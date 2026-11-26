//! Timings of the calendar hot paths (ROADMAP M11.9): DAX date functions over a few million
//! rows, M date functions, and reading dates from CSV text. Ignored by default; run with
//! `cargo test -p bicraft-engine --test calendar_bench -- --ignored --nocapture` (the test profile
//! is optimized) and compare the medians before and after a change to the calendar code.

use std::time::Instant;

use bicraft_data::csv::{CsvOptions, read_csv};
use bicraft_data::{Culture, DataType};
use bicraft_engine::Session;
use serde_json::json;

fn median_ms(rounds: usize, mut f: impl FnMut()) -> f64 {
    let mut t: Vec<f64> = (0..rounds)
        .map(|_| {
            let s = Instant::now();
            f();
            s.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    t.sort_by(f64::total_cmp);
    t[t.len() / 2]
}

#[test]
#[ignore = "timing run"]
fn calendar_hot_paths() {
    let mut s = Session::new();
    s.execute("model.new", &json!({"name": "Bench"})).unwrap();
    let dax = [
        (
            "dax YEAR+MONTH+DAY ×1M",
            "EVALUATE ROW(\"s\", SUMX(GENERATESERIES(1, 1000000), YEAR([Value] * 2.9) + MONTH([Value] * 2.9) + DAY([Value] * 2.9)))",
        ),
        (
            "dax EOMONTH+EDATE ×1M",
            "EVALUATE ROW(\"s\", SUMX(GENERATESERIES(1, 1000000), EOMONTH([Value] * 2.9, 1) + EDATE([Value] * 2.9, -13)))",
        ),
        (
            "dax DATEDIFF MONTH+YEAR ×1M",
            "EVALUATE ROW(\"s\", SUMX(GENERATESERIES(1, 1000000), DATEDIFF([Value], [Value] * 2.9, MONTH) + DATEDIFF([Value], [Value] * 2.9, YEAR)))",
        ),
        (
            "dax WEEKNUM ×1M",
            "EVALUATE ROW(\"s\", SUMX(GENERATESERIES(1, 1000000), WEEKNUM([Value] * 2.9, 21) + WEEKNUM([Value] * 2.9, 2)))",
        ),
    ];
    for (name, q) in dax {
        let ms = median_ms(5, || {
            s.execute("dax.evaluate", &json!({"query": q, "timeout_ms": 600_000}))
                .unwrap();
        });
        println!("{name}: {ms:.1} ms");
    }
    let m = "#table({\"v\"}, {{List.Sum(List.Transform({1..300000}, each let d = Date.From(_ * 9) in \
             Date.Year(d) + Date.Month(Date.AddMonths(d, 1)) + Date.DayOfYear(Date.EndOfMonth(d))))}})";
    let ms = median_ms(5, || {
        bicraft_data::m::load_m(m, &bicraft_data::Limits::default()).unwrap();
    });
    println!("m Date.Year/AddMonths/DayOfYear ×300k: {ms:.1} ms");
    // CSV dates: ISO and a culture's short date
    let mut iso = String::from("d\n");
    let mut fr = String::from("d\n");
    for i in 0..1_000_000u32 {
        let (y, m, d) = (1900 + i % 200, 1 + i % 12, 1 + i % 28);
        iso.push_str(&format!("{y:04}-{m:02}-{d:02}\n"));
        fr.push_str(&format!("{d:02}/{m:02}/{y:04}\n"));
    }
    for (name, text, culture) in [
        ("csv ISO dates ×1M", &iso, Culture::INVARIANT),
        ("csv fr-FR dates ×1M", &fr, Culture::from_name("fr-FR")),
    ] {
        let mut o = CsvOptions::new();
        o.culture = culture;
        o.column_types = vec![("d".into(), DataType::DateTime)];
        let ms = median_ms(5, || {
            read_csv(text.as_bytes(), &o).unwrap();
        });
        println!("{name}: {ms:.1} ms");
    }
}
