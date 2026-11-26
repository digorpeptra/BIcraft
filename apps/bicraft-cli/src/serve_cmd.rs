//! `bicraft serve`: the MCP Streamable HTTP endpoint (revision 2026-07-28), one workspace per
//! subfolder of `--root`, at `http://ADDR/w/<workspace>/mcp` (docs/web-deployment.md).
//!
//! Each workspace's session runs in server mode rooted at its folder (`docs/server-mode.md`):
//! tool paths are relative to the workspace and cannot leave it. Without authentication the
//! server listens on loopback only (127.0.0.1:7878 by default; any other address is refused).
//! With at least one authentication mode (OIDC bearer JWTs, a trusted header behind an auth
//! proxy, API tokens: docs/auth.md) and an ACL, every request is authenticated and any address
//! may be used. Network access (the Databricks connector of `model.refresh`) is off unless
//! `--allow-network` or `BICRAFT_SERVER_NETWORK=1` turns it on.
//!
//! Each workspace also has its files at `/w/<workspace>/files/` (upload, download, listing, a zipped
//! project's extraction, deletion; same authentication and ACL), the probes `/healthz` and `/readyz`
//! answer without authentication, and SIGTERM / SIGINT (Ctrl+C) drain the server gracefully:
//! new requests get 503, running ones get `--shutdown-grace` seconds (default 30), nothing is saved
//! implicitly. Exit code 0 when nothing was lost, 3 when requests were cancelled or sessions held
//! unsaved changes (docs/web-deployment.md, "Operating `bicraft serve`").
//!
//! `bicraft serve token create|hash` prints API tokens and their tokens-file entries.

use std::io::{Read as _, Write as _};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bicraft_auth::{Access, AuthConfig, Authenticator, Cidr, Grant, IssuerConfig, ProxyConfig, tokens};
use bicraft_engine::ServerConfig;
use bicraft_mcp::{
    ControlToken, DrainReport, HostPolicy, HttpConfig, HttpServer, LogFormat, ShutdownSignal, log_event,
};

use super::{Failure, OK, USAGE_ERROR, usage, value};

/// Where `serve` listens unless `--listen` says otherwise.
const DEFAULT_LISTEN: &str = "127.0.0.1:7878";
/// How long running requests get to finish after SIGTERM unless `--shutdown-grace` says otherwise.
const DEFAULT_GRACE_SECS: u64 = 30;
/// Exit code of a shutdown that cancelled requests or discarded unsaved changes.
const SHUTDOWN_LOST: u8 = 3;

