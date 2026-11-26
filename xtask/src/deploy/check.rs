//! `cargo xtask deploy check`: the static checks of `deploy/` (also a unit test, so `cargo xtask
//! ci` runs them on Windows and Linux without Docker, Helm or kubectl).
//!
//! - every YAML file parses (the chart's templates are Go templates: their braces must balance);
//! - every `BICRAFT_*` variable named in `deploy/` or `docs/kubernetes.md` is read by the code, and
//!   every `bicraft serve` flag used by the manifests, the chart or the image exists, so the
//!   deployment cannot drift from the binary;
//!   every `.Values.x.y` a template uses exists in `values.yaml`;
//! - the security invariants: non-root fixed UID, read-only root file system, no privilege
//!   escalation, all capabilities dropped, seccomp RuntimeDefault, no memory-backed volume, one
//!   replica with the Recreate strategy, probes on /healthz and /readyz;
//! - the sizing rules: pod memory limit = BICRAFT_MEMORY_LIMIT + 64 MiB, BICRAFT_SCAN_THREADS =
//!   CPU limit, terminationGracePeriodSeconds above --shutdown-grace, TMPDIR and
//!   BICRAFT_SPILL_DIR on a disk-backed emptyDir with a size limit;
//! - the image: pinned bases (tag and digest), an empty final base, a numeric non-root USER, the
//!   `bicraft serve` entry point, a `--locked` build;
//! - the metrics listener: its own port, declared as the `metrics` container port, and `POD_IP`
//!   from the downward API when `--metrics-listen` binds `$(POD_IP)`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use yaml_rust2::{Yaml, YamlLoader};

/// The base margin of a pod above `BICRAFT_MEMORY_LIMIT` (docs/beyond-ram.md, lot 5).
pub const POD_MARGIN_MIB: u64 = 64;

/// What a successful check covered.
#[derive(Debug, Default)]
pub struct Summary {
    pub yaml_files: usize,
    pub yaml_docs: usize,
    pub templates: usize,
    pub env_vars: BTreeSet<String>,
    pub flags: BTreeSet<String>,
    pub values_refs: usize,
}

pub fn run(root: &Path) -> Result<Summary, Vec<String>> {
    let mut errors = Vec::new();
    let mut s = Summary::default();
    let deploy = root.join("deploy");
    let files = walk(&deploy);
    if files.is_empty() {
        return Err(vec![format!("{}: no files", deploy.display())]);
    }
    let known_env = known_env_vars(root);
    let known_flags = known_serve_flags(root);
    if known_env.is_empty() || known_flags.is_empty() {
        errors.push("could not read the code's variables or `serve` flags".into());
    }

    let chart = deploy.join("helm").join("bicraft");
    let templates_dir = chart.join("templates");
    let ctl_chart = deploy.join("helm").join("bicraft-controller");
    let ctl_templates = ctl_chart.join("templates");
    let mut ctl_flags = known_controller_flags(root);
    ctl_flags.extend(known_flags.iter().cloned());
    let mut ctl_values = Yaml::Null;
    let mut values = Yaml::Null;
    let mut texts: Vec<(PathBuf, String)> = Vec::new();

    for f in &files {
        let rel = rel(root, f);
        let Ok(text) = std::fs::read_to_string(f) else {
            errors.push(format!("{rel}: not UTF-8 text"));
            continue;
        };
        if text.contains('\r') {
            errors.push(format!("{rel}: CRLF line endings (deploy files are LF)"));
        }
        let is_yaml = matches!(f.extension().and_then(|e| e.to_str()), Some("yaml" | "yml"));
        if f.starts_with(&ctl_templates) {
            s.templates += 1;
            let (open, close) = (text.matches("{{").count(), text.matches("}}").count());
            if open != close {
                errors.push(format!("{rel}: {open} `{{{{` but {close} `}}}}`"));
            }
            for flag in template_flags(&text) {
                if !ctl_flags.contains(&flag) {
                    errors.push(format!("{rel}: `bicraft controller` has no flag {flag}"));
                }
                s.flags.insert(flag);
            }
        } else if f.starts_with(&ctl_chart) && is_yaml {
            s.yaml_files += 1;
            match YamlLoader::load_from_str(&text) {
                Ok(docs) => {
                    s.yaml_docs += docs.len();
                    if f == &ctl_chart.join("values.yaml") {
                        ctl_values = docs.into_iter().next().unwrap_or(Yaml::Null);
                    } else if f == &ctl_chart.join("Chart.yaml") {
                        check_chart(root, &rel, docs.first(), "bicraft-controller", &mut errors);
                    }
                }
                Err(e) => errors.push(format!("{rel}: YAML: {e}")),
            }
        } else if f.starts_with(&templates_dir) {
            s.templates += 1;
            let (open, close) = (text.matches("{{").count(), text.matches("}}").count());
            if open != close {
                errors.push(format!("{rel}: {open} `{{{{` but {close} `}}}}`"));
            }
            for flag in template_flags(&text) {
                check_flag(&rel, &flag, &known_flags, &mut errors);
                s.flags.insert(flag);
            }
        } else if is_yaml {
            s.yaml_files += 1;
            match YamlLoader::load_from_str(&text) {
                Ok(docs) => {
                    s.yaml_docs += docs.len();
                    if f == &chart.join("values.yaml") {
                        values = docs.into_iter().next().unwrap_or(Yaml::Null);
                    } else if f == &chart.join("Chart.yaml") {
                        check_chart(root, &rel, docs.first(), "bicraft", &mut errors);
                    } else if f.starts_with(deploy.join("manifests")) {
                        for d in &docs {
                            check_manifest(&rel, d, &known_flags, &mut s.flags, &mut errors);
                        }
                    }
                }
                Err(e) => errors.push(format!("{rel}: YAML: {e}")),
            }
        } else if f.extension().and_then(|e| e.to_str()) == Some("json") {
            check_dashboard(&rel, &text, &known_metrics(root), &mut errors);
        } else if f.file_name().and_then(|n| n.to_str()) == Some("Dockerfile") {
            for flag in check_dockerfile(&rel, &text, &mut errors) {
                check_flag(&rel, &flag, &known_flags, &mut errors);
                s.flags.insert(flag);
            }
            // the image's wasm-bindgen CLI must be the crate version of Cargo.lock (docs/webui.md)
            let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap_or_default();
            let want = crate::webui::locked_version(&lock, "wasm-bindgen").unwrap_or_default();
            if !text.contains(&format!(
                "ARG WASM_BINDGEN_VERSION={want}
"
            )) {
                errors.push(format!(
                    "{rel}: ARG WASM_BINDGEN_VERSION must be {want} (the wasm-bindgen of Cargo.lock)"
                ));
            }
        }
        texts.push((f.clone(), text));
    }
    if let Ok(doc) = std::fs::read_to_string(root.join("docs").join("kubernetes.md")) {
        texts.push((root.join("docs").join("kubernetes.md"), doc));
    } else {
        errors.push("docs/kubernetes.md is missing".into());
    }

    // Variables named anywhere in deploy/ and docs/kubernetes.md.
    for (f, text) in &texts {
        for v in env_names(text) {
            if !known_env.contains(&v) {
                errors.push(format!(
                    "{}: {v} is not read by the code (no \"{v}\" literal in crates/*/src or apps/*/src)",
                    rel(root, f)
                ));
            }
            s.env_vars.insert(v);
        }
        if text.contains("medium: Memory") {
            errors.push(format!(
                "{}: a memory-backed volume (medium: Memory)",
                rel(root, f)
            ));
        }
    }

    // The chart: values, template references, security invariants.
    if values.is_null() {
        errors.push("deploy/helm/bicraft/values.yaml: missing or empty".into());
    } else {
        check_values(&values, &mut errors);
        for (f, text) in texts.iter().filter(|(f, _)| f.starts_with(&templates_dir)) {
            for r in values_refs(text) {
                s.values_refs += 1;
                if lookup(&values, &r).is_none() {
                    errors.push(format!("{}: .Values.{r} is not in values.yaml", rel(root, f)));
                }
            }
        }
    }
    let template = |name: &str| {
        texts
            .iter()
            .find(|(f, _)| f == &templates_dir.join(name))
            .map(|(_, t)| t.as_str())
            .unwrap_or_default()
    };
    check_chart_templates(template("deployment.yaml"), template("_helpers.tpl"), &mut errors);

    // The controller chart.
    if ctl_values.is_null() {
        errors.push("deploy/helm/bicraft-controller/values.yaml: missing or empty".into());
    } else {
        check_controller_values(&ctl_values, &mut errors);
        for (f, text) in texts.iter().filter(|(f, _)| f.starts_with(&ctl_templates)) {
            for r in values_refs(text) {
                s.values_refs += 1;
                if lookup(&ctl_values, &r).is_none() {
                    errors.push(format!("{}: .Values.{r} is not in values.yaml", rel(root, f)));
                }
            }
        }
    }
    let ctl_template = |name: &str| {
        texts
            .iter()
            .find(|(f, _)| f == &ctl_templates.join(name))
            .map(|(_, t)| t.as_str())
            .unwrap_or_default()
    };
    check_controller_templates(
        ctl_template("deployment.yaml"),
        ctl_template("rbac.yaml"),
        ctl_template("crd.yaml"),
        &mut errors,
    );

    if errors.is_empty() { Ok(s) } else { Err(errors) }
}

