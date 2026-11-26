//! Encoder tests on synthetic data: every encoded column must decode back to its input.

use bicraft_vertipaq::dictionary::PageStore;
use bicraft_vertipaq::{
    CellValue, ColumnEncoding, EncodeOptions, EncodedColumn, EncodingKind, EncodingRequest, Error,
    LogicalType, StorageType, StringCompression, ValueParams, decode_column, encode_column,
    encode_row_number, parse_dictionary, parse_hash_index, parse_idfmeta,
};

fn storage_code(s: StorageType) -> i64 {
    match s {
        StorageType::String => 2,
        StorageType::Long => 6,
        StorageType::Real => 8,
        StorageType::None => 19,
    }
}

fn decode(e: &EncodedColumn, logical: LogicalType) -> Vec<CellValue> {
    let kind = if e.stats.kind == EncodingKind::Hash { 1 } else { 2 };
    let enc = ColumnEncoding::from_dictionary_storage(
        kind,
        storage_code(e.stats.storage_type),
        e.stats.base_id,
        e.stats.magnitude,
    )
    .unwrap()
    .with_logical_type(Some(logical));
    decode_column(&e.idf, &e.idfmeta, e.dictionary.as_deref(), enc).unwrap()
}

/// Encodes, decodes, checks equality and returns the encoded column.
fn round_trip(values: &[CellValue], logical: LogicalType, options: &EncodeOptions) -> EncodedColumn {
    let e = encode_column(values, logical, options).unwrap();
    let back = decode(&e, logical);
    if back != values {
        let k = back
            .iter()
            .zip(values)
            .position(|(a, b)| a != b)
            .unwrap_or(back.len().min(values.len()));
        panic!(
            "{logical:?} round trip differs at row {k} of {}/{}: {:?} vs {:?}",
            back.len(),
            values.len(),
            back.get(k),
            values.get(k)
        );
    }
    assert_eq!(e.stats.rows, values.len() as u64);
    // Deterministic output.
    assert_eq!(encode_column(values, logical, options).unwrap(), e);
    e
}

fn opts() -> EncodeOptions {
    EncodeOptions::default()
}

/// A small deterministic generator.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[test]
fn int64_hash_and_value() {
    let values: Vec<CellValue> = (0..500).map(|i| CellValue::Int((i * 7919) % 103 - 50)).collect();
    let hash = EncodeOptions {
        encoding: EncodingRequest::Hash,
        ..opts()
    };
    let e = round_trip(&values, LogicalType::Int64, &hash);
    assert_eq!(e.stats.kind, EncodingKind::Hash);
    assert!(e.stats.is_operating_on_32);
    assert_eq!(e.stats.dictionary_len, 103);
    assert_eq!(e.stats.last_id, 105);
    assert_eq!(e.stats.distinct_values, 103);
    // 103 values: past 72 the engine's hash table has 32 bins (engine: 2 732 bytes)
    assert_eq!(e.stats.dictionary_size, Some(2732));
    let v = EncodeOptions {
        encoding: EncodingRequest::Value { params: None },
        ..opts()
    };
    let e = round_trip(&values, LogicalType::Int64, &v);
    assert_eq!(e.stats.kind, EncodingKind::Value);
    assert_eq!((e.stats.base_id, e.stats.magnitude), (-53, 1.0));
    assert_eq!((e.stats.min_data_id, e.stats.max_data_id), (3, 105));
    assert!(e.hidx.is_some());
    // 64-bit dictionary values.
    let wide = vec![
        CellValue::Int(i64::MIN),
        CellValue::Int(i64::MAX),
        CellValue::Int(0),
        CellValue::Null,
    ];
    let e = round_trip(&wide, LogicalType::Int64, &opts());
    assert!(!e.stats.is_operating_on_32);
    let d = parse_dictionary(e.dictionary.as_deref().unwrap()).unwrap();
    assert_eq!(d.element_size, Some(8));
    // Value encoding cannot hold this range.
    assert!(matches!(
        encode_column(&wide, LogicalType::Int64, &v),
        Err(Error::LimitExceeded(_))
    ));
}

