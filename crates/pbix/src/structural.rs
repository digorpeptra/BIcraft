//! Structural model edits of a PBIX `DataModel`: tables, columns and relationships added,
//! removed, renamed or given new data, written with their VertiPaq storage.
//!
//! [`DataModelEditor::open`] decompresses the part once. [`DataModelEditor::changes`] diffs an
//! edited model against the stored one ([`Changes`]); the caller supplies the values of every
//! new table and column ([`StructuralData`]) and [`DataModelEditor::write`] produces the new
//! part:
//!
//! 1. Values are encoded with `bicraft-vertipaq` (dictionary or value encoding, `.idf`,
//!    `.idfmeta`, `.hidx`), with the `H$` attribute hierarchy of each column and the `R$` index
//!    of each new relationship (keys read from the new values or decoded from the backup).
//! 2. `bicraft-metadata` writes the storage rows (`write_structural_sqlitedb_renamed_at`),
//!    renames first, and lists the ABF files to remove and add.
//! 3. A table whose partition changed (new data for an existing table) is replaced in two
//!    stages: removed with the relationships that touch it, then added back with the same
//!    name, lineage tags, measures and relationships.
//! 4. The ABF is re-packed (`AbfWriter`), re-compressed like the original part and read back.
//!
//! Without structural changes, [`DataModelEditor::write`] is the metadata-only writer.
//! See `docs/formats/pbix-package.md`, "Structural saves".

use std::collections::{HashMap, HashSet};

use bicraft_metadata as md;
use bicraft_metadata::{MetaRow, MetaValue, MetadataTables};
use bicraft_model::{Column, DataType, Relationship, SemanticModel, Table};
use bicraft_vertipaq::encode::{EncodeOptions, EncodingRequest, ValueParams};
use bicraft_vertipaq::system::{
    AttributeHierarchy, AttributeHierarchyOptions, Collation, TextStorage, UserHierarchyLevel,
    build_attribute_hierarchy_with, build_user_hierarchy,
};
use bicraft_vertipaq::{
    CellValue, ColumnEncoding, Dictionary, EncodingKind, LogicalType, StorageType, build_relationship_index,
    decode_data_ids, encode_column, encode_row_number, ids_to_values, parse_dictionary, parse_idfmeta,
};
use serde::Serialize;

use crate::{DataModelLimits, Error, decompress_data_model, metadata_error};

pub use bicraft_vertipaq::CellValue as Cell;

/// Most rows of one table written by a structural save.
pub const MAX_ROWS: usize = 1 << 24;

/// Values of one column of a new (or replaced) table, one per row.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnValues {
    pub name: String,
    pub values: Vec<CellValue>,
}

/// Values of every column of a new or replaced table (the row-number column excluded).
#[derive(Debug, Clone, PartialEq)]
pub struct TableValues {
    pub name: String,
    pub columns: Vec<ColumnValues>,
}

/// Values of a new column of an existing table, one per stored row.
#[derive(Debug, Clone, PartialEq)]
pub struct NewColumnValues {
    pub table: String,
    pub column: String,
    pub values: Vec<CellValue>,
}

/// The data a structural write needs: see [`Changes::tables_needing_data`] and
/// [`Changes::added_columns`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StructuralData {
    pub tables: Vec<TableValues>,
    pub columns: Vec<NewColumnValues>,
}

/// Rows of each partition of a new or replaced table of several partitions (model order): the
/// values of its [`TableValues`] are those partitions' rows concatenated. See
/// [`DataModelEditor::write_partitioned`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartitionRows {
    pub table: String,
    pub rows: Vec<u64>,
}

/// Structural differences between the stored model and an edited one. Names are the edited
/// model's, except for removals (the stored names).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Changes {
    pub added_tables: Vec<String>,
    pub removed_tables: Vec<String>,
    /// Existing tables whose partitions changed: their data is replaced.
    pub replaced_tables: Vec<String>,
    /// (old, new).
    pub renamed_tables: Vec<(String, String)>,
    /// (table, column) of kept tables.
    pub added_columns: Vec<(String, String)>,
    pub removed_columns: Vec<(String, String)>,
    /// (table, old column name, new column name), the table by its new name.
    pub renamed_columns: Vec<(String, String, String)>,
    pub added_relationships: Vec<String>,
    pub removed_relationships: Vec<String>,
    /// (table, hierarchy) of user hierarchies added to new or kept tables, or restructured
    /// (levels changed: also in `removed_hierarchies`).
    pub added_hierarchies: Vec<(String, String)>,
    /// (table, hierarchy) of kept tables, the table by its new name.
    pub removed_hierarchies: Vec<(String, String)>,
}

impl Changes {
    /// No structural change: a metadata-only save.
    pub fn is_empty(&self) -> bool {
        self.added_tables.is_empty()
            && self.removed_tables.is_empty()
            && self.replaced_tables.is_empty()
            && self.renamed_tables.is_empty()
            && self.added_columns.is_empty()
            && self.removed_columns.is_empty()
            && self.renamed_columns.is_empty()
            && self.added_relationships.is_empty()
            && self.removed_relationships.is_empty()
            && self.added_hierarchies.is_empty()
            && self.removed_hierarchies.is_empty()
    }

    /// Tables whose every column needs values: added and replaced ones.
    pub fn tables_needing_data(&self) -> impl Iterator<Item = &String> {
        self.added_tables.iter().chain(&self.replaced_tables)
    }
}

/// A decompressed `DataModel` part, ready for structural writes.
pub struct DataModelEditor {
    info: bicraft_xpress9::DataModelInfo,
    abf: Vec<u8>,
    base: SemanticModel,
}

fn eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn find<'a, T>(items: &'a [T], name: &str, get: impl Fn(&T) -> &str) -> Option<&'a T> {
    items
        .iter()
        .find(|x| get(x) == name)
        .or_else(|| items.iter().find(|x| eq(get(x), name)))
}

fn structural(msg: impl Into<String>) -> Error {
    Error::Structural(msg.into())
}

/// Refusals of the structural metadata writer, listed.
fn structural_error(e: md::Error) -> Error {
    match e {
        md::Error::UnsupportedChange(list) => Error::Structural(
            list.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
        ),
        other => Error::Metadata(other),
    }
}

/// The stored table an edited table stands for: same name, else the same lineage tag (a rename).
fn base_table<'a>(base: &'a SemanticModel, model: &SemanticModel, t: &Table) -> Option<&'a Table> {
    if let Some(b) = find(&base.tables, &t.name, |x| &x.name) {
        // A stored table with this name, unless it was renamed away and another took the name.
        let renamed_away = b.lineage_tag.is_some()
            && b.lineage_tag != t.lineage_tag
            && model
                .tables
                .iter()
                .any(|o| !std::ptr::eq(o, t) && o.lineage_tag == b.lineage_tag);
        if !renamed_away {
            return Some(b);
        }
    }
    let tag = t.lineage_tag.as_deref()?;
    base.tables.iter().find(|b| {
        b.lineage_tag.as_deref() == Some(tag) && find(&model.tables, &b.name, |x| &x.name).is_none()
    })
}

fn base_column<'a>(b: &'a Table, t: &Table, c: &Column) -> Option<&'a Column> {
    if let Some(o) = find(&b.columns, &c.name, |x| &x.name) {
        return Some(o);
    }
    let tag = c.lineage_tag.as_deref()?;
    b.columns
        .iter()
        .find(|o| o.lineage_tag.as_deref() == Some(tag) && find(&t.columns, &o.name, |x| &x.name).is_none())
}

fn is_row_number(c: &Column) -> bool {
    c.extra.get("type").and_then(|v| v.as_str()) == Some("rowNumber")
}

/// Structural diff (see [`Changes`]).
pub fn diff(base: &SemanticModel, model: &SemanticModel) -> Changes {
    let mut ch = Changes::default();
    let mut kept = HashSet::new();
    for t in &model.tables {
        let Some(b) = base_table(base, model, t) else {
            ch.added_tables.push(t.name.clone());
            for h in md::user_hierarchies(t) {
                ch.added_hierarchies.push((t.name.clone(), h.name));
            }
            continue;
        };
        kept.insert(b.name.to_lowercase());
        if b.name != t.name {
            ch.renamed_tables.push((b.name.clone(), t.name.clone()));
        }
        // A calculation group's rows are its items: other items (names, order) are new data.
        if b.partitions != t.partitions || calculation_rows(b) != calculation_rows(t) {
            ch.replaced_tables.push(t.name.clone());
            continue;
        }
        let mut kept_cols = HashSet::new();
        for c in &t.columns {
            match base_column(b, t, c) {
                None => ch.added_columns.push((t.name.clone(), c.name.clone())),
                Some(o) => {
                    kept_cols.insert(o.name.to_lowercase());
                    if o.name != c.name {
                        ch.renamed_columns
                            .push((t.name.clone(), o.name.clone(), c.name.clone()));
                    }
                }
            }
        }
        for o in &b.columns {
            if !kept_cols.contains(&o.name.to_lowercase()) {
                ch.removed_columns.push((t.name.clone(), o.name.clone()));
            }
        }
        hierarchy_changes(b, t, &mut ch);
    }
    for b in &base.tables {
        if !kept.contains(&b.name.to_lowercase()) {
            ch.removed_tables.push(b.name.clone());
        }
    }
    // A relationship whose cardinality or ends changed (`relationship.set`) keeps its name; its
    // storage (the `R$` indexes, the hash-encoded "one" ends) is rebuilt: removed, then added.
    let reshaped = |o: &bicraft_model::Relationship, r: &bicraft_model::Relationship| {
        let card = |c: &Option<String>, d: &str| c.as_deref().unwrap_or(d).to_ascii_lowercase();
        card(&o.from_cardinality, "many") != card(&r.from_cardinality, "many")
            || card(&o.to_cardinality, "one") != card(&r.to_cardinality, "one")
            || o.from_table != r.from_table
            || o.from_column != r.from_column
            || o.to_table != r.to_table
            || o.to_column != r.to_column
    };
    // A renamed relationship (`relationship.set {new_name}`) is kept: its row is renamed.
    let renames = model.relationship_renames(base);
    let same = |o: &bicraft_model::Relationship, r: &bicraft_model::Relationship| {
        (o.name == r.name || renames.iter().any(|(a, b)| *a == o.name && *b == r.name)) && !reshaped(o, r)
    };
    for o in &base.relationships {
        if !model.relationships.iter().any(|r| same(o, r)) {
            ch.removed_relationships.push(o.name.clone());
        }
    }
    for r in &model.relationships {
        if !base.relationships.iter().any(|o| same(o, r)) {
            ch.added_relationships.push(r.name.clone());
        }
    }
    ch
}

