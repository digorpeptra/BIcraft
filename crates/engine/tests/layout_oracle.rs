//! Legacy layout → PBIR against Power BI Desktop's own conversion (G13).
//!
//! Each oracle case holds `legacy/` (a generated legacy PBIP project, `cargo xtask layout-oracle`)
//! and `desktop/definition/` (the PBIR folder Desktop wrote when it upgraded that project on save,
//! `tools/layout-oracle/run.ps1`). Two cases are committed under `tests/fixtures/layout-oracle/`
//! (Desktop 2.159.883.0) and always run; the eight local ones under
//! `%BICRAFT_TESTDATA%\layout-oracle\` (Desktop 2.158) run when that folder exists. Each legacy
//! project is converted with `file.convert` (`report_format: "pbir"`) and every PBIR document is
//! compared with Desktop's, as JSON values (object key order ignored). The negative tests check
//! that perturbed output is reported. Method and Desktop's rules: `docs/formats/legacy-layout-bb.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use serde_json::{Value, json};

/// Every `*.json` document under `dir`, keyed by its path relative to `dir` (forward slashes).
fn documents(dir: &Path) -> BTreeMap<String, Value> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Value>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else if p.extension().is_some_and(|x| x == "json") {
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                let text = std::fs::read_to_string(&p).unwrap();
                let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
                    .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                out.insert(rel, v);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// A `$schema` URL without its version segment.
fn schema_name(url: &str) -> &str {
    url.trim_end_matches("/schema.json")
        .rsplit_once('/')
        .map_or(url, |(name, _version)| name)
}

/// Known differences that are not conversion errors (`docs/formats/legacy-layout-bb.md`):
/// - `$schema` versions: Desktop 2.158 writes visualContainer 2.13.0 and
///   visualContainerMobileState 2.8.0, bicraft the newest published ones; only the schema
///   name is compared;
/// - a bookmark's `explorationState.version`: Desktop recomputes it from the features the state
///   uses, bicraft copies the legacy one (which Desktop computed the same way when it wrote it);
fn ignored(path: &str, ours: &Value, desktop: &Value) -> bool {
    if path.ends_with(".$schema")
        && let (Some(a), Some(b)) = (ours.as_str(), desktop.as_str())
    {
        return schema_name(a) == schema_name(b);
    }
    if path == ".explorationState.version" {
        return true;
    }
    // - the order of a resource package's items (Desktop re-adds the custom theme last).
    if path.starts_with(".resourcePackages[")
        && path.ends_with("].items")
        && let (Some(a), Some(b)) = (ours.as_array(), desktop.as_array())
    {
        let sorted = |v: &[Value]| {
            let mut s: Vec<String> = v.iter().map(Value::to_string).collect();
            s.sort();
            s
        };
        return sorted(a) == sorted(b);
    }
    false
}

/// The JSON paths where `ours` and `desktop` differ (object key order ignored).
fn diff(path: &str, ours: &Value, desktop: &Value, out: &mut Vec<String>) {
    if ignored(path, ours, desktop) {
        return;
    }
    match (ours, desktop) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, va) in a {
                match b.get(k) {
                    Some(vb) => diff(&format!("{path}.{k}"), va, vb, out),
                    None => out.push(format!("{path}.{k}: only ours = {}", short(va))),
                }
            }
            for (k, vb) in b {
                if !a.contains_key(k) {
                    out.push(format!("{path}.{k}: only Desktop = {}", short(vb)));
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (va, vb)) in a.iter().zip(b).enumerate() {
                diff(&format!("{path}[{i}]"), va, vb, out);
            }
        }
        (Value::Number(a), Value::Number(b)) if a.as_f64() == b.as_f64() => {}
        _ if ours == desktop => {}
        _ => out.push(format!(
            "{path}: ours = {} / Desktop = {}",
            short(ours),
            short(desktop)
        )),
    }
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.chars().count() > 300 {
        format!("{}…", s.chars().take(300).collect::<String>())
    } else {
        s
    }
}

/// Converts one case and returns its differences with Desktop's PBIR.
fn compare_case(case_dir: &Path, scratch: &Path) -> Vec<String> {
    let legacy = case_dir.join("legacy");
    let pbip = std::fs::read_dir(&legacy)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "pbip"))
        .unwrap_or_else(|| panic!("{}: no .pbip", legacy.display()));
    let name = pbip.file_stem().unwrap().to_string_lossy().into_owned();
    let to = scratch.join(&name).join(format!("{name}.pbip"));
    let mut s = Session::new();
    s.execute(
        "file.convert",
        &json!({"from": pbip, "to": to, "report_format": "pbir", "overwrite": true}),
    )
    .unwrap_or_else(|e| panic!("{}: {e}", pbip.display()));
    let ours = documents(&scratch.join(&name).join(format!("{name}.Report/definition")));
    let desktop = documents(&case_dir.join("desktop/definition"));
    compare(&ours, &desktop)
}

