//! Table data for the viewer's data preview ([`super`]): columns decoded from a VertiPaq backup
//! (a PBIX `DataModel` or a PBIP `.pbi/cache.abf`) or loaded from partitions
//! (`bicraft_data::load_partition_opts`: M queries, DAX literal tables).
//!
//! Every column becomes a [`ColData`]: one code per row into a table of distinct values, code 0
//! being blank. Grouping then works on codes, and values are looked up once per distinct code.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use bicraft_model::{SemanticModel, Table};
use bicraft_sqlite::Value as Sql;
use bicraft_vertipaq::{
    CellValue, ColumnEncoding, EncodingKind, LogicalType, decode_data_ids, ids_to_values, parse_dictionary,
    parse_idfmeta,
};

#[path = "preview_store_lazy.rs"]
mod lazy;
pub(crate) use lazy::LazyLimits;

/// Why a column has no data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Miss {
    /// The session has no data for it (no backup, no literal partition…).
    Unavailable(String),
    /// Data exists but something failed (decoding, limits).
    Error(String),
}

/// One decoded column: `codes[row]` indexes `values`; `values[0]` is always blank.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ColData {
    pub codes: Codes,
    pub values: Vec<CellValue>,
    /// What evaluations derive from the values, computed once per decoded column and shared by
    /// every query over the same data.
    pub derived: Derived,
}

/// The code of each row of a column: in memory, or kept compressed (the VertiPaq segments of a
/// stored column, read a block at a time by the storage engine; [`Codes::plain`] unpacks them
/// once for the evaluator paths that need one code per row).
pub(crate) enum Codes {
    Plain(Vec<u32>),
    Packed {
        packed: bicraft_scan::Packed,
        plain: OnceLock<Vec<u32>>,
        /// Some row has the blank code 0 (known when the column is read).
        blank: bool,
        /// `(a, b)`: every data id `i` from `FIRST_DATA_ID` on holds the value `a + b × i` (in
        /// units: Currency 1/10 000, whole numbers 1) and the ids below are blanks (a
        /// value-encoded column).
        affine: Option<(i64, i64)>,
    },
}

impl Codes {
    /// The compressed segments, when the column is read from them (not unpacked).
    pub fn col_packed(&self) -> Option<&bicraft_scan::Packed> {
        match self {
            Codes::Packed { packed, plain, .. } if plain.get().is_none() => Some(packed),
            _ => None,
        }
    }

    /// The column's values as an affine function of its data ids, when they are
    /// ([`Codes::Packed`]), with the compressed segments.
    pub fn affine(&self) -> Option<(&bicraft_scan::Packed, i64, i64)> {
        match self {
            Codes::Packed {
                packed,
                plain,
                affine: Some((a, b)),
                ..
            } if plain.get().is_none() => Some((packed, *a, *b)),
            _ => None,
        }
    }

    /// Whether some row has the blank code 0, when known without reading the rows.
    pub fn known_blank(&self) -> Option<bool> {
        match self {
            Codes::Packed { blank, .. } => Some(*blank),
            Codes::Plain(_) => None,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Codes::Plain(v) => v.len(),
            Codes::Packed { packed, .. } => packed.rows(),
        }
    }

    /// The code of row `i`.
    pub fn get(&self, i: usize) -> Option<u32> {
        match self {
            Codes::Plain(v) => v.get(i).copied(),
            Codes::Packed { packed, plain, .. } => match plain.get() {
                Some(v) => v.get(i).copied(),
                None => (i < packed.rows()).then(|| packed.get(i)),
            },
        }
    }

    /// The codes as the storage engine's kernels read them.
    pub fn col(&self) -> bicraft_scan::Col<'_> {
        match self {
            Codes::Plain(v) => bicraft_scan::Col::Plain(v),
            Codes::Packed { packed, plain, .. } => match plain.get() {
                Some(v) => bicraft_scan::Col::Plain(v),
                None => bicraft_scan::Col::Packed(packed),
            },
        }
    }

    /// One code per row (unpacked on first use for a compressed column).
    pub fn plain(&self) -> &[u32] {
        match self {
            Codes::Plain(v) => v,
            Codes::Packed { packed, plain, .. }
                if plain.get().is_none() && !unpack_admitted(packed.rows()) =>
            {
                // refused under the memory limit: no codes, and the evaluation fails (lot 5)
                &[]
            }
            Codes::Packed { packed, plain, .. } => plain.get_or_init(|| {
                UNPACKED.fetch_add(packed.rows(), std::sync::atomic::Ordering::Relaxed);
                let mut v = vec![0u32; packed.rows()];
                packed.fill(0, &mut v);
                v
            }),
        }
    }

    pub fn iter(&self) -> std::slice::Iter<'_, u32> {
        self.plain().iter()
    }

    pub fn contains(&self, x: &u32) -> bool {
        self.plain().contains(x)
    }
}

/// Rows of compressed columns unpacked to one code per row since the start (what the
/// micro-bench reports: evaluator paths that still need a plain column).
pub(crate) static UNPACKED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Under a memory limit, where an evaluation on this thread reserves the codes it unpacks (lot 5):
/// the pool, the class, the reservations (held until the evaluation ends, then the codes are
/// counted with their column) and the first refusal (the evaluation then fails: no answer is
/// computed from codes that were not unpacked).
pub(crate) struct UnpackGate {
    pub pool: &'static bicraft_mem::Pool,
    pub class: bicraft_mem::Class,
    pub held: Vec<bicraft_mem::Reservation>,
    pub refused: Option<String>,
}

thread_local! {
    pub(crate) static UNPACK: std::cell::RefCell<Option<UnpackGate>> = const { std::cell::RefCell::new(None) };
}

/// Whether `rows` codes may be unpacked now (always, outside an evaluation under a limit).
fn unpack_admitted(rows: usize) -> bool {
    UNPACK.with(|u| {
        let Ok(mut g) = u.try_borrow_mut() else {
            return true;
        };
        let Some(gate) = g.as_mut() else {
            return true;
        };
        let bytes = rows.saturating_mul(4);
        let mut r = gate.pool.empty(gate.class);
        match r.grow(bytes, None, &mut |_| 0) {
            Ok(()) => {
                gate.held.push(r);
                true
            }
            Err(e) => {
                gate.refused.get_or_insert_with(|| {
                    format!(
                        "unpacking a column's codes (one per row) needs {}: {e}",
                        bicraft_mem::size_text(bytes)
                    )
                });
                false
            }
        }
    })
}

impl Codes {
    /// The same codes kept compressed as VertiPaq would (data id = code + 3, blank 2): segments of
    /// `seg` rows, alternately bit-packed and hybrid RLE (runs of 8 equal codes and more), for
    /// tests and micro-benchmarks of the packed paths on synthetic data.
    #[cfg(test)]
    pub fn pack(col: &ColData, seg: usize) -> Codes {
        Self::pack_with(col, seg, 0)
    }

