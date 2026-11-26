//! Trusted identity headers, API tokens, the ACL, mixed modes, challenges and the configuration
//! file.

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;

use std::collections::HashMap;
use std::net::IpAddr;

use bicraft_auth::tokens::{self, entry_json};
use bicraft_auth::{
    Access, AuthConfig, Authenticator, Cidr, Denial, Grant, Identity, IssuerConfig, ProxyConfig, Source,
};
use serde_json::json;
use tempdir::TempDir;

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

fn run(a: &Authenticator, peer: &str, headers: &[(&str, &str)]) -> Result<Identity, Denial> {
    let map: HashMap<String, String> = headers
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), (*v).to_string()))
        .collect();
    a.authenticate(
        ip(peer),
        |n| map.get(&n.to_ascii_lowercase()).map(String::as_str),
        &[],
    )
}

fn grants(list: &[&str]) -> bicraft_auth::Acl {
    bicraft_auth::Acl {
        grants: list.iter().map(|g| Grant::parse_text(g).unwrap()).collect(),
    }
}

fn proxy_auth() -> Authenticator {
    let mut p = ProxyConfig::new(vec![
        Cidr::parse("10.0.0.0/8").unwrap(),
        Cidr::parse("::1").unwrap(),
    ]);
    p.groups_separator = ",".into();
    Authenticator::new(AuthConfig {
        proxy: Some(p),
        acl: grants(&["group:bi-team=*", "email:ana@example.com=sales"]),
        ..AuthConfig::default()
    })
    .unwrap()
}

#[test]
fn trusted_header_only_from_trusted_peers() {
    let a = proxy_auth();
    let h = [
        ("X-Forwarded-User", "ana"),
        ("X-Forwarded-Email", "Ana@Example.com"),
        ("X-Forwarded-Groups", "bi-team, finance ,"),
    ];
    let id = run(&a, "10.1.2.3", &h).unwrap();
    assert_eq!(id.source, Source::Proxy);
    assert_eq!(id.subject, "ana");
    assert_eq!(id.groups, ["bi-team", "finance"]);
    assert_eq!(a.access(&id, "anything"), Some(Access::Editor));
    // IPv4-mapped peers of a dual-stack socket and the IPv6 loopback
    run(&a, "::ffff:10.9.9.9", &h).unwrap();
    run(&a, "::1", &h).unwrap();
    // anyone else sending the header is refused, not ignored
    for peer in ["192.168.1.1", "11.0.0.1", "::2"] {
        assert_eq!(
            run(&a, peer, &h),
            Err(Denial::InvalidRequest(
                "identity header from an untrusted address"
            )),
            "{peer}"
        );
    }
    // empty or control characters
    assert!(matches!(
        run(&a, "10.0.0.1", &[("X-Forwarded-User", " ")]),
        Err(Denial::InvalidRequest(_))
    ));
    assert!(matches!(
        run(&a, "10.0.0.1", &[("X-Forwarded-User", "a\u{1}b")]),
        Err(Denial::InvalidRequest(_))
    ));
    // a trusted proxy without the header: no credentials
    assert_eq!(run(&a, "10.0.0.1", &[]), Err(Denial::Missing));
    // the e-mail grant (case-insensitive) opens one workspace only
    let ana = run(
        &a,
        "10.0.0.1",
        &[
            ("X-Forwarded-User", "ana"),
            ("X-Forwarded-Email", "ANA@example.com"),
        ],
    )
    .unwrap();
    assert_eq!(a.access(&ana, "sales"), Some(Access::Editor));
    assert_eq!(a.access(&ana, "hr"), None);
}

