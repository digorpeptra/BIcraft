//! Column types of `COLMETADATA` (`TYPE_INFO`, [MS-TDS] 2.2.5.4–2.2.5.6) and the values of
//! `ROW`/`NBCROW` (`TYPE_VARBYTE`, 2.2.5.2), as M values (`RemoteType`, `RemoteValue`).
//!
//! The M types are those Power Query gives SQL Server columns (docs/formats/tds.md, *Types*):
//! integers keep their width, `decimal`/`numeric`/`money` are exact decimals, `real` is
//! single, `float` double, the character types text, `date`/`datetime*`/`time` their M kinds
//! (`datetimeoffset` a datetimezone), `uniqueidentifier` text, the binary types binary. Times
//! keep milliseconds (the evaluator's resolution): finer digits are dropped.

use std::io::Read;

use bicraft_data::connect::{ConnectError, ConnectErrorKind, RemoteType, RemoteValue};

use crate::wire::{Rd, protocol, utf16};

/// A column's type as `TYPE_INFO` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeInfo {
    /// The TDS type byte.
    pub code: u8,
    /// The declared length in bytes (fixed types: their size; `0xFFFF` with `plp`: `max`).
    pub len: u32,
    pub precision: u8,
    pub scale: u8,
    pub collation: Option<[u8; 5]>,
    /// Values are partially length-prefixed (`varchar(max)`, `nvarchar(max)`, `varbinary(max)`,
    /// `xml`, large UDTs).
    pub plp: bool,
}

// Fixed-length types.
const NULLTYPE: u8 = 0x1F;
const INT1: u8 = 0x30;
const BIT: u8 = 0x32;
const INT2: u8 = 0x34;
const INT4: u8 = 0x38;
const DATETIM4: u8 = 0x3A;
const FLT4: u8 = 0x3B;
const MONEY: u8 = 0x3C;
const DATETIME: u8 = 0x3D;
const FLT8: u8 = 0x3E;
const MONEY4: u8 = 0x7A;
const INT8: u8 = 0x7F;
// One-byte length types.
const GUID: u8 = 0x24;
const INTN: u8 = 0x26;
const DECIMAL: u8 = 0x37;
const NUMERIC: u8 = 0x3F;
const BITN: u8 = 0x68;
const DECIMALN: u8 = 0x6A;
const NUMERICN: u8 = 0x6C;
const FLTN: u8 = 0x6D;
const MONEYN: u8 = 0x6E;
const DATETIMN: u8 = 0x6F;
const DATEN: u8 = 0x28;
const TIMEN: u8 = 0x29;
const DATETIME2N: u8 = 0x2A;
const DATETIMEOFFSETN: u8 = 0x2B;
// Two-byte length types.
const BIGVARBIN: u8 = 0xA5;
const BIGVARCHR: u8 = 0xA7;
const BIGBINARY: u8 = 0xAD;
const BIGCHAR: u8 = 0xAF;
const NVARCHAR: u8 = 0xE7;
const NCHAR: u8 = 0xEF;
// Four-byte length types.
const IMAGE: u8 = 0x22;
const TEXT: u8 = 0x23;
const NTEXT: u8 = 0x63;
const XML: u8 = 0xF1;
const UDT: u8 = 0xF0;
const SSVARIANT: u8 = 0x62;

fn unsupported(message: impl Into<String>) -> ConnectError {
    ConnectError::new(ConnectErrorKind::DataSource, message.into())
}

/// The size of a fixed-length type.
fn fixed_len(code: u8) -> Option<u32> {
    Some(match code {
        NULLTYPE => 0,
        INT1 | BIT => 1,
        INT2 => 2,
        INT4 | DATETIM4 | FLT4 | MONEY4 => 4,
        MONEY | DATETIME | FLT8 | INT8 => 8,
        _ => return None,
    })
}

/// The bytes of a `time` value of scale `s`.
fn time_len(scale: u8) -> u8 {
    match scale {
        0..=2 => 3,
        3..=4 => 4,
        _ => 5,
    }
}

impl TypeInfo {
    fn of(code: u8) -> Self {
        Self {
            code,
            len: 0,
            precision: 0,
            scale: 0,
            collation: None,
            plp: false,
        }
    }

