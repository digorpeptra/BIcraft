//! Authentication of the MCP Streamable HTTP endpoint (docs/auth.md), over real sockets: the 401
//! challenge and the protected resource metadata (MCP authorization, RFC 9728, RFC 6750), OIDC
//! bearer JWTs from a loopback identity provider, API tokens, the trusted identity header from
//! trusted and untrusted peers, the workspace ACL (a closed workspace looks missing), duplicate
//! credentials, non-loopback binding, and hostile `Authorization` headers.

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;

#[path = "../../../testsupport/idp.rs"]
mod idp;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use bicraft_auth::tokens::{self, entry_json};
use bicraft_auth::{AuthConfig, Authenticator, Cidr, Grant, IssuerConfig, ProxyConfig};
use bicraft_mcp::http::{HTTP_PROTOCOL_VERSION, UNAUTHORIZED, WORKSPACE_NOT_FOUND};
use bicraft_mcp::{HostPolicy, HttpConfig, HttpServer};
use idp::{Idp, Key, now};
use serde_json::{Value, json};
use tempdir::TempDir;

struct Fixture {
    _dir: TempDir,
    server: HttpServer,
}

fn fixture(name: &str, auth: AuthConfig) -> Fixture {
    let dir = TempDir::new(name);
    for ws in ["a", "b"] {
        std::fs::create_dir_all(dir.path().join(ws)).unwrap();
    }
    let config = HttpConfig {
        auth: Some(Arc::new(Authenticator::new(auth).unwrap())),
        ..HttpConfig::default()
    };
    let server = HttpServer::start("127.0.0.1:0".parse().unwrap(), dir.path(), config).unwrap();
    Fixture { _dir: dir, server }
}

#[derive(Debug)]
struct Response {
    status: u16,
    head: String,
    body: String,
}

impl Response {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().find_map(|l| {
            let (n, v) = l.split_once(':')?;
            n.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
    }
}

fn raw(addr: SocketAddr, bytes: &[u8]) -> Response {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    s.write_all(bytes).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    Response {
        status: head.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0),
        head: head.to_string(),
        body: body.to_string(),
    }
}

