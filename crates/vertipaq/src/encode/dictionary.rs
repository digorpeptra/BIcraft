//! `.dictionary` writer ([MS-XLDM] §2.3.2), the inverse of [`crate::parse_dictionary`].

use super::huffman::limited_code_lengths;
use super::{EncodeOptions, StringCompression};
use crate::Error;
use crate::dictionary::{
    CHARSET_MULTIPLE, CHARSET_SINGLE, DictionaryValues, INVALID_BIN_COUNT, PAGE_BEGIN_MARK, PAGE_END_MARK,
    XM_TYPE_LONG, XM_TYPE_REAL, XM_TYPE_STRING,
};
use crate::huffman::{BitWriter, HuffmanCode};

/// A written dictionary and the facts the metadata needs.
pub(super) struct Written {
    pub bytes: Vec<u8>,
    pub len: u64,
    /// 4 or 8 for long dictionaries, 8 for reals, `None` for strings.
    pub element_size: Option<u32>,
    /// Longest string in UTF-16 units.
    pub longest: u64,
    /// UTF-16 units stored, terminators included (uncompressed pages).
    pub used_chars: u64,
    /// Huffman-compressed pages and the sum of their buffer sizes in bytes.
    pub compressed_pages: u64,
    pub compressed_bytes: u64,
}

/// Hash headers Desktop writes: (algorithm, entry size, bin size, local entries).
const LONG_HASH: (i32, u32, u32, u32) = (-1, 8, 64, 6);
/// Long dictionaries with 8-byte elements (observed on engine-written columns of large values).
const WIDE_LONG_HASH: (i32, u32, u32, u32) = (-1, 16, 64, 3);
const REAL_HASH: (i32, u32, u32, u32) = (-1, 16, 64, 3);
const STRING_HASH: (i32, u32, u32, u32) = (0, 8, 64, 6);

fn header(out: &mut Vec<u8>, type_tag: i32, h: (i32, u32, u32, u32)) {
    out.extend_from_slice(&type_tag.to_le_bytes());
    out.extend_from_slice(&h.0.to_le_bytes());
    out.extend_from_slice(&h.1.to_le_bytes());
    out.extend_from_slice(&h.2.to_le_bytes());
    out.extend_from_slice(&h.3.to_le_bytes());
    out.extend_from_slice(&INVALID_BIN_COUNT.to_le_bytes());
}

