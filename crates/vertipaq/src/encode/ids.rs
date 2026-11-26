//! Values → data ids: hash dictionaries and value-encoding parameters.

use std::collections::HashMap;

use super::{EncodeOptions, EncodingRequest, ValueParams};
use crate::column::{CellValue, EncodingKind, LogicalType, StorageType};
use crate::dictionary::DictionaryValues;
use crate::idf::{FIRST_DATA_ID, NULL_DATA_ID};
use crate::{Error, FileKind};

/// A column after id assignment.
pub(super) struct Assigned {
    pub ids: Vec<u32>,
    pub kind: EncodingKind,
    pub storage_type: StorageType,
    pub base_id: i64,
    pub magnitude: f64,
    /// Distinct non-blank values present in the rows.
    pub distinct: u64,
    /// Hash encoding: the dictionary in id order (index `i` = id `i + 3`).
    pub dictionary: Option<DictionaryValues>,
}

/// A value in its physical (storage) form.
enum Physical<'a> {
    Null,
    Long(i64),
    Real(f64),
    Text(&'a str),
}

/// Largest dictionary: ids must stay below `u32::MAX`.
const MAX_DICTIONARY: usize = (u32::MAX - FIRST_DATA_ID) as usize;

fn storage_of(logical: LogicalType) -> Result<StorageType, Error> {
    Ok(match logical {
        LogicalType::String => StorageType::String,
        LogicalType::Double | LogicalType::DateTime => StorageType::Real,
        LogicalType::Int64 | LogicalType::Decimal | LogicalType::Boolean => StorageType::Long,
        LogicalType::Binary => return Err(unsupported("binary columns".into())),
    })
}

fn unsupported(message: String) -> Error {
    Error::Unsupported {
        kind: FileKind::Column,
        message,
    }
}

fn physical(v: &CellValue, logical: LogicalType, row: usize) -> Result<Physical<'_>, Error> {
    Ok(match (logical, v) {
        (_, CellValue::Null) => Physical::Null,
        (LogicalType::String, CellValue::Text(s)) if s.contains('\0') => {
            // Uncompressed pages terminate strings with U+0000, so it cannot be stored.
            return Err(unsupported(format!("U+0000 inside the string at row {row}")));
        }
        (LogicalType::String, CellValue::Text(s)) => Physical::Text(s),
        (LogicalType::Int64, CellValue::Int(n)) => Physical::Long(*n),
        (LogicalType::Decimal, CellValue::Decimal(n)) => Physical::Long(*n),
        (LogicalType::Boolean, CellValue::Bool(b)) => Physical::Long(i64::from(*b)),
        (LogicalType::Boolean, CellValue::Int(n @ (0 | 1))) => Physical::Long(*n),
        (LogicalType::Double, CellValue::Real(r)) | (LogicalType::DateTime, CellValue::DateTime(r)) => {
            Physical::Real(*r)
        }
        (LogicalType::Double, CellValue::Int(n)) => Physical::Real(*n as f64),
        (LogicalType::DateTime, CellValue::Real(r)) => Physical::Real(*r),
        (l, v) => {
            return Err(unsupported(format!(
                "value {} at row {row} in a {l:?} column",
                variant_name(v)
            )));
        }
    })
}

fn variant_name(v: &CellValue) -> &'static str {
    match v {
        CellValue::Null => "Null",
        CellValue::Int(_) => "Int",
        CellValue::Real(_) => "Real",
        CellValue::Text(_) => "Text",
        CellValue::Bool(_) => "Bool",
        CellValue::DateTime(_) => "DateTime",
        CellValue::Decimal(_) => "Decimal",
    }
}

