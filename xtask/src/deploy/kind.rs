//! `cargo xtask deploy kind-smoke`: the real flow in a throwaway kind cluster. Needs docker, kind,
//! helm and kubectl on PATH (it says which are missing and fails otherwise).
//!
//! 1. `helm lint` and `helm template` of the chart (a configuration without authentication must be
//!    refused; ingress, HTTPRoute and Cilium policy rendered);
//! 2. the image built from `deploy/docker/Dockerfile` (it compiles the workspace);
//! 3. the fixture and the API token made with the image itself (`docker run`);
//! 4. a kind cluster created, the image loaded, `kubectl apply --dry-run=server` of the plain
//!    manifests, the chart installed with the token's Secret;
//! 5. `kubectl port-forward` and the same live checks as `deploy smoke` (the viewer's too, with a
//!    reader token and a reader grant);
//! 6. the pod scaled to zero: its log must end with a clean drain (exit 0);
//! 7. always (also on failure, unless `--keep`): the cluster deleted, the image removed, dangling
//!    images and the build cache pruned.

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::client::{self, Endpoint};

const USAGE: &str = "\
usage: cargo xtask deploy kind-smoke [--cluster NAME] [--image TAG] [--no-build] [--keep]

  --cluster NAME  kind cluster to create and delete (default bicraft-smoke)
  --image TAG     image tag (default bicraft:kind-smoke)
  --no-build      use the image TAG already built
  --keep          keep the cluster and the image afterwards
";

const NAMESPACE: &str = "bicraft-smoke";
const RELEASE: &str = "smoke";
const WORKSPACE: &str = "main";
const TOKEN_NAME: &str = "agent";
/// The reader token of the viewer checks.
const READER_NAME: &str = "smoke-reader";

struct Opts {
    cluster: String,
    image: String,
    build: bool,
    keep: bool,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        cluster: "bicraft-smoke".into(),
        image: "bicraft:kind-smoke".into(),
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
                    "deploy kind-smoke: unknown argument `{other}`\n\n{USAGE}"
                ));
            }
        }
    }
    Ok(o)
}

/// The tools of `tools` that are not on PATH.
pub fn missing_tools(tools: &[&str]) -> Vec<String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    tools
        .iter()
        .filter(|t| {
            !dirs
                .iter()
                .any(|d| d.join(t).is_file() || (cfg!(windows) && d.join(format!("{t}.exe")).is_file()))
        })
        .map(|t| (*t).to_owned())
        .collect()
}

pub(super) fn run(cmd: &mut Command, what: &str) -> Result<String, String> {
    eprintln!("$ {what}");
    let o = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{what}: {e}"))?;
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    if !o.status.success() {
        return Err(format!(
            "{what}: {}\n{out}{}",
            o.status,
            String::from_utf8_lossy(&o.stderr)
        ));
    }
    Ok(out)
}

pub(super) fn tool(name: &str, args: &[&str]) -> Result<String, String> {
    run(
        Command::new(name).args(args),
        &format!("{name} {}", args.join(" ")),
    )
}

/// Runs a command with its output streamed to the terminal.
pub(super) fn stream(name: &str, args: &[&str], dir: &Path) -> Result<(), String> {
    let what = format!("{name} {}", args.join(" "));
    eprintln!("$ {what}");
    let st = Command::new(name)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("{what}: {e}"))?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("{what}: {st}"))
    }
}

/// What has to be undone, whatever happens.
pub(super) struct Cleanup {
    pub(super) keep: bool,
    pub(super) cluster: Option<String>,
    pub(super) image: Option<String>,
    pub(super) children: Vec<Child>,
    pub(super) scratch: Option<PathBuf>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(s) = &self.scratch {
            let _ = std::fs::remove_dir_all(s);
        }
        for c in &mut self.children {
            let _ = c.kill();
            let _ = c.wait();
        }
        if self.keep {
            eprintln!(
                "deploy kind-smoke: --keep: cluster {:?} and image {:?} kept",
                self.cluster, self.image
            );
            return;
        }
        if let Some(c) = &self.cluster {
            let _ = tool("kind", &["delete", "cluster", "--name", c]);
        }
        if let Some(i) = &self.image {
            if let Ok(df) = tool("docker", &["system", "df"]) {
                eprintln!(
                    "deploy kind-smoke: docker disk use before cleanup:\n{}",
                    df.trim_end()
                );
            }
            let _ = tool("docker", &["image", "rm", "-f", i]);
            let _ = tool("docker", &["image", "prune", "-f"]);
            let _ = tool("docker", &["builder", "prune", "-af"]);
        }
    }
}

