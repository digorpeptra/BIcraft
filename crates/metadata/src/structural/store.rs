//! A mutable copy of the metadata rows with the few operations the structural writer needs:
//! find by ID or foreign key, delete, update, insert with the table's column set, allocate IDs.

use std::collections::HashSet;

use crate::Error;
use crate::input::{MetaRow, MetaValue, MetadataTables, Row, column_key};
use crate::structural::data::StoragePlan;
use crate::structural::names;

/// `RefreshedTime` of objects never refreshed (system partitions, columns): 1699-12-31T00:00Z as
/// FILETIME ticks (the engine's DMVs show that date). Observed on every column and system
/// partition of the real file.
pub(crate) const NEVER: i64 = 31_240_512_000_000_000;

pub(crate) struct Store {
    pub(crate) t: MetadataTables,
    pub(crate) now: i64,
    max_id: i64,
    allocated: bool,
    pub(crate) plan: StoragePlan,
    /// Tables whose rows changed (for `ModifiedTime` bookkeeping by callers).
    pub(crate) touched_tables: HashSet<i64>,
}

impl Store {
    pub(crate) fn new(base: &MetadataTables, now: i64) -> Result<Self, Error> {
        let mut seen = HashSet::new();
        let mut max_id = 0i64;
        for (name, rows) in base.iter() {
            if name.eq_ignore_ascii_case("DBPROPERTIES") {
                continue;
            }
            for r in rows {
                if let Some(id) = Row(r).id() {
                    if !seen.insert(id) {
                        return Err(Error::Invalid(format!("object ID {id} is used twice")));
                    }
                    max_id = max_id.max(id);
                }
            }
        }
        if let Some(m) = base
            .rows("DBPROPERTIES")
            .iter()
            .find(|r| is_maxid(r))
            .and_then(|r| Row(r).int("VALUE"))
        {
            max_id = max_id.max(m);
        }
        Ok(Self {
            t: base.clone(),
            now,
            max_id,
            allocated: false,
            plan: StoragePlan::default(),
            touched_tables: HashSet::new(),
        })
    }

    pub(crate) fn alloc(&mut self) -> Result<i64, Error> {
        let id = self
            .max_id
            .checked_add(1)
            .ok_or_else(|| Error::Invalid("object IDs exhausted".into()))?;
        self.max_id = id;
        self.allocated = true;
        Ok(id)
    }

