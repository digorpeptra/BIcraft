//! `Web.Contents(url, options)`: the bytes of an HTTP(S) resource, with the credential stored
//! for its URL (anonymous when none is, [`crate::auth`]).
//!
//! The options record (Microsoft Learn, *Web.Contents*): `RelativePath` (appended to the URL's
//! path), `Query` (a record of texts or lists of texts, added to the query string), `Headers` (a
//! record of texts), `Timeout` (a duration), `Content` (a binary: the request becomes a POST),
//! `ManualStatusHandling` (a list of status codes returned as content instead of an error),
//! `IsRetry` (true: the cache is not read) and `ExcludedFromCacheKey` (accepted; every request is
//! its own cache key), `ApiKeyName` (the query parameter a stored Web API key is sent in).

use std::time::Duration;

use bicraft_data::connect::{Arg, ConnectError};

use crate::http::{self, Client, Request, source, unsupported};

/// A `Web.Contents` call, checked.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub request: Request,
    pub manual_status: Vec<u16>,
    pub fresh: bool,
}

fn text<'a>(a: &'a Arg, what: &str) -> Result<&'a str, ConnectError> {
    match a {
        Arg::Text(t) => Ok(t),
        _ => Err(unsupported(format!("Web.Contents: `{what}` must be text"))),
    }
}

/// Texts by name from a record option (`Query`, `Headers`); a list value repeats the name.
fn pairs(a: &Arg, what: &str, lists: bool) -> Result<Vec<(String, String)>, ConnectError> {
    let Arg::Record(fields) = a else {
        return Err(unsupported(format!("Web.Contents: `{what}` must be a record")));
    };
    let mut out = Vec::with_capacity(fields.len());
    for (n, v) in fields {
        match v {
            Arg::Text(t) => out.push((n.clone(), t.clone())),
            Arg::List(items) if lists => {
                for i in items {
                    out.push((n.clone(), text(i, &format!("{what}[{n}]"))?.to_string()));
                }
            }
            _ => {
                return Err(unsupported(format!(
                    "Web.Contents: the value of `{what}[{n}]` must be text{}",
                    if lists { " or a list of texts" } else { "" }
                )));
            }
        }
    }
    Ok(out)
}

/// Checks a call's arguments.
pub fn call(args: &[Arg]) -> Result<Call, ConnectError> {
    let url = match args.first() {
        Some(Arg::Text(u)) => u.trim().to_string(),
        _ => return Err(unsupported("Web.Contents: the URL must be text")),
    };
    if args.len() > 2 {
        return Err(unsupported("Web.Contents takes a URL and an options record"));
    }
    let mut out = Call {
        request: Request {
            url,
            headers: Vec::new(),
            body: None,
            timeout: None,
            api_key_name: None,
        },
        manual_status: Vec::new(),
        fresh: false,
    };
    let mut relative = None;
    let mut query = Vec::new();
    match args.get(1) {
        None | Some(Arg::Null) => {}
        Some(Arg::Record(fields)) => {
            for (name, v) in fields {
                if matches!(v, Arg::Null) {
                    continue;
                }
                match name.as_str() {
                    "RelativePath" => relative = Some(text(v, "RelativePath")?.to_string()),
                    "Query" => query = pairs(v, "Query", true)?,
                    "Headers" => out.request.headers = pairs(v, "Headers", false)?,
                    "Timeout" => match v {
                        Arg::Duration(ms) if *ms > 0 => {
                            out.request.timeout = Some(Duration::from_millis(ms.unsigned_abs()));
                        }
                        _ => return Err(unsupported("Web.Contents: `Timeout` must be a positive duration")),
                    },
                    "Content" => match v {
                        Arg::Binary(b) => out.request.body = Some(b.clone()),
                        _ => return Err(unsupported("Web.Contents: `Content` must be a binary value")),
                    },
                    "ManualStatusHandling" => match v {
                        Arg::List(codes) => {
                            for c in codes {
                                match c {
                                    Arg::Number(n) if n.fract() == 0.0 && (100.0..=999.0).contains(n) => {
                                        out.manual_status.push(*n as u16);
                                    }
                                    _ => {
                                        return Err(unsupported(
                                            "Web.Contents: `ManualStatusHandling` must be a list of status codes",
                                        ));
                                    }
                                }
                            }
                        }
                        _ => {
                            return Err(unsupported(
                                "Web.Contents: `ManualStatusHandling` must be a list of status codes",
                            ));
                        }
                    },
                    "IsRetry" => match v {
                        Arg::Logical(b) => out.fresh = *b,
                        _ => return Err(unsupported("Web.Contents: `IsRetry` must be true or false")),
                    },
                    "ExcludedFromCacheKey" => {}
                    "ApiKeyName" => {
                        let name = text(v, "ApiKeyName")?;
                        if name.is_empty() || name.len() > 256 {
                            return Err(unsupported("Web.Contents: `ApiKeyName` must be a parameter name"));
                        }
                        out.request.api_key_name = Some(name.to_string());
                    }
                    other => {
                        return Err(unsupported(format!(
                            "Web.Contents: the option `{other}` is not supported by bicraft"
                        )));
                    }
                }
            }
        }
        Some(_) => return Err(unsupported("Web.Contents: the options must be a record")),
    }
    http::check_url(&out.request.url)?;
    if let Some(r) = relative {
        out.request.url = http::join_path(&out.request.url, &r);
    }
    out.request.url = http::with_query(&out.request.url, &query);
    http::check_url(&out.request.url)?;
    http::check_headers(&out.request.headers)?;
    Ok(out)
}