    /// Reads a `TYPE_INFO` (the type byte included).
    pub fn read<R: Read>(r: &mut Rd<R>) -> Result<Self, ConnectError> {
        let code = r.u8()?;
        let mut t = Self::of(code);
        if let Some(len) = fixed_len(code) {
            t.len = len;
            return Ok(t);
        }
        match code {
            GUID | INTN | BITN | FLTN | MONEYN | DATETIMN => t.len = u32::from(r.u8()?),
            DECIMAL | NUMERIC | DECIMALN | NUMERICN => {
                t.len = u32::from(r.u8()?);
                t.precision = r.u8()?;
                t.scale = r.u8()?;
                if !(1..=38).contains(&t.precision) || t.scale > t.precision || !(1..=17).contains(&t.len) {
                    return Err(protocol("a decimal column has an invalid precision"));
                }
            }
            DATEN => t.len = 3,
            TIMEN | DATETIME2N | DATETIMEOFFSETN => {
                t.scale = r.u8()?;
                if t.scale > 7 {
                    return Err(protocol("a time column has a scale above 7"));
                }
            }
            BIGVARBIN | BIGBINARY | BIGVARCHR | BIGCHAR | NVARCHAR | NCHAR => {
                t.len = u32::from(r.u16()?);
                t.plp = t.len == 0xFFFF;
                if t.plp && !matches!(code, BIGVARBIN | BIGVARCHR | NVARCHAR) {
                    return Err(protocol("a fixed-length column declares `max`"));
                }
                if matches!(code, BIGVARCHR | BIGCHAR | NVARCHAR | NCHAR) {
                    t.collation = Some(r.array()?);
                }
            }
            IMAGE | TEXT | NTEXT => {
                t.len = r.u32()?;
                if code != IMAGE {
                    t.collation = Some(r.array()?);
                }
            }
            XML => {
                t.plp = true;
                if r.u8()? != 0 {
                    r.b_varchar()?;
                    r.b_varchar()?;
                    r.us_varchar()?;
                }
            }
            UDT => {
                t.len = u32::from(r.u16()?);
                t.plp = t.len == 0xFFFF;
                r.b_varchar()?;
                r.b_varchar()?;
                r.b_varchar()?;
                r.us_varchar()?;
            }
            SSVARIANT => t.len = r.u32()?,
            other => return Err(protocol(format!("unknown column type 0x{other:02X}"))),
        }
        Ok(t)
    }

    /// Whether `COLMETADATA` names the table after this type (`text`, `ntext`, `image`).
    pub fn has_table_name(&self) -> bool {
        matches!(self.code, IMAGE | TEXT | NTEXT)
    }

    /// A character type read in a code page (`char`, `varchar`, `text`).
    pub fn is_ansi_text(&self) -> bool {
        matches!(self.code, BIGVARCHR | BIGCHAR | TEXT)
    }

    /// The M type of the column, and whether the server compares its values as M does (folds
    /// filters and sorts on it). `Err`: a type the connector does not read.
    pub fn remote(&self) -> Result<(RemoteType, bool), ConnectError> {
        Ok(match (self.code, self.len) {
            (NULLTYPE, _) => (RemoteType::Any, false),
            (INT1, _) | (INTN, 1) => (RemoteType::Int8, true),
            (INT2, _) | (INTN, 2) => (RemoteType::Int16, true),
            (INT4, _) | (INTN, 4) => (RemoteType::Int32, true),
            (INT8, _) | (INTN, 8) => (RemoteType::Int64, true),
            (BIT | BITN, _) => (RemoteType::Logical, true),
            (FLT4, _) | (FLTN, 4) => (RemoteType::Single, false),
            (FLT8, _) | (FLTN, 8) => (RemoteType::Double, true),
            (MONEY | MONEY4 | MONEYN | DECIMAL | NUMERIC | DECIMALN | NUMERICN, _) => {
                (RemoteType::Decimal, true)
            }
            (DATEN, _) => (RemoteType::Date, true),
            (DATETIME | DATETIM4 | DATETIMN | DATETIME2N, _) => (RemoteType::DateTime, true),
            (DATETIMEOFFSETN, _) => (RemoteType::DateTimeZone, false),
            (TIMEN, _) => (RemoteType::Time, false),
            (GUID | BIGVARCHR | BIGCHAR | NVARCHAR | NCHAR | TEXT | NTEXT | XML, _) => {
                (RemoteType::Text, false)
            }
            (BIGVARBIN | BIGBINARY | IMAGE | UDT, _) => (RemoteType::Binary, false),
            (SSVARIANT, _) => {
                return Err(unsupported(
                    "sql_variant columns are not read: CAST them in the query",
                ));
            }
            (code, len) => return Err(protocol(format!("type 0x{code:02X} of length {len} is invalid"))),
        })
    }

