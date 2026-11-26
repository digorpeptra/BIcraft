//! The test file of `project.test` (`bicraft.tests.yaml` / `.yml` / `.json`, version 1;
//! `docs/ci-tests.md`): its schema, checked strictly. Unknown keys, wrong types and duplicate
//! test ids are errors that name where they are (`dax[2].tolerance`), so a typo never silently
//! disables a gate.

use std::collections::HashSet;

use bicraft_pbir::Regex;
use serde_json::{Map, Value};

use crate::citest::Level;

/// The only test-file version this build reads.
pub(crate) const VERSION: u64 = 1;
/// Most DAX assertions, model rules or report rules in one file.
const MAX_ITEMS: usize = 10_000;
/// Default time budget of one DAX assertion.
pub(crate) const DEFAULT_TIMEOUT_MS: u64 = 60_000;
/// The DAX evaluator's own ceiling (`dax.evaluate`).
const MAX_TIMEOUT_MS: u64 = 600_000;
/// Longest DAX text in an assertion.
const MAX_DAX_BYTES: usize = 1 << 20;
/// Most rows an expected table may list.
pub(crate) const MAX_EXPECTED_ROWS: usize = 100_000;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Defaults {
    pub tolerance: f64,
    pub relative_tolerance: f64,
    pub require_data: bool,
    pub timeout_ms: u64,
}

#[derive(Debug)]
pub(crate) struct Spec {
    pub name: Option<String>,
    pub dax: Vec<DaxTest>,
    pub model: Option<ModelSpec>,
    pub report: Option<ReportSpec>,
    pub usage: Option<UsageSpec>,
}

#[derive(Debug)]
pub(crate) enum DaxTarget {
    Measure(String),
    Expression(String),
    Query(String),
}

#[derive(Debug)]
pub(crate) enum Expected {
    /// No value check: the assertion passes when the query runs.
    Nothing,
    /// One value (`null`: BLANK).
    Scalar(Value),
    Table {
        columns: Option<Vec<String>>,
        rows: Option<Vec<Value>>,
        row_count: Option<u64>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypeRule {
    Number,
    Integer,
    Text,
    Boolean,
    Date,
    Blank,
}

impl TypeRule {
    pub fn name(self) -> &'static str {
        match self {
            TypeRule::Number => "number",
            TypeRule::Integer => "integer",
            TypeRule::Text => "text",
            TypeRule::Boolean => "boolean",
            TypeRule::Date => "date",
            TypeRule::Blank => "blank",
        }
    }
}

#[derive(Debug)]
pub(crate) struct DaxTest {
    pub id: String,
    pub name: String,
    pub target: DaxTarget,
    pub filters: Vec<String>,
    pub expect: Expected,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub tolerance: f64,
    pub relative_tolerance: f64,
    pub type_rule: Option<TypeRule>,
    pub ordered: bool,
    pub require_data: bool,
    pub severity: Level,
    pub timeout_ms: u64,
}

#[derive(Debug)]
pub(crate) struct BpaSpec {
    pub default_rules: bool,
    pub rules: Vec<String>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// Lowest BPA severity (1-5) run.
    pub min_severity: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    Table,
    Column,
    Measure,
    Hierarchy,
    Role,
    Expression,
    // DAX objects (forbidden_functions, max_expression_length)
    CalculatedColumn,
    CalculatedTable,
    CalculationItem,
    RowFilter,
}

impl Scope {
    pub fn name(self) -> &'static str {
        match self {
            Scope::Table => "table",
            Scope::Column => "column",
            Scope::Measure => "measure",
            Scope::Hierarchy => "hierarchy",
            Scope::Role => "role",
            Scope::Expression => "expression",
            Scope::CalculatedColumn => "calculated_column",
            Scope::CalculatedTable => "calculated_table",
            Scope::CalculationItem => "calculation_item",
            Scope::RowFilter => "row_filter",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "table" => Scope::Table,
            "column" => Scope::Column,
            "measure" => Scope::Measure,
            "hierarchy" => Scope::Hierarchy,
            "role" => Scope::Role,
            "expression" => Scope::Expression,
            "calculated_column" => Scope::CalculatedColumn,
            "calculated_table" => Scope::CalculatedTable,
            "calculation_item" => Scope::CalculationItem,
            "row_filter" => Scope::RowFilter,
            _ => return None,
        })
    }
    /// The scopes holding DAX expressions.
    pub const DAX: &'static [Scope] = &[
        Scope::Measure,
        Scope::CalculatedColumn,
        Scope::CalculatedTable,
        Scope::CalculationItem,
        Scope::RowFilter,
    ];
    const NAMED: &'static [Scope] = &[
        Scope::Table,
        Scope::Column,
        Scope::Measure,
        Scope::Hierarchy,
        Scope::Role,
        Scope::Expression,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Property {
    Description,
    FormatString,
    DisplayFolder,
}