/// Every difference between our PBIR documents and Desktop's, keyed by path.
fn compare(ours: &BTreeMap<String, Value>, desktop: &BTreeMap<String, Value>) -> Vec<String> {
    let mut out = Vec::new();
    for (rel, d) in desktop {
        match ours.get(rel) {
            Some(o) => {
                let mut diffs = Vec::new();
                diff("", o, d, &mut diffs);
                out.extend(diffs.into_iter().map(|x| format!("{rel}: {x}")));
            }
            None => out.push(format!("{rel}: only Desktop writes it")),
        }
    }
    for rel in ours.keys().filter(|r| !desktop.contains_key(*r)) {
        out.push(format!("{rel}: only ours"));
    }
    out
}

/// The cases (`<case>/legacy/`, `<case>/desktop/definition/`) under an oracle folder. A folder that
/// exists but holds no case is an error: the test must never pass having compared nothing.
fn cases_in(oracle: &Path) -> Vec<PathBuf> {
    let mut cases: Vec<PathBuf> = std::fs::read_dir(oracle)
        .unwrap_or_else(|e| panic!("{}: {e}", oracle.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("legacy").is_dir() && p.join("desktop/definition").is_dir())
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "{}: no oracle case", oracle.display());
    cases
}

/// Compares every case of `oracle`; returns the names of the cases that differ.
/// `older_desktop`: the pairs come from an older Desktop, whose `reportVersionAtImport` (its own
/// current versions for recent themes) differs from the current one's; those values are not compared.
fn run_oracle(oracle: &Path, scratch: &Path, older_desktop: bool) -> (usize, Vec<String>) {
    let cases = cases_in(oracle);
    let mut failures = Vec::new();
    for case in &cases {
        let name = case.file_name().unwrap().to_string_lossy().into_owned();
        let mut diffs = compare_case(case, &scratch.join(&name));
        if older_desktop {
            diffs.retain(|d| !d.contains(".reportVersionAtImport."));
        }
        eprintln!("{name}: {} differences", diffs.len());
        for d in &diffs {
            eprintln!("  {d}");
        }
        if !diffs.is_empty() {
            failures.push(name);
        }
    }
    (cases.len(), failures)
}

/// The committed pairs (`tests/fixtures/layout-oracle/`: legacy projects generated by
/// `cargo xtask layout-oracle`, converted by Desktop 2.159.883.0), always compared.
#[test]
fn committed_pairs_match_desktop() {
    let oracle = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/layout-oracle");
    let scratch =
        std::env::temp_dir().join(format!("bicraft-layout-oracle-committed-{}", std::process::id()));
    let (n, failures) = run_oracle(&oracle, &scratch, false);
    let _ = std::fs::remove_dir_all(&scratch);
    assert!(n >= 2, "{n} committed cases");
    assert!(failures.is_empty(), "differences with Desktop in {failures:?}");
}

/// The local pairs under `BICRAFT_TESTDATA` (more cases, Desktop 2.158), when available.
#[test]
fn local_pairs_match_desktop() {
    let Some(root) = bicraft_env::var_os("BICRAFT_TESTDATA").map(PathBuf::from) else {
        eprintln!(
            "note: BICRAFT_TESTDATA is not set; local layout-oracle pairs skipped (committed pairs still run)"
        );
        return;
    };
    let oracle = root.join("layout-oracle");
    if !oracle.is_dir() {
        eprintln!(
            "note: {} is missing (tools/layout-oracle/run.ps1); skipped",
            oracle.display()
        );
        return;
    }
    let scratch = std::env::temp_dir().join(format!("bicraft-layout-oracle-{}", std::process::id()));
    let (_, failures) = run_oracle(&oracle, &scratch, true);
    let _ = std::fs::remove_dir_all(&scratch);
    assert!(failures.is_empty(), "differences with Desktop in {failures:?}");
}

/// A small PBIR document set of our own: a page with a group and a member.
fn sample_docs() -> BTreeMap<String, Value> {
    let page = "definition/pages/p1";
    let mut m = BTreeMap::new();
    m.insert(
        "definition/report.json".to_string(),
        json!({"$schema": "https://x/report/3.3.0/schema.json", "settings": {"useEnhancedTooltips": false}}),
    );
    m.insert(
        format!("{page}/page.json"),
        json!({"name": "p1", "displayName": "P", "width": 1280, "height": 720}),
    );
    m.insert(
        format!("{page}/visuals/g1/visual.json"),
        json!({"name": "g1", "position": {"x": 10, "y": 20}, "visualGroup": {"displayName": "G"}}),
    );
    m.insert(
        format!("{page}/visuals/v1/visual.json"),
        json!({"name": "v1", "position": {"x": 1, "y": 2}, "parentGroupName": "g1",
               "visual": {"visualType": "card", "query": {"queryState": {"Values": {"projections": [
                   {"field": {"Measure": {"Expression": {"SourceRef": {"Entity": "T"}}, "Property": "M"}},
                    "queryRef": "T.M", "active": true}]}}}}}),
    );
    m
}