/// The rows of a calculation group table: its items' (name, ordinal); empty for other tables.
pub fn calculation_rows(t: &Table) -> Vec<(String, i64)> {
    let mut rows: Vec<(String, i64)> = t
        .extra
        .get("calculationGroup")
        .and_then(|g| g.get("calculationItems"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .map(|i| {
            (
                i.get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                i.get("ordinal").and_then(serde_json::Value::as_i64).unwrap_or(0),
            )
        })
        .collect();
    rows.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    rows
}

/// Hierarchies of a kept table added, removed or restructured. Stored level columns are named
/// by the edited model (renamed columns follow their lineage tag), so a column rename alone
/// is no structural change.
fn hierarchy_changes(b: &Table, t: &Table, ch: &mut Changes) {
    let current = |stored: &str| -> String {
        t.columns
            .iter()
            .find(|c| base_column(b, t, c).is_some_and(|o| eq(&o.name, stored)))
            .map_or_else(|| stored.to_string(), |c| c.name.clone())
    };
    let before: Vec<md::UserHierarchyDef> = md::user_hierarchies(b)
        .into_iter()
        .map(|mut h| {
            for l in &mut h.levels {
                l.1 = current(&l.1);
            }
            h
        })
        .collect();
    let after = md::user_hierarchies(t);
    let same = |a: &md::UserHierarchyDef, b: &md::UserHierarchyDef| {
        a.levels.len() == b.levels.len()
            && a.levels
                .iter()
                .zip(&b.levels)
                .all(|((an, ac), (bn, bc))| eq(an, bn) && eq(ac, bc))
    };
    for h in &before {
        if !find(&after, &h.name, |x| &x.name).is_some_and(|n| same(h, n)) {
            ch.removed_hierarchies.push((t.name.clone(), h.name.clone()));
        }
    }
    for h in &after {
        if !find(&before, &h.name, |x| &x.name).is_some_and(|o| same(o, h)) {
            ch.added_hierarchies.push((t.name.clone(), h.name.clone()));
        }
    }
}

impl DataModelEditor {
    /// Decompresses `part` (within `limits`) and reads its model.
    pub fn open(part: &[u8], limits: &DataModelLimits) -> Result<Self, Error> {
        let info = bicraft_xpress9::inspect_datamodel(part)?;
        let abf = decompress_data_model(part, limits)?;
        let base = {
            let parsed = bicraft_abf::Abf::parse(&abf)?;
            let sqlitedb = parsed.metadata_sqlitedb().ok_or(Error::NoMetadata)?;
            md::read_sqlitedb(sqlitedb)?
        };
        Ok(Self { info, abf, base })
    }

    /// The stored model.
    pub fn base(&self) -> &SemanticModel {
        &self.base
    }

    /// Structural differences of `model` from the stored model.
    pub fn changes(&self, model: &SemanticModel) -> Changes {
        diff(&self.base, model)
    }

    /// [`changes`](Self::changes) plus a refresh of `refresh` (Desktop's "Refresh data"): each
    /// of those tables that the stored model has and `model` keeps is replaced (new data, same
    /// definition), as a table whose partitions changed is. Unknown names are left out.
    pub fn changes_refreshing(&self, model: &SemanticModel, refresh: &[String]) -> Changes {
        let mut ch = diff(&self.base, model);
        for name in refresh {
            let Some(t) = find(&model.tables, name, |t| &t.name) else {
                continue;
            };
            let kept = base_table(&self.base, model, t).is_some();
            let listed = ch
                .added_tables
                .iter()
                .chain(&ch.replaced_tables)
                .any(|x| eq(x, &t.name));
            if kept && !listed {
                ch.replaced_tables.push(t.name.clone());
            }
        }
        ch
    }

    /// Rows of each stored partition of `table`, in storage order (`None` when it has no
    /// stored data).
    pub fn partition_rows(&self, table: &str) -> Result<Option<Vec<u64>>, Error> {
        let abf = bicraft_abf::Abf::parse(&self.abf)?;
        let sqlitedb = abf.metadata_sqlitedb().ok_or(Error::NoMetadata)?;
        let t = md::tables_from_sqlitedb_all(sqlitedb)?;
        Ok(Store::new(&t, &abf, &HashMap::new()).partition_rows(table))
    }

    /// Bytes of the decompressed backup the editor holds in memory.
    pub fn backup_len(&self) -> usize {
        self.abf.len()
    }

    /// Rows stored for `table` (sum of its partitions), `None` when it has no stored data.
    pub fn row_count(&self, table: &str) -> Result<Option<u64>, Error> {
        let abf = bicraft_abf::Abf::parse(&self.abf)?;
        let sqlitedb = abf.metadata_sqlitedb().ok_or(Error::NoMetadata)?;
        let t = md::tables_from_sqlitedb_all(sqlitedb)?;
        Ok(Store::new(&t, &abf, &HashMap::new()).table_rows(table))
    }

    /// The stored values of a column, one per row (partitions in storage order).
    pub fn column_values(&self, table: &str, column: &str) -> Result<Vec<CellValue>, Error> {
        let abf = bicraft_abf::Abf::parse(&self.abf)?;
        let sqlitedb = abf.metadata_sqlitedb().ok_or(Error::NoMetadata)?;
        let t = md::tables_from_sqlitedb_all(sqlitedb)?;
        let overlay = HashMap::new();
        let s = Store::new(&t, &abf, &overlay)
            .column(table, column)
            .map_err(structural)?;
        ids_to_values(&s.ids, s.dictionary.as_ref(), s.encoding)
            .map_err(|e| structural(format!("'{table}'[{column}]: {e}")))
    }

    /// Writes `model` with `data` into a new `DataModel` part (see the module documentation).
    pub fn write(&self, model: &SemanticModel, data: &StructuralData) -> Result<Vec<u8>, Error> {
        self.write_at(model, data, None)
    }

    /// [`write`](Self::write) with an explicit clock (Windows FILETIME ticks).
    pub fn write_at(
        &self,
        model: &SemanticModel,
        data: &StructuralData,
        now: Option<i64>,
    ) -> Result<Vec<u8>, Error> {
        self.write_partitioned(model, data, &[], now)
    }

    /// [`write_at`](Self::write_at) for new or replaced tables of several partitions: each
    /// partition gets its own segments (one dictionary, `.hidx` and `H$` per column; row
    /// numbers continue across partitions), `partitions` giving each one's rows.
    pub fn write_partitioned(
        &self,
        model: &SemanticModel,
        data: &StructuralData,
        partitions: &[PartitionRows],
        now: Option<i64>,
    ) -> Result<Vec<u8>, Error> {
        self.write_refreshed(model, data, partitions, &[], now)
    }

    /// [`write_partitioned`](Self::write_partitioned) with the tables `refresh` given new data
    /// although their definition did not change ([`changes_refreshing`](Self::changes_refreshing)):
    /// `data` holds every column of each of them. The other tables' storage files are kept as
    /// they are; the relationships that touch a refreshed table get new indexes.
    pub fn write_refreshed(
        &self,
        model: &SemanticModel,
        data: &StructuralData,
        partitions: &[PartitionRows],
        refresh: &[String],
        now: Option<i64>,
    ) -> Result<Vec<u8>, Error> {
        let abf = bicraft_abf::Abf::parse(&self.abf)?;
        let sqlitedb = abf.metadata_sqlitedb().ok_or(Error::NoMetadata)?;
        let changes = self.changes_refreshing(model, refresh);
        let mut removes: Vec<String> = Vec::new();
        let mut adds: Vec<md::FileToWrite> = Vec::new();
        let new_sqlitedb = if changes.is_empty() {
            match now {
                Some(t) => md::write_sqlitedb_at(sqlitedb, model, t),
                None => md::write_sqlitedb(sqlitedb, model),
            }
            .map_err(metadata_error)?
        } else {
            let now = match now {
                Some(t) => t,
                None => filetime_now()?,
            };
            let culture = model.culture.as_deref().unwrap_or("en-US");
            let mut enc = Encoder {
                model,
                data,
                partitions,
                collation: Collation::for_culture(culture),
                keys: TextStorage::for_culture(culture, false),
                cache: HashMap::new(),
            };
            enc.check_data(&changes)?;
            let renames = md::Renames {
                tables: changes.renamed_tables.clone(),
                columns: changes
                    .renamed_columns
                    .iter()
                    .map(|(t, old, new)| {
                        let old_table = changes
                            .renamed_tables
                            .iter()
                            .find(|(_, n)| n == t)
                            .map_or(t.clone(), |(o, _)| o.clone());
                        (old_table, old.clone(), new.clone())
                    })
                    .collect(),
            };
            let mut stages = Vec::new();
            if !changes.replaced_tables.is_empty() {
                stages.push(without_tables(model, &changes.replaced_tables));
            }
            stages.push(model.clone());
            let mut current = sqlitedb.to_vec();
            let mut overlay: HashMap<String, Vec<u8>> = HashMap::new();
            for (i, stage) in stages.iter().enumerate() {
                let tables = md::tables_from_sqlitedb_all(&current)?;
                let stage_base = md::build_model(&tables)?;
                let stage_changes = if i == 0 {
                    diff(&self.base, stage)
                } else {
                    diff(&stage_base, stage)
                };
                let store = Store::new(&tables, &abf, &overlay);
                let store = if i == 0 {
                    store.with_renames(&changes)
                } else {
                    store
                };
                let encoded = enc.encode_stage(&stage_changes, stage, &store)?;
                let empty = md::Renames::default();
                let w = md::write_structural_sqlitedb_renamed_at(
                    &current,
                    if i == 0 { &renames } else { &empty },
                    stage,
                    &encoded,
                    now,
                )
                .map_err(structural_error)?;
                for r in w.plan.remove {
                    let key = r.replace('\\', "/").to_lowercase();
                    if overlay.remove(&key).is_some() {
                        adds.retain(|a| a.path.replace('\\', "/").to_lowercase() != key);
                    } else {
                        removes.push(r);
                    }
                }
                for a in w.plan.add {
                    overlay.insert(a.path.replace('\\', "/").to_lowercase(), a.bytes.clone());
                    adds.push(a);
                }
                current = w.sqlitedb;
            }
            current
        };
        let mut writer = abf.to_writer()?;
        for r in &removes {
            writer.remove(r)?;
        }
        for a in adds {
            writer.add(&a.path, a.bytes)?;
        }
        if new_sqlitedb.as_slice() != sqlitedb {
            writer.replace_metadata_sqlitedb(new_sqlitedb)?;
        }
        let new_abf = writer.finish()?;
        if self.info.layout == bicraft_xpress9::Layout::Uncompressed {
            return Ok(new_abf);
        }
        let out = bicraft_xpress9::compress_datamodel(&new_abf, Some(&self.info))?;
        if !bicraft_xpress9::verify_datamodel(&out, &new_abf)? {
            return Err(Error::Verify(
                "the compressed DataModel does not decode to the written backup",
            ));
        }
        Ok(out)
    }
}

/// Windows FILETIME ticks now.
fn filetime_now() -> Result<i64, Error> {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| structural("the system clock is before 1970"))?;
    let ticks = i64::try_from(since.as_nanos() / 100).map_err(|_| structural("clock overflow"))?;
    ticks
        .checked_add(116_444_736_000_000_000)
        .ok_or_else(|| structural("clock overflow"))
}

/// `model` without `tables`, the relationships that touch them and the role permissions on them.
fn without_tables(model: &SemanticModel, tables: &[String]) -> SemanticModel {
    let gone = |n: &str| tables.iter().any(|t| eq(t, n));
    let mut m = model.clone();
    m.tables.retain(|t| !gone(&t.name));
    m.relationships
        .retain(|r| !gone(&r.from_table) && !gone(&r.to_table));
    for role in &mut m.roles {
        role.table_permissions.retain(|p| !gone(&p.table));
    }
    // Perspective memberships and translations of the tables (the next stage brings them back).
    let named_gone = |v: &serde_json::Value| v.get("name").and_then(|n| n.as_str()).is_some_and(gone);
    for p in m
        .extra
        .get_mut("perspectives")
        .and_then(|v| v.as_array_mut())
        .into_iter()
        .flatten()
    {
        if let Some(t) = p.get_mut("tables").and_then(|v| v.as_array_mut()) {
            t.retain(|x| !named_gone(x));
        }
    }
    for c in m
        .extra
        .get_mut("cultures")
        .and_then(|v| v.as_array_mut())
        .into_iter()
        .flatten()
    {
        if let Some(t) = c
            .pointer_mut("/translations/model/tables")
            .and_then(|v| v.as_array_mut())
        {
            t.retain(|x| !named_gone(x));
        }
    }
    m
}

// ---------------------------------------------------------------------------------------------
// stored columns
// ---------------------------------------------------------------------------------------------

fn int(r: &MetaRow, col: &str) -> Option<i64> {
    match r.get(col)? {
        MetaValue::Integer(i) => Some(*i),
        MetaValue::Real(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < 9.0e15 => Some(*f as i64),
        _ => None,
    }
}

fn real(r: &MetaRow, col: &str) -> Option<f64> {
    match r.get(col)? {
        MetaValue::Integer(i) => Some(*i as f64),
        MetaValue::Real(f) => Some(*f),
        _ => None,
    }
}

fn text<'r>(r: &'r MetaRow, col: &str) -> Option<&'r str> {
    match r.get(col)? {
        MetaValue::Text(s) if !s.is_empty() => Some(s),
        _ => None,
    }
}

