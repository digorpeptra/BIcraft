//! Declarative report building (M2.4): `report.apply`, `report.check` and `report.extract`.
//!
//! A *spec* describes pages and their visuals. `report.apply` makes the open report match it: it
//! creates what is missing, updates what differs and leaves alone what already matches, so applying
//! the same spec twice changes nothing the second time. `report.check` plans the same changes
//! without making them, and `report.extract` turns the open report into a spec that applies as a
//! no-op.
//!
//! **Identity.** A spec item names its page or visual with a `key` (any string the author picks)
//! or with the `name` of an existing object. A key becomes the PBIR name through [`stable_name`]:
//! pages hash `key`, visuals hash `<page name>/<key>`. The same key always yields the same name
//! on every platform and in every session, which is what makes re-runs idempotent.
//!
//! **All or nothing.** The whole spec is parsed, resolved against the model, placed and planned
//! on a shared borrow of the report; the report is only touched once the plan holds no error.

use std::collections::{HashMap, HashSet};

use bicraft_model::SemanticModel;
use bicraft_report::{FieldBinding, FieldRef, Page, Position, Report, Visual};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::params::{allow, force as force_param, opt_bool, opt_str};
use crate::refs::{check_field, field_text, same_field};
use crate::report::{
    DISPLAY_OPTIONS, KNOWN_VISUAL_TYPES, MAX_FIELDS, MAX_SIZE, MAX_VISUAL_TYPE, check_coordinate, check_role,
    check_size, opt_num, page_index, parts, report_ref, resolve_for_binding,
};

/// Most pages one spec lists.
const MAX_SPEC_PAGES: usize = 2000;
/// Most visuals one spec page lists.
const MAX_SPEC_VISUALS: usize = 2000;
/// Most visuals one spec lists in all.
const MAX_SPEC_TOTAL_VISUALS: usize = 100_000;
/// Longest key accepted, in characters.
const MAX_KEY_CHARS: usize = 200;
/// Longest object name accepted, in characters.
const MAX_OBJECT_NAME: usize = 200;
/// Most changes, errors or warnings listed; counts are always exact.
const MAX_LISTED: usize = 2000;
/// Errors quoted in a failed `report.apply`'s message.
const MAX_ERRORS_IN_MESSAGE: usize = 10;
/// `visibility` of a hidden page.
const HIDDEN: &str = "HiddenInViewMode";
/// Default size of a new page, as `page.add`.
const DEFAULT_PAGE: (f64, f64) = (1280.0, 720.0);

// ---------------------------------------------------------------------------------------------
// Stable names
// ---------------------------------------------------------------------------------------------

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
/// Offset basis of the second FNV-1a stream (the 64-bit golden ratio, xor the standard basis).
const FNV_OFFSET_2: u64 = FNV_OFFSET ^ 0x9e37_79b9_7f4a_7c15;

fn fnv1a(basis: u64, bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(basis, |h, &b| (h ^ u64::from(b)).wrapping_mul(FNV_PRIME))
}

/// The SplitMix64 finalizer: spreads FNV's weak low bits over the whole word.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A 20-lowercase-hex-digit name derived from `input` (Desktop's shape for page and visual
/// names): two FNV-1a 64 hashes of its UTF-8 bytes with different offset bases, each passed
/// through the SplitMix64 finalizer; the name is the first hash (16 digits) followed by the top
/// 16 bits of the second (4 digits). Integer arithmetic only, so it is the same on every
/// platform.
pub(crate) fn stable_name(input: &str) -> String {
    let bytes = input.as_bytes();
    let a = mix(fnv1a(FNV_OFFSET, bytes));
    let b = mix(fnv1a(FNV_OFFSET_2, bytes));
    format!("{a:016x}{:04x}", b >> 48)
}

/// The name of the page with spec key `key`.
pub(crate) fn page_name_for_key(key: &str) -> String {
    stable_name(key)
}

/// The name of the visual with spec key `key` on the page named `page_name`.
pub(crate) fn visual_name_for_key(page_name: &str, key: &str) -> String {
    stable_name(&format!("{page_name}/{key}"))
}

/// A spec key: any non-empty string without control characters, up to [`MAX_KEY_CHARS`].
pub(crate) fn check_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("`key` must not be empty".into());
    }
    if key.chars().any(char::is_control) {
        return Err("`key` must not contain control characters".into());
    }
    if key.chars().count() > MAX_KEY_CHARS {
        return Err(format!("`key` is longer than {MAX_KEY_CHARS} characters"));
    }
    Ok(())
}

/// A name for a new page or visual: it becomes a folder name, so letters, digits, `_` and `-`.
fn check_new_object_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 100
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!(
            "`{name}` cannot name a new object: use 1 to 100 letters, digits, `_` or `-` (or a `key`)"
        ));
    }
    Ok(())
}

fn check_visual_type(t: &str) -> Result<(), String> {
    if t.is_empty() || t.len() > MAX_VISUAL_TYPE || !t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(format!(
            "`{t}` is not a visual type like card, clusteredColumnChart, tableEx or slicer"
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The spec
// ---------------------------------------------------------------------------------------------

/// How a spec item names its object.
#[derive(Clone, Copy)]
enum Ident<'a> {
    Key(&'a str),
    Name(&'a str),
}

impl Ident<'_> {
    fn key(&self) -> Option<&str> {
        match self {
            Ident::Key(k) => Some(k),
            Ident::Name(_) => None,
        }
    }
}

struct PageSpec<'a> {
    at: String,
    ident: Ident<'a>,
    display_name: Option<&'a str>,
    width: Option<f64>,
    height: Option<f64>,
    display_option: Option<&'static str>,
    hidden: Option<bool>,
    layout: Option<&'a Value>,
    /// `None`: the page's visuals are not managed by this spec (never pruned either).
    visuals: Option<Vec<VisualSpec<'a>>>,
}

struct VisualSpec<'a> {
    at: String,
    ident: Ident<'a>,
    visual_type: Option<&'a str>,
    group: bool,
    x: Option<f64>,
    y: Option<f64>,
    z: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    place: Option<&'a Value>,
    fields: Option<Vec<FieldSpec<'a>>>,
    props: Option<&'a Value>,
    hidden: Option<bool>,
}

impl VisualSpec<'_> {
    fn explicit_count(&self) -> usize {
        [self.x, self.y, self.width, self.height]
            .iter()
            .filter(|v| v.is_some())
            .count()
    }

    /// Placed by `layout::place_all`: it has a `place` request, or no position at all.
    fn needs_placement(&self) -> bool {
        !self.group && (self.place.is_some() || self.explicit_count() == 0)
    }
}

