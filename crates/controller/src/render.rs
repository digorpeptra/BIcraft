//! The objects the controller owns for one workspace, as server-side apply bodies: a
//! `Deployment` (one replica when awake, none when idle), a `Service`, a `NetworkPolicy` and a
//! `PersistentVolumeClaim`. The pod is the stage-1 chart's (`deploy/helm/bicraft`): same
//! security context, probes, sizing rules; `cargo xtask deploy check` keeps the two in step.

use serde_json::{Map, Value, json};

use crate::config::{
    Config, EDITOR_GROUP, EMAIL_HEADER, GROUPS_HEADER, KIND, MANAGER, POD_MARGIN_MIB, READER_GROUP, Reclaim,
    USER_HEADER, VERSION,
};
use crate::spec::{Workspace, claim_name, object_name, recovery_claim_name};

/// Where `bicraft serve` keeps its workspaces in the pod.
pub const ROOT: &str = "/srv/bicraft";
/// Where the recovery journals' volume is mounted (`--recovery-dir`; the server writes
/// `<dir>/<workspace>/`): outside [`ROOT`], so no file listing or download reaches them.
pub const RECOVERY_DIR: &str = "/var/lib/bicraft/recovery";

/// The labels of every object of workspace `ws`.
pub fn labels(cfg: &Config, ws: &str) -> Value {
    json!({
        "app.kubernetes.io/name": "bicraft",
        "app.kubernetes.io/instance": object_name(ws),
        "app.kubernetes.io/component": "workspace",
        "app.kubernetes.io/managed-by": MANAGER,
        workspace_label(cfg): ws,
    })
}

/// The label naming the workspace (`<group>/workspace`).
pub fn workspace_label(cfg: &Config) -> String {
    format!("{}/workspace", cfg.crd_group)
}

/// The selector of the managed objects (all workspaces).
pub fn managed_selector() -> String {
    format!("app.kubernetes.io/managed-by={MANAGER}")
}

fn selector_labels(ws: &str) -> Value {
    json!({
        "app.kubernetes.io/name": "bicraft",
        "app.kubernetes.io/instance": object_name(ws),
    })
}

fn owner(cfg: &Config, w: &Workspace) -> Value {
    // No blockOwnerDeletion: setting it needs rights on the owner's finalizers, which the
    // controller does not have (least privilege); the garbage collector deletes children anyway.
    json!([{
        "apiVersion": format!("{}/{VERSION}", cfg.crd_group),
        "kind": KIND,
        "name": w.name,
        "uid": w.uid,
        "controller": true,
    }])
}

fn metadata(cfg: &Config, w: &Workspace, name: String, owned: bool) -> Value {
    let mut m = json!({
        "name": name,
        "namespace": cfg.namespace,
        "labels": labels(cfg, &w.name),
    });
    if owned && let Some(o) = m.as_object_mut() {
        o.insert("ownerReferences".into(), owner(cfg, w));
    }
    m
}

/// The `Host` names a workspace pod answers: its Service's names (the gateway rewrites `Host`).
pub fn service_hosts(cfg: &Config, ws: &str) -> Vec<String> {
    let svc = object_name(ws);
    let ns = &cfg.namespace;
    let mut out = Vec::new();
    for n in [
        svc.clone(),
        format!("{svc}.{ns}"),
        format!("{svc}.{ns}.svc"),
        format!("{svc}.{ns}.svc.cluster.local"),
    ] {
        out.push(format!("{n}:{}", cfg.port));
        out.push(n);
    }
    out
}

/// The `Host` the gateway sends upstream.
pub fn upstream_host(cfg: &Config, ws: &str) -> String {
    format!("{}.{}.svc:{}", object_name(ws), cfg.namespace, cfg.port)
}

fn env(name: &str, value: impl Into<String>) -> Value {
    json!({"name": name, "value": value.into()})
}

