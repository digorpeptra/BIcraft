//! Synthetic VertiPaq column files (`.idf` + `.idfmeta`, and a `.dictionary` for hash
//! encoding), laid out as `docs/formats/vertipaq.md` describes: hybrid RLE segments whose
//! entries are either runs of one data id or pointers into a bit-packed sub-segment.

use bicraft_vertipaq::idfmeta::{DataObject, RleFacet, SegmentDataObject, SegmentStats, SubSegmentMeta};
use bicraft_vertipaq::{ColumnMeta, Compression, InnerCompression, SegmentMeta, bitpack};

use super::synth::Rng;

/// Rows per segment (Power BI's default segment size).
pub const SEGMENT_ROWS: usize = 1 << 20;

/// First data id of user values (0..=2 are reserved; 2 is blank).
const FIRST_ID: u32 = 3;

/// `.idf`, `.idfmeta` and the data id of every row.
pub type ColumnFiles = (Vec<u8>, Vec<u8>, Vec<u32>);

/// A column of `rows` data ids in `FIRST_ID..FIRST_ID + distinct`: alternating RLE runs (about
/// half the rows) and bit-packed stretches. Returns `(idf, idfmeta, ids)`.
pub fn hybrid_column(rows: usize, distinct: u32, seed: u64) -> Result<ColumnFiles, String> {
    let distinct = distinct.max(2);
    let width = (32 - (distinct - 1).leading_zeros()).max(1) as u8;
    // only widths the format has: round up to the next one that packs
    let width = (width..=32)
        .find(|&w| {
            let c = Compression::HybridRle(InnerCompression::NoSplit(w));
            Compression::from_classes(c.class_id(), c.sub_class_id()) == Some(c)
        })
        .ok_or("no bit width")?;
    let mut rng = Rng::new(seed);
    let mut idf = Vec::new();
    let mut metas = Vec::new();
    let mut objects = Vec::new();
    let mut all = Vec::with_capacity(rows);
    let mut left = rows;
    while left > 0 {
        let seg_rows = left.min(SEGMENT_ROWS);
        left -= seg_rows;
        let mut entries: Vec<(u32, u32)> = Vec::new();
        let mut packed: Vec<u32> = vec![FIRST_ID]; // the minimum, so the packing base is FIRST_ID
        let mut filled = 1usize;
        entries.push((u32::MAX, 1));
        all.push(FIRST_ID);
        while filled < seg_rows {
            let n = (16 + rng.below(240) as usize).min(seg_rows - filled);
            if rng.below(2) == 0 {
                let id = FIRST_ID + rng.below(u64::from(distinct)) as u32;
                entries.push((id, n as u32));
                all.extend(std::iter::repeat_n(id, n));
            } else {
                entries.push((u32::MAX - packed.len() as u32, n as u32));
                for _ in 0..n {
                    let id = FIRST_ID + rng.below(u64::from(distinct)) as u32;
                    packed.push(id);
                    all.push(id);
                }
            }
            filled += n;
        }
        let alloc = entries.len().max(16);
        let primary_at = idf.len() as u64;
        idf.extend_from_slice(&(alloc as u64).to_le_bytes());
        for k in 0..alloc {
            let (v, c) = entries.get(k).copied().unwrap_or((0, 0));
            idf.extend_from_slice(&v.to_le_bytes());
            idf.extend_from_slice(&c.to_le_bytes());
        }
        let rel: Vec<u32> = packed.iter().map(|v| v - FIRST_ID).collect();
        let words = bitpack::pack(&rel, width).ok_or("bitpack failed")?;
        let sub_at = idf.len() as u64;
        idf.extend_from_slice(&((words.len() / 8) as u64).to_le_bytes());
        idf.extend_from_slice(&words);
        let c = Compression::HybridRle(InnerCompression::NoSplit(width));
        metas.push(SegmentMeta {
            records: seg_rows as u64,
            base_id: 1,
            compression_class: c.class_id(),
            sub_compression_class: c.sub_class_id(),
            rle: Some(RleFacet {
                bookmark_bits: 4,
                storage_alloc_size: 2 * alloc as u64,
                storage_used_size: 2 * entries.len() as u64,
                segment_needs_resizing: 0,
            }),
            first_run_value: FIRST_ID,
            stats: SegmentStats {
                distinct_states: u64::from(distinct),
                min_data_id: FIRST_ID,
                max_data_id: FIRST_ID + distinct - 1,
                original_min_segment_data_id: 2,
                rle_sort_order: -1,
                row_count: seg_rows as u64,
                has_nulls: 0,
                rle_runs: 0,
                others_rle_runs: 0,
            },
            sub_segment: Some(SubSegmentMeta {
                records: packed.len() as u64,
                base_id: 0,
            }),
        });
        objects.push(SegmentDataObject {
            primary: DataObject {
                offset: primary_at,
                words: alloc as u64,
            },
            sub: Some(DataObject {
                offset: sub_at,
                words: (words.len() / 8) as u64,
            }),
        });
    }
    let meta = ColumnMeta {
        segments: metas,
        data_objects: Some(objects),
    };
    Ok((idf, meta.to_bytes(), all))
}

/// A hash dictionary of `n` 64-bit integers (`value(i) = i * 37 - 1000`), for data ids
/// `FIRST_ID..FIRST_ID + n`.
pub fn long_dictionary(n: u32) -> Vec<u8> {
    let mut d = 0i32.to_le_bytes().to_vec(); // type: long
    // hash header: algorithm, entry size, bucket bits, locals, mask
    d.extend_from_slice(&(-1i32).to_le_bytes());
    d.extend_from_slice(&8u32.to_le_bytes());
    d.extend_from_slice(&64u32.to_le_bytes());
    d.extend_from_slice(&6u32.to_le_bytes());
    d.extend_from_slice(&u64::MAX.to_le_bytes());
    d.extend_from_slice(&u64::from(n).to_le_bytes());
    d.extend_from_slice(&8u32.to_le_bytes());
    for i in 0..i64::from(n) {
        d.extend_from_slice(&(i * 37 - 1000).to_le_bytes());
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use bicraft_vertipaq::{
        CellValue, ColumnEncoding, Limits, StorageType, decode_column, decode_data_ids, parse_idfmeta,
    };

    #[test]
    fn hybrid_column_decodes_to_its_ids() {
        let rows = SEGMENT_ROWS + 5000; // two segments
        let (idf, meta, ids) = hybrid_column(rows, 1000, 9).unwrap();
        let m = parse_idfmeta(&meta).unwrap();
        assert_eq!(m.segments.len(), 2);
        assert_eq!(decode_data_ids(&idf, &m, &Limits::default()).unwrap(), ids);
        let cells = decode_column(
            &idf,
            &meta,
            None,
            ColumnEncoding::value(-3, 1.0, StorageType::Long),
        )
        .unwrap();
        assert_eq!(cells.len(), rows);
        assert_eq!(cells[0], CellValue::Int(0));
    }

    #[test]
    fn hash_column_decodes_through_the_dictionary() {
        let (idf, meta, ids) = hybrid_column(10_000, 50, 1).unwrap();
        let dict = long_dictionary(50);
        let cells = decode_column(&idf, &meta, Some(&dict), ColumnEncoding::hash(StorageType::Long)).unwrap();
        let expect: Vec<CellValue> = ids
            .iter()
            .map(|&id| CellValue::Int(i64::from(id - FIRST_ID) * 37 - 1000))
            .collect();
        assert_eq!(cells, expect);
    }
}
