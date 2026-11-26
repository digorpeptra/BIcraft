//! Storage names and paths: how Desktop names `.tbl`/`.prt` folders and VertiPaq files after
//! the objects they store (see `docs/formats/metadata-sqlitedb.md`, "Structural writing").

/// The storage form of an object name: every character that is not a letter, a digit or a space
/// becomes a space. Observed on the real file: `_`, `-` (GUID hyphens of row-number columns and
/// relationship names) and `>` (a hierarchy name) are replaced, accented letters are kept. Other
/// punctuation is not observed; replacing it too keeps `.` (the file-name separator), `\` and `$`
/// (the system-table separator) out of names.
pub(crate) fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' { c } else { ' ' })
        .collect()
}

/// `TableStorage.Name`, `ColumnStorage.Name`, `PartitionStorage.Name`,
/// `RelationshipStorage.Name` of a user object: `<sanitized name> (<object ID>)`.
pub(crate) fn object_storage_name(name: &str, id: i64) -> String {
    format!("{} ({id})", sanitize(name))
}

/// `StorageFolder.Path` of a user table: `<TableStorage.Name>.tbl`.
pub(crate) fn table_folder(table_storage_name: &str) -> String {
    format!("{table_storage_name}.tbl")
}

/// `StorageFolder.Path` of a system table: `<TableStorage.Name>$(<table ID>).tbl`.
pub(crate) fn system_table_folder(table_storage_name: &str, table_id: i64) -> String {
    format!("{table_storage_name}$({table_id}).tbl")
}

/// `StorageFolder.Path` of a partition: `<table folder>\<partition ID>.prt`.
pub(crate) fn partition_folder(table_folder: &str, partition_id: i64) -> String {
    format!("{table_folder}\\{partition_id}.prt")
}

/// Name of the `H$` table of a column: `H$<TableStorage.Name>$<ColumnStorage.Name>`.
pub(crate) fn hierarchy_table(table_storage_name: &str, column_storage_name: &str) -> String {
    format!("H${table_storage_name}${column_storage_name}")
}

/// Name of the `R$` table of a relationship:
/// `R$<from TableStorage.Name>$<RelationshipStorage.Name>`.
pub(crate) fn relationship_table(from_table_storage_name: &str, relationship_storage_name: &str) -> String {
    format!("R${from_table_storage_name}${relationship_storage_name}")
}

/// Name of the `U$` table of a user hierarchy:
/// `U$<TableStorage.Name>$<HierarchyStorage.Name>` (engine-observed).
pub(crate) fn user_hierarchy_table(table_storage_name: &str, hierarchy_storage_name: &str) -> String {
    format!("U${table_storage_name}${hierarchy_storage_name}")
}

/// Column data file: `<version>.<TableStorage.Name>.<ColumnStorage.Name>.<k>.idf`, `k` the
/// partition's position in the table (one file per column partition, every segment inside).
pub(crate) fn idf(version: i64, table: &str, column: &str, partition: usize) -> String {
    format!("{version}.{table}.{column}.{partition}.idf")
}

/// Segment statistics file: `<version>.<TableStorage.Name>.<ColumnStorage.Name>.<k>.idfmeta`.
pub(crate) fn idfmeta(version: i64, table: &str, column: &str, partition: usize) -> String {
    format!("{version}.{table}.{column}.{partition}.idfmeta")
}

/// Dictionary file (table folder): `<version>.<TableStorage.Name>.<ColumnStorage.Name>.dictionary`.
pub(crate) fn dictionary(version: i64, table: &str, column: &str) -> String {
    format!("{version}.{table}.{column}.dictionary")
}

/// Hash index file (table folder): `<version>.H$<TableStorage.Name>$<ColumnStorage.Name>.hidx`.
pub(crate) fn hidx(version: i64, table: &str, column: &str) -> String {
    format!("{version}.H${table}${column}.hidx")
}

/// The ABF logical path of a stored file: the folder path with `/` separators, then the name.
pub(crate) fn logical_path(folder: &str, file: &str) -> String {
    let folder = folder.replace('\\', "/");
    if folder.is_empty() {
        file.to_string()
    } else {
        format!("{folder}/{file}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_observed_patterns() {
        assert_eq!(sanitize("_Measures"), " Measures");
        assert_eq!(sanitize("shop_id"), "shop id");
        assert_eq!(sanitize("Ann\u{e9}e > Month"), "Ann\u{e9}e   Month");
        assert_eq!(
            sanitize("RowNumber-2662979B-1795-4F74-8F37-6A1BA8059B61"),
            "RowNumber 2662979B 1795 4F74 8F37 6A1BA8059B61"
        );
        assert_eq!(sanitize("a.b$c\\d"), "a b c d");
        assert_eq!(object_storage_name("Shop", 19), "Shop (19)");
        let tbl = table_folder("Shop (19)");
        assert_eq!(tbl, "Shop (19).tbl");
        assert_eq!(partition_folder(&tbl, 86), "Shop (19).tbl\\86.prt");
        let h = hierarchy_table("Shop (19)", "shop id (50)");
        assert_eq!(h, "H$Shop (19)$shop id (50)");
        assert_eq!(system_table_folder(&h, 777), "H$Shop (19)$shop id (50)$(777).tbl");
        assert_eq!(idf(1, "Shop (19)", "Name (51)", 0), "1.Shop (19).Name (51).0.idf");
        assert_eq!(idf(1, "Sales (10)", "Qty (22)", 7), "1.Sales (10).Qty (22).7.idf");
        assert_eq!(
            idfmeta(0, &h, "POS_TO_ID", 0),
            format!("0.{h}.POS_TO_ID.0.idfmeta")
        );
        assert_eq!(dictionary(1, "T (1)", "C (2)"), "1.T (1).C (2).dictionary");
        assert_eq!(hidx(1, "T (1)", "C (2)"), "1.H$T (1)$C (2).hidx");
        assert_eq!(relationship_table("V (22)", "a b (34)"), "R$V (22)$a b (34)");
        assert_eq!(logical_path("A.tbl\\3.prt", "f.idf"), "A.tbl/3.prt/f.idf");
        assert_eq!(logical_path("", "f"), "f");
    }
}
