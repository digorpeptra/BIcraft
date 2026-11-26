//! The 100-byte database header at the start of page 1.

use crate::{Error, Result, corrupt};

/// The magic string every SQLite 3 database starts with.
pub const MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// Length of the database header.
pub const HEADER_LEN: usize = 100;

/// How text values are stored in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    Utf8,
    Utf16Le,
    Utf16Be,
}

/// The fields of the database header that matter for reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// Page size in bytes, a power of two between 512 and 65536.
    pub page_size: u32,
    /// Bytes reserved at the end of every page (for extensions such as checksums).
    pub reserved_bytes: u8,
    /// `page_size - reserved_bytes`: the part of each page b-tree code may use.
    pub usable_size: u32,
    /// File format write version (1 = legacy rollback journal, 2 = WAL).
    pub write_version: u8,
    /// File format read version (1 = legacy rollback journal, 2 = WAL).
    pub read_version: u8,
    pub file_change_counter: u32,
    /// The database size in pages as stored in the header (only trusted when valid).
    pub header_page_count: u32,
    /// The number of pages this reader uses: the header value when it is valid, otherwise the
    /// number of whole pages in the file; never more than the file holds.
    pub page_count: u32,
    pub freelist_trunk: u32,
    pub freelist_count: u32,
    pub schema_cookie: u32,
    /// Schema format number, 1 to 4 (0 in a database that never had a schema).
    pub schema_format: u32,
    /// Largest root b-tree page when auto-vacuum or incremental vacuum is on, else 0.
    pub largest_root_page: u32,
    pub text_encoding: TextEncoding,
    pub user_version: u32,
    pub incremental_vacuum: bool,
    pub application_id: u32,
    pub version_valid_for: u32,
    /// `SQLITE_VERSION_NUMBER` of the library that last wrote the file.
    pub sqlite_version_number: u32,
}

impl Header {
    /// Whether the database uses auto-vacuum (and so contains pointer-map pages).
    pub fn auto_vacuum(&self) -> bool {
        self.largest_root_page != 0
    }

    /// Parse and validate the header of a database image of `file_len` bytes.
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self> {
        let h = bytes
            .get(..HEADER_LEN)
            .ok_or(Error::NotSqlite("file shorter than the 100-byte header"))?;
        if h.get(..16) != Some(MAGIC.as_slice()) {
            return Err(Error::NotSqlite("bad magic string"));
        }
        let page_size = match be_u16(h, 16)? {
            1 => 65536,
            n if (512..=32768).contains(&n) && n.is_power_of_two() => u32::from(n),
            n => return Err(corrupt(format!("invalid page size {n}"))),
        };
        let write_version = byte(h, 18)?;
        let read_version = byte(h, 19)?;
        if read_version > 2 {
            return Err(Error::Unsupported(format!(
                "file format read version {read_version}"
            )));
        }
        let reserved_bytes = byte(h, 20)?;
        let usable_size = page_size - u32::from(reserved_bytes);
        if usable_size < 480 {
            return Err(corrupt(format!("usable page size {usable_size} is below 480")));
        }
        let file_change_counter = be_u32(h, 24)?;
        let header_page_count = be_u32(h, 28)?;
        let version_valid_for = be_u32(h, 92)?;
        let pages_in_file = u32::try_from(bytes.len() as u64 / u64::from(page_size)).unwrap_or(u32::MAX);
        // The in-header size is valid only when non-zero and written by a library that also
        // updated version-valid-for; otherwise the size comes from the file length.
        let page_count = if header_page_count != 0 && file_change_counter == version_valid_for {
            header_page_count.min(pages_in_file)
        } else {
            pages_in_file
        };
        if page_count == 0 {
            return Err(corrupt("the file holds no complete page"));
        }
        let schema_format = be_u32(h, 44)?;
        if schema_format > 4 {
            return Err(Error::Unsupported(format!("schema format {schema_format}")));
        }
        let text_encoding = match be_u32(h, 56)? {
            // 0 appears in a database that never stored any text yet; SQLite treats it as UTF-8.
            0 | 1 => TextEncoding::Utf8,
            2 => TextEncoding::Utf16Le,
            3 => TextEncoding::Utf16Be,
            n => return Err(corrupt(format!("invalid text encoding {n}"))),
        };
        Ok(Self {
            page_size,
            reserved_bytes,
            usable_size,
            write_version,
            read_version,
            file_change_counter,
            header_page_count,
            page_count,
            freelist_trunk: be_u32(h, 32)?,
            freelist_count: be_u32(h, 36)?,
            schema_cookie: be_u32(h, 40)?,
            schema_format,
            largest_root_page: be_u32(h, 52)?,
            text_encoding,
            user_version: be_u32(h, 60)?,
            incremental_vacuum: be_u32(h, 64)? != 0,
            application_id: be_u32(h, 68)?,
            version_valid_for,
            sqlite_version_number: be_u32(h, 96)?,
        })
    }
}

fn byte(b: &[u8], at: usize) -> Result<u8> {
    b.get(at).copied().ok_or_else(|| corrupt("truncated header"))
}

pub(crate) fn be_u16(b: &[u8], at: usize) -> Result<u16> {
    let s = b
        .get(at..at.wrapping_add(2))
        .ok_or_else(|| corrupt("truncated 16-bit field"))?;
    let mut a = [0u8; 2];
    a.copy_from_slice(s);
    Ok(u16::from_be_bytes(a))
}

pub(crate) fn be_u32(b: &[u8], at: usize) -> Result<u32> {
    let s = b
        .get(at..at.wrapping_add(4))
        .ok_or_else(|| corrupt("truncated 32-bit field"))?;
    let mut a = [0u8; 4];
    a.copy_from_slice(s);
    Ok(u32::from_be_bytes(a))
}
