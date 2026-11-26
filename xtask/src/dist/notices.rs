//! `THIRD_PARTY_CRATES.md`: the Rust crates compiled into `bicraft`, from `cargo metadata`.
//!
//! The crate set is the resolved dependency graph of `bicraft-cli` for one target (normal and
//! build edges, no dev edges), as locked in `Cargo.lock`. Each crate's license files are read
//! from its source folder (the cargo registry) and identical texts are printed once.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use serde_json::Value;

/// Largest license file read; bigger files are cut (none comes close).
const MAX_LICENSE_BYTES: u64 = 256 * 1024;

/// One third-party crate.
#[derive(Debug, PartialEq)]
pub struct Krate {
    pub name: String,
    pub version: String,
    pub license: String,
    pub repository: String,
    /// `(file name, text)` of each license file shipped in the crate.
    pub files: Vec<(String, String)>,
}

/// The third-party crates reachable from `root_name` in a full `cargo metadata` document.
pub fn crates(meta: &Value, root_name: &str) -> Result<Vec<Krate>, String> {
    let packages = meta
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("cargo metadata: no packages")?;
    let by_id: HashMap<&str, &Value> = packages
        .iter()
        .filter_map(|p| Some((p.get("id")?.as_str()?, p)))
        .collect();
    let root = packages
        .iter()
        .find(|p| p.get("name").and_then(Value::as_str) == Some(root_name))
        .and_then(|p| p.get("id")?.as_str())
        .ok_or_else(|| format!("cargo metadata: no package {root_name}"))?;
    let nodes: HashMap<&str, &Value> = meta
        .pointer("/resolve/nodes")
        .and_then(Value::as_array)
        .ok_or("cargo metadata: no resolve graph (ran with --no-deps?)")?
        .iter()
        .filter_map(|n| Some((n.get("id")?.as_str()?, n)))
        .collect();

    let mut seen = BTreeSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let deps = nodes
            .get(id)
            .and_then(|n| n.get("deps"))
            .and_then(Value::as_array);
        for d in deps.into_iter().flatten() {
            let shipped = d.get("dep_kinds").and_then(Value::as_array).is_some_and(|ks| {
                ks.iter()
                    .any(|k| k.get("kind").and_then(Value::as_str) != Some("dev"))
            });
            if let (true, Some(pkg)) = (shipped, d.get("pkg").and_then(Value::as_str)) {
                stack.push(pkg);
            }
        }
    }

    let mut out = Vec::new();
    for id in seen {
        let Some(p) = by_id.get(id) else { continue };
        // Path (workspace) packages have no source: they are bicraft itself.
        if p.get("source").is_none_or(Value::is_null) {
            continue;
        }
        let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
        let mut license = s("license");
        if license.is_empty() && !s("license_file").is_empty() {
            license = format!("see {}", s("license_file"));
        }
        let dir = p
            .get("manifest_path")
            .and_then(Value::as_str)
            .map(Path::new)
            .and_then(Path::parent);
        let mut files = dir.map(license_files).unwrap_or_default();
        // crates that embed fonts keep the fonts' licences beside them (egui's default fonts:
        // `fonts/OFL.txt`, `fonts/UFL.txt`…)
        files.extend(dir.map(|d| font_licenses(&d.join("fonts"))).unwrap_or_default());
        out.push(Krate {
            name: s("name"),
            version: s("version"),
            license,
            repository: s("repository"),
            files,
        });
    }
    out.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    Ok(out)
}

/// The `.txt` licence files in a crate's `fonts/` folder, named `fonts/<file>`.
fn font_licenses(dir: &Path) -> Vec<(String, String)> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.to_ascii_lowercase().ends_with(".txt") || !e.path().is_file() {
            continue;
        }
        let mut buf = Vec::new();
        let read = std::fs::File::open(e.path()).and_then(|f| {
            use std::io::Read;
            f.take(MAX_LICENSE_BYTES).read_to_end(&mut buf)
        });
        if read.is_ok() {
            let text = String::from_utf8_lossy(&buf).replace("\r\n", "\n");
            out.push((format!("fonts/{name}"), text.trim_end().to_owned()));
        }
    }
    out.sort();
    out
}

/// License and notice files at the top of a crate folder, sorted by name.
fn license_files(dir: &Path) -> Vec<(String, String)> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let up = name.to_ascii_uppercase();
        let wanted = [
            "LICENSE",
            "LICENCE",
            "COPYING",
            "NOTICE",
            "UNLICENSE",
            "COPYRIGHT",
        ]
        .iter()
        .any(|p| up.starts_with(p));
        if !wanted || !e.path().is_file() {
            continue;
        }
        let mut buf = Vec::new();
        let read = std::fs::File::open(e.path()).and_then(|f| {
            use std::io::Read;
            f.take(MAX_LICENSE_BYTES).read_to_end(&mut buf)
        });
        if read.is_ok() {
            let text = String::from_utf8_lossy(&buf).replace("\r\n", "\n");
            out.push((name, text.trim_end().to_owned()));
        }
    }
    out.sort();
    out
}

