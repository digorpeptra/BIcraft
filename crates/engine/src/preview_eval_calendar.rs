//! Calendar-based time intelligence ([`super`]): the time intelligence functions when their
//! dates argument is a calendar of the model rather than dates.
//!
//! Semantics from Microsoft Learn: the Power BI page "Implement time-based calculations in Power
//! BI" (section "Calendar-based time intelligence": categories, period uniqueness, context
//! clearing, time-related columns, lateral and hierarchical shifts, `DATEADD` and
//! `DATESINPERIOD` with calendars) and the DAX reference pages of the functions (`DATEADD` and
//! its `extension`/`truncation` parameters, `DATESINPERIOD` and its `endbehavior`,
//! `DATESBETWEEN`, `PARALLELPERIOD`, `SAMEPERIODLASTYEAR`, the `*WEEK`/`*WTD` functions).
//! The rules kept (`docs/formats/dax-functions.md`, "Added on 2026-11-25" and "Calendar-based
//! time intelligence"):
//! - A calendar is metadata on a table (`calendars` in TMSL, `calendar` in TMDL): column groups,
//!   each a time unit (`year`, `weekOfYear`, `date`…) with a primary column and associated
//!   columns, or a group of time-related columns without a unit.
//! - The calendar's rows are ordered by its Date category's primary column, else by the primary
//!   columns from the coarsest unit to the finest; a primary column with a sort-by column sorts
//!   by it (the page: "the primary columns are used for sorting"). Rows that order the same are
//!   one position of time (a day when there is a Date category).
//! - A period is identified by the primary columns of a set of categories that identifies it
//!   uniquely (the page's "Period uniqueness" sets; for a week: Week, or Week of Year and Year,
//!   or Week of Quarter and Quarter, …); the first set the calendar has, in the page's order, is
//!   used. Periods are ordered by their first position; shifting by `n` periods moves `n` places
//!   in that order (the calendar's own periods: sparse dates are allowed).
//! - The current selection is the positions of the calendar's rows visible in the context (after
//!   context transition). `DATEADD` and `SAMEPERIODLASTYEAR` read it from the tagged categories
//!   only (filters on time-related and untagged columns of the table are kept and still apply
//!   afterwards: the page's "lateral shift" example).
//! - Functions return the calendar's rows they select, as a table of its primary columns and its
//!   time-related columns (`DATEADD` and `SAMEPERIODLASTYEAR`: primary columns only, since they
//!   keep the time-related filters). As a filter argument (and in `TOTAL*TD` and the balances)
//!   the filters on the columns tagged in every calendar of the table and on their sort-by
//!   columns are removed first (the page's context clearing and cross-calendar rule; the result
//!   sets every primary column, so every category is cleared); `DATEADD` and
//!   `SAMEPERIODLASTYEAR` keep the filters on time-related columns. Filters on untagged columns
//!   are kept (also on a table related by its date column: the classic rule that a date key's
//!   filter removes the table's other filters does not apply).
//! - `DATEADD(calendar, n, unit [, extension [, truncation]])`: a selection covering whole
//!   periods moves to the whole target periods; otherwise (a finer grain than the unit) each run
//!   of consecutive positions keeps its positions within the period (the reference's
//!   index-based approach), or, when it is made of whole periods of a finer unit (a month in a
//!   year), its index among them: positions the target lacks are dropped (`BLANKS`, the default) or
//!   land on its last position (`ANCHORED`); a run reaching its period's end extends to the
//!   target's end (`EXTENDING`, the default), keeps its positions (`PRECISE`) or keeps its
//!   length, aligned on the target's end (`ENDALIGNED`).
//! - `DATESINPERIOD(calendar, start, n, unit [, endbehavior])`: the boundary is `DATEADD` of the
//!   start position alone (`PRECISE` by default, a missing position anchored to the target's
//!   last one); `(boundary, start]` for a negative `n`, `[start, boundary)` for a positive one.
//!
//! - A calendar function whose calendar argument is itself a calendar function over a calendar
//!   (`PREVIOUSDAY(PREVIOUSMONTH(Cal))`) is an error, as the page's considerations say they
//!   cannot be nested; its alternative, the inner one as a `CALCULATETABLE` filter, works.

use std::collections::{BTreeMap, HashMap, HashSet};

use bicraft_dax::ast::Expr;

use super::super::Fail;
use super::tables::sort_cmp;
use super::{ArgEff, ColId, Engine, Scope, TCol, Tab, TabId, V, arg, dax_error, keyword, need, num};

