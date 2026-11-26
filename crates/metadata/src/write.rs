//! [`MetadataTables`] → rows shaped for an SQLite writer (`bicraft_sqlite::Row`: a rowid plus
//! one value per declared column, in declaration order), and [`write_sqlitedb`], the complete
//! model → `metadata.sqlitedb` rewrite.

use std::collections::{BTreeMap, HashMap, HashSet};

use bicraft_model::SemanticModel;
use bicraft_sqlite::{Database, Row as SqlRow, TableInfo, Value};

use crate::Error;
use crate::input::{MetaRow, MetaValue, MetadataTables, Row, column_key};
use crate::structural::data::{EncodedData, StoragePlan};
use crate::structural::rename::Renames;

/// Writes the metadata-only edits of `model` into a copy of the `metadata.sqlitedb` image
/// `original`: reads it, [`apply_model`](crate::apply_model)s the model, and rewrites only the
/// tables whose rows changed (every other table, and every index of an unchanged table, is
/// copied page by page). The SQLite header's change counter and schema cookie move forward
/// (see `bicraft_sqlite::rewrite`).
///
/// When the model holds no difference the original bytes come back unchanged. Unsupported
/// differences are [`Error::UnsupportedChange`]; nothing is written then. The result is read
/// back before it is returned: it must map to the same model as the edited rows.
///
/// Change markers (see `docs/formats/metadata-sqlitedb.md`, "Writing"): `DBPROPERTIES.MAXID`
/// follows the new IDs (done by `apply_model`); the commit markers [`COMMIT_MARKERS`] move
/// forward to `modified_time` (never backward), the way a commit would, so that no row is
/// newer than the last commit; the `Model` row and the other markers are left alone.
pub fn write_sqlitedb(original: &[u8], model: &SemanticModel) -> Result<Vec<u8>, Error> {
    write_sqlitedb_at(original, model, crate::apply::filetime_now()?)
}

/// [`write_sqlitedb`] with an explicit clock (Windows FILETIME ticks), for reproducible output.
pub fn write_sqlitedb_at(
    original: &[u8],
    model: &SemanticModel,
    modified_time: i64,
) -> Result<Vec<u8>, Error> {
    let db = Database::from_bytes(original)?;
    let base = crate::tables_from_database(&db)?;
    let edited = crate::apply_model_at(&base, model, modified_time)?;
    rewrite(original, &db, &base, edited, modified_time, false)
}

/// The result of [`write_structural_sqlitedb`]: the new `metadata.sqlitedb` image, the ABF
/// files to remove and add with it, and the model as written (see
/// [`StructuralEdit::model`](crate::StructuralEdit::model)).
#[derive(Debug, Clone, PartialEq)]
pub struct StructuralWrite {
    pub sqlitedb: Vec<u8>,
    pub plan: StoragePlan,
    pub model: SemanticModel,
}

/// [`write_sqlitedb`] for structural edits: reads every table of `original`,
/// [`apply_structural`](crate::apply_structural)s the model with the encoded `data`, and
/// rewrites the tables that changed (storage tables included). The returned
/// [`StoragePlan`] lists the ABF files the caller must remove and add in the same save.
pub fn write_structural_sqlitedb(
    original: &[u8],
    model: &SemanticModel,
    data: &EncodedData,
) -> Result<StructuralWrite, Error> {
    write_structural_sqlitedb_at(original, model, data, crate::apply::filetime_now()?)
}

/// [`write_structural_sqlitedb`] with an explicit clock (Windows FILETIME ticks).
pub fn write_structural_sqlitedb_at(
    original: &[u8],
    model: &SemanticModel,
    data: &EncodedData,
    modified_time: i64,
) -> Result<StructuralWrite, Error> {
    let db = Database::from_bytes(original)?;
    let base = crate::tables_from_database_all(&db)?;
    let edit = crate::apply_structural_at(&base, model, data, modified_time)?;
    let data_added = !edit.plan.add.is_empty();
    let sqlitedb = rewrite(original, &db, &base, edit.tables, modified_time, data_added)?;
    Ok(StructuralWrite {
        sqlitedb,
        plan: edit.plan,
        model: edit.model,
    })
}

/// [`write_structural_sqlitedb_at`] after renaming tables and columns ([`Renames`]): the renames
/// are applied to the base rows first (names only; storage names and files stay), then the
/// structural edit runs on the renamed base, so `model` names the renamed objects by their new
/// names. Every changed row is written, renamed rows included.
pub fn write_structural_sqlitedb_renamed_at(
    original: &[u8],
    renames: &Renames,
    model: &SemanticModel,
    data: &EncodedData,
    modified_time: i64,
) -> Result<StructuralWrite, Error> {
    let db = Database::from_bytes(original)?;
    let base = crate::tables_from_database_all(&db)?;
    let renamed = crate::structural::rename::apply_renames(&base, renames, modified_time)?;
    let edit = crate::apply_structural_at(&renamed, model, data, modified_time)?;
    let data_added = !edit.plan.add.is_empty();
    let sqlitedb = rewrite(original, &db, &base, edit.tables, modified_time, data_added)?;
    Ok(StructuralWrite {
        sqlitedb,
        plan: edit.plan,
        model: edit.model,
    })
}