pub(super) fn write(values: &DictionaryValues, options: &EncodeOptions) -> Result<Written, Error> {
    let mut out = Vec::new();
    match values {
        DictionaryValues::Long(v) => {
            let narrow = !options.wide_long_dictionary && v.iter().all(|&n| i32::try_from(n).is_ok());
            header(
                &mut out,
                XM_TYPE_LONG,
                if narrow { LONG_HASH } else { WIDE_LONG_HASH },
            );
            let size: u32 = if narrow { 4 } else { 8 };
            out.extend_from_slice(&(v.len() as u64).to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            for &n in v {
                if narrow {
                    out.extend_from_slice(&(n as i32).to_le_bytes());
                } else {
                    out.extend_from_slice(&n.to_le_bytes());
                }
            }
            Ok(numeric(out, v.len(), size))
        }
        DictionaryValues::Real(v) => {
            header(&mut out, XM_TYPE_REAL, REAL_HASH);
            out.extend_from_slice(&(v.len() as u64).to_le_bytes());
            out.extend_from_slice(&8u32.to_le_bytes());
            for r in v {
                out.extend_from_slice(&r.to_le_bytes());
            }
            Ok(numeric(out, v.len(), 8))
        }
        DictionaryValues::Text(v) => strings(out, v, options),
    }
}

fn numeric(bytes: Vec<u8>, len: usize, size: u32) -> Written {
    Written {
        bytes,
        len: len as u64,
        element_size: Some(size),
        longest: 0,
        used_chars: 0,
        compressed_pages: 0,
        compressed_bytes: 0,
    }
}

/// One page's strings as UTF-16 units.
struct Page {
    start: usize,
    strings: Vec<Vec<u16>>,
}

fn strings(mut out: Vec<u8>, values: &[String], options: &EncodeOptions) -> Result<Written, Error> {
    header(&mut out, XM_TYPE_STRING, STRING_HASH);
    let limit = options.max_page_chars.clamp(1, u64::from(u32::MAX));
    // Split into pages by character count (terminators included).
    let mut pages: Vec<Page> = Vec::new();
    let mut chars_on_page = 0u64;
    let mut longest = 0u64;
    for (i, s) in values.iter().enumerate() {
        let units: Vec<u16> = s.encode_utf16().collect();
        longest = longest.max(units.len() as u64);
        let need = units.len() as u64 + 1;
        let start_new = match pages.last() {
            None => true,
            Some(_) => chars_on_page.saturating_add(need) > limit,
        };
        if start_new {
            pages.push(Page {
                start: i,
                strings: Vec::new(),
            });
            chars_on_page = 0;
        }
        chars_on_page = chars_on_page.saturating_add(need);
        if let Some(p) = pages.last_mut() {
            p.strings.push(units);
        }
    }
    if u32::try_from(pages.len()).is_err() {
        return Err(Error::LimitExceeded(format!("{} dictionary pages", pages.len())));
    }
    out.extend_from_slice(&(values.len() as u64).to_le_bytes());
    out.push(1); // store_compressed: Desktop writes 1 even without compressed pages
    out.extend_from_slice(&longest.to_le_bytes());
    out.extend_from_slice(&(pages.len() as u64).to_le_bytes());
    let mut handles: Vec<(u32, u32)> = Vec::with_capacity(values.len());
    let mut used_total = 0u64;
    let mut compressed_pages = 0u64;
    let mut compressed_bytes = 0u64;
    for (page_no, page) in pages.iter().enumerate() {
        let page_no = page_no as u32;
        let (store, offsets, compressed) = page_store(page, options.string_compression)?;
        // mask: 1 on compressed pages, 0 on plain ones (engine-written dictionaries)
        out.extend_from_slice(&u64::from(compressed).to_le_bytes());
        out.push(u8::from(page.strings.iter().any(Vec::is_empty)));
        out.extend_from_slice(&(page.start as u64).to_le_bytes());
        out.extend_from_slice(&(page.strings.len() as u64).to_le_bytes());
        out.push(u8::from(compressed));
        out.extend_from_slice(&PAGE_BEGIN_MARK);
        out.extend_from_slice(&store);
        out.extend_from_slice(&PAGE_END_MARK);
        if compressed {
            // the store starts with total bits (u32), the character-set mode (u32) and the
            // buffer size (u64)
            let buffer = store
                .get(8..16)
                .and_then(|b| <[u8; 8]>::try_from(b).ok())
                .map_or(0, u64::from_le_bytes);
            compressed_pages += 1;
            compressed_bytes = compressed_bytes.saturating_add(buffer);
        } else {
            used_total += page.strings.iter().map(|s| s.len() as u64 + 1).sum::<u64>();
        }
        handles.extend(offsets.into_iter().map(|o| (o, page_no)));
    }
    out.extend_from_slice(&(handles.len() as u64).to_le_bytes());
    out.extend_from_slice(&8u32.to_le_bytes());
    for (offset, page) in handles {
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&page.to_le_bytes());
    }
    Ok(Written {
        bytes: out,
        len: values.len() as u64,
        element_size: None,
        longest,
        used_chars: used_total,
        compressed_pages,
        compressed_bytes,
    })
}

/// The store of one page (between the marks), the record-handle offsets, and whether it is
/// compressed.
fn page_store(page: &Page, mode: StringCompression) -> Result<(Vec<u8>, Vec<u32>, bool), Error> {
    let plain = uncompressed(page)?;
    let units: u64 = page.strings.iter().map(|s| s.len() as u64 + 1).sum();
    let large = units >= super::COMPRESS_PAGE_UNITS;
    let packed = match mode {
        StringCompression::Never => None,
        // a page of one distinct symbol stays plain (engine: 8 645 units of 'a' were not coded)
        StringCompression::LargePages if !large || symbol_count(page) < 2 => None,
        StringCompression::LargePages | StringCompression::WhenSmaller | StringCompression::Always => {
            compressed(page)
        }
    };
    match (mode, packed) {
        (StringCompression::Always | StringCompression::LargePages, Some(c)) => Ok((c.0, c.1, true)),
        (StringCompression::WhenSmaller, Some(c)) if c.0.len() < plain.0.len() => Ok((c.0, c.1, true)),
        _ => Ok((plain.0, plain.1, false)),
    }
}

