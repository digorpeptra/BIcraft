//! The checks of the Create and Edit relationship dialogs (docs/formats/model-objects.md,
//! "Manage relationships"), shared by `relationship.add` and `relationship.set`. Microsoft Learn
//! ("Create and manage relationships in Power BI Desktop") describes them: the dialog validates
//! the relationship as columns are picked and offers a cardinality and a cross filter direction
//! from the data; one of the columns must hold unique values unless the relationship is
//! many-to-many; only one relationship between two tables is active, and none that makes filter
//! paths ambiguous. `check: true` answers this review without changing the model.

use bicraft_model::{DataType, Relationship, SemanticModel};
use serde_json::{Value, json};

use crate::Session;
use crate::commands::model;
use crate::edit::{Edit, apply, check_name, dax_column, new_guid, parse_column_ref};
use crate::params::{allow, opt_bool, opt_str, str_param};
use crate::relationship_set::{ambiguous_pairs, check_unique};

/// Rows read per column for the uniqueness checks.
const MAX_CHECK_ROWS: usize = 5_000_000;

/// One finding: a stable code and a message.
#[derive(Debug, Clone)]
pub(crate) struct Finding {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl Finding {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    fn json(&self) -> Value {
        json!({"code": self.code, "message": self.message})
    }
}

/// What the review found.
#[derive(Debug, Default)]
pub(crate) struct Verdict {
    /// Refusals: the relationship is not made.
    pub(crate) errors: Vec<Finding>,
    /// Made, with these warnings (many-to-many, data types, made inactive).
    pub(crate) warnings: Vec<Finding>,
    /// Whether the relationship could be the active one.
    pub(crate) can_be_active: bool,
    /// Whether each end holds unique, non-blank values (`None`: no data in the session).
    pub(crate) from_unique: Option<bool>,
    pub(crate) to_unique: Option<bool>,
    /// The "one" ends checked against the data.
    pub(crate) data_checked: Vec<String>,
}

impl Verdict {
    /// The cardinality and cross filter the data suggests, as the dialog offers them.
    fn suggested(&self) -> Value {
        match (self.from_unique, self.to_unique) {
            (Some(true), Some(true)) => json!({"cardinality": "one_to_one", "cross_filter": "both"}),
            (Some(false), Some(true)) => json!({"cardinality": "many_to_one", "cross_filter": "single"}),
            (Some(true), Some(false)) => json!({"cardinality": "one_to_many", "cross_filter": "single"}),
            (Some(false), Some(false)) => json!({"cardinality": "many_to_many", "cross_filter": "single"}),
            _ => Value::Null,
        }
    }

    /// The answer of `check: true`.
    pub(crate) fn answer(&self, r: &Relationship) -> Value {
        let (fc, tc) = cards(r);
        json!({
            "check": true,
            "valid": self.errors.is_empty(),
            "errors": self.errors.iter().map(Finding::json).collect::<Vec<_>>(),
            "warnings": self.warnings.iter().map(Finding::json).collect::<Vec<_>>(),
            "from": dax_column(&r.from_table, &r.from_column),
            "to": dax_column(&r.to_table, &r.to_column),
            "from_cardinality": fc,
            "to_cardinality": tc,
            "cross_filter": if is_both(r.cross_filtering_behavior.as_deref()) { "both" } else { "single" },
            "active": r.is_active,
            "can_be_active": self.can_be_active,
            "from_unique": self.from_unique,
            "to_unique": self.to_unique,
            "suggested": self.suggested(),
            "data_checked": self.data_checked,
        })
    }

