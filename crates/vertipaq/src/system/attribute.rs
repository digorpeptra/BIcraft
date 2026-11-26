//! `H$` attribute hierarchies: the sort order of a column's distinct values.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use super::{
    BLANK_DATA_ID, Collation, SystemColumn, SystemTableData, SystemTableError, SystemTableKind, check_rows,
};
use crate::CellValue;

/// Options of [`build_attribute_hierarchy_with`].
#[derive(Debug, Clone, Copy)]
pub struct AttributeHierarchyOptions<'a> {
    /// Sort-by column (`Column.SortByColumnID`): the sort value of each data id of the column
    /// (the value the sort-by column holds on the rows carrying that id). Members without an
    /// entry sort as blank. `None` sorts by the column's own values.
    pub sort_by: Option<&'a [(u32, CellValue)]>,
    /// Emit `ID_TO_POS` (hash-encoded columns, `MaterializationType` 0). Value-encoded columns
    /// only get `POS_TO_ID` (`MaterializationType` 1, plus a `.hidx` the caller writes).
    pub id_to_pos: bool,
}

impl Default for AttributeHierarchyOptions<'_> {
    fn default() -> Self {
        AttributeHierarchyOptions {
            sort_by: None,
            id_to_pos: true,
        }
    }
}

/// A built attribute hierarchy (`H$` system table plus the `AttributeHierarchyStorage` facts).
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeHierarchy {
    /// Member data ids in sort order: `members[p]` is the data id at position `p`. The blank
    /// member (data id 2) is included when the column has blanks.
    pub members: Vec<u32>,
    /// The `H$` table: `POS_TO_ID` and, if requested, `ID_TO_POS`.
    pub table: SystemTableData,
    /// `AttributeHierarchyStorage.MaterializationType`: 0 with `ID_TO_POS`, 1 without.
    pub materialization_type: i64,
    /// `AttributeHierarchyStorage.DistinctDataCount`: the number of members (blank included).
    pub distinct_data_count: u64,
    /// `MinValue`: the first non-blank member in sort order (`None` if there is none).
    pub min_value: Option<CellValue>,
    /// `MaxValue`: the last non-blank member in sort order.
    pub max_value: Option<CellValue>,
    /// `StringValueMaxLength`: longest string member in UTF-16 units (0 for other types).
    pub string_value_max_length: u64,
    positions: HashMap<u32, u32>,
}

impl AttributeHierarchy {
    /// Sort position of a data id (`None` if it is not a member).
    pub fn position_of(&self, data_id: u32) -> Option<u32> {
        self.positions.get(&data_id).copied()
    }

    /// The member order of a stored attribute hierarchy: the first `DistinctDataCount` values
    /// of its `H$` table's `POS_TO_ID` column (what [`super::build_user_hierarchy`] needs for a
    /// level). The `H$` table is rebuilt from `members` as written; the statistics
    /// (`min_value`, …) are left empty.
    pub fn from_members(members: Vec<u32>) -> Result<Self, SystemTableError> {
        check_rows(members.len())?;
        let mut positions = HashMap::with_capacity(members.len());
        for (p, id) in members.iter().enumerate() {
            if positions
                .insert(*id, u32::try_from(p).unwrap_or(u32::MAX))
                .is_some()
            {
                return Err(SystemTableError::DuplicateDataId(*id));
            }
        }
        Ok(AttributeHierarchy {
            table: SystemTableData {
                kind: SystemTableKind::AttributeHierarchy,
                columns: vec![SystemColumn::new("POS_TO_ID", members.clone(), 0, 32, 7)],
                records_per_segment: (members.len() as u64).max(1),
            },
            materialization_type: 0,
            distinct_data_count: members.len() as u64,
            members,
            min_value: None,
            max_value: None,
            string_value_max_length: 0,
            positions,
        })
    }
}

