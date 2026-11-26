//! L4: best-practice analysis of a semantic model.
//!
//! Rules come in the "Best Practice Analyzer" JSON format: an array of rules whose `Expression`
//! is a Dynamic LINQ predicate over Tabular-Object-Model-like objects (tables, columns, measures,
//! relationships, ...). [`linq`] parses the predicate and [`eval`] interprets it against a view of
//! the [`SemanticModel`] ([`objects`]); references between DAX objects come from the DAX parser.
//!
//! The default rule set is Microsoft's `BPARules.json` (MIT, vendored unmodified under
//! `rules/`, see `rules/SOURCE.txt`). Callers add their own rule files in the same format.
//!
//! Constructs or properties the interpreter does not know make a rule "not evaluated" for the
//! objects concerned (with the reason), never silently "no violation". Evaluation is bounded:
//! parse depth, steps per object and per rule. See `docs/formats/bpa.md`.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod eval;
mod linq;
mod objects;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_constructs;
#[cfg(test)]
mod tests_default;
mod tokens;

use bicraft_model::SemanticModel;
use serde::Serialize;
use serde_json::Value;

/// Microsoft's best-practice rules (`BestPracticeRules/BPARules.json` of
/// microsoft/Analysis-Services, MIT), unmodified.
pub const DEFAULT_RULES_JSON: &str = include_str!("../rules/BPARules.json");

/// The source name of [`default_rules`].
pub const DEFAULT_SOURCE: &str = "default";

/// Most rules read from one rule file.
pub const MAX_RULES: usize = 10_000;
/// Longest rule expression accepted, in bytes.
pub const MAX_EXPRESSION_BYTES: usize = 64 * 1024;
/// Default cap on the findings returned by one analysis.
pub const DEFAULT_MAX_FINDINGS: usize = 10_000;

/// One rule of a BPA rule file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub category: String,
    pub description: String,
    /// 1 (information) to 5, as in the rule file (most files use 1-3).
    pub severity: u8,
    /// Comma-separated object kinds (`Measure, CalculatedColumn`).
    pub scope: String,
    pub expression: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility_level: Option<u32>,
    /// Where the rule comes from: [`DEFAULT_SOURCE`] or the rule file's path.
    pub source: String,
}

/// A rule file as read: the usable rules and the entries that could not be read.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
    /// `(position in the file, reason)` of entries that are not usable rules.
    pub invalid: Vec<(usize, String)>,
}

/// The default rule set ([`DEFAULT_RULES_JSON`]).
pub fn default_rules() -> RuleSet {
    parse_rules(DEFAULT_RULES_JSON, DEFAULT_SOURCE).unwrap_or_default()
}

/// Reads a BPA rule file: a JSON array of rule objects (`{"rules": [...]}` is accepted too).
/// Property names are matched case-insensitively. `source` names the file in the results.
pub fn parse_rules(text: &str, source: &str) -> Result<RuleSet, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let v: Value = serde_json::from_str(text).map_err(|e| format!("the rule file is not JSON: {e}"))?;
    parse_rules_value(&v, source)
}

/// [`parse_rules`] on parsed JSON.
pub fn parse_rules_value(v: &Value, source: &str) -> Result<RuleSet, String> {
    let list = match v {
        Value::Array(a) => a,
        Value::Object(o) => match o.iter().find(|(k, _)| k.eq_ignore_ascii_case("rules")) {
            Some((_, Value::Array(a))) => a,
            _ => return Err("a rule file is a JSON array of rules".into()),
        },
        _ => return Err("a rule file is a JSON array of rules".into()),
    };
    if list.len() > MAX_RULES {
        return Err(format!(
            "the rule file has {} rules; at most {MAX_RULES} are read",
            list.len()
        ));
    }
    let mut set = RuleSet::default();
    for (i, item) in list.iter().enumerate() {
        match read_rule(item, source) {
            Ok(r) => set.rules.push(r),
            Err(e) => set.invalid.push((i, e)),
        }
    }
    Ok(set)
}

fn field<'a>(o: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a Value> {
    o.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

