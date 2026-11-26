//! The data state of a visual: whether its evaluated dataset (`data` in the snapshot) holds
//! anything to show, read from the values, not from the drawn text, so it does not depend on
//! the UI language (`docs/render.md`, "Data states").

use serde_json::Value;

use crate::data::{arr, get, sk};

/// Rows of `data.rows` examined for blank cells.
const MAX_ROWS: usize = 100_000;

/// What a visual's evaluated dataset holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataKind {
    /// Rows and at least one non-blank value (or rows and no value columns, e.g. a slicer).
    Ok,
    /// Zero rows, and the visual groups by at least one column (chart, table, matrix, slicer).
    NoRows,
    /// Every value cell is blank, or zero rows with no grouping column (a card, KPI, gauge or
    /// multi-row card bound to measures only).
    BlankValues,
    /// Rows, some value cells blank and some not.
    PartialBlank,
    /// The visual has no data fields (text box, shape, image, button, an empty visual).
    NotBound,
    /// Fields are bound but the engine returned no dataset (`data.status` ≠ `ok`: no table
    /// data, an expression it cannot evaluate, an error); the renderer draws made-up values.
    NotEvaluated,
}

impl DataKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DataKind::Ok => "ok",
            DataKind::NoRows => "no_rows",
            DataKind::BlankValues => "blank_values",
            DataKind::PartialBlank => "partial_blank",
            DataKind::NotBound => "not_bound",
            DataKind::NotEvaluated => "not_evaluated",
        }
    }

    /// `no_rows`, `blank_values` and `not_evaluated`: nothing real to show.
    pub fn without_data(self) -> bool {
        matches!(
            self,
            DataKind::NoRows | DataKind::BlankValues | DataKind::NotEvaluated
        )
    }
}

/// A visual's data state with its counts (counts are 0 when there is no dataset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataState {
    pub kind: DataKind,
    /// Rows of the whole result (`data.total_rows`, else the rows returned); `None` without a
    /// dataset.
    pub rows: Option<usize>,
    /// Measure and aggregation cells examined (the returned rows, at most 100 000).
    pub value_cells: usize,
    /// Of those, the blank (null) ones.
    pub blank_cells: usize,
}

/// The data state of a snapshot visual (`fields` and `data`, `docs/view-snapshot.md`).
pub fn data_state(visual: &Value) -> DataState {
    data_state_of(get(visual, "fields"), get(visual, "data"))
}

/// [`data_state`] from a visual's `fields` and `data` values.
pub fn data_state_of(fields: &Value, d: &Value) -> DataState {
    let none = |kind| DataState {
        kind,
        rows: None,
        value_cells: 0,
        blank_cells: 0,
    };
    let bound = arr(fields).iter().any(Value::is_object);
    if !(d.is_object() && sk(d, "status") == "ok") {
        return none(if bound {
            DataKind::NotEvaluated
        } else {
            DataKind::NotBound
        });
    }
    let cols = arr(get(d, "columns"));
    if cols.is_empty() {
        return none(if bound {
            DataKind::NotEvaluated
        } else {
            DataKind::NotBound
        });
    }
    let is_value = |c: &Value| matches!(sk(c, "kind"), "measure" | "aggregation");
    let value_idx: Vec<usize> = cols
        .iter()
        .enumerate()
        .filter(|(_, c)| is_value(c))
        .map(|(i, _)| i)
        .collect();
    let groups = cols.len() > value_idx.len();
    let rows = arr(get(d, "rows"));
    let returned = rows.len();
    let total = get(d, "total_rows")
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(returned)
        .max(returned);
    let (mut value_cells, mut blank_cells) = (0usize, 0usize);
    for r in rows.iter().take(MAX_ROWS) {
        let cells = arr(r);
        for &i in &value_idx {
            value_cells = value_cells.saturating_add(1);
            if cells.get(i).is_none_or(Value::is_null) {
                blank_cells = blank_cells.saturating_add(1);
            }
        }
    }
    let kind = if returned == 0 {
        if groups {
            DataKind::NoRows
        } else {
            DataKind::BlankValues
        }
    } else if value_cells > 0 && blank_cells == value_cells {
        DataKind::BlankValues
    } else if blank_cells > 0 {
        DataKind::PartialBlank
    } else {
        DataKind::Ok
    };
    DataState {
        kind,
        rows: Some(total),
        value_cells,
        blank_cells,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn st(v: Value) -> (&'static str, Option<usize>, usize, usize) {
        let s = data_state(&v);
        (s.kind.as_str(), s.rows, s.value_cells, s.blank_cells)
    }

    #[test]
    fn states() {
        let m = json!({"kind": "measure"});
        let c = json!({"kind": "column"});
        let f = json!([{"role": "Values"}]);
        assert_eq!(st(json!({"type": "textbox"})), ("not_bound", None, 0, 0));
        assert_eq!(st(json!({"fields": [], "data": null})), ("not_bound", None, 0, 0));
        assert_eq!(
            st(json!({"fields": f, "data": {"status": "unsupported"}})),
            ("not_evaluated", None, 0, 0)
        );
        assert_eq!(st(json!({"fields": f})), ("not_evaluated", None, 0, 0));
        // card: one blank row, or no row at all
        assert_eq!(
            st(json!({"fields": f, "data": {"status": "ok", "columns": [m], "rows": [[null]]}})),
            ("blank_values", Some(1), 1, 1)
        );
        assert_eq!(
            st(json!({"fields": f, "data": {"status": "ok", "columns": [m], "rows": []}})),
            ("blank_values", Some(0), 0, 0)
        );
        assert_eq!(
            st(json!({"fields": f, "data": {"status": "ok", "columns": [m], "rows": [[1.5]]}})),
            ("ok", Some(1), 1, 0)
        );
        // grouped: zero rows, partial blanks, a short row, total_rows
        assert_eq!(
            st(json!({"fields": f, "data": {"status": "ok", "columns": [c, m], "rows": []}})),
            ("no_rows", Some(0), 0, 0)
        );
        assert_eq!(
            st(json!({"fields": f, "data": {"status": "ok", "columns": [c, m],
                "rows": [["a", 1], ["b", null], ["c"]], "total_rows": 9}})),
            ("partial_blank", Some(9), 3, 2)
        );
        assert_eq!(
            st(json!({"fields": f, "data": {"status": "ok", "columns": [c, m], "rows": [["a", null]]}})),
            ("blank_values", Some(1), 1, 1)
        );
        // a slicer: rows, no value columns
        assert_eq!(
            st(json!({"fields": f, "data": {"status": "ok", "columns": [c], "rows": [["a"]]}})),
            ("ok", Some(1), 0, 0)
        );
        // hostile shapes do not panic
        assert_eq!(
            st(json!({"fields": f, "data": {"status": "ok", "columns": [m], "rows": 3, "total_rows": -1}})),
            ("blank_values", Some(0), 0, 0)
        );
        assert!(DataKind::NotEvaluated.without_data() && !DataKind::PartialBlank.without_data());
    }
}
