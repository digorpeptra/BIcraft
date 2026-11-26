//! Model, report and usage rules of `project.test`. Built on the existing checks
//! (`model.validate`, `model.analyze`'s BPA rules, `report.validate`) plus declarative rules
//! evaluated here over the model and the report.

use std::path::{Path, PathBuf};
use std::time::Instant;

use bicraft_dax::TokenKind;
use bicraft_model::SemanticModel;
use bicraft_report::{Page, Report, Visual};
use serde_json::{Value, json};

use crate::Session;
use crate::citest::{Case, Finding, Level, Status, Target, TestSource, elapsed};
use crate::citest_spec::{
    BpaSpec, ModelRule, ModelRuleKind, ModelSpec, Property, ReportRule, ReportRuleKind, ReportSpec, Scope,
    UsageSpec, Where,
};

/// Deepest JSON walked looking for hard-coded colors.
const MAX_DEPTH: usize = 64;

// ---------------------------------------------------------------------------------------------
// glob patterns (`*`, `?`), case-insensitive, linear time
// ---------------------------------------------------------------------------------------------

pub(crate) fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        match p.get(pi) {
            Some('*') => {
                star = Some((pi, ti));
                pi += 1;
            }
            Some(c) if *c == '?' || Some(c) == t.get(ti) => {
                pi += 1;
                ti += 1;
            }
            _ => match star {
                Some((sp, st)) => {
                    pi = sp + 1;
                    ti = st + 1;
                    star = Some((sp, st + 1));
                }
                None => return false,
            },
        }
    }
    p.get(pi..).unwrap_or_default().iter().all(|c| *c == '*')
}

fn any_glob(patterns: &[String], text: &str) -> bool {
    patterns.iter().any(|p| glob(p, text))
}

/// Concludes a case whose findings carry their own levels: failed at the highest one.
fn conclude_mixed(case: Case, findings: Vec<Finding>, what: &str) -> Case {
    let level = findings.iter().map(|f| f.level).max();
    let c = case.conclude(findings, what);
    match level {
        Some(l) => c.at(l),
        None => c,
    }
}

// ---------------------------------------------------------------------------------------------
// model
// ---------------------------------------------------------------------------------------------

pub(crate) fn model_cases(
    s: &mut Session,
    spec: &ModelSpec,
    tests: &TestSource,
) -> Result<Vec<Case>, String> {
    let mut out = Vec::new();
    if s.model.is_none() {
        let why = match &s.model_error {
            Some(e) => format!("the model cannot be read: {e}"),
            None => "the open file has no model".into(),
        };
        out.push(Case::new("model", "model", "model", "model").error(why));
        return Ok(out);
    }
    if spec.validate {
        out.extend(validate_cases(s));
    }
    if let Some(b) = &spec.bpa {
        out.extend(bpa_cases(s, b, tests)?);
    }
    if let Some(m) = s.model.as_ref() {
        for r in &spec.rules {
            let started = Instant::now();
            let findings = model_rule(m, r);
            let what = match &r.kind {
                ModelRuleKind::Naming { .. } => "names break the rule",
                ModelRuleKind::Required(_) => "objects lack the property",
                ModelRuleKind::ForbiddenFunctions(_) => "expressions use a forbidden function",
                ModelRuleKind::MaxExpressionLength(_) => "expressions are too long",
            };
            let mut c = Case::new(
                "model",
                r.id.clone(),
                r.name.clone(),
                crate::citest_spec::kind_name(&r.kind),
            )
            .at(r.severity)
            .conclude(findings, what);
            c.elapsed_ms = elapsed(started);
            out.push(c);
        }
    }
    Ok(out)
}