/// A `tools/list` request to workspace `ws` with extra headers.
fn list_bytes(ws: &str, extra: &[(&str, &str)]) -> Vec<u8> {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {"_meta": {
        "io.modelcontextprotocol/protocolVersion": HTTP_PROTOCOL_VERSION,
        "io.modelcontextprotocol/clientCapabilities": {}}}})
    .to_string();
    let mut req = format!(
        "POST /w/{ws}/mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Accept: application/json\r\nMCP-Protocol-Version: {HTTP_PROTOCOL_VERSION}\r\n\
         Mcp-Method: tools/list\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (n, v) in extra {
        req.push_str(&format!("{n}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(&body);
    req.into_bytes()
}

fn list(addr: SocketAddr, ws: &str, extra: &[(&str, &str)]) -> Response {
    raw(addr, &list_bytes(ws, extra))
}

fn get(addr: SocketAddr, path: &str) -> Response {
    raw(
        addr,
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes(),
    )
}

fn grants(list: &[&str]) -> bicraft_auth::Acl {
    bicraft_auth::Acl {
        grants: list.iter().map(|g| Grant::parse_text(g).unwrap()).collect(),
    }
}

fn token_config(token: &str, acl: &[&str]) -> AuthConfig {
    let mut c = AuthConfig {
        acl: grants(acl),
        ..AuthConfig::default()
    };
    let doc = json!([entry_json("ci", &tokens::hash_token(token).unwrap(), &[], &[])]);
    c.add_tokens(tokens::parse(&doc).unwrap()).unwrap();
    c
}

#[test]
fn api_tokens_and_the_acl() {
    let token = tokens::generate().unwrap();
    let f = fixture("mcpauth-tokens", token_config(&token, &["token:ci=a"]));
    let addr = f.server.addr();
    let bearer = format!("Bearer {token}");
    // no credentials: 401 with a plain challenge (no OIDC: no resource metadata)
    let r = list(addr, "a", &[]);
    assert_eq!(r.status, 401, "{}", r.body);
    assert_eq!(
        r.header("WWW-Authenticate").as_deref(),
        Some("Bearer realm=\"bicraft\"")
    );
    assert_eq!(r.json()["error"]["code"], UNAUTHORIZED);
    assert!(r.json().get("id").is_none());
    // wrong token
    let r = list(
        addr,
        "a",
        &[(
            "Authorization",
            "Bearer bct_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        )],
    );
    assert_eq!(r.status, 401);
    assert!(
        r.header("WWW-Authenticate")
            .unwrap()
            .contains("error=\"invalid_token\"")
    );
    // the right token
    let r = list(addr, "a", &[("Authorization", &bearer)]);
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.json()["result"]["tools"].is_array());
    // workspace b exists but is closed to this token: answered exactly like a missing one
    let closed = list(addr, "b", &[("Authorization", &bearer)]);
    let missing = list(addr, "nope", &[("Authorization", &bearer)]);
    assert_eq!(closed.status, 404);
    assert_eq!(closed.body, missing.body);
    assert_eq!(closed.json()["error"]["code"], WORKSPACE_NOT_FOUND);
    // unauthenticated requests learn nothing about workspaces either
    assert_eq!(list(addr, "nope", &[]).status, 401);
    assert_eq!(list(addr, "b", &[]).status, 401);
    // GET and DELETE need credentials too, then answer 405
    let r = raw(addr, b"GET /w/a/mcp HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
    assert_eq!(r.status, 401);
    // no metadata without OIDC
    assert_eq!(get(addr, "/.well-known/oauth-protected-resource").status, 404);
    // duplicate credentials are refused before anything else
    let r = list(
        addr,
        "a",
        &[("Authorization", &bearer), ("Authorization", &bearer)],
    );
    assert_eq!(r.status, 400);
    // another scheme is "no credentials"; a malformed bearer is 400 invalid_request
    assert_eq!(list(addr, "a", &[("Authorization", "Basic YTpi")]).status, 401);
    let r = list(addr, "a", &[("Authorization", "Bearer a b")]);
    assert_eq!(r.status, 400);
    assert!(r.header("WWW-Authenticate").unwrap().contains("invalid_request"));
}

fn oidc_config(idp: &Idp, scopes: &[&str]) -> AuthConfig {
    let mut ic = IssuerConfig::new(idp.issuer.clone());
    ic.required_scopes = scopes.iter().map(|s| (*s).to_string()).collect();
    ic.jwks_min_refetch_secs = 0;
    AuthConfig {
        issuers: vec![ic],
        acl: grants(&["group:bi-team=a"]),
        ..AuthConfig::default()
    }
}

#[test]
fn oidc_challenge_metadata_and_tokens() {
    let key = Key::ec("k1");
    let idp = Idp::start(&[&key]);
    let f = fixture("mcpauth-oidc", oidc_config(&idp, &["bicraft"]));
    let addr = f.server.addr();
    let resource = "http://127.0.0.1/w/a/mcp";
    let metadata = "http://127.0.0.1/.well-known/oauth-protected-resource/w/a/mcp";
    // 401 names the resource metadata and the scopes (MCP authorization, RFC 9728 §5.1)
    let r = list(addr, "a", &[]);
    assert_eq!(r.status, 401);
    assert_eq!(
        r.header("WWW-Authenticate").as_deref(),
        Some(format!("Bearer resource_metadata=\"{metadata}\", scope=\"bicraft\"").as_str())
    );
    // the metadata: path-suffixed and root (RFC 9728 §3), public, cross-origin readable
    let m = get(addr, "/.well-known/oauth-protected-resource/w/a/mcp");
    assert_eq!(m.status, 200, "{}", m.body);
    assert_eq!(m.header("Access-Control-Allow-Origin").as_deref(), Some("*"));
    let doc = m.json();
    assert_eq!(doc["resource"], resource);
    assert_eq!(doc["authorization_servers"], json!([idp.issuer]));
    assert_eq!(doc["scopes_supported"], json!(["bicraft"]));
    assert_eq!(doc["bearer_methods_supported"], json!(["header"]));
    let root = get(addr, "/.well-known/oauth-protected-resource").json();
    assert_eq!(root["resource"], "http://127.0.0.1");
    // any well-formed id gets a document (no existence oracle); malformed paths are 404
    assert_eq!(
        get(addr, "/.well-known/oauth-protected-resource/w/zzz/mcp").status,
        200
    );
    assert_eq!(
        get(addr, "/.well-known/oauth-protected-resource/w/../mcp").status,
        404
    );
    assert_eq!(get(addr, "/.well-known/oauth-protected-resource/x").status, 404);
    let r = raw(
        addr,
        b"POST /.well-known/oauth-protected-resource HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\n\r\n",
    );
    assert_eq!(r.status, 405);
    // a token for this workspace's resource URL (RFC 8707)
    let good = key.token(&idp.claims(resource));
    let r = list(addr, "a", &[("Authorization", &format!("Bearer {good}"))]);
    assert_eq!(r.status, 200, "{}", r.body);
    // a token for the server's origin works too; a token for another workspace does not
    let origin = key.token(&idp.claims("http://127.0.0.1"));
    assert_eq!(
        list(addr, "a", &[("Authorization", &format!("Bearer {origin}"))]).status,
        200
    );
    let other = key.token(&idp.claims("http://127.0.0.1/w/b/mcp"));
    let r = list(addr, "a", &[("Authorization", &format!("Bearer {other}"))]);
    assert_eq!(r.status, 401);
    let challenge = r.header("WWW-Authenticate").unwrap();
    assert!(challenge.contains("error=\"invalid_token\""), "{challenge}");
    assert!(
        challenge.contains(&format!("resource_metadata=\"{metadata}\"")),
        "{challenge}"
    );
    // expired
    let mut c = idp.claims(resource);
    c["exp"] = json!(now() - 3600);
    let r = list(
        addr,
        "a",
        &[("Authorization", &format!("Bearer {}", key.token(&c)))],
    );
    assert_eq!(r.status, 401);
    assert!(r.header("WWW-Authenticate").unwrap().contains("token expired"));
    // missing scope: 403 insufficient_scope with the scopes needed
    let mut c = idp.claims(resource);
    c["scope"] = json!("openid");
    let r = list(
        addr,
        "a",
        &[("Authorization", &format!("Bearer {}", key.token(&c)))],
    );
    assert_eq!(r.status, 403);
    let challenge = r.header("WWW-Authenticate").unwrap();
    assert!(challenge.contains("error=\"insufficient_scope\""), "{challenge}");
    assert!(challenge.contains("scope=\"bicraft\""), "{challenge}");
    // a user outside the granted group: the workspace looks missing
    let mut c = idp.claims(resource);
    c["groups"] = json!(["finance"]);
    let r = list(
        addr,
        "a",
        &[("Authorization", &format!("Bearer {}", key.token(&c)))],
    );
    assert_eq!(r.status, 404);
    // the tools work with the token
    let body = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
        "name": "list_commands", "arguments": {}, "_meta": {
        "io.modelcontextprotocol/protocolVersion": HTTP_PROTOCOL_VERSION,
        "io.modelcontextprotocol/clientCapabilities": {}}}})
    .to_string();
    let req = format!(
        "POST /w/a/mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nAccept: application/json\r\n\
         MCP-Protocol-Version: {HTTP_PROTOCOL_VERSION}\r\nMcp-Method: tools/call\r\nMcp-Name: list_commands\r\n\
         Authorization: Bearer {good}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let r = raw(addr, req.as_bytes());
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["result"]["isError"], false, "{}", r.body);
}

#[test]
fn trusted_header_from_trusted_and_untrusted_peers() {
    let proxy = |cidr: &str| AuthConfig {
        proxy: Some(ProxyConfig::new(vec![Cidr::parse(cidr).unwrap()])),
        acl: grants(&["group:bi-team=a"]),
        ..AuthConfig::default()
    };
    // the test client connects from 127.0.0.1: a trusted proxy
    let f = fixture("mcpauth-proxy", proxy("127.0.0.0/8"));
    let addr = f.server.addr();
    let ok = list(
        addr,
        "a",
        &[("X-Forwarded-User", "ana"), ("X-Forwarded-Groups", "bi-team")],
    );
    assert_eq!(ok.status, 200, "{}", ok.body);
    assert_eq!(list(addr, "a", &[("X-Forwarded-User", "bob")]).status, 404);
    assert_eq!(list(addr, "a", &[]).status, 401);
    // a repeated identity header is refused
    let r = list(
        addr,
        "a",
        &[
            ("X-Forwarded-User", "ana"),
            ("X-Forwarded-User", "admin"),
            ("X-Forwarded-Groups", "bi-team"),
        ],
    );
    assert_eq!(r.status, 400);
    // the same headers from an address that is not a trusted proxy: refused, not ignored
    let f = fixture("mcpauth-proxy-untrusted", proxy("10.0.0.0/8"));
    let r = list(
        f.server.addr(),
        "a",
        &[("X-Forwarded-User", "ana"), ("X-Forwarded-Groups", "bi-team")],
    );
    assert_eq!(r.status, 400, "{}", r.body);
    assert!(
        r.header("WWW-Authenticate")
            .unwrap()
            .contains("untrusted address")
    );
}

#[test]
fn mixed_modes() {
    let key = Key::rsa("r1");
    let idp = Idp::start(&[&key]);
    let token = tokens::generate().unwrap();
    let mut c = oidc_config(&idp, &[]);
    c.add_tokens(token_config(&token, &[]).tokens).unwrap();
    c.acl.grants.push(Grant::parse_text("token:ci=*").unwrap());
    let f = fixture("mcpauth-mixed", c);
    let addr = f.server.addr();
    // a token for the server origin: the group opens a, not b
    let jwt = key.token(&idp.claims("http://127.0.0.1"));
    assert_eq!(
        list(addr, "a", &[("Authorization", &format!("Bearer {jwt}"))]).status,
        200
    );
    assert_eq!(
        list(addr, "b", &[("Authorization", &format!("Bearer {jwt}"))]).status,
        404
    );
    // a token bound to workspace a is not valid for b at all
    let bound = key.token(&idp.claims("http://127.0.0.1/w/a/mcp"));
    assert_eq!(
        list(addr, "b", &[("Authorization", &format!("Bearer {bound}"))]).status,
        401
    );
    assert_eq!(
        list(addr, "b", &[("Authorization", &format!("Bearer {token}"))]).status,
        200
    );
}

#[test]
fn non_loopback_addresses_need_authentication() {
    let dir = TempDir::new("mcpauth-bind");
    let hosts = HostPolicy::Names(vec!["bi.example.com".into()]);
    // a host list alone is not enough: no authentication, no network address
    let e = HttpServer::start(
        "0.0.0.0:0".parse().unwrap(),
        dir.path(),
        HttpConfig {
            hosts: hosts.clone(),
            ..HttpConfig::default()
        },
    )
    .err()
    .unwrap();
    assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput);
    assert!(e.to_string().contains("authentication"), "{e}");
    // with authentication the address is accepted (bound only on Unix: binding every interface
    // on a Windows workstation would raise a firewall prompt)
    let auth = Arc::new(Authenticator::new(token_config(&tokens::generate().unwrap(), &["*=*"])).unwrap());
    let config = HttpConfig {
        hosts,
        auth: Some(auth),
        ..HttpConfig::default()
    };
    if cfg!(unix) {
        let server = HttpServer::start("0.0.0.0:0".parse().unwrap(), dir.path(), config).unwrap();
        let port = server.addr().port();
        // the Host policy still applies
        let r = raw(
            SocketAddr::from(([127, 0, 0, 1], port)),
            b"POST /w/a/mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\n\r\n",
        );
        assert_eq!(r.status, 403);
    } else {
        // the checks run before binding: a loopback start with the same configuration works
        HttpServer::start("127.0.0.1:0".parse().unwrap(), dir.path(), config).unwrap();
    }
}