struct FieldSpec<'a> {
    role: &'a str,
    field: FieldText<'a>,
}

enum FieldText<'a> {
    /// `Table[Column]`, `[Measure]`, `Sum(Table[Column])`.
    Text(&'a str),
    /// A raw PBIR field (hierarchy levels and other fields the text syntax cannot write).
    Raw(&'a Value),
}

struct Spec<'a> {
    pages: Vec<PageSpec<'a>>,
    active_page: Option<&'a str>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Prune {
    None,
    /// Visuals of listed pages that the spec does not list.
    Visuals,
    /// Also pages the spec does not list.
    All,
}

fn object_at<'a>(v: &'a Value, at: &str, allowed: &[&str]) -> Result<&'a Value, String> {
    if !v.is_object() {
        return Err(format!("{at} must be an object"));
    }
    allow(v, allowed).map_err(|e| format!("{at}: {e}"))?;
    Ok(v)
}

fn s_str<'a>(v: &'a Value, key: &str, at: &str) -> Result<Option<&'a str>, String> {
    opt_str(v, key).map_err(|e| format!("{at}: {e}"))
}

fn s_bool(v: &Value, key: &str, at: &str) -> Result<Option<bool>, String> {
    opt_bool(v, key).map_err(|e| format!("{at}: {e}"))
}

fn s_num(
    v: &Value,
    key: &str,
    at: &str,
    check: fn(&str, f64) -> Result<f64, String>,
) -> Result<Option<f64>, String> {
    opt_num(v, key)
        .and_then(|n| n.map(|n| check(key, n)).transpose())
        .map_err(|e| format!("{at}: {e}"))
}

fn s_value<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get(key).filter(|x| !x.is_null())
}

fn ident<'a>(v: &'a Value, at: &str) -> Result<Ident<'a>, String> {
    match (s_str(v, "key", at)?, s_str(v, "name", at)?) {
        (Some(_), Some(_)) => Err(format!("{at}: give `key` or `name`, not both")),
        (None, None) => Err(format!(
            "{at}: needs a `key` (any string you choose) or the `name` of an existing object"
        )),
        (Some(k), None) => {
            check_key(k).map_err(|e| format!("{at}: {e}"))?;
            Ok(Ident::Key(k))
        }
        (None, Some(n)) => {
            if n.is_empty() || n.chars().count() > MAX_OBJECT_NAME || n.chars().any(char::is_control) {
                return Err(format!("{at}: `name` must be 1 to {MAX_OBJECT_NAME} characters"));
            }
            Ok(Ident::Name(n))
        }
    }
}

fn parse_spec(spec: &Value) -> Result<Spec<'_>, String> {
    let spec = object_at(spec, "spec", &["pages", "active_page"])?;
    let active_page = s_str(spec, "active_page", "spec")?;
    let pages_v = match s_value(spec, "pages") {
        None => &[][..],
        Some(Value::Array(a)) => a.as_slice(),
        Some(_) => return Err("spec.pages must be an array".into()),
    };
    if pages_v.len() > MAX_SPEC_PAGES {
        return Err(format!("a spec lists at most {MAX_SPEC_PAGES} pages"));
    }
    let mut total_visuals = 0usize;
    let mut pages = Vec::with_capacity(pages_v.len());
    for (i, p) in pages_v.iter().enumerate() {
        let at = format!("spec.pages[{i}]");
        let p = object_at(
            p,
            &at,
            &[
                "key",
                "name",
                "display_name",
                "width",
                "height",
                "display_option",
                "hidden",
                "layout",
                "visuals",
            ],
        )?;
        let ident = ident(p, &at)?;
        let display_name = s_str(p, "display_name", &at)?;
        if let Some(d) = display_name {
            crate::edit::check_name("page", d).map_err(|e| format!("{at}: {e}"))?;
        }
        let display_option = match s_str(p, "display_option", &at)? {
            None => None,
            Some(o) => Some(
                DISPLAY_OPTIONS
                    .iter()
                    .find(|d| d.eq_ignore_ascii_case(o))
                    .copied()
                    .ok_or_else(|| {
                        format!(
                            "{at}: unknown display_option `{o}` (expected {})",
                            DISPLAY_OPTIONS.join(", ")
                        )
                    })?,
            ),
        };
        let visuals = match s_value(p, "visuals") {
            None => None,
            Some(Value::Array(a)) => {
                if a.len() > MAX_SPEC_VISUALS {
                    return Err(format!("{at}: a page lists at most {MAX_SPEC_VISUALS} visuals"));
                }
                total_visuals = total_visuals.saturating_add(a.len());
                if total_visuals > MAX_SPEC_TOTAL_VISUALS {
                    return Err(format!("a spec lists at most {MAX_SPEC_TOTAL_VISUALS} visuals"));
                }
                Some(
                    a.iter()
                        .enumerate()
                        .map(|(j, v)| parse_visual(v, &format!("{at}.visuals[{j}]")))
                        .collect::<Result<Vec<_>, _>>()?,
                )
            }
            Some(_) => return Err(format!("{at}.visuals must be an array")),
        };
        pages.push(PageSpec {
            ident,
            display_name,
            width: s_num(p, "width", &at, check_size)?,
            height: s_num(p, "height", &at, check_size)?,
            display_option,
            hidden: s_bool(p, "hidden", &at)?,
            layout: s_value(p, "layout"),
            visuals,
            at,
        });
    }
    Ok(Spec { pages, active_page })
}

fn parse_visual<'a>(v: &'a Value, at: &str) -> Result<VisualSpec<'a>, String> {
    let v = object_at(
        v,
        at,
        &[
            "key", "name", "type", "group", "x", "y", "z", "width", "height", "place", "fields", "props",
            "hidden",
        ],
    )?;
    let group = s_bool(v, "group", at)?.unwrap_or(false);
    let visual_type = s_str(v, "type", at)?;
    if group && visual_type.is_some() {
        return Err(format!("{at}: a visual group (`group: true`) has no `type`"));
    }
    let fields = match s_value(v, "fields") {
        None => None,
        Some(f) => {
            if group {
                return Err(format!("{at}: a visual group has no fields"));
            }
            Some(parse_fields(f, at)?)
        }
    };
    let props = s_value(v, "props");
    if props.is_some_and(|p| !p.is_object()) {
        return Err(format!("{at}.props must be an object"));
    }
    let place = s_value(v, "place");
    if group && place.is_some() {
        return Err(format!(
            "{at}: visual groups are not placed; give x, y, width, height"
        ));
    }
    Ok(VisualSpec {
        ident: ident(v, at)?,
        visual_type,
        group,
        x: s_num(v, "x", at, check_coordinate)?,
        y: s_num(v, "y", at, check_coordinate)?,
        z: s_num(v, "z", at, check_coordinate)?,
        width: s_num(v, "width", at, check_size)?,
        height: s_num(v, "height", at, check_size)?,
        place,
        fields,
        props,
        hidden: s_bool(v, "hidden", at)?,
        at: at.to_string(),
    })
}

