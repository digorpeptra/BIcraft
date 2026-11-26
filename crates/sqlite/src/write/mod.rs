//! The writer: builds a complete SQLite database image in memory.
//!
//! [`DatabaseBuilder`] lays out a fresh database from `CREATE` statements and rows;
//! [`rewrite`] produces a copy of an existing database in which chosen tables get new rows.
//! Both are written from the public file-format description
//! (<https://www.sqlite.org/fileformat2.html>); see `docs/formats/sqlite.md` ("Writer").

mod pages;
mod record;

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use crate::database::Database;
use crate::header::{MAGIC, TextEncoding, be_u32};
use crate::record::{Row, Value};
use crate::schema::{
    Affinity, Generated, IndexParseError, KeyColumn, KeyConstraint, TableInfo, TableKind, parse_create_index,
    table_extras,
};
use crate::{Error, Result};

use pages::{Cell, Copier, Leaves, Pager, build_tree, index_cell, put_bytes, table_leaf_cell};
use record::{Collation, Field, encode_record, real_as_int, value_cmp};

/// `SQLITE_VERSION_NUMBER` written into a new database's header (the writer is not SQLite;
/// SQLite overwrites this the next time it modifies the file).
const NEW_DB_VERSION_NUMBER: u32 = 3_046_001;

/// The table SQLite keeps for `AUTOINCREMENT` counters.
const SEQUENCE_TABLE: &str = "sqlite_sequence";
const SEQUENCE_SQL: &str = "CREATE TABLE sqlite_sequence(name,seq)";

/// Header settings that are carried over from a source database.
#[derive(Debug, Clone)]
struct Settings {
    write_version: u8,
    read_version: u8,
    change_counter: u32,
    schema_cookie: Option<u32>,
    default_cache_size: u32,
    auto_vacuum: bool,
    incremental_vacuum: bool,
    user_version: u32,
    application_id: u32,
    sqlite_version: u32,
    /// The source header left the text encoding unset (0).
    encoding_unset: bool,
}

/// One column of an index key.
#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexColumn {
    column: usize,
    collation: Collation,
    desc: bool,
}

/// An index rebuilt from its table's rows.
#[derive(Debug, Clone)]
struct IndexDef {
    /// Position of the table in `objects`.
    table: usize,
    unique: bool,
    columns: Vec<IndexColumn>,
}

/// A rowid table built from rows.
#[derive(Debug, Clone)]
struct TableData {
    info: TableInfo,
    collations: Vec<Option<String>>,
    keys: Vec<KeyConstraint>,
    autoincrement: bool,
    /// Values per declared column (rowid alias and VIRTUAL columns hold NULL), by rowid.
    rows: BTreeMap<i64, Vec<Value>>,
}

#[derive(Debug, Clone)]
enum Body {
    /// Views, triggers, virtual tables: no b-tree (root page 0).
    NoStorage,
    Table(Box<TableData>),
    Index(IndexDef),
    /// The b-tree rooted at this page of the source database, copied page by page.
    Copy {
        src_root: u32,
    },
}

/// One `sqlite_schema` row and what its b-tree holds.
#[derive(Debug, Clone)]
struct Object {
    kind: String,
    name: String,
    tbl_name: String,
    sql: Option<String>,
    body: Body,
}

/// Builds a new SQLite database image.
///
/// ```
/// # fn main() -> bicraft_sqlite::Result<()> {
/// use bicraft_sqlite::{Database, DatabaseBuilder, Value};
/// let mut b = DatabaseBuilder::new(4096)?;
/// b.create_table("t", "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT UNIQUE)")?;
/// b.insert("t", 1, vec![Value::Null, Value::Text("one".into())])?;
/// let db = Database::open(b.finish()?)?;
/// assert_eq!(db.rows_vec("t")?.len(), 1);
/// # Ok(()) }
/// ```
#[derive(Debug, Clone)]
pub struct DatabaseBuilder {
    page_size: u32,
    reserved: u8,
    encoding: TextEncoding,
    schema_format: u32,
    settings: Settings,
    objects: Vec<Object>,
}

