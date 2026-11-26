//! Referential checks over metadata rows: every reference must name an existing object.

use std::collections::{BTreeSet, HashSet};

use crate::input::{MetaValue, MetadataTables, Row};

/// Columns ending in `ID` that are not plain references.
const NOT_REFERENCES: [&str; 3] = ["ID", "ObjectID", "OwnerID"];

/// Every reference of `t` that names no row: integer columns whose name ends in `ID` (other
/// than `ID`, `ObjectID` and the `Statistics_*DataID` data ids) and `StorageFile`/
/// `StorageFolder` `OwnerID`, when non-zero. Each
/// entry reads `Table.Column = value`; sorted, without duplicates.
pub fn dangling_references(t: &MetadataTables) -> Vec<String> {
    let mut ids = HashSet::new();
    for (name, rows) in t.iter() {
        if name.eq_ignore_ascii_case("DBPROPERTIES") {
            continue;
        }
        ids.extend(rows.iter().filter_map(|r| Row(r).id()));
    }
    let mut out = BTreeSet::new();
    for (name, rows) in t.iter() {
        if name.eq_ignore_ascii_case("DBPROPERTIES") {
            continue;
        }
        for r in rows {
            for (col, v) in r {
                // `Statistics_MinDataID` & co. are VertiPaq data ids, not object IDs.
                let is_ref = col.ends_with("ID")
                    && !col.starts_with("Statistics_")
                    && !NOT_REFERENCES.contains(&col.as_str());
                let is_owner = col == "OwnerID"
                    && (name.eq_ignore_ascii_case("StorageFile")
                        || name.eq_ignore_ascii_case("StorageFolder"));
                if !(is_ref || is_owner) {
                    continue;
                }
                if let MetaValue::Integer(id) = v
                    && *id != 0
                    && !ids.contains(id)
                {
                    out.insert(format!("{name}.{col} = {id}"));
                }
            }
        }
    }
    out.into_iter().collect()
}

/// The ABF logical paths named by the `StorageFile` rows (folder path + file name, `/`
/// separators), sorted.
pub fn storage_file_paths(t: &MetadataTables) -> Vec<String> {
    let folders: std::collections::HashMap<i64, String> = t
        .rows("StorageFolder")
        .iter()
        .map(Row)
        .filter_map(|r| Some((r.id()?, r.raw_text("Path")?.to_string())))
        .collect();
    let mut out: Vec<String> = t
        .rows("StorageFile")
        .iter()
        .map(Row)
        .filter_map(|r| {
            let folder = r
                .reference("StorageFolderID")
                .and_then(|f| folders.get(&f))
                .map_or("", String::as_str);
            Some(crate::structural::names::logical_path(
                folder,
                r.raw_text("FileName")?,
            ))
        })
        .collect();
    out.sort();
    out
}