/// Assigns data ids according to the options.
pub(super) fn assign(
    values: &[CellValue],
    logical: LogicalType,
    options: &EncodeOptions,
) -> Result<Assigned, Error> {
    let storage = storage_of(logical)?;
    let numeric_long = matches!(logical, LogicalType::Int64 | LogicalType::Decimal);
    match options.encoding {
        EncodingRequest::Hash => hash(values, logical, storage, options),
        EncodingRequest::Value { params } => {
            let assigned = if logical == LogicalType::Double {
                value_real(values, logical, params)?
            } else if numeric_long {
                value(values, logical, params)?
            } else {
                return Err(unsupported(format!("value encoding of a {logical:?} column")));
            };
            assigned.ok_or_else(|| Error::LimitExceeded("value-encoded ids do not fit in 32 bits".into()))
        }
        EncodingRequest::Auto => {
            if logical == LogicalType::Decimal
                && let Some(a) = value(values, logical, None)?
            {
                return Ok(a);
            }
            // Int64 without a hint (observed on engine- and Desktop-written columns of 28 to
            // 1 000 000 rows): value encoding when there are at most 20 rows per distinct value
            // (15 and 20 value-encoded, 22 and more hashed) and the ids stay below 2^31 (a
            // range of 4·10^9 was hashed, one of 6.4·10^5 value-encoded).
            if logical == LogicalType::Int64
                && let Some(a) = value(values, logical, None)?
                && a.distinct > 0
                && (values.len() as u64) <= a.distinct.saturating_mul(INT64_VALUE_ROWS_PER_VALUE)
                && a.ids.iter().all(|&id| id < 1 << 31)
            {
                return Ok(a);
            }
            hash(values, logical, storage, options)
        }
    }
}

/// Most rows per distinct value for which an int64 column without a hint is value-encoded.
const INT64_VALUE_ROWS_PER_VALUE: u64 = 20;

/// Hash key of a value: exact bits for reals (−0 folded into +0, one NaN).
#[derive(Hash, PartialEq, Eq)]
enum Key<'a> {
    Long(i64),
    Real(u64),
    Text(&'a str),
}

fn real_key(r: f64) -> u64 {
    if r == 0.0 {
        0
    } else if r.is_nan() {
        f64::NAN.to_bits()
    } else {
        r.to_bits()
    }
}

/// Hash dictionary under construction.
struct HashBuilder<'a> {
    map: HashMap<Key<'a>, u32>,
    longs: Vec<i64>,
    reals: Vec<f64>,
    texts: Vec<&'a str>,
}

impl<'a> HashBuilder<'a> {
    fn add(&mut self, p: Physical<'a>) -> Result<u32, Error> {
        let key = match p {
            Physical::Null => return Ok(NULL_DATA_ID),
            Physical::Long(n) => Key::Long(n),
            Physical::Real(r) => Key::Real(real_key(r)),
            Physical::Text(s) => Key::Text(s),
        };
        if let Some(&id) = self.map.get(&key) {
            return Ok(id);
        }
        let len = self.map.len();
        if len >= MAX_DICTIONARY {
            return Err(Error::LimitExceeded(format!(
                "more than {MAX_DICTIONARY} distinct values"
            )));
        }
        // `len` < MAX_DICTIONARY, so the id fits in u32.
        let id = FIRST_DATA_ID + len as u32;
        match &key {
            Key::Long(n) => self.longs.push(*n),
            Key::Real(bits) => self.reals.push(f64::from_bits(*bits)),
            Key::Text(s) => self.texts.push(s),
        }
        self.map.insert(key, id);
        Ok(id)
    }
}

fn hash(
    values: &[CellValue],
    logical: LogicalType,
    storage: StorageType,
    options: &EncodeOptions,
) -> Result<Assigned, Error> {
    let mut b = HashBuilder {
        map: HashMap::new(),
        longs: Vec::new(),
        reals: Vec::new(),
        texts: Vec::new(),
    };
    for (row, v) in options.dictionary_seed.iter().enumerate() {
        b.add(physical(v, logical, row)?)?;
    }
    let mut ids = Vec::with_capacity(values.len());
    let mut used: Vec<bool> = vec![false; b.map.len()];
    for (row, v) in values.iter().enumerate() {
        let id = b.add(physical(v, logical, row)?)?;
        if id != NULL_DATA_ID {
            let index = (id - FIRST_DATA_ID) as usize;
            if index >= used.len() {
                used.resize(index + 1, false);
            }
            if let Some(u) = used.get_mut(index) {
                *u = true;
            }
        }
        ids.push(id);
    }
    let distinct = used.iter().filter(|&&u| u).count() as u64;
    let dictionary = match storage {
        StorageType::String => DictionaryValues::Text(b.texts.into_iter().map(str::to_owned).collect()),
        StorageType::Real => DictionaryValues::Real(b.reals),
        _ => DictionaryValues::Long(b.longs),
    };
    Ok(Assigned {
        ids,
        kind: EncodingKind::Hash,
        storage_type: storage,
        base_id: 2,
        magnitude: 0.0,
        distinct,
        dictionary: Some(dictionary),
    })
}

