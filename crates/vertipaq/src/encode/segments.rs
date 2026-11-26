//! Data ids → `.idf` blocks and the `.idfmeta` that describes them.

use super::{EncodeOptions, SegmentStatsOut};
use crate::bitpack;
use crate::compression::{Compression, InnerCompression};
use crate::idf::{FIRST_DATA_ID, NULL_DATA_ID};
use crate::idfmeta::{
    ColumnMeta, DataObject, RleFacet, SegmentDataObject, SegmentMeta, SegmentStats, SubSegmentMeta,
};
use crate::{Error, bitpack::width_for};

/// RLE entries allocated in a primary block at least.
const MIN_RLE_ENTRIES: usize = 16;
/// `bookmark_bits` of XM123 segments (observed constant).
const XM123_BOOKMARK_BITS: u64 = 24;

/// Output of one column: `.idf`, `.idfmeta`, per-segment facts.
type Encoded = (Vec<u8>, Vec<u8>, Vec<SegmentStatsOut>);

/// One RLE entry: `(value, count)`.
type Entry = (u32, u32);

/// `original_min_segment_data_id` of a hash-encoded segment holding only blanks (observed).
pub(crate) const ONLY_BLANKS_ORIGINAL_MIN: u32 = 0x8000_0000;

/// Encodes data ids as hybrid RLE segments.
pub(super) fn encode_hybrid(
    ids: &[u32],
    value_encoded: bool,
    options: &EncodeOptions,
) -> Result<Encoded, Error> {
    let seg_rows = usize::try_from(options.segment_rows).unwrap_or(usize::MAX);
    let mut w = Writer::default();
    if ids.is_empty() {
        w.hybrid(&[], value_encoded, options)?;
    }
    for chunk in ids.chunks(seg_rows.max(1)) {
        w.hybrid(chunk, value_encoded, options)?;
    }
    Ok(w.finish())
}

/// Encodes a RowNumber column partition: XM123 segments counting up from id
/// `3 + first_row` (row numbers continue across a table's partitions).
pub(super) fn encode_row_number(
    first_row: u64,
    rows: u64,
    options: &EncodeOptions,
) -> Result<Encoded, Error> {
    let mut w = Writer::default();
    let mut done = 0u64;
    loop {
        let n = (rows - done).min(options.segment_rows);
        let first = u64::from(FIRST_DATA_ID)
            .checked_add(first_row)
            .and_then(|f| f.checked_add(done))
            .and_then(|f| u32::try_from(f).ok())
            .ok_or_else(|| Error::LimitExceeded(format!("{rows} rows from row {first_row}")))?;
        w.row_number(n, first)?;
        done += n;
        if done >= rows {
            break;
        }
    }
    Ok(w.finish())
}

#[derive(Default)]
struct Writer {
    idf: Vec<u8>,
    segments: Vec<SegmentMeta>,
    objects: Vec<SegmentDataObject>,
    stats: Vec<SegmentStatsOut>,
}

impl Writer {
    fn finish(self) -> Encoded {
        let meta = ColumnMeta {
            segments: self.segments,
            data_objects: Some(self.objects),
        };
        (self.idf, meta.to_bytes(), self.stats)
    }

    /// Appends a block `{u64 words, words × 8 bytes}` and returns where it sits.
    fn block(&mut self, payload: &[u8], words: usize) -> DataObject {
        let offset = self.idf.len() as u64;
        self.idf.extend_from_slice(&(words as u64).to_le_bytes());
        self.idf.extend_from_slice(payload);
        let pad = (words * 8).saturating_sub(payload.len());
        self.idf.resize(self.idf.len() + pad, 0);
        DataObject {
            offset,
            words: words as u64,
        }
    }

    fn primary(&mut self, entries: &[Entry]) -> (DataObject, usize) {
        let allocated = allocated_entries(entries.len());
        let mut payload = Vec::with_capacity(entries.len() * 8);
        for &(value, count) in entries {
            payload.extend_from_slice(&value.to_le_bytes());
            payload.extend_from_slice(&count.to_le_bytes());
        }
        (self.block(&payload, allocated), allocated)
    }

