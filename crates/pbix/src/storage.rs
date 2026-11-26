//! Selected files of a `DataModel` backup (or a PBIP `.pbi/cache.abf`): the file list,
//! `metadata.sqlitedb`, and the contents of the files a caller asks for, without keeping the
//! rest of the backup. Storage statistics (`model.storage_stats`, VPAX export) need only the
//! small files (`.idfmeta` segment maps) and every file's size.
//!
//! A backup within [`DataModelLimits`] is decompressed in memory once; a larger one (or a part
//! left in the file, see `bicraft_opc::Part::deferred`) is decoded as a stream
//! ([`bicraft_xpress9::stream_datamodel`]), keeping only the header page, the virtual
//! directory, the BackupLog and the requested files, as [`crate::large`] does for the model.

use std::collections::HashMap;
use std::io::Read;

use bicraft_abf::{FileEntry, PAGE_SIZE};
use bicraft_opc::Part;

use crate::large::MAX_STREAMED_FILE_BYTES;
use crate::{DataModelLimits, Error};

/// Largest total size of the requested files kept in memory.
pub const MAX_SELECTED_BYTES: u64 = 1 << 30;

/// Bytes kept before the virtual directory on the pass that reads it (the BackupLog is usually
/// just before it).
const LOOKBEHIND: u64 = 64 << 20;

/// The parts of a backup storage statistics need.
#[derive(Debug, Clone, Default)]
pub struct StorageFiles {
    /// Every file of the backup (logical paths, offsets and sizes).
    pub files: Vec<FileEntry>,
    /// `metadata.sqlitedb`.
    pub sqlitedb: Vec<u8>,
    /// The requested files, by logical path (`/`-separated, as in [`FileEntry::path`]).
    pub contents: HashMap<String, Vec<u8>>,
    /// Whether the backup was decoded as a stream (too large to decompress in memory).
    pub streamed: bool,
}

/// Which files to keep: called once per file of the backup.
pub type Want<'a> = &'a dyn Fn(&FileEntry) -> bool;

/// [`StorageFiles`] of a package part (a `DataModel`): read from memory, or streamed from the
/// file when the part was left there.
pub fn read_part(part: &Part, want: Want<'_>, limits: &DataModelLimits) -> Result<StorageFiles, Error> {
    if !part.is_deferred() {
        return read_bytes(&part.data, want, limits);
    }
    let open = || -> Result<Box<dyn Read + Send + '_>, Error> { Ok(part.reader()?) };
    streamed(&open, want)
}

/// [`StorageFiles`] of a `DataModel` (or `cache.abf`) held in memory.
pub fn read_bytes(bytes: &[u8], want: Want<'_>, limits: &DataModelLimits) -> Result<StorageFiles, Error> {
    let declared = bicraft_xpress9::inspect_datamodel(bytes)?.total_uncompressed;
    let limit = u64::try_from(limits.limit_for(bytes.len())).unwrap_or(u64::MAX);
    if declared > limit {
        let open = || -> Result<Box<dyn Read + Send + '_>, Error> { Ok(Box::new(bytes)) };
        return streamed(&open, want);
    }
    let abf_bytes = crate::decompress_data_model(bytes, limits)?;
    let abf = bicraft_abf::Abf::parse(&abf_bytes)?;
    let sqlitedb = abf.metadata_sqlitedb().ok_or(Error::NoMetadata)?.to_vec();
    let mut contents = HashMap::new();
    let mut total = 0u64;
    for f in abf.files() {
        if !want(f) {
            continue;
        }
        total = total.saturating_add(f.size as u64);
        if total > MAX_SELECTED_BYTES {
            return Err(too_large("set of selected files", total));
        }
        if let Some(data) = abf.data(f) {
            contents.insert(f.path.clone(), data.to_vec());
        }
    }
    Ok(StorageFiles {
        files: abf.files().to_vec(),
        sqlitedb,
        contents,
        streamed: false,
    })
}

/// [`StorageFiles`] of a `DataModel` (or `cache.abf`) file on disk: read in memory when it is
/// small enough for `limits`, streamed otherwise.
pub fn read_file(
    path: &std::path::Path,
    want: Want<'_>,
    limits: &DataModelLimits,
) -> Result<StorageFiles, Error> {
    let len = std::fs::metadata(path).map_err(bicraft_opc::Error::Io)?.len();
    if len <= u64::try_from(limits.max_bytes).unwrap_or(u64::MAX) {
        let bytes = std::fs::read(path).map_err(bicraft_opc::Error::Io)?;
        return read_bytes(&bytes, want, limits);
    }
    let open = || -> Result<Box<dyn Read + Send + '_>, Error> {
        let f = std::fs::File::open(path).map_err(bicraft_opc::Error::Io)?;
        Ok(Box::new(std::io::BufReader::new(f)))
    };
    streamed(&open, want)
}

fn too_large(what: &'static str, len: u64) -> Error {
    Error::Abf(bicraft_abf::Error::Unsupported(format!(
        "a {len}-byte {what} (over the limit for storage statistics)"
    )))
}

type Opener<'a> = dyn Fn() -> Result<Box<dyn Read + Send + 'a>, Error> + 'a;

