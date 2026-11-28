# Authentication of `bicraft-cli serve`

Status: implemented 2026-10-26 (web stage 1, authentication lot; `docs/web-deployment.md`,
decision 3 and security item 5). Code: `crates/auth` (`bicraft-auth`, L0), wired into the MCP
HTTP endpoint (`crates/mcp/src/http.rs`) and `bicraft-cli serve` (`apps/bicraft-cli/src/serve_cmd.rs`).

`bicraft-cli serve` authenticates every request once at least one mode is configured, then checks the
identity against a workspace ACL. Without any mode it keeps listening on loopback only and refuses
any other `--listen` address. Three modes, any combination:

| Mode | For | Credential |
|---|---|---|
| **OIDC / OAuth 2.1 resource server** | humans through MCP clients, workload identities | `Authorization: Bearer <JWT>` from a configured issuer |
| **Trusted identity header** | anything behind an auth proxy (oauth2-proxy, Pomerium, Authelia, Traefik forward-auth, a mesh) | a header set by the proxy, accepted only from the proxy's addresses |
| **Static API tokens** | agents and CI | `Authorization: Bearer bct_…`, stored as SHA-256 |

## What the MCP authorization spec asks, and what BIcraft does

Verified on 2026-10-26 against modelcontextprotocol.io, revision `2026-07-28` (pages
"Authorization", "Authorization Server Discovery", "Security Considerations"):

