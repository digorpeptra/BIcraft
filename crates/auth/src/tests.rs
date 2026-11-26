use super::*;

#[test]
fn bearer_header_forms() {
    assert!(matches!(
        bearer("Bearer abc.def-_~+/="),
        Bearer::Token("abc.def-_~+/=")
    ));
    assert!(matches!(bearer("bearer   abc"), Bearer::Token("abc")));
    assert!(matches!(bearer("BEARER\tabc"), Bearer::Token("abc")));
    assert!(matches!(bearer("Basic dXNlcjpwYXNz"), Bearer::OtherScheme));
    assert!(matches!(bearer("Bearer"), Bearer::Malformed));
    assert!(matches!(bearer("Bearer a b"), Bearer::Malformed));
    assert!(matches!(bearer("Bearer a,b"), Bearer::Malformed));
    assert!(matches!(bearer("Bearer é"), Bearer::Malformed));
    assert!(matches!(bearer(""), Bearer::OtherScheme));
}

#[test]
fn quoting_and_scopes() {
    assert_eq!(quote("a\"b\\c\r\nd"), "a\\\"b\\\\cd");
    assert!(scope_token("files:read"));
    assert!(!scope_token("a b"));
    assert!(!scope_token("a\"b"));
    assert!(!scope_token(""));
    assert!(valid_origin("https://bi.example.com"));
    assert!(valid_origin("http://127.0.0.1:7878"));
    assert!(!valid_origin("https://bi.example.com/mcp"));
    assert!(!valid_origin("bi.example.com"));
    assert!(!valid_origin("ftp://x"));
    assert!(!valid_origin("https://"));
}

#[test]
fn secrets_are_never_printed() {
    let s = Secret(b"super-secret-value".to_vec());
    assert!(!format!("{s:?}").contains("super"));
}
