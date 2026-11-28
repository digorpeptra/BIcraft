//! A TrueType subset for PDF embedding (`pdf.rs`): the glyphs a document uses, kept at their
//! glyph ids (the PDF maps CIDs to glyph ids one to one), every other glyph emptied. Only
//! `glyf`-based faces are subset; the tables kept are the ones a PDF reader needs to draw
//! (`head`, `hhea`, `maxp`, `hmtx`, `loca`, `glyf`, the hinting tables) plus `name` (the
//! font's copyright and licence notices travel with it), `OS/2`, `cmap` and a format 3 `post`.
//! The layout is the OpenType specification's ("The OpenType Font File", `glyf`, `loca`).
//!
//! Input is hostile: every read is bounds-checked, composite glyphs are followed to a bounded
//! depth, and sizes come from the file's own tables only.

use std::collections::BTreeSet;

/// Facts about a face that a PDF font descriptor needs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FaceInfo {
    pub upem: u16,
    pub num_glyphs: u16,
    /// `head` bounding box (font units).
    pub bbox: [i16; 4],
    pub ascent: i16,
    pub descent: i16,
    pub cap_height: i16,
    pub italic_angle: f32,
    /// PostScript name (name id 6), else the family name.
    pub ps_name: String,
    /// `OS/2` `fsType`.
    pub fs_type: u16,
    /// Has `glyf` and `loca` (else CFF outlines: not subset here).
    pub glyf: bool,
    /// Has a `CFF ` table (OpenType with PostScript outlines: embedded whole).
    pub cff: bool,
}

impl FaceInfo {
    /// Whether the face's licence lets a document embed a subset of it (`OS/2` `fsType`:
    /// restricted licence embedding, no subsetting and bitmap-only embedding refuse it).
    pub fn embeddable(&self) -> bool {
        let usage = self.fs_type & 0x000f;
        let restricted = usage == 0x0002;
        !restricted && self.fs_type & 0x0100 == 0 && self.fs_type & 0x0200 == 0
    }

    /// Whether the licence lets a document embed the whole face (a CFF face, which is not
    /// subset): "no subsetting" allows it; restricted and bitmap-only embedding do not.
    pub fn embeddable_whole(&self) -> bool {
        self.fs_type & 0x000f != 0x0002 && self.fs_type & 0x0200 == 0
    }
}

/// Largest face embedded whole (bytes of its tables).
const MAX_WHOLE_FACE: usize = 16 << 20;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([*s.first()?, *s.get(1)?]))
}

fn i16_at(b: &[u8], at: usize) -> Option<i16> {
    u16_at(b, at).map(|x| x as i16)
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([
        *s.first()?,
        *s.get(1)?,
        *s.get(2)?,
        *s.get(3)?,
    ]))
}

/// The offset of face `index`'s table directory (a TrueType collection, or 0 for a single face).
fn face_offset(data: &[u8], index: u32) -> Option<usize> {
    if data.get(0..4) == Some(b"ttcf") {
        let n = u32_at(data, 8)?;
        if index >= n {
            return None;
        }
        let at = 12usize.checked_add(usize::try_from(index).ok()?.checked_mul(4)?)?;
        usize::try_from(u32_at(data, at)?).ok()
    } else if index == 0 {
        Some(0)
    } else {
        None
    }
}

/// The face's tables: (tag, bytes), in directory order.
fn tables(data: &[u8], index: u32) -> Option<Vec<([u8; 4], &[u8])>> {
    let off = face_offset(data, index)?;
    let n = usize::from(u16_at(data, off.checked_add(4)?)?);
    if n > 256 {
        return None;
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let rec = off.checked_add(12)?.checked_add(i.checked_mul(16)?)?;
        let tag = data.get(rec..rec.checked_add(4)?)?;
        let o = usize::try_from(u32_at(data, rec.checked_add(8)?)?).ok()?;
        let l = usize::try_from(u32_at(data, rec.checked_add(12)?)?).ok()?;
        let body = data.get(o..o.checked_add(l)?)?;
        let mut t = [0u8; 4];
        t.copy_from_slice(tag);
        out.push((t, body));
    }
    Some(out)
}

