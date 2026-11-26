//! Power Query conversions per culture (`Number.ToText`, `Date.ToText`, `Text.From`,
//! `Number.FromText`, `Date.FromText`, `Currency.From`, …) against what Power BI Desktop
//! returned for the same probes (recorded with the maintainers' development oracles in
//! crates/locale/tests/oracle/m_expected.json). The agreement rates are in
//! docs/formats/locales.md.

use bicraft_data::{CellValue, SourceOptions, m};
use serde_json::Value;

fn load(name: &str) -> Value {
    let path = format!("{}/../locale/tests/oracle/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn text(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Evaluates `rows` (an M list of `{id, value}` pairs) with the model culture `culture`.
fn run(rows: &str, culture: &str) -> Vec<(String, Option<String>)> {
    let expr = format!(
        "let R = {{{rows}}} in #table(type table [id = text, v = text], \
         List.Transform(R, (r) => {{r{{0}}, try r{{1}} otherwise \"#ERR\"}}))"
    );
    let opts = SourceOptions {
        culture: Some(culture.to_string()),
        ..SourceOptions::default()
    };
    let t = m::load_m_with(&expr, &opts).unwrap_or_else(|e| panic!("{e}"));
    let col = |i: usize| t.columns().get(i).map(|c| c.values.clone()).unwrap_or_default();
    let (ids, vs) = (col(0), col(1));
    ids.into_iter()
        .zip(vs)
        .map(|(id, v)| {
            let id = match id {
                CellValue::Text(s) => s,
                other => format!("{other:?}"),
            };
            let v = match v {
                CellValue::Text(s) => Some(s),
                CellValue::Null => None,
                other => Some(format!("{other:?}")),
            };
            (id, v)
        })
        .collect()
}

#[test]
fn power_query_conversions_match_desktop() {
    let cases = load("m_cases.json");
    let expected = load("m_expected.json");
    let cultures: Vec<&str> = expected["cultures"]
        .as_array()
        .unwrap_or_else(|| panic!("cultures"))
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let probes = cases["probes"].as_array().unwrap_or_else(|| panic!("probes"));
    let model = cases["model_culture"].as_str().unwrap_or("en-US");
    let (mut ok, mut n) = (0usize, 0usize);
    let mut lines = Vec::new();
    let mut worst = (1.0f64, String::new());
    for (ci, culture) in cultures.iter().enumerate() {
        let rows: Vec<String> = probes
            .iter()
            .filter_map(|p| {
                let id = p.get(0)?.as_str()?;
                let body = p.get(1)?.as_str()?;
                Some(format!("{{{}, let c = {} in {body}}}", text(id), text(culture)))
            })
            .collect();
        let (mut cok, mut cn) = (0usize, 0usize);
        for (id, got) in run(&rows.join(", "), model) {
            let Some(want) = expected["results"][&id].get(ci).and_then(Value::as_str) else {
                continue;
            };
            cn += 1;
            if got.as_deref() == Some(want) {
                cok += 1;
            } else if lines.len() < 600 {
                lines.push(format!("{culture} {id}: got {got:?}, Desktop {want:?}"));
            }
        }
        let rate = cok as f64 / cn.max(1) as f64;
        if rate < worst.0 {
            worst = (rate, culture.to_string());
        }
        println!("{culture}: {cok}/{cn} ({:.1}%)", rate * 100.0);
        ok += cok;
        n += cn;
    }
    // Conversions without a culture argument use the model culture.
    let defaults = cases["defaults"].as_array().unwrap_or_else(|| panic!("defaults"));
    let rows: Vec<String> = defaults
        .iter()
        .filter_map(|p| {
            Some(format!(
                "{{{}, {}}}",
                text(p.get(0)?.as_str()?),
                p.get(1)?.as_str()?
            ))
        })
        .collect();
    for (id, got) in run(&rows.join(", "), model) {
        if let Some(want) = expected["defaults"][&id].as_str() {
            n += 1;
            if got.as_deref() == Some(want) {
                ok += 1;
            } else {
                lines.push(format!("model {model} {id}: got {got:?}, Desktop {want:?}"));
            }
        }
    }
    for l in &lines {
        println!("{l}");
    }
    let rate = ok as f64 / n.max(1) as f64;
    println!(
        "Power Query: {ok}/{n} ({:.2}%); worst {} {:.1}%",
        rate * 100.0,
        worst.1,
        worst.0 * 100.0
    );
    assert!(rate >= 0.995, "Power Query agreement {rate}");
}