/// Category sets that identify a day, a week, a month, a quarter and a year, in the page's
/// order (semesters after them), as TMSL time units in lower case.
const DAY_KEYS: &[&[&str]] = &[
    &["date"],
    &["dayofyear", "year"],
    &["dayofquarter", "quarter"],
    &["dayofquarter", "quarterofyear", "year"],
    &["dayofmonth", "month"],
    &["dayofmonth", "monthofyear", "year"],
    &["dayofweek", "week"],
    &["dayofweek", "weekofyear", "year"],
    &["dayofsemester", "semester"],
    &["dayofsemester", "semesterofyear", "year"],
];
const WEEK_KEYS: &[&[&str]] = &[
    &["week"],
    &["weekofyear", "year"],
    &["weekofquarter", "quarter"],
    &["weekofquarter", "quarterofyear", "year"],
    &["weekofmonth", "month"],
    &["weekofmonth", "monthofyear", "year"],
    &["weekofmonth", "monthofquarter", "quarter"],
    &["weekofmonth", "monthofquarter", "quarterofyear", "year"],
    &["weekofsemester", "semester"],
    &["weekofsemester", "semesterofyear", "year"],
];
const MONTH_KEYS: &[&[&str]] = &[
    &["month"],
    &["monthofyear", "year"],
    &["monthofquarter", "quarter"],
    &["monthofquarter", "quarterofyear", "year"],
    &["monthofsemester", "semester"],
    &["monthofsemester", "semesterofyear", "year"],
];
const QUARTER_KEYS: &[&[&str]] = &[
    &["quarter"],
    &["quarterofyear", "year"],
    &["quarterofsemester", "semester"],
    &["quarterofsemester", "semesterofyear", "year"],
];
const YEAR_KEYS: &[&[&str]] = &[&["year"]];

/// The time units from the coarsest to the finest: the order of a calendar's rows without a
/// Date category.
const UNIT_ORDER: &[&str] = &[
    "year",
    "semester",
    "semesterofyear",
    "quarter",
    "quarterofyear",
    "quarterofsemester",
    "month",
    "monthofyear",
    "monthofsemester",
    "monthofquarter",
    "week",
    "weekofyear",
    "weekofsemester",
    "weekofquarter",
    "weekofmonth",
    "date",
    "dayofyear",
    "dayofsemester",
    "dayofquarter",
    "dayofmonth",
    "dayofweek",
];

/// Largest day number kept (the DAX calendar's range, with room).
const MAX_DAY: f64 = 3_000_000.0;

/// Functions that take a calendar in place of their dates argument (the first one), and return
/// a table.
pub(super) fn takes_calendar(up: &str) -> bool {
    matches!(
        up,
        "DATESWTD"
            | "DATESMTD"
            | "DATESQTD"
            | "DATESYTD"
            | "STARTOFWEEK"
            | "STARTOFMONTH"
            | "STARTOFQUARTER"
            | "STARTOFYEAR"
            | "ENDOFWEEK"
            | "ENDOFMONTH"
            | "ENDOFQUARTER"
            | "ENDOFYEAR"
            | "FIRSTDATE"
            | "LASTDATE"
            | "PREVIOUSDAY"
            | "PREVIOUSWEEK"
            | "PREVIOUSMONTH"
            | "PREVIOUSQUARTER"
            | "PREVIOUSYEAR"
            | "NEXTDAY"
            | "NEXTWEEK"
            | "NEXTMONTH"
            | "NEXTQUARTER"
            | "NEXTYEAR"
            | "PARALLELPERIOD"
            | "DATEADD"
            | "SAMEPERIODLASTYEAR"
            | "DATESINPERIOD"
            | "DATESBETWEEN"
    )
}

/// The functions that only shift laterally and keep the time-related columns' filters (the
/// page's "fixed" group).
fn is_fixed(up: &str) -> bool {
    matches!(up, "DATEADD" | "SAMEPERIODLASTYEAR")
}

/// A calendar of the model.
pub(super) struct Calendar {
    name: String,
    /// The table it is defined on.
    table: TabId,
    /// Its column groups: the time unit (lower case; `None` for time-related columns) and the
    /// columns, the primary one first.
    groups: Vec<(Option<String>, Vec<ColId>)>,
    /// Every column tagged in a category of a calendar of the same table, and their sort-by
    /// columns.
    categorized: HashSet<ColId>,
    /// Every column tagged in a calendar of the same table (categories and time-related
    /// columns), and their sort-by columns: what the flexible functions clear.
    tagged: HashSet<ColId>,
    /// The sort-by column of each column of the table that has one.
    sort_by: HashMap<ColId, ColId>,
}

impl Calendar {
    fn primary(&self, unit: &str) -> Option<ColId> {
        self.groups
            .iter()
            .find(|(u, _)| u.as_deref() == Some(unit))
            .and_then(|(_, cols)| cols.first().copied())
    }

    /// The primary columns of the first category set of `sets` the calendar has.
    fn keys(&self, sets: &[&[&str]]) -> Option<Vec<ColId>> {
        sets.iter().find_map(|set| {
            set.iter()
                .map(|u| self.primary(u))
                .collect::<Option<Vec<ColId>>>()
        })
    }

