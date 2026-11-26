//! `oracle-gen decomp`: the decomposition tree oracle (docs/render.md, "Decomposition tree").
//! What does Power BI Desktop draw for a decomposition tree whose expansion state comes from the
//! file: the root, the levels it shows (pinned levels, their selected path in `root.children`,
//! AI levels with `AIInformation.method` = `MaxSplit` / `MinSplit`), the values and their order,
//! blanks, the bars kept per level, sorting, slicer and visual filters, formatting options.
//!
//! One tree per page, so each starts from the state its files describe. The expansion states
//! have the shapes Desktop 2.159 saved after the same actions in its UI (expanding a node with
//! a field, "High value", "Low value", selecting a node).
//!
//! ```text
//! cargo xtask oracle-gen decomp DIR           -> DIR/sales.csv, cmds1.json, cmds2.json, plan.json
//! bicraft run --file DIR/cmds1.json          -> DIR/proj/Decomp.pbip
//! cargo xtask oracle-gen decomp DIR patch     -> expansion states, options, filters
//! bicraft run --file DIR/cmds2.json          -> DIR/decomp.pbix
//! xtask/decomposition-tree-oracle/record.ps1 -Dir DIR     -> DIR/readings.json (Desktop: every tree's drawing)
//! cargo xtask oracle-gen decomp DIR collect   -> testsupport/decomp_oracle/corpus.json
//! ```
//! Culture en-US; Desktop formats in its own locale (the corpus keeps the parsed numbers).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::{Cmds, join, read_json, write_json};

pub const REGIONS: [&str; 5] = ["North", "South", "East", "West", "Central"];
pub const CATEGORIES: [&str; 3] = ["Bikes", "Clothing", "Accessories"];
pub const CHANNELS: [&str; 2] = ["Online", "Store"];
pub const SEGMENTS: [&str; 3] = ["Retail", "Corporate", ""];

/// `Sales`: 90 rows (region × category × channel × segment), 14 products, no two equal sums
/// on the paths the scenarios open (the AI splits then have one answer).
pub fn sales_csv() -> String {
    let mut s = String::from("Region,Category,Channel,Segment,Product,Amount,Cost\n");
    let mut k: i64 = 0;
    for (ri, r) in (0i64..).zip(REGIONS) {
        for (ci, c) in (0i64..).zip(CATEGORIES) {
            for (hi, h) in (0i64..).zip(CHANNELS) {
                for (si, sg) in (0i64..).zip(SEGMENTS) {
                    k += 1;
                    let p = (k * 5) % 14;
                    let mut amt = (1000 * (5 - ri) * (10 + ci * 6) / 10) * (10 + hi * 3) / 10;
                    amt += 37 * (si + 1) * (ri + 1) + 13 * p;
                    if ri == 1 && ci == 2 {
                        amt += 9000;
                    }
                    if ri == 2 && si == 2 {
                        amt += 2500;
                    }
                    let cost = amt * (40 + 7 * ((ri + 2 * ci + hi + si) % 5)) / 100;
                    let _ = writeln!(s, "{r},{c},{h},{sg},P{p:02},{amt},{cost}");
                }
            }
        }
    }
    s
}

fn lit(v: &str) -> Value {
    json!({"Literal": {"Value": v}})
}

fn e(v: &str) -> Value {
    json!({"expr": lit(v)})
}

fn key(col: &str) -> Value {
    json!({"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": col}})
}

/// One level of the expansion state (Desktop's shapes): `pinned` levels are shown, in order;
/// `ai` is `MaxSplit` / `MinSplit`; `keyed` levels have a member on the selected path.
fn level(col: &str, pinned: bool, ai: Option<&str>, keyed: bool) -> Value {
    let mut l = json!({"queryRefs": [format!("Sales.{col}")], "isCollapsed": true});
    if keyed {
        l["identityKeys"] = json!([key(col)]);
    }
    if pinned {
        l["isPinned"] = json!(true);
    }
    if let Some(m) = ai {
        l["AIInformation"] = json!({"method": m, "disabled": false});
    }
    l
}