/// Builds the attribute hierarchy of a hash-encoded column sorted by its own values.
///
/// `entries` holds every distinct data id of the column with its value: blank is data id 2
/// with [`CellValue::Null`]. Equivalent shorthand for [`build_attribute_hierarchy_with`] with
/// default options.
pub fn build_attribute_hierarchy(
    entries: &[(u32, CellValue)],
    collation: &Collation,
) -> Result<AttributeHierarchy, SystemTableError> {
    build_attribute_hierarchy_with(entries, collation, &AttributeHierarchyOptions::default())
}

/// Builds an attribute hierarchy (see [`AttributeHierarchyOptions`]).
pub fn build_attribute_hierarchy_with(
    entries: &[(u32, CellValue)],
    collation: &Collation,
    options: &AttributeHierarchyOptions<'_>,
) -> Result<AttributeHierarchy, SystemTableError> {
    check_rows(entries.len())?;
    let mut seen = HashSet::with_capacity(entries.len());
    let mut max_id = BLANK_DATA_ID;
    let mut has_blank = false;
    for (id, value) in entries {
        if *id < BLANK_DATA_ID {
            return Err(SystemTableError::ReservedDataId(*id));
        }
        if (*id == BLANK_DATA_ID) != matches!(value, CellValue::Null) {
            return Err(SystemTableError::BlankMismatch(*id));
        }
        if !seen.insert(*id) {
            return Err(SystemTableError::DuplicateDataId(*id));
        }
        has_blank |= *id == BLANK_DATA_ID;
        max_id = max_id.max(*id);
    }
    let sort_keys: Option<HashMap<u32, &CellValue>> = options
        .sort_by
        .map(|keys| keys.iter().map(|(id, v)| (*id, v)).collect());

    let mut order: Vec<&(u32, CellValue)> = entries.iter().collect();
    order.sort_by(|a, b| {
        let by_key = match &sort_keys {
            Some(keys) => {
                let ka = keys.get(&a.0).copied().unwrap_or(&CellValue::Null);
                let kb = keys.get(&b.0).copied().unwrap_or(&CellValue::Null);
                compare_values(ka, kb, collation)
            }
            None => Ordering::Equal,
        };
        by_key
            .then_with(|| compare_values(&a.1, &b.1, collation))
            .then_with(|| a.0.cmp(&b.0))
    });
    let members: Vec<u32> = order.iter().map(|(id, _)| *id).collect();

    let non_blank = members.len() - usize::from(has_blank);
    let stored_len = non_blank + 3;
    let mut pos_to_id = members.clone();
    pos_to_id.resize(stored_len.max(members.len()), 0);

    let mut positions = HashMap::with_capacity(members.len());
    for (p, id) in members.iter().enumerate() {
        positions.insert(*id, u32::try_from(p).unwrap_or(u32::MAX));
    }
    let member_count = u32::try_from(members.len()).unwrap_or(u32::MAX);

    let mut columns = vec![SystemColumn::new("POS_TO_ID", pos_to_id, 0, 32, 7)];
    if options.id_to_pos {
        let len = usize::try_from(max_id)
            .ok()
            .and_then(|m| m.checked_add(1))
            .ok_or(SystemTableError::TooManyRows(usize::MAX))?;
        check_rows(len)?;
        let mut id_to_pos = vec![0u32; len];
        if let Some(slot) = id_to_pos.get_mut(1) {
            *slot = member_count;
        }
        for (id, p) in &positions {
            if let Some(slot) = id_to_pos.get_mut(*id as usize) {
                *slot = *p;
            }
        }
        columns.push(SystemColumn::new("ID_TO_POS", id_to_pos, 0, 32, 5));
    }

    let mut non_blank_members = order.iter().filter(|(id, _)| *id != BLANK_DATA_ID);
    let min_value = non_blank_members.clone().next().map(|(_, v)| v.clone());
    let max_value = non_blank_members.next_back().map(|(_, v)| v.clone());
    let string_value_max_length = entries
        .iter()
        .filter_map(|(_, v)| match v {
            CellValue::Text(t) => Some(t.encode_utf16().count() as u64),
            _ => None,
        })
        .max()
        .unwrap_or(0);

    Ok(AttributeHierarchy {
        table: SystemTableData {
            kind: SystemTableKind::AttributeHierarchy,
            columns,
            records_per_segment: (members.len() as u64).max(1),
        },
        materialization_type: if options.id_to_pos { 0 } else { 1 },
        distinct_data_count: members.len() as u64,
        members,
        min_value,
        max_value,
        string_value_max_length,
        positions,
    })
}