/// Authentication settings from flags (the environment and the configuration file are merged in
/// [`auth_config`]).
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct AuthFlags {
    pub config: Option<PathBuf>,
    pub public_url: Option<String>,
    pub allow_hosts: Vec<String>,
    pub oidc_issuers: Vec<String>,
    pub oidc_audiences: Vec<String>,
    pub oidc_scopes: Vec<String>,
    pub api_tokens: Option<PathBuf>,
    pub trusted_proxies: Vec<String>,
    pub proxy_user_header: Option<String>,
    pub proxy_email_header: Option<String>,
    pub proxy_groups_header: Option<String>,
    pub grants: Vec<String>,
    /// `--reader-grant`: read-only grants (docs/auth.md "Roles").
    pub reader_grants: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ServeOptions {
    pub listen: SocketAddr,
    pub root: PathBuf,
    pub allowed_origins: Vec<String>,
    pub max_body_bytes: Option<usize>,
    pub queue: Option<usize>,
    /// `--allow-network`; `false` leaves the decision to `BICRAFT_SERVER_NETWORK`.
    pub allow_network: bool,
    pub max_upload_bytes: Option<u64>,
    /// `--shutdown-grace` in seconds (`None`: [`DEFAULT_GRACE_SECS`]).
    pub grace_secs: Option<u64>,
    pub idle_timeout_secs: Option<u64>,
    pub ready_memory_percent: Option<u8>,
    /// `--drain-on-stdin-close`: closing standard input starts the drain, like SIGTERM.
    pub drain_on_stdin_close: bool,
    /// `--metrics-listen`: the Prometheus listener (`/metrics` only, no authentication).
    pub metrics_listen: Option<SocketAddr>,
    /// `--metrics-workspace-label`: per-workspace series.
    pub metrics_workspace_label: bool,
    /// `--log-format` (`None`: `BICRAFT_LOG_FORMAT`, else text).
    pub log_format: Option<LogFormat>,
    /// `--no-viewer`: no live viewer at `/w/<id>/view/`.
    pub no_viewer: bool,
    /// `--viewer-copies N`: read-only copies of saved files kept for readers (default 1).
    pub viewer_copies: Option<usize>,
    /// `--max-viewers N`: browser views kept per workspace (default 256).
    pub max_viewers: Option<usize>,
    /// `--max-concurrent-reads N`: read-only calls running at once on one workspace (default 4).
    pub max_concurrent_reads: Option<usize>,
    /// `--max-views-per-user N`: document views (browser tabs on their own file) one identity
    /// holds at once (default 8).
    pub max_views_per_user: Option<usize>,
    /// `--max-views N`: document views open on one workspace (default 32).
    pub max_views: Option<usize>,
    /// `--view-idle SECS`: an unused document view closes after this long (default 1800).
    pub view_idle_secs: Option<u64>,
    /// `--recovery-dir DIR`: editor sessions journal their unsaved edits there (`None`:
    /// `BICRAFT_RECOVERY_DIR`, else no journal; docs/recovery.md).
    pub recovery_dir: Option<PathBuf>,
    /// `--allow-autosave`: the UI may save PBIP projects automatically.
    pub allow_autosave: bool,
    /// `--viewer-control`: the UI control channel relayed to the app pages.
    pub viewer_control: bool,
    /// `--viewer-control-file F`: where its token file goes (default `control-serve.json` next to
    /// the desktop's settings).
    pub viewer_control_file: Option<PathBuf>,
    /// `--local-offset OFFSET`: M's local clock (`DateTime.LocalNow()`): `±HH:MM`, minutes, `UTC`
    /// or `machine` (`None`: UTC, or the machine's zone when `TZ` is set; docs/formats/data-sources.md).
    pub local_offset: Option<String>,
    pub auth: AuthFlags,
}

pub(crate) fn parse(args: &[String]) -> Result<ServeOptions, Failure> {
    let mut listen: Option<String> = None;
    let mut root = None;
    let mut allow_network = false;
    let mut drain_on_stdin_close = false;
    let (mut metrics_listen, mut metrics_workspace_label, mut log_format) = (None, false, None);
    let mut allowed_origins = Vec::new();
    let (mut max_body_bytes, mut queue) = (None, None);
    let (mut no_viewer, mut viewer_copies, mut max_viewers) = (false, None, None);
    let mut max_concurrent_reads = None;
    let (mut max_views_per_user, mut max_views, mut view_idle_secs) = (None, None, None);
    let (mut recovery_dir, mut allow_autosave) = (None, false);
    let (mut viewer_control, mut viewer_control_file) = (false, None);
    let mut local_offset = None;
    let (mut max_upload_bytes, mut grace_secs, mut idle_timeout_secs, mut ready_memory_percent) =
        (None, None, None, None);
    let mut auth = AuthFlags::default();
    let number = |flag: &str, v: &str| -> Result<usize, Failure> {
        v.parse::<usize>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| usage(format!("{flag} needs a positive integer, not `{v}`")))
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let flag = a.as_str();
        match flag {
            "--listen" => listen = Some(value(&mut it, flag)?.to_string()),
            "--root" => root = Some(PathBuf::from(value(&mut it, flag)?)),
            "--allow-network" => allow_network = true,
            "--drain-on-stdin-close" => drain_on_stdin_close = true,
            "--metrics-listen" => {
                let v = value(&mut it, flag)?;
                metrics_listen = Some(v.parse::<SocketAddr>().map_err(|_| {
                    usage(format!(
                        "--metrics-listen needs IP:PORT (loopback or the pod address), not `{v}`"
                    ))
                })?);
            }
            "--metrics-workspace-label" => metrics_workspace_label = true,
            "--log-format" => {
                let v = value(&mut it, flag)?;
                log_format = Some(LogFormat::parse(v).map_err(|e| usage(format!("--log-format: {e}")))?);
            }
            "--allow-origin" => allowed_origins.push(value(&mut it, flag)?.to_string()),
            "--max-body-bytes" => max_body_bytes = Some(number(a, value(&mut it, a)?)?),
            "--queue" => queue = Some(number(a, value(&mut it, a)?)?),
            "--no-viewer" => no_viewer = true,
            "--viewer-control" => viewer_control = true,
            "--viewer-control-file" => viewer_control_file = Some(PathBuf::from(value(&mut it, a)?)),
            "--viewer-copies" => viewer_copies = Some(number(a, value(&mut it, a)?)?),
            "--max-viewers" => max_viewers = Some(number(a, value(&mut it, a)?)?),
            "--max-views-per-user" => max_views_per_user = Some(number(a, value(&mut it, a)?)?),
            "--max-views" => max_views = Some(number(a, value(&mut it, a)?)?),
            "--recovery-dir" => recovery_dir = Some(PathBuf::from(value(&mut it, a)?)),
            "--allow-autosave" => allow_autosave = true,
            "--local-offset" => {
                let v = value(&mut it, a)?;
                bicraft_engine::LocalClock::parse(v, Some(machine_offset()))
                    .map_err(|e| usage(format!("--local-offset {e}")))?;
                local_offset = Some(v.to_string());
            }
            "--view-idle" => {
                let v = value(&mut it, a)?;
                view_idle_secs = Some(u64::try_from(number(a, v)?).unwrap_or(u64::MAX));
            }
            "--max-concurrent-reads" => {
                let n: usize = number(a, value(&mut it, a)?)?;
                if n == 0 {
                    return Err(usage("--max-concurrent-reads must be at least 1".to_string()));
                }
                max_concurrent_reads = Some(n);
            }
            "--max-upload-bytes" => {
                let v = value(&mut it, a)?;
                max_upload_bytes = Some(bicraft_mem::parse_bytes(v).ok().flatten().ok_or_else(|| {
                    usage(format!("--max-upload-bytes needs a size such as 4G, not `{v}`"))
                })?);
            }
            "--shutdown-grace" => {
                let v = value(&mut it, a)?;
                grace_secs = Some(v.parse::<u64>().ok().filter(|n| *n <= 86_400).ok_or_else(|| {
                    usage(format!("--shutdown-grace needs seconds (0 to 86400), not `{v}`"))
                })?);
            }
            "--idle-timeout" => {
                let v = value(&mut it, a)?;
                idle_timeout_secs = Some(u64::try_from(number(a, v)?).unwrap_or(u64::MAX));
            }
            "--ready-memory-percent" => {
                let v = value(&mut it, a)?;
                ready_memory_percent = Some(
                    v.parse::<u8>()
                        .ok()
                        .filter(|p| (1..=100).contains(p))
                        .ok_or_else(|| usage(format!("--ready-memory-percent needs 1 to 100, not `{v}`")))?,
                );
            }
            other => {
                if !auth_flag(other, &mut it, &mut auth)? {
                    return Err(usage(format!("serve: unexpected argument `{other}`")));
                }
            }
        }
    }
    let listen_text = listen.as_deref().unwrap_or(DEFAULT_LISTEN);
    let listen: SocketAddr = listen_text
        .parse()
        .map_err(|_| usage(format!("--listen needs IP:PORT, not `{listen_text}`")))?;
    let root = root.ok_or_else(|| usage("serve needs --root DIR (one workspace per subfolder)"))?;
    if viewer_control_file.is_some() && !viewer_control {
        return Err(usage("--viewer-control-file goes with --viewer-control"));
    }
    if viewer_control && no_viewer {
        return Err(usage("--viewer-control needs the viewer (drop --no-viewer)"));
    }
    Ok(ServeOptions {
        listen,
        root,
        allowed_origins,
        max_body_bytes,
        queue,
        allow_network,
        max_upload_bytes,
        grace_secs,
        idle_timeout_secs,
        ready_memory_percent,
        drain_on_stdin_close,
        metrics_listen,
        metrics_workspace_label,
        log_format,
        no_viewer,
        viewer_copies,
        max_viewers,
        max_concurrent_reads,
        max_views_per_user,
        max_views,
        view_idle_secs,
        recovery_dir,
        allow_autosave,
        viewer_control,
        viewer_control_file,
        local_offset,
        auth,
    })
}

