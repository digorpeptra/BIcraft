//! A tiny, tolerant XML reader for the handful of small documents an ABF carries
//! (header `BackupLog`, `VirtualDirectory`, the files-section `BackupLog`).
//!
//! It builds an element tree and keeps only what the container needs: element local names,
//! their concatenated text, and their children. Attributes, namespaces, comments, processing
//! instructions and DOCTYPEs are skipped. Everything after the root element closes is ignored,
//! which tolerates the zero padding and end markers that follow these documents.
//!
//! Hostile input: nesting depth is capped, there is no recursion, and every allocation is
//! proportional to the input text.

use std::ops::Range;

use crate::Error;

/// Maximum element nesting. Real ABF documents nest at most 7 levels.
pub(crate) const MAX_DEPTH: usize = 64;

#[derive(Debug, Default)]
pub(crate) struct Element {
    /// Local name (any `prefix:` is stripped).
    pub name: String,
    /// Concatenated, entity-decoded character data directly inside this element.
    pub text: String,
    pub children: Vec<Element>,
    /// Byte range of everything between the start and end tags (raw, not unescaped), in the
    /// parsed document. `None` for a self-closing element. The writer patches values here.
    pub content: Option<Range<usize>>,
    /// Byte range of the whole element, from its `<` to the `>` that closes it.
    pub span: Range<usize>,
}

impl Element {
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn children_named<'s>(&'s self, name: &'s str) -> impl Iterator<Item = &'s Element> + 's {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Raw text of a direct child, untrimmed.
    pub fn child_text(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.as_str())
    }
}

/// Parses `doc` and returns its root element. `part` names the document in errors.
pub(crate) fn parse(doc: &str, part: &'static str) -> Result<Element, Error> {
    let bad = |reason: &str| Error::Malformed {
        part,
        reason: reason.to_string(),
    };
    let b = doc.as_bytes();
    let mut i = 0usize;
    let mut stack: Vec<Element> = Vec::new();

    loop {
        // Character data up to the next '<'.
        let next_lt = b
            .get(i..)
            .and_then(|rest| rest.iter().position(|&c| c == b'<'))
            .map(|p| i + p);
        let text_end = next_lt.unwrap_or(b.len());
        if let Some(top) = stack.last_mut() {
            if text_end > i {
                let raw = doc
                    .get(i..text_end)
                    .ok_or_else(|| bad("text not on a char boundary"))?;
                unescape_into(raw, &mut top.text);
            }
        } else if let Some(raw) = doc.get(i..text_end)
            && raw.trim_matches(is_prolog_noise).chars().next().is_some()
        {
            return Err(bad("text outside the root element"));
        }
        let Some(lt) = next_lt else {
            return Err(bad(if stack.is_empty() {
                "no root element"
            } else {
                "unexpected end of document"
            }));
        };
        let rest = b.get(lt..).unwrap_or_default();

        if rest.starts_with(b"<?") {
            i = skip_past(b, lt + 2, b"?>").ok_or_else(|| bad("unterminated processing instruction"))?;
            continue;
        }
        if rest.starts_with(b"<!--") {
            i = skip_past(b, lt + 4, b"-->").ok_or_else(|| bad("unterminated comment"))?;
            continue;
        }
        if rest.starts_with(b"<![CDATA[") {
            let start = lt + 9;
            let end = find(b, start, b"]]>").ok_or_else(|| bad("unterminated CDATA section"))?;
            let top = stack
                .last_mut()
                .ok_or_else(|| bad("CDATA outside the root element"))?;
            top.text.push_str(
                doc.get(start..end)
                    .ok_or_else(|| bad("CDATA not on a char boundary"))?,
            );
            i = end + 3;
            continue;
        }
        if rest.starts_with(b"<!") {
            // DOCTYPE or similar declaration: skip to the matching '>' (no internal subsets).
            i = skip_past(b, lt + 2, b">").ok_or_else(|| bad("unterminated declaration"))?;
            continue;
        }
        if rest.starts_with(b"</") {
            let name_start = lt + 2;
            let gt = find(b, name_start, b">").ok_or_else(|| bad("unterminated end tag"))?;
            let name = doc
                .get(name_start..gt)
                .ok_or_else(|| bad("end tag not on a char boundary"))?
                .trim();
            let mut elem = stack.pop().ok_or_else(|| bad("end tag without start tag"))?;
            if let Some(content) = elem.content.as_mut() {
                content.end = lt;
            }
            elem.span.end = gt + 1;
            if local_name(name) != elem.name {
                return Err(bad("mismatched end tag"));
            }
            i = gt + 1;
            match stack.last_mut() {
                Some(parent) => parent.children.push(elem),
                None => return Ok(elem),
            }
            continue;
        }

        // Start tag: name, then attributes (skipped, honouring quotes), then '>' or '/>'.
        let name_start = lt + 1;
        let name_end = b
            .get(name_start..)
            .and_then(|r| {
                r.iter()
                    .position(|&c| c.is_ascii_whitespace() || c == b'/' || c == b'>')
            })
            .map(|p| name_start + p)
            .ok_or_else(|| bad("unterminated start tag"))?;
        if name_end == name_start {
            return Err(bad("empty element name"));
        }
        let name = doc
            .get(name_start..name_end)
            .ok_or_else(|| bad("tag name not on a char boundary"))?;
        let (gt, self_closing) = tag_end(b, name_end).ok_or_else(|| bad("unterminated start tag"))?;
        let elem = Element {
            name: local_name(name).to_string(),
            content: (!self_closing).then_some(gt + 1..gt + 1),
            span: lt..gt + 1,
            ..Element::default()
        };
        i = gt + 1;
        if self_closing {
            match stack.last_mut() {
                Some(parent) => parent.children.push(elem),
                None => return Ok(elem),
            }
        } else {
            if stack.len() >= MAX_DEPTH {
                return Err(bad("elements nested too deeply"));
            }
            stack.push(elem);
        }
    }
}