/// The Deployment, with `replicas` 0 or 1. `trusted` are the addresses the pod accepts the
/// gateway's identity headers from.
pub fn deployment(cfg: &Config, w: &Workspace, replicas: u8, trusted: &[String]) -> Value {
    let c = &w.class;
    let pod_mem = if c.memory_limit_mib > 0 {
        c.memory_limit_mib.saturating_add(POD_MARGIN_MIB)
    } else {
        0
    };
    let p = &cfg.pod;
    let mut args: Vec<String> = vec![
        "--root".into(),
        ROOT.into(),
        "--listen".into(),
        format!("0.0.0.0:{}", cfg.port),
        "--shutdown-grace".into(),
        p.shutdown_grace_seconds.to_string(),
        "--max-upload-bytes".into(),
        p.max_upload_bytes.clone(),
        // One JSON line per request, without model data or user ids (docs/kubernetes.md, "Observability").
        "--log-format".into(),
        "json".into(),
        "--proxy-user-header".into(),
        USER_HEADER.into(),
        "--proxy-email-header".into(),
        EMAIL_HEADER.into(),
        "--proxy-groups-header".into(),
        GROUPS_HEADER.into(),
    ];
    // the public origin browsers send (the viewer's CSRF check; the gateway rewrites `Host`)
    if !cfg.public_url.is_empty() && !p.allowed_origins.contains(&cfg.public_url) {
        args.push("--allow-origin".into());
        args.push(cfg.public_url.trim_end_matches('/').to_string());
    }
    for o in &p.allowed_origins {
        args.push("--allow-origin".into());
        args.push(o.clone());
    }
    // document views (browser tabs on their own file): the server's defaults unless set
    for (flag, n) in [
        ("--max-views-per-user", u64::from(p.max_views_per_user)),
        ("--max-views", u64::from(p.max_views)),
        ("--view-idle", p.view_idle_seconds),
    ] {
        if n > 0 {
            args.push(flag.into());
            args.push(n.to_string());
        }
    }
    if cfg.recovery.enabled {
        args.push("--recovery-dir".into());
        args.push(RECOVERY_DIR.into());
    }
    if p.allow_autosave {
        args.push("--allow-autosave".into());
    }
    if p.viewer_control.is_some() {
        // agents reach it through the gateway, which forwards X-Bicraft-Control and the
        // /w/<id>/view/api/control routes as they are
        args.push("--viewer-control".into());
    }
    if !p.local_offset.is_empty() {
        args.push("--local-offset".into());
        args.push(p.local_offset.clone());
    }
    args.extend(p.extra_args.iter().cloned());
    let network = w.egress.iter().any(|(_, e)| e.allow_network);
    let mut envs = vec![env("TMPDIR", "/tmp"), env("BICRAFT_SPILL_DIR", "/tmp")];
    if c.memory_limit_mib > 0 {
        envs.push(env("BICRAFT_MEMORY_LIMIT", format!("{}M", c.memory_limit_mib)));
    }
    if c.memory_budget_mib > 0 {
        envs.push(env("BICRAFT_MEMORY_BUDGET", format!("{}M", c.memory_budget_mib)));
    }
    envs.push(env("BICRAFT_SCAN_THREADS", c.cpu.to_string()));
    envs.push(env("BICRAFT_AGG_THREADS", (c.cpu / 4).max(1).to_string()));
    envs.push(env(
        "BICRAFT_AGGREGATIONS",
        if c.aggregations { "1" } else { "0" },
    ));
    envs.push(env("BICRAFT_SERVER_NETWORK", if network { "1" } else { "0" }));
    envs.push(env(
        "BICRAFT_ALLOWED_HOSTS",
        service_hosts(cfg, &w.name).join(","),
    ));
    envs.push(env("BICRAFT_TRUSTED_PROXIES", trusted.join(",")));
    // The gateway has already checked the workspace's grants and says which access it granted in
    // a group; only it can reach the pod (NetworkPolicy) and only its address may send the identity
    // headers. Readers get the read-only command set (docs/auth.md "Roles").
    envs.push(env(
        "BICRAFT_AUTH_GRANTS",
        format!("group:{EDITOR_GROUP}={}", w.name),
    ));
    envs.push(env(
        "BICRAFT_AUTH_READER_GRANTS",
        format!("group:{READER_GROUP}={}", w.name),
    ));
    if let Some(s) = w.egress.iter().find_map(|(_, e)| e.databricks_secret.as_ref()) {
        envs.push(json!({
            "name": "BICRAFT_DATABRICKS_TOKEN",
            "valueFrom": {"secretKeyRef": {"name": s.name, "key": s.key}},
        }));
    }
    if let Some(s) = &p.viewer_control {
        envs.push(json!({
            "name": "BICRAFT_VIEWER_CONTROL_TOKEN",
            "valueFrom": {"secretKeyRef": {"name": s.name, "key": s.key}},
        }));
    }
    envs.extend(p.extra_env.iter().cloned());

    let mut requests = Map::new();
    let mut limits = Map::new();
    let cpu_request = if c.cpu_request.is_empty() {
        c.cpu.to_string()
    } else {
        c.cpu_request.clone()
    };
    requests.insert("cpu".into(), cpu_request.into());
    limits.insert("cpu".into(), c.cpu.to_string().into());
    if pod_mem > 0 {
        let req = if c.memory_request_mib > 0 {
            c.memory_request_mib
        } else {
            pod_mem
        };
        requests.insert("memory".into(), format!("{req}Mi").into());
        limits.insert("memory".into(), format!("{pod_mem}Mi").into());
    }
    let container_security = json!({
        "runAsNonRoot": true,
        "allowPrivilegeEscalation": false,
        "readOnlyRootFilesystem": true,
        "privileged": false,
        "capabilities": {"drop": ["ALL"]},
        "seccompProfile": {"type": "RuntimeDefault"},
    });
    let mut mounts = vec![
        json!({"name": "workspace", "mountPath": format!("{ROOT}/{}", w.name)}),
        json!({"name": "tmp", "mountPath": "/tmp"}),
    ];
    let mut volumes = vec![
        json!({"name": "workspace", "persistentVolumeClaim": {"claimName": claim_name(&w.name)}}),
        // Disk-backed (never the Memory medium): spill files must not count against the pod's
        // memory.
        json!({"name": "tmp", "emptyDir": {"sizeLimit": c.spill_size}}),
    ];
    if cfg.recovery.enabled {
        mounts.push(json!({"name": "recovery", "mountPath": RECOVERY_DIR}));
        volumes.push(json!({"name": "recovery",
                            "persistentVolumeClaim": {"claimName": recovery_claim_name(&w.name)}}));
    }
    let container = json!({
        "name": "bicraft",
        "image": cfg.image,
        "imagePullPolicy": cfg.image_pull_policy,
        "args": args,
        "env": envs,
        "ports": [{"name": "http", "containerPort": cfg.port, "protocol": "TCP"}],
        "livenessProbe": {
            "httpGet": {"path": "/healthz", "port": "http"},
            "initialDelaySeconds": 5, "periodSeconds": 10, "timeoutSeconds": 5, "failureThreshold": 6,
        },
        "readinessProbe": {
            "httpGet": {"path": "/readyz", "port": "http"},
            "periodSeconds": 2, "timeoutSeconds": 5, "failureThreshold": 2,
        },
        "resources": {"requests": requests, "limits": limits},
        "securityContext": container_security,
        "volumeMounts": mounts,
    });
    let mut pod_spec = json!({
        // BIcraft never talks to the Kubernetes API.
        "automountServiceAccountToken": false,
        // No <SERVICE>_PORT variables: a Service named bicraft would inject BICRAFT_* names.
        "enableServiceLinks": false,
        "terminationGracePeriodSeconds": p.shutdown_grace_seconds.saturating_add(5),
        "securityContext": {
            "runAsNonRoot": true,
            "runAsUser": p.run_as_user,
            "runAsGroup": p.run_as_group,
            "fsGroup": p.fs_group,
            "fsGroupChangePolicy": "OnRootMismatch",
            "seccompProfile": {"type": "RuntimeDefault"},
        },
        "containers": [container],
        "volumes": volumes,
    });
    if let Some(o) = pod_spec.as_object_mut() {
        if !p.node_selector.is_empty() {
            o.insert("nodeSelector".into(), json!(p.node_selector));
        }
        if !p.tolerations.is_empty() {
            o.insert("tolerations".into(), json!(p.tolerations));
        }
        if !p.priority_class_name.is_empty() {
            o.insert("priorityClassName".into(), json!(p.priority_class_name));
        }
        if !p.image_pull_secrets.is_empty() {
            let secrets: Vec<Value> = p.image_pull_secrets.iter().map(|n| json!({"name": n})).collect();
            o.insert("imagePullSecrets".into(), Value::Array(secrets));
        }
    }
    let mut template_meta = json!({"labels": labels(cfg, &w.name)});
    if !p.annotations.is_empty()
        && let Some(o) = template_meta.as_object_mut()
    {
        o.insert("annotations".into(), json!(p.annotations));
    }
    json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": metadata(cfg, w, object_name(&w.name), true),
        "spec": {
            // One session per workspace: never two pods on one volume (ReadWriteOnce).
            "replicas": replicas,
            "strategy": {"type": "Recreate"},
            "selector": {"matchLabels": selector_labels(&w.name)},
            "template": {"metadata": template_meta, "spec": pod_spec},
        },
    })
}

