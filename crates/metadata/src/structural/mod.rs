//! Structural edits: tables, columns and relationships added or removed, with every storage row
//! (`*Storage`, `StorageFolder`, `StorageFile`, `AttributeHierarchy`, the `H$`/`R$` system
//! tables) and the list of ABF files to add and remove. See `docs/formats/metadata-sqlitedb.md`,
//! section "Structural writing".
//!
//! The edit runs in two passes over the base rows:
//!
//! 1. **Structure.** The edited model is diffed against `build(base)` for removed and added
//!    tables, columns (of kept tables) and relationships (by name). Each is validated, then
//!    removed (cascading through the ownership graph of `remove.rs`) or added (with the encoded
//!    data supplied by the caller, `add.rs`). Anything that cannot be written safely is an
//!    [`UnsupportedChange`].
//! 2. **Metadata.** The existing metadata-only writer ([`crate::apply_model_at`]) then applies
//!    every other difference (descriptions, measures of new tables, annotations, sort-by, …) on
//!    top, exactly as for a metadata-only save.

mod add;
pub(crate) mod check;
pub mod data;
mod hierarchy;
mod names;
mod remove;
pub(crate) mod rename;
mod store;

use std::collections::HashMap;

use bicraft_model::{Column, SemanticModel, Table};

use crate::input::{MetadataTables, Row};
use crate::{Error, UnsupportedChange};
use add::{ColumnHome, NewColumn, RelationshipEnds, RelationshipShape, is_hash_encoded, is_row_number};
use data::{EncodedData, StoragePlan};
use store::Store;

pub use add::ROW_NUMBER_NAME;
pub use hierarchy::{UserHierarchyDef, user_hierarchies};

/// The result of a structural edit.
#[derive(Debug, Clone, PartialEq)]
pub struct StructuralEdit {
    /// Every metadata table, edited (load the base with
    /// [`tables_from_sqlitedb_all`](crate::tables_from_sqlitedb_all)).
    pub tables: MetadataTables,
    /// ABF files to remove and add, to apply together with the new `metadata.sqlitedb`.
    pub plan: StoragePlan,
    /// The model as written, `build_model(&tables)`: the input model plus what the writer
    /// generated (the row-number column of each new table, lineage tags of new objects that had
    /// none), with every collection in creation (ID) order.
    pub model: SemanticModel,
}

pub(crate) fn apply(
    base: &MetadataTables,
    model: &SemanticModel,
    data: &EncodedData,
    now: i64,
) -> Result<StructuralEdit, Error> {
    let model = &*crate::normalize_spellings(model);
    let orig = crate::build::build(base)?;
    let diff = Diff::new(&orig, model);
    if diff.is_empty() {
        let tables = crate::apply::apply(base, model, now)?;
        let model = crate::build::build(&tables)?;
        return Ok(StructuralEdit {
            tables,
            plan: StoragePlan::default(),
            model,
        });
    }
    let mut st = Store::new(base, now)?;
    let model_id = model_row_id(base).ok_or(Error::NoModel)?;
    let users = UserTables::new(&st, model_id);
    let mut unsupported = Vec::new();
    let mut ctx = Ctx {
        st: &mut st,
        users: &users,
        model,
        data,
        unsupported: &mut unsupported,
        model_id,
    };
    ctx.validate(&diff);
    if ctx.unsupported.is_empty() {
        ctx.execute(&diff)?;
    }
    if !unsupported.is_empty() {
        return Err(Error::UnsupportedChange(unsupported));
    }
    let (tables, plan) = st.finish();

    // The model as written: generated row-number columns and lineage tags.
    let built = crate::build::build(&tables)?;
    let mut written = model.clone();
    for t in &mut written.tables {
        let Some(b) = built.tables.iter().find(|b| b.name.eq_ignore_ascii_case(&t.name)) else {
            continue;
        };
        let added_table = diff.added_tables.iter().any(|n| n.eq_ignore_ascii_case(&t.name));
        if added_table {
            if t.lineage_tag.is_none() {
                t.lineage_tag = b.lineage_tag.clone();
            }
            if !t.columns.iter().any(is_row_number)
                && let Some(rn) = b.columns.iter().find(|c| is_row_number(c))
            {
                t.columns.insert(0, rn.clone());
            }
        }
        for c in &mut t.columns {
            let added_column = added_table
                || diff
                    .added_columns
                    .iter()
                    .any(|(tn, cn)| tn.eq_ignore_ascii_case(&b.name) && cn.eq_ignore_ascii_case(&c.name));
            if added_column
                && c.lineage_tag.is_none()
                && let Some(bc) = b.columns.iter().find(|x| x.name.eq_ignore_ascii_case(&c.name))
            {
                c.lineage_tag = bc.lineage_tag.clone();
            }
        }
    }
    let tables = crate::apply::apply_fresh(&tables, &written, now, &diff.added_tables)?;
    self_check(base, &tables, &plan)?;
    // What was written, generated values included (lineage tags of new measures too).
    let model = crate::build::build(&tables)?;
    Ok(StructuralEdit { tables, plan, model })
}

