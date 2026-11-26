//! `write_sqlitedb`: model → rewritten `metadata.sqlitedb` bytes, over the synthetic database of
//! gen_fixture.py.

use bicraft_metadata::{
    Error, MetaValue, read_sqlitedb, tables_from_sqlitedb, write_sqlitedb, write_sqlitedb_at,
};
use bicraft_model::{Measure, Role, TablePermission};
use bicraft_sqlite::Database;

const FIXTURE: &[u8] = include_bytes!("fixtures/synthetic_metadata.sqlitedb");
/// A fixed FILETIME clock (2026-01-01T00:00:00Z) so outputs are deterministic.
const NOW: i64 = 134_116_992_000_000_000;

fn header_u32(bytes: &[u8], at: usize) -> u32 {
    let w: [u8; 4] = bytes[at..at + 4].try_into().unwrap();
    u32::from_be_bytes(w)
}

#[test]
fn an_unchanged_model_gives_the_original_bytes() {
    let m = read_sqlitedb(FIXTURE).unwrap();
    assert_eq!(write_sqlitedb(FIXTURE, &m).unwrap(), FIXTURE);
}

#[test]
fn an_added_measure_is_written_and_reads_back() {
    let mut m = read_sqlitedb(FIXTURE).unwrap();
    m.table_mut("Sales").unwrap().measures.push(Measure {
        name: "bicraft test".into(),
        expression: "1".into(),
        format_string: Some("0".into()),
        ..Default::default()
    });
    let out = write_sqlitedb_at(FIXTURE, &m, NOW).unwrap();
    assert_eq!(write_sqlitedb_at(FIXTURE, &m, NOW).unwrap(), out, "deterministic");

    let back = read_sqlitedb(&out).unwrap();
    let (table, measure) = back.measure("bicraft test").unwrap();
    assert_eq!(table.name, "Sales");
    assert_eq!(measure.expression, "1");
    assert_eq!(measure.format_string.as_deref(), Some("0"));
    // everything else is as before (the lineage tag of the new measure was generated)
    let mut expected = m.clone();
    expected
        .table_mut("Sales")
        .unwrap()
        .measure_mut("bicraft test")
        .unwrap()
        .lineage_tag = measure.lineage_tag.clone();
    assert_eq!(back, expected);

    // only Measure and DBPROPERTIES (MAXID) changed; other tables keep their rows
    let (before, after) = (
        tables_from_sqlitedb(FIXTURE).unwrap(),
        tables_from_sqlitedb(&out).unwrap(),
    );
    for &name in bicraft_metadata::TABLES {
        let rows = before.rows(name);
        if name == "Measure" || name == "DBPROPERTIES" {
            assert_ne!(after.rows(name), rows, "{name}");
        } else {
            assert_eq!(after.rows(name), rows, "{name}");
        }
    }
    let maxid = after
        .rows("DBPROPERTIES")
        .iter()
        .find(|r| r.get("NAME") == Some(&MetaValue::from("MAXID")))
        .and_then(|r| r.get("VALUE").cloned());
    assert_eq!(maxid, Some(MetaValue::Integer(53)));

    // a valid SQLite file whose change counter and schema cookie moved forward
    let db = Database::from_bytes(&out).unwrap();
    assert_eq!(
        db.tables().len(),
        Database::from_bytes(FIXTURE).unwrap().tables().len()
    );
    assert_eq!(header_u32(&out, 24), header_u32(FIXTURE, 24) + 1);
    assert_eq!(header_u32(&out, 40), header_u32(FIXTURE, 40) + 1);
}

