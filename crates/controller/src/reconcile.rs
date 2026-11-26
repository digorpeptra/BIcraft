//! The reconciler: level-triggered, every few seconds and on demand. For each `BicraftWorkspace`
//! it checks the spec against the configuration, applies the owned objects (server-side apply,
//! skipped when nothing changed since the last apply, re-applied every few minutes to repair
//! drift), scales the Deployment to 1 (awake) or 0 (idle, suspended), and writes the status.
//!
//! Whether a workspace is awake lives here, in memory; after a restart it is read back from the
//! Deployment's replica count. Activity comes from the gateway ([`Controller::begin`]).

use std::collections::{BTreeMap, HashMap};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::config::{Config, KIND, VERSION};
use crate::kube::{Kube, KubeError, Resource};
use crate::render;
use crate::spec::{Object, Workspace, claim_name, object_name, recovery_claim_name, resolve, rfc3339};

/// Objects are applied again after this long even when nothing changed (drift repair).
const REAPPLY: Duration = Duration::from_secs(300);
/// `lastActivity` in the status moves by steps of at least this much (fewer API writes).
const ACTIVITY_STEP: u64 = 30;

/// The phases of a workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Starting,
    Running,
    Failed,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Idle => "Idle",
            Phase::Starting => "Starting",
            Phase::Running => "Running",
            Phase::Failed => "Failed",
        }
    }
}

/// What the controller knows about one workspace.
#[derive(Debug, Clone)]
pub struct WsState {
    pub awake: bool,
    pub inflight: usize,
    pub last: Instant,
    pub last_wall: u64,
    /// The checked spec (`None` while it is invalid).
    pub info: Option<Workspace>,
    pub phase: Phase,
    pub message: String,
    applied: Option<(u64, Instant)>,
    status_wall: u64,
}

/// One reconciliation pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub workspaces: usize,
    pub applied: usize,
    pub errors: Vec<String>,
}

/// The reconciler and the state it shares with the gateway.
pub struct Controller {
    pub cfg: Config,
    pub kube: Arc<dyn Kube>,
    /// Addresses the workspace pods accept the gateway's identity headers from.
    pub trusted: Vec<String>,
    state: Mutex<BTreeMap<String, WsState>>,
    /// One reconciliation at a time (the loop, wake-ups and API calls).
    reconcile: Mutex<()>,
    kick: (Mutex<bool>, Condvar),
    ready: AtomicBool,
}

impl std::fmt::Debug for Controller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Controller(namespace {})", self.cfg.namespace)
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn hash_of(values: &[&Value]) -> u64 {
    let mut h = DefaultHasher::new();
    for v in values {
        v.to_string().hash(&mut h);
    }
    h.finish()
}

/// Releases an activity slot on drop.
pub struct Activity {
    ctl: Arc<Controller>,
    name: String,
}

impl Drop for Activity {
    fn drop(&mut self) {
        let mut st = self.ctl.lock();
        if let Some(w) = st.get_mut(&self.name) {
            w.inflight = w.inflight.saturating_sub(1);
            w.last = Instant::now();
            w.last_wall = unix_now();
        }
    }
}

