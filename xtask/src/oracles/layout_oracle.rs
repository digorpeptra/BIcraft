//! `layout-oracle`: the legacy layout → PBIR oracle (G13, `docs/formats/legacy-layout-bb.md`).
//!
//! Synthetic legacy reports (our own files) covering visual types, formatting objects,
//! conditional formatting, sort definitions, filters of every level and type, page settings,
//! page bindings, bookmarks, groups, phone layouts, themes, report settings, report-level
//! measures and custom visuals. Power BI Desktop converts each one to PBIR; the local-only test
//! `crates/engine/tests/layout_oracle.rs` compares bicraft's conversion (`file.convert` with
//! `report_format: "pbir"`) with Desktop's, file by file.
//!
//! 1. `cargo xtask layout-oracle gen DIR` writes `DIR/cmds.json` (a `bicraft run` command list
//!    building the model and one PBIP project per case) and `DIR/legacy/<case>/` (the legacy
//!    `report.json` and its `StaticResources/` files).
//! 2. `bicraft run --file DIR/cmds.json` writes `DIR/<case>/<case>.pbip` (blank PBIR report).
//! 3. `cargo xtask layout-oracle install DIR` replaces each project's report with the legacy one
//!    (`report.json`, resources; the PBIR `definition/` folder removed).
//! 4. `tools/layout-oracle/run.ps1` opens each project in Desktop, lets it upgrade the report to
//!    PBIR on save, and copies the project to `%BICRAFT_TESTDATA%\layout-oracle\<case>\`.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::{Cmds, stable_name, write_json};

pub const USAGE: &str = "\
usage: cargo xtask layout-oracle <step> DIR   (docs/formats/legacy-layout-bb.md)
  gen DIR        write DIR/cmds.json (bicraft run: model + one PBIP per case) and DIR/legacy/
  install DIR    replace each DIR/<case>/<case>.Report with the legacy report.json
";

pub fn cmd(args: &[String]) -> Result<(), String> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["gen", dir] => gen_all(Path::new(dir)),
        ["install", dir] => install(Path::new(dir)),
        _ => Err(USAGE.into()),
    }
}

/// Case names, in generation order.
pub const CASES: &[&str] = &[
    "visuals",
    "filters",
    "pages",
    "bookmarks",
    "mobile",
    "report",
    "customvisual",
    "groups",
];