/// The edit must not leave a reference dangling that was not already dangling in the base,
/// and the file plan must agree with the `StorageFile` rows.
fn self_check(base: &MetadataTables, out: &MetadataTables, plan: &StoragePlan) -> Result<(), Error> {
    let before = check::dangling_references(base);
    let new: Vec<String> = check::dangling_references(out)
        .into_iter()
        .filter(|d| !before.contains(d))
        .collect();
    if let Some(first) = new.first() {
        return Err(Error::Invalid(format!(
            "the structural edit left {} dangling reference(s), e.g. {first}",
            new.len()
        )));
    }
    let paths = check::storage_file_paths(out);
    let has = |p: &str| paths.binary_search_by(|x| x.as_str().cmp(p)).is_ok();
    if plan.remove.iter().any(|p| has(p)) {
        return Err(Error::Invalid(
            "a removed file is still named by a StorageFile row".into(),
        ));
    }
    if plan.add.iter().any(|f| !has(&f.path)) {
        return Err(Error::Invalid("an added file has no StorageFile row".into()));
    }
    Ok(())
}

/// The `ID` of the model row (the lowest one), as `build` picks it.
fn model_row_id(t: &MetadataTables) -> Option<i64> {
    t.rows("Model").iter().filter_map(|r| Row(r).id()).min()
}

/// Structural differences between the base model and the edited one.
#[derive(Debug, Default)]
struct Diff {
    removed_tables: Vec<String>,
    added_tables: Vec<String>,
    /// (table, column) of kept tables.
    removed_columns: Vec<(String, String)>,
    added_columns: Vec<(String, String)>,
    removed_relationships: Vec<String>,
    added_relationships: Vec<String>,
    /// (table, hierarchy) of kept tables whose hierarchy is gone or restructured (levels, their
    /// names or columns changed: removed, then added back).
    removed_hierarchies: Vec<(String, String)>,
    /// (table, hierarchy) of new tables and kept tables.
    added_hierarchies: Vec<(String, String)>,
}

fn find<'a, T>(items: &'a [T], name: &str, get: impl Fn(&T) -> &str) -> Option<&'a T> {
    items.iter().find(|x| get(x).eq_ignore_ascii_case(name))
}

impl Diff {
    fn new(orig: &SemanticModel, model: &SemanticModel) -> Self {
        let mut d = Diff::default();
        for o in &orig.tables {
            match find(&model.tables, &o.name, |t| &t.name) {
                None => d.removed_tables.push(o.name.clone()),
                Some(t) => {
                    for c in &o.columns {
                        if find(&t.columns, &c.name, |x| &x.name).is_none() {
                            d.removed_columns.push((o.name.clone(), c.name.clone()));
                        }
                    }
                    for c in &t.columns {
                        if find(&o.columns, &c.name, |x| &x.name).is_none() {
                            d.added_columns.push((o.name.clone(), c.name.clone()));
                        }
                    }
                    let (before, after) = (user_hierarchies(o), user_hierarchies(t));
                    for h in &before {
                        let same = find(&after, &h.name, |x| &x.name).is_some_and(|n| same_levels(h, n));
                        if !same {
                            d.removed_hierarchies.push((o.name.clone(), h.name.clone()));
                        }
                    }
                    for h in &after {
                        let same = find(&before, &h.name, |x| &x.name).is_some_and(|b| same_levels(b, h));
                        if !same {
                            d.added_hierarchies.push((o.name.clone(), h.name.clone()));
                        }
                    }
                }
            }
        }
        for t in &model.tables {
            if find(&orig.tables, &t.name, |x| &x.name).is_none() {
                d.added_tables.push(t.name.clone());
                for h in user_hierarchies(t) {
                    d.added_hierarchies.push((t.name.clone(), h.name));
                }
            }
        }
        // Relationships match by exact name, as the metadata writer matches them.
        // A relationship whose cardinality or ends changed (`relationship.set`) keeps its name;
        // its storage (the `R$` indexes, the hash-encoded "one" ends) is rebuilt: removed, then
        // added.
        let reshaped = |o: &bicraft_model::Relationship, r: &bicraft_model::Relationship| {
            let card = |c: &Option<String>, d: &str| c.as_deref().unwrap_or(d).to_ascii_lowercase();
            card(&o.from_cardinality, "many") != card(&r.from_cardinality, "many")
                || card(&o.to_cardinality, "one") != card(&r.to_cardinality, "one")
                || o.from_table != r.from_table
                || o.from_column != r.from_column
                || o.to_table != r.to_table
                || o.to_column != r.to_column
        };
        for o in &orig.relationships {
            if !model
                .relationships
                .iter()
                .any(|r| r.name == o.name && !reshaped(o, r))
            {
                d.removed_relationships.push(o.name.clone());
            }
        }
        for r in &model.relationships {
            if !orig
                .relationships
                .iter()
                .any(|o| o.name == r.name && !reshaped(o, r))
            {
                d.added_relationships.push(r.name.clone());
            }
        }
        d
    }