impl Property {
    pub fn name(self) -> &'static str {
        match self {
            Property::Description => "description",
            Property::FormatString => "format_string",
            Property::DisplayFolder => "display_folder",
        }
    }
}

#[derive(Debug)]
pub(crate) enum ModelRuleKind {
    Naming {
        pattern: Option<(String, Regex)>,
        forbid: Option<(String, Regex)>,
    },
    Required(Property),
    ForbiddenFunctions(Vec<String>),
    MaxExpressionLength(usize),
}

/// Which objects a rule applies to.
#[derive(Debug, Default)]
pub(crate) struct Where {
    pub tables: Vec<String>,
    pub names: Vec<String>,
    pub hidden: Option<bool>,
}

#[derive(Debug)]
pub(crate) struct ModelRule {
    pub id: String,
    pub name: String,
    pub severity: Level,
    pub scopes: Vec<Scope>,
    pub filter: Where,
    pub kind: ModelRuleKind,
}

#[derive(Debug)]
pub(crate) struct ModelSpec {
    pub validate: bool,
    pub bpa: Option<BpaSpec>,
    pub rules: Vec<ModelRule>,
}

/// The categories of `report.validate` findings.
pub(crate) const CATEGORIES: &[&str] = &["schema", "structure", "fields", "filters", "bookmarks"];

#[derive(Debug)]
pub(crate) struct ValidateSpec {
    pub categories: Vec<&'static str>,
}

#[derive(Debug)]
pub(crate) enum ReportRuleKind {
    /// `report.validate` findings of these categories.
    Validate(Vec<&'static str>),
    MaxVisualsPerPage {
        max: usize,
        count_hidden: bool,
    },
    AltText {
        exclude_types: Vec<String>,
    },
    PageSize {
        allowed: Vec<(f64, f64)>,
        include_tooltips: bool,
    },
    Theme {
        custom: Option<Option<String>>,
        base: Option<String>,
        forbid_hardcoded_colors: bool,
    },
    NoHiddenPageNavigation,
}

#[derive(Debug)]
pub(crate) struct ReportRule {
    pub id: String,
    pub name: String,
    pub severity: Level,
    pub pages: Vec<String>,
    pub kind: ReportRuleKind,
}

#[derive(Debug)]
pub(crate) struct ReportSpec {
    pub validate: Option<ValidateSpec>,
    pub rules: Vec<ReportRule>,
}

#[derive(Debug)]
pub(crate) struct UsageSpec {
    pub no_unused_measures: Option<Level>,
}

/// Decorative visual types `alt_text` leaves out unless told otherwise.
pub(crate) const DECORATIVE: &[&str] = &["textbox", "shape", "basicShape", "actionButton", "line"];

// ---------------------------------------------------------------------------------------------

/// A JSON object being read, with its location for messages.
struct Obj<'a> {
    at: String,
    map: &'a Map<String, Value>,
}

fn obj<'a>(v: &'a Value, at: &str) -> Result<Obj<'a>, String> {
    match v {
        Value::Object(map) => Ok(Obj {
            at: at.to_string(),
            map,
        }),
        _ => Err(format!("{}: expected a mapping", show(at))),
    }
}

fn show(at: &str) -> &str {
    if at.is_empty() { "the test file" } else { at }
}

fn join(at: &str, key: &str) -> String {
    if at.is_empty() {
        key.to_string()
    } else {
        format!("{at}.{key}")
    }
}