/// The selected path as nested `root.children` (`None` = a blank member).
fn path(values: &[Option<&str>]) -> Value {
    let mut node: Option<Value> = None;
    for v in values.iter().rev() {
        let id = match v {
            Some(t) => lit(&format!("'{}'", t.replace('\'', "''"))),
            None => lit("null"),
        };
        let mut n = json!({"identityValues": [id], "isToggled": true});
        if let Some(c) = node.take() {
            n["children"] = json!([c]);
        }
        node = Some(n);
    }
    match node {
        Some(n) => json!({"children": [n]}),
        None => json!({}),
    }
}

/// A tree scenario.
pub struct Tree {
    pub page: &'static str,
    pub measure: &'static str,
    /// ExplainBy fields, in the visual's projection order.
    pub explain: Vec<&'static str>,
    /// Shown levels: (field, AI method).
    pub levels: Vec<(&'static str, Option<&'static str>)>,
    /// Selected path, one value per level from the first (`None`: blank).
    pub path: Vec<Option<&'static str>>,
    pub objects: Value,
    /// `query.sortDefinition`.
    pub sort: Option<Value>,
    pub size: (i64, i64),
    /// A slicer on the page: (field, selected values).
    pub slicer: Option<(&'static str, Vec<&'static str>)>,
    /// A visual filter: (field, kept values).
    pub filter: Option<(&'static str, Vec<&'static str>)>,
    pub note: &'static str,
}

impl Tree {
    /// The PBIR `expansionStates`: the shown levels first (pinned), then the other fields.
    pub fn expansion(&self) -> Value {
        let mut levels: Vec<Value> = self
            .levels
            .iter()
            .enumerate()
            .map(|(i, (f, ai))| level(f, true, *ai, i < self.path.len()))
            .collect();
        for f in &self.explain {
            if !self.levels.iter().any(|(l, _)| l == f) {
                levels.push(level(f, false, None, false));
            }
        }
        json!([{"roles": ["ExplainBy"], "levels": levels, "root": path(&self.path)}])
    }
}

