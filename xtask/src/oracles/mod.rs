//! Generators and comparers of the Windows-only oracles (formerly Python scripts under `tools/`):
//!
//! - `oracle-gen synth|parts|slicers|states|grids`: the synthetic reports of the render oracle
//!   (tools/render-oracle), written as `bicraft run` command files plus the layout JSON that
//!   `render-compare` reads;
//! - `dax-corpus`: the DAX oracle corpus (crates/dax/tests/oracle/corpus.json);
//! - `metadata-edit`: the TMSL edits of the metadata oracle;
//! - `storage-compare`: `model.storage_stats` against the engine's storage DMVs;
//! - `tmdl-model`: the synthetic TMSL model of the TMDL oracle.
//!
//! The engine side of the metadata, storage and TMDL oracles is run with the maintainers'
//! development oracles, kept outside this repository; these subcommands are pure data transforms.
//!
//! The outputs keep the layout the Python scripts wrote (see `pyjson`), with LF line ends.

mod dax_corpus;
mod dax_corpus_data;
mod decomp;
#[cfg(test)]
mod decomp_tests;
mod expand;
mod grids;
mod interactions;
mod interactions_collect;
#[cfg(test)]
mod interactions_tests;
mod kpis;
mod layout_oracle;
mod metadata_edits;
mod parts;
mod pyjson;
mod slicers;
mod states;
mod storage_compare;
mod synth;
#[cfg(test)]
#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;
mod tmdl_model;

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

pub use dax_corpus::cmd as cmd_dax_corpus;
pub use layout_oracle::cmd as cmd_layout_oracle;
pub use metadata_edits::cmd as cmd_metadata_edit;
pub use storage_compare::cmd as cmd_storage_compare;
pub use tmdl_model::cmd as cmd_tmdl_model;

pub const GEN_USAGE: &str = "\
usage: cargo xtask oracle-gen <set> ARGS   (tools/render-oracle/README.md)
  synth OUT.pbix CMDS.json LAYOUT.json   one page per visual family (set `synth`)
  grids OUT.pbix CMDS.json LAYOUT.json   tables and matrices (set `grids`)
  parts DIR [patch]                      pie / donut / gauge / cards / KPI / funnel / treemap
  slicers DIR [patch]                    every slicer mode
  expand DIR [patch]                     expanded matrices, per-level subtotals (set `expand`)
  kpis DIR [patch]                       KPI display units per format and magnitude (set `kpis`)
  states DIR [check [CAPTURE_DIR]]       state-selector check (docs/formats/visual-options.md)
  interactions DIR [patch | collect [OUT]]
                                         interaction oracle (docs/interaction-oracle.md)
  decomp DIR [patch | collect [OUT]]     decomposition tree oracle (docs/render.md)
";

/// `cargo xtask oracle-gen`.
pub fn cmd_gen(args: &[String]) -> Result<(), String> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["synth", out, cmds, layout] => synth::run(out, Path::new(cmds), Path::new(layout)),
        ["grids", out, cmds, layout] => grids::run(out, Path::new(cmds), Path::new(layout)),
        ["parts", dir] => parts::run(dir, false),
        ["parts", dir, "patch"] => parts::run(dir, true),
        ["expand", dir] => expand::run(dir, false),
        ["expand", dir, "patch"] => expand::run(dir, true),
        ["kpis", dir] => kpis::run(dir, false),
        ["kpis", dir, "patch"] => kpis::run(dir, true),
        ["slicers", dir] => slicers::run(dir, false),
        ["slicers", dir, "patch"] => slicers::run(dir, true),
        ["states", dir] => states::gen_cmds(dir),
        ["states", dir, "check"] => states::check(dir, &Path::new(dir).join("desktop")),
        ["states", dir, "check", cap] => states::check(dir, Path::new(cap)),
        ["interactions", rest @ ..] => interactions::cmd(rest),
        ["decomp", rest @ ..] => decomp::cmd(rest),
        _ => Err(GEN_USAGE.into()),
    }
}

/// A `bicraft run --file` command list.
#[derive(Default)]
pub struct Cmds(pub Vec<Value>);

impl Cmds {
    pub fn c(&mut self, cmd: &str, params: Value) {
        self.0.push(json!({"command": cmd, "params": params}));
    }

    pub fn value(self) -> Value {
        Value::Array(self.0)
    }
}

/// `[{"role": r, "field": f}, ...]`.
pub fn fields(pairs: &[(&str, &str)]) -> Value {
    Value::Array(
        pairs
            .iter()
            .map(|(r, f)| json!({"role": r, "field": f}))
            .collect(),
    )
}

