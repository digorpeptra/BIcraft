//! `validate_documents`: JSON Schema validation against each document's declared `$schema`, then
//! cross-file consistency.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::Value;

use crate::Issue;
use crate::json::{self, parse_schema_url, schema_url};
use crate::paths::{self, DocKind};
use crate::schema;

pub(crate) fn validate(docs: &[(String, Vec<u8>)]) -> Vec<Issue> {
    // Validation recurses with the schema; give it room independent of the caller's stack.
    schema::on_big_stack(|| run(docs))
        .unwrap_or_else(|| vec![issue("", "", "validation failed unexpectedly".to_string())])
}

fn issue(path: &str, pointer: &str, message: String) -> Issue {
    Issue {
        path: path.to_string(),
        pointer: pointer.to_string(),
        message,
    }
}

/// Desktop ignores a page or visual whose folder name or `name` is not made of letters, digits,
/// underscores and hyphens (packaging-verification.md, fact P-NAME).
fn name_issues(issues: &mut Vec<Issue>, path: &str, what: &str, folder: &str, name: &str) {
    if let Err(m) = paths::check_object_name(folder) {
        issues.push(issue(
            path,
            "",
            format!("{what} folder `{folder}`: {m}; Desktop ignores it"),
        ));
    }
    if name != folder
        && let Err(m) = paths::check_object_name(name)
    {
        issues.push(issue(
            path,
            "/name",
            format!("{what} name `{name}`: {m}; Desktop ignores it"),
        ));
    }
}

struct Json {
    path: String,
    kind: DocKind,
    value: Value,
}

fn run(docs: &[(String, Vec<u8>)]) -> Vec<Issue> {
    let mut issues = Vec::new();
    let mut parsed: Vec<Json> = Vec::new();
    let mut resources: HashSet<String> = HashSet::new();
    let mut seen = HashSet::new();
    for (raw, bytes) in docs {
        let path = paths::normalize(raw);
        let kind = match paths::classify(&path) {
            Ok(k) => k,
            Err(e) => {
                issues.push(issue(&path, "", e.to_string()));
                continue;
            }
        };
        match kind {
            DocKind::Ignored => continue,
            DocKind::Resource => {
                resources.insert(path);
                continue;
            }
            DocKind::OtherDefinition => {
                issues.push(issue(&path, "", "not a file of the PBIR format".into()));
                continue;
            }
            _ => {}
        }
        if !seen.insert(path.to_lowercase()) {
            issues.push(issue(
                &path,
                "",
                "duplicate document (paths differ only by case)".into(),
            ));
            continue;
        }
        let value = match json::parse(&path, bytes) {
            Ok(v) => v,
            Err(crate::Error::Invalid { message, .. }) => {
                issues.push(issue(&path, "", message));
                continue;
            }
            Err(e) => {
                issues.push(issue(&path, "", e.to_string()));
                continue;
            }
        };
        check_schema(&path, &kind, &value, &mut issues);
        parsed.push(Json { path, kind, value });
    }
    consistency(&parsed, &resources, &mut issues);
    issues
}

