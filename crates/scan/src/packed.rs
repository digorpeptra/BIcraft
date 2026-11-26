//! Columns kept compressed: the VertiPaq segments of a column (bit-packed data ids, hybrid RLE
//! runs over a bit-packed sub-segment, or consecutive ids) and the table mapping each data id to
//! the column's code. Scans unpack a block of rows at a time ([`Col::block`]); nothing holds one
//! code per row.

use std::sync::Arc;

use crate::lazy::Lazy;

/// Rows of one zone of a segment's zone map ([`Packed::zones`]): one morsel.
pub const ZONE_ROWS: usize = super::MORSEL;

/// Data ids of a run of values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ids {
    /// `id = base + field`: `width`-bit fields, `64 / width` per word, low bits first, never
    /// straddling two words.
    Bits { width: u8, base: u32, words: Vec<u64> },
    /// `id = first + i`.
    Seq { first: u32 },
}

impl Ids {
    fn per(width: u8) -> usize {
        if width == 0 || width > 32 {
            0
        } else {
            64 / usize::from(width)
        }
    }

    /// The `i`-th id.
    pub fn get(&self, i: usize) -> u32 {
        match self {
            Ids::Seq { first } => first.wrapping_add(u32::try_from(i).unwrap_or(u32::MAX)),
            Ids::Bits { width, base, words } => {
                let per = Self::per(*width);
                if per == 0 {
                    return 0;
                }
                let word = words.get(i / per).copied().unwrap_or(0);
                let shift = (i % per) * usize::from(*width);
                let mask = if *width >= 32 {
                    u64::from(u32::MAX)
                } else {
                    (1u64 << width) - 1
                };
                base.wrapping_add(((word >> shift) & mask) as u32)
            }
        }
    }

    /// Ids `from..from + out.len()` into `out`.
    pub fn fill(&self, from: usize, out: &mut [u32]) {
        match self {
            Ids::Seq { first } => {
                let mut id = first.wrapping_add(u32::try_from(from).unwrap_or(u32::MAX));
                for o in out.iter_mut() {
                    *o = id;
                    id = id.wrapping_add(1);
                }
            }
            Ids::Bits { width, base, words } => {
                let (base, words) = (*base, words.as_slice());
                match width {
                    1 => unpack::<1>(words, base, from, out),
                    2 => unpack::<2>(words, base, from, out),
                    3 => unpack::<3>(words, base, from, out),
                    4 => unpack::<4>(words, base, from, out),
                    5 => unpack::<5>(words, base, from, out),
                    6 => unpack::<6>(words, base, from, out),
                    7 => unpack::<7>(words, base, from, out),
                    8 => unpack::<8>(words, base, from, out),
                    9 => unpack::<9>(words, base, from, out),
                    10 => unpack::<10>(words, base, from, out),
                    12 => unpack::<12>(words, base, from, out),
                    16 => unpack::<16>(words, base, from, out),
                    21 => unpack::<21>(words, base, from, out),
                    32 => unpack::<32>(words, base, from, out),
                    _ => Self::fill_any(*width, base, words, from, out),
                }
            }
        }
    }

    /// [`Ids::fill`] of bit-packed ids at any width.
    fn fill_any(width: u8, base: u32, words: &[u64], from: usize, out: &mut [u32]) {
        {
            {
                let per = Self::per(width);
                if per == 0 {
                    out.fill(0);
                    return;
                }
                let w = usize::from(width);
                let mask = if width >= 32 {
                    u64::from(u32::MAX)
                } else {
                    (1u64 << width) - 1
                };
                let mut wi = from / per;
                let mut k = from % per;
                let mut word = words.get(wi).copied().unwrap_or(0) >> (k * w);
                for o in out.iter_mut() {
                    if k == per {
                        wi += 1;
                        k = 0;
                        word = words.get(wi).copied().unwrap_or(0);
                    }
                    *o = base.wrapping_add((word & mask) as u32);
                    word >>= w;
                    k += 1;
                }
            }
        }
    }
}

