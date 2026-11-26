//! Calculated tables and calculated columns evaluated for a structural PBIX save
//! (`crate::pbix_struct`), with the viewer's DAX evaluator ([`super`]).
//!
//! Data comes from the original `DataModel` (decoded lazily, as for the preview) and from what
//! the save provides: new tables (as literal tables of the preview cache) and the new data of
//! replaced tables (as decoded columns, overriding the stored ones). A calculated table is its
//! expression evaluated in an empty filter context; a calculated column is its expression
//! evaluated once per row of its table, with that row as the row context and no filter. Anything
//! the evaluator does not support is an error with its reason: the save is then refused.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bicraft_model::{DataType, SemanticModel};
use bicraft_vertipaq::CellValue;

use super::super::store::{ColData, LiteralTable, Miss, VertipaqStore, literal_key, model_table};
use super::super::{Ctx, Fail, MAX_BACKUP_BYTES, PreviewLimits, State};
use super::{Engine, Env, FCtx, RowB, Scope, V};

/// Longest time spent evaluating one calculated table or column.
const MAX_TOTAL: Duration = Duration::from_secs(120);

/// A calculated table's result: columns with their (lineage) data type and values.
pub(crate) struct EvalTable {
    pub columns: Vec<(String, Option<DataType>, Vec<CellValue>)>,
    /// Per column, the model column it takes its name and values from, as the engine writes a
    /// calculated table column's `sourceColumn` (`Table[Column]`, names as they are); `None`
    /// for a computed or explicitly named column (`[Name]`).
    pub sources: Vec<Option<String>>,
}

pub(crate) struct StructEval {
    session: crate::Session,
    st: State,
    model: SemanticModel,
    limits: PreviewLimits,
}

fn fail_text(f: Fail) -> String {
    match f {
        Fail::Unsupported(r) => format!("not supported by bicraft's DAX evaluator: {r}"),
        Fail::Miss(Miss::Unavailable(r)) => format!("{}: {r}", super::query::DATA_UNAVAILABLE),
        Fail::Miss(Miss::Error(r)) => r,
    }
}

impl StructEval {
    /// `data_model`: the package's `DataModel` part (its columns are read when needed).
    pub(crate) fn new(data_model: Option<&[u8]>, model: SemanticModel, max_rows: usize) -> Self {
        let st = State {
            store: data_model.map(|p| VertipaqStore::open(p, MAX_BACKUP_BYTES).map(Arc::new)),
            ..State::default()
        };
        let mut ev = Self {
            session: crate::Session::default(),
            st,
            model,
            limits: PreviewLimits {
                rows_per_visual: max_rows,
                table_rows: max_rows,
                fallback_rows: max_rows,
                budget: MAX_TOTAL,
                now: 0.0,
                utc_offset: 0.0,
            },
        };
        ev.set_clock(None, None);
        ev
    }

    /// The clock `NOW()`/`TODAY()` and `UTCNOW()`/`UTCTODAY()` read in calculated tables and
    /// columns: `utc_ms` (ms since 1899-12-30 UTC, a refresh's start; `None`: now) in local
    /// time, `offset_minutes` (local time minus UTC, `utc_offset_minutes`) or else the process's
    /// local offset at that instant ([`crate::clock`]), as `dax.evaluate` does for measures.
    pub(crate) fn set_clock(&mut self, utc_ms: Option<i64>, offset_minutes: Option<i32>) {
        use crate::preview::filters;
        let utc = utc_ms.map_or_else(filters::now_utc, |ms| ms as f64 / 86_400_000.0);
        let off = offset_minutes.map_or_else(|| filters::offset_days_at(utc), |m| f64::from(m) / 1440.0);
        self.limits.now = utc + off;
        self.limits.utc_offset = off;
    }

    /// Reads the stored data under the names a save gives it: tables (old, new), then columns
    /// (table by its new name, old, new).
    pub(crate) fn rename_stored(
        &mut self,
        tables: &[(String, String)],
        columns: &[(String, String, String)],
    ) {
        if let Some(Ok(store)) = self.st.store.as_mut() {
            // Fresh evaluators own their store; a shared one is copied.
            Arc::make_mut(store).rename(tables, columns);
        }
    }