fn validate_cases(s: &Session) -> Vec<Case> {
    let started = Instant::now();
    let v = match crate::commands::model_validation(s) {
        Ok(v) => v,
        Err(e) => return vec![Case::new("model", "validate", "model.validate", "validate").error(e)],
    };
    let level = |sev: Option<&str>| {
        if sev == Some("warning") {
            Level::Warning
        } else {
            Level::Error
        }
    };
    let mut dax = Vec::new();
    let arr = |v: &Value| v.as_array().cloned().unwrap_or_default();
    for o in arr(&v["objects_with_diagnostics"]) {
        let kind = o["kind"].as_str().unwrap_or_default();
        let table = o["table"].as_str().map(str::to_string);
        let name = o["name"].as_str().unwrap_or_default().to_string();
        for d in arr(&o["diagnostics"]) {
            dax.push(Finding::new(
                level(d["severity"].as_str()),
                format!(
                    "{kind} {}: {} [{}]",
                    object_label(table.as_deref(), &name),
                    d["message"].as_str().unwrap_or_default(),
                    d["code"].as_str().unwrap_or_default()
                ),
                Target::Model {
                    kind: kind.replace(' ', "_"),
                    table: table.clone(),
                    name: name.clone(),
                },
            ));
        }
    }
    let mut m = Vec::new();
    for o in arr(&v["m"]["objects_with_diagnostics"]) {
        let name = o["name"].as_str().unwrap_or_default().to_string();
        let table = o["table"].as_str().map(str::to_string);
        for d in arr(&o["diagnostics"]) {
            m.push(Finding::new(
                level(d["severity"].as_str()),
                format!(
                    "M of {}: {}",
                    table
                        .as_deref()
                        .map_or_else(|| name.clone(), |t| format!("'{t}'")),
                    d["message"].as_str().unwrap_or_default()
                ),
                match &table {
                    Some(t) => Target::Model {
                        kind: "table".into(),
                        table: None,
                        name: t.clone(),
                    },
                    None => Target::Model {
                        kind: "expression".into(),
                        table: None,
                        name: name.clone(),
                    },
                },
            ));
        }
    }
    let rel: Vec<Finding> = arr(&v["relationships"]["invalid"])
        .iter()
        .map(|r| {
            Finding::new(
                Level::Error,
                r.get("message")
                    .and_then(Value::as_str)
                    .map_or_else(|| r.to_string(), str::to_string),
                Target::Model {
                    kind: "relationship".into(),
                    table: None,
                    name: r
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                },
            )
        })
        .collect();
    let ms = elapsed(started);
    let mut out = vec![
        conclude_mixed(
            Case::new("model", "validate-dax", "DAX expressions are valid", "validate"),
            dax,
            "DAX diagnostics",
        ),
        conclude_mixed(
            Case::new(
                "model",
                "validate-m",
                "Power Query (M) expressions are valid",
                "validate",
            ),
            m,
            "M diagnostics",
        ),
        conclude_mixed(
            Case::new(
                "model",
                "validate-relationships",
                "Relationships open in Power BI Desktop",
                "validate",
            ),
            rel,
            "invalid relationships",
        ),
    ];
    if let Some(c) = out.first_mut() {
        c.elapsed_ms = ms;
    }
    out
}

fn object_label(table: Option<&str>, name: &str) -> String {
    match table {
        Some(t) if t != name => format!("'{}'[{}]", t.replace('\'', "''"), name.replace(']', "]]")),
        _ => format!("'{}'", name.replace('\'', "''")),
    }
}

/// A rule file path of the test file, resolved against its folder (and, in server mode, under
/// the workspace root).
fn rule_path(s: &Session, raw: &str, tests: &TestSource) -> Result<String, String> {
    match s.server_config() {
        None => {
            let p = Path::new(raw);
            let p = if p.is_absolute() {
                p.to_path_buf()
            } else {
                tests.dir.join(p)
            };
            Ok(p.display().to_string())
        }
        Some(server) => {
            let dir: PathBuf = tests
                .dir
                .strip_prefix(server.root())
                .map(Path::to_path_buf)
                .map_err(|_| "server mode: the test file is outside the workspace root".to_string())?;
            let joined = dir.join(raw);
            let text = joined.to_string_lossy().replace('\\', "/");
            server
                .resolve(&text)
                .map(|p| p.display().to_string())
                .map_err(|why| format!("model.bpa.rules: `{raw}` is refused in server mode: {why}"))
        }
    }
}

fn bpa_kind(object_type: &str) -> &'static str {
    let t = object_type.to_ascii_lowercase();
    if t.contains("measure") {
        "measure"
    } else if t.contains("column") {
        "column"
    } else if t.contains("hierarchy") {
        "hierarchy"
    } else if t.contains("relationship") {
        "relationship"
    } else if t.contains("role") {
        "role"
    } else if t.contains("partition") {
        "partition"
    } else if t.contains("table") {
        "table"
    } else {
        "model"
    }
}

/// The name in a BPA object text (`'Sales'[Amount]` → `Amount`, `'Sales'` → `Sales`).
fn bpa_name(object: &str) -> String {
    if let Some(inner) = object.strip_suffix(']')
        && let Some((_, name)) = inner.rsplit_once('[')
    {
        return name.replace("]]", "]");
    }
    object
        .strip_prefix('\'')
        .and_then(|x| x.strip_suffix('\''))
        .map_or_else(|| object.to_string(), |x| x.replace("''", "'"))
}