/// Rewrites the tables of `edited` that differ from `base`; the original bytes when none does.
fn rewrite(
    original: &[u8],
    db: &Database,
    base: &MetadataTables,
    mut edited: MetadataTables,
    modified_time: i64,
    data_added: bool,
) -> Result<Vec<u8>, Error> {
    if tables_to_rows(&edited).changed_tables(base).is_empty() {
        return Ok(original.to_vec());
    }
    bump_commit_markers(&mut edited, modified_time, data_added);
    let rows = tables_to_rows(&edited);
    let changed = rows.changed_tables(base);
    let mut replacement: HashMap<String, Vec<SqlRow>> = HashMap::new();
    for name in changed {
        let info = db
            .table(name)
            .ok_or_else(|| Error::Invalid(format!("table `{name}` is not in the database")))?;
        let has_id = info
            .columns
            .iter()
            .any(|c| c.rowid_alias && c.name.eq_ignore_ascii_case("ID"));
        let new_rows = if has_id {
            rows.rows_for(info)?
        } else {
            rows.rows_for_keyed(info, &db.rows_vec(&info.name)?)?
        };
        if let Some(r) = new_rows {
            replacement.insert(info.name.clone(), r);
        }
    }
    let out = bicraft_sqlite::rewrite(db, |name| replacement.remove(name))?;
    if !replacement.is_empty() {
        return Err(Error::Invalid("the SQLite writer skipped a changed table".into()));
    }
    // read back: the written file must describe the edited model
    if crate::build_model(&crate::tables_from_sqlitedb(&out)?)? != crate::build_model(&edited)? {
        return Err(Error::Invalid(
            "the rewritten metadata.sqlitedb does not read back as the edited model".into(),
        ));
    }
    Ok(out)
}

/// The rows of a [`MetadataTables`], converted table by table against the target file's schema.
/// Built by [`tables_to_rows`].
#[derive(Debug, Clone, Copy)]
pub struct SqliteRows<'a> {
    tables: &'a MetadataTables,
}

/// `DBPROPERTIES` entries (FILETIME values) that record the last committed change. Observed on
/// a Desktop file: every row's `ModifiedTime` / `StructureModifiedTime` / `RefreshedTime` is at
/// or before both of them.
pub const COMMIT_MARKERS: [&str; 2] = ["LASTCOMMITTED", "LASTMETADATACHANGEFORSYNCCOMMITTED"];

/// `DBPROPERTIES` entry (FILETIME) of the last data change: the engine raises it when a refresh
/// commits (it is `MDSCHEMA_CUBES.LAST_DATA_UPDATE`).
pub const DATA_CHANGE_MARKER: &str = "LASTCUBEDATACHANGE";

/// Raises the [`COMMIT_MARKERS`] present in `DBPROPERTIES` to `now` (never lowers them), and
/// [`DATA_CHANGE_MARKER`] too when the write materialized new data (`data_added`).
fn bump_commit_markers(t: &mut MetadataTables, now: i64, data_added: bool) {
    let Some(rows) = t.rows_mut("DBPROPERTIES") else {
        return;
    };
    for r in rows.iter_mut() {
        let is_marker = Row(r)
            .get("NAME")
            .and_then(|v| match v {
                MetaValue::Text(s) => Some(s.as_str()),
                _ => None,
            })
            .is_some_and(|n| {
                COMMIT_MARKERS.iter().any(|m| m.eq_ignore_ascii_case(n))
                    || (data_added && DATA_CHANGE_MARKER.eq_ignore_ascii_case(n))
            });
        if !is_marker {
            continue;
        }
        let Some(key) = column_key(r, "VALUE").map(str::to_string) else {
            continue;
        };
        if let Some(v @ MetaValue::Integer(_)) = r.get_mut(&key)
            && matches!(v, MetaValue::Integer(old) if *old < now)
        {
            *v = MetaValue::Integer(now);
        }
    }
}

/// Wraps edited tables (typically the output of [`apply_model`](crate::apply_model)) for an
/// SQLite writer: [`SqliteRows::rows_for`] gives the full new content of one table, and
/// [`SqliteRows::changed_tables`] names the tables that differ from the base, so a writer can
/// leave every other table untouched.
pub fn tables_to_rows(t: &MetadataTables) -> SqliteRows<'_> {
    SqliteRows { tables: t }
}