    /// Reads one value of this type.
    pub fn value<R: Read>(&self, r: &mut Rd<R>, cap: usize) -> Result<RemoteValue, ConnectError> {
        if let Some(n) = fixed_len(self.code) {
            if self.code == NULLTYPE {
                return Ok(RemoteValue::Null);
            }
            let b = r.bytes(u64::from(n), 8)?;
            return self.decode(&b);
        }
        match self.code {
            GUID | INTN | BITN | FLTN | MONEYN | DATETIMN | DECIMAL | NUMERIC | DECIMALN | NUMERICN
            | DATEN | TIMEN | DATETIME2N | DATETIMEOFFSETN => {
                let n = r.u8()?;
                if n == 0 {
                    return Ok(RemoteValue::Null);
                }
                let b = r.bytes(u64::from(n), 17)?;
                self.decode(&b)
            }
            _ if self.plp => match plp(r, cap)? {
                None => Ok(RemoteValue::Null),
                Some(b) => self.decode(&b),
            },
            BIGVARBIN | BIGBINARY | BIGVARCHR | BIGCHAR | NVARCHAR | NCHAR | UDT => {
                let n = r.u16()?;
                if n == 0xFFFF {
                    return Ok(RemoteValue::Null);
                }
                let b = r.bytes(u64::from(n), cap)?;
                self.decode(&b)
            }
            IMAGE | TEXT | NTEXT => {
                let ptr = r.u8()?;
                if ptr == 0 {
                    return Ok(RemoteValue::Null);
                }
                r.skip(u64::from(ptr) + 8)?;
                let n = r.u32()?;
                let b = r.bytes(u64::from(n), cap)?;
                self.decode(&b)
            }
            other => Err(protocol(format!("values of type 0x{other:02X} are not read"))),
        }
    }