/// `W`-bit ids `from..from + out.len()` (`64 / W` per word, low bits first): whole words at a
/// time, the field loop unrolled for the width.
#[inline]
fn unpack<const W: usize>(words: &[u64], base: u32, from: usize, out: &mut [u32]) {
    let per = 64 / W;
    let mask: u64 = if W >= 32 {
        u64::from(u32::MAX)
    } else {
        (1u64 << W) - 1
    };
    let mut done = 0usize;
    let mut pos = from;
    // up to the next word boundary
    while done < out.len() && !pos.is_multiple_of(per) {
        let w = words.get(pos / per).copied().unwrap_or(0);
        if let Some(o) = out.get_mut(done) {
            *o = base.wrapping_add(((w >> ((pos % per) * W)) & mask) as u32);
        }
        done += 1;
        pos += 1;
    }
    // whole words
    let first = pos / per;
    let rest = out.get_mut(done..).unwrap_or(&mut []);
    let whole = rest.len() / per;
    let mut chunks = rest.chunks_exact_mut(per);
    for (chunk, w) in chunks.by_ref().zip(words.get(first..).unwrap_or(&[]).iter()) {
        let mut w = *w;
        for o in chunk.iter_mut() {
            *o = base.wrapping_add((w & mask) as u32);
            w >>= W % 64;
        }
    }
    // the tail (and words missing in a malformed segment)
    let tail_from = first + whole;
    for (i, o) in chunks.into_remainder().iter_mut().enumerate() {
        let w = words.get(tail_from).copied().unwrap_or(0);
        *o = base.wrapping_add(((w >> (i * W)) & mask) as u32);
    }
    let have = words.len().saturating_sub(first);
    if whole > have {
        let bad = rest.get_mut(have * per..whole * per).unwrap_or(&mut []);
        bad.fill(base);
    }
}

/// One run of a hybrid RLE segment: `len` rows from `start`, all with data id `id`, or (when
/// `packed`) the sub-segment's ids from position `id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    pub start: u32,
    pub len: u32,
    pub id: u32,
    pub packed: bool,
}

/// How a segment stores its data ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegData {
    Plain(Ids),
    /// Runs covering the segment's rows in order, over a bit-packed sub-segment.
    Rle {
        runs: Vec<Run>,
        sub: Ids,
    },
}

/// One segment: `rows` rows, its data ids between `min_id` and `max_id` (statistics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub rows: usize,
    pub data: SegData,
    pub min_id: u32,
    pub max_id: u32,
}

impl SegData {
    fn run_at(runs: &[Run], at: usize) -> usize {
        let at = u32::try_from(at).unwrap_or(u32::MAX);
        runs.partition_point(|r| r.start.saturating_add(r.len) <= at)
    }

    fn get(&self, at: usize) -> u32 {
        match self {
            SegData::Plain(ids) => ids.get(at),
            SegData::Rle { runs, sub } => {
                let Some(r) = runs.get(Self::run_at(runs, at)) else {
                    return 0;
                };
                if r.packed {
                    sub.get((r.id as usize).saturating_add(at.saturating_sub(r.start as usize)))
                } else {
                    r.id
                }
            }
        }
    }

    fn fill(&self, from: usize, out: &mut [u32]) {
        match self {
            SegData::Plain(ids) => ids.fill(from, out),
            SegData::Rle { runs, sub } => {
                let mut ri = Self::run_at(runs, from);
                let mut at = from;
                let mut done = 0usize;
                while done < out.len() {
                    let Some(r) = runs.get(ri) else {
                        if let Some(rest) = out.get_mut(done..) {
                            rest.fill(0);
                        }
                        return;
                    };
                    let end = (r.start as usize).saturating_add(r.len as usize);
                    let n = end.saturating_sub(at).min(out.len() - done).max(1);
                    let Some(slot) = out.get_mut(done..done.saturating_add(n).min(out.len())) else {
                        return;
                    };
                    if r.packed {
                        sub.fill(
                            (r.id as usize).saturating_add(at.saturating_sub(r.start as usize)),
                            slot,
                        );
                    } else {
                        slot.fill(r.id);
                    }
                    done = done.saturating_add(slot.len());
                    at = at.saturating_add(slot.len());
                    ri += 1;
                }
            }
        }
    }
}

