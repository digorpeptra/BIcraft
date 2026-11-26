//! Reading the model of a `DataModel` too large to decompress into memory
//! (`docs/formats/pbix-package.md` §11, "Packages over 4 GiB"): a part left in the file
//! ([`bicraft_opc::Part::deferred`]), or one whose backup declares more than
//! [`crate::DataModelLimits::max_bytes`].
//!
//! Only `metadata.sqlitedb` is needed. The backup is decoded as a stream
//! ([`bicraft_xpress9::stream_datamodel`]) and only the byte ranges that matter are kept: the
//! header page, the virtual directory (with the bytes just before it, where the BackupLog
//! usually is), the BackupLog and `metadata.sqlitedb` (`bicraft_abf::streamed_index`). Each
//! range is one pass over the part that stops as soon as the range is complete; a bare ABF part
//! that can seek is read range by range instead.

use std::io::{Read, Seek, SeekFrom};

use bicraft_abf::PAGE_SIZE;
use bicraft_model::SemanticModel;
use bicraft_opc::{Part, ReadSeek};

use crate::Error;

/// Largest single range kept in memory (virtual directory, BackupLog, `metadata.sqlitedb`).
pub const MAX_STREAMED_FILE_BYTES: u64 = 512 << 20;

/// Bytes kept before the virtual directory on the pass that reads it.
const LOOKBEHIND: u64 = 64 << 20;

/// Something that returns byte ranges of the decompressed backup.
trait Ranges {
    fn fetch(&mut self, offset: u64, len: u64) -> Result<Vec<u8>, Error>;
}

fn too_large(what: &'static str, len: u64) -> Error {
    Error::Abf(bicraft_abf::Error::Unsupported(format!(
        "a {len}-byte {what} (over the {MAX_STREAMED_FILE_BYTES}-byte limit for streamed backups)"
    )))
}

fn out_of_bounds(what: &'static str, offset: u64, len: u64, stream_len: u64) -> Error {
    Error::Abf(bicraft_abf::Error::OutOfBounds {
        what,
        offset,
        len,
        stream_len: usize::try_from(stream_len).unwrap_or(usize::MAX),
    })
}

fn alloc(len: u64) -> Result<Vec<u8>, Error> {
    let n = usize::try_from(len).map_err(|_| too_large("range", len))?;
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| too_large("range", len))?;
    v.resize(n, 0);
    Ok(v)
}

/// A bare ABF with random access.
struct Seekable<'a>(Box<dyn ReadSeek + 'a>);

impl Ranges for Seekable<'_> {
    fn fetch(&mut self, offset: u64, len: u64) -> Result<Vec<u8>, Error> {
        let mut buf = alloc(len)?;
        self.0
            .seek(SeekFrom::Start(offset))
            .and_then(|_| self.0.read_exact(&mut buf))
            .map_err(|_| out_of_bounds("range", offset, len, offset))?;
        Ok(buf)
    }
}

/// An XPress9-framed part, decoded from the start on every fetch.
struct Streamed<'a> {
    open: &'a dyn Fn() -> Result<Box<dyn Read + Send + 'a>, Error>,
}

impl Ranges for Streamed<'_> {
    fn fetch(&mut self, offset: u64, len: u64) -> Result<Vec<u8>, Error> {
        let mut buf = alloc(len)?;
        let end = offset
            .checked_add(len)
            .ok_or_else(|| out_of_bounds("range", offset, len, u64::MAX))?;
        let mut filled = 0u64;
        let reader = (self.open)()?;
        let done = bicraft_xpress9::stream_datamodel(reader, &mut |at, bytes| {
            let piece_end = at.saturating_add(bytes.len() as u64);
            let (lo, hi) = (at.max(offset), piece_end.min(end));
            if lo < hi {
                let src = bytes.get((lo - at) as usize..(hi - at) as usize);
                let dst = buf.get_mut((lo - offset) as usize..(hi - offset) as usize);
                if let (Some(src), Some(dst)) = (src, dst) {
                    dst.copy_from_slice(src);
                    filled = filled.saturating_add(hi - lo);
                }
            }
            piece_end < end
        })?;
        if filled != len {
            return Err(out_of_bounds("range", offset, len, done.decoded));
        }
        Ok(buf)
    }
}

/// Reads the model from a backup through `ranges`.
fn read_model(ranges: &mut dyn Ranges) -> Result<SemanticModel, Error> {
    let (_, sqlitedb) = read_index(ranges, LOOKBEHIND)?;
    Ok(bicraft_metadata::read_sqlitedb(&sqlitedb)?)
}

/// A backup read through a function of (offset, length).
struct FnRanges<'f>(&'f mut dyn FnMut(u64, u64) -> Result<Vec<u8>, Error>);

