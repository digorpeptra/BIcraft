//! Row-level security for View as (Desktop's Modeling › View as; `view.interact {action:
//! "view_as"}`, `docs/view-snapshot.md` "View as").
//!
//! Under View as every evaluation (visuals, slicers, cards, `dax.evaluate`, `table.rows`,
//! `visual.records`, a slicer's values, the Filters pane's lists) reads the data through the
//! roles' row filters. The rows a table keeps are computed once per (session revision, data,
//! View as state) on the data as stored ([`general::rls::secured_rows`]): each role's DAX filter
//! is evaluated in the row context of each row of its table, the rows kept filter the tables
//! their active relationships reach (always from the one side into the many side, also back
//! when the relationship's `securityFilteringBehavior` is `bothDirections`), and the rows the
//! selected roles keep are united (a role that restricts nothing keeps everything).
//!
//! The evaluations then run on a state of their own (the **secured state**, [`SecuredSlot`]),
//! whose columns hold only those rows: `ALL`, `REMOVEFILTERS` or a relationship's blank row never
//! see the others, as row-level security is applied by the engine before any query. Its caches
//! (results, relationship maps, the storage engine's, `table.rows` orders) are its own and never
//! merge into the main state; report-derived aggregations are off there. A role filter that
//! cannot be evaluated fails every read under View as (never the data unfiltered).

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::fork::{Inherited, StateRef};
use super::store::{Codes, ColData, Derived, Miss};
use super::{Ctx, PreviewCache, PreviewLimits, State, general, memory};
use crate::Session;
use crate::interact::ViewAs;

/// The rows each secured table keeps under one View as state.
#[derive(Debug, Default)]
pub(super) struct Security {
    /// Lower-cased table name → rows kept (ascending); tables not listed keep every row.
    keep: HashMap<String, Arc<Vec<u32>>>,
    /// A role filter could not be evaluated: every read under View as fails with this.
    error: Option<String>,
    /// Object-level security: the lower-cased tables, and (table, column), hidden from every
    /// selected role (`metadataPermission: none`).
    hidden_tables: HashSet<String>,
    hidden_columns: HashSet<(String, String)>,
}

impl Security {
    /// The rows table `table` keeps (`None`: every row); the error of a failed computation.
    pub(super) fn rows_of(&self, table: &str) -> Result<Option<Arc<Vec<u32>>>, Miss> {
        if let Some(e) = &self.error {
            return Err(Miss::Error(e.clone()));
        }
        if self.hidden_tables.contains(&table.to_lowercase()) {
            return Err(Miss::Error(format!(
                "View as: cannot find table '{table}' (object-level security)"
            )));
        }
        Ok(self.keep.get(&table.to_lowercase()).cloned())
    }

    /// Err when object-level security hides column `column` of `table` (or the table) from the
    /// roles: a visual or a measure that reads it fails, as for a column that does not exist.
    pub(super) fn check_column(&self, table: &str, column: &str) -> Result<(), Miss> {
        let t = table.to_lowercase();
        if self.hidden_tables.contains(&t) {
            return Err(Miss::Error(format!(
                "View as: cannot find table '{table}' (object-level security)"
            )));
        }
        if self.hidden_columns.contains(&(t, column.to_lowercase())) {
            return Err(Miss::Error(format!(
                "View as: cannot find column '{table}'[{column}] (object-level security)"
            )));
        }
        Ok(())
    }
}

/// The hidden tables, and the hidden columns as (table, column).
type Hidden = (HashSet<String>, HashSet<(String, String)>);

