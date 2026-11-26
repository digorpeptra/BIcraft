//! `.idf`: column data as data ids, one per row ([MS-XLDM] §2.3.1, §2.7.1, §2.7.3).
//!
//! The file is a sequence of blocks `{u64 size in 8-byte units, size × 8 bytes}`. A hybrid
//! RLE segment has two blocks (the RLE entries, then the bit-packed sub-segment); every
//! other segment has one. The `.idfmeta` says which kind each segment is.

use crate::bitpack;
use crate::compression::{Compression, InnerCompression};
use crate::cursor::Cursor;
use crate::idfmeta::{ColumnMeta, SegmentMeta};
use crate::{Error, FileKind, Limits};

/// Data id that stands for a blank (NULL) value.
pub const NULL_DATA_ID: u32 = 2;
/// First data id given to a real value.
pub const FIRST_DATA_ID: u32 = 3;

/// The raw blocks of one segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdfSegment<'a> {
    /// Primary block: RLE entries for hybrid segments, packed words otherwise.
    pub primary: &'a [u8],
    /// Bit-packed sub-segment of a hybrid segment.
    pub sub: Option<&'a [u8]>,
}

/// One 8-byte entry of a hybrid RLE primary segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RleEntry {
    /// `count` rows with data id `data_id`.
    Run { data_id: u32, count: u32 },
    /// `count` rows taken from the bit-packed sub-segment, starting at `offset`.
    BitPacked { offset: u64, count: u32 },
}

/// Splits an `.idf` into its segments' blocks, using the compression in `meta`.
pub fn split_idf<'a>(idf: &'a [u8], meta: &ColumnMeta) -> Result<Vec<IdfSegment<'a>>, Error> {
    let mut c = Cursor::new(idf, FileKind::Idf);
    let mut out = Vec::with_capacity(meta.segments.len().min(idf.len() / 8));
    for (i, seg) in meta.segments.iter().enumerate() {
        let compression = compression_of(seg)?;
        let expected = meta.data_objects.as_ref().and_then(|d| d.get(i));
        let primary_at = c.pos() as u64;
        let words = c.u64("segment size")?;
        let primary = c.take_array(words, 8, "segment")?;
        if let Some(d) = expected
            && (d.primary.offset != primary_at || d.primary.words != words)
        {
            return Err(c.malformed(format!("segment {i} does not match its .idfmeta data object")));
        }
        let sub = if matches!(compression, Compression::HybridRle(_)) {
            let sub_at = c.pos() as u64;
            let words = c.u64("sub-segment size")?;
            let sub = c.take_array(words, 8, "sub-segment")?;
            if let Some(d) = expected.and_then(|d| d.sub)
                && (d.offset != sub_at || d.words != words)
            {
                return Err(c.malformed(format!("sub-segment {i} does not match its .idfmeta data object")));
            }
            Some(sub)
        } else {
            None
        };
        out.push(IdfSegment { primary, sub });
    }
    Ok(out)
}

/// Byte ranges of one segment's blocks in an `.idf` ([`locate_idf`]): the primary block and the
/// sub-segment of a hybrid RLE segment, without their size words.
pub type IdfRanges = (std::ops::Range<u64>, Option<std::ops::Range<u64>>);

