//! `.idfmeta`: per-segment compression and statistics of one `.idf` file.
//!
//! A tagged little-endian binary: ASCII open/close tags (`<1:CP\0` … `CP:1>\0`) around
//! fixed fields. Layout in `docs/formats/vertipaq.md`; field names follow the CC0 pbixray
//! `idfmeta.ksy`.

use crate::compression::{Compression, HYBRID_RLE};
use crate::cursor::Cursor;
use crate::{Error, FileKind};

const CP_OPEN: &[u8] = b"<1:CP\0";
const CP_CLOSE: &[u8] = b"CP:1>\0";
const CS_OPEN: &[u8] = b"<1:CS\0";
const CS_CLOSE: &[u8] = b"CS:1>\0";
const SS_OPEN: &[u8] = b"<1:SS\0";
const SS_CLOSE: &[u8] = b"SS:1>\0";
const SDOS_OPEN: &[u8] = b"<1:SDOs\0";
const SDOS_CLOSE: &[u8] = b"SDOs:1>\0";
const CSDOS_OPEN: &[u8] = b"<1:CSDOs\0";
const CSDOS_CLOSE: &[u8] = b"CSDOs:1>\0";

/// Smallest possible encoded segment (`CS` tags, two u64, the sub-segment flag).
const MIN_SEGMENT_BYTES: usize = 6 + 8 + 8 + 1 + 6;

/// A parsed `.idfmeta` file: the column partition (`CP`) element.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnMeta {
    pub segments: Vec<SegmentMeta>,
    /// `SDOs`: where each segment's blocks sit in the `.idf` (absent in some writers).
    pub data_objects: Option<Vec<SegmentDataObject>>,
}

/// One column segment (`CS` element).
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentMeta {
    /// Rows in the segment.
    pub records: u64,
    /// Observed 1 for hybrid RLE segments and 0 otherwise; meaning unknown.
    pub base_id: u64,
    pub compression_class: u32,
    pub sub_compression_class: u32,
    /// Present exactly when `compression_class` is hybrid RLE.
    pub rle: Option<RleFacet>,
    /// First data id of the segment (observed equal to the first row's id).
    pub first_run_value: u32,
    pub stats: SegmentStats,
    /// The bit-packed sub-segment of a hybrid segment.
    pub sub_segment: Option<SubSegmentMeta>,
}

/// RLE facet of a hybrid segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RleFacet {
    pub bookmark_bits: u64,
    /// Allocated primary-segment storage, in `u32` units (two per RLE entry).
    pub storage_alloc_size: u64,
    /// Used primary-segment storage, in `u32` units (two per RLE entry).
    pub storage_used_size: u64,
    pub segment_needs_resizing: u8,
}

/// Segment statistics (`SS` element). They mirror `ColumnStorage.Statistics_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentStats {
    pub distinct_states: u64,
    pub min_data_id: u32,
    pub max_data_id: u32,
    pub original_min_segment_data_id: u32,
    pub rle_sort_order: i64,
    pub row_count: u64,
    pub has_nulls: u8,
    pub rle_runs: u64,
    pub others_rle_runs: u64,
}

/// Sub-segment element nested in a primary `CS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubSegmentMeta {
    /// Number of bit-packed values stored in the sub-segment.
    pub records: u64,
    pub base_id: u64,
}

/// `CSDOs` entry: one segment's blocks in the `.idf`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentDataObject {
    pub primary: DataObject,
    pub sub: Option<DataObject>,
}

/// A block of the `.idf`: byte offset of its `u64` size field and its size in 8-byte units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataObject {
    pub offset: u64,
    pub words: u64,
}

impl SegmentMeta {
    /// The compression of this segment, if the class ids are known.
    pub fn compression(&self) -> Option<Compression> {
        Compression::from_classes(self.compression_class, self.sub_compression_class)
    }
}