/// A segment's statistics: the sum of its data ids, its rows of ids 0, 1 and 2, its largest id
/// ([`Packed::segment_stats`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegStats {
    sum: u64,
    low: [u64; 3],
    max: u32,
}

/// A column kept as its compressed segments, with the code of each data id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packed {
    rows: usize,
    segs: Vec<Segment>,
    /// First row of each segment.
    starts: Vec<usize>,
    /// Data id → code (0: blank, or an id the column does not use).
    lut: Vec<u32>,
    /// Per segment, statistics read with the column (small materialized aggregates, Moerkotte,
    /// VLDB 1998), exact since computed from the data: the sum of its data ids, its rows of ids
    /// 0, 1 and 2 (the blank ids), and its largest id.
    id_sums: Vec<SegStats>,
    /// Per segment, the lowest and highest data id of each zone of [`ZONE_ROWS`] rows from the
    /// segment's start (a zone map, computed in the same pass as the statistics; empty for
    /// segments read on demand).
    zones: Vec<Vec<(u32, u32)>>,
    /// Segments read on demand (`docs/beyond-ram.md`, lot 3): `segs` then hold only each
    /// segment's rows and id range, the data comes from here.
    lazy: Option<Arc<Lazy>>,
}

impl PartialEq for Lazy {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl Eq for Lazy {}

/// A segment's data: in the column, or read on demand.
enum SegRef<'a> {
    Here(&'a SegData),
    Read(Arc<SegData>),
}

impl std::ops::Deref for SegRef<'_> {
    type Target = SegData;
    fn deref(&self) -> &SegData {
        match self {
            SegRef::Here(d) => d,
            SegRef::Read(d) => d,
        }
    }
}

impl Packed {
    /// A column of `segs` in order; `lut` maps data ids to codes. Reads each segment once for
    /// its statistics (several segments at a time).
    pub fn new(segs: Vec<Segment>, lut: Vec<u32>) -> Packed {
        let mut starts = Vec::with_capacity(segs.len());
        let mut rows = 0usize;
        for s in &segs {
            starts.push(rows);
            rows = rows.saturating_add(s.rows);
        }
        let threads = super::configured_threads()
            .min(segs.len().max(1))
            .min(rows / (1 << 18) + 1);
        let serial = || segs.iter().map(Self::segment_scan).collect::<Vec<_>>();
        let scanned: Vec<(SegStats, Vec<(u32, u32)>)> = if threads <= 1 {
            serial()
        } else {
            let per = segs.len().div_ceil(threads).max(1);
            let chunks: Vec<&[Segment]> = segs.chunks(per).collect();
            // a failed worker: every segment again on this thread (never a partial answer)
            super::pool::map(chunks.len(), |i| {
                chunks
                    .get(i)
                    .map_or_else(Vec::new, |c| c.iter().map(Self::segment_scan).collect())
            })
            .map_or_else(serial, |parts| parts.concat())
        };
        let (id_sums, zones) = scanned.into_iter().unzip();
        Packed {
            rows,
            segs,
            starts,
            lut,
            id_sums,
            zones,
            lazy: None,
        }
    }

    /// A column whose segments are read on demand from `lazy` (`docs/beyond-ram.md`, lot 3):
    /// `segs` give each segment's rows and id range (their data is not read: an empty
    /// placeholder), `stats` each segment's [`Packed::segment_stats`], computed when the column
    /// was read. `None` when the lengths disagree.
    pub fn new_lazy(
        segs: Vec<Segment>,
        lut: Vec<u32>,
        stats: Vec<SegStats>,
        lazy: Arc<Lazy>,
    ) -> Option<Packed> {
        if stats.len() != segs.len() {
            return None;
        }
        let mut starts = Vec::with_capacity(segs.len());
        let mut rows = 0usize;
        for s in &segs {
            starts.push(rows);
            rows = rows.checked_add(s.rows)?;
        }
        Some(Packed {
            rows,
            segs,
            starts,
            lut,
            id_sums: stats,
            zones: Vec::new(),
            lazy: Some(lazy),
        })
    }

    /// Whether the segments are read on demand.
    pub fn is_lazy(&self) -> bool {
        self.lazy.is_some()
    }