fn table<'a>(ts: &[([u8; 4], &'a [u8])], tag: &[u8; 4]) -> Option<&'a [u8]> {
    ts.iter().find(|(t, _)| t == tag).map(|(_, b)| *b)
}

/// A `name` table string (Windows Unicode or Mac Roman) for `id`.
fn name_string(name: &[u8], id: u16) -> Option<String> {
    let count = usize::from(u16_at(name, 2)?);
    let storage = usize::from(u16_at(name, 4)?);
    let mut fallback = None;
    for i in 0..count.min(2000) {
        let r = 6usize.checked_add(i.checked_mul(12)?)?;
        let (pid, nid) = (u16_at(name, r)?, u16_at(name, r + 6)?);
        if nid != id {
            continue;
        }
        let len = usize::from(u16_at(name, r + 8)?);
        let at = storage.checked_add(usize::from(u16_at(name, r + 10)?))?;
        let Some(bytes) = name.get(at..at.checked_add(len)?) else {
            continue;
        };
        if pid == 3 || pid == 0 {
            let units: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| u16::from_be_bytes(*p))
                .collect();
            if let Ok(s) = String::from_utf16(&units) {
                return Some(s);
            }
        } else if pid == 1 && fallback.is_none() {
            fallback = Some(bytes.iter().map(|b| char::from(*b)).collect());
        }
    }
    fallback
}

/// A PDF name made of the safe characters of `s` (letters, digits, `-`, `_`), at most 60.
fn pdf_safe(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(60)
        .collect()
}

/// What a PDF needs to know about face `index` of `data`.
pub(crate) fn info(data: &[u8], index: u32) -> Option<FaceInfo> {
    let ts = tables(data, index)?;
    let head = table(&ts, b"head")?;
    let hhea = table(&ts, b"hhea")?;
    let maxp = table(&ts, b"maxp")?;
    let os2 = table(&ts, b"OS/2");
    let post = table(&ts, b"post");
    let name = table(&ts, b"name");
    let upem = u16_at(head, 18)?.clamp(16, 16384);
    let ps = name
        .and_then(|n| name_string(n, 6).or_else(|| name_string(n, 1)))
        .map(|s| pdf_safe(&s))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Font".into());
    let cap = os2
        .filter(|o| u16_at(o, 0).is_some_and(|v| v >= 2))
        .and_then(|o| i16_at(o, 88))
        .unwrap_or_else(|| (i32::from(upem) * 7 / 10) as i16);
    let italic = post
        .and_then(|p| u32_at(p, 4))
        .map(|f| (f as i32) as f32 / 65536.0)
        .filter(|a| a.is_finite() && a.abs() < 90.0)
        .unwrap_or(0.0);
    Some(FaceInfo {
        upem,
        num_glyphs: u16_at(maxp, 4)?,
        bbox: [
            i16_at(head, 36)?,
            i16_at(head, 38)?,
            i16_at(head, 40)?,
            i16_at(head, 42)?,
        ],
        ascent: i16_at(hhea, 4)?,
        descent: i16_at(hhea, 6)?,
        cap_height: cap,
        italic_angle: italic,
        ps_name: ps,
        fs_type: os2.and_then(|o| u16_at(o, 8)).unwrap_or(0),
        glyf: table(&ts, b"glyf").is_some() && table(&ts, b"loca").is_some(),
        cff: table(&ts, b"CFF ").is_some(),
    })
}

