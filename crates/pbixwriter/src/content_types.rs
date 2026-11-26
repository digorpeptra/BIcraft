//! `[Content_Types].xml`, generated as seen in Desktop-saved files (PK-CT): UTF-8 with BOM, no
//! line breaks, `Default` entries (one per extension, the content type of the first part with that
//! extension) before `Override` entries (extension-less parts, or parts whose type differs from
//! their extension's `Default`), each in part order.

use bicraft_opc::Part;

use crate::{content_type_of, part};

const HEADER: &str = r#"<?xml version="1.0" encoding="utf-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#;
const FOOTER: &str = "</Types>";
const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ContentTypes {
    /// `(extension, content type)`.
    pub defaults: Vec<(String, String)>,
    /// `(part name with leading '/', content type)`.
    pub overrides: Vec<(String, String)>,
}

impl ContentTypes {
    /// The content type OPC assigns to a part (by its zip entry name), if any.
    pub fn lookup(&self, name: &str) -> Option<&str> {
        let uri = format!("/{name}");
        if let Some((_, ct)) = self.overrides.iter().find(|(p, _)| p.eq_ignore_ascii_case(&uri)) {
            return Some(ct);
        }
        let ext = extension(name)?;
        self.defaults
            .iter()
            .find(|(e, _)| e.eq_ignore_ascii_case(ext))
            .map(|(_, ct)| ct.as_str())
    }
}

/// The extension of the last path segment (`png` for `a/b.png`), if it has a non-empty one.
pub(crate) fn extension(name: &str) -> Option<&str> {
    let last = name.rsplit('/').next()?;
    let (_, ext) = last.rsplit_once('.')?;
    (!ext.is_empty()).then_some(ext)
}

pub(crate) fn build(parts: &[Part]) -> Vec<u8> {
    let mut ct = ContentTypes::default();
    for p in parts {
        if p.name == part::CONTENT_TYPES {
            continue;
        }
        let ty = content_type_of(&p.name);
        let needs_override = match extension(&p.name) {
            None => true,
            Some(ext) => match ct.defaults.iter().find(|(e, _)| e.eq_ignore_ascii_case(ext)) {
                None => {
                    ct.defaults.push((ext.to_string(), ty.to_string()));
                    false
                }
                Some((_, existing)) => existing != ty,
            },
        };
        if needs_override {
            ct.overrides.push((format!("/{}", p.name), ty.to_string()));
        }
    }
    let mut xml = String::from(HEADER);
    for (ext, ty) in &ct.defaults {
        xml.push_str(&format!(
            r#"<Default Extension="{}" ContentType="{}" />"#,
            escape(ext),
            escape(ty)
        ));
    }
    for (name, ty) in &ct.overrides {
        xml.push_str(&format!(
            r#"<Override PartName="{}" ContentType="{}" />"#,
            escape(name),
            escape(ty)
        ));
    }
    xml.push_str(FOOTER);
    [&BOM[..], xml.as_bytes()].concat()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Minimal reader for `[Content_Types].xml`: collects `Default` and `Override` elements. It is
/// meant for validation, not as a general XML parser.
pub(crate) fn parse(bytes: &[u8]) -> Result<ContentTypes, String> {
    let body = bytes.strip_prefix(&BOM[..]).unwrap_or(bytes);
    let text = std::str::from_utf8(body).map_err(|_| "not UTF-8".to_string())?;
    if !text.contains("<Types") {
        return Err("no <Types> element".into());
    }
    let mut ct = ContentTypes::default();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        let after = rest.get(start.saturating_add(1)..).unwrap_or_default();
        let end = after.find('>').ok_or("unterminated element")?;
        let tag = after.get(..end).unwrap_or_default();
        rest = after.get(end.saturating_add(1)..).unwrap_or_default();
        let (kind, key) = if tag.starts_with("Default") && tag.ends_with('/') {
            (true, "Extension")
        } else if tag.starts_with("Override") && tag.ends_with('/') {
            (false, "PartName")
        } else {
            continue;
        };
        let k = attribute(tag, key).ok_or_else(|| format!("element without {key}"))?;
        let v = attribute(tag, "ContentType").ok_or("element without ContentType")?;
        if kind {
            ct.defaults.push((k, v));
        } else {
            ct.overrides.push((k, v));
        }
    }
    Ok(ct)
}

