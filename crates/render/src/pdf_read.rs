//! A structural reader of PDF files, so tests and agents can check a PDF without other tools:
//! the header, the cross-reference table and every object it lists, the page tree with each
//! page's size, the fonts (embedded or not, with a `ToUnicode` map or not), and the text of each
//! page extracted through those maps (`report.export_pdf`'s check, `docs/render.md`, "PDF").
//!
//! It reads classic cross-reference tables (what BIcraft writes, ISO 32000-1 §7.5.4), not
//! cross-reference streams or incremental updates. Input is hostile: every read is bounded
//! (nesting, sizes, object counts, decompressed bytes) and errors are returned, never panics.

use std::collections::{BTreeMap, HashMap};
use std::io::Read as _;

/// Most objects read.
const MAX_OBJECTS: usize = 1_000_000;
/// Most bytes one stream decompresses to.
const MAX_STREAM: u64 = 256 << 20;
/// Nesting of arrays, dictionaries, page trees and forms.
const MAX_DEPTH: usize = 64;
/// Most text extracted per page (characters).
const MAX_PAGE_TEXT: usize = 2_000_000;

/// A PDF value.
#[derive(Debug, Clone, PartialEq)]
pub enum Obj {
    Null,
    Bool(bool),
    Num(f64),
    Name(String),
    Str(Vec<u8>),
    Array(Vec<Obj>),
    Dict(BTreeMap<String, Obj>),
    Ref(usize, u16),
    /// A stream: its dictionary and its raw (still encoded) bytes.
    Stream(BTreeMap<String, Obj>, Vec<u8>),
}

impl Obj {
    fn dict(&self) -> Option<&BTreeMap<String, Obj>> {
        match self {
            Obj::Dict(d) | Obj::Stream(d, _) => Some(d),
            _ => None,
        }
    }
    fn get(&self, k: &str) -> Option<&Obj> {
        self.dict()?.get(k)
    }
    fn name(&self) -> Option<&str> {
        match self {
            Obj::Name(n) => Some(n),
            _ => None,
        }
    }
    fn num(&self) -> Option<f64> {
        match self {
            Obj::Num(n) => Some(*n),
            _ => None,
        }
    }
}

/// A font a page uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontInfo {
    /// The resource name (`F0`).
    pub resource: String,
    pub base_font: String,
    pub subtype: String,
    /// A font file (`FontFile`, `FontFile2` or `FontFile3`) is embedded.
    pub embedded: bool,
    pub to_unicode: bool,
}

/// One page.
#[derive(Debug, Clone, PartialEq)]
pub struct PageInfo {
    /// `MediaBox` width and height (points).
    pub width: f64,
    pub height: f64,
    /// The text drawn, one line per text object.
    pub text: String,
    /// Images drawn (image XObjects painted, forms followed).
    pub images: usize,
    /// The text drawn outside any marked content (neither tagged nor an artifact).
    pub untagged_text: String,
    /// Marked content that does not nest (an `EMC` without its start, a sequence left open).
    pub unbalanced_marks: bool,
}

/// An outline entry (a bookmark), in the outline's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineEntry {
    /// 0 for the outline's top level.
    pub depth: usize,
    pub title: String,
    /// The page it goes to (0-based), when its destination is a page of the document.
    pub page: Option<usize>,
}

/// A structure element, in the tree's order (ISO 32000-1 §14.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructNode {
    /// 0 for the elements directly under the root.
    pub depth: usize,
    /// The structure type (`Document`, `Sect`, `P`, `Figure`).
    pub role: String,
    pub title: Option<String>,
    pub alt: Option<String>,
    /// The page it is on (0-based), from its `Pg`.
    pub page: Option<usize>,
    /// The text of its own marked content.
    pub text: String,
}

/// What [`inspect`] found.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfInfo {
    pub version: String,
    /// Objects in the cross-reference table (in use).
    pub objects: usize,
    pub pages: Vec<PageInfo>,
    /// The fonts of every page, once each (by object).
    pub fonts: Vec<FontInfo>,
    /// The information dictionary's `Title`.
    pub title: Option<String>,
    /// The catalog's `Lang`.
    pub lang: Option<String>,
    /// The catalog's `MarkInfo` says the document is tagged.
    pub marked: bool,
    /// The outline (bookmarks).
    pub outline: Vec<OutlineEntry>,
    /// The structure tree (empty when the document has none).
    pub structure: Vec<StructNode>,
    /// What does not hold together in the structure tree: a marked-content reference that the
    /// parent tree does not map back to its element, or that no content has.
    pub structure_issues: Vec<String>,
}

/// A PDF text string: UTF-16BE after its byte order mark, else bytes as Latin-1.
fn text_of(o: &Obj) -> Option<String> {
    match o {
        Obj::Str(s) if s.starts_with(&[0xfe, 0xff]) => Some(utf16(s.get(2..).unwrap_or(&[]))),
        Obj::Str(s) => Some(s.iter().map(|b| char::from(*b)).collect()),
        _ => None,
    }
}

/// Most outline entries and structure elements read.
const MAX_NODES: usize = 1_000_000;

struct Lexer<'a> {
    b: &'a [u8],
    i: usize,
}

fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\n' | b'\r' | b'\t' | b'\x0c' | 0)
}

fn is_delim(c: u8) -> bool {
    matches!(
        c,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if is_ws(c) {
                self.i += 1;
            } else if c == b'%' {
                while let Some(c) = self.peek() {
                    if c == b'\n' || c == b'\r' {
                        break;
                    }
                    self.i += 1;
                }
            } else {
                break;
            }
        }
    }

    /// A bare word (a number, a keyword, `R`).
    fn word(&mut self) -> &'a [u8] {
        let s = self.i;
        while let Some(c) = self.peek() {
            if is_ws(c) || is_delim(c) {
                break;
            }
            self.i += 1;
        }
        self.b.get(s..self.i).unwrap_or(&[])
    }

    fn literal(&mut self) -> Result<Vec<u8>, String> {
        // after '('
        let mut out = Vec::new();
        let mut depth = 1usize;
        loop {
            let c = self.peek().ok_or("unterminated string")?;
            self.i += 1;
            match c {
                b'\\' => {
                    let e = self.peek().ok_or("unterminated string")?;
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'0'..=b'7' => {
                            let mut v = u32::from(e - b'0');
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(d @ b'0'..=b'7') => {
                                        v = v * 8 + u32::from(d - b'0');
                                        self.i += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push((v & 0xff) as u8);
                        }
                        b'\r' | b'\n' => {}
                        other => out.push(other),
                    }
                }
                b'(' => {
                    depth += 1;
                    out.push(c);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(out);
                    }
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
    }

    fn hex(&mut self) -> Result<Vec<u8>, String> {
        // after '<'
        let mut digits = Vec::new();
        loop {
            let c = self.peek().ok_or("unterminated hex string")?;
            self.i += 1;
            if c == b'>' {
                break;
            }
            if let Some(d) = (c as char).to_digit(16) {
                digits.push(d as u8);
            } else if !is_ws(c) {
                return Err("bad hex string".into());
            }
        }
        if digits.len() % 2 == 1 {
            digits.push(0);
        }
        Ok(digits
            .chunks(2)
            .map(|p| (p.first().copied().unwrap_or(0) << 4) | p.get(1).copied().unwrap_or(0))
            .collect())
    }

    fn name(&mut self) -> String {
        // after '/'
        let w = self.word();
        let mut out = Vec::new();
        let mut i = 0usize;
        while let Some(&c) = w.get(i) {
            if c == b'#'
                && let (Some(a), Some(b)) = (w.get(i + 1), w.get(i + 2))
                && let (Some(a), Some(b)) = ((*a as char).to_digit(16), (*b as char).to_digit(16))
            {
                out.push((a * 16 + b) as u8);
                i += 3;
                continue;
            }
            out.push(c);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// One value; `R` references are folded from `n g R`.
    fn value(&mut self, depth: usize) -> Result<Obj, String> {
        if depth > MAX_DEPTH {
            return Err("nesting too deep".into());
        }
        self.skip_ws();
        let c = self.peek().ok_or("unexpected end of file")?;
        match c {
            b'/' => {
                self.i += 1;
                Ok(Obj::Name(self.name()))
            }
            b'(' => {
                self.i += 1;
                Ok(Obj::Str(self.literal()?))
            }
            b'<' if self.b.get(self.i + 1) == Some(&b'<') => {
                self.i += 2;
                let mut d = BTreeMap::new();
                loop {
                    self.skip_ws();
                    match self.peek() {
                        Some(b'>') => {
                            self.i += 2;
                            break;
                        }
                        Some(b'/') => {
                            self.i += 1;
                            let k = self.name();
                            let v = self.value(depth + 1)?;
                            d.insert(k, v);
                        }
                        _ => return Err("bad dictionary".into()),
                    }
                }
                Ok(Obj::Dict(d))
            }
            b'<' => {
                self.i += 1;
                Ok(Obj::Str(self.hex()?))
            }
            b'[' => {
                self.i += 1;
                let mut a = Vec::new();
                loop {
                    self.skip_ws();
                    match self.peek() {
                        Some(b']') => {
                            self.i += 1;
                            break;
                        }
                        None => return Err("unterminated array".into()),
                        _ => a.push(self.value(depth + 1)?),
                    }
                    if a.len() > 10_000_000 {
                        return Err("array too long".into());
                    }
                }
                Ok(Obj::Array(a))
            }
            _ => {
                let w = self.word();
                if w.is_empty() {
                    self.i += 1;
                    return Err(format!("unexpected byte {c:#x}"));
                }
                match w {
                    b"null" => return Ok(Obj::Null),
                    b"true" => return Ok(Obj::Bool(true)),
                    b"false" => return Ok(Obj::Bool(false)),
                    _ => {}
                }
                let s = std::str::from_utf8(w).map_err(|_| "bad token")?;
                let n: f64 = s
                    .parse()
                    .map_err(|_| format!("bad token `{}`", s.chars().take(20).collect::<String>()))?;
                // `n g R`?
                let save = self.i;
                if n >= 0.0 && n.fract() == 0.0 {
                    self.skip_ws();
                    let g = self.word();
                    self.skip_ws();
                    if let Ok(g) = std::str::from_utf8(g).unwrap_or("x").parse::<u16>()
                        && self.peek() == Some(b'R')
                        && self.b.get(self.i + 1).is_none_or(|c| is_ws(*c) || is_delim(*c))
                    {
                        self.i += 1;
                        return Ok(Obj::Ref(n as usize, g));
                    }
                }
                self.i = save;
                Ok(Obj::Num(n))
            }
        }
    }
}

struct File<'a> {
    b: &'a [u8],
    offsets: HashMap<usize, usize>,
    cache: std::cell::RefCell<HashMap<usize, Obj>>,
}

fn find_last(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (0..=hay.len() - needle.len())
        .rev()
        .find(|&i| hay.get(i..i + needle.len()) == Some(needle))
}

impl<'a> File<'a> {
    /// Object `n` (direct values pass through `resolve`).
    fn object(&self, n: usize) -> Result<Obj, String> {
        if let Some(o) = self.cache.borrow().get(&n) {
            return Ok(o.clone());
        }
        let at = *self
            .offsets
            .get(&n)
            .ok_or_else(|| format!("object {n} is not in the cross-reference table"))?;
        let mut lx = Lexer { b: self.b, i: at };
        lx.skip_ws();
        let num = lx.word();
        lx.skip_ws();
        let _gen = lx.word();
        lx.skip_ws();
        let kw = lx.word();
        if std::str::from_utf8(num)
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            != Some(n)
            || kw != b"obj"
        {
            return Err(format!(
                "the cross-reference offset of object {n} does not point at it"
            ));
        }
        let v = lx.value(0)?;
        lx.skip_ws();
        let v = if self.b.get(lx.i..lx.i + 6) == Some(b"stream") {
            lx.i += 6;
            if self.b.get(lx.i) == Some(&b'\r') {
                lx.i += 1;
            }
            if self.b.get(lx.i) == Some(&b'\n') {
                lx.i += 1;
            }
            let Obj::Dict(d) = v else {
                return Err(format!("object {n}: a stream without a dictionary"));
            };
            let len = match d.get("Length") {
                Some(Obj::Num(x)) => *x,
                Some(Obj::Ref(r, _)) if *r != n => self.object(*r)?.num().unwrap_or(-1.0),
                _ => -1.0,
            };
            if !(0.0..=self.b.len() as f64).contains(&len) {
                return Err(format!("object {n}: bad stream length"));
            }
            let end = lx.i.checked_add(len as usize).ok_or("bad stream length")?;
            let data = self
                .b
                .get(lx.i..end)
                .ok_or_else(|| format!("object {n}: the stream runs past the file"))?;
            let mut tail = Lexer { b: self.b, i: end };
            tail.skip_ws();
            if tail.word() != b"endstream" {
                return Err(format!("object {n}: no endstream after its Length"));
            }
            Obj::Stream(d, data.to_vec())
        } else {
            v
        };
        if self.cache.borrow().len() < 100_000 {
            self.cache.borrow_mut().insert(n, v.clone());
        }
        Ok(v)
    }

    fn resolve(&self, o: &Obj) -> Result<Obj, String> {
        match o {
            Obj::Ref(n, _) => self.object(*n),
            x => Ok(x.clone()),
        }
    }

    fn get(&self, o: &Obj, k: &str) -> Result<Obj, String> {
        match o.get(k) {
            Some(v) => self.resolve(v),
            None => Ok(Obj::Null),
        }
    }

    /// A stream's decoded bytes (no filter, or FlateDecode).
    fn decode(&self, o: &Obj) -> Result<Vec<u8>, String> {
        let Obj::Stream(d, data) = o else {
            return Err("not a stream".into());
        };
        match d.get("Filter") {
            None => Ok(data.clone()),
            Some(Obj::Name(f)) if f == "FlateDecode" => inflate(data),
            Some(Obj::Array(a)) if a.len() == 1 && a.first().and_then(Obj::name) == Some("FlateDecode") => {
                inflate(data)
            }
            Some(f) => Err(format!("unsupported filter {f:?}")),
        }
    }
}

fn inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .take(MAX_STREAM)
        .read_to_end(&mut out)
        .map_err(|e| format!("cannot inflate a stream: {e}"))?;
    Ok(out)
}

/// A font's code → text map (`ToUnicode`), and its code width (1 or 2 bytes).
struct Decoder {
    two: bool,
    map: HashMap<u32, String>,
}

impl Decoder {
    fn text(&self, s: &[u8]) -> String {
        if self.two {
            s.chunks(2)
                .map(|p| {
                    let c = (u32::from(p.first().copied().unwrap_or(0)) << 8)
                        | u32::from(p.get(1).copied().unwrap_or(0));
                    self.map.get(&c).cloned().unwrap_or_else(|| "\u{fffd}".into())
                })
                .collect()
        } else {
            s.iter()
                .map(|b| {
                    self.map
                        .get(&u32::from(*b))
                        .cloned()
                        .unwrap_or_else(|| char::from(*b).to_string())
                })
                .collect()
        }
    }
}

fn utf16(b: &[u8]) -> String {
    let u: Vec<u16> = b
        .chunks(2)
        .map(|p| {
            (u16::from(p.first().copied().unwrap_or(0)) << 8) | u16::from(p.get(1).copied().unwrap_or(0))
        })
        .collect();
    String::from_utf16_lossy(&u)
}

fn code(b: &[u8]) -> u32 {
    b.iter().take(4).fold(0u32, |a, x| (a << 8) | u32::from(*x))
}

/// Parses a ToUnicode CMap's `bfchar` and `bfrange` sections.
fn parse_cmap(text: &[u8]) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    let mut lx = Lexer { b: text, i: 0 };
    let mut toks: Vec<Obj> = Vec::new();
    let mut guard = 0usize;
    while lx.i < text.len() && guard < 2_000_000 {
        guard += 1;
        lx.skip_ws();
        let Some(c) = lx.peek() else { break };
        if c == b'<' || c == b'[' || c == b'/' || c == b'(' {
            match lx.value(0) {
                Ok(v) => toks.push(v),
                Err(_) => break,
            }
            continue;
        }
        let w = lx.word();
        if w.is_empty() {
            lx.i += 1;
            continue;
        }
        match w {
            b"endbfchar" => {
                for p in toks.chunks(2) {
                    if let [Obj::Str(src), Obj::Str(dst)] = p
                        && map.len() < 1_000_000
                    {
                        map.insert(code(src), utf16(dst));
                    }
                }
                toks.clear();
            }
            b"endbfrange" => {
                for p in toks.chunks(3) {
                    if let [Obj::Str(lo), Obj::Str(hi), dst] = p {
                        let (lo, hi) = (code(lo), code(hi));
                        for (k, c) in (lo..=hi.min(lo.saturating_add(65_535))).enumerate() {
                            let t = match dst {
                                Obj::Str(d) => {
                                    let mut u: Vec<u16> = d
                                        .chunks(2)
                                        .map(|p| {
                                            (u16::from(p.first().copied().unwrap_or(0)) << 8)
                                                | u16::from(p.get(1).copied().unwrap_or(0))
                                        })
                                        .collect();
                                    if let Some(last) = u.last_mut() {
                                        *last = last.wrapping_add(k as u16);
                                    }
                                    String::from_utf16_lossy(&u)
                                }
                                Obj::Array(a) => match a.get(k) {
                                    Some(Obj::Str(d)) => utf16(d),
                                    _ => continue,
                                },
                                _ => continue,
                            };
                            if map.len() < 1_000_000 {
                                map.insert(c, t);
                            }
                        }
                    }
                }
                toks.clear();
            }
            _ => {
                if !matches!(w, b"beginbfchar" | b"beginbfrange") {
                    toks.clear();
                }
            }
        }
    }
    map
}

