//! The report rules of `model.analyze`: references the report makes to the model that no longer
//! resolve (after a rename or a removal), and report-internal names that went stale.
//!
//! Every reference is found by the walk `model.usage` uses ([`crate::usage_report::report_refs`]),
//! so the places covered are the same: visual roles, sorts, conditional formatting and other
//! formatting from fields, visual, page and report filters, drillthrough and tooltip page
//! bindings, bookmarks, and anything else the report keeps verbatim. Each finding names where it
//! is (page, visual, role, filter level, bookmark), the missing object, a near name the model has
//! (`suggestion`), and, where one command repairs it without touching anything else, that
//! command (`fix`): `report.rebind` to the suggestion, else a command that removes the reference
//! (`visual.unbind`, `*.clear_filters`, `report.remove_reference`), `visual.fix_query_refs` and
//! `parameter.fix_field` (`report_fix.rs`).
//!
//! Besides missing objects: projections whose `queryRef` names another field than the one they
//! project, formatting selectors (`selector.metadata`) that name no projection of their visual,
//! visual interactions naming a visual that is not on the page, and field parameters whose
//! `NAMEOF` names an object the model lacks.

use bicraft_dax::ast::Expr;
use bicraft_model::SemanticModel;
use bicraft_report::{FieldRef, Report, Visual};
use serde_json::{Map, Value, json};

use crate::report_fix;
use crate::usage_report::{ReportRef, report_refs};

/// The category of every report rule.
pub(crate) const CATEGORY: &str = "Report references";
/// The `source` of the report rules in `model.analyze`'s rule list.
pub(crate) const SOURCE: &str = "bicraft report rules";

/// One report rule: id, name, severity level (1 info, 2 warning, 3 error), what it checks.
pub(crate) struct ReportRule {
    pub id: &'static str,
    pub name: &'static str,
    pub severity: u8,
}

pub(crate) const VISUAL_FIELD: &str = "REPORT_BROKEN_VISUAL_FIELD";
pub(crate) const FILTER: &str = "REPORT_BROKEN_FILTER";
pub(crate) const DRILLTHROUGH: &str = "REPORT_BROKEN_DRILLTHROUGH";
pub(crate) const FORMATTING: &str = "REPORT_BROKEN_FORMATTING";
pub(crate) const BOOKMARK: &str = "REPORT_BROKEN_BOOKMARK";
pub(crate) const PROPERTY: &str = "REPORT_BROKEN_PROPERTY";
pub(crate) const FIELD_PARAMETER: &str = "REPORT_BROKEN_FIELD_PARAMETER";
pub(crate) const STALE_QUERY_REF: &str = "REPORT_STALE_QUERYREF";
pub(crate) const STALE_SELECTOR: &str = "REPORT_STALE_SELECTOR";
pub(crate) const DANGLING_INTERACTION: &str = "REPORT_DANGLING_INTERACTION";

/// The report rules, in the order findings are listed.
pub(crate) const RULES: &[ReportRule] = &[
    ReportRule {
        id: VISUAL_FIELD,
        name: "A visual's field or sort is missing from the model",
        severity: 3,
    },
    ReportRule {
        id: FILTER,
        name: "A visual, page or report filter is on a field missing from the model",
        severity: 3,
    },
    ReportRule {
        id: DRILLTHROUGH,
        name: "A drillthrough or tooltip page is bound to a field missing from the model",
        severity: 3,
    },
    ReportRule {
        id: FORMATTING,
        name: "Conditional formatting uses a field missing from the model",
        severity: 3,
    },
    ReportRule {
        id: FIELD_PARAMETER,
        name: "A field parameter lists a field missing from the model",
        severity: 3,
    },
    ReportRule {
        id: BOOKMARK,
        name: "A bookmark refers to a field missing from the model",
        severity: 2,
    },
    ReportRule {
        id: PROPERTY,
        name: "A report, page or visual property refers to a field missing from the model",
        severity: 2,
    },
    ReportRule {
        id: STALE_QUERY_REF,
        name: "A visual names a field by a queryRef of another field",
        severity: 2,
    },
    ReportRule {
        id: STALE_SELECTOR,
        name: "Formatting is keyed to a queryRef no field of the visual has",
        severity: 2,
    },
    ReportRule {
        id: DANGLING_INTERACTION,
        name: "A visual interaction names a visual that is not on the page",
        severity: 2,
    },
];

