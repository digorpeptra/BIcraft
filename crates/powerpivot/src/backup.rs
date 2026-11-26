//! The files of a Power Pivot backup: found by file name (each BackupLog file group has its own
//! root, so logical paths are not comparable), read decompressed (`ApplyCompression`, without
//! the CRC marker of `ErrorCode`).

use std::borrow::Cow;

use bicraft_abf::{Abf, FileEntry};

use crate::{Error, Limits};

/// Width of the CRC end marker that follows each stored file when the header's `ErrorCode` is
/// set ([MS-XLDM] §2.1.2.2.1).
const CRC_MARKER_BYTES: usize = 4;

pub(crate) struct Backup<'a> {
    abf: Abf<'a>,
    pub limits: &'a Limits,
}

/// `s` ends with `suffix`, ASCII case-insensitively.
pub(crate) fn ends_with_ci(s: &str, suffix: &str) -> bool {
    s.len() >= suffix.len()
        && s.is_char_boundary(s.len() - suffix.len())
        && s.get(s.len() - suffix.len()..)
            .is_some_and(|t| t.eq_ignore_ascii_case(suffix))
}

impl<'a> Backup<'a> {
    pub fn new(abf: Abf<'a>, limits: &'a Limits) -> Self {
        Backup { abf, limits }
    }

    /// The files whose name ends with `.<kind>.xml`, in stream order.
    pub fn xml_files(&self, kind: &str) -> Vec<&FileEntry> {
        let suffix = format!(".{kind}.xml");
        self.abf
            .files()
            .iter()
            .filter(|f| ends_with_ci(f.name(), &suffix))
            .collect()
    }

    /// A file by its name (last path segment), ASCII case-insensitively.
    pub fn by_name(&self, name: &str) -> Option<&FileEntry> {
        self.abf.files().iter().find(|f| f.name() == name).or_else(|| {
            self.abf
                .files()
                .iter()
                .find(|f| f.name().eq_ignore_ascii_case(name))
        })
    }

    /// The content of a file.
    pub fn read(&self, entry: &FileEntry) -> Result<Cow<'a, [u8]>, Error> {
        if entry.size as u64 > self.limits.max_file_bytes {
            return Err(Error::Limit(format!(
                "file of {} bytes, more than {}",
                entry.size, self.limits.max_file_bytes
            )));
        }
        if !entry.compressed {
            return self
                .abf
                .data(entry)
                .map(Cow::Borrowed)
                .ok_or_else(|| Error::Backup("a file lies outside the backup".into()));
        }
        let stored = self
            .abf
            .stored(entry)
            .ok_or_else(|| Error::Backup("a file lies outside the backup".into()))?;
        let chunks = if self.abf.header().crc_markers {
            stored
                .get(..stored.len().saturating_sub(CRC_MARKER_BYTES))
                .unwrap_or_default()
        } else {
            stored
        };
        bicraft_xpress8::decompress_chunked(chunks, Some(entry.size as u64), self.limits.max_file_bytes)
            .map(Cow::Owned)
            .map_err(|e| Error::Backup(format!("a compressed file: {e}")))
    }

    /// A file's content as text (UTF-16LE or UTF-8, with or without a byte order mark).
    pub fn read_text(&self, entry: &FileEntry) -> Result<String, Error> {
        if entry.size as u64 > self.limits.max_xml_bytes {
            return Err(Error::Limit(format!(
                "XML document of {} bytes, more than {}",
                entry.size, self.limits.max_xml_bytes
            )));
        }
        Ok(decode_text(&self.read(entry)?))
    }
}

/// Decodes an XML document's bytes: UTF-16LE (BOM `FF FE`, or a `<` followed by a zero byte),
/// else UTF-8 (BOM stripped). Invalid sequences become U+FFFD.
pub(crate) fn decode_text(bytes: &[u8]) -> String {
    let utf16 = |b: &[u8]| {
        let units = b.as_chunks::<2>().0.iter().map(|p| u16::from_le_bytes(*p));
        char::decode_utf16(units)
            .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect::<String>()
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        [b'<', 0, ..] => utf16(bytes),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}