/// Where each segment's blocks are in an `.idf` of `len` bytes that is not in memory, as
/// [`split_idf`] finds them: `read_u64(offset)` reads the little-endian word at `offset` of the
/// file (`None`: it cannot). The same checks as [`split_idf`].
pub fn locate_idf(
    len: u64,
    meta: &ColumnMeta,
    read_u64: &mut dyn FnMut(u64) -> Option<u64>,
) -> Result<Vec<IdfRanges>, Error> {
    let bad = |at: u64, m: String| Error::Malformed {
        kind: FileKind::Idf,
        offset: usize::try_from(at).unwrap_or(usize::MAX),
        message: m,
    };
    let mut block = |at: &mut u64, what: &str| -> Result<std::ops::Range<u64>, Error> {
        let head = *at;
        let words = head
            .checked_add(8)
            .filter(|e| *e <= len)
            .and_then(|_| read_u64(head))
            .ok_or_else(|| bad(head, format!("{what} size past the end")))?;
        let start = head + 8;
        let end = words
            .checked_mul(8)
            .and_then(|b| start.checked_add(b))
            .filter(|e| *e <= len)
            .ok_or_else(|| bad(head, format!("{what} past the end")))?;
        *at = end;
        Ok(start..end)
    };
    let mut at = 0u64;
    let mut out = Vec::with_capacity(
        meta.segments
            .len()
            .min(usize::try_from(len / 8).unwrap_or(usize::MAX)),
    );
    for (i, seg) in meta.segments.iter().enumerate() {
        let compression = compression_of(seg)?;
        let expected = meta.data_objects.as_ref().and_then(|d| d.get(i));
        let primary_at = at;
        let primary = block(&mut at, "segment")?;
        if let Some(d) = expected
            && (d.primary.offset != primary_at
                || d.primary.words.checked_mul(8) != Some(primary.end - primary.start))
        {
            return Err(bad(
                primary_at,
                format!("segment {i} does not match its .idfmeta data object"),
            ));
        }
        let sub = if matches!(compression, Compression::HybridRle(_)) {
            let sub_at = at;
            let sub = block(&mut at, "sub-segment")?;
            if let Some(d) = expected.and_then(|d| d.sub)
                && (d.offset != sub_at || d.words.checked_mul(8) != Some(sub.end - sub.start))
            {
                return Err(bad(
                    sub_at,
                    format!("sub-segment {i} does not match its .idfmeta data object"),
                ));
            }
            Some(sub)
        } else {
            None
        };
        out.push((primary, sub));
    }
    Ok(out)
}

/// Iterates the RLE entries of a hybrid primary block (including trailing padding entries).
pub fn rle_entries(primary: &[u8]) -> impl Iterator<Item = RleEntry> + '_ {
    let mut packed: u64 = 0;
    primary.as_chunks::<8>().0.iter().map(move |e| {
        let value = u32::from_le_bytes([e[0], e[1], e[2], e[3]]);
        let count = u32::from_le_bytes([e[4], e[5], e[6], e[7]]);
        // A bit-packed entry stores -1 - (values already bit-packed) as its "value".
        let marker = u64::from(u32::MAX).checked_sub(packed);
        if marker == Some(u64::from(value)) {
            let offset = packed;
            packed = packed.saturating_add(u64::from(count));
            RleEntry::BitPacked { offset, count }
        } else {
            RleEntry::Run {
                data_id: value,
                count,
            }
        }
    })
}

fn compression_of(seg: &SegmentMeta) -> Result<Compression, Error> {
    seg.compression().ok_or_else(|| Error::Unsupported {
        kind: FileKind::IdfMeta,
        message: format!(
            "compression class {} / {}",
            seg.compression_class, seg.sub_compression_class
        ),
    })
}

/// Decodes an `.idf` into one data id per row, all segments in order.
pub fn decode_data_ids(idf: &[u8], meta: &ColumnMeta, limits: &Limits) -> Result<Vec<u32>, Error> {
    let total = meta
        .row_count()
        .ok_or_else(|| Error::LimitExceeded("row count overflows".into()))?;
    check_rows(total, ID_BYTES, limits)?;
    let blocks = split_idf(idf, meta)?;
    // No reservation from the declared row count: each segment reserves its rows only once
    // its blocks show they are there.
    let mut out = Vec::new();
    for (seg, block) in meta.segments.iter().zip(&blocks) {
        decode_segment(seg, block, limits, &mut out)?;
    }
    Ok(out)
}

