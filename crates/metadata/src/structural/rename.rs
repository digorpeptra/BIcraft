//! Renames of tables and columns, applied to the base rows before a structural edit.
//!
//! A rename changes the object's name only: `Table.Name`, `Column.ExplicitName`. Storage names
//! (`TableStorage.Name`, `ColumnStorage.Name`, folders, files, `H$`/`R$` system tables) embed
//! the object ID next to the old name and are left as they are, so no file moves; the names are
//! opaque to the engine once written. DAX expressions are not rewritten.

use crate::input::{MetaValue, MetadataTables, Row};
use crate::{Error, UnsupportedChange};

/// Renames to apply before a structural edit (see [`crate::write_structural_sqlitedb_renamed_at`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Renames {
    /// (old table name, new table name).
    pub tables: Vec<(String, String)>,
    /// (table name **before** the table renames, old column name, new column name).
    pub columns: Vec<(String, String, String)>,
}

impl Renames {
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty() && self.columns.is_empty()
    }
}

/// The user table of the model called `name` (exact match first, then ASCII case-insensitive).
fn user_table_id(t: &MetadataTables, name: &str) -> Option<i64> {
    let users: Vec<(i64, &str)> = t
        .rows("Table")
        .iter()
        .map(Row)
        .filter(|r| r.int("SystemFlags").unwrap_or(0) & 1 == 0)
        .filter_map(|r| Some((r.id()?, r.text("Name")?)))
        .collect();
    users
        .iter()
        .find(|(_, n)| *n == name)
        .or_else(|| users.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)))
        .map(|(id, _)| *id)
}

/// Applies `renames` to a copy of `base`. A rename whose object is missing is refused.
pub(crate) fn apply_renames(
    base: &MetadataTables,
    renames: &Renames,
    now: i64,
) -> Result<MetadataTables, Error> {
    let mut out = base.clone();
    let mut refused = Vec::new();
    let refuse = |refused: &mut Vec<UnsupportedChange>, object: String, what: &str| {
        refused.push(UnsupportedChange {
            what: what.to_string(),
            object,
        });
    };

    // Columns first: they are named by their table's old name.
    let mut column_rows = base.rows("Column").to_vec();
    for (table, old, new) in &renames.columns {
        let object = format!("column '{table}'[{old}]");
        let Some(tid) = user_table_id(base, table) else {
            refuse(
                &mut refused,
                object,
                "renamed column's table not found in the metadata",
            );
            continue;
        };
        let found = column_rows.iter_mut().find(|r| {
            let row = Row(r);
            row.int("TableID") == Some(tid)
                && row
                    .text("ExplicitName")
                    .or_else(|| row.text("InferredName"))
                    .is_some_and(|n| n.eq_ignore_ascii_case(old))
        });
        let Some(row) = found else {
            refuse(&mut refused, object, "column to rename not found in the metadata");
            continue;
        };
        // A name inferred from the table expression (a calculated-table column that follows
        // its source column's rename) stays inferred; whether it becomes explicit is the edited
        // model's `isNameInferred` (applied afterwards).
        let key = if Row(row).text("ExplicitName").is_none() {
            "InferredName"
        } else {
            "ExplicitName"
        };
        row.insert(key.into(), MetaValue::Text(new.clone()));
        row.insert("ModifiedTime".into(), now.into());
        row.insert("StructureModifiedTime".into(), now.into());
    }
    out.insert_table("Column", column_rows);

    let mut table_rows = base.rows("Table").to_vec();
    for (old, new) in &renames.tables {
        let object = format!("table '{old}'");
        let Some(tid) = user_table_id(base, old) else {
            refuse(&mut refused, object, "table to rename not found in the metadata");
            continue;
        };
        let Some(row) = table_rows.iter_mut().find(|r| Row(r).id() == Some(tid)) else {
            continue;
        };
        row.insert("Name".into(), MetaValue::Text(new.clone()));
        row.insert("ModifiedTime".into(), now.into());
        row.insert("StructureModifiedTime".into(), now.into());
    }
    out.insert_table("Table", table_rows);

    if !refused.is_empty() {
        return Err(Error::UnsupportedChange(refused));
    }
    Ok(out)
}