#[test]
fn api_tokens() {
    let t1 = tokens::generate().unwrap();
    let t2 = tokens::generate().unwrap();
    let mut c = AuthConfig {
        acl: grants(&["token:ci=sales", "role:admin=*"]),
        ..AuthConfig::default()
    };
    let doc = json!({"tokens": [
        entry_json("ci", &tokens::hash_token(&t1).unwrap(), &[], &[]),
        entry_json("ops", &tokens::hash_token(&t2).unwrap(), &["admin".into()], &[]),
    ]});
    c.add_tokens(tokens::parse(&doc).unwrap()).unwrap();
    let a = Authenticator::new(c).unwrap();
    let bearer = |t: &str| format!("Bearer {t}");
    let ci = run(&a, "198.51.100.1", &[("Authorization", &bearer(&t1))]).unwrap();
    assert_eq!((ci.source.clone(), ci.subject.as_str()), (Source::Token, "ci"));
    assert_eq!(a.access(&ci, "sales"), Some(Access::Editor));
    assert_eq!(a.access(&ci, "hr"), None);
    let ops = run(&a, "198.51.100.1", &[("Authorization", &bearer(&t2))]).unwrap();
    assert_eq!(a.access(&ops, "hr"), Some(Access::Editor));
    // one character off, a prefix, a longer token, another scheme
    let mut wrong = t1.clone();
    wrong.pop();
    wrong.push(if t1.ends_with('A') { 'B' } else { 'A' });
    for bad in [wrong.as_str(), &t1[..20], &format!("{t1}x"), "bct_"] {
        assert_eq!(
            run(&a, "198.51.100.1", &[("Authorization", &bearer(bad))]),
            Err(Denial::InvalidToken("unknown token"))
        );
    }
    assert_eq!(
        run(&a, "198.51.100.1", &[("Authorization", &format!("Basic {t1}"))]),
        Err(Denial::Missing)
    );
    assert_eq!(run(&a, "198.51.100.1", &[]), Err(Denial::Missing));
    // a `user:ci` grant does not match the token named ci, and `token:` does not match users
    let users_only = grants(&["user:ci=*"]);
    assert_eq!(users_only.access(&ci, "sales"), None);
}

#[test]
fn constant_time_comparison_checks_every_byte() {
    // functional guarantees of the comparison used for token hashes (timing itself is not
    // measurable reliably in a unit test; the loop has no early exit by construction)
    let a = [7u8; 32];
    for i in 0..32 {
        let mut b = a;
        b[i] ^= 1;
        assert!(!tokens::constant_time_eq(&a, &b), "{i}");
    }
    assert!(tokens::constant_time_eq(&a, &a));
    assert!(!tokens::constant_time_eq(&a, &a[..31]));
}

#[test]
fn acl_subjects_and_parsing() {
    let id = Identity {
        source: Source::Oidc {
            issuer: "https://idp.example".into(),
        },
        subject: "u1".into(),
        email: Some("ana@example.com".into()),
        groups: vec!["cn=bi,ou=groups".into()],
        roles: vec!["editor".into()],
        scopes: vec![],
    };
    let acl = grants(&[
        "user:u1=a",
        "group:cn=bi,ou=groups=b,c",
        "role:editor=d",
        "issuer:https://idp.example=e",
        "email:ANA@example.com=f",
        "user:other=g",
    ]);
    for ws in ["a", "b", "c", "d", "e", "f"] {
        assert_eq!(acl.access(&id, ws), Some(Access::Editor), "{ws}");
    }
    assert_eq!(acl.access(&id, "g"), None);
    assert_eq!(acl.access(&id, "z"), None);
    assert_eq!(grants(&["*=*"]).access(&id, "z"), Some(Access::Editor));
    for bad in [
        "nobody",
        "group:x=",
        "x:y=*",
        "user:=*",
        "group:x=../a",
        "group:x=.h",
    ] {
        assert!(Grant::parse_text(bad).is_err(), "{bad}");
    }
    let r = Grant::parse_json(&json!({"subject": "group:x", "workspaces": "*", "access": "reader"})).unwrap();
    assert_eq!(r.access, Access::Reader);
    assert!(
        Grant::parse_json(&json!({"subject": "group:x", "workspaces": "*", "access": "owner"}))
            .unwrap_err()
            .contains("reader, editor")
    );
    // precedence: the highest access among the matching grants, whatever their order or kind
    let mut mixed = grants(&["role:editor=d"]);
    for (text, access) in [
        ("*=*", Access::Reader),
        ("email:ana@example.com=d,h", Access::Reader),
    ] {
        mixed
            .grants
            .insert(0, Grant::parse_text_as(text, access).unwrap());
    }
    assert_eq!(mixed.access(&id, "d"), Some(Access::Editor));
    assert_eq!(mixed.access(&id, "h"), Some(Access::Reader));
    assert_eq!(mixed.access(&id, "z"), Some(Access::Reader));
    let reader_only = bicraft_auth::Acl {
        grants: vec![Grant::parse_text_as("user:u1=a", Access::Reader).unwrap()],
    };
    assert_eq!(reader_only.access(&id, "a"), Some(Access::Reader));
    assert_eq!(reader_only.access(&id, "b"), None);
    assert!(Grant::parse_text_as("nobody", Access::Reader).is_err());
    assert!(Grant::parse_json(&json!({"subject": "group:x", "workspaces": ["a"], "role": "x"})).is_err());
    let g = Grant::parse_json(&json!({"subject": "group:x", "workspaces": ["a", "b"]})).unwrap();
    assert_eq!(g.access, Access::Editor);
}