    fn hybrid(&mut self, ids: &[u32], value_encoded: bool, options: &EncodeOptions) -> Result<(), Error> {
        let records = ids.len() as u64;
        if u32::try_from(ids.len()).is_err() {
            return Err(Error::LimitExceeded(format!("{records} rows in a segment")));
        }
        let (mut min, mut max, mut has_nulls) = (u32::MAX, 0u32, false);
        for &id in ids {
            if id == NULL_DATA_ID {
                has_nulls = true;
            } else {
                min = min.min(id);
                max = max.max(id);
            }
        }
        let only_blanks = has_nulls && min > max;
        if min > max {
            (min, max) = (NULL_DATA_ID, NULL_DATA_ID);
        }
        // Observed on engine-written hash-encoded columns holding only blanks: the original
        // minimum id is 0x8000_0000 (`Statistics_OriginalMinSegmentDataID` −2^31).
        // An empty segment: 2, whatever the encoding (observed).
        let original_min = match (value_encoded, only_blanks) {
            _ if records == 0 => 2,
            (true, _) => 3,
            (false, true) => ONLY_BLANKS_ORIGINAL_MIN,
            (false, false) => 2,
        };
        // Packed values are relative to the null id when the segment holds blanks.
        let base = if has_nulls { NULL_DATA_ID } else { min };
        let width = width_for((max - base).max(1));
        let (entries, packed, rle_runs) = rle_split(ids, options.rle_min_run);
        let others = entries.len() as u64 - rle_runs;
        let relative: Vec<u32> = packed.iter().map(|&id| id - base).collect();
        let packed_bytes = bitpack::pack(&relative, width)
            .ok_or_else(|| Error::LimitExceeded("bit packing failed".into()))?;
        // Room for one extra value, as Desktop allocates.
        let sub_words = bitpack::words_needed(packed.len() + 1, width).unwrap_or(1);
        let (primary, allocated) = self.primary(&entries);
        let sub = self.block(&packed_bytes, sub_words);
        let used = entries.len() as u64;
        // An empty segment (engine-written: the DirectQuery partition of a hybrid table) has
        // base id 0 and bookmark bits 1.
        let (base_id, bookmark) = if records == 0 {
            (0, 1)
        } else {
            (options.segment_base_id, bookmark_bits(records, used))
        };
        self.segments.push(SegmentMeta {
            records,
            base_id,
            compression_class: Compression::HybridRle(InnerCompression::NoSplit(width)).class_id(),
            sub_compression_class: Compression::HybridRle(InnerCompression::NoSplit(width)).sub_class_id(),
            rle: Some(RleFacet {
                bookmark_bits: bookmark,
                storage_alloc_size: 2 * allocated as u64,
                storage_used_size: 2 * (used + 1),
                segment_needs_resizing: 0,
            }),
            // the packing base: the null id when the segment holds blanks
            first_run_value: base,
            stats: SegmentStats {
                distinct_states: 0,
                min_data_id: min,
                max_data_id: max,
                original_min_segment_data_id: original_min,
                rle_sort_order: -1,
                row_count: records,
                has_nulls: u8::from(has_nulls),
                rle_runs,
                others_rle_runs: others,
            },
            sub_segment: Some(SubSegmentMeta {
                records: packed.len() as u64,
                base_id: 0,
            }),
        });
        self.objects.push(SegmentDataObject {
            primary,
            sub: Some(sub),
        });
        self.stats.push(SegmentStatsOut {
            records,
            min_data_id: min,
            max_data_id: max,
            original_min_segment_data_id: original_min,
            has_nulls,
            rle_runs,
            others_rle_runs: others,
            bit_packed_rows: packed.len() as u64,
            bit_width: width,
            rle_entries_allocated: allocated as u64,
        });
        Ok(())
    }

    fn row_number(&mut self, records: u64, first: u32) -> Result<(), Error> {
        let count = u32::try_from(records).map_err(|_| Error::LimitExceeded(format!("{records} rows")))?;
        // An empty segment's statistics hold the blank id (observed), its first run value the
        // next row number.
        let (min, max) = if records == 0 {
            (NULL_DATA_ID, NULL_DATA_ID)
        } else {
            let max = first
                .checked_add(count - 1)
                .ok_or_else(|| Error::LimitExceeded(format!("{records} rows")))?;
            (first, max)
        };
        // An empty partition (engine-written: the DirectQuery partition of a hybrid table) keeps
        // its one entry, with a count of 0.
        let entries: Vec<Entry> = vec![(u32::MAX, count)];
        let (primary, allocated) = self.primary(&entries);
        let sub = self.block(&[], 0);
        let used = entries.len() as u64;
        self.segments.push(SegmentMeta {
            records,
            base_id: 0,
            compression_class: Compression::HybridRle(InnerCompression::Xm123).class_id(),
            sub_compression_class: Compression::HybridRle(InnerCompression::Xm123).sub_class_id(),
            rle: Some(RleFacet {
                bookmark_bits: XM123_BOOKMARK_BITS,
                storage_alloc_size: 2 * allocated as u64,
                storage_used_size: 2 * used,
                segment_needs_resizing: 0,
            }),
            first_run_value: first,
            stats: SegmentStats {
                distinct_states: 0,
                min_data_id: min,
                max_data_id: max,
                original_min_segment_data_id: 2,
                rle_sort_order: -1,
                row_count: records,
                has_nulls: 0,
                rle_runs: 0,
                others_rle_runs: 0,
            },
            sub_segment: Some(SubSegmentMeta { records, base_id: 0 }),
        });
        self.objects.push(SegmentDataObject {
            primary,
            sub: Some(sub),
        });
        self.stats.push(SegmentStatsOut {
            records,
            min_data_id: min,
            max_data_id: max,
            original_min_segment_data_id: 2,
            has_nulls: false,
            rle_runs: 0,
            others_rle_runs: 0,
            bit_packed_rows: records,
            bit_width: 0,
            rle_entries_allocated: allocated as u64,
        });
        Ok(())
    }
}