/// Advance widths (font units) of `gids`, from `hmtx` (a glyph past the long metrics takes the
/// last one's advance).
pub(crate) fn advances(data: &[u8], index: u32, gids: &BTreeSet<u16>) -> Vec<(u16, u16)> {
    let Some(ts) = tables(data, index) else {
        return Vec::new();
    };
    let (Some(hhea), Some(hmtx)) = (table(&ts, b"hhea"), table(&ts, b"hmtx")) else {
        return Vec::new();
    };
    let long = usize::from(u16_at(hhea, 34).unwrap_or(0));
    if long == 0 {
        return Vec::new();
    }
    gids.iter()
        .map(|&g| {
            let i = usize::from(g).min(long - 1);
            (g, u16_at(hmtx, i * 4).unwrap_or(0))
        })
        .collect()
}

/// The `loca` offsets of every glyph (`num_glyphs + 1` entries) as byte ranges in `glyf`.
fn loca(head: &[u8], loca: &[u8], num_glyphs: usize) -> Option<Vec<u32>> {
    let long = i16_at(head, 50)? != 0;
    (0..=num_glyphs)
        .map(|i| {
            if long {
                u32_at(loca, i.checked_mul(4)?)
            } else {
                u16_at(loca, i.checked_mul(2)?).map(|x| u32::from(x) * 2)
            }
        })
        .collect()
}

/// The glyph ids a composite glyph's components name.
fn components(glyph: &[u8]) -> Vec<u16> {
    let mut out = Vec::new();
    if i16_at(glyph, 0).is_none_or(|n| n >= 0) {
        return out;
    }
    let mut at = 10usize;
    for _ in 0..256 {
        let (Some(flags), Some(gid)) = (u16_at(glyph, at), u16_at(glyph, at + 2)) else {
            break;
        };
        out.push(gid);
        at += 4;
        at += if flags & 0x0001 != 0 { 4 } else { 2 };
        if flags & 0x0008 != 0 {
            at += 2;
        } else if flags & 0x0040 != 0 {
            at += 4;
        } else if flags & 0x0080 != 0 {
            at += 8;
        }
        if flags & 0x0020 == 0 {
            break;
        }
    }
    out
}

/// The OpenType table checksum.
fn checksum(b: &[u8]) -> u32 {
    let mut sum = 0u32;
    for c in b.chunks(4) {
        let mut w = [0u8; 4];
        for (d, s) in w.iter_mut().zip(c) {
            *d = *s;
        }
        sum = sum.wrapping_add(u32::from_be_bytes(w));
    }
    sum
}