impl DatabaseBuilder {
    /// A new, empty UTF-8 database. `page_size` must be a power of two from 512 to 65536.
    pub fn new(page_size: u32) -> Result<Self> {
        if !(512..=65536).contains(&page_size) || !page_size.is_power_of_two() {
            return Err(Error::Invalid(format!(
                "page size {page_size} is not a power of two between 512 and 65536"
            )));
        }
        Ok(Self {
            page_size,
            reserved: 0,
            encoding: TextEncoding::Utf8,
            schema_format: 4,
            settings: Settings {
                write_version: 1,
                read_version: 1,
                change_counter: 1,
                schema_cookie: None,
                default_cache_size: 0,
                auto_vacuum: false,
                incremental_vacuum: false,
                user_version: 0,
                application_id: 0,
                sqlite_version: NEW_DB_VERSION_NUMBER,
                encoding_unset: false,
            },
            objects: Vec::new(),
        })
    }

    /// An empty builder with the header settings of `db`: page size, reserved bytes, text
    /// encoding, schema format, journal versions, user version, application id, default cache
    /// size and (incremental) auto-vacuum. The change counter and schema cookie continue from
    /// the source's.
    pub fn like(db: &Database) -> Self {
        let h = db.header();
        let bytes = db.bytes();
        Self {
            page_size: h.page_size,
            reserved: h.reserved_bytes,
            encoding: h.text_encoding,
            schema_format: h.schema_format,
            settings: Settings {
                write_version: h.write_version,
                read_version: h.read_version,
                change_counter: h.file_change_counter.wrapping_add(1),
                schema_cookie: Some(h.schema_cookie),
                default_cache_size: be_u32(bytes, 48).unwrap_or(0),
                auto_vacuum: h.auto_vacuum(),
                incremental_vacuum: h.incremental_vacuum,
                user_version: h.user_version,
                application_id: h.application_id,
                sqlite_version: h.sqlite_version_number,
                encoding_unset: be_u32(bytes, 56).unwrap_or(1) == 0,
            },
            objects: Vec::new(),
        }
    }

    /// Register a rowid table. `create_sql` is stored verbatim in `sqlite_schema` and must name
    /// the table `name`. `PRIMARY KEY` (other than an `INTEGER PRIMARY KEY` rowid alias) and
    /// `UNIQUE` constraints get their `sqlite_autoindex_<name>_<n>` indexes, and an
    /// `AUTOINCREMENT` table brings `sqlite_sequence`, as SQLite does.
    pub fn create_table(&mut self, name: &str, create_sql: &str) -> Result<()> {
        self.check_new_name(name)?;
        let data = table_def(name, create_sql, true)?;
        let autoindexes = constraint_indexes(&data, self.format4())?;
        let autoincrement = data.autoincrement;
        let table = self.objects.len();
        self.objects.push(Object {
            kind: "table".into(),
            name: name.into(),
            tbl_name: name.into(),
            sql: Some(create_sql.into()),
            body: Body::Table(Box::new(data)),
        });
        for (i, (unique, columns)) in autoindexes.into_iter().enumerate() {
            self.objects.push(Object {
                kind: "index".into(),
                name: format!("sqlite_autoindex_{name}_{}", i + 1),
                tbl_name: name.into(),
                sql: None,
                body: Body::Index(IndexDef {
                    table,
                    unique,
                    columns,
                }),
            });
        }
        if autoincrement && self.position(SEQUENCE_TABLE).is_none() {
            let seq = table_def(SEQUENCE_TABLE, SEQUENCE_SQL, false)?;
            self.objects.push(Object {
                kind: "table".into(),
                name: SEQUENCE_TABLE.into(),
                tbl_name: SEQUENCE_TABLE.into(),
                sql: Some(SEQUENCE_SQL.into()),
                body: Body::Table(Box::new(seq)),
            });
        }
        Ok(())
    }

