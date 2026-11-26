//! Data-role checks for visual bindings (G14): the role names, kinds and field counts each
//! built-in visual type accepts, from the capability table of `bicraft_visuals`
//! (`docs/formats/visual-roles-bb.md`).
//!
//! - unknown role → error, with a "did you mean";
//! - a measure in a role that takes columns only → error;
//! - more fields than a role, or any accepted combination, allows → error;
//! - a role spelled with another case than Desktop's → warning;
//! - nothing in the roles a visual needs to draw → warning (Desktop shows an empty visual); a
//!   visual with no field at all is a placeholder and is not reported.
//!
//! Custom visuals and the built-in types whose roles are not known are not checked.

use std::collections::BTreeMap;

use bicraft_model::SemanticModel;
use bicraft_report::{FieldBinding, FieldRef};
use bicraft_visuals::{self as visuals, DataRole, RoleBound, RoleCondition, RoleKind, VisualType};
use serde_json::Value;

/// Most accepted combinations a message lists.
const MAX_LISTED: usize = 6;

/// One finding about a visual's bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RoleIssue {
    pub error: bool,
    /// The role it is about, when it is about one.
    pub role: Option<String>,
    pub message: String,
}

impl RoleIssue {
    fn error(role: Option<&str>, message: String) -> Self {
        Self {
            error: true,
            role: role.map(str::to_string),
            message,
        }
    }

    fn warning(role: Option<&str>, message: String) -> Self {
        Self {
            error: false,
            role: role.map(str::to_string),
            message,
        }
    }
}

/// The catalog entry of a type whose roles are known (`None`: custom or unchecked type).
fn known_type(visual_type: Option<&str>) -> Option<&'static VisualType> {
    visuals::visual(visual_type?).filter(|v| v.roles_known)
}

/// Desktop's spelling of `role` for `visual_type` when it differs only by case.
pub(crate) fn canonical_role(visual_type: Option<&str>, role: &str) -> Option<&'static str> {
    known_type(visual_type)?
        .data_roles
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case(role))
        .map(|r| r.name.as_str())
}

/// Desktop's field-well captions people use instead of role names, for "did you mean" hints.
const ALIASES: &[(&str, &[&str])] = &[
    ("legend", &["Series"]),
    ("axis", &["Category"]),
    ("xaxis", &["Category", "X"]),
    ("x-axis", &["Category", "X"]),
    ("yaxis", &["Y"]),
    ("y-axis", &["Y"]),
    ("values", &["Y", "Data", "Value"]),
    ("value", &["Y", "Values", "Data"]),
    ("fields", &["Values", "Data"]),
    ("field", &["Values"]),
    ("smallmultiples", &["Rows"]),
    ("small multiples", &["Rows"]),
    ("details", &["Series"]),
    ("location", &["Category"]),
    ("latitude", &["Y"]),
    ("longitude", &["X"]),
    ("secondaryvalues", &["Y2"]),
    ("lineyaxis", &["Y2"]),
    ("tooltip", &["Tooltips"]),
    ("target", &["TargetValue"]),
];

fn did_you_mean(v: &VisualType, role: &str) -> Option<String> {
    let names = || v.data_roles.iter().map(|r| r.name.as_str());
    let lower = role.to_ascii_lowercase();
    let alias = ALIASES
        .iter()
        .find(|(a, _)| *a == lower)
        .and_then(|(_, to)| to.iter().find(|t| names().any(|n| n == **t)).copied());
    alias
        .or_else(|| visuals::suggest(role, names()))
        .map(str::to_string)
}

/// Whether a binding is a measure (model or report-level), a column, or unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    Measure,
    Other,
}

fn field_kind(f: &FieldRef) -> FieldKind {
    match f {
        FieldRef::Measure { .. } => FieldKind::Measure,
        FieldRef::Other { raw } if raw.get("Measure").is_some_and(Value::is_object) => FieldKind::Measure,
        _ => FieldKind::Other,
    }
}

/// The field as messages show it; report-level measures as `report-level measure [Name]`.
fn binding_text(f: &FieldRef) -> String {
    match f {
        FieldRef::Other { raw } => match raw
            .get("Measure")
            .and_then(|m| m.get("Property"))
            .and_then(Value::as_str)
        {
            Some(name) => format!("report-level measure {}", crate::edit::dax_measure(name)),
            None => crate::refs::field_text(f),
        },
        _ => crate::refs::field_text(f),
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "field" } else { "fields" }
}