/// Sort order of two cell values in an attribute hierarchy: blank first, then numbers (and
/// dates) by value, strings by `collation`, booleans `false` < `true`.
pub fn compare_values(a: &CellValue, b: &CellValue, collation: &Collation) -> Ordering {
    fn rank(v: &CellValue) -> u8 {
        match v {
            CellValue::Null => 0,
            CellValue::Int(_) | CellValue::Real(_) | CellValue::DateTime(_) | CellValue::Decimal(_) => 1,
            CellValue::Text(_) => 2,
            CellValue::Bool(_) => 3,
        }
    }
    match (a, b) {
        (CellValue::Int(x), CellValue::Int(y)) | (CellValue::Decimal(x), CellValue::Decimal(y)) => x.cmp(y),
        (CellValue::Text(x), CellValue::Text(y)) => collation.compare(x, y),
        (CellValue::Bool(x), CellValue::Bool(y)) => x.cmp(y),
        _ => match (number(a), number(b)) {
            (Some(x), Some(y)) => x.total_cmp(&y),
            _ => rank(a).cmp(&rank(b)),
        },
    }
}

fn number(v: &CellValue) -> Option<f64> {
    match v {
        CellValue::Int(n) => Some(*n as f64),
        CellValue::Real(r) | CellValue::DateTime(r) => Some(if *r == 0.0 { 0.0 } else { *r }),
        CellValue::Decimal(d) => Some(*d as f64 / 10_000.0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> CellValue {
        CellValue::Text(s.to_string())
    }

    #[test]
    fn layout_without_blank() {
        let entries = [
            (3, CellValue::Int(30)),
            (4, CellValue::Int(10)),
            (5, CellValue::Int(20)),
        ];
        let h = build_attribute_hierarchy(&entries, &Collation::default()).unwrap();
        assert_eq!(h.members, [4, 5, 3]);
        assert_eq!(h.table.columns[0].values, [4, 5, 3, 0, 0, 0]);
        assert_eq!(h.table.columns[1].values, [0, 3, 0, 2, 0, 1]);
        assert_eq!(h.table.records_per_segment, 3);
        assert_eq!(h.min_value, Some(CellValue::Int(10)));
        assert_eq!(h.max_value, Some(CellValue::Int(30)));
    }

    #[test]
    fn blank_sorts_first_and_takes_a_reserved_slot() {
        let entries = [(3, text("b")), (2, CellValue::Null), (4, text("a"))];
        let h = build_attribute_hierarchy(&entries, &Collation::default()).unwrap();
        assert_eq!(h.members, [2, 4, 3]);
        assert_eq!(h.table.columns[0].values, [2, 4, 3, 0, 0]);
        assert_eq!(h.table.columns[1].values, [0, 3, 0, 2, 1]);
        assert_eq!(h.distinct_data_count, 3);
        assert_eq!(h.string_value_max_length, 1);
    }

    #[test]
    fn rejects_bad_ids() {
        let c = Collation::default();
        assert_eq!(
            build_attribute_hierarchy(&[(1, CellValue::Int(1))], &c),
            Err(SystemTableError::ReservedDataId(1))
        );
        assert_eq!(
            build_attribute_hierarchy(&[(3, CellValue::Int(1)), (3, CellValue::Int(2))], &c),
            Err(SystemTableError::DuplicateDataId(3))
        );
        assert_eq!(
            build_attribute_hierarchy(&[(2, CellValue::Int(1))], &c),
            Err(SystemTableError::BlankMismatch(2))
        );
    }
}