    /// Register an index on a table created earlier. The writer builds the index b-tree from
    /// the table's rows (key = the indexed columns then the rowid). Plain column indexes with
    /// `COLLATE BINARY | NOCASE | RTRIM` and `ASC | DESC` are supported; expression and partial
    /// (`WHERE`) indexes are rejected with [`Error::Unsupported`].
    pub fn create_index(&mut self, name: &str, table: &str, create_sql: &str) -> Result<()> {
        self.check_new_name(name)?;
        let spec = parse_create_index(create_sql).map_err(|e| index_error(name, e))?;
        if spec.name != name {
            return Err(Error::Invalid(format!(
                "CREATE INDEX names `{}`, not `{name}`",
                spec.name
            )));
        }
        if !spec.table.eq_ignore_ascii_case(table) {
            return Err(Error::Invalid(format!(
                "CREATE INDEX `{name}` is on `{}`, not `{table}`",
                spec.table
            )));
        }
        if spec.partial {
            return Err(Error::Unsupported(format!(
                "partial index `{name}` (WHERE clause)"
            )));
        }
        let t = self
            .position(table)
            .ok_or_else(|| Error::NoSuchTable(table.into()))?;
        let format4 = self.format4();
        let obj = self
            .objects
            .get(t)
            .ok_or_else(|| Error::NoSuchTable(table.into()))?;
        let Body::Table(data) = &obj.body else {
            return Err(Error::Unsupported(format!(
                "`{table}` is not a table the writer builds"
            )));
        };
        let columns = resolve_columns(data, &spec.columns, format4)?;
        let tbl_name = obj.name.clone();
        self.objects.push(Object {
            kind: "index".into(),
            name: name.into(),
            tbl_name,
            sql: Some(create_sql.into()),
            body: Body::Index(IndexDef {
                table: t,
                unique: spec.unique,
                columns,
            }),
        });
        Ok(())
    }