#[test]
fn double_and_datetime() {
    let mut values: Vec<CellValue> = (0..300)
        .map(|i| CellValue::Real(f64::from(i % 37) * 0.25 - 3.0))
        .collect();
    values.extend([
        CellValue::Real(f64::MAX),
        CellValue::Real(f64::MIN_POSITIVE),
        CellValue::Real(f64::INFINITY),
        CellValue::Null,
        CellValue::Real(-0.0),
        CellValue::Real(0.0),
    ]);
    let e = round_trip(&values, LogicalType::Double, &opts());
    assert_eq!(e.stats.storage_type, StorageType::Real);
    assert_eq!(e.stats.xm_type, 1);
    assert!(e.stats.has_nulls);
    let dates: Vec<CellValue> = (0..1000)
        .map(|i| CellValue::DateTime(45_000.0 + f64::from(i / 3) + 0.5))
        .collect();
    let e = round_trip(&dates, LogicalType::DateTime, &opts());
    assert_eq!(e.stats.db_type, 7);
    assert_eq!(e.stats.distinct_values, 334);
}

#[test]
fn decimal_value_encoding_picks_magnitude() {
    // Multiples of 0.001 currency units (raw × 10 000 → multiples of 10).
    let values: Vec<CellValue> = (0..50).map(|i| CellValue::Decimal(437_020 + i * 1_250)).collect();
    let e = round_trip(&values, LogicalType::Decimal, &opts());
    assert_eq!(e.stats.kind, EncodingKind::Value);
    assert_eq!(e.stats.magnitude, 0.1);
    assert_eq!(e.stats.base_id, 43_702 - 3);
    assert_eq!(e.stats.db_type, 6);
    assert_eq!(
        parse_idfmeta(&e.idfmeta).unwrap().segments[0]
            .stats
            .original_min_segment_data_id,
        3
    );
    // Whole currency units → Magnitude 10^-4.
    let whole: Vec<CellValue> = [9, 12, 399, 9]
        .iter()
        .map(|n| CellValue::Decimal(n * 10_000))
        .collect();
    let e = round_trip(&whole, LogicalType::Decimal, &opts());
    assert_eq!((e.stats.base_id, e.stats.magnitude), (6, 0.0001));
    assert_eq!(
        e.stats.distinct_states, e.stats.distinct_values,
        "a .hidx was written"
    );
    let no_hidx = EncodeOptions {
        build_hidx: false,
        ..opts()
    };
    let e = round_trip(&whole, LogicalType::Decimal, &no_hidx);
    assert!(e.hidx.is_none());
    assert_eq!(e.stats.distinct_states, 391, "id range without a .hidx");
    // Forced parameters, and forced hash.
    let forced = EncodeOptions {
        encoding: EncodingRequest::Value {
            params: Some(ValueParams {
                base_id: 0,
                magnitude: 0.01,
            }),
        },
        ..opts()
    };
    let e = round_trip(&whole, LogicalType::Decimal, &forced);
    assert_eq!(e.stats.base_id, 0);
    let hash = EncodeOptions {
        encoding: EncodingRequest::Hash,
        ..opts()
    };
    round_trip(&whole, LogicalType::Decimal, &hash);
    // Huge spreads fall back to hash encoding.
    let wide = vec![
        CellValue::Decimal(1),
        CellValue::Decimal(i64::MAX),
        CellValue::Null,
    ];
    let e = round_trip(&wide, LogicalType::Decimal, &opts());
    assert_eq!(e.stats.kind, EncodingKind::Hash);
}

#[test]
fn strings_with_unicode_and_empties() {
    let words = [
        "",
        "a",
        "Zürich",
        "東京",
        "emoji 😀 pair",
        "naïve café",
        "tab\tnew\nline",
        "ÿ",
        "ĀĂ",
    ];
    let mut values: Vec<CellValue> = (0..400)
        .map(|i| CellValue::Text(words[i % words.len()].into()))
        .collect();
    values.insert(7, CellValue::Null);
    let e = round_trip(&values, LogicalType::String, &opts());
    assert_eq!(e.stats.dictionary_flags, 3);
    assert_eq!(e.stats.db_type, 130);
    assert_eq!(
        e.stats.longest_string,
        "emoji 😀 pair".encode_utf16().count() as u64
    );
    assert!(!e.stats.is_unique);
    let d = parse_dictionary(e.dictionary.as_deref().unwrap()).unwrap();
    let store = d.string_store.clone().unwrap();
    assert_eq!(store.pages.len(), 1);
    assert_eq!(store.pages[0].contains_nulls, 1);
    // First-occurrence order.
    assert_eq!(d.index_of(3), Some(0));
    match &d.values {
        bicraft_vertipaq::DictionaryValues::Text(v) => assert_eq!(v[..3], ["", "a", "Zürich"]),
        other => panic!("{other:?}"),
    }
}