fn parse_fields<'a>(v: &'a Value, at: &str) -> Result<Vec<FieldSpec<'a>>, String> {
    let items = v
        .as_array()
        .ok_or_else(|| format!("{at}.fields must be an array of {{role, field}} objects"))?;
    if items.len() > MAX_FIELDS {
        return Err(format!("{at}: a visual binds at most {MAX_FIELDS} fields"));
    }
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let at = format!("{at}.fields[{i}]");
            let item = object_at(item, &at, &["role", "field", "raw"])?;
            let role = s_str(item, "role", &at)?.ok_or_else(|| format!("{at}: missing string `role`"))?;
            let field = match (s_str(item, "field", &at)?, item.get("raw")) {
                (Some(t), None) => FieldText::Text(t),
                (None, Some(raw)) => FieldText::Raw(raw),
                (Some(_), Some(_)) => return Err(format!("{at}: give `field` or `raw`, not both")),
                (None, None) => {
                    return Err(format!(
                        "{at}: missing `field` (\"Table[Column]\", \"[Measure]\" or \"Sum(Table[Column])\")"
                    ));
                }
            };
            Ok(FieldSpec { role, field })
        })
        .collect()
}

fn parse_prune(p: &Value) -> Result<Prune, String> {
    match p.get("prune") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => Ok(Prune::None),
        Some(Value::Bool(true)) => Ok(Prune::Visuals),
        Some(Value::String(s)) if s == "visuals" => Ok(Prune::Visuals),
        Some(Value::String(s)) if s == "all" => Ok(Prune::All),
        Some(_) => Err("parameter `prune` must be true, false, \"visuals\" or \"all\"".into()),
    }
}

// ---------------------------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------------------------

/// The final page properties of an existing page whose properties change.
struct PageAttrs {
    display_name: String,
    width: f64,
    height: f64,
    display_option: Option<String>,
    visibility: Option<String>,
}

enum PagePlan {
    Existing {
        index: usize,
        attrs: Option<PageAttrs>,
        /// Changed visuals: (index in the page, the visual as it will be).
        replace: Vec<(usize, Visual)>,
        /// New visuals, appended in spec order.
        add: Vec<Visual>,
        /// Indices of visuals to remove.
        remove: HashSet<usize>,
    },
    New(Page),
}

#[derive(Default)]
struct Plan {
    pages: Vec<PagePlan>,
    /// Final page order, as indices into the current pages followed by new pages in plan order.
    /// Pages left out are removed.
    order: Vec<usize>,
    active_page: Option<String>,
    change_count: usize,
    changes: Vec<Value>,
    error_count: usize,
    errors: Vec<Value>,
    warning_count: usize,
    warnings: Vec<Value>,
    /// Key → name map of every spec item, for the result.
    ids: Vec<Value>,
}

impl Plan {
    fn change(&mut self, c: Value) {
        self.change_count = self.change_count.saturating_add(1);
        if self.changes.len() < MAX_LISTED {
            self.changes.push(c);
        }
    }

    fn error(&mut self, at: &str, message: impl Into<String>) {
        self.error_count = self.error_count.saturating_add(1);
        if self.errors.len() < MAX_LISTED {
            self.errors.push(json!({"at": at, "message": message.into()}));
        }
    }

    fn warning(&mut self, at: &str, message: impl Into<String>) {
        self.warning_count = self.warning_count.saturating_add(1);
        if self.warnings.len() < MAX_LISTED {
            self.warnings.push(json!({"at": at, "message": message.into()}));
        }
    }
}

/// Finds an object by exact name, then case-insensitively.
struct NameIndex {
    exact: HashMap<String, usize>,
    folded: HashMap<String, usize>,
}

impl NameIndex {
    fn new<'a>(names: impl Iterator<Item = &'a str>) -> Self {
        let mut exact = HashMap::new();
        let mut folded = HashMap::new();
        for (i, n) in names.enumerate() {
            exact.entry(n.to_string()).or_insert(i);
            folded.entry(n.to_lowercase()).or_insert(i);
        }
        Self { exact, folded }
    }

    fn find(&self, name: &str) -> Option<usize> {
        self.exact
            .get(name)
            .or_else(|| self.folded.get(&name.to_lowercase()))
            .copied()
    }
}

fn is_hidden(visibility: Option<&str>) -> bool {
    visibility == Some(HIDDEN)
}

fn same_display_option(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        (None, None) => true,
        _ => false,
    }
}

fn fits(width: f64, height: f64, pos: &Position) -> bool {
    pos.x >= 0.0 && pos.y >= 0.0 && pos.x + pos.width <= width && pos.y + pos.height <= height
}

struct Ctx<'a> {
    model: Option<&'a SemanticModel>,
    force: bool,
    prune: Prune,
    /// The report's own measures, which fields may name too.
    measures: Vec<crate::report_measures::ReportMeasure>,
}

