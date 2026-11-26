//! `R$` relationship indexes: from-column data id → row of the to-table.

use std::collections::HashMap;

use super::collation::{EquivalenceKey, TextEquivalence};
use super::{
    BLANK_DATA_ID, SystemColumn, SystemTableData, SystemTableError, SystemTableKind, biased, check_rows,
};
use crate::CellValue;
use crate::bitpack;

/// A built relationship index (`R$` system table plus the `RelationshipIndexStorage` facts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationshipIndex {
    /// The `R$` table: one `INDEX` column.
    pub table: SystemTableData,
    /// Logical index: for each data id of the from column (0 ..= last id), the 0-based row of
    /// the to-table with the same key, or −1 (ids 0–2, blank, and keys missing on the to side).
    pub index: Vec<i64>,
    /// Non-blank from-column values that have no matching to-row (referential-integrity
    /// violations; `RelationshipIndexStorage.Flags` was 1 when this is non-zero, 0 otherwise).
    pub unmatched: u64,
}

impl RelationshipIndex {
    /// `RelationshipIndexStorage.RecordCount`: entries of the index.
    pub fn record_count(&self) -> u64 {
        self.index.len() as u64
    }
}

/// Builds the relationship index of a relationship `from` → `to`.
///
/// - `from`: every distinct data id of the from column (the "many" side, or either side of a
///   one-to-one relationship) with its value; blank is data id 2.
/// - `to_rows`: the to column's value on each row of the to-table, in storage row order
///   (partitions concatenated).
///
/// Keys match when the values are equal: numbers by value, strings when `text` gives them the
/// same key. Analysis Services matches the stored text of both columns as it identifies values in
/// a column, so pass the model's [`super::TextStorage`] (en-US: `a` = `A` but `à` ≠ `À`); a
/// [`super::Collation`] gives linguistic equivalence. Blank never matches. If the to
/// column holds a key twice, the first row wins. A one-to-one relationship has one index per
/// direction (`RelationshipStorageID` and `RelationshipStorage2ID`).
pub fn build_relationship_index<K: TextEquivalence + ?Sized>(
    from: &[(u32, CellValue)],
    to_rows: &[CellValue],
    text: &K,
) -> Result<RelationshipIndex, SystemTableError> {
    check_rows(to_rows.len())?;
    let mut rows: HashMap<KeyValue, usize> = HashMap::with_capacity(to_rows.len());
    for (row, value) in to_rows.iter().enumerate() {
        if let Some(key) = KeyValue::of(value, text) {
            rows.entry(key).or_insert(row);
        }
    }
    let max_id = from
        .iter()
        .map(|(id, _)| *id)
        .max()
        .unwrap_or(BLANK_DATA_ID)
        .max(BLANK_DATA_ID);
    let len = usize::try_from(max_id)
        .ok()
        .and_then(|m| m.checked_add(1))
        .ok_or(SystemTableError::TooManyRows(usize::MAX))?;
    check_rows(len)?;
    let mut index = vec![-1i64; len];
    let mut unmatched = 0u64;
    for (id, value) in from {
        if *id < BLANK_DATA_ID {
            return Err(SystemTableError::ReservedDataId(*id));
        }
        let Some(key) = KeyValue::of(value, text) else {
            continue;
        };
        let slot = index
            .get_mut(*id as usize)
            .ok_or(SystemTableError::TooManyRows(len))?;
        match rows.get(&key) {
            Some(&row) => *slot = i64::try_from(row).unwrap_or(i64::MAX),
            None => unmatched += 1,
        }
    }
    let stored = biased(index.iter().copied())?;
    // The width fits any row of the to-table (stored row + 1 ≤ its row count), not just the
    // rows referenced: observed on 200 engine-written indexes whose keys reach only the first
    // rows of the to-table.
    let to_count = u32::try_from(to_rows.len()).map_err(|_| SystemTableError::TooManyRows(to_rows.len()))?;
    let width = bitpack::width_for(stored.iter().copied().max().unwrap_or(0).max(to_count));
    Ok(RelationshipIndex {
        table: SystemTableData {
            kind: SystemTableKind::RelationshipIndex,
            columns: vec![SystemColumn::new("INDEX", stored, -1, width, 3)],
            records_per_segment: (len as u64).max(1),
        },
        index,
        unmatched,
    })
}

/// A hashable join key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum KeyValue {
    Int(i64),
    Real(u64),
    Bool(bool),
    Text(EquivalenceKey),
}

impl KeyValue {
    fn of<K: TextEquivalence + ?Sized>(value: &CellValue, text: &K) -> Option<KeyValue> {
        Some(match value {
            CellValue::Null => return None,
            CellValue::Int(n) => KeyValue::Int(*n),
            CellValue::Bool(b) => KeyValue::Bool(*b),
            CellValue::Decimal(d) if d % 10_000 == 0 => KeyValue::Int(d / 10_000),
            CellValue::Decimal(d) => KeyValue::real(*d as f64 / 10_000.0),
            CellValue::Real(r) | CellValue::DateTime(r) => KeyValue::real(*r),
            CellValue::Text(t) => KeyValue::Text(text.text_key(t)),
        })
    }

    fn real(r: f64) -> KeyValue {
        if r.fract() == 0.0 && r.abs() < 9.0e18 {
            KeyValue::Int(r as i64)
        } else {
            KeyValue::Real(r.to_bits())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::{Collation, TextStorage};

    #[test]
    fn maps_ids_to_rows_plus_one() {
        // to rows: 3, 1, 5, 2, 4
        let to: Vec<CellValue> = [3, 1, 5, 2, 4].iter().map(|&v| CellValue::Int(v)).collect();
        let from = [
            (3, CellValue::Int(2)),
            (4, CellValue::Int(7)),
            (5, CellValue::Int(1)),
            (2, CellValue::Null),
        ];
        let r = build_relationship_index(&from, &to, &Collation::default()).unwrap();
        assert_eq!(r.index, [-1, -1, -1, 3, -1, 1]);
        assert_eq!(r.table.columns[0].values, [0, 0, 0, 4, 0, 2]);
        assert_eq!(r.table.columns[0].width, 3);
        assert_eq!(r.unmatched, 1);
    }

    #[test]
    fn string_keys_ignore_case() {
        let to = [CellValue::Text("abc".into()), CellValue::Text("Déf".into())];
        let from = [
            (3, CellValue::Text("ABC".into())),
            (4, CellValue::Text("déf".into())),
            (5, CellValue::Text("DEF".into())),
        ];
        let r = build_relationship_index(&from, &to, &Collation::default()).unwrap();
        assert_eq!(r.index, [-1, -1, -1, 0, 1, -1]);
    }

    #[test]
    fn stored_text_keys_follow_the_culture() {
        let t = |s: &str| CellValue::Text(s.into());
        // en-US: only ASCII case merges, so `À` and `à` are two keys; the stored (trimmed)
        // values of both sides are compared.
        let to = [t("À"), t("à"), t("a"), t("Ω")];
        let from = [(3, t("à")), (4, t("À")), (5, t("A")), (6, t("ω"))];
        let us = TextStorage::for_culture("en-US", false);
        let r = build_relationship_index(&from, &to, &us).unwrap();
        assert_eq!(r.index, [-1, -1, -1, 1, 0, 2, -1]);
        assert_eq!(r.unmatched, 1);
        // fr-FR: case variants are one value.
        let fr = TextStorage::for_culture("fr-FR", false);
        let r = build_relationship_index(&from, &[t("À"), t("Ω")], &fr).unwrap();
        assert_eq!(r.index, [-1, -1, -1, 0, 0, -1, 1]);
    }
}