    /// [`Codes::pack`], value-encoding only columns of at least `value_card` distinct values (as
    /// VertiPaq hash-encodes keys and small sets of numbers).
    #[cfg(test)]
    pub fn pack_with(col: &ColData, seg: usize, value_card: usize) -> Codes {
        use bicraft_scan::{Ids, Run, SegData, Segment};
        let codes = col.codes.plain();
        // value-encoded as VertiPaq would a column of whole numbers or Currency in a small range
        // (data id = value - min + 3), else ids in the codes' order
        let units: Option<Vec<i64>> = col
            .values
            .iter()
            .skip(1)
            .map(|v| match v {
                CellValue::Int(x) | CellValue::Decimal(x) => Some(*x),
                _ => None,
            })
            .collect();
        // (lowest value, step): ids = (value - lowest) / step + 3, as VertiPaq's value encoding
        fn gcd(a: i64, b: i64) -> i64 {
            if b == 0 { a.abs() } else { gcd(b, a % b) }
        }
        let range = units.as_ref().filter(|u| u.len() >= value_card).and_then(|u| {
            let (lo, hi) = (*u.iter().min()?, *u.iter().max()?);
            let step = u.iter().fold(0i64, |g, x| gcd(g, x - lo)).max(1);
            ((hi - lo) / step < (1 << 22)).then_some((lo, step))
        });
        let id_of_code: Vec<u32> = (0..col.values.len())
            .map(|c| match (c, &units, range) {
                (0, _, _) => 2,
                (c, Some(u), Some((lo, step))) => {
                    ((u.get(c - 1).copied().unwrap_or(lo) - lo) / step + 3) as u32
                }
                (c, _, _) => c as u32 + 3,
            })
            .collect();
        let id = |c: u32| id_of_code.get(c as usize).copied().unwrap_or(2);
        let bits = |vals: &[u32], base: u32| {
            let max = vals.iter().map(|v| v - base).max().unwrap_or(0);
            let need = (32 - max.leading_zeros()).max(1);
            let w = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 16, 21, 32]
                .into_iter()
                .find(|w| u32::from(*w) >= need)
                .unwrap_or(32);
            let per = 64 / usize::from(w);
            let words = vals
                .chunks(per)
                .map(|ch| {
                    ch.iter().enumerate().fold(0u64, |acc, (i, v)| {
                        acc | (u64::from(v - base) << (i * usize::from(w)))
                    })
                })
                .collect();
            Ids::Bits {
                width: w,
                base,
                words,
            }
        };
        let mut segs = Vec::new();
        for (k, chunk) in codes.chunks(seg.max(1)).enumerate() {
            let ids: Vec<u32> = chunk.iter().map(|c| id(*c)).collect();
            let min_id = ids.iter().copied().min().unwrap_or(2);
            let max_id = ids.iter().copied().max().unwrap_or(2);
            // runs of 8 rows or more over 90 % of the segment (sorted data): RLE as VertiPaq
            // would; else one segment in two
            let long: usize = ids
                .chunk_by(|a, b| a == b)
                .filter(|r| r.len() >= 8)
                .map(<[u32]>::len)
                .sum();
            let data = if k % 2 == 0 && long * 10 < ids.len() * 9 {
                SegData::Plain(bits(&ids, 2))
            } else {
                let mut runs: Vec<Run> = Vec::new();
                let mut sub = Vec::new();
                let mut i = 0;
                while i < ids.len() {
                    let mut j = i;
                    while j < ids.len() && ids.get(j) == ids.get(i) {
                        j += 1;
                    }
                    let (start, len) = (i as u32, (j - i) as u32);
                    if j - i >= 8 {
                        runs.push(Run {
                            start,
                            len,
                            id: ids.get(i).copied().unwrap_or(2),
                            packed: false,
                        });
                    } else {
                        match runs.last_mut() {
                            Some(r) if r.packed && r.start + r.len == start => r.len += len,
                            _ => runs.push(Run {
                                start,
                                len,
                                id: sub.len() as u32,
                                packed: true,
                            }),
                        }
                        sub.extend_from_slice(ids.get(i..j).unwrap_or(&[]));
                    }
                    i = j;
                }
                SegData::Rle {
                    runs,
                    sub: bits(&sub, 2),
                }
            };
            segs.push(Segment {
                rows: chunk.len(),
                data,
                min_id,
                max_id,
            });
        }
        let max_id = id_of_code.iter().copied().max().unwrap_or(3);
        let mut lut = vec![0u32; max_id as usize + 1];
        for (c, i) in id_of_code.iter().enumerate().skip(1) {
            if let Some(s) = lut.get_mut(*i as usize) {
                *s = c as u32;
            }
        }
        Codes::Packed {
            packed: bicraft_scan::Packed::new(segs, lut),
            plain: OnceLock::new(),
            blank: codes.contains(&0),
            affine: range.map(|(lo, step)| (lo - 3 * step, step)),
        }
    }
}

impl Codes {
    /// The compressed segments, when the column is kept so.
    pub fn as_packed(&self) -> Option<&bicraft_scan::Packed> {
        match self {
            Codes::Packed { packed, .. } => Some(packed),
            Codes::Plain(_) => None,
        }
    }
}

impl std::fmt::Debug for Codes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl Clone for Codes {
    fn clone(&self) -> Self {
        match self {
            Codes::Plain(v) => Codes::Plain(v.clone()),
            Codes::Packed {
                packed,
                blank,
                affine,
                ..
            } => Codes::Packed {
                packed: packed.clone(),
                plain: OnceLock::new(),
                blank: *blank,
                affine: *affine,
            },
        }
    }
}

impl PartialEq for Codes {
    fn eq(&self, other: &Self) -> bool {
        self.plain() == other.plain()
    }
}

impl From<Vec<u32>> for Codes {
    fn from(v: Vec<u32>) -> Self {
        Codes::Plain(v)
    }
}

/// The per-code tables derived from a column's values ([`ColData::derived`]), which
/// `dax.clear_cache` drops (the decoded codes stay).
#[derive(Default)]
pub(crate) struct Derived(std::sync::RwLock<Arc<DerivedSet>>);

impl Derived {
    /// The current tables (filled on first use).
    pub fn get(&self) -> Arc<DerivedSet> {
        match self.0.read() {
            Ok(g) => Arc::clone(&g),
            Err(_) => Arc::default(),
        }
    }

    /// Drops what was computed from the rows (rows per code), as Analysis Services' ClearCache
    /// drops its caches; the tables of the dictionary (value index, sort ranks, per-code values),
    /// part of a column as the engine keeps it in memory, stay.
    pub fn clear(&self) {
        if let Ok(mut g) = self.0.write() {
            let old = Arc::clone(&g);
            *g = Arc::new(DerivedSet {
                index: old.index.clone(),
                nums: old.nums.clone(),
                ranks: old.ranks.clone(),
                currency: old.currency.clone(),
                ints: old.ints.clone(),
                counts: OnceLock::new(),
                nonblank: old.nonblank.clone(),
                kinds: old.kinds.clone(),
                firsts: OnceLock::new(),
                rows_of: OnceLock::new(),
            });
        }
    }
}

/// See [`Derived`].
#[derive(Default)]
pub(crate) struct DerivedSet {
    /// Value key → code.
    pub index: OnceLock<Arc<HashMap<Key, u32>>>,
    /// Numeric value per code.
    pub nums: OnceLock<Arc<Vec<Option<f64>>>>,
    /// Sort rank per code, and the code of each rank.
    pub ranks: OnceLock<Arc<(Vec<u32>, Vec<u32>)>>,
    /// Currency value per code (`None`: not a Currency column, or a value does not convert).
    pub currency: OnceLock<Option<Arc<Vec<Option<i64>>>>>,
    /// Whole-number value per code when every number is whole and a sum over every row stays
    /// exact in a double.
    pub ints: OnceLock<Option<Arc<Vec<Option<i64>>>>>,
    /// Rows per code over the whole column (what an aggregation without filter reads).
    pub counts: OnceLock<Option<Arc<Vec<u64>>>>,
    /// Values of the dictionary that are not blank.
    pub nonblank: OnceLock<usize>,
    /// (some value is Currency, every value is Currency or blank, every value is a whole
    /// number or blank).
    pub kinds: OnceLock<(bool, bool, bool)>,
    /// First row of each code (`u32::MAX`: none), for columns of at most
    /// [`ColData::MAX_FIRST_ROWS`] codes.
    pub firsts: OnceLock<Option<Arc<Vec<u32>>>>,
    /// First row of every code (`u32::MAX`: none), read to the end ([`ColData::rows_of_codes`]).
    pub rows_of: OnceLock<Option<Arc<Vec<u32>>>>,
}

impl std::fmt::Debug for Derived {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Derived").finish_non_exhaustive()
    }
}

impl Clone for Derived {
    fn clone(&self) -> Self {
        Derived::default()
    }
}

impl PartialEq for Derived {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl ColData {
    /// (some value is Currency, every value is Currency or blank, every value is a whole number
    /// or blank), computed once per column.
    pub fn kinds(&self) -> (bool, bool, bool) {
        *self.derived.get().kinds.get_or_init(|| {
            let any_cur = self.values.iter().any(|v| matches!(v, CellValue::Decimal(_)));
            let all_cur = self
                .values
                .iter()
                .all(|v| matches!(v, CellValue::Null | CellValue::Decimal(_)));
            let all_int = self
                .values
                .iter()
                .all(|v| matches!(v, CellValue::Null | CellValue::Int(_)));
            (any_cur, all_cur, all_int)
        })
    }

    /// Codes up to which [`ColData::first_rows`] answers.
    pub const MAX_FIRST_ROWS: usize = 1 << 16;

    /// First row of each code (`u32::MAX`: the code has no row), read block by block from the
    /// top until every code is seen: a few blocks for a column of few values. `None` above
    /// [`ColData::MAX_FIRST_ROWS`] codes or past 2^32 rows.
    pub fn first_rows(&self) -> Option<Arc<Vec<u32>>> {
        self.derived
            .get()
            .firsts
            .get_or_init(|| {
                let card = self.values.len();
                let rows = self.codes.len();
                if card > Self::MAX_FIRST_ROWS || u32::try_from(rows).is_err() {
                    return None;
                }
                let mut first = vec![u32::MAX; card];
                // the codes some row holds: every code of a non-blank value, and code 0 when a
                // row is blank (unknown for an unpacked column: every code, read to the end)
                let mut left = match self.codes.known_blank() {
                    Some(blank) => {
                        self.values
                            .iter()
                            .skip(1)
                            .filter(|v| !matches!(v, CellValue::Null))
                            .count()
                            + usize::from(blank)
                    }
                    None => card,
                };
                let col = self.codes.col();
                let mut buf = Vec::new();
                let mut at = 0usize;
                while at < rows && left > 0 {
                    let len = (rows - at).min(2048);
                    for (i, c) in col.block(at, len, &mut buf).iter().enumerate() {
                        if let Some(f) = first.get_mut(*c as usize)
                            && *f == u32::MAX
                        {
                            *f = u32::try_from(at + i).unwrap_or(u32::MAX);
                            left = left.saturating_sub(1);
                        }
                    }
                    at += len;
                }
                Some(Arc::new(first))
            })
            .clone()
    }

