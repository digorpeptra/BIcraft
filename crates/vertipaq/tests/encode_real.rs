//! Re-encodes real Desktop-written columns (local only) and compares them byte for byte.
//!
//! `BICRAFT_VERTIPAQ_DIR` names a folder of extracted column files plus `manifest.json`, a
//! JSON array with one flat object per column:
//!
//! ```json
//! { "name": "c48", "idf": "c48.idf", "idfmeta": "c48.idfmeta",
//!   "dictionary": null, "hidx": "c48.hidx", "logical_type": 10, "row_number": false,
//!   "dictionary_type": 2, "data_type": 6, "base_id": 43699, "magnitude": 0.1 }
//! ```
//!
//! (`logical_type` is the TOM data type, `dictionary_type`/`data_type`/`base_id`/`magnitude`
//! the `DictionaryStorage` row.) The files come from client models: they stay outside the
//! repository and the test prints counts only. Unset variable: the test skips.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bicraft_vertipaq::{
    CellValue, ColumnEncoding, DictionaryValues, EncodeOptions, EncodingKind, EncodingRequest, LogicalType,
    ValueParams, convert, decode_column, encode_column, encode_row_number, parse_dictionary, parse_idfmeta,
};

#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
}

/// Parses the manifest: an array of flat objects with scalar values.
fn parse_manifest(text: &str) -> Vec<BTreeMap<String, Json>> {
    let b = text.as_bytes();
    let mut p = 0usize;
    let ws = |p: &mut usize| {
        while b.get(*p).is_some_and(u8::is_ascii_whitespace) {
            *p += 1;
        }
    };
    let expect = |p: &mut usize, c: u8| {
        ws(p);
        assert_eq!(b.get(*p), Some(&c), "manifest: expected {:?} at {p}", c as char);
        *p += 1;
    };
    let string = |p: &mut usize| -> String {
        expect(p, b'"');
        let mut out = String::new();
        loop {
            let c = text[*p..].chars().next().expect("manifest: unterminated string");
            *p += c.len_utf8();
            match c {
                '"' => return out,
                '\\' => {
                    let e = b[*p];
                    *p += 1;
                    match e {
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let code = u32::from_str_radix(&text[*p..*p + 4], 16).unwrap();
                            *p += 4;
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        other => out.push(other as char),
                    }
                }
                c => out.push(c),
            }
        }
    };
    let mut rows = Vec::new();
    expect(&mut p, b'[');
    ws(&mut p);
    if b.get(p) == Some(&b']') {
        return rows;
    }
    loop {
        expect(&mut p, b'{');
        let mut row = BTreeMap::new();
        loop {
            ws(&mut p);
            let key = string(&mut p);
            expect(&mut p, b':');
            ws(&mut p);
            let value = match b[p] {
                b'"' => Json::Str(string(&mut p)),
                b'n' => {
                    p += 4;
                    Json::Null
                }
                b't' => {
                    p += 4;
                    Json::Bool(true)
                }
                b'f' => {
                    p += 5;
                    Json::Bool(false)
                }
                _ => {
                    let start = p;
                    while b.get(p).is_some_and(|c| b"+-.eE0123456789".contains(c)) {
                        p += 1;
                    }
                    Json::Num(text[start..p].parse().expect("manifest: number"))
                }
            };
            row.insert(key, value);
            ws(&mut p);
            if b[p] == b',' {
                p += 1;
                continue;
            }
            expect(&mut p, b'}');
            break;
        }
        rows.push(row);
        ws(&mut p);
        if b[p] == b',' {
            p += 1;
            continue;
        }
        expect(&mut p, b']');
        return rows;
    }
}

fn num(row: &BTreeMap<String, Json>, k: &str) -> f64 {
    match row.get(k) {
        Some(Json::Num(n)) => *n,
        other => panic!("manifest: {k} is {other:?}"),
    }
}

fn file(dir: &Path, row: &BTreeMap<String, Json>, k: &str) -> Option<Vec<u8>> {
    match row.get(k) {
        Some(Json::Str(name)) => Some(std::fs::read(dir.join(name)).expect("manifest names a missing file")),
        _ => None,
    }
}

