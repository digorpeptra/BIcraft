//! The interaction oracle as tests (docs/interaction-oracle.md), on the committed corpus
//! `testsupport/interaction_oracle/corpus.json` (runs on Linux: no Desktop involved).
//!
//! - `observed_queries_match_the_dax_evaluator`: every DAX query Desktop sent during the
//!   recording, run by `dax.evaluate` on the same model, compared cell by cell with the engine's
//!   result (the measure of what bicraft's DAX evaluator already gets right for interaction
//!   queries: TREATAS filter tables, KEEPFILTERS highlights, ROLLUPADDISSUBTOTAL totals…).
//! - `file_states_match_the_viewer_data`: for the states that come from the files alone (slicer
//!   selections, sync groups, `visualInteractions` with slicers), the data bicraft's snapshot
//!   gives each visual compared with what Desktop displayed.
//! - `click_states_match_the_interactive_viewer`: the states reached by clicks, played on a
//!   session with `view.interact` (slicer items, data points, Ctrl, buttons, pages) and compared
//!   the same way, highlighted values included; slicer items by their ticks.

use std::collections::BTreeMap;

use bicraft_engine::Session;
use serde_json::{Value, json};

use super::interactions;
use super::interactions_collect::corpus_dir;
use super::read_json;
use super::tempdir::TempDir;

/// Queries that agree (a drop is a regression): every one since the DAX coverage work of
/// 2026-10-23 (`IGNORE()` and the line chart's `SAMPLEAXISWITHLOCALMINMAX`, 225 before it).
const QUERY_FLOOR: usize = 438;
/// File-state visuals that agreed when the corpus was recorded.
const FILE_STATE_FLOOR: usize = 171;

fn corpus() -> Option<Value> {
    let p = corpus_dir().join("corpus.json");
    if !p.is_file() {
        eprintln!("note: {} is missing; interaction oracle skipped", p.display());
        return None;
    }
    read_json(&p).ok()
}

/// A session on the oracle model with its data computed (the calculated tables), through a
/// PBIX saved by bicraft.
fn model_session(dir: &TempDir) -> Session {
    let bim = dir.path().join("model.bim");
    std::fs::write(&bim, serde_json::to_string(&interactions::model()).unwrap()).unwrap();
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": bim})).unwrap();
    let pbix = dir.path().join("m.pbix");
    s.execute("file.save_as", &json!({"path": pbix})).unwrap();
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": pbix})).unwrap();
    s
}

/// The generated report with its data, as `oracle-gen interactions` + `bicraft-cli run` build it.
fn report_session(dir: &TempDir) -> Session {
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::write(
        dir.path().join("model.bim"),
        serde_json::to_string(&interactions::model()).unwrap(),
    )
    .unwrap();
    let mut s = Session::new();
    for c in interactions::cmds1(&d).as_array().unwrap() {
        s.execute(c["command"].as_str().unwrap(), &c["params"])
            .unwrap_or_else(|e| panic!("{}: {e}", c["command"]));
    }
    interactions::patch_report(&dir.path().join("proj").join("Interactions.Report")).unwrap();
    let mut s = Session::new();
    for c in interactions::cmds2(&d).as_array().unwrap() {
        s.execute(c["command"].as_str().unwrap(), &c["params"])
            .unwrap_or_else(|e| panic!("{}: {e}", c["command"]));
    }
    let mut s2 = Session::new();
    s2.execute(
        "file.open",
        &json!({"path": dir.path().join("interactions.pbix")}),
    )
    .unwrap();
    // relative date slicers resolve against the clock: Desktop's, when the corpus was recorded
    if let Some(at) = corpus()
        .and_then(|c| c["recorded"].as_str().map(str::to_string))
        .and_then(|r| r.get(..19).map(str::to_string))
    {
        s2.execute("view.set_as_of", &json!({ "as_of": at })).unwrap();
    }
    drop(s);
    s2
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64()
}

fn same(a: &Value, b: &Value) -> bool {
    match (num(a), num(b)) {
        (Some(x), Some(y)) => (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0),
        _ => a == b,
    }
}