| Spec (resource server side) | BIcraft |
|---|---|
| MCP servers MUST implement Protected Resource Metadata (RFC 9728); the document MUST list at least one `authorization_servers` entry | `GET /.well-known/oauth-protected-resource/w/<id>/mcp` (path-suffixed, RFC 9728 §3) and `GET /.well-known/oauth-protected-resource` (root): `resource`, `authorization_servers` (the configured issuers with `advertise: true`), `scopes_supported`, `bearer_methods_supported: ["header"]`, `resource_name`. Served only with an OIDC issuer; public, `Access-Control-Allow-Origin: *`, the same for any well-formed workspace id (no existence oracle). |
| Servers MUST offer the metadata through `WWW-Authenticate` `resource_metadata` on 401, or at a well-known URI | both |
| Servers SHOULD send `scope` in the 401 challenge | `scope="…"` from `scopes_supported` (default: the issuers' required scopes) |
| Invalid or expired tokens MUST get 401 (OAuth 2.1 §5.3) | 401 `error="invalid_token"` with a coarse `error_description` (no token or claim value) |
| Insufficient scope SHOULD get 403 with `error="insufficient_scope"`, `scope`, `resource_metadata` | yes, all the required scopes in one challenge |
| Malformed authorization request: 400 | 400 `error="invalid_request"` |
| Servers MUST validate tokens (OAuth 2.1 §5.2) and MUST check they were issued for them as audience (RFC 8707) | signature, `iss`, `exp`/`nbf`/`iat`, `aud` ∈ {the workspace's resource URL, the public URL, configured audiences} |
| Tokens MUST travel in the `Authorization` header of every request, never in the query string | only the header is read, on every request (discovery and `tools/list` included) |
| Servers MUST NOT pass tokens through | BIcraft never forwards a token (the Databricks connector uses its own) |
| Canonical URI: accept uppercase scheme and host; no trailing slash preferred | audiences compare scheme and host case-insensitively and ignore one trailing `/` |

Client-side duties (PKCE, `resource` parameter, Client ID Metadata Documents or dynamic
registration) belong to MCP clients and authorization servers: an interactive MCP client can only
log in through an issuer that supports what the client needs (many enterprise issuers do not offer
dynamic registration; clients then use a pre-registered client id).

## Request flow

For `/w/<id>/mcp`, in order: `Host` check → route → **authentication** (401/403/400) → **ACL**
(a workspace the identity may not open answers 404 exactly like a missing one) → method, `Origin`,
body and the MCP checks of `docs/web-deployment.md`. Authentication runs before anything that could
reveal whether a workspace exists.

Per request, the modes are tried as follows:

1. Trusted-header mode configured and the user header present: the TCP peer must be in a trusted
   range (else 400 `invalid_request`, the header is never ignored silently); the header gives the
   identity and `Authorization` is not read (oauth2-proxy may forward its own ID token there).
2. Else `Authorization: Bearer <token>`: a token whose SHA-256 matches a configured API token is
   that token's identity; otherwise, with OIDC issuers, a JWT is validated against the issuer its
   `iss` names. Another scheme (`Basic`…) counts as no credentials.
3. No credentials: 401 with the challenge.

`Authorization` and the identity headers must appear at most once (400 otherwise).

## OIDC

Per issuer:

- **Keys**: the JWKS URL comes from `jwks_uri`, else from discovery: `{issuer}/.well-known/openid-configuration`,
  then RFC 8414 `/.well-known/oauth-authorization-server{path}`; the document's `issuer` must be
  identical to the configured one (mix-up protection). Keys are cached and refetched after
  `jwks_refresh_secs` (3600). A token whose `kid` is unknown refetches the keys once (rotation),
  at most every `jwks_min_refetch_secs` (30) so random `kid`s cannot flood the issuer; an
  unreachable issuer is retried at most every 10 s. Old keys stay in use while a refresh fails.
- **Fetching**: blocking HTTPS (`ureq` + rustls/ring + bundled Mozilla roots, no proxy, 10 s
  timeout, 1 MiB cap, up to 3 HTTPS-only redirects). `http://` only for loopback hosts (a local
  Keycloak, tests).
- **Algorithms**: RS256/384/512, PS256/384/512, ES256, ES384, EdDSA (Ed25519) by default.
  `none` is always refused (any case). HS256/384/512 only when listed in `algorithms` **and**
  `hmac_secret_file` or `hmac_secret_env` is set (32 bytes at least); listing them is never the
  default, so a public key cannot be used as an HMAC secret. A key's own `alg` and `use` are
  honoured. Tokens with `crit` or JWE headers are refused.
- **Claims**: `iss` exact; `exp` required, `nbf` and `iat` checked, with `leeway_secs` (60, at
  most 300); `aud` (string or array) must contain the workspace's resource URL
  (`<public>/w/<id>/mcp`), the public URL, or one of `audiences`; `required_scopes` against `scope`
  (RFC 9068) or `scp` (Entra ID, Okta; string or array); `required_claims` (a string claim equal
  to the value, or an array containing it; booleans compare as `true`/`false`).
- **Identity**: `subject_claim` (`sub`), `email_claim` (`email`), `groups_claim` (`groups`),
  `roles_claim` (`roles`); a claim path is the whole name first (namespaced claims such as
  `https://example.com/roles`), then a dotted path (`realm_access.roles`). `role_map` renames role
  values (`{"BIcraft.Editor": ["editor"]}`); unmapped values are kept.

Without `--public-url`, the public URL is `http://<Host>` (loopback development only); set it on
any real deployment so resource URLs, metadata URLs and audiences are the ones clients see.

## Trusted identity header

`proxy.trusted` lists the proxy addresses or CIDR ranges (`10.0.0.0/8`, `fd00::/8`, one address).
They are compared with the **TCP peer address** (IPv4-mapped IPv6 peers compare as IPv4), never
with `X-Forwarded-For`. Header names are configurable (defaults: oauth2-proxy's
`X-Forwarded-User`, `X-Forwarded-Email`, `X-Forwarded-Groups`, groups separated by `,`).

The proxy must **remove** these headers from client requests before setting its own, and the
network must make the proxy the only way in (a `NetworkPolicy` allowing ingress to the BIcraft pod
from the proxy pods only). A header from any other address is refused with 400.

## API tokens

```sh
bicraft-cli serve token create --name ci --role agent
# token: bct_…            (stdout, shown once: give it to the agent)
# entry: {"name":"ci","sha256":"…","roles":["agent"],"groups":[]}
```

Put the entries in a tokens file and pass it with `--api-tokens FILE` or `BICRAFT_API_TOKENS_FILE`
(in Kubernetes: a `Secret` mounted as a file):

```json
{"tokens": [
  {"name": "ci", "sha256": "5d41…", "roles": ["agent"]},
  {"name": "nightly-refresh", "sha256": "7c2a…", "groups": ["bi-team"]}
]}
```

A token generated elsewhere (a secret manager) is hashed with
`printf %s "$TOKEN" | bicraft-cli serve token hash --name ci`; it must have at least 32 characters.

**Why SHA-256 and not argon2.** A salted slow hash protects *passwords*, which are guessable. A
token is 256 random bits from the OS generator: no hash makes brute force feasible, so SHA-256
loses nothing, and every request stays cheap (a slow hash per request would let anyone burn the
server's CPU). The presented token is hashed and compared with every stored hash with a
constant-time comparison that never stops at the first match. Tokens are identities in the ACL as
`token:<name>`, and their `roles` and `groups` match `role:` and `group:` grants. Rotation: add the
new entry, deploy, switch the agent, remove the old entry (the file is read at start).

## Workspace ACL

Grants map a subject to workspaces. No matching grant: the workspace is closed (404, like a
missing one). At least one grant is required once a mode is configured (`serve` refuses to start
otherwise).

| Subject | Matches |
|---|---|
| `*` | any authenticated identity |
| `user:<subject>` | the OIDC subject claim or the proxy's user header (never a token) |
| `email:<address>` | the e-mail (ASCII case-insensitive) |
| `group:<name>` | an OIDC groups value, a proxy group, a token's group |
| `role:<name>` | a (mapped) OIDC role, a token's role |
| `token:<name>` | one API token |
| `issuer:<url>` | every identity from one OIDC issuer |

Flags and variables: `--grant 'group:bi-team=*'`, `--grant 'email:ana@example.com=sales,hr'`
(the last `=` separates, so LDAP group names with `=` work); `BICRAFT_AUTH_GRANTS` holds grants
separated by `;` or spaces. In the file: `{"subject": "group:bi-team", "workspaces": "*",
"access": "editor"}`.

Reader grants have the same form: `--reader-grant 'group:bi-viewers=*'`,
`BICRAFT_AUTH_READER_GRANTS`, or `"access": "reader"` in the file (`"editor"` is the default).

## Roles

Status: implemented 2026-10-27 (web stage 2, "ACL and roles"). Two access levels:

| Level | MCP endpoint | File routes |
|---|---|---|
| `editor` | every command server mode allows | `GET`, `HEAD`, `PUT`, `POST` (zip), `DELETE` |
| `reader` | read-only commands only (below) | `GET` and `HEAD` only; the others answer 403 |

**Precedence.** An identity's access on a workspace is the **highest** level among all the grants
that match it, whatever their kind (`user:`, `group:`, `role:`, `token:`…), their source (file,
variable, flag) or their order. There are no deny rules: someone in a reader group and in an editor
group is an editor; to make someone a reader only, take them out of every editor grant. No matching
grant: the workspace is closed (404). The levels are ordered (`bicraft_auth::Access`), so an
`owner` or `admin` level (managing a workspace or its ACL) can be added above `editor` later
without changing the rule.

**What a reader may run comes from the commands themselves.** Each engine command declares an
`effect` in its row of the command table (`bicraft_engine::Effect`, on its `ServerPolicy`), and
the default is `Mutating`, so a new command is never readable by accident.
`crates/engine/tests/commands/roles_tests.rs` lists every registered command with its expected class, and
fails when a command is added without being classified there.

| Effect | Commands | Reader |
|---|---|---|
| read-only | `file.inspect`, `model.summary`, `model.get`, `table.get`, `column.get`, `measure.get`, `relationship.list`, `translation.list`, `model.validate`, `model.analyze`, `model.storage_stats`, `dax.validate`, `dax.validate_query`, `m.validate`, `m.complete`, `m.signature`, `m.hover`, `dax.evaluate`, `report.summary`, `report.validate`, `report.check`, `report.extract`, `report_measure.list`, `report.custom_visuals`, `visual.props`, `visual.options`, `theme.get`, `edit.history`, `view.interactions` | allowed |
| read-only unless a writing parameter is given | `report.render` (`out`), `dax.memory` (`budget`, `query_budget`), `dax.aggregations` (`enabled`, `build`), `m.format` (`apply`) | allowed without them; a run that names one (even as `null`) is refused |
| opens | `file.open` | over MCP HTTP: private to the reader (below), never on the editor session; the engine's own check (allowed only on a session without unsaved changes) still guards any other front end |
| mutating (default) | every other command: model, report, page and visual edits, `file.save`/`save_as`/`convert`/`set_cache_abf`, `project.new`, `model.new`, `model.export_vpax` and `theme.export` (they write a file), `model.refresh` (network, data snapshot), `view.interact` and `view.set_*` (the shared session's view state), the Performance analyzer's `perf.*` (the shared session's recording; `perf.refresh` evaluates afresh), `dax.clear_cache`, `edit.undo`/`redo` | refused |

A read-only command reads files only under the workspace (rule files, `cache_abf`, fonts), which a
reader may download anyway.

**Readers read saved files** (since 2026-10-28, branch `feat/viewer`; code
`crates/mcp/src/http/readview.rs`). A workspace has one editor session (the editors' MCP calls,
unsaved edits included). A reader's calls never run on those edits:

- while the editor session is clean (nothing unsaved) and has the reader's file open, the reader's
  call runs on it, under its read lock (since `feat/concurrency`, beside other reads, up to
  `--max-concurrent-reads`; an editor's command waits until the reads are over, so the check and
  the read cannot be split by an edit). No extra memory in the common case;
- otherwise the call runs on a **read-only copy** of the saved file, opened from disk and shared by
  every reader of that file (one copy per file, `--viewer-copies` at most, default 1). Its memory
  counts against `BICRAFT_MEMORY_LIMIT` like any session's. Past the cap, when every copy is in
  use, the answer is a retryable 503 (`WORKSPACE_BUSY`, `Retry-After: 2`), never the dirty editor
  session. A copy is reloaded when its file changed: every save, upload or deletion through the
  server makes copies stale, and each read compares the file's fingerprint (paths, sizes,
  modification times and, on Unix, inode numbers of the files that make up the model, the project's
  item folders and the stored-data file) with the one the copy was loaded from. Unused copies go
  after 10 minutes, and as soon as the clean editor session serves the same file;
- **a reader's `file.open` is private**: it names the file this reader (this identity: mode,
  issuer and subject) reads from now on and answers `{path, private: true, session, summary}`. It
  never touches the editor session, so it can neither discard an editor's unsaved edits nor drop
  the editor's undo history. Without a `file.open` of their own, readers read the file the workspace
  has open (its saved state; nothing saved: an empty session). A `batch` holding a reader's
  `file.open` is refused (the open runs as a call of its own).

Each copy is its own session with its own preview cache, and the editor session's cached results
are keyed by a hash of the model they were computed on, so a reader never gets a value computed on
an unsaved state or on an older saved state. After an idle close the next call (a reader's too)
reopens the file the session had open; after a restart nothing is open and readers follow whatever
an editor opens, or open a file themselves.

Over MCP HTTP:

- `tools/list` leaves out the convenience tools whose command a reader may never run
  (`add_measure`, `save`, `new_model`, `refresh_model`…); `list_commands`, `run_command` and
  `batch` stay, and `list_commands` lists only the commands a reader may run (like server-denied
  commands, which nobody sees).
- A refused call is a tool error (`isError: true`, `structuredContent.error`), the shape a
  server-denied command already has: ``"`file.save` is not available to a reader of this
  workspace: …"``.
- A `batch` with one refused step runs nothing: the answer is the failed-batch shape with
  `failed_index`, `failed_command`, `rolled_back: false` and empty `results`.
- A server without authentication (loopback only) serves every caller as an editor.

## The viewer in a browser

Status: implemented 2026-10-28 (web stage 2, viewer server mode; code `crates/mcp/src/http/viewer.rs`,
operations in `docs/kubernetes.md`, "Viewer"). `bicraft-cli serve` serves each workspace's live viewer
at `/w/<id>/view/` with the same `Host` check, authentication and ACL as the MCP endpoint (a
workspace the identity may not open: 404). Editors and readers both get the page; what each sees is
in "Roles" (readers: saved files; editors: their MCP session, unsaved edits included). Slicer clicks,
selections and page changes are view state of that page only: no role writes anything through the
viewer.

Browsers cannot attach a bearer token to a page load, so the viewer accepts two browser
credentials besides the MCP ones:

| Path | How | When |
|---|---|---|
| **Trusted identity header** (primary) | the auth proxy (oauth2-proxy, Pomerium, Authelia, Traefik forward-auth) logs the user in with its own cookie and sets the identity headers on every request, page and API calls alike | any deployment with people: SSO, MFA and session lifetime stay the identity provider's |
| **One-time login URL** | a holder of a bearer credential (an API token, or an OIDC access token from a CLI) calls `POST /w/<id>/view/login` and gets `{url, expires_in: 60}`; opening the URL (`GET /w/<id>/view/login/<code>`, single use, 60 s) sets the login cookie and redirects to the page | no proxy, `bicraft-cli serve` reached directly (not through the controller's gateway, which authenticates every request itself: there, use the proxy): an operator or an agent hands a browser session to a person |

The login cookie: `bicraft_view=<64 hex>; Path=/w/<id>/; HttpOnly; SameSite=Lax` (and `Secure` when
the public URL is `https`). It names a server-side login (256 random bits from the OS generator,
stored as its SHA-256) bound to one workspace and one identity, re-checked against the ACL on every
request; it lasts 12 h at most and ends after 2 h unused; logins are in memory, so a restart logs
everyone out. It is accepted on the viewer's routes and on `GET`/`HEAD` of the file routes (the
download link) only: never on the MCP endpoint, never for uploads or deletions. Why not the token
in the URL: a long-lived credential in a URL ends up in proxy logs and browser history; the code
is useless once redeemed and expires in a minute. Why `Lax` and not `Strict`: a login link opened
from a chat or a mail is a cross-site navigation, and a `Strict` cookie set during it is not sent
on the redirect to the page; the routes a `Lax` cookie reaches cross-site are `GET`s that change
nothing, and the `POST` routes need the CSRF checks below.

**CSRF.** Each page load gets a private view id (`?v=`, bound to the identity that loaded the
page: another identity's requests with it answer 404) and an interaction token (32 hex digits,
embedded in the page; a native client such as `bicraft` gets both from `POST …/view/api/session`
with a bearer credential or the proxy's headers, never the cookie: docs/desktop.md). `POST …/api/interact` needs the token in `X-Bicraft-Token`, and, when the
browser sends them, `Origin` equal to the public origin (`--public-url`, else `http://<Host>`) and
`Sec-Fetch-Site: same-origin`; otherwise 403. Set `--public-url` on every deployment behind an
ingress: it is the origin browsers send.

**No open-in-Desktop.** There is no route that launches anything on the server. The page offers a
download of the saved file instead (`../files/<path>` through the file route: a `.pbix`, `.pbit`
or `.bim`; a project is a folder and is listed there). An editor's unsaved edits are not in the
downloaded file, and the link says so.

## Configuration

Settings come from a JSON file (`--auth-config FILE` or `BICRAFT_AUTH_CONFIG`), then the
variables, then the flags: lists add up, a public URL from a flag beats the variable, which beats
the file. Unknown fields in the file are errors (a typo in a security setting must not pass).
Relative file names in it are relative to its folder.

| Flag | Variable | Meaning |
|---|---|---|
| `--auth-config FILE` | `BICRAFT_AUTH_CONFIG` | the JSON file below |
| `--public-url URL` | `BICRAFT_PUBLIC_URL` | `scheme://host[:port]`, no path |
| `--allow-host HOST` | `BICRAFT_ALLOWED_HOSTS` | `Host` values answered besides the public URL's |
| `--oidc-issuer URL` | `BICRAFT_OIDC_ISSUERS` | an issuer with default settings |
| `--oidc-audience AUD` | `BICRAFT_OIDC_AUDIENCES` | extra audiences of those issuers |
| `--oidc-scope S` | `BICRAFT_OIDC_SCOPES` | scopes those issuers' tokens must carry |
| `--api-tokens FILE` | `BICRAFT_API_TOKENS_FILE` | the tokens file |
| `--trusted-proxy CIDR` | `BICRAFT_TRUSTED_PROXIES` | enables the trusted-header mode |
| `--proxy-user-header H`, `--proxy-email-header H`, `--proxy-groups-header H` | | header names (an empty value turns e-mail or groups off) |
| `--grant SUBJECT=WORKSPACES` | `BICRAFT_AUTH_GRANTS` | editor grants |
| `--reader-grant SUBJECT=WORKSPACES` | `BICRAFT_AUTH_READER_GRANTS` | reader grants (same form) |

Variables holding lists accept commas or spaces. A non-loopback `--listen` needs an
authentication mode **and** `--public-url` or `--allow-host` (the `Host` names answered;
the public URL's host is added with and without its default port).

Full file:

```json
{
  "public_url": "https://bicraft.example.com",
  "scopes_supported": ["api://bicraft/BIcraft.Use"],
  "oidc": [{
    "issuer": "https://login.microsoftonline.com/<tenant-id>/v2.0",
    "audiences": ["<api-client-id>", "api://bicraft"],
    "subject_claim": "oid",
    "email_claim": "preferred_username",
    "roles_claim": "roles",
    "role_map": {"BIcraft.Editor": ["editor"]},
    "required_scopes": ["BIcraft.Use"],
    "required_claims": {"tid": "<tenant-id>"},
    "algorithms": ["RS256"],
    "leeway_secs": 60,
    "jwks_refresh_secs": 3600,
    "jwks_min_refetch_secs": 30
  }],
  "proxy": {
    "trusted": ["10.42.0.0/16"],
    "user_header": "X-Forwarded-User",
    "email_header": "X-Forwarded-Email",
    "groups_header": "X-Forwarded-Groups",
    "groups_separator": ","
  },
  "tokens_file": "/etc/bicraft/tokens.json",
  "acl": [
    {"subject": "role:editor", "workspaces": "*"},
    {"subject": "token:ci", "workspaces": ["sales"]},
    {"subject": "group:bi-viewers", "workspaces": "*", "access": "reader"}
  ]
}
```

## Provider examples

Each example assumes `--public-url https://bicraft.example.com --root /data/workspaces`. Check
the provider's current documentation for claim names; the settings below are the usual ones.

### Microsoft Entra ID

Register an API app; expose a scope (`BIcraft.Use`) and app roles (`BIcraft.Editor`); in the
manifest set `requestedAccessTokenVersion` (`accessTokenAcceptedVersion`) to `2` so access tokens
carry the v2 issuer. A v2 access token's `aud` is the API's client id. Request tokens for this API,
never for Microsoft Graph (Graph tokens are not meant to be validated by others).

```sh
bicraft-cli serve --listen 0.0.0.0:7878 --public-url https://bicraft.example.com --root /data/workspaces \
  --auth-config /etc/bicraft/entra.json
```

with the `oidc` entry of the full file above (`subject_claim: "oid"`, `scp` read automatically,
`roles` mapped), and `{"subject": "role:editor", "workspaces": "*"}`. Groups come as object ids in
`groups` when the app is configured to emit them: `group:<object-id>`.

### Google

Google's OAuth access tokens are opaque; its **ID tokens** are JWTs (issuer
`https://accounts.google.com`, `aud` = the OAuth client id or the audience requested by a service
account). Use Google directly for workloads that present ID tokens, and put humans behind an auth
proxy (oauth2-proxy with the Google provider) or a broker such as Keycloak.

```sh
bicraft-cli serve ... --oidc-issuer https://accounts.google.com \
  --oidc-audience 1234567890-abc.apps.googleusercontent.com \
  --grant 'email:ana@example.com=*'
```

To restrict to a Workspace domain, use the file: `"required_claims": {"hd": "example.com",
"email_verified": "true"}`.

### Keycloak

Issuer `https://kc.example.com/realms/<realm>`. Add an **Audience** mapper to the client scope so
access tokens carry `aud: bicraft` (by default they say `account`), and a group mapper for
`groups` if wanted.

```json
{"oidc": [{"issuer": "https://kc.example.com/realms/bi", "audiences": ["bicraft"],
           "roles_claim": "realm_access.roles", "email_claim": "email"}],
 "acl": [{"subject": "role:bicraft-editor", "workspaces": "*"}]}
```

Client roles: `"roles_claim": "resource_access.bicraft.roles"`. Keycloak also brokers SAML and
LDAP identities, so they reach BIcraft as OIDC tokens.

### Okta and Auth0

Okta (custom authorization server): issuer `https://<org>.okta.com/oauth2/default`, default
audience `api://default`, scopes in `scp` (array), a `groups` claim added in the authorization
server's claims.

```sh
bicraft-cli serve ... --oidc-issuer https://example.okta.com/oauth2/default \
  --oidc-audience api://default --oidc-scope bicraft --grant 'group:BI Team=*'
```

(quote grants with spaces.) Auth0: the issuer **ends with a slash**, kept exactly as in `iss`:
`https://<tenant>.auth0.com/`; `aud` is the API identifier; with RBAC, permissions arrive in
`permissions`:

```json
{"oidc": [{"issuer": "https://example.eu.auth0.com/", "audiences": ["https://bicraft.example.com"],
           "roles_claim": "permissions", "required_scopes": ["use:bicraft"]}],
 "acl": [{"subject": "role:edit:workspaces", "workspaces": "*"}]}
```

Custom namespaced claims work as claim names: `"groups_claim": "https://example.com/groups"`.

### oauth2-proxy (and other auth proxies)

oauth2-proxy in reverse-proxy mode sends `X-Forwarded-User`, `X-Forwarded-Email` and
`X-Forwarded-Groups` (`--pass-user-headers`, on by default): the defaults.

```sh
bicraft-cli serve --listen 0.0.0.0:7878 --allow-host bicraft.example.com --root /data/workspaces \
  --trusted-proxy 10.42.0.0/16 --grant 'group:bi-team@example.com=*'
```

Behind ingress-nginx with `auth-url` (oauth2-proxy as the auth service), the identity comes back
as `X-Auth-Request-User`/`-Email`/`-Groups` through `auth-response-headers`, and the trusted peer
is the ingress controller: `--proxy-user-header X-Auth-Request-User --proxy-email-header
X-Auth-Request-Email --proxy-groups-header X-Auth-Request-Groups`. Authelia: `Remote-User`,
`Remote-Email`, `Remote-Groups`. Pomerium: `X-Pomerium-Claim-Email` (and `X-Pomerium-Claim-Groups`
when configured). Traefik forward-auth: whatever `authResponseHeaders` copies.

### Agents and CI with API tokens

```sh
bicraft-cli serve token create --name ci --role agent   # token for the agent, entry for tokens.json
kubectl create secret generic bicraft-tokens --from-file=tokens.json
bicraft-cli serve ... --api-tokens /etc/bicraft/tokens.json --grant 'role:agent=sales,finance'
```

Mixed modes are common: OIDC for people, tokens for agents, in one file or with both flag sets.

## Security notes

- Tokens, secrets and claim values are never logged or sent back; the startup line lists the
  modes, the issuers and counts only. Failure descriptions are coarse (`token expired`, `invalid
  signature`…).
- TLS ends at the ingress; inside the cluster use a mesh or a `NetworkPolicy`. With
  `--public-url http://…` on a non-loopback address, tokens cross the network in clear: only for
  tests.
- Tests (Linux-first): `crates/auth/tests/oidc.rs` (the JWT matrix against a loopback identity
  provider with locally generated RSA, EC and Ed25519 keys: valid tokens of each type, PS256,
  expired with and without leeway, `nbf`/`iat` in the future, no `exp`, wrong `iss` and `aud`,
  audience bound to one workspace, tampered payload, forged signature, `alg: none` in any case,
  HS256 key confusion refused and explicit HS256 accepted, `crit`, unknown `kid` refetched once,
  rate-limited refetches, key rotation dropping old keys, discovery naming another issuer, several
  issuers, 20 000 mutated headers), `crates/auth/tests/modes.rs` (trusted header from trusted and
  untrusted peers, API tokens, constant-time comparison, ACL subjects, configuration errors and
  file, challenge shapes), `crates/mcp/tests/http_auth.rs` (the same over real sockets: 401/403/400
  with `WWW-Authenticate`, both metadata documents, ACL 404 identical to a missing workspace,
  duplicate credentials, non-loopback binding refused without authentication, hostile headers),
  `crates/mcp/tests/http_roles.rs` (readers: hidden tools and commands, every refusal path,
  `batch` refused whole, writing parameters, `file.open` over unsaved changes, file routes GET
  only, reads after a restart and an idle close, precedence, an editor still doing everything),
  `crates/mcp/tests/http_viewer.rs` (the viewer: private views, readers on saved files over the
  viewer and MCP, a reader's private `file.open` keeping the editor's undo history, copies reloaded
  after a save, an upload and an external change, the copy cap, the one-time login and its cookie,
  CSRF and `Host` checks, the trusted header, logs and metrics without model data),
  `crates/engine/tests/commands/roles_tests.rs` (every command classified),
  `apps/bicraft-cli/tests/serve.rs` (token helpers, flags and variables, refusals).

## Left for later

- Per-identity rate limits (stage 2); an `owner` level. (Private reads per reader: done, "Roles".)
- Token introspection (RFC 7662) for issuers with opaque access tokens; mTLS-bound tokens.
- Reloading the tokens file without a restart.
- Viewer logins persisted across restarts; a logout route.