fn tree(
    page: &'static str,
    explain: &[&'static str],
    levels: &[(&'static str, Option<&'static str>)],
) -> Tree {
    Tree {
        page,
        measure: "Total",
        explain: explain.to_vec(),
        levels: levels.to_vec(),
        path: Vec::new(),
        objects: json!({}),
        sort: None,
        size: (1000, 400),
        slicer: None,
        filter: None,
        note: "",
    }
}

const ALL5: [&str; 5] = ["Region", "Category", "Channel", "Segment", "Product"];
const MAX: Option<&str> = Some("MaxSplit");
const MIN: Option<&str> = Some("MinSplit");

fn tree_obj(props: &[(&str, &str)]) -> Value {
    let mut p = Map::new();
    for (k, v) in props {
        p.insert((*k).to_string(), e(v));
    }
    Value::Object(p)
}

pub fn trees() -> Vec<Tree> {
    let mut out = Vec::new();
    let mut t = tree("D01", &ALL5, &[]);
    t.note = "root only";
    out.push(t);
    let mut t = tree("D02", &ALL5, &[("Region", None)]);
    t.note = "one level, nothing selected";
    out.push(t);
    let mut t = tree("D03", &ALL5, &[("Region", None), ("Category", None)]);
    t.path = vec![Some("South")];
    t.note = "two levels, South selected";
    out.push(t);
    let mut t = tree("D04", &ALL5, &[("Region", None), ("Category", MAX)]);
    t.path = vec![Some("East")];
    t.note = "High value under East, saved with a field that is not the best one";
    out.push(t);
    let mut t = tree("D05", &ALL5, &[("Region", MIN)]);
    t.note = "Low value from the root";
    out.push(t);
    let mut t = tree("D06", &ALL5, &[("Region", None), ("Channel", None)]);
    t.path = vec![Some("West")];
    t.note = "selected node below the bars shown";
    out.push(t);
    let mut t = tree("D07", &ALL5, &[("Region", None), ("Category", None)]);
    t.path = vec![Some("North")];
    t.sort = Some(
        json!({"sort": [{"field": {"Measure": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Total"}}, "direction": "Descending"}]}),
    );
    t.note = "sorted by the measure, descending";
    out.push(t);
    let mut t = tree(
        "D08",
        &["Segment", "Product", "Region"],
        &[("Segment", None), ("Product", None)],
    );
    t.measure = "Margin %";
    t.path = vec![None];
    t.objects = json!({"tree": [{"properties": tree_obj(&[("barsPerLevel", "6L")])}]});
    t.note = "ratio measure, blank member selected, 14 products, 6 bars per level";
    out.push(t);
    let mut t = tree("D09", &ALL5, &[("Region", None), ("Category", None)]);
    t.path = vec![Some("South")];
    t.size = (1000, 400);
    t.slicer = Some(("Channel", vec!["Online"]));
    t.filter = Some(("Category", vec!["Bikes", "Clothing"]));
    t.note = "a slicer on Channel and a visual filter on Category";
    out.push(t);
    let mut t = tree("D10", &ALL5, &[("Region", None), ("Category", None)]);
    t.path = vec![Some("East")];
    t.size = (600, 700);
    t.objects = json!({"tree": [{"properties": tree_obj(&[("density", "'sparse'")])}]});
    t.note = "tall and narrow, sparse";
    out.push(t);
    let mut t = tree("D11", &ALL5, &[("Region", None), ("Channel", None)]);
    t.path = vec![Some("North")];
    t.objects = json!({
        "tree": [{"properties": tree_obj(&[("density", "'dense'"), ("connectorType", "'round'")])}],
        "dataBars": [{"properties": tree_obj(&[("dataBarScalingType", "'parentNode'")])}],
        "dataLabels": [{"properties": tree_obj(&[("dataLabelDisplayUnits", "1000D"), ("dataLabelPrecision", "1L")])}]
    });
    t.note = "dense, round connectors, bars scaled to the parent, values in thousands";
    out.push(t);
    let mut t = tree("D12", &ALL5, &[("Region", None), ("Category", MAX)]);
    t.path = vec![Some("South")];
    t.objects = json!({"analysis": [{"properties": tree_obj(&[("aiMode", "'relative'")])}]});
    t.note = "relative High value under South";
    out.push(t);
    let mut t = tree("D13", &ALL5, &[("Region", MAX), ("Category", MAX)]);
    t.note = "two High value levels from the root";
    out.push(t);
    let mut t = tree(
        "D14",
        &["Category", "Region", "Channel", "Segment", "Product"],
        &[("Region", None), ("Segment", None), ("Product", None)],
    );
    t.path = vec![Some("East"), None];
    t.note = "levels in another order than the fields; blank segment on the path";
    out.push(t);
    // the window of a long level without a selection, and the bar area's size
    let mut t = tree(
        "D15",
        &["Segment", "Product", "Region"],
        &[("Segment", None), ("Product", None)],
    );
    t.path = vec![None];
    t.note = "as D08 with Total and no bar limit";
    out.push(t);
    let mut t = tree(
        "D16",
        &["Segment", "Product", "Region"],
        &[("Segment", None), ("Product", None)],
    );
    t.measure = "Margin %";
    t.path = vec![None];
    t.note = "as D08 without the bar limit";
    out.push(t);
    let mut t = tree(
        "D17",
        &["Segment", "Product", "Region"],
        &[("Segment", None), ("Product", None)],
    );
    t.path = vec![Some("Retail")];
    t.note = "a long level under the last member";
    out.push(t);
    let mut t = tree("D18", &ALL5, &[("Region", None), ("Category", None)]);
    t.path = vec![Some("East")];
    t.size = (1000, 300);
    t.note = "300 px high";
    out.push(t);
    let mut t = tree("D19", &ALL5, &[("Product", None)]);
    t.size = (1000, 700);
    t.note = "700 px high, 14 products from the root";
    out.push(t);
    let mut t = tree("D20", &ALL5, &[("Product", None)]);
    t.size = (1000, 700);
    t.objects = json!({"tree": [{"properties": tree_obj(&[("density", "'dense'")])}]});
    t.note = "700 px high, dense";
    out.push(t);
    let mut t = tree(
        "D21",
        &ALL5,
        &[("Region", None), ("Category", None), ("Channel", None)],
    );
    t.path = vec![Some("South"), Some("Accessories")];
    t.size = (420, 400);
    t.note = "narrower than its levels";
    out.push(t);
    let mut t = tree("D22", &ALL5, &[("Product", None)]);
    t.size = (1000, 500);
    t.objects = json!({"tree": [{"properties": tree_obj(&[("barsPerLevel", "3L")])}]});
    t.note = "500 px high, 3 bars per level";
    out.push(t);
    let mut t = tree("D23", &ALL5, &[("Product", None), ("Region", None)]);
    t.path = vec![Some("P06")];
    t.size = (1000, 500);
    t.note = "500 px high, a product in the middle selected";
    out.push(t);
    out
}

/// The page key of a tree scenario.
fn visual_name(t: &Tree) -> String {
    t.page.to_lowercase()
}

pub fn apply_spec(ts: &[Tree]) -> Value {
    let pages: Vec<Value> = ts
        .iter()
        .map(|t| {
            let mut fields = vec![json!({"role": "Analyze", "field": format!("[{}]", t.measure)})];
            for f in &t.explain {
                fields.push(json!({"role": "ExplainBy", "field": format!("Sales[{f}]")}));
            }
            let (w, h) = t.size;
            let mut visuals = vec![json!({"name": visual_name(t), "type": "decompositionTreeVisual",
                "x": 20, "y": 20, "width": w, "height": h, "fields": fields})];
            if let Some((f, _)) = &t.slicer {
                visuals.push(json!({"name": format!("{}s", visual_name(t)), "type": "slicer",
                    "x": 1060, "y": 20, "width": 200, "height": 200,
                    "fields": [{"role": "Values", "field": format!("Sales[{f}]")}]}));
            }
            json!({"key": t.page, "display_name": t.page, "width": 1280, "height": 720, "visuals": visuals})
        })
        .collect();
    json!({"pages": pages})
}

pub fn cmds1(dir: &str) -> Value {
    let mut c = Cmds::default();
    c.c("model.new", json!({"name": "Decomp", "culture": "en-US"}));
    c.c(
        "table.import",
        json!({"name": "Sales", "csv": join(dir, &["sales.csv"]),
               "types": {"Amount": "double", "Cost": "double"}}),
    );
    c.c(
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
    );
    c.c(
        "measure.add",
        json!({"table": "Sales", "name": "Margin %",
               "expression": "DIVIDE(SUM(Sales[Amount])-SUM(Sales[Cost]),SUM(Sales[Amount]))",
               "format_string": "0.0%"}),
    );
    c.c("report.new", json!({}));
    c.c(
        "report.apply",
        json!({"spec": apply_spec(&trees()), "prune": "all"}),
    );
    c.c(
        "project.new",
        json!({"dir": join(dir, &["proj"]), "name": "Decomp"}),
    );
    c.value()
}

pub fn cmds2(dir: &str) -> Value {
    json!([
        {"command": "file.open", "params": {"path": join(dir, &["proj", "Decomp.pbip"])}},
        {"command": "file.save_as", "params": {"path": join(dir, &["decomp.pbix"]), "overwrite": true}}
    ])
}

/// What the reader reads: every page with its tree's rectangle.
pub fn plan() -> Value {
    let ts = trees();
    let pages: Vec<Value> = ts
        .iter()
        .map(|t| {
            json!({"page": t.page, "visual": visual_name(t), "x": 20, "y": 20,
                        "width": t.size.0, "height": t.size.1, "note": t.note})
        })
        .collect();
    json!({"version": 1, "pbix": "decomp.pbix", "width": 1280, "height": 720, "pages": pages})
}

fn in_values(alias: &str, col: &str, vals: &[&str]) -> Value {
    let vals: Vec<Value> = vals.iter().map(|v| json!([lit(&format!("'{v}'"))])).collect();
    json!({"Version": 2, "From": [{"Name": alias, "Entity": "Sales", "Type": 0}],
           "Where": [{"Condition": {"In": {"Expressions": [
               {"Column": {"Expression": {"SourceRef": {"Source": alias}}, "Property": col}}],
               "Values": vals}}}]})
}

/// Writes each tree's expansion state, options, sort and filters, and the slicers' selections
/// into the generated project (`report_dir` = `…/Decomp.Report`).
pub fn patch_report(report_dir: &Path) -> Result<usize, String> {
    let ts = trees();
    let by_name: BTreeMap<String, &Tree> = ts.iter().map(|t| (visual_name(t), t)).collect();
    let pages_dir = report_dir.join("definition").join("pages");
    let mut patched = 0usize;
    for p in sorted_dirs(&pages_dir)? {
        let vdir = p.join("visuals");
        if !vdir.is_dir() {
            continue;
        }
        for vd in sorted_dirs(&vdir)? {
            let f = vd.join("visual.json");
            if !f.is_file() {
                continue;
            }
            let mut j = read_json(&f)?;
            let name = j
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(base) = name.strip_suffix('s')
                && let Some(t) = by_name.get(base)
                && let Some((col, vals)) = &t.slicer
            {
                let filter = in_values("s", col, vals);
                j["visual"]["objects"] = json!({"general": [{"properties": {"filter": {"filter": filter}}}]});
                write_json(&f, &j, 2)?;
                patched += 1;
                continue;
            }
            let Some(t) = by_name.get(&name) else {
                continue;
            };
            if let Some((col, vals)) = &t.filter {
                j["filterConfig"] = json!({"filters": [{
                    "name": "decompfilter", "type": "Categorical",
                    "field": {"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": col}},
                    "filter": in_values("s", col, vals)}]});
            }
            let Some(visual) = j.get_mut("visual").and_then(Value::as_object_mut) else {
                return Err(format!("{}: no `visual`", f.display()));
            };
            if !t.levels.is_empty() {
                visual.insert("expansionStates".into(), t.expansion());
            }
            if let Some(o) = t.objects.as_object().filter(|o| !o.is_empty()) {
                visual.insert("objects".into(), Value::Object(o.clone()));
            }
            if let Some(q) = visual.get_mut("query").and_then(Value::as_object_mut) {
                if let Some(s) = &t.sort {
                    q.insert("sortDefinition".into(), s.clone());
                }
                // the shown levels' projections first, marked active (as Desktop saves them)
                if let Some(ps) = q
                    .get_mut("queryState")
                    .and_then(|s| s.get_mut("ExplainBy"))
                    .and_then(|r| r.get_mut("projections"))
                    .and_then(Value::as_array_mut)
                {
                    let mut shown: Vec<Value> = Vec::new();
                    for (f, _) in &t.levels {
                        let r = format!("Sales.{f}");
                        if let Some(i) = ps
                            .iter()
                            .position(|p| p.get("queryRef").and_then(Value::as_str) == Some(&r))
                        {
                            let mut p = ps.remove(i);
                            p["active"] = json!(true);
                            shown.push(p);
                        }
                    }
                    if !shown.is_empty() {
                        for p in ps.iter_mut() {
                            p["active"] = json!(false);
                        }
                        shown.append(ps);
                        *ps = shown;
                    }
                }
            }
            write_json(&f, &j, 2)?;
            patched += 1;
        }
    }
    Ok(patched)
}

fn sorted_dirs(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    v.sort();
    Ok(v)
}

fn generate(dir: &str) -> Result<(), String> {
    let d = Path::new(dir);
    std::fs::create_dir_all(d).map_err(|e| format!("{dir}: {e}"))?;
    super::write_text(&d.join("sales.csv"), &sales_csv())?;
    write_json(&d.join("cmds1.json"), &cmds1(dir), 1)?;
    write_json(&d.join("cmds2.json"), &cmds2(dir), 1)?;
    write_json(&d.join("plan.json"), &plan(), 1)?;
    println!(
        "{} trees; next: bicraft run --file {}",
        trees().len(),
        join(dir, &["cmds1.json"])
    );
    Ok(())
}

/// The committed corpus folder.
pub fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("testsupport")
        .join("decomp_oracle")
}

/// A number as Desktop wrote it in its locale ("189 144", "1 234,5", "38,0%", "12,3K"):
/// separators dropped, the decimal comma read, a trailing % or K kept apart.
pub fn parse_shown(text: &str) -> Option<f64> {
    let t: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{a0}' && *c != '\u{202f}')
        .collect();
    let (t, k) = if let Some(x) = t.strip_suffix('%') {
        (x.to_string(), 0.01)
    } else if let Some(x) = t.strip_suffix('K') {
        (x.to_string(), 1000.0)
    } else {
        (t, 1.0)
    };
    let t = t.replace(',', ".");
    t.parse::<f64>().ok().map(|x| x * k)
}

/// readings.json (one entry per page: the tree's DOM reading) -> corpus.json.
/// How Desktop names a blank member in its UI languages (en, fr): read as `null`.
const BLANK_WORDS: [&str; 2] = ["(Blank)", "(Vide)"];

/// A header text read with its accessible title (`RegionRegion`): the text once.
fn once(v: Option<&Value>) -> Value {
    let Some(s) = v.and_then(Value::as_str) else {
        return Value::Null;
    };
    let n = s.chars().count();
    if n % 2 == 0 {
        let half: String = s.chars().take(n / 2).collect();
        if s == format!("{half}{half}") {
            return json!(half);
        }
    }
    json!(s)
}

fn collect(dir: &Path, out: Option<&Path>) -> Result<(), String> {
    let raw = read_json(&dir.join("readings.json"))?;
    let meta = read_json(&dir.join("instance.json")).unwrap_or(Value::Null);
    let mut trees_out = Vec::new();
    for r in raw.get("pages").and_then(Value::as_array).into_iter().flatten() {
        let page = r.get("page").and_then(Value::as_str).unwrap_or_default();
        let Some(tree) = r.get("tree").filter(|t| t.is_object()) else {
            trees_out.push(json!({"page": page, "found": false}));
            continue;
        };
        let k = tree.get("k").and_then(Value::as_f64).unwrap_or(1.0);
        let px = |v: &Value| -> Value {
            v.as_f64()
                .map_or(Value::Null, |x| json!((x * k * 10.0).round() / 10.0))
        };
        // nodes grouped into levels by their x
        let mut cols: Vec<f64> = Vec::new();
        let nodes = tree
            .get("nodes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for n in &nodes {
            let x = n.get("x").and_then(Value::as_f64).unwrap_or(0.0);
            if !cols.iter().any(|c| (c - x).abs() < 3.0) {
                cols.push(x);
            }
        }
        cols.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        // a level's scroll arrows sit under the middle of its bars
        let arrows = tree
            .get("more")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let first_y = nodes
            .iter()
            .filter_map(|n| n.get("y").and_then(Value::as_f64))
            .fold(f64::INFINITY, f64::min);
        let arrow = |cx: f64, up: bool| -> bool {
            arrows.iter().any(|m| {
                let (x, y) = (
                    m.get("x").and_then(Value::as_f64).unwrap_or(-1e9),
                    m.get("y").and_then(Value::as_f64).unwrap_or(0.0),
                );
                x > cx && x < cx + 120.0 && (y < first_y) == up
            })
        };
        let headers = tree
            .get("headers")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut levels = Vec::new();
        for (li, cx) in cols.iter().enumerate() {
            let mut ns: Vec<&Value> = nodes
                .iter()
                .filter(|n| (n.get("x").and_then(Value::as_f64).unwrap_or(0.0) - cx).abs() < 3.0)
                .collect();
            ns.sort_by(|a, b| {
                let y = |n: &Value| n.get("y").and_then(Value::as_f64).unwrap_or(0.0);
                y(a).partial_cmp(&y(b)).unwrap_or(std::cmp::Ordering::Equal)
            });
            let blank_word = r.get("blank").and_then(Value::as_str);
            let blank = |s: &str| blank_word.map_or_else(|| BLANK_WORDS.contains(&s), |b| b == s);
            let shown: Vec<Value> = ns
                .iter()
                .map(|n| {
                    let label = n.get("label").and_then(Value::as_str).unwrap_or_default();
                    let value = n.get("value").and_then(Value::as_str).unwrap_or_default();
                    json!({
                        "label": if blank(label) { Value::Null } else { json!(label) },
                        "value": parse_shown(value),
                        "value_text": value,
                        "on_path": n.get("path").and_then(Value::as_bool).unwrap_or(false),
                        "y": px(n.get("y").unwrap_or(&Value::Null)),
                        "bar": px(n.get("bar").unwrap_or(&Value::Null)),
                    })
                })
                .collect();
            // the header over this column (none over the root)
            let header = headers
                .iter()
                .find(|h| (h.get("x").and_then(Value::as_f64).unwrap_or(-1e9) - cx).abs() < 3.0);
            levels.push(json!({
                "x": px(&json!(cx)),
                "up": arrow(*cx, true),
                "down": arrow(*cx, false),
                "field": once(header.and_then(|h| h.get("title"))),
                "subtitle": match once(header.and_then(|h| h.get("subtitle"))) {
                    Value::String(s) if blank(&s) => json!({"blank": true}),
                    other => other,
                },
                "ai": header.and_then(|h| h.get("ai")).cloned().unwrap_or(json!(false)),
                "nodes": shown,
            }));
            let _ = li;
        }
        trees_out.push(json!({
            "page": page,
            "found": true,
            "levels": levels,
            "scroll_down": tree.get("more_down").cloned().unwrap_or(json!(0)),
            "scroll_up": tree.get("more_up").cloned().unwrap_or(json!(0)),
            "links": tree
                .get("links")
                .and_then(Value::as_array)
                .map(|ls| {
                    ls.iter()
                        .map(|l| json!({"d": l.get("d"), "stroke": l.get("stroke"), "width": l.get("width")}))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            "bar_width": px(tree.get("bar_width").unwrap_or(&Value::Null)),
        }));
    }
    let corpus = json!({
        "version": 1,
        "generator": "cargo xtask oracle-gen decomp (xtask/src/oracles/decomp.rs)",
        "desktop": meta.get("desktop").cloned().unwrap_or(Value::Null),
        "ui_culture": meta.get("ui_culture").cloned().unwrap_or(Value::Null),
        "recorded": meta.get("recorded").cloned().unwrap_or(Value::Null),
        "trees": trees_out,
    });
    let dest = out.map_or_else(|| corpus_dir().join("corpus.json"), Path::to_path_buf);
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    write_json(&dest, &corpus, 1)?;
    println!("wrote {}", dest.display());
    Ok(())
}

/// `cargo xtask oracle-gen decomp …`.
pub fn cmd(args: &[&str]) -> Result<(), String> {
    match args {
        [dir] => generate(dir),
        [dir, "patch"] => {
            let r = Path::new(dir).join("proj").join("Decomp.Report");
            let n = patch_report(&r)?;
            println!("patched {n} files");
            Ok(())
        }
        [dir, "collect"] => collect(Path::new(dir), None),
        [dir, "collect", out] => collect(Path::new(dir), Some(Path::new(out))),
        _ => Err("usage: oracle-gen decomp DIR [patch | collect [OUT.json]]".into()),
    }
}