fn make_plan(report: &Report, spec: &Spec<'_>, ctx: &Ctx<'_>) -> Plan {
    let mut plan = Plan::default();
    let pages_index = NameIndex::new(report.pages.iter().map(|p| p.name.as_str()));
    let n_pages = report.pages.len();
    // Which spec page claimed each existing page or new name (lowercase), for duplicates.
    let mut claimed: HashMap<String, String> = HashMap::new();
    // Each spec page's index in "current pages followed by new pages".
    let mut spec_order: Vec<usize> = Vec::new();
    let mut new_count = 0usize;
    // Final display names (lowercase) → (spec item, whether the name is new for that page).
    let mut display: Display = HashMap::new();
    // Spec key → final page name, for `active_page`.
    let mut key_names: HashMap<&str, String> = HashMap::new();

    for ps in &spec.pages {
        let at = ps.at.as_str();
        let name = match ps.ident {
            Ident::Key(k) => page_name_for_key(k),
            Ident::Name(n) => n.to_string(),
        };
        let existing = pages_index.find(&name);
        let name = existing
            .and_then(|i| report.pages.get(i))
            .map_or(name, |p| p.name.clone());
        if let Some(other) = claimed.insert(name.to_lowercase(), at.to_string()) {
            plan.error(at, format!("names the same page as {other}"));
            continue;
        }
        if let Some(k) = ps.ident.key() {
            key_names.insert(k, name.clone());
        }
        match existing.and_then(|i| report.pages.get(i).map(|p| (i, p))) {
            Some((i, page)) => {
                spec_order.push(i);
                plan_existing_page(&mut plan, ctx, ps, i, page, &mut display);
            }
            None => {
                if let Ident::Name(n) = ps.ident
                    && let Err(e) = check_new_object_name(n)
                {
                    plan.error(at, format!("no page is named `{n}`, and {e}"));
                    continue;
                }
                spec_order.push(n_pages.saturating_add(new_count));
                new_count = new_count.saturating_add(1);
                plan_new_page(&mut plan, ctx, ps, name, &mut display);
            }
        }
    }

    // Pages the spec does not list: kept (their display names still count), or pruned.
    let listed: HashSet<usize> = spec_order.iter().copied().collect();
    let mut removed = HashSet::new();
    for (i, page) in report.pages.iter().enumerate() {
        if listed.contains(&i) {
            continue;
        }
        if ctx.prune == Prune::All {
            removed.insert(i);
            let bookmarks: Vec<&str> = report
                .bookmarks
                .iter()
                .filter(|b| mentions(&b.document, &page.name, 0))
                .map(|b| b.name.as_str())
                .collect();
            for b in bookmarks {
                plan.warning(
                    &format!("page {}", page.name),
                    format!("bookmark `{b}` refers to the removed page"),
                );
            }
            plan.change(json!({"action": "remove", "kind": "page", "page": page.name,
                               "display_name": page.display_name, "visuals": page.visuals.len()}));
        } else if let Some((other, true)) = display.get(&page.display_name.to_lowercase()) {
            plan.error(
                other,
                format!(
                    "page `{}` ({}) already has this display name; page display names must be unique",
                    page.display_name, page.name
                ),
            );
        }
    }

    // Final order: listed pages take the slots listed pages occupy, in spec order; new pages go
    // last; unlisted pages keep their place.
    let after: Vec<usize> = (0..n_pages)
        .filter(|i| !removed.contains(i))
        .chain(n_pages..n_pages.saturating_add(new_count))
        .collect();
    let mut queue = spec_order.iter();
    let order: Vec<usize> = after
        .iter()
        .filter_map(|&i| {
            if listed.contains(&i) {
                queue.next().copied()
            } else {
                Some(i)
            }
        })
        .collect();
    if order.is_empty() && n_pages > 0 {
        plan.error(
            "spec",
            "the report would have no page left; Power BI needs at least one",
        );
    }
    let new_names: Vec<String> = plan
        .pages
        .iter()
        .filter_map(|p| match p {
            PagePlan::New(page) => Some(page.name.clone()),
            PagePlan::Existing { .. } => None,
        })
        .collect();
    let name_at = |i: usize| -> String {
        report.pages.get(i).map_or_else(
            || {
                new_names
                    .get(i.saturating_sub(n_pages))
                    .cloned()
                    .unwrap_or_default()
            },
            |p| p.name.clone(),
        )
    };
    if order != after {
        let names: Vec<String> = order.iter().map(|&i| name_at(i)).collect();
        plan.change(json!({"action": "reorder", "kind": "pages", "order": names}));
    }
    let final_names: Vec<String> = order.iter().map(|&i| name_at(i)).collect();
    plan.order = order;

    // The active page.
    let current = report.active_page.as_deref();
    let is_final = |a: &str| final_names.iter().any(|n| n.eq_ignore_ascii_case(a));
    let wanted = match spec.active_page {
        Some(a) => match key_names.get(a) {
            Some(name) => Some(name.clone()),
            None => match final_names.iter().find(|n| n.eq_ignore_ascii_case(a)) {
                Some(name) => Some(name.clone()),
                None => {
                    plan.error(
                        "spec.active_page",
                        format!("`{a}` is neither a page key of the spec nor a page name"),
                    );
                    None
                }
            },
        },
        // Keep the active page unless it is removed, or there was none and pages are created.
        None if current.is_some_and(|c| !is_final(c)) || (current.is_none() && new_count > 0) => {
            final_names.first().cloned()
        }
        None => None,
    };
    if let Some(w) = wanted
        && current.is_none_or(|c| !c.eq_ignore_ascii_case(&w) || !is_final(c))
    {
        plan.change(json!({"action": "set_active", "kind": "report", "page": w}));
        plan.active_page = Some(w);
    }
    plan
}

/// Final page display names (lowercase) → (spec item, whether the name is new for its page).
type Display = HashMap<String, (String, bool)>;

/// Records a page's final display name; two pages may only share one when neither changes it
/// (a report that already has duplicates is not made worse).
fn check_display_name(plan: &mut Plan, at: &str, name: &str, is_new: bool, display: &mut Display) {
    if let Some((other, other_new)) = display.insert(name.to_lowercase(), (at.to_string(), is_new))
        && (is_new || other_new)
    {
        plan.error(
            at,
            format!("display name `{name}` is also used by {other}; page display names must be unique"),
        );
    }
}

fn plan_existing_page(
    plan: &mut Plan,
    ctx: &Ctx<'_>,
    ps: &PageSpec<'_>,
    index: usize,
    page: &Page,
    display: &mut Display,
) {
    let at = ps.at.as_str();
    let mut attrs = PageAttrs {
        display_name: ps.display_name.unwrap_or(&page.display_name).to_string(),
        width: ps.width.unwrap_or(page.width),
        height: ps.height.unwrap_or(page.height),
        display_option: page.display_option.clone(),
        visibility: page.visibility.clone(),
    };
    let mut changed: Vec<&str> = Vec::new();
    if attrs.display_name != page.display_name {
        changed.push("display_name");
    }
    if attrs.width != page.width {
        changed.push("width");
    }
    if attrs.height != page.height {
        changed.push("height");
    }
    if let Some(o) = ps.display_option
        && !same_display_option(Some(o), page.display_option.as_deref())
    {
        attrs.display_option = Some(o.to_string());
        changed.push("display_option");
    }
    if let Some(h) = ps.hidden
        && h != is_hidden(page.visibility.as_deref())
    {
        attrs.visibility = h.then(|| HIDDEN.to_string());
        changed.push("hidden");
    }
    check_display_name(
        plan,
        at,
        &attrs.display_name,
        attrs.display_name != page.display_name,
        display,
    );
    if !changed.is_empty() {
        plan.change(json!({"action": "update", "kind": "page", "page": page.name,
                           "key": ps.ident.key(), "properties": changed}));
    }
    let (w, h) = (attrs.width, attrs.height);
    let visuals = match &ps.visuals {
        Some(v) => plan_visuals(plan, ctx, ps, &page.name, Some(page), (w, h), v),
        None => VisualsPlan::default(),
    };
    plan.ids
        .push(json!({"key": ps.ident.key(), "page": page.name, "visuals": visuals.ids}));
    plan.pages.push(PagePlan::Existing {
        index,
        attrs: (!changed.is_empty()).then_some(attrs),
        replace: visuals.replace,
        add: visuals.add,
        remove: visuals.remove,
    });
}