fn rel(root: &Path, f: &Path) -> String {
    f.strip_prefix(root)
        .unwrap_or(f)
        .to_string_lossy()
        .replace('\\', "/")
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() && depth < 16 => stack.push((p, depth + 1)),
                Ok(t) if t.is_file() => out.push(p),
                _ => {}
            }
        }
    }
    out.sort();
    out
}

// ---------------------------------------------------------------------------------------------
// What the code knows

/// Every `"BICRAFT_…"` string literal in the non-test sources of crates/*/src and apps/*/src.
pub fn known_env_vars(root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for top in ["crates", "apps"] {
        let Ok(rd) = std::fs::read_dir(root.join(top)) else {
            continue;
        };
        for krate in rd.flatten() {
            // build scripts too: build-time variables (BICRAFT_WEBUI_DIR, BICRAFT_REQUIRE_WEBUI)
            let mut files = walk(&krate.path().join("src"));
            files.push(krate.path().join("build.rs"));
            for f in files {
                let name = f.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if !name.ends_with(".rs") || name.ends_with("tests.rs") {
                    continue;
                }
                if let Ok(text) = std::fs::read_to_string(&f) {
                    for v in env_names(&text) {
                        if text.contains(&format!("\"{v}\"")) {
                            out.insert(v);
                        }
                    }
                }
            }
        }
    }
    // the browser app's build tools (BICRAFT_WEBUI_TOOLS), read by `cargo xtask webui`
    for f in [root.join("xtask").join("src").join("webui.rs")] {
        if let Ok(text) = std::fs::read_to_string(&f) {
            for v in env_names(&text) {
                if text.contains(&format!("\"{v}\"")) {
                    out.insert(v);
                }
            }
        }
    }
    out
}

/// The flags `bicraft serve` parses (`"--flag" =>` arms of serve_cmd.rs).
pub fn known_serve_flags(root: &Path) -> BTreeSet<String> {
    flags_in(root, "serve_cmd.rs")
}

/// The flags of `bicraft controller` (controller_cmd.rs; it also takes the authentication flags of
/// serve_cmd.rs).
pub fn known_controller_flags(root: &Path) -> BTreeSet<String> {
    flags_in(root, "controller_cmd.rs")
}

fn flags_in(root: &Path, file: &str) -> BTreeSet<String> {
    let path = root.join("apps").join("bicraft-cli").join("src").join(file);
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("\"--") else {
            continue;
        };
        let Some(end) = rest.find('"') else { continue };
        let name = rest.get(..end).unwrap_or_default();
        let after = rest.get(end + 1..).unwrap_or_default().trim_start();
        if !name.is_empty()
            && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
            && after.starts_with("=>")
        {
            out.insert(format!("--{name}"));
        }
    }
    out
}

/// The metric families `/metrics` exposes: the `bicraft_…` names in the metrics module's sources.
pub fn known_metrics(root: &Path) -> BTreeSet<String> {
    let path = root.join("crates/mcp/src/http/metrics.rs");
    let text = std::fs::read_to_string(path).unwrap_or_default();
    metric_names(&text).into_iter().collect()
}

/// `bicraft_x_y` names in a text.
fn metric_names(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = text.get(from..).and_then(|t| t.find("bicraft_")) {
        let start = from + i;
        let name: String = text
            .get(start..)
            .unwrap_or_default()
            .chars()
            .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
            .collect();
        let before_ok = start == 0
            || text
                .as_bytes()
                .get(start - 1)
                .is_some_and(|b| !b.is_ascii_alphanumeric() && *b != b'_');
        if before_ok && name.len() > "bicraft_".len() {
            out.push(name.trim_end_matches('_').to_owned());
        }
        from = start + name.len().max(1);
    }
    out
}

