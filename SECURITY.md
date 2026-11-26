# Security policy

BIcraft reads files from anyone and, with `bicraft serve`, answers requests from a network. We
treat both as hostile ([AGENTS.md](AGENTS.md) §3), and we want to hear about every way around
that.

## Reporting a vulnerability

**Report it privately; do not open a public issue, discussion or pull request.**

- Use GitHub's private vulnerability reporting: the repository's **Security** tab, then
  **Report a vulnerability**. Only the maintainers see the report.
- Say what is affected (version or commit, platform, command or endpoint), how to reproduce it,
  and what an attacker gains. A file that triggers the problem helps most: make it yourself, as
  small as you can. **Never send client files or real Power BI files authored by others**, even
  privately; describe their shape instead.
- If you found the problem with a fuzzer, include the input and the target (`fuzz/`,
  [docs/fuzzing.md](docs/fuzzing.md)).

We aim to acknowledge a report within a week, agree a fix and a disclosure date with you, and
credit you in the advisory unless you prefer not to be named. Please give us a reasonable time to
ship the fix before you disclose it.

## Supported versions

BIcraft has no stable release yet. Fixes go to `main`; once releases exist, the latest one gets
security fixes.

## Scope

In scope, in decreasing order of exposure:

| Area | What counts | Where |
|---|---|---|
| The server, `bicraft serve` | anything a network client can do beyond what its role allows: escaping the workspace root (paths, symlinks, uploaded and zipped projects), running a command the server policy denies, reaching the host (processes, network) from a command, request smuggling or parser bugs in the HTTP layer, Origin and DNS-rebinding bypasses, cross-workspace access, exhausting memory, disk, threads or connections past the configured caps | `crates/mcp/src/http*`, `crates/http`, `crates/engine/src/sandbox.rs`, [docs/server-mode.md](docs/server-mode.md), [docs/web-deployment.md](docs/web-deployment.md) |
| Authentication and the workspace ACL | accepting a token, JWT or identity header that should be refused (signature, `alg`, issuer, audience, expiry, proxy address), a reader acting as an editor, a token or secret written to a log or an answer | `crates/auth`, [docs/auth.md](docs/auth.md) |
| The file readers | a crash, hang, unbounded allocation, stack overflow or out-of-bounds read on a crafted PBIX, PBIT, PBIP, PBIR, TMDL, TMSL, ABF, VertiPaq, SQLite, XPress9, ZIP, JSON, DAX or M input; a write outside the target folder | the L0–L4 crates (`crates/opc`, `xpress9`, `abf`, `vertipaq`, `sqlite`, `pbix`, `pbip`, `pbir`, `tmdl`, `tmsl`, `metadata`, `dax`, `data`) |
| The live viewer and the desktop app | the viewer's token, Origin checks and routes; a report that makes the app run code or open something without the user | `crates/view`, `crates/ui`, `apps/` |
| The Kubernetes packaging and the controller | privileges, secrets and network policies of `deploy/`, the controller's gateway | `deploy/`, `crates/controller`, [docs/kubernetes.md](docs/kubernetes.md) |
| Dependencies | a RustSec advisory that reaches BIcraft's code (`cargo deny`, `deny.toml` lists the ones we judged unreachable and why) | `Cargo.lock`, `deny.toml` |

Out of scope: Power BI Desktop and Microsoft's services (report those to Microsoft), the local
CLI and stdio MCP server doing what their user asks with that user's own rights (a local user can
already read and write their files), and findings that need a modified BIcraft binary.

A panic on hostile input is a bug even when it is not exploitable; for one that is not a security
problem, a public issue is fine.