impl<'a> Obj<'a> {
    fn allow(&self, keys: &[&str]) -> Result<(), String> {
        for k in self.map.keys() {
            if !keys.contains(&k.as_str()) {
                return Err(format!(
                    "{}: unknown key `{k}` (expected one of: {})",
                    show(&self.at),
                    keys.join(", ")
                ));
            }
        }
        Ok(())
    }
    fn get(&self, key: &str) -> Option<&'a Value> {
        self.map.get(key).filter(|v| !v.is_null())
    }
    fn has(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }
    fn path(&self, key: &str) -> String {
        join(&self.at, key)
    }
    fn string(&self, key: &str) -> Result<Option<String>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(format!("{}: expected text", self.path(key))),
        }
    }
    fn text(&self, key: &str) -> Result<Option<String>, String> {
        match self.string(key)? {
            Some(s) if s.len() > MAX_DAX_BYTES => {
                Err(format!("{}: longer than {MAX_DAX_BYTES} bytes", self.path(key)))
            }
            other => Ok(other),
        }
    }
    fn boolean(&self, key: &str) -> Result<Option<bool>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(format!("{}: expected true or false", self.path(key))),
        }
    }
    fn number(&self, key: &str) -> Result<Option<f64>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => v
                .as_f64()
                .filter(|x| x.is_finite())
                .map(Some)
                .ok_or_else(|| format!("{}: expected a number", self.path(key))),
        }
    }
    fn non_negative(&self, key: &str) -> Result<Option<f64>, String> {
        match self.number(key)? {
            Some(x) if x < 0.0 => Err(format!("{}: must not be negative", self.path(key))),
            other => Ok(other),
        }
    }
    fn count(&self, key: &str) -> Result<Option<u64>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => v
                .as_u64()
                .map(Some)
                .ok_or_else(|| format!("{}: expected a non-negative integer", self.path(key))),
        }
    }
    fn strings(&self, key: &str) -> Result<Vec<String>, String> {
        match self.get(key) {
            None => Ok(Vec::new()),
            Some(Value::String(s)) => Ok(vec![s.clone()]),
            Some(Value::Array(items)) if items.len() <= MAX_ITEMS => items
                .iter()
                .enumerate()
                .map(|(i, x)| {
                    x.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| format!("{}[{i}]: expected text", self.path(key)))
                })
                .collect(),
            Some(_) => Err(format!("{}: expected text or a list of text", self.path(key))),
        }
    }
    fn list(&self, key: &str) -> Result<&'a [Value], String> {
        match self.get(key) {
            None => Ok(&[]),
            Some(Value::Array(items)) if items.len() <= MAX_ITEMS => Ok(items),
            Some(Value::Array(_)) => Err(format!("{}: more than {MAX_ITEMS} entries", self.path(key))),
            Some(_) => Err(format!("{}: expected a list", self.path(key))),
        }
    }
    fn level(&self, key: &str, default: Level) -> Result<Level, String> {
        match self.string(key)? {
            None => Ok(default),
            Some(s) => {
                Level::parse(&s).ok_or_else(|| format!("{}: expected error, warning or info", self.path(key)))
            }
        }
    }
    /// `enabled: false` turns a section off (default on).
    fn enabled(&self) -> Result<bool, String> {
        Ok(self.boolean("enabled")?.unwrap_or(true))
    }
}

/// A test id: letters, digits, `-`, `_`, `.`; derived from the name when absent.
fn test_id(o: &Obj<'_>, name: &str, fallback: String) -> Result<String, String> {
    if let Some(id) = o.string("id")? {
        if id.is_empty()
            || id.len() > 200
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err(format!(
                "{}: an id is 1 to 200 letters, digits, `-`, `_` or `.`",
                o.path("id")
            ));
        }
        return Ok(id);
    }
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
        if slug.len() >= 80 {
            break;
        }
    }
    let slug = slug.trim_end_matches('-').to_string();
    Ok(if slug.is_empty() { fallback } else { slug })
}

struct Ids(HashSet<String>);

impl Ids {
    fn claim(&mut self, suite: &str, id: &str, at: &str) -> Result<(), String> {
        if self.0.insert(format!("{suite}/{}", id.to_ascii_lowercase())) {
            Ok(())
        } else {
            Err(format!(
                "{at}: duplicate id `{id}` in `{suite}` (give the entry its own `id`)"
            ))
        }
    }
}