    /// Add a row. `values` holds one value per declared column; an `INTEGER PRIMARY KEY`
    /// column must be `Null` or equal `rowid`, and VIRTUAL generated columns are ignored.
    /// Values are stored with their own storage class, except that NaN becomes NULL and, as
    /// SQLite does, an integral REAL in an INTEGER/NUMERIC column becomes an INTEGER (a REAL
    /// column stores it in integer form and still reads it back as REAL).
    pub fn insert(&mut self, table: &str, rowid: i64, values: Vec<Value>) -> Result<()> {
        let t = self
            .position(table)
            .ok_or_else(|| Error::NoSuchTable(table.into()))?;
        let is_internal = self
            .objects
            .get(t)
            .is_some_and(|o| o.name.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("sqlite_")));
        if is_internal {
            return Err(Error::Invalid(format!("`{table}` is maintained by the writer")));
        }
        self.insert_at(t, rowid, values)
    }

    /// Lay out the database and return the file image.
    pub fn finish(self) -> Result<Vec<u8>> {
        self.build(None)
    }

    // -----------------------------------------------------------------------------------------

    fn format4(&self) -> bool {
        self.schema_format == 0 || self.schema_format >= 4
    }

    /// The object called `name`: exact match first, then ASCII case-insensitive.
    fn position(&self, name: &str) -> Option<usize> {
        self.objects.iter().position(|o| o.name == name).or_else(|| {
            self.objects
                .iter()
                .position(|o| o.name.eq_ignore_ascii_case(name))
        })
    }

    fn check_new_name(&self, name: &str) -> Result<()> {
        if name.is_empty() {
            return Err(Error::Invalid("empty object name".into()));
        }
        if name.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("sqlite_")) {
            return Err(Error::Invalid(format!(
                "`{name}`: names starting with sqlite_ are reserved"
            )));
        }
        if self.objects.iter().any(|o| o.name.eq_ignore_ascii_case(name)) {
            return Err(Error::Invalid(format!(
                "there is already an object named `{name}`"
            )));
        }
        Ok(())
    }

    fn insert_at(&mut self, t: usize, rowid: i64, values: Vec<Value>) -> Result<()> {
        let obj = self
            .objects
            .get_mut(t)
            .ok_or_else(|| Error::NoSuchTable(String::new()))?;
        let Body::Table(data) = &mut obj.body else {
            return Err(Error::Unsupported(format!(
                "`{}` is not a table the writer builds",
                obj.name
            )));
        };
        let cols = &data.info.columns;
        if values.len() != cols.len() {
            return Err(Error::Invalid(format!(
                "`{}` has {} columns but the row has {} values",
                obj.name,
                cols.len(),
                values.len()
            )));
        }
        let mut stored = Vec::with_capacity(values.len());
        for (col, v) in cols.iter().zip(values) {
            let v = if col.rowid_alias {
                match v {
                    Value::Null => Value::Null,
                    Value::Integer(i) if i == rowid => Value::Null,
                    other => {
                        return Err(Error::Invalid(format!(
                            "`{}`.`{}` is the rowid ({rowid}) but the row gives {other:?}",
                            obj.name, col.name
                        )));
                    }
                }
            } else if col.generated == Some(Generated::Virtual) {
                Value::Null
            } else {
                normalize(v, col.affinity)
            };
            stored.push(v);
        }
        if data.rows.insert(rowid, stored).is_some() {
            return Err(Error::Invalid(format!(
                "duplicate rowid {rowid} in `{}`",
                obj.name
            )));
        }
        Ok(())
    }

    /// Make `sqlite_sequence` hold, for every AUTOINCREMENT table built from rows, a value at
    /// least as large as its largest rowid.
    fn fix_sequence(&mut self) -> Result<()> {
        let Some(seq_at) = self.objects.iter().position(|o| o.name == SEQUENCE_TABLE) else {
            return Ok(());
        };
        let mut wanted: Vec<(String, i64)> = Vec::new();
        for o in &self.objects {
            if let Body::Table(d) = &o.body
                && d.autoincrement
                && let Some((&max, _)) = d.rows.last_key_value()
            {
                wanted.push((o.name.clone(), max));
            }
        }
        let Some(Object {
            body: Body::Table(seq),
            ..
        }) = self.objects.get_mut(seq_at)
        else {
            return Ok(());
        };
        if seq.info.columns.len() != 2 {
            return Err(Error::Unsupported(
                "sqlite_sequence does not have two columns".into(),
            ));
        }
        for (name, max) in wanted {
            let found = seq
                .rows
                .values_mut()
                .find(|v| v.first() == Some(&Value::Text(name.clone())));
            match found {
                Some(v) => {
                    let current = v.get(1).and_then(Value::as_i64);
                    if current.is_none_or(|c| c < max)
                        && let Some(slot) = v.get_mut(1)
                    {
                        *slot = Value::Integer(max);
                    }
                }
                None => {
                    let rowid = seq
                        .rows
                        .last_key_value()
                        .map_or(Some(1), |(k, _)| k.checked_add(1));
                    let rowid = rowid.ok_or_else(|| Error::Invalid("sqlite_sequence is full".into()))?;
                    seq.rows
                        .insert(rowid, vec![Value::Text(name), Value::Integer(max)]);
                }
            }
        }
        Ok(())
    }

    fn build(mut self, source: Option<&Database>) -> Result<Vec<u8>> {
        self.fix_sequence()?;
        let has_objects = !self.objects.is_empty();
        let schema_format = if self.schema_format == 0 && has_objects {
            4
        } else {
            self.schema_format
        };
        let format4 = schema_format >= 4;
        let enc = self.encoding;
        let mut pager = Pager::new(self.page_size, self.reserved, self.settings.auto_vacuum);

        // Root pages first, in schema order (auto-vacuum wants them at the front).
        let mut roots = Vec::with_capacity(self.objects.len());
        let mut copy_roots: BTreeMap<u32, u32> = BTreeMap::new();
        for o in &self.objects {
            let root = match &o.body {
                Body::NoStorage => 0,
                Body::Copy { src_root } => match copy_roots.get(src_root) {
                    Some(r) => *r,
                    None => {
                        let r = pager.alloc()?;
                        copy_roots.insert(*src_root, r);
                        r
                    }
                },
                Body::Table(_) | Body::Index(_) => pager.alloc()?,
            };
            roots.push(root);
        }

        let mut copier = source.map(|db| Copier::new(db.source()));
        let mut copied = BTreeSet::new();
        for (o, &root) in self.objects.iter().zip(&roots) {
            match &o.body {
                Body::NoStorage => {}
                Body::Table(data) => {
                    let mut cells = Vec::with_capacity(data.rows.len());
                    for (&rowid, values) in &data.rows {
                        let payload = table_record(data, values, enc, format4)?;
                        cells.push((rowid, table_leaf_cell(&mut pager, rowid, &payload)?));
                    }
                    build_tree(&mut pager, root, Leaves::Table(cells))?;
                }
                Body::Index(def) => {
                    let Some(Object {
                        body: Body::Table(data),
                        ..
                    }) = self.objects.get(def.table)
                    else {
                        return Err(Error::Invalid(format!(
                            "index `{}` has no table to build from",
                            o.name
                        )));
                    };
                    let payloads = index_records(&o.name, data, def, enc, format4)?;
                    let mut cells: Vec<Cell> = Vec::with_capacity(payloads.len());
                    for p in payloads {
                        cells.push(index_cell(&mut pager, &p)?);
                    }
                    build_tree(&mut pager, root, Leaves::Index(cells))?;
                }
                Body::Copy { src_root } => {
                    if copied.insert(*src_root) {
                        let c = copier
                            .as_mut()
                            .ok_or_else(|| Error::Invalid("page copy without a source database".into()))?;
                        c.copy_tree(&mut pager, *src_root, root)?;
                    }
                }
            }
        }

        // sqlite_schema on page 1.
        let mut cells = Vec::with_capacity(self.objects.len());
        for (i, (o, &root)) in self.objects.iter().zip(&roots).enumerate() {
            let values = [
                Value::Text(o.kind.clone()),
                Value::Text(o.name.clone()),
                Value::Text(o.tbl_name.clone()),
                Value::Integer(i64::from(root)),
                o.sql.clone().map_or(Value::Null, Value::Text),
            ];
            let fields: Vec<Field<'_>> = values
                .iter()
                .map(|value| Field {
                    value,
                    int_real: false,
                })
                .collect();
            let payload = encode_record(&fields, enc, format4)?;
            let rowid = i64::try_from(i + 1).map_err(|_| Error::Invalid("too many schema objects".into()))?;
            cells.push((rowid, table_leaf_cell(&mut pager, rowid, &payload)?));
        }
        build_tree(&mut pager, 1, Leaves::Table(cells))?;

        // The database header.
        let s = &self.settings;
        let page_count = pager.page_count();
        let largest_root = roots.iter().copied().max().unwrap_or(0).max(1);
        let schema_cookie = match s.schema_cookie {
            // the root pages moved, so readers must reload the schema
            Some(c) => c.wrapping_add(1).max(1),
            None => u32::try_from(self.objects.len()).unwrap_or(u32::MAX),
        };
        let enc_code = match enc {
            _ if s.encoding_unset && !has_objects => 0,
            TextEncoding::Utf8 => 1,
            TextEncoding::Utf16Le => 2,
            TextEncoding::Utf16Be => 3,
        };
        let mut h = [0u8; 100];
        h[..16].copy_from_slice(MAGIC);
        let ps = if self.page_size == 65536 {
            1u16
        } else {
            self.page_size as u16
        };
        h[16..18].copy_from_slice(&ps.to_be_bytes());
        h[18] = s.write_version;
        h[19] = s.read_version;
        h[20] = self.reserved;
        h[21] = 64;
        h[22] = 32;
        h[23] = 32;
        for (at, v) in [
            (24, s.change_counter),
            (28, page_count),
            (
                40,
                if has_objects {
                    schema_cookie
                } else {
                    s.schema_cookie.unwrap_or(0)
                },
            ),
            (44, schema_format),
            (48, s.default_cache_size),
            (52, if s.auto_vacuum { largest_root } else { 0 }),
            (56, enc_code),
            (60, s.user_version),
            (64, u32::from(s.auto_vacuum && s.incremental_vacuum)),
            (68, s.application_id),
            (92, s.change_counter),
            (96, s.sqlite_version),
        ] {
            put_bytes(&mut h, at, &u32::to_be_bytes(v))?;
        }
        let page1 = pager.page1_mut()?;
        if page1.len() < 100 {
            page1.resize(100, 0);
        }
        put_bytes(page1, 0, &h)?;
        pager.into_bytes()
    }
}