#[test]
fn configuration_errors() {
    // no mode, no grant
    assert!(
        Authenticator::new(AuthConfig::default())
            .unwrap_err()
            .contains("no authentication")
    );
    let tokens_only = |acl| AuthConfig {
        tokens: tokens::parse(&json!([entry_json("ci", &"0".repeat(64), &[], &[])])).unwrap(),
        acl,
        ..AuthConfig::default()
    };
    assert!(
        Authenticator::new(tokens_only(grants(&[])))
            .unwrap_err()
            .contains("no ACL grant")
    );
    let mut c = tokens_only(grants(&["*=*"]));
    c.public_url = Some("https://bi.example.com/mcp".into());
    assert!(Authenticator::new(c).unwrap_err().contains("no path"));
    // http issuers only on loopback
    let mut c = tokens_only(grants(&["*=*"]));
    c.issuers.push(IssuerConfig::new("http://idp.example.com"));
    assert!(Authenticator::new(c).unwrap_err().contains("https"));
    // a proxy needs trusted addresses and sane header names
    let mut c = tokens_only(grants(&["*=*"]));
    c.proxy = Some(ProxyConfig::new(vec![]));
    assert!(Authenticator::new(c).is_err());
    let mut c = tokens_only(grants(&["*=*"]));
    let mut p = ProxyConfig::new(vec![Cidr::parse("10.0.0.1").unwrap()]);
    p.user_header = "Authorization".into();
    c.proxy = Some(p);
    assert!(Authenticator::new(c).is_err());
}

#[test]
fn challenges_follow_rfc_6750_and_9728() {
    let mut c = AuthConfig {
        public_url: Some("https://bi.example.com".into()),
        scopes_supported: vec!["bicraft".into()],
        acl: grants(&["*=*"]),
        ..AuthConfig::default()
    };
    c.issuers
        .push(IssuerConfig::new("https://login.example.com/tenant/v2.0"));
    let a = Authenticator::new(c).unwrap();
    let md = "https://bi.example.com/.well-known/oauth-protected-resource/w/a/mcp";
    assert_eq!(
        a.challenge(&Denial::Missing, Some(md)),
        (
            401,
            format!("Bearer resource_metadata=\"{md}\", scope=\"bicraft\"")
        )
    );
    assert_eq!(
        a.challenge(&Denial::InvalidToken("token expired"), Some(md)),
        (
            401,
            format!(
                "Bearer resource_metadata=\"{md}\", error=\"invalid_token\", error_description=\"token expired\", scope=\"bicraft\""
            )
        )
    );
    let (status, h) = a.challenge(
        &Denial::InsufficientScope(vec!["bicraft".into(), "x".into()]),
        Some(md),
    );
    assert_eq!(status, 403);
    assert!(h.starts_with(&format!(
        "Bearer resource_metadata=\"{md}\", error=\"insufficient_scope\", scope=\"bicraft x\""
    )));
    assert_eq!(
        a.challenge(
            &Denial::InvalidRequest("malformed Authorization header"),
            Some(md)
        )
        .0,
        400
    );
    let doc = a.resource_metadata("https://bi.example.com/w/a/mcp").unwrap();
    assert_eq!(doc["resource"], "https://bi.example.com/w/a/mcp");
    assert_eq!(
        doc["authorization_servers"],
        json!(["https://login.example.com/tenant/v2.0"])
    );
    assert_eq!(doc["scopes_supported"], json!(["bicraft"]));
    assert_eq!(doc["bearer_methods_supported"], json!(["header"]));
    // without OIDC: a plain realm, no metadata
    let t = Authenticator::new(AuthConfig {
        tokens: tokens::parse(&json!([entry_json("ci", &"0".repeat(64), &[], &[])])).unwrap(),
        acl: grants(&["*=*"]),
        ..AuthConfig::default()
    })
    .unwrap();
    assert_eq!(
        t.challenge(&Denial::Missing, Some(md)),
        (401, "Bearer realm=\"bicraft\"".into())
    );
    assert!(t.resource_metadata("https://x").is_none());
}