fn type_rule(s: &str) -> Option<TypeRule> {
    Some(match s {
        "number" => TypeRule::Number,
        "integer" => TypeRule::Integer,
        "text" | "string" => TypeRule::Text,
        "boolean" | "bool" => TypeRule::Boolean,
        "date" | "datetime" => TypeRule::Date,
        "blank" => TypeRule::Blank,
        _ => return None,
    })
}

/// Checks the value of an expected cell: a scalar.
fn scalar(v: &Value, at: &str) -> Result<(), String> {
    match v {
        Value::Array(_) | Value::Object(_) => Err(format!("{at}: expected a single value")),
        _ => Ok(()),
    }
}

fn expected(o: &Obj<'_>, query: bool) -> Result<Expected, String> {
    if !o.has("expect") {
        return Ok(Expected::Nothing);
    }
    match o.map.get("expect") {
        None => Ok(Expected::Nothing),
        Some(Value::Object(_)) if query => {
            let e = obj(o.map.get("expect").unwrap_or(&Value::Null), &o.path("expect"))?;
            e.allow(&["columns", "rows", "row_count"])?;
            let columns = if e.has("columns") {
                Some(e.strings("columns")?)
            } else {
                None
            };
            let rows = match e.get("rows") {
                None => None,
                Some(Value::Array(rows)) if rows.len() <= MAX_EXPECTED_ROWS => {
                    for (i, r) in rows.iter().enumerate() {
                        let at = format!("{}[{i}]", e.path("rows"));
                        match r {
                            Value::Array(cells) => {
                                for (j, c) in cells.iter().enumerate() {
                                    scalar(c, &format!("{at}[{j}]"))?;
                                }
                            }
                            Value::Object(cells) => {
                                for (k, c) in cells {
                                    scalar(c, &format!("{at}.{k}"))?;
                                }
                            }
                            _ => return Err(format!("{at}: a row is a list of values or a mapping")),
                        }
                    }
                    Some(rows.clone())
                }
                Some(Value::Array(_)) => {
                    return Err(format!("{}: more than {MAX_EXPECTED_ROWS} rows", e.path("rows")));
                }
                Some(_) => return Err(format!("{}: expected a list of rows", e.path("rows"))),
            };
            Ok(Expected::Table {
                columns,
                rows,
                row_count: e.count("row_count")?,
            })
        }
        Some(v) => {
            scalar(v, &o.path("expect")).map_err(|e| {
                if query {
                    e
                } else {
                    format!("{e} (a measure or expression returns one value)")
                }
            })?;
            Ok(Expected::Scalar(v.clone()))
        }
    }
}

fn dax_test(v: &Value, i: usize, d: &Defaults, ids: &mut Ids) -> Result<DaxTest, String> {
    let at = format!("dax[{i}]");
    let o = obj(v, &at)?;
    o.allow(&[
        "id",
        "name",
        "measure",
        "expression",
        "query",
        "filters",
        "expect",
        "min",
        "max",
        "tolerance",
        "relative_tolerance",
        "type",
        "ordered",
        "require_data",
        "severity",
        "timeout_ms",
    ])?;
    let targets = [
        o.text("measure")?.map(DaxTarget::Measure),
        o.text("expression")?.map(DaxTarget::Expression),
        o.text("query")?.map(DaxTarget::Query),
    ];
    let mut targets = targets.into_iter().flatten();
    let target = targets
        .next()
        .ok_or_else(|| format!("{at}: needs one of `measure`, `expression` or `query`"))?;
    if targets.next().is_some() {
        return Err(format!(
            "{at}: give only one of `measure`, `expression` or `query`"
        ));
    }
    let query = matches!(target, DaxTarget::Query(_));
    let filters = o.strings("filters")?;
    if query && !filters.is_empty() {
        return Err(format!(
            "{at}: `filters` apply to a measure or expression; write them into the query"
        ));
    }
    if filters.iter().any(|f| f.len() > MAX_DAX_BYTES) {
        return Err(format!("{}: a filter is too long", o.path("filters")));
    }
    let name = match o.string("name")? {
        Some(n) => n,
        None => match &target {
            DaxTarget::Measure(m) => m.clone(),
            DaxTarget::Expression(_) | DaxTarget::Query(_) => format!("dax {}", i.saturating_add(1)),
        },
    };
    let id = test_id(&o, &name, format!("dax-{}", i.saturating_add(1)))?;
    ids.claim("dax", &id, &at)?;
    let type_rule = match o.string("type")? {
        None => None,
        Some(t) => Some(type_rule(&t).ok_or_else(|| {
            format!(
                "{}: expected number, integer, text, boolean, date or blank",
                o.path("type")
            )
        })?),
    };
    let timeout_ms = match o.count("timeout_ms")? {
        None => d.timeout_ms,
        Some(t) if (1..=MAX_TIMEOUT_MS).contains(&t) => t,
        Some(_) => return Err(format!("{}: 1 to {MAX_TIMEOUT_MS}", o.path("timeout_ms"))),
    };
    Ok(DaxTest {
        id,
        name,
        filters,
        expect: expected(&o, query)?,
        min: o.number("min")?,
        max: o.number("max")?,
        tolerance: o.non_negative("tolerance")?.unwrap_or(d.tolerance),
        relative_tolerance: o
            .non_negative("relative_tolerance")?
            .unwrap_or(d.relative_tolerance),
        type_rule,
        ordered: o.boolean("ordered")?.unwrap_or(true),
        require_data: o.boolean("require_data")?.unwrap_or(d.require_data),
        severity: o.level("severity", Level::Error)?,
        timeout_ms,
        target,
    })
}