/// A Grafana dashboard: valid JSON, and every metric its queries use is exposed by the code.
fn check_dashboard(rel: &str, text: &str, known: &BTreeSet<String>, errors: &mut Vec<String>) {
    let doc: serde_json::Value = match serde_json::from_str(text) {
        Ok(d) => d,
        Err(e) => {
            errors.push(format!("{rel}: JSON: {e}"));
            return;
        }
    };
    let mut exprs = Vec::new();
    let mut stack = vec![&doc];
    while let Some(v) = stack.pop() {
        match v {
            serde_json::Value::Object(o) => {
                if let Some(e) = o.get("expr").and_then(serde_json::Value::as_str) {
                    exprs.push(e.to_owned());
                }
                stack.extend(o.values());
            }
            serde_json::Value::Array(a) => stack.extend(a.iter()),
            _ => {}
        }
    }
    if exprs.is_empty() {
        errors.push(format!("{rel}: no Prometheus query"));
    }
    for e in &exprs {
        for name in metric_names(e) {
            let family = ["_bucket", "_sum", "_count"]
                .iter()
                .find_map(|s| name.strip_suffix(s))
                .filter(|f| known.contains(*f))
                .unwrap_or(&name);
            if !known.contains(family) {
                errors.push(format!(
                    "{rel}: {name} is not exposed by /metrics (crates/mcp/src/http/metrics.rs)"
                ));
            }
        }
    }
}

/// `BICRAFT_X` names in a text (not inside a longer word, not the bare prefix).
pub fn env_names(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(i) = text.get(from..).and_then(|t| t.find("BICRAFT_")) {
        let start = from + i;
        let mut end = start + "BICRAFT_".len();
        while bytes
            .get(end)
            .is_some_and(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'_')
        {
            end += 1;
        }
        let before_ok = start == 0 || bytes.get(start - 1).is_some_and(|b| !b.is_ascii_alphanumeric());
        let name = text.get(start..end).unwrap_or_default().trim_end_matches('_');
        if before_ok && name.len() > "BICRAFT_".len() {
            out.push(name.to_owned());
        }
        from = end;
    }
    out
}

fn check_flag(rel: &str, flag: &str, known: &BTreeSet<String>, errors: &mut Vec<String>) {
    if !known.contains(flag) {
        errors.push(format!("{rel}: `bicraft serve` has no flag {flag}"));
    }
}

/// Flags in a template's container args: lines `- --flag`.
fn template_flags(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.trim_start().strip_prefix("- --"))
        .map(|rest| {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || *c == '-')
                .collect();
            format!("--{name}")
        })
        .collect()
}

/// `.Values.a.b` paths used by a template.
fn values_refs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = text.get(from..).and_then(|t| t.find(".Values.")) {
        let start = from + i + ".Values.".len();
        let path: String = text
            .get(start..)
            .unwrap_or_default()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '_')
            .collect();
        let path = path.trim_end_matches('.').to_owned();
        if !path.is_empty() {
            out.push(path.clone());
        }
        from = start + path.len().max(1);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// YAML helpers

fn get<'a>(y: &'a Yaml, key: &str) -> Option<&'a Yaml> {
    y.as_hash()?.get(&Yaml::String(key.to_owned()))
}

fn lookup<'a>(y: &'a Yaml, dotted: &str) -> Option<&'a Yaml> {
    dotted.split('.').try_fold(y, |y, k| get(y, k))
}

