//! User hierarchies added or removed by a structural edit: the `Hierarchy`, `Level` and
//! `HierarchyStorage` rows and the `U$` node table (engine-observed, Desktop 2.158 engine;
//! `docs/formats/metadata-sqlitedb.md`, "Rows written for a new hierarchy").

use bicraft_model::Table;
use serde_json::Value;

use crate::Error;
use crate::structural::add::SystemKind;
use crate::structural::data::EncodedHierarchy;
use crate::structural::names;
use crate::structural::store::Store;

/// A user hierarchy of a model table, as its structure is stored: levels by ordinal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserHierarchyDef {
    pub name: String,
    /// (level name, column name), top level first.
    pub levels: Vec<(String, String)>,
}

/// The user hierarchies of a table (`extra.hierarchies`), levels sorted by `ordinal` (then
/// written order). Entries without a name are skipped.
pub fn user_hierarchies(t: &Table) -> Vec<UserHierarchyDef> {
    let mut out = Vec::new();
    for h in t
        .extra
        .get("hierarchies")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = h.get("name").and_then(Value::as_str) else {
            continue;
        };
        let mut levels: Vec<(i64, usize, String, String)> = h
            .get("levels")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, l)| {
                let text = |k: &str| l.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
                let ordinal = l.get("ordinal").and_then(Value::as_i64).unwrap_or(i as i64);
                (ordinal, i, text("name"), text("column"))
            })
            .collect();
        levels.sort_by_key(|l| (l.0, l.1));
        out.push(UserHierarchyDef {
            name: name.to_string(),
            levels: levels.into_iter().map(|(_, _, n, c)| (n, c)).collect(),
        });
    }
    out
}

impl Store {
    /// Adds a user hierarchy of table `table_id`: `Hierarchy` (display properties default; the
    /// metadata pass writes the model's), `Level` rows (ordinal 0, 1, …), `HierarchyStorage`
    /// (`LevelDefinition` from the level columns' storage names) and the `U$` table.
    /// `level_columns`: the column ID of each level, top level first.
    pub(crate) fn add_hierarchy(
        &mut self,
        table_id: i64,
        def: &UserHierarchyDef,
        level_columns: &[i64],
        enc: &EncodedHierarchy,
    ) -> Result<i64, Error> {
        let invalid = |what: &str| Error::Invalid(format!("hierarchy `{}`: {what}", def.name));
        if level_columns.len() != def.levels.len() || enc.level_starts.len() != def.levels.len() {
            return Err(invalid("levels and encoded nodes disagree"));
        }
        let now = self.now;
        let hierarchy_id = self.alloc()?;
        let storage_id = self.alloc()?;
        self.insert(
            "Hierarchy",
            vec![
                ("ID", hierarchy_id.into()),
                ("TableID", table_id.into()),
                ("Name", def.name.as_str().into()),
                ("Description", crate::input::MetaValue::Null),
                ("IsHidden", 0.into()),
                ("State", 1.into()),
                ("HierarchyStorageID", storage_id.into()),
                ("DisplayFolder", crate::input::MetaValue::Null),
                ("HideMembers", 0.into()),
                ("ModifiedTime", now.into()),
                ("StructureModifiedTime", now.into()),
                ("RefreshedTime", now.into()),
                ("LineageTag", crate::input::MetaValue::Null),
                ("SourceLineageTag", crate::input::MetaValue::Null),
            ],
        );
        let mut definition = String::from("$");
        for (ordinal, ((level, _), (&column, start))) in def
            .levels
            .iter()
            .zip(level_columns.iter().zip(&enc.level_starts))
            .enumerate()
        {
            let id = self.alloc()?;
            self.insert(
                "Level",
                vec![
                    ("ID", id.into()),
                    ("HierarchyID", hierarchy_id.into()),
                    ("Ordinal", (ordinal as i64).into()),
                    ("Name", level.as_str().into()),
                    ("Description", crate::input::MetaValue::Null),
                    ("ColumnID", column.into()),
                    ("ModifiedTime", now.into()),
                    ("LineageTag", crate::input::MetaValue::Null),
                    ("SourceLineageTag", crate::input::MetaValue::Null),
                ],
            );
            let storage_name = self
                .reference("Column", column, "ColumnStorageID")
                .and_then(|cs| self.text("ColumnStorage", cs, "Name"))
                .ok_or_else(|| invalid("a level column has no storage"))?;
            definition.push_str(&storage_name);
            definition.push('$');
            definition.push_str(&start.to_string());
            definition.push('$');
        }
        let table_storage = self
            .reference("Table", table_id, "TableStorageID")
            .and_then(|ts| self.text("TableStorage", ts, "Name"))
            .ok_or_else(|| invalid("no table storage"))?;
        let storage_name = names::object_storage_name(&def.name, hierarchy_id);
        let system = self.add_system_table(
            SystemKind::UserHierarchy,
            &names::user_hierarchy_table(&table_storage, &storage_name),
            &enc.nodes,
        )?;
        self.insert(
            "HierarchyStorage",
            vec![
                ("ID", storage_id.into()),
                ("HierarchyID", hierarchy_id.into()),
                ("Name", storage_name.as_str().into()),
                ("MaterializationType", 0.into()),
                ("StructureType", 2.into()),
                ("SystemTableID", system.into()),
                ("LevelDefinition", definition.into()),
            ],
        );
        self.touch_table(table_id);
        Ok(hierarchy_id)
    }
}