#[test]
fn boolean_column() {
    let values: Vec<CellValue> = (0..200).map(|i| CellValue::Bool(i % 3 == 0)).collect();
    let e = round_trip(&values, LogicalType::Boolean, &opts());
    assert_eq!(e.stats.db_type, 11);
    assert!(e.stats.is_operating_on_32);
    // Ints 0/1 are accepted for booleans; other ints are not.
    let ints = vec![CellValue::Int(1), CellValue::Int(0)];
    let e = encode_column(&ints, LogicalType::Boolean, &opts()).unwrap();
    assert_eq!(
        decode(&e, LogicalType::Boolean),
        vec![CellValue::Bool(true), CellValue::Bool(false)]
    );
    assert!(encode_column(&[CellValue::Int(2)], LogicalType::Boolean, &opts()).is_err());
}

#[test]
fn blanks_in_every_encoding() {
    let ints: Vec<CellValue> = (0..300)
        .map(|i| {
            if i % 4 == 0 {
                CellValue::Null
            } else {
                CellValue::Int(i % 9 + 100)
            }
        })
        .collect();
    for encoding in [EncodingRequest::Hash, EncodingRequest::Value { params: None }] {
        let o = EncodeOptions { encoding, ..opts() };
        let e = round_trip(&ints, LogicalType::Int64, &o);
        assert!(e.stats.has_nulls && e.stats.is_nullable);
        let seg = &parse_idfmeta(&e.idfmeta).unwrap().segments[0];
        assert_eq!(seg.stats.has_nulls, 1);
        assert_eq!(seg.stats.min_data_id, 3, "the null id is not the minimum");
    }
    let all_null = vec![CellValue::Null; 100];
    for logical in [
        LogicalType::String,
        LogicalType::Int64,
        LogicalType::Decimal,
        LogicalType::Double,
    ] {
        let e = round_trip(&all_null, logical, &opts());
        assert_eq!(e.stats.distinct_values, 0);
        assert_eq!(e.stats.id_range, 0);
    }
    // Regression (G10): hash-encoded columns of blanks are written like the engine's (calculated
    // columns `IF([Value] < 0, …)` of every type): no `.dictionary`, `LastId` 2, one distinct
    // state, original minimum id 0x8000_0000, and the observed `DictionaryStorage.Size`.
    let hash = EncodeOptions {
        encoding: EncodingRequest::Hash,
        ..opts()
    };
    for (logical, size) in [
        (LogicalType::Int64, 1296),
        (LogicalType::String, 16904),
        (LogicalType::Double, 2280),
        (LogicalType::DateTime, 2280),
    ] {
        let e = round_trip(&all_null, logical, &hash);
        assert!(e.dictionary.is_none(), "{logical:?}");
        let s = &e.stats;
        assert_eq!(
            (s.last_id, s.distinct_states, s.dictionary_size),
            (2, 1, Some(size))
        );
        assert_eq!(s.original_min_segment_data_id, 0x8000_0000);
        assert_eq!(s.is_operating_on_32, logical == LogicalType::Int64);
        assert_eq!((s.min_data_id, s.max_data_id, s.rle_runs()), (2, 2, 1));
        let seg = &parse_idfmeta(&e.idfmeta).unwrap().segments[0];
        assert_eq!(seg.stats.original_min_segment_data_id, 0x8000_0000);
    }
}