    fn is_empty(&self) -> bool {
        self.removed_tables.is_empty()
            && self.added_tables.is_empty()
            && self.removed_columns.is_empty()
            && self.added_columns.is_empty()
            && self.removed_relationships.is_empty()
            && self.added_relationships.is_empty()
            && self.removed_hierarchies.is_empty()
            && self.added_hierarchies.is_empty()
    }
}

/// The same stored structure: level names and columns, in order (ASCII case-insensitive).
fn same_levels(a: &UserHierarchyDef, b: &UserHierarchyDef) -> bool {
    a.levels.len() == b.levels.len()
        && a.levels
            .iter()
            .zip(&b.levels)
            .all(|((an, ac), (bn, bc))| an.eq_ignore_ascii_case(bn) && ac.eq_ignore_ascii_case(bc))
}

/// User tables of the base by lower-case name.
struct UserTables {
    by_name: HashMap<String, i64>,
}

impl UserTables {
    fn new(st: &Store, model_id: i64) -> Self {
        let by_name =
            st.t.rows("Table")
                .iter()
                .map(Row)
                .filter(|r| r.int("SystemFlags").unwrap_or(0) & 1 == 0)
                .filter(|r| r.int("ModelID").is_none_or(|m| m == model_id))
                .filter_map(|r| Some((r.text("Name")?.to_lowercase(), r.id()?)))
                .collect();
        Self { by_name }
    }

    fn get(&self, name: &str) -> Option<i64> {
        self.by_name.get(&name.to_lowercase()).copied()
    }
}

/// One structural edit in progress: the rows being edited and the inputs, borrowed for the
/// whole edit.
struct Ctx<'c> {
    st: &'c mut Store,
    users: &'c UserTables,
    model: &'c SemanticModel,
    data: &'c EncodedData,
    unsupported: &'c mut Vec<UnsupportedChange>,
    model_id: i64,
}

fn column_object(table: &str, column: &str) -> String {
    format!("column '{table}'[{column}]")
}

impl<'c> Ctx<'c> {
    fn refuse(&mut self, object: impl Into<String>, what: impl Into<String>) {
        self.unsupported.push(UnsupportedChange {
            what: what.into(),
            object: object.into(),
        });
    }

    /// The column of a user table, by name (`ExplicitName`, else `InferredName`).
    fn column_id(&self, table_id: i64, name: &str) -> Option<i64> {
        self.st
            .t
            .rows("Column")
            .iter()
            .map(Row)
            .filter(|r| r.int("TableID") == Some(table_id))
            .find(|r| {
                r.text("ExplicitName")
                    .or_else(|| r.text("InferredName"))
                    .or_else(|| r.text("Name"))
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
            .and_then(|r| r.id())
    }

    /// A table of the edited model. The reference lives as long as the model, not the context.
    fn model_table(&self, name: &str) -> Option<&'c Table> {
        let model: &'c SemanticModel = self.model;
        find(&model.tables, name, |t| &t.name)
    }

    /// Relationship rows of the base that the edit keeps.
    fn kept_relationships(&self, diff: &Diff) -> Vec<i64> {
        self.st.ids_matching("Relationship", |r| {
            r.text("Name")
                .is_none_or(|n| !diff.removed_relationships.iter().any(|x| x == n))
        })
    }

    /// Rows outside the column's own storage that would dangle if it were removed.
    fn column_blockers(&self, column_id: i64) -> Vec<&'static str> {
        let mut out = Vec::new();
        for (table, cols) in [
            ("Level", &["ColumnID"][..]),
            ("Variation", &["ColumnID", "DefaultColumnID"][..]),
            ("AlternateOf", &["ColumnID", "BaseColumnID"][..]),
            ("GroupByColumn", &["GroupingColumnID"][..]),
            ("RelatedColumnDetails", &["ColumnID"][..]),
            ("CalendarColumnReference", &["ColumnID"][..]),
            ("ColumnPermission", &["ColumnID"][..]),
            ("PerspectiveColumn", &["ColumnID"][..]),
            ("Expression", &["ParameterValuesColumnID"][..]),
        ] {
            let used = cols
                .iter()
                .any(|c| !self.st.ids_where(table, c, column_id).is_empty());
            if used {
                out.push(table);
            }
        }
        out
    }

    /// Whether `Variation` rows that the edited model keeps reference the column (as owner or
    /// default column). A variation the edit removes from its column (`column.remove_variation`,
    /// auto date tables removed) no longer blocks; one whose owner cannot be resolved does.
    fn kept_variation_uses(&self, column_id: i64) -> bool {
        let mut rows = self.st.ids_where("Variation", "ColumnID", column_id);
        rows.extend(self.st.ids_where("Variation", "DefaultColumnID", column_id));
        rows.into_iter().any(|vid| {
            let kept = || -> Option<bool> {
                let owner = self.st.reference("Variation", vid, "ColumnID")?;
                let name = self.st.text("Variation", vid, "Name")?;
                let table_id = self.st.reference("Column", owner, "TableID")?;
                let table = self.st.text("Table", table_id, "Name")?;
                let column = self
                    .st
                    .text("Column", owner, "ExplicitName")
                    .or_else(|| self.st.text("Column", owner, "InferredName"))?;
                let t = self.model_table(&table)?;
                let c = find(&t.columns, &column, |c| &c.name)?;
                Some(c.variations().iter().any(|v| v.name.eq_ignore_ascii_case(&name)))
            };
            kept().unwrap_or(true)
        })
    }

