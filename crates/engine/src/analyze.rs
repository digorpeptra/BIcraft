//! `model.analyze`: best-practice rules over the open model (crate `bicraft-bpa`).
//!
//! The default rule set (Microsoft's `BPARules.json`, embedded) runs unless `default_rules` is
//! false; `rules` adds rule files (paths) or inline rules in the same JSON format. The report
//! rules (`crate::report_lint`: references to the model that no longer resolve) run too unless
//! `report_rules` is false.

use bicraft_bpa::{Options, Rule, RuleSet};
use serde_json::{Value, json};

use crate::Session;
use crate::commands::model;
use crate::params::{allow, opt_bool};

/// Largest rule file read, in bytes.
const MAX_RULE_FILE: u64 = 16 * 1024 * 1024;

fn read_file(path: &str) -> Result<RuleSet, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("rules: cannot read {path}: {e}"))?;
    if meta.len() > MAX_RULE_FILE {
        return Err(format!("rules: {path} is larger than {MAX_RULE_FILE} bytes"));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("rules: cannot read {path}: {e}"))?;
    bicraft_bpa::parse_rules(&text, path).map_err(|e| format!("rules: {path}: {e}"))
}

/// The rule sets named by the `rules` parameter.
fn rule_sets(v: Option<&Value>) -> Result<Vec<RuleSet>, String> {
    let inline = |v: &Value| bicraft_bpa::parse_rules_value(v, "inline").map_err(|e| format!("rules: {e}"));
    match v {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(p)) => Ok(vec![read_file(p)?]),
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => {
            items.iter().filter_map(Value::as_str).map(read_file).collect()
        }
        Some(v @ (Value::Array(_) | Value::Object(_))) => Ok(vec![inline(v)?]),
        Some(_) => Err("parameter `rules` must be a path, a list of paths, or inline rules".into()),
    }
}

pub(crate) fn model_analyze(s: &Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "rules",
            "default_rules",
            "rule_ids",
            "severity_min",
            "max_findings",
            "report_rules",
        ],
    )?;
    let m = model(s)?;
    let mut sets = Vec::new();
    if opt_bool(p, "default_rules")?.unwrap_or(true) {
        sets.push(bicraft_bpa::default_rules());
    }
    sets.extend(rule_sets(p.get("rules"))?);
    let rule_ids = match p.get("rule_ids") {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) => Some(
            a.iter()
                .map(|x| {
                    x.as_str()
                        .map(str::to_string)
                        .ok_or("parameter `rule_ids` must be a list of strings")
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Some(_) => return Err("parameter `rule_ids` must be a list of strings".into()),
    };
    let severity_min = match p.get("severity_min") {
        None | Some(Value::Null) => 1,
        Some(Value::Number(n)) => n
            .as_u64()
            .filter(|x| (1..=5).contains(x))
            .map(|x| x as u8)
            .ok_or("parameter `severity_min` must be 1 to 5")?,
        Some(Value::String(t)) => bicraft_bpa::severity_from_text(t)
            .ok_or("parameter `severity_min` must be 1-5, info, warning or error")?,
        Some(_) => return Err("parameter `severity_min` must be 1-5, info, warning or error".into()),
    };
    let max_findings = match p.get("max_findings") {
        None | Some(Value::Null) => bicraft_bpa::DEFAULT_MAX_FINDINGS,
        Some(v) => v
            .as_u64()
            .and_then(|x| usize::try_from(x).ok())
            .ok_or("parameter `max_findings` must be a non-negative integer")?,
    };
    let mut invalid = Vec::new();
    let mut rules: Vec<Rule> = Vec::new();
    for set in sets {
        let source = set.rules.first().map(|r| r.source.clone()).unwrap_or_default();
        for (i, why) in set.invalid {
            invalid.push(json!({"source": source, "index": i, "reason": why}));
        }
        rules.extend(set.rules);
    }
    let opts = Options {
        rule_ids,
        severity_min,
        max_findings,
    };
    let report_rules = opt_bool(p, "report_rules")?.unwrap_or(true);
    let wanted = |id: &str| {
        opts.rule_ids
            .as_ref()
            .is_none_or(|ids| ids.iter().any(|x| x.eq_ignore_ascii_case(id)))
    };
    let mut a = bicraft_bpa::analyze(m, &rules, &opts);
    let mut outcomes: Vec<Value> = a
        .rules
        .iter()
        .filter_map(|r| serde_json::to_value(r).ok())
        .collect();
    let mut findings: Vec<Value> = a
        .findings
        .iter()
        .filter_map(|f| serde_json::to_value(f).ok())
        .collect();
    // the report rules run with a report open (a model alone has no report references)
    if report_rules && let Some(report) = s.report.as_ref() {
        let found = crate::report_lint::findings(m, report);
        for r in crate::report_lint::RULES {
            if !wanted(r.id) || r.severity < opts.severity_min.clamp(1, 5) {
                continue;
            }
            let mine: Vec<&Value> = found
                .iter()
                .filter(|f| f.get("rule_id").and_then(Value::as_str) == Some(r.id))
                .collect();
            outcomes.push(json!({
                "rule_id": r.id,
                "rule_name": r.name,
                "category": crate::report_lint::CATEGORY,
                "severity": bicraft_bpa::severity_name(r.severity),
                "source": crate::report_lint::SOURCE,
                "status": "evaluated",
                "findings": mine.len(),
            }));
            for f in mine {
                if findings.len() < opts.max_findings {
                    findings.push(f.clone());
                } else {
                    a.findings_omitted = a.findings_omitted.saturating_add(1);
                }
            }
        }
    }
    let count = |status: &str| {
        outcomes
            .iter()
            .filter(|r| r.get("status").and_then(Value::as_str) == Some(status))
            .count()
    };
    let by_severity = |sev: &str| {
        findings
            .iter()
            .filter(|f| f.get("severity").and_then(Value::as_str) == Some(sev))
            .count()
    };
    Ok(json!({
        "summary": {
            "rules": outcomes.len(),
            "evaluated": count("evaluated"),
            "partial": count("partial"),
            "not_evaluated": count("not_evaluated"),
            "skipped": count("skipped"),
            "findings": findings.len() + a.findings_omitted,
            "errors": by_severity("error"),
            "warnings": by_severity("warning"),
            "info": by_severity("info"),
        },
        "findings": findings,
        "findings_omitted": a.findings_omitted,
        "rules": outcomes,
        "invalid_rules": invalid,
    }))
}