fn rule(id: &str) -> Option<&'static ReportRule> {
    RULES.iter().find(|r| r.id == id)
}

/// The rule of a missing reference found at a usage of `kind` (`crate::usage_report`).
fn rule_of_kind(kind: &str) -> &'static str {
    match kind {
        "visual" | "sort" => VISUAL_FIELD,
        "visual filter" | "page filter" | "report filter" => FILTER,
        "drillthrough" | "tooltip page" => DRILLTHROUGH,
        "formatting" | "page formatting" | "report formatting" => FORMATTING,
        "bookmark" => BOOKMARK,
        _ => PROPERTY,
    }
}

/// Model names, for resolving and suggesting.
struct Names<'a> {
    m: &'a SemanticModel,
}

/// What a reference misses: its table, or its member on an existing table.
enum Missing {
    Table(String),
    Member {
        table: String,
        name: String,
    },
    Level {
        table: String,
        hierarchy: String,
        level: String,
    },
}

impl<'a> Names<'a> {
    /// `None` when `r` resolves (as `model.usage` resolves it), else what is missing.
    fn missing(&self, r: &ReportRef) -> Option<Missing> {
        match r {
            ReportRef::Column { table, name } | ReportRef::Measure { table, name } => {
                let measure_anywhere =
                    matches!(r, ReportRef::Measure { .. }) && self.m.measure(name).is_some();
                match self.m.table(table) {
                    None if measure_anywhere => None,
                    None => Some(Missing::Table(table.clone())),
                    Some(t) => {
                        let found = t.column(name).is_some() || t.measure(name).is_some() || measure_anywhere;
                        (!found).then(|| Missing::Member {
                            table: t.name.clone(),
                            name: name.clone(),
                        })
                    }
                }
            }
            ReportRef::Level {
                table,
                hierarchy,
                level,
            } => {
                let Some(t) = self.m.table(table) else {
                    return Some(Missing::Table(table.clone()));
                };
                let found = t
                    .extra
                    .get("hierarchies")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|h| {
                        h.get("name")
                            .and_then(Value::as_str)
                            .is_some_and(|n| n.eq_ignore_ascii_case(hierarchy))
                    })
                    .flat_map(|h| h.get("levels").and_then(Value::as_array).into_iter().flatten())
                    .any(|l| {
                        l.get("name")
                            .and_then(Value::as_str)
                            .is_some_and(|n| n.eq_ignore_ascii_case(level))
                    });
                (!found).then(|| Missing::Level {
                    table: t.name.clone(),
                    hierarchy: hierarchy.clone(),
                    level: level.clone(),
                })
            }
        }
    }

    /// A model object near `missing`: a close table name, a close member of the same table, or
    /// a member of that name on another table (DAX syntax).
    fn suggest(&self, missing: &Missing) -> Option<String> {
        match missing {
            Missing::Table(t) => near(t, self.m.tables.iter().map(|x| x.name.as_str())).map(quote_table),
            Missing::Member { table, name } => {
                let t = self.m.table(table)?;
                let members = t
                    .columns
                    .iter()
                    .filter(|c| !c.is_row_number())
                    .map(|c| c.name.as_str())
                    .chain(t.measures.iter().map(|x| x.name.as_str()));
                if let Some(n) = near(name, members) {
                    return Some(qualified(&t.name, n));
                }
                self.m.tables.iter().find_map(|o| {
                    o.column(name)
                        .map(|c| qualified(&o.name, &c.name))
                        .or_else(|| o.measure(name).map(|x| qualified(&o.name, &x.name)))
                })
            }
            Missing::Level { .. } => None,
        }
    }

    /// The field `report.rebind` re-points `r` at: the suggestion (for a missing table, the
    /// member of that name on the suggested table) when it is a column for a column reference,
    /// or a measure for a measure reference.
    fn rebind_target(&self, r: &ReportRef, missing: &Missing, suggestion: Option<&str>) -> Option<String> {
        let (measure, name) = match r {
            ReportRef::Column { name, .. } => (false, name),
            ReportRef::Measure { name, .. } => (true, name),
            ReportRef::Level { .. } => return None,
        };
        let text = match missing {
            Missing::Table(_) => qualified(
                &self
                    .m
                    .tables
                    .iter()
                    .find(|t| suggestion == Some(quote_table(&t.name).as_str()))?
                    .name,
                name,
            ),
            Missing::Member { .. } => suggestion?.to_string(),
            Missing::Level { .. } => return None,
        };
        let field = crate::refs::resolve_field(Some(self.m), &text).ok()?.field;
        match (&field, measure) {
            (FieldRef::Column { .. }, false) | (FieldRef::Measure { .. }, true) => Some(text),
            _ => None,
        }
    }
}