fn plan_new_page(plan: &mut Plan, ctx: &Ctx<'_>, ps: &PageSpec<'_>, name: String, display: &mut Display) {
    let at = ps.at.as_str();
    let display_name = match (ps.display_name, ps.ident) {
        (Some(d), _) => d.to_string(),
        (None, Ident::Key(k)) => k.trim().to_string(),
        (None, Ident::Name(n)) => n.to_string(),
    };
    if let Err(e) = crate::edit::check_name("page", &display_name) {
        plan.error(at, format!("{e} (give a `display_name`)"));
    }
    check_display_name(plan, at, &display_name, true, display);
    let (w, h) = (
        ps.width.unwrap_or(DEFAULT_PAGE.0),
        ps.height.unwrap_or(DEFAULT_PAGE.1),
    );
    let visuals = match &ps.visuals {
        Some(v) => plan_visuals(plan, ctx, ps, &name, None, (w, h), v),
        None => VisualsPlan::default(),
    };
    plan.change(
        json!({"action": "create", "kind": "page", "page": name, "key": ps.ident.key(),
                       "display_name": display_name}),
    );
    plan.ids
        .push(json!({"key": ps.ident.key(), "page": name, "visuals": visuals.ids}));
    plan.pages.push(PagePlan::New(Page {
        name,
        display_name,
        display_option: Some(ps.display_option.unwrap_or("FitToPage").to_string()),
        width: w,
        height: h,
        visibility: ps.hidden.unwrap_or(false).then(|| HIDDEN.to_string()),
        visuals: visuals.add,
        ..Default::default()
    }));
}

#[derive(Default)]
struct VisualsPlan {
    replace: Vec<(usize, Visual)>,
    add: Vec<Visual>,
    remove: HashSet<usize>,
    ids: Vec<Value>,
}

