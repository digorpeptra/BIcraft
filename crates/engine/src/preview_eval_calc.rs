//! Calculation groups and format strings in the DAX evaluator ([`super`]): `SELECTEDMEASURE`,
//! `SELECTEDMEASURENAME`, `ISSELECTEDMEASURE`, `SELECTEDMEASUREFORMATSTRING`, the rows of a
//! calculation group table, dynamic format strings and the `[_<measure> FormatString]`
//! references of Power BI's visual queries.
//!
//! Observed on the Analysis Services engine of Power BI Desktop 2.159 (corpus
//! `tests/dax_coverage`, see `docs/dax-oracle.md`):
//! - a calculation group table has one row per item, in ordinal order: the item column (TMSL
//!   `sourceColumn` `Name`) and the ordinal column (`Ordinal`);
//! - an item applies to a **measure reference** evaluated in a filter context where the item
//!   column is filtered directly to that single item (a filter on the ordinal column does not
//!   select an item; several items, or none, apply nothing). Expressions that are not measure
//!   references (`SUM(Sales[Amount])`, `COUNTROWS(Sales)`) are not transformed;
//! - with several groups, the highest precedence applies first: its `SELECTEDMEASURE()` is the
//!   measure with the lower groups' items applied;
//! - once an item is applied, its group is consumed for the rest of that evaluation: measure
//!   references inside the measure's body or inside the item (`SELECTEDMEASURE() + [Units]`) are
//!   not transformed again, until a `CALCULATE` filter (or a context transition) sets or removes a
//!   filter on the group's table, which makes the group apply again
//!   (`CALCULATE(SELECTEDMEASURE(), 'Time Calc'[Period] = "Double")` inside an item);
//! - `SELECTEDMEASURE()` outside an item (or a format string expression) is an error;
//! - the format string of a measure is the format string expression of the first applied item
//!   that has one, else the measure's dynamic format string (whose `SELECTEDMEASURE()` is the
//!   measure with the items applied), else its static format string, else blank;
//!   `SELECTEDMEASUREFORMATSTRING()` in an item is that of the measure with the remaining items.

use std::rc::Rc;

use bicraft_dax::ast::Expr;
use bicraft_model::SemanticModel;
use bicraft_vertipaq::CellValue;
use serde_json::Value;

use super::super::Fail;
use super::super::store::ColData;
use super::{ColId, Engine, Env, FCtx, MeasureDef, Scope, TabId, V, arg, dax_error, unsupported};

/// Most calculation groups of a model the evaluator follows (one bit each in [`FCtx`]).
const MAX_GROUPS: usize = 64;

/// A calculation item.
pub(super) struct CalcItem {
    name: String,
    ordinal: i64,
    expr: Option<Rc<Expr>>,
    format: Option<Rc<Expr>>,
}

/// A calculation group table.
pub(super) struct CalcGroup {
    pub table: TabId,
    precedence: i64,
    /// The item column and the ordinal column.
    pub name_col: Option<ColId>,
    pub ord_col: Option<ColId>,
    /// In row order (ordinal order).
    items: Vec<CalcItem>,
}

fn parse(text: Option<&str>) -> Option<Rc<Expr>> {
    let (e, diags) = bicraft_dax::parse_expression(text?);
    let errors = diags.iter().any(|d| d.severity == bicraft_dax::Severity::Error);
    e.filter(|_| !errors).map(Rc::new)
}