/// M's local clock on a server: `--local-offset`, else the machine's zone when `TZ` is set (a
/// container's way to name its zone), else UTC, as in the Power BI service.
pub(crate) fn server_clock(
    flag: Option<&str>,
    env_var: &dyn Fn(&str) -> Option<String>,
) -> Result<bicraft_engine::LocalClock, String> {
    match flag {
        Some(v) => bicraft_engine::LocalClock::parse(v, Some(machine_offset())),
        None if env_var("TZ").is_some_and(|t| !t.trim().is_empty()) => {
            Ok(bicraft_engine::LocalClock::Zone(machine_offset()))
        }
        None => Ok(bicraft_engine::LocalClock::Utc),
    }
}

fn machine_offset() -> bicraft_engine::OffsetFn {
    Arc::new(bicraft_locale::calendar::machine_offset_minutes)
}

/// The authentication flags shared by `serve` and `controller`: `true` when `flag` was one of them
/// (its value is consumed from `it`).
pub(crate) fn auth_flag<'a>(
    flag: &str,
    it: &mut impl Iterator<Item = &'a String>,
    auth: &mut AuthFlags,
) -> Result<bool, Failure> {
    match flag {
        "--auth-config" => auth.config = Some(PathBuf::from(value(it, flag)?)),
        "--public-url" => auth.public_url = Some(value(it, flag)?.to_string()),
        "--allow-host" => auth.allow_hosts.push(value(it, flag)?.to_string()),
        "--oidc-issuer" => auth.oidc_issuers.push(value(it, flag)?.to_string()),
        "--oidc-audience" => auth.oidc_audiences.push(value(it, flag)?.to_string()),
        "--oidc-scope" => auth.oidc_scopes.push(value(it, flag)?.to_string()),
        "--api-tokens" => auth.api_tokens = Some(PathBuf::from(value(it, flag)?)),
        "--trusted-proxy" => auth.trusted_proxies.push(value(it, flag)?.to_string()),
        "--proxy-user-header" => auth.proxy_user_header = Some(value(it, flag)?.to_string()),
        "--proxy-email-header" => auth.proxy_email_header = Some(value(it, flag)?.to_string()),
        "--proxy-groups-header" => auth.proxy_groups_header = Some(value(it, flag)?.to_string()),
        "--grant" => auth.grants.push(value(it, flag)?.to_string()),
        "--reader-grant" => auth.reader_grants.push(value(it, flag)?.to_string()),
        _ => return Ok(false),
    }
    Ok(true)
}