    /// Without `check`: the first refusal as the command's error.
    pub(crate) fn refusal(&self) -> Option<String> {
        self.errors.first().map(|f| f.message.clone())
    }
}

/// A relationship's cardinalities, defaults filled in (`many` → `one`).
pub(crate) fn cards(r: &Relationship) -> (String, String) {
    let c = |v: &Option<String>, d: &str| v.as_deref().unwrap_or(d).to_ascii_lowercase();
    (c(&r.from_cardinality, "many"), c(&r.to_cardinality, "one"))
}

pub(crate) fn is_both(v: Option<&str>) -> bool {
    v.is_some_and(|c| c.eq_ignore_ascii_case("bothDirections"))
}

fn rely(r: &Relationship) -> bool {
    r.extra.get("relyOnReferentialIntegrity").and_then(Value::as_bool) == Some(true)
}

/// Whether `table` is queried from its source (DirectQuery or Dual): where "Assume referential
/// integrity" applies.
fn queried_live(m: &SemanticModel, table: &str) -> bool {
    m.table(table)
        .is_some_and(|t| matches!(crate::storage_mode::table_mode(m, t), "directQuery" | "dual"))
}

/// Why `new`, made active in `next`, cannot be active: another active relationship between the
/// two tables, or a filter path `before` did not have that becomes ambiguous.
fn active_problem(before: &SemanticModel, next: &SemanticModel, new: &Relationship) -> Option<Finding> {
    let same_pair = |r: &Relationship| {
        (r.from_table.eq_ignore_ascii_case(&new.from_table) && r.to_table.eq_ignore_ascii_case(&new.to_table))
            || (r.from_table.eq_ignore_ascii_case(&new.to_table)
                && r.to_table.eq_ignore_ascii_case(&new.from_table))
    };
    if let Some(r) = next
        .relationships
        .iter()
        .find(|r| r.is_active && r.name != new.name && same_pair(r))
    {
        return Some(Finding::new(
            "active_exists",
            format!(
                "relationship `{}` is already the active one between `{}` and `{}`; make it inactive first",
                r.name, new.from_table, new.to_table
            ),
        ));
    }
    let (old, now) = (ambiguous_pairs(before)?, ambiguous_pairs(next)?);
    let added: Vec<String> = now
        .iter()
        .filter(|p| !old.contains(p))
        .map(|(a, b)| format!("`{a}` → `{b}`"))
        .collect();
    (!added.is_empty()).then(|| {
        Finding::new(
            "ambiguous",
            format!(
                "the relationship makes filter paths ambiguous: a filter could go from {} by two \
                 different active paths (make another relationship inactive or single-direction first)",
                added.join(", ")
            ),
        )
    })
}

/// Whether column `table[column]` holds unique, non-blank values, read from the session's data.
fn unique(s: &Session, table: &str, column: &str) -> Result<Option<Result<(), String>>, String> {
    Ok(crate::preview::column_values(s, table, column, MAX_CHECK_ROWS)?.map(|v| check_unique(&v)))
}

/// Inputs of a review.
pub(crate) struct Review<'a> {
    /// The model as it is.
    pub(crate) current: &'a SemanticModel,
    /// The relationship as it would be (`is_active` as asked, or true when not given).
    pub(crate) new: Relationship,
    /// Its index when it is edited (`relationship.set`), else it is added.
    pub(crate) index: Option<usize>,
    /// The relationship before the edit.
    pub(crate) old: Option<&'a Relationship>,
    /// `active` as passed.
    pub(crate) active_given: Option<bool>,
    /// Read both ends' data (the dialog's suggestion), not only the new "one" sides.
    pub(crate) read_both: bool,
}

