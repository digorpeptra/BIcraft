//! The controller's configuration: the admin-defined size classes, egress profiles, defaults and
//! the workspace pod settings (a JSON file, usually a `ConfigMap` written by the Helm chart).
//!
//! Model sizes are an admin setting (docs/web-deployment.md, decision 5): a workspace names a
//! class, never raw memory or CPU.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

/// The default custom resource group. A placeholder until the project's domain is chosen: the
/// group is configurable (`crdGroup`), and the chart renders the CRD with the same value.
pub const DEFAULT_GROUP: &str = "bicraft.dev";
/// The custom resource's version.
pub const VERSION: &str = "v1alpha1";
/// The custom resource's kind and plural.
pub const KIND: &str = "BicraftWorkspace";
pub const PLURAL: &str = "bicraftworkspaces";
/// Field manager of every server-side apply, and the `app.kubernetes.io/managed-by` label.
pub const MANAGER: &str = "bicraft-controller";
/// The pod margin above `BICRAFT_MEMORY_LIMIT` (docs/beyond-ram.md, lot 5).
pub const POD_MARGIN_MIB: u64 = 64;
/// Identity headers the gateway sets on forwarded requests (the workspace pods trust them from the
/// gateway's address only; any incoming copy is removed).
pub const USER_HEADER: &str = "X-Bicraft-Gateway-User";
pub const EMAIL_HEADER: &str = "X-Bicraft-Gateway-Email";
pub const GROUPS_HEADER: &str = "X-Bicraft-Gateway-Groups";
/// The group the gateway adds for the access it granted; the workspace pod maps it to editor or
/// reader access. Identity groups with this prefix are dropped by the gateway (no escalation through
/// a claim or a proxy header).
pub const ACCESS_GROUP_PREFIX: &str = "bicraft-gateway-";
pub const EDITOR_GROUP: &str = "bicraft-gateway-editor";
pub const READER_GROUP: &str = "bicraft-gateway-reader";
/// Largest configuration file.
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// What happens to a workspace's volume when the workspace is deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Reclaim {
    /// The PVC outlives the workspace (no owner reference); a workspace created again under the
    /// same name adopts it.
    Retain,
    /// The PVC is owned by the workspace and deleted with it by the garbage collector.
    Delete,
}

impl Reclaim {
    pub fn as_str(self) -> &'static str {
        match self {
            Reclaim::Retain => "Retain",
            Reclaim::Delete => "Delete",
        }
    }
}

/// One size class.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SizeClass {
    /// `BICRAFT_MEMORY_LIMIT` in MiB; the pod limit is this + 64 MiB. 0 = no limit.
    #[serde(rename = "memoryLimitMiB")]
    pub memory_limit_mib: u64,
    /// `BICRAFT_MEMORY_BUDGET` in MiB; 0 = half of the limit (the server's default).
    #[serde(default, rename = "memoryBudgetMiB")]
    pub memory_budget_mib: u64,
    /// Memory request in MiB; 0 = the pod limit (Guaranteed QoS).
    #[serde(default, rename = "memoryRequestMiB")]
    pub memory_request_mib: u64,
    /// CPU limit in whole cores (also `BICRAFT_SCAN_THREADS`).
    pub cpu: u32,
    /// CPU request (a Kubernetes quantity); empty = the limit.
    #[serde(default)]
    pub cpu_request: String,
    #[serde(default = "yes")]
    pub aggregations: bool,
    /// The `/tmp` emptyDir size limit (spill files).
    pub spill_size: String,
    /// The default volume size.
    pub volume_size: String,
    /// The largest volume a workspace of this class may ask for.
    pub max_volume_size: String,
}

fn yes() -> bool {
    true
}

/// An egress profile: where workspace pods that name it may connect.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EgressProfile {
    #[serde(default)]
    pub cidrs: Vec<String>,
    #[serde(default = "https_port")]
    pub ports: Vec<u16>,
    /// `BICRAFT_SERVER_NETWORK=1` (the Databricks refresh).
    #[serde(default)]
    pub allow_network: bool,
    /// The Databricks token: a Secret in the controller's namespace.
    #[serde(default)]
    pub databricks_secret: Option<SecretKey>,
}