impl Controller {
    pub fn new(cfg: Config, kube: Arc<dyn Kube>, trusted: Vec<String>) -> Arc<Controller> {
        Arc::new(Controller {
            cfg,
            kube,
            trusted,
            state: Mutex::new(BTreeMap::new()),
            reconcile: Mutex::new(()),
            kick: (Mutex::new(false), Condvar::new()),
            ready: AtomicBool::new(false),
        })
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, WsState>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether a reconciliation has succeeded (the controller's `/readyz`).
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    /// A copy of a workspace's state.
    pub fn workspace(&self, name: &str) -> Option<WsState> {
        self.lock().get(name).cloned()
    }

    /// Counts a request to `name` as running until the guard drops.
    pub fn begin(self: &Arc<Self>, name: &str) -> Activity {
        let mut st = self.lock();
        if let Some(w) = st.get_mut(name) {
            w.inflight = w.inflight.saturating_add(1);
            w.last = Instant::now();
            w.last_wall = unix_now();
        }
        Activity {
            ctl: Arc::clone(self),
            name: name.to_string(),
        }
    }

    /// Wakes the loop up for an immediate pass.
    pub fn kick(&self) {
        let (m, cv) = &self.kick;
        *m.lock().unwrap_or_else(|e| e.into_inner()) = true;
        cv.notify_all();
    }

    /// Runs passes every `interval` (or when kicked) until `stop`.
    pub fn run(&self, interval: Duration, stop: &AtomicBool) {
        let mut last_error = String::new();
        while !stop.load(Ordering::SeqCst) {
            match self.reconcile_all(Instant::now(), unix_now()) {
                Ok(r) => {
                    for e in &r.errors {
                        eprintln!("bicraft controller: {e}");
                    }
                    last_error.clear();
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg != last_error {
                        eprintln!("bicraft controller: reconciliation failed: {msg}");
                        last_error = msg;
                    }
                }
            }
            let (m, cv) = &self.kick;
            let mut kicked = m.lock().unwrap_or_else(|e| e.into_inner());
            if !*kicked {
                kicked = cv
                    .wait_timeout(kicked, interval)
                    .map(|(g, _)| g)
                    .unwrap_or_else(|e| e.into_inner().0);
            }
            *kicked = false;
        }
    }

    /// One pass over every workspace.
    pub fn reconcile_all(&self, now: Instant, wall: u64) -> Result<Report, KubeError> {
        let _one = self.reconcile.lock().unwrap_or_else(|e| e.into_inner());
        let objects = self.kube.list(Resource::Workspace, None)?;
        let deployments = self
            .kube
            .list(Resource::Deployment, Some(&render::managed_selector()))?;
        let by_name: HashMap<String, Value> = deployments
            .into_iter()
            .filter_map(|d| {
                let n = d.pointer("/metadata/name")?.as_str()?.to_string();
                Some((n, d))
            })
            .collect();
        let mut report = Report::default();
        let mut seen = Vec::new();
        for v in &objects {
            let Some(o) = Object::from_json(v) else { continue };
            report.workspaces = report.workspaces.saturating_add(1);
            let observed = by_name.get(&object_name(&o.name));
            match self.reconcile_one(&o, observed, now, wall) {
                Ok(applied) => {
                    if applied {
                        report.applied = report.applied.saturating_add(1);
                    }
                }
                Err(e) => report.errors.push(format!("workspace {}: {e}", o.name)),
            }
            seen.push(o.name);
        }
        self.lock().retain(|name, _| seen.contains(name));
        self.ready.store(true, Ordering::SeqCst);
        Ok(report)
    }

    /// Reconciles one workspace now (after a wake-up, a stop or an API creation).
    pub fn reconcile_named(&self, name: &str) -> Result<(), String> {
        let _one = self.reconcile.lock().unwrap_or_else(|e| e.into_inner());
        let v = self
            .kube
            .get(Resource::Workspace, name)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("workspace {name} does not exist"))?;
        let o = Object::from_json(&v).ok_or("malformed workspace object")?;
        let d = self
            .kube
            .get(Resource::Deployment, &object_name(name))
            .map_err(|e| e.to_string())?;
        self.reconcile_one(&o, d.as_ref(), Instant::now(), unix_now())
            .map(|_| ())
    }

    /// Marks `name` awake (or not) and reconciles it.
    pub fn set_awake(&self, name: &str, awake: bool) -> Result<(), String> {
        {
            let mut st = self.lock();
            let w = st
                .get_mut(name)
                .ok_or_else(|| format!("workspace {name} is not known yet"))?;
            if awake && w.info.as_ref().is_some_and(|i| i.suspend) {
                return Err(format!("workspace {name} is suspended"));
            }
            if awake && w.info.is_none() {
                return Err(format!("workspace {name}: {}", w.message));
            }
            if w.awake != awake {
                eprintln!(
                    "bicraft controller: workspace {name}: {}",
                    if awake { "waking up" } else { "stopping on request" }
                );
            }
            w.awake = awake;
            w.last = Instant::now();
            w.last_wall = unix_now();
        }
        self.reconcile_named(name)
    }

    /// Returns whether objects were applied.
    fn reconcile_one(
        &self,
        o: &Object,
        observed: Option<&Value>,
        now: Instant,
        wall: u64,
    ) -> Result<bool, String> {
        let observed_replicas = observed
            .and_then(|d| d.pointer("/spec/replicas"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let ready_replicas = observed
            .and_then(|d| d.pointer("/status/readyReplicas"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let resolved = o
            .spec
            .clone()
            .and_then(|s| resolve(&self.cfg, &o.name, &o.uid, o.generation, &s));

        // Update the in-memory state.
        let (awake, applied_before, status_wall_before, last_wall) = {
            let mut st = self.lock();
            let w = st.entry(o.name.clone()).or_insert_with(|| WsState {
                awake: observed_replicas >= 1,
                inflight: 0,
                last: now,
                last_wall: wall,
                info: None,
                phase: Phase::Idle,
                message: String::new(),
                applied: None,
                status_wall: 0,
            });
            if let Ok(info) = &resolved {
                if w.awake
                    && !info.suspend
                    && w.inflight == 0
                    && info
                        .idle_timeout
                        .is_some_and(|t| now.saturating_duration_since(w.last) >= t)
                {
                    eprintln!(
                        "bicraft controller: workspace {}: idle for {} s, scaling to zero",
                        o.name,
                        now.saturating_duration_since(w.last).as_secs()
                    );
                    w.awake = false;
                }
                if info.suspend {
                    w.awake = false;
                }
            }
            w.info = resolved.clone().ok();
            (w.awake, w.applied, w.status_wall, w.last_wall)
        };

        let mut applied = false;
        let mut failure: Option<String> = None;
        let mut replicas = 0u8;
        match &resolved {
            Err(e) => failure = Some(e.clone()),
            Ok(w) => {
                replicas = u8::from(awake);
                let deploy = render::deployment(&self.cfg, w, replicas, &self.trusted);
                let svc = render::service(&self.cfg, w);
                let np = render::network_policy(&self.cfg, w);
                let pvc = render::claim(&self.cfg, w);
                let rpvc = render::recovery_claim(&self.cfg, w);
                let h = hash_of(&[&deploy, &svc, &np, &pvc, rpvc.as_ref().unwrap_or(&Value::Null)]);
                let fresh = applied_before
                    .is_some_and(|(prev, at)| prev == h && now.saturating_duration_since(at) < REAPPLY);
                if !fresh || observed.is_none() {
                    let mut writes = vec![(Resource::Claim, claim_name(&w.name), pvc)];
                    if let Some(r) = rpvc {
                        writes.push((Resource::Claim, recovery_claim_name(&w.name), r));
                    }
                    writes.push((Resource::Service, object_name(&w.name), svc));
                    if self.cfg.network_policy {
                        writes.push((Resource::NetworkPolicy, object_name(&w.name), np));
                    }
                    writes.push((Resource::Deployment, object_name(&w.name), deploy));
                    let mut ok = true;
                    for (r, name, body) in writes {
                        if let Err(e) = self.kube.apply(r, &name, &body) {
                            failure = Some(format!("applying {r:?} {name}: {e}"));
                            ok = false;
                            break;
                        }
                    }
                    if ok {
                        applied = true;
                        if let Some(s) = self.lock().get_mut(&o.name) {
                            s.applied = Some((h, now));
                        }
                    }
                }
            }
        }

        let phase = match (&failure, replicas) {
            (Some(_), _) => Phase::Failed,
            (None, 0) => Phase::Idle,
            (None, _) if ready_replicas >= 1 => Phase::Running,
            (None, _) => Phase::Starting,
        };
        let message = match (&failure, &resolved) {
            (Some(f), _) => f.clone(),
            (None, Ok(w)) if w.suspend => "suspended".to_string(),
            (None, _) if phase == Phase::Idle => "scaled to zero; the next request wakes it".into(),
            _ => String::new(),
        };
        let status_wall =
            if last_wall.abs_diff(status_wall_before) >= ACTIVITY_STEP || status_wall_before == 0 {
                last_wall
            } else {
                status_wall_before
            };
        {
            let mut st = self.lock();
            if let Some(s) = st.get_mut(&o.name) {
                s.phase = phase;
                s.message.clone_from(&message);
                s.status_wall = status_wall;
            }
        }
        let endpoint = if self.cfg.public_url.is_empty() {
            format!("/w/{}/mcp", o.name)
        } else {
            format!("{}/w/{}/mcp", self.cfg.public_url.trim_end_matches('/'), o.name)
        };
        let mut status = json!({
            "phase": phase.as_str(),
            "message": message,
            "endpoint": endpoint,
            "lastActivity": rfc3339(status_wall),
            "readyReplicas": if replicas == 0 { 0 } else { ready_replicas },
            "observedGeneration": o.generation,
        });
        if let (Ok(w), Some(m)) = (&resolved, status.as_object_mut()) {
            m.insert("volumeClaim".into(), json!(claim_name(&w.name)));
            if self.cfg.recovery.enabled {
                m.insert("recoveryClaim".into(), json!(recovery_claim_name(&w.name)));
            }
            m.insert("sizeClass".into(), json!(w.class_name));
        }
        if status != o.status {
            let body = json!({
                "apiVersion": format!("{}/{VERSION}", self.cfg.crd_group),
                "kind": KIND,
                "metadata": {"name": o.name},
                "status": status,
            });
            self.kube
                .apply_status(Resource::Workspace, &o.name, &body)
                .map_err(|e| format!("writing the status: {e}"))?;
        }
        match failure {
            Some(f) if resolved.is_ok() => Err(f),
            _ => Ok(applied),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::sample;
    use crate::kube::FakeKube;

    fn setup() -> (Arc<FakeKube>, Arc<Controller>) {
        let kube = Arc::new(FakeKube::new());
        let ctl = Controller::new(sample(), kube.clone(), vec!["10.0.0.9/32".into()]);
        (kube, ctl)
    }

    fn create(kube: &FakeKube, name: &str, spec: Value) {
        kube.create(
            Resource::Workspace,
            &json!({"apiVersion": "bicraft.dev/v1alpha1", "kind": KIND, "metadata": {"name": name}, "spec": spec}),
        )
        .unwrap();
    }

    fn replicas(kube: &FakeKube, ws: &str) -> u64 {
        kube.object(Resource::Deployment, &object_name(ws))
            .and_then(|d| d.pointer("/spec/replicas").and_then(Value::as_u64))
            .unwrap()
    }

    fn phase(kube: &FakeKube, ws: &str) -> String {
        kube.object(Resource::Workspace, ws).unwrap()["status"]["phase"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    #[test]
    fn a_new_workspace_gets_its_objects_and_starts_idle() {
        let (kube, ctl) = setup();
        create(&kube, "sales", json!({"grants": ["token:agent"]}));
        let r = ctl.reconcile_all(Instant::now(), 1_793_059_200).unwrap();
        assert_eq!(
            r,
            Report {
                workspaces: 1,
                applied: 1,
                errors: vec![]
            }
        );
        for (res, name) in [
            (Resource::Claim, "bws-sales-data"),
            (Resource::Service, "bws-sales"),
            (Resource::NetworkPolicy, "bws-sales"),
            (Resource::Deployment, "bws-sales"),
        ] {
            assert!(kube.object(res, name).is_some(), "{res:?} {name}");
        }
        assert_eq!(replicas(&kube, "sales"), 0);
        let st = &kube.object(Resource::Workspace, "sales").unwrap()["status"];
        assert_eq!(st["phase"], "Idle");
        assert_eq!(st["endpoint"], "https://bi.example.com/w/sales/mcp");
        assert_eq!(st["lastActivity"], "2026-10-27T00:00:00Z");
        assert_eq!(st["volumeClaim"], "bws-sales-data");
        // A second pass with nothing changed writes nothing.
        kube.take_calls();
        let r = ctl.reconcile_all(Instant::now(), 1_793_059_200).unwrap();
        assert_eq!(r.applied, 0);
        assert_eq!(kube.take_calls(), vec!["list Workspace", "list Deployment"]);
    }

    #[test]
    fn wake_run_idle_stop() {
        let (kube, ctl) = setup();
        create(&kube, "sales", json!({"idleTimeoutSeconds": 60}));
        ctl.reconcile_all(Instant::now(), 0).unwrap();
        ctl.set_awake("sales", true).unwrap();
        assert_eq!(replicas(&kube, "sales"), 1);
        assert_eq!(phase(&kube, "sales"), "Starting");
        kube.set_status(Resource::Deployment, "bws-sales", json!({"readyReplicas": 1}));
        let t0 = Instant::now();
        ctl.reconcile_all(t0, 0).unwrap();
        assert_eq!(phase(&kube, "sales"), "Running");
        // A request in flight keeps it awake past the timeout.
        let guard = ctl.begin("sales");
        ctl.reconcile_all(t0 + Duration::from_secs(120), 0).unwrap();
        assert_eq!(replicas(&kube, "sales"), 1);
        drop(guard);
        ctl.reconcile_all(Instant::now() + Duration::from_secs(30), 0)
            .unwrap();
        assert_eq!(replicas(&kube, "sales"), 1);
        ctl.reconcile_all(Instant::now() + Duration::from_secs(61), 0)
            .unwrap();
        assert_eq!(replicas(&kube, "sales"), 0);
        assert_eq!(phase(&kube, "sales"), "Idle");
        assert!(!ctl.workspace("sales").unwrap().awake);
    }

    #[test]
    fn zero_idle_timeout_never_stops_and_suspend_always_stops() {
        let (kube, ctl) = setup();
        create(&kube, "a", json!({"idleTimeoutSeconds": 0}));
        create(&kube, "b", json!({"suspend": true}));
        ctl.reconcile_all(Instant::now(), 0).unwrap();
        ctl.set_awake("a", true).unwrap();
        ctl.reconcile_all(Instant::now() + Duration::from_secs(86_400 * 365), 0)
            .unwrap();
        assert_eq!(replicas(&kube, "a"), 1);
        assert!(ctl.set_awake("b", true).unwrap_err().contains("suspended"));
        assert_eq!(replicas(&kube, "b"), 0);
        assert_eq!(
            kube.object(Resource::Workspace, "b").unwrap()["status"]["message"],
            "suspended"
        );
    }

    #[test]
    fn an_invalid_spec_fails_without_objects() {
        let (kube, ctl) = setup();
        create(&kube, "x", json!({"sizeClass": "huge"}));
        create(&kube, "y", json!({"memory": "64Gi"}));
        let r = ctl.reconcile_all(Instant::now(), 0).unwrap();
        assert!(r.errors.is_empty());
        for ws in ["x", "y"] {
            assert_eq!(phase(&kube, ws), "Failed");
            assert!(kube.object(Resource::Deployment, &object_name(ws)).is_none());
        }
        let msg = kube.object(Resource::Workspace, "x").unwrap()["status"]["message"].clone();
        assert!(msg.as_str().unwrap().contains("sizeClass"));
        assert!(ctl.set_awake("x", true).is_err());
    }

    #[test]
    fn api_failures_are_reported_and_retried() {
        let (kube, ctl) = setup();
        create(&kube, "sales", json!({}));
        kube.fail_writes(Some((Resource::Deployment, 403)));
        let r = ctl.reconcile_all(Instant::now(), 0).unwrap();
        assert_eq!(r.errors.len(), 1);
        assert!(r.errors[0].contains("403"));
        assert_eq!(phase(&kube, "sales"), "Failed");
        kube.fail_writes(None);
        let r = ctl.reconcile_all(Instant::now(), 0).unwrap();
        assert!(r.errors.is_empty());
        assert_eq!(phase(&kube, "sales"), "Idle");
        assert!(kube.object(Resource::Deployment, "bws-sales").is_some());
    }

    #[test]
    fn a_restart_reads_awake_from_the_deployment_and_repairs_drift() {
        let (kube, ctl) = setup();
        create(&kube, "sales", json!({}));
        ctl.reconcile_all(Instant::now(), 0).unwrap();
        ctl.set_awake("sales", true).unwrap();
        let ctl2 = Controller::new(sample(), kube.clone(), vec!["10.0.0.9/32".into()]);
        ctl2.reconcile_all(Instant::now(), 0).unwrap();
        assert!(ctl2.workspace("sales").unwrap().awake);
        assert_eq!(replicas(&kube, "sales"), 1);
        // Someone deletes the Deployment: the next pass puts it back.
        kube.delete(Resource::Deployment, "bws-sales").unwrap();
        ctl2.reconcile_all(Instant::now(), 0).unwrap();
        assert_eq!(replicas(&kube, "sales"), 1);
    }

    #[test]
    fn deleting_the_workspace_deletes_its_objects_and_the_claim_follows_the_policy() {
        let (kube, ctl) = setup();
        create(&kube, "kept", json!({}));
        create(&kube, "gone", json!({"volume": {"reclaimPolicy": "Delete"}}));
        ctl.reconcile_all(Instant::now(), 0).unwrap();
        kube.delete(Resource::Workspace, "kept").unwrap();
        kube.delete(Resource::Workspace, "gone").unwrap();
        ctl.reconcile_all(Instant::now(), 0).unwrap();
        assert!(ctl.workspace("kept").is_none());
        for ws in ["kept", "gone"] {
            assert!(kube.object(Resource::Deployment, &object_name(ws)).is_none());
            assert!(kube.object(Resource::Service, &object_name(ws)).is_none());
        }
        assert!(kube.object(Resource::Claim, "bws-kept-data").is_some());
        assert!(kube.object(Resource::Claim, "bws-gone-data").is_none());
        // Created again, the retained claim is adopted (applied over, same name).
        create(&kube, "kept", json!({}));
        ctl.reconcile_all(Instant::now(), 0).unwrap();
        assert_eq!(phase(&kube, "kept"), "Idle");
    }
}
