//! `dax.evaluate` (M9.4): runs a DAX query (`DEFINE … EVALUATE … ORDER BY … START AT`) over the
//! stored data of the open file with the viewer's DAX evaluator ([`super`]), and returns the rows
//! of each `EVALUATE` with their column names and types, as DAX Studio shows them.
//!
//! Data: a PBIX's `DataModel`, a PBIP's `.pbi/cache.abf` (unless the caller passes
//! `allow_cache_abf: false`), then the tables' own partitions (M queries run by bicraft's Power
//! Query evaluator, DAX literal tables). The decoded columns are shared with the viewer's cache.
//!
//! The query is first validated against the model (`dax.validate_query`); an invalid query is
//! refused with its diagnostics. Query definitions:
//! - `MEASURE T[m]` is added to a copy of the model (replacing a model measure of that name);
//! - `COLUMN T[c]` is computed once per row of `T` before the first `EVALUATE`;
//! - `VAR` and `TABLE` are evaluated in order, in an empty filter context, and visible to the
//!   `EVALUATE` statements (not to measures);
//! - `FUNCTION` (user-defined functions) joins the model's functions for the query (replacing
//!   one of the same name); calls of user-defined functions are expanded in place before the
//!   query runs (`bicraft_dax::udf::inline_calls`: `val` parameters as variables, `expr`
//!   parameters as the argument's text).
//!
//! What the evaluator does not support is an error naming the function or construct; a result is
//! never approximated. Work is bounded by `timeout_ms`, a cap on row visits, on grouping
//! combinations, on table sizes and on the number of rows returned (`max_rows`).

use std::borrow::Cow;
use std::cmp::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bicraft_dax::ast::{Define, EvaluateStatement, Expr};
use bicraft_dax::{Context, Severity};
use bicraft_model::{DataType, Measure, SemanticModel};
use bicraft_vertipaq::CellValue;
use serde_json::{Map, Value, json};

use super::super::store::{ColData, Miss};
use super::super::{Fail, PreviewLimits, State, cache_abf};
use super::{
    Bind, ColInfo, Engine, Env, FCtx, RowB, Scope, TCol, Tab, V, compare_vals, currency, push_row, ymd,
};
use crate::params::{allow, opt_bool, opt_str, str_param};
use crate::{Session, Source};

/// Rows returned per `EVALUATE` unless `max_rows` says otherwise.
const DEFAULT_MAX_ROWS: u64 = 10_000;
/// Largest `max_rows`.
const MAX_ROWS: u64 = 1_000_000;
/// Time budget unless `timeout_ms` says otherwise.
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
/// Largest `timeout_ms`.
const MAX_TIMEOUT_MS: u64 = 600_000;
/// Most row visits for one query (a storage scan counts its rows once).
const MAX_WORK: u64 = 200_000_000_000;
/// Most grouping combinations one `SUMMARIZECOLUMNS` evaluates.
const MAX_CANDIDATES: usize = 50_000_000;
/// Largest table read (rows): codes and row numbers are 32-bit.
const MAX_TABLE_ROWS: usize = 2_000_000_000;
/// Most cells kept across all the results of one query.
const MAX_CELLS: usize = 20_000_000;
/// Longest query text.
const MAX_QUERY_BYTES: usize = 1 << 20;
/// How every error caused by data that cannot be read starts (`project.test` skips on it).
pub(crate) const DATA_UNAVAILABLE: &str = "data not available";

#[cfg(test)]
thread_local! {
    /// Correctness tests on this thread evaluate without a wall-clock budget (only the
    /// deterministic row-visit cap), so a busy machine cannot fail them.
    static NO_TIME_BUDGET: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
use std::cell::Cell;

/// Runs `f` with no time budget for the `dax.evaluate` calls it makes on this thread.
#[cfg(test)]
pub(crate) fn without_time_budget<R>(f: impl FnOnce() -> R) -> R {
    let before = NO_TIME_BUDGET.with(|c| c.replace(true));
    let r = f();
    NO_TIME_BUDGET.with(|c| c.set(before));
    r
}

fn uint_param(p: &Value, key: &str, default: u64, max: u64) -> Result<u64, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => match v.as_u64() {
            Some(n) if (1..=max).contains(&n) => Ok(n),
            _ => Err(format!("`{key}` must be an integer from 1 to {max}")),
        },
    }
}

