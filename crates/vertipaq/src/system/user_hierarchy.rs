//! `U$` user hierarchies: the node table of a multi-level hierarchy ([MS-XLDM] §2.4.4).

use super::{
    AttributeHierarchy, SystemColumn, SystemTableData, SystemTableError, SystemTableKind, biased, check_rows,
};

/// One level of a user hierarchy, top level first.
#[derive(Debug, Clone, Copy)]
pub struct UserHierarchyLevel<'a> {
    /// The level column's data id on each row of the table (all partitions, storage order).
    pub data_ids: &'a [u32],
    /// The level column's attribute hierarchy (gives the sort position of each data id).
    pub hierarchy: &'a AttributeHierarchy,
}

/// A built user hierarchy (`U$` system table plus `HierarchyStorage.LevelDefinition` facts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserHierarchy {
    /// The `U$` table: `MULTI_LEVEL_ID`, `PARENT_POS`, `FIRST_CHILD_POS`, `CHILD_COUNT`.
    pub table: SystemTableData,
    /// First node row of each level.
    pub level_starts: Vec<u32>,
}

impl UserHierarchy {
    /// `HierarchyStorage.LevelDefinition`: `$<name>$<start row>` per level, then `$`, where
    /// `<name>` is the level column's storage name (`<column name> (<column id>)`).
    pub fn level_definition(&self, level_names: &[&str]) -> String {
        let mut out = String::from("$");
        for (name, start) in level_names.iter().zip(&self.level_starts) {
            out.push_str(name);
            out.push('$');
            out.push_str(&start.to_string());
            out.push('$');
        }
        out
    }

    /// Number of nodes (rows of the `U$` table).
    pub fn node_count(&self) -> u64 {
        self.table.record_count()
    }
}