fn severity_number(o: &Obj<'_>, key: &str) -> Result<Option<u8>, String> {
    match o.get(key) {
        None => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .filter(|x| (1..=5).contains(x))
            .map(|x| Some(x as u8))
            .ok_or_else(|| format!("{}: 1 to 5", o.path(key))),
        Some(Value::String(t)) => bicraft_bpa::severity_from_text(t)
            .map(Some)
            .ok_or_else(|| format!("{}: 1-5, info, warning or error", o.path(key))),
        Some(_) => Err(format!("{}: 1-5, info, warning or error", o.path(key))),
    }
}

fn bpa(v: &Value, at: &str) -> Result<Option<BpaSpec>, String> {
    if let Value::Bool(b) = v {
        return Ok(b.then(|| BpaSpec {
            default_rules: true,
            rules: Vec::new(),
            include: Vec::new(),
            exclude: Vec::new(),
            min_severity: 2,
        }));
    }
    let o = obj(v, at)?;
    o.allow(&[
        "enabled",
        "default_rules",
        "rules",
        "include",
        "exclude",
        "min_severity",
    ])?;
    if !o.enabled()? {
        return Ok(None);
    }
    Ok(Some(BpaSpec {
        default_rules: o.boolean("default_rules")?.unwrap_or(true),
        rules: o.strings("rules")?,
        include: o.strings("include")?,
        exclude: o.strings("exclude")?,
        min_severity: severity_number(&o, "min_severity")?.unwrap_or(2),
    }))
}

fn regex(o: &Obj<'_>, key: &str) -> Result<Option<(String, Regex)>, String> {
    match o.string(key)? {
        None => Ok(None),
        Some(p) => Regex::new(&p).map(|r| Some((p.clone(), r))).ok_or_else(|| {
            format!(
                "{}: not a supported regular expression (ECMAScript subset: classes, groups, \
                 alternation, quantifiers, anchors, lookahead)",
                o.path(key)
            )
        }),
    }
}

fn scopes(o: &Obj<'_>, allowed: &[Scope], default: &[Scope]) -> Result<Vec<Scope>, String> {
    let given = o.strings("scope")?;
    if given.is_empty() {
        return Ok(default.to_vec());
    }
    given
        .iter()
        .map(|s| {
            Scope::parse(s).filter(|x| allowed.contains(x)).ok_or_else(|| {
                format!(
                    "{}: `{s}` is not a scope of this rule (expected: {})",
                    o.path("scope"),
                    allowed.iter().map(|x| x.name()).collect::<Vec<_>>().join(", ")
                )
            })
        })
        .collect()
}