    /// First row of every code (`u32::MAX`: the code has no row), whatever the number of codes:
    /// one pass over the rows (stopping once every code that some row holds is met). What
    /// `RELATED` reads to go from a key code to the row of the one side. `None` past 2^32 rows.
    pub fn rows_of_codes(&self) -> Option<Arc<Vec<u32>>> {
        if self.values.len() <= Self::MAX_FIRST_ROWS {
            return self.first_rows();
        }
        self.derived
            .get()
            .rows_of
            .get_or_init(|| {
                let rows = self.codes.len();
                u32::try_from(rows).ok()?;
                let card = self.values.len();
                let mut first = vec![u32::MAX; card];
                let mut left = match self.codes.known_blank() {
                    Some(blank) => {
                        self.values
                            .iter()
                            .skip(1)
                            .filter(|v| !matches!(v, CellValue::Null))
                            .count()
                            + usize::from(blank)
                    }
                    None => card,
                };
                let col = self.codes.col();
                let mut buf = Vec::new();
                let mut at = 0usize;
                while at < rows && left > 0 {
                    let len = (rows - at).min(2048);
                    for (i, c) in col.block(at, len, &mut buf).iter().enumerate() {
                        if let Some(f) = first.get_mut(*c as usize)
                            && *f == u32::MAX
                        {
                            *f = u32::try_from(at + i).unwrap_or(u32::MAX);
                            left = left.saturating_sub(1);
                        }
                    }
                    at += len;
                }
                Some(Arc::new(first))
            })
            .clone()
    }

    pub fn rows(&self) -> usize {
        self.codes.len()
    }

    /// Builds a column from per-row values, deduplicating them (texts case-insensitively).
    #[cfg(test)]
    pub fn from_values(values: Vec<CellValue>) -> Result<Self, Miss> {
        Self::build(values, Key::of)
    }

    /// Builds a column from values as the engine stores them (texts already deduplicated per the
    /// model's collation, see `crate::datasrc::stored_values`): distinct texts stay distinct.
    pub fn from_stored(values: Vec<CellValue>) -> Result<Self, Miss> {
        Self::build(values, |v| match v {
            CellValue::Text(s) => Some(Key::Text(s.clone())),
            other => Key::of(other),
        })
    }

    fn build(values: Vec<CellValue>, key_of: impl Fn(&CellValue) -> Option<Key>) -> Result<Self, Miss> {
        let mut codes: Vec<u32> = Vec::with_capacity(values.len());
        let mut out = ColData {
            codes: Codes::Plain(Vec::new()),
            values: vec![CellValue::Null],
            derived: Derived::default(),
        };
        let mut index: HashMap<Key, u32> = HashMap::new();
        for v in values {
            let Some(key) = key_of(&v) else {
                codes.push(0);
                continue;
            };
            let next = u32::try_from(out.values.len())
                .map_err(|_| Miss::Error("too many distinct values".into()))?;
            let code = *index.entry(key).or_insert(next);
            if code == next {
                out.values.push(v);
            }
            codes.push(code);
        }
        out.codes = Codes::Plain(codes);
        Ok(out)
    }
}

/// A hashable form of a non-blank value. Numbers of every kind compare by value (an `Int64` key
/// matches a `Double` key holding the same number); text compares case-insensitively
/// ([`Key::of`]) or as relationship keys do ([`Key::related`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    Num(u64),
    Text(String),
    Bool(bool),
    /// A relationship key: the stored-text identity of the model's culture.
    Related(bicraft_vertipaq::system::EquivalenceKey),
}

impl Key {
    pub fn of(v: &CellValue) -> Option<Key> {
        let num = |x: f64| {
            // +0.0 and -0.0 are the same key
            let x = if x == 0.0 { 0.0 } else { x };
            Some(Key::Num(x.to_bits()))
        };
        match v {
            CellValue::Null => None,
            CellValue::Int(n) => num(*n as f64),
            CellValue::Real(x) | CellValue::DateTime(x) => num(*x),
            CellValue::Decimal(n) => num(*n as f64 / 10_000.0),
            CellValue::Text(s) => Some(Key::Text(s.to_lowercase())),
            CellValue::Bool(b) => Some(Key::Bool(*b)),
        }
    }

    /// The key of a stored value at either end of a relationship: Analysis Services matches
    /// text keys as it identifies the values of a column (`keys`, see
    /// `crate::datasrc::relationship_keys`; en-US: `a` = `A` but `à` ≠ `À`), not by the
    /// case-insensitive comparison of [`Key::of`].
    pub fn related(v: &CellValue, keys: &bicraft_vertipaq::system::TextStorage) -> Option<Key> {
        match v {
            CellValue::Text(s) => Some(Key::Related(keys.equivalence_key(s))),
            other => Key::of(other),
        }
    }
}

/// Where a column's files are in the decompressed backup.
#[derive(Debug, Clone)]
struct ColumnLoc {
    /// `(.idf, .idfmeta)` per partition, in partition order.
    parts: Vec<(Range<usize>, Range<usize>)>,
    dictionary: Option<Range<usize>>,
    encoding: ColumnEncoding,
}

/// A decompressed VertiPaq backup and the location of every user column in it.
#[derive(Debug, Clone)]
pub(crate) struct VertipaqStore {
    abf: Arc<std::sync::RwLock<Backing>>,
    /// Keyed by lower-cased (table, column).
    columns: HashMap<(String, String), ColumnLoc>,
    /// Lower-cased table name → row count (sum of the partitions' segment maps).
    tables: HashMap<String, u64>,
}

/// Where a store's decompressed backup lives: in memory, or in a temporary file read by
/// positional reads (under a memory budget, `docs/beyond-ram.md`).
#[derive(Debug, Clone)]
enum Backing {
    Memory(Vec<u8>),
    File(Arc<SpillFile>),
}

/// A backup written to a temporary file. The file is deleted when the last store holding it is
/// dropped (and, on Windows, by the system when the handle closes, even after a crash; on Unix it
/// is unlinked as soon as it is created, the open handle keeping its data), also when writing it
/// fails half-way.
#[derive(Debug)]
pub(crate) struct SpillFile {
    file: Option<std::fs::File>,
    path: std::path::PathBuf,
    len: usize,
}

impl Drop for SpillFile {
    fn drop(&mut self) {
        drop(self.file.take());
        // already gone on Unix (unlinked) and Windows (deleted on close): best effort elsewhere
        let _ = std::fs::remove_file(&self.path);
        SPILLS_ALIVE.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        crate::obs::spill_bytes_changed(self.len, 0);
    }
}

/// Temporary files written by this process (their names stay unique within it).
static SPILLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Temporary files of this process alive now (tests: every one goes, error paths included).
pub(crate) static SPILLS_ALIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl SpillFile {
    /// A new, empty temporary file in `dir` (`bicraft-backup-<pid>-<n>.abf`), removed by the
    /// system on close where it can be (Windows: deleted on close, and marked temporary so the
    /// system keeps it in its cache rather than writing it out; Unix: unlinked now).
    pub fn create(dir: &Path) -> std::io::Result<SpillFile> {
        let n = SPILLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = dir.join(format!("bicraft-backup-{}-{n}.abf", std::process::id()));
        let mut o = std::fs::OpenOptions::new();
        o.read(true).write(true).create_new(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // GENERIC_READ | GENERIC_WRITE | DELETE, FILE_FLAG_DELETE_ON_CLOSE,
            // FILE_ATTRIBUTE_TEMPORARY; the other handles may read, write or delete it
            o.access_mode(0x8000_0000 | 0x4000_0000 | 0x0001_0000)
                .custom_flags(0x0400_0000)
                .attributes(0x100)
                .share_mode(0x1 | 0x2 | 0x4);
        }
        let file = o.open(&path)?;
        // Unix: the name goes now, the open handle keeps the data until it closes
        #[cfg(unix)]
        let _ = std::fs::remove_file(&path);
        SPILLS_ALIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(SpillFile {
            file: Some(file),
            path,
            len: 0,
        })
    }

    /// Writes `bytes` at `at` (the file grows to cover them).
    pub fn write_at(&mut self, at: u64, bytes: &[u8]) -> std::io::Result<()> {
        use std::io::{Seek, SeekFrom, Write};
        let end = at
            .checked_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
            .and_then(|e| usize::try_from(e).ok())
            .ok_or_else(|| std::io::Error::other("the temporary file is too large"))?;
        let f = self
            .file
            .as_mut()
            .ok_or_else(|| std::io::Error::other("the temporary file is closed"))?;
        f.seek(SeekFrom::Start(at))?;
        f.write_all(bytes)?;
        let len = self.len.max(end);
        crate::obs::spill_bytes_changed(self.len, len);
        self.len = len;
        Ok(())
    }

    /// Writes `bytes` at `at` through the shared handle (positional writes: several threads may
    /// write at once); the caller sets the length once done ([`SpillFile::set_len`]).
    pub fn write_shared(&self, at: u64, bytes: &[u8]) -> std::io::Result<()> {
        let f = self
            .file
            .as_ref()
            .ok_or_else(|| std::io::Error::other("the temporary file is closed"))?;
        #[cfg(unix)]
        {
            std::os::unix::fs::FileExt::write_all_at(f, bytes, at)
        }
        #[cfg(windows)]
        {
            let mut done = 0usize;
            while done < bytes.len() {
                let rest = bytes.get(done..).unwrap_or_default();
                let off = at.saturating_add(u64::try_from(done).unwrap_or(u64::MAX));
                let n = std::os::windows::fs::FileExt::seek_write(f, rest, off)?;
                if n == 0 {
                    return Err(std::io::Error::other("the temporary file takes no more bytes"));
                }
                done += n;
            }
            Ok(())
        }
    }

    /// The bytes written by [`SpillFile::write_shared`].
    pub fn set_len(&mut self, len: usize) {
        crate::obs::spill_bytes_changed(self.len, len);
        self.len = len;
    }

    /// Reads `buf.len()` bytes at `at`, within what was written.
    pub fn read_at(&self, at: u64, buf: &mut [u8]) -> std::io::Result<()> {
        let end = at.checked_add(u64::try_from(buf.len()).unwrap_or(u64::MAX));
        if end.is_none_or(|e| e > self.len as u64) {
            return Err(std::io::Error::other("read past the end of the temporary file"));
        }
        let f = self
            .file
            .as_ref()
            .ok_or_else(|| std::io::Error::other("the temporary file is closed"))?;
        bicraft_opc::read_exact_at(f, at, buf)
    }
}

