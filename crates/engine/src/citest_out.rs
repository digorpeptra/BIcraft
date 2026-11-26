//! `project.test` outputs: JSON (the command's result), human text, JUnit XML and SARIF 2.1.0.
//!
//! All four list the cases in the same order (suites in run order, cases in file order) and hold
//! no clock time or absolute path of their own, so two runs over the same files give the same
//! bytes except for the durations (`elapsed_ms`, JUnit `time`).
//!
//! - **JUnit**: one `<testsuite>` per suite. A case that fails the gate (`fail_on`) is a
//!   `<failure>` (or `<error>` when it could not run); a failed case below the gate passes with
//!   its findings in `<system-out>` prefixed `WARNING`, so CI dashboards show what gates the run.
//! - **SARIF**: one result per finding of a failed or errored case (level `error`, `warning` or
//!   `note`), rule id `<suite>/<case id>`, located in the project's files where known.

use std::fmt::Write as _;

use serde_json::{Value, json};

use crate::citest::{Case, Finding, Level, Run, Status};

/// The SARIF schema URL written into reports.
pub(crate) const SARIF_SCHEMA: &str = "https://json.schemastore.org/sarif-2.1.0.json";

/// The suites in their fixed order, then any other in run order.
const SUITES: &[&str] = &["dax", "model", "bpa", "report", "usage"];

fn suites(run: &Run) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for s in SUITES {
        if run.cases.iter().any(|c| c.suite == *s) {
            out.push(s);
        }
    }
    for c in &run.cases {
        if !out.contains(&c.suite) {
            out.push(c.suite);
        }
    }
    out
}

fn rule_id(c: &Case) -> String {
    format!("{}/{}", c.suite, c.id)
}

fn finding_json(f: &Finding) -> Value {
    let mut o = json!({"level": f.level.name(), "message": f.message});
    if let (Some(map), Some((path, line))) = (o.as_object_mut(), &f.location) {
        map.insert("location".into(), json!({"path": path, "line": line}));
    }
    o
}

pub(crate) fn json(run: &Run) -> Value {
    let suites: Vec<Value> = suites(run)
        .into_iter()
        .map(|name| {
            let cases: Vec<Value> = run
                .cases
                .iter()
                .filter(|c| c.suite == name)
                .map(|c| {
                    let mut o = json!({
                        "id": rule_id(c),
                        "name": c.name,
                        "kind": c.kind,
                        "status": c.status.name(),
                        "severity": c.level.name(),
                        "gates": c.gates(run.fail_on),
                        "message": c.message,
                        "findings": c.findings.iter().map(finding_json).collect::<Vec<_>>(),
                        "elapsed_ms": c.elapsed_ms,
                    });
                    if let Some(map) = o.as_object_mut() {
                        if c.omitted > 0 {
                            map.insert("findings_omitted".into(), json!(c.omitted));
                        }
                        if let Some(d) = &c.details {
                            map.insert("details".into(), d.clone());
                        }
                    }
                    o
                })
                .collect();
            json!({"name": name, "tests": cases})
        })
        .collect();
    json!({
        "passed": run.passed(),
        "exit_code": if run.passed() { 0 } else { 1 },
        "fail_on": run.fail_on.name(),
        "name": run.spec_name,
        "project": run.project,
        "tests_file": run.tests_file,
        "summary": {
            "tests": run.cases.len(),
            "passed": run.count(Status::Passed),
            "failed": run.count(Status::Failed),
            "errors": run.count(Status::Error),
            "skipped": run.count(Status::Skipped),
            "warnings": run.warnings(),
        },
        "notes": run.notes,
        "suites": suites,
    })
}

fn mark(c: &Case, fail_on: Level) -> &'static str {
    match c.status {
        Status::Passed => "PASS",
        Status::Skipped => "SKIP",
        _ if !c.gates(fail_on) => "WARN",
        Status::Failed => "FAIL",
        Status::Error => "ERROR",
    }
}

fn located(f: &Finding) -> String {
    match &f.location {
        Some((p, Some(l))) => format!(" ({p}:{l})"),
        Some((p, None)) => format!(" ({p})"),
        None => String::new(),
    }
}

