//! The linguistic collation against the order the engine of Power BI Desktop 2.158 gave to the
//! same probe words in a model of each culture (recorded with the development oracles into
//! crates/locale/tests/oracle/expected.json, `sort`).
//!
//! Strings that differ only at the third level (`æ`/`ae`, `ß`/`ss`) are ties the query order
//! does not settle (the `H$` order of tests/system.rs does); such pairs are not counted.

use bicraft_vertipaq::Collation;
use serde_json::Value;
use std::cmp::Ordering;

#[test]
fn sort_order_matches_the_engine() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../locale/tests/oracle/expected.json"
    );
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let json: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}"));
    let sorts = json["sort"].as_object().unwrap_or_else(|| panic!("sort"));
    let (mut pairs, mut wrong) = (0usize, 0usize);
    let mut exact = 0usize;
    for (culture, words) in sorts {
        let words: Vec<&str> = words
            .as_array()
            .unwrap_or_else(|| panic!("{culture}"))
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let c = Collation::for_culture(culture);
        let mut bad = Vec::new();
        for w in words.windows(2) {
            if let [a, b] = w {
                let (ka, kb) = (c.sort_key(a), c.sort_key(b));
                if ka.primary == kb.primary && ka.secondary == kb.secondary {
                    continue;
                }
                pairs += 1;
                if c.compare(a, b) == Ordering::Greater && !c.equivalent(a, b) {
                    bad.push(format!("{a} > {b}"));
                }
            }
        }
        if bad.is_empty() {
            exact += 1;
        } else {
            println!(
                "{culture}: {} pair(s) out of order: {}",
                bad.len(),
                bad.join(", ")
            );
        }
        wrong += bad.len();
    }
    let rate = 1.0 - wrong as f64 / pairs.max(1) as f64;
    println!(
        "{exact}/{} cultures in the engine's order; adjacent pairs {}/{pairs} ({:.2}%)",
        sorts.len(),
        pairs - wrong,
        rate * 100.0
    );
    assert!(rate >= 0.999, "collation agreement {rate}");
}