/// Rewrite `db`: the same schema objects in the same order (tables, indexes, views and
/// triggers; SQL text verbatim), with the rows of each table supplied by `rows_for` (`None`
/// keeps the original rows). A table that gets new rows has its b-tree and all its indexes
/// rebuilt, including the `sqlite_autoindex_*` indexes of its `UNIQUE` / `PRIMARY KEY`
/// constraints; kept tables (and their indexes) are copied page by page, so anything the
/// writer cannot rebuild — `WITHOUT ROWID` tables, expression or partial indexes — survives as
/// long as its rows are kept. Freelist pages are dropped. `sqlite_sequence` is raised to the
/// largest rowid of each rebuilt `AUTOINCREMENT` table.
///
/// `rows_for` is called once per table (including `sqlite_sequence` and `sqlite_stat*`), in
/// schema order.
pub fn rewrite(db: &Database, mut rows_for: impl FnMut(&str) -> Option<Vec<Row>>) -> Result<Vec<u8>> {
    let mut b = DatabaseBuilder::like(db);
    let format4 = b.format4();
    let schema = db.schema();

    // Pass 1: tables, in schema order.
    let mut bodies: BTreeMap<usize, Body> = BTreeMap::new();
    let mut rebuilt_autoinc = false;
    for (i, e) in schema.iter().enumerate() {
        if e.kind != "table" {
            continue;
        }
        let rows = rows_for(&e.name);
        let info = db.tables().iter().find(|t| t.name == e.name);
        let rebuildable = info.is_some_and(|t| t.kind == TableKind::Rowid && t.parse_error.is_none());
        let body = match (rows, rebuildable && e.root_page != 0) {
            (Some(rows), true) => {
                let data = table_def(&e.name, &e.sql, false)?;
                rebuilt_autoinc |= data.autoincrement;
                let t = b.objects.len();
                b.objects.push(Object {
                    kind: e.kind.clone(),
                    name: e.name.clone(),
                    tbl_name: e.tbl_name.clone(),
                    sql: Some(e.sql.clone()),
                    body: Body::Table(Box::new(data)),
                });
                for row in rows {
                    b.insert_at(t, row.rowid, row.values)?;
                }
                let Some(o) = b.objects.pop() else {
                    return Err(Error::Invalid("lost a table".into()));
                };
                o.body
            }
            (Some(_), false) => {
                return Err(Error::Unsupported(format!(
                    "cannot rebuild `{}` (WITHOUT ROWID, virtual or unparseable table)",
                    e.name
                )));
            }
            (None, _) if e.root_page == 0 => Body::NoStorage,
            (None, _) => Body::Copy {
                src_root: e.root_page,
            },
        };
        bodies.insert(i, body);
    }

    // sqlite_sequence must follow the rebuilt AUTOINCREMENT tables.
    if rebuilt_autoinc
        && let Some((i, e)) = schema
            .iter()
            .enumerate()
            .find(|(_, e)| e.kind == "table" && e.name == SEQUENCE_TABLE)
        && matches!(bodies.get(&i), Some(Body::Copy { .. }))
    {
        let mut data = table_def(&e.name, &e.sql, false)?;
        for row in db.rows_vec(&e.name)? {
            if data.rows.insert(row.rowid, row.values).is_some() {
                return Err(Error::Invalid("duplicate rowid in sqlite_sequence".into()));
            }
        }
        bodies.insert(i, Body::Table(Box::new(data)));
    }

    // Pass 2: indexes. Objects keep the schema order, so a table's object index is its schema
    // position.
    let find_table = |name: &str| {
        let tables = || schema.iter().enumerate().filter(|(_, e)| e.kind == "table");
        tables()
            .find(|(_, e)| e.name == name)
            .or_else(|| tables().find(|(_, e)| e.name.eq_ignore_ascii_case(name)))
            .map(|(i, _)| i)
    };
    for (i, e) in schema.iter().enumerate() {
        if e.kind != "index" {
            continue;
        }
        let parent = find_table(&e.tbl_name);
        let body = match parent.and_then(|p| bodies.get(&p).map(|body| (p, body))) {
            Some((p, Body::Table(data))) => {
                Body::Index(rebuild_index_def(&e.name, &e.sql, p, data, format4)?)
            }
            _ if e.root_page == 0 => Body::NoStorage,
            _ => Body::Copy {
                src_root: e.root_page,
            },
        };
        bodies.insert(i, body);
    }
    for (i, e) in schema.iter().enumerate() {
        b.objects.push(Object {
            kind: e.kind.clone(),
            name: e.name.clone(),
            tbl_name: e.tbl_name.clone(),
            sql: (!e.sql.is_empty()).then(|| e.sql.clone()),
            body: bodies.remove(&i).unwrap_or(Body::NoStorage),
        });
    }

    // Every constraint index of a rebuilt table must exist in the source (else the rebuilt
    // table would lose an index SQLite expects).
    for o in &b.objects {
        if let Body::Table(data) = &o.body {
            let derived = constraint_indexes(data, format4)?;
            for n in 1..=derived.len() {
                let want = format!("sqlite_autoindex_{}_{n}", o.name);
                if !b.objects.iter().any(|x| x.kind == "index" && x.name == want) {
                    return Err(Error::Unsupported(format!(
                        "`{}` needs automatic index `{want}`, which the source lacks",
                        o.name
                    )));
                }
            }
        }
    }
    b.build(Some(db))
}

