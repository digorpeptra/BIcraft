//! `.dictionary`: the distinct values of a hash-encoded column ([MS-XLDM] §2.3.2).
//!
//! Dictionary index `i` holds the value of data id [`FIRST_DATA_ID`]` + i`.

use crate::cursor::Cursor;
use crate::huffman::HuffmanCode;
use crate::idf::FIRST_DATA_ID;
use crate::{Error, FileKind};

/// `XM_TYPE_LONG`.
pub const XM_TYPE_LONG: i32 = 0;
/// `XM_TYPE_REAL`.
pub const XM_TYPE_REAL: i32 = 1;
/// `XM_TYPE_STRING`.
pub const XM_TYPE_STRING: i32 = 2;
/// Hash bin count meaning "no hash table follows" (`XM_HASH_BIN_VECTOR_INVALID_BIN_COUNT`).
pub const INVALID_BIN_COUNT: u64 = u64::MAX;
/// Marker before each page's string store.
pub const PAGE_BEGIN_MARK: [u8; 4] = [0xDD, 0xCC, 0xBB, 0xAA];
/// Marker after each page's string store.
pub const PAGE_END_MARK: [u8; 4] = [0xCD, 0xAB, 0xCD, 0xAB];
/// Character-set mode id of single-character-set Huffman pages.
pub const CHARSET_SINGLE: u32 = 703_121;
/// Character-set mode id of multiple-character-set Huffman pages.
pub const CHARSET_MULTIPLE: u32 = 703_122;

/// Smallest possible page (header, marks, the smaller store header).
const MIN_PAGE_BYTES: usize = 26 + 4 + 24 + 4;

/// The five required hash elements that start every dictionary ([MS-XLDM] §2.3.3.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HashHeader {
    pub algorithm: i32,
    pub entry_size: u32,
    pub bin_size: u32,
    pub local_entry_count: u32,
    pub bin_count: u64,
}

/// The decoded values, in data-id order.
#[derive(Debug, Clone, PartialEq)]
pub enum DictionaryValues {
    Long(Vec<i64>),
    Real(Vec<f64>),
    Text(Vec<String>),
}

/// A parsed `.dictionary`.
#[derive(Debug, Clone, PartialEq)]
pub struct Dictionary {
    /// `XM_TYPE_*` tag.
    pub type_tag: i32,
    pub hash: HashHeader,
    /// Element size of a numeric vector (4 or 8), `None` for strings.
    pub element_size: Option<u32>,
    pub values: DictionaryValues,
    /// Store and page headers of a string dictionary.
    pub string_store: Option<StringStore>,
}

/// String-store header (§2.3.2.1.2.3) plus per-page headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringStore {
    pub string_count: u64,
    /// Observed 1 even when no page is compressed.
    pub store_compressed: u8,
    pub longest_string: u64,
    pub pages: Vec<PageInfo>,
    pub handles: Vec<RecordHandle>,
}

/// Header of one string page (§2.3.2.1.2.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageInfo {
    pub mask: u64,
    pub contains_nulls: u8,
    pub start_index: u64,
    pub string_count: u64,
    pub store: PageStore,
}

/// The two store variants of a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageStore {
    /// NUL-terminated UTF-16LE strings.
    Uncompressed {
        remaining_chars: u64,
        used_chars: u64,
        allocation_bytes: u64,
    },
    /// Huffman-coded strings.
    Compressed {
        total_bits: u32,
        charset_mode: u32,
        charset: Option<u8>,
        decode_bits: u32,
        encode_array: Vec<u8>,
        allocation_bytes: u64,
        buffer_bytes: u64,
    },
}

/// Record handle (§2.3.2.1.3.5): where one string starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordHandle {
    /// Character offset (uncompressed page) or bit offset (compressed page).
    pub offset: u32,
    pub page: u32,
}

