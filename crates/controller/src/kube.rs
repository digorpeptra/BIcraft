//! A minimal Kubernetes API client: the five calls the controller needs (list, get, create,
//! server-side apply of an object or its status, delete) on five resource types, over `ureq` with
//! rustls. In a cluster it verifies the API server against the service account's CA and sends the
//! projected service-account token, re-read from its file (the kubelet rotates it). Outside a
//! cluster it talks to a URL such as `kubectl proxy`'s.
//!
//! [`Kube`] is the seam: the reconciler and the gateway are tested against [`FakeKube`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::config::{MANAGER, PLURAL, VERSION};

/// The service-account mount of a pod.
pub const SA_DIR: &str = "/var/run/secrets/kubernetes.io/serviceaccount";
/// How often the token file is read again (client-go re-reads it every minute).
const TOKEN_REFRESH: Duration = Duration::from_secs(60);
/// One API call's limit.
const TIMEOUT: Duration = Duration::from_secs(30);
/// The largest API answer read.
const MAX_ANSWER: u64 = 32 << 20;
/// The most pages of one list.
const MAX_PAGES: usize = 50;

/// The resources the controller reads and writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Resource {
    Workspace,
    Deployment,
    Service,
    NetworkPolicy,
    Claim,
}

impl Resource {
    /// The collection path in namespace `ns`.
    pub fn collection(self, group: &str, ns: &str) -> String {
        match self {
            Resource::Workspace => format!("/apis/{group}/{VERSION}/namespaces/{ns}/{PLURAL}"),
            Resource::Deployment => format!("/apis/apps/v1/namespaces/{ns}/deployments"),
            Resource::Service => format!("/api/v1/namespaces/{ns}/services"),
            Resource::NetworkPolicy => {
                format!("/apis/networking.k8s.io/v1/namespaces/{ns}/networkpolicies")
            }
            Resource::Claim => format!("/api/v1/namespaces/{ns}/persistentvolumeclaims"),
        }
    }
}

/// An API failure: the HTTP status when there was an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KubeError {
    pub status: Option<u16>,
    pub message: String,
}

impl KubeError {
    pub fn new(status: Option<u16>, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for KubeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.status {
            Some(s) => write!(f, "Kubernetes API {s}: {}", self.message),
            None => write!(f, "Kubernetes API: {}", self.message),
        }
    }
}

/// The calls the controller makes, all in its one namespace.
pub trait Kube: Send + Sync {
    /// Every object, optionally filtered by a label selector (`k=v`).
    fn list(&self, r: Resource, selector: Option<&str>) -> Result<Vec<Value>, KubeError>;
    fn get(&self, r: Resource, name: &str) -> Result<Option<Value>, KubeError>;
    /// Creates an object; 409 when it exists.
    fn create(&self, r: Resource, body: &Value) -> Result<Value, KubeError>;
    /// Server-side apply (field manager `bicraft-controller`, forced): creates or updates.
    fn apply(&self, r: Resource, name: &str, body: &Value) -> Result<Value, KubeError>;
    /// Server-side apply of the status subresource.
    fn apply_status(&self, r: Resource, name: &str, body: &Value) -> Result<Value, KubeError>;
    /// Deletes an object (background propagation); `false` when it did not exist.
    fn delete(&self, r: Resource, name: &str) -> Result<bool, KubeError>;
}

// ---------------------------------------------------------------------------------------------
// The real client

struct Token {
    path: Option<PathBuf>,
    value: String,
    read_at: Option<Instant>,
}

/// The HTTP client of the Kubernetes API.
pub struct HttpKube {
    base: String,
    group: String,
    namespace: String,
    agent: ureq::Agent,
    token: Mutex<Token>,
}

impl std::fmt::Debug for HttpKube {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HttpKube({}, namespace {})", self.base, self.namespace)
    }
}