fn model_rule(v: &Value, i: usize, ids: &mut Ids) -> Result<ModelRule, String> {
    let at = format!("model.rules[{i}]");
    let o = obj(v, &at)?;
    let kind = o.string("kind")?.ok_or_else(|| format!("{at}: needs a `kind`"))?;
    let mut keys = vec!["id", "name", "kind", "severity", "scope", "where"];
    let (kind, allowed, default): (_, &[Scope], &[Scope]) = match kind.as_str() {
        "naming" => {
            keys.extend(["pattern", "forbid_pattern"]);
            o.allow(&keys)?;
            let pattern = regex(&o, "pattern")?;
            let forbid = regex(&o, "forbid_pattern")?;
            if pattern.is_none() && forbid.is_none() {
                return Err(format!("{at}: naming needs `pattern` or `forbid_pattern`"));
            }
            (
                ModelRuleKind::Naming { pattern, forbid },
                Scope::NAMED,
                Scope::NAMED,
            )
        }
        "required" => {
            keys.push("property");
            o.allow(&keys)?;
            let p = match o.string("property")?.as_deref() {
                Some("description") => Property::Description,
                Some("format_string") => Property::FormatString,
                Some("display_folder") => Property::DisplayFolder,
                _ => {
                    return Err(format!(
                        "{}: description, format_string or display_folder",
                        o.path("property")
                    ));
                }
            };
            let allowed: &[Scope] = match p {
                Property::Description => Scope::NAMED,
                Property::FormatString | Property::DisplayFolder => &[Scope::Measure, Scope::Column],
            };
            (ModelRuleKind::Required(p), allowed, &[Scope::Measure][..])
        }
        "forbidden_functions" => {
            keys.push("functions");
            o.allow(&keys)?;
            let f = o.strings("functions")?;
            if f.is_empty() {
                return Err(format!("{at}: forbidden_functions needs `functions`"));
            }
            (
                ModelRuleKind::ForbiddenFunctions(f.iter().map(|x| x.to_ascii_uppercase()).collect()),
                Scope::DAX,
                Scope::DAX,
            )
        }
        "max_expression_length" => {
            keys.push("max");
            o.allow(&keys)?;
            let max = o
                .count("max")?
                .ok_or_else(|| format!("{at}: max_expression_length needs `max`"))?;
            (
                ModelRuleKind::MaxExpressionLength(usize::try_from(max).unwrap_or(usize::MAX)),
                Scope::DAX,
                Scope::DAX,
            )
        }
        other => {
            return Err(format!(
                "{}: unknown kind `{other}` (naming, required, forbidden_functions, \
                 max_expression_length)",
                o.path("kind")
            ));
        }
    };
    let scopes = scopes(&o, allowed, default)?;
    let filter = match o.get("where") {
        None => Where::default(),
        Some(w) => {
            let w = obj(w, &o.path("where"))?;
            w.allow(&["table", "name", "hidden"])?;
            Where {
                tables: w.strings("table")?,
                names: w.strings("name")?,
                hidden: w.boolean("hidden")?,
            }
        }
    };
    let name = o
        .string("name")?
        .unwrap_or_else(|| format!("{} rule {}", kind_name(&kind), i.saturating_add(1)));
    let id = test_id(&o, &name, format!("rule-{}", i.saturating_add(1)))?;
    ids.claim("model", &id, &at)?;
    Ok(ModelRule {
        id,
        name,
        severity: o.level("severity", Level::Error)?,
        scopes,
        filter,
        kind,
    })
}

pub(crate) fn kind_name(k: &ModelRuleKind) -> &'static str {
    match k {
        ModelRuleKind::Naming { .. } => "naming",
        ModelRuleKind::Required(_) => "required",
        ModelRuleKind::ForbiddenFunctions(_) => "forbidden_functions",
        ModelRuleKind::MaxExpressionLength(_) => "max_expression_length",
    }
}

fn model(v: &Value, ids: &mut Ids) -> Result<Option<ModelSpec>, String> {
    let o = obj(v, "model")?;
    o.allow(&["enabled", "validate", "bpa", "rules"])?;
    if !o.enabled()? {
        return Ok(None);
    }
    let bpa = match o.get("bpa") {
        None => None,
        Some(b) => bpa(b, "model.bpa")?,
    };
    let rules = o
        .list("rules")?
        .iter()
        .enumerate()
        .map(|(i, r)| model_rule(r, i, ids))
        .collect::<Result<_, _>>()?;
    Ok(Some(ModelSpec {
        validate: o.boolean("validate")?.unwrap_or(true),
        bpa,
        rules,
    }))
}

