//! Varints, records with minimal serial types, and SQLite's value ordering for index keys.

use std::cmp::Ordering;

use crate::header::TextEncoding;
use crate::record::Value;
use crate::{Error, Result};

/// The largest record payload the writer produces (SQLite's default `SQLITE_MAX_LENGTH`).
pub(crate) const MAX_PAYLOAD: usize = 1_000_000_000;

/// Number of bytes the varint encoding of `v` takes (1 to 9).
pub(crate) fn varint_len(v: u64) -> usize {
    if v > 0x00ff_ffff_ffff_ffff {
        return 9;
    }
    let mut n = 1;
    let mut x = v >> 7;
    while x != 0 {
        n += 1;
        x >>= 7;
    }
    n
}

/// Append the SQLite varint encoding of `v`: big-endian groups of 7 bits with the high bit set
/// on every byte but the last; a ninth byte, when needed, carries 8 bits.
pub(crate) fn put_varint(out: &mut Vec<u8>, v: u64) {
    if v > 0x00ff_ffff_ffff_ffff {
        let mut buf = [0u8; 9];
        buf[8] = (v & 0xff) as u8;
        let mut x = v >> 8;
        for b in buf[..8].iter_mut().rev() {
            *b = ((x & 0x7f) as u8) | 0x80;
            x >>= 7;
        }
        out.extend_from_slice(&buf);
        return;
    }
    let n = varint_len(v);
    for i in (0..n).rev() {
        let group = ((v >> (7 * i)) & 0x7f) as u8;
        out.push(if i == 0 { group } else { group | 0x80 });
    }
}

/// The integer SQLite stores for a REAL value in a column with numeric affinity: the value when
/// it is integral and strictly inside the 64-bit range, as `sqlite3VdbeIntegerAffinity` does.
pub(crate) fn real_as_int(r: f64) -> Option<i64> {
    let ix = r as i64; // saturating; NaN gives 0, which never equals NaN
    (ix as f64 == r && ix != i64::MIN && ix != i64::MAX).then_some(ix)
}

/// Encode text in the database encoding.
pub(crate) fn encode_text(s: &str, enc: TextEncoding) -> Vec<u8> {
    match enc {
        TextEncoding::Utf8 => s.as_bytes().to_vec(),
        TextEncoding::Utf16Le => s.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        TextEncoding::Utf16Be => s.encode_utf16().flat_map(u16::to_be_bytes).collect(),
    }
}

/// One value of a record, with whether a REAL is stored as an integer (REAL affinity column).
pub(crate) struct Field<'a> {
    pub value: &'a Value,
    pub int_real: bool,
}

fn int_serial(i: i64, format4: bool) -> (u64, usize) {
    match i {
        0 if format4 => (8, 0),
        1 if format4 => (9, 0),
        -0x80..=0x7f => (1, 1),
        -0x8000..=0x7fff => (2, 2),
        -0x80_0000..=0x7f_ffff => (3, 3),
        -0x8000_0000..=0x7fff_ffff => (4, 4),
        -0x8000_0000_0000..=0x7fff_ffff_ffff => (5, 6),
        _ => (6, 8),
    }
}