struct Walk<'f, 'a> {
    f: &'f File<'a>,
    fonts: BTreeMap<String, FontInfo>,
    decoders: HashMap<String, std::rc::Rc<Decoder>>,
    /// The text of each marked-content id, by the `StructParents` key of its stream.
    mcid_text: HashMap<(i64, i64), String>,
}

/// The marked content of the stream being walked.
struct Marks<'u> {
    /// The stream's `StructParents` key.
    key: Option<i64>,
    /// The open sequences, with their MCID when they have one.
    stack: Vec<Option<i64>>,
    /// Painted from inside marked content of the stream that painted it.
    inside: bool,
    /// An `EMC` without its start, or a sequence left open.
    unbalanced: bool,
    untagged: &'u mut String,
}

impl Walk<'_, '_> {
    fn font(&mut self, key: &str, fo: &Obj) -> Result<std::rc::Rc<Decoder>, String> {
        if let Some(d) = self.decoders.get(key) {
            return Ok(std::rc::Rc::clone(d));
        }
        let subtype = self.f.get(fo, "Subtype")?.name().unwrap_or("").to_string();
        let base = self.f.get(fo, "BaseFont")?.name().unwrap_or("").to_string();
        let tu = self.f.get(fo, "ToUnicode")?;
        let map = match &tu {
            Obj::Stream(..) => parse_cmap(&self.f.decode(&tu)?),
            _ => HashMap::new(),
        };
        let two = subtype == "Type0";
        let descriptor = if two {
            let desc = self.f.get(fo, "DescendantFonts")?;
            let first = match &desc {
                Obj::Array(a) => a
                    .first()
                    .map(|x| self.f.resolve(x))
                    .transpose()?
                    .unwrap_or(Obj::Null),
                _ => Obj::Null,
            };
            self.f.get(&first, "FontDescriptor")?
        } else {
            self.f.get(fo, "FontDescriptor")?
        };
        let embedded = ["FontFile", "FontFile2", "FontFile3"]
            .iter()
            .any(|k| matches!(descriptor.get(k), Some(Obj::Ref(..))));
        self.fonts.insert(
            key.to_string(),
            FontInfo {
                resource: String::new(),
                base_font: base,
                subtype,
                embedded,
                to_unicode: matches!(tu, Obj::Stream(..)),
            },
        );
        let d = std::rc::Rc::new(Decoder { two, map });
        self.decoders.insert(key.to_string(), std::rc::Rc::clone(&d));
        Ok(d)
    }

