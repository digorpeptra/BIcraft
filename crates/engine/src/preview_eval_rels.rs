//! Relationship indexes kept by the session between evaluations (`docs/storage-engine.md`,
//! "Relationship indexes").
//!
//! A relationship's code map (each code of one end → the code of the other end holding the same
//! key) costs a hash of every key of the dimension: O(dimension rows). Analysis Services builds
//! its relationship indexes when it processes the data, and `ClearCache` keeps them. The session
//! keeps them the same way: built on first use, reused by every later query, warm or cold
//! (`dax.clear_cache` keeps them, as it keeps the decoded columns), and dropped whenever the
//! model or the data changes.
//!
//! Invalidation is strict, on two levels:
//! - the whole set is dropped when the session's model hash or data generation changes (a model
//!   edit, a relationship edit, a refresh, another data source);
//! - each entry is also checked against what it was computed from: the two decoded columns (by
//!   identity: the entry holds them, so their address cannot be reused by other data) and the
//!   culture whose text identity matches the keys. Data not shared through the session (a
//!   calculation group's rows, a query's own columns) never matches an entry.

use std::collections::HashMap;
use std::sync::Arc;

use bicraft_vertipaq::CellValue;

use super::super::store::ColData;
use super::{Engine, Fail, Key, NONE};

/// Most entries kept (an entry is a few small vectors; the bound only stops pathological growth,
/// such as data rebuilt by every query).
const MAX_ENTRIES: usize = 4_096;

/// The relationship code maps and blank-key flags of one (model, data).
#[derive(Default)]
pub(in crate::preview) struct RelIndexes {
    /// The model hash and data generation they are for.
    key: Option<(u64, u64)>,
    /// (source column, target column) addresses → the code map from source to target.
    maps: HashMap<(usize, usize), MapEntry>,
    /// Column address → some row holds the blank code.
    blank_codes: HashMap<usize, (Arc<ColData>, bool)>,
    /// Tests: code maps built.
    #[cfg(test)]
    pub(in crate::preview) builds: u64,
}

struct MapEntry {
    src: Arc<ColData>,
    dst: Arc<ColData>,
    culture: Option<String>,
    map: Arc<Vec<u32>>,
    /// Some non-blank source code maps to no target code.
    unmatched: bool,
}

/// A code map with what [`Engine::has_blank_row`] reads from it.
pub(super) struct RelMap {
    pub(super) map: Arc<Vec<u32>>,
    pub(super) unmatched: bool,
}

fn addr(c: &Arc<ColData>) -> usize {
    Arc::as_ptr(c) as usize
}

impl RelIndexes {
    /// Drops everything (the data they were built from changed).
    pub(in crate::preview) fn clear(&mut self) {
        self.key = None;
        self.maps.clear();
        self.blank_codes.clear();
    }

    /// References the entries hold to the column at `addr` (a column the memory budget may
    /// evict: these entries are dropped with it).
    pub(in crate::preview) fn refs(&self, at: usize) -> usize {
        let maps = self
            .maps
            .values()
            .map(|e| usize::from(addr(&e.src) == at) + usize::from(addr(&e.dst) == at))
            .sum::<usize>();
        maps + usize::from(self.blank_codes.get(&at).is_some_and(|(c, _)| addr(c) == at))
    }

    /// Drops the entries built from the column at `addr` (evicted: a column read again is another
    /// value, and the entries would keep the evicted one alive).
    pub(in crate::preview) fn forget(&mut self, at: usize) {
        self.maps.retain(|_, e| addr(&e.src) != at && addr(&e.dst) != at);
        self.blank_codes.remove(&at);
    }

    /// Drops everything unless it is for this model and data.
    fn sync(&mut self, key: (u64, u64)) {
        if self.key != Some(key) {
            self.key = Some(key);
            self.maps.clear();
            self.blank_codes.clear();
        }
    }

    fn map(&self, src: &Arc<ColData>, dst: &Arc<ColData>, culture: Option<&str>) -> Option<RelMap> {
        let e = self.maps.get(&(addr(src), addr(dst)))?;
        (Arc::ptr_eq(&e.src, src) && Arc::ptr_eq(&e.dst, dst) && e.culture.as_deref() == culture).then(|| {
            RelMap {
                map: Arc::clone(&e.map),
                unmatched: e.unmatched,
            }
        })
    }

    fn keep_map(&mut self, src: &Arc<ColData>, dst: &Arc<ColData>, culture: Option<&str>, m: &RelMap) {
        if self.maps.len() >= MAX_ENTRIES {
            self.maps.clear();
        }
        self.maps.insert(
            (addr(src), addr(dst)),
            MapEntry {
                src: Arc::clone(src),
                dst: Arc::clone(dst),
                culture: culture.map(str::to_string),
                map: Arc::clone(&m.map),
                unmatched: m.unmatched,
            },
        );
    }

