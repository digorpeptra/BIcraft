//! Case-insensitive name lookup over a [`SemanticModel`].
//!
//! The binder, the type checker and the dependency graph look up a table, column or measure for
//! every reference they meet. [`SemanticModel::measure`] and friends scan the model, which is fine
//! for one expression but quadratic when a whole model is validated. A [`ModelIndex`] hashes every
//! name once; [`Names`] lets the same code run on either a plain model (scans, nothing built) or an
//! index. Both answer exactly the same: DAX compares names ASCII case-insensitively and the first
//! match in model order wins.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use bicraft_model::{Column, Measure, SemanticModel, Table};

/// Every table, column and measure name of one model, upper-cased. Build it once per model
/// snapshot and reuse it for every expression checked against that snapshot.
#[derive(Debug, Clone)]
pub struct ModelIndex<'m> {
    model: &'m SemanticModel,
    /// Table name → first table with that name.
    tables: HashMap<String, usize>,
    /// Measure name → first (table, measure) with that name, in model order.
    measures: HashMap<String, (usize, usize)>,
    /// Visible column name → first (table, column) with that name, in model order.
    any_column: HashMap<String, (usize, usize)>,
    /// Per table (same order as `model.tables`): its visible columns and its measures.
    per_table: Vec<TableNames>,
}

#[derive(Debug, Clone, Default)]
struct TableNames {
    columns: HashMap<String, usize>,
    measures: HashMap<String, usize>,
}

/// `map[name.to_ascii_uppercase()]`, without allocating for names of up to 64 bytes.
fn get<'a, V>(map: &'a HashMap<String, V>, name: &str) -> Option<&'a V> {
    let mut buf = [0u8; 64];
    if let Some(b) = buf.get_mut(..name.len()) {
        b.copy_from_slice(name.as_bytes());
        b.make_ascii_uppercase();
        // ASCII case mapping keeps UTF-8 valid, so this always succeeds.
        if let Ok(upper) = std::str::from_utf8(b) {
            return map.get(upper);
        }
    }
    map.get(&name.to_ascii_uppercase())
}

fn first<V>(map: &mut HashMap<String, V>, name: &str, value: V) {
    if let Entry::Vacant(e) = map.entry(name.to_ascii_uppercase()) {
        e.insert(value);
    }
}

impl<'m> ModelIndex<'m> {
    pub fn new(model: &'m SemanticModel) -> Self {
        let mut ix = ModelIndex {
            model,
            tables: HashMap::with_capacity(model.tables.len()),
            measures: HashMap::new(),
            any_column: HashMap::new(),
            per_table: Vec::with_capacity(model.tables.len()),
        };
        for (ti, t) in model.tables.iter().enumerate() {
            first(&mut ix.tables, &t.name, ti);
            let mut names = TableNames::default();
            for (ci, c) in t.columns.iter().enumerate() {
                // Engine-internal row-number columns are invisible to DAX (`Table::column`).
                if c.is_row_number() {
                    continue;
                }
                first(&mut names.columns, &c.name, ci);
                first(&mut ix.any_column, &c.name, (ti, ci));
            }
            for (mi, m) in t.measures.iter().enumerate() {
                first(&mut names.measures, &m.name, mi);
                first(&mut ix.measures, &m.name, (ti, mi));
            }
            ix.per_table.push(names);
        }
        ix
    }

    /// The indexed model.
    pub fn model(&self) -> &'m SemanticModel {
        self.model
    }

    /// Same as [`SemanticModel::table`].
    pub fn table(&self, name: &str) -> Option<&'m Table> {
        self.table_at(name).map(|t| t.table)
    }

    /// Same as [`SemanticModel::measure`].
    pub fn measure(&self, name: &str) -> Option<(&'m Table, &'m Measure)> {
        let &(ti, mi) = get(&self.measures, name)?;
        let t = self.model.tables.get(ti)?;
        t.measures.get(mi).map(|m| (t, m))
    }

    /// Same as `model.table(table)?.column(name)` ([`Table::column`]).
    pub fn column(&self, table: &str, name: &str) -> Option<(&'m Table, &'m Column)> {
        let t = self.table_at(table)?;
        self.column_of(t, name).map(|c| (t.table, c))
    }

    fn table_at(&self, name: &str) -> Option<TableAt<'m>> {
        let &idx = get(&self.tables, name)?;
        self.model.tables.get(idx).map(|table| TableAt { idx, table })
    }

    fn column_of(&self, t: TableAt<'m>, name: &str) -> Option<&'m Column> {
        let &ci = get(&self.per_table.get(t.idx)?.columns, name)?;
        t.table.columns.get(ci)
    }

    fn measure_of(&self, t: TableAt<'m>, name: &str) -> Option<&'m Measure> {
        let &mi = get(&self.per_table.get(t.idx)?.measures, name)?;
        t.table.measures.get(mi)
    }

    fn any_column(&self, name: &str) -> Option<(&'m Table, &'m Column)> {
        let &(ti, ci) = get(&self.any_column, name)?;
        let t = self.model.tables.get(ti)?;
        t.columns.get(ci).map(|c| (t, c))
    }
}

/// A table found by name, with its position in `model.tables`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TableAt<'m> {
    idx: usize,
    pub table: &'m Table,
}