#[test]
fn all_equal_and_long_runs_use_rle() {
    let same = vec![CellValue::Text("same".into()); 10_000];
    let e = round_trip(&same, LogicalType::String, &opts());
    let seg = &parse_idfmeta(&e.idfmeta).unwrap().segments[0];
    assert_eq!((seg.stats.rle_runs, seg.stats.others_rle_runs), (1, 0));
    assert_eq!(seg.sub_segment.unwrap().records, 0);
    // Mixed: long runs and short noise between them.
    let mut rng = Rng(7);
    let mut values = Vec::new();
    for block in 0..40 {
        if block % 2 == 0 {
            let id = rng.below(5) as i64;
            values.extend(std::iter::repeat_n(
                CellValue::Int(id),
                64 + rng.below(500) as usize,
            ));
        } else {
            values.extend((0..rng.below(70)).map(|_| CellValue::Int(rng.below(20) as i64)));
        }
    }
    let e = round_trip(&values, LogicalType::Int64, &opts());
    let s = e.stats.segments[0];
    assert!(s.rle_runs >= 20 && s.others_rle_runs >= 10, "{s:?}");
    // Below the threshold everything is bit-packed.
    let short: Vec<CellValue> = (0..2000).map(|i| CellValue::Int(i / 63)).collect();
    let e = round_trip(&short, LogicalType::Int64, &opts());
    assert_eq!(
        (e.stats.segments[0].rle_runs, e.stats.segments[0].others_rle_runs),
        (0, 1)
    );
    let o = EncodeOptions {
        rle_min_run: 10,
        ..opts()
    };
    let e = round_trip(&short, LogicalType::Int64, &o);
    assert!(e.stats.segments[0].rle_runs > 30);
}

#[test]
fn many_distinct_values() {
    let values: Vec<CellValue> = (0..100_000)
        .map(|i| CellValue::Text(format!("value-{}", (i * 7) % 99_991)))
        .collect();
    let e = round_trip(&values, LogicalType::String, &opts());
    assert_eq!(e.stats.distinct_values, 99_991);
    assert_eq!(e.stats.segments[0].bit_width, 21);
    let ints: Vec<CellValue> = (0..100_000i64).map(|i| CellValue::Int(i * 1_000_003)).collect();
    let e = round_trip(&ints, LogicalType::Int64, &opts());
    assert_eq!(e.stats.dictionary_len, 100_000);
    // all distinct, but 8-byte values: the engine writes IsUnique 0 for those
    assert!(!e.stats.is_unique);
    let small: Vec<CellValue> = (0..1000i64).map(|i| CellValue::Int(i * 7)).collect();
    let hash = EncodeOptions {
        encoding: EncodingRequest::Hash,
        ..opts()
    };
    assert!(round_trip(&small, LogicalType::Int64, &hash).stats.is_unique);
}

/// Regression (G10): without a hint, int64 columns are value-encoded when they have at most
/// 20 rows per distinct value and the ids stay below 2^31 (engine sweep: 15 and 20 rows per
/// value value-encoded, 22 to 100 hashed, a 4·10^9-wide range hashed; Desktop sample alike).
#[test]
fn int64_auto_choice_follows_rows_per_value() {
    let column = |rows: i64, distinct: i64| -> Vec<CellValue> {
        (1..=rows).map(|v| CellValue::Int((v * 7) % distinct)).collect()
    };
    for (rows, distinct, kind) in [
        (1000, 50, EncodingKind::Value),
        (30_000, 2000, EncodingKind::Value),
        (5000, 300, EncodingKind::Value),
        (30_000, 1364, EncodingKind::Hash),
        (30_000, 1000, EncodingKind::Hash),
        (1000, 30, EncodingKind::Hash),
        (5000, 2, EncodingKind::Hash),
    ] {
        let e = round_trip(&column(rows, distinct), LogicalType::Int64, &opts());
        assert_eq!(e.stats.kind, kind, "{rows} rows, {distinct} values");
    }
    let sparse: Vec<CellValue> = (1..=4000i64).map(|v| CellValue::Int(v * 1_000_003)).collect();
    assert_eq!(
        round_trip(&sparse, LogicalType::Int64, &opts()).stats.kind,
        EncodingKind::Hash
    );
    let blanks = vec![CellValue::Null; 10];
    assert_eq!(
        round_trip(&blanks, LogicalType::Int64, &opts()).stats.kind,
        EncodingKind::Hash
    );
}