/// A stored column: data ids per row (partitions in storage order) and how to read them.
struct StoredColumn {
    ids: Vec<u32>,
    encoding: ColumnEncoding,
    dictionary: Option<Dictionary>,
}

/// Reads stored columns: metadata rows of the current stage, files from the original backup or
/// added by an earlier stage.
struct Store<'s> {
    t: &'s MetadataTables,
    abf: &'s bicraft_abf::Abf<'s>,
    overlay: &'s HashMap<String, Vec<u8>>,
    /// Renames not applied to `t` yet: new table name → stored name, (new table, new column) →
    /// stored column name (lower-cased keys).
    tables_renamed: HashMap<String, String>,
    columns_renamed: HashMap<(String, String), String>,
}

impl<'s> Store<'s> {
    fn new(
        t: &'s MetadataTables,
        abf: &'s bicraft_abf::Abf<'s>,
        overlay: &'s HashMap<String, Vec<u8>>,
    ) -> Self {
        Self {
            t,
            abf,
            overlay,
            tables_renamed: HashMap::new(),
            columns_renamed: HashMap::new(),
        }
    }

    /// Names of the edited model resolved through pending renames (see [`Changes`]).
    fn with_renames(mut self, ch: &Changes) -> Self {
        for (old, new) in &ch.renamed_tables {
            self.tables_renamed.insert(new.to_lowercase(), old.clone());
        }
        for (table, old, new) in &ch.renamed_columns {
            self.columns_renamed
                .insert((table.to_lowercase(), new.to_lowercase()), old.clone());
        }
        self
    }

    fn stored_names(&self, table: &str, column: &str) -> (String, String) {
        let t = self
            .tables_renamed
            .get(&table.to_lowercase())
            .cloned()
            .unwrap_or_else(|| table.to_string());
        let c = self
            .columns_renamed
            .get(&(table.to_lowercase(), column.to_lowercase()))
            .cloned()
            .unwrap_or_else(|| column.to_string());
        (t, c)
    }

