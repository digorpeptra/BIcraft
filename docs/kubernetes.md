# Running `bicraft-cli serve` on Kubernetes

Status: web stage 1, last lot (2026-10-27, branch `feat/deploy`); the workspace controller of stage 2
(2026-10-28, branch `feat/controller`, section "The controller"). Files: `deploy/docker/Dockerfile`,
the Helm chart `deploy/helm/bicraft/`, plain manifests in `deploy/manifests/`. Checks:
`cargo xtask deploy check` (also a unit test in `cargo xtask ci`), `cargo xtask deploy smoke`
(container-free) and `cargo xtask deploy kind-smoke` (the image in a throwaway kind cluster). Plan and
decisions: `docs/web-deployment.md`; authentication: `docs/auth.md`; sandbox: `docs/server-mode.md`;
memory: `docs/beyond-ram.md`.

Agents first: MCP Streamable HTTP (revision `2026-07-28`) at `/w/<workspace>/mcp` and the
workspace's files at `/w/<workspace>/files/`; people second: the live viewer at `/w/<workspace>/view/`
(section "Viewer", stage 2).

## Topology

One Helm release = one workspace = one pod:

- a `Deployment` with **1 replica** and the **Recreate** strategy (never two sessions on one
  volume);
- a `PersistentVolumeClaim` (`ReadWriteOnce`) mounted at `/srv/bicraft/<workspace>`; the server
  root is `/srv/bicraft`, so the workspace id is the folder's name;
- a disk-backed `emptyDir` with a `sizeLimit` at `/tmp`, used as `TMPDIR` and `BICRAFT_SPILL_DIR`
  (spill files of models larger than the memory budget; a `tmpfs` would count against the pod's
  memory and defeat the budget);
- a `ClusterIP` `Service`, an optional `Ingress` or Gateway API `HTTPRoute` routing `/w/<id>/`
  (and `/.well-known/oauth-protected-resource` for OIDC) to it;
- a `NetworkPolicy`: ingress on the HTTP port, egress to DNS only unless CIDRs are allowed;
  optionally a `CiliumNetworkPolicy` for egress by host name;
- the API tokens file and the authentication configuration as `Secret`s mounted read-only, the
  Databricks token as a `Secret` variable.

Several workspaces = several releases (`helm install sales …`, `helm install finance …`), or one
release of the controller chart that creates them on demand (section "The controller").

## The image

```sh
docker build -f deploy/docker/Dockerfile -t registry.example.com/bicraft:0.1.0 .
```

Multi-stage: `rust:1.99.1-alpine3.22` (pinned by tag and digest) builds the static musl binary with
`cargo build --profile dist --locked` (the release archives' profile) for the build platform
(amd64 or arm64); the final stage is `FROM scratch` with `/usr/local/bin/bicraft-cli`, an empty
`/srv/bicraft` owned by UID 65532 and an empty `/tmp`. Nothing else: no shell, no package
manager, no CA bundle. The HTTPS clients (OIDC discovery and JWKS, Databricks) use the Mozilla
roots compiled into the binary (`webpki-roots`); **an issuer behind a private CA is not trusted**
(no system store is read; a later lot could add a configurable CA file).

- `USER 65532:65532`, `ENV TMPDIR=/tmp BICRAFT_SPILL_DIR=/tmp`, `EXPOSE 7878`.
- `ENTRYPOINT ["/usr/local/bin/bicraft-cli", "serve"]`, `CMD ["--root", "/srv/bicraft", "--listen",
  "0.0.0.0:7878"]`. `bicraft-cli serve` refuses a non-loopback address until authentication and the
  `Host` names are configured, so the image does not start without them: that is deliberate.
- `docker run --rm IMAGE token create --name ci --role agent` prints a token and its entry (the
  entry point already says `serve`).
- To package a static binary built beforehand instead of compiling:
  `--build-arg BINARY_SOURCE=prebuilt --build-context prebuilt=DIR` (DIR holds `bicraft-cli`, built
  for `x86_64-unknown-linux-musl`; the release archive's `bin/bicraft-cli` links glibc and does not
  run on the empty base).
- Nothing is pushed anywhere by the repository's tooling. Push to a registry you control.

The musl allocator is slower than glibc's under heavy multithreaded allocation
(`docs/releasing.md`); measuring the engine bench in the container is a stage-3 item.

## Install with Helm

```sh
# 1. a token for the agent (keep the token; the entry holds its SHA-256 only)
docker run --rm registry.example.com/bicraft:0.1.0 token create --name agent --role agent
echo '{"tokens": [<entry>]}' > tokens.json
kubectl create namespace bi
kubectl -n bi create secret generic bicraft-tokens --from-file=tokens.json

# 2. the release
helm install sales deploy/helm/bicraft -n bi \
  --set image.repository=registry.example.com/bicraft \
  --set workspace.id=sales \
  --set auth.apiTokens.existingSecret=bicraft-tokens \
  --set 'auth.grants={token:agent=sales}' \
  --set model.memoryLimitMiB=6080 --set model.cpu=4 \
  --set persistence.size=50Gi --set spill.sizeLimit=20Gi

# 3. a model into the workspace, then MCP
kubectl -n bi port-forward svc/sales-bicraft 7878:7878 &
curl -H 'Host: sales-bicraft:7878' -H "Authorization: Bearer $TOKEN" \
  -T model.pbix http://127.0.0.1:7878/w/sales/files/model.pbix
```

The agent's MCP URL is `<public URL>/w/sales/mcp` (or `http://sales-bicraft.bi.svc:7878/w/sales/mcp`
inside the cluster) with `Authorization: Bearer <token>`. The chart refuses to render without an
authentication mode and a grant, editor or reader (`bicraft.validate`), and an invalid workspace id.

The server answers only the `Host` names it knows. The chart adds the Service's names
(`sales-bicraft`, `sales-bicraft.bi`, `…bi.svc`, `…bi.svc.cluster.local`, each with and without
`:7878`), the ingress and route hosts and `auth.allowedHosts`; the public URL's host is added by the
server. Through `kubectl port-forward`, send one of those names as `Host`, or add
`127.0.0.1:7878` to `auth.allowedHosts`.

## Values reference