/// Value encoding; `Ok(None)` when the ids would not fit in 32 bits.
fn value(
    values: &[CellValue],
    logical: LogicalType,
    params: Option<ValueParams>,
) -> Result<Option<Assigned>, Error> {
    let mut longs: Vec<Option<i64>> = Vec::with_capacity(values.len());
    for (row, v) in values.iter().enumerate() {
        longs.push(match physical(v, logical, row)? {
            Physical::Long(n) => Some(n),
            _ => None,
        });
    }
    let (divisor, base_id, magnitude) = match params {
        Some(p) => {
            let divisor = divisor_of(p.magnitude)?;
            for n in longs.iter().flatten() {
                if n % divisor != 0 {
                    return Err(unsupported(format!(
                        "a value is not a multiple of 1/Magnitude ({divisor})"
                    )));
                }
            }
            (divisor, p.base_id, p.magnitude)
        }
        None => {
            let divisor = common_power_of_ten(longs.iter().flatten().copied());
            let min = longs.iter().flatten().map(|n| n / divisor).min().unwrap_or(0);
            let Some(base) = min.checked_sub(i64::from(FIRST_DATA_ID)) else {
                return Ok(None);
            };
            (divisor, base, 1.0 / divisor as f64)
        }
    };
    let mut ids = Vec::with_capacity(longs.len());
    for n in &longs {
        let id = match n {
            None => NULL_DATA_ID,
            Some(n) => {
                let id = (n / divisor)
                    .checked_sub(base_id)
                    .and_then(|i| u32::try_from(i).ok());
                match id {
                    Some(id) if (FIRST_DATA_ID..u32::MAX).contains(&id) => id,
                    _ if params.is_none() => return Ok(None),
                    _ => {
                        return Err(Error::LimitExceeded(
                            "a value maps outside the data-id range with the given BaseId".into(),
                        ));
                    }
                }
            }
        };
        ids.push(id);
    }
    let mut sorted: Vec<u32> = ids.iter().copied().filter(|&i| i != NULL_DATA_ID).collect();
    sorted.sort_unstable();
    sorted.dedup();
    Ok(Some(Assigned {
        ids,
        kind: EncodingKind::Value,
        storage_type: StorageType::Long,
        base_id,
        magnitude,
        distinct: sorted.len() as u64,
        dictionary: None,
    }))
}

/// Largest power of ten tried for a value-encoded double (`Magnitude` 10^k).
const MAX_REAL_DIGITS: u32 = 15;