/// Checks `fields` against the data roles of `visual_type`. Empty for custom visuals, groups and
/// types whose roles are not known.
pub(crate) fn check_roles(visual_type: Option<&str>, fields: &[FieldBinding]) -> Vec<RoleIssue> {
    let mut out = Vec::new();
    // Built-in visuals without data roles whose fields Desktop 2.158 ignores (no error).
    if let Some(t @ ("qnaVisual" | "aiNarratives")) = visual_type {
        if t == "qnaVisual" {
            out.push(RoleIssue::warning(
                None,
                "Desktop 2.158 announces that Q&A visuals are retired in February 2027".into(),
            ));
        }
        if !fields.is_empty() {
            out.push(RoleIssue::warning(
                fields.first().map(|b| b.role.as_str()),
                format!(
                    "`{t}` has no data roles: Desktop ignores its {} {} ({})",
                    fields.len(),
                    plural(fields.len()),
                    if t == "qnaVisual" {
                        "it answers its saved question, `hiddenProperties.savedUtterance`"
                    } else {
                        "it writes its narrative from the page's visuals"
                    }
                ),
            ));
        }
        return out;
    }
    let Some(v) = known_type(visual_type) else {
        return out;
    };
    let t = v.name.as_str();
    if v.data_roles.is_empty() {
        if !fields.is_empty() {
            out.push(RoleIssue::error(
                fields.first().map(|b| b.role.as_str()),
                format!(
                    "`{t}` takes no data fields; remove its {} {}",
                    fields.len(),
                    plural(fields.len())
                ),
            ));
        }
        return out;
    }
    // Field counts per role, by Desktop's spelling.
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut unknown: Vec<&str> = Vec::new();
    for b in fields {
        let Some(role) = v.data_roles.iter().find(|r| r.name.eq_ignore_ascii_case(&b.role)) else {
            if !unknown.iter().any(|u| u.eq_ignore_ascii_case(&b.role)) {
                unknown.push(&b.role);
            }
            continue;
        };
        if role.name != b.role && !counts.contains_key(role.name.as_str()) {
            out.push(RoleIssue::warning(
                Some(&b.role),
                format!("role `{}` of `{t}` is spelled `{}` by Desktop", b.role, role.name),
            ));
        }
        let n = counts.entry(role.name.as_str()).or_default();
        *n = n.saturating_add(1);
        if role.kind == RoleKind::Grouping && field_kind(&b.field) == FieldKind::Measure {
            out.push(RoleIssue::error(
                Some(&role.name),
                format!(
                    "{} is a measure, but role `{}` of `{t}` takes columns only",
                    binding_text(&b.field),
                    role.name
                ),
            ));
        }
    }
    for role in unknown {
        let hint = did_you_mean(v, role)
            .map(|s| format!("; did you mean `{s}`?"))
            .unwrap_or_default();
        out.push(RoleIssue::error(
            Some(role),
            format!(
                "`{t}` has no data role `{role}`{hint} (its roles: {})",
                v.roles.join(", ")
            ),
        ));
    }
    let mut too_many = false;
    for role in &v.data_roles {
        let n = counts.get(role.name.as_str()).copied().unwrap_or(0);
        if let Some(max) = role.max
            && n > max as usize
        {
            too_many = true;
            out.push(RoleIssue::error(
                Some(&role.name),
                if max == 0 {
                    format!("role `{}` of `{t}` takes no fields; it has {n}", role.name)
                } else {
                    format!(
                        "role `{}` of `{t}` takes at most {max} {}; it has {n}",
                        role.name,
                        plural(max as usize)
                    )
                },
            ));
        }
    }
    // A visual without any field is a placeholder still to be bound: not reported.
    if fields.is_empty() {
        return out;
    }
    let mut needs_warned = false;
    for group in &v.required_roles {
        if group
            .iter()
            .all(|r| counts.get(r.as_str()).copied().unwrap_or(0) == 0)
        {
            needs_warned = true;
            out.push(RoleIssue::warning(
                group.first().map(String::as_str),
                format!(
                    "the `{t}` needs a field in {}; Desktop shows it empty",
                    or_list(group)
                ),
            ));
        }
    }
    if too_many || v.role_conditions.is_empty() {
        return out;
    }
    // Drill roles hold one field per drill level; the bounds apply per level.
    let effective = |role: &str| -> usize {
        let n = counts.get(role).copied().unwrap_or(0);
        match v.data_roles.iter().find(|r| r.name == role) {
            Some(DataRole { drill: true, .. }) => n.min(1),
            _ => n,
        }
    };
    // Fields of the role whose kind a condition's `kind` rules out.
    let wrong_kind = |b: &RoleBound| -> Vec<&FieldBinding> {
        match b.kind {
            Some(k @ (RoleKind::Grouping | RoleKind::Measure)) => fields
                .iter()
                .filter(|f| f.role.eq_ignore_ascii_case(&b.role))
                .filter(|f| binding_kind(&f.field).is_some_and(|fk| fk != k))
                .collect(),
            _ => Vec::new(),
        }
    };
    // (too many, wrong kind, too few, forbidden roles used): a condition holds when all are 0.
    let score = |c: &RoleCondition| -> (usize, usize, usize, usize) {
        let (mut over, mut kinds, mut under, mut forbidden) = (0usize, 0usize, 0usize, 0usize);
        for b in &c.bounds {
            let n = effective(&b.role);
            if b.max.is_some_and(|m| n > m as usize) {
                over += 1;
                if b.max == Some(0) {
                    forbidden += 1;
                }
            }
            if n < b.min as usize {
                under += 1;
            }
            if !wrong_kind(b).is_empty() {
                kinds += 1;
            }
        }
        (over, kinds, under, forbidden)
    };
    if v.role_conditions.iter().any(|c| score(c) == (0, 0, 0, 0)) {
        return out;
    }
    let Some(best) = v.role_conditions.iter().min_by_key(|c| score(c)) else {
        return out;
    };
    let (over, kinds, _, _) = score(best);
    let accepted = accepted_list(&v.role_conditions);
    if over == 0 && kinds > 0 {
        // Desktop shows a data-mapping error in the visual ("conditionKindExpected…").
        let wrong: Vec<String> = best
            .bounds
            .iter()
            .filter_map(|b| {
                let bad = wrong_kind(b);
                let first = bad.first()?;
                Some(match b.kind {
                    Some(RoleKind::Measure) => format!(
                        "`{}` needs a measure or a summarized column, but {} is a column that is not summarized",
                        b.role,
                        binding_text(&first.field)
                    ),
                    _ => format!(
                        "`{}` needs a column that is not summarized, but {} is {}",
                        b.role,
                        binding_text(&first.field),
                        if field_kind(&first.field) == FieldKind::Measure {
                            "a measure"
                        } else {
                            "summarized"
                        }
                    ),
                })
            })
            .collect();
        let when = when_text(best, &effective);
        out.push(RoleIssue::error(
            best.bounds
                .iter()
                .find(|b| !wrong_kind(b).is_empty())
                .map(|b| b.role.as_str()),
            format!(
                "the `{t}` cannot draw these fields: {}{when}; Desktop shows an error in the visual (accepted combinations: {accepted})",
                wrong.join(", ")
            ),
        ));
        return out;
    }
    if over > 0 {
        let violations: Vec<String> = best
            .bounds
            .iter()
            .filter_map(|b| {
                let n = effective(&b.role);
                let max = b.max?;
                (n > max as usize).then(|| {
                    if max == 0 {
                        format!("`{}` must be empty", b.role)
                    } else {
                        format!("`{}` takes at most {max} {}", b.role, plural(max as usize))
                    }
                })
            })
            .collect();
        let when = when_text(best, &effective);
        out.push(RoleIssue::error(
            best.bounds
                .iter()
                .find(|b| b.max.is_some_and(|m| effective(&b.role) > m as usize))
                .map(|b| b.role.as_str()),
            format!(
                "too many fields for a `{t}`: {}{when} (accepted combinations: {accepted})",
                violations.join(", ")
            ),
        ));
    } else if !needs_warned {
        let missing: Vec<String> = best
            .bounds
            .iter()
            .filter(|b| effective(&b.role) < b.min as usize)
            .map(|b| format!("`{}` needs at least {} {}", b.role, b.min, plural(b.min as usize)))
            .collect();
        out.push(RoleIssue::warning(
            best.bounds
                .iter()
                .find(|b| effective(&b.role) < b.min as usize)
                .map(|b| b.role.as_str()),
            format!(
                "the `{t}` is missing fields: {}; Desktop shows it empty (accepted combinations: {accepted})",
                missing.join(", ")
            ),
        ));
    }
    out
}

