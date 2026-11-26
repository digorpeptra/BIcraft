//! Large synthetic models written by the Analysis Services engine (local only, G10).
//!
//! The maintainers' development oracles deploy our own synthetic models (1M–20M rows,
//! multi-segment and multi-partition tables, 200 tables, high-cardinality strings, every data type, blanks,
//! relationships, hierarchies, calculated columns and tables) to the engine, save them
//! with AMO `ImageSave`, record the engine's own aggregates of every column (DAX) and extract
//! the column files. Layout of `$BICRAFT_TESTDATA/large/<case>/`:
//!
//! - `manifest.json`: the `metadata.sqlitedb` rows of every table and column (`Table`,
//!   `Partition`, `PartitionStorage`, `SegmentMapStorage`, `Column`, `ColumnStorage`,
//!   `DictionaryStorage`, `ColumnPartitionStorage`, `AttributeHierarchyStorage`, plus
//!   relationship and hierarchy rows), with the name of each extracted file;
//! - `columns/`: the `.idf`, `.idfmeta`, `.dictionary` and `.hidx` files;
//! - `expected.json`: per user table and column, the engine's `COUNTROWS`, blank count,
//!   `DISTINCTCOUNT`, `MIN`, `MAX`, sum, string length sum, and a key-weighted checksum
//!   (`SUMX(T, MOD(T[key], 1009) * f(T[c]))`, which ties every value to its row).
//!
//! The tests decode every column (user and system) and compare with the engine, re-encode the
//! user columns and rebuild the `H$`/`R$`/`U$` system tables and compare them byte for byte.
//! The data are ours, but the files are large: they stay outside the repository. Without
//! `BICRAFT_TESTDATA` (or without `large/`), the tests print a note and pass. They are slow, so
//! they are opt-in: set `BICRAFT_LARGE=1` and run them in release:
//! `BICRAFT_LARGE=1 cargo test --release -p bicraft-vertipaq --test large -- --nocapture`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use bicraft_vertipaq::idfmeta::ColumnMeta;
use bicraft_vertipaq::system::{
    AttributeHierarchyOptions, UserHierarchyLevel, build_attribute_hierarchy_with, build_user_hierarchy,
};
use bicraft_vertipaq::{
    CellValue, Collation, ColumnEncoding, Dictionary, DictionaryValues, EncodeOptions, EncodingKind,
    EncodingRequest, Limits, LogicalType, ValueParams, build_relationship_index, decode_column_with_limits,
    decode_data_ids, encode_column, ids_to_values, parse_dictionary, parse_hash_index, parse_idfmeta,
};
use serde_json::Value;

/// Generous limits: the largest case has 20M rows (more than the 2^24 default).
const LIMITS: Limits = Limits {
    max_rows: 1 << 26,
    max_bytes: 1 << 34,
};

fn cases() -> Vec<PathBuf> {
    if bicraft_env::var_os("BICRAFT_LARGE").is_none_or(|v| v != "1") {
        eprintln!("BICRAFT_LARGE is not 1: skipping the large-model tests (opt-in, run in release)");
        return Vec::new();
    }
    let Some(root) = bicraft_env::var_os("BICRAFT_TESTDATA").map(PathBuf::from) else {
        eprintln!("BICRAFT_TESTDATA is not set: skipping the large-model tests");
        return Vec::new();
    };
    let large = root.join("large");
    let mut out: Vec<PathBuf> = std::fs::read_dir(&large)
        .map(|d| {
            d.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.join("manifest.json").is_file())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    if let Ok(only) = bicraft_env::var("BICRAFT_LARGE_CASES") {
        out.retain(|p| only.split(',').any(|c| p.file_name().is_some_and(|n| n == c)));
    }
    if out.is_empty() {
        eprintln!("no large-model case under {}: skipping", large.display());
    }
    out
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn int(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(Value::as_i64)
}

fn text<'v>(v: &'v Value, key: &str) -> Option<&'v str> {
    v.get(key).and_then(Value::as_str)
}

// ---------------------------------------------------------------------------------------------
// The model as the manifest describes it

struct Table {
    name: String,
    system: bool,
    /// `(RecordCount, SegmentCount, RecordsPerSegment, segment map Type)` per partition, in
    /// storage position order.
    partitions: Vec<(u64, u64, u64, i64)>,
}

struct Column<'m> {
    j: &'m Value,
    id: i64,
    table_id: i64,
    name: String,
    row_number: bool,
    logical: Option<LogicalType>,
    encoding: ColumnEncoding,
    /// `(.idf, .idfmeta)` per partition, in storage position order.
    parts: Vec<(PathBuf, PathBuf)>,
    dictionary: Option<PathBuf>,
    hidx: Option<PathBuf>,
}

struct Model<'m> {
    manifest: &'m Value,
    tables: BTreeMap<i64, Table>,
    columns: Vec<Column<'m>>,
}

impl<'m> Model<'m> {
    fn load(dir: &Path, manifest: &'m Value) -> Model<'m> {
        let files = dir.join("columns");
        let file = |v: &Value| text(v, "file").map(|f| files.join(f));
        let mut tables = BTreeMap::new();
        for t in manifest["tables"].as_array().into_iter().flatten() {
            let row = &t["table"];
            let mut parts: Vec<(i64, (u64, u64, u64, i64))> = t["partitions"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| {
                    let m = &p["segment_map"];
                    let n = |k| int(m, k).unwrap_or(0).max(0) as u64;
                    (
                        int(&p["storage"], "StoragePosition").unwrap_or(0),
                        (
                            n("RecordCount"),
                            n("SegmentCount"),
                            n("RecordsPerSegment"),
                            int(m, "Type").unwrap_or(-1),
                        ),
                    )
                })
                .collect();
            parts.sort_by_key(|p| p.0);
            tables.insert(
                int(row, "ID").unwrap_or(0),
                Table {
                    name: text(row, "Name").unwrap_or("").to_string(),
                    system: int(row, "SystemFlags").unwrap_or(0) & 1 != 0,
                    partitions: parts.into_iter().map(|p| p.1).collect(),
                },
            );
        }
        let mut columns = Vec::new();
        for c in manifest["columns"].as_array().into_iter().flatten() {
            let row = &c["column"];
            let Some(ds) = c.get("dictionary_storage").filter(|d| !d.is_null()) else {
                continue;
            };
            let logical = int(row, "ExplicitDataType")
                .filter(|t| *t != 1)
                .or_else(|| int(row, "InferredDataType"))
                .and_then(LogicalType::from_code);
            let encoding = ColumnEncoding::from_dictionary_storage(
                int(ds, "Type").unwrap_or(-1),
                int(ds, "DataType").unwrap_or(-1),
                int(ds, "BaseId").unwrap_or(0),
                ds.get("Magnitude").and_then(Value::as_f64).unwrap_or(1.0),
            )
            .unwrap_or_else(|e| panic!("column {}: {e}", row["ID"]))
            .with_logical_type(logical);
            let parts: Vec<(PathBuf, PathBuf)> = c["parts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| Some((file(&p["idf"])?, file(&p["idfmeta"])?)))
                .collect();
            if parts.is_empty() {
                continue;
            }
            columns.push(Column {
                j: c,
                id: int(row, "ID").unwrap_or(0),
                table_id: int(row, "TableID").unwrap_or(0),
                name: text(row, "ExplicitName")
                    .or_else(|| text(row, "InferredName"))
                    .unwrap_or("")
                    .to_string(),
                row_number: int(row, "Type") == Some(3),
                logical,
                encoding,
                parts,
                dictionary: c.get("dictionary").and_then(file),
                hidx: c.get("hidx").and_then(file),
            });
        }
        Model {
            manifest,
            tables,
            columns,
        }
    }

    fn table(&self, c: &Column<'_>) -> &Table {
        &self.tables[&c.table_id]
    }

    fn column(&self, table_id: i64, name: &str) -> Option<&Column<'m>> {
        self.columns
            .iter()
            .find(|c| c.table_id == table_id && c.name == name)
    }

    fn column_by_id(&self, id: i64) -> Option<&Column<'m>> {
        self.columns.iter().find(|c| c.id == id)
    }