/// Decodes one segment, appending `seg.records` data ids to `out`.
pub fn decode_segment(
    seg: &SegmentMeta,
    block: &IdfSegment<'_>,
    limits: &Limits,
    out: &mut Vec<u32>,
) -> Result<(), Error> {
    check_rows(seg.records, ID_BYTES, limits)?;
    let records = usize::try_from(seg.records)
        .map_err(|_| Error::LimitExceeded(format!("{} rows in a segment", seg.records)))?;
    match compression_of(seg)? {
        Compression::NoSplit(width) => {
            let base = u32::try_from(seg.base_id).map_err(|_| malformed("segment base id"))?;
            let values = bitpack::unpack(block.primary, width, records)
                .ok_or_else(|| malformed("bit-packed segment is shorter than its row count"))?;
            out.reserve(values.len());
            for v in values {
                out.push(v.checked_add(base).ok_or_else(|| malformed("data id overflow"))?);
            }
        }
        Compression::Xm123 => push_sequence(out, seg.stats.min_data_id, records)?,
        Compression::HybridRle(inner) => {
            let packed_count = seg
                .sub_segment
                .map_or(Ok(0), |s| usize::try_from(s.records))
                .map_err(|_| malformed("sub-segment record count"))?;
            if packed_count > records {
                return Err(malformed("more bit-packed values than rows"));
            }
            // The entries must cover the rows before anything is allocated for them.
            let covered = rle_coverage(block.primary, records);
            if covered != records {
                return Err(malformed(format!(
                    "RLE entries cover {covered} of {records} rows"
                )));
            }
            out.reserve(records);
            let mut packed = Vec::new();
            match inner {
                InnerCompression::NoSplit(width) => {
                    // A segment holding blanks packs relative to the null id.
                    let base = if seg.stats.has_nulls != 0 {
                        NULL_DATA_ID
                    } else {
                        seg.stats.min_data_id
                    };
                    let sub = block.sub.unwrap_or(&[]);
                    let raw = bitpack::unpack(sub, width, packed_count).ok_or_else(|| {
                        malformed("bit-packed sub-segment is shorter than its record count")
                    })?;
                    packed.reserve(raw.len());
                    for v in raw {
                        packed.push(v.checked_add(base).ok_or_else(|| malformed("data id overflow"))?);
                    }
                }
                InnerCompression::Xm123 => push_sequence(&mut packed, seg.stats.min_data_id, packed_count)?,
            }
            let mut produced = 0usize;
            for entry in rle_entries(block.primary) {
                if produced == records {
                    break;
                }
                match entry {
                    RleEntry::Run { data_id, count } => {
                        let n = usize::try_from(count)
                            .unwrap_or(usize::MAX)
                            .min(records - produced);
                        out.extend(std::iter::repeat_n(data_id, n));
                        produced += n;
                    }
                    RleEntry::BitPacked { offset, count } => {
                        let n = usize::try_from(count)
                            .unwrap_or(usize::MAX)
                            .min(records - produced);
                        let from = usize::try_from(offset).unwrap_or(usize::MAX);
                        let slice = from
                            .checked_add(n)
                            .and_then(|to| packed.get(from..to))
                            .ok_or_else(|| malformed("RLE entry points past the bit-packed values"))?;
                        out.extend_from_slice(slice);
                        produced += n;
                    }
                }
            }
            if produced != records {
                return Err(malformed(format!(
                    "RLE entries cover {produced} of {records} rows"
                )));
            }
        }
    }
    Ok(())
}

/// Rows the RLE entries of `primary` describe, counted up to `records`.
fn rle_coverage(primary: &[u8], records: usize) -> usize {
    let mut covered = 0usize;
    for entry in rle_entries(primary) {
        if covered == records {
            break;
        }
        let (RleEntry::Run { count, .. } | RleEntry::BitPacked { count, .. }) = entry;
        let left = records.saturating_sub(covered);
        covered = covered.saturating_add(usize::try_from(count).unwrap_or(usize::MAX).min(left));
    }
    covered
}

/// Bytes of one decoded data id.
pub(crate) const ID_BYTES: u64 = 4;

/// Checks `rows` against [`Limits::max_rows`] and `rows × bytes_per_row` against
/// [`Limits::max_bytes`], before anything that size is allocated.
pub(crate) fn check_rows(rows: u64, bytes_per_row: u64, limits: &Limits) -> Result<(), Error> {
    if rows > limits.max_rows {
        return Err(Error::LimitExceeded(format!(
            "{rows} rows (limit {})",
            limits.max_rows
        )));
    }
    match rows.checked_mul(bytes_per_row) {
        Some(bytes) if bytes <= limits.max_bytes => Ok(()),
        _ => Err(Error::LimitExceeded(format!(
            "{rows} rows exceed the {}-byte decoding limit",
            limits.max_bytes
        ))),
    }
}

fn push_sequence(out: &mut Vec<u32>, first: u32, count: usize) -> Result<(), Error> {
    let count32 = u32::try_from(count).map_err(|_| malformed("sequence too long"))?;
    first
        .checked_add(count32)
        .ok_or_else(|| malformed("data id overflow"))?;
    out.extend((0..count32).map(|i| first + i));
    Ok(())
}

fn malformed(message: impl Into<String>) -> Error {
    Error::Malformed {
        kind: FileKind::Idf,
        offset: 0,
        message: message.into(),
    }
}