    /// Text drawn: to the page's text, to its marked content's (the innermost with an `MCID`),
    /// or to the page's untagged text when it is in no marked content at all.
    fn put(&mut self, s: &str, out: &mut String, ctx: &mut Marks<'_>) {
        if out.len() < MAX_PAGE_TEXT {
            out.push_str(s);
        }
        match (ctx.stack.iter().rev().find_map(|m| *m), ctx.key) {
            (Some(mcid), Some(key)) => {
                let t = self.mcid_text.entry((key, mcid)).or_default();
                if t.len() < MAX_PAGE_TEXT {
                    t.push_str(s);
                }
            }
            _ if ctx.stack.is_empty() && !ctx.inside && ctx.untagged.len() < MAX_PAGE_TEXT => {
                ctx.untagged.push_str(s);
            }
            _ => {}
        }
    }

    /// Walks a content stream: text to `out`, images counted, forms followed.
    fn content(
        &mut self,
        data: &[u8],
        res: &Obj,
        out: &mut String,
        images: &mut usize,
        depth: usize,
        marks: &mut Marks<'_>,
    ) -> Result<(), String> {
        if depth > 16 {
            return Err("forms nested too deep".into());
        }
        let fonts = self.f.get(res, "Font")?;
        let xobjects = self.f.get(res, "XObject")?;
        let mut lx = Lexer { b: data, i: 0 };
        let mut stack: Vec<Obj> = Vec::new();
        let mut font: Option<std::rc::Rc<Decoder>> = None;
        let mut line_y: Option<f64> = None;
        let mut guard = 0usize;
        while lx.i < data.len() {
            guard += 1;
            if guard > 50_000_000 {
                return Err("content stream too long".into());
            }
            lx.skip_ws();
            let Some(c) = lx.peek() else { break };
            if matches!(c, b'/' | b'(' | b'<' | b'[')
                || c.is_ascii_digit()
                || c == b'-'
                || c == b'.'
                || c == b'+'
            {
                stack.push(lx.value(0)?);
                if stack.len() > 100_000 {
                    return Err("operand stack too deep".into());
                }
                continue;
            }
            let op = lx.word();
            if op.is_empty() {
                lx.i += 1;
                continue;
            }
            match op {
                b"Tf" => {
                    if let Some(Obj::Name(n)) = stack.first() {
                        let key = match fonts.get(n) {
                            Some(Obj::Ref(r, _)) => format!("{r}"),
                            _ => n.clone(),
                        };
                        let fo = self.f.get(&fonts, n)?;
                        let d = self.font(&key, &fo)?;
                        if let Some(fi) = self.fonts.get_mut(&key)
                            && fi.resource.is_empty()
                        {
                            fi.resource = n.clone();
                        }
                        font = Some(d);
                    }
                }
                b"Tj" | b"'" | b"\"" => {
                    if let (Some(Obj::Str(s)), Some(d)) = (stack.last(), &font) {
                        let t = d.text(s);
                        self.put(&t, out, marks);
                    }
                }
                b"TJ" => {
                    if let (Some(Obj::Array(a)), Some(d)) = (stack.last(), &font) {
                        let d = std::rc::Rc::clone(d);
                        for x in a {
                            match x {
                                Obj::Str(s) => {
                                    let t = d.text(s);
                                    self.put(&t, out, marks);
                                }
                                Obj::Num(n) if *n < -250.0 && !out.ends_with(' ') => {
                                    self.put(" ", out, marks);
                                }
                                _ => {}
                            }
                        }
                    }
                }
                // marked content (ISO 32000-1 §14.6): its MCID, if any
                b"BDC" => {
                    if marks.stack.len() > 1024 {
                        return Err("marked content nested too deep".into());
                    }
                    let mcid = stack
                        .last()
                        .and_then(|p| p.get("MCID"))
                        .and_then(Obj::num)
                        .map(|n| n as i64);
                    marks.stack.push(mcid);
                }
                b"BMC" => {
                    if marks.stack.len() > 1024 {
                        return Err("marked content nested too deep".into());
                    }
                    marks.stack.push(None);
                }
                b"EMC" => {
                    if marks.stack.pop().is_none() {
                        marks.unbalanced = true;
                    }
                }
                b"Tm" => {
                    // a new line when the baseline moves (the runs of one line join)
                    let y = stack.get(5).and_then(Obj::num);
                    if let (Some(y), Some(prev)) = (y, line_y)
                        && (y - prev).abs() > 0.5
                        && !out.is_empty()
                        && !out.ends_with('\n')
                    {
                        out.push('\n');
                    }
                    line_y = y.or(line_y);
                }
                b"Do" => {
                    if let Some(Obj::Name(n)) = stack.first() {
                        let x = self.f.get(&xobjects, n)?;
                        match self.f.get(&x, "Subtype")?.name() {
                            Some("Image") => *images += 1,
                            Some("Form") => {
                                let inner = self.f.decode(&x)?;
                                let r = match self.f.get(&x, "Resources")? {
                                    Obj::Null => res.clone(),
                                    r => r,
                                };
                                // a form's marked content has its own key (else its caller's)
                                let key = self
                                    .f
                                    .get(&x, "StructParents")?
                                    .num()
                                    .map(|k| k as i64)
                                    .or(marks.key);
                                let inside = marks.inside || !marks.stack.is_empty();
                                let mut inner_marks = Marks {
                                    key,
                                    stack: Vec::new(),
                                    inside,
                                    unbalanced: false,
                                    untagged: &mut *marks.untagged,
                                };
                                self.content(&inner, &r, out, images, depth + 1, &mut inner_marks)?;
                                let bad = inner_marks.unbalanced || !inner_marks.stack.is_empty();
                                marks.unbalanced |= bad;
                            }
                            _ => {}
                        }
                    }
                }
                b"BI" => return Err("inline images are not read".into()),
                _ => {}
            }
            stack.clear();
        }
        Ok(())
    }
}

