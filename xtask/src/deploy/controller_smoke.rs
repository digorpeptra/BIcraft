//! `cargo xtask deploy controller-smoke`: the workspace controller's real flow in a throwaway kind
//! cluster. Needs docker, kind, helm and kubectl on PATH.
//!
//! 1. `helm lint` and `helm template` of `deploy/helm/bicraft-controller` (refused without
//!    authentication; ingress and HTTPRoute rendered);
//! 2. the image built (it compiles the workspace) unless `--no-build`;
//! 3. a generated fixture and three API tokens (admin, agent, other) made with the image;
//! 4. a kind cluster, the controller chart installed in a `restricted` namespace;
//! 5. through a port-forward to the gateway: a workspace created with the API (an agent may not),
//!    woken by its first MCP request, the live checks of `deploy smoke` (upload of the zipped
//!    fixture, `open_file`, a DAX query over SSE, sandbox refusal), another identity refused;
//! 6. a pod in the namespace cannot reach the workspace pod directly (NetworkPolicy), while it can
//!    reach the gateway;
//! 7. the idle stop to zero replicas, then a wake-up by the next request (the file is still on the
//!    volume: `open_file` and the DAX query again);
//! 8. a declarative workspace (`kubectl apply`) with `reclaimPolicy: Delete`; both deleted, the
//!    retained volume stays, the other goes;
//! 9. always (unless `--keep`): the cluster, the images and the build cache removed.

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::client::{self, Endpoint};
use super::kind::{Cleanup, mem_available_mib, missing_tools, stream, tool, uid_gid};

const USAGE: &str = "\
usage: cargo xtask deploy controller-smoke [--cluster NAME] [--image TAG] [--no-build] [--keep]

  --cluster NAME  kind cluster to create and delete (default bicraft-ctl-smoke)
  --image TAG     image tag (default bicraft:controller-smoke)
  --no-build      use the image TAG already built
  --keep          keep the cluster and the images afterwards
";

const NAMESPACE: &str = "bicraft-ctl";
const RELEASE: &str = "ctl";
/// The base of the probe pod (wget), and the single-platform image built from it: `kind load` cannot
/// import a multi-platform index from the containerd image store.
const PROBE_BASE: &str = "busybox:1.36.1";
const PROBE_IMAGE: &str = "bicraft-smoke-probe:1";
/// Idle timeout of the test workspace.
const IDLE_SECS: u64 = 20;

struct Opts {
    cluster: String,
    image: String,
    build: bool,
    keep: bool,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        cluster: "bicraft-ctl-smoke".into(),
        image: "bicraft:controller-smoke".into(),
        build: true,
        keep: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--cluster" => o.cluster = it.next().ok_or("--cluster needs a name")?.clone(),
            "--image" => o.image = it.next().ok_or("--image needs a tag")?.clone(),
            "--no-build" => o.build = false,
            "--keep" => o.keep = true,
            "-h" | "--help" => return Err(USAGE.to_owned()),
            other => {
                return Err(format!(
                    "deploy controller-smoke: unknown argument `{other}`\n\n{USAGE}"
                ));
            }
        }
    }
    Ok(o)
}

/// The probe image, removed whatever happens (declared before `Cleanup`, dropped first).
struct ExtraImages {
    keep: bool,
    images: Vec<String>,
}

impl Drop for ExtraImages {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        for i in &self.images {
            let _ = tool("docker", &["image", "rm", "-f", i]);
        }
    }
}

fn step(s: &str) {
    eprintln!("deploy controller-smoke: ok  {s}");
}