fn plan_visuals(
    plan: &mut Plan,
    ctx: &Ctx<'_>,
    ps: &PageSpec<'_>,
    page_name: &str,
    page: Option<&Page>,
    (page_w, page_h): (f64, f64),
    specs: &[VisualSpec<'_>],
) -> VisualsPlan {
    let mut out = VisualsPlan::default();
    let existing: &[Visual] = page.map_or(&[], |p| p.visuals.as_slice());
    let index = NameIndex::new(existing.iter().map(|v| v.name.as_str()));

    // Identities.
    let mut claimed: HashMap<String, &str> = HashMap::new();
    let mut targets: Vec<Option<(String, Option<usize>)>> = Vec::with_capacity(specs.len());
    for vs in specs {
        let name = match vs.ident {
            Ident::Key(k) => visual_name_for_key(page_name, k),
            Ident::Name(n) => n.to_string(),
        };
        let found = index.find(&name);
        let name = found
            .and_then(|i| existing.get(i))
            .map_or(name, |v| v.name.clone());
        if let Some(other) = claimed.insert(name.to_lowercase(), vs.at.as_str()) {
            plan.error(&vs.at, format!("names the same visual as {other}"));
            targets.push(None);
            continue;
        }
        if found.is_none()
            && let Ident::Name(n) = vs.ident
            && let Err(e) = check_new_object_name(n)
        {
            plan.error(&vs.at, format!("no visual on this page is named `{n}`, and {e}"));
            targets.push(None);
            continue;
        }
        targets.push(Some((name, found)));
    }

    // Placement of every visual without a full explicit position, in spec order.
    let placed_ids: Vec<usize> = specs
        .iter()
        .enumerate()
        .filter(|(_, vs)| vs.needs_placement())
        .map(|(i, _)| i)
        .collect();
    let mut placed: HashMap<usize, crate::layout::Rect> = HashMap::new();
    if !placed_ids.is_empty() {
        let items: Vec<Option<&Value>> = placed_ids
            .iter()
            .map(|&i| specs.get(i).and_then(|vs| vs.place))
            .collect();
        match crate::layout::place_all(page_w, page_h, ps.layout, &items) {
            Ok(rects) if rects.len() == items.len() => {
                placed = placed_ids.iter().copied().zip(rects).collect();
            }
            Ok(_) => plan.error(
                &ps.at,
                "internal: the layout returned the wrong number of rectangles",
            ),
            Err(e) => plan.error(&ps.at, format!("cannot place its visuals: {e}")),
        }
    }

    // New visuals stack above the existing ones, like visual.add.
    let mut top_z = existing
        .iter()
        .map(|v| v.position.z)
        .fold(None, |m: Option<f64>, z| Some(m.map_or(z, |m| m.max(z))));
    let mut top_tab = existing.iter().filter_map(|v| v.position.tab_order).max();

    for (i, (vs, target)) in specs.iter().zip(&targets).enumerate() {
        let Some((name, found)) = target else { continue };
        let base = found.and_then(|f| existing.get(f));
        out.ids.push(json!({"key": vs.ident.key(), "visual": name}));
        let rect = placed.get(&i).copied();
        let built = build_visual(
            plan,
            ctx,
            vs,
            name,
            base,
            rect,
            (page_w, page_h),
            &mut top_z,
            &mut top_tab,
        );
        let Some((visual, changed)) = built else { continue };
        match (found, base) {
            (Some(f), Some(_)) => {
                if !changed.is_empty() {
                    plan.change(json!({"action": "update", "kind": "visual", "page": page_name,
                                       "visual": name, "key": vs.ident.key(), "properties": changed}));
                    out.replace.push((*f, visual));
                }
            }
            _ => {
                plan.change(json!({"action": "create", "kind": "visual", "page": page_name,
                                   "visual": name, "key": vs.ident.key(), "type": visual.visual_type}));
                out.add.push(visual);
            }
        }
    }

    // Pruning: visuals the spec does not list, except groups holding a listed visual.
    if ctx.prune != Prune::None && !existing.is_empty() {
        let mut keep: HashSet<usize> = targets.iter().flatten().filter_map(|(_, f)| *f).collect();
        let mut stack: Vec<usize> = keep.iter().copied().collect();
        while let Some(i) = stack.pop() {
            if let Some(g) = existing.get(i).and_then(|v| v.parent_group.as_deref())
                && let Some(gi) = index.find(g)
                && keep.insert(gi)
            {
                stack.push(gi);
            }
        }
        for (i, v) in existing.iter().enumerate() {
            if !keep.contains(&i) {
                out.remove.insert(i);
                plan.change(json!({"action": "remove", "kind": "visual", "page": page_name,
                                   "visual": v.name, "type": v.visual_type}));
            }
        }
    }
    out
}

/// The visual as the spec wants it, and which properties differ from `base` (all of them for a
/// new visual). `None` when the item has errors (recorded in `plan`).
#[allow(clippy::too_many_arguments)]
fn build_visual(
    plan: &mut Plan,
    ctx: &Ctx<'_>,
    vs: &VisualSpec<'_>,
    name: &str,
    base: Option<&Visual>,
    rect: Option<crate::layout::Rect>,
    (page_w, page_h): (f64, f64),
    top_z: &mut Option<f64>,
    top_tab: &mut Option<i64>,
) -> Option<(Visual, Vec<&'static str>)> {
    let at = vs.at.as_str();
    let errors_before = plan.error_count;
    let mut v = match base {
        Some(b) => b.clone(),
        None => Visual {
            name: name.to_string(),
            ..Default::default()
        },
    };
    let is_group = base.is_some_and(|b| b.visual_type.is_none());
    if vs.group && base.is_none() {
        plan.error(
            at,
            "creating visual groups is not supported; list existing groups only",
        );
    } else if vs.group && !is_group {
        plan.error(at, format!("`{name}` is not a visual group"));
    } else if !vs.group && is_group {
        plan.error(
            at,
            format!("`{name}` is a visual group: mark it `group: true` (groups have no type)"),
        );
    }

    // Type: one of BIcraft's visuals by name or guid is its guid (its package goes in when the
    // plan is applied, as with visual.add)
    let builtin = vs.visual_type.and_then(crate::builtin_visuals::find);
    let visual_type = builtin.map(|b| b.guid).or(vs.visual_type);
    match (visual_type, base) {
        (None, None) if !vs.group => plan.error(at, "a new visual needs a `type`"),
        (Some(t), _) if v.visual_type.as_deref() != Some(t) => match check_visual_type(t) {
            Ok(()) => {
                if builtin.is_none() && !KNOWN_VISUAL_TYPES.contains(&t) {
                    plan.warning(
                        at,
                        format!("`{t}` is not a built-in visual type; it must be a custom visual installed in the report"),
                    );
                }
                v.visual_type = Some(t.to_string());
            }
            Err(e) => plan.error(at, e),
        },
        _ => {}
    }

    // Position.
    let explicit = vs.explicit_count();
    if base.is_none() && rect.is_none() && explicit < 4 && !vs.needs_placement() {
        plan.error(
            at,
            "a new visual needs all of x, y, width and height, or none of them (or a `place`) to be placed automatically",
        );
    }
    let mut pos = v.position;
    if let Some(r) = rect {
        let ok = check_coordinate("x", r.x).is_ok()
            && check_coordinate("y", r.y).is_ok()
            && check_size("width", r.width).is_ok()
            && check_size("height", r.height).is_ok();
        if ok {
            (pos.x, pos.y, pos.width, pos.height) = (r.x, r.y, r.width, r.height);
        } else {
            plan.error(at, format!("the layout placed it out of range ({r:?})"));
        }
    }
    pos.x = vs.x.unwrap_or(pos.x);
    pos.y = vs.y.unwrap_or(pos.y);
    pos.width = vs.width.unwrap_or(pos.width);
    pos.height = vs.height.unwrap_or(pos.height);
    if let Some(z) = vs.z {
        pos.z = z;
    }
    if base.is_none() {
        if vs.z.is_none() {
            pos.z = top_z.map_or(0.0, |m| (m + 1000.0).min(MAX_SIZE));
        }
        pos.tab_order = Some(top_tab.map_or(0, |t| t.saturating_add(1000)));
        *top_z = Some(top_z.map_or(pos.z, |m| m.max(pos.z)));
        *top_tab = pos.tab_order.max(*top_tab);
    }
    v.position = pos;

    if let Some(h) = vs.hidden {
        v.is_hidden = h;
    }

    // Fields.
    if let Some(fields) = &vs.fields
        && let Some(mut new) = plan_fields(plan, ctx, at, &v.fields, fields)
    {
        // Data roles (G14): checked on a changed field list only.
        crate::roles::normalize_roles(v.visual_type.as_deref(), &mut new);
        // BIcraft's visuals: their own roles, spelled as they declare them (visual.add's check)
        if let Some(b) = v.visual_type.as_deref().and_then(crate::builtin_visuals::find) {
            for f in &mut new {
                match b.role(&f.role) {
                    Some(r) => r.clone_into(&mut f.role),
                    None => {
                        let message = format!(
                            "{} has no role `{}` (its roles: {})",
                            b.display_name,
                            f.role,
                            b.role_names().join(", ")
                        );
                        if ctx.force {
                            plan.warning(&format!("{at}.fields ({})", f.role), message);
                        } else {
                            plan.error(
                                &format!("{at}.fields ({})", f.role),
                                format!("{message} (pass \"force\": true to bind anyway)"),
                            );
                        }
                    }
                }
            }
        }
        for i in crate::roles::check_bindings(ctx.model, v.visual_type.as_deref(), &new) {
            let fat = i
                .role
                .as_ref()
                .map_or_else(|| format!("{at}.fields"), |r| format!("{at}.fields ({r})"));
            if i.error && !ctx.force {
                plan.error(
                    &fat,
                    format!("{} (pass \"force\": true to bind anyway)", i.message),
                );
            } else {
                plan.warning(&fat, i.message);
            }
        }
        v.fields = new;
    }

    // Properties (title, sort, filters, formatting), applied by the visual_props module.
    let mut props_changed = false;
    if let Some(props) = vs.props
        && plan.error_count == errors_before
    {
        let before = v.clone();
        match crate::visual_props::apply_props(&mut v, props, ctx.model) {
            Ok(_) => props_changed = v != before,
            Err(e) => plan.error(at, format!("props: {e}")),
        }
    }
    // a new Spec visual (or one turned into it) without a spec starts with
    // visual.spec_template's for its fields
    if base.is_none_or(|b| b.visual_type != v.visual_type)
        && let Err(e) = crate::spec_visual::default_spec(&mut v)
    {
        plan.warning(at, format!("no starting spec: {e}"));
    }
    if plan.error_count != errors_before {
        return None;
    }

    if let Some(b) = base
        && b.position != v.position
        && !fits(page_w, page_h, &v.position)
        && v.parent_group.is_none()
    {
        plan.warning(
            at,
            format!("the visual does not fit on the page ({page_w} x {page_h})"),
        );
    } else if base.is_none() && !fits(page_w, page_h, &v.position) {
        plan.warning(
            at,
            format!("the visual does not fit on the page ({page_w} x {page_h})"),
        );
    }

    let changed: Vec<&'static str> = match base {
        None => vec!["created"],
        Some(b) => {
            let mut c = Vec::new();
            if b.visual_type != v.visual_type {
                c.push("type");
            }
            let (p, q) = (&b.position, &v.position);
            if (p.x, p.y, p.width, p.height) != (q.x, q.y, q.width, q.height) {
                c.push("position");
            }
            if p.z != q.z {
                c.push("z");
            }
            if b.is_hidden != v.is_hidden {
                c.push("hidden");
            }
            if b.fields != v.fields {
                c.push("fields");
            }
            // Props may restore what `fields` replaced (small multiples bind `Rows`): only a
            // visual that ends up different has changed.
            if props_changed && *b != v {
                c.push("props");
            }
            if c.is_empty() && *b != v {
                c.push("other");
            }
            c
        }
    };
    Some((v, changed))
}

