//! Cascading removal: an object and every row and ABF file that exists only because of it.
//!
//! The ownership graph (observed, see `docs/formats/metadata-sqlitedb.md`):
//!
//! ```text
//! Table ─ TableStorage ─ StorageFolder (.tbl) ─ StorageFile*
//!   ├─ Partition ─ PartitionStorage ─ SegmentMapStorage, StorageFolder (.prt) ─ StorageFile*
//!   ├─ Column ─ ColumnStorage ─ DictionaryStorage (─ .dictionary)
//!   │    │                   └─ ColumnPartitionStorage (─ .idf) ─ SegmentStorage (─ .idfmeta)
//!   │    ├─ RelatedColumnDetails ─ GroupByColumn*
//!   │    └─ AttributeHierarchy ─ AttributeHierarchyStorage (─ .hidx) ─ H$ system table
//!   ├─ Measure (─ KPI, DetailRowsDefinition, FormatStringDefinition)
//!   └─ Hierarchy ─ Level, HierarchyStorage ─ U$ system table
//! Relationship ─ RelationshipStorage ─ RelationshipIndexStorage ─ R$ system table
//! ```
//!
//! Plus the rows keyed by (`ObjectType`, `ObjectID`): annotations, extended properties, changed
//! properties, translations, excluded artifacts.

use crate::Error;
use crate::codes::object_type as ot;
use crate::structural::store::Store;

/// `ObjectType` of an attribute hierarchy ([MS-SSAS-T]).
const OT_ATTRIBUTE_HIERARCHY: i64 = 5;
/// `ObjectType` of a KPI.
const OT_KPI: i64 = 12;

impl Store {
    /// Removes a column (user or system) with its storage, attribute hierarchy and `H$` table.
    pub(crate) fn drop_column(&mut self, column_id: i64) -> Result<(), Error> {
        if let Some(cs) = self.reference("Column", column_id, "ColumnStorageID") {
            self.drop_column_storage(cs);
        }
        for cs in self.ids_where("ColumnStorage", "ColumnID", column_id) {
            self.drop_column_storage(cs);
        }
        let mut hierarchies = self.ids_where("AttributeHierarchy", "ColumnID", column_id);
        if let Some(ah) = self.reference("Column", column_id, "AttributeHierarchyID")
            && !hierarchies.contains(&ah)
        {
            hierarchies.push(ah);
        }
        for ah in hierarchies {
            self.drop_attribute_hierarchy(ah)?;
        }
        for table in ["PerspectiveColumn", "ColumnPermission", "Variation"] {
            for id in self.ids_where(table, "ColumnID", column_id) {
                self.drop_dependents(variation_kind(table), id);
                self.delete(table, id);
            }
        }
        // Its related column details (a field parameter's grouping) and their group-by columns.
        let mut details = self.ids_where("RelatedColumnDetails", "ColumnID", column_id);
        if let Some(d) = self.reference("Column", column_id, "RelatedColumnDetailsID")
            && !details.contains(&d)
        {
            details.push(d);
        }
        for d in details {
            for g in self.ids_where("GroupByColumn", "RelatedColumnDetailsID", d) {
                self.drop_dependents(ot::GROUP_BY_COLUMN, g);
                self.delete("GroupByColumn", g);
            }
            self.drop_dependents(ot::RELATED_COLUMN_DETAILS, d);
            self.delete("RelatedColumnDetails", d);
        }
        self.drop_dependents(ot::COLUMN, column_id);
        self.delete("Column", column_id);
        Ok(())
    }

    fn drop_column_storage(&mut self, cs: i64) {
        let mut dictionaries = self.ids_where("DictionaryStorage", "ColumnStorageID", cs);
        if let Some(d) = self.reference("ColumnStorage", cs, "DictionaryStorageID")
            && !dictionaries.contains(&d)
        {
            dictionaries.push(d);
        }
        for d in dictionaries {
            self.drop_file_of("DictionaryStorage", d, "StorageFileID");
            self.drop_files_owned_by(d);
            self.delete("DictionaryStorage", d);
        }
        for cps in self.ids_where("ColumnPartitionStorage", "ColumnStorageID", cs) {
            self.drop_column_partition_storage(cps);
        }
        for (table, col) in [
            ("ColumnIndexStorage", "ColumnIndexStorageID"),
            ("StringIndexStorage", "StringIndexStorageID"),
            ("FullTextIndexStorage", "FullTextIndexStorageID"),
            ("DeltaTableColumnStorage", "DeltaTableColumnStorageID"),
        ] {
            let mut ids = self.ids_where(table, "ColumnStorageID", cs);
            if let Some(i) = self.reference("ColumnStorage", cs, col)
                && !ids.contains(&i)
            {
                ids.push(i);
            }
            for i in ids {
                self.drop_file_of(table, i, "StorageFileID");
                self.drop_files_owned_by(i);
                self.delete(table, i);
            }
        }
        self.drop_files_owned_by(cs);
        self.delete("ColumnStorage", cs);
    }