fn str_of(y: Option<&Yaml>) -> Option<String> {
    match y? {
        Yaml::String(s) | Yaml::Real(s) => Some(s.clone()),
        Yaml::Integer(i) => Some(i.to_string()),
        Yaml::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}

fn int_of(y: Option<&Yaml>) -> Option<i64> {
    match y? {
        Yaml::Integer(i) => Some(*i),
        Yaml::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn is_true(y: Option<&Yaml>) -> bool {
    matches!(y, Some(Yaml::Boolean(true)))
}

fn list(y: Option<&Yaml>) -> &[Yaml] {
    y.and_then(Yaml::as_vec).map(Vec::as_slice).unwrap_or_default()
}

/// A Kubernetes memory quantity in MiB (`2048Mi`, `2Gi`).
pub fn quantity_mib(q: &str) -> Option<u64> {
    let q = q.trim();
    for (suffix, factor) in [("Mi", 1u64), ("Gi", 1024), ("Ti", 1024 * 1024)] {
        if let Some(n) = q.strip_suffix(suffix) {
            return n.parse::<u64>().ok()?.checked_mul(factor);
        }
    }
    None
}

/// A BIcraft size (`1984M`, `2G`: binary suffixes) in MiB, when it is a whole number of MiB.
pub fn bicraft_size_mib(v: &str) -> Option<u64> {
    let v = v.trim();
    for (suffix, factor) in [("M", 1u64), ("G", 1024), ("T", 1024 * 1024)] {
        if let Some(n) = v.strip_suffix(suffix) {
            return n.parse::<u64>().ok()?.checked_mul(factor);
        }
    }
    let bytes = v.parse::<u64>().ok()?;
    (bytes % (1 << 20) == 0).then_some(bytes >> 20)
}

/// A CPU quantity in whole cores (`2`, `2000m`).
fn cpu_cores(q: &str) -> Option<u64> {
    match q.trim().strip_suffix('m') {
        Some(m) => {
            let m = m.parse::<u64>().ok()?;
            (m % 1000 == 0).then_some(m / 1000)
        }
        None => q.trim().parse().ok(),
    }
}

// ---------------------------------------------------------------------------------------------
// Chart

fn workspace_version(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let section = text.split("[workspace.package]").nth(1)?;
    section.lines().find_map(|l| {
        let v = l.trim().strip_prefix("version")?.trim().strip_prefix('=')?.trim();
        Some(v.trim_matches('"').to_owned())
    })
}

fn check_chart(root: &Path, rel: &str, chart: Option<&Yaml>, name: &str, errors: &mut Vec<String>) {
    let Some(c) = chart else {
        errors.push(format!("{rel}: empty"));
        return;
    };
    if str_of(get(c, "apiVersion")).as_deref() != Some("v2") {
        errors.push(format!("{rel}: apiVersion must be v2"));
    }
    if str_of(get(c, "name")).as_deref() != Some(name) {
        errors.push(format!("{rel}: name must be {name}"));
    }
    let semver = |v: &str| {
        let core = v.split(['-', '+']).next().unwrap_or_default();
        core.split('.').count() == 3 && core.split('.').all(|p| p.parse::<u64>().is_ok())
    };
    if !str_of(get(c, "version")).is_some_and(|v| semver(&v)) {
        errors.push(format!("{rel}: version must be SemVer"));
    }
    let app = str_of(get(c, "appVersion"));
    let ws = workspace_version(root);
    if app.is_none() || app != ws {
        errors.push(format!(
            "{rel}: appVersion {app:?} differs from the workspace version {ws:?} (Cargo.toml)"
        ));
    }
}

fn check_values(v: &Yaml, errors: &mut Vec<String>) {
    let mut need = |cond: bool, what: &str| {
        if !cond {
            errors.push(format!("deploy/helm/bicraft/values.yaml: {what}"));
        }
    };
    for k in ["runAsUser", "runAsGroup", "fsGroup"] {
        need(
            int_of(lookup(v, &format!("podSecurityContext.{k}"))).is_some_and(|n| n > 0),
            &format!("podSecurityContext.{k} must be a non-zero id"),
        );
    }
    need(
        int_of(lookup(v, "model.memoryLimitMiB")).is_some_and(|n| n > 0),
        "model.memoryLimitMiB must default to a limit",
    );
    need(
        int_of(lookup(v, "model.cpu")).is_some_and(|n| n >= 1),
        "model.cpu must be whole cores",
    );
    need(
        int_of(lookup(v, "server.terminationGraceMarginSeconds")).is_some_and(|n| n >= 1),
        "server.terminationGraceMarginSeconds must be at least 1",
    );
    need(
        is_true(lookup(v, "networkPolicy.enabled")),
        "networkPolicy.enabled must default to true (no egress by default)",
    );
    need(
        lookup(v, "networkPolicy.egressCidrs").is_some_and(|y| list(Some(y)).is_empty()),
        "networkPolicy.egressCidrs must default to none",
    );
    need(
        !is_true(lookup(v, "databricks.enabled")),
        "databricks.enabled must default to false",
    );
    need(
        list(lookup(v, "persistence.accessModes")) == [Yaml::String("ReadWriteOnce".into())],
        "persistence.accessModes must be [ReadWriteOnce]",
    );
    need(
        list(lookup(v, "server.extraArgs")).is_empty(),
        "server.extraArgs must default to none",
    );
    need(
        str_of(lookup(v, "spill.sizeLimit")).is_some_and(|s| quantity_mib(&s).is_some()),
        "spill.sizeLimit must be a size (e.g. 20Gi)",
    );
}

/// Invariants the chart hard-codes (values cannot turn them off).
fn check_chart_templates(deployment: &str, helpers: &str, errors: &mut Vec<String>) {
    for needed in [
        "replicas: 1",
        "type: Recreate",
        "automountServiceAccountToken: false",
        "enableServiceLinks: false",
        "runAsNonRoot: true",
        "allowPrivilegeEscalation: false",
        "readOnlyRootFilesystem: true",
        "privileged: false",
        "- ALL",
        "type: RuntimeDefault",
        "path: /healthz",
        "path: /readyz",
        "name: TMPDIR",
        "name: BICRAFT_SPILL_DIR",
        "name: BICRAFT_MEMORY_LIMIT",
        "name: BICRAFT_SCAN_THREADS",
        "name: BICRAFT_AGG_THREADS",
        "mountPath: /tmp",
        "sizeLimit: {{ .Values.spill.sizeLimit }}",
        "- --metrics-listen",
        "fieldPath: status.podIP",
        "name: BICRAFT_LOG_FORMAT",
        "containerPort: {{ int .Values.metrics.port }}",
        "add (int .Values.server.shutdownGraceSeconds) (int .Values.server.terminationGraceMarginSeconds)",
    ] {
        if !deployment.contains(needed) {
            errors.push(format!(
                "deploy/helm/bicraft/templates/deployment.yaml: `{needed}` missing"
            ));
        }
    }
    if deployment.matches("runAsNonRoot: true").count() < 2
        || deployment.matches("type: RuntimeDefault").count() < 2
    {
        errors.push("deploy/helm/bicraft/templates/deployment.yaml: runAsNonRoot and seccomp must be set on the pod and the container".into());
    }
    if !helpers.contains(&format!("add $limit {POD_MARGIN_MIB}")) {
        errors.push(format!(
            "deploy/helm/bicraft/templates/_helpers.tpl: the pod memory limit must be BICRAFT_MEMORY_LIMIT + {POD_MARGIN_MIB} MiB"
        ));
    }
}

const CTL: &str = "deploy/helm/bicraft-controller";

fn check_controller_values(v: &Yaml, errors: &mut Vec<String>) {
    let mut need = |cond: bool, what: &str| {
        if !cond {
            errors.push(format!("{CTL}/values.yaml: {what}"));
        }
    };
    for k in ["runAsUser", "runAsGroup", "fsGroup"] {
        need(
            int_of(lookup(v, &format!("podSecurityContext.{k}"))).is_some_and(|n| n > 0),
            &format!("podSecurityContext.{k} must be a non-zero id"),
        );
    }
    need(
        str_of(lookup(v, "crd.group")).as_deref() == Some("bicraft.dev"),
        "crd.group must default to bicraft.dev (DEFAULT_GROUP of crates/controller/src/config.rs)",
    );
    need(
        is_true(lookup(v, "networkPolicy.enabled")),
        "networkPolicy.enabled must default to true",
    );
    need(
        is_true(lookup(v, "workspaces.networkPolicy")),
        "workspaces.networkPolicy must default to true (only the gateway reaches a workspace pod)",
    );
    need(
        list(lookup(v, "workspaces.gatewayTrustedCidrs")).is_empty(),
        "workspaces.gatewayTrustedCidrs must default to none (the controller pod's own address)",
    );
    need(
        str_of(lookup(v, "workspaces.defaultReclaimPolicy")).as_deref() == Some("Retain"),
        "workspaces.defaultReclaimPolicy must default to Retain (deleting data is a decision)",
    );
    need(
        lookup(v, "workspaces.egressProfiles").is_some_and(|y| y.as_hash().is_some_and(|h| h.is_empty())),
        "workspaces.egressProfiles must default to none (no egress by default)",
    );
    let default_class = str_of(lookup(v, "workspaces.defaultClass")).unwrap_or_default();
    match lookup(v, "workspaces.classes").and_then(Yaml::as_hash) {
        Some(classes) if !classes.is_empty() => {
            if !classes.contains_key(&Yaml::String(default_class.clone())) {
                need(false, "workspaces.defaultClass must name a class");
            }
            for (name, c) in classes {
                let name = name.as_str().unwrap_or("?");
                need(
                    int_of(get(c, "memoryLimitMiB")).is_some_and(|n| n > 0),
                    &format!("class {name}: memoryLimitMiB must be a limit"),
                );
                need(
                    int_of(get(c, "cpu")).is_some_and(|n| n >= 1),
                    &format!("class {name}: cpu must be whole cores"),
                );
                for q in ["spillSize", "volumeSize", "maxVolumeSize"] {
                    need(
                        str_of(get(c, q)).is_some_and(|s| quantity_mib(&s).is_some()),
                        &format!("class {name}: {q} must be a size"),
                    );
                }
            }
        }
        _ => need(false, "workspaces.classes must define at least one class"),
    }
}

/// The controller chart's hard-coded invariants: the pod's security context, least-privilege RBAC
/// and a CRD whose group follows `crd.group`.
fn check_controller_templates(deployment: &str, rbac: &str, crd: &str, errors: &mut Vec<String>) {
    for needed in [
        "replicas: 1",
        "type: Recreate",
        "enableServiceLinks: false",
        "runAsNonRoot: true",
        "allowPrivilegeEscalation: false",
        "readOnlyRootFilesystem: true",
        "privileged: false",
        "- ALL",
        "type: RuntimeDefault",
        "path: /healthz",
        "path: /readyz",
        "name: POD_IP",
        "fieldPath: status.podIP",
        "- controller",
    ] {
        if !deployment.contains(needed) {
            errors.push(format!("{CTL}/templates/deployment.yaml: `{needed}` missing"));
        }
    }
    if deployment.matches("runAsNonRoot: true").count() < 2
        || deployment.matches("type: RuntimeDefault").count() < 2
    {
        errors.push(format!(
            "{CTL}/templates/deployment.yaml: runAsNonRoot and seccomp must be set on the pod and the container"
        ));
    }
    for banned in [
        "kind: ClusterRole",
        "\"secrets\"",
        "\"pods\"",
        "\"*\"",
        "\"update\"",
        "escalate",
        "\"bind\"",
    ] {
        if rbac.contains(banned) {
            errors.push(format!(
                "{CTL}/templates/rbac.yaml: `{banned}`: the controller's role is namespaced and least-privilege"
            ));
        }
    }
    let deletes = rbac.lines().filter(|l| l.contains("\"delete\"")).count();
    let rules = rbac.lines().filter(|l| l.contains("resources:")).count();
    let delete_ok = deletes == 1
        && rbac
            .lines()
            .zip(rbac.lines().skip(1))
            .any(|(a, b)| a.contains("[\"bicraftworkspaces\"]") && b.contains("\"delete\""));
    if !delete_ok || rules != 5 {
        errors.push(format!(
            "{CTL}/templates/rbac.yaml: `delete` only on bicraftworkspaces (the garbage collector deletes the rest), five rules"
        ));
    }
    for needed in [
        "group: {{ .Values.crd.group }}",
        "subresources:",
        "status: {}",
        "kind: BicraftWorkspace",
    ] {
        if !crd.contains(needed) {
            errors.push(format!("{CTL}/templates/crd.yaml: `{needed}` missing"));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Plain manifests

/// A Deployment's container as the smoke test needs it.
#[derive(Debug, Default)]
pub struct Container {
    pub args: Vec<String>,
    /// Literal values only (`valueFrom` entries are skipped).
    pub env: Vec<(String, String)>,
    /// mountPath → volume name
    pub mounts: Vec<(String, String)>,
}

pub fn container(deployment: &Yaml) -> Option<Container> {
    let c = list(lookup(deployment, "spec.template.spec.containers")).first()?;
    Some(Container {
        args: list(get(c, "args"))
            .iter()
            .filter_map(|a| str_of(Some(a)))
            .collect(),
        env: list(get(c, "env"))
            .iter()
            .filter_map(|e| Some((str_of(get(e, "name"))?, str_of(get(e, "value"))?)))
            .collect(),
        mounts: list(get(c, "volumeMounts"))
            .iter()
            .filter_map(|m| Some((str_of(get(m, "mountPath"))?, str_of(get(m, "name"))?)))
            .collect(),
    })
}

pub fn arg_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).map(String::as_str)
}

pub fn env_value<'a>(c: &'a Container, name: &str) -> Option<&'a str> {
    c.env.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

pub fn check_manifest(
    rel: &str,
    doc: &Yaml,
    known_flags: &BTreeSet<String>,
    flags: &mut BTreeSet<String>,
    errors: &mut Vec<String>,
) {
    let kind = str_of(get(doc, "kind")).unwrap_or_default();
    if doc.is_null() {
        return;
    }
    if kind.is_empty() {
        errors.push(format!("{rel}: a document without kind"));
        return;
    }
    if kind != "Deployment" {
        return;
    }
    let mut err = |m: String| errors.push(format!("{rel}: {m}"));
    let spec = get(doc, "spec");
    if int_of(spec.and_then(|s| get(s, "replicas"))) != Some(1) {
        err("replicas must be 1 (one session per workspace volume)".into());
    }
    if str_of(lookup(doc, "spec.strategy.type")).as_deref() != Some("Recreate") {
        err("strategy must be Recreate (never two pods on one volume)".into());
    }
    let pod = lookup(doc, "spec.template.spec").unwrap_or(&Yaml::Null);
    if !matches!(
        get(pod, "automountServiceAccountToken"),
        Some(Yaml::Boolean(false))
    ) {
        err("automountServiceAccountToken must be false".into());
    }
    if !matches!(get(pod, "enableServiceLinks"), Some(Yaml::Boolean(false))) {
        err("enableServiceLinks must be false (a Service named bicraft injects BICRAFT_* variables)".into());
    }
    let psc = get(pod, "securityContext").unwrap_or(&Yaml::Null);
    if !is_true(get(psc, "runAsNonRoot")) {
        err("pod securityContext.runAsNonRoot must be true".into());
    }
    if !int_of(get(psc, "runAsUser")).is_some_and(|u| u > 0) {
        err("pod securityContext.runAsUser must be a non-zero fixed id".into());
    }
    if str_of(lookup(psc, "seccompProfile.type")).as_deref() != Some("RuntimeDefault") {
        err("pod seccompProfile must be RuntimeDefault".into());
    }
    let volumes = list(get(pod, "volumes"));
    for v in volumes {
        if let Some(ed) = get(v, "emptyDir") {
            if str_of(get(ed, "medium")).is_some_and(|m| m == "Memory") {
                err("an emptyDir with medium: Memory".into());
            }
            if get(ed, "sizeLimit").is_none() {
                err(format!("emptyDir {:?} without sizeLimit", str_of(get(v, "name"))));
            }
        }
    }
    let containers = list(get(pod, "containers"));
    if containers.len() != 1 {
        err("exactly one container expected".into());
    }
    for c in containers {
        let sc = get(c, "securityContext").unwrap_or(&Yaml::Null);
        if !matches!(get(sc, "allowPrivilegeEscalation"), Some(Yaml::Boolean(false))) {
            err("container allowPrivilegeEscalation must be false".into());
        }
        if !is_true(get(sc, "readOnlyRootFilesystem")) {
            err("container readOnlyRootFilesystem must be true".into());
        }
        if is_true(get(sc, "privileged")) {
            err("container must not be privileged".into());
        }
        if !list(lookup(sc, "capabilities.drop")).contains(&Yaml::String("ALL".into())) {
            err("container capabilities.drop must contain ALL".into());
        }
        if !list(lookup(sc, "capabilities.add")).is_empty() {
            err("container must not add capabilities".into());
        }
        if str_of(lookup(sc, "seccompProfile.type")).is_some_and(|t| t != "RuntimeDefault") {
            err("container seccompProfile must be RuntimeDefault".into());
        }
        for (probe, path) in [("livenessProbe", "/healthz"), ("readinessProbe", "/readyz")] {
            if str_of(lookup(c, &format!("{probe}.httpGet.path"))).as_deref() != Some(path) {
                err(format!("{probe} must GET {path}"));
            }
        }
    }
    let Some(c) = container(doc) else { return };
    for a in c.args.iter().filter(|a| a.starts_with("--")) {
        if !known_flags.contains(a) {
            err(format!("`bicraft serve` has no flag {a}"));
        }
        flags.insert(a.clone());
    }
    // Metrics on a port of their own, declared, and the pod IP they bind defined.
    if let Some(m) = arg_value(&c.args, "--metrics-listen") {
        let port_of = |a: &str| a.rsplit_once(':').map(|(_, p)| p.to_owned());
        let listen = arg_value(&c.args, "--listen").and_then(port_of);
        let mport = port_of(m);
        if mport.is_none() || mport == listen {
            err(format!("--metrics-listen {m} needs a port other than --listen's"));
        }
        let first = containers.first().unwrap_or(&Yaml::Null);
        let declared = list(get(first, "ports")).iter().any(|p| {
            str_of(get(p, "name")).as_deref() == Some("metrics")
                && int_of(get(p, "containerPort")).map(|n| n.to_string()) == mport
        });
        if !declared {
            err(format!(
                "--metrics-listen {m}: no containerPort named metrics on that port"
            ));
        }
        let pod_ip = list(get(first, "env")).iter().any(|e| {
            str_of(get(e, "name")).as_deref() == Some("POD_IP")
                && str_of(lookup(e, "valueFrom.fieldRef.fieldPath")).as_deref() == Some("status.podIP")
        });
        if m.contains("$(POD_IP)") && !pod_ip {
            err("--metrics-listen uses $(POD_IP): declare POD_IP from status.podIP".into());
        }
    }
    // Termination grace above --shutdown-grace.
    let grace = arg_value(&c.args, "--shutdown-grace")
        .and_then(|g| g.parse::<i64>().ok())
        .unwrap_or(30);
    match int_of(get(pod, "terminationGracePeriodSeconds")) {
        Some(t) if t > grace => {}
        t => err(format!(
            "terminationGracePeriodSeconds {t:?} must be above --shutdown-grace {grace}"
        )),
    }
    // Memory: pod limit = BICRAFT_MEMORY_LIMIT + 64 MiB.
    let first = containers.first().unwrap_or(&Yaml::Null);
    let mem_limit = str_of(lookup(first, "resources.limits.memory")).and_then(|q| quantity_mib(&q));
    match (
        env_value(&c, "BICRAFT_MEMORY_LIMIT").map(bicraft_size_mib),
        mem_limit,
    ) {
        (Some(Some(limit)), Some(pod_mib)) if limit + POD_MARGIN_MIB == pod_mib => {}
        (Some(l), p) => err(format!(
            "memory: the pod limit ({p:?} MiB) must be BICRAFT_MEMORY_LIMIT ({l:?} MiB) + {POD_MARGIN_MIB} MiB"
        )),
        (None, _) => err("BICRAFT_MEMORY_LIMIT must be set (pod limit = it + 64 MiB)".into()),
    }
    // Threads = the CPU limit.
    let cpu = str_of(lookup(first, "resources.limits.cpu")).and_then(|q| cpu_cores(&q));
    let scan = env_value(&c, "BICRAFT_SCAN_THREADS").and_then(|v| v.parse::<u64>().ok());
    if cpu.is_none() || scan != cpu {
        err(format!(
            "BICRAFT_SCAN_THREADS ({scan:?}) must equal the CPU limit ({cpu:?} cores)"
        ));
    }
    if env_value(&c, "BICRAFT_AGG_THREADS")
        .and_then(|v| v.parse::<u64>().ok())
        .is_none()
    {
        err("BICRAFT_AGG_THREADS must be set".into());
    }
    // TMPDIR and the spill folder on a disk-backed emptyDir.
    let empty_dirs: Vec<String> = volumes
        .iter()
        .filter(|v| get(v, "emptyDir").is_some())
        .filter_map(|v| str_of(get(v, "name")))
        .collect();
    for var in ["TMPDIR", "BICRAFT_SPILL_DIR"] {
        let on_empty_dir = env_value(&c, var).is_some_and(|path| {
            c.mounts
                .iter()
                .any(|(m, vol)| empty_dirs.contains(vol) && (path == m || path.starts_with(&format!("{m}/"))))
        });
        if !on_empty_dir {
            err(format!("{var} must point into a mounted emptyDir"));
        }
    }
    // The workspace volume sits directly under --root, named after a granted workspace.
    let root = arg_value(&c.args, "--root")
        .unwrap_or_default()
        .trim_end_matches('/')
        .to_owned();
    let workspaces: Vec<&str> = c
        .mounts
        .iter()
        .filter_map(|(m, _)| m.strip_prefix(&format!("{root}/")))
        .filter(|w| !w.contains('/'))
        .collect();
    if root.is_empty() || workspaces.is_empty() {
        err("a volume must be mounted at <--root>/<workspace>".into());
    }
    // editor or reader grants (docs/auth.md "Roles")
    let grants = [
        env_value(&c, "BICRAFT_AUTH_GRANTS").unwrap_or_default(),
        env_value(&c, "BICRAFT_AUTH_READER_GRANTS").unwrap_or_default(),
    ]
    .join(";");
    for w in &workspaces {
        let granted = grants.split([';', ' ']).any(|g| {
            g.rsplit_once('=')
                .is_some_and(|(_, ws)| ws.split(',').any(|x| x == "*" || x == *w))
        });
        if !granted {
            err(format!("no grant opens the workspace {w}"));
        }
    }
    if !c
        .env
        .iter()
        .any(|(k, _)| k == "BICRAFT_ALLOWED_HOSTS" || k == "BICRAFT_PUBLIC_URL")
    {
        err("a non-loopback --listen needs BICRAFT_ALLOWED_HOSTS or BICRAFT_PUBLIC_URL".into());
    }
}

// ---------------------------------------------------------------------------------------------
// Dockerfile

/// Checks the Dockerfile; returns the `bicraft serve` flags of its CMD.
fn check_dockerfile(rel: &str, text: &str, errors: &mut Vec<String>) -> Vec<String> {
    let mut err = |m: String| errors.push(format!("{rel}: {m}"));
    let mut stages: Vec<String> = Vec::new();
    let mut last_from = String::new();
    let mut user = None;
    let mut entrypoint = None;
    let mut cmd = None;
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("FROM ") {
            let mut parts = rest.split_whitespace();
            let image = parts.next().unwrap_or_default().to_owned();
            if let (Some(kw), Some(name)) = (parts.next(), parts.next())
                && kw.eq_ignore_ascii_case("as")
            {
                stages.push(name.to_owned());
            }
            let is_stage = stages.contains(&image) || image.contains("${");
            if image != "scratch" && !is_stage {
                let (name_tag, digest) = image.split_once('@').unwrap_or((&image, ""));
                let tag = name_tag.rsplit_once(':').map(|(_, t)| t).unwrap_or_default();
                if tag.is_empty() || tag == "latest" {
                    err(format!("base image {image} must be pinned by tag"));
                }
                let hex = digest.strip_prefix("sha256:").unwrap_or_default();
                if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    err(format!("base image {image} must be pinned by digest"));
                }
            }
            last_from = image;
            user = None;
        } else if let Some(u) = t.strip_prefix("USER ") {
            user = Some(u.trim().to_owned());
        } else if let Some(e) = t.strip_prefix("ENTRYPOINT ") {
            entrypoint = serde_json::from_str::<Vec<String>>(e).ok();
        } else if let Some(c) = t.strip_prefix("CMD ") {
            cmd = serde_json::from_str::<Vec<String>>(c).ok();
        }
    }
    if last_from != "scratch" && !last_from.contains("distroless") {
        err(format!(
            "the final stage must be FROM scratch or distroless, not {last_from}"
        ));
    }
    let numeric_non_root = user
        .as_deref()
        .is_some_and(|u| u.split(':').all(|p| p.parse::<u32>().is_ok_and(|n| n > 0)));
    if !numeric_non_root {
        err(format!(
            "the final stage needs a numeric non-root USER, not {user:?}"
        ));
    }
    match &entrypoint {
        Some(e)
            if e.len() == 2
                && e.first().is_some_and(|b| b.ends_with("/bicraft"))
                && e.get(1).is_some_and(|s| s == "serve") => {}
        e => err(format!(
            "ENTRYPOINT must be [\"…/bicraft\", \"serve\"] (JSON form), not {e:?}"
        )),
    }
    if !text.contains("--locked") {
        err("the build must use --locked".into());
    }
    if !text.contains("TMPDIR=/tmp") {
        err("TMPDIR must be set".into());
    }
    // Every release binary embeds the browser app (docs/webui.md): built and required in the
    // build stage, checked on the binary that goes into the image (prebuilt ones too).
    for (needle, why) in [
        ("cargo xtask webui", "the build must run `cargo xtask webui`"),
        (
            "BICRAFT_REQUIRE_WEBUI=1",
            "the build must set BICRAFT_REQUIRE_WEBUI=1",
        ),
        (
            "build-info | grep -q '\"embedded\":true'",
            "the image must check `bicraft build-info` for the browser app",
        ),
    ] {
        if !text.contains(needle) {
            err(why.into());
        }
    }
    // The binary embeds MIT, OFL and Unicode material: its notices travel with the image.
    for f in [
        "THIRD_PARTY_NOTICES.md",
        "LICENSE-MIT",
        "LICENSE-APACHE",
        "licenses/",
    ] {
        if !text
            .lines()
            .any(|l| l.trim_start().starts_with("COPY ") && l.contains(f))
        {
            err(format!(
                "the image must ship {f} (COPY it under /usr/share/doc/bicraft)"
            ));
        }
    }
    cmd.unwrap_or_default()
        .into_iter()
        .filter(|a| a.starts_with("--"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        crate::root()
    }

    #[test]
    fn deploy_files_are_consistent_with_the_code() {
        match run(&root()) {
            Ok(s) => {
                assert!(s.yaml_files >= 8, "{s:?}");
                assert!(s.flags.contains("--shutdown-grace"), "{s:?}");
                assert!(s.env_vars.contains("BICRAFT_MEMORY_LIMIT"), "{s:?}");
                assert!(s.values_refs > 50, "{s:?}");
            }
            Err(e) => panic!("deploy check failed:\n  {}", e.join("\n  ")),
        }
    }

    #[test]
    fn the_code_knows_its_variables_and_flags() {
        let env = known_env_vars(&root());
        for v in [
            "BICRAFT_MEMORY_LIMIT",
            "BICRAFT_SPILL_DIR",
            "BICRAFT_API_TOKENS_FILE",
            "BICRAFT_DATABRICKS_TOKEN",
        ] {
            assert!(env.contains(v), "{v}");
        }
        assert!(!env.contains("BICRAFT_NOT_A_VARIABLE"));
        let flags = known_serve_flags(&root());
        for f in [
            "--root",
            "--listen",
            "--shutdown-grace",
            "--grant",
            "--drain-on-stdin-close",
        ] {
            assert!(flags.contains(f), "{f} {flags:?}");
        }
        let ctl = known_controller_flags(&root());
        for f in ["--config", "--admin", "--interval", "--kube-api"] {
            assert!(ctl.contains(f), "{f} {ctl:?}");
        }
        assert!(env.contains("BICRAFT_CONTROLLER_ADMINS"));
        assert!(env.contains("BICRAFT_CONTROLLER_CONFIG"));
    }

    #[test]
    fn controller_rbac_drift_is_caught() {
        let read = |n: &str| std::fs::read_to_string(root().join(CTL).join("templates").join(n)).unwrap();
        let (d, r, c) = (read("deployment.yaml"), read("rbac.yaml"), read("crd.yaml"));
        let mut e = Vec::new();
        check_controller_templates(&d, &r, &c, &mut e);
        assert!(e.is_empty(), "{e:?}");
        let wider = r.replace(
            "resources: [\"services\", \"persistentvolumeclaims\"]",
            "resources: [\"services\", \"persistentvolumeclaims\", \"secrets\"]",
        );
        let mut e = Vec::new();
        check_controller_templates(&d, &wider, &c, &mut e);
        assert!(e.iter().any(|m| m.contains("secrets")), "{e:?}");
        let deleting = r.replace(
            "verbs: [\"get\", \"list\", \"create\", \"patch\"]\n  - apiGroups: [\"\"]",
            "verbs: [\"get\", \"list\", \"create\", \"patch\", \"delete\"]\n  - apiGroups: [\"\"]",
        );
        let mut e = Vec::new();
        check_controller_templates(&d, &deleting, &c, &mut e);
        assert!(e.iter().any(|m| m.contains("delete")), "{e:?}");
        let mut e = Vec::new();
        check_controller_templates(&d.replace("readOnlyRootFilesystem: true", ""), &r, &c, &mut e);
        assert!(!e.is_empty());
    }

    #[test]
    fn names_and_sizes() {
        assert_eq!(
            env_names("a BICRAFT_X_Y, XBICRAFT_OLD BICRAFT_* \"BICRAFT_Z\""),
            ["BICRAFT_X_Y", "BICRAFT_Z"]
        );
        assert_eq!(quantity_mib("2Gi"), Some(2048));
        assert_eq!(quantity_mib("2048Mi"), Some(2048));
        assert_eq!(quantity_mib("2G"), None);
        assert_eq!(bicraft_size_mib("1984M"), Some(1984));
        assert_eq!(bicraft_size_mib("2G"), Some(2048));
        assert_eq!(bicraft_size_mib("1048576"), Some(1));
        assert_eq!(bicraft_size_mib("1000"), None);
        assert_eq!(cpu_cores("2000m"), Some(2));
        assert_eq!(cpu_cores("1500m"), None);
        assert_eq!(values_refs("{{ .Values.a.b }} {{ $.Values.c }}."), ["a.b", "c"]);
        assert_eq!(
            template_flags("  - --root\n  - {{ x }}\n  - --max-upload-bytes\n"),
            ["--root", "--max-upload-bytes"]
        );
    }

    /// The plain Deployment with one substitution; the errors of its check.
    fn manifest_errors(from: &str, to: &str) -> Vec<String> {
        let text = std::fs::read_to_string(root().join("deploy/manifests/deployment.yaml")).unwrap();
        assert!(text.contains(from), "{from}");
        let docs = YamlLoader::load_from_str(&text.replacen(from, to, 1)).unwrap();
        let mut errors = Vec::new();
        check_manifest(
            "d.yaml",
            &docs[0],
            &known_serve_flags(&root()),
            &mut BTreeSet::new(),
            &mut errors,
        );
        errors
    }

    #[test]
    fn manifest_drift_is_caught() {
        assert!(manifest_errors("replicas: 1", "replicas: 1").is_empty());
        // a reader grant opens the workspace too
        assert!(manifest_errors("name: BICRAFT_AUTH_GRANTS", "name: BICRAFT_AUTH_READER_GRANTS").is_empty());
        for (from, to, why) in [
            ("replicas: 1", "replicas: 2", "replicas"),
            ("type: Recreate", "type: RollingUpdate", "Recreate"),
            (
                "readOnlyRootFilesystem: true",
                "readOnlyRootFilesystem: false",
                "readOnlyRootFilesystem",
            ),
            (
                "allowPrivilegeEscalation: false",
                "allowPrivilegeEscalation: true",
                "allowPrivilegeEscalation",
            ),
            ("                - ALL", "                - NET_RAW", "drop"),
            ("runAsUser: 65532", "runAsUser: 0", "runAsUser"),
            ("value: 1984M", "value: 2048M", "BICRAFT_MEMORY_LIMIT"),
            ("value: \"2\"", "value: \"4\"", "BICRAFT_SCAN_THREADS"),
            (
                "terminationGracePeriodSeconds: 35",
                "terminationGracePeriodSeconds: 30",
                "terminationGracePeriodSeconds",
            ),
            ("- --shutdown-grace", "- --shutdown-grace-period", "no flag"),
            ("sizeLimit: 20Gi", "medium: Memory", "Memory"),
            ("path: /readyz", "path: /healthz", "readinessProbe"),
            (
                "enableServiceLinks: false",
                "enableServiceLinks: true",
                "enableServiceLinks",
            ),
            ("value: token:agent=main", "value: token:agent=other", "grant"),
            (
                "name: BICRAFT_SPILL_DIR\n              value: /tmp",
                "name: BICRAFT_SPILL_DIR\n              value: /data",
                "BICRAFT_SPILL_DIR",
            ),
            ("- $(POD_IP):9090", "- $(POD_IP):7878", "port other than"),
            (
                "containerPort: 9090",
                "containerPort: 9091",
                "containerPort named metrics",
            ),
            ("fieldPath: status.podIP", "fieldPath: status.hostIP", "POD_IP"),
        ] {
            let e = manifest_errors(from, to);
            assert!(e.iter().any(|m| m.contains(why)), "{from} -> {to}: {e:?}");
        }
    }

    #[test]
    fn dashboards_use_exposed_metrics() {
        let known = known_metrics(&root());
        for m in [
            "bicraft_commands_total",
            "bicraft_draining",
            "bicraft_memory_limit_bytes",
        ] {
            assert!(known.contains(m), "{m}: {known:?}");
        }
        let mut e = Vec::new();
        let ok = r#"{"panels":[{"targets":[{"expr":"histogram_quantile(0.95, sum by (le) (rate(bicraft_command_duration_seconds_bucket[5m])))"}]}]}"#;
        check_dashboard("d.json", ok, &known, &mut e);
        assert!(e.is_empty(), "{e:?}");
        let bad = r#"{"panels":[{"targets":[{"expr":"rate(bicraft_no_such_total[5m])"}]}]}"#;
        check_dashboard("d.json", bad, &known, &mut e);
        assert!(e.iter().any(|m| m.contains("bicraft_no_such_total")), "{e:?}");
        e.clear();
        check_dashboard("d.json", "{", &known, &mut e);
        assert!(e.iter().any(|m| m.contains("JSON")), "{e:?}");
    }

    #[test]
    fn dockerfile_rules() {
        let ok = "FROM rust:1.99@sha256:0000000000000000000000000000000000000000000000000000000000000000 AS b\nRUN cargo xtask webui --out /w && BICRAFT_REQUIRE_WEBUI=1 cargo build --locked\nRUN bicraft build-info | grep -q '\"embedded\":true'\nCOPY LICENSE-MIT LICENSE-APACHE THIRD_PARTY_NOTICES.md /d/\nCOPY licenses/ /d/licenses/\nFROM scratch\nENV TMPDIR=/tmp\nUSER 65532:65532\nENTRYPOINT [\"/usr/local/bin/bicraft\", \"serve\"]\nCMD [\"--root\", \"/srv\"]\n";
        let mut e = Vec::new();
        assert_eq!(check_dockerfile("D", ok, &mut e), ["--root"]);
        assert!(e.is_empty(), "{e:?}");
        for (from, to, why) in [
            (
                "rust:1.99@sha256:0000000000000000000000000000000000000000000000000000000000000000",
                "rust:latest",
                "pinned",
            ),
            ("USER 65532:65532", "USER root", "USER"),
            ("USER 65532:65532", "USER 0:0", "USER"),
            (
                "FROM scratch",
                "FROM alpine:3@sha256:0000000000000000000000000000000000000000000000000000000000000000",
                "final stage",
            ),
            ("\"serve\"", "\"mcp\"", "ENTRYPOINT"),
            ("--locked", "", "--locked"),
            (
                "COPY licenses/ /d/licenses/",
                "COPY other/ /d/other/",
                "licenses/",
            ),
            (" THIRD_PARTY_NOTICES.md", "", "THIRD_PARTY_NOTICES.md"),
            ("BICRAFT_REQUIRE_WEBUI=1 ", "", "BICRAFT_REQUIRE_WEBUI"),
            ("cargo xtask webui --out /w && ", "", "xtask webui"),
            ("RUN bicraft build-info", "RUN bicraft --version #", "build-info"),
        ] {
            let mut e = Vec::new();
            check_dockerfile("D", &ok.replace(from, to), &mut e);
            assert!(e.iter().any(|m| m.contains(why)), "{from} -> {to}: {e:?}");
        }
    }
}