fn check_schema(path: &str, kind: &DocKind, value: &Value, issues: &mut Vec<Issue>) {
    let Some((_, expected)) = kind.schema_kind() else {
        return;
    };
    let Some(obj) = value.as_object() else {
        issues.push(issue(path, "", "the document is not a JSON object".into()));
        return;
    };
    let Some(url) = obj.get("$schema").and_then(Value::as_str) else {
        issues.push(issue(path, "", "missing `$schema`".into()));
        return;
    };
    let Some((key, version)) = parse_schema_url(url) else {
        issues.push(issue(path, "/$schema", "not a PBIR schema URL".into()));
        return;
    };
    if key != expected {
        issues.push(issue(
            path,
            "/$schema",
            format!("declares the `{key}` schema; this file needs `{expected}`"),
        ));
        return;
    }
    let Some((file, substituted)) = schema::vendored(key, version) else {
        let (a, b, c) = version;
        issues.push(issue(
            path,
            "/$schema",
            format!("unknown schema version {a}.{b}.{c} (no known version with major {a})"),
        ));
        return;
    };
    let Some(store) = schema::vendored_schema(&file) else {
        issues.push(issue(
            path,
            "/$schema",
            format!("schema `{file}` could not be loaded"),
        ));
        return;
    };
    let found = if substituted {
        // A newer minor than the vendored ones: validate against the newest known minor, as
        // Desktop opens it (oracle case j12, docs/formats/packaging-verification.md P-SCHEMA).
        let mut copy = value.clone();
        let version = file.split('/').nth(1).unwrap_or_default();
        if let Some(m) = copy.as_object_mut() {
            m.insert("$schema".into(), Value::String(schema_url(key, version)));
        }
        store.validate(&copy)
    } else {
        store.validate(value)
    };
    let truncated = found.len() >= schema::MAX_ISSUES;
    issues.extend(found.into_iter().map(|(p, m)| issue(path, &p, m)));
    if truncated {
        issues.push(issue(path, "", "further schema issues omitted".into()));
    }
}