/// Error text for a failed evaluation.
fn fail_text(f: Fail, timeout_ms: u64) -> String {
    match f {
        Fail::Unsupported(r) if r == crate::CANCELLED => r,
        Fail::Unsupported(r) if r.contains("more work than the preview allows") => format!(
            "the query needs more than its budget (timeout_ms {timeout_ms}, or the cap of {MAX_WORK} row visits)"
        ),
        Fail::Unsupported(r) => format!(
            "not supported by bicraft's DAX evaluator: {}",
            r.replace("is not evaluated by the preview", "is not supported")
                .replace(
                    "is not a value the preview evaluates",
                    "is not a value bicraft evaluates"
                )
        ),
        Fail::Miss(Miss::Unavailable(r)) => format!("{DATA_UNAVAILABLE}: {r}"),
        Fail::Miss(Miss::Error(r)) => r,
    }
}

/// A data error when `.pbi/cache.abf` was not read says so.
fn noted(mut t: String, blocked: bool) -> String {
    if blocked && t.starts_with(DATA_UNAVAILABLE) {
        t.push_str(" (.pbi/cache.abf was not read: allow_cache_abf is false)");
    }
    t
}

/// Where the query's data comes from (for the result).
fn data_source(s: &Session, allow_abf: bool) -> &'static str {
    match &s.source {
        Some(Source::Package { package, .. }) if package.get(bicraft_pbix::DATA_MODEL).is_some() => {
            "DataModel"
        }
        Some(Source::Project { .. }) if crate::refresh::active_snapshot(s).is_some() => "bicraft-data",
        Some(Source::Project { .. }) if allow_abf && cache_abf(s).is_some() => "cache.abf",
        _ => "partitions",
    }
}

/// DAX sort order: blank first, then numbers (and dates), text (without case), booleans.
fn sort_cmp(a: &V, b: &V) -> Ordering {
    let rank = |v: &V| match v {
        V::Blank => 0,
        V::Num(_) | V::Cur(_) | V::Date(_) => 1,
        V::Text(_) => 2,
        V::Bool(_) => 3,
    };
    match rank(a).cmp(&rank(b)) {
        Ordering::Equal => compare_vals(a, b).unwrap_or(Ordering::Equal),
        o => o,
    }
}

/// `YYYY-MM-DDTHH:MM:SS[.fff]` of a serial date.
fn iso_date(x: f64) -> Value {
    if !x.is_finite() || x.abs() > 3_000_000.0 {
        return json!(x);
    }
    let day = x.floor();
    let ms = ((x - day) * 86_400_000.0).round() as i64;
    let (day, ms) = if ms >= 86_400_000 {
        (day as i64 + 1, 0)
    } else {
        (day as i64, ms)
    };
    let (y, m, d) = ymd(day);
    let (h, mi, s, f) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    let mut out = format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}");
    if f != 0 {
        out.push_str(&format!(".{f:03}"));
    }
    Value::String(out)
}

fn num_json(x: f64, int: bool) -> Value {
    if !x.is_finite() {
        return Value::String(
            if x.is_nan() {
                "NaN"
            } else if x > 0.0 {
                "Infinity"
            } else {
                "-Infinity"
            }
            .into(),
        );
    }
    if int && x.fract() == 0.0 && x.abs() < 9.0e15 {
        return json!(x as i64);
    }
    json!(x)
}

fn cell_json(v: &V, ty: &str) -> Value {
    match v {
        V::Blank => Value::Null,
        V::Num(x) => num_json(*x, ty == "int64"),
        V::Cur(n) => num_json(currency::to_f64(*n), false),
        V::Date(x) => iso_date(*x),
        V::Text(t) => Value::String(t.clone()),
        V::Bool(b) => Value::Bool(*b),
    }
}

fn type_of_values(rows: &[Vec<V>], i: usize) -> &'static str {
    let mut ty: Option<&'static str> = None;
    for r in rows {
        let t = match r.get(i) {
            None | Some(V::Blank) => continue,
            Some(V::Num(_)) => "double",
            Some(V::Cur(_)) => "decimal",
            Some(V::Date(_)) => "dateTime",
            Some(V::Text(_)) => "string",
            Some(V::Bool(_)) => "boolean",
        };
        ty = Some(match ty {
            None => t,
            Some(prev) if prev == t => t,
            Some("decimal" | "double") if matches!(t, "decimal" | "double") => "double",
            _ => "variant",
        });
    }
    ty.unwrap_or("unknown")
}

