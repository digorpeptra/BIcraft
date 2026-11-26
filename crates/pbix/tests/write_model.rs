//! `write_model` on synthetic packages: a PBIX whose `DataModel` is built here around the
//! synthetic `metadata.sqlitedb` of the metadata crate, and a PBIT.

use bicraft_model::{Measure, Role, TablePermission};
use bicraft_opc::{Encoding, Package, encode_text};
use bicraft_pbix::{DATA_MODEL, DATA_MODEL_SCHEMA, Error, read_model, synthetic, write_model};

const METADATA: &[u8] = include_bytes!("../../metadata/tests/fixtures/synthetic_metadata.sqlitedb");

fn pbix() -> Package {
    // through the ZIP writer and reader, like a file on disk
    Package::read_bytes(&synthetic::pbix(METADATA).unwrap().to_bytes().unwrap()).unwrap()
}

/// Every part but `changed` is byte-identical, same order and compression flag.
fn assert_only_changed(before: &Package, after: &Package, changed: &str) {
    let names = |p: &Package| p.parts.iter().map(|x| x.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(before), names(after));
    for (a, b) in before.parts.iter().zip(&after.parts) {
        if a.name == changed {
            assert_eq!(a.stored, b.stored, "{}", a.name);
        } else {
            assert_eq!(a, b, "{} changed", a.name);
        }
    }
}

#[test]
fn identity_save_keeps_the_model() {
    let pkg = pbix();
    let model = read_model(&pkg).unwrap();
    let out = write_model(&pkg, &model).unwrap();
    assert_only_changed(&pkg, &out, DATA_MODEL);
    assert!(out.get(DATA_MODEL).unwrap().stored);
    let reopened = Package::read_bytes(&out.to_bytes().unwrap()).unwrap();
    assert_eq!(read_model(&reopened).unwrap(), model);
    // the backup is unchanged: same ABF once decompressed
    let abf = |p: &Package| {
        bicraft_xpress9::decompress_datamodel(&p.get(DATA_MODEL).unwrap().data, 1 << 30).unwrap()
    };
    assert_eq!(abf(&reopened), abf(&pkg));
}