    /// The columns of the table a calendar function returns: the primary columns, then the
    /// time-related columns unless `fixed`.
    fn result_cols(&self, fixed: bool) -> Vec<ColId> {
        let mut out: Vec<ColId> = Vec::new();
        for (u, cols) in &self.groups {
            let add: Vec<ColId> = match u {
                Some(_) => cols.first().copied().into_iter().collect(),
                None if !fixed => cols.clone(),
                None => Vec::new(),
            };
            for c in add {
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
        out
    }
}

/// The columns of a calendar column group, the primary one first.
fn group_columns(g: &serde_json::Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(p) = g.get("primaryColumn").and_then(|v| v.as_str()) {
        out.push(p.to_string());
    }
    for key in ["associatedColumns", "columns"] {
        if let Some(a) = g.get(key).and_then(|v| v.as_array()) {
            out.extend(a.iter().filter_map(|v| v.as_str()).map(str::to_string));
        }
    }
    out
}

/// The time unit of a calendar column group, lower case; `None` for time-related columns.
fn group_unit(g: &serde_json::Value) -> Option<String> {
    g.get("timeUnit")
        .and_then(|u| u.as_str())
        .map(str::to_ascii_lowercase)
        .filter(|u| u != "unknown")
}

/// The positions of a calendar's rows: `ord[row]` is the row's position in time (rows that
/// order the same share one), `count` how many there are.
struct CalRows {
    ord: Vec<u32>,
    count: usize,
}

/// The periods of a unit: each one's positions, ascending, the periods ordered by their first
/// position; `of[position]` is the period that holds it.
struct Periods {
    list: Vec<Vec<u32>>,
    of: Vec<usize>,
}

impl Periods {
    fn of(&self, pos: u32) -> Option<usize> {
        self.of.get(pos as usize).copied().filter(|p| *p != usize::MAX)
    }

    /// Period `p` moved by `k` periods, when the calendar has it.
    fn moved(&self, p: usize, k: i64) -> Option<usize> {
        let q = i64::try_from(p).ok()?.checked_add(k)?;
        let q = usize::try_from(q).ok()?;
        (q < self.list.len()).then_some(q)
    }
}

/// `DATEADD`'s `extension`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Extension {
    Extending,
    Precise,
    EndAligned,
}

/// `DATEADD`'s `truncation`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Truncation {
    Blanks,
    Anchored,
}

/// The unit an interval keyword names.
fn interval_unit(e: Option<&Expr>, f: &str) -> Result<&'static str, Fail> {
    match e.and_then(keyword).as_deref() {
        Some("DAY") => Ok("day"),
        Some("WEEK") => Ok("week"),
        Some("MONTH") => Ok("month"),
        Some("QUARTER") => Ok("quarter"),
        Some("YEAR") => Ok("year"),
        _ => dax_error(format!(
            "{f}(): the interval must be DAY, WEEK, MONTH, QUARTER or YEAR"
        )),
    }
}

fn unit_sets(unit: &str) -> &'static [&'static [&'static str]] {
    match unit {
        "day" => DAY_KEYS,
        "week" => WEEK_KEYS,
        "month" => MONTH_KEYS,
        "quarter" => QUARTER_KEYS,
        _ => YEAR_KEYS,
    }
}

/// The unit a function's name ends with (`DATESWTD` → week, `NEXTDAY` → day…).
fn name_unit(up: &str) -> &'static str {
    if up.ends_with("WEEK") || up.ends_with("WTD") {
        "week"
    } else if up.ends_with("MONTH") || up.ends_with("MTD") {
        "month"
    } else if up.ends_with("QUARTER") || up.ends_with("QTD") {
        "quarter"
    } else if up.ends_with("DAY") {
        "day"
    } else {
        "year"
    }
}

/// The units finer than `unit` a shift can keep the index of, coarsest first (days are the
/// positions themselves).
fn finer_units(unit: &str) -> &'static [&'static str] {
    match unit {
        "year" => &["quarter", "month", "week"],
        "quarter" => &["month", "week"],
        "month" => &["week"],
        _ => &[],
    }
}

/// Where the run of items `s..=e` (0-based, of `n_src` items) lands among `n_dst` items (see
/// `DATEADD` in the module docs).
fn map_run(s: usize, e: usize, n_src: usize, n_dst: usize, ext: Extension, trunc: Truncation) -> Vec<usize> {
    let (Some(last_src), Some(last_dst)) = (n_src.checked_sub(1), n_dst.checked_sub(1)) else {
        return Vec::new();
    };
    if s == 0 && e == last_src {
        // the whole period: the whole target period
        (0..=last_dst).collect()
    } else if e == last_src && ext == Extension::EndAligned {
        let len = e.saturating_sub(s);
        (last_dst.saturating_sub(len)..=last_dst).collect()
    } else if e == last_src && ext == Extension::Extending && n_dst > n_src {
        (s..=last_dst).collect()
    } else {
        let mut out: Vec<usize> = (s..=e)
            .filter_map(|i| match (i <= last_dst, trunc) {
                (true, _) => Some(i),
                (false, Truncation::Anchored) => Some(last_dst),
                (false, Truncation::Blanks) => None,
            })
            .collect();
        out.dedup();
        out
    }
}

/// The periods of `level` that start within the positions `span` (ascending), in order.
fn starting_in(level: &Periods, span: &[u32]) -> Vec<usize> {
    let (Some(lo), Some(hi)) = (span.first(), span.last()) else {
        return Vec::new();
    };
    (0..level.list.len())
        .filter(|i| {
            level
                .list
                .get(*i)
                .and_then(|l| l.first())
                .is_some_and(|f| f >= lo && f <= hi)
        })
        .collect()
}

