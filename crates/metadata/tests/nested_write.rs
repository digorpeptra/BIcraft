//! Writing the objects kept as JSON in `extra`: what is refused, and why. Runs on the
//! engine-written fixtures of `tests/engine` (our own models).

use bicraft_metadata::{Error, read_sqlitedb, write_sqlitedb_at};
use bicraft_model::SemanticModel;
use serde_json::{Value, json};

const NOW: i64 = 134_000_000_000_000_000;

fn fixture(case: &str) -> (Vec<u8>, SemanticModel) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("engine")
        .join(format!("{case}.sqlitedb"));
    let bytes = std::fs::read(path).unwrap();
    let model = read_sqlitedb(&bytes).unwrap();
    (bytes, model)
}

/// The refusals of writing `m` over `bytes`, as `object: what` lines.
fn refusals(bytes: &[u8], m: &SemanticModel) -> Vec<String> {
    match write_sqlitedb_at(bytes, m, NOW) {
        Err(Error::UnsupportedChange(list)) => list.iter().map(ToString::to_string).collect(),
        Err(e) => panic!("unexpected error {e}"),
        Ok(_) => Vec::new(),
    }
}

fn table<'m>(m: &'m mut SemanticModel, name: &str) -> &'m mut bicraft_model::Table {
    m.table_mut(name).unwrap()
}

fn extra<'v>(v: &'v mut Value, path: &[&str]) -> &'v mut Value {
    let mut cur = v;
    for p in path {
        cur = match p.parse::<usize>() {
            Ok(i) => &mut cur[i],
            Err(_) => &mut cur[*p],
        };
    }
    cur
}

#[test]
fn objects_that_carry_stored_data_are_refused() {
    let (bytes, base) = fixture("objects");

    // User hierarchies are stored (U$ tables): a renamed one is a removal plus an addition,
    // both refused, and so is a level added or a level's column changed.
    let mut m = base.clone();
    let h = table(&mut m, "Sales").extra.get_mut("hierarchies").unwrap();
    *extra(h, &["0", "name"]) = json!("Geo2");
    let got = refusals(&bytes, &m);
    assert!(got.iter().any(|r| r.ends_with("hierarchy removed")), "{got:?}");
    assert!(got.iter().any(|r| r.ends_with("hierarchy added")), "{got:?}");
    let mut m = base.clone();
    let h = table(&mut m, "Sales").extra.get_mut("hierarchies").unwrap();
    let levels = extra(h, &["0", "levels"]).as_array_mut().unwrap();
    levels.push(json!({"name": "Id", "ordinal": 2, "column": "Id"}));
    levels[0]["column"] = json!("City");
    let got = refusals(&bytes, &m);
    assert!(got.iter().any(|r| r.ends_with("level added")), "{got:?}");
    assert!(
        got.iter().any(|r| r.contains("`column` of a level changed")),
        "{got:?}"
    );

    // Calculation items are the rows of the calculation group table.
    let mut m = base.clone();
    let cg = table(&mut m, "Time Calc")
        .extra
        .get_mut("calculationGroup")
        .unwrap();
    extra(cg, &["calculationItems"])
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "Triple", "expression": "SELECTEDMEASURE() * 3", "ordinal": 2}));
    let got = refusals(&bytes, &m);
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].contains("calculation item added"), "{got:?}");

    // A calculation group cannot be added to a table.
    let mut m = base.clone();
    table(&mut m, "Param")
        .extra
        .insert("calculationGroup".into(), json!({"precedence": 1}));
    let got = refusals(&bytes, &m);
    assert!(
        got.iter().any(|r| r.contains("calculation group added")),
        "{got:?}"
    );
}