impl VertipaqStore {
    /// Moves the decompressed backup to a temporary file in `dir` and frees it from memory;
    /// columns are then read from the file. Every clone of the store shares the move. A file
    /// that cannot be written leaves the backup in memory (the error is returned; the store stays
    /// usable). Reads go on while the file is written; only the switch takes the lock.
    pub fn spill(&self, dir: &Path) -> Result<(), String> {
        let file = {
            let guard = self.abf.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            let Backing::Memory(bytes) = &*guard else {
                return Ok(());
            };
            let written = (|| -> std::io::Result<SpillFile> {
                let mut f = SpillFile::create(dir)?;
                f.write_at(0, bytes)?;
                Ok(f)
            })();
            match written {
                Ok(f) => f,
                Err(e) => return Err(format!("cannot write the backup to {}: {e}", dir.display())),
            }
        };
        let mut guard = self
            .abf
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if matches!(&*guard, Backing::Memory(_)) {
            *guard = Backing::File(Arc::new(file));
        }
        Ok(())
    }

    /// Decompresses the `DataModel` part (or `cache.abf`) that `open` reads straight to a new
    /// temporary file in `dir` (`docs/beyond-ram.md`, lot 2: neither the part nor the backup is
    /// held whole in memory) and indexes its columns from the file. Returns the store and the
    /// backup's `metadata.sqlitedb` (the model). On any error the file is removed.
    pub fn open_streamed<'a>(
        open: &mut dyn FnMut() -> Result<Box<dyn std::io::Read + Send + 'a>, String>,
        dir: &Path,
        max_bytes: usize,
    ) -> Result<(Self, Vec<u8>), String> {
        Self::open_streamed_in(open, dir, max_bytes, bicraft_mem::global())
    }

    /// [`VertipaqStore::open_streamed`] with its decompression buffers reserved in `pool` (lot 5):
    /// under a memory limit, on fewer threads (and smaller pieces) when the buffers would not fit in
    /// half of what is left, refused when even one thread does not fit.
    pub fn open_streamed_in<'a>(
        open: &mut dyn FnMut() -> Result<Box<dyn std::io::Read + Send + 'a>, String>,
        dir: &Path,
        max_bytes: usize,
        pool: &'static bicraft_mem::Pool,
    ) -> Result<(Self, Vec<u8>), String> {
        let (plan, bytes) = stream_plan(pool);
        let mut buffers = pool.empty(bicraft_mem::Class::Io);
        buffers.grow(bytes, None, &mut |_| 0).map_err(|e| {
            format!(
                "decompressing the stored data needs about {} of buffers: {e}",
                bicraft_mem::size_text(bytes)
            )
        })?;
        let mut file = SpillFile::create(dir)
            .map_err(|e| format!("cannot create a temporary file in {}: {e}", dir.display()))?;
        let mut opener = || open().map_err(bicraft_xpress9::Error::Read);
        let shared = &file;
        let sink = |at: u64, bytes: &[u8]| {
            shared
                .write_shared(at, bytes)
                .map_err(|e| bicraft_xpress9::Error::Write(e.to_string()))
        };
        let limit = u64::try_from(max_bytes).unwrap_or(u64::MAX);
        let written = match plan {
            Some((threads, piece)) => {
                bicraft_xpress9::decompress_datamodel_into_with(&mut opener, limit, threads, piece, &sink)
            }
            None => bicraft_xpress9::decompress_datamodel_into_lean(&mut opener, limit, &sink),
        }
        .map_err(|e| e.to_string())?;
        file.set_len(usize::try_from(written).map_err(|_| "the backup is too large".to_string())?);
        drop(buffers);
        Self::from_spill(file)
    }

    /// Indexes the columns of a backup already in a temporary file; also returns its
    /// `metadata.sqlitedb`.
    pub fn from_spill(file: SpillFile) -> Result<(Self, Vec<u8>), String> {
        let (index_of, sqlitedb) = bicraft_pbix::backup_index_at(&mut |at, len| {
            let n = usize::try_from(len).map_err(|_| bicraft_pbix::Error::Verify("backup range"))?;
            let mut buf: Vec<u8> = Vec::new();
            buf.try_reserve_exact(n)
                .map_err(|_| bicraft_pbix::Error::Verify("backup range"))?;
            buf.resize(n, 0);
            file.read_at(at, &mut buf)
                .map_err(|e| bicraft_pbix::Error::Package(bicraft_opc::Error::Io(e)))?;
            Ok(buf)
        })
        .map_err(|e| e.to_string())?;
        let (columns, tables) = {
            let db = bicraft_sqlite::Database::from_bytes(&sqlitedb).map_err(|e| e.to_string())?;
            let len = file.len;
            index(
                &|path| {
                    let e = index_of.entry(path)?;
                    let end = e.offset.checked_add(e.size)?;
                    (end <= len).then_some(e.offset..end)
                },
                &db,
            )?
        };
        Ok((
            Self {
                abf: Arc::new(std::sync::RwLock::new(Backing::File(Arc::new(file)))),
                columns,
                tables,
            },
            sqlitedb,
        ))
    }

    /// Whether the backup is in a temporary file, and its size.
    pub fn backing(&self) -> (&'static str, usize) {
        match &*self.abf.read().unwrap_or_else(std::sync::PoisonError::into_inner) {
            Backing::Memory(b) => ("memory", b.len()),
            Backing::File(f) => ("file", f.len),
        }
    }

    /// Runs `f` on the backup's bytes at `r`: in memory, or read from the temporary file.
    fn read<R>(&self, r: &Range<usize>, f: impl FnOnce(&[u8]) -> Result<R, Miss>) -> Result<R, Miss> {
        read_backing(&self.abf, r, f)
    }

    /// Bytes of a column's files in the backup (segments and dictionary): a lower bound of what
    /// it takes once read, known before reading it. `None`: no such stored column.
    pub fn column_file_bytes(&self, table: &str, column: &str) -> Option<usize> {
        let loc = self.columns.get(&(table.to_lowercase(), column.to_lowercase()))?;
        let parts: usize = loc.parts.iter().map(|(idf, _)| idf.len()).sum();
        Some(parts.saturating_add(loc.dictionary.as_ref().map_or(0, Range::len)))
    }
}

type SqlRow = std::collections::BTreeMap<String, Sql>;

fn int(row: &SqlRow, col: &str) -> Option<i64> {
    match row.get(col)? {
        Sql::Integer(n) => Some(*n),
        Sql::Real(x) if x.fract() == 0.0 && x.abs() < 9.0e15 => Some(*x as i64),
        _ => None,
    }
}

fn real(row: &SqlRow, col: &str) -> Option<f64> {
    match row.get(col)? {
        Sql::Integer(n) => Some(*n as f64),
        Sql::Real(x) => Some(*x),
        _ => None,
    }
}

fn text<'r>(row: &'r SqlRow, col: &str) -> Option<&'r str> {
    match row.get(col)? {
        Sql::Text(s) if !s.is_empty() => Some(s),
        _ => None,
    }
}

fn by_id(rows: &[SqlRow]) -> HashMap<i64, &SqlRow> {
    rows.iter().filter_map(|r| Some((int(r, "ID")?, r))).collect()
}

impl VertipaqStore {
    /// Decompresses a `DataModel` part (or a `cache.abf`, same framing) and indexes its columns.
    pub fn open(part: &[u8], max_bytes: usize) -> Result<Self, String> {
        let abf_bytes = bicraft_xpress9::decompress_datamodel(part, max_bytes).map_err(|e| e.to_string())?;
        Self::from_backup(abf_bytes, max_bytes)
    }