    fn drop_column_partition_storage(&mut self, cps: i64) {
        self.drop_file_of("ColumnPartitionStorage", cps, "StorageFileID");
        let mut segments = self.ids_where("SegmentStorage", "ColumnPartitionStorageID", cps);
        if let Some(s) = self.reference("ColumnPartitionStorage", cps, "SegmentStorageID")
            && !segments.contains(&s)
        {
            segments.push(s);
        }
        for s in segments {
            self.drop_file_of("SegmentStorage", s, "StorageFileID");
            self.drop_files_owned_by(s);
            self.delete("SegmentStorage", s);
        }
        self.drop_files_owned_by(cps);
        self.delete("ColumnPartitionStorage", cps);
    }

    fn drop_attribute_hierarchy(&mut self, ah: i64) -> Result<(), Error> {
        let mut storages = self.ids_where("AttributeHierarchyStorage", "AttributeHierarchyID", ah);
        if let Some(s) = self.reference("AttributeHierarchy", ah, "AttributeHierarchyStorageID")
            && !storages.contains(&s)
        {
            storages.push(s);
        }
        for s in storages {
            self.drop_file_of("AttributeHierarchyStorage", s, "StorageFileID");
            self.drop_files_owned_by(s);
            if let Some(sys) = self.reference("AttributeHierarchyStorage", s, "SystemTableID") {
                self.drop_system_table(sys)?;
            }
            self.delete("AttributeHierarchyStorage", s);
        }
        self.drop_dependents(OT_ATTRIBUTE_HIERARCHY, ah);
        self.delete("AttributeHierarchy", ah);
        Ok(())
    }

    /// Removes an engine system table (`H$`, `R$`, `U$`). Refuses anything that is not one.
    pub(crate) fn drop_system_table(&mut self, table_id: i64) -> Result<(), Error> {
        let flags = self.int("Table", table_id, "SystemFlags").unwrap_or(0);
        if flags & 1 == 0 {
            // A dangling or wrong reference: never cascade into a user table.
            return if self.row("Table", table_id).is_none() {
                Ok(())
            } else {
                Err(Error::Invalid(format!(
                    "storage row points at table {table_id}, which is not a system table"
                )))
            };
        }
        self.drop_table_rows(table_id)
    }

    /// Removes a table's columns, partitions, storage and the table row (no user-level objects).
    fn drop_table_rows(&mut self, table_id: i64) -> Result<(), Error> {
        for c in self.ids_where("Column", "TableID", table_id) {
            self.drop_column(c)?;
        }
        for p in self.ids_where("Partition", "TableID", table_id) {
            self.drop_partition(p);
        }
        let mut storages = self.ids_where("TableStorage", "TableID", table_id);
        if let Some(ts) = self.reference("Table", table_id, "TableStorageID")
            && !storages.contains(&ts)
        {
            storages.push(ts);
        }
        for ts in storages {
            if let Some(folder) = self.reference("TableStorage", ts, "StorageFolderID") {
                self.drop_folder(folder);
            }
            for folder in self.ids_matching("StorageFolder", |r| {
                r.int("OwnerID") == Some(ts) && r.int("OwnerType") == Some(18)
            }) {
                self.drop_folder(folder);
            }
            self.delete("TableStorage", ts);
        }
        self.drop_dependents(ot::TABLE, table_id);
        self.delete("Table", table_id);
        Ok(())
    }

    fn drop_partition(&mut self, partition_id: i64) {
        let mut storages = self.ids_where("PartitionStorage", "PartitionID", partition_id);
        if let Some(ps) = self.reference("Partition", partition_id, "PartitionStorageID")
            && !storages.contains(&ps)
        {
            storages.push(ps);
        }
        for ps in storages {
            for cps in self.ids_where("ColumnPartitionStorage", "PartitionStorageID", ps) {
                self.drop_column_partition_storage(cps);
            }
            let mut maps = self.ids_where("SegmentMapStorage", "PartitionStorageID", ps);
            if let Some(m) = self.reference("PartitionStorage", ps, "SegmentMapStorageID")
                && !maps.contains(&m)
            {
                maps.push(m);
            }
            for m in maps {
                for g in self.ids_where(
                    "GeneralSegmentMapSegmentMetadataStorage",
                    "SegmentMapStorageID",
                    m,
                ) {
                    self.delete("GeneralSegmentMapSegmentMetadataStorage", g);
                }
                self.delete("SegmentMapStorage", m);
            }
            if let Some(folder) = self.reference("PartitionStorage", ps, "StorageFolderID") {
                self.drop_folder(folder);
            }
            for d in self.ids_where("DeltaTableMetadataStorage", "PartitionStorageID", ps) {
                for p in self.ids_where("ParquetFileStorage", "DeltaTableMetadataStorageID", d) {
                    self.delete("ParquetFileStorage", p);
                }
                self.delete("DeltaTableMetadataStorage", d);
            }
            self.delete("PartitionStorage", ps);
        }
        for d in self.ids_where("DataCoverageDefinition", "PartitionID", partition_id) {
            self.delete("DataCoverageDefinition", d);
        }
        self.drop_dependents(ot::PARTITION, partition_id);
        self.delete("Partition", partition_id);
    }

