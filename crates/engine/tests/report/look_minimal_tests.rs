//! BIcraft's look keeps its Minimal preset the earlier drawing (docs/formats/visual-look.md):
//! every golden case of the fourteen visuals (chart visuals, chart pack, Spec visual) laid out under
//! Minimal hashes to what it hashed before the look's second pass (main 8e665c46, 2026-11-24),
//! so a change to Modern or Bold cannot move a Minimal primitive. The hashes are in
//! `look_minimal.json`; `BICRAFT_BLESS_MINIMAL=1` rewrites them, only for a deliberate change of
//! the Minimal drawing. The Sankey cases whose links skip a column are the one deliberate change
//! since: such links now pass through waypoints in the columns between (a layout fix, G113, made
//! under every preset), listed in [`ROUTED`]; and the cases that ask for BIcraft's colours, which
//! draw its new palette ([`BICRAFT_COLORS`]); and the slope chart whose labels moved apart, which
//! draws leader lines to them (G110, [`LEADERS`]); and the Spec cases over two measures whose
//! marks read only the first, which now draw the second as its own series (G119, [`SERIES`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bicraft_render::{bvis, chart_pack, spec};
use serde_json::{Value, json};

/// Cases drawn differently under Minimal on purpose since the hashes were taken: Sankey links
/// that skip a column routed through waypoints (G113, 2026-11-24).
const ROUTED: &[&str] = &[
    "chart_pack/look_sankey_bold",
    "chart_pack/look_sankey_theme",
    "chart_pack/sankey_basic",
    "chart_pack/sankey_options",
    "chart_pack/sankey_fixed_no_labels",
];

/// Cases that ask for BIcraft's colours (`palette: bicraft`) under Minimal: they draw BIcraft's
/// palette, whose colours changed with the look's second pass (2026-11-24); without that option
/// Minimal keeps the earlier palette.
const BICRAFT_COLORS: &[&str] = &["bvis/look_dumbbell_bicraft", "spec/look_combo_flat"];

/// Cases whose labels are moved from their dots: since 1.3.1.0 each moved label has a thin leader
/// line under every preset (G110, 2026-11-24).
const LEADERS: &[&str] = &["bvis/slope_crowded"];

/// Spec cases over two measures whose marks are plain series of the first: since 1.5.0.0 each
/// other measure is drawn as its own series, with a legend when there is room (G119, 2026-11-25).
const SERIES: &[&str] = &[
    "spec/area_time",
    "spec/bar",
    "spec/by_category",
    "spec/look_bar_bold",
    "spec/no_axes_tiny",
    "spec/x_text_rule",
];

/// FNV-1a, 64 bits, of the layout's JSON text.
fn fnv(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Sets `obj[key]` to `v`, making `obj` an object first when it is not one.
fn put(obj: &mut Value, key: &str, v: Value) {
    if !obj.is_object() {
        *obj = json!({});
    }
    if let Some(o) = obj.as_object_mut() {
        o.insert(key.into(), v);
    }
}

/// Every golden case laid out under Minimal, as `family/name → hash`.
fn minimal_hashes() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (name, kind, mut input, w, h) in crate::chart_pack_tests::cases() {
        let mut o = input.get("options").cloned().unwrap_or(Value::Null);
        put(&mut o, "stylePreset", json!("minimal"));
        put(&mut input, "options", o);
        let got = chart_pack::layout(kind, &input, w, h).json();
        out.insert(format!("chart_pack/{name}"), fnv(&got.to_string()));
    }
    for (name, kind, data, mut opts, w, h) in crate::bvis_tests::cases() {
        let mut s = opts.get("style").cloned().unwrap_or(Value::Null);
        put(&mut s, "preset", json!("minimal"));
        put(&mut opts, "style", s);
        let got = bvis::layout_json(kind, &bvis::Data::from_json(&data), &opts, w, h);
        out.insert(format!("bvis/{name}"), fnv(&got.to_string()));
    }
    for (name, mut sp, data, w, h) in crate::spec_visual_tests::cases() {
        if w <= 0.0 {
            continue;
        }
        let mut s = sp.get("style").cloned().unwrap_or(Value::Null);
        put(&mut s, "preset", json!("minimal"));
        put(&mut sp, "style", s);
        let got = match spec::parse(&sp) {
            Ok(s) => spec::layout_json(&s, &spec::SpecData::from_json(&data), w, h),
            Err(e) => json!({"error": e}),
        };
        out.insert(format!("spec/{name}"), fnv(&got.to_string()));
    }
    out
}

fn hashes_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/report/look_minimal.json")
}

#[test]
fn minimal_draws_what_it_drew_before() {
    let got = minimal_hashes();
    let path = hashes_path();
    if bicraft_env::var_os("BICRAFT_BLESS_MINIMAL").is_some() {
        let text = serde_json::to_string_pretty(&got).unwrap();
        std::fs::write(&path, text + "\n").unwrap();
    }
    let want: BTreeMap<String, String> = serde_json::from_str(
        &std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e} (BICRAFT_BLESS_MINIMAL=1 writes it)", path.display())),
    )
    .unwrap();
    let mut errors = Vec::new();
    for (name, hash) in &got {
        match want.get(name) {
            // a case added since the hashes were taken has none
            None => {}
            Some(w)
                if ROUTED.contains(&name.as_str())
                    || BICRAFT_COLORS.contains(&name.as_str())
                    || LEADERS.contains(&name.as_str())
                    || SERIES.contains(&name.as_str()) =>
            {
                assert_ne!(
                    w, hash,
                    "{name} is listed as changed on purpose but draws as before"
                );
            }
            Some(w) if w != hash => errors.push(format!("{name}: Minimal changed")),
            Some(_) => {}
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
    // the hashes cover every case they were taken for
    for name in want.keys() {
        assert!(got.contains_key(name), "{name}: no longer a golden case");
    }
}