/// Encode a record: a header of serial types followed by the values, each in its smallest form.
/// Serial types 8 and 9 (the constants 0 and 1) are used only with schema format 4.
pub(crate) fn encode_record(fields: &[Field<'_>], enc: TextEncoding, format4: bool) -> Result<Vec<u8>> {
    let mut header = Vec::new();
    let mut body = Vec::new();
    for f in fields {
        match f.value {
            Value::Null => put_varint(&mut header, 0),
            Value::Integer(i) => put_int(&mut header, &mut body, *i, format4),
            Value::Real(r) => match real_as_int(*r).filter(|_| f.int_real) {
                Some(i) => put_int(&mut header, &mut body, i, format4),
                None if r.is_nan() => put_varint(&mut header, 0),
                None => {
                    put_varint(&mut header, 7);
                    body.extend_from_slice(&r.to_bits().to_be_bytes());
                }
            },
            Value::Text(s) => {
                let bytes = encode_text(s, enc);
                put_varint(&mut header, serial_for_len(bytes.len(), 13)?);
                body.extend_from_slice(&bytes);
            }
            Value::Blob(b) => {
                put_varint(&mut header, serial_for_len(b.len(), 12)?);
                body.extend_from_slice(b);
            }
        }
        if body.len() > MAX_PAYLOAD {
            return Err(too_big(body.len()));
        }
    }
    // The header size counts its own varint.
    let mut hs = header.len() + 1;
    while header.len() + varint_len(hs as u64) != hs {
        hs = header.len() + varint_len(hs as u64);
    }
    let total = hs + body.len();
    if total > MAX_PAYLOAD {
        return Err(too_big(total));
    }
    let mut out = Vec::with_capacity(total);
    put_varint(&mut out, hs as u64);
    out.extend_from_slice(&header);
    out.extend_from_slice(&body);
    Ok(out)
}

fn too_big(n: usize) -> Error {
    Error::Invalid(format!(
        "record of {n} bytes exceeds the {MAX_PAYLOAD}-byte limit"
    ))
}

fn serial_for_len(len: usize, base: u64) -> Result<u64> {
    if len > MAX_PAYLOAD {
        return Err(too_big(len));
    }
    Ok(len as u64 * 2 + base)
}

fn put_int(header: &mut Vec<u8>, body: &mut Vec<u8>, i: i64, format4: bool) {
    let (serial, len) = int_serial(i, format4);
    put_varint(header, serial);
    let bytes = i.to_be_bytes();
    body.extend_from_slice(bytes.get(8 - len..).unwrap_or(&[]));
}

/// A collating sequence for text in index keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Collation {
    /// `memcmp` of the stored bytes (in the database encoding).
    Binary,
    /// ASCII case folding over UTF-8.
    NoCase,
    /// BINARY over UTF-8 after trailing spaces are removed.
    RTrim,
}

impl Collation {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        if name.eq_ignore_ascii_case("BINARY") {
            Some(Self::Binary)
        } else if name.eq_ignore_ascii_case("NOCASE") {
            Some(Self::NoCase)
        } else if name.eq_ignore_ascii_case("RTRIM") {
            Some(Self::RTrim)
        } else {
            None
        }
    }
}

fn class(v: &Value) -> u8 {
    match v {
        Value::Null => 0,
        Value::Integer(_) | Value::Real(_) => 1,
        Value::Text(_) => 2,
        Value::Blob(_) => 3,
    }
}

/// SQLite's integer/real comparison (`sqlite3IntFloatCompare`), exact over the whole i64 range.
fn int_float_cmp(i: i64, r: f64) -> Ordering {
    if r.is_nan() {
        return Ordering::Greater;
    }
    if r < -9_223_372_036_854_775_808.0 {
        return Ordering::Greater;
    }
    if r >= 9_223_372_036_854_775_808.0 {
        return Ordering::Less;
    }
    let y = r as i64;
    match i.cmp(&y) {
        Ordering::Equal => (i as f64).partial_cmp(&r).unwrap_or(Ordering::Equal),
        o => o,
    }
}

fn bytes_cmp(a: &[u8], b: &[u8]) -> Ordering {
    // memcmp over the common prefix, then the shorter first: exactly slice ordering.
    a.cmp(b)
}