fn gen_all(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut c = Cmds::default();
    model_cmds(&mut c);
    for case in CASES {
        let (layout, resources) = layout_of(case)?;
        let out = dir.join("legacy").join(case);
        std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        let text = serde_json::to_string(&layout).map_err(|e| e.to_string())?;
        std::fs::write(out.join("report.json"), text).map_err(|e| format!("{}: {e}", out.display()))?;
        for (rel, bytes) in resources {
            let p = out.join(&rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            std::fs::write(&p, bytes).map_err(|e| format!("{}: {e}", p.display()))?;
        }
        c.c(
            "project.new",
            json!({"dir": dir.join(case).to_string_lossy(), "name": case}),
        );
    }
    write_json(&dir.join("cmds.json"), &c.value(), 1)?;
    println!(
        "wrote {} and {} legacy reports under {}",
        dir.join("cmds.json").display(),
        CASES.len(),
        dir.join("legacy").display()
    );
    Ok(())
}

fn install(dir: &Path) -> Result<(), String> {
    for case in CASES {
        let src = dir.join("legacy").join(case);
        let report = dir.join(case).join(format!("{case}.Report"));
        if !report.is_dir() {
            return Err(format!(
                "{} is missing: run `bicraft run --file {}` first",
                report.display(),
                dir.join("cmds.json").display()
            ));
        }
        let definition = report.join("definition");
        if definition.is_dir() {
            std::fs::remove_dir_all(&definition).map_err(|e| format!("{}: {e}", definition.display()))?;
        }
        copy_tree(&src, &report)?;
        println!("{case}: legacy report installed in {}", report.display());
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    let entries = std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let target: PathBuf = to.join(entry.file_name());
        if path.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
            copy_tree(&path, &target)?;
        } else {
            std::fs::copy(&path, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Model

fn model_cmds(c: &mut Cmds) {
    c.c("model.new", json!({"name": "LayoutOracle"}));
    let regions = ["North", "South", "East", "West"];
    let cats = ["Bikes", "Clothing", "Accessories"];
    let mut rows = vec!["Region,Category,Product,OrderDate,Amount,Qty".to_string()];
    let mut k = 0u32;
    for r in regions {
        for c in cats {
            for m in 1..=6u32 {
                k += 1;
                rows.push(format!(
                    "{r},{c},{c} {},2025-{m:02}-{:02},{},{}",
                    k % 4,
                    k % 27 + 1,
                    100 + (k * 37) % 400,
                    k % 9 + 1
                ));
            }
        }
    }
    c.c(
        "table.import",
        json!({"name": "Sales", "csv": rows.join("\n") + "\n",
               "types": {"OrderDate": "dateTime", "Amount": "double", "Qty": "int64"}}),
    );
    let mut reg = vec!["Region,Manager".to_string()];
    for (i, r) in regions.iter().enumerate() {
        reg.push(format!("{r},Manager {i}"));
    }
    c.c(
        "table.import",
        json!({"name": "Regions", "csv": reg.join("\n") + "\n"}),
    );
    c.c(
        "relationship.add",
        json!({"from": "Sales[Region]", "to": "Regions[Region]"}),
    );
    for (n, e, f) in [
        ("Total Sales", "SUM(Sales[Amount])", "#,0"),
        ("Total Qty", "SUM(Sales[Qty])", "#,0"),
        ("Avg Price", "DIVIDE([Total Sales],[Total Qty])", "0.00"),
    ] {
        c.c(
            "measure.add",
            json!({"table": "Sales", "name": n, "expression": e, "format_string": f}),
        );
    }
    c.c(
        "hierarchy.add",
        json!({"table": "Sales", "name": "Geo", "levels": ["Region", "Product"]}),
    );
}

// ---------------------------------------------------------------------------------------------
// Semantic-query helpers

/// A field of a visual query.
#[derive(Clone, Copy)]
enum F {
    /// Column (table, column).
    C(&'static str, &'static str),
    /// Measure (table, measure).
    M(&'static str, &'static str),
    /// Aggregation (table, column, function code).
    A(&'static str, &'static str, u8),
    /// Hierarchy level (table, hierarchy, level).
    H(&'static str, &'static str, &'static str),
    /// Report-level measure (`extension` schema; table, measure).
    X(&'static str, &'static str),
}

fn alias(table: &str, ext: bool) -> String {
    let first = table.chars().next().unwrap_or('t').to_ascii_lowercase();
    if ext {
        format!("{first}1")
    } else {
        first.to_string()
    }
}

/// `SourceRef` by alias (prototype queries, filter `Where`) or by entity (filter `expression`).
fn src(table: &str, ext: bool, by_alias: bool) -> Value {
    if by_alias {
        json!({"SourceRef": {"Source": alias(table, ext)}})
    } else if ext {
        json!({"SourceRef": {"Schema": "extension", "Entity": table}})
    } else {
        json!({"SourceRef": {"Entity": table}})
    }
}

fn agg_name(f: u8) -> &'static str {
    match f {
        0 => "Sum",
        1 => "Avg",
        2 => "Count",
        3 => "Min",
        4 => "Max",
        5 => "CountNonNull",
        _ => "Sum",
    }
}

impl F {
    fn table(self) -> (&'static str, bool) {
        match self {
            F::C(t, _) | F::M(t, _) | F::A(t, _, _) | F::H(t, _, _) => (t, false),
            F::X(t, _) => (t, true),
        }
    }

    fn expr(self, by_alias: bool) -> Value {
        match self {
            F::C(t, c) => json!({"Column": {"Expression": src(t, false, by_alias), "Property": c}}),
            F::M(t, m) => json!({"Measure": {"Expression": src(t, false, by_alias), "Property": m}}),
            F::X(t, m) => json!({"Measure": {"Expression": src(t, true, by_alias), "Property": m}}),
            F::A(t, c, f) => json!({"Aggregation": {
                "Expression": {"Column": {"Expression": src(t, false, by_alias), "Property": c}},
                "Function": f}}),
            F::H(t, h, l) => json!({"HierarchyLevel": {
                "Expression": {"Hierarchy": {"Expression": src(t, false, by_alias), "Hierarchy": h}},
                "Level": l}}),
        }
    }

    fn query_ref(self) -> String {
        match self {
            F::C(t, c) => format!("{t}.{c}"),
            F::M(t, m) | F::X(t, m) => format!("{t}.{m}"),
            F::A(t, c, f) => format!("{}({t}.{c})", agg_name(f)),
            F::H(t, h, l) => format!("{t}.{h}.{l}"),
        }
    }

    fn native(self) -> String {
        match self {
            F::C(_, c) => c.to_string(),
            F::M(_, m) | F::X(_, m) => m.to_string(),
            F::A(_, c, f) => format!("{} of {c}", agg_name(f)),
            F::H(_, h, l) => format!("{h} {l}"),
        }
    }
}

/// The `From` list for a set of fields (first-use order, deduplicated).
fn from_list(fields: &[F]) -> Vec<Value> {
    let mut seen: Vec<(String, bool)> = Vec::new();
    let mut out = Vec::new();
    for f in fields {
        let (t, ext) = f.table();
        if seen.iter().any(|(s, e)| s == t && *e == ext) {
            continue;
        }
        seen.push((t.to_string(), ext));
        let mut m = Map::new();
        m.insert("Name".into(), json!(alias(t, ext)));
        m.insert("Entity".into(), json!(t));
        if ext {
            m.insert("Schema".into(), json!("extension"));
        }
        m.insert("Type".into(), json!(0));
        out.push(Value::Object(m));
    }
    out
}

fn lit(v: &str) -> Value {
    json!({"expr": {"Literal": {"Value": v}}})
}

fn props(pairs: &[(&str, Value)]) -> Value {
    let mut m = Map::new();
    for (k, v) in pairs {
        m.insert((*k).into(), v.clone());
    }
    json!([{"properties": Value::Object(m)}])
}

fn color(hex: &str) -> Value {
    json!({"solid": {"color": lit(&format!("'{hex}'"))}})
}

// ---------------------------------------------------------------------------------------------
// Filters

fn in_values(f: F, values: &[&str]) -> Value {
    json!({"In": {"Expressions": [f.expr(true)],
                  "Values": values.iter().map(|v| json!([{"Literal": {"Value": v}}])).collect::<Vec<_>>()}})
}

fn filter(name: &str, f: F, ty: &str, conditions: Option<Vec<Value>>, more: Value) -> Value {
    let mut m = Map::new();
    m.insert("name".into(), json!(name));
    m.insert("expression".into(), f.expr(false));
    if let Some(conds) = conditions {
        m.insert(
            "filter".into(),
            json!({"Version": 2, "From": from_list(&[f]),
                   "Where": conds.into_iter().map(|c| json!({"Condition": c})).collect::<Vec<_>>()}),
        );
    }
    m.insert("type".into(), json!(ty));
    m.insert("howCreated".into(), json!(1));
    if let Value::Object(more) = more {
        for (k, v) in more {
            m.insert(k, v);
        }
    }
    Value::Object(m)
}

fn cmp(f: F, kind: u8, value: &str) -> Value {
    json!({"Comparison": {"ComparisonKind": kind, "Left": f.expr(true), "Right": {"Literal": {"Value": value}}}})
}

fn top_n(name: &str, by: F, n: i64, ordered_by: F, desc: bool) -> Value {
    let (t, _) = by.table();
    let mut inner_from = from_list(&[by]);
    for v in from_list(&[ordered_by]) {
        if !inner_from.contains(&v) {
            inner_from.push(v);
        }
    }
    let sub = json!({"Version": 2, "From": inner_from,
        "Select": [{"Column": {"Expression": {"SourceRef": {"Source": alias(t, false)}},
                    "Property": match by { F::C(_, c) => c, _ => "" }}, "Name": "field"}],
        "OrderBy": [{"Direction": if desc { 2 } else { 1 }, "Expression": ordered_by.expr(true)}],
        "Top": n});
    let mut from = vec![json!({"Name": "subquery", "Expression": {"Subquery": {"Query": sub}}, "Type": 2})];
    from.extend(from_list(&[by]));
    json!({"name": name, "expression": by.expr(false),
           "filter": {"Version": 2, "From": from,
                      "Where": [{"Condition": {"In": {"Expressions": [by.expr(true)],
                                 "Table": {"SourceRef": {"Source": "subquery"}}}}}]},
           "type": "TopN", "howCreated": 1})
}

/// Relative date (last `n` units, `unit` a time-unit code, including today).
fn relative(f: F, n: i64, unit: u8, span_unit: u8) -> Value {
    json!({"Between": {"Expression": f.expr(true),
        "LowerBound": {"DateSpan": {"Expression": {"DateAdd": {"Expression": {"DateAdd": {
            "Expression": {"Now": {}}, "Amount": 1, "TimeUnit": 0}}, "Amount": -n, "TimeUnit": unit}},
            "TimeUnit": span_unit}},
        "UpperBound": {"DateSpan": {"Expression": {"Now": {}}, "TimeUnit": span_unit}}}})
}

// ---------------------------------------------------------------------------------------------
// Visual containers

#[derive(Default)]
struct V {
    name: String,
    ty: String,
    pos: [f64; 5],
    roles: Vec<(&'static str, F)>,
    /// Extra `singleVisual` members (objects, vcObjects, …), in order.
    sv: Map<String, Value>,
    sort: Vec<(F, u8)>,
    filters: Vec<Value>,
    parent: Option<String>,
    phone: Option<[f64; 4]>,
    group: Option<Value>,
    /// Extra `config` members.
    config: Map<String, Value>,
    no_query: bool,
}

fn v(key: &str, ty: &str, x: f64, y: f64, w: f64, h: f64) -> V {
    V {
        name: stable_name(key),
        ty: ty.into(),
        pos: [x, y, 0.0, w, h],
        ..V::default()
    }
}

impl V {
    fn f(mut self, role: &'static str, f: F) -> Self {
        self.roles.push((role, f));
        self
    }
    fn s(mut self, k: &str, val: Value) -> Self {
        self.sv.insert(k.into(), val);
        self
    }
    fn obj(self, k: &str, val: Value) -> Self {
        self.s("objects", val_merge(k, val))
    }
    fn sort(mut self, f: F, dir: u8) -> Self {
        self.sort.push((f, dir));
        self
    }
    fn filter(mut self, f: Value) -> Self {
        self.filters.push(f);
        self
    }
    fn parent(mut self, g: &str) -> Self {
        self.parent = Some(g.into());
        self
    }
    fn phone(mut self, p: [f64; 4]) -> Self {
        self.phone = Some(p);
        self
    }
    fn cfg(mut self, k: &str, val: Value) -> Self {
        self.config.insert(k.into(), val);
        self
    }

    fn build(self, z: f64) -> Value {
        let [x, y, _, w, h] = self.pos;
        let tab = z;
        let mut layouts = vec![
            json!({"id": 0, "position": {"x": x, "y": y, "z": z, "width": w, "height": h, "tabOrder": tab}}),
        ];
        if let Some([px, py, pw, ph]) = self.phone {
            layouts.push(json!({"id": 1, "position": {"x": px, "y": py, "z": z, "width": pw, "height": ph, "tabOrder": tab}}));
        }
        let mut config = Map::new();
        config.insert("name".into(), json!(self.name));
        config.insert("layouts".into(), Value::Array(layouts));
        if let Some(g) = self.group {
            config.insert("singleVisualGroup".into(), g);
        } else {
            let mut sv = Map::new();
            sv.insert("visualType".into(), json!(self.ty));
            if !self.no_query {
                let mut proj = Map::new();
                for (role, f) in &self.roles {
                    let e = proj.entry(role.to_string()).or_insert_with(|| json!([]));
                    if let Value::Array(a) = e {
                        let mut p = json!({"queryRef": f.query_ref()});
                        if a.is_empty() && matches!(*role, "Category" | "Rows" | "Axis") {
                            p["active"] = json!(true);
                        }
                        a.push(p);
                    }
                }
                sv.insert("projections".into(), Value::Object(proj));
                let mut fields: Vec<F> = self.roles.iter().map(|(_, f)| *f).collect();
                fields.extend(self.sort.iter().map(|(f, _)| *f));
                let mut q = Map::new();
                q.insert("Version".into(), json!(2));
                q.insert("From".into(), Value::Array(from_list(&fields)));
                let mut selects: Vec<Value> = Vec::new();
                let mut seen: Vec<String> = Vec::new();
                for (_, f) in &self.roles {
                    let r = f.query_ref();
                    if seen.contains(&r) {
                        continue;
                    }
                    seen.push(r.clone());
                    let mut s = match f.expr(true) {
                        Value::Object(m) => m,
                        _ => Map::new(),
                    };
                    s.insert("Name".into(), json!(r));
                    s.insert("NativeReferenceName".into(), json!(f.native()));
                    selects.push(Value::Object(s));
                }
                q.insert("Select".into(), Value::Array(selects));
                if !self.sort.is_empty() {
                    q.insert(
                        "OrderBy".into(),
                        Value::Array(
                            self.sort
                                .iter()
                                .map(|(f, d)| json!({"Direction": d, "Expression": f.expr(true)}))
                                .collect(),
                        ),
                    );
                }
                if !self.roles.is_empty() || !self.sort.is_empty() {
                    sv.insert("prototypeQuery".into(), Value::Object(q));
                }
            }
            for (k, val) in self.sv {
                sv.insert(k, val);
            }
            config.insert("singleVisual".into(), Value::Object(sv));
        }
        if let Some(p) = self.parent {
            config.insert("parentGroupName".into(), json!(p));
        }
        for (k, val) in self.config {
            config.insert(k, val);
        }
        json!({"x": x, "y": y, "z": z, "width": w, "height": h,
               "config": Value::Object(config).to_string(),
               "filters": Value::Array(self.filters).to_string(),
               "tabOrder": tab})
    }
}

fn val_merge(k: &str, val: Value) -> Value {
    let mut m = Map::new();
    m.insert(k.into(), val);
    Value::Object(m)
}

fn group(key: &str, display: &str, x: f64, y: f64, w: f64, h: f64, mode: u8) -> V {
    let mut g = v(key, "", x, y, w, h);
    g.group = Some(json!({"displayName": display, "groupMode": mode}));
    g
}

// ---------------------------------------------------------------------------------------------
// Sections and documents

struct S {
    name: String,
    display: String,
    w: f64,
    h: f64,
    display_option: u8,
    config: Map<String, Value>,
    filters: Vec<Value>,
    visuals: Vec<V>,
}

fn section(key: &str, display: &str) -> S {
    S {
        name: format!("ReportSection{}", &stable_name(key)[..12]),
        display: display.into(),
        w: 1280.0,
        h: 720.0,
        display_option: 1,
        config: Map::new(),
        filters: Vec::new(),
        visuals: Vec::new(),
    }
}

impl S {
    fn build(self, ordinal: usize) -> Value {
        let containers: Vec<Value> = self
            .visuals
            .into_iter()
            .enumerate()
            .map(|(i, v)| v.build((i as f64) * 1000.0))
            .collect();
        json!({"name": self.name, "displayName": self.display,
               "filters": Value::Array(self.filters).to_string(), "ordinal": ordinal,
               "visualContainers": containers, "config": Value::Object(self.config).to_string(),
               "displayOption": self.display_option, "width": self.w, "height": self.h})
    }
}

fn base_config() -> Map<String, Value> {
    let mut c = Map::new();
    c.insert("version".into(), json!("5.61"));
    c.insert(
        "themeCollection".into(),
        json!({"baseTheme": {"name": "CY24SU10", "version": "5.61", "type": 2}}),
    );
    c.insert("activeSectionIndex".into(), json!(0));
    c.insert("linguisticSchemaSyncVersion".into(), json!(0));
    c.insert(
        "objects".into(),
        json!({"section": props(&[("verticalAlignment", lit("'Top'"))])}),
    );
    c
}

fn shared_packages() -> Vec<Value> {
    vec![json!({"resourcePackage": {"name": "SharedResources", "type": 2,
        "items": [{"type": 202, "path": "BaseThemes/CY24SU10.json", "name": "CY24SU10"}],
        "disabled": false}})]
}

fn document(
    config: Map<String, Value>,
    filters: Vec<Value>,
    sections: Vec<S>,
    packages: Vec<Value>,
) -> Value {
    let sections: Vec<Value> = sections
        .into_iter()
        .enumerate()
        .map(|(i, s)| s.build(i))
        .collect();
    json!({"id": 0, "resourcePackages": packages, "sections": sections,
           "config": Value::Object(config).to_string(), "layoutOptimization": 0,
           "filters": Value::Array(filters).to_string(), "pods": [], "publicCustomVisuals": []})
}

type Resources = Vec<(String, Vec<u8>)>;

fn layout_of(case: &str) -> Result<(Value, Resources), String> {
    match case {
        "visuals" => Ok((case_visuals(), Vec::new())),
        "filters" => Ok((case_filters(), Vec::new())),
        "pages" => Ok((
            case_pages(),
            vec![png("StaticResources/RegisteredResources/wallpaper5163.png")?],
        )),
        "bookmarks" => Ok((case_bookmarks(), Vec::new())),
        "mobile" => case_mobile(),
        "report" => case_report(),
        "customvisual" => Ok((case_custom_visual(), Vec::new())),
        "groups" => Ok((case_groups(), Vec::new())),
        other => Err(format!("unknown case `{other}`")),
    }
}

fn png(rel: &str) -> Result<(String, Vec<u8>), String> {
    let mut pm = tiny_skia::Pixmap::new(16, 16).ok_or("pixmap")?;
    pm.fill(tiny_skia::Color::from_rgba8(30, 120, 200, 255));
    let bytes = pm.encode_png().map_err(|e| e.to_string())?;
    Ok((rel.into(), bytes))
}

const REGION: F = F::C("Sales", "Region");
const CATEGORY: F = F::C("Sales", "Category");
const PRODUCT: F = F::C("Sales", "Product");
const DATE: F = F::C("Sales", "OrderDate");
const AMOUNT: F = F::A("Sales", "Amount", 0);
const QTY_AVG: F = F::A("Sales", "Qty", 1);
const TOTAL: F = F::M("Sales", "Total Sales");
const QTY: F = F::M("Sales", "Total Qty");
const PRICE: F = F::M("Sales", "Avg Price");
const MANAGER: F = F::C("Regions", "Manager");
const GEO_REGION: F = F::H("Sales", "Geo", "Region");
const GEO_PRODUCT: F = F::H("Sales", "Geo", "Product");

fn title(text: &str) -> Value {
    props(&[("show", lit("true")), ("text", lit(&format!("'{text}'")))])
}

fn case_visuals() -> Value {
    let mut charts = section("visuals/charts", "Charts");
    let gradient = json!({"solid": {"color": {"expr": {"FillRule": {
        "Input": TOTAL.expr(false),
        "FillRule": {"linearGradient2": {
            "min": {"color": {"Literal": {"Value": "'#F2C80F'"}}},
            "max": {"color": {"Literal": {"Value": "'#118DFF'"}}},
            "nullColoringStrategy": {"strategy": {"Literal": {"Value": "'asZero'"}}}}}}}}}});
    charts.visuals = vec![
        v("v/card", "card", 10.0, 10.0, 200.0, 120.0)
            .f("Values", TOTAL)
            .obj("labels", props(&[("fontSize", lit("24D")), ("color", color("#333333"))]))
            .s("vcObjects", val_merge("title", title("Sales"))),
        v("v/column", "clusteredColumnChart", 220.0, 10.0, 400.0, 250.0)
            .f("Category", REGION)
            .f("Y", AMOUNT)
            .f("Series", CATEGORY)
            .sort(AMOUNT, 2)
            .s("hasDefaultSort", json!(false))
            .s("columnProperties", json!({"Sum(Sales.Amount)": {"displayName": "Amount sold"}}))
            .obj("dataPoint", json!([
                {"properties": {"fill": color("#118DFF")}, "selector": {"metadata": "Sum(Sales.Amount)"}},
                {"properties": {"fill": gradient}, "selector": {"data": [{"dataViewWildcard": {"matchingOption": 1}}]}}]))
            .s("drillFilterOtherVisuals", json!(true)),
        v("v/bar", "clusteredBarChart", 630.0, 10.0, 300.0, 250.0)
            .f("Category", CATEGORY)
            .f("Y", TOTAL)
            .sort(CATEGORY, 1)
            .obj("labels", props(&[("show", lit("true"))])),
        v("v/stacked", "stackedColumnChart", 940.0, 10.0, 320.0, 250.0)
            .f("Category", GEO_REGION)
            .f("Category", GEO_PRODUCT)
            .f("Y", TOTAL),
        v("v/line", "lineChart", 10.0, 270.0, 400.0, 200.0)
            .f("Category", DATE)
            .f("Y", TOTAL)
            .f("Y", QTY)
            .s("showAllRoles", json!(["Category"]))
            .obj("categoryAxis", props(&[("axisType", lit("'Categorical'"))])),
        v("v/area", "areaChart", 420.0, 270.0, 300.0, 200.0)
            .f("Category", DATE)
            .f("Y", AMOUNT),
        v("v/pie", "pieChart", 730.0, 270.0, 260.0, 200.0)
            .f("Category", CATEGORY)
            .f("Y", TOTAL)
            .obj("legend", props(&[("show", lit("true")), ("position", lit("'Right'"))])),
        v("v/donut", "donutChart", 1000.0, 270.0, 260.0, 200.0)
            .f("Category", REGION)
            .f("Y", QTY),
        v("v/scatter", "scatterChart", 10.0, 480.0, 300.0, 230.0)
            .f("Category", PRODUCT)
            .f("X", TOTAL)
            .f("Y", QTY)
            .f("Size", PRICE)
            .cfg("howCreated", json!("InsertVisualButton")),
        v("v/waterfall", "waterfallChart", 320.0, 480.0, 300.0, 230.0)
            .f("Category", CATEGORY)
            .f("Y", TOTAL),
        v("v/combo", "lineClusteredColumnComboChart", 630.0, 480.0, 320.0, 230.0)
            .f("Category", REGION)
            .f("Y", TOTAL)
            .f("Y2", QTY),
        v("v/ribbon", "ribbonChart", 960.0, 480.0, 300.0, 230.0)
            .f("Category", REGION)
            .f("Series", CATEGORY)
            .f("Y", TOTAL),
    ];

    let mut tables = section("visuals/tables", "Tables");
    tables.visuals = vec![
        v("v/table", "tableEx", 10.0, 10.0, 500.0, 300.0)
            .f("Values", REGION)
            .f("Values", MANAGER)
            .f("Values", AMOUNT)
            .f("Values", QTY_AVG)
            .sort(AMOUNT, 2)
            .s("columnProperties", json!({"Avg(Sales.Qty)": {"displayName": "Average qty", "formatString": "0.0"}}))
            .obj("values", json!([{"properties": {"backColor": {"solid": {"color": {"expr": {"FillRule": {
                "Input": AMOUNT.expr(false),
                "FillRule": {"linearGradient2": {
                    "min": {"color": {"Literal": {"Value": "'#FFFFFF'"}}},
                    "max": {"color": {"Literal": {"Value": "'#118DFF'"}}},
                    "nullColoringStrategy": {"strategy": {"Literal": {"Value": "'asZero'"}}}}}}}}}}},
                "selector": {"data": [{"dataViewWildcard": {"matchingOption": 1}}], "metadata": "Sum(Sales.Amount)"}},
                {"properties": {"fontColor": {"solid": {"color": {"expr": {"Conditional": {"Cases": [
                    {"Condition": {"Comparison": {"ComparisonKind": 2,
                        "Left": QTY_AVG.expr(false), "Right": {"Literal": {"Value": "5D"}}}},
                     "Value": {"Literal": {"Value": "'#D64550'"}}}]}}}}}},
                "selector": {"data": [{"dataViewWildcard": {"matchingOption": 1}}], "metadata": "Avg(Sales.Qty)"}}]))
            .obj("total", props(&[("totals", lit("true"))])),
        v("v/matrix", "pivotTable", 520.0, 10.0, 500.0, 300.0)
            .f("Rows", GEO_REGION)
            .f("Rows", GEO_PRODUCT)
            .f("Columns", CATEGORY)
            .f("Values", TOTAL)
            .s("expansionStates", json!([{"roles": ["Rows"], "levels": [
                {"queryRefs": ["Sales.Geo.Region"], "isCollapsed": true, "identityKeys": [
                    {"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Region"}}],
                 "isPinned": true},
                {"queryRefs": ["Sales.Geo.Product"], "isCollapsed": true, "isPinned": true}],
                "root": {"identityValues": null}}]))
            .obj("subTotals", props(&[("rowSubtotals", lit("false"))])),
        v("v/multirow", "multiRowCard", 1030.0, 10.0, 240.0, 300.0)
            .f("Values", TOTAL)
            .f("Values", QTY)
            .f("Values", PRICE),
        v("v/slicer", "slicer", 10.0, 320.0, 200.0, 250.0)
            .f("Values", REGION)
            .obj("data", props(&[("mode", lit("'Basic'"))]))
            .obj("general", json!([{"properties": {"filter": {"filter": {"Version": 2,
                "From": [{"Name": "s", "Entity": "Sales", "Type": 0}],
                "Where": [{"Condition": in_values(REGION, &["'North'", "'East'"])}]}}}}]))
            .s("syncGroup", json!({"groupName": "Region", "fieldChanges": true, "filterChanges": true})),
        v("v/slicer2", "slicer", 220.0, 320.0, 200.0, 80.0)
            .f("Values", CATEGORY)
            .obj("data", props(&[("mode", lit("'Dropdown'"))]))
            .obj("selection", props(&[("singleSelect", lit("true"))])),
        v("v/funnel", "funnel", 430.0, 320.0, 300.0, 250.0)
            .f("Category", REGION)
            .f("Y", TOTAL),
        v("v/treemap", "treemap", 740.0, 320.0, 300.0, 250.0)
            .f("Group", CATEGORY)
            .f("Values", TOTAL),
        v("v/gauge", "gauge", 1050.0, 320.0, 220.0, 200.0)
            .f("Y", TOTAL)
            .f("MaxValue", QTY),
    ];

    let mut misc = section("visuals/misc", "Shapes");
    misc.visuals = vec![
        v("v/text", "textbox", 10.0, 10.0, 300.0, 80.0)
            .obj("general", json!([{"properties": {"paragraphs": [{"textRuns": [
                {"value": "Oracle ", "textStyle": {"fontWeight": "bold"}},
                {"value": "report", "textStyle": {"fontStyle": "italic", "color": "#118DFF"}}],
                "horizontalTextAlignment": "center"}]}}])),
        v("v/shape", "shape", 320.0, 10.0, 150.0, 80.0)
            .obj("shape", props(&[("tileShape", lit("'rectangleRounded'"))]))
            .obj("fill", props(&[("fillColor", color("#E6E6E6"))])),
        v("v/button", "actionButton", 480.0, 10.0, 120.0, 40.0)
            .obj("icon", props(&[("shapeType", lit("'back'"))]))
            .s("vcObjects", val_merge("visualLink", props(&[("show", lit("true")), ("type", lit("'Back'"))]))),
        v("v/kpi", "kpi", 610.0, 10.0, 200.0, 150.0)
            .f("Indicator", TOTAL)
            .f("TrendLine", DATE)
            .f("Goal", QTY),
        v("v/newcard", "cardVisual", 820.0, 10.0, 300.0, 150.0)
            .f("Data", TOTAL)
            .f("Data", QTY),
        group("v/group1", "Group 1", 10.0, 200.0, 620.0, 300.0, 0),
        group("v/group2", "Inner group", 10.0, 200.0, 300.0, 300.0, 1).parent(&stable_name("v/group1")),
        v("v/grouped1", "card", 10.0, 200.0, 300.0, 150.0)
            .f("Values", QTY)
            .parent(&stable_name("v/group2")),
        v("v/grouped2", "card", 10.0, 360.0, 300.0, 140.0)
            .f("Values", PRICE)
            .parent(&stable_name("v/group2")),
        v("v/grouped3", "clusteredColumnChart", 320.0, 200.0, 310.0, 300.0)
            .f("Category", REGION)
            .f("Y", TOTAL)
            .parent(&stable_name("v/group1"))
            .s("display", json!({"mode": "hidden"})),
        v("v/hidden", "card", 700.0, 200.0, 200.0, 120.0)
            .f("Values", TOTAL)
            .s("display", json!({"mode": "hidden"}))
            .s("vcObjects", json!({
                "background": props(&[("show", lit("true")), ("color", color("#FAFAFA")), ("transparency", lit("20D"))]),
                "border": props(&[("show", lit("true")), ("color", color("#252423")), ("radius", lit("8D"))]),
                "visualHeader": props(&[("show", lit("false"))]),
                "dropShadow": props(&[("show", lit("true")), ("preset", lit("'BottomRight'"))])})),
        v("v/autotype", "barChart", 920.0, 200.0, 300.0, 200.0)
            .f("Category", REGION)
            .f("Y", QTY)
            .s("autoSelectVisualType", json!(true)),
        v("v/sortonly", "tableEx", 700.0, 420.0, 400.0, 200.0)
            .f("Values", PRODUCT)
            .f("Values", TOTAL)
            .sort(TOTAL, 1)
            .sort(PRODUCT, 2)
            .s("isDrillDisabled", json!(true)),
    ];
    document(
        base_config(),
        Vec::new(),
        vec![charts, tables, misc],
        shared_packages(),
    )
}

fn case_filters() -> Value {
    let mut config = base_config();
    config.insert("filterSortOrder".into(), json!(3));
    let report_filters = vec![
        filter(
            "Filter_rcat",
            REGION,
            "Categorical",
            Some(vec![in_values(REGION, &["'North'", "'South'", "'East'"])]),
            json!({"displayName": "Region (report)", "ordinal": 1,
                   "objects": {"general": props(&[("requireSingleSelect", lit("true")), ("isInvertedSelectionMode", lit("false")), ("unknownProp", lit("1D"))])}}),
        ),
        filter(
            "Filter_radv",
            F::C("Sales", "Amount"),
            "Advanced",
            Some(vec![
                json!({"And": {"Left": cmp(F::C("Sales", "Amount"), 2, "50D"), "Right": cmp(F::C("Sales", "Amount"), 3, "1000D")}}),
            ]),
            json!({"ordinal": 0, "isHiddenInViewMode": true}),
        ),
        filter(
            "Filter_rdate",
            DATE,
            "RelativeDate",
            Some(vec![relative(DATE, 3, 2, 0)]),
            json!({"ordinal": 2, "isLockedInViewMode": true}),
        ),
        filter(
            "Filter_rpass",
            MANAGER,
            "Categorical",
            None,
            json!({"ordinal": 3}),
        ),
        filter(
            "Filter_rtrans",
            CATEGORY,
            "Categorical",
            Some(vec![in_values(CATEGORY, &["'Bikes'"])]),
            json!({"isTransient": true}),
        ),
    ];
    let mut page = section("filters/p1", "Filtered");
    page.config.insert("filterSortOrder".into(), json!(2));
    page.filters = vec![
        top_n("Filter_ptop", PRODUCT, 5, TOTAL, true),
        filter(
            "Filter_pexcl",
            CATEGORY,
            "Categorical",
            Some(vec![
                json!({"Not": {"Expression": in_values(CATEGORY, &["'Clothing'"])}}),
            ]),
            json!({"howCreated": 4}),
        ),
        filter(
            "Filter_ptime",
            DATE,
            "RelativeTime",
            Some(vec![relative(DATE, 2, 4, 4)]),
            json!({}),
        ),
        filter(
            "Filter_pcached",
            REGION,
            "Categorical",
            Some(vec![in_values(REGION, &["'West'"])]),
            json!({"cachedDisplayNames": [{"id": {"scopeId": {"Comparison": {"ComparisonKind": 0,
                "Left": REGION.expr(false), "Right": {"Literal": {"Value": "'West'"}}}}}, "displayName": "West"}]}),
        ),
        filter(
            "Filter_padvtext",
            PRODUCT,
            "Advanced",
            Some(vec![json!({"Or": {
                "Left": {"Contains": {"Left": PRODUCT.expr(true), "Right": {"Literal": {"Value": "'Bikes'"}}}},
                "Right": {"Not": {"Expression": {"StartsWith": {"Left": PRODUCT.expr(true), "Right": {"Literal": {"Value": "'Acc'"}}}}}}}})]),
            json!({}),
        ),
    ];
    page.visuals = vec![
        v("f/table", "tableEx", 10.0, 10.0, 600.0, 300.0)
            .f("Values", REGION)
            .f("Values", PRODUCT)
            .f("Values", TOTAL)
            .s("filterSortOrder", json!(1))
            .filter(filter("Filter_vmeas", TOTAL, "Advanced", Some(vec![cmp(TOTAL, 1, "500D")]), json!({"displayName": "Big sales"})))
            .filter(filter("Filter_vincl", PRODUCT, "Include",
                Some(vec![in_values(PRODUCT, &["'Bikes 1'", "'Bikes 2'"])]), json!({"howCreated": 3})))
            .filter(top_n("Filter_vtop", REGION, 2, QTY, false))
            .filter(filter("Filter_vtuple", REGION, "Tuple", Some(vec![json!({"In": {
                "Expressions": [REGION.expr(true), CATEGORY.expr(true)],
                "Values": [[{"Literal": {"Value": "'North'"}}, {"Literal": {"Value": "'Bikes'"}}]]}})]), json!({}))),
        v("f/chart", "clusteredColumnChart", 620.0, 10.0, 600.0, 300.0)
            .f("Category", CATEGORY)
            .f("Y", TOTAL)
            .filter(filter("Filter_vhier", GEO_REGION, "Categorical",
                Some(vec![in_values(GEO_REGION, &["'North'"])]), json!({"isHiddenInViewMode": true, "isLockedInViewMode": true})))
            .filter(filter("Filter_vagg", AMOUNT, "Advanced", Some(vec![cmp(AMOUNT, 2, "100D")]), json!({"howCreated": 0})))
            .filter(filter("Filter_vdrill", REGION, "Categorical", Some(vec![in_values(REGION, &["'South'"])]), json!({"howCreated": 2}))),
        v("f/slicer", "slicer", 10.0, 320.0, 200.0, 200.0)
            .f("Values", DATE)
            .obj("data", props(&[("mode", lit("'Between'"))]))
            .s("cachedFilterDisplayItems", json!([{"id": {"scopeId": {"Literal": {"Value": "datetime'2025-01-01T00:00:00'"}}}, "displayName": "1/1/2025"}])),
    ];
    let mut page2 = section("filters/p2", "Unfiltered");
    page2.visuals = vec![v("f/card", "card", 10.0, 10.0, 200.0, 120.0).f("Values", QTY)];
    document(config, report_filters, vec![page, page2], shared_packages())
}

fn case_pages() -> Value {
    let mut config = base_config();
    config.insert("activeSectionIndex".into(), json!(2));
    config.insert(
        "objects".into(),
        json!({
        "section": props(&[("verticalAlignment", lit("'Middle'"))]),
        "outspacePane": props(&[("backgroundColor", color("#EEEEEE")), ("width", lit("250L"))])}),
    );
    let image = json!({"expr": {"ResourcePackageItem": {"PackageName": "RegisteredResources", "PackageType": 1,
        "ItemName": "wallpaper5163.png"}}});

    let tooltip_name = format!("ReportSection{}", &stable_name("pages/tooltip")[..12]);
    let drill_name = format!("ReportSection{}", &stable_name("pages/drill")[..12]);

    let mut main = section("pages/main", "Main");
    main.w = 1600.0;
    main.h = 900.0;
    main.display_option = 2;
    main.config.insert("objects".into(), json!({
        "background": props(&[("color", color("#FFFFEE")), ("transparency", lit("10D"))]),
        "outspace": props(&[("color", color("#202020")), ("image", json!({"image": {"name": lit("'wallpaper.png'"), "url": image, "scaling": lit("'Fit'")}}))]),
        "displayArea": props(&[("verticalAlignment", lit("'Top'"))])}));
    let chart = v("p/chart", "clusteredColumnChart", 10.0, 10.0, 500.0, 300.0)
        .f("Category", REGION)
        .f("Y", TOTAL)
        .s(
            "vcObjects",
            val_merge(
                "visualTooltip",
                props(&[
                    ("type", lit("'Canvas'")),
                    ("section", lit(&format!("'{tooltip_name}'"))),
                ]),
            ),
        );
    let card = v("p/card", "card", 520.0, 10.0, 200.0, 120.0).f("Values", QTY);
    let table = v("p/table", "tableEx", 10.0, 320.0, 500.0, 300.0)
        .f("Values", REGION)
        .f("Values", TOTAL);
    let chart_name = chart.name.clone();
    let card_name = card.name.clone();
    let table_name = table.name.clone();
    main.config.insert(
        "relationships".into(),
        json!([
        {"source": chart_name, "target": card_name, "type": 3},
        {"source": chart_name, "target": table_name, "type": 2},
        {"source": table_name, "target": chart_name, "type": 1}]),
    );
    main.visuals = vec![chart, card, table];

    let mut fourthree = section("pages/43", "Four by three");
    fourthree.w = 960.0;
    fourthree.h = 720.0;
    fourthree.display_option = 0;
    fourthree.config.insert("objects".into(), json!({
        "background": props(&[("image", json!({"image": {"name": lit("'wallpaper.png'"), "url": image, "scaling": lit("'Fill'")}})), ("transparency", lit("50D"))])}));
    fourthree.visuals = vec![v("p/c2", "card", 10.0, 10.0, 200.0, 100.0).f("Values", TOTAL)];

    let mut letter = section("pages/letter", "Letter");
    letter.w = 816.0;
    letter.h = 1056.0;
    letter.display_option = 3;
    letter.visuals = vec![v("p/c3", "card", 10.0, 10.0, 200.0, 100.0).f("Values", PRICE)];

    let mut hidden = section("pages/hidden", "Hidden page");
    hidden.config.insert("visibility".into(), json!(1));
    hidden.visuals = vec![v("p/c4", "card", 10.0, 10.0, 200.0, 100.0).f("Values", QTY)];

    let mut tooltip = section("pages/tooltip", "Tooltip page");
    tooltip.w = 320.0;
    tooltip.h = 240.0;
    tooltip.display_option = 3;
    tooltip.config.insert("visibility".into(), json!(1));
    tooltip.config.insert("type".into(), json!(1));
    tooltip.visuals = vec![v("p/tt", "card", 0.0, 0.0, 320.0, 240.0).f("Values", TOTAL)];

    let mut drill = section("pages/drill", "Drillthrough page");
    drill.config.insert("type".into(), json!(2));
    drill.filters = vec![filter(
        "Filter_drill",
        REGION,
        "Categorical",
        None,
        json!({"howCreated": 5}),
    )];
    drill.visuals = vec![
        v("p/back", "actionButton", 10.0, 10.0, 40.0, 40.0)
            .obj("icon", props(&[("shapeType", lit("'back'"))]))
            .s(
                "vcObjects",
                val_merge(
                    "visualLink",
                    props(&[("show", lit("true")), ("type", lit("'Back'"))]),
                ),
            ),
        v("p/dt", "tableEx", 10.0, 60.0, 500.0, 300.0)
            .f("Values", PRODUCT)
            .f("Values", TOTAL),
    ];

    let mut xdrill = section("pages/xdrill", "Cross-report target");
    xdrill.config.insert("type".into(), json!(2));
    xdrill.filters = vec![filter(
        "Filter_xdrill",
        CATEGORY,
        "Categorical",
        None,
        json!({"howCreated": 5}),
    )];
    xdrill.visuals = vec![v("p/xc", "card", 10.0, 10.0, 200.0, 100.0).f("Values", TOTAL)];
    let xdrill_name = xdrill.name.clone();

    let mut doc = document(
        config,
        Vec::new(),
        vec![main, fourthree, letter, hidden, tooltip, drill, xdrill],
        shared_packages(),
    );
    // Display order differs from the array order.
    if let Some(Value::Array(sections)) = doc.get_mut("sections") {
        for (s, ord) in sections.iter_mut().zip([0, 2, 1, 3, 4, 5, 6]) {
            s["ordinal"] = json!(ord);
        }
    }
    doc["pods"] = json!([
        {"name": "Pod_tooltip", "boundSection": tooltip_name, "config": "{}", "parameters": "[]", "type": 2},
        {"name": "Pod_drill", "boundSection": drill_name, "config": json!({"acceptsFilterContext": 1}).to_string(),
         "parameters": json!([{"name": "Sales.Region", "boundFilter": "Filter_drill", "fieldExpr": REGION.expr(false), "asAggregation": false}]).to_string(),
         "type": 1},
        {"name": "Pod_xdrill", "boundSection": xdrill_name, "config": "{}", "referenceScope": 1,
         "parameters": json!([{"name": "Sales.Category", "boundFilter": "Filter_xdrill", "fieldExpr": CATEGORY.expr(false)}]).to_string(),
         "type": 1}]);
    if let Some(Value::Array(p)) = doc.get_mut("resourcePackages") {
        p.push(json!({"resourcePackage": {"name": "RegisteredResources", "type": 1,
            "items": [{"type": 100, "path": "wallpaper5163.png", "name": "wallpaper5163.png"}], "disabled": false}}));
    }
    doc
}

fn case_bookmarks() -> Value {
    let mut config = base_config();
    let mut p1 = section("bm/p1", "Story");
    let chart = v("b/chart", "clusteredColumnChart", 10.0, 10.0, 500.0, 300.0)
        .f("Category", REGION)
        .f("Y", TOTAL);
    let card = v("b/card", "card", 520.0, 10.0, 200.0, 120.0).f("Values", QTY);
    let nav = v("b/nav", "bookmarkNavigator", 10.0, 320.0, 400.0, 60.0)
        .obj("bookmarks", props(&[("bookmarkGroup", lit("'Bookmark_group'"))]));
    let (chart_n, card_n) = (chart.name.clone(), card.name.clone());
    p1.visuals = vec![chart, card, nav];
    let mut p2 = section("bm/p2", "Detail");
    p2.visuals = vec![
        v("b/table", "tableEx", 10.0, 10.0, 500.0, 300.0)
            .f("Values", REGION)
            .f("Values", TOTAL),
    ];
    let (s1, s2) = (p1.name.clone(), p2.name.clone());

    let hidden_card = json!({"version": "1.3", "activeSection": s1,
        "sections": {s1.clone(): {"visualContainers": {card_n.clone(): {"singleVisual": {"visualType": "card", "objects": {}, "display": {"mode": "hidden"}}}}}},
        "objects": {"merge": {"outspacePane": [{"properties": {"expanded": {"expr": {"Literal": {"Value": "false"}}}}}]}}});
    let filtered = json!({"version": "1.3", "activeSection": s1,
        "filters": {"byExpr": [filter("Filter_bm", REGION, "Categorical", Some(vec![in_values(REGION, &["'North'"])]), json!({}))]},
        "sections": {s1.clone(): {"visualContainers": {chart_n.clone(): {"singleVisual": {"visualType": "clusteredColumnChart", "objects": {"merge": {"legend": [{"properties": {"show": {"expr": {"Literal": {"Value": "false"}}}}}]}}}}}}}});
    let on_detail = json!({"version": "1.3", "activeSection": s2, "sections": {s2.clone(): {}}});
    config.insert("bookmarks".into(), json!([
        {"name": "Bookmark_plain", "displayName": "Plain", "explorationState": on_detail,
         "options": {"targetVisualNames": []}},
        {"name": "Bookmark_group", "displayName": "Group", "children": [
            {"name": "Bookmark_hide", "displayName": "Hide card", "explorationState": hidden_card,
             "options": {"suppressData": true, "targetVisualNames": [card_n.clone()], "applyOnlyToTargetVisuals": true}},
            {"name": "Bookmark_filter", "displayName": "Filtered", "explorationState": filtered,
             "options": {"suppressActiveSection": true, "suppressDisplay": true}}]},
        {"name": "Bookmark_last", "displayName": "Last", "explorationState": on_detail.clone(),
         "options": {"preserveNewVisualVisibility": true}}]));
    document(config, Vec::new(), vec![p1, p2], shared_packages())
}

fn case_mobile() -> Result<(Value, Resources), String> {
    let mut p = section("mobile/p1", "Phone");
    p.visuals = vec![
        v("m/card", "card", 10.0, 10.0, 200.0, 120.0)
            .f("Values", TOTAL)
            .phone([0.0, 0.0, 360.0, 160.0])
            .s(
                "mobileObjects",
                val_merge("labels", props(&[("fontSize", lit("30D"))])),
            )
            .s(
                "mobileVcObjects",
                val_merge("title", props(&[("show", lit("false"))])),
            ),
        v("m/chart", "clusteredColumnChart", 220.0, 10.0, 500.0, 300.0)
            .f("Category", REGION)
            .f("Y", TOTAL)
            .phone([0.0, 170.0, 360.0, 300.0]),
        v("m/slicer", "slicer", 730.0, 10.0, 200.0, 200.0)
            .f("Values", CATEGORY)
            .phone([0.0, 480.0, 360.0, 120.0])
            .s(
                "mobileObjects",
                val_merge("data", props(&[("mode", lit("'Dropdown'"))])),
            ),
        v("m/notonphone", "card", 10.0, 320.0, 200.0, 120.0).f("Values", QTY),
    ];
    let page = p.name.clone();
    let chart = stable_name("m/chart");
    let other = stable_name("m/notonphone");
    let mut doc = document(base_config(), Vec::new(), vec![p], shared_packages());
    doc["layoutOptimization"] = json!(0);
    // The phone formatting of the mobile state document (`mobileState.json`).
    let state = json!({"version": "1.0", "activeSection": "", "sections": {page: {"visualContainers": {
        chart: {"singleVisual": {"visualType": "clusteredColumnChart",
            "mobileObjects": val_merge("legend", props(&[("show", lit("false"))])),
            "mobileVcObjects": val_merge("title", props(&[("show", lit("false"))]))}},
        other: {"singleVisual": {"visualType": "card",
            "mobileObjects": val_merge("labels", props(&[("fontSize", lit("18D"))]))}}}}}});
    let bytes = serde_json::to_vec(&state).map_err(|e| e.to_string())?;
    Ok((doc, vec![("mobileState.json".into(), bytes)]))
}

fn case_report() -> Result<(Value, Resources), String> {
    let mut config = base_config();
    config.insert(
        "themeCollection".into(),
        json!({
        "baseTheme": {"name": "CY24SU10", "version": "5.61", "type": 2},
        "customTheme": {"name": "OracleTheme7731.json", "version": "5.61", "type": 1}}),
    );
    config.insert(
        "objects".into(),
        json!({
        "section": props(&[("verticalAlignment", lit("'Top'"))]),
        "outspacePane": props(&[("expanded", lit("false"))])}),
    );
    config.insert("settings".into(), json!({
        "useNewFilterPaneExperience": true, "allowChangeFilterTypes": true, "useStylableVisualContainerHeader": true,
        "exportDataMode": 1, "useEnhancedTooltips": true, "pagesPosition": 1, "queryLimitOption": 6,
        "isPersistentUserStateDisabled": true, "hideVisualContainerHeader": false,
        "useDefaultAggregateDisplayName": true, "filterPaneHiddenInEditMode": false}));
    config.insert("defaultDrillFilterOtherVisuals".into(), json!(true));
    config.insert("modelExtensions".into(), json!([{"name": "extension", "entities": [{"name": "Sales", "extends": "Sales",
        "measures": [{"name": "Report Measure", "dataType": 3, "expression": "[Total Sales] * 2", "errorMessage": null,
                      "hidden": false, "formulaOverride": null, "formatInformation": {"formatString": "#,0", "format": "General"}}]}]}]));
    let mut p = section("report/p1", "Report");
    p.visuals = vec![
        v("r/card", "card", 10.0, 10.0, 200.0, 120.0).f("Values", F::X("Sales", "Report Measure")),
        v("r/table", "tableEx", 220.0, 10.0, 500.0, 300.0)
            .f("Values", REGION)
            .f("Values", F::X("Sales", "Report Measure"))
            .f("Values", TOTAL),
        v("r/image", "image", 10.0, 320.0, 100.0, 100.0).obj(
            "general",
            props(&[(
                "imageUrl",
                json!({"expr": {"ResourcePackageItem": {"PackageName": "RegisteredResources",
                "PackageType": 1, "ItemName": "logo9910.png"}}}),
            )]),
        ),
    ];
    let mut doc = document(config, Vec::new(), vec![p], shared_packages());
    if let Some(Value::Array(pk)) = doc.get_mut("resourcePackages") {
        pk.push(
            json!({"resourcePackage": {"name": "RegisteredResources", "type": 1, "items": [
            {"type": 201, "path": "OracleTheme7731.json", "name": "OracleTheme7731.json"},
            {"type": 100, "path": "logo9910.png", "name": "logo9910.png"}], "disabled": false}}),
        );
    }
    let theme = json!({"name": "Oracle Theme", "dataColors": ["#1B4F72", "#C0392B", "#27AE60", "#F39C12"],
        "background": "#FFFFFF", "foreground": "#252423", "tableAccent": "#1B4F72",
        "visualStyles": {"*": {"*": {"title": [{"fontSize": 12, "bold": true}]}}}});
    let theme_bytes = serde_json::to_vec_pretty(&theme).map_err(|e| e.to_string())?;
    Ok((
        doc,
        vec![
            (
                "StaticResources/RegisteredResources/OracleTheme7731.json".into(),
                theme_bytes,
            ),
            png("StaticResources/RegisteredResources/logo9910.png")?,
        ],
    ))
}

fn case_custom_visual() -> Value {
    let id = "WordCloud1447959067750";
    let mut p = section("cv/p1", "Custom");
    p.visuals = vec![
        v("cv/word", id, 10.0, 10.0, 500.0, 300.0)
            .f("Category", PRODUCT)
            .f("Values", TOTAL)
            .obj("general", props(&[("minFontSize", lit("12D"))])),
        v("cv/card", "card", 520.0, 10.0, 200.0, 120.0).f("Values", TOTAL),
    ];
    let mut doc = document(base_config(), Vec::new(), vec![p], shared_packages());
    doc["publicCustomVisuals"] = json!([id]);
    doc
}

fn case_groups() -> Value {
    let mut config = base_config();
    // Groups are kept only with the stylable container header.
    config.insert(
        "settings".into(),
        json!({"useStylableVisualContainerHeader": true}),
    );
    // A theme version not among the base themes Desktop offers.
    config.insert(
        "themeCollection".into(),
        json!({"baseTheme": {"name": "CY24SU10", "version": "5.99", "type": 2}}),
    );
    let mut p1 = section("groups/p1", "Grouped");
    p1.visuals = vec![
        group("g/outer", "Outer", 100.0, 50.0, 700.0, 400.0, 0),
        group("g/inner", "Inner", 20.0, 30.0, 300.0, 300.0, 1).parent(&stable_name("g/outer")),
        v("g/c1", "card", 10.0, 10.0, 200.0, 120.0)
            .f("Values", TOTAL)
            .parent(&stable_name("g/inner")),
        v("g/c2", "card", 10.0, 140.0, 200.0, 120.0)
            .f("Values", QTY)
            .parent(&stable_name("g/inner")),
        v("g/chart", "clusteredColumnChart", 340.0, 0.0, 300.0, 300.0)
            .f("Category", REGION)
            .f("Y", TOTAL)
            .parent(&stable_name("g/outer")),
        v("g/free", "card", 900.0, 10.0, 200.0, 120.0).f("Values", PRICE),
    ];
    p1.filters = vec![filter(
        "Filter_twocached",
        REGION,
        "Categorical",
        Some(vec![in_values(REGION, &["'North'", "'East'"])]),
        json!({"cachedDisplayNames": [
            {"id": {"scopeId": {"Comparison": {"ComparisonKind": 0, "Left": REGION.expr(false), "Right": {"Literal": {"Value": "'North'"}}}}}, "displayName": "North"},
            {"id": {"scopeId": {"Comparison": {"ComparisonKind": 0, "Left": REGION.expr(false), "Right": {"Literal": {"Value": "'East'"}}}}}, "displayName": "East"}]}),
    )];
    let mut p2 = section("groups/p2", "No ordinal");
    p2.visuals = vec![v("g/c3", "card", 10.0, 10.0, 200.0, 120.0).f("Values", QTY)];
    let mut p3 = section("groups/p3", "Ordinal one");
    p3.visuals = vec![v("g/c4", "card", 10.0, 10.0, 200.0, 120.0).f("Values", TOTAL)];
    let mut doc = document(config, Vec::new(), vec![p1, p2, p3], shared_packages());
    // Ordinals 1, missing, 0: Desktop sorts a missing ordinal as 0.
    if let Some(Value::Array(sections)) = doc.get_mut("sections") {
        if let Some(Value::Object(s)) = sections.get_mut(0) {
            s.insert("ordinal".into(), json!(1));
        }
        if let Some(Value::Object(s)) = sections.get_mut(1) {
            s.shift_remove("ordinal");
        }
        if let Some(Value::Object(s)) = sections.get_mut(2) {
            s.insert("ordinal".into(), json!(0));
        }
    }
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_case_builds_a_legacy_layout() {
        for case in CASES {
            let (doc, _) = layout_of(case).unwrap();
            assert!(
                doc["sections"].as_array().is_some_and(|s| !s.is_empty()),
                "{case}"
            );
            for s in doc["sections"].as_array().unwrap() {
                for c in s["visualContainers"].as_array().unwrap() {
                    let cfg: Value = serde_json::from_str(c["config"].as_str().unwrap()).unwrap();
                    assert!(cfg["name"].as_str().is_some_and(|n| n.len() == 20), "{case}");
                }
            }
        }
    }
}