    /// Removes a user table and everything that belongs to it: columns, partitions, measures
    /// (with their KPIs, detail rows and format string definitions), hierarchies (with their
    /// `U$` tables), permissions, perspective entries, refresh policy.
    pub(crate) fn drop_user_table(&mut self, table_id: i64) -> Result<(), Error> {
        for m in self.ids_where("Measure", "TableID", table_id) {
            self.drop_measure(m);
        }
        for h in self.ids_where("Hierarchy", "TableID", table_id) {
            self.drop_hierarchy(h)?;
        }
        for tp in self.ids_where("TablePermission", "TableID", table_id) {
            for cp in self.ids_where("ColumnPermission", "TablePermissionID", tp) {
                self.delete("ColumnPermission", cp);
            }
            self.drop_dependents(ot::TABLE_PERMISSION, tp);
            self.delete("TablePermission", tp);
        }
        for pt in self.ids_where("PerspectiveTable", "TableID", table_id) {
            for child in [
                "PerspectiveColumn",
                "PerspectiveMeasure",
                "PerspectiveHierarchy",
                "PerspectiveSet",
            ] {
                for id in self.ids_where(child, "PerspectiveTableID", pt) {
                    self.delete(child, id);
                }
            }
            self.delete("PerspectiveTable", pt);
        }
        // Sets: perspective entries elsewhere first, then the sets with their dependents.
        for set in self.ids_where("Set", "TableID", table_id) {
            for ps in self.ids_where("PerspectiveSet", "SetID", set) {
                self.drop_dependents(ot::PERSPECTIVE_SET, ps);
                self.delete("PerspectiveSet", ps);
            }
            self.drop_dependents(ot::SET, set);
            self.delete("Set", set);
        }
        for rp in self.ids_where("RefreshPolicy", "TableID", table_id) {
            self.delete("RefreshPolicy", rp);
        }
        // A calculation group with its items and selection expressions (and their format
        // string definitions).
        let mut groups = self.ids_where("CalculationGroup", "TableID", table_id);
        if let Some(g) = self.reference("Table", table_id, "CalculationGroupID")
            && !groups.contains(&g)
        {
            groups.push(g);
        }
        for g in groups {
            for (child, kind) in [
                ("CalculationItem", ot::CALCULATION_ITEM),
                ("CalculationExpression", ot::CALCULATION_EXPRESSION),
            ] {
                for i in self.ids_where(child, "CalculationGroupID", g) {
                    let mut formats = self.ids_matching("FormatStringDefinition", |r| {
                        r.int("ObjectType") == Some(kind) && r.int("ObjectID") == Some(i)
                    });
                    if let Some(f) = self.reference(child, i, "FormatStringDefinitionID")
                        && !formats.contains(&f)
                    {
                        formats.push(f);
                    }
                    for f in formats {
                        self.drop_dependents(ot::FORMAT_STRING_DEFINITION, f);
                        self.delete("FormatStringDefinition", f);
                    }
                    self.drop_dependents(kind, i);
                    self.delete(child, i);
                }
            }
            self.drop_dependents(ot::CALCULATION_GROUP, g);
            self.delete("CalculationGroup", g);
        }
        if let Some(d) = self.reference("Table", table_id, "DefaultDetailRowsDefinitionID") {
            self.delete("DetailRowsDefinition", d);
        }
        for d in self.ids_matching("DetailRowsDefinition", |r| {
            r.int("ObjectType") == Some(ot::TABLE) && r.int("ObjectID") == Some(table_id)
        }) {
            self.delete("DetailRowsDefinition", d);
        }
        self.drop_table_rows(table_id)
    }