/// The comparison is sensitive: each kind of perturbation of one side is reported, and only the
/// documented tolerances (schema versions, key order) pass.
#[test]
fn the_comparison_reports_every_kind_of_difference() {
    let base = sample_docs();
    assert!(compare(&base, &base).is_empty());
    const V1: &str = "definition/pages/p1/visuals/v1/visual.json";
    const G1: &str = "definition/pages/p1/visuals/g1/visual.json";
    type Perturb = fn(&mut BTreeMap<String, Value>);
    let cases: [(&str, Perturb, &str); 9] = [
        (
            "moved",
            |m| m.get_mut(V1).unwrap()["position"]["x"] = json!(31),
            "position.x",
        ),
        (
            "integral vs fraction",
            |m| m.get_mut(V1).unwrap()["position"]["y"] = json!(2.5),
            "position.y",
        ),
        (
            "removed group",
            |m| {
                m.remove(G1);
            },
            "only Desktop",
        ),
        (
            "extra file",
            |m| {
                m.insert("definition/pages/p1/visuals/x/visual.json".into(), json!({}));
            },
            "only ours",
        ),
        (
            "dropped property",
            |m| {
                m.get_mut(V1)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove("parentGroupName");
            },
            "parentGroupName",
        ),
        (
            "added property",
            |m| {
                m.get_mut(V1).unwrap()["isHidden"] = json!(true);
            },
            "isHidden",
        ),
        (
            "deep value",
            |m| {
                m.get_mut(V1).unwrap()["visual"]["query"]["queryState"]["Values"]["projections"][0]["active"] =
                    json!(false);
            },
            "active",
        ),
        (
            "array length",
            |m| {
                let p = &mut m.get_mut(V1).unwrap()["visual"]["query"]["queryState"]["Values"]["projections"];
                let first = p[0].clone();
                p.as_array_mut().unwrap().push(first);
            },
            "projections",
        ),
        (
            "setting",
            |m| {
                m.get_mut("definition/report.json").unwrap()["settings"]["useEnhancedTooltips"] = json!(true);
            },
            "useEnhancedTooltips",
        ),
    ];
    for (what, perturb, needle) in cases {
        let mut ours = base.clone();
        perturb(&mut ours);
        let d = compare(&ours, &base);
        assert!(d.iter().any(|x| x.contains(needle)), "{what}: {d:?}");
        // And in the other direction.
        let d = compare(&base, &ours);
        assert!(!d.is_empty(), "{what} (reversed)");
    }
    // Tolerated: a different schema version, and key order.
    let mut ours = base.clone();
    ours.get_mut("definition/report.json").unwrap()["$schema"] = json!("https://x/report/3.1.0/schema.json");
    assert!(compare(&ours, &base).is_empty());
    let reordered: Value = serde_json::from_str(
        r#"{"position": {"y": 2, "x": 1}, "parentGroupName": "g1", "name": "v1", "visual": {"visualType": "card", "query": {"queryState": {"Values": {"projections": [{"active": true, "queryRef": "T.M", "field": {"Measure": {"Property": "M", "Expression": {"SourceRef": {"Entity": "T"}}}}}]}}}}}"#,
    )
    .unwrap();
    ours.insert(V1.into(), reordered);
    assert!(compare(&ours, &base).is_empty());
}

/// On real converted output (the committed `visuals` pair): perturbing our conversion is caught.
#[test]
fn perturbed_conversions_are_caught() {
    let case = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/layout-oracle/visuals");
    let scratch = std::env::temp_dir().join(format!("bicraft-layout-oracle-neg-{}", std::process::id()));
    assert!(
        compare_case(&case, &scratch).is_empty(),
        "the unperturbed case matches"
    );
    let report = scratch.join("visuals").join("visuals.Report").join("definition");
    let desktop = documents(&case.join("desktop/definition"));
    let ours = documents(&report);
    // One visual moved, one visual file removed, one group file added.
    let (rel, doc) = ours
        .iter()
        .find(|(k, v)| k.ends_with("visual.json") && v.get("position").is_some())
        .map(|(k, v)| (k.clone(), v.clone()))
        .unwrap();
    let mut moved = ours.clone();
    let mut d = doc.clone();
    d["position"]["x"] = json!(d["position"]["x"].as_f64().unwrap() + 1.0);
    moved.insert(rel.clone(), d);
    assert!(compare(&moved, &desktop).iter().any(|x| x.contains("position.x")));
    let mut removed = ours.clone();
    removed.remove(&rel);
    assert!(
        compare(&removed, &desktop)
            .iter()
            .any(|x| x.contains("only Desktop"))
    );
    let mut grouped = ours.clone();
    let page = rel.split("/visuals/").next().unwrap().to_string();
    grouped.insert(
        format!("{page}/visuals/g0/visual.json"),
        json!({"name": "g0", "position": {"x": 0, "y": 0}, "visualGroup": {"displayName": "G"}}),
    );
    assert!(
        compare(&grouped, &desktop)
            .iter()
            .any(|x| x.contains("only ours"))
    );
    let _ = std::fs::remove_dir_all(&scratch);
}
