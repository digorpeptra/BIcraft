//! The decomposition tree oracle as tests (docs/render.md, "Decomposition tree"), on the
//! committed corpus `testsupport/decomp_oracle/corpus.json` (runs on Linux: no Desktop involved).
//!
//! The report `oracle-gen decomp` generates is built in-process, its snapshot taken, and each
//! tree compared with what Desktop drew: the root value, the levels shown and their fields, the
//! members in each level's window (labels and values in order), the selected path, the AI
//! levels, and the renderer's layout (window, bar centres and lengths, level positions).

use bicraft_engine::Session;
use serde_json::{Value, json};

use super::decomp;
use super::read_json;
use super::tempdir::TempDir;

fn corpus() -> Option<Value> {
    let p = decomp::corpus_dir().join("corpus.json");
    if !p.is_file() {
        eprintln!(
            "note: {} is missing; decomposition tree oracle skipped",
            p.display()
        );
        return None;
    }
    read_json(&p).ok()
}

/// The generated report with its data, as `oracle-gen decomp` + `bicraft run` build it.
fn report_session(dir: &TempDir) -> Session {
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::write(dir.path().join("sales.csv"), decomp::sales_csv()).unwrap();
    let mut s = Session::new();
    for c in decomp::cmds1(&d).as_array().unwrap() {
        s.execute(c["command"].as_str().unwrap(), &c["params"])
            .unwrap_or_else(|e| panic!("{}: {e}", c["command"]));
    }
    decomp::patch_report(&dir.path().join("proj").join("Decomp.Report")).unwrap();
    let mut s = Session::new();
    for c in decomp::cmds2(&d).as_array().unwrap() {
        s.execute(c["command"].as_str().unwrap(), &c["params"])
            .unwrap_or_else(|e| panic!("{}: {e}", c["command"]));
    }
    let mut s2 = Session::new();
    s2.execute("file.open", &json!({"path": dir.path().join("decomp.pbix")}))
        .unwrap();
    s2
}

/// Each tree visual of the snapshot with its page: (display name, page name, visual).
fn trees(s: &Session) -> Vec<(String, String, Value)> {
    let limits = bicraft_engine::SnapshotLimits {
        data_budget_ms: 600_000,
        ..Default::default()
    };
    let snap = s.snapshot_with(&limits);
    let mut out = Vec::new();
    for p in snap["report"]["pages"].as_array().into_iter().flatten() {
        for v in p["visuals"].as_array().into_iter().flatten() {
            if v["type"] == "decompositionTreeVisual" {
                out.push((
                    p["display_name"].as_str().unwrap_or_default().to_string(),
                    p["name"].as_str().unwrap_or_default().to_string(),
                    v.clone(),
                ));
            }
        }
    }
    out
}

/// Desktop's shown number agrees with ours at the precision it showed ("593 333", "46,5%",
/// "593,3K").
fn close(ours: f64, shown: f64, text: &str) -> bool {
    let digits = text.trim_end_matches(['%', 'K']);
    let decimals = digits
        .rsplit_once([',', '.'])
        .map_or(0, |(_, t)| t.chars().filter(char::is_ascii_digit).count());
    let unit = if text.ends_with('%') {
        0.01
    } else if text.ends_with('K') {
        1000.0
    } else {
        1.0
    };
    let tol = unit * 10f64.powi(-i32::try_from(decimals).unwrap_or(0)) * 0.5 + 1e-9;
    (ours - shown).abs() <= tol
}

/// A member as Desktop labels it: the text, `null` for a blank.
fn label(m: &Value) -> Value {
    match m {
        Value::String(s) if s.is_empty() => Value::Null,
        Value::Number(n) => json!(n.to_string()),
        other => other.clone(),
    }
}