    fn blank_code(&self, d: &Arc<ColData>) -> Option<bool> {
        let (kept, b) = self.blank_codes.get(&addr(d))?;
        Arc::ptr_eq(kept, d).then_some(*b)
    }

    fn keep_blank_code(&mut self, d: &Arc<ColData>, b: bool) {
        if self.blank_codes.len() >= MAX_ENTRIES {
            self.blank_codes.clear();
        }
        self.blank_codes.insert(addr(d), (Arc::clone(d), b));
    }
}

/// Builds the code map from `src`'s codes to `dst`'s codes (keys matched as Analysis Services
/// stores them, not case-insensitively).
fn build(src: &ColData, dst: &ColData, keys: &bicraft_vertipaq::system::TextStorage) -> RelMap {
    if let Some(m) = build_ints(src, dst) {
        return m;
    }
    let mut index: HashMap<Key, u32> = HashMap::with_capacity(dst.values.len());
    for (code, v) in dst.values.iter().enumerate().skip(1) {
        if let (Some(k), Ok(code)) = (Key::related(v, keys), u32::try_from(code)) {
            index.entry(k).or_insert(code);
        }
    }
    let map: Vec<u32> = src
        .values
        .iter()
        .map(|v| {
            Key::related(v, keys)
                .and_then(|k| index.get(&k).copied())
                .unwrap_or(NONE)
        })
        .collect();
    let unmatched = map.iter().skip(1).any(|m| *m == NONE);
    RelMap {
        map: Arc::new(map),
        unmatched,
    }
}

/// [`build`] for whole-number keys in a dense range (surrogate keys): a table indexed by key
/// instead of a hash of every key. Keys within ±2^53 only, where two keys are the same exactly
/// when their [`Key`]s are (`Key::of` keys numbers by their double). `None`: other keys.
fn build_ints(src: &ColData, dst: &ColData) -> Option<RelMap> {
    const EXACT: u64 = 1 << 53;
    let ints = |d: &ColData| {
        d.values.iter().skip(1).all(|v| match v {
            CellValue::Null => true,
            CellValue::Int(n) => n.unsigned_abs() <= EXACT,
            _ => false,
        })
    };
    if !ints(src) || !ints(dst) {
        return None;
    }
    let (mut lo, mut hi) = (i64::MAX, i64::MIN);
    for v in dst.values.iter().skip(1) {
        if let CellValue::Int(n) = v {
            lo = lo.min(*n);
            hi = hi.max(*n);
        }
    }
    let span = if lo <= hi {
        usize::try_from(hi.checked_sub(lo)?.checked_add(1)?).ok()?
    } else {
        0
    };
    if span > dst.values.len().saturating_mul(4).max(1 << 20) {
        return None;
    }
    let mut table = vec![NONE; span];
    for (code, v) in dst.values.iter().enumerate().skip(1) {
        if let CellValue::Int(n) = v
            && let Ok(code) = u32::try_from(code)
            && let Some(slot) = n
                .checked_sub(lo)
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| table.get_mut(i))
            && *slot == NONE
        {
            *slot = code;
        }
    }
    let map: Vec<u32> = src
        .values
        .iter()
        .map(|v| match v {
            CellValue::Int(n) => n
                .checked_sub(lo)
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| table.get(i).copied())
                .unwrap_or(NONE),
            _ => NONE,
        })
        .collect();
    let unmatched = map.iter().skip(1).any(|m| *m == NONE);
    Some(RelMap {
        map: Arc::new(map),
        unmatched,
    })
}

impl Engine<'_, '_> {
    /// The session's relationship indexes, synchronized with this model and data once per
    /// evaluation.
    fn rel_indexes(&mut self) -> &mut RelIndexes {
        if !self.rels_synced {
            self.rels_synced = true;
            let s = self.ctx.s;
            let st = &mut *self.ctx.st;
            let key = (st.model_hash(s), st.generation);
            st.rel_indexes.sync(key);
            #[cfg(test)]
            if !KEEP.with(std::cell::Cell::get) {
                st.rel_indexes.clear();
                st.rel_indexes.sync(key);
            }
        }
        &mut self.ctx.st.rel_indexes
    }

    /// The code map from `src`'s codes to `dst`'s codes, built once per (model, data).
    pub(super) fn rel_map_of(&mut self, src: &Arc<ColData>, dst: &Arc<ColData>) -> RelMap {
        let culture = self.model.culture.clone();
        if let Some(m) = self.rel_indexes().map(src, dst, culture.as_deref()) {
            return m;
        }
        let keys = crate::datasrc::relationship_keys(self.model);
        let m = build(src, dst, &keys);
        let idx = self.rel_indexes();
        #[cfg(test)]
        {
            idx.builds = idx.builds.saturating_add(1);
        }
        idx.keep_map(src, dst, culture.as_deref(), &m);
        m
    }

