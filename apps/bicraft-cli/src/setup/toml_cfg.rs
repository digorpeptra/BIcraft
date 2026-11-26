//! Codex `config.toml`: the `[mcp_servers.bicraft]` table.
//!
//! The file is scanned (not rebuilt), and edits are made to the text in place: a new table is
//! appended, an update replaces only the values of `command` and `args`, and an uninstall removes
//! the `[mcp_servers.bicraft]` table and its sub-tables. Everything else, comments and formatting
//! included, is left byte for byte. The scanner checks the TOML structure (headers, keys, strings,
//! arrays, inline tables) well enough to refuse a file it cannot edit safely; it is not a full
//! TOML validator.

use super::{Analysis, Change, EntryState, Mode, SERVER_ARGS, SERVER_NAME, Want};

const TABLE: &str = "mcp_servers";
const MAX_DEPTH: usize = 64;

#[derive(Debug)]
enum Val {
    Str(String),
    Arr(Vec<Val>),
    Table(Vec<(Vec<String>, Val)>),
    Other,
}

#[derive(Debug)]
enum Kind {
    Blank,
    Comment,
    Header {
        path: Vec<String>,
        array: bool,
    },
    Pair {
        key: Vec<String>,
        value: Val,
        vstart: usize,
        vend: usize,
    },
}

#[derive(Debug)]
struct Item {
    kind: Kind,
    /// Byte offset of the start of the item's first line.
    start: usize,
    /// Byte offset just after the item's last line ending (or the end of the text).
    end: usize,
}

struct Scanner<'a> {
    s: &'a str,
    pos: usize,
}

type Res<T> = Result<T, String>;

impl<'a> Scanner<'a> {
    fn peek(&self) -> Option<u8> {
        self.s.as_bytes().get(self.pos).copied()
    }