    /// The value of the bytes `b` (not null).
    fn decode(&self, b: &[u8]) -> Result<RemoteValue, ConnectError> {
        let bad = || {
            protocol(format!(
                "a value of type 0x{:02X} has {} bytes",
                self.code,
                b.len()
            ))
        };
        Ok(match self.code {
            INT1 | INT2 | INT4 | INT8 | INTN => RemoteValue::Int(match *b {
                [x] => i64::from(x),
                [a, c] => i64::from(i16::from_le_bytes([a, c])),
                [a, c, d, e] => i64::from(i32::from_le_bytes([a, c, d, e])),
                [a, c, d, e, f, g, h, i] => i64::from_le_bytes([a, c, d, e, f, g, h, i]),
                _ => return Err(bad()),
            }),
            BIT | BITN => match *b {
                [x] => RemoteValue::Logical(x != 0),
                _ => return Err(bad()),
            },
            FLT4 | FLT8 | FLTN => RemoteValue::Double(match *b {
                [a, c, d, e] => f64::from(f32::from_le_bytes([a, c, d, e])),
                [a, c, d, e, f, g, h, i] => f64::from_le_bytes([a, c, d, e, f, g, h, i]),
                _ => return Err(bad()),
            }),
            MONEY | MONEY4 | MONEYN => {
                let units: i64 = match *b {
                    [a, c, d, e] => i64::from(i32::from_le_bytes([a, c, d, e])),
                    // the high 32 bits first, then the low ones
                    [a, c, d, e, f, g, h, i] => {
                        let hi = i64::from(i32::from_le_bytes([a, c, d, e]));
                        let lo = i64::from(u32::from_le_bytes([f, g, h, i]));
                        (hi << 32) | lo
                    }
                    _ => return Err(bad()),
                };
                RemoteValue::Decimal(scaled(i128::from(units), 4))
            }
            DECIMAL | NUMERIC | DECIMALN | NUMERICN => {
                let (sign, digits) = b.split_first().ok_or_else(bad)?;
                if digits.len() > 16 || digits.is_empty() {
                    return Err(bad());
                }
                let mut le = [0u8; 16];
                le.get_mut(..digits.len())
                    .ok_or_else(bad)?
                    .copy_from_slice(digits);
                let n = u128::from_le_bytes(le);
                if n >= 10u128.pow(38) {
                    return Err(protocol("a decimal value has more than 38 digits"));
                }
                // n < 10^38 < i128::MAX
                let v = i128::try_from(n).map_err(|_| bad())?;
                RemoteValue::Decimal(scaled(if *sign == 0 { -v } else { v }, u32::from(self.scale)))
            }
            DATETIME | DATETIM4 | DATETIMN => match *b {
                [a, c, d, e, f, g, h, i] => {
                    let days = i64::from(i32::from_le_bytes([a, c, d, e]));
                    let ticks = u64::from(u32::from_le_bytes([f, g, h, i]));
                    // 1/300 s, rounded to the millisecond
                    let ms = (ticks * 10 + 1) / 3;
                    if ms >= 86_400_000 {
                        return Err(protocol("a datetime value has an invalid time"));
                    }
                    datetime(DAYS_1900 + days, ms)?
                }
                [a, c, d, e] => {
                    let days = i64::from(u16::from_le_bytes([a, c]));
                    let minutes = u64::from(u16::from_le_bytes([d, e]));
                    if minutes >= 1440 {
                        return Err(protocol("a smalldatetime value has an invalid time"));
                    }
                    datetime(DAYS_1900 + days, minutes * 60_000)?
                }
                _ => return Err(bad()),
            },
            DATEN => match *b {
                [a, c, d] => {
                    let (year, month, day) = civil(DAYS_0001 + i64::from(u24([a, c, d])));
                    RemoteValue::Date { year, month, day }
                }
                _ => return Err(bad()),
            },
            TIMEN => RemoteValue::Time {
                millis: u32::try_from(time(b, self.scale).ok_or_else(bad)?).map_err(|_| bad())?,
            },
            DATETIME2N => {
                let t = time_len(self.scale) as usize;
                let (tb, db) = (b.get(..t).ok_or_else(bad)?, b.get(t..).ok_or_else(bad)?);
                let ms = time(tb, self.scale).ok_or_else(bad)?;
                let days = match *db {
                    [a, c, d] => DAYS_0001 + i64::from(u24([a, c, d])),
                    _ => return Err(bad()),
                };
                datetime(days, ms)?
            }
            DATETIMEOFFSETN => {
                let t = time_len(self.scale) as usize;
                let tb = b.get(..t).ok_or_else(bad)?;
                let ms = time(tb, self.scale).ok_or_else(bad)?;
                let (days, offset) = match *b.get(t..).ok_or_else(bad)? {
                    [a, c, d, e, f] => (DAYS_0001 + i64::from(u24([a, c, d])), i16::from_le_bytes([e, f])),
                    _ => return Err(bad()),
                };
                if !(-840..=840).contains(&offset) {
                    return Err(protocol("a datetimeoffset value has an invalid offset"));
                }
                // the value is UTC: the local time is UTC plus the offset
                let local =
                    days * 86_400_000 + i64::try_from(ms).map_err(|_| bad())? + i64::from(offset) * 60_000;
                let (year, month, day) = civil(local.div_euclid(86_400_000));
                RemoteValue::DateTimeZone {
                    year,
                    month,
                    day,
                    millis: u32::try_from(local.rem_euclid(86_400_000)).map_err(|_| bad())?,
                    offset_minutes: offset,
                }
            }
            GUID => match *b {
                [a0, a1, a2, a3, b0, b1, c0, c1, d0, d1, e0, e1, e2, e3, e4, e5] => {
                    RemoteValue::Text(format!(
                        "{:08x}-{:04x}-{:04x}-{d0:02x}{d1:02x}-{e0:02x}{e1:02x}{e2:02x}{e3:02x}{e4:02x}{e5:02x}",
                        u32::from_le_bytes([a0, a1, a2, a3]),
                        u16::from_le_bytes([b0, b1]),
                        u16::from_le_bytes([c0, c1]),
                    ))
                }
                _ => return Err(bad()),
            },
            NVARCHAR | NCHAR | NTEXT | XML => {
                if !b.len().is_multiple_of(2) {
                    return Err(bad());
                }
                RemoteValue::Text(utf16(b))
            }
            BIGVARCHR | BIGCHAR | TEXT => RemoteValue::Text(ansi(b, self.collation)?),
            BIGVARBIN | BIGBINARY | IMAGE | UDT => RemoteValue::Binary(b.to_vec()),
            other => return Err(protocol(format!("values of type 0x{other:02X} are not read"))),
        })
    }
}