/// Value encoding of a double column: `Magnitude` = 10^k (k ≥ 0) such that every value times
/// 10^k is an integer `n` with `n / 10^k` giving back exactly the same double; data id =
/// `n − BaseId`. `Ok(None)` when no such k ≤ 15 exists (without `params`) or the ids would not
/// fit in 32 bits. Observed on engine-written columns: `Value/2.5` stored with Magnitude 10,
/// `Value/62.5` with Magnitude 1000, `Value/10000` with Magnitude 10^4,
/// `DictionaryStorage.DataType` 8. Given `params`, k may also be negative (down to −15): the
/// engine stored the multiples of 10^15 of `Value * 1E+15` with Magnitude 10^−12 (`n` = value /
/// 10^12, read back as `n × 10^12`); without `params` the encoder never picks k < 0 (when the
/// engine does is unknown).
fn value_real(
    values: &[CellValue],
    logical: LogicalType,
    params: Option<ValueParams>,
) -> Result<Option<Assigned>, Error> {
    let mut reals: Vec<Option<f64>> = Vec::with_capacity(values.len());
    for (row, v) in values.iter().enumerate() {
        reals.push(match physical(v, logical, row)? {
            Physical::Real(r) => Some(r),
            _ => None,
        });
    }
    // n with n / scale == r exactly, if any. A scale below 1 is 1 / an integer power of ten
    // (`down`): n = r / down, read back as n × down (the reader's rule).
    let scaled = |r: f64, scale: f64| -> Option<i64> {
        if scale < 1.0 {
            let down = (1.0 / scale).round();
            if !(r.fract() == 0.0 && r.abs() < 9.0e18 && (1.0..1e18).contains(&down)) {
                return None;
            }
            let (whole, down) = (r as i64, down as i64);
            let n = whole.checked_div(down)?;
            let back = n.checked_mul(down)?;
            return (back == whole && back as f64 == r).then_some(n);
        }
        let n = (r * scale).round();
        (n.abs() < 9.0e15 && n / scale == r).then_some(n as i64)
    };
    let (scale, base_id, magnitude) = match params {
        Some(p) => {
            let m = p.magnitude;
            let k = m.log10().round();
            let digits = f64::from(MAX_REAL_DIGITS);
            if !m.is_finite() || !(-digits..=digits).contains(&k) || 10f64.powi(k as i32) != m {
                return Err(unsupported(format!(
                    "value-encoding Magnitude {m} for a double column"
                )));
            }
            if reals.iter().flatten().any(|&r| scaled(r, m).is_none()) {
                return Err(unsupported(format!(
                    "a value is not a multiple of 1/Magnitude ({m})"
                )));
            }
            (m, p.base_id, m)
        }
        None => {
            let Some(scale) = (0..=MAX_REAL_DIGITS)
                .map(|k| 10f64.powi(k as i32))
                .find(|&m| reals.iter().flatten().all(|&r| scaled(r, m).is_some()))
            else {
                return Ok(None);
            };
            let min = reals
                .iter()
                .flatten()
                .filter_map(|&r| scaled(r, scale))
                .min()
                .unwrap_or(0);
            let Some(base) = min.checked_sub(i64::from(FIRST_DATA_ID)) else {
                return Ok(None);
            };
            (scale, base, scale)
        }
    };
    let mut ids = Vec::with_capacity(reals.len());
    for r in &reals {
        let id = match r {
            None => NULL_DATA_ID,
            Some(r) => {
                let id = scaled(*r, scale)
                    .and_then(|n| n.checked_sub(base_id))
                    .and_then(|i| u32::try_from(i).ok());
                match id {
                    Some(id) if (FIRST_DATA_ID..u32::MAX).contains(&id) => id,
                    _ if params.is_none() => return Ok(None),
                    _ => {
                        return Err(Error::LimitExceeded(
                            "a value maps outside the data-id range with the given BaseId".into(),
                        ));
                    }
                }
            }
        };
        ids.push(id);
    }
    let mut sorted: Vec<u32> = ids.iter().copied().filter(|&i| i != NULL_DATA_ID).collect();
    sorted.sort_unstable();
    sorted.dedup();
    Ok(Some(Assigned {
        ids,
        kind: EncodingKind::Value,
        storage_type: StorageType::Real,
        base_id,
        magnitude,
        distinct: sorted.len() as u64,
        dictionary: None,
    }))
}

/// `1 / magnitude` as an integer divisor (magnitude must be `1/m`, `m` ≥ 1).
fn divisor_of(magnitude: f64) -> Result<i64, Error> {
    let bad = || unsupported(format!("value-encoding Magnitude {magnitude}"));
    if !magnitude.is_finite() || magnitude <= 0.0 || magnitude > 1.0 {
        return Err(bad());
    }
    let inverse = 1.0 / magnitude;
    let m = inverse.round();
    if (inverse - m).abs() > 1e-9 * m || m > 1e18 {
        return Err(bad());
    }
    Ok(m as i64)
}