fn index_error(name: &str, e: IndexParseError) -> Error {
    match e {
        IndexParseError::Malformed(reason) => Error::Schema {
            table: name.into(),
            reason,
        },
        IndexParseError::Unsupported(reason) => Error::Unsupported(reason),
    }
}

/// The definition of a source index on a table that is being rebuilt.
fn rebuild_index_def(
    name: &str,
    sql: &str,
    table: usize,
    data: &TableData,
    format4: bool,
) -> Result<IndexDef> {
    if sql.is_empty() {
        // An automatic index: the n-th constraint index of its table.
        let derived = constraint_indexes(data, format4)?;
        let prefix = format!("sqlite_autoindex_{}_", data.info.name);
        let n = name
            .strip_prefix(&prefix)
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|&n| n >= 1);
        let def = n.and_then(|n| derived.into_iter().nth(n - 1));
        return match def {
            Some((unique, columns)) => Ok(IndexDef {
                table,
                unique,
                columns,
            }),
            None => Err(Error::Unsupported(format!(
                "cannot match automatic index `{name}` to a constraint of `{}`",
                data.info.name
            ))),
        };
    }
    let spec = parse_create_index(sql).map_err(|e| index_error(name, e))?;
    if spec.partial {
        return Err(Error::Unsupported(format!(
            "cannot rebuild partial index `{name}` (WHERE clause); keep the rows of `{}`",
            data.info.name
        )));
    }
    Ok(IndexDef {
        table,
        unique: spec.unique,
        columns: resolve_columns(data, &spec.columns, format4)?,
    })
}