/// Reads `bytes` as a PDF: the structure, the pages and their text. Errors name what is wrong.
pub fn inspect(bytes: &[u8]) -> Result<PdfInfo, String> {
    let head = bytes.get(..8).ok_or("not a PDF: too short")?;
    if !head.starts_with(b"%PDF-") {
        return Err("not a PDF: no %PDF- header".into());
    }
    let version = String::from_utf8_lossy(head.get(5..8).unwrap_or(&[])).into_owned();
    let tail_from = bytes.len().saturating_sub(1024);
    let tail = bytes.get(tail_from..).unwrap_or(&[]);
    if find_last(tail, b"%%EOF").is_none() {
        return Err("no %%EOF at the end".into());
    }
    let sx = find_last(tail, b"startxref").ok_or("no startxref")? + tail_from;
    let mut lx = Lexer { b: bytes, i: sx + 9 };
    lx.skip_ws();
    let xref: usize = std::str::from_utf8(lx.word())
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or("bad startxref offset")?;
    let mut lx = Lexer { b: bytes, i: xref };
    if lx.word() != b"xref" {
        return Err("startxref does not point at a cross-reference table".into());
    }
    let mut offsets = HashMap::new();
    loop {
        lx.skip_ws();
        if bytes.get(lx.i..lx.i + 7) == Some(b"trailer") {
            lx.i += 7;
            break;
        }
        let start: usize = std::str::from_utf8(lx.word())
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or("bad xref subsection")?;
        lx.skip_ws();
        let count: usize = std::str::from_utf8(lx.word())
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or("bad xref subsection")?;
        if count > MAX_OBJECTS || offsets.len() + count > MAX_OBJECTS {
            return Err("too many objects".into());
        }
        for k in 0..count {
            lx.skip_ws();
            let off: usize = std::str::from_utf8(lx.word())
                .ok()
                .and_then(|s| s.parse().ok())
                .ok_or("bad xref entry")?;
            lx.skip_ws();
            let _gen = lx.word();
            lx.skip_ws();
            let kind = lx.word();
            if kind == b"n" {
                if off >= bytes.len() {
                    return Err(format!("object {} lies past the end of the file", start + k));
                }
                offsets.insert(start + k, off);
            } else if kind != b"f" {
                return Err("bad xref entry".into());
            }
        }
    }
    let trailer = lx.value(0)?;
    let f = File {
        b: bytes,
        offsets,
        cache: Default::default(),
    };
    // every object listed reads
    let mut ids: Vec<usize> = f.offsets.keys().copied().collect();
    ids.sort_unstable();
    for n in &ids {
        f.object(*n)?;
    }
    let root = f.get(&trailer, "Root")?;
    if root.get("Type").and_then(Obj::name) != Some("Catalog") {
        return Err("the trailer's Root is not a Catalog".into());
    }
    let title = text_of(&f.get(&f.get(&trailer, "Info")?, "Title")?);
    // the page tree, in order (with each page's object number)
    let mut leaves: Vec<Obj> = Vec::new();
    let mut page_ids: HashMap<usize, usize> = HashMap::new();
    let mut todo: Vec<(Obj, usize, Option<usize>)> = vec![(f.get(&root, "Pages")?, 0, None)];
    while let Some((node, depth, id)) = todo.pop() {
        if depth > MAX_DEPTH || leaves.len() > 100_000 {
            return Err("page tree too deep or too large".into());
        }
        match node.get("Type").and_then(Obj::name) {
            Some("Pages") => {
                let kids = f.get(&node, "Kids")?;
                let Obj::Array(k) = kids else {
                    return Err("a Pages node without Kids".into());
                };
                for kid in k.iter().rev() {
                    let kid_id = match kid {
                        Obj::Ref(n, _) => Some(*n),
                        _ => None,
                    };
                    todo.push((f.resolve(kid)?, depth + 1, kid_id));
                }
            }
            Some("Page") => {
                if let Some(id) = id {
                    page_ids.entry(id).or_insert(leaves.len());
                }
                leaves.push(node);
            }
            _ => return Err("a page tree node that is neither Pages nor Page".into()),
        }
    }
    let count = f.get(&f.get(&root, "Pages")?, "Count")?.num().unwrap_or(-1.0);
    if count != leaves.len() as f64 {
        return Err(format!(
            "the page tree's Count ({count}) is not its number of pages ({})",
            leaves.len()
        ));
    }
    let mut walk = Walk {
        f: &f,
        fonts: BTreeMap::new(),
        decoders: HashMap::new(),
        mcid_text: HashMap::new(),
    };
    let mut pages = Vec::new();
    for p in &leaves {
        let mb = f.get(p, "MediaBox")?;
        let Obj::Array(m) = &mb else {
            return Err("a page without a MediaBox".into());
        };
        let n = |i: usize| m.get(i).and_then(Obj::num).unwrap_or(0.0);
        let res = f.get(p, "Resources")?;
        let mut data = Vec::new();
        match f.get(p, "Contents")? {
            s @ Obj::Stream(..) => data = f.decode(&s)?,
            Obj::Array(a) => {
                for x in &a {
                    data.extend(f.decode(&f.resolve(x)?)?);
                    data.push(b'\n');
                }
            }
            _ => {}
        }
        let mut text = String::new();
        let mut images = 0usize;
        let mut untagged = String::new();
        let key = f.get(p, "StructParents")?.num().map(|k| k as i64);
        let mut marks = Marks {
            key,
            stack: Vec::new(),
            inside: false,
            unbalanced: false,
            untagged: &mut untagged,
        };
        walk.content(&data, &res, &mut text, &mut images, 0, &mut marks)?;
        let unbalanced = marks.unbalanced || !marks.stack.is_empty();
        pages.push(PageInfo {
            width: n(2) - n(0),
            height: n(3) - n(1),
            text,
            images,
            untagged_text: untagged,
            unbalanced_marks: unbalanced,
        });
    }
    let lang = text_of(&f.get(&root, "Lang")?);
    let marked = f.get(&f.get(&root, "MarkInfo")?, "Marked")? == Obj::Bool(true);
    let outline = outline(&f, &f.get(&root, "Outlines")?, &page_ids)?;
    let mut issues = Vec::new();
    let structure = structure(
        &f,
        &f.get(&root, "StructTreeRoot")?,
        &page_ids,
        &walk.mcid_text,
        &mut issues,
    )?;
    Ok(PdfInfo {
        version,
        objects: ids.len(),
        pages,
        fonts: walk.fonts.into_values().collect(),
        title,
        lang,
        marked,
        outline,
        structure,
        structure_issues: issues,
    })
}