/// ` when `Series` has a field…`: the bounds of `c` with a minimum that the fields meet.
fn when_text(c: &RoleCondition, effective: &dyn Fn(&str) -> usize) -> String {
    let when: Vec<String> = c
        .bounds
        .iter()
        .filter(|b| b.min > 0 && effective(&b.role) >= b.min as usize)
        .filter(|b| b.max.is_none_or(|m| effective(&b.role) <= m as usize))
        .map(|b| format!("`{}` has {}", b.role, describe_count(effective(&b.role))))
        .collect();
    if when.is_empty() {
        String::new()
    } else {
        format!(" when {}", when.join(" and "))
    }
}

/// What a bound field is for the `kind` of a condition: a measure (model or report-level
/// measure, aggregation, visual calculation) or a grouping (a column or hierarchy level that is
/// not summarized). `None`: not known, never counted as the wrong kind.
fn binding_kind(f: &FieldRef) -> Option<RoleKind> {
    match f {
        FieldRef::Measure { .. } | FieldRef::Aggregation { .. } => Some(RoleKind::Measure),
        FieldRef::Column { .. } => Some(RoleKind::Grouping),
        FieldRef::Other { raw } => {
            let o = raw.as_object()?;
            if o.contains_key("Measure")
                || o.contains_key("Aggregation")
                || o.contains_key("NativeVisualCalculation")
            {
                Some(RoleKind::Measure)
            } else if o.contains_key("Column") || o.contains_key("HierarchyLevel") {
                Some(RoleKind::Grouping)
            } else {
                None
            }
        }
    }
}

fn describe_count(n: usize) -> String {
    if n == 1 {
        "a field".into()
    } else {
        format!("{n} fields")
    }
}