pub fn service(cfg: &Config, w: &Workspace) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Service",
        "metadata": metadata(cfg, w, object_name(&w.name), true),
        "spec": {
            "type": "ClusterIP",
            "selector": selector_labels(&w.name),
            "ports": [{"name": "http", "port": cfg.port, "targetPort": "http", "protocol": "TCP"}],
        },
    })
}

/// Ingress from the controller's pods only; egress DNS plus the workspace's egress profiles.
pub fn network_policy(cfg: &Config, w: &Workspace) -> Value {
    let mut egress = vec![json!({
        "to": [{
            "namespaceSelector": cfg.dns_namespace_selector,
            "podSelector": cfg.dns_pod_selector,
        }],
        "ports": [{"port": 53, "protocol": "UDP"}, {"port": 53, "protocol": "TCP"}],
    })];
    for (_, e) in &w.egress {
        if e.cidrs.is_empty() {
            continue;
        }
        let to: Vec<Value> = e.cidrs.iter().map(|c| json!({"ipBlock": {"cidr": c}})).collect();
        let ports: Vec<Value> = e
            .ports
            .iter()
            .map(|p| json!({"port": p, "protocol": "TCP"}))
            .collect();
        egress.push(json!({"to": to, "ports": ports}));
    }
    json!({
        "apiVersion": "networking.k8s.io/v1",
        "kind": "NetworkPolicy",
        "metadata": metadata(cfg, w, object_name(&w.name), true),
        "spec": {
            "podSelector": {"matchLabels": selector_labels(&w.name)},
            "policyTypes": ["Ingress", "Egress"],
            "ingress": [{
                "from": [{"podSelector": {"matchLabels": cfg.controller_pod_labels}}],
                "ports": [{"port": "http", "protocol": "TCP"}],
            }],
            "egress": egress,
        },
    })
}