fn str_at<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn consistency(docs: &[Json], resources: &HashSet<String>, issues: &mut Vec<Issue>) {
    let find = |k: &DocKind| docs.iter().find(|d| &d.kind == k);
    for (kind, path) in [
        (DocKind::Version, paths::VERSION),
        (DocKind::Report, paths::REPORT),
        (DocKind::Pages, paths::PAGES),
    ] {
        if find(&kind).is_none() {
            issues.push(issue(path, "", "required document is missing".into()));
        }
    }

    // Pages: folder → name.
    let mut pages: BTreeMap<&str, &str> = BTreeMap::new();
    let mut page_names: HashMap<&str, &str> = HashMap::new();
    for d in docs {
        if let DocKind::Page { page } = &d.kind {
            let name = str_at(&d.value, "name").unwrap_or(page);
            if page_names.insert(name, &d.path).is_some() {
                issues.push(issue(&d.path, "/name", format!("duplicate page name `{name}`")));
            }
            name_issues(issues, &d.path, "page", page, name);
            pages.insert(page, name);
        }
    }
    if pages.is_empty() {
        issues.push(issue(paths::PAGES, "", "the report has no page".into()));
    }
    if let Some(meta) = find(&DocKind::Pages) {
        if let Some(order) = meta.value.get("pageOrder").and_then(Value::as_array) {
            let mut listed = HashSet::new();
            for (i, entry) in order.iter().enumerate() {
                let Some(name) = entry.as_str() else { continue };
                if !page_names.contains_key(name) {
                    issues.push(issue(
                        &meta.path,
                        &format!("/pageOrder/{i}"),
                        format!("page `{name}` does not exist"),
                    ));
                }
                if !listed.insert(name) {
                    issues.push(issue(
                        &meta.path,
                        &format!("/pageOrder/{i}"),
                        format!("page `{name}` is listed twice"),
                    ));
                }
            }
            // Pages missing from `pageOrder` are fine: Power BI Desktop 2.158 opens the report
            // with them after the listed ones and adds them to `pageOrder` on save.
        }
        if let Some(active) = str_at(&meta.value, "activePageName")
            && !page_names.contains_key(active)
        {
            issues.push(issue(
                &meta.path,
                "/activePageName",
                format!("page `{active}` does not exist"),
            ));
        }
    }

    // Visuals per page folder.
    let mut visuals: BTreeMap<&str, Vec<(&str, &Json)>> = BTreeMap::new();
    for d in docs {
        if let DocKind::Visual { page, visual } = &d.kind {
            if !pages.contains_key(page.as_str()) {
                issues.push(issue(
                    &d.path,
                    "",
                    format!("page folder `{page}` has no page.json"),
                ));
            }
            let name = str_at(&d.value, "name").unwrap_or(visual);
            name_issues(issues, &d.path, "visual", visual, name);
            visuals.entry(page).or_default().push((name, d));
        }
    }
    for list in visuals.values() {
        let mut names = HashSet::new();
        for (name, d) in list {
            if !names.insert(*name) {
                issues.push(issue(
                    &d.path,
                    "/name",
                    format!("duplicate visual name `{name}` on this page"),
                ));
            }
        }
        for (_, d) in list {
            if let Some(parent) = str_at(&d.value, "parentGroupName") {
                let group = list.iter().find(|(n, _)| *n == parent);
                match group {
                    None => issues.push(issue(
                        &d.path,
                        "/parentGroupName",
                        format!("group `{parent}` does not exist on this page"),
                    )),
                    Some((_, g)) if g.value.get("visualGroup").is_none() => issues.push(issue(
                        &d.path,
                        "/parentGroupName",
                        format!("`{parent}` is not a visual group"),
                    )),
                    _ => {}
                }
            }
        }
    }
    for d in docs {
        if let DocKind::Mobile { page, visual } = &d.kind {
            let has_visual = docs.iter().any(
                |v| matches!(&v.kind, DocKind::Visual { page: p, visual: w } if p == page && w == visual),
            );
            if !has_visual {
                issues.push(issue(&d.path, "", "mobile layout without a visual.json".into()));
            }
        }
    }

    // Registered and shared resources.
    if let Some(report) = find(&DocKind::Report) {
        let packages = report.value.get("resourcePackages").and_then(Value::as_array);
        for (i, package) in packages.into_iter().flatten().enumerate() {
            let (Some(name), Some(kind)) = (str_at(package, "name"), str_at(package, "type")) else {
                continue;
            };
            // Custom visual items resolve under the visual's `resources/` folder (Desktop
            // 2.158; pbix-package.md §2, "Custom visuals").
            let folder = match kind {
                "RegisteredResources" | "SharedResources" => format!("StaticResources/{name}"),
                "CustomVisual" => format!("CustomVisuals/{name}/resources"),
                _ => continue,
            };
            let items = package.get("items").and_then(Value::as_array);
            for (j, item) in items.into_iter().flatten().enumerate() {
                let Some(p) = str_at(item, "path") else { continue };
                let file = format!("{folder}/{}", paths::normalize(p));
                if !resources.contains(&file) {
                    issues.push(issue(
                        &report.path,
                        &format!("/resourcePackages/{i}/items/{j}/path"),
                        format!("resource file `{file}` is missing"),
                    ));
                }
            }
        }
    }

    // Bookmarks.
    let bookmark_names: HashSet<&str> = docs
        .iter()
        .filter_map(|d| match &d.kind {
            DocKind::Bookmark { stem } => Some(str_at(&d.value, "name").unwrap_or(stem)),
            _ => None,
        })
        .collect();
    match find(&DocKind::Bookmarks) {
        None if !bookmark_names.is_empty() => {
            issues.push(issue(
                paths::BOOKMARKS,
                "",
                "bookmark files exist but bookmarks.json is missing".into(),
            ));
        }
        None => {}
        Some(meta) => {
            let items = meta.value.get("items").and_then(Value::as_array);
            for (i, item) in items.into_iter().flatten().enumerate() {
                if let Some(children) = item.get("children").and_then(Value::as_array) {
                    for (j, c) in children.iter().enumerate() {
                        if let Some(n) = c.as_str()
                            && !bookmark_names.contains(n)
                        {
                            issues.push(issue(
                                &meta.path,
                                &format!("/items/{i}/children/{j}"),
                                format!("bookmark `{n}` does not exist"),
                            ));
                        }
                    }
                } else if let Some(n) = str_at(item, "name")
                    && !bookmark_names.contains(n)
                {
                    issues.push(issue(
                        &meta.path,
                        &format!("/items/{i}/name"),
                        format!("bookmark `{n}` does not exist"),
                    ));
                }
            }
        }
    }
}