    /// Indexes the columns of an already decompressed backup (refused over `max_bytes`, as
    /// [`VertipaqStore::open`] refuses to decompress one).
    pub fn from_backup(abf_bytes: Vec<u8>, max_bytes: usize) -> Result<Self, String> {
        if abf_bytes.len() > max_bytes {
            return Err(format!(
                "the {}-byte backup is over the {max_bytes}-byte limit",
                abf_bytes.len()
            ));
        }
        let (columns, tables) = {
            let abf = bicraft_abf::Abf::parse(&abf_bytes).map_err(|e| e.to_string())?;
            let sqlitedb = abf
                .metadata_sqlitedb()
                .ok_or("the backup has no metadata.sqlitedb")?;
            let db = bicraft_sqlite::Database::from_bytes(sqlitedb).map_err(|e| e.to_string())?;
            index(
                &|path| {
                    let e = abf.entry(path)?;
                    Some(e.offset..e.offset.checked_add(e.size)?)
                },
                &db,
            )?
        };
        Ok(Self {
            abf: Arc::new(std::sync::RwLock::new(Backing::Memory(abf_bytes))),
            columns,
            tables,
        })
    }

    /// Reads the stored tables and columns under new names: `tables` as (old, new), then
    /// `columns` as (table by its new name, old, new). For a save that renames objects and
    /// evaluates expressions over them.
    pub fn rename(&mut self, tables: &[(String, String)], columns: &[(String, String, String)]) {
        let lower = |s: &str| s.to_lowercase();
        for (old, new) in tables {
            let (old, new) = (lower(old), lower(new));
            if let Some(rows) = self.tables.remove(&old) {
                self.tables.insert(new.clone(), rows);
            }
            let keys: Vec<(String, String)> =
                self.columns.keys().filter(|(t, _)| *t == old).cloned().collect();
            let moved: Vec<((String, String), ColumnLoc)> = keys
                .into_iter()
                .filter_map(|k| self.columns.remove(&k).map(|loc| ((new.clone(), k.1), loc)))
                .collect();
            self.columns.extend(moved);
        }
        let moved: Vec<((String, String), ColumnLoc)> = columns
            .iter()
            .filter_map(|(t, old, new)| {
                self.columns
                    .remove(&(lower(t), lower(old)))
                    .map(|loc| ((lower(t), lower(new)), loc))
            })
            .collect();
        self.columns.extend(moved);
    }

    pub fn has_table(&self, table: &str) -> bool {
        self.tables.contains_key(&table.to_lowercase())
    }

    pub fn row_count(&self, table: &str) -> Option<u64> {
        self.tables.get(&table.to_lowercase()).copied()
    }

    /// One column, kept compressed when the packed reader takes it, else decoded; tables over
    /// `max_rows` rows are refused.
    pub fn column(&self, table: &str, column: &str, max_rows: usize) -> Result<ColData, Miss> {
        match self.column_packed(table, column, max_rows)? {
            Some(c) => Ok(c),
            None => self.column_plain(table, column, max_rows),
        }
    }

    /// [`VertipaqStore::column`] kept compressed; `None`: a column the packed reader does not
    /// take (data ids out of the range its statistics give), read in full instead.
    fn column_packed(&self, table: &str, column: &str, max_rows: usize) -> Result<Option<ColData>, Miss> {
        let key = (table.to_lowercase(), column.to_lowercase());
        let loc = self
            .columns
            .get(&key)
            .ok_or_else(|| Miss::Unavailable(format!("no stored data for column '{table}'[{column}]")))?;
        let limits = bicraft_vertipaq::Limits {
            max_rows: u64::try_from(max_rows).unwrap_or(u64::MAX),
            ..bicraft_vertipaq::Limits::default()
        };
        // the segments of every partition, kept compressed
        let mut segs: Vec<bicraft_scan::Segment> = Vec::new();
        let mut rows_total = 0u64;
        for (idf, meta) in &loc.parts {
            let meta = self.read(meta, |b| parse_idfmeta(b).map_err(|e| Miss::Error(e.to_string())))?;
            let rows = meta.row_count().unwrap_or(u64::MAX);
            rows_total = rows_total.saturating_add(rows);
            if rows_total > limits.max_rows {
                return Err(Miss::Error(format!(
                    "table '{table}' has more than {max_rows} rows (preview limit)"
                )));
            }
            segs.extend(self.read(idf, |b| packed_segments(b, &meta))?);
        }
        let dictionary = match (loc.encoding.kind, &loc.dictionary) {
            (EncodingKind::Hash, Some(r)) => {
                Some(self.read(r, |b| parse_dictionary(b).map_err(|e| Miss::Error(e.to_string())))?)
            }
            // A column holding only blanks has no `.dictionary`; `ids_to_values` refuses any
            // other id without one.
            _ => None,
        };
        // distinct ids in first-seen order → codes 1.. (as the rows meet them)
        let Some(distinct) = first_seen_ids(&mut segs)? else {
            return Ok(None);
        };
        let (values, lut, blank, affine) = codes_of(&distinct, dictionary.as_ref(), loc.encoding)?;
        Ok(Some(ColData {
            codes: Codes::Packed {
                packed: bicraft_scan::Packed::new(segs, lut),
                plain: OnceLock::new(),
                blank,
                affine,
            },
            values,
            derived: Derived::default(),
        }))
    }

    /// Decodes one column into one code per row, refusing tables over `max_rows` rows.
    fn column_plain(&self, table: &str, column: &str, max_rows: usize) -> Result<ColData, Miss> {
        let key = (table.to_lowercase(), column.to_lowercase());
        let loc = self
            .columns
            .get(&key)
            .ok_or_else(|| Miss::Unavailable(format!("no stored data for column '{table}'[{column}]")))?;
        let limits = bicraft_vertipaq::Limits {
            max_rows: u64::try_from(max_rows).unwrap_or(u64::MAX),
            ..bicraft_vertipaq::Limits::default()
        };
        let mut ids: Vec<u32> = Vec::new();
        for (idf, meta) in &loc.parts {
            let meta = self.read(meta, |b| parse_idfmeta(b).map_err(|e| Miss::Error(e.to_string())))?;
            let rows = meta.row_count().unwrap_or(u64::MAX);
            let total = u64::try_from(ids.len()).unwrap_or(u64::MAX).saturating_add(rows);
            if total > limits.max_rows {
                return Err(Miss::Error(format!(
                    "table '{table}' has more than {max_rows} rows (preview limit)"
                )));
            }
            let part = self.read(idf, |b| decode_parallel(b, &meta, &limits))?;
            ids.extend(part);
        }
        let dictionary = match (loc.encoding.kind, &loc.dictionary) {
            (EncodingKind::Hash, Some(r)) => {
                Some(self.read(r, |b| parse_dictionary(b).map_err(|e| Miss::Error(e.to_string())))?)
            }
            // A column holding only blanks has no `.dictionary`; `ids_to_values` refuses any
            // other id without one.
            _ => None,
        };
        // distinct ids in first-seen order → codes: data ids index the dictionary (or are value
        // offsets), so a table indexed by id does the numbering when they are dense enough
        let max_id = ids.iter().copied().max().unwrap_or(0) as usize;
        let mut distinct: Vec<u32> = Vec::new();
        let mut codes: Vec<u32> = Vec::with_capacity(ids.len());
        if max_id <= ids.len().saturating_mul(4).max(1 << 20) {
            let mut code_of: Vec<u32> = vec![0; max_id.saturating_add(1)];
            for id in ids {
                let Some(slot) = code_of.get_mut(id as usize) else {
                    return Err(Miss::Error("data id out of range".into()));
                };
                if *slot == 0 {
                    distinct.push(id);
                    *slot = u32::try_from(distinct.len())
                        .map_err(|_| Miss::Error("too many distinct values".into()))?;
                }
                codes.push(*slot);
            }
        } else {
            let mut code_of: HashMap<u32, u32> = HashMap::new();
            for id in ids {
                let next = u32::try_from(distinct.len())
                    .ok()
                    .and_then(|n| n.checked_add(1))
                    .ok_or_else(|| Miss::Error("too many distinct values".into()))?;
                let code = *code_of.entry(id).or_insert(next);
                if code == next {
                    distinct.push(id);
                }
                codes.push(code);
            }
        }
        let decoded = ids_to_values(&distinct, dictionary.as_ref(), loc.encoding)
            .map_err(|e| Miss::Error(e.to_string()))?;
        let mut values = Vec::with_capacity(decoded.len().saturating_add(1));
        values.push(CellValue::Null);
        values.extend(decoded);
        // blank ids (and values that decode to blank) all use code 0
        let blank: Vec<bool> = values.iter().map(|v| matches!(v, CellValue::Null)).collect();
        if blank.iter().skip(1).any(|b| *b) {
            for c in &mut codes {
                if blank.get(*c as usize).copied().unwrap_or(true) {
                    *c = 0;
                }
            }
        }
        Ok(ColData {
            codes: Codes::Plain(codes),
            values,
            derived: Derived::default(),
        })
    }
}

