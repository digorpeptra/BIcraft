//! Safe extraction of an uploaded ZIP archive (a zipped PBIP project, `bicraft-cli serve`).
//!
//! The archive is hostile. [`for_each_entry`] walks its entries and hands each plain file's
//! decompressed bytes to the caller as a stream, never as a whole buffer, after these checks:
//!
//! - at most [`ZipLimits::max_entries`] entries;
//! - only plain files and folders: symbolic links and every other special entry (devices, FIFOs,
//!   sockets, by their Unix mode) are refused, and so are encrypted entries;
//! - names: relative, no `..`, no root or drive, no NUL, no empty component (the zip crate's
//!   `enclosed_name` plus our own component check); two entries whose names differ only in
//!   ASCII case are refused (they would collide on Windows and macOS);
//! - sizes are counted on the bytes actually decompressed, never trusted from the headers: each
//!   entry at most [`ZipLimits::max_entry_bytes`], all of them at most
//!   [`ZipLimits::max_total_bytes`], and an entry past 1 MiB may not expand more than
//!   [`ZipLimits::max_ratio`] times its compressed size (ZIP bombs stop early);
//! - CRC-32 mismatches fail (the zip crate checks them at the end of each entry).
//!
//! Where the bytes go is the caller's business: `bicraft-cli serve` resolves every name with the
//! engine's workspace sandbox and writes plain files only (never links).

use std::collections::HashSet;
use std::io::{self, Read, Seek};

use zip::ZipArchive;

/// Caps of one extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZipLimits {
    pub max_entries: usize,
    /// Largest decompressed entry.
    pub max_entry_bytes: u64,
    /// Largest sum of decompressed entries.
    pub max_total_bytes: u64,
    /// Largest expansion (decompressed / compressed) of an entry past 1 MiB.
    pub max_ratio: u64,
}

impl Default for ZipLimits {
    /// 50 000 entries, 4 GiB per entry, 8 GiB in total, ratio 1000.
    fn default() -> Self {
        Self {
            max_entries: 50_000,
            max_entry_bytes: 4 * 1024 * 1024 * 1024,
            max_total_bytes: 8 * 1024 * 1024 * 1024,
            max_ratio: 1000,
        }
    }
}

/// Entries smaller than this are not ratio-checked (tiny files compress very well).
const RATIO_FLOOR: u64 = 1024 * 1024;

/// One entry handed to the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The entry's name: `/`-separated, relative, without a trailing `/`.
    pub name: String,
    pub is_dir: bool,
}

/// What an extraction did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Extracted {
    pub files: usize,
    pub dirs: usize,
    pub bytes: u64,
}

/// Why an archive was refused.
#[derive(Debug)]
pub enum ExtractError {
    /// Not a ZIP archive, or a damaged one.
    Invalid(String),
    /// A rule of the module docs (the message says which).
    Refused(String),
    /// A cap: entries, bytes or expansion ratio.
    TooLarge(String),
    /// The caller's sink failed.
    Sink(String),
}

impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(m) => write!(f, "not a valid ZIP archive: {m}"),
            Self::Refused(m) | Self::TooLarge(m) | Self::Sink(m) => f.write_str(m),
        }
    }
}

/// File-type bits of a Unix mode.
const S_IFMT: u32 = 0o170000;
const S_IFREG: u32 = 0o100000;
const S_IFDIR: u32 = 0o040000;

/// Counts the bytes read from an entry and fails as soon as a cap is passed.
struct Counted<'a, R> {
    inner: R,
    read: u64,
    cap: u64,
    compressed: u64,
    ratio: u64,
    tripped: &'a mut Option<String>,
    name: &'a str,
}

impl<R: Read> Read for Counted<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read = self.read.saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
        let why = if self.read > self.cap {
            Some(format!(
                "`{}` decompresses past the {}-byte limit",
                self.name, self.cap
            ))
        } else if self.read > RATIO_FLOOR && self.read / self.compressed.max(1) > self.ratio {
            Some(format!(
                "`{}` expands more than {} times (a ZIP bomb?)",
                self.name, self.ratio
            ))
        } else {
            None
        };
        if let Some(why) = why {
            *self.tripped = Some(why.clone());
            return Err(io::Error::new(io::ErrorKind::InvalidData, why));
        }
        Ok(n)
    }
}