/// Reviews a relationship the way the Create and Edit relationship dialogs validate it. Returns the
/// relationship as it is made (inactive when `active` was not given and it cannot be active).
pub(crate) fn review(s: &Session, r: Review<'_>) -> (Relationship, Verdict) {
    let Review {
        current,
        mut new,
        index,
        old,
        active_given,
        read_both,
    } = r;
    let mut v = Verdict::default();
    let (fc, tc) = cards(&new);
    if new.from_table.eq_ignore_ascii_case(&new.to_table) {
        v.errors.push(Finding::new(
            "same_table",
            format!(
                "a relationship needs two different tables (both ends are in `{}`)",
                new.from_table
            ),
        ));
    }
    if let Some(r) = current.relationships.iter().enumerate().find(|(i, r)| {
        Some(*i) != index
            && r.from_table.eq_ignore_ascii_case(&new.from_table)
            && r.from_column.eq_ignore_ascii_case(&new.from_column)
            && r.to_table.eq_ignore_ascii_case(&new.to_table)
            && r.to_column.eq_ignore_ascii_case(&new.to_column)
    }) {
        v.errors.push(Finding::new(
            "duplicate",
            format!(
                "relationship `{}` already links {} to {}",
                r.1.name,
                dax_column(&new.from_table, &new.from_column),
                dax_column(&new.to_table, &new.to_column)
            ),
        ));
    }
    if fc == "one" && tc == "many" {
        v.errors.push(Finding::new(
            "one_to_many",
            format!(
                "a one-to-many relationship is written from the many side: swap the ends (from {} and to {})",
                dax_column(&new.to_table, &new.to_column),
                dax_column(&new.from_table, &new.from_column)
            ),
        ));
    }
    if is_both(new.extra.get("securityFilteringBehavior").and_then(Value::as_str))
        && !is_both(new.cross_filtering_behavior.as_deref())
    {
        v.errors.push(Finding::new(
            "security_needs_both",
            "security filtering in both directions needs cross filtering in both directions \
             (cross_filter: \"both\")",
        ));
    }
    // data types
    let dtype = |t: &str, c: &str| {
        current
            .table(t)
            .and_then(|t| t.column(c))
            .map_or(DataType::Unknown, |c| c.data_type)
    };
    let (ft, tt) = (
        dtype(&new.from_table, &new.from_column),
        dtype(&new.to_table, &new.to_column),
    );
    if ft != tt && ft != DataType::Unknown && tt != DataType::Unknown {
        v.warnings.push(Finding::new(
            "data_types",
            format!(
                "the columns have different data types ({} and {})",
                ft.as_tmsl().unwrap_or("unknown"),
                tt.as_tmsl().unwrap_or("unknown")
            ),
        ));
    }
    if fc == "many" && tc == "many" {
        v.warnings.push(Finding::new(
            "many_to_many",
            "a many-to-many relationship: use it only when neither column holds unique values, \
             knowing that it filters differently from a many-to-one relationship",
        ));
    }
    // the data: unique "one" sides, and both ends for the suggestion
    let moved = |side_from: bool| {
        old.is_none_or(|o| {
            if side_from {
                !o.from_table.eq_ignore_ascii_case(&new.from_table)
                    || !o.from_column.eq_ignore_ascii_case(&new.from_column)
            } else {
                !o.to_table.eq_ignore_ascii_case(&new.to_table)
                    || !o.to_column.eq_ignore_ascii_case(&new.to_column)
            }
        })
    };
    let (ofc, otc) = old.map_or((String::new(), String::new()), cards);
    for (side, is_one, was_one, from_side) in [
        ("from", fc == "one", ofc == "one", true),
        ("to", tc == "one", otc == "one", false),
    ] {
        let (table, column) = if from_side {
            (new.from_table.clone(), new.from_column.clone())
        } else {
            (new.to_table.clone(), new.to_column.clone())
        };
        let needed = is_one && (!was_one || moved(from_side));
        if !needed && !read_both {
            continue;
        }
        let col = dax_column(&table, &column);
        match unique(s, &table, &column) {
            Ok(Some(res)) => {
                let ok = res.is_ok();
                if from_side {
                    v.from_unique = Some(ok);
                } else {
                    v.to_unique = Some(ok);
                }
                if is_one {
                    v.data_checked.push(col.clone());
                    if let Err(e) = res {
                        v.errors.push(Finding::new(
                            "not_unique",
                            format!(
                                "the {side} side {col} {e}; it cannot be the \"one\" side of the \
                                 relationship (the cardinality is not valid for this relationship)"
                            ),
                        ));
                    }
                }
            }
            Ok(None) => {
                if needed {
                    v.warnings.push(Finding::new(
                        "not_checked",
                        format!(
                            "{col} is a \"one\" side; its values were not checked (no data in the session)"
                        ),
                    ));
                }
            }
            Err(e) => {
                if needed {
                    v.warnings.push(Finding::new(
                        "not_checked",
                        format!("{col} is a \"one\" side; its values could not be read: {e}"),
                    ));
                }
            }
        }
    }
    // a "one" side with duplicates: whether the other end is unique tells which message applies
    if v.errors.iter().any(|f| f.code == "not_unique") {
        let missing = [
            (
                v.from_unique.is_none(),
                new.from_table.clone(),
                new.from_column.clone(),
                true,
            ),
            (
                v.to_unique.is_none(),
                new.to_table.clone(),
                new.to_column.clone(),
                false,
            ),
        ];
        for (unread, table, column, from_side) in missing {
            if unread && let Ok(Some(res)) = unique(s, &table, &column) {
                if from_side {
                    v.from_unique = Some(res.is_ok());
                } else {
                    v.to_unique = Some(res.is_ok());
                }
            }
        }
    }
    if v.from_unique == Some(false) && v.to_unique == Some(false) && !(fc == "many" && tc == "many") {
        // Learn: one of the columns must have unique values
        v.errors.retain(|f| f.code != "not_unique");
        v.errors.push(Finding::new(
            "no_unique_side",
            "one of the columns must have unique values: neither column does (choose many-to-many, \
             or relate a table of the distinct keys)",
        ));
    }
    // Assume referential integrity: DirectQuery, not many-to-many
    if rely(&new) {
        if !(queried_live(current, &new.from_table) && queried_live(current, &new.to_table)) {
            v.errors.push(Finding::new(
                "referential_integrity",
                "assume referential integrity applies to DirectQuery (or Dual) tables only",
            ));
        } else if fc == "many" && tc == "many" {
            v.errors.push(Finding::new(
                "referential_integrity",
                "assume referential integrity does not apply to a many-to-many relationship",
            ));
        }
    }
    // activity
    let mut next = current.clone();
    let mut active = new.clone();
    active.is_active = true;
    match index.and_then(|i| next.relationships.get_mut(i)) {
        Some(slot) => *slot = active.clone(),
        None => next.relationships.push(active.clone()),
    }
    let problem = if v.errors.iter().any(|f| f.code == "same_table") {
        None
    } else {
        active_problem(current, &next, &active)
    };
    v.can_be_active = problem.is_none();
    if ambiguous_pairs(&next).is_none() {
        v.warnings.push(Finding::new(
            "too_many_paths",
            "the model has too many relationship paths to check them for ambiguity",
        ));
    }
    if let Some(p) = problem {
        let was_active = old.is_some_and(|o| o.is_active);
        match active_given {
            None if old.is_none() => {
                new.is_active = false;
                v.warnings.push(Finding::new(
                    "made_inactive",
                    format!("the relationship is made inactive: {}", p.message),
                ));
            }
            None if !was_active => {}
            Some(false) => {}
            _ if new.is_active => v.errors.push(p),
            _ => {}
        }
    }
    (new, v)
}