/// The page a destination goes to (an explicit destination: `[page …]`).
fn dest_page(f: &File<'_>, item: &Obj, pages: &HashMap<usize, usize>) -> Result<Option<usize>, String> {
    let mut dest = f.get(item, "Dest")?;
    if dest == Obj::Null {
        // a GoTo action
        dest = f.get(&f.get(item, "A")?, "D")?;
    }
    Ok(match dest {
        Obj::Array(a) => match a.first() {
            Some(Obj::Ref(n, _)) => pages.get(n).copied(),
            _ => None,
        },
        _ => None,
    })
}

/// The outline's entries, depth first.
fn outline(f: &File<'_>, root: &Obj, pages: &HashMap<usize, usize>) -> Result<Vec<OutlineEntry>, String> {
    let mut out = Vec::new();
    // (the first child of a level, its depth)
    let mut todo: Vec<(Obj, usize)> = Vec::new();
    if let o @ Obj::Dict(_) = f.get(root, "First")? {
        todo.push((o, 0));
    }
    let mut seen = std::collections::HashSet::new();
    while let Some((item, depth)) = todo.pop() {
        if depth > MAX_DEPTH || out.len() >= MAX_NODES {
            return Err("outline too deep or too large".into());
        }
        out.push(OutlineEntry {
            depth,
            title: text_of(&f.get(&item, "Title")?).unwrap_or_default(),
            page: dest_page(f, &item, pages)?,
        });
        // the next sibling after this entry's children
        if let Some(Obj::Ref(n, _)) = item.get("Next")
            && seen.insert(*n)
        {
            todo.push((f.object(*n)?, depth));
        }
        if let Some(Obj::Ref(n, _)) = item.get("First")
            && seen.insert(*n)
        {
            todo.push((f.object(*n)?, depth + 1));
        }
    }
    Ok(out)
}