/// Parse a `CREATE TABLE` for the writer.
fn table_def(name: &str, sql: &str, check_name: bool) -> Result<TableData> {
    let info = TableInfo::from_schema(name.into(), 0, sql.into());
    if let Some(reason) = &info.parse_error {
        return Err(Error::Schema {
            table: name.into(),
            reason: reason.clone(),
        });
    }
    match info.kind {
        TableKind::Rowid => {}
        TableKind::WithoutRowid => {
            return Err(Error::Unsupported(format!("`{name}` is a WITHOUT ROWID table")));
        }
        TableKind::Virtual => return Err(Error::Unsupported(format!("`{name}` is a virtual table"))),
    }
    let extras = table_extras(sql).map_err(|reason| Error::Schema {
        table: name.into(),
        reason,
    })?;
    if check_name && extras.table_name != name {
        return Err(Error::Invalid(format!(
            "CREATE TABLE names `{}`, not `{name}`",
            extras.table_name
        )));
    }
    Ok(TableData {
        info,
        collations: extras.collations,
        keys: extras.keys,
        autoincrement: extras.autoincrement,
        rows: BTreeMap::new(),
    })
}

/// Resolve the key columns of an index against its table.
fn resolve_columns(data: &TableData, cols: &[KeyColumn], format4: bool) -> Result<Vec<IndexColumn>> {
    let table = &data.info.name;
    let mut out = Vec::with_capacity(cols.len());
    for kc in cols {
        let ci = data.info.column_index(&kc.name).ok_or_else(|| Error::Schema {
            table: table.clone(),
            reason: format!("no column `{}` to index", kc.name),
        })?;
        let col = data
            .info
            .columns
            .get(ci)
            .ok_or_else(|| Error::Invalid("column index".into()))?;
        if col.generated == Some(Generated::Virtual) {
            return Err(Error::Unsupported(format!(
                "index on VIRTUAL generated column `{table}`.`{}`",
                col.name
            )));
        }
        let coll_name = kc
            .collation
            .as_ref()
            .or(data.collations.get(ci).and_then(Option::as_ref));
        let collation = match coll_name {
            None => Collation::Binary,
            Some(n) => Collation::parse(n).ok_or_else(|| Error::Unsupported(format!("collation `{n}`")))?,
        };
        out.push(IndexColumn {
            column: ci,
            collation,
            // descending keys need schema format 4
            desc: kc.desc && format4,
        });
    }
    Ok(out)
}