#[test]
fn multiple_segments() {
    let mut rng = Rng(99);
    let values: Vec<CellValue> = (0..10_500)
        .map(|i| match i % 11 {
            0 => CellValue::Null,
            _ if i > 5000 && i < 6000 => CellValue::Int(42),
            _ => CellValue::Int(rng.below(3000) as i64),
        })
        .collect();
    let o = EncodeOptions {
        segment_rows: 1000,
        ..opts()
    };
    for encoding in [EncodingRequest::Hash, EncodingRequest::Value { params: None }] {
        let e = round_trip(
            &values,
            LogicalType::Int64,
            &EncodeOptions {
                encoding,
                ..o.clone()
            },
        );
        assert_eq!(e.stats.segment_count(), 11);
        assert_eq!(e.stats.records_per_segment, 1000);
        assert_eq!(e.stats.segments.last().unwrap().records, 500);
        let meta = parse_idfmeta(&e.idfmeta).unwrap();
        assert_eq!(meta.segments.len(), 11);
    }
    // The default segment size (2^20 rows) splits larger columns.
    let big: Vec<CellValue> = (0..(1u32 << 20) + 5)
        .map(|i| CellValue::Int(i64::from(i % 1000)))
        .collect();
    let e = round_trip(&big, LogicalType::Int64, &opts());
    assert_eq!(e.stats.segment_count(), 2);
    assert_eq!(e.stats.segments[1].records, 5);
}

#[test]
fn row_numbers() {
    for rows in [0u64, 1, 24, 9503] {
        let e = encode_row_number(rows, &opts()).unwrap();
        let enc = ColumnEncoding::value(-3, 1.0, StorageType::Long);
        let cells = decode_column(&e.idf, &e.idfmeta, None, enc).unwrap();
        assert_eq!(cells, (0..rows as i64).map(CellValue::Int).collect::<Vec<_>>());
        assert_eq!(e.stats.distinct_states, rows);
    }
    let o = EncodeOptions {
        segment_rows: 100,
        ..opts()
    };
    let e = encode_row_number(1050, &o).unwrap();
    assert_eq!(e.stats.segment_count(), 11);
    let enc = ColumnEncoding::value(-3, 1.0, StorageType::Long);
    let cells = decode_column(&e.idf, &e.idfmeta, None, enc).unwrap();
    assert_eq!(cells, (0..1050).map(CellValue::Int).collect::<Vec<_>>());
}

#[test]
fn huffman_pages() {
    let mut rng = Rng(3);
    let ascii: Vec<CellValue> = (0..3000)
        .map(|i| {
            let len = rng.below(30) as usize;
            let s: String = (0..len).map(|_| (b'a' + rng.below(6) as u8) as char).collect();
            CellValue::Text(format!("{s}{}", i % 500))
        })
        .collect();
    let mixed: Vec<CellValue> = (0..800)
        .map(|i| CellValue::Text(format!("{}-é東😀-{}", i % 97, "ab".repeat(i % 5))))
        .collect();
    let empties = vec![
        CellValue::Text(String::new()),
        CellValue::Text("x".into()),
        CellValue::Text(String::new()),
    ];
    for mode in [StringCompression::Always, StringCompression::WhenSmaller] {
        for (values, max_page_chars) in [(&ascii, 4096u64), (&mixed, 1 << 20), (&empties, 1 << 20)] {
            let o = EncodeOptions {
                string_compression: mode,
                max_page_chars,
                ..opts()
            };
            let e = round_trip(values, LogicalType::String, &o);
            let d = parse_dictionary(e.dictionary.as_deref().unwrap()).unwrap();
            let store = d.string_store.unwrap();
            if mode == StringCompression::Always {
                assert!(
                    store
                        .pages
                        .iter()
                        .all(|p| matches!(p.store, PageStore::Compressed { .. }))
                );
            }
            if max_page_chars == 4096 {
                assert!(store.pages.len() > 5, "pages split");
            }
        }
    }
}

#[test]
fn uncompressed_pages_split() {
    let values: Vec<CellValue> = (0..1000).map(|i| CellValue::Text(format!("s{i}"))).collect();
    let o = EncodeOptions {
        max_page_chars: 100,
        ..opts()
    };
    let e = round_trip(&values, LogicalType::String, &o);
    let d = parse_dictionary(e.dictionary.as_deref().unwrap()).unwrap();
    assert!(d.string_store.unwrap().pages.len() > 40);
    // the page split does not change the size: 17 072 + 8 × 1 000 + 2 × 4 890 units
    assert_eq!(e.stats.dictionary_size, Some(34_852));
}