#[test]
fn hostile_authorization_headers_never_crash() {
    let key = Key::ec("k");
    let idp = Idp::start(&[&key]);
    let token = tokens::generate().unwrap();
    let mut c = oidc_config(&idp, &[]);
    c.issuers[0].jwks_min_refetch_secs = 3600;
    c.add_tokens(token_config(&token, &[]).tokens).unwrap();
    c.acl.grants.push(Grant::parse_text("*=*").unwrap());
    let f = fixture("mcpauth-fuzz", c);
    let addr = f.server.addr();
    let good = key.token(&idp.claims("http://127.0.0.1/w/a/mcp"));
    let mut seed = 0x1234_5678_9ABC_DEF1u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for i in 0..400 {
        let mut v: Vec<u8> = format!("Bearer {good}").into_bytes();
        for _ in 0..1 + next() % 3 {
            let len = v.len().max(1);
            match next() % 4 {
                0 => {
                    let at = (next() as usize) % len;
                    if let Some(b) = v.get_mut(at) {
                        *b = b"A.-_ \t=\"\\%\x7f"[(next() % 11) as usize];
                    }
                }
                1 => v.truncate((next() as usize) % len),
                2 => v.extend(std::iter::repeat_n(b'A', (next() % 4000) as usize)),
                _ => v.insert((next() as usize) % len, b'.'),
            }
        }
        let value = String::from_utf8_lossy(&v).replace(['\r', '\n'], "");
        let r = list(addr, "a", &[("Authorization", &value)]);
        assert!(
            matches!(r.status, 200 | 400 | 401 | 431),
            "{i}: {} {}",
            r.status,
            r.body
        );
    }
    // still serving
    assert_eq!(
        list(addr, "a", &[("Authorization", &format!("Bearer {good}"))]).status,
        200
    );
    assert_eq!(
        list(addr, "a", &[("Authorization", &format!("Bearer {token}"))]).status,
        200
    );
}