impl Dictionary {
    /// Number of values.
    pub fn len(&self) -> usize {
        match &self.values {
            DictionaryValues::Long(v) => v.len(),
            DictionaryValues::Real(v) => v.len(),
            DictionaryValues::Text(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Index of `data_id` in [`Dictionary::values`], if it is a value id in range.
    pub fn index_of(&self, data_id: u32) -> Option<usize> {
        let i = usize::try_from(data_id.checked_sub(FIRST_DATA_ID)?).ok()?;
        (i < self.len()).then_some(i)
    }
}

/// Parses a `.dictionary` file and decodes all its values.
pub fn parse_dictionary(bytes: &[u8]) -> Result<Dictionary, Error> {
    let mut c = Cursor::new(bytes, FileKind::Dictionary);
    let type_tag = c.i32("dictionary type")?;
    let hash = HashHeader {
        algorithm: c.i32("hash algorithm")?,
        entry_size: c.u32("hash entry size")?,
        bin_size: c.u32("hash bin size")?,
        local_entry_count: c.u32("local entry count")?,
        bin_count: c.u64("bin count")?,
    };
    if hash.bin_count != INVALID_BIN_COUNT {
        return Err(c.malformed("dictionary carries a hash table (bin count is not -1)"));
    }
    let (element_size, values, string_store) = match type_tag {
        XM_TYPE_LONG | XM_TYPE_REAL => {
            let count = c.u64("value count")?;
            let size = c.u32("element size")?;
            let data = c.take_array(count, usize::try_from(size).unwrap_or(usize::MAX), "values")?;
            let values = match (type_tag, size) {
                (XM_TYPE_LONG, 4) => DictionaryValues::Long(
                    data.as_chunks::<4>()
                        .0
                        .iter()
                        .map(|b| i64::from(i32::from_le_bytes(*b)))
                        .collect(),
                ),
                (XM_TYPE_LONG, 8) => {
                    DictionaryValues::Long(crate::cursor::words(data).map(|w| w as i64).collect())
                }
                (XM_TYPE_REAL, 8) => {
                    DictionaryValues::Real(crate::cursor::words(data).map(f64::from_bits).collect())
                }
                _ => return Err(c.malformed(format!("element size {size} for type {type_tag}"))),
            };
            (Some(size), values, None)
        }
        XM_TYPE_STRING => {
            let (store, strings) = read_strings(&mut c, bytes.len())?;
            (None, DictionaryValues::Text(strings), Some(store))
        }
        other => {
            return Err(Error::Unsupported {
                kind: FileKind::Dictionary,
                message: format!("dictionary type {other}"),
            });
        }
    };
    if !bytes.get(c.pos()..).unwrap_or(&[]).iter().all(|&b| b == 0) {
        return Err(c.malformed("trailing bytes after the dictionary"));
    }
    Ok(Dictionary {
        type_tag,
        hash,
        element_size,
        values,
        string_store,
    })
}

/// Raw bytes of a page's store, kept for the decoding pass.
struct PageData<'a> {
    buffer: &'a [u8],
    code: Option<HuffmanCode>,
}

fn read_strings(c: &mut Cursor<'_>, input_len: usize) -> Result<(StringStore, Vec<String>), Error> {
    let string_count = c.u64("string count")?;
    let store_compressed = c.u8("store compressed flag")?;
    let longest_string = c.u64("longest string")?;
    let page_count = c.u64("page count")?;
    if usize::try_from(page_count).map_or(true, |n| n > c.remaining() / MIN_PAGE_BYTES) {
        return Err(c.malformed(format!("page count {page_count} exceeds the file size")));
    }
    let mut pages = Vec::new();
    let mut data = Vec::new();
    for _ in 0..page_count {
        let mask = c.u64("page mask")?;
        let contains_nulls = c.u8("page contains nulls")?;
        let start_index = c.u64("page start index")?;
        let page_strings = c.u64("page string count")?;
        let compressed = c.u8("page compressed flag")?;
        c.expect(&PAGE_BEGIN_MARK, "page begin mark")?;
        let (store, buffer, code) = match compressed {
            0 => {
                let remaining_chars = c.u64("remaining characters")?;
                let used_chars = c.u64("used characters")?;
                let allocation_bytes = c.u64("allocation size")?;
                if used_chars.checked_mul(2).is_none_or(|n| n > allocation_bytes) {
                    return Err(c.malformed("used characters exceed the page allocation"));
                }
                let buffer = c.take_array(allocation_bytes, 1, "character buffer")?;
                let used = buffer.get(..(used_chars as usize) * 2).unwrap_or(buffer);
                let store = PageStore::Uncompressed {
                    remaining_chars,
                    used_chars,
                    allocation_bytes,
                };
                (store, used, None)
            }
            1 => {
                let total_bits = c.u32("store total bits")?;
                let charset_mode = c.u32("character set mode")?;
                let allocation_bytes = c.u64("allocation size")?;
                let charset = match charset_mode {
                    CHARSET_SINGLE => Some(c.u8("character set")?),
                    CHARSET_MULTIPLE => None,
                    other => {
                        return Err(Error::Unsupported {
                            kind: FileKind::Dictionary,
                            message: format!("character set mode {other}"),
                        });
                    }
                };
                let decode_bits = c.u32("decode bits")?;
                let encode_array = c.take(128, "encode array")?;
                let code = HuffmanCode::from_encode_array(encode_array).map_err(|m| c.malformed(m))?;
                let buffer_bytes = c.u64("buffer size")?;
                let buffer = c.take_array(buffer_bytes, 1, "compressed strings")?;
                if u64::from(total_bits) > buffer_bytes.saturating_mul(8) {
                    return Err(c.malformed("store total bits exceed the buffer"));
                }
                let store = PageStore::Compressed {
                    total_bits,
                    charset_mode,
                    charset,
                    decode_bits,
                    encode_array: encode_array.to_vec(),
                    allocation_bytes,
                    buffer_bytes,
                };
                (store, buffer, Some(code))
            }
            other => return Err(c.malformed(format!("page compressed flag {other}"))),
        };
        c.expect(&PAGE_END_MARK, "page end mark")?;
        pages.push(PageInfo {
            mask,
            contains_nulls,
            start_index,
            string_count: page_strings,
            store,
        });
        data.push(PageData { buffer, code });
    }
    let handle_count = c.u64("record handle count")?;
    let handle_size = c.u32("record handle size")?;
    if handle_size != 8 {
        return Err(c.malformed(format!("record handle size {handle_size}")));
    }
    if handle_count != string_count {
        return Err(c.malformed(format!(
            "{handle_count} record handles for {string_count} strings"
        )));
    }
    let raw = c.take_array(handle_count, 8, "record handles")?;
    let handles: Vec<RecordHandle> = raw
        .as_chunks::<8>()
        .0
        .iter()
        .map(|h| RecordHandle {
            offset: u32::from_le_bytes([h[0], h[1], h[2], h[3]]),
            page: u32::from_le_bytes([h[4], h[5], h[6], h[7]]),
        })
        .collect();
    // Strings are stored in handle order, so a compressed string ends where the next
    // handle of the same page starts (equal offsets: an empty string).
    let mut next_on_page: Vec<Option<usize>> = vec![None; handles.len()];
    let mut last_on_page: Vec<Option<usize>> = vec![None; data.len()];
    for (k, h) in handles.iter().enumerate() {
        let page = h.page as usize;
        let last = last_on_page
            .get_mut(page)
            .ok_or_else(|| c.malformed(format!("record handle names missing page {}", h.page)))?;
        if let Some(prev) = *last
            && let Some(slot) = next_on_page.get_mut(prev)
        {
            *slot = Some(k);
        }
        *last = Some(k);
    }
    // Decoded output is bounded by the input: codes are at least 1 bit long (the engine gives a
    // dominant symbol a 1-bit code), so at most 8 symbols per compressed byte.
    let mut budget = input_len.saturating_mul(8).saturating_add(1024);
    let mut strings = Vec::with_capacity(handles.len());
    for (k, h) in handles.iter().enumerate() {
        let (Some(info), Some(page)) = (pages.get(h.page as usize), data.get(h.page as usize)) else {
            return Err(c.malformed("record handle names a missing page"));
        };
        let next = next_on_page
            .get(k)
            .copied()
            .flatten()
            .and_then(|n| handles.get(n))
            .map(|n| n.offset);
        let units = decode_string(info, page, h.offset, next).map_err(|m| Error::Malformed {
            kind: FileKind::Dictionary,
            offset: 0,
            message: format!("string at page {} offset {}: {m}", h.page, h.offset),
        })?;
        budget = budget
            .checked_sub(units.len())
            .ok_or_else(|| Error::LimitExceeded("decoded strings exceed the input bound".into()))?;
        strings.push(String::from_utf16_lossy(&units));
    }
    let store = StringStore {
        string_count,
        store_compressed,
        longest_string,
        pages,
        handles,
    };
    Ok((store, strings))
}

/// UTF-16 code units of the string starting at `offset` on `page`; `next` is the offset of
/// the following string on the same page.
fn decode_string(
    info: &PageInfo,
    page: &PageData<'_>,
    offset: u32,
    next: Option<u32>,
) -> Result<Vec<u16>, &'static str> {
    match (&info.store, &page.code) {
        (PageStore::Uncompressed { .. }, _) => {
            let start = (offset as usize)
                .checked_mul(2)
                .filter(|&s| s <= page.buffer.len())
                .ok_or("character offset past the used characters")?;
            let rest = page.buffer.get(start..).unwrap_or(&[]);
            Ok(rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_le_bytes(*b))
                .take_while(|&u| u != 0)
                .collect())
        }
        (
            PageStore::Compressed {
                total_bits, charset, ..
            },
            Some(code),
        ) => {
            let end = next.unwrap_or(*total_bits);
            let bytes = code.decode(page.buffer, u64::from(offset), u64::from(end))?;
            Ok(match charset {
                Some(cs) => bytes
                    .iter()
                    .map(|&b| (u16::from(*cs) << 8) | u16::from(b))
                    .collect(),
                None => bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| u16::from_le_bytes(*b))
                    .collect(),
            })
        }
        (PageStore::Compressed { .. }, None) => Err("compressed page without a code"),
    }
}