/// The model with the query's measures added (replacing model measures of the same name): the
/// session's model itself, not a copy, when the query defines none.
fn query_model<'m>(
    model: &'m SemanticModel,
    text: &str,
    defines: &[Define],
) -> Result<Cow<'m, SemanticModel>, String> {
    if !defines
        .iter()
        .any(|d| matches!(d, Define::Measure { .. } | Define::Function { .. }))
    {
        return Ok(Cow::Borrowed(model));
    }
    let mut m = model.clone();
    for d in defines {
        match d {
            Define::Measure {
                table,
                name,
                expr_span,
                ..
            } => {
                let body = text
                    .get(expr_span.start..expr_span.end)
                    .ok_or_else(|| format!("MEASURE {table}[{name}]: bad expression span"))?;
                // The model's own definition, as a Define quick query writes it: nothing to
                // replace (model measures that use it see the same expression either way).
                let same = model
                    .tables
                    .iter()
                    .find(|t| t.name.eq_ignore_ascii_case(table))
                    .is_some_and(|t| {
                        t.measures
                            .iter()
                            .any(|x| x.name.eq_ignore_ascii_case(name) && x.expression.trim() == body.trim())
                    });
                if same {
                    continue;
                }
                // A query measure replaces a model measure for the query's own references only:
                // model measures keep using the model's (Desktop 2.158). Refused when one does.
                let needle = format!("[{}]", name.replace(']', "]]")).to_lowercase();
                let user = model.tables.iter().flat_map(|t| &t.measures).find(|x| {
                    !x.name.eq_ignore_ascii_case(name) && x.expression.to_lowercase().contains(&needle)
                });
                if let Some(user) = user {
                    return Err(format!(
                        "not supported by bicraft's DAX evaluator: MEASURE {table}[{name}] redefines a \
                         model measure that model measure [{}] uses (it would keep the model's \
                         definition); give the query measure another name",
                        user.name
                    ));
                }
                for t in &mut m.tables {
                    t.measures.retain(|x| !x.name.eq_ignore_ascii_case(name));
                }
                let Some(t) = m.tables.iter_mut().find(|t| t.name.eq_ignore_ascii_case(table)) else {
                    return Err(format!(
                        "MEASURE {table}[{name}]: no table '{table}' in the model"
                    ));
                };
                t.measures.push(Measure {
                    name: name.clone(),
                    expression: body.to_string(),
                    ..Default::default()
                });
            }
            Define::Function {
                name,
                name_span,
                span,
                ..
            } => {
                // `FUNCTION Name = ( params ) => body`: the text after `=` is the definition, as
                // a model function stores it; the query's replaces the model's of that name
                let after = text.get(name_span.end..span.end).unwrap_or_default();
                let def = after.trim_start().strip_prefix('=').unwrap_or(after).trim();
                let f = json!({"name": name, "expression": def});
                match m.extra.get_mut("functions") {
                    Some(Value::Array(a)) => {
                        a.retain(|x| {
                            !x.get("name")
                                .and_then(Value::as_str)
                                .is_some_and(|n| n.eq_ignore_ascii_case(name))
                        });
                        a.push(f);
                    }
                    _ => {
                        m.extra.insert("functions".into(), Value::Array(vec![f]));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(Cow::Owned(m))
}

/// The query text with the calls of user-defined functions expanded in every definition, `EVALUATE`
/// and `ORDER BY` expression ([`bicraft_dax::udf::inline_calls`]); `None` when it calls none.
fn expand_query(
    text: &str,
    q: &bicraft_dax::ast::Query,
    qm: &SemanticModel,
) -> Result<Option<String>, String> {
    if bicraft_dax::udf::model_function_names(qm).is_empty() {
        return Ok(None);
    }
    let mut spans: Vec<bicraft_dax::Span> = Vec::new();
    for d in &q.defines {
        match d {
            Define::Measure { expr_span, .. }
            | Define::Column { expr_span, .. }
            | Define::Var { expr_span, .. }
            | Define::Table { expr_span, .. } => spans.push(*expr_span),
            Define::Function { .. } => {}
        }
    }
    for st in &q.statements {
        spans.push(st.expr_span);
        spans.extend(st.order_by.iter().map(|k| k.expr.span()));
    }
    spans.sort_by_key(|s| s.start);
    let mut out = String::with_capacity(text.len());
    let mut at = 0usize;
    let mut changed = false;
    for s in spans {
        if s.start < at {
            continue;
        }
        let Some(slice) = text.get(s.start..s.end) else {
            continue;
        };
        if let Some(x) = bicraft_dax::udf::inline_calls(slice, qm)
            .map_err(|e| format!("not supported by bicraft's DAX evaluator: {e}"))?
        {
            out.push_str(text.get(at..s.start).unwrap_or_default());
            out.push_str(&x);
            at = s.end;
            changed = true;
        }
    }
    if !changed {
        return Ok(None);
    }
    out.push_str(text.get(at..).unwrap_or_default());
    Ok(Some(out))
}

/// Runs a query; see the module docs.
pub(crate) fn dax_evaluate(s: &Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "query",
            "max_rows",
            "timeout_ms",
            "allow_cache_abf",
            "as_of",
            "utc_offset_minutes",
        ],
    )?;
    let text = str_param(p, "query")?;
    // stopped by a `dax.cancel` made after this point
    let _running = crate::dax_queries::Running::start(s);
    if text.len() > MAX_QUERY_BYTES {
        return Err(format!("the query is longer than {MAX_QUERY_BYTES} bytes"));
    }
    let max_rows = uint_param(p, "max_rows", DEFAULT_MAX_ROWS, MAX_ROWS)?;
    let max_rows = usize::try_from(max_rows).unwrap_or(usize::MAX);
    let timeout_ms = uint_param(p, "timeout_ms", DEFAULT_TIMEOUT_MS, MAX_TIMEOUT_MS)?;
    let allow_abf = opt_bool(p, "allow_cache_abf")?.unwrap_or(true);
    let as_of = opt_str(p, "as_of")?
        .map(super::super::filters::parse_as_of)
        .transpose()?;
    let s: &Session = s;
    // `utc_offset_minutes`: local time minus UTC, what separates NOW() from UTCNOW()
    let given = match p.get("utc_offset_minutes") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_i64() {
            Some(m) if (-1080..=1080).contains(&m) => Some(m as f64 / 1440.0),
            _ => return Err("`utc_offset_minutes` must be an integer from -1080 to 1080".into()),
        },
    };
    use super::super::filters;
    let (now, utc_offset) = match given {
        // the system clock is UTC: local time is UTC plus the offset
        Some(off) => (
            as_of
                .or_else(|| s.as_of_now())
                .unwrap_or_else(|| filters::now_utc() + off),
            off,
        ),
        // without an offset, the process's local clock (the machine's time zone in the desktop
        // app and the CLI, UTC or `--local-offset` in `bicraft-cli serve`)
        None => match as_of {
            Some(t) => (t, filters::offset_days_at(t)),
            None => (filters::clock(s), filters::utc_offset(s)),
        },
    };
    let Some(model) = s.model.as_ref() else {
        return Err(match &s.model_error {
            Some(e) => format!("the open file's model cannot be read: {e}"),
            None => "no model is open (file.open first)".into(),
        });
    };
    // validated and parsed in one pass (the tree is the one `parse_query` returns)
    let (v, parsed) = bicraft_dax::validate_query_parsed(
        text,
        &Context {
            model: Some(model),
            table: None,
        },
    );
    let errors: Vec<String> = v
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| format!("{}:{}: {} [{}]", d.line, d.column, d.message, d.code))
        .collect();
    if !v.valid || !errors.is_empty() {
        return Err(format!("the query is not valid: {}", errors.join("; ")));
    }
    let q = parsed.ok_or_else(|| "the query does not parse".to_string())?;
    let qm = query_model(model, text, &q.defines)?;
    // calls of user-defined functions (the model's and the query's) expanded in place
    let owned;
    let (text, q, qm) = match expand_query(text, &q, &qm)? {
        None => (text, q, qm),
        Some(t) => {
            owned = t;
            let (q2, diags) = bicraft_dax::parse_query(&owned);
            let q2 = q2
                .filter(|_| diags.iter().all(|d| d.severity != Severity::Error))
                .ok_or("internal: the query with its user-defined functions expanded does not parse")?;
            let qm2 = query_model(model, &owned, &q2.defines)?;
            (owned.as_str(), q2, qm2)
        }
    };

    let started = Instant::now();
    let timeout = Duration::from_millis(timeout_ms);
    #[cfg(test)]
    let timeout = if NO_TIME_BUDGET.with(Cell::get) {
        Duration::from_secs(365 * 86_400)
    } else {
        timeout
    };
    let limits = PreviewLimits {
        rows_per_visual: max_rows,
        table_rows: MAX_TABLE_ROWS,
        fallback_rows: MAX_TABLE_ROWS,
        budget: timeout,
        now,
        utc_offset,
    };
    let blocked = !allow_abf && matches!(s.source, Some(Source::Project { .. }));
    let mut private;
    let mut guard;
    let st: &mut State = if blocked {
        private = State {
            store: Some(Err(
                "reading .pbi/cache.abf is disabled (allow_cache_abf: false)".to_string()
            )),
            ..State::default()
        };
        &mut private
    } else {
        // the main state, or a fork when another evaluation holds it (`preview_fork.rs`)
        guard = s.preview.acquire(s);
        &mut guard
    };
    #[cfg(test)]
    super::super::test_gate::pass(text);
    let mut ctx = super::super::Ctx {
        s,
        st,
        model: &qm,
        limits: &limits,
    };
    let failures = bicraft_scan::lazy::load_failures();
    let mut eng = Engine::new(&mut ctx);
    eng.max_time = timeout;
    eng.max_work = MAX_WORK;
    eng.max_candidates = MAX_CANDIDATES;
    // ALLSELECTED restores the shadow filter context of the innermost iterator or
    // SUMMARIZECOLUMNS; tracking it costs a filter per iteration, so only when it is used.
    // (what the measures say is kept by the session for its own model)
    eng.shadow = text.to_ascii_uppercase().contains("ALLSELECTED") || eng.measure_facts().allselected;

    let fc = FCtx::default();
    let mut env = Env::new();
    for d in &q.defines {
        let scope = Scope {
            fc: &fc,
            rc: &[],
            env: &env,
            depth: 0,
            mdepth: 0,
        };
        let bound = match d {
            Define::Var { name, expr, .. } => {
                // a `FILTER` too: used as a filter, it can be read from codes without its table
                let lazy = eng.store.is_some()
                    && matches!(expr, Expr::Call { name, .. }
                        if name.eq_ignore_ascii_case("SUMMARIZECOLUMNS") || name.eq_ignore_ascii_case("FILTER"));
                let b = if lazy {
                    // evaluated on first use: a `TOPN` over it builds only the rows it keeps
                    Ok(Bind::L(std::rc::Rc::new(super::topn::Lazy {
                        expr: expr.clone(),
                        env: env.clone(),
                        cell: std::cell::OnceCell::new(),
                    })))
                } else if eng.is_table_expr(expr, &env) {
                    eng.table(expr, &scope).map(Bind::T)
                } else {
                    eng.scalar(expr, &scope).map(Bind::S)
                };
                Some((name, b))
            }
            Define::Table { name, expr, .. } => {
                Some((name, query_table(&mut eng, expr, &scope).map(Bind::T)))
            }
            Define::Column {
                table, name, expr, ..
            } => {
                define_column(&mut eng, table, name, expr, &env).map_err(|f| {
                    format!(
                        "COLUMN {table}[{name}]: {}",
                        noted(fail_text(f, timeout_ms), blocked)
                    )
                })?;
                None
            }
            Define::Measure { .. } | Define::Function { .. } => None,
        };
        if let Some((name, b)) = bound {
            let b = b.map_err(|f| {
                let t = noted(fail_text(f, timeout_ms), blocked);
                format!("{} {name}: {t}", d.kind().to_uppercase())
            })?;
            env.push((name.to_lowercase(), b));
        }
    }

    let mut results = Vec::with_capacity(q.statements.len());
    let mut cells = 0usize;
    let many = q.statements.len() > 1;
    for (k, stmt) in q.statements.iter().enumerate() {
        let r = evaluate_statement(&mut eng, stmt, &env, max_rows, &mut cells).map_err(|f| {
            let t = noted(fail_text(f, timeout_ms), blocked);
            if many {
                format!("EVALUATE {}: {t}", k.saturating_add(1))
            } else {
                t
            }
        })?;
        results.push(r);
    }
    drop(eng);
    // codes that could not be unpacked under the memory limit: no answer from them (lot 5)
    if let Some(e) = ctx.st.memory.unpack_refused.take() {
        return Err(e);
    }
    // a column segment read on demand failed meanwhile: no answer from it (lot 3)
    if bicraft_scan::lazy::load_failures() != failures {
        ctx.st.storage = None;
        return Err(super::super::SEGMENT_READ_FAILED.into());
    }
    // built by hand: `json!` would copy the results (it serializes its values)
    let mut o = Map::with_capacity(3);
    o.insert("results".into(), Value::Array(results));
    o.insert("data_source".into(), json!(data_source(s, allow_abf)));
    o.insert(
        "elapsed_ms".into(),
        json!(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
    );
    Ok(Value::Object(o))
}

/// A query `TABLE`: a new table, like a calculated table, so its columns lose the lineage of
/// the model columns they come from (`SUMX(T, Sales[Amount])` over it fails as in the engine).
fn query_table(eng: &mut Engine<'_, '_>, expr: &Expr, scope: &Scope<'_>) -> Result<Tab, Fail> {
    let t = eng.table(expr, scope)?;
    let cols: Vec<TCol> = eng
        .tab_cols(&t)
        .iter()
        .map(|c| {
            TCol::named(
                eng.tcol_name(c),
                c.ty.or_else(|| c.lin.and_then(|l| eng.col_type(l))),
            )
        })
        .collect();
    let rows = eng.tab_values(&t)?;
    Ok(Tab::Tuples {
        cols: Arc::new(cols),
        rows,
    })
}

/// Adds the query column `table[name]` to the engine and computes it row by row.
fn define_column(
    eng: &mut Engine<'_, '_>,
    table: &str,
    name: &str,
    expr: &Expr,
    env: &Env,
) -> Result<(), Fail> {
    let Some(t) = eng.table_id(table) else {
        return super::unsupported(format!("unknown table '{table}'"));
    };
    if eng.col_id(table, name).is_some() {
        return super::dax_error(format!("the table already has a column named [{name}]"));
    }
    let n = eng.rows(t)?;
    if let Some(s) = &mut eng.store {
        s.private = true;
    }
    let ci = eng.cols.len();
    eng.cols.push(ColInfo {
        table: t,
        name: name.to_string(),
        date: false,
        row_number: false,
        data: None,
        index: None,
        nums: None,
        ranks: None,
        dates: None,
    });
    if let Some(info) = eng.tabs.get_mut(t) {
        info.cols.push(ci);
    }
    Arc::make_mut(&mut eng.col_ix).insert((table.to_lowercase(), name.to_lowercase()), ci);
    let fc = FCtx::default();
    let mut values = Vec::with_capacity(n);
    for row in 0..n {
        let Ok(row) = u32::try_from(row) else {
            return super::unsupported("a table with too many rows");
        };
        eng.tick(1)?;
        let rc = [RowB::Row { table: t, row }];
        let scope = Scope {
            fc: &fc,
            rc: &rc,
            env,
            depth: 0,
            mdepth: 0,
        };
        values.push(eng.scalar(expr, &scope)?.to_cell());
    }
    let date = values.iter().any(|v| matches!(v, CellValue::DateTime(_)))
        && values
            .iter()
            .all(|v| matches!(v, CellValue::DateTime(_) | CellValue::Null));
    // The type the expression has (as the engine infers it), else the values'.
    let ty = eng.static_type(expr, 0).or_else(|| {
        let mut ty = None;
        for v in &values {
            let t = match v {
                CellValue::Null => continue,
                CellValue::Int(_) => DataType::Int64,
                CellValue::Real(_) => DataType::Double,
                CellValue::Decimal(_) => DataType::Decimal,
                CellValue::DateTime(_) => DataType::DateTime,
                CellValue::Text(_) => DataType::String,
                CellValue::Bool(_) => DataType::Boolean,
            };
            match ty {
                None => ty = Some(t),
                Some(p) if p == t => {}
                Some(_) => return None,
            }
        }
        ty
    });
    if let Some(t) = ty {
        eng.col_types.insert(ci, t);
    }
    let data = ColData::from_stored(values).map_err(Fail::Miss)?;
    if let Some(info) = eng.cols.get_mut(ci) {
        info.data = Some(Arc::new(data));
        info.date = date;
    }
    Ok(())
}

/// The result name of a column: `Table[Column]` for a model column, `[Name]` otherwise.
fn result_name(eng: &Engine<'_, '_>, c: &TCol) -> String {
    match (&c.name, c.lin.and_then(|l| eng.cols.get(l))) {
        (None, Some(info)) => {
            let t = eng.tabs.get(info.table).map_or("", |t| t.name.as_str());
            format!("{t}[{}]", info.name)
        }
        _ => format!("[{}]", eng.tcol_name(c)),
    }
}

fn type_name(t: DataType) -> &'static str {
    t.as_tmsl().unwrap_or("unknown")
}