    fn rows(&self, name: &str) -> &'m [Value] {
        self.manifest["rows"][name].as_array().map_or(&[], Vec::as_slice)
    }

    fn collation(&self) -> Collation {
        let culture = self
            .rows("Model")
            .first()
            .and_then(|m| text(m, "Culture"))
            .unwrap_or("en-US");
        Collation::for_culture(culture)
    }
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Data ids of every row (partitions concatenated) and each partition's parsed `.idfmeta`.
fn decode_ids(c: &Column<'_>) -> (Vec<u32>, Vec<ColumnMeta>) {
    let mut ids = Vec::new();
    let mut metas = Vec::new();
    for (idf, meta) in &c.parts {
        let meta = parse_idfmeta(&read(meta)).unwrap_or_else(|e| panic!("column {} {}: {e}", c.id, c.name));
        let part = decode_data_ids(&read(idf), &meta, &LIMITS)
            .unwrap_or_else(|e| panic!("column {} {}: {e}", c.id, c.name));
        ids.extend(part);
        metas.push(meta);
    }
    (ids, metas)
}

fn dictionary(c: &Column<'_>) -> Option<Dictionary> {
    let path = c.dictionary.as_ref()?;
    Some(parse_dictionary(&read(path)).unwrap_or_else(|e| panic!("column {} {}: {e}", c.id, c.name)))
}

/// Applies `f` to every row's value, converting ids in chunks.
fn for_each_value(
    ids: &[u32],
    dict: Option<&Dictionary>,
    enc: ColumnEncoding,
    mut f: impl FnMut(usize, &CellValue),
) {
    let mut row = 0usize;
    for chunk in ids.chunks(1 << 16) {
        let values = ids_to_values(chunk, dict, enc).unwrap_or_else(|e| panic!("ids_to_values: {e}"));
        for v in &values {
            f(row, v);
            row += 1;
        }
    }
}

fn values(ids: &[u32], dict: Option<&Dictionary>, enc: ColumnEncoding) -> Vec<CellValue> {
    let mut out = Vec::with_capacity(ids.len());
    for_each_value(ids, dict, enc, |_, v| out.push(v.clone()));
    out
}