    fn drop_measure(&mut self, measure_id: i64) {
        let mut kpis = self.ids_where("KPI", "MeasureID", measure_id);
        if let Some(k) = self.reference("Measure", measure_id, "KPIID")
            && !kpis.contains(&k)
        {
            kpis.push(k);
        }
        for k in kpis {
            self.drop_dependents(OT_KPI, k);
            self.delete("KPI", k);
        }
        for (table, col) in [
            ("DetailRowsDefinition", "DetailRowsDefinitionID"),
            ("FormatStringDefinition", "FormatStringDefinitionID"),
        ] {
            if let Some(d) = self.reference("Measure", measure_id, col) {
                self.delete(table, d);
            }
            for d in self.ids_matching(table, |r| {
                r.int("ObjectType") == Some(ot::MEASURE) && r.int("ObjectID") == Some(measure_id)
            }) {
                self.delete(table, d);
            }
        }
        for pm in self.ids_where("PerspectiveMeasure", "MeasureID", measure_id) {
            self.delete("PerspectiveMeasure", pm);
        }
        self.drop_dependents(ot::MEASURE, measure_id);
        self.delete("Measure", measure_id);
    }

    pub(crate) fn drop_hierarchy(&mut self, hierarchy_id: i64) -> Result<(), Error> {
        for l in self.ids_where("Level", "HierarchyID", hierarchy_id) {
            self.drop_dependents(ot::LEVEL, l);
            self.delete("Level", l);
        }
        let mut storages = self.ids_where("HierarchyStorage", "HierarchyID", hierarchy_id);
        if let Some(s) = self.reference("Hierarchy", hierarchy_id, "HierarchyStorageID")
            && !storages.contains(&s)
        {
            storages.push(s);
        }
        for s in storages {
            if let Some(sys) = self.reference("HierarchyStorage", s, "SystemTableID") {
                self.drop_system_table(sys)?;
            }
            self.delete("HierarchyStorage", s);
        }
        for p in self.ids_where("PerspectiveHierarchy", "HierarchyID", hierarchy_id) {
            self.delete("PerspectiveHierarchy", p);
        }
        self.drop_dependents(ot::HIERARCHY, hierarchy_id);
        self.delete("Hierarchy", hierarchy_id);
        Ok(())
    }

    /// Removes a relationship with its storage rows and `R$` tables.
    pub(crate) fn drop_relationship(&mut self, relationship_id: i64) -> Result<(), Error> {
        let mut storages = self.ids_where("RelationshipStorage", "RelationshipID", relationship_id);
        for col in ["RelationshipStorageID", "RelationshipStorage2ID"] {
            if let Some(s) = self.reference("Relationship", relationship_id, col)
                && !storages.contains(&s)
            {
                storages.push(s);
            }
        }
        for s in storages {
            let mut indexes = self.ids_where("RelationshipIndexStorage", "RelationshipStorageID", s);
            if let Some(i) = self.reference("RelationshipStorage", s, "RelationshipIndexStorageID")
                && !indexes.contains(&i)
            {
                indexes.push(i);
            }
            for i in indexes {
                self.drop_file_of("RelationshipIndexStorage", i, "StorageFileID");
                if let Some(folder) = self.reference("RelationshipIndexStorage", i, "StorageFolderID") {
                    self.drop_folder(folder);
                }
                for col in ["SystemTableID", "SecondarySystemTableID"] {
                    if let Some(sys) = self.reference("RelationshipIndexStorage", i, col) {
                        self.drop_system_table(sys)?;
                    }
                }
                self.drop_files_owned_by(i);
                self.delete("RelationshipIndexStorage", i);
            }
            self.delete("RelationshipStorage", s);
        }
        self.drop_dependents(ot::RELATIONSHIP, relationship_id);
        self.delete("Relationship", relationship_id);
        Ok(())
    }

    /// Renumbers `ColumnStorage.StoragePosition` of a table's columns to 0, 1, … in their
    /// current order (after a removal left a gap).
    pub(crate) fn compact_column_positions(&mut self, table_id: i64) {
        let mut storages: Vec<(i64, i64)> = self
            .ids_where("Column", "TableID", table_id)
            .into_iter()
            .filter_map(|c| self.reference("Column", c, "ColumnStorageID"))
            .filter_map(|cs| Some((self.int("ColumnStorage", cs, "StoragePosition")?, cs)))
            .collect();
        storages.sort_unstable();
        for (new, (old, cs)) in storages.into_iter().enumerate() {
            let new = new as i64;
            if new != old {
                self.set("ColumnStorage", cs, "StoragePosition", new.into());
            }
        }
    }
}

/// `ObjectType` of the rows deleted alongside a column.
fn variation_kind(table: &str) -> i64 {
    match table {
        "PerspectiveColumn" => 31,
        "ColumnPermission" => 42,
        _ => 37,
    }
}