/// The new field list, or `None` when `base` already matches the spec. Fields equal to the binding
/// at the same place (same role, same text) are kept as they are, without a model lookup, so a
/// spec extracted from a report re-applies even where the report binds fields the model lacks.
fn plan_fields(
    plan: &mut Plan,
    ctx: &Ctx<'_>,
    at: &str,
    base: &[FieldBinding],
    specs: &[FieldSpec<'_>],
) -> Option<Vec<FieldBinding>> {
    let mut warnings = Vec::new();
    let mut out: Vec<FieldBinding> = Vec::with_capacity(specs.len());
    let mut failed = false;
    for (i, fs) in specs.iter().enumerate() {
        let same_slot = base.get(i).filter(|b| b.role.eq_ignore_ascii_case(fs.role));
        let field = match fs.field {
            FieldText::Raw(raw) => FieldRef::Other { raw: raw.clone() },
            FieldText::Text(t) => match same_slot.filter(|b| field_text(&b.field) == t) {
                Some(b) => b.field.clone(),
                None => match resolve_for_binding(ctx.model, &ctx.measures, t, ctx.force, &mut warnings) {
                    Ok(f) => f,
                    Err(e) => {
                        plan.error(&format!("{at}.fields[{i}]"), e);
                        failed = true;
                        continue;
                    }
                },
            },
        };
        out.push(FieldBinding {
            display_name: None,
            show_as: None,
            role: same_slot.map_or_else(|| fs.role.to_string(), |b| b.role.clone()),
            field,
        });
    }
    if failed {
        return None;
    }
    let unchanged = out.len() == base.len()
        && out
            .iter()
            .zip(base)
            .all(|(a, b)| a.role.eq_ignore_ascii_case(&b.role) && same_field(&a.field, &b.field));
    if unchanged {
        return None;
    }
    // Only a changed list is checked: an unchanged one is the report's business (report.validate).
    for (i, b) in out.iter().enumerate() {
        let fat = format!("{at}.fields[{i}]");
        if let Err(e) = check_role(&b.role) {
            plan.error(&fat, e);
        }
        if out
            .iter()
            .take(i)
            .any(|o| o.role.eq_ignore_ascii_case(&b.role) && same_field(&o.field, &b.field))
        {
            plan.error(
                &fat,
                format!("{} is bound twice to role {}", field_text(&b.field), b.role),
            );
        }
        if let Some(m) = ctx.model
            && base.get(i).is_some_and(|x| x.field == b.field)
            && let Some(problem) = check_field(m, &b.field)
        {
            plan.warning(&fat, format!("kept as bound: {problem}"));
        }
    }
    for w in warnings {
        plan.warning(at, w);
    }
    Some(out)
}

/// Whether a JSON document mentions `text` as a string value anywhere (bounded depth).
fn mentions(v: &Value, text: &str, depth: usize) -> bool {
    if depth > 128 {
        return false;
    }
    match v {
        Value::String(s) => s == text,
        Value::Array(a) => a.iter().any(|x| mentions(x, text, depth.saturating_add(1))),
        Value::Object(o) => o.values().any(|x| mentions(x, text, depth.saturating_add(1))),
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------------------------

/// Applies a plan without errors. Nothing here can fail.
fn execute(report: &mut Report, plan: Plan) {
    let mut new_pages = Vec::new();
    for pp in plan.pages {
        match pp {
            PagePlan::New(page) => new_pages.push(page),
            PagePlan::Existing {
                index,
                attrs,
                replace,
                add,
                remove,
            } => {
                let Some(page) = report.pages.get_mut(index) else {
                    continue;
                };
                if let Some(a) = attrs {
                    page.display_name = a.display_name;
                    page.width = a.width;
                    page.height = a.height;
                    page.display_option = a.display_option;
                    page.visibility = a.visibility;
                }
                for (i, v) in replace {
                    if let Some(slot) = page.visuals.get_mut(i) {
                        *slot = v;
                    }
                }
                if !remove.is_empty() {
                    let mut i = 0usize;
                    page.visuals.retain(|_| {
                        let keep = !remove.contains(&i);
                        i = i.saturating_add(1);
                        keep
                    });
                }
                page.visuals.extend(add);
            }
        }
    }
    let mut slots: Vec<Option<Page>> = std::mem::take(&mut report.pages)
        .into_iter()
        .chain(new_pages)
        .map(Some)
        .collect();
    report.pages = plan
        .order
        .iter()
        .filter_map(|&i| slots.get_mut(i).and_then(Option::take))
        .collect();
    if let Some(a) = plan.active_page {
        report.active_page = Some(a);
    }
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

fn plan_result(plan: &Plan, applied: bool, dry_run: bool) -> Value {
    let count = |action: &str, kind: &str| {
        plan.changes
            .iter()
            .filter(|c| c["action"] == action && c["kind"] == kind)
            .count()
    };
    json!({
        "valid": plan.error_count == 0,
        "in_sync": plan.error_count == 0 && plan.change_count == 0,
        "applied": applied,
        "dry_run": dry_run,
        "change_count": plan.change_count,
        "summary": {
            "pages_created": count("create", "page"),
            "pages_updated": count("update", "page"),
            "pages_removed": count("remove", "page"),
            "visuals_created": count("create", "visual"),
            "visuals_updated": count("update", "visual"),
            "visuals_removed": count("remove", "visual"),
        },
        "changes": plan.changes,
        "error_count": plan.error_count,
        "errors": plan.errors,
        "warning_count": plan.warning_count,
        "warnings": plan.warnings,
        "ids": plan.ids,
    })
}

fn run_apply(s: &mut Session, p: &Value, dry_run: bool) -> Result<Value, String> {
    let spec_v = p
        .get("spec")
        .ok_or("missing parameter `spec` (an object {pages: [...]})")?;
    let spec = parse_spec(spec_v)?;
    let ctx_force = force_param(p)?;
    let prune = parse_prune(p)?;
    let (model, r) = parts(s)?;
    let ctx = Ctx {
        model,
        force: ctx_force,
        prune,
        measures: crate::report_measures::report_measures(r),
    };
    let plan = make_plan(r, &spec, &ctx);
    if dry_run {
        return Ok(plan_result(&plan, false, true));
    }
    if plan.error_count > 0 {
        let first: Vec<String> = plan
            .errors
            .iter()
            .take(MAX_ERRORS_IN_MESSAGE)
            .map(|e| {
                format!(
                    "{}: {}",
                    e["at"].as_str().unwrap_or_default(),
                    e["message"].as_str().unwrap_or_default()
                )
            })
            .collect();
        let more = plan.error_count.saturating_sub(first.len());
        return Err(format!(
            "report.apply changed nothing: {} problem(s): {}{}",
            plan.error_count,
            first.join("; "),
            if more > 0 {
                format!("; and {more} more (report.check lists them all)")
            } else {
                String::new()
            }
        ));
    }
    let changed = plan.change_count > 0;
    let mut out = plan_result(&plan, changed, false);
    let mut installed: Vec<&str> = Vec::new();
    if changed {
        // BIcraft's visuals the plan creates or changes: their packages first (on first use,
        // as visual.add does; a legacy report refuses before anything changes)
        for b in builtins_in(&plan) {
            if crate::builtin_visuals::ensure(s, b)? {
                installed.push(b.name);
            }
        }
        let (_, r) = parts(s)?;
        execute(r, plan);
        s.report_dirty = true;
    }
    if let Some(o) = out.as_object_mut() {
        o.remove("errors");
        o.remove("error_count");
        o.insert("installed".into(), json!(installed));
    }
    Ok(out)
}

/// BIcraft's visuals used by the visuals a plan creates or changes, each once.
fn builtins_in(plan: &Plan) -> Vec<&'static crate::builtin_visuals::BuiltinVisual> {
    let mut out: Vec<&'static crate::builtin_visuals::BuiltinVisual> = Vec::new();
    for pp in &plan.pages {
        let visuals: Vec<&Visual> = match pp {
            PagePlan::New(page) => page.visuals.iter().collect(),
            PagePlan::Existing { replace, add, .. } => {
                replace.iter().map(|(_, v)| v).chain(add.iter()).collect()
            }
        };
        for v in visuals {
            if let Some(b) = v.visual_type.as_deref().and_then(crate::builtin_visuals::find)
                && !out.iter().any(|x| x.guid == b.guid)
            {
                out.push(b);
            }
        }
    }
    out
}

pub(crate) fn report_apply(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["spec", "prune", "dry_run", "force"])?;
    let dry_run = opt_bool(p, "dry_run")?.unwrap_or(false);
    run_apply(s, p, dry_run)
}

pub(crate) fn report_check(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["spec", "prune", "force"])?;
    // the dry run of `report.apply`, on a shared session: the plan only
    let spec_v = p
        .get("spec")
        .ok_or("missing parameter `spec` (an object {pages: [...]})")?;
    let spec = parse_spec(spec_v)?;
    let ctx_force = force_param(p)?;
    let prune = parse_prune(p)?;
    let r = crate::report::report_ref(s)?;
    let ctx = Ctx {
        model: s.model.as_ref(),
        force: ctx_force,
        prune,
        measures: crate::report_measures::report_measures(r),
    };
    let plan = make_plan(r, &spec, &ctx);
    Ok(plan_result(&plan, false, true))
}

pub(crate) fn report_extract(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page"])?;
    let only = opt_str(p, "page")?;
    let r = report_ref(s)?;
    let only = only.map(|k| page_index(r, k)).transpose()?;
    let pages: Vec<Value> = r
        .pages
        .iter()
        .enumerate()
        .filter(|(i, _)| only.is_none_or(|o| o == *i))
        .map(|(_, page)| extract_page(page))
        .collect();
    let mut spec = Map::new();
    spec.insert("pages".into(), Value::Array(pages));
    if only.is_none()
        && let Some(a) = &r.active_page
    {
        spec.insert("active_page".into(), json!(a));
    }
    Ok(Value::Object(spec))
}

fn put_size(o: &mut Map<String, Value>, key: &str, v: f64) {
    if check_size(key, v).is_ok() {
        o.insert(key.into(), json!(v));
    }
}

fn put_coordinate(o: &mut Map<String, Value>, key: &str, v: f64) {
    if check_coordinate(key, v).is_ok() {
        o.insert(key.into(), json!(v));
    }
}

fn extract_page(page: &Page) -> Value {
    let mut o = Map::new();
    o.insert("name".into(), json!(page.name));
    o.insert("display_name".into(), json!(page.display_name));
    put_size(&mut o, "width", page.width);
    put_size(&mut o, "height", page.height);
    if let Some(d) = page
        .display_option
        .as_deref()
        .and_then(|d| DISPLAY_OPTIONS.iter().find(|o| o.eq_ignore_ascii_case(d)))
    {
        o.insert("display_option".into(), json!(d));
    }
    if is_hidden(page.visibility.as_deref()) {
        o.insert("hidden".into(), json!(true));
    }
    let visuals: Vec<Value> = page.visuals.iter().map(extract_visual).collect();
    o.insert("visuals".into(), Value::Array(visuals));
    Value::Object(o)
}

fn extract_visual(v: &Visual) -> Value {
    let mut o = Map::new();
    o.insert("name".into(), json!(v.name));
    match &v.visual_type {
        Some(t) => {
            o.insert("type".into(), json!(t));
        }
        None => {
            o.insert("group".into(), json!(true));
        }
    }
    let p = &v.position;
    put_coordinate(&mut o, "x", p.x);
    put_coordinate(&mut o, "y", p.y);
    put_size(&mut o, "width", p.width);
    put_size(&mut o, "height", p.height);
    if p.z != 0.0 {
        put_coordinate(&mut o, "z", p.z);
    }
    if v.is_hidden {
        o.insert("hidden".into(), json!(true));
    }
    if !v.fields.is_empty() && v.visual_type.is_some() {
        let fields: Vec<Value> = v
            .fields
            .iter()
            .map(|b| match &b.field {
                FieldRef::Other { raw } => json!({"role": b.role, "raw": raw}),
                f => json!({"role": b.role, "field": field_text(f)}),
            })
            .collect();
        o.insert("fields".into(), Value::Array(fields));
    }
    let props = crate::visual_props::extract_props(v);
    if props.as_object().is_some_and(|p| !p.is_empty()) {
        o.insert("props".into(), props);
    }
    Value::Object(o)
}