fn number(v: &CellValue) -> Option<f64> {
    match v {
        CellValue::Int(n) => Some(*n as f64),
        CellValue::Real(x) | CellValue::DateTime(x) => Some(*x),
        CellValue::Decimal(n) => Some(*n as f64 / 10_000.0),
        CellValue::Bool(b) => Some(f64::from(u8::from(*b))),
        CellValue::Null | CellValue::Text(_) => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Aggregates, computed the way the oracle's DAX computes them

#[derive(Default, Debug)]
struct Agg {
    blank: u64,
    dc: u64,
    min: Option<CellValue>,
    max: Option<CellValue>,
    sum: f64,
    len: u64,
    trues: u64,
    chk: f64,
}

fn aggregate(
    ids: &[u32],
    dict: Option<&Dictionary>,
    enc: ColumnEncoding,
    kind: &str,
    weights: Option<&[i64]>,
    collation: &Collation,
) -> Agg {
    let mut a = Agg::default();
    let distinct: HashSet<u32> = ids.iter().copied().collect();
    a.dc = distinct.len() as u64;
    let less = |x: &CellValue, y: &CellValue| match (x, y) {
        (CellValue::Text(p), CellValue::Text(q)) => collation.compare(p, q).is_lt(),
        _ => number(x) < number(y),
    };
    // Kahan-free sums are fine: the comparison is relative.
    for_each_value(ids, dict, enc, |row, v| {
        let w = weights.map_or(1.0, |w| w[row].rem_euclid(1009) as f64);
        if matches!(v, CellValue::Null) {
            a.blank += 1;
            return;
        }
        if a.min.as_ref().is_none_or(|m| less(v, m)) {
            a.min = Some(v.clone());
        }
        if a.max.as_ref().is_none_or(|m| less(m, v)) {
            a.max = Some(v.clone());
        }
        match (kind, v) {
            ("str", CellValue::Text(s)) => {
                let units: Vec<u16> = s.encode_utf16().collect();
                a.len += units.len() as u64;
                let last = units.last().map_or(0.0, |u| f64::from(*u));
                a.chk += w * (units.len() as f64 + last);
            }
            ("bool", CellValue::Bool(b)) => {
                a.trues += u64::from(*b);
                a.chk += w * f64::from(u8::from(*b));
            }
            ("int", CellValue::Int(n)) => {
                a.sum += *n as f64;
                a.chk += w * n.rem_euclid(1_000_003) as f64;
            }
            (_, v) => {
                let x = number(v).unwrap_or(f64::NAN);
                a.sum += x;
                a.chk += w * x;
            }
        }
    });
    a
}

fn close(ours: f64, engine: f64) -> bool {
    (ours - engine).abs() <= 1e-9 * ours.abs().max(engine.abs()).max(1.0)
}

/// Compares one aggregate; returns a description of the difference.
fn compare_agg(a: &Agg, e: &Value, kind: &str, collation: &Collation) -> Vec<String> {
    let mut diffs = Vec::new();
    let num = |k: &str| e.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    let mut check = |what: &str, ours: f64, engine: f64| {
        if !close(ours, engine) {
            diffs.push(format!("{what}: ours {ours} engine {engine}"));
        }
    };
    check("blank", a.blank as f64, num("blank"));
    check("distinctcount", a.dc as f64, num("dc"));
    if kind != "big" {
        check("checksum", a.chk, num("chk"));
    }
    match kind {
        "str" => {
            check("len", a.len as f64, num("len"));
            for (what, ours) in [("min", &a.min), ("max", &a.max)] {
                let engine = e.get(what).and_then(Value::as_str);
                let ours = match ours {
                    Some(CellValue::Text(s)) => Some(s.as_str()),
                    _ => None,
                };
                let same = match (ours, engine) {
                    (Some(o), Some(e)) => collation.equivalent(o, e),
                    (o, e) => o.is_none() && e.is_none(),
                };
                if !same {
                    diffs.push(format!("{what}: ours {ours:?} engine {engine:?}"));
                }
            }
        }
        "bool" => check("true", a.trues as f64, num("true")),
        // int64 near ±2^62: sums cancel in floating point; extremes only
        "big" => {
            for (what, ours) in [("min", &a.min), ("max", &a.max)] {
                check(what, ours.as_ref().and_then(number).unwrap_or(0.0), num(what));
            }
        }
        _ => {
            check("sum", a.sum, num("sum"));
            for (what, ours) in [("min", &a.min), ("max", &a.max)] {
                check(what, ours.as_ref().and_then(number).unwrap_or(0.0), num(what));
            }
        }
    }
    diffs
}

/// Facts about the string pages of a dictionary: how full the pages are, and how the engine's
/// Huffman pages compare with the encoder's choices.
fn page_facts(d: &Dictionary, note: &mut impl FnMut(String)) {
    use bicraft_vertipaq::dictionary::PageStore;
    let (Some(store), DictionaryValues::Text(strings)) = (&d.string_store, &d.values) else {
        return;
    };
    let units = |s: &String| s.encode_utf16().count() as u64 + 1;
    let pages = store.pages.len();
    for (k, p) in store.pages.iter().enumerate() {
        let from = p.start_index as usize;
        let to = from + p.string_count as usize;
        let used: u64 = strings[from..to].iter().map(units).sum();
        if k + 1 < pages {
            // a full page: what it holds, and what one more string would make it
            let next = strings.get(to).map_or(0, units);
            note(format!(
                "full page: holds < 2^19 units: {}, one more string reaches 2^19: {}",
                used < 1 << 19,
                used + next >= 1 << 19
            ));
        }
        if let PageStore::Compressed {
            total_bits,
            charset,
            decode_bits,
            encode_array,
            allocation_bytes,
            buffer_bytes,
            ..
        } = &p.store
        {
            let lengths: Vec<u8> = encode_array.iter().flat_map(|b| [b & 0x0F, b >> 4]).collect();
            let longest = lengths.iter().copied().max().unwrap_or(0);
            note(format!(
                "compressed page: decode_bits = longest code: {}",
                u32::from(longest) == *decode_bits
            ));
            note(format!(
                "compressed page: buffer = 4-byte words of the bits: {}, allocation = buffer: {}",
                *buffer_bytes == u64::from(*total_bits).div_ceil(32) * 4,
                allocation_bytes == buffer_bytes
            ));
            let mut freqs = [0u64; 256];
            for s in &strings[from..to] {
                for u in s.encode_utf16() {
                    if charset.is_some() {
                        freqs[usize::from(u & 0xFF)] += 1;
                    } else {
                        for b in u.to_le_bytes() {
                            freqs[usize::from(b)] += 1;
                        }
                    }
                }
            }
            let ours = bicraft_vertipaq::encode::limited_code_lengths(&freqs);
            note(format!(
                "compressed page: code lengths = encoder's: {}",
                ours[..] == lengths[..]
            ));
        }
    }
}

// ---------------------------------------------------------------------------------------------

/// Decodes every column (user and system), checks the segment structure against the metadata
/// rows and the values against the engine's aggregates.
#[test]
fn large_models_decode_like_the_engine() {
    for dir in cases() {
        let manifest = read_json(&dir.join("manifest.json"));
        let expected = dir.join("expected.json");
        let expected = expected.is_file().then(|| read_json(&expected));
        let model = Model::load(&dir, &manifest);
        let collation = model.collation();
        let case = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let started = Instant::now();
        let mut observed: BTreeMap<String, u64> = BTreeMap::new();
        let mut note = |k: String| *observed.entry(k).or_insert(0) += 1;
        let (mut cells, mut cols) = (0u64, 0u64);
        let mut decoded_ids: HashMap<i64, Vec<u32>> = HashMap::new();

        for c in &model.columns {
            let table = model.table(c);
            let (ids, metas) = decode_ids(c);
            cols += 1;
            cells += ids.len() as u64;
            // rows per partition = the segment map; segment sizes = RecordsPerSegment
            assert_eq!(
                metas.len(),
                table.partitions.len(),
                "{case}: {} {}: partitions",
                table.name,
                c.name
            );
            for (meta, (records, seg_count, per_seg, map_type)) in metas.iter().zip(&table.partitions) {
                let rows: u64 = meta.segments.iter().map(|s| s.records).sum();
                if table.system {
                    note(format!("system map type {map_type}"));
                } else {
                    assert_eq!(rows, *records, "{case}: {} {}: rows", table.name, c.name);
                    assert_eq!(
                        meta.segments.len() as u64,
                        *seg_count,
                        "{case}: {} {}: segments",
                        table.name,
                        c.name
                    );
                }
                for (i, s) in meta.segments.iter().enumerate() {
                    if i + 1 < meta.segments.len() {
                        assert_eq!(
                            s.records, *per_seg,
                            "{case}: {} {}: segment {i} size",
                            table.name, c.name
                        );
                    }
                    note(format!(
                        "{} class {}/{} base_id {}",
                        if table.system { "system" } else { "user" },
                        s.compression_class,
                        s.sub_compression_class,
                        s.base_id
                    ));
                }
                if meta.segments.len() > 1 && !table.system {
                    note(format!("multi-segment partition: {per_seg} rows per segment"));
                }
            }
            if !table.system && !c.row_number {
                // segment statistics against the decoded ids
                let mut at = 0usize;
                for meta in &metas {
                    for s in &meta.segments {
                        let seg = &ids[at..at + s.records as usize];
                        at += s.records as usize;
                        let non_blank = seg.iter().copied().filter(|&i| i != 2);
                        let (mn, mx) = non_blank.fold((u32::MAX, 0), |(a, b), i| (a.min(i), b.max(i)));
                        let nulls = seg.contains(&2);
                        if mn <= mx {
                            assert_eq!(
                                (s.stats.min_data_id, s.stats.max_data_id),
                                (mn, mx),
                                "{case}: {} {}",
                                table.name,
                                c.name
                            );
                        }
                        assert_eq!(
                            s.stats.has_nulls != 0,
                            nulls,
                            "{case}: {} {}: has_nulls",
                            table.name,
                            c.name
                        );
                    }
                }
            }
            let dict = dictionary(c);
            if let Some(d) = &dict {
                note(format!(
                    "dictionary type {} element {:?}",
                    d.type_tag, d.element_size
                ));
                if let Some(store) = &d.string_store {
                    note(format!(
                        "string dictionary pages {}",
                        if store.pages.len() > 1 { "several" } else { "one" }
                    ));
                    for p in &store.pages {
                        match &p.store {
                            bicraft_vertipaq::dictionary::PageStore::Compressed {
                                charset_mode,
                                decode_bits,
                                ..
                            } => note(format!(
                                "compressed page mode {charset_mode} decode_bits {decode_bits}"
                            )),
                            bicraft_vertipaq::dictionary::PageStore::Uncompressed { .. } => {
                                note("uncompressed page".into())
                            }
                        }
                    }
                    page_facts(d, &mut note);
                }
            }
            if let Some(h) = &c.hidx {
                let h = parse_hash_index(&read(h))
                    .unwrap_or_else(|e| panic!("{case}: {} {}: {e}", table.name, c.name));
                note(format!(
                    "hidx bins {} overflow {}",
                    h.bin_count,
                    !h.overflow.is_empty()
                ));
            }
            // the public whole-column API on single-partition columns
            if c.parts.len() == 1 && !table.system {
                let (idf, meta) = &c.parts[0];
                let full = decode_column_with_limits(
                    &read(idf),
                    &read(meta),
                    c.dictionary.as_ref().map(|p| read(p)).as_deref(),
                    c.encoding,
                    &LIMITS,
                )
                .unwrap_or_else(|e| panic!("{case}: {} {}: {e}", table.name, c.name));
                assert_eq!(full.len(), ids.len());
            }
            if !table.system {
                decoded_ids.insert(c.id, ids);
            }
        }
        let t_decode = started.elapsed().as_secs_f64();

        // values against the engine
        let mut compared = 0;
        let mut failures = Vec::new();
        if let Some(expected) = &expected {
            // `_queries`: engine-only checks of relationships and hierarchies (development oracles)
            for (tname, spec) in expected["tables"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(t, _)| !t.starts_with('_'))
            {
                let (tid, _) = model
                    .tables
                    .iter()
                    .find(|(_, t)| &t.name == tname && !t.system)
                    .unwrap_or_else(|| panic!("{case}: no table {tname}"));
                let rows: u64 = model.tables[tid].partitions.iter().map(|p| p.0).sum();
                assert_eq!(
                    rows as i64,
                    int(spec, "rows").unwrap_or(-1),
                    "{case}: {tname} rows"
                );
                let key = text(spec, "key").filter(|k| !k.is_empty());
                let weights: Option<Vec<i64>> = key.map(|k| {
                    let c = model
                        .column(*tid, k)
                        .unwrap_or_else(|| panic!("{case}: key {tname}[{k}]"));
                    let mut w = Vec::with_capacity(rows as usize);
                    for_each_value(&decoded_ids[&c.id], dictionary(c).as_ref(), c.encoding, |_, v| {
                        w.push(match v {
                            CellValue::Int(n) => *n,
                            _ => 0,
                        })
                    });
                    w
                });
                for (cname, e) in spec["columns"].as_object().into_iter().flatten() {
                    let c = model
                        .column(*tid, cname)
                        .unwrap_or_else(|| panic!("{case}: no column {tname}[{cname}]"));
                    let kind = text(e, "kind").unwrap_or("");
                    let a = aggregate(
                        &decoded_ids[&c.id],
                        dictionary(c).as_ref(),
                        c.encoding,
                        kind,
                        weights.as_deref(),
                        &collation,
                    );
                    compared += 1;
                    for d in compare_agg(&a, e, kind, &collation) {
                        failures.push(format!("{tname}[{cname}] {d}"));
                    }
                }
            }
        }
        let mut kinds: Vec<String> = observed.iter().map(|(k, n)| format!("{k}: {n}")).collect();
        kinds.truncate(60);
        eprintln!(
            "{case}: {cols} columns, {cells} cells decoded in {t_decode:.2}s; {compared} columns compared with the engine\n  {}",
            kinds.join("\n  ")
        );
        assert!(
            failures.is_empty(),
            "{case}: values differ from the engine:\n{}",
            failures.join("\n")
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Encoder

/// The dictionary's values as the encoder's seed (data-id order).
fn seed(d: &Dictionary, logical: Option<LogicalType>) -> Vec<CellValue> {
    use bicraft_vertipaq::convert;
    match &d.values {
        DictionaryValues::Long(v) => v.iter().map(|&x| convert(CellValue::Int(x), logical)).collect(),
        DictionaryValues::Real(v) => v.iter().map(|&x| convert(CellValue::Real(x), logical)).collect(),
        DictionaryValues::Text(v) => v.iter().cloned().map(CellValue::Text).collect(),
    }
}

/// Where two `.idfmeta` differ, field by field.
fn meta_diff(ours: &[u8], engine: &[u8]) -> String {
    let (Ok(a), Ok(b)) = (parse_idfmeta(ours), parse_idfmeta(engine)) else {
        return "unparsable".into();
    };
    if a.segments.len() != b.segments.len() {
        return format!("{} vs {} segments", a.segments.len(), b.segments.len());
    }
    for (i, (x, y)) in a.segments.iter().zip(&b.segments).enumerate() {
        let mut d = Vec::new();
        if x.records != y.records {
            d.push(format!("records {} vs {}", x.records, y.records));
        }
        if x.base_id != y.base_id {
            d.push(format!("base_id {} vs {}", x.base_id, y.base_id));
        }
        if (x.compression_class, x.sub_compression_class) != (y.compression_class, y.sub_compression_class) {
            d.push(format!(
                "class {}/{} vs {}/{}",
                x.compression_class, x.sub_compression_class, y.compression_class, y.sub_compression_class
            ));
        }
        if x.rle != y.rle {
            d.push(format!("rle {:?} vs {:?}", x.rle, y.rle));
        }
        if x.first_run_value != y.first_run_value {
            d.push(format!(
                "first_run {} vs {}",
                x.first_run_value, y.first_run_value
            ));
        }
        if x.stats != y.stats {
            d.push(format!("stats {:?} vs {:?}", x.stats, y.stats));
        }
        if x.sub_segment != y.sub_segment {
            d.push(format!("sub {:?} vs {:?}", x.sub_segment, y.sub_segment));
        }
        if !d.is_empty() {
            return format!("segment {i}/{}: {}", a.segments.len(), d.join("; "));
        }
    }
    if a.data_objects != b.data_objects {
        return "data objects".into();
    }
    "same fields, different bytes".into()
}

fn first_diff(a: &[u8], b: &[u8]) -> String {
    let at = a
        .iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()));
    format!("{} vs {} bytes, first difference at {at}", a.len(), b.len())
}

/// Re-encodes every user column with its engine dictionary order and value parameters and
/// compares the four files byte for byte.
#[test]
fn large_models_reencode_like_the_engine() {
    for dir in cases() {
        let manifest = read_json(&dir.join("manifest.json"));
        let model = Model::load(&dir, &manifest);
        let case = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let started = Instant::now();
        let mut same: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
        let mut diffs: Vec<String> = Vec::new();
        let mut stat_diffs: Vec<String> = Vec::new();
        let mut tally = |kind: &'static str, ok: bool| {
            let e = same.entry(kind).or_insert((0, 0));
            e.0 += u64::from(ok);
            e.1 += 1;
        };
        let related: HashSet<i64> = model
            .rows("Relationship")
            .iter()
            .flat_map(|r| [int(r, "FromColumnID"), int(r, "ToColumnID")])
            .flatten()
            .collect();
        for c in model.columns.iter().filter(|c| !model.table(c).system) {
            let table = model.table(c);
            let label = format!("{}[{}]", table.name, c.name);
            let (ids, metas) = decode_ids(c);
            let dict = dictionary(c);
            let base_id = metas
                .first()
                .and_then(|m| m.segments.first())
                .map_or(1, |s| s.base_id);
            // The encoder's own hash-versus-value choice for int64 columns without a hint (and
            // outside relationships, whose ends the engine hashes).
            if c.logical == Some(LogicalType::Int64)
                && !c.row_number
                && int(&c.j["column"], "EncodingHint").unwrap_or(0) == 0
                && !related.contains(&c.id)
            {
                let vals = values(&ids, dict.as_ref(), c.encoding);
                let auto = encode_column(
                    &vals,
                    LogicalType::Int64,
                    &EncodeOptions {
                        max_rows: LIMITS.max_rows,
                        ..EncodeOptions::default()
                    },
                )
                .unwrap_or_else(|e| panic!("{case}: {label}: {e}"));
                let ok = auto.stats.kind == c.encoding.kind;
                tally("int64 auto choice", ok);
                if !ok {
                    stat_diffs.push(format!(
                        "int64 choice {label}: ours {:?} engine {:?}",
                        auto.stats.kind, c.encoding.kind
                    ));
                }
            }
            let mut at = 0usize;
            let mut dictionary_done = false;
            for (k, ((idf, meta), part)) in c.parts.iter().zip(&table.partitions).enumerate() {
                let rows = part.0 as usize;
                let part_ids = &ids[at..at + rows];
                at += rows;
                let mut options = EncodeOptions {
                    segment_rows: part.2.max(1),
                    segment_base_id: base_id,
                    build_hidx: c.hidx.is_some(),
                    max_rows: LIMITS.max_rows,
                    ..EncodeOptions::default()
                };
                let e = if c.row_number {
                    let first_row = (at - rows) as u64;
                    bicraft_vertipaq::encode_row_number_from(first_row, rows as u64, &options).unwrap()
                } else {
                    let logical = c.logical.unwrap_or(LogicalType::Int64);
                    match (&dict, c.encoding.kind) {
                        (Some(d), EncodingKind::Hash) => {
                            options.encoding = EncodingRequest::Hash;
                            options.dictionary_seed = seed(d, c.logical);
                            options.wide_long_dictionary = d.element_size == Some(8) && d.type_tag == 0;
                        }
                        // only blanks: no `.dictionary`
                        (None, EncodingKind::Hash) => options.encoding = EncodingRequest::Hash,
                        _ => {
                            options.encoding = EncodingRequest::Value {
                                params: Some(ValueParams {
                                    base_id: c.encoding.base_id,
                                    magnitude: c.encoding.magnitude,
                                }),
                            }
                        }
                    }
                    let vals = values(part_ids, dict.as_ref(), c.encoding);
                    encode_column(&vals, logical, &options).unwrap_or_else(|e| panic!("{case}: {label}: {e}"))
                };
                if k == 0 && table.partitions.len() == 1 {
                    // statistics of a single-partition column against its metadata rows
                    let cs = &c.j["column_storage"];
                    let ds = &c.j["dictionary_storage"];
                    let s = &e.stats;
                    let fields: [(&str, Option<i64>, i64); 12] = [
                        (
                            "DistinctStates",
                            int(cs, "Statistics_DistinctStates"),
                            s.distinct_states as i64,
                        ),
                        (
                            "MinDataID",
                            int(cs, "Statistics_MinDataID"),
                            i64::from(s.min_data_id),
                        ),
                        (
                            "MaxDataID",
                            int(cs, "Statistics_MaxDataID"),
                            i64::from(s.max_data_id),
                        ),
                        ("HasNulls", int(cs, "Statistics_HasNulls"), i64::from(s.has_nulls)),
                        ("RLERuns", int(cs, "Statistics_RLERuns"), s.rle_runs() as i64),
                        (
                            "OthersRLERuns",
                            int(cs, "Statistics_OthersRLERuns"),
                            s.others_rle_runs() as i64,
                        ),
                        ("RowCount", int(cs, "Statistics_RowCount"), s.rows as i64),
                        ("LastId", int(ds, "LastId"), i64::from(s.last_id)),
                        ("IsNullable", int(ds, "IsNullable"), i64::from(s.is_nullable)),
                        ("IsUnique", int(ds, "IsUnique"), i64::from(s.is_unique)),
                        (
                            "IsOperatingOn32",
                            int(ds, "IsOperatingOn32"),
                            i64::from(s.is_operating_on_32),
                        ),
                        (
                            "DictionarySize",
                            int(ds, "Size"),
                            s.dictionary_size.map_or(-1, |x| x as i64),
                        ),
                    ];
                    for (name, engine, ours) in fields {
                        let ok = engine == Some(ours) || (name == "DictionarySize" && ours == -1);
                        tally(name, ok);
                        // a few examples per statistic
                        if !ok && stat_diffs.iter().filter(|d| d.starts_with(name)).count() < 4 {
                            stat_diffs.push(format!("{name} {label}: ours {ours} engine {engine:?}"));
                        }
                    }
                }
                let (engine_idf, engine_meta) = (read(idf), read(meta));
                let ok = e.idf == engine_idf;
                tally("idf", ok);
                if !ok && diffs.len() < 40 {
                    diffs.push(format!("{label} p{k} .idf: {}", first_diff(&e.idf, &engine_idf)));
                }
                let ok = e.idfmeta == engine_meta;
                tally("idfmeta", ok);
                if !ok && diffs.len() < 40 {
                    diffs.push(format!(
                        "{label} p{k} .idfmeta: {}",
                        meta_diff(&e.idfmeta, &engine_meta)
                    ));
                }
                if !dictionary_done && let (Some(ours), Some(path)) = (&e.dictionary, &c.dictionary) {
                    dictionary_done = true;
                    let engine = read(path);
                    let ok = *ours == engine;
                    tally("dictionary", ok);
                    if !ok && diffs.len() < 40 {
                        diffs.push(format!("{label} .dictionary: {}", first_diff(ours, &engine)));
                    }
                }
            }
            if let Some(path) = &c.hidx {
                let options = EncodeOptions {
                    encoding: EncodingRequest::Value {
                        params: Some(ValueParams {
                            base_id: c.encoding.base_id,
                            magnitude: c.encoding.magnitude,
                        }),
                    },
                    max_rows: LIMITS.max_rows,
                    ..EncodeOptions::default()
                };
                let vals = values(&ids, None, c.encoding);
                let e = encode_column(&vals, c.logical.unwrap_or(LogicalType::Int64), &options).unwrap();
                let engine = read(path);
                let ok = e.hidx.as_deref() == Some(engine.as_slice());
                tally("hidx", ok);
                if !ok && diffs.len() < 40 {
                    diffs.push(format!(
                        "{label} .hidx: {}",
                        first_diff(e.hidx.as_deref().unwrap_or(&[]), &engine)
                    ));
                }
            }
        }
        let summary: Vec<String> = same.iter().map(|(k, (s, n))| format!("{k} {s}/{n}")).collect();
        eprintln!(
            "{case}: re-encoded in {:.1}s, identical: {}\n  {}\n  {}",
            started.elapsed().as_secs_f64(),
            summary.join(", "),
            diffs.join("\n  "),
            stat_diffs.join("\n  ")
        );
    }
}

// ---------------------------------------------------------------------------------------------
// System tables

/// Writes the files of a re-encoded model for the development oracles (`rewrite`, then AMO
/// `ImageLoad` into the engine and the same aggregates): every hash-encoded user column
/// re-encoded with the encoder's own choices (dictionary ids in first-occurrence order,
/// uncompressed string pages) and every `H$`/`R$`/`U$` system table rebuilt from the new data
/// ids. Only files whose bytes change are written. Runs when `BICRAFT_LARGE_REWRITE` names an
/// output folder (one sub-folder per case), with `BICRAFT_LARGE=1`.
#[test]
fn large_models_rewrite_for_the_engine() {
    let Some(out_root) = bicraft_env::var_os("BICRAFT_LARGE_REWRITE").map(PathBuf::from) else {
        eprintln!("BICRAFT_LARGE_REWRITE is not set: not writing re-encoded models");
        return;
    };
    for dir in cases() {
        let manifest = read_json(&dir.join("manifest.json"));
        let model = Model::load(&dir, &manifest);
        let collation = model.collation();
        let case = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let out = out_root.join(&case);
        std::fs::create_dir_all(&out).unwrap();
        let started = Instant::now();
        let mut list: Vec<Value> = Vec::new();
        let mut written = 0u64;
        let mut put = |path: &str, bytes: &[u8], engine: &Path| -> bool {
            if read(engine) == bytes {
                return false;
            }
            let file = format!("f{}.bin", list.len());
            std::fs::write(out.join(&file), bytes).unwrap();
            written += bytes.len() as u64;
            list.push(serde_json::json!({ "path": path, "file": file }));
            true
        };
        let mut ids_of: HashMap<i64, Vec<u32>> = HashMap::new();
        let mut dict_of: HashMap<i64, Option<Dictionary>> = HashMap::new();
        let mut changed_columns = 0;
        for c in model
            .columns
            .iter()
            .filter(|c| !model.table(c).system && !c.row_number)
        {
            let table = model.table(c);
            let (ids, metas) = decode_ids(c);
            let dict = dictionary(c);
            if c.encoding.kind != EncodingKind::Hash {
                ids_of.insert(c.id, ids);
                dict_of.insert(c.id, dict);
                continue;
            }
            // natural re-encoding: the whole column first, for its dictionary and ids
            let all = values(&ids, dict.as_ref(), c.encoding);
            let logical = c.logical.unwrap_or(LogicalType::String);
            let base_id = metas
                .first()
                .and_then(|m| m.segments.first())
                .map_or(1, |s| s.base_id);
            // Own order: first occurrence, then the engine's values that no row holds any more
            // (a DirectQuery partition converted from import keeps its values in the dictionary;
            // `LastId` in the metadata, left as is, must still match).
            let mut own: Vec<CellValue> = Vec::new();
            {
                let mut seen = HashSet::new();
                for v in all.iter().chain(
                    dict.as_ref()
                        .map(|d| seed(d, c.logical))
                        .unwrap_or_default()
                        .iter(),
                ) {
                    if !matches!(v, CellValue::Null) && seen.insert(format!("{v:?}")) {
                        own.push(v.clone());
                    }
                }
            }
            let options = EncodeOptions {
                encoding: EncodingRequest::Hash,
                dictionary_seed: own,
                segment_base_id: base_id,
                max_rows: LIMITS.max_rows,
                ..EncodeOptions::default()
            };
            let whole = encode_column(&all, logical, &options).unwrap();
            // a column of blanks: no dictionary
            let new_dict = whole.dictionary.as_deref().map(|d| parse_dictionary(d).unwrap());
            let seeded = EncodeOptions {
                dictionary_seed: new_dict.as_ref().map(|d| seed(d, c.logical)).unwrap_or_default(),
                ..options.clone()
            };
            let mut new_ids = Vec::with_capacity(ids.len());
            let mut changed = false;
            let mut at = 0usize;
            for (k, part) in table.partitions.iter().enumerate() {
                let rows = part.0 as usize;
                let o = EncodeOptions {
                    segment_rows: part.2.max(1),
                    ..seeded.clone()
                };
                let e = encode_column(&all[at..at + rows], logical, &o).unwrap();
                at += rows;
                let p = &c.j["parts"][k];
                changed |= put(text(&p["idf"], "path").unwrap(), &e.idf, &c.parts[k].0);
                changed |= put(text(&p["idfmeta"], "path").unwrap(), &e.idfmeta, &c.parts[k].1);
                let meta = parse_idfmeta(&e.idfmeta).unwrap();
                new_ids.extend(decode_data_ids(&e.idf, &meta, &LIMITS).unwrap());
            }
            // a column of blanks has no .dictionary (nor does the re-encoded one)
            if let (Some(path), Some(bytes), Some(engine)) = (
                text(&c.j["dictionary"], "path"),
                whole.dictionary.as_deref(),
                c.dictionary.as_ref(),
            ) {
                changed |= put(path, bytes, engine);
            }
            changed_columns += usize::from(changed || new_ids != ids);
            ids_of.insert(c.id, new_ids);
            dict_of.insert(c.id, new_dict);
        }
        // system tables from the new ids
        let mut replace_system = |system_table: i64, data: &bicraft_vertipaq::SystemTableData| {
            for (i, col) in data.columns.iter().enumerate() {
                let engine = model.column(system_table, col.name).unwrap();
                let enc = data.encode(i).unwrap();
                let p = &engine.j["parts"][0];
                put(text(&p["idf"], "path").unwrap(), &enc.idf, &engine.parts[0].0);
                put(
                    text(&p["idfmeta"], "path").unwrap(),
                    &enc.idfmeta,
                    &engine.parts[0].1,
                );
            }
        };
        let mut hierarchies: HashMap<i64, bicraft_vertipaq::AttributeHierarchy> = HashMap::new();
        for c in model
            .columns
            .iter()
            .filter(|c| !model.table(c).system && !c.row_number)
        {
            let ahs = &c.j["attribute_hierarchy_storage"];
            let options = AttributeHierarchyOptions {
                sort_by: None,
                id_to_pos: int(ahs, "MaterializationType") == Some(0),
            };
            let h = build_attribute_hierarchy_with(
                &entries(&ids_of[&c.id], dict_of[&c.id].as_ref(), c.encoding),
                &collation,
                &options,
            )
            .unwrap();
            let st = int(ahs, "SystemTableID").unwrap_or(0);
            if st != 0 {
                replace_system(st, &h.table);
            }
            hierarchies.insert(c.id, h);
        }
        let storage: HashMap<i64, &Value> = model
            .rows("RelationshipStorage")
            .iter()
            .filter_map(|r| Some((int(r, "ID")?, r)))
            .collect();
        let index_storage: HashMap<i64, &Value> = model
            .rows("RelationshipIndexStorage")
            .iter()
            .filter_map(|r| Some((int(r, "ID")?, r)))
            .collect();
        for r in model.rows("Relationship") {
            let (Some(from), Some(to)) = (
                int(r, "FromColumnID").and_then(|i| model.column_by_id(i)),
                int(r, "ToColumnID").and_then(|i| model.column_by_id(i)),
            ) else {
                continue;
            };
            let Some(st) = int(r, "RelationshipStorageID")
                .and_then(|i| storage.get(&i))
                .and_then(|s| int(s, "RelationshipIndexStorageID"))
                .and_then(|i| index_storage.get(&i))
                .and_then(|s| int(s, "SystemTableID"))
                // 0: no R$ (a hybrid from-table)
                .filter(|&st| st != 0)
            else {
                continue;
            };
            let from_entries = entries(&ids_of[&from.id], dict_of[&from.id].as_ref(), from.encoding);
            let to_rows = values(&ids_of[&to.id], dict_of[&to.id].as_ref(), to.encoding);
            let index = build_relationship_index(&from_entries, &to_rows, &collation).unwrap();
            replace_system(st, &index.table);
        }
        let hierarchy_storage: HashMap<i64, &Value> = model
            .rows("HierarchyStorage")
            .iter()
            .filter_map(|r| Some((int(r, "ID")?, r)))
            .collect();
        for h in model.rows("Hierarchy") {
            let Some(hid) = int(h, "ID") else { continue };
            let mut levels: Vec<&Value> = model
                .rows("Level")
                .iter()
                .filter(|l| int(l, "HierarchyID") == Some(hid))
                .collect();
            levels.sort_by_key(|l| int(l, "Ordinal"));
            let cols: Vec<i64> = levels.iter().filter_map(|l| int(l, "ColumnID")).collect();
            let built: Vec<UserHierarchyLevel<'_>> = cols
                .iter()
                .map(|c| UserHierarchyLevel {
                    data_ids: &ids_of[c],
                    hierarchy: &hierarchies[c],
                })
                .collect();
            let u = build_user_hierarchy(&built).unwrap();
            if let Some(st) = int(h, "HierarchyStorageID")
                .and_then(|i| hierarchy_storage.get(&i))
                .and_then(|s| int(s, "SystemTableID"))
                .filter(|&st| st != 0)
            {
                replace_system(st, &u.table);
            }
        }
        drop(replace_system);
        std::fs::write(
            out.join("rewrite.json"),
            serde_json::to_vec_pretty(&list).unwrap(),
        )
        .unwrap();
        eprintln!(
            "{case}: {changed_columns} hash columns re-encoded with their own dictionary order; {} files ({written} bytes) differ from the engine's, written to {} in {:.1}s",
            list.len(),
            out.display(),
            started.elapsed().as_secs_f64()
        );
    }
}

/// Every distinct data id of a column with its value (blank = id 2).
fn entries(ids: &[u32], dict: Option<&Dictionary>, enc: ColumnEncoding) -> Vec<(u32, CellValue)> {
    let mut distinct: Vec<u32> = ids
        .iter()
        .copied()
        .collect::<HashSet<u32>>()
        .into_iter()
        .collect();
    distinct.sort_unstable();
    let vals = values(&distinct, dict, enc);
    distinct.into_iter().zip(vals).collect()
}

/// Rebuilds the `H$`, `R$` and `U$` system tables from the decoded user columns and compares
/// their files with the engine's byte for byte.
#[test]
fn large_models_system_tables_like_the_engine() {
    for dir in cases() {
        let manifest = read_json(&dir.join("manifest.json"));
        let model = Model::load(&dir, &manifest);
        let collation = model.collation();
        let case = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let started = Instant::now();
        let same: std::cell::RefCell<BTreeMap<&str, (u64, u64)>> = Default::default();
        let tally = |kind: &'static str, ok: bool| {
            let mut m = same.borrow_mut();
            let e = m.entry(kind).or_insert((0, 0));
            e.0 += u64::from(ok);
            e.1 += 1;
        };
        let mut diffs: Vec<String> = Vec::new();
        let mut compare =
            |kind: &'static str, label: &str, system_table: i64, data: &bicraft_vertipaq::SystemTableData| {
                for (i, col) in data.columns.iter().enumerate() {
                    let Some(engine) = model.column(system_table, col.name) else {
                        diffs.push(format!("{kind} {label}: no engine column {}", col.name));
                        continue;
                    };
                    let ours = data.encode(i).unwrap_or_else(|e| panic!("{case}: {label}: {e}"));
                    let (idf, meta) = &engine.parts[0];
                    let ok = ours.idf == read(idf) && ours.idfmeta == read(meta);
                    tally(kind, ok);
                    if !ok && diffs.len() < 30 {
                        let engine_meta = read(meta);
                        diffs.push(format!(
                            "{kind} {label} {}: idf {}, idfmeta {}",
                            col.name,
                            first_diff(&ours.idf, &read(idf)),
                            meta_diff(&ours.idfmeta, &engine_meta)
                        ));
                    }
                }
            };
        let mut ids_of: HashMap<i64, Vec<u32>> = HashMap::new();
        let mut hierarchies: HashMap<i64, bicraft_vertipaq::AttributeHierarchy> = HashMap::new();
        // H$
        for c in model
            .columns
            .iter()
            .filter(|c| !model.table(c).system && !c.row_number)
        {
            let ahs = &c.j["attribute_hierarchy_storage"];
            let system_table = int(ahs, "SystemTableID").unwrap_or(0);
            let (ids, _) = decode_ids(c);
            let dict = dictionary(c);
            let options = AttributeHierarchyOptions {
                sort_by: None,
                id_to_pos: int(ahs, "MaterializationType") == Some(0),
            };
            let h = build_attribute_hierarchy_with(
                &entries(&ids, dict.as_ref(), c.encoding),
                &collation,
                &options,
            )
            .unwrap_or_else(|e| panic!("{case}: {}[{}]: {e}", model.table(c).name, c.name));
            if system_table != 0 {
                let label = format!("{}[{}]", model.table(c).name, c.name);
                compare("H$", &label, system_table, &h.table);
                tally(
                    "DistinctDataCount",
                    int(ahs, "DistinctDataCount") == Some(h.distinct_data_count as i64),
                );
            }
            ids_of.insert(c.id, ids);
            hierarchies.insert(c.id, h);
        }
        // R$
        let storage: HashMap<i64, &Value> = model
            .rows("RelationshipStorage")
            .iter()
            .filter_map(|r| Some((int(r, "ID")?, r)))
            .collect();
        let index_storage: HashMap<i64, &Value> = model
            .rows("RelationshipIndexStorage")
            .iter()
            .filter_map(|r| Some((int(r, "ID")?, r)))
            .collect();
        for r in model.rows("Relationship") {
            let (Some(from), Some(to)) = (
                int(r, "FromColumnID").and_then(|i| model.column_by_id(i)),
                int(r, "ToColumnID").and_then(|i| model.column_by_id(i)),
            ) else {
                continue;
            };
            let Some(ris) = int(r, "RelationshipStorageID")
                .and_then(|i| storage.get(&i))
                .and_then(|s| int(s, "RelationshipIndexStorageID"))
                .and_then(|i| index_storage.get(&i))
            else {
                continue;
            };
            let from_entries = entries(&ids_of[&from.id], dictionary(from).as_ref(), from.encoding);
            let to_rows = values(&ids_of[&to.id], dictionary(to).as_ref(), to.encoding);
            let index = build_relationship_index(&from_entries, &to_rows, &collation)
                .unwrap_or_else(|e| panic!("{case}: relationship: {e}"));
            let label = format!(
                "{}[{}] -> {}[{}]",
                model.table(from).name,
                from.name,
                model.table(to).name,
                to.name
            );
            tally(
                "R$ RecordCount",
                int(ris, "RecordCount") == Some(index.record_count() as i64),
            );
            compare("R$", &label, int(ris, "SystemTableID").unwrap_or(0), &index.table);
        }
        // U$
        let hierarchy_storage: HashMap<i64, &Value> = model
            .rows("HierarchyStorage")
            .iter()
            .filter_map(|r| Some((int(r, "ID")?, r)))
            .collect();
        for h in model.rows("Hierarchy") {
            let Some(hid) = int(h, "ID") else { continue };
            let mut levels: Vec<&Value> = model
                .rows("Level")
                .iter()
                .filter(|l| int(l, "HierarchyID") == Some(hid))
                .collect();
            levels.sort_by_key(|l| int(l, "Ordinal"));
            let cols: Vec<i64> = levels.iter().filter_map(|l| int(l, "ColumnID")).collect();
            let built: Vec<UserHierarchyLevel<'_>> = cols
                .iter()
                .map(|c| UserHierarchyLevel {
                    data_ids: &ids_of[c],
                    hierarchy: &hierarchies[c],
                })
                .collect();
            let u = build_user_hierarchy(&built).unwrap_or_else(|e| panic!("{case}: hierarchy: {e}"));
            let Some(hs) = int(h, "HierarchyStorageID").and_then(|i| hierarchy_storage.get(&i)) else {
                continue;
            };
            let label = text(h, "Name").unwrap_or("?").to_string();
            compare("U$", &label, int(hs, "SystemTableID").unwrap_or(0), &u.table);
        }
        let summary: Vec<String> = same
            .borrow()
            .iter()
            .map(|(k, (s, n))| format!("{k} {s}/{n}"))
            .collect();
        eprintln!(
            "{case}: system tables rebuilt in {:.1}s, identical: {}\n  {}",
            started.elapsed().as_secs_f64(),
            summary.join(", "),
            diffs.join("\n  ")
        );
    }
}