fn https_port() -> Vec<u16> {
    vec![443]
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretKey {
    pub name: String,
    pub key: String,
}

/// Settings of every workspace pod.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PodSettings {
    #[serde(default = "default_uid")]
    pub run_as_user: u32,
    #[serde(default = "default_uid")]
    pub run_as_group: u32,
    #[serde(default = "default_uid")]
    pub fs_group: u32,
    #[serde(default = "default_grace")]
    pub shutdown_grace_seconds: u32,
    #[serde(default = "default_upload")]
    pub max_upload_bytes: String,
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    #[serde(default)]
    pub extra_args: Vec<String>,
    /// Extra `env` entries (Kubernetes objects, copied as they are).
    #[serde(default)]
    pub extra_env: Vec<Value>,
    #[serde(default)]
    pub node_selector: BTreeMap<String, String>,
    #[serde(default)]
    pub tolerations: Vec<Value>,
    #[serde(default)]
    pub priority_class_name: String,
    #[serde(default)]
    pub image_pull_secrets: Vec<String>,
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
    /// `--max-views-per-user`, `--max-views`, `--view-idle` (document views, docs/mcp.md); 0: the
    /// server's defaults (8, 32, 1800 s).
    #[serde(default)]
    pub max_views_per_user: u32,
    #[serde(default)]
    pub max_views: u32,
    #[serde(default)]
    pub view_idle_seconds: u64,
    /// `--allow-autosave`: the app may save PBIP projects automatically.
    #[serde(default)]
    pub allow_autosave: bool,
    /// `--viewer-control` with its token from this Secret (`BICRAFT_VIEWER_CONTROL_TOKEN`), in the
    /// controller's namespace; `None`: no UI control channel.
    #[serde(default)]
    pub viewer_control: Option<SecretKey>,
    /// `--local-offset` (Power Query's local clock); empty: UTC, as in the Power BI service.
    #[serde(default)]
    pub local_offset: String,
}

/// The recovery journals of unsaved edits (docs/recovery.md): a claim per workspace of its own,
/// mounted outside the workspace's files, passed as `--recovery-dir`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoverySettings {
    #[serde(default = "yes")]
    pub enabled: bool,
    /// The claim's size.
    #[serde(default = "default_recovery_size")]
    pub volume_size: String,
    /// Empty: the workspace volume's storage class.
    #[serde(default)]
    pub storage_class: String,
}

fn default_recovery_size() -> String {
    "1Gi".into()
}

impl Default for RecoverySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            volume_size: default_recovery_size(),
            storage_class: String::new(),
        }
    }
}

fn default_uid() -> u32 {
    65532
}
fn default_grace() -> u32 {
    30
}
fn default_upload() -> String {
    "4G".into()
}

impl Default for PodSettings {
    fn default() -> Self {
        Self {
            run_as_user: default_uid(),
            run_as_group: default_uid(),
            fs_group: default_uid(),
            shutdown_grace_seconds: default_grace(),
            max_upload_bytes: default_upload(),
            allowed_origins: Vec::new(),
            extra_args: Vec::new(),
            extra_env: Vec::new(),
            node_selector: BTreeMap::new(),
            tolerations: Vec::new(),
            priority_class_name: String::new(),
            image_pull_secrets: Vec::new(),
            annotations: BTreeMap::new(),
            max_views_per_user: 0,
            max_views: 0,
            view_idle_seconds: 0,
            allow_autosave: false,
            viewer_control: None,
            local_offset: String::new(),
        }
    }
}

/// The whole configuration.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    /// The custom resource group (default [`DEFAULT_GROUP`]).
    #[serde(default = "default_group")]
    pub crd_group: String,
    /// The namespace of the workspaces (default: the controller's own, from the service account).
    #[serde(default)]
    pub namespace: String,
    /// The workspace pods' image (normally the controller's own).
    pub image: String,
    #[serde(default = "default_pull")]
    pub image_pull_policy: String,
    pub classes: BTreeMap<String, SizeClass>,
    pub default_class: String,
    #[serde(default = "default_idle")]
    pub default_idle_timeout_seconds: u64,
    #[serde(default = "default_reclaim")]
    pub default_reclaim_policy: Reclaim,
    #[serde(default)]
    pub storage_class: String,
    #[serde(default)]
    pub egress_profiles: BTreeMap<String, EgressProfile>,
    /// Where the cluster DNS runs (egress on port 53 is allowed there).
    #[serde(default = "default_dns_ns")]
    pub dns_namespace_selector: Value,
    #[serde(default = "default_dns_pod")]
    pub dns_pod_selector: Value,
    /// Labels of the controller's pods: the only peers allowed into a workspace pod.
    #[serde(default = "default_controller_labels")]
    pub controller_pod_labels: BTreeMap<String, String>,
    /// Whether to create a `NetworkPolicy` per workspace (default true).
    #[serde(default = "yes")]
    pub network_policy: bool,
    /// Addresses the workspace pods accept the gateway's identity headers from. Empty (default):
    /// the controller's own pod address (`POD_IP`), the tightest choice; a controller restart with a
    /// new address then rolls the awake workspace pods.
    #[serde(default)]
    pub gateway_trusted_cidrs: Vec<String>,
    /// The workspace pods' HTTP port.
    #[serde(default = "default_port")]
    pub port: u16,
    /// How long a request waits for a waking workspace.
    #[serde(default = "default_wake")]
    pub wake_timeout_seconds: u64,
    /// The public origin (`https://bicraft.example.com`): the `endpoint` in the status.
    #[serde(default)]
    pub public_url: String,
    #[serde(default)]
    pub pod: PodSettings,
    #[serde(default)]
    pub recovery: RecoverySettings,
}