    fn rest(&self) -> &'a str {
        self.s.get(self.pos..).unwrap_or("")
    }

    fn err<T>(&self, msg: &str) -> Res<T> {
        let line = self.s.get(..self.pos).unwrap_or("").matches('\n').count() + 1;
        Err(format!("invalid TOML at line {line}: {msg}"))
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.pos += 1;
        }
    }

    fn skip_comment(&mut self) {
        if self.peek() == Some(b'#') {
            let n = self.rest().find('\n').unwrap_or(self.rest().len());
            self.pos += n;
            // A comment ending in `\r\n` leaves the `\r` to `newline`.
            if self.s.as_bytes().get(self.pos.wrapping_sub(1)) == Some(&b'\r') {
                self.pos -= 1;
            }
        }
    }

    /// Consumes one line ending; false at anything else.
    fn newline(&mut self) -> bool {
        if self.rest().starts_with("\r\n") {
            self.pos += 2;
            true
        } else if self.peek() == Some(b'\n') {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// The end of a line: spaces, an optional comment, then a line ending or the end of the text.
    fn eol(&mut self) -> Res<()> {
        self.skip_ws();
        self.skip_comment();
        if self.newline() || self.peek().is_none() {
            Ok(())
        } else {
            self.err("expected the end of the line")
        }
    }

    /// Spaces, comments and line endings (inside arrays and inline tables).
    fn skip_ws_nl(&mut self) {
        loop {
            self.skip_ws();
            self.skip_comment();
            if !self.newline() {
                break;
            }
        }
    }

    fn next_char(&mut self) -> Option<char> {
        let c = self.rest().chars().next()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn key(&mut self) -> Res<Vec<String>> {
        let mut parts = Vec::new();
        loop {
            self.skip_ws();
            let part = match self.peek() {
                Some(b'"') => {
                    if self.rest().starts_with("\"\"\"") {
                        return self.err("a key cannot be a multi-line string");
                    }
                    self.basic()?
                }
                Some(b'\'') => {
                    if self.rest().starts_with("'''") {
                        return self.err("a key cannot be a multi-line string");
                    }
                    self.literal()?
                }
                _ => {
                    let start = self.pos;
                    while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                    {
                        self.pos += 1;
                    }
                    if self.pos == start {
                        return self.err("expected a key");
                    }
                    self.s.get(start..self.pos).unwrap_or("").to_string()
                }
            };
            parts.push(part);
            self.skip_ws();
            if self.peek() == Some(b'.') {
                self.pos += 1;
            } else {
                return Ok(parts);
            }
        }
    }

    fn escape(&mut self, out: &mut String, multiline: bool) -> Res<()> {
        let Some(c) = self.next_char() else {
            return self.err("unterminated string");
        };
        let decoded = match c {
            'b' => '\u{8}',
            't' => '\t',
            'n' => '\n',
            'f' => '\u{c}',
            'r' => '\r',
            'e' => '\u{1b}',
            '"' => '"',
            '\\' => '\\',
            'u' | 'U' => {
                let n = if c == 'u' { 4 } else { 8 };
                let hex = self.rest().get(..n).unwrap_or("");
                let code = (hex.len() == n)
                    .then(|| u32::from_str_radix(hex, 16).ok())
                    .flatten()
                    .and_then(char::from_u32);
                let Some(ch) = code else {
                    return self.err("invalid unicode escape");
                };
                self.pos += n;
                ch
            }
            ' ' | '\t' | '\r' | '\n' if multiline => {
                // A line-ending backslash trims the whitespace that follows.
                self.pos -= c.len_utf8();
                let save = self.pos;
                self.skip_ws();
                if !self.newline() {
                    self.pos = save;
                    return self.err("invalid escape");
                }
                while matches!(self.peek(), Some(b' ' | b'\t' | b'\n')) || self.rest().starts_with("\r\n") {
                    if !self.newline() {
                        self.pos += 1;
                    }
                }
                return Ok(());
            }
            _ => return self.err("invalid escape"),
        };
        out.push(decoded);
        Ok(())
    }

    fn basic(&mut self) -> Res<String> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            match self.next_char() {
                None | Some('\n') => return self.err("unterminated string"),
                Some('"') => return Ok(out),
                Some('\\') => self.escape(&mut out, false)?,
                Some(c) => out.push(c),
            }
        }
    }

    fn literal(&mut self) -> Res<String> {
        self.pos += 1;
        let rest = self.rest();
        let Some(n) = rest.find('\'') else {
            return self.err("unterminated string");
        };
        let body = rest.get(..n).unwrap_or("");
        if body.contains('\n') {
            return self.err("unterminated string");
        }
        self.pos += n + 1;
        Ok(body.to_string())
    }

    fn multiline(&mut self, quote: &str) -> Res<String> {
        self.pos += 3;
        self.newline();
        let mut out = String::new();
        loop {
            if self.rest().starts_with(quote) {
                self.pos += 3;
                // Up to two quotes may sit right before the closing delimiter.
                let (q, qc) = if quote.starts_with('"') {
                    (b'"', '"')
                } else {
                    (b'\'', '\'')
                };
                for _ in 0..2 {
                    if self.peek() == Some(q) {
                        self.pos += 1;
                        out.push(qc);
                    }
                }
                return Ok(out);
            }
            match self.next_char() {
                None => return self.err("unterminated multi-line string"),
                Some('\\') if quote == "\"\"\"" => self.escape(&mut out, true)?,
                Some(c) => out.push(c),
            }
        }
    }

    fn value(&mut self, depth: usize) -> Res<Val> {
        if depth > MAX_DEPTH {
            return self.err("values nested too deeply");
        }
        match self.peek() {
            Some(b'"') if self.rest().starts_with("\"\"\"") => self.multiline("\"\"\"").map(Val::Str),
            Some(b'"') => self.basic().map(Val::Str),
            Some(b'\'') if self.rest().starts_with("'''") => self.multiline("'''").map(Val::Str),
            Some(b'\'') => self.literal().map(Val::Str),
            Some(b'[') => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws_nl();
                    if self.peek() == Some(b']') {
                        self.pos += 1;
                        return Ok(Val::Arr(items));
                    }
                    items.push(self.value(depth + 1)?);
                    self.skip_ws_nl();
                    match self.peek() {
                        Some(b',') => self.pos += 1,
                        Some(b']') => {
                            self.pos += 1;
                            return Ok(Val::Arr(items));
                        }
                        _ => return self.err("expected `,` or `]` in an array"),
                    }
                }
            }
            Some(b'{') => {
                self.pos += 1;
                let mut pairs = Vec::new();
                loop {
                    self.skip_ws_nl();
                    if self.peek() == Some(b'}') {
                        self.pos += 1;
                        return Ok(Val::Table(pairs));
                    }
                    let key = self.key()?;
                    if self.peek() != Some(b'=') {
                        return self.err("expected `=` in an inline table");
                    }
                    self.pos += 1;
                    self.skip_ws();
                    let v = self.value(depth + 1)?;
                    pairs.push((key, v));
                    self.skip_ws_nl();
                    match self.peek() {
                        Some(b',') => self.pos += 1,
                        Some(b'}') => {
                            self.pos += 1;
                            return Ok(Val::Table(pairs));
                        }
                        _ => return self.err("expected `,` or `}` in an inline table"),
                    }
                }
            }
            _ => {
                let start = self.pos;
                let scalar =
                    |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.' | b'_' | b':');
                while self.peek().is_some_and(scalar) {
                    self.pos += 1;
                }
                // A date and a time may be separated by a space.
                let token = self.s.get(start..self.pos).unwrap_or("");
                let is_date = token.len() == 10 && token.as_bytes().get(4) == Some(&b'-');
                if is_date
                    && self.peek() == Some(b' ')
                    && self
                        .s
                        .as_bytes()
                        .get(self.pos + 1)
                        .is_some_and(u8::is_ascii_digit)
                {
                    self.pos += 1;
                    while self.peek().is_some_and(scalar) {
                        self.pos += 1;
                    }
                }
                if self.pos == start {
                    return self.err("expected a value");
                }
                Ok(Val::Other)
            }
        }
    }

    fn items(mut self) -> Res<Vec<Item>> {
        let mut items = Vec::new();
        while self.pos < self.s.len() {
            let start = self.pos;
            self.skip_ws();
            let kind = match self.peek() {
                None => Kind::Blank,
                Some(b'\n' | b'\r') => {
                    if !self.newline() {
                        return self.err("stray carriage return");
                    }
                    Kind::Blank
                }
                Some(b'#') => {
                    self.eol()?;
                    Kind::Comment
                }
                Some(b'[') => {
                    self.pos += 1;
                    let array = self.peek() == Some(b'[');
                    if array {
                        self.pos += 1;
                    }
                    let path = self.key()?;
                    for _ in 0..(1 + usize::from(array)) {
                        if self.peek() != Some(b']') {
                            return self.err("expected `]` after a table name");
                        }
                        self.pos += 1;
                    }
                    self.eol()?;
                    Kind::Header { path, array }
                }
                Some(_) => {
                    let key = self.key()?;
                    if self.peek() != Some(b'=') {
                        return self.err("expected `=` after a key");
                    }
                    self.pos += 1;
                    self.skip_ws();
                    let vstart = self.pos;
                    let value = self.value(0)?;
                    let vend = self.pos;
                    self.eol()?;
                    Kind::Pair {
                        key,
                        value,
                        vstart,
                        vend,
                    }
                }
            };
            items.push(Item {
                kind,
                start,
                end: self.pos,
            });
        }
        Ok(items)
    }
}

