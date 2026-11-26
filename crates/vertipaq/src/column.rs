//! High-level column decoding: data ids → values.

use crate::dictionary::{Dictionary, DictionaryValues, parse_dictionary};
use crate::idf::{ID_BYTES, NULL_DATA_ID, check_rows, decode_data_ids};
use crate::idfmeta::parse_idfmeta;
use crate::{Error, FileKind, Limits};

/// One decoded cell.
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    /// Blank.
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Bool(bool),
    /// OLE Automation date: days since 1899-12-30, fraction = time of day.
    DateTime(f64),
    /// Fixed decimal (TOM `decimal`, Power BI "currency"): the value times 10 000.
    Decimal(i64),
}

/// How the column's data ids map to values (`DictionaryStorage.Type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodingKind {
    /// Data ids index a `.dictionary` (Type 1).
    Hash,
    /// Data ids are arithmetic: value = (id + base) / magnitude (Type 2).
    Value,
    /// No dictionary: the data ids themselves are the values (Type 0, engine system tables
    /// such as attribute-hierarchy position maps and relationship indexes).
    Raw,
}

/// Physical value type (`DictionaryStorage.DataType`, TOM data-type codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageType {
    /// 6: 64-bit integers (also booleans, decimals and value-encoded dates).
    Long,
    /// 8: doubles (also dates).
    Real,
    /// 2: strings.
    String,
    /// 19 (and anything else on a raw column): no value type, data ids only.
    None,
}

/// The column's declared type (`Column.ExplicitDataType`, falling back to `InferredDataType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalType {
    String,
    Int64,
    Double,
    DateTime,
    Decimal,
    Boolean,
    Binary,
}

impl LogicalType {
    /// From the TOM `DataType` code (2, 6, 8, 9, 10, 11, 17); `None` for others.
    pub fn from_code(code: i64) -> Option<LogicalType> {
        Some(match code {
            2 => LogicalType::String,
            6 => LogicalType::Int64,
            8 => LogicalType::Double,
            9 => LogicalType::DateTime,
            10 => LogicalType::Decimal,
            11 => LogicalType::Boolean,
            17 => LogicalType::Binary,
            _ => return None,
        })
    }
}

impl StorageType {
    /// From `DictionaryStorage.DataType`.
    pub fn from_code(code: i64) -> Option<StorageType> {
        Some(match code {
            2 => StorageType::String,
            6 => StorageType::Long,
            8 => StorageType::Real,
            19 => StorageType::None,
            _ => return None,
        })
    }
}

/// Everything `metadata.sqlitedb` says about how to read a column's storage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnEncoding {
    pub kind: EncodingKind,
    pub storage_type: StorageType,
    /// `DictionaryStorage.BaseId` (value encoding).
    pub base_id: i64,
    /// `DictionaryStorage.Magnitude` (value encoding).
    pub magnitude: f64,
    /// When set, values are converted to the logical type (bool, date, decimal).
    pub logical_type: Option<LogicalType>,
}

impl ColumnEncoding {
    /// A hash-encoded column.
    pub fn hash(storage_type: StorageType) -> Self {
        ColumnEncoding {
            kind: EncodingKind::Hash,
            storage_type,
            base_id: 0,
            magnitude: 1.0,
            logical_type: None,
        }
    }

    /// A value-encoded column.
    pub fn value(base_id: i64, magnitude: f64, storage_type: StorageType) -> Self {
        ColumnEncoding {
            kind: EncodingKind::Value,
            storage_type,
            base_id,
            magnitude,
            logical_type: None,
        }
    }

    /// A column without dictionary whose data ids are returned as integers.
    pub fn raw() -> Self {
        ColumnEncoding {
            kind: EncodingKind::Raw,
            storage_type: StorageType::None,
            base_id: 0,
            magnitude: 1.0,
            logical_type: None,
        }
    }

