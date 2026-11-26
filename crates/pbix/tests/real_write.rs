//! Metadata-only saves of a real Desktop PBIX: local only, never committed.
//!
//! Reads `$BICRAFT_TESTDATA/sample1/report.pbix` and writes four edited copies to
//! `$BICRAFT_TESTDATA/m5/` (`identity.pbix`, `add_measure.pbix`, `format.pbix`, `rls.pbix`)
//! for a manual check in Power BI Desktop. Without the variable or the file the test prints a
//! note and passes. Only structural facts and the output paths are printed, never content.

use std::path::{Path, PathBuf};

use bicraft_model::{Measure, Role, SemanticModel, TablePermission};
use bicraft_opc::Package;
use bicraft_pbix::{DATA_MODEL, read_model, write_model};

const SAMPLE: &str = "sample1/report.pbix";

fn sample() -> Option<(PathBuf, PathBuf)> {
    let Some(dir) = bicraft_env::var_os("BICRAFT_TESTDATA").map(PathBuf::from) else {
        eprintln!("note: BICRAFT_TESTDATA is not set; skipping the real PBIX write checks");
        return None;
    };
    let file = dir.join(SAMPLE);
    if !file.exists() {
        eprintln!("note: {SAMPLE} not found under BICRAFT_TESTDATA; skipping");
        return None;
    }
    Some((file, dir.join("m5")))
}

fn abf_of(pkg: &Package) -> Vec<u8> {
    let part = &pkg.get(DATA_MODEL).expect("DataModel").data;
    bicraft_xpress9::decompress_datamodel(part, 4 << 30).expect("decompress")
}

/// Writes `pkg` to `out/name`, reopens it and returns the reopened package and model.
fn save(pkg: &Package, out: &Path, name: &str) -> (Package, SemanticModel) {
    let path = out.join(name);
    std::fs::write(&path, pkg.to_bytes().expect("zip")).expect("write");
    let reopened = Package::open(&path).expect("reopen");
    let model = read_model(&reopened).expect("read back");
    println!("wrote {}", path.display());
    (reopened, model)
}

/// Every part but `DataModel` is identical (name, order, bytes, compression).
fn assert_other_parts_identical(a: &Package, b: &Package) {
    assert_eq!(a.parts.len(), b.parts.len());
    for (x, y) in a.parts.iter().zip(&b.parts) {
        assert_eq!(x.name, y.name);
        if x.name != DATA_MODEL {
            assert!(x == y, "part {} changed", x.name);
        } else {
            assert!(y.stored, "DataModel must be stored");
        }
    }
}

#[test]
fn real_pbix_metadata_saves() {
    let Some((file, out)) = sample() else { return };
    std::fs::create_dir_all(&out).expect("create m5 output folder");
    let pkg = Package::open(&file).expect("open sample");
    let model = read_model(&pkg).expect("read sample");
    let original_abf = abf_of(&pkg);

    // (a) identity
    let t = std::time::Instant::now();
    let written = write_model(&pkg, &model).expect("identity save");
    eprintln!("identity write_model: {:?}", t.elapsed());
    assert_other_parts_identical(&pkg, &written);
    let (reopened, back) = save(&written, &out, "identity.pbix");
    assert_eq!(back, model, "identity: model differs");
    assert!(abf_of(&reopened) == original_abf, "identity: the backup changed");
    let (a, b) = (
        bicraft_xpress9::inspect_datamodel(&pkg.get(DATA_MODEL).unwrap().data).unwrap(),
        bicraft_xpress9::inspect_datamodel(&reopened.get(DATA_MODEL).unwrap().data).unwrap(),
    );
    assert_eq!(a.banner, b.banner);
    assert_eq!(a.chunks.len(), b.chunks.len());
    // the default (fast) mode searches less deeply than Desktop; the exact mode gives
    // Desktop's size back
    let exact = bicraft_xpress9::datamodel::compress_with_plan(
        &original_abf,
        &bicraft_xpress9::FramingPlan::new(Some(&a)).with_mode(bicraft_xpress9::CompressionMode::Exact),
    )
    .unwrap();
    assert_eq!(
        exact.len(),
        pkg.get(DATA_MODEL).unwrap().data.len(),
        "identity: the exact re-framing has Desktop's size"
    );

    // (b) a new measure in an existing table
    let home = model
        .tables
        .iter()
        .position(|t| !t.measures.is_empty())
        .unwrap_or(0);
    let mut edited = model.clone();
    edited.tables[home].measures.push(Measure {
        name: "bicraft test".into(),
        expression: "1".into(),
        ..Default::default()
    });
    let written = write_model(&pkg, &edited).expect("add measure");
    assert_other_parts_identical(&pkg, &written);
    let (_, back) = save(&written, &out, "add_measure.pbix");
    let (table, m) = back.measure("bicraft test").expect("new measure present");
    assert_eq!(table.name, model.tables[home].name);
    assert_eq!(m.expression, "1");
    let mut expected = edited.clone();
    if let Some(x) = expected.tables[home].measures.last_mut() {
        x.lineage_tag = m.lineage_tag.clone();
    }
    assert_eq!(back, expected, "add measure: something else changed");

    // (c) a format string change
    let mut edited = model.clone();
    let (ti, mi) = edited
        .tables
        .iter()
        .enumerate()
        .find_map(|(i, t)| (!t.measures.is_empty()).then_some((i, 0)))
        .expect("the sample has a measure");
    let target = &mut edited.tables[ti].measures[mi];
    let new_format = if target.format_string.as_deref() == Some("0.00") {
        "#,0"
    } else {
        "0.00"
    };
    target.format_string = Some(new_format.into());
    let written = write_model(&pkg, &edited).expect("format string");
    assert_other_parts_identical(&pkg, &written);
    let (_, back) = save(&written, &out, "format.pbix");
    assert_eq!(back, edited, "format string: model differs");

    // (d) an RLS role with a filter
    let mut edited = model.clone();
    let table = edited.tables[home].name.clone();
    edited.roles.push(Role {
        name: "bicraft test role".into(),
        model_permission: Some("read".into()),
        table_permissions: vec![TablePermission {
            table,
            filter_expression: Some("TRUE()".into()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let written = write_model(&pkg, &edited).expect("rls");
    assert_other_parts_identical(&pkg, &written);
    let (_, back) = save(&written, &out, "rls.pbix");
    assert_eq!(back, edited, "rls: model differs");
}