/// Calls `f` with every leaf key path of `value` under `path` (inline tables are flattened).
fn leaves<'v>(path: &mut Vec<String>, value: &'v Val, f: &mut dyn FnMut(&[String], &'v Val)) {
    if let Val::Table(pairs) = value {
        for (k, v) in pairs {
            let n = path.len();
            path.extend(k.iter().cloned());
            leaves(path, v, f);
            path.truncate(n);
        }
    } else {
        f(path, value);
    }
}

fn starts_with(path: &[String], prefix: &[&str]) -> bool {
    path.len() >= prefix.len() && path.iter().zip(prefix).all(|(a, b)| a == b)
}

/// A TOML basic string.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn args_text() -> String {
    let parts: Vec<String> = SERVER_ARGS.iter().map(|a| quote(a)).collect();
    format!("[{}]", parts.join(", "))
}

/// Reads `bytes` (`None`: no file) and plans the change for `mode`.
pub fn analyze(bytes: Option<&[u8]>, want: &Want, mode: Mode) -> Result<Analysis, String> {
    let raw = match bytes {
        None => "",
        Some(b) => std::str::from_utf8(b).map_err(|_| "the file is not UTF-8".to_string())?,
    };
    let (bom, text) = match raw.strip_prefix('\u{feff}') {
        Some(t) => ("\u{feff}", t),
        None => ("", raw),
    };
    let items = Scanner { s: text, pos: 0 }.items()?;
    let ours: [&str; 2] = [TABLE, SERVER_NAME];

    // Sections: indexes of the headers of `[mcp_servers.bicraft]` and its sub-tables.
    let mut sections = Vec::new();
    let mut main = None;
    let mut command: Option<(Option<String>, usize, usize)> = None;
    let mut args: Option<(Option<Vec<String>>, usize, usize)> = None;
    let mut foreign = false; // bicraft defined with dotted keys or an inline table
    let mut blocks_add = None; // `mcp_servers` defined in a way a new header would conflict with
    let mut table: Vec<String> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        match &item.kind {
            Kind::Header { path, array } => {
                table = path.clone();
                if starts_with(path, &ours) {
                    if *array {
                        return Err(format!("`[[{TABLE}.{SERVER_NAME}]]` is an array of tables"));
                    }
                    sections.push(i);
                    if path.len() == 2 && main.is_none() {
                        main = Some(i);
                    }
                } else if path.len() == 1 && path.first().is_some_and(|p| p == TABLE) && *array {
                    blocks_add = Some(format!("`[[{TABLE}]]` is an array of tables"));
                }
            }
            Kind::Pair {
                key,
                value,
                vstart,
                vend,
            } => {
                let in_ours = starts_with(&table, &ours);
                let mut full = table.clone();
                full.extend(key.iter().cloned());
                if !starts_with(&table, &[TABLE]) && starts_with(&full, &[TABLE]) {
                    blocks_add = Some(format!(
                        "`{TABLE}` is defined with dotted keys or an inline table"
                    ));
                }
                if in_ours && table.len() == 2 && key.len() == 1 {
                    match key.first().map(String::as_str) {
                        Some("command") if command.is_none() => {
                            let v = if let Val::Str(s) = value {
                                Some(s.clone())
                            } else {
                                None
                            };
                            command = Some((v, *vstart, *vend));
                        }
                        Some("args") if args.is_none() => {
                            args = Some((string_array(value), *vstart, *vend));
                        }
                        _ => {}
                    }
                    continue;
                }
                if in_ours {
                    continue;
                }
                let mut path = full;
                leaves(&mut path, value, &mut |p, v| {
                    if starts_with(p, &ours) {
                        foreign = true;
                        if p.len() == 3 {
                            match p.get(2).map(String::as_str) {
                                Some("command") if command.is_none() => {
                                    let s = if let Val::Str(s) = v {
                                        Some(s.clone())
                                    } else {
                                        None
                                    };
                                    command = Some((s, 0, 0));
                                }
                                Some("args") if args.is_none() => {
                                    args = Some((string_array(v), 0, 0));
                                }
                                _ => {}
                            }
                        }
                    }
                });
            }
            Kind::Blank | Kind::Comment => {}
        }
    }

    let present = !sections.is_empty() || foreign;
    let registered = command.as_ref().and_then(|c| c.0.clone());
    let is_aligned = registered.as_deref().is_some_and(|c| want.same_command(c))
        && args
            .as_ref()
            .and_then(|a| a.0.as_ref())
            .is_some_and(|a| a.iter().map(String::as_str).eq(SERVER_ARGS));
    let state = if !present {
        EntryState::Absent
    } else if is_aligned {
        EntryState::Aligned
    } else {
        EntryState::Stale { command: registered }
    };

    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let manual = |what: &str| {
        Change::Blocked(format!(
            "`{TABLE}.{SERVER_NAME}` is defined with dotted keys or an inline table; {what} it by hand"
        ))
    };
    let change = match (mode, &state) {
        (Mode::Install, EntryState::Aligned) | (Mode::Uninstall, EntryState::Absent) => Change::None,
        (Mode::Install, EntryState::Absent) => match blocks_add {
            Some(why) => Change::Blocked(format!("{why}; add `{TABLE}.{SERVER_NAME}` by hand")),
            None => {
                let mut out = text.to_string();
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push_str(nl);
                }
                if !out.trim().is_empty() && !out.ends_with(&format!("{nl}{nl}")) {
                    out.push_str(nl);
                }
                out.push_str(&block(&want.command, nl));
                Change::Write(format!("{bom}{out}").into_bytes())
            }
        },
        (Mode::Install, EntryState::Stale { .. }) if foreign => manual("update"),
        (Mode::Install, EntryState::Stale { .. }) => {
            let edits = match main.and_then(|i| items.get(i)) {
                None => {
                    // Only sub-tables (`[mcp_servers.bicraft.env]`): put the table before them.
                    let at = sections
                        .first()
                        .and_then(|&i| items.get(i))
                        .map_or(text.len(), |it| it.start);
                    vec![(at, at, format!("{}{nl}", block(&want.command, nl)))]
                }
                Some(header) => {
                    let mut edits = Vec::new();
                    let mut lines = String::new();
                    if !text.get(..header.end).unwrap_or("").ends_with('\n') {
                        lines.push_str(nl);
                    }
                    match &command {
                        Some((_, s, e)) => edits.push((*s, *e, quote(&want.command))),
                        None => lines.push_str(&format!("command = {}{nl}", quote(&want.command))),
                    }
                    match &args {
                        Some((_, s, e)) => edits.push((*s, *e, args_text())),
                        None => lines.push_str(&format!("args = {}{nl}", args_text())),
                    }
                    if !lines.trim().is_empty() {
                        edits.push((header.end, header.end, lines));
                    }
                    edits
                }
            };
            Change::Write(format!("{bom}{}", apply(text, edits)).into_bytes())
        }
        (Mode::Uninstall, _) if foreign => manual("remove"),
        (Mode::Uninstall, _) => {
            let mut ranges: Vec<(usize, usize)> = Vec::new();
            for &i in &sections {
                let Some(header) = items.get(i) else { continue };
                // The section runs to the next header, minus the comments right above it.
                let next = items
                    .iter()
                    .enumerate()
                    .skip(i + 1)
                    .find(|(_, it)| matches!(it.kind, Kind::Header { .. }))
                    .map(|(j, _)| j);
                let end = match next {
                    None => text.len(),
                    Some(j) => {
                        let mut k = j;
                        while k > i + 1
                            && items
                                .get(k - 1)
                                .is_some_and(|it| matches!(it.kind, Kind::Comment))
                        {
                            k -= 1;
                        }
                        items.get(k).map_or(text.len(), |it| it.start)
                    }
                };
                ranges.push((header.start, end));
            }
            // Merge touching sections (a table and its sub-tables).
            ranges.sort_unstable();
            let mut merged: Vec<(usize, usize)> = Vec::new();
            for (s, e) in ranges {
                match merged.last_mut() {
                    Some(last) if s <= last.1 => last.1 = last.1.max(e),
                    _ => merged.push((s, e)),
                }
            }
            // At the end of the file, the blank lines before the table go too.
            if let Some(last) = merged.last_mut().filter(|r| r.1 == text.len()) {
                while let Some(blank) = items
                    .iter()
                    .find(|it| it.end == last.0 && it.start < it.end && matches!(it.kind, Kind::Blank))
                {
                    last.0 = blank.start;
                }
            }
            let edits = merged.into_iter().map(|(s, e)| (s, e, String::new())).collect();
            Change::Write(format!("{bom}{}", apply(text, edits)).into_bytes())
        }
    };
    Ok(Analysis { state, change })
}

fn string_array(v: &Val) -> Option<Vec<String>> {
    match v {
        Val::Arr(items) => items
            .iter()
            .map(|i| {
                if let Val::Str(s) = i {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect(),
        _ => None,
    }
}

fn block(command: &str, nl: &str) -> String {
    format!(
        "[{TABLE}.{SERVER_NAME}]{nl}command = {}{nl}args = {}{nl}",
        quote(command),
        args_text()
    )
}

/// Applies non-overlapping `(start, end, replacement)` edits to `text`.
fn apply(text: &str, mut edits: Vec<(usize, usize, String)>) -> String {
    edits.sort_by_key(|e| (e.0, e.1));
    let mut out = String::with_capacity(text.len() + 128);
    let mut at = 0;
    for (start, end, rep) in edits {
        if start < at {
            continue;
        }
        out.push_str(text.get(at..start).unwrap_or(""));
        out.push_str(&rep);
        at = end.max(start);
    }
    out.push_str(text.get(at..).unwrap_or(""));
    out
}
