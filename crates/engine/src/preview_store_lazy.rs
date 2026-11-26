//! Columns read segment by segment (`docs/beyond-ram.md`, lot 3): a column too large for the
//! memory budget keeps its id table, its dictionary and each segment's statistics; the segments'
//! data is read from the backup when a scan reaches them ([`bicraft_scan::lazy`]).

use std::ops::Range;
use std::sync::{Arc, OnceLock};

use bicraft_scan::lazy::{Lazy, SegCache, SegSource};
use bicraft_scan::{Ids, Packed, SegData, Segment};
use bicraft_vertipaq::{EncodingKind, parse_dictionary, parse_idfmeta};

use super::{
    Backing, Codes, ColData, Derived, Miss, VertipaqStore, codes_of, first_seen_ids, parse_segment,
    read_backing,
};

/// One segment's place in the backup: its `.idfmeta` entry, its primary block and its
/// sub-segment (hybrid RLE).
type SegPlace = (bicraft_vertipaq::SegmentMeta, Range<usize>, Option<Range<usize>>);

/// Where a column's segments are in the backup (the store's backing, shared: the temporary file
/// stays while a column reads from it).
#[derive(Debug)]
pub(crate) struct SegFiles {
    abf: Arc<std::sync::RwLock<Backing>>,
    segs: Vec<SegPlace>,
}

impl SegFiles {
    /// Bytes of segment `i` in the backup.
    fn size(&self, i: usize) -> usize {
        self.segs.get(i).map_or(0, |(_, p, s)| {
            s.as_ref().map_or(p.end, |s| s.end).saturating_sub(p.start)
        })
    }

    /// Segment `i` read from the backup.
    fn segment(&self, i: usize) -> Result<Segment, Miss> {
        let (meta, p, s) = self
            .segs
            .get(i)
            .ok_or_else(|| Miss::Error("segment out of range".into()))?;
        // the sub-segment follows the primary block: one read covers both
        let span = p.start..s.as_ref().map_or(p.end, |s| s.end);
        let bad = || Miss::Error("malformed column data: segment blocks".into());
        read_backing(&self.abf, &span, |b| {
            let primary = b.get(..p.len()).ok_or_else(bad)?;
            let sub = match s {
                Some(s) => Some(
                    b.get(s.start.saturating_sub(p.start)..s.end.saturating_sub(p.start))
                        .ok_or_else(bad)?,
                ),
                None => None,
            };
            parse_segment(meta, &bicraft_vertipaq::idf::IdfSegment { primary, sub })
        })
    }
}

impl SegSource for SegFiles {
    fn load(&self, seg: usize) -> Option<SegData> {
        self.segment(seg).ok().map(|s| s.data)
    }
}

/// How a column is read segment by segment.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LazyLimits {
    /// Most rows of the table.
    pub max_rows: usize,
    /// Largest segment accepted (one segment is the least a scan holds).
    pub max_segment: usize,
    /// Bytes of segments read together while the column is read (its statistics and ids).
    pub batch: usize,
}