pub fn cmd(args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    let missing = missing_tools(&["docker", "kind", "helm", "kubectl"]);
    if !missing.is_empty() {
        return Err(format!(
            "deploy controller-smoke needs docker, kind, helm and kubectl on PATH; missing: {}",
            missing.join(", ")
        ));
    }
    let root = crate::root();
    let chart = root.join("deploy").join("helm").join("bicraft-controller");
    let chart_s = chart.to_string_lossy().into_owned();
    let t0 = Instant::now();
    let mut extra = ExtraImages {
        keep: o.keep,
        images: Vec::new(),
    };
    let mut cleanup = Cleanup {
        keep: o.keep,
        cluster: None,
        image: None,
        children: Vec::new(),
        scratch: None,
    };
    let scratch = std::env::temp_dir().join(format!("bicraft-ctl-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    cleanup.scratch = Some(scratch.clone());
    let write = |name: &str, text: &str| -> Result<String, String> {
        let p = scratch.join(name);
        std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))?;
        Ok(p.to_string_lossy().into_owned())
    };

    // 1. The chart, statically.
    let auth = write(
        "auth.yaml",
        "auth:\n  apiTokens:\n    existingSecret: tokens\ncontroller:\n  admins: [\"token:admin\"]\n",
    )?;
    let every = write(
        "every.yaml",
        "ingress:\n  enabled: true\n  hosts: [bi.example.com]\n\
         httpRoute:\n  enabled: true\n  parentRefs: [{name: gw}]\n  hostnames: [bi.example.com]\n\
         crd:\n  group: bicraft.example.com\n\
         networkPolicy:\n  apiServer:\n    cidrs: [172.18.0.0/16]\n  egressCidrs: [203.0.113.0/24]\n\
         workspaces:\n  egressProfiles:\n    databricks: {cidrs: [203.0.113.0/24], allowNetwork: true}\n",
    )?;
    tool("helm", &["lint", chart_s.as_str(), "--strict", "-f", &auth])?;
    if tool("helm", &["template", "r", chart_s.as_str()]).is_ok() {
        return Err("helm template without authentication must fail".into());
    }
    let full = tool(
        "helm",
        &["template", "r", chart_s.as_str(), "-f", &auth, "-f", &every],
    )?;
    for needed in [
        "kind: Ingress",
        "kind: HTTPRoute",
        "kind: CustomResourceDefinition",
        "name: bicraftworkspaces.bicraft.example.com",
        "kind: Role",
        "kind: NetworkPolicy",
    ] {
        if !full.contains(needed) {
            return Err(format!("helm template with every option: `{needed}` missing"));
        }
    }
    step("helm lint --strict, helm template (refusal without authentication, every option, CRD group)");

    // 2. The image.
    if o.build {
        if let Some(m) = mem_available_mib() {
            eprintln!("deploy controller-smoke: {m} MiB available before the image build");
        }
        stream(
            "docker",
            &["build", "-f", "deploy/docker/Dockerfile", "-t", &o.image, "."],
            &root,
        )?;
    }
    cleanup.image = Some(o.image.clone());

    let result = flow(&o, &chart_s, &scratch, &mut cleanup, &mut extra);
    if result.is_ok() {
        eprintln!(
            "deploy controller-smoke: PASSED in {:.0} s",
            t0.elapsed().as_secs_f64()
        );
    } else if let Some(c) = &cleanup.cluster {
        diagnose(&format!("kind-{c}"));
    }
    result
}

/// What the cluster looked like when a step failed (before it is deleted).
fn diagnose(ctx: &str) {
    for args in [
        &["get", "pods,deploy,svc,pvc,networkpolicy,bws", "-o", "wide"][..],
        &["get", "events", "--sort-by=.lastTimestamp"][..],
        &["describe", "pods"][..],
        &[
            "logs",
            "-l",
            "app.kubernetes.io/name=bicraft-controller",
            "--tail=200",
            "--previous",
        ][..],
        &[
            "logs",
            "-l",
            "app.kubernetes.io/name=bicraft-controller",
            "--tail=200",
        ][..],
        &[
            "logs",
            "-l",
            "app.kubernetes.io/component=workspace",
            "--tail=100",
        ][..],
    ] {
        let mut a = vec!["--context", ctx, "-n", NAMESPACE];
        a.extend(args);
        match Command::new("kubectl").args(&a).stdin(Stdio::null()).output() {
            Ok(o) => eprintln!(
                "deploy controller-smoke: diagnostics: kubectl {}\n{}{}",
                args.join(" "),
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ),
            Err(e) => eprintln!("deploy controller-smoke: kubectl: {e}"),
        }
    }
}