/// Splits a segment into RLE entries (runs of at least `min_run` equal ids) and bit-packed
/// entries (everything between them). Returns the entries, the bit-packed ids and the
/// number of RLE entries. The segment must have fewer than 2^32 rows.
///
/// A run whose id equals the marker the next bit-packed entry would carry
/// (`0xFFFFFFFF − packed so far`) would be misread, so it is bit-packed instead; data ids
/// never get that large in practice.
fn rle_split(ids: &[u32], min_run: u32) -> (Vec<Entry>, Vec<u32>, u64) {
    let mut entries: Vec<Entry> = Vec::new();
    let mut runs = 0u64;
    let mut packed: Vec<u32> = Vec::new();
    let mut pending_from: Option<usize> = None;
    let min_run = usize::try_from(min_run).unwrap_or(usize::MAX);
    let flush = |entries: &mut Vec<Entry>, packed: &mut Vec<u32>, from: usize, to: usize| {
        let marker = u32::MAX - packed.len() as u32;
        entries.push((marker, (to - from) as u32));
        packed.extend_from_slice(ids.get(from..to).unwrap_or(&[]));
    };
    let mut i = 0;
    while i < ids.len() {
        let id = ids.get(i).copied().unwrap_or(0);
        let mut j = i + 1;
        while ids.get(j) == Some(&id) {
            j += 1;
        }
        let marker = u32::MAX - (packed.len() + pending_from.map_or(0, |p| i - p)) as u32;
        if j - i >= min_run && id != marker {
            if let Some(from) = pending_from.take() {
                flush(&mut entries, &mut packed, from, i);
            }
            entries.push((id, (j - i) as u32));
            runs += 1;
        } else if pending_from.is_none() {
            pending_from = Some(i);
        }
        i = j;
    }
    if let Some(from) = pending_from {
        flush(&mut entries, &mut packed, from, ids.len());
    }
    (entries, packed, runs)
}

/// RLE entries allocated for `used` entries: the next power of two above `used`, at least 16.
fn allocated_entries(used: usize) -> usize {
    used.saturating_add(1)
        .checked_next_power_of_two()
        .unwrap_or(usize::MAX)
        .max(MIN_RLE_ENTRIES)
}

/// `bookmark_bits` of a hybrid segment: the smallest `b` with
/// `2^b × (entries + 1) ≥ 10 × (records + 1)`, i.e. `ceil(log2(10 (records + 1) / (entries + 1)))`.
/// Fitted on 401 distinct (records, entries) pairs of engine-written segments (1 to 1 048 576
/// rows, 1 to 616 entries), all reproduced.
fn bookmark_bits(records: u64, entries: u64) -> u64 {
    let target = u128::from(records).saturating_add(1).saturating_mul(10);
    let per = u128::from(entries).saturating_add(1);
    let mut bits = 0u64;
    while bits < 64 && (1u128 << bits).saturating_mul(per) < target {
        bits += 1;
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmark_bits_match_observations() {
        for (records, entries, bits) in [
            (1, 1, 4),
            (4, 1, 5),
            (6, 1, 6),
            (24, 1, 7),
            (28, 1, 8),
            (731, 1, 12),
            (9503, 1, 16),
            (731, 2, 12),
            (731, 8, 10),
            (9503, 6, 14),
            (9503, 23, 12),
            (9503, 43, 12),
            // large engine-written segments
            (1125, 10, 10),
            (1134, 10, 11),
            (2240, 10, 11),
            (2256, 10, 12),
            (20_000, 211, 10),
            (951_424, 2, 22),
            (1_000_000, 14, 20),
            (1_048_576, 4, 22),
            (1_048_576, 616, 15),
        ] {
            assert_eq!(
                bookmark_bits(records, entries),
                bits,
                "{records} rows, {entries} entries"
            );
        }
    }

    #[test]
    fn allocation() {
        assert_eq!(allocated_entries(1), 16);
        assert_eq!(allocated_entries(15), 16);
        assert_eq!(allocated_entries(16), 32);
        assert_eq!(allocated_entries(23), 32);
        assert_eq!(allocated_entries(43), 64);
    }

    #[test]
    fn split_runs() {
        let mut ids = vec![3u32; 70];
        ids.extend([4, 5, 4]);
        ids.extend(vec![6u32; 64]);
        ids.extend([7]);
        let (entries, packed, runs) = rle_split(&ids, 64);
        assert_eq!(entries, vec![(3, 70), (u32::MAX, 3), (6, 64), (u32::MAX - 3, 1)]);
        assert_eq!(packed, vec![4, 5, 4, 7]);
        assert_eq!(runs, 2);
        let (entries, packed, _) = rle_split(&[5; 10], 64);
        assert_eq!(entries, vec![(u32::MAX, 10)]);
        assert_eq!(packed.len(), 10);
    }
}