#[test]
fn malformed_and_dangling_values_are_refused() {
    let (bytes, base) = fixture("objects");

    // An unknown property, an unknown code, a column the table does not have.
    let mut m = base.clone();
    let p = m.extra.get_mut("perspectives").unwrap();
    extra(p, &["0"])["color"] = json!("red");
    let sales = table(&mut m, "Sales");
    sales.extra.get_mut("hierarchies").unwrap()[0]["hideMembers"] = json!("sometimes");
    let total = sales.measure_mut("Total").unwrap();
    total
        .extra
        .insert("detailRowsDefinition".into(), json!({"expression": 1}));
    let got = refusals(&bytes, &m);
    let want = [
        "property `color` of a perspective is not supported",
        "unknown value 'sometimes'",
        "`expression`: not a string",
    ];
    for w in want {
        assert!(got.iter().any(|r| r.contains(w)), "{w}: {got:?}");
    }
    let mut m = base.clone();
    m.extra.get_mut("perspectives").unwrap()[0]["tables"][0]["columns"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "NoSuchColumn"}));
    let got = refusals(&bytes, &m);
    assert!(
        got.iter().any(|r| r.contains("no column named 'NoSuchColumn'")),
        "{got:?}"
    );

    // Translations of objects the model does not have; a JSON extended property that is not
    // an object (TOM refuses it).
    let mut m = base.clone();
    let cultures = m.extra.get_mut("cultures").unwrap();
    cultures[1]["translations"]["model"]["tables"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "Nope", "translatedCaption": "x"}));
    m.extra.insert(
        "extendedProperties".into(),
        json!([{"type": "json", "name": "J", "value": [1]}]),
    );
    let got = refusals(&bytes, &m);
    assert!(
        got.iter().any(|r| r.contains("translations: no table 'Nope'")),
        "{got:?}"
    );
    assert!(
        got.iter()
            .any(|r| r.contains("malformed `extendedProperties` entry")),
        "{got:?}"
    );
}

#[test]
fn referenced_objects_and_encrypted_data_sources_are_refused() {
    let (bytes, base) = fixture("objects");
    // The query group still holds the shared expression.
    let mut m = base.clone();
    m.extra.shift_remove("queryGroups");
    let got = refusals(&bytes, &m);
    assert!(
        got.iter()
            .any(|r| r.contains("query group removed while `Expression` rows use it")),
        "{got:?}"
    );

    // A data source needs an encrypted connection string or credential.
    let (bytes, base) = fixture("sources");
    let mut m = base.clone();
    m.extra
        .get_mut("dataSources")
        .unwrap()
        .as_array_mut()
        .unwrap()
        .push(json!({"type": "structured", "name": "New"}));
    let got = refusals(&bytes, &m);
    assert!(got.iter().any(|r| r.contains("data source added")), "{got:?}");
    // The legacy data source is used by a partition.
    let mut m = base.clone();
    let ds = m.extra.get_mut("dataSources").unwrap().as_array_mut().unwrap();
    ds.retain(|d| d["name"] != "LegacyDS");
    let got = refusals(&bytes, &m);
    assert!(
        got.iter()
            .any(|r| r.contains("data source removed while `Partition` rows use it")),
        "{got:?}"
    );
}

#[test]
fn a_measure_with_a_kpi_is_removed_with_it() {
    let (bytes, base) = fixture("objects");
    let mut m = base.clone();
    // Still in a perspective: refused.
    table(&mut m, "Sales").measures.retain(|x| x.name != "Total");
    let got = refusals(&bytes, &m);
    assert!(got.iter().any(|r| r.contains("is in a perspective")), "{got:?}");
    // Out of the perspective too: the KPI, its annotations and the detail rows go with it.
    let tables = m.extra.get_mut("perspectives").unwrap()[0]["tables"][0]
        .as_object_mut()
        .unwrap();
    tables.shift_remove("measures");
    let cultures = m.extra.get_mut("cultures").unwrap();
    cultures[1]["translations"]["model"]["tables"][0]
        .as_object_mut()
        .unwrap()
        .shift_remove("measures");
    let out = write_sqlitedb_at(&bytes, &m, NOW).unwrap();
    assert_eq!(read_sqlitedb(&out).unwrap(), m);
    let rows = bicraft_metadata::tables_from_sqlitedb(&out).unwrap();
    assert!(rows.rows("KPI").is_empty());
    assert_eq!(
        rows.rows("DetailRowsDefinition").len(),
        1,
        "the table's default detail rows stay"
    );
}