    /// Some row of `d` holds the blank code, computed once per (model, data).
    pub(super) fn blank_code_kept(&mut self, d: &Arc<ColData>) -> Result<bool, Fail> {
        if let Some(b) = self.rel_indexes().blank_code(d) {
            return Ok(b);
        }
        let b = if self.store.is_some() {
            self.has_blank_code(d)?
        } else {
            d.codes.contains(&0)
        };
        self.rel_indexes().keep_blank_code(d, b);
        Ok(b)
    }
}

// Test switch: `false` drops the relationship indexes at the start of every evaluation (the
// micro-bench's "rebuilt by every query" column).
#[cfg(test)]
thread_local! {
    static KEEP: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

#[cfg(test)]
pub(super) fn set_keep(keep: bool) {
    KEEP.with(|k| k.set(keep));
}

/// What an evaluation reads from every measure's text, kept by the session for its model.
#[derive(Clone, Copy)]
pub(in crate::preview) struct MeasureFacts {
    /// The model hash they are for.
    model: u64,
    /// Fingerprint of the measures (part of the keys of results kept by expression text).
    pub(super) fingerprint: u64,
    /// Some measure uses `ALLSELECTED`.
    pub(super) allselected: bool,
}

fn measure_facts_of(model: &bicraft_model::SemanticModel, hash: u64) -> MeasureFacts {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut allselected = false;
    for t in &model.tables {
        for m in &t.measures {
            std::hash::Hash::hash(&(&t.name, &m.name, &m.expression), &mut h);
            allselected = allselected || m.expression.to_ascii_uppercase().contains("ALLSELECTED");
        }
    }
    MeasureFacts {
        model: hash,
        fingerprint: std::hash::Hasher::finish(&h),
        allselected,
    }
}

impl Engine<'_, '_> {
    /// The facts of this evaluation's measures: the session's, computed once per model, when
    /// the evaluation runs on the session's model itself (no query measure).
    pub(super) fn measure_facts(&mut self) -> MeasureFacts {
        let s = self.ctx.s;
        let own = s.model.as_ref().is_some_and(|m| std::ptr::eq(m, self.model));
        if !own {
            return measure_facts_of(self.model, 0);
        }
        let st = &mut *self.ctx.st;
        let hash = st.model_hash(s);
        if let Some(f) = st.measure_facts
            && f.model == hash
        {
            return f;
        }
        let f = measure_facts_of(self.model, hash);
        st.measure_facts = Some(f);
        f
    }
}

#[cfg(test)]
mod int_keys {
    use super::*;

    fn col(values: Vec<CellValue>) -> ColData {
        ColData::from_stored(values).unwrap()
    }

    fn hashed(src: &ColData, dst: &ColData) -> (Vec<u32>, bool) {
        let keys = bicraft_vertipaq::system::TextStorage::exact();
        let mut index: HashMap<Key, u32> = HashMap::new();
        for (code, v) in dst.values.iter().enumerate().skip(1) {
            if let (Some(k), Ok(code)) = (Key::related(v, &keys), u32::try_from(code)) {
                index.entry(k).or_insert(code);
            }
        }
        let map: Vec<u32> = src
            .values
            .iter()
            .map(|v| {
                Key::related(v, &keys)
                    .and_then(|k| index.get(&k).copied())
                    .unwrap_or(NONE)
            })
            .collect();
        let unmatched = map.iter().skip(1).any(|m| *m == NONE);
        (map, unmatched)
    }

    /// The table of whole-number keys maps every code as the hash of keys does: unknown keys,
    /// blanks, negative keys, keys out of the dimension's range; other keys take the hash.
    #[test]
    fn whole_number_keys_map_as_the_hash() {
        let int = |n: i64| CellValue::Int(n);
        let dst = col((1..=500)
            .rev()
            .map(int)
            .chain([int(-3), CellValue::Null, int(0)])
            .collect());
        let src = col([
            int(7),
            int(-3),
            CellValue::Null,
            int(501),
            int(0),
            int(-9),
            int(250),
            int(7),
        ]
        .into());
        let fast = build_ints(&src, &dst).unwrap();
        assert_eq!((fast.map.as_ref().clone(), fast.unmatched), hashed(&src, &dst));
        // all matched
        let src2 = col(vec![int(3), int(4), CellValue::Null]);
        let fast = build_ints(&src2, &dst).unwrap();
        assert_eq!((fast.map.as_ref().clone(), fast.unmatched), hashed(&src2, &dst));
        assert!(!fast.unmatched);
        // not whole numbers, or too sparse: the hash
        let text = col(vec![CellValue::Text("a".into())]);
        assert!(build_ints(&text, &dst).is_none());
        let sparse = col(vec![int(1), int(1 << 40)]);
        assert!(build_ints(&src, &sparse).is_none());
        let huge = col(vec![int(i64::MAX)]);
        assert!(build_ints(&huge, &dst).is_none());
    }
}