    /// Tests: where the segments are read from.
    #[cfg(test)]
    pub(crate) fn lazy_source(&self) -> Option<&Arc<Lazy>> {
        self.lazy.as_ref()
    }

    /// The data of segment `i`.
    fn data(&self, i: usize) -> Option<SegRef<'_>> {
        let s = self.segs.get(i)?;
        Some(match &self.lazy {
            Some(l) => SegRef::Read(l.get(i)),
            None => SegRef::Here(&s.data),
        })
    }

    /// A segment's statistics, as [`Packed::new`] computes them (one step per RLE run).
    pub fn segment_stats(s: &Segment) -> SegStats {
        Self::segment_scan(s).0
    }

    /// A segment's statistics and its zone map (one step per RLE run): the lowest and highest
    /// id of each [`ZONE_ROWS`] rows from its start.
    fn segment_scan(s: &Segment) -> (SegStats, Vec<(u32, u32)>) {
        /// The running statistics and zones of one segment.
        struct Acc {
            sum: u64,
            low: [u64; 3],
            max: u32,
            zones: Vec<(u32, u32)>,
        }
        impl Acc {
            fn add(&mut self, id: u32, n: u64) {
                if n > 0 {
                    self.max = self.max.max(id);
                }
                self.sum = self.sum.wrapping_add(u64::from(id).wrapping_mul(n));
                if let Some(l) = self.low.get_mut(id as usize) {
                    *l += n;
                }
            }
            /// Ids `lo..=hi` met on rows `first..=last` of the segment.
            fn zone(&mut self, lo: u32, hi: u32, first: usize, last: usize) {
                let end = (last / ZONE_ROWS).min(self.zones.len().saturating_sub(1));
                for z in self.zones.get_mut(first / ZONE_ROWS..=end).unwrap_or(&mut []) {
                    z.0 = z.0.min(lo);
                    z.1 = z.1.max(hi);
                }
            }
        }
        let mut a = Acc {
            sum: 0,
            low: [0; 3],
            max: 0,
            zones: vec![(u32::MAX, 0); s.rows.div_ceil(ZONE_ROWS)],
        };
        let mut buf = vec![0u32; super::BLOCK];
        // `len` ids of `ids` from position `from`, on the segment's rows from `row`
        let mut ids = |a: &mut Acc, ids: &Ids, from: usize, row: usize, len: usize| {
            let mut done = 0usize;
            while done < len {
                let n = (len - done).min(buf.len());
                if let Some(b) = buf.get_mut(..n) {
                    ids.fill(from.saturating_add(done), b);
                    let at = row.saturating_add(done);
                    // by zone: a block of rows spans at most two
                    let split = (ZONE_ROWS - at % ZONE_ROWS).min(n);
                    let parts = [
                        (b.get(..split).unwrap_or(&[]), at),
                        (b.get(split..).unwrap_or(&[]), at.saturating_add(split)),
                    ];
                    for (part, start) in parts {
                        let (mut lo, mut hi) = (u32::MAX, 0u32);
                        for id in part {
                            a.add(*id, 1);
                            lo = lo.min(*id);
                            hi = hi.max(*id);
                        }
                        if !part.is_empty() {
                            a.zone(lo, hi, start, start.saturating_add(part.len() - 1));
                        }
                    }
                }
                done += n;
            }
        };
        match &s.data {
            SegData::Plain(i) => ids(&mut a, i, 0, 0, s.rows),
            SegData::Rle { runs, sub } => {
                for r in runs {
                    let (start, len) = (r.start as usize, r.len as usize);
                    if r.packed {
                        ids(&mut a, sub, r.id as usize, start, len);
                    } else if len > 0 {
                        a.add(r.id, u64::from(r.len));
                        a.zone(r.id, r.id, start, start.saturating_add(len - 1));
                    }
                }
            }
        }
        let Acc { sum, low, max, zones } = a;
        (SegStats { sum, low, max }, zones)
    }

    /// The zone map of segment `si`: the lowest and highest data id of each [`ZONE_ROWS`] rows
    /// from its start (a zone without rows: `(u32::MAX, 0)`); empty for segments read on demand.
    pub fn zones(&self, si: usize) -> &[(u32, u32)] {
        self.zones.get(si).map_or(&[], Vec::as_slice)
    }

    /// The sum of the data ids from `first` on (`first` at most 3) over rows `start..end`, and
    /// how many, from the segments' statistics when the rows cover whole segments; `None` when
    /// they do not (or `first` is above 3).
    pub fn id_sum(&self, start: usize, end: usize, first: u32) -> Option<(u64, u64)> {
        if first > 3 {
            return None;
        }
        let si = self.seg_of(start);
        if self.starts.get(si).copied() != Some(start) {
            return None;
        }
        let (mut sum, mut n) = (0u64, 0u64);
        let mut at = start;
        let mut k = si;
        while at < end {
            let (seg, st) = (self.segs.get(k)?, self.id_sums.get(k)?);
            let (s, low) = (&st.sum, &st.low);
            let seg_end = at.saturating_add(seg.rows);
            if seg_end > end {
                return None;
            }
            let mut sk = *s;
            let mut nk = seg.rows as u64;
            for (id, c) in low.iter().enumerate().take(first as usize) {
                sk = sk.wrapping_sub((id as u64).wrapping_mul(*c));
                nk = nk.saturating_sub(*c);
            }
            sum = sum.wrapping_add(sk);
            n = n.saturating_add(nk);
            at = seg_end;
            k += 1;
        }
        Some((sum, n))
    }

    /// Whether every row of `start..end` holds a data id from `first` on (`first` at most 3) and
    /// below `limit`, from the segments' statistics (exact: computed from the data). `false` when
    /// the rows reach past the column.
    pub fn ids_within(&self, start: usize, end: usize, first: u32, limit: usize) -> bool {
        if first > 3 {
            return false;
        }
        let mut k = self.seg_of(start);
        let mut at = start;
        while at < end {
            let (Some(st), Some(seg), Some(seg_start)) =
                (self.id_sums.get(k), self.segs.get(k), self.starts.get(k))
            else {
                return false;
            };
            if st.low.iter().take(first as usize).any(|c| *c > 0) || st.max as usize >= limit {
                return false;
            }
            at = at.max(seg_start.saturating_add(seg.rows));
            k += 1;
        }
        true
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segs
    }

    /// Rows held in runs of one id (RLE segments' unpacked runs).
    pub fn run_rows(&self) -> usize {
        (0..self.segs.len())
            .filter_map(|i| self.data(i))
            .map(|d| match &*d {
                SegData::Rle { runs, .. } => runs.iter().filter(|r| !r.packed).map(|r| r.len as usize).sum(),
                SegData::Plain(_) => 0,
            })
            .sum()
    }

    /// The code of each data id.
    pub fn lut(&self) -> &[u32] {
        &self.lut
    }

    /// Bytes held (segments and id table; for segments read on demand, their statistics only:
    /// the data read is held by the session's [`crate::lazy::SegCache`]).
    pub fn bytes(&self) -> usize {
        if self.lazy.is_some() {
            return self
                .segs
                .len()
                .saturating_mul(std::mem::size_of::<Segment>() + std::mem::size_of::<SegStats>() + 8)
                .saturating_add(self.lut.len().saturating_mul(4));
        }
        let segs: usize = self
            .segs
            .iter()
            .map(|s| {
                let ids = |i: &Ids| match i {
                    Ids::Bits { words, .. } => words.len() * 8,
                    Ids::Seq { .. } => 0,
                };
                match &s.data {
                    SegData::Plain(i) => ids(i),
                    SegData::Rle { runs, sub } => runs.len() * 13 + ids(sub),
                }
            })
            .sum();
        segs + self.lut.len() * 4
    }

    /// The segment holding `row`: (index, first row, rows).
    pub fn segment_at(&self, row: usize) -> Option<(usize, usize, usize)> {
        let si = self.seg_of(row);
        Some((si, *self.starts.get(si)?, self.segs.get(si)?.rows))
    }

    fn seg_of(&self, row: usize) -> usize {
        self.starts.partition_point(|s| *s <= row).saturating_sub(1)
    }

    /// The code of row `row` (0 past the end).
    pub fn get(&self, row: usize) -> u32 {
        if row >= self.rows {
            return 0;
        }
        let si = self.seg_of(row);
        let (Some(d), Some(start)) = (self.data(si), self.starts.get(si)) else {
            return 0;
        };
        let id = d.get(row - start);
        self.lut.get(id as usize).copied().unwrap_or(0)
    }

    /// The data id of row `row` (0 past the end).
    pub fn get_id(&self, row: usize) -> u32 {
        if row >= self.rows {
            return 0;
        }
        let si = self.seg_of(row);
        match (self.data(si), self.starts.get(si)) {
            (Some(d), Some(start)) => d.get(row - start),
            _ => 0,
        }
    }

    /// The data id of rows `start..end` when they all lie in one run of one id (an RLE run).
    pub fn run_through(&self, start: usize, end: usize) -> Option<u32> {
        let si = self.seg_of(start);
        let (d, first) = (self.data(si)?, *self.starts.get(si)?);
        let SegData::Rle { runs, .. } = &*d else {
            return None;
        };
        let at = start.checked_sub(first)?;
        let r = runs.get(SegData::run_at(runs, at))?;
        let run_end = first
            .saturating_add(r.start as usize)
            .saturating_add(r.len as usize);
        (!r.packed && (r.start as usize) <= at && run_end >= end).then_some(r.id)
    }

    /// The runs of one id covering rows `start..end`, as (offset from `start`, rows, id) in
    /// `out`; `false` (and `out` unspecified) when some of those rows are bit-packed.
    pub fn runs_in(&self, start: usize, end: usize, out: &mut Vec<(u32, u32, u32)>) -> bool {
        out.clear();
        let mut at = start;
        while at < end {
            let si = self.seg_of(at);
            let (Some(s), Some(first), Some(d)) =
                (self.segs.get(si), self.starts.get(si).copied(), self.data(si))
            else {
                return false;
            };
            let SegData::Rle { runs, .. } = &*d else {
                return false;
            };
            let seg_end = first.saturating_add(s.rows).min(end);
            let mut ri = SegData::run_at(runs, at.saturating_sub(first));
            while at < seg_end {
                let Some(r) = runs.get(ri) else {
                    return false;
                };
                if r.packed {
                    return false;
                }
                let run_end = first
                    .saturating_add(r.start as usize)
                    .saturating_add(r.len as usize)
                    .min(seg_end);
                if run_end <= at {
                    return false;
                }
                let (Ok(off), Ok(len)) = (u32::try_from(at - start), u32::try_from(run_end - at)) else {
                    return false;
                };
                out.push((off, len, r.id));
                at = run_end;
                ri += 1;
            }
        }
        true
    }

    /// The data ids of rows `from..from + out.len()` (0 past the end).
    pub fn fill_ids(&self, from: usize, out: &mut [u32]) {
        let mut si = self.seg_of(from);
        let mut at = from;
        let mut done = 0usize;
        while done < out.len() {
            let (Some(s), Some(start)) = (self.segs.get(si), self.starts.get(si)) else {
                if let Some(rest) = out.get_mut(done..) {
                    rest.fill(0);
                }
                return;
            };
            let off = at.saturating_sub(*start);
            let n = s.rows.saturating_sub(off).min(out.len() - done);
            if n == 0 {
                si += 1;
                continue;
            }
            if let (Some(slot), Some(d)) = (out.get_mut(done..done + n), self.data(si)) {
                d.fill(off, slot);
            }
            done += n;
            at += n;
            si += 1;
        }
    }

    /// The codes of rows `from..from + out.len()` (0 past the end).
    pub fn fill(&self, from: usize, out: &mut [u32]) {
        self.fill_ids(from, out);
        for o in out.iter_mut() {
            *o = self.lut.get(*o as usize).copied().unwrap_or(0);
        }
    }

    /// Rows per code over the whole column (`card` codes), from the runs of RLE segments
    /// (one step per run) and blocks of the others, segments split among `threads` workers.
    pub fn counts_parallel(&self, card: usize, threads: usize) -> Vec<u64> {
        // each worker holds a table of `card` counts
        let threads = threads
            .min(self.segs.len())
            .min((256usize << 20) / card.max(1).saturating_mul(8))
            .max(1);
        if threads <= 1 {
            return self.counts(card);
        }
        let per = self.segs.len().div_ceil(threads).max(1);
        let n = self.segs.len();
        let chunks: Vec<std::ops::Range<usize>> = (0..n).step_by(per).map(|a| a..(a + per).min(n)).collect();
        let parts = super::pool::map(chunks.len(), |i| {
            chunks
                .get(i)
                .map_or_else(Vec::new, |c| self.counts_of(c.clone(), card))
        });
        // a failed worker: the whole column again on this thread (never a partial answer)
        let Some(parts) = parts else {
            return self.counts(card);
        };
        let mut out = vec![0u64; card.max(1)];
        for p in parts {
            for (a, b) in out.iter_mut().zip(p) {
                *a = a.wrapping_add(b);
            }
        }
        out
    }

    /// Rows per code over the whole column (`card` codes), from the runs of RLE segments
    /// (one step per run) and blocks of the others.
    pub fn counts(&self, card: usize) -> Vec<u64> {
        self.counts_of(0..self.segs.len(), card)
    }

    /// [`Packed::counts`] over the segments `segs`.
    fn counts_of(&self, segs: std::ops::Range<usize>, card: usize) -> Vec<u64> {
        let lut = &self.lut;
        let mut out = vec![0u64; card.max(1)];
        let mut add = |code: u32, n: u64| {
            if let Some(c) = out.get_mut(code as usize) {
                *c = c.wrapping_add(n);
            }
        };
        let mut buf = vec![0u32; super::BLOCK];
        for i in segs {
            let (Some(s), Some(d)) = (self.segs.get(i), self.data(i)) else {
                continue;
            };
            match &*d {
                SegData::Rle { runs, sub } => {
                    for r in runs {
                        if r.packed {
                            let mut left = r.len as usize;
                            let mut pos = r.id as usize;
                            while left > 0 {
                                let n = left.min(buf.len());
                                if let Some(b) = buf.get_mut(..n) {
                                    sub.fill(pos, b);
                                    for id in b.iter() {
                                        add(lut.get(*id as usize).copied().unwrap_or(0), 1);
                                    }
                                }
                                left -= n;
                                pos += n;
                            }
                        } else {
                            add(lut.get(r.id as usize).copied().unwrap_or(0), u64::from(r.len));
                        }
                    }
                }
                SegData::Plain(ids) => {
                    let mut pos = 0usize;
                    while pos < s.rows {
                        let n = (s.rows - pos).min(buf.len());
                        if let Some(b) = buf.get_mut(..n) {
                            ids.fill(pos, b);
                            for id in b.iter() {
                                add(lut.get(*id as usize).copied().unwrap_or(0), 1);
                            }
                        }
                        pos += n;
                    }
                }
            }
        }
        out
    }
}

