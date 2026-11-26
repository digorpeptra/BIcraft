//! Synthetic VertiPaq files built from the layouts in `docs/formats/vertipaq.md`, with our
//! own data, decoded end to end.

use bicraft_vertipaq::bitpack;
use bicraft_vertipaq::dictionary::{CHARSET_MULTIPLE, CHARSET_SINGLE, DictionaryValues};
use bicraft_vertipaq::huffman::{BitWriter, HuffmanCode};
use bicraft_vertipaq::idfmeta::{DataObject, RleFacet, SegmentDataObject, SegmentStats, SubSegmentMeta};
use bicraft_vertipaq::{
    CellValue, ColumnEncoding, ColumnMeta, Compression, Error, InnerCompression, Limits, LogicalType,
    SegmentMeta, StorageType, decode_column, decode_column_with_limits, decode_data_ids, parse_dictionary,
    parse_hash_index, parse_idfmeta,
};

// ---------------------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------------------

fn stats(min: u32, max: u32, rows: u64, has_nulls: bool) -> SegmentStats {
    SegmentStats {
        distinct_states: 0,
        min_data_id: min,
        max_data_id: max,
        original_min_segment_data_id: 2,
        rle_sort_order: -1,
        row_count: rows,
        has_nulls: u8::from(has_nulls),
        rle_runs: 0,
        others_rle_runs: 0,
    }
}

/// One hybrid segment: `runs` are (data id, count) RLE runs; `None` marks a bit-packed run
/// whose values come from `packed` in order.
struct Hybrid {
    runs: Vec<(Option<u32>, u32)>,
    packed: Vec<u32>,
    width: u8,
    has_nulls: bool,
}

/// Builds the `.idf` and `.idfmeta` of a column made of hybrid segments.
fn hybrid_files(segments: &[Hybrid]) -> (Vec<u8>, Vec<u8>) {
    let mut idf = Vec::new();
    let mut metas = Vec::new();
    let mut objects = Vec::new();
    for seg in segments {
        let base = if seg.has_nulls {
            2
        } else {
            *seg.packed.iter().min().unwrap_or(&3)
        };
        let mut entries = Vec::new();
        let mut packed_so_far = 0u32;
        let mut rows = 0u64;
        let mut min = u32::MAX;
        let mut max = 0;
        for &(id, count) in &seg.runs {
            let value = match id {
                Some(v) => {
                    min = min.min(v);
                    max = max.max(v);
                    v
                }
                None => {
                    let v = u32::MAX - packed_so_far;
                    packed_so_far += count;
                    v
                }
            };
            entries.push((value, count));
            rows += u64::from(count);
        }
        for &v in &seg.packed {
            if v != 2 {
                min = min.min(v);
            }
            max = max.max(v);
        }
        // Writers allocate at least 16 entries; trailing ones are zero padding.
        let alloc = entries.len().max(16);
        let primary_at = idf.len() as u64;
        idf.extend_from_slice(&(alloc as u64).to_le_bytes());
        for k in 0..alloc {
            let (v, c) = entries.get(k).copied().unwrap_or((0, 0));
            idf.extend_from_slice(&v.to_le_bytes());
            idf.extend_from_slice(&c.to_le_bytes());
        }
        let rel: Vec<u32> = seg.packed.iter().map(|v| v - base).collect();
        let words = bitpack::pack(&rel, seg.width).unwrap();
        let sub_at = idf.len() as u64;
        idf.extend_from_slice(&((words.len() / 8) as u64).to_le_bytes());
        idf.extend_from_slice(&words);
        let first = match seg.runs.first() {
            Some((Some(v), _)) => *v,
            _ => seg.packed.first().copied().unwrap_or(0),
        };
        metas.push(SegmentMeta {
            records: rows,
            base_id: 1,
            compression_class: Compression::HybridRle(InnerCompression::NoSplit(seg.width)).class_id(),
            sub_compression_class: Compression::HybridRle(InnerCompression::NoSplit(seg.width))
                .sub_class_id(),
            rle: Some(RleFacet {
                bookmark_bits: 4,
                storage_alloc_size: 2 * alloc as u64,
                storage_used_size: 2 * entries.len() as u64,
                segment_needs_resizing: 0,
            }),
            first_run_value: first,
            stats: stats(min, max, rows, seg.has_nulls),
            sub_segment: Some(SubSegmentMeta {
                records: seg.packed.len() as u64,
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
    (idf, meta.to_bytes())
}

fn hash_header(algorithm: i32, entry: u32, locals: u32) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&algorithm.to_le_bytes());
    v.extend_from_slice(&entry.to_le_bytes());
    v.extend_from_slice(&64u32.to_le_bytes());
    v.extend_from_slice(&locals.to_le_bytes());
    v.extend_from_slice(&u64::MAX.to_le_bytes());
    v
}

fn long_dictionary(values: &[i64], element_size: u32) -> Vec<u8> {
    let mut d = 0i32.to_le_bytes().to_vec();
    d.extend(hash_header(-1, 8, 6));
    d.extend_from_slice(&(values.len() as u64).to_le_bytes());
    d.extend_from_slice(&element_size.to_le_bytes());
    for &v in values {
        if element_size == 4 {
            d.extend_from_slice(&(v as i32).to_le_bytes());
        } else {
            d.extend_from_slice(&v.to_le_bytes());
        }
    }
    d
}

fn real_dictionary(values: &[f64]) -> Vec<u8> {
    let mut d = 1i32.to_le_bytes().to_vec();
    d.extend(hash_header(-1, 16, 3));
    d.extend_from_slice(&(values.len() as u64).to_le_bytes());
    d.extend_from_slice(&8u32.to_le_bytes());
    for &v in values {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d
}

enum Page<'a> {
    Plain(&'a [&'a str]),
    /// Single character set: every char is `charset << 8 | low byte`.
    Single(&'a [&'a str], u8),
    Multiple(&'a [&'a str]),
}