/// `DATEADD` of the positions `sel` by `k` periods of `per`. `levels`: the periods of the units
/// finer than the shift's, coarsest first: a run made of whole periods of one of them keeps its
/// index among that unit's periods (a month shifted by a year stays the same month of the
/// year); else it keeps its positions.
fn shift(
    per: &Periods,
    levels: &[Periods],
    sel: &[u32],
    k: i64,
    ext: Extension,
    trunc: Truncation,
) -> Vec<u32> {
    let mut by_period: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for pos in sel {
        let Some(p) = per.of(*pos) else { continue };
        let Some(i) = per.list.get(p).and_then(|l| l.binary_search(pos).ok()) else {
            continue;
        };
        by_period.entry(p).or_default().push(i);
    }
    let mut out: Vec<u32> = Vec::new();
    for (p, mut at) in by_period {
        at.sort_unstable();
        at.dedup();
        let (Some(src), Some(dst)) = (per.list.get(p), per.moved(p, k).and_then(|q| per.list.get(q))) else {
            continue;
        };
        // runs of consecutive positions
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for i in at {
            match runs.last_mut() {
                Some((_, e)) if e.checked_add(1) == Some(i) => *e = i,
                _ => runs.push((i, i)),
            }
        }
        'run: for (s, e) in runs {
            let (Some(&first), Some(&last)) = (src.get(s), src.get(e)) else {
                continue;
            };
            if !(s == 0 && e.checked_add(1) == Some(src.len())) {
                for level in levels {
                    let (Some(a), Some(b)) = (level.of(first), level.of(last)) else {
                        continue;
                    };
                    // whole periods of the level, all within the run
                    let whole = (a..=b).all(|i| {
                        level.list.get(i).is_some_and(|l| {
                            l.first().is_some_and(|f| *f >= first) && l.last().is_some_and(|x| *x <= last)
                        })
                    });
                    if !whole {
                        continue;
                    }
                    let (from, to) = (starting_in(level, src), starting_in(level, dst));
                    let (Some(ia), Some(ib)) = (
                        from.iter().position(|i| *i == a),
                        from.iter().position(|i| *i == b),
                    ) else {
                        continue;
                    };
                    for j in map_run(ia, ib, from.len(), to.len(), ext, trunc) {
                        if let Some(l) = to.get(j).and_then(|q| level.list.get(*q)) {
                            out.extend(l.iter().copied());
                        }
                    }
                    continue 'run;
                }
            }
            for i in map_run(s, e, src.len(), dst.len(), ext, trunc) {
                if let Some(x) = dst.get(i) {
                    out.push(*x);
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

impl Engine<'_, '_> {
    /// The calendar `e` names (a bare or quoted name), when the model has one of that name.
    pub(super) fn calendar_of(&self, e: &Expr) -> Option<Calendar> {
        let Expr::Name { name, .. } = e else {
            return None;
        };
        for t in &self.model.tables {
            let Some(cals) = t.extra.get("calendars").and_then(|v| v.as_array()) else {
                continue;
            };
            let Some(cal) = cals.iter().find(|c| {
                c.get("name")
                    .and_then(|n| n.as_str())
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            }) else {
                continue;
            };
            let table = self.table_id(&t.name)?;
            let col = |c: &str| self.col_id(&t.name, c);
            let sort_by: HashMap<ColId, ColId> = t
                .columns
                .iter()
                .filter_map(|c| Some((col(&c.name)?, col(c.sort_by_column.as_deref()?)?)))
                .collect();
            let groups = cal
                .get("calendarColumnGroups")
                .and_then(|v| v.as_array())
                .map(|gs| {
                    gs.iter()
                        .map(|g| {
                            (
                                group_unit(g),
                                group_columns(g).iter().filter_map(|c| col(c)).collect(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            let mut categorized: HashSet<ColId> = HashSet::new();
            let mut tagged: HashSet<ColId> = HashSet::new();
            for g in cals
                .iter()
                .filter_map(|c| c.get("calendarColumnGroups").and_then(|v| v.as_array()))
                .flatten()
            {
                let unit = group_unit(g);
                for c in group_columns(g).iter().filter_map(|c| col(c)) {
                    let with_sort = std::iter::once(c).chain(sort_by.get(&c).copied());
                    if unit.is_some() {
                        categorized.extend(with_sort.clone());
                    }
                    tagged.extend(with_sort);
                }
            }
            return Some(Calendar {
                name: name.clone(),
                table,
                groups,
                categorized,
                tagged,
                sort_by,
            });
        }
        None
    }

    /// The error for a calendar function whose calendar argument `e` is itself a calendar
    /// function over a calendar: the page's considerations say calendar functions cannot be
    /// nested (`PREVIOUSDAY(PREVIOUSMONTH('Calendar'))`) and give the inner one as a
    /// `CALCULATETABLE` filter instead. `Ok(())` when `e` is not such a call.
    pub(super) fn refuse_nested_calendar(&self, outer: &str, e: Option<&Expr>) -> Result<(), Fail> {
        let Some(Expr::Call { name, args, .. }) = e else {
            return Ok(());
        };
        let inner = name.to_ascii_uppercase();
        if !takes_calendar(&inner) {
            return Ok(());
        }
        let Some(cal) = arg(args, 0).and_then(|a| self.calendar_of(a)) else {
            return Ok(());
        };
        dax_error(format!(
            "{outer}() over {inner}({}): calendar-based time intelligence functions cannot be nested;              use CALCULATETABLE({outer}(calendar), {inner}(calendar)) instead",
            cal.name
        ))
    }

    /// The period keys of `unit` in `cal`, or the error naming what is missing.
    fn period_keys(&self, cal: &Calendar, unit: &str, f: &str) -> Result<Vec<ColId>, Fail> {
        match cal.keys(unit_sets(unit)) {
            Some(k) => Ok(k),
            None => dax_error(format!(
                "{f}(): the calendar '{}' has no categories that identify a {unit}",
                cal.name
            )),
        }
    }

    /// The positions of the calendar's rows (see the module docs).
    fn cal_rows(&mut self, cal: &Calendar) -> Result<CalRows, Fail> {
        let sort: Vec<ColId> = match cal.primary("date") {
            Some(d) => vec![d],
            None => UNIT_ORDER.iter().filter_map(|u| cal.primary(u)).collect(),
        };
        if sort.is_empty() {
            return dax_error(format!("the calendar '{}' has no category", cal.name));
        }
        let mut datas = Vec::with_capacity(sort.len());
        for c in &sort {
            let by = cal.sort_by.get(c).copied().unwrap_or(*c);
            datas.push(self.col(by)?);
        }
        let n = self.rows(cal.table)?;
        self.tick(n.saturating_mul(sort.len().saturating_add(1)))?;
        let keys: Vec<Vec<V>> = (0..n)
            .map(|r| {
                datas
                    .iter()
                    .map(|d| {
                        let code = d.codes.get(r).unwrap_or(0);
                        d.values.get(code as usize).map_or(V::Blank, V::of_cell)
                    })
                    .collect()
            })
            .collect();
        let cmp = |a: &Vec<V>, b: &Vec<V>| {
            a.iter()
                .zip(b)
                .map(|(x, y)| sort_cmp(x, y))
                .find(|o| o.is_ne())
                .unwrap_or(std::cmp::Ordering::Equal)
        };
        let mut idx: Vec<usize> = (0..n).collect();
        idx.sort_by(|a, b| match (keys.get(*a), keys.get(*b)) {
            (Some(x), Some(y)) => cmp(x, y),
            _ => std::cmp::Ordering::Equal,
        });
        let mut ord = vec![0u32; n];
        let mut count = 0usize;
        let mut prev: Option<usize> = None;
        for r in idx {
            let same = match (prev.and_then(|p| keys.get(p)), keys.get(r)) {
                (Some(x), Some(y)) => cmp(x, y).is_eq(),
                _ => false,
            };
            if !same {
                count = count.saturating_add(1);
            }
            if let Some(slot) = ord.get_mut(r) {
                *slot = u32::try_from(count.saturating_sub(1)).unwrap_or(u32::MAX);
            }
            prev = Some(r);
        }
        Ok(CalRows { ord, count })
    }

    /// The periods identified by the columns `keys`.
    fn periods(&mut self, rows: &CalRows, keys: &[ColId]) -> Result<Periods, Fail> {
        let mut datas = Vec::with_capacity(keys.len());
        for k in keys {
            datas.push(self.col(*k)?);
        }
        self.tick(rows.ord.len().saturating_mul(keys.len().saturating_add(1)))?;
        let mut by_key: HashMap<Vec<u32>, Vec<u32>> = HashMap::new();
        for (r, pos) in rows.ord.iter().enumerate() {
            let key: Vec<u32> = datas.iter().map(|d| d.codes.get(r).unwrap_or(0)).collect();
            by_key.entry(key).or_default().push(*pos);
        }
        let mut list: Vec<Vec<u32>> = by_key
            .into_values()
            .map(|mut v| {
                v.sort_unstable();
                v.dedup();
                v
            })
            .collect();
        list.sort_by_key(|v| v.first().copied().unwrap_or(u32::MAX));
        let mut of = vec![usize::MAX; rows.count];
        for (p, l) in list.iter().enumerate() {
            for pos in l {
                if let Some(slot) = of.get_mut(*pos as usize)
                    && *slot == usize::MAX
                {
                    *slot = p;
                }
            }
        }
        Ok(Periods { list, of })
    }

    /// The periods of the units finer than `unit` the calendar identifies, coarsest first.
    fn shift_levels(&mut self, cal: &Calendar, rows: &CalRows, unit: &str) -> Result<Vec<Periods>, Fail> {
        let mut out = Vec::new();
        for u in finer_units(unit) {
            if let Some(keys) = cal.keys(unit_sets(u)) {
                out.push(self.periods(rows, &keys)?);
            }
        }
        Ok(out)
    }

    /// The positions of the calendar's rows visible in the context of `s` (after context
    /// transition); `fixed`: read from the tagged categories only.
    fn cal_selection(
        &mut self,
        cal: &Calendar,
        rows: &CalRows,
        fixed: bool,
        s: &Scope,
    ) -> Result<Vec<u32>, Fail> {
        let mut fc = if s.rc.is_empty() {
            s.fc.child()
        } else {
            self.transitioned(s.fc, s.rc)?
        };
        if fixed {
            let others: HashSet<ColId> = self
                .table_cols(cal.table)
                .into_iter()
                .filter(|c| !cal.categorized.contains(c))
                .collect();
            self.remove_cols(&mut fc.filters, &others)?;
        }
        let visible = self.visible_rows(&fc, cal.table)?;
        let mut out: Vec<u32> = visible
            .iter()
            .filter_map(|r| rows.ord.get(*r as usize).copied())
            .collect();
        out.sort_unstable();
        out.dedup();
        Ok(out)
    }

    /// The position of the calendar's day `v` (a value of its Date category's primary column).
    fn cal_day(&mut self, cal: &Calendar, rows: &CalRows, v: &V, f: &str) -> Result<Option<u32>, Fail> {
        let Some(dc) = cal.primary("date") else {
            return dax_error(format!("{f}(): the calendar '{}' has no Date category", cal.name));
        };
        if matches!(v, V::Blank) {
            return Ok(None);
        }
        let mut code = self.code_of(dc, v)?;
        if code.is_none() {
            // a date given as a number, or a number as a date
            let other = match v {
                V::Num(x) => Some(V::Date(*x)),
                V::Date(x) => Some(V::Num(*x)),
                _ => None,
            };
            if let Some(o) = other {
                code = self.code_of(dc, &o)?;
            }
        }
        let Some(code) = code.filter(|c| *c != 0) else {
            return Ok(None);
        };
        let d = self.col(dc)?;
        self.tick(rows.ord.len())?;
        Ok(rows
            .ord
            .iter()
            .enumerate()
            .find(|(r, _)| d.codes.get(*r) == Some(code))
            .map(|(_, p)| *p))
    }

    /// The table a calendar function returns: the calendar's rows at the positions `picked`, as
    /// its primary columns (and time-related ones unless `fixed`).
    fn cal_result(
        &mut self,
        cal: &Calendar,
        rows: &CalRows,
        picked: &[u32],
        fixed: bool,
    ) -> Result<Tab, Fail> {
        let cols = cal.result_cols(fixed);
        if cols.is_empty() {
            return dax_error(format!("the calendar '{}' has no category", cal.name));
        }
        let mut on = vec![false; rows.count];
        for p in picked {
            if let Some(slot) = on.get_mut(*p as usize) {
                *slot = true;
            }
        }
        let mut datas = Vec::with_capacity(cols.len());
        for c in &cols {
            datas.push(self.col(*c)?);
        }
        self.tick(rows.ord.len().saturating_mul(cols.len()))?;
        let mut seen: HashSet<Vec<u32>> = HashSet::new();
        let mut out: Vec<Vec<V>> = Vec::new();
        let mut order: Vec<(u32, usize)> = Vec::new();
        for (r, pos) in rows.ord.iter().enumerate() {
            if !on.get(*pos as usize).copied().unwrap_or(false) {
                continue;
            }
            let key: Vec<u32> = datas.iter().map(|d| d.codes.get(r).unwrap_or(0)).collect();
            if !seen.insert(key.clone()) {
                continue;
            }
            let vals = datas
                .iter()
                .zip(&key)
                .map(|(d, code)| d.values.get(*code as usize).map_or(V::Blank, V::of_cell))
                .collect();
            order.push((*pos, out.len()));
            out.push(vals);
        }
        // in the calendar's order
        order.sort_by_key(|(p, i)| (*p, *i));
        let mut sorted: Vec<Vec<V>> = Vec::with_capacity(out.len());
        let mut slots: Vec<Option<Vec<V>>> = out.into_iter().map(Some).collect();
        for (_, i) in order {
            if let Some(v) = slots.get_mut(i).and_then(Option::take) {
                sorted.push(v);
            }
        }
        Ok(Tab::tuples(cols.into_iter().map(TCol::of).collect(), sorted))
    }

    /// A calendar function `up(calendar, …)` (one of [`takes_calendar`]): its table, and whether
    /// it shifts laterally only (`DATEADD`, `SAMEPERIODLASTYEAR`).
    pub(super) fn calendar_table(
        &mut self,
        up: &str,
        cal: &Calendar,
        args: &[Option<Expr>],
        s: &Scope,
    ) -> Result<(Tab, bool), Fail> {
        let f = up;
        let fixed = is_fixed(up);
        let max_args = match up {
            "DATEADD" => 5,
            "DATESINPERIOD" => 5,
            "PARALLELPERIOD" | "DATESBETWEEN" => 3,
            _ => 1,
        };
        if args.len() > max_args {
            return dax_error(match up {
                "DATESYTD" | "STARTOFYEAR" | "ENDOFYEAR" | "PREVIOUSYEAR" | "NEXTYEAR" => {
                    format!("{f}(): a year-end date is only allowed with a date column, not with a calendar")
                }
                _ => format!("{f}() takes at most {max_args} arguments with a calendar"),
            });
        }
        let rows = self.cal_rows(cal)?;
        let picked: Vec<u32> = match up {
            "DATESBETWEEN" => {
                let lo = self.scalar(need(args, 1, f)?, s)?;
                let hi = self.scalar(need(args, 2, f)?, s)?;
                let last = u32::try_from(rows.count.saturating_sub(1)).unwrap_or(u32::MAX);
                let lo = self.cal_day(cal, &rows, &lo, f)?.unwrap_or(0);
                let hi = self.cal_day(cal, &rows, &hi, f)?.unwrap_or(last);
                if rows.count == 0 {
                    Vec::new()
                } else {
                    (lo..=hi).collect()
                }
            }
            "DATESINPERIOD" => {
                let start = self.scalar(need(args, 1, f)?, s)?;
                let k = self.int_arg(need(args, 2, f)?, s)?;
                let unit = interval_unit(arg(args, 3), f)?;
                let ext = match arg(args, 4).and_then(keyword).as_deref() {
                    None | Some("PRECISE") => Extension::Precise,
                    Some("ENDALIGNED") => Extension::EndAligned,
                    Some(other) => {
                        return dax_error(format!("{f}(): end behavior `{other}` (PRECISE or ENDALIGNED)"));
                    }
                };
                let keys = self.period_keys(cal, unit, f)?;
                let per = self.periods(&rows, &keys)?;
                match self.cal_day(cal, &rows, &start, f)? {
                    None => Vec::new(),
                    Some(_) if k == 0 => Vec::new(),
                    Some(st) => {
                        let bound = shift(&per, &[], &[st], k, ext, Truncation::Anchored)
                            .last()
                            .copied();
                        let last = u32::try_from(rows.count.saturating_sub(1)).unwrap_or(u32::MAX);
                        if k < 0 {
                            // (boundary, start]
                            let lo = bound.map_or(0, |b| b.saturating_add(1));
                            (lo..=st).collect()
                        } else {
                            // [start, boundary)
                            match bound {
                                Some(b) if b > st => (st..b).collect(),
                                Some(_) => Vec::new(),
                                None => (st..=last).collect(),
                            }
                        }
                    }
                }
            }
            _ => {
                let sel = self.cal_selection(cal, &rows, fixed, s)?;
                let (Some(&first), Some(&last)) = (sel.first(), sel.last()) else {
                    return Ok((self.cal_result(cal, &rows, &[], fixed)?, fixed));
                };
                match up {
                    "FIRSTDATE" => vec![first],
                    "LASTDATE" => vec![last],
                    "DATEADD" | "SAMEPERIODLASTYEAR" => {
                        let (k, unit) = if up == "DATEADD" {
                            (
                                self.int_arg(need(args, 1, f)?, s)?,
                                interval_unit(arg(args, 2), f)?,
                            )
                        } else {
                            (-1, "year")
                        };
                        let ext = match arg(args, 3).and_then(keyword).as_deref() {
                            None | Some("EXTENDING") | Some("EXTENDED") => Extension::Extending,
                            Some("PRECISE") => Extension::Precise,
                            Some("ENDALIGNED") => Extension::EndAligned,
                            Some(other) => {
                                return dax_error(format!(
                                    "{f}(): extension `{other}` (EXTENDING, PRECISE or ENDALIGNED)"
                                ));
                            }
                        };
                        let trunc = match arg(args, 4).and_then(keyword).as_deref() {
                            None | Some("BLANKS") | Some("BLANK") => Truncation::Blanks,
                            Some("ANCHORED") => Truncation::Anchored,
                            Some(other) => {
                                return dax_error(format!(
                                    "{f}(): truncation `{other}` (BLANKS or ANCHORED)"
                                ));
                            }
                        };
                        let keys = self.period_keys(cal, unit, f)?;
                        let per = self.periods(&rows, &keys)?;
                        let levels = self.shift_levels(cal, &rows, unit)?;
                        shift(&per, &levels, &sel, k, ext, trunc)
                    }
                    "PARALLELPERIOD" => {
                        let k = self.int_arg(need(args, 1, f)?, s)?;
                        let unit = interval_unit(arg(args, 2), f)?;
                        let keys = self.period_keys(cal, unit, f)?;
                        let per = self.periods(&rows, &keys)?;
                        let (Some(pf), Some(pl)) = (per.of(first), per.of(last)) else {
                            return Ok((self.cal_result(cal, &rows, &[], fixed)?, fixed));
                        };
                        // the shifted periods the calendar has (clamped to its ends)
                        let lo = i64::try_from(pf).unwrap_or(i64::MAX).saturating_add(k).max(0);
                        let hi = i64::try_from(pl).unwrap_or(i64::MAX).saturating_add(k);
                        let n = i64::try_from(per.list.len()).unwrap_or(i64::MAX);
                        let hi = hi.min(n.saturating_sub(1));
                        let mut out = Vec::new();
                        if lo <= hi {
                            for q in lo..=hi {
                                if let Some(l) = usize::try_from(q).ok().and_then(|q| per.list.get(q)) {
                                    out.extend(l.iter().copied());
                                }
                            }
                        }
                        out
                    }
                    _ => {
                        let unit = name_unit(up);
                        let keys = self.period_keys(cal, unit, f)?;
                        let per = self.periods(&rows, &keys)?;
                        let pivot =
                            if up.starts_with("END") || up.starts_with("NEXT") || up.starts_with("DATES") {
                                last
                            } else {
                                first
                            };
                        let Some(p) = per.of(pivot) else {
                            return Ok((self.cal_result(cal, &rows, &[], fixed)?, fixed));
                        };
                        let target = if up.starts_with("PREVIOUS") {
                            per.moved(p, -1)
                        } else if up.starts_with("NEXT") {
                            per.moved(p, 1)
                        } else {
                            Some(p)
                        };
                        let Some(l) = target.and_then(|q| per.list.get(q)) else {
                            return Ok((self.cal_result(cal, &rows, &[], fixed)?, fixed));
                        };
                        if up.starts_with("DATES") {
                            l.iter().copied().filter(|x| *x <= last).collect()
                        } else if up.starts_with("STARTOF") {
                            l.first().copied().into_iter().collect()
                        } else if up.starts_with("ENDOF") {
                            l.last().copied().into_iter().collect()
                        } else {
                            l.clone()
                        }
                    }
                }
            }
        };
        Ok((self.cal_result(cal, &rows, &picked, fixed)?, fixed))
    }

    /// A calendar function as a filter argument of `CALCULATE` (or `CALCULATETABLE`): the
    /// filters on the calendar's tagged columns removed, then its rows set. `Ok(false)` when
    /// `e` is not a calendar function call.
    pub(super) fn calendar_filter(
        &mut self,
        e: &Expr,
        s: &Scope,
        keep: bool,
        out: &mut Vec<ArgEff>,
    ) -> Result<bool, Fail> {
        let Expr::Call { name, args, .. } = e else {
            return Ok(false);
        };
        let up = name.to_ascii_uppercase();
        if !takes_calendar(&up) {
            return Ok(false);
        }
        let Some(cal) = arg(args, 0).and_then(|a| self.calendar_of(a)) else {
            return Ok(false);
        };
        let (tab, fixed) = self.calendar_table(&up, &cal, args, s)?;
        let filters = self.tab_filters(&tab)?;
        out.push(ArgEff::Remove(if fixed { cal.categorized } else { cal.tagged }));
        out.push(ArgEff::Set {
            filters,
            keep,
            also_remove: Vec::new(),
        });
        Ok(true)
    }

    /// `TOTALWTD`/`TOTALMTD`/`TOTALQTD`/`TOTALYTD(expression, calendar [, filter])`.
    pub(super) fn calendar_total(
        &mut self,
        up: &str,
        cal: &Calendar,
        args: &[Option<Expr>],
        s: &Scope,
    ) -> Result<V, Fail> {
        let body = need(args, 0, up)?;
        if arg(args, 3).is_some() {
            return dax_error(format!(
                "{up}() takes at most 3 arguments with a calendar (a year-end date only with a date column)"
            ));
        }
        let dates_fn = format!("DATES{}", up.get(5..).unwrap_or("YTD"));
        let (dates, _) = self.calendar_table(&dates_fn, cal, args.get(1..2).unwrap_or_default(), s)?;
        let filters: Vec<&Expr> = arg(args, 2).into_iter().collect();
        let fc = self.calc_ctx_removing(&filters, vec![dates], Some(cal.tagged.clone()), s)?;
        let inner = Scope {
            fc: &fc,
            rc: &[],
            ..*s
        };
        self.scalar(body, &inner)
    }

    /// `OPENINGBALANCE*`/`CLOSINGBALANCE*(expression, calendar [, filter])`.
    pub(super) fn calendar_balance(
        &mut self,
        up: &str,
        cal: &Calendar,
        args: &[Option<Expr>],
        s: &Scope,
    ) -> Result<V, Fail> {
        let body = need(args, 0, up)?;
        if arg(args, 3).is_some() {
            return dax_error(format!("{up}() takes at most 3 arguments with a calendar"));
        }
        let unit = name_unit(up);
        let keys = self.period_keys(cal, unit, up)?;
        let Some(dc) = cal.primary("date") else {
            return dax_error(format!(
                "{up}(): the calendar '{}' has no Date category",
                cal.name
            ));
        };
        // the days of the current context (after context transition)
        let codes = if s.rc.is_empty() {
            self.values_codes(s.fc, dc, false)?
        } else {
            let fc = self.transitioned(s.fc, s.rc)?;
            self.values_codes(&fc, dc, false)?
        };
        let mut days: Vec<i64> = Vec::with_capacity(codes.len());
        for code in codes {
            if let Some(x) =
                num(&self.value_of_code(dc, code)?)?.filter(|x| x.is_finite() && x.abs() < MAX_DAY)
            {
                days.push(x.floor() as i64);
            }
        }
        let (first, last) = (days.iter().min().copied(), days.iter().max().copied());
        // every row of the calendar's table: its day and the codes of its period's columns
        let dcol = self.col(dc)?;
        let mut kcols = Vec::with_capacity(keys.len());
        for k in &keys {
            kcols.push(self.col(*k)?);
        }
        let n = dcol.codes.len();
        self.tick(n.saturating_mul(keys.len().saturating_add(1)))?;
        let mut rows: Vec<(i64, Vec<u32>)> = Vec::with_capacity(n);
        for r in 0..n {
            let code = dcol.codes.get(r).unwrap_or(0);
            let v = dcol.values.get(code as usize).map_or(V::Blank, V::of_cell);
            let Some(x) = num(&v)?.filter(|x| x.is_finite() && x.abs() < MAX_DAY) else {
                continue;
            };
            let key: Vec<u32> = kcols.iter().map(|k| k.codes.get(r).unwrap_or(0)).collect();
            rows.push((x.floor() as i64, key));
        }
        let period_of = |day: i64| rows.iter().find(|(d, _)| *d == day).map(|(_, k)| k.clone());
        let target: Option<i64> = if up.starts_with("OPENING") {
            first.and_then(|x| {
                let k = period_of(x)?;
                let start = rows.iter().filter(|(_, rk)| *rk == k).map(|(d, _)| *d).min()?;
                rows.iter().map(|(d, _)| *d).filter(|d| *d < start).max()
            })
        } else {
            last.and_then(|x| {
                let k = period_of(x)?;
                rows.iter().filter(|(_, rk)| *rk == k).map(|(d, _)| *d).max()
            })
        };
        let dates = match target {
            Some(d) => self.dates_in(dc, &[(d, d)])?,
            None => Tab::empty_of(dc),
        };
        let filters: Vec<&Expr> = arg(args, 2).into_iter().collect();
        let fc = self.calc_ctx_removing(&filters, vec![dates], Some(cal.tagged.clone()), s)?;
        let inner = Scope {
            fc: &fc,
            rc: &[],
            ..*s
        };
        self.scalar(body, &inner)
    }
}