/// The candidate nearest `word`: by edit distance ([`bicraft_visuals::suggest`]), else one that
/// starts or ends with it, or that it starts or ends with (`Category` → `Category Name`).
fn near<'b>(word: &str, candidates: impl Iterator<Item = &'b str> + Clone) -> Option<&'b str> {
    if let Some(c) = bicraft_visuals::suggest(word, candidates.clone()) {
        return Some(c);
    }
    let w = word.to_lowercase();
    if w.chars().count() < 3 {
        return None;
    }
    let mut hits = candidates.filter(|c| {
        let c = c.to_lowercase();
        c.starts_with(&w) || c.ends_with(&w) || w.starts_with(&c) || w.ends_with(&c)
    });
    let first = hits.next()?;
    // an ambiguous affix is no suggestion
    hits.next().is_none().then_some(first)
}

fn quote_table(t: &str) -> String {
    format!("'{}'", t.replace('\'', "''"))
}

fn qualified(t: &str, n: &str) -> String {
    format!("{}[{}]", quote_table(t), n.replace(']', "]]"))
}

fn ref_text(r: &ReportRef) -> String {
    match r {
        ReportRef::Column { table, name } | ReportRef::Measure { table, name } => qualified(table, name),
        ReportRef::Level {
            table,
            hierarchy,
            level,
        } => format!("{}[{}].[{}]", quote_table(table), hierarchy, level),
    }
}

fn kind_text(r: &ReportRef) -> &'static str {
    match r {
        ReportRef::Column { .. } => "column",
        ReportRef::Measure { .. } => "measure",
        ReportRef::Level { .. } => "hierarchy level",
    }
}

/// The page with display name (or name) `display`: its name.
fn page_name<'r>(r: &'r Report, display: &str) -> Option<&'r str> {
    r.pages
        .iter()
        .find(|p| p.display_name == display)
        .or_else(|| r.pages.iter().find(|p| p.name == display))
        .map(|p| p.name.as_str())
}

/// Where a usage is, for messages: `page "Overview", visual "v1" (role Values)`.
fn where_text(loc: &Map<String, Value>) -> String {
    let s = |k: &str| loc.get(k).and_then(Value::as_str);
    let mut parts = Vec::new();
    if let Some(b) = s("bookmark") {
        parts.push(format!("bookmark \"{b}\""));
    }
    if let Some(p) = s("page") {
        parts.push(format!("page \"{p}\""));
    }
    if let Some(v) = s("visual") {
        parts.push(format!("visual \"{v}\""));
    }
    let mut text = if parts.is_empty() {
        "the report".to_string()
    } else {
        parts.join(", ")
    };
    if let Some(role) = s("role") {
        text.push_str(&format!(" (role {role})"));
    }
    text
}

/// One finding, in the shape of `model.analyze`'s, plus `location`, `message`, `suggestion` and
/// `fix`.
#[allow(clippy::too_many_arguments)]
fn finding(
    rule_id: &str,
    object_type: &str,
    object: &str,
    table: Option<&str>,
    location: Map<String, Value>,
    message: String,
    suggestion: Option<String>,
    fix: Option<Value>,
) -> Option<Value> {
    let r = rule(rule_id)?;
    let mut o = Map::new();
    o.insert("rule_id".into(), json!(r.id));
    o.insert("rule_name".into(), json!(r.name));
    o.insert("category".into(), json!(CATEGORY));
    o.insert("severity".into(), json!(bicraft_bpa::severity_name(r.severity)));
    o.insert("severity_level".into(), json!(r.severity));
    o.insert("object_type".into(), json!(object_type));
    o.insert("object".into(), json!(object));
    if let Some(t) = table {
        o.insert("table".into(), json!(t));
    }
    o.insert("location".into(), Value::Object(location));
    o.insert("message".into(), json!(message));
    if let Some(s) = suggestion {
        o.insert("suggestion".into(), json!(s));
    }
    if let Some(f) = fix {
        o.insert("fix".into(), f);
    }
    Some(Value::Object(o))
}