fn is_prolog_noise(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}' || c == '\0'
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn find(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    let hay = b.get(from..)?;
    hay.windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

fn skip_past(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    find(b, from, needle).map(|p| p + needle.len())
}

/// Finds the `>` closing a start tag whose attributes begin at `from`. Returns its index and
/// whether the tag is self-closing.
fn tag_end(b: &[u8], from: usize) -> Option<(usize, bool)> {
    let mut quote: Option<u8> = None;
    let mut prev = 0u8;
    for (off, &c) in b.get(from..)?.iter().enumerate() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == b'"' || c == b'\'' => quote = Some(c),
            None if c == b'>' => return Some((from + off, prev == b'/')),
            None => {}
        }
        prev = c;
    }
    None
}

/// Appends `raw` to `out`, decoding the five predefined entities and numeric character
/// references. Unknown or malformed references are kept literally.
fn unescape_into(raw: &str, out: &mut String) {
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        out.push_str(rest.get(..amp).unwrap_or_default());
        let after = rest.get(amp + 1..).unwrap_or_default();
        let decoded = after
            .get(..after.len().min(12))
            .and_then(|window| window.find(';'))
            .and_then(|semi| Some((decode_entity(after.get(..semi)?)?, semi)));
        match decoded {
            Some((ch, semi)) => {
                out.push(ch);
                rest = after.get(semi + 1..).unwrap_or_default();
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
}

fn decode_entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let num = name.strip_prefix('#')?;
            let code = match num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => num.parse::<u32>().ok()?,
            };
            char::from_u32(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_elements_and_text() {
        let root = parse("<?xml version=\"1.0\"?><!-- c --><a x='1>2'><b>hi &amp; &#x41;&#66;</b><c/><ns:d>z</ns:d></a>\0\0", "t")
            .unwrap();
        assert_eq!(root.name, "a");
        assert_eq!(root.child_text("b"), Some("hi & AB"));
        assert!(root.child("c").is_some());
        assert_eq!(root.child_text("d"), Some("z"));
    }

    #[test]
    fn records_content_ranges() {
        let doc = "<a><b> 12 </b><c/><d>x<e>y</e></d></a>";
        let root = parse(doc, "t").unwrap();
        let span = |e: &Element| e.content.clone().map(|r| &doc[r]);
        assert_eq!(span(root.child("b").unwrap()), Some(" 12 "));
        assert_eq!(span(root.child("c").unwrap()), None);
        assert_eq!(span(root.child("d").unwrap()), Some("x<e>y</e>"));
        assert_eq!(span(&root), Some("<b> 12 </b><c/><d>x<e>y</e></d>"));
        let outer = |e: &Element| &doc[e.span.clone()];
        assert_eq!(outer(root.child("b").unwrap()), "<b> 12 </b>");
        assert_eq!(outer(root.child("c").unwrap()), "<c/>");
        assert_eq!(outer(root.child("d").unwrap()), "<d>x<e>y</e></d>");
        assert_eq!(outer(&root), doc);
    }

    #[test]
    fn keeps_unknown_entities_literally() {
        let root = parse("<a>x &foo; &#xZZ; & y</a>", "t").unwrap();
        assert_eq!(root.text, "x &foo; &#xZZ; & y");
    }

    #[test]
    fn cdata_and_self_closing_root() {
        assert_eq!(parse("<a><![CDATA[<x>]]></a>", "t").unwrap().text, "<x>");
        assert_eq!(parse("\u{feff} <a/>", "t").unwrap().name, "a");
    }

    #[test]
    fn rejects_malformed() {
        for doc in [
            "",
            "text",
            "<a>",
            "<a></b>",
            "</a>",
            "<a",
            "<>",
            "<a><!-- x</a>",
            "<a><![CDATA[x</a>",
            "x<a/>",
        ] {
            assert!(parse(doc, "t").is_err(), "{doc:?}");
        }
    }

    #[test]
    fn depth_is_capped() {
        let doc = "<a>".repeat(MAX_DEPTH + 1);
        assert!(matches!(parse(&doc, "t"), Err(Error::Malformed { .. })));
        let ok = format!("{}{}", "<a>".repeat(MAX_DEPTH), "</a>".repeat(MAX_DEPTH));
        assert!(parse(&ok, "t").is_ok());
    }
}
