//! `bicraft-cli test [PATH] [--tests FILE] [--format text|json|junit|sarif] [--output FILE]
//! [--fail-on error|warning]`: the engine's `project.test` (CI quality gates, docs/ci-tests.md).
//!
//! Exit codes (stable): 0 every gate passed; 1 a failed or errored test reaches `--fail-on`;
//! 2 usage error, unreadable or invalid test file, or a project that cannot be opened.

use bicraft_engine::Session;
use serde_json::{Value, json};

use super::{Failure, INVALID, OK, USAGE_ERROR, usage, value};

pub(crate) const HELP: &str = "usage: bicraft-cli test [PATH] [--tests FILE] [--format text|json|junit|sarif]
                        [--output FILE] [--fail-on error|warning]
  PATH      a .pbip, project folder, .pbix, .pbit or model.bim (default: the current folder)
  --tests   the test file (default: bicraft.tests.yaml, .yml or .json next to the project)
  --format  text (default), json, junit (JUnit XML) or sarif (SARIF 2.1.0)
  --output  write the report to FILE; the text summary still goes to stdout
  --fail-on lowest severity of a failed test that fails the run (default error)
Paths in reports are relative to the current folder. Exit codes: 0 passed, 1 a test failed,
2 usage error, invalid test file or unreadable project.";

pub(crate) fn test(args: &[String]) -> Result<u8, Failure> {
    let mut path: Option<&str> = None;
    let mut tests = None;
    let mut format = "text";
    let mut output = None;
    let mut fail_on = "error";
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--tests" => tests = Some(value(&mut it, a)?),
            "--format" => format = value(&mut it, a)?,
            "--output" | "-o" => output = Some(value(&mut it, a)?),
            "--fail-on" => fail_on = value(&mut it, a)?,
            "--help" | "-h" => {
                outln!("{HELP}");
                return Ok(OK);
            }
            other if other.starts_with('-') && other != "-" => {
                return Err(usage(format!("test: unknown option `{other}`\n\n{HELP}")));
            }
            other => {
                if path.replace(other).is_some() {
                    return Err(usage("test takes one PATH"));
                }
            }
        }
    }
    if !matches!(format, "text" | "json" | "junit" | "sarif") {
        return Err(usage(format!(
            "--format: `{format}` is not text, json, junit or sarif"
        )));
    }
    if !matches!(fail_on, "error" | "warning") {
        return Err(usage(format!("--fail-on: `{fail_on}` is not error or warning")));
    }
    let cwd = std::env::current_dir().map_err(|e| (USAGE_ERROR, format!("current folder: {e}")))?;
    // the report in `format`, plus the text summary for the CI log when the report goes to a file
    let mut formats = vec![format];
    if output.is_some() && format != "text" {
        formats.push("text");
    }
    let mut params = json!({
        "path": path.unwrap_or("."),
        "fail_on": fail_on,
        "base_dir": cwd.display().to_string(),
        "format": formats,
    });
    if let (Some(t), Some(o)) = (tests, params.as_object_mut()) {
        o.insert("tests".into(), json!(t));
    }
    let mut session = Session::new();
    let mut v = session
        .execute("project.test", &params)
        .map_err(|e| (USAGE_ERROR, e))?;
    let mut take = |f: &str| -> String {
        let o = v.as_object_mut();
        let found = match o {
            Some(o) if formats.len() == 1 => {
                o.remove("format");
                o.remove("output")
            }
            Some(o) => o
                .get_mut("outputs")
                .and_then(Value::as_object_mut)
                .and_then(|m| m.remove(f)),
            None => None,
        };
        match found {
            Some(Value::String(s)) => s,
            _ => String::new(),
        }
    };
    let report = if format == "json" {
        String::new()
    } else {
        take(format)
    };
    let summary = if formats.len() > 1 {
        take("text")
    } else {
        String::new()
    };
    if let Some(o) = v.as_object_mut() {
        o.remove("outputs");
    }
    let report = if format == "json" {
        let mut s = serde_json::to_string_pretty(&v).unwrap_or_default();
        s.push('\n');
        s
    } else {
        report
    };
    match output {
        Some(file) => {
            std::fs::write(file, report.as_bytes()).map_err(|e| (USAGE_ERROR, format!("{file}: {e}")))?;
            if !summary.is_empty() {
                outln!("{}", summary.trim_end());
            } else if format == "text" {
                outln!("{}", report.trim_end());
            }
        }
        None => outln!("{}", report.trim_end()),
    }
    Ok(if v.get("passed") == Some(&Value::Bool(true)) {
        OK
    } else {
        INVALID
    })
}