fn default_group() -> String {
    DEFAULT_GROUP.into()
}
fn default_pull() -> String {
    "IfNotPresent".into()
}
fn default_idle() -> u64 {
    900
}
fn default_reclaim() -> Reclaim {
    Reclaim::Retain
}
fn default_dns_ns() -> Value {
    serde_json::json!({"matchLabels": {"kubernetes.io/metadata.name": "kube-system"}})
}
fn default_dns_pod() -> Value {
    serde_json::json!({"matchLabels": {"k8s-app": "kube-dns"}})
}
fn default_controller_labels() -> BTreeMap<String, String> {
    BTreeMap::from([(
        "app.kubernetes.io/name".to_string(),
        "bicraft-controller".to_string(),
    )])
}
fn default_port() -> u16 {
    7878
}
fn default_wake() -> u64 {
    180
}

impl Config {
    /// Parses and checks a configuration document.
    pub fn from_json(text: &str) -> Result<Config, String> {
        let c: Config = serde_json::from_str(text).map_err(|e| format!("controller configuration: {e}"))?;
        c.check()?;
        Ok(c)
    }

    /// Reads a configuration file.
    pub fn from_file(path: &std::path::Path) -> Result<Config, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.len() > MAX_CONFIG_BYTES {
            return Err(format!("{}: larger than 1 MiB", path.display()));
        }
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Config::from_json(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The consistency rules.
    pub fn check(&self) -> Result<(), String> {
        if !valid_group(&self.crd_group) {
            return Err(format!(
                "crdGroup `{}` is not a DNS name with a dot",
                self.crd_group
            ));
        }
        if self.image.trim().is_empty() {
            return Err("image is required".into());
        }
        if !matches!(
            self.image_pull_policy.as_str(),
            "Always" | "IfNotPresent" | "Never"
        ) {
            return Err(format!("imagePullPolicy `{}`", self.image_pull_policy));
        }
        if self.classes.is_empty() {
            return Err("at least one size class is required".into());
        }
        if !self.classes.contains_key(&self.default_class) {
            return Err(format!("defaultClass `{}` is not a class", self.default_class));
        }
        for (name, c) in &self.classes {
            if !dns_label(name, 63) {
                return Err(format!("class name `{name}` must be a DNS label"));
            }
            if c.cpu == 0 || c.cpu > 1024 {
                return Err(format!("class `{name}`: cpu must be 1 to 1024 whole cores"));
            }
            if c.memory_limit_mib > 16 * 1024 * 1024 {
                return Err(format!("class `{name}`: memoryLimitMiB too large"));
            }
            for (what, q) in [
                ("spillSize", &c.spill_size),
                ("volumeSize", &c.volume_size),
                ("maxVolumeSize", &c.max_volume_size),
            ] {
                if quantity_bytes(q).is_none() {
                    return Err(format!("class `{name}`: {what} `{q}` is not a quantity"));
                }
            }
            if quantity_bytes(&c.volume_size) > quantity_bytes(&c.max_volume_size) {
                return Err(format!("class `{name}`: volumeSize above maxVolumeSize"));
            }
            if !c.cpu_request.is_empty() && !cpu_quantity(&c.cpu_request) {
                return Err(format!("class `{name}`: cpuRequest `{}`", c.cpu_request));
            }
        }
        for (name, p) in &self.egress_profiles {
            if !dns_label(name, 63) {
                return Err(format!("egress profile name `{name}` must be a DNS label"));
            }
            for c in &p.cidrs {
                bicraft_auth::Cidr::parse(c).map_err(|e| format!("egress profile `{name}`: {e}"))?;
            }
            if p.ports.contains(&0) {
                return Err(format!("egress profile `{name}`: port 0"));
            }
        }
        for c in &self.gateway_trusted_cidrs {
            bicraft_auth::Cidr::parse(c).map_err(|e| format!("gatewayTrustedCidrs: {e}"))?;
        }
        if !self.storage_class.is_empty() && !dns_subdomain(&self.storage_class) {
            return Err(format!("storageClass `{}`", self.storage_class));
        }
        if !self.namespace.is_empty() && !dns_label(&self.namespace, 63) {
            return Err(format!("namespace `{}`", self.namespace));
        }
        if self.port == 0 {
            return Err("port 0".into());
        }
        if self.wake_timeout_seconds == 0 || self.wake_timeout_seconds > 3600 {
            return Err("wakeTimeoutSeconds must be 1 to 3600".into());
        }
        if self.pod.shutdown_grace_seconds > 86_400 {
            return Err("pod.shutdownGraceSeconds above 86400".into());
        }
        if self.pod.view_idle_seconds > 30 * 86_400 {
            return Err("pod.viewIdleSeconds above 30 days".into());
        }
        if let Some(s) = &self.pod.viewer_control
            && (!dns_subdomain(&s.name) || s.key.trim().is_empty())
        {
            return Err(format!("pod.viewerControl: Secret `{}` key `{}`", s.name, s.key));
        }
        if !self.pod.local_offset.is_empty() && !offset_text(&self.pod.local_offset) {
            return Err(format!(
                "pod.localOffset `{}`: ±HH:MM, minutes, UTC or machine",
                self.pod.local_offset
            ));
        }
        if self.recovery.enabled {
            if quantity_bytes(&self.recovery.volume_size).is_none_or(|b| b == 0) {
                return Err(format!(
                    "recovery.volumeSize `{}` is not a quantity",
                    self.recovery.volume_size
                ));
            }
            if !self.recovery.storage_class.is_empty() && !dns_subdomain(&self.recovery.storage_class) {
                return Err(format!("recovery.storageClass `{}`", self.recovery.storage_class));
            }
        }
        Ok(())
    }
}

/// A DNS-1123 label of at most `max` characters.
pub fn dns_label(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

/// A DNS-1123 subdomain.
pub fn dns_subdomain(s: &str) -> bool {
    !s.is_empty() && s.len() <= 253 && s.split('.').all(|l| dns_label(l, 63))
}

/// The forms `bicraft-cli serve --local-offset` takes (it checks the value again at start).
fn offset_text(s: &str) -> bool {
    let t = s.trim();
    if ["utc", "z", "machine", "local"]
        .iter()
        .any(|w| t.eq_ignore_ascii_case(w))
    {
        return true;
    }
    let body = t.strip_prefix(['+', '-']).unwrap_or(t);
    let digits = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    match body.split_once(':') {
        Some((h, m)) => t.len() == 6 && h.len() == 2 && m.len() == 2 && digits(h) && digits(m),
        None => body.len() <= 4 && digits(body),
    }
}

fn valid_group(s: &str) -> bool {
    dns_subdomain(s) && s.contains('.')
}

fn cpu_quantity(s: &str) -> bool {
    let digits = s.strip_suffix('m').unwrap_or(s);
    !digits.is_empty() && digits.len() <= 9 && digits.bytes().all(|b| b.is_ascii_digit())
}

/// A Kubernetes storage quantity (`20Gi`, `512Mi`, `1G`, `1000`) in bytes; `None` when it is
/// not one this controller understands (decimal fractions and exponents are refused).
pub fn quantity_bytes(q: &str) -> Option<u64> {
    let q = q.trim();
    let split = q.find(|c: char| !c.is_ascii_digit()).unwrap_or(q.len());
    let (digits, unit) = q.split_at(split);
    if digits.is_empty() || digits.len() > 15 {
        return None;
    }
    let n: u64 = digits.parse().ok()?;
    let mult: u64 = match unit {
        "" => 1,
        "k" => 1000,
        "M" => 1000_u64.pow(2),
        "G" => 1000_u64.pow(3),
        "T" => 1000_u64.pow(4),
        "Ki" => 1 << 10,
        "Mi" => 1 << 20,
        "Gi" => 1 << 30,
        "Ti" => 1 << 40,
        _ => return None,
    };
    n.checked_mul(mult)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn sample_json() -> String {
        serde_json::json!({
            "image": "bicraft:test",
            "namespace": "bi",
            "defaultClass": "small",
            "classes": {
                "small": {"memoryLimitMiB": 448, "cpu": 1, "spillSize": "1Gi", "volumeSize": "1Gi", "maxVolumeSize": "5Gi"},
                "large": {"memoryLimitMiB": 8128, "cpu": 8, "memoryBudgetMiB": 4096, "aggregations": false,
                          "spillSize": "20Gi", "volumeSize": "50Gi", "maxVolumeSize": "200Gi"}
            },
            "egressProfiles": {
                "databricks": {"cidrs": ["203.0.113.0/24"], "allowNetwork": true,
                               "databricksSecret": {"name": "dbx", "key": "token"}}
            },
            "gatewayTrustedCidrs": ["10.244.0.5/32"],
            "publicUrl": "https://bi.example.com"
        })
        .to_string()
    }

    pub fn sample() -> Config {
        Config::from_json(&sample_json()).unwrap()
    }

    #[test]
    fn parses_with_defaults() {
        let c = sample();
        assert_eq!(c.crd_group, DEFAULT_GROUP);
        assert_eq!(c.default_reclaim_policy, Reclaim::Retain);
        assert_eq!(c.pod.run_as_user, 65532);
        assert_eq!(c.port, 7878);
        assert!(c.classes["small"].aggregations);
        assert!(!c.classes["large"].aggregations);
    }

    #[test]
    fn refusals() {
        let mut v: Value = serde_json::from_str(&sample_json()).unwrap();
        v["defaultClass"] = "huge".into();
        assert!(
            Config::from_json(&v.to_string())
                .unwrap_err()
                .contains("defaultClass")
        );
        let mut v: Value = serde_json::from_str(&sample_json()).unwrap();
        v["classes"]["small"]["volumeSize"] = "10Gi".into();
        assert!(
            Config::from_json(&v.to_string())
                .unwrap_err()
                .contains("maxVolumeSize")
        );
        let mut v: Value = serde_json::from_str(&sample_json()).unwrap();
        v["crdGroup"] = "nodot".into();
        assert!(Config::from_json(&v.to_string()).is_err());
        let mut v: Value = serde_json::from_str(&sample_json()).unwrap();
        v["surprise"] = 1.into();
        assert!(Config::from_json(&v.to_string()).is_err());
        let mut v: Value = serde_json::from_str(&sample_json()).unwrap();
        v["egressProfiles"]["databricks"]["cidrs"] = serde_json::json!(["nope"]);
        assert!(Config::from_json(&v.to_string()).is_err());
        assert!(Config::from_json("[]").is_err());
    }

    #[test]
    fn workspace_pod_settings_of_the_new_flags() {
        let c = sample();
        assert!(c.recovery.enabled, "auto-recovery is on by default");
        assert_eq!(c.recovery.volume_size, "1Gi");
        assert_eq!(c.pod.viewer_control, None);
        let mut v: Value = serde_json::from_str(&sample_json()).unwrap();
        v["pod"] = serde_json::json!({"maxViewsPerUser": 4, "maxViews": 16, "viewIdleSeconds": 600,
            "allowAutosave": true, "viewerControl": {"name": "ctl", "key": "token"}, "localOffset": "+02:00"});
        v["recovery"] = serde_json::json!({"volumeSize": "2Gi", "storageClass": "fast"});
        let c = Config::from_json(&v.to_string()).unwrap();
        assert_eq!(
            (c.pod.max_views_per_user, c.pod.max_views, c.pod.view_idle_seconds),
            (4, 16, 600)
        );
        assert!(c.pod.allow_autosave);
        assert_eq!(c.pod.viewer_control.as_ref().unwrap().name, "ctl");
        for (key, bad) in [
            ("localOffset", serde_json::json!("Europe/Paris")),
            ("localOffset", serde_json::json!("+2:00")),
            (
                "viewerControl",
                serde_json::json!({"name": "Bad_Name", "key": "k"}),
            ),
            ("viewIdleSeconds", serde_json::json!(31 * 86_400)),
        ] {
            let mut w = v.clone();
            w["pod"][key] = bad;
            assert!(Config::from_json(&w.to_string()).is_err(), "{key}");
        }
        let mut w = v.clone();
        w["recovery"]["volumeSize"] = "lots".into();
        assert!(Config::from_json(&w.to_string()).is_err());
        w["recovery"]["enabled"] = false.into();
        assert!(Config::from_json(&w.to_string()).is_ok(), "unchecked when off");
    }

    #[test]
    fn quantities() {
        assert_eq!(quantity_bytes("1Gi"), Some(1 << 30));
        assert_eq!(quantity_bytes("5G"), Some(5_000_000_000));
        assert_eq!(quantity_bytes("100"), Some(100));
        assert_eq!(quantity_bytes("1.5Gi"), None);
        assert_eq!(quantity_bytes("Gi"), None);
        assert_eq!(quantity_bytes("99999999999999Ti"), None);
        assert!(dns_label("sales-2026", 40));
        assert!(!dns_label("Sales", 40));
        assert!(!dns_label("-a", 40));
    }
}