/// `(a, b)` when every id from `FIRST_DATA_ID` on (`distinct[k]`, value `values[k + 1]`) holds the
/// whole-number or Currency value `a + b × id` and every id below holds a blank.
fn affine_values(distinct: &[u32], values: &[CellValue]) -> Option<(i64, i64)> {
    let first = bicraft_vertipaq::FIRST_DATA_ID;
    let units = |v: &CellValue| match v {
        CellValue::Decimal(x) | CellValue::Int(x) => Some(*x),
        _ => None,
    };
    let kind = |v: &CellValue| std::mem::discriminant(v);
    let mut pts: Vec<(i64, i64)> = Vec::new();
    let mut want = None;
    for (k, id) in distinct.iter().enumerate() {
        let v = values.get(k + 1)?;
        if *id < first {
            if !matches!(v, CellValue::Null) {
                return None;
            }
            continue;
        }
        let x = units(v)?;
        // one kind of value only (Currency or whole numbers)
        match want {
            None => want = Some(kind(v)),
            Some(w) if w != kind(v) => return None,
            Some(_) => {}
        }
        if pts.len() < 2 && pts.first().is_none_or(|p| p.0 != i64::from(*id)) {
            pts.push((i64::from(*id), x));
        }
    }
    let (b, a) = match pts.as_slice() {
        [(_, x)] => (0, *x),
        [(i1, x1), (i2, x2)] => {
            let (dx, di) = (x2.checked_sub(*x1)?, i2.checked_sub(*i1)?);
            if di == 0 || dx % di != 0 {
                return None;
            }
            let b = dx / di;
            (b, x1.checked_sub(b.checked_mul(*i1)?)?)
        }
        _ => return None,
    };
    for (k, id) in distinct.iter().enumerate() {
        if *id < first {
            continue;
        }
        let x = units(values.get(k + 1)?)?;
        if b.checked_mul(i64::from(*id)).and_then(|y| y.checked_add(a)) != Some(x) {
            return None;
        }
    }
    Some((a, b))
}

/// One chunk of consecutive segments: its first-seen data ids (in row order across the chunk),
/// the set of those ids as a bitmap, and each segment's lowest and highest id.
type ChunkIds = (Vec<u32>, Vec<u64>, Vec<(u32, u32)>);

/// Largest data id a packed column numbers with a table (beyond: a malformed column).
const MAX_DATA_ID: u32 = 1 << 30;

/// The distinct data ids of the segments, in the order the rows first meet them.
///
/// The segments are cut into chunks of consecutive segments, one per thread; each chunk yields
/// its ids in first-seen order and their bitmap. The chunks are then joined in order: an id of
/// chunk `k` is kept unless an earlier chunk has it, tested against the union of the earlier
/// chunks' bitmaps (a running union, built once in order, then the chunks filtered on several
/// threads). The bitmaps are sized by the segments' statistics; each segment's exact id range
/// replaces its statistics on the way (what segment skipping trusts). An id outside the
/// statistics, or ids too large: `None` (the column is decoded in full instead).
fn first_seen_ids(segs: &mut [bicraft_scan::Segment]) -> Result<Option<Vec<u32>>, Miss> {
    let max_id = segs.iter().map(|s| s.max_id).max().unwrap_or(0);
    if max_id > MAX_DATA_ID {
        return Ok(None);
    }
    let overflow = std::sync::atomic::AtomicBool::new(false);
    let words = (max_id as usize).saturating_add(1).div_ceil(64);
    let chunk_ids = |chunk: &[bicraft_scan::Segment]| -> ChunkIds {
        let mut seen = vec![0u64; words];
        let mut out = Vec::new();
        let mut ranges = Vec::with_capacity(chunk.len());
        let mut buf = vec![0u32; 4096];
        for seg in chunk {
            let (mut lo, mut hi) = (u32::MAX, 0u32);
            let mut take = |ids: &bicraft_scan::Ids, from: usize, len: usize| {
                let mut at = 0usize;
                while at < len {
                    let n = (len - at).min(buf.len());
                    if let Some(b) = buf.get_mut(..n) {
                        ids.fill(from + at, b);
                        for id in b.iter() {
                            lo = lo.min(*id);
                            hi = hi.max(*id);
                            let (w, bit) = (*id as usize / 64, id % 64);
                            match seen.get_mut(w) {
                                Some(x) if *x & (1u64 << bit) == 0 => {
                                    *x |= 1u64 << bit;
                                    out.push(*id);
                                }
                                Some(_) => {}
                                None => overflow.store(true, std::sync::atomic::Ordering::Relaxed),
                            }
                        }
                    }
                    at += n;
                }
            };
            match &seg.data {
                bicraft_scan::SegData::Plain(ids) => take(ids, 0, seg.rows),
                bicraft_scan::SegData::Rle { runs, sub } => {
                    for r in runs {
                        if r.packed {
                            take(sub, r.id as usize, r.len as usize);
                        } else {
                            take(&bicraft_scan::Ids::Seq { first: r.id }, 0, 1);
                        }
                    }
                }
            }
            ranges.push((lo, hi));
        }
        (out, seen, ranges)
    };
    let threads = bicraft_scan::configured_threads().min(segs.len()).max(1);
    let per = segs.len().div_ceil(threads).max(1);
    let mut chunks: Vec<ChunkIds> = if threads == 1 {
        vec![chunk_ids(segs)]
    } else {
        let parts: Vec<&[bicraft_scan::Segment]> = segs.chunks(per).collect();
        bicraft_scan::pool::map(parts.len(), |i| parts.get(i).map(|c| chunk_ids(c)))
            .ok_or_else(|| Miss::Error("a column reader failed".into()))?
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| Miss::Error("a column reader failed".into()))?
    };
    if overflow.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(None);
    }
    // each segment's exact id range
    for (seg, (lo, hi)) in segs
        .iter_mut()
        .zip(chunks.iter().flat_map(|c| c.2.iter().copied()))
    {
        if lo <= hi {
            seg.min_id = lo;
            seg.max_id = hi;
        }
    }
    if chunks.len() == 1 {
        return Ok(chunks.pop().map(|c| c.0));
    }
    // each chunk's bitmap becomes the union of the chunks before it
    let mut union = vec![0u64; words];
    for chunk in &mut chunks {
        for (u, b) in union.iter_mut().zip(chunk.1.iter_mut()) {
            let mine = *b;
            *b = *u;
            *u |= mine;
        }
    }
    drop(union);
    let keep = |c: &ChunkIds| -> Vec<u32> {
        c.0.iter()
            .copied()
            .filter(|id| {
                c.1.get(*id as usize / 64)
                    .is_none_or(|w| w & (1u64 << (id % 64)) == 0)
            })
            .collect()
    };
    let kept: Vec<Vec<u32>> = bicraft_scan::pool::map(chunks.len(), |i| chunks.get(i).map(keep))
        .ok_or_else(|| Miss::Error("a column reader failed".into()))?
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| Miss::Error("a column reader failed".into()))?;
    let mut distinct = Vec::with_capacity(kept.iter().map(Vec::len).sum());
    for k in kept {
        distinct.extend(k);
    }
    Ok(Some(distinct))
}

/// The segments of one partition's `.idf`, kept compressed (the words copied out of the backup).
fn packed_segments(
    idf: &[u8],
    meta: &bicraft_vertipaq::ColumnMeta,
) -> Result<Vec<bicraft_scan::Segment>, Miss> {
    let err = |e: bicraft_vertipaq::Error| Miss::Error(e.to_string());
    let blocks = bicraft_vertipaq::split_idf(idf, meta).map_err(err)?;
    let one = parse_segment;
    let pairs: Vec<_> = meta.segments.iter().zip(&blocks).collect();
    // the words of each segment copied out of the backup on several threads
    if pairs.len() > 1 && bicraft_scan::configured_threads() > 1 {
        let done = bicraft_scan::pool::map(pairs.len(), |i| pairs.get(i).map(|(s, b)| one(s, b)))
            .ok_or_else(|| Miss::Error("a column reader failed".into()))?;
        return done
            .into_iter()
            .map(|r| r.unwrap_or_else(|| Err(Miss::Error("a column reader failed".into()))))
            .collect();
    }
    pairs.into_iter().map(|(s, b)| one(s, b)).collect()
}