/// A list from an environment variable: items separated by commas or white space.
fn env_list(env: &dyn Fn(&str) -> Option<String>, name: &str) -> Vec<String> {
    env(name)
        .map(|v| {
            v.split([',', ' ', '\t', '\n', '\r'])
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The authentication configuration: the file (`--auth-config` or `BICRAFT_AUTH_CONFIG`), then
/// the `BICRAFT_*` variables, then the flags (lists add up; the public URL of a flag wins over
/// the variable, which wins over the file). `None` when nothing is configured.
pub(crate) fn auth_config(
    flags: &AuthFlags,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<AuthConfig>, String> {
    let mut c = AuthConfig::default();
    let file = flags.config.clone().or_else(|| {
        env("BICRAFT_AUTH_CONFIG")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    });
    if let Some(f) = &file {
        c = AuthConfig::from_file(f)?;
    }
    if let Some(u) = flags
        .public_url
        .clone()
        .or_else(|| env("BICRAFT_PUBLIC_URL").filter(|v| !v.is_empty()))
    {
        c.public_url = Some(u);
    }
    let mut issuers = env_list(env, "BICRAFT_OIDC_ISSUERS");
    issuers.extend(flags.oidc_issuers.iter().cloned());
    let mut audiences = env_list(env, "BICRAFT_OIDC_AUDIENCES");
    audiences.extend(flags.oidc_audiences.iter().cloned());
    let mut scopes = env_list(env, "BICRAFT_OIDC_SCOPES");
    scopes.extend(flags.oidc_scopes.iter().cloned());
    if issuers.is_empty() && (!audiences.is_empty() || !scopes.is_empty()) {
        return Err("--oidc-audience and --oidc-scope apply to --oidc-issuer issuers: none given".into());
    }
    for i in issuers {
        let mut ic = IssuerConfig::new(i);
        ic.audiences.clone_from(&audiences);
        ic.required_scopes.clone_from(&scopes);
        c.issuers.push(ic);
    }
    let tokens_file = flags.api_tokens.clone().or_else(|| {
        env("BICRAFT_API_TOKENS_FILE")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    });
    if let Some(f) = tokens_file {
        c.add_tokens(bicraft_auth::load_tokens_file(&f)?)?;
    }
    let mut proxies = env_list(env, "BICRAFT_TRUSTED_PROXIES");
    proxies.extend(flags.trusted_proxies.iter().cloned());
    if !proxies.is_empty() {
        let trusted = proxies
            .iter()
            .map(|p| Cidr::parse(p))
            .collect::<Result<Vec<_>, _>>()?;
        let p = c.proxy.get_or_insert_with(|| ProxyConfig::new(Vec::new()));
        p.trusted.extend(trusted);
    }
    let header_flags = [
        &flags.proxy_user_header,
        &flags.proxy_email_header,
        &flags.proxy_groups_header,
    ];
    if header_flags.iter().any(|h| h.is_some()) {
        let p = c
            .proxy
            .as_mut()
            .ok_or("--proxy-*-header needs --trusted-proxy (or a proxy section in the config file)")?;
        if let Some(h) = &flags.proxy_user_header {
            p.user_header.clone_from(h);
        }
        if let Some(h) = &flags.proxy_email_header {
            p.email_header = Some(h.clone()).filter(|h| !h.is_empty());
        }
        if let Some(h) = &flags.proxy_groups_header {
            p.groups_header = Some(h.clone()).filter(|h| !h.is_empty());
        }
    }
    // editor grants, then reader grants (docs/auth.md "Roles": the highest matching access wins,
    // so the order does not matter)
    for (var, from_flags, access) in [
        ("BICRAFT_AUTH_GRANTS", &flags.grants, Access::Editor),
        ("BICRAFT_AUTH_READER_GRANTS", &flags.reader_grants, Access::Reader),
    ] {
        let mut grants: Vec<String> = env(var)
            .map(|v| {
                v.split([';', ' ', '\t', '\n', '\r'])
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        grants.extend(from_flags.iter().cloned());
        for g in grants {
            c.acl.grants.push(Grant::parse_text_as(&g, access)?);
        }
    }
    if !c.any_mode() && c.acl.grants.is_empty() && file.is_none() {
        if c.public_url.is_some() {
            return Err("--public-url / BICRAFT_PUBLIC_URL needs an authentication mode".into());
        }
        return Ok(None);
    }
    Ok(Some(c))
}

/// The `Host` values the server answers: `--allow-host` / `BICRAFT_ALLOWED_HOSTS`, and the
/// public URL's host with and without its default port. Loopback names when none is given.
pub(crate) fn host_policy(
    flags: &AuthFlags,
    env: &dyn Fn(&str) -> Option<String>,
    public: Option<&str>,
) -> HostPolicy {
    let mut names = env_list(env, "BICRAFT_ALLOWED_HOSTS");
    names.extend(flags.allow_hosts.iter().cloned());
    if let Some(u) = public
        && let Some((scheme, authority)) = u.split_once("://")
    {
        let default_port = if scheme.eq_ignore_ascii_case("https") {
            "443"
        } else {
            "80"
        };
        names.push(authority.to_string());
        match authority.rsplit_once(':') {
            Some((host, port)) if !authority.ends_with(']') => {
                if port == default_port {
                    names.push(host.to_string());
                }
            }
            _ => names.push(format!("{authority}:{default_port}")),
        }
    }
    if names.is_empty() {
        HostPolicy::Loopback
    } else {
        HostPolicy::Names(names)
    }
}

pub(crate) fn env_var(name: &str) -> Option<String> {
    bicraft_env::var(name).ok()
}

pub(crate) fn serve(args: &[String]) -> Result<u8, Failure> {
    if args.first().map(String::as_str) == Some("token") {
        return token(args.get(1..).unwrap_or_default());
    }
    let o = parse(args)?;
    let from_env = ServerConfig::network_from_env().map_err(|e| (USAGE_ERROR, format!("serve: {e}")))?;
    let allow_network = o.allow_network || from_env;
    let auth_cfg = auth_config(&o.auth, &env_var).map_err(|e| (USAGE_ERROR, format!("serve: {e}")))?;
    let auth = match auth_cfg {
        Some(c) => Some(Arc::new(
            Authenticator::new(c).map_err(|e| (USAGE_ERROR, format!("serve: {e}")))?,
        )),
        None => None,
    };
    let hosts = host_policy(&o.auth, &env_var, auth.as_ref().and_then(|a| a.public_url()));
    if !o.listen.ip().is_loopback() {
        if auth.is_none() {
            return Err(usage(format!(
                "serve: --listen {} is not a loopback address: configure authentication first \
                 (--oidc-issuer, --api-tokens, --trusted-proxy or --auth-config, plus --grant or --reader-grant; \
                 docs/auth.md)",
                o.listen.ip()
            )));
        }
        if hosts == HostPolicy::Loopback {
            return Err(usage(
                "serve: a non-loopback --listen needs --public-url or --allow-host (the Host names \
                 the server answers)",
            ));
        }
    }
    let mut config = HttpConfig {
        allowed_origins: o.allowed_origins,
        allow_network,
        hosts,
        auth: auth.clone(),
        ..HttpConfig::default()
    };
    if let Some(n) = o.max_body_bytes {
        config.max_body_bytes = n;
    }
    if let Some(n) = o.queue {
        config.queue_per_workspace = n;
    }
    if let Some(n) = o.max_upload_bytes {
        config.max_upload_bytes = n;
    }
    config.idle_timeout = o.idle_timeout_secs.map(Duration::from_secs);
    config.ready_memory_percent = o.ready_memory_percent;
    config.metrics_listen = o.metrics_listen;
    config.metrics_workspace_label = o.metrics_workspace_label;
    config.viewer = !o.no_viewer;
    if let Some(n) = o.viewer_copies {
        config.viewer_copies = n;
    }
    if let Some(n) = o.max_viewers {
        config.max_viewers = n;
    }
    if let Some(n) = o.max_concurrent_reads {
        config.max_concurrent_reads = n;
    }
    if let Some(n) = o.max_views_per_user {
        config.max_views_per_user = n;
    }
    if let Some(n) = o.max_views {
        config.max_views_per_workspace = n;
    }
    if let Some(s) = o.view_idle_secs {
        config.view_idle = Duration::from_secs(s);
    }
    config.recovery_dir = o.recovery_dir.or_else(|| {
        env_var(bicraft_engine::RECOVERY_DIR_ENV)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    });
    config.allow_autosave = o.allow_autosave;
    let clock = server_clock(o.local_offset.as_deref(), &env_var)
        .map_err(|e| usage(format!("serve: --local-offset {e}")))?;
    let clock_line = clock.describe();
    bicraft_engine::set_local_clock(clock);
    // the UI control channel (docs/web-deployment.md): the operator's token
    // (BICRAFT_VIEWER_CONTROL_TOKEN), else a new one written, never printed, to an owner-only file
    let mut control_file = None;
    if o.viewer_control {
        if !bicraft_view::webui::available() {
            return Err(usage(
                "--viewer-control: this bicraft was built without the browser app that takes the requests (docs/webui.md)",
            ));
        }
        let given = env_var("BICRAFT_VIEWER_CONTROL_TOKEN").filter(|t| !t.trim().is_empty());
        let token = match &given {
            Some(t) => ControlToken::new(t)
                .map_err(|e| usage(format!("serve: BICRAFT_VIEWER_CONTROL_TOKEN: {e}")))?,
            None => ControlToken::random().map_err(|e| (USAGE_ERROR, format!("serve: {e}")))?,
        };
        let file = match (&o.viewer_control_file, given.is_some()) {
            (Some(f), _) => Some(f.clone()),
            (None, false) => Some(
                crate::ui_cmd::default_file(crate::ui_cmd::Kind::Serve).ok_or_else(|| {
                    usage("--viewer-control: no place for the token file: give --viewer-control-file")
                })?,
            ),
            (None, true) => None,
        };
        control_file = file.map(|f| (f, token.clone()));
        config.viewer_control = Some(token);
    }
    let format = match o.log_format {
        Some(f) => f,
        None => match env_var("BICRAFT_LOG_FORMAT").filter(|v| !v.is_empty()) {
            Some(v) => LogFormat::parse(&v).map_err(|e| usage(format!("serve: BICRAFT_LOG_FORMAT: {e}")))?,
            None => LogFormat::Text,
        },
    };
    config.log_format = format;
    let grace = Duration::from_secs(o.grace_secs.unwrap_or(DEFAULT_GRACE_SECS));
    if let Some(a) = &auth {
        for issuer in a.prefetch() {
            say(
                format,
                "warn",
                "auth",
                &format!(
                    "bicraft serve: warning: the keys of OIDC issuer {issuer} cannot be fetched now (retried on use)"
                ),
            );
        }
    }
    // SIGTERM / SIGINT / SIGHUP and Ctrl+C / Ctrl+Break start the drain; a second one ends the
    // process at once (a person at a terminal who does not want to wait)
    let signal = ShutdownSignal::new();
    let handler_signal = Arc::clone(&signal);
    if let Err(e) = ctrlc::set_handler(move || {
        if handler_signal.is_raised() {
            stderr_line("bicraft serve: second signal: exiting without waiting");
            std::process::exit(130);
        }
        handler_signal.raise();
    }) {
        say(
            format,
            "warn",
            "start",
            &format!("bicraft serve: warning: no signal handler ({e}): the server cannot drain gracefully"),
        );
    }
    let server =
        HttpServer::start(o.listen, &o.root, config).map_err(|e| (USAGE_ERROR, format!("serve: {e}")))?;
    let posture = match &auth {
        Some(a) => format!("authentication: {}", a.summary()),
        None => "loopback only, no authentication".into(),
    };
    let line = format!(
        "bicraft {}: MCP Streamable HTTP (2026-07-28) on {} for the workspaces in {} ({posture}; \
         tool paths confined to each workspace; network access {})",
        env!("CARGO_PKG_VERSION"),
        server.url("<workspace>"),
        o.root.display(),
        if allow_network { "on" } else { "off" }
    );
    say(format, "info", "start", &line);
    let line = format!(
        "bicraft serve: files at {}, probes /healthz and /readyz; SIGTERM or Ctrl+C{} drains (grace {} s)",
        server.files_url("<workspace>"),
        if o.drain_on_stdin_close {
            " or the end of standard input"
        } else {
            ""
        },
        grace.as_secs()
    );
    say(format, "info", "start", &line);
    if clock_line != "UTC" {
        say(
            format,
            "info",
            "start",
            &format!("bicraft serve: Power Query's local clock (DateTime.LocalNow) is {clock_line}"),
        );
    }
    if !o.no_viewer {
        say(
            format,
            "info",
            "start",
            &format!(
                "bicraft serve: live viewer at {} (private views; readers see saved files; never writes the project)",
                server.view_url("<workspace>")
            ),
        );
    }
    if let Some((file, token)) = &control_file {
        let base = o
            .auth
            .public_url
            .clone()
            .map(|u| u.trim_end_matches('/').to_string())
            .unwrap_or_else(|| format!("http://{}", server.addr()));
        let body = serde_json::json!({"kind": "serve", "url": base, "token": token.reveal(),
                                      "pid": std::process::id()});
        let text = serde_json::to_string_pretty(&body).unwrap_or_default();
        bicraft_http::write_owner_only(file, text.as_bytes()).map_err(|e| {
            (
                USAGE_ERROR,
                format!("--viewer-control-file {}: {e}", file.display()),
            )
        })?;
    }
    if o.viewer_control {
        let place = match &control_file {
            Some((f, _)) => format!("token in {}", f.display()),
            None => "token from BICRAFT_VIEWER_CONTROL_TOKEN".to_string(),
        };
        say(
            format,
            "info",
            "start",
            &format!(
                "bicraft serve: UI control channel relayed to the app pages at {}api/control (editors and the control token; {place})",
                server.view_url("<workspace>")
            ),
        );
    }
    if let Some(m) = server.metrics_addr() {
        say(
            format,
            "info",
            "start",
            &format!(
                "bicraft serve: metrics at http://{m}/metrics (Prometheus text format; no authentication on that port)"
            ),
        );
    }
    // For supervisors that cannot send a signal (Windows has no SIGTERM: services, test
    // harnesses): with `--drain-on-stdin-close`, the end of standard input starts the same drain.
    if o.drain_on_stdin_close {
        let stdin_signal = Arc::clone(&signal);
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut stdin = std::io::stdin().lock();
            while matches!(stdin.read(&mut buf), Ok(n) if n > 0) {}
            stdin_signal.raise();
        });
    }
    signal.wait();
    say(
        format,
        "info",
        "drain",
        &format!(
            "bicraft serve: shutting down: new requests get 503, running ones have {} s to finish; \
             nothing is saved",
            grace.as_secs()
        ),
    );
    let report = server.drain(grace);
    if let Some((file, _)) = &control_file {
        let _ = std::fs::remove_file(file);
    }
    say(
        format,
        if report.clean() { "info" } else { "warn" },
        "drain",
        &format!("bicraft serve: {}", drain_summary(&report)),
    );
    Ok(if report.clean() { OK } else { SHUTDOWN_LOST })
}

/// One line on standard error, ignoring a write error: a server whose log pipe was closed (a
/// supervisor gone, a test that stopped reading) keeps serving instead of panicking in `eprintln!`.
fn stderr_line(line: &str) {
    let _ = writeln!(std::io::stderr().lock(), "{line}");
}

/// A server event: the human line as it is in the text format, a JSON line (without the
/// `bicraft serve: ` prefix) in the JSON format.
fn say(format: LogFormat, level: &str, event: &str, line: &str) {
    match format {
        LogFormat::Text => stderr_line(line),
        LogFormat::Json => {
            let message = line.strip_prefix("bicraft serve: ").unwrap_or(line);
            log_event(LogFormat::Json, None, level, event, message);
        }
    }
}

/// One line about a drain (workspace ids are the operator's folder names, never file names).
fn drain_summary(r: &DrainReport) -> String {
    let mut s = if r.finished_in_grace {
        "every request finished".to_string()
    } else {
        format!(
            "grace period over: {} tool call(s) cancelled, {} request(s) abandoned",
            r.cancelled, r.abandoned
        )
    };
    s.push_str(&format!("; {} workspace session(s) closed", r.closed));
    if !r.unsaved.is_empty() {
        s.push_str(&format!(
            "; unsaved changes discarded (not saved) in: {}",
            r.unsaved.join(", ")
        ));
    }
    if r.temp_removed > 0 {
        s.push_str(&format!("; {} upload temp folder(s) removed", r.temp_removed));
    }
    s.push_str(if r.clean() { "; exit 0" } else { "; exit 3" });
    s
}

/// `bicraft serve token create --name N [--role R]... [--group G]...` and
/// `bicraft serve token hash --name N` (the token on stdin).
fn token(args: &[String]) -> Result<u8, Failure> {
    let (sub, rest) = match args.split_first() {
        Some((s, r)) => (s.as_str(), r),
        None => return Err(usage("serve token needs `create` or `hash`")),
    };
    let mut name = None;
    let (mut roles, mut groups) = (Vec::new(), Vec::new());
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--name" => name = Some(value(&mut it, "--name")?.to_string()),
            "--role" => roles.push(value(&mut it, "--role")?.to_string()),
            "--group" => groups.push(value(&mut it, "--group")?.to_string()),
            other => return Err(usage(format!("serve token: unexpected argument `{other}`"))),
        }
    }
    let name =
        name.ok_or_else(|| usage("serve token needs --name NAME (the identity in the ACL: token:NAME)"))?;
    let (secret, hash) = match sub {
        "create" => {
            let t = tokens::generate().map_err(|e| (1, format!("serve token: {e}")))?;
            let h = tokens::hash_token(&t).map_err(|e| (1, format!("serve token: {e}")))?;
            (Some(t), h)
        }
        "hash" => {
            let mut input = String::new();
            std::io::stdin()
                .take(4096)
                .read_to_string(&mut input)
                .map_err(|e| (1, format!("serve token hash: {e}")))?;
            let h = tokens::hash_token(input.trim())
                .map_err(|e| (USAGE_ERROR, format!("serve token hash: {e}")))?;
            (None, h)
        }
        other => {
            return Err(usage(format!(
                "serve token: unknown action `{other}` (create, hash)"
            )));
        }
    };
    let entry = tokens::entry_json(&name, &hash, &roles, &groups);
    if let Some(t) = secret {
        // printed once on stdout, stored nowhere
        println!("token: {t}");
        eprintln!("Give the token to the agent (Authorization: Bearer <token>); it is shown only once.");
    }
    println!("entry: {entry}");
    eprintln!(
        "Add the entry to the \"tokens\" list of the tokens file (--api-tokens FILE or BICRAFT_API_TOKENS_FILE)."
    );
    Ok(OK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_string()).collect()
    }

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn operations_flags() {
        let o = parse(&args(&[
            "--root",
            "r",
            "--max-upload-bytes",
            "2G",
            "--shutdown-grace",
            "0",
            "--idle-timeout",
            "600",
            "--ready-memory-percent",
            "90",
            "--drain-on-stdin-close",
        ]))
        .unwrap();
        assert_eq!(o.max_upload_bytes, Some(2 << 30));
        assert_eq!(o.grace_secs, Some(0));
        assert_eq!(o.idle_timeout_secs, Some(600));
        assert_eq!(o.ready_memory_percent, Some(90));
        assert!(o.drain_on_stdin_close);
        let o = parse(&args(&[
            "--root",
            "r",
            "--max-views-per-user",
            "3",
            "--max-views",
            "12",
            "--view-idle",
            "60",
        ]))
        .unwrap();
        assert_eq!(
            (o.max_views_per_user, o.max_views, o.view_idle_secs),
            (Some(3), Some(12), Some(60))
        );
        for flag in ["--max-views-per-user", "--max-views", "--view-idle"] {
            assert!(parse(&args(&["--root", "r", flag, "0"])).is_err(), "{flag}");
        }
        let o = parse(&args(&[
            "--root",
            "r",
            "--recovery-dir",
            "rec",
            "--allow-autosave",
        ]))
        .unwrap();
        assert_eq!(o.recovery_dir, Some(PathBuf::from("rec")));
        assert!(o.allow_autosave);
        let o = parse(&args(&["--root", "r"])).unwrap();
        assert_eq!((o.recovery_dir, o.allow_autosave), (None, false));
        assert!(parse(&args(&["--root", "r", "--recovery-dir"])).is_err());
        // M's local clock: UTC unless --local-offset, or TZ, says otherwise
        assert_eq!(parse(&args(&["--root", "r"])).unwrap().local_offset, None);
        let o = parse(&args(&["--root", "r", "--local-offset", "+02:00"])).unwrap();
        assert_eq!(o.local_offset.as_deref(), Some("+02:00"));
        assert!(parse(&args(&["--root", "r", "--local-offset", "machine"])).is_ok());
        for bad in ["Europe/Paris", "+15:00", ""] {
            assert!(
                parse(&args(&["--root", "r", "--local-offset", bad])).is_err(),
                "{bad}"
            );
        }
        let no_env = |_: &str| None;
        let tz = |n: &str| (n == "TZ").then(|| "CET-1CEST,M3.5.0,M10.5.0/3".to_string());
        let at = |c: bicraft_engine::LocalClock| c.offset_at(0);
        assert_eq!(server_clock(None, &no_env).map(at), Ok(0));
        assert_eq!(server_clock(Some("-05:30"), &tz).map(at), Ok(-330));
        assert!(matches!(
            server_clock(None, &tz),
            Ok(bicraft_engine::LocalClock::Zone(_))
        ));
        assert!(matches!(
            server_clock(Some("utc"), &tz),
            Ok(bicraft_engine::LocalClock::Utc)
        ));
        assert!(!parse(&args(&["--root", "r"])).unwrap().drain_on_stdin_close);
        let o = parse(&args(&[
            "--root",
            "r",
            "--metrics-listen",
            "127.0.0.1:9090",
            "--metrics-workspace-label",
            "--log-format",
            "json",
        ]))
        .unwrap();
        assert_eq!(o.metrics_listen, Some("127.0.0.1:9090".parse().unwrap()));
        assert!(o.metrics_workspace_label);
        assert_eq!(o.log_format, Some(LogFormat::Json));
        let o = parse(&args(&["--root", "r"])).unwrap();
        assert_eq!(
            (o.metrics_listen, o.metrics_workspace_label, o.log_format),
            (None, false, None)
        );
        for (flag, v) in [("--metrics-listen", "9090"), ("--log-format", "xml")] {
            assert!(parse(&args(&["--root", "r", flag, v])).is_err(), "{flag} {v}");
        }
        for (flag, v) in [
            ("--max-upload-bytes", "lots"),
            ("--max-upload-bytes", "0"),
            ("--shutdown-grace", "-1"),
            ("--shutdown-grace", "999999"),
            ("--idle-timeout", "0"),
            ("--ready-memory-percent", "0"),
            ("--ready-memory-percent", "101"),
        ] {
            assert!(parse(&args(&["--root", "r", flag, v])).is_err(), "{flag} {v}");
        }
        let r = DrainReport {
            finished_in_grace: false,
            cancelled: 1,
            unsaved: vec!["sales".into()],
            ..DrainReport::default()
        };
        let s = drain_summary(&r);
        assert!(
            s.contains("1 tool call(s) cancelled") && s.contains("sales") && s.ends_with("exit 3"),
            "{s}"
        );
    }

    #[test]
    fn serve_needs_a_root() {
        let e = parse(&args(&["--listen", "127.0.0.1:0"])).unwrap_err();
        assert!(e.1.contains("--root"), "{}", e.1);
        let e = parse(&args(&["--root", "r", "--queue", "0"])).unwrap_err();
        assert!(e.1.contains("positive"), "{}", e.1);
        // the pre-sandbox flag is gone: the sandbox is always on
        let e = parse(&args(&["--root", "r", "--insecure-local-only"])).unwrap_err();
        assert!(e.1.contains("unexpected argument"), "{}", e.1);
        let o = parse(&args(&[
            "--root",
            "r",
            "--listen",
            "[::1]:0",
            "--allow-origin",
            "https://a.example",
            "--max-body-bytes",
            "1024",
            "--allow-network",
        ]))
        .unwrap();
        assert_eq!(o.listen, "[::1]:0".parse().unwrap());
        assert_eq!(o.allowed_origins, ["https://a.example"]);
        assert_eq!(o.max_body_bytes, Some(1024));
        assert!(o.allow_network);
        let o = parse(&args(&["--root", "r"])).unwrap();
        assert_eq!(o.listen, DEFAULT_LISTEN.parse().unwrap());
        assert!(!o.allow_network);
        // a non-loopback address parses: `serve` refuses it unless authentication is configured
        let o = parse(&args(&["--root", "r", "--listen", "0.0.0.0:80"])).unwrap();
        assert!(!o.listen.ip().is_loopback());
    }

    #[test]
    fn auth_from_flags_and_environment() {
        let none = env_of(&[]);
        assert!(auth_config(&AuthFlags::default(), &none).unwrap().is_none());
        let o = parse(&args(&[
            "--root",
            "r",
            "--oidc-issuer",
            "https://login.example.com/t/v2.0",
            "--oidc-audience",
            "api://bicraft",
            "--oidc-scope",
            "bicraft.use",
            "--trusted-proxy",
            "10.0.0.0/8",
            "--proxy-user-header",
            "Remote-User",
            "--grant",
            "group:bi=*",
            "--reader-grant",
            "group:viewers=sales,hr",
            "--public-url",
            "https://bi.example.com",
        ]))
        .unwrap();
        let env = env_of(&[
            ("BICRAFT_OIDC_ISSUERS", "https://accounts.google.com"),
            ("BICRAFT_AUTH_GRANTS", "email:ana@example.com=sales;token:ci=*"),
            ("BICRAFT_AUTH_READER_GRANTS", "token:dash=*"),
            ("BICRAFT_TRUSTED_PROXIES", "192.168.0.0/16, ::1"),
        ]);
        let c = auth_config(&o.auth, &env).unwrap().unwrap();
        assert_eq!(c.issuers.len(), 2);
        assert!(c.issuers.iter().all(|i| i.audiences == ["api://bicraft"]));
        assert!(c.issuers.iter().all(|i| i.required_scopes == ["bicraft.use"]));
        let p = c.proxy.as_ref().unwrap();
        assert_eq!(p.trusted.len(), 3);
        assert_eq!(p.user_header, "Remote-User");
        assert_eq!(c.acl.grants.len(), 5);
        let readers: Vec<String> = c
            .acl
            .grants
            .iter()
            .filter(|g| g.access == Access::Reader)
            .map(|g| format!("{:?}", g.subject))
            .collect();
        assert_eq!(readers, ["Token(\"dash\")", "Group(\"viewers\")"]);
        assert_eq!(c.public_url.as_deref(), Some("https://bi.example.com"));
        Authenticator::new(c).unwrap();
        // errors: header without proxy, audience without issuer, bad grant, bad CIDR, URL alone
        for (flags, why) in [
            (vec!["--proxy-user-header", "X"], "--trusted-proxy"),
            (vec!["--oidc-audience", "a"], "--oidc-issuer"),
            (vec!["--grant", "nobody"], "SUBJECT=WORKSPACES"),
            (vec!["--reader-grant", "nobody"], "SUBJECT=WORKSPACES"),
            (vec!["--trusted-proxy", "10.0.0.0/99"], "CIDR"),
            (vec!["--public-url", "https://x.example"], "authentication mode"),
        ] {
            let mut a = vec!["--root", "r"];
            a.extend(flags);
            let o = parse(&args(&a)).unwrap();
            let e = auth_config(&o.auth, &none).unwrap_err();
            assert!(e.contains(why), "{e}");
        }
    }

    #[test]
    fn hosts_from_the_public_url() {
        let none = env_of(&[]);
        let f = AuthFlags::default();
        assert_eq!(host_policy(&f, &none, None), HostPolicy::Loopback);
        assert_eq!(
            host_policy(&f, &none, Some("https://bi.example.com")),
            HostPolicy::Names(vec!["bi.example.com".into(), "bi.example.com:443".into()])
        );
        assert_eq!(
            host_policy(&f, &none, Some("https://bi.example.com:443")),
            HostPolicy::Names(vec!["bi.example.com:443".into(), "bi.example.com".into()])
        );
        assert_eq!(
            host_policy(&f, &none, Some("http://bi.internal:8080")),
            HostPolicy::Names(vec!["bi.internal:8080".into()])
        );
        assert_eq!(
            host_policy(&f, &none, Some("https://[2001:db8::1]")),
            HostPolicy::Names(vec!["[2001:db8::1]".into(), "[2001:db8::1]:443".into()])
        );
        let f = AuthFlags {
            allow_hosts: vec!["bicraft.svc".into()],
            ..AuthFlags::default()
        };
        let env = env_of(&[("BICRAFT_ALLOWED_HOSTS", "a.example,b.example")]);
        assert_eq!(
            host_policy(&f, &env, None),
            HostPolicy::Names(vec!["a.example".into(), "b.example".into(), "bicraft.svc".into()])
        );
    }
}
