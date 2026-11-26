//! `SemanticModel` → edits on existing TMSCHEMA rows: the inverse of `build`, limited to
//! metadata-only changes. See `docs/formats/metadata-sqlitedb.md`, section "Writing".
//!
//! The edited model is diffed against `build(base)`. Every difference is either applied to a copy
//! of the base rows or recorded as an [`UnsupportedChange`]; if any were recorded the whole edit
//! is refused. Row positions found while indexing the base stay valid until [`Editor::finish`],
//! which bumps modification times, drops deleted rows and appends new ones.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use bicraft_model::{Annotation, Column, Measure, Properties, Relationship, Role, SemanticModel, Table};
use serde_json::Value;

use crate::codes::{self, measure_data_type as mdt, object_type as ot};
use crate::input::{MetaRow, MetaValue, MetadataTables, Row, column_key};
use crate::nested::{Lookups, WOwner, Writer, handles};
use crate::spec::{self, Child};
use crate::{Error, UnsupportedChange};

/// Seconds between 1601-01-01 (FILETIME epoch) and 1970-01-01 (Unix epoch).
const FILETIME_UNIX_OFFSET_SECS: u64 = 11_644_473_600;

/// The current time as Windows FILETIME ticks (100 ns since 1601-01-01 UTC).
pub(crate) fn filetime_now() -> Result<i64, Error> {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::Invalid("system clock is before 1970".into()))?;
    since
        .as_secs()
        .checked_add(FILETIME_UNIX_OFFSET_SECS)
        .and_then(|s| s.checked_mul(10_000_000))
        .and_then(|t| t.checked_add(u64::from(since.subsec_nanos() / 100)))
        .and_then(|t| i64::try_from(t).ok())
        .ok_or_else(|| Error::Invalid("system clock out of range".into()))
}

pub(crate) fn apply(base: &MetadataTables, model: &SemanticModel, now: i64) -> Result<MetadataTables, Error> {
    apply_fresh(base, model, now, &[])
}

/// [`apply`] after a structural edit that wrote the stored data of the tables `fresh` (added
/// in this edit): their nested objects whose rows are stored data (a calculation group and its
/// items, the rows of the table) may be added too.
pub(crate) fn apply_fresh(
    base: &MetadataTables,
    model: &SemanticModel,
    now: i64,
    fresh: &[String],
) -> Result<MetadataTables, Error> {
    let model = &*crate::normalize_spellings(model);
    let orig = crate::build::build(base)?;
    if orig == *model {
        return Ok(base.clone());
    }
    let ix = Index::new(base, &orig)?;
    let mut d = Differ {
        base,
        orig: &orig,
        ix: &ix,
        ed: Editor::new(base, now)?,
        look: Lookups::new(base),
        unsupported: Vec::new(),
        fresh,
        stored_ok: false,
    };
    d.run(model)?;
    if !d.unsupported.is_empty() {
        return Err(Error::UnsupportedChange(d.unsupported));
    }
    d.ed.finish()
}

// ---------------------------------------------------------------------------------------------
// Index: base row positions aligned with the collections of `build(base)`.

/// A row of the base: its position in its table's row list and its `ID`.
#[derive(Debug, Clone, Copy)]
struct At {
    pos: usize,
    id: Option<i64>,
}

struct TableAt {
    at: At,
    columns: Vec<At>,
    measures: Vec<At>,
}

struct RoleAt {
    at: At,
    permissions: Vec<At>,
}

struct Index {
    model: At,
    tables: Vec<TableAt>,
    roles: Vec<RoleAt>,
    expressions: Vec<At>,
    /// (ObjectType, ObjectID) → `Annotation` rows that `build` maps, in ID order.
    annotations: HashMap<(i64, i64), Vec<usize>>,
}

