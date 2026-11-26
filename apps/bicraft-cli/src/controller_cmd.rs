//! `bicraft controller`: the workspace controller of a Kubernetes deployment (crate
//! `bicraft-controller`; docs/kubernetes.md, "The controller"). It reconciles `BicraftWorkspace`
//! resources and serves the gateway: the admin API and every workspace's MCP endpoint and files,
//! with wake on request and idle scale to zero.
//!
//! Authentication is the one of `bicraft serve` (same flags and `BICRAFT_*` variables, docs/auth.md);
//! `--admin SUBJECT` (or `BICRAFT_CONTROLLER_ADMINS`, `;`-separated) names who may create and
//! delete workspaces. The configuration (classes, egress profiles, pod settings) is a JSON file:
//! `--config FILE` or `BICRAFT_CONTROLLER_CONFIG`.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bicraft_auth::acl::Acl;
use bicraft_auth::{Authenticator, Grant};
use bicraft_controller::{Config, Controller, Gateway, GatewayConfig, HttpKube, Kube, Upstream};
use bicraft_mcp::{HostPolicy, ShutdownSignal};

use super::serve_cmd::{AuthFlags, auth_config, auth_flag, env_var, host_policy};
use super::{Failure, OK, USAGE_ERROR, usage, value};

const DEFAULT_LISTEN: &str = "127.0.0.1:7979";
const DEFAULT_INTERVAL_SECS: u64 = 5;
const DEFAULT_GRACE_SECS: u64 = 30;
/// Exit code when connections were still open at the end of the grace period.
const SHUTDOWN_LOST: u8 = 3;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ControllerOptions {
    pub config: Option<PathBuf>,
    pub listen: Option<String>,
    pub kube_api: Option<String>,
    pub namespace: Option<String>,
    pub admins: Vec<String>,
    pub interval_secs: Option<u64>,
    pub grace_secs: Option<u64>,
    pub auth: AuthFlags,
}

pub(crate) fn parse(args: &[String]) -> Result<ControllerOptions, Failure> {
    let mut o = ControllerOptions::default();
    let secs = |flag: &str, v: &str, max: u64| -> Result<u64, Failure> {
        v.parse::<u64>()
            .ok()
            .filter(|n| *n <= max)
            .ok_or_else(|| usage(format!("{flag} needs seconds (0 to {max}), not `{v}`")))
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let flag = a.as_str();
        match flag {
            "--config" => o.config = Some(PathBuf::from(value(&mut it, flag)?)),
            "--listen" => o.listen = Some(value(&mut it, flag)?.to_string()),
            "--kube-api" => o.kube_api = Some(value(&mut it, flag)?.to_string()),
            "--namespace" => o.namespace = Some(value(&mut it, flag)?.to_string()),
            "--admin" => o.admins.push(value(&mut it, flag)?.to_string()),
            "--interval" => {
                let v = secs(flag, value(&mut it, flag)?, 3600)?;
                if v == 0 {
                    return Err(usage("--interval needs at least 1 second"));
                }
                o.interval_secs = Some(v);
            }
            "--shutdown-grace" => o.grace_secs = Some(secs(flag, value(&mut it, flag)?, 86_400)?),
            other => {
                if !auth_flag(other, &mut it, &mut o.auth)? {
                    return Err(usage(format!("controller: unexpected argument `{other}`")));
                }
            }
        }
    }
    Ok(o)
}