/// The result column an `ORDER BY` key reads, when evaluating the key in the row context of a
/// result row would just return that row's value: the same lookups as [`Engine::scalar`] for a
/// column reference over a row of tuples, in its order (an added column `[name]`; a model column
/// `T[c]` that no measure shadows, at its first place in the row). `None` evaluates the key.
fn result_column_key(eng: &mut Engine<'_, '_>, tab: &Tab, key: &Expr) -> Option<usize> {
    let Tab::Tuples { cols, .. } = tab else {
        return None;
    };
    let Expr::Column {
        table,
        name,
        variation: None,
        ..
    } = key
    else {
        return None;
    };
    match table {
        None => {
            // `ext_value`: the first column of that name, when it is not a model column
            let i = cols
                .iter()
                .position(|c| eng.tcol_name(c).eq_ignore_ascii_case(name))?;
            cols.get(i).filter(|c| c.lin.is_none()).map(|_| i)
        }
        Some(t) => {
            let c = match eng.refs.get(&(key as *const Expr as usize)).copied() {
                Some(c) => c,
                None => {
                    if eng.measure_def(Some(t), name).is_some() {
                        return None;
                    }
                    eng.col_id(t, name)?
                }
            };
            cols.iter().position(|x| x.lin == Some(c))
        }
    }
}