fn text_cmp(a: &str, b: &str, coll: Collation, enc: TextEncoding) -> Ordering {
    match coll {
        Collation::Binary => match enc {
            TextEncoding::Utf8 => bytes_cmp(a.as_bytes(), b.as_bytes()),
            _ => bytes_cmp(&encode_text(a, enc), &encode_text(b, enc)),
        },
        Collation::NoCase => {
            let (x, y) = (a.as_bytes(), b.as_bytes());
            let n = x.len().min(y.len());
            for (p, q) in x.iter().zip(y.iter()).take(n) {
                let o = p.to_ascii_lowercase().cmp(&q.to_ascii_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
            }
            x.len().cmp(&y.len())
        }
        Collation::RTrim => bytes_cmp(
            a.trim_end_matches(' ').as_bytes(),
            b.trim_end_matches(' ').as_bytes(),
        ),
    }
}

/// Compare two values the way SQLite orders index keys: NULL < numbers < text < blobs, numbers
/// by value across INTEGER and REAL, text by collation, blobs by `memcmp`.
pub(crate) fn value_cmp(a: &Value, b: &Value, coll: Collation, enc: TextEncoding) -> Ordering {
    match (a, b) {
        (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
        (Value::Real(x), Value::Real(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::Integer(x), Value::Real(y)) => int_float_cmp(*x, *y),
        (Value::Real(x), Value::Integer(y)) => int_float_cmp(*y, *x).reverse(),
        (Value::Text(x), Value::Text(y)) => text_cmp(x, y, coll, enc),
        (Value::Blob(x), Value::Blob(y)) => bytes_cmp(x, y),
        _ => class(a).cmp(&class(b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{decode_record, read_varint};

    #[test]
    fn varint_round_trip() {
        for v in [
            0u64,
            1,
            127,
            128,
            16383,
            16384,
            (1 << 21) - 1,
            1 << 21,
            (1 << 56) - 1,
            1 << 56,
            u64::MAX,
            0x8000_0000_0000_0000,
        ] {
            let mut b = Vec::new();
            put_varint(&mut b, v);
            assert_eq!(b.len(), varint_len(v), "{v}");
            assert_eq!(read_varint(&b, 0).unwrap(), (v, b.len()), "{v}");
        }
    }

    #[test]
    fn minimal_serial_types() {
        let vals = [
            Value::Integer(0),
            Value::Integer(1),
            Value::Integer(-1),
            Value::Integer(200),
            Value::Integer(-40000),
            Value::Integer(1 << 23),
            Value::Integer(1 << 40),
            Value::Integer(i64::MIN),
            Value::Real(2.5),
            Value::Real(3.0),
            Value::Text("hé".into()),
            Value::Blob(vec![1, 2]),
            Value::Null,
        ];
        let fields: Vec<Field<'_>> = vals
            .iter()
            .enumerate()
            .map(|(i, value)| Field {
                value,
                int_real: i == 9,
            })
            .collect();
        let rec = encode_record(&fields, TextEncoding::Utf8, true).unwrap();
        assert_eq!(&rec[1..14], &[8, 9, 1, 2, 3, 4, 5, 6, 7, 1, 19, 16, 0]);
        let back = decode_record(&rec, TextEncoding::Utf8).unwrap();
        let mut want = vals.to_vec();
        want[9] = Value::Integer(3);
        assert_eq!(back, want);
        // Without schema format 4, 0 and 1 take a byte each.
        let rec = encode_record(&fields[..2], TextEncoding::Utf8, false).unwrap();
        assert_eq!(rec, vec![3, 1, 1, 0, 1]);
    }

    #[test]
    fn real_to_int_rule() {
        assert_eq!(real_as_int(3.0), Some(3));
        assert_eq!(real_as_int(-0.0), Some(0));
        assert_eq!(real_as_int(2.5), None);
        assert_eq!(real_as_int(f64::INFINITY), None);
        assert_eq!(real_as_int(f64::NAN), None);
        assert_eq!(real_as_int(9_223_372_036_854_775_808.0), None);
        assert_eq!(real_as_int(-9_223_372_036_854_775_808.0), None);
    }

    #[test]
    fn ordering() {
        let e = TextEncoding::Utf8;
        let b = Collation::Binary;
        assert_eq!(value_cmp(&Value::Null, &Value::Integer(-5), b, e), Ordering::Less);
        assert_eq!(
            value_cmp(&Value::Integer(2), &Value::Real(1.5), b, e),
            Ordering::Greater
        );
        assert_eq!(
            value_cmp(&Value::Integer(2), &Value::Real(2.0), b, e),
            Ordering::Equal
        );
        assert_eq!(
            value_cmp(&Value::Real(1e300), &Value::Text(String::new()), b, e),
            Ordering::Less
        );
        assert_eq!(
            value_cmp(&Value::Text("zz".into()), &Value::Blob(vec![]), b, e),
            Ordering::Less
        );
        assert_eq!(
            value_cmp(&Value::Integer(i64::MAX), &Value::Real(9.3e18), b, e),
            Ordering::Less
        );
        let (a, z) = (Value::Text("ABC".into()), Value::Text("abd".into()));
        assert_eq!(value_cmp(&a, &z, Collation::Binary, e), Ordering::Less);
        assert_eq!(
            value_cmp(
                &Value::Text("abc".into()),
                &Value::Text("ABC".into()),
                Collation::NoCase,
                e
            ),
            Ordering::Equal
        );
        assert_eq!(
            value_cmp(
                &Value::Text("a  ".into()),
                &Value::Text("a".into()),
                Collation::RTrim,
                e
            ),
            Ordering::Equal
        );
        // UTF-16LE BINARY order is byte order of the little-endian units.
        let (x, y) = (Value::Text("\u{0100}".into()), Value::Text("\u{0001}".into()));
        assert_eq!(value_cmp(&x, &y, b, TextEncoding::Utf16Le), Ordering::Less);
        assert_eq!(value_cmp(&x, &y, b, TextEncoding::Utf8), Ordering::Greater);
    }
}