fn string_dictionary(pages: &[Page<'_>]) -> Vec<u8> {
    let mut body = Vec::new();
    let mut handles = Vec::new();
    let mut total = 0u64;
    let mut longest = 0u64;
    for (p, page) in pages.iter().enumerate() {
        let strings = match page {
            Page::Plain(s) | Page::Single(s, _) | Page::Multiple(s) => *s,
        };
        body.extend_from_slice(&0u64.to_le_bytes()); // mask
        body.push(0); // contains nulls
        body.extend_from_slice(&total.to_le_bytes()); // start index
        body.extend_from_slice(&(strings.len() as u64).to_le_bytes());
        for s in strings {
            longest = longest.max(s.encode_utf16().count() as u64);
        }
        total += strings.len() as u64;
        match page {
            Page::Plain(strings) => {
                body.push(0);
                body.extend_from_slice(&[0xDD, 0xCC, 0xBB, 0xAA]);
                let mut chars: Vec<u16> = Vec::new();
                for s in *strings {
                    handles.push((chars.len() as u32, p as u32));
                    chars.extend(s.encode_utf16());
                    chars.push(0);
                }
                body.extend_from_slice(&0u64.to_le_bytes());
                body.extend_from_slice(&(chars.len() as u64).to_le_bytes());
                body.extend_from_slice(&(chars.len() as u64 * 2).to_le_bytes());
                for c in chars {
                    body.extend_from_slice(&c.to_le_bytes());
                }
            }
            Page::Single(strings, _) | Page::Multiple(strings) => {
                let charset = match page {
                    Page::Single(_, cs) => Some(*cs),
                    _ => None,
                };
                let encoded: Vec<Vec<u8>> = strings
                    .iter()
                    .map(|s| match charset {
                        Some(cs) => s
                            .encode_utf16()
                            .map(|u| {
                                assert_eq!((u >> 8) as u8, cs, "test string outside charset");
                                u as u8
                            })
                            .collect(),
                        None => s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect(),
                    })
                    .collect();
                // A complete code: every byte value gets 8 bits, except the most common
                // ones which get shorter codes (1, 2, 3, … bits) while Kraft stays at 1.
                let mut freq = [0u32; 256];
                for e in &encoded {
                    for &b in e {
                        freq[usize::from(b)] += 1;
                    }
                }
                let lengths = complete_lengths(&freq);
                let code = HuffmanCode::from_lengths(&lengths).unwrap();
                let mut w = BitWriter::new();
                for e in &encoded {
                    handles.push((w.bit_len() as u32, p as u32));
                    for &b in e {
                        let (c, l) = code.codeword(b).unwrap();
                        w.push(c, l);
                    }
                }
                let total_bits = w.bit_len() as u32;
                let buf = w.into_bytes();
                body.push(1);
                body.extend_from_slice(&[0xDD, 0xCC, 0xBB, 0xAA]);
                body.extend_from_slice(&total_bits.to_le_bytes());
                body.extend_from_slice(
                    &(if charset.is_some() {
                        CHARSET_SINGLE
                    } else {
                        CHARSET_MULTIPLE
                    })
                    .to_le_bytes(),
                );
                body.extend_from_slice(&(buf.len() as u64).to_le_bytes());
                if let Some(cs) = charset {
                    body.push(cs);
                }
                body.extend_from_slice(&12u32.to_le_bytes());
                body.extend_from_slice(&code.encode_array());
                body.extend_from_slice(&(buf.len() as u64).to_le_bytes());
                body.extend_from_slice(&buf);
            }
        }
        body.extend_from_slice(&[0xCD, 0xAB, 0xCD, 0xAB]);
    }
    let mut d = 2i32.to_le_bytes().to_vec();
    d.extend(hash_header(0, 8, 6));
    d.extend_from_slice(&total.to_le_bytes());
    d.push(1);
    d.extend_from_slice(&longest.to_le_bytes());
    d.extend_from_slice(&(pages.len() as u64).to_le_bytes());
    d.extend(body);
    d.extend_from_slice(&(handles.len() as u64).to_le_bytes());
    d.extend_from_slice(&8u32.to_le_bytes());
    for (o, p) in handles {
        d.extend_from_slice(&o.to_le_bytes());
        d.extend_from_slice(&p.to_le_bytes());
    }
    d
}

/// Complete code lengths: the 1st most frequent symbol 2 bits, then 3, 4, … up to 7,
/// remaining symbols share the rest of the code space.
fn complete_lengths(freq: &[u32; 256]) -> [u8; 256] {
    let mut order: Vec<usize> = (0..256).collect();
    order.sort_by_key(|&s| std::cmp::Reverse(freq[s]));
    // Kraft budget in units of 2^-15.
    let mut lengths = [0u8; 256];
    let mut budget: u32 = 1 << 15;
    for (rank, &s) in order.iter().enumerate() {
        let left = 256 - rank as u32 - 1;
        let mut l = (1 + rank).min(15) as u8;
        // Leave room for the remaining symbols at 15 bits.
        while l < 15 && budget - (1 << (15 - l)) < left {
            l += 1;
        }
        lengths[s] = l;
        budget -= 1 << (15 - l);
    }
    // Spend any leftover space by shortening codes (keeps decoding valid either way).
    lengths
}

fn hash_cells(d: &[u8], ids: &[u32]) -> Vec<CellValue> {
    let dict = parse_dictionary(d).unwrap();
    bicraft_vertipaq::ids_to_values(ids, Some(&dict), ColumnEncoding::hash(StorageType::Long)).unwrap()
}

// ---------------------------------------------------------------------------------------
// .idf / .idfmeta
// ---------------------------------------------------------------------------------------

#[test]
fn hybrid_rle_and_bit_packing() {
    let seg = Hybrid {
        runs: vec![(Some(5), 70), (None, 4), (Some(3), 64), (None, 3)],
        packed: vec![3, 4, 6, 9, 7, 7, 3],
        width: 3,
        has_nulls: false,
    };
    let (idf, meta) = hybrid_files(&[seg]);
    let m = parse_idfmeta(&meta).unwrap();
    assert_eq!(m.to_bytes(), meta, "idfmeta round trip");
    let ids = decode_data_ids(&idf, &m, &Limits::default()).unwrap();
    let mut expected = vec![5; 70];
    expected.extend([3, 4, 6, 9]);
    expected.extend(vec![3; 64]);
    expected.extend([7, 7, 3]);
    assert_eq!(ids, expected);
}

#[test]
fn several_segments_and_null_base() {
    let a = Hybrid {
        runs: vec![(None, 5)],
        packed: vec![3, 4, 5, 6, 7],
        width: 4,
        has_nulls: false,
    };
    // A segment with blanks packs relative to the null id 2.
    let b = Hybrid {
        runs: vec![(Some(2), 64), (None, 4)],
        packed: vec![2, 10, 2, 11],
        width: 4,
        has_nulls: true,
    };
    let (idf, meta) = hybrid_files(&[a, b]);
    let enc = ColumnEncoding::value(-2, 1.0, StorageType::Long);
    let cells = decode_column(&idf, &meta, None, enc).unwrap();
    assert_eq!(cells.len(), 73);
    assert_eq!(&cells[..5], &(1..=5).map(CellValue::Int).collect::<Vec<_>>()[..]);
    assert!(cells[5..69].iter().all(|c| *c == CellValue::Null));
    assert_eq!(
        &cells[69..],
        &[
            CellValue::Null,
            CellValue::Int(8),
            CellValue::Null,
            CellValue::Int(9)
        ]
    );
}

#[test]
fn row_number_column_is_xm123() {
    // RowNumber: a single bit-packed entry and an empty sub-segment; ids count up from 3.
    let meta = ColumnMeta {
        segments: vec![SegmentMeta {
            records: 10,
            base_id: 0,
            compression_class: 703_066,
            sub_compression_class: 703_067,
            rle: Some(RleFacet {
                bookmark_bits: 24,
                storage_alloc_size: 32,
                storage_used_size: 2,
                segment_needs_resizing: 0,
            }),
            first_run_value: 3,
            stats: stats(3, 12, 10, false),
            sub_segment: Some(SubSegmentMeta {
                records: 10,
                base_id: 0,
            }),
        }],
        data_objects: None,
    };
    let mut idf = 16u64.to_le_bytes().to_vec();
    idf.extend_from_slice(&u32::MAX.to_le_bytes());
    idf.extend_from_slice(&10u32.to_le_bytes());
    idf.extend(vec![0u8; 15 * 8]);
    idf.extend_from_slice(&0u64.to_le_bytes());
    let enc = ColumnEncoding::value(-3, 1.0, StorageType::Long);
    let cells = decode_column(&idf, &meta.to_bytes(), None, enc).unwrap();
    assert_eq!(cells, (0..10).map(CellValue::Int).collect::<Vec<_>>());
}

#[test]
fn plain_no_split_segments() {
    // System-table layout: NoSplit<32>, one block per segment, values stored as-is.
    let seg = |records: u64| SegmentMeta {
        records,
        base_id: 0,
        compression_class: 703_062,
        sub_compression_class: 1,
        rle: None,
        first_run_value: 0,
        stats: stats(2, 2, 0, false),
        sub_segment: None,
    };
    let first: Vec<u32> = (3..9).collect();
    let second = vec![0, 1, 2];
    let mut idf = Vec::new();
    let mut objects = Vec::new();
    for vals in [&first, &second] {
        let words = bitpack::pack(vals, 32).unwrap();
        objects.push(SegmentDataObject {
            primary: DataObject {
                offset: idf.len() as u64,
                words: (words.len() / 8) as u64,
            },
            sub: None,
        });
        idf.extend_from_slice(&((words.len() / 8) as u64).to_le_bytes());
        idf.extend(words);
    }
    let meta = ColumnMeta {
        segments: vec![seg(6), seg(3)],
        data_objects: Some(objects),
    };
    let bytes = meta.to_bytes();
    let parsed = parse_idfmeta(&bytes).unwrap();
    assert_eq!(parsed, meta);
    let ids = decode_data_ids(&idf, &parsed, &Limits::default()).unwrap();
    assert_eq!(ids, vec![3, 4, 5, 6, 7, 8, 0, 1, 2]);
    let raw = decode_column(&idf, &bytes, None, ColumnEncoding::raw()).unwrap();
    assert_eq!(raw[6], CellValue::Int(0));
}

#[test]
fn data_object_mismatch_is_an_error() {
    let seg = Hybrid {
        runs: vec![(Some(3), 4)],
        packed: vec![],
        width: 1,
        has_nulls: false,
    };
    let (idf, meta) = hybrid_files(&[seg]);
    let mut m = parse_idfmeta(&meta).unwrap();
    if let Some(objects) = m.data_objects.as_mut() {
        objects[0].primary.offset = 8;
    }
    assert!(matches!(
        decode_data_ids(&idf, &m, &Limits::default()),
        Err(Error::Malformed { .. })
    ));
}

#[test]
fn row_limit_applies_to_rle_runs() {
    let seg = Hybrid {
        runs: vec![(Some(3), u32::MAX)],
        packed: vec![],
        width: 1,
        has_nulls: false,
    };
    let (idf, meta) = hybrid_files(&[seg]);
    let enc = ColumnEncoding::value(0, 1.0, StorageType::Long);
    let small = Limits {
        max_rows: 1000,
        ..Limits::default()
    };
    assert!(matches!(
        decode_column_with_limits(&idf, &meta, None, enc, &small),
        Err(Error::LimitExceeded(_))
    ));
    assert!(matches!(
        decode_column(&idf, &meta, None, enc),
        Err(Error::LimitExceeded(_))
    ));
}

/// F2 (docs/fuzzing.md): one 8-byte RLE run of 2^26 rows used to decode to 67M cells
/// (~1.9 GB) under the default limits. Now the default row cap refuses it before allocating.
#[test]
fn rle_bomb_is_refused_by_default_limits() {
    let seg = Hybrid {
        runs: vec![(Some(3), 1 << 26)],
        packed: vec![],
        width: 1,
        has_nulls: false,
    };
    let (idf, meta) = hybrid_files(&[seg]);
    assert!(idf.len() + meta.len() < 1024, "a tiny input");
    let enc = ColumnEncoding::value(0, 1.0, StorageType::Long);
    let err = decode_column(&idf, &meta, None, enc).unwrap_err();
    assert!(matches!(err, Error::LimitExceeded(_)), "{err}");
    let m = parse_idfmeta(&meta).unwrap();
    assert!(matches!(
        decode_data_ids(&idf, &m, &Limits::default()),
        Err(Error::LimitExceeded(_))
    ));
}

/// `Limits::max_bytes` bounds the ids, the cells, and the text the cells copy.
#[test]
fn byte_budget_bounds_ids_cells_and_text() {
    let rows = 1000u32;
    let seg = Hybrid {
        runs: vec![(Some(3), rows)],
        packed: vec![],
        width: 1,
        has_nulls: false,
    };
    let (idf, meta) = hybrid_files(&[seg]);
    let m = parse_idfmeta(&meta).unwrap();
    let budget = |max_bytes: u64| Limits {
        max_bytes,
        ..Limits::default()
    };

    // Data ids: 4 bytes each.
    assert!(decode_data_ids(&idf, &m, &budget(4 * 1000)).is_ok());
    assert!(matches!(
        decode_data_ids(&idf, &m, &budget(4 * 1000 - 1)),
        Err(Error::LimitExceeded(_))
    ));

    // Cells: refused before the ids are decoded when ids + cells cannot fit.
    let value = ColumnEncoding::value(0, 1.0, StorageType::Long);
    let cell = std::mem::size_of::<CellValue>() as u64;
    let exact = 1000 * (4 + cell);
    assert_eq!(
        decode_column_with_limits(&idf, &meta, None, value, &budget(exact))
            .unwrap()
            .len(),
        1000
    );
    assert!(matches!(
        decode_column_with_limits(&idf, &meta, None, value, &budget(exact - 1)),
        Err(Error::LimitExceeded(_))
    ));

    // Text: each cell copies its dictionary string (here 1 000 bytes × 1 000 rows).
    let long = "x".repeat(1000);
    let dict = string_dictionary(&[Page::Plain(&[long.as_str()])]);
    let text = ColumnEncoding::hash(StorageType::String);
    let cells =
        decode_column_with_limits(&idf, &meta, Some(&dict), text, &budget(exact + 1_000_000)).unwrap();
    assert_eq!(cells.len(), 1000);
    assert_eq!(cells[999], CellValue::Text(long.clone()));
    assert!(matches!(
        decode_column_with_limits(&idf, &meta, Some(&dict), text, &budget(exact + 999_999)),
        Err(Error::LimitExceeded(_))
    ));
}

/// A segment that claims many rows but whose RLE entries cover few is rejected before the
/// claimed rows are allocated.
#[test]
fn claimed_rows_need_rle_entries() {
    let seg = Hybrid {
        runs: vec![(Some(3), 5)],
        packed: vec![],
        width: 1,
        has_nulls: false,
    };
    let (idf, meta) = hybrid_files(&[seg]);
    let mut m = parse_idfmeta(&meta).unwrap();
    m.segments[0].records = 1 << 24;
    let err = decode_data_ids(&idf, &m, &Limits::default()).unwrap_err();
    assert!(err.to_string().contains("cover 5 of 16777216 rows"), "{err}");
}

#[test]
fn rle_runs_must_cover_the_rows() {
    let seg = Hybrid {
        runs: vec![(Some(3), 10)],
        packed: vec![],
        width: 1,
        has_nulls: false,
    };
    let (idf, meta) = hybrid_files(&[seg]);
    let mut m = parse_idfmeta(&meta).unwrap();
    m.segments[0].records = 11;
    assert!(decode_data_ids(&idf, &m, &Limits::default()).is_err());
    // Bit-packed entry pointing past the stored values.
    let seg = Hybrid {
        runs: vec![(None, 3)],
        packed: vec![3, 4],
        width: 2,
        has_nulls: false,
    };
    let (idf, meta) = hybrid_files(&[seg]);
    let mut m = parse_idfmeta(&meta).unwrap();
    m.segments[0].sub_segment = Some(SubSegmentMeta {
        records: 2,
        base_id: 0,
    });
    assert!(decode_data_ids(&idf, &m, &Limits::default()).is_err());
}

// ---------------------------------------------------------------------------------------
// Value encoding
// ---------------------------------------------------------------------------------------

#[test]
fn value_encoding_base_and_magnitude() {
    let seg = Hybrid {
        runs: vec![(None, 4)],
        packed: vec![3, 4, 5, 10],
        width: 4,
        has_nulls: false,
    };
    let (idf, meta) = hybrid_files(&[seg]);
    // Decimal with magnitude 10^-4: value × 10 000 = (id + base) × 10 000.
    let dec =
        ColumnEncoding::value(-2, 0.0001, StorageType::Long).with_logical_type(LogicalType::from_code(10));
    let cells = decode_column(&idf, &meta, None, dec).unwrap();
    assert_eq!(
        cells,
        vec![
            CellValue::Decimal(10_000),
            CellValue::Decimal(20_000),
            CellValue::Decimal(30_000),
            CellValue::Decimal(80_000)
        ]
    );
    // Magnitude 0.1 on integers: multiples of ten.
    let tens = ColumnEncoding::value(100, 0.1, StorageType::Long);
    let cells = decode_column(&idf, &meta, None, tens).unwrap();
    assert_eq!(cells[0], CellValue::Int(1030));
    // Magnitude 4 on reals: a true division.
    let quarter = ColumnEncoding::value(-3, 4.0, StorageType::Real);
    let cells = decode_column(&idf, &meta, None, quarter).unwrap();
    assert_eq!(cells[1], CellValue::Real(0.25));
    // Invalid magnitudes are errors, not panics.
    let bad = ColumnEncoding::value(0, 0.0, StorageType::Long);
    assert!(decode_column(&idf, &meta, None, bad).is_err());
    let overflow = ColumnEncoding::value(i64::MAX, 1.0, StorageType::Long);
    assert!(matches!(
        decode_column(&idf, &meta, None, overflow),
        Err(Error::ValueOverflow { .. })
    ));
}

#[test]
fn encoding_from_metadata_rows() {
    let e = ColumnEncoding::from_dictionary_storage(2, 6, -3, 1.0).unwrap();
    assert_eq!(e, ColumnEncoding::value(-3, 1.0, StorageType::Long));
    let e = ColumnEncoding::from_dictionary_storage(1, 2, 2, 0.0).unwrap();
    assert_eq!(e, ColumnEncoding::hash(StorageType::String));
    let e = ColumnEncoding::from_dictionary_storage(0, 19, 0, 0.0).unwrap();
    assert_eq!(e, ColumnEncoding::raw());
    assert!(ColumnEncoding::from_dictionary_storage(1, 7, 0, 0.0).is_err());
    assert!(ColumnEncoding::from_dictionary_storage(5, 6, 0, 0.0).is_err());
}

// ---------------------------------------------------------------------------------------
// Dictionaries
// ---------------------------------------------------------------------------------------

#[test]
fn numeric_dictionaries() {
    let d32 = long_dictionary(&[10, -20, 30], 4);
    assert_eq!(
        hash_cells(&d32, &[3, 5, 2, 4]),
        vec![
            CellValue::Int(10),
            CellValue::Int(30),
            CellValue::Null,
            CellValue::Int(-20)
        ]
    );
    let d64 = long_dictionary(&[i64::MIN, 1 << 40], 8);
    assert_eq!(hash_cells(&d64, &[4]), vec![CellValue::Int(1 << 40)]);
    let real = real_dictionary(&[45_000.5, -1.25]);
    let dict = parse_dictionary(&real).unwrap();
    assert_eq!(dict.values, DictionaryValues::Real(vec![45_000.5, -1.25]));
    let dates = bicraft_vertipaq::ids_to_values(
        &[3],
        Some(&dict),
        ColumnEncoding::hash(StorageType::Real).with_logical_type(LogicalType::from_code(9)),
    )
    .unwrap();
    assert_eq!(dates, vec![CellValue::DateTime(45_000.5)]);
    // Booleans are an int64 dictionary of 0/1.
    let bools = bicraft_vertipaq::ids_to_values(
        &[3, 4],
        Some(&parse_dictionary(&long_dictionary(&[0, 1], 4)).unwrap()),
        ColumnEncoding::hash(StorageType::Long).with_logical_type(LogicalType::from_code(11)),
    )
    .unwrap();
    assert_eq!(bools, vec![CellValue::Bool(false), CellValue::Bool(true)]);
    // Out-of-range ids are errors.
    let dict = parse_dictionary(&d32).unwrap();
    assert!(matches!(
        bicraft_vertipaq::ids_to_values(&[6], Some(&dict), ColumnEncoding::hash(StorageType::Long)),
        Err(Error::DataIdOutOfRange { id: 6, len: 3 })
    ));
}

#[test]
fn uncompressed_string_pages() {
    let d = string_dictionary(&[Page::Plain(&["alpha", "", "Ünïcødé ✓"]), Page::Plain(&["z"])]);
    let dict = parse_dictionary(&d).unwrap();
    assert_eq!(
        dict.values,
        DictionaryValues::Text(vec!["alpha".into(), "".into(), "Ünïcødé ✓".into(), "z".into()])
    );
    let store = dict.string_store.unwrap();
    assert_eq!(store.pages.len(), 2);
    assert_eq!(store.pages[1].start_index, 3);
}

#[test]
fn huffman_string_pages() {
    let latin = ["Female", "Male", "", "Fee", "aaaaaaaaaaaaaaaaaaaaaaaaaaaa"];
    let cyrillic = ["Привет", "мир"];
    let mixed = ["mixed Ω and ascii", "日本語"];
    let d = string_dictionary(&[
        Page::Single(&latin, 0),
        Page::Single(&cyrillic, 0x04),
        Page::Multiple(&mixed),
        Page::Plain(&["plain"]),
    ]);
    let dict = parse_dictionary(&d).unwrap();
    let mut expected: Vec<String> = latin.iter().map(|s| s.to_string()).collect();
    expected.extend(cyrillic.iter().map(|s| s.to_string()));
    expected.extend(mixed.iter().map(|s| s.to_string()));
    expected.push("plain".into());
    assert_eq!(dict.values, DictionaryValues::Text(expected));
    // End to end through a column.
    let seg = Hybrid {
        runs: vec![(None, 3), (Some(2), 64)],
        packed: vec![3, 4, 12],
        width: 4,
        has_nulls: true,
    };
    let (idf, meta) = hybrid_files(&[seg]);
    let cells = decode_column(&idf, &meta, Some(&d), ColumnEncoding::hash(StorageType::String)).unwrap();
    assert_eq!(cells[0], CellValue::Text("Female".into()));
    assert_eq!(cells[1], CellValue::Text("Male".into()));
    assert_eq!(cells[2], CellValue::Text("plain".into()));
    assert_eq!(cells[3], CellValue::Null);
    assert!(matches!(
        decode_column(&idf, &meta, None, ColumnEncoding::hash(StorageType::String)),
        Err(Error::MissingDictionary)
    ));
}

#[test]
fn string_dictionary_inconsistencies_are_errors() {
    let d = string_dictionary(&[Page::Plain(&["a", "b"])]);
    // Handle count must equal the string count (field right after the page layout).
    let mut bad = d.clone();
    let count_at = 4 + 24;
    bad[count_at] = 3;
    assert!(parse_dictionary(&bad).is_err());
    // A handle naming a missing page.
    let mut bad = d.clone();
    let last = bad.len() - 1;
    bad[last] = 9;
    assert!(parse_dictionary(&bad).is_err());
    // Begin mark.
    let mut bad = d;
    let mark = bad
        .windows(4)
        .position(|w| w == [0xDD, 0xCC, 0xBB, 0xAA])
        .unwrap();
    bad[mark] = 0;
    assert!(parse_dictionary(&bad).is_err());
}

// ---------------------------------------------------------------------------------------
// .hidx
// ---------------------------------------------------------------------------------------

fn hidx(entries: &[(u32, u32)], bins: usize, locals: u32, stats: bool) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&(-1i32).to_le_bytes());
    v.extend_from_slice(&8u32.to_le_bytes());
    v.extend_from_slice(&(16 + 8 * locals).to_le_bytes());
    v.extend_from_slice(&locals.to_le_bytes());
    v.extend_from_slice(&(bins as u64).to_le_bytes());
    v.extend_from_slice(&(entries.len() as i64).to_le_bytes());
    v.extend_from_slice(&((bins as i64) - 1).to_le_bytes());
    v.push(u8::from(stats));
    if stats {
        for x in [entries.len() as u64, bins as u64, 0, 0, u64::from(locals), 0, 2] {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v.extend_from_slice(&4u32.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&2u32.to_le_bytes());
    }
    // A bin's count covers all its entries; those beyond its local ones go to the overflow
    // list, bin by bin (the engine's layout).
    let mut table: Vec<Vec<(u32, u32)>> = vec![Vec::new(); bins];
    for &(h, k) in entries {
        table[(h as usize) % bins].push((h, k));
    }
    let overflow: Vec<(u32, u32)> = table
        .iter()
        .flat_map(|b| b.iter().skip(locals as usize).copied())
        .collect();
    for b in &table {
        v.extend_from_slice(&0u64.to_le_bytes());
        v.extend_from_slice(&(b.len() as u32).to_le_bytes());
        for k in 0..locals as usize {
            let (h, key) = b.get(k).copied().unwrap_or((0, 0));
            v.extend_from_slice(&h.to_le_bytes());
            v.extend_from_slice(&key.to_le_bytes());
        }
        v.extend_from_slice(&0u32.to_le_bytes());
    }
    v.extend_from_slice(&(overflow.len() as u64).to_le_bytes());
    for (h, k) in overflow {
        v.extend_from_slice(&h.to_le_bytes());
        v.extend_from_slice(&k.to_le_bytes());
    }
    v
}