#[test]
fn format_strings_and_roles_are_written() {
    let mut m = read_sqlitedb(FIXTURE).unwrap();
    m.table_mut("Sales")
        .unwrap()
        .measure_mut("Total")
        .unwrap()
        .format_string = Some("#,0.00".into());
    m.roles.push(Role {
        name: "bicraft readers".into(),
        model_permission: Some("read".into()),
        table_permissions: vec![TablePermission {
            table: "Sales".into(),
            filter_expression: Some("TRUE()".into()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let out = write_sqlitedb_at(FIXTURE, &m, NOW).unwrap();
    let back = read_sqlitedb(&out).unwrap();
    assert_eq!(
        back.measure("Total").unwrap().1.format_string.as_deref(),
        Some("#,0.00")
    );
    let role = back.roles.iter().find(|r| r.name == "bicraft readers").unwrap();
    assert_eq!(role.table_permissions.len(), 1);
    assert_eq!(role.table_permissions[0].table, "Sales");
    assert_eq!(
        role.table_permissions[0].filter_expression.as_deref(),
        Some("TRUE()")
    );
    assert_eq!(back, m);
}

#[test]
fn unsupported_changes_are_listed() {
    let mut m = read_sqlitedb(FIXTURE).unwrap();
    m.tables.retain(|t| t.name != "Calendar");
    match write_sqlitedb(FIXTURE, &m) {
        Err(Error::UnsupportedChange(list)) => {
            assert!(list.iter().any(|c| c.object.contains("Calendar")), "{list:?}");
        }
        other => panic!("expected UnsupportedChange, got {other:?}"),
    }
}

#[test]
fn garbage_is_an_error() {
    let m = read_sqlitedb(FIXTURE).unwrap();
    assert!(write_sqlitedb(b"not a database", &m).is_err());
    assert!(write_sqlitedb(&FIXTURE[..4096], &m).is_err());
}

/// The fixture with commit markers added to `DBPROPERTIES`.
fn with_markers(committed: i64) -> Vec<u8> {
    use bicraft_sqlite::{Row, Value};
    let db = Database::from_bytes(FIXTURE).unwrap();
    let mut rows = db.rows_vec("DBPROPERTIES").unwrap();
    let last = rows.iter().map(|r| r.rowid).max().unwrap();
    for (rowid, (name, value)) in (last + 1..).zip([
        ("LASTCOMMITTED", committed),
        ("LASTMETADATACHANGEFORSYNCCOMMITTED", committed),
        ("LASTCUBESCHEMACHANGE", 7),
    ]) {
        rows.push(Row {
            rowid,
            values: vec![Value::Text(name.into()), Value::Integer(value)],
        });
    }
    bicraft_sqlite::rewrite(&db, |t| (t == "DBPROPERTIES").then(|| rows.clone())).unwrap()
}

fn property(bytes: &[u8], name: &str) -> Option<MetaValue> {
    tables_from_sqlitedb(bytes)
        .unwrap()
        .rows("DBPROPERTIES")
        .iter()
        .find(|r| r.get("NAME") == Some(&MetaValue::from(name)))
        .and_then(|r| r.get("VALUE").cloned())
}

#[test]
fn commit_markers_move_forward_only() {
    let original = with_markers(5);
    let mut m = read_sqlitedb(&original).unwrap();
    // unchanged model: nothing moves
    assert_eq!(write_sqlitedb_at(&original, &m, NOW).unwrap(), original);
    m.table_mut("Sales").unwrap().measures[0].description = Some("Edited".into());
    let out = write_sqlitedb_at(&original, &m, NOW).unwrap();
    for name in bicraft_metadata::COMMIT_MARKERS {
        assert_eq!(property(&out, name), Some(MetaValue::Integer(NOW)), "{name}");
    }
    assert_eq!(
        property(&out, "LASTCUBESCHEMACHANGE"),
        Some(MetaValue::Integer(7))
    );
    assert_eq!(property(&out, "MAXID"), property(&original, "MAXID"));
    // a marker already later than the clock is kept
    let later = with_markers(NOW + 1);
    let out = write_sqlitedb_at(&later, &m, NOW).unwrap();
    assert_eq!(property(&out, "LASTCOMMITTED"), Some(MetaValue::Integer(NOW + 1)));
}