/// Builds the node table of a user hierarchy.
///
/// Nodes are the distinct combinations of level members that occur on some row: level 0
/// first, each level's nodes grouped under their parent in the parent's order and sorted by
/// the attribute-hierarchy position of their own member (blank is a regular member). Stored
/// columns: `MULTI_LEVEL_ID` = the node's data id; `PARENT_POS`, `FIRST_CHILD_POS` and
/// `CHILD_COUNT` hold the parent row, first child row and child count (−1 at the top / leaf
/// level), stored with a +1 bias.
pub fn build_user_hierarchy(levels: &[UserHierarchyLevel<'_>]) -> Result<UserHierarchy, SystemTableError> {
    let first = levels.first().ok_or(SystemTableError::NoLevels)?;
    let rows = first.data_ids.len();
    if levels.iter().any(|l| l.data_ids.len() != rows) {
        return Err(SystemTableError::LevelLengthMismatch);
    }
    let depth = levels.len();
    check_rows(rows.saturating_mul(depth))?;

    // Position tuple of every row, sorted and deduplicated: the leaf nodes in order.
    let mut tuples: Vec<u32> = Vec::with_capacity(rows * depth);
    for r in 0..rows {
        for level in levels {
            let id = level.data_ids.get(r).copied().unwrap_or(0);
            let pos = level
                .hierarchy
                .position_of(id)
                .ok_or(SystemTableError::UnknownMember(id))?;
            tuples.push(pos);
        }
    }
    let mut leaves: Vec<&[u32]> = tuples.chunks(depth).collect();
    leaves.sort_unstable();
    leaves.dedup();

    // Nodes of each level: distinct prefixes, in order.
    let mut nodes: Vec<Vec<&[u32]>> = Vec::with_capacity(depth);
    for l in 0..depth {
        let mut level_nodes: Vec<&[u32]> = leaves.iter().filter_map(|t| t.get(..=l)).collect();
        level_nodes.dedup();
        nodes.push(level_nodes);
    }
    let mut level_starts = Vec::with_capacity(depth);
    let mut total = 0usize;
    for level_nodes in &nodes {
        level_starts.push(u32::try_from(total).map_err(|_| SystemTableError::TooManyRows(total))?);
        total += level_nodes.len();
    }
    check_rows(total)?;

    let mut multi_level_id = Vec::with_capacity(total);
    let mut parent = Vec::with_capacity(total);
    let mut first_child = vec![-1i64; total];
    let mut child_count = vec![-1i64; total];
    for (l, level_nodes) in nodes.iter().enumerate() {
        let start = level_starts.get(l).copied().map_or(0, i64::from);
        let parent_start = l
            .checked_sub(1)
            .and_then(|p| level_starts.get(p))
            .copied()
            .map_or(0, i64::from);
        let mut parent_index = 0usize;
        for (k, node) in level_nodes.iter().enumerate() {
            let pos = node.last().copied().unwrap_or(0);
            let level = levels.get(l).ok_or(SystemTableError::NoLevels)?;
            let id = level
                .hierarchy
                .members
                .get(pos as usize)
                .copied()
                .ok_or(SystemTableError::UnknownMember(pos))?;
            multi_level_id.push(id);
            let row = start + k as i64;
            if l == 0 {
                parent.push(-1);
                continue;
            }
            // Advance to this node's parent (parents are sorted like their children).
            let parents = nodes.get(l - 1).map(Vec::as_slice).unwrap_or(&[]);
            while parents
                .get(parent_index)
                .is_some_and(|p| Some(*p) != node.get(..l))
            {
                parent_index += 1;
            }
            let parent_row = parent_start + parent_index as i64;
            parent.push(parent_row);
            let slot = usize::try_from(parent_row).unwrap_or(usize::MAX);
            if let (Some(fc), Some(cc)) = (first_child.get_mut(slot), child_count.get_mut(slot)) {
                if *fc < 0 {
                    *fc = row;
                    *cc = 0;
                }
                *cc += 1;
            }
        }
    }

    let mut columns = vec![
        SystemColumn::new("MULTI_LEVEL_ID", multi_level_id, 0, 32, 16),
        SystemColumn::new("PARENT_POS", biased(parent)?, -1, 32, 8),
        SystemColumn::new("FIRST_CHILD_POS", biased(first_child)?, -1, 32, 9),
        SystemColumn::new("CHILD_COUNT", biased(child_count)?, -1, 32, 17),
    ];
    for c in &mut columns {
        c.binary_characters = 10240;
    }
    Ok(UserHierarchy {
        table: SystemTableData {
            kind: SystemTableKind::UserHierarchy,
            columns,
            records_per_segment: (total as u64).max(1),
        },
        level_starts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CellValue;
    use crate::system::{Collation, build_attribute_hierarchy};

    #[test]
    fn spec_example() {
        // [MS-XLDM] §2.4.4: Country / Area.
        let text = |s: &str| CellValue::Text(s.into());
        let country = build_attribute_hierarchy(
            &[(3, text("United States")), (4, text("Canada"))],
            &Collation::default(),
        )
        .unwrap();
        let area = build_attribute_hierarchy(
            &[
                (3, text("Northwest")),
                (4, text("Northeast")),
                (5, text("Southwest")),
                (6, text("Southeast")),
                (7, text("East")),
                (8, text("West")),
            ],
            &Collation::default(),
        )
        .unwrap();
        let c_ids = [3, 3, 3, 3, 4, 4, 3];
        let a_ids = [3, 4, 5, 6, 7, 8, 3];
        let u = build_user_hierarchy(&[
            UserHierarchyLevel {
                data_ids: &c_ids,
                hierarchy: &country,
            },
            UserHierarchyLevel {
                data_ids: &a_ids,
                hierarchy: &area,
            },
        ])
        .unwrap();
        let col = |n: &str| u.table.column(n).unwrap().logical_values();
        assert_eq!(col("MULTI_LEVEL_ID"), [4, 3, 7, 8, 4, 3, 6, 5]);
        assert_eq!(col("PARENT_POS"), [-1, -1, 0, 0, 1, 1, 1, 1]);
        assert_eq!(col("FIRST_CHILD_POS"), [2, 4, -1, -1, -1, -1, -1, -1]);
        assert_eq!(col("CHILD_COUNT"), [2, 4, -1, -1, -1, -1, -1, -1]);
        assert_eq!(u.level_starts, [0, 2]);
        assert_eq!(u.level_definition(&["C (1)", "A (2)"]), "$C (1)$0$A (2)$2$");
    }
}