/// Namespace System.IO.Packaging requires on `<Types>`.
const NAMESPACE: &str = "http://schemas.openxmlformats.org/package/2006/content-types";

/// What .NET's public System.IO.Packaging API, used as an oracle, refuses in a
/// `[Content_Types].xml` that [`parse`] lets through (`docs/formats/pbix-package.md` §11,
/// "`[Content_Types].xml`"): a missing namespace, elements or
/// attributes outside the schema, two `Default`s for one extension or two `Override`s for one
/// part (case-insensitive), a content type that is neither empty nor `type/subtype`, and an
/// `Override` of `/[Content_Types].xml` itself. Desktop refused the duplicates; the others make
/// the library refuse the package.
pub(crate) fn strict_issues(bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let body = bytes.strip_prefix(&BOM[..]).unwrap_or(bytes);
    let Ok(text) = std::str::from_utf8(body) else {
        return out; // reported by `parse`
    };
    let mut defaults: Vec<String> = Vec::new();
    let mut overrides: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        let after = rest.get(start.saturating_add(1)..).unwrap_or_default();
        let Some(end) = after.find('>') else { break };
        let tag = after
            .get(..end)
            .unwrap_or_default()
            .trim_end_matches('/')
            .trim_end();
        rest = after.get(end.saturating_add(1)..).unwrap_or_default();
        if tag.starts_with('?') || tag.starts_with('!') || tag.starts_with('/') {
            continue;
        }
        let element = tag.split_whitespace().next().unwrap_or_default();
        let allowed: &[&str] = match element {
            "Types" => {
                if attribute(tag, "xmlns").as_deref() != Some(NAMESPACE) {
                    out.push("`[Content_Types].xml`: `<Types>` lacks the content-types namespace".into());
                }
                continue;
            }
            "Default" => &["Extension", "ContentType"],
            "Override" => &["PartName", "ContentType"],
            other => {
                out.push(format!(
                    "`[Content_Types].xml`: element `<{other}>` is not allowed"
                ));
                continue;
            }
        };
        for name in attribute_names(tag) {
            if !allowed.contains(&name.as_str()) {
                out.push(format!(
                    "`[Content_Types].xml`: attribute `{name}` is not allowed on `<{element}>`"
                ));
            }
        }
        if let Some(ct) = attribute(tag, "ContentType")
            && !ct.is_empty()
            && !valid_media_type(&ct)
        {
            out.push(format!(
                "`[Content_Types].xml`: content type `{ct}` is not `type/subtype`"
            ));
        }
        let (key, list) = if element == "Default" {
            ("Extension", &mut defaults)
        } else {
            ("PartName", &mut overrides)
        };
        if let Some(k) = attribute(tag, key) {
            let lower = k.to_lowercase();
            if element == "Override" && lower == "/[content_types].xml" {
                out.push("`[Content_Types].xml`: an `Override` names the content-types part".into());
            }
            if list.contains(&lower) {
                out.push(format!(
                    "`[Content_Types].xml`: two `<{element}>` elements for `{k}` (names ignore case)"
                ));
            } else {
                list.push(lower);
            }
        }
    }
    out
}

/// `type/subtype`, optionally followed by `;` parameters, with RFC 7230 token characters.
fn valid_media_type(ct: &str) -> bool {
    let main = ct.split(';').next().unwrap_or_default().trim();
    let token = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c))
    };
    matches!(main.split_once('/'), Some((t, st)) if token(t) && token(st))
}