/// Findings listed per case in the text report.
const TEXT_FINDINGS: usize = 20;

pub(crate) fn text(run: &Run) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "bicraft test: {}{}",
        run.project.as_deref().unwrap_or("(open file)"),
        run.tests_file
            .as_deref()
            .map_or_else(|| " with inline tests".to_string(), |t| format!(" with {t}"))
    );
    if let Some(n) = &run.spec_name {
        let _ = writeln!(out, "{n}");
    }
    for suite in suites(run) {
        let _ = writeln!(out, "\n{suite}");
        for c in run.cases.iter().filter(|c| c.suite == suite) {
            let _ = writeln!(out, "  {:<5} {}  {}", mark(c, run.fail_on), c.id, c.name);
            if c.status != Status::Passed && !c.message.is_empty() {
                let _ = writeln!(out, "        {}", c.message);
            }
            if matches!(c.status, Status::Failed | Status::Error) && c.findings.len() > 1 {
                for f in c.findings.iter().take(TEXT_FINDINGS) {
                    let _ = writeln!(out, "        - {}{}", f.message, located(f));
                }
                let more = c.findings.len().saturating_sub(TEXT_FINDINGS) + c.omitted;
                if more > 0 {
                    let _ = writeln!(out, "        ... {more} more");
                }
            } else if let Some(f) = c.findings.first().filter(|_| c.status != Status::Passed) {
                let loc = located(f);
                if !loc.is_empty() {
                    let _ = writeln!(out, "       {loc}");
                }
            }
        }
    }
    for n in &run.notes {
        let _ = writeln!(out, "\nnote: {n}");
    }
    let _ = writeln!(
        out,
        "\n{} tests: {} passed, {} failed, {} errors, {} skipped, {} warnings: {} (fail on {})",
        run.cases.len(),
        run.count(Status::Passed),
        run.count(Status::Failed),
        run.count(Status::Error),
        run.count(Status::Skipped),
        run.warnings(),
        if run.passed() { "PASSED" } else { "FAILED" },
        run.fail_on.name()
    );
    out
}

/// XML text and attribute escaping; characters XML 1.0 cannot hold are dropped.
fn xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 || c == '\u{fffe}' || c == '\u{ffff}' => {}
            c => out.push(c),
        }
    }
    out
}

fn seconds(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

fn finding_lines(c: &Case) -> String {
    let mut s = String::new();
    for f in &c.findings {
        let _ = writeln!(s, "[{}] {}{}", f.level.name(), f.message, located(f));
    }
    if c.omitted > 0 {
        let _ = writeln!(s, "... {} more", c.omitted);
    }
    s
}

pub(crate) fn junit(run: &Run) -> String {
    let count = |cases: &[&Case], f: &dyn Fn(&Case) -> bool| cases.iter().filter(|c| f(c)).count();
    let all: Vec<&Case> = run.cases.iter().collect();
    let fails = |c: &Case| c.gates(run.fail_on) && c.status == Status::Failed;
    let errors = |c: &Case| c.gates(run.fail_on) && c.status == Status::Error;
    let skipped = |c: &Case| c.status == Status::Skipped;
    let total_ms: u64 = run
        .cases
        .iter()
        .map(|c| c.elapsed_ms)
        .fold(0, u64::saturating_add);
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        out,
        "<testsuites name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"{}\" skipped=\"{}\" time=\"{}\">",
        xml(run.spec_name.as_deref().unwrap_or("bicraft test")),
        all.len(),
        count(&all, &fails),
        count(&all, &errors),
        count(&all, &skipped),
        seconds(total_ms)
    );
    for suite in suites(run) {
        let cases: Vec<&Case> = run.cases.iter().filter(|c| c.suite == suite).collect();
        let ms: u64 = cases.iter().map(|c| c.elapsed_ms).fold(0, u64::saturating_add);
        let _ = writeln!(
            out,
            "  <testsuite name=\"{suite}\" tests=\"{}\" failures=\"{}\" errors=\"{}\" skipped=\"{}\" time=\"{}\">",
            cases.len(),
            count(&cases, &fails),
            count(&cases, &errors),
            count(&cases, &skipped),
            seconds(ms)
        );
        for c in cases {
            let _ = write!(
                out,
                "    <testcase name=\"{}\" classname=\"bicraft.{suite}\" time=\"{}\"",
                xml(&format!("{}: {}", c.id, c.name)),
                seconds(c.elapsed_ms)
            );
            let body = match c.status {
                Status::Passed => None,
                Status::Skipped => Some(format!("      <skipped message=\"{}\"/>\n", xml(&c.message))),
                _ if !c.gates(run.fail_on) => Some(format!(
                    "      <system-out>{}</system-out>\n",
                    xml(&format!(
                        "WARNING ({}): {}\n{}",
                        c.level.name(),
                        c.message,
                        finding_lines(c)
                    ))
                )),
                status => {
                    let tag = if status == Status::Error {
                        "error"
                    } else {
                        "failure"
                    };
                    Some(format!(
                        "      <{tag} message=\"{}\" type=\"{}\">{}</{tag}>\n",
                        xml(&c.message),
                        c.level.name(),
                        xml(&finding_lines(c))
                    ))
                }
            };
            match body {
                None => out.push_str("/>\n"),
                Some(b) => {
                    out.push_str(">\n");
                    out.push_str(&b);
                    out.push_str("    </testcase>\n");
                }
            }
        }
        out.push_str("  </testsuite>\n");
    }
    out.push_str("</testsuites>\n");
    out
}