/// A field as the filter commands take it (`'T'[C]`).
fn field_param(r: &ReportRef) -> Option<String> {
    match r {
        ReportRef::Column { table, name } | ReportRef::Measure { table, name } => {
            Some(qualified(table, name))
        }
        ReportRef::Level { .. } => None,
    }
}

/// Whether binding `f` projects reference `r`.
fn binds(f: &FieldRef, r: &ReportRef) -> bool {
    let (t, n) = match f {
        FieldRef::Column { table, column } | FieldRef::Aggregation { table, column, .. } => (table, column),
        FieldRef::Measure { table, measure } => (table, measure),
        FieldRef::Other { .. } => return false,
    };
    match r {
        ReportRef::Column { table, name } | ReportRef::Measure { table, name } => {
            t.eq_ignore_ascii_case(table) && n.eq_ignore_ascii_case(name)
        }
        ReportRef::Level { .. } => false,
    }
}

/// The command that removes a stale reference at `loc`, when one does so without touching
/// anything else: `visual.unbind` for a visual's field (by its position in the role), the
/// `*.clear_filters` commands for a filter card on the field, `report.remove_reference` for a
/// drillthrough or tooltip field, formatting and a bookmark's state (when it removes every
/// reference there, tried on a copy). Other properties have no such command.
fn fix_for(r: &Report, rr: &ReportRef, kind: &str, loc: &Map<String, Value>) -> Option<Value> {
    let s = |k: &str| loc.get(k).and_then(Value::as_str);
    let page = s("page_name");
    let field = || field_param(rr);
    let remove = |place: &str, at: Value| {
        let mut params = json!({"field": field()?, "place": place});
        if let (Some(o), Value::Object(at)) = (params.as_object_mut(), at) {
            o.extend(at);
        }
        Some(json!({"command": "report.remove_reference", "params": params}))
    };
    let find_page = |name: &str| r.pages.iter().find(|p| p.name == name);
    match kind {
        "drillthrough" | "tooltip page" => {
            let f = report_fix::FieldName::parse(&field()?).ok()?;
            let pg = find_page(page?)?;
            let (mut config, mut extra) = (pg.filter_config.clone(), pg.extra.clone());
            report_fix::strip_drill(&mut config, &mut extra, &f)
                .clean()
                .then(|| remove("drillthrough", json!({"page": pg.name})))?
        }
        "formatting" | "page formatting" | "report formatting" => {
            let f = report_fix::FieldName::parse(&field()?).ok()?;
            let (st, at) = match (page, s("visual")) {
                (Some(p), Some(vn)) => {
                    let v = find_page(p)?.visuals.iter().find(|v| v.name == vn)?;
                    let (mut a, mut b) = (v.visual_extra.clone(), v.extra.clone());
                    let mut st = report_fix::strip_formatting(&mut a, &f);
                    let st2 = report_fix::strip_formatting(&mut b, &f);
                    st.removed = st.removed.saturating_add(st2.removed);
                    st.remaining = st.remaining.saturating_add(st2.remaining);
                    (st, json!({"page": p, "visual": vn}))
                }
                (Some(p), None) => {
                    let mut extra = find_page(p)?.extra.clone();
                    (report_fix::strip_formatting(&mut extra, &f), json!({"page": p}))
                }
                (None, _) => {
                    let mut extra = r.extra.clone();
                    (report_fix::strip_formatting(&mut extra, &f), json!({}))
                }
            };
            st.clean().then(|| remove("formatting", at))?
        }
        "bookmark" => {
            let f = report_fix::FieldName::parse(&field()?).ok()?;
            let name = s("bookmark")?;
            let mut doc = r.bookmarks.iter().find(|b| b.name == name)?.document.clone();
            report_fix::strip_bookmark(&mut doc, &f)
                .clean()
                .then(|| remove("bookmark", json!({"bookmark": name})))?
        }
        "visual" => {
            let (page, visual, role) = (page?, s("visual")?, s("role")?);
            let v = r
                .pages
                .iter()
                .find(|p| p.name == page)?
                .visuals
                .iter()
                .find(|v| v.name == visual)?;
            let index = v
                .fields
                .iter()
                .filter(|b| b.role.eq_ignore_ascii_case(role))
                .position(|b| binds(&b.field, rr))?;
            Some(json!({"command": "visual.unbind",
                        "params": {"page": page, "visual": visual, "role": role, "index": index}}))
        }
        "visual filter" => Some(json!({"command": "visual.clear_filters",
            "params": {"page": page?, "visual": s("visual")?, "field": field_param(rr)?}})),
        "page filter" => Some(json!({"command": "page.clear_filters",
            "params": {"page": page?, "field": field_param(rr)?}})),
        "report filter" => Some(json!({"command": "report.clear_filters",
            "params": {"field": field_param(rr)?}})),
        _ => None,
    }
}