/// One segment of an `.idf`, its words copied out of the backup (`seg`: its `.idfmeta` entry).
fn parse_segment(
    seg: &bicraft_vertipaq::SegmentMeta,
    block: &bicraft_vertipaq::idf::IdfSegment<'_>,
) -> Result<bicraft_scan::Segment, Miss> {
    use bicraft_scan::{Ids, Run, SegData, Segment};
    use bicraft_vertipaq::{Compression, InnerCompression};
    let bad = |m: &str| Miss::Error(format!("malformed column data: {m}"));
    let words = |b: &[u8]| -> Vec<u64> {
        b.as_chunks::<8>()
            .0
            .iter()
            .map(|w| u64::from_le_bytes(*w))
            .collect()
    };
    let rows = usize::try_from(seg.records).map_err(|_| bad("segment rows"))?;
    // the statistics' id range (an id beyond it makes the column decoded in full)
    let min_id = seg.stats.min_data_id;
    let mut max_id = seg.stats.max_data_id.max(min_id);
    if seg.stats.has_nulls != 0 {
        max_id = max_id.max(bicraft_vertipaq::NULL_DATA_ID);
    }
    let compression = seg.compression().ok_or_else(|| bad("unknown compression"))?;
    let check_bits = |w: u8, n: usize, b: &[u8]| -> Result<(), Miss> {
        let per = 64 / usize::from(w.max(1));
        if w == 0 || w > 32 || n.div_ceil(per) > b.len() / 8 {
            return Err(bad("bit-packed segment is shorter than its rows"));
        }
        Ok(())
    };
    let data = match compression {
        Compression::NoSplit(w) => {
            check_bits(w, rows, block.primary)?;
            let base = u32::try_from(seg.base_id).map_err(|_| bad("segment base id"))?;
            SegData::Plain(Ids::Bits {
                width: w,
                base,
                words: words(block.primary),
            })
        }
        Compression::Xm123 => {
            max_id = max_id.max(min_id.saturating_add(u32::try_from(rows).unwrap_or(u32::MAX)));
            SegData::Plain(Ids::Seq { first: min_id })
        }
        Compression::HybridRle(inner) => {
            let packed_count = seg
                .sub_segment
                .map_or(Ok(0), |s| usize::try_from(s.records))
                .map_err(|_| bad("sub-segment record count"))?;
            let sub = match inner {
                InnerCompression::NoSplit(w) => {
                    let b = block.sub.unwrap_or(&[]);
                    check_bits(w, packed_count, b)?;
                    let base = if seg.stats.has_nulls != 0 {
                        bicraft_vertipaq::NULL_DATA_ID
                    } else {
                        min_id
                    };
                    Ids::Bits {
                        width: w,
                        base,
                        words: words(b),
                    }
                }
                InnerCompression::Xm123 => {
                    max_id =
                        max_id.max(min_id.saturating_add(u32::try_from(packed_count).unwrap_or(u32::MAX)));
                    Ids::Seq { first: min_id }
                }
            };
            let mut runs = Vec::new();
            let mut produced = 0usize;
            for e in bicraft_vertipaq::idf::rle_entries(block.primary) {
                if produced == rows {
                    break;
                }
                let (len, id, packed) = match e {
                    bicraft_vertipaq::idf::RleEntry::Run { data_id, count } => (count, data_id, false),
                    bicraft_vertipaq::idf::RleEntry::BitPacked { offset, count } => {
                        let end = usize::try_from(offset)
                            .ok()
                            .and_then(|o| o.checked_add(count as usize));
                        if end.is_none_or(|e| e > packed_count) {
                            return Err(bad("RLE entry points past the bit-packed values"));
                        }
                        (count, u32::try_from(offset).map_err(|_| bad("RLE offset"))?, true)
                    }
                };
                let n = (len as usize).min(rows - produced);
                runs.push(Run {
                    start: u32::try_from(produced).map_err(|_| bad("segment rows"))?,
                    len: u32::try_from(n).map_err(|_| bad("run length"))?,
                    id,
                    packed,
                });
                produced += n;
            }
            if produced != rows {
                return Err(bad("RLE entries do not cover the segment"));
            }
            SegData::Rle { runs, sub }
        }
    };
    Ok(Segment {
        rows,
        data,
        min_id,
        max_id,
    })
}

/// The data ids of one partition, its segments decoded on several threads (each into its own
/// buffer, joined in order).
fn decode_parallel(
    idf: &[u8],
    meta: &bicraft_vertipaq::ColumnMeta,
    limits: &bicraft_vertipaq::Limits,
) -> Result<Vec<u32>, Miss> {
    let err = |e: bicraft_vertipaq::Error| Miss::Error(e.to_string());
    let threads = bicraft_scan::configured_threads().min(meta.segments.len());
    if threads <= 1 {
        return decode_data_ids(idf, meta, limits).map_err(err);
    }
    let blocks = bicraft_vertipaq::split_idf(idf, meta).map_err(err)?;
    let pairs: Vec<_> = meta.segments.iter().zip(&blocks).collect();
    let per = pairs.len().div_ceil(threads).max(1);
    let chunks: Vec<_> = pairs.chunks(per).collect();
    let parts: Vec<Result<Vec<u32>, Miss>> = bicraft_scan::pool::map(chunks.len(), |i| {
        let mut out = Vec::new();
        for (seg, block) in chunks.get(i).copied().unwrap_or_default() {
            bicraft_vertipaq::idf::decode_segment(seg, block, limits, &mut out).map_err(err)?;
        }
        Ok(out)
    })
    .ok_or_else(|| Miss::Error("a column decoder failed".into()))?;
    let mut out = Vec::new();
    for p in parts {
        out.extend(p?);
    }
    Ok(out)
}

/// Maps every user column to its files.
#[allow(clippy::type_complexity)]
fn index(
    entry: &dyn Fn(&str) -> Option<Range<usize>>,
    db: &bicraft_sqlite::Database,
) -> Result<(HashMap<(String, String), ColumnLoc>, HashMap<String, u64>), String> {
    let load = |name: &str| -> Result<Vec<SqlRow>, String> {
        if db.table(name).is_none() {
            return Ok(Vec::new());
        }
        db.query_all(name).map_err(|e| format!("{name}: {e}"))
    };
    let tables = load("Table")?;
    let columns = load("Column")?;
    let column_storage = load("ColumnStorage")?;
    let dictionary_storage = load("DictionaryStorage")?;
    let column_partitions = load("ColumnPartitionStorage")?;
    let segment_storage = load("SegmentStorage")?;
    let partition_storage = load("PartitionStorage")?;
    let segment_maps = load("SegmentMapStorage")?;
    let partitions = load("Partition")?;
    let files = load("StorageFile")?;
    let folders = load("StorageFolder")?;

    let tables_by_id = by_id(&tables);
    let cs_by_id = by_id(&column_storage);
    let ds_by_id = by_id(&dictionary_storage);
    let ss_by_id = by_id(&segment_storage);
    let ps_by_id = by_id(&partition_storage);
    let sm_by_id = by_id(&segment_maps);
    let folders_by_id = by_id(&folders);
    let files_by_id = by_id(&files);

    // StorageFile id → byte range in the backup
    let file_range = |id: i64| -> Option<Range<usize>> {
        let f = files_by_id.get(&id)?;
        let name = text(f, "FileName")?;
        let folder = int(f, "StorageFolderID")
            .and_then(|fid| folders_by_id.get(&fid))
            .and_then(|r| text(r, "Path"))
            .unwrap_or("");
        let path = if folder.is_empty() {
            name.to_string()
        } else {
            format!("{folder}\\{name}")
        };
        entry(&path)
    };

    // column storage id → [(partition storage order key, idf file, idfmeta file)]
    let mut parts_of: HashMap<i64, Vec<((i64, i64), i64, i64)>> = HashMap::new();
    for cps in &column_partitions {
        let (Some(cs), Some(idf), Some(seg)) = (
            int(cps, "ColumnStorageID"),
            int(cps, "StorageFileID"),
            int(cps, "SegmentStorageID"),
        ) else {
            continue;
        };
        let Some(meta) = ss_by_id.get(&seg).and_then(|s| int(s, "StorageFileID")) else {
            continue;
        };
        let ps = int(cps, "PartitionStorageID").unwrap_or(0);
        let order = ps_by_id
            .get(&ps)
            .and_then(|p| int(p, "StoragePosition"))
            .unwrap_or(0);
        parts_of.entry(cs).or_default().push(((order, ps), idf, meta));
    }

    // table rows: the partitions' segment maps
    let mut rows_of_table: HashMap<i64, u64> = HashMap::new();
    for p in &partitions {
        let (Some(table), Some(ps)) = (int(p, "TableID"), int(p, "PartitionStorageID")) else {
            continue;
        };
        let records = ps_by_id
            .get(&ps)
            .and_then(|ps| int(ps, "SegmentMapStorageID"))
            .and_then(|sm| sm_by_id.get(&sm))
            .and_then(|sm| int(sm, "RecordCount"))
            .and_then(|n| u64::try_from(n).ok())
            .unwrap_or(0);
        let e = rows_of_table.entry(table).or_insert(0);
        *e = e.saturating_add(records);
    }

    let mut out = HashMap::new();
    let mut table_rows = HashMap::new();
    for c in &columns {
        // Type 3: the row-number column
        if int(c, "Type") == Some(3) {
            continue;
        }
        let Some(table_id) = int(c, "TableID") else {
            continue;
        };
        let Some(table) = tables_by_id.get(&table_id) else {
            continue;
        };
        // system tables (H$/R$/U$ storage)
        if int(table, "SystemFlags").unwrap_or(0) & 1 != 0 {
            continue;
        }
        let Some(table_name) = text(table, "Name") else {
            continue;
        };
        let Some(name) = text(c, "ExplicitName").or_else(|| text(c, "InferredName")) else {
            continue;
        };
        let logical = int(c, "ExplicitDataType")
            .filter(|t| *t != 1)
            .or_else(|| int(c, "InferredDataType"))
            .and_then(LogicalType::from_code);
        let Some(cs_id) = int(c, "ColumnStorageID") else {
            continue;
        };
        let Some(cs) = cs_by_id.get(&cs_id) else {
            continue;
        };
        let Some(ds) = int(cs, "DictionaryStorageID").and_then(|d| ds_by_id.get(&d)) else {
            continue;
        };
        let encoding = ColumnEncoding::from_dictionary_storage(
            int(ds, "Type").unwrap_or(-1),
            int(ds, "DataType").unwrap_or(-1),
            int(ds, "BaseId").unwrap_or(0),
            real(ds, "Magnitude").unwrap_or(1.0),
        );
        let Ok(encoding) = encoding else {
            continue;
        };
        let encoding = encoding.with_logical_type(logical);
        let dictionary = int(ds, "StorageFileID").and_then(file_range);
        let mut parts = parts_of.remove(&cs_id).unwrap_or_default();
        parts.sort_by_key(|(order, _, _)| *order);
        let parts: Option<Vec<(Range<usize>, Range<usize>)>> = parts
            .into_iter()
            .map(|(_, idf, meta)| Some((file_range(idf)?, file_range(meta)?)))
            .collect();
        let Some(parts) = parts else {
            continue;
        };
        let tkey = table_name.to_lowercase();
        table_rows.insert(tkey.clone(), rows_of_table.get(&table_id).copied().unwrap_or(0));
        out.insert(
            (tkey, name.to_lowercase()),
            ColumnLoc {
                parts,
                dictionary,
                encoding,
            },
        );
    }
    Ok((out, table_rows))
}