impl VertipaqStore {
    /// One column whose segments are read on demand through `cache` (see the module's notes).
    /// Refused when a segment is over `limits.max_segment` (before any segment is read) or the
    /// table over `limits.max_rows` rows. The codes are the ones [`VertipaqStore::column`] gives.
    pub fn column_lazy(
        &self,
        table: &str,
        column: &str,
        cache: &Arc<SegCache>,
        limits: LazyLimits,
    ) -> Result<ColData, Miss> {
        let key = (table.to_lowercase(), column.to_lowercase());
        let loc = self
            .columns
            .get(&key)
            .ok_or_else(|| Miss::Unavailable(format!("no stored data for column '{table}'[{column}]")))?;
        let max_rows = u64::try_from(limits.max_rows).unwrap_or(u64::MAX);
        let mut places: Vec<SegPlace> = Vec::new();
        let mut rows_total = 0u64;
        for (idf, meta) in &loc.parts {
            let meta = self.read(meta, |b| parse_idfmeta(b).map_err(|e| Miss::Error(e.to_string())))?;
            rows_total = rows_total.saturating_add(meta.row_count().unwrap_or(u64::MAX));
            if rows_total > max_rows {
                return Err(Miss::Error(format!(
                    "table '{table}' has more than {} rows (preview limit)",
                    limits.max_rows
                )));
            }
            let base = idf.start;
            let len = u64::try_from(idf.len()).unwrap_or(u64::MAX);
            let blocks = bicraft_vertipaq::idf::locate_idf(len, &meta, &mut |at| {
                let start = base.checked_add(usize::try_from(at).ok()?)?;
                self.read(&(start..start.checked_add(8)?), |b| {
                    b.first_chunk::<8>()
                        .map(|w| u64::from_le_bytes(*w))
                        .ok_or_else(|| Miss::Error("short read".into()))
                })
                .ok()
            })
            .map_err(|e| Miss::Error(e.to_string()))?;
            let abs = |r: &Range<u64>| -> Option<Range<usize>> {
                let s = base.checked_add(usize::try_from(r.start).ok()?)?;
                let e = base.checked_add(usize::try_from(r.end).ok()?)?;
                Some(s..e)
            };
            for (seg, (p, s)) in meta.segments.into_iter().zip(blocks) {
                let bad = || Miss::Error("malformed column data: segment range".into());
                let p = abs(&p).ok_or_else(bad)?;
                let s = match s {
                    Some(s) => Some(abs(&s).ok_or_else(bad)?),
                    None => None,
                };
                places.push((seg, p, s));
            }
        }
        let files = Arc::new(SegFiles {
            abf: Arc::clone(&self.abf),
            segs: places,
        });
        let n = files.segs.len();
        if let Some(i) = (0..n).find(|i| files.size(*i) > limits.max_segment) {
            let size = super::super::memory::size_text;
            return Err(Miss::Error(format!(
                "column '{table}'[{column}]: a segment needs {}, over what the memory budget leaves for one \
                 segment ({}; BICRAFT_MEMORY_BUDGET or dax.memory)",
                size(files.size(i)),
                size(limits.max_segment)
            )));
        }
        // every segment read once, a batch at a time: statistics and first-seen ids; the first
        // segments stay in the segment cache while it has room (the scan that follows starts
        // with them)
        let lazy = Lazy::new(Arc::clone(&files) as Arc<dyn SegSource>, n, cache);
        let mut placeholders: Vec<Segment> = Vec::with_capacity(n);
        let mut stats = Vec::with_capacity(n);
        let mut distinct: Vec<u32> = Vec::new();
        let mut seen: Vec<u64> = Vec::new();
        let mut i = 0usize;
        while i < n {
            let mut j = i;
            let mut bytes = 0usize;
            while j < n && (j == i || bytes.saturating_add(files.size(j)) <= limits.batch) {
                bytes = bytes.saturating_add(files.size(j));
                j += 1;
            }
            let range: Vec<usize> = (i..j).collect();
            let read = if range.len() > 1 && bicraft_scan::configured_threads() > 1 {
                bicraft_scan::pool::map(range.len(), |k| range.get(k).map(|s| files.segment(*s)))
                    .ok_or_else(|| Miss::Error("a column reader failed".into()))?
                    .into_iter()
                    .map(|r| r.unwrap_or_else(|| Err(Miss::Error("a column reader failed".into()))))
                    .collect::<Result<Vec<_>, Miss>>()?
            } else {
                range
                    .iter()
                    .map(|s| files.segment(*s))
                    .collect::<Result<Vec<_>, Miss>>()?
            };
            let mut segs = read;
            let Some(ids) = first_seen_ids(&mut segs)? else {
                return Err(Miss::Error(format!(
                    "column '{table}'[{column}] cannot be read segment by segment (data ids outside its \
                     segments' statistics)"
                )));
            };
            for id in ids {
                let (w, bit) = (id as usize / 64, id % 64);
                if seen.len() <= w {
                    seen.resize(w.saturating_add(1), 0);
                }
                if let Some(x) = seen.get_mut(w)
                    && *x & (1u64 << bit) == 0
                {
                    *x |= 1u64 << bit;
                    distinct.push(id);
                }
            }
            for (k, s) in segs.into_iter().enumerate() {
                stats.push(Packed::segment_stats(&s));
                placeholders.push(Segment {
                    rows: s.rows,
                    data: SegData::Plain(Ids::Seq { first: 0 }),
                    min_id: s.min_id,
                    max_id: s.max_id,
                });
                lazy.offer(i.saturating_add(k), s.data);
            }
            i = j;
        }
        drop(seen);
        let dictionary = match (loc.encoding.kind, &loc.dictionary) {
            (EncodingKind::Hash, Some(r)) => {
                Some(self.read(r, |b| parse_dictionary(b).map_err(|e| Miss::Error(e.to_string())))?)
            }
            _ => None,
        };
        let (values, lut, blank, affine) = codes_of(&distinct, dictionary.as_ref(), loc.encoding)?;
        drop(dictionary);
        let packed = Packed::new_lazy(placeholders, lut, stats, lazy)
            .ok_or_else(|| Miss::Error("a column reader failed".into()))?;
        Ok(ColData {
            codes: Codes::Packed {
                packed,
                plain: OnceLock::new(),
                blank,
                affine,
            },
            values,
            derived: Derived::default(),
        })
    }

    /// Bytes of a column's dictionary file in the backup (0: none): a lower bound of what a column
    /// read segment by segment keeps.
    pub fn dictionary_file_bytes(&self, table: &str, column: &str) -> Option<usize> {
        let loc = self.columns.get(&(table.to_lowercase(), column.to_lowercase()))?;
        Some(loc.dictionary.as_ref().map_or(0, Range::len))
    }
}