/// The volume: owned by the workspace (deleted with it) only with `reclaimPolicy: Delete`.
pub fn claim(cfg: &Config, w: &Workspace) -> Value {
    let mut spec = json!({
        "accessModes": ["ReadWriteOnce"],
        "resources": {"requests": {"storage": w.volume_size}},
    });
    if !w.storage_class.is_empty()
        && let Some(o) = spec.as_object_mut()
    {
        o.insert("storageClassName".into(), json!(w.storage_class));
    }
    json!({
        "apiVersion": "v1",
        "kind": "PersistentVolumeClaim",
        "metadata": metadata(cfg, w, claim_name(&w.name), w.reclaim == Reclaim::Delete),
        "spec": spec,
    })
}

/// The recovery journals' volume (`None` when the configuration turns auto-recovery off): small,
/// on the workspace's storage class unless the configuration names one, kept or deleted with the
/// workspace as its files are.
pub fn recovery_claim(cfg: &Config, w: &Workspace) -> Option<Value> {
    let r = &cfg.recovery;
    if !r.enabled {
        return None;
    }
    let mut spec = json!({
        "accessModes": ["ReadWriteOnce"],
        "resources": {"requests": {"storage": r.volume_size}},
    });
    let class = if r.storage_class.is_empty() {
        &w.storage_class
    } else {
        &r.storage_class
    };
    if !class.is_empty()
        && let Some(o) = spec.as_object_mut()
    {
        o.insert("storageClassName".into(), json!(class));
    }
    Some(json!({
        "apiVersion": "v1",
        "kind": "PersistentVolumeClaim",
        "metadata": metadata(cfg, w, recovery_claim_name(&w.name), w.reclaim == Reclaim::Delete),
        "spec": spec,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::sample;
    use crate::spec::{WorkspaceSpec, resolve};

    fn ws(spec: WorkspaceSpec) -> Workspace {
        resolve(&sample(), "sales", "uid-1", 1, &spec).unwrap()
    }

    fn env_of<'a>(d: &'a Value, name: &str) -> Option<&'a Value> {
        d.pointer("/spec/template/spec/containers/0/env")?
            .as_array()?
            .iter()
            .find(|e| e["name"] == name)
    }

    #[test]
    fn the_pod_keeps_the_stage_1_invariants() {
        let cfg = sample();
        let d = deployment(&cfg, &ws(WorkspaceSpec::default()), 1, &["10.0.0.7/32".into()]);
        let c = &d["spec"]["template"]["spec"]["containers"][0];
        assert_eq!(d["spec"]["replicas"], 1);
        assert_eq!(d["spec"]["strategy"]["type"], "Recreate");
        assert_eq!(c["resources"]["limits"]["memory"], "512Mi");
        assert_eq!(c["securityContext"]["readOnlyRootFilesystem"], true);
        assert_eq!(c["securityContext"]["capabilities"]["drop"][0], "ALL");
        assert_eq!(
            d["spec"]["template"]["spec"]["securityContext"]["runAsUser"],
            65532
        );
        assert_eq!(
            d["spec"]["template"]["spec"]["automountServiceAccountToken"],
            false
        );
        assert_eq!(c["volumeMounts"][0]["mountPath"], "/srv/bicraft/sales");
        assert!(
            d["spec"]["template"]["spec"]["volumes"][1]["emptyDir"]
                .get("medium")
                .is_none()
        );
        assert_eq!(env_of(&d, "BICRAFT_MEMORY_LIMIT").unwrap()["value"], "448M");
        assert_eq!(env_of(&d, "BICRAFT_SCAN_THREADS").unwrap()["value"], "1");
        assert_eq!(
            env_of(&d, "BICRAFT_AUTH_GRANTS").unwrap()["value"],
            "group:bicraft-gateway-editor=sales"
        );
        assert_eq!(
            env_of(&d, "BICRAFT_AUTH_READER_GRANTS").unwrap()["value"],
            "group:bicraft-gateway-reader=sales"
        );
        assert_eq!(
            env_of(&d, "BICRAFT_TRUSTED_PROXIES").unwrap()["value"],
            "10.0.0.7/32"
        );
        assert_eq!(env_of(&d, "BICRAFT_SERVER_NETWORK").unwrap()["value"], "0");
        assert!(env_of(&d, "BICRAFT_DATABRICKS_TOKEN").is_none());
        let hosts = env_of(&d, "BICRAFT_ALLOWED_HOSTS").unwrap()["value"]
            .as_str()
            .unwrap();
        assert!(hosts.contains("bws-sales.bi.svc:7878"));
        assert_eq!(d["metadata"]["ownerReferences"][0]["uid"], "uid-1");
        assert_eq!(
            d["metadata"]["ownerReferences"][0]["apiVersion"],
            "bicraft.dev/v1alpha1"
        );
        let args: Vec<&str> = c["args"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(args.windows(2).any(|w| w == ["--proxy-user-header", USER_HEADER]));
        assert!(args.windows(2).any(|w| w == ["--log-format", "json"]));
        assert_eq!(d["spec"]["template"]["spec"]["terminationGracePeriodSeconds"], 35);
    }

    #[test]
    fn the_public_origin_is_an_allowed_origin_of_the_pod() {
        // the viewer's CSRF check compares Origin with it: the gateway rewrites Host
        let mut cfg = sample();
        cfg.public_url = "https://bi.example.com/".into();
        let d = deployment(&cfg, &ws(WorkspaceSpec::default()), 1, &[]);
        let args: Vec<&str> = d["spec"]["template"]["spec"]["containers"][0]["args"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(
            args.windows(2)
                .any(|w| w == ["--allow-origin", "https://bi.example.com"])
        );
    }

    #[test]
    fn egress_profiles_open_the_network() {
        let cfg = sample();
        let w = ws(WorkspaceSpec {
            egress: vec!["databricks".into()],
            size_class: Some("large".into()),
            ..WorkspaceSpec::default()
        });
        let d = deployment(&cfg, &w, 0, &[]);
        assert_eq!(d["spec"]["replicas"], 0);
        assert_eq!(env_of(&d, "BICRAFT_SERVER_NETWORK").unwrap()["value"], "1");
        assert_eq!(
            env_of(&d, "BICRAFT_DATABRICKS_TOKEN").unwrap()["valueFrom"]["secretKeyRef"]["name"],
            "dbx"
        );
        assert_eq!(env_of(&d, "BICRAFT_AGG_THREADS").unwrap()["value"], "2");
        assert_eq!(env_of(&d, "BICRAFT_AGGREGATIONS").unwrap()["value"], "0");
        let np = network_policy(&cfg, &w);
        assert_eq!(
            np["spec"]["egress"][1]["to"][0]["ipBlock"]["cidr"],
            "203.0.113.0/24"
        );
        assert_eq!(
            np["spec"]["ingress"][0]["from"][0]["podSelector"]["matchLabels"]["app.kubernetes.io/name"],
            "bicraft-controller"
        );
    }

    fn args_of(d: &Value) -> Vec<String> {
        d["spec"]["template"]["spec"]["containers"][0]["args"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn recovery_views_and_the_control_channel_reach_the_pod() {
        // the defaults: journals on a volume of their own, outside the workspace's files; the
        // server's own view caps; no control channel, no autosave, UTC
        let cfg = sample();
        let w = ws(WorkspaceSpec::default());
        let d = deployment(&cfg, &w, 1, &[]);
        let args = args_of(&d);
        assert!(args.windows(2).any(|a| a == ["--recovery-dir", RECOVERY_DIR]));
        assert!(!RECOVERY_DIR.starts_with(ROOT));
        for flag in [
            "--max-views-per-user",
            "--max-views",
            "--view-idle",
            "--viewer-control",
            "--allow-autosave",
            "--local-offset",
        ] {
            assert!(!args.iter().any(|a| a == flag), "{flag}");
        }
        assert!(env_of(&d, "BICRAFT_VIEWER_CONTROL_TOKEN").is_none());
        let pod = &d["spec"]["template"]["spec"];
        assert_eq!(pod["containers"][0]["volumeMounts"][2]["mountPath"], RECOVERY_DIR);
        assert_eq!(
            pod["volumes"][2]["persistentVolumeClaim"]["claimName"],
            "bws-sales-recovery"
        );
        let rc = recovery_claim(&cfg, &w).unwrap();
        assert_eq!(rc["metadata"]["name"], "bws-sales-recovery");
        assert_eq!(rc["spec"]["resources"]["requests"]["storage"], "1Gi");
        assert!(
            rc["metadata"].get("ownerReferences").is_none(),
            "kept like the files"
        );

        // everything set
        let mut cfg = sample();
        cfg.pod.max_views_per_user = 4;
        cfg.pod.max_views = 16;
        cfg.pod.view_idle_seconds = 600;
        cfg.pod.allow_autosave = true;
        cfg.pod.local_offset = "+02:00".into();
        cfg.pod.viewer_control = Some(crate::config::SecretKey {
            name: "ctl".into(),
            key: "token".into(),
        });
        let d = deployment(&cfg, &w, 1, &[]);
        let args = args_of(&d);
        for pair in [
            ["--max-views-per-user", "4"],
            ["--max-views", "16"],
            ["--view-idle", "600"],
            ["--local-offset", "+02:00"],
        ] {
            assert!(args.windows(2).any(|a| a == pair), "{pair:?}");
        }
        assert!(args.iter().any(|a| a == "--viewer-control"));
        assert!(args.iter().any(|a| a == "--allow-autosave"));
        assert_eq!(
            env_of(&d, "BICRAFT_VIEWER_CONTROL_TOKEN").unwrap()["valueFrom"]["secretKeyRef"],
            json!({"name": "ctl", "key": "token"})
        );

        // off: no claim, no flag, no mount
        cfg.recovery.enabled = false;
        let d = deployment(&cfg, &w, 1, &[]);
        assert!(!args_of(&d).iter().any(|a| a == "--recovery-dir"));
        assert_eq!(
            d["spec"]["template"]["spec"]["volumes"].as_array().unwrap().len(),
            2
        );
        assert!(recovery_claim(&cfg, &w).is_none());
    }

    #[test]
    fn the_claim_is_owned_only_when_deleted_with_the_workspace() {
        let cfg = sample();
        let kept = claim(&cfg, &ws(WorkspaceSpec::default()));
        assert!(kept["metadata"].get("ownerReferences").is_none());
        assert_eq!(kept["metadata"]["name"], "bws-sales-data");
        let mut spec = WorkspaceSpec::default();
        spec.volume.reclaim_policy = Some(Reclaim::Delete);
        spec.volume.size = Some("3Gi".into());
        let gone = claim(&cfg, &ws(spec));
        assert_eq!(gone["metadata"]["ownerReferences"][0]["kind"], "BicraftWorkspace");
        assert_eq!(gone["spec"]["resources"]["requests"]["storage"], "3Gi");
    }
}