/// A table loaded from literal partitions.
#[derive(Debug)]
pub(crate) struct LiteralTable {
    pub rows: usize,
    /// Lower-cased column name → data.
    pub columns: HashMap<String, Arc<ColData>>,
}

/// Fingerprint of what [`load_literal`] reads, to know when to reload.
pub(crate) fn literal_key(t: &Table) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.name.hash(&mut h);
    for p in &t.partitions {
        p.source_type.hash(&mut h);
        p.expression.hash(&mut h);
    }
    for c in &t.columns {
        c.name.hash(&mut h);
        c.source_column.hash(&mut h);
        c.expression.hash(&mut h);
        format!("{:?}", c.data_type).hash(&mut h);
    }
    h.finish()
}

/// Loads a table from its partitions, all concatenated: M queries through bicraft's Power Query
/// evaluator (the model's culture and shared queries; files only under `root`), DAX literal
/// tables. Text comes out as Analysis Services stores it (`crate::datasrc::stored_values`).
pub(crate) fn load_literal(
    t: &Table,
    model: &SemanticModel,
    root: Option<&Path>,
    max_rows: usize,
) -> Result<LiteralTable, Miss> {
    if t.partitions.is_empty() {
        return Err(Miss::Unavailable(format!(
            "table '{}' has no partition to load",
            t.name
        )));
    }
    let limits = bicraft_data::Limits {
        max_rows,
        ..bicraft_data::Limits::default()
    };
    let mut merged: Vec<(String, Vec<CellValue>)> = Vec::new();
    let mut rows = 0usize;
    let mut imported = false;
    for p in &t.partitions {
        let expr = p.expression.as_deref().unwrap_or_default();
        imported |= p.source_type.eq_ignore_ascii_case("m");
        let opts = crate::datasrc::source_options(Some(model), &t.name, expr, root, limits);
        let data = bicraft_data::load_partition_opts(p, &t.columns, &opts).map_err(|e| {
            let text = crate::datasrc::describe(&e, root);
            match e {
                bicraft_data::Error::UnsupportedSource(_) | bicraft_data::Error::ExternalSource(_) => {
                    Miss::Unavailable(format!("table '{}': {text}", t.name))
                }
                _ => Miss::Error(format!("table '{}': {text}", t.name)),
            }
        })?;
        rows = rows
            .checked_add(data.row_count())
            .filter(|r| *r <= max_rows)
            .ok_or_else(|| Miss::Error(format!("table '{}' has more than {max_rows} rows", t.name)))?;
        for c in data.into_columns() {
            match merged.iter_mut().find(|(n, _)| *n == c.name) {
                Some((_, values)) => values.extend(c.values),
                None => merged.push((c.name, c.values)),
            }
        }
    }
    let mut columns = HashMap::new();
    for (name, values) in merged {
        if values.len() != rows {
            return Err(Miss::Error(format!(
                "table '{}': partitions disagree on column '{name}'",
                t.name
            )));
        }
        let values = crate::datasrc::stored_values(model, imported, &values);
        columns.insert(name.to_lowercase(), Arc::new(ColData::from_stored(values)?));
    }
    Ok(LiteralTable { rows, columns })
}

/// A table of `model` by name (exact, then case-insensitive).
pub(crate) fn model_table<'m>(model: &'m SemanticModel, name: &str) -> Option<&'m Table> {
    model
        .tables
        .iter()
        .find(|t| t.name == name)
        .or_else(|| model.tables.iter().find(|t| t.name.eq_ignore_ascii_case(name)))
}

/// Runs `f` on the backup's bytes at `r`: in memory, or read from the temporary file.
fn read_backing<R>(
    abf: &std::sync::RwLock<Backing>,
    r: &Range<usize>,
    f: impl FnOnce(&[u8]) -> Result<R, Miss>,
) -> Result<R, Miss> {
    let outside = || Miss::Error("column file outside the backup".into());
    let file = match &*abf.read().unwrap_or_else(std::sync::PoisonError::into_inner) {
        Backing::Memory(b) => return f(b.get(r.clone()).ok_or_else(outside)?),
        Backing::File(file) => Arc::clone(file),
    };
    if r.start > r.end || r.end > file.len {
        return Err(outside());
    }
    let mut buf: Vec<u8> = Vec::new();
    buf.try_reserve_exact(r.end - r.start)
        .map_err(|_| Miss::Error("not enough memory to read a column".into()))?;
    buf.resize(r.end - r.start, 0);
    file.read_at(r.start as u64, &mut buf)
        .map_err(|e| Miss::Error(format!("cannot read the backup's temporary file: {e}")))?;
    f(&buf)
}

/// The column's values in code order (code 0 blank) and the code of each data id, from its
/// distinct data ids in first-seen order: (values, data id → code, some row is blank, the values
/// as an affine function of the ids).
#[allow(clippy::type_complexity)]
fn codes_of(
    distinct: &[u32],
    dictionary: Option<&bicraft_vertipaq::Dictionary>,
    encoding: ColumnEncoding,
) -> Result<(Vec<CellValue>, Vec<u32>, bool, Option<(i64, i64)>), Miss> {
    let decoded = ids_to_values(distinct, dictionary, encoding).map_err(|e| Miss::Error(e.to_string()))?;
    let mut values = Vec::with_capacity(decoded.len().saturating_add(1));
    values.push(CellValue::Null);
    values.extend(decoded);
    let max_id = distinct.iter().copied().max().unwrap_or(0) as usize;
    let mut lut = vec![0u32; max_id.saturating_add(1)];
    for (k, id) in distinct.iter().enumerate() {
        let code =
            u32::try_from(k.saturating_add(1)).map_err(|_| Miss::Error("too many distinct values".into()))?;
        // blank ids (and values that decode to blank) all use code 0
        let blank = matches!(values.get(code as usize), Some(CellValue::Null) | None);
        if let Some(slot) = lut.get_mut(*id as usize) {
            *slot = if blank { 0 } else { code };
        }
    }
    let blank = distinct
        .iter()
        .any(|id| lut.get(*id as usize).copied().unwrap_or(0) == 0);
    let affine = affine_values(distinct, &values);
    Ok((values, lut, blank, affine))
}

/// How a streamed decompression runs under `pool`'s limit (lot 5), with the bytes of its buffers:
/// `Some((threads, piece))` for the parallel decoder (the defaults without a limit; under one, as
/// many threads as fit in half of what is left, with 1 MiB pieces when 8 MiB do not fit), `None`
/// for the lean reference decoder when not even one thread's buffers fit.
fn stream_plan(pool: &bicraft_mem::Pool) -> (Option<(usize, usize)>, usize) {
    let piece = bicraft_xpress9::STREAM_PIECE;
    let Some(limit) = pool.limit() else {
        return (Some((0, piece)), 0);
    };
    let room = limit.saturating_sub(pool.stats().used) / 2;
    let auto = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    for t in (1..=auto).rev() {
        let b = bicraft_xpress9::stream_buffer_bytes(t, piece);
        if b <= room {
            return (Some((t, piece)), b);
        }
    }
    let small = 1 << 20;
    let b = bicraft_xpress9::stream_buffer_bytes(1, small);
    if b <= room {
        return (Some((1, small)), b);
    }
    (None, bicraft_xpress9::LEAN_BUFFER_BYTES)
}