/// A subset of face `index` of `data` with the glyphs `used` (plus `.notdef` and the
/// components of composite glyphs) at their own glyph ids; `None` for a face that is not
/// `glyf`-based or whose tables do not read.
pub(crate) fn subset(data: &[u8], index: u32, used: &BTreeSet<u16>) -> Option<Vec<u8>> {
    let ts = tables(data, index)?;
    let head = table(&ts, b"head")?;
    let glyf = table(&ts, b"glyf")?;
    let maxp = table(&ts, b"maxp")?;
    let n = usize::from(u16_at(maxp, 4)?);
    let offs = loca(head, table(&ts, b"loca")?, n)?;
    let glyph = |g: usize| -> &[u8] {
        let (Some(a), Some(b)) = (offs.get(g), offs.get(g + 1)) else {
            return &[];
        };
        let (a, b) = (*a as usize, *b as usize);
        if a >= b {
            &[]
        } else {
            glyf.get(a..b).unwrap_or(&[])
        }
    };
    // the closure: used glyphs, .notdef and composite components (bounded)
    let mut keep: BTreeSet<usize> = used.iter().map(|g| usize::from(*g)).filter(|g| *g < n).collect();
    keep.insert(0);
    let mut todo: Vec<usize> = keep.iter().copied().collect();
    let mut steps = 0usize;
    while let Some(g) = todo.pop() {
        steps += 1;
        if steps > 100_000 {
            break;
        }
        for c in components(glyph(g)) {
            let c = usize::from(c);
            if c < n && keep.insert(c) {
                todo.push(c);
            }
        }
    }
    // glyf and a long-format loca
    let mut new_glyf: Vec<u8> = Vec::new();
    let mut new_loca: Vec<u8> = Vec::with_capacity((n + 1) * 4);
    for g in 0..n {
        new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
        if keep.contains(&g) {
            new_glyf.extend_from_slice(glyph(g));
            while !new_glyf.len().is_multiple_of(4) {
                new_glyf.push(0);
            }
        }
    }
    new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
    let mut new_head = head.to_vec();
    if new_head.len() < 54 {
        return None;
    }
    new_head[8..12].copy_from_slice(&[0, 0, 0, 0]);
    new_head[50..52].copy_from_slice(&1i16.to_be_bytes());
    // post format 3: no glyph names
    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
    if let Some(p) = table(&ts, b"post")
        && let Some(x) = p.get(4..16)
    {
        post[4..16].copy_from_slice(x);
    }
    let mut out_tables: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    for (tag, body) in &ts {
        let b: Option<Vec<u8>> = match tag {
            b"head" => Some(new_head.clone()),
            b"glyf" => Some(new_glyf.clone()),
            b"loca" => Some(new_loca.clone()),
            b"post" => Some(post.clone()),
            b"hhea" | b"maxp" | b"hmtx" | b"cvt " | b"fpgm" | b"prep" | b"name" | b"OS/2" | b"cmap"
            | b"gasp" => Some(body.to_vec()),
            _ => None,
        };
        if let Some(b) = b {
            out_tables.push((*tag, b));
        }
    }
    if !out_tables.iter().any(|(t, _)| t == b"post") {
        out_tables.push((*b"post", post));
    }
    Some(sfnt(0x0001_0000, out_tables))
}

/// Face `index` of `data` as a file of its own, every table kept: a CFF face is embedded whole
/// (`FontFile3` of subtype `OpenType`), since it is not subset. `None` when it is not a CFF
/// face, its tables do not read, or it is larger than 16 MB.
pub(crate) fn opentype_face(data: &[u8], index: u32) -> Option<Vec<u8>> {
    let ts = tables(data, index)?;
    table(&ts, b"CFF ")?;
    let total = ts.iter().try_fold(0usize, |a, (_, b)| a.checked_add(b.len()))?;
    if total > MAX_WHOLE_FACE {
        return None;
    }
    let mut tables: Vec<([u8; 4], Vec<u8>)> = ts.iter().map(|(t, b)| (*t, b.to_vec())).collect();
    // the checksum adjustment is computed again over the new file
    if let Some((_, head)) = tables.iter_mut().find(|(t, _)| t == b"head")
        && let Some(s) = head.get_mut(8..12)
    {
        s.copy_from_slice(&[0; 4]);
    }
    Some(sfnt(u32::from_be_bytes(*b"OTTO"), tables))
}

