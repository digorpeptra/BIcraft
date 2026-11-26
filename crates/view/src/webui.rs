//! The browser app (docs/webui.md): the shared egui UI of `crates/ui` in its browser shell,
//! compiled to WebAssembly (`apps/bicraft-web`), served under `/app/` by `bicraft view` and,
//! per workspace, by `bicraft serve` (`/w/<id>/view/app/`), next to the HTML page.
//!
//! The page ([`app_page`]) is a canvas, a loading line and `boot.js`, which loads the module
//! wasm-bindgen generated (`bicraft_web.js`) and its WebAssembly (`bicraft_web_bg.wasm`). The
//! tokens travel as the HTML page's do: in `<meta>` slots of the page, read by the app, never put
//! in browser storage. The two generated files are embedded at build time when
//! `BICRAFT_WEBUI_DIR` is set (`build.rs`); without them [`available`] is false and the routes
//! answer 404 with [`NOT_BUNDLED`].

use std::borrow::Cow;
use std::io::{self, Read, Write};

use bicraft_http::{Status, write_response};

include!(concat!(env!("OUT_DIR"), "/webui.rs"));

const APP: &str = include_str!("webui/app.html");
const BOOT: &str = include_str!("webui/boot.js");

/// The app page's `Content-Security-Policy`: scripts from this origin only, WebAssembly
/// compilation allowed, no inline script, requests to this origin only.
pub const APP_CONTENT_SECURITY_POLICY: &str = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; \
     style-src 'unsafe-inline'; img-src data: 'self'; connect-src 'self'; base-uri 'none'; \
     form-action 'none'; frame-ancestors 'none'";

/// The answer of an app route in a build without the app.
pub const NOT_BUNDLED: &str = "This bicraft was built without the browser app. Build it with \
     `cargo xtask webui`, then build bicraft with BICRAFT_WEBUI_DIR set (docs/webui.md); the HTML \
     viewer is one level up.\n";

/// The app's static files, as served under `app/`.
pub const FILES: [&str; 3] = ["boot.js", "bicraft_web.js", "bicraft_web_bg.wasm"];

/// Whether this build embeds the app.
pub fn available() -> bool {
    BUNDLED
}

/// The sizes of the embedded files: (JavaScript, WebAssembly) uncompressed, then both gzip-ed.
pub fn sizes() -> (usize, usize, usize) {
    (JS_LEN, WASM_LEN, JS_GZ.len().saturating_add(WASM_GZ.len()))
}

