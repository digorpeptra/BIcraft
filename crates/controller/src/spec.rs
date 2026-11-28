//! The `BicraftWorkspace` resource: its spec, checked against the admin's configuration, and the
//! derived names.

use std::time::Duration;

use bicraft_auth::acl::{Access, Grant, Subject, Workspaces};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{Config, EgressProfile, Reclaim, SizeClass, dns_label, dns_subdomain, quantity_bytes};

/// Longest workspace name: `bws-<name>-recovery` and the label values stay within 63 characters.
pub const MAX_NAME: usize = 40;
/// Most grants on one workspace.
pub const MAX_GRANTS: usize = 256;

/// `spec.volume`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VolumeSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reclaim_policy: Option<Reclaim>,
}

impl Serialize for Reclaim {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

/// `spec` as written by a user or the API.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_class: Option<String>,
    #[serde(default)]
    pub volume: VolumeSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_timeout_seconds: Option<u64>,
    /// Subjects with editor access.
    #[serde(default)]
    pub grants: Vec<String>,
    /// Subjects with reader access (read-only commands and file downloads, docs/auth.md "Roles").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub readers: Vec<String>,
    #[serde(default)]
    pub egress: Vec<String>,
    #[serde(default)]
    pub suspend: bool,
}

/// A workspace whose spec passed the checks: everything needed to render its objects.
#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    pub name: String,
    pub uid: String,
    pub generation: i64,
    pub class_name: String,
    pub class: SizeClass,
    pub volume_size: String,
    pub storage_class: String,
    pub reclaim: Reclaim,
    /// `None`: never stopped for idleness.
    pub idle_timeout: Option<Duration>,
    pub grants: Vec<Grant>,
    pub egress: Vec<(String, EgressProfile)>,
    pub suspend: bool,
}

/// Names derived from a workspace's name.
pub fn object_name(ws: &str) -> String {
    format!("bws-{ws}")
}
pub fn claim_name(ws: &str) -> String {
    format!("bws-{ws}-data")
}
/// The claim of the workspace's recovery journals (docs/recovery.md).
pub fn recovery_claim_name(ws: &str) -> String {
    format!("bws-{ws}-recovery")
}

/// Whether `name` can be a workspace name (and so a workspace id of `bicraft-cli serve`).
pub fn valid_name(name: &str) -> bool {
    dns_label(name, MAX_NAME)
}