/// Missing model objects anywhere in the report.
fn missing_refs(m: &SemanticModel, r: &Report, out: &mut Vec<Value>) {
    let names = Names { m };
    let mut seen: Vec<(String, Value, String)> = Vec::new();
    report_refs("open", r, &mut |rr, usage| {
        let Some(missing) = names.missing(&rr) else {
            return;
        };
        let Value::Object(mut loc) = usage else {
            return;
        };
        loc.shift_remove("report");
        let kind = loc
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if let Some(display) = loc.get("page").and_then(Value::as_str)
            && let Some(name) = page_name(r, display)
        {
            loc.insert("page_name".into(), json!(name));
        }
        let rule_id = rule_of_kind(&kind);
        let object = ref_text(&rr);
        let key = (rule_id.to_string(), Value::Object(loc.clone()), object.clone());
        if seen.contains(&key) {
            return;
        }
        seen.push(key);
        let what = match &missing {
            Missing::Table(t) => format!("the model has no table {}", quote_table(t)),
            Missing::Member { table, name } => {
                format!("table {} has no column or measure [{name}]", quote_table(table))
            }
            Missing::Level {
                table,
                hierarchy,
                level,
            } => format!(
                "table {} has no hierarchy [{hierarchy}] with a level [{level}]",
                quote_table(table)
            ),
        };
        let suggestion = names.suggest(&missing);
        let mut message = format!(
            "{} {} ({kind}) refers to {object}: {what}",
            where_text(&loc),
            kind_text(&rr)
        );
        if let Some(s) = &suggestion {
            message.push_str(&format!("; did you mean {s}?"));
        }
        // a near field of the same kind: re-bind to it; else remove the reference
        let fix = match names.rebind_target(&rr, &missing, suggestion.as_deref()) {
            Some(to) => Some(json!({"command": "report.rebind", "params": {"from": object, "to": to}})),
            None => fix_for(r, &rr, &kind, &loc),
        };
        let table = match &rr {
            ReportRef::Column { table, .. }
            | ReportRef::Measure { table, .. }
            | ReportRef::Level { table, .. } => table,
        };
        let object_type = if loc.contains_key("bookmark") {
            "Bookmark"
        } else if loc.contains_key("visual") {
            "Visual"
        } else if loc.contains_key("page") {
            "Page"
        } else {
            "Report"
        };
        out.extend(finding(
            rule_id,
            object_type,
            &object,
            Some(table),
            loc,
            message,
            suggestion,
            fix,
        ));
    });
}

/// A visual's location (page display name and name, visual, type).
fn visual_loc(page: &bicraft_report::Page, v: &Visual) -> Map<String, Value> {
    let mut loc = Map::new();
    let display = if page.display_name.is_empty() {
        &page.name
    } else {
        &page.display_name
    };
    loc.insert("page".into(), json!(display));
    loc.insert("page_name".into(), json!(page.name));
    loc.insert("visual".into(), json!(v.name));
    if let Some(t) = &v.visual_type {
        loc.insert("visual_type".into(), json!(t));
    }
    loc
}