/// Distinct Huffman symbols of a page (bytes of its UTF-16 units, as [`compressed`] codes them),
/// counted up to 2.
fn symbol_count(page: &Page) -> usize {
    let mut units = page.strings.iter().flatten();
    let Some(&first) = units.next() else {
        return 0;
    };
    let [lo, hi] = first.to_le_bytes();
    let single = page.strings.iter().flatten().all(|u| (u >> 8) as u8 == hi);
    let one = if single {
        page.strings.iter().flatten().all(|u| (u & 0xFF) as u8 == lo)
    } else {
        // multiple character sets code both bytes of each unit
        false
    };
    if one { 1 } else { 2 }
}

fn uncompressed(page: &Page) -> Result<(Vec<u8>, Vec<u32>), Error> {
    let used: u64 = page.strings.iter().map(|s| s.len() as u64 + 1).sum();
    let mut out = Vec::new();
    out.extend_from_slice(&0u64.to_le_bytes()); // remaining characters
    out.extend_from_slice(&used.to_le_bytes());
    out.extend_from_slice(&used.saturating_mul(2).to_le_bytes());
    let mut offsets = Vec::with_capacity(page.strings.len());
    let mut at = 0u64;
    for s in &page.strings {
        offsets.push(u32::try_from(at).map_err(|_| too_large())?);
        for u in s {
            out.extend_from_slice(&u.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]);
        at += s.len() as u64 + 1;
    }
    Ok((out, offsets))
}

fn too_large() -> Error {
    Error::LimitExceeded("string page too large".into())
}

/// Huffman-compressed store; `None` when the page cannot be compressed (no characters, or
/// more than 2^32 bits).
fn compressed(page: &Page) -> Option<(Vec<u8>, Vec<u32>)> {
    // Single character set when every unit shares its high byte.
    let first_high = page.strings.iter().flatten().next().map(|u| (u >> 8) as u8)?;
    let single = page
        .strings
        .iter()
        .flatten()
        .all(|u| (u >> 8) as u8 == first_high);
    let symbols = |s: &[u16]| -> Vec<u8> {
        if single {
            s.iter().map(|u| (u & 0xFF) as u8).collect()
        } else {
            s.iter().flat_map(|u| u.to_le_bytes()).collect()
        }
    };
    let mut freqs = [0u64; 256];
    for s in &page.strings {
        for b in symbols(s) {
            if let Some(f) = freqs.get_mut(usize::from(b)) {
                *f += 1;
            }
        }
    }
    let lengths = limited_code_lengths(&freqs);
    let code = HuffmanCode::from_lengths(&lengths).ok()?;
    let mut table = [(0u32, 0u8); 256];
    for (sym, slot) in table.iter_mut().enumerate() {
        if let Some(c) = code.codeword(sym as u8) {
            *slot = c;
        }
    }
    let mut w = BitWriter::new();
    let mut offsets = Vec::with_capacity(page.strings.len());
    for s in &page.strings {
        offsets.push(u32::try_from(w.bit_len()).ok()?);
        for b in symbols(s) {
            let (c, l) = table.get(usize::from(b)).copied().unwrap_or((0, 0));
            w.push(c, l);
        }
    }
    let total_bits = u32::try_from(w.bit_len()).ok()?;
    let max_len = lengths.iter().copied().max().unwrap_or(0);
    let mut buffer = w.into_bytes();
    // The engine's buffer has room for one more 16-bit word than the bits need (121 pages).
    let words = usize::try_from(total_bits.div_ceil(16)).ok()?.checked_add(1)?;
    buffer.resize(words.checked_mul(2)?, 0);
    let mut out = Vec::new();
    out.extend_from_slice(&total_bits.to_le_bytes());
    out.extend_from_slice(&(if single { CHARSET_SINGLE } else { CHARSET_MULTIPLE }).to_le_bytes());
    out.extend_from_slice(&(buffer.len() as u64).to_le_bytes());
    if single {
        out.push(first_high);
    }
    out.extend_from_slice(&u32::from(max_len.min(12)).to_le_bytes());
    out.extend_from_slice(&code.encode_array());
    out.extend_from_slice(&(buffer.len() as u64).to_le_bytes());
    out.extend_from_slice(&buffer);
    Some((out, offsets))
}
