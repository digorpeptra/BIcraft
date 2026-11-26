# Serving BIcraft on the web from Kubernetes

Status: plan, 2026-10-25, written against `main` 62998b0. Implemented since (2026-10-26): the path
sandbox and command policy (`docs/server-mode.md`), the shared `std` HTTP crate, MCP over
Streamable HTTP at `2026-07-28`, the cancel flag, `bicraft serve` with the sandbox wired in, and
authentication with a workspace ACL, which lets `bicraft serve` leave loopback (`docs/auth.md`,
branch `feat/auth`), and the operations lot: file upload and download per workspace, the probes,
the graceful drain on SIGTERM and the idle close (section "Operating `bicraft serve`", branch
`feat/serveops`), and the Kubernetes packaging (2026-10-27, branch `feat/deploy`): the image, the Helm
chart, plain manifests and their checks (`docs/kubernetes.md`). Stage 1 is complete except the
viewer (moved to stage 2). Stage 2 so far: reader access (2026-10-27, branch `feat/roles`, `docs/auth.md`
"Roles"): reader grants, read-only commands declared by each engine command, refusals over MCP
HTTP and on the file routes; observability (2026-10-27, branch `feat/metrics`, `docs/kubernetes.md`
"Observability"): JSON request logs, the Prometheus `/metrics` listener, Helm and manifest wiring and
a Grafana dashboard; the workspace controller (2026-10-28, branch `feat/controller`,
section "Workspace controller (stage 2)", `docs/kubernetes.md` "The controller"); the viewer's server mode (2026-10-28, branch `feat/viewer`, `docs/kubernetes.md`
"Viewer", `docs/auth.md` "The viewer in a browser"): `/w/<id>/view/` with the MCP endpoint's
authentication and ACL, private views per browser page, readers on saved files (shared read-only
copies, private `file.open`), browser login by trusted header or one-time URL. The rest is plan.

## Résumé (français)

**Oui, BIcraft se porte bien dans Kubernetes.** Le binaire est un exécutable Linux statique (musl,
~22 Mo, aucune dépendance Windows à l'exécution) : image minuscule, sans root, système de fichiers
en lecture seule. État au 2026-10-26 (étape 1, réservée aux agents) :

**Fait**

1. **Bac à sable de chemins et politique de commandes** (`docs/server-mode.md`) : en mode serveur,
   chaque chemin passé par un outil est résolu sous le dossier de l'espace de travail (chemins
   absolus, `..`, liens qui sortent : refusés) ; `file.open_in_desktop` est interdit et le
   connecteur Databricks n'a le réseau que si l'opérateur l'autorise.
2. **MCP sur HTTP** (Streamable HTTP, révision `2026-07-28`, la plus récente, sans sessions de
   protocole) : `bicraft serve --root DIR`, un espace de travail par sous-dossier, à l'URL
   `/w/<espace>/mcp` ; une session moteur par espace, appels sérialisés avec file bornée, espaces
   en parallèle, annulation quand le client ferme le flux.
3. **Authentification** (`docs/auth.md`), trois modes combinables : jetons JWT OIDC / OAuth 2.1
   selon la spécification d'autorisation MCP (métadonnées RFC 9728, en-tête `WWW-Authenticate`,
   découverte et JWKS avec rotation des clés, audience liée au serveur ; Entra ID, Google, Okta,
   Auth0, Keycloak…), en-tête d'identité posé par un proxy d'authentification de confiance
   (oauth2-proxy, Pomerium, Authelia…), jetons d'API statiques stockés hachés pour les agents et
   la CI. Une ACL associe identités, groupes et rôles aux espaces de travail ; un espace interdit
   répond comme un espace inexistant. Sans authentification, le serveur reste limité au loopback ;
   avec, il peut écouter sur le réseau (derrière un ingress TLS).