/// Compares `dax.evaluate`'s results with the engine's: same result count, column names and row
/// multisets (in order when the query has ORDER BY).
fn compare_results(ours: &Value, theirs: &Value, ordered: bool) -> Result<(), String> {
    let (ours, theirs) = (
        ours.as_array().ok_or("no results")?,
        theirs.as_array().ok_or("no results")?,
    );
    if ours.len() != theirs.len() {
        return Err(format!("{} results, engine {}", ours.len(), theirs.len()));
    }
    for (k, (o, t)) in ours.iter().zip(theirs).enumerate() {
        let names = |v: &Value| -> Vec<String> {
            v["columns"]
                .as_array()
                .map(|c| {
                    c.iter()
                        .map(|x| x["name"].as_str().unwrap_or("").to_string())
                        .collect()
                })
                .unwrap_or_default()
        };
        let cols = names(t);
        if names(o) != cols {
            return Err(format!("result {k}: columns {:?}, engine {cols:?}", names(o)));
        }
        let row = |r: &Value| -> Vec<Value> { cols.iter().map(|c| r[c.as_str()].clone()).collect() };
        let mut ro: Vec<Vec<Value>> = o["rows"].as_array().unwrap().iter().map(row).collect();
        let mut rt: Vec<Vec<Value>> = t["rows"].as_array().unwrap().iter().map(row).collect();
        if ro.len() != rt.len() {
            return Err(format!("result {k}: {} rows, engine {}", ro.len(), rt.len()));
        }
        if !ordered {
            let key = |r: &Vec<Value>| {
                serde_json::to_string(
                    &r.iter()
                        .map(|v| match v.as_f64() {
                            Some(x) => json!(format!("{x:.6e}")),
                            None => v.clone(),
                        })
                        .collect::<Vec<_>>(),
                )
                .unwrap()
            };
            ro.sort_by_key(key);
            rt.sort_by_key(key);
        }
        for (i, (a, b)) in ro.iter().zip(&rt).enumerate() {
            for (j, (x, y)) in a.iter().zip(b).enumerate() {
                if !same(x, y) {
                    return Err(format!("result {k} row {i} column {}: {x}, engine {y}", cols[j]));
                }
            }
        }
    }
    Ok(())
}

#[test]
fn observed_queries_match_the_dax_evaluator() {
    let Some(c) = corpus() else { return };
    let dir = TempDir::new("xtask-interaction-queries");
    let mut s = model_session(&dir);
    let (mut agree, mut refused, mut wrong) = (0usize, Vec::new(), Vec::new());
    let mut n = 0usize;
    let mut without_ignore = (0usize, 0usize);
    for q in c["queries"].as_array().unwrap() {
        let (Some(id), Some(dax)) = (q["id"].as_str(), q["dax"].as_str()) else {
            continue;
        };
        let Some(expected) = q.get("results") else {
            continue;
        };
        n += 1;
        match s.execute("dax.evaluate", &json!({"query": dax, "max_rows": 100_000})) {
            Err(e) => {
                refused.push(format!("{id}: {}", e.lines().next().unwrap_or_default()));
                // information only: the same query without its IGNORE() wrappers
                let plain = strip_ignore(dax);
                if plain != dax
                    && let Ok(got) = s.execute("dax.evaluate", &json!({"query": plain, "max_rows": 100_000}))
                {
                    match compare_results(&got["results"], expected, dax.contains("ORDER BY")) {
                        Ok(()) => without_ignore.0 += 1,
                        Err(_) => without_ignore.1 += 1,
                    }
                }
            }
            Ok(got) => match compare_results(&got["results"], expected, dax.contains("ORDER BY")) {
                Ok(()) => agree += 1,
                Err(d) => wrong.push(format!("{id}: {d}")),
            },
        }
    }
    println!(
        "interaction oracle, observed queries: {agree}/{n} agree, {} refused, {} different",
        refused.len(),
        wrong.len()
    );
    println!(
        "  refused queries evaluated without IGNORE() (information only): {} agree, {} differ",
        without_ignore.0, without_ignore.1
    );
    for r in refused.iter().take(20) {
        println!("  refused  {r}");
    }
    for w in wrong.iter().take(40) {
        println!("  wrong    {w}");
    }
    assert!(
        wrong.is_empty(),
        "observed queries whose result differs from the engine's:
{}",
        wrong.join(
            "
"
        )
    );
    assert!(
        agree >= QUERY_FLOOR,
        "observed queries: {agree} agree, floor {QUERY_FLOOR}"
    );
}

