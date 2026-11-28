//! Dependency layering rules (see the architecture table in AGENTS.md).
//!
//! The rule engine is a pure function over a small, metadata-independent
//! model so it can be unit-tested; `from_metadata` builds that model from
//! `cargo metadata --format-version 1 --no-deps`.

use serde_json::Value;

/// Where a workspace crate sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Layered crate (L0..L6 libraries, L7 apps).
    Layer(u8),
    /// Below L0: a leaf helper with no workspace dependency that every layered crate may use
    /// (`bicraft-env`, the environment-variable reader of the rename's compatibility window).
    Foundation,
    /// Build tooling (`xtask`): exempt from the rules.
    Exempt,
}

/// The layering table, by package name.
pub const TABLE: &[(&str, Class)] = &[
    ("bicraft-env", Class::Foundation),
    ("bicraft-mem", Class::Foundation),
    ("bicraft-opc", Class::Layer(0)),
    ("bicraft-locale", Class::Layer(0)),
    ("bicraft-xpress9", Class::Layer(0)),
    // the Plain LZ77 codec of [MS-XCA] (Excel Power Pivot backups)
    ("bicraft-xpress8", Class::Layer(0)),
    ("bicraft-sqlite", Class::Layer(0)),
    ("bicraft-abf", Class::Layer(0)),
    ("bicraft-vertipaq", Class::Layer(0)),
    ("bicraft-scan", Class::Layer(0)),
    ("bicraft-versions", Class::Layer(0)),
    // M language services: the catalogue of the functions the evaluator runs, completion, hover
    // and signature help (the engine's m.* commands and the UI's Power Query editor share it)
    ("bicraft-mlang", Class::Layer(0)),
    // a custom visual's TypeScript compiled and bundled with swc (native only: the engine
    // uses it, and the browser app never builds the engine)
    ("bicraft-tsbuild", Class::Layer(0)),
    // std HTTP/1.1 plumbing shared by the viewer and the MCP HTTP endpoint (both L6)
    ("bicraft-http", Class::Layer(0)),
    // authentication of `bicraft-cli serve` (OIDC, trusted header, API tokens) and the workspace ACL
    ("bicraft-auth", Class::Layer(0)),
    ("bicraft-model", Class::Layer(1)),
    ("bicraft-report", Class::Layer(1)),
    ("bicraft-visuals", Class::Layer(1)),
    ("bicraft-dax", Class::Layer(2)),
    ("bicraft-data", Class::Layer(2)),
    ("bicraft-databricks", Class::Layer(3)),
    ("bicraft-sqlserver", Class::Layer(3)),
    ("bicraft-webdata", Class::Layer(3)),
    ("bicraft-diff", Class::Layer(3)),
    ("bicraft-tmsl", Class::Layer(3)),
    ("bicraft-tmdl", Class::Layer(3)),
    ("bicraft-metadata", Class::Layer(3)),
    // Excel Power Pivot workbooks: the backup in xl/model/item.data, its XML metadata and data
    ("bicraft-powerpivot", Class::Layer(3)),
    ("bicraft-pbir", Class::Layer(3)),
    ("bicraft-layout", Class::Layer(3)),
    ("bicraft-pbix", Class::Layer(4)),
    ("bicraft-pbip", Class::Layer(4)),
    ("bicraft-pbixwriter", Class::Layer(4)),
    ("bicraft-bpa", Class::Layer(4)),
    ("bicraft-render", Class::Layer(4)),
    ("bicraft-engine", Class::Layer(5)),
    ("bicraft-mcp", Class::Layer(6)),
    // the viewer page and its loopback server: no engine dependency, so the MCP HTTP endpoint (L6)
    // serves the same page in server mode
    ("bicraft-view", Class::Layer(5)),
    // the workspace controller of a Kubernetes deployment (reconciler + gateway); no engine
    ("bicraft-controller", Class::Layer(6)),
    // the app's UI (egui over bicraft-render), shared by the desktop and the browser shells: no
    // engine, threads or files (it builds for wasm32); the engine only for its tests
    ("bicraft-ui", Class::Layer(6)),
    ("bicraft-cli", Class::Layer(7)),
    // the native shell of the UI (egui/wgpu, worker threads, the engine, the blocking client)
    ("bicraft-desktop", Class::Layer(7)),
    // the browser shell of the UI (wasm32; its native build only says so): docs/webui.md
    ("bicraft-web", Class::Layer(8)),
    ("xtask", Class::Exempt),
];

/// External crates that are async runtimes. Only L5+ may use them.
pub const ASYNC_RUNTIMES: &[&str] = &[
    "tokio",
    "async-std",
    "smol",
    "async-executor",
    "futures-executor",
    "actix-rt",
];