#[test]
fn dictionary_seed_keeps_ids() {
    let seed: Vec<CellValue> = ["c", "b", "a", "unused", "b"]
        .iter()
        .map(|s| CellValue::Text(s.to_string()))
        .collect();
    let values: Vec<CellValue> = ["a", "b", "c", "d", "a"]
        .iter()
        .map(|s| CellValue::Text(s.to_string()))
        .collect();
    let o = EncodeOptions {
        dictionary_seed: seed,
        ..opts()
    };
    let e = round_trip(&values, LogicalType::String, &o);
    let d = parse_dictionary(e.dictionary.as_deref().unwrap()).unwrap();
    match d.values {
        bicraft_vertipaq::DictionaryValues::Text(v) => assert_eq!(v, ["c", "b", "a", "unused", "d"]),
        other => panic!("{other:?}"),
    }
    assert_eq!(e.stats.distinct_values, 4);
    assert_eq!(e.stats.last_id, 7);
}

#[test]
fn hidx_for_value_encoding() {
    let values: Vec<CellValue> = (0..731).map(|i| CellValue::Int(i % 52 + 1)).collect();
    let o = EncodeOptions {
        encoding: EncodingRequest::Value { params: None },
        ..opts()
    };
    let e = round_trip(&values, LogicalType::Int64, &o);
    let h = parse_hash_index(e.hidx.as_deref().unwrap()).unwrap();
    assert_eq!(h.record_count, 52);
    assert_eq!(h.bin_count, 16);
    assert!(h.overflow.is_empty());
    assert_eq!(e.hidx.as_ref().unwrap().len(), 1133);
}

#[test]
fn empty_column() {
    for logical in [LogicalType::String, LogicalType::Int64, LogicalType::Decimal] {
        let e = round_trip(&[], logical, &opts());
        assert_eq!(e.stats.segment_count(), 1);
        assert_eq!(e.stats.rows, 0);
    }
}

#[test]
fn rejects_bad_input() {
    let text = vec![CellValue::Text("x".into())];
    assert!(matches!(
        encode_column(&text, LogicalType::Int64, &opts()),
        Err(Error::Unsupported { .. })
    ));
    let value = EncodeOptions {
        encoding: EncodingRequest::Value { params: None },
        ..opts()
    };
    assert!(encode_column(&text, LogicalType::String, &value).is_err());
    // doubles are value-encoded only when a power of ten makes them integers (engine rule)
    assert!(encode_column(&[CellValue::Real(1.0 / 3.0)], LogicalType::Double, &value).is_err());
    let real = encode_column(
        &[CellValue::Real(0.4), CellValue::Real(2.5)],
        LogicalType::Double,
        &value,
    )
    .unwrap();
    assert_eq!((real.stats.base_id, real.stats.magnitude), (1, 10.0));
    assert!(encode_column(&[], LogicalType::Binary, &opts()).is_err());
    let nul = vec![CellValue::Text("x\u{0}y".into())];
    assert!(matches!(
        encode_column(&nul, LogicalType::String, &opts()),
        Err(Error::Unsupported { .. })
    ));
    let forced = |base_id: i64, magnitude: f64| EncodeOptions {
        encoding: EncodingRequest::Value {
            params: Some(ValueParams { base_id, magnitude }),
        },
        ..opts()
    };
    let ints = vec![CellValue::Int(5), CellValue::Int(15)];
    assert!(
        encode_column(&ints, LogicalType::Int64, &forced(10, 1.0)).is_err(),
        "id below 3"
    );
    assert!(
        encode_column(&ints, LogicalType::Int64, &forced(0, 0.1)).is_err(),
        "not a multiple of 10"
    );
    assert!(encode_column(&ints, LogicalType::Int64, &forced(0, -1.0)).is_err());
    assert!(encode_column(&ints, LogicalType::Int64, &forced(0, f64::NAN)).is_err());
    assert!(encode_column(&ints, LogicalType::Int64, &forced(i64::MIN, 1.0)).is_err());
    let capped = EncodeOptions {
        max_rows: 1,
        ..opts()
    };
    assert!(matches!(
        encode_column(&ints, LogicalType::Int64, &capped),
        Err(Error::LimitExceeded(_))
    ));
    assert!(encode_row_number(2, &capped).is_err());
    for bad in [
        EncodeOptions {
            segment_rows: 0,
            ..opts()
        },
        EncodeOptions {
            segment_rows: 1 << 40,
            ..opts()
        },
        EncodeOptions {
            rle_min_run: 0,
            ..opts()
        },
    ] {
        assert!(encode_column(&ints, LogicalType::Int64, &bad).is_err());
    }
}