fn parse_cardinality(key: &str, v: Option<&str>) -> Result<Option<String>, String> {
    match v {
        None => Ok(None),
        Some(c) if c.eq_ignore_ascii_case("one") => Ok(Some("one".into())),
        Some(c) if c.eq_ignore_ascii_case("many") => Ok(Some("many".into())),
        Some(c) => Err(format!("unknown {key} `{c}` (expected one or many)")),
    }
}

pub(crate) fn parse_direction(
    key: &str,
    v: Option<&str>,
    none: bool,
) -> Result<Option<&'static str>, String> {
    match v {
        None => Ok(None),
        Some(c) if c.eq_ignore_ascii_case("single") || c.eq_ignore_ascii_case("oneDirection") => {
            Ok(Some("oneDirection"))
        }
        Some(c) if c.eq_ignore_ascii_case("both") || c.eq_ignore_ascii_case("bothDirections") => {
            Ok(Some("bothDirections"))
        }
        Some(c) if none && c.eq_ignore_ascii_case("none") => Ok(Some("none")),
        Some(c) => Err(format!(
            "unknown {key} `{c}` (expected single{})",
            if none { ", both or none" } else { " or both" }
        )),
    }
}

/// Resolves a column reference to the model's spelling of its table and column.
pub(crate) fn resolve_column(m: &SemanticModel, text: &str) -> Result<(String, String), String> {
    let (table, column) = parse_column_ref(text)?;
    let t = m
        .table(&table)
        .ok_or_else(|| format!("no table named `{table}`"))?;
    let c = t
        .column(&column)
        .ok_or_else(|| format!("table `{}` has no column `{column}`", t.name))?;
    Ok((t.name.clone(), c.name.clone()))
}