/// An OpenType file of `tables` (at most 4,095): the offset table with `version`, the table
/// directory in tag order, the tables padded to 4 bytes, and `head`'s checksum adjustment.
pub(crate) fn sfnt(version: u32, mut out_tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    out_tables.truncate(4095);
    out_tables.sort_by_key(|a| a.0);
    // the file: offset table, directory, tables padded to 4 bytes
    let nt = out_tables.len() as u16;
    let mut es = 0u16;
    while (1u32 << (es + 1)) <= u32::from(nt) {
        es += 1;
    }
    let sr = (1u16 << es) * 16;
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(&version.to_be_bytes());
    out.extend_from_slice(&nt.to_be_bytes());
    out.extend_from_slice(&sr.to_be_bytes());
    out.extend_from_slice(&es.to_be_bytes());
    out.extend_from_slice(&(nt * 16 - sr).to_be_bytes());
    let mut at = 12 + usize::from(nt) * 16;
    let mut body: Vec<u8> = Vec::new();
    let mut head_at = None;
    for (tag, b) in &out_tables {
        out.extend_from_slice(tag);
        out.extend_from_slice(&checksum(b).to_be_bytes());
        out.extend_from_slice(&(at as u32).to_be_bytes());
        out.extend_from_slice(&(b.len() as u32).to_be_bytes());
        if tag == b"head" {
            head_at = Some(at);
        }
        body.extend_from_slice(b);
        while !body.len().is_multiple_of(4) {
            body.push(0);
        }
        at = 12 + usize::from(nt) * 16 + body.len();
    }
    out.extend_from_slice(&body);
    if let Some(h) = head_at {
        let adj = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
        if let Some(s) = out.get_mut(h + 8..h + 12) {
            s.copy_from_slice(&adj.to_be_bytes());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::{NOTO_SANS_REGULAR, SELAWIK_REGULAR};

    #[test]
    fn selawik_subsets_and_reads_back() {
        let i = info(SELAWIK_REGULAR, 0).unwrap();
        assert!(i.glyf && i.embeddable() && i.num_glyphs > 100, "{i:?}");
        assert!(i.ps_name.starts_with("Selawik"), "{}", i.ps_name);
        let used: BTreeSet<u16> = [36u16, 37, 38].into_iter().collect();
        let s = subset(SELAWIK_REGULAR, 0, &used).unwrap();
        assert!(
            s.len() < SELAWIK_REGULAR.len() / 2,
            "{} vs {}",
            s.len(),
            SELAWIK_REGULAR.len()
        );
        // the subset parses with skrifa, keeps the glyph count and the used outlines
        let f = skrifa::FontRef::new(&s).unwrap();
        use skrifa::MetadataProvider;
        use skrifa::raw::TableProvider;
        assert_eq!(f.maxp().unwrap().num_glyphs(), i.num_glyphs);
        let og = f.outline_glyphs();
        assert!(og.get(skrifa::GlyphId::new(36)).is_some());
        // whole-file checksum is the magic number
        assert_eq!(checksum(&s), 0xB1B0_AFBA);
        let adv = advances(SELAWIK_REGULAR, 0, &used);
        assert_eq!(adv.len(), 3);
        assert!(adv.iter().all(|(_, w)| *w > 0));
    }

    #[test]
    fn noto_subset_is_small() {
        let used: BTreeSet<u16> = (1u16..20).collect();
        let s = subset(NOTO_SANS_REGULAR, 0, &used).unwrap();
        assert!(s.len() < NOTO_SANS_REGULAR.len(), "{}", s.len());
        assert!(info(&s, 0).is_some());
    }

    #[test]
    fn restricted_faces_are_not_embedded() {
        let mut i = info(SELAWIK_REGULAR, 0).unwrap();
        for (fs, ok) in [
            (0, true),
            (2, false),
            (4, true),
            (8, true),
            (0x100, false),
            (0x200, false),
        ] {
            i.fs_type = fs;
            assert_eq!(i.embeddable(), ok, "{fs:#x}");
        }
    }

    #[test]
    fn hostile_fonts_do_not_panic() {
        let used: BTreeSet<u16> = [1u16, 2, 65535].into_iter().collect();
        for b in [&b""[..], b"ttcf", b"\0\x01\0\0\0\xff", &SELAWIK_REGULAR[..200]] {
            let _ = info(b, 0);
            let _ = subset(b, 0, &used);
            let _ = subset(b, 3, &used);
            let _ = advances(b, 0, &used);
        }
        // every truncation of a real face
        for cut in (0..SELAWIK_REGULAR.len()).step_by(997) {
            let b = &SELAWIK_REGULAR[..cut];
            let _ = info(b, 0);
            let _ = subset(b, 0, &used);
        }
        // a composite glyph pointing at itself stays bounded
        assert!(components(&[0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x20, 0, 5]).len() <= 256);
    }
}
