//! Calculated-table columns whose name is inferred follow the renames of their source (G15).
//!
//! A calculated table's column without an explicit name (`isNameInferred`) is named after the
//! column its values come from (`sourceColumn`). Observed on the Analysis Services engine of
//! Power BI Desktop 2.158 (TOM `RequestRename`, the engine-side rename with formula fix-up that
//! Desktop uses), with `C = Sales`, `VALUES(Sales[Amount])`, `SUMMARIZE(Sales, Sales[Amount], …)`,
//! `FILTER(C, …)` and `SELECTCOLUMNS(Sales, Sales[Amount])`:
//!
//! - renaming `Sales[Amount]` to `Net` renames every inferred column sourced from it: `Amount`
//!   becomes `Net`, and the `Sales_Amount` of an unnamed `SELECTCOLUMNS` column becomes
//!   `Sales_Net`; the column stays inferred and its `sourceColumn` names the new column;
//! - that rename is itself a column rename: DAX referencing `C[Amount]` is rewritten, and
//!   inferred columns of other calculated tables sourced from `C[Amount]` follow in turn;
//! - renaming the table `Sales` renames only the `Sales_…` columns;
//! - renaming an inferred column by hand makes its name explicit (`isNameInferred` false), and
//!   the columns inferred from it follow.
//!
//! DAX that did not bind before the rename (an expression in error) is left as it is, as the
//! engine does.

use bicraft_dax::deps::ObjectRef;
use bicraft_model::SemanticModel;
use serde_json::{Value, json};

use crate::dax_rename::{DaxChange, RenameOutcome, rename_in_model};
use crate::rename_refs::{NameChange, follow, source_parts};

/// Most follow-up renames one rename triggers.
const MAX_FOLLOWERS: usize = 256;

/// One inferred column renamed because its source was renamed.
pub(crate) struct Follower {
    pub table: String,
    pub old: String,
    pub new: String,
    /// The model just before this rename (report-level DAX is resolved against it).
    pub before: SemanticModel,
}

/// Everything the follow-up renames did.
#[derive(Default)]
pub(crate) struct Followers {
    pub renamed: Vec<Follower>,
    pub dax: RenameOutcome,
    /// `metadata_updated` entries.
    pub metadata: Vec<Value>,
}

impl Followers {
    pub(crate) fn summary(&self) -> Value {
        Value::Array(
            self.renamed
                .iter()
                .map(|f| json!({"table": f.table, "old": f.old, "new": f.new}))
                .collect(),
        )
    }
}

fn eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn is_inferred_table_column(c: &bicraft_model::Column) -> bool {
    c.extra.get("type").and_then(Value::as_str) == Some("calculatedTableColumn")
        && c.extra.get("isNameInferred").and_then(Value::as_bool) == Some(true)
}

/// The inferred columns that `change` (already applied, `sourceColumn`s already followed)
/// renames: `(table, old name, new name)`.
fn pending(m: &SemanticModel, change: NameChange<'_>) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for t in &m.tables {
        for c in t.columns.iter().filter(|c| is_inferred_table_column(c)) {
            let Some((table, _, column)) = c.source_column.as_deref().and_then(source_parts) else {
                continue;
            };
            let new = match change {
                NameChange::Column { table: ct, old, new } if eq(&table, ct) && eq(&column, new) => {
                    if eq(&c.name, old) {
                        Some(new.to_string())
                    } else if eq(&c.name, &format!("{table}_{old}")) {
                        Some(format!("{table}_{new}"))
                    } else {
                        None
                    }
                }
                NameChange::Table { old, new }
                    if eq(&table, new) && eq(&c.name, &format!("{old}_{column}")) =>
                {
                    Some(format!("{new}_{column}"))
                }
                _ => None,
            };
            let Some(new) = new else { continue };
            // A name taken by another column of the table: left as it is.
            let taken = t.columns.iter().any(|o| !std::ptr::eq(o, c) && eq(&o.name, &new));
            if new != c.name && !taken {
                out.push((t.name.clone(), c.name.clone(), new));
            }
        }
    }
    out
}

/// Renames the column `old` of `table` to `new` the way a column rename does, except DAX when
/// `update_dax` is false. Returns the `metadata_updated` entries.
fn rename_column(
    m: &mut SemanticModel,
    table: &str,
    old: &str,
    new: &str,
    update_dax: bool,
    dax: &mut RenameOutcome,
) -> Vec<Value> {
    if update_dax {
        let target = ObjectRef::Column {
            table: table.to_string(),
            name: old.to_string(),
        };
        let o = rename_in_model(m, &target, &DaxChange::Rename(new.to_string()));
        dax.rewritten.extend(o.rewritten);
        dax.unparsable.extend(o.unparsable);
    }
    if let Some(t) = m.tables.iter_mut().find(|t| eq(&t.name, table)) {
        for c in &mut t.columns {
            if eq(&c.name, old) {
                c.name = new.to_string();
            }
            if c.sort_by_column.as_deref().is_some_and(|s| eq(s, old)) {
                c.sort_by_column = Some(new.to_string());
            }
        }
        crate::edit::rename_column_in_hierarchies(t, old, new);
    }
    for r in &mut m.relationships {
        if eq(&r.from_table, table) && eq(&r.from_column, old) {
            r.from_column = new.to_string();
        }
        if eq(&r.to_table, table) && eq(&r.to_column, old) {
            r.to_column = new.to_string();
        }
    }
    follow(m, NameChange::Column { table, old, new })
}

