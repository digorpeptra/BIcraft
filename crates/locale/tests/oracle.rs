//! Compares `FORMAT` and `VALUE` with what the engine of Power BI Desktop returned for the corpus
//! of tools/locale-oracle/gen_cases.mjs (tests/oracle/cases.json → tests/oracle/expected.json),
//! culture by culture. The agreement rates are recorded in docs/formats/locales.md.

use bicraft_locale::{Locale, parse, vb};
use serde_json::Value as Json;

fn load(name: &str) -> Json {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/oracle/");
    let text = std::fs::read_to_string(format!("{path}{name}")).unwrap_or_else(|e| panic!("{name}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// The value of a corpus expression: a number, or `DATE(y,m,d)` / `TIME(h,m,s)` terms added up
/// (a date when one of them is there).
fn eval(expr: &str) -> vb::Value<'static> {
    if let Ok(x) = expr.parse::<f64>() {
        return vb::Value::Number(x);
    }
    let mut total = 0.0;
    for term in expr.split('+') {
        let (f, args) = term.split_once('(').unwrap_or_else(|| panic!("{expr}"));
        let n: Vec<i64> = args
            .trim_end_matches(')')
            .split(',')
            .map(|a| a.trim().parse().unwrap_or_else(|_| panic!("{expr}")))
            .collect();
        total += match (f, n.as_slice()) {
            ("DATE", [y, m, d]) => {
                let days = bicraft_locale::datefmt::days_from_civil(*y, *m as u32, *d as u32);
                (days + 25_569) as f64
            }
            ("TIME", [h, m, s]) => (h * 3600 + m * 60 + s) as f64 / 86_400.0,
            _ => panic!("{expr}"),
        };
    }
    vb::Value::Date(total)
}

#[test]
fn format_matches_the_engine() {
    let cases = load("cases.json");
    let expected = load("expected.json");
    let cultures: Vec<&str> = expected["cultures"]
        .as_array()
        .unwrap_or_else(|| panic!("cultures"))
        .iter()
        .filter_map(Json::as_str)
        .collect();
    let mut total = (0usize, 0usize);
    let mut lines = Vec::new();
    let mut worst = 1.0f64;
    for (ci, culture) in cultures.iter().enumerate() {
        let loc = Locale::get(culture);
        assert_eq!(loc.name, *culture, "the table lacks {culture}");
        let (mut ok, mut n) = (0usize, 0usize);
        for case in cases["format"].as_array().unwrap_or_else(|| panic!("format")) {
            let id = case["id"].as_str().unwrap_or_default();
            // 123456789012345678 is an int64 the engine formats exactly; bicraft's evaluator
            // holds numbers as doubles (2^53 and beyond lose digits).
            if id.starts_with("n12_") {
                continue;
            }
            let want = expected["format"][id][ci].as_str().unwrap_or_default();
            if want == "#ERR" {
                continue;
            }
            let fmt = case["format"].as_str().unwrap_or_default();
            let got = vb::format(eval(case["value"].as_str().unwrap_or_default()), fmt, loc);
            n += 1;
            if got.as_deref() == Some(want) {
                ok += 1;
            } else if lines.len() < 400 {
                lines.push(format!("{culture} {id} {fmt:?}: got {got:?}, engine {want:?}"));
            }
        }
        let rate = ok as f64 / n.max(1) as f64;
        worst = worst.min(rate);
        println!("{culture}: {ok}/{n} ({:.1}%)", rate * 100.0);
        total.0 += ok;
        total.1 += n;
    }
    for l in &lines {
        println!("{l}");
    }
    let rate = total.0 as f64 / total.1.max(1) as f64;
    println!("FORMAT: {}/{} ({:.2}%)", total.0, total.1, rate * 100.0);
    assert!(rate >= 0.999, "FORMAT agreement {rate}");
    assert!(worst >= 0.99, "worst culture {worst}");
}

#[test]
fn value_matches_the_engine() {
    let expected = load("expected.json");
    let cultures: Vec<&str> = expected["cultures"]
        .as_array()
        .unwrap_or_else(|| panic!("cultures"))
        .iter()
        .filter_map(Json::as_str)
        .collect();
    let (mut ok, mut n) = (0usize, 0usize);
    let mut lines = Vec::new();
    for (text, results) in expected["value"].as_object().unwrap_or_else(|| panic!("value")) {
        for (ci, culture) in cultures.iter().enumerate() {
            let want = results.get(ci).and_then(Json::as_f64);
            let got = parse::value(text, Locale::get(culture));
            n += 1;
            let same = match (got, want) {
                (Some(a), Some(b)) => (a - b).abs() <= 1e-9 * b.abs().max(1.0),
                (None, None) => true,
                _ => false,
            };
            if same {
                ok += 1;
            } else {
                lines.push(format!("{culture} {text:?}: got {got:?}, engine {want:?}"));
            }
        }
    }
    for l in &lines {
        println!("{l}");
    }
    let rate = ok as f64 / n.max(1) as f64;
    println!("VALUE: {ok}/{n} ({:.2}%)", rate * 100.0);
    assert!(rate >= 0.96, "VALUE agreement {rate}");
}