/// Checks an entry name (see the module docs); returns it without a trailing `/`.
fn clean_name(raw: &str) -> Result<String, String> {
    let trimmed = raw.strip_suffix('/').unwrap_or(raw);
    let bad = || format!("entry name `{}` is not a plain relative path", raw.escape_debug());
    if trimmed.is_empty() || trimmed.contains('\0') || trimmed.starts_with('/') || trimmed.starts_with('\\') {
        return Err(bad());
    }
    for part in trimmed.split(['/', '\\']) {
        if part.is_empty() || part == "." || part == ".." || part.contains(':') {
            return Err(bad());
        }
    }
    Ok(trimmed.to_string())
}

/// Walks the archive's entries under `limits` (module docs) and calls `sink` for each folder and
/// plain file, in archive order, with a stream of its decompressed bytes (empty for a folder).
/// The sink must read the stream to its end; an error from it stops the walk.
pub fn for_each_entry<R, F>(reader: R, limits: &ZipLimits, mut sink: F) -> Result<Extracted, ExtractError>
where
    R: Read + Seek,
    F: FnMut(&Entry, &mut dyn Read) -> Result<(), String>,
{
    let mut archive = ZipArchive::new(reader).map_err(|e| ExtractError::Invalid(e.to_string()))?;
    let count = archive.len();
    if count > limits.max_entries {
        return Err(ExtractError::TooLarge(format!(
            "the archive holds {count} entries, more than the {} allowed",
            limits.max_entries
        )));
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Extracted::default();
    for i in 0..count {
        // metadata first, without decompressing (an encrypted entry is refused, not an error)
        let (name, is_dir, compressed) = {
            let f = archive
                .by_index_raw(i)
                .map_err(|e| ExtractError::Invalid(e.to_string()))?;
            let fname = f.name().map_err(|e| ExtractError::Invalid(e.to_string()))?;
            if f.encrypted() {
                return Err(ExtractError::Refused(format!(
                    "`{}` is encrypted",
                    fname.escape_debug()
                )));
            }
            let special = f.is_symlink()
                || f.unix_mode().is_some_and(|mode| {
                    let kind = mode & S_IFMT;
                    kind != 0 && kind != S_IFREG && kind != S_IFDIR
                });
            {
                if special {
                    return Err(ExtractError::Refused(format!(
                        "`{}` is a symbolic link or a special file: only plain files and folders \
                         are extracted",
                        fname.escape_debug()
                    )));
                }
            }
            if f.enclosed_name().is_none() {
                return Err(ExtractError::Refused(format!(
                    "entry name `{}` leaves the destination",
                    fname.escape_debug()
                )));
            }
            let name = clean_name(&fname).map_err(ExtractError::Refused)?;
            (name, f.is_dir(), f.compressed_size())
        };
        if !seen.insert(name.to_ascii_lowercase()) {
            return Err(ExtractError::Refused(format!(
                "`{}` appears twice (names are compared ignoring ASCII case)",
                name.escape_debug()
            )));
        }
        let entry = Entry {
            name: name.clone(),
            is_dir,
        };
        if is_dir {
            sink(&entry, &mut io::empty()).map_err(ExtractError::Sink)?;
            out.dirs = out.dirs.saturating_add(1);
            continue;
        }
        let file = archive
            .by_index(i)
            .map_err(|e| ExtractError::Invalid(e.to_string()))?;
        let remaining = limits.max_total_bytes.saturating_sub(out.bytes);
        let mut tripped = None;
        let mut counted = Counted {
            inner: file,
            read: 0,
            cap: limits.max_entry_bytes.min(remaining),
            compressed,
            ratio: limits.max_ratio,
            tripped: &mut tripped,
            name: &name,
        };
        let result = sink(&entry, &mut counted);
        let read = counted.read;
        drop(counted);
        if let Some(why) = tripped {
            return Err(ExtractError::TooLarge(why));
        }
        result.map_err(ExtractError::Sink)?;
        out.bytes = out.bytes.saturating_add(read);
        out.files = out.files.saturating_add(1);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in entries {
            if name.ends_with('/') {
                w.add_directory(*name, SimpleFileOptions::default()).unwrap();
            } else {
                w.start_file(*name, SimpleFileOptions::default()).unwrap();
                w.write_all(data).unwrap();
            }
        }
        w.finish().unwrap().into_inner()
    }

    fn walk(bytes: &[u8], limits: &ZipLimits) -> Result<Vec<(String, Vec<u8>)>, ExtractError> {
        let mut got = Vec::new();
        for_each_entry(Cursor::new(bytes), limits, |e, r| {
            let mut data = Vec::new();
            r.read_to_end(&mut data).map_err(|e| e.to_string())?;
            got.push((e.name.clone(), data));
            Ok(())
        })?;
        Ok(got)
    }

    #[test]
    fn plain_files_and_folders_stream_through() {
        let z = zip_of(&[("p/", b""), ("p/a.json", b"{}"), ("p/b/c.tmdl", b"table x")]);
        let got = walk(&z, &ZipLimits::default()).unwrap();
        assert_eq!(got.len(), 3);
        assert_eq!(got[1], ("p/a.json".into(), b"{}".to_vec()));
        assert_eq!(got[0].0, "p");
    }

    #[test]
    fn hostile_names_are_refused() {
        for name in [
            "../x",
            "a/../../x",
            "/etc/x",
            "a//b",
            "./a",
            "c:/x",
            "a\\..\\b",
            "a/b:c",
        ] {
            let z = zip_of(&[(name, b"x")]);
            let e = walk(&z, &ZipLimits::default());
            assert!(matches!(e, Err(ExtractError::Refused(_))), "{name}: {e:?}");
        }
        let z = zip_of(&[("A.txt", b"1"), ("a.TXT", b"2")]);
        assert!(matches!(
            walk(&z, &ZipLimits::default()),
            Err(ExtractError::Refused(_))
        ));
    }

    #[test]
    fn symlink_entries_are_refused() {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        w.add_symlink("link", "/etc/passwd", SimpleFileOptions::default())
            .unwrap();
        let z = w.finish().unwrap().into_inner();
        let e = walk(&z, &ZipLimits::default()).unwrap_err();
        assert!(e.to_string().contains("symbolic link"), "{e}");
    }

    #[test]
    fn caps_count_real_bytes() {
        let big = vec![0u8; 3 * 1024 * 1024];
        let z = zip_of(&[("zeros.bin", &big)]);
        // ratio: 3 MiB of zeros deflate to a few KiB
        let e = walk(&z, &ZipLimits::default()).unwrap_err();
        assert!(e.to_string().contains("expands"), "{e}");
        let loose = ZipLimits {
            max_ratio: u64::MAX,
            ..ZipLimits::default()
        };
        assert_eq!(walk(&z, &loose).unwrap()[0].1.len(), big.len());
        let small = ZipLimits {
            max_entry_bytes: 1000,
            ..loose
        };
        assert!(walk(&z, &small).unwrap_err().to_string().contains("limit"));
        let total = ZipLimits {
            max_total_bytes: 5,
            ..loose
        };
        let z2 = zip_of(&[("a", b"123"), ("b", b"456")]);
        assert!(walk(&z2, &total).unwrap_err().to_string().contains("limit"));
        let few = ZipLimits {
            max_entries: 1,
            ..loose
        };
        assert!(walk(&z2, &few).unwrap_err().to_string().contains("entries"));
    }

    #[test]
    fn garbage_is_invalid_not_a_crash() {
        assert!(matches!(
            walk(b"not a zip at all", &ZipLimits::default()),
            Err(ExtractError::Invalid(_))
        ));
        let mut z = zip_of(&[("a.txt", b"hello world hello world")]);
        // flip bytes everywhere: never a panic
        for i in 0..z.len() {
            let mut m = z.clone();
            m[i] ^= 0xff;
            let _ = walk(&m, &ZipLimits::default());
        }
        z.truncate(z.len() / 2);
        assert!(walk(&z, &ZipLimits::default()).is_err());
    }
}