/// Sets or clears `assume_referential_integrity` and `security_filtering` on `r`.
pub(crate) fn set_flags(r: &mut Relationship, rely: Option<bool>, security: Option<&str>) {
    match rely {
        Some(true) => {
            r.extra.insert("relyOnReferentialIntegrity".into(), json!(true));
        }
        Some(false) => {
            r.extra.remove("relyOnReferentialIntegrity");
        }
        None => {}
    }
    match security {
        Some("oneDirection") => {
            r.extra.remove("securityFilteringBehavior");
        }
        Some(sf) => {
            r.extra.insert("securityFilteringBehavior".into(), json!(sf));
        }
        None => {}
    }
}

/// `relationship.add`.
pub(crate) fn relationship_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "from",
            "to",
            "active",
            "cross_filter",
            "from_cardinality",
            "to_cardinality",
            "security_filtering",
            "assume_referential_integrity",
            "name",
            "check",
        ],
    )?;
    let given_name = opt_str(p, "name")?;
    if let Some(n) = given_name {
        check_name("relationship", n)?;
    }
    let from = str_param(p, "from")?;
    let to = str_param(p, "to")?;
    let active = opt_bool(p, "active")?;
    let check = opt_bool(p, "check")?.unwrap_or(false);
    let cross_filter = parse_direction("cross_filter", opt_str(p, "cross_filter")?, false)?;
    let security = parse_direction("security_filtering", opt_str(p, "security_filtering")?, true)?;
    let rely = opt_bool(p, "assume_referential_integrity")?;
    let from_cardinality = parse_cardinality("from_cardinality", opt_str(p, "from_cardinality")?)?;
    let to_cardinality = parse_cardinality("to_cardinality", opt_str(p, "to_cardinality")?)?;
    // Power BI Desktop only has one-to-one relationships that cross filter in both directions
    // (it refuses to open a file with a single-direction one): default to both, refuse single.
    let one_to_one = from_cardinality.as_deref() == Some("one") && to_cardinality.as_deref() == Some("one");
    let both_by_default = one_to_one && cross_filter.is_none();
    let cross_filter = match (one_to_one, cross_filter) {
        (true, Some("oneDirection")) => return Err(crate::edit::ONE_TO_ONE_SINGLE.into()),
        (true, _) => Some("bothDirections"),
        (false, c) => c,
    };
    let current = model(s)?;
    let (ft, fc) = resolve_column(current, from)?;
    let (tt, tc) = resolve_column(current, to)?;
    let name = match given_name {
        Some(n) => {
            if let Some(r) = current
                .relationships
                .iter()
                .find(|r| r.name.eq_ignore_ascii_case(n))
            {
                return Err(format!("a relationship named `{}` already exists", r.name));
            }
            n.to_string()
        }
        None => {
            let mut name = new_guid();
            while current
                .relationships
                .iter()
                .any(|r| r.name.eq_ignore_ascii_case(&name))
            {
                name = new_guid();
            }
            name
        }
    };
    let mut new = Relationship {
        name,
        from_table: ft,
        from_column: fc,
        to_table: tt,
        to_column: tc,
        is_active: active.unwrap_or(true),
        cross_filtering_behavior: cross_filter.map(str::to_string),
        from_cardinality,
        to_cardinality,
        ..Default::default()
    };
    set_flags(&mut new, rely, security);
    let (new, verdict) = review(
        s,
        Review {
            current,
            new,
            index: None,
            old: None,
            active_given: active,
            read_both: check,
        },
    );
    if check {
        return Ok(verdict.answer(&new));
    }
    if let Some(e) = verdict.refusal() {
        return Err(e);
    }
    apply(s, p, move |m| {
        let mut edit = Edit::new(json!({
            "name": new.name,
            "from": dax_column(&new.from_table, &new.from_column),
            "to": dax_column(&new.to_table, &new.to_column),
            "active": new.is_active,
            "data_checked": verdict.data_checked,
        }));
        edit.notes = verdict.warnings.iter().map(|f| f.message.clone()).collect();
        if both_by_default {
            edit.notes
                .push("a one-to-one relationship cross filters in both directions".into());
        }
        m.relationships.push(new);
        Ok(edit)
    })
}