/// The differences between Desktop's drawing of one tree and ours (`body`: the visual's inner
/// box in report pixels).
fn compare(desk: &Value, v: &Value, body: (f32, f32)) -> Vec<String> {
    let mut diffs = Vec::new();
    let d = &v["data"]["decomp"];
    if !d.is_object() {
        return vec![format!("no data.decomp: {}", v["data"])];
    }
    let cols = v["data"]["columns"].as_array().cloned().unwrap_or_default();
    let dl = desk["levels"].as_array().cloned().unwrap_or_default();
    let ours = d["levels"].as_array().cloned().unwrap_or_default();
    let root = dl
        .first()
        .and_then(|l| l["nodes"].get(0))
        .cloned()
        .unwrap_or(Value::Null);
    let rv = d["root"].as_f64().unwrap_or(f64::NAN);
    if !close(
        rv,
        root["value"].as_f64().unwrap_or(f64::NAN),
        root["value_text"].as_str().unwrap_or(""),
    ) {
        diffs.push(format!("root {rv} vs Desktop {}", root["value_text"]));
    }
    if dl.len() != ours.len() + 1 {
        diffs.push(format!(
            "{} levels vs Desktop {}",
            ours.len(),
            dl.len().saturating_sub(1)
        ));
    }
    let lay = bicraft_render::decomp::layout(d, body.0, body.1);
    for (li, dk) in dl.iter().enumerate() {
        let Some(lb) = lay.levels.get(li) else {
            diffs.push(format!("level {li}: not laid out"));
            continue;
        };
        let dx = dk["x"].as_f64().unwrap_or(0.0) as f32;
        if (lb.x - dx).abs() > 1.5 {
            diffs.push(format!("level {li}: x {:.1} vs Desktop {dx:.1}", lb.x));
        }
        let shown = dk["nodes"].as_array().cloned().unwrap_or_default();
        if lb.nodes.len() != shown.len() {
            diffs.push(format!(
                "level {li}: {} bars vs Desktop {}",
                lb.nodes.len(),
                shown.len()
            ));
        }
        if lb.up != (dk["up"].as_bool().unwrap_or(false)) && dk.get("up").is_some() {
            diffs.push(format!("level {li}: scroll up {}", lb.up));
        }
        let geometry = |j: usize,
                        name: &Value,
                        nb: &bicraft_render::decomp::NodeBox,
                        sn: &Value,
                        diffs: &mut Vec<String>| {
            let desk_centre = sn["y"].as_f64().unwrap_or(0.0) as f32 + 4.86;
            if (nb.y - desk_centre).abs() > 1.5 {
                diffs.push(format!(
                    "level {li} bar {j} {name}: centre {:.1} vs Desktop {desk_centre:.1}",
                    nb.y
                ));
            }
            let db = sn["bar"].as_f64().unwrap_or(0.0) as f32;
            if (nb.bar - db).abs() > 1.5 {
                diffs.push(format!(
                    "level {li} bar {j} {name}: length {:.1} vs Desktop {db:.1}",
                    nb.bar
                ));
            }
        };
        if li == 0 {
            if let (Some(nb), Some(sn)) = (lb.nodes.first(), shown.first()) {
                geometry(0, &json!("root"), nb, sn, &mut diffs);
            }
            continue;
        }
        let Some(ol) = ours.get(li - 1) else { continue };
        let col = ol["column"].as_u64().unwrap_or(0) as usize;
        let field = cols.get(col).map(|c| c["name"].clone()).unwrap_or(Value::Null);
        if dk["field"] != field {
            diffs.push(format!("level {li}: field {field} vs Desktop {}", dk["field"]));
        }
        if dk["ai"].as_bool().unwrap_or(false) != ol["ai"].is_string() {
            diffs.push(format!("level {li}: ai {} vs Desktop {}", ol["ai"], dk["ai"]));
        }
        let nodes = ol["nodes"].as_array().cloned().unwrap_or_default();
        for (j, (sn, nb)) in shown.iter().zip(&lb.nodes).enumerate() {
            let Some(n) = nodes.get(nb.index) else { continue };
            if label(&n[0]) != sn["label"] {
                diffs.push(format!("level {li} bar {j}: {} vs Desktop {}", n[0], sn["label"]));
                continue;
            }
            let ov = n[1].as_f64().unwrap_or(f64::NAN);
            if !close(
                ov,
                sn["value"].as_f64().unwrap_or(f64::NAN),
                sn["value_text"].as_str().unwrap_or(""),
            ) {
                diffs.push(format!(
                    "level {li} {}: {ov} vs Desktop {}",
                    n[0], sn["value_text"]
                ));
            }
            if nb.on_path != sn["on_path"].as_bool().unwrap_or(false) {
                diffs.push(format!("level {li} {}: on path {} vs Desktop", n[0], nb.on_path));
            }
            geometry(j, &n[0], nb, sn, &mut diffs);
        }
    }
    diffs
}