/// The indexes SQLite creates for a table's `PRIMARY KEY` and `UNIQUE` constraints, in
/// creation order: an `INTEGER PRIMARY KEY` rowid alias needs none, and a constraint with the
/// same columns and collations as an earlier one reuses it.
fn constraint_indexes(data: &TableData, format4: bool) -> Result<Vec<(bool, Vec<IndexColumn>)>> {
    let mut out: Vec<(bool, Vec<IndexColumn>)> = Vec::new();
    for key in &data.keys {
        if key.primary
            && let [only] = key.columns.as_slice()
            && data
                .info
                .column_index(&only.name)
                .and_then(|i| data.info.columns.get(i))
                .is_some_and(|c| c.rowid_alias)
        {
            continue;
        }
        let cols = resolve_columns(data, &key.columns, format4)?;
        let same = |other: &[IndexColumn]| {
            other.len() == cols.len()
                && other
                    .iter()
                    .zip(&cols)
                    .all(|(a, b)| a.column == b.column && a.collation == b.collation)
        };
        if out.iter().any(|(_, o)| same(o)) {
            continue;
        }
        out.push((true, cols));
    }
    Ok(out)
}

/// Apply the storage rules for a value going into a column of affinity `aff`.
fn normalize(v: Value, aff: Affinity) -> Value {
    match v {
        Value::Real(r) if r.is_nan() => Value::Null,
        Value::Real(r) if matches!(aff, Affinity::Integer | Affinity::Numeric) => {
            real_as_int(r).map_or(Value::Real(r), Value::Integer)
        }
        v => v,
    }
}

/// The record of one table row (VIRTUAL generated columns are not stored).
fn table_record(data: &TableData, values: &[Value], enc: TextEncoding, format4: bool) -> Result<Vec<u8>> {
    let fields: Vec<Field<'_>> = data
        .info
        .columns
        .iter()
        .zip(values)
        .filter(|(c, _)| c.generated != Some(Generated::Virtual))
        .map(|(c, value)| Field {
            value,
            int_real: c.affinity == Affinity::Real,
        })
        .collect();
    encode_record(&fields, enc, format4)
}

/// The sorted records of an index: the key columns then the rowid.
fn index_records(
    name: &str,
    data: &TableData,
    def: &IndexDef,
    enc: TextEncoding,
    format4: bool,
) -> Result<Vec<Vec<u8>>> {
    let cols = &data.info.columns;
    let mut keys: Vec<(i64, Vec<Value>)> = Vec::with_capacity(data.rows.len());
    for (&rowid, values) in &data.rows {
        let key = def
            .columns
            .iter()
            .map(|ic| match cols.get(ic.column) {
                Some(c) if c.rowid_alias => Value::Integer(rowid),
                _ => values.get(ic.column).cloned().unwrap_or(Value::Null),
            })
            .collect();
        keys.push((rowid, key));
    }
    let cmp_key = |a: &[Value], b: &[Value]| {
        for ((x, y), ic) in a.iter().zip(b).zip(&def.columns) {
            let o = value_cmp(x, y, ic.collation, enc);
            let o = if ic.desc { o.reverse() } else { o };
            if o != Ordering::Equal {
                return o;
            }
        }
        Ordering::Equal
    };
    keys.sort_by(|(ra, a), (rb, b)| cmp_key(a, b).then(ra.cmp(rb)));
    if def.unique {
        for w in keys.windows(2) {
            if let [(_, a), (_, b)] = w
                && !a.iter().any(Value::is_null)
                && cmp_key(a, b) == Ordering::Equal
            {
                return Err(Error::Invalid(format!(
                    "UNIQUE constraint failed: index `{name}` on `{}` has duplicate keys",
                    data.info.name
                )));
            }
        }
    }
    let real: Vec<bool> = def
        .columns
        .iter()
        .map(|ic| {
            cols.get(ic.column)
                .is_some_and(|c| c.affinity == Affinity::Real && !c.rowid_alias)
        })
        .collect();
    let mut out = Vec::with_capacity(keys.len());
    for (rowid, key) in &keys {
        let rowid_value = Value::Integer(*rowid);
        let mut fields: Vec<Field<'_>> = key
            .iter()
            .zip(&real)
            .map(|(value, &int_real)| Field { value, int_real })
            .collect();
        fields.push(Field {
            value: &rowid_value,
            int_real: false,
        });
        out.push(encode_record(&fields, enc, format4)?);
    }
    Ok(out)
}