fn sarif_level(l: Level) -> &'static str {
    match l {
        Level::Error => "error",
        Level::Warning => "warning",
        Level::Info => "note",
    }
}

pub(crate) fn sarif(run: &Run) -> String {
    let reported: Vec<&Case> = run
        .cases
        .iter()
        .filter(|c| matches!(c.status, Status::Failed | Status::Error))
        .collect();
    let rules: Vec<Value> = reported
        .iter()
        .map(|c| {
            json!({
                "id": rule_id(c),
                "name": c.name,
                "shortDescription": {"text": c.name},
                "defaultConfiguration": {"level": sarif_level(c.level)},
                "properties": {"suite": c.suite, "kind": c.kind},
            })
        })
        .collect();
    let mut results = Vec::new();
    for (i, c) in reported.iter().enumerate() {
        let listed: Vec<Finding> = if c.findings.is_empty() {
            vec![Finding::new(
                c.level,
                c.message.clone(),
                crate::citest::Target::None,
            )]
        } else {
            c.findings.clone()
        };
        for f in listed {
            let mut r = json!({
                "ruleId": rule_id(c),
                "ruleIndex": i,
                "level": sarif_level(f.level),
                "message": {"text": f.message},
            });
            if let (Some(map), Some((path, line))) = (r.as_object_mut(), &f.location) {
                let mut physical = json!({"artifactLocation": {"uri": uri(path)}});
                if let (Some(p), Some(line)) = (physical.as_object_mut(), line) {
                    p.insert("region".into(), json!({"startLine": line}));
                }
                map.insert("locations".into(), json!([{"physicalLocation": physical}]));
            }
            results.push(r);
        }
    }
    let doc = json!({
        "$schema": SARIF_SCHEMA,
        "version": "2.1.0",
        "runs": [{
            "tool": {"driver": {
                "name": "bicraft",
                "semanticVersion": env!("CARGO_PKG_VERSION"),
                "rules": rules,
            }},
            "results": results,
            "properties": {
                "passed": run.passed(),
                "failOn": run.fail_on.name(),
                "tests": run.cases.len(),
            },
        }],
    });
    let mut s = serde_json::to_string_pretty(&doc).unwrap_or_default();
    s.push('\n');
    s
}

/// A relative path as a URI reference: `/`-separated, with characters URIs reserve escaped.
fn uri(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let b = path.as_bytes();
    // an absolute path (outside the base folder, local mode only): a file URI
    if b.first() == Some(&b'/') {
        out.push_str("file://");
    } else if b.get(1) == Some(&b':') {
        out.push_str("file:///");
    }
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(b as char)
            }
            b'\\' => out.push('/'),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}