#[test]
fn trees_match_desktop() {
    let Some(c) = corpus() else { return };
    let dir = TempDir::new("xtask-decomp-oracle");
    let s = report_session(&dir);
    let ours = trees(&s);
    let specs = decomp::trees();
    let mut bad = Vec::new();
    let mut n = 0usize;
    for t in c["trees"].as_array().unwrap() {
        let page = t["page"].as_str().unwrap();
        n += 1;
        let Some((_, _, v)) = ours.iter().find(|(p, _, _)| p == page) else {
            bad.push(format!("{page}: no tree in the snapshot"));
            continue;
        };
        // the inner box: the visual less Desktop's 5 px padding
        let (w, h) = specs
            .iter()
            .find(|s| s.page == page)
            .map_or((1000, 400), |s| s.size);
        let body = ((w - 10) as f32, (h - 10) as f32);
        for d in compare(t, v, body) {
            bad.push(format!("{page}: {d}"));
        }
    }
    assert!(n >= 14, "corpus has {n} trees");
    assert!(bad.is_empty(), "{} differences:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn reader_actions_change_the_tree() {
    let dir = TempDir::new("xtask-decomp-actions");
    let mut s = report_session(&dir);
    let tree = |s: &Session, name: &str| -> (String, Value) {
        trees(s)
            .into_iter()
            .find(|(p, _, _)| p == name)
            .map(|(_, pn, v)| (pn, v))
            .unwrap()
    };
    let (pname, _) = tree(&s, "D03");
    let sel_of = |l: &Value| -> Value {
        let k = l["selected"].as_u64().unwrap() as usize;
        l["nodes"][k][0].clone()
    };
    // D03: Region > Category under South; move the path to North
    let r = s
        .execute(
            "view.interact",
            &json!({"action": "decomp_path", "page": pname, "visual": "d03", "level": 1, "value": "North"}),
        )
        .unwrap();
    assert_eq!(r["changed"], true, "{r}");
    let (_, v) = tree(&s, "D03");
    let l = v["data"]["decomp"]["levels"].as_array().unwrap().clone();
    assert_eq!(sel_of(&l[0]), "North");
    assert_eq!(l.len(), 2);
    // a High value split under the first Category member
    let cat = l[1]["nodes"][0][0].clone();
    s.execute(
        "view.interact",
        &json!({"action": "decomp_expand", "page": pname, "visual": "d03", "level": 2, "value": cat,
                "ai": "high", "path": ["North"], "columns": [1, 2]}),
    )
    .unwrap();
    let (_, v) = tree(&s, "D03");
    let l = v["data"]["decomp"]["levels"].as_array().unwrap().clone();
    assert_eq!(l.len(), 3);
    assert_eq!(l[2]["ai"], "high");
    assert_eq!(l[2]["auto"], true);
    assert_eq!(sel_of(&l[1]), cat);
    // the cross of level 2 removes it and the AI level below
    s.execute(
        "view.interact",
        &json!({"action": "decomp_collapse", "page": pname, "visual": "d03", "level": 2}),
    )
    .unwrap();
    let (_, v) = tree(&s, "D03");
    assert_eq!(v["data"]["decomp"]["levels"].as_array().unwrap().len(), 1);
    // bad actions are refused
    for bad in [
        json!({"action": "decomp_path", "page": pname, "visual": "d03", "level": 9, "value": "x"}),
        json!({"action": "decomp_expand", "page": pname, "visual": "d03", "level": 1, "value": "North", "column": 0}),
        json!({"action": "decomp_expand", "page": pname, "visual": "d03", "level": 1, "value": [1], "column": 2}),
        json!({"action": "decomp_expand", "page": pname, "visual": "d03", "level": 0, "ai": "sideways"}),
    ] {
        assert!(s.execute("view.interact", &bad).is_err(), "{bad}");
    }
    // a node on the path selects: a data point of the tree, as Desktop's cross-filter
    let r = s
        .execute(
            "view.interact",
            &json!({"action": "select", "page": pname, "visual": "d03", "point": [{"c": 1, "v": "North"}]}),
        )
        .unwrap();
    assert_eq!(r["changed"], true);
    // reset: the stored tree again
    s.execute("view.interact", &json!({"action": "reset"})).unwrap();
    let (_, v) = tree(&s, "D03");
    assert_eq!(sel_of(&v["data"]["decomp"]["levels"][0]), "South");
}