/// A `PLP_BODY` ([MS-TDS] 2.2.5.2.3): `None` for null.
fn plp<R: Read>(r: &mut Rd<R>, cap: usize) -> Result<Option<Vec<u8>>, ConnectError> {
    const PLP_NULL: u64 = u64::MAX;
    const UNKNOWN: u64 = u64::MAX - 1;
    let total = r.u64()?;
    if total == PLP_NULL {
        return Ok(None);
    }
    if total != UNKNOWN && total > cap as u64 {
        return Err(protocol(format!(
            "a value of {total} bytes exceeds the {cap}-byte limit"
        )));
    }
    let mut out = Vec::new();
    loop {
        let n = r.u32()?;
        if n == 0 {
            break;
        }
        let room = cap.saturating_sub(out.len());
        let chunk = r.bytes(u64::from(n), room)?;
        out.extend_from_slice(&chunk);
    }
    if total != UNKNOWN && out.len() as u64 != total {
        return Err(protocol("a value's length does not match its chunks"));
    }
    Ok(Some(out))
}

/// Days from 1970-01-01 to 1900-01-01 and to 0001-01-01.
const DAYS_1900: i64 = -25_567;
const DAYS_0001: i64 = -719_162;

fn u24([a, b, c]: [u8; 3]) -> u32 {
    u32::from_le_bytes([a, b, c, 0])
}

/// Milliseconds since midnight of a `time` of scale `scale` (finer digits dropped).
fn time(b: &[u8], scale: u8) -> Option<u64> {
    if b.len() != usize::from(time_len(scale)) {
        return None;
    }
    let mut le = [0u8; 8];
    le.get_mut(..b.len())?.copy_from_slice(b);
    let units = u64::from_le_bytes(le);
    let per_ms = 10u64.pow(u32::from(scale));
    let ms = units.checked_mul(1000)? / per_ms;
    (ms < 86_400_000).then_some(ms)
}

fn datetime(days: i64, ms: u64) -> Result<RemoteValue, ConnectError> {
    let (year, month, day) = civil(days);
    Ok(RemoteValue::DateTime {
        year,
        month,
        day,
        millis: u32::try_from(ms).map_err(|_| protocol("a time of day is out of range"))?,
    })
}

/// The civil date of a day count from 1970-01-01 (proleptic Gregorian; H. Hinnant's
/// `civil_from_days`, public domain).
pub(crate) fn civil(z: i64) -> (i32, u8, u8) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (
        i32::try_from(y).unwrap_or(i32::MAX),
        u8::try_from(m).unwrap_or(1),
        u8::try_from(d).unwrap_or(1),
    )
}

/// `v / 10^scale` as exact decimal text.
fn scaled(v: i128, scale: u32) -> String {
    let neg = v < 0;
    let digits = v.unsigned_abs().to_string();
    let s = scale as usize;
    let text = if s == 0 {
        digits
    } else if digits.len() > s {
        let (a, b) = digits.split_at(digits.len() - s);
        format!("{a}.{b}")
    } else {
        format!("0.{}{digits}", "0".repeat(s - digits.len()))
    };
    if neg { format!("-{text}") } else { text }
}

/// The code page of a collation ([MS-TDS] 2.2.5.1.2): 65001 for UTF-8 collations, 1252 for the
/// Western European Windows collations and `SQL_Latin1_General_CP1_*`, else `None` (only ASCII
/// text is read then).
fn code_page(collation: Option<[u8; 5]>) -> Option<u32> {
    let [a, b, c, d, sort_id] = collation?;
    let info = u32::from_le_bytes([a, b, c, d]);
    if info & (1 << 26) != 0 {
        return Some(65001);
    }
    if sort_id != 0 {
        return matches!(sort_id, 51..=54).then_some(1252);
    }
    // the primary language of the LCID
    let western = [
        0x03, 0x06, 0x07, 0x09, 0x0A, 0x0B, 0x0C, 0x0F, 0x10, 0x13, 0x14, 0x16, 0x1D, 0x21, 0x2D, 0x36, 0x38,
        0x3E, 0x41, 0x56,
    ];
    western.contains(&(info & 0x3FF)).then_some(1252)
}