#[test]
fn parses_manifest_format() {
    let rows = parse_manifest(r#"[ {"a": "x\"y", "b": null, "c": -3, "d": 0.1, "e": true} ]"#);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["a"], Json::Str("x\"y".into()));
    assert_eq!(rows[0]["b"], Json::Null);
    assert_eq!(rows[0]["c"], Json::Num(-3.0));
    assert_eq!(rows[0]["e"], Json::Bool(true));
    assert!(parse_manifest("[]").is_empty());
}

#[test]
fn reencode_real_columns() {
    let Some(dir) = bicraft_env::var_os("BICRAFT_VERTIPAQ_DIR").map(PathBuf::from) else {
        eprintln!("note: BICRAFT_VERTIPAQ_DIR is not set; skipping the real-column re-encoding test");
        return;
    };
    let Ok(text) = std::fs::read_to_string(dir.join("manifest.json")) else {
        eprintln!("note: no manifest.json in BICRAFT_VERTIPAQ_DIR; skipping");
        return;
    };
    let rows = parse_manifest(&text);
    let (mut faithful_same, mut natural_same, mut user_columns, mut user_natural) = (0, 0, 0, 0);
    for row in &rows {
        let name = match row.get("name") {
            Some(Json::Str(s)) => s.clone(),
            _ => "?".into(),
        };
        let idf = file(&dir, row, "idf").expect("idf");
        let idfmeta = file(&dir, row, "idfmeta").expect("idfmeta");
        let dictionary = file(&dir, row, "dictionary");
        let hidx = file(&dir, row, "hidx");
        let logical = LogicalType::from_code(num(row, "logical_type") as i64).expect("logical type");
        let encoding = ColumnEncoding::from_dictionary_storage(
            num(row, "dictionary_type") as i64,
            num(row, "data_type") as i64,
            num(row, "base_id") as i64,
            num(row, "magnitude"),
        )
        .unwrap()
        .with_logical_type(Some(logical));
        let values = decode_column(&idf, &idfmeta, dictionary.as_deref(), encoding).unwrap();
        let meta = parse_idfmeta(&idfmeta).unwrap();
        let mut options = EncodeOptions {
            segment_base_id: meta.segments.first().map_or(1, |s| s.base_id),
            build_hidx: hidx.is_some(),
            ..EncodeOptions::default()
        };
        if row.get("row_number") == Some(&Json::Bool(true)) {
            let e = encode_row_number(values.len() as u64, &options).unwrap();
            assert_eq!(
                decode_column(&e.idf, &e.idfmeta, None, encoding).unwrap(),
                values,
                "{name}"
            );
            assert!(e.idf == idf && e.idfmeta == idfmeta, "{name}: RowNumber differs");
            faithful_same += 1;
            natural_same += 1;
            continue;
        }
        user_columns += 1;
        let hash = encoding.kind == EncodingKind::Hash;
        let compare = |options: &EncodeOptions| -> bool {
            let e = encode_column(&values, logical, options).unwrap();
            let back_encoding = ColumnEncoding::from_dictionary_storage(
                if hash { 1 } else { 2 },
                num(row, "data_type") as i64,
                e.stats.base_id,
                e.stats.magnitude,
            )
            .unwrap()
            .with_logical_type(Some(logical));
            let back = decode_column(&e.idf, &e.idfmeta, e.dictionary.as_deref(), back_encoding).unwrap();
            assert_eq!(back, values, "{name}: decode(encode(x)) != x");
            e.idf == idf && e.idfmeta == idfmeta && e.dictionary == dictionary && e.hidx == hidx
        };
        // Natural: only the encoding kind is given.
        options.encoding = if hash {
            EncodingRequest::Hash
        } else {
            EncodingRequest::Value { params: None }
        };
        if compare(&options) {
            natural_same += 1;
            user_natural += 1;
        }
        // Faithful: the original dictionary order and value parameters.
        if let Some(d) = &dictionary {
            let lt = Some(logical);
            options.dictionary_seed = match parse_dictionary(d).unwrap().values {
                DictionaryValues::Long(v) => v.into_iter().map(|x| convert(CellValue::Int(x), lt)).collect(),
                DictionaryValues::Real(v) => v.into_iter().map(|x| convert(CellValue::Real(x), lt)).collect(),
                DictionaryValues::Text(v) => v.into_iter().map(CellValue::Text).collect(),
            };
        } else {
            options.encoding = EncodingRequest::Value {
                params: Some(ValueParams {
                    base_id: num(row, "base_id") as i64,
                    magnitude: num(row, "magnitude"),
                }),
            };
        }
        assert!(
            compare(&options),
            "{name}: not byte-identical with the original dictionary order"
        );
        faithful_same += 1;
    }
    eprintln!(
        "re-encoded {} columns ({user_columns} non-RowNumber): byte-identical {faithful_same} with the \
         original dictionary order, {natural_same} with first-occurrence order ({user_natural} non-RowNumber)",
        rows.len()
    );
}