#[test]
fn random_columns_round_trip() {
    let mut rng = Rng(0x5eed);
    for case in 0..60 {
        let rows = rng.below(3000) as usize;
        let distinct = 1 + rng.below(if case % 3 == 0 { 5 } else { 2000 });
        let null_rate = rng.below(4);
        let run = 1 + rng.below(150) as usize;
        let mut values = Vec::with_capacity(rows);
        while values.len() < rows {
            let v = if rng.below(10) < null_rate {
                CellValue::Null
            } else {
                CellValue::Int(rng.below(distinct) as i64 * 10 - 7)
            };
            let n = 1 + rng.below(run as u64) as usize;
            values.extend(std::iter::repeat_n(v, n.min(rows - values.len())));
        }
        let o = EncodeOptions {
            segment_rows: 1 + rng.below(4000),
            encoding: if case % 2 == 0 {
                EncodingRequest::Hash
            } else {
                EncodingRequest::Value { params: None }
            },
            ..opts()
        };
        round_trip(&values, LogicalType::Int64, &o);
        let texts: Vec<CellValue> = values
            .iter()
            .map(|v| match v {
                CellValue::Int(n) => CellValue::Text(format!("t{n}é")),
                _ => CellValue::Null,
            })
            .collect();
        let o = EncodeOptions {
            string_compression: if case % 2 == 0 {
                StringCompression::WhenSmaller
            } else {
                StringCompression::Never
            },
            ..o
        };
        round_trip(
            &texts,
            LogicalType::String,
            &EncodeOptions {
                encoding: EncodingRequest::Auto,
                ..o
            },
        );
    }
}

/// Regression (G10, the empty DirectQuery partition of an engine-written hybrid table): empty
/// column partitions are written like the engine's. Hybrid segments: base id 0, bookmark bits
/// 1, statistics on the blank id with original minimum 2 (value encoding too). Row numbers: one
/// RLE entry with a count of 0, statistics on the blank id, first run value the next row.
#[test]
fn empty_partitions_match_the_engine() {
    let value = EncodeOptions {
        encoding: EncodingRequest::Value {
            params: Some(ValueParams {
                base_id: -2,
                magnitude: 1.0,
            }),
        },
        ..opts()
    };
    let hash = EncodeOptions {
        encoding: EncodingRequest::Hash,
        dictionary_seed: vec![CellValue::Int(7)],
        ..opts()
    };
    for o in [&value, &hash] {
        let e = encode_column(&[], LogicalType::Int64, o).unwrap();
        let seg = &parse_idfmeta(&e.idfmeta).unwrap().segments[0];
        assert_eq!((seg.records, seg.base_id), (0, 0));
        assert_eq!(seg.rle.unwrap().bookmark_bits, 1);
        let st = &seg.stats;
        assert_eq!(
            (st.min_data_id, st.max_data_id, st.original_min_segment_data_id),
            (2, 2, 2)
        );
    }
    let rn = bicraft_vertipaq::encode_row_number_from(58_304, 0, &opts()).unwrap();
    let seg = &parse_idfmeta(&rn.idfmeta).unwrap().segments[0];
    assert_eq!(seg.rle.unwrap().storage_used_size, 2);
    assert_eq!((seg.stats.min_data_id, seg.stats.max_data_id), (2, 2));
    assert_eq!(seg.first_run_value, 58_307);
    assert_eq!(&rn.idf[8..12], &u32::MAX.to_le_bytes());
}