/// Windows-1252's characters 0x80–0x9F (the Unicode mapping of the code page).
const CP1252_HIGH: [u16; 32] = [
    0x20AC, 0xFFFD, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152,
    0xFFFD, 0x017D, 0xFFFD, 0xFFFD, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122,
    0x0161, 0x203A, 0x0153, 0xFFFD, 0x017E, 0x0178,
];

/// Text of a code-page column.
fn ansi(b: &[u8], collation: Option<[u8; 5]>) -> Result<String, ConnectError> {
    if b.is_ascii() {
        return Ok(b.iter().map(|&x| char::from(x)).collect());
    }
    match code_page(collation) {
        Some(65001) => Ok(String::from_utf8_lossy(b).into_owned()),
        Some(1252) => Ok(b
            .iter()
            .map(|&x| match x {
                0x80..=0x9F => CP1252_HIGH
                    .get(usize::from(x - 0x80))
                    .and_then(|&u| char::from_u32(u32::from(u)))
                    .unwrap_or('\u{FFFD}'),
                _ => char::from(x),
            })
            .collect()),
        _ => Err(unsupported(
            "a char/varchar value in a code page bicraft does not read (its collation is not Western \
             European or UTF-8): CAST the column to nvarchar in the query",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn val(t: &TypeInfo, bytes: &[u8]) -> RemoteValue {
        let mut r = Rd::new(bytes);
        t.value(&mut r, 1 << 20).unwrap()
    }

    fn ty(bytes: &[u8]) -> TypeInfo {
        TypeInfo::read(&mut Rd::new(bytes)).unwrap()
    }

    #[test]
    fn numbers() {
        assert_eq!(val(&ty(&[INT4]), &(-5i32).to_le_bytes()), RemoteValue::Int(-5));
        assert_eq!(
            val(&ty(&[INTN, 8]), &[8, 1, 0, 0, 0, 0, 0, 0, 0x80]),
            RemoteValue::Int(i64::MIN + 1)
        );
        assert_eq!(val(&ty(&[INTN, 1]), &[1, 0xFF]), RemoteValue::Int(255));
        assert_eq!(val(&ty(&[INTN, 4]), &[0]), RemoteValue::Null);
        assert_eq!(val(&ty(&[BITN, 1]), &[1, 1]), RemoteValue::Logical(true));
        assert_eq!(
            val(&ty(&[FLTN, 8]), &[&[8][..], &1.5f64.to_le_bytes()].concat()),
            RemoteValue::Double(1.5)
        );
        // decimal(10,2) -123.45
        let d = ty(&[DECIMALN, 5, 10, 2]);
        assert_eq!(
            val(&d, &[5, 0, 0x39, 0x30, 0, 0]),
            RemoteValue::Decimal("-123.45".into())
        );
        assert_eq!(val(&d, &[5, 1, 5, 0, 0, 0]), RemoteValue::Decimal("0.05".into()));
        assert_eq!(d.remote().unwrap(), (RemoteType::Decimal, true));
        // money 12.3456: 123456 units, high half first
        let mut m = vec![8];
        m.extend_from_slice(&0i32.to_le_bytes());
        m.extend_from_slice(&123_456u32.to_le_bytes());
        assert_eq!(val(&ty(&[MONEYN, 8]), &m), RemoteValue::Decimal("12.3456".into()));
        assert_eq!(
            val(&ty(&[MONEY4]), &(-1i32).to_le_bytes()),
            RemoteValue::Decimal("-0.0001".into())
        );
    }

    #[test]
    fn dates_and_times() {
        // date 2026-11-18: 739937 days after 0001-01-01
        let days = 739_937u32.to_le_bytes();
        assert_eq!(
            val(&ty(&[DATEN]), &[3, days[0], days[1], days[2]]),
            RemoteValue::Date {
                year: 2026,
                month: 11,
                day: 18
            }
        );
        // datetime 1900-01-02 00:00:01.003 (301 ticks of 1/300 s)
        let mut dt = 1i32.to_le_bytes().to_vec();
        dt.extend_from_slice(&301u32.to_le_bytes());
        assert_eq!(
            val(&ty(&[DATETIME]), &dt),
            RemoteValue::DateTime {
                year: 1900,
                month: 1,
                day: 2,
                millis: 1003
            }
        );
        // time(7) 12:00:00.1234567 → milliseconds
        let t = ty(&[TIMEN, 7]);
        let units = (12u64 * 3600 * 10_000_000 + 1_234_567).to_le_bytes();
        assert_eq!(
            val(&t, &[&[5][..], &units[..5]].concat()),
            RemoteValue::Time { millis: 43_200_123 }
        );
        // datetimeoffset(0) 2026-01-01T00:30:00 UTC at +01:00 → 01:30 local
        let o = ty(&[DATETIMEOFFSETN, 0]);
        let mut v = vec![8];
        v.extend_from_slice(&1800u32.to_le_bytes()[..3]);
        v.extend_from_slice(&739_616u32.to_le_bytes()[..3]);
        v.extend_from_slice(&60i16.to_le_bytes());
        assert_eq!(
            val(&o, &v),
            RemoteValue::DateTimeZone {
                year: 2026,
                month: 1,
                day: 1,
                millis: 5_400_000,
                offset_minutes: 60
            }
        );
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(DAYS_0001), (1, 1, 1));
    }

    #[test]
    fn texts_and_binary() {
        let latin1 = [0x09, 0x04, 0xD0, 0x00, 0x34];
        let mut t = vec![BIGVARCHR, 10, 0];
        t.extend_from_slice(&latin1);
        let t = ty(&t);
        assert_eq!(
            val(&t, &[3, 0, b'a', 0x80, 0xE9]),
            RemoteValue::Text("a€é".into())
        );
        assert_eq!(val(&t, &[0xFF, 0xFF]), RemoteValue::Null);
        let n = ty(&[NVARCHAR, 0xFF, 0xFF, 0x09, 0x04, 0xD0, 0x00, 0x34]);
        assert!(n.plp);
        // PLP "hé" in two chunks
        let mut v = 4u64.to_le_bytes().to_vec();
        v.extend_from_slice(&2u32.to_le_bytes());
        v.extend_from_slice(&[b'h', 0]);
        v.extend_from_slice(&2u32.to_le_bytes());
        v.extend_from_slice(&[0xE9, 0]);
        v.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(val(&n, &v), RemoteValue::Text("hé".into()));
        assert_eq!(val(&n, &u64::MAX.to_le_bytes()), RemoteValue::Null);
        let g = ty(&[GUID, 16]);
        let mut b = vec![16];
        b.extend_from_slice(&[
            0x33, 0x22, 0x11, 0x00, 0x55, 0x44, 0x77, 0x66, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
        ]);
        assert_eq!(
            val(&g, &b),
            RemoteValue::Text("00112233-4455-6677-8899-aabbccddeeff".into())
        );
        // a code page bicraft does not read (Japanese_CI_AS, LCID 0x0411)
        let mut j = vec![BIGVARCHR, 10, 0];
        j.extend_from_slice(&[0x11, 0x04, 0xD0, 0x00, 0x00]);
        let j = ty(&j);
        assert!(j.value(&mut Rd::new(&[2u8, 0, 0x82, 0xA0][..]), 100).is_err());
        assert_eq!(val(&j, &[2, 0, b'o', b'k']), RemoteValue::Text("ok".into()));
    }

    #[test]
    fn hostile_values_are_errors() {
        // length beyond the cap
        let n = ty(&[NVARCHAR, 0xFF, 0xFF, 0, 0, 0, 0, 0]);
        let mut v = u64::from(u32::MAX).to_le_bytes().to_vec();
        v.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(n.value(&mut Rd::new(v.as_slice()), 1000).is_err());
        // a decimal of 17 bytes over 38 digits
        let d = ty(&[DECIMALN, 17, 38, 0]);
        let mut v = vec![17, 1];
        v.extend_from_slice(&[0xFF; 16]);
        assert!(d.value(&mut Rd::new(v.as_slice()), 100).is_err());
        // an INTN of length 3
        assert!(
            ty(&[INTN, 4])
                .value(&mut Rd::new(&[3u8, 1, 2, 3][..]), 100)
                .is_err()
        );
        // bad TYPE_INFO
        for bad in [
            &[0x99u8][..],
            &[DECIMALN, 5, 0, 0],
            &[TIMEN, 9],
            &[NCHAR, 0xFF, 0xFF, 0, 0, 0, 0, 0],
        ] {
            assert!(TypeInfo::read(&mut Rd::new(bad)).is_err());
        }
        assert!(ty(&[SSVARIANT, 0, 1, 0, 0]).remote().is_err());
    }
}
