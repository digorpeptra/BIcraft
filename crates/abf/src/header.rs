//! The header page ([MS-XLDM] §2.1.1): BOM, stream storage signature, `BackupLog` XML of type
//! `BackupLogHeaderType`, zero padding to 4096 bytes.

use crate::text::{decode_xml, utf16_len_before_nul};
use crate::xml::{self, Element};
use crate::{Error, PAGE_SIZE};

/// The stream storage signature of [MS-XLDM] §2.1.1.2, stored as UTF-16LE after the BOM.
const SIGNATURE: &str = "STREAM_STORAGE_SIGNATURE_)!@#$%^&*(";
/// Offset of the header XML: 2-byte BOM + 35 UTF-16 code units of signature.
pub(crate) const HEADER_XML_OFFSET: usize = 2 + 2 * 35;
/// Start of the UTF-16LE banner that prefixes a still-compressed PBIX `DataModel`
/// (see `docs/formats/xpress9.md`).
const XPRESS9_BANNER_PREFIX: &str = "This backup was created";

/// Fields of the header `BackupLog` (`BackupLogHeaderType`, [MS-XLDM] §2.1.1.3) that the reader
/// uses. Unknown elements are ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// `BackupRestoreSyncVersion` (140 in every file observed so far, Excel and Power BI).
    pub sync_version: Option<u32>,
    /// `ErrorCode`: true when every file in the files section ends with a CRC marker.
    /// Power BI Desktop writes false.
    pub crc_markers: bool,
    /// `EncryptionFlag`.
    pub encrypted: bool,
    /// `ApplyCompression`: whole-stream compression (the Excel "xpress8" flavor).
    pub apply_compression: bool,
    /// `m_cbOffsetHeader`: absolute offset of the virtual directory.
    pub directory_offset: u64,
    /// `DataSize`: byte length of the virtual directory XML.
    pub directory_size: u64,
    /// `Files`: number of virtual directory entries announced by the header.
    pub file_count: Option<u64>,
    /// `ObjectID`: a GUID.
    pub object_id: Option<String>,
    /// `m_cbOffsetData`: start of the files section (4096 in practice).
    pub data_offset: Option<u64>,
}

/// Checks the BOM and signature, or explains what the input looks like instead.
pub(crate) fn check_signature(bytes: &[u8]) -> Result<(), Error> {
    if starts_with_utf16(bytes, XPRESS9_BANNER_PREFIX) {
        return Err(Error::Xpress9Compressed);
    }
    let Some(sig) = bytes.get(2..HEADER_XML_OFFSET) else {
        return Err(Error::NotAbf("shorter than the stream header".into()));
    };
    if bytes.get(..2) != Some(&[0xFF, 0xFE][..]) {
        return Err(Error::NotAbf("missing UTF-16LE byte order mark".into()));
    }
    if !starts_with_utf16(sig, SIGNATURE) {
        return Err(Error::NotAbf("missing stream storage signature".into()));
    }
    Ok(())
}

fn starts_with_utf16(bytes: &[u8], ascii: &str) -> bool {
    let mut units = bytes.as_chunks::<2>().0.iter();
    ascii
        .encode_utf16()
        .all(|u| units.next().is_some_and(|p| *p == u.to_le_bytes()))
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Header, Error> {
    check_signature(bytes)?;
    let end = bytes.len().min(PAGE_SIZE);
    let page = bytes.get(HEADER_XML_OFFSET..end).unwrap_or_default();
    let xml_bytes = page.get(..utf16_len_before_nul(page)).unwrap_or_default();
    let doc = decode_xml(xml_bytes, "header")?;
    let root = xml::parse(&doc, "header")?;
    if root.name != "BackupLog" {
        return Err(malformed("root element is not BackupLog"));
    }
    let header = Header {
        sync_version: opt_num(&root, "BackupRestoreSyncVersion")?,
        crc_markers: opt_bool(&root, "ErrorCode")?.unwrap_or(false),
        encrypted: opt_bool(&root, "EncryptionFlag")?.unwrap_or(false),
        apply_compression: opt_bool(&root, "ApplyCompression")?.unwrap_or(false),
        directory_offset: opt_num(&root, "m_cbOffsetHeader")?
            .ok_or_else(|| malformed("no m_cbOffsetHeader"))?,
        directory_size: opt_num(&root, "DataSize")?.ok_or_else(|| malformed("no DataSize"))?,
        file_count: opt_num(&root, "Files")?,
        object_id: root.child_text("ObjectID").map(|s| s.trim().to_string()),
        data_offset: opt_num(&root, "m_cbOffsetData")?,
    };
    Ok(header)
}

fn malformed(reason: &str) -> Error {
    Error::Malformed {
        part: "header",
        reason: reason.to_string(),
    }
}

fn opt_num<T: std::str::FromStr>(e: &Element, name: &str) -> Result<Option<T>, Error> {
    num_field(e, name, "header")
}

fn opt_bool(e: &Element, name: &str) -> Result<Option<bool>, Error> {
    bool_field(e, name, "header")
}

/// Parses an optional numeric child element. Present but unparsable is an error.
pub(crate) fn num_field<T: std::str::FromStr>(
    e: &Element,
    name: &str,
    part: &'static str,
) -> Result<Option<T>, Error> {
    match e.child_text(name) {
        None => Ok(None),
        Some(t) => t.trim().parse::<T>().map(Some).map_err(|_| Error::Malformed {
            part,
            reason: format!("{name} is not a number"),
        }),
    }
}

/// Parses an optional boolean child element (`true`/`false`/`1`/`0`, any case).
pub(crate) fn bool_field(e: &Element, name: &str, part: &'static str) -> Result<Option<bool>, Error> {
    match e.child_text(name).map(str::trim) {
        None => Ok(None),
        Some(t) if t.eq_ignore_ascii_case("true") || t == "1" => Ok(Some(true)),
        Some(t) if t.eq_ignore_ascii_case("false") || t == "0" => Ok(Some(false)),
        Some(_) => Err(Error::Malformed {
            part,
            reason: format!("{name} is not a boolean"),
        }),
    }
}
