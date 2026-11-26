//! Parts too large to hold in memory (`docs/formats/pbix-package.md` §11, "Packages over
//! 4 GiB"). When a package is read from a source that can be opened again (a file), a part whose
//! declared size is over [`crate::ReadLimits::max_part_bytes`] is not read: the part records where
//! its compressed bytes are ([`Deferred`]) and [`crate::Package::write`] copies them from there,
//! in fixed-size pieces, without decompressing or recompressing them.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// A reader that can also seek.
pub trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

/// Something that can open the whole archive again, any number of times.
pub trait ArchiveSource: Send + Sync + std::fmt::Debug {
    /// A fresh reader positioned at the start of the archive.
    fn open(&self) -> io::Result<Box<dyn ReadSeek>>;

    /// Reads `buf.len()` bytes of the archive at `offset`, on several threads where the source
    /// allows it, and returns their CRC-32. `None`: this source cannot (the caller reads through
    /// [`ArchiveSource::open`]).
    fn read_at(&self, _offset: u64, _buf: &mut [u8]) -> Option<io::Result<u32>> {
        None
    }
}

/// A package file on disk. Opening it again fails when the file changed since it was read (its
/// length or modification time differ), so a part is never copied from a different file.
#[derive(Debug, Clone)]
pub struct FileSource {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
}

impl FileSource {
    /// Records the file's current length and modification time.
    pub fn new(path: &Path) -> io::Result<FileSource> {
        let meta = std::fs::metadata(path)?;
        Ok(FileSource {
            path: path.to_path_buf(),
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file, opened again; an error when it changed since it was recorded.
    fn reopen(&self) -> io::Result<std::fs::File> {
        let file = std::fs::File::open(&self.path)?;
        let meta = file.metadata()?;
        if meta.len() != self.len || meta.modified().ok() != self.modified {
            return Err(io::Error::other(format!(
                "{} changed since it was opened; open it again",
                self.path.display()
            )));
        }
        Ok(file)
    }
}

impl ArchiveSource for FileSource {
    fn open(&self) -> io::Result<Box<dyn ReadSeek>> {
        Ok(Box::new(io::BufReader::with_capacity(1 << 16, self.reopen()?)))
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Option<io::Result<u32>> {
        Some(
            self.reopen()
                .and_then(|_| crate::parread::read_at(&self.path, offset, buf)),
        )
    }
}

/// Where the bytes of a part that was not read into memory are.
#[derive(Debug, Clone)]
pub struct Deferred {
    pub source: Arc<dyn ArchiveSource>,
    /// Offset of the part's compressed data in the archive.
    pub data_offset: u64,
    pub compressed_size: u64,
    /// Decompressed size.
    pub size: u64,
    pub crc32: u32,
}

/// Two deferred parts are equal when they describe the same bytes (the sources are not
/// compared).
impl PartialEq for Deferred {
    fn eq(&self, other: &Self) -> bool {
        self.compressed_size == other.compressed_size && self.size == other.size && self.crc32 == other.crc32
    }
}

impl Eq for Deferred {}

impl Deferred {
    /// The compressed bytes as stored in the archive.
    pub fn raw_reader(&self) -> io::Result<Window<Box<dyn ReadSeek>>> {
        Window::new(self.source.open()?, self.data_offset, self.compressed_size)
    }
}

/// A read-only, seekable view of `len` bytes of `inner` starting at `base`.
#[derive(Debug)]
pub struct Window<R> {
    inner: R,
    base: u64,
    len: u64,
    pos: u64,
}

impl<R: Read + Seek> Window<R> {
    pub fn new(mut inner: R, base: u64, len: u64) -> io::Result<Window<R>> {
        base.checked_add(len)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "window past the end"))?;
        inner.seek(SeekFrom::Start(base))?;
        Ok(Window {
            inner,
            base,
            len,
            pos: 0,
        })
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<R: Read + Seek> Read for Window<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.len.saturating_sub(self.pos);
        let want = usize::try_from(left).unwrap_or(usize::MAX).min(buf.len());
        let Some(dst) = buf.get_mut(..want) else {
            return Ok(0);
        };
        if dst.is_empty() {
            return Ok(0);
        }
        let n = self.inner.read(dst)?;
        self.pos = self.pos.saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
        Ok(n)
    }
}

impl<R: Read + Seek> Seek for Window<R> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let bad = || io::Error::new(io::ErrorKind::InvalidInput, "seek outside the window");
        let target = match to {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        }
        .ok_or_else(bad)?;
        let absolute = self.base.checked_add(target).ok_or_else(bad)?;
        self.inner.seek(SeekFrom::Start(absolute))?;
        self.pos = target;
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn window_reads_and_seeks_inside_its_range() {
        let data: Vec<u8> = (0u8..100).collect();
        let mut w = Window::new(Cursor::new(data), 10, 20).unwrap();
        let mut out = Vec::new();
        w.read_to_end(&mut out).unwrap();
        assert_eq!(out, (10u8..30).collect::<Vec<_>>());
        assert_eq!(w.seek(SeekFrom::End(-5)).unwrap(), 15);
        let mut b = [0u8; 10];
        assert_eq!(w.read(&mut b).unwrap(), 5);
        assert_eq!(b[..5], [25, 26, 27, 28, 29]);
        assert!(w.seek(SeekFrom::Current(-100)).is_err());
        assert!(Window::new(Cursor::new(vec![]), u64::MAX, 2).is_err());
    }

    #[test]
    fn a_changed_file_is_refused() {
        let dir = std::env::temp_dir().join(format!("bicraft-opc-src-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.bin");
        std::fs::write(&path, b"abc").unwrap();
        let src = FileSource::new(&path).unwrap();
        assert!(src.open().is_ok());
        std::fs::write(&path, b"abcd").unwrap();
        assert!(src.open().is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