#[test]
fn hash_index_parses_and_validates() {
    let entries: Vec<(u32, u32)> = (0..20).map(|i| (i * 16 + 1, i + 3)).collect();
    let bytes = hidx(&entries, 16, 6, true);
    let h = parse_hash_index(&bytes).unwrap();
    assert_eq!(h.record_count, 20);
    assert_eq!(h.overflow.len(), 14, "all hashes land in bin 1");
    assert_eq!(h.stats.as_ref().unwrap().histogram, vec![1, 2]);
    let mut keys: Vec<u32> = h.used_entries().map(|e| e.key).collect();
    keys.sort_unstable();
    assert_eq!(keys, (3..23).collect::<Vec<_>>());
    let plain = hidx(&entries[..3], 4, 2, false);
    assert!(parse_hash_index(&plain).unwrap().stats.is_none());
    // Wrong record count.
    let mut bad = bytes.clone();
    bad[24] = 21;
    assert!(parse_hash_index(&bad).is_err());
}

// ---------------------------------------------------------------------------------------
// Hostile input: every truncation and many corruptions fail cleanly.
// ---------------------------------------------------------------------------------------

#[test]
fn truncated_and_corrupted_files_never_panic() {
    let seg = Hybrid {
        runs: vec![(Some(4), 64), (None, 5)],
        packed: vec![3, 4, 5, 6, 7],
        width: 3,
        has_nulls: false,
    };
    let (idf, meta) = hybrid_files(&[seg]);
    let dict = string_dictionary(&[
        Page::Single(&["one", "two", "three"], 0),
        Page::Plain(&["four", "five"]),
    ]);
    let entries: Vec<(u32, u32)> = (0..9).map(|i| (i * 7, i + 3)).collect();
    let hx = hidx(&entries, 4, 2, true);
    let longs = long_dictionary(&[1, 2, 3], 8);
    let enc = ColumnEncoding::hash(StorageType::String);

    assert!(decode_column(&idf, &meta, Some(&dict), enc).is_ok());
    for n in 0..meta.len() {
        // The only valid prefix is the one that stops before the optional SDOs block.
        match parse_idfmeta(&meta[..n]) {
            Err(_) => {}
            Ok(m) => assert!(m.data_objects.is_none(), "idfmeta prefix {n}"),
        }
    }
    for n in 0..idf.len() {
        assert!(
            decode_column(&idf[..n], &meta, Some(&dict), enc).is_err(),
            "idf prefix {n}"
        );
    }
    for n in 0..dict.len() {
        assert!(parse_dictionary(&dict[..n]).is_err(), "dictionary prefix {n}");
    }
    for n in 0..hx.len() {
        assert!(parse_hash_index(&hx[..n]).is_err(), "hidx prefix {n}");
    }
    for n in 0..longs.len() {
        assert!(
            parse_dictionary(&longs[..n]).is_err(),
            "long dictionary prefix {n}"
        );
    }
    // Byte corruption: any outcome but a panic is fine.
    for (name, file) in [("idf", &idf), ("meta", &meta), ("dict", &dict), ("hidx", &hx)] {
        for i in 0..file.len() {
            for x in [0x01u8, 0x80, 0xFF] {
                let mut bad = file.clone();
                bad[i] ^= x;
                let _ = match name {
                    "idf" => decode_column(&bad, &meta, Some(&dict), enc).map(|_| ()),
                    "meta" => decode_column(&idf, &bad, Some(&dict), enc).map(|_| ()),
                    "dict" => decode_column(&idf, &meta, Some(&bad), enc).map(|_| ()),
                    _ => parse_hash_index(&bad).map(|_| ()),
                };
            }
        }
    }
}