/// `dax` with every `IGNORE(x)` replaced by `x` (IGNORE only marks a SUMMARIZECOLUMNS
/// expression as not deciding which rows are kept).
fn strip_ignore(dax: &str) -> String {
    let mut out = String::new();
    let mut rest = dax;
    while let Some(p) = rest.find("IGNORE(") {
        out.push_str(&rest[..p]);
        let inner = &rest[p + 7..];
        let mut depth = 1usize;
        let mut end = None;
        for (i, ch) in inner.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(e) = end else {
            out.push_str(&rest[p..]);
            return out;
        };
        out.push_str(&strip_ignore(&inner[..e]));
        rest = &inner[e + 1..];
    }
    out.push_str(rest);
    out
}

/// The rows Desktop's visual received: the last result set of its query, without the total row
/// and the helper columns (`[IsGrandTotalRowTotal]`, intersection counts), as value vectors in
/// column order.
fn desktop_rows(results: &Value) -> Vec<Vec<Value>> {
    let Some(set) = results.as_array().and_then(|a| a.last()) else {
        return vec![];
    };
    let cols: Vec<String> = set["columns"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["name"].as_str().map(str::to_string))
        .collect();
    let keep: Vec<&String> = cols
        .iter()
        .filter(|c| !c.contains("IsGrandTotal") && !c.contains("IntersectionCount"))
        .collect();
    set["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| {
            cols.iter()
                .filter(|c| c.contains("IsGrandTotal"))
                .all(|c| r[c.as_str()] != Value::Bool(true))
        })
        .map(|r| keep.iter().map(|c| r[c.as_str()].clone()).collect())
        .collect()
}

fn sorted(mut rows: Vec<Vec<Value>>) -> Vec<Vec<Value>> {
    rows.sort_by_key(|r| {
        serde_json::to_string(
            &r.iter()
                .map(|v| match v.as_f64() {
                    Some(x) => json!(format!("{x:.6e}")),
                    None => v.clone(),
                })
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default()
    });
    rows
}

fn rows_equal(a: &[Vec<Value>], b: &[Vec<Value>]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q)))
}

/// Visual data of the snapshot, by visual name (computes every visual: no time budget).
fn snapshot_data(s: &Session) -> BTreeMap<String, Value> {
    let limits = bicraft_engine::SnapshotLimits {
        data_budget_ms: 600_000,
        ..Default::default()
    };
    let snap = s.snapshot_with(&limits);
    let mut out = BTreeMap::new();
    for p in snap["report"]["pages"].as_array().into_iter().flatten() {
        for v in p["visuals"].as_array().into_iter().flatten() {
            if let Some(n) = v["name"].as_str() {
                out.insert(n.to_string(), v["data"].clone());
            }
        }
    }
    out
}