    /// The folder Power Query sources of tables not provided may read (`None`: no file access).
    pub(crate) fn set_data_root(&mut self, root: Option<std::path::PathBuf>) {
        self.session.data_root = root;
    }

    /// Replaces the model (e.g. once a calculated table's columns are known); provided data is
    /// kept.
    pub(crate) fn set_model(&mut self, model: SemanticModel) {
        self.model = model;
        // Literal tables are keyed by their definition: re-key what was provided.
        let keys: Vec<String> = self.st.literal.keys().cloned().collect();
        for k in keys {
            if let (Some(t), Some((key, _))) = (model_table(&self.model, &k), self.st.literal.get_mut(&k)) {
                *key = literal_key(t);
            }
        }
    }

    /// Provides the data of a table: a new table (`stored` false) becomes a literal table; a
    /// table that is also stored (replaced data) overrides its stored columns. Calculated
    /// columns not given yet read as unavailable, never as their stored values.
    pub(crate) fn provide(
        &mut self,
        table: &str,
        rows: usize,
        columns: &[(String, Vec<CellValue>)],
        stored: bool,
    ) -> Result<(), String> {
        let lower = table.to_lowercase();
        let mut map = HashMap::new();
        for (name, values) in columns {
            let data = ColData::from_stored(values.clone()).map_err(|m| match m {
                Miss::Unavailable(r) | Miss::Error(r) => r,
            })?;
            map.insert(name.to_lowercase(), Arc::new(data));
        }
        if stored {
            self.st.provided_rows.insert(lower.clone(), rows);
            if let Some(t) = model_table(&self.model, table) {
                for c in &t.columns {
                    let key = (lower.clone(), c.name.to_lowercase());
                    let entry = match map.get(&c.name.to_lowercase()) {
                        Some(d) => Ok(Arc::clone(d)),
                        None => Err(Miss::Unavailable(format!(
                            "column '{}'[{}] is not computed yet",
                            t.name, c.name
                        ))),
                    };
                    self.st.columns.insert(key, entry);
                }
            }
        } else {
            let key = model_table(&self.model, table).map_or(0, literal_key);
            self.st
                .literal
                .insert(lower, (key, Ok(Arc::new(LiteralTable { rows, columns: map }))));
        }
        Ok(())
    }

    /// Provides the computed values of one column (a calculated column of a provided or stored
    /// table), so later expressions can read it.
    pub(crate) fn provide_column(
        &mut self,
        table: &str,
        column: &str,
        values: &[CellValue],
    ) -> Result<(), String> {
        let data = Arc::new(ColData::from_stored(values.to_vec()).map_err(|m| match m {
            Miss::Unavailable(r) | Miss::Error(r) => r,
        })?);
        let lower = table.to_lowercase();
        if let Some((_, Ok(lit))) = self.st.literal.get_mut(&lower) {
            let mut next = LiteralTable {
                rows: lit.rows,
                columns: lit.columns.clone(),
            };
            next.columns.insert(column.to_lowercase(), data);
            *lit = Arc::new(next);
        } else {
            self.st.columns.insert((lower, column.to_lowercase()), Ok(data));
        }
        Ok(())
    }