4. **Exploitation** (section « Operating `bicraft serve` ») : téléversement et téléchargement des
   fichiers par espace (`/w/<espace>/files/`, mêmes authentification et ACL, écrits en flux puis
   renommés atomiquement, jamais de lien créé, pas d'écrasement sans `Overwrite: T`), projet PBIP
   zippé extrait sans risque (zip-slip, liens, bombes refusés) ; sondes `/healthz` et `/readyz`
   sans authentification ni donnée ; arrêt propre sur SIGTERM / Ctrl+C (503 pour les nouvelles
   requêtes, délai de grâce, requêtes DAX longues annulées, **jamais d'enregistrement implicite**,
   code de sortie 0 ou 3) ; fermeture des espaces inactifs, rouverts à la demande.

5. **Kubernetes** (`docs/kubernetes.md`) : image `FROM scratch` (binaire musl statique, UID 65532),
   chart Helm générique (un espace de travail par release, PVC, NetworkPolicy, sondes, sécurité
   « restricted », mémoire du pod = `BICRAFT_MEMORY_LIMIT` + 64 Mio), manifestes simples, contrôles
   `cargo xtask deploy check|smoke|kind-smoke`.

**Reste à faire** :
mode serveur du visualiseur, contrôleur d'espaces, rôle lecteur, métriques (étape 2) ;
durcissement production (étape 3). Topologie recommandée : un pod par modèle (5,4 Go à 100 M
lignes, 6,9 Go avec agrégations ; ouverture 0,44 s une fois le fichier sur le volume du pod).

**Gouvernance des données** : les fichiers clients ne quittent jamais le contrôle de leur
propriétaire sans son accord explicite. Chaque opérateur choisit et contrôle son cluster ; rien
n'est déployé, publié ni envoyé ailleurs sans validation.

## Decisions (user, 2026-10-26)

These answer the open questions at the end of this note. Where an earlier section says otherwise,
this section wins.

1. **Where it runs: both** the user's own clusters and clients' clusters. **BIcraft is open
   source**, so the deployment must be generic: a Helm chart and plain manifests with every value
   configurable, nothing tied to one cloud. The operator of each cluster owns its data and approves
   its environment; the governance rules below are written for any operator.
2. **Storage: one `PersistentVolumeClaim` per workspace** (`ReadWriteOnce`). No object-storage
   backend in stages 1–2.
3. **Authentication: compatible with as many providers as possible.** Three modes, all standard:
   - **OIDC / OAuth 2.1 resource server** (the MCP authorization spec): BIcraft validates bearer
     JWTs from any OIDC issuer by discovery and JWKS (Entra ID, Google, Okta, Auth0, Keycloak,
     Authentik, Zitadel, Dex…); issuer, audience and claim-to-role mapping are configuration.
   - **Trusted identity header** behind an auth proxy (oauth2-proxy, Pomerium, Authelia, Traefik
     forward-auth, a service mesh). This also covers SAML and LDAP through the proxy or Keycloak.
   - **Static API tokens** for agents and CI, hashed in a Kubernetes `Secret`.
4. **Agents first, humans second.** Stage 1 is MCP over HTTP only. The viewer's server mode moves
   to stage 2.
5. **Model sizes are an admin setting.** Per workspace (with a deployment default): the pod's
   memory and CPU requests and limits, `BICRAFT_MEMORY_LIMIT` (since lot 5, `docs/beyond-ram.md`:
   **pod memory limit = `BICRAFT_MEMORY_LIMIT` + 64 MiB**), `BICRAFT_MEMORY_BUDGET`, aggregations
   on/off, and the volume size. The process-wide cap the plan listed for stage 3 already exists.
6. **Databricks refresh: allowed.** The `NetworkPolicy` allows egress to the configured Databricks
   hosts only. The token is a per-workspace `Secret`.
7. **MCP protocol: the most recent revision only** (`2026-07-28` today) over HTTP. There is no
   legacy `Mcp-Session-Id` era on the HTTP transport. The stdio transport keeps its current
   versions.

Revised stage 1 (agents only): path sandbox and command policy; `std` HTTP module; MCP Streamable
HTTP at the latest revision with the workspace in the URL; the three auth modes; cancel flag;
`bicraft serve` with upload, download and probes; Dockerfile and Helm chart; a kind/k3d smoke test.
Stage 2 adds the viewer's server mode, the workspace controller, ACL and metrics.

## Data governance first

BIcraft's test data includes client material (`AGENTS.md`, "Local-only test data"), and the
models a web deployment would serve are client models. Rules for any deployment built from this
plan:

- **The user picks and controls the cluster.** No deployment to a shared or third-party cluster,
  no image push to a public registry, no upload of a model to object storage, without the user's
  explicit approval per environment.
- **No data egress by default.** The server makes no outbound connection except the Databricks
  refresh (`refresh_model` / `model.refresh`, `crates/databricks`), which already needs a token
  (`BICRAFT_DATABRICKS_TOKEN`) and should be disabled by a server flag unless enabled per
  deployment. Kubernetes `NetworkPolicy` should deny egress by default.
- **Logs never carry model data.** Command names, durations, sizes and error kinds only: no DAX
  results, no table rows, no file names of client files at info level.
- **Deletion is real.** A closed workspace's files (PBIX, spill files) are removed from the pod
  volume; retention of uploaded models in storage is a user decision (open question 1).

## What exists today

| Part | Code | Network posture today |
|---|---|---|
| MCP server | `crates/mcp` (L6), `bicraft mcp` | stdio only: newline-delimited JSON-RPC, hand-written, no async runtime. Revisions `2025-06-18`, `2025-03-26`, `2024-11-05` (`PROTOCOL_VERSION`, `SUPPORTED_VERSIONS`). One `Server` owns one `Session`; messages are handled strictly in order. `notifications/cancelled` is accepted and ignored. Lines capped at 64 MiB (`MAX_LINE_BYTES`). |
| Live viewer | `crates/view` (L6), `bicraft view PATH`, `bicraft mcp --view` | plain `std` HTTP/1.1, `TcpListener`, one capped thread per connection. `start_with` **refuses non-loopback addresses**; every request needs a localhost `Host` (`host_allowed`, DNS-rebinding guard). POST routes need a per-run random token (`X-Bicraft-Token`) plus same-origin `Origin`/`Sec-Fetch-Site`. Bodies ≤ 64 KiB, read deadlines, write timeouts, connection caps, SSE keepalive every 15 s. No user authentication. |
| Engine session | `crates/engine` `Session` | one source, one model, one report, undo history, `data_root`, `cache_abf`, display settings. Commands run through `Session::execute`, inside `catch_unwind`. Nothing global stops several `Session`s per process; the memory budget is **per session** (`preview_memory.rs`), the scan thread pool is per process. |
| Settings | `crates/env` | every `BICRAFT_*` variable read through one place. Server-relevant: `BICRAFT_MEMORY_BUDGET`, `BICRAFT_SPILL_DIR`, `BICRAFT_SCAN_THREADS`, `BICRAFT_AGG_THREADS`, `BICRAFT_AGGREGATIONS`, `BICRAFT_AGG_SYNC`, `BICRAFT_DATABRICKS_TOKEN`. |
| Release build | `cargo xtask dist`, `docs/releasing.md` | `x86_64-unknown-linux-musl`, static-pie, 22.0 MB binary, 8.1 MB archive; GitHub workflow also builds Linux arm64. TLS client only (rustls, Databricks). |

### Memory and timing figures to size pods

From `docs/benchmarks.md` (100 M-row star, 31 GB laptop) and `docs/beyond-ram.md`:

| Fact rows | `file.open` | Peak working set |
|---|---|---|
| 1 M | 28 ms | 0.11 GB ¹ |
| 10 M | 68 ms | 0.84 GB ¹ |
| 100 M | 0.44 s | 5.40 GB, **6.90 GB with report-derived aggregations** |

¹ From the earlier tier table in `docs/benchmarks.md`; the other figures are from "Milestone 2"
(2026-10-25). The honest start at 100 M (process + `initialize` + `file.open` + one query) is
0.49–0.85 s for most queries.

- Since milestone 2 the first query no longer pays a separate decompression step (column reads
  are parallel). Aggregations are built by a **background thread** per session (`preview_eval_aggs.rs`, `BICRAFT_AGG_THREADS`, a quarter of
  the threads by default) after the first report queries; `BICRAFT_AGG_SYNC=1` builds them in the
  foreground.
- Under `BICRAFT_MEMORY_BUDGET` (or `dax.memory {budget}`), decoded columns are evicted LRU, the
  decompressed backup moves to a temp file in `BICRAFT_SPILL_DIR`, caches are capped. **Not
  counted** (`beyond-ram.md`): the package bytes themselves (about 0.9 GB at 100 M) and tables
  loaded from partitions. A pod's memory limit must therefore be *budget + package + headroom*,
  not the budget alone.
- `dax.evaluate` / `evaluate_dax_query` are bounded by `timeout_ms` (default 30 s, max 600 s) and
  a cap on row visits (`dax_query.rs`); the evaluator checks the deadline in its work counter
  (`preview_eval.rs`). There is no external cancellation flag yet.

## MCP over HTTP

### The spec moved: sessions are gone

The current MCP revision is **`2026-07-28`**. Verified on 2026-10-26 against
modelcontextprotocol.io (`/specification/latest` resolves to `2026-07-28`): pages "Transports",
"Streamable HTTP", "Versioning and Compatibility", "Overview" (`_meta`, error codes),
"Discovery", "Cancellation" and the changelog. Corrections to the first draft of this plan are
marked **(corrected)**.

The revision is **stateless**:

- **(corrected) There is no `initialize` / `notifications/initialized` handshake at all**, not just
  no session. Every request carries in `params._meta`:
  `io.modelcontextprotocol/protocolVersion` (string, **required**),
  `io.modelcontextprotocol/clientCapabilities` (object, **required**),
  `io.modelcontextprotocol/clientInfo` (optional, SHOULD), `io.modelcontextprotocol/logLevel`
  (optional). A request missing a required field is malformed: JSON-RPC `-32602`, HTTP `400`.
- **(corrected) `server/discover` is mandatory** for servers: result `supportedVersions`,
  `capabilities`, `instructions`, `_meta["io.modelcontextprotocol/serverInfo"]`. Clients may skip
  it and send any request directly.
- Version negotiation is per request: an unsupported version gets HTTP `400` with
  `UnsupportedProtocolVersion` **`-32022`**, `data: {supported: [...], requested}`. A modern-only
  server SHOULD name its versions in any error it returns to a legacy `initialize`.
- **(corrected)** Every result carries `resultType` (`"complete"`, or `"input_required"` for
  multi round-trip requests); `tools/list` (and the other list/read results) carry the cache hints
  `ttlMs` and `cacheScope` (`"public"` / `"private"`). Results SHOULD carry
  `_meta["io.modelcontextprotocol/serverInfo"]`.
- `ping`, `logging/setLevel` and `notifications/roots/list_changed` are removed; server-initiated
  requests are replaced by `InputRequiredResult` (MRTR); change notifications move to a
  `subscriptions/listen` request whose response stream stays open.
- Error codes: `-32000`..`-32019` legacy (new implementations SHOULD NOT use them),
  `-32020`..`-32099` reserved for the spec (`-32020` `HeaderMismatch`, `-32021`
  `MissingRequiredClientCapability`, `-32022` `UnsupportedProtocolVersion`). Application codes
  SHOULD be outside `-32768`..`-32000`.

Its Streamable HTTP binding (all confirmed):

- one endpoint, **POST only**; each JSON-RPC request or notification is its own POST; the client
  MUST send `Accept` listing `application/json` and `text/event-stream`; the reply to a request is
  `application/json` or an SSE stream scoped to that request (request-related notifications, then
  the response, which SHOULD end the stream; servers SHOULD send `X-Accel-Buffering: no`; no event
  ids, no `Last-Event-ID` resumption); an accepted notification gets `202` with no body;
- no protocol session: a server for this revision only SHOULD answer GET and DELETE with `405`,
  ignore `Mcp-Session-Id` (never mint or echo one) and ignore `Last-Event-ID`;
- mirrored headers, REQUIRED: `MCP-Protocol-Version` (must equal the `_meta` version), `Mcp-Method`
  (all requests), `Mcp-Name` (`params.name` for `tools/call` and `prompts/get`, `params.uri` for
  `resources/read`); values outside visible ASCII travel as `=?base64?…?=` and are decoded before
  comparing; a tool parameter can be mirrored as `Mcp-Param-{Name}` through `x-mcp-header` in its
  schema (optional for servers). Missing, malformed or mismatching: HTTP `400`, JSON-RPC
  **`-32020`**. Header requirements for notification POSTs are not defined by the revision;
- unknown method: HTTP **`404`** with `-32601` (the body tells it apart from a legacy server's 404);
- **cancellation = the client closes the request's response stream**; the server SHOULD stop work
  and MUST NOT send anything more for it (`notifications/cancelled` is stdio only);
- the server MUST validate `Origin` (`403` when present and invalid, body MAY be a JSON-RPC error
  without `id`), SHOULD bind localhost when local, SHOULD authenticate.

Revisions `2025-03-26` to `2025-11-25` used `Mcp-Session-Id` (minted at `initialize`, ended by
`DELETE`). Decision 7 above: BIcraft serves **only `2026-07-28` over HTTP**; stdio keeps
`2025-06-18`, `2025-03-26`, `2024-11-05`. The MCP authorization spec (OAuth 2.1
resource server) was verified with the authentication lot: `docs/auth.md`.

### Consequence: BIcraft carries the workspace identity

BIcraft is stateful by nature: an open model, its report, its undo history, warm decoded columns
and aggregations. With no protocol session, the workspace must be named outside it. Proposal:

- **The URL names the workspace**: `https://host/w/<workspace-id>/mcp`. The ingress routes by path
  to the pod holding it (see "Topology"); the pod maps the id to its `Session`. It needs nothing
  from the client beyond the URL. (The spec's own advice for cross-call state is explicit handles
  passed as tool arguments; the URL keeps the tools identical instead.)
- Rejected alternative: a `workspace` parameter on every tool with `x-mcp-header`. It would change
  all 27 tool schemas and every agent prompt; the URL keeps the tools identical to stdio.

### Server mechanics

- **Keep the "no async runtime" style.** `crates/view` already shows a hardened `std` HTTP/1.1
  server (bounded heads, bodies, deadlines, connection caps). The HTTP transport reuses that code,
  moved into a shared crate rather than adding tokio/hyper; std keeps the binary small and the
  never-crash audit simple. **(corrected)** The crate cannot be L6: its users `bicraft-view` and
  `bicraft-mcp` are L6 and may only depend on strictly lower layers, so `bicraft-http` is **L0**
  (it depends on no workspace crate).
- **Concurrency.** Stage 1 serialized the calls of one workspace behind a mutex; different
  workspaces ran in parallel. **(done, `feat/concurrency`)** Read-only calls of one workspace now
  run side by side under a read lock and edits run alone, with optimistic concurrency for editors:
  see [Concurrency on one workspace](#concurrency-on-one-workspace).
- **Cancellation.** Add a per-request cancel flag (`Arc<AtomicBool>`) passed into `Session` and
  checked where the evaluator already checks `max_time`, in the render loop and in long saves. A
  closed response stream (2026-07-28) or `notifications/cancelled` (legacy, stdio) sets it.
  Lot 2 wires the evaluator only (render and save loops: later).
- **Progress.** Long commands (open of a large PBIX, structural save, render) can stream
  `notifications/progress` on the SSE response; optional, stage 2.
- **Never crash.** The HTTP layer inherits the viewer's limits: request line and header caps,
  header count, body cap (the stdio 64 MiB line cap is too large for a network endpoint: 8 MiB
  default, configurable), read deadline, idle-connection timeout, per-workspace queue length (503
  beyond), per-IP connection cap. Every rejection path gets a regression test, as in `crates/view`.

### MCP over HTTP: what is implemented

Stage 1, lot 2 (2026-10-26, branch `feat/mcphttp`):

- **`crates/http` (`bicraft-http`, L0)**: the viewer's plumbing, moved without behaviour change
  (the viewer's tests are unchanged and green): bounded head (`Limits`: bytes, header count,
  target length, deadline), headers declared unique refused when repeated, `Content-Length` only
  (`Transfer-Encoding` → 411), `read_body` with its own deadline, bounded drain on close,
  loopback `Host` check, random tokens, a non-blocking accept loop with a global and a per-address
  connection cap (`Connections`, 503 `busy` beyond), SSE helpers, `peer_closed` (peek without
  consuming).
- **`crates/mcp/src/http.rs` (`HttpServer`)**: `/w/<workspace-id>/mcp`, the id being a subfolder
  of the server root (1–64 of `[A-Za-z0-9._-]`, not starting with `.`; anything else 404). Checks,
  in order: `Host` (loopback names, or a configured list; 400 missing / 403 other), route (404),
  method (`POST`; anything else 405 `Allow: POST`; `Mcp-Session-Id` and `Last-Event-ID` are
  ignored), `Origin` against `allowed_origins` (403 with an id-less JSON-RPC error; agents send
  none), workspace folder (404, code `-31002`), `Content-Type: application/json` (415),
  `Content-Length` (411) ≤ `max_body_bytes` (8 MiB default, 413, refused before reading), `Accept`
  (406 when it lists neither JSON nor SSE), body within `body_deadline` (60 s), JSON parse (400
  `-32700`), one object (batches and other values 400 `-32600`; client responses 400). A
  notification gets 202 (a mirrored `Mcp-Method`, when sent, must agree). A request: `id` string
  or number, `params` an object (400 `-32602`); `initialize` → 400 `-32022` naming the served
  versions; mirrored headers (400 `-32020`, base64 sentinel decoded); `_meta` required fields (400
  `-32602`); header version = body version (`-32020`); version served (400 `-32022` with
  `{supported, requested}`). Methods: `server/discover`, `tools/list` (`ttlMs` 3 600 000,
  `cacheScope: "private"`), `tools/call`; others 404 `-32601`. Results carry `resultType:
  "complete"` and `_meta.serverInfo`. The tools are the stdio ones (`tool_definitions`,
  `call_tool`): no schema change, no `x-mcp-header`.
- **Replies**: `tools/call` answers with an SSE stream when the client accepts
  `text/event-stream` (head sent once the call holds its queue place, `:` comment keepalives every
  15 s, the response as one `data:` event, then close), plain JSON otherwise; discovery and the
  tool list are always JSON. Everything says `Connection: close` (one request per connection).
- **Workspaces**: one `Session` per workspace, created on first `tools/call` (at most 64 open:
  503), calls serialized behind its mutex (read-only calls side by side since `feat/concurrency`)
  with a queue of 8 running + waiting (503, code `-31001`, `Retry-After: 1`); different workspaces run in parallel, discovery and the tool list
  never wait. Application codes are outside the JSON-RPC reserved range, as the spec asks.
- **Cancellation**: while a call waits or runs, a watcher polls the socket (`peer_closed` every
  50 ms; SSE keepalive write failures too) and raises the request's `Arc<AtomicBool>`, which the
  engine holds during the call (`Session::set_cancel`); the evaluator checks it in its work
  counter next to the deadline and fails with `cancelled by the client`. Nothing more is written
  for a cancelled request. Limits: a client that half-closes its sending side after the request
  is treated as gone; only DAX evaluation stops early (render, save and open run to completion).
- **`bicraft serve --root DIR [--listen 127.0.0.1:7878] [--allow-network] [--allow-origin O]...
  [--max-body-bytes N] [--queue N] [auth flags]`**: binds loopback (127.0.0.1:7878 by default);
  a non-loopback `--listen` needs an authentication mode and `--public-url` or `--allow-host`
  (since the authentication lot, `docs/auth.md`; `HttpServer::start` itself refuses a
  non-loopback address without `HttpConfig::auth` or under `HostPolicy::Loopback`). Every workspace session is in server mode (wired 2026-10-26, see
  below), so no extra flag is needed for the sandbox: the lot-2 `--insecure-local-only` flag is
  gone (an unknown argument now). Network access (the Databricks connector of `model.refresh`)
  is off unless `--allow-network` or `BICRAFT_SERVER_NETWORK=1`; an invalid
  `BICRAFT_SERVER_NETWORK` value refuses to start.
- **Sandbox wiring** (2026-10-26, branch `feat/wire`): `bicraft_mcp::http::new_session(dir,
  allow_network)` builds `Session::server(ServerConfig::new(dir)?.allow_network(..))` with the
  workspace folder as root (`HttpConfig::allow_network`, default off). A workspace folder must be
  the root's real direct child named exactly as the id (canonicalized; a link pointing at another
  workspace or outside the root, or another case spelling of an id, is 404). Over HTTP, `list_commands` and `tools/list`
  (`server_tool_definitions`) hide commands denied in server mode (`file.open_in_desktop`);
  calling one through `run_command` or `batch` returns the policy error. Tests in
  `crates/mcp/tests/http.rs`: absolute paths, `..`, link escapes, render `out`, batch, two
  workspaces isolated, workspace alias links, denied command hidden and refused, Databricks
  refresh with the network setting off (refused) and on. stdio and the CLI keep local sessions.
- **Tests** (Linux-first, all over real sockets): `crates/mcp/tests/http.rs` (discovery, list and
  call over JSON and SSE, 202, 405/404, every header mismatch, `_meta`, unsupported version,
  `initialize`, Origin 403, Host 400/403, 413/415/411/406, malformed bodies, mutated requests,
  queue full 503, cancellation of a minutes-long DAX query by closing the stream (JSON and SSE),
  two workspaces in parallel and one workspace in order), `crates/http/src/tests.rs` (parser fuzz
  over 20 000 mutated heads, caps, bodies, closed peers, accept loop), engine
  `a_raised_cancel_flag_stops_the_evaluation`, CLI `tests/serve.rs`.

- **Authentication** (2026-10-26, branch `feat/auth`, `docs/auth.md`): new L0 crate
  `bicraft-auth` (OIDC bearer JWTs with discovery, cached JWKS and key rotation; trusted identity
  header from configured proxy ranges; SHA-256-hashed API tokens; the workspace ACL). The endpoint
  authenticates every request after the `Host` check and the route, before any workspace lookup:
  401 / 403 `insufficient_scope` / 400 with `WWW-Authenticate` (`resource_metadata`, `scope`),
  a closed workspace answers 404 like a missing one, and
  `/.well-known/oauth-protected-resource[/w/<id>/mcp]` serves the RFC 9728 metadata.

Left for the next lots: `notifications/progress` on the SSE stream; cancellation of render and save.
Upload/download, the probes, the SIGTERM drain and the idle close: "Operating `bicraft serve`" below.

## Operating `bicraft serve`

Stage 1, operations lot (2026-10-26, branch `feat/serveops`). Code: `crates/mcp/src/http/files.rs`
(files), `crates/mcp/src/http/ops.rs` (probes, drain, idle close), `crates/opc/src/extract.rs`
(archive walker), `apps/bicraft-cli/src/serve_cmd.rs` (flags, signals). Tests:
`crates/mcp/tests/http_ops.rs`, `apps/bicraft-cli/tests/serve.rs`.

### Files

Each workspace's folder is reachable at `/w/<id>/files/`, with the same `Host` check,
authentication, ACL (a closed workspace is 404 like a missing one), `Origin` allow-list and
connection caps as the MCP endpoint; a token for the workspace's MCP endpoint serves its files too.
Writing needs editor access (the only level stage 1 grants).

| Request | Answer |
|---|---|
| `GET /w/<id>/files/` (or a folder path) | 200 JSON `{path, entries: [{path, type, size, modified}], truncated}`: `type` is `file`, `dir`, `link` or `other`; recursive, sorted, at most 10 000 entries and 32 levels; links are listed, never followed; `.bicraft-tmp/` is hidden |
| `GET` / `HEAD /w/<id>/files/<file>` | 200, streamed in 64 KiB chunks; `Content-Type` by extension (`application/json` for `.json`/`.bim`/`.pbir`/`.pbism`/`.pbip`/`.platform`, text for `.tmdl`/`.csv`/`.m`/`.dax`, opaque bytes for `.pbix`/`.pbit`/`.abf`), `Content-Length`, `Content-Disposition: attachment` with an ASCII and an RFC 8187 UTF-8 name, `Content-Security-Policy: sandbox`, `Cache-Control: no-store` |
| `PUT /w/<id>/files/<file>` | 201 `{path, size, replaced: false, session_closed}`, or 200 when replacing |
| `POST /w/<id>/files/[<folder>]` with `Content-Type: application/zip` | 201 `{path, files, dirs, bytes, replaced, session_closed}`: the archive extracted into the folder (created when missing) |
| `DELETE /w/<id>/files/<path>` | 204; a non-empty folder needs `Depth: infinity` (409 otherwise); the workspace folder itself 405 |

Rules:

- **Paths.** The URL path is percent-decoded (a broken escape, invalid UTF-8 or a control
  character is 400) and resolved by the engine's sandbox (`docs/server-mode.md`): absolute, drive,
  UNC/device, `..`, NUL (and on Windows streams and reserved names) 400; through a link leaving the
  workspace 403. The folder `.bicraft-tmp/` at the workspace root is reserved (404). A link in
  the last place of a write or a deletion is the entry itself: a deletion removes the link, an
  upload replaces it with a plain file; nothing is ever written or deleted through a link that
  leaves the workspace.
- **Streaming and atomicity.** `Content-Length` is required (411; no chunked bodies) and at most
  `--max-upload-bytes` (default 4 GiB, 413 from the head, before any byte is read). The body is
  copied chunk by chunk to `<workspace>/.bicraft-tmp/upload-<random>.part` (`create_new`), with a
  stall limit (30 s) and a deadline (2 h), synced, then renamed over the target (same volume): a
  reader sees the old file or the new one. A body cut short, a full volume (507) or a stopped
  server leaves no file behind (the temp file is removed; leftovers of a crash are removed at the
  next start and at shutdown).
- **No overwrite by default.** An existing target is 412 unless the request says
  `Overwrite: T` (WebDAV, RFC 4918 §10.6; `F` or absent: refuse); a folder in the way is 409. The
  check runs before the body is read and again at the final step.
- **Plain files only.** Files are created with `create_new`, folders with `create_dir_all`; no
  link of any kind is created.
- **Zipped projects.** A PBIP project is a folder tree; it can be uploaded file by file with `PUT`
  (parents are created) or as one archive with `POST` + `application/zip` (`PUT` never extracts,
  so a `.pbix`, itself a ZIP, is always stored as is). The archive is streamed to a temp file,
  then walked by `bicraft_opc::for_each_zip_entry`: at most 50 000 entries, plain files and
  folders only (symbolic links, devices, FIFOs and encrypted entries refused), names relative
  without `..`, root, drive, `:` or empty components (zip-slip), no two names equal ignoring ASCII
  case, decompressed bytes counted on the real stream (4 GiB per entry, 8 GiB in total, and an
  entry past 1 MiB may not expand more than 1000 times its compressed size: ZIP bombs stop early),
  CRC-32 checked. Entries are written to a staging folder under `.bicraft-tmp/` (each name
  resolved again by the sandbox), and only when the whole archive passed are the files moved into
  place. A refused archive (400, or 413 past a cap) writes nothing outside the staging folder,
  which is removed. The move itself is file by file (each file atomic, the set not).
- **Uploads and the open session.** The final step of an upload, an extraction or a deletion runs
  under one server-wide lock and the workspace's session lock, so it never interleaves with a
  tool call (it waits up to 30 s for a running call, then 503 `Retry-After: 5`). When it would
  replace or remove a file the session has open (the PBIX, `model.bim`, the `.pbip` file or a file
  inside the open project's item folders, or a folder holding one): **409 if the session holds
  unsaved changes** (save them, or reopen the file to discard them); otherwise the session is
  closed (`session_closed: true`) and the next tool call reopens the file, now the new bytes (a
  deleted file is not reopened). Other files of the workspace can be written while a model is
  open. Downloads do not take the lock: they stream the file as it is on disk (a session's
  unsaved changes are not in it).

### Probes

`GET /healthz` and `GET /readyz` (also `HEAD`; other methods 405) are answered before the `Host`
check (the kubelet sends the pod address) and without authentication. The body is `ok`, or the
reason of a 503 (`draining`, `memory`, `stopping`): never a workspace name or count.

- `/healthz` (liveness): 200 while the accept loop dispatches connections. Under the connection
  cap the accept thread answers 503 `busy` to every new connection, probes included: give the
  liveness probe a `failureThreshold` that tolerates short bursts.
- `/readyz` (readiness): 503 from the moment a drain starts; with `--ready-memory-percent P`, also
  while BIcraft's reserved memory is above P % of `BICRAFT_MEMORY_LIMIT` (no limit: never).

### Graceful shutdown

`bicraft serve` installs a handler with the `ctrlc` crate (3.5, MIT OR Apache-2.0; its signal and
console calls are the crate's own, BIcraft's crates keep `unsafe_code = "forbid"`): SIGTERM,
SIGINT and SIGHUP on Unix, Ctrl+C and Ctrl+Break on Windows. The first signal starts
`HttpServer::drain(grace)`; a second one exits at once (code 130). On Windows, closing the console
window ends the process as soon as the handler returns: there is no time to drain (Windows is a
development platform here; deployments are Linux).

The drain, in order:

1. `/readyz` answers 503 and every new request 503 with `Retry-After: 5` (a JSON-RPC error
   `-31004` on the MCP endpoint, `{"error": …}` on the file routes). The listener stays open
   during the grace period so late requests get a clear 503 rather than a refused connection.
2. Running requests (tool calls, uploads, downloads) get `--shutdown-grace` seconds (default 30;
   set `terminationGracePeriodSeconds` a few seconds above it).
3. Past the grace period, every running tool call's cancel flag is raised (the engine stops a DAX
   evaluation at its next work check; render, save and open run to completion) and uploads and
   downloads stop between chunks; the drain waits up to 5 s more.
4. The listener closes; every workspace session is dropped **without saving**: a session holding
   unsaved changes is reported by name (workspace ids are the operator's folder names), never
   written. Dropping a session deletes its spill files. The upload temp folders are removed.
5. Exit code **0** when every request finished in time and nothing unsaved was discarded, **3**
   otherwise; one line on stderr says what happened.

### Idle close

With `--idle-timeout SECS` (off by default; in the one-pod-per-model topology the pod itself is
what scales to zero), a workspace session unused for that long (no tool call running or waiting)
is dropped, which frees its decoded columns, caches and aggregations. The file it had open is
remembered (path, `data_root`, `cache_abf` relative to the workspace, UI language, culture, as-of
date), and the next tool call reopens it before running (`file.open`; aggregations rebuild in the
background). The viewer's interaction state and the undo history are not kept. **A session with
unsaved changes is never closed for idleness**: it stays open, the server logs it once, and it is
closed without saving at shutdown. If the remembered file was deleted meanwhile, the next session
starts empty and the call reports what it finds.

### Concurrency on one workspace

Added with `feat/concurrency` (2026-10-28). Before, every call of a workspace waited for the one
before it, so a 10 s DAX query blocked everyone, readers on the clean editor session included.

**Reads run side by side, edits alone.** The workspace's session sits behind a read-write lock
(`crates/mcp/src/http/rwgate.rs`):

- A call is a **read** when it runs one command whose server policy is read-only
  (`docs/auth.md`, "Roles": `Effect::ReadOnly`) and names none of the parameters that make it
  write (`report.render {out}`, `dax.memory {budget}`, `dax.aggregations {enabled}`, …) or swap
  session fields for its duration (`cache_abf`); `list_commands` too. Viewer reads (snapshots,
  interactions on private views) and readers' calls on the clean editor session or on a read-only
  copy are reads. Everything else (`batch`, every editing command, `file.open`, a pending lazy
  reopen) is a **write**.
- Up to `--max-concurrent-reads` reads (default 4; Helm `server.maxConcurrentReads`) run at once
  on one session; more wait in the queue (`--queue`, 8 running + waiting, as before).
- A write waits for the running reads and then runs alone. **Writers do not starve**: a waiting
  write stops new reads from entering (writer-preferring gate, not the platform's lock, whose
  fairness is unspecified); the reads behind it see its change.
- Local `bicraft mcp` (stdio) and the CLI are unchanged: one call at a time, `Session::execute`.

**In the engine.** Read-only commands have a second entry point, `Session::execute_shared(&self)`
(`crates/engine/src/shared.rs`): a table gives each read-only command a function taking
`&Session`, so the compiler proves it cannot change the session (a test checks the table against
the server policy). What such a run still updates is synchronized on its own:

- **The cancel flag** is per call (`call_scope.rs`, seen by `Session::is_cancelled`): closing one
  request's stream stops that query only.
- **`report.render {ui_language, culture, as_of}`** used to swap the session's fields for the
  render; the overrides now live on the calling thread for the call.
- **The preview caches** (decoded columns, storage-engine caches, aggregations, memory accounting)
  live in one state that an evaluation holds while it runs. A second evaluation that finds it busy
  **forks** (`preview_fork.rs`): a private state seeded from what the busy one published (the
  opened backup, bicraft's data snapshot, the model registry, weak references to the decoded
  columns, the memory settings). It never opens a backup or reads a data snapshot itself: when the
  seed does not fit the session's revision and data, it waits for the main state (as before). At
  the end, the columns and literal tables it read go back to the main state, only for the same
  revision and data generation and only where the main state has nothing (else they are dropped);
  its storage-engine caches are dropped. Forks answer from a read-only snapshot of the
  report-derived aggregations the main state built (the same plans and tables, published with the
  seed for its revision and data; a model or data change drops them as before). The aggregation
  hits a fork counts and the facts it wished aggregations for go back to the main state, whose
  caller starts the background builds.
- **A short wait before forking.** An evaluation that finds the main state busy waits up to 50 ms
  for it (woken when it is released) before it forks: a fork starts with empty storage-engine
  result caches, so a warm query is answered faster on the main state a few ms later; a long
  query is not waited for.
- **One read per column.** Evaluations that miss the same stored column at the same time read it
  once: the first claims it, the others wait for it and take the same decoded column (a read
  that succeeded stays listed weakly, for evaluations missing it later while it is alive
  somewhere). A claim is held only while its column is read, never while waiting for another.
- **Memory.** Each evaluation has its own query budget (`BICRAFT_QUERY_BUDGET`, `dax.memory
  {query_budget}`), every reservation goes to the one process pool, so `BICRAFT_MEMORY_LIMIT`
  holds across all of them. The seed holds no column alive (weak references), so the session's
  memory budget can still evict any column no query holds; a fork's own columns count against
  the budget within the fork, and the main state enforces it again when it takes them over (in
  between, a session may hold up to its budget once per running fork). Background aggregation
  jobs wait until no evaluation of the session runs.

**Optimistic concurrency for editors.** Every `tools/call` result of the server carries the
session's revision after the call in `_meta` (`"io.bicraft/revision"`); `run_command
{command: "session.revision"}` reads it cheaply (with the unsaved-changes flags). The revision
goes up by one after every call that may have changed the session and never repeats for a
workspace: a new session starts above the last one's revision and above the wall clock in
milliseconds, so a revision read before an idle close, an upload over the open file or a restart
never matches. `run_command` and `batch` take an optional `expected_revision` (in their schemas):

- equal to the session's revision: the call runs (a `batch` checks it once, before its first step,
  under the same exclusive lock);
- different: a tool error with `code: "revision_conflict"`, `expected_revision`,
  `current_revision` and the current revision in the message; **nothing runs**
  (`bicraft_revision_conflicts_total` counts them);
- absent: as before, **the last writer wins**.

Convenience tools (`add_measure`, …) take no precondition: use `run_command` or `batch`. Readers
cannot pass one (they change nothing; refused). Over stdio the argument works too, without the
`_meta` revision (use `session.revision`).

**Metrics.** `bicraft_concurrent_reads` (gauge), `bicraft_write_lock_wait_seconds` (histogram:
how long a write waited for running reads), `bicraft_revision_conflicts_total` (counter).

**Measured**: `docs/benchmarks.md`, "Concurrency on one workspace" (`cargo xtask
concurrency-bench`). On 10 M rows, a short read 20 ms after a 1.6 s query answers in 64–94 ms
instead of 1.6 s. A separate fix removed a ~60 ms floor from every HTTP tool call: the
request's connection watcher is no longer joined.

### Flags

| Flag | Default | Meaning |
|---|---|---|
| `--max-upload-bytes N` | `4G` | largest upload (`K`/`M`/`G` suffixes) |
| `--shutdown-grace SECS` | 30 | grace period of the drain (0 to 86 400) |
| `--idle-timeout SECS` | off | idle close of workspace sessions |
| `--ready-memory-percent P` | off | `/readyz` 503 above P % of `BICRAFT_MEMORY_LIMIT` |
| `--drain-on-stdin-close` | off | the end of standard input starts the drain, like SIGTERM (for supervisors without signals: Windows services, test harnesses; added with `feat/deploy`) |
| `--no-viewer` | viewer on | no live viewer at `/w/<id>/view/` (`docs/kubernetes.md`, "Viewer"; added with `feat/viewer`) |
| `--viewer-copies N` | 1 | read-only copies of saved files kept for readers (one per file) |
| `--max-viewers N` | 256 | private browser views kept per workspace |
| `--max-concurrent-reads N` | 4 | read-only calls and viewer reads running at once on one workspace (1: one at a time; added with `feat/concurrency`) |
| `--max-views-per-user N` | 8 | document views (a browser tab on its own file, a session of its own) one identity holds at once, over the server (added with `feat/webui-parallel-views`; docs/mcp.md, "Document views") |
| `--max-views N` | 32 | document views open on one workspace |
| `--view-idle SECS` | 1800 | an unused document view closes (four times as long with unsaved edits) |
| `--recovery-dir DIR` | off (`BICRAFT_RECOVERY_DIR`) | auto-recovery: editor sessions journal their unsaved edits in `DIR/<workspace>/`, so an expired view, a dropped session or a restart loses nothing; reopening the file offers them (docs/recovery.md; added with `feat/auto-recovery`). Keep it on a persistent volume, outside the workspace root; it holds users' unsaved DAX, M and names |
| `--allow-autosave` | off | the UI may save PBIP projects automatically (the snapshot's `view.limits.autosave`; saves go through the per-file check) |
| `--viewer-control` | off | the UI control channel relayed to editors' app pages, behind the server's authentication and a control token (below, "UI control channel"; added with `feat/control-serve`) |
| `--viewer-control-file F` | `control-serve.json` next to the desktop's settings | where the control token is written (owner-only) |
| `--local-offset OFFSET` | UTC (`TZ` set: the host's zone) | Power Query's local clock (`DateTime.LocalNow`): `±HH:MM`, minutes, `UTC` or `machine` (docs/formats/data-sources.md, "Clock"; added with `chore/deploy-followups`) |

In Kubernetes these flags are chart values (docs/kubernetes.md, "Values reference" and
"Viewer"): `viewer.documentViews.perUser` / `.perWorkspace` / `.idleSeconds`
(`--max-views-per-user`, `--max-views`, `--view-idle`; 0 keeps the server's defaults),
`recovery.*` (`--recovery-dir` on a claim of its own, mounted at `/var/lib/bicraft/recovery`,
outside the workspace root; on by default) and `recovery.allowAutosave`, `viewer.control.*`
(`--viewer-control`, the token from a Secret as `BICRAFT_VIEWER_CONTROL_TOKEN`: the pod's file
system is read-only, so no token file), `server.localOffset`. The controller has the same under
`workspaces.pod.*` and `workspaces.recovery.*`.

Library defaults (`HttpConfig`): `max_upload_bytes`, `upload_limits` (30 s stall, 2 h),
`zip_limits` (`ZipLimits`), `commit_wait` (30 s), `idle_timeout`, `ready_memory_percent`.

Residual risks: a second process serving the same root would remove the first one's upload temp
folders at start; one server per root (one pod per volume) is the supported setup. Hard links
cannot arrive through uploads (ZIP has no such entry, and `PUT` writes new files only).

## The live viewer on a network

**Implemented** (stage 2, 2026-10-28, branch `feat/viewer`): `bicraft serve` serves the viewer
itself, under each workspace's prefix, rather than letting `crates/view` leave loopback (`bicraft
view` is unchanged: loopback only, no authentication). Operations: `docs/kubernetes.md`, "Viewer";
browser credentials and CSRF: `docs/auth.md`, "The viewer in a browser"; what readers see:
`docs/auth.md`, "Roles".

| Item | `bicraft view` (local, unchanged) | `bicraft serve` (server mode) |
|---|---|---|
| Bind address | 127.0.0.1 only (`start_with` refuses others) | the MCP listener: no second port |
| `Host` check | localhost names only | the server's allow-list (`--public-url`, `--allow-host`), DNS rebinding still refused |
| Authentication | none; random per-run token in the page | the MCP endpoint's: trusted identity header (oauth2-proxy, the primary browser path), bearer tokens, plus a cookie from a one-time login URL; the workspace ACL (404 like a missing workspace) |
| CSRF / origin | token + `Origin` + `Sec-Fetch-Site: same-origin` | the same three, per page; the expected origin is the configured public origin |
| TLS | none | terminated at the ingress |
| Open in Desktop | `POST /api/open-in-desktop` launches the OS handler | no such route; the page downloads the saved file through `/w/<id>/files/` |
| Workspace scope | one viewer per process | `/w/<id>/view/…`; the page uses relative URLs, so one page serves both |
| Who sees what | one session, shared with the agent | editors: the editor session (their MCP session); readers: saved files only (the clean editor session, else one shared read-only copy per file); every page: a private view state (slicers, selections, page) |
| Limits | bodies ≤ 64 KiB, 16 queued interactions, caps on connections and assets | bodies ≤ 64 KiB; `--max-viewers` views per workspace; event streams at most half the connection cap; `--viewer-copies` copies; per-identity rate limits stay at the ingress |

**Decision: private views over shared sessions.** The two reader questions left by the roles lot:

1. *Readers must not see an editor's unsaved edits, and one viewer's slicers must not change
   another's view.* Chosen: a **private view state per page** (the engine's `Interactions`, a few
   KB) evaluated over a **shared session** (`Session::snapshot_view`, `Session::interact_view`):
   readers share the clean editor session when it has their file open, else one read-only copy per
   saved file. Memory is one model (two while an editor holds unsaved edits), whatever the number
   of readers; evaluated visuals are cached per session and reused by every view with the same
   filters. Rejected: a copy per reader (N times a 100 M-row model) and the session's own view
   state (one viewer's click changing everyone's page, as `bicraft view` does locally).
2. *A reader opening a file must not drop an editor's undo history.* A reader's `file.open` is
   private: it names the file that reader reads (served from the shared copy of its saved state)
   and never runs on the editor session.
3. *Browser tabs on different files at once* (2026-11-05, `feat/webui-parallel-views`): a
   **document view** is a private session of its own on one workspace file (an editor's; a
   reader's reads the saved state as above), opened by `POST …/view/api/session {"file"}` and
   called at `/w/<id>/mcp/view/<view>`; capped per user and per workspace, idle-closed, refused
   when memory is short; saves are checked per file (`file_conflict`). The workspace's session
   and its clients are unchanged ([mcp.md](mcp.md), "Document views").

### UI control channel (`--viewer-control`)

Added with `feat/control-serve` (2026-11-05; [gaps.md](gaps.md) G39). The desktop's control
channel ([desktop.md](desktop.md), "Control channel": the same methods and replies) relayed to
the browser app pages of each workspace, so an editor's agent drives its own app page and takes
screenshots of it: open a file in a document tab, run a command as its button does, switch tabs
and pages, the palette and the forms, `ui.screenshot`. Off by default; off, the routes below
answer 404 and the app page has no control slot.

```text
bicraft serve --root R --viewer-control [--viewer-control-file F]   # token in F (owner-only)
BICRAFT_SERVER_TOKEN=<an editor's API token> \
  bicraft ui --serve --workspace sales ui.inspect                   # the default token file
bicraft ui --url https://bi.example.com/w/sales/view/ server.views  # BICRAFT_CONTROL_TOKEN
bicraft ui --serve --workspace sales --view-id <view> ui.screenshot --out page.png
```

| Request | Who | Effect |
|---|---|---|
| `POST /w/<id>/view/api/control[?v=<view>]`, the request object as the body | the agent: an editor credential (bearer or proxy headers, never the viewer cookie) and `X-Bicraft-Control: <control token>` | relayed to one of the caller's app pages; the answer is the page's reply |
| `GET /w/<id>/view/api/control` | the same | the caller's views: `{pages: [{view, connected, docs: [{view, file}]}]}` (`bicraft ui … server.views`) |
| `GET /w/<id>/view/api/control/events?v=<page view>` | the app page (its viewer credentials, the cookie included) | Server-Sent Events, the requests for this page |
| `POST /w/<id>/view/api/control/reply?v=<page view>` | the app page: the view's token in `X-Bicraft-Token`, same origin | the reply to a request this page took |

- **The control token.** New at each start (128 bits) and written, never printed, to an
  owner-only file: `--viewer-control-file F`, else `control-serve.json` next to the desktop's
  settings (`%APPDATA%\BIcraft`, `~/.config/bicraft`), `{"kind": "serve", "url", "token",
  "pid"}` (`url`: `--public-url`, else the listen address); removed after the drain. An operator
  who supplies the token instead (a Kubernetes secret) sets `BICRAFT_VIEWER_CONTROL_TOKEN` (32
  to 256 visible ASCII characters); no file is written then unless `--viewer-control-file` is
  given. Owner-only means mode 0600 on Unix, and on Windows an explicit access list: inheritance
  removed, one entry, full control for the user's SID (set through `icacls`, before the token
  is written). `--viewer-control` needs the browser app embedded and the viewer on.
- **Which page.** A request goes to one app page of the **caller's own identity**, never
  another's: `?v=` naming one of its page views (each app page load is one), or a **document
  view** that page opened (`POST …/api/session?v=<page view>`, as the app does for each document
  tab): the page switches to that file's tab first, as a click on the tab does, then runs the
  request there. Without `?v=`: the caller's app page connected last. A document view no app
  page opened (a native client's) answers 409; a view of someone else, 404; no page of the
  caller connected, 503.
- **Security model.** Two credentials, both required on the agent's route: the server's own
  (the identity, checked against the ACL: editors and owners only, 403 for readers) and the
  control token, which only the file's owner (or the operator) holds. A web page cannot use the
  route: a request carrying `Origin` or `Sec-Fetch-Site` is refused (403), even with both, and
  the browser cookie never authenticates it. The page's routes need the viewer's usual
  credentials and, for replies, the view's own token and the same-origin checks of
  `/api/interact`; a page's stream is its owner's (another identity's `?v=` is a 404), and only
  the page a request went to can answer it. A relayed request can do what the page can: the
  page's own credentials run its commands (an editor's document view, its MCP endpoint), so the
  channel adds no right the identity does not have; it adds the reach of an agent into that
  identity's open browser tab. The page never sees the control token.
- **Caps.** Requests per workspace at most 200 a second (429), at most `--queue` waiting
  (`queue_per_workspace`, 8 by default; 64 at most: 503), bodies at most 1 MiB (and
  `--max-body-bytes`), replies at most 48 MiB (a screenshot), 60 s for a reply (a request no page
  took by then never runs; one it took is "outcome unknown"). Page streams count against the
  viewer's stream cap (half the connection cap) and `--max-viewers`; a drain ends them at once.
- **Metrics.** `bicraft_control_requests_total` counts the agents' requests that reached the
  relay (requests and listings), `bicraft_control_request_errors_total` those that failed
  (refused, no page, timed out, or a reply with `"ok": false`).
- **In Kubernetes.** The chart's `viewer.control.enabled` with `viewer.control.existingSecret`
  (the controller: `workspaces.pod.viewerControl.existingSecret`) passes the token as
  `BICRAFT_VIEWER_CONTROL_TOKEN`; agents call `<public URL>/w/<id>/view/api/control` with their
  credential and `X-Bicraft-Control`. The controller's gateway forwards the header and the
  `/w/<id>/view/…` and `/w/<id>/mcp/view/<view>` routes as they are (only credentials and
  identity headers are replaced: `gateway::tests::view_and_control_routes_reach_the_pod_with_the_control_header`).
- Tests: `crates/mcp/tests/http_control.rs` (the page played by raw HTTP: routing by default, by
  page and by document view; the listing; every refusal; replies only from the page that took
  the request; the drain; the metrics); `apps/bicraft-cli/tests/browser_control.rs` (headless Firefox,
  ignored by default: [webui.md](webui.md), "Control channel in the browser").

## File storage and path sandboxing

**Implemented** (stage 1 lot 1, 2026-10-26): `docs/server-mode.md` has the rules, the full path-parameter
audit (16 commands, 26 parameters) and the process/network audit of every command.

Commands take **client-supplied filesystem paths** today, which is correct for a local tool and a
critical hole on a server:

- `file.open {path, data_root, cache_abf}` (`commands.rs`), `file.save_as {path}` and
  `project.new {dir}` (`save.rs`), `convert {path}` (`convert.rs`), `report.render {out}`
  (`render.rs`), image and custom-visual imports (`images.rs`, `custom_visuals.rs`), theme
  import/export (`theme_edit.rs`), VPAX export (`vpax.rs`), `data_root` for Power Query
  `File.Contents` (`datasrc.rs`, already a sandbox root for M reads).

Needed: a **workspace root** on `Session` (like `data_root`, but mandatory in server mode). Every
path parameter is resolved relative to it, rejected when absolute, when it contains `..` after
normalization, or when its canonical form (symlinks resolved) leaves the root. One helper in
`engine/src/paths.rs`, called by every command above, with a test per command. `data_root` and
`cache_abf` must also stay inside the root.

Storage layout per workspace: `/<root>/<workspace-id>/` holding the PBIX or PBIP folder, the
`.pbi/` local files (`cache.abf` when present), renders and exports. Files arrive through an
**upload endpoint** (multipart or raw PUT, size-capped, streamed to disk, never buffered whole) and
leave through a download endpoint; the agent then calls `file.open {path: "model.pbix"}`. **Implemented**
(raw `PUT`, streamed, capped; zipped projects by `POST`): "Operating `bicraft serve`", "Files".

Backing store options (open question 2): a `PersistentVolumeClaim` per workspace
(`ReadWriteOnce`, simplest, survives pod restarts), or object storage (S3/Azure Blob/MinIO) synced
to an `emptyDir` on open and written back on save. Object storage needs a client crate and
credentials; a PVC needs neither.

## Container image

**Implemented** (`feat/deploy`): `deploy/docker/Dockerfile`, `docs/kubernetes.md`. No CA bundle is needed:
the HTTPS clients use the Mozilla roots compiled into the binary (`webpki-roots`).

- **Binary**: the existing `dist` musl build (static-pie). Image `FROM scratch` (or distroless
  static) + the binary + CA roots only if the Databricks refresh is enabled: about 25 MB. The
  musl allocator is slower under heavy multithreaded allocation (`docs/releasing.md`); measure the
  engine bench in the container and switch to a glibc build on distroless/base, or link mimalloc,
  if it costs more than a few percent.
- **Non-root** (`runAsNonRoot`, fixed UID), `readOnlyRootFilesystem: true`, all capabilities
  dropped, `seccompProfile: RuntimeDefault`.
- **Writable volumes**: the workspace volume, and a spill volume for `BICRAFT_SPILL_DIR` and the
  temp dir (`TMPDIR`). The spill volume must be disk-backed: `beyond-ram.md` warns that a `tmpfs`
  `/tmp` lives in memory and defeats the budget; use `emptyDir` without `medium: Memory`, with a
  `sizeLimit`.
- **Threads**: set `BICRAFT_SCAN_THREADS` to the pod's CPU limit explicitly (Rust's
  `available_parallelism` reads the cgroup quota on Linux, but an explicit value is
  reproducible), `BICRAFT_AGG_THREADS` to a quarter of it.
- **Probes** (implemented, "Operating `bicraft serve`"): `GET /healthz` for liveness, `GET /readyz`
  for readiness (503 while draining; optionally while reserved memory is above a share of
  `BICRAFT_MEMORY_LIMIT`). Neither touches model data.
- **Graceful shutdown** (implemented): on `SIGTERM`, `/readyz` and new requests answer 503, running
  requests get `--shutdown-grace` (default 30 s; keep it a few seconds under
  `terminationGracePeriodSeconds`), longer DAX queries are cancelled, **nothing is saved
  implicitly** (an unsaved workspace is reported, not written), sessions and their spill files and
  the upload temp folders are removed, exit 0 or 3.

## Kubernetes topology

**Stage 1 implemented** (`feat/deploy`): the Helm chart `deploy/helm/bicraft` (one static release per
workspace) and `deploy/manifests/`. **Stage 2 implemented** (`feat/controller`): the controller, see
"Workspace controller (stage 2)" below.

**One pod per open workspace (recommended).** Each pod runs `bicraft serve` (new subcommand) with
one `Session`, the model warm in memory, its own memory limit sized from the table above, and its
own volume. Routing: the ingress maps `/w/<id>/` to that pod's Service. Isolation between
customers is the pod boundary, memory accounting is the cgroup, and a crash or OOM loses one
workspace only. Cost: one idle pod per open model; scale to zero after an idle timeout.

**Shared pods (several sessions per process)**: cheaper for many small models (a 1 M-row model
peaks at 0.11 GB). Possible in code (sessions are independent, the budget is per session) but
needs a process-wide memory cap across sessions, consistent-hash routing by workspace id at the
ingress (sticky), and gives weaker isolation. Stage 3 at the earliest, for small models only.

Components:

- **Controller** (stage 2): a small service that creates the workspace (PVC, upload URL), starts
  the pod (a `Deployment` with one replica or a bare `Pod`/`StatefulSet` ordinal), registers the
  route, and deletes the pod after the idle timeout. Without it (stage 1), one static `Deployment`
  per model, declared by hand.
- **Ingress**: NGINX or Gateway API; path routing, TLS, OIDC (oauth2-proxy), body-size limits, SSE
  without buffering (the spec asks the server to send `X-Accel-Buffering: no`), request timeouts
  longer than the longest allowed DAX query (600 s).
- **Cold start**: pod scheduling + image pull (small image, seconds) + copying the model to the
  volume (bandwidth-bound; none with a PVC) + `file.open` (0.44 s at 100 M) + the first query.
  Aggregations rebuild in the background after each start: they are not persisted today.
- **Autoscaling**: per workspace it is 0 or 1 pod (a `Session` cannot be split). Cluster capacity
  scales with the cluster autoscaler. Horizontal scaling of *one* model (read replicas of a
  read-only report) is a stage-3 option: N pods open the same saved file read-only.
- **Secrets**: the Databricks token as a Kubernetes `Secret` mounted as an env var, only in pods
  of workspaces that enabled refresh; OIDC client secret only in the auth proxy.
- **Observability** (implemented, stage 2, `feat/metrics`: `docs/kubernetes.md`, "Observability"):
  structured logs to stderr (`--log-format json` / `BICRAFT_LOG_FORMAT`: one JSON line per request
  with route, workspace id, caller kind, command id, status, outcome, duration, sizes; no data, no
  file names, no user ids); Prometheus metrics on a separate listener (`--metrics-listen`, no
  authentication, `/metrics` only, never on the MCP port: requests, command latency histograms, queue
  and in-flight, open workspaces, memory limit use, evictions and spill bytes, aggregation build state,
  open/save durations, drain state); Helm values, ServiceMonitor and a Grafana dashboard
  (`deploy/grafana/`). Tracing later.

## Workspace controller (stage 2)

Design chosen 2026-10-27 (branch `feat/controller`). Goal: workspaces are created, started,
routed, stopped when idle (scale to zero) and deleted on demand, instead of one Helm release
declared by hand per workspace.

### Choice: a CRD reconciled by `bicraft controller`, which is also the gateway

- **A `BicraftWorkspace` custom resource** (`bicraft.dev/v1alpha1`, namespaced, short name
  `bws`; the group `bicraft.dev` is a placeholder until the project's domain is chosen, and is
  configurable: `crdGroup` in the controller configuration, `crd.group` in the chart). The resource is the source of truth: `kubectl get bws` lists workspaces, GitOps tools
  can declare them, and the controller's HTTP API is a thin layer that creates and deletes these
  objects. `metadata.name` is the workspace id (a DNS label of at most 40 characters, so the
  derived names `bws-<id>` fit the 63-character limits).
  - `spec`: `sizeClass` (a class the admin defined; the class fixes memory limit, budget, CPU,
    aggregations, spill size and the default and largest volume), `volume` (`size`,
    `storageClass`, `reclaimPolicy: Retain | Delete`), `idleTimeoutSeconds` (0 = never stop),
    `grants` and `readers` (subjects of `docs/auth.md` with editor and reader access: `token:agent`,
    `group:bi-team`, `email:…`; each opens this workspace only), `egress` (names of admin-defined egress profiles: CIDRs, ports, and
    optionally the Databricks token Secret), `suspend` (force zero replicas).
  - `status` (subresource): `phase` (`Pending`, `Idle`, `Starting`, `Running`, `Failed`),
    `message`, `endpoint` (`<public URL>/w/<id>/mcp`), `lastActivity` (RFC 3339), `readyReplicas`,
    `volumeClaim`, `observedGeneration`.
- **Model sizes stay admin parameters** (decision 5): users pick a class name, never raw
  memory or CPU; a requested volume above the class's maximum is refused (`Failed`).
- **The controller owns, per workspace**, a `Deployment` `bws-<id>` (1 or 0 replicas, Recreate,
  the same pod as the stage-1 chart: non-root 65532, read-only root, no capabilities, seccomp,
  disk-backed `/tmp`, probes, memory limit = `BICRAFT_MEMORY_LIMIT` + 64 MiB), a `Service`, a
  `NetworkPolicy` (ingress only from the controller's pods, egress DNS plus the chosen profiles)
  and a `PersistentVolumeClaim` `bws-<id>-data`. Everything is written with **server-side
  apply** (one idempotent PATCH per object, field manager `bicraft-controller`) and carries an
  `ownerReference` to the workspace, so **deleting the workspace is deleting the resource**: the
  Kubernetes garbage collector removes the children. The PVC gets the owner reference only with
  `reclaimPolicy: Delete`; with `Retain` (the default) it outlives the workspace, and a workspace
  created again under the same name adopts it. No finalizer is needed.
- **The reconciler is level-triggered**: every few seconds (and at once after an API call) it
  lists the workspaces and their children, computes the desired objects from the spec and the
  runtime state (awake or not), applies what differs and writes the status. A restart loses
  nothing: whether a workspace is awake is read back from its Deployment's replica count.
- **The controller is also the gateway** (router + activator): one `Ingress`/`HTTPRoute` sends
  `/w/…`, `/api/…` and `/.well-known/oauth-protected-resource…` to the controller's Service. It
  authenticates the request with the same code and the same three modes as `bicraft serve`
  (`bicraft-auth`), checks the `Host`, checks the workspace's grants, then forwards the request to
  `bws-<id>:7878` and streams both directions without buffering (SSE, 4 GiB uploads, downloads;
  one request per connection, as `bicraft serve` answers `Connection: close`).
  - **Nobody bypasses the gateway inside the cluster.** The forwarded head carries no credential
    (`Authorization` and identity headers removed); the identity the gateway checked travels in
    `X-Bicraft-Gateway-User`/`-Email`/`-Groups`. Workspace pods run `bicraft serve` in
    trusted-header mode only, trusting those headers from the controller pod's address (`POD_IP`,
    or admin-set CIDRs), with the grants `group:bicraft-gateway-editor=<id>` and
    `group:bicraft-gateway-reader=<id>` (reader access, `docs/auth.md` "Roles"): the gateway enforces
    the workspace's grants and sends the level it granted as that group, and their `NetworkPolicy` admits the controller's pods only. A pod elsewhere in the cluster is
    refused by the policy and, without it, by the trusted-address check.
  - **Availability**: one controller replica is a single point of failure and the upload
    bandwidth path. Stage 3: leader election for the reconciler (a `Lease`), stateless gateway
    replicas (activity merged through the workspace status), so the data path scales
    horizontally.
  - **Wake on request**: a request for a workspace at zero replicas sets it awake, applies 1
    replica, and holds the request until the pod's `/readyz` answers 200 (default limit 180 s,
    then 503 with `Retry-After`). Unauthenticated or unauthorized requests never wake anything.
  - **Idle stop**: the gateway sees every request, so activity is exact: a workspace with no open
    connection and no request for `idleTimeoutSeconds` goes back to 0 replicas. The pod drains as
    in stage 1 (nothing saved implicitly: an agent must save before going idle; a session holding
    unsaved changes is reported in the pod's log). `lastActivity` is published in the status.
  - Why in the data path rather than routing the ingress straight to each pod: it works with any
    ingress controller or Gateway implementation (one static route), wake-up needs no
    ingress-specific fallback, activity needs no extra signal from the pod, and network policy can
    say "only the controller reaches workspace pods". Cost: one extra hop and the bytes copied
    once more by a `std` thread per connection (the same model as `bicraft serve`).
- **Admin/agent API** (same authentication; creating and deleting needs an admin subject, from
  `--admin` / `BICRAFT_CONTROLLER_ADMINS`): `GET /api/v1/workspaces` (the ones the caller may
  open; all for admins), `POST /api/v1/workspaces` (a JSON body shaped like `spec` plus `name`),
  `GET` / `DELETE /api/v1/workspaces/<id>`, `POST /api/v1/workspaces/<id>/wake` and `/stop`.
- **No new dependency.** The Kubernetes API is reached by a small hand-written client over
  `ureq` + rustls (already used by `bicraft-auth` and `bicraft-databricks`): in cluster, the
  service-account token (projected, re-read every minute and after a 401, as the kubelet rotates
  it) and the API server verified against the service account's CA; outside, `--kube-api URL` (e.g. `kubectl
  proxy`). The controller needs five verbs on five resource types. `kube-rs` was considered and
  rejected: it brings tokio, hyper, tower and `k8s-openapi` (hundreds of crates, a long build,
  async below the CLI), for list/apply/delete calls that are a few lines each with server-side
  apply. Watches are not used (polling every few seconds is enough for tens of workspaces; a
  watch can come later without changing the design).
- **Code**: crate `bicraft-controller` (L6, beside the MCP server and the viewer; depends on
  `bicraft-auth`, `bicraft-http`, `serde`, `serde_json`, `ureq`; no async runtime, no
  engine), subcommand `bicraft controller` in the same binary and image (one image to build,
  scan and sign). The reconciler talks to a `Kube` trait, so `cargo xtask ci` tests it against an
  in-memory fake; the gateway is tested against a fake upstream.
- **Deployment**: chart `deploy/helm/bicraft-controller` (the CRD as a template, so its group follows
  `crd.group`, kept on uninstall, the controller's
  Deployment with the same security context as a workspace pod, a `ServiceAccount`, a
  namespaced `Role` with least privilege: `bicraftworkspaces` get/list/create/patch/delete and
  their status patch; `deployments`, `services`, `persistentvolumeclaims`, `networkpolicies`
  get/list/create/patch; nothing else, no `delete` on children since the garbage collector does
  it, no access to pods or secrets), a `ConfigMap` with the classes, egress profiles and
  workspace pod settings, and optional `Ingress`/`HTTPRoute`.
- **Scope cut for this lot**: one controller replica (no leader election; Recreate), polling
  instead of watches, workspaces start at zero replicas and wake on their first request, no
  per-workspace Secrets created by the API (a Databricks token comes from an admin-defined egress
  profile). The viewer, document views and the UI control channel go through the gateway like
  every `/w/<id>/…` path; each workspace also gets a recovery claim `bws-<id>-recovery`
  (`--recovery-dir`) unless `workspaces.recovery.enabled` is false.

## Security review items

1. **Path traversal** from client paths (see "File storage"). Blocking for any network exposure. **Done** (stage 1 lot 1): `docs/server-mode.md`.
2. **Commands that touch the host**: `file.open_in_desktop` and the viewer's open route launch an
   OS handler (`desktop_open.rs`); disable in server mode. Audit every command for process launch
   or network access; keep a server-mode deny-list in `engine` (`Command` gets a flag). **Done**: `docs/server-mode.md`.
3. **Resource exhaustion**: upload size cap; PBIX decompression already caps allocations
   (never-crash rules) but the caps were set for a desktop machine; memory budget mandatory in
   server mode; DAX `timeout_ms` capped lower than 600 s per deployment; render size caps; batch
   length cap (`batch`); per-identity rate limits.
4. **Hostile files**: inputs are already treated as hostile (fuzzing, `docs/fuzzing.md`); a server
   makes that the main attack surface. Keep the fuzz smoke in CI and add the HTTP parser to it.
5. **Authentication and ACL**: identity from the ingress, a workspace ACL (owner, readers,
   editors), viewer read-only for readers (`--read-only` exists for `bicraft view`). **Done for
   MCP** (stage 1): `docs/auth.md` (three modes, ACL with editor grants); **readers done for MCP and
   files** (stage 2, `docs/auth.md` "Roles"); **the viewer for readers and editors done** (stage 2,
   `feat/viewer`); an owner level remains.
6. **Origin and DNS rebinding**: MCP spec requires `Origin` validation; reuse the viewer's code.
7. **Multi-tenancy**: one namespace per tenant or at least `NetworkPolicy` isolation; no shared
   volumes between workspaces.
8. **Supply chain**: the release already generates `THIRD_PARTY_CRATES.md` from `cargo metadata
   --locked`; dependency licences, sources and advisories are checked by `cargo deny` on every
   push (`deny.toml`, docs/ci.md, M11.5); image scanning and a signed image remain.

## Engine and API changes

| Change | Where | Stage |
|---|---|---|
| Workspace root + path resolution helper, applied to every path parameter | `engine` `Session`, `sandbox.rs`, the commands listed above | 1, done (`docs/server-mode.md`) |
| Server-mode command policy (deny OS launch, optional deny refresh) | `engine` `Command` table | 1, done |
| Cancel flag checked with the evaluator deadline (done), render and save loops | `engine` `preview_eval.rs`, `render.rs`, `save.rs` | 1 |
| Streamable HTTP transport, `2026-07-28` only (decision 7) (done) | `mcp` `http.rs` + `bicraft-http` (L0) | 1 |
| Viewer server mode: host allow-list, public origin, identity header and login cookie, no open route, private views, readers on saved files (done) | `mcp` `http/viewer.rs`, `http/readview.rs`, `engine` `snapshot_view`, `view` page | 2 |
| `bicraft serve` subcommand: MCP + upload/download + probes + signal handling (done); viewer | `apps/bicraft-cli` | 1 |
| Upload/download endpoints (streamed, capped) (done) | `mcp` `http/files.rs`, `opc` `extract.rs` | 1 |
| Authentication (OIDC, trusted header, API tokens), workspace ACL with editor grants | `bicraft-auth` (L0), `mcp` `http.rs`, `serve` | 1, done (`docs/auth.md`) |
| Reader access level (done, `docs/auth.md` "Roles"), per-identity rate limits | `bicraft-auth`, `engine`, `mcp` | 2 |
| Metrics endpoint, JSON logs, dashboard (done) | `mcp` `http/metrics.rs`, `http/reqlog.rs`, `deploy/grafana/` | 2 |
| Parallel reads on one session, optimistic concurrency (`expected_revision`) (done) | `engine` `shared.rs`, `call_scope.rs`, `preview_fork.rs`; `mcp` `http/rwgate.rs`, `tools.rs` | 2 |
| Progress notifications for open/save/render | `mcp`, `engine` | 2 |
| Process-wide memory cap across sessions (shared pods) | `engine` | 3 |
| Persisted aggregations (faster cold start) | `engine`, storage format | 3 |

## Staged plan

Estimates are focused engineering days, one person, including tests and docs; WSL CI must pass
for each item (the server is Linux-first).

**Stage 1: internal MVP (~15 days).** One model per pod, one trusted team.

- Path sandbox and server-mode command policy (3 d). **Done** (lot 1, `feat/sandbox`; wired into
  `bicraft serve` on `feat/wire`).
- `std` HTTP module extracted from `crates/view`; MCP Streamable HTTP, `2026-07-28` only
  (decision 7); cancel flag (5 d). **Done** (lot 2, `feat/mcphttp`).
- Authentication (OIDC resource server, trusted header, API tokens) and a minimal workspace ACL;
  non-loopback binding once configured. **Done** (`feat/auth`, `docs/auth.md`).
- Viewer server mode (2 d). **Done** in stage 2 (`feat/viewer`).
- `bicraft serve` with upload/download, `/healthz`, `/readyz`, SIGTERM handling (3 d). **Done**
  (`feat/serveops`, "Operating `bicraft serve`"), with the idle close of workspaces.
- Dockerfile (scratch + musl binary), example manifests (Deployment, Service, PVC, Ingress with
  oauth2-proxy, NetworkPolicy), a kind/k3d smoke test in CI that opens a generated fixture over
  HTTP (2 d). **Done** (`feat/deploy`, `docs/kubernetes.md`): the image, the Helm chart (one workspace
  per release; Ingress or HTTPRoute; NetworkPolicy and an optional Cilium FQDN policy), plain
  manifests, `cargo xtask deploy check` (in CI), `deploy smoke` (container-free) and `deploy
  kind-smoke` (kind + Helm).

**Stage 2: multi-user (~20 days).**

- Workspace controller: create, start, route, idle-stop, delete (8 d). **Done** (`feat/controller`,
  `docs/kubernetes.md` "The controller"): CRD, reconciler, gateway with wake on request and idle
  scale to zero, admin API, chart, `cargo xtask deploy controller-smoke`. Leader election and
  stateless gateway replicas: stage 3.
- ACL and roles (readers done for MCP and files, `docs/auth.md` "Roles"), viewer for readers
  (4 d). **Done** (`feat/roles`, `feat/viewer`).
- Metrics, JSON logs, dashboards (3 d). **Done** (2026-10-27, `feat/metrics`).
- Object-storage backend if chosen (3 d); progress notifications (2 d).

**Stage 3: production hardening (~20–30 days).**

- Security review and penetration test of the HTTP surface; fuzzing of the HTTP parser and the
  upload path (5 d).
- Load tests at 100 M rows per pod, memory-limit tuning, musl vs glibc allocator measurement (5 d).
- Shared pods for small models with a process-wide cap (5–8 d).
- Persisted aggregations for cold start; read replicas for read-only reports (5–10 d).
- Signed images, SBOM, backup and retention policies (2 d).

## Open questions for the user

1. **Where will it run, and whose data?** Your own cluster (on-premise, a cloud account you
   control) or a client's? Client data must never leave its owner's control: who approves each
   environment?
2. **Storage**: PVC per workspace, or object storage? Retention after a workspace closes?
3. **Users**: an internal team (stage 1 is enough) or external clients (stages 2–3, multi-tenant)?
   Which identity provider (Entra ID, Google, Keycloak)?
4. **Who drives it**: remote agents over MCP only, humans in the viewer only, or both? A
   browser-based editor is out of scope today (the viewer never writes the project).
5. **Model sizes** expected: mostly under 10 M rows (shared pods pay off) or up to 100 M+ (one pod
   each, 6–8 GB)?
6. **Databricks refresh** in the cluster: allowed (outbound egress, a token per workspace) or off?
7. **Protocol era**: which MCP clients must connect (some may only speak `2025-06-18`/`2025-11-25`
   sessions)?