    fn row(&self, table: &str, id: i64) -> Option<&'s MetaRow> {
        self.t.rows(table).iter().find(|r| int(r, "ID") == Some(id))
    }

    fn table_id(&self, name: &str) -> Option<i64> {
        let name = self
            .tables_renamed
            .get(&name.to_lowercase())
            .map_or(name, String::as_str);
        let users: Vec<&MetaRow> = self
            .t
            .rows("Table")
            .iter()
            .filter(|r| int(r, "SystemFlags").unwrap_or(0) & 1 == 0)
            .collect();
        users
            .iter()
            .find(|r| text(r, "Name") == Some(name))
            .or_else(|| {
                users
                    .iter()
                    .find(|r| text(r, "Name").is_some_and(|n| eq(n, name)))
            })
            .and_then(|r| int(r, "ID"))
    }

    /// Rows of each partition of `table`, in storage order.
    fn partition_rows(&self, table: &str) -> Option<Vec<u64>> {
        let tid = self.table_id(table)?;
        let mut parts = Vec::new();
        for p in self
            .t
            .rows("Partition")
            .iter()
            .filter(|p| int(p, "TableID") == Some(tid))
        {
            let ps = int(p, "PartitionStorageID").and_then(|ps| self.row("PartitionStorage", ps))?;
            let n = int(ps, "SegmentMapStorageID")
                .and_then(|sm| self.row("SegmentMapStorage", sm))
                .and_then(|sm| int(sm, "RecordCount"))
                .and_then(|n| u64::try_from(n).ok())?;
            parts.push((
                int(ps, "StoragePosition").unwrap_or(0),
                int(ps, "ID").unwrap_or(0),
                n,
            ));
        }
        parts.sort_unstable();
        Some(parts.into_iter().map(|(_, _, n)| n).collect())
    }

    fn table_rows(&self, table: &str) -> Option<u64> {
        let tid = self.table_id(table)?;
        let mut total = 0u64;
        let mut any = false;
        for p in self
            .t
            .rows("Partition")
            .iter()
            .filter(|p| int(p, "TableID") == Some(tid))
        {
            let n = int(p, "PartitionStorageID")
                .and_then(|ps| self.row("PartitionStorage", ps))
                .and_then(|ps| int(ps, "SegmentMapStorageID"))
                .and_then(|sm| self.row("SegmentMapStorage", sm))
                .and_then(|sm| int(sm, "RecordCount"))
                .and_then(|n| u64::try_from(n).ok())?;
            total = total.saturating_add(n);
            any = true;
        }
        any.then_some(total)
    }

    fn file(&self, id: i64) -> Option<&'s [u8]> {
        let f = self.row("StorageFile", id)?;
        let name = text(f, "FileName")?;
        let folder = int(f, "StorageFolderID")
            .and_then(|fid| self.row("StorageFolder", fid))
            .and_then(|r| text(r, "Path"))
            .unwrap_or("");
        let path = if folder.is_empty() {
            name.to_string()
        } else {
            format!("{folder}/{name}")
        };
        let key = path.replace('\\', "/").to_lowercase();
        if let Some(b) = self.overlay.get(&key) {
            return Some(b.as_slice());
        }
        self.abf.read(&path)
    }

    /// The member order of a stored column's attribute hierarchy: the first
    /// `DistinctDataCount` values of its `H$` table's `POS_TO_ID` column.
    fn hierarchy_members(&self, table: &str, column: &str) -> Result<Vec<u32>, String> {
        let object = format!("'{table}'[{column}]");
        let missing = |what: &str| format!("{object}: stored {what} is missing");
        let (_, stored_column) = self.stored_names(table, column);
        let tid = self
            .table_id(table)
            .ok_or_else(|| format!("{object}: the table has no stored data"))?;
        let c = self
            .t
            .rows("Column")
            .iter()
            .filter(|c| int(c, "TableID") == Some(tid))
            .find(|c| {
                text(c, "ExplicitName")
                    .or_else(|| text(c, "InferredName"))
                    .is_some_and(|n| eq(n, &stored_column))
            })
            .ok_or_else(|| format!("{object}: the column has no stored data"))?;
        let ahs = int(c, "AttributeHierarchyID")
            .and_then(|a| self.row("AttributeHierarchy", a))
            .and_then(|a| int(a, "AttributeHierarchyStorageID"))
            .and_then(|a| self.row("AttributeHierarchyStorage", a))
            .ok_or_else(|| missing("attribute hierarchy"))?;
        let count = int(ahs, "DistinctDataCount")
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| format!("{object}: the column has no attribute hierarchy"))?;
        let sys = int(ahs, "SystemTableID")
            .filter(|&s| s != 0)
            .ok_or_else(|| format!("{object}: the column has no attribute hierarchy"))?;
        let pos = self
            .t
            .rows("Column")
            .iter()
            .filter(|c| int(c, "TableID") == Some(sys))
            .find(|c| text(c, "ExplicitName") == Some("POS_TO_ID"))
            .ok_or_else(|| missing("POS_TO_ID column"))?;
        let cs = int(pos, "ColumnStorageID").ok_or_else(|| missing("POS_TO_ID storage"))?;
        let cps = self
            .t
            .rows("ColumnPartitionStorage")
            .iter()
            .find(|r| int(r, "ColumnStorageID") == Some(cs))
            .ok_or_else(|| missing("POS_TO_ID segments"))?;
        let idf = int(cps, "StorageFileID")
            .and_then(|f| self.file(f))
            .ok_or_else(|| missing("POS_TO_ID file"))?;
        let meta = int(cps, "SegmentStorageID")
            .and_then(|s| self.row("SegmentStorage", s))
            .and_then(|s| int(s, "StorageFileID"))
            .and_then(|f| self.file(f))
            .ok_or_else(|| missing("POS_TO_ID statistics"))?;
        let meta = parse_idfmeta(meta).map_err(|e| format!("{object}: {e}"))?;
        let limits = bicraft_vertipaq::Limits {
            max_rows: MAX_ROWS as u64,
            ..bicraft_vertipaq::Limits::default()
        };
        let mut members = decode_data_ids(idf, &meta, &limits).map_err(|e| format!("{object}: {e}"))?;
        if members.len() < count {
            return Err(format!("{object}: the stored attribute hierarchy is truncated"));
        }
        members.truncate(count);
        Ok(members)
    }

    fn column(&self, table: &str, column: &str) -> Result<StoredColumn, String> {
        let object = format!("'{table}'[{column}]");
        let (_, stored_column) = self.stored_names(table, column);
        let column = stored_column.as_str();
        let tid = self
            .table_id(table)
            .ok_or_else(|| format!("{object}: the table has no stored data"))?;
        let c = self
            .t
            .rows("Column")
            .iter()
            .filter(|c| int(c, "TableID") == Some(tid))
            .find(|c| {
                text(c, "ExplicitName")
                    .or_else(|| text(c, "InferredName"))
                    .is_some_and(|n| eq(n, column))
            })
            .ok_or_else(|| format!("{object}: the column has no stored data"))?;
        let missing = |what: &str| format!("{object}: stored {what} is missing");
        let cs_id = int(c, "ColumnStorageID").ok_or_else(|| missing("column storage"))?;
        let cs = self
            .row("ColumnStorage", cs_id)
            .ok_or_else(|| missing("column storage"))?;
        let ds = int(cs, "DictionaryStorageID")
            .and_then(|d| self.row("DictionaryStorage", d))
            .ok_or_else(|| missing("dictionary storage"))?;
        let logical = int(c, "ExplicitDataType")
            .filter(|t| *t != 1)
            .or_else(|| int(c, "InferredDataType"))
            .and_then(LogicalType::from_code);
        let encoding = ColumnEncoding::from_dictionary_storage(
            int(ds, "Type").unwrap_or(-1),
            int(ds, "DataType").unwrap_or(-1),
            int(ds, "BaseId").unwrap_or(0),
            real(ds, "Magnitude").unwrap_or(1.0),
        )
        .map_err(|e| format!("{object}: {e}"))?
        .with_logical_type(logical);
        // A hash-encoded column holding only blanks has no `.dictionary` (`StorageFileID` 0);
        // `ids_to_values` refuses any other id without one.
        let dictionary = match (encoding.kind, int(ds, "StorageFileID").filter(|&f| f != 0)) {
            (EncodingKind::Hash, Some(f)) => {
                let bytes = self.file(f).ok_or_else(|| missing("dictionary"))?;
                Some(parse_dictionary(bytes).map_err(|e| format!("{object}: {e}"))?)
            }
            _ => None,
        };
        let mut parts: Vec<(i64, i64, i64)> = Vec::new();
        for cps in self
            .t
            .rows("ColumnPartitionStorage")
            .iter()
            .filter(|r| int(r, "ColumnStorageID") == Some(cs_id))
        {
            let idf = int(cps, "StorageFileID").ok_or_else(|| missing("segment file"))?;
            let meta = int(cps, "SegmentStorageID")
                .and_then(|s| self.row("SegmentStorage", s))
                .and_then(|s| int(s, "StorageFileID"))
                .ok_or_else(|| missing("segment statistics"))?;
            let order = int(cps, "PartitionStorageID")
                .and_then(|p| self.row("PartitionStorage", p))
                .and_then(|p| int(p, "StoragePosition"))
                .unwrap_or(0);
            parts.push((order, idf, meta));
        }
        parts.sort_by_key(|p| p.0);
        let limits = bicraft_vertipaq::Limits {
            max_rows: MAX_ROWS as u64,
            ..bicraft_vertipaq::Limits::default()
        };
        let mut ids = Vec::new();
        for (_, idf, meta) in parts {
            let idf = self.file(idf).ok_or_else(|| missing("segment file"))?;
            let meta = self.file(meta).ok_or_else(|| missing("segment statistics"))?;
            let meta = parse_idfmeta(meta).map_err(|e| format!("{object}: {e}"))?;
            let part = decode_data_ids(idf, &meta, &limits).map_err(|e| format!("{object}: {e}"))?;
            if ids.len().saturating_add(part.len()) > MAX_ROWS {
                return Err(format!("{object}: more than {MAX_ROWS} rows"));
            }
            ids.extend(part);
        }
        Ok(StoredColumn {
            ids,
            encoding,
            dictionary,
        })
    }
}

// ---------------------------------------------------------------------------------------------
// encoding
// ---------------------------------------------------------------------------------------------

/// A column encoded for a structural write, with what relationships and sort-by columns need.
#[derive(Clone)]
struct Encoded {
    storage: md::ColumnStorageData,
    /// Every distinct data id with its value (blank: id 2, `Null`).
    entries: Vec<(u32, CellValue)>,
    /// The data id of each row.
    ids: Vec<u32>,
    /// Attribute-hierarchy members in sort order (`None` without a hierarchy).
    members: Option<Vec<u32>>,
}

/// Keys (from data ids or values) of one relationship end.
struct End {
    /// Distinct (data id, value) of the column.
    entries: Vec<(u32, CellValue)>,
    /// Value of each row.
    rows: Vec<CellValue>,
}

struct Encoder<'e> {
    model: &'e SemanticModel,
    data: &'e StructuralData,
    partitions: &'e [PartitionRows],
    collation: Collation,
    /// Relationship key equality: the stored-text identity of the model's culture, applied to
    /// stored values (en-US: `a` = `A` but `à` ≠ `À`).
    keys: TextStorage,
    /// Encoded new columns by lower-cased (table, column), shared between stages.
    cache: HashMap<(String, String), Encoded>,
}

fn logical_type(t: DataType) -> Option<LogicalType> {
    Some(match t {
        DataType::String => LogicalType::String,
        DataType::Int64 => LogicalType::Int64,
        DataType::Double => LogicalType::Double,
        DataType::Decimal => LogicalType::Decimal,
        DataType::DateTime => LogicalType::DateTime,
        DataType::Boolean => LogicalType::Boolean,
        _ => return None,
    })
}

fn table_values<'d>(data: &'d StructuralData, table: &str) -> Option<&'d TableValues> {
    find(&data.tables, table, |t| &t.name)
}