/// A column's codes for a scan: one per row in memory, or compressed segments.
#[derive(Debug, Clone, Copy)]
pub enum Col<'a> {
    Plain(&'a [u32]),
    Packed(&'a Packed),
}

impl<'a> Col<'a> {
    pub fn len(&self) -> usize {
        match self {
            Col::Plain(c) => c.len(),
            Col::Packed(p) => p.rows(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The code of `row` (0 past the end).
    #[inline]
    pub fn get(&self, row: usize) -> u32 {
        match self {
            Col::Plain(c) => c.get(row).copied().unwrap_or(0),
            Col::Packed(p) => p.get(row),
        }
    }

    /// The codes of rows `start..start + len`: a slice of the column, or unpacked into `buf`.
    #[inline]
    pub fn block<'b>(&'b self, start: usize, len: usize, buf: &'b mut Vec<u32>) -> &'b [u32] {
        match self {
            Col::Plain(c) => match c.get(start..start.saturating_add(len)) {
                Some(s) => s,
                None => {
                    buf.clear();
                    buf.extend((start..start.saturating_add(len)).map(|r| c.get(r).copied().unwrap_or(0)));
                    buf
                }
            },
            Col::Packed(p) => {
                buf.resize(len, 0);
                p.fill(start, buf);
                buf
            }
        }
    }
}