// G17: `DictionaryStorage.Size` of hash-encoded long and real dictionaries follows the engine's
// in-memory hash table (64-byte bins, 6 local entries for 4-byte longs, 4 for 8-byte keys,
// grown past a 3/4 load). Expected sizes are the engine's, from images it wrote for our own
// synthetic models (docs/formats/vertipaq.md, "Dictionary size") for dictionaries of the same
// length without hash collisions (8-byte keys: each collision adds 12 bytes the formula leaves out).
#[test]
fn numeric_dictionary_sizes_follow_the_engine_hash_table() {
    let hash = EncodeOptions {
        encoding: EncodingRequest::Hash,
        ..opts()
    };
    let size = |values: Vec<CellValue>, logical: LogicalType| {
        round_trip(&values, logical, &hash).stats.dictionary_size
    };
    // 4-byte longs: (distinct values, engine size), around each growth of the table
    for (n, engine) in [
        (1u64, 1300u64),
        (69, 1572),
        (78, 2632),
        (139, 2876),
        (153, 4980),
        (287, 5516),
        (296, 9648),
        (564, 10720),
        (580, 18976),
        (1150, 21256),
        (1154, 37656),
        (2290, 42200),
        (2315, 75068),
        (20000, 604560),
    ] {
        let values = (0..n)
            .map(|i| CellValue::Int(i64::try_from(i).unwrap()))
            .collect();
        assert_eq!(size(values, LogicalType::Int64), Some(engine), "{n} 4-byte longs");
    }
    // 8-byte longs (multiples of 1 000 003 past 2^31)
    for (n, engine) in [(4000u64, 163_344u64), (10_000, 342_416), (15_000, 644_560)] {
        let values = (1..=n)
            .map(|i| CellValue::Int(i64::try_from(i * 1_000_003).unwrap()))
            .collect();
        assert_eq!(size(values, LogicalType::Int64), Some(engine), "{n} 8-byte longs");
    }
    // 8-byte longs (multiples of 10^10) and reals as the engine reports them after a load
    let wide = |n: i64| (0..n).map(|j| CellValue::Int(j * 10_000_000_000)).collect();
    assert_eq!(size(wide(10), LogicalType::Int64), Some(1376));
    assert_eq!(size(wide(96), LogicalType::Int64), Some(3088));
    assert_eq!(size(wide(97), LogicalType::Int64), Some(5144));
    let reals = |n: u32| (0..n).map(|j| CellValue::Real(f64::from(j) + 0.5)).collect();
    for (n, engine) in [(1, 1264), (6, 1304), (20, 1416), (49, 2672), (97, 5104)] {
        assert_eq!(size(reals(n), LogicalType::Double), Some(engine), "{n} reals");
    }
    let dates = (0..6)
        .map(|j| CellValue::DateTime(36_526.0 + f64::from(j)))
        .collect();
    assert_eq!(size(dates, LogicalType::DateTime), Some(1304));
}

// G17: string dictionaries. Engine figures after loading the PBIX bicraft wrote for the same
// values (docs/formats/vertipaq.md, "Dictionary size"); strings past the 6 local entries of their
// hash bin add 8 bytes each in the engine, which the formula leaves out.
#[test]
fn string_dictionary_sizes_follow_the_engine() {
    let strings = |n: u32, prefix: &str| -> Vec<CellValue> {
        (0..n).map(|j| CellValue::Text(format!("{prefix}{j}"))).collect()
    };
    let size = |values: Vec<CellValue>| {
        round_trip(&values, LogicalType::String, &opts())
            .stats
            .dictionary_size
            .unwrap()
    };
    // one plain page, no collision: the engine's 17 212 exactly
    assert_eq!(size(strings(10, "s")), 17_212);
    // one Huffman page (10 290 units): 17 072 + 800 + buffer 1 356 + 1 409; the engine has
    // 20 669 (4 collisions)
    let x = "x".repeat(100);
    assert_eq!(size(strings(100, &x)), 20_637);
    // the hash table doubles past 5.4 strings per bin: 256 bins for 1 382 strings, 512 for 1 383
    let units = |n: u32| -> u64 { (0..n).map(|j| format!("s{j}").len() as u64 + 1).sum() };
    assert_eq!(size(strings(1382, "s")), 17_072 + 8 * 1382 + 2 * units(1382));
    assert_eq!(
        size(strings(1383, "s")),
        17_072 + 16_384 + 8 * 1383 + 2 * units(1383)
    );
}