/// Projections whose `queryRef` names another field, and formatting selectors keyed to a
/// `queryRef` no projection has.
fn stale_names(r: &Report, out: &mut Vec<Value>) {
    for page in &r.pages {
        for v in &page.visuals {
            let refs: Vec<String> = v
                .visual_extra
                .get("query")
                .and_then(|q| q.get("queryState"))
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|states| states.values())
                .flat_map(|state| {
                    state
                        .get("projections")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                })
                .filter_map(|p| p.get("queryRef").and_then(Value::as_str).map(str::to_string))
                .collect();
            for stale in report_fix::stale_refs(v) {
                let (qr, expected) = (&stale.query_ref, &stale.expected);
                let mut loc = visual_loc(page, v);
                loc.insert("kind".into(), json!("visual"));
                loc.insert("role".into(), json!(stale.role));
                loc.insert("query_ref".into(), json!(qr));
                let mut message = format!(
                    "{} projects {} under the queryRef `{qr}`, which names another field (expected \
                     `{expected}`): formatting and sorting keyed to it can apply to the wrong field",
                    where_text(&loc),
                    crate::refs::field_text(&stale.field)
                );
                if !stale.fixable {
                    message.push_str(&format!(
                        "; `{expected}` is already used in the visual, so it is not renamed automatically"
                    ));
                }
                let fix = stale.fixable.then(|| {
                    json!({"command": "visual.fix_query_refs",
                           "params": {"page": page.name, "visual": v.name}})
                });
                out.extend(finding(
                    STALE_QUERY_REF,
                    "Visual",
                    &crate::refs::field_text(&stale.field),
                    None,
                    loc,
                    message,
                    Some(expected.clone()),
                    fix,
                ));
            }
            if refs.is_empty() {
                continue;
            }
            let Some(objects) = v.visual_extra.get("objects").and_then(Value::as_object) else {
                continue;
            };
            for (object, entries) in objects {
                for e in entries.as_array().into_iter().flatten() {
                    let Some(meta) = e.pointer("/selector/metadata").and_then(Value::as_str) else {
                        continue;
                    };
                    if refs.iter().any(|r| r == meta) {
                        continue;
                    }
                    let mut loc = visual_loc(page, v);
                    loc.insert("kind".into(), json!("formatting"));
                    loc.insert("object".into(), json!(object));
                    loc.insert("query_ref".into(), json!(meta));
                    let suggestion = near(meta, refs.iter().map(String::as_str)).map(str::to_string);
                    let message = format!(
                        "{} formats `{object}` for the queryRef `{meta}`, which no field of the visual has: \
                         the formatting is not applied",
                        where_text(&loc)
                    );
                    out.extend(finding(
                        STALE_SELECTOR,
                        "Visual",
                        meta,
                        None,
                        loc,
                        message,
                        suggestion,
                        None,
                    ));
                }
            }
        }
    }
}

/// `visualInteractions` entries naming a visual not on their page.
fn dangling_interactions(r: &Report, out: &mut Vec<Value>) {
    for page in &r.pages {
        let Some(list) = page.extra.get("visualInteractions").and_then(Value::as_array) else {
            continue;
        };
        let display = if page.display_name.is_empty() {
            &page.name
        } else {
            &page.display_name
        };
        for (i, e) in list.iter().enumerate() {
            for end in ["source", "target"] {
                let Some(name) = e.get(end).and_then(Value::as_str) else {
                    continue;
                };
                if page.visuals.iter().any(|v| v.name == name) {
                    continue;
                }
                let mut loc = Map::new();
                loc.insert("kind".into(), json!("page property"));
                loc.insert("page".into(), json!(display));
                loc.insert("page_name".into(), json!(page.name));
                loc.insert("interaction".into(), json!(i));
                loc.insert("end".into(), json!(end));
                let message = format!(
                    "page \"{display}\": visual interaction {i} names the {end} visual `{name}`, which is \
                     not on the page"
                );
                out.extend(finding(
                    DANGLING_INTERACTION,
                    "Page",
                    name,
                    None,
                    loc,
                    message,
                    None,
                    None,
                ));
            }
        }
    }
}