impl Ranges for FnRanges<'_> {
    fn fetch(&mut self, offset: u64, len: u64) -> Result<Vec<u8>, Error> {
        (self.0)(offset, len)
    }
}

/// The file list and `metadata.sqlitedb` of a decompressed backup read through
/// `fetch(offset, len)` (positional reads of a backup written to a file, `docs/beyond-ram.md`):
/// the header page, the virtual directory, the BackupLog and `metadata.sqlitedb`, each within
/// [`MAX_STREAMED_FILE_BYTES`].
pub fn backup_index_at(
    fetch: &mut dyn FnMut(u64, u64) -> Result<Vec<u8>, Error>,
) -> Result<(bicraft_abf::StreamedIndex, Vec<u8>), Error> {
    read_index(&mut FnRanges(fetch), 0)
}

/// The index and `metadata.sqlitedb` of a backup through `ranges`, the virtual directory read
/// with the `lookbehind` bytes before it (where the BackupLog usually is: one pass fewer when
/// each range is a pass over a stream).
fn read_index(
    ranges: &mut dyn Ranges,
    lookbehind: u64,
) -> Result<(bicraft_abf::StreamedIndex, Vec<u8>), Error> {
    let page = ranges.fetch(0, PAGE_SIZE as u64)?;
    let header = bicraft_abf::streamed_header(&page)?;
    let (dir_at, dir_len) = (header.directory_offset, header.directory_size);
    if dir_len > MAX_STREAMED_FILE_BYTES {
        return Err(too_large("virtual directory", dir_len));
    }
    let start = dir_at
        .saturating_sub(lookbehind)
        .max(PAGE_SIZE as u64)
        .min(dir_at);
    let dir_end = dir_at
        .checked_add(dir_len)
        .ok_or_else(|| out_of_bounds("virtual directory", dir_at, dir_len, u64::MAX))?;
    let block = ranges.fetch(start, dir_end - start)?;
    let within = |offset: u64, len: u64| -> Option<&[u8]> {
        let lo = usize::try_from(offset.checked_sub(start)?).ok()?;
        let hi = lo.checked_add(usize::try_from(len).ok()?)?;
        block.get(lo..hi)
    };
    let directory = within(dir_at, dir_len).ok_or(Error::Verify("virtual directory range"))?;
    let (log_at, log_len) = bicraft_abf::streamed_log_range(&header, directory)?;
    if log_len > MAX_STREAMED_FILE_BYTES {
        return Err(too_large("BackupLog", log_len));
    }
    let fetched_log;
    let log = match within(log_at, log_len) {
        Some(b) => b,
        None => {
            fetched_log = ranges.fetch(log_at, log_len)?;
            &fetched_log
        }
    };
    let index = bicraft_abf::streamed_index(header.clone(), directory, log)?;
    let meta = index.metadata_entry().ok_or(Error::NoMetadata)?;
    let (meta_at, meta_len) = (meta.offset as u64, meta.size as u64);
    if meta_len > MAX_STREAMED_FILE_BYTES {
        return Err(too_large("metadata.sqlitedb", meta_len));
    }
    let fetched_meta;
    let sqlitedb = match within(meta_at, meta_len) {
        Some(b) => b,
        None => {
            fetched_meta = ranges.fetch(meta_at, meta_len)?;
            &fetched_meta
        }
    };
    let sqlitedb = sqlitedb.to_vec();
    Ok((index, sqlitedb))
}

/// Reads the model of a `DataModel` part (or `cache.abf`) without decompressing it into memory:
/// a deferred part is read from its source, a part in memory from its bytes.
pub fn read_data_model_streamed(part: &Part) -> Result<SemanticModel, Error> {
    let mut head = Vec::new();
    part.reader()?
        .take(bicraft_xpress9::datamodel::BANNER_SIZE as u64)
        .read_to_end(&mut head)
        .map_err(bicraft_opc::Error::Io)?;
    let layout = bicraft_xpress9::datamodel::detect_layout(&head)?;
    if layout == bicraft_xpress9::Layout::Uncompressed
        && let Some(seekable) = part.seekable()?
    {
        return read_model(&mut Seekable(seekable));
    }
    let open = || -> Result<Box<dyn Read + Send + '_>, Error> { Ok(part.reader()?) };
    read_model(&mut Streamed { open: &open })
}

/// [`read_data_model_streamed`] for a part's bytes in memory.
pub fn read_data_model_bytes_streamed(bytes: &[u8]) -> Result<SemanticModel, Error> {
    let layout = bicraft_xpress9::datamodel::detect_layout(bytes)?;
    if layout == bicraft_xpress9::Layout::Uncompressed {
        return read_model(&mut Seekable(Box::new(std::io::Cursor::new(bytes))));
    }
    let open = || -> Result<Box<dyn Read + Send + '_>, Error> { Ok(Box::new(bytes)) };
    read_model(&mut Streamed { open: &open })
}