#[test]
fn huge_counts_are_rejected_without_allocation() {
    // idfmeta claiming 2^60 segments.
    let mut meta = b"<1:CP\0".to_vec();
    meta.extend_from_slice(&(1u64 << 60).to_le_bytes());
    assert!(parse_idfmeta(&meta).is_err());
    // Dictionary claiming 2^62 values.
    let mut d = 0i32.to_le_bytes().to_vec();
    d.extend(hash_header(-1, 8, 6));
    d.extend_from_slice(&(1u64 << 62).to_le_bytes());
    d.extend_from_slice(&8u32.to_le_bytes());
    assert!(parse_dictionary(&d).is_err());
    // String dictionary claiming 2^40 pages.
    let mut s = 2i32.to_le_bytes().to_vec();
    s.extend(hash_header(0, 8, 6));
    s.extend_from_slice(&1u64.to_le_bytes());
    s.push(0);
    s.extend_from_slice(&1u64.to_le_bytes());
    s.extend_from_slice(&(1u64 << 40).to_le_bytes());
    assert!(parse_dictionary(&s).is_err());
    // A hash index claiming 2^50 bins.
    let mut h = hidx(&[(1, 3)], 1, 1, false);
    h[16..24].copy_from_slice(&(1u64 << 50).to_le_bytes());
    assert!(parse_hash_index(&h).is_err());
}