#[test]
fn file_states_match_the_viewer_data() {
    let Some(c) = corpus() else { return };
    let queries: BTreeMap<&str, &Value> = c["queries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|q| Some((q["id"].as_str()?, &q["results"])))
        .collect();
    let dir = TempDir::new("xtask-interaction-files");
    let s = report_session(&dir);
    let data = snapshot_data(&s);
    let (mut agree, mut n) = (0usize, 0usize);
    let mut diffs = Vec::new();
    for st in c["states"].as_array().unwrap() {
        if !st["actions"].as_array().is_some_and(Vec::is_empty) {
            continue;
        }
        for v in st["visuals"].as_array().into_iter().flatten() {
            let (Some(name), Some(q)) = (v["name"].as_str(), v["query"].as_str()) else {
                continue;
            };
            let ty = v["type"].as_str().unwrap_or_default();
            if ty.contains("licer") {
                continue; // slicer items: compared once the viewer exposes them
            }
            let Some(results) = queries.get(q) else {
                continue;
            };
            n += 1;
            let want = sorted(desktop_rows(results));
            let got = data.get(name).cloned().unwrap_or(Value::Null);
            if got["status"] != "ok" {
                diffs.push(format!(
                    "{} {name}: viewer data {} ({})",
                    st["id"].as_str().unwrap_or_default(),
                    got["status"],
                    got["reason"].as_str().unwrap_or_default()
                ));
                continue;
            }
            let rows: Vec<Vec<Value>> = got["rows"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|r| r.as_array().cloned().unwrap_or_default())
                .collect();
            let rows = sorted(rows);
            if rows_equal(&rows, &want) {
                agree += 1;
            } else {
                diffs.push(format!(
                    "{} {name}: viewer {}, Desktop {}",
                    st["id"].as_str().unwrap_or_default(),
                    Value::from(rows.iter().map(|r| Value::from(r.clone())).collect::<Vec<_>>()),
                    Value::from(want.iter().map(|r| Value::from(r.clone())).collect::<Vec<_>>())
                ));
            }
        }
    }
    println!("interaction oracle, file states: {agree}/{n} visuals agree with Desktop");
    for d in diffs.iter().take(60) {
        println!("  {d}");
    }
    assert!(
        agree >= FILE_STATE_FLOOR,
        "file states: {agree} agree, floor {FILE_STATE_FLOOR}"
    );
}

/// The snapshot's visuals by name (no time budget).
fn snapshot_visuals(s: &Session) -> BTreeMap<String, Value> {
    let limits = bicraft_engine::SnapshotLimits {
        data_budget_ms: 600_000,
        ..Default::default()
    };
    let snap = s.snapshot_with(&limits);
    let mut out = BTreeMap::new();
    for p in snap["report"]["pages"].as_array().into_iter().flatten() {
        for v in p["visuals"].as_array().into_iter().flatten() {
            if let Some(n) = v["name"].as_str() {
                out.insert(n.to_string(), v.clone());
            }
        }
    }
    out
}

/// The report page holding the scenario page `id`'s visuals (`I16`: the page of `i16_*`).
fn page_of(s: &Session, id: &str) -> Option<String> {
    let prefix = format!("{}_", id.to_ascii_lowercase());
    let r = s.report.as_ref()?;
    r.pages
        .iter()
        .find(|p| p.visuals.iter().any(|v| v.name.starts_with(&prefix)))
        .map(|p| p.name.clone())
}

/// A clicked value (the corpus keeps the text of the mark or item) typed like the visual's
/// first column field (`3` for an integer month).
fn typed(visual: &Value, text: &str) -> (usize, Value) {
    let fields = visual["fields"].as_array().cloned().unwrap_or_default();
    let c = fields.iter().position(|f| f["kind"] == "column").unwrap_or(0);
    let ty = visual["data"]["columns"][c]["data_type"]
        .as_str()
        .unwrap_or_default();
    let v = if matches!(ty, "int64" | "double" | "decimal") {
        text.parse::<f64>().map_or(json!(text), |x| {
            if x.fract() == 0.0 {
                json!(x as i64)
            } else {
                json!(x)
            }
        })
    } else {
        json!(text)
    };
    (c, v)
}

/// What the viewer shows a visual received: its rows, each followed by its highlighted values
/// when the selection highlights it (Desktop's query returns both in one row, rule 11).
fn viewer_rows(v: &Value) -> Option<Vec<Vec<Value>>> {
    let d = &v["data"];
    if d["status"] != "ok" {
        return None;
    }
    let kinds: Vec<bool> = d["columns"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| c["kind"] == "column")
        .collect();
    let hl = d["highlights"].as_array();
    Some(
        d["rows"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, r)| {
                let mut row = r.as_array().cloned().unwrap_or_default();
                if let Some(h) = hl.and_then(|h| h.get(i)).and_then(Value::as_array) {
                    row.extend(h.iter().zip(&kinds).filter(|(_, g)| !**g).map(|(x, _)| x.clone()));
                }
                row
            })
            .collect(),
    )
}

/// Click-state visuals that agreed when the viewer's interactions were aligned on the corpus
/// (2026-10-23); a drop is a regression.
const CLICK_STATE_FLOOR: usize = 180;
/// Slicer ticks of the click states that agreed (same date).
const CLICK_SLICER_FLOOR: usize = 12;