fn or_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => format!("`{one}`"),
        [init @ .., last] => format!(
            "{} or `{last}`",
            init.iter()
                .map(|s| format!("`{s}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// `[Series=0] or [Series=1, Y≤1]…`, the bounds of each accepted combination.
fn accepted_list(conditions: &[RoleCondition]) -> String {
    let mut parts: Vec<String> = conditions
        .iter()
        .take(MAX_LISTED)
        .map(|c| {
            let bounds: Vec<String> = c
                .bounds
                .iter()
                .filter_map(|b| {
                    let kind = match b.kind {
                        Some(RoleKind::Measure) => " summarized",
                        Some(RoleKind::Grouping) => " not summarized",
                        _ => "",
                    };
                    match (b.min, b.max) {
                        (0, None) if kind.is_empty() => None,
                        (0, None) => Some(format!("{}{kind}", b.role)),
                        (min, Some(max)) if min == max => Some(format!("{}={min}{kind}", b.role)),
                        (0, Some(max)) => Some(format!("{}≤{max}{kind}", b.role)),
                        (min, None) => Some(format!("{}≥{min}{kind}", b.role)),
                        (min, Some(max)) => Some(format!("{min}≤{}≤{max}{kind}", b.role)),
                    }
                })
                .collect();
            format!("[{}]", bounds.join(", "))
        })
        .collect();
    if conditions.len() > MAX_LISTED {
        parts.push(format!("… ({} in all)", conditions.len()));
    }
    parts.join(" or ")
}

/// Rewrites role names that differ from Desktop's only by case to Desktop's spelling.
pub(crate) fn normalize_roles(visual_type: Option<&str>, fields: &mut [FieldBinding]) {
    for b in fields {
        if let Some(c) = canonical_role(visual_type, &b.role)
            && c != b.role
        {
            b.role = c.to_string();
        }
    }
}

/// [`check_roles`], plus the checks that need the model: the fields of a role that must be
/// hierarchically related (multi-field slicers, list slicers).
pub(crate) fn check_bindings(
    model: Option<&SemanticModel>,
    visual_type: Option<&str>,
    fields: &[FieldBinding],
) -> Vec<RoleIssue> {
    let mut out = check_roles(visual_type, fields);
    if let Some(m) = model {
        out.extend(check_related(m, visual_type, fields));
    }
    out
}

/// Most steps the relationship search takes before giving up (and reporting nothing).
const MAX_PATH_STEPS: usize = 100_000;

/// The roles whose fields must be "hierarchically related" (`areHierarchicallyRelated` in a
/// condition): one table, or tables Desktop can walk along one chain of active relationships
/// from the first field's table (`docs/formats/visual-roles-bb.md`, "Kinds and relationships").
/// Desktop 2.158 shows "fields that don't have a supported set of relationships" otherwise.
fn check_related(
    model: &SemanticModel,
    visual_type: Option<&str>,
    fields: &[FieldBinding],
) -> Vec<RoleIssue> {
    let mut out = Vec::new();
    let Some(v) = known_type(visual_type) else {
        return out;
    };
    let mut roles: Vec<&str> = v
        .role_conditions
        .iter()
        .flat_map(|c| c.bounds.iter())
        .filter(|b| b.related)
        .map(|b| b.role.as_str())
        .collect();
    roles.dedup();
    for role in roles {
        let in_role: Vec<&FieldBinding> = fields
            .iter()
            .filter(|f| f.role.eq_ignore_ascii_case(role))
            .collect();
        if in_role.len() < 2 {
            continue;
        }
        // Tables in the order of the fields, each once; an unresolved field skips the check
        // (the field checks report it).
        let mut tables: Vec<usize> = Vec::new();
        let mut names: Vec<&str> = Vec::new();
        let mut resolved = true;
        for f in &in_role {
            match binding_table(&f.field)
                .and_then(|t| model.tables.iter().position(|x| x.name.eq_ignore_ascii_case(t)))
            {
                Some(i) => {
                    if !tables.contains(&i) {
                        tables.push(i);
                        names.push(model.tables.get(i).map_or("", |t| t.name.as_str()));
                    }
                }
                None => resolved = false,
            }
        }
        if !resolved || tables.len() < 2 || hierarchically_related(model, &tables) != Some(false) {
            continue;
        }
        out.push(RoleIssue::error(
            Some(role),
            format!(
                "the fields of role `{role}` of the `{}` come from tables that no chain of active relationships links from `{}` ({}); Desktop shows \"fields that don't have a supported set of relationships\"",
                v.name,
                names.first().copied().unwrap_or_default(),
                names.join(", ")
            ),
        ));
    }
    out
}

/// The model table a field reads (`None`: not a plain column, aggregation, measure or level of a
/// table's own hierarchy).
fn binding_table(f: &FieldRef) -> Option<&str> {
    match f {
        FieldRef::Column { table, .. }
        | FieldRef::Aggregation { table, .. }
        | FieldRef::Measure { table, .. } => Some(table),
        FieldRef::Other { raw } => {
            let source = raw
                .get("Column")
                .and_then(|c| c.get("Expression"))
                .or_else(|| {
                    raw.get("HierarchyLevel")
                        .and_then(|h| h.get("Expression"))
                        .and_then(|h| h.get("Hierarchy"))
                        .and_then(|h| h.get("Expression"))
                })?
                .get("SourceRef")?;
            if source.get("Schema").is_some() {
                return None;
            }
            source.get("Entity").and_then(Value::as_str)
        }
    }
}

/// Desktop's "hierarchically related" test over model tables (indexes into `model.tables`, in
/// field order). Edges follow active relationships from their one side to their many side
/// (both ways for one-to-one); many-to-many relationships give none. Starting from the first
/// table, one chain along the edges must reach every other table, or the tables it misses must
/// all be reached by one chain walking the edges backwards. `None`: the search ran out of steps.
fn hierarchically_related(model: &SemanticModel, tables: &[usize]) -> Option<bool> {
    let n = model.tables.len();
    let index = |name: &str| {
        model
            .tables
            .iter()
            .position(|t| t.name.eq_ignore_ascii_case(name))
    };
    let mut forward: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut backward: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut edge = |from: usize, to: usize| {
        if let Some(f) = forward.get_mut(from)
            && !f.contains(&to)
        {
            f.push(to);
        }
        if let Some(b) = backward.get_mut(to)
            && !b.contains(&from)
        {
            b.push(from);
        }
    };
    for r in model.relationships.iter().filter(|r| r.is_active) {
        let (Some(from), Some(to)) = (index(&r.from_table), index(&r.to_table)) else {
            continue;
        };
        let many = |c: &Option<String>, default_many: bool| {
            c.as_deref()
                .map_or(default_many, |c| c.eq_ignore_ascii_case("many"))
        };
        match (many(&r.from_cardinality, true), many(&r.to_cardinality, false)) {
            (true, false) => edge(to, from),
            (false, true) => edge(from, to),
            (false, false) => {
                edge(to, from);
                edge(from, to);
            }
            (true, true) => {}
        }
    }
    let (first, rest) = tables.split_first()?;
    let mut steps = 0usize;
    let goal: Vec<usize> = rest.to_vec();
    let fwd = PathSearch::new(&forward).run(*first, &goal, &mut steps)?;
    if fwd.iter().any(Vec::is_empty) && fwd.len() == 1 {
        return Some(true);
    }
    let mut missed: Vec<usize> = fwd.iter().flatten().copied().collect();
    missed.sort_unstable();
    missed.dedup();
    let bwd = PathSearch::new(&backward).run(*first, &missed, &mut steps)?;
    if bwd.len() == 1 && bwd.iter().all(Vec::is_empty) {
        return Some(true);
    }
    Some(
        fwd.iter()
            .any(|s| bwd.iter().any(|r| !s.iter().any(|t| r.contains(t)))),
    )
}

/// Depth-first walk of single chains from a table, as Desktop does it: each result is the set of
/// target tables one chain could not reach; a chain that reaches them all ends the search.
struct PathSearch<'a> {
    edges: &'a [Vec<usize>],
    active: std::collections::HashSet<(usize, Vec<usize>)>,
    done: std::collections::HashMap<(usize, Vec<usize>), Vec<Vec<usize>>>,
}

impl<'a> PathSearch<'a> {
    fn new(edges: &'a [Vec<usize>]) -> Self {
        Self {
            edges,
            active: Default::default(),
            done: Default::default(),
        }
    }

    fn run(mut self, start: usize, targets: &[usize], steps: &mut usize) -> Option<Vec<Vec<usize>>> {
        let mut t = targets.to_vec();
        t.sort_unstable();
        self.visit(start, t, steps, 0)
    }

    fn visit(
        &mut self,
        at: usize,
        left: Vec<usize>,
        steps: &mut usize,
        depth: usize,
    ) -> Option<Vec<Vec<usize>>> {
        *steps = steps.saturating_add(1);
        if *steps > MAX_PATH_STEPS || depth > self.edges.len().saturating_add(1) {
            return None;
        }
        if left.is_empty() {
            return Some(vec![left]);
        }
        let key = (at, left.clone());
        if let Some(r) = self.done.get(&key) {
            return Some(r.clone());
        }
        if self.active.contains(&key) {
            return Some(Vec::new());
        }
        self.active.insert(key.clone());
        let next: Vec<usize> = self.edges.get(at).cloned().unwrap_or_default();
        let mut results: Vec<Vec<usize>> = Vec::new();
        let mut complete = None;
        for n in next {
            let rest: Vec<usize> = left.iter().copied().filter(|&x| x != n).collect();
            let r = self.visit(n, rest, steps, depth.saturating_add(1))?;
            if r.len() == 1 && r.iter().all(Vec::is_empty) {
                complete = Some(r);
                break;
            }
            results.extend(r);
        }
        let out = complete.unwrap_or_else(|| if results.is_empty() { vec![left] } else { results });
        self.active.remove(&key);
        self.done.insert(key, out.clone());
        Some(out)
    }
}

/// The issues of `after` that `before` does not have (edits are judged on what they add).
pub(crate) fn new_issues(before: &[RoleIssue], after: Vec<RoleIssue>) -> Vec<RoleIssue> {
    after.into_iter().filter(|i| !before.contains(i)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(role: &str, c: &str) -> FieldBinding {
        FieldBinding {
            display_name: None,
            show_as: None,
            role: role.into(),
            field: FieldRef::Column {
                table: "T".into(),
                column: c.into(),
            },
        }
    }

    fn mea(role: &str, m: &str) -> FieldBinding {
        FieldBinding {
            display_name: None,
            show_as: None,
            role: role.into(),
            field: FieldRef::Measure {
                table: "T".into(),
                measure: m.into(),
            },
        }
    }

    fn errors(t: &str, f: &[FieldBinding]) -> Vec<String> {
        check_roles(Some(t), f)
            .into_iter()
            .filter(|i| i.error)
            .map(|i| i.message)
            .collect()
    }

    fn warnings(t: &str, f: &[FieldBinding]) -> Vec<String> {
        check_roles(Some(t), f)
            .into_iter()
            .filter(|i| !i.error)
            .map(|i| i.message)
            .collect()
    }

    #[test]
    fn a_valid_line_chart_is_clean() {
        let f = [
            col("Category", "Date"),
            mea("Y", "Sales"),
            mea("Y", "Cost"),
            mea("Tooltips", "N"),
        ];
        assert!(check_roles(Some("lineChart"), &f).is_empty());
        // A date hierarchy in the drill role counts per level.
        let f = [
            col("Category", "Year"),
            col("Category", "Month"),
            col("Series", "Region"),
            mea("Y", "Sales"),
        ];
        assert!(
            check_roles(Some("lineChart"), &f).is_empty(),
            "{:?}",
            check_roles(Some("lineChart"), &f)
        );
    }

    #[test]
    fn unknown_roles_suggest() {
        let e = errors("lineChart", &[col("Legend", "Region"), mea("Y", "S")]);
        assert!(e[0].contains("did you mean `Series`"), "{e:?}");
        let e = errors("clusteredColumnChart", &[col("Categry", "A"), mea("Y", "S")]);
        assert!(e[0].contains("did you mean `Category`"), "{e:?}");
        let e = errors("card", &[mea("Data", "S")]);
        assert!(e[0].contains("its roles: Values"), "{e:?}");
    }

    #[test]
    fn too_many_fields() {
        let e = errors(
            "lineChart",
            &[col("Series", "A"), col("Series", "B"), mea("Y", "S")],
        );
        assert!(e[0].contains("takes at most 1 field; it has 2"), "{e:?}");
        let e = errors("card", &[mea("Values", "A"), mea("Values", "B")]);
        assert_eq!(e.len(), 1, "{e:?}");
        // Legend + two measures: the combination fails.
        let e = errors(
            "lineChart",
            &[
                col("Category", "D"),
                col("Series", "A"),
                mea("Y", "S"),
                mea("Y", "T"),
            ],
        );
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(
            e[0].contains("`Y` takes at most 1 field when `Series` has a field"),
            "{e:?}"
        );
        assert!(e[0].contains("accepted combinations"), "{e:?}");
        // The slicer's drill role takes a hierarchy.
        assert!(errors("slicer", &[col("Values", "A"), col("Values", "B")]).is_empty());
        // A pie with a legend and two values.
        let e = errors(
            "pieChart",
            &[
                col("Category", "C"),
                col("Series", "S"),
                mea("Y", "A"),
                mea("Y", "B"),
            ],
        );
        assert_eq!(e.len(), 1, "{e:?}");
    }

    #[test]
    fn measures_in_grouping_roles() {
        let e = errors("clusteredColumnChart", &[mea("Category", "Total"), mea("Y", "S")]);
        assert!(
            e[0].contains("[Total] is a measure, but role `Category`"),
            "{e:?}"
        );
        // Columns in measure roles are aggregated by Desktop: fine.
        assert!(errors("card", &[col("Values", "Amount")]).is_empty());
        // Report-level measures are measures too.
        let ext = FieldBinding {
            display_name: None,
            show_as: None,
            role: "Category".into(),
            field: FieldRef::Other {
                raw: serde_json::json!({"Measure": {"Expression": {"SourceRef": {"Schema": "extension", "Entity": "T"}}, "Property": "M"}}),
            },
        };
        let e = errors("clusteredColumnChart", &[ext, mea("Y", "S")]);
        assert!(e[0].contains("report-level measure [M] is a measure"), "{e:?}");
    }

    #[test]
    fn missing_fields_warn() {
        // A line chart with an axis only draws nothing: it needs a value (render tests).
        let w = warnings("lineChart", &[mea("Tooltips", "S")]);
        assert!(w[0].contains("needs a field in `Y` or `Y2`"), "{w:?}");
        let w = warnings("keyDriversVisual", &[col("ExplainBy", "A")]);
        assert!(w[0].contains("`Target` needs at least 1 field"), "{w:?}");
        assert!(check_roles(Some("tableEx"), &[]).is_empty());
        assert!(warnings("pivotTable", &[col("Rows", "A")]).is_empty());
        // The matrix has no tooltips well (wells-A): an unknown role.
        let e = errors("pivotTable", &[mea("Tooltips", "A")]);
        assert!(e[0].contains("no data role `Tooltips`"), "{e:?}");
    }

    #[test]
    fn case_spelling_and_unchecked_types() {
        let w = warnings("lineChart", &[col("category", "A"), mea("Y", "S")]);
        assert!(w[0].contains("spelled `Category`"), "{w:?}");
        let mut f = [col("category", "A")];
        normalize_roles(Some("lineChart"), &mut f);
        assert_eq!(f[0].role, "Category");
        assert!(check_roles(Some("myCustomVisual"), &[col("Anything", "A")]).is_empty());
        assert!(errors("qnaVisual", &[col("Anything", "A")]).is_empty());
        assert!(check_roles(None, &[col("Anything", "A")]).is_empty());
        // A textbox with a field draws nothing and shows no error (kinds-t04): not checked. A
        // scorecard with a field shows an error (kinds-t03).
        assert!(check_roles(Some("textbox"), &[col("Values", "A")]).is_empty());
        let e = errors("scorecard", &[col("Values", "A")]);
        assert!(e[0].contains("takes no data fields"), "{e:?}");
    }

    #[test]
    fn hostile_inputs_never_panic() {
        let long = "x".repeat(10_000);
        let f: Vec<FieldBinding> = (0..2000)
            .map(|i| col(if i % 2 == 0 { &long } else { "Y" }, "c"))
            .collect();
        for t in bicraft_visuals::visual_types() {
            let _ = check_roles(Some(t), &f);
        }
    }
}

#[cfg(test)]
#[path = "roles_kind_tests.rs"]
mod kind_tests;

/// Real reports, local only (skipped without `BICRAFT_TESTDATA`): no data-role or report-measure
/// error on any visual Desktop wrote. Only visual types, role names and counts are printed, never
/// field names (client material).
#[cfg(test)]
mod real_tests {
    use serde_json::json;

    use super::check_bindings;
    use crate::Session;

    #[test]
    fn real_visuals_pass_the_role_checks() {
        let Some(root) = bicraft_env::var_os("BICRAFT_TESTDATA").map(std::path::PathBuf::from) else {
            eprintln!("BICRAFT_TESTDATA unset: skipping");
            return;
        };
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&root)
            .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default();
        files.push(root.join("sample1").join("report.pbix"));
        files.retain(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("pbix") || e.eq_ignore_ascii_case("pbit"))
        });
        files.sort();
        let (mut visuals, mut warnings) = (0usize, 0usize);
        let mut errors: Vec<String> = Vec::new();
        for path in files {
            let mut s = Session::new();
            if s.execute("file.open", &json!({"path": path})).is_err() || s.report.is_none() {
                continue;
            }
            let r = s.report.as_ref().unwrap();
            for p in &r.pages {
                for v in &p.visuals {
                    visuals += 1;
                    for i in check_bindings(s.model.as_ref(), v.visual_type.as_deref(), &v.fields) {
                        if i.error {
                            errors.push(format!(
                                "{} role {}",
                                v.visual_type.as_deref().unwrap_or("?"),
                                i.role.as_deref().unwrap_or("-")
                            ));
                        } else {
                            warnings += 1;
                            eprintln!(
                                "warning: {} role {}",
                                v.visual_type.as_deref().unwrap_or("?"),
                                i.role.as_deref().unwrap_or("-")
                            );
                        }
                    }
                }
            }
            let out = s.execute("report.validate", &json!({})).unwrap();
            let measure_errors = out["issues"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|i| i["severity"] == "error" && (i.get("report_measure").is_some()))
                .count();
            assert_eq!(measure_errors, 0, "report-measure errors in a real report");
        }
        eprintln!("real visuals: {visuals} checked, {warnings} role warnings");
        assert!(errors.is_empty(), "role errors on real visuals: {errors:#?}");
    }
}

