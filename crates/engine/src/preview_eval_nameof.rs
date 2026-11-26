//! `NAMEOF` in the preview evaluator ([`super`]): the name of a measure, column or table as text,
//! as field parameters (`{("Amount", NAMEOF('Sales'[Amount]), 0), …}`) store it.
//!
//! Rules (checked against the Analysis Services engine of Power BI Desktop 2.158, entries
//! `nameof-*` of `tests/table_functions/expected.json`):
//! - the object is a measure (`[M]` or `T[M]`), a fully qualified column (`T[C]`) or a table;
//!   anything else (a variable, an unqualified column, an expression) is an error;
//! - names are spelled as the model spells them, whatever the case of the reference;
//! - escaped (the default): a table is `'T'` (always quoted, `'` doubled), a column or measure
//!   `[C]` (`]` doubled); the full name is `'T'[C]`, a measure's included (its home table);
//! - components: `FULL` (default); `SELF` (the object's own part); `PARENT` and `TABLE` (the
//!   table of a column or measure); `COLUMN` (columns only); `MEASURE` (measures only); a
//!   table accepts `FULL`, `SELF` and `TABLE`;
//! - `UNESCAPED` gives the bare names; the full name of a column cannot be unescaped (error),
//!   the full name of a measure is then the table and measure names run together.
//!
//! Calendar references are not evaluated.

use bicraft_dax::ast::Expr;

use super::super::Fail;
use super::{Engine, Scope, V, arg, dax_error, keyword, need, unsupported};

/// What a `NAMEOF` argument names.
enum Object {
    Table(String),
    Column { table: String, name: String },
    Measure { table: String, name: String },
}

fn quote_table(name: &str) -> String {
    format!("'{}'", name.replace('\'', "''"))
}

fn bracket(name: &str) -> String {
    format!("[{}]", name.replace(']', "]]"))
}

impl Engine<'_, '_> {
    /// `NAMEOF(object [, component [, escaping]])`.
    pub(super) fn nameof(&mut self, args: &[Option<Expr>], s: &Scope) -> Result<V, Fail> {
        let object = self.nameof_object(need(args, 0, "NAMEOF")?, s)?;
        let component = match arg(args, 1) {
            None => "FULL".to_string(),
            Some(e) => match keyword(e) {
                Some(k) => k,
                None => return dax_error("NAMEOF() component must be a keyword"),
            },
        };
        let escaped = match arg(args, 2).map(keyword) {
            None => true,
            Some(Some(k)) if k == "ESCAPED" => true,
            Some(Some(k)) if k == "UNESCAPED" => false,
            Some(_) => return dax_error("NAMEOF() escaping must be ESCAPED or UNESCAPED"),
        };
        let table_text = |t: &str| if escaped { quote_table(t) } else { t.to_string() };
        let self_text = |n: &str| if escaped { bracket(n) } else { n.to_string() };
        let invalid = || {
            dax_error(format!(
                "NAMEOF() component {component} is not valid for this object"
            ))
        };
        let text = match (&object, component.as_str()) {
            (Object::Table(t), "FULL" | "SELF" | "TABLE") => table_text(t),
            (Object::Table(_), _) => return invalid(),
            (Object::Column { table, .. } | Object::Measure { table, .. }, "PARENT" | "TABLE") => {
                table_text(table)
            }
            (Object::Column { name, .. }, "SELF" | "COLUMN")
            | (Object::Measure { name, .. }, "SELF" | "MEASURE") => self_text(name),
            (Object::Column { table, name }, "FULL") => {
                if !escaped {
                    return dax_error("NAMEOF(): a fully qualified column name cannot be unescaped");
                }
                format!("{}{}", quote_table(table), bracket(name))
            }
            (Object::Measure { table, name }, "FULL") => {
                format!("{}{}", table_text(table), self_text(name))
            }
            (_, "CALENDAR") | (Object::Column { .. } | Object::Measure { .. }, _) => return invalid(),
        };
        Ok(V::Text(text))
    }

    fn nameof_object(&mut self, e: &Expr, s: &Scope) -> Result<Object, Fail> {
        let refused = || dax_error("NAMEOF() takes a measure, a fully qualified column or a table reference");
        match e {
            Expr::Name { name, .. } => {
                let lower = name.to_lowercase();
                if s.env.iter().any(|(n, _)| *n == lower) {
                    return refused();
                }
                match self.table_id(name).and_then(|t| self.tabs.get(t)) {
                    Some(t) => Ok(Object::Table(t.name.clone())),
                    None => unsupported(format!("NAMEOF() of `{name}`, which is not a table")),
                }
            }
            Expr::Column {
                variation: Some(_), ..
            } => unsupported("NAMEOF() of a column variation"),
            Expr::Column { table, name, .. } => {
                if let Some(def) = self.measure_def(table.as_deref(), name) {
                    let lower = def.name.to_lowercase();
                    let home = self
                        .model
                        .tables
                        .iter()
                        .find(|t| match table {
                            Some(q) => t.name.eq_ignore_ascii_case(q),
                            None => t.measures.iter().any(|m| m.name.to_lowercase() == lower),
                        })
                        .map(|t| t.name.clone());
                    return match home {
                        Some(table) => Ok(Object::Measure {
                            table,
                            name: def.name.clone(),
                        }),
                        None => unsupported(format!("NAMEOF() of [{name}]: its table is unknown")),
                    };
                }
                let Some(t) = table else {
                    return refused();
                };
                match self.col_id(t, name).and_then(|c| self.cols.get(c)) {
                    Some(c) if !c.row_number => {
                        let table = self.tabs.get(c.table).map(|t| t.name.clone()).unwrap_or_default();
                        Ok(Object::Column {
                            table,
                            name: c.name.clone(),
                        })
                    }
                    _ => unsupported(format!("NAMEOF() of unknown column '{t}'[{name}]")),
                }
            }
            _ => refused(),
        }
    }
}