#[test]
fn click_states_match_the_interactive_viewer() {
    let Some(c) = corpus() else { return };
    let queries: BTreeMap<&str, &Value> = c["queries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|q| Some((q["id"].as_str()?, &q["results"])))
        .collect();
    let dir = TempDir::new("xtask-interaction-clicks");
    let mut s = report_session(&dir);
    let (mut agree, mut n, mut sl_agree, mut sl_n, mut states) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut diffs = Vec::new();
    let mut scenario = String::new();
    for st in c["states"].as_array().unwrap() {
        let actions = st["actions"].as_array().cloned().unwrap_or_default();
        if actions.is_empty() {
            continue;
        }
        let id = st["id"].as_str().unwrap_or_default();
        let sc = id.split('.').next().unwrap_or_default().to_string();
        if sc != scenario {
            // each scenario starts from the files (the corpus records them one after the other)
            s.execute("view.interact", &json!({"action": "reset"})).unwrap();
            scenario = sc;
        }
        states += 1;
        let page = page_of(&s, st["page"].as_str().unwrap_or_default()).unwrap();
        for a in &actions {
            let before = snapshot_visuals(&s);
            let ctrl = a["ctrl"].as_bool().unwrap_or(false);
            let name = a["visual"].as_str().unwrap_or_default();
            let params = match a["do"].as_str().unwrap_or_default() {
                "click" => {
                    let (col, v) = typed(&before[name], a["point"].as_str().unwrap_or_default());
                    json!({"action": "select", "page": page, "visual": name,
                           "point": [{"c": col, "v": v}], "ctrl": ctrl})
                }
                "item" => {
                    let (_, v) = typed(&before[name], a["item"].as_str().unwrap_or_default());
                    json!({"action": "slicer_toggle", "page": page, "visual": name, "value": v, "ctrl": ctrl})
                }
                "button" => json!({"action": "button", "page": page, "visual": name}),
                "page" => {
                    let target = page_of(&s, a["page"].as_str().unwrap_or_default()).unwrap();
                    json!({"action": "page", "page": target})
                }
                other => panic!("{id}: unknown action {other}"),
            };
            if let Err(e) = s.execute("view.interact", &params) {
                diffs.push(format!("{id}: {params} refused: {e}"));
            }
        }
        let shown = snapshot_visuals(&s);
        for v in st["visuals"].as_array().into_iter().flatten() {
            let Some(name) = v["name"].as_str() else {
                continue;
            };
            let ty = v["type"].as_str().unwrap_or_default();
            let got = shown.get(name).cloned().unwrap_or(Value::Null);
            if ty.contains("licer") {
                // the items Desktop shows ticked
                let Some(items) = v["display"]["items"].as_array() else {
                    continue;
                };
                sl_n += 1;
                let mut want: Vec<String> = items
                    .iter()
                    .filter(|i| i["selected"] == true && i.get("text").is_some())
                    .filter_map(|i| i["text"].as_str().map(str::to_lowercase))
                    .collect();
                let mut ours: Vec<String> = got["slicer"]["selected"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|x| x.as_str().map_or_else(|| x.to_string(), str::to_lowercase))
                    .collect();
                want.sort();
                ours.sort();
                if want == ours {
                    sl_agree += 1;
                } else {
                    diffs.push(format!("{id} {name}: viewer ticks {ours:?}, Desktop {want:?}"));
                }
                continue;
            }
            let Some(q) = v["query"].as_str() else {
                continue;
            };
            let Some(results) = queries.get(q) else {
                continue;
            };
            n += 1;
            let mut want = desktop_rows(results);
            // rule 13: after the last selected point is clicked again Desktop redraws its first
            // data from its cache (nothing fresh), so the visual keeps the attribution of its
            // highlight query while it draws no highlight: the drawn values are the full ones
            let drawn_hl = v["display"]["points"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|p| p.get("highlight").is_some() || p.get("ratio").is_some());
            if v["fresh"] == false && !drawn_hl && got["data"]["highlights"].is_null() {
                let width = got["data"]["columns"].as_array().map_or(0, Vec::len);
                for r in &mut want {
                    r.truncate(width.max(1));
                }
            }
            let want = sorted(want);
            match viewer_rows(&got) {
                None => diffs.push(format!(
                    "{id} {name} ({q}): viewer data {} ({})",
                    got["data"]["status"],
                    got["data"]["reason"].as_str().unwrap_or_default()
                )),
                Some(rows) => {
                    let rows = sorted(rows);
                    if rows_equal(&rows, &want) {
                        agree += 1;
                    } else {
                        diffs.push(format!(
                            "{id} {name} ({q}, {}): viewer {}, Desktop {}",
                            got["interaction"]["mode"].as_str().unwrap_or("-"),
                            Value::from(rows.iter().map(|r| Value::from(r.clone())).collect::<Vec<_>>()),
                            Value::from(want.iter().map(|r| Value::from(r.clone())).collect::<Vec<_>>())
                        ));
                    }
                }
            }
        }
    }
    println!(
        "interaction oracle, click states ({states}): {agree}/{n} visuals agree with Desktop, \
         slicer ticks {sl_agree}/{sl_n}"
    );
    for d in diffs.iter().take(80) {
        println!("  {d}");
    }
    assert_eq!(states, 20, "click states in the corpus");
    assert!(
        agree >= CLICK_STATE_FLOOR,
        "click states: {agree} agree, floor {CLICK_STATE_FLOOR}"
    );
    assert!(
        sl_agree >= CLICK_SLICER_FLOOR,
        "slicer ticks: {sl_agree} agree, floor {CLICK_SLICER_FLOOR}"
    );
}