/// Under a memory cap, an `EVALUATE SUMMARIZECOLUMNS(...) ORDER BY ...` whose keys are its group-by
/// columns or items, cut by `max_rows`: the selection of `TOPN(max_rows, ..., keys)` (only the kept
/// groups' rows are built; ties with the last kept row come too, and the caller takes `max_rows`
/// of them in order), with the number of rows of the whole grouping. `None`: another shape, or a
/// column whose type would be read from the rows left out.
fn streamed(
    eng: &mut Engine<'_, '_>,
    stmt: &EvaluateStatement,
    scope: &Scope<'_>,
    max_rows: usize,
) -> Result<Option<(Tab, usize)>, Fail> {
    if !eng.mem.on() || stmt.order_by.is_empty() || !stmt.start_at.is_empty() {
        return Ok(None);
    }
    let keys: Vec<(&Expr, bool)> = stmt.order_by.iter().map(|k| (&k.expr, k.descending)).collect();
    let Some((t, n)) = eng.top_n_grouping(max_rows as f64, &stmt.expr, &keys, scope, true)? else {
        return Ok(None);
    };
    let declared = eng.tab_cols(&t).iter().all(|c| {
        c.ty.or_else(|| c.lin.and_then(|l| eng.col_type(l)))
            .is_some_and(|t| t != DataType::Unknown)
    });
    Ok(declared.then_some((t, n)))
}