| Value | Default | Meaning |
|---|---|---|
| `image.repository`, `image.tag`, `image.digest`, `image.pullPolicy` | `bicraft`, appVersion, none, `IfNotPresent` | the image; a digest pins it |
| `workspace.id` | `main` | the workspace (URL segment and folder): 1–64 of `[A-Za-z0-9._-]`, not starting with `.` |
| `workspace.root` | `/srv/bicraft` | `--root`; the volume is mounted at `<root>/<id>` |
| `persistence.existingClaim`, `storageClass`, `accessModes`, `size`, `annotations` | none, default class, `[ReadWriteOnce]`, `20Gi` | the workspace volume |
| `persistence.keep` | `true` | `helm.sh/resource-policy: keep`: the volume outlives the release |
| `model.memoryLimitMiB` | `1984` | `BICRAFT_MEMORY_LIMIT`; **pod memory limit = this + 64 MiB**; 0 = no limit |
| `model.memoryBudgetMiB` | `0` | `BICRAFT_MEMORY_BUDGET`; 0 = half of the limit |
| `model.memoryRequestMiB` | `0` | 0 = the pod limit (Guaranteed QoS with a matching CPU request) |
| `model.cpu` | `2` | CPU limit in whole cores |
| `model.cpuRequest` | `""` | CPU request; empty = the limit |
| `model.scanThreads` | `0` | `BICRAFT_SCAN_THREADS`; 0 = `model.cpu` |
| `model.aggThreads` | `0` | `BICRAFT_AGG_THREADS`; 0 = `model.cpu / 4`, at least 1 |
| `model.aggregations` | `true` | `BICRAFT_AGGREGATIONS` (report-derived aggregations) |
| `spill.sizeLimit` | `20Gi` | the `/tmp` emptyDir (`TMPDIR`, `BICRAFT_SPILL_DIR`) |
| `server.port` | `7878` | container port (`--listen 0.0.0.0:<port>`) |
| `server.shutdownGraceSeconds` | `30` | `--shutdown-grace` |
| `server.terminationGraceMarginSeconds` | `5` | `terminationGracePeriodSeconds` = grace + margin |
| `server.maxUploadBytes` | `4G` | `--max-upload-bytes` |
| `server.maxBodyBytes`, `server.queue` | `0` | `--max-body-bytes`, `--queue`; 0 = the server's defaults (8 MiB, 8) |
| `server.maxConcurrentReads` | `0` | `--max-concurrent-reads`: read-only calls and viewer reads running at once on the workspace's session (0 = the server's default, 4; 1 = one at a time); edits run alone (`docs/web-deployment.md`, "Concurrency on one workspace"). Each running query has its own query budget: size `model.memoryLimitMiB` for them |
| `server.idleTimeoutSeconds` | `0` | `--idle-timeout`; 0 = off |
| `server.readyMemoryPercent` | `0` | `--ready-memory-percent`; 0 = off |
| `server.localOffset` | `""` | `--local-offset`: Power Query's local clock (`+02:00`, minutes, `UTC`, `machine`); empty = UTC, as in the Power BI service (docs/formats/data-sources.md, "Clock") |
| `recovery.enabled` | `true` | `--recovery-dir <mountPath>`: journals of unsaved edits (docs/recovery.md), one subfolder per workspace, on a claim of their own |
| `recovery.mountPath` | `/var/lib/bicraft/recovery` | must be outside `workspace.root` (the chart refuses otherwise) |
| `recovery.persistence.existingClaim`, `storageClass`, `accessModes`, `size`, `keep` | none, default class, `[ReadWriteOnce]`, `1Gi`, `true` | the recovery volume (`<release>-recovery`) |
| `recovery.allowAutosave` | `false` | `--allow-autosave`: the app may save PBIP projects automatically |
| `server.allowedOrigins` | `[]` | `--allow-origin` (browsers only) |
| `server.extraArgs`, `server.extraEnv` | `[]` | more `serve` arguments, more variables |
| `metrics.enabled` | `true` | `--metrics-listen <host>:<port>`: Prometheus `/metrics` on a port of its own (see "Observability") |
| `metrics.port`, `metrics.host` | `9090`, `$(POD_IP)` | the metrics port and bind address (the pod IP from the downward API; `[$(POD_IP)]` on IPv6, `0.0.0.0` for `kubectl port-forward`) |
| `metrics.workspaceLabel` | `false` | `--metrics-workspace-label` (per-workspace series) |
| `metrics.podAnnotations` | `true` | `prometheus.io/scrape`, `/port`, `/path` on the pod |
| `metrics.scrapeFrom` | `[]` | NetworkPolicy peers allowed on the metrics port; empty = any pod |
| `metrics.serviceMonitor.enabled`, `.interval`, `.scrapeTimeout`, `.labels` | off, `30s`, `10s`, `{}` | a headless `<release>-metrics` Service and a Prometheus Operator `ServiceMonitor` |
| `logging.format` | `json` | `BICRAFT_LOG_FORMAT`: `json` (one line per request) or `text` |
| `auth.publicUrl` | `""` | `BICRAFT_PUBLIC_URL` |
| `auth.allowedHosts` | `[]` | added to `BICRAFT_ALLOWED_HOSTS` |
| `auth.grants` | `[]` | `BICRAFT_AUTH_GRANTS` (`SUBJECT=WORKSPACES`): editors |
| `auth.readerGrants` | `[]` | `BICRAFT_AUTH_READER_GRANTS` (same form): readers, read-only commands and file downloads (docs/auth.md "Roles"); an identity matching both is an editor |
| `auth.apiTokens.existingSecret` / `.key` / `.tokensJson` | | the tokens file (`BICRAFT_API_TOKENS_FILE`) from a Secret, or inline (hashes only) |
| `auth.oidc.issuers`, `.audiences`, `.scopes` | `[]` | `BICRAFT_OIDC_ISSUERS`, `_AUDIENCES`, `_SCOPES` |
| `auth.trustedProxy.cidrs` | `[]` | `BICRAFT_TRUSTED_PROXIES` |
| `auth.trustedProxy.userHeader`, `.emailHeader`, `.groupsHeader` | `""` | `--proxy-user-header`, `--proxy-email-header`, `--proxy-groups-header` |
| `auth.config.existingSecret` / `.key` / `.json` | | the JSON configuration (`BICRAFT_AUTH_CONFIG`) |
| `databricks.enabled`, `.existingSecret`, `.key` | `false` | `BICRAFT_SERVER_NETWORK=1` and `BICRAFT_DATABRICKS_TOKEN` from the Secret |
| `service.type`, `.port`, `.annotations` | `ClusterIP`, `7878` | |
| `ingress.enabled`, `.className`, `.annotations`, `.hosts`, `.tls` | off; ingress-nginx annotations (4g bodies, no buffering, 660 s timeouts) | |
| `httpRoute.enabled`, `.parentRefs`, `.hostnames`, `.timeout` | off, 660 s | Gateway API route |
| `networkPolicy.enabled` | `true` | ingress on the port; egress DNS only |
| `networkPolicy.ingressFrom` | `[]` | allowed peers; empty = any pod |
| `networkPolicy.egressCidrs`, `.egressPorts` | `[]`, `[443]` | egress for an OIDC issuer or Databricks |
| `networkPolicy.dns` | kube-system / `k8s-app: kube-dns` | where DNS runs |
| `networkPolicy.cilium.enabled`, `.fqdns` | off | `CiliumNetworkPolicy` with `toFQDNs` |
| `serviceAccount.create`, `.name`, `.annotations` | off | the token is never mounted |
| `podSecurityContext.runAsUser`, `runAsGroup`, `fsGroup` | `65532` | the fixed identity |
| `probes.liveness.*`, `probes.readiness.*` | 6 failures × 10 s, 2 × 5 s | |
| `podAnnotations`, `podLabels`, `nodeSelector`, `tolerations`, `affinity`, `priorityClassName` | | |