/// The role checks through the commands: `visual.add`, `visual.bind`, `report.apply`,
/// `report.validate`.
#[cfg(test)]
mod command_tests {
    use serde_json::{Value, json};

    use crate::Session;

    fn session() -> Session {
        let mut s = Session::new();
        for (id, p) in [
            ("model.new", json!({"name": "Shop"})),
            ("table.add", json!({"name": "Sales"})),
            (
                "column.add",
                json!({"table": "Sales", "name": "Region", "data_type": "string"}),
            ),
            (
                "column.add",
                json!({"table": "Sales", "name": "Date", "data_type": "dateTime"}),
            ),
            (
                "column.add",
                json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
            ),
            (
                "measure.add",
                json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
            ),
            (
                "measure.add",
                json!({"table": "Sales", "name": "Cost", "expression": "1"}),
            ),
            ("report.new", json!({})),
        ] {
            s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
        }
        s
    }

    fn add(s: &mut Session, t: &str, fields: Value) -> Result<Value, String> {
        s.execute(
            "visual.add",
            &json!({"page": "Page 1", "visual_type": t, "x": 0, "y": 0, "width": 300, "height": 200, "fields": fields}),
        )
    }

    #[test]
    fn visual_add_checks_roles() {
        let mut s = session();
        let e = add(
            &mut s,
            "lineChart",
            json!([{"role": "Legend", "field": "Sales[Region]"}]),
        )
        .unwrap_err();
        assert!(e.contains("did you mean `Series`"), "{e}");
        let e = add(
            &mut s,
            "clusteredColumnChart",
            json!([{"role": "Category", "field": "[Total]"}, {"role": "Y", "field": "[Cost]"}]),
        )
        .unwrap_err();
        assert!(e.contains("[Total] is a measure, but role `Category`"), "{e}");
        let e = add(
            &mut s,
            "lineChart",
            json!([{"role": "Category", "field": "Sales[Date]"}, {"role": "Series", "field": "Sales[Region]"},
                   {"role": "Y", "field": "[Total]"}, {"role": "Y", "field": "[Cost]"}]),
        )
        .unwrap_err();
        assert!(
            e.contains("`Y` takes at most 1 field when `Series` has a field"),
            "{e}"
        );
        assert!(s.report.as_ref().unwrap().pages[0].visuals.is_empty());
        // Role names are spelled as Desktop does; missing roles warn.
        let out = add(
            &mut s,
            "lineChart",
            json!([{"role": "tooltips", "field": "[Total]"}]),
        )
        .unwrap();
        assert_eq!(out["fields"][0]["role"], "Tooltips", "{out}");
        assert!(out["warnings"].to_string().contains("needs a field"), "{out}");
        // force keeps errors as warnings.
        let out = s
            .execute(
                "visual.add",
                &json!({"page": "Page 1", "visual_type": "card", "x": 0, "y": 300, "width": 100, "height": 100,
                        "fields": [{"role": "Values", "field": "[Total]"}, {"role": "Values", "field": "[Cost]"}],
                        "force": true}),
            )
            .unwrap();
        assert!(out["warnings"].to_string().contains("at most 1 field"), "{out}");
        // Custom visuals are not checked.
        add(
            &mut s,
            "myCustomVisual0123",
            json!([{"role": "Anything", "field": "[Total]"}]),
        )
        .unwrap();
    }

