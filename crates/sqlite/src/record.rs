//! Varints and the record format that encodes a row's values.

use crate::header::TextEncoding;
use crate::{Result, corrupt};

/// One SQLite value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// The integer, if this is an `Integer`.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    /// The number as a float, if this is an `Integer` or a `Real`.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Integer(i) => Some(*i as f64),
            Value::Real(r) => Some(*r),
            _ => None,
        }
    }

    /// The text, if this is `Text`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    /// The bytes, if this is a `Blob`.
    pub fn as_blob(&self) -> Option<&[u8]> {
        match self {
            Value::Blob(b) => Some(b),
            _ => None,
        }
    }
}

/// One row of a rowid table.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub rowid: i64,
    /// One value per declared column, in declaration order. An `INTEGER PRIMARY KEY` column
    /// holds the rowid. A record that carries more values than the table declares (which only a
    /// malformed file does) keeps the extra values at the end.
    pub values: Vec<Value>,
}

/// Read a varint at `at`; returns the value and the number of bytes used (1 to 9).
pub(crate) fn read_varint(b: &[u8], at: usize) -> Result<(u64, usize)> {
    let mut v: u64 = 0;
    for i in 0..9usize {
        let byte = *b
            .get(
                at.checked_add(i)
                    .ok_or_else(|| corrupt("varint offset overflow"))?,
            )
            .ok_or_else(|| corrupt("truncated varint"))?;
        if i == 8 {
            return Ok(((v << 8) | u64::from(byte), 9));
        }
        v = (v << 7) | u64::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            return Ok((v, i + 1));
        }
    }
    Err(corrupt("unterminated varint"))
}

/// Decode a record (the payload of a table b-tree leaf cell) into its values.
pub(crate) fn decode_record(payload: &[u8], enc: TextEncoding) -> Result<Vec<Value>> {
    let (header_size, mut hpos) = read_varint(payload, 0)?;
    let header_size = usize::try_from(header_size).map_err(|_| corrupt("record header size overflow"))?;
    if header_size < hpos || header_size > payload.len() {
        return Err(corrupt(format!(
            "record header size {header_size} outside a {}-byte payload",
            payload.len()
        )));
    }
    let header = payload
        .get(..header_size)
        .ok_or_else(|| corrupt("record header out of bounds"))?;
    let mut body = header_size;
    let mut values = Vec::new();
    while hpos < header_size {
        let (serial, n) = read_varint(header, hpos)?;
        hpos += n;
        let len = serial_len(serial)?;
        let end = body
            .checked_add(len)
            .filter(|&e| e <= payload.len())
            .ok_or_else(|| corrupt("record value runs past the payload"))?;
        let data = payload
            .get(body..end)
            .ok_or_else(|| corrupt("record value out of bounds"))?;
        values.push(decode_value(serial, data, enc)?);
        body = end;
    }
    Ok(values)
}

fn serial_len(serial: u64) -> Result<usize> {
    let len: u64 = match serial {
        0 | 8 | 9 => 0,
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 4,
        5 => 6,
        6 | 7 => 8,
        10 | 11 => return Err(corrupt(format!("reserved serial type {serial}"))),
        n => (n - 12) / 2,
    };
    usize::try_from(len).map_err(|_| corrupt("record value length overflow"))
}

fn decode_value(serial: u64, data: &[u8], enc: TextEncoding) -> Result<Value> {
    Ok(match serial {
        0 => Value::Null,
        1..=6 => Value::Integer(be_signed(data)),
        7 => {
            let mut a = [0u8; 8];
            let s = data.get(..8).ok_or_else(|| corrupt("short real"))?;
            a.copy_from_slice(s);
            Value::Real(f64::from_be_bytes(a))
        }
        8 => Value::Integer(0),
        9 => Value::Integer(1),
        n if n % 2 == 0 => Value::Blob(data.to_vec()),
        _ => Value::Text(decode_text(data, enc)),
    })
}

/// A big-endian two's-complement integer of 1 to 8 bytes.
fn be_signed(data: &[u8]) -> i64 {
    let negative = data.first().is_some_and(|b| b & 0x80 != 0);
    let mut v: u64 = if negative { u64::MAX } else { 0 };
    for &b in data {
        v = (v << 8) | u64::from(b);
    }
    v as i64
}

/// Decode stored text. Invalid sequences become U+FFFD (SQLite itself stores whatever bytes it
/// was given); a trailing odd byte of UTF-16 text is dropped.
pub(crate) fn decode_text(data: &[u8], enc: TextEncoding) -> String {
    match enc {
        TextEncoding::Utf8 => String::from_utf8_lossy(data).into_owned(),
        TextEncoding::Utf16Le => {
            let units: Vec<u16> = data
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect();
            String::from_utf16_lossy(&units)
        }
        TextEncoding::Utf16Be => {
            let units: Vec<u16> = data
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_be_bytes(*c))
                .collect();
            String::from_utf16_lossy(&units)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints() {
        assert_eq!(read_varint(&[0x00], 0).unwrap(), (0, 1));
        assert_eq!(read_varint(&[0x7f], 0).unwrap(), (127, 1));
        assert_eq!(read_varint(&[0x81, 0x00], 0).unwrap(), (128, 2));
        assert_eq!(read_varint(&[0xff; 9], 0).unwrap(), (u64::MAX, 9));
        assert!(read_varint(&[0x81], 0).is_err());
        assert!(read_varint(&[], 0).is_err());
        assert!(read_varint(&[0], usize::MAX).is_err());
    }

    #[test]
    fn signed_widths() {
        assert_eq!(be_signed(&[0xff]), -1);
        assert_eq!(be_signed(&[0x7f]), 127);
        assert_eq!(be_signed(&[0x80, 0x00, 0x00]), -(1 << 23));
        assert_eq!(be_signed(&[0x80, 0, 0, 0, 0, 0, 0, 0]), i64::MIN);
    }

    #[test]
    fn record_bounds() {
        // header claims more bytes than the payload has
        assert!(decode_record(&[0x05, 0x01], TextEncoding::Utf8).is_err());
        // value runs past the end
        assert!(decode_record(&[0x02, 0x06, 0x01], TextEncoding::Utf8).is_err());
        // reserved serial type
        assert!(decode_record(&[0x02, 0x0a], TextEncoding::Utf8).is_err());
        let v = decode_record(&[0x04, 0x01, 0x09, 0x0f, 0xfe, b'a'], TextEncoding::Utf8).unwrap();
        assert_eq!(
            v,
            vec![Value::Integer(-2), Value::Integer(1), Value::Text("a".into())]
        );
    }
}