/// One pass over the stream that fills every `(offset, len)` range; stops after the last one.
fn fetch_many(open: &Opener<'_>, ranges: &[(u64, u64)]) -> Result<Vec<Vec<u8>>, Error> {
    let mut bufs: Vec<Vec<u8>> = Vec::with_capacity(ranges.len());
    let mut total = 0u64;
    for &(_, len) in ranges {
        total = total.saturating_add(len);
        if total > MAX_SELECTED_BYTES.saturating_add(MAX_STREAMED_FILE_BYTES) {
            return Err(too_large("set of selected files", total));
        }
        let n = usize::try_from(len).map_err(|_| too_large("range", len))?;
        let mut v = Vec::new();
        v.try_reserve_exact(n).map_err(|_| too_large("range", len))?;
        v.resize(n, 0);
        bufs.push(v);
    }
    let mut order: Vec<usize> = (0..ranges.len()).collect();
    order.sort_by_key(|&i| ranges.get(i).map_or(0, |r| r.0));
    let last_end = ranges
        .iter()
        .map(|&(o, l)| o.saturating_add(l))
        .max()
        .unwrap_or(0);
    let mut filled = vec![0u64; ranges.len()];
    // Index into `order` of the first range that may still overlap the next piece.
    let mut first = 0usize;
    let reader = open()?;
    let done = bicraft_xpress9::stream_datamodel(reader, &mut |at, bytes| {
        let piece_end = at.saturating_add(bytes.len() as u64);
        while let Some(&i) = order.get(first) {
            match ranges.get(i) {
                Some(&(o, l)) if o.saturating_add(l) <= at => first += 1,
                _ => break,
            }
        }
        for &i in order.iter().skip(first) {
            let Some(&(offset, len)) = ranges.get(i) else {
                continue;
            };
            if offset >= piece_end {
                break;
            }
            let end = offset.saturating_add(len);
            let (lo, hi) = (at.max(offset), piece_end.min(end));
            if lo >= hi {
                continue;
            }
            let src = bytes.get((lo - at) as usize..(hi - at) as usize);
            let dst = bufs
                .get_mut(i)
                .and_then(|b| b.get_mut((lo - offset) as usize..(hi - offset) as usize));
            if let (Some(src), Some(dst)) = (src, dst) {
                dst.copy_from_slice(src);
                if let Some(f) = filled.get_mut(i) {
                    *f = f.saturating_add(hi - lo);
                }
            }
        }
        piece_end < last_end
    })?;
    for (i, &(offset, len)) in ranges.iter().enumerate() {
        if filled.get(i).copied() != Some(len) {
            return Err(Error::Abf(bicraft_abf::Error::OutOfBounds {
                what: "range",
                offset,
                len,
                stream_len: usize::try_from(done.decoded).unwrap_or(usize::MAX),
            }));
        }
    }
    Ok(bufs)
}

fn fetch(open: &Opener<'_>, offset: u64, len: u64) -> Result<Vec<u8>, Error> {
    fetch_many(open, &[(offset, len)])?
        .pop()
        .ok_or(Error::Verify("streamed range"))
}

fn streamed(open: &Opener<'_>, want: Want<'_>) -> Result<StorageFiles, Error> {
    let page = fetch(open, 0, PAGE_SIZE as u64)?;
    let header = bicraft_abf::streamed_header(&page)?;
    let (dir_at, dir_len) = (header.directory_offset, header.directory_size);
    if dir_len > MAX_STREAMED_FILE_BYTES {
        return Err(too_large("virtual directory", dir_len));
    }
    let start = dir_at
        .saturating_sub(LOOKBEHIND)
        .max(PAGE_SIZE as u64)
        .min(dir_at);
    let dir_end = dir_at
        .checked_add(dir_len)
        .ok_or(Error::Verify("virtual directory range"))?;
    let block = fetch(open, start, dir_end - start)?;
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
            fetched_log = fetch(open, log_at, log_len)?;
            &fetched_log
        }
    };
    let index = bicraft_abf::streamed_index(header.clone(), directory, log)?;
    let meta = index.metadata_entry().ok_or(Error::NoMetadata)?.clone();
    if meta.size as u64 > MAX_STREAMED_FILE_BYTES {
        return Err(too_large("metadata.sqlitedb", meta.size as u64));
    }
    // One more pass for metadata.sqlitedb and every requested file not already in `block`.
    let mut ranges = Vec::new();
    let mut paths = Vec::new();
    let mut contents = HashMap::new();
    let mut sqlitedb = within(meta.offset as u64, meta.size as u64).map(<[u8]>::to_vec);
    if sqlitedb.is_none() {
        ranges.push((meta.offset as u64, meta.size as u64));
        paths.push(None);
    }
    for f in &index.files {
        if !want(f) {
            continue;
        }
        match within(f.offset as u64, f.size as u64) {
            Some(b) => {
                contents.insert(f.path.clone(), b.to_vec());
            }
            None => {
                ranges.push((f.offset as u64, f.size as u64));
                paths.push(Some(f.path.clone()));
            }
        }
    }
    if !ranges.is_empty() {
        for (data, path) in fetch_many(open, &ranges)?.into_iter().zip(paths) {
            match path {
                Some(p) => {
                    contents.insert(p, data);
                }
                None => sqlitedb = Some(data),
            }
        }
    }
    Ok(StorageFiles {
        files: index.files,
        sqlitedb: sqlitedb.ok_or(Error::NoMetadata)?,
        contents,
        streamed: true,
    })
}