fn categories(o: &Obj<'_>) -> Result<Vec<&'static str>, String> {
    let include = o.strings("include")?;
    let exclude = o.strings("exclude")?;
    for c in include.iter().chain(&exclude) {
        if !CATEGORIES.contains(&c.as_str()) {
            return Err(format!(
                "{}: unknown category `{c}` (expected: {})",
                o.at,
                CATEGORIES.join(", ")
            ));
        }
    }
    Ok(CATEGORIES
        .iter()
        .copied()
        .filter(|c| include.is_empty() || include.iter().any(|x| x == c))
        .filter(|c| !exclude.iter().any(|x| x == c))
        .collect())
}

fn report_rule(v: &Value, i: usize, ids: &mut Ids) -> Result<ReportRule, String> {
    let at = format!("report.rules[{i}]");
    let o = obj(v, &at)?;
    let kind = o.string("kind")?.ok_or_else(|| format!("{at}: needs a `kind`"))?;
    let mut keys = vec!["id", "name", "kind", "severity", "pages"];
    let rule = match kind.as_str() {
        "fields_exist" => ReportRuleKind::Validate(vec!["fields"]),
        "bookmarks_valid" => ReportRuleKind::Validate(vec!["bookmarks"]),
        "max_visuals_per_page" => {
            keys.extend(["max", "count_hidden"]);
            let max = o
                .count("max")?
                .ok_or_else(|| format!("{at}: max_visuals_per_page needs `max`"))?;
            ReportRuleKind::MaxVisualsPerPage {
                max: usize::try_from(max).unwrap_or(usize::MAX),
                count_hidden: o.boolean("count_hidden")?.unwrap_or(false),
            }
        }
        "alt_text" => {
            keys.push("exclude_types");
            ReportRuleKind::AltText {
                exclude_types: if o.has("exclude_types") {
                    o.strings("exclude_types")?
                } else {
                    DECORATIVE.iter().map(|s| (*s).to_string()).collect()
                },
            }
        }
        "page_size" => {
            keys.extend(["width", "height", "allowed", "include_tooltips"]);
            let mut allowed = Vec::new();
            if let (Some(w), Some(h)) = (o.number("width")?, o.number("height")?) {
                allowed.push((w, h));
            } else if o.has("width") || o.has("height") {
                return Err(format!("{at}: give both `width` and `height`"));
            }
            for (j, a) in o.list("allowed")?.iter().enumerate() {
                let a = obj(a, &format!("{}[{j}]", o.path("allowed")))?;
                a.allow(&["width", "height"])?;
                match (a.number("width")?, a.number("height")?) {
                    (Some(w), Some(h)) => allowed.push((w, h)),
                    _ => return Err(format!("{}: needs `width` and `height`", a.at)),
                }
            }
            if allowed.is_empty() {
                return Err(format!(
                    "{at}: page_size needs `width` and `height`, or `allowed`"
                ));
            }
            ReportRuleKind::PageSize {
                allowed,
                include_tooltips: o.boolean("include_tooltips")?.unwrap_or(false),
            }
        }
        "theme" => {
            keys.extend(["custom_theme", "base_theme", "forbid_hardcoded_colors"]);
            let custom = match o.get("custom_theme") {
                None => None,
                Some(Value::Bool(true)) => Some(None),
                Some(Value::Bool(false)) => None,
                Some(Value::String(s)) => Some(Some(s.clone())),
                Some(_) => {
                    return Err(format!(
                        "{}: a theme name, or true for any custom theme",
                        o.path("custom_theme")
                    ));
                }
            };
            let base = o.string("base_theme")?;
            let forbid = o.boolean("forbid_hardcoded_colors")?.unwrap_or(false);
            if custom.is_none() && base.is_none() && !forbid {
                return Err(format!(
                    "{at}: theme needs `custom_theme`, `base_theme` or `forbid_hardcoded_colors`"
                ));
            }
            ReportRuleKind::Theme {
                custom,
                base,
                forbid_hardcoded_colors: forbid,
            }
        }
        "no_hidden_page_navigation" => ReportRuleKind::NoHiddenPageNavigation,
        other => {
            return Err(format!(
                "{}: unknown kind `{other}` (fields_exist, bookmarks_valid, max_visuals_per_page, \
                 alt_text, page_size, theme, no_hidden_page_navigation)",
                o.path("kind")
            ));
        }
    };
    o.allow(&keys)?;
    let name = o.string("name")?.unwrap_or_else(|| kind.replace('_', " "));
    let id = test_id(&o, &name, format!("rule-{}", i.saturating_add(1)))?;
    ids.claim("report", &id, &at)?;
    Ok(ReportRule {
        id,
        name,
        severity: o.level("severity", Level::Error)?,
        pages: o.strings("pages")?,
        kind: rule,
    })
}

fn report(v: &Value, ids: &mut Ids) -> Result<Option<ReportSpec>, String> {
    let o = obj(v, "report")?;
    o.allow(&["enabled", "validate", "rules"])?;
    if !o.enabled()? {
        return Ok(None);
    }
    let validate = match o.get("validate") {
        None | Some(Value::Bool(true)) => Some(ValidateSpec {
            categories: CATEGORIES.to_vec(),
        }),
        Some(Value::Bool(false)) => None,
        Some(v) => {
            let w = obj(v, "report.validate")?;
            w.allow(&["enabled", "include", "exclude"])?;
            if w.enabled()? {
                Some(ValidateSpec {
                    categories: categories(&w)?,
                })
            } else {
                None
            }
        }
    };
    let rules = o
        .list("rules")?
        .iter()
        .enumerate()
        .map(|(i, r)| report_rule(r, i, ids))
        .collect::<Result<_, _>>()?;
    Ok(Some(ReportSpec { validate, rules }))
}

fn usage(v: &Value) -> Result<Option<UsageSpec>, String> {
    let o = obj(v, "usage")?;
    o.allow(&["enabled", "no_unused_measures"])?;
    if !o.enabled()? {
        return Ok(None);
    }
    let no_unused_measures = match o.get("no_unused_measures") {
        None | Some(Value::Bool(false)) => None,
        Some(Value::Bool(true)) => Some(Level::Warning),
        Some(Value::String(s)) => Some(Level::parse(s).ok_or_else(|| {
            "usage.no_unused_measures: true, false, or a severity (error, warning, info)".to_string()
        })?),
        Some(_) => {
            return Err("usage.no_unused_measures: true, false, or a severity (error, warning, info)".into());
        }
    };
    Ok(Some(UsageSpec { no_unused_measures }))
}

/// Reads a test file's JSON form (after YAML or JSON parsing).
pub(crate) fn parse(doc: &Value) -> Result<Spec, String> {
    let o = obj(doc, "")?;
    o.allow(&["version", "name", "defaults", "dax", "model", "report", "usage"])?;
    match o.get("version") {
        Some(v) if v.as_u64() == Some(VERSION) => {}
        Some(v) => {
            return Err(format!(
                "version: this bicraft reads test files of version {VERSION}, not {v}"
            ));
        }
        None => return Err(format!("version: required (`version: {VERSION}`)")),
    }
    let mut d = Defaults {
        tolerance: 0.0,
        relative_tolerance: 1e-9,
        require_data: false,
        timeout_ms: DEFAULT_TIMEOUT_MS,
    };
    if let Some(v) = o.get("defaults") {
        let w = obj(v, "defaults")?;
        w.allow(&["tolerance", "relative_tolerance", "require_data", "timeout_ms"])?;
        d.tolerance = w.non_negative("tolerance")?.unwrap_or(d.tolerance);
        d.relative_tolerance = w
            .non_negative("relative_tolerance")?
            .unwrap_or(d.relative_tolerance);
        d.require_data = w.boolean("require_data")?.unwrap_or(d.require_data);
        if let Some(t) = w.count("timeout_ms")? {
            if !(1..=MAX_TIMEOUT_MS).contains(&t) {
                return Err(format!("defaults.timeout_ms: 1 to {MAX_TIMEOUT_MS}"));
            }
            d.timeout_ms = t;
        }
    }
    let mut ids = Ids(HashSet::new());
    let dax = o
        .list("dax")?
        .iter()
        .enumerate()
        .map(|(i, t)| dax_test(t, i, &d, &mut ids))
        .collect::<Result<_, _>>()?;
    let model = match o.get("model") {
        None => None,
        Some(v) => model(v, &mut ids)?,
    };
    let report = match o.get("report") {
        None => None,
        Some(v) => report(v, &mut ids)?,
    };
    let usage = match o.get("usage") {
        None => None,
        Some(v) => usage(v)?,
    };
    Ok(Spec {
        name: o.string("name")?,
        dax,
        model,
        report,
        usage,
    })
}