pub(super) fn mem_available_mib() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = text
        .lines()
        .find_map(|l| l.strip_prefix("MemAvailable:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    Some(kb / 1024)
}

pub fn cmd(args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    let missing = missing_tools(&["docker", "kind", "helm", "kubectl"]);
    if !missing.is_empty() {
        return Err(format!(
            "deploy kind-smoke needs docker, kind, helm and kubectl on PATH; missing: {}. \
             Install them (or run it where they are, e.g. inside WSL), or use `cargo xtask deploy smoke`, \
             which needs none of them.",
            missing.join(", ")
        ));
    }
    let root = crate::root();
    let chart = root.join("deploy").join("helm").join("bicraft");
    let chart_s = chart.to_string_lossy().into_owned();
    let t0 = Instant::now();
    let mut cleanup = Cleanup {
        keep: o.keep,
        cluster: None,
        image: None,
        children: Vec::new(),
        scratch: None,
    };

    // 1. The chart, statically.
    for (t, args) in [
        ("docker", &["version", "--format", "{{.Server.Version}}"][..]),
        ("kind", &["version"][..]),
        ("helm", &["version", "--short"][..]),
        ("kubectl", &["version", "--client"][..]),
    ] {
        let v = tool(t, args).unwrap_or_default();
        eprintln!("deploy kind-smoke: {t}: {}", v.lines().next().unwrap_or("?"));
    }
    let scratch = std::env::temp_dir().join(format!("bicraft-kind-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    cleanup.scratch = Some(scratch.clone());
    let write = |name: &str, text: &str| -> Result<String, String> {
        let p = scratch.join(name);
        std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))?;
        Ok(p.to_string_lossy().into_owned())
    };
    let auth = write(
        "auth.yaml",
        "auth:\n  apiTokens:\n    existingSecret: tokens\n  grants: [\"token:agent=main\"]\n",
    )?;
    let every = write(
        "every.yaml",
        "ingress:\n  enabled: true\n  hosts: [bi.example.com]\n\
         httpRoute:\n  enabled: true\n  parentRefs: [{name: gw}]\n  hostnames: [bi.example.com]\n\
         networkPolicy:\n  egressCidrs: [203.0.113.0/24]\n  cilium:\n    enabled: true\n    fqdns: [login.example.com, \"*.cloud.databricks.com\"]\n\
         databricks:\n  enabled: true\n  existingSecret: dbx\n\
         auth:\n  config:\n    json: '{\"acl\": []}'\n  oidc:\n    issuers: [https://login.example.com]\n\
         serviceAccount:\n  create: true\n",
    )?;
    tool("helm", &["lint", chart_s.as_str(), "--strict", "-f", &auth])?;
    if tool("helm", &["template", "r", chart_s.as_str()]).is_ok() {
        return Err("helm template without authentication must fail (bicraft.validate)".into());
    }
    eprintln!("deploy kind-smoke: ok  helm template without authentication is refused");
    let rendered = tool("helm", &["template", "r", chart_s.as_str(), "-f", &auth])?;
    let full = tool(
        "helm",
        &["template", "r", chart_s.as_str(), "-f", &auth, "-f", &every],
    )?;
    tool(
        "helm",
        &["lint", chart_s.as_str(), "--strict", "-f", &auth, "-f", &every],
    )?;
    for (needed, what) in [
        ("memory: \"2048Mi\"", "pod memory limit 1984 + 64 MiB"),
        ("readOnlyRootFilesystem: true", "read-only root"),
        ("type: Recreate", "Recreate"),
    ] {
        if !rendered.contains(needed) {
            return Err(format!("helm template: {what} (`{needed}`) missing"));
        }
    }
    for kind in [
        "kind: Ingress",
        "kind: HTTPRoute",
        "kind: CiliumNetworkPolicy",
        "kind: NetworkPolicy",
        "kind: Secret",
    ] {
        if !full.contains(kind) {
            return Err(format!("helm template with every option: {kind} missing"));
        }
    }
    eprintln!("deploy kind-smoke: ok  helm lint --strict, helm template (defaults and every option)");

    // 2. The image.
    if o.build {
        if let Some(m) = mem_available_mib() {
            eprintln!("deploy kind-smoke: {m} MiB available before the image build");
        }
        stream(
            "docker",
            &["build", "-f", "deploy/docker/Dockerfile", "-t", &o.image, "."],
            &root,
        )?;
    }
    cleanup.image = Some(o.image.clone());
    let size = tool("docker", &["image", "inspect", "-f", "{{.Size}}", &o.image])?;
    let size_mb = size.trim().parse::<f64>().unwrap_or(0.0) / 1e6;
    let history = tool(
        "docker",
        &[
            "image",
            "inspect",
            "-f",
            "{{.Config.User}} {{json .Config.Entrypoint}} {{json .Config.Cmd}}",
            &o.image,
        ],
    )?;
    eprintln!(
        "deploy kind-smoke: ok  image {} is {size_mb:.1} MB, user/entrypoint/cmd: {}",
        o.image,
        history.trim()
    );
    if !history.starts_with("65532:65532") {
        return Err(format!("image user: {history}"));
    }

    // 3. Fixture and token, with the image's own binary.
    let result = flow(&o, &root, &chart_s, &scratch, &mut cleanup);
    if result.is_ok() {
        eprintln!("deploy kind-smoke: PASSED in {:.0} s", t0.elapsed().as_secs_f64());
    }
    result
}

pub(super) fn uid_gid() -> String {
    let id = |flag: &str| {
        tool("id", &[flag])
            .map(|s| s.trim().to_owned())
            .unwrap_or_else(|_| "0".into())
    };
    format!("{}:{}", id("-u"), id("-g"))
}

fn flow(o: &Opts, root: &Path, chart: &str, scratch: &Path, cleanup: &mut Cleanup) -> Result<(), String> {
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
            "/usr/local/bin/bicraft",
            o.image.as_str(),
        ];
        a.extend(args);
        tool("docker", &a)
    };
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
    let tok = bicraft(&[
        "serve", "token", "create", "--name", TOKEN_NAME, "--role", "agent",
    ])?;
    let token = tok
        .lines()
        .find_map(|l| l.strip_prefix("token: "))
        .ok_or("no token")?
        .to_owned();
    let entry = tok
        .lines()
        .find_map(|l| l.strip_prefix("entry: "))
        .ok_or("no entry")?;
    // a reader too, for the viewer checks
    let rtok = bicraft(&["serve", "token", "create", "--name", READER_NAME])?;
    let reader_token = rtok
        .lines()
        .find_map(|l| l.strip_prefix("token: "))
        .ok_or("no reader token")?
        .to_owned();
    let reader_entry = rtok
        .lines()
        .find_map(|l| l.strip_prefix("entry: "))
        .ok_or("no reader entry")?;
    let tokens_file = scratch.join("tokens.json");
    std::fs::write(&tokens_file, format!("{{\"tokens\": [{entry}, {reader_entry}]}}"))
        .map_err(|e| e.to_string())?;
    eprintln!(
        "deploy kind-smoke: ok  fixture ({} bytes zipped) and API token made by the image",
        zip.len()
    );

    // 4. Cluster.
    if let Some(m) = mem_available_mib() {
        eprintln!("deploy kind-smoke: {m} MiB available before the cluster");
        if m < 3000 {
            return Err(format!(
                "only {m} MiB available: a kind cluster needs about 2 GB; free memory first"
            ));
        }
    }
    tool(
        "kind",
        &["create", "cluster", "--name", &o.cluster, "--wait", "180s"],
    )?;
    cleanup.cluster = Some(o.cluster.clone());
    let ctx = format!("kind-{}", o.cluster);
    let k = |args: &[&str]| -> Result<String, String> {
        let mut a = vec!["--context", ctx.as_str()];
        a.extend(args);
        tool("kubectl", &a)
    };
    tool("kind", &["load", "docker-image", &o.image, "--name", &o.cluster])?;
    // The plain manifests, validated by the API server (the namespace must exist for a dry run).
    let manifests = root.join("deploy").join("manifests");
    k(&["create", "namespace", "bicraft"])?;
    let dry = k(&["apply", "--dry-run=server", "-k", &manifests.to_string_lossy()])?;
    eprintln!(
        "deploy kind-smoke: ok  kubectl apply --dry-run=server -k deploy/manifests:\n{}",
        dry.trim_end()
    );
    k(&["delete", "namespace", "bicraft", "--wait=false"])?;

    k(&["create", "namespace", NAMESPACE])?;
    k(&[
        "-n",
        NAMESPACE,
        "label",
        "namespace",
        NAMESPACE,
        "pod-security.kubernetes.io/enforce=restricted",
    ])?;
    k(&[
        "-n",
        NAMESPACE,
        "create",
        "secret",
        "generic",
        "smoke-tokens",
        &format!("--from-file=tokens.json={}", tokens_file.display()),
    ])?;
    let (repo, tag) = o.image.rsplit_once(':').unwrap_or((o.image.as_str(), "latest"));
    let values = scratch.join("smoke-values.yaml");
    std::fs::write(
        &values,
        format!(
            "image:\n  repository: {repo}\n  tag: \"{tag}\"\n  pullPolicy: Never\n\
             workspace:\n  id: {WORKSPACE}\n\
             auth:\n  apiTokens:\n    existingSecret: smoke-tokens\n  grants: [\"token:{TOKEN_NAME}={WORKSPACE}\"]\n  \
             readerGrants: [\"token:{READER_NAME}={WORKSPACE}\"]\n\
             model:\n  memoryLimitMiB: 448\n  cpu: 1\n\
             persistence:\n  size: 1Gi\n  keep: false\n\
             spill:\n  sizeLimit: 1Gi\n\
             server:\n  shutdownGraceSeconds: 10\n"
        ),
    )
    .map_err(|e| format!("{}: {e}", values.display()))?;
    let values_s = values.to_string_lossy().into_owned();
    let install = vec![
        "--kube-context",
        ctx.as_str(),
        "install",
        RELEASE,
        chart,
        "-n",
        NAMESPACE,
        "--wait",
        "--timeout",
        "300s",
        "-f",
        values_s.as_str(),
    ];
    if let Err(e) = tool("helm", &install) {
        let pods = k(&["-n", NAMESPACE, "describe", "pods"]).unwrap_or_default();
        return Err(format!("{e}\n{pods}"));
    }
    let full = format!("{RELEASE}-bicraft");
    let pod_json = k(&[
        "-n",
        NAMESPACE,
        "get",
        "pods",
        "-l",
        "app.kubernetes.io/instance=smoke",
        "-o",
        "jsonpath={.items[0].spec.containers[0].resources.limits.memory} {.items[0].status.phase}",
    ])?;
    eprintln!(
        "deploy kind-smoke: ok  helm install (restricted Pod Security namespace): pod limit and phase {pod_json}"
    );

    // 5. Port-forward and the live checks.
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
            &format!("svc/{full}"),
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
    let ep = Endpoint {
        addr: format!("127.0.0.1:{port}").parse().map_err(|e| format!("{e}"))?,
        host: format!("{full}.{NAMESPACE}.svc:7878"),
        token,
        workspace: WORKSPACE.into(),
    };
    client::checks(&ep, &zip, "Smoke.pbip")?;
    client::viewer_checks(&ep, &reader_token, true)?;
    client::concurrency_checks(&ep)?;

    // 6. Drain: follow the log, scale to zero.
    let mut logs = Command::new("kubectl")
        .args([
            "--context",
            &ctx,
            "-n",
            NAMESPACE,
            "logs",
            "-f",
            &format!("deploy/{full}"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("kubectl logs: {e}"))?;
    let log_out = logs.stdout.take();
    let (ltx, lrx) = mpsc::channel();
    if let Some(out) = log_out {
        std::thread::spawn(move || {
            for l in BufReader::new(out).lines().map_while(Result::ok) {
                eprintln!("  [pod] {l}");
                let _ = ltx.send(l);
            }
        });
    }
    cleanup.children.push(logs);
    std::thread::sleep(Duration::from_secs(1));
    k(&[
        "-n",
        NAMESPACE,
        "scale",
        &format!("deploy/{full}"),
        "--replicas=0",
    ])?;
    let start = Instant::now();
    let mut drained = None;
    while start.elapsed() < Duration::from_secs(90) {
        match lrx.recv_timeout(Duration::from_secs(1)) {
            Ok(l) if l.contains("exit 0") || l.contains("exit 3") => {
                drained = Some(l);
                break;
            }
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    match drained {
        Some(l) if l.contains("exit 0") => {
            eprintln!(
                "deploy kind-smoke: ok  scaled to zero: clean drain in {} ms ({})",
                start.elapsed().as_millis(),
                l.trim()
            );
            Ok(())
        }
        other => Err(format!(
            "no clean drain in the pod's log after scaling to zero: {other:?}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_tools_are_named() {
        let m = missing_tools(&["surely-not-a-tool-7f3a"]);
        assert_eq!(m, ["surely-not-a-tool-7f3a"]);
    }
}
