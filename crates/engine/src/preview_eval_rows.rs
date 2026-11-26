//! Calculated columns the stored data does not hold, evaluated for the Data view's rows
//! (`table.rows`, `preview_rows.rs`) with the viewer's DAX evaluator ([`super`]): the expression
//! once per asked row, with that row as the row context and no filter, as a calculated column is
//! defined (and as `preview_eval_struct.rs` computes one for a structural save).

use std::time::{Duration, Instant};

use bicraft_vertipaq::CellValue;

use super::super::{Ctx, Fail, Miss};
use super::{Engine, Env, FCtx, RowB, Scope};

fn fail_text(f: Fail) -> String {
    match f {
        Fail::Unsupported(r) => format!("not supported by bicraft's DAX evaluator: {r}"),
        Fail::Miss(Miss::Unavailable(r)) => format!("{}: {r}", super::query::DATA_UNAVAILABLE),
        Fail::Miss(Miss::Error(r)) => r,
    }
}

/// `values` as a column of type `ty` holds them: a number in a date/time column is the date it
/// stands for (Desktop's day, hour, minute and second bins compute serial numbers into a
/// date/time column, docs/formats/groups.md).
pub(crate) fn as_declared(mut values: Vec<CellValue>, ty: bicraft_model::DataType) -> Vec<CellValue> {
    if ty == bicraft_model::DataType::DateTime {
        for v in &mut values {
            match *v {
                CellValue::Int(n) => *v = CellValue::DateTime(n as f64),
                CellValue::Real(x) => *v = CellValue::DateTime(x),
                _ => {}
            }
        }
    }
    values
}

/// The values of calculated column `expression` of `table` at `rows` (row indexes of the
/// table), in that order; an error names the first row that fails. `budget` caps the whole run.
pub(crate) fn calc_rows(
    ctx: &mut Ctx<'_>,
    table: &str,
    expression: &str,
    rows: &[u32],
    budget: Duration,
) -> Result<Vec<CellValue>, String> {
    let (expr, diags) = bicraft_dax::parse_expression(expression);
    if let Some(d) = diags.iter().find(|d| d.severity == bicraft_dax::Severity::Error) {
        return Err(format!("the expression does not parse: {}", d.message));
    }
    let expr = expr.ok_or_else(|| "the expression does not parse".to_string())?;
    let mut eng = Engine::new(ctx);
    let t = eng
        .table_id(table)
        .ok_or_else(|| format!("table '{table}' is not in the model"))?;
    let n = eng.rows(t).map_err(fail_text)?;
    let fc = FCtx::default();
    let env = Env::new();
    let start = Instant::now();
    let mut out = Vec::with_capacity(rows.len());
    for &row in rows {
        if usize::try_from(row).map_or(true, |r| r >= n) {
            out.push(CellValue::Null);
            continue;
        }
        if start.elapsed() > budget {
            return Err(format!(
                "computing the column takes more than {} s",
                budget.as_secs()
            ));
        }
        if crate::call_scope::cancelled() {
            return Err("cancelled".into());
        }
        // the evaluator's per-expression budget applies to each row
        eng.work = 0;
        eng.start = Instant::now();
        let rc = [RowB::Row { table: t, row }];
        let scope = Scope {
            fc: &fc,
            rc: &rc,
            env: &env,
            depth: 0,
            mdepth: 0,
        };
        let v = eng
            .scalar(&expr, &scope)
            .map_err(|f| format!("row {}: {}", u64::from(row).saturating_add(1), fail_text(f)))?;
        out.push(v.to_cell());
    }
    Ok(out)
}