/// Parses an `.idfmeta` file.
pub fn parse_idfmeta(bytes: &[u8]) -> Result<ColumnMeta, Error> {
    let mut c = Cursor::new(bytes, FileKind::IdfMeta);
    c.expect(CP_OPEN, "<1:CP")?;
    let count = c.u64("segment count")?;
    let max = c.remaining() / MIN_SEGMENT_BYTES;
    if usize::try_from(count).map_or(true, |n| n > max) {
        return Err(c.malformed(format!("segment count {count} exceeds the file size")));
    }
    let mut segments = Vec::new();
    for _ in 0..count {
        segments.push(read_segment(&mut c)?);
    }
    c.expect(CP_CLOSE, "CP:1>")?;
    let data_objects = if c.is_at_end() {
        None
    } else {
        c.expect(SDOS_OPEN, "<1:SDOs")?;
        let mut objects = Vec::with_capacity(segments.len());
        for seg in &segments {
            c.expect(CSDOS_OPEN, "<1:CSDOs")?;
            let primary = read_data_object(&mut c)?;
            let sub = if seg.sub_segment.is_some() {
                c.expect(CSDOS_OPEN, "<1:CSDOs")?;
                let d = read_data_object(&mut c)?;
                c.expect(CSDOS_CLOSE, "CSDOs:1>")?;
                Some(d)
            } else {
                None
            };
            c.expect(CSDOS_CLOSE, "CSDOs:1>")?;
            objects.push(SegmentDataObject { primary, sub });
        }
        c.expect(SDOS_CLOSE, "SDOs:1>")?;
        Some(objects)
    };
    if !c.is_at_end() && !bytes.get(c.pos()..).unwrap_or(&[]).iter().all(|&b| b == 0) {
        return Err(c.malformed("trailing bytes after the column partition"));
    }
    Ok(ColumnMeta {
        segments,
        data_objects,
    })
}

fn read_data_object(c: &mut Cursor<'_>) -> Result<DataObject, Error> {
    Ok(DataObject {
        offset: c.u64("data object offset")?,
        words: c.u64("data object size")?,
    })
}

fn read_segment(c: &mut Cursor<'_>) -> Result<SegmentMeta, Error> {
    c.expect(CS_OPEN, "<1:CS")?;
    let records = c.u64("records")?;
    let base_id = c.u64("base id")?;
    let compression_class = c.u32("compression class")?;
    let sub_compression_class = c.u32("sub compression class")?;
    let rle = if compression_class == HYBRID_RLE {
        Some(RleFacet {
            bookmark_bits: c.u64("bookmark bits")?,
            storage_alloc_size: c.u64("storage alloc size")?,
            storage_used_size: c.u64("storage used size")?,
            segment_needs_resizing: c.u8("needs resizing")?,
        })
    } else {
        None
    };
    let first_run_value = c.u32("first run value")?;
    c.expect(SS_OPEN, "<1:SS")?;
    let stats = SegmentStats {
        distinct_states: c.u64("distinct states")?,
        min_data_id: c.u32("min data id")?,
        max_data_id: c.u32("max data id")?,
        original_min_segment_data_id: c.u32("original min data id")?,
        rle_sort_order: c.i64("rle sort order")?,
        row_count: c.u64("row count")?,
        has_nulls: c.u8("has nulls")?,
        rle_runs: c.u64("rle runs")?,
        others_rle_runs: c.u64("others rle runs")?,
    };
    c.expect(SS_CLOSE, "SS:1>")?;
    let has_sub = c.u8("has sub-segment")?;
    let sub_segment = if has_sub != 0 {
        c.expect(CS_OPEN, "<1:CS")?;
        let sub = SubSegmentMeta {
            records: c.u64("sub-segment records")?,
            base_id: c.u64("sub-segment base id")?,
        };
        if c.u8("sub-segment has sub-segment")? != 0 {
            return Err(c.malformed("a sub-segment cannot nest another sub-segment"));
        }
        c.expect(CS_CLOSE, "CS:1>")?;
        Some(sub)
    } else {
        None
    };
    c.expect(CS_CLOSE, "CS:1>")?;
    Ok(SegmentMeta {
        records,
        base_id,
        compression_class,
        sub_compression_class,
        rle,
        first_run_value,
        stats,
        sub_segment,
    })
}