    /// Evaluates a calculated table expression.
    pub(crate) fn table(&mut self, expression: &str) -> Result<EvalTable, String> {
        let expr = parse(expression)?;
        let mut ctx = Ctx {
            s: &self.session,
            st: &mut self.st,
            model: &self.model,
            limits: &self.limits,
        };
        let mut eng = Engine::new(&mut ctx);
        let fc = FCtx::default();
        let env = Env::new();
        let scope = Scope {
            fc: &fc,
            rc: &[],
            env: &env,
            depth: 0,
            mdepth: 0,
        };
        let tab = eng.table(&expr, &scope).map_err(fail_text)?;
        // Rows of a model table: its columns but the hidden row number, then the added ones.
        let cols = eng.tab_cols(&tab);
        let rows = eng.tab_values(&tab).map_err(fail_text)?;
        let mut columns: Vec<(String, Option<DataType>, Vec<CellValue>)> = Vec::new();
        let mut sources = Vec::new();
        for (i, c) in cols.iter().enumerate() {
            let lineage = c.lin.and_then(|l| eng.cols.get(l)).map(|info| {
                let table = eng.tabs.get(info.table).map_or("", |t| t.name.as_str());
                (table.to_string(), info.name.clone())
            });
            // A column that lost its lineage (`UNION` of different columns) keeps the first
            // table's column as its source.
            let origin = c.origin.and_then(|l| eng.cols.get(l)).map(|info| {
                let table = eng.tabs.get(info.table).map_or("", |t| t.name.as_str());
                format!("{table}[{}]", info.name)
            });
            let (name, source) = match lineage {
                None if origin.is_some() => (eng.tcol_name(c), origin),
                // An unnamed `SELECTCOLUMNS` column `T[C]` becomes `T_C` in a calculated table.
                Some((table, column)) if c.qualified => {
                    (format!("{table}_{column}"), Some(format!("{table}[{column}]")))
                }
                Some((table, column)) if c.name.is_none() => {
                    (column.clone(), Some(format!("{table}[{column}]")))
                }
                _ => (eng.tcol_name(c), None),
            };
            sources.push(source);
            let data_type = c.ty.or_else(|| c.lin.and_then(|l| eng.col_type(l)));
            if columns.iter().any(|(n, _, _)| n.eq_ignore_ascii_case(&name)) {
                return Err(format!("the result has two columns named `{name}`"));
            }
            let values = rows
                .iter()
                .map(|r| r.get(i).map_or(CellValue::Null, V::to_cell))
                .collect();
            columns.push((name, data_type, values));
        }
        Ok(EvalTable { columns, sources })
    }

    /// Evaluates a calculated column of `table`: one value per row.
    pub(crate) fn column(&mut self, table: &str, expression: &str) -> Result<Vec<CellValue>, String> {
        let expr = parse(expression)?;
        let mut ctx = Ctx {
            s: &self.session,
            st: &mut self.st,
            model: &self.model,
            limits: &self.limits,
        };
        let mut eng = Engine::new(&mut ctx);
        let t = eng
            .table_id(table)
            .ok_or_else(|| format!("table '{table}' is not in the model"))?;
        let n = eng.rows(t).map_err(fail_text)?;
        let fc = FCtx::default();
        let env = Env::new();
        let start = Instant::now();
        let mut out = Vec::with_capacity(n);
        for row in 0..n {
            if start.elapsed() > MAX_TOTAL {
                return Err(format!(
                    "evaluating the column over {n} rows takes more than {} s",
                    MAX_TOTAL.as_secs()
                ));
            }
            // The evaluator's per-expression budget applies to each row.
            eng.work = 0;
            eng.start = Instant::now();
            let rc = [RowB::Row {
                table: t,
                row: u32::try_from(row).map_err(|_| "too many rows".to_string())?,
            }];
            let scope = Scope {
                fc: &fc,
                rc: &rc,
                env: &env,
                depth: 0,
                mdepth: 0,
            };
            let v = eng
                .scalar(&expr, &scope)
                .map_err(|f| format!("row {}: {}", row.saturating_add(1), fail_text(f)))?;
            out.push(v.to_cell());
        }
        Ok(out)
    }
}

fn parse(expression: &str) -> Result<bicraft_dax::ast::Expr, String> {
    let (expr, diags) = bicraft_dax::parse_expression(expression);
    if let Some(d) = diags.iter().find(|d| d.severity == bicraft_dax::Severity::Error) {
        return Err(format!("the expression does not parse: {}", d.message));
    }
    expr.ok_or_else(|| "the expression does not parse".to_string())
}