    /// From a `DictionaryStorage` row: `Type`, `DataType`, `BaseId`, `Magnitude`.
    pub fn from_dictionary_storage(
        type_code: i64,
        data_type: i64,
        base_id: i64,
        magnitude: f64,
    ) -> Result<Self, Error> {
        if type_code == 0 {
            return Ok(ColumnEncoding::raw());
        }
        let storage_type = StorageType::from_code(data_type).ok_or_else(|| Error::Unsupported {
            kind: FileKind::Column,
            message: format!("dictionary data type {data_type}"),
        })?;
        match type_code {
            1 => Ok(ColumnEncoding::hash(storage_type)),
            2 => Ok(ColumnEncoding::value(base_id, magnitude, storage_type)),
            other => Err(Error::Unsupported {
                kind: FileKind::Column,
                message: format!("dictionary storage type {other}"),
            }),
        }
    }

    /// Sets the logical type used for the final conversion.
    pub fn with_logical_type(mut self, logical_type: Option<LogicalType>) -> Self {
        self.logical_type = logical_type;
        self
    }
}

/// Decodes a whole column (all segments of one partition) with default [`Limits`].
///
/// `dictionary` is required for hash-encoded columns and ignored otherwise.
pub fn decode_column(
    idf: &[u8],
    idfmeta: &[u8],
    dictionary: Option<&[u8]>,
    encoding: ColumnEncoding,
) -> Result<Vec<CellValue>, Error> {
    decode_column_with_limits(idf, idfmeta, dictionary, encoding, &Limits::default())
}