impl<'e> Encoder<'e> {
    fn model_table(&self, name: &str) -> Option<&'e Table> {
        let model: &'e SemanticModel = self.model;
        find(&model.tables, name, |t| &t.name)
    }

    /// Rows of each partition of a new or replaced table: empty for one partition; for
    /// several, the caller's [`PartitionRows`], which must cover every row.
    fn partition_counts(&self, t: &Table, rows: usize) -> Result<Vec<u64>, Error> {
        if t.partitions.len() <= 1 {
            return Ok(Vec::new());
        }
        let object = format!("table '{}'", t.name);
        let counts = self
            .partitions
            .iter()
            .find(|p| eq(&p.table, &t.name))
            .map(|p| p.rows.clone())
            .ok_or_else(|| {
                structural(format!(
                    "{object}: {} partitions, but the rows of each were not given",
                    t.partitions.len()
                ))
            })?;
        let total = counts.iter().fold(0u64, |a, &n| a.saturating_add(n));
        if counts.len() != t.partitions.len() || total != rows as u64 {
            return Err(structural(format!(
                "{object}: {} partition row counts adding up to {total}, for {} partitions and {rows} rows",
                counts.len(),
                t.partitions.len()
            )));
        }
        Ok(counts)
    }

    /// Every table and column that needs data has it, with consistent row counts.
    fn check_data(&self, ch: &Changes) -> Result<(), Error> {
        let mut problems = Vec::new();
        for name in ch.tables_needing_data() {
            let Some(t) = self.model_table(name) else { continue };
            let Some(v) = table_values(self.data, name) else {
                problems.push(format!("table '{name}': no data was supplied"));
                continue;
            };
            let rows = v.columns.first().map_or(0, |c| c.values.len());
            if rows > MAX_ROWS {
                problems.push(format!("table '{name}': more than {MAX_ROWS} rows"));
            }
            for c in t.columns.iter().filter(|c| !is_row_number(c)) {
                match find(&v.columns, &c.name, |x| &x.name) {
                    None => problems.push(format!("column '{name}'[{}]: no data was supplied", c.name)),
                    Some(cv) if cv.values.len() != rows => problems.push(format!(
                        "column '{name}'[{}]: {} values, the table has {rows} rows",
                        c.name,
                        cv.values.len()
                    )),
                    Some(_) => {}
                }
                if logical_type(c.data_type).is_none() {
                    problems.push(format!(
                        "column '{name}'[{}]: data type {} cannot be stored",
                        c.name,
                        c.data_type.as_tmsl().unwrap_or("unknown")
                    ));
                }
            }
        }
        for (table, column) in &ch.added_columns {
            let has = self
                .data
                .columns
                .iter()
                .any(|c| eq(&c.table, table) && eq(&c.column, column));
            if !has {
                problems.push(format!("column '{table}'[{column}]: no data was supplied"));
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(structural(problems.join("; ")))
        }
    }

    /// How the engine stores the text of column `c` of `table`: per the model's culture, trimmed
    /// when it comes from an import partition (not from DAX).
    fn text_storage(&self, table: &str, c: &Column) -> TextStorage {
        let calculated_table = self.model_table(table).is_some_and(|t| {
            t.partitions
                .iter()
                .any(|p| p.source_type.eq_ignore_ascii_case("calculated"))
        });
        let imported = c.expression.is_none() && !calculated_table;
        TextStorage::for_culture(self.model.culture.as_deref().unwrap_or("en-US"), imported)
    }

    /// Whether `(table, column)` is stored as a unique "one" end of a relationship of the edited
    /// model (hash-encoded): the to end of a many-to-one relationship, either end of a
    /// one-to-one one.
    fn is_to_end(&self, table: &str, column: &str) -> bool {
        self.model.relationships.iter().any(|r| {
            let card = |c: &Option<String>, d: &str| c.as_deref().unwrap_or(d).to_ascii_lowercase();
            let (from_card, to_card) = (card(&r.from_cardinality, "many"), card(&r.to_cardinality, "one"));
            let to = eq(&r.to_table, table) && eq(&r.to_column, column);
            let from = eq(&r.from_table, table) && eq(&r.from_column, column);
            (to_card == "one" && to) || (from_card == "one" && to_card == "one" && from)
        })
    }

    /// The values of a new or replaced table's column, or of a new column.
    fn values_of(&self, table: &str, column: &str) -> Option<&'e [CellValue]> {
        let data: &'e StructuralData = self.data;
        if let Some(t) = table_values(data, table) {
            return find(&t.columns, column, |c| &c.name).map(|c| c.values.as_slice());
        }
        data.columns
            .iter()
            .find(|c| eq(&c.table, table) && eq(&c.column, column))
            .map(|c| c.values.as_slice())
    }

    /// A new column of a table of several partitions (incremental refresh policy ranges, hybrid
    /// tables): the segments of each partition (`counts` rows each, storage order), encoded with
    /// the whole column's dictionary order or value parameters so that every partition shares
    /// `e`'s dictionary, `.hidx` and `H$`. The `ColumnStorage` RLE statistics become the sums
    /// over every partition's segments, as the engine stores them.
    fn split_partitions(
        &self,
        table: &str,
        c: &Column,
        values: &[CellValue],
        counts: &[u64],
        e: &Encoded,
    ) -> Result<(md::ColumnStorageData, Vec<md::PartitionSegments>), Error> {
        let object = format!("column '{table}'[{}]", c.name);
        let fail = |e: &dyn std::fmt::Display| structural(format!("{object}: {e}"));
        let logical = logical_type(c.data_type).ok_or_else(|| fail(&"data type cannot be stored"))?;
        let total = counts.iter().fold(0u64, |a, &n| a.saturating_add(n));
        if total != values.len() as u64 {
            return Err(fail(&format!(
                "{} values, the partitions store {total} rows",
                values.len()
            )));
        }
        let canonical;
        let values = if logical == LogicalType::String {
            canonical = self.text_storage(table, c).canonicalize(values);
            canonical.as_slice()
        } else {
            values
        };
        let d = &e.storage.dictionary;
        let encoding = if d.kind == 1 {
            EncodingRequest::Hash
        } else {
            EncodingRequest::Value {
                params: Some(ValueParams {
                    base_id: d.base_id,
                    magnitude: d.magnitude,
                }),
            }
        };
        let seed: Vec<CellValue> = if d.kind == 1 {
            let mut entries: Vec<&(u32, CellValue)> = e.entries.iter().filter(|(id, _)| *id != 2).collect();
            entries.sort_by_key(|(id, _)| *id);
            entries.into_iter().map(|(_, v)| v.clone()).collect()
        } else {
            Vec::new()
        };
        let options = EncodeOptions {
            encoding,
            dictionary_seed: seed,
            segment_base_id: if c.expression.is_some() { 0 } else { 1 },
            max_rows: MAX_ROWS as u64,
            build_hidx: false,
            ..EncodeOptions::default()
        };
        let mut parts = Vec::with_capacity(counts.len());
        let (mut rle, mut others) = (0u64, 0u64);
        let mut at = 0usize;
        for &n in counts {
            let n = usize::try_from(n).map_err(|_| fail(&"partition too large"))?;
            let end = at.checked_add(n).ok_or_else(|| fail(&"partition too large"))?;
            let slice = values
                .get(at..end)
                .ok_or_else(|| fail(&"partition rows out of range"))?;
            at = end;
            let p = encode_column(slice, logical, &options).map_err(|e| fail(&e))?;
            // The seed holds every value: a partition with values sees the whole dictionary.
            if d.kind == 1 && p.stats.distinct_values > 0 && i64::from(p.stats.last_id) != d.last_id {
                return Err(fail(&"internal check failed: a partition's dictionary differs"));
            }
            rle = rle.saturating_add(p.stats.rle_runs());
            others = others.saturating_add(p.stats.others_rle_runs());
            parts.push(md::PartitionSegments {
                idf: p.idf,
                idfmeta: p.idfmeta,
                segment_count: to_i64(p.stats.segment_count()),
                rows: to_i64(n as u64),
            });
        }
        let mut data = e.storage.clone();
        data.statistics.rle_runs = to_i64(rle);
        data.statistics.others_rle_runs = to_i64(others);
        if let Some(first) = parts.first() {
            data.idf = first.idf.clone();
            data.idfmeta = first.idfmeta.clone();
            data.segment_count = first.segment_count;
        }
        Ok((data, parts))
    }

    fn encode(
        &mut self,
        table: &str,
        c: &Column,
        values: &[CellValue],
        store: &Store<'_>,
    ) -> Result<Encoded, Error> {
        let key = (table.to_lowercase(), c.name.to_lowercase());
        if let Some(e) = self.cache.get(&key) {
            return Ok(e.clone());
        }
        let object = format!("column '{table}'[{}]", c.name);
        let fail = |e: &dyn std::fmt::Display| structural(format!("{object}: {e}"));
        let logical = logical_type(c.data_type).ok_or_else(|| fail(&"data type cannot be stored"))?;
        let options = EncodeOptions {
            encoding: if self.is_to_end(table, &c.name) {
                EncodingRequest::Hash
            } else {
                EncodingRequest::Auto
            },
            segment_base_id: if c.expression.is_some() { 0 } else { 1 },
            max_rows: MAX_ROWS as u64,
            ..EncodeOptions::default()
        };
        // Text as the engine stores it: one dictionary entry per equivalent string (the first one
        // seen), so "a" after "A" reads back as "A"; imported values lose trailing blanks.
        let canonical;
        let values = if logical == LogicalType::String {
            canonical = self.text_storage(table, c).canonicalize(values);
            canonical.as_slice()
        } else {
            values
        };
        let enc = encode_column(values, logical, &options).map_err(|e| fail(&e))?;
        let s = &enc.stats;
        // Read the written column back: the data ids every row got.
        let meta = parse_idfmeta(&enc.idfmeta).map_err(|e| fail(&e))?;
        let limits = bicraft_vertipaq::Limits {
            max_rows: MAX_ROWS as u64,
            ..bicraft_vertipaq::Limits::default()
        };
        let ids = decode_data_ids(&enc.idf, &meta, &limits).map_err(|e| fail(&e))?;
        let dictionary = match &enc.dictionary {
            Some(d) => Some(parse_dictionary(d).map_err(|e| fail(&e))?),
            None => None,
        };
        let encoding = match s.kind {
            EncodingKind::Hash => ColumnEncoding::hash(s.storage_type),
            _ => ColumnEncoding::value(s.base_id, s.magnitude, s.storage_type),
        }
        .with_logical_type(Some(logical));
        let entries = distinct_entries(&ids, dictionary.as_ref(), encoding).map_err(|e| fail(&e))?;
        // Round trip: the stored values must be the given ones.
        let back = ids_to_values(&ids, dictionary.as_ref(), encoding).map_err(|e| fail(&e))?;
        if back.len() != values.len() || back.iter().zip(values).any(|(a, b)| !same_cell(a, b)) {
            return Err(fail(
                &"internal check failed: the encoded column does not decode to its values",
            ));
        }

        let in_mdx = md::column_available_in_mdx(&c.extra);
        let mut members = None;
        let hierarchy = if in_mdx && values.is_empty() {
            // A column without rows: no dictionary file, no `H$` table, nothing materialized,
            // as the engine stores an empty table (`datamodel-fresh.md`, "Tables without rows").
            members = Some(Vec::new());
            Some(md::AttributeHierarchyData {
                materialization_type: 2,
                distinct_data_count: 0,
                min_value: None,
                max_value: None,
                string_value_max_length: 0,
                hidx: None,
                system_table: None,
            })
        } else if in_mdx {
            let sort_keys = match c.sort_by_column.as_deref() {
                Some(by) => Some(self.sort_keys(table, by, &ids, store)?),
                None => None,
            };
            let h = build_attribute_hierarchy_with(
                &entries,
                &self.collation,
                &AttributeHierarchyOptions {
                    sort_by: sort_keys.as_deref(),
                    id_to_pos: s.kind == EncodingKind::Hash,
                },
            )
            .map_err(|e| fail(&e))?;
            members = Some(h.members.clone());
            Some(md::AttributeHierarchyData {
                materialization_type: h.materialization_type,
                distinct_data_count: to_i64(h.distinct_data_count),
                min_value: h.min_value.as_ref().map(value_text),
                max_value: h.max_value.as_ref().map(value_text),
                string_value_max_length: to_i64(h.string_value_max_length),
                hidx: if s.kind == EncodingKind::Hash {
                    None
                } else {
                    enc.hidx.clone()
                },
                system_table: Some(system_table(&h.table).map_err(|e| fail(&e))?),
            })
        } else {
            None
        };
        let mut dictionary = dictionary_data(&enc);
        if values.is_empty() {
            dictionary.file = None;
        }
        let storage = md::ColumnStorageData {
            idf: enc.idf.clone(),
            idfmeta: enc.idfmeta.clone(),
            segment_count: to_i64(s.segment_count()),
            dictionary,
            statistics: statistics(s),
            hierarchy,
        };
        let out = Encoded {
            storage,
            entries,
            ids,
            members,
        };
        self.cache.insert(key, out.clone());
        Ok(out)
    }

    /// Sort key of each data id: the sort-by column's value on the first row carrying it.
    fn sort_keys(
        &self,
        table: &str,
        by: &str,
        ids: &[u32],
        store: &Store<'_>,
    ) -> Result<Vec<(u32, CellValue)>, Error> {
        let values: Vec<CellValue> = match self.values_of(table, by) {
            Some(v) => v.to_vec(),
            None => {
                let s = store.column(table, by).map_err(structural)?;
                ids_to_values(&s.ids, s.dictionary.as_ref(), s.encoding)
                    .map_err(|e| structural(format!("'{table}'[{by}]: {e}")))?
            }
        };
        if values.len() != ids.len() {
            return Err(structural(format!(
                "column '{table}'[{by}] (sort-by column) has {} rows, not {}",
                values.len(),
                ids.len()
            )));
        }
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for (id, v) in ids.iter().zip(values) {
            if seen.insert(*id) {
                out.push((*id, v));
            }
        }
        Ok(out)
    }

    /// The keys of a relationship end, as stored: new values when the column is being written
    /// (text deduplicated and trimmed as the engine stores it), else decoded from the backup.
    fn end(&mut self, table: &str, column: &str, store: &Store<'_>) -> Result<End, Error> {
        if let Some(values) = self.values_of(table, column) {
            let c = self
                .model_table(table)
                .and_then(|t| find(&t.columns, column, |x| &x.name))
                .ok_or_else(|| structural(format!("column '{table}'[{column}] not found")))?;
            let e = self.encode(table, c, values, store)?;
            return Ok(End {
                entries: e.entries,
                rows: self.text_storage(table, c).canonicalize(values),
            });
        }
        let s = store.column(table, column).map_err(structural)?;
        let fail = |e: bicraft_vertipaq::Error| structural(format!("'{table}'[{column}]: {e}"));
        let rows = ids_to_values(&s.ids, s.dictionary.as_ref(), s.encoding).map_err(fail)?;
        let entries = match (&s.dictionary, s.encoding.kind) {
            (Some(d), EncodingKind::Hash) => {
                let n = u32::try_from(d.len()).map_err(|_| structural("dictionary too large"))?;
                let mut ids: Vec<u32> = (0..n).filter_map(|i| i.checked_add(3)).collect();
                if s.ids.contains(&2) {
                    ids.insert(0, 2);
                }
                let values = ids_to_values(&ids, s.dictionary.as_ref(), s.encoding).map_err(fail)?;
                ids.into_iter().zip(values).collect()
            }
            _ => distinct_entries(&s.ids, s.dictionary.as_ref(), s.encoding).map_err(fail)?,
        };
        Ok(End { entries, rows })
    }

    /// The encoded data of one stage.
    fn encode_stage(
        &mut self,
        ch: &Changes,
        stage: &SemanticModel,
        store: &Store<'_>,
    ) -> Result<md::EncodedData, Error> {
        let mut out = md::EncodedData::default();
        for name in &ch.added_tables {
            let Some(t) = find(&stage.tables, name, |x| &x.name) else {
                continue;
            };
            let Some(v) = table_values(self.data, name) else {
                return Err(structural(format!("table '{name}': no data was supplied")));
            };
            let rows = v.columns.first().map_or(0, |c| c.values.len());
            let counts = self.partition_counts(t, rows)?;
            let rn = encode_row_number(rows as u64, &EncodeOptions::default())
                .map_err(|e| structural(format!("table '{name}': {e}")))?;
            let mut segment_map = md::SegmentMap::partition(
                to_i64(rows as u64),
                to_i64(rn.stats.segment_count()),
                to_i64(rn.stats.records_per_segment),
            );
            let mut row_number = row_number_data(&rn, rows);
            // Several partitions: row numbers continue across them (engine-observed).
            let mut parts: Vec<md::EncodedPartition> = Vec::with_capacity(counts.len());
            let (mut first, mut segments, mut rle) = (0u64, 0u64, 0u64);
            for &n in &counts {
                let p = bicraft_vertipaq::encode::encode_row_number_from(first, n, &EncodeOptions::default())
                    .map_err(|e| structural(format!("table '{name}': {e}")))?;
                first = first.saturating_add(n);
                segments = segments.saturating_add(p.stats.segment_count());
                rle = rle.saturating_add(p.stats.rle_runs());
                parts.push(md::EncodedPartition {
                    segment_map: md::SegmentMap::partition(
                        to_i64(n),
                        to_i64(p.stats.segment_count()),
                        to_i64(p.stats.records_per_segment),
                    ),
                    row_number: md::PartitionSegments {
                        idf: p.idf,
                        idfmeta: p.idfmeta,
                        segment_count: to_i64(p.stats.segment_count()),
                        rows: to_i64(n),
                    },
                    columns: Vec::new(),
                });
            }
            if !parts.is_empty() {
                segment_map = md::SegmentMap::partition(
                    segment_map.record_count,
                    to_i64(segments),
                    segment_map.records_per_segment,
                );
                row_number.segment_count = to_i64(segments);
                row_number.statistics.rle_runs = to_i64(rle);
            }
            let mut columns = Vec::new();
            for c in t.columns.iter().filter(|c| !is_row_number(c)) {
                let values = find(&v.columns, &c.name, |x| &x.name)
                    .map(|x| x.values.as_slice())
                    .ok_or_else(|| {
                        structural(format!("column '{name}'[{}]: no data was supplied", c.name))
                    })?;
                let e = self.encode(&t.name, c, values, store)?;
                if parts.is_empty() {
                    columns.push((c.name.clone(), e.storage));
                    continue;
                }
                let (data, segs) = self.split_partitions(&t.name, c, values, &counts, &e)?;
                for (p, seg) in parts.iter_mut().zip(segs) {
                    p.columns.push((c.name.clone(), seg));
                }
                columns.push((c.name.clone(), data));
            }
            out.tables.push(md::EncodedTable {
                name: t.name.clone(),
                segment_map,
                row_number,
                columns,
                partitions: parts,
            });
        }
        for (table, column) in &ch.added_columns {
            let Some(c) =
                find(&stage.tables, table, |x| &x.name).and_then(|t| find(&t.columns, column, |x| &x.name))
            else {
                continue;
            };
            let values = self
                .values_of(table, column)
                .ok_or_else(|| structural(format!("column '{table}'[{column}]: no data was supplied")))?;
            if let Some(stored) = store.table_rows(table)
                && stored != values.len() as u64
            {
                return Err(structural(format!(
                    "column '{table}'[{column}]: {} values, the table stores {stored} rows",
                    values.len()
                )));
            }
            let e = self.encode(table, c, values, store)?;
            let counts = store.partition_rows(table).unwrap_or_default();
            let (data, partitions) = if counts.len() > 1 {
                self.split_partitions(table, c, values, &counts, &e)?
            } else {
                (e.storage, Vec::new())
            };
            out.columns.push(md::EncodedNewColumn {
                table: table.clone(),
                column: column.clone(),
                data,
                partitions,
            });
        }
        for name in &ch.added_relationships {
            let Some(r) = stage.relationships.iter().find(|r| &r.name == name) else {
                continue;
            };
            if let Some(e) = self.relationship(r, store)? {
                out.relationships.push(e);
            }
        }
        for (table, name) in &ch.added_hierarchies {
            let Some(t) = find(&stage.tables, table, |x| &x.name) else {
                continue;
            };
            let Some(def) = find(&md::user_hierarchies(t), name, |h| &h.name).cloned() else {
                continue;
            };
            out.hierarchies.push(self.hierarchy(t, &def, store)?);
        }
        Ok(out)
    }

    /// The `U$` node table of a new user hierarchy: each level's data ids (new values, else the
    /// stored column) and attribute-hierarchy order (the encoder's, else the stored `H$`).
    fn hierarchy(
        &mut self,
        t: &Table,
        def: &md::UserHierarchyDef,
        store: &Store<'_>,
    ) -> Result<md::EncodedHierarchy, Error> {
        let object = format!("hierarchy '{}'[{}]", t.name, def.name);
        let fail = |e: &dyn std::fmt::Display| structural(format!("{object}: {e}"));
        let mut levels = Vec::with_capacity(def.levels.len());
        for (level, column) in &def.levels {
            let c = find(&t.columns, column, |x| &x.name)
                .ok_or_else(|| fail(&format!("level `{level}`: no column `{column}`")))?;
            let (ids, members) = match self.values_of(&t.name, &c.name) {
                Some(values) => {
                    let e = self.encode(&t.name, c, values, store)?;
                    let members = e.members.ok_or_else(|| {
                        fail(&format!(
                            "level `{level}`: column `{column}` has no attribute hierarchy"
                        ))
                    })?;
                    (e.ids, members)
                }
                None => {
                    let s = store.column(&t.name, &c.name).map_err(structural)?;
                    (
                        s.ids,
                        store.hierarchy_members(&t.name, &c.name).map_err(structural)?,
                    )
                }
            };
            levels.push((
                ids,
                AttributeHierarchy::from_members(members).map_err(|e| fail(&e))?,
            ));
        }
        let refs: Vec<UserHierarchyLevel<'_>> = levels
            .iter()
            .map(|(ids, h)| UserHierarchyLevel {
                data_ids: ids,
                hierarchy: h,
            })
            .collect();
        let u = build_user_hierarchy(&refs).map_err(|e| fail(&e))?;
        Ok(md::EncodedHierarchy {
            table: t.name.clone(),
            name: def.name.clone(),
            nodes: system_table(&u.table).map_err(|e| fail(&e))?,
            level_starts: u.level_starts,
        })
    }

    /// The stored index(es) of a new relationship: `None` for many-to-many (the engine stores
    /// none), one index for many-to-one, two for one-to-one (`docs/formats/metadata-sqlitedb.md`,
    /// "Rows written for a new relationship").
    fn relationship(
        &mut self,
        r: &Relationship,
        store: &Store<'_>,
    ) -> Result<Option<md::EncodedRelationship>, Error> {
        let object = format!(
            "relationship '{}'[{}] → '{}'[{}]",
            r.from_table, r.from_column, r.to_table, r.to_column
        );
        let from_card = r.from_cardinality.as_deref().unwrap_or("many");
        let to_card = r.to_cardinality.as_deref().unwrap_or("one");
        let one_to_one = match (
            from_card.to_ascii_lowercase().as_str(),
            to_card.to_ascii_lowercase().as_str(),
        ) {
            ("many", "one") => false,
            ("one", "one") => true,
            ("many", "many") => {
                // Resolved at query time: both columns must exist, blanks and duplicates are fine.
                for (t, c) in [(&r.from_table, &r.from_column), (&r.to_table, &r.to_column)] {
                    let found = self
                        .model_table(t)
                        .is_some_and(|x| find(&x.columns, c, |y| &y.name).is_some());
                    if !found {
                        return Err(structural(format!("{object}: column '{t}'[{c}] not found")));
                    }
                }
                return Ok(None);
            }
            _ => {
                return Err(structural(format!(
                    "{object}: {from_card}-to-{to_card} relationships cannot be written (use many-to-one, \
                     one-to-one or many-to-many)"
                )));
            }
        };
        let from = self.end(&r.from_table, &r.from_column, store)?;
        let to = self.end(&r.to_table, &r.to_column, store)?;
        let unique = |end: &End, t: &str, c: &str, side: &str| {
            check_unique(&end.rows, &self.keys)
                .map_err(|e| structural(format!("{object}: the {side} side '{t}'[{c}] {e}")))
        };
        unique(&to, &r.to_table, &r.to_column, "\"one\"")?;
        if one_to_one {
            unique(&from, &r.from_table, &r.from_column, "from (one-to-one)")?;
        }
        let index = |a: &End, b: &End, keys: &TextStorage| -> Result<(md::SystemTableData, bool), Error> {
            let index = build_relationship_index(&a.entries, &b.rows, keys)
                .map_err(|e| structural(format!("{object}: {e}")))?;
            let table = system_table(&index.table).map_err(|e| structural(format!("{object}: {e}")))?;
            // A blank from value has no match either: Desktop then shows the blank row too.
            let unmatched =
                index.unmatched > 0 || a.entries.iter().any(|(_, v)| matches!(v, CellValue::Null));
            Ok((table, unmatched))
        };
        let (forward, unmatched) = index(&from, &to, &self.keys)?;
        let mut out = md::EncodedRelationship::new(r.name.clone(), forward, unmatched);
        if one_to_one {
            out.reverse = Some(index(&to, &from, &self.keys)?);
        }
        Ok(Some(out))
    }
}