/// `{**a, **b}`: keys of `b` replace those of `a` in place or come after them.
pub fn merged(a: Value, b: Value) -> Value {
    let mut m = match a {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    if let Value::Object(b) = b {
        for (k, v) in b {
            m.insert(k, v);
        }
    }
    Value::Object(m)
}

/// The name `report.apply` / `visual.add` give a page or visual from its `key`
/// (crates/engine/src/report_apply.rs `stable_name`).
pub fn stable_name(text: &str) -> String {
    const PRIME: u64 = 0x100000001b3;
    fn fnv(basis: u64, bs: &[u8]) -> u64 {
        bs.iter()
            .fold(basis, |h, b| (h ^ u64::from(*b)).wrapping_mul(PRIME))
    }
    fn mix(z: u64) -> u64 {
        let z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        let z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    let bs = text.as_bytes();
    let o = 0xcbf29ce484222325u64;
    format!(
        "{:016x}{:04x}",
        mix(fnv(o, bs)),
        mix(fnv(o ^ 0x9e3779b97f4a7c15, bs)) >> 48
    )
}

/// `os.path.join(dir, parts...)` as a string.
pub fn join(dir: &str, parts: &[&str]) -> String {
    let mut p = PathBuf::from(dir);
    for x in parts {
        p.push(x);
    }
    p.to_string_lossy().into_owned()
}

pub fn write_text(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// `json.dump(v, open(path, "w"), indent=indent)` (ASCII escapes, no final newline).
pub fn write_json(path: &Path, v: &Value, indent: usize) -> Result<(), String> {
    write_text(path, &pyjson::dumps(v, Some(indent), true))
}

/// Reads a JSON file (a UTF-8 byte-order mark is skipped).
pub fn read_json(path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    serde_json::from_slice(bytes).map_err(|e| format!("{}: bad JSON: {e}", path.display()))
}

/// The two-step generators (parts, slicers): one visual and the visual objects patched into
/// its PBIR afterwards.
pub struct Vis {
    pub page: &'static str,
    pub name: &'static str,
    pub ty: &'static str,
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub fields: Value,
    pub objects: Value,
}

/// Distinct pages in order of first appearance.
pub fn pages_of<'a>(pages: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    for p in pages {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// The `report.apply` spec of a visual list.
pub fn apply_spec(vis: &[Vis]) -> Value {
    let pages = pages_of(vis.iter().map(|v| v.page));
    let pages: Vec<Value> = pages
        .iter()
        .map(|pg| {
            let visuals: Vec<Value> = vis
                .iter()
                .filter(|v| v.page == *pg)
                .map(|v| {
                    json!({"name": v.name, "type": v.ty, "x": v.x, "y": v.y,
                           "width": v.w, "height": v.h, "fields": v.fields})
                })
                .collect();
            json!({"key": pg, "display_name": pg, "width": 1280, "height": 720,
                   "visuals": visuals})
        })
        .collect();
    json!({"pages": pages})
}

/// Writes cmds1.json (model, report, project), cmds2.json (project -> `pbix`) and layout.json.
pub fn two_step_gen(dir: &str, mut c: Cmds, vis: &[Vis], pbix: &str) -> Result<(), String> {
    let d = Path::new(dir);
    std::fs::create_dir_all(d).map_err(|e| format!("{dir}: {e}"))?;
    let pages = pages_of(vis.iter().map(|v| v.page));
    c.c("report.apply", json!({"spec": apply_spec(vis), "prune": "all"}));
    c.c(
        "project.new",
        json!({"dir": join(dir, &["proj"]), "name": "Synth"}),
    );
    write_json(&d.join("cmds1.json"), &c.value(), 1)?;
    let cmds2 = json!([
        {"command": "file.open", "params": {"path": join(dir, &["proj", "Synth.pbip"])}},
        {"command": "file.save_as", "params": {"path": join(dir, &[pbix]), "overwrite": true}}
    ]);
    write_json(&d.join("cmds2.json"), &cmds2, 1)?;
    let layout: Vec<Value> = vis
        .iter()
        .map(|v| {
            let page = pages.iter().position(|p| *p == v.page).unwrap_or(0);
            json!({"page": page, "page_name": v.page, "type": v.name, "x": v.x, "y": v.y,
                   "width": v.w, "height": v.h})
        })
        .collect();
    write_json(&d.join("layout.json"), &Value::Array(layout), 1)?;
    println!("pages: {}", pages.join(","));
    Ok(())
}

/// Writes each visual's objects into `DIR/proj/Synth.Report/definition/pages/*/visuals/*/
/// visual.json`. With `card_default`, the new card's entries get the `default` state selector
/// (it reads its formatting from that state).
pub fn two_step_patch(dir: &str, vis: &[Vis], card_default: bool) -> Result<(), String> {
    let pages = Path::new(dir)
        .join("proj")
        .join("Synth.Report")
        .join("definition")
        .join("pages");
    let mut files = Vec::new();
    for p in sorted_dirs(&pages)? {
        for v in sorted_dirs(&p.join("visuals"))? {
            let f = v.join("visual.json");
            if f.is_file() {
                files.push(f);
            }
        }
    }
    for f in files {
        let mut j = read_json(&f)?;
        let name = j.get("name").and_then(Value::as_str).unwrap_or_default();
        let Some(v) = vis.iter().find(|v| v.name == name) else {
            continue;
        };
        let Some(objs) = v.objects.as_object().filter(|o| !o.is_empty()) else {
            continue;
        };
        let mut objs = objs.clone();
        let Some(visual) = j.get_mut("visual").and_then(Value::as_object_mut) else {
            return Err(format!("{}: no `visual`", f.display()));
        };
        if card_default && visual.get("visualType").and_then(Value::as_str) == Some("cardVisual") {
            for es in objs.values_mut() {
                for e in es.as_array_mut().into_iter().flatten() {
                    if let Some(e) = e.as_object_mut() {
                        e.insert("selector".into(), json!({"id": "default"}));
                    }
                }
            }
        }
        // `$visual`: keys of the PBIR `visual` object itself (`expansionStates`)
        if let Some(Value::Object(keys)) = objs.remove("$visual") {
            visual.extend(keys);
        }
        // `$active`: roles whose projections are all marked `active` (a matrix hierarchy that
        // Desktop shows expanded rather than drilled to its top level)
        if let Some(Value::Array(roles)) = objs.remove("$active") {
            for role in roles.iter().filter_map(Value::as_str) {
                let ps = visual
                    .get_mut("query")
                    .and_then(|q| q.get_mut("queryState"))
                    .and_then(|s| s.get_mut(role))
                    .and_then(|r| r.get_mut("projections"))
                    .and_then(Value::as_array_mut);
                for p in ps.into_iter().flatten() {
                    if let Some(p) = p.as_object_mut() {
                        p.insert("active".into(), Value::Bool(true));
                    }
                }
            }
        }
        // `$activeAt`: {role: k}: only the k-th projection active (drilled down to that level)
        if let Some(Value::Object(at)) = objs.remove("$activeAt") {
            for (role, k) in &at {
                let ps = visual
                    .get_mut("query")
                    .and_then(|q| q.get_mut("queryState"))
                    .and_then(|s| s.get_mut(role.as_str()))
                    .and_then(|r| r.get_mut("projections"))
                    .and_then(Value::as_array_mut);
                for (i, p) in ps.into_iter().flatten().enumerate() {
                    if let Some(p) = p.as_object_mut() {
                        let on = k.as_u64() == u64::try_from(i).ok();
                        p.insert("active".into(), Value::Bool(on));
                    }
                }
            }
        }
        if objs.is_empty() {
            write_json(&f, &j, 2)?;
            println!("patched {}", v.name);
            continue;
        }
        let target = visual
            .entry("objects")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(t) = target.as_object_mut() {
            t.extend(objs);
        }
        write_json(&f, &j, 2)?;
        println!("patched {}", v.name);
    }
    Ok(())
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

/// The sine-shaped amounts of the synthetic sales tables:
/// `round(1000*(5-ri)*(1+ci*0.6)+300*sin(m*0.7+ri)+40*m, 2)`.
pub fn wave_amount(ri: i64, ci: i64, m: i64) -> f64 {
    let base = (1000 * (5 - ri)) as f64 * (1.0 + ci as f64 * 0.6);
    let wave = 300.0 * (m as f64 * 0.7 + ri as f64).sin();
    pyjson::round2(base + wave + (40 * m) as f64)
}

pub use pyjson::float_repr as py_float;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_name_is_the_engine_one() {
        // same function as crates/engine/src/report_apply.rs; values of the former Python port
        assert_eq!(stable_name("Cards"), "031c5288e6b1fe7047bf");
        assert_eq!(
            stable_name(&format!("{}/v1", stable_name("Stress"))),
            "358186ee05f923595c40"
        );
    }
}