/// The subjects of `--admin` and `BICRAFT_CONTROLLER_ADMINS`.
fn admins(o: &ControllerOptions, env: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let mut v: Vec<String> = env("BICRAFT_CONTROLLER_ADMINS")
        .map(|s| {
            s.split([';', '\n', '\r'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    v.extend(o.admins.iter().cloned());
    v
}

/// Where workspace pods accept the gateway's identity headers from: the configured ranges, else
/// this pod's own address (`POD_IP`, from the downward API).
fn trusted_cidrs(cfg: &Config, pod_ip: Option<String>) -> Result<Vec<String>, String> {
    if !cfg.gateway_trusted_cidrs.is_empty() {
        return Ok(cfg.gateway_trusted_cidrs.clone());
    }
    let ip: IpAddr = pod_ip
        .as_deref()
        .and_then(|s| s.trim().parse().ok())
        .ok_or("set POD_IP (the downward API's status.podIP) or gatewayTrustedCidrs in the configuration")?;
    Ok(vec![match ip {
        IpAddr::V4(_) => format!("{ip}/32"),
        IpAddr::V6(_) => format!("{ip}/128"),
    }])
}

pub(crate) fn controller(args: &[String]) -> Result<u8, Failure> {
    let o = parse(args)?;
    let fail = |e: String| (USAGE_ERROR, format!("controller: {e}"));
    let config_path = o
        .config
        .clone()
        .or_else(|| {
            env_var("BICRAFT_CONTROLLER_CONFIG")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
        .ok_or_else(|| usage("controller needs --config FILE (or BICRAFT_CONTROLLER_CONFIG)"))?;
    let mut cfg = Config::from_file(&config_path).map_err(fail)?;
    if let Some(ns) = &o.namespace {
        cfg.namespace.clone_from(ns);
    }
    let kube: HttpKube = match &o.kube_api {
        Some(url) => HttpKube::with_url(url, &cfg.crd_group, &cfg.namespace),
        None => HttpKube::in_cluster(&cfg.crd_group, &cfg.namespace),
    }
    .map_err(fail)?;
    cfg.namespace = kube.namespace().to_string();
    cfg.check().map_err(fail)?;

    let admin_subjects = admins(&o, &env_var);
    let mut admin_grants = Vec::new();
    for s in &admin_subjects {
        admin_grants.push(Grant::parse_text(&format!("{s}=*")).map_err(|e| fail(format!("--admin: {e}")))?);
    }
    let mut auth_cfg = auth_config(&o.auth, &env_var).map_err(fail)?.ok_or_else(|| {
        usage(
            "controller: configure authentication (--api-tokens, --oidc-issuer, --trusted-proxy or \
                 --auth-config; docs/auth.md)",
        )
    })?;
    auth_cfg.acl.grants.extend(admin_grants.iter().cloned());
    let auth = Arc::new(Authenticator::new(auth_cfg).map_err(fail)?);
    let hosts = match host_policy(&o.auth, &env_var, auth.public_url()) {
        HostPolicy::Loopback => Vec::new(),
        HostPolicy::Names(n) => n,
    };
    let listen_text = o.listen.as_deref().unwrap_or(DEFAULT_LISTEN);
    let listen: SocketAddr = listen_text
        .parse()
        .map_err(|_| usage(format!("--listen needs IP:PORT, not `{listen_text}`")))?;
    if !listen.ip().is_loopback() && hosts.is_empty() {
        return Err(usage(
            "controller: a non-loopback --listen needs --public-url or --allow-host (the Host names answered)",
        ));
    }
    let trusted = trusted_cidrs(&cfg, std::env::var("POD_IP").ok()).map_err(fail)?;
    if admin_subjects.is_empty() {
        eprintln!(
            "bicraft controller: warning: no --admin: nobody can create or delete workspaces through the API \
             (BicraftWorkspace resources still can be, with kubectl)"
        );
    }

    let namespace = cfg.namespace.clone();
    let port = cfg.port;
    let allowed_origins = cfg.pod.allowed_origins.clone();
    let kube: Arc<dyn Kube> = Arc::new(kube);
    let ctl = Controller::new(cfg, kube, trusted.clone());
    let upstream: Upstream = Arc::new(move |ws: &str| format!("bws-{ws}.{namespace}.svc:{port}"));
    let mut gw_cfg = GatewayConfig::new(Arc::clone(&auth), Acl { grants: admin_grants }, upstream);
    gw_cfg.hosts = hosts;
    gw_cfg.allowed_origins = allowed_origins;

    let stop = Arc::new(AtomicBool::new(false));
    let interval = Duration::from_secs(o.interval_secs.unwrap_or(DEFAULT_INTERVAL_SECS));
    let loop_ctl = Arc::clone(&ctl);
    let loop_stop = Arc::clone(&stop);
    let reconciler = std::thread::Builder::new()
        .name("bicraft-reconcile".into())
        .spawn(move || loop_ctl.run(interval, &loop_stop))
        .map_err(|e| fail(format!("reconciler thread: {e}")))?;
    let gateway =
        Gateway::start(listen, Arc::clone(&ctl), gw_cfg).map_err(|e| fail(format!("{listen}: {e}")))?;
    eprintln!(
        "bicraft {}: workspace controller for namespace {} (resources {}/{}), gateway on {} ({}; admins: {}; \
         workspace pods trust identity headers from {})",
        env!("CARGO_PKG_VERSION"),
        ctl.cfg.namespace,
        ctl.cfg.crd_group,
        bicraft_controller::config::VERSION,
        gateway.addr(),
        auth.summary(),
        admin_subjects.len(),
        trusted.join(", ")
    );

    let signal = ShutdownSignal::new();
    let handler_signal = Arc::clone(&signal);
    if let Err(e) = ctrlc::set_handler(move || {
        if handler_signal.is_raised() {
            std::process::exit(130);
        }
        handler_signal.raise();
    }) {
        eprintln!("bicraft controller: warning: no signal handler ({e})");
    }
    signal.wait();
    let grace = Duration::from_secs(o.grace_secs.unwrap_or(DEFAULT_GRACE_SECS));
    eprintln!(
        "bicraft controller: shutting down: no new connections, {} s for the open ones",
        grace.as_secs()
    );
    stop.store(true, Ordering::SeqCst);
    ctl.kick();
    let clean = gateway.stop(grace);
    let _ = reconciler.join();
    eprintln!(
        "bicraft controller: stopped ({})",
        if clean {
            "exit 0"
        } else {
            "connections cut: exit 3"
        }
    );
    Ok(if clean { OK } else { SHUTDOWN_LOST })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn flags() {
        let o = parse(&args(&[
            "--config",
            "c.json",
            "--listen",
            "0.0.0.0:7878",
            "--admin",
            "token:admin",
            "--api-tokens",
            "t.json",
            "--interval",
            "2",
        ]))
        .unwrap();
        assert_eq!(o.config, Some(PathBuf::from("c.json")));
        assert_eq!(o.admins, ["token:admin"]);
        assert_eq!(o.auth.api_tokens, Some(PathBuf::from("t.json")));
        assert_eq!(o.interval_secs, Some(2));
        assert!(parse(&args(&["--interval", "0"])).is_err());
        assert!(parse(&args(&["--bogus"])).is_err());
        assert!(parse(&args(&["--admin"])).is_err());
        let env = |k: &str| (k == "BICRAFT_CONTROLLER_ADMINS").then(|| "group:ops; token:ci".to_string());
        assert_eq!(admins(&o, &env), ["group:ops", "token:ci", "token:admin"]);
    }

    #[test]
    fn trusted_addresses() {
        let cfg = Config::from_json(
            r#"{"image": "i", "defaultClass": "s", "classes": {"s": {"memoryLimitMiB": 448, "cpu": 1,
               "spillSize": "1Gi", "volumeSize": "1Gi", "maxVolumeSize": "1Gi"}}}"#,
        )
        .unwrap();
        assert_eq!(
            trusted_cidrs(&cfg, Some("10.244.0.7".into())).unwrap(),
            ["10.244.0.7/32"]
        );
        assert_eq!(
            trusted_cidrs(&cfg, Some("fd00::7".into())).unwrap(),
            ["fd00::7/128"]
        );
        assert!(trusted_cidrs(&cfg, None).is_err());
        let mut c2 = cfg.clone();
        c2.gateway_trusted_cidrs = vec!["10.0.0.0/8".into()];
        assert_eq!(trusted_cidrs(&c2, None).unwrap(), ["10.0.0.0/8"]);
    }
}