fn to_i64(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// Distinct data ids of `ids` (first-seen order) with their values.
fn distinct_entries(
    ids: &[u32],
    dictionary: Option<&Dictionary>,
    encoding: ColumnEncoding,
) -> Result<Vec<(u32, CellValue)>, bicraft_vertipaq::Error> {
    let mut seen = HashSet::new();
    let distinct: Vec<u32> = ids.iter().copied().filter(|id| seen.insert(*id)).collect();
    let values = ids_to_values(&distinct, dictionary, encoding)?;
    Ok(distinct.into_iter().zip(values).collect())
}

/// Equal as stored: numbers by value (an integer stored in a double column reads back as a
/// double), text exactly.
fn same_cell(a: &CellValue, b: &CellValue) -> bool {
    match (a, b) {
        (CellValue::Null, CellValue::Null) => true,
        (CellValue::Text(x), CellValue::Text(y)) => x == y,
        (CellValue::Real(x), CellValue::Real(y)) if x.is_nan() && y.is_nan() => true,
        _ => match (key_of(a, &TextStorage::exact()), key_of(b, &TextStorage::exact())) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        },
    }
}

/// Hashable key of a relationship value, text compared under `keys`.
#[derive(Hash, PartialEq, Eq)]
enum KeyOf {
    Num(u64),
    Text(bicraft_vertipaq::system::EquivalenceKey),
    Bool(bool),
}