fn bpa_cases(s: &mut Session, b: &BpaSpec, tests: &TestSource) -> Result<Vec<Case>, String> {
    let started = Instant::now();
    let rules = b
        .rules
        .iter()
        .map(|r| rule_path(s, r, tests))
        .collect::<Result<Vec<_>, _>>()?;
    let params = json!({
        "default_rules": b.default_rules,
        "rules": rules,
        "severity_min": b.min_severity,
    });
    let a = match crate::analyze::model_analyze(s, &params) {
        Ok(a) => a,
        // an unreadable or invalid rule file is a configuration error
        Err(e) => return Err(format!("model.bpa: {e}")),
    };
    let ms = elapsed(started);
    let mut out = Vec::new();
    let invalid: Vec<Finding> = a["invalid_rules"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|r| {
                    Finding::new(
                        Level::Error,
                        format!(
                            "rule {} of {}: {}",
                            r["index"],
                            r["source"].as_str().unwrap_or_default(),
                            r["reason"].as_str().unwrap_or_default()
                        ),
                        Target::None,
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if !invalid.is_empty() {
        out.push(
            Case::new("bpa", "invalid-rules", "BPA rule files are valid", "bpa")
                .conclude(invalid, "invalid rules"),
        );
    }
    let findings = a["findings"].as_array().cloned().unwrap_or_default();
    for r in a["rules"].as_array().into_iter().flatten() {
        let id = r["rule_id"].as_str().unwrap_or_default();
        let status = r["status"].as_str().unwrap_or_default();
        if status == "skipped"
            || (!b.include.is_empty() && !any_glob(&b.include, id))
            || any_glob(&b.exclude, id)
        {
            continue;
        }
        let level = match r["severity"].as_str() {
            Some("error") => Level::Error,
            Some("warning") => Level::Warning,
            _ => Level::Info,
        };
        let case = Case::new(
            "bpa",
            id.to_string(),
            r["rule_name"].as_str().unwrap_or(id).to_string(),
            r["category"].as_str().unwrap_or("bpa").to_string(),
        )
        .at(level);
        let mut case = if status == "not_evaluated" {
            case.skip(format!(
                "not evaluated: {}",
                r["reason"]
                    .as_str()
                    .unwrap_or("the rule uses a construct bicraft does not interpret")
            ))
        } else {
            let list: Vec<Finding> = findings
                .iter()
                .filter(|f| f["rule_id"].as_str() == Some(id))
                .map(|f| {
                    let object = f["object"].as_str().unwrap_or_default();
                    let kind = f["object_type"].as_str().unwrap_or_default();
                    Finding::new(
                        level,
                        format!("{kind} {object}"),
                        Target::Model {
                            kind: bpa_kind(kind).into(),
                            table: f["table"].as_str().map(str::to_string),
                            name: bpa_name(object),
                        },
                    )
                })
                .collect();
            let mut c = case.conclude(list, "objects break the rule");
            if c.status == Status::Failed {
                c.message = format!("{}: {}", c.message, r["rule_name"].as_str().unwrap_or(id));
            }
            if status == "partial"
                && let Some(why) = r["reason"].as_str()
            {
                c.message = format!("{} (partly evaluated: {why})", c.message);
            }
            c
        };
        case.details = Some(json!({"source": r["source"], "status": status, "objects": r["objects"]}));
        out.push(case);
    }
    if let Some(c) = out.first_mut() {
        c.elapsed_ms = ms;
    }
    Ok(out)
}

/// One object a declarative model rule looks at.
struct Obj<'m> {
    scope: Scope,
    table: Option<&'m str>,
    name: &'m str,
    hidden: bool,
    description: Option<&'m str>,
    format_string: Option<&'m str>,
    display_folder: Option<&'m str>,
    expression: Option<&'m str>,
}

impl Obj<'_> {
    fn label(&self) -> String {
        let what = self.scope.name().replace('_', " ");
        match self.scope {
            Scope::Table | Scope::CalculatedTable | Scope::Role | Scope::Expression => {
                format!("{what} '{}'", self.name)
            }
            _ => format!("{what} {}", object_label(self.table, self.name)),
        }
    }
    fn target(&self) -> Target {
        let kind = match self.scope {
            Scope::CalculatedColumn => "column",
            Scope::CalculatedTable => "table",
            Scope::CalculationItem => "calculation_item",
            other => other.name(),
        };
        Target::Model {
            kind: kind.into(),
            table: self.table.map(str::to_string),
            name: self.name.to_string(),
        }
    }
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn objects<'m>(m: &'m SemanticModel, scopes: &[Scope]) -> Vec<Obj<'m>> {
    let mut out = Vec::new();
    let blank = Obj {
        scope: Scope::Table,
        table: None,
        name: "",
        hidden: false,
        description: None,
        format_string: None,
        display_folder: None,
        expression: None,
    };
    for t in &m.tables {
        if scopes.contains(&Scope::Table) {
            out.push(Obj {
                scope: Scope::Table,
                name: &t.name,
                hidden: t.is_hidden,
                description: t.description.as_deref(),
                ..blank
            });
        }
        if scopes.contains(&Scope::Column) {
            for c in &t.columns {
                out.push(Obj {
                    scope: Scope::Column,
                    table: Some(&t.name),
                    name: &c.name,
                    hidden: c.is_hidden,
                    description: c.description.as_deref(),
                    format_string: c.format_string.as_deref(),
                    display_folder: c.display_folder.as_deref(),
                    ..blank
                });
            }
        }
        if scopes.contains(&Scope::Measure) {
            for x in &t.measures {
                out.push(Obj {
                    scope: Scope::Measure,
                    table: Some(&t.name),
                    name: &x.name,
                    hidden: x.is_hidden,
                    description: x.description.as_deref(),
                    format_string: x.format_string.as_deref(),
                    display_folder: x.display_folder.as_deref(),
                    expression: Some(&x.expression),
                });
            }
        }
        if scopes.contains(&Scope::Hierarchy) {
            for h in t
                .extra
                .get("hierarchies")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                out.push(Obj {
                    scope: Scope::Hierarchy,
                    table: Some(&t.name),
                    name: str_of(h, "name").unwrap_or_default(),
                    hidden: h.get("isHidden").and_then(Value::as_bool).unwrap_or(false),
                    description: str_of(h, "description"),
                    ..blank
                });
            }
        }
    }
    if scopes.contains(&Scope::Role) {
        for r in &m.roles {
            out.push(Obj {
                scope: Scope::Role,
                name: &r.name,
                description: r.extra.get("description").and_then(Value::as_str),
                ..blank
            });
        }
    }
    if scopes.contains(&Scope::Expression) {
        for e in &m.expressions {
            out.push(Obj {
                scope: Scope::Expression,
                name: &e.name,
                description: e.description.as_deref(),
                expression: Some(&e.expression),
                ..blank
            });
        }
    }
    // DAX objects other than measures
    let dax_scopes = [
        Scope::CalculatedColumn,
        Scope::CalculatedTable,
        Scope::CalculationItem,
        Scope::RowFilter,
    ];
    if dax_scopes.iter().any(|x| scopes.contains(x)) {
        for o in crate::commands::dax_objects(m) {
            let scope = match o.label {
                "calculated column" => Scope::CalculatedColumn,
                "calculated table" => Scope::CalculatedTable,
                "calculation item" | "calculation item format string" => Scope::CalculationItem,
                "row filter" => Scope::RowFilter,
                _ => continue,
            };
            if !scopes.contains(&scope) {
                continue;
            }
            let table = m.tables.iter().find(|t| t.name == o.table);
            let hidden = match scope {
                Scope::CalculatedColumn => table
                    .and_then(|t| t.columns.iter().find(|c| c.name == o.name))
                    .is_some_and(|c| c.is_hidden),
                Scope::CalculatedTable => table.is_some_and(|t| t.is_hidden),
                _ => false,
            };
            out.push(Obj {
                scope,
                table: (scope != Scope::CalculatedTable).then_some(o.table),
                name: o.name,
                hidden,
                expression: Some(o.expression),
                ..blank
            });
        }
    }
    out
}