/// The calculation groups of `model`; `col_id(table, column)` finds a column's id.
pub(super) fn load(model: &SemanticModel, col_id: impl Fn(&str, &str) -> Option<ColId>) -> Vec<CalcGroup> {
    let mut out = Vec::new();
    for (ti, t) in model.tables.iter().enumerate() {
        let Some(g) = t.extra.get("calculationGroup").and_then(Value::as_object) else {
            continue;
        };
        let source = |want: &str| {
            t.columns
                .iter()
                .find(|c| {
                    c.source_column
                        .as_deref()
                        .is_some_and(|s| s.eq_ignore_ascii_case(want))
                })
                .and_then(|c| col_id(&t.name, &c.name))
        };
        let mut items: Vec<CalcItem> = g
            .get("calculationItems")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|i| CalcItem {
                        name: i
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        ordinal: i.get("ordinal").and_then(Value::as_i64).unwrap_or(0),
                        expr: parse(i.get("expression").and_then(expression_text).as_deref()),
                        format: parse(
                            i.get("formatStringDefinition")
                                .and_then(|f| f.get("expression"))
                                .and_then(expression_text)
                                .as_deref(),
                        ),
                    })
                    .collect()
            })
            .unwrap_or_default();
        items.sort_by_key(|i| i.ordinal);
        out.push(CalcGroup {
            table: ti,
            precedence: g.get("precedence").and_then(Value::as_i64).unwrap_or(0),
            name_col: source("Name"),
            ord_col: source("Ordinal"),
            items,
        });
    }
    out
}