    /// The row of `table` with `ID` = `id`.
    pub(crate) fn row(&self, table: &str, id: i64) -> Option<Row<'_>> {
        if id == 0 {
            return None;
        }
        self.t.rows(table).iter().map(Row).find(|r| r.id() == Some(id))
    }

    /// An integer column of the row `table[id]`.
    pub(crate) fn int(&self, table: &str, id: i64, col: &str) -> Option<i64> {
        self.row(table, id).and_then(|r| r.int(col))
    }

    /// A non-zero reference held by `table[id].col`.
    pub(crate) fn reference(&self, table: &str, id: i64, col: &str) -> Option<i64> {
        self.row(table, id).and_then(|r| r.reference(col))
    }

    pub(crate) fn text(&self, table: &str, id: i64, col: &str) -> Option<String> {
        self.row(table, id)
            .and_then(|r| r.raw_text(col))
            .map(str::to_string)
    }

    /// IDs of the rows of `table` whose `col` equals `value`.
    pub(crate) fn ids_where(&self, table: &str, col: &str, value: i64) -> Vec<i64> {
        self.t
            .rows(table)
            .iter()
            .map(Row)
            .filter(|r| r.int(col) == Some(value))
            .filter_map(|r| r.id())
            .collect()
    }

    /// IDs of the rows of `table` matching `pred`.
    pub(crate) fn ids_matching(&self, table: &str, pred: impl Fn(Row<'_>) -> bool) -> Vec<i64> {
        self.t
            .rows(table)
            .iter()
            .map(Row)
            .filter(|r| pred(*r))
            .filter_map(|r| r.id())
            .collect()
    }

    /// Deletes the row of `table` with `ID` = `id` (no-op when absent).
    pub(crate) fn delete(&mut self, table: &str, id: i64) {
        if let Some(rows) = self.t.rows_mut(table) {
            rows.retain(|r| Row(r).id() != Some(id));
        }
    }

    /// Sets `col` of `table[id]` when the row has that column and the value differs. Returns
    /// whether something changed.
    pub(crate) fn set(&mut self, table: &str, id: i64, col: &str, v: MetaValue) -> bool {
        let Some(row) = self
            .t
            .rows_mut(table)
            .and_then(|rows| rows.iter_mut().find(|r| Row(r).id() == Some(id)))
        else {
            return false;
        };
        let Some(key) = column_key(row, col).map(str::to_string) else {
            return false;
        };
        if row.get(&key) == Some(&v) {
            return false;
        }
        row.insert(key, v);
        true
    }

    /// Marks a user table changed: `ModifiedTime` and `StructureModifiedTime` become now.
    pub(crate) fn touch_table(&mut self, table_id: i64) {
        if self.touched_tables.insert(table_id) {
            let now = MetaValue::Integer(self.now);
            self.set("Table", table_id, "ModifiedTime", now.clone());
            self.set("Table", table_id, "StructureModifiedTime", now);
        }
    }

    /// Appends a row to `table`. When the table has rows, the new row takes exactly their
    /// columns: a cell for a column they lack is dropped if it is NULL and kept otherwise (the
    /// SQLite adapter then reports it), and a column the cells do not name gets the integer every
    /// existing row shares, else NULL. An empty table takes the cells as given.
    pub(crate) fn insert(&mut self, table: &str, cells: Vec<(&str, MetaValue)>) {
        let existing = self.t.rows(table);
        let mut row = MetaRow::new();
        if existing.is_empty() {
            for (k, v) in cells {
                row.insert(k.to_string(), v);
            }
        } else {
            let mut columns: Vec<&str> = Vec::new();
            for r in existing {
                for k in r.keys() {
                    if !columns.iter().any(|c| c == k) {
                        columns.push(k);
                    }
                }
            }
            let mut cells: Vec<(&str, Option<MetaValue>)> =
                cells.into_iter().map(|(k, v)| (k, Some(v))).collect();
            for col in &columns {
                let v = match cells.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(col)) {
                    Some((_, v)) => v.take().unwrap_or(MetaValue::Null),
                    None => shared_integer(existing, col),
                };
                row.insert((*col).to_string(), v);
            }
            for (k, v) in cells {
                if let Some(v) = v
                    && v != MetaValue::Null
                {
                    row.insert(k.to_string(), v);
                }
            }
        }
        let key = self.t.key(table).unwrap_or(table).to_string();
        self.t.push_row(&key, row);
    }

    // --- storage files and folders -----------------------------------------------------------

    /// The ABF logical path of a `StorageFile` row.
    pub(crate) fn file_path(&self, file_id: i64) -> Option<String> {
        let f = self.row("StorageFile", file_id)?;
        let name = f.raw_text("FileName")?;
        let folder = f
            .reference("StorageFolderID")
            .and_then(|id| self.text("StorageFolder", id, "Path"))
            .unwrap_or_default();
        Some(names::logical_path(&folder, name))
    }

    /// Deletes a `StorageFile` row and queues its ABF file for removal.
    pub(crate) fn drop_file(&mut self, file_id: i64) {
        if file_id == 0 {
            return;
        }
        if let Some(path) = self.file_path(file_id)
            && !self.plan.remove.contains(&path)
        {
            self.plan.remove.push(path);
        }
        self.delete("StorageFile", file_id);
    }

    /// Deletes the file referenced by `table[id].col` (a `StorageFileID`).
    pub(crate) fn drop_file_of(&mut self, table: &str, id: i64, col: &str) {
        if let Some(f) = self.reference(table, id, col) {
            self.drop_file(f);
        }
    }

    /// Deletes every `StorageFile` owned by `owner` (any owner type).
    pub(crate) fn drop_files_owned_by(&mut self, owner: i64) {
        for f in self.ids_where("StorageFile", "OwnerID", owner) {
            self.drop_file(f);
        }
    }

    /// Deletes a `StorageFolder` row and every file stored in it.
    pub(crate) fn drop_folder(&mut self, folder_id: i64) {
        if folder_id == 0 {
            return;
        }
        for f in self.ids_where("StorageFile", "StorageFolderID", folder_id) {
            self.drop_file(f);
        }
        self.delete("StorageFolder", folder_id);
    }

    /// Inserts a `StorageFolder` row; returns its ID.
    pub(crate) fn new_folder(&mut self, owner: i64, owner_type: i64, path: &str) -> Result<i64, Error> {
        let id = self.alloc()?;
        self.insert(
            "StorageFolder",
            vec![
                ("ID", id.into()),
                ("OwnerID", owner.into()),
                ("OwnerType", owner_type.into()),
                ("Path", path.into()),
            ],
        );
        Ok(id)
    }

    /// Inserts a `StorageFile` row for `bytes` and queues the file; returns the row ID.
    pub(crate) fn new_file(
        &mut self,
        owner: i64,
        owner_type: i64,
        folder_id: i64,
        name: &str,
        bytes: Vec<u8>,
    ) -> Result<i64, Error> {
        let folder = self
            .text("StorageFolder", folder_id, "Path")
            .ok_or_else(|| Error::Invalid(format!("storage folder {folder_id} is missing")))?;
        let path = names::logical_path(&folder, name);
        if self.plan.add.iter().any(|f| f.path.eq_ignore_ascii_case(&path)) {
            return Err(Error::Invalid(format!("two new files share the path `{path}`")));
        }
        let id = self.alloc()?;
        self.insert(
            "StorageFile",
            vec![
                ("ID", id.into()),
                ("OwnerID", owner.into()),
                ("OwnerType", owner_type.into()),
                ("StorageFolderID", folder_id.into()),
                ("FileName", name.into()),
            ],
        );
        self.plan
            .add
            .push(crate::structural::data::FileToWrite { path, bytes });
        Ok(id)
    }

    /// Deletes the rows that hang off object (`kind`, `id`) through `ObjectType`/`ObjectID`.
    pub(crate) fn drop_dependents(&mut self, kind: i64, id: i64) {
        for table in [
            "Annotation",
            "ExtendedProperty",
            "ChangedProperty",
            "ObjectTranslation",
            "ExcludedArtifact",
        ] {
            for d in self.ids_matching(table, |r| {
                r.int("ObjectType") == Some(kind) && r.int("ObjectID") == Some(id)
            }) {
                self.delete(table, d);
            }
        }
    }

    /// The final rows: `DBPROPERTIES.MAXID` raised to the last allocated ID.
    pub(crate) fn finish(mut self) -> (MetadataTables, StoragePlan) {
        if self.allocated
            && let Some(rows) = self.t.rows_mut("DBPROPERTIES")
            && let Some(row) = rows.iter_mut().find(|r| is_maxid(r))
        {
            let key = column_key(row, "VALUE").unwrap_or("VALUE").to_string();
            row.insert(key, MetaValue::Integer(self.max_id));
        }
        (self.t, self.plan)
    }
}

fn is_maxid(r: &MetaRow) -> bool {
    Row(r)
        .text("NAME")
        .is_some_and(|n| n.eq_ignore_ascii_case("MAXID"))
}

/// The integer every row holds in `col`, or NULL.
fn shared_integer(rows: &[MetaRow], col: &str) -> MetaValue {
    let mut values = rows.iter().map(|r| Row(r).get(col));
    match values.next().flatten() {
        Some(v @ MetaValue::Integer(_)) if values.all(|x| x == Some(v)) => v.clone(),
        _ => MetaValue::Null,
    }
}