/// The parent tree: for each key, the elements of its marked-content ids (or the element of an
/// object reference).
fn parent_tree(f: &File<'_>, node: &Obj, out: &mut HashMap<i64, Obj>, depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH || out.len() > MAX_NODES {
        return Err("parent tree too deep or too large".into());
    }
    if let Obj::Array(nums) = f.get(node, "Nums")? {
        for pair in nums.chunks(2) {
            if let (Some(Obj::Num(k)), Some(v)) = (pair.first(), pair.get(1)) {
                out.insert(*k as i64, f.resolve(v)?);
            }
        }
    }
    if let Obj::Array(kids) = f.get(node, "Kids")? {
        for k in &kids {
            parent_tree(f, &f.resolve(k)?, out, depth + 1)?;
        }
    }
    Ok(())
}

/// The structure tree, depth first, with the text of each element's marked content; what does
/// not hold together goes to `issues`.
fn structure(
    f: &File<'_>,
    root: &Obj,
    pages: &HashMap<usize, usize>,
    mcid_text: &HashMap<(i64, i64), String>,
    issues: &mut Vec<String>,
) -> Result<Vec<StructNode>, String> {
    let mut out = Vec::new();
    if *root == Obj::Null {
        return Ok(out);
    }
    let mut parents = HashMap::new();
    parent_tree(f, &f.get(root, "ParentTree")?, &mut parents, 0)?;
    let page_of = |o: &Obj| match o {
        Obj::Ref(n, _) => pages.get(n).copied(),
        _ => None,
    };
    let kids = |o: &Obj| -> Vec<Obj> {
        match o.get("K") {
            Some(Obj::Array(a)) => a.clone(),
            Some(x) => vec![x.clone()],
            None => Vec::new(),
        }
    };
    // (an element's reference, its depth), in reverse document order
    let mut todo: Vec<(Obj, usize)> = kids(root).into_iter().rev().map(|k| (k, 0)).collect();
    let mut seen = std::collections::HashSet::new();
    while let Some((r, depth)) = todo.pop() {
        if depth > MAX_DEPTH || out.len() >= MAX_NODES {
            return Err("structure tree too deep or too large".into());
        }
        let id = match &r {
            Obj::Ref(n, _) => {
                if !seen.insert(*n) {
                    issues.push(format!("element {n} is reached twice"));
                    continue;
                }
                Some(*n)
            }
            _ => None,
        };
        let e = f.resolve(&r)?;
        if e.get("S").is_none() {
            continue;
        }
        let pg = e.get("Pg").cloned();
        let mut text = String::new();
        let mut children = Vec::new();
        for k in kids(&e) {
            let (mcid, kpg, stm) = match &k {
                Obj::Num(n) => (Some(*n as i64), pg.clone(), None),
                Obj::Dict(d) if d.get("Type").and_then(Obj::name) == Some("MCR") => (
                    d.get("MCID").and_then(Obj::num).map(|n| n as i64),
                    d.get("Pg").cloned().or_else(|| pg.clone()),
                    d.get("Stm").cloned(),
                ),
                Obj::Ref(..) => {
                    children.push(k);
                    continue;
                }
                _ => continue,
            };
            let Some(mcid) = mcid else { continue };
            // the key of the stream that holds the content
            let holder = match &stm {
                Some(s) => f.resolve(s)?,
                None => match &kpg {
                    Some(p) => f.resolve(p)?,
                    None => Obj::Null,
                },
            };
            let key = holder.get("StructParents").and_then(Obj::num).map(|k| k as i64);
            let Some(key) = key else {
                issues.push(format!(
                    "marked content {mcid} is in a stream without StructParents"
                ));
                continue;
            };
            // (graphics have no text; their id must still be in the parent tree)
            if let Some(t) = mcid_text.get(&(key, mcid)) {
                text.push_str(t);
            }
            let back = match parents.get(&key) {
                Some(Obj::Array(a)) => a.get(usize::try_from(mcid).unwrap_or(usize::MAX)).cloned(),
                _ => None,
            };
            let ok = matches!((&back, id), (Some(Obj::Ref(b, _)), Some(id)) if *b == id);
            if !ok {
                issues.push(format!(
                    "the parent tree does not map marked content {mcid} of key {key} back to its element"
                ));
            }
        }
        out.push(StructNode {
            depth,
            role: e.get("S").and_then(Obj::name).unwrap_or("").to_string(),
            title: e.get("T").and_then(text_of),
            alt: e.get("Alt").and_then(text_of),
            page: pg.as_ref().and_then(page_of),
            text,
        });
        for c in children.into_iter().rev() {
            todo.push((c, depth + 1));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_bytes_are_errors() {
        for b in [
            &b""[..],
            b"%PDF-1.7",
            b"%PDF-1.7\nstartxref\n99999\n%%EOF",
            b"%PDF-1.7\nxref\n0 1\n0000000000 65535 f \ntrailer\n<< /Root 9 0 R >>\nstartxref\n9\n%%EOF",
            b"%PDF-1.7\n1 0 obj\n<< /Length 999 >>\nstream\nab\nendstream\nendobj\nxref\n0 2\n0000000000 65535 f \n0000000009 00000 n \ntrailer\n<< >>\nstartxref\n62\n%%EOF",
            b"%PDF-1.7\n1 0 obj\n[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[\nendobj\nxref\n0 2\n0000000000 65535 f \n0000000009 00000 n \ntrailer\n<< >>\nstartxref\n93\n%%EOF",
        ] {
            assert!(inspect(b).is_err(), "{}", String::from_utf8_lossy(b));
        }
    }

    #[test]
    fn cmaps_parse() {
        let m = parse_cmap(b"2 beginbfchar\n<0003> <0020>\n<0024> <0041>\nendbfchar\n1 beginbfrange\n<0030> <0032> <0061>\nendbfrange\n");
        assert_eq!(m.get(&3).map(String::as_str), Some(" "));
        assert_eq!(m.get(&0x24).map(String::as_str), Some("A"));
        assert_eq!(m.get(&0x32).map(String::as_str), Some("c"));
    }
}