fn text_field(o: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    match field(o, key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn read_rule(v: &Value, source: &str) -> Result<Rule, String> {
    let Value::Object(o) = v else {
        return Err("a rule is a JSON object".into());
    };
    let id = text_field(o, "ID")
        .filter(|s| !s.trim().is_empty())
        .ok_or("the rule has no ID")?;
    let expression = text_field(o, "Expression").unwrap_or_default();
    if expression.len() > MAX_EXPRESSION_BYTES {
        return Err(format!(
            "rule {id}: the expression is longer than {MAX_EXPRESSION_BYTES} bytes"
        ));
    }
    let severity = match field(o, "Severity") {
        Some(Value::Number(n)) => n.as_u64().map_or(1, |x| x.clamp(1, 5) as u8),
        Some(Value::String(s)) => severity_from_text(s).unwrap_or(1),
        _ => 1,
    };
    let compatibility_level = match field(o, "CompatibilityLevel") {
        Some(Value::Number(n)) => n.as_u64().and_then(|x| u32::try_from(x).ok()),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    };
    Ok(Rule {
        name: text_field(o, "Name").unwrap_or_else(|| id.clone()),
        category: text_field(o, "Category").unwrap_or_default(),
        description: text_field(o, "Description").unwrap_or_default(),
        severity,
        scope: text_field(o, "Scope").unwrap_or_default(),
        expression,
        fix_expression: text_field(o, "FixExpression").filter(|s| !s.trim().is_empty()),
        compatibility_level,
        source: source.to_string(),
        id,
    })
}

/// Parses a severity: a number 1-5, or `info`/`information`/`low` (1), `warning`/`medium` (2),
/// `error`/`high` (3).
pub fn severity_from_text(s: &str) -> Option<u8> {
    let s = s.trim().to_ascii_lowercase();
    if let Ok(n) = s.parse::<u8>() {
        return (1..=5).contains(&n).then_some(n);
    }
    match s.as_str() {
        "info" | "information" | "low" => Some(1),
        "warning" | "medium" => Some(2),
        "error" | "high" => Some(3),
        _ => None,
    }
}

/// The name of a severity level: 1 `info`, 2 `warning`, 3 and above `error`.
pub fn severity_name(level: u8) -> &'static str {
    match level {
        0 | 1 => "info",
        2 => "warning",
        _ => "error",
    }
}

/// What to run and how much to return.
#[derive(Debug, Clone)]
pub struct Options {
    /// Only these rule ids (case-insensitive); `None`: every rule.
    pub rule_ids: Option<Vec<String>>,
    /// Lowest severity reported (1-5).
    pub severity_min: u8,
    /// Most findings returned; the rest are counted in [`Analysis::findings_omitted`].
    pub max_findings: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            rule_ids: None,
            severity_min: 1,
            max_findings: DEFAULT_MAX_FINDINGS,
        }
    }
}

/// One rule violation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    pub rule_id: String,
    pub rule_name: String,
    pub category: String,
    /// `info`, `warning` or `error`.
    pub severity: &'static str,
    pub severity_level: u8,
    /// The object's kind (`Measure`, `Calculated Column`, `Relationship`, ...).
    pub object_type: String,
    /// The object, in DAX syntax where there is one (`'Sales'[Amount]`, `[Total]`).
    pub object: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
}

/// How one rule went.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RuleOutcome {
    pub rule_id: String,
    pub rule_name: String,
    pub category: String,
    pub severity: &'static str,
    /// [`DEFAULT_SOURCE`] or the rule file it came from.
    pub source: String,
    /// `evaluated`; `partial` (some objects could not be evaluated: findings on them may be
    /// missing); `not_evaluated` (the expression or its scope is not supported); `skipped` (the
    /// model's compatibility level is below the rule's).
    pub status: &'static str,
    /// Objects the rule was evaluated on.
    pub objects: usize,
    pub findings: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The rule's `FixExpression`, reported as is (never applied).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix_expression: Option<String>,
}

/// The result of [`analyze`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct Analysis {
    pub rules: Vec<RuleOutcome>,
    pub findings: Vec<Finding>,
    /// Findings left out by `max_findings`.
    pub findings_omitted: usize,
}

impl Analysis {
    fn push(&mut self, f: Finding, max: usize) {
        if self.findings.len() < max {
            self.findings.push(f);
        } else {
            self.findings_omitted = self.findings_omitted.saturating_add(1);
        }
    }
}

/// Runs `rules` over `model`.
pub fn analyze(model: &SemanticModel, rules: &[Rule], opts: &Options) -> Analysis {
    let view = objects::ModelView::new(model);
    let mut out = Analysis::default();
    let min = opts.severity_min.clamp(1, 5);
    let mut interp = eval::Interp::new(&view);
    for rule in rules {
        let wanted = opts
            .rule_ids
            .as_ref()
            .is_none_or(|ids| ids.iter().any(|x| x.eq_ignore_ascii_case(&rule.id)));
        if rule.severity < min || !wanted {
            continue;
        }
        eval::run_rule(&mut interp, rule, opts.max_findings, &mut out);
    }
    out
}