/// [`decode_column`] with explicit limits.
pub fn decode_column_with_limits(
    idf: &[u8],
    idfmeta: &[u8],
    dictionary: Option<&[u8]>,
    encoding: ColumnEncoding,
    limits: &Limits,
) -> Result<Vec<CellValue>, Error> {
    let meta = parse_idfmeta(idfmeta)?;
    // The ids stay alive while the cells are built: budget both before decoding either.
    let rows = meta
        .row_count()
        .ok_or_else(|| Error::LimitExceeded("row count overflows".into()))?;
    check_rows(rows, ID_BYTES.saturating_add(CELL_BYTES), limits)?;
    let ids = decode_data_ids(idf, &meta, limits)?;
    // A hash-encoded column holding only blanks has no `.dictionary` (engine-written:
    // `DictionaryStorage.StorageFileID` 0); `values_within` accepts that for blank ids only.
    let dict = match (encoding.kind, dictionary) {
        (EncodingKind::Hash, Some(d)) => Some(parse_dictionary(d)?),
        (EncodingKind::Hash, None) | (EncodingKind::Value | EncodingKind::Raw, _) => None,
    };
    let id_bytes = u64::try_from(ids.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(ID_BYTES);
    values_within(
        &ids,
        dict.as_ref(),
        encoding,
        limits.max_bytes.saturating_sub(id_bytes),
    )
}

/// Bytes of one decoded cell, not counting the text it owns.
const CELL_BYTES: u64 = std::mem::size_of::<CellValue>() as u64;

/// Maps data ids to values. `dictionary` must be given for hash encoding, except when every id
/// is the blank id (the engine writes no `.dictionary` for a column holding only blanks).
///
/// Unbounded: every text cell is a copy of its dictionary string, so the caller bounds `ids`.
/// [`decode_column_with_limits`] applies [`Limits::max_bytes`].
pub fn ids_to_values(
    ids: &[u32],
    dictionary: Option<&Dictionary>,
    encoding: ColumnEncoding,
) -> Result<Vec<CellValue>, Error> {
    values_within(ids, dictionary, encoding, u64::MAX)
}

/// [`ids_to_values`] producing at most `max_bytes` (cells plus the text they own).
fn values_within(
    ids: &[u32],
    dictionary: Option<&Dictionary>,
    encoding: ColumnEncoding,
    max_bytes: u64,
) -> Result<Vec<CellValue>, Error> {
    let over = || Error::LimitExceeded(format!("decoded cells exceed the {max_bytes}-byte limit"));
    let mut used = u64::try_from(ids.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(CELL_BYTES);
    if used > max_bytes {
        return Err(over());
    }
    let mut out = Vec::with_capacity(ids.len());
    match encoding.kind {
        EncodingKind::Hash => {
            for &id in ids {
                let v = if id == NULL_DATA_ID {
                    CellValue::Null
                } else {
                    let dict = dictionary.ok_or(Error::MissingDictionary)?;
                    let i = dict
                        .index_of(id)
                        .ok_or(Error::DataIdOutOfRange { id, len: dict.len() })?;
                    match &dict.values {
                        DictionaryValues::Long(v) => v.get(i).map(|&x| CellValue::Int(x)),
                        DictionaryValues::Real(v) => v.get(i).map(|&x| CellValue::Real(x)),
                        DictionaryValues::Text(v) => match v.get(i) {
                            Some(s) => {
                                used = used.saturating_add(u64::try_from(s.len()).unwrap_or(u64::MAX));
                                if used > max_bytes {
                                    return Err(over());
                                }
                                Some(CellValue::Text(s.clone()))
                            }
                            None => None,
                        },
                    }
                    .unwrap_or(CellValue::Null)
                };
                out.push(convert(v, encoding.logical_type));
            }
        }
        EncodingKind::Raw => out.extend(ids.iter().map(|&id| CellValue::Int(i64::from(id)))),
        EncodingKind::Value => {
            let scale = Scale::new(encoding.magnitude)?;
            for &id in ids {
                let v = if id == NULL_DATA_ID {
                    CellValue::Null
                } else {
                    let n = i64::from(id)
                        .checked_add(encoding.base_id)
                        .ok_or(Error::ValueOverflow { id })?;
                    scale.apply(n, encoding.storage_type, id)?
                };
                out.push(convert(v, encoding.logical_type));
            }
        }
    }
    Ok(out)
}

/// Division by `Magnitude`, exact when the magnitude is 1, 10^-k or an integer.
enum Scale {
    One,
    Multiply(i64),
    Divide(f64),
}

impl Scale {
    fn new(magnitude: f64) -> Result<Scale, Error> {
        if !magnitude.is_finite() || magnitude <= 0.0 {
            return Err(Error::Malformed {
                kind: FileKind::Column,
                offset: 0,
                message: format!("value-encoding magnitude {magnitude}"),
            });
        }
        if magnitude == 1.0 {
            return Ok(Scale::One);
        }
        let inverse = 1.0 / magnitude;
        let rounded = inverse.round();
        if magnitude < 1.0 && (inverse - rounded).abs() <= 1e-9 * rounded && rounded < 1e18 {
            return Ok(Scale::Multiply(rounded as i64));
        }
        Ok(Scale::Divide(magnitude))
    }

    fn apply(&self, n: i64, storage: StorageType, id: u32) -> Result<CellValue, Error> {
        let real = match self {
            Scale::One => None,
            Scale::Multiply(k) => {
                let v = n.checked_mul(*k).ok_or(Error::ValueOverflow { id })?;
                return Ok(match storage {
                    StorageType::Real => CellValue::Real(v as f64),
                    _ => CellValue::Int(v),
                });
            }
            Scale::Divide(m) => Some(n as f64 / m),
        };
        Ok(match (real, storage) {
            (None, StorageType::Real) => CellValue::Real(n as f64),
            (None, _) => CellValue::Int(n),
            (Some(r), StorageType::Real) => CellValue::Real(r),
            (Some(r), _) if r.is_finite() && r.fract() == 0.0 && r.abs() < 9.0e18 => CellValue::Int(r as i64),
            (Some(r), _) => CellValue::Real(r),
        })
    }
}

/// Converts a physical value to the column's logical type.
pub fn convert(v: CellValue, logical: Option<LogicalType>) -> CellValue {
    match (logical, v) {
        (_, CellValue::Null) => CellValue::Null,
        (Some(LogicalType::Boolean), CellValue::Int(n)) => CellValue::Bool(n != 0),
        (Some(LogicalType::DateTime), CellValue::Real(r)) => CellValue::DateTime(r),
        (Some(LogicalType::DateTime), CellValue::Int(n)) => CellValue::DateTime(n as f64),
        (Some(LogicalType::Decimal), CellValue::Int(n)) => CellValue::Decimal(n),
        (Some(LogicalType::Double), CellValue::Int(n)) => CellValue::Real(n as f64),
        (_, v) => v,
    }
}