fn hex32(t: &str) -> bool {
    t.len() == 32
        && t.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The app page with its tokens: the interaction token (`None`: read-only), the
/// open-in-Desktop token (`bicraft view` only) and the private view id (`bicraft serve` only).
/// Each must be 32 lowercase hex digits; anything else leaves its slot empty.
pub fn app_page(interact_token: Option<&str>, open_token: Option<&str>, view_id: Option<&str>) -> String {
    let mut page = APP.to_string();
    for (name, value) in [
        ("bicraft-interact-token", interact_token),
        ("bicraft-open-token", open_token),
        ("bicraft-view-id", view_id),
    ] {
        if let Some(v) = value.filter(|v| hex32(v)) {
            page = page.replacen(
                &format!(r#"<meta name="{name}" content="">"#),
                &format!(r#"<meta name="{name}" content="{v}">"#),
                1,
            );
        }
    }
    page
}

/// The app page `page` with the page token of the UI control channel's relay (`bicraft view
/// --control`, [`crate::control`]): the app then takes the relayed requests and posts its replies
/// with it. Anything but 32 lowercase hex digits leaves the slot empty (no relay).
pub fn with_control(page: &str, token: &str) -> String {
    if !hex32(token) {
        return page.to_string();
    }
    page.replacen(
        r#"<meta name="bicraft-control-token" content="">"#,
        &format!(r#"<meta name="bicraft-control-token" content="{token}">"#),
        1,
    )
}

/// A static file of the app, ready to send.
#[derive(Debug)]
pub struct Served {
    pub status: Status,
    pub content_type: &'static str,
    pub body: Cow<'static, [u8]>,
    /// The body is gzip-compressed (`Content-Encoding: gzip`).
    pub gzip: bool,
    pub etag: &'static str,
}

/// Whether an `Accept-Encoding` header accepts gzip (`gzip` or `*`, not with `q=0`).
pub fn accepts_gzip(accept_encoding: Option<&str>) -> bool {
    accept_encoding.is_some_and(|h| {
        h.split(',').any(|item| {
            let mut parts = item.split(';');
            let name = parts.next().unwrap_or_default().trim();
            let q_zero = parts.any(|p| {
                p.trim()
                    .strip_prefix("q=")
                    .and_then(|q| q.trim().parse::<f32>().ok())
                    .is_some_and(|q| q <= 0.0)
            });
            (name.eq_ignore_ascii_case("gzip") || name == "*") && !q_zero
        })
    })
}

/// The file `name` of [`FILES`] (`None`: not one of them, or not in this build). A matching
/// `If-None-Match` answers 304; a client that does not accept gzip gets the file inflated.
pub fn file(name: &str, accept_encoding: Option<&str>, if_none_match: Option<&str>) -> Option<Served> {
    if !BUNDLED {
        return None;
    }
    let (content_type, gz, etag, len) = match name {
        "boot.js" => {
            return Some(Served {
                status: Status::Ok,
                content_type: "text/javascript; charset=utf-8",
                body: Cow::Borrowed(BOOT.as_bytes()),
                gzip: false,
                etag: "",
            });
        }
        "bicraft_web.js" => ("text/javascript; charset=utf-8", JS_GZ, JS_ETAG, JS_LEN),
        "bicraft_web_bg.wasm" => ("application/wasm", WASM_GZ, WASM_ETAG, WASM_LEN),
        _ => return None,
    };
    if if_none_match.is_some_and(|h| h.split(',').any(|t| t.trim() == etag || t.trim() == "*")) {
        return Some(Served {
            status: Status::NotModified,
            content_type,
            body: Cow::Borrowed(&[]),
            gzip: false,
            etag,
        });
    }
    if accepts_gzip(accept_encoding) {
        return Some(Served {
            status: Status::Ok,
            content_type,
            body: Cow::Borrowed(gz),
            gzip: true,
            etag,
        });
    }
    let mut raw = Vec::with_capacity(len);
    // trusted input (compressed at build time), bounded by its known length anyway
    let limit = u64::try_from(len).unwrap_or(u64::MAX).saturating_add(1);
    if flate2::read::GzDecoder::new(gz)
        .take(limit)
        .read_to_end(&mut raw)
        .is_err()
        || raw.len() != len
    {
        return Some(Served {
            status: Status::InternalError,
            content_type: "text/plain; charset=utf-8",
            body: Cow::Borrowed(b"the embedded browser app is damaged\n"),
            gzip: false,
            etag: "",
        });
    }
    Some(Served {
        status: Status::Ok,
        content_type,
        body: Cow::Owned(raw),
        gzip: false,
        etag,
    })
}

impl Served {
    /// Writes the response: revalidated on every load (`no-cache` with the entity tag), same
    /// origin only.
    pub fn write(&self, stream: &mut impl Write) -> io::Result<()> {
        let mut extra: Vec<(&str, &str)> = vec![
            ("Cache-Control", "no-cache"),
            ("Cross-Origin-Resource-Policy", "same-origin"),
            ("Vary", "Accept-Encoding"),
        ];
        if !self.etag.is_empty() {
            extra.push(("ETag", self.etag));
        }
        if self.gzip {
            extra.push(("Content-Encoding", "gzip"));
        }
        write_response(stream, self.status, self.content_type, &extra, &self.body)
    }
}

/// Answers an app route of the HTML-less build: 404 with [`NOT_BUNDLED`].
pub fn write_not_bundled(stream: &mut impl Write) -> io::Result<()> {
    write_response(
        stream,
        Status::NotFound,
        "text/plain; charset=utf-8",
        &[("Cache-Control", "no-store")],
        NOT_BUNDLED.as_bytes(),
    )
}

/// Writes the app page.
pub fn write_page(stream: &mut impl Write, page: &str) -> io::Result<()> {
    write_response(
        stream,
        Status::Ok,
        "text/html; charset=utf-8",
        &[
            ("Cache-Control", "no-store"),
            ("Content-Security-Policy", APP_CONTENT_SECURITY_POLICY),
        ],
        page.as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn the_app_page_has_the_viewers_favicon() {
        let icons = |p: &str| -> Vec<String> {
            p.lines()
                .filter(|l| l.starts_with(r#"<link rel="icon""#))
                .map(str::to_string)
                .collect()
        };
        let ours = icons(&app_page(None, None, None));
        assert_eq!(ours.len(), 2);
        assert_eq!(ours, icons(include_str!("page.html")));
    }

    #[test]
    fn page_slots() {
        let p = app_page(Some(T), None, Some(T));
        assert!(p.contains(&format!(r#"<meta name="bicraft-interact-token" content="{T}">"#)));
        assert!(p.contains(&format!(r#"<meta name="bicraft-view-id" content="{T}">"#)));
        assert!(p.contains(r#"<meta name="bicraft-open-token" content="">"#));
        // anything but 32 lowercase hex digits stays out
        let bad = app_page(
            Some("\"><script>"),
            Some("ABCDEF0123456789ABCDEF0123456789"),
            None,
        );
        assert!(!bad.contains("<script>alert") && !bad.contains("\"><script>"));
        assert!(bad.contains(r#"<meta name="bicraft-interact-token" content="">"#));
        assert!(bad.contains(r#"<meta name="bicraft-open-token" content="">"#));
        assert!(p.contains(r#"<script type="module" src="boot.js"></script>"#));
        // the CSP allows exactly what the page loads
        assert!(APP_CONTENT_SECURITY_POLICY.contains("script-src 'self' 'wasm-unsafe-eval';"));
        assert!(!APP_CONTENT_SECURITY_POLICY.contains("script-src 'self' 'unsafe-inline'"));
    }

    #[test]
    fn gzip_negotiation() {
        assert!(accepts_gzip(Some("gzip, deflate, br")));
        assert!(accepts_gzip(Some("br;q=1.0, GZIP;q=0.5")));
        assert!(accepts_gzip(Some("*")));
        assert!(!accepts_gzip(Some("gzip;q=0")));
        assert!(!accepts_gzip(Some("br, deflate")));
        assert!(!accepts_gzip(None));
    }

    #[test]
    fn files() {
        assert!(file("../secret", Some("gzip"), None).is_none());
        assert!(file("index.html", Some("gzip"), None).is_none());
        if !available() {
            assert!(file("bicraft_web_bg.wasm", Some("gzip"), None).is_none());
            assert!(file("boot.js", None, None).is_none());
            return;
        }
        let boot = file("boot.js", None, None).unwrap();
        assert_eq!(boot.status, Status::Ok);
        assert!(String::from_utf8_lossy(&boot.body).contains("bicraft_web_bg.wasm"));
        let gz = file("bicraft_web_bg.wasm", Some("gzip"), None).unwrap();
        assert!(gz.gzip && gz.body.starts_with(&[0x1f, 0x8b]));
        let raw = file("bicraft_web_bg.wasm", None, None).unwrap();
        assert!(!raw.gzip && raw.body.starts_with(b"\0asm"));
        assert_eq!(raw.body.len(), sizes().1);
        let again = file("bicraft_web_bg.wasm", Some("gzip"), Some(gz.etag)).unwrap();
        assert_eq!(again.status, Status::NotModified);
        assert!(again.body.is_empty());
    }
}