fn evaluate_statement(
    eng: &mut Engine<'_, '_>,
    stmt: &EvaluateStatement,
    env: &Env,
    max_rows: usize,
    cells: &mut usize,
) -> Result<Value, Fail> {
    let fc = FCtx::default();
    let scope = Scope {
        fc: &fc,
        rc: &[],
        env,
        depth: 0,
        mdepth: 0,
    };
    // under a memory cap, a grouping cut by `max_rows` is selected without building every row
    let (tab, total) = match streamed(eng, stmt, &scope, max_rows)? {
        Some((t, n)) => (t, Some(n)),
        None => (eng.table(&stmt.expr, &scope)?, None),
    };
    let cols = eng.tab_cols(&tab);
    let rows = eng.tab_values(&tab)?;
    let nr = rows.len();
    let _order_charge = eng.charge_bytes(
        &move || format!("the order of the answer's {nr} rows"),
        nr.saturating_mul(std::mem::size_of::<usize>()),
    )?;
    let mut order: Vec<usize> = (0..rows.len()).collect();

    // ORDER BY: each key evaluated in the row context of the result row. A key that names a
    // column of the result (a grouped column `T[c]`, or an added column `[name]`) is read in
    // place, as the evaluator would read it from the row, instead of copying every row.
    // (rows a streamed selection returns are in order already)
    if total.is_none() && (!stmt.order_by.is_empty() || !stmt.start_at.is_empty()) {
        let src: Vec<Option<usize>> = stmt
            .order_by
            .iter()
            .map(|k| result_column_key(eng, &tab, &k.expr))
            .collect();
        let evaluated = src.iter().filter(|s| s.is_none()).count();
        let mut keys: Vec<Vec<V>> = Vec::new();
        let mut _keys_charge = super::mem::Charge::default();
        if evaluated > 0 || stmt.order_by.is_empty() {
            _keys_charge = eng.charge_bytes(
                &move || format!("the ORDER BY keys of {nr} rows"),
                nr.saturating_mul(super::mem::row_bytes(evaluated)),
            )?;
            keys.reserve(rows.len());
            for i in 0..tab.len() {
                let mut rc = Vec::new();
                push_row(&tab, i, &mut rc);
                let inner = Scope { rc: &rc, ..scope };
                let mut k = Vec::with_capacity(evaluated);
                for (key, s) in stmt.order_by.iter().zip(&src) {
                    if s.is_none() {
                        k.push(eng.scalar(&key.expr, &inner)?);
                    }
                }
                keys.push(k);
            }
        }
        // slot of each key: a result column, or its place among the evaluated keys
        let mut slots: Vec<(bool, usize, Option<usize>)> = Vec::with_capacity(src.len());
        let mut e = 0usize;
        for (key, s) in stmt.order_by.iter().zip(&src) {
            slots.push((key.descending, e, *s));
            if s.is_none() {
                e = e.saturating_add(1);
            }
        }
        let blank = V::Blank;
        let key_of = |i: usize, k: usize| -> &V {
            match slots.get(k) {
                Some((_, _, Some(j))) => rows.get(i).and_then(|r| r.get(*j)).unwrap_or(&blank),
                Some((_, e, None)) => keys.get(i).and_then(|r| r.get(*e)).unwrap_or(&blank),
                None => &blank,
            }
        };
        let desc_cmp = |k: usize, o: Ordering| -> Ordering {
            if slots.get(k).is_some_and(|s| s.0) {
                o.reverse()
            } else {
                o
            }
        };
        let rows_known = |i: usize| i < rows.len() && (keys.is_empty() || i < keys.len());
        order.sort_by(|a, b| {
            // a row without keys (none expected) sorts as having no keys: equal to every row
            if !rows_known(*a) || !rows_known(*b) {
                return Ordering::Equal;
            }
            for k in 0..slots.len() {
                let o = desc_cmp(k, sort_cmp(key_of(*a, k), key_of(*b, k)));
                if o != Ordering::Equal {
                    return o;
                }
            }
            Ordering::Equal
        });
        // START AT: the first row whose keys are not before the given values.
        if !stmt.start_at.is_empty() {
            if stmt.start_at.len() > stmt.order_by.len() {
                return super::dax_error("START AT has more values than ORDER BY has keys");
            }
            let mut start = Vec::with_capacity(stmt.start_at.len());
            for e in &stmt.start_at {
                if let Expr::Name { name, .. } = e
                    && name.starts_with('@')
                {
                    return super::unsupported(format!("query parameter {name} in START AT"));
                }
                start.push(eng.scalar(e, &scope)?);
            }
            let first = order
                .iter()
                .position(|i| {
                    if !rows_known(*i) {
                        return true;
                    }
                    for (k, s) in start.iter().enumerate() {
                        let o = desc_cmp(k, sort_cmp(key_of(*i, k), s));
                        if o != Ordering::Equal {
                            return o != Ordering::Less;
                        }
                    }
                    true
                })
                .unwrap_or(order.len());
            order.drain(..first);
        }
    }

    // column names and types
    let mut names: Vec<String> = Vec::with_capacity(cols.len());
    let mut types: Vec<&'static str> = Vec::with_capacity(cols.len());
    for (i, c) in cols.iter().enumerate() {
        let mut name = result_name(eng, c);
        let base = name.clone();
        let mut n = 2usize;
        while names.contains(&name) {
            name = format!("{base} ({n})");
            n = n.saturating_add(1);
        }
        names.push(name);
        let declared = c.ty.or_else(|| c.lin.and_then(|l| eng.col_type(l)));
        types.push(match declared {
            Some(t) if t != DataType::Unknown => type_name(t),
            _ => type_of_values(&rows, i),
        });
    }

    let row_count = total.unwrap_or(order.len());
    let take = order.len().min(max_rows);
    *cells = cells.saturating_add(take.saturating_mul(cols.len().max(1)));
    if *cells > MAX_CELLS {
        return super::unsupported(format!(
            "the results hold more than {MAX_CELLS} cells (lower max_rows)"
        ));
    }
    // the answer's rows, reserved before they are built (kept until the end of the query)
    if eng.mem.on() {
        let per_row = 64usize.saturating_add(names.iter().map(|n| n.len().saturating_add(80)).sum::<usize>());
        let text: usize = order
            .iter()
            .take(take)
            .filter_map(|i| rows.get(*i))
            .flatten()
            .map(super::mem::text_bytes)
            .sum();
        let c = eng.charge_bytes(
            &move || format!("the answer's {take} rows (lower max_rows)"),
            take.saturating_mul(per_row).saturating_add(text),
        )?;
        eng.mem.keep(c);
    }
    let mut out = Vec::with_capacity(take);
    for i in order.iter().take(take) {
        let Some(r) = rows.get(*i) else { continue };
        let mut o = Map::with_capacity(names.len());
        for (j, name) in names.iter().enumerate() {
            let ty = types.get(j).copied().unwrap_or("unknown");
            o.insert(name.clone(), r.get(j).map_or(Value::Null, |v| cell_json(v, ty)));
        }
        out.push(Value::Object(o));
    }
    let columns: Vec<Value> = names
        .iter()
        .zip(&types)
        .map(|(n, t)| json!({"name": n, "type": t}))
        .collect();
    // built by hand: `json!` would copy the rows (it serializes its values)
    let mut o = Map::with_capacity(4);
    o.insert("columns".into(), Value::Array(columns));
    o.insert("rows".into(), Value::Array(out));
    o.insert("row_count".into(), json!(row_count));
    o.insert("truncated".into(), Value::Bool(take < row_count));
    Ok(Value::Object(o))
}