/// The engine crate. Only L6+ may depend on it (also implied by the layer rule;
/// kept explicit so the error message names the real problem).
pub const ENGINE: &str = "bicraft-engine";

/// Crates that may use the engine in their tests only: the shared UI builds for wasm32, where the
/// engine's session (threads, files) has no place; its shells bring the sources.
pub const ENGINE_DEV_ONLY: &[&str] = &["bicraft-ui"];

/// First layer allowed to use an async runtime.
pub const ASYNC_MIN_LAYER: u8 = 5;

pub fn classify(pkg: &str) -> Option<Class> {
    TABLE.iter().find(|(n, _)| *n == pkg).map(|(_, c)| *c)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}

#[derive(Debug, Clone)]
pub struct Dep {
    pub name: String,
    pub kind: DepKind,
    /// `true` if the dependency is a workspace member (or a path dependency).
    pub workspace: bool,
}

#[derive(Debug, Clone)]
pub struct Crate {
    pub name: String,
    pub deps: Vec<Dep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    Unregistered {
        krate: String,
    },
    Upward {
        krate: String,
        dep: String,
        from: u8,
        to: u8,
        kind: DepKind,
    },
    EngineBelowL6 {
        krate: String,
        layer: u8,
        kind: DepKind,
    },
    EngineNotDevOnly {
        krate: String,
        kind: DepKind,
    },
    AsyncBelowL5 {
        krate: String,
        dep: String,
        layer: u8,
    },
    FoundationDep {
        krate: String,
        dep: String,
        kind: DepKind,
    },
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Violation::Unregistered { krate } => write!(
                f,
                "{krate}: unknown workspace crate; register it in xtask/src/layers.rs TABLE \
                 and in the AGENTS.md layer table"
            ),
            Violation::Upward {
                krate,
                dep,
                from,
                to,
                kind,
            } => write!(
                f,
                "{krate} (L{from}) -> {dep} (L{to}) [{kind:?}]: may only depend on strictly lower layers"
            ),
            Violation::FoundationDep { krate, dep, kind } => write!(
                f,
                "{krate} (foundation) -> {dep} [{kind:?}]: a foundation crate depends on no workspace crate"
            ),
            Violation::EngineBelowL6 { krate, layer, kind } => write!(
                f,
                "{krate} (L{layer}) -> {ENGINE} [{kind:?}]: only L6+ (mcp, apps) may depend on the engine"
            ),
            Violation::EngineNotDevOnly { krate, kind } => write!(
                f,
                "{krate} -> {ENGINE} [{kind:?}]: {krate} builds for wasm32 and may use the engine in tests only"
            ),
            Violation::AsyncBelowL5 { krate, dep, layer } => write!(
                f,
                "{krate} (L{layer}) depends on async runtime `{dep}`; async runtimes are only allowed in L5+"
            ),
        }
    }
}

/// Check all rules. Returns violations sorted for stable output.
pub fn check(crates: &[Crate]) -> Vec<Violation> {
    let mut out = Vec::new();
    for c in crates {
        let layer = match classify(&c.name) {
            None => {
                out.push(Violation::Unregistered {
                    krate: c.name.clone(),
                });
                continue;
            }
            Some(Class::Exempt) => continue,
            Some(Class::Foundation) => {
                for d in c.deps.iter().filter(|d| d.workspace && d.name != c.name) {
                    out.push(Violation::FoundationDep {
                        krate: c.name.clone(),
                        dep: d.name.clone(),
                        kind: d.kind,
                    });
                }
                continue;
            }
            Some(Class::Layer(l)) => l,
        };
        for d in &c.deps {
            // Self dev-dependencies (e.g. to enable features in tests) are fine.
            if d.name == c.name {
                continue;
            }
            if d.workspace {
                if d.name == ENGINE && layer < 6 {
                    out.push(Violation::EngineBelowL6 {
                        krate: c.name.clone(),
                        layer,
                        kind: d.kind,
                    });
                    continue;
                }
                if d.name == ENGINE && d.kind != DepKind::Dev && ENGINE_DEV_ONLY.contains(&c.name.as_str()) {
                    out.push(Violation::EngineNotDevOnly {
                        krate: c.name.clone(),
                        kind: d.kind,
                    });
                    continue;
                }
                let to = match classify(&d.name) {
                    // Unregistered deps are reported on their own entry; a
                    // non-member path dependency has no entry and is ignored.
                    None => continue,
                    // Nothing may depend on tooling.
                    Some(Class::Exempt) => u8::MAX,
                    // Below every layer.
                    Some(Class::Foundation) => continue,
                    Some(Class::Layer(to)) => to,
                };
                if to >= layer {
                    out.push(Violation::Upward {
                        krate: c.name.clone(),
                        dep: d.name.clone(),
                        from: layer,
                        to,
                        kind: d.kind,
                    });
                }
            } else if layer < ASYNC_MIN_LAYER && ASYNC_RUNTIMES.contains(&d.name.as_str()) {
                out.push(Violation::AsyncBelowL5 {
                    krate: c.name.clone(),
                    dep: d.name.clone(),
                    layer,
                });
            }
        }
    }
    out.sort_by_key(|v| v.to_string());
    out.dedup();
    out
}