/// After `change` was applied to `m` (and [`follow`] ran for it): renames the inferred
/// calculated-table columns that follow it, transitively.
pub(crate) fn follow_inferred(m: &mut SemanticModel, change: NameChange<'_>, update_dax: bool) -> Followers {
    let mut out = Followers::default();
    let mut queue = pending(m, change);
    while let Some((table, old, new)) = queue.pop() {
        if out.renamed.len() >= MAX_FOLLOWERS {
            break;
        }
        let before = m.clone();
        let metadata = rename_column(m, &table, &old, &new, update_dax, &mut out.dax);
        out.metadata.extend(metadata);
        queue.extend(pending(
            m,
            NameChange::Column {
                table: &table,
                old: &old,
                new: &new,
            },
        ));
        out.renamed.push(Follower {
            table,
            old,
            new,
            before,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bicraft_model::{Column, Measure, Partition, Table};

    fn inferred(name: &str, source: &str) -> Column {
        let mut c = Column {
            name: name.into(),
            source_column: Some(source.into()),
            ..Default::default()
        };
        c.extra.insert("type".into(), json!("calculatedTableColumn"));
        c.extra.insert("isNameInferred".into(), json!(true));
        c
    }

    fn calc(name: &str, expr: &str, columns: Vec<Column>) -> Table {
        Table {
            name: name.into(),
            columns,
            partitions: vec![Partition {
                name: name.into(),
                source_type: "calculated".into(),
                expression: Some(expr.into()),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    /// The engine's behavior (module documentation): `C1 = Sales`, `C2 = SELECTCOLUMNS(Sales,
    /// Sales[Amount])`, `C9 = FILTER(C1, C1[Amount] > 1)` and a measure on `C1[Amount]`, after
    /// `Sales[Amount]` → `Net` (the rename itself and `follow` already applied).
    #[test]
    fn inferred_names_follow_transitively() {
        let mut c1 = calc("C1", "Sales", vec![inferred("Amount", "Sales[Net]")]);
        c1.measures.push(Measure {
            name: "M1".into(),
            expression: "SUM(C1[Amount])".into(),
            ..Default::default()
        });
        let mut m = SemanticModel {
            tables: vec![
                calc(
                    "Sales",
                    "DATATABLE(\"Amount\", DOUBLE, {{1}})",
                    vec![Column {
                        name: "Net".into(),
                        source_column: Some("[Amount]".into()),
                        ..Default::default()
                    }],
                ),
                c1,
                calc(
                    "C2",
                    "SELECTCOLUMNS(Sales, Sales[Net])",
                    vec![inferred("Sales_Amount", "Sales[Net]")],
                ),
                calc(
                    "C9",
                    "FILTER(C1, C1[Amount] > 1)",
                    vec![inferred("Amount", "C1[Amount]")],
                ),
            ],
            ..Default::default()
        };
        let f = follow_inferred(
            &mut m,
            NameChange::Column {
                table: "Sales",
                old: "Amount",
                new: "Net",
            },
            true,
        );
        let names = |m: &SemanticModel, t: &str| -> Vec<String> {
            m.table(t)
                .unwrap()
                .columns
                .iter()
                .map(|c| c.name.clone())
                .collect()
        };
        assert_eq!(names(&m, "C1"), ["Net"]);
        assert_eq!(names(&m, "C2"), ["Sales_Net"]);
        assert_eq!(names(&m, "C9"), ["Net"]);
        let c9 = m.table("C9").unwrap();
        assert_eq!(c9.columns[0].source_column.as_deref(), Some("C1[Net]"));
        assert_eq!(
            c9.partitions[0].expression.as_deref(),
            Some("FILTER(C1, C1[Net] > 1)")
        );
        assert_eq!(m.table("C1").unwrap().measures[0].expression, "SUM(C1[Net])");
        assert_eq!(f.renamed.len(), 3);

        // A table rename renames only the `Table_Column` names.
        let mut f2 = m.clone();
        f2.tables[0].name = "Orders".into();
        follow(
            &mut f2,
            NameChange::Table {
                old: "Sales",
                new: "Orders",
            },
        );
        follow_inferred(
            &mut f2,
            NameChange::Table {
                old: "Sales",
                new: "Orders",
            },
            true,
        );
        assert_eq!(names(&f2, "C2"), ["Orders_Net"]);
        assert_eq!(names(&f2, "C1"), ["Net"]);
    }
}