/// The Markdown document.
pub fn render(crates: &[Krate], version: &str, triple: &str) -> String {
    let mut md = format!(
        "# Third-party Rust crates in bicraft {version} ({triple})\n\n\
         The `bicraft` binary is compiled from bicraft's own crates (MIT OR Apache-2.0) and the \
         {} third-party crates below, as locked in `Cargo.lock`. This file was generated by \
         `cargo xtask dist` from `cargo metadata`; the license texts are the files each crate \
         ships, printed once per distinct text. Bundled data files (XPress9 port, Selawik font, \
         JSON schemas, best-practice rules, CLDR data) are listed in `THIRD_PARTY_NOTICES.md` and \
         their licenses are under `licenses/`.\n\n",
        crates.len()
    );

    let mut by_license: BTreeMap<&str, usize> = BTreeMap::new();
    for c in crates {
        *by_license.entry(c.license.as_str()).or_default() += 1;
    }
    md.push_str("| License expression | Crates |\n|---|---|\n");
    for (l, n) in &by_license {
        md.push_str(&format!("| {} | {n} |\n", cell(l)));
    }

    // Distinct texts, numbered in first-use order.
    let mut texts: Vec<(&str, Vec<String>)> = Vec::new();
    let mut refs: Vec<Vec<usize>> = Vec::new();
    for c in crates {
        let mut mine = Vec::new();
        for (file, text) in &c.files {
            let i = match texts.iter().position(|(t, _)| *t == text.as_str()) {
                Some(i) => i,
                None => {
                    texts.push((text.as_str(), Vec::new()));
                    texts.len() - 1
                }
            };
            if let Some((_, users)) = texts.get_mut(i) {
                users.push(format!("{} {} ({file})", c.name, c.version));
            }
            mine.push(i + 1);
        }
        refs.push(mine);
    }

    md.push_str(
        "\n## Crates\n\n| Crate | Version | License | Repository | License texts |\n|---|---|---|---|---|\n",
    );
    for (c, mine) in crates.iter().zip(&refs) {
        let texts = if mine.is_empty() {
            "none shipped in the crate".to_owned()
        } else {
            mine.iter()
                .map(|i| format!("[{i}](#text-{i})"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        md.push_str(&format!(
            "| {} | {} | {} | {} | {texts} |\n",
            cell(&c.name),
            cell(&c.version),
            cell(&c.license),
            cell(&c.repository)
        ));
    }

    md.push_str("\n## License texts\n");
    for (i, (text, users)) in texts.iter().enumerate() {
        let n = i + 1;
        md.push_str(&format!(
            "\n<a id=\"text-{n}\"></a>\n### Text {n}\n\nUsed by: {}.\n\n````text\n{text}\n````\n",
            users.join(", ")
        ));
    }
    md
}

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn walks_normal_and_build_edges_only() {
        let meta = json!({
            "packages": [
                {"id": "cli", "name": "bicraft-cli", "version": "0.1.0", "source": null},
                {"id": "a", "name": "a", "version": "1.0.0", "source": "registry", "license": "MIT"},
                {"id": "b", "name": "b", "version": "2.0.0", "source": "registry", "license": "Apache-2.0"},
                {"id": "devonly", "name": "devonly", "version": "1.0.0", "source": "registry", "license": "MIT"},
                {"id": "lib", "name": "bicraft-lib", "version": "0.1.0", "source": null}
            ],
            "resolve": {"nodes": [
                {"id": "cli", "deps": [
                    {"pkg": "lib", "dep_kinds": [{"kind": null}]},
                    {"pkg": "devonly", "dep_kinds": [{"kind": "dev"}]}
                ]},
                {"id": "lib", "deps": [
                    {"pkg": "a", "dep_kinds": [{"kind": null}]},
                    {"pkg": "b", "dep_kinds": [{"kind": "build"}]}
                ]},
                {"id": "a", "deps": []}, {"id": "b", "deps": []}, {"id": "devonly", "deps": []}
            ]}
        });
        let got = crates(&meta, "bicraft-cli").unwrap();
        let names: Vec<&str> = got.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        assert!(crates(&meta, "missing").is_err());
        assert!(crates(&json!({"packages": []}), "bicraft-cli").is_err());
    }

    #[test]
    fn identical_texts_printed_once() {
        let k = |name: &str, text: &str| Krate {
            name: name.into(),
            version: "1.0.0".into(),
            license: "MIT".into(),
            repository: "https://example.invalid/x|y".into(),
            files: vec![("LICENSE-MIT".into(), text.into())],
        };
        let md = render(&[k("a", "same"), k("b", "same"), k("c", "other")], "0.1.0", "t");
        assert_eq!(md.matches("### Text").count(), 2);
        assert!(md.contains("Used by: a 1.0.0 (LICENSE-MIT), b 1.0.0 (LICENSE-MIT)."));
        assert!(md.contains("| MIT | 3 |"));
        assert!(md.contains("x\\|y"));
    }
}