/// Name lookup: scans of the model, or a prebuilt [`ModelIndex`]. Same answers either way.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Names<'m> {
    Scan(&'m SemanticModel),
    Index(&'m ModelIndex<'m>),
    /// A model and tables added after its own (a query's result and variable tables), found by
    /// a scan as [`Names::Scan`] finds the tables of a model holding them at its end, without a
    /// copy of the model.
    Overlay(&'m SemanticModel, &'m [Table]),
}

impl<'m> Names<'m> {
    pub fn model(self) -> &'m SemanticModel {
        match self {
            Names::Scan(m) | Names::Overlay(m, _) => m,
            Names::Index(ix) => ix.model,
        }
    }

    /// The first table named `name` ([`SemanticModel::table`]).
    pub fn table_at(self, name: &str) -> Option<TableAt<'m>> {
        match self {
            Names::Scan(m) => m
                .tables
                .iter()
                .enumerate()
                .find(|(_, t)| t.name.eq_ignore_ascii_case(name))
                .map(|(idx, table)| TableAt { idx, table }),
            Names::Index(ix) => ix.table_at(name),
            Names::Overlay(m, extra) => Names::Scan(m).table_at(name).or_else(|| {
                extra
                    .iter()
                    .enumerate()
                    .find(|(_, t)| t.name.eq_ignore_ascii_case(name))
                    .map(|(i, table)| TableAt {
                        idx: m.tables.len().saturating_add(i),
                        table,
                    })
            }),
        }
    }

    pub fn table(self, name: &str) -> Option<&'m Table> {
        self.table_at(name).map(|t| t.table)
    }

    /// A visible column of `t` ([`Table::column`]).
    pub fn column_of(self, t: TableAt<'m>, name: &str) -> Option<&'m Column> {
        match self {
            Names::Scan(_) | Names::Overlay(..) => t.table.column(name),
            Names::Index(ix) => ix.column_of(t, name),
        }
    }

    /// `model.table(table)?.column(name)`.
    pub fn column(self, table: &str, name: &str) -> Option<(&'m Table, &'m Column)> {
        let t = self.table_at(table)?;
        self.column_of(t, name).map(|c| (t.table, c))
    }

    /// A measure of `t` ([`Table::measure`]).
    pub fn measure_of(self, t: TableAt<'m>, name: &str) -> Option<&'m Measure> {
        match self {
            Names::Scan(_) | Names::Overlay(..) => t.table.measure(name),
            Names::Index(ix) => ix.measure_of(t, name),
        }
    }

    /// A measure anywhere in the model ([`SemanticModel::measure`]).
    pub fn measure(self, name: &str) -> Option<(&'m Table, &'m Measure)> {
        match self {
            Names::Scan(m) => m.measure(name),
            Names::Overlay(m, extra) => m
                .measure(name)
                .or_else(|| extra.iter().find_map(|t| t.measure(name).map(|x| (t, x)))),
            Names::Index(ix) => ix.measure(name),
        }
    }

    /// The first visible column named `name` in model order.
    pub fn any_column(self, name: &str) -> Option<(&'m Table, &'m Column)> {
        match self {
            Names::Scan(m) => m.tables.iter().find_map(|t| t.column(name).map(|c| (t, c))),
            Names::Overlay(m, extra) => m
                .tables
                .iter()
                .chain(extra.iter())
                .find_map(|t| t.column(name).map(|c| (t, c))),
            Names::Index(ix) => ix.any_column(name),
        }
    }
}