/// Runs a checked call: the body of a successful response (or of a status the call handles
/// itself); any other status is a catchable `DataSource.Error`, as in Power Query.
pub fn run(client: &Client, c: &Call, max_bytes: usize) -> Result<Vec<u8>, ConnectError> {
    let r = client.send(&c.request, max_bytes, c.fresh)?;
    if (200..300).contains(&r.status) || c.manual_status.contains(&r.status) {
        return Ok(r.body.to_vec());
    }
    Err(source(format!(
        "Web.Contents failed to get contents from '{}' ({}): {}",
        http::clip(&c.request.url),
        r.status,
        r.reason
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(f: Vec<(&str, Arg)>) -> Arg {
        Arg::Record(f.into_iter().map(|(n, v)| (n.to_string(), v)).collect())
    }

    #[test]
    fn options() {
        let c = call(&[
            Arg::Text("https://api.example.com/v1".into()),
            rec(vec![
                ("RelativePath", Arg::Text("orders".into())),
                (
                    "Query",
                    rec(vec![
                        ("top", Arg::Text("10".into())),
                        (
                            "tag",
                            Arg::List(vec![Arg::Text("a".into()), Arg::Text("b c".into())]),
                        ),
                    ]),
                ),
                (
                    "Headers",
                    rec(vec![("Accept", Arg::Text("application/json".into()))]),
                ),
                ("Timeout", Arg::Duration(5_000)),
                ("ManualStatusHandling", Arg::List(vec![Arg::Number(404.0)])),
            ]),
        ])
        .unwrap();
        assert_eq!(
            c.request.url,
            "https://api.example.com/v1/orders?top=10&tag=a&tag=b%20c"
        );
        assert_eq!(c.request.timeout, Some(Duration::from_secs(5)));
        assert_eq!(c.manual_status, [404]);
        for bad in [
            vec![Arg::Number(1.0)],
            vec![Arg::Text("file:///etc/passwd".into())],
            vec![
                Arg::Text("https://h/".into()),
                rec(vec![("ApiKeyName", Arg::Number(1.0))]),
            ],
            vec![
                Arg::Text("https://h/".into()),
                rec(vec![("Bogus", Arg::Null), ("Other", Arg::Logical(true))]),
            ],
            vec![
                Arg::Text("https://h/".into()),
                rec(vec![("Content", Arg::Text("x".into()))]),
            ],
            vec![
                Arg::Text("https://h/".into()),
                rec(vec![("Headers", rec(vec![("Host", Arg::Text("x".into()))]))]),
            ],
        ] {
            assert!(call(&bad).is_err(), "{bad:?}");
        }
    }
}