fn key_of(v: &CellValue, keys: &TextStorage) -> Option<KeyOf> {
    let num = |x: f64| KeyOf::Num(if x == 0.0 { 0 } else { x.to_bits() });
    Some(match v {
        CellValue::Null => return None,
        CellValue::Int(i) => num(*i as f64),
        CellValue::Real(x) | CellValue::DateTime(x) => num(*x),
        CellValue::Decimal(d) => num(*d as f64 / 10_000.0),
        CellValue::Bool(b) => KeyOf::Bool(*b),
        CellValue::Text(s) => KeyOf::Text(keys.equivalence_key(s)),
    })
}

/// The "one" side of a relationship (stored values): no blank, no duplicate under `keys`.
fn check_unique(rows: &[CellValue], keys: &TextStorage) -> Result<(), String> {
    let mut seen = HashSet::with_capacity(rows.len());
    for (i, v) in rows.iter().enumerate() {
        let Some(k) = key_of(v, keys) else {
            return Err(format!("has a blank value (row {})", i.saturating_add(1)));
        };
        if !seen.insert(k) {
            return Err(format!(
                "has duplicate values (row {}: {}); the \"one\" side must be unique",
                i.saturating_add(1),
                value_text(v)
            ));
        }
    }
    Ok(())
}

fn dictionary_data(enc: &bicraft_vertipaq::encode::EncodedColumn) -> md::DictionaryData {
    let s = &enc.stats;
    let file_len = enc.dictionary.as_ref().map_or(0, |d| d.len() as u64);
    md::DictionaryData {
        file: enc.dictionary.clone(),
        kind: match s.kind {
            EncodingKind::Hash => 1,
            _ => 2,
        },
        data_type: match s.storage_type {
            StorageType::Long => 6,
            StorageType::Real => 8,
            StorageType::String => 2,
            StorageType::None => 19,
        },
        base_id: s.base_id,
        magnitude: s.magnitude,
        last_id: i64::from(s.last_id),
        is_nullable: s.is_nullable,
        is_unique: s.is_unique,
        is_operating_on_32: s.is_operating_on_32,
        flags: s.dictionary_flags,
        size: to_i64(s.dictionary_size.unwrap_or(file_len)),
    }
}