/// `NAMEOF(...)` references of an expression tree (table constructors, tuples and calls only,
/// the shape Desktop writes for a field parameter).
fn nameof_refs<'e>(e: &'e Expr, depth: usize, out: &mut Vec<(&'e Option<String>, &'e String)>) {
    if depth > 64 {
        return;
    }
    match e {
        Expr::TableCtor { rows, .. } => rows.iter().for_each(|x| nameof_refs(x, depth + 1, out)),
        Expr::Tuple { items, .. } => items.iter().for_each(|x| nameof_refs(x, depth + 1, out)),
        Expr::Call { name, args, .. } if name.eq_ignore_ascii_case("NAMEOF") => {
            if let Some(Some(Expr::Column { table, name, .. })) = args.first() {
                out.push((table, name));
            }
        }
        _ => {}
    }
}

/// Field parameters listing a field the model lacks.
fn field_parameters(m: &SemanticModel, out: &mut Vec<Value>) {
    for t in &m.tables {
        if !t.columns.iter().any(crate::parameters::is_fields_column) {
            continue;
        }
        let Some(expr) = t
            .partitions
            .iter()
            .find(|p| p.source_type.eq_ignore_ascii_case("calculated"))
            .and_then(|p| p.expression.as_deref())
        else {
            continue;
        };
        let (Some(tree), _) = bicraft_dax::parse_expression(expr) else {
            continue;
        };
        let mut refs = Vec::new();
        nameof_refs(&tree, 0, &mut refs);
        let rows = match &tree {
            Expr::TableCtor { rows, .. } => rows.len(),
            _ => 0,
        };
        let names = Names { m };
        for (table, name) in refs {
            let rr = match table {
                Some(tb) => ReportRef::Column {
                    table: tb.clone(),
                    name: name.clone(),
                },
                None => ReportRef::Measure {
                    table: String::new(),
                    name: name.clone(),
                },
            };
            let missing = match (&rr, table) {
                (_, Some(_)) => names.missing(&rr),
                (_, None) => m.measure(name).is_none().then(|| Missing::Member {
                    table: String::new(),
                    name: name.clone(),
                }),
            };
            let Some(missing) = missing else {
                continue;
            };
            let object = match table {
                Some(tb) => qualified(tb, name),
                None => format!("[{name}]"),
            };
            let suggestion = match (&missing, table) {
                (_, None) => near(
                    name,
                    m.tables
                        .iter()
                        .flat_map(|x| x.measures.iter().map(|y| y.name.as_str())),
                )
                .map(|n| format!("[{n}]")),
                _ => names.suggest(&missing),
            };
            let mut loc = Map::new();
            loc.insert("kind".into(), json!("field parameter"));
            loc.insert("field_parameter".into(), json!(t.name));
            let mut message = format!(
                "field parameter {} lists {object}, which the model does not have",
                quote_table(&t.name)
            );
            if let Some(s) = &suggestion {
                message.push_str(&format!("; did you mean {s}?"));
            }
            // point the NAMEOF at the near field, else drop its row (not the only one)
            let to = suggestion.as_deref().filter(|s| {
                crate::refs::resolve_field(Some(m), s)
                    .is_ok_and(|x| !matches!(x.field, FieldRef::Aggregation { .. } | FieldRef::Other { .. }))
            });
            let fix = match to {
                Some(to) => Some(json!({"name": t.name, "field": object, "to": to})),
                None => (rows > 1).then(|| json!({"name": t.name, "field": object})),
            }
            .map(|params| json!({"command": "parameter.fix_field", "params": params}));
            out.extend(finding(
                FIELD_PARAMETER,
                "Field parameter",
                &object,
                Some(&t.name),
                loc,
                message,
                suggestion,
                fix,
            ));
        }
    }
}

/// Every report finding for `r` against `m`, rule order then document order.
pub(crate) fn findings(m: &SemanticModel, r: &Report) -> Vec<Value> {
    let mut all = Vec::new();
    missing_refs(m, r, &mut all);
    stale_names(r, &mut all);
    dangling_interactions(r, &mut all);
    field_parameters(m, &mut all);
    let order = |f: &Value| {
        let id = f.get("rule_id").and_then(Value::as_str).unwrap_or_default();
        RULES.iter().position(|r| r.id == id).unwrap_or(RULES.len())
    };
    all.sort_by_key(order);
    all
}