fn read_small(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > 1024 * 1024 {
        return Err(format!("{}: larger than 1 MiB", path.display()));
    }
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

impl HttpKube {
    /// The in-cluster client: `KUBERNETES_SERVICE_HOST`/`_PORT`, the service account's CA,
    /// token and namespace (`namespace` overrides the last when not empty).
    pub fn in_cluster(group: &str, namespace: &str) -> Result<HttpKube, String> {
        let host = std::env::var("KUBERNETES_SERVICE_HOST")
            .map_err(|_| "KUBERNETES_SERVICE_HOST is not set: not in a cluster (use --kube-api URL)")?;
        let port = std::env::var("KUBERNETES_SERVICE_PORT").unwrap_or_else(|_| "443".into());
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host
        };
        let dir = Path::new(SA_DIR);
        let ca = read_small(&dir.join("ca.crt"))?;
        let mut certs = Vec::new();
        for item in ureq::tls::parse_pem(ca.as_bytes()) {
            if let Ok(ureq::tls::PemItem::Certificate(c)) = item {
                certs.push(c);
            }
        }
        if certs.is_empty() {
            return Err(format!("{SA_DIR}/ca.crt: no certificate"));
        }
        let ns = if namespace.is_empty() {
            read_small(&dir.join("namespace"))?.trim().to_string()
        } else {
            namespace.to_string()
        };
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::new_with_certs(&certs))
            .build();
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .https_only(true)
            .timeout_global(Some(TIMEOUT))
            .tls_config(tls)
            .build()
            .into();
        let c = HttpKube {
            base: format!("https://{host}:{port}"),
            group: group.to_string(),
            namespace: ns,
            agent,
            token: Mutex::new(Token {
                path: Some(dir.join("token")),
                value: String::new(),
                read_at: None,
            }),
        };
        c.bearer()?;
        Ok(c)
    }

    /// A client of `url` without credentials (e.g. `kubectl proxy` on loopback).
    pub fn with_url(url: &str, group: &str, namespace: &str) -> Result<HttpKube, String> {
        let url = url.trim_end_matches('/');
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(format!("--kube-api `{url}`: an http:// or https:// URL"));
        }
        if namespace.is_empty() {
            return Err("--kube-api needs the namespace (--namespace or the configuration)".into());
        }
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .build()
            .into();
        Ok(HttpKube {
            base: url.to_string(),
            group: group.to_string(),
            namespace: namespace.to_string(),
            agent,
            token: Mutex::new(Token {
                path: None,
                value: String::new(),
                read_at: None,
            }),
        })
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The current token (`None` without one), read again after [`TOKEN_REFRESH`] or `force`.
    fn token(&self, force: bool) -> Result<Option<String>, KubeError> {
        let mut t = self.token.lock().unwrap_or_else(|e| e.into_inner());
        let Some(path) = t.path.clone() else {
            return Ok(None);
        };
        let stale = force || t.read_at.is_none_or(|at| at.elapsed() >= TOKEN_REFRESH);
        if stale {
            let v = read_small(&path).map_err(|e| KubeError::new(None, e))?;
            t.value = v.trim().to_string();
            t.read_at = Some(Instant::now());
        }
        Ok(Some(t.value.clone()))
    }

    fn bearer(&self) -> Result<(), String> {
        self.token(true).map(|_| ()).map_err(|e| e.message)
    }

    fn url(&self, r: Resource, name: Option<&str>, sub: &str, query: &str) -> String {
        let mut u = format!("{}{}", self.base, r.collection(&self.group, &self.namespace));
        if let Some(n) = name {
            u.push('/');
            u.push_str(&encode(n));
        }
        u.push_str(sub);
        if !query.is_empty() {
            u.push('?');
            u.push_str(query);
        }
        u
    }

    /// One call; a 401 re-reads the token and retries once.
    fn call(&self, method: &str, url: &str, body: Option<(&str, String)>) -> Result<(u16, Value), KubeError> {
        let mut forced = false;
        loop {
            let token = self.token(forced)?;
            let auth = token.map(|t| format!("Bearer {t}"));
            let result = match (method, &body) {
                ("GET", _) => {
                    let mut rq = self.agent.get(url).header("Accept", "application/json");
                    if let Some(a) = &auth {
                        rq = rq.header("Authorization", a);
                    }
                    rq.call()
                }
                ("DELETE", _) => {
                    let mut rq = self.agent.delete(url).header("Accept", "application/json");
                    if let Some(a) = &auth {
                        rq = rq.header("Authorization", a);
                    }
                    rq.call()
                }
                (m, Some((ct, text))) => {
                    let mut rq = if m == "POST" {
                        self.agent.post(url)
                    } else {
                        self.agent.patch(url)
                    };
                    rq = rq
                        .header("Accept", "application/json")
                        .header("Content-Type", *ct);
                    if let Some(a) = &auth {
                        rq = rq.header("Authorization", a);
                    }
                    rq.send(text.as_str())
                }
                (m, None) => return Err(KubeError::new(None, format!("{m} without a body"))),
            };
            let mut resp =
                result.map_err(|e| KubeError::new(None, format!("{method} {}: {e}", strip(url))))?;
            let status = resp.status().as_u16();
            if status == 401 && !forced && auth.is_some() {
                forced = true;
                continue;
            }
            let bytes = resp
                .body_mut()
                .with_config()
                .limit(MAX_ANSWER)
                .read_to_vec()
                .map_err(|e| KubeError::new(Some(status), format!("reading the answer: {e}")))?;
            let v: Value = if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap_or(Value::Null)
            };
            if !(200..300).contains(&status) {
                let msg = v
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{method} {}", strip(url)));
                return Err(KubeError::new(Some(status), msg));
            }
            return Ok((status, v));
        }
    }
}