/// Positions of the rows of `table`, sorted by `ID` exactly as `build` sorts them.
fn sorted<'a>(t: &'a MetadataTables, table: &str) -> Vec<(usize, Row<'a>)> {
    let mut rows: Vec<(usize, Row<'_>)> = t.rows(table).iter().map(Row).enumerate().collect();
    rows.sort_by_key(|(_, r)| r.id().unwrap_or(i64::MAX));
    rows
}

fn at(pos: usize, r: Row<'_>) -> At {
    At { pos, id: r.id() }
}

impl Index {
    fn new(t: &MetadataTables, orig: &SemanticModel) -> Result<Self, Error> {
        let (model_pos, model_row) = sorted(t, "Model").into_iter().next().ok_or(Error::NoModel)?;
        let model_id = model_row.id();
        let in_model = |r: &Row<'_>| match (model_id, r.int("ModelID")) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        };
        let children = |table: &str, fk: &str, parent: Option<i64>| -> Vec<At> {
            match parent {
                Some(p) => sorted(t, table)
                    .into_iter()
                    .filter(|(_, r)| r.int(fk) == Some(p))
                    .map(|(pos, r)| at(pos, r))
                    .collect(),
                None => Vec::new(),
            }
        };

        let mut tables = Vec::new();
        let mut user_tables = HashSet::new();
        for (pos, r) in sorted(t, "Table") {
            if in_model(&r) && r.int("SystemFlags").unwrap_or(0) & 1 == 0 {
                let a = at(pos, r);
                if let Some(id) = a.id {
                    user_tables.insert(id);
                }
                tables.push(TableAt {
                    at: a,
                    columns: children("Column", "TableID", a.id),
                    measures: children("Measure", "TableID", a.id),
                });
            }
        }

        let mut roles = Vec::new();
        for (pos, r) in sorted(t, "Role") {
            if in_model(&r) {
                let a = at(pos, r);
                let permissions = match a.id {
                    Some(id) => sorted(t, "TablePermission")
                        .into_iter()
                        .filter(|(_, p)| {
                            p.int("RoleID") == Some(id)
                                && p.int("TableID").is_some_and(|x| user_tables.contains(&x))
                        })
                        .map(|(pos, p)| at(pos, p))
                        .collect(),
                    None => Vec::new(),
                };
                roles.push(RoleAt { at: a, permissions });
            }
        }

        let expressions = sorted(t, "Expression")
            .into_iter()
            .filter(|(_, r)| in_model(r))
            .map(|(pos, r)| at(pos, r))
            .collect::<Vec<_>>();

        let mut annotations: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (pos, r) in sorted(t, "Annotation") {
            if let (Some(k), Some(o), Some(_)) = (r.int("ObjectType"), r.int("ObjectID"), r.text("Name")) {
                annotations.entry((k, o)).or_default().push(pos);
            }
        }

        // The index must line up with what `build` produced from the same rows.
        let aligned = tables.len() == orig.tables.len()
            && tables
                .iter()
                .zip(&orig.tables)
                .all(|(a, b)| a.columns.len() == b.columns.len() && a.measures.len() == b.measures.len())
            && roles.len() == orig.roles.len()
            && roles
                .iter()
                .zip(&orig.roles)
                .all(|(a, b)| a.permissions.len() == b.table_permissions.len())
            && expressions.len() == orig.expressions.len();
        if !aligned {
            return Err(Error::Invalid(
                "metadata rows do not line up with the model read from them".into(),
            ));
        }
        Ok(Self {
            model: At {
                pos: model_pos,
                id: model_id,
            },
            tables,
            roles,
            expressions,
            annotations,
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Editor: row-level edits on a copy of the base.

/// A cell of a new row. `Engine` cells are bookkeeping values the writer picks (dropped when the
/// table has no such column); `User` cells carry model content (kept, so a schema without the
/// column is reported by the SQLite adapter instead of losing the value).
pub(crate) enum Cell {
    User(MetaValue),
    Engine(MetaValue),
}

pub(crate) struct Editor<'b> {
    base: &'b MetadataTables,
    out: MetadataTables,
    pub(crate) now: i64,
    /// Highest ID in use (rows and `DBPROPERTIES.MAXID`).
    max_id: i64,
    allocated: bool,
    /// Changed rows: (table, position) → also bump `StructureModifiedTime`.
    touched: BTreeMap<(String, usize), bool>,
    deleted: BTreeSet<(String, usize)>,
    added: Vec<(String, MetaRow)>,
}

impl<'b> Editor<'b> {
    fn new(base: &'b MetadataTables, now: i64) -> Result<Self, Error> {
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
        if let Some(m) = maxid_row(base.rows("DBPROPERTIES")).and_then(|r| Row(r).int("VALUE")) {
            max_id = max_id.max(m);
        }
        Ok(Self {
            base,
            out: base.clone(),
            now,
            max_id,
            allocated: false,
            touched: BTreeMap::new(),
            deleted: BTreeSet::new(),
            added: Vec::new(),
        })
    }

    pub(crate) fn alloc_id(&mut self) -> Result<i64, Error> {
        let id = self
            .max_id
            .checked_add(1)
            .ok_or_else(|| Error::Invalid("object IDs exhausted".into()))?;
        self.max_id = id;
        self.allocated = true;
        Ok(id)
    }

    /// Sets `col` of the base row `table[pos]` (no-op when equal) and marks the row changed.
    pub(crate) fn set(&mut self, table: &str, pos: usize, col: &str, v: MetaValue) -> Result<(), Error> {
        let row = self
            .out
            .rows_mut(table)
            .and_then(|rows| rows.get_mut(pos))
            .ok_or_else(|| Error::Invalid(format!("row {pos} of `{table}` is missing")))?;
        let key = column_key(row, col).unwrap_or(col).to_string();
        if row.get(&key) == Some(&v) {
            return Ok(());
        }
        row.insert(key, v);
        self.touched.entry((table.to_string(), pos)).or_insert(false);
        Ok(())
    }

    /// Sets `col` only when the base row already has that column.
    pub(crate) fn set_if_present(
        &mut self,
        table: &str,
        pos: usize,
        col: &str,
        v: MetaValue,
    ) -> Result<(), Error> {
        let present = self
            .base
            .rows(table)
            .get(pos)
            .is_some_and(|r| column_key(r, col).is_some());
        if present {
            self.set(table, pos, col, v)
        } else {
            Ok(())
        }
    }

    fn structural(&mut self, table: &str, pos: usize) {
        self.touched.insert((table.to_string(), pos), true);
    }

    pub(crate) fn delete(&mut self, table: &str, pos: usize) {
        self.deleted.insert((table.to_string(), pos));
    }

    /// Whether this edit removes the base row `table[pos]`.
    pub(crate) fn is_deleted(&self, table: &str, pos: usize) -> bool {
        self.deleted.contains(&(table.to_string(), pos))
    }

    /// Positions of base rows of `table` matching `pred`.
    pub(crate) fn find(&self, table: &str, pred: impl Fn(Row<'_>) -> bool) -> Vec<usize> {
        self.base
            .rows(table)
            .iter()
            .enumerate()
            .filter(|(_, r)| pred(Row(r)))
            .map(|(p, _)| p)
            .collect()
    }

    /// Deletes the rows that hang off object (`kind`, `id`) through `ObjectType`/`ObjectID`.
    pub(crate) fn delete_dependents(&mut self, kind: i64, id: i64) {
        for table in [
            "Annotation",
            "ExtendedProperty",
            "ChangedProperty",
            "ObjectTranslation",
            "ExcludedArtifact",
        ] {
            for pos in self.find(table, |r| {
                r.int("ObjectType") == Some(kind) && r.int("ObjectID") == Some(id)
            }) {
                self.delete(table, pos);
            }
        }
    }

    /// Queues a new row. When the table already has rows, the row takes exactly their columns:
    /// cells for columns the table lacks are dropped (`Engine` cells, and `User` cells holding
    /// NULL), and columns the template does not know get the value every existing row shares if
    /// it is an integer, else NULL.
    pub(crate) fn add(&mut self, table: &str, cells: Vec<(&str, Cell)>) {
        let existing = self.base.rows(table);
        let mut row = MetaRow::new();
        if existing.is_empty() {
            for (k, c) in cells {
                let (Cell::User(v) | Cell::Engine(v)) = c;
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
            let mut cells: Vec<(&str, Option<Cell>)> = cells.into_iter().map(|(k, c)| (k, Some(c))).collect();
            for col in &columns {
                let v = match cells.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(col)) {
                    Some((_, c)) => match c.take() {
                        Some(Cell::User(v) | Cell::Engine(v)) => v,
                        None => MetaValue::Null,
                    },
                    None => shared_integer(existing, col),
                };
                row.insert((*col).to_string(), v);
            }
            for (k, c) in cells {
                if let Some(Cell::User(v)) = c
                    && v != MetaValue::Null
                {
                    row.insert(k.to_string(), v);
                }
            }
        }
        self.added.push((table.to_string(), row));
    }

    fn finish(mut self) -> Result<MetadataTables, Error> {
        let now = MetaValue::Integer(self.now);
        let touched = std::mem::take(&mut self.touched);
        for ((table, pos), structural) in touched {
            if self.deleted.contains(&(table.clone(), pos)) {
                continue;
            }
            self.set_if_present(&table, pos, "ModifiedTime", now.clone())?;
            if structural {
                self.set_if_present(&table, pos, "StructureModifiedTime", now.clone())?;
            }
        }
        let mut by_table: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
        for (table, pos) in &self.deleted {
            by_table.entry(table.as_str()).or_default().insert(*pos);
        }
        for (table, positions) in by_table {
            if let Some(rows) = self.out.rows_mut(table) {
                let mut i = 0usize;
                rows.retain(|_| {
                    let keep = !positions.contains(&i);
                    i = i.saturating_add(1);
                    keep
                });
            }
        }
        for (table, row) in std::mem::take(&mut self.added) {
            let key = self.out.key(&table).unwrap_or(&table).to_string();
            self.out.push_row(&key, row);
        }
        if self.allocated
            && let Some(rows) = self.out.rows_mut("DBPROPERTIES")
            && let Some(row) = rows.iter_mut().find(|r| is_maxid(r))
        {
            let key = column_key(row, "VALUE").unwrap_or("VALUE").to_string();
            row.insert(key, MetaValue::Integer(self.max_id));
        }
        Ok(self.out)
    }
}

fn is_maxid(r: &MetaRow) -> bool {
    Row(r)
        .text("NAME")
        .is_some_and(|n| n.eq_ignore_ascii_case("MAXID"))
}

fn maxid_row(rows: &[MetaRow]) -> Option<&MetaRow> {
    rows.iter().find(|r| is_maxid(r))
}

/// The integer every row holds in `col`, or NULL.
fn shared_integer(rows: &[MetaRow], col: &str) -> MetaValue {
    let mut values = rows.iter().map(|r| Row(r).get(col));
    match values.next().flatten() {
        Some(v @ MetaValue::Integer(_)) if values.all(|x| x == Some(v)) => v.clone(),
        _ => MetaValue::Null,
    }
}

// ---------------------------------------------------------------------------------------------
// Differ: model differences → edits or unsupported changes.

struct Differ<'a> {
    base: &'a MetadataTables,
    orig: &'a SemanticModel,
    ix: &'a Index,
    ed: Editor<'a>,
    /// Lookups for the objects kept as JSON in `extra` (see `nested`).
    look: Lookups<'a>,
    unsupported: Vec<UnsupportedChange>,
    /// Tables whose stored data this edit wrote ([`apply_fresh`]).
    fresh: &'a [String],
    /// The owner being written is one of `fresh`.
    stored_ok: bool,
}

fn text(v: Option<&str>) -> MetaValue {
    v.map_or(MetaValue::Null, MetaValue::from)
}

fn flag(v: bool) -> MetaValue {
    MetaValue::Integer(i64::from(v))
}

fn find_name<T>(items: &[T], name: &str, get: impl Fn(&T) -> &str) -> Option<usize> {
    items.iter().position(|x| get(x).eq_ignore_ascii_case(name))
}

/// Keys whose values differ between two `extra` maps, in first-seen order.
fn extra_diff<'k>(a: &'k Properties, b: &'k Properties) -> Vec<&'k str> {
    let mut keys: Vec<&str> = Vec::new();
    for k in a.keys().chain(b.keys()) {
        if a.get(k) != b.get(k) && !keys.contains(&k.as_str()) {
            keys.push(k);
        }
    }
    keys
}

/// The measure `DataType` of a literal expression; `None` when the type needs DAX inference.
fn literal_type(expr: &str) -> Option<i64> {
    let e = expr.trim();
    if e.parse::<i64>().is_ok() {
        return Some(mdt::INT64);
    }
    if e.contains('.') && e.parse::<f64>().is_ok_and(f64::is_finite) {
        return Some(mdt::DOUBLE);
    }
    if let Some(inner) = e.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
        && !inner.replace("\"\"", "").contains('"')
    {
        return Some(mdt::STRING);
    }
    let upper = e.to_ascii_uppercase();
    if ["TRUE", "FALSE", "TRUE()", "FALSE()"].contains(&upper.as_str()) {
        return Some(mdt::BOOLEAN);
    }
    None
}

/// A random-looking, deterministic RFC 4122 version-4 GUID (lower case, like Desktop's lineage
/// tags) derived from `seed`.
pub(crate) fn guid(seed: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in seed {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let a = splitmix(h);
    let b = splitmix(a);
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        a >> 32,
        (a >> 16) & 0xffff,
        a & 0x0fff,
        ((b >> 48) & 0x3fff) | 0x8000,
        b & 0xffff_ffff_ffff
    )
}

fn splitmix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl<'a> Differ<'a> {
    fn unsupported(&mut self, object: impl Into<String>, what: impl Into<String>) {
        self.unsupported.push(UnsupportedChange {
            what: what.into(),
            object: object.into(),
        });
    }

    fn writer(&mut self) -> Writer<'_, 'a> {
        Writer {
            ed: &mut self.ed,
            look: &self.look,
            unsupported: &mut self.unsupported,
            stored_ok: self.stored_ok,
        }
    }

    /// Refuses the `extra` differences that neither `children` (nor `allowed`) cover; returns
    /// whether one that `children` covers differs.
    fn extra_split(
        &mut self,
        children: &[Child],
        old: &Properties,
        new: &Properties,
        object: &str,
        allowed: &[&str],
    ) -> bool {
        let mut nested = false;
        for k in extra_diff(old, new) {
            if handles(children, k) {
                nested = true;
            } else if !allowed.contains(&k) {
                self.unsupported(object, format!("property `{k}` changed"));
            }
        }
        nested
    }

    /// A typed base object as the owner of nested collections.
    fn owner(
        &self,
        table: &'static str,
        at: At,
        kind: i64,
        scope: Option<i64>,
        object: &str,
    ) -> Option<WOwner<'a>> {
        let base: &'a MetadataTables = self.base;
        let row = base.rows(table).get(at.pos).map(|r| (at.pos, Row(r)));
        Some(WOwner {
            table,
            row,
            id: at.id?,
            object_type: Some(kind),
            scope,
            object: object.to_string(),
        })
    }

    /// Writes the nested differences of a typed base object.
    #[allow(clippy::too_many_arguments)]
    fn update_nested(
        &mut self,
        children: &[Child],
        table: &'static str,
        at: At,
        kind: i64,
        scope: Option<i64>,
        (old, new): (&Properties, &Properties),
        annotations: bool,
        object: &str,
    ) -> Result<(), Error> {
        let Some(owner) = self.owner(table, at, kind, scope, object) else {
            self.unsupported(object, "nested objects changed on an object without an ID");
            return Ok(());
        };
        self.writer().update(children, &owner, old, new, annotations)
    }

    fn run(&mut self, model: &SemanticModel) -> Result<(), Error> {
        let orig = self.orig;
        let ix = self.ix;
        // Renamed measures keep their row; nested objects resolve them by the new name.
        for (k, name) in renamed_measures(orig, model) {
            if let Some(id) = measure_at(orig, ix, k).and_then(|a| a.id) {
                self.look.rename_measure(id, &name);
            }
        }
        // Renamed relationships keep their row; variations resolve them by the new name.
        for (old, new) in model.relationship_renames(orig) {
            self.look.rename_relationship(&old, &new);
        }
        // Model object. `name`, `compatibility_level` and `database_extra` are not stored here.
        if model.culture != orig.culture {
            self.unsupported("model", "culture changed");
        }
        // New measures first: perspectives and translations may name them (a measure of a table
        // given new data comes back as a new row).
        let added = self.new_measures(model)?;
        // Model scalars `model.set` changes (M9.1): plain columns of the Model row.
        let scalars = ["discourageImplicitMeasures", "sourceQueryCulture"];
        for key in scalars {
            let (old, new) = (orig.extra.get(key), model.extra.get(key));
            if old == new {
                continue;
            }
            let value = match (key, new) {
                ("discourageImplicitMeasures", v) => Some(flag(v.and_then(Value::as_bool).unwrap_or(false))),
                (_, None) => Some(MetaValue::Null),
                (_, Some(Value::String(s))) => Some(MetaValue::Text(s.clone())),
                _ => None,
            };
            match value {
                Some(v) => {
                    let column = if key == "sourceQueryCulture" {
                        "SourceQueryCulture"
                    } else {
                        "DiscourageImplicitMeasures"
                    };
                    self.ed.set("Model", ix.model.pos, column, v)?;
                }
                None => self.unsupported("model", format!("property `{key}` changed")),
            }
        }
        if self.extra_split(spec::MODEL_CHILDREN, &orig.extra, &model.extra, "model", &scalars) {
            self.update_nested(
                spec::MODEL_CHILDREN,
                "Model",
                ix.model,
                ot::MODEL,
                None,
                (&orig.extra, &model.extra),
                false,
                "model",
            )?;
        }
        if model.description != orig.description {
            self.ed.set(
                "Model",
                ix.model.pos,
                "Description",
                text(model.description.as_deref()),
            )?;
        }
        self.annotations(
            ot::MODEL,
            ix.model.id,
            &orig.annotations,
            &model.annotations,
            "model",
        )?;

        self.tables(model)?;
        for (at, object) in self.measures(model, &added)? {
            self.remove_measure(at, &object)?;
        }
        self.relationships(model)?;
        self.expressions(model)?;
        self.roles(model)?;
        Ok(())
    }

    /// Annotations of one object: matched by name (n-th occurrence to n-th occurrence).
    fn annotations(
        &mut self,
        kind: i64,
        id: Option<i64>,
        old: &[Annotation],
        new: &[Annotation],
        object: &str,
    ) -> Result<(), Error> {
        if old == new {
            return Ok(());
        }
        let Some(id) = id else {
            self.unsupported(object, "annotations changed on an object without an ID");
            return Ok(());
        };
        let positions = self.ix.annotations.get(&(kind, id)).cloned().unwrap_or_default();
        if positions.len() != old.len() {
            return Err(Error::Invalid(format!(
                "annotation rows of {object} do not line up"
            )));
        }
        let mut used = vec![false; old.len()];
        for a in new {
            if a.name.is_empty() {
                self.unsupported(object, "annotation with an empty name");
                continue;
            }
            let matched = old
                .iter()
                .enumerate()
                .position(|(i, o)| o.name == a.name && !used.get(i).copied().unwrap_or(true));
            match matched {
                Some(i) => {
                    if let Some(u) = used.get_mut(i) {
                        *u = true;
                    }
                    if old.get(i).is_some_and(|o| o.value != a.value)
                        && let Some(&pos) = positions.get(i)
                    {
                        self.ed
                            .set("Annotation", pos, "Value", MetaValue::from(a.value.as_str()))?;
                    }
                }
                None => self.new_annotation(kind, id, a)?,
            }
        }
        for (i, u) in used.iter().enumerate() {
            if !u && let Some(&pos) = positions.get(i) {
                self.ed.delete("Annotation", pos);
            }
        }
        Ok(())
    }

    fn new_annotation(&mut self, kind: i64, object_id: i64, a: &Annotation) -> Result<(), Error> {
        let id = self.ed.alloc_id()?;
        let now = self.ed.now;
        self.ed.add(
            "Annotation",
            vec![
                ("ID", Cell::User(id.into())),
                ("ObjectID", Cell::User(object_id.into())),
                ("ObjectType", Cell::User(kind.into())),
                ("Name", Cell::User(a.name.as_str().into())),
                ("Value", Cell::User(a.value.as_str().into())),
                ("ModifiedTime", Cell::Engine(now.into())),
            ],
        );
        Ok(())
    }

    fn new_annotations(
        &mut self,
        kind: i64,
        object_id: i64,
        list: &[Annotation],
        object: &str,
    ) -> Result<(), Error> {
        for a in list {
            if a.name.is_empty() {
                self.unsupported(object, "annotation with an empty name");
            } else {
                self.new_annotation(kind, object_id, a)?;
            }
        }
        Ok(())
    }

    // --- tables and columns ------------------------------------------------------------------

    fn tables(&mut self, model: &SemanticModel) -> Result<(), Error> {
        let orig = self.orig;
        let ix = self.ix;
        let mut matched = vec![false; orig.tables.len()];
        for t in &model.tables {
            let object = format!("table '{}'", t.name);
            let Some(i) = find_name(&orig.tables, &t.name, |x| &x.name) else {
                self.unsupported(object, "table added");
                continue;
            };
            if matched.get(i).copied().unwrap_or(true) {
                self.unsupported(object, "duplicate table name");
                continue;
            }
            if let Some(m) = matched.get_mut(i) {
                *m = true;
            }
            let (Some(o), Some(at)) = (orig.tables.get(i), ix.tables.get(i)) else {
                continue;
            };
            self.table(o, t, at, &object)?;
        }
        for (i, m) in matched.iter().enumerate() {
            if !m && let Some(o) = orig.tables.get(i) {
                self.unsupported(format!("table '{}'", o.name), "table removed");
            }
        }
        Ok(())
    }

    fn table(&mut self, o: &Table, t: &Table, at: &TableAt, object: &str) -> Result<(), Error> {
        if o.name != t.name {
            self.unsupported(object, "table renamed");
        }
        if o.lineage_tag != t.lineage_tag {
            self.unsupported(object, "lineageTag changed");
        }
        if o.partitions != t.partitions {
            self.unsupported(object, "partitions changed");
        }
        if self.extra_split(
            spec::TABLE_CHILDREN,
            &o.extra,
            &t.extra,
            object,
            &["dataCategory"],
        ) {
            self.stored_ok = self.fresh.iter().any(|f| f.eq_ignore_ascii_case(&t.name));
            let written = self.update_nested(
                spec::TABLE_CHILDREN,
                "Table",
                at.at,
                ot::TABLE,
                at.at.id,
                (&o.extra, &t.extra),
                false,
                object,
            );
            self.stored_ok = false;
            written?;
        }
        let pos = at.at.pos;
        if o.description != t.description {
            self.ed
                .set("Table", pos, "Description", text(t.description.as_deref()))?;
        }
        if o.is_hidden != t.is_hidden {
            self.ed.set("Table", pos, "IsHidden", flag(t.is_hidden))?;
        }
        // The data category (`Time`: a date table) is metadata.
        let category = |x: &Table| {
            x.extra
                .get("dataCategory")
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        if category(o) != category(t) {
            if t.extra.get("dataCategory").is_some_and(|v| !v.is_string()) {
                self.unsupported(object, "dataCategory is not text");
            } else {
                self.ed
                    .set("Table", pos, "DataCategory", text(category(t).as_deref()))?;
            }
        }
        self.annotations(ot::TABLE, at.at.id, &o.annotations, &t.annotations, object)?;

        let mut matched = vec![false; o.columns.len()];
        for c in &t.columns {
            let cobj = format!("column '{}'[{}]", o.name, c.name);
            let Some(i) = find_name(&o.columns, &c.name, |x| &x.name) else {
                self.unsupported(cobj, "column added");
                continue;
            };
            if matched.get(i).copied().unwrap_or(true) {
                self.unsupported(cobj, "duplicate column name");
                continue;
            }
            if let Some(m) = matched.get_mut(i) {
                *m = true;
            }
            let (Some(oc), Some(cat)) = (o.columns.get(i), at.columns.get(i)) else {
                continue;
            };
            self.column(o, at, oc, c, *cat, &cobj)?;
        }
        // The row-number column is the key while no user column is: the engine refuses a table
        // with two key columns (checked on the engine, 2026-10-14).
        let user_key = |x: &Table| {
            x.columns
                .iter()
                .any(|c| !c.is_row_number() && c.extra.get("isKey").and_then(Value::as_bool) == Some(true))
        };
        if user_key(o) != user_key(t)
            && let Some(i) = o.columns.iter().position(Column::is_row_number)
            && let Some(cat) = at.columns.get(i)
        {
            self.ed.set("Column", cat.pos, "IsKey", flag(!user_key(t)))?;
        }
        for (i, m) in matched.iter().enumerate() {
            if !m && let Some(oc) = o.columns.get(i) {
                self.unsupported(format!("column '{}'[{}]", o.name, oc.name), "column removed");
            }
        }
        Ok(())
    }

    fn column(
        &mut self,
        table: &Table,
        tat: &TableAt,
        o: &Column,
        c: &Column,
        at: At,
        object: &str,
    ) -> Result<(), Error> {
        if o.name != c.name {
            self.unsupported(object, "column renamed");
        }
        if o.data_type != c.data_type {
            self.unsupported(object, "dataType changed");
        }
        if o.expression != c.expression {
            self.unsupported(object, "expression changed");
        }
        // A calculated-table column's source column and name inference follow renames
        // (the engine rewrites them with its formula fix-up); they do not touch stored data.
        let table_column = c.extra.get("type").and_then(Value::as_str) == Some("calculatedTableColumn");
        if o.source_column != c.source_column && !table_column {
            self.unsupported(object, "sourceColumn changed");
        }
        if o.lineage_tag != c.lineage_tag {
            self.unsupported(object, "lineageTag changed");
        }
        let allowed: &[&str] = if table_column {
            &["isNameInferred", "isKey"]
        } else {
            &["isKey"]
        };
        if self.extra_split(spec::COLUMN_CHILDREN, &o.extra, &c.extra, object, allowed) {
            self.update_nested(
                spec::COLUMN_CHILDREN,
                "Column",
                at,
                ot::COLUMN,
                tat.at.id,
                (&o.extra, &c.extra),
                false,
                object,
            )?;
        }
        let pos = at.pos;
        let set = |d: &mut Self, col: &str, v: MetaValue| d.ed.set("Column", pos, col, v);
        if table_column && o.source_column != c.source_column {
            set(self, "SourceColumn", text(c.source_column.as_deref()))?;
        }
        let inferred = |x: &Column| x.extra.get("isNameInferred").and_then(Value::as_bool) == Some(true);
        if table_column && inferred(o) != inferred(c) {
            if inferred(c) {
                set(self, "ExplicitName", MetaValue::Null)?;
                set(self, "InferredName", MetaValue::Text(c.name.clone()))?;
            } else {
                set(self, "ExplicitName", MetaValue::Text(c.name.clone()))?;
            }
        }
        if o.format_string != c.format_string {
            set(self, "FormatString", text(c.format_string.as_deref()))?;
        }
        if o.display_folder != c.display_folder {
            set(self, "DisplayFolder", text(c.display_folder.as_deref()))?;
        }
        if o.data_category != c.data_category {
            set(self, "DataCategory", text(c.data_category.as_deref()))?;
        }
        // `isKey`: a date table's date column (metadata only).
        let key = |x: &Column| x.extra.get("isKey").and_then(Value::as_bool) == Some(true);
        if key(o) != key(c) {
            set(self, "IsKey", flag(key(c)))?;
        }
        if o.description != c.description {
            set(self, "Description", text(c.description.as_deref()))?;
        }
        if o.is_hidden != c.is_hidden {
            set(self, "IsHidden", flag(c.is_hidden))?;
        }
        if o.summarize_by != c.summarize_by {
            let code = match c.summarize_by.as_deref() {
                None => Some(1),
                Some(s) => codes::code_of(codes::summarize_by, s),
            };
            match code {
                Some(code) => set(self, "SummarizeBy", code.into())?,
                None => self.unsupported(object, "unknown summarizeBy value"),
            }
        }
        if o.sort_by_column != c.sort_by_column {
            let target = match c.sort_by_column.as_deref() {
                None => Some(0),
                Some(name) => find_name(&table.columns, name, |x| &x.name)
                    .and_then(|i| tat.columns.get(i))
                    .and_then(|a| a.id),
            };
            match target {
                Some(id) => {
                    set(self, "SortByColumnID", id.into())?;
                    self.ed.structural("Column", pos);
                }
                None => self.unsupported(object, "sortByColumn names no column of the table"),
            }
        }
        self.annotations(ot::COLUMN, at.id, &o.annotations, &c.annotations, object)
    }

    // --- measures ----------------------------------------------------------------------------

    /// Inserts the measures of existing tables that the base has under no name (renamed ones
    /// excluded); returns their lower-cased names.
    fn new_measures(&mut self, model: &SemanticModel) -> Result<HashSet<String>, Error> {
        let orig = self.orig;
        let ix = self.ix;
        let renamed = renamed_measures(orig, model);
        let mut added = HashSet::new();
        for t in &model.tables {
            let Some(target) = find_name(&orig.tables, &t.name, |x| &x.name) else {
                continue;
            };
            for m in &t.measures {
                let key = m.name.to_ascii_lowercase();
                let known = m.name.trim().is_empty()
                    || added.contains(&key)
                    || orig
                        .tables
                        .iter()
                        .any(|o| o.measures.iter().any(|x| x.name.eq_ignore_ascii_case(&m.name)))
                    || renamed.iter().any(|(_, n)| n.eq_ignore_ascii_case(&m.name));
                if known {
                    continue;
                }
                let object = format!("measure '{}'[{}]", t.name, m.name);
                let table_id = ix.tables.get(target).and_then(|t| t.at.id);
                self.new_measure(t, m, table_id, &object)?;
                added.insert(key);
            }
        }
        Ok(added)
    }

    /// Measures changed (those of `added` were inserted by [`Self::new_measures`]); returns the
    /// ones to remove (row, object name).
    fn measures(
        &mut self,
        model: &SemanticModel,
        added: &HashSet<String>,
    ) -> Result<Vec<(At, String)>, Error> {
        let orig = self.orig;
        let ix = self.ix;
        // Every base measure: (table index, measure index).
        let all: Vec<(usize, usize)> = orig
            .tables
            .iter()
            .enumerate()
            .flat_map(|(ti, t)| (0..t.measures.len()).map(move |mi| (ti, mi)))
            .collect();
        let measure = |(ti, mi): (usize, usize)| orig.tables.get(ti).and_then(|t| t.measures.get(mi));
        let mut matched = vec![false; all.len()];
        let mut names: HashSet<String> = HashSet::new();
        let renamed = renamed_measures(orig, model);
        for t in &model.tables {
            // Measures of added tables are covered by "table added".
            let Some(target) = find_name(&orig.tables, &t.name, |x| &x.name) else {
                continue;
            };
            for m in &t.measures {
                let object = format!("measure '{}'[{}]", t.name, m.name);
                if m.name.trim().is_empty() {
                    self.unsupported(object, "measure without a name");
                    continue;
                }
                if !names.insert(m.name.to_ascii_lowercase()) {
                    self.unsupported(object, "duplicate measure name");
                    continue;
                }
                let found = all
                    .iter()
                    .position(|&k| measure(k).is_some_and(|o| o.name.eq_ignore_ascii_case(&m.name)))
                    .or_else(|| {
                        renamed
                            .iter()
                            .find(|(_, n)| n.eq_ignore_ascii_case(&m.name))
                            .and_then(|(at, _)| all.iter().position(|k| k == at))
                    });
                match found {
                    Some(k) => {
                        if let Some(x) = matched.get_mut(k) {
                            *x = true;
                        }
                        let Some(&(ti, mi)) = all.get(k) else { continue };
                        let (Some(o), Some(at)) = (
                            measure((ti, mi)),
                            ix.tables.get(ti).and_then(|t| t.measures.get(mi)),
                        ) else {
                            continue;
                        };
                        let to_table = (ti != target).then(|| ix.tables.get(target).and_then(|t| t.at.id));
                        self.measure(o, m, *at, to_table, &object)?;
                    }
                    None if added.contains(&m.name.to_ascii_lowercase()) => {}
                    None => {
                        let table_id = ix.tables.get(target).and_then(|t| t.at.id);
                        self.new_measure(t, m, table_id, &object)?;
                    }
                }
            }
        }
        let mut removed = Vec::new();
        for (k, was) in matched.iter().enumerate() {
            if *was {
                continue;
            }
            let Some(&(ti, mi)) = all.get(k) else { continue };
            let (Some(t), Some(o), Some(at)) = (
                orig.tables.get(ti),
                measure((ti, mi)),
                ix.tables.get(ti).and_then(|t| t.measures.get(mi)),
            ) else {
                continue;
            };
            removed.push((*at, format!("measure '{}'[{}]", t.name, o.name)));
        }
        Ok(removed)
    }

    fn measure_extra_ok(&mut self, old: &Properties, new: &Properties, object: &str) -> bool {
        let mut ok = true;
        for k in extra_diff(old, new) {
            let valid = match k {
                "dataCategory" | "sourceLineageTag" => new.get(k).is_none_or(Value::is_string),
                "isSimpleMeasure" => new.get(k).is_none_or(Value::is_boolean),
                _ => handles(spec::MEASURE_CHILDREN, k),
            };
            if !valid {
                self.unsupported(object, format!("property `{k}` changed"));
                ok = false;
            }
        }
        ok
    }

    fn measure(
        &mut self,
        o: &Measure,
        m: &Measure,
        at: At,
        to_table: Option<Option<i64>>,
        object: &str,
    ) -> Result<(), Error> {
        let pos = at.pos;
        let extra_ok = self.measure_extra_ok(&o.extra, &m.extra, object);
        let set = |d: &mut Self, col: &str, v: MetaValue| d.ed.set("Measure", pos, col, v);
        if let Some(table_id) = to_table {
            let Some(table_id) = table_id else {
                self.unsupported(object, "moved to a table without an ID");
                return Ok(());
            };
            set(self, "TableID", table_id.into())?;
            self.ed.structural("Measure", pos);
        }
        if o.name != m.name {
            set(self, "Name", m.name.as_str().into())?;
            self.ed.structural("Measure", pos);
        }
        if o.expression != m.expression {
            set(self, "Expression", m.expression.as_str().into())?;
            if let Some(code) = literal_type(&m.expression) {
                self.ed.set_if_present("Measure", pos, "DataType", code.into())?;
            }
            self.ed.set_if_present("Measure", pos, "State", 1.into())?;
            self.ed
                .set_if_present("Measure", pos, "ErrorMessage", MetaValue::Null)?;
            self.ed.structural("Measure", pos);
        }
        if o.format_string != m.format_string {
            set(self, "FormatString", text(m.format_string.as_deref()))?;
        }
        if o.display_folder != m.display_folder {
            set(self, "DisplayFolder", text(m.display_folder.as_deref()))?;
        }
        if o.description != m.description {
            set(self, "Description", text(m.description.as_deref()))?;
        }
        if o.lineage_tag != m.lineage_tag {
            set(self, "LineageTag", text(m.lineage_tag.as_deref()))?;
        }
        if o.is_hidden != m.is_hidden {
            set(self, "IsHidden", flag(m.is_hidden))?;
        }
        if extra_ok {
            for (key, col) in [
                ("dataCategory", "DataCategory"),
                ("sourceLineageTag", "SourceLineageTag"),
            ] {
                if o.extra.get(key) != m.extra.get(key) {
                    set(self, col, text(m.extra.get(key).and_then(Value::as_str)))?;
                }
            }
            if o.extra.get("isSimpleMeasure") != m.extra.get("isSimpleMeasure") {
                let v = m
                    .extra
                    .get("isSimpleMeasure")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                set(self, "IsSimpleMeasure", flag(v))?;
            }
            if extra_diff(&o.extra, &m.extra)
                .into_iter()
                .any(|k| handles(spec::MEASURE_CHILDREN, k))
            {
                let base: &'a MetadataTables = self.base;
                let scope = match to_table {
                    Some(t) => t,
                    None => base.rows("Measure").get(pos).and_then(|r| Row(r).int("TableID")),
                };
                self.update_nested(
                    spec::MEASURE_CHILDREN,
                    "Measure",
                    at,
                    ot::MEASURE,
                    scope,
                    (&o.extra, &m.extra),
                    false,
                    object,
                )?;
            }
        }
        self.annotations(ot::MEASURE, at.id, &o.annotations, &m.annotations, object)
    }

    fn new_measure(
        &mut self,
        t: &Table,
        m: &Measure,
        table_id: Option<i64>,
        object: &str,
    ) -> Result<(), Error> {
        let empty = Properties::new();
        if !self.measure_extra_ok(&empty, &m.extra, object) {
            return Ok(());
        }
        let Some(table_id) = table_id else {
            self.unsupported(object, "added to a table without an ID");
            return Ok(());
        };
        let id = self.ed.alloc_id()?;
        let now = self.ed.now;
        let lineage = match &m.lineage_tag {
            Some(tag) => Cell::User(tag.as_str().into()),
            None => {
                let mut seed = Vec::new();
                seed.extend_from_slice(&now.to_le_bytes());
                seed.extend_from_slice(&id.to_le_bytes());
                seed.extend_from_slice(t.name.as_bytes());
                seed.push(0);
                seed.extend_from_slice(m.name.as_bytes());
                Cell::Engine(guid(&seed).into())
            }
        };
        let extra_text = |key: &str| text(m.extra.get(key).and_then(Value::as_str));
        let simple = m
            .extra
            .get("isSimpleMeasure")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let data_type = literal_type(&m.expression).unwrap_or(mdt::VARIANT);
        let owner = WOwner {
            table: "Measure",
            row: None,
            id,
            object_type: Some(ot::MEASURE),
            scope: Some(table_id),
            object: object.to_string(),
        };
        let pointers = self
            .writer()
            .insert_children(spec::MEASURE_CHILDREN, &owner, &m.extra, false)?;
        let pointer = |col: &str| {
            let id = pointers.iter().find(|(c, _)| *c == col).map_or(0, |(_, id)| *id);
            if id == 0 {
                Cell::Engine(0.into())
            } else {
                Cell::User(id.into())
            }
        };
        self.ed.add(
            "Measure",
            vec![
                ("ID", Cell::User(id.into())),
                ("TableID", Cell::User(table_id.into())),
                ("Name", Cell::User(m.name.as_str().into())),
                ("Description", Cell::User(text(m.description.as_deref()))),
                ("DataType", Cell::Engine(data_type.into())),
                ("Expression", Cell::User(m.expression.as_str().into())),
                ("FormatString", Cell::User(text(m.format_string.as_deref()))),
                ("IsHidden", Cell::User(flag(m.is_hidden))),
                ("State", Cell::Engine(1.into())),
                ("ModifiedTime", Cell::Engine(now.into())),
                ("StructureModifiedTime", Cell::Engine(now.into())),
                ("KPIID", pointer("KPIID")),
                (
                    "IsSimpleMeasure",
                    if simple {
                        Cell::User(flag(true))
                    } else {
                        Cell::Engine(flag(false))
                    },
                ),
                ("ErrorMessage", Cell::Engine(MetaValue::Null)),
                ("DisplayFolder", Cell::User(text(m.display_folder.as_deref()))),
                ("DetailRowsDefinitionID", pointer("DetailRowsDefinitionID")),
                ("DataCategory", Cell::User(extra_text("dataCategory"))),
                ("FormatStringDefinitionID", pointer("FormatStringDefinitionID")),
                ("LineageTag", lineage),
                ("SourceLineageTag", Cell::User(extra_text("sourceLineageTag"))),
            ],
        );
        self.look.add_measure(id, table_id, &m.name);
        self.new_annotations(ot::MEASURE, id, &m.annotations, object)
    }

    fn remove_measure(&mut self, at: At, object: &str) -> Result<(), Error> {
        let Some(id) = at.id else {
            self.unsupported(object, "measure without an ID removed");
            return Ok(());
        };
        // Perspective entries the same edit removes do not count.
        let in_perspective = self
            .ed
            .find("PerspectiveMeasure", |r| r.int("MeasureID") == Some(id))
            .into_iter()
            .any(|pos| !self.ed.is_deleted("PerspectiveMeasure", pos));
        let is_default = !self
            .ed
            .find("Model", |r| r.reference("DefaultMeasureID") == Some(id))
            .is_empty();
        if in_perspective || is_default {
            self.unsupported(
                object,
                "measure removed while it is in a perspective or is the default measure",
            );
            return Ok(());
        }
        // Its KPI, detail rows and format string definitions go with it.
        if let Some(owner) = self.owner("Measure", at, ot::MEASURE, None, object) {
            self.writer().delete_children(spec::MEASURE_CHILDREN, &owner);
        }
        self.ed.delete("Measure", at.pos);
        self.ed.delete_dependents(ot::MEASURE, id);
        Ok(())
    }

    // --- relationships and expressions -------------------------------------------------------

    fn relationships(&mut self, model: &SemanticModel) -> Result<(), Error> {
        let orig = self.orig;
        if orig.relationships == model.relationships {
            return Ok(());
        }
        let mut matched = vec![false; orig.relationships.len()];
        let renames = model.relationship_renames(orig);
        for r in &model.relationships {
            let object = format!("relationship '{}'", r.name);
            let old_name = renames
                .iter()
                .find(|(_, new)| *new == r.name)
                .map_or(r.name.as_str(), |(old, _)| old.as_str());
            let found = orig
                .relationships
                .iter()
                .enumerate()
                .position(|(i, o)| o.name == old_name && !matched.get(i).copied().unwrap_or(true));
            match found {
                Some(i) => {
                    if let Some(x) = matched.get_mut(i) {
                        *x = true;
                    }
                    if let Some(o) = orig.relationships.get(i)
                        && o != r
                    {
                        self.relationship(o, r, &object)?;
                    }
                }
                None => self.unsupported(object, "relationship added"),
            }
        }
        for (i, m) in matched.iter().enumerate() {
            if !m && let Some(o) = orig.relationships.get(i) {
                self.unsupported(format!("relationship '{}'", o.name), "relationship removed");
            }
        }
        Ok(())
    }

    /// A kept relationship changed in place (`relationship.set`): name, activity, cross
    /// filtering, security filtering, referential integrity and date join are metadata; other
    /// changes (ends, cardinalities) need the structural writer.
    fn relationship(&mut self, o: &Relationship, r: &Relationship, object: &str) -> Result<(), Error> {
        let same_ends = o.from_table == r.from_table
            && o.from_column == r.from_column
            && o.to_table == r.to_table
            && o.to_column == r.to_column
            && o.from_cardinality == r.from_cardinality
            && o.to_cardinality == r.to_cardinality;
        if !same_ends {
            self.unsupported(object, "relationship ends or cardinality changed");
            return Ok(());
        }
        const SCALARS: [&str; 3] = [
            "securityFilteringBehavior",
            "relyOnReferentialIntegrity",
            "joinOnDateBehavior",
        ];
        for k in extra_diff(&o.extra, &r.extra) {
            if !SCALARS.contains(&k) {
                self.unsupported(object, format!("property `{k}` changed"));
            }
        }
        let positions = self
            .ed
            .find("Relationship", |row| row.text("Name") == Some(o.name.as_str()));
        let [pos] = positions.as_slice() else {
            self.unsupported(object, "relationship row not found by name");
            return Ok(());
        };
        let pos = *pos;
        let id = self
            .base
            .rows("Relationship")
            .get(pos)
            .and_then(|row| Row(row).id());
        if o.name != r.name {
            self.ed
                .set("Relationship", pos, "Name", text(Some(r.name.as_str())))?;
        }
        if o.is_active != r.is_active {
            self.ed.set("Relationship", pos, "IsActive", flag(r.is_active))?;
        }
        let code = |f: fn(i64) -> Option<&'static str>, v: Option<&str>, default: i64| match v {
            None => Some(default),
            Some(s) => codes::code_of(f, s),
        };
        if o.cross_filtering_behavior != r.cross_filtering_behavior {
            match code(codes::cross_filtering, r.cross_filtering_behavior.as_deref(), 1) {
                Some(c) => self
                    .ed
                    .set("Relationship", pos, "CrossFilteringBehavior", c.into())?,
                None => self.unsupported(object, "unknown crossFilteringBehavior value"),
            }
        }
        let text_of = |x: &Relationship, k: &str| x.extra.get(k).and_then(Value::as_str).map(str::to_string);
        for (key, col, f) in [
            (
                "securityFilteringBehavior",
                "SecurityFilteringBehavior",
                codes::security_filtering as fn(i64) -> Option<&'static str>,
            ),
            ("joinOnDateBehavior", "JoinOnDateBehavior", codes::join_on_date),
        ] {
            let (a, b) = (text_of(o, key), text_of(r, key));
            if a != b {
                match code(f, b.as_deref(), 1) {
                    Some(c) => self.ed.set("Relationship", pos, col, c.into())?,
                    None => self.unsupported(object, format!("unknown {key} value")),
                }
            }
        }
        let rely = |x: &Relationship| {
            x.extra.get("relyOnReferentialIntegrity").and_then(Value::as_bool) == Some(true)
        };
        if rely(o) != rely(r) {
            self.ed
                .set("Relationship", pos, "RelyOnReferentialIntegrity", flag(rely(r)))?;
        }
        self.annotations(ot::RELATIONSHIP, id, &o.annotations, &r.annotations, object)
    }

    fn expressions(&mut self, model: &SemanticModel) -> Result<(), Error> {
        let orig = self.orig;
        let ix = self.ix;
        let mut matched = vec![false; orig.expressions.len()];
        for e in &model.expressions {
            let object = format!("expression '{}'", e.name);
            let Some(i) = find_name(&orig.expressions, &e.name, |x| &x.name) else {
                self.unsupported(object, "expression added");
                continue;
            };
            if matched.get(i).copied().unwrap_or(true) {
                self.unsupported(object, "duplicate expression name");
                continue;
            }
            if let Some(x) = matched.get_mut(i) {
                *x = true;
            }
            let (Some(o), Some(at)) = (orig.expressions.get(i), ix.expressions.get(i)) else {
                continue;
            };
            if o.name != e.name {
                self.unsupported(&object, "expression renamed");
            }
            if o.kind != e.kind {
                self.unsupported(&object, "kind changed");
            }
            if o.lineage_tag != e.lineage_tag {
                self.unsupported(&object, "lineageTag changed");
            }
            if self.extra_split(spec::EXPRESSION_CHILDREN, &o.extra, &e.extra, &object, &[]) {
                self.update_nested(
                    spec::EXPRESSION_CHILDREN,
                    "Expression",
                    *at,
                    ot::EXPRESSION,
                    None,
                    (&o.extra, &e.extra),
                    false,
                    &object,
                )?;
            }
            if o.expression != e.expression {
                self.ed
                    .set("Expression", at.pos, "Expression", e.expression.as_str().into())?;
                self.ed.structural("Expression", at.pos);
            }
            if o.description != e.description {
                self.ed.set(
                    "Expression",
                    at.pos,
                    "Description",
                    text(e.description.as_deref()),
                )?;
            }
            self.annotations(ot::EXPRESSION, at.id, &o.annotations, &e.annotations, &object)?;
        }
        for (i, m) in matched.iter().enumerate() {
            if !m && let Some(o) = orig.expressions.get(i) {
                self.unsupported(format!("expression '{}'", o.name), "expression removed");
            }
        }
        Ok(())
    }

    // --- roles -------------------------------------------------------------------------------

    fn roles(&mut self, model: &SemanticModel) -> Result<(), Error> {
        let orig = self.orig;
        let ix = self.ix;
        let mut matched = vec![false; orig.roles.len()];
        for r in &model.roles {
            let object = format!("role '{}'", r.name);
            match find_name(&orig.roles, &r.name, |x| &x.name) {
                Some(i) if matched.get(i).copied().unwrap_or(true) => {
                    self.unsupported(object, "duplicate role name");
                }
                Some(i) => {
                    if let Some(x) = matched.get_mut(i) {
                        *x = true;
                    }
                    if let (Some(o), Some(at)) = (orig.roles.get(i), ix.roles.get(i)) {
                        self.role(o, r, at, &object)?;
                    }
                }
                None => self.new_role(r, &object)?,
            }
        }
        for (i, m) in matched.iter().enumerate() {
            if !m && let Some(at) = ix.roles.get(i) {
                self.remove_role(at.at);
            }
        }
        Ok(())
    }

    fn role_extra_ok(&mut self, old: &Properties, new: &Properties, object: &str) -> bool {
        let mut ok = true;
        for k in extra_diff(old, new) {
            if !((k == "description" && new.get(k).is_none_or(Value::is_string))
                || handles(spec::ROLE_CHILDREN, k))
            {
                self.unsupported(object, format!("property `{k}` changed"));
                ok = false;
            }
        }
        ok
    }

    fn permission_extra_ok(&mut self, old: &Properties, new: &Properties, object: &str) -> bool {
        let mut ok = true;
        for k in extra_diff(old, new) {
            let valid = (k == "metadataPermission"
                && new.get(k).is_none_or(|v| {
                    v.as_str()
                        .is_some_and(|s| codes::code_of(codes::metadata_permission, s).is_some())
                }))
                || k == "annotations"
                || handles(spec::TABLE_PERMISSION_CHILDREN, k);
            if !valid {
                self.unsupported(object, format!("property `{k}` changed"));
                ok = false;
            }
        }
        ok
    }

    fn model_permission(&mut self, v: Option<&str>, object: &str) -> Option<i64> {
        // A role without a permission is created with `read`, as Desktop does.
        let code = match v {
            None => Some(2),
            Some(s) => codes::code_of(codes::model_permission, s),
        };
        if code.is_none() {
            self.unsupported(object, "unknown modelPermission value");
        }
        code
    }

    fn role(&mut self, o: &Role, r: &Role, at: &RoleAt, object: &str) -> Result<(), Error> {
        let pos = at.at.pos;
        if o.name != r.name {
            self.ed.set("Role", pos, "Name", r.name.as_str().into())?;
        }
        if o.model_permission != r.model_permission
            && let Some(code) = self.model_permission(r.model_permission.as_deref(), object)
        {
            self.ed.set("Role", pos, "ModelPermission", code.into())?;
        }
        if self.role_extra_ok(&o.extra, &r.extra, object)
            && o.extra.get("description") != r.extra.get("description")
        {
            let d = r.extra.get("description").and_then(Value::as_str);
            self.ed.set("Role", pos, "Description", text(d))?;
        }
        if extra_diff(&o.extra, &r.extra)
            .into_iter()
            .any(|k| handles(spec::ROLE_CHILDREN, k))
        {
            self.update_nested(
                spec::ROLE_CHILDREN,
                "Role",
                at.at,
                ot::ROLE,
                None,
                (&o.extra, &r.extra),
                false,
                object,
            )?;
        }
        self.annotations(ot::ROLE, at.at.id, &o.annotations, &r.annotations, object)?;

        let mut matched = vec![false; o.table_permissions.len()];
        for p in &r.table_permissions {
            let pobj = format!("{object} permission on '{}'", p.table);
            match find_name(&o.table_permissions, &p.table, |x| &x.table) {
                Some(i) if matched.get(i).copied().unwrap_or(true) => {
                    self.unsupported(pobj, "duplicate table permission");
                }
                Some(i) => {
                    if let Some(x) = matched.get_mut(i) {
                        *x = true;
                    }
                    let (Some(op), Some(pat)) = (o.table_permissions.get(i), at.permissions.get(i)) else {
                        continue;
                    };
                    if !self.permission_extra_ok(&op.extra, &p.extra, &pobj) {
                        continue;
                    }
                    let ppos = pat.pos;
                    if op.filter_expression != p.filter_expression {
                        self.ed.set(
                            "TablePermission",
                            ppos,
                            "FilterExpression",
                            text(p.filter_expression.as_deref()),
                        )?;
                        self.ed
                            .set_if_present("TablePermission", ppos, "State", 1.into())?;
                        self.ed
                            .set_if_present("TablePermission", ppos, "ErrorMessage", MetaValue::Null)?;
                    }
                    if op.extra.get("metadataPermission") != p.extra.get("metadataPermission") {
                        let code = metadata_permission(&p.extra);
                        self.ed
                            .set("TablePermission", ppos, "MetadataPermission", code.into())?;
                    }
                    if extra_diff(&op.extra, &p.extra)
                        .into_iter()
                        .any(|k| k == "annotations" || handles(spec::TABLE_PERMISSION_CHILDREN, k))
                    {
                        let base: &'a MetadataTables = self.base;
                        let scope = base
                            .rows("TablePermission")
                            .get(ppos)
                            .and_then(|r| Row(r).int("TableID"));
                        self.update_nested(
                            spec::TABLE_PERMISSION_CHILDREN,
                            "TablePermission",
                            *pat,
                            ot::TABLE_PERMISSION,
                            scope,
                            (&op.extra, &p.extra),
                            true,
                            &pobj,
                        )?;
                    }
                }
                None => match at.at.id {
                    Some(role_id) => self.new_permission(role_id, p, &pobj)?,
                    None => self.unsupported(pobj, "permission added to a role without an ID"),
                },
            }
        }
        for (i, m) in matched.iter().enumerate() {
            if !m && let Some(pat) = at.permissions.get(i) {
                self.remove_permission(*pat);
            }
        }
        Ok(())
    }

    fn new_role(&mut self, r: &Role, object: &str) -> Result<(), Error> {
        if r.name.trim().is_empty() {
            self.unsupported(object, "role without a name");
            return Ok(());
        }
        let empty = Properties::new();
        let extra_ok = self.role_extra_ok(&empty, &r.extra, object);
        let permission = self.model_permission(r.model_permission.as_deref(), object);
        let (Some(permission), true) = (permission, extra_ok) else {
            return Ok(());
        };
        let Some(model_id) = self.ix.model.id else {
            self.unsupported(object, "role added to a model without an ID");
            return Ok(());
        };
        let id = self.ed.alloc_id()?;
        let now = self.ed.now;
        let description = text(r.extra.get("description").and_then(Value::as_str));
        self.ed.add(
            "Role",
            vec![
                ("ID", Cell::User(id.into())),
                ("ModelID", Cell::User(model_id.into())),
                ("Name", Cell::User(r.name.as_str().into())),
                ("Description", Cell::User(description)),
                ("ModelPermission", Cell::User(permission.into())),
                ("ModifiedTime", Cell::Engine(now.into())),
            ],
        );
        self.new_annotations(ot::ROLE, id, &r.annotations, object)?;
        let owner = WOwner {
            table: "Role",
            row: None,
            id,
            object_type: Some(ot::ROLE),
            scope: None,
            object: object.to_string(),
        };
        self.writer()
            .insert_children(spec::ROLE_CHILDREN, &owner, &r.extra, false)?;
        for p in &r.table_permissions {
            let pobj = format!("{object} permission on '{}'", p.table);
            self.new_permission(id, p, &pobj)?;
        }
        Ok(())
    }

    fn new_permission(
        &mut self,
        role_id: i64,
        p: &bicraft_model::TablePermission,
        object: &str,
    ) -> Result<(), Error> {
        let empty = Properties::new();
        if !self.permission_extra_ok(&empty, &p.extra, object) {
            return Ok(());
        }
        let table_id = find_name(&self.orig.tables, &p.table, |x| &x.name)
            .and_then(|i| self.ix.tables.get(i))
            .and_then(|t| t.at.id);
        let Some(table_id) = table_id else {
            self.unsupported(object, "permission on a table the model does not have");
            return Ok(());
        };
        let id = self.ed.alloc_id()?;
        let now = self.ed.now;
        self.ed.add(
            "TablePermission",
            vec![
                ("ID", Cell::User(id.into())),
                ("RoleID", Cell::User(role_id.into())),
                ("TableID", Cell::User(table_id.into())),
                (
                    "FilterExpression",
                    Cell::User(text(p.filter_expression.as_deref())),
                ),
                ("ModifiedTime", Cell::Engine(now.into())),
                ("State", Cell::Engine(1.into())),
                ("ErrorMessage", Cell::Engine(MetaValue::Null)),
                (
                    "MetadataPermission",
                    Cell::User(metadata_permission(&p.extra).into()),
                ),
            ],
        );
        let owner = WOwner {
            table: "TablePermission",
            row: None,
            id,
            object_type: Some(ot::TABLE_PERMISSION),
            scope: Some(table_id),
            object: object.to_string(),
        };
        self.writer()
            .insert_children(spec::TABLE_PERMISSION_CHILDREN, &owner, &p.extra, true)?;
        Ok(())
    }

    fn remove_permission(&mut self, at: At) {
        self.ed.delete("TablePermission", at.pos);
        if let Some(id) = at.id {
            self.ed.delete_dependents(ot::TABLE_PERMISSION, id);
        }
        if let Some(owner) = self.owner(
            "TablePermission",
            at,
            ot::TABLE_PERMISSION,
            None,
            "table permission",
        ) {
            self.writer()
                .delete_children(spec::TABLE_PERMISSION_CHILDREN, &owner);
        }
    }

    fn remove_role(&mut self, at: At) {
        self.ed.delete("Role", at.pos);
        let Some(id) = at.id else { return };
        self.ed.delete_dependents(ot::ROLE, id);
        if let Some(owner) = self.owner("Role", at, ot::ROLE, None, "role") {
            self.writer().delete_children(spec::ROLE_CHILDREN, &owner);
        }
        // Every permission of the role, including ones on tables the model does not show.
        let perms: Vec<At> = self
            .base
            .rows("TablePermission")
            .iter()
            .enumerate()
            .filter(|(_, r)| Row(r).int("RoleID") == Some(id))
            .map(|(pos, r)| at_row(pos, r))
            .collect();
        for p in perms {
            self.remove_permission(p);
        }
    }
}

/// Measures renamed by the edit: `((table index, measure index) in `orig`, new name)`. A
/// measure of `model` whose name is not in `orig` is the rename of the `orig` measure whose name
/// is gone from `model` and that has the same lineage tag or, without tags, is the only such
/// measure of the same table (facing the only new one).
fn renamed_measures(orig: &SemanticModel, model: &SemanticModel) -> Vec<((usize, usize), String)> {
    let has = |m: &SemanticModel, name: &str| m.measure(name).is_some();
    let mut out: Vec<((usize, usize), String)> = Vec::new();
    for m in model.tables.iter().flat_map(|t| t.measures.iter()) {
        let Some(tag) = m.lineage_tag.as_deref() else {
            continue;
        };
        if has(orig, &m.name) {
            continue;
        }
        let found = orig.tables.iter().enumerate().find_map(|(ti, t)| {
            t.measures
                .iter()
                .position(|o| o.lineage_tag.as_deref() == Some(tag) && !has(model, &o.name))
                .map(|mi| (ti, mi))
        });
        if let Some(at) = found
            && !out.iter().any(|(k, _)| *k == at)
        {
            out.push((at, m.name.clone()));
        }
    }
    for (ti, o) in orig.tables.iter().enumerate() {
        let Some(t) = model.tables.iter().find(|t| t.name.eq_ignore_ascii_case(&o.name)) else {
            continue;
        };
        let gone: Vec<usize> = (0..o.measures.len())
            .filter(|&mi| {
                o.measures
                    .get(mi)
                    .is_some_and(|x| !has(model, &x.name) && !out.iter().any(|(k, _)| *k == (ti, mi)))
            })
            .collect();
        let new: Vec<&Measure> = t
            .measures
            .iter()
            .filter(|x| !has(orig, &x.name) && !out.iter().any(|(_, n)| n.eq_ignore_ascii_case(&x.name)))
            .collect();
        if let ([mi], [x]) = (gone.as_slice(), new.as_slice()) {
            out.push(((ti, *mi), x.name.clone()));
        }
    }
    out
}

/// The base row of `orig`'s measure `(table index, measure index)`.
fn measure_at(orig: &SemanticModel, ix: &Index, (ti, mi): (usize, usize)) -> Option<At> {
    orig.tables.get(ti)?.measures.get(mi)?;
    ix.tables.get(ti)?.measures.get(mi).copied()
}

fn at_row(pos: usize, r: &MetaRow) -> At {
    at(pos, Row(r))
}

/// `metadataPermission` of a table permission's `extra` as a code; `0` (default) when absent.
fn metadata_permission(extra: &Properties) -> i64 {
    extra
        .get("metadataPermission")
        .and_then(Value::as_str)
        .and_then(|s| codes::code_of(codes::metadata_permission, s))
        .unwrap_or(0)
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn literal_types() {
        assert_eq!(literal_type(" 1 "), Some(mdt::INT64));
        assert_eq!(literal_type("-3"), Some(mdt::INT64));
        assert_eq!(literal_type("1.5"), Some(mdt::DOUBLE));
        assert_eq!(literal_type("\"a\"\"b\""), Some(mdt::STRING));
        assert_eq!(literal_type("\"a\" & \"b\""), None);
        assert_eq!(literal_type("true()"), Some(mdt::BOOLEAN));
        assert_eq!(literal_type("SUM(T[C])"), None);
        assert_eq!(literal_type("NaN"), None);
    }

    #[test]
    fn guids_look_like_desktop_lineage_tags() {
        let g = guid(b"seed");
        assert_eq!(g.len(), 36);
        assert_eq!(g, guid(b"seed"));
        assert_ne!(g, guid(b"seed2"));
        let parts: Vec<&str> = g.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(
            g.chars()
                .all(|c| c == '-' || c.is_ascii_digit() || ('a'..='f').contains(&c))
        );
        assert_eq!(parts.get(2).and_then(|p| p.chars().next()), Some('4'));
    }

    #[test]
    fn filetime_now_is_after_2020() {
        // 2020-01-01 in FILETIME ticks.
        assert!(filetime_now().is_ok_and(|t| t > 132_223_104_000_000_000));
    }
}