/// Checks `spec` of workspace `name` against the configuration.
pub fn resolve(
    cfg: &Config,
    name: &str,
    uid: &str,
    generation: i64,
    spec: &WorkspaceSpec,
) -> Result<Workspace, String> {
    if !valid_name(name) {
        return Err(format!(
            "name `{name}`: 1 to {MAX_NAME} lowercase letters, digits and `-`, not at either end"
        ));
    }
    let class_name = spec
        .size_class
        .clone()
        .unwrap_or_else(|| cfg.default_class.clone());
    let class = cfg.classes.get(&class_name).cloned().ok_or_else(|| {
        format!(
            "sizeClass `{class_name}` is not defined (classes: {})",
            cfg.classes.keys().cloned().collect::<Vec<_>>().join(", ")
        )
    })?;
    let volume_size = spec
        .volume
        .size
        .clone()
        .unwrap_or_else(|| class.volume_size.clone());
    let bytes = quantity_bytes(&volume_size)
        .ok_or_else(|| format!("volume.size `{volume_size}` is not a quantity (e.g. 20Gi)"))?;
    if bytes == 0 || Some(bytes) > quantity_bytes(&class.max_volume_size) {
        return Err(format!(
            "volume.size {volume_size} is above the {} of class `{class_name}`",
            class.max_volume_size
        ));
    }
    let storage_class = spec
        .volume
        .storage_class
        .clone()
        .unwrap_or_else(|| cfg.storage_class.clone());
    if !storage_class.is_empty() && !dns_subdomain(&storage_class) {
        return Err(format!("volume.storageClass `{storage_class}`"));
    }
    let reclaim = spec.volume.reclaim_policy.unwrap_or(cfg.default_reclaim_policy);
    let idle = spec
        .idle_timeout_seconds
        .unwrap_or(cfg.default_idle_timeout_seconds);
    if idle > 30 * 86_400 {
        return Err("idleTimeoutSeconds above 30 days".into());
    }
    if spec.grants.len().saturating_add(spec.readers.len()) > MAX_GRANTS {
        return Err(format!("more than {MAX_GRANTS} grants"));
    }
    let mut grants = Vec::new();
    for (list, access, what) in [
        (&spec.grants, Access::Editor, "grants"),
        (&spec.readers, Access::Reader, "readers"),
    ] {
        for g in list {
            let subject = Subject::parse(g).map_err(|e| format!("{what}: {e}"))?;
            grants.push(Grant {
                subject,
                workspaces: Workspaces::List(vec![name.to_string()]),
                access,
            });
        }
    }
    let mut egress = Vec::new();
    for e in &spec.egress {
        let p = cfg.egress_profiles.get(e).ok_or_else(|| {
            format!(
                "egress profile `{e}` is not defined (profiles: {})",
                cfg.egress_profiles.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        if !egress.iter().any(|(n, _)| n == e) {
            egress.push((e.clone(), p.clone()));
        }
    }
    Ok(Workspace {
        name: name.to_string(),
        uid: uid.to_string(),
        generation,
        class_name,
        class,
        volume_size,
        storage_class,
        reclaim,
        idle_timeout: (idle > 0).then(|| Duration::from_secs(idle)),
        grants,
        egress,
        suspend: spec.suspend,
    })
}

/// The pieces of a `BicraftWorkspace` object the controller reads.
#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    pub name: String,
    pub uid: String,
    pub generation: i64,
    pub spec: Result<WorkspaceSpec, String>,
    pub status: Value,
}

impl Object {
    pub fn from_json(v: &Value) -> Option<Object> {
        let meta = v.get("metadata")?;
        let name = meta.get("name")?.as_str()?.to_string();
        let uid = meta
            .get("uid")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let generation = meta.get("generation").and_then(Value::as_i64).unwrap_or(0);
        let spec = match v.get("spec") {
            None | Some(Value::Null) => Ok(WorkspaceSpec::default()),
            Some(s) => serde_json::from_value::<WorkspaceSpec>(s.clone()).map_err(|e| format!("spec: {e}")),
        };
        Some(Object {
            name,
            uid,
            generation,
            spec,
            status: v.get("status").cloned().unwrap_or(Value::Null),
        })
    }
}

/// RFC 3339 UTC time (seconds) of a Unix time.
pub fn rfc3339(unix: u64) -> String {
    let days = i64::try_from(unix / 86_400).unwrap_or(i64::MAX);
    let secs = unix % 86_400;
    let (y, m, d) = bicraft_locale::calendar::civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::sample;

    #[test]
    fn defaults_come_from_the_class() {
        let cfg = sample();
        let w = resolve(&cfg, "sales", "u1", 1, &WorkspaceSpec::default()).unwrap();
        assert_eq!(w.class_name, "small");
        assert_eq!(w.volume_size, "1Gi");
        assert_eq!(w.reclaim, Reclaim::Retain);
        assert_eq!(w.idle_timeout, Some(Duration::from_secs(900)));
        assert!(w.grants.is_empty());
    }

    #[test]
    fn spec_refusals() {
        let cfg = sample();
        let spec = |f: &dyn Fn(&mut WorkspaceSpec)| {
            let mut s = WorkspaceSpec::default();
            f(&mut s);
            resolve(&cfg, "w", "", 0, &s)
        };
        assert!(
            spec(&|s| s.size_class = Some("huge".into()))
                .unwrap_err()
                .contains("sizeClass")
        );
        assert!(
            spec(&|s| s.volume.size = Some("6Gi".into()))
                .unwrap_err()
                .contains("above")
        );
        assert!(spec(&|s| s.volume.size = Some("lots".into())).is_err());
        assert!(spec(&|s| s.grants = vec!["nobody".into()]).is_err());
        assert!(
            spec(&|s| s.egress = vec!["internet".into()])
                .unwrap_err()
                .contains("egress")
        );
        assert!(
            spec(&|s| s.idle_timeout_seconds = Some(0))
                .unwrap()
                .idle_timeout
                .is_none()
        );
        assert!(resolve(&cfg, "Bad_Name", "", 0, &WorkspaceSpec::default()).is_err());
        assert!(resolve(&cfg, &"a".repeat(41), "", 0, &WorkspaceSpec::default()).is_err());
    }

    #[test]
    fn readers_get_reader_access() {
        let spec = WorkspaceSpec {
            grants: vec!["token:agent".into()],
            readers: vec!["group:viewers".into()],
            ..WorkspaceSpec::default()
        };
        let w = resolve(&sample(), "w", "", 0, &spec).unwrap();
        assert_eq!(w.grants.len(), 2);
        assert_eq!(w.grants[0].access, Access::Editor);
        assert_eq!(w.grants[1].access, Access::Reader);
        let bad = WorkspaceSpec {
            readers: vec!["nobody".into()],
            ..WorkspaceSpec::default()
        };
        assert!(
            resolve(&sample(), "w", "", 0, &bad)
                .unwrap_err()
                .contains("readers")
        );
    }

    #[test]
    fn objects_parse_and_unknown_fields_are_reported() {
        let o = Object::from_json(&serde_json::json!({
            "metadata": {"name": "a", "uid": "u", "generation": 3},
            "spec": {"sizeClass": "large", "volume": {"reclaimPolicy": "Delete"}}
        }))
        .unwrap();
        let spec = o.spec.unwrap();
        assert_eq!(spec.volume.reclaim_policy, Some(Reclaim::Delete));
        let bad = Object::from_json(&serde_json::json!({"metadata": {"name": "a"}, "spec": {"memory": 1}}))
            .unwrap();
        assert!(bad.spec.is_err());
        assert!(Object::from_json(&serde_json::json!({})).is_none());
    }

    #[test]
    fn times() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_793_059_200), "2026-10-27T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        // year boundaries, a leap day's last second, a century that is not leap (G37)
        assert_eq!(rfc3339(94_694_399), "1972-12-31T23:59:59Z");
        assert_eq!(rfc3339(946_684_799), "1999-12-31T23:59:59Z");
        assert_eq!(rfc3339(946_684_800), "2000-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_709_251_199), "2024-02-29T23:59:59Z");
        assert_eq!(rfc3339(4_107_499_200), "2100-02-28T12:00:00Z");
        assert_eq!(rfc3339(4_107_542_400), "2100-03-01T00:00:00Z");
    }
}