/// The URL without its origin (for messages).
fn strip(url: &str) -> &str {
    url.find("/api").and_then(|i| url.get(i..)).unwrap_or(url)
}

/// Percent-encodes a query value or a path segment (unreserved characters kept).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

const APPLY: &str = "application/apply-patch+yaml";

impl Kube for HttpKube {
    fn list(&self, r: Resource, selector: Option<&str>) -> Result<Vec<Value>, KubeError> {
        let mut out = Vec::new();
        let mut cont = String::new();
        for _ in 0..MAX_PAGES {
            let mut q = String::from("limit=500");
            if let Some(s) = selector {
                q.push_str("&labelSelector=");
                q.push_str(&encode(s));
            }
            if !cont.is_empty() {
                q.push_str("&continue=");
                q.push_str(&encode(&cont));
            }
            let (_, v) = self.call("GET", &self.url(r, None, "", &q), None)?;
            if let Some(items) = v.get("items").and_then(Value::as_array) {
                out.extend(items.iter().cloned());
            }
            cont = v
                .pointer("/metadata/continue")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if cont.is_empty() {
                return Ok(out);
            }
        }
        Err(KubeError::new(None, format!("more than {MAX_PAGES} pages")))
    }

    fn get(&self, r: Resource, name: &str) -> Result<Option<Value>, KubeError> {
        match self.call("GET", &self.url(r, Some(name), "", ""), None) {
            Ok((_, v)) => Ok(Some(v)),
            Err(e) if e.status == Some(404) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn create(&self, r: Resource, body: &Value) -> Result<Value, KubeError> {
        let q = format!("fieldManager={MANAGER}");
        self.call(
            "POST",
            &self.url(r, None, "", &q),
            Some(("application/json", body.to_string())),
        )
        .map(|(_, v)| v)
    }

    fn apply(&self, r: Resource, name: &str, body: &Value) -> Result<Value, KubeError> {
        let q = format!("fieldManager={MANAGER}&force=true");
        self.call(
            "PATCH",
            &self.url(r, Some(name), "", &q),
            Some((APPLY, body.to_string())),
        )
        .map(|(_, v)| v)
    }

    fn apply_status(&self, r: Resource, name: &str, body: &Value) -> Result<Value, KubeError> {
        let q = format!("fieldManager={MANAGER}&force=true");
        self.call(
            "PATCH",
            &self.url(r, Some(name), "/status", &q),
            Some((APPLY, body.to_string())),
        )
        .map(|(_, v)| v)
    }

    fn delete(&self, r: Resource, name: &str) -> Result<bool, KubeError> {
        let url = self.url(r, Some(name), "", "propagationPolicy=Background");
        match self.call("DELETE", &url, None) {
            Ok(_) => Ok(true),
            Err(e) if e.status == Some(404) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The fake

#[derive(Default)]
struct FakeState {
    objects: BTreeMap<(Resource, String), Value>,
    calls: Vec<String>,
    next_uid: u64,
    fail: Option<(Resource, u16)>,
}

/// An in-memory API server for tests: objects keyed by resource and name, server-side apply as
/// "replace everything but the status", the garbage collector on delete (owner references by
/// uid), and an optional injected failure.
#[derive(Default)]
pub struct FakeKube {
    state: Mutex<FakeState>,
}

impl std::fmt::Debug for FakeKube {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FakeKube")
    }
}

fn label_match(v: &Value, selector: Option<&str>) -> bool {
    let Some(sel) = selector else { return true };
    sel.split(',').all(|term| match term.split_once('=') {
        Some((k, want)) => {
            v.pointer("/metadata/labels")
                .and_then(|l| l.get(k))
                .and_then(Value::as_str)
                == Some(want)
        }
        None => false,
    })
}

impl FakeKube {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The object, as stored.
    pub fn object(&self, r: Resource, name: &str) -> Option<Value> {
        self.lock().objects.get(&(r, name.to_string())).cloned()
    }

    /// Sets an object's status as the cluster would (a Deployment's `readyReplicas`).
    pub fn set_status(&self, r: Resource, name: &str, status: Value) {
        if let Some(o) = self.lock().objects.get_mut(&(r, name.to_string()))
            && let Some(m) = o.as_object_mut()
        {
            m.insert("status".into(), status);
        }
    }

    /// The calls so far (`apply Deployment bws-a`), then forgets them.
    pub fn take_calls(&self) -> Vec<String> {
        std::mem::take(&mut self.lock().calls)
    }

    /// Makes every write of `r` fail with `status` (`None`: stop failing).
    pub fn fail_writes(&self, fail: Option<(Resource, u16)>) {
        self.lock().fail = fail;
    }

    fn check_fail(st: &FakeState, r: Resource) -> Result<(), KubeError> {
        match st.fail {
            Some((f, status)) if f == r => Err(KubeError::new(Some(status), "injected failure")),
            _ => Ok(()),
        }
    }

    fn insert_new(st: &mut FakeState, r: Resource, name: &str, body: &Value) -> Value {
        st.next_uid = st.next_uid.saturating_add(1);
        let mut v = body.clone();
        if let Some(m) = v.get_mut("metadata").and_then(Value::as_object_mut) {
            m.insert("uid".into(), json!(format!("uid-{}", st.next_uid)));
            m.insert("generation".into(), json!(1));
        }
        st.objects.insert((r, name.to_string()), v.clone());
        v
    }
}

impl Kube for FakeKube {
    fn list(&self, r: Resource, selector: Option<&str>) -> Result<Vec<Value>, KubeError> {
        let mut st = self.lock();
        st.calls.push(format!("list {r:?}"));
        Ok(st
            .objects
            .iter()
            .filter(|((res, _), v)| *res == r && label_match(v, selector))
            .map(|(_, v)| v.clone())
            .collect())
    }

    fn get(&self, r: Resource, name: &str) -> Result<Option<Value>, KubeError> {
        Ok(self.lock().objects.get(&(r, name.to_string())).cloned())
    }

    fn create(&self, r: Resource, body: &Value) -> Result<Value, KubeError> {
        let mut st = self.lock();
        Self::check_fail(&st, r)?;
        let name = body
            .pointer("/metadata/name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        st.calls.push(format!("create {r:?} {name}"));
        if st.objects.contains_key(&(r, name.clone())) {
            return Err(KubeError::new(Some(409), "already exists"));
        }
        Ok(Self::insert_new(&mut st, r, &name, body))
    }

    fn apply(&self, r: Resource, name: &str, body: &Value) -> Result<Value, KubeError> {
        let mut st = self.lock();
        Self::check_fail(&st, r)?;
        st.calls.push(format!("apply {r:?} {name}"));
        let key = (r, name.to_string());
        let Some(old) = st.objects.get(&key).cloned() else {
            return Ok(Self::insert_new(&mut st, r, name, body));
        };
        let mut v = body.clone();
        if let Some(m) = v.get_mut("metadata").and_then(Value::as_object_mut) {
            for k in ["uid", "generation"] {
                if let Some(x) = old.pointer(&format!("/metadata/{k}")) {
                    m.insert(k.into(), x.clone());
                }
            }
        }
        if let (Some(s), Some(m)) = (old.get("status"), v.as_object_mut()) {
            m.insert("status".into(), s.clone());
        }
        st.objects.insert(key, v.clone());
        Ok(v)
    }

    fn apply_status(&self, r: Resource, name: &str, body: &Value) -> Result<Value, KubeError> {
        let mut st = self.lock();
        Self::check_fail(&st, r)?;
        st.calls.push(format!("status {r:?} {name}"));
        let o = st
            .objects
            .get_mut(&(r, name.to_string()))
            .ok_or_else(|| KubeError::new(Some(404), "not found"))?;
        if let Some(m) = o.as_object_mut() {
            m.insert(
                "status".into(),
                body.get("status").cloned().unwrap_or(Value::Null),
            );
        }
        Ok(o.clone())
    }

    fn delete(&self, r: Resource, name: &str) -> Result<bool, KubeError> {
        let mut st = self.lock();
        Self::check_fail(&st, r)?;
        st.calls.push(format!("delete {r:?} {name}"));
        let Some(gone) = st.objects.remove(&(r, name.to_string())) else {
            return Ok(false);
        };
        // The garbage collector: children owned by the deleted object's uid.
        if let Some(uid) = gone.pointer("/metadata/uid").and_then(Value::as_str) {
            let uid = uid.to_string();
            st.objects.retain(|_, v| {
                !v.pointer("/metadata/ownerReferences")
                    .and_then(Value::as_array)
                    .is_some_and(|refs| refs.iter().any(|o| o["uid"] == uid.as_str()))
            });
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_encoding() {
        assert_eq!(
            Resource::Workspace.collection("bicraft.dev", "bi"),
            "/apis/bicraft.dev/v1alpha1/namespaces/bi/bicraftworkspaces"
        );
        assert_eq!(
            Resource::Claim.collection("g.x", "bi"),
            "/api/v1/namespaces/bi/persistentvolumeclaims"
        );
        assert_eq!(
            encode("app.kubernetes.io/managed-by=x"),
            "app.kubernetes.io%2Fmanaged-by%3Dx"
        );
        let k = HttpKube::with_url("http://127.0.0.1:8001/", "bicraft.dev", "bi").unwrap();
        assert_eq!(
            k.url(Resource::Deployment, Some("bws-a"), "/status", "x=1"),
            "http://127.0.0.1:8001/apis/apps/v1/namespaces/bi/deployments/bws-a/status?x=1"
        );
        assert!(HttpKube::with_url("ftp://x", "g.x", "bi").is_err());
        assert!(HttpKube::with_url("http://x", "g.x", "").is_err());
    }

    #[test]
    fn an_unreachable_api_is_an_error_not_a_crash() {
        let k = HttpKube::with_url("http://127.0.0.1:9", "bicraft.dev", "bi").unwrap();
        let e = k.list(Resource::Workspace, None).unwrap_err();
        assert_eq!(e.status, None);
    }

    #[test]
    fn the_fake_collects_garbage() {
        let k = FakeKube::new();
        let owner = k
            .create(Resource::Workspace, &json!({"metadata": {"name": "a"}}))
            .unwrap();
        let uid = owner["metadata"]["uid"].as_str().unwrap().to_string();
        k.apply(
            Resource::Deployment,
            "bws-a",
            &json!({"metadata": {"name": "bws-a", "ownerReferences": [{"uid": uid}]}}),
        )
        .unwrap();
        k.apply(
            Resource::Claim,
            "bws-a-data",
            &json!({"metadata": {"name": "bws-a-data"}}),
        )
        .unwrap();
        assert!(
            k.create(Resource::Workspace, &json!({"metadata": {"name": "a"}}))
                .is_err()
        );
        assert!(k.delete(Resource::Workspace, "a").unwrap());
        assert!(k.object(Resource::Deployment, "bws-a").is_none());
        assert!(k.object(Resource::Claim, "bws-a-data").is_some());
        assert!(!k.delete(Resource::Workspace, "a").unwrap());
    }
}