/// A token and its entry made by the image's binary.
fn make_token(
    bicraft: &dyn Fn(&[&str]) -> Result<String, String>,
    name: &str,
) -> Result<(String, String), String> {
    let out = bicraft(&["serve", "token", "create", "--name", name, "--role", "agent"])?;
    let token = out
        .lines()
        .find_map(|l| l.strip_prefix("token: "))
        .ok_or("no token")?
        .to_owned();
    let entry = out
        .lines()
        .find_map(|l| l.strip_prefix("entry: "))
        .ok_or("no entry")?
        .to_owned();
    Ok((token, entry))
}

/// Polls `f` every second until it returns `Some`, for at most `limit`.
fn wait_until<T>(
    limit: Duration,
    what: &str,
    mut f: impl FnMut() -> Option<T>,
) -> Result<(T, Duration), String> {
    let start = Instant::now();
    loop {
        if let Some(v) = f() {
            return Ok((v, start.elapsed()));
        }
        if start.elapsed() >= limit {
            return Err(format!(
                "timed out after {} s waiting for {what}",
                limit.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn flow(
    o: &Opts,
    chart: &str,
    scratch: &Path,
    cleanup: &mut Cleanup,
    extra: &mut ExtraImages,
) -> Result<(), String> {
    let user = uid_gid();
    let out = format!("{}:/out", scratch.display());
    let bicraft = |args: &[&str]| -> Result<String, String> {
        let mut a = vec![
            "run",
            "--rm",
            "--user",
            user.as_str(),
            "-v",
            out.as_str(),
            "--entrypoint",
            "/usr/local/bin/bicraft-cli",
            o.image.as_str(),
        ];
        a.extend(args);
        tool("docker", &a)
    };
    // 3. Fixture and tokens.
    bicraft(&["new-project", "/out/fixture", "Smoke"])?;
    let import = r#"table.import={"name":"Sales","csv":"Region,Amount\nNorth,10\nSouth,32"}"#;
    bicraft(&[
        "run",
        "--open",
        "/out/fixture/Smoke.pbip",
        "--cmd",
        import,
        "--save",
    ])?;
    let zip = super::smoke::zip_dir(&scratch.join("fixture"))?;
    let (admin, admin_e) = make_token(&bicraft, "admin")?;
    let (agent, agent_e) = make_token(&bicraft, "agent")?;
    let (other, other_e) = make_token(&bicraft, "other")?;
    let (viewer, viewer_e) = make_token(&bicraft, "viewer")?;
    let tokens_file = scratch.join("tokens.json");
    std::fs::write(
        &tokens_file,
        format!("{{\"tokens\": [{admin_e}, {agent_e}, {other_e}, {viewer_e}]}}"),
    )
    .map_err(|e| e.to_string())?;
    step(&format!(
        "fixture ({} bytes zipped) and four API tokens made by the image",
        zip.len()
    ));
    let probe_dir = scratch.join("probe");
    std::fs::create_dir_all(&probe_dir).map_err(|e| e.to_string())?;
    std::fs::write(probe_dir.join("Dockerfile"), format!("FROM {PROBE_BASE}\n"))
        .map_err(|e| e.to_string())?;
    extra.images.push(PROBE_IMAGE.into());
    extra.images.push(PROBE_BASE.into());
    tool(
        "docker",
        &["build", "-q", "-t", PROBE_IMAGE, &probe_dir.to_string_lossy()],
    )?;

    // 4. Cluster and chart.
    if let Some(m) = mem_available_mib() {
        eprintln!("deploy controller-smoke: {m} MiB available before the cluster");
        if m < 3000 {
            return Err(format!("only {m} MiB available: a kind cluster needs about 2 GB"));
        }
    }
    tool(
        "kind",
        &["create", "cluster", "--name", &o.cluster, "--wait", "180s"],
    )?;
    cleanup.cluster = Some(o.cluster.clone());
    let ctx = format!("kind-{}", o.cluster);
    let k = |args: &[&str]| -> Result<String, String> {
        let mut a = vec!["--context", ctx.as_str(), "-n", NAMESPACE];
        a.extend(args);
        tool("kubectl", &a)
    };
    let k_quiet = |args: &[&str]| -> Option<String> {
        let mut a = vec!["--context", ctx.as_str(), "-n", NAMESPACE];
        a.extend(args);
        Command::new("kubectl")
            .args(&a)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    };
    tool("kind", &["load", "docker-image", &o.image, "--name", &o.cluster])?;
    tool(
        "kind",
        &["load", "docker-image", PROBE_IMAGE, "--name", &o.cluster],
    )?;
    tool("kubectl", &["--context", &ctx, "create", "namespace", NAMESPACE])?;
    k(&[
        "label",
        "namespace",
        NAMESPACE,
        "pod-security.kubernetes.io/enforce=restricted",
    ])?;
    k(&[
        "create",
        "secret",
        "generic",
        "tokens",
        &format!("--from-file=tokens.json={}", tokens_file.display()),
    ])?;
    let (repo, tag) = o.image.rsplit_once(':').unwrap_or((o.image.as_str(), "latest"));
    let values = scratch.join("values.yaml");
    std::fs::write(
        &values,
        format!(
            "image:\n  repository: {repo}\n  tag: \"{tag}\"\n  pullPolicy: Never\n\
             auth:\n  apiTokens:\n    existingSecret: tokens\n\
             controller:\n  admins: [\"token:admin\"]\n  intervalSeconds: 2\n\
             workspaces:\n  defaultClass: tiny\n  classes:\n    tiny:\n      memoryLimitMiB: 448\n      cpu: 1\n      spillSize: 1Gi\n      volumeSize: 1Gi\n      maxVolumeSize: 2Gi\n\
             \x20 defaultIdleTimeoutSeconds: {IDLE_SECS}\n  pod:\n    shutdownGraceSeconds: 10\n"
        ),
    )
    .map_err(|e| e.to_string())?;
    let values_s = values.to_string_lossy().into_owned();
    if let Err(e) = tool(
        "helm",
        &[
            "--kube-context",
            &ctx,
            "install",
            RELEASE,
            chart,
            "-n",
            NAMESPACE,
            "--wait",
            "--timeout",
            "300s",
            "-f",
            &values_s,
        ],
    ) {
        let pods = k(&["describe", "pods"]).unwrap_or_default();
        let logs = k(&["logs", &format!("deploy/{RELEASE}-bicraft-controller")]).unwrap_or_default();
        return Err(format!("{e}\n{pods}\n{logs}"));
    }
    step("controller chart installed in a restricted namespace (CRD, Role, ConfigMap, gateway ready)");
    let svc = format!("{RELEASE}-bicraft-controller");

    // Controller log, followed.
    let mut logs = Command::new("kubectl")
        .args([
            "--context",
            &ctx,
            "-n",
            NAMESPACE,
            "logs",
            "-f",
            &format!("deploy/{svc}"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("kubectl logs: {e}"))?;
    let (ltx, lrx) = mpsc::channel::<String>();
    if let Some(out) = logs.stdout.take() {
        std::thread::spawn(move || {
            for l in BufReader::new(out).lines().map_while(Result::ok) {
                eprintln!("  [controller] {l}");
                let _ = ltx.send(l);
            }
        });
    }
    cleanup.children.push(logs);

    // 5. Port-forward to the gateway.
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map_err(|e| e.to_string())?
        .port();
    let mut pf = Command::new("kubectl")
        .args([
            "--context",
            &ctx,
            "-n",
            NAMESPACE,
            "port-forward",
            &format!("svc/{svc}"),
            &format!("{port}:7878"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("kubectl port-forward: {e}"))?;
    let pf_out = pf.stdout.take();
    cleanup.children.push(pf);
    let (tx, rx) = mpsc::channel();
    if let Some(out) = pf_out {
        std::thread::spawn(move || {
            for l in BufReader::new(out).lines().map_while(Result::ok) {
                if l.contains("Forwarding from") {
                    let _ = tx.send(());
                }
            }
        });
    }
    rx.recv_timeout(Duration::from_secs(30))
        .map_err(|_| "kubectl port-forward did not start".to_owned())?;
    let ep = |token: &str| Endpoint {
        addr: format!("127.0.0.1:{port}")
            .parse()
            .unwrap_or_else(|_| ([127, 0, 0, 1], port).into()),
        host: format!("{svc}.{NAMESPACE}.svc:7878"),
        token: token.to_owned(),
        workspace: "sales".into(),
    };
    let (adm, agt, oth, vwr) = (ep(&admin), ep(&agent), ep(&other), ep(&viewer));
    let api = |e: &Endpoint, method: &str, path: &str, body: &Value| -> Result<(u16, Value), String> {
        let b = if body.is_null() {
            String::new()
        } else {
            body.to_string()
        };
        let r = client::request(
            e,
            method,
            path,
            &[("Content-Type", "application/json")],
            b.as_bytes(),
            true,
        )?;
        let v = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
        Ok((r.status, v))
    };

    // The API.
    let spec = json!({
        "name": "sales",
        "grants": ["token:agent"],
        "readers": ["token:viewer"],
        "idleTimeoutSeconds": IDLE_SECS
    });
    let (s, v) = api(&agt, "POST", "/api/v1/workspaces", &spec)?;
    if s != 403 {
        return Err(format!("an agent created a workspace: {s} {v}"));
    }
    let (s, v) = api(&adm, "POST", "/api/v1/workspaces", &spec)?;
    if s != 201 {
        return Err(format!("POST /api/v1/workspaces: {s} {v}"));
    }
    step(&format!(
        "POST /api/v1/workspaces as admin -> 201 (phase {}), as agent -> 403",
        v["phase"]
    ));
    let (phase, _) = wait_until(Duration::from_secs(30), "phase Idle", || {
        k_quiet(&["get", "bws", "sales", "-o", "jsonpath={.status.phase}"]).filter(|p| p == "Idle")
    })?;
    let replicas = k(&["get", "deploy", "bws-sales", "-o", "jsonpath={.spec.replicas}"])?;
    let pvc = k(&["get", "pvc", "bws-sales-data", "-o", "jsonpath={.metadata.name}"])?;
    if replicas.trim() != "0" {
        return Err(format!(
            "a new workspace should start at 0 replicas, not {replicas}"
        ));
    }
    step(&format!(
        "kubectl get bws sales: {phase}, deployment bws-sales at 0 replicas, PVC {pvc}"
    ));
    let (_, list) = api(&oth, "GET", "/api/v1/workspaces", &Value::Null)?;
    if list.to_string().contains("\"sales\"") {
        return Err(format!("another identity sees the workspace: {list}"));
    }
    step("GET /api/v1/workspaces as another identity does not list it");

    // Wake on the first request.
    let t = Instant::now();
    let r = client::mcp(&agt, "server/discover", json!({}), false, true)?;
    if r.status != 200 {
        return Err(format!("first request (wake-up): HTTP {} {}", r.status, r.text()));
    }
    step(&format!(
        "first MCP request woke the workspace: 200 after {:.1} s (pod scheduled, volume bound, /readyz 200)",
        t.elapsed().as_secs_f64()
    ));
    let r = client::mcp(&oth, "server/discover", json!({}), false, true)?;
    if r.status != 404 {
        return Err(format!(
            "a foreign identity reached the workspace: HTTP {}",
            r.status
        ));
    }
    step("the same request with another identity's token -> 404 at the gateway");
    client::checks(&agt, &zip, "Smoke.pbip")?;
    // The viewer through the gateway: page and data as the editor and the reader, private views
    // (no one-time login: the gateway authenticates every request itself).
    client::viewer_checks(&agt, &viewer, false)?;
    client::concurrency_checks(&agt)?;
    // A reader, through the gateway: the pod applies the reader rules to the forwarded identity.
    let q = client::call(&vwr, "evaluate_dax_query", json!({"query": client::QUERY}), true)?;
    if q.get("isError") != Some(&Value::Bool(false)) {
        return Err(format!("a reader could not query: {q}"));
    }
    let r = client::request(&vwr, "PUT", "/w/sales/files/reader.txt", &[], b"x", true)?;
    if (200..300).contains(&r.status) {
        return Err(format!("a reader uploaded a file: HTTP {}", r.status));
    }
    let (s, _) = api(&vwr, "POST", "/api/v1/workspaces/sales/stop", &Value::Null)?;
    if s != 403 {
        return Err(format!("a reader stopped the workspace: {s}"));
    }
    step(&format!(
        "a reader (spec.readers) queries through the gateway; its upload is refused by the pod (HTTP {}), stop -> 403",
        r.status
    ));

    // 6. Direct access from another pod.
    let pod_ip = k(&[
        "get",
        "pods",
        "-l",
        "app.kubernetes.io/instance=bws-sales",
        "-o",
        "jsonpath={.items[0].status.podIP}",
    ])?;
    let probe = |name: &str, url: &str, header: Option<&str>| -> Result<(bool, String), String> {
        let mut cmd = format!("wget -q -T 5 -O - {url}");
        if let Some(h) = header {
            cmd = format!("wget -q -T 5 --header '{h}' -O - {url}");
        }
        let overrides = json!({"spec": {
            "securityContext": {"runAsNonRoot": true, "runAsUser": 65534, "seccompProfile": {"type": "RuntimeDefault"}},
            "containers": [{
                "name": name, "image": PROBE_IMAGE, "imagePullPolicy": "Never",
                "command": ["sh", "-c", cmd],
                "securityContext": {"allowPrivilegeEscalation": false, "capabilities": {"drop": ["ALL"]}}
            }]
        }})
        .to_string();
        k(&[
            "run",
            name,
            "--image",
            PROBE_IMAGE,
            "--restart=Never",
            "--overrides",
            &overrides,
        ])?;
        let (phase, _) = wait_until(Duration::from_secs(90), "the probe pod", || {
            k_quiet(&["get", "pod", name, "-o", "jsonpath={.status.phase}"])
                .filter(|p| p == "Succeeded" || p == "Failed")
        })?;
        let out = k_quiet(&["logs", name]).unwrap_or_default();
        let _ = k(&["delete", "pod", name, "--wait=false"]);
        Ok((phase == "Succeeded", out))
    };
    let (ok, out) = probe("probe-gateway", &format!("http://{svc}:7878/healthz"), None)?;
    if !ok {
        return Err(format!("a pod could not reach the gateway (control): {out}"));
    }
    let (ok, out) = probe(
        "probe-direct",
        &format!("http://{}:7878/healthz", pod_ip.trim()),
        None,
    )?;
    if ok {
        return Err(format!(
            "a pod reached the workspace pod directly ({out}): the NetworkPolicy is not enforced"
        ));
    }
    let (ok, out) = probe(
        "probe-forged",
        "http://bws-sales:7878/w/sales/files/",
        Some("X-Bicraft-Gateway-User: token:agent"),
    )?;
    if ok {
        return Err(format!("a forged identity header reached the workspace: {out}"));
    }
    step(&format!(
        "a pod in the namespace reaches the gateway but not the workspace pod {} (NetworkPolicy), even with a forged identity header",
        pod_ip.trim()
    ));

    // 7. Idle stop, then wake on request.
    let (_, waited) = wait_until(Duration::from_secs(IDLE_SECS + 90), "the idle stop", || {
        k_quiet(&["get", "deploy", "bws-sales", "-o", "jsonpath={.spec.replicas}"]).filter(|r| r == "0")
    })?;
    wait_until(Duration::from_secs(60), "no pod left", || {
        k_quiet(&[
            "get",
            "pods",
            "-l",
            "app.kubernetes.io/instance=bws-sales",
            "-o",
            "name",
        ])
        .filter(|p| p.is_empty())
    })?;
    let idle_line = lrx.try_iter().find(|l| l.contains("idle for"));
    step(&format!(
        "idle stop: scaled to zero after {:.0} s more of waiting (idle timeout {IDLE_SECS} s), pod gone; log: {}",
        waited.as_secs_f64(),
        idle_line.unwrap_or_default()
    ));
    let phase = k(&["get", "bws", "sales", "-o", "jsonpath={.status.phase}"])?;
    let t = Instant::now();
    let opened = client::call(&agt, "open_file", json!({"path": "Smoke.pbip"}), false)?;
    if opened.get("isError") != Some(&Value::Bool(false)) {
        return Err(format!("open_file after the wake-up: {opened}"));
    }
    let wake = t.elapsed();
    let q = client::call(&agt, "evaluate_dax_query", json!({"query": client::QUERY}), true)?;
    let text = q
        .pointer("/content/0/text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let answer: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let total = answer
        .pointer("/results/0/rows/0")
        .and_then(Value::as_object)
        .and_then(|row| row.values().find_map(Value::as_f64));
    if total != Some(client::EXPECTED_TOTAL) {
        return Err(format!("DAX after the wake-up: {answer}"));
    }
    step(&format!(
        "phase {phase}; the next request woke it ({:.1} s), Smoke.pbip still on the volume, DAX -> {}",
        wake.as_secs_f64(),
        client::EXPECTED_TOTAL
    ));

    // 8. Declarative workspace with reclaimPolicy Delete; deletions.
    let manifest = write_file(
        scratch,
        "scratch.yaml",
        "apiVersion: bicraft.dev/v1alpha1\nkind: BicraftWorkspace\nmetadata:\n  name: scratch\nspec:\n  volume:\n    reclaimPolicy: Delete\n",
    )?;
    k(&["apply", "-f", &manifest])?;
    wait_until(Duration::from_secs(30), "the scratch PVC", || {
        k_quiet(&["get", "pvc", "bws-scratch-data", "-o", "name"]).filter(|s| !s.is_empty())
    })?;
    step("kubectl apply of a BicraftWorkspace (reclaimPolicy Delete): its objects created");
    let (s, _) = api(&agt, "DELETE", "/api/v1/workspaces/sales", &Value::Null)?;
    if s != 403 {
        return Err(format!("an agent deleted a workspace: {s}"));
    }
    let (s, v) = api(&adm, "DELETE", "/api/v1/workspaces/sales", &Value::Null)?;
    if s != 202 {
        return Err(format!("DELETE sales: {s} {v}"));
    }
    let (s, v2) = api(&adm, "DELETE", "/api/v1/workspaces/scratch", &Value::Null)?;
    if s != 202 {
        return Err(format!("DELETE scratch: {s} {v2}"));
    }
    wait_until(Duration::from_secs(90), "the garbage collector", || {
        let d = k_quiet(&[
            "get",
            "deploy,svc,networkpolicy",
            "-l",
            "app.kubernetes.io/managed-by=bicraft-controller",
            "-o",
            "name",
        ])?;
        let gone = k_quiet(&["get", "pvc", "bws-scratch-data", "-o", "name"]).is_none();
        (d.is_empty() && gone).then_some(())
    })?;
    let kept = k(&["get", "pvc", "bws-sales-data", "-o", "jsonpath={.status.phase}"])?;
    step(&format!(
        "DELETE as agent -> 403; as admin -> 202 ({}); Deployments, Services, NetworkPolicies collected; \
         retained PVC bws-sales-data {kept}, PVC bws-scratch-data deleted",
        v["volume"]
    ));
    let panics: Vec<String> = lrx.try_iter().filter(|l| l.contains("panicked")).collect();
    if !panics.is_empty() {
        return Err(format!("the controller panicked: {panics:?}"));
    }
    Ok(())
}

fn write_file(dir: &Path, name: &str, text: &str) -> Result<String, String> {
    let p = dir.join(name);
    std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(p.to_string_lossy().into_owned())
}