impl ColumnMeta {
    /// Total rows over all segments (`None` on overflow).
    pub fn row_count(&self) -> Option<u64> {
        self.segments
            .iter()
            .try_fold(0u64, |acc, s| acc.checked_add(s.records))
    }

    /// Serializes back to the `.idfmeta` byte layout (inverse of [`parse_idfmeta`]).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(CP_OPEN);
        out.extend_from_slice(&(self.segments.len() as u64).to_le_bytes());
        for s in &self.segments {
            out.extend_from_slice(CS_OPEN);
            out.extend_from_slice(&s.records.to_le_bytes());
            out.extend_from_slice(&s.base_id.to_le_bytes());
            out.extend_from_slice(&s.compression_class.to_le_bytes());
            out.extend_from_slice(&s.sub_compression_class.to_le_bytes());
            if s.compression_class == HYBRID_RLE {
                let r = s.rle.unwrap_or(RleFacet {
                    bookmark_bits: 0,
                    storage_alloc_size: 0,
                    storage_used_size: 0,
                    segment_needs_resizing: 0,
                });
                out.extend_from_slice(&r.bookmark_bits.to_le_bytes());
                out.extend_from_slice(&r.storage_alloc_size.to_le_bytes());
                out.extend_from_slice(&r.storage_used_size.to_le_bytes());
                out.push(r.segment_needs_resizing);
            }
            out.extend_from_slice(&s.first_run_value.to_le_bytes());
            let st = &s.stats;
            out.extend_from_slice(SS_OPEN);
            out.extend_from_slice(&st.distinct_states.to_le_bytes());
            out.extend_from_slice(&st.min_data_id.to_le_bytes());
            out.extend_from_slice(&st.max_data_id.to_le_bytes());
            out.extend_from_slice(&st.original_min_segment_data_id.to_le_bytes());
            out.extend_from_slice(&st.rle_sort_order.to_le_bytes());
            out.extend_from_slice(&st.row_count.to_le_bytes());
            out.push(st.has_nulls);
            out.extend_from_slice(&st.rle_runs.to_le_bytes());
            out.extend_from_slice(&st.others_rle_runs.to_le_bytes());
            out.extend_from_slice(SS_CLOSE);
            match &s.sub_segment {
                Some(sub) => {
                    out.push(1);
                    out.extend_from_slice(CS_OPEN);
                    out.extend_from_slice(&sub.records.to_le_bytes());
                    out.extend_from_slice(&sub.base_id.to_le_bytes());
                    out.push(0);
                    out.extend_from_slice(CS_CLOSE);
                }
                None => out.push(0),
            }
            out.extend_from_slice(CS_CLOSE);
        }
        out.extend_from_slice(CP_CLOSE);
        if let Some(objects) = &self.data_objects {
            out.extend_from_slice(SDOS_OPEN);
            for (o, s) in objects.iter().zip(&self.segments) {
                out.extend_from_slice(CSDOS_OPEN);
                out.extend_from_slice(&o.primary.offset.to_le_bytes());
                out.extend_from_slice(&o.primary.words.to_le_bytes());
                if s.sub_segment.is_some() {
                    let sub = o.sub.unwrap_or(DataObject { offset: 0, words: 0 });
                    out.extend_from_slice(CSDOS_OPEN);
                    out.extend_from_slice(&sub.offset.to_le_bytes());
                    out.extend_from_slice(&sub.words.to_le_bytes());
                    out.extend_from_slice(CSDOS_CLOSE);
                }
                out.extend_from_slice(CSDOS_CLOSE);
            }
            out.extend_from_slice(SDOS_CLOSE);
        }
        out
    }
}

/// True if `bytes` looks like an `.idfmeta` file (starts with the `CP` tag).
pub fn is_idfmeta(bytes: &[u8]) -> bool {
    Cursor::new(bytes, FileKind::IdfMeta).peek_is(CP_OPEN)
}