    /// Whether a column groups the related column details of a column outside `own` (a group-by
    /// row whose owner is not one of `own`, or cannot be found).
    fn grouped_from_outside(&self, column_id: i64, own: &[i64]) -> bool {
        self.st
            .ids_where("GroupByColumn", "GroupingColumnID", column_id)
            .into_iter()
            .any(|g| {
                let owner = self
                    .st
                    .reference("GroupByColumn", g, "RelatedColumnDetailsID")
                    .and_then(|d| self.st.reference("RelatedColumnDetails", d, "ColumnID"));
                owner.is_none_or(|o| !own.contains(&o))
            })
    }

    /// The `Hierarchy` row of a user table's hierarchy, by name.
    fn hierarchy_id(&self, table: &str, name: &str) -> Option<i64> {
        let tid = self.users.get(table)?;
        self.st
            .ids_matching("Hierarchy", |r| {
                r.int("TableID") == Some(tid) && r.text("Name").is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
            .first()
            .copied()
    }

    fn validate(&mut self, diff: &Diff) {
        // Removed (or restructured) hierarchies: their levels no longer hold their columns.
        let mut removed_hierarchies = Vec::new();
        for (table, name) in &diff.removed_hierarchies {
            let object = format!("hierarchy '{table}'[{name}]");
            match self.hierarchy_id(table, name) {
                None => self.refuse(object, "hierarchy to remove not found in the metadata"),
                Some(h) => {
                    if !self.st.ids_where("Variation", "DefaultHierarchyID", h).is_empty() {
                        self.refuse(&object, "removing a hierarchy a date variation uses");
                    }
                    removed_hierarchies.push(h);
                }
            }
        }
        for (table, name) in &diff.added_hierarchies {
            let object = format!("hierarchy '{table}'[{name}]");
            let Some(t) = self.model_table(table) else {
                continue;
            };
            let Some(def) = find(&user_hierarchies(t), name, |h| &h.name).cloned() else {
                continue;
            };
            if def.levels.is_empty() {
                self.refuse(&object, "a hierarchy without levels");
                continue;
            }
            let enc = self
                .data
                .hierarchies
                .iter()
                .find(|h| h.table.eq_ignore_ascii_case(table) && h.name.eq_ignore_ascii_case(name));
            match enc {
                None => self.refuse(&object, "hierarchy added without encoded nodes"),
                Some(e) if e.level_starts.len() != def.levels.len() => {
                    self.refuse(&object, "the encoded nodes do not match the levels");
                }
                Some(_) => {}
            }
            for (k, (level, column)) in def.levels.iter().enumerate() {
                if level.trim().is_empty() {
                    self.refuse(&object, format!("level {k} has no name"));
                }
                let repeated = def
                    .levels
                    .iter()
                    .take(k)
                    .any(|(l, c)| l.eq_ignore_ascii_case(level) || c.eq_ignore_ascii_case(column));
                if repeated {
                    self.refuse(&object, format!("level `{level}` repeats a level name or column"));
                }
                match find(&t.columns, column, |c| &c.name) {
                    None => self.refuse(&object, format!("level `{level}`: no column `{column}`")),
                    Some(c) if !crate::column_available_in_mdx(&c.extra) => self.refuse(
                        &object,
                        format!("level `{level}`: column `{column}` has no attribute hierarchy (isAvailableInMdx false)"),
                    ),
                    Some(_) => {}
                }
            }
        }

        let kept_rels = self.kept_relationships(diff);
        let rel_uses = |st: &Store, col: &str, id: i64| {
            kept_rels
                .iter()
                .any(|&r| st.int("Relationship", r, col) == Some(id))
        };

        // Removed tables.
        for name in &diff.removed_tables {
            let object = format!("table '{name}'");
            let Some(tid) = self.users.get(name) else {
                self.refuse(object, "table to remove not found in the metadata");
                continue;
            };
            if rel_uses(self.st, "FromTableID", tid) || rel_uses(self.st, "ToTableID", tid) {
                self.refuse(&object, "table removed while a kept relationship uses it");
            }
            if !self.st.ids_where("Calendar", "TableID", tid).is_empty() {
                self.refuse(&object, "removing a table with calendars");
            }
            let measures = self.st.ids_where("Measure", "TableID", tid);
            let default_measure = self
                .st
                .t
                .rows("Model")
                .iter()
                .filter_map(|r| Row(r).reference("DefaultMeasureID"))
                .any(|m| measures.contains(&m));
            if default_measure {
                self.refuse(&object, "removing the table of the default measure");
            }
            let own_columns = self.st.ids_where("Column", "TableID", tid);
            for &c in &own_columns {
                // Rows owned by the table itself (levels, permissions, perspectives, its columns'
                // group-by columns) go with it; these would dangle.
                let mut blockers = self.column_blockers(c);
                if !self.grouped_from_outside(c, &own_columns) {
                    blockers.retain(|b| *b != "GroupByColumn");
                }
                if !self.kept_variation_uses(c) {
                    blockers.retain(|b| *b != "Variation");
                }
                for blocker in [
                    "Variation",
                    "AlternateOf",
                    "GroupByColumn",
                    "CalendarColumnReference",
                    "Expression",
                ] {
                    if blockers.contains(&blocker) {
                        self.refuse(&object, format!("a column is referenced by `{blocker}` rows"));
                    }
                }
                if rel_uses(self.st, "FromColumnID", c) || rel_uses(self.st, "ToColumnID", c) {
                    self.refuse(&object, "a column is used by a kept relationship");
                }
            }
        }

        // Removed columns of kept tables.
        for (table, column) in &diff.removed_columns {
            let object = column_object(table, column);
            let Some(cid) = self.users.get(table).and_then(|t| self.column_id(t, column)) else {
                self.refuse(object, "column to remove not found in the metadata");
                continue;
            };
            match self.st.int("Column", cid, "Type") {
                Some(3) => self.refuse(&object, "removing the row-number column"),
                Some(4) => self.refuse(
                    &object,
                    "removing a calculated-table column (the table expression defines the columns)",
                ),
                _ => {}
            }
            if rel_uses(self.st, "FromColumnID", cid) || rel_uses(self.st, "ToColumnID", cid) {
                self.refuse(&object, "column removed while a kept relationship uses it");
            }
            let in_removed_hierarchies = self.st.ids_where("Level", "ColumnID", cid).into_iter().all(|l| {
                self.st
                    .reference("Level", l, "HierarchyID")
                    .is_some_and(|h| removed_hierarchies.contains(&h))
            });
            for blocker in self.column_blockers(cid) {
                if blocker == "Level" && in_removed_hierarchies {
                    continue;
                }
                self.refuse(
                    &object,
                    format!("column removed while `{blocker}` rows reference it"),
                );
            }
            // Columns sorted by this one must stop being sorted by it in the edited model.
            let sorted_by = self.st.ids_where("Column", "SortByColumnID", cid);
            for other in sorted_by {
                let other_name = self
                    .st
                    .row("Column", other)
                    .and_then(|r| r.text("ExplicitName").or_else(|| r.text("InferredName")))
                    .unwrap_or_default()
                    .to_string();
                let still = self
                    .model_table(table)
                    .and_then(|t| find(&t.columns, &other_name, |c| &c.name))
                    .and_then(|c| c.sort_by_column.as_deref())
                    .is_some_and(|s| s.eq_ignore_ascii_case(column));
                if still {
                    self.refuse(
                        column_object(table, &other_name),
                        format!("sortByColumn names the removed column [{column}]"),
                    );
                }
            }
        }

        // Added tables.
        for name in &diff.added_tables {
            let object = format!("table '{name}'");
            let Some(t) = self.model_table(name) else { continue };
            let data: &'c EncodedData = self.data;
            let Some(enc) = find(&data.tables, name, |e| &e.name) else {
                self.refuse(object, "table added without encoded data");
                continue;
            };
            let encoded_parts = enc.partitions.len().max(1);
            if t.partitions.is_empty() || t.partitions.len() != encoded_parts {
                self.refuse(
                    &object,
                    format!(
                        "{} partitions, {encoded_parts} encoded (a new table needs data for each partition)",
                        t.partitions.len()
                    ),
                );
            }
            for p in &t.partitions {
                let several = t.partitions.len() > 1;
                if !matches!(
                    p.source_type.as_str(),
                    "calculated" | "m" | "query" | "calculationGroup"
                ) || (several && matches!(p.source_type.as_str(), "calculated" | "calculationGroup"))
                {
                    self.refuse(&object, format!("partition source type `{}`", p.source_type));
                }
                if !p.source_extra.is_empty() {
                    self.refuse(
                        &object,
                        "partition source properties (data source, expression source)",
                    );
                }
            }
            let mut total = 0i64;
            for (k, p) in enc.partitions.iter().enumerate() {
                total = total.saturating_add(p.segment_map.record_count);
                if p.row_number.rows != p.segment_map.record_count {
                    self.refuse(
                        &object,
                        format!("partition {k}: row-number segments do not match"),
                    );
                }
                for c in t.columns.iter().filter(|c| !is_row_number(c)) {
                    match find(&p.columns, &c.name, |(n, _)| n) {
                        None => self.refuse(
                            column_object(name, &c.name),
                            format!("partition {k}: no encoded segments"),
                        ),
                        Some((_, seg)) if seg.rows != p.segment_map.record_count => self.refuse(
                            column_object(name, &c.name),
                            format!(
                                "partition {k}: {} encoded rows, the partition {}",
                                seg.rows, p.segment_map.record_count
                            ),
                        ),
                        Some(_) => {}
                    }
                }
            }
            if !enc.partitions.is_empty() && total != enc.segment_map.record_count {
                self.refuse(&object, "the partitions' rows do not add up to the table's");
            }
            // A calculation group's items are the rows written here: added with the table.
            if t.extra.contains_key("defaultDetailRowsDefinition") {
                self.refuse(&object, "new table with `defaultDetailRowsDefinition`");
            }
            let rows = enc.segment_map.record_count;
            for c in t.columns.iter().filter(|c| !is_row_number(c)) {
                let cobj = column_object(name, &c.name);
                match find(&enc.columns, &c.name, |(n, _)| n) {
                    None => self.refuse(cobj, "column of a new table without encoded data"),
                    Some((_, d)) => self.check_column_data(c, d, rows, &cobj),
                }
            }
        }

        // Added columns of existing tables.
        for (table, column) in &diff.added_columns {
            let object = column_object(table, column);
            let Some(tid) = self.users.get(table) else {
                continue;
            };
            let Some(c) = self
                .model_table(table)
                .and_then(|t| find(&t.columns, column, |x| &x.name))
            else {
                continue;
            };
            if is_row_number(c)
                || c.extra.get("type").and_then(|v| v.as_str()) == Some("calculatedTableColumn")
            {
                self.refuse(&object, "adding a row-number or calculated-table column");
                continue;
            }
            let data: &'c EncodedData = self.data;
            let Some(enc) = data
                .columns
                .iter()
                .find(|e| e.table.eq_ignore_ascii_case(table) && e.column.eq_ignore_ascii_case(column))
            else {
                self.refuse(object, "column added without encoded data");
                continue;
            };
            let parts = self.partitions(tid);
            match (parts.as_slice(), enc.partitions.as_slice()) {
                ([], _) => self.refuse(&object, "column added to a table without a partition"),
                ([(_, rows)], []) => self.check_column_data(c, &enc.data, *rows, &object),
                ([_], _) => self.refuse(&object, "segments per partition for a single-partition table"),
                (parts, segments) if parts.len() != segments.len() => self.refuse(
                    &object,
                    format!(
                        "{} encoded partitions for a table of {} partitions",
                        segments.len(),
                        parts.len()
                    ),
                ),
                (parts, segments) => {
                    let mut total = 0i64;
                    for (k, ((_, rows), seg)) in parts.iter().zip(segments).enumerate() {
                        total = total.saturating_add(*rows);
                        if seg.rows != *rows {
                            self.refuse(
                                &object,
                                format!("partition {k}: encoded {} rows, the partition {rows}", seg.rows),
                            );
                        }
                    }
                    self.check_column_data(c, &enc.data, total, &object);
                }
            }
        }

        // Added relationships.
        for name in &diff.added_relationships {
            let object = format!("relationship '{name}'");
            let Some(r) = self.model.relationships.iter().find(|r| &r.name == name) else {
                continue;
            };
            if r.name.trim().is_empty() {
                self.refuse(&object, "relationship without a name");
                continue;
            }
            let shape = RelationshipShape::of(r);
            let enc = find(&self.data.relationships, name, |e| &e.name);
            match (shape, enc) {
                (None, _) => {
                    self.refuse(
                        &object,
                        "cardinality other than many-to-one, one-to-one or many-to-many",
                    );
                    continue;
                }
                // Many-to-many: no index is stored (engine-observed).
                (Some(RelationshipShape::ManyToMany), _) => {}
                (Some(_), None) => self.refuse(&object, "relationship added without an encoded index"),
                (Some(RelationshipShape::OneToOne), Some(e)) if e.reverse.is_none() => {
                    self.refuse(&object, "one-to-one relationship without its second index");
                }
                (Some(_), Some(_)) => {}
            }
            let hash_ends: &[(&str, &str, &str)] = match shape {
                Some(RelationshipShape::ManyToOne) => &[(&r.to_table, &r.to_column, "\"one\"")],
                Some(RelationshipShape::OneToOne) => &[
                    (&r.from_table, &r.from_column, "from"),
                    (&r.to_table, &r.to_column, "to"),
                ],
                _ => &[],
            };
            for &(table, column, which) in hash_ends {
                match self.end_is_hash(diff, table, column) {
                    None => self.refuse(&object, "an end of the relationship does not resolve"),
                    Some(false) => self.refuse(&object, format!("the {which} end is not hash-encoded")),
                    Some(true) => {}
                }
            }
            for (table, column) in [(&r.from_table, &r.from_column), (&r.to_table, &r.to_column)] {
                let ok = self
                    .model_table(table)
                    .is_some_and(|t| find(&t.columns, column, |c| &c.name).is_some());
                if !ok {
                    self.refuse(&object, format!("the end '{table}'[{column}] does not resolve"));
                }
            }
        }
    }

    /// Whether the column of a relationship end is (or will be) hash-encoded; `None` when it
    /// does not resolve.
    fn end_is_hash(&self, diff: &Diff, table: &str, column: &str) -> Option<bool> {
        if diff.added_tables.iter().any(|t| t.eq_ignore_ascii_case(table)) {
            return find(&self.data.tables, table, |e| &e.name)
                .and_then(|e| find(&e.columns, column, |(n, _)| n))
                .map(|(_, d)| is_hash_encoded(d));
        }
        if let Some(enc) = self
            .data
            .columns
            .iter()
            .find(|e| e.table.eq_ignore_ascii_case(table) && e.column.eq_ignore_ascii_case(column))
        {
            return Some(is_hash_encoded(&enc.data));
        }
        self.users
            .get(table)
            .and_then(|t| self.column_id(t, column))
            .and_then(|c| self.st.reference("Column", c, "ColumnStorageID"))
            .and_then(|cs| self.st.reference("ColumnStorage", cs, "DictionaryStorageID"))
            .map(|d| self.st.int("DictionaryStorage", d, "Type") == Some(1))
    }

    fn check_column_data(&mut self, c: &Column, d: &data::ColumnStorageData, rows: i64, object: &str) {
        if add::data_type_code(c.data_type).is_none() {
            self.refuse(object, "column without a storable data type");
        }
        if d.statistics.row_count != rows {
            self.refuse(
                object,
                format!(
                    "encoded data has {} rows, the partition {rows}",
                    d.statistics.row_count
                ),
            );
        }
        let in_mdx = crate::column_available_in_mdx(&c.extra);
        if in_mdx != d.hierarchy.is_some() {
            self.refuse(
                object,
                "isAvailableInMdx and the encoded attribute hierarchy disagree",
            );
        }
    }

    /// The partitions of a table in storage order: (PartitionStorage ID, record count). Empty
    /// when one of them has no storage rows.
    fn partitions(&self, table_id: i64) -> Vec<(i64, i64)> {
        let mut out = Vec::new();
        for p in self.st.ids_where("Partition", "TableID", table_id) {
            let row = (|| {
                let ps = self.st.reference("Partition", p, "PartitionStorageID")?;
                let map = self.st.reference("PartitionStorage", ps, "SegmentMapStorageID")?;
                let rows = self.st.int("SegmentMapStorage", map, "RecordCount")?;
                let position = self
                    .st
                    .int("PartitionStorage", ps, "StoragePosition")
                    .unwrap_or(0);
                Some((position, ps, rows))
            })();
            match row {
                Some(r) => out.push(r),
                None => return Vec::new(),
            }
        }
        out.sort_by_key(|&(position, ps, _)| (position, ps));
        out.into_iter().map(|(_, ps, rows)| (ps, rows)).collect()
    }

    fn locale(&self) -> i64 {
        let mut counts: HashMap<i64, usize> = HashMap::new();
        for r in self.st.t.rows("ColumnStorage").iter().map(Row) {
            if let Some(l) = r.int("Locale").filter(|&l| l != 0) {
                *counts.entry(l).or_default() += 1;
            }
        }
        counts
            .into_iter()
            .max_by_key(|&(l, n)| (n, -l))
            .map_or(1033, |(l, _)| l)
    }

    fn execute(&mut self, diff: &Diff) -> Result<(), Error> {
        let now = self.st.now;
        // 1. Removals: relationships, columns, tables.
        for name in &diff.removed_relationships {
            for r in self
                .st
                .ids_matching("Relationship", |r| r.text("Name") == Some(name.as_str()))
            {
                self.st.drop_relationship(r)?;
            }
        }
        for (table, name) in &diff.removed_hierarchies {
            if let Some(h) = self.hierarchy_id(table, name) {
                self.st.drop_hierarchy(h)?;
                if let Some(t) = self.users.get(table) {
                    self.st.touch_table(t);
                }
            }
        }
        let mut shrunk = Vec::new();
        for (table, column) in &diff.removed_columns {
            let Some(tid) = self.users.get(table) else {
                continue;
            };
            let Some(cid) = self.column_id(tid, column) else {
                continue;
            };
            for other in self.st.ids_where("Column", "SortByColumnID", cid) {
                self.st.set("Column", other, "SortByColumnID", 0.into());
                self.st.set("Column", other, "ModifiedTime", now.into());
                self.st.set("Column", other, "StructureModifiedTime", now.into());
            }
            self.st.drop_column(cid)?;
            if !shrunk.contains(&tid) {
                shrunk.push(tid);
            }
        }
        for tid in shrunk {
            self.st.compact_column_positions(tid);
            self.st.touch_table(tid);
        }
        for name in &diff.removed_tables {
            if let Some(tid) = self.users.get(name) {
                self.st.drop_user_table(tid)?;
            }
        }

        // 2. Additions: tables, columns, relationships.
        let model = self.model;
        // Columns stored as a unique "one" end: the to end of a many-to-one relationship, both
        // ends of a one-to-one one (many-to-many ends stay as they are).
        let to_end = |t: &str, c: &str| {
            model.relationships.iter().any(|r| {
                let to = r.to_table.eq_ignore_ascii_case(t) && r.to_column.eq_ignore_ascii_case(c);
                let from = r.from_table.eq_ignore_ascii_case(t) && r.from_column.eq_ignore_ascii_case(c);
                match RelationshipShape::of(r) {
                    Some(RelationshipShape::ManyToOne) => to,
                    Some(RelationshipShape::OneToOne) => to || from,
                    _ => false,
                }
            })
        };
        let locale = self.locale();
        let mut lineage = |seed: &str| {
            let mut bytes = now.to_le_bytes().to_vec();
            bytes.extend_from_slice(b"structural\0");
            bytes.extend_from_slice(seed.as_bytes());
            crate::apply::guid(&bytes)
        };
        let mut new_tables: HashMap<String, i64> = HashMap::new();
        for name in &diff.added_tables {
            let data: &'c EncodedData = self.data;
            let (Some(t), Some(enc)) = (self.model_table(name), find(&data.tables, name, |e| &e.name)) else {
                continue;
            };
            let tid = self
                .st
                .add_table(self.model_id, t, enc, &to_end, locale, &mut lineage)?;
            new_tables.insert(name.to_lowercase(), tid);
        }
        for (table, column) in &diff.added_columns {
            let Some(tid) = self.users.get(table) else {
                continue;
            };
            let (Some(c), Some(enc)) = (
                self.model_table(table)
                    .and_then(|t| find(&t.columns, column, |x| &x.name)),
                {
                    let data: &'c EncodedData = self.data;
                    data.columns.iter().find(|e| {
                        e.table.eq_ignore_ascii_case(table) && e.column.eq_ignore_ascii_case(column)
                    })
                },
            ) else {
                continue;
            };
            let home = self.home(tid, locale)?;
            let position = self
                .st
                .ids_where("Column", "TableID", tid)
                .into_iter()
                .filter_map(|c| self.st.reference("Column", c, "ColumnStorageID"))
                .filter_map(|cs| self.st.int("ColumnStorage", cs, "StoragePosition"))
                .max()
                .map_or(0, |p| p.saturating_add(1));
            let tag = c
                .lineage_tag
                .is_none()
                .then(|| lineage(&format!("{table}\0{column}")));
            self.st.add_column(
                &home,
                &NewColumn {
                    model: Some(c),
                    data: &enc.data,
                    position,
                    to_end: to_end(table, column),
                    row_number_is_key: false,
                    lineage_tag: tag,
                    partitions: &enc.partitions,
                },
            )?;
            self.st.touch_table(tid);
        }
        let table_id = |name: &str| {
            new_tables
                .get(&name.to_lowercase())
                .copied()
                .or_else(|| self.users.get(name))
        };
        for name in &diff.added_relationships {
            let Some(r) = model.relationships.iter().find(|r| &r.name == name) else {
                continue;
            };
            let enc = find(&self.data.relationships, name, |e| &e.name);
            if enc.is_none() && RelationshipShape::of(r) != Some(RelationshipShape::ManyToMany) {
                continue;
            }
            let ends = (|| {
                let from_table = table_id(&r.from_table)?;
                let to_table = table_id(&r.to_table)?;
                Some(RelationshipEnds {
                    from_table,
                    from_column: self.column_id(from_table, &r.from_column)?,
                    to_table,
                    to_column: self.column_id(to_table, &r.to_column)?,
                })
            })();
            let Some(ends) = ends else {
                self.refuse(format!("relationship '{name}'"), "an end does not resolve");
                continue;
            };
            self.st.add_relationship(self.model_id, r, ends, enc)?;
        }
        for (table, name) in &diff.added_hierarchies {
            let (Some(t), Some(tid)) = (self.model_table(table), table_id(table)) else {
                continue;
            };
            let Some(def) = find(&user_hierarchies(t), name, |h| &h.name).cloned() else {
                continue;
            };
            let Some(enc) = self
                .data
                .hierarchies
                .iter()
                .find(|h| h.table.eq_ignore_ascii_case(table) && h.name.eq_ignore_ascii_case(name))
            else {
                continue;
            };
            let mut columns = Vec::with_capacity(def.levels.len());
            for (_, column) in &def.levels {
                let c = self.column_id(tid, column).ok_or_else(|| {
                    Error::Invalid(format!("hierarchy '{table}'[{name}]: no column `{column}`"))
                })?;
                columns.push(c);
            }
            self.st.add_hierarchy(tid, &def, &columns, enc)?;
        }
        Ok(())
    }

    /// Where a new column of an existing table goes.
    fn home(&self, table_id: i64, locale: i64) -> Result<ColumnHome, Error> {
        let missing = |what: &str| Error::Invalid(format!("table {table_id}: {what} is missing"));
        let parts = self.partitions(table_id);
        if parts.is_empty() {
            return Err(missing("a partition"));
        }
        let mut partitions = Vec::with_capacity(parts.len());
        for (ps, _) in parts {
            let folder = self
                .st
                .reference("PartitionStorage", ps, "StorageFolderID")
                .ok_or_else(|| missing("the partition folder"))?;
            partitions.push((ps, folder));
        }
        let ts = self
            .st
            .reference("Table", table_id, "TableStorageID")
            .ok_or_else(|| missing("TableStorage"))?;
        Ok(ColumnHome {
            table_id,
            table_storage_name: self
                .st
                .text("TableStorage", ts, "Name")
                .ok_or_else(|| missing("the TableStorage name"))?,
            table_folder: self
                .st
                .reference("TableStorage", ts, "StorageFolderID")
                .ok_or_else(|| missing("the table folder"))?,
            partitions,
            system_flags: self.st.int("Table", table_id, "SystemFlags").unwrap_or(0) & 2,
            locale,
        })
    }
}