#[test]
fn an_added_measure_survives_save_and_reopen() {
    let pkg = pbix();
    let mut model = read_model(&pkg).unwrap();
    model.table_mut("Sales").unwrap().measures.push(Measure {
        name: "bicraft test".into(),
        expression: "1".into(),
        ..Default::default()
    });
    model
        .table_mut("Sales")
        .unwrap()
        .measure_mut("Total")
        .unwrap()
        .format_string = Some("0.0%".into());
    model.roles.push(Role {
        name: "bicraft role".into(),
        model_permission: Some("read".into()),
        table_permissions: vec![TablePermission {
            table: "Calendar".into(),
            filter_expression: Some("TRUE()".into()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let out = write_model(&pkg, &model).unwrap();
    assert_only_changed(&pkg, &out, DATA_MODEL);
    let reopened = Package::read_bytes(&out.to_bytes().unwrap()).unwrap();
    let back = read_model(&reopened).unwrap();
    let (table, m) = back.measure("bicraft test").unwrap();
    assert_eq!((table.name.as_str(), m.expression.as_str()), ("Sales", "1"));
    assert_eq!(
        back.measure("Total").unwrap().1.format_string.as_deref(),
        Some("0.0%")
    );
    assert!(back.roles.iter().any(|r| r.name == "bicraft role"));
    // the backup's other files are untouched
    let abf_bytes =
        bicraft_xpress9::decompress_datamodel(&reopened.get(DATA_MODEL).unwrap().data, 1 << 30).unwrap();
    let abf = bicraft_abf::Abf::parse(&abf_bytes).unwrap();
    assert_eq!(abf.read("0.CryptKey.bin"), Some(&b"\x01\x02\x03"[..]));
    // saving the reopened model again changes nothing in the model
    let again = write_model(&reopened, &back).unwrap();
    assert_eq!(read_model(&again).unwrap(), back);
}

#[test]
fn unsupported_changes_are_refused_with_a_list() {
    let pkg = pbix();
    let mut model = read_model(&pkg).unwrap();
    model.tables.retain(|t| t.name != "Calendar");
    match write_model(&pkg, &model) {
        Err(e @ Error::UnsupportedChange(_)) => {
            let msg = e.to_string();
            assert!(msg.contains("Calendar"), "{msg}");
        }
        other => panic!("expected UnsupportedChange, got {other:?}"),
    }
}

#[test]
fn bare_abf_parts_stay_bare() {
    let backup = synthetic::abf(&[("metadata.sqlitedb", METADATA)]).unwrap();
    let mut pkg = Package::default();
    pkg.put(DATA_MODEL, backup.clone(), true);
    let mut model = read_model(&pkg).unwrap();
    model.table_mut("Sales").unwrap().measures[0].description = Some("Counted".into());
    let out = write_model(&pkg, &model).unwrap();
    let part = &out.get(DATA_MODEL).unwrap().data;
    assert_eq!(&part[..2], &[0xFF, 0xFE]);
    assert_eq!(read_model(&out).unwrap(), model);
}

#[test]
fn templates_get_tmsl_in_the_original_encoding() {
    let schema =
        r#"{"name":"m","model":{"tables":[{"name":"T","measures":[{"name":"M","expression":"1"}]}]}}"#;
    let mut pkg = Package::default();
    pkg.put("Version", encode_text("1.28", Encoding::Utf16Le), false);
    pkg.put(DATA_MODEL_SCHEMA, encode_text(schema, Encoding::Utf16Le), false);
    pkg.put("Settings", encode_text("{}", Encoding::Utf16Le), false);
    let mut model = read_model(&pkg).unwrap();
    model.tables[0].measures.push(Measure {
        name: "N".into(),
        expression: "2".into(),
        ..Default::default()
    });
    let out = write_model(&pkg, &model).unwrap();
    assert_only_changed(&pkg, &out, DATA_MODEL_SCHEMA);
    let part = out.get(DATA_MODEL_SCHEMA).unwrap();
    assert_eq!(
        bicraft_opc::decode_text(&part.name, &part.data).unwrap().1,
        Encoding::Utf16Le
    );
    assert_eq!(read_model(&out).unwrap(), model);
}

#[test]
fn packages_without_a_model_or_with_garbage_are_errors() {
    let model = read_model(&pbix()).unwrap();
    assert!(matches!(
        write_model(&Package::default(), &model),
        Err(Error::NoModel)
    ));
    let mut pkg = Package::default();
    pkg.put(DATA_MODEL, vec![0; 200], true);
    assert!(write_model(&pkg, &model).is_err());
}

/// F3 (docs/fuzzing.md): a `DataModel` part may expand to at most `max_ratio` × its size (with a
/// floor for small models), checked from the declared sizes before decompressing.
#[test]
fn data_model_expansion_is_capped_relative_to_the_part() {
    use bicraft_pbix::{DataModelLimits, read_data_model, read_data_model_with};

    let zeros = vec![0u8; 4 << 20];
    let part = bicraft_xpress9::compress_datamodel(&zeros, None).unwrap();
    let tight = DataModelLimits {
        min_allowance: 1 << 20,
        max_ratio: 100,
        ..DataModelLimits::default()
    };
    assert!(part.len() * 100 < zeros.len(), "part of {} bytes", part.len());
    match read_data_model_with(&part, &tight) {
        Err(Error::DataModelTooLarge {
            part: p,
            declared,
            limit,
        }) => {
            assert_eq!((p, declared), (part.len(), 4 << 20));
            assert_eq!(limit, 1 << 20);
        }
        other => panic!("expected DataModelTooLarge, got {other:?}"),
    }
    // Raising the ratio lets it decompress (the zeros are then not an ABF).
    let loose = DataModelLimits {
        max_ratio: 100_000,
        ..tight
    };
    assert!(matches!(read_data_model_with(&part, &loose), Err(Error::Abf(_))));
    // Default limits: a 64 MiB floor, so this small model passes the size check.
    assert!(matches!(read_data_model(&part), Err(Error::Abf(_))));

    // The F3 shape: a few KB declaring far more than the floor, refused before decoding.
    let mut lying = part.clone();
    let at = bicraft_xpress9::datamodel::BANNER_SIZE;
    lying[at..at + 4].copy_from_slice(&(256u32 << 20).to_le_bytes());
    assert!(matches!(
        read_data_model(&lying),
        Err(Error::DataModelTooLarge { limit, .. }) if limit == 64 << 20
    ));
    assert_eq!(DataModelLimits::default().limit_for(1 << 20), 1000 << 20);
    assert_eq!(DataModelLimits::default().limit_for(1000), 64 << 20);
    assert_eq!(DataModelLimits::default().limit_for(100 << 20), 4 << 30);
}