/// Build the model from `cargo metadata --format-version 1 --no-deps`.
pub fn from_metadata(meta: &Value) -> Result<Vec<Crate>, String> {
    let pkgs = meta
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("cargo metadata: no `packages` array")?;
    let members: Vec<&str> = pkgs
        .iter()
        .filter_map(|p| p.get("name").and_then(Value::as_str))
        .collect();
    let mut out = Vec::new();
    for p in pkgs {
        let name = p
            .get("name")
            .and_then(Value::as_str)
            .ok_or("cargo metadata: package without a name")?
            .to_owned();
        let mut deps = Vec::new();
        for d in p
            .get("dependencies")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(dname) = d.get("name").and_then(Value::as_str) else {
                return Err(format!("cargo metadata: {name} has a dependency without a name"));
            };
            let kind = match d.get("kind").and_then(Value::as_str) {
                Some("dev") => DepKind::Dev,
                Some("build") => DepKind::Build,
                _ => DepKind::Normal,
            };
            let workspace = members.contains(&dname) || d.get("path").is_some_and(Value::is_string);
            deps.push(Dep {
                name: dname.to_owned(),
                kind,
                workspace,
            });
        }
        out.push(Crate { name, deps });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

pub fn describe(class: Option<Class>) -> String {
    match class {
        Some(Class::Layer(7)) => "L7 (app)".into(),
        Some(Class::Layer(l)) => format!("L{l}"),
        Some(Class::Foundation) => "foundation (below L0)".into(),
        Some(Class::Exempt) => "exempt".into(),
        None => "UNREGISTERED".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use DepKind::*;

    fn c(name: &str, deps: &[(&str, DepKind, bool)]) -> Crate {
        Crate {
            name: name.into(),
            deps: deps
                .iter()
                .map(|(n, k, w)| Dep {
                    name: (*n).into(),
                    kind: *k,
                    workspace: *w,
                })
                .collect(),
        }
    }

    #[test]
    fn clean_downward_graph_passes() {
        let g = [
            c("bicraft-opc", &[("zip", Normal, false)]),
            c("bicraft-model", &[("serde", Normal, false)]),
            c("bicraft-dax", &[("bicraft-model", Normal, true)]),
            c(
                "bicraft-tmsl",
                &[("bicraft-model", Normal, true), ("bicraft-dax", Dev, true)],
            ),
            c(
                "bicraft-pbix",
                &[("bicraft-opc", Normal, true), ("bicraft-tmsl", Normal, true)],
            ),
            c(
                "bicraft-engine",
                &[("bicraft-pbix", Normal, true), ("tokio", Normal, false)],
            ),
            c("bicraft-mcp", &[("bicraft-engine", Normal, true)]),
            c(
                "bicraft-cli",
                &[("bicraft-mcp", Normal, true), ("bicraft-engine", Normal, true)],
            ),
        ];
        assert!(check(&g).is_empty(), "{:?}", check(&g));
    }

    #[test]
    fn upward_dependency_flagged() {
        let v = check(&[c("bicraft-model", &[("bicraft-dax", Normal, true)])]);
        assert!(
            matches!(v[..], [Violation::Upward { from: 1, to: 2, .. }]),
            "{v:?}"
        );
    }

    #[test]
    fn sideways_dependency_flagged() {
        let v = check(&[c("bicraft-tmsl", &[("bicraft-tmdl", Normal, true)])]);
        assert!(
            matches!(v[..], [Violation::Upward { from: 3, to: 3, .. }]),
            "{v:?}"
        );
        let v = check(&[c("bicraft-pbip", &[("bicraft-pbix", Normal, true)])]);
        assert!(
            matches!(v[..], [Violation::Upward { from: 4, to: 4, .. }]),
            "{v:?}"
        );
    }

    #[test]
    fn upward_build_and_dev_dependencies_flagged() {
        let v = check(&[c("bicraft-opc", &[("bicraft-model", Build, true)])]);
        assert!(matches!(v[..], [Violation::Upward { kind: Build, .. }]), "{v:?}");
        let v = check(&[c("bicraft-dax", &[("bicraft-tmdl", Dev, true)])]);
        assert!(matches!(v[..], [Violation::Upward { kind: Dev, .. }]), "{v:?}");
    }

    #[test]
    fn the_ui_uses_the_engine_in_tests_only() {
        assert!(check(&[c("bicraft-ui", &[("bicraft-engine", Dev, true)])]).is_empty());
        for kind in [Normal, Build] {
            let v = check(&[c("bicraft-ui", &[("bicraft-engine", kind, true)])]);
            assert!(matches!(v[..], [Violation::EngineNotDevOnly { .. }]), "{v:?}");
        }
    }

    #[test]
    fn engine_below_l6_flagged() {
        for (krate, layer) in [("bicraft-pbip", 4), ("bicraft-model", 1)] {
            for kind in [Normal, Dev] {
                let v = check(&[c(krate, &[("bicraft-engine", kind, true)])]);
                assert!(
                    matches!(v[..], [Violation::EngineBelowL6 { layer: l, .. }] if l == layer),
                    "{krate}: {v:?}"
                );
                assert!(v[0].to_string().contains("engine"));
            }
        }
        assert!(check(&[c("bicraft-mcp", &[("bicraft-engine", Normal, true)])]).is_empty());
    }

    #[test]
    fn async_runtime_below_l5_flagged() {
        for dep in ASYNC_RUNTIMES {
            let v = check(&[c("bicraft-pbix", &[(dep, Normal, false)])]);
            assert!(
                matches!(v[..], [Violation::AsyncBelowL5 { layer: 4, .. }]),
                "{dep}: {v:?}"
            );
        }
        assert!(check(&[c("bicraft-engine", &[("tokio", Normal, false)])]).is_empty());
        assert!(check(&[c("bicraft-mcp", &[("tokio", Normal, false)])]).is_empty());
        assert!(check(&[c("bicraft-dax", &[("tokio-util-like", Normal, false)])]).is_empty());
    }

    #[test]
    fn self_dev_dependency_ignored() {
        assert!(check(&[c("bicraft-model", &[("bicraft-model", Dev, true)])]).is_empty());
    }

    #[test]
    fn foundation_usable_by_every_layer_and_depends_on_nothing() {
        let ok = [
            c("bicraft-env", &[]),
            c("bicraft-opc", &[("bicraft-env", Dev, true)]),
            c("bicraft-scan", &[("bicraft-env", Normal, true)]),
            c("bicraft-cli", &[("bicraft-env", Build, true)]),
        ];
        assert!(check(&ok).is_empty(), "{:?}", check(&ok));
        let v = check(&[c("bicraft-env", &[("bicraft-opc", Normal, true)])]);
        assert!(
            matches!(&v[..], [Violation::FoundationDep { krate, dep, .. }] if krate == "bicraft-env" && dep == "bicraft-opc")
        );
    }

    #[test]
    fn unregistered_crate_is_error() {
        let v = check(&[c("bicraft-mystery", &[])]);
        assert!(matches!(&v[..], [Violation::Unregistered { krate }] if krate == "bicraft-mystery"));
        assert!(v[0].to_string().contains("register"));
    }

    #[test]
    fn depending_on_xtask_flagged_but_xtask_exempt() {
        assert!(!check(&[c("bicraft-cli", &[("xtask", Normal, true)])]).is_empty());
        assert!(
            check(&[c(
                "xtask",
                &[("bicraft-cli", Normal, true), ("tokio", Normal, false)]
            )])
            .is_empty()
        );
    }

    #[test]
    fn non_member_path_dependency_ignored() {
        assert!(check(&[c("bicraft-opc", &[("vendored-thing", Normal, true)])]).is_empty());
    }

    #[test]
    fn metadata_parsing() {
        let meta: Value = serde_json::from_str(
            r#"{"packages":[
                {"name":"bicraft-dax","dependencies":[
                    {"name":"bicraft-model","kind":null,"path":"/x/crates/model"},
                    {"name":"serde","kind":null},
                    {"name":"proptest","kind":"dev"},
                    {"name":"cc","kind":"build"}]},
                {"name":"bicraft-model","dependencies":[]}
            ]}"#,
        )
        .unwrap();
        let g = from_metadata(&meta).unwrap();
        assert_eq!(g.len(), 2);
        let dax = g.iter().find(|c| c.name == "bicraft-dax").unwrap();
        assert!(dax.deps[0].workspace && !dax.deps[1].workspace);
        assert_eq!(dax.deps[2].kind, Dev);
        assert_eq!(dax.deps[3].kind, Build);
        assert!(check(&g).is_empty());
    }

    #[test]
    fn metadata_errors_do_not_panic() {
        assert!(from_metadata(&serde_json::json!({})).is_err());
        assert!(from_metadata(&serde_json::json!({"packages":[{"dependencies":[]}]})).is_err());
        assert!(from_metadata(&serde_json::json!({"packages":[{"name":"x","dependencies":[{}]}]})).is_err());
    }
}