#[test]
fn configuration_file() {
    let dir = TempDir::new("auth-config");
    let token = tokens::generate().unwrap();
    let tokens_doc =
        json!({"tokens": [entry_json("ci", &tokens::hash_token(&token).unwrap(), &["agent".into()], &[])]});
    std::fs::write(dir.path().join("tokens.json"), tokens_doc.to_string()).unwrap();
    std::fs::write(dir.path().join("hs.key"), "0123456789abcdef0123456789abcdef\n").unwrap();
    let doc = json!({
        "public_url": "https://bi.example.com",
        "oidc": [
            {"issuer": "https://login.microsoftonline.com/tid/v2.0", "audiences": ["api://bicraft"],
             "subject_claim": "oid", "email_claim": "preferred_username", "roles_claim": "roles",
             "role_map": {"BIcraft.Editor": ["editor"]}, "required_scopes": ["bicraft.use"],
             "required_claims": {"tid": "tid"}},
            {"issuer": "http://127.0.0.1:8080/realms/bi", "roles_claim": "realm_access.roles",
             "groups_claim": null, "algorithms": ["HS256"], "hmac_secret_file": "hs.key", "advertise": false}
        ],
        "proxy": {"trusted": ["10.0.0.0/8"], "user_header": "Remote-User", "groups_header": "Remote-Groups",
                  "email_header": null},
        "tokens_file": "tokens.json",
        "acl": [{"subject": "role:editor", "workspaces": "*"}, {"subject": "token:ci", "workspaces": ["sales"]}]
    });
    let path = dir.path().join("auth.json");
    std::fs::write(&path, doc.to_string()).unwrap();
    let c = AuthConfig::from_file(&path).unwrap();
    assert_eq!(c.issuers.len(), 2);
    assert_eq!(c.issuers[0].subject_claim, "oid");
    assert_eq!(c.issuers[1].groups_claim, None);
    assert_eq!(c.issuers[1].hmac_secret.as_ref().unwrap().0.len(), 32);
    assert_eq!(c.proxy.as_ref().unwrap().email_header, None);
    assert_eq!(c.tokens.len(), 1);
    assert_eq!(c.acl.grants.len(), 2);
    let a = Authenticator::new(c).unwrap();
    assert!(a.summary().contains("1 API token(s)"));
    assert!(!a.summary().contains(&token));
    // only advertised issuers are listed in the metadata
    assert_eq!(
        a.resource_metadata("https://bi.example.com").unwrap()["authorization_servers"],
        json!(["https://login.microsoftonline.com/tid/v2.0"])
    );
    let id = run(
        &a,
        "203.0.113.1",
        &[("Authorization", &format!("Bearer {token}"))],
    )
    .unwrap();
    assert_eq!(id.roles, ["agent"]);
    // unknown fields are errors (a typo in a security setting must not pass silently)
    for bad in [
        json!({"oidc": [{"issuer": "https://x.example", "audience": "a"}]}),
        json!({"acls": []}),
        json!({"proxy": {"trusted": ["10.0.0.1"], "user": "x"}}),
        json!({"oidc": [{"issuer": "https://x.example", "algorithms": ["none"]}]}),
    ] {
        let mut c = AuthConfig::default();
        assert!(c.merge_json(&bad, dir.path()).is_err(), "{bad}");
    }
}