/// Attribute names of an element's start tag (`Default Extension="x" …` gives `Extension`).
fn attribute_names(tag: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = tag
        .split_once(char::is_whitespace)
        .map(|(_, r)| r)
        .unwrap_or_default();
    while let Some(eq) = rest.find('=') {
        let name = rest.get(..eq).unwrap_or_default().trim();
        if !name.is_empty() {
            names.push(name.to_string());
        }
        let after = rest.get(eq.saturating_add(1)..).unwrap_or_default().trim_start();
        let Some(quote) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            break;
        };
        let inner = after.get(1..).unwrap_or_default();
        let Some(close) = inner.find(quote) else { break };
        rest = inner.get(close.saturating_add(1)..).unwrap_or_default();
    }
    names
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let mut rest = tag;
    loop {
        let at = rest.find(name)?;
        let preceded_by_space = rest
            .get(..at)
            .and_then(|s| s.chars().last())
            .is_some_and(char::is_whitespace);
        let after = rest.get(at.saturating_add(name.len())..)?;
        let after_eq = after.trim_start().strip_prefix('=').map(str::trim_start);
        if let (true, Some(v)) = (preceded_by_space, after_eq) {
            let quote = v.chars().next().filter(|c| *c == '"' || *c == '\'')?;
            let inner = v.get(1..)?;
            let close = inner.find(quote)?;
            return inner.get(..close).map(unescape);
        }
        rest = after;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str) -> Part {
        Part::new(name, vec![], false)
    }

    #[test]
    fn defaults_then_overrides_like_system_io_packaging() {
        let parts = [
            p("Version"),
            p("[Content_Types].xml"),
            p("Settings"),
            p("Report/StaticResources/a.json"),
            p("Report/StaticResources/b.png"),
            p("Report/definition/x.json"),
        ];
        let xml = build(&parts);
        assert!(xml.starts_with(&BOM));
        let text = std::str::from_utf8(&xml[3..]).unwrap();
        assert_eq!(
            text,
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"json\" ContentType=\"\" /><Default Extension=\"png\" ContentType=\"\" /><Override PartName=\"/Version\" ContentType=\"\" /><Override PartName=\"/Settings\" ContentType=\"application/json\" /></Types>"
        );
        let ct = parse(&xml).unwrap();
        assert_eq!(ct.lookup("Settings"), Some("application/json"));
        assert_eq!(ct.lookup("report/definition/y.JSON"), Some(""));
        assert_eq!(ct.lookup("Other"), None);
    }

    /// The variants System.IO.Packaging refused or accepted on this machine (pbix-package.md §1).
    #[test]
    fn strict_issues_match_system_io_packaging() {
        let built = build(&[p("Version"), p("Settings"), p("a/b.json")]);
        let ok = String::from_utf8(built[3..].to_vec()).unwrap();
        assert_eq!(strict_issues(ok.as_bytes()), Vec::<String>::new());
        let refused = [
            ok.replace(NAMESPACE, "urn:x"),
            ok.replace("<Override", "<Foo Bar=\"1\" /><Override"),
            ok.replace("<Default ", "<Default Foo=\"1\" "),
            ok.replace(
                "<Override",
                "<Default Extension=\"JSON\" ContentType=\"\" /><Override",
            ),
            ok.replace(
                "</Types>",
                "<Override PartName=\"/version\" ContentType=\"\" /></Types>",
            ),
            ok.replace("ContentType=\"application/json\"", "ContentType=\"json\""),
            ok.replace(
                "</Types>",
                "<Override PartName=\"/[Content_Types].xml\" ContentType=\"\" /></Types>",
            ),
        ];
        for xml in refused {
            assert!(!strict_issues(xml.as_bytes()).is_empty(), "{xml}");
        }
        let accepted = [
            ok.replace(
                "ContentType=\"application/json\"",
                "ContentType=\"application/json; charset=utf-16\"",
            ),
            ok.replace("><", ">\r\n  <"),
            ok.replace(
                "</Types>",
                "<Override PartName=\"/Nothing/Here\" ContentType=\"\" /></Types>",
            ),
        ];
        for xml in accepted {
            assert_eq!(strict_issues(xml.as_bytes()), Vec::<String>::new(), "{xml}");
        }
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(parse(b"\xff\xfe").is_err());
        assert!(parse(b"<Types><Default Extension=\"x\" /></Types>").is_err());
        assert!(parse(b"<Types><Default").is_err());
        assert_eq!(parse(b"<Types/>").unwrap(), ContentTypes::default());
    }
}