fn wanted(o: &Obj<'_>, w: &Where) -> bool {
    let table = o.table.unwrap_or(o.name);
    (w.tables.is_empty() || any_glob(&w.tables, table))
        && (w.names.is_empty() || any_glob(&w.names, o.name))
        && w.hidden.is_none_or(|h| h == o.hidden)
}

/// The functions an expression calls (upper case, once each, in order).
fn called_functions(expression: &str) -> Vec<String> {
    let tokens = bicraft_dax::tokenize(expression);
    let mut out: Vec<String> = Vec::new();
    for pair in tokens.windows(2) {
        if let [a, b] = pair
            && let TokenKind::Ident(name) = &a.kind
            && b.kind == TokenKind::LParen
        {
            let up = name.to_ascii_uppercase();
            if !out.contains(&up) {
                out.push(up);
            }
        }
    }
    out
}

fn model_rule(m: &SemanticModel, r: &ModelRule) -> Vec<Finding> {
    let mut out = Vec::new();
    for o in objects(m, &r.scopes) {
        if !wanted(&o, &r.filter) {
            continue;
        }
        match &r.kind {
            ModelRuleKind::Naming { pattern, forbid } => {
                if let Some((p, re)) = pattern
                    && !re.is_match(o.name)
                {
                    out.push(Finding::new(
                        r.severity,
                        format!("{}: the name does not match `{p}`", o.label()),
                        o.target(),
                    ));
                }
                if let Some((p, re)) = forbid
                    && re.is_match(o.name)
                {
                    out.push(Finding::new(
                        r.severity,
                        format!("{}: the name matches the forbidden `{p}`", o.label()),
                        o.target(),
                    ));
                }
            }
            ModelRuleKind::Required(p) => {
                let v = match p {
                    Property::Description => o.description,
                    Property::FormatString => o.format_string,
                    Property::DisplayFolder => o.display_folder,
                };
                if v.is_none_or(|x| x.trim().is_empty()) {
                    out.push(Finding::new(
                        r.severity,
                        format!("{} has no {}", o.label(), p.name().replace('_', " ")),
                        o.target(),
                    ));
                }
            }
            ModelRuleKind::ForbiddenFunctions(list) => {
                let Some(e) = o.expression else { continue };
                if o.scope == Scope::Expression {
                    continue;
                }
                let used: Vec<String> = called_functions(e)
                    .into_iter()
                    .filter(|f| list.contains(f))
                    .collect();
                if !used.is_empty() {
                    out.push(Finding::new(
                        r.severity,
                        format!("{} uses {}", o.label(), used.join(", ")),
                        o.target(),
                    ));
                }
            }
            ModelRuleKind::MaxExpressionLength(max) => {
                let Some(e) = o.expression else { continue };
                let n = e.chars().count();
                if n > *max {
                    out.push(Finding::new(
                        r.severity,
                        format!(
                            "{}: the expression is {n} characters long (at most {max})",
                            o.label()
                        ),
                        o.target(),
                    ));
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// report
// ---------------------------------------------------------------------------------------------

/// A `report.validate` finding with its category.
struct Issue {
    category: &'static str,
    finding: Finding,
    page: Option<String>,
}

fn validation_issues(s: &Session) -> Result<Vec<Issue>, String> {
    let v = crate::report::report_validation(s, true)?;
    let mut out = Vec::new();
    for i in v["schema_issues"].as_array().into_iter().flatten() {
        let path = i["path"].as_str().unwrap_or_default().to_string();
        let pointer = i["pointer"].as_str().unwrap_or_default();
        out.push(Issue {
            category: "schema",
            finding: Finding::new(
                Level::Error,
                format!(
                    "{path}{}{pointer}: {}",
                    if pointer.is_empty() { "" } else { " " },
                    i["message"].as_str().unwrap_or_default()
                ),
                Target::ReportFile(path),
            ),
            page: None,
        });
    }
    for i in v["issues"].as_array().into_iter().flatten() {
        let message = i["message"].as_str().unwrap_or_default();
        let part = i["part"].as_str().unwrap_or_default();
        let page = i["page"].as_str().map(str::to_string);
        let visual = i["visual"].as_str().map(str::to_string);
        let bookmark = part.strip_prefix("bookmark ").map(str::to_string);
        let lower = message.to_ascii_lowercase();
        let category = if bookmark.is_some() || lower.contains("bookmark") {
            "bookmarks"
        } else if part == "filters" || lower.contains("filter") {
            "filters"
        } else if i.get("field").is_some() {
            "fields"
        } else {
            "structure"
        };
        let mut text = String::new();
        if let Some(p) = &page {
            text.push_str(&format!("page '{p}'"));
        }
        if let Some(v) = &visual {
            text.push_str(&format!("{}visual {v}", if text.is_empty() { "" } else { ", " }));
        }
        if let Some(b) = &bookmark {
            text.push_str(&format!(
                "{}bookmark {b}",
                if text.is_empty() { "" } else { ", " }
            ));
        }
        if let Some(f) = i["field"].as_str() {
            text.push_str(&format!("{}{f}", if text.is_empty() { "" } else { ", " }));
        }
        let level = if i["severity"].as_str() == Some("error") {
            Level::Error
        } else {
            Level::Warning
        };
        out.push(Issue {
            category,
            finding: Finding::new(
                level,
                if text.is_empty() {
                    message.to_string()
                } else {
                    format!("{text}: {message}")
                },
                Target::Report {
                    page: page.clone(),
                    visual,
                    bookmark,
                },
            ),
            page,
        });
    }
    Ok(out)
}

fn page_wanted(r: &Report, rule_pages: &[String], key: Option<&str>) -> bool {
    if rule_pages.is_empty() {
        return true;
    }
    let Some(key) = key else { return false };
    match r.page(key) {
        Some(p) => any_glob(rule_pages, &p.display_name) || any_glob(rule_pages, &p.name),
        None => any_glob(rule_pages, key),
    }
}

pub(crate) fn report_cases(s: &Session, spec: &ReportSpec) -> Vec<Case> {
    let mut out = Vec::new();
    let Some(r) = s.report.as_ref() else {
        let why = match &s.report_error {
            Some(e) => format!("the report cannot be read: {e}"),
            None => "the open file has no report".into(),
        };
        out.push(Case::new("report", "report", "report", "report").error(why));
        return out;
    };
    let needs_validation = spec.validate.is_some()
        || spec
            .rules
            .iter()
            .any(|x| matches!(x.kind, ReportRuleKind::Validate(_)));
    let started = Instant::now();
    let issues = if needs_validation {
        Some(validation_issues(s))
    } else {
        None
    };
    let ms = elapsed(started);
    let from_issues = |case: Case, cats: &[&'static str], pages: &[String], level: Option<Level>| -> Case {
        match &issues {
            Some(Ok(list)) => {
                let findings: Vec<Finding> = list
                    .iter()
                    .filter(|i| cats.contains(&i.category))
                    .filter(|i| page_wanted(r, pages, i.page.as_deref()))
                    .map(|i| {
                        let mut f = i.finding.clone();
                        if let Some(l) = level {
                            // a rule's own severity applies to its errors; warnings stay warnings
                            if f.level == Level::Error {
                                f.level = l;
                            }
                        }
                        f
                    })
                    .collect();
                conclude_mixed(case, findings, "report problems")
            }
            Some(Err(e)) => case.error(e.clone()),
            None => case,
        }
    };
    if let Some(v) = &spec.validate {
        for cat in &v.categories {
            let c = Case::new(
                "report",
                format!("validate-{cat}"),
                format!("report.validate: {cat}"),
                "validate",
            );
            out.push(from_issues(c, &[*cat], &[], None));
        }
    }
    for rule in &spec.rules {
        let case = Case::new(
            "report",
            rule.id.clone(),
            rule.name.clone(),
            kind_name(&rule.kind),
        )
        .at(rule.severity);
        let c = match &rule.kind {
            ReportRuleKind::Validate(cats) => from_issues(case, cats, &rule.pages, Some(rule.severity)),
            _ => {
                let t = Instant::now();
                let mut c = case.conclude(report_rule(r, rule), "report problems");
                c.elapsed_ms = elapsed(t);
                c
            }
        };
        out.push(c);
    }
    if let Some(c) = out.first_mut() {
        c.elapsed_ms = c.elapsed_ms.saturating_add(ms);
    }
    out
}

fn kind_name(k: &ReportRuleKind) -> &'static str {
    match k {
        ReportRuleKind::Validate(c) if c.as_slice() == ["bookmarks"] => "bookmarks_valid",
        ReportRuleKind::Validate(_) => "fields_exist",
        ReportRuleKind::MaxVisualsPerPage { .. } => "max_visuals_per_page",
        ReportRuleKind::AltText { .. } => "alt_text",
        ReportRuleKind::PageSize { .. } => "page_size",
        ReportRuleKind::Theme { .. } => "theme",
        ReportRuleKind::NoHiddenPageNavigation => "no_hidden_page_navigation",
    }
}

fn page_target(p: &Page, v: Option<&Visual>) -> Target {
    Target::Report {
        page: Some(p.name.clone()),
        visual: v.map(|v| v.name.clone()),
        bookmark: None,
    }
}

fn is_hidden_page(p: &Page) -> bool {
    p.visibility.as_deref() == Some("HiddenInViewMode")
}

fn literal(prop: Option<&Value>) -> Option<String> {
    let s = prop?.get("expr")?.get("Literal")?.get("Value")?.as_str()?;
    Some(
        s.strip_prefix('\'')
            .and_then(|x| x.strip_suffix('\''))
            .map_or_else(|| s.to_string(), |x| x.replace("''", "'")),
    )
}

/// A visual's alt text (`general.altText`, a literal), when set and not blank.
pub(crate) fn alt_text(v: &Visual) -> Option<String> {
    for key in ["visualContainerObjects", "vcObjects", "objects"] {
        let Some(list) = v
            .visual_extra
            .get(key)
            .and_then(|o| o.get("general"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for e in list {
            if let Some(t) = literal(e.get("properties").and_then(|p| p.get("altText")))
                && !t.trim().is_empty()
            {
                return Some(t);
            }
        }
    }
    None
}

/// Literal colors (`solid.color` with a literal value) under a visual's formatting objects.
fn hardcoded_colors(v: &Value, depth: usize, out: &mut Vec<String>) {
    if depth > MAX_DEPTH || out.len() >= 20 {
        return;
    }
    match v {
        Value::Object(o) => {
            if let Some(c) = o.get("solid").and_then(|s| s.get("color"))
                && let Some(text) = literal(Some(c))
                && !out.contains(&text)
            {
                out.push(text);
            }
            for x in o.values() {
                hardcoded_colors(x, depth + 1, out);
            }
        }
        Value::Array(a) => {
            for x in a {
                hardcoded_colors(x, depth + 1, out);
            }
        }
        _ => {}
    }
}

fn theme_name<'a>(r: &'a Report, key: &str) -> Option<&'a str> {
    r.theme_collection
        .as_ref()?
        .get(key)?
        .get("name")?
        .as_str()
        .filter(|n| !n.is_empty())
}

fn report_rule(r: &Report, rule: &ReportRule) -> Vec<Finding> {
    let sev = rule.severity;
    let mut out = Vec::new();
    let pages = r
        .pages
        .iter()
        .filter(|p| page_wanted(r, &rule.pages, Some(&p.name)));
    match &rule.kind {
        ReportRuleKind::Validate(_) => {}
        ReportRuleKind::MaxVisualsPerPage { max, count_hidden } => {
            for p in pages {
                let n = p
                    .visuals
                    .iter()
                    .filter(|v| v.visual_type.is_some() && (*count_hidden || !v.is_hidden))
                    .count();
                if n > *max {
                    out.push(Finding::new(
                        sev,
                        format!("page '{}' has {n} visuals (at most {max})", p.display_name),
                        page_target(p, None),
                    ));
                }
            }
        }
        ReportRuleKind::AltText { exclude_types } => {
            for p in pages {
                for v in &p.visuals {
                    let Some(ty) = v.visual_type.as_deref() else {
                        continue;
                    };
                    if v.is_hidden || exclude_types.iter().any(|x| x.eq_ignore_ascii_case(ty)) {
                        continue;
                    }
                    if alt_text(v).is_none() {
                        out.push(Finding::new(
                            sev,
                            format!("page '{}', {ty} {}: no alt text", p.display_name, v.name),
                            page_target(p, Some(v)),
                        ));
                    }
                }
            }
        }
        ReportRuleKind::PageSize {
            allowed,
            include_tooltips,
        } => {
            for p in pages {
                let tooltip = p.extra.get("type").and_then(Value::as_str) == Some("Tooltip");
                if tooltip && !include_tooltips {
                    continue;
                }
                let fits = allowed
                    .iter()
                    .any(|(w, h)| (p.width - w).abs() < 0.5 && (p.height - h).abs() < 0.5);
                if !fits {
                    let want: Vec<String> = allowed.iter().map(|(w, h)| format!("{w}x{h}")).collect();
                    out.push(Finding::new(
                        sev,
                        format!(
                            "page '{}' is {}x{} (expected {})",
                            p.display_name,
                            p.width,
                            p.height,
                            want.join(" or ")
                        ),
                        page_target(p, None),
                    ));
                }
            }
        }
        ReportRuleKind::Theme {
            custom,
            base,
            forbid_hardcoded_colors,
        } => {
            let file = Target::ReportFile("definition/report.json".into());
            if let Some(want) = custom {
                match (theme_name(r, "customTheme"), want) {
                    (None, _) => out.push(Finding::new(sev, "the report has no custom theme", file.clone())),
                    (Some(got), Some(w)) if !got.eq_ignore_ascii_case(w) && !glob(w, got) => {
                        out.push(Finding::new(
                            sev,
                            format!("the custom theme is `{got}`, expected `{w}`"),
                            file.clone(),
                        ))
                    }
                    _ => {}
                }
            }
            if let Some(w) = base {
                match theme_name(r, "baseTheme") {
                    Some(got) if got.eq_ignore_ascii_case(w) || glob(w, got) => {}
                    got => out.push(Finding::new(
                        sev,
                        format!("the base theme is `{}`, expected `{w}`", got.unwrap_or("(none)")),
                        file.clone(),
                    )),
                }
            }
            if *forbid_hardcoded_colors {
                for p in pages {
                    for v in &p.visuals {
                        let mut colors = Vec::new();
                        for key in ["objects", "visualContainerObjects", "vcObjects"] {
                            if let Some(o) = v.visual_extra.get(key) {
                                hardcoded_colors(o, 0, &mut colors);
                            }
                        }
                        if !colors.is_empty() {
                            out.push(Finding::new(
                                sev,
                                format!(
                                    "page '{}', visual {}: hard-coded colors {} instead of theme colors",
                                    p.display_name,
                                    v.name,
                                    colors.join(", ")
                                ),
                                page_target(p, Some(v)),
                            ));
                        }
                    }
                }
            }
        }
        ReportRuleKind::NoHiddenPageNavigation => {
            for p in pages {
                for v in &p.visuals {
                    if v.is_hidden {
                        continue;
                    }
                    let Some(link) = crate::interact::visual_link(v) else {
                        continue;
                    };
                    if link.kind != "PageNavigation" {
                        continue;
                    }
                    let Some(target) = link.page.as_deref().and_then(|t| r.page(t)) else {
                        out.push(Finding::new(
                            sev,
                            format!(
                                "page '{}', visual {}: navigates to a page that does not exist ({})",
                                p.display_name,
                                v.name,
                                link.page.as_deref().unwrap_or("none")
                            ),
                            page_target(p, Some(v)),
                        ));
                        continue;
                    };
                    if is_hidden_page(target) {
                        out.push(Finding::new(
                            sev,
                            format!(
                                "page '{}', visual {}: navigates to the hidden page '{}'",
                                p.display_name, v.name, target.display_name
                            ),
                            page_target(p, Some(v)),
                        ));
                    }
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// usage
// ---------------------------------------------------------------------------------------------

/// The command that lists unused model objects (`usage.rs`).
const UNUSED_COMMAND: &str = "model.unused";

pub(crate) fn usage_cases(s: &mut Session, spec: &UsageSpec) -> (Vec<Case>, Option<String>) {
    let Some(level) = spec.no_unused_measures else {
        return (Vec::new(), None);
    };
    let case = Case::new("usage", "no-unused-measures", "No unused measures", "usage").at(level);
    let Some(cmd) = crate::find_command(UNUSED_COMMAND) else {
        return (
            vec![case.skip(format!(
                "skipped: needs the `{UNUSED_COMMAND}` command, which this build does not have"
            ))],
            Some(format!(
                "usage rules need `{UNUSED_COMMAND}`; they are skipped by this build"
            )),
        );
    };
    let started = Instant::now();
    let v = match (cmd.run)(s, &json!({})) {
        Ok(v) => v,
        Err(e) => return (vec![case.error(e)], None),
    };
    let mut c = case.conclude(unused_measures(&v, level), "unused measures");
    c.elapsed_ms = elapsed(started);
    (vec![c], None)
}

/// The unused measures of a `model.unused` result (`{unused: {measures: [{table, name, why}],
/// columns, tables, expressions}, kept, warnings}`, `usage.rs`): only measures are judged.
pub(crate) fn unused_measures(v: &Value, level: Level) -> Vec<Finding> {
    v.pointer("/unused/measures")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|o| {
            let name = str_of(o, "name")?;
            let table = str_of(o, "table");
            let why = str_of(o, "why").map_or_else(String::new, |w| format!(" ({w})"));
            Some(Finding::new(
                level,
                format!("measure {} is not used{why}", object_label(table, name)),
                Target::Model {
                    kind: "measure".into(),
                    table: table.map(str::to_string),
                    name: name.to_string(),
                },
            ))
        })
        .collect()
}