/// `ColumnStorage` statistics; the RLE run counts are sums over all segments, as the engine
/// stores them.
fn statistics(s: &bicraft_vertipaq::encode::ColumnStats) -> md::ColumnStatistics {
    md::ColumnStatistics {
        distinct_states: to_i64(s.distinct_states),
        min_data_id: i64::from(s.min_data_id),
        max_data_id: i64::from(s.max_data_id),
        // a signed 32-bit value in the metadata (0x8000_0000: −2^31, a column of blanks)
        original_min_segment_data_id: i64::from(i32::from_le_bytes(
            s.original_min_segment_data_id.to_le_bytes(),
        )),
        rle_sort_order: -1,
        row_count: to_i64(s.rows),
        has_nulls: s.has_nulls,
        rle_runs: to_i64(s.rle_runs()),
        others_rle_runs: to_i64(s.others_rle_runs()),
        db_type: s.db_type,
        xm_type: s.xm_type,
        compression_type: 0,
        compression_param: 0,
    }
}

fn row_number_data(rn: &bicraft_vertipaq::encode::EncodedColumn, rows: usize) -> md::ColumnStorageData {
    md::ColumnStorageData {
        idf: rn.idf.clone(),
        idfmeta: rn.idfmeta.clone(),
        segment_count: to_i64(rn.stats.segment_count()),
        dictionary: dictionary_data(rn),
        statistics: statistics(&rn.stats),
        hierarchy: Some(md::AttributeHierarchyData {
            // A table without rows: nothing materialized (engine, `datamodel-fresh.md`).
            materialization_type: if rows == 0 { 2 } else { 3 },
            distinct_data_count: to_i64(rows as u64),
            min_value: (rows > 0).then(|| "0".to_string()),
            max_value: rows.checked_sub(1).map(|m| m.to_string()),
            string_value_max_length: 0,
            hidx: None,
            system_table: None,
        }),
    }
}

fn system_table(
    t: &bicraft_vertipaq::system::SystemTableData,
) -> Result<md::SystemTableData, bicraft_vertipaq::system::SystemTableError> {
    let mut columns = Vec::with_capacity(t.columns.len());
    for (i, c) in t.columns.iter().enumerate() {
        let e = t.encode(i)?;
        columns.push(md::SystemColumnData {
            name: c.name.to_string(),
            idf: e.idf,
            idfmeta: e.idfmeta,
            segment_count: to_i64(e.meta.segments.len() as u64),
            settings: Some(c.storage_settings),
        });
    }
    Ok(md::SystemTableData {
        segment_map: md::SegmentMap {
            kind: t.kind.segment_map_type(),
            record_count: to_i64(t.record_count()),
            segment_count: to_i64(t.segment_count()),
            records_per_segment: to_i64(t.records_per_segment),
        },
        columns,
    })
}

/// `AttributeHierarchyStorage.MinValue`/`MaxValue` text (docs/formats/vertipaq-system-tables.md
/// §2): integers plain, decimals as the decimal value, doubles in round-trip form, dates
/// `M/D/YYYY`, booleans `TRUE`/`FALSE`, strings verbatim.
pub(crate) fn value_text(v: &CellValue) -> String {
    match v {
        CellValue::Null => String::new(),
        CellValue::Int(i) => i.to_string(),
        CellValue::Real(x) => real_text(*x),
        CellValue::Decimal(d) => decimal_text(*d),
        CellValue::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        CellValue::Text(s) => s.clone(),
        CellValue::DateTime(x) => date_text(*x),
    }
}

fn real_text(x: f64) -> String {
    if !x.is_finite() {
        return x.to_string();
    }
    let a = x.abs();
    if a != 0.0 && !(1e-5..1e15).contains(&a) {
        // .NET round-trip form: 1E+300, 1.5E-07.
        let s = format!("{x:E}");
        return match s.split_once('E') {
            Some((m, e)) => {
                let (sign, digits) = match e.strip_prefix('-') {
                    Some(d) => ('-', d),
                    None => ('+', e),
                };
                let digits = if digits.len() < 2 {
                    format!("0{digits}")
                } else {
                    digits.to_string()
                };
                format!("{m}E{sign}{digits}")
            }
            None => s,
        };
    }
    format!("{x}")
}

fn decimal_text(d: i64) -> String {
    let neg = d < 0;
    let a = d.unsigned_abs();
    let (int, frac) = (a / 10_000, a % 10_000);
    let mut s = int.to_string();
    if frac != 0 {
        let f = format!("{frac:04}");
        s.push('.');
        s.push_str(f.trim_end_matches('0'));
    }
    if neg { format!("-{s}") } else { s }
}

/// OLE Automation date → `M/D/YYYY` (with `h:mm:ss AM` when it has a time).
fn date_text(x: f64) -> String {
    if !x.is_finite() || x.abs() > 3.0e6 {
        return x.to_string();
    }
    let days = x.floor() as i64;
    let secs = ((x - x.floor()) * 86_400.0).round() as i64;
    let (y, m, d) = bicraft_locale::calendar::civil_from_serial(days);
    let date = format!("{m}/{d}/{y}");
    if secs == 0 || secs >= 86_400 {
        return date;
    }
    let (h, mi, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    let (h12, ampm) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!("{date} {h12}:{mi:02}:{s:02} {ampm}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hierarchy_value_text() {
        assert_eq!(value_text(&CellValue::Int(-3)), "-3");
        assert_eq!(value_text(&CellValue::Decimal(-45_000)), "-4.5");
        assert_eq!(value_text(&CellValue::Decimal(120_000)), "12");
        assert_eq!(value_text(&CellValue::Real(1e300)), "1E+300");
        assert_eq!(value_text(&CellValue::Real(1.5)), "1.5");
        assert_eq!(value_text(&CellValue::Real(1.5e-7)), "1.5E-07");
        assert_eq!(value_text(&CellValue::Bool(true)), "TRUE");
        assert_eq!(value_text(&CellValue::DateTime(45_658.0)), "1/1/2025");
        assert_eq!(value_text(&CellValue::DateTime(45_658.5)), "1/1/2025 12:00:00 PM");
        assert_eq!(value_text(&CellValue::DateTime(2.0)), "1/1/1900");
    }

    #[test]
    fn one_side_must_be_unique() {
        let c = TextStorage::for_culture("en-US", false);
        let t = |s: &str| CellValue::Text(s.into());
        assert!(check_unique(&[t("a"), t("b")], &c).is_ok());
        assert!(check_unique(&[t("a"), t("A")], &c).is_err());
        assert!(check_unique(&[CellValue::Int(1), CellValue::Null], &c).is_err());
        assert!(check_unique(&[CellValue::Int(1), CellValue::Real(1.0)], &c).is_err());
        // en-US keeps non-ASCII case variants apart; fr-FR merges them.
        assert!(check_unique(&[t("À"), t("à"), t("Ω"), t("ω")], &c).is_ok());
        let fr = TextStorage::for_culture("fr-FR", false);
        assert!(check_unique(&[t("À"), t("à")], &fr).is_err());
        assert!(check_unique(&[t("a"), t("à")], &fr).is_ok());
    }

    #[test]
    fn rle_statistics_sum_all_segments() {
        // 3 segments of 4 rows; every segment is one run of a repeated value.
        let values: Vec<CellValue> = (0..12).map(|i| CellValue::Int(i / 4)).collect();
        let options = EncodeOptions {
            segment_rows: 4,
            ..EncodeOptions::default()
        };
        let enc = encode_column(&values, LogicalType::Int64, &options).unwrap();
        let st = &enc.stats;
        assert_eq!(st.segment_count(), 3);
        let first = st.segments.first().unwrap();
        let s = statistics(st);
        let sum =
            |f: fn(&bicraft_vertipaq::encode::SegmentStatsOut) -> u64| st.segments.iter().map(f).sum::<u64>();
        assert_eq!(s.rle_runs, sum(|g| g.rle_runs) as i64);
        assert_eq!(s.others_rle_runs, sum(|g| g.others_rle_runs) as i64);
        assert!(s.rle_runs + s.others_rle_runs > (first.rle_runs + first.others_rle_runs) as i64);
    }

    fn model(tables: Vec<Table>) -> SemanticModel {
        SemanticModel {
            tables,
            ..Default::default()
        }
    }

    fn table(name: &str, tag: &str, cols: &[(&str, &str)]) -> Table {
        Table {
            name: name.into(),
            lineage_tag: Some(tag.into()),
            columns: cols
                .iter()
                .map(|(n, t)| Column {
                    name: (*n).into(),
                    lineage_tag: Some((*t).into()),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn diff_finds_renames_by_lineage_tag() {
        let base = model(vec![
            table("A", "ta", &[("x", "cx"), ("y", "cy")]),
            table("B", "tb", &[]),
        ]);
        let mut edited = model(vec![
            table("A2", "ta", &[("x2", "cx"), ("z", "cz")]),
            table("C", "tc", &[]),
        ]);
        let ch = diff(&base, &edited);
        assert_eq!(ch.renamed_tables, vec![("A".to_string(), "A2".to_string())]);
        assert_eq!(
            ch.renamed_columns,
            vec![("A2".to_string(), "x".to_string(), "x2".to_string())]
        );
        assert_eq!(ch.added_columns, vec![("A2".to_string(), "z".to_string())]);
        assert_eq!(ch.removed_columns, vec![("A2".to_string(), "y".to_string())]);
        assert_eq!(ch.added_tables, vec!["C".to_string()]);
        assert_eq!(ch.removed_tables, vec!["B".to_string()]);
        // Same name, other partition: replaced.
        edited = base.clone();
        if let Some(t) = edited.tables.get_mut(1) {
            t.partitions.push(bicraft_model::Partition::default());
        }
        let ch = diff(&base, &edited);
        assert_eq!(ch.replaced_tables, vec!["B".to_string()]);
        assert!(diff(&base, &base).is_empty());
    }
}