/// An expression as TMSL writes it: a string, or an array of lines.
fn expression_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(lines) => Some(
            lines
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

/// The dynamic format string expression of a model measure.
pub(super) fn measure_format_expr(m: &bicraft_model::Measure) -> Option<Expr> {
    parse(
        m.extra
            .get("formatStringDefinition")
            .and_then(|f| f.get("expression"))
            .and_then(expression_text)
            .as_deref(),
    )
    .map(|e| (*e).clone())
}

impl Engine<'_, '_> {
    pub(super) fn group_of_table(&self, t: TabId) -> Option<&CalcGroup> {
        self.groups.iter().find(|g| g.table == t)
    }

    /// Rows of a calculation group table (its items).
    pub(super) fn group_rows(&self, t: TabId) -> Option<usize> {
        self.group_of_table(t).map(|g| g.items.len())
    }

    /// The item or ordinal column of a calculation group table, built from its items.
    pub(super) fn group_col_data(&self, c: ColId) -> Option<Result<ColData, Fail>> {
        let g = self
            .groups
            .iter()
            .find(|g| g.name_col == Some(c) || g.ord_col == Some(c))?;
        let values: Vec<CellValue> = if g.name_col == Some(c) {
            g.items.iter().map(|i| CellValue::Text(i.name.clone())).collect()
        } else {
            g.items.iter().map(|i| CellValue::Int(i.ordinal)).collect()
        };
        Some(ColData::from_stored(values).map_err(Fail::Miss))
    }

    /// The groups whose tables have one of `cols`: a filter set or removed on them makes them
    /// apply again.
    pub(super) fn reset_groups(&self, fc: &mut FCtx, cols: &mut dyn Iterator<Item = ColId>) {
        if fc.applied == 0 {
            return;
        }
        for c in cols {
            let Some(t) = self.cols.get(c).map(|i| i.table) else {
                continue;
            };
            if let Some(i) = self.groups.iter().position(|g| g.table == t)
                && i < MAX_GROUPS
            {
                fc.applied &= !(1u64 << i);
            }
        }
    }

    /// The items that apply to a measure reference in `fc`, highest precedence first.
    fn applicable(&mut self, fc: &FCtx) -> Result<Vec<(usize, usize)>, Fail> {
        let mut out = Vec::new();
        for gi in 0..self.groups.len().min(MAX_GROUPS) {
            if fc.applied & (1u64 << gi) != 0 {
                continue;
            }
            let Some(nc) = self.groups.get(gi).and_then(|g| g.name_col) else {
                continue;
            };
            if !fc.filters.iter().any(|f| f.cols.contains(&nc)) {
                continue;
            }
            let codes = self.values_codes(fc, nc, false)?;
            let [code] = codes.as_slice() else {
                continue;
            };
            let V::Text(name) = self.value_of_code(nc, *code)? else {
                continue;
            };
            let Some(g) = self.groups.get(gi) else {
                continue;
            };
            if let Some(ii) = g.items.iter().position(|i| i.name.eq_ignore_ascii_case(&name)) {
                out.push((gi, ii));
            }
        }
        out.sort_by_key(|(gi, _)| std::cmp::Reverse(self.groups.get(*gi).map_or(0, |g| g.precedence)));
        Ok(out)
    }

    /// `fc` with group `gi` consumed.
    fn consumed(fc: &FCtx, gi: usize) -> FCtx {
        let mut out = fc.child();
        out.applied |= 1u64 << gi;
        out
    }

    /// Evaluates `e` with `def` as the selected measure.
    fn with_selected(&mut self, def: &Rc<MeasureDef>, e: &Expr, fc: &FCtx, s: &Scope) -> Result<V, Fail> {
        let env = Env::new();
        let inner = Scope {
            fc,
            rc: &[],
            env: &env,
            depth: s.depth.saturating_add(1),
            mdepth: s.mdepth.saturating_add(1),
        };
        self.selected.push(Rc::clone(def));
        let r = self.scalar(e, &inner);
        self.selected.pop();
        r
    }

    /// A measure reference in `fc` (row contexts already transitioned): the first applicable
    /// item with this measure selected, else the measure's own expression.
    pub(super) fn measure_with_items(
        &mut self,
        def: &Rc<MeasureDef>,
        fc: &FCtx,
        s: &Scope,
    ) -> Result<Option<V>, Fail> {
        if self.groups.is_empty() {
            return Ok(None);
        }
        let Some(&(gi, ii)) = self.applicable(fc)?.first() else {
            return Ok(None);
        };
        let Some(expr) = self
            .groups
            .get(gi)
            .and_then(|g| g.items.get(ii))
            .map(|i| i.expr.clone())
        else {
            return Ok(None);
        };
        let Some(expr) = expr else {
            return unsupported("a calculation item that does not parse");
        };
        let fc2 = Self::consumed(fc, gi);
        self.with_selected(def, &expr, &fc2, s).map(Some)
    }

    /// The format string of measure `def` in `fc` (see the module notes).
    pub(super) fn format_of(&mut self, def: &Rc<MeasureDef>, fc: &FCtx, s: &Scope) -> Result<V, Fail> {
        let mut cur = fc.child();
        for (gi, ii) in self.applicable(fc)? {
            let f = self
                .groups
                .get(gi)
                .and_then(|g| g.items.get(ii))
                .and_then(|i| i.format.clone());
            match f {
                Some(f) => {
                    let fc2 = Self::consumed(&cur, gi);
                    return self.with_selected(def, &f, &fc2, s);
                }
                None => cur.applied |= 1u64 << gi,
            }
        }
        match def.format_expr.clone() {
            // the measure's own dynamic format string sees the items applied
            Some(fe) => self.with_selected(def, &fe, fc, s),
            None => Ok(def.format.clone().map_or(V::Blank, V::Text)),
        }
    }

    /// `SELECTEDMEASURE`, `SELECTEDMEASURENAME`, `ISSELECTEDMEASURE`,
    /// `SELECTEDMEASUREFORMATSTRING`.
    pub(super) fn selected_measure(&mut self, up: &str, args: &[Option<Expr>], s: &Scope) -> Result<V, Fail> {
        let Some(def) = self.selected.last().cloned() else {
            return dax_error(
                "there is no measure reference in the current context to be applied as SELECTEDMEASURE",
            );
        };
        match up {
            "SELECTEDMEASURE" => match self.measure_with_items(&def, s.fc, s)? {
                Some(v) => Ok(v),
                None => self.measure_body(&def, s.fc, s),
            },
            "SELECTEDMEASURENAME" => Ok(V::Text(def.name.clone())),
            "SELECTEDMEASUREFORMATSTRING" => self.format_of(&def, s.fc, s),
            _ => {
                let mut hit = false;
                for i in 0..args.len() {
                    match arg(args, i) {
                        Some(Expr::Column { name, .. }) => hit |= name.eq_ignore_ascii_case(&def.name),
                        _ => return unsupported("ISSELECTEDMEASURE() of something other than a measure"),
                    }
                }
                Ok(V::Bool(hit))
            }
        }
    }
}