#[test]
fn duplicate_handles_cannot_amplify_output() {
    // Many record handles pointing at one long string must hit the output bound.
    let long: String = "x".repeat(4000);
    let mut d = string_dictionary(&[Page::Plain(&[long.as_str()])]);
    // Rewrite the handle vector: 100 000 handles to offset 0 on page 0.
    let handles_at = d.len() - 12 - 8;
    d.truncate(handles_at);
    let n = 100_000u64;
    d.extend_from_slice(&n.to_le_bytes());
    d.extend_from_slice(&8u32.to_le_bytes());
    for _ in 0..n {
        d.extend_from_slice(&[0u8; 8]);
    }
    // Also patch the store string count so counts agree.
    d[28..36].copy_from_slice(&n.to_le_bytes());
    assert!(matches!(parse_dictionary(&d), Err(Error::LimitExceeded(_))));
}

/// Regression (G10, engine-written): a hash-encoded column holding only blanks has no
/// `.dictionary` (`DictionaryStorage.StorageFileID` 0, `LastId` 2). It decodes to blanks; any
/// other id still needs the dictionary.
#[test]
fn all_blank_hash_column_without_dictionary() {
    let seg = Hybrid {
        runs: vec![(Some(2), 50)],
        packed: vec![],
        width: 1,
        has_nulls: true,
    };
    let (idf, meta) = hybrid_files(&[seg]);
    for storage in [StorageType::Long, StorageType::Real, StorageType::String] {
        let cells = decode_column(&idf, &meta, None, ColumnEncoding::hash(storage)).unwrap();
        assert_eq!(cells, vec![CellValue::Null; 50]);
    }
    assert_eq!(
        bicraft_vertipaq::ids_to_values(&[2, 2], None, ColumnEncoding::hash(StorageType::Long)),
        Ok(vec![CellValue::Null, CellValue::Null])
    );
    assert!(matches!(
        bicraft_vertipaq::ids_to_values(&[2, 3], None, ColumnEncoding::hash(StorageType::Long)),
        Err(Error::MissingDictionary)
    ));
}

/// Regression (G10, engine-written `REPT("z", 4000) & …` strings): a page whose dominant symbol
/// has a 1-bit code decodes to 8 characters per compressed byte; the input bound allowed 4.
#[test]
fn one_bit_codes_decode_eight_symbols_per_byte() {
    let long = "z".repeat(40_000);
    let strings = [long.as_str(), "y", "zy"];
    let d = string_dictionary(&[Page::Single(&strings, 0)]);
    let dict = parse_dictionary(&d).unwrap();
    assert_eq!(
        dict.values,
        DictionaryValues::Text(strings.iter().map(|s| s.to_string()).collect())
    );
}