    #[test]
    fn visual_bind_checks_what_it_adds() {
        let mut s = session();
        let v = add(
            &mut s,
            "pieChart",
            json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]),
        )
        .unwrap()["visual"]
            .as_str()
            .unwrap()
            .to_string();
        let bind = |s: &mut Session, role: &str, field: &str| {
            s.execute(
                "visual.bind",
                &json!({"page": "Page 1", "visual": v, "role": role, "field": field}),
            )
        };
        bind(&mut s, "Series", "Sales[Date]").unwrap();
        let e = bind(&mut s, "Y", "[Cost]").unwrap_err();
        assert!(
            e.contains("`Y` takes at most 1 field when `Series` has a field"),
            "{e}"
        );
        let e = bind(&mut s, "Series", "Sales[Amount]").unwrap_err();
        assert!(e.contains("takes at most 1 field; it has 2"), "{e}");
        let e = bind(&mut s, "Values", "[Cost]").unwrap_err();
        assert!(e.contains("no data role `Values`"), "{e}");
        // A drill role takes a hierarchy.
        bind(&mut s, "Category", "Sales[Amount]").unwrap();
        let out = bind(&mut s, "tooltips", "[Cost]").unwrap();
        assert_eq!(out["role"], "Tooltips");
    }

    #[test]
    fn report_apply_and_validate_check_roles() {
        let mut s = session();
        let spec = json!({"pages": [{"key": "p", "visuals": [{"key": "v", "type": "card",
            "x": 0, "y": 0, "width": 100, "height": 100,
            "fields": [{"role": "Values", "field": "[Total]"}, {"role": "Values", "field": "[Cost]"}]}]}]});
        let e = s.execute("report.apply", &json!({"spec": spec})).unwrap_err();
        assert!(e.contains("at most 1 field"), "{e}");
        let out = s
            .execute("report.apply", &json!({"spec": spec, "force": true}))
            .unwrap();
        assert!(out.to_string().contains("at most 1 field"), "{out}");
        let v = s.execute("report.validate", &json!({})).unwrap();
        let roles: Vec<&Value> = v["issues"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["role"] == "Values")
            .collect();
        assert_eq!(roles.len(), 1, "{v}");
        assert_eq!(roles[0]["severity"], "error");
        assert_eq!(v["valid"], false);
    }
}
