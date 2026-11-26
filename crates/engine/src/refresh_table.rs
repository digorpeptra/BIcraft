//! `table.add_refresh_info`: an optional "Refresh info" table the report author can put in a
//! visual (a card, a table) to show when the data was last refreshed. Nothing is added unless the
//! author runs the command.
//!
//! The table is a standard import table with one row, made by a Power Query (M) query that reads
//! the clock: `DateTimeZone.UtcNow()` and `DateTime.LocalNow()`. Power BI Desktop, the Power BI
//! service and bicraft's `model.refresh` evaluate it at each refresh, so the row is the time of
//! the last refresh. An optional measure (`Last refresh` by default) returns it with the
//! `General Date` format, which each viewer's culture displays in its own way. Nothing in it is
//! specific to bicraft: the file opens and refreshes the same way in Desktop and the service.
//! In the service, `DateTime.LocalNow()` is the service's clock (UTC); the UTC column is the one
//! that means the same everywhere, and the measure reads it.

use serde_json::{Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, opt_str};

/// The default table name.
pub(crate) const DEFAULT_TABLE: &str = "Refresh info";
/// The default measure name.
pub(crate) const DEFAULT_MEASURE: &str = "Last refresh";
/// The table's columns.
pub(crate) const UTC_COLUMN: &str = "Last refresh (UTC)";
pub(crate) const LOCAL_COLUMN: &str = "Last refresh (local)";

/// The table's Power Query expression.
pub(crate) fn m_expression() -> String {
    format!(
        "let\n    Source = #table(\n        type table [#\"{UTC_COLUMN}\" = datetime, #\"{LOCAL_COLUMN}\" = datetime],\n        \
         {{{{DateTimeZone.RemoveZone(DateTimeZone.UtcNow()), DateTime.LocalNow()}}}}\n    )\nin\n    Source"
    )
}

/// A table name as a quoted DAX table reference.
fn quoted(table: &str) -> String {
    format!("'{}'", table.replace('\'', "''"))
}

/// `table.add_refresh_info {name?, measure?, measure_name?, hidden?}`.
pub(crate) fn add_refresh_info(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "measure", "measure_name", "hidden"])?;
    let name = opt_str(p, "name")?.unwrap_or(DEFAULT_TABLE).trim().to_string();
    let with_measure = opt_bool(p, "measure")?.unwrap_or(true);
    let measure = opt_str(p, "measure_name")?
        .unwrap_or(DEFAULT_MEASURE)
        .trim()
        .to_string();
    let hidden = opt_bool(p, "hidden")?.unwrap_or(false);
    if name.is_empty() {
        return Err("`name` must not be empty".into());
    }
    let model = crate::commands::model(s)?;
    if model.table(&name).is_some() {
        return Err(format!("the model already has a table named '{name}'"));
    }
    if with_measure {
        if measure.is_empty() {
            return Err("`measure_name` must not be empty".into());
        }
        if let Some((t, _)) = model.measure(&measure) {
            return Err(format!(
                "the model already has a measure named [{measure}] (in '{}')",
                t.name
            ));
        }
    }
    let m = m_expression();
    crate::pbix_struct::table_import(
        s,
        &json!({
            "name": name,
            "m": m,
            "hidden": hidden,
            "description": "When the data was last refreshed: one row, evaluated at each refresh \
                            (Power Query DateTimeZone.UtcNow and DateTime.LocalNow).",
        }),
    )?;
    let mut out = json!({
        "table": name,
        "columns": [UTC_COLUMN, LOCAL_COLUMN],
        "m": m,
    });
    if with_measure {
        let expression = format!("MAX({}[{UTC_COLUMN}])", quoted(&name));
        crate::model_props::measure_add(
            s,
            &json!({
                "table": name,
                "name": measure,
                "expression": expression,
                "format_string": "General Date",
                "description": "The time of the last data refresh (UTC).",
            }),
        )?;
        out["measure"] = json!({"name": measure, "expression": expression});
    }
    Ok(out)
}

#[cfg(test)]
#[path = "refresh_table_tests.rs"]
mod refresh_table_tests;