/// Largest `10^k` dividing every value (1 when all are zero or there are none).
fn common_power_of_ten(values: impl Iterator<Item = i64>) -> i64 {
    let mut k = 18u32;
    let mut any_nonzero = false;
    for n in values {
        if n == 0 {
            continue;
        }
        any_nonzero = true;
        while k > 0 && n % 10i64.pow(k) != 0 {
            k -= 1;
        }
        if k == 0 {
            break;
        }
    }
    if any_nonzero { 10i64.pow(k) } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powers_of_ten() {
        assert_eq!(common_power_of_ten([90000, 3990000].into_iter()), 10000);
        assert_eq!(common_power_of_ten([437020, 0].into_iter()), 10);
        assert_eq!(common_power_of_ten([7, 70].into_iter()), 1);
        assert_eq!(common_power_of_ten([0, 0].into_iter()), 1);
        assert_eq!(common_power_of_ten([i64::MIN].into_iter()), 1);
        assert_eq!(divisor_of(0.0001).unwrap(), 10000);
        assert_eq!(divisor_of(1.0).unwrap(), 1);
        assert!(divisor_of(0.3).is_err());
        assert!(divisor_of(2.0).is_err());
        assert!(divisor_of(f64::NAN).is_err());
    }

    #[test]
    fn doubles_with_few_decimals_are_value_encoded() {
        let vals: Vec<CellValue> = (1..=100).map(|i| CellValue::Real(f64::from(i) / 2.5)).collect();
        let a = value_real(&vals, LogicalType::Double, None).unwrap().unwrap();
        assert_eq!(
            (a.magnitude, a.base_id, a.storage_type),
            (10.0, 1, StorageType::Real)
        );
        assert_eq!(a.ids.first(), Some(&3));
        let thirds: Vec<CellValue> = (1..=10).map(|i| CellValue::Real(f64::from(i) / 3.0)).collect();
        assert!(value_real(&thirds, LogicalType::Double, None).unwrap().is_none());
        let p = ValueParams {
            base_id: 13,
            magnitude: 1000.0,
        };
        let sixteenths: Vec<CellValue> = (1..=10).map(|i| CellValue::Real(f64::from(i) / 62.5)).collect();
        let a = value_real(&sixteenths, LogicalType::Double, Some(p))
            .unwrap()
            .unwrap();
        assert_eq!(a.ids.first(), Some(&3));
        let bad = ValueParams {
            base_id: 0,
            magnitude: 3.0,
        };
        assert!(value_real(&sixteenths, LogicalType::Double, Some(bad)).is_err());
    }

    /// Regression (G10, engine-written `Value * 1E+15`): Magnitude 10^-12 with the engine's
    /// BaseId re-encodes to the engine's ids; values that are not multiples are refused.
    #[test]
    fn doubles_scaled_down_by_a_negative_power_of_ten() {
        let vals: Vec<CellValue> = (1..=5000).map(|i| CellValue::Real(f64::from(i) * 1e15)).collect();
        let p = ValueParams {
            base_id: 997,
            magnitude: 1e-12,
        };
        let a = value_real(&vals, LogicalType::Double, Some(p)).unwrap().unwrap();
        assert_eq!(a.ids.first(), Some(&3));
        assert_eq!(a.ids.last(), Some(&4_999_003));
        assert!(value_real(&[CellValue::Real(1.5e12)], LogicalType::Double, Some(p)).is_err());
        assert!(value_real(&[CellValue::Real(0.5)], LogicalType::Double, Some(p)).is_err());
        let p16 = ValueParams {
            base_id: 0,
            magnitude: 1e-16,
        };
        assert!(value_real(&vals, LogicalType::Double, Some(p16)).is_err());
        // Without parameters the encoder does not pick a negative power: hash encoding.
        assert!(value_real(&vals, LogicalType::Double, None).unwrap().is_none());
    }
}