/// What object-level security hides from the members of `roles`: an object is hidden when every
/// role hides it (a role that does not secure it grants read). `Err` when row-level and
/// object-level security come from different roles, which the engine refuses at query time
/// (Microsoft Learn, "Object-level security", Restrictions).
fn object_security(roles: &[&bicraft_model::Role]) -> Result<Hidden, String> {
    let none = |v: Option<&serde_json::Value>| v.and_then(serde_json::Value::as_str) == Some("none");
    let mut per_role = Vec::with_capacity(roles.len());
    for r in roles {
        let mut tables = HashSet::new();
        let mut columns = HashSet::new();
        for tp in &r.table_permissions {
            let t = tp.table.to_lowercase();
            if none(tp.extra.get("metadataPermission")) {
                tables.insert(t.clone());
            }
            for cp in tp
                .extra
                .get("columnPermissions")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
            {
                if none(cp.get("metadataPermission"))
                    && let Some(c) = cp.get("name").and_then(serde_json::Value::as_str)
                {
                    columns.insert((t.clone(), c.to_lowercase()));
                }
            }
        }
        let rls = r.table_permissions.iter().any(|tp| {
            tp.filter_expression
                .as_deref()
                .is_some_and(|f| !f.trim().is_empty())
        });
        per_role.push((r.name.as_str(), rls, tables, columns));
    }
    let ols_role = per_role
        .iter()
        .find(|x| !x.2.is_empty() || !x.3.is_empty())
        .map(|x| x.0);
    if let Some(o) = ols_role
        && let Some(r) = per_role.iter().find(|x| x.1 && x.0 != o)
    {
        return Err(format!(
            "View as: roles `{}` (row-level security) and `{o}` (object-level security) cannot be \
             combined: row-level and object-level security from different roles are refused",
            r.0
        ));
    }
    let mut it = per_role.into_iter();
    let Some((_, _, mut tables, mut columns)) = it.next() else {
        return Ok((HashSet::new(), HashSet::new()));
    };
    for (_, _, t, c) in it {
        tables.retain(|x| t.contains(x));
        columns.retain(|x| c.contains(x));
    }
    Ok((tables, columns))
}

/// The secured state and what it was built for: (revision, data fingerprint, data generation,
/// View as state).
#[derive(Default)]
pub(super) struct SecuredSlot {
    key: Option<(u64, u64, u64, u64)>,
    pub(super) state: State,
}

/// The rows `keep` of column `c` (its distinct values renumbered: only those the rows use).
pub(super) fn keep_rows(c: &ColData, keep: &[u32]) -> Result<ColData, Miss> {
    let n = c.codes.len();
    let mut remap: HashMap<u32, u32> = HashMap::new();
    let mut values = vec![bicraft_vertipaq::CellValue::Null];
    let mut codes = Vec::with_capacity(keep.len());
    for &row in keep {
        let i = usize::try_from(row).map_err(|_| Miss::Error("row out of range".into()))?;
        if i >= n {
            return Err(Miss::Error(
                "View as: the rows of a secured table changed under it".into(),
            ));
        }
        let code = c.codes.get(i).unwrap_or(0);
        if code == 0 {
            codes.push(0);
            continue;
        }
        let next = u32::try_from(values.len()).map_err(|_| Miss::Error("too many distinct values".into()))?;
        let new = *remap.entry(code).or_insert(next);
        if new == next {
            let v = usize::try_from(code)
                .ok()
                .and_then(|k| c.values.get(k))
                .cloned()
                .unwrap_or(bicraft_vertipaq::CellValue::Null);
            values.push(v);
        }
        codes.push(new);
    }
    Ok(ColData {
        codes: Codes::Plain(codes),
        values,
        derived: Derived::default(),
    })
}

fn fingerprint(v: &ViewAs) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

impl PreviewCache {
    /// The state an evaluation of `s` runs on: under View as ([`Session::view_as_now`]) the
    /// secured state, else [`PreviewCache::acquire_raw`].
    pub(in crate::preview) fn acquire<'a>(&'a self, s: &'a Session) -> StateRef<'a> {
        match s.view_as_now() {
            None => self.acquire_raw(s),
            Some(v) => self.secured(s, &v),
        }
    }

    /// Drops the secured state (View as stopped; the next View as builds it again).
    pub(crate) fn drop_secured(&self) {
        let mut g = self
            .secured
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *g = SecuredSlot::default();
    }

