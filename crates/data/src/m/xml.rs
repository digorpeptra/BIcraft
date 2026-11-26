//! A minimal, non-validating XML event reader for the SpreadsheetML parts of
//! `.xlsx` files. No DTD processing and no external entities: only the five
//! predefined entities and numeric character references are expanded.

pub(crate) enum Event {
    /// Local name (namespace prefix dropped) and attributes (local names).
    Start(String, Vec<(String, String)>),
    End(String),
    Text(String),
}

pub(crate) struct Reader<'a> {
    s: &'a str,
    pos: usize,
    pending_end: Option<String>,
}

impl<'a> Reader<'a> {
    pub fn new(s: &'a str) -> Self {
        Self {
            s: s.strip_prefix('\u{feff}').unwrap_or(s),
            pos: 0,
            pending_end: None,
        }
    }
}

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_string()
}

pub(crate) fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(rest.get(..i).unwrap_or(""));
        let after = rest.get(i + 1..).unwrap_or("");
        let Some(end) = after.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = after;
            continue;
        };
        let ent = after.get(..end).unwrap_or("");
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") || e.starts_with("#X") => {
                u32::from_str_radix(e.get(2..).unwrap_or(""), 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            e if e.starts_with('#') => e.get(1..).and_then(|d| d.parse().ok()).and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = after.get(end + 1..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn attrs(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s;
    loop {
        rest = rest.trim_start();
        let Some(eq) = rest.find('=') else { break };
        let name = rest.get(..eq).unwrap_or("").trim();
        let after = rest.get(eq + 1..).unwrap_or("").trim_start();
        let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            break;
        };
        let body = after.get(1..).unwrap_or("");
        let Some(close) = body.find(q) else { break };
        out.push((local(name), unescape(body.get(..close).unwrap_or(""))));
        rest = body.get(close + 1..).unwrap_or("");
    }
    out
}

impl Iterator for Reader<'_> {
    type Item = Event;

    fn next(&mut self) -> Option<Event> {
        if let Some(n) = self.pending_end.take() {
            return Some(Event::End(n));
        }
        loop {
            let rest = self.s.get(self.pos..)?;
            if rest.is_empty() {
                return None;
            }
            if !rest.starts_with('<') {
                let end = rest.find('<').unwrap_or(rest.len());
                self.pos += end;
                let text = rest.get(..end).unwrap_or("");
                return Some(Event::Text(unescape(text)));
            }
            if let Some(body) = rest.strip_prefix("<![CDATA[") {
                let end = body.find("]]>").unwrap_or(body.len());
                self.pos += 9 + end + 3;
                return Some(Event::Text(body.get(..end).unwrap_or("").to_string()));
            }
            if rest.starts_with("<!--") {
                let end = rest.find("-->").map_or(rest.len(), |e| e + 3);
                self.pos += end;
                continue;
            }
            if rest.starts_with("<?") || rest.starts_with("<!") {
                let end = rest.find('>').map_or(rest.len(), |e| e + 1);
                self.pos += end;
                continue;
            }
            let end = rest.find('>').map_or(rest.len(), |e| e + 1);
            self.pos += end;
            let tag = rest.get(1..end.saturating_sub(1)).unwrap_or("");
            if let Some(name) = tag.strip_prefix('/') {
                return Some(Event::End(local(name.trim())));
            }
            let (tag, self_closing) = match tag.strip_suffix('/') {
                Some(t) => (t, true),
                None => (tag, false),
            };
            let split = tag.find(|c: char| c.is_whitespace()).unwrap_or(tag.len());
            let name = local(tag.get(..split).unwrap_or(""));
            let a = attrs(tag.get(split..).unwrap_or(""));
            if self_closing {
                self.pending_end = Some(name.clone());
            }
            return Some(Event::Start(name, a));
        }
    }
}

pub(crate) fn attr<'a>(a: &'a [(String, String)], name: &str) -> Option<&'a str> {
    a.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
}