impl<'a> SqliteRows<'a> {
    /// Names of the tables whose rows differ from `base` (as stored in the edited tables).
    pub fn changed_tables(&self, base: &MetadataTables) -> Vec<&'a str> {
        self.tables
            .iter()
            .filter(|(name, rows)| base.rows(name) != *rows || !base.has_table(name))
            .map(|(name, _)| name)
            .collect()
    }

    /// The content of `table` (matched by name, ASCII case-insensitively), or `None` when the
    /// edited tables do not hold it (the writer then keeps the original rows).
    ///
    /// For tables whose `ID` column is an `INTEGER PRIMARY KEY` (every TMSCHEMA object table) the
    /// rowid is the `ID`, which also fills the `ID` slot, as `bicraft-sqlite` reports it. Rows
    /// come sorted by rowid. Tables without such a column (`DBPROPERTIES`) carry no rowid in
    /// [`MetadataTables`]; use [`rows_for_keyed`](Self::rows_for_keyed) for them.
    ///
    /// Errors: a row without an integer `ID`, a duplicate `ID`, or a non-NULL value in a column
    /// the table does not declare.
    pub fn rows_for(&self, table: &TableInfo) -> Result<Option<Vec<SqlRow>>, Error> {
        if !self.tables.has_table(&table.name) {
            return Ok(None);
        }
        let rows = self.tables.rows(&table.name);
        let id_col = table
            .columns
            .iter()
            .position(|c| c.rowid_alias && c.name.eq_ignore_ascii_case("ID"))
            .ok_or_else(|| {
                Error::Invalid(format!(
                    "table `{}` has no INTEGER PRIMARY KEY `ID`; use rows_for_keyed",
                    table.name
                ))
            })?;
        let mut out = Vec::with_capacity(rows.len());
        let mut seen = HashSet::new();
        for r in rows {
            let id = Row(r)
                .get("ID")
                .and_then(|v| match v {
                    MetaValue::Integer(i) => Some(*i),
                    _ => None,
                })
                .ok_or_else(|| Error::Invalid(format!("a `{}` row has no integer ID", table.name)))?;
            if !seen.insert(id) {
                return Err(Error::Invalid(format!("duplicate ID {id} in `{}`", table.name)));
            }
            let mut values = values(table, r)?;
            if let Some(slot) = values.get_mut(id_col) {
                *slot = Value::Integer(id);
            }
            out.push(SqlRow { rowid: id, values });
        }
        out.sort_by_key(|r| r.rowid);
        Ok(Some(out))
    }

    /// The content of a table without a rowid alias, keeping the rowids of `original` (the
    /// table's current rows): each edited row is matched to an original row by the declared
    /// primary-key columns (e.g. `DBPROPERTIES.NAME`). Edited rows without an original get rowids
    /// above the largest one. `None` when the edited tables do not hold the table.
    pub fn rows_for_keyed(
        &self,
        table: &TableInfo,
        original: &[SqlRow],
    ) -> Result<Option<Vec<SqlRow>>, Error> {
        if !self.tables.has_table(&table.name) {
            return Ok(None);
        }
        let key_cols: Vec<usize> = table
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.primary_key)
            .map(|(i, _)| i)
            .collect();
        if key_cols.is_empty() {
            return Err(Error::Invalid(format!(
                "table `{}` has no primary key",
                table.name
            )));
        }
        let key_of = |vals: &[Value]| -> Vec<Value> {
            key_cols
                .iter()
                .map(|&i| vals.get(i).cloned().unwrap_or(Value::Null))
                .collect()
        };
        let mut by_key: BTreeMap<String, i64> = BTreeMap::new();
        let mut next = 0i64;
        for o in original {
            by_key.insert(format!("{:?}", key_of(&o.values)), o.rowid);
            next = next.max(o.rowid);
        }
        let mut out = Vec::new();
        let mut used = HashSet::new();
        for r in self.tables.rows(&table.name) {
            let values = values(table, r)?;
            let key = format!("{:?}", key_of(&values));
            let rowid = match by_key.get(&key) {
                Some(&id) => id,
                None => {
                    next = next
                        .checked_add(1)
                        .ok_or_else(|| Error::Invalid(format!("rowids of `{}` exhausted", table.name)))?;
                    next
                }
            };
            if !used.insert(rowid) {
                return Err(Error::Invalid(format!(
                    "duplicate primary key in `{}`",
                    table.name
                )));
            }
            out.push(SqlRow { rowid, values });
        }
        out.sort_by_key(|r| r.rowid);
        Ok(Some(out))
    }
}

/// One value per declared column. Missing columns take the declared default (else NULL);
/// non-NULL values for undeclared columns are an error.
fn values(table: &TableInfo, r: &MetaRow) -> Result<Vec<Value>, Error> {
    for (k, v) in r {
        if *v != MetaValue::Null && table.column_index(k).is_none() {
            return Err(Error::Invalid(format!(
                "table `{}` has no column `{k}` to store a value in",
                table.name
            )));
        }
    }
    Ok(table
        .columns
        .iter()
        .map(|c| match column_key(r, &c.name).and_then(|k| r.get(k)) {
            Some(v) => convert(v),
            None => c.default.clone().unwrap_or(Value::Null),
        })
        .collect())
}

fn convert(v: &MetaValue) -> Value {
    match v {
        MetaValue::Null => Value::Null,
        MetaValue::Integer(i) => Value::Integer(*i),
        MetaValue::Real(f) => Value::Real(*f),
        MetaValue::Text(s) => Value::Text(s.clone()),
        MetaValue::Blob(b) => Value::Blob(b.clone()),
    }
}