Not configurable, on purpose: one replica, Recreate, `runAsNonRoot`, `readOnlyRootFilesystem`,
`allowPrivilegeEscalation: false`, all capabilities dropped, seccomp `RuntimeDefault`,
`automountServiceAccountToken: false`, `enableServiceLinks: false` (a Service called `bicraft`
would inject `<SERVICE>_PORT`-style variables named `BICRAFT_…`), a disk-backed `/tmp`. The pods meet the `restricted`
Pod Security Standard.

## Sizing

Model sizes are an admin setting per release. Figures from `docs/benchmarks.md` (100 M-row star
schema, "Milestone 2") and `docs/beyond-ram.md`:

| Fact rows | `file.open` | Peak working set | Suggested `memoryLimitMiB` | Pod limit | `cpu` |
|---|---|---|---|---|---|
| 1 M | 28 ms | 0.11 GB | 448 | 512 Mi | 1 |
| 10 M | 68 ms | 0.84 GB | 1984 | 2 Gi | 2 |
| 100 M | 0.44 s | 5.40 GB (6.90 GB with aggregations) | 6080 (aggregations off) or 8128 | 6 Gi or 8 Gi | 4–8 |

- **Pod memory limit = `BICRAFT_MEMORY_LIMIT` + 64 MiB** (lot 5's measured base margin); the chart
  computes it. Under a limit there is always a budget (half of the limit unless
  `memoryBudgetMiB`); beyond it decoded columns are evicted and the decompressed backup goes to
  `/tmp`, so a smaller limit trades speed for memory rather than failing (`docs/beyond-ram.md`).
- `spill.sizeLimit`: at least the decompressed size of the largest model's backup, plus the staging
  of zipped project uploads. `persistence.size`: the models, their saved copies, and one upload in
  flight (an upload is written next to its target before replacing it).
- `BICRAFT_SCAN_THREADS` = the CPU limit (explicit, reproducible), `BICRAFT_AGG_THREADS` a quarter.
- Aggregations are rebuilt in the background after each start (not persisted yet).

## Authentication setups

All three modes of `docs/auth.md` work through values; they can be combined.

**API tokens (agents, CI).** As in "Install with Helm": tokens file in a Secret
(`auth.apiTokens.existingSecret`), grants `token:<name>=<workspace>`. Rotation: add the new entry,
roll the pod (the file is read at start; inline `tokensJson` changes restart the pod by checksum),
switch the agent, remove the old entry.

**OIDC (any issuer).** Set `auth.publicUrl` (audiences and resource metadata use it), the issuer,
and allow the pod to reach the issuer's keys:

```yaml
auth:
  publicUrl: https://bicraft.example.com
  oidc:
    issuers: ["https://login.microsoftonline.com/<tenant>/v2.0"]
    audiences: ["<api-client-id>"]
  grants: ["role:editor=*"]
ingress:
  enabled: true
  hosts: [bicraft.example.com]
  tls: [{secretName: bicraft-tls, hosts: [bicraft.example.com]}]
networkPolicy:
  egressCidrs: ["<issuer CIDRs>"]        # or cilium.fqdns: [login.microsoftonline.com]
```

Claim names, role maps and required claims need the JSON file: `auth.config.json` (or a Secret in
`auth.config.existingSecret`) with the content of `docs/auth.md`, "Configuration". Plain
`NetworkPolicy` only knows CIDRs; issuers behind changing addresses need the Cilium option (or
your CNI's FQDN policy), else egress has to be opened wider.

**Trusted header behind oauth2-proxy.** The proxy authenticates people and forwards
`X-Forwarded-User` / `-Email` / `-Groups`; BIcraft trusts them only from the proxy's addresses, so
restrict ingress to the proxy as well:

```yaml
auth:
  trustedProxy:
    cidrs: ["10.42.0.0/16"]               # the proxy pods' (or the ingress controller's) range
  grants: ["group:bi-team@example.com=*"]
  allowedHosts: [bicraft.example.com]
networkPolicy:
  ingressFrom:
    - namespaceSelector: {matchLabels: {kubernetes.io/metadata.name: oauth2-proxy}}
```

Behind ingress-nginx's `auth-url` with oauth2-proxy as the auth service, the identity arrives as
`X-Auth-Request-*`: set `auth.trustedProxy.userHeader: X-Auth-Request-User` (and the e-mail and
groups headers) and trust the ingress controller's range. The proxy must strip these headers from
client requests.

## Plain manifests

For clusters without Helm, `deploy/manifests/` holds the same objects for workspace `main` in
namespace `bicraft` (2 GiB pod: `BICRAFT_MEMORY_LIMIT=1984M`, 2 CPUs):

```sh
kubectl create namespace bicraft
kubectl -n bicraft create secret generic bicraft-tokens --from-file=tokens.json
kubectl apply -k deploy/manifests          # namespace, PVC, Deployment, Service, NetworkPolicy
# optional: edit and apply deploy/manifests/ingress.yaml; with the Prometheus Operator, servicemonitor.yaml
```

To change the size, keep the rules together: memory limit = `BICRAFT_MEMORY_LIMIT` + 64 MiB,
`BICRAFT_SCAN_THREADS` = the CPU limit, `terminationGracePeriodSeconds` > `--shutdown-grace`.
`cargo xtask deploy check` enforces them.

## Upgrades, restarts and the drain

On SIGTERM (pod deletion, `helm upgrade`, a node drain) `bicraft-cli serve`:

1. answers 503 on `/readyz` and to every new request (`Retry-After: 5`);
2. gives running requests `--shutdown-grace` seconds (`server.shutdownGraceSeconds`), then cancels
   the DAX queries still running;
3. **saves nothing**: unsaved changes of a workspace session are discarded and reported by
   workspace id in the log; spill and upload temporary files are removed;
4. exits 0 (nothing lost) or 3.

The pod gets `terminationGracePeriodSeconds` = grace + 5 s, so the kubelet never kills it in the
middle of the drain. With Recreate, an upgrade stops the old pod before the new one mounts the
volume: agents see 503 then a short refusal, and must reopen the file (`file.open`) and save their
work before upgrades. The workspace's files on the volume are untouched.

## Observability

Two outputs, neither carrying model data (`docs/web-deployment.md`, "Logs never carry model data").

**Logs.** `--log-format json` (or `BICRAFT_LOG_FORMAT=json`, the chart's and the manifests' default)
writes one JSON object per line on stderr: every request (probes excepted) and every server event
(`start`, `idle`, `reopen`, `drain`, `auth`). The default `text` keeps the human event lines and
writes nothing per request. A request line:

```json
{"ts":"2026-10-27T08:15:02.123Z","level":"info","event":"request","route":"mcp","method":"POST",
 "workspace":"main","caller":"token","rpc":"tools/call","tool":"evaluate_dax_query",
 "command":"dax.evaluate","status":200,"outcome":"ok","duration_ms":12.345,"bytes_in":312,"bytes_out":845}
```

| Field | Values |
|---|---|
| `route` | `mcp`, `files`, `view`, `resource_metadata`, `other` |
| `workspace` | the workspace id (an operator-chosen folder name, validated) |
| `caller` | the authentication **mode** only: `local` (no authentication), `oidc`, `proxy`, `token`, `unauthenticated` (refused), `none` (public routes) |
| `rpc`, `tool`, `command` | the JSON-RPC method, the tool and the engine command id, each from a fixed set (`other`, `unknown` otherwise): never the client's text |
| `status`, `outcome`, `error` | HTTP status; `ok`, `error` (a tool call that failed: `command_error`), `refused` (the caller's role refused the call before anything ran, a reader calling an editing command: `forbidden`), `rejected`, `cancelled`, `closed`; the JSON-RPC error kind (`busy`, `unauthorized`, `header_mismatch`…) or the status class |
| `duration_ms`, `bytes_in`, `bytes_out` | from the request head to the last byte of the answer (not the lingering close); declared request body; response bytes (downloads as declared) |

Never logged: DAX text, command parameters, values, query results, file names or paths (a file
route logs `route: files` and the method, not the name), tokens, and **user ids**. The choice is to
omit them rather than hash them: a hash of an e-mail address is reversible by dictionary, and a
salted one cannot be joined with the identity provider's sign-in log, which is where per-user audit
belongs. `crates/mcp/tests/http_metrics.rs` proves it with a query whose text, result and error
carry a distinctive literal, file names carrying it, and a `run_command` naming it as a command; and
`deploy smoke` checks the live log for the query, the fixture's data, the file names and the token.

**Metrics.** `--metrics-listen ADDR:PORT` (off by default in the binary, on in the chart at
`$(POD_IP):9090`) starts a second listener that serves `GET /metrics` (Prometheus text format
0.0.4) and nothing else (404; 405 for other methods). It has **no authentication and no `Host`
check** (Prometheus scrapes the pod address), so bind it to the pod IP or loopback and restrict it
with the NetworkPolicy (`metrics.scrapeFrom`); the MCP listener never serves `/metrics` (404). It
stays up during the drain (so `bicraft_draining` is seen) and closes when the process exits.

| Family | Type | Labels |
|---|---|---|
| `bicraft_http_requests_total` | counter | `route`, `code` (26 known codes, `other`, `none`) |
| `bicraft_http_request_duration_seconds` | histogram | `route` |
| `bicraft_http_request_bytes_total`, `bicraft_http_response_bytes_total` | counter | `route` |
| `bicraft_http_requests_in_flight`, `bicraft_http_connections_open` | gauge | |
| `bicraft_command_duration_seconds` | histogram (1 ms to 600 s) | `command`: an engine command id, `list_commands`, `batch` or `unknown`. **Open and save durations** are `command="file.open"` (lazy reopens after an idle close included) and `file.save` / `file.save_as` |
| `bicraft_commands_total` | counter | `command`, `outcome` (`ok`, `error`, `cancelled`, `refused`: a reader's call refused by its role) |
| `bicraft_queue_wait_seconds` | histogram | |
| `bicraft_tool_calls_queued`, `bicraft_tool_calls_running` | gauge | queued: tool calls waiting now at their workspace's session lock (behind a running call, or behind a waiting write); a call that gets the lock at once is never counted |
| `bicraft_queue_rejections_total` | counter | 503 `busy` answers |
| `bicraft_concurrent_reads` | gauge | read-only calls and viewer reads running now on shared sessions |
| `bicraft_write_lock_wait_seconds` | histogram | how long a call that may change a session waited for the running reads |
| `bicraft_revision_conflicts_total` | counter | calls refused because their `expected_revision` did not match (nothing ran) |
| `bicraft_workspaces_open`, `bicraft_workspaces_max` | gauge | |
| `bicraft_workspace_idle_closes_total`, `bicraft_workspace_reopens_total`, `bicraft_workspace_reopen_failures_total` | counter | |
| `bicraft_memory_limit_bytes` (0: no limit), `bicraft_memory_reserved_bytes`, `bicraft_memory_peak_reserved_bytes` | gauge | |
| `bicraft_memory_reserved_class_bytes` | gauge | `class` (`kept`, `query`, `background`, `io`) |
| `bicraft_memory_evictions_total`, `bicraft_memory_refused_total` | counter | |
| `bicraft_memory_running_queries`, `bicraft_process_resident_bytes` | gauge | |
| `bicraft_spill_files`, `bicraft_spill_bytes` | gauge | backups moved to `BICRAFT_SPILL_DIR` under the budget |
| `bicraft_aggregation_builds_running` | gauge | |
| `bicraft_aggregation_builds_total` | counter | `result` (`built`, `stopped`) |
| `bicraft_draining`, `bicraft_build_info` (`version`), `bicraft_start_time_seconds` | gauge | |
| `bicraft_workspace_tool_calls_total`, `bicraft_workspace_tool_call_seconds_total` | counter | `workspace`, only with `--metrics-workspace-label` (and only for workspaces an authorized tool call opened; 1024 at most) |
| `bicraft_document_views_open` | gauge | `mode` (`edit`: an editor's session of its own; `read`: a reader's): browser tabs on their own file (docs/mcp.md, "Document views") |
| `bicraft_document_views_max` | gauge | `--max-views` |
| `bicraft_workspace_document_views_open` | gauge | `workspace`, only with `--metrics-workspace-label` |
| `bicraft_recovery_journals`, `bicraft_recovery_journal_bytes` | gauge | journals of unsaved edits kept in `--recovery-dir`, and their bytes with the recovery copies (docs/recovery.md); read when scraped |
| `bicraft_workspace_recovery_journals` | gauge | `workspace`, only with `--metrics-workspace-label` |
| `bicraft_control_requests_total`, `bicraft_control_request_errors_total` | counter | agents' requests to the UI control channel's relay (`--viewer-control`), and those that failed |

The memory series come from the process-wide accounting of `BICRAFT_MEMORY_LIMIT`
(`docs/beyond-ram.md`, lot 5): without a limit nothing is counted and they read 0. Per-session
details (`dax.memory`) stay a command: a scrape never locks a session.

Label cardinality is bounded by construction: routes, codes, outcomes and classes are fixed sets,
command labels come from the engine's command table (a series appears once used), and workspace
ids are opt-in. Cost on the query path: atomics and one thread-local record (no lock), measured at
**167 ns per tool call** for the command histogram, the queue-wait histogram and the request
counters together (`cargo test -p bicraft-mcp --release --lib cost_per_request -- --ignored`,
Windows, one thread), against tool calls of 100 µs and up; the JSON line, when enabled, is
formatted after the answer is sent.

**Scraping.** The pod carries `prometheus.io/scrape|port|path` annotations; with the Prometheus
Operator set `metrics.serviceMonitor.enabled` (a headless `<release>-metrics` Service and a
`ServiceMonitor`; add the labels your Prometheus selects by). Plain manifests: the same annotations,
and `deploy/manifests/servicemonitor.yaml`.

**Dashboard.** `deploy/grafana/bicraft-dashboard.json` (import it, pick the Prometheus data
source): draining, open workspaces, in flight and queued, memory used / limit, queue rejections,
version; requests per second by route and status; request latency p50/p95; command latency p50/p95
and outcomes by command id; memory by class against the limit and the resident set; evictions and
refusals; spill bytes; aggregation builds; queue wait. `cargo xtask deploy check` verifies that
every metric the dashboard queries is exposed.

## The controller

Web stage 2 (2026-10-27, branch `feat/controller`). Files: the crate `crates/controller`
(`bicraft-controller`, L6), the subcommand `bicraft-cli controller` (`apps/bicraft-cli/src/controller_cmd.rs`),
the chart `deploy/helm/bicraft-controller/`. Design and the reasons behind it:
`docs/web-deployment.md`, "Workspace controller (stage 2)".

Instead of one Helm release per workspace, one release of the controller chart serves every
workspace of a namespace:

```
agents ──HTTPS──▶ Ingress / HTTPRoute ──▶ bicraft-cli controller (gateway + reconciler)
                                             │  /api/v1/workspaces    (admin API)
                                             │  /w/<id>/…  ──▶ Service bws-<id> ──▶ pod bws-<id> (bicraft-cli serve)
                                             └─ Kubernetes API: BicraftWorkspace, Deployment, Service,
                                                NetworkPolicy, PersistentVolumeClaim (server-side apply)
```

### The resource

```yaml
apiVersion: bicraft.dev/v1alpha1      # crd.group: a placeholder until the project's domain is chosen
kind: BicraftWorkspace
metadata:
  name: sales                          # the workspace id: a DNS label of at most 40 characters
spec:
  sizeClass: medium                    # one of workspaces.classes (admin-defined)
  volume: {size: 20Gi, reclaimPolicy: Retain}   # size at most the class's maxVolumeSize
  idleTimeoutSeconds: 900              # 0: never stopped for idleness
  grants: ["token:agent", "group:bi-team"]   # editor access
  readers: ["group:bi-viewers"]               # reader access (docs/auth.md "Roles")
  egress: []                           # names of workspaces.egressProfiles
  suspend: false
status:                                # written by the controller
  phase: Idle | Starting | Running | Failed
  message: …
  endpoint: https://bi.example.com/w/sales/mcp
  lastActivity: "2026-10-27T10:00:00Z"
  volumeClaim: bws-sales-data
  recoveryClaim: bws-sales-recovery
```

`kubectl get bws` lists the workspaces with their phase, class and last activity. For each one the
controller applies (server-side apply, field manager `bicraft-controller`):

| Object | Name | Notes |
|---|---|---|
| `Deployment` | `bws-<id>` | 1 replica when awake, 0 when idle or suspended; Recreate; the stage-1 pod (UID 65532, read-only root, no capabilities, seccomp, disk-backed `/tmp`, probes, memory limit = class `memoryLimitMiB` + 64 MiB, `BICRAFT_SCAN_THREADS` = class CPU) |
| `Service` | `bws-<id>` | ClusterIP, port 7878 |
| `NetworkPolicy` | `bws-<id>` | ingress from the controller's pods only; egress DNS plus the workspace's egress profiles |
| `PersistentVolumeClaim` | `bws-<id>-data` | `ReadWriteOnce`, mounted at `/srv/bicraft/<id>` |
| `PersistentVolumeClaim` | `bws-<id>-recovery` | the recovery journals (`--recovery-dir`, docs/recovery.md), mounted at `/var/lib/bicraft/recovery`, outside the files; `workspaces.recovery.volumeSize` (1Gi); same reclaim policy as the data claim; none with `workspaces.recovery.enabled: false` |

Every object but a `Retain` claim carries an owner reference to the workspace: **deleting the
workspace deletes them** (Kubernetes' garbage collector). With `reclaimPolicy: Retain` (the
default) the claims stay, and a workspace created again with the same name gets it back; with
`Delete` it goes with the workspace. A spec the configuration refuses (unknown class, volume above
the class's maximum, unknown egress profile, a bad grant) sets `phase: Failed` with the reason and
creates nothing.

### The gateway

The controller's Service is the only entry point: route `/w/`, `/api/` and
`/.well-known/oauth-protected-resource` to it (`ingress.*` or `httpRoute.*`). For each request it

1. answers `/healthz` and `/readyz` (ready once a reconciliation reached the API) without
   authentication;
2. checks `Host` (the Service's names, the ingress and route hosts, `auth.allowedHosts`, the
   public URL's host);
3. authenticates with the three modes of `docs/auth.md` (`auth.*` values, as in the `bicraft`
   chart), answering 401 with the `WWW-Authenticate` challenge otherwise;
4. checks access: an admin (editor), a controller-wide grant (`auth.grants`, `auth.readerGrants`)
   or the workspace's `spec.grants` (editor) and `spec.readers` (reader), the highest level winning;
   otherwise 404, as if the workspace did not exist (nothing wakes);
5. wakes a workspace at zero replicas (applies 1 replica) and holds the request until the pod's
   `/readyz` answers 200 (`workspaces.wakeTimeoutSeconds`, default 180 s, then 503 with
   `Retry-After`);
6. forwards the request, streamed both ways (uploads up to the pod's `--max-upload-bytes`, SSE,
   downloads; nothing is buffered), with `Host` set to the workspace Service, the credentials removed
   (`Authorization`, any identity header) and the checked identity in `X-Bicraft-Gateway-User`,
   `-Email`, `-Groups`; the groups end with `bicraft-gateway-editor` or `bicraft-gateway-reader`, the
   access the gateway granted (identity groups with the `bicraft-gateway-` prefix are dropped).

**Idle stop.** The gateway sees every request, so a workspace with no open connection and no request
for `idleTimeoutSeconds` is scaled back to zero by the next reconciliation (every
`controller.intervalSeconds`, default 5 s). The pod drains as in stage 1: **nothing is saved
implicitly**, an agent must save before it goes idle.

**Workspace pods trust only the gateway.** They run `bicraft-cli serve` in trusted-header mode only
(`--proxy-user-header X-Bicraft-Gateway-User`, `BICRAFT_TRUSTED_PROXIES` = the controller pod's
address from `POD_IP`, or `workspaces.gatewayTrustedCidrs`) with the grants
`group:bicraft-gateway-editor=<id>` (editor) and `group:bicraft-gateway-reader=<id>` (reader), so the
pod applies the reader rules of `docs/auth.md` to readers; their NetworkPolicy admits the controller's
pods only. A pod elsewhere in the cluster cannot reach a
workspace (refused by the policy) and, if it could, its identity headers would be refused (400,
untrusted address). The gateway enforces the workspace's grants; the pod enforces that the request
came through the gateway. This relies on a CNI that enforces NetworkPolicy. With the default
(pod-address trust), a controller restart on a new address re-applies every workspace's Deployment,
which restarts the awake workspaces (sessions are dropped without saving).

### The admin and agent API

All routes need authentication; JSON in and out.

| Request | Who | Answer |
|---|---|---|
| `GET /api/v1/workspaces` | anyone authenticated | 200 `{workspaces: [{name, phase, message, endpoint, lastActivity, volumeClaim, spec}]}`: the workspaces the caller may open |
| `POST /api/v1/workspaces` `{name, sizeClass?, volume?, idleTimeoutSeconds?, grants?, readers?, egress?, suspend?}` | admin | 201 with the workspace (phase `Idle`); 400 with the reason when the configuration refuses it; 409 when it exists |
| `GET /api/v1/workspaces/<id>` | admin or granted | 200; 404 otherwise |
| `DELETE /api/v1/workspaces/<id>` | admin | 202 `{name, deleted, volume}`: `retained: bws-<id>-data` or `deleted with the workspace` |
| `POST /api/v1/workspaces/<id>/wake` | admin or granted | 200 once the pod is ready; 503 past the wake timeout |
| `POST /api/v1/workspaces/<id>/stop` | admin or editor (403 for a reader) | 200; scaled to zero (unsaved changes are lost) |

### Install

```sh
kubectl create namespace bi
kubectl -n bi create secret generic bicraft-tokens --from-file=tokens.json   # admin + agent entries
helm install bi deploy/helm/bicraft-controller -n bi \
  --set image.repository=registry.example.com/bicraft \
  --set auth.apiTokens.existingSecret=bicraft-tokens \
  --set 'controller.admins={token:admin}' \
  --set auth.publicUrl=https://bi.example.com --set ingress.enabled=true --set 'ingress.hosts={bi.example.com}'
curl -H "Authorization: Bearer $ADMIN" -H 'Content-Type: application/json' \
  -d '{"name": "sales", "sizeClass": "medium", "grants": ["token:agent"]}' https://bi.example.com/api/v1/workspaces
# the agent: MCP at https://bi.example.com/w/sales/mcp with its own token; the first request wakes the pod
```

The CRD is cluster-wide; it is installed with the release (`crd.install`, kept on uninstall with
`crd.keep`). Several controller releases in one cluster (one per namespace) share it: install it
with the first and set `crd.install=false` on the others, all with the same `crd.group`.

### Values reference (controller chart)

| Value | Default | Meaning |
|---|---|---|
| `image.*`, `imagePullSecrets` | `bicraft`, appVersion | the image of the controller and of the workspace pods |
| `crd.install`, `crd.group`, `crd.keep` | `true`, `bicraft.dev`, `true` | the CRD; the group is a placeholder until the project's domain is chosen |
| `controller.admins` | `[]` | subjects allowed to create and delete workspaces (`--admin`, also `BICRAFT_CONTROLLER_ADMINS`) |
| `controller.port`, `intervalSeconds`, `shutdownGraceSeconds` | 7878, 5, 30 | the gateway port, the reconciliation period, the drain |
| `controller.resources`, `extraArgs`, `extraEnv` | 100m/64Mi requests, 1 CPU/256Mi limits | |
| `auth.*` | | as in the `bicraft` chart: tokens, OIDC, trusted proxy, configuration file, public URL, allowed hosts, controller-wide grants (`grants`, `readerGrants`) |
| `workspaces.classes`, `defaultClass` | small (448 MiB, 1 CPU), medium (1984 MiB, 2), large (8128 MiB, 8) | the size classes: `memoryLimitMiB`, `memoryBudgetMiB`, `memoryRequestMiB`, `cpu`, `cpuRequest`, `aggregations`, `spillSize`, `volumeSize`, `maxVolumeSize` |
| `workspaces.defaultIdleTimeoutSeconds` | 900 | 0 = never stop |
| `workspaces.defaultReclaimPolicy`, `storageClass` | `Retain`, cluster default | |
| `workspaces.egressProfiles` | `{}` | `{name: {cidrs, ports, allowNetwork, databricksSecret: {name, key}}}` |
| `workspaces.gatewayTrustedCidrs` | `[]` | empty: the controller pod's own address |
| `workspaces.wakeTimeoutSeconds`, `networkPolicy` | 180, `true` | |
| `workspaces.pod.*` | grace 30 s, uploads 4G | `shutdownGraceSeconds`, `maxUploadBytes`, `allowedOrigins`, `extraArgs`, `extraEnv`, `nodeSelector`, `tolerations`, `priorityClassName`, `annotations` |
| `workspaces.pod.maxViewsPerUser`, `maxViews`, `viewIdleSeconds` | 0 (the server's 8, 32, 1800 s) | `--max-views-per-user`, `--max-views`, `--view-idle`: document views (browser tabs on their own file); each editor's is a model in memory, so size the classes for them |
| `workspaces.pod.allowAutosave` | `false` | `--allow-autosave` |
| `workspaces.pod.localOffset` | `""` | `--local-offset` (empty: UTC) |
| `workspaces.pod.viewerControl.existingSecret`, `.key` | `""`, `token` | `--viewer-control` with the token from this Secret (`BICRAFT_VIEWER_CONTROL_TOKEN`, at least 32 characters); the gateway forwards `X-Bicraft-Control` |
| `workspaces.recovery.enabled`, `volumeSize`, `storageClass` | `true`, `1Gi`, the workspace's | the claim `bws-<id>-recovery` and `--recovery-dir` |
| `service.*`, `ingress.*`, `httpRoute.*` | ClusterIP 7878; off | the gateway's route (`/w/`, `/api/`, `/.well-known/oauth-protected-resource`) |
| `networkPolicy.enabled`, `ingressFrom`, `apiServer.cidrs`/`ports`, `dns`, `egressCidrs` | on, any pod, any address on 443/6443 | the controller's own policy: egress to DNS, its workspace pods and the API server |

The controller reads its configuration (`--config`, or `BICRAFT_CONTROLLER_CONFIG`) from the
chart's `ConfigMap`; outside Helm, the same JSON can be written by hand
(`crates/controller/src/config.rs` documents every field).

**RBAC** (namespaced `Role`): `bicraftworkspaces` get/list/watch/create/patch/delete and their
status get/patch; `deployments`, `services`, `persistentvolumeclaims`, `networkpolicies`
get/list/create/patch. No delete on those (the garbage collector removes them), no pods, no secrets,
no cluster role. `cargo xtask deploy check` refuses a wider role.

**Kubernetes API client.** Hand-written over `ureq` + rustls (no new dependency): in a pod it
verifies the API server against the service account's `ca.crt`, sends the projected token and
reads it again every minute and after a 401 (the kubelet rotates it). `--kube-api URL` points it at
another endpoint (e.g. `kubectl proxy`, for development).

### Limits of this lot

- **One controller replica** (single point of failure; the gateway is also the data path, so its
  CPU and network bound the throughput of uploads). Stage 3: leader election for the reconciler
  (a `Lease`) and stateless gateway replicas sharing activity through the workspace status.
- Polling (every few seconds) rather than watches; fine for tens of workspaces.
- Workspaces start at zero replicas and wake on their first request (the volume is bound then, with
  `WaitForFirstConsumer` storage classes).
- Per-workspace Secrets (a Databricks token per workspace) come from egress profiles the admin
  defines, not from the API; the UI control token is one Secret for every workspace.
- Aggregations are rebuilt after every wake-up (not persisted).

## Viewer

Status: web stage 2 (2026-10-28, branch `feat/viewer`). Code: `crates/mcp/src/http/viewer.rs` (routes,
browser login, private views), `crates/mcp/src/http/readview.rs` (what readers read), the page of
`crates/view` (shared with `bicraft-cli view`, whose local behaviour is unchanged: loopback only, no
authentication, open-in-Desktop). Authentication and CSRF: `docs/auth.md`, "The viewer in a browser".

`bicraft-cli serve` serves the report viewer of each workspace at `<public URL>/w/<id>/view/` (the
chart's ingress already routes `/w/<id>/`). Same `Host` allow-list (`auth.publicUrl`,
`auth.allowedHosts`), authentication and grants as the MCP endpoint; a person needs a grant like an
agent (`auth.readerGrants` for read-only people).

| Route | Answer |
|---|---|
| `GET /w/<id>/view` | 308 to `/w/<id>/view/` |
| `GET /w/<id>/view/` | the page; each load gets a **private view** (id and interaction token in the page) |
| `GET /w/<id>/view/app/` | the browser build of the egui app (docs/webui.md), with its own private view like the page; its files `app/boot.js`, `app/bicraft_web.js`, `app/bicraft_web_bg.wasm` (gzip, ETag); 404 in a build without it |
| `GET …/api/state?v=` | the view's snapshot (`docs/view-snapshot.md`), with `source.path` relative to the workspace, `source.download` (the saved file's path for the download link) and `view: {role, session}` (`editor`, `copy` or `empty`) |
| `GET …/api/events?v=` | SSE: an `event: snapshot` each time the view should refetch (an editor's change, a save, an upload, its own interaction; every 250 ms while visuals still compute); keepalives every 15 s; ends at the drain |
| `GET …/api/asset/<hash>?v=` | the report images of the view's snapshot (content-addressed, immutable, sandboxed) |
| `POST …/api/session` | a native client's view (`bicraft`, docs/desktop.md): bearer credentials or the proxy's headers only, never the cookie; `{version: 1, view, token, access}` |
| `POST …/api/interact?v=` | a slicer click, a data-point selection, a page change, a slicer button: on **this view only** |
| `POST /w/<id>/view/login`, `GET /w/<id>/view/login/<code>` | the one-time browser login (bearer credential → URL → cookie) |

There is no open-in-Desktop route on a server; the page's button becomes a download of the saved
file through `/w/<id>/files/`. Nothing the viewer does writes the project, for editors too.

**Readers, editors and memory.** Editors see the editor session (the one their MCP calls use,
unsaved edits included). Readers see saved files only: the clean editor session itself when it has
their file open (no extra memory), otherwise one read-only copy per file, shared by every reader of
it (`viewer.copies`, default 1) and reloaded when the file changes (`docs/auth.md`, "Roles"). The
view state of a page (slicers, selections, current page) is a few KB per page, kept apart from the
session: N readers on a 100 M-row model cost one model (two while an editor holds unsaved edits),
never N. Size `model.memoryLimitMiB` for two models if readers browse while editors edit; a copy
that does not fit is refused by the memory limit and the reader is told to retry. Copies unused for
10 minutes go.

| Value | Flag | Default | Meaning |
|---|---|---|---|
| `viewer.enabled` | `--no-viewer` when false | true | serve the viewer |
| `viewer.copies` | `--viewer-copies N` | 1 | read-only copies of saved files kept for readers; past it, a copy nobody uses makes room, else a retryable 503 |
| `viewer.maxViews` | `--max-viewers N` | 256 | private views kept per workspace (the least recently used goes; its page asks to be reloaded) |
| `viewer.documentViews.perUser` | `--max-views-per-user N` | 0 (8) | document views (a browser tab on its own file, a session of its own) one identity holds at once |
| `viewer.documentViews.perWorkspace` | `--max-views N` | 0 (32) | document views open on the workspace; an editor's is a model in memory: size `model.memoryLimitMiB` for them |
| `viewer.documentViews.idleSeconds` | `--view-idle SECS` | 0 (1800) | an unused document view closes (four times as long with unsaved edits, then kept in the recovery journal) |
| `viewer.control.enabled`, `.existingSecret`, `.key` | `--viewer-control` | off | the UI control channel relayed to editors' app pages (docs/web-deployment.md, "UI control channel"), its token from the Secret as `BICRAFT_VIEWER_CONTROL_TOKEN`; the chart refuses it without the Secret or without the viewer |

**Browsers.** Put people behind the auth proxy (the trusted-header mode, `auth.trustedProxy`):
oauth2-proxy in front of the ingress logs them in and sets the identity headers on every request,
page and API calls alike. Without a proxy, an operator hands out a one-time URL:

```sh
curl -s -X POST -H "Authorization: Bearer $TOKEN" https://bicraft.example.com/w/main/view/login
# {"url":"https://bicraft.example.com/w/main/view/login/3f…","expires_in":60}  (open it within a minute, once)
```

Set `auth.publicUrl` to the URL browsers use: the viewer compares `Origin` with it.

**Through the controller's gateway** (section "The controller"). The gateway forwards
`/w/<id>/view/…` like every `/w/<id>/…` path: it authenticates each request itself, wakes the
workspace, and sends the identity and the access it granted in its `X-Bicraft-Gateway-*` headers,
which the pod accepts from the gateway's address only (the trusted-header mode). So in a controller
deployment browsers authenticate at the gateway: put the auth proxy (oauth2-proxy) in front of it,
or let it validate OIDC bearer tokens. The one-time login URL works on `bicraft-cli serve` directly, not
through the gateway (the redeeming request carries no credential the gateway accepts); the cookie's
`Path=/w/<id>/` would be the same through it. The gateway rewrites `Host` to the workspace Service,
so the pod's CSRF check accepts the public origin through its `--allow-origin` list: the controller
adds its `public_url` there for every workspace pod (`crates/controller/src/render.rs`). Private
views live in the pod: an idle scale-to-zero ends them (the page asks to be reloaded).

**Observability.** Viewer requests are counted under `route="view"` and logged like the others,
with `view` naming the route kind (`page`, `state`, `events`, `asset`, `interact`, `login`, `other`):
never the view id, the action's values, a file name or a user id.

| Family | Type | Meaning |
|---|---|---|
| `bicraft_viewer_copies_open`, `bicraft_viewer_copy_bytes` | gauge | read-only copies open, and the memory their loads reserved |
| `bicraft_viewer_copy_loads_total`, `bicraft_viewer_copy_load_failures_total`, `bicraft_viewer_copy_busy_total` | counter | copies loaded (each logged as `copy_load` with its duration and size), loads that failed, reads refused because every copy was in use |
| `bicraft_viewer_views_open`, `bicraft_viewer_logins_open` | gauge | private views and browser logins kept |
| `bicraft_viewer_pages_total`, `bicraft_viewer_interactions_total` | counter | pages served, interactions run |

## Validation

| Check | Where it runs | What it proves |
|---|---|---|
| `cargo xtask deploy check` (unit test in `cargo xtask ci`) | Windows, Linux, no tools needed | every YAML parses; the Grafana dashboard parses and queries only metrics the code exposes; the metrics port differs from the MCP port and is declared; every `BICRAFT_*` variable named in `deploy/` and in this page is read by the code; every `serve` flag used by the manifests, the chart and the image exists; every `.Values` reference exists; the security and sizing invariants; the Dockerfile's pins, final stage, user and entry point |
| `cargo xtask deploy smoke [--target T]` | Windows, Linux | the binary started with the plain manifest's arguments and variables (paths mapped to a scratch folder, listen on 127.0.0.1): probes, 401 without a token, 403 for a foreign `Host`, discovery, tool list, a generated PBIP project uploaded zipped, `open_file`, a DAX query over SSE, a sandbox refusal; `/metrics` scraped on its own port (draining 0, one open workspace, `dax.evaluate` and `file.open` series, MCP 200 and 401 counts, the manifest's memory limit, no query text or token) and 404 on the MCP port; then a half-sent upload, the shutdown (SIGTERM on Linux, `--drain-on-stdin-close` on Windows), 503 while draining, `bicraft_draining 1` while draining, the upload completed, exit 0, `TMPDIR` empty; every JSON log line parses and none carries the query, the fixture's data, a file name or the token |
| `cargo xtask deploy kind-smoke` | where docker, kind, helm and kubectl are (WSL Ubuntu on the development machine) | `helm lint --strict`, `helm template` (refusal without authentication, every option rendered), the image built and inspected, fixture and token made by the image, a kind cluster, `kubectl apply --dry-run=server -k deploy/manifests`, the chart installed in a `restricted` namespace, the same live checks through `kubectl port-forward`, scale to zero with a clean drain in the pod's log; the cluster, the image, dangling images and the whole BuildKit cache are always removed afterwards (use a docker host dedicated to builds, or `--keep`) |
| `cargo xtask deploy controller-smoke` | same tools (WSL Ubuntu) | `helm lint`/`template` of the controller chart (refusal without authentication, every option, a non-default CRD group), the image, four tokens and the fixture made by the image, a kind cluster, the chart in a `restricted` namespace; through a port-forward to the gateway: workspace created with the API (an agent may not), not listed for another identity, woken by its first MCP request, the live checks of `deploy smoke` through the gateway, a reader querying but refused an upload by the pod and refused a stop; a probe pod reaching the gateway but not the workspace pod (NetworkPolicy), even with a forged identity header; idle stop to zero, wake-up by the next request with the file still on the volume; a `kubectl apply`-declared workspace with `reclaimPolicy: Delete`; deletions (agent 403, admin 202), children collected, the retained volume kept and the other deleted; no panic in the controller log. On failure it prints pods, events, descriptions and logs before deleting the cluster; the cluster, the images, the probe image and the build cache are always removed |

Last runs (2026-10-27): `deploy smoke` passed on Windows (release build, drain via standard input)
and in WSL Ubuntu (the static musl binary, drain via SIGTERM), each in about 1 s after the build.
`deploy kind-smoke` passed in WSL Ubuntu with docker 29.1.3 + buildx, kind v0.29.0, helm v3.22.1,
kubectl v1.37.1 in 708 s, most of it the image build: image 10.8 MB as docker reports it (user
65532:65532, entry point `bicraft-cli serve`), the chart installed in a `restricted` namespace (pod limit
512 Mi = 448 + 64), every live check through the port-forward, and a clean drain (exit 0) on scale
to zero. Afterwards the cluster, the image and its 2.0 GB of build cache were removed (the
`kindest/node` image stays cached).

With observability (2026-10-28, `feat/metrics`): `deploy smoke` passed on Windows and in WSL Ubuntu (three
runs in a row), scraping `/metrics` on its own port (185 series, the manifest's 1984 MiB limit, 404 on
the MCP port, `bicraft_draining 1` during the drain) and checking the 17-line JSON log; `deploy
kind-smoke` passed in WSL in 808 s with the chart's metrics on the pod IP and the JSON log in the pod,
clean drain on scale to zero; cluster, image and build cache removed afterwards.

`deploy controller-smoke` passed in WSL Ubuntu (2026-10-28; docker 29, kind 0.29 whose default CNI, kindnet,
enforces NetworkPolicy, so no other CNI is needed) in 644 s, most of it the image build: wake-up on
the first request 7.7 s (pod scheduled, volume provisioned and bound, `/readyz` 200), idle stop 21 s
after the last request with a 20 s timeout and a 2 s reconciliation period, second wake-up 3.2 s with
the project still on the volume and the DAX query answering 42. An earlier run lost its
`kubectl port-forward` during the first wake-up (`lost connection to pod`); it did not happen again,
and the smoke test now prints the cluster's state when a step fails.

With the viewer (2026-10-28, `feat/viewer`): `deploy smoke` passed on Windows (release build) and in
WSL Ubuntu (musl), now with the viewer checks (`client::viewer_checks`, a reader token and grant added
by the smoke): the page and `api/state` as a reader and as the editor (card 42, the reader served by
the clean editor session), a reader's slicer giving 10 on that view while the reader's second page and
the editor's stay at 42, a foreign `Origin` refused, the one-time login URL redeemed once for a cookie
that loads the page and its data. `deploy kind-smoke` passed in WSL in 621 s with the same checks
through the port-forward (reader grant in the chart values); `deploy controller-smoke` passed in 668 s
with the viewer checks through the gateway (no one-time login there). Clusters and images removed
afterwards.

`kind-smoke` fails with the list of missing tools when one is not on PATH. It compiles the
workspace inside the image build: take a build slot on shared machines.

## Not done yet

- Controller (done, section "The controller"): leader election and stateless gateway replicas, watches
  instead of polling, the controller's own `/metrics` (stage 3).
- Tracing (OpenTelemetry spans per tool call); alerting rules shipped with the chart.
- Per-identity rate limits (stage 2); a private CA for OIDC issuers.
- Signed images, SBOM, image scanning, allocator measurements in the container (stage 3).
- No image is published: building and pushing to a registry is the operator's decision.