    fn secured<'a>(&'a self, s: &'a Session, v: &ViewAs) -> StateRef<'a> {
        // the secured state first, then the main one (never the other way round)
        let mut slot = match self.secured.lock() {
            Ok(g) => g,
            // a panic while evaluating under View as: built again
            Err(p) => {
                let mut g = p.into_inner();
                *g = SecuredSlot::default();
                self.secured.clear_poison();
                g
            }
        };
        let mut main = self.acquire_raw(s);
        let key = (
            s.revision,
            main.source_fp.unwrap_or(0),
            main.generation,
            fingerprint(v),
        );
        if slot.key != Some(key) {
            let security = compute(s, &mut main, v);
            let mut mem = memory::Memory::background(main.memory.setting, main.memory.pool());
            mem.background = false;
            mem.query_setting = main.memory.query_setting;
            let state = State {
                revision: main.revision,
                source_fp: main.source_fp,
                generation: main.generation,
                store: main.store.clone(),
                own: main.own.clone(),
                literal: main.literal.clone(),
                literal_inputs: main.literal_inputs,
                model_hash: main.model_hash,
                registry: main.registry.clone(),
                measure_facts: main.measure_facts,
                inherited: Some(Inherited::of(&main.columns)),
                // the aggregations are built from every row: never under View as
                aggs_config: general::storage::aggs::AggConfig {
                    enabled: Some(false),
                    ..main.aggs_config
                },
                activity: Arc::clone(&main.activity),
                security: Some(Arc::new(security)),
                user: Some(v.user.clone().unwrap_or_default()),
                memory: mem,
                ..State::default()
            };
            // the old secured state goes before the new one counts its memory
            slot.state = State::default();
            slot.state = state;
            slot.key = Some(key);
        }
        drop(main);
        slot.state.sync(s);
        StateRef::Secured(slot)
    }
}

/// The rows the roles of `v` keep, computed on the data as stored (`main`).
fn compute(s: &Session, main: &mut State, v: &ViewAs) -> Security {
    let Some(model) = s.model.as_ref() else {
        return Security {
            error: Some("View as: no model is open".into()),
            ..Security::default()
        };
    };
    let mut roles = Vec::with_capacity(v.roles.len());
    for name in &v.roles {
        match model.roles.iter().find(|r| r.name.eq_ignore_ascii_case(name)) {
            Some(r) => roles.push(r),
            None => {
                return Security {
                    error: Some(format!("View as: the model has no role `{name}` any more")),
                    ..Security::default()
                };
            }
        }
    }
    let limits = PreviewLimits {
        rows_per_visual: 0,
        table_rows: crate::SnapshotLimits::default().data_table_rows,
        fallback_rows: crate::SnapshotLimits::default().data_table_rows,
        budget: general::rls::MAX_TOTAL,
        now: super::filters::clock(s),
        utc_offset: super::filters::utc_offset(s),
    };
    // the user the filters see (`USERPRINCIPALNAME()`), on the main state for this computation
    let before = main.user.replace(v.user.clone().unwrap_or_default());
    let out = {
        let mut ctx = Ctx {
            s,
            st: main,
            model,
            limits: &limits,
        };
        // the stored data opened here, so the secured state shares it
        let _ = ctx.store();
        let _ = ctx.own();
        general::rls::secured_rows(&mut ctx, &roles)
    };
    main.user = before;
    let (hidden_tables, hidden_columns) = match object_security(&roles) {
        Ok(h) => h,
        Err(e) => {
            return Security {
                error: Some(e),
                ..Security::default()
            };
        }
    };
    match out {
        Ok(keep) => Security {
            keep: keep.into_iter().map(|(t, rows)| (t, Arc::new(rows))).collect(),
            error: None,
            hidden_tables,
            hidden_columns,
        },
        Err(e) => Security {
            error: Some(format!("View as: {e}")),
            ..Security::default()
        },
    }
}
